// The table's action menus (static/prosemirror/lib/table/menu.js): a row's
// or a column's from its grip, and the caret's cell menu from the keyboard
// (Shift+F10, the ContextMenu key or Alt+Enter). components/composer/
// node-menu.gjs (a DDropdownMenu of DButtons) is drawn here as plain DOM in
// a float-kit menu (Discourse.float.menu, static/js/float.js).
import {
  addColumn,
  addHeaderRow,
  addRow,
  ALIGNMENTS,
  clearCellContents,
  columnAlignment,
  currentCell,
  deleteColumn,
  deleteRow,
  deleteTable,
  duplicateColumn,
  duplicateRow,
  moveColumn,
  moveRow,
  runCommand,
  setColumnAlignment,
} from "discourse/rich-editor/table/commands";
import { cellTarget, columnTarget, rowTarget } from "discourse/rich-editor/table/grid";
import { iconHtml } from "discourse/rich-editor/toolbar";

export var TABLE_MENU_IDENTIFIER = "composer-table-menu";

var ALIGNMENT_ICONS = {
  left: "align-left",
  center: "align-center",
  right: "align-right",
};

var triggerSequence = 0;
var shown = null;

// The table menu shown last, open or closed: the menu service's
// getByIdentifier, which Discourse.float.close does not reach for a menu
// shown as a bottom sheet.
export function shownMenu() {
  return shown;
}

// Each editor view's plugin params, for cellMenuItems called with the view
// alone (the reference reads its strings from a global i18n).
var viewParams = new WeakMap();

export function registerTableView(view, pluginParams) {
  viewParams.set(view, pluginParams);
}

// The a11y service's announce, where the host gives one.
export function announce(pluginParams, message) {
  var a11y = pluginParams.getContext().a11y;
  if (a11y && message) {
    a11y.announce(message);
  }
}

function i18n(pluginParams, key, params) {
  var context = pluginParams.getContext();
  return context.i18n ? context.i18n(key, params) : key;
}

function isRtl(view) {
  return getComputedStyle(view.dom).direction === "rtl";
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// NodeMenu
function menuContent(items, run) {
  var list = document.createElement("ul");
  list.className = "dropdown-menu " + TABLE_MENU_IDENTIFIER;

  items.forEach(function (item) {
    var li = document.createElement("li");
    if (item.divider) {
      li.innerHTML = '<hr class="dropdown-menu__divider">';
      list.appendChild(li);
      return;
    }
    li.className = "dropdown-menu__item";
    var button = document.createElement("button");
    var classes = [item.className];
    if (item.active) {
      classes.push("--active");
    }
    if (item.dangerous) {
      classes.push("--dangerous");
    }
    classes.push("btn", "btn-icon-text");
    button.className = classes.join(" ");
    button.type = "button";
    if (item.active === true || item.active === false) {
      button.setAttribute("aria-pressed", String(item.active));
    }
    button.innerHTML = iconHtml(item.icon) + escapeHtml(item.label);
    button.addEventListener("click", function (event) {
      event.preventDefault();
      event.stopPropagation();
      run(item);
    });
    li.appendChild(button);
    list.appendChild(li);
  });

  return list;
}

function showMenu(pluginParams, view, trigger, items, options) {
  if (!view.editable) {
    return null;
  }

  var instance = null;
  var isElement = trigger instanceof HTMLElement;

  if (isElement) {
    if (!trigger.id) {
      trigger.id = TABLE_MENU_IDENTIFIER + "-trigger-" + ++triggerSequence;
    }
    trigger.setAttribute("aria-expanded", "false");
  }

  var run = function (item) {
    if (instance) {
      instance.close();
    }
    if (item.action() && item.announcement) {
      announce(pluginParams, item.announcement);
    }
  };

  instance = window.Discourse.float.menu(trigger, {
    identifier: TABLE_MENU_IDENTIFIER,
    content: menuContent(items, run),
    placement: options.placement,
    contentRole: "none",
    // The mobile composer sits above the composer dropdown layer, and a
    // sheet is a better touch target than a dropdown on a thin grip.
    modalForMobile: true,
    autofocus: true,
    trapTab: true,
    onClose: function () {
      if (isElement) {
        trigger.setAttribute("aria-expanded", "false");
      }
      view.focus();
    },
  });

  if (instance && instance.expanded && isElement) {
    trigger.setAttribute("aria-expanded", "true");
  }

  shown = instance;
  return instance;
}

function item(pluginParams, attrs) {
  return Object.assign({}, attrs, {
    label: i18n(pluginParams, attrs.label),
    announcement: attrs.announcement && i18n(pluginParams, attrs.announcement),
  });
}

function itemsFor(view, pluginParams) {
  var make = function (attrs) {
    return item(pluginParams, attrs);
  };

  return {
    insertRowAbove: function (target) {
      return make({
        icon: "arrow-up",
        label: "composer.table.insert_row_above",
        className: "composer-table-menu__insert-above",
        announcement: "composer.table.row_inserted",
        action: function () {
          return runCommand(view, addRow(-1, target));
        },
      });
    },
    insertHeaderAbove: function (target) {
      return make({
        icon: "arrow-up",
        label: "composer.table.insert_header_above",
        className: "composer-table-menu__insert-header-above",
        announcement: "composer.table.header_inserted",
        action: function () {
          return runCommand(view, addHeaderRow(target));
        },
      });
    },
    insertRowBelow: function (target) {
      return make({
        icon: "arrow-down",
        label: "composer.table.insert_row_below",
        className: "composer-table-menu__insert-below",
        announcement: "composer.table.row_inserted",
        action: function () {
          return runCommand(view, addRow(1, target));
        },
      });
    },
    insertColumnLeft: function (target) {
      return make({
        icon: "arrow-left",
        label: "composer.table.insert_column_left",
        className: "composer-table-menu__insert-left",
        announcement: "composer.table.column_inserted",
        action: function () {
          return runCommand(view, addColumn(isRtl(view) ? 1 : -1, target));
        },
      });
    },
    insertColumnRight: function (target) {
      return make({
        icon: "arrow-right",
        label: "composer.table.insert_column_right",
        className: "composer-table-menu__insert-right",
        announcement: "composer.table.column_inserted",
        action: function () {
          return runCommand(view, addColumn(isRtl(view) ? -1 : 1, target));
        },
      });
    },
    duplicateRow: function (target) {
      return make({
        icon: "copy",
        label: "composer.table.duplicate_row",
        className: "composer-table-menu__duplicate-row",
        announcement: "composer.table.row_duplicated",
        action: function () {
          return runCommand(view, duplicateRow(target));
        },
      });
    },
    duplicateColumn: function (target) {
      return make({
        icon: "copy",
        label: "composer.table.duplicate_column",
        className: "composer-table-menu__duplicate-column",
        announcement: "composer.table.column_duplicated",
        action: function () {
          return runCommand(view, duplicateColumn(target));
        },
      });
    },
    moveRowUp: function (row, target) {
      return make({
        icon: "arrow-up",
        label: "composer.table.move_row_up",
        className: "composer-table-menu__move-row-up",
        announcement: "composer.table.row_moved_up",
        action: function () {
          return runCommand(view, moveRow(row, row - 1, target));
        },
      });
    },
    moveRowDown: function (row, target) {
      return make({
        icon: "arrow-down",
        label: "composer.table.move_row_down",
        className: "composer-table-menu__move-row-down",
        announcement: "composer.table.row_moved_down",
        action: function () {
          return runCommand(view, moveRow(row, row + 2, target));
        },
      });
    },
    moveColumnLeft: function (col, target) {
      return make({
        icon: "arrow-left",
        label: "composer.table.move_column_left",
        className: "composer-table-menu__move-column-left",
        announcement: "composer.table.column_moved_left",
        action: function () {
          return runCommand(view, moveColumn(col, isRtl(view) ? col + 2 : col - 1, target));
        },
      });
    },
    moveColumnRight: function (col, target) {
      return make({
        icon: "arrow-right",
        label: "composer.table.move_column_right",
        className: "composer-table-menu__move-column-right",
        announcement: "composer.table.column_moved_right",
        action: function () {
          return runCommand(view, moveColumn(col, isRtl(view) ? col - 1 : col + 2, target));
        },
      });
    },
    clearContents: function (target) {
      return make({
        icon: "circle-xmark",
        label: "composer.table.clear_contents",
        className: "composer-table-menu__clear-contents",
        announcement: "composer.table.contents_cleared",
        action: function () {
          return runCommand(view, clearCellContents(target));
        },
      });
    },
    deleteRow: function (target) {
      return make({
        icon: "trash-can",
        label: "composer.table.delete_row",
        className: "composer-table-menu__delete-row",
        announcement: "composer.table.row_deleted",
        dangerous: true,
        action: function () {
          return runCommand(view, deleteRow(target));
        },
      });
    },
    deleteColumn: function (target) {
      return make({
        icon: "trash-can",
        label: "composer.table.delete_column",
        className: "composer-table-menu__delete-column",
        announcement: "composer.table.column_deleted",
        dangerous: true,
        action: function () {
          return runCommand(view, deleteColumn(target));
        },
      });
    },
    deleteTable: function (table) {
      return make({
        icon: "trash-can",
        label: "composer.table.delete_table",
        className: "composer-table-menu__delete-table",
        announcement: "composer.table.table_deleted",
        dangerous: true,
        action: function () {
          return runCommand(view, deleteTable(table));
        },
      });
    },
    alignment: function (alignment, target) {
      return ALIGNMENTS.map(function (value) {
        return {
          icon: ALIGNMENT_ICONS[value],
          label: i18n(pluginParams, "composer.table.align_" + value),
          className: "composer-table-menu__align-" + value,
          active: alignment === value,
          announcement: i18n(pluginParams, "composer.table.alignment_changed", {
            alignment: i18n(pluginParams, "composer.table.alignment_" + value),
          }),
          action: function () {
            return runCommand(view, setColumnAlignment(alignment === value ? null : value, target));
          },
        };
      });
    },
  };
}

function insertRowAboveItems(items, table, row, target) {
  return table.grid.rows[row].header ? [items.insertHeaderAbove(target)] : [items.insertRowAbove(target)];
}

function rowMoveItems(items, table, row, target) {
  var out = [];
  if (row > 0) {
    out.push(items.moveRowUp(row, target));
  }
  if (row < table.grid.height - 1) {
    out.push(items.moveRowDown(row, target));
  }
  return out;
}

function columnMoveItems(view, items, table, col, target) {
  var last = table.grid.width - 1;
  var rtl = isRtl(view);
  var out = [];
  if (rtl ? col < last : col > 0) {
    out.push(items.moveColumnLeft(col, target));
  }
  if (rtl ? col > 0 : col < last) {
    out.push(items.moveColumnRight(col, target));
  }
  return out;
}

function rowMenuItems(view, pluginParams, at) {
  var items = itemsFor(view, pluginParams);
  var target = rowTarget(at.table, at.row);

  return []
    .concat(insertRowAboveItems(items, at.table, at.row, target))
    .concat([items.insertRowBelow(target), items.duplicateRow(target)])
    .concat(rowMoveItems(items, at.table, at.row, target))
    .concat([items.clearContents(target), { divider: true }, items.deleteRow(target)]);
}

function columnMenuItems(view, pluginParams, at) {
  var items = itemsFor(view, pluginParams);
  var target = columnTarget(at.table, at.col);

  return []
    .concat([items.insertColumnLeft(target), items.insertColumnRight(target), items.duplicateColumn(target)])
    .concat(columnMoveItems(view, items, at.table, at.col, target))
    .concat([{ divider: true }])
    .concat(items.alignment(columnAlignment(at.table, at.col), target))
    .concat([{ divider: true }, items.clearContents(target), items.deleteColumn(target)]);
}

// Everything that applies to the caret's cell. The only menu reachable
// without a pointer, so it covers both axes.
export function cellMenuItems(view, at, pluginParams) {
  pluginParams = pluginParams || viewParams.get(view);
  var items = itemsFor(view, pluginParams);
  var table = at.table;
  var cell = cellTarget(table, at.row, at.col);
  var column = columnTarget(table, at.col);
  var rowBand = rowTarget(table, at.row);

  return []
    .concat(insertRowAboveItems(items, table, at.row, rowBand))
    .concat([
      items.insertRowBelow(rowBand),
      items.insertColumnLeft(column),
      items.insertColumnRight(column),
      { divider: true },
      items.duplicateRow(rowBand),
      items.duplicateColumn(column),
    ])
    .concat(rowMoveItems(items, table, at.row, rowBand))
    .concat(columnMoveItems(view, items, table, at.col, column))
    .concat([{ divider: true }])
    .concat(items.alignment(columnAlignment(table, at.col), column))
    .concat([
      { divider: true },
      items.clearContents(cell),
      items.deleteRow(rowBand),
      items.deleteColumn(column),
      items.deleteTable(table),
    ]);
}

export function openGripMenu(gripContext, target) {
  var view = gripContext.view;
  if (!view.editable) {
    return null;
  }

  var items =
    gripContext.kind === "row"
      ? rowMenuItems(view, gripContext.pluginParams, target)
      : columnMenuItems(view, gripContext.pluginParams, target);

  return showMenu(gripContext.pluginParams, view, gripContext.grip, items, {
    placement: gripContext.kind === "row" ? "right-start" : "bottom-start",
  });
}

// What a context menu opened now would act on: { table, row, col } or null.
export function menuTargetFor(state) {
  var table = currentCell(state);
  if (!table) {
    return null;
  }
  return { table: table, row: table.rect.top, col: table.rect.left };
}

// Shift+F10 and the ContextMenu key open the menu of what is focused; macOS
// has neither by default, so Alt+Enter means the same there.
function opensTableMenu(event) {
  return (
    event.key === "ContextMenu" ||
    (event.key === "F10" && event.shiftKey) ||
    (event.key === "Enter" && event.altKey && !event.ctrlKey && !event.metaKey)
  );
}

export function handleContextMenuKey(view, event, pluginParams) {
  if (!view.editable || !opensTableMenu(event)) {
    return false;
  }

  var target = menuTargetFor(view.state);
  if (!target) {
    return false;
  }

  event.preventDefault();

  var table = target.table;
  var anchor = view.nodeDOM(table.start + table.grid.rows[target.row].cells[target.col].offset) || view.dom;

  showMenu(pluginParams, view, anchor, cellMenuItems(view, target, pluginParams), { placement: "bottom-start" });

  return true;
}
