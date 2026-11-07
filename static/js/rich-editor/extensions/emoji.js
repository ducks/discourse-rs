// extensions/emoji.js: `:code:` as an emoji node, typed or parsed, and
// the only-emoji class on a block holding one to three emoji and nothing
// else. The images and emoticons are the engine's (engine.js emojiImage,
// emojiTranslations: buildEmojiUrl, isCustomEmoji, emojiExists and
// pretty-text/emoji/data's translations).
import { Plugin } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import { emojiImage, emojiTranslations } from "discourse/rich-editor/engine";
import { getChangedRanges, isBoundary } from "discourse/rich-editor/plugin-utils";

function escapeRegExp(string) {
  return string.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function createOnlyEmojiPlugin() {
  return new Plugin({
    state: {
      init: function () {
        return DecorationSet.empty;
      },
      apply: function (tr, oldSet, oldState, newState) {
        if (!tr.docChanged) {
          return oldSet.map(tr.mapping, tr.doc);
        }

        var changedRanges = getChangedRanges(tr);
        var newSet = oldSet.map(tr.mapping, tr.doc);

        changedRanges.forEach(function (change) {
          newState.doc.nodesBetween(change.new.from, change.new.to, function (node, pos) {
            if (!node.isTextblock) {
              return true;
            }

            var blockFrom = pos;
            var blockTo = pos + node.nodeSize;

            newSet = newSet.remove(newSet.find(blockFrom, blockTo));

            var emojiNodes = [];
            var hasOnlyEmojis = true;

            node.descendants(function (child, childPos) {
              if (child.type.name === "emoji") {
                emojiNodes.push({
                  from: blockFrom + 1 + childPos,
                  to: blockFrom + 1 + childPos + child.nodeSize,
                });
                return true;
              }

              if (child.type.name === "text" && !(child.text && child.text.trim())) {
                return true;
              }

              hasOnlyEmojis = false;
              return false;
            });

            if (emojiNodes.length > 0 && emojiNodes.length <= 3 && hasOnlyEmojis) {
              newSet = newSet.add(
                newState.doc,
                emojiNodes.map(function (emoji) {
                  return Decoration.inline(emoji.from, emoji.to, { class: "only-emoji" });
                })
              );
            }

            return false;
          });
        });

        return newSet;
      },
    },
    props: {
      decorations: function (state) {
        return this.getState(state);
      },
    },
  });
}

// The emoticons as one pattern (the editor sets the engine before its
// input rules are built).
function shortcutPattern() {
  var keys = Object.keys(emojiTranslations());
  return new RegExp("(^|\\W)(" + keys.map(escapeRegExp).join("|") + ") $");
}

export default {
  nodeSpec: {
    emoji: {
      attrs: { code: {} },
      inline: true,
      group: "inline",
      draggable: true,
      selectable: false,
      parseDOM: [
        {
          tag: "img.emoji",
          getAttrs: function (dom) {
            return { code: dom.getAttribute("alt").replace(/:/g, "") };
          },
          priority: 60,
        },
      ],
      toDOM: function (node) {
        var code = node.attrs.code.toLowerCase();
        var title = ":" + code + ":";
        var image = emojiImage(code);

        return [
          "img",
          {
            class: image && image.custom ? "emoji emoji-custom" : "emoji",
            alt: title,
            title: title,
            src: image ? image.url : undefined,
          },
        ];
      },
    },
  },

  inputRules: function (params) {
    var siteSettings = params.getContext().siteSettings || {};
    return [
      {
        match: /(^|\W):([^:]+):$/,
        handler: function (state, match, start, end) {
          if (emojiImage(match[2])) {
            var emojiStart = start + match[1].length;
            var emojiNode = state.schema.nodes.emoji.create({ code: match[2] });
            var tr = state.tr.replaceWith(emojiStart, end, emojiNode);

            state.doc
              .resolve(emojiStart)
              .marks()
              .forEach(function (mark) {
                tr.addMark(emojiStart, emojiStart + 1, mark);
              });
            return tr;
          }
        },
        options: { undoable: false },
      },
      {
        match: shortcutPattern(),
        handler: function (state, match, start, end) {
          if (!siteSettings.enable_emoji_shortcuts) {
            return;
          }
          var emojiStart = start + match[1].length;
          var emojiNode = state.schema.nodes.emoji.create({
            code: emojiTranslations()[match[2]],
          });
          var tr = state.tr.replaceWith(emojiStart, end, emojiNode).insertText(" ");

          state.doc
            .resolve(emojiStart)
            .marks()
            .forEach(function (mark) {
              tr.addMark(emojiStart, emojiStart + 2, mark);
            });
          return tr;
        },
      },
    ];
  },

  parse: {
    emoji: {
      node: "emoji",
      getAttrs: function (token) {
        return { code: token.attrGet("alt").slice(1, -1) };
      },
    },
  },

  serializeNode: {
    emoji: function (state, node) {
      state.flushClose();
      if (!isBoundary(state.out, state.out.length - 1)) {
        state.write(" ");
      }

      state.write(":" + node.attrs.code + ":");
    },
  },

  plugins: function () {
    return [createOnlyEmojiPlugin()];
  },
};
