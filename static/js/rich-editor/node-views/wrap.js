// components/wrap-node-view.gjs in GlimmerNodeView's shell
// (lib/glimmer-node-view.js): .composer-wrap-node around the content, and
// after it the indicator button that opens WrapAttributesModal
// (getContext().modal) to rename, re-attribute or unwrap it.
import { NodeSelection } from "prosemirror-state";
import { parseAttributesString, serializeAttributes } from "discourse/rich-editor/wrap-utils";

export function WrapNodeView(node, view, getPos, params) {
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.params = params;

  var tag = node.isInline ? "span" : "div";
  this.dom = document.createElement(tag);
  this.dom.classList.add("composer-wrap-node");
  this.contentDOM = document.createElement(tag);
  this.dom.appendChild(this.contentDOM);

  this.button = document.createElement("button");
  this.button.className = "d-wrap-indicator btn-flat " + (node.isInline ? "--inline" : "--block");
  this.button.contentEditable = "false";
  this.button.type = "button";
  this.button.addEventListener("click", this.editAttributes.bind(this));
  this.dom.appendChild(this.button);
  this.render();
}

WrapNodeView.prototype.attributesDisplay = function () {
  var attrs = this.node.attrs.data || {};
  if (Object.keys(attrs).length === 0) {
    return "[wrap]";
  }
  if (attrs.wrap) {
    return "[wrap=" + attrs.wrap + "]";
  }
  return "[wrap]";
};

WrapNodeView.prototype.render = function () {
  this.button.textContent = this.attributesDisplay();
};

WrapNodeView.prototype.editAttributes = function () {
  var self = this;
  this.params.getContext().modal.show("wrap-attributes", {
    model: {
      initialAttributes: serializeAttributes(this.node.attrs.data || {}),
      onApply: function (attrsString) {
        self.updateAttributes(attrsString);
      },
      onRemove: function () {
        self.removeWrap();
      },
    },
  });
};

WrapNodeView.prototype.removeWrap = function () {
  var pos = this.getPos();
  var node = this.node;

  if (node.content.size === 0) {
    this.view.dispatch(this.view.state.tr.delete(pos, pos + node.nodeSize));
  } else {
    var tr = this.view.state.tr;
    tr.replaceWith(pos, pos + node.nodeSize, node.content);
    this.view.dispatch(tr);
  }
};

WrapNodeView.prototype.updateAttributes = function (attrsString) {
  var pos = this.getPos();
  var tr = this.view.state.tr;

  tr.setNodeMarkup(pos, null, { data: parseAttributesString(attrsString) });
  tr.setSelection(NodeSelection.create(tr.doc, pos));
  this.view.dispatch(tr);
};

WrapNodeView.prototype.update = function (node) {
  if (node.type !== this.node.type) {
    return false;
  }
  this.node = node;
  this.render();
  return true;
};

WrapNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

WrapNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

// GlimmerNodeView's, for a component without its own.
WrapNodeView.prototype.stopEvent = function () {
  return false;
};

WrapNodeView.prototype.ignoreMutation = function () {
  return true;
};
