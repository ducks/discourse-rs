// extensions/table.js: markdown's pipe tables, with their cell alignment,
// the editing plugin (Tab, pasted tables, repairs, selections across cells),
// the grips and menus, and the table and cell node views (table/).
//
// | Left-aligned | Center-aligned | Right-aligned |
// | :---         |     :---:      |          ---: |
// | git status   | git status     | git status    |
import { tableChrome } from "discourse/rich-editor/table/chrome";
import tableEditing from "discourse/rich-editor/table/editing";
import { buildTableNodeView, TableCellView } from "discourse/rich-editor/table/node-view";

function findMaxColumns(tbody) {
  var maxColumns = 0;
  tbody.forEach(function (row) {
    maxColumns = Math.max(maxColumns, row.childCount);
  });
  return maxColumns;
}

function createHeaderRow(firstRow, maxColumns, schema) {
  var headerCells = [];
  for (var i = 0; i < maxColumns; i++) {
    if (i < firstRow.childCount) {
      var cell = firstRow.child(i);
      headerCells.push(schema.nodes.table_header_cell.create(cell.attrs, cell.content));
    } else {
      headerCells.push(schema.nodes.table_header_cell.create());
    }
  }
  return schema.nodes.table_row.create({}, headerCells);
}

function createBodyRows(tbody, startIndex, maxColumns, schema) {
  return tbody.content.content.slice(startIndex).map(function (row) {
    var cells = [];
    for (var i = 0; i < maxColumns; i++) {
      cells.push(i < row.childCount ? row.child(i) : schema.nodes.table_cell.create());
    }
    return schema.nodes.table_row.create({}, cells);
  });
}

// Pasted and quoted HTML often carries empty tables. Whitespace and line
// breaks don't count as content.
function hasContent(table) {
  var found = false;
  table.descendants(function (node) {
    if (found) {
      return false;
    }
    if (node.isText) {
      found = node.text.trim() !== "";
    } else if (node.isInline && node.isLeaf) {
      found = node.type.name !== "hard_break";
    }
  });
  return found;
}

// A table without a head takes its first body row as one.
function normalizeTable(tableNode) {
  var schema = tableNode.type.schema;
  var tbody;
  var thead;

  tableNode.descendants(function (node) {
    if (node.type.name === "table_body") {
      tbody = node;
      return false;
    }
    if (node.type.name === "table_head") {
      thead = node;
      return false;
    }
  });

  if (thead || !tbody) {
    return tableNode;
  }

  var maxColumns = findMaxColumns(tbody);
  var firstRow = tbody.firstChild;

  if (!firstRow || maxColumns === 0) {
    return tableNode;
  }

  var header = schema.nodes.table_head.create({}, createHeaderRow(firstRow, maxColumns, schema));
  var body = schema.nodes.table_body.create({}, createBodyRows(tbody, 1, maxColumns, schema));
  return schema.nodes.table.create({}, [header, body]);
}

function cellAlignment(token) {
  var style = token.attrGet("style");
  var match = style && style.match(/text-align:(\w+)/);
  return match ? match[1] : undefined;
}

function cellDOM(tag) {
  return function (node) {
    return [tag, { style: node.attrs.alignment ? "text-align: " + node.attrs.alignment : undefined }, 0];
  };
}

function hasTableNodes(fragment) {
  var found = false;
  fragment.descendants(function (node) {
    if (node.type.name === "table") {
      found = true;
      return false;
    }
  });
  return found;
}

export default {
  nodeViews: {
    table: buildTableNodeView,
    table_cell: TableCellView,
    table_header_cell: TableCellView,
  },
  nodeSpec: {
    table: {
      content: "table_head? table_body",
      group: "block",
      tableRole: "table",
      isolating: true,
      selectable: true,
      draggable: true,
      parseDOM: [{ tag: "table" }],
      toDOM: function () {
        return ["div", { class: "md-table" }, ["table", 0]];
      },
    },
    table_head: {
      content: "table_row",
      tableRole: "head",
      isolating: true,
      parseDOM: [{ tag: "thead" }],
      toDOM: function () {
        return ["thead", 0];
      },
    },
    table_body: {
      content: "table_row*",
      tableRole: "body",
      isolating: true,
      parseDOM: [{ tag: "tbody" }],
      toDOM: function () {
        return ["tbody", 0];
      },
    },
    table_row: {
      content: "(table_cell | table_header_cell)+",
      tableRole: "row",
      // Isolating cells would otherwise open gap cursor positions between them.
      allowGapCursor: false,
      parseDOM: [{ tag: "tr" }],
      toDOM: function () {
        return ["tr", 0];
      },
    },
    table_header_cell: {
      content: "inline*",
      tableRole: "header_cell",
      isolating: true,
      attrs: { alignment: { default: null } },
      parseDOM: [
        {
          tag: "th",
          getAttrs: function (dom) {
            return { alignment: dom.style.textAlign };
          },
        },
      ],
      toDOM: cellDOM("th"),
    },
    table_cell: {
      content: "inline*",
      tableRole: "cell",
      isolating: true,
      attrs: { alignment: { default: null } },
      parseDOM: [
        {
          tag: "td",
          getAttrs: function (dom) {
            return { alignment: dom.style.textAlign };
          },
        },
      ],
      toDOM: cellDOM("td"),
    },
  },
  parse: {
    table: { block: "table" },
    thead: { block: "table_head" },
    tbody: { block: "table_body" },
    tr: { block: "table_row" },
    th: {
      block: "table_header_cell",
      getAttrs: function (token) {
        return { alignment: cellAlignment(token) };
      },
    },
    td: {
      block: "table_cell",
      getAttrs: function (token) {
        return { alignment: cellAlignment(token) };
      },
    },
  },
  serializeNode: {
    table: function (state, node) {
      // Checked before flushing, so the blocks around stay apart.
      if (!hasContent(node)) {
        return;
      }

      state.flushClose(1);

      node = normalizeTable(node);

      var prevInTable = state.inTable;
      state.inTable = true;

      // Keeps a table after a blockquote from being parsed as part of it.
      if (state.out) {
        state.out += "\n";
      }

      // Each group is table_head or table_body.
      var isFirstRow = true;
      node.forEach(function (group) {
        var isHead = group.type.name === "table_head";

        group.forEach(function (row, rowOffset, rowIndex) {
          var shouldTreatAsHeader =
            isHead ||
            (isFirstRow &&
              rowIndex === 0 &&
              row.childCount > 0 &&
              row.firstChild &&
              row.firstChild.type.name === "table_header_cell");
          var headerBuffer = shouldTreatAsHeader ? state.delim : undefined;

          row.forEach(function (cell, cellOffset, cellIndex) {
            if (state.delim && state.atBlank()) {
              state.out += state.delim;
            }
            state.out += cellIndex === 0 ? "| " : " | ";

            var cellStart = state.out.length;
            state.renderInline(cell);
            state.out = state.out.slice(0, cellStart) + state.out.slice(cellStart).replaceAll("|", "\\|");

            if (headerBuffer !== undefined) {
              if (cell.attrs.alignment === "center") {
                headerBuffer += "|:---:";
              } else if (cell.attrs.alignment === "left") {
                headerBuffer += "|:---";
              } else if (cell.attrs.alignment === "right") {
                headerBuffer += "|---:";
              } else {
                headerBuffer += "|----";
              }
            }
          });

          state.out += " |\n";

          if (headerBuffer !== undefined) {
            state.out += headerBuffer + "|\n";
          }
        });
        isFirstRow = false;
      });
      state.out += "\n";
      state.inTable = prevInTable;
    },
    table_head: function () {},
    table_body: function () {},
    table_row: function () {},
    table_header_cell: function () {},
    table_cell: function () {},
  },
  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var Slice = params.pmModel.Slice;
    var Fragment = params.pmModel.Fragment;

    var transformNode = function (node) {
      if (node.type.name === "table") {
        return normalizeTable(node);
      }
      if (node.content && node.content.size > 0) {
        return node.type.create(node.attrs, node.content.content.map(transformNode), node.marks);
      }
      return node;
    };

    var pastedTables = new Plugin({
      props: {
        transformPasted: function (paste) {
          if (!hasTableNodes(paste.content)) {
            return paste;
          }
          return new Slice(Fragment.from(paste.content.content.map(transformNode)), paste.openStart, paste.openEnd);
        },
      },
    });

    return [pastedTables, tableEditing(), tableChrome(params)];
  },
};
