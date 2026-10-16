// The admin API keys pages in the browser: ApiKeyItem's menu (revoke,
// undo revoke), ApiKeysShow (editing the description, revoke, undo
// revoke, delete, the scopes' Allowed URLs modal) and ApiKeysNew (the
// single user's EmailGroupUserChooser, the granular scopes' table, the
// required description and scope checks, then the generated key with
// its copy button). A change the page shows otherwise loads it again.
(function () {
  "use strict";

  if (!Discourse.once("admin-api-keys")) {
    return;
  }

  var ui = Discourse.adminUi;
  var t = ui.t;

  function page() {
    return document.querySelector(".admin-contents.admin-api-keys");
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function reload() {
    location.reload();
  }

  function keyPath(id) {
    return "/admin/api/keys/" + encodeURIComponent(id);
  }

  // ApiKeyItem's DMenu.
  ui.menus(function (trigger, content, menu) {
    var row = trigger.closest("[data-api-key-id]");
    if (!row) {
      return;
    }
    content.addEventListener("click", function (event) {
      var button = event.target.closest("button");
      if (!button) {
        return;
      }
      var action = button.classList.contains("api-key-item__revoke") ? "/revoke" : "/undo-revoke";
      ui.ajax("POST", keyPath(row.dataset.apiKeyId) + action)
        .then(function () {
          if (menu && menu.close) {
            menu.close();
          }
          reload();
        })
        .catch(ui.failed);
    });
  });

  // ApiKeyUrlsModal
  function showUrls(urls) {
    Discourse.modal.show({
      title: t("admin.api.scopes.allowed_urls"),
      body: function (body) {
        body.innerHTML =
          "<div><ul>" +
          urls
            .map(function (url) {
              return "<li><code>" + escapeHtml(url) + "</code></li>";
            })
            .join("") +
          "</ul></div>";
      },
    });
  }

  function button(cls, label) {
    return (
      '<button class="btn ' + cls + '" type="button"><span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  // ApiKeysShow's description: the input with Save and Cancel.
  function editDescription(show) {
    var row = show.querySelector(".api-key-show__edit-description").closest(".input-area");
    var span = row.querySelector(":scope > span");
    var controls = row.querySelector(".controls");
    var original = { span: span.outerHTML, controls: controls.innerHTML };
    var current = span.dataset.description !== undefined ? span.dataset.description : span.textContent;
    var input = document.createElement("input");
    input.maxLength = 255;
    input.name = "description";
    input.placeholder = t("admin.api.description_placeholder");
    input.type = "text";
    input.value = show.dataset.description !== undefined ? show.dataset.description : current;
    span.replaceWith(input);
    controls.innerHTML =
      button("btn-default api-key-show__save-description", t("admin.api_keys.save")) +
      button("btn-default api-key-show__cancel-description", t("admin.api_keys.cancel"));
    controls.querySelector(".api-key-show__save-description").addEventListener("click", function () {
      ui.ajax("PUT", keyPath(show.dataset.apiKeyId), { json: { key: { description: input.value } } })
        .then(reload)
        .catch(ui.failed);
    });
    controls.querySelector(".api-key-show__cancel-description").addEventListener("click", function () {
      input.outerHTML = original.span;
      controls.innerHTML = original.controls;
    });
    input.focus();
  }

  // ApiKeysNew: the form's state as FormKit keeps it.
  function form() {
    return document.querySelector(".admin-contents.admin-api-keys.new form.form-kit");
  }

  function fieldAfter(form, name, template) {
    var existing = form.querySelector('.form-kit__field[data-name="' + name + '"]');
    return { existing: existing, template: form.querySelector(template) };
  }

  function syncUserField(form) {
    var mode = form.querySelector('select[name="user_mode"]').value;
    var f = fieldAfter(form, "user", "template.api-keys-new__user-field");
    if (mode === "single" && !f.existing) {
      var node = f.template.content.firstElementChild.cloneNode(true);
      form.querySelector('.form-kit__field[data-name="user_mode"]').after(node);
    } else if (mode !== "single" && f.existing) {
      f.existing.remove();
    }
  }

  function syncScopes(form) {
    var mode = form.querySelector('select[name="scope_mode"]').value;
    var existing = form.querySelector(".scopes-title");
    var template = form.querySelector("template.api-keys-new__scopes");
    if (mode === "granular" && !existing) {
      var nodes = template.content.cloneNode(true);
      // form.Object and each resource's Collection around the rows.
      var tbody = nodes.querySelector("tbody");
      var object = document.createElement("div");
      object.className = "form-kit__object scopes-table__object";
      var collection = null;
      Array.prototype.slice.call(tbody.children).forEach(function (row) {
        if (row.classList.contains("scope-resource-name")) {
          object.appendChild(row);
          collection = document.createElement("div");
          collection.className = "form-kit__collection";
          object.appendChild(collection);
        } else if (collection) {
          collection.appendChild(row);
        }
      });
      tbody.appendChild(object);
      form.querySelector(".form-kit__actions").before(nodes);
    } else if (mode !== "granular" && existing) {
      var p = existing.nextElementSibling;
      var table = p && p.nextElementSibling;
      existing.remove();
      if (p) {
        p.remove();
      }
      if (table) {
        table.remove();
      }
    }
  }

  function selectedScopes(form) {
    var scopes = [];
    form.querySelectorAll("tr[data-scope-id]").forEach(function (row) {
      var box = row.querySelector(".form-kit__control-checkbox");
      if (!box || !box.checked) {
        return;
      }
      var parts = row.dataset.scopeId.split(":");
      var scope = { scope_id: row.dataset.scopeId, key: parts[1], name: parts[1], params: [] };
      row.querySelectorAll("td:last-child .form-kit__control-input").forEach(function (input) {
        var name = input.name.split(".").pop();
        scope.params.push(name);
        if (input.value) {
          scope[name] = input.value;
        }
      });
      scopes.push(scope);
    });
    return scopes;
  }

  // FKErrors under a field.
  function setError(field, message) {
    var content = field.querySelector(".form-kit__container-content");
    var meta = content.querySelector(".form-kit__meta");
    if (meta) {
      meta.remove();
    }
    field.classList.toggle("has-error", !!message);
    if (message) {
      content.insertAdjacentHTML(
        "beforeend",
        '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors"><span>' +
          ui.icon("triangle-exclamation") +
          " " +
          escapeHtml(message) +
          "</span></p></div>"
      );
    }
  }

  function submit(form) {
    var description = form.querySelector('input[name="description"]');
    var userMode = form.querySelector('select[name="user_mode"]').value;
    var scopeMode = form.querySelector('select[name="scope_mode"]').value;
    var valid = true;
    var required = t("form_kit.errors.required");
    setError(description.closest(".form-kit__field"), description.value.trim() ? null : required);
    valid = valid && !!description.value.trim();
    var payload = { description: description.value, scope_mode: scopeMode };
    if (userMode === "single") {
      var chooser = form.querySelector(".email-group-user-chooser");
      var username = chooser && chooser.dataset.value;
      setError(chooser.closest(".form-kit__field"), username ? null : required);
      valid = valid && !!username;
      payload.username = username;
    }
    var scopes = [];
    if (scopeMode === "granular") {
      scopes = selectedScopes(form);
      if (!scopes.length) {
        Discourse.dialog.alert({
          title: t("admin.api.scopes.title"),
          message: t("admin.api.scopes.one_or_more"),
        });
        valid = false;
      }
      payload.scopes = scopes;
    } else if (scopeMode === "read_only") {
      payload.scopes = [{ scope_id: "global:read", key: "read", name: "read", params: [] }];
    }
    if (!valid) {
      return;
    }
    ui.ajax("POST", "/admin/api/keys", { json: { key: payload } })
      .then(function (result) {
        showGenerated(form, result.key.key, {
          description: description.value,
          scopeMode: scopeMode,
          scopes: scopeMode === "granular" ? scopes : null,
        });
      })
      .catch(ui.failed);
  }

  // The generated key in place of the form.
  function showGenerated(form, key, data) {
    var rows = "";
    if (data.description) {
      rows +=
        '<div class="generated-api-key__detail-row"><span class="generated-api-key__label">' +
        escapeHtml(t("admin.api.description")) +
        "</span><span>" +
        escapeHtml(data.description) +
        "</span></div>";
    }
    rows +=
      '<div class="generated-api-key__detail-row"><span class="generated-api-key__label">' +
      escapeHtml(t("admin.api.scope_mode")) +
      "</span><span>" +
      escapeHtml(t("admin.api.scopes." + data.scopeMode)) +
      "</span></div>";
    if (data.scopes) {
      rows +=
        '<div class="generated-api-key__detail-row"><span class="generated-api-key__label">' +
        escapeHtml(t("admin.api.scopes.title")) +
        '</span><span class="generated-api-key__scope-badges">' +
        data.scopes
          .map(function (scope) {
            var params = scope.params
              .filter(function (p) {
                return scope[p];
              })
              .map(function (p) {
                return (
                  '<span class="generated-api-key__scope-param">(' +
                  escapeHtml(p) +
                  ": " +
                  escapeHtml(scope[p]) +
                  ")</span>"
                );
              })
              .join("");
            return '<span class="generated-api-key__scope-badge">' + escapeHtml(scope.scope_id) + params + "</span>";
          })
          .join("") +
        "</span></div>";
    }
    var card = form.closest(".admin-config-area-card");
    card.innerHTML =
      '<div class="generated-api-key-container"><div class="alert alert-warning">' +
      ui.icon("triangle-exclamation") +
      "<span>" +
      escapeHtml(t("admin.api.not_shown_again")) +
      '</span></div><div class="generated-api-key__details">' +
      rows +
      '</div><div class="generated-api-key__key-row"><code class="generated-api-key">' +
      escapeHtml(key) +
      '</code><button class="btn btn-icon-text btn-default generated-api-key__copy-btn" type="button">' +
      ui.icon("copy") +
      '<span class="d-button-label">' +
      escapeHtml(t("admin.api_keys.copy_key")) +
      '</span></button></div><button class="btn btn-default continue" type="button" data-href="/admin/api/keys"><span class="d-button-label">' +
      escapeHtml(t("admin.api_keys.continue")) +
      "</span></button></div>";
    card.querySelector(".generated-api-key__copy-btn").addEventListener("click", function () {
      navigator.clipboard.writeText(key).then(function () {
        if (ui.toast) {
          ui.toast(t("admin.api_keys.key_copied_to_clipboard"));
        }
      });
    });
  }

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root) {
      return;
    }
    var urls = event.target.closest("button[data-urls]");
    if (urls) {
      try {
        showUrls(JSON.parse(urls.dataset.urls));
      } catch (_) {
        showUrls([]);
      }
      return;
    }
    var show = event.target.closest(".api-key-show");
    if (show) {
      var id = show.dataset.apiKeyId;
      if (event.target.closest(".api-key-show__edit-description")) {
        editDescription(show);
      } else if (event.target.closest(".api-key-show__revoke")) {
        ui.ajax("POST", keyPath(id) + "/revoke").then(reload).catch(ui.failed);
      } else if (event.target.closest(".api-key-show__undo-revoke")) {
        ui.ajax("POST", keyPath(id) + "/undo-revoke").then(reload).catch(ui.failed);
      } else if (event.target.closest(".api-key-show__delete")) {
        ui.ajax("DELETE", keyPath(id))
          .then(function () {
            window.location = ui.base() + "/admin/api/keys";
          })
          .catch(ui.failed);
      }
      return;
    }
  });

  document.addEventListener("change", function (event) {
    var f = form();
    if (!f || !f.contains(event.target)) {
      return;
    }
    if (event.target.name === "user_mode") {
      syncUserField(f);
    } else if (event.target.name === "scope_mode") {
      syncScopes(f);
    }
  });

  // Before the boost's own listener on the form: the form is FormKit's,
  // never sent as it stands.
  document.addEventListener(
    "submit",
    function (event) {
      var f = event.target.closest(".admin-contents.admin-api-keys.new form.form-kit");
      if (f) {
        event.preventDefault();
        event.stopImmediatePropagation();
        submit(f);
      }
    },
    true
  );

  Discourse.onPage(function () {
    if (page()) {
      ui.strings("admin-api-keys-strings");
    }
  });
})();
