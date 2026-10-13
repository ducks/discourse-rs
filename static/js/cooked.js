// The one place cooked content is decorated, as Ember's
// decorateCookedElement (instance-initializers/post-decorations.js) runs
// its decorators on each post as it renders:
//
//   Discourse.decorateCooked(name, fn)
//       fn(el) for each .cooked element on the page now and for each one
//       that arrives later (a page swapped in, a post over the live
//       stream, a revision), once per element and decorator. A redrawn
//       post is a new element, so it is decorated again.
//
// Decorators run in the order they register.
//
//   Discourse.decorateCooked.apply(el)
//       runs every decorator on el, which need not be a .cooked element:
//       the content of an expanded quote, as Ember's PostCookedHtml
//       decorates it.
//   Discourse.cooked.setting(name), Discourse.cooked.t(key),
//   Discourse.cooked.icon(name, extraClass)
//       what decorators read and draw: the siteSettings that gate them and
//       their js.* strings, from the page's #cooked-config, and iconHTML.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.decorateCooked) {
    return;
  }

  var decorators = [];

  function apply(el, decorator) {
    if (decorator.done.has(el)) {
      return;
    }
    decorator.done.add(el);
    try {
      decorator.fn(el);
    } catch (e) {
      // One decorator failing leaves the others, as Ember's do.
      window.console && console.error("decorateCooked " + decorator.name, e);
    }
  }

  function cookedIn(root) {
    if (root.nodeType !== 1) {
      return [];
    }
    var found = Array.prototype.slice.call(root.querySelectorAll(".cooked"));
    if (root.classList.contains("cooked")) {
      found.unshift(root);
    }
    return found;
  }

  function decorateAll(root) {
    cookedIn(root).forEach(function (el) {
      decorators.forEach(function (decorator) {
        apply(el, decorator);
      });
    });
  }

  Discourse.decorateCooked = function (name, fn) {
    var decorator = { name: name, fn: fn, done: new WeakSet() };
    decorators.push(decorator);
    cookedIn(document.documentElement).forEach(function (el) {
      apply(el, decorator);
    });
  };

  Discourse.decorateCooked.apply = function (el) {
    decorators.forEach(function (decorator) {
      apply(el, decorator);
    });
  };

  var config;
  function cookedConfig() {
    if (!config) {
      var el = document.getElementById("cooked-config");
      try {
        config = el ? JSON.parse(el.textContent) : {};
      } catch (e) {
        window.console && console.error("cooked-config", e);
        config = {};
      }
      config.settings = config.settings || {};
      config.strings = config.strings || {};
    }
    return config;
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  Discourse.cooked = {
    // The settings the decorators check all default to true, so a page
    // without the config gets the defaults.
    setting: function (name) {
      return cookedConfig().settings[name] !== false;
    },
    t: function (key) {
      var text = cookedConfig().strings[key];
      return text === undefined ? key : text;
    },
    // iconHTML
    icon: function (name, extraClass) {
      return (
        '<svg class="fa d-icon d-icon-' + escapeHtml(name) + " svg-icon fa-width-auto" +
        (extraClass ? " " + escapeHtml(extraClass) : "") +
        ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
        escapeHtml(name) + '"></use></svg>'
      );
    },
  };

  new MutationObserver(function (mutations) {
    mutations.forEach(function (m) {
      m.addedNodes.forEach(decorateAll);
    });
  }).observe(document.documentElement, { childList: true, subtree: true });
})();
