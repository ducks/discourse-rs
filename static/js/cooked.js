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

  new MutationObserver(function (mutations) {
    mutations.forEach(function (m) {
      m.addedNodes.forEach(decorateAll);
    });
  }).observe(document.documentElement, { childList: true, subtree: true });
})();
