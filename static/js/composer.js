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

  // open({mode, topicId, replyTo, replyToUsername, postId, postNumber,
  // category}): to reply, create a topic or edit a post.
  function open(opts) {
    if (state && state.mode === opts.mode && control.classList.contains("draft")) {
      expand();
      return;
    }
    state = opts;
    setMode(opts.mode);
    if (opts.mode === "reply" && opts.replyToUsername) {
      actionButton.querySelector(".d-button-label").textContent =
        opts.replyToUsername;
    }
    textarea.value = "";
    title.value = "";
    showError("");
    if (opts.mode === "create-topic") {
      selectCategory(opts.category || data.defaultCategory);
    }
    control.classList.remove("closed", "draft");
    control.classList.add("open");
    root.style.setProperty("--composer-height", defaultHeight(opts.mode));
    updatePreview();
    if (opts.mode === "edit") {
      textarea.disabled = true;
      fetch(base + "/raw/" + opts.topicId + "/" + opts.postNumber, {
        credentials: "same-origin",
      })
        .then(function (r) {
          if (!r.ok) {
            throw new Error(r.status);
          }
          return r.text();
        })
        .then(function (raw) {
          textarea.value = raw;
          textarea.disabled = false;
          textarea.focus();
          updatePreview();
        })
        .catch(function () {
          textarea.disabled = false;
          showError("Could not load the post to edit.");
        });
    } else if (opts.mode === "create-topic") {
      title.focus();
    } else {
      textarea.focus();
    }
  }

  function close() {
    control.classList.remove("open", "draft", "fullscreen");
    control.classList.add("closed");
    document.body.classList.remove("fullscreen-composer");
    state = null;
  }

  function dirty() {
    return textarea.value.trim() !== "" || title.value.trim() !== "";
  }

  // Minimized, the composer is a bar that opens again when clicked.
  function minimize() {
    control.classList.remove("open", "fullscreen");
    control.classList.add("draft");
    document.body.classList.remove("fullscreen-composer");
  }

  function expand() {
    control.classList.remove("draft");
    control.classList.add("open");
  }

  function toggleFullscreen() {
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
      var pairs = [["raw", textarea.value]];
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
      // Drafts are not kept yet: closing keeps the text for this page.
      minimize();
    } else if (event.target.closest(".discard-button")) {
      if (!dirty() || window.confirm("Discard what you have written?")) {
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

  window.composer = { open: open };
})();
