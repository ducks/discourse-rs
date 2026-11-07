// discourse-details' lib/rich-editor-extension.js: [details="Summary"] as a
// details node (bbcode_open details and summary in the node tree) whose
// summary toggles it open.
export default {
  nodeSpec: {
    details: {
      attrs: { open: { default: true } },
      content: "summary block+",
      group: "block",
      selectable: true,
      defining: true,
      isolating: true,
      parseDOM: [{ tag: "details" }],
      toDOM: function (node) {
        return ["details", { open: node.attrs.open || undefined }, 0];
      },
    },
    summary: {
      content: "inline*",
      parseDOM: [{ tag: "summary" }],
      toDOM: function () {
        return ["summary", 0];
      },
    },
  },
  parse: {
    bbcode_open: function (state, token) {
      if (token.tag === "details") {
        state.openNode(state.schema.nodes.details);
        return true;
      }

      if (token.tag === "summary") {
        state.openNode(state.schema.nodes.summary);
        return true;
      }
    },
    bbcode_close: function (state, token) {
      if (token.tag === "details" || token.tag === "summary") {
        state.closeNode();
        return true;
      }
    },
  },
  serializeNode: {
    details: function (state, node) {
      state.renderContent(node);
      state.write("[/details]\n\n");
    },
    summary: function (state, node) {
      var hasSummary = false;
      // A [details] without a summary.
      if (node.content.childCount === 0) {
        state.write("[details");
      } else {
        hasSummary = true;
        state.write('[details="');
        node.content.forEach(function (child) {
          if (child.text) {
            state.text(child.text.replace(/"/g, "“"), state.inAutolink);
          }
        });
      }
      var finalState = "]\n";
      if (hasSummary) {
        finalState = '"' + finalState;
      }
      state.write(finalState);
    },
  },
  plugins: {
    props: {
      handleClickOn: function (view, pos, node, nodePos) {
        // only a click at the summary's first position toggles
        if (pos > nodePos + 1 || node.type.name !== "summary") {
          return false;
        }

        var details = view.state.doc.nodeAt(nodePos - 1);
        view.dispatch(
          view.state.tr.setNodeMarkup(nodePos - 1, null, {
            open: !details.attrs.open,
          })
        );
        return true;
      },
    },
  },
};
