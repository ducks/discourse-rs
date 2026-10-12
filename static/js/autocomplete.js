// DAutocompleteModifier (ui-kit/modifiers/d-autocomplete.js) with
// TextareaAutocompleteHandler: a trigger key typed at a word's start (or
// a custom onKeyUp match) searches the dataSource as the term grows, the
// results open in a d-menu (DAutocompleteResults) placed against the
// textarea, arrows move the selection, Enter or Tab (or a click) put the
// chosen result in place of the term, Escape closes.
//
//   Discourse.autocomplete.setup(textarea, {
//     key, dataSource(term) -> results | Promise | SKIP,
//     render(results, selectedIndex) -> html, transformComplete(result),
//     afterComplete(text, event), onKeyUp(text, caret), triggerRule(opts),
//     autoSelectFirstSuggestion, preserveKey })
//   Discourse.autocomplete.SKIP, .CANCELLED, .isOpen()
(function () {
  "use strict";

  if (!Discourse.once("autocomplete")) {
    return;
  }

  var SKIP = "skip";
  var CANCELLED = "__CANCELLED";
  var ALLOWED_LETTERS = /[\s[{(+]/;
  // EMOJI_ALLOWED_PRECEDING_CHARS_REGEXP
  var EMOJI_ALLOWED_PRECEDING = /[\s.?,@/#!%&*;:\[\]{}=\-_()+]/;
  var MENU_OFFSET = 10;
  var MENU_MAX_WIDTH = 400;
  var PLACEMENTS = ["top-start", "top-end", "bottom-start", "bottom-end"];

  // The one d-autocomplete menu (menu.show with identifier d-autocomplete
  // replaces whatever it showed).
  var menu = null;

  function portals() {
    var node = document.getElementById("d-menu-portals");
    if (!node) {
      node = document.createElement("div");
      node.id = "d-menu-portals";
      document.body.appendChild(node);
    }
    return node;
  }

  function placementRect(placement, t, w, h) {
    var parts = placement.split("-");
    var y = parts[0] === "top" ? t.top - h - MENU_OFFSET : t.bottom + MENU_OFFSET;
    var x = parts[1] === "start" ? t.left : t.right - w;
    return { x: x, y: y };
  }

  // AUTO_PLACEMENT over the allowed placements: the first that fits.
  function place(anchor, content) {
    var t = anchor.getBoundingClientRect();
    var w = content.offsetWidth;
    var h = content.offsetHeight;
    var vw = document.documentElement.clientWidth;
    var vh = window.innerHeight;
    var chosen = PLACEMENTS[0];
    var spot = placementRect(chosen, t, w, h);
    for (var i = 0; i < PLACEMENTS.length; i++) {
      var candidate = placementRect(PLACEMENTS[i], t, w, h);
      if (candidate.x >= 0 && candidate.y >= 0 && candidate.x + w <= vw && candidate.y + h <= vh) {
        chosen = PLACEMENTS[i];
        spot = candidate;
        break;
      }
    }
    content.dataset.placement = chosen;
    content.style.left = spot.x + window.scrollX + "px";
    content.style.top = spot.y + window.scrollY + "px";
    content.style.visibility = "visible";
  }

  function closeMenu() {
    if (menu) {
      var closing = menu;
      menu = null;
      closing.element.remove();
      if (closing.onClose) {
        closing.onClose();
      }
    }
  }

  function setup(textarea, options) {
    var state = {
      expanded: false,
      results: [],
      selectedIndex: -1,
      completeStart: null,
      completeEnd: null,
      previousTerm: null,
      pendingSpaceSearch: false,
    };
    var autoSelect = options.autoSelectFirstSuggestion !== false;

    function value() {
      return textarea.value;
    }

    function caret() {
      return textarea.selectionStart;
    }

    function shouldTrigger(opts) {
      if (!options.triggerRule) {
        return Promise.resolve(true);
      }
      return Promise.resolve(options.triggerRule(textarea, opts || {}))
        .then(function (result) {
          return result === undefined || result === null ? true : result;
        })
        .catch(function () {
          return true;
        });
    }

    function render() {
      if (!menu || menu.owner !== state) {
        return;
      }
      var body = menu.element.querySelector(".fk-d-menu__inner-content > div");
      body.innerHTML = options.render(state.results, state.selectedIndex);
      var selected = body.querySelector("li a.selected");
      if (selected && !menu.initial) {
        selected.scrollIntoView({ block: "nearest", behavior: "smooth" });
      }
      menu.initial = false;
      place(textarea, menu.element);
    }

    function openMenu() {
      state.pendingSpaceSearch = false;
      state.selectedIndex = autoSelect ? 0 : -1;
      closeMenu();
      var element = document.createElement("div");
      element.className = "fk-d-menu -animated -expanded";
      element.dataset.content = "";
      element.dataset.identifier = "d-autocomplete";
      element.setAttribute("role", "dialog");
      element.style.maxWidth = "min(" + MENU_MAX_WIDTH + "px, -20px + 100dvw)";
      element.style.visibility = "hidden";
      element.dataset.strategy = "absolute";
      element.innerHTML = '<div class="fk-d-menu__inner-content"><div tabindex="-1"></div></div>';
      element.addEventListener("click", function (event) {
        var link = event.target.closest("li a");
        if (!link) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        var links = Array.prototype.slice.call(element.querySelectorAll("li a"));
        var index = links.indexOf(link);
        if (index >= 0) {
          selectResult(state.results[index], event);
        }
      });
      portals().appendChild(element);
      menu = {
        element: element,
        owner: state,
        closeOwner: closeAutocomplete,
        initial: true,
        onClose: function () {
          state.expanded = false;
          if (options.onClose) {
            options.onClose();
          }
        },
      };
      state.expanded = true;
      render();
    }

    function closeAutocomplete(resetSearchState) {
      if (menu && menu.owner === state) {
        closeMenu();
      }
      state.expanded = false;
      state.selectedIndex = -1;
      if (resetSearchState === false) {
        return;
      }
      state.pendingSpaceSearch = false;
      state.completeStart = null;
      state.completeEnd = null;
      state.results = [];
      state.previousTerm = null;
    }

    function updateResults(results) {
      if (state.completeStart === null || results === SKIP || results === CANCELLED) {
        return;
      }
      var old = JSON.stringify(state.results);
      state.results = results;
      if (!results || results.length === 0) {
        closeAutocomplete();
        return;
      }
      if (state.expanded) {
        // Only a change of results resets the selection.
        if (old !== JSON.stringify(results)) {
          state.selectedIndex = autoSelect ? 0 : -1;
        }
        render();
        return;
      }
      openMenu();
    }

    function performSearch(term) {
      if (state.previousTerm === term) {
        return Promise.resolve();
      }
      state.previousTerm = term;
      // A term with spaces keeps searching (full names) for mentions.
      if (options.key === "@" && term.indexOf(" ") !== -1 && term.trim().length > 0) {
        state.pendingSpaceSearch = true;
        if (term.slice(-1) === " ") {
          closeAutocomplete(false);
        }
      }
      var after = value()[caret()];
      if ((term.length !== 0 && term.trim().length === 0) || (after && after.trim())) {
        closeAutocomplete();
        return Promise.resolve();
      }
      var results = options.dataSource(term);
      if (results && typeof results.then === "function") {
        return results.then(
          function (resolved) {
            updateResults(resolved || []);
          },
          function () {
            closeAutocomplete();
          }
        );
      }
      updateResults(results || []);
      return Promise.resolve();
    }

    // guessCompletePosition: back from the caret to the trigger key, while
    // no whitespace intervenes.
    function guessCompletePosition(opts) {
      opts = opts || {};
      var pos = caret();
      if (opts.backSpace) {
        pos -= 1;
      }
      var initial = pos;
      var text = value();
      function step(position) {
        if (position < 0) {
          return Promise.resolve({ completeStart: null, completeEnd: null });
        }
        position -= 1;
        var prev = text[position];
        if (prev === options.key) {
          var before = text[position - 1];
          return shouldTrigger({ backSpace: opts.backSpace }).then(function (trigger) {
            var allowed =
              options.key === ":"
                ? before === undefined || EMOJI_ALLOWED_PRECEDING.test(before)
                : before === undefined || ALLOWED_LETTERS.test(before);
            if (trigger && allowed) {
              var term = text.substring(position + 1, initial);
              return { completeStart: position, completeEnd: position + term.length, term: term };
            }
            return next(position, before);
          });
        }
        return next(position, prev);
      }
      function next(position, prev) {
        var good = !/\s/.test(prev);
        if (opts.completeTerm) {
          good = good || prev === " ";
        }
        if (!good) {
          return Promise.resolve({ completeStart: null, completeEnd: null });
        }
        return step(position);
      }
      return step(pos);
    }

    function performAutocomplete() {
      var position = caret();
      var text = value();
      var key = text[position - 1];
      if (state.completeStart !== null && position >= state.completeStart) {
        state.completeEnd = position - 1;
      }
      if (
        state.pendingSpaceSearch &&
        (position < state.completeStart || position > state.completeEnd + 1)
      ) {
        closeAutocomplete();
        return Promise.resolve();
      }
      var custom = Promise.resolve(false);
      if (options.key && options.onKeyUp && key !== options.key) {
        var match = options.onKeyUp(text, position);
        if (match) {
          custom = shouldTrigger().then(function (trigger) {
            if (!trigger) {
              return false;
            }
            state.completeStart = position - match[0].length;
            state.completeEnd = position - 1;
            return performSearch(match[0].substring(1)).then(function () {
              return true;
            });
          });
        }
      }
      return custom.then(function (handled) {
        if (handled) {
          return;
        }
        if (state.completeStart === null && position > 0) {
          return guessCompletePosition().then(function (found) {
            if (found.completeStart !== null) {
              state.completeStart = found.completeStart;
              state.completeEnd = position - 1;
              return performSearch(found.term || "");
            }
            if (key === options.key) {
              var prev = text.charAt(position - 2);
              if (!prev || ALLOWED_LETTERS.test(prev)) {
                return shouldTrigger().then(function (trigger) {
                  if (trigger) {
                    state.completeStart = position - 1;
                    state.completeEnd = position - 1;
                    return performSearch("");
                  }
                });
              }
            }
          });
        }
        if (state.completeStart !== null) {
          var term = text.substring(state.completeStart + (options.key ? 1 : 0), position);
          if (!options.key || text[state.completeStart] === options.key) {
            state.completeEnd = position - 1;
            return performSearch(term);
          }
          closeAutocomplete();
        }
      });
    }

    function handleBackspace() {
      if (state.completeStart === null && options.key) {
        return guessCompletePosition({ backSpace: true }).then(function (found) {
          if (found.completeStart !== null) {
            state.completeStart = found.completeStart;
            state.completeEnd = caret() - 1;
            return performAutocomplete();
          }
        });
      }
      return Promise.resolve();
    }

    // TextareaAutocompleteHandler#replaceTerm: the term and its key give
    // way to the replacement and a space (unless one follows), as typing
    // would, so undo takes it back.
    function replaceTerm(start, end, term) {
      var space = value().substring(end + 1, end + 2) === " " ? "" : " ";
      textarea.setSelectionRange(start, end + 1);
      textarea.focus();
      if (!document.execCommand("insertText", false, term + space)) {
        textarea.setRangeText(term + space, start, end + 1, "end");
        textarea.dispatchEvent(new Event("input", { bubbles: true }));
      }
      var at = start + 1 + term.trim().length;
      textarea.setSelectionRange(at, at);
    }

    function completeTerm(result, event) {
      if (!result) {
        return Promise.resolve();
      }
      return Promise.resolve(options.transformComplete ? options.transformComplete(result, event) : result).then(
        function (term) {
          if (!term) {
            return;
          }
          var replacement = (options.preserveKey === false ? "" : options.key || "") + term;
          // Where the term is now: the text may have changed under it.
          return guessCompletePosition({ completeTerm: true }).then(function (found) {
            var start = found.completeStart;
            var end = found.completeEnd;
            if (start === null || start === undefined || end === null || end === undefined) {
              start = end = caret();
            }
            replaceTerm(start, end, replacement);
            if (options.afterComplete) {
              options.afterComplete(value(), event);
            }
          });
        }
      );
    }

    function selectResult(result, event) {
      return completeTerm(result, event).then(function () {
        closeAutocomplete();
        state.previousTerm = null;
      });
    }

    function hasModifier(event) {
      return event.ctrlKey || event.altKey || event.metaKey;
    }

    textarea.addEventListener("keyup", function (event) {
      if (hasModifier(event) || ["Enter", "Escape", "Tab"].indexOf(event.key) !== -1) {
        return;
      }
      performAutocomplete();
    });

    textarea.addEventListener("keydown", function (event) {
      if (state.expanded) {
        switch (event.key) {
          case "ArrowUp":
          case "ArrowDown":
            event.preventDefault();
            if (state.results.length) {
              var direction = event.key === "ArrowUp" ? -1 : 1;
              state.selectedIndex = Math.max(
                0,
                Math.min(state.results.length - 1, state.selectedIndex + direction)
              );
              render();
            }
            break;
          case "Enter":
          case "Tab":
            event.preventDefault();
            event.stopImmediatePropagation();
            // The composer's own Enter (a send) waits for no menu.
            event.stopPropagation();
            if (state.selectedIndex >= 0) {
              selectResult(state.results[state.selectedIndex], event);
            }
            break;
          case "Escape":
            event.preventDefault();
            event.stopPropagation();
            closeAutocomplete();
            break;
          case "ArrowRight":
            if (value()[caret()] === " ") {
              closeAutocomplete();
            }
            break;
          case "Backspace":
            handleBackspace();
            break;
        }
      } else if (event.key === "Backspace" && !hasModifier(event)) {
        handleBackspace();
      }
    });

    textarea.addEventListener("paste", function () {
      requestAnimationFrame(function () {
        performAutocomplete();
      });
    });

    return {
      close: closeAutocomplete,
    };
  }

  // handleGlobalClick: a click anywhere else closes the menu.
  document.addEventListener("click", function () {
    if (menu) {
      menu.closeOwner();
    }
  });

  Discourse.autocomplete = {
    setup: setup,
    SKIP: SKIP,
    CANCELLED: CANCELLED,
    isOpen: function () {
      return !!document.querySelector(".autocomplete");
    },
  };
})();
