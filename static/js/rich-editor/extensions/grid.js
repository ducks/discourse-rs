// extensions/grid.js: [grid] (bbcode_open div.d-image-grid in the node
// tree) as a grid of its images, each image kept in a paragraph of its own
// and an empty grid removed. The node view is node-views/grid.js.
import { GridNodeView } from "discourse/rich-editor/node-views/grid";

export default {
  nodeViews: {
    grid: function (params) {
      return function (node, view, getPos) {
        return new GridNodeView(node, view, getPos, params);
      };
    },
  },

  nodeSpec: {
    grid: {
      content: "block+",
      group: "block",
      attrs: {
        mode: { default: "grid" },
      },
      createGapCursor: true,
      parseDOM: [
        {
          tag: "div.d-image-grid",
          getAttrs: function (dom) {
            return { mode: dom.getAttribute("data-mode") || "grid" };
          },
        },
        {
          tag: "div.composer-image-grid",
          getAttrs: function (dom) {
            return { mode: dom.getAttribute("data-mode") || "grid" };
          },
        },
      ],
      toDOM: function (node) {
        return ["div", { class: "composer-image-grid", "data-mode": node.attrs.mode }, 0];
      },
    },
  },

  parse: {
    bbcode_open: function (state, token) {
      if (token.attrGet("class") === "d-image-grid") {
        state.openNode(state.schema.nodes.grid, {
          mode: token.attrGet("data-mode") || "grid",
        });
        return true;
      }
    },
    bbcode_close: function (state) {
      if (state.top().type.name === "grid") {
        state.closeNode();
        return true;
      }
    },
  },

  serializeNode: {
    grid: function (state, node) {
      var attrs = "";
      if (node.attrs.mode && node.attrs.mode !== "grid") {
        attrs += " mode=" + node.attrs.mode;
      }
      state.write("\n[grid" + attrs + "]\n\n");
      state.renderContent(node.content);
      state.write("\n[/grid]\n\n");
    },
  },

  inputRules: function () {
    return {
      match: /^\[grid]$/,
      handler: function (state, match, start, end) {
        var grid = state.schema.nodes.grid.createAndFill();
        return state.tr.replaceWith(start - 1, end, grid);
      },
    };
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    return new Plugin({
      appendTransaction: function (transactions, oldState, newState) {
        if (
          !transactions.some(function (tr) {
            return tr.docChanged;
          })
        ) {
          return null;
        }

        var tr = newState.tr;
        var modified = false;

        var gridNodes = [];
        newState.doc.descendants(function (node, pos) {
          if (node.type.name === "grid") {
            gridNodes.push({ node: node, pos: pos });
          }
        });

        gridNodes.reverse().forEach(function (entry) {
          var node = entry.node;
          if (node.childCount === 0) {
            tr.delete(entry.pos, entry.pos + node.nodeSize);
            modified = true;
            return;
          }

          var changes = [];
          var currentPos = entry.pos + 1;

          node.content.forEach(function (child) {
            if (child.type.name === "paragraph") {
              if (child.content.size === 0) {
                if (node.childCount > 1) {
                  changes.push({ type: "remove", from: currentPos, to: currentPos + child.nodeSize });
                }
              } else {
                var images = [];
                child.content.forEach(function (grandchild) {
                  if (grandchild.type.name === "image") {
                    images.push(grandchild);
                  }
                });

                if (images.length > 1) {
                  changes.push({ type: "split", from: currentPos, to: currentPos + child.nodeSize, images: images });
                }
              }
            }
            currentPos += child.nodeSize;
          });

          changes.reverse().forEach(function (change) {
            if (change.type === "remove") {
              tr.delete(change.from, change.to);
            } else {
              tr.replaceWith(
                change.from,
                change.to,
                change.images.map(function (img) {
                  return newState.schema.nodes.paragraph.create({}, [img]);
                })
              );
            }
            modified = true;
          });
        });

        return modified ? tr.setMeta("addToHistory", false) : null;
      },
    });
  },
};
