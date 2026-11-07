// A table pasted into a cell (static/prosemirror/lib/table/paste.js): tables
// don't nest, so its cells are written into the grid from the target cell,
// growing the table where the paste overflows it.
import { Fragment } from "prosemirror-model";
import { currentCell, selectCell } from "discourse/rich-editor/table/commands";
import { cellType, copyCell, isTable, tableGrid } from "discourse/rich-editor/table/grid";

function pastedRows(slice) {
  var table = slice.content.childCount === 1 && slice.content.firstChild;
  if (!table || !isTable(table)) {
    return null;
  }

  return tableGrid(table).rows.map(function (row) {
    return row.cells.map(function (cell) {
      return cell.node;
    });
  });
}

function alignmentAt(grid, col) {
  var cell = grid.rows[0] && grid.rows[0].cells[col];
  return cell ? cell.node.attrs.alignment : null;
}

export default function handlePaste(view, event, slice) {
  if (!view.editable) {
    return false;
  }
  var table = currentCell(view.state);
  if (!table) {
    return false;
  }

  var rows = pastedRows(slice);
  if (!rows) {
    return false;
  }

  var state = view.state;
  var schema = state.schema;
  var top = table.rect.top;
  var left = table.rect.left;
  var width = Math.max(
    table.grid.width,
    left +
      Math.max.apply(
        null,
        rows.map(function (row) {
          return row.length;
        })
      )
  );
  var height = Math.max(table.grid.height, top + rows.length);

  var built = [];
  for (var row = 0; row < height; row++) {
    var source = table.grid.rows[row];
    var type = cellType(schema, row === 0 && !!table.grid.head);
    var cells = [];

    for (var col = 0; col < width; col++) {
      var alignment = alignmentAt(table.grid, col);
      var incoming = col >= left && row >= top && rows[row - top] ? rows[row - top][col - left] : undefined;
      var existing = source && source.cells[col] ? source.cells[col].node : undefined;

      if (incoming) {
        cells.push(copyCell(type, incoming, { alignment: alignment }));
      } else if (existing) {
        cells.push(copyCell(type, existing));
      } else {
        cells.push(type.createAndFill({ alignment: alignment }));
      }
    }

    built.push(schema.nodes.table_row.create(null, Fragment.from(cells)));
  }

  var sections = [];
  if (table.grid.head) {
    sections.push(schema.nodes.table_head.create(null, built[0]));
  }
  sections.push(schema.nodes.table_body.create(null, Fragment.from(built.slice(table.grid.head ? 1 : 0))));

  view.dispatch(
    selectCell(
      state.tr.replaceWith(
        table.pos,
        table.pos + table.node.nodeSize,
        schema.nodes.table.create(table.node.attrs, Fragment.from(sections))
      ),
      table.pos,
      top,
      left
    )
      .setMeta("paste", true)
      .setMeta("uiEvent", "paste")
  );
  return true;
}
