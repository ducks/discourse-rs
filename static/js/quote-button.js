// Quoting selected text in a topic, as Ember's PostTextSelection
// (components/post-text-selection.gjs) and its toolbar
// (post-text-selection-toolbar.gjs) do it: selecting text in a post shows
// the toolbar by the selection (div.quote-button in a DMenu): Quote,
// Edit (the fast edit of a post the viewer can edit), Copy Quote and the
// share_quote_buttons sources. Quote hands the quote to the composer
// (TopicController#selectText, through the composer:quote event
// composer.js handles); Edit opens the fast edit modal when the selection
// is simple enough (computeSupportsFastEdit), else the composer on the
// post.
//
//   Discourse.buildQuote(post, contents, opts)
//       lib/quote.js's buildQuote: the [quote] BBCode around the contents,
//       for a post's {name, username, post_number, topic_id}; the
//       composer's "Quote whole post" uses it too.
//
// The selection's markdown is QuoteState#markdown's: the selected HTML
// (selectedHTML: a code block's selection as code, a onebox's as its
// source) turned into markdown as the rich editor's serializer writes the
// cooked elements a selection holds (paragraphs, emphasis, code, links,
// mentions, hashtags, emoji, images, lists, headings, block quotes and
// quotes, rules); the text of anything else. `full` when it is the whole
// post. The config (topic, viewer, settings and strings) is the page's
// #quote-button-config.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (!Discourse.once("quote-button")) {
    return;
  }

  // lib/environment.js
  var INPUT_DELAY = 250;

  var config = null;
  function readConfig() {
    var el = document.getElementById("quote-button-config");
    config = null;
    if (el) {
      try {
        config = JSON.parse(el.textContent);
      } catch (e) {
        window.console && console.error("quote-button-config", e);
      }
    }
  }

  function setting(name) {
    return config && config.settings ? config.settings[name] : undefined;
  }

  function t(key) {
    var strings = (config && config.strings) || {};
    return strings[key] === undefined ? key : strings[key];
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' + escapeHtml(name) +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      escapeHtml(name) + '"></use></svg>'
    );
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    return headers;
  }

  // discourse-markdown-it's stripQuotationMarks
  var QUOTATION_MARKS = /["'«»“”‘’„‚‹›]/g;

  // prioritizeNameFallback
  function displayName(name, username) {
    if (setting("display_name_on_posts") && !setting("prioritize_username_in_ux")) {
      return name || username;
    }
    return username;
  }

  // lib/quote.js buildQuote
  function buildQuote(post, contents, opts) {
    opts = opts || {};
    if (!post || !contents) {
      return "";
    }
    var fullName = post.name;
    if (opts.username && opts.username !== post.username) {
      fullName = null;
    }
    var name = displayName(fullName, opts.username || post.username);
    var params = [
      (name || "").replace(QUOTATION_MARKS, ""),
      "post:" + (opts.post || post.post_number),
      "topic:" + (opts.topic || post.topic_id),
    ];
    if (opts.full) {
      params.push("full:true");
    }
    if (setting("display_name_on_posts") && !setting("prioritize_username_in_ux") && fullName) {
      params.push("username:" + (opts.username || post.username));
    }
    return '[quote="' + params.join(", ") + '"]\n' + contents.trim() + "\n[/quote]\n\n";
  }
  Discourse.buildQuote = buildQuote;

  // The post (by its id on the page) as /posts/by_number answers it, kept
  // for the page.
  var posts = {};
  function postNumberOf(id) {
    var article = document.querySelector('#posts article[data-post-id="' + id + '"]');
    var wrapper = article && article.closest("[data-post-number]");
    return wrapper ? parseInt(wrapper.dataset.postNumber, 10) : null;
  }

  function fetchPost(id) {
    if (!posts[id]) {
      var number = postNumberOf(id);
      if (!number) {
        return Promise.reject(new Error("post " + id + " is not on the page"));
      }
      posts[id] = fetch(basePath() + "/posts/by_number/" + config.topic.id + "/" + number, {
        headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
        credentials: "same-origin",
      }).then(function (r) {
        if (!r.ok) {
          delete posts[id];
          throw new Error(r.status + " " + r.statusText);
        }
        return r.json();
      });
    }
    return posts[id];
  }

  // ---- The selection's markdown (QuoteState#markdown, toMarkdown) ----

  // prosemirror-markdown's esc
  function esc(text, startOfLine) {
    text = text.replace(/[`*\\~\[\]_]/g, function (m, i) {
      if (m === "_" && i > 0 && i + 1 < text.length && /\w/.test(text[i - 1]) && /\w/.test(text[i + 1])) {
        return m;
      }
      return "\\" + m;
    });
    if (startOfLine) {
      text = text
        .replace(/^(\+[ ]|[\-*>])/, "\\$&")
        .replace(/^(\s*)(#{1,6})(\s|$)/, "$1\\$2$3")
        .replace(/^(\s*\d+)\.\s/, "$1\\. ");
    }
    return text;
  }

  function hasClass(el, name) {
    return el.classList && el.classList.contains(name);
  }

  // The inline content of an element as markdown.
  function inline(node) {
    var out = "";
    node.childNodes.forEach(function (child) {
      out += inlineNode(child);
    });
    return out;
  }

  // A word boundary before an atom (mention, hashtag, emoji), as their
  // serializers add one.
  function spaced(prefix, text) {
    return (prefix.length && !/[\s(\[{]$/.test(prefix) ? " " : "") + text;
  }

  function inlineNode(node) {
    if (node.nodeType === Node.TEXT_NODE) {
      return esc(node.textContent.replace(/\s+/g, " "), false);
    }
    if (node.nodeType !== Node.ELEMENT_NODE) {
      return "";
    }
    var tag = node.tagName;
    switch (tag) {
      case "BR":
        return "\n";
      case "STRONG":
      case "B":
        return wrap("**", inline(node));
      case "EM":
      case "I":
        return wrap("*", inline(node));
      case "S":
      case "DEL":
      case "STRIKE":
        return wrap("~~", inline(node));
      case "CODE":
        return codeSpan(node.textContent);
      case "IMG":
        if (hasClass(node, "emoji")) {
          var code = (node.getAttribute("title") || node.getAttribute("alt") || "").replace(/^:|:$/g, "");
          return code ? ":" + code + ":" : "";
        }
        return "![" + esc(node.getAttribute("alt") || "", false) + "](" + (node.getAttribute("src") || "") + ")";
      case "A":
        if (hasClass(node, "mention") || hasClass(node, "mention-group")) {
          return node.textContent;
        }
        if (hasClass(node, "hashtag-cooked")) {
          return "#" + (node.dataset.slug || node.textContent.replace(/^#/, ""));
        }
        var href = node.getAttribute("href") || "";
        var text = inline(node);
        if (!href) {
          return text;
        }
        if (text === href || text === esc(href, false)) {
          return href;
        }
        return "[" + text + "](" + href + ")";
      default:
        return inline(node);
    }
  }

  function wrap(mark, text) {
    var lead = text.match(/^\s*/)[0];
    var trail = text.match(/\s*$/)[0];
    var core = text.trim();
    return core ? lead + mark + core + mark + trail : text;
  }

  function codeSpan(text) {
    var ticks = "`";
    while (text.indexOf(ticks) !== -1) {
      ticks += "`";
    }
    var pad = text[0] === "`" || text[text.length - 1] === "`" ? " " : "";
    return ticks + pad + text + pad + ticks;
  }

  function isBlock(node) {
    return (
      node.nodeType === Node.ELEMENT_NODE &&
      /^(P|DIV|UL|OL|LI|BLOCKQUOTE|PRE|H[1-6]|HR|ASIDE|TABLE|DETAILS|SECTION)$/.test(node.tagName)
    );
  }

  // Block content: blocks separated by a blank line.
  function blocks(node) {
    var out = [];
    var run = [];
    function flush() {
      if (run.length) {
        var text = run.map(inlineNode).join("").trim();
        if (text) {
          out.push(lineStart(text));
        }
        run = [];
      }
    }
    node.childNodes.forEach(function (child) {
      if (isBlock(child)) {
        flush();
        var md = block(child);
        if (md) {
          out.push(md);
        }
      } else {
        run.push(child);
      }
    });
    flush();
    return out.join("\n\n");
  }

  // A paragraph's first characters escaped as esc(startOfLine) does.
  function lineStart(text) {
    return text
      .split("\n")
      .map(function (line) {
        return line
          .replace(/^(\+[ ]|[\-*>])/, "\\$&")
          .replace(/^(\s*)(#{1,6})(\s|$)/, "$1\\$2$3")
          .replace(/^(\s*\d+)\.\s/, "$1\\. ");
      })
      .join("\n");
  }

  function prefixLines(prefix, text, first) {
    return text
      .split("\n")
      .map(function (line, i) {
        if (i === 0 && first !== undefined) {
          return first + line;
        }
        return line ? prefix + line : prefix.replace(/\s+$/, "");
      })
      .join("\n");
  }

  function block(node) {
    var tag = node.tagName;
    switch (tag) {
      case "P":
        return lineStart(inline(node).trim());
      case "H1":
      case "H2":
      case "H3":
      case "H4":
      case "H5":
      case "H6":
        return "######".slice(0, parseInt(tag[1], 10)) + " " + inline(node).trim();
      case "HR":
        return "---";
      case "PRE":
        var code = node.querySelector("code");
        var lang = code && (code.className.match(/lang-([\w-]+)/) || [])[1];
        var body = (code || node).textContent.replace(/\n$/, "");
        var fence = "```";
        while (body.indexOf(fence) !== -1) {
          fence += "`";
        }
        return fence + (lang && lang !== "auto" ? lang : "") + "\n" + body + "\n" + fence;
      case "BLOCKQUOTE":
        return prefixLines("> ", blocks(node));
      case "UL":
      case "OL":
        var start = parseInt(node.getAttribute("start") || "1", 10);
        var items = [];
        Array.prototype.forEach.call(node.children, function (li, i) {
          if (li.tagName !== "LI") {
            return;
          }
          var marker = tag === "OL" ? start + i + ". " : "* ";
          var pad = new Array(marker.length + 1).join(" ");
          items.push(prefixLines(pad, blocks(li), marker));
        });
        return items.join("\n");
      case "LI":
        return prefixLines("  ", blocks(node), "* ");
      case "ASIDE":
        if (hasClass(node, "quote")) {
          return quoteBlock(node);
        }
        if (hasClass(node, "onebox") && node.dataset.oneboxSrc) {
          return node.dataset.oneboxSrc;
        }
        return blocks(node);
      default:
        return blocks(node);
    }
  }

  // A quote as its [quote] BBCode, its title left out.
  function quoteBlock(aside) {
    var params = [];
    var username = aside.dataset.username;
    if (username) {
      params.push(username);
    }
    if (aside.dataset.post) {
      params.push("post:" + aside.dataset.post);
    }
    if (aside.dataset.topic) {
      params.push("topic:" + aside.dataset.topic);
    }
    if (aside.dataset.full === "true") {
      params.push("full:true");
    }
    var quote = aside.querySelector("blockquote");
    var inner = quote ? blocks(quote) : "";
    var open = params.length ? '[quote="' + params.join(", ") + '"]' : "[quote]";
    return open + "\n" + inner + "\n[/quote]";
  }

  function toMarkdown(html) {
    var doc = new DOMParser().parseFromString(html, "text/html");
    // The decorations of the quotes and code blocks drawn in the post.
    doc.body
      .querySelectorAll(".quote-controls, .codeblock-buttons, .cooked-selection-barrier")
      .forEach(function (el) {
        el.remove();
      });
    return blocks(doc.body).trim();
  }

  // ---- The selection (lib/utilities.js) ----

  function selectedRange() {
    var selection = window.getSelection();
    return selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
  }

  function elementOf(node) {
    return node && node.nodeType === Node.ELEMENT_NODE ? node : node && node.parentElement;
  }

  function isFirstMeaningfulChild(node) {
    for (var sibling = node.previousSibling; sibling; sibling = sibling.previousSibling) {
      if (sibling.nodeType !== Node.TEXT_NODE || sibling.textContent.trim()) {
        return false;
      }
    }
    return true;
  }

  // rangeWithoutTrailingHusk
  function rangeWithoutTrailingHusk(range) {
    if (range.endOffset !== 0) {
      return range;
    }
    var common = range.commonAncestorContainer;
    var boundary = range.endContainer;
    while (boundary !== common && boundary.parentNode !== common && isFirstMeaningfulChild(boundary)) {
      boundary = boundary.parentNode;
    }
    if (boundary === common || !boundary.parentNode) {
      return range;
    }
    var trimmed = range.cloneRange();
    trimmed.setEndBefore(boundary);
    return trimmed;
  }

  // selectedHTML
  function selectedHTML() {
    var selection = window.getSelection();
    if (selection.isCollapsed) {
      return "";
    }
    var div = document.createElement("div");
    for (var r = 0; r < selection.rangeCount; r++) {
      var range = selection.getRangeAt(r);
      var ancestor = elementOf(range.commonAncestorContainer);
      var onebox = ancestor.closest("aside.onebox[data-onebox-src]");
      var pre = ancestor.closest("pre");
      if (pre) {
        var code = document.createElement("code");
        code.append(range.cloneContents());
        if (/\n/.test(code.innerText)) {
          var block = document.createElement("pre");
          block.append(code);
          div.append(block);
        } else {
          div.append(code);
        }
      } else if (onebox) {
        div.append(onebox.dataset.oneboxSrc);
      } else {
        // A selection inside a list item keeps its list.
        var effective = rangeWithoutTrailingHusk(range);
        var fragment = effective.cloneContents();
        var list = ancestor.closest("ul, ol");
        if (list && fragment.firstChild && fragment.firstChild.nodeName === "LI") {
          var wrapper = document.createElement(list.tagName);
          wrapper.append(fragment);
          div.append(wrapper);
        } else {
          div.append(fragment);
        }
      }
    }
    div.querySelectorAll("aside.onebox[data-onebox-src]").forEach(function (element) {
      element.replaceWith(element.dataset.oneboxSrc);
    });
    return div.outerHTML;
  }

  // ---- PostTextSelection ----

  var CSS_TO_DISABLE_FAST_EDIT = ["aside.quote", "aside.onebox", ".cooked-date", "body.encrypted-topic-page"].join(",");

  var quoteState = null;
  var menuInstance = null;
  var timer = null;
  var generation = 0;

  function computeCurrentCooked() {
    var range = selectedRange();
    if (!range) {
      return null;
    }
    // A triple click's selection ends at the barrier: end it before.
    var end = range.endContainer;
    if (end && end.classList && end.classList.contains("cooked-selection-barrier")) {
      range.setEndBefore(end);
      return null;
    }
    var selection = window.getSelection();
    if (selection.isCollapsed || !selection.toString().trim().length) {
      return null;
    }
    var element = elementOf(range.commonAncestorContainer);
    var cooked = element && element.closest(".cooked");
    if (!cooked || !cooked.closest("#posts") || cooked.closest(".small-action")) {
      return null;
    }
    return cooked;
  }

  function computeQuoteState(cooked) {
    var html = selectedHTML();
    var range = selectedRange();
    var article = cooked.closest(".boxed, .reply");
    var postId = article ? article.dataset.postId : null;
    var startNode = range.startContainer;
    function isFirstCookedChild(node) {
      if (node === cooked) {
        return true;
      }
      if (node.parentNode.firstChild !== node) {
        return false;
      }
      return isFirstCookedChild(node.parentNode);
    }
    var canBeFull = range.startOffset === 0 && cooked.contains(startNode) && isFirstCookedChild(startNode);
    var opts = { full: false };
    for (var element = elementOf(range.commonAncestorContainer); element && element.tagName !== "ARTICLE"; element = element.parentElement) {
      if (element.tagName === "ASIDE" && element.classList.contains("quote")) {
        opts.username = element.dataset.username || quoteTitle(element);
        opts.post = element.dataset.post;
        opts.topic = element.dataset.topic;
        break;
      }
    }
    var buffer = (function () {
      var template = document.createElement("template");
      template.innerHTML = html;
      return template.content.textContent || "";
    })();
    var markdown = null;
    return {
      postId: postId,
      opts: opts,
      buffer: buffer,
      markdown: function () {
        if (!markdown) {
          var selected = "";
          var full = false;
          try {
            selected = html ? toMarkdown(html) : "";
            full = canBeFull && selected === toMarkdown(cooked.innerHTML);
          } catch (e) {
            window.console && console.error("toMarkdown failed:", e);
          }
          markdown = { markdown: selected || buffer, full: full };
        }
        return { markdown: markdown.markdown, opts: Object.assign({}, opts, { full: markdown.full }) };
      },
    };
  }

  // getQuoteTitle
  function quoteTitle(element) {
    var title = element.querySelector(".title");
    if (!title) {
      return undefined;
    }
    var link = title.querySelector("a:not(.back)");
    if (link) {
      return link.textContent.trim();
    }
    return title.textContent.trim().replace(/:$/, "");
  }

  function canEditPost(postId) {
    return !!setting("enable_fast_edit") && config.editablePosts.indexOf(parseInt(postId, 10)) !== -1;
  }

  function computeSupportsFastEdit(cooked, state) {
    var supports = canEditPost(state.postId);
    var selection = window.getSelection();
    var start = elementOf(selection.getRangeAt(0).startContainer);
    if (!start || start.closest(CSS_TO_DISABLE_FAST_EDIT)) {
      supports = false;
    }
    if (supports) {
      var markdown = state.markdown().markdown;
      try {
        var matches = cooked.innerHTML.match(
          new RegExp(markdown.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi")
        );
        if (
          markdown.length === 0 ||
          markdown.indexOf("|") !== -1 ||
          /\n/.test(markdown) ||
          /[‚‘’„“”«»‹›™±…→←↔¶]/.test(markdown) ||
          !matches ||
          matches.length !== 1
        ) {
          supports = false;
        }
      } catch (e) {
        supports = false;
      }
    }
    return supports;
  }

  // Sharing.activeSources over the sharing-sources initializer's sources.
  var SOURCES = {
    twitter: {
      icon: "fab-x-twitter",
      title: "share.twitter",
      popupHeight: 265,
      popup: true,
      url: function (link, title, quote) {
        var text = quote ? '"' + quote + '" -- ' : title;
        return "http://x.com/intent/tweet?url=" + encodeURIComponent(link) + "&text=" + encodeURIComponent(text);
      },
    },
    facebook: {
      icon: "fab-facebook",
      title: "share.facebook",
      popup: true,
      url: function (link, title, quote) {
        var app = setting("facebook_app_id");
        var base = app
          ? "https://www.facebook.com/dialog/share?app_id=" + app + "&quote=" + encodeURIComponent(quote || "") + "&href="
          : "https://www.facebook.com/sharer.php?u=";
        return base + encodeURIComponent(link);
      },
    },
    email: {
      icon: "envelope",
      title: "share.email",
      privateContext: true,
      url: function (link, title, quote) {
        var body = quote ? quote + " \n\n " + link : link;
        return (
          "mailto:?to=&subject=" +
          encodeURIComponent("[" + setting("title") + "] " + title) +
          "&body=" +
          encodeURIComponent(body)
        );
      },
    },
  };

  function shareSources() {
    var privateContext = setting("login_required") || config.topic.privateMessage;
    return String(setting("share_quote_buttons") || "")
      .split("|")
      .map(function (id) {
        return SOURCES[id] ? Object.assign({ id: id }, SOURCES[id]) : null;
      })
      .filter(function (s) {
        return s && (!privateContext || s.privateContext);
      });
  }

  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  function quoteSharingEnabled() {
    var visibility = setting("share_quote_visibility");
    return (
      !mobileView() &&
      shareSources().length > 0 &&
      !config.topic.invisible &&
      !config.topic.readRestricted &&
      (visibility === "all" || (visibility === "anonymous" && !config.user))
    );
  }

  function embedQuoteButton() {
    return (config.topic.canCreatePost || config.topic.canReplyAsNewTopic) && !!config.user && config.user.enableQuoting;
  }

  function button(classes, iconName, label, title) {
    return (
      '<button class="btn btn-icon-text btn-flat ' + classes + '" title="' + escapeHtml(title) +
      '" type="button">' + icon(iconName) + '  <span class="d-button-label">' + escapeHtml(label) +
      "</span></button>"
    );
  }

  function iconButton(iconName, title) {
    return (
      '<button class="btn no-text btn-icon btn-flat" title="' + escapeHtml(title) + '" type="button">' +
      icon(iconName) + '  <span aria-hidden="true">&#8203;</span></button>'
    );
  }

  function toolbarContent(state, supportsFastEdit) {
    var wrapper = document.createElement("div");
    wrapper.className = "quote-button visible";
    var html = '<div class="buttons">';
    if (embedQuoteButton()) {
      html += button("insert-quote", "quote-left", t("post.quote_reply"), t("post.quote_reply_shortcut"));
    }
    if (canEditPost(state.postId)) {
      html += button("quote-edit-label", "pencil", t("post.quote_edit"), t("post.quote_edit_shortcut"));
    }
    if (config.user && config.user.enableQuoting) {
      html += button("copy-quote", "copy", t("post.quote_copy"), t("post.quote_copy"));
    }
    var sources = quoteSharingEnabled() ? shareSources() : [];
    sources.forEach(function (source) {
      html += iconButton(source.icon, t(source.title));
    });
    html += '</div><div class="extra"></div>';
    wrapper.innerHTML = html;
    wrapper.addEventListener("click", function (event) {
      var target = event.target.closest("button");
      if (!target) {
        return;
      }
      if (target.classList.contains("insert-quote")) {
        insertQuote(state);
      } else if (target.classList.contains("quote-edit-label")) {
        toggleFastEdit(state, supportsFastEdit);
      } else if (target.classList.contains("copy-quote")) {
        copyQuote(state);
      } else {
        var index = Array.prototype.indexOf.call(wrapper.querySelectorAll(".buttons > .btn.no-text"), target);
        if (sources[index]) {
          share(sources[index], state);
        }
      }
    });
    return wrapper;
  }

  // shouldRenderBelow: on phones and tablets the bar goes under the
  // selection, clear of the platform's own menu.
  function shouldRenderBelow() {
    var html = document.documentElement;
    var ua = navigator.userAgent;
    var touch = html.classList.contains("discourse-touch");
    return (
      html.classList.contains("mobile-device") ||
      html.classList.contains("ios-device") ||
      /Android|iPhone|iPad|iPod/.test(ua) ||
      /Opera|OPR\//.test(ua) ||
      (touch && /Firefox/.test(ua))
    );
  }

  function inViewport(el) {
    if (!el || !el.getBoundingClientRect) {
      return true;
    }
    var rect = el.getBoundingClientRect();
    return rect.top >= 0 && rect.left >= 0 && rect.bottom <= window.innerHeight && rect.right <= window.innerWidth;
  }

  // virtualElementFromTextRange
  function virtualElement() {
    var range = selectedRange();
    var rect = range.getBoundingClientRect();
    return {
      getBoundingClientRect: function () {
        return rect;
      },
      getClientRects: function () {
        return range.getClientRects();
      },
      contextElement: elementOf(range.commonAncestorContainer),
    };
  }

  function showToolbar(cooked) {
    var below = shouldRenderBelow();
    var state = computeQuoteState(cooked);
    quoteState = state;
    var supportsFastEdit = computeSupportsFastEdit(cooked, state);
    var offset = 3;
    if (below) {
      offset = 20;
      var range = selectedRange();
      if (!inViewport(range.startContainer.parentNode) || !inViewport(range.endContainer.parentNode)) {
        offset = 70;
      }
    }
    menuInstance = Discourse.float.menu(virtualElement(), {
      identifier: "post-text-selection-toolbar",
      placement: below ? "bottom-start" : "top-start",
      fallbackPlacements: below ? ["bottom-end"] : ["top-end"],
      offset: offset,
      trapTab: false,
      closeOnScroll: false,
      content: toolbarContent(state, supportsFastEdit),
    });
  }

  function hideToolbar() {
    quoteState = null;
    if (menuInstance) {
      var instance = menuInstance;
      menuInstance = null;
      return instance.destroy();
    }
    return Promise.resolve();
  }

  function selectionChange() {
    if (!config) {
      return;
    }
    // The toolbar's own buttons take focus without changing what is
    // selected; a click in the bar is not a new selection.
    var active = document.activeElement;
    if (active && active.closest && active.closest('[data-identifier="post-text-selection-toolbar"]')) {
      return;
    }
    hideToolbar();
    clearTimeout(timer);
    var mine = ++generation;
    timer = setTimeout(function () {
      if (mine !== generation) {
        return;
      }
      var cooked = computeCurrentCooked();
      if (cooked) {
        showToolbar(cooked);
      } else {
        hideToolbar();
      }
    }, INPUT_DELAY);
  }

  function failed(error) {
    var message = error && error.message ? error.message : String(error);
    if (Discourse.dialog && Discourse.dialog.popupAjaxError) {
      Discourse.dialog.popupAjaxError(message);
    } else {
      window.alert(message);
    }
  }

  function quotePost(state) {
    var md = state.markdown();
    return fetchPost(state.postId).then(function (post) {
      return { post: post, quote: buildQuote(post, md.markdown, md.opts) };
    });
  }

  // TopicController#selectText: the composer takes the quote (at the
  // cursor when open, after the draft otherwise, replying to the post
  // unless it is the first). Without the right to reply, Ember starts a
  // new topic with it instead, which composer.js does not do yet.
  function insertQuote(state) {
    hideToolbar();
    quotePost(state)
      .then(function (result) {
        var post = result.post;
        document.dispatchEvent(
          new CustomEvent("composer:quote", {
            detail: {
              quote: result.quote,
              topicId: post.topic_id,
              postNumber: post.post_number,
              username: post.username,
              canCreatePost: config.topic.canCreatePost,
            },
          })
        );
      })
      .catch(failed);
  }

  // copyQuoteToClipboard
  function copyQuote(state) {
    var text = quotePost(state).then(function (result) {
      return result.quote;
    });
    var done = function () {
      toast(t("post.quote_copied_to_clipboard"));
      hideToolbar();
    };
    if (navigator.clipboard && window.ClipboardItem) {
      navigator.clipboard
        .write([
          new window.ClipboardItem({
            "text/plain": text.then(function (s) {
              return new Blob([s], { type: "text/plain" });
            }),
          }),
        ])
        .then(done, failed);
    } else {
      text
        .then(function (s) {
          return navigator.clipboard.writeText(s);
        })
        .then(done, failed);
    }
  }

  // toasts.success({ duration: "short" })
  function toast(text) {
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var output = document.createElement("output");
    output.className = "fk-d-toast";
    output.setAttribute("role", "status");
    output.innerHTML =
      '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
      icon("check") +
      '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>';
    output.querySelector(".fk-d-default-toast__message").textContent = text;
    section.appendChild(output);
    var close = function () {
      output.remove();
    };
    output.querySelector("button").addEventListener("click", close);
    setTimeout(close, 3000);
  }

  // Sharing.shareSource for the post's URL, with the selected text.
  function share(source, state) {
    var article = document.querySelector('article[data-post-id="' + state.postId + '"]');
    var wrapper = article && article.closest("[data-post-number]");
    var number = wrapper ? parseInt(wrapper.dataset.postNumber, 10) : 1;
    var topic = config.topic;
    var url = location.protocol + "//" + location.host + basePath() + "/t/" + (topic.slug ? topic.slug + "/" : "") + topic.id + (number > 1 ? "/" + number : "");
    var link = source.url(url, topic.title, window.getSelection().toString());
    if (source.popup) {
      window.open(
        link,
        "",
        "menubar=no,toolbar=no,resizable=yes,scrollbars=yes,width=600,height=" + (source.popupHeight || 315)
      );
    } else if (source.id === "email") {
      window.location.href = link;
    } else {
      window.open(link, "_blank");
    }
  }

  // toggleFastEdit: the fast edit modal for a simple selection, else the
  // composer on the post.
  function toggleFastEdit(state, supportsFastEdit) {
    var postId = state.postId;
    var markdown = state.markdown().markdown;
    hideToolbar();
    if (supportsFastEdit) {
      fastEdit(postId, markdown);
      return;
    }
    var article = document.querySelector('article[data-post-id="' + postId + '"]');
    var wrapper = article && article.closest("[data-post-number]");
    document.dispatchEvent(
      new CustomEvent("composer:edit", {
        detail: {
          topicId: config.topic.id,
          postId: postId,
          postNumber: wrapper ? wrapper.dataset.postNumber : null,
        },
      })
    );
  }

  // FastEditModal and FastEdit: the selection in a textarea; saving puts
  // its new text in place of the old in the post's raw.
  function fastEdit(postId, initialValue) {
    if (!Discourse.modal) {
      return;
    }
    var close = Discourse.modal.show({
      className: "fast-edit-modal",
      title: t("post.quote_edit"),
      body: function (body) {
        body.innerHTML =
          '<div class="fast-edit-container"><textarea id="fast-edit-input"></textarea>' +
          '<div class="fast-edit-container__footer"><button class="btn btn-icon-text btn-primary btn-small save-fast-edit" disabled type="button">' +
          icon("pencil") + '<span class="d-button-label">' + escapeHtml(t("composer.save_edit")) +
          "</span></button></div></div>";
        var textarea = body.querySelector("textarea");
        var save = body.querySelector(".save-fast-edit");
        textarea.value = initialValue;
        textarea.addEventListener("input", function () {
          save.disabled = textarea.value === initialValue;
        });
        var saving = false;
        var submit = function () {
          if (saving || textarea.value === initialValue) {
            return;
          }
          saving = true;
          save.disabled = true;
          delete posts[postId];
          fetchPost(postId)
            .then(function (post) {
              var raw = post.raw.replace(initialValue, textarea.value);
              if (raw === post.raw) {
                throw new Error("Failed to update the post. Did your fast edit include a special character?");
              }
              var headers = csrfHeaders();
              headers["Content-Type"] = "application/x-www-form-urlencoded";
              headers["X-Requested-With"] = "XMLHttpRequest";
              headers.Accept = "application/json";
              return fetch(basePath() + "/posts/" + postId, {
                method: "PUT",
                headers: headers,
                credentials: "same-origin",
                body: new URLSearchParams([["post[raw]", raw]]),
              }).then(function (r) {
                if (!r.ok) {
                  return r.json().then(
                    function (body) {
                      throw new Error((body.errors || [r.status + " " + r.statusText]).join(" "));
                    },
                    function () {
                      throw new Error(r.status + " " + r.statusText);
                    }
                  );
                }
                delete posts[postId];
              });
            })
            .catch(failed)
            .finally(function () {
              close();
            });
        };
        save.addEventListener("click", submit);
        textarea.addEventListener("keydown", function (event) {
          if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
            submit();
          }
        });
        setTimeout(function () {
          textarea.focus({ preventScroll: true });
        }, 0);
      },
    });
  }

  document.addEventListener("selectionchange", selectionChange);

  Discourse.onPage(function () {
    readConfig();
    posts = {};
    return function () {
      hideToolbar();
      config = null;
    };
  });
})();
