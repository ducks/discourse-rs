// The profile's Invites tab (controllers/user-invited/show.js): removing
// an invite, copying its link, removing the expired ones, sending the
// pending ones again, and the search. The invite modals (creating,
// editing, bulk) are not ported.
(function () {
  "use strict";

  if (!Discourse.once("user-invited")) {
    return;
  }

  function strings() {
    var el = document.getElementById("user-invited-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (e) {
      return {};
    }
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function csrf() {
    return (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
  }

  function request(method, url, data) {
    var body = Object.keys(data || {})
      .map(function (k) {
        return encodeURIComponent(k) + "=" + encodeURIComponent(data[k]);
      })
      .join("&");
    return fetch(base() + url, {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
      },
      body: body,
    }).then(function (response) {
      return response
        .json()
        .catch(function () {
          return {};
        })
        .then(function (json) {
          if (!response.ok) {
            var message = (json.errors && json.errors[0]) || json.error || response.statusText;
            throw new Error(message);
          }
          return json;
        });
    });
  }

  function popupAjaxError(error) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(error.message || String(error));
    }
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function closeMenus(except) {
    document.querySelectorAll(".invites-menu-content").forEach(function (menu) {
      if (menu !== except) {
        menu.hidden = true;
        var trigger = menu.parentNode.querySelector(".invites-menu-trigger");
        if (trigger) {
          trigger.setAttribute("aria-expanded", "false");
        }
      }
    });
  }

  function init() {
    var content = document.getElementById("user-content");
    if (!content || !content.closest(".user-invites-page, body") || content.dataset.ready) {
      return;
    }
    if (!document.getElementById("user-invited-strings")) {
      return;
    }
    content.dataset.ready = "true";
    var S = strings();

    content.addEventListener("click", function (event) {
      var target = event.target.closest("button");
      if (!target) {
        return;
      }
      var row = target.closest("tr[data-invite-id]");
      if (target.matches(".invite-button, .edit-invite")) {
        unsupported("the invite modal");
      } else if (target.matches(".bulk-invite-button")) {
        unsupported("bulk invites from a CSV file");
      } else if (target.matches(".invites-menu-trigger")) {
        var menu = target.parentNode.querySelector(".invites-menu-content");
        closeMenus(menu);
        menu.hidden = !menu.hidden;
        target.setAttribute("aria-expanded", menu.hidden ? "false" : "true");
      } else if (target.matches(".remove-invite") && row) {
        // Invite#destroy, then the row leaves the list.
        request("DELETE", "/invites", { id: row.dataset.inviteId })
          .then(function () {
            row.remove();
          })
          .catch(popupAjaxError);
      } else if (target.matches(".copy-invite-link") && row) {
        var input = row.querySelector(".invite-link-target");
        if (input && navigator.clipboard) {
          navigator.clipboard.writeText(input.value).then(function () {
            target.querySelector(".d-button-label").textContent = target.dataset.copied;
          });
        }
      } else if (target.matches(".bulk-remove-expired")) {
        Discourse.dialog.deleteConfirm({
          message: S.remove_all_confirm,
          didConfirm: function () {
            request("POST", "/invites/destroy-all-expired", { username: S.username })
              .then(function () {
                location.reload();
              })
              .catch(popupAjaxError);
          },
        });
      } else if (target.matches(".reinvite-all")) {
        Discourse.dialog.yesNoConfirm({
          message: S.reinvite_all_confirm,
          didConfirm: function () {
            request("POST", "/invites/reinvite-all", {})
              .then(function () {
                var done = document.createElement("span");
                done.className = "reinvited-all";
                done.innerHTML =
                  '<button class="btn btn-icon-text btn-default" disabled type="button"><svg class="fa d-icon d-icon-check svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#check"></use></svg><span class="d-button-label"></span></button>';
                done.querySelector(".d-button-label").textContent = S.reinvited_all;
                target.replaceWith(done);
              })
              .catch(popupAjaxError);
          },
        });
      }
    });
    document.addEventListener("click", function (event) {
      if (!event.target.closest(".invite-actions")) {
        closeMenus(null);
      }
    });

    // _searchTermChanged: the list again for the term, after a pause.
    var search = content.querySelector(".user-invite-search input");
    if (search) {
      var timer = null;
      var form = search.closest("form");
      form.addEventListener("submit", function (event) {
        event.preventDefault();
      });
      search.addEventListener("input", function () {
        clearTimeout(timer);
        timer = setTimeout(function () {
          var url = new URL(window.location.href);
          if (search.value) {
            url.searchParams.set("search", search.value);
          } else {
            url.searchParams.delete("search");
          }
          window.location.href = url.toString();
        }, 250);
      });
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
  document.addEventListener("htmx:afterSettle", init);
})();
