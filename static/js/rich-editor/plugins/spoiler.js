// spoiler-alert's lib/rich-editor-extension.js: [spoiler] as a spoiler
// block (wrap_bbcode div.spoiler in the node tree) or an inline spoiler
// (bbcode_spoiler), blurred until the selection is in it.
var SPOILER_NODES = ["inline_spoiler", "spoiler"];

export default {
  nodeSpec: {
    spoiler: {
      attrs: { blurred: { default: true } },
      group: "block",
      content: "block+",
      createGapCursor: true,
      parseDOM: [{ tag: "div.spoiled" }],
      toDOM: function () {
        return ["div", { class: "spoiled" }, 0];
      },
    },
    inline_spoiler: {
      attrs: { blurred: { default: true } },
      group: "inline",
      inline: true,
      content: "inline*",
      parseDOM: [{ tag: "span.spoiled" }],
      toDOM: function () {
        return ["span", { class: "spoiled" }, 0];
      },
    },
  },
  parse: {
    bbcode_spoiler: { block: "inline_spoiler" },
    wrap_bbcode: function (state, token) {
      if (token.nesting === 1 && token.attrGet("class") === "spoiler") {
        state.openNode(state.schema.nodes.spoiler);
        return true;
      } else if (token.nesting === -1 && state.top().type.name === "spoiler") {
        state.closeNode();
        return true;
      }
    },
  },
  serializeNode: {
    spoiler: function (state, node) {
      state.write("[spoiler]\n");
      state.renderContent(node);
      state.write("[/spoiler]\n\n");
    },
    inline_spoiler: function (state, node) {
      state.write("[spoiler]");
      state.renderInline(node);
      state.write("[/spoiler]");
    },
  },
  inputRules: function (params) {
    var TextSelection = params.pmState.TextSelection;
    var i18n = params.getContext().i18n;
    return {
      match: /\[spoiler\]$/,
      handler: function (state, match, start, end) {
        var schema = state.schema;
        var textNode = schema.text(i18n("composer.spoiler_text"));

        var atStart = state.doc.resolve(start).parentOffset === 0;

        var tr = state.tr;

        if (atStart) {
          tr.replaceWith(
            start - 1,
            end,
            schema.nodes.spoiler.createAndFill(null, schema.nodes.paragraph.createAndFill(null, textNode))
          );
        } else {
          tr.replaceWith(start, end, schema.nodes.inline_spoiler.createAndFill(null, textNode));
        }

        return tr.setSelection(TextSelection.create(tr.doc, start + 1, start + 1 + textNode.nodeSize));
      },
    };
  },
  state: function (params, state) {
    return {
      inSpoiler: SPOILER_NODES.some(function (nodeType) {
        return params.utils.inNode(state, params.schema.nodes[nodeType]);
      }),
    };
  },
  commands: function (params) {
    var schema = params.schema;
    var utils = params.utils;
    var TextSelection = params.pmState.TextSelection;
    var pmCommands = params.pmCommands;
    var i18n = params.getContext().i18n;

    return {
      toggleSpoiler: function () {
        return function (state, dispatch, view) {
          var selection = state.selection;
          var empty = selection.empty;
          var $from = selection.$from;
          var $to = selection.$to;

          var inSpoiler = SPOILER_NODES.some(function (nodeType) {
            return utils.inNode(state, schema.nodes[nodeType]);
          });

          if (inSpoiler) {
            // The nearest spoiler, replaced by what it holds.
            for (var depth = $from.depth; depth > 0; depth--) {
              var node = $from.node(depth);
              if (SPOILER_NODES.indexOf(node.type.name) !== -1) {
                var spoilerStart = $from.before(depth);
                var spoilerEnd = spoilerStart + node.nodeSize;

                var unwrapped = state.tr.replaceWith(spoilerStart, spoilerEnd, node.content);

                if (dispatch) {
                  dispatch(unwrapped);
                }

                return true;
              }
            }

            return false;
          }

          // An empty selection: a spoiler with placeholder text.
          if (empty) {
            var textNode = schema.text(i18n("composer.spoiler_text"));
            var isBlockSpoiler = view.endOfTextblock("backward");
            var spoilerNode = isBlockSpoiler
              ? schema.nodes.spoiler.createAndFill(null, schema.nodes.paragraph.createAndFill(null, textNode))
              : schema.nodes.inline_spoiler.createAndFill(null, textNode);

            var tr = state.tr.replaceSelectionWith(spoilerNode);
            tr.setSelection(TextSelection.create(tr.doc, $from.pos + 1, $from.pos + 1 + textNode.nodeSize));

            if (dispatch) {
              dispatch(tr);
            }

            return true;
          }

          var isBlockNodeSelection =
            $from.parent === $to.parent &&
            $from.parentOffset === 0 &&
            $to.parentOffset === $from.parent.content.size &&
            $from.parent.isBlock &&
            $from.depth > 0;
          if (isBlockNodeSelection) {
            return pmCommands.wrapIn(schema.nodes.spoiler)(state, dispatch);
          }

          var slice = selection.content();
          var isInlineSelection = slice.openStart > 0 || slice.openEnd > 0;

          if (isInlineSelection) {
            var content = [];
            slice.content.forEach(function (child) {
              if (child.isBlock) {
                child.content.forEach(function (grandchild) {
                  content.push(grandchild);
                });
              } else {
                content.push(child);
              }
            });
            var inlineSpoiler = schema.nodes.inline_spoiler.createAndFill(null, content);

            var inlineTr = state.tr.replaceWith($from.pos, $to.pos, inlineSpoiler);
            inlineTr.setSelection(
              TextSelection.create(inlineTr.doc, $from.pos + 1, $from.pos + 1 + inlineSpoiler.content.size)
            );

            if (dispatch) {
              dispatch(inlineTr);
            }

            return true;
          } else {
            return pmCommands.wrapIn(schema.nodes.spoiler)(state, function (wrapTr) {
              wrapTr.setSelection(TextSelection.create(wrapTr.doc, $from.pos + 2, $to.pos));

              if (dispatch) {
                dispatch(wrapTr);
              }
            });
          }
        };
      },
    };
  },
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var Decoration = params.pmView.Decoration;
    var DecorationSet = params.pmView.DecorationSet;

    return new Plugin({
      props: {
        decorations: function (state) {
          return this.getState(state);
        },
      },
      state: {
        init: function (config, state) {
          // Every spoiler starts blurred.
          var decorations = [];

          state.doc.descendants(function (node, pos) {
            if (SPOILER_NODES.indexOf(node.type.name) !== -1) {
              decorations.push(Decoration.node(pos, pos + node.nodeSize, { class: "spoiler-blurred" }));
            }
            return true;
          });

          return DecorationSet.create(state.doc, decorations);
        },
        apply: function (tr, set, oldState, newState) {
          if (tr.getMeta(this)) {
            return tr.getMeta(this);
          }

          set = set.map(tr.mapping, tr.doc);

          if (tr.selectionSet || tr.docChanged) {
            var decorations = [];
            var selection = newState.selection;

            newState.doc.descendants(function (node, pos) {
              if (SPOILER_NODES.indexOf(node.type.name) !== -1) {
                var nodeStart = pos;
                var nodeEnd = pos + node.nodeSize;

                var cursorInSpoiler =
                  (selection.from > nodeStart && selection.from < nodeEnd) ||
                  (selection.to > nodeStart && selection.to < nodeEnd) ||
                  (selection.from <= nodeStart && selection.to >= nodeEnd);

                if (!cursorInSpoiler) {
                  decorations.push(Decoration.node(nodeStart, nodeEnd, { class: "spoiler-blurred" }));
                }
              }
              return true;
            });

            return DecorationSet.create(newState.doc, decorations);
          }

          return set;
        },
      },
    });
  },
};
