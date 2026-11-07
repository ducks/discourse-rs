// The rich editor's extensions (lib/composer/rich-editor-extensions.ts) and
// the core ones from static/prosemirror/extensions/. An extension is a plain
// object with any of: nodeSpec, markSpec (schema.js), parse (parser.js),
// serializeNode, serializeMark (serializer.js), inputRules (inputrules.js),
// keymap (keymap.js), plugins, nodeViews, commands, state (editor.js),
// transformParsedHTML (to-markdown.js), each a value or a function of the
// plugin params (editor.js pluginParams).
//
// register-default.ts's list, the larger ones in extensions/, less what
// later steps bring (ROADMAP 196r3: onebox and its toolbar, preview source
// and toolbar) and [wrap], which is not ported: its token is refused.
import { schema } from "prosemirror-markdown";
import codemark from "prosemirror-codemark";
import codeBlock from "discourse/rich-editor/extensions/code-block";
import emoji from "discourse/rich-editor/extensions/emoji";
import grid from "discourse/rich-editor/extensions/grid";
import hashtag from "discourse/rich-editor/extensions/hashtag";
import htmlBlock from "discourse/rich-editor/extensions/html-block";
import htmlInline from "discourse/rich-editor/extensions/html-inline";
import image from "discourse/rich-editor/extensions/image";
import link from "discourse/rich-editor/extensions/link";
import linkToolbar from "discourse/rich-editor/extensions/link-toolbar";
import markdownPaste from "discourse/rich-editor/extensions/markdown-paste";
import mention from "discourse/rich-editor/extensions/mention";
import overrideDragGhost from "discourse/rich-editor/extensions/override-drag-ghost";
import quote from "discourse/rich-editor/extensions/quote";
import table from "discourse/rich-editor/extensions/table";
import trailingInlineSpace from "discourse/rich-editor/extensions/trailing-inline-space";
import uploadPlaceholder from "discourse/rich-editor/extensions/upload-placeholder";
import wordPaste from "discourse/rich-editor/extensions/word-paste";

var registered = [];

export function registerRichEditorExtension(extension) {
  registered.push(extension);
}

export function getExtensions() {
  return registered.slice();
}

// For the specs: the defaults alone, as the reference's
// clearRichEditorExtensions leaves the editor with prosemirror-markdown's.
export function clearRichEditorExtensions() {
  registered.length = 0;
}

export function resetRichEditorExtensions() {
  registered.length = 0;
  defaultExtensions.forEach(registerRichEditorExtension);
}

// lib/list-utils.js isListTight: no <li> holds a <p> directly.
function isListTight(listElement) {
  var listItems = listElement.querySelectorAll(":scope > li");
  for (var i = 0; i < listItems.length; i++) {
    var children = listItems[i].children;
    for (var j = 0; j < children.length; j++) {
      if (children[j].tagName === "P") {
        return false;
      }
    }
  }
  return true;
}

// extensions/code.ts
export var code = {
  plugins: function (params) {
    return codemark({ markType: params.schema.marks.code });
  },
};

// extensions/heading.js
export var heading = {
  nodeSpec: {
    heading: Object.assign({}, schema.nodes.heading.spec, { content: "inline*" }),
  },
};

// extensions/strikethrough.ts
export var strikethrough = {
  markSpec: {
    strikethrough: {
      before: "link",
      parseDOM: [
        { tag: "s" },
        { tag: "del" },
        { tag: "strike" },
        {
          getAttrs: function (value) {
            return /(^|[\s])line-through([\s]|$)/u.test(value) && null;
          },
          style: "text-decoration",
        },
      ],
      toDOM: function () {
        return ["s"];
      },
    },
  },
  inputRules: function (params) {
    return params.utils.markInputRule(/~~([^~]+)~~$/, params.schema.marks.strikethrough);
  },
  parse: {
    s: { mark: "strikethrough" },
    bbcode_s: { mark: "strikethrough" },
  },
  serializeMark: {
    strikethrough: {
      open: "~~",
      close: "~~",
      mixable: true,
      expelEnclosingWhitespace: true,
    },
  },
};

// extensions/strong.ts
export var strong = {
  markSpec: {
    strong: Object.assign({}, schema.marks.strong.spec, {
      parseDOM: [
        { tag: "strong" },
        {
          tag: "b",
          getAttrs: function (node) {
            return node.style.fontWeight !== "normal" && null;
          },
        },
        {
          style: "font-weight=400",
          clearMark: function (m) {
            return m.type.name === "strong";
          },
        },
        {
          // A copied page inlines theme weights; 500 is CSS medium, not
          // authored bold.
          style: "font-weight",
          getAttrs: function (value) {
            var weight = Number.parseFloat(value);
            return (/^bold(er)?$/.test(value) || (weight >= 600 && weight <= 1000)) && null;
          },
        },
      ],
    }),
  },
};

// extensions/underline.ts
export var underline = {
  markSpec: {
    underline: {
      before: "link",
      toDOM: function () {
        return ["u", 0];
      },
      parseDOM: [{ tag: "u" }],
    },
  },
  inputRules: function (params) {
    return params.utils.markInputRule(/\[u]$/, params.schema.marks.underline);
  },
  keymap: function (params) {
    return { "Mod-u": params.pmCommands.toggleMark(params.schema.marks.underline) };
  },
  parse: {
    bbcode_u: { mark: "underline" },
  },
  serializeMark: {
    underline: {
      open: "[u]",
      close: "[/u]",
      mixable: true,
      expelEnclosingWhitespace: true,
    },
  },
};

// extensions/trailing-paragraph.ts: an empty paragraph stays last, so the
// cursor can always go below the last block.
export var trailingParagraph = {
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var PluginKey = params.pmState.PluginKey;
    var plugin = new PluginKey("trailing-paragraph");

    return new Plugin({
      key: plugin,
      appendTransaction: function (_, __, state) {
        if (!plugin.getState(state)) {
          return;
        }

        return state.tr.setMeta("addToHistory", false).insert(state.doc.content.size, state.schema.nodes.paragraph.create());
      },
      state: {
        init: function (_, state) {
          return !isLastChildParagraph(state);
        },
        apply: function (tr, value) {
          if (!tr.docChanged) {
            return value;
          }

          return !isLastChildParagraph(tr);
        },
      },
    });
  },
};

function isLastChildParagraph(state) {
  return state.doc.lastChild.type.name === "paragraph";
}

// extensions/ordered-list.js
export var orderedList = {
  nodeSpec: {
    ordered_list: Object.assign({}, schema.nodes.ordered_list.spec, {
      attrs: { order: { default: 1 }, tight: { default: true } },
      parseDOM: [
        {
          tag: "ol",
          getAttrs: function (dom) {
            return {
              order: dom.hasAttribute("start") ? +dom.getAttribute("start") : 1,
              tight: dom.hasAttribute("data-tight") || isListTight(dom),
            };
          },
        },
      ],
    }),
  },
};

// extensions/bullet-list.js
export var bulletList = {
  nodeSpec: {
    bullet_list: Object.assign({}, schema.nodes.bullet_list.spec, {
      attrs: { tight: { default: true } },
      parseDOM: [
        {
          tag: "ul",
          getAttrs: function (dom) {
            return { tight: dom.hasAttribute("data-tight") || isListTight(dom) };
          },
        },
      ],
    }),
  },
};

// extensions/hard-break.ts
export var hardBreak = {
  nodeSpec: {
    hard_break: Object.assign({}, schema.nodes.hard_break.spec, { linebreakReplacement: true }),
  },
};

// extensions/placeholder.ts: the editor always has it (editor.js). The
// placeholder shows on the one empty paragraph of an empty document, and a
// transaction with discourseContextChanged { key: "placeholder" } changes it.
export var placeholder = {
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var PluginKey = params.pmState.PluginKey;
    var Decoration = params.pmView.Decoration;
    var DecorationSet = params.pmView.DecorationSet;
    var current;

    return new Plugin({
      key: new PluginKey("placeholder"),
      view: function () {
        current = params.getContext().placeholder;
        return {};
      },
      state: {
        init: function () {
          return current;
        },
        apply: function (tr) {
          var contextChanged = tr.getMeta("discourseContextChanged");
          if (contextChanged && contextChanged.key === "placeholder") {
            current = contextChanged.value;
          }

          return current;
        },
      },
      props: {
        decorations: function (state) {
          var $head = state.selection.$head;

          if (state.doc.childCount === 1 && state.doc.firstChild === $head.parent && isEmptyParagraph($head.parent)) {
            var decoration = Decoration.node($head.before(), $head.after(), {
              "data-placeholder": this.getState(state),
            });
            return DecorationSet.create(state.doc, [decoration]);
          }
        },
      },
    });
  },
};

function isEmptyParagraph(node) {
  return node.type.name === "paragraph" && node.nodeSize === 2;
}

// register-default.ts's order.
export var defaultExtensions = [
  emoji,
  image,
  code,
  link,
  linkToolbar,
  heading,
  codeBlock,
  quote,
  hashtag,
  mention,
  strikethrough,
  strong,
  underline,
  htmlInline,
  htmlBlock,
  trailingParagraph,
  table,
  markdownPaste,
  wordPaste,
  orderedList,
  bulletList,
  trailingInlineSpace,
  overrideDragGhost,
  hardBreak,
  grid,
  uploadPlaceholder,
];

defaultExtensions.forEach(registerRichEditorExtension);
