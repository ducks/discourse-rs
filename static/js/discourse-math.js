// discourse-math in the browser, which the layout loads while
// discourse_math_enabled is on: initializers/discourse-math.js with
// lib/math-renderer.js and its loaders. The cooked `.math` and
// `.asciimath` elements are typeset with MathJax or KaTeX, by
// discourse_math_provider.
//
//   Discourse.renderMath(root)
//       typesets what is under root: posts (as a cooked decorator, so
//       pages and posts htmx swaps in too), the composer's preview (after
//       200ms of quiet, as Rails delays a `.d-editor-preview`) and chat
//       messages.
//
// The renderers are the discourse_math_bundle gem's, served under the
// script tag's data-bundle-url (lib/math-bundle-paths.js: the site's
// discourse_math_bundle_url under the base path; crate::plugins::math).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.renderMath) {
    return;
  }

  var script = document.currentScript;
  var data = script.dataset;
  // buildDiscourseMathOptions
  var opts = {
    provider: data.provider,
    enable_menu: data.enableMenu === "true",
    enable_asciimath: data.enableAsciimath === "true" && data.provider === "mathjax",
    enable_accessibility: data.enableAccessibility === "true",
    mathjax_output: data.mathjaxOutput,
    zoom_on_click: data.zoomOnClick === "true",
  };
  // getMathJaxBasePath and getKaTeXBasePath.
  var bundleBase = data.bundleUrl || "/plugins/discourse-math";
  var mathjaxBase = bundleBase + "/mathjax";
  var katexBase = bundleBase + "/katex";

  var CSS_CLASSES = {
    HIDDEN: "math-hidden",
    APPLIED_MATHJAX: "math-applied-mathjax",
    APPLIED_KATEX: "math-applied-katex",
  };
  var PREVIEW_RENDER_DELAY = 200;
  var ORIGINAL_TEXT_ATTR = "data-math-original";
  var SAFE_HTML_ID_REGEX = /^[A-Za-z_][A-Za-z0-9_.:-]*$/;

  var loaded = {};
  function loadScript(src, css) {
    if (!loaded[src]) {
      loaded[src] = new Promise(function (resolve, reject) {
        var el;
        if (css) {
          el = document.createElement("link");
          el.rel = "stylesheet";
          el.href = src;
        } else {
          el = document.createElement("script");
          el.src = src;
        }
        el.onload = resolve;
        el.onerror = function () {
          reject(new Error("could not load " + src));
        };
        document.head.appendChild(el);
      });
    }
    return loaded[src];
  }

  function warnMathRender(message, error) {
    var suffix = error && error.message ? " (" + error.message + ")" : "";
    window.console && console.warn("discourse-math: " + message + suffix);
  }

  // ---- MathJax ----

  // buildMathJaxConfig
  function buildMathJaxConfig() {
    var config = {
      startup: {
        typeset: false,
        ready: function () {
          return window.MathJax.startup.defaultReady();
        },
      },
      chtml: {},
      svg: {},
      // Rails sets only `mathjax`. MathJax's default `fonts` path is a
      // third-party CDN, where it fetches the font's dynamic glyph ranges, which
      // the gem does not ship; pinned to the bundle so no request leaves
      // the site (those ranges 404 instead).
      loader: {
        load: ["ui/safe"],
        paths: { mathjax: mathjaxBase, fonts: mathjaxBase },
      },
      options: { menuOptions: { settings: {} } },
      tex: {
        inlineMath: [["\\(", "\\)"]],
        displayMath: [["\\[", "\\]"]],
      },
      asciimath: { delimiters: [["%", "%"]] },
    };
    if (opts.mathjax_output === "html") {
      config.chtml.fontURL = mathjaxBase + "/woff-v2";
    } else if (opts.mathjax_output === "svg") {
      config.svg.fontCache = "global";
    }
    config.options.enableMenu = !!opts.enable_menu;
    config.options.menuOptions.settings = { enrich: !!opts.enable_accessibility };
    if (opts.zoom_on_click) {
      config.options.menuOptions.settings.zoom = "Click";
      config.options.menuOptions.settings.zscale = "175%";
    }
    if (opts.enable_asciimath) {
      config.loader.load.push("input/asciimath");
    }
    return config;
  }

  // loadMathJax: the output format is locked to the first load.
  var mathjaxPromise = null;
  function ensureMathJax() {
    if (!mathjaxPromise) {
      window.MathJax = buildMathJaxConfig();
      var bundle = opts.mathjax_output === "svg" ? "tex-mml-svg.js" : "tex-mml-chtml.js";
      mathjaxPromise = loadScript(mathjaxBase + "/" + bundle).then(function () {
        return window.MathJax.startup.promise.then(function () {
          return window.MathJax;
        });
      });
    }
    return mathjaxPromise;
  }

  function createHiddenWrapper(tag, className, content) {
    var wrapper = document.createElement(tag);
    wrapper.className = className + " " + CSS_CLASSES.HIDDEN;
    wrapper.setAttribute("hidden", "");
    wrapper.textContent = content;
    return wrapper;
  }

  function buildMathJaxWrapper(elem) {
    var wrapper = null;
    if (elem.classList.contains("math")) {
      var tag = elem.tagName === "DIV" ? "div" : "span";
      var displayClass = tag === "div" ? "block-math" : "inline-math";
      var delimiter = tag === "div" ? ["\\[", "\\]"] : ["\\(", "\\)"];
      wrapper = createHiddenWrapper(
        tag,
        "math-container " + displayClass + " mathjax-math",
        delimiter[0] + elem.textContent + delimiter[1]
      );
    } else if (elem.classList.contains("asciimath")) {
      wrapper = createHiddenWrapper(
        "span",
        "math-container inline-math ascii-math mathjax-math",
        "%" + elem.textContent + "%"
      );
    }
    if (wrapper) {
      elem.after(wrapper);
    }
    return wrapper;
  }

  function selector() {
    return opts.enable_asciimath ? ".math, .asciimath" : ".math";
  }

  function collectMathWrappers(elem) {
    var wrappers = [];
    elem.querySelectorAll(selector()).forEach(function (mathElem) {
      if (mathElem.classList.contains(CSS_CLASSES.APPLIED_MATHJAX)) {
        return;
      }
      mathElem.classList.add(CSS_CLASSES.APPLIED_MATHJAX);
      var wrapper = buildMathJaxWrapper(mathElem);
      if (wrapper) {
        wrappers.push({ original: mathElem, wrapper: wrapper });
      }
    });
    return wrappers;
  }

  function isElementVisible(elem) {
    if (!elem || !elem.isConnected || elem.closest("[hidden]")) {
      return false;
    }
    var style = window.getComputedStyle(elem);
    return style.display !== "none" && style.visibility !== "hidden";
  }

  function setHidden(elem, hidden) {
    if (elem && elem.isConnected) {
      elem.classList.toggle(CSS_CLASSES.HIDDEN, hidden);
      if (hidden) {
        elem.setAttribute("hidden", "");
      } else {
        elem.removeAttribute("hidden");
      }
    }
  }

  function showRenderedMath(active) {
    active.forEach(function (item) {
      setHidden(item.original, true);
      if (item.wrapper.isConnected) {
        // Inline styles are stripped so the plugin's CSS lays it out.
        item.wrapper.querySelectorAll("mjx-container").forEach(function (container) {
          container.style.removeProperty("display");
        });
        setHidden(item.wrapper, false);
      }
    });
  }

  function revertFailedMathRendering(wrappers) {
    wrappers.forEach(function (item) {
      if (item.original.isConnected) {
        item.original.classList.remove(CSS_CLASSES.HIDDEN, CSS_CLASSES.APPLIED_MATHJAX);
        item.original.removeAttribute("hidden");
      }
      if (item.wrapper.isConnected) {
        item.wrapper.remove();
      }
    });
  }

  function typesetMathJax(wrappers) {
    return ensureMathJax()
      .then(function (MathJax) {
        var active = wrappers.filter(function (item) {
          return item.wrapper.isConnected && isElementVisible(item.original);
        });
        if (active.length === 0 || !MathJax || !MathJax.typesetPromise) {
          return;
        }
        return MathJax.typesetPromise(
          active.map(function (item) {
            return item.wrapper;
          })
        ).then(function () {
          showRenderedMath(active);
        });
      })
      .catch(function (error) {
        warnMathRender("MathJax rendering failed", error);
        revertFailedMathRendering(wrappers);
      });
  }

  // scheduleMathJaxTypeset: a preview's wrappers gather until it is quiet.
  var pending = new WeakMap();
  function renderMathJax(elem) {
    var wrappers = collectMathWrappers(elem);
    if (wrappers.length === 0) {
      return;
    }
    var delay = elem.classList.contains("d-editor-preview") ? PREVIEW_RENDER_DELAY : 0;
    var entry = pending.get(elem);
    if (entry) {
      clearTimeout(entry.timer);
      entry.wrappers = entry.wrappers.concat(wrappers);
    } else {
      entry = { wrappers: wrappers };
      pending.set(elem, entry);
    }
    entry.timer = setTimeout(function () {
      pending.delete(elem);
      typesetMathJax(entry.wrappers);
    }, delay);
  }

  // ---- KaTeX ----

  var katexPromise = null;
  function ensureKaTeX() {
    if (!katexPromise) {
      katexPromise = loadScript(katexBase + "/katex.min.css", true)
        .then(function () {
          return loadScript(katexBase + "/katex.min.js");
        })
        .then(function () {
          return Promise.all([
            loadScript(katexBase + "/contrib/mhchem.min.js"),
            loadScript(katexBase + "/contrib/copy-tex.min.js"),
          ]);
        });
    }
    return katexPromise;
  }

  // sanitizeHref, as far as the browser's URL takes it.
  function sanitizeHref(url) {
    if (!url || /[<>"']/.test(url)) {
      return null;
    }
    try {
      var parsed = new URL(url, window.location.origin);
      var relative = url.startsWith("/") || url.startsWith("#");
      if (!relative && ["http:", "https:", "mailto:"].indexOf(parsed.protocol) === -1) {
        return null;
      }
      return url.trim() === "" ? null : url;
    } catch (e) {
      return null;
    }
  }

  var katexOpts = {
    trust: function (context) {
      if (context.command === "\\href") {
        return Boolean(sanitizeHref(context.url));
      }
      if (context.command === "\\htmlId") {
        var htmlId = context.url || context.text || context.id;
        return Boolean(htmlId) && SAFE_HTML_ID_REGEX.test(htmlId);
      }
      return false;
    },
    macros: {
      "\\eqref": "\\href{###1}{(\\text{#1})}",
      "\\ref": "\\href{###1}{\\text{#1}}",
      "\\label": "\\htmlId{#1}{}",
    },
    displayMode: false,
  };

  function decorateKatex(elem) {
    if (elem.classList.contains(CSS_CLASSES.APPLIED_KATEX) || !elem.classList.contains("math")) {
      return;
    }
    elem.classList.add(CSS_CLASSES.APPLIED_KATEX);
    var displayMode = elem.tagName === "DIV";
    var displayClass = displayMode ? "block-math" : "inline-math";
    var hasKatexContent = !!elem.querySelector(".katex");
    var text = hasKatexContent ? elem.getAttribute(ORIGINAL_TEXT_ATTR) : elem.textContent;
    var annotation = elem.querySelector("annotation[encoding='application/x-tex']");
    var rawText = text || (annotation && annotation.textContent) || elem.textContent || "";
    elem.setAttribute(ORIGINAL_TEXT_ATTR, rawText);
    elem.classList.add("math-container", displayClass, "katex-math");
    elem.textContent = "";
    try {
      window.katex.render(rawText, elem, Object.assign({}, katexOpts, { displayMode: displayMode }));
    } catch (error) {
      warnMathRender("KaTeX rendering failed", error);
      elem.textContent = rawText;
      elem.removeAttribute(ORIGINAL_TEXT_ATTR);
      elem.classList.remove(CSS_CLASSES.APPLIED_KATEX, "math-container", displayClass, "katex-math");
    }
  }

  function renderKatex(elem) {
    var mathElems = elem.querySelectorAll(".math");
    if (mathElems.length === 0) {
      return;
    }
    ensureKaTeX().then(
      function () {
        mathElems.forEach(decorateKatex);
      },
      function (error) {
        warnMathRender("KaTeX failed to load", error);
      }
    );
  }

  // renderMathInElement
  Discourse.renderMath = function (elem) {
    if (!elem) {
      return;
    }
    if (opts.provider === "mathjax") {
      renderMathJax(elem);
    } else if (opts.provider === "katex") {
      renderKatex(elem);
    }
  };

  if (Discourse.decorateCooked) {
    Discourse.decorateCooked("discourse-math", Discourse.renderMath);
  }

  // ---- The composer's Insert Math (modal/math-insert.gjs) ----

  var strings = {};
  try {
    strings = JSON.parse(data.strings || "{}");
  } catch (e) {
    strings = {};
  }
  function t(key) {
    return strings[key] === undefined ? key : strings[key];
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // isAtLineStart: nothing but whitespace since the last newline.
  function isAtLineStart(pre) {
    if (!pre) {
      return true;
    }
    var lastNewline = pre.lastIndexOf("\n");
    return (lastNewline === -1 ? pre : pre.slice(lastNewline + 1)).trim() === "";
  }

  function checkIcon() {
    return Discourse.cooked ? Discourse.cooked.icon("check") : "";
  }

  Discourse.mathInsert = {
    // pre: the composer's text before the cursor; addText inserts.
    show: function (pre, addText) {
      if (!Discourse.modal) {
        return;
      }
      var isBlock = isAtLineStart(pre);
      var root = document.createElement("div");
      root.innerHTML =
        '<div class="math-insert-modal__toggle"><div class="d-toggle-switch">' +
        '<label class="d-toggle-switch__label"><button aria-checked="' + isBlock +
        '" class="d-toggle-switch__checkbox" role="switch" type="button"></button>' +
        '<span class="d-toggle-switch__checkbox-slider">' + (isBlock ? checkIcon() : "") +
        "</span></label>" +
        '<span class="d-toggle-switch__checkbox-label">' + escapeHtml(t("discourse_math.insert_modal.block_mode")) +
        "</span></div></div>" +
        '<form class="form-kit" novalidate><div class="form-kit__container form-kit__field form-kit__field-textarea --full" data-name="text">' +
        '<label class="form-kit__container-title"><span>' + escapeHtml(t("discourse_math.insert_modal.label")) +
        "</span></label>" +
        '<div class="form-kit__container-content"><textarea name="text" class="form-kit__control-textarea math-insert-modal__textarea" placeholder="' +
        escapeHtml(t("discourse_math.insert_modal.placeholder")) + '"></textarea></div>' +
        '<p class="form-kit__errors" hidden></p></div></form>';
      var toggle = root.querySelector(".d-toggle-switch__checkbox");
      var slider = root.querySelector(".d-toggle-switch__checkbox-slider");
      var textarea = root.querySelector("textarea");
      var errors = root.querySelector(".form-kit__errors");
      var close;
      var titleOf = function () {
        return t(isBlock ? "discourse_math.insert_modal.title_block" : "discourse_math.insert_modal.title_inline");
      };
      function submit() {
        var text = textarea.value.trim();
        // @validation="required"
        if (!textarea.value) {
          errors.textContent = t("form_kit.errors.required");
          errors.hidden = false;
          return;
        }
        if (text) {
          addText(isBlock ? "$$\n" + text + "\n$$\n" : "$" + text + "$");
        }
        close();
      }
      close = Discourse.modal.show({
        className: "math-insert-modal",
        title: titleOf(),
        body: function (el) {
          el.appendChild(root);
        },
        footer: function (el) {
          el.innerHTML =
            '<button class="btn btn-text btn-primary math-insert-modal__insert" type="button"><span class="d-button-label">' +
            escapeHtml(t("discourse_math.insert_modal.insert")) + "</span></button>" +
            '<button class="btn btn-text btn-default" type="button"><span class="d-button-label">' +
            escapeHtml(t("cancel")) + "</span></button>";
          el.querySelector(".math-insert-modal__insert").addEventListener("click", submit);
          el.querySelector(".btn-default").addEventListener("click", function () {
            close();
          });
        },
      });
      toggle.addEventListener("click", function () {
        isBlock = !isBlock;
        toggle.setAttribute("aria-checked", String(isBlock));
        slider.innerHTML = isBlock ? checkIcon() : "";
        var title = document.querySelector(".math-insert-modal .d-modal__title-text");
        if (title) {
          title.textContent = titleOf();
        }
      });
      root.querySelector("form").addEventListener("submit", function (event) {
        event.preventDefault();
        submit();
      });
      textarea.focus();
    },
  };
})();
