// Editing a topic's title, category and tags in place, for who may edit
// the topic (details.can_edit: the title carries the edit pencil and
// script.topic-title-editor-config, html.rs topic_title_editor_config).
// As Ember does it:
//
//   templates/topic.gjs            the title's click (handleTitleClick:
//                                  editTopic, then jumpTop) swaps the h1
//                                  and the category for .edit-topic-title
//   components/topic-title         Escape cancels, Ctrl+Enter or Cmd+Enter
//                                  saves (a plain Enter does nothing)
//   components/topic-title-editor  #edit-title, focused
//   components/topic-metadata      the CategoryChooser (not a message's),
//                                  the MiniTagChooser (canEditTags), the
//                                  save and cancel buttons
//   controllers/topic.js           the buffered title, category_id and
//                                  tags; finishedEditingTopic sends what
//                                  changed (Topic.update with fastEdit),
//                                  popupAjaxError on a failure, the editor
//                                  left open
//
// The choosers are copies of the composer's (topic-admin-modals.js's
// categoryChooser, composer.js's miniTagChooser.copy). Once saved the
// title is the answer's fancy_title at once, as Topic.update sets it,
// and the rest of what the topic's reload draws again (the category, the
// tags, the body's classes, the document's title, the docked header's
// topic) comes from the page asked for again; the address takes the new
// slug. A live redraw of the title (routes/live.rs render_title) waits
// while the editor is open, as the buffer outlives Ember's reload.
//
// Not ported: the edit-topic-title, edit-topic-category, edit-topic-tags
// and edit-topic plugin outlets, the featured link's removal, editing a
// localization (editingTopicLocalization), MiniTagChooser's
// useHeaderFilter (the chooser is the composer's, its filter in its
// body).
(function () {
  "use strict";

  if (!Discourse.once("topic-title-editor")) {
    return;
  }

  // The open editor: its element, the nodes it took the place of, the
  // buffer, and the title a live redraw brought meanwhile.
  var editing = null;

  function container() {
    return document.getElementById("topic-title");
  }

  function config() {
    var el = document.querySelector("#topic-title script.topic-title-editor-config");
    try {
      return el ? JSON.parse(el.textContent) : null;
    } catch (_) {
      return null;
    }
  }

  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (_) {
      return {};
    }
  }

  function icon(name, extraClass) {
    return Discourse.cooked ? Discourse.cooked.icon(name, extraClass) : "";
  }

  function t(cfg, key, args) {
    var s = (cfg.strings || {})[key] || "";
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  // lib/intercept-click.js wantsNewWindow()
  function wantsNewWindow(event) {
    return event.shiftKey || event.metaKey || event.ctrlKey || (event.button && event.button !== 0);
  }

  // DButton, an icon alone.
  function button(className, label, iconName) {
    var b = document.createElement("button");
    b.className = "btn no-text btn-icon " + className;
    b.type = "button";
    b.setAttribute("aria-label", label);
    b.innerHTML = icon(iconName) + '<span aria-hidden="true">&#8203;</span>';
    return b;
  }

  // The title block as the server draws it now, or kept for when the
  // open editor closes.
  function setTitle(html) {
    if (editing) {
      editing.incoming = html;
      return;
    }
    var el = container();
    if (!el) {
      return;
    }
    el.innerHTML = html;
    if (window.htmx) {
      window.htmx.process(el);
    }
  }

  function close() {
    var state = editing;
    if (!state) {
      return;
    }
    editing = null;
    state.root.remove();
    state.shown.forEach(function (node) {
      state.wrapper.appendChild(node);
    });
    if (state.incoming !== null) {
      setTitle(state.incoming);
    }
  }

  // The category's minimumRequiredTags, which the composer's rows carry.
  function minimumRequiredTags(state) {
    var row =
      state.categoryChooser && state.categoryId
        ? state.categoryChooser.querySelector('.category-row[data-value="' + Number(state.categoryId) + '"]')
        : null;
    return row ? parseInt(row.dataset.minimumRequiredTags || "0", 10) : 0;
  }

  // TopicMetadata's choosers: showCategoryChooser (not a message) and
  // canEditTags (site.can_tag_topics, which the composer's chooser stands
  // for, and site.can_tag_pms for a message).
  function metadata(state, cfg, root) {
    var modals = Discourse.topicAdminModals;
    if (!cfg.private_message && modals) {
      var category = modals.categoryChooser(
        cfg,
        function (id) {
          // topicCategoryChanged
          state.categoryId = id;
          if (state.tagChooser) {
            state.tagChooser.header();
          }
        },
        { value: cfg.category_id }
      );
      if (category) {
        category.classList.add("small");
        state.categoryChooser = category;
        var categoryWrapper = document.createElement("div");
        categoryWrapper.className = "edit-category__wrapper";
        categoryWrapper.appendChild(category);
        root.appendChild(categoryWrapper);
      }
    }
    var composer = document.getElementById("reply-control");
    var canTag = !cfg.private_message || (composer && composer.dataset.canTagPms === "true");
    var details = canTag && Discourse.miniTagChooser ? Discourse.miniTagChooser.copy() : null;
    if (details) {
      var chooser = Discourse.miniTagChooser(details, {
        categoryId: function () {
          return state.categoryId ? String(state.categoryId) : "";
        },
        minimum: function () {
          return minimumRequiredTags(state);
        },
        prioritizeRecent: function () {
          return chooser ? chooser.options.prioritizeRecent : false;
        },
        // topicTagsChanged
        changed: function () {
          state.tags = chooser.selected().slice();
        },
        opened: function () {
          if (modals) {
            modals.floatBody(details);
          }
        },
      });
      state.tagChooser = chooser;
      chooser.set(
        (cfg.tags || []).map(function (tag) {
          return typeof tag === "string" ? { name: tag } : { id: tag.id, name: tag.name };
        })
      );
      var tagsWrapper = document.createElement("div");
      tagsWrapper.className = "edit-tags__wrapper";
      tagsWrapper.appendChild(details);
      root.appendChild(tagsWrapper);
    }
  }

  // editTopic: editingTopic for who may edit the title.
  function open() {
    var cfg = config();
    var el = container();
    var wrapper = el && el.querySelector(".title-wrapper");
    if (editing || !cfg || !wrapper) {
      return;
    }
    var root = document.createElement("div");
    root.className = "edit-topic-title";
    if (cfg.private_message) {
      root.innerHTML =
        '<span class="private-message-glyph-wrapper">' + icon("envelope", "private-message-glyph") + "</span>";
    }
    var titleWrapper = document.createElement("div");
    titleWrapper.className = "edit-title__wrapper";
    // DTextField
    var input = document.createElement("input");
    input.id = "edit-title";
    input.className = "ember-text-field";
    input.type = "text";
    input.autofocus = true;
    input.maxLength = cfg.max_topic_title_length;
    input.value = cfg.title;
    if (cfg.support_mixed_text_direction) {
      var syncDir = function () {
        input.dir = input.value ? "auto" : document.documentElement.classList.contains("rtl") ? "rtl" : "ltr";
      };
      input.addEventListener("input", syncDir);
      syncDir();
    }
    titleWrapper.appendChild(input);
    root.appendChild(titleWrapper);

    var state = {
      cfg: cfg,
      root: root,
      wrapper: wrapper,
      shown: Array.prototype.slice.call(wrapper.childNodes),
      input: input,
      categoryId: cfg.category_id,
      tags: null,
      categoryChooser: null,
      tagChooser: null,
      incoming: null,
      saving: false,
    };
    metadata(state, cfg, root);

    var controls = document.createElement("div");
    controls.className = "edit-controls";
    var submit = button("btn-primary submit-edit", t(cfg, "composer.save_edit"), "check");
    var cancel = button("btn-default cancel-edit", t(cfg, "composer.cancel"), "xmark");
    submit.addEventListener("click", save);
    cancel.addEventListener("click", close);
    controls.appendChild(submit);
    controls.appendChild(cancel);
    root.appendChild(controls);

    state.shown.forEach(function (node) {
      node.remove();
    });
    wrapper.appendChild(root);
    editing = state;
    input.focus();
  }

  // ajax-error's extractError for a failed response.
  function extractError(cfg, response) {
    if (!(response instanceof Response)) {
      return Promise.resolve(t(cfg, "generic_error"));
    }
    return response
      .json()
      .catch(function () {
        return null;
      })
      .then(function (json) {
        var errors = json && json.errors;
        if (errors && errors.length > 1) {
          return t(cfg, "multiple_errors", {
            errors: errors
              .map(function (e, i) {
                return i + 1 + ") " + e;
              })
              .join(" "),
          });
        }
        if (errors && errors.length) {
          return t(cfg, "generic_error_with_reason", { error: errors[0] });
        }
        return t(cfg, "generic_error");
      });
  }

  // The topic's reload, for what the answer does not carry: the title
  // block, the body's category and tag classes, the document's title and
  // the docked header's topic (header:update-topic).
  function reload() {
    var url = location.href;
    return fetch(url, {
      credentials: "same-origin",
      cache: "no-store",
      headers: { "HX-Request": "true", "HX-Boosted": "true", "HX-Current-URL": url },
    })
      .then(function (response) {
        if (!response.ok) {
          throw new Error("reload " + response.status);
        }
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var title = doc.getElementById("topic-title");
        var heading = title && title.querySelector("h1[data-topic-id]");
        var topic = document.getElementById("topic");
        if (location.href !== url || !heading || !topic || heading.dataset.topicId !== topic.dataset.topicId) {
          return;
        }
        setTitle(title.innerHTML);
        if (doc.title) {
          document.title = doc.title;
        }
        var classes = document.body.classList;
        var stale = /^(category|category-.*|tag-.*)$/;
        Array.prototype.slice.call(classes).forEach(function (name) {
          if (stale.test(name)) {
            classes.remove(name);
          }
        });
        Array.prototype.slice.call(doc.body.classList).forEach(function (name) {
          if (stale.test(name)) {
            classes.add(name);
          }
        });
        var info = document.getElementById("header-topic-info");
        var fresh = doc.getElementById("header-topic-info");
        if (info && fresh) {
          info.innerHTML = fresh.innerHTML;
          document.dispatchEvent(new CustomEvent("discourse:header-update-topic"));
        }
      })
      .catch(function (error) {
        console.error(error);
      });
  }

  // Topic.update's then: the title as the server cleaned it up, the
  // pencil after it, and the address with the topic's slug.
  function saved(cfg, result) {
    var topic = (result && result.basic_topic) || {};
    close();
    var link = document.querySelector("#topic-title h1 a.fancy-title");
    if (link && typeof topic.fancy_title === "string") {
      var pencil = link.querySelector(".edit-topic__wrapper");
      link.innerHTML = topic.fancy_title;
      if (pencil) {
        link.appendChild(pencil);
      }
    }
    var path = /^(.*\/t\/)[^/]+(\/\d+(?:\/\d+)?\/?)$/.exec(location.pathname);
    if (topic.slug && path && path[1] + topic.slug + path[2] !== location.pathname) {
      history.replaceState(history.state, "", path[1] + topic.slug + path[2] + location.search + location.hash);
    }
    reload();
  }

  // finishedEditingTopic: the buffer (what differs from the topic) through
  // Topic.update, a message's category never changed, the tags as
  // serializeTags sends them.
  function save() {
    var state = editing;
    if (!state || state.saving) {
      return;
    }
    var cfg = state.cfg;
    var data = {};
    if (state.input.value !== cfg.title) {
      data.title = state.input.value;
    }
    if (!cfg.private_message && state.categoryId !== cfg.category_id) {
      data.category_id = state.categoryId;
    }
    if (state.tags) {
      data.tags = state.tags.map(function (tag) {
        return tag.isNew || !tag.id ? { name: tag.name } : { id: tag.id, name: tag.name };
      });
    }
    data.keep_existing_draft = true;
    var topic = /^(.*\/t\/[^/]+\/\d+)(?:\/\d+)?\/?$/.exec(location.pathname);
    if (!topic) {
      return;
    }
    var headers = memberHeaders();
    headers.Accept = "application/json";
    headers["Content-Type"] = "application/json";
    state.saving = true;
    fetch(topic[1], {
      method: "PUT",
      headers: headers,
      credentials: "same-origin",
      body: JSON.stringify(data),
    })
      .then(function (response) {
        if (!response.ok) {
          return Promise.reject(response);
        }
        return response.json();
      })
      .then(
        function (result) {
          state.saving = false;
          if (editing === state) {
            saved(cfg, result);
          }
        },
        function (response) {
          state.saving = false;
          // popupAjaxError
          return extractError(cfg, response).then(function (message) {
            if (Discourse.dialog) {
              Discourse.dialog.popupAjaxError(message);
            } else {
              window.alert(message);
            }
          });
        }
      );
  }

  // handleTitleClick, ahead of the boosted link: the editor for who may
  // edit (editTopic prevents the link's own), then jumpTop, the first
  // post when the page has it.
  document.addEventListener(
    "click",
    function (event) {
      var link = event.target.closest && event.target.closest("#topic-title h1 a.fancy-title");
      if (!link || !config()) {
        return;
      }
      var newWindow = wantsNewWindow(event);
      event.preventDefault();
      event.stopPropagation();
      var href = link.getAttribute("href");
      open();
      if (!newWindow && Discourse.topicRoute && document.getElementById("post_1")) {
        Discourse.topicRoute.routeToPost(href + "/1", { skipIfOnScreen: false, highlight: false });
        if (editing) {
          editing.input.focus({ preventScroll: true });
        }
      }
    },
    true
  );

  // TopicTitle#keyDown. An open chooser takes its own keys (select-kit
  // stops them), its Escape closing it.
  document.addEventListener("keydown", function (event) {
    if (!editing || event.defaultPrevented || !event.target.closest || !event.target.closest("#topic-title")) {
      return;
    }
    if (event.key === "Escape") {
      var chooser = event.target.closest("details[open]");
      if (chooser) {
        chooser.open = false;
        return;
      }
      event.preventDefault();
      close();
    } else if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      save();
    }
  });

  // The live stream's redraw of the title waits for the editor to close.
  document.addEventListener("htmx:oobBeforeSwap", function (event) {
    var detail = event.detail;
    if (!editing || !detail || !detail.target || detail.target.id !== "topic-title") {
      return;
    }
    var incoming = detail.fragment && detail.fragment.querySelector("#topic-title");
    if (incoming) {
      editing.incoming = incoming.innerHTML;
    }
    detail.shouldSwap = false;
  });

  // Another page: nothing is being edited.
  Discourse.onPage(function () {
    editing = null;
  });
})();
