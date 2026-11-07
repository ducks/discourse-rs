// The bundled plugins' rich editor extensions (each plugin's
// api.registerRichEditorExtension), by the name the composer's context
// lists for the plugins the site has on (src/composer/rich_editor.rs).
// Discourse loads only an enabled plugin's JavaScript, so only those are
// registered, after the defaults and in the plugins' load order.
import { getExtensions, registerRichEditorExtension } from "discourse/rich-editor/extensions";
import checklist from "discourse/rich-editor/plugins/checklist";
import details from "discourse/rich-editor/plugins/details";
import footnote from "discourse/rich-editor/plugins/footnote";
import localDates from "discourse/rich-editor/plugins/local-dates";
import math from "discourse/rich-editor/plugins/math";
import policy from "discourse/rich-editor/plugins/policy";
import poll from "discourse/rich-editor/plugins/poll";
import spoiler from "discourse/rich-editor/plugins/spoiler";

export var PLUGIN_EXTENSIONS = {
  checklist: checklist,
  details: details,
  "local-dates": localDates,
  math: math,
  policy: policy,
  footnote: footnote,
  poll: poll,
  spoiler: spoiler,
};

// Each named plugin's extension, once.
export function registerPluginExtensions(names) {
  var registered = getExtensions();
  Object.keys(PLUGIN_EXTENSIONS).forEach(function (name) {
    var extension = PLUGIN_EXTENSIONS[name];
    if ((names || []).indexOf(name) !== -1 && registered.indexOf(extension) === -1) {
      registerRichEditorExtension(extension);
    }
  });
}
