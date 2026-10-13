// Float-kit (frontend/discourse/float-kit): how Discourse places a menu or
// a tooltip by its trigger, on @floating-ui/dom as Ember does (vendored,
// static/vendor/README, loaded before this file as window.FloatingUIDOM).
// Placement, flip, shift, arrow and the defaults are float-kit's own:
//
//   Discourse.float.position(trigger, content, options) -> Promise
//       updatePosition (lib/update-position.ts): places content by trigger
//       once, setting its left/top, data-placement and data-strategy.
//   Discourse.float.attach(trigger, content, options) -> stop()
//       the apply-floating-ui modifier: places content now and again as
//       the page scrolls or resizes (options.autoUpdate), until stop().
//   Discourse.float.menu(trigger, options) -> instance
//       the menu service's show(trigger, options): a DMenu content
//       (div.fk-d-menu) in #d-menu-portals, closed on a click outside or
//       Escape. options.content is an element or HTML.
//   Discourse.float.tooltip(trigger, options) -> instance
//       a DTooltip: wraps trigger in its span.fk-d-tooltip__trigger and
//       opens its content (div.fk-d-tooltip__content in #d-tooltip-portals)
//       on hover or click as DTooltip's listeners do.
//   Discourse.float.close(identifier)
//       closes the open float with that identifier.
//
// An instance has show(), close(options) and destroy(), and while open
// its content element as instance.content. Only one float per identifier
// is open at a time, as the services keep it.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.float) {
    return;
  }

  // lib/constants.ts
  var PLACEMENTS = [
    "top",
    "top-start",
    "top-end",
    "right",
    "right-start",
    "right-end",
    "bottom",
    "bottom-start",
    "bottom-end",
    "left",
    "left-start",
    "left-end",
  ];

  var TOOLTIP = {
    animated: true,
    ariaLabel: null,
    arrow: true,
    closeOnClickOutside: true,
    closeOnEscape: true,
    closeOnScroll: true,
    content: null,
    identifier: null,
    inline: null,
    interactive: false,
    maxWidth: 350,
    offset: 10,
    triggers: { mobile: ["click"], desktop: ["hover", "click"] },
    untriggers: { mobile: ["click"], desktop: ["hover", "click"] },
    placement: "top",
    shiftBeforeVisibilityOptimizer: false,
    visibilityOptimizer: "flip",
    fallbackPlacements: PLACEMENTS,
    autoUpdate: true,
    trapTab: true,
    onClose: null,
    onShow: null,
  };

  var MENU = {
    animated: true,
    ariaLabel: null,
    arrow: false,
    autofocus: false,
    closeOnEscape: true,
    closeOnClickOutside: true,
    closeOnScroll: false,
    content: null,
    identifier: null,
    interactive: true,
    maxWidth: 400,
    offset: 10,
    placement: "bottom-start",
    shiftBeforeVisibilityOptimizer: false,
    visibilityOptimizer: "flip",
    fallbackPlacements: PLACEMENTS,
    autoUpdate: true,
    trapTab: true,
    contentRole: "dialog",
    onClose: null,
    onShow: null,
    inline: null,
  };

  function merge(defaults, options) {
    var merged = {};
    Object.keys(defaults).forEach(function (key) {
      merged[key] = defaults[key];
    });
    Object.keys(options || {}).forEach(function (key) {
      if (options[key] !== undefined) {
        merged[key] = options[key];
      }
    });
    return merged;
  }

  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  // lib/offset-calculator.js headerOffset(): the --header-offset Ember's
  // site header keeps on <html>, the bottom of .d-header-wrap. Read the
  // header itself when nothing has set it.
  function headerOffset() {
    var set = parseInt(document.documentElement.style.getPropertyValue("--header-offset"), 10);
    if (set) {
      return set;
    }
    var wrap = document.querySelector(".d-header-wrap");
    return wrap ? Math.max(0, Math.floor(wrap.getBoundingClientRect().bottom)) : 0;
  }

  function padding(options) {
    return options.padding || { top: headerOffset(), left: 10, right: 10, bottom: 10 };
  }

  // horizontalViewportInset()
  function horizontalInset(options) {
    var p = padding(options);
    return typeof p === "number" ? p * 2 : (p.left || 0) + (p.right || 0);
  }

  function icon(name, cls) {
    return (
      '<svg class="fa d-icon d-icon-' + name + " svg-icon " + (cls ? cls + " " : "") +
      'fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" ' +
      'xmlns="http://www.w3.org/2000/svg"><use href="#' + name + '"></use></svg>'
    );
  }

  function ensureArrow(content) {
    var arrow = content.querySelector(".arrow");
    if (!arrow) {
      var holder = document.createElement("div");
      holder.innerHTML = icon("tippy-rounded-arrow", "arrow");
      arrow = holder.firstChild;
      content.appendChild(arrow);
    }
    return arrow;
  }

  // buildMiddleware()
  function middleware(ui, options, content) {
    var p = padding(options);
    var overflow = { padding: p };
    if (options.boundary) {
      overflow.boundary = options.boundary;
    }
    var list = [];
    var arrow = null;

    if (options.placement === "center") {
      list.push(
        ui.offset(function (state) {
          return -state.rects.reference.height / 2 - state.rects.floating.height / 2;
        })
      );
    } else {
      list.push(ui.offset(options.offset == null ? 10 : options.offset));
      if (options.inline) {
        list.push(ui.inline());
      }
      var shift = ui.shift({
        padding: p,
        limiter: options.limitShift ? ui.limitShift(options.limitShift) : undefined,
        crossAxis: options.crossAxisShift == null ? true : options.crossAxisShift,
      });
      var optimizer = null;
      var kind = options.visibilityOptimizer || "flip";
      if (kind === "autoPlacement") {
        optimizer = ui.autoPlacement(
          Object.assign({ allowedPlacements: options.allowedPlacements || PLACEMENTS }, overflow)
        );
      } else if (kind !== "none") {
        optimizer = ui.flip(
          Object.assign({ fallbackPlacements: options.fallbackPlacements || PLACEMENTS }, overflow)
        );
      }
      if (!optimizer) {
        list.push(shift);
      } else if (options.shiftBeforeVisibilityOptimizer) {
        list.push(shift, optimizer);
      } else {
        list.push(optimizer, shift);
      }
    }

    if (options.arrow) {
      arrow = ensureArrow(content);
      list.push(ui.arrow({ element: arrow }));
    }
    if (options.hide) {
      list.push(ui.hide({ padding: p }));
    }
    if (options.matchTriggerWidth || options.matchTriggerMinWidth) {
      list.push(
        ui.size({
          apply: function (state) {
            var width = state.rects.reference.width + "px";
            if (options.matchTriggerWidth) {
              state.elements.floating.style.width = width;
            }
            if (options.matchTriggerMinWidth) {
              state.elements.floating.style.minWidth = width;
            }
          },
        })
      );
    }
    if (options.constrainHeightToViewport) {
      list.push(
        ui.size(
          Object.assign(
            {
              apply: function (state) {
                var max = Math.max(50, (state.availableHeight || 0) - 4);
                var floating = state.elements.floating;
                var inner = floating.querySelector(".fk-d-menu__inner-content");
                var hidden = options.minHeight && state.availableHeight < options.minHeight;
                if (inner) {
                  inner.style.display = hidden ? "none" : "";
                  inner.style.maxHeight = max + "px";
                } else {
                  floating.style.display = hidden ? "none" : "";
                  floating.style.maxHeight = max + "px";
                  floating.style.overflow = "auto";
                }
              },
            },
            overflow
          )
        )
      );
    }
    return { list: list, arrow: arrow };
  }

  function position(trigger, content, options) {
    options = options || {};
    var ui = window.FloatingUIDOM;
    var built = middleware(ui, options, content);
    var strategy = options.strategy || "absolute";
    content.dataset.strategy = strategy;
    return ui
      .computePosition(trigger, content, {
        placement: options.placement === "center" ? "bottom" : options.placement || "bottom",
        strategy: strategy,
        middleware: built.list,
      })
      .then(function (result) {
        if (options.computePosition) {
          options.computePosition(content, Object.assign({ arrowElement: built.arrow }, result));
        } else {
          // applyComputedPosition()
          content.dataset.placement = result.placement;
          content.style.left = result.x + "px";
          content.style.top = result.y + "px";
          content.style.visibility =
            result.middlewareData.hide && result.middlewareData.hide.referenceHidden ? "hidden" : "visible";
          if (result.middlewareData.arrow && built.arrow) {
            var a = result.middlewareData.arrow;
            built.arrow.style.left = a.x != null ? a.x + "px" : "";
            built.arrow.style.top = a.y != null ? a.y + "px" : "";
          }
        }
        if (options.onPositioned) {
          options.onPositioned(content);
        }
      });
  }

  function attach(trigger, content, options) {
    options = options || {};
    function update() {
      if (trigger instanceof Element && !trigger.isConnected) {
        return;
      }
      position(trigger, content, options);
    }
    if (options.autoUpdate) {
      return window.FloatingUIDOM.autoUpdate(
        trigger,
        content,
        update,
        typeof options.autoUpdate === "object" ? options.autoUpdate : {}
      );
    }
    update();
    return function () {};
  }

  // get-scroll-parent.ts
  function scrollParent(node) {
    if (!node || node === document.documentElement) {
      return null;
    }
    if (node instanceof HTMLElement) {
      var overflowY = window.getComputedStyle(node).overflowY;
      if (overflowY !== "visible" && overflowY !== "hidden" && node.scrollHeight > node.clientHeight) {
        return node;
      }
    }
    return scrollParent(node.parentNode) || window;
  }

  // The portal outlets Ember's application renders in #main after the app.
  function portal(id) {
    var node = document.getElementById(id);
    if (!node) {
      node = document.createElement("div");
      node.id = id;
      (document.getElementById("main") || document.body).appendChild(node);
    }
    return node;
  }

  var guid = 0;
  var open = [];

  function triggersFor(list) {
    if (list && !Array.isArray(list)) {
      return mobileView() ? list.mobile || ["click"] : list.desktop || ["click"];
    }
    return list || ["click"];
  }

  // The d-trap-tab modifier: Tab and Shift-Tab cycle in the content, and
  // with autofocus the first focusable element takes focus.
  var FOCUSABLE = "details:not(.is-disabled) summary, [autofocus], a, input, select, textarea, summary";

  function canHoldFocus(el) {
    if (el.matches(":disabled") || el.closest("[inert]")) {
      return false;
    }
    if (!(el.offsetParent || el.getClientRects().length || el.getBoundingClientRect().width)) {
      return false;
    }
    return getComputedStyle(el).visibility === "visible";
  }

  function trapTab(element, autofocus) {
    if (autofocus === undefined || autofocus) {
      var preset = element.querySelector("[autofocus]");
      if (!preset || document.activeElement !== preset) {
        var target =
          preset ||
          Array.prototype.slice
            .call(element.querySelectorAll(FOCUSABLE + ", button:not(.modal-close)"))
            .find(canHoldFocus);
        if (target) {
          target.focus({ preventScroll: true });
        }
      }
    }
    function onKeydown(event) {
      if (event.key !== "Tab") {
        return;
      }
      var all = Array.prototype.slice
        .call(element.querySelectorAll(FOCUSABLE + ", button:enabled"))
        .filter(function (el) {
          return el.getAttribute("tabindex") !== "-1" && canHoldFocus(el);
        });
      var first = all[0];
      var last = all[all.length - 1];
      if (event.shiftKey) {
        if (document.activeElement === first) {
          last && last.focus();
          event.preventDefault();
        }
      } else if (document.activeElement === last) {
        event.preventDefault();
        var close = element.querySelector(".modal-close") || first;
        close && close.focus({ preventScroll: true });
      }
    }
    element.addEventListener("keydown", onKeydown);
    return function () {
      element.removeEventListener("keydown", onKeydown);
    };
  }

  // FloatKitInstance with the DMenuInstance / DTooltipInstance parts.
  function Instance(kind, trigger, options) {
    this.kind = kind;
    this.trigger = trigger;
    this.options = options;
    this.expanded = false;
    this.content = null;
    this.id = (trigger instanceof HTMLElement && trigger.id) || "ember-float-" + ++guid;
    this.cleanups = [];
    this.listeners = [];
    var self = this;
    ["show", "close", "destroy", "onClick", "onPointerMove", "onPointerLeave", "trapPointerDown"].forEach(
      function (name) {
        self[name] = self[name].bind(self);
      }
    );
  }

  Instance.prototype.listen = function (el, name, fn, opts) {
    el.addEventListener(name, fn, opts);
    this.listeners.push(function () {
      el.removeEventListener(name, fn, opts);
    });
  };

  // setupListeners(): what opens and closes a float its trigger owns.
  Instance.prototype.setupListeners = function () {
    var el = this.trigger instanceof HTMLElement ? this.trigger : null;
    if (!el) {
      return;
    }
    this.listen(el, "pointerdown", this.trapPointerDown);
    var self = this;
    triggersFor(this.options.triggers).forEach(function (name) {
      if (name === "hover") {
        self.listen(el, "pointermove", self.onPointerMove, { passive: true });
        if (!self.options.interactive) {
          self.listen(el, "pointerleave", self.onPointerLeave, { passive: true });
        }
      } else if (name === "click") {
        self.listen(el, "click", self.onClick, { passive: true });
      }
    });
  };

  Instance.prototype.trapPointerDown = function (event) {
    event.stopPropagation();
  };

  Instance.prototype.onPointerMove = function (event) {
    if (this.expanded && this.trigger.contains(event.target) && event.pointerType !== "touch") {
      return;
    }
    this.show();
  };

  Instance.prototype.onPointerLeave = function () {
    if (triggersFor(this.options.untriggers).indexOf("hover") !== -1) {
      this.close();
    }
  };

  Instance.prototype.onClick = function () {
    if (this.expanded && triggersFor(this.options.untriggers).indexOf("click") !== -1) {
      this.close();
      return;
    }
    this.show();
  };

  // DFloatBody: the content element, with the classes and attributes
  // float-kit gives it.
  Instance.prototype.render = function () {
    var o = this.options;
    var menu = this.kind === "menu";
    var content = document.createElement("div");
    var classes = [menu ? "fk-d-menu" : "fk-d-tooltip__content"];
    if (o.mainClass) {
      classes.push(o.mainClass);
    }
    if (o.animated) {
      classes.push("-animated");
    }
    classes.push("-expanded");
    content.className = classes.join(" ");
    var role = menu ? o.contentRole : "tooltip";
    var presentational = role === "none" || role === "presentation";
    if (!presentational && o.ariaLabel) {
      content.setAttribute("aria-label", o.ariaLabel);
    } else if (!presentational && document.getElementById(this.id)) {
      content.setAttribute("aria-labelledby", this.id);
    }
    content.dataset.content = "";
    if (o.identifier) {
      content.dataset.identifier = o.identifier;
    }
    if (role) {
      content.setAttribute("role", role);
    }
    var maxWidth =
      typeof o.maxWidth === "number"
        ? "min(" + o.maxWidth + "px, calc(100dvw - " + horizontalInset(o) + "px))"
        : o.maxWidth;
    content.setAttribute("style", "max-width: " + maxWidth);
    var inner = document.createElement("div");
    inner.className = menu ? "fk-d-menu__inner-content" : "fk-d-tooltip__inner-content";
    if (o.content instanceof Node) {
      inner.appendChild(o.content);
    } else if (o.content != null) {
      inner.innerHTML = o.content;
    }
    content.appendChild(inner);
    return content;
  };

  Instance.prototype.show = function () {
    if (this.expanded) {
      return this;
    }
    var self = this;
    // The services keep one float per identifier open.
    open.slice().forEach(function (other) {
      if (other !== self && self.options.identifier && other.options.identifier === self.options.identifier) {
        other.close({ focusTrigger: false });
      }
    });
    if (open.indexOf(this) === -1) {
      open.push(this);
    }
    this.expanded = true;
    var o = this.options;
    var el = this.trigger instanceof HTMLElement ? this.trigger : null;
    if (el && this.kind === "tooltip" && el.classList.contains("fk-d-tooltip__trigger")) {
      el.setAttribute("aria-expanded", "true");
      el.classList.add("-expanded");
    }
    var content = (this.content = this.render());
    portal(this.kind === "menu" ? "d-menu-portals" : "d-tooltip-portals").appendChild(content);
    this.cleanups.push(attach(this.trigger, content, o));

    // trapInteractionPropagation: a press in the content is not a click
    // outside it.
    ["pointerdown", "mousedown", "touchend"].forEach(function (name) {
      content.addEventListener(name, function (event) {
        event.stopPropagation();
      });
    });
    var trap = this.kind === "menu" ? o.trapTab : o.interactive && o.trapTab;
    if (trap) {
      this.cleanups.push(trapTab(content, o.autofocus));
    }
    if (o.closeOnClickOutside) {
      var outside = function (event) {
        if (!content.contains(event.target)) {
          self.close({ focusTrigger: false });
        }
      };
      document.addEventListener("pointerdown", outside, { passive: true });
      this.cleanups.push(function () {
        document.removeEventListener("pointerdown", outside, { passive: true });
      });
    }
    if (o.closeOnEscape) {
      var escape = function (event) {
        if (event.key === "Escape") {
          event.stopPropagation();
          event.preventDefault();
          self.close();
        }
      };
      document.addEventListener("keydown", escape, { capture: true });
      this.cleanups.push(function () {
        document.removeEventListener("keydown", escape, { capture: true });
      });
    }
    if (o.closeOnScroll && el) {
      var parent = scrollParent(el);
      var onScroll = function () {
        self.close();
      };
      parent.addEventListener("scroll", onScroll, { passive: true });
      this.cleanups.push(function () {
        parent.removeEventListener("scroll", onScroll);
      });
    }
    if (o.onShow) {
      o.onShow(this);
    }
    return this;
  };

  function animateClosing(element) {
    if (!element || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      return Promise.resolve();
    }
    element.classList.add("-closing");
    return new Promise(function (resolve) {
      var style = window.getComputedStyle(element);
      var duration = parseFloat(style.animationDuration) * 1000 || 0;
      var delay = parseFloat(style.animationDelay) * 1000 || 0;
      var done = function () {
        clearTimeout(timer);
        element.removeEventListener("animationend", done);
        resolve();
      };
      var timer = setTimeout(done, Math.max(duration + delay + 50, 50));
      element.addEventListener("animationend", done);
    }).then(function () {
      element.classList.remove("-closing");
    });
  }

  Instance.prototype.teardown = function () {
    this.cleanups.forEach(function (fn) {
      fn();
    });
    this.cleanups = [];
    if (this.content) {
      this.content.remove();
    }
    this.content = null;
    var i = open.indexOf(this);
    if (i !== -1) {
      open.splice(i, 1);
    }
  };

  Instance.prototype.close = function (options) {
    if (!this.expanded) {
      return Promise.resolve();
    }
    options = options || {};
    var self = this;
    var o = this.options;
    var content = this.content;
    var el = this.trigger instanceof HTMLElement ? this.trigger : null;
    this.expanded = false;
    if (el && el.classList.contains("fk-d-tooltip__trigger")) {
      el.setAttribute("aria-expanded", "false");
      el.classList.remove("-expanded");
    }
    if (this.kind === "tooltip") {
      this.teardown();
      if (o.onClose) {
        o.onClose(options.data);
      }
      return Promise.resolve();
    }
    // DMenuInstance.close(): animated out, then focus back to the trigger
    // unless asked not to (and focus was not left on nothing).
    var ownedFocus = content && content.contains(document.activeElement);
    return animateClosing(content).then(function () {
      self.teardown();
      var unowned = !document.activeElement || document.activeElement === document.body;
      if ((options.focusTrigger == null ? true : options.focusTrigger) || (ownedFocus && unowned)) {
        var target = o.focusTarget ? o.focusTarget() : el;
        target && target.focus();
      }
      if (o.onClose) {
        o.onClose(options.data);
      }
    });
  };

  Instance.prototype.destroy = function () {
    var closing = this.close();
    this.listeners.forEach(function (fn) {
      fn();
    });
    this.listeners = [];
    return closing;
  };

  // The menu service's show(trigger, options) for a trigger it does not
  // know yet: a new instance, opened now.
  function menu(trigger, options) {
    var instance = new Instance("menu", trigger, merge(MENU, options));
    return instance.show();
  }

  // DTooltip: the trigger span around what the caller gives, listening
  // as DTooltip's instance does (listeners: true).
  function tooltip(inner, options) {
    var o = merge(TOOLTIP, options);
    var trigger = document.createElement("span");
    var instance = new Instance("tooltip", trigger, o);
    if (o.identifier) {
      o.mainClass = o.identifier + "-content";
    }
    trigger.setAttribute("aria-expanded", "false");
    trigger.className = "fk-d-tooltip__trigger";
    if (o.identifier) {
      trigger.dataset.identifier = o.identifier;
    }
    trigger.dataset.trigger = "";
    trigger.id = instance.id;
    trigger.setAttribute("role", "button");
    var container = document.createElement("span");
    container.className = "fk-d-tooltip__trigger-container";
    if (inner) {
      container.appendChild(inner);
    }
    trigger.appendChild(container);
    instance.setupListeners();
    return instance;
  }

  Discourse.float = {
    PLACEMENTS: PLACEMENTS,
    headerOffset: headerOffset,
    position: position,
    attach: attach,
    menu: menu,
    tooltip: tooltip,
    close: function (identifier) {
      open.slice().forEach(function (instance) {
        if (instance.options.identifier === identifier) {
          instance.close();
        }
      });
    },
  };

  // A float does not outlive its page.
  if (Discourse.onPage) {
    Discourse.onPage(function () {
      return function () {
        open.slice().forEach(function (instance) {
          instance.teardown();
        });
      };
    });
  }
})();
