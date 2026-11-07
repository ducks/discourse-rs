// extensions/trailing-inline-space.ts: a block ending in an inline
// container (inline HTML) gets a space after it, so the cursor can leave
// it; Enter inside one splits the block outside the container.
import { splitBlock } from "prosemirror-commands";
import { TextSelection } from "prosemirror-state";

function isInlineContainer(node) {
  return Boolean(
    node.isInline && !node.isText && node.type && node.type.spec && node.type.spec.content && !node.type.spec.atom
  );
}

function findInlineContainerDepth($from) {
  for (var depth = $from.depth; depth > 0; depth--) {
    if (isInlineContainer($from.node(depth))) {
      return depth;
    }
  }
  return null;
}

export default {
  keymap: function () {
    return {
      Enter: function (state, dispatch) {
        var selection = state.selection;
        if (!(selection instanceof TextSelection)) {
          return false;
        }

        var initialDepth = findInlineContainerDepth(selection.$from);
        if (!dispatch) {
          return selection.empty ? initialDepth !== null : true;
        }
        if (selection.empty && initialDepth === null) {
          return false;
        }

        var tr = state.tr;
        if (!selection.empty) {
          tr.delete(selection.from, selection.to);
        }

        var basePos = selection.empty ? selection.$from.pos : selection.from;
        var mappedBasePos = tr.mapping.map(basePos, -1);
        var $from = tr.doc.resolve(mappedBasePos);
        var depth = findInlineContainerDepth($from);
        if (depth === null) {
          if (tr.steps.length) {
            dispatch(tr);
          }
          return splitBlock(tr.steps.length ? state.apply(tr) : state, dispatch);
        }

        var atStart = $from.pos === $from.start(depth);
        var atEnd = $from.pos === $from.end(depth);
        var pos;

        if (atStart) {
          pos = $from.before(depth);
          tr.setSelection(TextSelection.create(tr.doc, pos));
          dispatch(tr);
          return splitBlock(state.apply(tr), dispatch);
        }

        if (atEnd) {
          pos = $from.after(depth);
          var next = tr.doc.resolve(pos).nodeAfter;
          if (next && next.isText && next.text && next.text[0] === " ") {
            pos += 1;
          }
          tr.setSelection(TextSelection.create(tr.doc, pos));
          dispatch(tr);
          return splitBlock(state.apply(tr), dispatch);
        }

        var fromPos = $from.pos;
        var containerEnd = $from.end(depth);
        var rightSlice = tr.doc.slice(fromPos, containerEnd);

        tr.delete(fromPos, containerEnd);
        $from = tr.doc.resolve(fromPos);
        var outsidePos = $from.after(depth);
        if (rightSlice.content.size) {
          tr.insert(outsidePos, rightSlice.content);
        }
        tr.setSelection(TextSelection.create(tr.doc, outsidePos));
        dispatch(tr);
        return splitBlock(state.apply(tr), dispatch);
      },
    };
  },
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var PluginKey = params.pmState.PluginKey;
    var changedDescendants = params.utils.changedDescendants;

    return new Plugin({
      key: new PluginKey("trailingInlineSpace"),

      appendTransaction: function (transactions, oldState, newState) {
        var tr = newState.tr;

        changedDescendants(oldState.doc, newState.doc, function (node, pos) {
          if (!node.isBlock || !node.inlineContent || node.childCount === 0) {
            return;
          }
          var lastChild = node.lastChild;
          if (lastChild && isInlineContainer(lastChild)) {
            // pos is the new document's, mapped past the spaces already added
            tr.insert(tr.mapping.map(pos + node.nodeSize - 1), newState.schema.text(" "));
          }
        });

        return tr.steps.length ? tr : null;
      },
    });
  },
};
