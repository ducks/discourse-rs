// The markdown engine the extensions ask (static/prosemirror/lib/
// markdown-it.js): Discourse's builds a markdown-it engine once and hands
// out its parser, its linkify and its allow list. The port's engine is the
// markdown crate as WebAssembly (crates/markdown/src/wasm.rs), configured
// with the site's settings, which answers the same questions
// (docs/rich-editor.md, "The engine"):
//
//   tree(markdown)          the node tree the parser reads
//   linkify                 { test, match, matchAtStart }: linkify-it's,
//                           offsets in UTF-16 units
//   allowList               { tags, attrAllowed(tag, name, value) }
//   emoji(code)             { url, custom } of a code that exists, or null
//   emojiTranslations       emoticon -> emoji name
//   emojiUnescape(text)     lib/text's emojiUnescape: the text's emoji as
//                           images (HTML)
//
// createEditor sets it (editor.js); like the reference's, it is one per
// page.
var current = null;

export function setEngine(engine) {
  current = engine || null;
}

export function getEngine() {
  return current;
}

function required() {
  if (!current) {
    throw new Error("the rich editor has no markdown engine: markdown.wasm is not loaded");
  }
  return current;
}

// discourse/lib/link-matcher's LinkMatcher
export function getLinkify() {
  return required().linkify;
}

// pretty-text's AllowLister, as far as html-inline reads it.
export function getAllowLister() {
  return required().allowList;
}

export function emojiImage(code) {
  return current ? current.emoji(code) : null;
}

export function emojiTranslations() {
  return current ? current.emojiTranslations : {};
}

export function emojiUnescape(text) {
  return current ? current.emojiUnescape(text) : text;
}

// The engine over markdown.wasm's exports, configured: `call(fn, text)`
// runs one export over the text and returns its output, throwing on a
// failure (as composer.js's loadRenderer does).
export function wasmEngine(exports, call) {
  var json = function (fn, text) {
    return JSON.parse(call(fn, text));
  };
  var tags = new Set(json(exports.html_allow_list, ""));
  var attrCache = new Map();
  var translations = json(exports.emoji_translations, "");
  var emojiCache = new Map();

  return {
    tree: function (markdown) {
      return json(exports.rich_editor_tree, markdown);
    },
    linkify: {
      test: function (text) {
        return !!text && json(exports.linkify_match, text) !== null;
      },
      match: function (text) {
        return text ? json(exports.linkify_match, text) : null;
      },
      matchAtStart: function (text) {
        return text ? json(exports.linkify_match_at_start, text) : null;
      },
    },
    allowList: {
      tags: tags,
      // null: the tag is not on the list.
      attrAllowed: function (tag, name, value) {
        var key = JSON.stringify([tag, name, value]);
        if (!attrCache.has(key)) {
          attrCache.set(key, json(exports.html_attr_allowed, key));
        }
        return attrCache.get(key);
      },
    },
    emoji: function (code) {
      if (!emojiCache.has(code)) {
        emojiCache.set(code, json(exports.emoji_image, code));
      }
      return emojiCache.get(code);
    },
    emojiTranslations: translations,
    emojiUnescape: function (text) {
      return json(exports.emoji_unescape, text);
    },
  };
}
