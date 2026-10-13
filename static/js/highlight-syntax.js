// Syntax highlighting of code blocks, as Ember's lib/highlight-syntax.js
// does it from the post-decorations initializer:
//
//   Discourse.highlightSyntax(el)
//       highlights el's `pre code[class]` blocks (every `pre code` with
//       autohighlight_all_code), loading highlight.js the first time a
//       page has one. Returns a promise. Each block is looked at once.
//
// Runs on every cooked post (Discourse.decorateCooked); the composer's
// preview and chat messages call it themselves. The page's
// meta[name=discourse-highlight-js] gives the site's language bundle
// (HighlightJs.path), the core module's url and the setting.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.highlightSyntax) {
    return;
  }

  // bounds language detection cost, which scales with content length
  var AUTO_DETECT_SAMPLE_LENGTH = 2000;

  var hljsLoadPromise = null;
  var seen = new WeakSet();

  function config() {
    return document.querySelector('meta[name="discourse-highlight-js"]');
  }

  Discourse.highlightSyntax = function (elem) {
    var meta = config();
    if (!elem || !meta) {
      return Promise.resolve();
    }

    var selector = meta.hasAttribute("data-all-code") ? "pre code" : "pre code[class]";

    var codeblocks = Array.prototype.filter.call(elem.querySelectorAll(selector), function (e) {
      return !seen.has(e);
    });

    if (!codeblocks.length) {
      return Promise.resolve();
    }

    return ensureHighlightJs(meta).then(function (hljs) {
      codeblocks.forEach(function (e) {
        if (seen.has(e)) {
          return;
        }
        seen.add(e);

        var lang;
        for (var i = 0; i < e.classList.length; i++) {
          var m = e.classList[i].match(/^lang(?:uage)?-(.+)$/);
          if (m) {
            lang = m[1];
            break;
          }
        }

        var autodetect = lang === "auto";
        if (autodetect) {
          // not a real hljs language, removed even from blocks skipped below
          e.classList.remove("lang-auto");
        }

        // Large code blocks can cause crashes or slowdowns
        if (e.innerHTML.length > 30000) {
          return;
        }

        if (autodetect) {
          var text = e.textContent;
          var language = hljs.highlightAuto(text.trimStart().slice(0, AUTO_DETECT_SAMPLE_LENGTH)).language;

          if (language) {
            // highlightElement will use this grammar instead of re-detecting
            e.classList.add("language-" + language);
          } else if (text.length > AUTO_DETECT_SAMPLE_LENGTH) {
            // too large for a full re-scan; smaller blocks fall through
            return;
          }

          hljs.highlightElement(e);
        } else if (lang && hljs.getLanguage(lang)) {
          hljs.highlightElement(e);
        } else {
          // To make debugging easier, add a data attribute to indicate we skipped it
          e.dataset.unknownHljsLang = lang;
        }
      });
    });
  };

  function ensureHighlightJs(meta) {
    if (!hljsLoadPromise) {
      hljsLoadPromise = loadHighlightJs(meta).catch(function (e) {
        // Allow retry next time
        hljsLoadPromise = null;
        throw e;
      });
    }
    return hljsLoadPromise;
  }

  // The core and the site's language bundle (HighlightJsController), both
  // ES modules; URL adds the page's host to the relative urls.
  function loadHighlightJs(meta) {
    return Promise.all([
      import(new URL(meta.dataset.core, window.location).href),
      import(new URL(meta.content, window.location).href),
    ]).then(function (modules) {
      var hljs = modules[0].default;
      modules[1].default(hljs);
      hljs.addPlugin(mergeHTMLPlugin);
      hljs.configure({
        ignoreUnescapedHTML: true,
      });
      return hljs;
    });
  }

  // lib/highlight-syntax-merge-html-plugin.js, from
  // https://github.com/highlightjs/highlight.js/issues/2889#issue-748412174:
  // keeps the markup inside a code block (a link, a mention) by merging it
  // with the highlighted token stream.
  var mergeHTMLPlugin = (function () {
    var originalStream;

    function escapeHTML(value) {
      return value
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#x27;");
    }

    var plugin = {
      // preserve the original HTML token stream
      "before:highlightElement": function (data) {
        originalStream = nodeStream(data.el);
      },
      // merge it afterwards with the highlighted token stream
      "after:highlightElement": function (data) {
        if (!originalStream.length) {
          return;
        }

        var resultNode = document.createElement("div");
        resultNode.innerHTML = data.result.value;
        data.result.value = mergeStreams(originalStream, nodeStream(resultNode), data.text);
        data.el.innerHTML = data.result.value;
      },
    };

    function tag(node) {
      return node.nodeName.toLowerCase();
    }

    function nodeStream(node) {
      var result = [];
      (function _nodeStream(node, offset) {
        for (var child = node.firstChild; child; child = child.nextSibling) {
          if (child.nodeType === 3) {
            offset += child.nodeValue.length;
          } else if (child.nodeType === 1) {
            result.push({
              event: "start",
              offset: offset,
              node: child,
            });
            offset = _nodeStream(child, offset);
            // Prevent void elements from having an end tag that would actually
            // double them in the output. There are more void elements in HTML
            // but we list only those realistically expected in code display.
            if (!tag(child).match(/br|hr|img|input/)) {
              result.push({
                event: "stop",
                offset: offset,
                node: child,
              });
            }
          }
        }
        return offset;
      })(node, 0);
      return result;
    }

    function mergeStreams(original, highlighted, value) {
      var processed = 0;
      var result = "";
      var nodeStack = [];

      function selectStream() {
        if (!original.length || !highlighted.length) {
          return original.length ? original : highlighted;
        }
        if (original[0].offset !== highlighted[0].offset) {
          return original[0].offset < highlighted[0].offset ? original : highlighted;
        }
        // Original always starts first and closes last.
        return highlighted[0].event === "start" ? original : highlighted;
      }

      function open(node) {
        function attributeString(attr) {
          return " " + attr.nodeName + '="' + escapeHTML(attr.value) + '"';
        }
        result += "<" + tag(node) + [].map.call(node.attributes, attributeString).join("") + ">";
      }

      function close(node) {
        result += "</" + tag(node) + ">";
      }

      function render(event) {
        (event.event === "start" ? open : close)(event.node);
      }

      while (original.length || highlighted.length) {
        var stream = selectStream();
        result += escapeHTML(value.substring(processed, stream[0].offset));
        processed = stream[0].offset;
        if (stream === original) {
          // On any opening or closing tag of the original markup we first
          // close the entire highlighted node stack, then render the
          // original tag along with all the following original tags at the
          // same offset and then reopen all the tags on the highlighted
          // stack.
          nodeStack.reverse().forEach(close);
          do {
            render(stream.splice(0, 1)[0]);
            stream = selectStream();
          } while (stream === original && stream.length && stream[0].offset === processed);
          nodeStack.reverse().forEach(open);
        } else {
          if (stream[0].event === "start") {
            nodeStack.push(stream[0].node);
          } else {
            nodeStack.pop();
          }
          render(stream.splice(0, 1)[0]);
        }
      }
      return result + escapeHTML(value.substr(processed));
    }

    return plugin;
  })();

  if (Discourse.decorateCooked) {
    Discourse.decorateCooked("highlight-syntax", Discourse.highlightSyntax);
  }
})();
