// The table's structural commands (static/prosemirror/lib/table/commands.js).
// A target is a table (findTable) with a rect of rows and columns; without
// one a command acts on the table and cell the caret is in.
import { Fragment } from "prosemirror-model";
import { TextSelection } from "prosemirror-state";
import {
  cellAround,
  cellCoords,
  cellType,
  copyCell,
  findTable,
  rowRange,
  tableGrid,
} from "discourse/rich-editor/table/grid";

export function runCommand(view, command) {
  if (!view.editable) {
    return false;
  }
  var handled = command(view.state, view.dispatch);
  if (handled) {
    view.focus();
  }
  return handled;
}

export function selectCell(tr, tablePos, row, col) {
  var grid = tableGrid(tr.doc.nodeAt(tablePos));
  var cell = grid.rows[row] && grid.rows[row].cells[col];
  return cell ? tr.setSelection(TextSelection.create(tr.doc, tablePos + cell.offset + 2)) : tr;
}

export var ALIGNMENTS = ["left", "center", "right"];

// The table and cell holding the start of the text selection, or null
// outside a table.
export function currentCell(state) {
  var $cell = cellAround(state.selection.$from);
  if (!$cell) {
    return null;
  }

  var table = findTable($cell);
  if (!table) {
    return null;
  }

  var coords = cellCoords(table.grid, $cell.pos - table.start);
  if (!coords) {
    return null;
  }

  return Object.assign({}, table, {
    rect: { top: coords.row, bottom: coords.row, left: coords.col, right: coords.col },
  });
}

// A command over `target`, or the caret's table. `apply` builds the
// transaction (null dispatches nothing); `enabled` may reject the command
// first. A target whose table has since moved or changed is rejected.
function tableCommand(target, apply, enabled) {
  return function (state, dispatch) {
    var table = target || currentCell(state);
    if (
      !table ||
      table.pos > state.doc.content.size ||
      state.doc.nodeAt(table.pos) !== table.node ||
      (enabled && enabled(table) === false)
    ) {
      return false;
    }

    if (dispatch) {
      var tr = apply(state.tr, table);
      if (tr) {
        dispatch(tr);
      }
    }
    return true;
  };
}

function isEmptyCell(cell) {
  return !cell || cell.node.content.size === 0;
}

// Trailing columns whose every cell is empty, so they can go without losing
// anything. One column always stays.
export function emptyTrailingColumns(grid) {
  var count = 0;
  for (var col = grid.width - 1; col > 0; col--) {
    var empty = grid.rows.every(function (row) {
      return isEmptyCell(row.cells[col]);
    });
    if (!empty) {
      break;
    }
    count++;
  }
  return count;
}

// The same for body rows. A header-only table is still a table.
export function emptyTrailingRows(grid) {
  var body = grid.rows.filter(function (row) {
    return !row.header;
  });
  var count = 0;
  for (var index = body.length - 1; index >= 0; index--) {
    if (!body[index].cells.every(isEmptyCell)) {
      break;
    }
    count++;
  }
  return count;
}

export function columnAlignment(table, col) {
  var row = table.grid.rows[0];
  var cell = row && row.cells[col];
  return cell ? cell.node.attrs.alignment : null;
}

function headOffset(grid) {
  return grid.head ? 1 : 0;
}

function forEachRowBottomUp(table, f) {
  for (var index = table.grid.rows.length - 1; index >= 0; index--) {
    f(table.grid.rows[index], index);
  }
}

function times(count, f) {
  var out = [];
  for (var i = 0; i < count; i++) {
    out.push(f(i));
  }
  return out;
}

function insertColumn(tr, table, col, count) {
  var schema = table.node.type.schema;

  forEachRowBottomUp(table, function (row) {
    var last = row.cells[row.cells.length - 1];
    var at =
      col >= row.cells.length
        ? table.start + last.offset + last.node.nodeSize
        : table.start + row.cells[col].offset;

    var cells = times(count || 1, function () {
      return cellType(schema, row.header).createAndFill();
    });
    tr.insert(at, Fragment.from(cells));
  });

  return tr;
}

// Where a body row inserted at `bodyIndex` would start.
function bodyInsertPos(table, bodyIndex) {
  var start = table.start;
  var grid = table.grid;
  var rows = grid.rows.filter(function (row) {
    return !row.header;
  });

  if (!rows.length) {
    return start + grid.body.offset + 1;
  }
  if (bodyIndex >= rows.length) {
    var last = rows[rows.length - 1];
    return start + last.offset + last.node.nodeSize;
  }
  return start + rows[Math.max(bodyIndex, 0)].offset;
}

// Inserts empty body rows; returns where they start.
function insertRow(tr, table, bodyIndex, count) {
  var schema = table.node.type.schema;
  var cells = times(table.grid.width, function (col) {
    return cellType(schema, false).createAndFill({ alignment: columnAlignment(table, col) });
  });

  var pos = bodyInsertPos(table, bodyIndex);
  var rows = times(count || 1, function () {
    return schema.nodes.table_row.create(null, Fragment.from(cells));
  });
  tr.insert(pos, Fragment.from(rows));
  return pos;
}

function removeRows(tr, table, top, bottom) {
  var grid = table.grid;
  var start = table.start;
  var removingHeader = !!grid.head && top === 0;
  var promoted = removingHeader ? grid.rows[bottom + 1] : null;

  if (removingHeader && !promoted) {
    return tr.delete(table.pos, table.pos + table.node.nodeSize);
  }

  // Highest position first, so the offsets read from the original grid hold.
  if (promoted) {
    tr.delete(start + promoted.offset, start + promoted.offset + promoted.node.nodeSize);
  }

  for (var index = bottom; index >= Math.max(top, headOffset(grid)); index--) {
    var row = grid.rows[index];
    tr.delete(start + row.offset, start + row.offset + row.node.nodeSize);
  }

  if (promoted) {
    var schema = table.node.type.schema;
    var headRow = grid.rows[0];
    var cells = [];
    promoted.node.forEach(function (cell) {
      cells.push(copyCell(cellType(schema, true), cell));
    });

    tr.replaceWith(
      start + headRow.offset + 1,
      start + headRow.offset + headRow.node.nodeSize - 1,
      Fragment.from(cells)
    );
  }

  return tr;
}

function selectCellContent(tr, pos) {
  var cell = tr.doc.nodeAt(pos);
  if (!cell) {
    return tr;
  }
  return tr.setSelection(
    TextSelection.between(tr.doc.resolve(pos + 1), tr.doc.resolve(pos + cell.nodeSize - 1))
  );
}

export function addColumn(side, target, count) {
  return tableCommand(target, function (tr, table) {
    return insertColumn(tr, table, side < 0 ? table.rect.left : table.rect.right + 1, count).scrollIntoView();
  });
}

export function deleteColumn(target) {
  return tableCommand(target, function (tr, table) {
    var left = table.rect.left;
    var right = table.rect.right;

    if (right - left + 1 >= table.grid.width) {
      return tr.delete(table.pos, table.pos + table.node.nodeSize);
    }

    forEachRowBottomUp(table, function (row) {
      for (var col = right; col >= left; col--) {
        var cell = row.cells[col];
        if (cell) {
          tr.delete(table.start + cell.offset, table.start + cell.offset + cell.node.nodeSize);
        }
      }
    });

    return tr;
  });
}

export function duplicateColumn(target) {
  return tableCommand(target, function (tr, table) {
    var left = table.rect.left;
    var right = table.rect.right;

    forEachRowBottomUp(table, function (row) {
      var copies = [];
      for (var col = left; col <= right; col++) {
        var cell = row.cells[col].node;
        copies.push(copyCell(cell.type, cell));
      }
      var ref = row.cells[right];
      tr.insert(table.start + ref.offset + ref.node.nodeSize, Fragment.from(copies));
    });

    return tr.scrollIntoView();
  });
}

// `from` is the column moved, `to` its insertion index in the unmoved grid.
export function moveColumn(from, to, tableTarget) {
  return tableCommand(
    tableTarget,
    function (tr, table) {
      var target = to > from ? to - 1 : to;

      forEachRowBottomUp(table, function (row) {
        var cells = row.cells.map(function (cell) {
          return cell.node;
        });
        var moved = cells.splice(from, 1)[0];
        cells.splice(target, 0, moved);
        tr.replaceWith(
          table.start + row.offset + 1,
          table.start + row.offset + row.node.nodeSize - 1,
          Fragment.from(cells)
        );
      });

      return tr;
    },
    function () {
      return to !== from && to !== from + 1;
    }
  );
}

export function setColumnAlignment(alignment, target) {
  return tableCommand(target, function (tr, table) {
    table.grid.rows.forEach(function (row) {
      for (var col = table.rect.left; col <= table.rect.right; col++) {
        var cell = row.cells[col];
        if (cell) {
          tr.setNodeMarkup(
            table.start + cell.offset,
            null,
            Object.assign({}, cell.node.attrs, { alignment: alignment })
          );
        }
      }
    });
    return tr;
  });
}

export function addRow(side, target, count) {
  return tableCommand(
    target,
    function (tr, table) {
      var offset = headOffset(table.grid);
      var index = side < 0 ? Math.max(table.rect.top - offset, 0) : table.rect.bottom - offset + 1;
      var pos = insertRow(tr, table, index, count);
      return selectCellContent(tr, pos + 1).scrollIntoView();
    },
    function (table) {
      return !!table.grid.body;
    }
  );
}

export function deleteRow(target) {
  return tableCommand(target, function (tr, table) {
    return removeRows(tr, table, table.rect.top, table.rect.bottom);
  });
}

export function duplicateRow(target) {
  return tableCommand(
    target,
    function (tr, table) {
      var top = table.rect.top;
      var bottom = table.rect.bottom;
      var schema = table.node.type.schema;
      var copies = [];

      for (var index = top; index <= bottom; index++) {
        var row = table.grid.rows[index];
        var cells = [];
        row.node.forEach(function (cell) {
          // A duplicated header row is a regular row: markdown has only one.
          var type = row.header ? schema.nodes.table_cell : cell.type;
          cells.push(copyCell(type, cell));
        });
        copies.push(schema.nodes.table_row.create(null, Fragment.from(cells)));
      }

      tr.insert(bodyInsertPos(table, bottom - headOffset(table.grid) + 1), Fragment.from(copies));
      return tr.scrollIntoView();
    },
    function (table) {
      return !!table.grid.body;
    }
  );
}

// Both sections replaced together, so promoting a header stays schema-valid.
function replaceRows(tr, table, order) {
  var schema = table.node.type.schema;

  var rows = order.map(function (row, index) {
    var type = cellType(schema, index === 0);
    var cells = [];
    row.forEach(function (cell) {
      cells.push(copyCell(type, cell));
    });
    return schema.nodes.table_row.create(null, Fragment.from(cells));
  });

  var replacement = schema.nodes.table.create(table.node.attrs, [
    schema.nodes.table_head.create(null, rows[0]),
    schema.nodes.table_body.create(null, Fragment.from(rows.slice(1))),
  ]);

  return tr.replaceWith(table.pos, table.pos + table.node.nodeSize, replacement);
}

function reorderAcrossHeader(tr, table, from, to) {
  var order = table.grid.rows.map(function (row) {
    return row.node;
  });
  var moved = order.splice(from, 1)[0];
  var landing = to > from ? to - 1 : to;
  order.splice(landing, 0, moved);

  return selectCell(replaceRows(tr, table, order), table.pos, landing, 0);
}

// `from` is the row moved, `to` its insertion index in the unmoved grid.
export function moveRow(from, to, tableTarget) {
  return tableCommand(
    tableTarget,
    function (tr, table) {
      // Header status belongs to the first row, not the moved content.
      if (table.grid.head && (from === 0 || to === 0)) {
        return reorderAcrossHeader(tr, table, from, to);
      }

      var range = rowRange(table, from);
      var target = bodyInsertPos(table, to - headOffset(table.grid));

      tr.delete(range.from, range.to);
      tr.insert(tr.mapping.map(target), table.grid.rows[from].node);
      return tr;
    },
    function (table) {
      return to !== from && to !== from + 1 && !!table.grid.rows[from];
    }
  );
}

// A new header row; the old one goes to the body.
export function addHeaderRow(target) {
  return tableCommand(
    target,
    function (tr, table) {
      var schema = table.node.type.schema;
      var cells = times(table.grid.width, function (col) {
        return cellType(schema, true).createAndFill({ alignment: columnAlignment(table, col) });
      });

      var header = schema.nodes.table_row.create(null, Fragment.from(cells));
      var order = [header].concat(
        table.grid.rows.map(function (row) {
          return row.node;
        })
      );

      // The head's own row holds the first cell two positions in.
      return selectCellContent(replaceRows(tr, table, order), table.start + 2).scrollIntoView();
    },
    function (table) {
      return !!table.grid.head;
    }
  );
}

export function deleteTable(target) {
  return tableCommand(target, function (tr, table) {
    return tr.delete(table.pos, table.pos + table.node.nodeSize);
  });
}

export function goToNextCell(dir) {
  return function (state, dispatch) {
    var table = currentCell(state);
    if (!table) {
      return false;
    }

    var grid = table.grid;
    var rect = table.rect;
    var index = rect.top * grid.width + rect.left + dir;

    if (index < 0) {
      return false;
    }

    if (index >= grid.width * grid.height) {
      if (dir < 0 || !grid.body) {
        return false;
      }
      if (dispatch) {
        var tr = state.tr;
        var pos = insertRow(tr, table, grid.height - headOffset(grid));
        dispatch(selectCellContent(tr, pos + 1).scrollIntoView());
      }
      return true;
    }

    if (dispatch) {
      var row = Math.floor(index / grid.width);
      var col = index % grid.width;
      dispatch(selectCellContent(state.tr, table.start + grid.rows[row].cells[col].offset).scrollIntoView());
    }
    return true;
  };
}

export function clearCellContents(target) {
  return tableCommand(target, function (tr, table) {
    var cells = [];
    for (var row = table.rect.top; row <= table.rect.bottom; row++) {
      for (var col = table.rect.left; col <= table.rect.right; col++) {
        var cell = table.grid.rows[row] && table.grid.rows[row].cells[col];
        if (cell) {
          cells.push({ node: cell.node, pos: table.start + cell.offset });
        }
      }
    }

    for (var index = cells.length - 1; index >= 0; index--) {
      if (cells[index].node.content.size) {
        tr.delete(cells[index].pos + 1, cells[index].pos + cells[index].node.nodeSize - 1);
      }
    }

    return tr.docChanged ? tr : null;
  });
}
