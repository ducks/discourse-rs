// DOverflowControls (ui-kit/d-overflow-controls.gts) for the strips the
// server renders: a horizontal-overflow-nav's pills, the composer's
// toolbar. The scroller is stamped with its edge state (the
// data-d-scroll-overflow, -at-start and -at-end attributes the
// scroll-strip stylesheet fades by), and a chevron sits on each edge that
// can still scroll; a click scrolls one viewport. A horizontal overflow
// nav first brings its active pill to the middle (revealActive).
(function () {
  "use strict";

  if (!Discourse.once("overflow-controls")) {
    return;
  }

  var EDGE_TOLERANCE = 2;

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function scrollerOf(wrapper) {
    if (wrapper.classList.contains("--owned-scroller")) {
      for (var i = 0; i < wrapper.children.length; i++) {
        var child = wrapper.children[i];
        if (!child.classList.contains("d-overflow-controls__btn")) {
          return child;
        }
      }
      return null;
    }
    return wrapper.querySelector(":scope > .d-overflow-controls__content");
  }

  // The chevrons' extra classes: @buttonClass and @edgeButtonClasses.
  function buttonClasses(wrapper, edge) {
    if (wrapper.classList.contains("horizontal-overflow-nav__controls")) {
      return " horizontal-overflow-nav__scroll-" + edge;
    }
    if (wrapper.classList.contains("d-editor-button-bar__wrap")) {
      return " d-editor-button-bar__scroll-btn";
    }
    return "";
  }

  function measure(scroller) {
    var viewport = scroller.clientWidth;
    var content = scroller.scrollWidth;
    var offset = Math.abs(scroller.scrollLeft);
    var overflowing = content - viewport > EDGE_TOLERANCE;
    return {
      overflowing: overflowing,
      atStart: overflowing && offset <= EDGE_TOLERANCE,
      atEnd: overflowing && content - offset - viewport <= EDGE_TOLERANCE,
    };
  }

  function edgeButton(wrapper, scroller, edge) {
    var button = document.createElement("button");
    button.setAttribute("aria-hidden", "true");
    button.className = "d-overflow-controls__btn --" + edge + buttonClasses(wrapper, edge);
    button.tabIndex = -1;
    button.type = "button";
    button.innerHTML = icon("chevron-" + edge);
    button.addEventListener("mousedown", function (event) {
      event.preventDefault();
    });
    button.addEventListener("click", function () {
      var max = scroller.scrollWidth - scroller.offsetWidth;
      var target = scroller.scrollLeft + (edge === "left" ? -1 : 1) * scroller.offsetWidth;
      var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      scroller.scrollTo({ left: Math.max(0, Math.min(target, max)), behavior: reduce ? "instant" : "smooth" });
    });
    return button;
  }

  function update(wrapper, scroller) {
    var state = measure(scroller);
    scroller.setAttribute("data-d-scroll-axis", "horizontal");
    scroller.toggleAttribute("data-d-scroll-overflow", state.overflowing);
    scroller.toggleAttribute("data-d-scroll-at-start", state.atStart);
    scroller.toggleAttribute("data-d-scroll-at-end", state.atEnd);
    var left = wrapper.querySelector(":scope > .d-overflow-controls__btn.--left");
    var right = wrapper.querySelector(":scope > .d-overflow-controls__btn.--right");
    var wantLeft = state.overflowing && !state.atStart;
    var wantRight = state.overflowing && !state.atEnd;
    if (wantLeft && !left) {
      wrapper.insertBefore(edgeButton(wrapper, scroller, "left"), scroller);
    } else if (!wantLeft && left) {
      left.remove();
    }
    if (wantRight && !right) {
      scroller.insertAdjacentElement("afterend", edgeButton(wrapper, scroller, "right"));
    } else if (!wantRight && right) {
      right.remove();
    }
  }

  // revealInScroller with align: "center", the strip's own scroll only.
  function revealActive(scroller) {
    var active = scroller.querySelector("a.active, button.active");
    if (!active) {
      return;
    }
    var start = active.offsetLeft - scroller.offsetLeft;
    var target = start + active.offsetWidth / 2 - scroller.clientWidth / 2;
    var max = scroller.scrollWidth - scroller.clientWidth;
    var left = Math.max(0, Math.min(target, max));
    if (Math.abs(left - scroller.scrollLeft) > 0.5) {
      scroller.scrollTo({ left: left, behavior: "instant" });
    }
  }

  function setup(wrapper) {
    if (wrapper.dataset.overflowBound) {
      return null;
    }
    var scroller = scrollerOf(wrapper);
    if (!scroller) {
      return null;
    }
    wrapper.dataset.overflowBound = "true";
    if (wrapper.classList.contains("horizontal-overflow-nav__controls")) {
      revealActive(scroller);
    }
    var refresh = function () {
      update(wrapper, scroller);
    };
    refresh();
    scroller.addEventListener("scroll", refresh, { passive: true });
    var observer = window.ResizeObserver ? new ResizeObserver(refresh) : null;
    if (observer) {
      observer.observe(scroller);
    }
    return function () {
      scroller.removeEventListener("scroll", refresh);
      if (observer) {
        observer.disconnect();
      }
      delete wrapper.dataset.overflowBound;
    };
  }

  Discourse.onPage(function () {
    var cleanups = [];
    document.querySelectorAll(".d-overflow-controls").forEach(function (wrapper) {
      var cleanup = setup(wrapper);
      if (cleanup) {
        cleanups.push(cleanup);
      }
    });
    return function () {
      cleanups.forEach(function (fn) {
        fn();
      });
    };
  });
})();
