// Chat's admin plugin pages (plugins/chat/admin_view.rs): the header's
// Create export (ChatAdminPluginActions: the confirmation, POST
// /chat/admin/export/messages, then the alert), the webhooks list's
// delete (its deleteConfirm, DELETE /admin/plugins/chat/hooks/:id, the
// row gone) and ChatIncomingWebhookEditForm: its required fields, the
// emoji picker and its reset, and the save (PUT for a webhook, POST for a
// new one, which then goes to its edit page) with the toasts.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.once && !Discourse.once("chat-admin")) {
    return;
  }

  var HOOKS = "/admin/plugins/chat/hooks";

  function ui() {
    return Discourse.adminUi;
  }

  function t(key) {
    if (!ui()) {
      return key;
    }
    ui().strings("chat-admin-strings");
    return ui().t(key);
  }

  function go(url) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    (document.getElementById("main-outlet") || document.body).appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  function exportMessages() {
    Discourse.dialog.confirm({
      message: t("chat.admin.export_messages.confirm_export"),
      didConfirm: function () {
        ui()
          .ajax("POST", "/chat/admin/export/messages")
          .then(function () {
            Discourse.dialog.alert(t("chat.admin.export_messages.export_has_started"));
          })
          .catch(ui().failed);
      },
    });
  }

  function destroyWebhook(row) {
    Discourse.dialog.deleteConfirm({
      message: t("chat.incoming_webhooks.confirm_destroy"),
      didConfirm: function () {
        ui()
          .ajax("DELETE", HOOKS + "/" + row.dataset.webhookId)
          .then(function () {
            row.remove();
          })
          .catch(ui().failed);
      },
    });
  }

  function setError(form, name, message) {
    var field = form.querySelector(".form-kit__field[data-name='" + name + "']");
    if (!field) {
      return;
    }
    var errors = field.querySelector(".form-kit__errors");
    field.classList.toggle("has-error", !!message);
    if (!message) {
      if (errors) {
        errors.remove();
      }
      return;
    }
    if (!errors) {
      errors = document.createElement("p");
      errors.className = "form-kit__errors";
      field.querySelector(".form-kit__container-content").appendChild(errors);
    }
    errors.textContent = message;
  }

  function save(wrapper) {
    var form = wrapper.querySelector("form");
    var value = function (name) {
      var el = form.querySelector("[name='" + name + "']");
      return el ? el.value : "";
    };
    var data = {
      name: value("name"),
      description: value("description"),
      username: value("username"),
      chat_channel_id: value("chat_channel_id"),
      emoji: value("emoji"),
    };
    var required = t("form_kit.errors.required");
    setError(form, "name", data.name ? null : required);
    setError(form, "chat_channel_id", data.chat_channel_id ? null : required);
    if (!data.name || !data.chat_channel_id) {
      return;
    }
    var id = wrapper.dataset.webhookId;
    if (id) {
      ui()
        .ajax("PUT", HOOKS + "/" + id, data)
        .then(function () {
          ui().toast(t("chat.incoming_webhooks.saved"));
        })
        .catch(ui().failed);
    } else {
      ui()
        .ajax("POST", HOOKS, data)
        .then(function (webhook) {
          ui().toast(t("chat.incoming_webhooks.created"));
          go(ui().base() + HOOKS + "/" + webhook.id + "/edit");
        })
        .catch(ui().failed);
    }
  }

  function setEmoji(wrapper, emoji) {
    var input = wrapper.querySelector("input[name='emoji']");
    input.value = emoji || "";
    wrapper.querySelector(".admin-chat-webhooks-clear-emoji").disabled = !emoji;
  }

  document.addEventListener("click", function (event) {
    var target = event.target;
    if (target.closest(".admin-chat-export")) {
      event.preventDefault();
      exportMessages();
      return;
    }
    // EditCategoryChat's subcategory channels toggle.
    var toggle = target.closest(".edit-category-chat__subcategory-toggle .d-toggle-switch__checkbox");
    if (toggle) {
      var on = toggle.getAttribute("aria-checked") !== "true";
      toggle.setAttribute("aria-checked", on ? "true" : "false");
      var slider = toggle.parentNode.querySelector(".d-toggle-switch__checkbox-slider");
      slider.innerHTML = on
        ? '<svg class="fa d-icon d-icon-check svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#check"></use></svg>'
        : "";
      document.querySelectorAll(".edit-category-chat__table tr[data-subcategory]").forEach(function (row) {
        row.hidden = !on;
      });
      return;
    }
    var del = target.closest(".admin-chat-incoming-webhooks-delete");
    if (del) {
      event.preventDefault();
      destroyWebhook(del.closest(".incoming-chat-webhooks-row"));
      return;
    }
    var wrapper = target.closest(".chat-incoming-webhook-form");
    if (!wrapper) {
      return;
    }
    if (target.closest(".admin-chat-webhooks-clear-emoji")) {
      setEmoji(wrapper, null);
    } else if (target.closest(".emoji-picker-trigger") && Discourse.emojiPicker) {
      Discourse.emojiPicker.open(target.closest(".emoji-picker-trigger"), {
        onSelect: function (emoji) {
          setEmoji(wrapper, ":" + emoji + ":");
        },
      });
    } else if (target.closest("button[type='submit']")) {
      event.preventDefault();
      save(wrapper);
    }
  });

  document.addEventListener("submit", function (event) {
    var wrapper = event.target.closest && event.target.closest(".chat-incoming-webhook-form");
    if (wrapper) {
      event.preventDefault();
      save(wrapper);
    }
  });
})();
