// The login flow's pages as the Ember app runs them, over the markup
// src/login_page_view.rs draws:
//
// - login (LoginPageController, LocalLoginForm, LoginPageCta,
//   LoginButtons): the flash, logging in and the hidden login form, the
//   password mask and caps lock warning, the second factor and security
//   key forms, passkeys, the email login link, the one-time code form,
//   external logins, the ForgotPassword modal, NotActivated;
// - signup (SignupPageController): the validations as the fields change,
//   the username and email checks, creating the account, an external
//   login's new account, the code signup's email step;
// - the login-required front page and login-preferences: their buttons;
// - the forgot-password route over the homepage: its URL and the modal;
// - password reset (PasswordResetController).
//
// #login-data names the page and carries its token, settings and strings.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (!Discourse.once || !Discourse.once("login")) {
    return;
  }

  var D = {};
  var S = {};

  function t(key, params) {
    var text = S[key];
    // A pluralized string: its form for the count.
    if (text === undefined && params && params.count !== undefined) {
      text = S[key + (Number(params.count) === 1 ? ".one" : ".other")];
    }
    if (text === undefined) {
      text = "[en." + key + "]";
    }
    Object.keys(params || {}).forEach(function (name) {
      text = text.split("%{" + name + "}").join(params[name]);
    });
    return text;
  }

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML.replace(/"/g, "&quot;");
  }

  function base() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return (meta && meta.content) || "";
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  // DButton: `classes` the invocation's, `attrs` the other attributes.
  function dButton(classes, label, opts) {
    opts = opts || {};
    var content = "";
    if (opts.icon || opts.loading) {
      content = label ? " btn-icon-text" : " no-text btn-icon";
    }
    var svg = opts.loading ? icon("spinner", "loading-icon") : opts.icon ? icon(opts.icon) : "";
    return (
      '<button class="' +
      (opts.loading ? "is-loading " : "") +
      "btn" +
      content +
      " " +
      classes +
      '"' +
      (opts.loading || opts.disabled ? ' disabled=""' : "") +
      (opts.attrs || "") +
      ' type="' +
      (opts.type || "button") +
      '">' +
      svg +
      (label
        ? '<span class="d-button-label">' + escape(label) + "</span>"
        : '<span aria-hidden="true">&#8203;</span>') +
      "</button>"
    );
  }

  function fromHtml(html) {
    var template = document.createElement("template");
    template.innerHTML = html.trim();
    return template.content.firstChild;
  }

  function readCookie(name) {
    var match = document.cookie.match(new RegExp("(?:^|; )" + name + "=([^;]*)"));
    if (!match) {
      return null;
    }
    try {
      return decodeURIComponent(match[1].replace(/\+/g, " "));
    } catch (_) {
      return null;
    }
  }

  function setCookie(name, value) {
    document.cookie = name + "=" + encodeURIComponent(value) + "; path=/";
  }

  function removeCookie(name) {
    document.cookie = name + "=; path=/; max-age=0";
  }

  function storage(key, value) {
    try {
      if (value === undefined) {
        var stored = sessionStorage.getItem(key);
        sessionStorage.removeItem(key);
        return stored;
      }
      sessionStorage.setItem(key, value);
    } catch (_) {
      // Storage blocked (a private window): nothing carries over.
    }
    return null;
  }

  // An ajax request as the client makes them, with the page's token.
  function request(method, url, data) {
    var body = new URLSearchParams();
    (function add(prefix, value) {
      if (value === undefined || value === null) {
        return;
      }
      if (typeof value === "object") {
        Object.keys(value).forEach(function (k) {
          add(prefix ? prefix + "[" + k + "]" : k, value[k]);
        });
        return;
      }
      body.append(prefix, value);
    })("", data || {});
    var get = method === "GET";
    return fetch(base() + url + (get && data ? "?" + body.toString() : ""), {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json, text/javascript, */*; q=0.01",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": D.csrf || "",
        "X-Requested-With": "XMLHttpRequest",
      },
      body: get ? undefined : body.toString(),
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = JSON.parse(text);
        } catch (_) {
          json = null;
        }
        return { ok: response.ok, status: response.status, body: json || {}, text: text };
      });
    });
  }

  // applyBehaviorTransformer: the plugins' transformers for `name`
  // (Discourse.behaviorTransformers, registerBehaviorTransformer), each
  // wrapping the next with `next` and this page's `request`, around
  // `callback`.
  function applyBehaviorTransformer(name, callback) {
    var list = (Discourse.behaviorTransformers || {})[name] || [];
    var run = list.reduceRight(
      function (next, transformer) {
        return function () {
          return Promise.resolve(transformer({ next: next, request: request }));
        };
      },
      function () {
        return Promise.resolve(callback());
      }
    );
    return run();
  }

  // extractError: the response's errors, its error, else its text.
  function extractError(result) {
    var b = (result && result.body) || {};
    if (b.errors && b.errors.length) {
      return b.errors.join("\n");
    }
    if (b.error) {
      return b.error;
    }
    if (result && result.text && result.status >= 500) {
      return result.text.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();
    }
    return "Something went wrong.";
  }

  function popupAjaxError(result) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(extractError(result));
    }
  }

  function alertDialog(message) {
    if (Discourse.dialog) {
      Discourse.dialog.alert(message);
    }
  }

  // What the port refuses rather than guess at, on the page's error line.
  function refuse(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function timezone() {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    } catch (_) {
      return "";
    }
  }

  // isValidDestinationUrl
  function validDestination(url) {
    return (
      url &&
      url !== base() + "/" &&
      !["/login", "/signup", "/session/", "/auth/", "/uploads/"].some(function (p) {
        return url.indexOf(base() + p) === 0;
      })
    );
  }

  // The login and signup routes' beforeModel: the page left for this one
  // is where a login returns to. That is the URL the transition starts
  // from: the page navigated away from, or on a page load the one asked
  // for, which is the login page itself unless a route sent its visitor
  // here (showLogin from /new-topic, the forgot-password route).
  function keepDestination() {
    var previous =
      Discourse.previousUrl !== undefined ? Discourse.previousUrl : location.pathname + location.search;
    if (validDestination(previous)) {
      setCookie("destination_url", previous);
    } else if (document.referrer) {
      try {
        var referrer = new URL(document.referrer);
        if (referrer.origin === location.origin && referrer.pathname.indexOf(base() + "/t/") === 0) {
          setCookie("destination_url", document.referrer);
        }
      } catch (_) {
        // Not a URL: nothing to return to.
      }
    }
  }

  // The routes' DiscourseConnect and single external login starts.
  function autoLogin(signup) {
    if (D.settings.enable_discourse_connect) {
      var returnPath = readCookie("destination_url")
        ? base() + "/"
        : encodeURIComponent(location.pathname);
      window.location = base() + "/session/sso?return_path=" + returnPath;
      return true;
    }
    if (D.single_external) {
      externalLogin(D.single_external, signup);
      return true;
    }
    return false;
  }

  // A LoginButtons button's authenticator: its class besides btn and
  // btn-social.
  function providerOf(button) {
    return Array.prototype.filter.call(button.classList, function (c) {
      return c !== "btn" && c !== "btn-social";
    })[0];
  }

  // LoginMethod#doLogin: a POST to the provider's start, with the token.
  function externalLogin(name, signup) {
    var form = document.createElement("form");
    form.setAttribute("style", "display:none;");
    form.setAttribute("method", "post");
    form.setAttribute("action", base() + "/auth/" + name);
    form.setAttribute("hx-boost", "false");
    var params = { authenticity_token: D.csrf };
    if (signup) {
      params.signup = true;
      var email = readCookie("email");
      if (email) {
        params.email = email;
      }
    }
    var destination = readCookie("destination_url");
    if (destination) {
      params.origin = destination;
    }
    Object.keys(params).forEach(function (key) {
      var input = document.createElement("input");
      input.setAttribute("name", key);
      input.setAttribute("value", params[key]);
      form.appendChild(input);
    });
    document.body.appendChild(form);
    form.submit();
  }

  // DFlashMessage at the top of the page (`flash`, `flashType`), or none.
  function setFlash(root, html, type) {
    var old = root.querySelector(":scope > .alert");
    if (old) {
      old.remove();
    }
    if (!html) {
      return;
    }
    var flash = document.createElement("div");
    flash.className = "alert" + (type ? " alert-" + type : "");
    flash.innerHTML = html;
    root.insertBefore(flash, root.firstChild);
  }

  // DPasswordField's caps lock guess and DTogglePasswordMask, for a
  // password input and the warning its owner draws.
  function passwordField(input, warning, onToggleShown) {
    var canToggle = false;
    function caps(on) {
      if (warning) {
        warning.classList.toggle("hidden", !on);
      }
    }
    input.addEventListener("keypress", function (e) {
      var w = e.which || e.keyCode;
      if ((w >= 65 && w <= 90 && !e.shiftKey) || (w >= 97 && w <= 122 && e.shiftKey)) {
        canToggle = true;
        caps(true);
      } else if ((w >= 65 && w <= 90 && e.shiftKey) || (w >= 97 && w <= 122 && !e.shiftKey)) {
        canToggle = true;
        caps(false);
      }
    });
    input.addEventListener("keyup", function (e) {
      if ((e.which || e.keyCode) === 20 && canToggle && warning) {
        caps(warning.classList.contains("hidden"));
      }
    });
    input.addEventListener("focusout", function () {
      caps(false);
    });
    input.addEventListener("focusin", function () {
      canToggle = false;
    });
    if (onToggleShown) {
      onToggleShown();
    }
  }

  function maskButton(masked, tabindex) {
    return dButton("btn-transparent toggle-password-mask", "", {
      icon: masked ? "far-eye" : "far-eye-slash",
      attrs:
        (tabindex ? ' tabindex="' + tabindex + '"' : "") +
        ' title="' +
        escape(t(masked ? "login.show_password_title" : "login.hide_password_title")) +
        '"',
    });
  }

  function bindMask(button, input) {
    button.addEventListener("click", function () {
      var masked = input.type === "password";
      input.type = masked ? "text" : "password";
      var next = fromHtml(maskButton(!masked, button.getAttribute("tabindex")));
      button.innerHTML = next.innerHTML;
      button.title = next.title;
    });
  }

  function valueEntered(input) {
    input.classList.toggle("value-entered", !!input.value);
  }

  // A six-slot DOtp over `input`, whose value the slots follow.
  function otpHtml(attrs) {
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
      '<div class="d-otp"><div class="d-otp-group">' +
      slots +
      '</div><div class="d-otp-input-wrapper"><input aria-label="' +
      escape(t("d_otp.screen_reader", { count: 6 })) +
      '" autocapitalize="off" autocomplete="one-time-code" autocorrect="off" autofocus="" class="d-otp-input' +
      (attrs.className ? " " + attrs.className : "") +
      '" data-slot="input-otp" inputmode="numeric" maxlength="6" spellcheck="false"' +
      (attrs.id ? ' id="' + attrs.id + '"' : "") +
      "></div></div>"
    );
  }

  function bindOtp(root, onFill) {
    var input = root.querySelector(".d-otp-input");
    var slots = root.querySelectorAll(".d-otp-slot");
    function paint(code, focused) {
      slots.forEach(function (slot, i) {
        var digit = code[i];
        var current = focused && i === Math.min(code.length, 5);
        var cursor = current && !digit;
        var placeholder = !digit && !cursor;
        slot.className =
          "d-otp-slot" +
          (current ? " --is-focused" : "") +
          (cursor ? " --show-cursor" : "") +
          (placeholder ? " --placeholder" : "");
        slot.innerHTML = '<span aria-hidden="true">&#8203;</span>' + (digit ? escape(digit) : placeholder ? "-" : "");
      });
    }
    input.addEventListener("input", function () {
      var code = input.value.replace(/\D/g, "").slice(0, 6);
      input.value = code;
      paint(code, true);
      if (code.length === 6 && onFill) {
        onFill(code);
      }
    });
    input.addEventListener("focus", function () {
      paint(input.value, true);
    });
    input.addEventListener("blur", function () {
      paint(input.value, false);
    });
    input.focus();
    return {
      input: input,
      clear: function () {
        input.value = "";
        paint("", document.activeElement === input);
      },
    };
  }

  // ---- The login page -------------------------------------------------

  var TOTP = 1;
  var BACKUP_CODE = 2;
  var SECURITY_KEY = 3;

  function loginPage() {
    var root = document.querySelector(".login-fullpage");
    if (!root) {
      return;
    }
    var page = root.querySelector(".login-body");
    var mobile = document.documentElement.classList.contains("mobile-view");
    var flash = function (html, type) {
      setFlash(root, html, type);
    };
    var flashText = function (text, type) {
      flash(escape(text), type);
    };

    var loginFlash = storage("discourse-login-flash");
    if (loginFlash) {
      flashText(loginFlash, "error");
    }

    var state = {
      loggingIn: false,
      loggedIn: false,
      loginName: storage("discourse-login-name") || "",
      loginPassword: "",
      secondFactorRequired: false,
      showSecondFactor: false,
      showSecurityKey: false,
      secondFactorMethod: TOTP,
      secondFactorToken: "",
      securityKeyCredential: null,
      codeSelected: false,
    };

    function bodyClasses() {
      var classes = ["login-body", "login-body"];
      var hasButton = !!document.querySelector("#login-buttons:not(.hidden)") || D.has_login_button;
      var twoFactor = state.showSecondFactor || state.showSecurityKey;
      if (hasButton && !twoFactor) {
        classes.push("has-alt-auth");
      }
      if (!D.settings.enable_local_logins) {
        classes.push("no-local-login");
      }
      if (twoFactor) {
        classes.push("second-factor");
      }
      if (state.codeSelected) {
        classes.push("code-login");
      }
      page.className = classes.join(" ");
    }
    D.has_login_button = !!root.querySelector("#login-buttons:not(.hidden)");

    // LoginPageCta's buttons as the state draws them.
    function drawCta() {
      var cta = root.querySelector(".login-page-cta__buttons");
      if (!cta) {
        return;
      }
      var old = cta.querySelector("#login-button");
      var html = state.showSecurityKey
        ? ""
        : dButton("btn-large btn-primary login-page-cta__login", t(state.loggingIn ? "login.logging_in" : "login.title"), {
            loading: state.loggingIn,
            disabled: state.loggingIn || state.loggedIn,
            type: "submit",
            attrs:
              ' form="login-form" id="login-button"' + (state.showSecondFactor ? "" : ' tabindex="2"'),
          });
      if (old) {
        if (html) {
          old.replaceWith(fromHtml(html));
        } else {
          old.remove();
        }
      } else if (html) {
        cta.insertBefore(fromHtml(html), cta.firstChild);
      }
      // showSignupLink: the text and the Sign Up button, not drawn while
      // the second factor is asked for.
      var signup = cta.querySelector("#new-account-link");
      var signupText = cta.querySelector(".login-page-cta__no-account-yet");
      var showSignup = !!signupHtml && !state.showSecondFactor;
      if (!showSignup) {
        if (signup) {
          signup.remove();
        }
        if (signupText) {
          signupText.remove();
        }
      } else {
        if (!signup) {
          cta.insertAdjacentHTML("beforeend", signupHtml);
          signup = cta.querySelector("#new-account-link");
          bindCta();
        }
        if (state.loggingIn) {
          signup.setAttribute("disabled", "");
        } else {
          signup.removeAttribute("disabled");
        }
      }
    }
    // The signup link's markup, to draw it again: the page's, or the call
    // to action's template when the page starts on the code form.
    var signupHtml = (function () {
      var scope = root;
      var template = document.getElementById("login-cta");
      if (!root.querySelector(".login-page-cta") && template) {
        scope = template.content;
      }
      var link = scope.querySelector("#new-account-link");
      var text = scope.querySelector(".login-page-cta__no-account-yet");
      return link ? (text ? text.outerHTML : "") + link.outerHTML : "";
    })();

    function form() {
      return root.querySelector("#login-form");
    }

    // LocalLoginForm's behaviour on a freshly drawn form.
    function bindLocalForm() {
      var f = form();
      if (!f) {
        return;
      }
      var name = f.querySelector("#login-account-name");
      var password = f.querySelector("#login-account-password");
      name.value = state.loginName;
      valueEntered(name);
      var emailLink = f.querySelector("#email-login-link");
      function nameChanged() {
        state.loginName = name.value;
        valueEntered(name);
        if (emailLink) {
          emailLink.className = state.loginName ? "" : "no-login-filled";
        }
      }
      nameChanged();
      name.addEventListener("input", nameChanged);
      [name, password].forEach(function (input) {
        input.addEventListener("focusin", function () {
          if (input.scrollIntoView) {
            input.scrollIntoView({ behavior: "smooth", block: "center" });
          }
        });
      });
      var warning = password.parentNode.querySelector(".caps-lock-warning");
      passwordField(password, warning);
      password.addEventListener("input", function () {
        state.loginPassword = password.value;
        valueEntered(password);
        var mask = password.parentNode.querySelector(".toggle-password-mask");
        if (state.loginPassword && !mask) {
          mask = fromHtml(maskButton(password.type === "password"));
          password.parentNode.querySelector("label.alt-placeholder").after(mask);
          bindMask(mask, password);
        } else if (!state.loginPassword && mask) {
          mask.remove();
        }
      });
      f.addEventListener("submit", function (event) {
        event.preventDefault();
        localLogin();
      });
      f.querySelector("#forgot-password-link").addEventListener("click", function (event) {
        event.preventDefault();
        forgotPassword(state.loginName);
      });
      if (emailLink) {
        emailLink.addEventListener("click", emailLogin);
      }
      var codeLink = f.querySelector("#one-time-code-link");
      if (codeLink) {
        codeLink.addEventListener("click", function (event) {
          event.preventDefault();
          showCodeLogin(true);
        });
      }
      if (D.passkeys) {
        passkeyLogin("conditional");
      }
    }

    // The one-time code form in place of the password's, and back.
    function showCodeLogin(on) {
      var left = root.querySelector("#login-form, .code-login-form").parentNode;
      var current = left.querySelector("#login-form, .code-login-form");
      var cta = root.querySelector(".login-page-cta");
      state.codeSelected = on;
      if (on) {
        var code = fromHtml(document.getElementById("login-code-form").innerHTML);
        current.replaceWith(code);
        if (cta) {
          cta.remove();
        }
        bindCodeForm(code, "login", function (email) {
          if (typeof email === "string") {
            state.loginName = email;
          }
          showCodeLogin(false);
        }, state.loginName);
      } else {
        var local = fromHtml(document.getElementById("login-password-form").innerHTML);
        current.replaceWith(local);
        var ctaHtml = fromHtml(document.getElementById("login-cta").innerHTML);
        if (mobile) {
          page.appendChild(ctaHtml);
        } else {
          local.after(ctaHtml);
        }
        bindCta();
        bindLocalForm();
        drawCta();
      }
      bodyClasses();
    }

    function bindCta() {
      var signup = root.querySelector("#new-account-link");
      if (signup) {
        signup.addEventListener("click", function () {
          if (signup.hasAttribute("disabled")) {
            return;
          }
          // createAccount: what was typed carries over.
          var prefill = {};
          if (state.loginName.indexOf("@") > 0) {
            prefill.accountEmail = state.loginName;
          } else {
            prefill.accountUsername = state.loginName;
          }
          storage("discourse-signup-prefill", JSON.stringify(prefill));
          window.location = base() + "/signup";
        });
      }
    }

    // LocalLoginForm#emailLogin: a login link by email.
    var processingEmailLink = false;
    function emailLogin(event) {
      event.preventDefault();
      if (processingEmailLink) {
        return;
      }
      if (!state.loginName) {
        flashText(t("login.blank_username"), "info");
        return;
      }
      processingEmailLink = true;
      request("POST", "/u/email-login", { login: state.loginName.trim() })
        .then(function (result) {
          if (!result.ok) {
            popupAjaxError(result);
            return;
          }
          var loginName = escape(state.loginName);
          var key = loginName.match(/@/) ? "email_login.complete_email" : "email_login.complete_username";
          var data = result.body;
          if (data.user_found === false) {
            flash(t(key + "_not_found", { email: loginName, username: loginName }), "error");
          } else {
            flash(t(key + (data.hide_taken ? "" : "_found"), { email: loginName, username: loginName }), "success");
          }
        })
        .finally(function () {
          processingEmailLink = false;
        });
    }

    // LoginPageController#localLogin
    function localLogin() {
      if (state.loggingIn || state.loggedIn) {
        return;
      }
      if (!state.loginName || !state.loginPassword) {
        flashText(t("login.blank_username_or_password"), "error");
        return;
      }
      state.loggingIn = true;
      drawCta();
      request("POST", "/session", {
        login: state.loginName,
        password: state.loginPassword,
        second_factor_token: state.securityKeyCredential || state.secondFactorToken,
        second_factor_method: state.secondFactorMethod,
        timezone: timezone(),
      })
        .then(function (r) {
          if (!r.ok) {
            failed(r);
            return;
          }
          var result = r.body;
          if (result && result.error) {
            state.loggingIn = false;
            flash(null);
            if ((result.security_key_enabled || result.totp_enabled) && !state.secondFactorRequired) {
              state.secondFactorRequired = true;
              showSecondFactorForm(result);
              drawCta();
              return;
            } else if (result.reason === "not_activated") {
              showNotActivated(result);
            } else if (result.reason === "suspended") {
              alertDialog(result.error);
            } else if (result.reason === "expired") {
              flash(t("login.password_expired", { reset_url: base() + "/password-reset" }), "error");
            } else {
              flashText(result.error, "error");
            }
            if (state.showSecurityKey) {
              state.securityKeyCredential = null;
            }
            drawCta();
            return;
          }
          state.loggedIn = true;
          drawCta();
          submitHiddenForm(state.loginName, state.loginPassword, null);
        })
        .catch(function () {
          failed(null);
        });
    }

    function failed(r) {
      state.loggingIn = false;
      drawCta();
      if (r && r.status === 429) {
        flashText(t("login.rate_limit"), "error");
      } else if (r && r.status === 503) {
        flashText(t(D.staff_writes_only ? "staff_writes_only_mode.login_disabled" : "read_only_mode.login_disabled"), "error");
      } else if (!navigator.cookieEnabled) {
        flashText(t("login.cookies_error"), "error");
      } else {
        flashText(t("login.error"), "error");
      }
    }

    // The second factor (SecondFactorForm with DSecondFactorInput) or the
    // security key (SecurityKeyForm) in place of the credentials.
    var sf = {};
    function showSecondFactorForm(result) {
      sf = {
        other: result.multiple_second_factor_methods,
        backup: result.backup_enabled,
        totp: result.totp_enabled,
        challenge: result.challenge,
        allowed: result.allowed_credential_ids || [],
      };
      state.showSecondFactor = !!result.totp_enabled;
      state.showSecurityKey = !!result.security_key_enabled;
      state.secondFactorMethod = result.security_key_enabled ? SECURITY_KEY : TOTP;
      // showLoginButtons: false.
      root.querySelectorAll("#login-buttons").forEach(function (b) {
        var side = b.closest(".login-right-side");
        (side || b).remove();
      });
      drawSecondFactor();
    }

    function drawSecondFactor() {
      var f = form();
      var credentials = f.querySelector("#credentials");
      var twoFactor = state.showSecondFactor || state.showSecurityKey;
      credentials.className = twoFactor ? "hidden" : "";
      f.querySelector("#login-account-name").disabled = state.showSecondFactor;
      f.querySelector("#login-account-password").disabled = twoFactor;
      var old = f.querySelector("#second-factor");
      if (old) {
        old.remove();
      }
      bodyClasses();
      if (!twoFactor) {
        return;
      }
      var method = state.secondFactorMethod;
      var title = method === BACKUP_CODE ? t("login.second_factor_backup_title") : t("login.second_factor_title");
      var description = {};
      description[TOTP] = t("login.second_factor_description");
      description[SECURITY_KEY] = t("login.security_key_description");
      description[BACKUP_CODE] = t("login.second_factor_backup_description");
      var inner;
      if (state.showSecurityKey) {
        inner =
          '<div id="security-key">' +
          dButton("btn-large btn-primary", t("login.security_key_authenticate"), {
            icon: "key",
            attrs: ' id="security-key-authenticate-button"',
          }) +
          "<p>" +
          (sf.other
            ? '<a class="toggle-second-factor-method" href="">' + escape(t("login.security_key_alternative")) + "</a>"
            : "") +
          "</p></div>";
      } else if (method === TOTP) {
        inner = otpHtml({ className: "second-factor-token-input", id: "login-second-factor" });
      } else {
        inner =
          '<input autocapitalize="off" autocorrect="off" autofocus="" class="second-factor-token-input" maxlength="32" pattern="[a-z0-9]{16}" type="text" id="login-second-factor" value="">';
      }
      var toggle =
        sf.backup && sf.totp && method !== SECURITY_KEY
          ? '<p><a class="toggle-second-factor-method" href="">' +
            escape(t(method === TOTP ? "login.second_factor_backup" : "login.second_factor")) +
            "</a></p>"
          : "";
      var form2 = fromHtml(
        '<div id="second-factor"><h3>' +
          escape(title) +
          '</h3><p class="second-factor__description">' +
          escape(description[method]) +
          "</p>" +
          inner +
          toggle +
          "</div>"
      );
      f.appendChild(form2);
      if (state.showSecurityKey) {
        form2.querySelector("#security-key-authenticate-button").addEventListener("click", authenticateSecurityKey);
        var other = form2.querySelector("#security-key .toggle-second-factor-method");
        if (other) {
          other.addEventListener("click", function (event) {
            event.preventDefault();
            state.showSecurityKey = false;
            state.showSecondFactor = true;
            if (sf.totp) {
              state.secondFactorMethod = TOTP;
            } else if (sf.backup) {
              state.secondFactorMethod = BACKUP_CODE;
            }
            drawSecondFactor();
            drawCta();
          });
        }
      } else if (method === TOTP) {
        bindOtp(form2, function (code) {
          state.secondFactorToken = code;
          localLogin();
        });
      } else {
        var token = form2.querySelector("#login-second-factor");
        token.addEventListener("input", function () {
          state.secondFactorToken = token.value;
        });
        token.focus();
      }
      var link = form2.querySelector(":scope > p > .toggle-second-factor-method");
      if (link) {
        link.addEventListener("click", function (event) {
          event.preventDefault();
          state.secondFactorToken = "";
          state.secondFactorMethod = state.secondFactorMethod === TOTP ? BACKUP_CODE : TOTP;
          drawSecondFactor();
        });
      }
    }

    function toBuffer(s) {
      return Uint8Array.from(s, function (c) {
        return c.charCodeAt(0);
      });
    }

    function toBase64(b) {
      return btoa(String.fromCharCode.apply(null, new Uint8Array(b)));
    }

    var abort = null;
    function abortSignal() {
      if (abort) {
        var error = new Error("Cancelling pending webauthn call");
        error.name = "AbortError";
        abort.abort(error);
      }
      abort = new AbortController();
      return abort.signal;
    }

    // getWebauthnCredential
    function authenticateSecurityKey() {
      if (typeof PublicKeyCredential === "undefined") {
        flashText(t("login.security_key_support_missing_error"), "error");
        return;
      }
      navigator.credentials
        .get({
          publicKey: {
            challenge: toBuffer(sf.challenge || ""),
            allowCredentials: sf.allowed.map(function (id) {
              return { id: toBuffer(atob(id)), type: "public-key" };
            }),
            timeout: 60000,
            userVerification: "discouraged",
          },
          signal: abortSignal(),
        })
        .then(function (credential) {
          if (
            !sf.allowed.some(function (id) {
              return toBase64(credential.rawId) === id;
            })
          ) {
            flashText(t("login.security_key_no_matching_credential_error"), "error");
            return;
          }
          state.securityKeyCredential = {
            signature: toBase64(credential.response.signature),
            clientData: toBase64(credential.response.clientDataJSON),
            authenticatorData: toBase64(credential.response.authenticatorData),
            credentialId: toBase64(credential.rawId),
          };
          localLogin();
        })
        .catch(function (err) {
          flashText(err.name === "NotAllowedError" ? t("login.security_key_not_allowed_error") : String(err.message || err), "error");
        });
    }

    // LoginPageController#passkeyLogin with getPasskeyCredential.
    function passkeyLogin(mediation) {
      if (typeof PublicKeyCredential === "undefined") {
        if (mediation !== "conditional") {
          alertDialog(t("login.security_key_support_missing_error"));
        }
        return;
      }
      request("GET", "/session/passkey/challenge.json")
        .then(function (resp) {
          if (!resp.ok) {
            throw resp;
          }
          return navigator.credentials.get({
            publicKey: { challenge: toBuffer(resp.body.challenge), userVerification: "required" },
            signal: abortSignal(),
            mediation: mediation,
          });
        })
        .then(function (credential) {
          if (!credential) {
            return;
          }
          return request("POST", "/session/passkey/auth.json", {
            publicKeyCredential: {
              signature: toBase64(credential.response.signature),
              clientData: toBase64(credential.response.clientDataJSON),
              authenticatorData: toBase64(credential.response.authenticatorData),
              credentialId: toBase64(credential.rawId),
              userHandle: toBase64(credential.response.userHandle),
            },
          }).then(function (r) {
            if (!r.ok) {
              popupAjaxError(r);
            } else if (r.body && !r.body.error) {
              var destination = readCookie("destination_url");
              if (destination) {
                removeCookie("destination_url");
                window.location.assign(destination);
              } else {
                window.location.reload();
              }
            } else {
              alertDialog(r.body.error);
            }
          });
        })
        .catch(function (error) {
          if (!error || error.name === "AbortError" || mediation === "conditional") {
            return;
          }
          if (error.status) {
            popupAjaxError(error);
          } else if (error.name === "NotAllowedError") {
            alertDialog(t("login.security_key_not_allowed_error"));
          } else if (error.name === "SecurityError") {
            alertDialog(t("login.passkey_security_error", { message: error.message }));
          } else {
            alertDialog(error.message);
          }
        });
    }

    // NotActivatedModal
    function showNotActivated(result) {
      if (!Discourse.modal) {
        return;
      }
      Discourse.modal.show({
        className: "not-activated-modal",
        title: t("log_in"),
        body: function (body) {
          body.innerHTML = t("login.not_activated", { sentTo: escape(result.sent_to_email) });
        },
        footer: function (footer) {
          var controls = '<div class="activation-controls">';
          if (!D.settings.must_approve_users) {
            controls += dButton("btn-primary resend", t("login.resend_title"), { icon: "envelope" });
          }
          if (D.settings.enable_local_logins || D.settings.email_editable) {
            controls += dButton("btn-default edit-email", t("login.change_email"), { icon: "pencil" });
          }
          footer.innerHTML = controls + "</div>";
          var resend = footer.querySelector(".resend");
          if (resend) {
            resend.addEventListener("click", function () {
              refuse("resending the activation email (users#send_activation_email)");
            });
          }
          var edit = footer.querySelector(".edit-email");
          if (edit) {
            edit.addEventListener("click", function () {
              refuse("changing the activation email (ActivationEdit)");
            });
          }
        },
      });
    }

    root.querySelectorAll("#login-buttons .btn-social:not(.passkey-login-button)").forEach(function (button) {
      button.addEventListener("click", function () {
        if (state.loggingIn || state.loggedIn) {
          return;
        }
        state.loggingIn = true;
        drawCta();
        externalLogin(providerOf(button), false);
      });
    });
    root.querySelectorAll(".passkey-login-button").forEach(function (button) {
      button.addEventListener("click", function () {
        passkeyLogin("optional");
      });
    });

    if (autoLogin(false)) {
      return;
    }
    if (root.querySelector(".code-login-form")) {
      state.codeSelected = true;
      bindCodeForm(root.querySelector(".code-login-form"), "login", function (email) {
        if (typeof email === "string") {
          state.loginName = email;
        }
        showCodeLogin(false);
      });
    } else {
      bindCta();
      bindLocalForm();
    }
  }

  // The hidden static login form, so the browser's password manager sees
  // a login; then on to where the reader was headed.
  function submitHiddenForm(username, password, redirect) {
    var form = document.getElementById("hidden-login-form");
    if (!form) {
      window.location = redirect || base() + "/";
      return;
    }
    var set = function (key, value) {
      form.querySelector("input[name=" + key + "]").value = value;
    };
    set("username", username);
    set("password", password || "");
    var destination = readCookie("destination_url");
    if (redirect) {
      set("redirect", redirect);
    } else if (destination) {
      removeCookie("destination_url");
      set("redirect", destination);
    } else {
      set("redirect", window.location.href);
    }
    var ua = navigator.userAgent;
    if (/iPhone|iPad|iPod/.test(ua) && /Safari/.test(ua) && !/CriOS|FxiOS/.test(ua)) {
      window.location.href = form.querySelector("input[name=redirect]").value;
    } else {
      form.submit();
    }
  }

  // ---- CodeLoginForm --------------------------------------------------

  // The email step (login and signup), the code step (a password reset):
  // `onUsePassword` for the login page's way back.
  function bindCodeForm(root, context, onUsePassword, initialEmail) {
    var email = initialEmail || "";
    var emailForm = root.querySelector("form.code-login-form__email-step");

    function setError(container, text) {
      var error = container.querySelector(".code-login-form__error");
      if (!text) {
        if (error && !error.hasAttribute("data-keep")) {
          error.remove();
        }
        return;
      }
      if (!error) {
        error = fromHtml('<div aria-live="polite" class="code-login-form__error" role="alert"></div>');
        var actions = container.querySelector(".code-login-form__email-actions");
        actions.before(error);
      }
      error.textContent = text;
    }

    // FormKit's error on the email field: the form's summary and the
    // field's meta, its input described by the error.
    function fieldError(text, withIcon) {
      var field = emailForm.querySelector("#control-email");
      var input = field.querySelector("input");
      var summary = emailForm.querySelector(".form-kit__errors-summary");
      var meta = field.querySelector(".form-kit__meta");
      if (summary) {
        summary.remove();
      }
      if (meta) {
        meta.remove();
      }
      field.classList.toggle("has-error", !!text);
      if (!text) {
        input.removeAttribute("aria-describedby");
        input.removeAttribute("aria-invalid");
        return;
      }
      var label = field.querySelector(".form-kit__container-title span").textContent;
      emailForm.insertBefore(
        fromHtml(
          '<div aria-live="assertive" class="form-kit__errors-summary"><h2 class="form-kit__errors-summary-title">' +
            icon("triangle-exclamation") +
            escape(t("form_kit.errors_summary_title", { count: 1 })) +
            '</h2><ul class="form-kit__errors-summary-list"><li><a href="#control-email" rel="noopener noreferrer">' +
            escape(label) +
            "</a>: " +
            escape(text) +
            "</li></ul></div>"
        ),
        emailForm.firstChild
      );
      input.setAttribute("aria-describedby", "control-email-error");
      input.setAttribute("aria-invalid", "true");
      field
        .querySelector(".form-kit__container-content")
        .appendChild(
          fromHtml(
            '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors" id="control-email-error"><span>' +
              (withIcon ? icon("triangle-exclamation") : "") +
              escape(text) +
              "</span></p></div>"
          )
        );
    }

    if (emailForm) {
      var input = emailForm.querySelector('input[type="email"]');
      if (email) {
        input.value = email;
      }
      emailForm.addEventListener("submit", function (event) {
        event.preventDefault();
        var value = (input.value || "").trim();
        if (!value) {
          fieldError(t("form_kit.errors.required"), true);
          return;
        }
        if (!/^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?$/.test(value)) {
          fieldError(t("user.email.invalid"), true);
          return;
        }
        fieldError(null);
        email = value;
        sendCode();
      });
      var toggle = emailForm.querySelector(".code-login-form__password-toggle");
      if (toggle && onUsePassword) {
        toggle.addEventListener("click", function () {
          onUsePassword(input.value);
        });
      }
    }

    // sendCode
    function sendCode() {
      setError(root, null);
      return request("GET", "/session/hp.json").then(function (hp) {
        if (!hp.ok) {
          popupAjaxError(hp);
          return false;
        }
        return request("POST", "/session/login-code", {
          email: email,
          signup: context === "signup",
          password_confirmation: hp.body.value,
          challenge: (hp.body.challenge || "").split("").reverse().join(""),
        }).then(function (result) {
          if (!result.ok) {
            popupAjaxError(result);
            return false;
          }
          if (result.body.error) {
            setError(root, result.body.error);
            return false;
          }
          // The code step outside a password reset waits on the login
          // codes themselves (/session/login-code), which the server
          // refuses so far.
          refuse("the one-time login code's code step");
          return true;
        });
      });
    }
  }

  // The password reset code (ForgotPassword's codeSent): CodeLoginForm in
  // the password-reset context at its code step.
  function passwordResetCodeStep(container, email, onChangeEmail) {
    var cooldown = 30;
    var timer = null;
    var code = "";
    var verifying = false;
    container.innerHTML =
      '<div class="code-login-form"><input aria-hidden="true" autocomplete="username" class="code-login-form__hidden-email sr-only" name="email" readonly="" tabindex="-1" type="email"><div class="code-login-form__code-step"><h2 class="code-login-form__title">' +
      escape(t("code_login.check_your_email")) +
      '</h2><p class="code-login-form__instructions">' +
      escape(t("forgot_password.code_instructions")) +
      "</p>" +
      otpHtml({ id: "code-login-otp-0" }) +
      '<div aria-live="polite" class="code-login-form__error" role="alert"></div>' +
      dButton("btn-primary code-login-form__verify", t("code_login.verify_button"), { type: "submit" }) +
      '<div class="code-login-form__actions">' +
      dButton("btn-flat code-login-form__resend", t("code_login.resend")) +
      dButton("btn-flat code-login-form__change-email", t("code_login.use_different_email")) +
      "</div></div></div>";
    var form = container.querySelector(".code-login-form");
    // value={{this.email}}: the property, not the attribute.
    form.querySelector(".code-login-form__hidden-email").value = email;
    var otp = bindOtp(form, function (value) {
      code = value;
      verify();
    });
    otp.input.addEventListener("input", function () {
      code = otp.input.value;
    });
    var error = form.querySelector(".code-login-form__error");
    var resend = form.querySelector(".code-login-form__resend");
    function tick() {
      var label = resend.querySelector(".d-button-label");
      if (cooldown > 0) {
        resend.disabled = true;
        label.textContent = t("code_login.resend_countdown", { count: cooldown });
        cooldown -= 1;
        timer = setTimeout(tick, 1000);
      } else {
        resend.disabled = false;
        label.textContent = t("code_login.resend");
      }
    }
    tick();
    function verify() {
      if (code.length < 6 || verifying) {
        return;
      }
      verifying = true;
      error.textContent = "";
      request("POST", "/session/password-reset-code/verify", { email: email, code: code, timezone: timezone() })
        .then(function (r) {
          if (!r.ok) {
            popupAjaxError(r);
            return;
          }
          var result = r.body;
          if (result.success === false && result.message) {
            error.textContent = result.message;
          } else if (result.second_factor_required) {
            refuse("a password reset code's second factor step");
          } else if (result.error) {
            error.textContent = result.error;
            code = "";
            otp.clear();
          } else {
            window.location = result.redirect_url;
          }
        })
        .finally(function () {
          verifying = false;
        });
    }
    form.querySelector(".code-login-form__verify").addEventListener("click", verify);
    resend.addEventListener("click", function () {
      if (resend.disabled) {
        return;
      }
      request("POST", "/session/forgot_password", { login: email.trim() }).then(function (r) {
        if (!r.ok) {
          popupAjaxError(r);
          return;
        }
        var notice = form.querySelector(".code-login-form__notice");
        if (!notice) {
          notice = fromHtml('<p aria-live="polite" class="code-login-form__notice"></p>');
          error.after(notice);
        }
        notice.textContent = t("code_login.code_resent");
        code = "";
        otp.clear();
        cooldown = 30;
        clearTimeout(timer);
        tick();
      });
    });
    form.querySelector(".code-login-form__change-email").addEventListener("click", function () {
      clearTimeout(timer);
      onChangeEmail();
    });
  }

  // ---- ForgotPassword -------------------------------------------------

  function forgotPassword(loginName) {
    if (!Discourse.modal) {
      return;
    }
    var hideTaken = D.settings.hide_email_address_taken;
    // handleForgotPassword: what looks like an email carries over, or
    // anything when taken addresses aren't hidden.
    var filled = loginName;
    if (hideTaken && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test((filled || "").trim())) {
      filled = null;
    }
    var value = filled || readCookie("email") || "";
    var offerHelp = null;
    var helpSeen = false;
    var codeSent = false;
    var disabled = false;
    var close = null;
    var bodyEl = null;
    var footerEl = null;

    function submitDisabled() {
      if (disabled) {
        return true;
      }
      return hideTaken ? value.indexOf("@") === -1 : !value.trim();
    }

    function modalFlash(text) {
      var container = bodyEl.parentNode;
      var old = container.querySelector("#modal-alert");
      if (old) {
        old.remove();
      }
      if (text) {
        var alert = fromHtml('<div id="modal-alert" role="alert" class="alert alert-error"></div>');
        alert.innerHTML = text;
        bodyEl.before(alert);
      }
    }

    function draw() {
      if (codeSent) {
        passwordResetCodeStep(bodyEl, value, function () {
          codeSent = false;
          draw();
        });
      } else if (offerHelp) {
        bodyEl.innerHTML = offerHelp;
      } else if (hideTaken) {
        bodyEl.innerHTML =
          '<label for="username-or-email">' +
          escape(t("forgot_password.invite_no_username")) +
          '</label><input autocapitalize="off" autocorrect="off" id="username-or-email" placeholder="' +
          escape(t("email")) +
          '" type="text" value="' +
          escape(value) +
          '">';
      } else {
        bodyEl.innerHTML =
          "<p>" +
          escape(t("forgot_password.invite")) +
          '</p><label for="username-or-email">' +
          escape(t("forgot_password.email-username")) +
          '</label><input autocapitalize="off" autocorrect="off" id="username-or-email" placeholder="' +
          escape(t("login.email_placeholder")) +
          '" type="text" value="' +
          escape(value) +
          '">';
      }
      var input = bodyEl.querySelector("#username-or-email");
      if (input) {
        input.addEventListener("input", function () {
          value = input.value;
          drawFooter();
        });
        input.addEventListener("keydown", function (event) {
          if (event.key === "Enter") {
            event.preventDefault();
            resetPassword();
          }
        });
      }
      drawFooter();
    }

    function drawFooter() {
      if (offerHelp) {
        footerEl.innerHTML =
          dButton("btn-large btn-primary", t("forgot_password.button_ok"), { type: "submit" }) +
          (helpSeen ? "" : dButton("btn-large", t("forgot_password.button_help"), { icon: "circle-question" }));
        footerEl.querySelector(".btn-primary").addEventListener("click", function () {
          close();
        });
        var help = footerEl.querySelector(".btn-large:not(.btn-primary)");
        if (help) {
          help.addEventListener("click", function () {
            offerHelp = t("forgot_password.help", { basePath: base() });
            helpSeen = true;
            draw();
          });
        }
      } else if (!codeSent) {
        footerEl.innerHTML = dButton("btn-primary forgot-password-reset", t("forgot_password.reset"), {
          type: "submit",
          disabled: submitDisabled(),
        });
        footerEl.querySelector(".forgot-password-reset").addEventListener("click", resetPassword);
      } else {
        footerEl.innerHTML = "";
      }
    }

    function resetPassword() {
      if (submitDisabled()) {
        return;
      }
      disabled = true;
      modalFlash(null);
      drawFooter();
      request("POST", "/session/forgot_password", { login: value.trim() })
        .then(function (r) {
          if (!r.ok) {
            modalFlash(escape(extractError(r)));
            return;
          }
          var data = r.body;
          if (data.email_code && data.user_found !== false) {
            codeSent = true;
            return;
          }
          var shown = escape(value);
          var key = "forgot_password.complete" + (shown.match(/@/) ? "_email" : "_username");
          if (data.user_found === false) {
            modalFlash(t(key + "_not_found", { email: shown, username: shown }));
          } else {
            key += data.user_found ? "_found" : "";
            value = "";
            offerHelp = t(key, { email: shown, username: shown });
            helpSeen = !data.user_found;
          }
        })
        .catch(function () {
          modalFlash(escape(t("login.error")));
        })
        .finally(function () {
          disabled = false;
          draw();
        });
    }

    close = Discourse.modal.show({
      className: "forgot-password-modal",
      title: t("forgot_password.title"),
      body: function (body) {
        bodyEl = body;
      },
      footer: function (footer) {
        footerEl = footer;
      },
    });
    draw();
  }

  // ---- The signup page ------------------------------------------------

  function emailValid(email) {
    return /^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?$/.test(email);
  }

  function signupPage() {
    var root = document.querySelector(".signup-fullpage");
    if (!root) {
      return;
    }
    var page = root.querySelector(".signup-body");
    var mobile = document.documentElement.classList.contains("mobile-view");
    var auth = null;
    try {
      auth = JSON.parse(storage("discourse-authentication-data") || "null");
    } catch (_) {
      auth = null;
    }
    // Back on the page (a failed signup keeps it): the data stays.
    if (auth) {
      storage("discourse-authentication-data", JSON.stringify(auth));
    }
    var prefill = {};
    try {
      prefill = JSON.parse(storage("discourse-signup-prefill") || "{}") || {};
    } catch (_) {
      prefill = {};
    }

    root.querySelectorAll("#login-buttons .btn-social:not(.passkey-login-button)").forEach(function (button) {
      button.addEventListener("click", function () {
        externalLogin(providerOf(button), true);
      });
    });

    if (!auth && autoLogin(true)) {
      return;
    }

    // An external login's new account: the create-account form with its
    // data (showCreateForm with authOptions), no other way in.
    if (auth) {
      var template = document.getElementById("signup-auth-form");
      var left = page.querySelector(":scope > div");
      if (template) {
        left.innerHTML = template.innerHTML;
      }
      var right = page.querySelector(".login-right-side");
      if (right) {
        right.remove();
      }
      var separator = page.querySelector(".login-or-separator");
      if (separator) {
        separator.remove();
      }
      page.className = page.className
        .split(" ")
        .filter(function (c) {
          return c !== "has-alt-auth" && c !== "passwordless-signup";
        })
        .join(" ");
      if (auth.auth_provider) {
        left.classList.add(auth.auth_provider);
      }
      // progressBarStep: activate.
      var segments = left.querySelectorAll(".signup-progress-bar__segment");
      segments.forEach(function (segment, i) {
        segment.className = "signup-progress-bar__segment --" + (i === 1 ? "active" : i < 1 ? "completed" : "incomplete");
      });
    }

    var form = root.querySelector("form#login-form");
    if (!form) {
      var code = root.querySelector(".code-login-form");
      if (code) {
        bindCodeForm(code, "signup", null, prefill.accountEmail || readCookie("email") || "");
      }
      return;
    }

    var emailInput = form.querySelector("#new-account-email");
    var usernameInput = form.querySelector("#new-account-username");
    var nameInput = form.querySelector("#new-account-name");
    var passwordInput = form.querySelector("#new-account-password");
    var honeypot = form.querySelector("#new-account-confirmation");
    var challenge = form.querySelector("#new-account-challenge");
    var inviteCode = form.querySelector("#inviteCode");
    var cta = root.querySelector(".signup-page-cta");
    var signupButton = cta.querySelector(".signup-page-cta__signup");

    var passwordRequired = !(auth && auth.auth_provider);
    var forceReason = false;
    var emailValidationVisible = false;
    var server = { email: null, emailValidation: null };
    var username = { checked: null, result: null, prefilled: null, timer: null };
    var rejectedEmails = [];
    var rejectedPasswords = {};
    var isDeveloper = false;
    var formSubmitted = false;
    var hp = { challengeDate: null, expiry: 0, promise: null };

    if (auth && auth.associate_url) {
      form.insertBefore(
        fromHtml(
          '<div class="input-group create-account-associate-link"><span>' +
            t("create_account.associate", {
              associate_link: auth.associate_url,
              provider: (D.provider_names || {})[auth.auth_provider] || auth.auth_provider,
            }) +
            "</span></div>"
        ),
        form.firstChild
      );
    }
    if (!passwordRequired) {
      // Only the honeypot stays in the password group.
      Array.prototype.slice.call(passwordInput.parentNode.children).forEach(function (child) {
        if (!child.classList.contains("password-confirmation")) {
          child.remove();
        }
      });
      passwordInput = null;
    }
    if (auth) {
      var existing = cta.querySelector(".signup-page-cta__existing-account");
      if (existing) {
        existing.remove();
      }
      var loginButton = cta.querySelector(".signup-page-cta__login");
      if (loginButton) {
        loginButton.remove();
      }
      emailInput.value = auth.email || "";
      usernameInput.value = auth.username || "";
      if (nameInput) {
        nameInput.value = auth.name || "";
      }
      if (auth.can_edit_username === false) {
        usernameInput.disabled = true;
      }
      if (nameInput && auth.can_edit_name === false && auth.name) {
        nameInput.disabled = true;
      }
    } else {
      if (readCookie("email")) {
        emailInput.value = readCookie("email");
      }
      if (prefill.accountEmail) {
        emailInput.value = prefill.accountEmail;
      }
      if (prefill.accountUsername) {
        usernameInput.value = prefill.accountUsername;
      }
    }

    function el(html) {
      return fromHtml(html);
    }

    function tip(id, validation) {
      return (
        '<div class="tip ' +
        (validation.failed ? "bad" : "good") +
        '" id="' +
        id +
        '">' +
        (validation.reason ? icon(validation.failed ? "xmark" : "check") + escape(validation.reason) : "") +
        "</div>"
      );
    }

    function setAria(input, validation) {
      if (validation.failed) {
        input.setAttribute("aria-invalid", "");
      } else {
        input.removeAttribute("aria-invalid");
      }
    }

    // emailValidation
    function emailValidation() {
      var email = emailInput.value;
      if (server.email === email && server.emailValidation) {
        return server.emailValidation;
      }
      if (!email) {
        return {
          failed: true,
          element: emailInput,
          reason: forceReason ? t("user.email.required") : null,
        };
      }
      if (rejectedEmails.indexOf(email) !== -1 || !emailValid(email)) {
        return { failed: true, element: emailInput, reason: t("user.email.invalid") };
      }
      if (auth && auth.email === email && auth.email_valid && auth.auth_provider) {
        var provider = (D.provider_names || {})[auth.auth_provider] || auth.auth_provider;
        return { ok: true, reason: t("user.email.authenticated", { provider: provider }) };
      }
      return { ok: true, reason: t("user.email.ok") };
    }

    // usernameValidation (UsernameValidationHelper)
    function usernameValidation() {
      var value = usernameInput.value;
      if (username.result && username.checked === value) {
        return username.result;
      }
      if (value && value === username.prefilled) {
        return { ok: true, reason: t("user.username.prefilled") };
      }
      if (!value) {
        return { failed: true, element: usernameInput, reason: forceReason ? t("user.username.required") : null };
      }
      if (value.length < D.settings.min_username_length) {
        return { failed: true, element: usernameInput, reason: t("user.username.too_short") };
      }
      if (value.length > D.settings.max_username_length) {
        return { failed: true, element: usernameInput, reason: t("user.username.too_long") };
      }
      clearTimeout(username.timer);
      username.timer = setTimeout(checkUsername, 500);
      return { failed: true, element: usernameInput, reason: t("user.username.checking") };
    }

    function checkUsername() {
      var value = usernameInput.value;
      request("GET", "/u/check_username.json", { username: value, email: emailInput.value }).then(function (r) {
        var result = r.body || {};
        username.checked = value;
        isDeveloper = !!result.is_developer;
        if (result.available) {
          username.result = { ok: true, reason: t("user.username.available") };
        } else if (result.suggestion) {
          username.result = {
            failed: true,
            element: usernameInput,
            reason: t("user.username.not_available", { suggestion: result.suggestion }),
          };
        } else {
          username.result = {
            failed: true,
            element: usernameInput,
            reason: result.errors ? result.errors.join(" ") : t("user.username.not_available_no_suggestion"),
          };
        }
        drawUsername();
        drawPassword();
      });
    }

    // fetchExistingUsername: a username suggested from the email.
    function fetchExistingUsername() {
      request("GET", "/u/check_username.json", { email: emailInput.value }).then(function (r) {
        var result = r.body || {};
        if (result.suggestion && (!usernameInput.value || (auth && usernameInput.value === auth.username))) {
          usernameInput.value = result.suggestion;
          username.prefilled = result.suggestion;
          valueEntered(usernameInput);
          drawUsername();
        }
      });
    }

    // nameValidation
    function nameValidation() {
      if (D.settings.full_name_required && nameInput && !nameInput.value) {
        return { failed: true, element: nameInput, reason: forceReason ? t("user.name.required") : null };
      }
      return { ok: true };
    }

    // passwordValidation (PasswordValidationHelper)
    function minPasswordLength() {
      return isDeveloper ? D.settings.min_admin_password_length : D.settings.min_password_length;
    }

    function passwordValidation() {
      if (!passwordRequired) {
        return { ok: true };
      }
      var password = passwordInput.value;
      if (rejectedPasswords[password] !== undefined) {
        return { failed: true, element: passwordInput, reason: rejectedPasswords[password] || t("user.password.common") };
      }
      if (!password) {
        return { failed: true, element: passwordInput, reason: forceReason ? t("user.password.required") : null };
      }
      if (password.length < minPasswordLength()) {
        return { failed: true, element: passwordInput, reason: t("user.password.too_short", { count: minPasswordLength() }) };
      }
      if (usernameInput.value && password === usernameInput.value) {
        return { failed: true, element: passwordInput, reason: t("user.password.same_as_username") };
      }
      if (nameInput && nameInput.value && password === nameInput.value) {
        return { failed: true, element: passwordInput, reason: t("user.password.same_as_name") };
      }
      if (emailInput.value && password === emailInput.value) {
        return { failed: true, element: passwordInput, reason: t("user.password.same_as_email") };
      }
      return { ok: true, reason: t("user.password.ok") };
    }

    function drawEmail() {
      var v = emailValidation();
      setAria(emailInput, v);
      var group = emailInput.parentNode;
      var old = group.querySelector("#account-email-validation, #account-email-validation-more-info");
      var show = v.ok || (v.reason && emailValidationVisible);
      var next = show
        ? el(tip("account-email-validation", v))
        : el('<span class="more-info" id="account-email-validation-more-info">' + escape(D.email_instructions || "") + "</span>");
      if (old) {
        old.replaceWith(next);
      }
      valueEntered(emailInput);
      emailInput.disabled = !!(auth && auth.email === emailInput.value && auth.email_valid);
    }

    function drawUsername() {
      var v = usernameValidation();
      setAria(usernameInput, v);
      var group = usernameInput.parentNode;
      var old = group.querySelector("#username-validation, #username-validation-more-info");
      var next =
        D.username_instructions && !v.reason
          ? el('<span class="more-info" id="username-validation-more-info">' + escape(D.username_instructions) + "</span>")
          : el(tip("username-validation", v));
      if (old) {
        old.replaceWith(next);
      }
      usernameInput.className = usernameInput.value ? "value-entered" : "";
    }

    function drawName() {
      if (!nameInput) {
        return;
      }
      var v = nameValidation();
      if (D.settings.full_name_required) {
        setAria(nameInput, v);
      }
      var group = nameInput.parentNode;
      var old = group.querySelector("#fullname-validation, #fullname-validation-more-info");
      var next =
        D.full_name_instructions && !v.reason
          ? el('<span class="more-info" id="fullname-validation-more-info">' + escape(D.full_name_instructions) + "</span>")
          : el(tip("fullname-validation", v));
      if (old) {
        old.replaceWith(next);
      }
      valueEntered(nameInput);
    }

    function drawPassword() {
      if (!passwordInput) {
        return;
      }
      var v = passwordValidation();
      setAria(passwordInput, v);
      var holder = passwordInput.parentNode.querySelector(".create-account__password-tip-validation");
      var old = holder.querySelector("#password-validation, #password-validation-more-info");
      if (old) {
        old.remove();
      }
      var next = null;
      if (v.ok || v.reason) {
        next = el(tip("password-validation", v));
      } else if (D.password_instructions) {
        next = el(
          '<span class="more-info" id="password-validation-more-info">' +
            escape(D.password_instructions.replace("%{count}", minPasswordLength())) +
            "</span>"
        );
      }
      if (next) {
        holder.insertBefore(next, holder.firstChild);
      }
      valueEntered(passwordInput);
    }

    function drawAll() {
      drawEmail();
      drawUsername();
      drawName();
      drawPassword();
    }

    // The server's instructions, as first drawn.
    var moreInfo = form.querySelector("#account-email-validation-more-info");
    D.email_instructions = moreInfo ? moreInfo.textContent : "";
    var usernameInfo = form.querySelector("#username-validation-more-info");
    D.username_instructions = usernameInfo ? usernameInfo.textContent : "";
    var nameInfo = form.querySelector("#fullname-validation-more-info");
    D.full_name_instructions = nameInfo ? nameInfo.textContent : "";
    var passwordInfo = form.querySelector("#password-validation-more-info");
    D.password_instructions = passwordInfo ? passwordInfo.textContent.replace(String(D.settings.min_password_length), "%{count}") : "";

    // checkEmailAvailability
    function checkEmail() {
      var v = emailValidation();
      emailValidationVisible = !!v.reason;
      drawEmail();
      if (!v.ok || server.email === emailInput.value) {
        return;
      }
      var email = emailInput.value;
      request("GET", "/u/check_email.json", { email: email })
        .then(function (r) {
          var result = r.body || {};
          server.email = email;
          if (!r.ok) {
            server.email = null;
            server.emailValidation = null;
          } else if (result.failed) {
            server.emailValidation = { failed: true, element: emailInput, reason: (result.errors || [])[0] };
          } else {
            server.emailValidation = { ok: true, reason: t("user.email.ok") };
          }
          drawEmail();
        })
        .catch(function () {
          server.email = null;
          server.emailValidation = null;
        });
    }

    // prefillUsername: a valid email looks for the username it implies.
    var prefillTimer = null;
    function prefillUsername() {
      if (username.prefilled) {
        if (usernameInput.value === username.prefilled) {
          usernameInput.value = "";
        }
        username.prefilled = null;
      }
      if (emailValidation().ok && (!usernameInput.value || (auth && auth.email))) {
        clearTimeout(prefillTimer);
        prefillTimer = setTimeout(fetchExistingUsername, 500);
      }
    }

    emailInput.addEventListener("input", function () {
      prefillUsername();
      drawEmail();
      drawUsername();
      drawPassword();
    });
    emailInput.addEventListener("focusout", checkEmail);
    usernameInput.addEventListener("input", function () {
      drawUsername();
      drawPassword();
    });
    if (nameInput) {
      nameInput.addEventListener("input", function () {
        drawName();
        drawPassword();
      });
    }
    if (passwordInput) {
      passwordInput.addEventListener("input", drawPassword);
      passwordField(passwordInput, passwordInput.parentNode.querySelector(".caps-lock-warning"));
      var mask = passwordInput.parentNode.querySelector(".toggle-password-mask");
      if (mask) {
        bindMask(mask, passwordInput);
      }
    }
    form.querySelectorAll("input").forEach(function (input) {
      input.addEventListener("focusin", function () {
        if (input.scrollIntoView) {
          input.scrollIntoView({ behavior: "smooth", block: "center" });
        }
      });
    });

    // fetchConfirmationValue: the honeypot and challenge.
    function fetchConfirmation() {
      if (hp.promise) {
        return hp.promise;
      }
      hp.promise = request("GET", "/session/hp.json")
        .then(function (r) {
          if (!r.ok) {
            return;
          }
          hp.challengeDate = new Date();
          hp.expiry = Math.max(parseInt(r.body.expires_in, 10) - 30, 30);
          honeypot.value = r.body.value;
          challenge.value = (r.body.challenge || "").split("").reverse().join("");
        })
        .finally(function () {
          hp.promise = null;
        });
      return hp.promise;
    }
    fetchConfirmation();

    function drawSubmit() {
      var html = dButton("btn-large btn-primary signup-page-cta__signup", t("create_account.title"), {
        loading: formSubmitted,
        disabled: formSubmitted,
      });
      var next = fromHtml(html);
      signupButton.replaceWith(next);
      signupButton = next;
      signupButton.addEventListener("click", createAccount);
      var login = cta.querySelector(".signup-page-cta__login");
      if (login) {
        if (formSubmitted) {
          login.setAttribute("disabled", "");
        } else {
          login.removeAttribute("disabled");
        }
      }
    }

    function flash(text) {
      setFlash(root, text ? escape(text) : null, null);
    }

    // performAccountCreation
    function performAccountCreation() {
      if (!hp.challengeDate || new Date() - hp.challengeDate > 1000 * hp.expiry) {
        return fetchConfirmation().then(performAccountCreation);
      }
      var data = {
        email: emailInput.value,
        password: passwordInput ? passwordInput.value : undefined,
        username: usernameInput.value,
        password_confirmation: honeypot.value,
        challenge: challenge.value,
        user_fields: undefined,
        timezone: timezone(),
      };
      if (nameInput) {
        data.name = nameInput.value;
      }
      if (inviteCode) {
        data.invite_code = inviteCode.value;
      }
      formSubmitted = true;
      drawSubmit();
      return applyBehaviorTransformer("create-account", function () {
        return request("POST", "/u", data);
      })
        .then(function (r) {
          var result = r.body || {};
          if (!r.ok) {
            throw r;
          }
          isDeveloper = false;
          if (result.success) {
            hp.expiry = 1;
            storage("discourse-authentication-data");
            var destination = auth && auth.destination_url;
            submitHiddenForm(
              usernameInput.value,
              passwordInput ? passwordInput.value : "",
              destination && destination !== "/signup" ? destination : base() + "/u/account-created"
            );
            return;
          }
          flash(result.message || t("create_account.failed"));
          if (result.is_developer) {
            isDeveloper = true;
          }
          var passwordErrors = (result.errors || {})["user_password.password"];
          if (passwordErrors && passwordErrors.length && passwordInput) {
            rejectedPasswords[passwordInput.value] = "";
          }
          formSubmitted = false;
          drawSubmit();
          drawAll();
          removeCookie("destination_url");
        })
        .catch(function () {
          formSubmitted = false;
          drawSubmit();
          removeCookie("destination_url");
          flash(t("create_account.failed"));
        });
    }

    // createAccount
    function createAccount() {
      if (formSubmitted) {
        return;
      }
      flash(null);
      forceReason = true;
      emailValidationVisible = true;
      drawAll();
      var failed = [emailValidation(), usernameValidation(), nameValidation(), passwordValidation()].filter(function (v) {
        return v.failed;
      })[0];
      if (failed) {
        if (failed.element) {
          failed.element.focus();
        }
        return;
      }
      forceReason = false;
      performAccountCreation();
    }

    signupButton.addEventListener("click", createAccount);
    form.addEventListener("submit", function (event) {
      event.preventDefault();
    });
    // actionOnEnter
    form.addEventListener("keydown", function (event) {
      if (event.key === "Enter" && !formSubmitted && event.target.tagName === "INPUT") {
        event.preventDefault();
        event.stopPropagation();
        createAccount();
      }
    });
    var goLogin = cta.querySelector(".signup-page-cta__login");
    if (goLogin) {
      goLogin.addEventListener("click", function () {
        // goToLogin: the email or username carries over.
        storage("discourse-login-name", emailInput.value || usernameInput.value || "");
        window.location = base() + "/login";
      });
    }
    if (emailInput.value) {
      prefillUsername();
    }
    if (auth || emailInput.value || usernameInput.value) {
      drawAll();
    }
    // The form drawn for an external login's data takes the autofocused
    // email field's focus away, and its focusout checks the address.
    if (auth) {
      checkEmail();
    }
  }

  // ---- Password reset -------------------------------------------------

  // PasswordResetController over templates/password-reset.gjs.
  function passwordResetPage() {
    var form = document.querySelector(".password-reset .change-password-form");
    if (!form) {
      return;
    }
    var model = D.password_reset || {};
    var token = D.token;
    // The route's afterModel: the token confirmed if it is a signup's,
    // here rather than when an email client crawls the link.
    request("GET", "/u/confirm-email-token/" + encodeURIComponent(token) + ".json");

    var state = {
      totp: !!model.second_factor_required,
      securityKey: !!model.security_key_required,
      backup: !!model.backup_enabled,
      method: model.security_key_required ? SECURITY_KEY : model.second_factor_required ? TOTP : model.backup_enabled ? BACKUP_CODE : TOTP,
      token: "",
      credential: null,
      isDeveloper: !!model.is_developer,
      errorMessage: null,
      loading: false,
      rejected: {},
    };

    function minLength() {
      return model.admin || state.isDeveloper ? D.settings.min_admin_password_length : D.settings.min_password_length;
    }

    function passwordValidation(password) {
      if (state.rejected[password] !== undefined) {
        return { failed: true, reason: state.rejected[password] || t("user.password.common") };
      }
      if (!password) {
        return { failed: true, reason: null };
      }
      if (password.length < minLength()) {
        return { failed: true, reason: t("user.password.too_short", { count: minLength() }) };
      }
      return { ok: true, reason: t("user.password.ok") };
    }

    function errorHtml() {
      return state.errorMessage
        ? '<div class="alert alert-error">' + escape(state.errorMessage) + "</div><br>"
        : "";
    }

    function submitButton(label) {
      return dButton("btn-primary", label, { type: "submit", loading: state.loading });
    }

    // The template's two branches: the second factor first, then the new
    // password.
    function draw() {
      if (state.totp || state.securityKey) {
        var securityForm = state.securityKey && state.method === SECURITY_KEY;
        var html =
          "<h2>" +
          escape(t("user.change_password.title")) +
          "</h2><p>" +
          escape(t("user.change_password.verify_identity")) +
          "</p>" +
          errorHtml();
        if (securityForm) {
          html +=
            '<div id="security-key">' +
            dButton("btn-large btn-primary", t("login.security_key_authenticate"), {
              icon: "key",
              attrs: ' id="security-key-authenticate-button"',
            }) +
            "<p>" +
            (model.multiple_second_factor_methods
              ? '<a class="toggle-second-factor-method" href="">' + escape(t("login.security_key_alternative")) + "</a>"
              : "") +
            "</p></div>";
        } else {
          var method = state.method === SECURITY_KEY ? TOTP : state.method;
          var input =
            method === TOTP
              ? otpHtml({ className: "second-factor-token-input", id: "second-factor" })
              : '<input autocapitalize="off" autocorrect="off" autofocus="" class="second-factor-token-input" maxlength="32" pattern="[a-z0-9]{16}" type="text" id="second-factor" value="">';
          html +=
            '<div id="second-factor"><h3>' +
            escape(t(method === BACKUP_CODE ? "login.second_factor_backup_title" : "login.second_factor_title")) +
            '</h3><p class="second-factor__description">' +
            escape(t(method === BACKUP_CODE ? "login.second_factor_backup_description" : "login.second_factor_description")) +
            "</p>" +
            input +
            (state.backup && state.totp
              ? '<p><a class="toggle-second-factor-method" href="">' +
                escape(t(method === TOTP ? "user.second_factor_backup.use" : "user.second_factor.use")) +
                "</a></p>"
              : "") +
            "</div>" +
            submitButton(t("submit"));
        }
        form.innerHTML = html;
        if (securityForm) {
          form.querySelector("#security-key-authenticate-button").addEventListener("click", authenticateSecurityKey);
          var other = form.querySelector("#security-key .toggle-second-factor-method");
          if (other) {
            other.addEventListener("click", function (event) {
              event.preventDefault();
              state.method = state.totp ? TOTP : BACKUP_CODE;
              draw();
            });
          }
        } else {
          var tokenInput = form.querySelector(".second-factor-token-input");
          if (tokenInput.classList.contains("d-otp-input")) {
            bindOtp(form.querySelector("#second-factor"));
          } else {
            tokenInput.focus();
          }
          tokenInput.addEventListener("input", function () {
            state.token = tokenInput.value;
          });
          var toggle = form.querySelector("#second-factor > p > .toggle-second-factor-method");
          if (toggle) {
            toggle.addEventListener("click", function (event) {
              event.preventDefault();
              state.token = "";
              state.method = state.method === TOTP ? BACKUP_CODE : TOTP;
              draw();
            });
          }
        }
        return;
      }
      var password = form.querySelector("#new-account-password");
      var value = password ? password.value : "";
      form.innerHTML =
        "<h2>" +
        escape(t("user.change_password.choose_new")) +
        "</h2>" +
        errorHtml() +
        '<div class="input"><input placeholder="" autocomplete="new-password" autofocus="autofocus" id="new-account-password" class="ember-text-field ember-view change-password-form__password" type="password"><div class="change-password__password-info"><div class="change-password_tip-validation"><div class="caps-lock-warning hidden">' +
        icon("triangle-exclamation") +
        escape(t("login.caps_lock_warning")) +
        "</div></div>" +
        maskButton(true) +
        "</div></div>" +
        submitButton(t("user.change_password.set_password"));
      bindPassword(value);
    }

    function bindPassword(value) {
      var password = form.querySelector("#new-account-password");
      password.value = value || "";
      var holder = form.querySelector(".change-password_tip-validation");
      function tip() {
        var old = holder.querySelector(".tip");
        if (old) {
          old.remove();
        }
        var v = passwordValidation(password.value);
        if (v.ok || v.reason) {
          holder.insertBefore(
            fromHtml(
              '<div class="tip ' +
                (v.failed ? "bad" : "good") +
                '">' +
                (v.reason ? icon(v.failed ? "xmark" : "check") + escape(v.reason) : "") +
                "</div>"
            ),
            holder.firstChild
          );
        }
      }
      password.addEventListener("input", tip);
      tip();
      passwordField(password, holder.querySelector(".caps-lock-warning"));
      bindMask(form.querySelector(".toggle-password-mask"), password);
      password.focus();
    }

    function authenticateSecurityKey() {
      state.method = SECURITY_KEY;
      if (typeof PublicKeyCredential === "undefined") {
        state.errorMessage = t("login.security_key_support_missing_error");
        draw();
        return;
      }
      var allowed = model.allowed_credential_ids || [];
      var toBuffer = function (s) {
        return Uint8Array.from(s, function (c) {
          return c.charCodeAt(0);
        });
      };
      var toBase64 = function (b) {
        return btoa(String.fromCharCode.apply(null, new Uint8Array(b)));
      };
      navigator.credentials
        .get({
          publicKey: {
            challenge: toBuffer(model.challenge || ""),
            allowCredentials: allowed.map(function (id) {
              return { id: toBuffer(atob(id)), type: "public-key" };
            }),
            timeout: 60000,
            userVerification: "discouraged",
          },
        })
        .then(function (credential) {
          state.credential = {
            signature: toBase64(credential.response.signature),
            clientData: toBase64(credential.response.clientDataJSON),
            authenticatorData: toBase64(credential.response.authenticatorData),
            credentialId: toBase64(credential.rawId),
          };
          submit();
        })
        .catch(function (err) {
          state.securityKey = true;
          state.errorMessage = err.name === "NotAllowedError" ? t("login.security_key_not_allowed_error") : String(err.message || err);
          draw();
        });
    }

    // PasswordResetController#submit
    function submit() {
      if (state.loading) {
        return;
      }
      var password = form.querySelector("#new-account-password");
      var accountPassword = password ? password.value : state.password;
      if (password) {
        state.password = password.value;
      }
      state.loading = true;
      var button = form.querySelector("button[type=submit]");
      if (button) {
        button.replaceWith(fromHtml(dButton("btn-primary", button.textContent, { type: "submit", loading: true })));
      }
      request("PUT", "/u/password-reset/" + encodeURIComponent(token) + ".json", {
        password: accountPassword,
        second_factor_token: state.credential || state.token,
        second_factor_method: state.method,
        timezone: timezone(),
      })
        .then(function (r) {
          state.loading = false;
          if (r.status === 429) {
            state.errorMessage = t("user.second_factor.rate_limit");
            draw();
            return;
          }
          if (!r.ok) {
            popupAjaxError(r);
            draw();
            return;
          }
          var result = r.body;
          if (result.success) {
            var html = "<p>" + escape(result.message) + "</p>";
            if (result.requires_approval) {
              html += "<p>" + escape(t("login.not_approved")) + "</p>";
            }
            form.innerHTML = html;
            if (!result.requires_approval) {
              window.location = result.redirect_to || base() + "/";
            }
            return;
          }
          var errors = result.errors || {};
          if (errors.security_keys || errors.user_second_factors) {
            state.credential = null;
          } else if (state.totp || state.securityKey) {
            state.totp = false;
            state.securityKey = false;
            state.errorMessage = null;
          } else if ((errors["user_password.password"] || []).length) {
            state.rejected[accountPassword] = (result.friendly_messages || []).join("\n");
          }
          if (result.message) {
            state.errorMessage = result.message;
          }
          draw();
          var input = form.querySelector("#new-account-password");
          if (input && state.password) {
            input.value = state.password;
            input.dispatchEvent(new Event("input"));
          }
        })
        .catch(function () {
          state.loading = false;
          draw();
        });
    }

    form.addEventListener("submit", function (event) {
      event.preventDefault();
      submit();
    });
    if (form.querySelector("#new-account-password")) {
      bindPassword("");
    } else {
      draw();
    }
  }

  // ---- The login-required front page, login-preferences --------------

  function staticPages() {
    var signup = document.querySelector(".body-page-button-container .sign-up-button");
    if (signup) {
      signup.addEventListener("click", function () {
        window.location = base() + "/signup";
      });
    }
    var login = document.querySelector(".body-page-button-container .login-button");
    if (login) {
      login.addEventListener("click", function () {
        window.location = base() + "/login";
      });
    }
    // account-created's ActivationControls, whose routes wait on
    // users#send_activation_email and users#update_activation_email.
    var resend = document.querySelector(".account-created .activation-controls .resend");
    if (resend) {
      resend.addEventListener("click", function () {
        refuse("resending the activation email (users#send_activation_email)");
      });
    }
    var editEmail = document.querySelector(".account-created .activation-controls .edit-email");
    if (editEmail) {
      editEmail.addEventListener("click", function () {
        refuse("changing the activation email (account-created.edit-email)");
      });
    }
    if (D.page === "login-preferences") {
      var button = document.querySelector("#main-outlet > .container .btn-primary");
      if (button) {
        button.addEventListener("click", function () {
          window.location = base() + "/login";
        });
      }
    }
  }

  function init() {
    var el = document.getElementById("login-data");
    if (!el || el.dataset.ready) {
      return;
    }
    el.dataset.ready = "true";
    try {
      D = JSON.parse(el.textContent);
    } catch (_) {
      return;
    }
    S = D.strings || {};
    if (D.page === "login") {
      keepDestination();
      // A route that sent its visitor here (new-topic's showLogin, the
      // forgot-password route) ends on the login route's URL.
      if (location.pathname !== base() + "/login") {
        history.replaceState(history.state, "", base() + "/login");
      }
      loginPage();
    } else if (D.page === "signup") {
      keepDestination();
      signupPage();
    } else if (D.page === "password-reset") {
      passwordResetPage();
    } else if (D.page === "forgot-password") {
      // The forgot-password route over the homepage: it ends on the URL
      // it replaced itself with.
      if (location.pathname + location.search !== D.url) {
        history.replaceState(history.state, "", D.url);
      }
    } else {
      staticPages();
    }
    if (D.forgot_password) {
      // modal.js is deferred: on a page load it comes after this script.
      if (Discourse.modal || document.readyState !== "loading") {
        forgotPassword("");
      } else {
        document.addEventListener("DOMContentLoaded", function () {
          forgotPassword("");
        });
      }
    }
  }

  Discourse.login = { forgotPassword: forgotPassword };

  if (Discourse.onPage) {
    Discourse.onPage(init);
  } else {
    init();
  }
})();
