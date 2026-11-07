// Editing inside tables (static/prosemirror/lib/table/editing.js): Tab
// between cells, a table pasted into a cell, malformed tables repaired after
// each change, a text selection across cells that replaces only each cell's
// selected content, and the classes marking the caret's table, row and
// column.
import { Fragment, Slice } from "prosemirror-model";
import { Plugin, TextSelection } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import { currentCell, goToNextCell } from "discourse/rich-editor/table/commands";
import { cellAround, findTable } from "discourse/rich-editor/table/grid";
import handleTablePaste from "discourse/rich-editor/table/paste";
import { fixTables } from "discourse/rich-editor/table/repair";

function crossCellRange(selection) {
  if (!(selection instanceof TextSelection) || selection.empty) {
    return null;
  }

  var $fromCell = cellAround(selection.$from);
  var $toCell = cellAround(selection.$to);
  if (!$fromCell || !$toCell || $fromCell.pos === $toCell.pos) {
    return null;
  }

  var table = findTable($fromCell);
  var endTable = findTable($toCell);
  return table && endTable && endTable.pos === table.pos ? table : null;
}

// The native text selection and clipboard slices stay, but replacing it
// replaces only the selected content of each cell, also for commands that
// insert nodes directly.
class TableTextSelection extends TextSelection {
  map(doc, mapping) {
    var selection = super.map(doc, mapping);
    return crossCellRange(selection) ? new TableTextSelection(selection.$anchor, selection.$head) : selection;
  }

  replace(tr, content) {
    content = content || Slice.empty;
    var table = crossCellRange(this);
    var mapFrom = tr.steps.length;
    var ranges = [];
    var self = this;
    table.grid.rows.forEach(function (row) {
      row.cells.forEach(function (cell) {
        var start = table.start + cell.offset + 1;
        var from = Math.max(self.from, start);
        var to = Math.min(self.to, start + cell.node.content.size);
        if (from < to) {
          ranges.push({ from: from, to: to });
        }
      });
    });
    ranges.reverse().forEach(function (range) {
      tr.delete(range.from, range.to);
    });
    tr.setSelection(TextSelection.create(tr.doc, tr.mapping.slice(mapFrom).map(this.from, -1)));
    tr.replaceSelection(content);
  }

  replaceWith(tr, node) {
    this.replace(tr, new Slice(Fragment.from(node), 0, 0));
  }
}

function replaceCrossCellSelection(view, text) {
  if (!view.editable || !(view.state.selection instanceof TableTextSelection)) {
    return false;
  }
  var tr = view.state.tr;
  view.dispatch((text === undefined ? tr.deleteSelection() : tr.insertText(text)).scrollIntoView());
  return true;
}

function handleDelete(deleting) {
  return function (view, event) {
    if (!deleting(event)) {
      return false;
    }

    var handled = replaceCrossCellSelection(view);
    if (handled) {
      event.preventDefault();
    }
    return handled;
  };
}

function changeInsideOneCell(doc, from, to) {
  var safeFrom = Math.min(from, doc.content.size);
  var safeTo = Math.min(Math.max(to, safeFrom), doc.content.size);
  var $from = cellAround(doc.resolve(safeFrom));
  var $to = cellAround(doc.resolve(safeTo));

  return (
    !!$from &&
    !!$to &&
    $from.pos === $to.pos &&
    safeFrom > $from.pos &&
    safeTo < $from.pos + $from.nodeAfter.nodeSize
  );
}

// The changed span that may hold a malformed table.
function repairableRange(oldDoc, newDoc) {
  var start = oldDoc.content.findDiffStart(newDoc.content);
  if (start === null) {
    return null;
  }

  var end = oldDoc.content.findDiffEnd(newDoc.content);
  if (end && changeInsideOneCell(oldDoc, start, end.a) && changeInsideOneCell(newDoc, start, end.b)) {
    return null;
  }

  var size = newDoc.content.size;
  var last = end ? end.b : size;

  // Widened by one either side: nodesBetween skips a node that begins or
  // ends right on the boundary, and a deletion collapses to one position.
  return {
    from: Math.max(Math.min(start, last) - 1, 0),
    to: Math.min(Math.max(start, last) + 1, size),
  };
}

function selectionDecorations(state) {
  var decorations = [];
  var current = currentCell(state);

  // Touch input reveals the controls of the active table without hover. The
  // class must not follow the selection: a node decoration on the table
  // redraws its whole content, which would rebuild every grip inside it.
  if (current) {
    decorations.push(Decoration.node(current.pos, current.pos + current.node.nodeSize, { class: "is-active" }));

    var currentRow = current.grid.rows[current.rect.top];
    var rowCell = currentRow && currentRow.cells[0];
    var columnCell = current.grid.rows[0] && current.grid.rows[0].cells[current.rect.left];

    if (rowCell) {
      decorations.push(
        Decoration.node(current.start + rowCell.offset, current.start + rowCell.offset + rowCell.node.nodeSize, {
          class: "is-current-row",
        })
      );
    }
    if (columnCell) {
      decorations.push(
        Decoration.node(
          current.start + columnCell.offset,
          current.start + columnCell.offset + columnCell.node.nodeSize,
          { class: "is-current-column" }
        )
      );
    }
  }

  return decorations.length ? DecorationSet.create(state.doc, decorations) : null;
}

function handleKeyDown(view, event) {
  if (!view.editable) {
    return false;
  }

  if (event.key === "Tab") {
    if (!currentCell(view.state)) {
      return false;
    }
    var handled = goToNextCell(event.shiftKey ? -1 : 1)(view.state, view.dispatch, view);
    if (handled) {
      event.preventDefault();
    }
    return handled;
  }

  return false;
}

export default function tableEditing() {
  return new Plugin({
    props: {
      decorations: selectionDecorations,
      handleKeyDown: handleKeyDown,
      handlePaste: handleTablePaste,
      handleTextInput: function (view, from, to, text) {
        return replaceCrossCellSelection(view, text);
      },
      handleDOMEvents: {
        beforeinput: handleDelete(function (event) {
          return !!event.inputType && event.inputType.indexOf("delete") === 0;
        }),
        keydown: handleDelete(function (event) {
          return event.key === "Backspace" || event.key === "Delete";
        }),
      },
    },

    appendTransaction: function (transactions, oldState, newState) {
      var changed = transactions.some(function (tr) {
        return tr.docChanged;
      })
        ? repairableRange(oldState.doc, newState.doc)
        : null;
      var tr = changed ? fixTables(newState, null, changed.from, changed.to) : null;
      var selection = tr ? tr.selection : newState.selection;
      if (!(selection instanceof TableTextSelection) && crossCellRange(selection)) {
        tr = tr || newState.tr;
        tr.setSelection(new TableTextSelection(selection.$anchor, selection.$head));
      }
      return tr;
    },
  });
}
