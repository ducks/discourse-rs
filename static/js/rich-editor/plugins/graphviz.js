// discourse-graphviz's lib/rich-editor-extension.js: [graphviz] as a
// preview block (node-views/preview.js) over its dot source, its diagram
// drawn by graphviz.js (Discourse.graphvizDiagram, GraphvizPreview), the
// preview toolbar's Expand diagram (GraphvizFullscreen), and the
// [graphviz] input rule putting the caret in a new diagram's source.
import { previewNodeView, previewSourceNode, selectPreviewSource } from "discourse/rich-editor/node-views/preview";

var LANGUAGE = "dot";

// kept in step with the engines the markdown rule accepts
var ENGINES = ["dot", "neato", "circo", "fdp", "osage", "twopi"];

function toEngine(engine) {
  return ENGINES.indexOf(engine) !== -1 ? engine : "dot";
}

function graphviz() {
  return window.Discourse || {};
}

export default {
  nodeSpec: {
    graphviz: {
      attrs: { engine: { default: "dot" } },
      group: "block",
      content: "preview_source",
      atom: true,
      defining: true,
      isolating: true,
      createGapCursor: true,
      parseDOM: [
        {
          tag: "div.graphviz",
          getAttrs: function (dom) {
            return { engine: toEngine(dom.dataset.engine) };
          },
          // cooked source is plain text the parser would otherwise collapse
          getContent: function (dom, schema) {
            return schema.nodes.graphviz.create(null, previewSourceNode(schema, dom.textContent, LANGUAGE)).content;
          },
        },
      ],
      toDOM: function (node) {
        return ["div", { class: "graphviz", "data-engine": node.attrs.engine }, 0];
      },
      previewControls: [
        {
          id: "graphviz-fullscreen",
          icon: "discourse-expand",
          title: "graphviz.fullscreen",
          className: "composer-preview-toolbar__graphviz-fullscreen",
          action: function (args) {
            if (graphviz().graphvizFullscreen) {
              graphviz().graphvizFullscreen(args.node.textContent, args.node.attrs.engine);
            }
          },
        },
      ],
    },
  },

  nodeViews: {
    // GraphvizPreview: GraphvizDiagram over the source
    graphviz: previewNodeView(function (element, node, source) {
      if (graphviz().graphvizDiagram) {
        element.appendChild(graphviz().graphvizDiagram(source, node.attrs.engine));
      }
    }),
  },

  parse: {
    graphviz: function (state, token) {
      state.openNode(state.schema.nodes.graphviz, { engine: token.attrGet("data-engine") });
      state.openNode(state.schema.nodes.preview_source, { params: LANGUAGE });
      state.addText(token.content.trim());
      state.closeNode();
      state.closeNode();

      return true;
    },
  },

  inputRules: function (params) {
    var schema = params.schema;
    var TextSelection = params.pmState.TextSelection;
    return {
      match: /\[graphviz(?: engine=(\w+))?]$/,
      handler: function (state, match, start, end) {
        var node = schema.nodes.graphviz.create({ engine: toEngine(match[1]) }, previewSourceNode(schema, "", LANGUAGE));
        var isAtStart = state.doc.resolve(start).parentOffset === 0;
        var from = isAtStart ? start - 1 : start;

        return selectPreviewSource(state.tr.replaceWith(from, end, node), TextSelection, from);
      },
    };
  },

  serializeNode: {
    graphviz: function (state, node) {
      var engine = node.attrs.engine;

      state.write("[graphviz" + (engine === "dot" ? "" : " engine=" + engine) + "]\n");
      state.text(node.textContent, false);
      state.write("\n[/graphviz]");
      state.closeBlock(node);
    },
  },
};
