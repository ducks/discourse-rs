// The image's node view: components/media-node-view.gjs (an image, or a
// video for `|video`), image-node-view.gjs (the image, its upload
// placeholder overlay, the image toolbar and alt text menus, the lightbox
// and the grid moves) and lib/glimmer-node-view.js's wrapper (a span with
// the composer-image-node class, mutations ignored). The menus are shown
// inside the node's DOM, as the reference's portalOutletElement does
// (inlineMenu, which the grid's node view shares), and lib/lightbox.js's
// openLightbox is here for the editor's images.
import { NodeSelection } from "prosemirror-state";
import { ImageAltTextInput } from "discourse/rich-editor/node-views/image-alt-text-input";
import { VideoNodeView } from "discourse/rich-editor/node-views/video";
import { iconHtml, renderToolbarButtons, ToolbarBase } from "discourse/rich-editor/toolbar";

var PLACEHOLDER_CLASSES = ["upload-placeholder", "--image"];

var MIN_SCALE = 25;
var MAX_SCALE = 100;
var SCALE_STEP = 25;

var MENU_PADDING = 8;

var GRID_ADD_DIRECTION_END = -1;
var GRID_ADD_DIRECTION_START = 1;

function i18nOf(params) {
  var context = params.getContext();
  return (
    context.i18n ||
    function (key) {
      return key;
    }
  );
}

// A float-kit menu (DFloatBody's markup) shown in `outlet` by `trigger`,
// positioned with Discourse.float.attach: the menu service's newInstance
// with portalOutletElement. Returns { content, expanded, show, close,
// destroy }.
export function inlineMenu(outlet, trigger, options) {
  var menu = {
    content: null,
    expanded: false,
    stop: null,
    show: function () {
      if (menu.expanded) {
        return Promise.resolve(menu);
      }
      var content = document.createElement("div");
      content.className = "fk-d-menu -animated -expanded";
      content.dataset.content = "";
      content.dataset.identifier = options.identifier;
      content.setAttribute("role", options.contentRole || "dialog");
      content.setAttribute("style", "max-width: min(400px, calc(100dvw - 20px))");
      content.contentEditable = "false";
      var inner = document.createElement("div");
      inner.className = "fk-d-menu__inner-content";
      options.render(inner);
      content.appendChild(inner);
      outlet.appendChild(content);
      menu.content = content;
      menu.expanded = true;
      menu.stop = window.Discourse.float.attach(trigger, content, {
        placement: options.placement,
        fallbackPlacements: options.fallbackPlacements,
        padding: options.padding,
        offset: options.offset,
        limitShift: options.limitShift,
        onPositioned: options.onPositioned,
        autoUpdate: true,
      });
      return Promise.resolve(menu);
    },
    close: function () {
      if (menu.stop) {
        menu.stop();
        menu.stop = null;
      }
      if (menu.content) {
        menu.content.remove();
      }
      menu.content = null;
      menu.expanded = false;
    },
    destroy: function () {
      menu.close();
    },
  };
  return menu;
}

// lib/lightbox.js openLightbox: a PhotoSwipe gallery over standalone media
// elements, with the titles lightbox.js's script tag carries. Resolves
// when it closes.
export function openLightbox(items, opts) {
  var index = (opts && opts.index) || 0;
  var script = document.querySelector("script[data-lightbox]");
  if (!script) {
    return Promise.resolve();
  }
  var config = script.dataset;
  var dataSource = items.map(function (item) {
    var element = item.element;
    return {
      src: item.src,
      msrc: (element && element.currentSrc) || item.src,
      element: element,
      width: (element && element.naturalWidth) || 800,
      height: (element && element.naturalHeight) || 600,
      alt: item.alt !== undefined && item.alt !== null ? item.alt : (element && element.alt) || "",
      thumbCropped: true,
    };
  });

  return Promise.all([import(config.lightbox), import(config.photoswipe)]).then(function (modules) {
    var PhotoSwipeLightbox = modules[0].default;
    var PhotoSwipe = modules[1].default;
    var lightbox = new PhotoSwipeLightbox({
      arrowPrevTitle: config.titlePrevious,
      arrowNextTitle: config.titleNext,
      closeTitle: config.titleClose,
      zoomTitle: config.titleZoom,
      errorMsg: config.titleError,
      showHideAnimationType: "zoom",
      escKey: false,
      tapAction: function (pt, event) {
        var pswp = lightbox.pswp;
        if (event.target.classList.contains("pswp__img")) {
          if (pswp && pswp.element) {
            pswp.element.classList.toggle("pswp--ui-visible");
          }
        } else if (pswp) {
          pswp.close();
        }
      },
      dataSource: dataSource,
      pswpModule: PhotoSwipe,
    });

    var onKeyDown = function (event) {
      if (event.key !== "Escape") {
        return;
      }
      event.stopPropagation();
      event.preventDefault();
      lightbox.pswp.close();
    };
    lightbox.on("afterInit", function () {
      lightbox.pswp.element.addEventListener("keydown", onKeyDown);
    });

    lightbox.on("uiRegister", function () {
      lightbox.pswp.ui.registerElement({
        name: "caption",
        order: 11,
        isButton: false,
        appendTo: "root",
        html: "",
        onInit: function (caption, pswp) {
          var titleEl = document.createElement("div");
          titleEl.className = "pswp__caption-title";
          caption.appendChild(titleEl);
          pswp.on("change", function () {
            titleEl.textContent = (pswp.currSlide && pswp.currSlide.data.alt) || "";
          });
        },
      });
    });

    lightbox.on("contentLoadImage", function (event) {
      var content = event.content;
      var image = content.element;
      image.addEventListener(
        "load",
        function () {
          syncSlideSize(content, image.naturalWidth, image.naturalHeight);
        },
        { once: true }
      );
    });

    return new Promise(function (resolve) {
      lightbox.addFilter("thumbEl", function (thumbEl, itemData) {
        return itemData.element && itemData.element.isConnected ? itemData.element : undefined;
      });
      lightbox.on("close", resolve);
      lightbox.on("closingAnimationEnd", function () {
        lightbox.destroy();
      });

      lightbox.init();
      if (!lightbox.loadAndOpen(index)) {
        lightbox.destroy();
        resolve();
      }
    });
  });
}

function syncSlideSize(content, width, height) {
  if (!width || !height || (content.width === width && content.height === height)) {
    return;
  }
  content.width = content.data.width = width;
  content.height = content.data.height = height;
  if (content.slide) {
    content.slide.width = width;
    content.slide.height = height;
    content.slide.resize();
    content.slide.updateContentSize(true);
  }
}

function setAttr(el, name, value) {
  if (value === null || value === undefined || value === false) {
    el.removeAttribute(name);
  } else {
    el.setAttribute(name, value);
  }
}

// image-node-view.gjs
export function ImageNodeView(owner) {
  this.owner = owner;
  this.dom = owner.dom;
  this.params = owner.params;
  this.i18n = i18nOf(owner.params);
  this.imageLoaded = false;
  this.uploadProgress = 0;
  this.menuInstance = null;
  this.altMenuInstance = null;
  this.progressEvent = null;
  this.onUploadProgress = this.onUploadProgress.bind(this);

  var self = this;
  var img = (this.image = document.createElement("img"));
  img.addEventListener("load", function () {
    self.imageLoaded = true;
    self.renderImage();
  });
  img.addEventListener("mousedown", function (event) {
    // keep the alt input focused so the click handler treats this click as
    // a dismissal; ProseMirror also ignores default-prevented events
    if (event.button === 0 && self.altInputHasFocus()) {
      event.preventDefault();
    }
  });
  img.addEventListener("click", function (event) {
    self.handleImageClick(event);
  });
  this.dom.appendChild(img);

  if (this.isPlaceholder()) {
    var fileId = this.node().attrs.title;
    PLACEHOLDER_CLASSES.forEach(function (c) {
      self.dom.classList.add(c);
    });
    this.dom.dataset.uploadId = fileId;
    this.progressEvent = "composer:upload-progress:" + fileId;
    var appEvents = this.appEvents();
    if (appEvents) {
      appEvents.on(this.progressEvent, this.onUploadProgress);
    }
    this.renderOverlay();
  }

  this.renderImage();
}

ImageNodeView.prototype.node = function () {
  return this.owner.node;
};

ImageNodeView.prototype.appEvents = function () {
  return this.params.getContext().appEvents;
};

ImageNodeView.prototype.isPlaceholder = function () {
  return !!this.node().attrs.placeholder;
};

ImageNodeView.prototype.maxDimensions = function () {
  if (!this.imageLoaded) {
    return null;
  }
  var siteSettings = this.params.getContext().siteSettings || {};
  var widthRatio = siteSettings.max_image_width / this.image.naturalWidth;
  var heightRatio = siteSettings.max_image_height / this.image.naturalHeight;
  var ratio = Math.min(widthRatio, heightRatio);
  return {
    width: Math.floor(this.image.naturalWidth * ratio),
    height: Math.floor(this.image.naturalHeight * ratio),
  };
};

ImageNodeView.prototype.imageWidth = function () {
  var attrs = this.node().attrs;
  var max = this.maxDimensions();
  var width = attrs.width !== null && attrs.width !== undefined ? attrs.width : max && max.width;
  if (!width) {
    return null;
  }
  var scale = (attrs.scale !== null && attrs.scale !== undefined ? attrs.scale : 100) / 100;
  return width * scale;
};

ImageNodeView.prototype.renderImage = function () {
  var attrs = this.node().attrs;
  var img = this.image;
  setAttr(img, "alt", attrs.alt);
  setAttr(img, "data-orig-src", attrs.originalSrc);
  setAttr(img, "data-placeholder", attrs.placeholder);
  setAttr(img, "data-scale", attrs.scale);
  setAttr(img, "data-thumbnail", attrs.extras === "thumbnail" ? "true" : null);
  setAttr(img, "height", attrs.height);
  img.setAttribute("role", "button");
  if (img.getAttribute("src") !== attrs.src) {
    setAttr(img, "src", attrs.src);
  }
  var width = this.imageWidth();
  setAttr(img, "style", width ? "width: " + width + "px" : null);
  setAttr(img, "title", attrs.title);
  setAttr(img, "width", attrs.width);
};

ImageNodeView.prototype.renderOverlay = function () {
  var self = this;
  var overlay = (this.overlay = document.createElement("span"));
  overlay.className = "upload-placeholder__overlay";
  var progress = (this.progressEl = document.createElement("span"));
  progress.className = "upload-placeholder__progress";
  progress.textContent = this.uploadProgress + "%";
  var cancel = document.createElement("button");
  cancel.className = "upload-placeholder__cancel btn-transparent no-text";
  cancel.contentEditable = "false";
  cancel.setAttribute("aria-label", this.i18n("cancel"));
  cancel.title = this.i18n("cancel");
  cancel.innerHTML = iconHtml("xmark");
  cancel.addEventListener("click", function (event) {
    self.cancelUpload(event);
  });
  overlay.appendChild(progress);
  overlay.appendChild(cancel);
  this.dom.appendChild(overlay);
};

ImageNodeView.prototype.onUploadProgress = function (percentage) {
  this.uploadProgress = percentage;
  if (this.progressEl) {
    this.progressEl.textContent = percentage + "%";
  }
};

ImageNodeView.prototype.cancelUpload = function (event) {
  event.preventDefault();
  event.stopPropagation();
  var appEvents = this.appEvents();
  if (appEvents) {
    appEvents.trigger("composer:cancel-upload", { fileId: this.node().attrs.title });
  }
};

ImageNodeView.prototype.isInGrid = function () {
  var pos = this.owner.getPos();
  var $pos = this.owner.view.state.doc.resolve(pos);
  for (var depth = $pos.depth; depth >= 0; depth--) {
    if ($pos.node(depth).type.name === "grid") {
      return true;
    }
  }
  return false;
};

ImageNodeView.prototype.altInputHasFocus = function () {
  return !!(this.altMenuInstance && this.altMenuInstance.content && this.altMenuInstance.content.contains(document.activeElement));
};

ImageNodeView.prototype.update = function () {
  var self = this;
  if (this.progressEvent && !this.node().attrs.placeholder) {
    PLACEHOLDER_CLASSES.forEach(function (c) {
      self.dom.classList.remove(c);
    });
    delete this.dom.dataset.uploadId;
    var appEvents = this.appEvents();
    if (appEvents) {
      appEvents.off(this.progressEvent, this.onUploadProgress);
    }
    this.progressEvent = null;
    if (this.overlay) {
      this.overlay.remove();
      this.overlay = null;
    }
  }
  this.renderImage();
  if (this.menuInstance && this.menuInstance.content) {
    this.renderToolbar(this.menuInstance.content.querySelector(".fk-d-menu__inner-content"));
  }
};

ImageNodeView.prototype.stopEvent = function (event) {
  if (["dragover", "dragend", "drop", "dragleave"].indexOf(event.type) >= 0) {
    return false;
  }
  return !!(
    (this.menuInstance && this.menuInstance.content && this.menuInstance.content.contains(event.target)) ||
    (this.altMenuInstance && this.altMenuInstance.content && this.altMenuInstance.content.contains(event.target))
  );
};

ImageNodeView.prototype.imageToolbar = function () {
  if (this.toolbar) {
    return this.toolbar;
  }
  var self = this;
  var toolbar = new ToolbarBase({ i18n: this.i18n });
  var isInGrid = this.isInGrid();
  var scale = function () {
    var value = self.node().attrs.scale;
    return value !== null && value !== undefined ? value : 100;
  };

  if (!isInGrid) {
    toolbar.addButton({
      id: "image-scale-down",
      icon: "magnifying-glass-minus",
      title: "composer.image_toolbar.zoom_out",
      className: "composer-image-toolbar__zoom-out",
      get disabled() {
        return !(scale() > MIN_SCALE);
      },
      action: function () {
        self.scaleDown();
      },
      tabindex: 0,
    });

    toolbar.addButton({
      id: "image-scale-up",
      icon: "magnifying-glass-plus",
      title: "composer.image_toolbar.zoom_in",
      className: "composer-image-toolbar__zoom-in",
      get disabled() {
        return !(scale() < MAX_SCALE);
      },
      action: function () {
        self.scaleUp();
      },
      tabindex: 0,
    });
  }

  toolbar.addButton({
    id: "image-remove",
    icon: "trash-can",
    title: "composer.image_toolbar.remove",
    className: "composer-image-toolbar__trash",
    action: function () {
      self.removeImage();
    },
    tabindex: 0,
  });

  if (isInGrid) {
    toolbar.addButton({
      id: "image-move-outside-grid",
      icon: "table-cells-minus",
      title: "composer.image_toolbar.move_outside_grid",
      className: "composer-image-toolbar__move-outside-grid",
      action: function () {
        self.moveOutsideGrid();
      },
      tabindex: 0,
    });
  } else {
    toolbar.addButton({
      id: "image-add-to-grid",
      icon: "table-cells-plus",
      title: "composer.image_toolbar.add_to_grid",
      className: "composer-image-toolbar__add-to-grid",
      action: function () {
        self.addToGrid();
      },
      tabindex: 0,
    });
  }

  this.toolbar = toolbar;
  return toolbar;
};

ImageNodeView.prototype.renderToolbar = function (container) {
  if (container) {
    renderToolbarButtons(container, this.imageToolbar());
  }
};

ImageNodeView.prototype.showToolbar = function () {
  var self = this;
  this.menuInstance = inlineMenu(this.dom, this.dom, {
    identifier: "composer-image-toolbar",
    placement: "top-start",
    fallbackPlacements: ["top-start"],
    padding: MENU_PADDING,
    render: function (inner) {
      self.renderToolbar(inner);
    },
    offset: function (state) {
      return {
        mainAxis: -MENU_PADDING - state.rects.floating.height,
        crossAxis: MENU_PADDING,
      };
    },
    limitShift: {
      offset: function (state) {
        var inputHeight = (self.altMenuInstance && self.altMenuInstance.content && self.altMenuInstance.content.offsetHeight) || 0;
        return {
          crossAxis: Math.min(
            state.rects.floating.height + 2 * MENU_PADDING + inputHeight,
            state.rects.reference.height - MENU_PADDING
          ),
        };
      },
    },
  });
  return this.menuInstance.show();
};

ImageNodeView.prototype.showAltText = function () {
  var self = this;
  var view = this.owner.view;
  this.altMenuInstance = inlineMenu(this.dom, this.dom, {
    identifier: "composer-image-alt-text",
    placement: "bottom-start",
    fallbackPlacements: ["bottom-start"],
    padding: MENU_PADDING,
    render: function (inner) {
      self.altInput = new ImageAltTextInput(inner, {
        alt: self.node().attrs.alt,
        i18n: self.i18n,
        view: view,
        onSave: function (altText, forceFocus) {
          self.saveAltText(altText, forceFocus);
        },
        onClose: function () {
          view.focus();
        },
      });
    },
    offset: function (state) {
      return {
        mainAxis: -MENU_PADDING - state.rects.floating.height,
        crossAxis: MENU_PADDING,
      };
    },
    limitShift: {
      offset: function (state) {
        var toolbarHeight = (self.menuInstance && self.menuInstance.content && self.menuInstance.content.offsetHeight) || 0;
        return {
          crossAxis: toolbarHeight + 2 * MENU_PADDING + state.rects.floating.height,
        };
      },
    },
  });
  return this.altMenuInstance.show();
};

ImageNodeView.prototype.saveAltText = function (altText, forceFocus) {
  var pos = this.owner.getPos();
  var view = this.owner.view;
  var tr = view.state.tr;
  tr.setNodeMarkup(pos, null, Object.assign({}, this.node().attrs, { alt: altText || null }));
  tr.setSelection(NodeSelection.create(tr.doc, pos));
  view.dispatch(tr);
  if (forceFocus) {
    view.focus();
  }
};

ImageNodeView.prototype.removeImage = function () {
  var pos = this.owner.getPos();
  var view = this.owner.view;
  view.dispatch(view.state.tr.delete(pos, pos + 1));
};

ImageNodeView.prototype.scaleDown = function () {
  var current = this.node().attrs.scale;
  current = current !== null && current !== undefined ? current : 100;
  this.scaleImage(Math.max(MIN_SCALE, current - SCALE_STEP));
};

ImageNodeView.prototype.scaleUp = function () {
  var current = this.node().attrs.scale;
  current = current !== null && current !== undefined ? current : 100;
  this.scaleImage(Math.min(MAX_SCALE, current + SCALE_STEP));
};

ImageNodeView.prototype.scaleImage = function (scale) {
  var pos = this.owner.getPos();
  var view = this.owner.view;
  var tr = view.state.tr;
  if (!this.node().attrs.width || !this.node().attrs.height) {
    var dimensions = this.maxDimensions();
    if (dimensions) {
      tr.setNodeAttribute(pos, "width", dimensions.width);
      tr.setNodeAttribute(pos, "height", dimensions.height);
    }
  }
  tr.setNodeAttribute(pos, "scale", scale);
  tr.setSelection(NodeSelection.create(tr.doc, pos));
  view.dispatch(tr);
};

ImageNodeView.prototype.selectNode = function () {
  this.image.classList.add("ProseMirror-selectednode");
  if (!this.isPlaceholder()) {
    this.toolbar = null;
    this.showToolbar();
    this.showAltText();
  }
};

ImageNodeView.prototype.deselectNode = function () {
  this.image.classList.remove("ProseMirror-selectednode");
  if (!this.isPlaceholder()) {
    this.closeMenus();
  }
};

ImageNodeView.prototype.closeMenus = function () {
  if (this.menuInstance) {
    this.menuInstance.close();
  }
  this.menuInstance = null;
  if (this.altMenuInstance) {
    this.altMenuInstance.close();
  }
  this.altMenuInstance = null;
};

ImageNodeView.prototype.handleImageClick = function (event) {
  var view = this.owner.view;
  if (this.altInputHasFocus()) {
    event.preventDefault();
    view.focus();
    return;
  }

  if (!this.image.classList.contains("ProseMirror-selectednode")) {
    return;
  }
  event.preventDefault();
  event.stopPropagation();

  // image nodes also hold videos and audio; only rendered images belong
  // in the gallery
  var slides = [];
  view.state.doc.descendants(function (node, pos) {
    if (node.type.name !== "image") {
      return;
    }
    var dom = view.nodeDOM(pos);
    var element = dom && dom.querySelector && dom.querySelector(":scope > img");
    if (element) {
      slides.push({ src: node.attrs.src, element: element });
    }
  });

  var self = this;
  openLightbox(slides, {
    index: slides.findIndex(function (slide) {
      return slide.element === self.image;
    }),
  }).then(function () {
    var pos = self.owner.getPos();
    if (pos !== null && pos !== undefined && pos >= 0 && !view.isDestroyed) {
      view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, pos)));
    }
  });
};

ImageNodeView.prototype.addToGrid = function () {
  var pos = this.owner.getPos();
  var view = this.owner.view;
  var state = view.state;
  var imageNode = this.node();
  var existingGrid = this.findNearbyGrid(pos);

  if (existingGrid) {
    this.addToExistingGrid(existingGrid, pos, imageNode, state, view);
  } else {
    var $pos = state.doc.resolve(pos);
    this.createNewGrid(pos, imageNode, this.isImageOnlyChildInParagraph($pos), state, view);
  }
};

ImageNodeView.prototype.moveOutsideGrid = function () {
  if (!this.isInGrid()) {
    return;
  }

  var pos = this.owner.getPos();
  var view = this.owner.view;
  var state = view.state;
  var tr = state.tr;
  var imageNode = this.node();

  var $pos = state.doc.resolve(pos);
  var gridDepth = null;
  var gridPos = null;

  for (var depth = $pos.depth; depth >= 0; depth--) {
    if ($pos.node(depth).type.name === "grid") {
      gridDepth = depth;
      gridPos = $pos.start(depth);
      break;
    }
  }

  if (gridDepth === null) {
    return;
  }

  var gridNode = $pos.node(gridDepth);

  var willGridBeEmpty =
    gridNode.childCount === 1 &&
    gridNode.firstChild.type.name === "paragraph" &&
    gridNode.firstChild.content.size === 1 &&
    gridNode.firstChild.firstChild.type.name === "image";

  var gridEndPos = gridPos + gridNode.nodeSize - 1;
  var paragraphWithImage = state.schema.nodes.paragraph.create(null, imageNode);

  if (willGridBeEmpty) {
    tr.replaceWith(gridPos - 1, gridPos + gridNode.nodeSize, paragraphWithImage);
    tr.setSelection(NodeSelection.create(tr.doc, gridPos)).scrollIntoView();
  } else {
    tr.delete(pos - 1, pos + 1);
    var adjustedGridEndPos = tr.mapping.map(gridEndPos);
    tr.insert(adjustedGridEndPos, paragraphWithImage);
    tr.setSelection(NodeSelection.create(tr.doc, adjustedGridEndPos + 1)).scrollIntoView();
  }

  view.dispatch(tr);
};

ImageNodeView.prototype.addToExistingGrid = function (existingGrid, pos, imageNode, state, view) {
  var gridNode = existingGrid.node;
  var gridPos = existingGrid.pos;
  var tr = state.tr;
  var paragraphWithImage = state.schema.nodes.paragraph.create(null, imageNode);

  if (existingGrid.direction === GRID_ADD_DIRECTION_END) {
    var insertPos = gridPos + gridNode.nodeSize - 2;
    tr.insert(insertPos, paragraphWithImage);
    var adjustedPos = pos + paragraphWithImage.nodeSize;
    tr.delete(adjustedPos, adjustedPos + 1);

    // if empty paragraph is left at adjustedPos, remove it explicitly
    var $adjustedPos = tr.doc.resolve(adjustedPos);
    if ($adjustedPos.parent.type.name === "paragraph" && $adjustedPos.parent.content.size === 0) {
      tr.delete(adjustedPos - 1, adjustedPos);
    }

    tr.setSelection(NodeSelection.create(tr.doc, insertPos + 1)).scrollIntoView();
  } else {
    tr.insert(gridPos, paragraphWithImage);
    tr.delete(pos, pos + 1);

    var imagePos = gridPos;
    var $pos = tr.doc.resolve(pos);
    if ($pos.parent.type.name === "paragraph" && $pos.parent.content.size === 0) {
      tr.delete(pos - 1, pos);
      imagePos -= 2;
    }

    tr.setSelection(NodeSelection.create(tr.doc, imagePos)).scrollIntoView();
  }

  view.dispatch(tr);
};

ImageNodeView.prototype.createNewGrid = function (pos, imageNode, isOnlyChildInParagraph, state, view) {
  var tr = state.tr;
  var gridNode = state.schema.nodes.grid.create(null, state.schema.nodes.paragraph.create(null, imageNode));

  var gridStartPos;
  if (isOnlyChildInParagraph) {
    tr.replaceWith(pos - 1, pos + 1, gridNode);
    gridStartPos = pos - 1;
  } else {
    tr.replaceWith(pos, pos + 1, gridNode);
    gridStartPos = pos;
  }

  // the grid's and the paragraph's opening positions, then the image
  tr.setSelection(NodeSelection.create(tr.doc, gridStartPos + 2)).scrollIntoView();
  view.dispatch(tr);
};

ImageNodeView.prototype.isImageOnlyChildInParagraph = function ($pos) {
  var parent = $pos.parent;
  if (parent.type.name !== "paragraph" || parent.childCount !== 1) {
    return false;
  }
  return parent.child($pos.index()).type.name === "image";
};

ImageNodeView.prototype.findNearbyGrid = function (pos) {
  var state = this.owner.view.state;
  var $pos = state.doc.resolve(pos);

  var blockDepth = $pos.depth;
  while (blockDepth > 0 && $pos.node(blockDepth).type.name !== "paragraph") {
    blockDepth--;
  }

  if (blockDepth === 0) {
    return null;
  }

  var blockPos = $pos.start(blockDepth);
  var blockNode = $pos.node(blockDepth);
  var blockParent = $pos.node(blockDepth - 1);
  var blockIndex = $pos.index(blockDepth - 1);

  if (blockIndex > 0) {
    var leftNode = blockParent.child(blockIndex - 1);
    if (leftNode.type.name === "grid") {
      return { node: leftNode, pos: blockPos - leftNode.nodeSize, direction: GRID_ADD_DIRECTION_END };
    }
  }

  if (blockIndex < blockParent.childCount - 1) {
    var rightNode = blockParent.child(blockIndex + 1);
    if (rightNode.type.name === "grid") {
      return { node: rightNode, pos: blockPos + blockNode.nodeSize, direction: GRID_ADD_DIRECTION_START };
    }
  }

  return null;
};

ImageNodeView.prototype.destroy = function () {
  if (this.progressEvent) {
    var appEvents = this.appEvents();
    if (appEvents) {
      appEvents.off(this.progressEvent, this.onUploadProgress);
    }
  }
  this.closeMenus();
};

function isVideo(node) {
  return node.attrs.extras === "video";
}

// media-node-view.gjs in glimmer-node-view.js's wrapper: the video's view
// for `|video`, the image's otherwise.
export function MediaNodeView(node, view, getPos, params) {
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.params = params;
  this.dom = document.createElement("span");
  this.dom.classList.add("composer-image-node");
  this.component = isVideo(node) ? new VideoNodeView(this) : new ImageNodeView(this);
}

MediaNodeView.prototype.update = function (node) {
  if (node.type !== this.node.type || isVideo(node) !== isVideo(this.node) || node.attrs.extras === "audio") {
    return false;
  }
  if (node !== this.node) {
    this.node = node;
    this.component.update(node);
  }
  return true;
};

MediaNodeView.prototype.selectNode = function () {
  this.component.selectNode();
};

MediaNodeView.prototype.deselectNode = function () {
  this.component.deselectNode();
};

MediaNodeView.prototype.stopEvent = function (event) {
  return this.component.stopEvent ? !!this.component.stopEvent(event) : false;
};

MediaNodeView.prototype.ignoreMutation = function () {
  return true;
};

MediaNodeView.prototype.destroy = function () {
  this.component.destroy();
};
