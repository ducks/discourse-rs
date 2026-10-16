// What the advanced and customize admin pages' scripts share
// (src/admin_ui.rs draws their markup):
//
//   Discourse.adminUi.t(key, args)      the page's strings (a JSON block
//                                       read by strings(id)), `%{name}`
//                                       filled from args
//   Discourse.adminUi.strings(id)       loads the strings block with that id
//   Discourse.adminUi.ajax(method, path, data)
//                                       a request as discourse/lib/ajax
//                                       makes it, resolving with the JSON;
//                                       data a plain object (form encoded)
//                                       or FormData, or { json: object }
//   Discourse.adminUi.failed(error)     popupAjaxError
//   Discourse.adminUi.unsupported(what) the page error for what is not
//                                       ported, saying so
//   Discourse.adminUi.icon(name)        dIcon's svg
//   Discourse.adminUi.menus(root)       DMenu triggers (their content in
//                                       the template after them) opening
//                                       their menu on click
//   Discourse.adminUi.subscribe(channel, fn)
//                                       the message bus's channel, from the
//                                       page's position; returns a stop()
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.adminUi) {
    return;
  }

  var strings = {};

  function loadStrings(id) {
    var el = document.getElementById(id);
    try {
      var own = el ? JSON.parse(el.textContent) : {};
      Object.keys(own).forEach(function (k) {
        strings[k] = own[k];
      });
    } catch (_) {
      // A broken block leaves the keys showing.
    }
  }

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
    if (data instanceof FormData) {
      options.body = data;
    } else if (data && data.json) {
      options.headers["Content-Type"] = "application/json";
      options.body = JSON.stringify(data.json);
    } else if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = encode(data);
    }
    return fetch(base() + path, options).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (_) {
          // Not JSON: the status decides.
        }
        if (!response.ok || (json && json.failed === "FAILED")) {
          var message =
            (json && json.errors && [].concat(json.errors).join(" ")) ||
            (json && json.message) ||
            (json && json.error) ||
            t("generic_error");
          var error = new Error(message);
          error.status = response.status;
          error.json = json;
          throw error;
        }
        return json;
      });
    });
  }

  // jQuery's param: nested objects and arrays as Rails reads them.
  function encode(data) {
    var parts = [];
    function add(key, value) {
      if (Array.isArray(value)) {
        value.forEach(function (v) {
          add(key + "[]", v);
        });
      } else if (value !== null && typeof value === "object") {
        Object.keys(value).forEach(function (k) {
          add(key + "[" + k + "]", value[k]);
        });
      } else {
        parts.push(
          encodeURIComponent(key) + "=" + encodeURIComponent(value === null || value === undefined ? "" : value)
        );
      }
    }
    Object.keys(data).forEach(function (k) {
      add(k, data[k]);
    });
    return parts.join("&");
  }

  function failed(error) {
    Discourse.dialog.alert(String((error && error.message) || error));
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
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

  // DMenu: the trigger's content is the template beside it; its buttons
  // are handled by the page's own click listener on the menu content.
  function menus(onOpen) {
    document.addEventListener("click", function (event) {
      var trigger = event.target.closest(".fk-d-menu__trigger[data-identifier]");
      if (!trigger || !trigger.closest(".admin-contents") || !Discourse.float) {
        return;
      }
      var template = trigger.nextElementSibling;
      if (!template || template.tagName !== "TEMPLATE" || !template.classList.contains("d-menu-content")) {
        return;
      }
      // DMenu's trigger toggles its menu, expanded while it is open.
      if (trigger.getAttribute("aria-expanded") === "true") {
        Discourse.float.close(trigger.dataset.identifier);
        return;
      }
      var content = template.content.firstElementChild.cloneNode(true);
      trigger.setAttribute("aria-expanded", "true");
      trigger.classList.add("-expanded");
      var menu = Discourse.float.menu(trigger, {
        identifier: trigger.dataset.identifier,
        mainClass: trigger.dataset.identifier + "-content",
        content: content,
        onClose: function () {
          trigger.setAttribute("aria-expanded", "false");
          trigger.classList.remove("-expanded");
        },
      });
      if (onOpen) {
        onOpen(trigger, content, menu);
      }
    });
  }

  function subscribe(channel, fn) {
    if (!window.EventSource) {
      return function () {};
    }
    var meta = document.querySelector("meta[name='bus-position']");
    var url =
      base() +
      "/bus/events?channels=" +
      encodeURIComponent(channel) +
      (meta && meta.content ? "&position=" + encodeURIComponent(meta.content) : "");
    var source = new EventSource(url, { withCredentials: true });
    source.addEventListener("message", function (event) {
      try {
        fn(JSON.parse(event.data).data);
      } catch (_) {
        // A message the page can't read is skipped.
      }
    });
    return function () {
      source.close();
    };
  }

  Discourse.adminUi = {
    t: t,
    strings: loadStrings,
    ajax: ajax,
    base: base,
    failed: failed,
    unsupported: unsupported,
    icon: icon,
    menus: menus,
    subscribe: subscribe,
  };
})();
