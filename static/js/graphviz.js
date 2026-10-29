// discourse-graphviz in the browser, which the layout loads while
// discourse_graphviz_enabled is on: api-initializers/discourse-graphviz.js
// with lib/render-graphviz.js and the GraphvizInline, GraphvizDiagram and
// GraphvizFullscreen components.
//
//   Discourse.renderGraphviz(root)
//       replaces each cooked div.graphviz under root by its diagram, drawn
//       by viz.js: posts (as a cooked decorator, so what htmx swaps in
//       too) and the composer's preview.
//
// viz.js is the plugin's own copy, served where Rails serves it
// (/plugins/discourse-graphviz/javascripts/viz-3.0.1.js), loaded the
// first time a page has a graph.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.renderGraphviz) {
    return;
  }

  var script = document.currentScript;
  var vizUrl = script.dataset.viz;
  var sample = script.dataset.sample || "";
  var fullscreenLabel = script.dataset.fullscreen || "";

  var vizPromise = null;
  function loadViz() {
    if (!vizPromise) {
      vizPromise = new Promise(function (resolve, reject) {
        var el = document.createElement("script");
        el.src = vizUrl;
        el.onload = resolve;
        el.onerror = function () {
          vizPromise = null;
          reject(new Error("could not load " + vizUrl));
        };
        document.head.appendChild(el);
      });
    }
    return vizPromise;
  }

  // hrefAllowed (pretty-text/sanitizer) for http(s), relative and anchor
  // links; mailto is dropped as well.
  function hrefAllowed(href) {
    var trimmed = href.trim();
    if (/^\s*mailto:/i.test(trimmed)) {
      return false;
    }
    var scheme = /^([a-z][a-z0-9+.-]*):/i.exec(trimmed);
    if (!scheme) {
      return true;
    }
    return ["http", "https"].indexOf(scheme[1].toLowerCase()) !== -1;
  }

  // sanitizeGraphvizSvg: viz.js does not sanitize its output.
  function sanitizeGraphvizSvg(svg) {
    svg.querySelectorAll("script, foreignObject").forEach(function (el) {
      el.remove();
    });
    svg.querySelectorAll("*").forEach(function (el) {
      Array.prototype.slice.call(el.attributes).forEach(function (attr) {
        if (attr.name.toLowerCase().startsWith("on") || /javascript:/i.test(attr.value)) {
          el.removeAttribute(attr.name);
        }
      });
    });
    svg.querySelectorAll("a").forEach(function (anchor) {
      var href = anchor.getAttribute("href") || anchor.getAttribute("xlink:href");
      if (href && !hrefAllowed(href)) {
        anchor.replaceWith.apply(anchor, Array.prototype.slice.call(anchor.childNodes));
      }
    });
    return svg;
  }

  // generateGraph: the svg element, or a thrown error with viz's message.
  function generateGraph(source, engine) {
    return loadViz().then(function () {
      /* global vizRenderStringSync */
      var rendered = window.vizRenderStringSync(source, { format: "svg", engine: engine });
      var doc = new DOMParser().parseFromString(rendered, "image/svg+xml");
      var error = doc.querySelector("parsererror");
      if (error) {
        throw new Error(error.textContent);
      }
      return sanitizeGraphvizSvg(document.importNode(doc.documentElement, true));
    });
  }

  // GraphvizDiagram: the graph drawn into a .graphviz-diagram, or its
  // error; zoomable in the fullscreen view.
  function diagram(src, engine, enableZoom) {
    var el = document.createElement("div");
    el.className = "graphviz-diagram" + (enableZoom ? " zoomable" : "");
    el.innerHTML = '<div class="spinner small"></div>';
    el.addEventListener("click", function (event) {
      event.preventDefault();
      if (enableZoom) {
        el.classList.toggle("zoomed");
      }
    });
    generateGraph(src, engine).then(
      function (svg) {
        el.innerHTML = "";
        el.appendChild(svg);
      },
      function (error) {
        el.innerHTML = "";
        var message = document.createElement("div");
        message.className = "graph-error";
        message.textContent = error.message;
        el.appendChild(message);
      }
    );
    return el;
  }

  function fullscreen(src, engine) {
    if (!Discourse.modal) {
      return;
    }
    Discourse.modal.show({
      className: "graphviz-fullscreen",
      body: function (el) {
        el.appendChild(diagram(src, engine, true));
      },
    });
  }

  function icon(name) {
    return Discourse.cooked ? Discourse.cooked.icon(name) : "";
  }

  // applyGraphviz: the cooked placeholder becomes a .graphviz-wrapper with
  // GraphvizInline (the fullscreen button and the diagram).
  function applyGraphviz(element) {
    var src = element.textContent.trim();
    if (!src) {
      return;
    }
    var engine = element.dataset.engine;
    var wrapper = document.createElement("div");
    wrapper.classList.add("graphviz-wrapper");
    var controls = document.createElement("div");
    controls.className = "graphviz-diagram-controls";
    var button = document.createElement("button");
    button.className = "btn no-text btn-icon btn-flat graphviz-fullscreen-button";
    button.type = "button";
    if (fullscreenLabel) {
      button.title = fullscreenLabel;
    }
    button.innerHTML = icon("discourse-expand");
    button.addEventListener("click", function () {
      fullscreen(src, engine);
    });
    controls.appendChild(button);
    wrapper.appendChild(controls);
    wrapper.appendChild(diagram(src, engine, false));
    element.replaceWith(wrapper);
  }

  Discourse.renderGraphviz = function (root) {
    root.querySelectorAll("div.graphviz").forEach(applyGraphviz);
  };

  // The composer's options menu item: a sample graph, or the selection,
  // between the tags.
  Discourse.graphvizComposer = function (applySurround) {
    applySurround("\n[graphviz]\n", "\n[/graphviz]\n", sample, false);
  };

  if (Discourse.decorateCooked) {
    Discourse.decorateCooked("discourse-graphviz", Discourse.renderGraphviz);
  }
})();
