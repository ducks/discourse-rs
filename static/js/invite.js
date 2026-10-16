// The invite page (controllers/invites/show.js with CodeLoginForm): a
// signed-in member accepts with one button; anyone else either asks for a
// login code (the email step the page renders, then the code and username
// steps drawn here) or fills the signup form, which PUTs to
// /invites/show/:key.
(function () {
  "use strict";

  if (!Discourse.once("invite")) {
    return;
  }

  var RESEND_COOLDOWN_SECONDS = 30;

  function strings() {
    var el = document.getElementById("invite-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (e) {
      return {};
    }
  }

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML;
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

  function button(classes, label, type) {
    return (
      '<button class="btn ' +
      classes +
      '" type="' +
      (type || "button") +
      '"><span class="d-button-label">' +
      escape(label) +
      "</span></button>"
    );
  }

  function timezone() {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    } catch (e) {
      return "";
    }
  }

  function init() {
    var root = document.querySelector(".invites-show");
    if (!root || root.dataset.ready) {
      return;
    }
    root.dataset.ready = "true";
    var S = strings();
    var key = root.dataset.inviteKey;
    var base = root.dataset.basePath || "";
    var csrf = root.dataset.csrf || "";
    var isLink = root.dataset.inviteLink === "true";

    function request(method, url, data) {
      var body = Object.keys(data || {})
        .filter(function (k) {
          return data[k] !== undefined && data[k] !== null;
        })
        .map(function (k) {
          return encodeURIComponent(k) + "=" + encodeURIComponent(data[k]);
        })
        .join("&");
      return fetch(base + url, {
        method: method,
        credentials: "same-origin",
        headers: {
          Accept: "application/json",
          "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
          "X-CSRF-Token": csrf,
          "X-Requested-With": "XMLHttpRequest",
        },
        body: method === "GET" ? undefined : body,
      }).then(function (response) {
        return response
          .json()
          .catch(function () {
            return {};
          })
          .then(function (json) {
            return { ok: response.ok, status: response.status, body: json };
          });
      });
    }

    function errorOf(result) {
      var b = result.body || {};
      if (b.message) {
        return b.message;
      }
      if (b.errors && b.errors.length) {
        return b.errors[0];
      }
      if (b.error) {
        return b.error;
      }
      return S.generic_error || "";
    }

    function showSuccess(message) {
      var col = root.querySelector(".invite-form");
      col.className = "invite-success";
      col.innerHTML =
        '<div class="col-form"><div class="success-info"><p>' + message + "</p></div></div>";
    }

    // The controller's submit: PUT /invites/show/:key.json.
    function accept(data, alertEl) {
      data.timezone = timezone();
      return request("PUT", "/invites/show/" + encodeURIComponent(key) + ".json", data).then(
        function (result) {
          var b = result.body || {};
          if (result.ok && b.success) {
            showSuccess(b.message || S.success);
            if (b.redirect_to) {
              window.location.href = b.redirect_to;
            }
          } else if (alertEl) {
            alertEl.textContent = errorOf(result);
            alertEl.hidden = false;
          }
        }
      );
    }

    var acceptButton = root.querySelector(".accept-invitation");
    if (acceptButton) {
      acceptButton.addEventListener("click", function () {
        acceptButton.disabled = true;
        accept({}, root.querySelector(".alert-error")).finally(function () {
          acceptButton.disabled = false;
        });
      });
    }

    var signup = root.querySelector("form.invite-signup");
    if (signup) {
      var passwordInput = signup.querySelector("#new-account-password");
      var mask = signup.querySelector(".toggle-password-mask");
      if (mask && passwordInput) {
        mask.addEventListener("click", function () {
          passwordInput.type = passwordInput.type === "password" ? "text" : "password";
        });
      }
      signup.addEventListener("submit", function (event) {
        event.preventDefault();
        var value = function (selector) {
          var el = signup.querySelector(selector);
          return el ? el.value : undefined;
        };
        var data = {
          username: value("#new-account-username"),
          name: value("#new-account-name"),
          password: value("#new-account-password"),
        };
        if (isLink) {
          data.email = value("#new-account-email");
        } else {
          data.email_token = new URLSearchParams(window.location.search).get("t") || undefined;
        }
        accept(data, signup.querySelector(".alert-error"));
      });
    }

    var form = root.querySelector(".code-login-form");
    if (!form) {
      return;
    }
    var email = form.dataset.email || "";
    var code = "";
    var cooldownTimer = null;

    // showInviteIntroduction is false past the email step; the progress
    // bar moves on.
    function leaveIntroduction(step) {
      ["invited-by", "email-message", "disclaimer", "login-welcome-header"].forEach(function (cls) {
        var el = root.querySelector("." + cls);
        if (el) {
          el.remove();
        }
      });
      var segments = root.querySelectorAll(".signup-progress-bar__segment");
      var steps = Array.prototype.map.call(segments, function (_, i) {
        return i;
      });
      var current = step === "login" ? steps.length - 1 : 1;
      segments.forEach(function (segment, i) {
        segment.className =
          "signup-progress-bar__segment --" +
          (i === current ? "active" : i < current ? "completed" : "incomplete");
      });
    }

    function setError(text) {
      var el = form.querySelector(".code-login-form__error");
      if (el) {
        el.textContent = text || "";
        el.hidden = !text;
      }
    }

    function hp() {
      return request("GET", "/session/hp.json").then(function (r) {
        return r.body || {};
      });
    }

    function requestCode() {
      return hp().then(function (pot) {
        return request("POST", "/session/login-code.json", {
          invite_key: key,
          email: email,
          password_confirmation: pot.value,
          challenge: (pot.challenge || "").split("").reverse().join(""),
        });
      });
    }

    function startCooldown() {
      var left = RESEND_COOLDOWN_SECONDS;
      var resend = form.querySelector(".code-login-form__resend");
      if (!resend) {
        return;
      }
      clearInterval(cooldownTimer);
      var label = resend.querySelector(".d-button-label");
      var tick = function () {
        if (left > 0) {
          resend.disabled = true;
          label.textContent = (S.resend_countdown || "").replace("%{count}", left);
          left -= 1;
        } else {
          resend.disabled = false;
          label.textContent = S.resend;
          clearInterval(cooldownTimer);
        }
      };
      tick();
      cooldownTimer = setInterval(tick, 1000);
    }

    function otp() {
      var slots = "";
      for (var i = 0; i < 6; i++) {
        slots +=
          '<div class="d-otp-slot' +
          (i === 0 ? " --is-focused --show-cursor" : " --placeholder") +
          '" data-index="' +
          i +
          '"><span aria-hidden="true">​</span>' +
          (i === 0 ? "" : "-") +
          "</div>";
      }
      return (
        '<div class="d-otp"><div class="d-otp-group">' +
        slots +
        '</div><div class="d-otp-input-wrapper"><input aria-label="' +
        escape(S.otp_label) +
        '" autocapitalize="off" autocomplete="one-time-code" autocorrect="off" class="d-otp-input" data-slot="input-otp" inputmode="numeric" maxlength="6" spellcheck="false" id="code-login-otp-0" autofocus=""></div></div>'
      );
    }

    function codeStep() {
      leaveIntroduction("activate");
      form.innerHTML =
        '<input aria-hidden="true" autocomplete="username" class="code-login-form__hidden-email sr-only" name="email" readonly="" tabindex="-1" type="email" value="' +
        escape(email) +
        '"><div class="code-login-form__code-step"><h2 class="code-login-form__title">' +
        escape(S.check_your_email) +
        '</h2><p class="code-login-form__instructions">' +
        (S.code_instructions || "").replace("%{email}", escape(email)) +
        "</p>" +
        otp() +
        '<div aria-live="polite" class="code-login-form__error" role="alert"></div>' +
        button("btn-primary code-login-form__verify", S.verify, "submit") +
        '<div class="code-login-form__actions">' +
        button("btn-flat code-login-form__resend", S.resend) +
        (isLink ? button("btn-flat code-login-form__change-email", S.use_different_email) : "") +
        "</div></div>";
      var input = form.querySelector(".d-otp-input");
      var slots = form.querySelectorAll(".d-otp-slot");
      input.focus();
      input.addEventListener("input", function () {
        code = input.value.replace(/\D/g, "").slice(0, 6);
        input.value = code;
        slots.forEach(function (slot, i) {
          var digit = code[i];
          slot.className = "d-otp-slot" + (digit ? "" : " --placeholder") + (i === code.length ? " --is-focused --show-cursor" : "");
          slot.innerHTML = '<span aria-hidden="true">​</span>' + (digit ? escape(digit) : i === code.length ? "" : "-");
        });
        if (code.length === 6) {
          verify();
        }
      });
      form.querySelector(".code-login-form__verify").addEventListener("click", function () {
        verify();
      });
      form.querySelector(".code-login-form__resend").addEventListener("click", function () {
        requestCode().then(function () {
          var notice = form.querySelector(".code-login-form__notice");
          if (!notice) {
            notice = document.createElement("p");
            notice.className = "code-login-form__notice";
            notice.setAttribute("aria-live", "polite");
            form.querySelector(".code-login-form__error").after(notice);
          }
          notice.textContent = S.code_resent;
          startCooldown();
        });
      });
      var change = form.querySelector(".code-login-form__change-email");
      if (change) {
        change.addEventListener("click", function () {
          window.location.reload();
        });
      }
      startCooldown();
    }

    function signupDetailsStep(suggested) {
      leaveIntroduction("login");
      form.innerHTML =
        '<input aria-hidden="true" autocomplete="username" class="code-login-form__hidden-email sr-only" name="email" readonly="" tabindex="-1" type="email" value="' +
        escape(email) +
        '"><div class="code-login-form__signup-details-step"><h2 class="code-login-form__title">' +
        escape(S.signup_details_title) +
        '</h2><div class="code-login-form__new-account"><button aria-label="' +
        escape(S.change_avatar) +
        '" class="code-login-form__avatar" title="' +
        escape(S.change_avatar) +
        '" type="button"><span class="code-login-form__avatar-placeholder">' +
        icon("user") +
        '</span><span class="code-login-form__avatar-edit">' +
        icon("pencil") +
        '</span></button><div class="code-login-form__username-field"><label for="code-login-username">' +
        escape(S.username_label) +
        '</label><div class="code-login-form__username-input"><input aria-describedby="code-login-username-error" autocomplete="off" class="code-login-form__new-account-username" id="code-login-username" name="username" placeholder="' +
        escape(S.username_placeholder) +
        '" type="text" value="' +
        escape(suggested || "") +
        '"></div><div aria-live="polite" class="code-login-form__error" id="code-login-username-error" role="alert"></div></div></div>' +
        button("btn-large btn-primary code-login-form__continue-to-site", S.account_ready_continue) +
        "</div>";
      var input = form.querySelector("#code-login-username");
      input.focus();
      // AvatarSelectorModal is not ported.
      form.querySelector(".code-login-form__avatar").addEventListener("click", function () {
        var error = document.getElementById("page-error");
        if (error) {
          error.textContent = "Not ported yet: choosing an avatar while signing up.";
          error.hidden = false;
        }
      });
      form.querySelector(".code-login-form__continue-to-site").addEventListener("click", function () {
        verify(input.value.trim());
      });
    }

    function verify(username) {
      var data = {
        invite_key: key,
        email: email,
        code: code,
        timezone: timezone(),
      };
      if (username !== undefined) {
        data.username = username;
      }
      return request("POST", "/session/login-code/verify.json", data).then(function (result) {
        var b = result.body || {};
        if (b.username_required) {
          signupDetailsStep(b.username);
        } else if (b.username_errors && b.username_errors.length) {
          var el = form.querySelector("#code-login-username-error");
          if (el) {
            el.textContent = b.username_errors[0];
          }
        } else if (b.success && (b.redirect_url || b.account_created)) {
          window.location.href = b.redirect_url || base + "/";
        } else {
          setError(errorOf(result));
        }
      });
    }

    var emailForm = form.querySelector("form");
    var continueButton = form.querySelector(".code-login-form__continue");
    var submitEmail = function (event) {
      if (event) {
        event.preventDefault();
      }
      if (isLink) {
        email = (form.querySelector("input[name=email]").value || "").trim();
        if (!email) {
          setError(S.email_required);
          return;
        }
      }
      continueButton.disabled = true;
      requestCode()
        .then(function (result) {
          var b = result.body || {};
          if (result.ok && b.success) {
            codeStep();
          } else {
            setError(errorOf(result));
          }
        })
        .finally(function () {
          if (continueButton.isConnected) {
            continueButton.disabled = false;
          }
        });
    };
    if (emailForm) {
      emailForm.addEventListener("submit", submitEmail);
    } else if (continueButton) {
      continueButton.addEventListener("click", function () {
        submitEmail();
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
