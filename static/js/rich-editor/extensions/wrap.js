// extensions/wrap.js: [wrap=name attr=value] as a wrap block (wrap_open
// div.d-wrap in the node tree) or an inline wrap (span.d-wrap), its
// attributes kept as a flat object (wrap-utils.js), drawn by
// node-views/wrap.js; the insertWrap, updateWrap and removeWrap commands.
import { WrapNodeView } from "discourse/rich-editor/node-views/wrap";
import { camelCaseToDash, parseAttributesString, serializeAttributes } from "discourse/rich-editor/wrap-utils";

function toDataAttrs(data) {
  var attrs = {};
  if (!data) {
    return attrs;
  }
  Object.keys(data).forEach(function (key) {
    var value = data[key];
    if (value !== null && value !== undefined && value !== "") {
      attrs["data-" + camelCaseToDash(key)] = value;
    }
  });
  return attrs;
}

function wrapNodeView(params) {
  return function (node, view, getPos) {
    return new WrapNodeView(node, view, getPos, params);
  };
}

function isWrap(node) {
  return node.type.name === "wrap_block" || node.type.name === "wrap_inline";
}

export default {
  nodeViews: {
    wrap_block: wrapNodeView,
    wrap_inline: wrapNodeView,
  },

  commands: function (params) {
    var schema = params.schema;

    return {
      insertWrap: function (attributes) {
        return function (state, dispatch, view) {
          var selection = state.selection;
          var $from = selection.$from;
          var $to = selection.$to;

          var attrs = { data: attributes || {} };
          var tr = state.tr;
          var wrapType;

          if (selection.empty) {
            // Block or inline by where the cursor is.
            var isBlockWrap = view && view.endOfTextblock && view.endOfTextblock("backward");
            wrapType = isBlockWrap ? schema.nodes.wrap_block : schema.nodes.wrap_inline;

            if (!wrapType) {
              return false;
            }

            var wrap = isBlockWrap ? wrapType.create(attrs, schema.nodes.paragraph.create()) : wrapType.create(attrs);

            tr.replaceSelectionWith(wrap);

            if (isBlockWrap) {
              tr.setSelection(selection.constructor.create(tr.doc, $from.pos + 2));
            }
          } else {
            var isBlockSelection =
              $from.parent === $to.parent &&
              $from.parentOffset === 0 &&
              $to.parentOffset === $from.parent.content.size &&
              $from.parent.isBlock &&
              $from.depth > 0;

            if (isBlockSelection) {
              wrapType = schema.nodes.wrap_block;
              if (!wrapType) {
                return false;
              }

              var range = $from.blockRange($to);
              if (!range) {
                return false;
              }

              tr.wrap(range, [{ type: wrapType, attrs: attrs }]);
            } else {
              wrapType = schema.nodes.wrap_inline;
              if (!wrapType) {
                return false;
              }

              var content = state.doc.slice(selection.from, selection.to);
              tr.replaceWith(selection.from, selection.to, wrapType.create(attrs, content.content));
            }
          }

          if (dispatch) {
            dispatch(tr);
          }
          return true;
        };
      },

      updateWrap: function (attributesString) {
        return function (state, dispatch) {
          var $from = state.selection.$from;

          for (var depth = $from.depth; depth >= 0; depth--) {
            var node = $from.node(depth);
            if (isWrap(node)) {
              var pos = $from.before(depth);
              var tr = state.tr.setNodeMarkup(pos, null, { data: parseAttributesString(attributesString) });
              if (dispatch) {
                dispatch(tr);
              }
              return true;
            }
          }

          return false;
        };
      },

      removeWrap: function () {
        return function (state, dispatch) {
          var $from = state.selection.$from;

          for (var depth = $from.depth; depth >= 0; depth--) {
            var node = $from.node(depth);
            if (isWrap(node)) {
              var pos = $from.before(depth);
              var tr = state.tr;

              if (node.content.size === 0) {
                tr.delete(pos, pos + node.nodeSize);
              } else {
                tr.replaceWith(pos, pos + node.nodeSize, node.content);
              }

              if (dispatch) {
                dispatch(tr);
              }
              return true;
            }
          }

          return false;
        };
      },
    };
  },

  state: function (params, state) {
    var inWrap =
      params.utils.inNode(state, params.schema.nodes.wrap_block) ||
      params.utils.inNode(state, params.schema.nodes.wrap_inline);

    if (!inWrap) {
      return { inWrap: false };
    }

    var $from = state.selection.$from;
    for (var depth = $from.depth; depth >= 0; depth--) {
      var node = $from.node(depth);
      if (isWrap(node)) {
        return {
          inWrap: true,
          wrapAttributes: serializeAttributes(node.attrs.data || {}),
        };
      }
    }

    return { inWrap: false };
  },

  nodeSpec: {
    wrap_block: {
      content: "block+",
      group: "block",
      defining: true,
      createGapCursor: true,
      attrs: {
        data: { default: {} },
      },
      parseDOM: [
        {
          tag: "div.d-wrap",
          getAttrs: function (dom) {
            return { data: Object.assign({}, dom.dataset) };
          },
        },
      ],
      toDOM: function (node) {
        return ["div", Object.assign({ class: "d-wrap" }, toDataAttrs(node.attrs.data)), 0];
      },
    },

    wrap_inline: {
      inline: true,
      group: "inline",
      content: "inline*",
      defining: true,
      attrs: {
        data: { default: {} },
      },
      parseDOM: [
        {
          tag: "span.d-wrap",
          getAttrs: function (dom) {
            return { data: Object.assign({}, dom.dataset) };
          },
        },
      ],
      toDOM: function (node) {
        return ["span", Object.assign({ class: "d-wrap" }, toDataAttrs(node.attrs.data)), 0];
      },
    },
  },

  parse: {
    wrap_open: function (state, token) {
      var nodeType = token.tag === "span" ? state.schema.nodes.wrap_inline : state.schema.nodes.wrap_block;

      var data = {};
      (token.attrs || []).forEach(function (pair) {
        var name = pair[0];
        var value = pair[1];
        if (name.startsWith("data-")) {
          data[name.slice(5)] = value;
        } else if (name !== "class" || value !== "d-wrap") {
          data[name] = value;
        }
      });
      state.openNode(nodeType, { data: data });
      return true;
    },

    wrap_close: function (state) {
      var top = state.top();
      if (!top) {
        return;
      }
      if (isWrap(top)) {
        state.closeNode();
        return true;
      }
    },
  },

  serializeNode: {
    wrap_block: function (state, node) {
      state.write("[wrap" + serializeAttributes(node.attrs.data) + "]\n");
      state.renderContent(node);
      state.write("[/wrap]\n\n");
    },

    wrap_inline: function (state, node) {
      state.write("[wrap" + serializeAttributes(node.attrs.data) + "]");
      state.renderInline(node);
      state.write("[/wrap]");
    },
  },

  inputRules: function (params) {
    var i18n = params.getContext().i18n;
    return {
      match: /\[wrap([^\]]*)]$/,
      handler: function (state, match, start, end) {
        var schema = state.schema;
        var isAtStart = state.doc.resolve(start).parentOffset === 0;
        var attributes = parseAttributesString(match[1] || "");
        var tr = state.tr;

        if (isAtStart) {
          var wrapNode = schema.nodes.wrap_block.createAndFill(
            { data: attributes },
            schema.nodes.paragraph.createAndFill()
          );

          tr.replaceWith(start - 1, end, wrapNode);
        } else {
          // An inline wrap starts with placeholder text.
          var inlineNode = schema.nodes.wrap_inline.createAndFill(
            { data: attributes },
            schema.text(i18n("composer.wrap_text"))
          );

          tr.replaceWith(start, end, inlineNode);
        }

        return tr;
      },
    };
  },
};
