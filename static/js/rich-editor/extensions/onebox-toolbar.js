// extensions/onebox-toolbar.js: while a onebox (or an inline onebox) is
// the selected node, a toolbar for it: copy its url, remove the preview
// (back to an automatic link) and open it. A full onebox's floats in its
// top right corner, inside it (the image toolbar's placing); an inline
// onebox's under it, as the link toolbar's. Without room
// (capabilities.viewport.sm) it takes the composer toolbar's place
// (getContext().replaceToolbar). Tab moves into it.
import { NodeSelection, TextSelection } from "prosemirror-state";
import { inlineMenu } from "discourse/rich-editor/node-views/image";
import { renderToolbarButtons, rovingButtonBar, toastSuccess, ToolbarBase } from "discourse/rich-editor/toolbar";

var MENU_PADDING = 8;
var IDENTIFIER = "composer-onebox-toolbar";

function buildOneboxToolbar(opts, i18n) {
  var toolbar = new ToolbarBase({ i18n: i18n });

  toolbar.addButton({
    id: "onebox-copy",
    icon: "copy",
    title: "composer.onebox_toolbar.copy",
    className: "composer-onebox-toolbar__copy",
    action: opts.copyLink,
  });

  toolbar.addButton({
    id: "onebox-remove-preview",
    icon: "compress",
    title: "composer.onebox_toolbar.remove_preview",
    className: "composer-onebox-toolbar__remove-preview",
    action: opts.removePreview,
  });

  toolbar.addSeparator({});

  toolbar.addButton({
    id: "onebox-visit",
    icon: "up-right-from-square",
    title: "composer.onebox_toolbar.visit",
    className: "composer-onebox-toolbar__visit",
    get href() {
      return opts.getUrl();
    },
  });

  return toolbar;
}

function OneboxToolbarPluginView(params) {
  this.getContext = params.getContext;
  this.menuInstance = null;
  this.menuType = null;
  this.toolbarReplaced = false;
  this.toolbar = null;
  this.state = null;
  this.view = null;
  this.content = null;
}

OneboxToolbarPluginView.prototype.update = function (view) {
  this.view = view;
  var selection = view.state.selection;

  if (!(selection instanceof NodeSelection)) {
    this.resetToolbar();
    return;
  }

  var node = selection.node;
  if (node.type.name !== "onebox" && node.type.name !== "onebox_inline") {
    this.resetToolbar();
    return;
  }

  this.updateState(node, selection);
  this.displayToolbar();
};

OneboxToolbarPluginView.prototype.destroy = function () {
  if (this.menuInstance) {
    this.menuInstance.destroy({ focusTrigger: false });
  }
  this.menuInstance = null;
  this.toolbar = null;
};

OneboxToolbarPluginView.prototype.resetToolbar = function () {
  if (this.menuInstance) {
    this.menuInstance.destroy({ focusTrigger: false });
  }
  this.menuInstance = null;

  if (this.toolbarReplaced) {
    this.getContext().replaceToolbar(null, this.toolbar);
    this.toolbarReplaced = false;
  }
};

OneboxToolbarPluginView.prototype.updateState = function (node, selection) {
  var self = this;
  var attrs = { url: node.attrs.url, type: node.type.name, from: selection.from, to: selection.to };

  if (!this.toolbar) {
    this.state = attrs;
    this.toolbar = buildOneboxToolbar(
      {
        copyLink: function () {
          self.copyLink();
        },
        removePreview: function () {
          self.removePreview();
        },
        getUrl: function () {
          return self.state.url;
        },
      },
      this.getContext().i18n
    );
    this.toolbar.rovingButtonBar = function (event) {
      if (event.key === "Tab") {
        event.preventDefault();
        self.view.focus();
        return false;
      }
      return rovingButtonBar(event);
    };
  } else {
    Object.assign(this.state, attrs);
  }
};

OneboxToolbarPluginView.prototype.displayToolbar = function () {
  var context = this.getContext();
  var viewport = (context.capabilities || {}).viewport || {};
  if (viewport.sm || typeof context.replaceToolbar !== "function") {
    this.showFloatingToolbar();
  } else {
    context.replaceToolbar(this.toolbar);
    this.toolbarReplaced = true;
  }
};

OneboxToolbarPluginView.prototype.copyLink = function () {
  var i18n = this.getContext().i18n;
  return navigator.clipboard.writeText(this.state.url).then(function () {
    toastSuccess(i18n("composer.onebox_toolbar.link_copied"));
  });
};

OneboxToolbarPluginView.prototype.removePreview = function () {
  var state = this.view.state;
  var selection = state.selection;
  var node = selection.node;
  var url = node.attrs.url;
  var href = /^https?:\/\//.test(url) ? url : "https://" + url;
  var isBlock = node.type.name === "onebox";

  var linkMark = state.schema.marks.link.create({ href: href, markup: "autolink" });
  var textNode = state.schema.text(href, [linkMark]);

  var content = isBlock ? state.schema.nodes.paragraph.create(null, textNode) : textNode;
  var tr = state.tr.replaceWith(selection.from, selection.to, content);

  // The cursor at the end of the new text
  var cursorPos = isBlock ? selection.from + 1 + href.length : selection.from + href.length;
  tr.setSelection(TextSelection.create(tr.doc, Math.min(cursorPos, tr.doc.content.size)));

  this.view.dispatch(tr);
  this.view.focus();
};

OneboxToolbarPluginView.prototype.getNodeDOM = function () {
  var from = this.state.from;

  // nodeDOM works for inline nodes, domAtPos for block nodes
  var nodeDOM = this.view.nodeDOM(from);
  if (nodeDOM) {
    return nodeDOM;
  }

  var at = this.view.domAtPos(from);
  if (at.node.nodeType === Node.ELEMENT_NODE) {
    return at.node.childNodes[at.offset];
  }
  return at.node.parentElement;
};

OneboxToolbarPluginView.prototype.showFloatingToolbar = function () {
  var trigger = this.getNodeDOM();
  if (!trigger) {
    return;
  }

  var type = this.state.type;

  // The same type's menu stays.
  if (this.menuInstance && this.menuInstance.expanded && this.menuType === type) {
    this.menuInstance.trigger = trigger;
    return;
  }

  // A switch between block and inline draws it again.
  if (this.menuInstance) {
    this.menuInstance.destroy({ focusTrigger: false });
  }
  this.menuInstance = null;
  this.menuType = type;

  if (type === "onebox") {
    this.showBlockToolbar(trigger);
  } else {
    this.showInlineToolbar(trigger);
  }
};

// The image toolbar's placing, inside the onebox, top right.
OneboxToolbarPluginView.prototype.showBlockToolbar = function (trigger) {
  var self = this;
  this.menuInstance = inlineMenu(trigger, trigger, {
    identifier: IDENTIFIER,
    placement: "top-end",
    fallbackPlacements: ["top-end"],
    padding: MENU_PADDING,
    render: function (inner) {
      self.content = inner;
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
  this.menuInstance.show();
};

// The link toolbar's placing, under the inline onebox.
OneboxToolbarPluginView.prototype.showInlineToolbar = function (trigger) {
  var self = this;
  var content = document.createElement("div");
  content.style.display = "contents";
  renderToolbarButtons(content, this.toolbar);
  var instance = (this.menuInstance = window.Discourse.float.menu(trigger, {
    identifier: IDENTIFIER,
    content: content,
    placement: "bottom",
    padding: 0,
    hide: true,
    boundary: this.view.dom.parentElement,
    fallbackPlacements: ["bottom-end", "bottom-start", "top", "top-end", "top-start"],
    closeOnClickOutside: false,
    trapTab: false,
    onClose: function () {
      if (self.menuInstance === instance && self.view && !self.view.isDestroyed) {
        self.view.focus();
      }
    },
  }));
};

export default {
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;

    return new Plugin({
      props: {
        handleKeyDown: function (view, event) {
          if (event.key !== "Tab" || event.shiftKey) {
            return false;
          }

          var selection = view.state.selection;
          if (!(selection instanceof NodeSelection)) {
            return false;
          }

          var node = selection.node;
          if (node.type.name !== "onebox" && node.type.name !== "onebox_inline") {
            return false;
          }

          var activeMenu = document.querySelector('[data-identifier="' + IDENTIFIER + '"]:not(.-closing)');
          if (!activeMenu) {
            return false;
          }

          event.preventDefault();

          var focusable = activeMenu.querySelector('button, a, [tabindex]:not([tabindex="-1"]), .select-kit');

          if (focusable) {
            focusable.focus();
            return true;
          }

          return false;
        },
      },

      view: function () {
        return new OneboxToolbarPluginView(params);
      },
    });
  },
};
