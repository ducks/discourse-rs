// BookmarkModal (components/modal/bookmark.gjs) for a bookmark the member
// has, with DTimeShortcutPicker (ui-kit/d-time-shortcut-picker.gjs over
// lib/time-shortcut.js and lib/time-utils.js) for its reminder.
//
//   Discourse.bookmarkModal.open({ bookmark, afterSave, afterDelete })
//       bookmark: { id, name, reminder_at, auto_delete_preference,
//       bookmarkable_id, bookmarkable_type } (BookmarkFormData's). The
//       name, what happens once the member is reminded, and the reminder:
//       a tile saves and closes at once, the custom date and time and the
//       relative time wait for Save. A click outside saves too; Cancel and
//       the close button do not. Delete asks first when there is a
//       reminder. PUT and DELETE /bookmarks/:id, as the bookmarkApi
//       service; afterSave(saveData) and afterDelete(id) then.
//   Discourse.bookmarkModal.t(key, args)
//       its strings, which BookmarkMenu (topic-bookmarks-menu.js) reads.
//   Discourse.timeShortcutPicker.load() -> Promise, .shortcuts()
//       moment loaded, then lib/time-shortcut's timeShortcuts by name.
//   Discourse.timeShortcutPicker.futureDateInput(root, { input,
//       onChangeInput }) -> { destroy }
//       DFutureDateInput without its shortcuts: a date and a time.
//   Discourse.timeShortcutPicker.mount(root, opts) -> { destroy }
//       DTimeShortcutPicker in root. opts: { timezone, prefilledDatetime,
//       hiddenOptions, customLabels, timeShortcuts(shortcuts) (the
//       options, defaultTimeShortcuts without), onTimeSelected(type,
//       moment or null), keys (bind its key sequences, a function taking
//       { sequence: fn }) }.
//
// The strings, the member's timezone, suggest_weekends_in_date_pickers,
// the icons and the scripts loaded on demand (moment, Pikaday) are the
// script tag's (crate::bookmarks::modal_script_attrs).
//
// Not ported: creating a bookmark through the modal (a post's and the
// topic's bookmark buttons toggle), the bookmark-modal-after-name plugin
// outlet.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  var script = document.currentScript;
  if (!Discourse.once("bookmark-modal")) {
    return;
  }

  var s = {};
  try {
    s = JSON.parse((script && script.dataset.config) || "{}");
  } catch (e) {
    window.console && console.error("bookmark-modal config", e);
  }
  var strings = s.strings || {};

  function t(key, args) {
    var text = strings[key] === undefined ? key : strings[key];
    Object.keys(args || {}).forEach(function (name) {
      text = text.split("%{" + name + "}").join(args[name]);
    });
    return text;
  }

  function tCount(key, count) {
    return t(key + (count === 1 ? ".one" : ".other"), { count: count });
  }

  function escapeHtml(value) {
    return String(value === null || value === undefined ? "" : value)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return (s.icons && s.icons[name]) || "";
  }

  // site.mobileView, as the page is drawn now.
  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      return {};
    }
  }

  function request(method, path, form) {
    var headers = memberHeaders();
    headers.Accept = "application/json";
    var init = { method: method, headers: headers, credentials: "same-origin" };
    if (form) {
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      init.body = new URLSearchParams(form).toString();
    }
    return fetch(base() + path, init).then(function (r) {
      if (r.ok) {
        return r;
      }
      // extractError: the errors the server gave.
      return r
        .json()
        .catch(function () {
          return {};
        })
        .then(function (body) {
          var errors = body && body.errors;
          throw new Error(errors && errors.length ? errors.join("\n") : t("generic_error"));
        });
    });
  }

  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var tag = document.createElement("script");
      tag.src = src;
      tag.onload = resolve;
      tag.onerror = reject;
      document.head.appendChild(tag);
    });
  }

  var ready = null;
  function load() {
    if (!ready) {
      var moment = window.moment ? Promise.resolve() : loadScript(s.moment);
      ready = moment.then(function () {
        return window.moment.tz ? null : loadScript(s.moment_timezone);
      });
    }
    return ready;
  }

  var pikaday = null;
  function loadPikaday() {
    if (window.Pikaday) {
      return Promise.resolve();
    }
    if (!pikaday) {
      pikaday = loadScript(s.pikaday);
    }
    return pikaday;
  }

  // keyValueStore, as lib/key-value-store keeps it.
  var store = {
    get: function (key) {
      try {
        return window.localStorage.getItem("discourse_" + key);
      } catch (e) {
        return null;
      }
    },
    set: function (key, value) {
      try {
        window.localStorage.setItem("discourse_" + key, value);
      } catch (e) {
        // A browser that keeps nothing offers no last custom time.
      }
    },
  };

  // ---- lib/time-utils.js

  var START_OF_DAY_HOUR = 8;
  var LATER_TODAY_CUTOFF_HOUR = 17;
  var LATER_TODAY_MAX_HOUR = 18;
  var SUNDAY = 0;
  var MONDAY = 1;
  var THURSDAY = 4;
  var FRIDAY = 5;
  var SATURDAY = 6;

  function now(timezone) {
    return window.moment.tz(timezone);
  }

  function startOfDay(date) {
    return date.hour(START_OF_DAY_HOUR).startOf("hour");
  }

  function laterToday(timezone) {
    var later = now(timezone).add(3, "hours");
    if (later.hour() >= LATER_TODAY_MAX_HOUR) {
      return later.hour(LATER_TODAY_MAX_HOUR).startOf("hour");
    }
    return later.minutes() < 30 ? later.startOf("hour") : later.add(30, "minutes").startOf("hour");
  }

  function parseCustomDatetime(date, time, currentTimezone, parseTimezone) {
    date = date || new Date().toISOString().split("T")[0];
    var dateTime = time ? date + " " + time : date;
    parseTimezone = parseTimezone || currentTimezone;
    var parsed = window.moment.tz(dateTime, parseTimezone);
    if (parseTimezone !== currentTimezone) {
      parsed = parsed.tz(currentTimezone);
    }
    return parsed;
  }

  // ---- lib/time-shortcut.js

  // timeShortcuts(timezone): each shortcut by name.
  function timeShortcuts(timezone) {
    var day = now(timezone).day();
    return {
      oneHour: function () {
        return {
          id: "one_hour",
          label: "time_shortcut.in_one_hour",
          time: now(timezone).add(1, "hours"),
          format: "dates.time",
        };
      },
      twoHours: function () {
        return {
          id: "two_hours",
          label: "time_shortcut.in_two_hours",
          time: now(timezone).add(2, "hours"),
          format: "dates.time",
        };
      },
      threeDays: function () {
        return {
          id: "three_days",
          label: "time_shortcut.three_days",
          time: startOfDay(now(timezone).add(3, "days")),
          format: "dates.time_short_day",
        };
      },
      laterToday: function () {
        return { id: "later_today", label: "time_shortcut.later_today", time: laterToday(timezone), format: "dates.time" };
      },
      tomorrow: function () {
        return {
          id: "tomorrow",
          label: "time_shortcut.tomorrow",
          time: startOfDay(now(timezone).add(1, "day")),
          format: "dates.time_short_day",
        };
      },
      laterThisWeek: function () {
        return {
          id: "later_this_week",
          label: "time_shortcut.later_this_week",
          time: startOfDay(now(timezone).add(2, "days")),
          format: "dates.time_short_day",
        };
      },
      thisWeekend: function () {
        return {
          id: "this_weekend",
          label: "time_shortcut.this_weekend",
          time: startOfDay(now(timezone).day(SATURDAY)),
          format: "dates.time_short_day",
        };
      },
      monday: function () {
        return {
          id: "start_of_next_business_week",
          label:
            day === MONDAY || day === SUNDAY
              ? "time_shortcut.start_of_next_business_week_alt"
              : "time_shortcut.start_of_next_business_week",
          time: startOfDay(now(timezone).add(7, "days")).day(MONDAY),
          format: "dates.long_no_year",
        };
      },
      nextMonth: function () {
        return {
          id: "next_month",
          label: "time_shortcut.next_month",
          time: startOfDay(now(timezone).add(1, "month").startOf("month")),
          format: "dates.long_no_year",
        };
      },
    };
  }

  function defaultTimeShortcuts(timezone) {
    var shortcuts = timeShortcuts(timezone);
    return [
      shortcuts.laterToday(),
      shortcuts.tomorrow(),
      shortcuts.laterThisWeek(),
      shortcuts.thisWeekend(),
      shortcuts.monday(),
      shortcuts.nextMonth(),
    ];
  }

  // specialShortcutOptions: the last custom time (hidden until there is
  // one), custom, none.
  function specialShortcutOptions() {
    return [
      { id: "last_custom", label: "time_shortcut.last_custom", time: null, hidden: true },
      { id: "custom", label: "time_shortcut.custom", time: null, custom: true },
      { id: "none", label: "time_shortcut.none", time: null },
    ];
  }

  function hideDynamicTimeShortcuts(shortcuts, timezone) {
    var hide = {};
    var at = now(timezone);
    if (at.hour() >= LATER_TODAY_CUTOFF_HOUR) {
      hide.later_today = true;
    }
    // `_now.day === MOMENT_SUNDAY` compares the function in Ember, so
    // Sunday does not hide it there either.
    if (at.day() >= THURSDAY) {
      hide.later_this_week = true;
    }
    if (!s.suggest_weekends || at.day() === FRIDAY || at.day() === SATURDAY || at.day() === SUNDAY) {
      hide.this_weekend = true;
    }
    return shortcuts.filter(function (shortcut) {
      return !hide[shortcut.id];
    });
  }

  // ---- a select-kit ComboBox: { id, className, content: [{ id, name }],
  // value, onChange(id) }, its rows floated under its header.
  function comboBox(opts) {
    var details = document.createElement("details");
    details.className = "select-kit single-select combobox combo-box" + (opts.className ? " " + opts.className : "");
    if (opts.id) {
      details.id = opts.id;
    }
    details.innerHTML =
      '<summary tabindex="0" class="select-kit-header single-select-header combo-box-header"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div>';
    var summary = details.querySelector("summary");
    var body = details.querySelector(".select-kit-body");
    var value = opts.value;

    function selected() {
      return (
        opts.content.find(function (c) {
          return String(c.id) === String(value);
        }) || null
      );
    }

    function drawHeader() {
      var item = selected();
      var name = item ? item.name : "";
      var label = item ? t("select_kit.filter_by", { name: name }) : t("select_kit.select_to_filter");
      summary.setAttribute("name", label);
      summary.setAttribute("aria-label", label);
      summary.dataset.name = name;
      if (item) {
        summary.dataset.value = item.id;
      } else {
        delete summary.dataset.value;
      }
      details.classList.toggle("has-selection", !!item);
      summary.querySelector(".select-kit-header-wrapper").innerHTML =
        '<div class="select-kit-selected-name selected-name choice" data-name="' +
        escapeHtml(name) +
        '"' +
        (item ? ' data-value="' + escapeHtml(item.id) + '"' : "") +
        ' title="' +
        escapeHtml(name) +
        '"><span class="name">' +
        escapeHtml(name) +
        "</span></div>" +
        icon(details.open ? "angle-up" : "angle-down");
    }

    function choose(row) {
      value = row.dataset.value;
      details.open = false;
      drawHeader();
      opts.onChange(value);
    }

    details.addEventListener("toggle", function () {
      details.classList.toggle("is-expanded", details.open);
      if (!details.open) {
        body.innerHTML = "";
        body.removeAttribute("style");
        drawHeader();
        return;
      }
      body.innerHTML =
        '<div class="select-kit-filter"></div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
        opts.content
          .map(function (c, index) {
            var chosen = String(c.id) === String(value);
            return (
              '<li tabindex="0" title="' +
              escapeHtml(c.name) +
              '" data-value="' +
              escapeHtml(c.id) +
              '" data-name="' +
              escapeHtml(c.name) +
              '" data-index="' +
              index +
              '" role="menuitemradio" aria-checked="' +
              chosen +
              '" class="select-kit-row' +
              (chosen ? " is-highlighted is-selected" : "") +
              '"><span class="name">' +
              escapeHtml(c.name) +
              "</span></li>"
            );
          })
          .join("") +
        "</ul>";
      drawHeader();
      var fui = window.FloatingUIDOM;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = Math.round(summary.getBoundingClientRect().width) + "px";
      if (fui) {
        fui
          .computePosition(summary, body, {
            placement: "bottom-start",
            strategy: "fixed",
            middleware: [fui.flip({ padding: 3 }), fui.shift({ limiter: fui.limitShift() }), fui.offset(3)],
          })
          .then(function (result) {
            body.style.transform = "translate(" + Math.round(result.x) + "px, " + Math.round(result.y) + "px)";
            body.style.visibility = "visible";
          });
      }
    });
    body.addEventListener("click", function (event) {
      var row = event.target.closest(".select-kit-row");
      if (row) {
        event.preventDefault();
        choose(row);
      }
    });
    body.addEventListener("keydown", function (event) {
      var row = event.target.closest(".select-kit-row");
      if (row && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault();
        event.stopPropagation();
        choose(row);
      }
    });
    drawHeader();
    return {
      element: details,
      set: function (content, next) {
        opts.content = content;
        value = next;
        drawHeader();
      },
    };
  }

  // ---- DRelativeTimePicker (ui-kit/d-relative-time-picker.gjs)

  var HOUR = 60;
  var DAY = 24 * HOUR;
  var MONTH = 30 * DAY;
  var YEAR = 365 * DAY;

  function roundDuration(duration) {
    var rounded = Math.round(parseFloat(duration.toFixed(1)) * 2) / 2;
    return rounded % 1 === 0 ? rounded.toFixed(0) : rounded;
  }

  function inputValueFromMinutes(minutes) {
    if (!minutes) {
      return null;
    } else if (minutes >= YEAR) {
      return roundDuration(minutes / YEAR);
    } else if (minutes >= MONTH) {
      return roundDuration(minutes / MONTH);
    } else if (minutes >= DAY) {
      return roundDuration(minutes / DAY);
    } else if (minutes >= HOUR) {
      return roundDuration(minutes / HOUR);
    }
    return minutes;
  }

  function intervalFromMinutes(minutes) {
    if (minutes >= YEAR) {
      return "years";
    } else if (minutes >= MONTH) {
      return "months";
    } else if (minutes >= DAY) {
      return "days";
    } else if (minutes >= HOUR) {
      return "hours";
    }
    return "mins";
  }

  function minutesFrom(duration, interval) {
    if (duration === null || isNaN(duration)) {
      return null;
    }
    switch (interval) {
      case "mins":
        // Rounded up, should a step under one be typed.
        return Math.ceil(duration);
      case "hours":
        return duration * HOUR;
      case "days":
        return duration * DAY;
      case "months":
        return duration * MONTH;
      case "years":
        return duration * YEAR;
    }
    return null;
  }

  // { id, durationMinutes, onChange(minutes) }
  function relativeTimePicker(opts) {
    var inputValue = inputValueFromMinutes(opts.durationMinutes);
    var interval =
      opts.durationMinutes !== undefined && opts.durationMinutes !== null
        ? intervalFromMinutes(opts.durationMinutes)
        : "days";
    var duration = minutesFrom(inputValue === null ? null : parseFloat(inputValue), interval);

    var root = document.createElement("div");
    root.className = "relative-time-picker";
    root.innerHTML = '<input class="relative-time-duration" type="number">';
    var input = root.querySelector("input");
    input.id = opts.id;

    function intervals() {
      var count = duration ? parseFloat(duration) : 0;
      return [
        { id: "mins", name: tCount("relative_time_picker.minutes", count) },
        { id: "hours", name: tCount("relative_time_picker.hours", count) },
        { id: "days", name: tCount("relative_time_picker.days", count) },
        { id: "months", name: tCount("relative_time_picker.months", count) },
        { id: "years", name: tCount("relative_time_picker.years", count) },
      ];
    }

    var combo = comboBox({
      className: "relative-time-intervals",
      content: intervals(),
      value: interval,
      onChange: function (next) {
        interval = next;
        var changed = minutesFrom(inputValue === null ? null : parseFloat(inputValue), interval);
        if (changed !== duration) {
          duration = changed;
          inputValue = inputValueFromMinutes(changed);
          draw();
          opts.onChange(duration);
        } else {
          draw();
        }
      },
    });
    root.appendChild(combo.element);

    function draw() {
      input.min = interval === "mins" ? 1 : 0.5;
      input.step = interval === "mins" ? 1 : 0.5;
      if (document.activeElement !== input || String(input.value) !== String(inputValue === null ? "" : inputValue)) {
        input.value = inputValue === null ? "" : inputValue;
      }
      combo.set(intervals(), interval);
    }

    input.addEventListener("change", function () {
      if (input.value.trim() === "") {
        duration = null;
        inputValue = null;
      } else {
        var changed = minutesFrom(parseFloat(input.value), interval);
        // On the edge of an interval, on to the next value (24 hours
        // become 1.5 days, not 1 day).
        if (changed > duration && (duration === YEAR || duration === MONTH || duration === DAY || duration === HOUR)) {
          changed = minutesFrom(parseFloat(input.value) * 1.5, interval);
        }
        duration = changed;
        interval = intervalFromMinutes(changed);
        inputValue = inputValueFromMinutes(changed);
      }
      draw();
      opts.onChange(duration);
    });
    draw();
    return root;
  }

  // ---- DTimeShortcutPicker

  var PICKER_KEYS = {
    "l t": "later_today",
    "n d": "tomorrow",
    "n b w": "start_of_next_business_week",
    "n m": "next_month",
    "c r": "custom",
    "n r": "none",
  };

  function mountPicker(root, opts) {
    var m = window.moment;
    var timezone = opts.timezone;
    var hidden = opts.hiddenOptions || [];
    var labels = opts.customLabels || {};
    var selectedShortcut = null;
    var customDate = null;
    var customTime = null;
    var durationMins = null;
    var picker = null;
    var destroyed = false;

    // parsePrefilledDatetime
    if (opts.prefilledDatetime) {
      var prefilled = parseCustomDatetime(opts.prefilledDatetime, null, timezone);
      if (prefilled.isSame(laterToday(timezone))) {
        selectedShortcut = "later_today";
      } else {
        customDate = prefilled.format("YYYY-MM-DD");
        customTime = prefilled.format("HH:mm");
        selectedShortcut = "custom";
      }
    }

    function options() {
      var list = opts.timeShortcuts ? opts.timeShortcuts(timeShortcuts(timezone)) : defaultTimeShortcuts(timezone);
      list = hideDynamicTimeShortcuts(list, timezone);
      var special = specialShortcutOptions();
      // _loadLastUsedCustomDatetime
      var lastTime = store.get("lastCustomTime");
      var lastDate = store.get("lastCustomDate");
      if (lastTime && lastDate) {
        var parsed = parseCustomDatetime(lastDate, lastTime, timezone);
        if (parsed.isValid() && !(parsed < now(timezone))) {
          special[0].time = parsed;
          special[0].format = "dates.long_no_year";
          special[0].hidden = false;
        }
      }
      list = list.concat(special);
      list.forEach(function (option) {
        if (hidden.indexOf(option.id) !== -1) {
          option.hidden = true;
        }
        if (labels[option.id]) {
          option.label = labels[option.id];
        }
        option.timeFormatted = option.time && option.format ? option.time.format(t(option.format)) : "";
      });
      return list;
    }

    function tileHtml(option) {
      var active = selectedShortcut === option.id;
      return (
        '<div id="tap_tile_' +
        option.id +
        '" class="tap-tile' +
        (active ? " active" : "") +
        '" role="button" aria-pressed="' +
        active +
        '" tabindex="0" data-tile="' +
        option.id +
        '"><div class="tap-tile-title">' +
        escapeHtml(t(option.label)) +
        '</div><div class="tap-tile-date">' +
        escapeHtml(option.timeFormatted) +
        "</div></div>"
      );
    }

    function draw() {
      if (picker) {
        picker.destroy();
        picker = null;
      }
      var html = '<div class="tap-tile-grid">';
      options().forEach(function (option) {
        if (!option.hidden) {
          html += tileHtml(option);
        }
        if (option.custom && selectedShortcut === "custom") {
          html +=
            '<div class="control-group custom-date-time-wrap custom-input-wrap"><div class="tap-tile-date-input">' +
            icon("calendar-days") +
            '<div class="date-picker-wrapper"><input class="date-picker" id="custom-date" autocomplete="off" placeholder="' +
            escapeHtml(t("dates.placeholder")) +
            '" type="' +
            (mobileView() ? "date" : "text") +
            '"></div></div><div class="tap-tile-time-input">' +
            icon("far-clock") +
            '<input class="time-input" id="custom-time" placeholder="--:--" type="time"></div></div>' +
            '<div class="control-group custom-date-time-wrap custom-relative-wrap">' +
            '<label class="control-label" for="bookmark-relative-time-picker">' +
            escapeHtml(t("relative_time_picker.relative")) +
            "</label></div>";
        }
      });
      root.innerHTML = html + "</div>";
      var wrap = root.querySelector(".custom-relative-wrap");
      if (!wrap) {
        return;
      }
      wrap.appendChild(
        relativeTimePicker({
          id: "bookmark-relative-time-picker",
          durationMinutes: durationMins,
          onChange: function (minutes) {
            // relativeTimeChanged
            durationMins = minutes;
            opts.onTimeSelected("relative", now(timezone).add(minutes, "minutes"));
          },
        })
      );
      var dateInput = root.querySelector("#custom-date");
      var timeInput = root.querySelector("#custom-time");
      dateInput.value = customDate || "";
      timeInput.value = customTime || "";
      // customDatetimeChanged: both filled picks the custom time.
      var changed = function () {
        if (customDate && customTime) {
          select("custom", true);
        }
      };
      timeInput.addEventListener("input", function () {
        customTime = timeInput.value;
        changed();
      });
      // A date typed in (the phone's date input, or the field before or
      // without Pikaday): taken when it is one.
      dateInput.addEventListener("change", function () {
        var typed = m(dateInput.value, "YYYY-MM-DD", true);
        if (typed.isValid() && dateInput.value !== customDate) {
          customDate = dateInput.value;
          changed();
        }
      });
      if (mobileView()) {
        return;
      }
      // DatePickerFuture: Pikaday from today on.
      loadPikaday().then(function () {
        if (destroyed || !dateInput.isConnected) {
          return;
        }
        picker = new window.Pikaday({
          field: dateInput,
          container: null,
          bound: true,
          format: "YYYY-MM-DD",
          firstDay: 1,
          minDate: m().toDate(),
          defaultDate: customDate ? m(customDate).toDate() : m().add(1, "day").toDate(),
          setDefaultDate: !!customDate,
          i18n: {
            previousMonth: t("dates.previous_month"),
            nextMonth: t("dates.next_month"),
            months: m.months(),
            weekdays: m.weekdays(),
            weekdaysShort: m.weekdaysMin(),
          },
          onSelect: function (date) {
            customDate = m(date).format("YYYY-MM-DD");
            dateInput.value = customDate;
            changed();
          },
        });
      })
      // Without the picker the field still takes a typed date.
      .catch(function () {});
    }

    // selectShortcut. `keep` leaves the tiles as they are (the custom
    // inputs are being typed in).
    function select(type, keep) {
      var list = options();
      var option = list.find(function (o) {
        return o.id === type;
      });
      if (!option || option.hidden) {
        return;
      }
      var dateTime = null;
      if (type === "custom") {
        // _defaultCustomDateTime: an hour from now.
        var fallback = m.tz(timezone).add(1, "hour");
        customDate = customDate || fallback.format("YYYY-MM-DD");
        customTime = customTime || fallback.format("HH:mm");
        var custom = parseCustomDatetime(customDate, customTime, timezone);
        if (custom.isValid() && customDate) {
          dateTime = custom;
          store.set("lastCustomTime", customTime);
          store.set("lastCustomDate", customDate);
        }
      } else {
        dateTime = option.time;
      }
      selectedShortcut = type;
      if (!keep) {
        draw();
      }
      opts.onTimeSelected(type, dateTime);
    }

    function activate(event) {
      var tile = event.target.closest && event.target.closest(".tap-tile");
      if (tile && root.contains(tile)) {
        event.preventDefault();
        select(tile.dataset.tile);
      }
    }
    root.addEventListener("click", activate);
    root.addEventListener("keydown", function (event) {
      if (event.key === "Enter" && event.target.classList && event.target.classList.contains("tap-tile")) {
        event.stopPropagation();
        activate(event);
      }
    });
    if (opts.keys) {
      var bindings = {};
      Object.keys(PICKER_KEYS).forEach(function (sequence) {
        bindings[sequence] = function () {
          select(PICKER_KEYS[sequence]);
        };
      });
      opts.keys(bindings);
    }
    draw();
    return {
      destroy: function () {
        destroyed = true;
        if (picker) {
          picker.destroy();
          picker = null;
        }
      },
    };
  }

  // ItsATrap's sequences, for a modal: keys typed outside inputs, a
  // sequence's keys within a second of each other. Returns its unbind.
  function keySequences(scope, bindings) {
    var typed = [];
    var last = 0;
    function onKeydown(event) {
      var target = event.target;
      if (
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        (target && target.closest && target.closest("input, textarea, select, [contenteditable], .select-kit"))
      ) {
        return;
      }
      if (!scope.isConnected) {
        return;
      }
      var key = event.key === "Enter" ? "enter" : event.key.length === 1 ? event.key.toLowerCase() : null;
      if (!key) {
        return;
      }
      var at = Date.now();
      if (at - last > 1000) {
        typed = [];
      }
      last = at;
      typed.push(key);
      var sequence = typed.join(" ");
      var names = Object.keys(bindings);
      if (bindings[sequence]) {
        typed = [];
        event.preventDefault();
        bindings[sequence]();
        return;
      }
      var prefix = names.some(function (name) {
        return name.indexOf(sequence + " ") === 0;
      });
      if (!prefix) {
        typed = bindings[key] || names.some(function (name) {
          return name.indexOf(key + " ") === 0;
        })
          ? [key]
          : [];
        if (typed.length && bindings[key]) {
          typed = [];
          event.preventDefault();
          bindings[key]();
        }
      }
    }
    document.addEventListener("keydown", onKeydown);
    return function () {
      document.removeEventListener("keydown", onKeydown);
    };
  }

  // ---- BookmarkModal

  var AUTO_DELETE = [
    ["never", 0],
    ["clear_reminder", 3],
    ["when_reminder_sent", 1],
    ["on_owner_reply", 2],
  ];

  // lib/bookmark's formattedReminderTime.
  function formattedReminderTime(reminderAt, timezone) {
    var m = window.moment;
    var at = m.tz(reminderAt, timezone);
    var formatted = at.format(t("dates.time"));
    var today = m.tz(timezone);
    var tomorrow = m(today).add(1, "day");
    if (at.isSame(tomorrow, "date")) {
      return t("bookmarks.reminders.tomorrow_with_time", { time: formatted });
    } else if (at.isSame(today, "date")) {
      return t("bookmarks.reminders.today_with_time", { time: formatted });
    }
    return t("bookmarks.reminders.at_time", { date_time: at.format(t("dates.long_with_year")) });
  }

  function open(model) {
    if (!Discourse.modal) {
      return;
    }
    load().then(
      function () {
        show(model);
      },
      function () {
        Discourse.dialog && Discourse.dialog.alert && Discourse.dialog.alert(t("generic_error"));
      }
    );
  }

  function show(model) {
    var timezone = s.timezone;
    // BookmarkFormData
    var bookmark = {
      id: model.bookmark.id,
      reminderAt: model.bookmark.reminder_at || null,
      name: model.bookmark.name || "",
      bookmarkableId: model.bookmark.bookmarkable_id,
      bookmarkableType: model.bookmark.bookmarkable_type,
      autoDeletePreference:
        model.bookmark.auto_delete_preference === null || model.bookmark.auto_delete_preference === undefined
          ? 3
          : model.bookmark.auto_delete_preference,
      selectedReminderType: "none",
      selectedDatetime: null,
    };
    var hasReminder = !!(bookmark.id && bookmark.reminderAt);
    var closeWithoutSaving = false;
    var savingManually = false;
    var saving = false;
    var deleting = false;
    var settled = false;
    var close = null;
    var unbind = null;
    var picker = null;
    var bodyEl = null;

    function reminderAtISO() {
      if (bookmark.selectedReminderType === "none") {
        return null;
      }
      if (!bookmark.selectedReminderType || !bookmark.selectedDatetime) {
        return bookmark.reminderAt ? new Date(bookmark.reminderAt).toISOString() : null;
      }
      return bookmark.selectedDatetime.toISOString();
    }

    function saveData() {
      return {
        reminder_at: reminderAtISO() || "",
        name: bookmark.name,
        id: bookmark.id,
        auto_delete_preference: bookmark.autoDeletePreference,
        bookmarkable_id: bookmark.bookmarkableId,
        bookmarkable_type: bookmark.bookmarkableType,
      };
    }

    function flash(message) {
      var old = bodyEl.parentNode.querySelector("#modal-alert");
      if (old) {
        old.remove();
      }
      if (!message) {
        return;
      }
      var alert = document.createElement("div");
      alert.id = "modal-alert";
      alert.setAttribute("role", "alert");
      alert.className = "alert alert-error";
      alert.textContent = message;
      bodyEl.parentNode.insertBefore(alert, bodyEl);
    }

    // #saveBookmark
    function saveBookmark() {
      if (bookmark.selectedReminderType === "custom" && !reminderAtISO()) {
        return Promise.reject(new Error(t("bookmarks.invalid_custom_datetime")));
      }
      var data = saveData();
      return request("PUT", "/bookmarks/" + encodeURIComponent(bookmark.id) + ".json", data).then(function () {
        if (model.afterSave) {
          model.afterSave(data);
        }
      });
    }

    function done() {
      settled = true;
      close();
    }

    function saveAndClose() {
      flash(null);
      if (saving || deleting) {
        return;
      }
      saving = true;
      savingManually = true;
      saveBookmark()
        .then(done, function (error) {
          savingManually = false;
          flash(error && error.message ? error.message : String(error));
        })
        .then(function () {
          saving = false;
        });
    }

    function destroy() {
      if (!bookmark.id) {
        return;
      }
      deleting = true;
      var act = function () {
        closeWithoutSaving = true;
        request("DELETE", "/bookmarks/" + encodeURIComponent(bookmark.id) + ".json")
          .then(function () {
            deleting = false;
            if (model.afterDelete) {
              model.afterDelete(bookmark.id);
            }
            done();
          })
          .catch(function (error) {
            deleting = false;
            flash(error && error.message ? error.message : String(error));
          });
      };
      if (hasReminder && Discourse.dialog && Discourse.dialog.deleteConfirm) {
        Discourse.dialog.deleteConfirm({
          message: escapeHtml(t("bookmarks.confirm_delete")),
          didConfirm: act,
          didCancel: function () {
            deleting = false;
          },
        });
      } else {
        act();
      }
    }

    close = Discourse.modal.show({
      className: "bookmark-reminder-modal",
      title: t("bookmarks.edit"),
      attributes: { id: "bookmark-reminder-modal", "data-bookmark-id": bookmark.id },
      headerPrimaryAction: function (el) {
        el.innerHTML =
          '<button class="btn btn-text btn-transparent" title="' +
          escapeHtml(t("modal.close")) +
          '" type="button"><span class="d-button-label">' +
          escapeHtml(t("bookmarks.save")) +
          "</span></button>";
        el.querySelector("button").addEventListener("click", saveAndClose);
      },
      // closingModal: a click outside saves, unless it is saving or
      // deleting already.
      beforeClose: function (initiatedBy) {
        if (settled || initiatedBy !== "click-outside" || closeWithoutSaving || savingManually) {
          return true;
        }
        settled = true;
        saveBookmark()
          .catch(function (error) {
            if (Discourse.dialog && Discourse.dialog.alert) {
              Discourse.dialog.alert(error && error.message ? error.message : String(error));
            }
          })
          .then(function () {
            close();
          });
        return false;
      },
      onClose: function () {
        if (unbind) {
          unbind();
        }
        if (picker) {
          picker.destroy();
        }
      },
      body: function (body) {
        bodyEl = body;
        var showExisting = bookmark.reminderAt && Date.parse(bookmark.reminderAt) > Date.now();
        body.innerHTML =
          '<div class="control-group"><input aria-label="' +
          escapeHtml(t("post.bookmarks.name_input_label")) +
          '" class="ember-text-field bookmark-name" id="bookmark-name" name="bookmark-name" placeholder="' +
          escapeHtml(t("post.bookmarks.name_placeholder")) +
          '" type="text"></div>' +
          '<div class="bookmark-options-panel"><label class="control-label" for="bookmark_auto_delete_preference">' +
          escapeHtml(t("bookmarks.auto_delete_preference.label")) +
          "</label></div>" +
          (showExisting
            ? '<div class="alert alert-info existing-reminder-at-alert">' +
              icon("far-clock") +
              "<span>" +
              escapeHtml(
                t("bookmarks.reminders.existing_reminder", {
                  at_date_time: formattedReminderTime(bookmark.reminderAt, timezone),
                })
              ) +
              "</span></div>"
            : "") +
          '<div class="control-group"><label class="control-label">' +
          escapeHtml(t("post.bookmarks.set_reminder")) +
          '</label><div class="bookmark-time-shortcuts"></div></div>';
        var name = body.querySelector("#bookmark-name");
        name.value = bookmark.name;
        name.addEventListener("input", function () {
          bookmark.name = name.value;
        });
        name.addEventListener("keydown", function (event) {
          if (event.key === "Enter") {
            event.preventDefault();
            saveAndClose();
          }
        });
        body.querySelector(".bookmark-options-panel").appendChild(
          comboBox({
            id: "bookmark-auto-delete-preference",
            className: "bookmark-option-selector",
            content: AUTO_DELETE.map(function (option) {
              return { id: option[1], name: t("bookmarks.auto_delete_preference." + option[0]) };
            }),
            value: bookmark.autoDeletePreference,
            onChange: function (value) {
              bookmark.autoDeletePreference = Number(value);
            },
          }).element
        );
        var shortcuts = body.querySelector(".bookmark-time-shortcuts");
        if (!timezone) {
          // userHasTimezoneSet
          shortcuts.innerHTML = '<div class="alert alert-info">' + t("bookmarks.no_timezone", { basePath: base() }) + "</div>";
          return;
        }
        // #initializeExistingBookmarkData
        if (hasReminder) {
          bookmark.selectedReminderType = null;
          bookmark.selectedDatetime = parseCustomDatetime(bookmark.reminderAt, null, timezone);
        }
        // #loadPostLocalDates: a local date in the bookmarked post is
        // offered as a time.
        var local = null;
        if (bookmark.bookmarkableType === "Post") {
          var post = document.querySelector('[data-post-id="' + bookmark.bookmarkableId + '"]');
          var el = post && post.querySelector(".discourse-local-date");
          if (el && el.dataset.date && el.dataset.time) {
            var parsed = parseCustomDatetime(el.dataset.date, el.dataset.time, timezone, el.dataset.timezone);
            if (parsed.isValid() && !(parsed < now(timezone))) {
              local = parsed;
            }
          }
        }
        picker = mountPicker(shortcuts, {
          timezone: timezone,
          prefilledDatetime: hasReminder ? bookmark.reminderAt : null,
          // An existing bookmark without a reminder has none to remove.
          hiddenOptions: hasReminder ? [] : ["none"],
          customLabels: hasReminder ? { none: "bookmarks.remove_reminder_keep_bookmark" } : {},
          timeShortcuts: function () {
            var list = defaultTimeShortcuts(timezone);
            if (local) {
              list.push({
                id: "post_local_date",
                label: "time_shortcut.post_local_date",
                time: local,
                format: "dates.long_no_year",
              });
            }
            return list;
          },
          // onTimeSelected: the custom and relative times wait for Save.
          onTimeSelected: function (type, time) {
            bookmark.selectedReminderType = type;
            bookmark.selectedDatetime = time;
            if (type !== "custom" && type !== "relative") {
              saveAndClose();
            }
          },
          keys: function (bindings) {
            bindings.enter = saveAndClose;
            bindings["d d"] = destroy;
            unbind = keySequences(body, bindings);
          },
        });
      },
      footer: function (footer) {
        var html = "";
        if (!mobileView()) {
          html +=
            '<button class="btn btn-text btn-primary" id="save-bookmark" type="button"><span class="d-button-label">' +
            escapeHtml(t("bookmarks.save")) +
            '</span></button><button class="btn btn-text btn-transparent d-modal-cancel" type="button"><span class="d-button-label">' +
            escapeHtml(t("cancel")) +
            "</span></button>";
        }
        html +=
          '<button class="btn no-text btn-icon delete-bookmark btn-danger" id="delete-bookmark" aria-label="' +
          escapeHtml(t("post.bookmarks.actions.delete_bookmark.name")) +
          '" title="' +
          escapeHtml(t("post.bookmarks.actions.delete_bookmark.name")) +
          '" type="button">' +
          icon("trash-can") +
          '<span aria-hidden="true">&#8203;</span></button>';
        footer.innerHTML = html;
        footer.addEventListener("click", function (event) {
          var button = event.target.closest("button");
          if (!button) {
            return;
          }
          if (button.id === "save-bookmark") {
            saveAndClose();
          } else if (button.id === "delete-bookmark") {
            destroy();
          } else if (button.classList.contains("d-modal-cancel")) {
            // closeWithoutSavingBookmark
            closeWithoutSaving = true;
            close();
          }
        });
      },
    });
    // didInsert: a phone keeps its keyboard down.
    var nameInput = document.getElementById("bookmark-name");
    if (nameInput && !mobileView()) {
      nameInput.focus();
    }
  }

  // DFutureDateInput with @noRelativeOptions (ui-kit/d-future-date-input):
  // a date (DatePickerFuture) and a time, no shortcuts to choose from.
  // opts: { input (a time moment reads), onChangeInput(value) } where the
  // value is "YYYY-MM-DD HH:mmZ" in the browser's zone (FORMAT), or null
  // without a date. Called once load() has resolved.
  function futureDateInput(root, opts) {
    var m = window.moment;
    var date = null;
    var time = null;
    var picker = null;
    var destroyed = false;
    if (opts.input) {
      var at = m(opts.input);
      if (at.isValid()) {
        date = at.format("YYYY-MM-DD");
        time = at.format("HH:mm");
      }
    }
    root.innerHTML =
      '<div class="future-date-input"><div class="control-group future-date-input-date-picker">' +
      icon("calendar-days") +
      '<div class="date-picker-wrapper"><input class="date-picker" autocomplete="off" placeholder="' +
      escapeHtml(t("dates.placeholder")) +
      '" type="' +
      (mobileView() ? "date" : "text") +
      '"></div></div><div class="control-group future-date-input-time-picker">' +
      icon("far-clock") +
      '<input class="time-input" placeholder="--:--" type="time"></div></div>';
    var dateInput = root.querySelector(".date-picker");
    var timeInput = root.querySelector(".time-input");
    dateInput.value = date || "";
    timeInput.value = time || "";
    timeInput.disabled = !date;

    // _dateTimeChanged
    function changed() {
      var dateTime = m((date || "") + (time ? " " + time : ""));
      opts.onChangeInput(date && dateTime.isValid() ? dateTime.locale("en").format("YYYY-MM-DD HH:mmZ") : null);
    }
    // onChangeDate: no date takes the time with it.
    function onDate(value) {
      date = value || null;
      if (!date) {
        time = null;
        timeInput.value = "";
      }
      timeInput.disabled = !date;
      changed();
    }
    timeInput.addEventListener("input", function () {
      // onChangeTime
      if (date) {
        time = timeInput.value || null;
        changed();
      }
    });
    // A date typed in (the phone's date input, or the field before or
    // without Pikaday), or the field emptied.
    dateInput.addEventListener("change", function () {
      var value = dateInput.value.trim();
      if (!value) {
        onDate(null);
      } else if (m(value, "YYYY-MM-DD", true).isValid() && value !== date) {
        onDate(value);
      }
    });
    if (!mobileView()) {
      loadPikaday()
        .then(function () {
          if (destroyed || !dateInput.isConnected) {
            return;
          }
          picker = new window.Pikaday({
            field: dateInput,
            container: null,
            bound: true,
            format: "YYYY-MM-DD",
            firstDay: 1,
            minDate: m().toDate(),
            defaultDate: date ? m(date).toDate() : m().add(1, "day").toDate(),
            setDefaultDate: !!date,
            i18n: {
              previousMonth: t("dates.previous_month"),
              nextMonth: t("dates.next_month"),
              months: m.months(),
              weekdays: m.weekdays(),
              weekdaysShort: m.weekdaysMin(),
            },
            onSelect: function (d) {
              var value = m(d).format("YYYY-MM-DD");
              dateInput.value = value;
              if (value !== date) {
                onDate(value);
              }
            },
          });
        })
        // Without the picker the field still takes a typed date.
        .catch(function () {});
    }
    return {
      destroy: function () {
        destroyed = true;
        if (picker) {
          picker.destroy();
          picker = null;
        }
      },
    };
  }

  Discourse.bookmarkModal = { open: open, t: t };
  Discourse.timeShortcutPicker = {
    mount: function (root, opts) {
      return mountPicker(root, opts);
    },
    futureDateInput: futureDateInput,
    load: load,
    // timeShortcuts in the member's timezone, else the browser's (once
    // loaded).
    shortcuts: function () {
      return timeShortcuts(s.timezone || window.moment.tz.guess());
    },
  };
})();
