// select-kit's ComboBox over the server's <details class="prefs-combo">
// (preferences_view's combo): what the pages drawing one share, each
// binding its own events (preferences.js, login.js's signup user fields).
//
//   Discourse.comboBox.expanded(chooser)   is-expanded and the caret, as open
//   Discourse.comboBox.float(chooser)      the body under the header
//   Discourse.comboBox.filterRows(chooser, text)
//   Discourse.comboBox.select(chooser, row)  fires prefs-change
//   Discourse.comboBox.value(chooser)      the selected value, or null
(function () {
  "use strict";

  if (!Discourse.once("combo-box")) {
    return;
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function expanded(chooser) {
    chooser.classList.toggle("is-expanded", chooser.open);
    var caret = chooser.querySelector(".angle-icon");
    if (caret) {
      caret.outerHTML = icon(chooser.open ? "angle-up" : "angle-down", "angle-icon");
    }
  }

  // ComboBox and DropdownSelectBox: the body floats under the header as
  // select-kit's does.
  function floatBody(chooser) {
    var header = chooser.querySelector("summary");
    var body = chooser.querySelector(".select-kit-body");
    if (!body || !window.FloatingUIDOM) {
      return;
    }
    var ui = window.FloatingUIDOM;
    var small = window.innerWidth <= 450;
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    body.style.minWidth = small ? "" : Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
    body.style.width = small ? window.innerWidth - 20 + "px" : "";
    ui.computePosition(header, body, {
      placement: "bottom-start",
      strategy: "fixed",
      middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
    }).then(function (result) {
      var x = small ? 10 : Math.round(result.x);
      body.style.transform = "translate(" + x + "px," + Math.round(result.y) + "px)";
    });
  }

  function filterRows(chooser, text) {
    var needle = text.toLowerCase();
    chooser.querySelectorAll(".select-kit-row").forEach(function (row) {
      row.hidden = needle !== "" && (row.dataset.name || "").toLowerCase().indexOf(needle) === -1;
    });
  }

  // Selecting a row: the header shows it, the rows mark it.
  function select(chooser, row) {
    var value = row.classList.contains("none") ? null : row.dataset.value;
    var name = row.dataset.name;
    var summary = chooser.querySelector("summary");
    var choice = chooser.querySelector(".select-kit-selected-name");
    [summary, choice].forEach(function (el) {
      el.dataset.name = name;
      if (value === null) {
        delete el.dataset.value;
      } else {
        el.dataset.value = value;
      }
    });
    choice.title = name;
    choice.querySelector(".name").textContent = name;
    chooser.classList.toggle("has-selection", value !== null);
    chooser.querySelectorAll(".select-kit-row").forEach(function (r) {
      var is = r === row;
      r.classList.toggle("is-selected", is && value !== null);
      r.classList.toggle("is-highlighted", is);
      r.setAttribute("aria-checked", is ? "true" : "false");
    });
    chooser.dataset.dirty = "true";
    chooser.dispatchEvent(new CustomEvent("prefs-change", { bubbles: true, detail: { value: value } }));
  }

  function comboValue(chooser) {
    var summary = chooser.querySelector("summary");
    return summary.dataset.value === undefined ? null : summary.dataset.value;
  }

  Discourse.comboBox = {
    expanded: expanded,
    float: floatBody,
    filterRows: filterRows,
    select: select,
    value: comboValue,
  };
})();
