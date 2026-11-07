// Malformed tables put right (static/prosemirror/lib/table/repair.js): a
// table without rows becomes a paragraph, a cell of the wrong type for its
// section is re-typed and a short row is padded out.
import { Fragment } from "prosemirror-model";
import { cellType, isTable, tableGrid } from "discourse/rich-editor/table/grid";

function fixTable(state, entry, tr) {
  var schema = state.schema;
  var node = entry.node;
  var pos = entry.pos;
  var grid = tableGrid(node);
  var start = pos + 1;
  var result = tr;

  if (!grid.height) {
    result = result || state.tr;
    result.replaceWith(pos, pos + node.nodeSize, schema.nodes.paragraph.createAndFill());
    return result;
  }

  grid.rows.forEach(function (row) {
    var expected = cellType(schema, row.header);
    row.cells.forEach(function (cell) {
      if (cell.node.type !== expected) {
        result = result || state.tr;
        result.setNodeMarkup(start + cell.offset, expected, cell.node.attrs);
      }
    });
  });

  // Bottom-up: the cell type fixes above never move positions.
  for (var index = grid.rows.length - 1; index >= 0; index--) {
    var row = grid.rows[index];
    var missing = grid.width - row.cells.length;
    if (missing <= 0) {
      continue;
    }

    var type = cellType(schema, row.header);
    var cells = [];
    for (var offset = 0; offset < missing; offset++) {
      var header = grid.rows[0] && grid.rows[0].cells[row.cells.length + offset];
      cells.push(type.createAndFill({ alignment: header ? header.node.attrs.alignment : null }));
    }

    result = result || state.tr;
    result.insert(start + row.offset + row.node.nodeSize - 1, Fragment.from(cells));
  }

  return result;
}

export function fixTables(state, tr, from, to) {
  if (from === undefined) {
    from = 0;
  }
  if (to === undefined) {
    to = state.doc.content.size;
  }
  var tables = [];
  state.doc.nodesBetween(from, to, function (node, pos) {
    if (!node.isBlock) {
      return false;
    }
    if (isTable(node)) {
      tables.push({ node: node, pos: pos });
      return false;
    }
  });

  var result = tr;
  // Bottom-up, so positions read from the untouched document stay valid.
  for (var index = tables.length - 1; index >= 0; index--) {
    result = fixTable(state, tables[index], result);
  }
  return result;
}
