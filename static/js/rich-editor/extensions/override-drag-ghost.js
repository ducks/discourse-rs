// extensions/override-drag-ghost.ts: an empty drag image so the drop
// cursor shows where a dragged node lands, a dragged image moved inside a
// paragraph of its own, and the image (not its paragraph) selected after a
// drop.
import { Fragment, Slice } from "prosemirror-model";
import { NodeSelection, Plugin } from "prosemirror-state";

// A 1px transparent image to use as drag image.
var EMPTY_DRAG_IMG = new Image();
EMPTY_DRAG_IMG.src = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

var dropSelectionPlugin = new Plugin({
  appendTransaction: function (transactions, oldState, newState) {
    var hasDropEvent = transactions.some(function (tr) {
      return tr.getMeta("uiEvent") === "drop";
    });

    if (!hasDropEvent) {
      return null;
    }

    var selection = newState.selection;
    if (
      selection instanceof NodeSelection &&
      selection.node.type.name === "paragraph" &&
      selection.node.childCount === 1 &&
      selection.node.child(0).type.name === "image"
    ) {
      return newState.tr.setSelection(NodeSelection.create(newState.doc, selection.from + 1));
    }
    return null;
  },
});

export default {
  plugins: [
    dropSelectionPlugin,
    {
      props: {
        handleDOMEvents: {
          dragover: function (view) {
            if (!view.dragging) {
              return false;
            }

            var dragging = view.dragging.slice.content.firstChild;
            if (dragging && dragging.type.name === "image") {
              var wrappedNode = view.state.schema.nodes.paragraph.create(null, dragging);
              view.dragging.slice = new Slice(Fragment.from(wrappedNode), 0, 0);
            }
            return false;
          },
          dragstart: function (view, event) {
            if (event.dataTransfer) {
              event.dataTransfer.setDragImage(EMPTY_DRAG_IMG, 0, 0);
            }
            return false;
          },
        },
      },
    },
  ],
};
