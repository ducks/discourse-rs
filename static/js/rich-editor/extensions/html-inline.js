// extensions/html-inline.js: inline HTML the cook keeps (its allow list,
// the engine's) as html_inline nodes, with their allowed attributes; the
// tags with a markdown form (<b>, <em>, <a href>, <img src>, <br>) as their
// marks and nodes; an outline on the tags with no look of their own; the
// `<tag>` input rule; and the small text commands.
import { Fragment } from "prosemirror-model";
import { TextSelection } from "prosemirror-state";
import { getAllowLister } from "discourse/rich-editor/engine";

var HTML_INLINE_MARKS = {
  s: "strikethrough",
  strike: "strikethrough",
  strong: "strong",
  b: "strong",
  em: "em",
  i: "em",
  code: "code",
};

var INLINE_TAGS = [
  "kbd",
  "sup",
  "sub",
  "small",
  "big",
  "del",
  "ins",
  "mark",
  "ruby",
  "rb",
  "rt",
  "rp",
  "span",
];

// Tags with a look of their own on the element (markers and cell borders
// count, margins and link colour don't). Any other tag is outlined.
var STYLED_TAGS = INLINE_TAGS.filter(function (tag) {
  return tag !== "span";
}).concat([
  "abbr",
  "blockquote",
  "dd",
  "details",
  "dl",
  "dt",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "li",
  "ol",
  "pre",
  "summary",
  "table",
  "ul",
]);

var VOID_TAGS = [
  "area",
  "base",
  "br",
  "col",
  "embed",
  "hr",
  "img",
  "input",
  "link",
  "meta",
  "param",
  "source",
  "track",
  "wbr",
];

// Tags that open an HTML block at a line start (markdown-it's `html_blocks`
// plus pre/script/style/textarea), so they are only kept inside table cells.
var HTML_BLOCK_TAGS = [
  "address",
  "article",
  "aside",
  "base",
  "basefont",
  "blockquote",
  "body",
  "caption",
  "center",
  "col",
  "colgroup",
  "dd",
  "details",
  "dialog",
  "dir",
  "div",
  "dl",
  "dt",
  "fieldset",
  "figcaption",
  "figure",
  "footer",
  "form",
  "frame",
  "frameset",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "head",
  "header",
  "hr",
  "html",
  "iframe",
  "legend",
  "li",
  "link",
  "main",
  "menu",
  "menuitem",
  "nav",
  "noframes",
  "ol",
  "optgroup",
  "option",
  "p",
  "param",
  "pre",
  "script",
  "search",
  "section",
  "style",
  "summary",
  "table",
  "tbody",
  "td",
  "textarea",
  "tfoot",
  "th",
  "thead",
  "title",
  "tr",
  "track",
  "ul",
];

// Allow-listed by the cook, but would load media or frames in the editor.
var EMBEDDED_TAGS = ["audio", "iframe", "picture", "video"];

function insideTableCell(state) {
  return state.stack.some(function (entry) {
    return ["cell", "header_cell"].indexOf(entry.type.spec.tableRole) >= 0;
  });
}

function isAllowListedTag(state, tagName) {
  if (
    VOID_TAGS.indexOf(tagName) >= 0 ||
    EMBEDDED_TAGS.indexOf(tagName) >= 0 ||
    !getAllowLister().tags.has(tagName)
  ) {
    return false;
  }

  return HTML_BLOCK_TAGS.indexOf(tagName) === -1 || insideTableCell(state);
}

// The element's attributes the allow list keeps (isAllowListedAttr or a
// custom check), or null for none; null too for a tag not on the list.
function extractHtmlAttrs(element, tagName) {
  var allowLister = getAllowLister();
  if (!allowLister.tags.has(tagName)) {
    return null;
  }

  var attrs = {};
  var hasAny = false;
  Array.prototype.forEach.call(element.attributes, function (attr) {
    if (allowLister.attrAllowed(tagName, attr.name, attr.value)) {
      attrs[attr.name] = attr.value;
      hasAny = true;
    }
  });
  return hasAny ? attrs : null;
}

function closeHtmlInline(state, tagName) {
  var htmlInline = state.schema.nodes.html_inline;
  var depth = -1;

  for (var i = state.stack.length - 1; i > 0; i--) {
    var entry = state.stack[i];
    if (!entry.type.isInline) {
      break;
    }
    if (entry.type === htmlInline && entry.attrs.tag === tagName) {
      depth = i;
      break;
    }
  }

  if (depth === -1) {
    return false;
  }

  while (state.stack.length > depth) {
    state.closeNode();
  }
  return true;
}

function serializeHtmlAttrs(htmlAttrs) {
  if (!htmlAttrs) {
    return "";
  }
  return Object.keys(htmlAttrs)
    .map(function (k) {
      var v = htmlAttrs[k];
      return v === ""
        ? " " + k
        : " " +
            k +
            '="' +
            v.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;") +
            '"';
    })
    .join("");
}

var INPUT_RULE_TAGS = Object.keys(HTML_INLINE_MARKS).concat(INLINE_TAGS);

// Pasted DOM is mostly cooked output, where the allow-list is far too wide.
// Spans are left out: publishing tools stamp `lang` on every one of them.
var DOM_PARSED_INLINE = INLINE_TAGS.filter(function (tag) {
  return tag !== "span";
}).concat(["abbr"]);

// Only lists have no markdown form inside a cell. A selector starting with
// `ul`/`ol` would stop ProseMirror from normalizing malformed nested lists.
var DOM_PARSED_IN_CELL = ["li", "ul", "ol", "dl", "dt", "dd"];

var SMALL = "small";

function isSmall(node, schema) {
  return node.type === schema.nodes.html_inline && node.attrs.tag === SMALL;
}

function withoutSmall(fragment, schema) {
  var nodes = [];
  fragment.forEach(function (child) {
    var content = child.content.size ? withoutSmall(child.content, schema) : [];

    if (isSmall(child, schema)) {
      nodes.push.apply(nodes, content);
    } else if (child.content.size) {
      nodes.push(child.copy(Fragment.fromArray(content)));
    } else {
      nodes.push(child);
    }
  });
  return nodes;
}

function splitTrailingWhitespace(content) {
  var main = content.slice();
  var lastNode = main[main.length - 1];
  var match = lastNode && lastNode.isText ? lastNode.text.match(/\s+$/) : null;
  var trailingWhitespace = match ? match[0] : null;

  if (!trailingWhitespace) {
    return [main, []];
  }

  main.pop();
  var boundary = lastNode.nodeSize - trailingWhitespace.length;
  if (boundary > 0) {
    main.push(lastNode.cut(0, boundary));
  }

  return [main, [lastNode.cut(boundary)]];
}

// Small is offered as a text size beside the heading levels, so it applies
// to whole lines rather than to the selected range.
function selectedLines(state) {
  var selection = state.selection;
  var lines = [];

  if (selection.empty) {
    for (var depth = selection.$from.depth; depth > 0; depth--) {
      var node = selection.$from.node(depth);
      if (node.isTextblock && !node.type.spec.code) {
        lines.push({ node: node, pos: selection.$from.before(depth) });
        return lines;
      }
    }
  }

  state.doc.nodesBetween(selection.from, selection.to, function (node, pos) {
    if (node.isTextblock) {
      if (!node.type.spec.code) {
        lines.push({ node: node, pos: pos });
      }
      return false;
    }
  });

  return lines;
}

function selectedSmallRanges(state, schema) {
  var selection = state.selection;
  var $from = selection.$from;

  if (selection.empty) {
    for (var depth = 1; depth <= $from.depth; depth++) {
      var node = $from.node(depth);
      if (isSmall(node, schema)) {
        return [{ node: node, from: $from.before(depth), to: $from.after(depth) }];
      }
    }
    return [];
  }

  var ranges = [];
  var hasContent = false;
  var allSmall = true;

  state.doc.nodesBetween(selection.from, selection.to, function (node, pos) {
    if (node.isBlock) {
      return true;
    }
    if (isSmall(node, schema)) {
      hasContent = true;
      ranges.push({ node: node, from: pos, to: pos + node.nodeSize });
      return false;
    }
    if (node.isText && !node.text.trim()) {
      return false;
    }
    hasContent = true;
    allSmall = false;
    return false;
  });

  return hasContent && allSmall ? ranges : [];
}

function containsSmall(node, schema) {
  var found = false;
  node.descendants(function (child) {
    if (isSmall(child, schema)) {
      found = true;
    }
    return !found;
  });
  return found;
}

function replaceLines(state, lines, replacement, schema) {
  var tr = state.tr;

  lines.forEach(function (line) {
    var mappedPos = tr.mapping.map(line.pos);

    if (schema && line.node.type === schema.nodes.heading) {
      tr.setNodeMarkup(mappedPos, schema.nodes.paragraph, {});
    }

    tr.replaceWith(
      tr.mapping.map(line.pos + 1),
      tr.mapping.map(line.pos + line.node.nodeSize - 1),
      replacement(line.node)
    );
  });

  return tr;
}

function unwrapSmallRanges(state, ranges, schema) {
  var tr = state.tr;

  ranges.forEach(function (range) {
    tr.replaceWith(tr.mapping.map(range.from), tr.mapping.map(range.to), withoutSmall(range.node.content, schema));
  });

  return tr;
}

export default {
  nodeSpec: {
    html_inline: {
      group: "inline",
      inline: true,
      defining: true,
      content: "inline*",
      attrs: { tag: {}, htmlAttrs: { default: null } },
      parseDOM: [
        // Outranks the list rules, so a pasted list stays inline HTML in a cell.
        {
          tag: DOM_PARSED_IN_CELL.join(","),
          priority: 60,
          context: "table_cell//|table_header_cell//",
          getAttrs: function (element) {
            var tag = element.tagName.toLowerCase();
            return { tag: tag, htmlAttrs: extractHtmlAttrs(element, tag) };
          },
        },
      ].concat(
        DOM_PARSED_INLINE.map(function (tag) {
          return {
            tag: tag,
            getAttrs: function (element) {
              return { tag: tag, htmlAttrs: extractHtmlAttrs(element, tag) };
            },
          };
        })
      ),
      toDOM: function (node) {
        return [node.attrs.tag, Object.assign({}, node.attrs.htmlAttrs), 0];
      },
    },
  },
  parse: {
    html_inline: function (state, token) {
      var openMatch = token.content.match(/^<([a-z][a-z0-9]*)(\s[^>]*)?\/?>$/i);
      var closeMatch = token.content.match(/^<\/([a-z][a-z0-9]*)>$/i);
      var tagName;
      var markName;

      if (openMatch) {
        tagName = openMatch[1].toLowerCase();
        var hasAttributes = openMatch[2];

        if (tagName === "br") {
          state.addNode(state.schema.nodes.hard_break);
          return;
        }

        markName = HTML_INLINE_MARKS[tagName];
        if (markName) {
          state.openMark(state.schema.marks[markName].create());
          return;
        }

        if (hasAttributes) {
          var doc = new DOMParser().parseFromString(token.content, "text/html");
          var element = doc.body.firstElementChild;

          if (element) {
            // a link is the link mark
            if (tagName === "a" && element.href) {
              state.openMark(
                state.schema.marks.link.create({
                  href: element.getAttribute("href"),
                  title: element.title || null,
                })
              );
              return;
            }

            // an image is the image node
            if (tagName === "img" && element.src) {
              state.addNode(state.schema.nodes.image, {
                src: element.src,
                alt: element.alt || null,
                title: element.title || null,
                width: element.width || null,
                height: element.height || null,
              });
              return;
            }

            if (isAllowListedTag(state, tagName)) {
              state.openNode(state.schema.nodes.html_inline, {
                tag: tagName,
                htmlAttrs: extractHtmlAttrs(element, tagName),
              });
              return;
            }
          }
        }

        if (isAllowListedTag(state, tagName)) {
          state.openNode(state.schema.nodes.html_inline, { tag: tagName });
        }

        return;
      }

      if (closeMatch) {
        tagName = closeMatch[1].toLowerCase();

        if (closeHtmlInline(state, tagName)) {
          return;
        }

        if (tagName === "a") {
          state.closeMark(state.schema.marks.link);
          return;
        }

        markName = HTML_INLINE_MARKS[tagName];
        if (markName) {
          state.closeMark(state.schema.marks[markName].create());
        }
      }
    },
  },
  serializeNode: {
    html_inline: function (state, node) {
      state.write("<" + node.attrs.tag + serializeHtmlAttrs(node.attrs.htmlAttrs) + ">");
      state.renderInline(node);
      state.write("</" + node.attrs.tag + ">");
    },
  },
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var Decoration = params.pmView.Decoration;
    var DecorationSet = params.pmView.DecorationSet;

    function outlineDecorations(doc) {
      var decorations = [];
      doc.descendants(function (node, pos) {
        if (node.type.name !== "html_inline") {
          return;
        }

        var tag = node.attrs.tag;
        var htmlAttrs = node.attrs.htmlAttrs;
        if (STYLED_TAGS.indexOf(tag) >= 0) {
          return;
        }

        var attrs = { class: "composer-html-inline-node" };
        if (!(htmlAttrs && htmlAttrs.title)) {
          attrs.title = "<" + tag + serializeHtmlAttrs(htmlAttrs) + ">";
        }
        decorations.push(Decoration.node(pos, pos + node.nodeSize, attrs));
      });
      return DecorationSet.create(doc, decorations);
    }

    return new Plugin({
      state: {
        init: function (_, state) {
          return outlineDecorations(state.doc);
        },
        apply: function (tr, set) {
          return tr.docChanged ? outlineDecorations(tr.doc) : set;
        },
      },
      props: {
        decorations: function (state) {
          return this.getState(state);
        },
      },
    });
  },
  inputRules: {
    match: new RegExp("<(" + INPUT_RULE_TAGS.join("|") + ")>$", "i"),
    handler: function (state, match, start, end) {
      var tag = match[1];

      var markName = HTML_INLINE_MARKS[tag];

      var tr = state.tr;

      if (markName) {
        tr.delete(start, end);
        tr.insertText(" ");
        tr.addMark(start, start + 1, state.schema.marks[markName].create());
        tr.removeStoredMark(state.schema.marks[markName]);
      } else {
        tr.replaceWith(start, end, state.schema.nodes.html_inline.create({ tag: tag }, [state.schema.text(" ")]));

        start += 1;
      }

      tr.insertText(" ");
      tr.setSelection(state.selection.constructor.create(tr.doc, start, start + 1));

      return tr;
    },
  },
  commands: function (params) {
    var schema = params.schema;
    var i18n = params.getContext().i18n;
    return {
      toggleSmall: function () {
        return function (state, dispatch) {
          var smallRanges = selectedSmallRanges(state, schema);
          var tr;

          if (smallRanges.length) {
            if (dispatch) {
              dispatch(unwrapSmallRanges(state, smallRanges, schema));
            }
            return true;
          }

          var lines = selectedLines(state).filter(function (line) {
            return line.node.content.size > 0;
          });

          if (!lines.length && state.selection.empty) {
            var first = selectedLines(state)[0];
            if (!first) {
              return false;
            }

            var text = i18n("composer.heading_level_small_text");
            tr = state.tr;
            if (first.node.type === schema.nodes.heading) {
              tr.setNodeMarkup(first.pos, schema.nodes.paragraph, {});
            }
            tr.replaceWith(first.pos + 1, first.pos + 1, schema.nodes.html_inline.create({ tag: SMALL }, schema.text(text)));
            tr.setSelection(TextSelection.create(tr.doc, first.pos + 2, first.pos + 2 + text.length));
            if (dispatch) {
              dispatch(tr);
            }
            return true;
          }

          if (!lines.length) {
            return false;
          }

          tr = replaceLines(
            state,
            lines,
            function (node) {
              var split = splitTrailingWhitespace(withoutSmall(node.content, schema));
              return Fragment.fromArray([schema.nodes.html_inline.create({ tag: SMALL }, split[0])].concat(split[1]));
            },
            schema
          );

          // The caret stays inside the wrapper, so the option shows as on
          // and toggles back off with the same selection. Absorbing an
          // inner wrapper shortens the line, hence the clamp.
          if (state.selection.empty) {
            var contentStart = tr.mapping.map(lines[0].pos) + 2;
            var wrapper = tr.doc.nodeAt(contentStart - 1);

            if (wrapper) {
              tr.setSelection(
                TextSelection.create(tr.doc, Math.min(state.selection.from + 1, contentStart + wrapper.content.size))
              );
            }
          }

          if (dispatch) {
            dispatch(tr);
          }
          return true;
        };
      },

      removeSmall: function () {
        return function (state, dispatch) {
          var lines = selectedLines(state).filter(function (line) {
            return containsSmall(line.node, schema);
          });

          if (!lines.length) {
            return false;
          }

          if (dispatch) {
            dispatch(
              replaceLines(state, lines, function (node) {
                return withoutSmall(node.content, schema);
              })
            );
          }
          return true;
        };
      },
    };
  },
  state: function (params, state) {
    return { inSmall: selectedSmallRanges(state, params.schema).length > 0 };
  },
};
