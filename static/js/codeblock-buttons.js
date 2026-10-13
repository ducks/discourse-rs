// Copy and fullscreen buttons on a post's code blocks, as Ember's
// codeblock-buttons initializer (onlyStream, when
// show_copy_button_on_codeblocks) attaches lib/codeblock-buttons.js:
// pre.codeblock-buttons gets div.codeblock-button-wrapper with the copy
// button and, on the desktop view, the fullscreen one, which shows the code
// in the fullscreen-code modal (with only a copy button of its own).
// Syntax highlighting isn't ported, so the modal's code is plain.
(function () {
  "use strict";

  var Discourse = window.Discourse;
  if (!Discourse || !Discourse.decorateCooked || Discourse.codeblockButtons) {
    return;
  }
  Discourse.codeblockButtons = true;

  var t = function (key) {
    return Discourse.cooked.t(key);
  };

  // A code block directly in the element or in anything but a nested post
  // or a quote.
  function getCodeBlocks(element) {
    return element.querySelectorAll(
      ":scope > pre > code, :scope :not(article):not(blockquote) > pre > code"
    );
  }

  function createButtons(codeBlocks, opts) {
    var pendingOffsets = [];

    codeBlocks.forEach(function (codeBlock) {
      var wrapperEl = document.createElement("div");
      wrapperEl.classList.add("codeblock-button-wrapper");

      codeBlock.parentElement.classList.add("codeblock-buttons");
      codeBlock.before(wrapperEl);

      if (opts.showCopy) {
        var copyButton = document.createElement("button");
        copyButton.classList.add("btn", "nohighlight", "copy-cmd", "btn-flat");
        copyButton.setAttribute("aria-label", t("copy_codeblock.copy"));
        copyButton.innerHTML = Discourse.cooked.icon("copy");
        wrapperEl.appendChild(copyButton);
        pendingOffsets.push([wrapperEl, codeBlock]);
      }

      if (opts.showFullscreen) {
        var fullscreenButton = document.createElement("button");
        fullscreenButton.classList.add("btn", "nohighlight", "fullscreen-cmd", "btn-flat");
        fullscreenButton.setAttribute("aria-label", t("copy_codeblock.fullscreen"));
        fullscreenButton.innerHTML = Discourse.cooked.icon("discourse-expand");
        wrapperEl.appendChild(fullscreenButton);
      }
    });

    // Every scrollbar gutter is read before any is written back, so the
    // layout is computed once.
    var gutters = pendingOffsets.map(function (pair) {
      return pair[1].offsetWidth - pair[1].clientWidth;
    });
    pendingOffsets.forEach(function (pair, index) {
      pair[0].style.right = gutters[index] + "px";
    });
  }

  // clipboardCopy
  function clipboardCopy(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return navigator.clipboard.writeText(text);
    }
    var textArea = document.createElement("textarea");
    textArea.value = text;
    textArea.style.position = "fixed";
    textArea.style.opacity = "0";
    document.body.appendChild(textArea);
    textArea.select();
    var ok = false;
    try {
      ok = document.execCommand("copy");
    } catch (e) {
      ok = false;
    }
    textArea.remove();
    return ok;
  }

  var fadeRunners = new WeakMap();

  // The button says it's copied for 3 seconds, from the latest copy.
  function copyComplete(button) {
    button.classList.add("action-complete");
    var state = button.innerHTML;
    button.innerHTML = t("copy_codeblock.copied");
    clearTimeout(fadeRunners.get(button));
    fadeRunners.set(
      button,
      setTimeout(function () {
        button.classList.remove("action-complete");
        button.innerHTML = state;
        fadeRunners.delete(button);
      }, 3000)
    );
  }

  // FullscreenCodeModal
  function showFullscreenCode(code, codeClasses) {
    Discourse.modal.show({
      className: "fullscreen-code-modal --max",
      title: t("copy_codeblock.view_code"),
      body: function (body) {
        var pre = document.createElement("pre");
        var codeEl = document.createElement("code");
        if (codeClasses) {
          codeEl.className = codeClasses;
        }
        codeEl.textContent = code;
        pre.appendChild(codeEl);
        body.appendChild(pre);
        attach(body, { showFullscreen: false, showCopy: true });
      },
    });
  }

  function handleClick(event) {
    var button = event.target;
    var isCopy = button.classList.contains("copy-cmd");
    var isFullscreen = button.classList.contains("fullscreen-cmd");
    if (!isCopy && !isFullscreen) {
      return;
    }
    var codeEl = button.parentElement.parentElement.querySelector("code");
    if (!codeEl) {
      return;
    }
    // Odd whitespace characters become plain spaces.
    var text = codeEl.innerText
      .replace(/[\f\v   -   　﻿]/g, " ")
      .trim();

    if (isCopy) {
      var result = clipboardCopy(text);
      if (result && result.then) {
        result.then(function () {
          copyComplete(button);
        });
      } else if (result) {
        copyComplete(button);
      }
    } else {
      showFullscreenCode(text, codeEl.className);
    }
  }

  function attach(element, opts) {
    var codeBlocks = getCodeBlocks(element);
    if (!codeBlocks.length) {
      return;
    }
    createButtons(codeBlocks, opts);
    element.addEventListener("click", handleClick, false);
  }

  if (!Discourse.cooked.setting("show_copy_button_on_codeblocks")) {
    return;
  }

  Discourse.decorateCooked("codeblock-buttons", function (el) {
    // onlyStream: a post of the topic's stream.
    if (!el.closest("article[data-post-id]")) {
      return;
    }
    attach(el, { showFullscreen: true, showCopy: true });
  });
})();
