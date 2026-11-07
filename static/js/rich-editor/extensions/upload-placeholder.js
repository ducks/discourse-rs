// extensions/upload-placeholder.js: a file being uploaded, shown while it
// goes and serialized as nothing, and the plugin that cancels the upload
// of a placeholder the member deletes and drops duplicated ones. The
// composer's side of it is lib/text-manipulation.ts's
// ProsemirrorPlaceholderHandler (placeholderHandler below: an image file
// is an image node with `placeholder` and the file's id as its title, any
// other a generic placeholder) and autoGridImages.
import { NodeSelection } from "prosemirror-state";
import { UploadPlaceholderNodeView } from "discourse/rich-editor/node-views/upload-placeholder";
import { lookupCachedUploadUrl } from "discourse/rich-editor/extensions/image";

export { cacheShortUploadUrl } from "discourse/rich-editor/extensions/image";

function firstBlockOf(doc) {
  return doc.firstChild || doc;
}

function revokeBlobUrl(node) {
  if (node.attrs.src && node.attrs.src.startsWith("blob:")) {
    URL.revokeObjectURL(node.attrs.src);
  }
}

function findPlaceholder(view, fileId) {
  var schema = view.state.schema;
  var result = null;
  view.state.doc.descendants(function (node, pos) {
    if (result) {
      return false;
    }
    if (
      (node.type === schema.nodes.image && node.attrs.placeholder && node.attrs.title === fileId) ||
      (node.type === schema.nodes.upload_placeholder && node.attrs.fileId === fileId)
    ) {
      result = { node: node, pos: pos };
      return false;
    }
  });
  return result;
}

// ProsemirrorPlaceholderHandler over an editor (editor.js createEditor's
// result). A file is { id, name, data: File }; uploadingText is
// i18n("uploading_filename"), with %{filename} left in.
export function placeholderHandler(editor, uploadingText) {
  var view = editor.view;
  var schema = editor.schema;

  return {
    insert: function (file) {
      var isImage = !!(file.data && file.data.type && file.data.type.startsWith("image/"));
      var parent = view.state.selection.$from.parent;
      var isEmptyParagraph = parent.type.name === "paragraph" && parent.nodeSize === 2;

      var node = isImage
        ? schema.nodes.image.create({
            src: URL.createObjectURL(file.data),
            alt: uploadingText.replace("%{filename}", file.name),
            title: file.id,
            placeholder: true,
          })
        : schema.nodes.upload_placeholder.create({ fileId: file.id, filename: file.name });

      view.dispatch(
        view.state.tr
          .insert(view.state.selection.from, isEmptyParagraph ? node : schema.nodes.paragraph.create(null, node))
          .setMeta("addToHistory", false)
      );
    },

    cancelAll: function () {
      var toDelete = [];
      view.state.doc.descendants(function (node, pos) {
        if (node.type === schema.nodes.image && node.attrs.placeholder) {
          revokeBlobUrl(node);
          toDelete.push({ pos: pos, size: node.nodeSize });
        } else if (node.type === schema.nodes.upload_placeholder) {
          toDelete.push({ pos: pos, size: node.nodeSize });
        }
      });

      if (toDelete.length) {
        var tr = view.state.tr;
        toDelete.reverse().forEach(function (entry) {
          tr.delete(entry.pos, entry.pos + entry.size);
        });
        view.dispatch(tr.setMeta("addToHistory", false));
      }
    },

    cancel: function (file) {
      var found = findPlaceholder(view, file.id);
      if (found) {
        revokeBlobUrl(found.node);
        view.dispatch(view.state.tr.delete(found.pos, found.pos + found.node.nodeSize).setMeta("addToHistory", false));
      }
    },

    // The placeholder becomes the upload's markdown; false when it is gone.
    success: function (file, markdown) {
      var found = findPlaceholder(view, file.id);
      if (!found) {
        return false;
      }

      var wasSelected = view.state.selection.from === found.pos;

      // keeping compatibility with plugins that change the upload markdown
      var tr = view.state.tr;
      var replacement = firstBlockOf(editor.convertFromMarkdown(markdown)).content;

      if (found.node.type === schema.nodes.image) {
        revokeBlobUrl(found.node);
      }

      tr.replaceWith(found.pos, found.pos + found.node.nodeSize, replacement);

      // resolve transparent.png placeholders using the upload URL cache,
      // which was populated before success() was called
      tr.doc.nodesBetween(found.pos, found.pos + replacement.size, function (node, pos) {
        if (
          node.type.name === "image" &&
          node.attrs.originalSrc &&
          node.attrs.src &&
          node.attrs.src.includes("transparent.png")
        ) {
          var cached = lookupCachedUploadUrl(node.attrs.originalSrc);
          if (cached && cached.url) {
            tr.setNodeMarkup(pos, null, Object.assign({}, node.attrs, { src: cached.url }));
          }
        }
      });

      if (wasSelected && tr.doc.resolve(found.pos).nodeAfter) {
        tr.setSelection(NodeSelection.create(tr.doc, found.pos));
      }

      view.dispatch(tr.setMeta("uploadPlaceholderResolved", true));
      return true;
    },
  };
}

function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// TextManipulation#autoGridImages: the consecutive placeholders of these
// filenames wrapped in a grid.
export function autoGridImages(editor, consecutiveImages, uploadingText) {
  if (!consecutiveImages || consecutiveImages.length === 0) {
    return;
  }

  var view = editor.view;
  var schema = editor.schema;
  var imagesToWrapGrid = new Set(consecutiveImages);
  var placeholderNodes = [];

  view.state.doc.descendants(function (node, pos) {
    if (node.type === schema.nodes.grid) {
      return false;
    }

    if (node.type === schema.nodes.image && node.attrs.placeholder && node.attrs.alt) {
      var uploadingTextMatch = uploadingText.replace("%{filename}", "%placeholder%").match(/^.*(?=: %placeholder%\s?…)/);

      if (uploadingTextMatch && uploadingTextMatch[0]) {
        var pattern = new RegExp(escapeRegExp(uploadingTextMatch[0].trim()) + "\\s?: ([^…]+)");
        var match = node.attrs.alt.match(pattern);

        if (match && match[1] && imagesToWrapGrid.has(match[1])) {
          placeholderNodes.push({ node: node, pos: pos, filename: match[1] });
        }
      }
    } else if (node.type === schema.nodes.upload_placeholder && imagesToWrapGrid.has(node.attrs.filename)) {
      placeholderNodes.push({ node: node, pos: pos, filename: node.attrs.filename });
    }
  });

  if (placeholderNodes.length !== consecutiveImages.length) {
    return;
  }

  placeholderNodes.sort(function (a, b) {
    return a.pos - b.pos;
  });

  for (var i = 1; i < placeholderNodes.length; i++) {
    var prevNode = placeholderNodes[i - 1];
    if (placeholderNodes[i].pos > prevNode.pos + prevNode.node.nodeSize + 2) {
      return;
    }
  }

  var startPos = placeholderNodes[0].pos;
  var lastNode = placeholderNodes[placeholderNodes.length - 1];
  var endPos = lastNode.pos + lastNode.node.nodeSize;

  var tr = view.state.tr;
  var content = tr.doc.slice(startPos, endPos).content;
  var gridNode = schema.nodes.grid.createAndFill(null, content);

  if (gridNode) {
    tr.replaceWith(startPos, endPos, gridNode);
    view.dispatch(tr);
  }
}

export default {
  nodeSpec: {
    upload_placeholder: {
      inline: true,
      atom: true,
      selectable: true,
      draggable: true,
      group: "inline",
      attrs: {
        fileId: { default: null },
        filename: { default: "" },
      },
      toDOM: function (node) {
        return ["span", { class: "upload-placeholder --file", "data-upload-id": node.attrs.fileId }, node.attrs.filename];
      },
      parseDOM: [],
    },
  },

  nodeViews: {
    upload_placeholder: function (params) {
      return function (node, view, getPos) {
        return new UploadPlaceholderNodeView(node, view, getPos, params);
      };
    },
  },

  serializeNode: {
    upload_placeholder: function () {},
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var PluginKey = params.pmState.PluginKey;
    var getContext = params.getContext;

    function placeholderId(node) {
      if (node.type.name === "upload_placeholder") {
        return node.attrs.fileId;
      }
      if (node.type.name === "image" && node.attrs.placeholder) {
        return node.attrs.title;
      }
      return null;
    }

    function collectPlaceholderIds(doc) {
      var ids = new Set();
      doc.descendants(function (node) {
        var id = placeholderId(node);
        if (id !== null) {
          ids.add(id);
        }
      });
      return ids;
    }

    return new Plugin({
      key: new PluginKey("uploadPlaceholder"),

      appendTransaction: function (transactions, oldState, newState) {
        var isUserAction = transactions.some(function (tr) {
          return tr.docChanged && tr.getMeta("addToHistory") !== false;
        });
        var resolvesUpload = transactions.some(function (tr) {
          return tr.getMeta("uploadPlaceholderResolved");
        });
        if (!isUserAction || resolvesUpload) {
          return;
        }

        var oldIds = collectPlaceholderIds(oldState.doc);
        if (oldIds.size === 0) {
          return;
        }

        var newIds = collectPlaceholderIds(newState.doc);
        var appEvents = getContext().appEvents;
        oldIds.forEach(function (id) {
          if (!newIds.has(id) && appEvents) {
            appEvents.trigger("composer:cancel-upload", { fileId: id });
          }
        });

        // Remove duplicated placeholders (e.g. from Option+drag)
        var seen = new Set();
        var dupes = [];
        newState.doc.descendants(function (node, pos) {
          var id = placeholderId(node);
          if (id) {
            if (seen.has(id)) {
              dupes.push({ pos: pos, size: node.nodeSize });
            } else {
              seen.add(id);
            }
          }
        });

        if (dupes.length) {
          var tr = newState.tr;
          dupes.reverse().forEach(function (entry) {
            tr.delete(entry.pos, entry.pos + entry.size);
          });
          return tr.setMeta("addToHistory", false);
        }
      },
    });
  },
};
