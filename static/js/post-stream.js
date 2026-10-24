// The topic's post stream loading more posts as the reader nears either
// end, as Ember's does (models/post-stream.js appendMore and prependMore,
// components/post-stream.gjs, post/load-more-accessible.gjs, DLoadMore).
// The page draws a window of posts and the stream's ids (#posts'
// data-stream); a DLoadMore sentinel above the posts while the first is
// not drawn (canPrependMore) and below them while the last is not
// (canAppendMore). When one comes into view the next chunk of ids is
// asked for with the params Ember sends GET /t/:id/posts.json, from
// /live/posts/:id, which answers with the posts drawn as the page draws
// them. While a chunk loads its sentinel goes, the spinner beside the
// stream shows (DConditionalLoadingSpinner) and, below, a placeholder
// stands in for each post (appendingPlaceholders). Once the stream's last
// post is in (loadedAllPosts) what Ember draws only then comes out of its
// templates: the bottom topic map, the footer buttons, the more topics
// (the suggested topics the first chunk brought when the page had none)
// and the page footer; and the page's live stream starts appending new
// posts (its tail), from where it left off.
(function () {
  "use strict";

  if (!Discourse.once("post-stream")) {
    return;
  }

  // DLoadMore's discourseDebounce(this, action, 100).
  var DEBOUNCE_MS = 100;

  var PLACEHOLDER =
    '<article class="placeholder"><div class="row"><div class="topic-avatar"><div class="placeholder-avatar placeholder-animation"></div></div><div class="topic-body"><div class="placeholder-text placeholder-animation"></div><div class="placeholder-text placeholder-animation"></div><div class="placeholder-text placeholder-animation"></div></div></div></article>';

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function spinner(container, on) {
    if (!container) {
      return;
    }
    container.classList.toggle("visible", on);
    if (on) {
      container.setAttribute("data-loading", "true");
      container.innerHTML = '<div class="spinner "></div>';
    } else {
      container.removeAttribute("data-loading");
      container.innerHTML = "";
    }
  }

  // The page's live stream connected again from the last event it had,
  // with the posts the page now draws (their changes are redrawn) and,
  // once they reach the end, the tail (new posts appended).
  var lastEventId = null;
  document.addEventListener("htmx:sseMessage", function (event) {
    if (event.detail && event.detail.lastEventId) {
      lastEventId = event.detail.lastEventId;
    }
  });

  function reconnectLive(range, tail) {
    var sink = document.querySelector('[sse-connect*="/live?"][sse-connect*="topic="]');
    if (!sink || !window.htmx) {
      return;
    }
    var url = new URL(sink.getAttribute("sse-connect"), location.href);
    if (lastEventId) {
      url.searchParams.set("position", lastEventId);
    }
    if (tail) {
      url.searchParams.set("tail", "1");
    }
    if (range) {
      url.searchParams.set("posts", range);
    }
    var fresh = sink.cloneNode(false);
    fresh.setAttribute("sse-connect", url.pathname + url.search);
    // The extension closes the old element's EventSource on its cleanup.
    window.htmx.trigger(sink, "htmx:beforeCleanupElement");
    sink.replaceWith(fresh);
    window.htmx.process(fresh);
  }

  Discourse.onPage(function () {
    var posts = document.getElementById("posts");
    var topic = document.getElementById("topic");
    if (!posts || !topic || !posts.dataset.stream) {
      return;
    }
    var topicId = topic.dataset.topicId;
    var stream = JSON.parse(posts.dataset.stream);
    var chunk = parseInt(posts.dataset.chunkSize, 10) || 20;
    var hasSuggested = posts.hasAttribute("data-has-suggested");
    var moreTopics = null;
    var spinners = {
      above: posts.previousElementSibling,
      below: posts.nextElementSibling,
    };
    var loading = { above: false, below: false };
    var timers = {};
    var observers = [];
    var gone = false;

    function loadedIds() {
      return Array.prototype.map.call(posts.querySelectorAll(":scope > [data-post-number] > article[data-post-id]"), function (a) {
        return parseInt(a.dataset.postId, 10);
      });
    }

    // loadedAllPosts: the stream's last post is drawn.
    function loadedAll() {
      var last = stream[stream.length - 1];
      return last !== undefined && loadedIds().indexOf(last) !== -1;
    }

    // firstPostPresent
    function firstLoaded() {
      return !!posts.querySelector(':scope > [data-post-number="1"]');
    }

    // nextWindow and previousWindow
    function nextWindow() {
      var ids = loadedIds();
      var index = stream.indexOf(ids[ids.length - 1]);
      return index === -1 ? [] : stream.slice(index + 1, index + chunk + 1);
    }

    function previousWindow() {
      var index = stream.indexOf(loadedIds()[0]);
      return index === -1 ? [] : stream.slice(Math.max(index - chunk, 0), index);
    }

    function sentinel(direction) {
      var heading = document.getElementById("post-stream-load-more-heading__" + direction);
      return heading ? heading.parentElement.previousElementSibling : null;
    }

    // The headings' labels (post.load_more_posts_above and _below).
    var labels = {};
    ["above", "below"].forEach(function (direction) {
      var heading = document.getElementById("post-stream-load-more-heading__" + direction);
      labels[direction] = heading ? heading.textContent : "";
    });

    // PostLoadMoreAccessible: the sentinel and the screen reader heading.
    function loadMoreHtml(direction) {
      var html =
        '<div aria-hidden="true" class="load-more-sentinel"></div><div class="post-stream-load-more-accessible sr-only"><h2 class="post-stream-load-more-accessible__heading" id="post-stream-load-more-heading__' +
        direction +
        '"></h2></div>';
      var t = document.createElement("template");
      t.innerHTML = html;
      t.content.querySelector("h2").textContent = labels[direction];
      return t.content;
    }

    function removeLoadMore(direction) {
      var el = sentinel(direction);
      if (el) {
        el.nextElementSibling.remove();
        el.remove();
      }
    }

    // DLoadMore: an IntersectionObserver on the sentinel, made again each
    // time it is drawn (so one still in view loads the next chunk too).
    function watch(direction) {
      var el = sentinel(direction);
      if (!el) {
        return;
      }
      var observer = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting && !loading[direction]) {
              clearTimeout(timers[direction]);
              timers[direction] = setTimeout(function () {
                load(direction);
              }, DEBOUNCE_MS);
            }
          });
        },
        { threshold: 0, rootMargin: "0px 0px 0px 0px" }
      );
      observer.observe(el);
      observers.push(observer);
    }

    function insert(html, direction) {
      var template = document.createElement("template");
      template.innerHTML = html;
      var more = template.content.querySelector("template.post-stream-more-topics");
      if (more) {
        // _setSuggestedTopics
        moreTopics = more.innerHTML;
        hasSuggested = true;
        more.remove();
      }
      var drawn = {};
      posts.querySelectorAll(":scope > [data-post-number]").forEach(function (el) {
        drawn[el.dataset.postNumber] = true;
      });
      var nodes = Array.prototype.slice.call(template.content.childNodes).filter(function (node) {
        return !(node.dataset && node.dataset.postNumber && drawn[node.dataset.postNumber]);
      });
      var fragment = document.createDocumentFragment();
      nodes.forEach(function (node) {
        fragment.appendChild(node);
      });
      if (direction === "above") {
        posts.insertBefore(fragment, posts.firstChild);
      } else {
        var boundary = posts.querySelector(":scope > .post-stream__bottom-boundary");
        posts.insertBefore(fragment, boundary);
      }
      nodes.forEach(function (node) {
        if (node.nodeType === 1 && window.htmx) {
          window.htmx.process(node);
        }
      });
    }

    function fetchPosts(ids) {
      var query = ids
        .map(function (id) {
          return "post_ids%5B%5D=" + id;
        })
        .join("&");
      query += "&include_suggested=" + (hasSuggested ? "false" : "true");
      return fetch(base() + "/live/posts/" + topicId + "?" + query, {
        credentials: "same-origin",
        headers: { Accept: "text/html", "X-Requested-With": "XMLHttpRequest" },
      }).then(function (r) {
        // A 403: the page reloads rather than retrying (loadIntoIdentityMap).
        if (r.status === 403) {
          window.location.reload();
          return null;
        }
        if (!r.ok) {
          throw new Error("/live/posts/" + topicId + ": " + r.status);
        }
        return r.text();
      });
    }

    // What the page draws once the last post is in.
    function showLoadedAll() {
      posts.insertAdjacentHTML("beforeend", '<div class="post-stream__bottom-boundary"></div>');
      document.querySelectorAll("template.loaded-all-posts").forEach(function (template) {
        var content = template.content;
        var container = content.querySelector(".more-topics__container");
        if (container && moreTopics !== null) {
          var t = document.createElement("template");
          t.innerHTML = moreTopics;
          container.replaceWith(t.content);
        }
        var nodes = Array.prototype.slice.call(content.childNodes);
        template.replaceWith(content);
        nodes.forEach(function (node) {
          if (node.nodeType === 1 && window.htmx) {
            window.htmx.process(node);
          }
        });
      });
      if (Discourse.showFooter) {
        Discourse.showFooter();
      }
      document.dispatchEvent(new CustomEvent("post-stream:loaded-all"));
    }

    // The post numbers drawn, first to last.
    function range() {
      var numbers = Array.prototype.map.call(posts.querySelectorAll(":scope > [data-post-number]"), function (el) {
        return parseInt(el.dataset.postNumber, 10);
      });
      return numbers.length ? Math.min.apply(null, numbers) + "-" + Math.max.apply(null, numbers) : null;
    }

    // appendMore and prependMore.
    function load(direction) {
      if (gone || loading[direction]) {
        return;
      }
      var ids = direction === "below" ? nextWindow() : previousWindow();
      if (!ids.length) {
        return;
      }
      loading[direction] = true;
      removeLoadMore(direction);
      spinner(spinners[direction], true);
      var placeholders = [];
      var loaded = false;
      if (direction === "below") {
        ids.forEach(function () {
          var t = document.createElement("template");
          t.innerHTML = PLACEHOLDER;
          var el = t.content.firstChild;
          posts.appendChild(el);
          placeholders.push(el);
        });
      }
      fetchPosts(ids)
        .then(function (html) {
          if (gone || html === null) {
            return;
          }
          // The anchor's place before the posts above go in (captureAnchor):
          // the first post on screen stays where it is.
          var anchor = null;
          var anchorTop = null;
          if (direction === "above") {
            anchor = posts.querySelector(":scope > [data-post-number]");
            if (anchor) {
              var rect = anchor.getBoundingClientRect();
              if (rect.bottom > 0 && rect.top < window.innerHeight) {
                anchorTop = rect.top;
              }
            }
          }
          placeholders.forEach(function (el) {
            el.remove();
          });
          insert(html, direction);
          loaded = true;
          spinner(spinners[direction], false);
          if (anchorTop !== null) {
            var shift = anchor.getBoundingClientRect().top - anchorTop;
            if (shift !== 0) {
              window.scrollBy(0, shift);
            }
          }
        })
        .catch(function (e) {
          placeholders.forEach(function (el) {
            el.remove();
          });
          spinner(spinners[direction], false);
          console.error(e);
        })
        .then(function () {
          loading[direction] = false;
          if (gone) {
            return;
          }
          if (direction === "below") {
            if (loadedAll()) {
              showLoadedAll();
            } else {
              posts.appendChild(loadMoreHtml("below"));
              watch("below");
            }
          } else if (!firstLoaded()) {
            posts.insertBefore(loadMoreHtml("above"), posts.firstChild);
            watch("above");
          }
          if (loaded) {
            reconnectLive(range(), loadedAll());
          }
        });
    }

    watch("above");
    watch("below");

    Discourse.postStream = {
      loadedAll: loadedAll,
      appendMore: function () {
        load("below");
      },
      prependMore: function () {
        load("above");
      },
    };

    return function () {
      gone = true;
      observers.forEach(function (o) {
        o.disconnect();
      });
      clearTimeout(timers.above);
      clearTimeout(timers.below);
      Discourse.postStream = null;
    };
  });
})();
