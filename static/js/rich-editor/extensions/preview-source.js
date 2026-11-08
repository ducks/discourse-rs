// extensions/preview-source.js: the source of a block shown rendered by
// node-views/preview.js. Highlighted like a code block (code-block.js'
// highlight plugin takes preview_source too), but the language belongs to
// the feature rather than the author, so there is no language picker.
export default {
  nodeSpec: {
    preview_source: {
      attrs: { params: { default: "" } },
      content: "text*",
      code: true,
      defining: true,
      // clicks resolve to the block around it, which is the unit to select
      selectable: false,
      marks: "",
      parseDOM: [{ tag: "pre.preview-source", preserveWhitespace: "full" }],
      toDOM: function () {
        return ["pre", { class: "preview-source" }, ["code", 0]];
      },
    },
  },

  serializeNode: {
    preview_source: function (state, node) {
      state.text(node.textContent, false);
    },
  },
};
