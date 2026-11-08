// lib/wrap-utils.js: a [wrap]'s attributes as the flat object its node
// keeps ({ wrap: "toc", id: "123" }) and back, quoted as the server parses
// them (parseBBCodeTag).
import { buildBBCodeAttrs, parseBBCodeTag, serializeBBCodeAttr } from "discourse/rich-editor/plugin-utils";

export function parseAttributesString(attrsString) {
  if (!attrsString.trim()) {
    return {};
  }

  var source = "[wrap" + attrsString + "]";
  var parsed = parseBBCodeTag(source, 0, source.length);

  if (!parsed || !parsed.attrs) {
    return {};
  }

  var rest = Object.assign({}, parsed.attrs);
  var wrap = rest._default;
  delete rest._default;
  return wrap ? Object.assign({ wrap: wrap }, rest) : rest;
}

export function serializeAttributes(data) {
  if (!data || Object.keys(data).length === 0) {
    return "";
  }

  // An empty name serializes the wrap as the nameless `=value` default.
  var wrapName = data.wrap ? serializeBBCodeAttr(data.wrap, "").trim() : "";
  var otherAttrs = buildBBCodeAttrs(data, { skipAttrs: ["wrap"] });

  if (wrapName && otherAttrs) {
    return wrapName + " " + otherAttrs;
  }

  return wrapName || (otherAttrs ? " " + otherAttrs : "");
}

// The modal's name and { key, value } rows.
export function serializeFromForm(name, attributes) {
  var data = {};
  if (name) {
    data.wrap = name;
  }
  (attributes || []).forEach(function (attr) {
    if (attr.key && attr.value) {
      data[attr.key] = attr.value;
    }
  });
  return serializeAttributes(data);
}

// lib/case-converter.js
export function camelCaseToDash(str) {
  return str.replace(/([a-zA-Z])(?=[A-Z])/g, "$1-").toLowerCase();
}
