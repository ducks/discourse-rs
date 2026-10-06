// The composer (#reply-control, rendered by composer_view): opens to
// reply to a topic or a post, create a topic or edit a post, and posts
// through /posts as Ember's composer does. Its height, sizing, minimizing
// and fullscreen follow services/composer.js and composer-container; the
// toolbar applies its markdown as d-editor's buttons do.
(function () {
  "use strict";

  var control = document.getElementById("reply-control");
  if (!control) {
    return;
  }
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

  function setMode(mode) {
    ["reply", "create-topic", "edit"].forEach(function (m) {
      control.classList.toggle("composer-action-" + m, m === mode);
    });
    control.classList.toggle("edit-title", mode === "create-topic");
    control.classList.toggle("topic", mode === "create-topic");
    fields.hidden = mode !== "create-topic";
    var key = mode.replace("-", "");
    var camel = { reply: "Reply", createtopic: "CreateTopic", edit: "Edit" }[key];
    actionButton.innerHTML = data["action" + camel];
    submit.innerHTML = data["submit" + camel];
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
    if (s.mode === "create-topic") {
      d.title = title.value;
      var category = chooser.querySelector("summary").dataset.value;
      d.categoryId = category ? parseInt(category, 10) : null;
    } else if (s.mode === "edit") {
      d.postId = parseInt(s.postId, 10);
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
    if (!draftTimer || !canSaveDraft()) {
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
    if (s.mode === "reply" && editDraft && draft.postId) {
      s.mode = "edit";
      s.postId = draft.postId;
      setMode("edit");
    }
    if (s.mode === "edit") {
      if (editDraft && String(draft.postId) === String(s.postId)) {
        return Promise.resolve(draft.reply || "");
      }
      return rawToEdit(s);
    }
    if (draft && !editDraft) {
      if (s.mode === "create-topic") {
        title.value = draft.title || "";
        if (draft.categoryId) {
          selectCategory(draft.categoryId);
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
  function open(opts) {
    if (state && state.mode === opts.mode && control.classList.contains("draft")) {
      expand();
      return;
    }
    state = opts;
    var s = state;
    setMode(opts.mode);
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
    // A new topic's key is new: there is no draft to look up.
    var lookup =
      opts.mode === "create-topic" && !resuming
        ? Promise.resolve(null)
        : fetchDraft(s.draftKey);
    lookup
      .then(function (found) {
        return initialText(s, found);
      })
      .then(function (text) {
        if (state !== s) {
          return;
        }
        textarea.value = text;
        updatePreview();
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
    clearTimeout(draftTimer);
    draftTimer = null;
    control.classList.remove("open", "draft", "fullscreen");
    control.classList.add("closed");
    document.body.classList.remove("fullscreen-composer");
    state = null;
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
    var on = !control.classList.contains("fullscreen");
    control.classList.toggle("fullscreen", on);
    document.body.classList.toggle("fullscreen-composer", on);
    var button = control.querySelector(".toggle-fullscreen");
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

  function previewShown() {
    return (getItem("composer.showPreview") || "true") === "true";
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
        } catch (e) {
          preview.textContent = e.message;
        }
      })
      .catch(function (e) {
        preview.textContent = "The preview could not load: " + e.message;
      });
  }

  function schedulePreview() {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(updatePreview, 30);
  }

  function applyPreviewShown() {
    if (!previewToggle) {
      return;
    }
    var shown = previewShown();
    control.classList.toggle("show-preview", shown);
    control.classList.toggle("hide-preview", !shown);
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
    chooser.querySelectorAll(".category-row").forEach(function (r) {
      r.classList.toggle("is-selected", r === row);
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

  function form(pairs) {
    return pairs
      .map(function (p) {
        return encodeURIComponent(p[0]) + "=" + encodeURIComponent(p[1]);
      })
      .join("&");
  }

  function save() {
    if (!state) {
      return;
    }
    var request;
    if (state.mode === "edit") {
      request = fetch(base + "/posts/" + state.postId, {
        method: "PUT",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form([["post[raw]", textarea.value]]),
      });
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

  // Openers anywhere on the page.
  document.addEventListener("click", function (event) {
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
    if (event.key === "Escape") {
      closeDraftsMenu();
    }
  });

  window.composer = { open: open };
})();
