// extensions/link.ts: the link mark with its markup (null for [text](url),
// "autolink" for <url>, "linkify" for a bare url), attachments
// (`|attachment`) and upload links (`data-orig-href`); the input rules for
// both forms; a pasted url over a selection links it; and as the member
// types, bare urls are linked and unlinked with the engine's linkify, as
// the cooked post will.
import { ReplaceAroundStep, ReplaceStep } from "prosemirror-transform";
import { getChangedRanges, markInputRule } from "discourse/rich-editor/plugin-utils";

function addLinkMark(view, text, utils) {
  var matches = utils.getLinkify().match(text);
  var isFullMatch = matches && matches.length === 1 && matches[0].raw === text;

  if (!isFullMatch) {
    return;
  }

  var from = view.state.selection.from;
  var to = view.state.selection.to;
  var tr = view.state.tr;

  // only ever over a selection, so no markup: linkify
  tr.addMark(from, to, view.state.schema.marks.link.create({ href: text }));

  return tr.doc.slice(from, to);
}

function isAutoLink(mark) {
  return mark.type.name === "link" && mark.attrs.markup !== "linkify";
}

export default {
  markSpec: {
    link: {
      attrs: {
        href: {},
        title: { default: null },
        // the markdown-it token's markup
        markup: { default: null },
        attachment: { default: false },
        "data-orig-href": { default: null },
      },
      inclusive: false,
      parseDOM: [
        {
          tag: "a[href]:not(.lightbox)",
          getAttrs: function (dom) {
            if (dom.textContent.trim() === dom.getAttribute("href")) {
              return false;
            }

            return {
              href: dom.getAttribute("href"),
              title: dom.getAttribute("title"),
              attachment: dom.classList.contains("attachment"),
              "data-orig-href": dom.getAttribute("data-orig-href"),
              markup: dom.getAttribute("data-markup"),
            };
          },
        },
      ],
      toDOM: function (node) {
        return [
          "a",
          {
            href: node.attrs.href,
            title: node.attrs.title,
            class: node.attrs.attachment ? "attachment" : undefined,
            "data-orig-href": node.attrs["data-orig-href"] || undefined,
            "data-markup": node.attrs.markup || undefined,
          },
          0,
        ];
      },
    },
  },

  parse: {
    link: {
      mark: "link",
      getAttrs: function (tok, tokens, i) {
        // The text may span several tokens when it holds inline formatting.
        var attachment = false;
        for (var j = i + 1; j < tokens.length; j++) {
          if (tokens[j].type === "link_close") {
            break;
          }
          if (tokens[j].type === "text" && tokens[j].content.endsWith("|attachment")) {
            tokens[j].content = tokens[j].content.slice(0, -11);
            attachment = true;
            break;
          }
        }

        return {
          href: tok.attrGet("href"),
          title: tok.attrGet("title") || null,
          markup: tok.markup || null,
          attachment: attachment,
          "data-orig-href": tok.attrGet("data-orig-href"),
        };
      },
    },
  },

  serializeMark: {
    // `|attachment` and the link's own markup
    link: {
      open: function (state, mark) {
        state.linkMarkup = mark.attrs.markup;

        if (state.linkMarkup === "autolink") {
          state.inAutolink = true;
          return "<";
        }

        if (state.linkMarkup === "linkify") {
          state.inAutolink = true;
          return "";
        }

        return "[";
      },
      close: function (state, mark) {
        state.inAutolink = undefined;

        var linkMarkup = state.linkMarkup;
        state.linkMarkup = undefined;

        if (linkMarkup === "autolink") {
          return ">";
        }

        if (linkMarkup === "linkify") {
          return "";
        }

        var attachment = mark.attrs.attachment ? "|attachment" : "";
        var origHref = mark.attrs["data-orig-href"];
        var href =
          origHref !== null && origHref !== undefined ? origHref : mark.attrs.href.replace(/[()"]/g, "\\$&");
        var title = mark.attrs.title ? ' "' + mark.attrs.title.replace(/"/g, '\\"') + '"' : "";

        return attachment + "](" + href + title + ")";
      },
      mixable: true,
    },
  },

  inputRules: function (params) {
    var schema = params.schema;
    return [
      markInputRule(/\[([^\]]+)]\(([^)\s]+)(?:\s+[“"']([^“"']+)[”"'])?\)$/, schema.marks.link, function (match) {
        return { href: match[2], title: match[3] };
      }),
      markInputRule(
        // AUTOLINK_RE from markdown-it's lib/rules_inline/autolink.mjs
        // eslint-disable-next-line no-control-regex
        /<([a-zA-Z][a-zA-Z0-9+.-]{1,31}:[^<>\x00-\x20]*)>$/,
        schema.marks.link,
        function (match) {
          return { href: match[1], markup: "autolink" };
        }
      ),
    ];
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var utils = params.utils;

    // A pasted plain-text url over a selection links it.
    function parseClipboardText(text, $context, plain, view) {
      if (view.state.selection.empty || !utils.getLinkify().test(text)) {
        return;
      }

      return addLinkMark(view, text, utils);
    }

    return new Plugin({
      props: {
        clipboardTextParser: parseClipboardText,
        // Pasted rich content that is one text node with a url, over a
        // selection, links it.
        transformPasted: function (slice, view) {
          if (view.state.selection.empty) {
            return slice;
          }

          var node = null;

          if (slice.content.childCount === 1) {
            var only = slice.content.child(0);
            if (only.isText) {
              node = only;
            } else if (only.type.name === "paragraph" && only.childCount === 1 && only.child(0).isText) {
              node = only.child(0);
            }
          }

          if (
            !node ||
            !node.text ||
            node.marks.some(function (mark) {
              return mark.type.name === "link" || mark.type.name === "code";
            }) ||
            !utils.getLinkify().test(node.text)
          ) {
            return slice;
          }

          // Each transformPasted gets the one before's result, so the slice
          // goes on unchanged when the text is not a lone link.
          return addLinkMark(view, node.text, utils) || slice;
        },
      },

      // Links and unlinks bare urls as the member types.
      appendTransaction: function (transactions, prevState, state) {
        var transaction = prevState.tr;
        transactions
          .filter(function (tr) {
            return tr.docChanged && tr.getMeta("addToHistory") !== false;
          })
          .forEach(function (tr) {
            tr.steps.forEach(function (step) {
              if (step instanceof ReplaceStep || step instanceof ReplaceAroundStep) {
                transaction.step(step);
              }
            });
          });

        var changedRanges = getChangedRanges(transaction);

        var tr = state.tr;

        changedRanges.forEach(function (change) {
          var from = change.new.from;
          var to = change.new.to;
          if (from === to) {
            from = Math.max(from - 1, 0);
            to = Math.min(to + 1, state.doc.nodeSize - 2);
          }

          // nodes already handled as a node's nodeAfter
          var visited = new Set();
          state.doc.nodesBetween(from, to, function (node, pos) {
            if (
              visited.has(node) ||
              !node.isText ||
              node.marks.some(function (mark) {
                return isAutoLink(mark) || mark.type.name === "code";
              })
            ) {
              return true;
            }

            var text = node.text;

            var changeStart = Math.max(0, change.new.from - pos);
            var changeEnd = Math.min(text.length, change.new.to - pos);

            var wordStart = changeStart;
            while (wordStart > 0 && !utils.isWhiteSpace(text[wordStart - 1])) {
              wordStart--;
            }
            var wordEnd = changeEnd;
            while (wordEnd < text.length && !utils.isWhiteSpace(text[wordEnd])) {
              wordEnd++;
            }

            var textSlice = text.slice(wordStart, wordEnd);

            var nodeBefore = state.doc.nodeAt(pos - 1);
            var textBefore = "";

            if (
              wordStart === 0 &&
              nodeBefore &&
              nodeBefore.isText &&
              !utils.isWhiteSpace(nodeBefore.text[nodeBefore.text.length - 1]) &&
              !utils.isWhiteSpace(text[0]) &&
              !nodeBefore.marks.some(isAutoLink)
            ) {
              textBefore = nodeBefore.text;
            }

            var nodeAfter = state.doc.nodeAt(pos + node.nodeSize);
            var textAfter = "";
            if (
              wordEnd === text.length &&
              nodeAfter &&
              nodeAfter.isText &&
              !utils.isWhiteSpace(text[text.length - 1]) &&
              !utils.isWhiteSpace(nodeAfter.text[0]) &&
              nodeAfter.marks.length === 1 &&
              nodeAfter.marks.some(function (mark) {
                return mark.type.name === "link" && mark.attrs.markup === "linkify";
              })
            ) {
              textAfter = nodeAfter.text;
              visited.add(nodeAfter);
            }

            var fullText = textBefore + textSlice + textAfter;

            var startPos = pos + wordStart - textBefore.length;

            tr.removeMark(startPos, startPos + fullText.length, state.schema.marks.link);

            if (!utils.getLinkify().test(fullText)) {
              return;
            }

            (utils.getLinkify().match(fullText) || []).forEach(function (match) {
              // not right after a backtick
              if (fullText[match.index - 1] === "`") {
                return;
              }

              tr.addMark(
                startPos + match.index,
                startPos + match.index + match.raw.length,
                state.schema.marks.link.create({ href: match.url, markup: "linkify" })
              );
            });
          });
        });

        return tr;
      },
    });
  },
};
