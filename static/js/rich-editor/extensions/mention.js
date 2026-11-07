// extensions/mention.js: @name as a mention node, typed or parsed. The
// editor checks every mention with GET /composer/mentions: one naming no
// user or group (nor @here) goes back to text, and the answer's warnings
// go to the composer as composer-messages:create, as Ember's do.
import { isBoundary } from "discourse/rich-editor/plugin-utils";

var VALID_MENTIONS = new Set();
var INVALID_MENTIONS = new Set();

// pretty-text/mentions mentionRegex(true): the reference hands it the
// unicodeEnabled function, which is truthy, so the Unicode form is the one
// in use whatever unicode_usernames says.
var ALNUM = "\\p{Alphabetic}\\p{Mark}\\p{Decimal_Number}";
var MENTION_SOURCE = "@([" + ALNUM + "_][" + ALNUM + "._-]{0,58}[" + ALNUM + "])|@([" + ALNUM + "_])";

function basePath() {
  var meta = document.querySelector('meta[name="discourse-base-uri"]');
  return meta ? meta.content : "";
}

function unique(list) {
  return list.filter(function (item, index) {
    return list.indexOf(item) === index;
  });
}

async function fetchMentions(names, context) {
  // only the mentions not validated yet
  names = unique(names).filter(function (name) {
    var lower = name.toLowerCase();
    return !VALID_MENTIONS.has(lower) && !INVALID_MENTIONS.has(lower);
  });

  if (!names.length) {
    return;
  }

  var query = names.map(function (name) {
    return "names[]=" + encodeURIComponent(name);
  });
  if (context.topicId) {
    query.push("topic_id=" + encodeURIComponent(context.topicId));
  }
  var res = await fetch(basePath() + "/composer/mentions?" + query.join("&"), {
    credentials: "same-origin",
    headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
  });
  if (!res.ok) {
    throw new Error("/composer/mentions " + res.status);
  }
  var response = await res.json();

  names.forEach(function (name) {
    var lowerName = name.toLowerCase();

    if (response.users.indexOf(lowerName) >= 0 || response.groups[lowerName]) {
      VALID_MENTIONS.add(lowerName);
    } else {
      INVALID_MENTIONS.add(lowerName);
    }

    checkMentionWarning(name, response, context);
  });
}

function checkMentionWarning(name, response, context) {
  var i18n = context.i18n;
  var hereCount = parseInt(response && response.here_count, 10) || 0;
  var maxMentions = parseInt(response && response.max_users_notified_per_group_mention, 10);
  var lowerName = name.toLowerCase();
  var group = response.groups[lowerName];

  var reason;
  var body;

  if (hereCount > 0) {
    body = i18n("composer.here_mention", {
      here: context.siteSettings.here_mention,
      count: hereCount,
    });
  } else if (response.users.indexOf(lowerName) >= 0) {
    reason = response.user_reasons && response.user_reasons[lowerName];

    if (reason) {
      body = i18n("composer.cannot_see_mention." + reason, { username: name });
    }
  } else if (group) {
    var userCount = group.user_count || 0;
    var notifiedCount = group.notified_count || 0;
    reason = response.group_reasons && response.group_reasons[lowerName];

    var groupLink = basePath() + "/g/" + name + "/members";

    if (reason) {
      body = i18n("composer.cannot_see_group_mention." + reason, {
        group: name,
        count: notifiedCount,
      });
    } else if (notifiedCount > maxMentions) {
      body = i18n("composer.group_mentioned_limit", {
        group: "@" + name,
        count: maxMentions,
        group_link: groupLink,
      });
    } else if (userCount > 0) {
      var translationKey = userCount >= 5 ? "composer.larger_group_mentioned" : "composer.group_mentioned";

      body = i18n(translationKey, {
        group: "@" + name,
        count: userCount,
        group_link: groupLink,
      });
    }
  }

  if (body && context.appEvents) {
    context.appEvents.trigger("composer-messages:create", {
      extraClass: "custom-body",
      templateName: "education",
      body: body,
    });
  }
}

export default {
  nodeSpec: {
    mention: {
      attrs: { name: {} },
      inline: true,
      group: "inline",
      draggable: true,
      selectable: false,
      parseDOM: [
        {
          priority: 60,
          tag: "a.mention, a.mention-group",
          preserveWhitespace: "full",
          getAttrs: function (dom) {
            var name = dom.getAttribute("data-name");
            if (!name) {
              var clone = dom.cloneNode(true);
              clone.querySelectorAll("img.user-status").forEach(function (el) {
                el.remove();
              });
              name = clone.textContent.trim().slice(1);
            }
            return { name: name };
          },
        },
      ],
      toDOM: function (node) {
        return ["a", { class: "mention", "data-name": node.attrs.name }, "@" + node.attrs.name];
      },
    },
  },

  inputRules: {
    match: new RegExp("(^|\\W)(" + MENTION_SOURCE + ") $", "u"),
    handler: function (state, match, start, end) {
      var $from = state.selection.$from;
      if ($from.nodeBefore && $from.nodeBefore.type === state.schema.nodes.mention) {
        return null;
      }
      var mentionStart = start + match[1].length;
      var name = match[2].slice(1);

      return state.tr.replaceWith(mentionStart, end, [
        state.schema.nodes.mention.create({ name: name }),
        state.schema.text(" "),
      ]);
    },
    options: { undoable: false },
  },

  parse: {
    mention: {
      block: "mention",
      getAttrs: function (token, tokens, i) {
        // the text the mention_open/close pair holds is the name
        return { name: tokens.splice(i + 1, 1)[0].content.slice(1) };
      },
    },
  },

  serializeNode: {
    mention: function (state, node, parent, index) {
      state.flushClose();
      if (!isBoundary(state.out, state.out.length - 1)) {
        state.write(" ");
      }

      state.write("@" + node.attrs.name);

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
    var key = new PluginKey("mention");

    return new Plugin({
      key: key,
      view: function () {
        return {
          update: function (view) {
            this.processMentionNodes(view);
          },
          processMentionNodes: function (view) {
            var self = this;
            var mentionNames = [];
            var mentionNodes = [];
            var context = getContext();
            var hereMention = (context.siteSettings.here_mention || "").toLowerCase();

            if (this._processingMentionNodes) {
              return;
            }

            this._processingMentionNodes = true;

            view.state.doc.descendants(function (node, pos) {
              if (node.type.name !== "mention") {
                return;
              }

              var name = node.attrs.name;
              mentionNames.push(name);
              mentionNodes.push({ name: name, node: node, pos: pos });
            });

            // in reverse, so the positions hold as nodes are replaced
            mentionNodes.sort(function (a, b) {
              return b.pos - a.pos;
            });

            var invalidateMentions = async function () {
              await fetchMentions(mentionNames, context);

              mentionNodes.forEach(function (mentionNode) {
                if (view.isDestroyed) {
                  return;
                }
                var lowerName = mentionNode.name.toLowerCase();
                if (VALID_MENTIONS.has(lowerName) || hereMention === lowerName) {
                  return;
                }

                // an invalid mention goes back to text
                var textNode = view.state.schema.text("@" + mentionNode.name);
                view.dispatch(
                  view.state.tr
                    .replaceWith(mentionNode.pos, mentionNode.pos + mentionNode.node.nodeSize, textNode)
                    .setMeta("addToHistory", false)
                );
              });
            };

            invalidateMentions().then(
              function () {
                self._processingMentionNodes = false;
              },
              function (e) {
                self._processingMentionNodes = false;
                throw e;
              }
            );
          },
        };
      },
    });
  },
};
