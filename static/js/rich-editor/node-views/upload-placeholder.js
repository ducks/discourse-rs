// components/upload-placeholder-node-view.gjs in glimmer-node-view.js's
// wrapper: a file being uploaded, with its name, its progress and a
// cancel button.
import { iconHtml } from "discourse/rich-editor/toolbar";

export function UploadPlaceholderNodeView(node, view, getPos, params) {
  var self = this;
  var context = params.getContext();
  var i18n =
    context.i18n ||
    function (key) {
      return key;
    };
  this.node = node;
  this.appEvents = context.appEvents;
  this.dom = document.createElement("span");
  this.dom.classList.add("composer-upload_placeholder-node", "upload-placeholder", "--file");
  this.dom.dataset.uploadId = node.attrs.fileId;

  var icon = document.createElement("span");
  icon.className = "upload-placeholder__icon";
  icon.innerHTML = iconHtml("file");
  this.filename = document.createElement("span");
  this.filename.className = "upload-placeholder__filename";
  this.filename.textContent = node.attrs.filename;
  this.progress = document.createElement("span");
  this.progress.className = "upload-placeholder__progress";
  this.progress.textContent = "0%";
  var cancel = document.createElement("button");
  cancel.setAttribute("aria-label", i18n("cancel"));
  cancel.className = "upload-placeholder__cancel btn-transparent no-text";
  cancel.contentEditable = "false";
  cancel.title = i18n("cancel");
  cancel.innerHTML = iconHtml("xmark");
  cancel.addEventListener("click", function (event) {
    event.preventDefault();
    event.stopPropagation();
    if (self.appEvents) {
      self.appEvents.trigger("composer:cancel-upload", { fileId: self.node.attrs.fileId });
    }
  });
  this.dom.append(icon, this.filename, this.progress, cancel);

  this.onProgress = function (percentage) {
    self.progress.textContent = percentage + "%";
  };
  this.progressEvent = "composer:upload-progress:" + node.attrs.fileId;
  if (this.appEvents) {
    this.appEvents.on(this.progressEvent, this.onProgress);
  }
}

UploadPlaceholderNodeView.prototype.update = function (node) {
  if (node.type !== this.node.type) {
    return false;
  }
  this.node = node;
  this.filename.textContent = node.attrs.filename;
  return true;
};

UploadPlaceholderNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

UploadPlaceholderNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

UploadPlaceholderNodeView.prototype.ignoreMutation = function () {
  return true;
};

UploadPlaceholderNodeView.prototype.destroy = function () {
  if (this.appEvents) {
    this.appEvents.off(this.progressEvent, this.onProgress);
  }
};
