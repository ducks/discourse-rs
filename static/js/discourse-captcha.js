// discourse-captcha's client: the h-captcha / re-captcha components in
// the create-account form's captcha-fields-connector, the captcha
// service's token and state, and the "create-account" behavior
// transformer that hands the token to /captcha/:provider/create before
// the account is created (pre-initializers/h-captcha.js).
//
// The connector (src/plugins/captcha.rs) carries the provider, its site
// key and the strings; the provider's script is loaded on first sight of
// it, as BaseCaptcha's DAsyncContent does.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.captchaLoaded) {
    return;
  }
  Discourse.captchaLoaded = true;

  var PROVIDERS = {
    hcaptcha: {
      scriptUrl: "https://js.hcaptcha.com/1/api.js?onload=discourseHCaptchaCallback&render=explicit",
      callbackName: "discourseHCaptchaCallback",
      apiName: "hcaptcha",
      containerId: "h-captcha-field",
    },
    recaptcha: {
      scriptUrl: "https://www.google.com/recaptcha/api.js?onload=discourseReCaptchaCallback&render=explicit",
      callbackName: "discourseReCaptchaCallback",
      apiName: "grecaptcha",
      containerId: "g-recaptcha",
    },
  };

  // The captcha service.
  var service = { invalid: true, submitted: false, token: null, widgetId: null, api: null, connector: null };

  function strings(connector) {
    try {
      return JSON.parse(connector.getAttribute("data-strings") || "{}");
    } catch (_) {
      return {};
    }
  }

  // DInputTip under the widget once a submit found no token.
  function drawTip() {
    var connector = service.connector;
    if (!connector) {
      return;
    }
    var group = connector.querySelector(".input-group");
    var old = group.querySelector(".captcha-service-tip");
    if (old) {
      old.remove();
    }
    if (!(service.submitted && service.invalid)) {
      return;
    }
    var tip = document.createElement("div");
    tip.className = "tip bad captcha-service-tip";
    tip.textContent = strings(connector).missing_token || "";
    group.appendChild(tip);
  }

  function reset() {
    service.invalid = true;
    service.submitted = false;
    service.token = null;
    if (service.api && service.widgetId !== null) {
      service.api.reset(service.widgetId);
    }
    drawTip();
  }

  // loadCaptchaScript: the api when the page has it, else the provider's
  // script with its onload callback.
  function loadScript(provider) {
    if (window[provider.apiName]) {
      return Promise.resolve(window[provider.apiName]);
    }
    return new Promise(function (resolve, reject) {
      window[provider.callbackName] = function () {
        resolve(window[provider.apiName]);
      };
      var script = document.createElement("script");
      script.src = provider.scriptUrl;
      script.async = true;
      script.onerror = reject;
      document.head.appendChild(script);
    });
  }

  function render(connector) {
    if (connector.getAttribute("data-rendered")) {
      return;
    }
    connector.setAttribute("data-rendered", "true");
    service.connector = connector;
    var name = connector.getAttribute("data-provider");
    var siteKey = connector.getAttribute("data-site-key");
    var provider = PROVIDERS[name];
    var group = connector.querySelector(".input-group");
    if (!provider) {
      return;
    }
    loadScript(provider).then(
      function (api) {
        group.innerHTML = "";
        var container = document.createElement("div");
        container.className = "captcha-container";
        container.setAttribute("data-sitekey", siteKey || "");
        container.id = provider.containerId;
        group.appendChild(container);
        if (!api || !siteKey) {
          return;
        }
        var options = {
          sitekey: siteKey,
          callback: function (response) {
            service.token = response;
            service.invalid = !response;
            drawTip();
          },
          "expired-callback": function () {
            service.invalid = true;
            drawTip();
          },
        };
        if (name === "hcaptcha") {
          options["error-callback"] = function (error) {
            // eslint-disable-next-line no-console
            console.error("hCaptcha error:", error);
            service.invalid = true;
            drawTip();
          };
        }
        service.api = api;
        service.widgetId = api.render(container, options);
      },
      function () {
        group.innerHTML = "";
        var alert = document.createElement("div");
        alert.className = "alert alert-error";
        alert.textContent = strings(connector).contact_system_administrator || "";
        group.appendChild(alert);
      }
    );
  }

  function scan() {
    document.querySelectorAll(".captcha-fields-connector[data-provider]").forEach(render);
  }

  // The form an external login's data needs is drawn later, from a
  // template.
  new MutationObserver(scan).observe(document.documentElement, { childList: true, subtree: true });
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", scan);
  } else {
    scan();
  }

  var transformers = (Discourse.behaviorTransformers = Discourse.behaviorTransformers || {});
  (transformers["create-account"] = transformers["create-account"] || []).push(function (ctx) {
    var connector = service.connector || document.querySelector(".captcha-fields-connector[data-provider]");
    if (!connector) {
      return ctx.next();
    }
    var s = strings(connector);
    service.connector = connector;
    service.submitted = true;
    drawTip();
    if (service.invalid) {
      return { ok: true, status: 200, body: { success: false, message: s.missing_token } };
    }
    var route = connector.getAttribute("data-provider");
    return ctx.request("POST", "/captcha/" + route + "/create.json", { token: service.token }).then(function (r) {
      if (!r.ok) {
        reset();
        return { ok: true, status: 200, body: { success: false, message: s.verification_failed } };
      }
      return Promise.resolve(ctx.next()).finally(reset);
    });
  });
})();
