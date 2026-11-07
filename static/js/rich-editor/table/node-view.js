// The table's node views (static/prosemirror/lib/table/node-view.js): the
// cells, which ignore the classes and alignment styles decorations set, and
// the table, wrapped in a scroller with the append bars (click to add one
// column or row, drag to add or remove several), the drop indicator and the
// drag ghost. Only the hovered cell's row and column show their grips.
import { DOMSerializer } from "prosemirror-model";
import {
  addColumn,
  addRow,
  deleteColumn,
  deleteRow,
  emptyTrailingColumns,
  emptyTrailingRows,
  runCommand,
} from "discourse/rich-editor/table/commands";
import dragAutoscroll from "discourse/rich-editor/table/drag-autoscroll";
import { columnTarget, isTable, rowTarget, tableGrid } from "discourse/rich-editor/table/grid";
import { announce, TABLE_MENU_IDENTIFIER } from "discourse/rich-editor/table/menu";
import trackPointer from "discourse/rich-editor/table/track-pointer";
import { iconElement } from "discourse/rich-editor/toolbar";

var APPEND_DRAG_THRESHOLD = 4;
var APPEND_DRAG_MAX = { column: 8, row: 12 };
var APPEND_DRAG_STEP_EM = { column: 5, row: 2 };
var TOUCH_SCROLL_THRESHOLD = 8;

var AXES = {
  column: {
    target: columnTarget,
    add: addColumn,
    remove: deleteColumn,
    emptyTrailing: emptyTrailingColumns,
  },
  row: {
    target: rowTarget,
    add: addRow,
    remove: deleteRow,
    emptyTrailing: emptyTrailingRows,
  },
};

export class TableCellView {
  constructor(node) {
    this.node = node;
    var rendered = DOMSerializer.renderSpec(document, node.type.spec.toDOM(node));
    this.dom = rendered.dom;
    this.contentDOM = rendered.contentDOM;
  }

  ignoreMutation(mutation) {
    return (
      mutation.target === this.dom &&
      mutation.type === "attributes" &&
      (mutation.attributeName === "class" ||
        (mutation.attributeName === "style" && this.dom.style.textAlign === (this.node.attrs.alignment || "")))
    );
  }
}

export function buildTableNodeView(pluginParams) {
  var i18n = function (key, params) {
    var context = pluginParams.getContext();
    return context.i18n ? context.i18n(key, params) : key;
  };

  return class TableNodeView {
    constructor(node, view, getPos) {
      this.node = node;
      this.view = view;
      this.getPos = getPos;
      this._hovered = null;
      this._stopAppendDrag = null;
      this._stopTouchScroll = null;

      this._clearPointer = this.clearPointer.bind(this);
      this._trackPointer = this.trackPointer.bind(this);
      this._trackTouchScroll = this.trackTouchScroll.bind(this);

      this.dom = document.createElement("div");
      this.dom.className = "md-table composer-table";

      this.inner = document.createElement("div");
      this.inner.className = "composer-table__inner";
      this.dom.appendChild(this.inner);

      this.contentDOM = document.createElement("table");
      this.inner.appendChild(this.contentDOM);

      this.inner.appendChild(this.appendButton("column", "composer.table.add_column"));
      this.inner.appendChild(this.appendButton("row", "composer.table.add_row"));

      this.dropIndicator = document.createElement("div");
      this.dropIndicator.className = "composer-table__drop-indicator";
      this.inner.appendChild(this.dropIndicator);

      this.appendGhost = document.createElement("div");
      this.appendGhost.className = "composer-table__ghost";
      this.inner.appendChild(this.appendGhost);

      // Only the pointer's own row and column show a handle, so the table
      // stays quiet while it is typed in. Done in the DOM, not through a
      // transaction: hovering a cell is not a document change.
      this.dom.addEventListener("mousemove", this._trackPointer);
      this.dom.addEventListener("mouseleave", this._clearPointer);
      this.contentDOM.addEventListener("pointerdown", this._trackTouchScroll);
    }

    clearPointer() {
      this._hovered = null;
      this.showHandlesFor(null);
    }

    trackPointer(event) {
      if (!this.view.editable) {
        this.clearPointer();
        return;
      }

      // A reorder owns the handle it started from; re-targeting mid-drag
      // would take it out from under the pointer.
      if (this.inner.classList.contains("is-reordering")) {
        return;
      }

      var cell = event.target.closest && event.target.closest("th, td");
      if (!cell || !this.contentDOM.contains(cell) || cell === this._hovered) {
        return;
      }

      this._hovered = cell;
      this.showHandlesFor(cell);
    }

    trackTouchScroll(startEvent) {
      if (
        startEvent.pointerType !== "touch" ||
        !startEvent.isPrimary ||
        startEvent.button !== 0 ||
        (startEvent.target.closest && startEvent.target.closest(".composer-table__grip"))
      ) {
        return;
      }

      if (this._stopTouchScroll) {
        this._stopTouchScroll();
      }

      var self = this;
      var clientX = startEvent.clientX;
      var clientY = startEvent.clientY;
      var initialScroll = this.dom.scrollLeft;
      var scrolling = false;
      var stopTracking;

      var stop = function () {
        self.dom.classList.remove("is-panning");
        stopTracking();
        self._stopTouchScroll = null;
      };

      var move = function (event) {
        var horizontal = event.clientX - clientX;
        var vertical = event.clientY - clientY;
        if (
          !scrolling &&
          (Math.abs(horizontal) < TOUCH_SCROLL_THRESHOLD || Math.abs(horizontal) <= Math.abs(vertical))
        ) {
          return;
        }

        scrolling = true;
        event.preventDefault();
        self.dom.classList.add("is-panning");
        self.dom.scrollLeft = initialScroll - horizontal;
      };

      stopTracking = trackPointer(startEvent, move, stop);
      this._stopTouchScroll = stop;
    }

    destroy() {
      if (this._stopAppendDrag) {
        this._stopAppendDrag();
      }
      if (this._stopTouchScroll) {
        this._stopTouchScroll();
      }
      this.dom.removeEventListener("mousemove", this._trackPointer);
      this.dom.removeEventListener("mouseleave", this._clearPointer);
      this.contentDOM.removeEventListener("pointerdown", this._trackTouchScroll);
    }

    update(node) {
      if (node.type !== this.node.type) {
        return false;
      }
      if (this._stopAppendDrag) {
        this._stopAppendDrag();
      }
      this.node = node;
      return true;
    }

    stopEvent(event) {
      return !!(
        event.target.closest &&
        event.target.closest('.composer-table__append, [data-identifier="' + TABLE_MENU_IDENTIFIER + '"]')
      );
    }

    ignoreMutation(mutation) {
      return mutation.type !== "selection" && !this.contentDOM.contains(mutation.target);
    }

    resize(kind, delta) {
      if (!this.view.editable || !delta) {
        return;
      }

      var table = this.table();
      if (!table) {
        return;
      }

      var axis = AXES[kind];
      if (delta < 0) {
        delta = -Math.min(-delta, axis.emptyTrailing(table.grid));
        if (!delta) {
          return;
        }
      }
      var size = kind === "column" ? table.grid.width : table.grid.height;
      var target = axis.target(table, delta > 0 ? size - 1 : size + delta, size - 1);

      var changed = runCommand(this.view, delta > 0 ? axis.add(1, target, delta) : axis.remove(target));
      if (changed) {
        announce(
          pluginParams,
          i18n("composer.table." + kind + "s_" + (delta > 0 ? "added" : "removed"), { count: Math.abs(delta) })
        );
      }
    }

    appendButton(kind, labelKey) {
      var self = this;
      var dragged = false;
      var button = this.button("append --" + kind, labelKey, function (event) {
        if (dragged && event.detail !== 0) {
          dragged = false;
          return;
        }
        self.resize(kind, 1);
      });

      button.appendChild(iconElement("plus"));

      var count = document.createElement("span");
      count.className = "composer-table__append-count";
      count.ariaHidden = "true";
      button.appendChild(count);

      button.addEventListener("pointerdown", function (event) {
        self.trackAppend(event, kind, button, function (value) {
          dragged = value;
        });
      });
      return button;
    }

    button(className, labelKey, onClick) {
      var button = document.createElement("button");
      button.type = "button";
      button.className = "composer-table__" + className;
      button.contentEditable = "false";
      button.tabIndex = -1;
      button.setAttribute("aria-label", i18n(labelKey));
      // A click keeps synthesized activation working; mousedown only keeps
      // the caret where it is.
      button.addEventListener("mousedown", function (event) {
        event.preventDefault();
      });
      button.addEventListener("click", onClick);
      return button;
    }

    showGhost(kind, delta, step, bounds, rtl) {
      var ghost = this.appendGhost;

      ghost.className = "composer-table__ghost";
      ghost.removeAttribute("style");
      this.inner.style.paddingInlineEnd = "";
      this.inner.style.paddingBlockEnd = "";

      if (!delta) {
        return;
      }

      var adding = delta > 0;
      var count = Math.abs(delta);
      var span = adding ? count * step : this.trailingExtent(kind, count);

      ghost.classList.add("--visible", "--" + kind, adding ? "--add" : "--remove");
      ghost.style.setProperty("--ghost-step", span / count + "px");

      if (kind === "column") {
        if (adding) {
          this.inner.style.paddingInlineEnd = span + "px";
        }
        // Adding draws just past the edge, removing over the doomed columns.
        var offset = adding ? bounds.width : bounds.width - span;
        Object.assign(ghost.style, {
          top: "0px",
          left: this.contentDOM.offsetLeft + (rtl ? bounds.width - offset - span : offset) + "px",
          width: span + "px",
          height: bounds.height + "px",
        });
      } else {
        if (adding) {
          this.inner.style.paddingBlockEnd = span + "px";
        }
        Object.assign(ghost.style, {
          top: (adding ? bounds.height : bounds.height - span) + "px",
          left: this.contentDOM.offsetLeft + "px",
          width: bounds.width + "px",
          height: span + "px",
        });
      }
    }

    trailingExtent(kind, count) {
      var rows = Array.from(this.contentDOM.rows);

      if (kind === "row") {
        return rows.slice(-count).reduce(function (total, row) {
          return total + row.getBoundingClientRect().height;
        }, 0);
      }

      return Array.from(rows[0] ? rows[0].cells : [])
        .slice(-count)
        .reduce(function (total, cell) {
          return total + cell.getBoundingClientRect().width;
        }, 0);
    }

    showHandlesFor(cell) {
      this.contentDOM.querySelectorAll(".composer-table__grip").forEach(function (grip) {
        grip.classList.remove("is-visible");
      });

      if (!cell) {
        return;
      }

      var row = cell.parentElement;
      var column = Array.from(row.cells).indexOf(cell);

      var rowGrip = row.cells[0] && row.cells[0].querySelector(".composer-table__grip.--row");
      if (rowGrip) {
        rowGrip.classList.add("is-visible");
      }
      var header = this.contentDOM.rows[0] && this.contentDOM.rows[0].cells[column];
      var columnGrip = header && header.querySelector(".composer-table__grip.--column");
      if (columnGrip) {
        columnGrip.classList.add("is-visible");
      }
    }

    table() {
      var pos = this.getPos();
      if (pos === undefined) {
        return null;
      }

      var node = this.view.state.doc.nodeAt(pos);
      if (!isTable(node)) {
        return null;
      }

      return { node: node, pos: pos, start: pos + 1, grid: tableGrid(node) };
    }

    trackAppend(startEvent, kind, button, setDragged) {
      if (!this.view.editable || startEvent.button !== 0) {
        return;
      }

      startEvent.preventDefault();

      // A drag that ends off the button gets no click, so the next press
      // clears the suppression rather than the click it eats.
      setDragged(false);
      if (this._stopAppendDrag) {
        this._stopAppendDrag();
      }

      var table = this.table();
      if (!table) {
        return;
      }

      var self = this;
      var column = kind === "column";
      var initialDoc = this.view.state.doc;
      var origin = column ? startEvent.clientX : startEvent.clientY;

      var bounds = this.contentDOM.getBoundingClientRect();
      var style = getComputedStyle(this.contentDOM);
      var measuredStep = column ? bounds.width / table.grid.width : bounds.height / table.grid.height;
      var fontSize = parseFloat(style.fontSize) || 16;
      var step = Math.max(measuredStep, fontSize * APPEND_DRAG_STEP_EM[kind]);
      var rtl = style.direction === "rtl";
      var direction = column && rtl ? -1 : 1;
      var removable = AXES[kind].emptyTrailing(table.grid);

      var countElement = button.querySelector(".composer-table__append-count");

      var dragging = false;
      var delta = 0;
      var current = origin;
      var scrollDistance = 0;
      var stopTracking;

      var reset = function () {
        button.classList.remove("is-appending", "is-removing");
        countElement.textContent = "";
        self.showGhost(kind, 0, step, bounds);
      };

      var updateDelta = function () {
        var distance = (current - origin) * direction + scrollDistance;

        if (!dragging && Math.abs(distance) < APPEND_DRAG_THRESHOLD) {
          return false;
        }

        if (!dragging) {
          dragging = true;
          setDragged(true);
        }

        var steps = Math.min(Math.round(Math.abs(distance) / Math.max(step, 1)), APPEND_DRAG_MAX[kind]);
        delta = distance > 0 ? steps : -Math.min(steps, removable);

        if (!delta) {
          reset();
          return true;
        }

        button.classList.toggle("is-appending", delta > 0);
        button.classList.toggle("is-removing", delta < 0);
        countElement.textContent = delta > 0 ? "+" + delta : "−" + -delta;
        self.showGhost(kind, delta, step, bounds, rtl);
        return true;
      };

      var autoscroll = dragAutoscroll(button.closest(".composer-table"), column ? "X" : "Y", function (change) {
        scrollDistance += change * direction;
        updateDelta();
      });

      var move = function (event) {
        current = column ? event.clientX : event.clientY;
        if (!updateDelta()) {
          return;
        }
        event.preventDefault();
        autoscroll.update(event);
      };

      var finish = function (event) {
        if (self._stopAppendDrag) {
          self._stopAppendDrag();
        }

        if (event.type === "pointercancel" || self.view.state.doc !== initialDoc || !self.view.editable) {
          setDragged(false);
          return;
        }

        self.resize(kind, delta);
      };

      this._stopAppendDrag = function () {
        reset();
        autoscroll.stop();
        stopTracking();
        self._stopAppendDrag = null;
      };

      stopTracking = trackPointer(startEvent, move, finish);
    }
  };
}
