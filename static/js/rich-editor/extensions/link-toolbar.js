// extensions/link-toolbar.js: while the cursor is in a link, a toolbar for
// it: edit, copy, unlink (not an automatic link), load its preview (an
// automatic link a onebox could show) and open it. With room for it
// (capabilities.viewport.sm) it floats under the cursor
// (Discourse.float.menu), else it takes the composer toolbar's place
// (getContext().replaceToolbar).
//
// Edit opens UpsertHyperlink, the composer's link modal, through the
// context's modal service ("upsert-hyperlink"). Not ported: Load preview
// needs the onebox extension (196r3), so it is never offered.
import { NodeSelection, TextSelection } from "prosemirror-state";
import { rovingButtonBar, toastSuccess, ToolbarBase, renderToolbarButtons } from "discourse/rich-editor/toolbar";

var AUTO_LINKS = ["autolink", "linkify"];
var MENU_OFFSET = 12;
var STRIP_PROTOCOLS = /^(mailto:|https:\/\/)/;
var IDENTIFIER = "composer-link-toolbar";

function buildLinkToolbar(opts, i18n) {
  var toolbar = new ToolbarBase({ i18n: i18n });

  toolbar.addButton({
    id: "link-edit",
    icon: "pen",
    title: "composer.link_toolbar.edit",
    className: "composer-link-toolbar__edit",
    action: opts.editLink,
  });

  toolbar.addButton({
    id: "link-copy",
    icon: "copy",
    title: "composer.link_toolbar.copy",
    className: "composer-link-toolbar__copy",
    action: opts.copyLink,
  });

  toolbar.addButton({
    id: "link-unlink",
    icon: "link-slash",
    title: "composer.link_toolbar.remove",
    className: "composer-link-toolbar__unlink",
    condition: opts.canUnlink,
    action: opts.unlinkText,
  });

  toolbar.addButton({
    id: "link-show-preview",
    icon: "expand",
    title: "composer.link_toolbar.load_preview",
    className: "composer-link-toolbar__show-preview",
    condition: opts.canShowPreview,
    action: opts.showPreview,
  });

  toolbar.addSeparator({
    condition: function () {
      return opts.canVisit() || opts.canUnlink() || opts.canShowPreview();
    },
  });

  toolbar.addButton({
    id: "link-visit",
    get icon() {
      return opts.canVisit() ? "up-right-from-square" : null;
    },
    title: "composer.link_toolbar.visit",
    className: "composer-link-toolbar__visit",
    condition: function () {
      return opts.canVisit() || opts.canUnlink();
    },
    get href() {
      return opts.canVisit() ? opts.getHref() : null;
    },
    get translatedLabel() {
      if (opts.canUnlink()) {
        var label = opts.getHref();

        // the site's own links without its origin
        var origin = window.location.origin;
        if (label.startsWith(origin)) {
          return label.replace(origin, "");
        }

        // mailto: and https:// left out
        return label.replace(STRIP_PROTOCOLS, "");
      }
    },
    get disabled() {
      return !opts.canVisit();
    },
  });

  return toolbar;
}

function LinkToolbarPluginView(params) {
  this.utils = params.utils;
  this.getContext = params.getContext;
  this.menuInstance = null;
  this.toolbarReplaced = false;
  this.linkToolbar = null;
  this.linkState = null;
  this.calculatingCoords = false;
  this.view = null;
  this.content = null;
}

LinkToolbarPluginView.prototype.update = function (view) {
  this.view = view;

  var markRange =
    !(view.state.selection instanceof NodeSelection) &&
    this.utils.getMarkRange(view.state.selection.$head, view.state.schema.marks.link);

  if (markRange) {
    this.updateLinkState(markRange);
    this.displayToolbar();
    return;
  }

  // Not while a toolbar button has the focus.
  if (!(this.menuInstance && this.menuInstance.content && this.menuInstance.content.contains(document.activeElement))) {
    this.resetToolbar();
  }
};

LinkToolbarPluginView.prototype.destroy = function () {
  if (this.menuInstance) {
    this.menuInstance.destroy({ focusTrigger: false });
  }
  this.menuInstance = null;
  this.linkToolbar = null;
};

LinkToolbarPluginView.prototype.resetToolbar = function () {
  if (this.menuInstance) {
    this.menuInstance.destroy({ focusTrigger: false });
  }
  this.menuInstance = null;

  if (this.toolbarReplaced) {
    this.getContext().replaceToolbar(null, this.linkToolbar);
    this.toolbarReplaced = false;
  }
};

LinkToolbarPluginView.prototype.updateLinkState = function (markRange) {
  var self = this;
  var attrs = Object.assign({}, markRange.mark.attrs, {
    range: markRange,
    head: this.view.state.selection.head,
  });

  if (!this.linkToolbar) {
    this.linkState = attrs;
    var context = this.getContext();

    this.linkToolbar = buildLinkToolbar(
      {
        editLink: function () {
          self.openLinkEditor();
        },
        copyLink: function () {
          self.copyLink();
        },
        unlinkText: function () {
          self.unlinkText();
        },
        showPreview: function () {},
        canVisit: function () {
          return self.canVisit();
        },
        getHref: function () {
          return self.linkState.href;
        },
        canUnlink: function () {
          return self.canUnlink();
        },
        canShowPreview: function () {
          return false;
        },
      },
      context.i18n
    );

    this.linkToolbar.rovingButtonBar = function (event) {
      return self.rovingButtonBar(event);
    };
  } else {
    Object.assign(this.linkState, attrs);
  }
};

LinkToolbarPluginView.prototype.rovingButtonBar = function (event) {
  if (event.key === "Tab") {
    event.preventDefault();
    this.view.focus();
    return false;
  }
  return rovingButtonBar(event);
};

LinkToolbarPluginView.prototype.displayToolbar = function () {
  var context = this.getContext();
  var viewport = (context.capabilities || {}).viewport || {};
  if (viewport.sm || typeof context.replaceToolbar !== "function") {
    this.showFloatingToolbar();
  } else {
    context.replaceToolbar(this.linkToolbar);
    this.toolbarReplaced = true;
  }
};

LinkToolbarPluginView.prototype.openLinkEditor = function () {
  var self = this;
  var range = this.linkState.range;
  var schema = this.view.state.schema;
  var tempTr = this.view.state.tr.removeMark(range.from, range.to, schema.marks.link);

  var currentLinkText = this.utils.convertToMarkdown(
    schema.topNodeType.create(null, schema.nodes.paragraph.create(null, tempTr.doc.slice(range.from, range.to).content))
  );

  this.getContext().modal.show("upsert-hyperlink", {
    model: {
      editing: true,
      linkText: currentLinkText,
      linkUrl: this.linkState.href,
      toolbarEvent: {
        addText: function (text) {
          self.replaceText(text);
        },
        selected: { value: this.linkState.href },
      },
    },
  });
};

LinkToolbarPluginView.prototype.replaceText = function (text) {
  var content = this.utils.convertFromMarkdown(text).content;
  var range = this.linkState.range;

  if (content.firstChild && content.firstChild.content.size > 0) {
    var state = this.view.state;
    var tr = state.tr.replaceWith(range.from, range.to, content.firstChild.content);

    var newPos = Math.min(this.view.state.selection.from, range.from + content.firstChild.content.size);
    var resolvedPos = tr.doc.resolve(newPos);
    tr.setSelection(new TextSelection(resolvedPos, resolvedPos));
    this.view.dispatch(tr);
    this.view.focus();
  }
};

LinkToolbarPluginView.prototype.copyLink = function () {
  var i18n = this.getContext().i18n;
  navigator.clipboard.writeText(this.linkState.href).then(function () {
    toastSuccess(i18n("composer.link_toolbar.link_copied"));
  });
};

LinkToolbarPluginView.prototype.unlinkText = function () {
  var range = this.view.state.selection.empty ? this.linkState.range : this.view.state.selection;

  if (range) {
    var state = this.view.state;
    this.view.dispatch(state.tr.removeMark(range.from, range.to, state.schema.marks.link));
    this.view.focus();
  }
};

LinkToolbarPluginView.prototype.canVisit = function () {
  return !!this.utils.getLinkify().matchAtStart(this.linkState.href);
};

LinkToolbarPluginView.prototype.canUnlink = function () {
  return AUTO_LINKS.indexOf(this.linkState.markup) === -1;
};

LinkToolbarPluginView.prototype.showFloatingToolbar = function () {
  var self = this;
  var element = this.view.domAtPos(this.linkState.head).node;
  var trigger = element.nodeType === Node.TEXT_NODE ? element.parentElement : element;

  trigger.getBoundingClientRect = function () {
    return self.getTriggerClientRect();
  };

  if (this.menuInstance && this.menuInstance.expanded) {
    this.menuInstance.trigger = trigger;
    renderToolbarButtons(this.content, this.linkToolbar);
    window.Discourse.float.position(trigger, this.menuInstance.content, this.menuInstance.options);
    return;
  }

  if (this.menuInstance) {
    this.menuInstance.destroy({ focusTrigger: false });
  }
  this.content = document.createElement("div");
  // ToolbarButtons draws its buttons straight into the menu.
  this.content.style.display = "contents";
  renderToolbarButtons(this.content, this.linkToolbar);
  var instance = (this.menuInstance = window.Discourse.float.menu(trigger, {
    identifier: IDENTIFIER,
    content: this.content,
    placement: "bottom",
    padding: 0,
    hide: true,
    boundary: this.view.dom.parentElement,
    fallbackPlacements: ["bottom-end", "bottom-start", "top", "top-end", "top-start"],
    closeOnClickOutside: false,
    trapTab: false,
    // Only the open toolbar gives the focus back: one replaced while it
    // animates out would take it from the toolbar that replaced it.
    onClose: function () {
      if (self.menuInstance === instance && self.view && !self.view.isDestroyed) {
        self.view.focus();
      }
    },
  }));
};

LinkToolbarPluginView.prototype.getTriggerClientRect = function () {
  var docView = this.view.docView;
  var head = this.linkState.head;
  var doc = this.view.state.doc;

  if (this.calculatingCoords || !docView || head > doc.content.size) {
    return { left: 0, top: 0, width: 0, height: 0, right: 0, bottom: 0, x: 0, y: 0 };
  }

  this.calculatingCoords = true;
  try {
    var coords = this.view.coordsAtPos(head);
    var top = coords.top + MENU_OFFSET;
    return { left: coords.left, top: top, width: 0, height: 0, right: coords.left, bottom: top, x: coords.left, y: top };
  } finally {
    this.calculatingCoords = false;
  }
};

export default {
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var utils = params.utils;

    return new Plugin({
      props: {
        handleKeyDown: function (view, event) {
          if (event.key !== "Tab" || event.shiftKey) {
            return false;
          }

          var range = utils.getMarkRange(view.state.selection.$head, view.state.schema.marks.link);
          if (!range) {
            return false;
          }

          // The open one, not one still animating closed.
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
        return new LinkToolbarPluginView(params);
      },
    });
  },
};
