// The editor's schema (static/prosemirror/core/schema.js): prosemirror-
// markdown's CommonMark schema, unless includeDefault is false, with each
// extension's nodeSpec and markSpec on top. A mark spec naming `before`
// goes before that mark, which orders how marks nest when serialized.
import OrderedMap from "orderedmap";
import { schema as defaultMarkdownSchema } from "prosemirror-markdown";
import { Schema } from "prosemirror-model";

export function createSchema(extensions, includeDefault) {
  if (includeDefault === undefined) {
    includeDefault = true;
  }

  var nodes = includeDefault ? defaultMarkdownSchema.spec.nodes : OrderedMap.from({});
  var marks = includeDefault ? defaultMarkdownSchema.spec.marks : OrderedMap.from({});

  var nodeSpecs = extractSpecs(extensions, "nodeSpec");
  Object.keys(nodeSpecs).forEach(function (type) {
    nodes = nodes.update(type, nodeSpecs[type]);
  });

  var markSpecs = extractSpecs(extensions, "markSpec");
  Object.keys(markSpecs).forEach(function (type) {
    var spec = markSpecs[type];
    marks = spec.before ? marks.addBefore(spec.before, type, spec) : marks.update(type, spec);
  });

  return new Schema({ nodes: nodes, marks: marks });
}

function extractSpecs(extensions, key) {
  var specs = {};
  extensions.forEach(function (extension) {
    Object.assign(specs, extension[key]);
  });
  return specs;
}
