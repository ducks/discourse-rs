// The rich editor (static/prosemirror/components/prosemirror-editor.gts) as
// a plain module: createEditor builds the EditorView in a
// .ProseMirror-container, parses the markdown it is given into it and
// reports the serialized markdown of each of the member's edits.
//
//   createEditor({
//     container,      // element the .ProseMirror-container goes in
//     value,          // the markdown to start from
//     parseTree,      // markdown -> the markdown crate's node tree
//                     // (docs/rich-editor.md); without it only "" opens
//     onChange,       // (markdown) after each edit that can be undone
//     onUnsupported,  // (error) the markdown has what no extension parses;
//                     // the reference alerts composer.unsupported_token
//                     // and toggles back to markdown
//     extensions,     // default: getExtensions()
//     includeDefault, placeholder, disabled, className,
//     keymap,         // the host's { "ctrl+m": fn } keys, as d-editor's
//     focusIn, focusOut, toggleRichEditor, context (getContext extras)
//   })
//
// returns { view, schema, parser, serializer, setValue, getValue,
// convertFromMarkdown, convertToMarkdown, commands, customState, focus,
// setPlaceholder, destroy }.
import * as ProsemirrorCommands from "prosemirror-commands";
import { baseKeymap } from "prosemirror-commands";
import { dropCursor } from "prosemirror-dropcursor";
import { gapCursor } from "prosemirror-gapcursor";
import * as ProsemirrorHistory from "prosemirror-history";
import { history } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import * as ProsemirrorModel from "prosemirror-model";
import * as ProsemirrorSchemaList from "prosemirror-schema-list";
import * as ProsemirrorState from "prosemirror-state";
import { EditorState, Plugin } from "prosemirror-state";
import * as ProsemirrorTransform from "prosemirror-transform";
import * as ProsemirrorView from "prosemirror-view";
import { EditorView } from "prosemirror-view";
import { getExtensions, placeholder } from "discourse/rich-editor/extensions";
import { buildInputRules } from "discourse/rich-editor/inputrules";
import { buildKeymap } from "discourse/rich-editor/keymap";
import Parser, { UnsupportedTokenError } from "discourse/rich-editor/parser";
import * as utils from "discourse/rich-editor/plugin-utils";
import { createSchema } from "discourse/rich-editor/schema";
import Serializer from "discourse/rich-editor/serializer";

export { UnsupportedTokenError };

var AUTOCOMPLETE_KEY_DOWN_SUPPRESS = ["Enter", "Tab", "ArrowDown", "ArrowUp"];

// core/plugin.js extractPlugins: a plugin, a spec, a function of the params
// (which may resolve later: that plugin is added once it does) or a list.
function processPlugin(pluginArg, params, handleAsyncPlugin) {
  if (typeof pluginArg === "function") {
    var ret = pluginArg(params);

    if (ret instanceof Promise) {
      ret.then(function (plugin) {
        handleAsyncPlugin(processPlugin(plugin, params));
      });
      return;
    }

    return processPlugin(ret, params, handleAsyncPlugin);
  }

  if (pluginArg instanceof Array) {
    return pluginArg.map(function (plugin) {
      return processPlugin(plugin, params, handleAsyncPlugin);
    });
  }

  if (pluginArg instanceof Plugin) {
    return pluginArg;
  }

  return new Plugin(pluginArg);
}

function flat(list) {
  return list.reduce(function (all, item) {
    return all.concat(item instanceof Array ? flat(item) : [item]);
  }, []);
}

export function extractPlugins(extensions, params, handleAsyncPlugin) {
  var plugins = [];
  extensions.forEach(function (extension) {
    (extension.plugins ? [].concat(extension.plugins) : []).forEach(function (plugin) {
      plugins.push(processPlugin(plugin, params, handleAsyncPlugin));
    });
  });
  return flat(plugins).filter(Boolean);
}

// core/plugin.js extractNodeViews, less the Glimmer component descriptors:
// a node view class, or a function of the params returning one (or a
// NodeViewConstructor).
export function extractNodeViews(extensions, params) {
  var nodeViews = {};
  extensions.forEach(function (extension) {
    if (!extension.nodeViews) {
      return;
    }
    Object.keys(extension.nodeViews).forEach(function (name) {
      var nodeView = extension.nodeViews[name];
      if (!nodeView.toString().startsWith("class")) {
        nodeView = nodeView(params);
      }
      if (nodeView.toString().startsWith("class")) {
        var NodeView = nodeView;
        nodeViews[name] = function (node, view, getPos, decorations, innerDecorations) {
          return new NodeView(node, view, getPos, decorations, innerDecorations);
        };
      } else {
        nodeViews[name] = nodeView;
      }
    });
  });
  return nodeViews;
}

// core/commands.js
function buildCommands(extensions, params, view) {
  var commands = {};
  extensions.forEach(function (extension) {
    if (!extension.commands) {
      return;
    }
    var built = extension.commands(params);
    Object.keys(built).forEach(function (name) {
      commands[name] = function () {
        view.focus();
        return built[name].apply(null, arguments)(view.state, view.dispatch, view);
      };
    });
  });
  return commands;
}

function buildCustomState(extensions, params) {
  return function (viewState) {
    var states = {};
    extensions.forEach(function (extension) {
      if (!extension.state) {
        return;
      }
      var result = extension.state(params, viewState);
      Object.keys(result).forEach(function (name) {
        states[name] = result[name];
      });
    });
    return states;
  };
}

// keymapFromArgs: d-editor's "shift+tab" style keys as ProseMirror's
// "Shift-Tab"; a host handler returning false (or nothing) handles the key.
function hostKeymap(keys) {
  var replacements = { shift: "Shift", tab: "Tab" };
  var result = {};
  Object.keys(keys || {}).forEach(function (key) {
    var pmKey = key
      .split("+")
      .map(function (word) {
        return replacements[word] || word;
      })
      .join("-");
    result[pmKey] = function () {
      return !(keys[key]() || false);
    };
  });
  return result;
}

function splitNonEmptyLines(text) {
  return text.split(/\r?\n/).filter(function (line) {
    return line.trim().length > 0;
  });
}

function buildListNode(schema, listType, lines) {
  var listItems = lines.map(function (line) {
    return schema.nodes.list_item.create(null, [
      schema.nodes.paragraph.create(null, line.length > 0 ? schema.text(line) : undefined),
    ]);
  });

  if (typeof listType === "string") {
    listType = schema.nodes[listType];
  }

  return listType.create(null, listItems);
}

export function createEditor(options) {
  var given = options.extensions || getExtensions();
  // The placeholder is always there.
  var extensions = given.indexOf(placeholder) >= 0 ? given : [placeholder].concat(given);
  var includeDefault = options.includeDefault;
  var schema = createSchema(extensions, includeDefault);
  var context = { placeholder: options.placeholder };
  var lastSerialized;
  var view;
  var parser;
  var serializer;

  function convertFromMarkdown(markdown) {
    return parser.convert(schema, markdown);
  }

  function convertToMarkdown(doc) {
    return serializer.convert(doc);
  }

  var params = {
    utils: Object.assign({}, utils, {
      convertFromMarkdown: convertFromMarkdown,
      convertToMarkdown: convertToMarkdown,
      splitNonEmptyLines: splitNonEmptyLines,
      buildListNode: buildListNode,
      toggleRichEditor: options.toggleRichEditor,
    }),
    schema: schema,
    pmState: ProsemirrorState,
    pmModel: ProsemirrorModel,
    pmView: ProsemirrorView,
    pmHistory: ProsemirrorHistory,
    pmTransform: ProsemirrorTransform,
    pmSchemaList: ProsemirrorSchemaList,
    pmCommands: ProsemirrorCommands,
    getContext: function () {
      return Object.assign({}, options.context, context);
    },
  };

  function handleAsyncPlugin(plugin) {
    view.updateState(view.state.reconfigure({ plugins: view.state.plugins.concat(plugin) }));
  }

  var plugins = [
    buildInputRules(extensions, params, includeDefault),
    keymap(buildKeymap(extensions, hostKeymap(options.keymap), params, includeDefault)),
    keymap(baseKeymap),
    dropCursor({ color: "var(--tertiary-high)", width: 4 }),
    gapCursor(),
    history(),
  ].concat(extractPlugins(extensions, params, handleAsyncPlugin));

  parser = new Parser(extensions, params, includeDefault, options.parseTree);
  serializer = new Serializer(extensions, params, includeDefault);

  var container = document.createElement("div");
  container.className = "ProseMirror-container";
  options.container.appendChild(container);

  view = new EditorView(container, {
    state: EditorState.create({ schema: schema, plugins: plugins }),
    nodeViews: extractNodeViews(extensions, params),
    attributes: { class: options.className || "" },
    editable: function () {
      return options.disabled !== true;
    },
    dispatchTransaction: function (tr) {
      view.updateState(view.state.apply(tr));

      // Serializing rewrites the whole document, so only the member's own
      // edits, the ones they can undo, replace the value.
      if (tr.docChanged && tr.getMeta("addToHistory") !== false) {
        var value = convertToMarkdown(view.state.doc);
        lastSerialized = value;
        if (options.onChange) {
          options.onChange(value);
        }
      }
    },
    handleDOMEvents: {
      focus: function () {
        if (options.focusIn) {
          options.focusIn();
        }
        return false;
      },
      blur: function () {
        if (options.focusOut) {
          setTimeout(options.focusOut, 0);
        }
        return false;
      },
    },
    handleKeyDown: function (_, event) {
      // The autocomplete's keys while it is open.
      return AUTOCOMPLETE_KEY_DOWN_SUPPRESS.indexOf(event.key) >= 0 && !!document.querySelector(".autocomplete");
    },
  });

  // convertFromValue: the markdown it was given, unless it is what the
  // editor just wrote. Not an edit to undo.
  function setValue(value) {
    value = value || "";
    if (value === lastSerialized) {
      return true;
    }
    var doc;
    try {
      doc = value === "" && !options.parseTree ? schema.topNodeType.createAndFill() : convertFromMarkdown(value);
    } catch (e) {
      if (e instanceof UnsupportedTokenError && options.onUnsupported) {
        options.onUnsupported(e);
        return false;
      }
      throw e;
    }
    var tr = view.state.tr;
    tr.replaceWith(0, view.state.doc.content.size, doc.content).setMeta("addToHistory", false);
    view.updateState(view.state.apply(tr));
    return true;
  }

  var opened = setValue(options.value);

  return {
    view: view,
    schema: schema,
    parser: parser,
    serializer: serializer,
    params: params,
    opened: opened,
    commands: buildCommands(extensions, params, view),
    customState: buildCustomState(extensions, params),
    convertFromMarkdown: convertFromMarkdown,
    convertToMarkdown: convertToMarkdown,
    setValue: setValue,
    getValue: function () {
      return convertToMarkdown(view.state.doc);
    },
    focus: function () {
      view.focus();
    },
    // updateContext: a placeholder change reaches its plugin.
    setPlaceholder: function (value) {
      context.placeholder = value;
      view.dispatch(
        view.state.tr.setMeta("addToHistory", false).setMeta("discourseContextChanged", { key: "placeholder", value: value })
      );
    },
    destroy: function () {
      view.destroy();
      container.remove();
    },
  };
}
