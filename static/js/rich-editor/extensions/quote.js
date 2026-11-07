// extensions/quote.js: [quote="name, post:1, topic:2"] as a quote node
// (aside.quote around its blockquote), drawn by a node view with the
// title (the quoted post's avatar where the host can look it up, then the
// name) and selectable by its title.
import { schema } from "prosemirror-markdown";

// lib/avatar-utils translateSize("tiny")
var TINY = 24;

// null values are left out, by the serializer and the node view alike
function quoteDataAttrs(node) {
  var attrs = node.attrs;

  return {
    "data-username": attrs.username,
    "data-post": attrs.postNumber,
    "data-topic": attrs.topicId,
    "data-full": attrs.full === "true" ? "true" : "false",
    "data-display-name": attrs.displayName,
  };
}

function basePath() {
  var meta = document.querySelector('meta[name="discourse-base-uri"]');
  return meta ? meta.content : "";
}

// avatarUrl(template, "tiny"): getRawAvatarSize's allowed size for the
// device's pixel ratio, then getURLWithCDN.
function avatarUrl(template, size, siteSettings) {
  if (!template) {
    return "";
  }
  var pixelRatio = window.devicePixelRatio || 1;
  var raw = size * (pixelRatio > 1.1 && pixelRatio < 2.1 ? 2 : pixelRatio >= 2.1 ? 3 : 1);
  var allowed = String(siteSettings.avatar_sizes || "")
    .split("|")
    .map(function (s) {
      return parseInt(s, 10);
    })
    .filter(function (n) {
      return !isNaN(n);
    })
    .sort(function (a, b) {
      return a - b;
    });
  var rawSize = allowed.find(function (s) {
    return s >= raw;
  });
  if (rawSize === undefined) {
    rawSize = allowed.length ? allowed[allowed.length - 1] : raw;
  }
  var path = template.replace(/\{size\}/g, rawSize);
  return path.startsWith("/") && !path.startsWith("//") ? basePath() + path : path;
}

function avatarElement(avatarTemplate, siteSettings) {
  var src = avatarUrl(avatarTemplate, TINY, siteSettings);

  if (!src) {
    return null;
  }

  var img = document.createElement("img");
  img.className = "avatar";
  img.alt = "";
  img.src = src;
  img.width = TINY;
  img.height = TINY;

  return img;
}

function createQuoteNodeView(params) {
  var getContext = params.getContext;

  // A class, which extractNodeViews (editor.js) builds with `new`.
  return class QuoteNodeView {
    constructor(node) {
      this.node = node;
      this.title = null;

      this.dom = document.createElement("aside");
      // set once: the editor manages classes on it too
      this.dom.className = "quote";

      this.contentDOM = document.createElement("blockquote");
      this.dom.append(this.contentDOM);

      this.render(node);
    }

    update(node) {
      // also called for edits inside; the header follows the markup alone
      if (!this.node.sameMarkup(node)) {
        this.render(node);
      }

      this.node = node;

      return true;
    }

    selectNode() {
      this.dom.classList.add("ProseMirror-selectednode");
    }

    deselectNode() {
      this.dom.classList.remove("ProseMirror-selectednode");
    }

    render(node) {
      var dom = this.dom;
      var attrs = quoteDataAttrs(node);
      Object.keys(attrs).forEach(function (name) {
        var value = attrs[name];
        if (value === null || value === undefined) {
          dom.removeAttribute(name);
        } else {
          dom.setAttribute(name, value);
        }
      });

      if (this.title) {
        this.title.remove();
      }
      this.title = this.buildTitle(node);

      if (this.title) {
        dom.insertBefore(this.title, this.contentDOM);
      }
    }

    buildTitle(node) {
      var attrs = node.attrs;

      if (!attrs.username) {
        return null;
      }

      var title = document.createElement("div");
      title.className = "title";
      title.contentEditable = false;

      var context = getContext();
      var options = context.markdownOptions || {};
      var avatarTemplate =
        attrs.postNumber &&
        attrs.topicId &&
        options.lookupAvatarTemplateByPostNumber &&
        options.lookupAvatarTemplateByPostNumber(parseInt(attrs.postNumber, 10), parseInt(attrs.topicId, 10));

      var img = avatarTemplate && avatarElement(avatarTemplate, context.siteSettings || {});

      if (img) {
        title.append(img);
      }

      title.append((attrs.displayName || attrs.username) + ":");

      return title;
    }
  };
}

export default {
  nodeViews: { quote: createQuoteNodeView },

  nodeSpec: {
    blockquote: Object.assign({ createGapCursor: true }, schema.nodes.blockquote.spec),
    quote: {
      content: "block+",
      group: "block",
      createGapCursor: true,
      defining: true,
      attrs: {
        username: { default: null },
        displayName: { default: null },
        postNumber: { default: null },
        topicId: { default: null },
        full: { default: null },
      },
      parseDOM: [
        {
          tag: "aside.quote",
          contentElement: "blockquote",
          getAttrs: function (dom) {
            return {
              username: dom.getAttribute("data-username"),
              displayName: dom.getAttribute("data-display-name"),
              postNumber: dom.getAttribute("data-post"),
              topicId: dom.getAttribute("data-topic"),
              full: dom.getAttribute("data-full"),
            };
          },
        },
      ],
      toDOM: function (node) {
        var domSpec = ["aside", Object.assign({ class: "quote" }, quoteDataAttrs(node))];

        if (node.attrs.username) {
          domSpec.push(["div", { class: "title" }, (node.attrs.displayName || node.attrs.username) + ":"]);
        }

        domSpec.push(["blockquote", 0]);

        return domSpec;
      },
    },
  },

  parse: {
    quote_header_open: function (state, token, tokens, i) {
      // The header's text goes: the token order is quote_header_open
      // quote_controls_open quote_controls_close text quote_header_close,
      // and a contentEditable=false title node would break keyboard moves.
      tokens[i + 3].content = "";
    },
    quote_header_close: function () {},
    quote_controls: { ignore: true },
    bbcode_open: function (state, token) {
      if (token.tag === "aside") {
        state.openNode(state.schema.nodes.quote, {
          username: token.attrGet("data-username"),
          displayName: token.attrGet("data-display-name"),
          postNumber: token.attrGet("data-post"),
          topicId: token.attrGet("data-topic"),
          full: token.attrGet("data-full"),
        });
        return true;
      }

      // handled, as nothing
      if (token.tag === "blockquote") {
        return true;
      }
    },
    bbcode_close: function (state, token) {
      if (token.tag === "aside") {
        state.closeNode();
        return true;
      }

      if (token.tag === "blockquote") {
        return true;
      }
    },
  },

  serializeNode: {
    quote: function (state, node) {
      var attrs = node.attrs;
      var postNumberParam = attrs.postNumber ? ", post:" + attrs.postNumber : "";
      var topicIdParam = attrs.topicId ? ", topic:" + attrs.topicId : "";
      var fullParam = attrs.full === "true" ? ", full:true" : "";
      var usernameParam = attrs.displayName && attrs.username ? ", username:" + attrs.username : "";
      var name = attrs.displayName || attrs.username;
      var quoteValue = name ? '="' + name + postNumberParam + topicIdParam + fullParam + usernameParam + '"' : "";

      state.write("[quote" + quoteValue + "]\n");
      state.renderContent(node);
      state.write("[/quote]\n\n");
    },
  },

  inputRules: function (params) {
    var convertFromMarkdown = params.utils.convertFromMarkdown;
    return {
      match: /^\[quote([^\]]*)\]$/,
      handler: function (state, match, start, end) {
        var markdown = match[0] + "\n[/quote]";

        return state.tr.replaceWith(start - 1, end, convertFromMarkdown(markdown)).scrollIntoView();
      },
    };
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var NodeSelection = params.pmState.NodeSelection;
    var Slice = params.pmModel.Slice;
    var Fragment = params.pmModel.Fragment;

    return new Plugin({
      props: {
        transformPasted: function (slice, view) {
          // A quote whose first child is a list: Slice.maxOpen opens it too
          // deep (openStart 4) and the paste would unwrap the quote, so the
          // opening is limited.
          var wasFixed = false;
          var firstChild = slice.content.firstChild;
          if (
            firstChild &&
            firstChild.type.name === "quote" &&
            slice.openStart > 2 &&
            firstChild.firstChild &&
            (firstChild.firstChild.type.name === "bullet_list" || firstChild.firstChild.type.name === "ordered_list")
          ) {
            slice = new Slice(slice.content, 1, 1);
            wasFixed = true;
          }

          if (
            view.endOfTextblock("forward") &&
            slice.content.childCount === 1 &&
            slice.content.firstChild.type.name === "quote"
          ) {
            var quote = slice.content.firstChild;
            var paragraph = view.state.schema.nodes.paragraph.create();

            return wasFixed
              ? new Slice(Fragment.from([quote, paragraph]), slice.openStart, slice.openEnd)
              : Slice.maxOpen(Fragment.from([quote, paragraph]), false);
          }

          return slice;
        },

        handleClickOn: function (view, pos, node, nodePos, event) {
          if (node.type.name === "quote" && event.target.closest("aside.quote > .title")) {
            view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, nodePos)));

            return true;
          }
        },
      },
    });
  },
};
