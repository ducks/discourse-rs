// instance-initializers/auth-complete.js: what an external login's
// callback left in the authentication_data cookie, read once on the page
// the callback sent the browser to. A login that failed for a reason
// goes to the login page with that reason; one that logged on goes to
// where the reader was headed; a new account goes to the signup page
// with what the provider told about it (kept for the page in
// sessionStorage, as the cookie is read once).
(function () {
  "use strict";

  var script = document.currentScript;

  function base() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return (meta && meta.content) || "";
  }

  function readCookie(name) {
    var match = document.cookie.match(new RegExp("(?:^|; )" + name + "=([^;]*)"));
    if (!match) {
      return null;
    }
    try {
      return decodeURIComponent(match[1].replace(/\+/g, " "));
    } catch (e) {
      return null;
    }
  }

  function removeCookie(name, path) {
    document.cookie = name + "=; path=" + path + "; max-age=0";
  }

  function store(key, value) {
    try {
      sessionStorage.setItem(key, value);
    } catch (e) {
      // Private windows may refuse storage; the page then shows nothing.
    }
  }

  var raw = readCookie("authentication_data");
  if (!raw) {
    return;
  }
  removeCookie("authentication_data", base() + "/");
  // ApplicationHelper#authentication_data: read only for an anonymous
  // page; a logged in one just drops it.
  if (document.querySelector('meta[name="csrf-token"]')) {
    return;
  }
  var options;
  try {
    options = JSON.parse(raw);
  } catch (e) {
    return;
  }

  var labels = (script && script.dataset) || {};
  var errors = [
    ["admin_not_allowed_from_ip_address", labels.adminNotAllowed],
    ["awaiting_activation", labels.awaitingActivation],
    ["awaiting_approval", labels.awaitingApproval],
    ["not_allowed_from_ip_address", labels.notAllowed],
    ["requires_invite", labels.requiresInvite],
  ];
  for (var i = 0; i < errors.length; i++) {
    if (options[errors[i][0]]) {
      store("discourse-login-flash", errors[i][1] || errors[i][0]);
      window.location = base() + "/login";
      return;
    }
  }
  if (options.suspended) {
    store("discourse-login-flash", options.suspended_message || "");
    window.location = base() + "/login";
    return;
  }
  if (options.omniauth_disallow_totp) {
    store("discourse-login-flash", labels.disallowTotp || "");
    store("discourse-login-name", options.email || "");
    window.location = base() + "/login";
    return;
  }
  if (options.authenticated) {
    var destination = readCookie("destination_url") || options.destination_url;
    if (destination) {
      removeCookie("destination_url", "/");
      window.location.href = destination;
    } else if (window.location.pathname === base() + "/login") {
      window.location = base() + "/";
    } else {
      window.location.reload();
    }
    return;
  }
  // A new account: the signup page, filled in from the provider.
  store("discourse-authentication-data", raw);
  window.location = base() + "/signup";
})();
