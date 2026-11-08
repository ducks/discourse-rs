// The rich editor's loader, for pages with the composer: writes the import
// map its ES modules resolve through (static/vendor/prosemirror and
// static/js/rich-editor, no bundler) as soon as it runs, and loads them the
// first time Discourse.richEditor.load() is called (load-rich-editor.js),
// resolving to the editor module (rich-editor/editor.js).
//
// The map must be in the document before the page imports any module
// (lightbox.js and highlight-syntax.js import theirs on demand), so this
// script goes before those. Its tag may carry data-assets, a JSON object of
// asset name to url (with its ?v= digest); a name it lacks is loaded from
// /assets/<name>. The bare specifiers are the packages' own
// ('prosemirror-model'), 'discourse/rich-editor/<module>' for the editor's,
// and 'markdown-it' for rich-editor/markdown-it-stub.js.
(function () {
  "use strict";

  window.Discourse = window.Discourse || {};
  if (Discourse.richEditor) {
    return;
  }

  var script = document.currentScript;
  var control = document.getElementById("reply-control");
  var base = (script && script.dataset.basePath) || (control && control.dataset.basePath) || "";
  var urls = {};
  try {
    urls = JSON.parse((script && script.dataset.assets) || "{}");
  } catch (e) {
    // Undigested urls below: revalidated rather than cached for good.
  }

  var PACKAGES = [
    "orderedmap",
    "prosemirror-commands",
    "prosemirror-dropcursor",
    "prosemirror-gapcursor",
    "prosemirror-highlightjs",
    "prosemirror-history",
    "prosemirror-inputrules",
    "prosemirror-keymap",
    "prosemirror-markdown",
    "prosemirror-model",
    "prosemirror-schema-list",
    "prosemirror-state",
    "prosemirror-transform",
    "prosemirror-view",
    "rope-sequence",
    "w3c-keyname",
  ];
  // prosemirror-codemark's files import each other as './plugin', with no
  // extension: those urls are mapped too.
  var CODEMARK = ["plugin", "actions", "inputRules", "utils", "types"];
  var MODULES = [
    "editor",
    "extensions",
    "inputrules",
    "keymap",
    "parser",
    "plugin-utils",
    "schema",
    "serializer",
    "to-markdown",
    "engine",
    "text-manipulation",
    "toolbar",
    "extensions/emoji",
    "extensions/mention",
    "extensions/hashtag",
    "extensions/link",
    "extensions/link-toolbar",
    "extensions/quote",
    "extensions/code-block",
    "extensions/html-inline",
    "extensions/html-block",
    "extensions/trailing-inline-space",
    "extensions/markdown-paste",
    "extensions/word-paste",
    "extensions/table",
    "table/chrome",
    "table/commands",
    "table/drag-autoscroll",
    "table/editing",
    "table/grid",
    "table/menu",
    "table/node-view",
    "table/paste",
    "table/repair",
    "table/track-pointer",
    "extensions/image",
    "extensions/upload-placeholder",
    "extensions/grid",
    "extensions/override-drag-ghost",
    "node-views/image",
    "node-views/video",
    "node-views/grid",
    "node-views/upload-placeholder",
    "node-views/image-alt-text-input",
    "extensions/wrap",
    "node-views/wrap",
    "wrap-utils",
    "wrap-modal",
    "plugins",
    "plugins/checklist",
    "plugins/details",
    "plugins/footnote",
    "plugins/local-dates",
    "plugins/math",
    "plugins/policy",
    "plugins/poll",
    "plugins/poll-node-view",
    "plugins/spoiler",
  ];

  function url(name) {
    return urls[name] || base + "/assets/" + name;
  }

  var imports = {};
  PACKAGES.forEach(function (pkg) {
    imports[pkg] = url("prosemirror/" + pkg + ".js");
  });
  imports["prosemirror-codemark"] = url("prosemirror/prosemirror-codemark/index.js");
  CODEMARK.forEach(function (file) {
    imports[base + "/assets/prosemirror/prosemirror-codemark/" + file] = url(
      "prosemirror/prosemirror-codemark/" + file + ".js"
    );
  });
  MODULES.forEach(function (module) {
    imports["discourse/rich-editor/" + module] = url("rich-editor/" + module + ".js");
  });
  imports["markdown-it"] = url("rich-editor/markdown-it-stub.js");

  var importMap = { imports: imports };
  if (!document.querySelector('script[type="importmap"][data-rich-editor]')) {
    var tag = document.createElement("script");
    tag.type = "importmap";
    tag.dataset.richEditor = "";
    tag.textContent = JSON.stringify(importMap);
    document.head.appendChild(tag);
  }

  var loading = null;

  Discourse.richEditor = {
    importMap: importMap,
    load: function () {
      if (!loading) {
        loading = import("discourse/rich-editor/editor").catch(function (e) {
          // Allow a retry next time.
          loading = null;
          throw e;
        });
      }
      return loading;
    },
  };
})();
