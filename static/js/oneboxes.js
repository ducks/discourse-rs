// The composer preview's oneboxes, as Ember loads them
// (lib/load-oneboxes.js, pretty-text's oneboxer.js and inline-oneboxer.js):
//
//   Discourse.oneboxes.load(container, opts)
//       Each `a.onebox` within max_oneboxes_per_post is fetched from
//       /onebox, one request at a time 150ms apart (2s after a 429), and
//       put in place of its link; each `a.inline-onebox-loading` gets its
//       title from /inline-onebox in batches of 10. opts: topicId,
//       categoryId, maxOneboxes, refresh.
//   Discourse.oneboxes.applyCached(container)
//       What the preview's markdown rules take from the client's caches:
//       loaded oneboxes in place of their links, inline titles.
//
// Rendered previews come back with plain links each time, so the cached
// results are applied before loading the rest.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.oneboxes) {
    return;
  }

  var LOADING = "loading-onebox";
  var localCache = {};
  var failedCache = {};
  var inlineCache = {};
  var queue = [];
  var timer = null;

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  // oneboxer-cache's normalize: no trailing slash.
  function normalize(url) {
    return url.replace(/\/$/, "");
  }

  function query(params) {
    var parts = [];
    Object.keys(params).forEach(function (k) {
      var v = params[k];
      if (v === undefined || v === null) {
        return;
      }
      if (Array.isArray(v)) {
        v.forEach(function (item) {
          parts.push(encodeURIComponent(k) + "[]=" + encodeURIComponent(item));
        });
      } else {
        parts.push(encodeURIComponent(k) + "=" + encodeURIComponent(v));
      }
    });
    return parts.join("&");
  }

  function get(path, params, accept) {
    return fetch(basePath() + path + "?" + query(params), {
      credentials: "same-origin",
      headers: { Accept: accept, "X-Requested-With": "XMLHttpRequest" },
    });
  }

  function fromHtml(html) {
    var template = document.createElement("template");
    template.innerHTML = html.trim();
    return template.content.firstChild;
  }

  // applySquareGenericOnebox: a square thumbnail shown as an avatar.
  function resolveSize(img) {
    img.classList.add("size-resolved");
    if (img.width > 0 && img.width === img.height) {
      img.classList.add("onebox-avatar");
    }
  }

  function applySquareGenericOnebox(elem) {
    if (!elem.classList || !elem.classList.contains("allowlistedgeneric")) {
      return;
    }
    var img = elem.querySelector(".onebox-body img.thumbnail");
    if (!img || img.classList.contains("size-resolved")) {
      return;
    }
    if (img.complete) {
      resolveSize(img);
    } else {
      img.addEventListener("load", function onLoad() {
        resolveSize(img);
        img.removeEventListener("load", onLoad);
      });
    }
  }

  function loadNext() {
    if (queue.length === 0) {
      timer = null;
      return;
    }
    var wait = 150;
    var removeLoading = true;
    var item = queue.shift();
    get(
      "/onebox",
      { url: item.url, refresh: item.refresh, category_id: item.categoryId, topic_id: item.topicId },
      "text/html"
    )
      .then(function (r) {
        if (r.status === 429) {
          wait = 2000;
          removeLoading = false;
          queue.unshift(item);
          return null;
        }
        if (!r.ok) {
          failedCache[normalize(item.url)] = true;
          return null;
        }
        return r.text().then(function (html) {
          var node = fromHtml(html);
          if (!node) {
            return;
          }
          localCache[normalize(item.url)] = node;
          item.elem.replaceWith(node);
          applySquareGenericOnebox(node);
        });
      })
      .catch(function () {
        failedCache[normalize(item.url)] = true;
      })
      .finally(function () {
        timer = setTimeout(loadNext, wait);
        if (removeLoading) {
          item.elem.classList.remove(LOADING);
          item.elem.dataset.oneboxLoaded = "";
        }
      });
  }

  // oneboxer's load
  function load(elem, opts) {
    if (elem.dataset.oneboxLoaded !== undefined || elem.classList.contains(LOADING)) {
      return;
    }
    var url = elem.href;
    if (!opts.refresh) {
      if (localCache[normalize(url)]) {
        return;
      }
      if (failedCache[normalize(url)]) {
        return;
      }
    }
    elem.classList.add(LOADING);
    queue.push({
      url: url,
      refresh: opts.refresh,
      elem: elem,
      categoryId: opts.categoryId,
      topicId: opts.topicId,
    });
    timer = timer || setTimeout(loadNext, 150);
  }

  // inline-oneboxer's applyInlineOneboxes, in batches of 10.
  function applyInline(inline, opts) {
    var urls = Object.keys(inline).filter(function (url) {
      return !inlineCache[url];
    });
    urls.forEach(function (url) {
      inlineCache[url] = {};
    });
    var batches = [];
    for (var i = 0; i < urls.length; i += 10) {
      batches.push(urls.slice(i, i + 10));
    }
    batches.reduce(function (done, batch, index) {
      return done.then(function (stop) {
        if (stop) {
          return true;
        }
        return get(
          "/inline-onebox",
          { urls: batch, category_id: opts.categoryId, topic_id: opts.topicId },
          "application/json"
        ).then(function (r) {
          if (r.status === 429) {
            urls.slice(index * 10).forEach(function (url) {
              delete inlineCache[url];
            });
            return true;
          }
          if (!r.ok) {
            return false;
          }
          return r.json().then(function (result) {
            (result["inline-oneboxes"] || []).forEach(function (onebox) {
              if (!onebox.title) {
                return;
              }
              inlineCache[onebox.url] = onebox;
              (inline[onebox.url] || []).forEach(function (link) {
                link.innerText = onebox.title;
                link.classList.add("inline-onebox");
                if (onebox.css_class) {
                  link.classList.add(onebox.css_class);
                }
                link.classList.remove("inline-onebox-loading");
              });
            });
            return false;
          });
        });
      });
    }, Promise.resolve(false));
  }

  function applyCached(container) {
    container.querySelectorAll("a.onebox").forEach(function (link) {
      var cached = localCache[normalize(link.href)];
      if (cached) {
        link.replaceWith(cached.cloneNode(true));
      }
    });
    container.querySelectorAll("a.inline-onebox-loading").forEach(function (link) {
      var cached = inlineCache[link.href];
      if (cached && cached.title) {
        link.innerText = cached.title;
        link.classList.add("inline-onebox");
        link.classList.remove("inline-onebox-loading");
      }
    });
  }

  // loadOneboxes
  function loadOneboxes(container, opts) {
    var oneboxes = {};
    var inline = {};
    var loaded = container.querySelectorAll(
      "aside.onebox, a." + LOADING + ", a.inline-onebox"
    ).length;
    container.querySelectorAll("a.onebox, a.inline-onebox-loading").forEach(function (link) {
      var isInline = link.classList.contains("inline-onebox-loading");
      var map = isInline ? inline : oneboxes;
      if (loaded < opts.maxOneboxes) {
        if (map[link.href] === undefined) {
          map[link.href] = [];
          loaded++;
        }
        map[link.href].push(link);
      } else if (map[link.href] !== undefined) {
        map[link.href].push(link);
      } else if (isInline) {
        link.classList.remove("inline-onebox-loading");
      }
    });
    Object.keys(oneboxes).forEach(function (url) {
      oneboxes[url].forEach(function (elem) {
        load(elem, opts);
      });
    });
    if (Object.keys(inline).length > 0) {
      // _loadInlineOneboxes passes the category as the topic and the
      // topic as the category, as Ember does.
      applyInline(inline, { categoryId: opts.topicId, topicId: opts.categoryId });
    }
    return Object.keys(oneboxes).length + Object.keys(inline).length;
  }

  Discourse.oneboxes = { load: loadOneboxes, applyCached: applyCached };
})();
