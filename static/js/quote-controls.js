// Quote expand and collapse, as Ember's quote-controls decorator
// (lib/post-cooked-html-decorators/quote-controls.gjs) renders
// PostQuotedContent (components/post/quoted-content.gjs) in each
// aside.quote of a post: the title gets the toggle and, for a quote of the
// same topic, the link to the quoted post; expanding replaces the quote's
// text with the whole quoted post from /posts/by_number/:topic/:post, the
// quoted text highlighted in it. A quote of someone the viewer ignores
// (#posts' data-ignored-users) is marked ignored-user, keeps no text and
// cannot be expanded.
(function () {
  "use strict";

  var Discourse = window.Discourse;
  if (!Discourse || !Discourse.decorateCooked || Discourse.quoteControls) {
    return;
  }
  Discourse.quoteControls = true;

  var t = function (key) {
    return Discourse.cooked.t(key);
  };

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  // postUrl
  function postUrl(slug, topicId, postNumber) {
    var url = basePath() + "/t/";
    if (slug) {
      url += slug + "/";
    }
    url += topicId;
    if (postNumber > 1) {
      url += "/" + postNumber;
    }
    return url;
  }

  // currentUser.ignored_users, as the topic page lists them.
  function ignoredUsers() {
    var posts = document.getElementById("posts");
    var list = posts && posts.dataset.ignoredUsers;
    return list ? list.split(",") : [];
  }

  // The topic the page shows: its id and slug (from the title's link).
  function currentTopic() {
    var h1 = document.querySelector("#topic-title h1[data-topic-id]");
    if (!h1) {
      return null;
    }
    var link = h1.querySelector("a.fancy-title");
    var match = link && link.getAttribute("href").match(/\/t\/([^/]+)\/\d+/);
    return { id: parseInt(h1.dataset.topicId, 10), slug: match ? match[1] : null };
  }

  // lib/highlight-html.js, matching case: wraps the words where they stand
  // alone in span.highlighted.
  function highlight(node, pattern) {
    if (
      [Node.ELEMENT_NODE, Node.TEXT_NODE].indexOf(node.nodeType) === -1 ||
      ["SCRIPT", "STYLE"].indexOf(node.tagName) !== -1 ||
      (node.tagName === "SPAN" && node.className === "highlighted")
    ) {
      return 0;
    }
    if (node.nodeType === Node.ELEMENT_NODE) {
      for (var i = 0; i < node.childNodes.length; i++) {
        i += highlight(node.childNodes[i], pattern);
      }
      return 0;
    }
    var match;
    try {
      match = node.data.match(pattern);
    } catch (e) {
      return 0;
    }
    if (!match) {
      return 0;
    }
    var element = document.createElement("span");
    element.className = "highlighted";
    element.innerText = match[2];
    var matchNode = node.splitText(match.index + match[1].length);
    matchNode.splitText(match[2].length);
    matchNode.parentNode.replaceChild(element, matchNode);
    return 1;
  }

  function highlightHTML(node, text) {
    if (!text) {
      return;
    }
    var word = text.replace(/[-\/\\^$*+?.()|[\]{}]/g, "\\$&");
    highlight(node, new RegExp("(\\W|^)(" + word + ")(\\W|$)"));
  }

  function loading() {
    var el = document.createElement("div");
    el.className = "loading-container visible";
    el.dataset.loading = "true";
    el.innerHTML = '<div class="spinner "></div>';
    return el;
  }

  // DAsyncContent's error block.
  function errorContent(status, message) {
    var el = document.createElement("div");
    if (status === 403 || status === 404) {
      el.className = "quote-error expanded-quote icon-only";
      el.innerHTML = Discourse.cooked.icon(status === 403 ? "lock" : "trash-can");
    } else {
      el.className = "quote-error expanded-quote";
      var alert = document.createElement("div");
      alert.className = "alert alert-error";
      alert.setAttribute("role", "alert");
      alert.textContent = message;
      el.appendChild(alert);
    }
    return el;
  }

  // extractErrorInfo's message for a failed request.
  function errorMessage(result) {
    var body = result.body || {};
    var errors = body.errors || [];
    if (errors.length > 1) {
      return t("multiple_errors").replace(
        "%{errors}",
        errors.map(function (e, i) { return i + 1 + ") " + e; }).join(" ")
      );
    }
    if (errors.length > 0) {
      return t("generic_error_with_reason").replace("%{error}", errors[0]);
    }
    if (body.error || body.message || body.failed) {
      return body.error || body.message || body.failed;
    }
    if (result.status >= 400) {
      return result.status + " " + (result.statusText || "");
    }
    return t("generic_error");
  }

  function fetchPost(topicId, postNumber) {
    return fetch(basePath() + "/posts/by_number/" + topicId + "/" + postNumber, {
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
      credentials: "same-origin",
    }).then(function (response) {
      return response.json().then(
        function (body) {
          return { status: response.status, statusText: response.statusText, ok: response.ok, body: body };
        },
        function () {
          return { status: response.status, statusText: response.statusText, ok: false, body: {} };
        }
      );
    });
  }

  function decorateQuote(aside, index, post) {
    var quotedPostRaw = aside.dataset.post;
    if (!quotedPostRaw) {
      return;
    }
    var quotedTopicId = parseInt(aside.dataset.topic || post.topicId, 10);
    var quotedPostNumber = parseInt(quotedPostRaw, 10);
    var id = "quote-id-" + quotedTopicId + "-" + quotedPostNumber + "-" + index;
    var fullQuote = aside.dataset.full === "true";
    var notFound = aside.classList.contains("quote-post-not-found");
    var expanded = aside.dataset.expanded === "true";
    // isQuotedPostIgnored
    var ignored = ignoredUsers().indexOf(aside.dataset.username) !== -1;
    if (ignored) {
      aside.classList.add("ignored-user");
    }

    var title = aside.querySelector(".title");
    if (!title) {
      title = document.createElement("div");
      title.className = "title";
      aside.prepend(title);
    }
    var oldControls = title.querySelector(".quote-controls");
    if (oldControls) {
      oldControls.remove();
    }

    var blockquote = aside.querySelector("blockquote");
    if (!blockquote) {
      blockquote = document.createElement("blockquote");
      aside.appendChild(blockquote);
    }
    var originalText = blockquote.textContent.trim();
    blockquote.id = id;
    var collapsed = document.createElement("div");
    collapsed.className = "post__contents-cooked-quote";
    while (blockquote.firstChild) {
      collapsed.appendChild(blockquote.firstChild);
    }
    blockquote.appendChild(collapsed);

    // quotedPostUrl: only for a quote of the post's own topic.
    var quotedPostUrl =
      quotedTopicId === post.topicId && post.slug && quotedPostNumber
        ? postUrl(post.slug, quotedTopicId, quotedPostNumber)
        : null;
    var canNavigate = !notFound && quotedPostUrl ? quotedPostUrl : null;
    var canToggle = !notFound && !fullQuote && !ignored;

    aside.setAttribute("role", "none");

    if (canNavigate) {
      title.dataset.canNavigateToPost = canNavigate;
    }
    if (canToggle) {
      title.dataset.canToggleQuote = "";
    }
    if (canNavigate || canToggle) {
      title.dataset.hasQuoteControls = canNavigate || "";
    }

    var toggle = null;
    if (canNavigate || canToggle) {
      var controls = document.createElement("div");
      controls.className = "quote-controls";
      if (canToggle) {
        toggle = document.createElement("button");
        toggle.setAttribute("aria-controls", id);
        toggle.className = "btn no-text btn-flat quote-toggle";
        toggle.type = "button";
        toggle.addEventListener("click", toggleExpanded);
        controls.appendChild(toggle);
      }
      if (canNavigate) {
        var back = document.createElement("a");
        back.setAttribute("aria-label", t("post.follow_quote"));
        back.className = "btn no-text btn-flat back";
        back.href = canNavigate;
        back.title = t("post.follow_quote");
        back.innerHTML = Discourse.cooked.icon(
          post.postNumber < quotedPostNumber ? "arrow-down" : "arrow-up"
        );
        controls.appendChild(back);
      }
      title.appendChild(controls);
    }

    if (canToggle) {
      title.addEventListener("click", function (event) {
        // The path as dispatched: the toggle redraws its icon before the
        // click gets here.
        var path = event.composedPath();
        var onControl = path.slice(0, path.indexOf(title)).some(function (node) {
          return node.nodeType === 1 && (node.matches("a") || node.matches(".quote-controls"));
        });
        if (onControl) {
          return;
        }
        toggleExpanded();
      });
    }

    var quotedPost = null;
    var request = 0;

    function showExpanded() {
      var current = ++request;
      blockquote.replaceChildren(loading());
      var load = quotedPost
        ? Promise.resolve({ ok: true, body: quotedPost })
        : fetchPost(quotedTopicId, quotedPostNumber);
      load
        .then(function (result) {
          if (current !== request) {
            return;
          }
          if (!result.ok) {
            blockquote.replaceChildren(errorContent(result.status, errorMessage(result)));
            return;
          }
          quotedPost = result.body;
          var wrapper = document.createElement("div");
          wrapper.className = "expanded-quote";
          wrapper.dataset.postId = quotedPost.id;
          var cooked = document.createElement("div");
          cooked.className = "post__contents-cooked-quote";
          cooked.innerHTML = quotedPost.cooked || "";
          wrapper.appendChild(cooked);
          blockquote.replaceChildren(wrapper);
          highlightHTML(cooked, originalText);
          Discourse.decorateCooked.apply(cooked);
        })
        .catch(function () {
          if (current === request) {
            blockquote.replaceChildren(errorContent(0, t("generic_error")));
          }
        });
    }

    function render() {
      aside.dataset.expanded = expanded ? "true" : "false";
      if (toggle) {
        var label = t(expanded ? "post.collapse" : "expand");
        toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
        toggle.setAttribute("aria-label", label);
        toggle.title = label;
        toggle.innerHTML = Discourse.cooked.icon(expanded ? "chevron-up" : "chevron-down");
      }
      if (expanded) {
        showExpanded();
      } else {
        request++;
        if (ignored) {
          blockquote.replaceChildren();
        } else {
          blockquote.replaceChildren(collapsed);
        }
      }
    }

    function toggleExpanded() {
      expanded = !expanded;
      render();
    }

    render();
  }

  Discourse.decorateCooked("quote-controls", function (el) {
    // A post of the topic's stream, whose model the quotes need.
    var article = el.closest("article[data-post-id]");
    var topic = currentTopic();
    if (!article || !topic) {
      return;
    }
    var wrapper = article.closest("[data-post-number]");
    var post = {
      topicId: topic.id,
      slug: topic.slug,
      postNumber: wrapper ? parseInt(wrapper.dataset.postNumber, 10) : null,
    };
    el.querySelectorAll("aside.quote").forEach(function (aside, index) {
      decorateQuote(aside, index, post);
    });
  });
})();
