// The profile's Invites tab (controllers/user-invited/show.js): removing
// an invite, copying its link, removing the expired ones, sending the
// pending ones again, and the search. Invite and an invite's Edit open
// CreateInvite (create-invite-modal.js), Bulk Invite opens CreateInviteBulk
// (components/modal/create-invite-bulk.gjs, here).
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

  // CreateInviteBulk over CreateInviteUploader: a CSV file chosen and
  // sent to POST /invites/upload_csv (the button counting the progress),
  // then the word that a message follows.
  function bulkInviteModal(S) {
    if (!Discourse.modal) {
      return;
    }
    var bodyEl = null;
    var footerEl = null;
    var file = null;
    var uploading = false;
    var uploaded = false;
    var progress = 0;

    function draw() {
      if (uploaded) {
        bodyEl.textContent = S.bulk_success;
      } else {
        var input = bodyEl.querySelector("#csv-file");
        if (input) {
          input.disabled = uploading;
        }
      }
      var label = uploading ? S.bulk_progress.split("%{progress}").join(progress) : S.bulk_text;
      footerEl.innerHTML =
        (uploaded
          ? ""
          : '<button class="btn btn-icon-text btn-primary bulk-invite-upload"' +
            (!file || uploading ? ' disabled=""' : "") +
            ' type="button">' +
            S.link_icon +
            '<span class="d-button-label"></span></button>') +
        '<button class="btn btn-text btn-primary bulk-invite-close" type="button"><span class="d-button-label"></span></button>';
      var upload = footerEl.querySelector(".bulk-invite-upload .d-button-label");
      if (upload) {
        upload.textContent = label;
      }
      footerEl.querySelector(".bulk-invite-close .d-button-label").textContent = S.close;
    }

    // startUpload: the file as uppy's XHR upload sends it.
    function start() {
      if (!file || uploading) {
        return;
      }
      uploading = true;
      progress = 0;
      draw();
      var form = new FormData();
      form.append("type", "csv");
      form.append("name", file.name);
      form.append("file", file, file.name);
      var xhr = new XMLHttpRequest();
      xhr.open("POST", base() + "/invites/upload_csv.json");
      xhr.setRequestHeader("Accept", "application/json");
      xhr.setRequestHeader("X-CSRF-Token", csrf());
      xhr.setRequestHeader("X-Requested-With", "XMLHttpRequest");
      xhr.upload.addEventListener("progress", function (event) {
        if (event.lengthComputable && event.total > 0) {
          progress = Math.round((event.loaded / event.total) * 100);
          draw();
        }
      });
      var failed = function () {
        uploading = false;
        var message = xhr.statusText;
        try {
          var json = JSON.parse(xhr.responseText);
          message = (json.errors && json.errors[0]) || json.error || json.message || message;
        } catch (e) {
          // The status text stands.
        }
        draw();
        popupAjaxError(new Error(message));
      };
      xhr.addEventListener("load", function () {
        if (xhr.status >= 200 && xhr.status < 300) {
          // uploadDone
          uploading = false;
          uploaded = true;
          draw();
        } else {
          failed();
        }
      });
      xhr.addEventListener("error", failed);
      xhr.send(form);
    }

    var close = Discourse.modal.show({
      className: "create-invite-bulk-modal --large",
      title: S.bulk_text,
      body: function (body) {
        bodyEl = body;
      },
      footer: function (footer) {
        footerEl = footer;
      },
    });
    if (!bodyEl || !footerEl) {
      return;
    }
    // The locale's own markup (trustHTML).
    bodyEl.innerHTML = S.bulk_instructions + '<input accept=".csv" id="csv-file" type="file">';
    bodyEl.querySelector("#csv-file").addEventListener("change", function (event) {
      var chosen = event.target.files && event.target.files[0];
      // validateUploadedFile's csvOnly.
      if (chosen && !/\.csv$/i.test(chosen.name)) {
        event.target.value = "";
        chosen = null;
        Discourse.dialog.alert(S.bulk_error);
      }
      file = chosen || null;
      draw();
    });
    footerEl.addEventListener("click", function (event) {
      var button = event.target.closest("button");
      if (!button) {
        return;
      }
      if (button.classList.contains("bulk-invite-upload")) {
        start();
      } else if (button.classList.contains("bulk-invite-close")) {
        close();
      }
    });
    draw();
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
      if (target.matches(".invite-button")) {
        // CreateInvite (create-invite-modal.js); the list is drawn again
        // once the modal that saved one has closed.
        if (Discourse.createInviteModal) {
          var made = false;
          Discourse.createInviteModal.open({
            onSaved: function () {
              made = true;
            },
            onClose: function () {
              if (made) {
                location.reload();
              }
            },
          });
        }
      } else if (target.matches(".edit-invite") && row) {
        // editInvite: the modal for the row's invite; the list is drawn
        // again once one that saved it has closed.
        var editing = null;
        try {
          editing = JSON.parse(row.dataset.invite || "null");
        } catch (e) {
          editing = null;
        }
        if (Discourse.createInviteModal && editing) {
          var saved = false;
          Discourse.createInviteModal.open({
            editing: true,
            invite: editing,
            onSaved: function () {
              saved = true;
            },
            onClose: function () {
              if (saved) {
                location.reload();
              }
            },
          });
        }
      } else if (target.matches(".bulk-invite-button")) {
        // createInviteCsv
        bulkInviteModal(S);
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
