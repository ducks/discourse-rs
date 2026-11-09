// DAutocompleteModifier (ui-kit/modifiers/d-autocomplete.js) over an
// AutocompleteHandler: a trigger key typed at a word's start (or a custom
// onKeyUp match) searches the dataSource as the term grows, the results
// open in a d-menu (DAutocompleteResults) placed at the term (or against
// the whole input, fixedTextareaPosition), arrows move the selection,
// Enter or Tab (or a click) put the chosen result in place of the term,
// Escape closes.
//
//   Discourse.autocomplete.setup(element, {
//     key, dataSource(term) -> results | Promise | SKIP,
//     render(results, selectedIndex) -> html, transformComplete(result),
//     afterComplete(text, event), onKeyUp(text, caret),
//     triggerRule(element, { backSpace, inCodeBlock, inLink }),
//     onRender(results), onClose(), autoSelectFirstSuggestion,
//     preserveKey, fixedTextareaPosition,
//     handler,              // the AutocompleteHandler; by default the
//                           // element's own, a textarea's
//   }) -> { close, destroy }
//   Discourse.autocomplete.textareaHandler(textarea, { inCodeBlock })
//   Discourse.autocomplete.SKIP, .CANCELLED, .isOpen()
//
// An AutocompleteHandler (lib/composer/text-manipulation's interface) is
// { getValue, getCaretPosition, replaceTerm(start, end, term),
// getCaretCoords(position), inCodeBlock, inLink }: the textarea's here,
// the rich editor's in rich-editor/text-manipulation.js.
//
// The sources the composer and chat both search are here too:
// lib/user-search (userSearch), lib/hashtag-autocomplete (hashtagOptions)
// and the results' components (renderUsers, renderEmojis, renderHashtags).
(function () {
  "use strict";

  if (!Discourse.once("autocomplete")) {
    return;
  }

  var SKIP = "skip";
  var CANCELLED = "__CANCELLED";
  var ALLOWED_LETTERS = /[\s[{(+]/;
  // EMOJI_ALLOWED_PRECEDING_CHARS_REGEXP
  var EMOJI_ALLOWED_PRECEDING = /[\s.…?,@/#!%&*;:\[\]{}=\-_()+]/;
  var MENU_OFFSET = 10;
  var MENU_MAX_WIDTH = 400;
  var PLACEMENTS = ["top-start", "top-end", "bottom-start", "bottom-end"];
  // createVirtualElementAtCaret's offsets from the term's start.
  var TRIGGER_CHAR_RELATIVE_OFFSET = 9;
  var VERTICAL_RELATIVE_OFFSET = 10;

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

  // AUTO_PLACEMENT over the allowed placements: the first that fits
  // against the reference rect `t`.
  function place(t, content) {
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

  // lib/caret-position's caretCoordinates: where `pos` is in the textarea,
  // relative to it, measured on an off-screen clone of its content.
  function caretCoordinates(element, pos) {
    var styles = getComputedStyle(element);
    var important = function (prop) {
      return styles.getPropertyValue(prop);
    };
    var isRTL = document.documentElement.classList.contains("rtl");
    var clone = document.createElement("div");
    var p = document.createElement("p");
    clone.appendChild(p);
    document.body.appendChild(clone);
    try {
      Object.assign(clone.style, {
        boxSizing: "content-box",
        border: "1px solid black",
        padding: important("padding"),
        resize: important("resize"),
        maxHeight: important("height"),
        overflowY: "auto",
        wordWrap: "break-word",
        position: "absolute",
        left: isRTL ? "auto" : "-7000px",
        right: isRTL ? "-7000px" : "auto",
        width: important("width"),
        height: important("height"),
      });
      Object.assign(p.style, {
        margin: 0,
        padding: 0,
        wordWrap: "break-word",
        letterSpacing: important("letter-spacing"),
        fontFamily: important("font-family"),
        fontSize: important("font-size"),
        lineHeight: important("line-height"),
      });
      var val = element.value.replace("\r", "");
      var before = pos - 1;
      var after = pos;
      // if before and after are \n insert a space
      var insertSpaceAfterBefore = val[before] === "\n" && val[after] === "\n";
      var guard = function (v) {
        return v
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;")
          .replace(/[ ]/g, "&#x200b;&nbsp;&#x200b;")
          .replace(/\n/g, "<br />");
      };
      var makeCursor = function (index, klass, color) {
        var l = val.substring(index, index + 1);
        if (l === "\n") {
          return "<br>";
        }
        return (
          "<span class='" + klass + "' style='background-color:" + color + "; margin:0; padding: 0'>" + guard(l) + "</span>"
        );
      };
      var html = "";
      if (before >= 0) {
        html += guard(val.substring(0, pos - 1)) + makeCursor(before, "before", "#d0ffff");
        if (insertSpaceAfterBefore) {
          html += makeCursor(0, "post-before", "#d0ffff");
        }
      }
      if (after >= 0) {
        html += makeCursor(after, "after", "#ffd0ff");
        if (after - 1 < val.length) {
          html += guard(val.substring(after + 1));
        }
      }
      p.innerHTML = html;
      clone.scrollTop = element.scrollTop;
      var letter = p.querySelector("span");
      var letterRect = letter.getBoundingClientRect();
      var left = letterRect.left;
      if (letter.classList.contains("before")) {
        left += letter.offsetWidth;
      }
      var pRect = p.getBoundingClientRect();
      return { left: left - pRect.left, top: letterRect.top - pRect.top - clone.scrollTop };
    } finally {
      clone.remove();
    }
  }

  // TextareaAutocompleteHandler. `opts.inCodeBlock` says whether the caret
  // is in code (lib/utilities' inCodeBlock needs the page's markdown
  // engine, below); without it, never.
  function textareaHandler(textarea, opts) {
    opts = opts || {};
    return {
      getValue: function () {
        return textarea.value;
      },
      // The term and its key give way to the replacement and a space
      // (unless one follows), as typing would, so undo takes it back.
      replaceTerm: function (start, end, term) {
        var space = textarea.value.substring(end + 1, end + 2) === " " ? "" : " ";
        textarea.setSelectionRange(start, end + 1);
        textarea.focus();
        if (!document.execCommand("insertText", false, term + space)) {
          textarea.setRangeText(term + space, start, end + 1, "end");
          textarea.dispatchEvent(new Event("input", { bubbles: true }));
        }
        var at = start + 1 + term.trim().length;
        textarea.setSelectionRange(at, at);
      },
      getCaretPosition: function () {
        return textarea.selectionStart || 0;
      },
      getCaretCoords: function (start) {
        return caretCoordinates(textarea, start + 1);
      },
      inCodeBlock: function () {
        return Promise.resolve(opts.inCodeBlock ? opts.inCodeBlock() : false);
      },
    };
  }

  // utilities' inCodeBlock(text, pos): the caret, marked with a
  // private-use character, cooks inside code. `cook` is the page's
  // markdown engine, resolving to the HTML.
  var CODE_MARKERS = /    |```|~~~|[^`]`[^`]|\[code\]/;

  function inCodeBlock(text, pos, cook) {
    if (!CODE_MARKERS.test(text)) {
      return Promise.resolve(false);
    }
    var marker = null;
    for (var code = 0xe000; code <= 0xf8ff; ++code) {
      if (text.indexOf(String.fromCharCode(code)) === -1) {
        marker = String.fromCharCode(code);
        break;
      }
    }
    if (!marker) {
      return Promise.resolve(false);
    }
    return Promise.resolve(cook(text.slice(0, pos) + marker + text.slice(pos))).then(function (html) {
      var doc = new DOMParser().parseFromString("<body>" + html + "</body>", "text/html");
      var found = false;
      doc.querySelectorAll("code").forEach(function (node) {
        if (node.textContent.indexOf(marker) !== -1) {
          found = true;
        }
      });
      return found;
    });
  }

  function setup(element, options) {
    var handler = options.handler || textareaHandler(element);
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
      return handler.getValue();
    }

    function caret() {
      return handler.getCaretPosition();
    }

    function shouldTrigger(opts) {
      if (!options.triggerRule) {
        return Promise.resolve(true);
      }
      var context = Object.assign({}, opts || {}, {
        inCodeBlock: function () {
          return handler.inCodeBlock ? handler.inCodeBlock() : false;
        },
        inLink: function () {
          return handler.inLink ? handler.inLink() : false;
        },
      });
      return Promise.resolve()
        .then(function () {
          return options.triggerRule(element, context);
        })
        .then(function (result) {
          return result === undefined || result === null ? true : result;
        })
        .catch(function (e) {
          console.error("[autocomplete] triggerRule error: ", e);
          return true;
        });
    }

    // What the menu is placed against: the whole input
    // (createVirtualElementAtTextarea), else the term's start as it was
    // when the menu opened (createVirtualElementAtCaret).
    function anchorRect() {
      var rect = element.getBoundingClientRect();
      if (options.fixedTextareaPosition || !handler.getCaretCoords) {
        return rect;
      }
      if (menu.rect) {
        return menu.rect;
      }
      try {
        var coords = handler.getCaretCoords(state.completeStart !== null ? state.completeStart : caret());
        var left = rect.left + coords.left + TRIGGER_CHAR_RELATIVE_OFFSET;
        var top = rect.top + coords.top + VERTICAL_RELATIVE_OFFSET;
        menu.rect = { left: left, top: top, right: left + 1, bottom: top + 10 };
      } catch (e) {
        console.error("[autocomplete] getAbsoluteCaretCoords: ", e);
        menu.rect = rect;
      }
      return menu.rect;
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
      place(anchorRect(), menu.element);
      if (options.onRender) {
        options.onRender(state.results);
      }
    }

    function openMenu() {
      state.pendingSpaceSearch = false;
      state.selectedIndex = autoSelect ? 0 : -1;
      closeMenu();
      var content = document.createElement("div");
      content.className = "fk-d-menu -animated -expanded";
      content.dataset.content = "";
      content.dataset.identifier = "d-autocomplete";
      content.setAttribute("role", "dialog");
      content.style.maxWidth = "min(" + MENU_MAX_WIDTH + "px, -20px + 100dvw)";
      content.style.visibility = "hidden";
      content.dataset.strategy = "absolute";
      content.innerHTML = '<div class="fk-d-menu__inner-content"><div tabindex="-1"></div></div>';
      content.addEventListener("click", function (event) {
        var link = event.target.closest("li a");
        if (!link) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        var links = Array.prototype.slice.call(content.querySelectorAll("li a"));
        var index = links.indexOf(link);
        if (index >= 0) {
          selectResult(state.results[index], event);
        }
      });
      portals().appendChild(content);
      menu = {
        element: content,
        owner: state,
        initial: true,
        rect: null,
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
            // A newer term's search went out meanwhile: its results, not
            // these, are the menu's (a source's answers may come out of
            // order).
            if (state.previousTerm !== term) {
              return;
            }
            updateResults(resolved || []);
          },
          function (e) {
            if (!e || e.name !== "AbortError") {
              console.error("[autocomplete] performSearch: ", e);
            }
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

    // completeTextareaTerm: a transformComplete that throws (a result
    // missing its name) completes nothing, its message in a toast.
    function completeTerm(result, event) {
      if (!result) {
        return Promise.resolve();
      }
      return Promise.resolve()
        .then(function () {
          return options.transformComplete ? options.transformComplete(result, event) : result;
        })
        .then(
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
                start = caret();
                end = start - 1;
              }
              handler.replaceTerm(start, end, replacement);
              if (options.afterComplete) {
                options.afterComplete(value(), event);
              }
            });
          },
          function (e) {
            toastError((e && e.message) || String(e));
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

    function onKeyUp(event) {
      if (hasModifier(event) || ["Enter", "Escape", "Tab"].indexOf(event.key) !== -1) {
        return;
      }
      performAutocomplete().catch(function (e) {
        console.error("[autocomplete] handleKeyup: ", e);
      });
    }

    function onKeyDown(event) {
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
    }

    function onPaste() {
      requestAnimationFrame(function () {
        performAutocomplete();
      });
    }

    // handleGlobalClick: a click anywhere else closes the menu.
    function onGlobalClick() {
      if (state.expanded) {
        closeAutocomplete();
      }
    }

    element.addEventListener("keyup", onKeyUp);
    element.addEventListener("keydown", onKeyDown);
    element.addEventListener("paste", onPaste);
    document.addEventListener("click", onGlobalClick);

    return {
      close: closeAutocomplete,
      destroy: function () {
        element.removeEventListener("keyup", onKeyUp);
        element.removeEventListener("keydown", onKeyDown);
        element.removeEventListener("paste", onPaste);
        document.removeEventListener("click", onGlobalClick);
        closeAutocomplete();
      },
    };
  }

  // toasts.error({ duration: "short", data: { message } }).
  function toastError(message) {
    var container = document.querySelector(".fk-d-default-toast-container");
    if (!container) {
      container = document.createElement("section");
      container.className = "fk-d-default-toast-container";
      document.body.appendChild(container);
    }
    var toast = document.createElement("div");
    toast.className = "fk-d-default-toast -error";
    toast.setAttribute("role", "log");
    toast.innerHTML =
      '<div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts">' +
      '<div class="fk-d-default-toast__message"></div></div></div>';
    toast.querySelector(".fk-d-default-toast__message").textContent = message;
    container.appendChild(toast);
    setTimeout(function () {
      toast.remove();
    }, 3000);
  }

  function escapeHtml(s) {
    return String(s === undefined || s === null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#x27;");
  }

  // iconHTML: the sprite's symbol, with any extra class before svg-string.
  function iconHtml(name, extraClass) {
    return (
      '<svg class="fa d-icon d-icon-' + escapeHtml(name) + " svg-icon fa-width-auto" +
      (extraClass ? " " + escapeHtml(extraClass) : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      escapeHtml(name) + '"></use></svg>'
    );
  }

  function basePath() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function cloneJSON(value) {
    return value === undefined ? value : JSON.parse(JSON.stringify(value));
  }

  // A promise of the results unless the next search (or five seconds)
  // overtakes it, as user-search and hashtag-autocomplete cancel theirs:
  // `request(signal)` runs once `delay` passes with no newer search.
  function debounced(holder, delay, request) {
    clearTimeout(holder.timer);
    if (holder.controller) {
      holder.controller.abort();
      holder.controller = null;
    }
    if (holder.resolve) {
      holder.resolve(CANCELLED);
    }
    return new Promise(function (resolve) {
      holder.resolve = resolve;
      var giveUp = setTimeout(function () {
        resolve(CANCELLED);
      }, 5000);
      holder.timer = setTimeout(function () {
        var controller = new AbortController();
        holder.controller = controller;
        Promise.resolve()
          .then(function () {
            return request(controller.signal);
          })
          .then(function (result) {
            clearTimeout(giveUp);
            resolve(result);
          })
          .catch(function () {
            clearTimeout(giveUp);
            resolve(CANCELLED);
          })
          .finally(function () {
            if (holder.controller === controller) {
              holder.controller = null;
            }
            if (holder.resolve === resolve) {
              holder.resolve = null;
            }
          });
      }, delay);
    });
  }

  function getJson(url, signal) {
    return fetch(url, {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
      signal: signal,
    }).then(function (r) {
      if (!r.ok) {
        var error = new Error(String(r.status));
        error.response = r;
        throw error;
      }
      return r.json();
    });
  }

  // lib/user-search: the term searched 300ms after typing stops, cached
  // for 30 seconds per url and parameters, ranked by organizeResults.
  var USER_SEARCH_LIMIT = 6;
  // ajax-error's DEFAULT_RATE_LIMIT_WAIT_SECONDS
  var RATE_LIMIT_WAIT = 15;
  var userSearchState = null;

  function resetUserSearchCache() {
    if (userSearchState) {
      clearTimeout(userSearchState.holder.timer);
      if (userSearchState.holder.resolve) {
        userSearchState.holder.resolve(CANCELLED);
      }
    }
    userSearchState = {
      cache: {},
      cacheKey: null,
      cacheTime: null,
      currentTerm: null,
      rateLimitedUntil: 0,
      holder: {},
    };
  }
  resetUserSearchCache();

  // all punctuation except for -, _ and . which are allowed in usernames
  // note: these are valid in names, but will end up tripping search anyway so just skip
  // this means searching for `sam saffron` is OK but if my name is `sam$ saffron` autocomplete
  // will not find me, which is a reasonable compromise
  //
  // we also ignore if we notice a double space or a string that is only a space
  var USER_IGNORE =
    /([ -⁯⸀-⹿\\'!"#$%&()*,\/:;<=>?\[\]^`{|}~])|\s\s|^\s$|^[^+]*\+[^@]*$/;

  function skipSearch(term, allowEmails, lastSeenUsers) {
    if (lastSeenUsers) {
      return false;
    }
    if (term.indexOf("@") !== -1 && !allowEmails) {
      return true;
    }
    return USER_IGNORE.test(term);
  }

  function eagerCompleteSearch(term, scopedId) {
    return term === "" && !!scopedId;
  }

  // utilities' emailValid
  function emailValid(email) {
    return /^[a-zA-Z0-9!#$%&'*+\/=?\^_`{|}~\-]+(?:\.[a-zA-Z0-9!#$%&'\*+\/=?\^_`{|}~\-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?$/.test(
      email
    );
  }

  function lowerCaseIncludes(string, term) {
    return !!(string && term && string.toLowerCase().indexOf(term.toLowerCase()) !== -1);
  }

  function lower(s) {
    return s === undefined || s === null ? s : String(s).toLowerCase();
  }

  function organizeResults(r, options) {
    if (r === CANCELLED) {
      return r;
    }
    var exclude = options.exclude || [];
    // Sometimes the term passed contains spaces, but the search is limited
    // to the first word only.
    var term = options.term ? options.term.trim().split(/\s/, 1)[0] : options.term;
    var users = [];
    var emails = [];
    var groups = [];
    (r.users || []).forEach(function (user) {
      if (exclude.indexOf(user.username) === -1) {
        user.isUser = true;
        user.isMetadataMatch = !!term && !lowerCaseIncludes(user.username, term) && !lowerCaseIncludes(user.name, term);
        users.push(user);
      }
    });
    if (options.allowEmails && emailValid(options.term)) {
      emails.push({ username: options.term, isEmail: true });
    }
    (r.groups || []).forEach(function (group) {
      if (exclude.indexOf(group.name) === -1) {
        group.isGroup = true;
        groups.push(group);
      }
    });
    // These sets keep a user or group from being suggested twice.
    var userSet = {};
    var groupSet = {};
    var pick = function (list, set, key, test) {
      return list.filter(function (item) {
        if (set[item[key]] || !test(item)) {
          return false;
        }
        set[item[key]] = true;
        return true;
      });
    };
    var results = [].concat(
      // 1. Exact username matches
      pick(users, userSet, "username", function (u) {
        return lower(u.username) === lower(term);
      }),
      // 2. Exact group name matches
      pick(groups, groupSet, "name", function (g) {
        return lower(g.name) === lower(term);
      }),
      emails,
      // 3. Partial username matches
      pick(users, userSet, "username", function (u) {
        return !u.isMetadataMatch && !(term && !lowerCaseIncludes(u.username, term));
      }),
      // 4. Partial group name matches
      pick(groups, groupSet, "name", function (g) {
        return !(term && !lowerCaseIncludes(g.name, term));
      }),
      // 5. Exact name matches
      pick(users, userSet, "username", function (u) {
        return lower(u.name) === lower(term) && !u.isMetadataMatch;
      }),
      // 6. Partial name matches
      pick(users, userSet, "username", function (u) {
        return lowerCaseIncludes(u.name, term) && !u.isMetadataMatch;
      }),
      // 7. Exact group full_name matches
      pick(groups, groupSet, "name", function (g) {
        return !!g.full_name && lower(g.full_name) === lower(term);
      }),
      // 8. Partial group full_name matches
      pick(groups, groupSet, "name", function (g) {
        return !!g.full_name && lowerCaseIncludes(g.full_name, term);
      }),
      // 9. Users with metadata match
      users.filter(function (u) {
        return u.isMetadataMatch;
      })
    ).slice(0, options.limit);
    // Only the users, emails and groups actually in the results.
    results.users = results.filter(function (item) {
      return item.isUser;
    });
    results.emails = results.filter(function (item) {
      return item.isEmail;
    });
    results.groups = results.filter(function (item) {
      return item.isGroup;
    });
    return results;
  }

  // rateLimitWaitSeconds: Retry-After, else the body's wait_seconds.
  function rateLimitWait(response) {
    var retryAfter = parseInt(response.headers.get("Retry-After"), 10);
    if (retryAfter > 0) {
      return Promise.resolve(retryAfter);
    }
    return response
      .json()
      .then(function (json) {
        var seconds = json && json.extras && json.extras.wait_seconds;
        return seconds > 0 ? seconds : RATE_LIMIT_WAIT;
      })
      .catch(function () {
        return RATE_LIMIT_WAIT;
      });
  }

  function performUserSearch(url, term, params, signal) {
    var s = userSearchState;
    var cached = s.cache[term];
    if (cached) {
      return Promise.resolve(cloneJSON(cached));
    }
    if (s.rateLimitedUntil > Date.now()) {
      return Promise.resolve(CANCELLED);
    }
    var eager = eagerCompleteSearch(term, params.topic_id || params.category_id);
    if (term === "" && !eager && !params.last_seen_users) {
      // The server returns no results in this case, so no point checking;
      // nor an empty list, which would end the autocomplete.
      return Promise.resolve(CANCELLED);
    }
    var query = new URLSearchParams();
    query.append("term", term);
    Object.keys(params).forEach(function (key) {
      var v = params[key];
      if (v === undefined || v === null) {
        return;
      }
      if (Array.isArray(v)) {
        v.forEach(function (item) {
          query.append(key + "[]", item);
        });
      } else {
        query.append(key, v);
      }
    });
    var returnVal = CANCELLED;
    return getJson(url + (url.indexOf("?") === -1 ? "?" : "&") + query.toString(), signal)
      .then(function (r) {
        var hasResults = !!(
          (r.users && r.users.length) ||
          (r.groups && r.groups.length) ||
          (r.emails && r.emails.length)
        );
        // Eager loading with no results: not an empty list either.
        if (eager && !hasResults) {
          r = CANCELLED;
        }
        s.cache[term] = r;
        s.cacheTime = Date.now();
        // A newer search term: this one is cancelled.
        if (term === s.currentTerm) {
          returnVal = r;
        }
      })
      .catch(function (error) {
        var response = error && error.response;
        if (response && response.status === 429) {
          return rateLimitWait(response).then(function (seconds) {
            s.rateLimitedUntil = Date.now() + seconds * 1000;
          });
        }
      })
      .then(function () {
        return cloneJSON(returnVal);
      });
  }

  function camelCaseToSnakeCase(text) {
    return text.replace(/([a-zA-Z])(?=[A-Z])/g, "$1_").toLowerCase();
  }

  // userSearch(options): the users (and groups, and an email) for
  // options.term, with lib/user-search's options.
  function userSearch(options) {
    if (options.term && options.term.length > 0 && options.term[0] === "@") {
      options.term = options.term.substring(1);
    }
    var term = options.term || "";
    var limit = options.limit || USER_SEARCH_LIMIT;
    var url = options.url || basePath() + "/u/search/users";
    var params = {
      search_context: options.searchContext,
      topic_id: options.topicId,
      category_id: options.categoryId,
      include_groups: options.includeGroups,
      include_mentionable_groups: options.includeMentionableGroups,
      include_messageable_groups: options.includeMessageableGroups,
      groups: options.groupMembersOf,
      topic_allowed_users: options.allowedUsers,
      include_staged_users: options.includeStagedUsers,
      last_seen_users: options.lastSeenUsers,
      prioritized_user_id: options.prioritizedUserId,
      limit: limit,
    };
    Object.keys(options.customUserSearchOptions || {}).forEach(function (key) {
      params[camelCaseToSnakeCase(key)] = options.customUserSearchOptions[key];
    });
    var s = userSearchState;
    s.currentTerm = term;
    var newCacheKey = JSON.stringify([url, params]);
    if (Date.now() - s.cacheTime > 30000 || s.cacheKey !== newCacheKey) {
      s.cache = {};
    }
    s.cacheKey = newCacheKey;
    if (skipSearch(term, options.allowEmails, options.lastSeenUsers)) {
      return Promise.resolve([]);
    }
    return debounced(s.holder, 300, function (signal) {
      return performUserSearch(url, term, params, signal);
    }).then(function (r) {
      return organizeResults(r, Object.assign({}, options, { limit: limit }));
    });
  }

  // validateSearchResult: a result missing its name throws (`messages`:
  // composer.autocomplete's username_missing and name_missing).
  function validateSearchResult(obj, messages) {
    messages = messages || {};
    if ((obj.isUser || obj.isEmail) && !obj.username) {
      throw new Error(messages.usernameMissing || "Selected autocomplete result is missing username.");
    }
    if (obj.isGroup && !obj.name) {
      throw new Error(messages.nameMissing || "Selected autocomplete result is missing name.");
    }
    return true;
  }

  // initUserStatusHtml: each user with a status keeps its index among the
  // results' users, which its item carries.
  function indexUserStatuses(results) {
    if (Array.isArray(results)) {
      (results.users || []).forEach(function (user, index) {
        if (user.status) {
          user.index = index;
        }
      });
    }
    return results;
  }

  // UserAutocompleteResults: users (their cssClasses, and a status in
  // place of its span as renderUserStatusHtml puts it), emails, groups.
  function renderUsers(results, selected) {
    var out = '<div class="autocomplete ac-user"><ul>';
    results.forEach(function (result, index) {
      var classes = [];
      // Only users carry custom classes.
      if (result.isUser === true && result.cssClasses) {
        classes.push(result.cssClasses);
      }
      if (index === selected) {
        classes.push("selected");
      }
      var title = result.isUser === true ? result.name : result.isEmail === true ? result.username : result.full_name;
      out +=
        "<li" + (result.index === undefined ? "" : ' data-index="' + escapeHtml(result.index) + '"') + "><a" +
        (classes.length ? ' class="' + escapeHtml(classes.join(" ")) + '"' : "") + ' href=""' +
        (title === undefined || title === null ? "" : ' title="' + escapeHtml(title) + '"') + ">";
      if (result.isUser) {
        out +=
          '<img alt="" width="24" height="24" src="' +
          escapeHtml((result.avatar_template || "").replace("{size}", "24")) +
          '" class="avatar" title="' + escapeHtml(result.username) + '">' +
          '<span class="text-content"><span class="username">' + escapeHtml(result.username) + "</span>" +
          (result.name ? '<span class="name">' + escapeHtml(result.name) + "</span>" : "") +
          "</span>" +
          (result.status
            ? Discourse.userStatus
              ? Discourse.userStatus.plainHtml(result.status, true)
              : '<span class="user-status"></span>'
            : "");
      } else if (result.isEmail) {
        out += iconHtml("envelope") + '<span class="text-content username">' + escapeHtml(result.username) + "</span>";
      } else if (result.isGroup) {
        out +=
          iconHtml("users") +
          '<span class="text-content"><span class="username">' + escapeHtml(result.name) +
          '</span><span class="name">' + escapeHtml(result.full_name) + "</span></span>";
      }
      out += "</a></li>";
    });
    return out + "</ul></div>";
  }

  // EmojiAutocompleteResults
  function renderEmojis(results, selected) {
    var out = '<div class="autocomplete ac-emoji"><ul>';
    results.forEach(function (result, index) {
      out += "<li><a" + (index === selected ? ' class="selected"' : "") + ' href=""><span class="text-content">';
      if (result.src) {
        out += '<img class="emoji" src="' + escapeHtml(result.src) + '"><span class="emoji-shortname">' +
          escapeHtml(result.code) + "</span>";
      } else {
        out += escapeHtml(result.label);
      }
      out += "</span></a></li>";
    });
    return out + "</ul></div>";
  }

  // RichHashtagAutocompleteResults
  function renderHashtags(results, selected) {
    var out = '<div class="autocomplete hashtag-autocomplete"><div class="hashtag-autocomplete__fadeout"><ul>';
    results.forEach(function (result, index) {
      out +=
        '<li class="hashtag-autocomplete__option"><a class="hashtag-autocomplete__link' +
        (index === selected ? " selected" : "") + '" href="">' + (result.iconHtml || "") +
        '<span class="hashtag-autocomplete__text">' + result.text +
        (result.secondary_text
          ? ' <span class="hashtag-autocomplete__meta-text">(' + escapeHtml(result.secondary_text) + ")</span>"
          : "") +
        "</span></a></li>";
    });
    return out + "</ul></div></div>";
  }

  // The hashtag types the client draws (hashtag-type-registry): their
  // color classes (generateColorCssClasses, added to
  // #hashtag-css-generator as each loads) and icons (generateIconHTML).
  var HASHTAG_TYPES = ["category", "tag", "channel"];
  var loadedHashtags = null;

  function emojiUrl(name) {
    return Discourse.emojiPicker ? Discourse.emojiPicker.url(name) : "";
  }

  function hashtagCss(result) {
    if (!loadedHashtags) {
      // The site's categories, preloaded with the page's style.
      loadedHashtags = {};
      var preloaded = document.getElementById("hashtag-css-generator");
      var ids = preloaded && preloaded.dataset.categoryIds;
      (ids ? ids.split(",") : []).forEach(function (id) {
        loadedHashtags["category-" + id] = true;
      });
    }
    var key = result.type + "-" + result.id;
    if (loadedHashtags[key] || !result.colors) {
      return;
    }
    loadedHashtags[key] = true;
    var rule = null;
    if (result.type === "category") {
      var color = result.colors.length === 1 ? result.colors[0] : result.colors[1];
      var parent = result.colors.length === 1 ? null : result.colors[0];
      if (result.style_type === "square") {
        rule = parent
          ? "background: linear-gradient(-90deg, #" + color + " 50%, #" + parent + " 50%);"
          : "background-color: #" + color + ";";
        rule = ".hashtag-color--category-" + result.id + " { " + rule + " }";
      } else if (result.style_type === "icon") {
        rule = ".hashtag-color--category-" + result.id + " { color: #" + color + "; }";
      }
    } else if (result.type === "channel") {
      rule = ".d-icon.hashtag-color--channel-" + result.id + " { color: #" + result.colors[0] + " }";
    }
    if (!rule) {
      return;
    }
    var style = document.getElementById("hashtag-css-generator");
    if (!style) {
      style = document.createElement("style");
      style.id = "hashtag-css-generator";
      style.innerHTML = ".hashtag-category-square { background-color: var(--primary-medium); }";
      document.head.appendChild(style);
    }
    style.innerHTML += "\n" + rule;
  }

  function hashtagIcon(result) {
    var color = "hashtag-color--" + result.type + "-" + result.id;
    if (result.type === "category") {
      var inner = "";
      if (result.style_type === "icon" && result.icon) {
        inner = iconHtml(result.icon);
      }
      if (result.style_type === "emoji" && result.emoji) {
        inner =
          '<img width="20" height="20" src="' + escapeHtml(emojiUrl(result.emoji)) +
          '" title="' + escapeHtml(result.emoji) + '" alt="' + escapeHtml(result.emoji) + '" class="emoji">';
      }
      return '<span class="hashtag-category-' + escapeHtml(result.style_type) + " " + color + '">' + inner + "</span>";
    }
    if (result.type === "channel" && result.emoji) {
      return (
        '<img width="20" height="20" src="' + escapeHtml(emojiUrl(result.emoji)) +
        '" title="' + escapeHtml(result.emoji) + '" alt="' + escapeHtml(result.emoji) + '" class="emoji">'
      );
    }
    return iconHtml(result.icon, color);
  }

  // emojiUnescape(escapeExpression(text)), through the emoji picker's data
  // where the page has loaded it.
  function unescapeText(text) {
    var escaped = escapeHtml(text);
    if (!Discourse.emojiPicker) {
      return Promise.resolve(escaped);
    }
    return Discourse.emojiPicker.unescape(escaped, { shortcuts: true }).catch(function () {
      return escaped;
    });
  }

  // lib/hashtag-autocomplete: searched 250ms (INPUT_DELAY) after typing
  // stops, the empty term too for the defaults, cached for 30 seconds.
  var hashtagState = { cache: {}, cacheTime: 0, holder: {} };

  function hashtagSearch(term, order) {
    var s = hashtagState;
    if (Date.now() - s.cacheTime > 30000) {
      s.cache = {};
    }
    if (s.cache[term]) {
      return s.cache[term];
    }
    return debounced(s.holder, 250, function (signal) {
      var query = new URLSearchParams({ term: term });
      // Only the types the client can draw.
      order
        .filter(function (type) {
          return HASHTAG_TYPES.indexOf(type) !== -1;
        })
        .forEach(function (type) {
          query.append("order[]", type);
        });
      return getJson(basePath() + "/hashtags/search.json?" + query.toString(), signal).then(function (response) {
        var results = response.results;
        if (!results) {
          return CANCELLED;
        }
        return Promise.all(
          results.map(function (result) {
            return unescapeText(result.text).then(function (text) {
              result.text = text;
              if (HASHTAG_TYPES.indexOf(result.type) !== -1) {
                hashtagCss(result);
                result.iconHtml = hashtagIcon(result);
              }
              return result;
            });
          })
        ).then(function (all) {
          s.cache[term] = all;
          s.cacheTime = Date.now();
          return all;
        });
      });
    });
  }

  // hashtagTriggerRule: neither in code nor in a link.
  function hashtagTriggerRule(context) {
    return Promise.resolve(context.inCodeBlock()).then(function (inCode) {
      if (inCode) {
        return false;
      }
      return Promise.resolve(context.inLink ? context.inLink() : false).then(function (inLink) {
        return !inLink;
      });
    });
  }

  // hashtagAutocompleteOptions(order, options): the # autocomplete over
  // the context's hashtag types in their order, `extra` over its options.
  function hashtagOptions(order, extra) {
    return Object.assign(
      {
        key: "#",
        render: renderHashtags,
        autoSelectFirstSuggestion: true,
        transformComplete: function (obj) {
          return obj.ref;
        },
        dataSource: function (term) {
          if (/\s/.test(term)) {
            return null;
          }
          return hashtagSearch(term, order || []);
        },
        triggerRule: function (_, context) {
          return hashtagTriggerRule(context);
        },
      },
      extra || {}
    );
  }

  Discourse.autocomplete = {
    setup: setup,
    textareaHandler: textareaHandler,
    inCodeBlock: inCodeBlock,
    caretCoordinates: caretCoordinates,
    userSearch: userSearch,
    resetUserSearchCache: resetUserSearchCache,
    validateSearchResult: validateSearchResult,
    indexUserStatuses: indexUserStatuses,
    hashtagOptions: hashtagOptions,
    renderUsers: renderUsers,
    renderEmojis: renderEmojis,
    renderHashtags: renderHashtags,
    EMOJI_ALLOWED_PRECEDING: EMOJI_ALLOWED_PRECEDING,
    SKIP: SKIP,
    CANCELLED: CANCELLED,
    isOpen: function () {
      return !!document.querySelector(".autocomplete");
    },
  };
})();
