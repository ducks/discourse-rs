// chat's lib/rich-editor-extension.js: a [chat] transcript as a chat node
// over chat-transcript.js' tokens (the markdown crate's, written one by one
// in the node tree: docs/rich-editor.md), drawn as the rule draws it and
// headed by its channel or source as group-chat-transcripts.js'
// buildTranscriptHeader heads one, and written back from its attributes
// and the markdown between its tags.
import { emojiUnescape } from "discourse/rich-editor/engine";
import { iconHtml } from "discourse/rich-editor/toolbar";

// toDOM has no params: the context of the editor last created, as the
// reference reads its i18n and owner globals.
var lastContext = null;

function i18n(context, key) {
  return context && context.i18n ? context.i18n(key) : key;
}

function basePath() {
  var meta = document.querySelector('meta[name="discourse-base-uri"]');
  return meta ? meta.content : "";
}

// discourse/lib/utilities escapeExpression
function escapeExpression(string) {
  return String(string).replace(/[&<>"'`=]/g, function (c) {
    return {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#x27;",
      "`": "&#x60;",
      "=": "&#x3D;",
    }[c];
  });
}

// transcript-source-types.js: how a source type is presented; unknown
// types (sourceType comes from post markup) take the defaults.
var sourceTypes = new Map();

export function registerTranscriptSourceType(type, presentation) {
  sourceTypes.set(type, { icon: presentation.icon, label: presentation.label });
}

function transcriptSourcePresentation(type, context) {
  var registered = sourceTypes.get(type);
  return {
    icon: (registered && registered.icon) || "comment",
    label: i18n(context, (registered && registered.label) || "chat.quote.transcript_label"),
  };
}

// ids come from cooked attributes a post author can write, so they're
// encoded to keep each one a single path segment
function channelPath(channelId) {
  return basePath() + "/chat/c/-/" + encodeURIComponent(channelId);
}

// getHashtagTypeClasses().channel: chat's hashtag type, static/js/hashtags.js
function channelType() {
  var hashtags = window.Discourse && window.Discourse.hashtags;
  return hashtags && hashtags.generateIconHtml ? hashtags : null;
}

function channelIconHtml(channelId, context, emoji) {
  var site = (context && context.site) || {};
  var icon = (site.hashtag_icons && site.hashtag_icons.channel) || "comment";
  var type = channelType();
  if (type && channelId) {
    var html = type.generateIconHtml("channel", { id: parseInt(channelId, 10), icon: icon, emoji: emoji });
    if (html) {
      return html.trim();
    }
  }
  return iconHtml(icon);
}

var channelEmojiRequests = new Map();

function fetchChannelEmoji(channelId) {
  if (!channelEmojiRequests.has(channelId)) {
    channelEmojiRequests.set(
      channelId,
      fetch(basePath() + "/hashtags/by-ids?channel%5B%5D=" + encodeURIComponent(channelId), {
        credentials: "same-origin",
        headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
      })
        .then(function (r) {
          return r.ok ? r.json() : {};
        })
        .then(function (result) {
          return result.channel && result.channel[0] ? result.channel[0].emoji : null;
        })
        .catch(function () {
          return null;
        })
    );
  }
  return channelEmojiRequests.get(channelId);
}

function fromHtml(html) {
  var template = document.createElement("template");
  template.innerHTML = html;
  return template.content.firstElementChild;
}

// group-chat-transcripts.js buildTranscriptHeader: the chat channel a
// transcript was quoted from, or its unlinked source presented by its
// registered type. The editor knows no joined channels, so a linked
// channel's emoji is looked up.
export function buildTranscriptHeader(attrs, context) {
  var header = document.createElement("div");
  header.classList.add("chat-transcript-group__header");

  var fromChannel = !!attrs.channelName;
  var linked = fromChannel && attrs.channelId;

  var channel = document.createElement(linked ? "a" : "span");
  channel.classList.add("hashtag-cooked", "chat-transcript-group__channel");
  if (linked) {
    channel.href = channelPath(attrs.channelId);
  }

  var iconMarkup, labelText;
  if (fromChannel) {
    iconMarkup = channelIconHtml(attrs.channelId, context);
    labelText = i18n(context, "chat.quote.transcript_label");
  } else {
    var presentation = transcriptSourcePresentation(attrs.sourceType, context);
    iconMarkup = iconHtml(presentation.icon);
    labelText = presentation.label;
  }

  channel.innerHTML =
    iconMarkup + "<span>" + emojiUnescape(escapeExpression(fromChannel ? attrs.channelName : attrs.source)) + "</span>";

  if (linked && channelType()) {
    fetchChannelEmoji(attrs.channelId).then(function (emoji) {
      if (emoji && channel.firstElementChild) {
        channel.firstElementChild.replaceWith(fromHtml(channelIconHtml(attrs.channelId, context, emoji)));
      }
    });
  }

  var label = document.createElement("span");
  label.classList.add("chat-transcript-group__label");
  label.textContent = labelText;

  header.append(channel, label);
  return header;
}

// moment(datetime).format(i18n("dates.long_no_year")), in the browser's
// zone: moment where the page has it, else dates.js' English format.
function formatDateTime(datetime, context) {
  var format = i18n(context, "dates.long_no_year");
  if (window.moment) {
    return window.moment(datetime).format(format);
  }
  var dates = window.Discourse && window.Discourse.dates;
  var ms = Date.parse(datetime);
  return dates && !isNaN(ms) ? dates.format(ms, format) : datetime;
}

function transcriptElement(node, context) {
  // NOTE: This HTML representation of the node in a lot of ways is duplicated from
  // the chat transcript markdown-it rule, this is unavoidable unless we completely
  // decouple the token generation from the HTML generation, which is a lot of work,
  // so this is acceptable for now.

  var wrapperElement = document.createElement("div");
  wrapperElement.classList.add("chat-transcript");

  if (node.attrs.chained) {
    wrapperElement.classList.add("chat-transcript-chained");
  }

  // only a transcript's first block names the channel or source; later
  // blocks join its box, which the stylesheet finds through these attributes
  if (node.attrs.channel) {
    wrapperElement.dataset.channelName = node.attrs.channel;
  } else if (node.attrs.source) {
    wrapperElement.dataset.source = node.attrs.source;
  }

  var headerElement;
  if (node.attrs.channel || node.attrs.source) {
    headerElement = buildTranscriptHeader(
      {
        channelName: node.attrs.channel,
        channelId: node.attrs.channelId,
        source: node.attrs.source,
        sourceType: node.attrs.sourceType,
      },
      context
    );
  }

  var userElement = document.createElement("div");
  userElement.classList.add("chat-transcript-user");

  // avatars can't be looked up here, so this stays an empty placeholder
  var avatarElement = document.createElement("div");
  avatarElement.classList.add("chat-transcript-user-avatar");
  userElement.appendChild(avatarElement);

  var usernameElement = document.createElement("span");
  usernameElement.classList.add("chat-transcript-username");
  usernameElement.textContent = node.attrs.username;
  userElement.appendChild(usernameElement);

  var datetimeElement = document.createElement("span");
  datetimeElement.classList.add("chat-transcript-datetime");
  datetimeElement.textContent = formatDateTime(node.attrs.datetime, context);
  userElement.appendChild(datetimeElement);

  var messagesElement = document.createElement("div");
  messagesElement.classList.add("chat-transcript-messages");
  messagesElement.innerHTML = node.attrs.html;

  if (headerElement) {
    wrapperElement.appendChild(headerElement);
  }

  if (node.attrs.threadId) {
    var threadElement = document.createElement("div");
    threadElement.classList.add("chat-transcript-thread");

    var threadHeaderElement = document.createElement("div");
    threadHeaderElement.classList.add("chat-transcript-thread-header");
    threadHeaderElement.innerHTML =
      '<svg class="fa d-icon d-icon-discourse-threads svg-icon svg-node"><use href="#discourse-threads"></use></svg>';

    var threadTitleElement = document.createElement("span");
    threadTitleElement.classList.add("chat-transcript-thread-header__title");
    threadTitleElement.innerHTML = node.attrs.threadTitle
      ? emojiUnescape(escapeExpression(node.attrs.threadTitle))
      : i18n(context, "chat.quote.default_thread_title");

    threadHeaderElement.appendChild(threadTitleElement);
    threadElement.appendChild(threadHeaderElement);
    threadElement.appendChild(userElement);
    threadElement.appendChild(messagesElement);
    wrapperElement.appendChild(threadElement);

    if (node.attrs.threadHtml) {
      wrapperElement.insertAdjacentHTML("beforeend", node.attrs.threadHtml);
    }
  } else {
    wrapperElement.appendChild(userElement);
    wrapperElement.appendChild(messagesElement);
  }

  return wrapperElement;
}

export default {
  nodeSpec: {
    chat: {
      attrs: {
        messageId: {},
        username: {},
        datetime: {},
        channel: {},
        channelId: {},
        source: { default: null },
        sourceType: { default: null },
        html: {},
        rawContent: {},
        multiQuote: {},
        chained: {},
        threadId: { default: null },
        threadTitle: { default: null },
        threadHtml: { default: null },
      },
      group: "block",
      isolating: true,
      selectable: true,
      parseDOM: [{ tag: "div.chat-transcript" }],
      toDOM: function (node) {
        return transcriptElement(node, lastContext);
      },
    },
  },
  nodeViews: {
    chat: function (params) {
      lastContext = params.getContext();
      return function (node) {
        return { dom: transcriptElement(node, params.getContext()) };
      };
    },
  },
  serializeNode: function (params) {
    var buildBBCodeAttrs = params.utils.buildBBCodeAttrs;
    return {
      chat: function (state, node) {
        var bbCodeAttrs = 'quote="' + node.attrs.username + ";" + node.attrs.messageId + ";" + node.attrs.datetime + '"';
        bbCodeAttrs +=
          " " +
          buildBBCodeAttrs(node.attrs, {
            skipAttrs: ["username", "messageId", "datetime", "rawContent", "html", "threadHtml"],
          });

        state.write("[chat " + bbCodeAttrs + "]\n");
        state.write(node.attrs.rawContent);
        state.write("\n[/chat]\n");
      },
    };
  },
  parse: {
    div_chat_transcript_wrap_open: function (state, token, tokens, i) {
      // The slice here makes sure we get the html_raw content
      // only for the current [chat] bbcode block based on the
      // token index.
      var nextHtmlRaw = tokens.slice(i).find(function (t) {
        return t.type === "html_raw" && t.nesting === 1;
      });
      var messagesHtml = nextHtmlRaw ? nextHtmlRaw.content : undefined;

      var threadHtmlRaw;
      if (token.attrGet("data-thread-id")) {
        threadHtmlRaw = tokens.slice(tokens.indexOf(nextHtmlRaw) + 1).find(function (t) {
          return t.type === "html_raw" && t.nesting === 1;
        });
      }

      // So this content and the whole wrap_open happens for every single instance of
      // the `[chat]` bbcode, including nested which is the case for threads.
      //
      // Only the first one will have multiQuote and chained
      //
      // Multiquote is > 1 message
      // Chained is > 1 message by different users
      state.openNode(state.schema.nodes.chat, {
        messageId: token.attrGet("data-message-id"),
        username: token.attrGet("data-username"),
        datetime: token.attrGet("data-datetime"),
        channel: token.attrGet("data-channel-name"),
        channelId: token.attrGet("data-channel-id"),
        source: token.attrGet("data-source"),
        sourceType: token.attrGet("data-source-type"),
        chained: token.attrGet("data-chained"),
        multiQuote: token.attrGet("data-multiquote"),
        threadId: token.attrGet("data-thread-id"),
        threadTitle: token.attrGet("data-thread-title"),
        threadHtml: threadHtmlRaw ? threadHtmlRaw.content : null,
        html: messagesHtml,
        rawContent: token.content,
      });
      return true;
    },
    div_chat_transcript_wrap_close: function (state) {
      state.closeNode();
      return true;
    },
    div_chat_transcript_user: { ignore: true },
    div_chat_transcript_user_avatar: { ignore: true },
    div_chat_transcript_username: { ignore: true },
    div_chat_transcript_datetime: { ignore: true },
    div_chat_transcript_messages: { ignore: true },
    div_chat_transcript_reaction: { ignore: true },

    // Reaction-related tokens are not used in the live preview,
    // they are only used when archiving a channel, not needed here.
    div_chat_transcript_reactions: { ignore: true },
    div_chat_transcript_meta: { ignore: true },

    // Thread-related tokens
    details_chat_transcript_wrap: { ignore: true },
    summary_chat_transcript: { ignore: true },
    div_thread: { ignore: true },
    div_thread_header: { ignore: true },
    svg_thread_header: { ignore: true },
    use_svg_thread: { ignore: true },
    span_thread_title: { ignore: true },

    html_raw: { ignore: true, noCloseToken: true },

    span_open: function (state) {
      if (state.top().type.name === "chat") {
        return true;
      }
    },
    span_close: function (state) {
      if (state.top().type.name === "chat") {
        return true;
      }
    },
  },
};
