// extensions/hashtag.js: #slug as a hashtag node, typed or parsed. The
// editor looks each one up with GET /hashtags in the composer's order: a
// valid one is drawn with its type's icon (static/js/hashtags.js
// generateIconHtml) and its text, an invalid one goes back to text.
import { emojiUnescape } from "discourse/rich-editor/engine";
import { isBoundary } from "discourse/rich-editor/plugin-utils";

var VALID_HASHTAGS = new Map();
var INVALID_HASHTAGS = new Set();

function basePath() {
  var meta = document.querySelector('meta[name="discourse-base-uri"]');
  return meta ? meta.content : "";
}

async function fetchHashtags(hashtags, context) {
  var slugs = hashtags.filter(function (tag) {
    return !VALID_HASHTAGS.has(tag) && !INVALID_HASHTAGS.has(tag);
  });

  if (!slugs.length) {
    return;
  }

  var site = context.site || {};
  var order = (site.hashtag_configurations || {})["topic-composer"] || [];
  var query = slugs
    .map(function (slug) {
      return "slugs[]=" + encodeURIComponent(slug);
    })
    .concat(
      order.map(function (type) {
        return "order[]=" + encodeURIComponent(type);
      })
    );
  var res = await fetch(basePath() + "/hashtags?" + query.join("&"), {
    credentials: "same-origin",
    headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
  });
  if (!res.ok) {
    throw new Error("/hashtags " + res.status);
  }
  var response = await res.json();

  var validTags = [];
  Object.keys(response || {}).forEach(function (type) {
    (response[type] || []).forEach(function (tag) {
      if (tag) {
        validTags.push(tag);
      }
    });
  });

  validTags.forEach(function (tag) {
    VALID_HASHTAGS.set(tag.ref, tag);
    hashtags.splice(hashtags.indexOf(tag.ref), 1);
  });

  // the rest are invalid, so they are not asked for again
  hashtags.forEach(function (tag) {
    INVALID_HASHTAGS.add(tag);
  });
}

// getHashtagTypeClasses()[type].generateIconHTML
function iconHtml(hashtag) {
  var hashtags = window.Discourse && window.Discourse.hashtags;
  var html = hashtags && hashtags.generateIconHtml ? hashtags.generateIconHtml(hashtag.type, hashtag) : null;
  return html ? html.trim() : "";
}

export default {
  nodeSpec: {
    hashtag: {
      attrs: {
        name: {},
        processed: { default: false },
      },
      inline: true,
      group: "inline",
      draggable: true,
      selectable: false,
      parseDOM: [
        {
          tag: "a.hashtag-cooked, a.hashtag",
          priority: 60,
          preserveWhitespace: "full",
          getAttrs: function (dom) {
            var name = dom.dataset.name !== undefined ? dom.dataset.name : dom.dataset.ref;

            if (!name && dom.dataset.slug) {
              name = dom.dataset.slug + (dom.dataset.type ? "::" + dom.dataset.type : "");
            }

            if (name === undefined || name === null) {
              name = dom.textContent.slice(1);
            }

            return { name: name, processed: dom.dataset.processed };
          },
        },
      ],
      toDOM: function (node) {
        return [
          "a",
          {
            class: "hashtag-cooked",
            "data-name": node.attrs.name,
            "data-processed": node.attrs.processed,
          },
          "#" + node.attrs.name,
        ];
      },
    },
  },

  inputRules: [
    {
      match:
        /(^|\W)(#[À-῿Ⰰ-퟿\w:-](?:[À-῿Ⰰ-퟿\w:.-]{0,99}[À-῿Ⰰ-퟿\w:-])?)\s$/,
      handler: function (state, match, start, end) {
        var hashtagStart = start + match[1].length;
        var name = match[2].slice(1);

        // Not an anchor in a url
        var $pos = state.doc.resolve(hashtagStart);
        if (
          $pos.marks().some(function (mark) {
            return mark.type === state.schema.marks.link;
          })
        ) {
          return false;
        }

        var nodeBefore = state.selection.$from.nodeBefore;
        return (
          (!nodeBefore || nodeBefore.type !== state.schema.nodes.hashtag) &&
          state.tr.replaceWith(hashtagStart, end, [
            state.schema.nodes.hashtag.create({ name: name }),
            state.schema.text(" "),
          ])
        );
      },
      options: { undoable: false },
    },
  ],

  parse: {
    span_open: function (state, token, tokens, i) {
      if (token.attrGet("class") === "hashtag-raw") {
        state.openNode(state.schema.nodes.hashtag, {
          // the text the span_open/close pair holds is the hashtag
          name: tokens.splice(i + 1, 1)[0].content.slice(1),
        });
        return true;
      }
    },
    span_close: function (state) {
      if (state.top().type.name === "hashtag") {
        state.closeNode();
        return true;
      }
    },
  },

  serializeNode: {
    hashtag: function (state, node, parent, index) {
      state.flushClose();
      if (!isBoundary(state.out, state.out.length - 1)) {
        state.write(" ");
      }

      state.write("#" + node.attrs.name);

      var nextSibling = parent.childCount > index + 1 ? parent.child(index + 1) : null;
      if (nextSibling && nextSibling.isText && !isBoundary(nextSibling.text, 0)) {
        state.write(" ");
      }
    },
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var PluginKey = params.pmState.PluginKey;
    var getContext = params.getContext;
    var key = new PluginKey("hashtag");

    return new Plugin({
      key: key,
      view: function () {
        return {
          update: function (view) {
            this.processHashtags(view);
          },
          processHashtags: function (view) {
            var hashtagNames = [];
            var hashtagNodes = [];

            view.state.doc.descendants(function (node, pos) {
              if (node.type.name !== "hashtag" || node.attrs.processed) {
                return;
              }

              hashtagNodes.push({ name: node.attrs.name, node: node, pos: pos });
              hashtagNames.push(node.attrs.name);
            });

            if (!hashtagNodes.length) {
              return;
            }

            // in reverse, so the positions hold as nodes are replaced
            hashtagNodes.sort(function (a, b) {
              return b.pos - a.pos;
            });

            var updateHashtags = async function () {
              await fetchHashtags(hashtagNames, getContext());

              hashtagNodes.forEach(function (hashtagNode) {
                if (view.isDestroyed) {
                  return;
                }
                var name = hashtagNode.name;
                var node = hashtagNode.node;
                var pos = hashtagNode.pos;
                var validHashtag = VALID_HASHTAGS.get(name.toLowerCase());

                // the node may be gone by now
                var current = view.state.doc.nodeAt(pos);
                if (!current || current.type !== node.type) {
                  return;
                }

                var change;
                if (validHashtag) {
                  change = view.state.tr.setNodeMarkup(pos, null, Object.assign({}, node.attrs, { processed: true }));
                } else {
                  change = view.state.tr.replaceWith(pos, pos + node.nodeSize, view.state.schema.text("#" + name));
                }

                view.dispatch(change.setMeta("addToHistory", false));

                var domNode = view.nodeDOM(pos);
                if (!validHashtag || !domNode) {
                  return;
                }

                // drawn by its type
                var displayText = validHashtag.text || name;
                var tagText = typeof displayText === "string" ? emojiUnescape(displayText) : displayText;
                var hashtagIconHTML = validHashtag.iconHtml || iconHtml(validHashtag);

                domNode.innerHTML = hashtagIconHTML + tagText;
              });
            };

            updateHashtags();
          },
        };
      },
    });
  },
};
