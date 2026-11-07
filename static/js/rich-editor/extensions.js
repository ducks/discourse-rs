// The rich editor's extensions (lib/composer/rich-editor-extensions.ts) and
// the core ones from static/prosemirror/extensions/. An extension is a plain
// object with any of: nodeSpec, markSpec (schema.js), parse (parser.js),
// serializeNode, serializeMark (serializer.js), inputRules (inputrules.js),
// keymap (keymap.js), plugins, nodeViews, commands, state (editor.js), each
// a value or a function of the plugin params (editor.js pluginParams).
//
// register-default.ts's list less what later steps bring (ROADMAP 196r2,
// 196r3, 196r4): emoji, image, onebox and its toolbar, link and its
// toolbar, code block, quote, hashtag, mention, html inline and block,
// table, markdown and word paste, wrap, trailing inline space, drag ghost,
// grid, upload placeholder, preview source and toolbar.
import { schema } from "prosemirror-markdown";
import codemark from "prosemirror-codemark";

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
  coreExtensions.forEach(registerRichEditorExtension);
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

// register-default.ts's order, of the ones here.
export var coreExtensions = [
  code,
  heading,
  strikethrough,
  strong,
  underline,
  trailingParagraph,
  orderedList,
  bulletList,
  hardBreak,
];

coreExtensions.forEach(registerRichEditorExtension);
