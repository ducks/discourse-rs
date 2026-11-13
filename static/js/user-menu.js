// The header's user menu (UserDropdown, UserMenuWrapper, UserMenu): the
// avatar opens and closes it, rendered by the server (GET /user-menu?tab=,
// crate::user_menu_view) after the header's icons; a tab other than the
// current one loads in place (a click on the current one, or one asking
// for a new window, follows its link); an item closes the menu as it goes,
// an unread notification asking the server to mark it read on the way
// (the cn cookie, as UserMenuNotificationItem#onClick sets it); Dismiss
// marks the tab's notifications read, asking first while important ones
// are unread (DismissNotificationConfirmationModal); the profile tab
// toggles presence, sets the user status (user-status.js), pauses
// notifications (DoNotDisturbModal) or leaves do not disturb, and logs
// out. A click outside,
// on the mobile view's cloak, or Escape closes it.
(function () {
  "use strict";

  if (!Discourse.once("user-menu")) {
    return;
  }

  function config() {
    return document.querySelector("script[data-user-menu]");
  }

  function toggle() {
    return document.getElementById("toggle-current-user");
  }

  function wrapper() {
    return document.querySelector(".d-header .user-menu-dropdown-wrapper");
  }

  function csrf() {
    var meta = document.querySelector('meta[name="csrf-token"]');
    return meta ? meta.content : "";
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function close() {
    var open = wrapper();
    if (open) {
      open.remove();
    }
    var button = toggle();
    if (button) {
      button.removeAttribute("aria-expanded");
      button.closest("li").classList.remove("active");
    }
  }

  function showError(message) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = message || "Something went wrong.";
      error.hidden = false;
    }
  }

  // Renders the menu on the given tab, in place of an open one.
  function load(tab) {
    var script = config();
    if (!script) {
      return Promise.resolve();
    }
    var url = script.dataset.userMenu + (tab ? "?tab=" + encodeURIComponent(tab) : "");
    return fetch(url, { credentials: "same-origin" })
      .then(function (response) {
        return response.text().then(function (body) {
          if (!response.ok) {
            var message;
            try {
              message = (JSON.parse(body).errors || []).join(" ");
            } catch (_) {}
            throw new Error(message || "Something went wrong.");
          }
          return body;
        });
      })
      .then(function (html) {
        var button = toggle();
        if (!button) {
          return;
        }
        var open = wrapper();
        var template = document.createElement("template");
        template.innerHTML = html;
        var menu = template.content.firstElementChild;
        if (open) {
          open.replaceWith(menu);
        } else {
          button.closest("ul").insertAdjacentElement("afterend", menu);
        }
        // The panel's links swap pages in as the body's do (hx-boost);
        // the tabs load in place.
        if (window.htmx) {
          window.htmx.process(menu.querySelector(".quick-access-panel"));
        }
        button.setAttribute("aria-expanded", "");
        button.closest("li").classList.add("active");
        // UserMenu#focusFirstTab
        var active = menu.querySelector(".top-tabs .btn.active");
        if (active && !tab) {
          active.focus({ preventScroll: true });
        }
      })
      .catch(function (e) {
        showError(e.message);
      });
  }

  function request(method, url, body) {
    var headers = {
      "X-CSRF-Token": csrf(),
      "X-Requested-With": "XMLHttpRequest",
      Accept: "application/json",
    };
    var options = { method: method, credentials: "same-origin", headers: headers };
    if (body) {
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = new URLSearchParams(body).toString();
    }
    return fetch(url, options).then(function (response) {
      if (!response.ok) {
        return response.text().then(function (text) {
          var message;
          try {
            message = (JSON.parse(text).errors || []).join(" ");
          } catch (_) {}
          throw new Error(message || "Something went wrong.");
        });
      }
      return response;
    });
  }

  function currentTab() {
    var menu = document.querySelector(".d-header .user-menu");
    return menu ? menu.dataset.tabId : null;
  }

  // UserMenuNotificationsList#performDismiss, then the list again.
  function dismiss(button) {
    var types = button.dataset.dismissTypes;
    request("PUT", basePath() + "/notifications/mark-read", types ? { dismiss_types: types } : null)
      .then(function () {
        return load(currentTab());
      })
      .catch(function (e) {
        showError(e.message);
      });
  }

  function confirmDismiss(button) {
    var t = Discourse.cooked ? Discourse.cooked.t : null;
    var label = function (key, fallback) {
      var value = t ? t(key) : null;
      return value && value !== key ? value : fallback;
    };
    Discourse.modal.show({
      className: "dismiss-notification-confirmation",
      hideHeader: true,
      body: function (body) {
        body.textContent = button.dataset.confirm;
      },
      footer: function (footer, closeModal) {
        var ok = document.createElement("button");
        ok.className = "btn btn-icon-text btn-primary";
        ok.type = "button";
        ok.innerHTML =
          '<svg class="fa d-icon d-icon-check svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#check"></use></svg><span class="d-button-label"></span>';
        ok.querySelector(".d-button-label").textContent = label("notifications.dismiss_confirmation.dismiss", "Dismiss");
        ok.addEventListener("click", function () {
          closeModal();
          dismiss(button);
        });
        var cancel = document.createElement("button");
        cancel.className = "btn btn-text btn-default";
        cancel.type = "button";
        cancel.innerHTML = '<span class="d-button-label"></span>';
        cancel.querySelector(".d-button-label").textContent = label("notifications.dismiss_confirmation.cancel", "Cancel");
        cancel.addEventListener("click", function () {
          closeModal();
        });
        footer.appendChild(ok);
        footer.appendChild(cancel);
      },
    });
  }

  // doNotDisturbClick's saving
  var leavingDoNotDisturb = false;

  // DoNotDisturbModal (modal/do-not-disturb.gjs): a DTapTile per duration,
  // which enterDoNotDisturbFor saves (the header's moon follows over the
  // live stream, as updateDoNotDisturbStatus draws it), and the button to
  // the notification schedule preferences.
  function doNotDisturbModal(model) {
    Discourse.modal.show({
      className: "do-not-disturb-modal",
      title: model.title,
      body: function (body, closeModal) {
        // @flash, as DFlashMessage draws it above the body.
        function flash(message) {
          var alert = body.parentNode.querySelector("#modal-alert");
          if (!alert) {
            alert = document.createElement("div");
            alert.id = "modal-alert";
            alert.setAttribute("role", "alert");
            alert.className = "alert";
            body.parentNode.insertBefore(alert, body);
          }
          alert.textContent = message;
        }
        function saveDuration(duration) {
          request("POST", basePath() + "/do-not-disturb.json", { duration: duration })
            .then(function () {
              closeModal();
            })
            .catch(function (e) {
              flash(e.message);
            });
        }
        var grid = document.createElement("div");
        grid.className = "tap-tile-grid";
        model.options.forEach(function (option) {
          var tile = document.createElement("div");
          tile.setAttribute("role", "button");
          tile.tabIndex = 0;
          tile.setAttribute("aria-pressed", "false");
          tile.className = "do-not-disturb-tile tap-tile";
          tile.id = "tap_tile_" + option[0];
          tile.textContent = option[1];
          tile.addEventListener("click", function () {
            saveDuration(option[0]);
          });
          tile.addEventListener("keydown", function (event) {
            if (event.key === "Enter") {
              event.stopPropagation();
              saveDuration(option[0]);
            }
          });
          grid.appendChild(tile);
        });
        body.appendChild(grid);
        var schedule = document.createElement("button");
        schedule.className = "btn btn-text btn-default";
        schedule.type = "button";
        schedule.innerHTML = '<span class="d-button-label"></span>';
        schedule.querySelector(".d-button-label").textContent = model.set_schedule;
        // navigateToNotificationSchedule
        schedule.addEventListener("click", function () {
          closeModal();
          window.location.href = model.schedule_url;
        });
        body.appendChild(schedule);
      },
    });
  }

  function wantsNewWindow(event) {
    return event.shiftKey || event.metaKey || event.ctrlKey || event.button === 1;
  }

  document.addEventListener("click", function (event) {
    var target = event.target;
    if (target.closest("#toggle-current-user")) {
      event.preventDefault();
      if (wrapper()) {
        close();
      } else {
        load(null);
      }
      return;
    }
    var open = wrapper();
    if (!open) {
      return;
    }
    // The cloak and anything outside the menu close it.
    if (target.classList.contains("header-cloak") || !target.closest(".user-menu-dropdown-wrapper")) {
      if (!target.closest(".modal-container")) {
        close();
      }
      return;
    }
    var tab = target.closest(".user-menu-tab");
    if (tab) {
      if (wantsNewWindow(event) || tab.classList.contains("active")) {
        if (tab.getAttribute("href")) {
          close();
        }
        return;
      }
      event.preventDefault();
      load(tab.id.replace(/^user-menu-button-/, ""));
      return;
    }
    var dismissButton = target.closest(".notifications-dismiss");
    if (dismissButton) {
      if (dismissButton.dataset.confirm && Discourse.modal) {
        confirmDismiss(dismissButton);
      } else {
        dismiss(dismissButton);
      }
      return;
    }
    var username = config().dataset.username;
    if (target.closest(".presence-toggle button")) {
      // togglePresence: the user option, saved.
      var hidden = !target.closest(".presence-toggle").classList.contains("enabled");
      request("PUT", basePath() + "/u/" + encodeURIComponent(username) + ".json", { hide_presence: hidden ? "false" : "true" })
        .then(function () {
          return load("profile");
        })
        .catch(function (e) {
          showError(e.message);
        });
      return;
    }
    var dnd = target.closest(".do-not-disturb");
    if (dnd && target.closest("button")) {
      // doNotDisturbClick: leaveDoNotDisturb while paused, else the modal.
      if (dnd.classList.contains("enabled")) {
        if (leavingDoNotDisturb) {
          return;
        }
        leavingDoNotDisturb = true;
        request("DELETE", basePath() + "/do-not-disturb.json")
          .then(function () {
            return load("profile");
          })
          .catch(function (e) {
            showError(e.message);
          })
          .then(function () {
            leavingDoNotDisturb = false;
          });
      } else {
        var modal = dnd.dataset.modal ? JSON.parse(dnd.dataset.modal) : null;
        close();
        if (modal && Discourse.modal) {
          doNotDisturbModal(modal);
        }
      }
      return;
    }
    if (target.closest(".set-user-status button")) {
      // setUserStatusClick: UserStatusModal (user-status.js) with the
      // member's status, saved or cleared by the UserStatusService.
      var item = target.closest(".set-user-status");
      var status = item.dataset.status ? JSON.parse(item.dataset.status) : null;
      close();
      if (!Discourse.userStatus) {
        return;
      }
      Discourse.userStatus.open({
        status: status,
        pauseNotifications: !!document.querySelector("#toggle-current-user .do-not-disturb-background"),
        saveAction: function (newStatus, pause) {
          return Discourse.userStatus.set(newStatus, pause).then(function () {
            Discourse.userStatus.drawBubble(newStatus);
          });
        },
        deleteAction: function () {
          return Discourse.userStatus.clear().then(function () {
            Discourse.userStatus.drawBubble(null);
          });
        },
      });
      return;
    }
    if (target.closest(".enable-anonymous button, .disable-anonymous button")) {
      request("POST", basePath() + "/u/toggle-anon")
        .then(function () {
          window.location.reload();
        })
        .catch(function (e) {
          showError(e.message);
        });
      return;
    }
    if (target.closest(".logout button")) {
      // The logout route action: the session gone, then the site's root.
      request("DELETE", basePath() + "/session/" + encodeURIComponent(username))
        .then(function () {
          window.location.href = basePath() + "/";
        })
        .catch(function (e) {
          showError(e.message);
        });
      return;
    }
    var link = target.closest("a[href]");
    if (link) {
      var item = link.closest("li.notification.unread");
      var id = item && item.dataset.notificationId;
      if (id) {
        document.cookie = "cn=" + id + "; path=" + (basePath() || "/");
      }
      if (!wantsNewWindow(event)) {
        close();
      }
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape" && wrapper()) {
      close();
      var button = toggle();
      if (button) {
        button.focus();
      }
    }
  });
})();
