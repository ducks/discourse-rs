// The dialog service (dialog-holder: services/dialog.js and
// components/dialog-holder.gjs) for the server-rendered pages:
//
//   Discourse.dialog.alert(message | { message, title, didConfirm,
//                                      didCancel, buttons })   OK
//   Discourse.dialog.confirm({ message, title, didConfirm, didCancel,
//                              confirmLabel, cancelLabel, confirmClass })
//                                                     OK and Cancel
//   Discourse.dialog.yesNoConfirm({ ... })            Yes and No
//   Discourse.dialog.deleteConfirm({ ... })           Delete (btn-danger)
//                                                     and Cancel
//   Discourse.dialog.popupAjaxError(text)             an ajax error's alert
//
// rendered as DialogHolder renders them into #dialog-holder, closed by
// its buttons, the close button, the overlay or Escape (the last three
// cancel). `buttons` replaces the footer's: { label, class, icon,
// action }. The labels come from the page's #dialog-strings (ok_value,
// cancel, yes_value, no_value, modal.close).
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

  var cancelCurrent = null;

  function close(cancelled) {
    var holder = document.getElementById("dialog-holder");
    if (holder) {
      holder.remove();
    }
    document.documentElement.classList.remove("dialog-open");
    document.removeEventListener("keydown", onKey);
    var cancel = cancelCurrent;
    cancelCurrent = null;
    if (cancelled && cancel) {
      cancel();
    }
  }

  function onKey(event) {
    if (event.key === "Escape") {
      close(true);
    }
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function button(cls, label, iconName) {
    return (
      '<button class="btn ' +
      (iconName ? "btn-icon-text " : "") +
      cls +
      '" type="button">' +
      (iconName ? icon(iconName) : "") +
      '<span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  function show(options) {
    close(false);
    var s = strings();
    var holder = document.createElement("div");
    holder.id = "dialog-holder";
    holder.className = "dialog-container " + (options.class || "");
    holder.setAttribute("aria-labelledby", "dialog-title");
    holder.setAttribute("aria-modal", "true");
    holder.setAttribute("tabindex", "-1");
    holder.setAttribute("role", "dialog");
    var html = '<div class="dialog-overlay" data-a11y-dialog-hide></div><div class="dialog-content" role="document">';
    if (options.title) {
      html +=
        '<div class="dialog-header"><h3 id="dialog-title">' +
        escapeHtml(options.title) +
        '</h3><button class="btn no-text btn-icon btn-flat dialog-close close" title="' +
        escapeHtml(s["modal.close"] || "close") +
        '" type="button">' +
        icon("xmark") +
        '<span aria-hidden="true"> &#8203; </span></button></div>';
    }
    if (options.message) {
      html += '<div class="dialog-body"><p>' + options.message + "</p></div>";
    }
    html += '<div class="dialog-footer">';
    var actions = [];
    if (options.buttons) {
      options.buttons.forEach(function (b) {
        html += button(b.class || "btn-default", b.label, b.icon);
        actions.push(b.action || null);
      });
    } else {
      html += button(options.confirmClass || "btn-primary", options.confirmLabel || s.ok_value || "OK");
      actions.push(options.didConfirm || null);
      if (options.cancel) {
        html += button("btn-default", options.cancelLabel || s.cancel || "Cancel");
        actions.push("cancel");
      }
    }
    html += "</div></div>";
    holder.innerHTML = html;
    var root = document.querySelector(".discourse-root") || document.body;
    root.appendChild(holder);
    document.documentElement.classList.add("dialog-open");
    document.addEventListener("keydown", onKey);
    cancelCurrent = options.didCancel || null;
    var buttons = holder.querySelectorAll(".dialog-footer .btn");
    buttons.forEach(function (el, i) {
      el.addEventListener("click", function () {
        if (actions[i] === "cancel") {
          close(true);
          return;
        }
        close(false);
        if (actions[i]) {
          actions[i]();
        }
      });
    });
    if (buttons[0]) {
      buttons[0].focus();
    }
    holder.querySelectorAll(".dialog-overlay, .dialog-close").forEach(function (el) {
      el.addEventListener("click", function () {
        close(true);
      });
    });
  }

  Discourse.dialog = {
    alert: function (options) {
      show(typeof options === "string" ? { message: options } : options);
    },
    confirm: function (options) {
      show(Object.assign({ cancel: true }, options));
    },
    yesNoConfirm: function (options) {
      var s = strings();
      show(
        Object.assign({ cancel: true }, options, {
          confirmLabel: s.yes_value || "Yes",
          cancelLabel: s.no_value || "No",
        })
      );
    },
    deleteConfirm: function (options) {
      var s = strings();
      show(
        Object.assign({ cancel: true }, options, {
          confirmClass: "btn-danger",
          confirmLabel: options.confirmLabel || s.delete || "Delete",
        })
      );
    },
    popupAjaxError: function (text) {
      show({ message: escapeHtml(text) });
    },
  };
})();
