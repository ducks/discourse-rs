// Hashtags as Ember's hashtag types handle them (lib/hashtag-types,
// lib/hashtag-decorator.js), for what the server leaves to the client:
//
//   Discourse.hashtags.resolve(el, order, rerender)
//       linkSeenHashtagsInContext: each span.hashtag-raw in el whose slug
//       was looked up becomes its hashtag link; the slugs not looked up
//       yet are fetched from /hashtags (fetchUnseenHashtagsInContext),
//       then rerender runs.
//   Discourse.hashtags.decorate(el)
//       decorateHashtags: each a.hashtag-cooked's icon placeholder becomes
//       its type's icon, and the link gets an aria-label.
//
// Posts come decorated from the server; a channel hashtag in one still
// loads its color here, as generateIconHTML does. The color classes go to
// the #hashtag-css-generator style the page carries, which also gives the
// categories already there (data-category-ids) and how emoji and the tag
// icon look (data-emoji-base, data-emoji-version, data-tag-icon).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.hashtags) {
    return;
  }

  function style() {
    return document.getElementById("hashtag-css-generator");
  }

  function config(name) {
    var el = style();
    return (el && el.dataset[name]) || "";
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function escapeHtml(s) {
    return String(s === undefined || s === null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#x27;");
  }

  // iconHTML
  function iconHtml(name, extraClass) {
    return (
      '<svg class="fa d-icon d-icon-' + escapeHtml(name) + " svg-icon fa-width-auto" +
      (extraClass ? " " + escapeHtml(extraClass) : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      escapeHtml(name) + '"></use></svg>'
    );
  }

  // dReplaceEmoji(":name:")
  function emojiHtml(name) {
    var path = name.replace(/:t([1-6])$/, "/$1");
    return (
      '<img width="20" height="20" src="' + escapeHtml(config("emojiBase") + "/" + path + ".png?v=" + config("emojiVersion")) +
      '" title="' + escapeHtml(name) + '" alt="' + escapeHtml(name) + '" class="emoji">'
    );
  }

  // The types' loaded ids: the site's categories come with the page
  // (CategoryHashtagType counts every category loaded without
  // lazy_load_categories), tags preload none.
  var loaded = { category: {}, tag: {}, channel: {} };
  config("categoryIds")
    .split(",")
    .filter(Boolean)
    .forEach(function (id) {
      loaded.category[id] = true;
    });

  // generateColorCssClasses
  function colorCss(type, item) {
    var colors = item.colors || [];
    if (type === "category") {
      var color = colors.length === 1 ? colors[0] : colors[1];
      var parent = colors.length === 1 ? null : colors[0];
      var rule;
      if (item.style_type === "square") {
        rule = parent
          ? "background: linear-gradient(-90deg, #" + color + " 50%, #" + parent + " 50%);"
          : "background-color: #" + color + ";";
      } else if (item.style_type === "icon") {
        rule = "color: #" + color + ";";
      } else {
        return [];
      }
      return [".hashtag-color--category-" + item.id + " { " + rule + " }"];
    }
    if (type === "channel") {
      return [".d-icon.hashtag-color--channel-" + item.id + " { color: #" + colors[0] + " }"];
    }
    return [];
  }

  // onLoad: the hashtag's color classes, once.
  function onLoad(type, item) {
    var id = parseInt(item.id, 10);
    if (loaded[type][id]) {
      return;
    }
    loaded[type][id] = true;
    var el = style();
    if (el) {
      el.innerHTML += "\n" + colorCss(type, item).join("\n");
    }
  }

  // load: the ids not loaded yet, fetched together from /hashtags/by-ids
  // after 100ms.
  var loading = {};
  var loadTimer = null;
  function load(type, id) {
    id = parseInt(id, 10);
    if (type === "category" || loaded[type][id]) {
      return;
    }
    (loading[type] = loading[type] || {})[id] = true;
    clearTimeout(loadTimer);
    loadTimer = setTimeout(function () {
      var data = loading;
      loading = {};
      var query = [];
      Object.keys(data).forEach(function (t) {
        Object.keys(data[t]).forEach(function (i) {
          query.push(encodeURIComponent(t) + "[]=" + encodeURIComponent(i));
        });
      });
      if (!query.length) {
        return;
      }
      fetch(basePath() + "/hashtags/by-ids?" + query.join("&"), {
        credentials: "same-origin",
        headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
      })
        .then(function (r) {
          return r.ok ? r.json() : {};
        })
        .then(function (hashtags) {
          Object.keys(hashtags).forEach(function (t) {
            (hashtags[t] || []).forEach(function (item) {
              if (loaded[t]) {
                onLoad(t, item);
              }
            });
          });
        });
    }, 100);
  }

  // generateIconHTML
  function generateIconHtml(type, hashtag) {
    if (type === "category") {
      if (hashtag.preloaded) {
        onLoad(type, hashtag);
      } else {
        load(type, hashtag.id);
      }
      var inner = "";
      if (hashtag.style_type === "icon" && hashtag.icon) {
        inner = iconHtml(hashtag.icon);
      }
      if (hashtag.style_type === "emoji" && hashtag.emoji) {
        inner = emojiHtml(hashtag.emoji);
      }
      return (
        '<span class="hashtag-category-' + escapeHtml(hashtag.style_type) +
        " hashtag-color--category-" + escapeHtml(hashtag.id) + '">' + inner + "</span>"
      );
    }
    if (type === "tag") {
      return iconHtml(hashtag.icon, "hashtag-color--tag-" + hashtag.id);
    }
    if (type === "channel") {
      if (hashtag.colors) {
        onLoad(type, hashtag);
      } else {
        load(type, hashtag.id);
      }
      return hashtag.emoji
        ? emojiHtml(hashtag.emoji)
        : iconHtml(hashtag.icon, "hashtag-color--channel-" + hashtag.id);
    }
    return null;
  }

  function fromHtml(html) {
    var template = document.createElement("template");
    template.innerHTML = html.trim();
    return template.content.firstChild;
  }

  // decorateHashtags
  function decorate(el) {
    el.querySelectorAll(".hashtag-cooked").forEach(function (link) {
      var placeholder = link.querySelector(".hashtag-icon-placeholder");
      var type = link.dataset.type;
      if (placeholder && loaded[type]) {
        var styleType = ["square", "icon", "emoji"].indexOf(link.dataset.styleType) >= 0
          ? link.dataset.styleType
          : "square";
        var opts = {
          icon: type === "tag" ? config("tagIcon") : type === "channel" ? "comment" : "folder",
          id: parseInt(link.dataset.id, 10),
          slug: link.dataset.slug,
          style_type: styleType,
        };
        if (styleType === "icon") {
          opts.icon = link.dataset.icon;
        }
        if (styleType === "emoji") {
          opts.emoji = link.dataset.emoji;
        }
        var html = generateIconHtml(type, opts);
        if (html) {
          placeholder.replaceWith(fromHtml(html));
        }
      } else if (type === "channel") {
        load(type, link.dataset.id);
      }
      link.setAttribute("aria-label", link.innerText.trim());
    });
  }

  // generatePlaceholderHashtagHTML
  function link(type, span, data) {
    var a = document.createElement("a");
    a.classList.add("hashtag-cooked");
    a.href = data.relative_url;
    a.dataset.type = type;
    a.dataset.id = data.id;
    a.dataset.slug = data.slug;
    a.dataset.styleType = data.style_type;
    if (data.style_type === "icon") {
      a.dataset.icon = data.icon;
    }
    if (data.style_type === "emoji") {
      a.dataset.emoji = data.emoji;
    }
    a.innerHTML = (generateIconHtml(type, data) || "") + "<span>" + escapeHtml(data.text) + "</span>";
    span.replaceWith(a);
  }

  var seen = {};
  var checked = {};

  // linkSeenHashtagsInContext, then fetchUnseenHashtagsInContext for the
  // rest.
  function resolve(el, order, rerender) {
    var spans = Array.prototype.slice.call(el.querySelectorAll("span.hashtag-raw"));
    if (!spans.length) {
      return;
    }
    var slugs = spans.map(function (span) {
      return span.innerText.replace(/^#/, "");
    });
    spans.forEach(function (span, i) {
      order.forEach(function (type) {
        var found = seen[type] && seen[type][slugs[i]];
        if (found && span.parentNode) {
          var data = Object.assign({ preloaded: true }, found);
          link(type, span, data);
        }
      });
    });
    var unseen = slugs
      .map(function (s) {
        return s.toLowerCase();
      })
      .filter(function (s, i, all) {
        return all.indexOf(s) === i && !checked[s];
      });
    if (!unseen.length) {
      return;
    }
    var query = unseen
      .map(function (s) {
        return "slugs[]=" + encodeURIComponent(s);
      })
      .concat(
        order.map(function (t) {
          return "order[]=" + encodeURIComponent(t);
        })
      );
    fetch(basePath() + "/hashtags?" + query.join("&"), {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        return r.ok ? r.json() : null;
      })
      .then(function (response) {
        if (!response) {
          return;
        }
        Object.keys(response).forEach(function (type) {
          seen[type] = seen[type] || {};
          response[type].forEach(function (item) {
            seen[type][item.ref] = seen[type][item.ref] || item;
          });
        });
        unseen.forEach(function (s) {
          checked[s] = true;
        });
        if (rerender) {
          rerender();
        }
      });
  }

  Discourse.hashtags = { resolve: resolve, decorate: decorate };

  // A post's channel hashtags load their colors.
  if (Discourse.onPage) {
    Discourse.onPage(function () {
      document.querySelectorAll('.cooked a.hashtag-cooked[data-type="channel"]').forEach(function (a) {
        load("channel", a.dataset.id);
      });
    });
  }
})();
