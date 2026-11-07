// extensions/html-block.js: an HTML block as a code-like html_block node,
// its text written back as it is. The cook ends an HTML block at a blank
// line, so the node never holds two newlines in a row.
export default {
  nodeSpec: {
    html_block: {
      attrs: { params: { default: "html" } },
      group: "block",
      createGapCursor: true,
      content: "text*",
      code: true,
      defining: true,
      marks: "",
      selectable: true,
      parseDOM: [{ priority: 60, tag: "pre.html-block", preserveWhitespace: "full" }],
      toDOM: function () {
        return ["pre", { class: "html-block" }, ["code", 0]];
      },
    },
  },
  parse: {
    html_block: function (state, token) {
      state.openNode(state.schema.nodes.html_block);
      state.addText(token.content.trim());
      state.closeNode();
    },
  },
  serializeNode: {
    html_block: function (state, node) {
      state.text(node.textContent, false);
      state.write("\n\n");
    },
  },
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var changedDescendants = params.utils.changedDescendants;

    return new Plugin({
      appendTransaction: function (transactions, prevState, state) {
        if (
          !transactions.some(function (tr) {
            return tr.docChanged;
          })
        ) {
          return null;
        }

        var tr = state.tr;
        changedDescendants(prevState.doc, state.doc, function (node, pos) {
          if (node.type.name === "html_block") {
            var content = node.textContent;
            var normalized = content.replace(/\n{2,}/g, "\n");

            if (content !== normalized) {
              var textNode = node.content.firstChild;
              tr.replaceWith(pos + 1, pos + 1 + textNode.nodeSize, state.schema.text(normalized));
            }
          }
        });

        return tr;
      },
    });
  },
};
