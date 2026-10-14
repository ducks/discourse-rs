// The dialog service (dialog-holder) as the admin pages use it:
//
//   Discourse.dialog.alert(message)                  OK
//   Discourse.dialog.confirm({ message, title, didConfirm, confirmLabel,
//                              cancelLabel, confirmClass })  OK and Cancel
//
// rendered as DialogHolder renders them into #dialog-holder, closed by
// its buttons, the close button, the overlay or Escape. The labels come
// from the page's #dialog-strings (ok_value, cancel, modal.close).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.dialog) {
    return;
  }

  function strings() {
    var el = document.getElementById("dialog-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      return {};
    }
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function close() {
    var holder = document.getElementById("dialog-holder");
    if (holder) {
      holder.remove();
    }
    document.documentElement.classList.remove("dialog-open");
    document.removeEventListener("keydown", onKey);
  }

  function onKey(event) {
    if (event.key === "Escape") {
      close();
    }
  }

  function button(cls, label) {
    return (
      '<button class="btn btn-text ' +
      cls +
      '" type="button"><span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  function show(options) {
    close();
    var s = strings();
    var holder = document.createElement("div");
    holder.id = "dialog-holder";
    holder.className = "dialog-container " + (options.class || "");
    holder.setAttribute("aria-labelledby", "dialog-title");
    var html = '<div class="dialog-overlay" data-a11y-dialog-hide></div><div class="dialog-content" role="document">';
    if (options.title) {
      html +=
        '<div class="dialog-header"><h3 id="dialog-title">' +
        escapeHtml(options.title) +
        '</h3><button class="btn no-text btn-icon btn-flat dialog-close close" title="' +
        escapeHtml(s["modal.close"] || "close") +
        '" type="button"><svg class="fa d-icon d-icon-xmark svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#xmark"></use></svg><span aria-hidden="true"> &#8203; </span></button></div>';
    }
    if (options.message) {
      html += '<div class="dialog-body"><p>' + options.message + "</p></div>";
    }
    html +=
      '<div class="dialog-footer">' +
      button(options.confirmClass || "btn-primary", options.confirmLabel || s.ok_value || "OK");
    if (options.cancel) {
      html += button("btn-default", options.cancelLabel || s.cancel || "Cancel");
    }
    html += "</div></div>";
    holder.innerHTML = html;
    var root = document.querySelector(".discourse-root") || document.body;
    root.appendChild(holder);
    document.documentElement.classList.add("dialog-open");
    document.addEventListener("keydown", onKey);
    var buttons = holder.querySelectorAll(".dialog-footer .btn");
    buttons[0].focus();
    buttons[0].addEventListener("click", function () {
      close();
      if (options.didConfirm) {
        options.didConfirm();
      }
    });
    if (buttons[1]) {
      buttons[1].addEventListener("click", close);
    }
    holder.querySelectorAll(".dialog-overlay, .dialog-close").forEach(function (el) {
      el.addEventListener("click", close);
    });
  }

  Discourse.dialog = {
    alert: function (message) {
      show({ message: message });
    },
    confirm: function (options) {
      show(Object.assign({ cancel: true }, options));
    },
  };
})();
