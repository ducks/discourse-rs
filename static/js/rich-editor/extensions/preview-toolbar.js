// extensions/preview-toolbar.js: a toolbar for the preview block
// (node-views/preview.js) selected as a node or whose source the caret is
// in: the block type's previewControls, then show source or show preview.
// It floats in the block's top right corner, inside it (the image
// toolbar's placing), or without room (capabilities.viewport.sm) takes the
// composer toolbar's place. Tab from the selected block moves into it.
import { NodeSelection } from "prosemirror-state";
import { inlineMenu } from "discourse/rich-editor/node-views/image";
import { previewNodeViewFor, TOOLBAR_IDENTIFIER } from "discourse/rich-editor/node-views/preview";
import { renderToolbarButtons, rovingButtonBar, ToolbarBase } from "discourse/rich-editor/toolbar";

var MENU_PADDING = 8;

// The preview block a selection acts on: one selected as a node, or the one
// whose source the caret is in. { node, pos } or null.
export function activePreviewBlock(state) {
  var selection = state.selection;
  var previewSource = state.schema.nodes.preview_source;

  if (!previewSource) {
    return null;
  }

  if (selection instanceof NodeSelection && selection.node.firstChild && selection.node.firstChild.type === previewSource) {
    return { node: selection.node, pos: selection.from };
  }

  var $head = selection.$head;

  for (var depth = $head.depth; depth > 0; depth--) {
    var node = $head.node(depth);

    if (node.firstChild && node.firstChild.type === previewSource) {
      return { node: node, pos: $head.before(depth) };
    }
  }

  return null;
}

function buildPreviewToolbar(opts, i18n) {
  var toolbar = new ToolbarBase({ i18n: i18n });

  opts.controls.forEach(function (control) {
    toolbar.addButton(
      Object.assign({}, control, {
        action: function () {
          opts.runControl(control);
        },
      })
    );
  });

  toolbar.addButton({
    id: "preview-show-source",
    icon: "code",
    title: "composer.preview_node.show_source",
    className: "composer-preview-toolbar__show-source",
    condition: function () {
      return !opts.isShowingSource();
    },
    action: opts.toggleSource,
  });

  toolbar.addButton({
    id: "preview-show-preview",
    icon: "eye",
    title: "composer.preview_node.show_preview",
    className: "composer-preview-toolbar__show-preview",
    condition: function () {
      return opts.isShowingSource();
    },
    action: opts.toggleSource,
  });

  return toolbar;
}

function PreviewToolbarPluginView(params) {
  this.getContext = params.getContext;
  this.menuInstance = null;
  this.menuTrigger = null;
  this.replacedToolbar = null;
  this.toolbars = new Map();
  this.toolbar = null;
  this.pos = null;
  this.view = null;
}

PreviewToolbarPluginView.prototype.update = function (view) {
  this.view = view;

  var active = activePreviewBlock(view.state);

  if (!active) {
    // Not while a toolbar button has the focus.
    if (!(this.menuInstance && this.menuInstance.content && this.menuInstance.content.contains(document.activeElement))) {
      this.resetToolbar();
    }
    return;
  }

  this.pos = active.pos;
  this.toolbar = this.toolbarFor(active.node.type);
  this.displayToolbar();
  this.redraw();
};

PreviewToolbarPluginView.prototype.focusToolbar = function () {
  var focusable =
    this.menuInstance &&
    this.menuInstance.content &&
    this.menuInstance.content.querySelector('button, a, [tabindex]:not([tabindex="-1"])');

  if (!focusable) {
    return false;
  }

  focusable.focus();
  return true;
};

PreviewToolbarPluginView.prototype.destroy = function () {
  this.resetToolbar();
  this.toolbars.clear();
  this.toolbar = null;
};

PreviewToolbarPluginView.prototype.resetToolbar = function () {
  if (this.menuInstance) {
    this.menuInstance.destroy();
  }
  this.menuInstance = null;
  this.menuTrigger = null;

  if (this.replacedToolbar) {
    this.getContext().replaceToolbar(null, this.replacedToolbar);
    this.replacedToolbar = null;
  }
};

// One toolbar per node type: its buttons are fixed and find their target
// from the selection when they run.
PreviewToolbarPluginView.prototype.toolbarFor = function (type) {
  var self = this;
  var toolbar = this.toolbars.get(type.name);

  if (!toolbar) {
    toolbar = buildPreviewToolbar(
      {
        controls: type.spec.previewControls || [],
        runControl: function (control) {
          self.runControl(control);
        },
        toggleSource: function () {
          var nodeView = self.nodeView();
          if (nodeView) {
            nodeView.toggleSource();
          }
        },
        isShowingSource: function () {
          var nodeView = self.nodeView();
          return !!(nodeView && nodeView.showingSource);
        },
      },
      this.getContext().i18n
    );

    toolbar.rovingButtonBar = function (event) {
      if (event.key === "Tab") {
        event.preventDefault();
        self.view.focus();
        return false;
      }
      return rovingButtonBar(event);
    };
    this.toolbars.set(type.name, toolbar);
  }

  return toolbar;
};

PreviewToolbarPluginView.prototype.nodeView = function () {
  var dom = this.view.nodeDOM(this.pos);
  return dom instanceof HTMLElement ? previewNodeViewFor(dom) : undefined;
};

PreviewToolbarPluginView.prototype.runControl = function (control) {
  var active = activePreviewBlock(this.view.state);

  if (!active) {
    return;
  }

  control.action({ node: active.node, view: this.view, context: this.getContext() });
};

PreviewToolbarPluginView.prototype.displayToolbar = function () {
  var context = this.getContext();
  var viewport = (context.capabilities || {}).viewport || {};
  if (viewport.sm || typeof context.replaceToolbar !== "function") {
    this.showFloatingToolbar();
  } else if (this.replacedToolbar !== this.toolbar) {
    // replacing again on every transaction would draw the whole bar again
    // for each key typed into the source
    context.replaceToolbar(this.toolbar);
    this.replacedToolbar = this.toolbar;
  }
};

// The buttons follow the block's face (show source or show preview).
PreviewToolbarPluginView.prototype.redraw = function () {
  if (this.menuInstance && this.menuInstance.inner) {
    renderToolbarButtons(this.menuInstance.inner, this.toolbar);
  }
};

PreviewToolbarPluginView.prototype.showFloatingToolbar = function () {
  var self = this;
  var trigger = this.view.nodeDOM(this.pos);

  if (!(trigger instanceof HTMLElement) || this.menuTrigger === trigger) {
    return;
  }

  if (this.menuInstance) {
    this.menuInstance.destroy();
  }
  this.menuTrigger = trigger;

  var menu = inlineMenu(trigger, trigger, {
    identifier: TOOLBAR_IDENTIFIER,
    placement: "top-end",
    fallbackPlacements: ["top-end"],
    padding: MENU_PADDING,
    render: function (inner) {
      menu.inner = inner;
      renderToolbarButtons(inner, self.toolbar);
    },
    offset: function (state) {
      return { mainAxis: -MENU_PADDING - state.rects.floating.height, crossAxis: -MENU_PADDING };
    },
    limitShift: {
      offset: function (state) {
        return {
          crossAxis: Math.min(state.rects.floating.height + 2 * MENU_PADDING, state.rects.reference.height - MENU_PADDING),
        };
      },
    },
  });
  this.menuInstance = menu;
  menu.show();
};

export default {
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var pluginView;

    return new Plugin({
      props: {
        handleKeyDown: function (view, event) {
          if (event.key !== "Tab" || event.shiftKey) {
            return false;
          }

          var selection = view.state.selection;
          var previewSource = view.state.schema.nodes.preview_source;

          // only from the selected block: inside the source, Tab is typing
          if (
            !previewSource ||
            !(selection instanceof NodeSelection) ||
            !selection.node.firstChild ||
            selection.node.firstChild.type !== previewSource
          ) {
            return false;
          }

          // this editor's own toolbar, so a second editor cannot take the
          // focus
          if (!pluginView || !pluginView.focusToolbar()) {
            return false;
          }

          event.preventDefault();
          return true;
        },
      },

      view: function () {
        pluginView = new PreviewToolbarPluginView(params);
        return pluginView;
      },
    });
  },
};
