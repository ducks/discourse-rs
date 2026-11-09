// lib/text-manipulation.ts: the rich editor's TextManipulation, what the
// composer's toolbar, its menus and its keys act through (d-editor's
// newToolbarEvent): getSelected, applySurround, applyList, applyHeading,
// formatCode, addText, insertBlock, insertText, applyLink, emojiSelected,
// indentSelection, selectText, replaceText, toggleDirection, and `state`,
// what the toolbar draws as active (updateState after each transaction).
//
// Markup is applied through the parser, as the reference does: a probe of
// the head and tail around a private-use character tells a mark (toggled
// with toggleMark), a list or blockquote (wrapped and lifted), or else the
// markdown is parsed in the selection's place and what was selected
// selected again.
//
// The autocomplete handler (ProsemirrorAutocompleteHandler) is what
// d-editor's autocompletes (autocomplete.js, autocomplete()) read and
// complete through: a term completes as its markdown parses, into a
// mention, hashtag or emoji node. Not ported here: the placeholder
// handler, which the composer's own upload code is (docs/rich-editor.md).
import { lift, setBlockType, toggleMark, wrapIn } from "prosemirror-commands";
import { Fragment, Slice } from "prosemirror-model";
import { liftListItem, sinkListItem, wrapInList, wrapRangeInList } from "prosemirror-schema-list";
import { NodeSelection, Selection, TextSelection } from "prosemirror-state";
import { hasMark, inNode, isNodeActive } from "discourse/rich-editor/plugin-utils";

// A private-use character has no meaning in markdown, so it can't turn into
// syntax such as a checkbox or an emoji the way a letter could.
var PROBE_TEXT = "";

function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// Reuses a resolved head for the first line so function heads run once per
// line.
function headWithFirst(head, first) {
  return function (previous) {
    return previous === undefined ? first : typeof head === "function" ? head(previous) : head;
  };
}

function singleChild(content) {
  return content && content.childCount === 1 ? content.firstChild : null;
}

function flattenSelection(doc, from, to) {
  if (from === undefined) {
    from = 0;
  }
  if (to === undefined) {
    to = doc instanceof Fragment ? doc.size : doc.content.size;
  }
  var nodes = [];
  var segments = [];
  var size = 0;
  doc.nodesBetween(from, to, function (node, pos) {
    if (!node.isText && !node.isAtom) {
      return true;
    }
    if (node.isText) {
      var lower = Math.max(from - pos, 0);
      node = node.cut(lower, Math.min(to - pos, node.nodeSize));
      pos += lower;
    }
    nodes.push(node.mark([]));
    segments.push({ start: size, end: size + node.nodeSize, pos: pos });
    size += node.nodeSize;
    return false;
  });
  return { content: Fragment.fromArray(nodes), segments: segments };
}

function isPlainTextFragment(fragment, schema) {
  return fragment.content.every(function (node) {
    if (node.isText) {
      return node.marks.length === 0;
    }
    if (node.type === schema.nodes.hard_break) {
      return true;
    }
    if (node.type === schema.nodes.paragraph) {
      return isPlainTextFragment(node.content, schema);
    }
    return false;
  });
}

// The first block of a document parsed from markdown. Non-empty markdown
// always yields at least one block, which every caller here relies on.
function firstBlockOf(doc) {
  return doc.content.firstChild;
}

class ProsemirrorAutocompleteHandler {
  constructor(opts) {
    this.schema = opts.schema;
    this.view = opts.view;
    this.convertFromMarkdown = opts.convertFromMarkdown;
  }

  // The textual value of the selected text block
  getValue() {
    var $head = this.view.state.selection.$head;
    return (
      ($head.nodeBefore ? $head.nodeBefore.textContent : "") + ($head.nodeAfter ? $head.nodeAfter.textContent : "") ||
      " "
    );
  }

  // Replaces the term between start-end in the currently selected text block
  replaceTerm(start, end, term) {
    var node = this.view.state.selection.$head.nodeBefore;
    if (!node) {
      throw new Error("Autocomplete replaces a term before the caret");
    }
    var from = this.view.state.selection.from - node.nodeSize + start;
    var to = this.view.state.selection.from - node.nodeSize + end + 1;

    var tr = this.view.state.tr.replaceWith(from, to, firstBlockOf(this.convertFromMarkdown(term)).content);
    tr.insertText(" ", tr.selection.from);

    this.view.dispatch(tr);
  }

  // Gets the textual caret position within the selected text block
  getCaretPosition() {
    var node = this.view.state.selection.$head.nodeBefore;

    if (!node || !node.isText) {
      return 0;
    }

    return node.nodeSize;
  }

  getCaretCoords(start) {
    var node = this.view.state.selection.$head.nodeBefore;
    if (!node) {
      throw new Error("Autocomplete measures a term before the caret");
    }
    var pos = this.view.state.selection.from - node.nodeSize + start;
    var coords = this.view.coordsAtPos(pos);

    var rootRect = this.view.dom.getBoundingClientRect();

    return {
      left: coords.left - rootRect.left,
      top: coords.top - rootRect.top,
    };
  }

  inCodeBlock() {
    var schema = this.schema;
    var $from = this.view.state.selection.$from;

    var isInCodeBlock = $from.parent.type === schema.nodes.code_block;

    var hasCodeMark = $from.marks().some(function (mark) {
      return mark.type === schema.marks.code;
    });

    return Promise.resolve(isInCodeBlock || hasCodeMark);
  }

  inLink() {
    var schema = this.schema;
    var $from = this.view.state.selection.$from;

    return Promise.resolve(
      $from.marks().some(function (mark) {
        return mark.type === schema.marks.link;
      })
    );
  }
}

export default class ProsemirrorTextManipulation {
  // opts: schema, view, convertFromMarkdown, tryConvertFromMarkdown,
  // convertToMarkdown, splitNonEmptyLines, buildListNode, commands,
  // customState, i18n
  constructor(opts) {
    this.allowPreview = false;
    this.schema = opts.schema;
    this.view = opts.view;
    this.convertFromMarkdown = opts.convertFromMarkdown;
    this.tryConvertFromMarkdown = opts.tryConvertFromMarkdown;
    this.convertToMarkdown = opts.convertToMarkdown;
    this.splitNonEmptyLines = opts.splitNonEmptyLines;
    this.buildListNode = opts.buildListNode;
    this.commands = opts.commands;
    this.customState = opts.customState;
    this.i18n = opts.i18n;
    this.state = {};
    this.autocompletes = [];
    this.autocompleteHandler = new ProsemirrorAutocompleteHandler({
      schema: this.schema,
      view: this.view,
      convertFromMarkdown: this.convertFromMarkdown,
    });
  }

  // DAutocompleteModifier (autocomplete.js) on the editor's element;
  // autocomplete("destroy") removes them all.
  autocomplete(options) {
    if (options === "destroy") {
      this.autocompletes.forEach(function (ac) {
        ac.destroy();
      });
      this.autocompletes = [];
      return;
    }

    var ac = window.Discourse.autocomplete.setup(
      this.view.dom,
      Object.assign({}, options, { handler: this.autocompleteHandler })
    );
    this.autocompletes.push(ac);
    return ac;
  }

  getSelected(trimLeading) {
    var state = this.view.state;
    var to = state.selection.to;
    var from = state.selection.from;
    if (trimLeading && state.selection instanceof TextSelection) {
      var found = false;
      state.doc.nodesBetween(from, to, function (node, pos) {
        if (found) {
          return false;
        }
        if (node.isText) {
          var start = Math.max(from - pos, 0);
          var text = node.text.slice(start, to - pos);
          var whitespace = /^\s*/.exec(text)[0].length;
          from = pos + start + whitespace;
          found = whitespace < text.length;
        } else if (node.isInline) {
          found = true;
        }
        return !found;
      });
    }
    var selection = from === state.selection.from ? state.selection : TextSelection.create(state.doc, from, to);
    var value = this.convertToMarkdown(selection.content());

    // Document-absolute pre/post to match textarea semantics
    var pre = state.doc.textBetween(0, from, "\n", "\n");
    var post = state.doc.textBetween(to, state.doc.content.size, "\n", "\n");

    return { start: from, end: to, pre: pre, value: value, post: post };
  }

  focus() {
    this.view.focus();
  }

  blurAndFocus() {
    this.focus();
  }

  putCursorAtEnd() {
    var view = this.view;
    this.focus();
    setTimeout(function () {
      view.dispatch(view.state.tr.setSelection(Selection.atEnd(view.state.doc)).scrollIntoView());
    }, 0);
  }

  applySurroundSelection(head, tail, exampleKey, opts) {
    this.applySurround(this.getSelected(), head, tail, exampleKey, opts);
  }

  applySurround(sel, head, tail, exampleKey, opts) {
    this._restoreSelection(sel);
    var hval = typeof head === "function" ? head() : head;
    var needsBlockSpacing =
      opts &&
      opts.useBlockMode &&
      !opts.multiline &&
      !this.view.state.selection.empty &&
      this._selectedMarkdownOr(exampleKey).indexOf("\n") !== -1;
    var mark = needsBlockSpacing ? null : this._probeMark(hval, tail);

    if (mark) {
      toggleMark(mark.type, mark.attrs)(this.view.state, this.view.dispatch);
    } else if (opts && opts.multiline && !this.view.state.selection.empty) {
      this._applySurroundLines(headWithFirst(head, hval), tail, opts);
    } else {
      this._applySurroundFallback(hval, tail, exampleKey, opts);
    }

    this.focus();
  }

  applyLink(url) {
    var state = this.view.state;
    var selection = state.selection;
    if (selection.empty) {
      return;
    }
    this.view.dispatch(state.tr.addMark(selection.from, selection.to, state.schema.marks.link.create({ href: url })));
    this.focus();
  }

  addText(sel, text) {
    // assumes it returns a single block node
    var block = firstBlockOf(this.convertFromMarkdown(text));
    var content = block.type.name === "paragraph" ? block.content : block;

    this.view.dispatch(this.view.state.tr.replaceWith(sel.start, sel.end, content));

    this.focus();
  }

  insertBlock(block) {
    var doc = this.convertFromMarkdown(block);

    var tr = this.view.state.tr.replaceSelection(new Slice(doc.content, 0, 0));
    if (!tr.selection.$from.nodeAfter) {
      tr.setSelection(new TextSelection(tr.doc.resolve(tr.selection.from + 1)));
    }
    this.view.dispatch(tr);

    this.focus();
  }

  applyList(sel, head, exampleKey, opts) {
    this._restoreSelection(sel);
    var hval = typeof head === "function" ? head() : head;
    var nextHead = headWithFirst(head, hval);
    var doc = hval === null || hval === undefined ? null : this.tryConvertFromMarkdown(hval + PROBE_TEXT);
    var probe = singleChild(doc);
    var nodes = this.schema.nodes;

    if (!probe) {
      this._applyListFallback(nextHead, exampleKey, opts);
      return;
    }

    var isList = probe.type === nodes.bullet_list || probe.type === nodes.ordered_list;
    var container = isList ? singleChild(probe) : probe;
    var paragraph = singleChild(container);
    if (
      (isList && (!container || container.type !== nodes.list_item)) ||
      !paragraph ||
      paragraph.type !== nodes.paragraph
    ) {
      this._applyListFallback(nextHead, exampleKey, opts);
      return;
    }
    var probeText = singleChild(paragraph);
    var isPlainProbe = !!probeText && probeText.eq(this.schema.text(PROBE_TEXT));

    if (isPlainProbe && isList && (probe.type !== nodes.ordered_list || probe.attrs.order === 1)) {
      this._toggleListType(probe.type);
      this.focus();
      return;
    }

    if (isPlainProbe && probe.type === nodes.blockquote) {
      var command = inNode(this.view.state, nodes.blockquote) ? lift : wrapIn(nodes.blockquote);
      command(this.view.state, this.view.dispatch);
      this.focus();
      return;
    }

    this._applyListFallback(nextHead, exampleKey, opts);
  }

  applyHeading(_selection, level) {
    if (this.commands.removeSmall) {
      this.commands.removeSmall();
    }

    var command;
    if (level === 0) {
      command = setBlockType(this.schema.nodes.paragraph);
    } else {
      command = setBlockType(this.schema.nodes.heading, { level: level });
    }
    command(this.view.state, this.view.dispatch);
    this.focus();
  }

  // Bridge method from pre-existing API to the new command system:
  // whether the command was applied.
  formatCode() {
    return this.commands.formatCode(this.view.state, this.view.dispatch);
  }

  emojiSelected(code) {
    var self = this;
    var index = 0;

    var value = this.autocompleteHandler.getValue();
    var match = /\B:([\p{L}\p{N}_]*)$/u.exec(value);
    if (match) {
      index = value.length - match.index;
    }

    var selection = this.view.state.selection;

    this.view.dispatch(
      this.view.state.tr
        .replaceRangeWith(selection.from - index, selection.to, this.schema.nodes.emoji.create({ code: code }))
        .insertText(" ")
    );

    setTimeout(function () {
      self.focus();
    }, 0);
  }

  selectText(from, length, opts) {
    var tr = this.view.state.tr.setSelection(
      new TextSelection(this.view.state.doc.resolve(from), this.view.state.doc.resolve(from + length))
    );

    if (opts && opts.scroll) {
      tr.scrollIntoView();
    }

    this.view.dispatch(tr);
  }

  inCodeBlock() {
    return this.autocompleteHandler.inCodeBlock();
  }

  indentSelection(direction) {
    var selection = this.view.state.selection;

    var isInsideListItem =
      selection.$head.depth > 0 && selection.$head.node(-1).type === this.schema.nodes.list_item;

    if (isInsideListItem) {
      var command =
        direction === "right"
          ? sinkListItem(this.schema.nodes.list_item)
          : liftListItem(this.schema.nodes.list_item);
      command(this.view.state, this.view.dispatch);
      return true;
    }
  }

  insertText(text) {
    var block = firstBlockOf(this.convertFromMarkdown(text));

    this.view.dispatch(this.view.state.tr.replaceSelectionWith(block).scrollIntoView());

    this.focus();
  }

  replaceText(oldValue, newValue, opts) {
    opts = opts || {};
    // Replacing Markdown text is not reliable and should eventually be
    // deprecated
    var markdown = this.convertToMarkdown(this.view.state.doc);

    var regex = opts.regex || new RegExp(escapeRegExp(oldValue), "g");
    var index = opts.index || 0;
    var matchCount = 0;

    var newMarkdown = markdown.replace(regex, function (match) {
      if (matchCount++ === index) {
        return newValue;
      }
      return match;
    });

    if (markdown === newMarkdown) {
      return;
    }

    var newDoc = this.convertFromMarkdown(newMarkdown);
    if (!newDoc) {
      return;
    }

    var diff = newValue.length - oldValue.length;
    var startOffset = this.view.state.selection.from + diff;
    var endOffset = this.view.state.selection.to + diff;

    var tr = this.view.state.tr.replaceWith(0, this.view.state.doc.content.size, newDoc.content);

    if (!opts.skipNewSelection && (opts.forceFocus || this.view.dom === document.activeElement)) {
      var adjustedStart = Math.min(startOffset, tr.doc.content.size);
      var adjustedEnd = Math.min(endOffset, tr.doc.content.size);

      tr.setSelection(TextSelection.create(tr.doc, adjustedStart, adjustedEnd));
    }

    this.view.dispatch(tr);
  }

  toggleDirection() {
    this.view.dom.dir = this.view.dom.dir === "rtl" ? "ltr" : "rtl";
  }

  // The toolbar state object, from the editor's active states.
  updateState() {
    var view = this.view;
    var schema = this.schema;
    var activeHeadingLevel = [1, 2, 3, 4, 5, 6].find(function (headingLevel) {
      return isNodeActive(view.state, schema.nodes.heading, { level: headingLevel });
    });

    Object.assign(
      this.state,
      {
        inBold: hasMark(view.state, schema.marks.strong),
        inItalic: hasMark(view.state, schema.marks.em),
        inLink: hasMark(view.state, schema.marks.link),
        inCode: hasMark(view.state, schema.marks.code),
        inBulletList: inNode(view.state, schema.nodes.bullet_list),
        inOrderedList: inNode(view.state, schema.nodes.ordered_list),
        inCodeBlock: inNode(view.state, schema.nodes.code_block),
        inBlockquote: inNode(view.state, schema.nodes.blockquote),
        inHeading: !!activeHeadingLevel,
        inHeadingLevel: activeHeadingLevel,
        inParagraph: inNode(view.state, schema.nodes.paragraph),
      },
      this.customState(view.state)
    );
  }

  _probeMark(head, tail) {
    var doc = this.tryConvertFromMarkdown(head + PROBE_TEXT + tail);
    var paragraph = singleChild(doc);
    var text = paragraph && paragraph.type === this.schema.nodes.paragraph ? singleChild(paragraph) : null;

    return text && text.isText && text.text === PROBE_TEXT && text.marks.length === 1 ? text.marks[0] : null;
  }

  _restoreSelection(sel) {
    var state = this.view.state;
    if (sel.start !== state.selection.from || sel.end !== state.selection.to) {
      this.view.dispatch(
        state.tr.setSelection(TextSelection.between(state.doc.resolve(sel.start), state.doc.resolve(sel.end)))
      );
    }
  }

  _applySurroundLines(head, tail, opts) {
    var self = this;
    var state = this.view.state;
    var from = state.selection.from;
    var to = state.selection.to;
    var lines = [];
    var previous;
    var addLine = function (start, end) {
      if (start < end || opts.applyEmptyLines) {
        previous = head(previous);
        lines.push({ from: start, to: end, head: previous });
      }
    };

    state.doc.nodesBetween(from, to, function (node, pos) {
      if (!node.isTextblock) {
        return true;
      }
      var start = Math.max(from, pos + 1);
      var end = Math.min(to, pos + 1 + node.content.size);
      node.forEach(function (child, offset) {
        var childPos = pos + 1 + offset;
        if (child.type === self.schema.nodes.hard_break && childPos >= start && childPos < end) {
          addLine(start, childPos);
          start = childPos + child.nodeSize;
        }
      });
      addLine(start, end);
      return false;
    });

    var tr = state.tr;
    lines.reverse().forEach(function (line) {
      var text = self.convertToMarkdown(state.doc.slice(line.from, line.to));
      self._surroundRange(tr, line.from, line.to, text, line.head, tail, opts);
    });
    tr.setSelection(
      TextSelection.between(tr.doc.resolve(tr.mapping.map(from, -1)), tr.doc.resolve(tr.mapping.map(to, 1)))
    );
    this.view.dispatch(tr);
  }

  _applySurroundFallback(head, tail, exampleKey, opts) {
    var state = this.view.state;
    var from = state.selection.from;
    var to = state.selection.to;
    var text = this._selectedMarkdownOr(exampleKey);
    var result = this._surroundRange(state.tr, from, to, text, head, tail, opts);
    this._selectInserted(result.tr, from, to, text, result.prefix);
    this.view.dispatch(result.tr);
  }

  _surroundRange(tr, from, to, text, head, tail, opts) {
    var schema = this.schema;
    var $from = tr.doc.resolve(from);
    var $to = tr.doc.resolve(to);
    var withinTextblock = $from.sameParent($to) && $from.parent.isTextblock;
    var startsLine =
      $from.parentOffset === 0 || (!!$from.nodeBefore && $from.nodeBefore.type === schema.nodes.hard_break);
    var replacementFrom = from;
    var replacementTo = to;

    if (!tail && from !== to && withinTextblock && startsLine) {
      replacementTo = $to.end();
      if (opts && opts.multiline) {
        $from.parent.forEach(function (child, offset) {
          var pos = $from.start() + offset;
          if (child.type === schema.nodes.hard_break && pos >= from) {
            replacementTo = Math.min(replacementTo, pos);
          }
        });
      }
      text = this.convertToMarkdown(tr.doc.slice(from, replacementTo));
    }

    var useBlockMode = (opts && opts.useBlockMode) || (tail.length > 0 && !withinTextblock);
    if (useBlockMode && text.indexOf("\n") !== -1) {
      if (!head.endsWith("\n")) {
        head += "\n";
      }
      if (!tail.startsWith("\n")) {
        tail = "\n" + tail;
      }
    }

    var markdown = head + text + tail;
    var partialTextblock =
      withinTextblock &&
      (!startsLine ||
        (tail.length > 0 && $from.parentOffset > 0) ||
        (tail.length > 0 && $to.parentOffset < $from.parent.content.size));
    var inlineContent = partialTextblock ? this._convertInlineMarkdown(markdown) : null;
    if (inlineContent) {
      return {
        tr: tr.replaceWith(replacementFrom, replacementTo, inlineContent),
        prefix: this._surroundPrefix(head, tail, true),
      };
    }
    var parsed = this.convertFromMarkdown(markdown);
    if (
      !tail &&
      withinTextblock &&
      startsLine &&
      (parsed.childCount !== 1 || !parsed.firstChild || parsed.firstChild.type !== schema.nodes.paragraph)
    ) {
      if ($from.nodeBefore && $from.nodeBefore.type === schema.nodes.hard_break) {
        replacementFrom--;
      }
      var after = tr.doc.resolve(replacementTo).nodeAfter;
      if (after && after.type === schema.nodes.hard_break) {
        replacementTo++;
      }
    }
    return {
      tr: this._replaceWithParsed(tr, replacementFrom, replacementTo, parsed),
      prefix: this._surroundPrefix(head, tail),
    };
  }

  // Leading text keeps block rules from claiming the markup, like mid-line
  // markdown when cooked.
  _convertInlineMarkdown(markdown) {
    var lead = PROBE_TEXT + " ";
    var doc = this.tryConvertFromMarkdown(lead + markdown);
    var paragraph = singleChild(doc);
    var first = paragraph && paragraph.type === this.schema.nodes.paragraph ? paragraph.firstChild : null;

    return first && first.isText && first.text.startsWith(lead) ? paragraph.content.cut(lead.length) : null;
  }

  _replaceWithParsed(tr, from, to, doc) {
    var paragraph =
      doc.childCount === 1 && doc.firstChild && doc.firstChild.type === this.schema.nodes.paragraph
        ? doc.firstChild
        : null;

    if (paragraph) {
      return tr.replaceWith(from, to, paragraph.content);
    }

    // replaceRange would also swallow covered non-defining ancestors such
    // as a blockquote, so a fully covered textblock is replaced explicitly.
    var $from = tr.doc.resolve(from);
    var $to = tr.doc.resolve(to);
    var wholeTextblock =
      $from.sameParent($to) &&
      $from.parent.isTextblock &&
      $from.parentOffset === 0 &&
      $to.parentOffset === $to.parent.content.size &&
      $from.node(-1).canReplace($from.index(-1), $from.indexAfter(-1), doc.content);

    return wholeTextblock
      ? tr.replaceWith($from.before(), $to.after(), doc.content)
      : tr.replaceRange(from, to, new Slice(doc.content, 0, 0));
  }

  _surroundPrefix(head, tail, inline) {
    var markdown = head + PROBE_TEXT + tail;
    var doc = inline ? this._convertInlineMarkdown(markdown) : this.tryConvertFromMarkdown(markdown);
    if (!doc) {
      return Fragment.empty;
    }

    var content = flattenSelection(doc).content;
    var prefix = Fragment.empty;
    content.forEach(function (node, offset) {
      var index = node.isText ? node.text.indexOf(PROBE_TEXT) : -1;
      if (index >= 0) {
        prefix = content.cut(0, offset + index);
      }
    });
    return prefix;
  }

  _selectInserted(tr, from, to, text, prefix) {
    prefix = prefix || Fragment.empty;
    // The replace step knows where the content landed, even when it was
    // placed before the selection's textblock.
    var start = from;
    var end = to;
    var maps = tr.mapping.maps;
    if (maps.length) {
      maps[maps.length - 1].forEach(function (_, __, newStart, newEnd) {
        start = newStart;
        end = newEnd;
      });
    }
    var parsed = this.tryConvertFromMarkdown(text);
    var expected = parsed
      ? flattenSelection(parsed).content
      : Fragment.from(text ? this.schema.text(text) : null);
    var flat = flattenSelection(tr.doc, start, end);
    var content = flat.content;
    var segments = flat.segments;
    var firstNode = expected.firstChild;
    // A wrapper can repeat the selected text, so prefer the parsed inner
    // boundary.
    var match =
      prefix.size > 0 &&
      content.cut(0, prefix.size).eq(prefix) &&
      content.cut(prefix.size, prefix.size + expected.size).eq(expected)
        ? prefix.size
        : undefined;
    if (firstNode) {
      content.forEach(function (node, offset) {
        if (match !== undefined) {
          return;
        }
        var index =
          node.isText && firstNode.isText ? node.text.indexOf(firstNode.text) : node.eq(firstNode) ? 0 : -1;
        while (index >= 0) {
          if (content.cut(offset + index, offset + index + expected.size).eq(expected)) {
            match = offset + index;
            return;
          }
          if (!node.isText || !firstNode.isText) {
            break;
          }
          index = node.text.indexOf(firstNode.text, index + 1);
        }
      });
    }

    if (firstNode && match !== undefined) {
      var matchStart = match;
      var first = segments.find(function (segment) {
        return segment.end > matchStart;
      });
      var lastIndex = matchStart + expected.size - 1;
      var last = segments.find(function (segment) {
        return segment.end > lastIndex;
      });
      var selectionFrom = first.pos + matchStart - first.start;
      tr.setSelection(
        expected.childCount === 1 && NodeSelection.isSelectable(firstNode)
          ? NodeSelection.create(tr.doc, selectionFrom)
          : TextSelection.create(tr.doc, selectionFrom, last.pos + lastIndex - last.start + 1)
      );
    } else if (content.childCount === 1 && NodeSelection.isSelectable(content.firstChild)) {
      tr.setSelection(NodeSelection.create(tr.doc, segments[0].pos));
    } else {
      tr.setSelection(TextSelection.between(tr.doc.resolve(start), tr.doc.resolve(end)));
    }
  }

  _toggleListType(targetType) {
    var self = this;
    var state = this.view.state;
    var $from = state.selection.$from;
    var nodes = this.schema.nodes;

    var currentListType = null;
    for (var depth = $from.depth; depth > 0; depth--) {
      var type = $from.node(depth).type;
      if (type === nodes.bullet_list || type === nodes.ordered_list) {
        currentListType = type;
        break;
      }
    }

    if (!currentListType) {
      var content = state.selection.content().content;
      var lines = this.splitNonEmptyLines(
        state.doc.textBetween(state.selection.from, state.selection.to, "\n", "\n")
      );
      if (isPlainTextFragment(content, this.schema) && lines.length > 1) {
        var list = this.buildListNode(this.schema, targetType, lines);
        this.view.dispatch(state.tr.replaceSelectionWith(list).scrollIntoView());
        return;
      }
      wrapInList(targetType)(state, this.view.dispatch);
      return;
    }

    if (currentListType === targetType) {
      liftListItem(nodes.list_item)(state, this.view.dispatch);
      return;
    }

    // Lifting and re-wrapping in one transaction keeps the switch a single
    // undo step.
    liftListItem(nodes.list_item)(state, function (tr) {
      var range = tr.selection.$from.blockRange(tr.selection.$to);
      if (range && wrapRangeInList(tr, range, targetType)) {
        self.view.dispatch(tr);
      }
    });
  }

  _applyListFallback(head, exampleKey, opts) {
    var state = this.view.state;
    var from = state.selection.from;
    var to = state.selection.to;
    var text = this._selectedMarkdownOr(exampleKey);
    var firstHead = head();
    if (opts && opts.multiline === false) {
      this._applySurroundFallback(firstHead, "", exampleKey, opts);
      this.focus();
      return;
    }

    var previous;
    var result = text
      .split("\n")
      .map(function (line) {
        if (!(opts && opts.applyEmptyLines) && !line.length) {
          return line;
        }
        previous = head(previous);
        return previous + line;
      })
      .join("\n");

    var tr = this._replaceWithParsed(state.tr, from, to, this.convertFromMarkdown(result));

    this._selectInserted(tr, from, to, text, this._surroundPrefix(firstHead, ""));

    this.view.dispatch(tr);
    this.focus();
  }

  _selectedMarkdownOr(exampleKey) {
    var selection = this.view.state.selection;
    return selection.empty
      ? this.i18n("composer." + exampleKey)
      : this.convertToMarkdown(this.view.state.doc.slice(selection.from, selection.to));
  }
}
