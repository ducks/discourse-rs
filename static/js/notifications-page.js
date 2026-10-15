// The notifications pages under the profile (UserNotificationsController):
// Dismiss All (marking every notification read, after
// DismissNotificationConfirmationModal while high priority ones are
// unread), the NotificationsFilter dropdown (the route's ?filter), and a
// notification's click marking it read as MenuItem does.
(function () {
  "use strict";

  if (!Discourse.once("notifications-page")) {
    return;
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function meta(name) {
    return (document.querySelector('meta[name="' + name + '"]') || {}).content || "";
  }

  function strings() {
    var el = document.getElementById("notifications-page-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      return {};
    }
  }

  function markRead(id) {
    return fetch(meta("discourse-base-uri") + "/notifications/mark-read.json", {
      method: "PUT",
      credentials: "same-origin",
      keepalive: true,
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": meta("csrf-token"),
        "X-Requested-With": "XMLHttpRequest",
      },
      body: id ? "id=" + encodeURIComponent(id) : "",
    });
  }

  function dismissAll(button) {
    markRead(null).then(function (response) {
      if (!response.ok) {
        return;
      }
      document.querySelectorAll(".user-notifications-list li.notification.unread").forEach(function (li) {
        li.classList.remove("unread");
        li.classList.add("read");
      });
      button.disabled = true;
    });
  }

  function confirmDismiss(button) {
    var s = strings();
    Discourse.modal.show({
      className: "dismiss-notification-confirmation",
      hideHeader: true,
      body: function (body) {
        body.textContent = s.confirm || "";
      },
      footer: function (footer, closeModal) {
        var ok = document.createElement("button");
        ok.className = "btn btn-icon-text btn-primary";
        ok.type = "button";
        ok.innerHTML = icon("check") + '<span class="d-button-label"></span>';
        ok.querySelector(".d-button-label").textContent = s.dismiss || "Dismiss";
        ok.addEventListener("click", function () {
          closeModal();
          dismissAll(button);
        });
        var cancel = document.createElement("button");
        cancel.className = "btn btn-text btn-default";
        cancel.type = "button";
        cancel.innerHTML = '<span class="d-button-label"></span>';
        cancel.querySelector(".d-button-label").textContent = s.cancel || "Cancel";
        cancel.addEventListener("click", function () {
          closeModal();
        });
        footer.appendChild(ok);
        footer.appendChild(cancel);
      },
    });
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(".user-notifications-filter .notifications-filter")) {
        return;
      }
      var open = chooser.open;
      chooser.classList.toggle("is-expanded", open);
      var caret = chooser.querySelector(".angle-icon");
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : "angle-down", "angle-icon");
      }
      var header = chooser.querySelector("summary");
      var body = chooser.querySelector(".select-kit-body");
      if (!open || !body || !window.FloatingUIDOM) {
        return;
      }
      var ui = window.FloatingUIDOM;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = Math.round(header.getBoundingClientRect().width) + "px";
      ui.computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
      }).then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      });
    },
    true
  );

  document.addEventListener("click", function (event) {
    var dismiss = event.target.closest(".user-notifications-page .dismiss-notifications");
    if (dismiss) {
      if ((strings().high_priority || 0) > 0) {
        confirmDismiss(dismiss);
      } else {
        dismissAll(dismiss);
      }
      return;
    }
    var row = event.target.closest(".user-notifications-filter .select-kit-row");
    document.querySelectorAll(".user-notifications-filter details[open]").forEach(function (chooser) {
      if (!chooser.contains(event.target) || row) {
        chooser.open = false;
      }
    });
    if (row) {
      var url = new URL(window.location.href);
      if (row.dataset.value === "all") {
        url.searchParams.delete("filter");
      } else {
        url.searchParams.set("filter", row.dataset.value);
      }
      window.location.href = url.toString();
      return;
    }
    var item = event.target.closest(".user-notifications-list li.notification.unread[data-notification-id] a");
    if (item) {
      var li = item.closest("li");
      li.classList.remove("unread");
      li.classList.add("read");
      markRead(li.dataset.notificationId);
    }
  });
})();
