// components/composer/preview-node-view.gjs in GlimmerNodeView's shell
// (lib/glimmer-node-view.js), with lib/composer/preview-block.js: a block
// whose preview_source child is shown rendered in its place, the source a
// toggle away (the preview toolbar's, extensions/preview-toolbar.js).
//
// The wrapper must be `atom` and `isolating`, or the editor will step into
// source it is not showing and merge neighboring text into it:
//
//   nodeSpec: { my_node: { content: "preview_source", atom: true,
//                          isolating: true, previewControls: [...] } },
//   nodeViews: { my_node: previewNodeView(render) }
//
// render(element, node, source) draws the preview into element; controls
// get { node, view, context }.

// The identifier of the menu the toolbar for a preview block is shown in.
export var TOOLBAR_IDENTIFIER = "composer-preview-toolbar";

var nodeViews = new WeakMap();

// The view rendering the block at `dom`.
export function previewNodeViewFor(dom) {
  return nodeViews.get(dom);
}

export function PreviewNodeView(node, view, getPos, params, render) {
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.params = params;
  this.renderPreview = render;

  this.dom = document.createElement("div");
  this.dom.classList.add("composer-" + node.type.name + "-node", "composer-preview-node");
  this.contentDOM = document.createElement("div");
  this.contentDOM.classList.add("composer-preview-node__source");
  this.dom.appendChild(this.contentDOM);

  this.preview = document.createElement("div");
  this.preview.className = "composer-preview-node__preview";
  this.preview.contentEditable = "false";
  this.dom.appendChild(this.preview);

  // the node the preview last rendered, so it holds still while the source
  // is edited and is not laid out again over a source that has not changed
  this.renderedNode = null;
  this.showingSource = !node.textContent;

  this.syncMode();
  this.syncRendered();
  nodeViews.set(this.dom, this);
}

PreviewNodeView.prototype.toggleSource = function () {
  this.showingSource = !this.showingSource;
  this.syncMode();
  this.syncRendered();

  var pos = this.getPos();
  if (pos === undefined) {
    return;
  }

  var view = this.view;
  var pmState = this.params.pmState;
  var tr = view.state.tr;

  tr.setSelection(
    this.showingSource
      ? // just inside the source, past the wrapper and the source's own start
        pmState.TextSelection.create(tr.doc, pos + 2 + this.node.textContent.length)
      : pmState.NodeSelection.create(tr.doc, pos)
  );

  view.dispatch(tr);
  view.focus();
};

PreviewNodeView.prototype.update = function (node) {
  if (node.type !== this.node.type) {
    return false;
  }
  this.node = node;
  this.syncRendered();
  return true;
};

PreviewNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

PreviewNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

// the toolbar is portaled into this node view, so its clicks are not
// document clicks
PreviewNodeView.prototype.stopEvent = function (event) {
  return event.target instanceof Node && !!(event.target.closest && event.target.closest('[data-identifier="' + TOOLBAR_IDENTIFIER + '"]'));
};

// The preview's drawing and the toolbar in the block are no change to the
// document: only the source's are.
PreviewNodeView.prototype.ignoreMutation = function (mutation) {
  if (mutation.type === "selection") {
    return false;
  }
  return !this.contentDOM.contains(mutation.target);
};

PreviewNodeView.prototype.destroy = function () {
  nodeViews.delete(this.dom);
};

PreviewNodeView.prototype.syncMode = function () {
  this.dom.classList.toggle("--source", this.showingSource);
  this.preview.setAttribute("aria-hidden", this.showingSource ? "true" : "false");
};

PreviewNodeView.prototype.syncRendered = function () {
  if (this.showingSource || this.renderedNode === this.node) {
    return;
  }
  if (this.renderedNode && this.renderedNode.eq(this.node)) {
    this.renderedNode = this.node;
    return;
  }
  this.renderedNode = this.node;
  this.preview.replaceChildren();
  this.renderPreview(this.preview, this.node, this.node.textContent);
};

// The nodeViews entry for a preview block drawn by render(element, node,
// source).
export function previewNodeView(render) {
  return function (params) {
    return function (node, view, getPos) {
      return new PreviewNodeView(node, view, getPos, params, render);
    };
  };
}

// previewSourceNode: a preview block's source, trimmed of the padding
// cooked markup puts around it, highlighted as `params`.
export function previewSourceNode(schema, source, params) {
  var trimmed = source.trim();
  return schema.nodes.preview_source.create({ params: params || "" }, trimmed ? schema.text(trimmed) : null);
}

// selectPreviewSource: the caret in the source of the preview block a
// transaction just wrote at `from` (fitting it in can move it past
// `from`, so the first preview_source at or after it).
export function selectPreviewSource(tr, TextSelection, from) {
  var pos;

  tr.doc.nodesBetween(from, tr.doc.content.size, function (node, nodePos) {
    if (pos !== undefined) {
      return false;
    }

    if (node.type.name === "preview_source") {
      pos = nodePos + 1;
      return false;
    }
  });

  return pos === undefined ? tr : tr.setSelection(TextSelection.create(tr.doc, pos)).scrollIntoView();
}
