// A table's geometry (static/prosemirror/lib/table/grid.js). Markdown tables
// never merge cells, so a normalized grid is a plain rectangle and a cell is
// addressed by (row, col) alone. Offsets are relative to the table's content
// start, which keeps the grid cacheable per node.
var CELL_ROLES = ["cell", "header_cell"];

var gridCache = new WeakMap();

export function isCell(node) {
  return !!node && CELL_ROLES.indexOf(node.type.spec.tableRole) >= 0;
}

export function isTable(node) {
  return !!node && node.type.spec.tableRole === "table";
}

export function cellType(schema, header) {
  return header ? schema.nodes.table_header_cell : schema.nodes.table_cell;
}

// The cell re-created as `type`, its content and marks carried over.
export function copyCell(type, cell, attrs) {
  return type.create(attrs === undefined ? cell.attrs : attrs, cell.content, cell.marks);
}

// { rows: [{ node, offset, header, cells: [{ node, offset }] }], head, body,
//   width, height }
export function tableGrid(table) {
  var cached = gridCache.get(table);
  if (cached) {
    return cached;
  }

  var rows = [];
  var head = null;
  var body = null;
  var sectionOffset = 0;

  table.forEach(function (section) {
    var header = section.type.spec.tableRole === "head";
    if (header) {
      head = { node: section, offset: sectionOffset };
    } else {
      body = { node: section, offset: sectionOffset };
    }

    var rowOffset = sectionOffset + 1;
    section.forEach(function (row) {
      var cells = [];
      var cellOffset = rowOffset + 1;
      row.forEach(function (cell) {
        cells.push({ node: cell, offset: cellOffset });
        cellOffset += cell.nodeSize;
      });
      rows.push({ node: row, offset: rowOffset, header: header, cells: cells });
      rowOffset += row.nodeSize;
    });

    sectionOffset += section.nodeSize;
  });

  var grid = {
    rows: rows,
    head: head,
    body: body,
    height: rows.length,
    width: rows.reduce(function (max, row) {
      return Math.max(max, row.cells.length);
    }, 0),
  };

  gridCache.set(table, grid);
  return grid;
}

// The table enclosing a resolved position: { node, pos, start, grid }.
export function findTable($pos) {
  for (var depth = $pos.depth; depth > 0; depth--) {
    var node = $pos.node(depth);
    if (isTable(node)) {
      return {
        node: node,
        pos: $pos.before(depth),
        start: $pos.start(depth),
        grid: tableGrid(node),
      };
    }
  }
  return null;
}

// The position before the cell enclosing $pos, resolved.
export function cellAround($pos) {
  for (var depth = $pos.depth; depth > 0; depth--) {
    if (isCell($pos.node(depth))) {
      return $pos.doc.resolve($pos.before(depth));
    }
  }
  return null;
}

export function cellCoords(grid, offset) {
  for (var row = 0; row < grid.rows.length; row++) {
    var col = grid.rows[row].cells.findIndex(function (cell) {
      return cell.offset === offset;
    });
    if (col !== -1) {
      return { row: row, col: col };
    }
  }
  return null;
}

export function rowRange(table, row) {
  var entry = table.grid.rows[row];
  return {
    from: table.start + entry.offset,
    to: table.start + entry.offset + entry.node.nodeSize,
  };
}

export function rectFor(grid, kind, index, through) {
  if (through === undefined) {
    through = index;
  }
  return kind === "row"
    ? { top: index, bottom: through, left: 0, right: grid.width - 1 }
    : { top: 0, bottom: grid.height - 1, left: index, right: through };
}

export function rowTarget(table, row, through) {
  return Object.assign({}, table, { rect: rectFor(table.grid, "row", row, through) });
}

export function columnTarget(table, col, through) {
  return Object.assign({}, table, { rect: rectFor(table.grid, "column", col, through) });
}

export function cellTarget(table, row, col) {
  return Object.assign({}, table, { rect: { top: row, bottom: row, left: col, right: col } });
}
