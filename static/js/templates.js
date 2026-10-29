// discourse-templates in the browser, for a member who can_use_templates
// (the d-templates and d-templates-modal services, the initializer's
// options menu item, keyboard shortcut and chat composer button, and the
// topic's template controls).
//
// The filterable list is drawn by the server (GET /live/templates, with
// the insert's context: the composer's topic and the post it replies to
// or edits, or the chat channel and the message replied to), which also
// replaces the template variables; htmx asks for it again as the keyword
// or the tag changes. Here: where the list shows (the insert template
// modal, or the composer preview's panel on a desktop with the preview
// shown), the insert an item's button makes, its POST
// /discourse_templates/:id/use, and the tag kept in the keyValueStore.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.once && !Discourse.once("templates")) {
    return;
  }
  var script = document.currentScript;
  var labels = {
    insert: script ? script.dataset.labelInsert : "Insert template",
    close: script ? script.dataset.labelClose : "Close the templates selector",
  };
  // FilterableList's PREV_TEMPLATE_TAG_ID, in the keyValueStore.
  var TAG_KEY = "discourse_template-selected-tag";
  var ALL_TAGS_ID = "all-tags";
  var apple = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent);

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function storedTag() {
    try {
      return window.localStorage.getItem(TAG_KEY);
    } catch (e) {
      return null;
    }
  }

  function storeTag(tag) {
    try {
      if (tag && tag !== ALL_TAGS_ID) {
        window.localStorage.setItem(TAG_KEY, tag);
      } else {
        window.localStorage.removeItem(TAG_KEY);
      }
    } catch (e) {
      // Storage blocked: the tag lasts while the list is open.
    }
  }

  function headers() {
    var out = {};
    try {
      out = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    out["X-Requested-With"] = "XMLHttpRequest";
    out.Accept = "application/json";
    return out;
  }

  function alertError(message) {
    if (Discourse.dialog && Discourse.dialog.alert) {
      Discourse.dialog.alert(message);
    } else {
      window.alert(message);
    }
  }

  // popupAjaxError
  function failed(response) {
    return response
      .json()
      .then(function (body) {
        alertError((body.errors || [response.statusText]).join("\n"));
      })
      .catch(function () {
        alertError(response.statusText);
      });
  }

  function icon(name) {
    return Discourse.cooked && Discourse.cooked.icon ? Discourse.cooked.icon(name) : "";
  }

  // The list for a context, into `el`; `onInsert(title, content)` takes
  // the picked template, then `after()` (closing what showed it).
  function load(el, context, onInsert, after) {
    el.innerHTML = '<div class="templates-filterable-list"><div class="spinner"></div></div>';
    var params = new URLSearchParams(context || {});
    var tag = storedTag();
    if (tag) {
      params.set("tag", tag);
    }
    fetch(basePath() + "/live/templates?" + params.toString(), { credentials: "same-origin" })
      .then(function (response) {
        if (!response.ok) {
          el.innerHTML = '<div class="templates-filterable-list"></div>';
          return failed(response);
        }
        return response.text().then(function (html) {
          el.innerHTML = html;
          var list = el.querySelector(".templates-filterable-list");
          if (list) {
            storeTag(list.dataset.selectedTag);
          }
          if (window.htmx) {
            window.htmx.process(el);
          }
          var filter = el.querySelector(".templates-filter");
          if (filter) {
            filter.focus();
          }
        });
      })
      .catch(function () {
        alertError("Something went wrong.");
      });
    el.onclick = function (event) {
      var apply = event.target.closest(".templates-apply");
      if (!apply || !el.contains(apply)) {
        return;
      }
      event.preventDefault();
      onInsert(apply.dataset.title || "", apply.dataset.content || "");
      if (after) {
        after();
      }
      fetch(basePath() + "/discourse_templates/" + apply.dataset.templateId + "/use", {
        method: "POST",
        credentials: "same-origin",
        headers: headers(),
      }).then(function (response) {
        return response.ok ? undefined : failed(response);
      });
    };
  }

  // A tag picked in the list (its htmx swap): kept for the next time.
  document.addEventListener("htmx:afterSettle", function (event) {
    var row = event.detail && event.detail.elt;
    if (!row || !row.matches || !row.matches(".tag-drop .select-kit-row")) {
      return;
    }
    var list = document.querySelector(".d-templates .templates-filterable-list");
    if (list) {
      storeTag(list.dataset.selectedTag);
    }
  });

  // The tag drop's filter: the rows whose value or name hold it.
  document.addEventListener("input", function (event) {
    var input = event.target;
    if (!input.matches || !input.matches(".d-templates .tag-drop .filter-input")) {
      return;
    }
    var term = input.value.toLowerCase();
    input
      .closest(".tag-drop")
      .querySelectorAll(".select-kit-row")
      .forEach(function (row) {
        var value = (row.dataset.value || "").toLowerCase();
        var name = (row.dataset.name || "").toLowerCase();
        row.hidden = term !== "" && value.indexOf(term) === -1 && name.indexOf(term) === -1;
      });
  });

  // d-templates-modal-container: a DModal of its own, beside any other
  // modal (a textarea in one keeps it open underneath).
  var active = null;

  function closeModal() {
    if (!active) {
      return;
    }
    var current = active;
    active = null;
    document.documentElement.removeEventListener("keydown", current.onKeydown, { capture: true });
    current.container.remove();
    if (!document.querySelector(".d-modal")) {
      document.documentElement.classList.remove("modal-open");
    }
    // DTemplatesModalService#hide: the focus back on the textarea.
    if (current.textarea && current.textarea.isConnected) {
      current.textarea.focus();
    }
  }

  function showModal(textarea, context, onInsert) {
    closeModal();
    var container = document.createElement("div");
    container.className = "modal-container d-templates-modal-container";
    container.innerHTML =
      '<div aria-labelledby="discourse-modal-title" aria-modal="true" class="modal d-modal d-templates d-templates-modal" data-keyboard="false" role="dialog">' +
      '<div class="d-modal__container"><div class="d-modal__header"><div class="d-modal__title">' +
      '<h1 class="d-modal__title-text" id="discourse-modal-title"></h1></div>' +
      '<button class="btn no-text btn-icon btn-transparent modal-close" type="button">' + icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div>' +
      '<div class="d-modal__body" tabindex="-1"></div></div></div><div class="d-modal__backdrop"></div>';
    container.querySelector(".d-modal__title-text").textContent = labels.insert;
    var onKeydown = function (event) {
      if (event.key === "Escape" && active && active.container === container) {
        event.stopPropagation();
        closeModal();
      }
    };
    active = { container: container, textarea: textarea, onKeydown: onKeydown };
    container.querySelector(".modal-close").addEventListener("click", closeModal);
    container.querySelector(".d-modal__backdrop").addEventListener("click", closeModal);
    (document.querySelector(".discourse-root") || document.body).appendChild(container);
    document.documentElement.classList.add("modal-open");
    document.documentElement.addEventListener("keydown", onKeydown, { capture: true });
    load(container.querySelector(".d-modal__body"), context, onInsert, closeModal);
  }

  // The editor-preview connector: the list in the composer's preview
  // wrapper, the preview hidden while it shows.
  function hidePanel(wrapper) {
    var panel = wrapper.querySelector(".d-templates");
    if (panel) {
      panel.innerHTML = "";
    }
    var preview = wrapper.querySelector(".d-editor-preview");
    if (preview) {
      preview.style.display = "";
    }
  }

  function showPanel(wrapper, context, onInsert) {
    var panel = wrapper.querySelector(".d-templates");
    if (!panel) {
      panel = document.createElement("div");
      panel.className = "d-templates";
      wrapper.appendChild(panel);
    }
    var preview = wrapper.querySelector(".d-editor-preview");
    if (preview) {
      preview.style.display = "none";
    }
    panel.innerHTML =
      '<div class="d-templates-container"><button class="btn no-text btn-icon modal-close close btn-flat" type="button" title="' +
      labels.close.replace(/"/g, "&quot;") + '">' + icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button><div class="d-templates-list"></div></div>';
    panel.querySelector(".modal-close").addEventListener("click", function () {
      hidePanel(wrapper);
    });
    load(panel.querySelector(".d-templates-list"), context, onInsert, function () {
      hidePanel(wrapper);
    });
  }

  function composerContext(composer) {
    var c = composer.context() || {};
    var out = { context: "composer" };
    if (c.topicId) {
      out.topic_id = c.topicId;
    }
    if (c.postNumber) {
      out.post_number = c.postNumber;
    }
    return out;
  }

  // TextareaTextManipulation#insertBlock into any textarea, as an input
  // the page's own handlers see.
  function insertBlock(textarea, text) {
    text = (text || "").trim();
    if (!text) {
      return;
    }
    var value = textarea.value;
    var start = textarea.selectionStart;
    var end = textarea.selectionEnd;
    while (end > start && /\s/.test(value.charAt(end - 1))) {
      end--;
    }
    var pre = value.slice(0, start);
    var post = value.slice(end);
    start -= pre.match(/\n*$/)[0].length;
    if (pre.length > 0) {
      text = "\n\n" + text;
    }
    end += post.match(/^\n*/)[0].length;
    text = post.length > 0 ? text + "\n\n" : text + "\n";
    textarea.setRangeText(text, start, end, "end");
    textarea.focus();
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  }

  // The chat channel composer's context (extractVariablesFromChatChannel).
  function chatContext(textarea) {
    var channel = document.querySelector(".chat-channel[data-id]");
    if (!channel) {
      return {};
    }
    var out = { context: "chat", chat_channel_id: channel.dataset.id };
    var wrapper = textarea.closest(".chat-composer__wrapper");
    var details = wrapper && wrapper.querySelector('.chat-composer-message-details[data-action="reply"]');
    if (details && details.dataset.id) {
      out.chat_reply_to_id = details.dataset.id;
    }
    return out;
  }

  function isChatComposer(element) {
    return Boolean(element) && element.id === "channel-composer";
  }

  // DTemplatesService#showTextAreaUI
  function showTextAreaUI(textarea, context) {
    if (!textarea || textarea.nodeName !== "TEXTAREA") {
      return;
    }
    var modal = textarea.closest(".d-modal");
    if (modal && modal.classList.contains("d-templates")) {
      return;
    }
    showModal(textarea, context, function (title, content) {
      insertBlock(textarea, content);
    });
  }

  Discourse.templates = {
    // DTemplatesService#showComposerUI
    showComposerUI: function () {
      var composer = Discourse.composerTemplates;
      if (!composer) {
        return;
      }
      var onInsert = function (title, content) {
        composer.insert(title, content);
      };
      var mobile = document.documentElement.classList.contains("mobile-view");
      var wrapper = composer.previewWrapper();
      if (mobile || !composer.previewVisible() || !wrapper) {
        showModal(null, composerContext(composer), onInsert);
      } else {
        showPanel(wrapper, composerContext(composer), onInsert);
      }
    },
    showTextAreaUI: showTextAreaUI,
  };

  // The ${PLATFORM_KEY_MODIFIER}+shift+i shortcut, global: the composer's
  // UI while the composer has the focus, the chat composer's or any other
  // textarea's otherwise.
  document.addEventListener("keydown", function (event) {
    var modifier = apple ? event.metaKey : event.ctrlKey;
    if (!modifier || !event.shiftKey || event.altKey || event.code !== "KeyI") {
      return;
    }
    event.preventDefault();
    var composer = Discourse.composerTemplates;
    if (composer && composer.focused()) {
      Discourse.templates.showComposerUI();
      return;
    }
    var active = document.activeElement;
    if (!active || active.nodeName !== "TEXTAREA") {
      return;
    }
    showTextAreaUI(active, isChatComposer(active) ? chatContext(active) : {});
  });

  // The chat composer's dropdown button (d-templates-chat-insert-template-btn).
  Discourse.chatComposerButtons = Discourse.chatComposerButtons || [];
  if (script && script.dataset.chat === "true") {
    Discourse.chatComposerButtons.push({
      id: "d-templates-chat-insert-template-btn",
      icon: "far-clipboard",
      label: labels.insert,
      displayed: function (context, textarea) {
        return Boolean(textarea) && (context === "channel" || context === "thread");
      },
      action: function (context, textarea) {
        showTextAreaUI(textarea, context === "channel" ? chatContext(textarea) : {});
      },
    });
  }

  // The topic's template controls (discourse-templates-button.gjs): copy
  // the first post's raw (GET /raw/:topic_id/1), and a new topic from the
  // body, category and tags the server read from its options header.
  document.addEventListener("click", function (event) {
    var controls = event.target.closest(".template-topic-controls");
    if (!controls) {
      return;
    }
    if (event.target.closest(".template-copy")) {
      var button = event.target.closest(".template-copy");
      fetch(basePath() + "/raw/" + controls.dataset.topicId + "/1", { credentials: "same-origin" })
        .then(function (response) {
          return response.text();
        })
        .then(function (text) {
          return navigator.clipboard.writeText(text);
        })
        .then(function () {
          var svg = button.querySelector("svg use");
          button.classList.add("ok");
          if (svg) {
            svg.setAttribute("href", "#check");
          }
          setTimeout(function () {
            button.classList.remove("ok");
            if (svg) {
              svg.setAttribute("href", "#copy");
            }
          }, 2000);
        });
      return;
    }
    if (event.target.closest(".template-new-topic") && Discourse.composerOpen) {
      var tags = controls.dataset.tags ? controls.dataset.tags.split(",") : [];
      Discourse.composerOpen({
        mode: "create-topic",
        body: controls.dataset.body,
        category: controls.dataset.categoryId ? parseInt(controls.dataset.categoryId, 10) : undefined,
        tags: tags,
      });
    }
  });
})();
