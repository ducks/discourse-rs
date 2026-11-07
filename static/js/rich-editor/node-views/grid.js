// components/grid-node-view.gjs in glimmer-node-view.js's wrapper (with
// content): the grid's images, its mode buttons at the top end and its
// remove button at the bottom end, both persistent menus in the node's DOM
// (inlineMenu, as portalOutletElement), the controls' height kept as the
// grid's minimum block size so a short grid fits them.
import { iconHtml } from "discourse/rich-editor/toolbar";
import { inlineMenu } from "discourse/rich-editor/node-views/image";

var MENU_PADDING = 8;
var MODE_MENU_IDENTIFIER = "composer-image-grid-mode";
var REMOVE_MENU_IDENTIFIER = "composer-image-grid-remove";

var guid = 0;

export function GridNodeView(node, view, getPos, params) {
  var context = params.getContext();
  this.i18n =
    context.i18n ||
    function (key) {
      return key;
    };
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.dom = document.createElement("div");
  this.dom.classList.add("composer-grid-node", "composer-image-grid");
  this.contentDOM = document.createElement("div");
  this.contentDOM.classList.add("composer-image-grid__content");
  this.dom.appendChild(this.contentDOM);
  this.showMenus("grid-" + ++guid);
}

GridNodeView.prototype.renderModeButtons = function (inner) {
  var self = this;
  var i18n = this.i18n;
  inner.replaceChildren();
  var group = document.createElement("div");
  group.className = "composer-image-gallery__mode-buttons";
  group.contentEditable = "false";
  group.setAttribute("role", "group");
  [
    ["grid", "composer.grid_mode_grid", "table-cells"],
    ["carousel", "composer.grid_mode_carousel", "image"],
  ].forEach(function (entry) {
    var mode = entry[0];
    var active = self.node.attrs.mode === mode;
    var button = document.createElement("button");
    button.setAttribute("aria-label", i18n(entry[1]));
    button.setAttribute("aria-pressed", active ? "true" : "false");
    button.className = "composer-image-gallery__mode-btn" + (active ? " is-active" : "");
    button.dataset.mode = mode;
    button.title = i18n("composer.grid_mode_title", { mode: i18n(entry[1]) });
    button.type = "button";
    button.innerHTML = iconHtml(entry[2]);
    var label = document.createElement("span");
    label.textContent = i18n(entry[1]);
    button.appendChild(label);
    button.addEventListener("click", function (event) {
      self.setMode(mode, event);
    });
    group.appendChild(button);
  });
  inner.appendChild(group);
};

GridNodeView.prototype.renderRemoveButton = function (inner) {
  var self = this;
  var button = document.createElement("button");
  button.className = "composer-image-grid__remove-btn";
  button.contentEditable = "false";
  button.title = this.i18n("composer.remove_grid");
  button.type = "button";
  var label = document.createElement("span");
  label.textContent = this.i18n("composer.remove_grid");
  button.appendChild(label);
  button.addEventListener("click", function (event) {
    self.removeGrid(event);
  });
  inner.appendChild(button);
};

GridNodeView.prototype.setMode = function (mode, event) {
  event.preventDefault();
  var pos = this.getPos();
  this.view.dispatch(this.view.state.tr.setNodeMarkup(pos, null, Object.assign({}, this.node.attrs, { mode: mode })));
};

GridNodeView.prototype.removeGrid = function (event) {
  event.preventDefault();
  event.stopPropagation();
  var pos = this.getPos();
  var node = this.view.state.doc.nodeAt(pos);
  var tr = this.view.state.tr;
  tr.replaceWith(pos, pos + node.nodeSize, node.content);
  this.view.dispatch(tr);
};

GridNodeView.prototype.showMenus = function (menuId) {
  var self = this;
  this.modeMenu = inlineMenu(this.dom, this.dom, {
    identifier: MODE_MENU_IDENTIFIER + "-" + menuId,
    contentRole: "none",
    placement: "top-end",
    fallbackPlacements: ["top-end"],
    padding: MENU_PADDING,
    render: function (inner) {
      self.renderModeButtons(inner);
    },
    offset: function (state) {
      return { mainAxis: -MENU_PADDING - state.rects.floating.height, crossAxis: -MENU_PADDING };
    },
    onPositioned: function () {
      self.updateControlsMinBlockSize();
    },
    limitShift: {
      offset: function (state) {
        var removeButtonHeight = (self.removeMenu && self.removeMenu.content && self.removeMenu.content.offsetHeight) || 0;
        return {
          crossAxis: Math.min(
            state.rects.floating.height + 2 * MENU_PADDING + removeButtonHeight,
            state.rects.reference.height - MENU_PADDING
          ),
        };
      },
    },
  });

  this.removeMenu = inlineMenu(this.dom, this.dom, {
    identifier: REMOVE_MENU_IDENTIFIER + "-" + menuId,
    contentRole: "none",
    placement: "bottom-end",
    fallbackPlacements: ["bottom-end"],
    padding: MENU_PADDING,
    render: function (inner) {
      self.renderRemoveButton(inner);
    },
    offset: function (state) {
      return { mainAxis: -MENU_PADDING - state.rects.floating.height, crossAxis: -MENU_PADDING };
    },
    onPositioned: function () {
      self.updateControlsMinBlockSize();
    },
    limitShift: {
      offset: function (state) {
        var modeButtonsHeight = (self.modeMenu && self.modeMenu.content && self.modeMenu.content.offsetHeight) || 0;
        return { crossAxis: modeButtonsHeight + 2 * MENU_PADDING + state.rects.floating.height };
      },
    },
  });

  this.modeMenu.show();
  this.removeMenu.show();
  this.updateControlsMinBlockSize();
};

GridNodeView.prototype.updateControlsMinBlockSize = function () {
  var modeButtonsHeight = (this.modeMenu && this.modeMenu.content && this.modeMenu.content.offsetHeight) || 0;
  var removeButtonHeight = (this.removeMenu && this.removeMenu.content && this.removeMenu.content.offsetHeight) || 0;
  if (!modeButtonsHeight || !removeButtonHeight) {
    return;
  }
  var minBlockSize = modeButtonsHeight + removeButtonHeight + 3 * MENU_PADDING + "px";
  if (this.dom.style.getPropertyValue("--composer-image-grid-controls-min-block-size") !== minBlockSize) {
    this.dom.style.setProperty("--composer-image-grid-controls-min-block-size", minBlockSize);
  }
};

GridNodeView.prototype.update = function (node) {
  if (node.type !== this.node.type) {
    return false;
  }
  if (node !== this.node) {
    var modeChanged = node.attrs.mode !== this.node.attrs.mode;
    this.node = node;
    if (modeChanged && this.modeMenu.content) {
      this.renderModeButtons(this.modeMenu.content.querySelector(".fk-d-menu__inner-content"));
    }
  }
  return true;
};

GridNodeView.prototype.stopEvent = function (event) {
  return !!(
    (this.modeMenu.content && this.modeMenu.content.contains(event.target)) ||
    (this.removeMenu.content && this.removeMenu.content.contains(event.target))
  );
};

// The images are the editor's; the menus are ours.
GridNodeView.prototype.ignoreMutation = function (mutation) {
  if (mutation.type === "selection") {
    return false;
  }
  return !this.contentDOM.contains(mutation.target);
};

GridNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

GridNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

GridNodeView.prototype.destroy = function () {
  this.modeMenu.destroy();
  this.removeMenu.destroy();
};
