// discourse-user-notes for staff: any element with data-user-notes (the
// post's notes icon, "Add user note" in the post admin menu, the user
// card's, profile's, admin user page's and review item's buttons) opens
// the notes modal (UserNotesModal), drawn by the server at
// /user-notes/:user_id. Its form adds a note (POST /user_notes), its
// trash buttons remove one after a confirmation (DELETE /user_notes/:id),
// and the modal is drawn again after each. The opener's count follows,
// as the callbacks showUserNotes is given update theirs. A review note
// sent with "Copy note" checked is copied to the author's notes
// (the reviewablenote:created handler).
(function () {
  "use strict";

  if (!window.Discourse || !Discourse.once || !Discourse.once("user-notes")) {
    return;
  }

  // user_notes_icon_placement, from the script tag.
  var script = document.currentScript;
  var avatarPlacement = !!script && script.dataset.placement === "avatar";

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function headers() {
    var h = {};
    try {
      h = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    h["X-Requested-With"] = "XMLHttpRequest";
    return h;
  }

  function request(method, path, form) {
    var h = headers();
    var init = { method: method, headers: h, credentials: "same-origin" };
    if (form) {
      h["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      init.body = new URLSearchParams(form).toString();
    }
    return fetch(base() + path, init);
  }

  // popupAjaxError
  function popupError(r) {
    return r
      .json()
      .catch(function () {
        return {};
      })
      .then(function (body) {
        var text = (body.errors || [r.statusText]).join("\n");
        if (Discourse.dialog) {
          Discourse.dialog.popupAjaxError(text);
        } else {
          window.alert(text);
        }
      });
  }

  function confirmDelete(message, didConfirm) {
    if (Discourse.dialog && Discourse.dialog.deleteConfirm) {
      Discourse.dialog.deleteConfirm({ message: message, didConfirm: didConfirm });
    } else if (window.confirm(message)) {
      didConfirm();
    }
  }

  // The opener's callback: the profile and admin buttons relabel, the
  // review button and the post icon show while there are notes.
  function updateCount(opener, count) {
    if (!opener) {
      return;
    }
    var kind = opener.dataset.userNotesKind;
    if (kind === "profile" || kind === "admin") {
      var label = opener.querySelector(".d-button-label");
      if (label) {
        label.textContent =
          count > 0 ? opener.dataset.labelShow.replace("%{count}", count) : opener.dataset.labelTitle;
      }
    } else if (kind === "flags") {
      opener.hidden = count <= 0;
      opener.title = opener.dataset.labelShow.replace("%{count}", count);
    } else if (opener.dataset.postId) {
      var article = document.querySelector('article[data-post-id="' + opener.dataset.postId + '"]');
      var icon = article && article.querySelector(".user-notes-icon");
      if (icon) {
        icon.hidden = count <= 0;
      } else if (article && count > 0 && opener.iconTemplate) {
        var names = article.querySelector(avatarPlacement ? ".topic-avatar" : ".names");
        if (names) {
          var span = document.createElement("span");
          span.className = "user-notes-icon";
          span.dataset.userNotes = opener.dataset.userNotes;
          span.dataset.postId = opener.dataset.postId;
          span.innerHTML = opener.iconTemplate;
          names.appendChild(span);
        }
      }
    }
  }

  function load(userId, postId) {
    var path = "/user-notes/" + encodeURIComponent(userId) + (postId ? "?post_id=" + encodeURIComponent(postId) : "");
    return request("GET", path).then(function (r) {
      if (!r.ok) {
        return popupError(r).then(function () {
          return null;
        });
      }
      return r.text();
    });
  }

  function fill(bodyEl, html, opener) {
    bodyEl.innerHTML = html;
    var root = bodyEl.querySelector(".user-notes-modal-body");
    var template = root.querySelector("template.user-notes-icon-template");
    if (opener && template) {
      opener.iconTemplate = template.innerHTML;
    }
    var textarea = root.querySelector("textarea");
    if (textarea) {
      textarea.focus();
    }
    return root;
  }

  function show(opener) {
    var userId = opener.dataset.userNotes;
    var postId = opener.dataset.postId || "";
    load(userId, postId).then(function (html) {
      if (html === null || html === undefined) {
        return;
      }
      var holder = document.createElement("div");
      holder.innerHTML = html;
      var title = (holder.querySelector(".user-notes-modal-body") || { dataset: {} }).dataset.title || "";
      Discourse.modal.show({
        className: "user-notes-modal",
        title: title,
        body: function (bodyEl) {
          var root = fill(bodyEl, html, opener);
          wire(bodyEl, root, opener);
        },
      });
    });
  }

  function redraw(bodyEl, root, opener) {
    return load(root.dataset.userId, root.dataset.postId).then(function (html) {
      if (html === null || html === undefined) {
        return;
      }
      var next = fill(bodyEl, html, opener);
      wire(bodyEl, next, opener);
      updateCount(opener, next.querySelectorAll(".user-note").length);
    });
  }

  function wire(bodyEl, root, opener) {
    var form = root.querySelector("form.user-notes-form");
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      var textarea = form.querySelector("textarea");
      // validation="required:trim"
      if (!textarea.value.trim()) {
        textarea.focus();
        return;
      }
      var submit = form.querySelector("button[type=submit]");
      submit.disabled = true;
      var data = { "user_note[raw]": textarea.value, "user_note[user_id]": root.dataset.userId };
      if (root.dataset.postId) {
        data["user_note[post_id]"] = root.dataset.postId;
      }
      request("POST", "/user_notes", data).then(function (r) {
        if (!r.ok) {
          submit.disabled = false;
          return popupError(r);
        }
        return redraw(bodyEl, root, opener);
      });
    });
    root.querySelectorAll(".user-note .remove-user-note").forEach(function (button) {
      button.addEventListener("click", function () {
        var note = button.closest(".user-note");
        confirmDelete(root.dataset.deleteConfirm, function () {
          request(
            "DELETE",
            "/user_notes/" + encodeURIComponent(note.dataset.noteId) + "?user_id=" + encodeURIComponent(root.dataset.userId)
          ).then(function (r) {
            if (!r.ok) {
              return popupError(r);
            }
            return redraw(bodyEl, root, opener);
          });
        });
      });
    });
  }

  document.addEventListener("click", function (event) {
    var opener = event.target.closest && event.target.closest("[data-user-notes]");
    if (!opener || !Discourse.modal) {
      return;
    }
    event.preventDefault();
    show(opener);
  });

  // handleReviewableNoteCreation: review.js tells of each note it adds.
  document.addEventListener("reviewablenote:created", function (event) {
    var detail = event.detail || {};
    var form = detail.form;
    var box = form && form.querySelector("input.copy-note-to-user");
    if (!box || !box.checked || !box.dataset.userId) {
      return;
    }
    request("POST", "/user_notes", {
      "user_note[user_id]": box.dataset.userId,
      "user_note[raw]": (detail.content || "").trim(),
      "user_note[reviewable_id]": box.dataset.reviewableId,
    }).then(function (r) {
      if (!r.ok) {
        return popupError(r);
      }
      box.checked = false;
    });
  });
})();
