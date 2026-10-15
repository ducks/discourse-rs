// The security preferences (controllers/preferences/security.js with
// UserPasskeys, AuthTokenDropdown and UserApiKeys), the dialogs they open
// (ConfirmSession, AuthTokenModal), the calendar tab
// (preferences/calendar-subscriptions.gjs with CalendarSubscriptionUrl),
// the second factor page and chat's tab (preferences/chat.gjs). Managing
// second factors and passkeys is 155's: their endpoints refuse it.
(function () {
  "use strict";

  if (!Discourse.once("prefs-security")) {
    return;
  }

  function strings() {
    var el = document.getElementById("prefs-security-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (e) {
      return {};
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

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML;
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function csrf() {
    return (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
  }

  function encode(prefix, value, out) {
    if (value === undefined) {
      return;
    }
    if (Array.isArray(value)) {
      value.forEach(function (item) {
        encode(prefix + "[]", item, out);
      });
    } else if (value !== null && typeof value === "object") {
      Object.keys(value).forEach(function (k) {
        encode(prefix + "[" + k + "]", value[k], out);
      });
    } else {
      out.push(encodeURIComponent(prefix) + "=" + encodeURIComponent(value === null ? "" : value));
    }
  }

  function ajax(method, url, data) {
    var body;
    if (data) {
      var out = [];
      Object.keys(data).forEach(function (k) {
        encode(k, data[k], out);
      });
      body = out.join("&").replace(/%20/g, "+");
    }
    return fetch(url.indexOf(base()) === 0 && base() ? url : base() + url, {
      method: method,
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json, text/javascript, */*; q=0.01",
      },
      body: body,
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (e) {
          json = null;
        }
        if (!response.ok) {
          var error = new Error(
            (json && json.errors && json.errors.join("\n")) ||
              (json && json.error) ||
              "Sorry, an error has occurred."
          );
          error.json = json;
          throw error;
        }
        return json;
      });
    });
  }

  function popupAjaxError(e) {
    Discourse.dialog.popupAjaxError(e.message);
  }

  function userPath() {
    var link = document.querySelector(".user-navigation-secondary a[href*='/preferences/']");
    return link ? link.getAttribute("href").replace(/\/preferences\/.*$/, "") : "";
  }

  // ConfirmSession in a notice dialog: the password (or a passkey, 155's)
  // confirms the session, then didConfirm.
  function confirmSession(didConfirm) {
    var s = strings();
    var body =
      '<div class="control-group confirm-session"><div class="confirm-session__instructions">' +
      escape(s.instructions) +
      '</div><div class="confirm-session__instructions"><span>' +
      escape(s.logged_in_as) +
      ' </span><a aria-label="' +
      escape(s.viewer_profile) +
      '" data-user-card="' +
      escape(s.viewer_username) +
      '" href="' +
      escape(s.viewer_path) +
      '" tabindex="0">' +
      escape(s.viewer_username) +
      '</a></div><form><label class="control-label">' +
      escape(s.password_label) +
      '</label><div class="controls"><div class="inline-form"><input id="password" class="ember-text-field ember-view input-large" autofocus="autofocus" type="password"><button class="btn btn-primary" type="submit"><span class="d-button-label">' +
      escape(s.password_confirm) +
      '</span></button></div><div class="confirm-session__reset"><button class="btn btn-link btn-flat confirm-session__reset-btn" type="button"><span class="d-button-label">' +
      escape(s.forgot_password) +
      "</span></button></div>" +
      (s.viewer_has_passkeys
        ? '<div class="confirm-session__passkey"><button class="btn btn-icon-text btn-default" type="button">' +
          icon("user") +
          '<span class="d-button-label">' +
          escape(s.passkey_confirm) +
          "</span></button></div>"
        : "") +
      '</div></form><div class="confirm-session__fine-print">' +
      escape(s.fine_print) +
      "</div></div>";
    var holder = Discourse.dialog.dialog({ title: s.confirm_access_title, bodyHtml: body, type: "notice" });
    var root = holder.querySelector(".confirm-session");
    var input = root.querySelector("#password");
    input.focus();
    function error(text) {
      var alert = holder.querySelector(".dialog-body > .alert-error");
      if (!text) {
        if (alert) {
          alert.remove();
        }
        return;
      }
      if (!alert) {
        alert = document.createElement("div");
        alert.className = "alert alert-error";
        root.parentNode.insertBefore(alert, root);
      }
      alert.textContent = text;
    }
    root.querySelector("form").addEventListener("submit", function (event) {
      event.preventDefault();
      var button = root.querySelector(".inline-form .btn-primary");
      button.disabled = true;
      button.classList.add("is-loading");
      error(input.value ? null : s.incorrect_password);
      ajax("POST", "/u/confirm-session.json", { password: input.value })
        .then(function (result) {
          if (result && result.success) {
            Discourse.dialog.close();
            didConfirm();
          } else {
            error(s.incorrect_password);
          }
        })
        .catch(popupAjaxError)
        .finally(function () {
          button.disabled = false;
          button.classList.remove("is-loading");
        });
    });
    root.querySelector(".confirm-session__reset-btn").addEventListener("click", function () {
      var sent = root.querySelector(".confirm-session__reset-email-sent");
      ajax("POST", "/session/forgot_password.json", { login: s.viewer_username })
        .then(function (result) {
          if (result && result.email_code) {
            Discourse.dialog.close();
            forgotPasswordCode();
          } else if (result && result.success) {
            error(null);
            if (!sent) {
              sent = document.createElement("span");
              sent.className = "confirm-session__reset-email-sent";
              root.querySelector(".confirm-session__reset").appendChild(sent);
            }
            sent.textContent = s.password_reset_email_sent;
          } else {
            error(s.cannot_send_password_reset_email);
          }
        })
        .catch(function (e) {
          error(e.message || s.cannot_send_password_reset_email);
        });
    });
    var passkey = root.querySelector(".confirm-session__passkey .btn");
    if (passkey) {
      passkey.addEventListener("click", function () {
        ajax("POST", "/u/confirm-session.json", { publicKeyCredential: "passkey" }).catch(popupAjaxError);
      });
    }
  }

  // The ForgotPassword modal a reset code opens is the login modals'
  // (196).
  function forgotPasswordCode() {
    Discourse.dialog.alert(escape("Not ported yet: the forgot password modal (196)."));
  }

  // model.trustedSession(), then the action or ConfirmSession first.
  function withTrustedSession(action) {
    ajax("GET", "/u/trusted-session.json")
      .then(function (result) {
        if (result && result.success) {
          action();
        } else {
          confirmSession(action);
        }
      })
      .catch(popupAjaxError);
  }

  function changePassword(button) {
    var s = strings();
    var progress = button.parentNode.querySelector(".password-progress");
    if (progress.textContent) {
      return;
    }
    progress.textContent = s.in_progress;
    ajax("POST", "/session/forgot_password.json", { login: button.dataset.login })
      .then(function (result) {
        if (result && result.email_code) {
          progress.textContent = "";
          forgotPasswordCode();
        } else {
          progress.textContent = s.success;
        }
      })
      .catch(function () {
        progress.textContent = s.error;
      });
  }

  function removePassword(button) {
    var s = strings();
    withTrustedSession(function () {
      Discourse.dialog.deleteConfirm({
        title: s.remove,
        message: escape(s.remove_detail),
        confirmLabel: s.remove,
        didConfirm: function () {
          button.hidden = true;
          ajax("PUT", userPath() + "/remove-password.json")
            .then(function (response) {
              button.hidden = false;
              if (response && response.success) {
                // no_password: the button sets one, nothing to remove.
                var change = document.getElementById("change-password-button");
                change.innerHTML = icon("envelope") + " " + escape(change.dataset.setPassword);
                var group = button.closest(".pref-password");
                button.closest(".controls").remove();
                var instructions = group.querySelector(".controls + .instructions");
                if (instructions) {
                  instructions.remove();
                }
              }
            })
            .catch(function (e) {
              button.hidden = false;
              popupAjaxError(e);
            });
        },
      });
    });
  }

  // AuthTokenModal: the device and the latest post, a link away from
  // securing the account.
  function showToken(token) {
    var s = strings();
    Discourse.modal.show({
      title: s.was_this_you,
      className: "auth-token-modal",
      body: function (body) {
        body.innerHTML =
          "<div><p>" +
          escape(s.was_this_you_description) +
          "</p><p>" +
          s.extended_description +
          "</p></div><div><h3>" +
          escape(s.details) +
          "</h3><ul><li>" +
          icon("far-clock") +
          " " +
          token.seen_at_html +
          "</li><li>" +
          icon("location-dot") +
          " " +
          escape(token.location) +
          "</li><li>" +
          icon(token.icon) +
          " " +
          escape(s.browser_and_device.replace("%{browser}", token.browser).replace("%{device}", token.device)) +
          "</li></ul></div>";
        ajax("GET", s.viewer_path + "/activity.json")
          .then(function (posts) {
            if (!posts || !posts.length) {
              return;
            }
            var post = posts[0];
            var latest = document.createElement("div");
            latest.innerHTML =
              "<h3>" + escape(s.latest_post) + ' <a href="">' + icon("angle-down") + "</a></h3><blockquote></blockquote>";
            var quote = latest.querySelector("blockquote");
            quote.innerHTML = post.excerpt || "";
            var expanded = false;
            var toggle = latest.querySelector("h3 a");
            toggle.addEventListener("click", function (event) {
              event.preventDefault();
              expanded = !expanded;
              toggle.innerHTML = icon(expanded ? "angle-up" : "angle-down");
              quote.innerHTML = (expanded ? post.cooked : post.excerpt) || "";
            });
            body.appendChild(latest);
            toggle.focus();
          })
          .catch(function () {});
      },
      footer: function (footer, close) {
        footer.innerHTML =
          '<button class="btn btn-icon-text btn-primary" type="button">' +
          icon("lock") +
          '<span class="d-button-label">' +
          escape(s.secure_account) +
          '</span></button><button class="btn btn-flat d-modal-cancel" type="button"><span class="d-button-label">' +
          escape(s.cancel) +
          "</span></button>";
        footer.querySelectorAll(".btn").forEach(function (b) {
          b.addEventListener("click", close);
        });
      },
    });
  }

  // revokeAuthToken: one device, or (none given) every device and then
  // logging out.
  function revokeAuthToken(token) {
    ajax("POST", userPath() + "/preferences/revoke-auth-token", token ? { token_id: token.id } : {})
      .then(function () {
        if (!token) {
          window.location.href = base() + "/";
        }
      })
      .catch(popupAjaxError);
  }

  function toggleAuthTokens(link) {
    var group = link.closest(".pref-auth-tokens");
    var showing = link.dataset.showing === "true";
    var list = group.querySelector(".auth-tokens");
    if (showing) {
      list.querySelectorAll(".auth-token").forEach(function (row, i) {
        if (i >= 2) {
          row.remove();
        }
      });
    } else {
      list.appendChild(group.querySelector("template.auth-tokens-rest").content.cloneNode(true));
    }
    link.dataset.showing = showing ? "false" : "true";
    link.innerHTML =
      icon(showing ? "angle-down" : "angle-up") +
      " <span>" +
      escape(showing ? link.dataset.showAll : link.dataset.showFew) +
      "</span>";
  }

  function apiKey(button, revoke) {
    var row = button.closest(".user-api-key");
    ajax("POST", revoke ? "/user-api-key/revoke" : "/user-api-key/undo-revoke", { id: row.dataset.keyId })
      .then(function () {
        row.querySelector(".revoke-api-key").style.display = revoke ? "none" : "";
        row.querySelector(".undo-revoke-api-key").style.display = revoke ? "" : "none";
      })
      .catch(popupAjaxError);
  }

  // The calendar tab.
  function calendarStrings(root) {
    try {
      return JSON.parse(root.dataset.strings || "{}");
    } catch (e) {
      return {};
    }
  }

  function calendarButton(cls, iconName, label) {
    return (
      '<button class="btn ' +
      (iconName ? "btn-icon-text " : "btn-text ") +
      cls +
      '" type="button">' +
      (iconName ? icon(iconName) : "") +
      '<span class="d-button-label">' +
      escape(label) +
      "</span></button>"
    );
  }

  function calendarState(root, state, urls) {
    var s = calendarStrings(root);
    root.querySelectorAll(".calendar-subscriptions__urls, .calendar-subscriptions__active, .calendar-generate").forEach(
      function (el) {
        el.remove();
      }
    );
    var html;
    if (state === "urls") {
      var url = urls.bookmarks;
      var webcal = url.replace(/^https?:\/\//, "webcal://");
      html =
        '<div class="calendar-subscriptions__urls"><div class="alert alert-info calendar-subscriptions__warning">' +
        icon("triangle-exclamation") +
        " " +
        escape(s.urls_warning) +
        '</div><div class="calendar-subscription-url"><div class="calendar-subscription-url__header"><span class="calendar-subscription-url__label">' +
        escape(s.bookmarks) +
        '</span> <span class="calendar-subscription-url__description">' +
        escape(s.bookmarks_description) +
        '</span></div><div class="calendar-subscription-url__actions"><a class="btn btn-default btn-small calendar-copy" href="' +
        escape(url) +
        '">' +
        icon("copy") +
        " " +
        escape(s.copy) +
        '</a><a class="btn btn-flat btn-small" href="' +
        escape("https://calendar.google.com/calendar/r?cid=" + encodeURIComponent(webcal)) +
        '" rel="noopener noreferrer" target="_blank">' +
        icon("fab-google") +
        " " +
        escape(s.add_to_google) +
        '</a><a class="btn btn-flat btn-small" href="' +
        escape(
          "https://outlook.live.com/owa?path=/calendar/action/compose&rru=addsubscription&url=" +
            encodeURIComponent(url) +
            "&name=" +
            encodeURIComponent(s.bookmarks)
        ) +
        '" rel="noopener noreferrer" target="_blank">' +
        icon("fab-microsoft") +
        " " +
        escape(s.add_to_outlook) +
        '</a><a class="btn btn-flat btn-small" href="' +
        escape(webcal) +
        '">' +
        icon("fab-apple") +
        " " +
        escape(s.add_to_apple) +
        '</a></div></div><div class="calendar-subscriptions__actions">' +
        calendarButton("btn-danger calendar-revoke", null, s.revoke) +
        "</div></div>";
    } else if (state === "active") {
      html =
        '<div class="calendar-subscriptions__active"><p class="calendar-subscriptions__active-status">' +
        icon("check") +
        " " +
        escape(s.active_subscription) +
        '</p><div class="calendar-subscriptions__actions">' +
        calendarButton("btn-primary calendar-regenerate", "arrows-rotate", s.regenerate) +
        calendarButton("btn-danger calendar-revoke", null, s.revoke) +
        "</div></div>";
    } else {
      html = calendarButton("btn-primary calendar-generate", "calendar-days", s.generate);
    }
    root.insertAdjacentHTML("beforeend", html);
  }

  function generateUrls(root, button) {
    if (button) {
      button.disabled = true;
      button.classList.add("is-loading");
    }
    ajax("POST", "/calendar-subscriptions.json")
      .then(function (result) {
        calendarState(root, "urls", result.urls);
      })
      .catch(function (e) {
        if (button) {
          button.disabled = false;
          button.classList.remove("is-loading");
        }
        popupAjaxError(e);
      });
  }

  // chat's tab: FormKit's form, saved as User#save(CHAT_ATTRS).
  function chatForm() {
    return document.querySelector(".user-preferences .chat-preferences");
  }

  function chatSubmit(form) {
    var attrs = JSON.parse(form.dataset.chatAttrs || "{}");
    var before = attrs.chat_enabled;
    form.querySelectorAll("input.form-kit__control-checkbox").forEach(function (input) {
      attrs[input.name] = input.checked;
    });
    var radio = form.querySelector("input.form-kit__control-radio:checked");
    if (radio) {
      attrs.chat_quick_reaction_type = radio.value;
    }
    // The select shows its first option for a value none has (default);
    // only a choice changes the field.
    var select = form.querySelector("select[name=chat_separate_sidebar_mode]");
    if (select && select.dataset.dirty === "true") {
      attrs.chat_separate_sidebar_mode = select.value;
    }
    var emojis = Array.prototype.map.call(form.querySelectorAll(".chat-custom-reaction"), function (b) {
      return b.dataset.emoji;
    });
    if (emojis.length) {
      attrs.chat_quick_reactions_custom = emojis.join("|");
    }
    var saved = form.querySelector(".save-controls .saved");
    if (saved) {
      saved.remove();
    }
    ajax("PUT", form.dataset.saveUrl, attrs)
      .then(function () {
        form.dataset.chatAttrs = JSON.stringify(attrs);
        var span = document.createElement("span");
        span.className = "saved";
        span.textContent = form.dataset.savedLabel;
        form.querySelector(".save-controls").appendChild(span);
        if (attrs.chat_enabled !== before) {
          window.location.reload();
        }
      })
      .catch(popupAjaxError);
  }

  function syncChatCustom(form) {
    var radio = form.querySelector("input.form-kit__control-radio:checked");
    var custom = form.querySelector("#control-chat_quick_reactions_custom");
    if (custom) {
      custom.style.display = radio && radio.value === "custom" ? "" : "none";
    }
  }

  document.addEventListener("change", function (event) {
    var form = chatForm();
    if (form && form.contains(event.target) && event.target.matches(".form-kit__control-radio")) {
      syncChatCustom(form);
    }
    if (form && form.contains(event.target) && event.target.matches("select")) {
      event.target.dataset.dirty = "true";
    }
  });

  document.addEventListener("prefs-action", function (event) {
    var chooser = event.target;
    var value = event.detail.value;
    if (chooser.classList.contains("auth-token-dropdown")) {
      var token = JSON.parse(chooser.closest(".auth-token").dataset.token);
      if (value === "notYou") {
        showToken(token);
      } else if (value === "logOut") {
        revokeAuthToken(token);
      }
    } else if (chooser.classList.contains("passkey-options-dropdown")) {
      var id = chooser.dataset.passkeyId;
      if (value === "delete") {
        withTrustedSession(function () {
          ajax("DELETE", "/u/delete_passkey/" + id + ".json").catch(popupAjaxError);
        });
      } else {
        ajax("PUT", "/u/rename_passkey/" + id + ".json").catch(popupAjaxError);
      }
    } else if (chooser.classList.contains("two-factor-backup-dropdown")) {
      // Regenerating makes new codes; disabling updates the backup factor.
      ajax("PUT", value === "disable" ? "/u/second_factor.json" : "/u/second_factors_backup.json").catch(
        popupAjaxError
      );
    } else if (chooser.classList.contains("security-key-dropdown")) {
      ajax("PUT", "/u/security_key.json", { id: chooser.dataset.secondFactorId }).catch(popupAjaxError);
    } else if (chooser.classList.contains("token-based-auth-dropdown")) {
      ajax("PUT", "/u/second_factor.json", { second_factor_target: chooser.dataset.secondFactorId }).catch(
        popupAjaxError
      );
    }
  });

  document.addEventListener("click", function (event) {
    var target = event.target;
    var el;
    if ((el = target.closest("#change-password-button"))) {
      event.preventDefault();
      changePassword(el);
    } else if ((el = target.closest("#remove-password-link"))) {
      event.preventDefault();
      removePassword(el);
    } else if (target.closest(".pref-second-factor .btn-second-factor")) {
      withTrustedSession(function () {
        window.location.href = userPath() + "/preferences/second-factor";
      });
    } else if (target.closest(".pref-passkeys__add .btn")) {
      withTrustedSession(function () {
        ajax("POST", "/u/create_passkey.json").catch(popupAjaxError);
      });
    } else if ((el = target.closest(".pref-auth-tokens .auth-tokens-toggle"))) {
      event.preventDefault();
      toggleAuthTokens(el);
    } else if (target.closest(".pref-auth-tokens .log-out-all")) {
      event.preventDefault();
      revokeAuthToken(null);
    } else if ((el = target.closest(".user-api-key .revoke-api-key"))) {
      apiKey(el, true);
    } else if ((el = target.closest(".user-api-key .undo-revoke-api-key"))) {
      apiKey(el, false);
    } else if ((el = target.closest(".user-api-key__scopes-toggle a"))) {
      event.preventDefault();
      var list = el.closest(".user-api-key").querySelector(".user-api-key__scopes-list");
      list.style.display = list.style.display === "none" ? "" : "none";
    } else if ((el = target.closest(".calendar-subscriptions .calendar-generate"))) {
      generateUrls(el.closest(".calendar-subscriptions"), el);
    } else if ((el = target.closest(".calendar-subscriptions .calendar-regenerate"))) {
      var regenRoot = el.closest(".calendar-subscriptions");
      Discourse.dialog.confirm({
        message: escape(calendarStrings(regenRoot).regenerate_confirm),
        didConfirm: function () {
          generateUrls(regenRoot, el);
        },
      });
    } else if ((el = target.closest(".calendar-subscriptions .calendar-revoke"))) {
      var revokeRoot = el.closest(".calendar-subscriptions");
      Discourse.dialog.confirm({
        message: escape(calendarStrings(revokeRoot).revoke_confirm),
        didConfirm: function () {
          ajax("DELETE", "/calendar-subscriptions.json")
            .then(function () {
              calendarState(revokeRoot, "none");
            })
            .catch(popupAjaxError);
        },
      });
    } else if ((el = target.closest(".calendar-subscriptions .calendar-copy"))) {
      event.preventDefault();
      var copyStrings = calendarStrings(el.closest(".calendar-subscriptions"));
      navigator.clipboard.writeText(el.getAttribute("href")).then(function () {
        el.innerHTML = icon("check") + " " + escape(copyStrings.copied);
        setTimeout(function () {
          el.innerHTML = icon("copy") + " " + escape(copyStrings.copy);
        }, 2000);
      });
    } else if (target.closest(".second-factor .new-totp")) {
      ajax("POST", "/u/create_second_factor_totp.json").catch(popupAjaxError);
    } else if (target.closest(".second-factor .new-security-key")) {
      ajax("POST", "/u/create_second_factor_security_key.json").catch(popupAjaxError);
    } else if (target.closest(".second-factor .new-second-factor-backup")) {
      ajax("PUT", "/u/second_factors_backup.json").catch(popupAjaxError);
    } else if (target.closest(".second-factor .second-factor-disable-all")) {
      ajax("PUT", "/u/disable_second_factor.json").catch(popupAjaxError);
    } else if ((el = target.closest(".chat-preferences .chat-custom-reaction"))) {
      var button = el;
      Discourse.emojiPicker.open(button, {
        context: "chat_preferences",
        onSelect: function (emoji) {
          button.dataset.emoji = emoji;
          var img = button.querySelector("img");
          img.src = Discourse.emojiPicker.url(emoji);
          img.title = emoji;
          img.alt = emoji;
        },
      });
    } else if ((el = target.closest(".chat-preferences .save-controls .btn-primary"))) {
      event.preventDefault();
      chatSubmit(el.closest(".chat-preferences"));
    }
  });
})();
