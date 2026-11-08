// The helpers extensions receive as params.utils, from the reference's
// static/prosemirror/lib/plugin-utils.js. isWhiteSpace and isBoundary are
// markdown-it's own (common/utils.mjs: isWhiteSpace, isPunctChar as P or S),
// written out here since the editor loads no markdown-it; getLinkify is the
// engine's (engine.js). buildBBCodeAttrs and serializeBBCodeAttr are
// lib/text.js's, parseBBCodeTag bbcode-block.js's, which lib/wrap-utils.js
// and the plugins' extensions import.
import { InputRule } from "prosemirror-inputrules";
import { StepMap } from "prosemirror-transform";

export { getLinkify } from "discourse/rich-editor/engine";

var PUNCT_OR_SYMBOL = /[\p{P}\p{S}]/u;

function isWhiteSpaceCode(code) {
  if (code >= 0x2000 && code <= 0x200a) {
    return true;
  }
  switch (code) {
    case 0x09:
    case 0x0a:
    case 0x0b:
    case 0x0c:
    case 0x0d:
    case 0x20:
    case 0xa0:
    case 0x1680:
    case 0x202f:
    case 0x205f:
    case 0x3000:
      return true;
  }
  return false;
}

export function isWhiteSpace(str, index) {
  return !str || isWhiteSpaceCode(str.charCodeAt(index));
}

export function isBoundary(str, index) {
  return (
    !str ||
    isWhiteSpaceCode(str.charCodeAt(index)) ||
    PUNCT_OR_SYMBOL.test(String.fromCharCode(str.charCodeAt(index)))
  );
}

// An InputRule applying a mark over its match: a single group is the
// marked text, and getAttrs may give the mark's attrs and override the
// match or its start (inputrules.js adapts two-group patterns that way).
// Initially from https://discuss.prosemirror.net/t/input-rules-for-wrapping-marks/537
export function markInputRule(regexp, markType, getAttrs) {
  return new InputRule(regexp, function (state, match, start, end) {
    var attrs = getAttrs instanceof Function ? getAttrs(match) : getAttrs;
    var tr = state.tr;

    var markAttrs = {};
    var attrsMatch;
    var attrsStart;
    if (attrs) {
      Object.keys(attrs).forEach(function (key) {
        if (key === "match") {
          attrsMatch = attrs[key];
        } else if (key === "start") {
          attrsStart = attrs[key];
        } else {
          markAttrs[key] = attrs[key];
        }
      });
    }

    match = attrsMatch !== undefined && attrsMatch !== null ? attrsMatch : match;
    start = start + (attrsStart !== undefined && attrsStart !== null ? attrsStart : 0);

    if (match[1]) {
      var fullMatch = match[0];
      var capturedContent = match[1];
      var contentStart = fullMatch.indexOf(capturedContent);
      var contentEnd = contentStart + capturedContent.length;

      var ranges = [];
      if (contentStart > 0) {
        ranges.push([start, start + contentStart]);
      }
      if (contentEnd < fullMatch.length) {
        ranges.push([start + contentEnd, start + fullMatch.length]);
      }

      for (var i = 0; i < ranges.length; i++) {
        var hasCodeMark = false;
        state.doc.nodesBetween(ranges[i][0], ranges[i][1], function (node) {
          if (
            node.isInline &&
            node.marks.some(function (m) {
              return m.type.spec.code;
            })
          ) {
            hasCodeMark = true;
            return false;
          }
        });
        if (hasCodeMark) {
          return null;
        }
      }
    }

    if (match[1]) {
      var textStart = start + match[0].indexOf(match[1]);
      var textEnd = textStart + match[1].length;
      if (textEnd < end) {
        tr.delete(textEnd, end);
      }
      if (textStart > start) {
        tr.delete(start, textStart);
      }

      tr.addMark(start, start + match[1].length, markType.create(markAttrs));
      tr.setStoredMarks(tr.doc.resolve(start + match[1].length + 1).marks());
    } else {
      tr.delete(start, end);
      tr.insertText(" ");
      tr.addMark(start, start + 1, markType.create(markAttrs));
      tr.removeStoredMark(markType);
      tr.insertText(" ");

      tr.setSelection(state.selection.constructor.create(tr.doc, start, start + 1));
    }

    return tr;
  });
}

export function getChangedRanges(tr) {
  var steps = tr.steps;
  var mapping = tr.mapping;
  var changes = [];

  mapping.maps.forEach(function (stepMap, index) {
    var ranges = [];

    if (stepMap === StepMap.empty) {
      if (steps[index].from === undefined || steps[index].to === undefined) {
        return;
      }

      ranges.push(steps[index]);
    } else {
      stepMap.forEach(function (from, to) {
        ranges.push({ from: from, to: to });
      });
    }

    ranges.forEach(function (range) {
      var change = { new: {}, old: {} };
      change.new.from = mapping.slice(index).map(range.from, -1);
      change.new.to = mapping.slice(index).map(range.to);
      change.old.from = mapping.invert().map(change.new.from, -1);
      change.old.to = mapping.invert().map(change.new.to);

      changes.push(change);
    });
  });

  return changes;
}

// From prosemirror-commands' atBlockStart.
export function atBlockStart(state, view) {
  var $cursor = state.selection.$cursor;
  if (
    !$cursor ||
    (view ? !view.endOfTextblock("backward", state) : $cursor.parentOffset > 0)
  ) {
    return null;
  }
  return $cursor;
}

// https://github.com/discourse/discourse/pull/31933#discussion_r2019739410
export function changedDescendants(old, cur, f, offset) {
  offset = offset || 0;
  var oldSize = old.childCount;
  var curSize = cur.childCount;
  outer: for (var i = 0, j = 0; i < curSize; i++) {
    var child = cur.child(i);
    for (var scan = j, e = Math.min(oldSize, i + 5); scan < e; scan++) {
      if (old.child(scan) === child) {
        j = scan + 1;
        offset += child.nodeSize;
        continue outer;
      }
    }
    f(child, offset);
    if (j < oldSize && old.child(j).sameMarkup(child)) {
      changedDescendants(old.child(j), child, f, offset + 1);
    } else {
      child.nodesBetween(0, child.content.size, f, offset + 1);
    }
    offset += child.nodeSize;
  }
}

// The continuous range of a mark of `type` (matching `attrs`) at $pos:
// { from, to, mark }, or undefined.
export function getMarkRange($pos, type, attrs) {
  attrs = attrs || {};
  if (!$pos || !type) {
    return;
  }

  // The node after, then the one before.
  var start = $pos.parent.childAfter($pos.parentOffset);
  if (!start.node || !findMarkOfType(start.node.marks, type, attrs)) {
    start = $pos.parent.childBefore($pos.parentOffset);
    if (!start.node || !findMarkOfType(start.node.marks, type, attrs)) {
      return;
    }
  }

  var mark = findMarkOfType(start.node.marks, type, attrs);

  var from = $pos.start() + start.offset;
  var to = from + start.node.nodeSize;

  var index = start.index;
  while (index > 0 && findMarkOfType($pos.parent.child(index - 1).marks, type, mark.attrs)) {
    index--;
    from -= $pos.parent.child(index).nodeSize;
  }

  index = start.index + 1;
  while (
    index < $pos.parent.childCount &&
    findMarkOfType($pos.parent.child(index).marks, type, mark.attrs)
  ) {
    to += $pos.parent.child(index).nodeSize;
    index++;
  }

  return { from: from, to: to, mark: mark };
}

export function findMarkOfType(marks, type, attrs) {
  attrs = attrs || {};
  return marks.find(function (item) {
    return (
      item.type === type &&
      Object.keys(attrs).every(function (key) {
        return item.attrs[key] === attrs[key];
      })
    );
  });
}

// A mark of the type in the selection, optionally with these attrs.
export function hasMark(state, markType, attrs) {
  attrs = attrs || {};
  var selection = state.selection;

  // An empty selection: the stored marks or the marks at the position.
  if (selection.empty) {
    var storedMarks = state.storedMarks || selection.$from.marks();
    return !!findMarkOfType(storedMarks, markType, attrs);
  }

  return (
    state.doc.rangeHasMark(selection.from, selection.to, markType) &&
    (!Object.keys(attrs).length ||
      state.doc.rangeHasMark(selection.from, selection.to, markType.create(attrs)))
  );
}

// The selection is inside a node of the type, optionally with these attrs.
export function inNode(state, nodeType, attrs) {
  attrs = attrs || {};
  var $from = state.selection.$from;

  for (var depth = $from.depth; depth >= 0; depth--) {
    var node = $from.node(depth);
    if (node.type === nodeType) {
      if (!Object.keys(attrs).length) {
        return true;
      }

      return Object.keys(attrs).every(function (key) {
        return node.attrs[key] === attrs[key];
      });
    }
  }

  return false;
}

// The node type (with these attrs) is active in the selection and no node
// of another type is in it.
export function isNodeActive(state, nodeType, attrs) {
  attrs = attrs || {};
  var selection = state.selection;
  var from = selection.from;
  var to = selection.to;
  var nodeRanges = [];

  state.doc.nodesBetween(from, to, function (node, pos) {
    if (node.isText) {
      return;
    }

    nodeRanges.push({
      node: node,
      from: Math.max(from, pos),
      to: Math.min(to, pos + node.nodeSize),
    });
  });

  var selectionRange = to - from;

  var matchedNodeRanges = nodeRanges.filter(function (nodeRange) {
    return (
      nodeType.name === nodeRange.node.type.name &&
      Object.keys(attrs).every(function (key) {
        return nodeRange.node.attrs[key] === attrs[key];
      })
    );
  });

  if (selection.empty) {
    return !!matchedNodeRanges.length;
  }

  // No gaps between the matched ranges: nothing else is selected.
  var range = matchedNodeRanges.reduce(function (sum, nodeRange) {
    return sum + nodeRange.to - nodeRange.from;
  }, 0);

  return range >= selectionRange;
}

// bbcode-block.js's QUOTATION_MARKS and the patterns parseBBCodeTag uses.
export var QUOTATION_MARKS = ['""', "''", "«»", "“”", "””", "‘’", "„“", "‚’", "‹›"];

var QUOTATION_MARKS_NO_MATCH = QUOTATION_MARKS.map(function (pair) {
  return pair[0] + "[^" + pair[1] + "]+" + pair[1];
}).join("|");

var QUOTATION_MARKS_WITH_MATCH = QUOTATION_MARKS.map(function (pair) {
  return pair[0] + "([^" + pair[1] + "]+)" + pair[1];
}).join("|");

var BBCODE_CLOSING_TAG_REGEXP = /^\[\/([-\w]+)\]/i;

var BBCODE_QUOTE_OR_DETAILS_TAG_REGEXP = new RegExp(
  "^\\[(quote|details)=(\\s*[^" + QUOTATION_MARKS.join("") + "].+?)\\]",
  "i"
);

// Without the closing "]", which is checked by hand: the regexp would
// backtrack too much with it.
var BBCODE_TAG_REGEXP = new RegExp(
  "\\[(?:(?:[-\\w]+(?:=(?:" + QUOTATION_MARKS_NO_MATCH + "|[^\\s\\]]+))?)+\\s*)+",
  "i"
);

var BBCODE_ATTR_REGEXP = new RegExp("([-\\w]+)(?:=(?:" + QUOTATION_MARKS_WITH_MATCH + "|([^\\s\\]]+)))?", "gi");

function trailingSpaceOnly(src, start, max) {
  for (var i = start; i < max; i++) {
    var code = src.charCodeAt(i);
    if (code === 0x0a) {
      return true;
    }
    if (!isWhiteSpaceCode(code)) {
      return false;
    }
  }
  return true;
}

export function parseBBCodeTag(src, start, max, multiline) {
  var m;
  var text = src.slice(start, max);

  m = BBCODE_CLOSING_TAG_REGEXP.exec(text);
  if (m && m[0] && m[1]) {
    if (multiline && !trailingSpaceOnly(src, start + m[0].length, max)) {
      return null;
    }
    return { tag: m[1].toLowerCase(), closing: true, length: m[0].length };
  }

  m = BBCODE_QUOTE_OR_DETAILS_TAG_REGEXP.exec(text);
  if (m && m[0] && m[1] && m[2]) {
    if (multiline && !trailingSpaceOnly(src, start + m[0].length, max)) {
      return null;
    }
    return { tag: m[1], length: m[0].length, attrs: { _default: m[2] } };
  }

  m = BBCODE_TAG_REGEXP.exec(text);
  var bbcode = m ? m[0] : null;
  if (!bbcode) {
    return null;
  }
  if (text.length <= bbcode.length || text[bbcode.length] !== "]") {
    return null;
  }

  var r = {};
  BBCODE_ATTR_REGEXP.lastIndex = 0;
  while ((m = BBCODE_ATTR_REGEXP.exec(bbcode))) {
    var key = m[1];
    var value = m.slice(2).find(Boolean);

    if (!key) {
      return null;
    }

    if (!r.tag) {
      r.tag = key.toLowerCase();
      r.length = bbcode.length + 1;
      if (m.index === 1) {
        r.attrs = {};
        if (value) {
          r.attrs._default = value.trim();
        }
      } else {
        return null;
      }
    } else if (r.attrs) {
      r.attrs[key] = (value && value.trim()) || "";
    } else {
      return null;
    }
  }

  if (r.tag) {
    if (multiline && !trailingSpaceOnly(src, start + bbcode.length + 1, max)) {
      return null;
    }
    return r;
  }
  return null;
}

var BBCODE_REQUIRES_QUOTES_PATTERN = /[\s\]]/;

// lib/text.js: ` name=value`, quoted with the first quotation marks the
// value does not contain where it has a space or "]".
export function serializeBBCodeAttr(value, name) {
  if (!value) {
    return "";
  }

  var stringValue = String(value);
  if (!BBCODE_REQUIRES_QUOTES_PATTERN.test(stringValue)) {
    return " " + name + "=" + stringValue;
  }

  for (var i = 0; i < QUOTATION_MARKS.length; i++) {
    var open = QUOTATION_MARKS[i][0];
    var close = QUOTATION_MARKS[i][1];
    if (stringValue.indexOf(open) === -1 && stringValue.indexOf(close) === -1) {
      return " " + name + "=" + open + stringValue + close;
    }
  }

  return " " + name + '="' + stringValue.split('"').join("") + '"';
}

// lib/text.js: `name=value foo="bar baz"`, skipping null and undefined.
export function buildBBCodeAttrs(attrs, opts) {
  opts = opts || {};
  var skipAttrs = opts.skipAttrs || [];

  return Object.keys(attrs)
    .map(function (key) {
      if (attrs[key] === null || attrs[key] === undefined || skipAttrs.indexOf(key) !== -1) {
        return null;
      }
      return serializeBBCodeAttr(attrs[key], key).trim();
    })
    .filter(Boolean)
    .join(" ");
}
