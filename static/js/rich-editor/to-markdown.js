// HTML into markdown through the editor's schema (static/prosemirror/
// to-markdown.js, lib/to-markdown for a paste or a quoted selection): the
// HTML is parsed with the schema's parseDOM rules and written with the
// serializer, after Word's HTML clean-up (extensions/word-paste.js
// transformWordHtml). Each extension's transformParsedHTML may rewrite the
// parsed document first.
import { DOMParser as ProseMirrorDOMParser } from "prosemirror-model";
import { getExtensions } from "discourse/rich-editor/extensions";
import { isBoundary } from "discourse/rich-editor/plugin-utils";
import { createSchema } from "discourse/rich-editor/schema";
import Serializer from "discourse/rich-editor/serializer";
import { transformWordHtml } from "discourse/rich-editor/extensions/word-paste";

// Chrome and Safari copy the space next to an inline element as a
// non-breaking space, which stops a pasted emoji from cooking. Only that
// copy artifact, a lone nbsp in a bare span, is turned back, so an
// author's own nbsp is kept.
function restoreReplacedSpaces(root) {
  var spans = root.querySelectorAll("span:not([class]):not([style]), span.Apple-converted-space");
  spans.forEach(function (span) {
    if (span.childNodes.length === 1 && span.textContent === " ") {
      span.replaceWith(root.ownerDocument.createTextNode(" "));
    }
  });
}

export default function toMarkdown(html, extensions) {
  extensions = extensions || getExtensions();
  var schema = createSchema(extensions);
  var domParser = ProseMirrorDOMParser.fromSchema(schema);
  var pluginParams = { utils: { isBoundary: isBoundary } };
  var serializer = new Serializer(extensions, pluginParams);

  var parsedDoc = new DOMParser().parseFromString(transformWordHtml(html), "text/html");

  restoreReplacedSpaces(parsedDoc.body);

  extensions.forEach(function (ext) {
    if (typeof ext.transformParsedHTML === "function") {
      try {
        ext.transformParsedHTML(parsedDoc);
      } catch (e) {
        // eslint-disable-next-line no-console
        console.warn(
          "toMarkdown: transformParsedHTML failed for extension " + (ext.name || ext.id || "(unknown)"),
          e
        );
      }
    }
  });

  return serializer.convert(domParser.parse(parsedDoc)).trim();
}
