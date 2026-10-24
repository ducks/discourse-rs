// The admin's page for one user (AdminUserIndexController) in the
// browser, its actions through the admin user endpoints: log out, edit
// the name and title (AdminEditableField, PUT /u/:username), activate and
// deactivate, approve, grant and revoke admin and moderation, change and
// lock the trust level, suspend and silence (PenalizeUser, in
// static/js/penalize-user.js), unsuspend and unsilence, clear the penalty
// history, the primary group, impersonate, anonymize, delete. The page is
// loaded again after a change, as the controller refreshes its model.
// Not ported, and saying so when used: the username's edit, checking the
// email, the bounce score's reset, the activation email, the IP lookup,
// disabling two-factor, the upcoming changes and merge modals, the custom
// groups' picker, deleting every post, and the user archive export.
(function () {
  "use strict";

  if (!Discourse.once("admin-user")) {
    return;
  }

  var strings = {};

  function t(key, args) {
    var s = strings[key];
    if (s === undefined) {
      return "[en." + key + "]";
    }
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  function page() {
    return document.querySelector(".admin-contents.admin-user");
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : "";
  }

  function ajax(method, path, data) {
    var options = {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
    };
    if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = new URLSearchParams(data).toString();
    }
    return fetch(base() + path, options).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (_) {}
        if (!response.ok || (json && json.failed)) {
          var message = (json && json.errors && json.errors.join(" ")) || (json && json.message) || t("generic_error");
          throw new Error(message);
        }
        return json;
      });
    });
  }

  function failed(e) {
    Discourse.dialog.alert(String(e.message));
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function refresh() {
    location.reload();
  }

  function userId() {
    return page().dataset.userId;
  }

  function username() {
    return page().dataset.username;
  }

  // adminTools.showSuspendModal / showSilenceModal with the page's
  // AdminUser (static/js/penalize-user.js), the page loaded again after.
  function showPenalty(type) {
    var el = document.getElementById("penalize-user-config");
    var user = null;
    try {
      user = el ? JSON.parse(el.textContent).user : null;
    } catch (e) {
      user = null;
    }
    if (!user || !Discourse.penalizeUser) {
      unsupported("the " + type + " modal");
      return;
    }
    Discourse.penalizeUser.show({ penaltyType: type, user: user, successCallback: refresh });
  }

  function put(action, data) {
    return ajax("PUT", "/admin/users/" + userId() + "/" + action, data);
  }

  // AdminEditableField: the value becomes an input with save and cancel.
  function startEditing(row) {
    if (row.querySelector("input")) {
      return;
    }
    var value = row.querySelector(".value");
    var current = value.querySelector("span").textContent;
    value.innerHTML = "";
    var input = document.createElement("input");
    input.type = "text";
    input.value = current;
    value.appendChild(input);
    var controls = row.querySelector(".controls");
    controls.dataset.original = controls.innerHTML;
    controls.innerHTML =
      '<button class="btn no-text btn-icon btn-primary btn-small submit-edit" type="button">' +
      icon("check") +
      '<span aria-hidden="true"> &#8203; </span></button><button class="btn no-text btn-icon btn-small cancel-edit" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true"> &#8203; </span></button>';
    input.focus();
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

  // A ComboBox's rows, opened under its header: choosing one shows the
  // save and cancel buttons beside it.
  function openCombo(details) {
    var body = details.querySelector(".select-kit-body");
    if (details.classList.contains("is-expanded")) {
      details.classList.remove("is-expanded");
      body.innerHTML = "";
      return;
    }
    var options = JSON.parse(details.dataset.options || "[]");
    var current = details.querySelector("summary").dataset.value;
    details.classList.add("is-expanded");
    body.innerHTML =
      '<div class="select-kit-collection" role="listbox">' +
      options
        .map(function (o) {
          var selected = String(o.id) === String(current);
          return (
            '<div class="select-kit-row' +
            (selected ? " is-selected" : "") +
            '" role="option" data-value="' +
            o.id +
            '" data-name="' +
            String(o.name).replace(/"/g, "&quot;") +
            '" tabindex="0"><span class="name">' +
            String(o.name).replace(/&/g, "&amp;").replace(/</g, "&lt;") +
            "</span></div>"
          );
        })
        .join("") +
      "</div>";
  }

  function chooseCombo(details, row) {
    var summary = details.querySelector("summary");
    var name = row.dataset.name;
    summary.dataset.original = summary.dataset.original || summary.dataset.value;
    summary.dataset.value = row.dataset.value;
    summary.dataset.name = name;
    details.querySelector(".select-kit-selected-name .name").textContent = name;
    details.classList.remove("is-expanded");
    details.querySelector(".select-kit-body").innerHTML = "";
    var value = details.closest(".value");
    if (!value.querySelector(".combo-buttons")) {
      var div = document.createElement("div");
      div.className = "combo-buttons";
      div.innerHTML =
        '<button class="btn no-text btn-icon ok" type="button">' +
        icon("check") +
        '<span aria-hidden="true"> &#8203; </span></button><button class="btn no-text btn-icon cancel" type="button">' +
        icon("xmark") +
        '<span aria-hidden="true"> &#8203; </span></button>';
      value.appendChild(div);
    }
  }

  Discourse.onPage(function () {
    var json = document.getElementById("admin-user-strings");
    strings = json ? JSON.parse(json.textContent) : {};
  });

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var target = event.target;
    var editable = target.closest("[data-editable]");
    if (editable && (target.closest(".inline-editable-field") || target.closest(".controls .btn-default"))) {
      event.preventDefault();
      if (editable.dataset.editable === "username") {
        unsupported("changing a username");
        return;
      }
      startEditing(editable);
      return;
    }
    if (editable && target.closest(".cancel-edit")) {
      refresh();
      return;
    }
    if (editable && target.closest(".submit-edit")) {
      var field = editable.dataset.editable;
      var data = {};
      data[field] = editable.querySelector("input").value;
      ajax("PUT", "/u/" + encodeURIComponent(username()) + ".json", data).then(refresh, failed);
      return;
    }
    var combo = target.closest("details.select-kit.combo-box");
    if (combo) {
      event.preventDefault();
      if (combo.classList.contains("is-disabled")) {
        return;
      }
      var choice = target.closest(".select-kit-row");
      if (choice) {
        chooseCombo(combo, choice);
      } else if (target.closest("summary")) {
        openCombo(combo);
      }
      return;
    }
    var ok = target.closest(".combo-buttons .ok");
    if (ok) {
      var details = ok.closest(".value").querySelector("details");
      var value = details.querySelector("summary").dataset.value;
      if (details.classList.contains("change-trust-level-dropdown")) {
        put("trust_level", { level: value }).then(refresh, function (e) {
          Discourse.dialog.alert(e.message || t("admin.user.trust_level_change_failed"));
        });
      } else {
        put("primary_group", { primary_group_id: value || "" }).then(refresh, failed);
      }
      return;
    }
    if (target.closest(".combo-buttons .cancel")) {
      refresh();
      return;
    }
    if (target.closest(".group-chooser summary")) {
      event.preventDefault();
      unsupported("the custom groups' picker");
      return;
    }
    var deleteBox = target.closest("details.btn-user-delete");
    if (deleteBox) {
      event.preventDefault();
      var body = deleteBox.querySelector(".select-kit-body");
      var option = target.closest(".select-kit-row");
      if (option) {
        var flags = JSON.parse(option.dataset.flags || "{}");
        Discourse.dialog.confirm({
          title: t("admin.user.delete_confirm_title"),
          message: t("admin.user.delete_confirm"),
          confirmLabel: t("admin.user.delete"),
          confirmClass: "btn-danger",
          class: "delete-user-modal",
          didConfirm: function () {
            ajax("DELETE", "/admin/users/" + userId() + ".json", Object.assign({ context: location.pathname }, flags)).then(function () {
              location.assign(base() + "/admin/users/list/active");
            }, failed);
          },
        });
        return;
      }
      if (deleteBox.classList.contains("is-expanded")) {
        deleteBox.classList.remove("is-expanded");
        body.innerHTML = "";
      } else {
        deleteBox.classList.add("is-expanded");
        body.innerHTML = document.getElementById("admin-user-delete-options").innerHTML;
      }
      return;
    }
    var button = target.closest("button, a.btn");
    if (!button) {
      return;
    }
    var label = (button.querySelector(".d-button-label") || button).textContent.trim();
    var is = function (key) {
      return label === t(key);
    };
    if (is("admin.user.action_logs")) {
      location.assign(base() + "/admin/logs/staff_action_logs?filters=" + encodeURIComponent(JSON.stringify({ target_user: username() })));
    } else if (is("admin.user.log_out")) {
      ajax("POST", "/admin/users/" + userId() + "/log_out").then(function () {
        Discourse.dialog.alert(t("admin.user.logged_out"));
      }, failed);
    } else if (is("admin.user.deactivate_account")) {
      put("deactivate").then(refresh, failed);
    } else if (is("admin.user.activate")) {
      put("activate").then(refresh, failed);
    } else if (is("admin.user.approve")) {
      put("approve").then(refresh, failed);
    } else if (is("admin.user.revoke_admin")) {
      put("revoke_admin").then(refresh, failed);
    } else if (is("admin.user.grant_admin")) {
      put("grant_admin").then(function (result) {
        if (result && result.email_confirmation_required) {
          Discourse.dialog.alert(t("admin.user.grant_admin_confirm"));
        } else {
          refresh();
        }
      }, failed);
    } else if (is("admin.user.revoke_moderation")) {
      put("revoke_moderation").then(refresh, failed);
    } else if (is("admin.user.grant_moderation")) {
      put("grant_moderation").then(refresh, failed);
    } else if (is("admin.user.lock_trust_level") || is("admin.user.unlock_trust_level")) {
      put("trust_level_lock", { locked: is("admin.user.lock_trust_level") ? "true" : "false" }).then(refresh, failed);
    } else if (is("admin.user.unsuspend")) {
      put("unsuspend").then(refresh, failed);
    } else if (is("admin.user.unsilence")) {
      put("unsilence").then(refresh, failed);
    } else if (is("admin.user.clear_penalty_history.title")) {
      ajax("DELETE", "/admin/users/" + userId() + "/penalty_history").then(refresh, failed);
    } else if (is("admin.impersonate.title")) {
      ajax("POST", "/admin/impersonate", { username_or_email: username() }).then(function () {
        location.assign(base() + "/");
      }, function (e) {
        Discourse.dialog.alert(e.message || t("admin.impersonate.error"));
      });
    } else if (is("admin.user.anonymize")) {
      Discourse.dialog.confirm({
        message: t("admin.user.anonymize_confirm"),
        confirmLabel: t("admin.user.anonymize_yes"),
        confirmClass: "btn-danger",
        didConfirm: function () {
          put("anonymize").then(function (result) {
            if (result && result.username) {
              location.assign(base() + "/admin/users/" + userId() + "/" + result.username);
            } else {
              Discourse.dialog.alert(t("admin.user.anonymize_failed"));
            }
          }, failed);
        },
      });
    } else if (is("admin.user.suspend")) {
      showPenalty("suspend");
    } else if (is("admin.user.silence")) {
      showPenalty("silence");
    } else if (is("admin.users.check_email.text") || is("admin.users.check_sso.text")) {
      unsupported("revealing the email");
    } else if (is("admin.user.reset_bounce_score.label")) {
      unsupported("resetting the bounce score");
    } else if (is("admin.user.send_activation_email")) {
      unsupported("the activation email");
    } else if (is("admin.user.ip_lookup")) {
      unsupported("the IP lookup");
    } else if (is("user.second_factor.disable")) {
      unsupported("disabling two-factor authentication");
    } else if (is("admin.user.upcoming_changes.view_modal")) {
      unsupported("the user's upcoming changes modal");
    } else if (is("admin.user.merge.button")) {
      unsupported("the merge modal");
    } else if (is("admin.user.delete_posts.button")) {
      unsupported("deleting all posts");
    } else if (is("admin.user.exports.download.button")) {
      unsupported("the user archive export");
    } else if (is("admin.users.delete_associated_accounts.text")) {
      unsupported("deleting the associated accounts");
    } else if (is("admin.user.discourse_connect.delete_sso_record")) {
      unsupported("deleting the DiscourseConnect record");
    }
  });
})();
