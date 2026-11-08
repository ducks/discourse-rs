// An editor document back into markdown (static/prosemirror/core/
// serializer.js): prosemirror-markdown's serializers, unless includeDefault
// is false, a hard break written as a newline (`<br>` in a table), text
// escaped but in autolinks, then each extension's serializeNode and
// serializeMark. serializeNode may carry an afterSerialize run on the
// state once the document is written (footnotes append their notes so).
import { defaultMarkdownSerializer, MarkdownSerializerState } from "prosemirror-markdown";
import { Fragment, Node } from "prosemirror-model";
import { Transform } from "prosemirror-transform";

var FLANKING_SENSITIVE_MARKS = ["strong", "em", "strikethrough"];

// markdown-it's isPunctChar (P or S).
var PUNCTUATION = /[\p{P}\p{S}]/u;
var LEADING_PUNCTUATION = new RegExp("^" + PUNCTUATION.source + "+", "u");
var TRAILING_PUNCTUATION = new RegExp(PUNCTUATION.source + "+$", "u");

function isWordChar(char) {
  return !!char && !PUNCTUATION.test(char) && !/\s/u.test(char);
}

// lib/expel-boundary-punctuation.ts: the punctuation counterpart of
// prosemirror-markdown's expelEnclosingWhitespace. `**text.**text` reparses
// as literal asterisks, so it is written `**text**.text`. Slices and
// fragments (a selection's) pass through untouched.
export function expelBoundaryPunctuation(content) {
  if (!(content instanceof Node)) {
    return content;
  }

  var tr = new Transform(content);
  var markTypes = FLANKING_SENSITIVE_MARKS.map(function (name) {
    return content.type.schema.marks[name];
  }).filter(Boolean);

  // RemoveMarkStep doesn't shift positions, so no mapping is needed.
  content.descendants(function (node, pos) {
    if (!node.isTextblock) {
      return;
    }

    node.forEach(function (child, offset, index) {
      var text = child.text;
      if (text === undefined) {
        return;
      }

      var start = pos + 1 + offset;
      var end = start + text.length;
      var next = index + 1 < node.childCount ? node.child(index + 1) : null;
      var prev = index > 0 ? node.child(index - 1) : null;
      var nextChar = next && next.text ? next.text[0] : undefined;
      var prevText = prev ? prev.text : undefined;
      var prevChar = prevText ? prevText[prevText.length - 1] : undefined;

      markTypes.forEach(function (markType) {
        if (!markType.isInSet(child.marks)) {
          return;
        }

        if ((!next || !markType.isInSet(next.marks)) && isWordChar(nextChar)) {
          var trailing = text.match(TRAILING_PUNCTUATION);
          if (trailing) {
            tr.removeMark(end - trailing[0].length, end, markType);
          }
        }

        if ((!prev || !markType.isInSet(prev.marks)) && isWordChar(prevChar)) {
          var leading = text.match(LEADING_PUNCTUATION);
          if (leading) {
            tr.removeMark(start, start + leading[0].length, markType);
          }
        }
      });
    });
  });

  return tr.doc;
}

export default function Serializer(extensions, pluginParams, includeDefault) {
  if (includeDefault === undefined) {
    includeDefault = true;
  }
  this.afterSerializers = null;
  this.nodes = includeDefault ? Object.assign({}, defaultMarkdownSerializer.nodes) : {};
  this.nodes.hard_break = function (state) {
    state.write(state.inTable ? "<br>" : "\n");
  };
  this.nodes.text = function (state, node) {
    state.text(node.text, !state.inAutolink);
  };

  this.marks = includeDefault ? Object.assign({}, defaultMarkdownSerializer.marks) : {};

  var self = this;
  extensions.forEach(function (extension) {
    var serializer =
      typeof extension.serializeNode === "function"
        ? extension.serializeNode(pluginParams)
        : extension.serializeNode;
    Object.assign(self.nodes, serializer);
    if (serializer && serializer.afterSerialize) {
      self.afterSerializers = self.afterSerializers || [];
      self.afterSerializers.push(serializer.afterSerialize);
    }
  });
  extensions.forEach(function (extension) {
    var serializer =
      typeof extension.serializeMark === "function"
        ? extension.serializeMark(pluginParams)
        : extension.serializeMark;
    Object.assign(self.marks, serializer);
  });
}

// A document, a slice or a fragment (a selection's content): inline
// content is rendered as a paragraph's, so its marks are written.
Serializer.prototype.convert = function (doc) {
  var state = new MarkdownSerializerState(this.nodes, this.marks, {});
  var normalized = expelBoundaryPunctuation(doc);
  var content = normalized instanceof Fragment ? normalized : normalized.content;
  if (content.firstChild && content.firstChild.isInline) {
    var parent =
      normalized instanceof Node ? normalized : content.firstChild.type.schema.nodes.paragraph.create(null, content);
    state.renderInline(parent);
  } else {
    state.renderContent(content);
  }

  if (this.afterSerializers) {
    this.afterSerializers.forEach(function (afterSerializer) {
      afterSerializer(state);
    });
  }

  return state.out;
};
