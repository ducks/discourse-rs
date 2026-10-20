// The composer (#reply-control, rendered by composer_view): opens to
// reply to a topic or a post, create a topic or edit a post, and posts
// through /posts as Ember's composer does. Its height, sizing, minimizing
// and fullscreen follow services/composer.js and composer-container; the
// toolbar applies its markdown as d-editor's buttons do.
(function () {
  "use strict";

  var control = document.getElementById("reply-control");
  // Bound once per composer: it stays across pages (hx-preserve, page.js),
  // and its script tag comes back with each. A page without one drops it;
  // the next composer is bound afresh and this one's listeners go quiet.
  if (!control || control.dataset.bound) {
    return;
  }
  control.dataset.bound = "true";
  var root = document.documentElement;
  var textarea = control.querySelector(".d-editor-input");
  var title = control.querySelector("#reply-title");
  var fields = control.querySelector(".title-and-category");
  var chooser = control.querySelector(".category-chooser");
  var actionButton = control.querySelector(".composer-actions");
  var submit = control.querySelector(".save-or-cancel .create");
  var error = control.querySelector(".composer-error");
  var data = control.dataset;
  var state = null;
  var base = control.dataset.basePath || "";

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    return headers;
  }

  function getItem(key) {
    try {
      return window.localStorage.getItem("discourse_" + key);
    } catch (e) {
      return null;
    }
  }

  function setItem(key, value) {
    try {
      window.localStorage.setItem("discourse_" + key, value);
    } catch (e) {
      // Storage blocked: the height lasts for this page.
    }
  }

  // _getDefaultComposerHeight
  function defaultHeight(mode) {
    return (
      getItem("composerHeight") ||
      (mode === "reply"
        ? "var(--reply-composer-height, 255px)"
        : "var(--new-topic-composer-height, 400px)")
    );
  }

  // canEditTitle: creating a topic, or editing a first post (with its
  // title, category and tags above the editor).
  function setMode(mode, firstPost) {
    var titled = mode === "create-topic" || (mode === "edit" && !!firstPost);
    ["reply", "create-topic", "edit"].forEach(function (m) {
      control.classList.toggle("composer-action-" + m, m === mode);
    });
    control.classList.toggle("edit-title", titled);
    control.classList.toggle("topic", mode === "create-topic");
    fields.hidden = !titled;
    title.disabled = false;
    chooser.closest(".category-input").hidden = false;
    actionButton.innerHTML = data["action" + modeName(mode)];
    submit.innerHTML = data["submit" + modeName(mode)];
  }

  // The mode as the data attributes name it.
  function modeName(mode) {
    return { reply: "Reply", "create-topic": "CreateTopic", edit: "Edit" }[mode];
  }

  function showError(messages) {
    error.textContent = messages;
    error.hidden = !messages;
  }

  // Drafts, as models/composer's saveDraft and services/composer's
  // _saveDraft keep them: saved two seconds after typing stops (at once
  // when the last save is fifteen seconds old), on minimizing, and by
  // beacon when the page is left with a save pending; restored when the
  // composer opens on the same key. A reply and an edit share the topic's
  // key (`topic_<id>`), a new topic has its own (`new_topic_<time>`).
  // The owner is this page, as MessageBus's client id is in Ember.
  var clientId = Array.from(crypto.getRandomValues(new Uint8Array(16)), function (b) {
    return b.toString(16).padStart(2, "0");
  }).join("");
  var draftStatus = control.querySelector("#draft-status");
  var draftTimer = null;
  var lastDraftSaved = null;
  var draftSaving = null;

  // The composer's draftStatus: an error while drafts cannot be saved.
  function showDraftStatus(text) {
    var span = draftStatus.querySelector(".draft-error");
    span.title = text || "";
    while (span.childNodes.length > 1) {
      span.removeChild(span.lastChild);
    }
    if (text) {
      span.appendChild(document.createTextNode(text));
    }
    draftStatus.hidden = !text;
  }

  // The composer's action as a draft names it.
  function draftAction(mode) {
    return { reply: "reply", "create-topic": "createTopic", edit: "edit" }[mode];
  }

  // _draft_serializer's fields the composer has.
  function draftData(s) {
    var d = { reply: textarea.value, action: draftAction(s.mode), archetypeId: "regular" };
    d.original_text = s.originalText === undefined ? null : s.originalText;
    if (s.mode === "create-topic") {
      d.title = title.value;
      var category = chooser.querySelector("summary").dataset.value;
      d.categoryId = category ? parseInt(category, 10) : null;
      if (tagChooser) {
        d.tags = serializedTags();
      }
    } else if (s.mode === "edit") {
      d.postId = parseInt(s.postId, 10);
      // A first post's draft keeps the topic fields being edited and what
      // they were.
      if (s.topic) {
        d.title = title.value;
        var editCategory = chooser.querySelector("summary").dataset.value;
        d.categoryId = editCategory ? parseInt(editCategory, 10) : null;
        if (tagChooser) {
          d.tags = serializedTags();
        }
        d.original_title = s.topic.title;
        d.original_tags = s.topic.tags;
      }
    } else {
      d.reply_to_post_number = s.replyTo ? parseInt(s.replyTo, 10) : null;
    }
    return d;
  }

  // canSaveDraft
  function canSaveDraft() {
    if (!state || state.loading || draftSaving) {
      return false;
    }
    if (state.mode === "create-topic") {
      return textarea.value !== "" || title.value !== "";
    }
    return textarea.value !== "";
  }

  function saveDraft() {
    clearTimeout(draftTimer);
    draftTimer = null;
    if (draftSaving) {
      draftTimer = setTimeout(saveDraft, 2000);
      return draftSaving;
    }
    if (!canSaveDraft()) {
      return Promise.resolve();
    }
    var s = state;
    var sequence = s.draftSequence;
    s.draftSequence += 1;
    draftSaving = fetch(base + "/drafts.json", {
      method: "POST",
      headers: csrfHeaders(),
      credentials: "same-origin",
      body: form([
        ["draft_key", s.draftKey],
        ["sequence", sequence],
        ["data", JSON.stringify(draftData(s))],
        ["owner", clientId],
        ["force_save", s.forceSave ? "true" : "false"],
      ]),
    })
      .then(function (r) {
        return r
          .json()
          .catch(function () {
            return null;
          })
          .then(function (json) {
            if (r.ok && json) {
              if ("draft_sequence" in json) {
                s.draftSequence = json.draft_sequence;
              }
              s.forceSave = false;
              showDraftStatus(json.conflict_user ? data.labelEditConflict : "");
              return;
            }
            var message = null;
            if (r.status === 409 && json && json.errors && json.errors.length) {
              message = json.errors[0];
              // Ember's dialog offers to reload or to ignore the newer
              // draft (forceSave on the next save).
              if (json.extras && json.extras.description) {
                if (window.confirm(json.extras.description)) {
                  window.location.reload();
                } else {
                  s.forceSave = true;
                }
              }
            }
            showDraftStatus(message || data.labelDraftsOffline);
          });
      })
      .catch(function () {
        showDraftStatus(data.labelDraftsOffline);
      })
      .finally(function () {
        draftSaving = null;
        lastDraftSaved = Date.now();
      });
    return draftSaving;
  }

  // _shouldSaveDraft, on the reply and the title changing.
  function scheduleDraft() {
    if (!state || state.loading) {
      return;
    }
    if (!lastDraftSaved) {
      lastDraftSaved = Date.now();
    }
    if (Date.now() - lastDraftSaved > 15000) {
      saveDraft();
    } else {
      clearTimeout(draftTimer);
      draftTimer = setTimeout(saveDraft, 2000);
    }
  }

  // destroyDraft: after a save in flight.
  function destroyDraft(s) {
    clearTimeout(draftTimer);
    draftTimer = null;
    return (draftSaving || Promise.resolve()).then(function () {
      return fetch(base + "/drafts/" + encodeURIComponent(s.draftKey) + ".json", {
        method: "DELETE",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form([
          ["draft_key", s.draftKey],
          ["sequence", s.draftSequence],
        ]),
      });
    });
  }

  // Draft.get: the draft (parsed) and the key's sequence.
  function fetchDraft(key) {
    return fetch(base + "/drafts/" + encodeURIComponent(key) + ".json", {
      credentials: "same-origin",
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (json) {
        var draft = null;
        try {
          draft = json.draft ? JSON.parse(json.draft) : null;
        } catch (e) {
          // loadDraft: a draft that does not parse is dropped.
        }
        return { draft: draft, sequence: json.draft_sequence || 0 };
      });
  }

  // _beaconSaveDraft
  window.addEventListener("beforeunload", function () {
    if (!control.isConnected || !draftTimer || !canSaveDraft()) {
      return;
    }
    clearTimeout(draftTimer);
    draftTimer = null;
    var sequence = state.draftSequence;
    state.draftSequence += 1;
    // Form-encoded rather than Ember's FormData: the server does not
    // parse multipart bodies.
    navigator.sendBeacon(
      base + "/drafts.json",
      new URLSearchParams([
        ["draft_key", state.draftKey],
        ["sequence", sequence],
        ["data", JSON.stringify(draftData(state))],
        ["owner", clientId],
        ["authenticity_token", csrfHeaders()["X-CSRF-Token"] || ""],
      ])
    );
  });

  // The topic of a first post being edited: what the composer shows and
  // what Topic.update sends against (original_title, original_tags).
  function loadTopic(s) {
    return fetch(base + "/t/" + s.topicId + ".json", {
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (topic) {
        // The topic's tags as it serializes them ({id, name, slug}),
        // which original_tags sends back.
        var tags = topic.tags || [];
        var details = topic.details || {};
        s.topic = {
          id: topic.id,
          slug: topic.slug,
          title: topic.title,
          categoryId: topic.category_id,
          tags: tags,
          privateMessage: topic.archetype === "private_message",
          canEdit: !!details.can_edit,
          canEditTags: !!details.can_edit_tags,
        };
        if (state !== s) {
          return;
        }
        title.value = topic.title;
        // disableTitleInput: editing without the right to edit the topic.
        title.disabled = !s.topic.canEdit;
        // canCategorize: never for a message.
        chooser.closest(".category-input").hidden = s.topic.privateMessage;
        if (topic.category_id) {
          selectCategory(topic.category_id);
          keepCategory(topic.category_id);
        }
        setTags(
          (topic.tags || []).map(function (t) {
            return typeof t === "string" ? { name: t, id: null } : t;
          })
        );
      });
  }

  function rawToEdit(s) {
    return fetch(base + "/raw/" + s.topicId + "/" + s.postNumber, {
      credentials: "same-origin",
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(r.status);
      }
      return r.text();
    });
  }

  // What the composer opens with: the key's draft when it is for this
  // composer, an edit's raw markdown otherwise. A reply opened over an
  // edit's draft opens that edit (loadDraft takes the draft's action).
  function initialText(s, found) {
    var draft = found && found.draft;
    if (found) {
      s.draftSequence = found.sequence;
    }
    var editDraft = draft && draft.action === "edit";
    // selectText: a quote opens the reply with it, after the key's draft
    // when there is one (composerOpts.reply), else as the text it
    // started from (composerOpts.quote).
    if (s.mode === "reply" && s.quote) {
      s.originalText = draft ? null : s.quote;
      return Promise.resolve(draft ? (draft.reply || "") + "\n" + s.quote : s.quote);
    }
    if (s.mode === "reply" && editDraft && draft.postId) {
      s.mode = "edit";
      s.postId = draft.postId;
      setMode("edit");
    }
    // originalText: what the composer started from, as a draft keeps it
    // (original_text); an edit's is the post's raw, a new reply's none.
    s.originalText = draft && "original_text" in draft ? draft.original_text : null;
    if (s.mode === "edit") {
      if (editDraft && String(draft.postId) === String(s.postId)) {
        if (s.topic && typeof draft.title === "string") {
          title.value = draft.title;
          if (draft.categoryId) {
            selectCategory(draft.categoryId);
          }
          if (draft.tags) {
            setTags(draft.tags);
          }
        }
        return Promise.resolve(draft.reply || "");
      }
      return rawToEdit(s).then(function (raw) {
        s.originalText = raw;
        return raw;
      });
    }
    if (draft && !editDraft) {
      if (s.mode === "create-topic") {
        title.value = draft.title || "";
        if (draft.categoryId) {
          selectCategory(draft.categoryId);
        }
        if (draft.tags) {
          setTags(draft.tags);
        }
      } else if ("reply_to_post_number" in draft) {
        s.replyTo = draft.reply_to_post_number;
      }
      return Promise.resolve(draft.reply || "");
    }
    return Promise.resolve("");
  }

  // open({mode, topicId, replyTo, replyToUsername, postId, postNumber,
  // category, draftKey, draftSequence}): to reply, create a topic or edit
  // a post; a draft key from the drafts menu resumes that draft.
  // A closed composer holds only its grippie, as Ember renders the reply
  // area once the composer is visible.
  var replyArea = control.querySelector(".reply-area");

  // The mobile view's icon discard: cancelIcon.
  function applyDiscardIcon(mode) {
    var discardIcon = control.querySelector(".discard-button.no-text use");
    if (discardIcon) {
      discardIcon.setAttribute("href", mode === "edit" ? "#xmark" : "#trash-can");
      discardIcon.parentNode.setAttribute(
        "class",
        "fa d-icon d-icon-" + (mode === "edit" ? "xmark" : "trash-can") + " svg-icon fa-width-auto svg-string"
      );
    }
  }

  function open(opts) {
    replyArea.hidden = false;
    if (state && state.mode === opts.mode && control.classList.contains("draft")) {
      expand();
      return;
    }
    state = opts;
    var s = state;
    setMode(opts.mode);
    applyDiscardIcon(opts.mode);
    if (opts.mode === "reply" && opts.replyToUsername) {
      actionButton.querySelector(".d-button-label").textContent =
        opts.replyToUsername;
    }
    textarea.value = "";
    title.value = "";
    showError("");
    showDraftStatus("");
    lastDraftSaved = null;
    if (opts.mode === "create-topic") {
      selectCategory(opts.category || data.defaultCategory);
      setTags([]);
    }
    s.firstPost = opts.mode === "edit" && String(opts.postNumber) === "1";
    if (s.firstPost) {
      setMode("edit", true);
      title.disabled = true;
    }
    control.classList.remove("closed", "draft");
    control.classList.add("open");
    root.style.setProperty("--composer-height", defaultHeight(opts.mode));
    updatePreview();

    var resuming = Boolean(opts.draftKey);
    if (opts.mode === "create-topic") {
      s.draftKey = opts.draftKey || "new_topic_" + Date.now();
    } else {
      s.draftKey = "topic_" + opts.topicId;
    }
    s.draftSequence = opts.draftSequence || 0;
    s.loading = true;
    textarea.disabled = true;
    announce();
    // A new topic's key is new: there is no draft to look up.
    var lookup =
      opts.mode === "create-topic" && !resuming
        ? Promise.resolve(null)
        : fetchDraft(s.draftKey);
    // Editing a first post: the topic's title, category and tags, as
    // the topic model holds them in Ember.
    var topicLoad = s.firstPost ? loadTopic(s) : Promise.resolve();
    Promise.all([lookup, topicLoad])
      .then(function (results) {
        return initialText(s, results[0]);
      })
      .then(function (text) {
        if (state !== s) {
          return;
        }
        textarea.value = text;
        updatePreview();
        announce();
      })
      .catch(function () {
        if (state === s) {
          showError(
            s.mode === "edit" ? "Could not load the post to edit." : "Could not load the draft."
          );
        }
      })
      .finally(function () {
        if (state !== s) {
          return;
        }
        s.loading = false;
        textarea.disabled = false;
        if (s.mode === "create-topic" && title.value === "") {
          title.focus();
        } else {
          textarea.focus();
        }
      });
  }

  function close() {
    cancelUploads();
    clearTimeout(draftTimer);
    draftTimer = null;
    control.classList.remove("open", "draft", "fullscreen");
    control.classList.add("closed");
    replyArea.hidden = true;
    document.body.classList.remove("fullscreen-composer");
    state = null;
    announce();
  }

  // The composer's model as discourse-presence's ComposerPresenceDisplay
  // reads it: the action, its topic or post, and replyDirty (the text
  // changed from what it opened with). Null while closed.
  function announce() {
    var detail = null;
    if (state) {
      detail = {
        mode: state.mode,
        topicId: state.topicId || null,
        postId: state.postId || null,
        // replyDirty: the trimmed text against the trimmed original.
        dirty: !state.loading && textarea.value.trim() !== (state.originalText || "").trim(),
      };
    }
    control.dispatchEvent(new CustomEvent("composer:presence", { detail: detail, bubbles: true }));
  }

  function dirty() {
    return textarea.value.trim() !== "" || title.value.trim() !== "";
  }

  // Minimized, the composer is a bar that opens again when clicked; its
  // draft is saved (collapse).
  function minimize() {
    saveDraft();
    control.classList.remove("open", "fullscreen");
    control.classList.add("draft");
    document.body.classList.remove("fullscreen-composer");
  }

  function expand() {
    control.classList.remove("draft");
    control.classList.add("open");
  }

  function toggleFullscreen() {
    saveDraft();
    control.classList.toggle("fullscreen", !control.classList.contains("fullscreen"));
    applyFullscreen();
  }

  function applyFullscreen() {
    var on = control.classList.contains("fullscreen");
    document.body.classList.toggle("fullscreen-composer", on);
    var button = control.querySelector(".toggle-fullscreen");
    if (!button) {
      return;
    }
    button.title = on ? data.labelExitFullscreen : data.labelFullscreen;
    button.querySelector("use").setAttribute(
      "href",
      on ? "#discourse-compress" : "#discourse-expand"
    );
  }

  // The grippie: drag to resize, the height kept for next time.
  control.querySelector(".grippie").addEventListener("pointerdown", function (event) {
    if (!control.classList.contains("open")) {
      return;
    }
    event.preventDefault();
    var startY = event.clientY;
    var startHeight = control.getBoundingClientRect().height;
    function move(e) {
      var size = Math.max(255, startHeight + (startY - e.clientY));
      root.style.setProperty("--composer-height", size + "px");
    }
    function up(e) {
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", up);
      setItem(
        "composerHeight",
        Math.max(255, startHeight + (startY - e.clientY)) + "px"
      );
    }
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", up);
  });

  // The toolbar: markdown around the selection, or its example text.
  function surround(before, after, example) {
    var start = textarea.selectionStart;
    var end = textarea.selectionEnd;
    var selected = textarea.value.slice(start, end) || example;
    textarea.setRangeText(before + selected + after, start, end, "end");
    textarea.selectionStart = start + before.length;
    textarea.selectionEnd = start + before.length + selected.length;
    textarea.focus();
  }

  function prefixLines(prefix, example) {
    var start = textarea.selectionStart;
    var end = textarea.selectionEnd;
    var lineStart = textarea.value.lastIndexOf("\n", start - 1) + 1;
    var selected = textarea.value.slice(lineStart, end) || example;
    var replaced = selected
      .split("\n")
      .map(function (line) {
        return prefix + line;
      })
      .join("\n");
    textarea.setRangeText(replaced, lineStart, Math.max(end, lineStart), "end");
    textarea.focus();
  }

  var actions = {
    bold: function () {
      surround("**", "**", data.textBold);
    },
    italic: function () {
      surround("*", "*", data.textItalic);
    },
    link: function () {
      surround("[", "](https://)", data.textLink);
    },
    blockquote: function () {
      prefixLines("> ", data.textBlockquote);
    },
    code: function () {
      var selected = textarea.value.slice(
        textarea.selectionStart,
        textarea.selectionEnd
      );
      if (selected.indexOf("\n") >= 0 || selected === "") {
        surround("```\n", "\n```", data.textCode);
      } else {
        surround("`", "`", "");
      }
    },
    upload: function () {
      fileInput.click();
    },
    list: function () {
      prefixLines("- ", data.textList);
    },
  };

  // The preview: the markdown crate as WebAssembly, loaded the first time
  // the composer opens with the preview shown. Its exports are in
  // crates/markdown/src/wasm.rs.
  var preview = control.querySelector(".d-editor-preview");
  var previewToggle = control.querySelector(".toggle-preview");
  var renderer = null;
  var previewTimer = null;

  function loadRenderer() {
    if (!renderer) {
      renderer = Promise.all([
        WebAssembly.instantiateStreaming(fetch(data.previewWasm), {}),
        fetch(data.previewSettings, { credentials: "same-origin" }).then(function (r) {
          if (!r.ok) {
            throw new Error("settings " + r.status);
          }
          return r.text();
        }),
      ]).then(function (loaded) {
        var wasm = loaded[0].instance.exports;
        var call = function (fn, text) {
          var bytes = new TextEncoder().encode(text);
          var ptr = wasm.alloc(bytes.length);
          new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
          var status = fn(ptr, bytes.length);
          wasm.dealloc(ptr, bytes.length);
          var out = new TextDecoder().decode(
            new Uint8Array(wasm.memory.buffer, wasm.output_ptr(), wasm.output_len())
          );
          if (status !== 0) {
            throw new Error(out);
          }
          return out;
        };
        call(wasm.configure, loaded[1]);
        return function (raw) {
          return call(wasm.preview, raw);
        };
      });
    }
    return renderer;
  }

  // isPreviewVisible: the stored choice, for those whose editor has a
  // preview (allowPreview: markdown mode, not the rich editor's).
  function previewShown() {
    return (
      data.allowPreview !== "false" &&
      (getItem("composer.showPreview") || "true") === "true"
    );
  }

  function updatePreview() {
    if (!previewToggle || !previewShown()) {
      return;
    }
    var raw = textarea.value;
    loadRenderer()
      .then(function (render) {
        // What the server cooks the renderer refuses; the preview says so
        // rather than showing something the post will not be.
        try {
          preview.innerHTML = render(raw);
          resolveShortUrls();
          // The topic composer's hashtags (hashtag_configurations'
          // topic-composer order), linked once looked up, then decorated.
          if (window.Discourse && Discourse.hashtags) {
            Discourse.hashtags.resolve(preview, ["category", "tag", "channel"], updatePreview);
            Discourse.hashtags.decorate(preview);
          }
          // Oneboxes: the cached ones now, the rest loaded after 450ms of
          // quiet (composer-editor's _loadOneboxes).
          if (window.Discourse && Discourse.oneboxes) {
            Discourse.oneboxes.applyCached(preview);
            clearTimeout(oneboxTimer);
            oneboxTimer = setTimeout(loadPreviewOneboxes, 450);
          }
          // The post decorators Ember runs on the preview too.
          if (window.Discourse && Discourse.highlightSyntax) {
            Discourse.highlightSyntax(preview);
          }
          if (window.Discourse && Discourse.applyLocalDates) {
            Discourse.applyLocalDates(preview, { title: true });
          }
        } catch (e) {
          preview.textContent = e.message;
        }
      })
      .catch(function (e) {
        preview.textContent = "The preview could not load: " + e.message;
      });
  }

  // _loadOneboxes: an edited post's oneboxes are refreshed once.
  var oneboxTimer = null;
  function loadPreviewOneboxes() {
    if (!state) {
      return;
    }
    var category = state.mode === "create-topic" ? chooser.querySelector("summary").dataset.value : null;
    var refresh = state.mode === "edit" && !state.refreshedPost;
    var painted = Discourse.oneboxes.load(preview, {
      topicId: state.topicId || null,
      categoryId: category ? parseInt(category, 10) : null,
      maxOneboxes: parseInt(data.maxOneboxes, 10) || 50,
      refresh: refresh,
    });
    if (refresh && painted > 0) {
      state.refreshedPost = true;
    }
  }

  function schedulePreview() {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(updatePreview, 30);
  }

  // The mobile view has no preview toggle, and shows no preview.
  function applyPreviewShown() {
    var shown = Boolean(previewToggle) && previewShown();
    control.classList.toggle("show-preview", shown);
    control.classList.toggle("hide-preview", !shown);
    if (!previewToggle) {
      return;
    }
    previewToggle.hidden = data.allowPreview === "false";
    previewToggle.classList.toggle("active", !shown);
    previewToggle.title = shown ? data.labelHidePreview : data.labelShowPreview;
  }

  function togglePreview() {
    setItem("composer.showPreview", String(!previewShown()));
    applyPreviewShown();
    updatePreview();
  }

  applyPreviewShown();
  textarea.addEventListener("input", schedulePreview);
  textarea.addEventListener("input", announce);

  // The category chooser.
  function selectCategory(id) {
    var row = chooser.querySelector('.category-row[data-value="' + id + '"]');
    var header = chooser.querySelector(".select-kit-selected-name");
    var summary = chooser.querySelector("summary");
    if (!row) {
      return;
    }
    var badges = row.querySelectorAll(".category-status > .badge-category__wrapper");
    var badge = badges[badges.length - 1].cloneNode(true);
    var count = badge.querySelector(".topic-count");
    if (count) {
      count.remove();
    }
    header.innerHTML = "";
    var name = document.createElement("span");
    name.className = "name";
    name.appendChild(badge);
    header.appendChild(name);
    header.dataset.value = id;
    header.dataset.name = row.dataset.name;
    summary.dataset.value = id;
    chooser.classList.add("has-selection");
    if (tagChooser) {
      tagChooser.setAttribute("category-id", id);
      tagHeader();
      validateTags(false);
    }
    chooser.querySelectorAll(".category-row").forEach(function (r) {
      r.classList.toggle("is-selected", r === row);
    });
  }

  // A topic's category that isn't among those the chooser lists (one
  // the editor can't post in) stays selected, shown with the topic
  // title's badge, as Ember's chooser shows its value.
  function keepCategory(id) {
    var summary = chooser.querySelector("summary");
    if (summary.dataset.value === String(id)) {
      return;
    }
    var header = chooser.querySelector(".select-kit-selected-name");
    var badge = document.querySelector("#topic-title .badge-category__wrapper");
    header.innerHTML = "";
    var name = document.createElement("span");
    name.className = "name";
    if (badge) {
      name.appendChild(badge.cloneNode(true));
    }
    header.appendChild(name);
    header.dataset.value = id;
    summary.dataset.value = id;
    chooser.classList.add("has-selection");
    chooser.querySelectorAll(".category-row").forEach(function (r) {
      r.classList.remove("is-selected");
    });
  }

  chooser.addEventListener("toggle", function () {
    chooser.classList.toggle("is-expanded", chooser.open);
    if (chooser.open) {
      chooser.querySelector(".filter-input").focus();
    }
  });
  chooser.querySelector(".filter-input").addEventListener("input", function (e) {
    var term = e.target.value.toLowerCase();
    chooser.querySelectorAll(".category-row").forEach(function (row) {
      row.hidden = term !== "" && row.dataset.name.toLowerCase().indexOf(term) < 0;
    });
  });
  chooser.addEventListener("click", function (e) {
    var row = e.target.closest(".category-row");
    if (row) {
      selectCategory(row.dataset.value);
      chooser.open = false;
    }
  });

  // The tag chooser: MiniTagChooser (a multi select over
  // /tags/filter/search) as composer-container renders it for a member
  // who may tag topics. Its body is drawn when it opens, as select-kit
  // draws it.
  var tagChooser = control.querySelector(".mini-tag-chooser");
  var tagsTip = control.querySelector(".tags-input .popup-tip");
  var selectedTags = [];
  var tagSearchTimer = null;
  var tagSearchSeq = 0;
  var tagRequired = null;
  var tagForbidden = false;
  var tagHighlighted = -1;
  var tagRows = [];
  var tagOpts = {};
  if (tagChooser) {
    try {
      tagOpts = JSON.parse(tagChooser.dataset.options || "{}");
    } catch (_) {}
  }

  function tagLabel(key, vars) {
    var text = (tagOpts.labels || {})[key] || "";
    Object.keys(vars || {}).forEach(function (k) {
      text = text.split("%{" + k + "}").join(vars[k]);
    });
    return text;
  }

  // The category's minimumRequiredTags (its minimum, or its required
  // groups' counts when more).
  function minimumRequiredTags() {
    if (!chooser) {
      return 0;
    }
    var id = chooser.querySelector("summary").dataset.value;
    var row = id ? chooser.querySelector('.category-row[data-value="' + id + '"]') : null;
    return row ? parseInt(row.dataset.minimumRequiredTags || "0", 10) : 0;
  }

  // serializeTags
  function serializedTags() {
    return selectedTags.map(function (t) {
      return t.isNew || !t.id ? { name: t.name } : { id: t.id, name: t.name };
    });
  }

  function renderTag(name, count) {
    var html =
      '<a data-tag-name="' + escapeHtml(name.toLowerCase()) + '" class="discourse-tag ' +
      escapeHtml(tagOpts.style || "simple") + '">' + escapeHtml(name) + "</a>";
    if (count) {
      html += ' <span class="discourse-tag-count">x' + count + "</span>";
    }
    return html;
  }

  function tagHeader() {
    var summary = tagChooser.querySelector("summary");
    // modifyNoSelection: the category's minimum, when it has one.
    var minimum = minimumRequiredTags();
    var none =
      minimum > 0
        ? tagLabel(minimum === 1 ? "required_one" : "required_other", { count: minimum })
        : tagLabel("optional");
    var names = selectedTags.map(function (t) {
      return t.name;
    });
    var max = tagOpts.maximum;
    var caret = max && selectedTags.length >= max ? "" : svg("plus", "angle-icon");
    summary.querySelector(".select-kit-header-wrapper").innerHTML =
      '<span class="formatted-selection">' + escapeHtml(names.length ? names.join(", ") : none) + "</span>" + caret;
    summary.dataset.name = names.join(",");
    summary.dataset.value = selectedTags
      .map(function (t) {
        return t.id || t.name;
      })
      .join(",");
    tagChooser.classList.toggle("no-tags", selectedTags.length === 0);
    tagChooser.classList.toggle("has-selection", selectedTags.length > 0);
  }

  function setTags(tags) {
    selectedTags = (tags || []).map(function (t) {
      return typeof t === "string" ? { name: t, isNew: true } : { id: t.id, name: t.name, isNew: !t.id };
    });
    if (tagChooser) {
      tagHeader();
      validateTags(false);
    }
  }

  function tagFilter() {
    var input = tagChooser.querySelector(".filter-input");
    return input ? input.value : "";
  }

  // tagUtils.createContentFromInput
  function tagFromInput(input) {
    input = input
      .trim()
      .replace(/\s+/g, "-")
      .replace(/[\/\?#\[\]@!\$&'\(\)\*\+,;=%\\`^\s|\{\}"<>]+/g, "")
      .replace(/-{2,}/g, "-")
      .substring(0, tagOpts.maxLength || 20)
      .replace(/^\.+|\.+$/g, "");
    if (tagOpts.forceLowercase) {
      input = input.toLowerCase();
    }
    return input;
  }

  // tagUtils.validateCreate
  function canCreateTag(filter, results) {
    if (!tagOpts.allowAny || !filter.length) {
      return false;
    }
    if (tagOpts.maximum && selectedTags.length >= tagOpts.maximum) {
      return false;
    }
    var term = filter.replace(new RegExp(tagOpts.filterRegexp || "^$", "g"), "").trim().toLowerCase();
    if (tagForbidden) {
      return false;
    }
    var inCollection = results.some(function (r) {
      return (r.name || "").toLowerCase() === term;
    });
    var inSelection = selectedTags.some(function (t) {
      return t.name.toLowerCase() === term;
    });
    return !(inCollection || inSelection);
  }

  function drawTagBody(results, filter) {
    var body = tagChooser.querySelector(".select-kit-body");
    var input = body.querySelector(".filter-input");
    var focused = document.activeElement === input;
    var value = input ? input.value : "";
    var placeholder = tagRequired
      ? tagLabel(tagRequired.min_count === 1 ? "required_group_one" : "required_group_other", {
          count: tagRequired.min_count,
          name: tagRequired.name,
        })
      : tagLabel(tagOpts.allowAny ? "filter_placeholder_with_any" : "filter_placeholder");
    var html =
      '<div class="select-kit-filter multi-select-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      escapeHtml(placeholder) + '" spellcheck="false" tabindex="0" type="search"></div>';
    if (selectedTags.length) {
      html += '<div class="selected-content">';
      selectedTags.forEach(function (t, i) {
        html +=
          '<button aria-label="' + escapeHtml(tagLabel("delete_item", { name: t.name })) +
          '" class="btn btn-default selected-choice tag-choice " data-name="' + escapeHtml(t.name) +
          '" data-value="' + escapeHtml(String(t.id || t.name)) + '" data-index="' + i + '" type="button">' +
          svg("xmark") + '<span class="d-button-label">' + escapeHtml(t.name) + "</span></button>";
      });
      html += "</div>";
    }
    tagRows = [];
    var maxReached = tagOpts.maximum && selectedTags.length >= tagOpts.maximum;
    if (maxReached && filter) {
      html +=
        '<ul class="select-kit-errors-collection"><li class="select-kit-error">' +
        escapeHtml(tagLabel(tagOpts.maximum === 1 ? "max_one" : "max_other", { count: tagOpts.maximum })) +
        "</li></ul>";
    }
    if (canCreateTag(filter, results)) {
      var created = tagFromInput(filter);
      if (created) {
        tagRows.push({ create: true, name: created });
      }
    }
    results.forEach(function (r) {
      tagRows.push(r);
    });
    if (tagRows.length) {
      html += '<ul aria-live="polite" class="select-kit-collection" role="menu">';
      tagRows.forEach(function (r, i) {
        var highlighted = i === tagHighlighted ? " is-highlighted" : "";
        if (r.create) {
          html +=
            '<li aria-checked="false" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(r.name) +
            '" data-value="' + escapeHtml(r.name) + '" title="' + escapeHtml(r.name) + '" tabindex="0" class="select-kit-row' +
            highlighted + '"><span class="name">' + escapeHtml(tagLabel("create", { content: r.name })) + "</span></li>";
          return;
        }
        var disabled = r.disabled ? " disabled" : "";
        html +=
          '<li aria-checked="false" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(r.name) +
          '" data-value="' + escapeHtml(String(r.id)) + '" title="' + escapeHtml(r.title || r.name) + '" tabindex="0"' +
          (r.disabled ? ' aria-disabled="true"' : "") + ' class="select-kit-row tag-row' + highlighted + disabled + '">  ' +
          renderTag(r.name, r.count);
        if (r.disabled) {
          html += '<span class="disabled-reason">' + escapeHtml(r.title || "") + "</span>";
        } else if (r.target_tag) {
          html += '<span class="synonym-hint">' + escapeHtml(tagLabel("synonym_hint", { tag_name: r.target_tag.name })) + "</span>";
        }
        html += "</li>";
      });
      html += "</ul>";
    }
    if (filter) {
      if (!tagRows.length) {
        html += '<span class="no-content" role="alert">' + escapeHtml(tagLabel("no_content")) + "</span>";
      } else {
        html +=
          '<span class="results-count" role="alert">' +
          escapeHtml(tagLabel(tagRows.length === 1 ? "results_one" : "results_other", { count: tagRows.length })) +
          "</span>";
      }
    }
    body.innerHTML = html;
    var next = body.querySelector(".filter-input");
    next.value = value;
    if (focused || tagChooser.open) {
      next.focus();
    }
  }

  // MiniTagChooser#search and _transformJson: the results less the
  // selected tags, sorted by name when the site sorts tags so.
  function searchTags() {
    var filter = tagFilter();
    var seq = ++tagSearchSeq;
    if (tagOpts.maximum === 0) {
      drawTagBody([], filter);
      return;
    }
    var params = new URLSearchParams();
    params.append("q", filter || "");
    params.append("limit", tagOpts.limit);
    var category = chooser ? chooser.querySelector("summary").dataset.value : "";
    if (category) {
      params.append("categoryId", category);
    }
    selectedTags
      .filter(function (t) {
        return t.id;
      })
      .slice(0, 100)
      .forEach(function (t) {
        params.append("selected_tag_ids[]", t.id);
      });
    params.append("filterForInput", "true");
    var recent = tagOpts.prioritizeRecent && !filter;
    if (recent) {
      params.append("prioritizeRecentTags", "true");
    }
    fetch(base + "/tags/filter/search?" + params.toString(), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    })
      .then(function (r) {
        return r.json();
      })
      .then(function (json) {
        if (seq !== tagSearchSeq || !tagChooser.open) {
          return;
        }
        tagForbidden = Boolean(json.forbidden);
        tagRequired = json.required_tag_group || null;
        var results = json.results || [];
        if (!recent && tagOpts.sortAlphabetically) {
          var byName = function (a, b) {
            return a.name.localeCompare(b.name);
          };
          results = results
            .filter(function (r) {
              return !r.disabled;
            })
            .sort(byName)
            .concat(
              results
                .filter(function (r) {
                  return r.disabled;
                })
                .sort(byName)
            );
        }
        var names = selectedTags.map(function (t) {
          return t.name;
        });
        results = results.filter(function (r) {
          return names.indexOf(r.name) < 0;
        });
        // select-kit highlights the first row once there is a filter.
        tagHighlighted = filter ? 0 : -1;
        drawTagBody(results, filter);
      })
      .catch(function () {
        showError("Could not search tags.");
      });
  }

  function chooseTagRow(index) {
    var row = tagRows[index];
    if (!row || row.disabled) {
      return;
    }
    if (tagOpts.maximum && selectedTags.length >= tagOpts.maximum) {
      return;
    }
    if (row.create) {
      selectedTags.push({ name: row.name, isNew: true });
    } else {
      selectedTags.push({ id: row.id, name: row.name });
    }
    var input = tagChooser.querySelector(".filter-input");
    if (input) {
      input.value = "";
    }
    tagHeader();
    validateTags(false);
    scheduleDraft();
    searchTags();
  }

  // composer.tagValidation: staff and members of sites that can't tag
  // aside, the category's minimumRequiredTags.
  function validateTags(show) {
    if (!tagsTip) {
      return true;
    }
    var min = minimumRequiredTags();
    var failed = !tagOpts.staff && state && state.mode === "create-topic" && min > selectedTags.length;
    if (failed && show) {
      var text = tagLabel(min === 1 ? "tags_missing_one" : "tags_missing_other", { count: min });
      tagsTip.className = "popup-tip bad";
      tagsTip.setAttribute("role", "alert");
      tagsTip.setAttribute("aria-label", text);
      tagsTip.innerHTML = escapeHtml(text) + " " + svg("circle-xmark");
    } else if (!failed) {
      tagsTip.className = "popup-tip good hide";
      tagsTip.removeAttribute("role");
      tagsTip.removeAttribute("aria-label");
      tagsTip.innerHTML = " " + svg("circle-xmark");
    }
    return !failed;
  }

  if (tagChooser) {
    tagChooser.addEventListener("toggle", function () {
      tagChooser.classList.toggle("is-expanded", tagChooser.open);
      if (tagChooser.open) {
        // select-kit's floating body: on screens up to 450px wide it
        // spans the viewport less 10px each side (applySmallScreenOffset
        // and applySmallScreenMaxWidth).
        var body = tagChooser.querySelector(".select-kit-body");
        if (window.innerWidth <= 450) {
          var x = tagChooser.getBoundingClientRect().x;
          body.style.left = 10 - x + "px";
          body.style.width = window.innerWidth - 20 + "px";
          body.style.minWidth = "0";
        } else {
          body.style.left = "";
          body.style.width = "";
          body.style.minWidth = "";
        }
        tagHighlighted = -1;
        drawTagBody([], "");
        searchTags();
      } else {
        tagChooser.querySelector(".select-kit-body").innerHTML = "";
      }
    });
    tagChooser.addEventListener("input", function (e) {
      if (!e.target.classList.contains("filter-input")) {
        return;
      }
      clearTimeout(tagSearchTimer);
      tagSearchTimer = setTimeout(searchTags, 200);
    });
    tagChooser.addEventListener("keydown", function (e) {
      if (!tagChooser.open || !e.target.classList.contains("filter-input")) {
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        tagChooser.open = false;
      } else if (e.key === "Enter") {
        e.preventDefault();
        chooseTagRow(tagHighlighted);
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (!tagRows.length) {
          return;
        }
        var from = tagHighlighted < 0 ? (e.key === "ArrowDown" ? -1 : 0) : tagHighlighted;
        tagHighlighted = (from + (e.key === "ArrowDown" ? 1 : tagRows.length - 1)) % tagRows.length;
        tagChooser.querySelectorAll(".select-kit-row").forEach(function (row, i) {
          row.classList.toggle("is-highlighted", i === tagHighlighted);
        });
      } else if (e.key === "Backspace" && tagFilter() === "" && selectedTags.length) {
        selectedTags.pop();
        tagHeader();
        validateTags(false);
        scheduleDraft();
        searchTags();
      }
    });
    tagChooser.addEventListener("click", function (e) {
      var choice = e.target.closest(".selected-choice");
      if (choice) {
        e.preventDefault();
        selectedTags.splice(parseInt(choice.dataset.index, 10), 1);
        tagHeader();
        validateTags(false);
        scheduleDraft();
        searchTags();
        return;
      }
      var row = e.target.closest(".select-kit-row");
      if (row) {
        e.preventDefault();
        chooseTagRow(parseInt(row.dataset.index, 10));
      }
    });
    tagChooser.addEventListener("mouseover", function (e) {
      var row = e.target.closest(".select-kit-row");
      if (row) {
        tagHighlighted = parseInt(row.dataset.index, 10);
        tagChooser.querySelectorAll(".select-kit-row").forEach(function (r) {
          r.classList.toggle("is-highlighted", r === row);
        });
      }
    });
    // A click elsewhere closes it, as select-kit's click outside does.
    document.addEventListener("click", function (e) {
      if (tagChooser.open && !tagChooser.contains(e.target)) {
        tagChooser.open = false;
      }
    });
  }

  function form(pairs) {
    return pairs
      .map(function (p) {
        return encodeURIComponent(p[0]) + "=" + encodeURIComponent(p[1]);
      })
      .join("&");
  }

  // Composer#editPost: a first post's topic fields through Topic.update
  // (PUT /t, or only its tags through PUT /t/:id/tags without the right
  // to edit the topic), then the post through PUT /posts. A 409 marks an
  // edit conflict: saving again overwrites.
  function editPost(s) {
    var promise = Promise.resolve(null);
    if (s.topic && title.value) {
      var topic = s.topic;
      if (topic.canEdit) {
        // The tags only where tagging is on (the topic has none otherwise).
        var props = { title: title.value };
        if (tagChooser) {
          props.tags = serializedTags();
        }
        var category = chooser.querySelector("summary").dataset.value;
        // Topic.update: never a category for a message.
        if (!topic.privateMessage) {
          props.category_id = category ? parseInt(category, 10) : null;
        }
        if (!s.editConflict) {
          props.original_title = topic.title;
          if (tagChooser) {
            props.original_tags = topic.tags;
          }
        }
        var headers = csrfHeaders();
        headers["Content-Type"] = "application/json";
        promise = fetch(base + "/t/" + (topic.slug || "-") + "/" + topic.id, {
          method: "PUT",
          headers: headers,
          credentials: "same-origin",
          body: JSON.stringify(props),
        });
      } else if (topic.canEditTags) {
        // Topic#updateTags: the tags as objects ({id, name}), as Rack
        // reads `tags[][...]`; none sends no tags.
        var tagPairs = [];
        serializedTags().forEach(function (t) {
          if (t.id) {
            tagPairs.push(["tags[][id]", t.id]);
          }
          tagPairs.push(["tags[][name]", t.name]);
        });
        promise = fetch(base + "/t/" + topic.id + "/tags", {
          method: "PUT",
          headers: csrfHeaders(),
          credentials: "same-origin",
          body: form(tagPairs),
        });
      }
    }
    return promise.then(function (topicResponse) {
      if (topicResponse && !topicResponse.ok) {
        if (topicResponse.status === 409) {
          s.editConflict = true;
        }
        return topicResponse;
      }
      var pairs = [
        ["post[raw]", textarea.value],
        ["post[topic_id]", s.topicId],
      ];
      if (!s.editConflict && s.originalText !== null && s.originalText !== undefined) {
        pairs.push(["post[original_text]", s.originalText]);
      }
      return fetch(base + "/posts/" + s.postId, {
        method: "PUT",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form(pairs),
      }).then(function (r) {
        if (r.status === 409) {
          s.editConflict = true;
        }
        return r;
      });
    });
  }

  function save() {
    if (!state) {
      return;
    }
    // cantSubmitPost on the tags: the validation's tip shows instead.
    if (state.mode === "create-topic" && !validateTags(true)) {
      return;
    }
    var request;
    if (state.mode === "edit") {
      request = editPost(state);
    } else {
      // The draft goes with the post (PostCreator's draft_key).
      var pairs = [
        ["raw", textarea.value],
        ["draft_key", state.draftKey],
      ];
      if (state.mode === "create-topic") {
        pairs.push(["title", title.value]);
        var category = chooser.querySelector("summary").dataset.value;
        if (category) {
          pairs.push(["category", category]);
        }
        // serializeTags: an existing tag by id and name, a new one by
        // name, as Rack reads `tags[][...]`.
        serializedTags().forEach(function (t) {
          if (t.id) {
            pairs.push(["tags[][id]", t.id]);
          }
          pairs.push(["tags[][name]", t.name]);
        });
      } else {
        pairs.push(["topic_id", state.topicId]);
        if (state.replyTo) {
          pairs.push(["reply_to_post_number", state.replyTo]);
        }
      }
      request = fetch(base + "/posts", {
        method: "POST",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form(pairs),
      });
    }
    // A pending draft save would outlive the post.
    clearTimeout(draftTimer);
    draftTimer = null;
    submit.disabled = true;
    request
      .then(function (r) {
        return r.text().then(function (body) {
          var json = null;
          try {
            json = JSON.parse(body);
          } catch (e) {
            // Not JSON: an error page.
          }
          return { ok: r.ok, status: r.status, json: json };
        });
      })
      .then(function (res) {
        submit.disabled = false;
        if (!res.ok || (res.json && res.json.errors)) {
          showError(
            res.json && res.json.errors
              ? res.json.errors.join(" ")
              : "Could not save (" + res.status + ")."
          );
          return;
        }
        var mode = state.mode;
        close();
        // A new topic opens; a reply or an edit arrives on the topic page's
        // live stream.
        if (mode === "create-topic" && res.json) {
          var post = res.json.post || res.json;
          location.href =
            base + "/t/" + post.topic_slug + "/" + post.topic_id;
        }
      })
      .catch(function () {
        submit.disabled = false;
        showError("Something went wrong.");
      });
  }

  // The toolbar's options menu (ToolbarPopupMenuOptions over
  // Composer#popupMenuOptions): the items the server listed for the
  // site's settings, as a DMenu. The ones whose actions aren't ported
  // say so instead of acting.
  var optionsTrigger = control.querySelector(".toolbar-menu__options-trigger");
  var popup = { items: [], unported: [] };
  if (optionsTrigger) {
    try {
      popup = JSON.parse(optionsTrigger.dataset.popupOptions);
    } catch (_) {}
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function svg(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' + name + " svg-icon fa-width-auto" + (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  var apple = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent);

  // DShortcut's keycaps for `mod+<shortcut>` (formatShortcut): Ctrl (⌘ on
  // Apple platforms) first, then Shift, then the key.
  function shortcutKbd(shortcut, always) {
    if (!window.matchMedia("(pointer: fine)").matches) {
      return "";
    }
    var keys = shortcut.split("+");
    var key = keys.pop();
    var caps = [apple ? ["⌘", true] : ["Ctrl", false]];
    if (keys.indexOf("Shift") !== -1) {
      caps.push(apple ? ["⇧", true] : ["Shift", false]);
    }
    caps.push([key.length === 1 ? key.toUpperCase() : key, false]);
    return (
      '<kbd aria-hidden="true" class="d-shortcut shortcut' + (always ? " --always-visible" : "") + '" dir="ltr">' +
      caps
        .map(function (cap) {
          return (
            '<kbd class="d-shortcut__key' + (cap[1] ? " d-shortcut__key--glyph" : "") + '">' + escapeHtml(cap[0]) + "</kbd>"
          );
        })
        .join("") +
      "</kbd>"
    );
  }

  function popupItemShown(item) {
    if (item.when === "touch") {
      return document.documentElement.classList.contains("discourse-touch");
    }
    if (item.when === "edit-reply") {
      return !!state && state.mode === "edit" && parseInt(state.postNumber, 10) > 1;
    }
    return true;
  }

  function popupMenuContent() {
    var html = '<ul class="dropdown-menu">';
    popup.items.filter(popupItemShown).forEach(function (item) {
      html +=
        '<li class="dropdown-menu__item"><button class="btn no-text btn-icon" title="' + escapeHtml(item.label) + '"' +
        (item.named ? ' data-name="' + escapeHtml(item.name) + '"' : "") + ' type="button">' + svg(item.icon) +
        '<span class="d-button-label"><span class="d-button-label__text">' + escapeHtml(item.label) + "</span>" +
        (item.shortcut ? shortcutKbd(item.shortcut, item.alwaysShowShortcut) : "") +
        (item.showActiveIcon ? svg("check", "d-button-label__active-icon") : "") +
        "</span></button></li>";
    });
    return html + "</ul>";
  }

  // applySurround as the plugins call it: the selection (or the example
  // text) wrapped, on lines of its own in block mode when the selection
  // spans lines.
  function applySurround(head, tail, example, blockMode) {
    var selected = textarea.value.slice(textarea.selectionStart, textarea.selectionEnd);
    if (blockMode && selected.indexOf("\n") !== -1) {
      surround(head + "\n", "\n" + tail, example);
    } else {
      surround(head, tail, example);
    }
  }

  function addText(text) {
    insertText(text);
    textarea.focus();
    updatePreview();
    scheduleDraft();
  }

  // The post a JSON request answers, or why not.
  function fetchJson(path) {
    return fetch(base + path, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(r.status + " " + r.statusText);
      }
      return r.json();
    });
  }

  // Composer#importQuote: the whole post replied to (the topic's first
  // post when replying to the topic; when editing, the post the edited
  // one replies to, else that post) quoted at the cursor.
  function importQuote() {
    if (!state || !state.topicId || !Discourse.buildQuote) {
      return;
    }
    var s = state;
    var post;
    if (s.mode === "edit") {
      post = fetchJson("/posts/by_number/" + s.topicId + "/" + s.postNumber).then(function (edited) {
        return edited.reply_to_post_number
          ? fetchJson("/posts/by_number/" + s.topicId + "/" + edited.reply_to_post_number)
          : edited;
      });
    } else if (s.mode === "reply") {
      post = fetchJson("/posts/by_number/" + s.topicId + "/" + (s.replyTo || 1));
    } else {
      return;
    }
    control.classList.add("loading");
    post
      .then(function (p) {
        if (state !== s) {
          return;
        }
        insertText(Discourse.buildQuote(p, p.raw, { full: true }));
        textarea.blur();
        textarea.focus({ preventScroll: true });
        updatePreview();
        scheduleDraft();
      })
      .catch(function (e) {
        showError(String(e.message || e));
      })
      .finally(function () {
        control.classList.remove("loading");
      });
  }

  var popupActions = {
    quote: importQuote,
    details: function () {
      applySurround(
        '\n[details="' + data.textDetailsTitle + '"]\n',
        "\n[/details]\n",
        data.textDetails,
        false
      );
    },
    footnote: function () {
      addText("^[" + data.textFootnote + "]");
    },
    spoiler: function () {
      applySurround("[spoiler]", "[/spoiler]", data.textSpoiler, true);
    },
    "local-dates": function () {
      if (Discourse.localDatesCreate) {
        Discourse.localDatesCreate.show(addText);
      }
    },
  };

  // The options' shortcuts in the editor (mod+Shift+D, mod+Shift+.):
  // the option's shortcutAction, else its action.
  var popupShortcuts = {
    "local-dates": function () {
      if (Discourse.localDatesCreate) {
        Discourse.localDatesCreate.currentDateMarkup().then(function (markup) {
          if (markup) {
            addText(markup);
          }
        });
      }
    },
  };

  function runPopupAction(item, viaShortcut) {
    var action = (viaShortcut && popupShortcuts[item.name]) || popupActions[item.name];
    if (item.ported && action) {
      action();
    } else {
      showError(data.labelUnported + " " + item.label);
    }
  }

  if (optionsTrigger) {
    optionsTrigger.addEventListener("click", function (event) {
      event.stopPropagation();
      if (popup.unported.length) {
        showError(data.labelUnported + " " + popup.unported.join(", "));
        return;
      }
      var menu = Discourse.float.menu(optionsTrigger, {
        identifier: "toolbar-menu__options",
        mainClass: "toolbar-menu__options-content toolbar-popup-menu-options",
        placement: "bottom",
        fallbackPlacements: ["top"],
        offset: 5,
        modalForMobile: true,
        content: popupMenuContent(),
      });
      var content = menu && menu.content;
      if (content) {
        content.addEventListener("click", function (e) {
          var button = e.target.closest(".dropdown-menu__item button");
          if (!button) {
            return;
          }
          var index = Array.prototype.indexOf.call(
            content.querySelectorAll(".dropdown-menu__item button"),
            button
          );
          var item = popup.items.filter(popupItemShown)[index];
          menu.close();
          if (item) {
            runPopupAction(item, false);
          }
        });
      }
    });
  }

  textarea.addEventListener("keydown", function (event) {
    if (!(event.ctrlKey || event.metaKey) || !event.shiftKey || popup.unported.length) {
      return;
    }
    popup.items.forEach(function (item) {
      if (!item.shortcut || !popupItemShown(item)) {
        return;
      }
      var key = item.shortcut.split("+").pop();
      // Shift+. arrives as ">" on most layouts; the code is the key.
      var code = key === "." ? "Period" : "Key" + key.toUpperCase();
      if (event.code === code) {
        event.preventDefault();
        runPopupAction(item, true);
      }
    });
  });

  // The preview's renderer for others: the insert date modal's preview.
  // The composer service's open, for openers outside this file (the
  // full-page search's "start a new topic").
  Discourse.composerOpen = function (opts) {
    open(opts);
  };

  Discourse.composerCook = function (raw) {
    return loadRenderer().then(function (render) {
      return render(raw);
    });
  };

  control.addEventListener("click", function (event) {
    var tool = event.target.closest(".toolbar__button");
    if (tool && actions[tool.dataset.action]) {
      actions[tool.dataset.action]();
      return;
    }
    if (event.target.closest(".toggle-preview")) {
      togglePreview();
    } else if (event.target.closest(".toggle-fullscreen")) {
      toggleFullscreen();
    } else if (event.target.closest(".toggle-toolbar")) {
      // ComposerEditor#toggleToolbar: the mobile view's toolbar.
      var wmd = control.querySelector(".wmd-controls");
      var visible = !wmd.classList.contains("toolbar-visible");
      wmd.classList.toggle("toolbar-visible", visible);
      event.target.closest(".toggle-toolbar").title = visible
        ? data.labelHideToolbar
        : data.labelShowToolbar;
    } else if (event.target.closest("#mobile-file-upload")) {
      var picker = control.querySelector("#file-uploader");
      if (picker) {
        picker.click();
      }
    } else if (event.target.closest(".toggle-minimize")) {
      minimize();
    } else if (event.target.closest(".toggle-save-and-close")) {
      // saveAndCloseComposer: the draft is kept for the next time the
      // composer opens on its key; with nothing written it goes.
      if (state && dirty()) {
        saveDraft();
        close();
      } else if (state) {
        destroyDraft(state);
        close();
      }
    } else if (event.target.closest(".discard-button")) {
      // cancelComposer: DiscardDraftModal's question, then the draft goes.
      var confirmText =
        state && state.mode === "edit" ? data.labelDiscardConfirmEdit : data.labelDiscardConfirm;
      if (state && (!dirty() || window.confirm(confirmText))) {
        destroyDraft(state);
        close();
      }
    } else if (event.target.closest(".save-or-cancel .create")) {
      save();
    } else if (control.classList.contains("draft")) {
      expand();
    }
  });

  // A redraw in the other layout (page.js) keeps the composer, its text
  // and its state, and takes the layout's controls from the new page:
  // ComposerToggles, and the save and discard buttons with what goes
  // beside them (the panel's nodes before the upload line).
  function lead(panel) {
    var nodes = [];
    for (var n = panel.firstChild; n && !(n.matches && n.matches("#file-uploading, #draft-status, .composer-error")); n = n.nextSibling) {
      nodes.push(n);
    }
    return nodes;
  }

  control.addEventListener("discourse:redraw", function (event) {
    var incoming = event.detail;
    control.querySelector(".composer-controls").replaceWith(incoming.querySelector(".composer-controls"));
    var panel = control.querySelector(".submit-panel");
    var before = lead(panel);
    var rest = before.length ? before[before.length - 1].nextSibling : panel.firstChild;
    before.forEach(function (n) {
      n.remove();
    });
    lead(incoming.querySelector(".submit-panel")).forEach(function (n) {
      panel.insertBefore(n, rest);
    });
    var saving = submit.disabled;
    submit = control.querySelector(".save-or-cancel .create");
    submit.disabled = saving;
    previewToggle = control.querySelector(".toggle-preview");
    if (state) {
      submit.innerHTML = data["submit" + modeName(state.mode)];
      applyDiscardIcon(state.mode);
    }
    var toolbar = control.querySelector(".toggle-toolbar");
    if (toolbar) {
      toolbar.title = control.querySelector(".wmd-controls").classList.contains("toolbar-visible")
        ? data.labelHideToolbar
        : data.labelShowToolbar;
    }
    applyFullscreen();
    applyPreviewShown();
    updatePreview();
  });

  // d-editor's focus ring is on the wrapper.
  var wrapper = control.querySelector(".d-editor-textarea-wrapper");
  textarea.addEventListener("focus", function () {
    wrapper.classList.add("in-focus");
  });
  textarea.addEventListener("blur", function () {
    wrapper.classList.remove("in-focus");
  });

  textarea.addEventListener("input", scheduleDraft);
  title.addEventListener("input", scheduleDraft);

  textarea.addEventListener("keydown", function (event) {
    // Ctrl or Cmd + Enter submits, as in Ember.
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      save();
    }
  });

  // insertBlock (TextareaTextManipulation#_addBlock): the text on lines
  // of its own in place of the selection, the cursor after it.
  function insertBlock(text) {
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
    textarea.setSelectionRange(start + text.length, start + text.length);
    textarea.blur();
    textarea.focus({ preventScroll: true });
    updatePreview();
    scheduleDraft();
  }

  // TopicController#selectText from quote-button.js: into the open
  // composer at its cursor, after a minimized one's text, else a reply
  // (to the post unless it is the first) opened with the quote. Without
  // the right to reply Ember starts a new topic with it instead.
  document.addEventListener("composer:quote", function (event) {
    var d = event.detail || {};
    if (!control.isConnected || !d.quote) {
      return;
    }
    if (state && control.classList.contains("open")) {
      insertBlock(d.quote);
      return;
    }
    if (state && control.classList.contains("draft")) {
      textarea.value = textarea.value + "\n" + d.quote;
      expand();
      updatePreview();
      scheduleDraft();
      return;
    }
    if (!d.canCreatePost) {
      var message = data.labelUnported + " replying as a new topic";
      if (Discourse.dialog) {
        Discourse.dialog.alert(message);
      } else {
        window.alert(message);
      }
      return;
    }
    var first = parseInt(d.postNumber, 10) === 1;
    open({
      mode: "reply",
      topicId: d.topicId,
      replyTo: first ? undefined : d.postNumber,
      replyToUsername: first ? undefined : d.username,
      quote: d.quote,
    });
  });

  // The post's edit from quote-button.js (toggleFastEdit's editPost).
  document.addEventListener("composer:edit", function (event) {
    var d = event.detail || {};
    if (!control.isConnected || !d.postId) {
      return;
    }
    open({ mode: "edit", topicId: d.topicId, postId: d.postId, postNumber: d.postNumber });
  });

  // Openers anywhere on the page.
  document.addEventListener("click", function (event) {
    if (!control.isConnected) {
      return;
    }
    var create = event.target.closest("#create-topic");
    if (create) {
      open({ mode: "create-topic", category: create.dataset.category });
      return;
    }
    var topic = document.querySelector("#topic[data-topic-id]");
    var topicId = topic && topic.dataset.topicId;
    var edit = event.target.closest(".post-action-menu__edit");
    if (edit && topicId) {
      open({
        mode: "edit",
        topicId: topicId,
        postId: edit.dataset.postId,
        postNumber: edit.dataset.postNumber,
      });
      return;
    }
    var postReply = event.target.closest(".post-action-menu__reply");
    if (postReply && topicId) {
      open({
        mode: "reply",
        topicId: topicId,
        replyTo: postReply.dataset.postNumber,
        replyToUsername: postReply.dataset.username,
      });
      return;
    }
    if (
      topicId &&
      event.target.closest(".topic-footer-main-buttons .create, .reply-to-post")
    ) {
      open({ mode: "reply", topicId: topicId });
    }
  });

  // Uploads, as lib/uppy/composer-upload.js sends them (one POST to
  // /uploads.json per file, without Uppy) with TextareaPlaceholderHandler's
  // placeholders: "[Uploading: name…]()" at the cursor while the file goes,
  // replaced by getUploadMarkdown's markdown when it is stored. Validation
  // is the server's (its errors are shown); Ember also checks extensions and
  // sizes before sending.
  var fileInput = control.querySelector("#file-uploader");
  var uploadingLine = control.querySelector("#file-uploading");
  var uploads = [];
  var uploadSequence = 0;
  // upload-short-url.js's cache: short url to {url, short_path}.
  var shortUrls = {};

  function escapeRegExp(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  // insertText: at the cursor, replacing the selection.
  function insertText(text) {
    var start = textarea.selectionStart;
    textarea.setRangeText(text, start, textarea.selectionEnd, "end");
  }

  // replaceText: the first occurrence, keeping the cursor where it was
  // relative to the text around it.
  function replaceText(from, to) {
    var at = textarea.value.indexOf(from);
    if (at < 0) {
      return;
    }
    var cursor = textarea.selectionStart;
    textarea.setRangeText(to, at, at + from.length, "preserve");
    if (cursor > at) {
      var moved = Math.max(at + to.length, cursor + to.length - from.length);
      textarea.selectionStart = textarea.selectionEnd = moved;
    }
    updatePreview();
    scheduleDraft();
  }

  // #uploadPlaceholder: the file's name (numbered when a placeholder for
  // the same name is already there), on a line of its own.
  function uploadPlaceholder(file) {
    var name = file.name.replace(/[​-‍﻿]/g, "") || data.labelClipboard;
    var pattern = new RegExp(
      "\\[" +
        escapeRegExp(data.labelUploadingFilename).replace(
          "%\\{filename\\}",
          escapeRegExp(name) + "(?:\\()?([0-9])?(?:\\))?"
        ) +
        "\\]\\(\\)",
      "g"
    );
    var matches = textarea.value.match(pattern);
    if (matches) {
      var last = new RegExp(pattern.source).exec(matches[matches.length - 1]);
      name = name + "(" + (last[1] ? parseInt(last[1], 10) + 1 : 1) + ")";
    }
    var placeholder =
      "[" + data.labelUploadingFilename.replace("%{filename}", name) + "]()\n";
    var start = textarea.selectionStart;
    if (!(start === 0 || textarea.value.charAt(start - 1) === "\n")) {
      placeholder = "\n" + placeholder;
    }
    return placeholder;
  }

  // I18n.toHumanSize
  function humanSize(bytes) {
    var units = JSON.parse(data.sizeUnits);
    var size = bytes;
    var iterations = 0;
    while (size >= 1024 && iterations < 4) {
      size = size / 1024;
      iterations += 1;
    }
    var number;
    var unit;
    if (iterations === 0) {
      number = String(size);
      unit = size === 1 ? units.byte_one : units.byte_other;
    } else {
      number = size - Math.floor(size) === 0 ? size.toFixed(0) : size.toFixed(1);
      unit = units[[null, "kb", "mb", "gb", "tb"][iterations]];
    }
    return units.format.replace("%n", number).replace("%u", unit);
  }

  function markdownName(filename) {
    var name = filename.slice(0, filename.lastIndexOf("."));
    return name.replace(/\[|\]|\|/g, "");
  }

  // getUploadMarkdown
  function uploadMarkdown(upload) {
    var name = upload.original_filename;
    if (/\.(png|webp|jpe?g|gif|svg|ico|heic|heif|avif|jxl)$/i.test(name)) {
      return (
        "![" + markdownName(name) + "|" + upload.thumbnail_width + "x" +
        upload.thumbnail_height + "](" + (upload.short_url || upload.url) + ")"
      );
    }
    if (/\.(mp3|og[ga]|opus|wav|m4[abpr]|aac|flac)$/i.test(name)) {
      return "![" + markdownName(name) + "|audio](" + upload.short_url + ")";
    }
    if (/\.(mov|mp4|webm|m4v|3gp|ogv|avi|mpeg)$/i.test(name)) {
      return "![" + markdownName(name) + "|video](" + upload.short_url + ")";
    }
    return (
      "[" + name.replace(/\[|\]|\|/g, "") + "|attachment](" + upload.short_url +
      ") (" + humanSize(upload.filesize) + ")"
    );
  }

  // The progress line: the uploads' combined progress while they send,
  // "Processing upload" while the server stores them.
  function showProgress() {
    if (uploads.length === 0) {
      uploadingLine.hidden = true;
      return;
    }
    var sent = 0;
    var total = 0;
    uploads.forEach(function (u) {
      sent += u.sent;
      total += u.total;
    });
    var sending = uploads.some(function (u) {
      return u.sent < u.total;
    });
    var label = uploadingLine.querySelector("span");
    label.textContent = sending
      ? data.labelUploading + " " + Math.round((sent / Math.max(total, 1)) * 100) + "%"
      : data.labelProcessing;
    uploadingLine.querySelector("#cancel-file-upload").hidden = !sending;
    uploadingLine.hidden = false;
  }

  function finishUpload(upload) {
    uploads = uploads.filter(function (u) {
      return u !== upload;
    });
    showProgress();
  }

  function uploadFile(file, pasted) {
    var upload = {
      id: ++uploadSequence,
      sent: 0,
      total: file.size,
      placeholder: uploadPlaceholder(file),
      xhr: new XMLHttpRequest(),
    };
    uploads.push(upload);
    insertText(upload.placeholder);
    updatePreview();
    showProgress();

    var body = new FormData();
    body.append("upload_type", "composer");
    if (pasted) {
      body.append("pasted", "true");
    }
    body.append("file", file, file.name);
    var xhr = upload.xhr;
    xhr.open("POST", base + "/uploads.json?client_id=" + clientId);
    var headers = csrfHeaders();
    delete headers["Content-Type"];
    Object.keys(headers).forEach(function (k) {
      xhr.setRequestHeader(k, headers[k]);
    });
    xhr.upload.addEventListener("progress", function (e) {
      if (e.lengthComputable) {
        upload.sent = e.loaded;
        upload.total = e.total;
        showProgress();
      }
    });
    xhr.addEventListener("load", function () {
      var json = null;
      try {
        json = JSON.parse(xhr.responseText);
      } catch (e) {
        // An error page.
      }
      finishUpload(upload);
      if (xhr.status >= 200 && xhr.status < 300 && json && json.short_url) {
        // cacheShortUploadUrl: the preview needs no lookup for it.
        shortUrls[json.short_url] = { url: json.url, short_path: json.short_path };
        replaceText(upload.placeholder.trim(), uploadMarkdown(json));
      } else {
        replaceText(upload.placeholder, "");
        showError(
          json && json.errors ? json.errors.join(" ") : "Could not upload (" + xhr.status + ")."
        );
      }
    });
    xhr.addEventListener("error", function () {
      finishUpload(upload);
      replaceText(upload.placeholder, "");
      showError("Could not upload " + file.name + ".");
    });
    xhr.send(body);
  }

  function addFiles(files, pasted) {
    if (!state || files.length === 0) {
      return;
    }
    showError("");
    Array.prototype.forEach.call(files, function (file) {
      uploadFile(file, pasted);
    });
  }

  // The cancel link: every upload stopped and its placeholder removed.
  function cancelUploads() {
    uploads.slice().forEach(function (upload) {
      upload.xhr.abort();
      finishUpload(upload);
      replaceText(upload.placeholder, "");
    });
  }

  if (fileInput) {
    fileInput.addEventListener("change", function () {
      addFiles(fileInput.files, false);
      fileInput.value = "";
    });
    uploadingLine.querySelector("#cancel-file-upload").addEventListener("click", function (e) {
      e.preventDefault();
      cancelUploads();
    });
    // _pasteEventListener: files pasted without text are uploaded.
    textarea.addEventListener("paste", function (event) {
      var clipboard = event.clipboardData;
      if (!clipboard) {
        return;
      }
      var types = Array.prototype.slice.call(clipboard.types);
      if (!types.includes("Files") || types.includes("text/plain")) {
        return;
      }
      if (clipboard.files && clipboard.files.length > 0) {
        event.preventDefault();
        addFiles(clipboard.files, true);
      }
    });
    // DropTarget: files dropped anywhere on the composer.
    control.addEventListener("dragover", function (event) {
      if (event.dataTransfer && Array.prototype.includes.call(event.dataTransfer.types, "Files")) {
        event.preventDefault();
      }
    });
    control.addEventListener("drop", function (event) {
      if (event.dataTransfer && event.dataTransfer.files.length > 0) {
        event.preventDefault();
        addFiles(event.dataTransfer.files, false);
      }
    });
  }

  // resolveAllShortUrls on the preview: upload:// sources and links
  // pointed at the uploads, from the cache or one lookup for the rest.
  var pendingLookup = null;
  function resolveShortUrls() {
    var elements = preview.querySelectorAll("img[data-orig-src], source[data-orig-src], a[data-orig-href]");
    var missing = [];
    elements.forEach(function (el) {
      var attr = el.hasAttribute("data-orig-href") ? "data-orig-href" : "data-orig-src";
      var short = el.getAttribute(attr);
      var cached = shortUrls[short];
      if (!cached) {
        if (missing.indexOf(short) < 0) {
          missing.push(short);
        }
        return;
      }
      if (cached.url === "missing") {
        return;
      }
      el.removeAttribute(attr);
      if (attr === "data-orig-href") {
        el.href = cached.short_path;
      } else {
        el.src = cached.url;
      }
    });
    if (missing.length === 0 || pendingLookup) {
      return;
    }
    pendingLookup = fetch(base + "/uploads/lookup-urls", {
      method: "POST",
      headers: csrfHeaders(),
      credentials: "same-origin",
      body: form(
        missing.map(function (u) {
          return ["short_urls[]", u];
        })
      ),
    })
      .then(function (r) {
        return r.ok ? r.json() : [];
      })
      .then(function (found) {
        found.forEach(function (u) {
          shortUrls[u.short_url] = { url: u.url, short_path: u.short_path };
        });
        missing.forEach(function (u) {
          shortUrls[u] = shortUrls[u] || { url: "missing", short_path: "missing" };
        });
      })
      .catch(function () {
        // Unresolved: the placeholders stay.
      })
      .finally(function () {
        pendingLookup = null;
        resolveShortUrls();
      });
  }

  // TopicDraftsDropdown: the member's latest drafts in a DMenu under the
  // trigger beside New Topic. A draft on a topic goes to the topic (its
  // postUrl, which takes the draft's post id for a post number, as
  // Ember's does); a new topic's opens the composer on it.
  var DRAFTS_LIMIT = 4;
  var draftsMenu = null;

  function closeDraftsMenu() {
    if (!draftsMenu) {
      return;
    }
    draftsMenu.trigger.setAttribute("aria-expanded", "false");
    draftsMenu.trigger.classList.remove("-expanded");
    draftsMenu.content.remove();
    draftsMenu = null;
  }

  // lib/utilities' postUrl
  function draftPostUrl(draft) {
    var url = base + "/t/" + (draft.slug ? draft.slug + "/" : "topic/") + draft.topic_id;
    var postNumber = draft.data.postId || null;
    if (postNumber > 1) {
      url += "/" + postNumber;
    }
    return url;
  }

  function draftIcon(key) {
    var name = key.indexOf("new_topic") === 0
      ? "layer-group"
      : key.indexOf("new_private_message") === 0
        ? "envelope"
        : "reply";
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  function escapeHtml(s) {
    var div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }

  function openDraftsMenu(trigger) {
    var labels = trigger.dataset;
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu topic-drafts-menu-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.content = "";
    content.dataset.identifier = "topic-drafts-menu";
    content.dataset.strategy = "absolute";
    content.dataset.placement = "bottom-end";
    content.style.visibility = "hidden";
    content.innerHTML = '<div class="fk-d-menu__inner-content"><ul class="dropdown-menu"></ul></div>';
    portals.appendChild(content);
    trigger.setAttribute("aria-expanded", "true");
    trigger.classList.add("-expanded");
    draftsMenu = { trigger: trigger, content: content, drafts: [] };
    var menu = draftsMenu;

    // UserDraftsStream's first page.
    fetch(base + "/drafts.json?offset=0&limit=30", { credentials: "same-origin" })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (result) {
        if (draftsMenu !== menu) {
          return;
        }
        var drafts = (result.drafts || []).slice(0, DRAFTS_LIMIT).map(function (d) {
          d.data = JSON.parse(d.data);
          if (d.draft_key.indexOf("new_topic") === 0 || d.draft_key.indexOf("new_private_message") === 0) {
            d.title = d.data.title;
          }
          return d;
        });
        menu.drafts = drafts;
        var html = drafts
          .map(function (d, i) {
            return (
              '<li class="dropdown-menu__item topic-drafts-item"><button class="btn btn-icon-text" type="button" data-draft-index="' +
              i + '">' + draftIcon(d.draft_key) +
              '<span class="d-button-label">' + escapeHtml(d.title || labels.labelUntitled) +
              "</span></button></li>"
            );
          })
          .join("");
        var count = parseInt(labels.draftCount, 10);
        if (count > DRAFTS_LIMIT) {
          var other = count - DRAFTS_LIMIT;
          var otherText = (other === 1 ? labels.labelOtherDraftsOne : labels.labelOtherDraftsOther).replace(
            "%{count}",
            other
          );
          html +=
            '<li><hr class="dropdown-menu__divider"></li><li class="dropdown-menu__item"><a class="btn btn-link view-all-drafts" href="' +
            base + '/my/activity/drafts"><span data-other-drafts="' + other + '">' +
            escapeHtml(otherText) + "</span><span>" + escapeHtml(labels.labelViewAll) + "</span></a></li>";
        }
        content.querySelector(".dropdown-menu").innerHTML = html;
        // bottom-end, ten pixels below the trigger.
        var rect = trigger.getBoundingClientRect();
        content.style.left = rect.right + window.scrollX - content.offsetWidth + "px";
        content.style.top = rect.bottom + window.scrollY + 10 + "px";
        content.style.visibility = "visible";
      })
      .catch(function (e) {
        console.error("Failed to fetch drafts with error:", e);
        closeDraftsMenu();
      });
  }

  document.addEventListener("click", function (event) {
    if (!control.isConnected) {
      return;
    }
    var trigger = event.target.closest(".topic-drafts-menu-trigger");
    if (trigger) {
      if (draftsMenu) {
        closeDraftsMenu();
      } else {
        openDraftsMenu(trigger);
      }
      return;
    }
    if (!draftsMenu) {
      return;
    }
    var item = event.target.closest(".topic-drafts-item [data-draft-index]");
    if (item) {
      var draft = draftsMenu.drafts[parseInt(item.dataset.draftIndex, 10)];
      closeDraftsMenu();
      if (draft.topic_id) {
        window.location.href = draftPostUrl(draft);
      } else {
        open({
          mode: "create-topic",
          draftKey: draft.draft_key,
          draftSequence: draft.sequence,
        });
      }
    } else if (!event.target.closest(".fk-d-menu")) {
      closeDraftsMenu();
    }
  });
  document.addEventListener("keydown", function (event) {
    if (control.isConnected && event.key === "Escape") {
      closeDraftsMenu();
    }
  });

  // DVirtualHeight, on a phone or an iPad: --composer-vh follows the
  // visual viewport, and html.keyboard-visible says the on-screen keyboard
  // took more than 150px of it.
  if (window.visualViewport && (root.classList.contains("mobile-view") || root.classList.contains("ipados-device"))) {
    var previousHeight = null;
    var viewportTimer = null;
    var onViewportResize = function () {
      // isZoomed
      if (window.visualViewport.scale > 1) {
        return;
      }
      var height = Math.round(window.visualViewport.height);
      if (previousHeight && Math.abs(previousHeight - height) <= 1) {
        return;
      }
      previousHeight = height;
      root.style.setProperty("--composer-vh", height / 100 + "px");
      root.classList.toggle("keyboard-visible", window.innerHeight - window.visualViewport.height > 150);
    };
    onViewportResize();
    window.visualViewport.addEventListener("resize", function () {
      clearTimeout(viewportTimer);
      viewportTimer = setTimeout(onViewportResize, 50);
    });
  }

  // setupComposerPosition on a phone or an iPad: the page scrolled so the
  // composer's top went above the window comes back (correctScrollPosition),
  // and a drag in an editor with nothing to scroll doesn't scroll the page.
  if (root.classList.contains("mobile-device") || root.classList.contains("ipados-device")) {
    window.addEventListener("scroll", function () {
      if (!control.classList.contains("open")) {
        return;
      }
      setTimeout(function () {
        var rect = control.getBoundingClientRect();
        if (rect.top < -1) {
          window.scrollTo({ top: window.scrollY + rect.top, behavior: "instant" });
        }
      }, 150);
    });
    textarea.addEventListener("touchmove", function (event) {
      if (event.touches.length > 1) {
        return;
      }
      if (textarea.scrollHeight <= textarea.clientHeight && window.getSelection().toString() === "") {
        event.preventDefault();
        event.stopPropagation();
      }
    }, { passive: false });
  }

  window.composer = { open: open };
})();
