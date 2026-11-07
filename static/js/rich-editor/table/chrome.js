// The table's grips (static/prosemirror/lib/table/chrome.js): a widget in
// each row's first cell and each column's header cell, placed by CSS, that
// opens the row's or column's menu on a click and reorders it on a drag.
// A grip finds its row and column from its current position, so reused
// widgets stay right after edits.
import { Plugin, PluginKey } from "prosemirror-state";
import { Decoration, DecorationSet } from "prosemirror-view";
import { moveColumn, moveRow, runCommand } from "discourse/rich-editor/table/commands";
import dragAutoscroll from "discourse/rich-editor/table/drag-autoscroll";
import { cellAround, cellCoords, findTable, isTable, rectFor, tableGrid } from "discourse/rich-editor/table/grid";
import { announce, handleContextMenuKey, openGripMenu, registerTableView } from "discourse/rich-editor/table/menu";
import trackPointer from "discourse/rich-editor/table/track-pointer";

var DRAG_THRESHOLD = 4;
var TOUCH_DRAG_THRESHOLD = 12;
var TABLE_CHROME_KEY = new PluginKey("tableChrome");

function i18n(pluginParams, key, params) {
  var context = pluginParams.getContext();
  return context.i18n ? context.i18n(key, params) : key;
}

export function tableChrome(pluginParams) {
  var cache = new WeakMap();
  var activeGestures = new Set();
  var stopGestures = function () {
    Array.from(activeGestures).forEach(function (stop) {
      stop();
    });
  };

  return new Plugin({
    key: TABLE_CHROME_KEY,

    state: {
      init: function () {
        return null;
      },
      apply: function (tr, target) {
        var next = tr.getMeta(TABLE_CHROME_KEY);
        if (next !== undefined) {
          return next;
        }
        return tr.selectionSet || tr.docChanged ? null : target;
      },
    },

    props: {
      decorations: function (state) {
        return chromeDecorations(state, pluginParams, cache, activeGestures, TABLE_CHROME_KEY.getState(state));
      },
      handleKeyDown: function (view, event) {
        return handleContextMenuKey(view, event, pluginParams);
      },
    },

    view: function (view) {
      registerTableView(view, pluginParams);
      syncGripAccessibility(view, pluginParams);

      return {
        update: function (_, previousState) {
          if (!view.editable || view.state.doc !== previousState.doc) {
            stopGestures();
          }
          syncGripAccessibility(view, pluginParams);
        },
        destroy: stopGestures,
      };
    },
  });
}

function chromeDecorations(state, pluginParams, cache, activeGestures, target) {
  var set = cache.get(state.doc);
  if (!set) {
    set = gripDecorations(state, pluginParams, activeGestures);
    cache.set(state.doc, set);
  }

  var selected = targetDecorations(state, target);
  return selected.length ? set.add(state.doc, selected) : set;
}

function gripDecorations(state, pluginParams, activeGestures) {
  var decorations = [];

  state.doc.descendants(function (node, pos) {
    if (node.isTextblock || !node.isBlock) {
      return false;
    }
    if (!isTable(node)) {
      return;
    }

    var grid = tableGrid(node);
    var start = pos + 1;

    grid.rows.forEach(function (row, index) {
      if (row.cells[0]) {
        decorations.push(gripDecoration(start + row.cells[0].offset + 1, "row", pluginParams, activeGestures));
      }

      if (index === 0) {
        row.cells.forEach(function (cell) {
          decorations.push(gripDecoration(start + cell.offset + 1, "column", pluginParams, activeGestures));
        });
      }
    });

    return false;
  });

  return DecorationSet.create(state.doc, decorations);
}

function targetDecorations(state, target) {
  if (!target) {
    return [];
  }

  var node = state.doc.nodeAt(target.tablePos);
  if (!isTable(node)) {
    return [];
  }

  var grid = tableGrid(node);
  var rect = rectFor(grid, target.kind, target.index);
  var decorations = [];

  // The band is one row or column, so only its two ends along the axis need
  // marking; its long sides follow from the axis class.
  var alongRow = target.kind === "row";
  var first = alongRow ? rect.left : rect.top;
  var last = alongRow ? rect.right : rect.bottom;

  for (var row = rect.top; row <= rect.bottom; row++) {
    for (var col = rect.left; col <= rect.right; col++) {
      var cell = grid.rows[row] && grid.rows[row].cells[col];
      if (!cell) {
        continue;
      }

      var index = alongRow ? col : row;
      var classes = ["is-structural-target", "--axis-" + target.kind];

      if (index === first) {
        classes.push("--edge-start");
      }
      if (index === last) {
        classes.push("--edge-end");
      }

      var pos = target.tablePos + 1 + cell.offset;
      decorations.push(Decoration.node(pos, pos + cell.node.nodeSize, { class: classes.join(" ") }));
    }
  }

  return decorations;
}

function gripDecoration(pos, kind, pluginParams, activeGestures) {
  return Decoration.widget(
    pos,
    function (view, getPos) {
      return buildGrip(kind, view, getPos, pluginParams, activeGestures);
    },
    {
      side: -1,
      key: "table-grip-" + kind,
      ignoreSelection: true,
      stopEvent: function () {
        return true;
      },
    }
  );
}

function buildGrip(kind, view, getPos, pluginParams, activeGestures) {
  var grip = document.createElement("button");
  grip.type = "button";
  grip.className = "composer-table__grip --" + kind;
  grip.contentEditable = "false";
  grip.draggable = false;
  grip.tabIndex = -1;
  grip.setAttribute("aria-expanded", "false");
  var context = {
    kind: kind,
    view: view,
    getPos: getPos,
    pluginParams: pluginParams,
    activeGestures: activeGestures,
    grip: grip,
    dragged: false,
  };
  grip.addEventListener("mousedown", function (event) {
    event.preventDefault();
  });
  grip.addEventListener("pointerdown", function (event) {
    onGripPointerDown(event, context);
  });
  // Opens on click, so a touch's delayed click cannot land on the backdrop.
  grip.addEventListener("click", function (event) {
    onGripClick(event, context);
  });

  return grip;
}

function onGripPointerDown(event, context) {
  if (!context.view.editable || event.button !== 0) {
    return;
  }
  event.preventDefault();
  context.dragged = false;

  var target = locateGrip(context);
  if (!target) {
    return;
  }

  highlightAt(context.view, target, context.kind);
  trackDrag(event, context, target);
}

function onGripClick(event, context) {
  if (!context.view.editable) {
    return;
  }

  if (event.detail !== 0 && context.dragged) {
    return;
  }

  var target = locateGrip(context);
  if (!target) {
    return;
  }

  highlightAt(context.view, target, context.kind);
  openGripMenu(context, target);
}

// The table, row and column the grip belongs to now.
function locateGrip(context) {
  var pos = context.getPos();
  if (pos === undefined) {
    return null;
  }

  var $cell = cellAround(context.view.state.doc.resolve(pos));
  if (!$cell) {
    return null;
  }

  var table = findTable($cell);
  var coords = cellCoords(table.grid, $cell.pos - table.start);
  return coords ? { table: table, row: coords.row, col: coords.col } : null;
}

function highlightAt(view, target, kind) {
  view.dispatch(
    view.state.tr.setMeta(TABLE_CHROME_KEY, {
      tablePos: target.table.pos,
      kind: kind,
      index: kind === "row" ? target.row : target.col,
    })
  );
}

function trackDrag(startEvent, context, target) {
  var activeGestures = context.activeGestures;
  var kind = context.kind;
  var grip = context.grip;
  var view = context.view;
  var inner = grip.closest(".composer-table__inner");
  var indicator = inner && inner.querySelector(".composer-table__drop-indicator");
  var grid = inner && inner.querySelector("table");
  if (!inner || !grid) {
    return;
  }

  var axis = kind === "row" ? "Y" : "X";
  var origin = kind === "row" ? startEvent.clientY : startEvent.clientX;
  var index = kind === "row" ? target.row : target.col;
  var dragThreshold = startEvent.pointerType === "touch" ? TOUCH_DRAG_THRESHOLD : DRAG_THRESHOLD;

  // Measured once: the dragged row or column follows the pointer, so
  // measuring mid-drag would read its own offset back and chase itself.
  var spans = measureSpans(inner, grid, kind);
  var forward = spanDirection(spans);
  var moving =
    kind === "row"
      ? Array.from(grid.rows[index] ? grid.rows[index].cells : [])
      : Array.from(grid.rows)
          .map(function (row) {
            return row.cells[index];
          })
          .filter(Boolean);

  var dragging = false;
  var dropIndex = null;
  var current = origin;
  var avatar = null;
  var stopped = false;
  var stopTracking;

  var updateLanding = function () {
    var over = spanUnderPointer(spans, current, forward);
    dropIndex = over === index ? null : over > index ? over + 1 : over;
    positionIndicator(indicator, kind, spans, dropIndex, forward);
  };

  var autoscroll = dragAutoscroll(inner, axis, function (change) {
    spans.forEach(function (span) {
      span.start -= change;
      span.end -= change;
    });
    updateLanding();
  });

  var lift = function (offset) {
    moving.forEach(function (cell) {
      cell.style.transform = "translate" + axis + "(" + offset + "px)";
    });
    if (avatar) {
      avatar.style.transform = "translate" + axis + "(" + offset + "px)";
    }
  };

  var settle = function () {
    moving.forEach(function (cell) {
      cell.style.transform = "";
      cell.classList.remove("is-moving");
    });
    if (avatar) {
      avatar.remove();
    }
    avatar = null;
    grip.classList.remove("is-drag-origin");
    inner.classList.remove("is-reordering");
    if (indicator) {
      indicator.className = "composer-table__drop-indicator";
      indicator.removeAttribute("style");
    }
  };

  var stop = function () {
    if (stopped) {
      return;
    }
    stopped = true;
    stopTracking();
    activeGestures.delete(stop);
    autoscroll.stop();
    settle();
  };

  var move = function (event) {
    current = kind === "row" ? event.clientY : event.clientX;
    var offset = current - origin;

    if (!dragging && Math.abs(offset) < dragThreshold) {
      return;
    }

    event.preventDefault();
    if (!dragging) {
      dragging = true;
      context.dragged = true;
      inner.classList.add("is-reordering");
      moving.forEach(function (cell) {
        cell.classList.add("is-moving");
      });
      avatar = dragAvatar(grip, kind);
      grip.classList.add("is-drag-origin");
    }

    lift(offset);
    autoscroll.update(event);

    // Landing goes by the row or column under the pointer, not past its
    // middle: an index from midpoints needs a drag of one and a half
    // columns to move a column by one.
    updateLanding();
  };

  var finish = function (event) {
    var landed = event.type === "pointercancel" ? null : dropIndex;
    stop();

    if (dragging && landed !== null) {
      var command =
        kind === "row" ? moveRow(index, landed, target.table) : moveColumn(index, landed, target.table);
      if (runCommand(view, command)) {
        announceMove(context, index, landed);
      }
    }
  };

  stopTracking = trackPointer(startEvent, move, finish);
  activeGestures.add(stop);
}

function announceMove(context, index, landed) {
  var forward = landed > index;
  var key;

  if (context.kind === "row") {
    key = forward ? "row_moved_down" : "row_moved_up";
  } else {
    var rtl = getComputedStyle(context.view.dom).direction === "rtl";
    key = forward !== rtl ? "column_moved_right" : "column_moved_left";
  }

  announce(context.pluginParams, i18n(context.pluginParams, "composer.table." + key));
}

function syncGripAccessibility(view, pluginParams) {
  view.dom.querySelectorAll(".composer-table__grip, .composer-table__append").forEach(function (button) {
    button.hidden = !view.editable;
    button.disabled = !view.editable;
  });
  view.dom.querySelectorAll(".composer-table__grip").forEach(function (grip) {
    var row = grip.closest("tr");
    var cell = grip.closest("th, td");
    var kind = grip.classList.contains("--row") ? "row" : "column";
    var current =
      !!cell &&
      (kind === "row"
        ? cell.matches(".is-current-row, .is-structural-target.--axis-row")
        : cell.matches(".is-current-column, .is-structural-target.--axis-column"));
    var number = kind === "row" ? row && row.rowIndex + 1 : cell && cell.cellIndex + 1;

    grip.setAttribute("aria-hidden", current ? "false" : "true");
    if (number) {
      grip.setAttribute("aria-label", i18n(pluginParams, "composer.table." + kind + "_actions", { number: number }));
    }
  });
}

function dragAvatar(grip, kind) {
  var bounds = grip.getBoundingClientRect();
  var avatar = document.createElement("div");
  avatar.className = "composer-table__drag-avatar --" + kind;
  avatar.ariaHidden = "true";
  Object.assign(avatar.style, {
    top: bounds.top + "px",
    left: bounds.left + "px",
    width: bounds.width + "px",
    height: bounds.height + "px",
  });
  document.body.appendChild(avatar);
  return avatar;
}

// Start, end, offset within `inner` and size of every row or column.
function measureSpans(inner, grid, kind) {
  var bounds = inner.getBoundingClientRect();
  var elements = kind === "row" ? Array.from(grid.rows) : Array.from(grid.rows[0] ? grid.rows[0].cells : []);

  return elements.map(function (element) {
    var box = element.getBoundingClientRect();
    return kind === "row"
      ? { start: box.top, end: box.bottom, offset: box.top - bounds.top, size: box.height }
      : { start: box.left, end: box.right, offset: box.left - bounds.left, size: box.width };
  });
}

// +1 when the axis grows with the index, -1 for columns in RTL.
function spanDirection(spans) {
  if (spans.length < 2) {
    return 1;
  }
  var first = spans[0];
  var last = spans[spans.length - 1];
  return Math.sign(last.start + last.end - (first.start + first.end)) || 1;
}

function spanUnderPointer(spans, position, forward) {
  if (!spans.length) {
    return 0;
  }

  var found = spans.findIndex(function (span) {
    return position >= span.start && position < span.end;
  });
  if (found !== -1) {
    return found;
  }

  var firstCenter = (spans[0].start + spans[0].end) / 2;
  return (position - firstCenter) * forward < 0 ? 0 : spans.length - 1;
}

function positionIndicator(indicator, kind, spans, dropIndex, forward) {
  if (!indicator) {
    return;
  }

  if (dropIndex === null) {
    indicator.className = "composer-table__drop-indicator";
    indicator.removeAttribute("style");
    return;
  }

  var after = dropIndex > 0 ? spans[dropIndex - 1] : null;
  var edge = after
    ? after.offset + (forward > 0 ? after.size : 0)
    : spans[0].offset + (forward > 0 ? 0 : spans[0].size);

  indicator.className = "composer-table__drop-indicator --" + kind;
  if (kind === "row") {
    indicator.style.top = edge + "px";
  } else {
    indicator.style.left = edge + "px";
  }
}
