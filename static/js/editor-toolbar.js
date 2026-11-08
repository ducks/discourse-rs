// A d-editor's button bar (the composer's, the profile's About me), shipped
// in front of composer.js: lib/composer/toolbar.ts's Toolbar (bold, italic,
// the heading menu, link, blockquote, code, the list menu, their active
// states and shortcuts), d-editor's newToolbarEvent and keymap (the
// buttons' shortcuts, Tab and Shift+Tab indenting), and the toolbar half of
// lib/textarea-text-manipulation.ts for the markdown editor. The rich
// editor's TextManipulation is rich-editor/text-manipulation.js; the bar
// acts through whichever editor is open.
//
//   Discourse.editorToolbar.textarea(textarea, { i18n, siteSettings })
//       the textarea's TextManipulation
//   Discourse.editorToolbar.bind(host, {
//     textManipulation,  // () -> the open editor's TextManipulation
//     i18n,              // (key, params) -> string
//     actions,           // { link, upload, emoji }: a button's
//                        // sendAction, given the toolbar event
//     listOptions,       // the plugins' list menu options
//                        // (pluginListOptions)
//   }) -> { refresh(state), keymap, keydown(event), event(trimLeading) }
//       draws the bar's active states and the heading icon (refresh, after
//       each of the editor's transactions), opens the menus, and runs a
//       button for its click or shortcut: `keymap` is d-editor's for the
//       rich editor (createEditor's keymap option), `keydown` the same
//       keys in the textarea.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.editorToolbar) {
    return;
  }

  var apple = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent || "");
  // PLATFORM_KEY_MODIFIER
  var MOD = apple ? "meta" : "ctrl";
  var FOUR_SPACES_INDENT = "4-spaces-indent";
  var OP = { NONE: 0, REMOVED: 1, ADDED: 2 };

  // Our head can be a static string or a function that returns a string
  // based on input (like for numbered lists).
  function getHead(head, prev) {
    if (typeof head === "string") {
      return [head, head.length];
    }
    return getHead(head(prev));
  }

  // insertAtTextarea: as typing would, so the browser's undo has it.
  function insertAtTextarea(textarea, start, end, text) {
    textarea.setSelectionRange(start, end);
    textarea.focus();
    var done =
      start !== end && text === ""
        ? document.execCommand("delete", false)
        : document.execCommand("insertText", false, text);
    if (!done) {
      textarea.setRangeText(text, start, end, "end");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    }
  }

  // TextareaTextManipulation, what the toolbar uses of it.
  function TextareaTextManipulation(textarea, opts) {
    this.textarea = textarea;
    this.i18n = opts.i18n;
    this.siteSettings = opts.siteSettings || {};
    this.allowPreview = true;
    this.state = {};
  }

  var T = TextareaTextManipulation.prototype;

  Object.defineProperty(T, "value", {
    get: function () {
      return this.textarea.value;
    },
  });

  // ensures textarea scroll position is correct
  T.blurAndFocus = function () {
    this.textarea.blur();
    this.textarea.focus({ preventScroll: true });
  };

  T.focus = function () {
    this.textarea.focus({ preventScroll: true });
  };

  T.getSelected = function (trimLeading, opts) {
    var value = this.value;
    var start = this.textarea.selectionStart;
    var end = this.textarea.selectionEnd;

    // trim trailing spaces cause **test ** would be invalid
    while (end > start && /\s/.test(value.charAt(end - 1))) {
      end--;
    }

    if (trimLeading) {
      // trim leading spaces cause ** test** would be invalid
      while (end > start && /\s/.test(value.charAt(start))) {
        start++;
      }
    }

    var selected = {
      start: start,
      end: end,
      value: value.substring(start, end),
      pre: value.slice(0, start),
      post: value.slice(end),
    };
    if (opts && opts.lineVal) {
      selected.lineVal = value.split("\n")[value.slice(0, this.textarea.selectionStart).split("\n").length - 1];
    }
    return selected;
  };

  T.selectText = function (from, length, opts) {
    opts = opts || { scroll: true };
    this.textarea.selectionStart = from;
    this.textarea.selectionEnd = from + length;
    if (opts.scroll === true || typeof opts.scroll === "number") {
      var oldScrollPos = typeof opts.scroll === "number" ? opts.scroll : this.textarea.scrollTop;
      this.textarea.focus();
      this.textarea.scrollTop = oldScrollPos;
    }
  };

  T._insertAt = function (start, end, text) {
    insertAtTextarea(this.textarea, start, end, text);
  };

  T.replaceText = function (oldVal, newVal, opts) {
    opts = opts || {};
    var at = this.value.indexOf(oldVal);
    if (at === -1) {
      return;
    }
    var start = this.textarea.selectionStart;
    var end = this.textarea.selectionEnd;
    this._insertAt(at, at + oldVal.length, newVal);
    if ((opts.forceFocus || this.textarea === document.activeElement) && !opts.skipNewSelection) {
      // determinePostReplaceSelection, for a needle before or around the
      // selection.
      var diff = newVal.length - oldVal.length;
      var moved = function (pos) {
        return pos >= at + oldVal.length ? pos + diff : Math.min(pos, at + newVal.length);
      };
      this.selectText(moved(start), moved(end) - moved(start));
    }
  };

  T.applySurroundSelection = function (head, tail, exampleKey, opts) {
    this.applySurround(this.getSelected(), head, tail, exampleKey, opts);
  };

  T.applySurround = function (sel, head, tail, exampleKey, opts) {
    if (opts && opts.wholeLine) {
      sel = this._expandToLines(sel);
    }

    var pre = sel.pre;
    var post = sel.post;
    var tlen = tail.length;
    var h;

    if (sel.start === sel.end) {
      if (tlen === 0) {
        return;
      }
      h = getHead(head);
      var example = this.i18n("composer." + exampleKey);
      this._insertAt(sel.start, sel.end, h[0] + example + tail);
      this.selectText(pre.length + h[1], example.length);
    } else if (opts && opts.wholeLine) {
      this._applyWholeLineSurround(sel, head, tail, opts);
    } else if (opts && !opts.multiline) {
      h = getHead(head);
      var hval = h[0];
      var hlen = h[1];

      if (opts.useBlockMode && sel.value.split("\n").length > 1) {
        hval += "\n";
        hlen += 1;
        tail = "\n" + tail;
      }

      if (pre.slice(-hlen) === hval && post.slice(0, tail.length) === tail) {
        // Already wrapped in the surround. Remove it.
        this._insertAt(sel.start - hlen, sel.end + tail.length, sel.value);
        this.selectText(sel.start - hlen, sel.value.length);
      } else {
        this._insertAt(sel.start, sel.end, hval + sel.value + tail);
        this.selectText(sel.start + hlen, sel.value.length);
      }
    } else {
      var lines = sel.value.split("\n");
      h = getHead(head);
      if (lines.length === 1 && pre.slice(-tlen) === tail && post.slice(0, h[1]) === h[0]) {
        // Already wrapped in the surround. Remove it.
        this._insertAt(sel.start - h[1], sel.end + tlen, sel.value);
        this.selectText(sel.start - h[1], sel.value.length);
      } else {
        var contents = this._getMultilineContents(lines, head, h[0], h[1], tail, tlen, opts);

        this._insertAt(sel.start, sel.end, contents);
        if (lines.length === 1 && tlen > 0) {
          this.selectText(sel.start + h[1], sel.value.length);
        } else {
          this.selectText(sel.start, contents.length);
        }
      }
    }
  };

  T._applyWholeLineSurround = function (sel, head, tail, opts) {
    var h = getHead(head);
    var hval = h[0];
    var hlen = h[1];
    var lines = sel.value.split("\n");
    var formattedLines = lines.filter(function (line) {
      return opts.applyEmptyLines || line.length > 0;
    });
    var removing =
      formattedLines.length > 0 &&
      formattedLines.every(function (line) {
        return line.startsWith(hval) && line.endsWith(tail);
      });

    var contents = lines
      .map(function (line) {
        if (!opts.applyEmptyLines && line.length === 0) {
          return line;
        }
        if (removing) {
          return line.slice(hlen, tail.length ? -tail.length : undefined);
        }
        var content = line;
        if (hval) {
          content = content.split(hval).join("");
        }
        if (tail) {
          content = content.split(tail).join("");
        }
        return hval + content + tail;
      })
      .join("\n");

    this._insertAt(sel.start, sel.end, contents);

    if (lines.length === 1) {
      this.selectText(
        sel.start + (removing ? 0 : hlen),
        removing ? contents.length : contents.length - hlen - tail.length
      );
    } else {
      this.selectText(sel.start, contents.length);
    }
  };

  T._expandToLines = function (sel) {
    var value = this.value;
    var start = value.lastIndexOf("\n", sel.start - 1) + 1;
    var endAnchor = sel.end > sel.start && value[sel.end - 1] === "\n" ? sel.end - 1 : sel.end;
    var nextNewline = value.indexOf("\n", endAnchor);
    var end = nextNewline === -1 ? value.length : nextNewline;

    return {
      start: start,
      end: end,
      value: value.slice(start, end),
      pre: value.slice(0, start),
      post: value.slice(end),
      lineVal: value.slice(start, end),
    };
  };

  // perform the same operation over many lines of text
  T._getMultilineContents = function (lines, head, hval, hlen, tail, tlen, opts) {
    var operation = OP.NONE;
    var applyEmptyLines = opts && opts.applyEmptyLines;
    var next = function () {
      var h = getHead(head, hval);
      hval = h[0];
      hlen = h[1];
    };

    return lines
      .map(function (l) {
        if (!applyEmptyLines && l.length === 0) {
          return l;
        }

        if (operation !== OP.ADDED && l.slice(0, hlen) === hval && (tlen === 0 || l.slice(-tlen) === tail)) {
          operation = OP.REMOVED;
          var removed = tlen === 0 ? l.slice(hlen) : l.slice(hlen, -tlen);
          next();
          return removed;
        } else if (operation === OP.NONE) {
          operation = OP.ADDED;
        } else if (operation === OP.REMOVED) {
          return l;
        }

        var result = hval + l + tail;
        next();
        return result;
      })
      .join("\n");
  };

  T.applyLink = function (url) {
    var sel = this.getSelected();
    if (sel.start === sel.end) {
      return;
    }
    this._insertAt(sel.start, sel.end, "[" + sel.value + "](" + url + ")");
    this.blurAndFocus();
  };

  T.addText = function (sel, text, options) {
    if (options && options.ensureSpace) {
      if (sel.pre.length > 0 && !/\s$/.test(sel.pre)) {
        text = " " + text;
      }
      if (sel.post.length > 0 && !/^\s/.test(sel.post)) {
        text = text + " ";
      }
    }

    this._insertAt(sel.start, sel.end, text);
    this.blurAndFocus();
  };

  T.insertText = function (text, options) {
    this.addText(this.getSelected(), text, options);
  };

  T.applyList = function (sel, head, exampleKey, opts) {
    var collapsedSelection = sel.start === sel.end;

    if (sel.value.indexOf("\n") !== -1) {
      this.applySurround(sel, head, "", exampleKey, opts);
      return;
    }

    var h = getHead(head);
    var hval = h[0];
    var hlen = h[1];

    var absorbedHead = false;
    if (hlen > 0 && sel.pre.slice(sel.pre.lastIndexOf("\n") + 1) === hval) {
      sel.start -= hlen;
      sel.pre = sel.pre.slice(0, -hlen);
      sel.value = hval + sel.value;
      absorbedHead = true;
    }

    if (collapsedSelection && !absorbedHead) {
      sel.value = this.i18n("composer." + exampleKey);
    }

    // Special handling for markdown headings starting with #, they are
    // "list-like" in that they have a character at the start and a level,
    // rather than having a surrounding format.
    var number;
    var removedHead = false;
    if (hval.indexOf("#") !== -1) {
      var currentHeadingLevel = sel.value.search(/[^#]/);

      // Remove existing heading level if same as the new one, mirrors
      // list behavior.
      if (sel.value.startsWith(hval) && currentHeadingLevel + 1 === hlen) {
        number = sel.value.slice(hlen);
        removedHead = true;
      } else if (currentHeadingLevel > 0) {
        // Replace the existing heading level with the new one.
        number = hval + sel.value.slice("#".repeat(currentHeadingLevel).length + 1);
      } else {
        number = hval + sel.value;
      }
    } else if (sel.value.startsWith(hval)) {
      // Remove existing list item if it's the same as the new head, e.g.
      // if a line is "* list item", then it converts it to "list item"
      number = sel.value.slice(hlen);
      removedHead = true;
    } else {
      number = hval + sel.value;
    }

    var preNewlines = sel.pre.trim() && "\n\n";
    var postNewlines = sel.post.trim() && "\n\n";

    var textToInsert = preNewlines + number + postNewlines;

    var preChars = sel.pre.length - sel.pre.trimEnd().length;
    var postChars = sel.post.length - sel.post.trimStart().length;

    this._insertAt(sel.start - preChars, sel.end + postChars, textToInsert);

    var headSelectionOffset = ((opts && opts.excludeHeadInSelection) || collapsedSelection) && !removedHead ? hval.length : 0;
    this.selectText(
      sel.start + (preNewlines.length - preChars) + headSelectionOffset,
      number.length - headSelectionOffset
    );
  };

  T.applyHeading = function (sel, level) {
    if (level > 0) {
      this.applyList(sel, "#".repeat(level) + " ", "heading_text", { excludeHeadInSelection: true });
      return;
    }
    // Remove heading when the Paragraph level (0) is selected.
    var currentHeadingLevel = sel.lineVal.search(/[^#]/);
    if (currentHeadingLevel >= 0) {
      // When you apply the list with the same head chars, then they are
      // removed, so we can use the same function.
      this.applyList(sel, "#".repeat(currentHeadingLevel) + " ", "heading_text");
    }
  };

  T.formatCode = function () {
    var sel = this.getSelected("", { lineVal: true });
    var selValue = sel.value;
    var hasNewLine = selValue.indexOf("\n") !== -1;
    var isBlankLine = sel.lineVal.trim().length === 0;
    var isFourSpacesIndent = this.siteSettings.code_formatting_style === FOUR_SPACES_INDENT;

    if (!hasNewLine) {
      if (selValue.length === 0 && isBlankLine) {
        if (isFourSpacesIndent) {
          var example = this.i18n("composer.code_text");
          this._insertAt(sel.start, sel.end, "    " + example);
          return this.selectText(sel.pre.length + 4, example.length);
        }
        return this.applySurround(sel, "```\n", "\n```", "paste_code_text");
      }
      return this.applySurround(sel, "`", "`", "code_title");
    }
    if (isFourSpacesIndent) {
      return this.applySurround(sel, "    ", "", "code_text");
    }
    var preNewline = sel.pre.slice(-1) !== "\n" && sel.pre !== "" ? "\n" : "";
    var postNewline = sel.post[0] !== "\n" ? "\n" : "";
    return this.addText(sel, preNewline + "```\n" + sel.value + "\n```" + postNewline);
  };

  T.emojiSelected = function (code) {
    var selected = this.getSelected();
    var captures = selected.pre.match(/\B:([\p{L}\p{N}_]*)$/u);

    if (!captures) {
      this.addText(selected, (/\S$/.test(selected.pre) ? " :" : ":") + code + ":");
    } else {
      this._insertAt(selected.start - captures[1].length, selected.end, code + ":");
    }
  };

  // Removes the char from str up until the limit, or until a character
  // that is not the one given.
  function deindentLine(str, char, limit) {
    var eaten = 0;
    for (var i = 0; i < str.length; i++) {
      if (eaten < limit && str[i] === char) {
        eaten += 1;
      } else {
        return str.slice(eaten);
      }
    }
    return str;
  }

  T.indentSelection = function (direction) {
    if (direction !== "left" && direction !== "right") {
      return;
    }

    var selected = this.getSelected(null, { lineVal: true });
    var lineVal = selected.lineVal;
    var value = selected.value;

    // A guess at whether the text indents with tabs or spaces, spaces when
    // there are none.
    var tabs = (value.match(/^\t/gm) || []).length;
    var spaces = (value.match(/^ /gm) || []).length;
    var indentationSteps = tabs > spaces ? 1 : 2;
    var indentationChar = tabs > spaces ? "\t" : " ";

    // The whole first line's indentation goes along, wherever the
    // selection starts in it.
    var indentationRegexp = new RegExp("^" + indentationChar + "+");
    var lineStartsWithIndentationChar = lineVal.match(indentationRegexp);
    var indentationCharsBeforeSelection = value.match(indentationRegexp);
    if (lineStartsWithIndentationChar) {
      var charsToSubtract = indentationCharsBeforeSelection ? indentationCharsBeforeSelection[0] : "";
      value = lineStartsWithIndentationChar[0].replace(charsToSubtract, "") + value;
    }

    var newValue = value
      .split("\n")
      .map(function (line) {
        return direction === "left"
          ? deindentLine(line, indentationChar, indentationSteps)
          : Array(indentationSteps + 1).join(indentationChar) + line;
      })
      .join("\n");

    if (newValue.trim() !== "") {
      this.replaceText(value, newValue, { skipNewSelection: true });
      this.selectText(this.value.indexOf(newValue), newValue.length);
      return true;
    }
  };

  T.toggleDirection = function () {
    var currentDir = this.textarea.getAttribute("dir") || document.documentElement.dir || "ltr";
    this.textarea.setAttribute("dir", currentDir === "ltr" ? "rtl" : "ltr");
    this.textarea.focus();
  };

  // d-editor's newToolbarEvent over a TextManipulation.
  function newToolbarEvent(tm, trimLeading) {
    var selected = tm.getSelected(trimLeading, { lineVal: true });
    return {
      commands: tm.commands,
      state: tm.state,
      selected: selected,
      selectText: function (from, length) {
        tm.selectText(from, length, { scroll: false });
      },
      applySurround: function (head, tail, exampleKey, opts) {
        tm.applySurround(selected, head, tail, exampleKey, opts);
      },
      applyList: function (head, exampleKey, opts) {
        tm.applyList(selected, head, exampleKey, opts);
      },
      applyHeading: function (level, exampleKey) {
        tm.applyHeading(selected, level, exampleKey);
      },
      formatCode: function () {
        return tm.formatCode();
      },
      addText: function (text) {
        tm.addText(selected, text);
      },
      applyLink: function (url) {
        tm.applyLink(url);
      },
      getText: function () {
        return tm.value !== undefined ? tm.value : tm.convertToMarkdown(tm.view.state.doc);
      },
      toggleDirection: function () {
        tm.toggleDirection();
      },
      replaceText: function (oldVal, newVal, opts) {
        tm.replaceText(oldVal, newVal, opts);
      },
    };
  }

  // The plugins' list menu options (addComposerToolbarPopupMenuOption with
  // menu "list"), by the plugins on: checklist's.
  function pluginListOptions(plugins) {
    if (plugins.indexOf("checklist") === -1) {
      return [];
    }
    return [
      {
        name: "list-checklist",
        icon: "list-check",
        label: "checklist.composer.checklist",
        showActiveIcon: true,
        active: function (state) {
          return state.inCheckList;
        },
        action: function (e) {
          if (e.commands && e.commands.toggleChecklist) {
            e.commands.toggleChecklist();
          } else {
            e.applyList("- [ ] ", "list_item");
          }
        },
      },
    ];
  }

  // Toolbar#getListPopupMenuOptions
  var LIST_OPTIONS = [
    {
      name: "list-bullet",
      icon: "list-ul",
      label: "composer.ulist_title",
      shortcut: "Shift+8",
      showActiveIcon: true,
      active: function (state) {
        return state.inBulletList;
      },
      action: function (e) {
        if (!e.commands || !e.commands.toggleBulletList || !e.commands.toggleBulletList()) {
          e.applyList("* ", "list_item");
        }
      },
    },
    {
      name: "list-ordered",
      icon: "list-ol",
      label: "composer.olist_title",
      shortcut: "Shift+7",
      showActiveIcon: true,
      active: function (state) {
        return state.inOrderedList;
      },
      action: function (e) {
        if (!e.commands || !e.commands.toggleOrderedList || !e.commands.toggleOrderedList()) {
          e.applyList(function (i) {
            return !i ? "1. " : parseInt(i, 10) + 1 + ". ";
          }, "list_item");
        }
      },
    },
  ];

  function headingOptions() {
    var options = [];
    [1, 2, 3, 4].forEach(function (level) {
      options.push({
        name: "heading-" + level,
        icon: "discourse-h" + level,
        label: "composer.heading_level_n",
        title: "composer.heading_level_n_title",
        params: { levelNumber: level },
        shortcut: "Alt+" + level,
        showActiveIcon: true,
        active: function (state) {
          return !!state.inHeading && state.inHeadingLevel === level;
        },
        action: function (e) {
          e.applyHeading(level, "heading");
        },
      });
    });
    options.push({
      name: "heading-paragraph",
      icon: "discourse-text",
      label: "composer.heading_level_paragraph",
      title: "composer.heading_level_paragraph_title",
      shortcut: "Alt+0",
      showActiveIcon: true,
      active: function (state) {
        return state.inParagraph && !state.inSmall;
      },
      action: function (e) {
        e.applyHeading(0, "heading");
      },
    });
    options.push({
      name: "heading-small",
      icon: "discourse-text",
      label: "composer.heading_level_small",
      title: "composer.heading_level_small_title",
      shortcut: "Alt+5",
      showActiveIcon: true,
      active: function (state) {
        return state.inSmall;
      },
      action: function (e) {
        if (e.commands && e.commands.toggleSmall) {
          e.commands.toggleSmall();
        } else {
          e.applySurround("<small>", "</small>", "heading_level_small_text", { multiline: true, wholeLine: true });
        }
      },
    });
    return options;
  }

  // The Toolbar's buttons, by id (the bar's data-action). `actions` are
  // the host's sendActions.
  function buttons(actions, listOptions) {
    var heading = headingOptions();
    var list = LIST_OPTIONS.concat(listOptions || []);
    return {
      bold: {
        shortcut: "B",
        trimLeading: true,
        perform: function (e) {
          e.applySurround("**", "**", "bold_text");
        },
        active: function (state) {
          return state.inBold;
        },
      },
      italic: {
        shortcut: "I",
        trimLeading: true,
        perform: function (e) {
          e.applySurround("*", "*", "italic_text");
        },
        active: function (state) {
          return state.inItalic;
        },
      },
      heading: {
        active: function (state) {
          if (state.inSmall) {
            return true;
          }
          return !!state.inHeading && !(state.inHeadingLevel > 4);
        },
        icon: function (state) {
          if (!state.inHeading || state.inHeadingLevel > 4) {
            return "discourse-text";
          }
          return "discourse-h" + state.inHeadingLevel;
        },
        popupMenu: heading,
      },
      link: {
        shortcut: "K",
        trimLeading: true,
        sendAction: actions.link,
        active: function (state) {
          return state.inLink;
        },
      },
      blockquote: {
        shortcut: "Shift+9",
        perform: function (e) {
          e.applyList("> ", "blockquote_text", { applyEmptyLines: true, multiline: true });
        },
        active: function (state) {
          return state.inBlockquote;
        },
      },
      code: {
        shortcut: "E",
        trimLeading: true,
        perform: function (e) {
          e.formatCode();
        },
        active: function (state) {
          return state.inCode || state.inCodeBlock;
        },
      },
      upload: { sendAction: actions.upload },
      emoji: { sendAction: actions.emoji },
      list: {
        active: function (state) {
          return list.some(function (option) {
            return option.active(state);
          });
        },
        icon: function (state) {
          var active = list.find(function (option) {
            return option.active(state);
          });
          return active ? active.icon : "list-ul";
        },
        popupMenu: list,
      },
    };
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function svg(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' + name + " svg-icon fa-width-auto" + (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  // formatShortcut's keycaps for `mod+<shortcut>`: Control, Alt, Shift,
  // then Command, then the key.
  function shortcutKbd(shortcut, i18n) {
    if (!window.matchMedia("(pointer: fine)").matches) {
      return "";
    }
    var keys = shortcut.split("+");
    var key = keys.pop();
    var caps = [];
    if (!apple) {
      caps.push([i18n("shortcut_modifier_key.ctrl"), false]);
    }
    if (keys.indexOf("Alt") !== -1) {
      caps.push(apple ? ["⌥", true] : [i18n("shortcut_modifier_key.alt"), false]);
    }
    if (keys.indexOf("Shift") !== -1) {
      caps.push(apple ? ["⇧", true] : [i18n("shortcut_modifier_key.shift"), false]);
    }
    if (apple) {
      caps.push(["⌘", true]);
    }
    caps.push([key.length === 1 ? key.toUpperCase() : key, false]);
    return (
      '<kbd aria-hidden="true" class="d-shortcut shortcut" dir="ltr">' +
      caps
        .map(function (cap) {
          return '<kbd class="d-shortcut__key' + (cap[1] ? " d-shortcut__key--glyph" : "") + '">' + escapeHtml(cap[0]) + "</kbd>";
        })
        .join("") +
      "</kbd>"
    );
  }

  // ToolbarPopupMenuOptions' DDropdownMenu
  function menuContent(options, state, i18n) {
    var html = '<ul class="dropdown-menu">';
    options.forEach(function (option) {
      var label = i18n(option.label, option.params);
      var title = option.title ? i18n(option.title, option.params) : label;
      html +=
        '<li class="dropdown-menu__item"><button class="btn no-text btn-icon' +
        (option.active(state) ? " --active" : "") +
        '" title="' + escapeHtml(title) + '" data-name="' + escapeHtml(option.name) + '" type="button">' +
        svg(option.icon) +
        '<span class="d-button-label"><span class="d-button-label__text">' + escapeHtml(label) + "</span>" +
        (option.shortcut ? shortcutKbd(option.shortcut, i18n) : "") +
        (option.showActiveIcon ? svg("check", "d-button-label__active-icon") : "") +
        "</span></button></li>";
    });
    return html + "</ul>";
  }

  // `mod+<shortcut>` as a key binding: { ctrl/meta, alt, shift, code }.
  function binding(shortcut) {
    var keys = shortcut.toLowerCase().split("+");
    var key = keys.pop();
    return {
      name: (MOD + "+" + shortcut).toLowerCase(),
      alt: keys.indexOf("alt") !== -1,
      shift: keys.indexOf("shift") !== -1,
      code: /^[0-9]$/.test(key) ? "Digit" + key : "Key" + key.toUpperCase(),
    };
  }

  function bind(host, opts) {
    var defs = buttons(opts.actions || {}, opts.listOptions);
    var bar = host.querySelector(".d-editor-button-bar");
    var i18n = opts.i18n;

    function tm() {
      return opts.textManipulation();
    }

    // ToolbarBase#addButton's action
    function run(def, option) {
      var e = newToolbarEvent(tm(), def.trimLeading);
      if (option) {
        option.action(e);
      } else if (def.sendAction) {
        def.sendAction(e);
      } else if (def.perform) {
        def.perform(e);
      }
    }

    function openMenu(trigger, def) {
      var state = tm().state;
      var menu = Discourse.float.menu(trigger, {
        identifier: "toolbar-menu__" + trigger.dataset.action,
        groupIdentifier: "toolbar-menu",
        mainClass: "toolbar-menu__" + trigger.dataset.action + "-content toolbar-popup-menu-options",
        placement: "bottom",
        fallbackPlacements: ["top"],
        offset: 5,
        modalForMobile: true,
        content: menuContent(def.popupMenu, state, i18n),
      });
      var content = menu && menu.content;
      if (!content) {
        return;
      }
      content.addEventListener("click", function (event) {
        var button = event.target.closest(".dropdown-menu__item button");
        if (!button) {
          return;
        }
        var option = def.popupMenu.find(function (o) {
          return o.name === button.dataset.name;
        });
        Promise.resolve(menu.close()).then(function () {
          if (option) {
            run(def, option);
          }
        });
      });
    }

    if (bar) {
      bar.addEventListener("click", function (event) {
        var button = event.target.closest(".toolbar__button[data-action]");
        var def = button && defs[button.dataset.action];
        if (!def) {
          return;
        }
        event.stopPropagation();
        if (def.popupMenu) {
          openMenu(button, def);
        } else {
          run(def);
        }
      });
      // preventFocus: the editor keeps the focus and its selection.
      bar.addEventListener("mousedown", function (event) {
        var button = event.target.closest(".toolbar__button[data-action]");
        if (button && defs[button.dataset.action] && !defs[button.dataset.action].popupMenu) {
          event.preventDefault();
        }
      });
    }

    // The shortcuts: the buttons', then their menus' options.
    var shortcuts = [];
    Object.keys(defs).forEach(function (id) {
      var def = defs[id];
      if (def.shortcut) {
        shortcuts.push({ binding: binding(def.shortcut), def: def });
      }
      (def.popupMenu || []).forEach(function (option) {
        if (option.shortcut) {
          shortcuts.push({ binding: binding(option.shortcut), def: def, option: option });
        }
      });
    });

    // d-editor's keymap, for the rich editor: a handler returning false
    // has handled the key.
    var keymap = {};
    shortcuts.forEach(function (s) {
      keymap[s.binding.name] = function () {
        run(s.def, s.option);
        return false;
      };
    });
    // indentSelection says whether it indented
    keymap.tab = function () {
      return !tm().indentSelection("right");
    };
    keymap["shift+tab"] = function () {
      return !tm().indentSelection("left");
    };

    // The same keys in the textarea.
    function keydown(event) {
      if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
        if (tm().indentSelection(event.shiftKey ? "left" : "right")) {
          event.preventDefault();
        }
        return;
      }
      var mod = apple ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
      if (!mod) {
        return;
      }
      var found = shortcuts.find(function (s) {
        return s.binding.code === event.code && s.binding.shift === event.shiftKey && s.binding.alt === event.altKey;
      });
      if (found) {
        event.preventDefault();
        run(found.def, found.option);
      }
    }

    // ComposerToolbarButtons' isButtonActive and the buttons' icons
    function refresh(state) {
      state = state || {};
      if (!bar) {
        return;
      }
      Array.prototype.forEach.call(bar.querySelectorAll(".toolbar__button[data-action]"), function (button) {
        var def = defs[button.dataset.action];
        if (!def) {
          return;
        }
        button.classList.toggle("--active", !!(def.active && def.active(state)));
        if (typeof def.icon === "function") {
          var name = def.icon(state);
          var icon = button.querySelector(".d-icon");
          if (icon && !icon.classList.contains("d-icon-" + name)) {
            icon.outerHTML = svg(name);
          }
        }
      });
    }

    return {
      refresh: refresh,
      keymap: keymap,
      keydown: keydown,
      event: function (trimLeading) {
        return newToolbarEvent(tm(), trimLeading);
      },
    };
  }

  Discourse.editorToolbar = {
    textarea: function (textarea, opts) {
      return new TextareaTextManipulation(textarea, opts);
    },
    bind: bind,
    newToolbarEvent: newToolbarEvent,
    pluginListOptions: pluginListOptions,
  };
})();
