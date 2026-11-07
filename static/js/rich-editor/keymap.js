// The editor's keys (static/prosemirror/core/keymap.js): the host's keymap,
// each extension's keymap chained before it, then undo and redo,
// Backspace's input rule undo and block unsetting, Escape, and with the
// default schema Shift-Enter, Enter's double-space hard break and list
// splitting, and Mod-Shift-_ for a horizontal rule. baseKeymap goes after
// this one (editor.js).
import {
  chainCommands,
  exitCode,
  joinTextblockBackward,
  selectParentNode,
  setBlockType,
} from "prosemirror-commands";
import { redo, undo } from "prosemirror-history";
import { undoInputRule } from "prosemirror-inputrules";
import { splitListItem } from "prosemirror-schema-list";
import { atBlockStart, inNode } from "discourse/rich-editor/plugin-utils";

var BACKSPACE_UNSET_NODES = ["heading", "code_block"];

var isMac = typeof navigator !== "undefined" ? /Mac|iP(hone|[oa]d)/.test(navigator.platform) : false;

export function buildKeymap(extensions, initialKeymap, params, includeDefault) {
  if (includeDefault === undefined) {
    includeDefault = true;
  }
  var keys = Object.assign({}, initialKeymap);
  var schema = params.schema;

  var extensionKeys = extractKeymap(extensions, params);
  Object.keys(extensionKeys).forEach(function (key) {
    var command = extensionKeys[key];
    keys[key] = keys[key] ? chainCommands(command, keys[key]) : command;
  });

  function chainWithExisting(key) {
    var commands = Array.prototype.slice.call(arguments, 1);
    if (keys[key]) {
      keys[key] = chainCommands.apply(null, [keys[key]].concat(commands));
    } else {
      keys[key] = chainCommands.apply(null, commands);
    }
  }

  chainWithExisting("Mod-z", undo);
  chainWithExisting("Shift-Mod-z", redo);

  function backspaceUnset(state, dispatch, view) {
    var $pos = atBlockStart(state, view);
    if ($pos && BACKSPACE_UNSET_NODES.indexOf($pos.parent.type.name) >= 0) {
      return setBlockType(schema.nodes.paragraph)(state, dispatch, view);
    }
    return false;
  }

  // The hashtag extension's (196r2) nodes are looked up again once edited.
  function unsetHashtags(state) {
    var $from = state.selection.$from;

    if ($from.parent.type.name === "paragraph") {
      $from.parent.descendants(function (node) {
        if (node.type.name !== "hashtag") {
          return;
        }
        node.attrs.processed = false;
      });
    }
    return false;
  }

  chainWithExisting("Backspace", undoInputRule, backspaceUnset, unsetHashtags, joinTextblockBackward);

  if (!isMac) {
    chainWithExisting("Mod-y", redo);
  }

  chainWithExisting("Escape", selectParentNode);

  // The keys above are always there.
  if (!includeDefault) {
    return keys;
  }

  chainWithExisting("Shift-Enter", exitCode, function (state, dispatch) {
    if (dispatch) {
      dispatch(state.tr.replaceSelectionWith(schema.nodes.hard_break.create()).scrollIntoView());
    }
    return true;
  });

  function doubleSpaceHardBreak(state, dispatch) {
    var $from = state.selection.$from;
    if ($from.parent.type.spec.code || inNode(state, schema.nodes.code_block)) {
      return false;
    }

    if ($from.nodeBefore && $from.nodeBefore.isText && $from.nodeBefore.text.endsWith("  ")) {
      if (dispatch) {
        var tr = state.tr.replaceRangeWith($from.pos - 2, $from.pos, schema.nodes.hard_break.create());
        dispatch(tr.scrollIntoView());
      }
      return true;
    }
    return false;
  }

  chainWithExisting("Enter", doubleSpaceHardBreak, unsetHashtags, splitListItem(schema.nodes.list_item));

  chainWithExisting("Mod-Shift-_", function (state, dispatch) {
    if (dispatch) {
      dispatch(state.tr.replaceSelectionWith(schema.nodes.horizontal_rule.create()).scrollIntoView());
    }
    return true;
  });

  return keys;
}

function extractKeymap(extensions, params) {
  var combined = {};
  extensions.forEach(function (extension) {
    var keymap = extension.keymap instanceof Function ? extension.keymap(params) : extension.keymap;
    if (keymap) {
      Object.assign(combined, keymap);
    }
  });
  return combined;
}
