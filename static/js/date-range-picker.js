// DashboardDateRangePicker (the presets, the two month calendar, the
// typed dates, apply and cancel), shared by the pages with a
// DashboardDateRange: the dashboard and the MCP activity. The caller
// gives the strings, escaping and icons, what a preset and an applied
// range do, and how the menu closes.
(function () {
  "use strict";

  if (Discourse.dateRangePicker) {
    return;
  }

  Discourse.dateRangePicker = function (trigger, opts) {
    var t = opts.t;
    var escapeHtml = opts.escapeHtml;
    var icon = opts.icon;
    var close = opts.close;
    var presets = JSON.parse(trigger.dataset.presets);
    var active = trigger.dataset.active;
    var from = trigger.dataset.from ? moment(trigger.dataset.from, "YYYY-MM-DD") : null;
    var to = trigger.dataset.to ? moment(trigger.dataset.to, "YYYY-MM-DD") : null;
    var maxDate = moment().startOf("day");
    var two = !document.documentElement.classList.contains("mobile-view");
    var state = { start: null, end: null, hover: null, view: null };
    var el = document.createElement("div");
    el.className = "d-date-range-picker";

    function committed() {
      return state.start === null;
    }
    function currentStart() {
      return committed() ? from : state.start;
    }
    function currentEnd() {
      return committed() ? to : state.end;
    }
    function viewStart() {
      return state.view || (from || moment().startOf("day")).clone().startOf("month");
    }
    function applyDisabled() {
      if (!state.start || !state.end) {
        return true;
      }
      if (!from || !to) {
        return false;
      }
      return state.start.isSame(from, "day") && state.end.isSame(to, "day");
    }
    function dayClass(day) {
      var c = ["d-date-range-picker__day"];
      var s = currentStart();
      var e = currentEnd();
      var hoverEnd =
        !committed() && !state.end && state.hover && !state.hover.isBefore(state.start, "day") ? state.hover : null;
      if (day.isAfter(maxDate, "day")) c.push("--disabled");
      if (s && day.isSame(s, "day")) c.push("--start");
      if (e && day.isSame(e, "day")) c.push("--end");
      if (s && e && day.isAfter(s, "day") && day.isBefore(e, "day")) c.push("--in-range");
      if (hoverEnd && day.isAfter(state.start, "day") && day.isBefore(hoverEnd, "day")) c.push("--hover-preview");
      if (hoverEnd && day.isSame(hoverEnd, "day") && day.isAfter(state.start, "day")) c.push("--hover-end");
      if (day.date() === 1) c.push("--month-start");
      if (day.date() === day.daysInMonth()) c.push("--month-end");
      return c.join(" ");
    }
    function month(m, index, last) {
      var html = '<div class="d-date-range-picker__month"><div class="d-date-range-picker__month-header">';
      if (index === 0) {
        html +=
          '<button class="btn no-text btn-icon btn-transparent d-date-range-picker__nav --prev" aria-label="' +
          escapeHtml(t("dates.previous_month")) +
          '" type="button" data-shift="-1">' +
          icon("chevron-left") +
          '<span aria-hidden="true"> &#8203; </span></button>';
      }
      html += '<span class="d-date-range-picker__month-title">' + escapeHtml(m.format("MMMM YYYY")) + "</span>";
      if (index === last) {
        html +=
          '<button class="btn no-text btn-icon btn-transparent d-date-range-picker__nav --next" aria-label="' +
          escapeHtml(t("dates.next_month")) +
          '" type="button" data-shift="1">' +
          icon("chevron-right") +
          '<span aria-hidden="true"> &#8203; </span></button>';
      }
      html += '</div><div aria-hidden="true" class="d-date-range-picker__weekdays">';
      moment.weekdaysMin(true).forEach(function (d) {
        html += "<span>" + escapeHtml(d) + "</span>";
      });
      html +=
        '</div><div aria-label="' + escapeHtml(m.format("MMMM YYYY")) + '" class="d-date-range-picker__grid" role="grid">';
      var cursor = m.clone().startOf("month").startOf("week");
      var end = m.clone().endOf("month").endOf("week");
      while (cursor.isSameOrBefore(end)) {
        html += '<div class="d-date-range-picker__week" role="row">';
        for (var i = 0; i < 7; i++) {
          if (!cursor.isSame(m, "month")) {
            html += '<span aria-hidden="true"></span>';
          } else {
            var disabled = cursor.isAfter(maxDate, "day");
            var s = currentStart();
            var e = currentEnd();
            var selected = (s && cursor.isSame(s, "day")) || (e && cursor.isSame(e, "day"));
            html +=
              "<button" +
              (disabled ? ' aria-disabled="true"' : "") +
              ' aria-label="' +
              escapeHtml(cursor.format("LL")) +
              '" aria-selected="' +
              (selected ? "true" : "false") +
              '" class="' +
              dayClass(cursor) +
              '"' +
              (disabled ? " disabled" : "") +
              ' role="gridcell" tabindex="-1" type="button" data-day="' +
              cursor.format("YYYY-MM-DD") +
              '">' +
              cursor.format("D") +
              "</button>";
          }
          cursor.add(1, "day");
        }
        html += "</div>";
      }
      return html + "</div></div>";
    }
    function render() {
      var activeId = committed() ? active : null;
      var html =
        '<div aria-label="' +
        escapeHtml(t("date_range_picker.presets.label")) +
        '" class="d-date-range-picker__presets" role="group">';
      presets.forEach(function (p) {
        var on = p.id === activeId;
        html +=
          "<button" +
          (on ? ' aria-current="true"' : "") +
          ' class="d-date-range-picker__preset' +
          (on ? " is-active" : "") +
          '" type="button" data-preset="' +
          escapeHtml(p.id) +
          '">' +
          escapeHtml(p.label) +
          "</button>";
      });
      html += '</div><div class="d-date-range-picker__body"><div class="d-date-range-picker__calendar"><div class="d-date-range-picker__months">';
      var months = [viewStart().clone()];
      if (two) {
        months.push(viewStart().clone().add(1, "month"));
      }
      months.forEach(function (m, i) {
        html += month(m, i, months.length - 1);
      });
      var s = currentStart();
      var e = currentEnd();
      html +=
        '</div></div><div class="d-date-range-picker__inputs"><input aria-label="' +
        escapeHtml(t("date_range_picker.start_date")) +
        '" autocomplete="off" class="d-date-range-picker__input" inputmode="numeric" placeholder="' +
        escapeHtml(t("date_range_picker.date_placeholder")) +
        '" type="text" value="' +
        (s ? s.format("YYYY/MM/DD") : "") +
        '" data-input="start"><input aria-label="' +
        escapeHtml(t("date_range_picker.end_date")) +
        '" autocomplete="off" class="d-date-range-picker__input" inputmode="numeric" placeholder="' +
        escapeHtml(t("date_range_picker.date_placeholder")) +
        '" type="text" value="' +
        (e ? e.format("YYYY/MM/DD") : "") +
        '" data-input="end"></div><div class="d-date-range-picker__footer"><button class="btn btn-text btn-default d-date-range-picker__cancel" type="button" data-action="cancel"><span class="d-button-label">' +
        escapeHtml(t("cancel")) +
        '</span></button><button class="btn btn-text btn-primary d-date-range-picker__apply"' +
        (applyDisabled() ? " disabled" : "") +
        ' type="button" data-action="apply"><span class="d-button-label">' +
        escapeHtml(t("date_range_picker.apply")) +
        "</span></button></div></div>";
      el.innerHTML = html;
    }
    function parseTyped(value) {
      var parsed = moment(value, "YYYY/MM/DD", true);
      if (!parsed.isValid()) {
        return null;
      }
      var day = parsed.startOf("day");
      return day.isAfter(maxDate, "day") ? null : day;
    }
    el.addEventListener("click", function (event) {
      var preset = event.target.closest("[data-preset]");
      if (preset) {
        close();
        opts.onPreset(preset.dataset.preset);
        return;
      }
      var shift = event.target.closest("[data-shift]");
      if (shift) {
        state.view = viewStart().clone().add(parseInt(shift.dataset.shift, 10), "months");
        render();
        return;
      }
      var dayButton = event.target.closest("[data-day]");
      if (dayButton && !dayButton.disabled) {
        var day = moment(dayButton.dataset.day, "YYYY-MM-DD");
        if (committed() || state.end) {
          state.start = day;
          state.end = null;
        } else if (day.isBefore(state.start, "day")) {
          state.start = day;
        } else {
          state.end = day;
        }
        render();
        return;
      }
      var action = event.target.closest("[data-action]");
      if (action && action.dataset.action === "cancel") {
        close();
      } else if (action && action.dataset.action === "apply" && !applyDisabled()) {
        close();
        opts.onRange(state.start.format("YYYY-MM-DD"), state.end.format("YYYY-MM-DD"));
      }
    });
    el.addEventListener("mouseover", function (event) {
      var dayButton = event.target.closest("[data-day]");
      if (!dayButton || committed() || dayButton.disabled) {
        return;
      }
      var day = moment(dayButton.dataset.day, "YYYY-MM-DD");
      if (!state.hover || !state.hover.isSame(day, "day")) {
        state.hover = day;
        render();
      }
    });
    el.addEventListener("change", function (event) {
      var input = event.target.closest("[data-input]");
      if (!input) {
        return;
      }
      var parsed = parseTyped(input.value);
      if (input.dataset.input === "start") {
        if (!parsed) {
          render();
          return;
        }
        var end = currentEnd();
        state.start = parsed;
        state.end = end && !end.isBefore(parsed, "day") ? end.clone() : null;
      } else {
        var start = currentStart();
        if (!parsed || !start || parsed.isBefore(start, "day")) {
          render();
          return;
        }
        state.start = start.clone();
        state.end = parsed;
      }
      render();
    });
    render();
    return el;
  };
})();
