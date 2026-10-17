// The security preferences (controllers/preferences/security.js with
// UserPasskeys, AuthTokenDropdown and UserApiKeys), the dialogs they open
// (ConfirmSession, AuthTokenModal), the calendar tab
// (preferences/calendar-subscriptions.gjs with CalendarSubscriptionUrl),
// the second factor page and chat's tab (preferences/chat.gjs). The
// modals that add and edit second factors are 155d's.
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

  // ConfirmSession in a notice dialog: the password (or a passkey)
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
        // ConfirmSession#confirmWithPasskey
        if (!webauthnSupported()) {
          Discourse.dialog.alert(escape(s.security_key_support_missing_error));
          return;
        }
        passkeyCredential()
          .then(function (publicKeyCredential) {
            return ajax("POST", "/u/confirm-session.json", { publicKeyCredential: publicKeyCredential }).then(
              function (result) {
                if (result && result.success) {
                  Discourse.dialog.close();
                  didConfirm();
                } else {
                  error(s.incorrect_passkey);
                }
              }
            );
          })
          .catch(function (e) {
            if (e.name === "NotAllowedError" || e.name === "AbortError") {
              return;
            }
            popupAjaxError(e);
          });
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

  // lib/webauthn: strings and buffers each way.
  function toBuffer(s) {
    return Uint8Array.from(s, function (c) {
      return c.charCodeAt(0);
    });
  }

  function toBase64(buffer) {
    return btoa(String.fromCharCode.apply(null, new Uint8Array(buffer)));
  }

  function webauthnSupported() {
    return typeof PublicKeyCredential !== "undefined";
  }

  // getPasskeyCredential: the challenge, then the browser's assertion.
  function passkeyCredential() {
    return ajax("GET", "/session/passkey/challenge.json").then(function (resp) {
      return navigator.credentials
        .get({ publicKey: { challenge: toBuffer(resp.challenge), userVerification: "required" } })
        .then(function (credential) {
          return {
            signature: toBase64(credential.response.signature),
            clientData: toBase64(credential.response.clientDataJSON),
            authenticatorData: toBase64(credential.response.authenticatorData),
            credentialId: toBase64(credential.rawId),
            userHandle: toBase64(credential.response.userHandle),
          };
        });
    });
  }

  function button(cls, label, iconName, type) {
    return (
      '<button class="btn ' +
      (iconName ? (label ? "btn-icon-text " : "btn-icon no-text ") : "btn-text ") +
      cls +
      '" type="' +
      (type || "button") +
      '">' +
      (iconName ? icon(iconName) : "") +
      (label ? '<span class="d-button-label">' + escape(label) + "</span>" : "") +
      "</button>"
    );
  }

  // The second factor page's lists, user and strings.
  function factorData() {
    var el = document.getElementById("second-factor-data");
    try {
      return el ? JSON.parse(el.textContent) : null;
    } catch (e) {
      return null;
    }
  }

  // SecondFactorController#handleError: an invalid access goes back to the
  // page, anything else is an alert.
  function handleError(e) {
    var d = factorData();
    if (e.json && e.json.error_type === "invalid_access" && d) {
      window.location.href = userPath() + "/preferences/second-factor";
    } else {
      popupAjaxError(e);
    }
  }

  // markDirty, then loadSecondFactors once the modal closes: the page again.
  var dirty = false;
  function reloadIfDirty() {
    if (dirty) {
      window.location.reload();
    }
  }

  function alertBox(body, text) {
    var box = body.querySelector(".second-factor-alert");
    if (!text) {
      if (box) {
        box.remove();
      }
      return;
    }
    if (!box) {
      box = document.createElement("div");
      box.className = "control-group second-factor-alert";
      box.innerHTML = '<div class="controls"><div class="alert alert-error"></div></div>';
      body.insertBefore(box, body.firstChild);
    }
    box.querySelector(".alert").textContent = text;
  }

  // DOtp: six slots over one numeric input; onChange with its value.
  function otpMarkup(label) {
    var slots = "";
    for (var i = 0; i < 6; i++) {
      slots +=
        '<div class="d-otp-slot' +
        (i === 0 ? " --is-focused --show-cursor" : " --placeholder") +
        '" data-index="' +
        i +
        '"><span aria-hidden="true">&#8203;</span>' +
        (i === 0 ? "" : "-") +
        "</div>";
    }
    return (
      '<div class="d-otp second-factor-token-input"><div class="d-otp-group">' +
      slots +
      '</div><div class="d-otp-input-wrapper"><input aria-label="' +
      escape(label) +
      '" autocapitalize="off" autocomplete="one-time-code" autocorrect="off" class="d-otp-input" data-slot="input-otp" inputmode="numeric" maxlength="6" spellcheck="false"></div></div>'
    );
  }

  function bindOtp(root, onChange) {
    var input = root.querySelector(".d-otp-input");
    var slots = root.querySelectorAll(".d-otp-slot");
    input.addEventListener("input", function () {
      var code = input.value.replace(/\D/g, "").slice(0, 6);
      input.value = code;
      slots.forEach(function (slot, i) {
        var digit = code[i];
        slot.className =
          "d-otp-slot" + (digit ? "" : " --placeholder") + (i === code.length ? " --is-focused --show-cursor" : "");
        slot.innerHTML = '<span aria-hidden="true">&#8203;</span>' + (digit ? escape(digit) : i === code.length ? "" : "-");
      });
      onChange(code);
    });
  }

  // modal/second-factor-add-totp
  function addTotpModal() {
    var d = factorData();
    var S = d.strings;
    var name = "";
    var token = "";
    var loading = false;
    Discourse.modal.show({
      title: S.totp_add,
      onClose: reloadIfDirty,
      body: function (body, close) {
        body.innerHTML =
          '<div class="control-group totp-description"><div class="controls">' +
          S.enable_description +
          '</div></div><div class="control-group totp-qr"><div class="controls"><div class="qr-code"><img></div><p class="key-code"><a class="show-second-factor-key" href>' +
          escape(S.show_key_description) +
          '</a></p></div></div><div class="control-group totp-inputs"><label class="control-label input-prepend">' +
          escape(S.name) +
          '</label><div class="controls totp-app-name"><input id="second-factor-name" maxlength="300" placeholder="' +
          escape(S.totp_default_name) +
          '" type="text"></div><label class="control-label input-prepend">' +
          escape(S.label) +
          '</label><div class="controls totp-app-token" id="second-factor-token">' +
          otpMarkup(S.otp_label.replace("%{count}", "6")) +
          '</div></div><div class="control-group totp-actions"><div class="controls totp-enable">' +
          button("btn-primary add-totp", S.enable) +
          "</div></div>";
        var keyCode = body.querySelector(".key-code");
        var secondFactorKey = "";
        ajax("POST", "/u/create_second_factor_totp.json")
          .then(function (response) {
            if (response.error) {
              alertBox(body, response.error);
              return;
            }
            alertBox(body, null);
            secondFactorKey = response.key;
            body.querySelector(".qr-code img").src = response.qr;
          })
          .catch(function (e) {
            close();
            handleError(e);
          });
        keyCode.querySelector(".show-second-factor-key").addEventListener("click", function (e) {
          e.preventDefault();
          e.stopImmediatePropagation();
          keyCode.innerHTML = '<div class="second-factor-key"></div>';
          keyCode.firstChild.textContent = secondFactorKey;
        });
        body.querySelector("#second-factor-name").addEventListener("input", function (e) {
          name = e.target.value;
        });
        bindOtp(body, function (code) {
          token = code;
        });
        body.querySelector(".add-totp").addEventListener("click", function () {
          if (!token || !name) {
            alertBox(body, S.name_and_code_required_error);
            return;
          }
          if (loading) {
            return;
          }
          loading = true;
          ajax("POST", "/u/enable_second_factor_totp.json", { second_factor_token: token, name: name })
            .then(function (response) {
              if (response.error) {
                alertBox(body, response.error);
                return;
              }
              dirty = true;
              alertBox(body, null);
              close();
            })
            .catch(handleError)
            .finally(function () {
              loading = false;
            });
        });
      },
    });
  }

  // modal/second-factor-add-security-key
  function addSecurityKeyModal() {
    var d = factorData();
    var S = d.strings;
    var ua = navigator.userAgent;
    var iPhone = /iPhone|iPod/.test(ua);
    var android = /Android/.test(ua);
    var defaultName = iPhone
      ? S.security_key_iphone_default_name
      : android
        ? S.security_key_android_default_name
        : S.security_key_default_name;
    Discourse.modal.show({
      title: S.security_key_add,
      onClose: reloadIfDirty,
      body: function (body, close) {
        body.innerHTML =
          '<div class="control-group"><div class="controls">' +
          S.enable_security_key_description +
          '</div></div><div class="control-group"><div class="controls"><input id="security-key-name" maxlength="300" placeholder="security key name" type="text"></div></div><div class="control-group"><div class="controls">' +
          (webauthnSupported() ? button("btn-primary add-security-key", S.security_key_register) : "") +
          "</div></div>";
        var nameInput = body.querySelector("#security-key-name");
        nameInput.value = defaultName;
        var challenge = null;
        ajax("POST", "/u/create_second_factor_security_key.json")
          .then(function (response) {
            if (response.error) {
              alertBox(body, response.error);
              return;
            }
            alertBox(body, webauthnSupported() ? null : S.security_key_support_missing_error);
            challenge = response;
          })
          .catch(function (e) {
            close();
            handleError(e);
          });
        var register = body.querySelector(".add-security-key");
        if (!register) {
          return;
        }
        register.addEventListener("click", function () {
          if (!nameInput.value) {
            alertBox(body, S.security_key_name_required_error);
            return;
          }
          if (!challenge) {
            return;
          }
          navigator.credentials
            .create({
              publicKey: {
                challenge: toBuffer(challenge.challenge),
                rp: { name: challenge.rp_name, id: challenge.rp_id },
                user: {
                  id: toBuffer(challenge.user_secure_id),
                  displayName: d.username_lower,
                  name: d.username_lower,
                },
                pubKeyCredParams: challenge.supported_algorithms.map(function (alg) {
                  return { type: "public-key", alg: alg };
                }),
                excludeCredentials: challenge.existing_active_credential_ids.map(function (id) {
                  return { type: "public-key", id: toBuffer(atob(id)) };
                }),
                timeout: 20000,
                attestation: "none",
                authenticatorSelection: { userVerification: "discouraged" },
              },
            })
            .then(
              function (credential) {
                ajax("POST", "/u/register_second_factor_security_key.json", {
                  id: credential.id,
                  rawId: toBase64(credential.rawId),
                  type: credential.type,
                  attestation: toBase64(credential.response.attestationObject),
                  clientData: toBase64(credential.response.clientDataJSON),
                  name: nameInput.value,
                })
                  .then(function (response) {
                    if (response.error) {
                      alertBox(body, response.error);
                      return;
                    }
                    dirty = true;
                    alertBox(body, null);
                    close();
                  })
                  .catch(handleError);
              },
              function (err) {
                if (err.name === "InvalidStateError") {
                  alertBox(body, S.security_key_already_added_error);
                } else if (err.name === "NotAllowedError") {
                  alertBox(body, S.security_key_not_allowed_error);
                } else {
                  alertBox(body, err.message);
                }
              }
            );
        });
      },
    });
  }

  // modal/second-factor-edit and -edit-security-key: the name, saved.
  function editNameModal(kind, factor) {
    var S = factorData().strings;
    var key = kind === "security_key";
    Discourse.modal.show({
      title: key ? S.security_key_edit : S.edit_title,
      onClose: reloadIfDirty,
      body: function (body) {
        body.innerHTML =
          '<div class="input-group"><label for="' +
          (key ? "security-key-name" : "authenticator-name") +
          '">' +
          escape(key ? S.security_key_edit_description : S.edit_description) +
          '</label><input maxlength="300" ' +
          (key ? 'id="security-key-name" name="security-key-name"' : 'name="authenticator-name"') +
          ' type="text"></div>';
        body.querySelector("input").value = factor.name || "";
      },
      footer: function (footer, close) {
        footer.innerHTML = button("btn-primary", key ? S.security_key_save : S.save, null, key ? "submit" : "button");
        footer.querySelector(".btn").addEventListener("click", function () {
          var name = document.querySelector(".d-modal__body input").value;
          var request = key
            ? ajax("PUT", "/u/security_key.json", { name: name, disable: false, id: factor.id })
            : ajax("PUT", "/u/second_factor.json", {
                second_factor_target: factor.method || 1,
                name: name,
                disable: false,
                id: factor.id,
              });
          request
            .then(function (response) {
              if (!response || !response.error) {
                dirty = true;
              }
              close();
            })
            .catch(function (e) {
              close();
              handleError(e);
            });
        });
      },
    });
  }

  // modal/second-factor-backup-edit with BackupCodes.
  function backupModal() {
    var d = factorData();
    var S = d.strings;
    var enabled = d.backup_enabled;
    var remaining = d.remaining_codes;
    var codes = null;
    var hideTimer = null;
    function remainingHtml() {
      return enabled
        ? (remaining === 1 ? S.remaining_one : S.remaining_other).replace("%{count}", remaining)
        : S.not_enabled;
    }
    function slug(title) {
      var plain = title.normalize ? title.normalize("NFD").replace(/[̀-ͯ]/g, "") : title;
      var ascii = /^[\040-\176]*$/.test(plain) ? plain : "discourse";
      return ascii
        .trim()
        .toLowerCase()
        .replace(/\s|_+/g, "-")
        .replace(/[^\w\-]+/g, "")
        .replace(/\-\-+/g, "-")
        .replace(/^-+/, "")
        .replace(/-+$/, "");
    }
    Discourse.modal.show({
      className: "second-factor-backup-edit-modal",
      title: S.backup_title,
      onClose: reloadIfDirty,
      body: function (body) {
        function message(cls, text) {
          var old = body.querySelector(".backup-message");
          if (old) {
            old.remove();
          }
          if (text) {
            var el = document.createElement("div");
            el.className = "alert backup-message " + cls;
            el.textContent = text;
            body.insertBefore(el, body.firstChild);
          }
          clearTimeout(hideTimer);
          hideTimer = setTimeout(function () {
            var stale = body.querySelector(".backup-message");
            if (stale) {
              stale.remove();
            }
          }, 2000);
        }
        function render() {
          var html = "";
          if (codes) {
            var formatted = codes.join("\r\n").trim();
            var b64 = btoa(
              encodeURIComponent(formatted).replace(/%([0-9A-F]{2})/g, function (m, p1) {
                return String.fromCharCode("0x" + p1);
              })
            );
            html +=
              "<h3>" +
              escape(S.codes_title) +
              "</h3><p>" +
              escape(S.codes_description) +
              '</p><div class="backup-codes"><div class="wrapper"><textarea class="backup-codes-area" id="backupCodes" readonly rows="10">' +
              escape(formatted) +
              '</textarea><div class="controls"><button class="btn no-text btn-icon backup-codes-copy-btn" type="button" aria-label="' +
              escape(S.copy_to_clipboard) +
              '" title="' +
              escape(S.copy_to_clipboard) +
              '">' +
              icon("copy") +
              '</button><a class="btn no-text btn-icon backup-codes-download-btn" aria-label="' +
              escape(S.download_backup_codes) +
              '" title="' +
              escape(S.download_backup_codes) +
              '" download="' +
              escape(slug(d.site_title || "")) +
              '-backup-codes.txt" rel="noopener noreferrer" target="_blank" href="data:application/octet-stream;charset=utf-8;base64,' +
              b64 +
              '">' +
              icon("download") +
              "</a></div></div></div>";
          }
          html += '<div class="backup-remaining">' + remainingHtml() + "</div>";
          var keep = body.querySelector(".backup-message");
          body.innerHTML = html;
          if (keep) {
            body.insertBefore(keep, body.firstChild);
          }
          var area = body.querySelector(".backup-codes-area");
          if (area) {
            area.style.height = area.scrollHeight + "px";
            var selectAll = function () {
              area.focus();
              area.setSelectionRange(0, area.value.length);
            };
            area.addEventListener("click", selectAll);
            body.querySelector(".backup-codes-copy-btn").addEventListener("click", function () {
              selectAll();
              var copied = false;
              try {
                copied = document.execCommand("copy");
              } catch (e) {
                copied = false;
              }
              if (copied) {
                message("alert-success", S.copied_to_clipboard);
              } else {
                message("alert-error", S.copy_to_clipboard_error);
              }
            });
          }
        }
        render();
        body.render = render;
        body.message = message;
      },
      footer: function (footer, close) {
        function draw() {
          footer.innerHTML =
            '<div class="actions">' +
            (enabled
              ? button("btn-primary", S.regenerate, "arrow-rotate-right", "submit")
              : button("btn-primary", S.backup_enable, null, "submit")) +
            "</div>";
          footer.querySelector(".btn").addEventListener("click", generate);
        }
        function generate() {
          var btn = footer.querySelector(".btn");
          btn.disabled = true;
          btn.classList.add("is-loading");
          var body = document.querySelector(".second-factor-backup-edit-modal .d-modal__body");
          ajax("PUT", "/u/second_factors_backup.json")
            .then(function (response) {
              if (response.error) {
                body.message("alert-error", response.error);
                return;
              }
              dirty = true;
              codes = response.backup_codes;
              enabled = true;
              remaining = codes.length;
              body.render();
              draw();
            })
            .catch(function (e) {
              close();
              handleError(e);
            })
            .finally(function () {
              var b = footer.querySelector(".btn");
              b.disabled = false;
              b.classList.remove("is-loading");
            });
        }
        draw();
      },
    });
  }

  // dialog.deleteConfirm with a ban icon and a flat cancel.
  function banConfirm(options) {
    var S = factorData().strings;
    var holder = Discourse.dialog.dialog({
      title: options.title,
      message: options.message,
      bodyHtml: options.bodyHtml,
      buttons: [
        { label: options.confirmLabel, class: "btn-danger", icon: "ban", action: options.didConfirm },
        { label: S.cancel, class: "btn-flat", action: "cancel" },
      ],
    });
    return holder;
  }

  // disableAllSecondFactors with SecondFactorConfirmPhrase.
  function disableAll() {
    var d = factorData();
    var S = d.strings;
    var items = d.totps
      .concat(d.security_keys)
      .map(function (f) {
        return "<li>" + escape(f.name) + "</li>";
      })
      .join("");
    if (d.backup_enabled) {
      items += "<li>" + escape(S.backup_title) + "</li>";
    }
    var holder = banConfirm({
      title: S.disable_confirm,
      bodyHtml:
        escape(S.delete_confirm_header) +
        "<ul>" +
        items +
        "</ul><p>" +
        S.delete_confirm_instruction.replace("%{confirm}", escape(S.disable)) +
        '</p><input autocapitalize="off" autocorrect="off" id="confirm-phrase" class="ember-text-field" type="text">',
      confirmLabel: S.disable,
      didConfirm: function () {
        ajax("PUT", "/u/disable_second_factor.json")
          .then(function () {
            window.location.href = userPath() + "/preferences";
          })
          .catch(handleError);
      },
    });
    var confirm = holder.querySelector(".dialog-footer .btn-danger");
    confirm.disabled = true;
    holder.querySelector("#confirm-phrase").addEventListener("input", function (e) {
      confirm.disabled = e.target.value.toLocaleLowerCase() !== S.disable.toLocaleLowerCase();
    });
  }

  // disableSingleSecondFactor: the last one is all of them.
  function disableSingle(kind, factor) {
    var d = factorData();
    var S = d.strings;
    if (d.totps.length + d.security_keys.length === 1) {
      disableAll();
      return;
    }
    banConfirm({
      title: S.delete_single_confirm_title,
      message: S.delete_single_confirm_message.replace("%{name}", escape(factor.name)),
      confirmLabel: S.delete,
      didConfirm: function () {
        var request =
          kind === "security_key"
            ? ajax("PUT", "/u/security_key.json", { name: factor.name, disable: true, id: factor.id })
            : ajax("PUT", "/u/second_factor.json", {
                second_factor_target: factor.method || 1,
                name: factor.name,
                disable: true,
                id: factor.id,
              });
        request
          .then(function (response) {
            if (!response || !response.error) {
              window.location.reload();
            }
          })
          .catch(handleError);
      },
    });
  }

  // disableSecondFactorBackup
  function disableBackup() {
    var S = factorData().strings;
    banConfirm({
      title: S.delete_backup_codes_confirm_title,
      message: escape(S.delete_backup_codes_confirm_message),
      confirmLabel: S.delete,
      didConfirm: function () {
        ajax("PUT", "/u/second_factor.json", {
          second_factor_target: 2,
          name: "",
          disable: true,
          id: 0,
        })
          .then(function (response) {
            if (response && response.error) {
              Discourse.dialog.alert(escape(response.error));
              return;
            }
            window.location.reload();
          })
          .catch(handleError);
      },
    });
  }

  function findFactor(kind, id) {
    var d = factorData();
    var list = kind === "security_key" ? d.security_keys : d.totps;
    return list.filter(function (f) {
      return String(f.id) === String(id);
    })[0];
  }

  // UserPasskeys#createPasskey, then RenamePasskey for its name.
  function createPasskey() {
    var s = strings();
    ajax("POST", "/u/create_passkey.json")
      .then(function (response) {
        return navigator.credentials
          .create({
            publicKey: {
              challenge: toBuffer(response.challenge),
              rp: { name: response.rp_name, id: response.rp_id },
              user: {
                id: toBuffer(response.user_secure_id),
                name: s.viewer_username,
                displayName: s.viewer_username,
              },
              pubKeyCredParams: response.supported_algorithms.map(function (alg) {
                return { type: "public-key", alg: alg };
              }),
              excludeCredentials: response.existing_passkey_credential_ids.map(function (id) {
                return { type: "public-key", id: toBuffer(atob(id)) };
              }),
              authenticatorSelection: { userVerification: "required", requireResidentKey: true },
            },
          })
          .then(function (credential) {
            return ajax("POST", "/u/register_passkey.json", {
              id: credential.id,
              rawId: toBase64(credential.rawId),
              type: credential.type,
              attestation: toBase64(credential.response.attestationObject),
              clientData: toBase64(credential.response.clientDataJSON),
              name: s.passkey_default_name,
            }).then(function (registration) {
              if (registration.error) {
                Discourse.dialog.alert(escape(registration.error));
                return;
              }
              renamePasskey(registration.id, registration.name, s.passkey_successfully_created);
            });
          });
      })
      .catch(function (error) {
        Discourse.dialog.alert(
          escape(error.name === "InvalidStateError" ? s.passkey_already_added_error : s.passkey_not_allowed_error)
        );
      });
  }

  // RenamePasskey in a notice dialog; the page again once saved or closed.
  function renamePasskey(id, name, title) {
    var s = strings();
    var holder = Discourse.dialog.dialog({
      title: title || s.rename_passkey,
      type: "notice",
      bodyHtml:
        '<div class="rename-passkey__form"><div class="rename-passkey__message"><p>' +
        escape(s.rename_passkey_instructions) +
        '</p></div><form><div class="rename-passkey__form inline-form"><input autofocus type="text">' +
        button("btn-primary", s.passkey_save, null, "submit") +
        "</div></form></div>",
      didCancel: function () {
        if (title) {
          window.location.reload();
        }
      },
    });
    var input = holder.querySelector("input");
    input.value = name || "";
    input.focus();
    holder.querySelector("form").addEventListener("submit", function (e) {
      e.preventDefault();
      ajax("PUT", "/u/rename_passkey/" + id + ".json", { name: input.value })
        .then(function () {
          Discourse.dialog.close();
          window.location.reload();
        })
        .catch(function (err) {
          var form = holder.querySelector(".rename-passkey__form");
          var alert = holder.querySelector(".dialog-body > .alert-error");
          if (!alert) {
            alert = document.createElement("div");
            alert.className = "alert alert-error";
            form.parentNode.insertBefore(alert, form);
          }
          alert.textContent = err.message;
        });
    });
  }

  function confirmDeletePasskey(id) {
    var s = strings();
    Discourse.dialog.deleteConfirm({
      title: s.confirm_delete_passkey,
      didConfirm: function () {
        ajax("DELETE", "/u/delete_passkey/" + id + ".json")
          .then(function (response) {
            if (response && response.success) {
              window.location.reload();
            } else {
              Discourse.dialog.alert(escape((response && response.message) || ""));
            }
          })
          .catch(popupAjaxError);
      },
    });
  }

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
          confirmDeletePasskey(id);
        });
      } else {
        renamePasskey(id, chooser.dataset.passkeyName);
      }
    } else if (chooser.classList.contains("two-factor-backup-dropdown")) {
      if (value === "disable") {
        disableBackup();
      } else {
        backupModal();
      }
    } else if (
      chooser.classList.contains("security-key-dropdown") ||
      chooser.classList.contains("token-based-auth-dropdown")
    ) {
      var kind = chooser.classList.contains("security-key-dropdown") ? "security_key" : "totp";
      var factor = findFactor(kind, chooser.dataset.secondFactorId);
      if (!factor) {
        return;
      }
      if (value === "disable") {
        disableSingle(kind, factor);
      } else {
        editNameModal(kind, factor);
      }
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
        createPasskey();
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
      withTrustedSession(addTotpModal);
    } else if (target.closest(".second-factor .new-security-key")) {
      withTrustedSession(addSecurityKeyModal);
    } else if (target.closest(".second-factor .new-second-factor-backup")) {
      backupModal();
    } else if (target.closest(".second-factor .second-factor-disable-all")) {
      disableAll();
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
