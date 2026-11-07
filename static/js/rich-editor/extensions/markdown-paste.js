// extensions/markdown-paste.js: pasted plain text is markdown, parsed as
// the editor parses its value; several lines pasted into an empty list
// item become items of that list.
export default {
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var Fragment = params.pmModel.Fragment;
    var Slice = params.pmModel.Slice;
    var utils = params.utils;

    function buildListSlice(schema, listType, lines) {
      return Slice.maxOpen(Fragment.from(utils.buildListNode(schema, listType, lines)));
    }

    function getEmptyListPasteContext(view) {
      var selection = view.state.selection;
      var schema = view.state.schema;

      if (!selection.empty) {
        return null;
      }

      var $from = selection.$from;

      for (var depth = $from.depth; depth > 1; depth--) {
        var node = $from.node(depth);

        if (node.type !== schema.nodes.list_item) {
          continue;
        }

        var parentList = $from.node(depth - 1);
        var isSupportedList =
          parentList.type === schema.nodes.bullet_list || parentList.type === schema.nodes.ordered_list;
        var isEmptyParagraph =
          node.childCount === 1 &&
          node.firstChild &&
          node.firstChild.type === schema.nodes.paragraph &&
          node.firstChild.content.size === 0;

        if (isSupportedList && isEmptyParagraph && $from.parentOffset === 0) {
          return { listType: parentList.type.name };
        }

        return null;
      }

      return null;
    }

    return new Plugin({
      props: {
        clipboardTextParser: function (text, $context, plain, view) {
          var listContext = view && getEmptyListPasteContext(view);

          if (listContext) {
            var lines = utils.splitNonEmptyLines(text);

            if (lines.length > 1) {
              return buildListSlice(view.state.schema, listContext.listType, lines);
            }
          }

          var doc = utils.convertFromMarkdown(text);
          return Slice.maxOpen(Fragment.from(doc.content));
        },
      },
    });
  },
};
