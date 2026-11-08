// discourse-poll's builder (components/modal/poll-ui-builder.gjs with
// lib/poll-settings.js), the composer options menu's Build poll: the
// poll's type, its options (one input each, or one per line in advanced
// mode), its min, max and step, who voted shown, and in advanced mode its
// title, dynamic options, groups, automatic close, results and chart,
// inserted at the cursor as the [poll] markdown it builds. The groups are
// select-kit's GroupChooser, the automatic close DDateTimeInput (DDateInput
// and DTimeInput) and the results a ComboBox, as below. Loaded by
// static/js/composer.js the first time it is asked for, with the strings
// and settings the composer's data-poll-builder carries.
//
//   Discourse.pollBuilder.show(config, { getText, addText }, edit)
//
// edit is the rich editor's Edit poll ({ poll, onSave }): its settings,
// saved as the poll's attrs.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.pollBuilder) {
    return;
  }

  var REGULAR = "regular";
  var MULTIPLE = "multiple";
  var NUMBER = "number";
  var RANKED = "ranked_choice";
  var ALWAYS = "always";

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function svg(name) {
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  function isBlank(v) {
    return v === null || v === undefined || String(v).trim() === "";
  }

  // lib/text serializeBBCodeAttr: quoted when the value has a space or `]`.
  function bbcodeAttr(value, name) {
    if (!value) {
      return "";
    }
    var s = String(value);
    if (!/[\s\]]/.test(s)) {
      return name + "=" + s;
    }
    return name + '="' + s.replace(/"/g, "") + '"';
  }

  // buildBBCodeAttrs
  function bbcodeAttrs(attrs) {
    return Object.keys(attrs)
      .map(function (key) {
        return attrs[key] === null || attrs[key] === undefined ? "" : bbcodeAttr(attrs[key], key);
      })
      .filter(Boolean)
      .join(" ");
  }

  function rangeAttr(value) {
    if (isBlank(value) || !isFinite(Number(value))) {
      return null;
    }
    return String(Number(value));
  }

  // select-kit's normalize (lib/input-utils): lowercased, accents gone.
  function normalize(s) {
    s = String(s == null ? "" : s).toLowerCase();
    return typeof s.normalize === "function" ? s.normalize("NFD").replace(/[̀-ͯ]/g, "") : s;
  }

  // The select-kit body placed under its header (updateFloatingUiPosition).
  function floatBody(summary, body) {
    var fui = window.FloatingUIDOM;
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    body.style.minWidth = Math.round(summary.getBoundingClientRect().width) + "px";
    if (!fui) {
      return;
    }
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

  function rowHtml(item, index, opts) {
    var label = item.isNew ? opts.t("select_kit.create", { content: item.name }) : item.name;
    return (
      '<li tabindex="0" title="' + escapeHtml(item.name) + '" data-value="' + escapeHtml(item.id) +
      '" data-name="' + escapeHtml(item.name) + '" data-index="' + index + '" role="menuitemradio" aria-checked="' +
      !!item.selected + '" class="select-kit-row' + (item.isNew ? " create" : "") +
      (item.highlighted ? " is-highlighted" : "") + (item.selected ? " is-selected" : "") + '"><span class="name">' +
      escapeHtml(label) + "</span></li>"
    );
  }

  // What select-kit's keys do in its filter: the arrows move the
  // highlight, Enter selects the highlighted row (Escape is DModal's,
  // static/js/modal.js, which closes the open select-kit first).
  function filterKeys(input, collection, select) {
    input.addEventListener("keydown", function (event) {
      var rows = collection();
      var at = rows.findIndex(function (r) {
        return r.classList.contains("is-highlighted");
      });
      var move = function (to) {
        rows.forEach(function (r) {
          r.classList.remove("is-highlighted");
        });
        if (rows[to]) {
          rows[to].classList.add("is-highlighted");
          rows[to].scrollIntoView({ block: "nearest" });
        }
      };
      if (event.key === "ArrowDown") {
        event.preventDefault();
        move(at === -1 ? 0 : Math.min(at + 1, rows.length - 1));
      } else if (event.key === "ArrowUp") {
        event.preventDefault();
        move(at === -1 ? rows.length - 1 : Math.max(at - 1, 0));
      } else if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        if (at !== -1) {
          select(rows[at]);
        }
      }
    });
  }

  // A select-kit ComboBox (single-select combobox combo-box):
  //   { className, content: [{ id, name }], value, none (translatedNone),
  //     allowAny, filterPlaceholder, onChange(id, or the typed text) }
  // The filter shows with allowAny; typed text that is no row's value is
  // offered as a "Create" row (allowAny's newItem) ahead of the matches.
  function comboBox(t, opts) {
    var details = document.createElement("details");
    details.className = "select-kit single-select combobox combo-box" + (opts.className ? " " + opts.className : "");
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
      var name = item ? item.name : opts.none || "";
      var label = item ? t("select_kit.filter_by", { name: name }) : t("select_kit.select_to_filter");
      summary.setAttribute("name", label);
      summary.setAttribute("aria-label", label);
      summary.dataset.name = item ? item.name : "";
      if (item) {
        summary.dataset.value = item.id;
        summary.removeAttribute("title");
      } else {
        delete summary.dataset.value;
        if (opts.none) {
          summary.title = opts.none;
        }
      }
      details.classList.toggle("has-selection", !!item);
      summary.querySelector(".select-kit-header-wrapper").innerHTML =
        '<div class="select-kit-selected-name selected-name choice" data-name="' +
        escapeHtml(name) + '"' + (item ? ' data-value="' + escapeHtml(item.id) + '"' : "") + ' title="' +
        escapeHtml(name) + '"><span class="name">' + escapeHtml(name) + "</span></div>" +
        svg(details.open ? "angle-up" : "angle-down").replace("svg-icon", "svg-icon angle-icon");
    }

    // search, then allowAny's newItem and the highlight (_searchWrapper).
    function rows(filter) {
      var term = normalize(filter);
      var list = opts.content
        .filter(function (c) {
          return !term || normalize(c.name).indexOf(term) !== -1;
        })
        .map(function (c) {
          return { id: c.id, name: c.name, selected: String(c.id) === String(value) };
        });
      if (
        opts.allowAny &&
        filter &&
        filter !== opts.none &&
        // validateCreate compares strictly: a typed "900" is no option's
        // 900.
        !opts.content.some(function (c) {
          return c.id === filter;
        }) &&
        value !== filter
      ) {
        list.unshift({ id: filter, name: filter, isNew: true });
      }
      var highlight = value
        ? list.find(function (r) {
            return r.selected;
          })
        : filter
          ? list[0]
          : null;
      if (highlight) {
        highlight.highlighted = true;
      }
      return list;
    }

    function drawRows(filter) {
      var list = rows(filter);
      var old = body.querySelector(".select-kit-collection");
      if (old) {
        old.remove();
      }
      if (list.length) {
        body.insertAdjacentHTML(
          "beforeend",
          '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
            list
              .map(function (r, i) {
                return rowHtml(r, i, { t: t });
              })
              .join("") +
            "</ul>"
        );
      }
      var current = body.querySelector(".select-kit-row.is-highlighted");
      if (current) {
        current.scrollIntoView({ block: "nearest" });
      }
    }

    function choose(row) {
      var isNew = row.classList.contains("create");
      var chosen = isNew ? row.dataset.value : opts.content.find(function (c) {
        return String(c.id) === row.dataset.value;
      });
      details.open = false;
      if (!isNew) {
        value = chosen.id;
      }
      drawHeader();
      opts.onChange(isNew ? chosen : chosen.id);
    }

    details.addEventListener("toggle", function () {
      details.classList.toggle("is-expanded", details.open);
      if (!details.open) {
        body.innerHTML = "";
        body.removeAttribute("style");
        drawHeader();
        return;
      }
      // SelectKitFilter#isHidden: shown with allowAny, or for ComboBox's
      // autoFilterable (FILTER_VISIBILITY_THRESHOLD rows or more).
      var filterable = opts.allowAny || opts.content.length >= 10;
      body.innerHTML = filterable
        ? '<div class="select-kit-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
          escapeHtml(
            opts.filterPlaceholder ||
              t(opts.allowAny ? "select_kit.filter_placeholder_with_any" : "select_kit.filter_placeholder")
          ) +
          '" spellcheck="false" tabindex="0" type="search"></div>'
        : '<div class="select-kit-filter"></div>';
      drawRows("");
      drawHeader();
      floatBody(summary, body);
      var input = body.querySelector(".filter-input");
      if (input) {
        input.addEventListener("input", function () {
          drawRows(input.value);
        });
        filterKeys(
          input,
          function () {
            return Array.prototype.slice.call(body.querySelectorAll(".select-kit-row"));
          },
          choose
        );
        input.focus();
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
        choose(row);
      }
    });
    drawHeader();
    return details;
  }

  // GroupChooser (select-kit/components/group-chooser over MultiSelect)
  // with the names given, valueProperty and labelProperty "name":
  //   { content: [names], value: [names], onChange(names) }
  // The rows leave out the chosen groups and show at most
  // displayedGroupsLimit (100) with FilterForMore's tip past it; a row
  // clicked joins the choices and the menu stays open (closeOnChange
  // false), a choice clicked leaves them.
  var DISPLAYED_GROUPS_LIMIT = 100;

  function groupChooser(t, opts) {
    var details = document.createElement("details");
    details.className = "select-kit multi-select group-chooser";
    details.innerHTML =
      '<summary tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div>';
    var summary = details.querySelector("summary");
    var body = details.querySelector(".select-kit-body");
    var value = (opts.value || []).filter(function (v) {
      return opts.content.indexOf(v) !== -1;
    });

    function drawHeader() {
      summary.dataset.value = value.join(",");
      summary.dataset.name = value.join(",");
      if (value.length) {
        summary.removeAttribute("title");
      } else {
        summary.title = t("select_kit.default_header_text");
      }
      details.classList.toggle("has-selection", value.length > 0);
      summary.querySelector(".select-kit-header-wrapper").innerHTML =
        '<span class="formatted-selection">' +
        escapeHtml(value.length ? value.join(", ") : t("select_kit.default_header_text")) +
        "</span>" +
        svg("plus").replace("svg-icon", "svg-icon angle-icon");
    }

    function drawBody(filter) {
      var term = normalize(filter);
      var found = opts.content.filter(function (name) {
        return value.indexOf(name) === -1 && (!term || normalize(name).indexOf(term) !== -1);
      });
      var more = found.length > DISPLAYED_GROUPS_LIMIT;
      found = found.slice(0, DISPLAYED_GROUPS_LIMIT);
      var html = "";
      if (value.length) {
        html +=
          '<div class="selected-content">' +
          value
            .map(function (name) {
              return (
                '<button aria-label="' + escapeHtml(t("select_kit.delete_item", { name: name })) +
                '" class="btn btn-default selected-choice tag-choice" data-name="' + escapeHtml(name) +
                '" data-value="' + escapeHtml(name) + '" type="button">' + svg("xmark") +
                '<span class="d-button-label">' + escapeHtml(name) + "</span></button>"
              );
            })
            .join("") +
          "</div>";
      }
      if (found.length) {
        html +=
          '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
          found
            .map(function (name, i) {
              return rowHtml({ id: name, name: name, highlighted: !!filter && i === 0 }, i, { t: t });
            })
            .join("") +
          "</ul>";
      }
      if (more) {
        html += '<div class="filter-for-more">' + escapeHtml(t("select_kit.components.filter_for_more")) + "</div>";
      }
      if (filter) {
        html += found.length
          ? '<span class="results-count" role="alert">' +
            escapeHtml(t(found.length === 1 ? "select_kit.results_count.one" : "select_kit.results_count.other", { count: found.length })) +
            "</span>"
          : '<span class="no-content" role="alert">' + escapeHtml(t("select_kit.no_content")) + "</span>";
      }
      var filterEl = body.querySelector(".select-kit-filter");
      while (filterEl.nextSibling) {
        filterEl.nextSibling.remove();
      }
      filterEl.insertAdjacentHTML("afterend", html);
    }

    function change(next) {
      value = next;
      var input = body.querySelector(".filter-input");
      if (input) {
        input.value = "";
        input.focus();
      }
      drawHeader();
      if (details.open) {
        drawBody("");
        floatBody(summary, body);
      }
      opts.onChange(value.slice());
    }

    details.addEventListener("toggle", function () {
      details.classList.toggle("is-expanded", details.open);
      if (!details.open) {
        body.innerHTML = "";
        body.removeAttribute("style");
        return;
      }
      body.innerHTML =
        '<div class="select-kit-filter multi-select-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
        escapeHtml(t("select_kit.filter_placeholder")) +
        '" spellcheck="false" tabindex="0" type="search"></div>';
      drawBody("");
      floatBody(summary, body);
      var input = body.querySelector(".filter-input");
      input.addEventListener("input", function () {
        drawBody(input.value);
      });
      filterKeys(
        input,
        function () {
          return Array.prototype.slice.call(body.querySelectorAll(".select-kit-row"));
        },
        function (row) {
          change(value.concat([row.dataset.value]));
        }
      );
      // Backspace in an empty filter takes the last choice off.
      input.addEventListener("keydown", function (event) {
        if (event.key === "Backspace" && input.value === "" && value.length) {
          event.preventDefault();
          change(value.slice(0, -1));
        }
      });
      input.focus();
    });
    body.addEventListener("click", function (event) {
      var choice = event.target.closest(".selected-choice");
      if (choice) {
        event.preventDefault();
        change(
          value.filter(function (v) {
            return v !== choice.dataset.value;
          })
        );
        return;
      }
      var row = event.target.closest(".select-kit-row");
      if (row) {
        event.preventDefault();
        change(value.concat([row.dataset.value]));
      }
    });
    drawHeader();
    return details;
  }

  // DTimeInput's options: 15 minute steps over the day (no relativeDate
  // here), with the time chosen added when it is not one of them.
  function uses12HourTime(t) {
    var format = t("dates.time");
    if (format && format !== "dates.time") {
      return /[aA]/.test(format.replace(/\[[^\]]*\]/g, ""));
    }
    return (document.documentElement.lang || "en").indexOf("en") === 0;
  }

  function minutesToString(n, twelve) {
    var hours = Math.floor(n / 60);
    var minutes = n % 60;
    var mm = String(minutes).padStart(2, "0");
    if (twelve) {
      return (hours % 12 || 12) + ":" + mm + " " + (hours >= 12 ? "PM" : "AM");
    }
    return String(hours).padStart(2, "0") + ":" + mm;
  }

  function timeOptions(time, twelve) {
    var options = [];
    for (var option = 0, i = 0; i <= 95; i++, option += 15) {
      options.push(option);
    }
    if (time !== null && options.indexOf(time) === -1) {
      options.push(time);
    }
    options.sort(function (a, b) {
      return a - b;
    });
    return options.map(function (o) {
      return { id: o, name: minutesToString(o, twelve) };
    });
  }

  var AM_PM = /^\s*(\d{1,2}):(\d{2})\s*(am|pm)\s*$/i;

  // DTimeInput#onChangeTime: an option's minutes, or what was typed
  // ("h:mm am" or "HH:MM"); null for what it can't read.
  function parseTime(time) {
    if (typeof time === "number") {
      return { hours: Math.floor(time / 60), minutes: time % 60 };
    }
    var hours;
    var minutes;
    var ampm = time.match(AM_PM);
    if (ampm) {
      hours = parseInt(ampm[1], 10);
      minutes = parseInt(ampm[2], 10);
      var pm = ampm[3].toLowerCase() === "pm";
      if (pm && hours !== 12) {
        hours += 12;
      } else if (!pm && hours === 12) {
        hours = 0;
      }
    } else {
      var parts = time.split(":");
      if (!parts[0] || !parts[1]) {
        return null;
      }
      hours = parseInt(parts[0], 10);
      minutes = parseInt(parts[1], 10);
    }
    if (!isFinite(hours) || !isFinite(minutes)) {
      return null;
    }
    return { hours: Math.max(0, Math.min(23, hours)), minutes: Math.max(0, Math.min(59, minutes)) };
  }

  // DDateInput's useNativePicker.
  function isInputDateSupported() {
    var input = document.createElement("input");
    input.setAttribute("type", "date");
    input.setAttribute("value", "a");
    return input.value !== "a";
  }

  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var script = document.createElement("script");
      script.src = src;
      script.onload = resolve;
      script.onerror = reject;
      document.head.appendChild(script);
    });
  }

  function ymd(date) {
    return (
      date.getFullYear() + "-" + String(date.getMonth() + 1).padStart(2, "0") + "-" + String(date.getDate()).padStart(2, "0")
    );
  }

  // DDateTimeInput (clearable, useGlobalPickerContainer) in the browser's
  // timezone (moment.tz.guess, no @timezone given): DDateInput's date
  // picker (the native date input, or Pikaday where there is none),
  // DTimeInput's combo box and the clear button.
  //   { date: Date or null, onChange(Date or null), assets: { pikaday, moment } }
  function dateTimeInput(t, opts) {
    var date = opts.date;
    var native = isInputDateSupported();
    var twelve = uses12HourTime(t);
    var picker = null;
    var el = document.createElement("div");
    el.className = "d-date-time-input";

    function minutesOf(d) {
      return d ? d.getHours() * 60 + d.getMinutes() : null;
    }

    // The new date given to the date picker (setDate, as didUpdateAttrs
    // does, unless it is the picker's own) and to the time input.
    function set(next, fromPicker) {
      date = next;
      if (!fromPicker) {
        setPickerDate();
      }
      drawTime();
      opts.onChange(date);
    }

    function setPickerDate() {
      var input = el.querySelector(".date-picker");
      if (native) {
        input.value = date ? ymd(date) : "";
      } else if (picker) {
        picker.setDate(date, true);
        if (!date) {
          input.value = "";
        }
      }
    }

    // DDateTimeInput#onChangeDate: the day chosen, at the time there was.
    function onChangeDate(day, fromPicker) {
      if (!day) {
        set(null, fromPicker);
        return;
      }
      set(new Date(day.getFullYear(), day.getMonth(), day.getDate(), date ? date.getHours() : 0, date ? date.getMinutes() : 0), fromPicker);
    }

    // DDateTimeInput#onChangeTime: the time on the day chosen, else today.
    function onChangeTime(time) {
      var parsed = parseTime(time);
      if (!parsed) {
        return;
      }
      var day = date || new Date();
      set(new Date(day.getFullYear(), day.getMonth(), day.getDate(), parsed.hours, parsed.minutes));
    }

    // DDateInput, drawn once: the date chosen reaches it through
    // setPickerDate, so typing in it is never interrupted.
    function drawDate() {
      el.innerHTML =
        '<div class="d-date-input"><input class="date-picker" placeholder="' + escapeHtml(t("dates.placeholder")) +
        '" type="' + (native ? "date" : "text") + '"></div><div class="d-time-input"></div>' +
        '<button class="btn no-text btn-icon btn-default clear-date-time" type="button">' + svg("xmark") +
        '<span aria-hidden="true">&#8203;</span></button>';
      var input = el.querySelector(".date-picker");
      if (native) {
        input.value = date ? ymd(date) : "";
        // The input event (onChangeDate) and the native picker's
        // onchange (_handleSelection) both select the day.
        var fromNative = function () {
          var value = input.value ? new Date(input.value + "T00:00") : null;
          var same = value && date && ymd(value) === ymd(date);
          if (!same) {
            onChangeDate(value, true);
          }
        };
        input.addEventListener("input", function () {
          input.classList.toggle("--has-value", !!input.value);
          fromNative();
        });
        input.addEventListener("change", fromNative);
      } else {
        loadPikaday().then(function () {
          if (!input.isConnected) {
            return;
          }
          var m = window.moment;
          picker = new window.Pikaday({
            field: input,
            container: null,
            bound: true,
            format: "LL",
            firstDay: 1,
            i18n: {
              previousMonth: t("dates.previous_month"),
              nextMonth: t("dates.next_month"),
              months: m.months(),
              weekdays: m.weekdays(),
              weekdaysShort: m.weekdaysShort(),
            },
            onSelect: function (d) {
              onChangeDate(d, true);
            },
          });
          if (date) {
            picker.setDate(date, true);
          }
        });
        input.addEventListener("input", function () {
          input.classList.toggle("--has-value", !!input.value);
          if (!input.value) {
            onChangeDate(null, true);
          }
        });
      }
      el.querySelector(".clear-date-time").addEventListener("click", function () {
        set(null);
      });
      drawTime();
    }

    // DTimeInput, drawn again for each date (its options follow the time).
    function drawTime() {
      var time = minutesOf(date);
      var holder = el.querySelector(".d-time-input");
      holder.innerHTML = "";
      holder.appendChild(
        comboBox(t, {
          content: timeOptions(time, twelve),
          value: time,
          none: "--:--",
          allowAny: true,
          filterPlaceholder: "--:--",
          onChange: onChangeTime,
        })
      );
    }

    function loadPikaday() {
      var assets = opts.assets || {};
      var moment = window.moment ? Promise.resolve() : loadScript(assets.moment);
      return moment.then(function () {
        return window.Pikaday ? null : loadScript(assets.pikaday);
      });
    }

    drawDate();
    return el;
  }

  // moment(close): a date alone is local midnight.
  function parseClose(close) {
    var date = /^(\d{4})-(\d{2})-(\d{2})$/.exec(close);
    var parsed = date ? new Date(+date[1], +date[2] - 1, +date[3]) : new Date(close);
    return isNaN(parsed.getTime()) ? null : parsed;
  }

  // edit: the rich editor's Edit poll, { poll: its node's attrs and
  // optionCount, onSave(attrs) }. The options and title are edited in the
  // document, so the builder shows its settings alone, all of them.
  function show(config, composer, edit) {
    var strings = config.strings || {};
    var t = function (key, vars) {
      var s = strings[key] || key;
      Object.keys(vars || {}).forEach(function (k) {
        s = s.split("%{" + k + "}").join(vars[k]);
      });
      return s;
    };
    var maxOptions = config.maxOptions || 20;
    var m = {
      chartType: "bar",
      dynamic: false,
      autoClose: null,
      groups: [],
      max: 2,
      min: 1,
      optionsText: "",
      result: ALWAYS,
      step: 1,
      title: "",
      type: REGULAR,
      publicPoll: !!config.defaultPublic,
      advanced: false,
      options: [""],
    };
    var focusIndex = 0;
    var poll = edit ? edit.poll : null;
    var isEditing = !!poll;

    var isNumber = function () {
      return m.type === NUMBER;
    };
    var isMultiple = function () {
      return m.type === MULTIPLE;
    };
    var hasRange = function () {
      return isMultiple() || isNumber();
    };
    var optionsCount = function () {
      if (isEditing) {
        return poll.optionCount;
      }
      return m.options.filter(function (o) {
        return o.trim();
      }).length;
    };
    var given = function (value, fallback) {
      return value === null || value === undefined ? fallback : value;
    };

    if (poll) {
      m.type = poll.type || REGULAR;
      m.chartType = poll.chartType || "bar";
      m.dynamic = poll.dynamic === "true";
      m.autoClose = poll.close ? parseClose(poll.close) : null;
      m.groups = poll.groups ? poll.groups.split(",") : [];
      m.min = Number(given(poll.min, 1));
      m.max = Number(given(poll.max, isNumber() ? maxOptions : poll.optionCount));
      m.step = Number(given(poll.step, 1));
      m.result = poll.results || ALWAYS;
      m.publicPoll = poll.public === "true";
      m.advanced = true;
    }

    // Option counts are edited in the document, not here.
    function minNumOfOptionsValidation() {
      if (!isNumber() && !isEditing) {
        if (optionsCount() < 1) {
          return { failed: true, reason: t("poll.ui_builder.help.options_min_count") };
        }
        if (optionsCount() > maxOptions) {
          return { failed: true, reason: t("poll.ui_builder.help.options_max_count", { count: maxOptions }) };
        }
      }
      return { ok: true };
    }

    function showMinNumOfOptionsValidation() {
      return m.options.length !== 1 || m.options[0] !== "";
    }

    function minMaxValueValidation() {
      if (!hasRange()) {
        return { ok: true };
      }
      var min = parseInt(m.min, 10) || 0;
      var max = parseInt(m.max, 10) || 0;
      var step = parseInt(m.step, 10) || 0;
      if (isBlank(m.min) || min < 0) {
        return { failed: true, reason: t("poll.ui_builder.help.invalid_min_value") };
      }
      if (isBlank(m.max) || max < 0 || (isMultiple() && max > optionsCount())) {
        return { failed: true, reason: t("poll.ui_builder.help.invalid_max_value") };
      }
      if (min > max) {
        return { failed: true, reason: t("poll.ui_builder.help.invalid_values") };
      }
      if (isNumber()) {
        if (step < 1) {
          return { failed: true, reason: t("poll.ui_builder.help.min_step_value") };
        }
        var count = (max - min + 1) / step;
        if (count < 1) {
          return { failed: true, reason: t("poll.ui_builder.help.options_min_count") };
        }
        if (count > maxOptions) {
          return { failed: true, reason: t("poll.ui_builder.help.options_max_count", { count: maxOptions }) };
        }
      }
      return { ok: true };
    }

    function disableInsert() {
      return !minMaxValueValidation().ok || !minNumOfOptionsValidation().ok;
    }

    // #enforceMinMaxValues
    function enforceMinMax() {
      if (!isMultiple()) {
        return;
      }
      var count = optionsCount();
      if (m.min <= 0 || m.min >= m.max || m.min >= count) {
        m.min = count > 0 ? 1 : 0;
      }
      if (m.max <= 0 || m.min >= m.max || m.max > count) {
        m.max = count;
      }
    }

    // The automatic close, as DDateTimeInput gives it (pollAutoClose's
    // toISOString).
    function closeIso() {
      return m.autoClose && !isNaN(m.autoClose.getTime()) ? m.autoClose.toISOString() : null;
    }

    // lib/poll-settings.js pollAttrsFromSettings: an edited poll keeps its
    // name, status, order and its close as written while it is the same
    // moment, and gains no default it did not carry.
    function pollAttrs() {
      var close = closeIso();
      var originalClose = poll ? poll.close : null;
      if (close && originalClose) {
        var original = parseClose(originalClose);
        if (original && original.getTime() === m.autoClose.getTime()) {
          close = originalClose;
        }
      }
      var attrs = {
        name: poll ? given(poll.name, null) : null,
        type: m.type || null,
        results: m.result || null,
        min: hasRange() ? rangeAttr(m.min) : null,
        max: hasRange() ? rangeAttr(m.max) : null,
        step: isNumber() ? String(Math.max(Number(m.step) || 1, 1)) : null,
        public: String(m.publicPoll),
        chartType: !isNumber() && m.chartType ? m.chartType : null,
        dynamic: m.dynamic ? "true" : null,
        groups: m.groups.length ? m.groups.join(",") : null,
        close: close,
        status: poll ? given(poll.status, null) : null,
        order: poll ? given(poll.order, null) : null,
      };
      if (poll) {
        var implied = {
          type: REGULAR,
          results: ALWAYS,
          public: "false",
          chartType: "bar",
          min: "1",
          max: String(isNumber() ? maxOptions : poll.optionCount),
          step: "1",
        };
        // Omitted defaults stay omitted, so the bounds still follow the
        // options as they change.
        Object.keys(implied).forEach(function (name) {
          if (attrs[name] === implied[name] && !poll[name]) {
            attrs[name] = null;
          }
        });
      }
      return attrs;
    }

    // #pollOutput: named after the polls the text has already.
    function pollOutput() {
      var attrs = pollAttrs();
      var existing = composer.getText().match(/\[poll(\s+name=[^\s\]]+)*.*\]/gim);
      if (existing) {
        attrs.name = "poll" + (existing.length + 1);
      }
      var built = bbcodeAttrs(attrs);
      var title = m.title ? "# " + m.title.trim() + "\n" : "";
      var options = isNumber()
        ? ""
        : m.options
            .map(function (o) {
              return o.trim();
            })
            .filter(Boolean)
            .map(function (o) {
              return "* " + o + "\n";
            })
            .join("");
      return "[poll" + (built ? " " + built : "") + "]\n" + title + options + "[/poll]\n";
    }

    function tip(validation) {
      return (
        '<div class="tip ' + (validation.failed ? "bad" : "good") + '">' +
        (validation.reason ? svg(validation.failed ? "xmark" : "check") + escapeHtml(validation.reason) : "") +
        "</div>"
      );
    }

    function toggleSwitch(cls, on, label) {
      return (
        '<div class="d-toggle-switch"><label class="d-toggle-switch__label"><button aria-checked="' + on +
        '" class="d-toggle-switch__checkbox ' + cls + '" role="switch" type="button"></button>' +
        '<span class="d-toggle-switch__checkbox-slider">' + (on ? svg("check") : "") + "</span></label>" +
        '<span class="d-toggle-switch__checkbox-label">' + escapeHtml(label) + "</span></div>"
      );
    }

    function typePill(type, key) {
      return (
        '<li><button class="btn no-text poll-type-value poll-type-value-' + type.replace("_", "-") +
        (m.type === type ? " active" : "") + '" data-poll-type="' + type + '" type="button">' +
        escapeHtml(t("poll.ui_builder.poll_type." + key)) + "</button></li>"
      );
    }

    function bodyHtml() {
      var html = '<ul class="nav nav-pills poll-type">' + typePill(REGULAR, "regular") + typePill(MULTIPLE, "multiple");
      if (m.advanced || isNumber()) {
        html += typePill(NUMBER, "number");
      }
      if (m.advanced || m.type === RANKED) {
        html += typePill(RANKED, "ranked_choice");
      }
      html += "</ul>";
      // showTitle
      if (m.advanced && !isEditing) {
        html +=
          '<div class="input-group poll-title"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_title.label")) + '</label><input type="text" value="' +
          escapeHtml(m.title) + '"></div>';
      }
      // showOptions
      if (!isNumber() && !isEditing) {
        html += '<div class="poll-options">';
        var count = minNumOfOptionsValidation();
        if (m.advanced) {
          html +=
            '<label class="input-group-label">' + escapeHtml(t("poll.ui_builder.poll_options.label")) +
            "</label><textarea>" + escapeHtml(m.optionsText) + "</textarea>";
          if (showMinNumOfOptionsValidation() && !count.ok) {
            html += tip(count);
          }
        } else {
          m.options.forEach(function (option, index) {
            html +=
              '<div class="input-group poll-option-value"><input data-index="' + index + '" type="text" value="' +
              escapeHtml(option) + '">' +
              (m.options.length > 1
                ? '<button class="btn no-text btn-icon" data-remove="' + index + '" type="button">' + svg("trash-can") +
                  '<span aria-hidden="true">&#8203;</span></button>'
                : "") +
              "</div>";
          });
          html +=
            '<div class="poll-option-controls"><button class="btn btn-icon-text btn-default poll-option-add" type="button">' +
            svg("plus") + '<span class="d-button-label">' + escapeHtml(t("poll.ui_builder.poll_options.add")) +
            "</span></button>" + (showMinNumOfOptionsValidation() && !count.ok ? tip(count) : "") + "</div>";
        }
        html += "</div>";
      }
      if (hasRange()) {
        html +=
          '<div class="options"><div class="input-group poll-number"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_config.min")) + '</label><input class="poll-options-min" min="1" type="number" value="' +
          escapeHtml(m.min) + '"></div><div class="input-group poll-number"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_config.max")) + '</label><input class="poll-options-max" min="1" type="number" value="' +
          escapeHtml(m.max) + '"></div>';
        if (isNumber()) {
          html +=
            '<div class="input-group poll-number"><label class="input-group-label">' +
            escapeHtml(t("poll.ui_builder.poll_config.step")) + '</label><input class="poll-options-step" min="1" type="number" value="' +
            escapeHtml(m.step) + '"></div>';
        }
        html += "</div>";
        var range = minMaxValueValidation();
        if (!range.ok) {
          html += tip(range);
        }
      }
      html +=
        '<div class="input-group poll-public">' +
        toggleSwitch("poll-toggle-public", m.publicPoll, t("poll.ui_builder.poll_public.label")) + "</div>";
      if (m.advanced) {
        html +=
          '<div class="input-group poll-dynamic">' +
          toggleSwitch("poll-toggle-dynamic", m.dynamic, t("poll.ui_builder.poll_dynamic.label")) + "</div>";
        html +=
          '<div class="input-group poll-allowed-groups"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_groups.label")) + "</label></div>";
        html +=
          '<div class="input-group poll-date"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.automatic_close.label")) + "</label></div>";
        html +=
          '<div class="input-group poll-select"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_result.label")) + "</label></div>";
        if (!(m.type === RANKED || isNumber())) {
          html +=
            '<div class="input-group poll-select column"><label class="input-group-label">' +
            escapeHtml(t("poll.ui_builder.poll_chart_type.label")) + "</label>";
          [
            ["bar", "chart-bar"],
            ["pie", "chart-pie"],
          ].forEach(function (c) {
            html +=
              '<div class="radio-group"><input id="poll-chart-type-' + c[0] + '" name="poll-chart-type" type="radio" value="' +
              c[0] + '"' + (m.chartType === c[0] ? " checked" : "") + '><label for="poll-chart-type-' + c[0] + '">' +
              svg(c[1]) + " " + escapeHtml(t("poll.ui_builder.poll_chart_type." + c[0])) + "</label></div>";
          });
          html += "</div>";
        }
      }
      return html;
    }

    var body = null;
    var insertButton = null;
    var advancedButton = null;

    // pollResults: staff_only for staff.
    function pollResults() {
      var results = [
        { id: ALWAYS, name: t("poll.ui_builder.poll_result.always") },
        { id: "on_vote", name: t("poll.ui_builder.poll_result.vote") },
        { id: "on_close", name: t("poll.ui_builder.poll_result.closed") },
      ];
      if (config.staff) {
        results.push({ id: "staff_only", name: t("poll.ui_builder.poll_result.staff") });
      }
      return results;
    }

    // Advanced mode's GroupChooser, DDateTimeInput and results ComboBox.
    function mountWidgets() {
      var groups = body.querySelector(".poll-allowed-groups");
      if (groups) {
        groups.appendChild(
          groupChooser(t, {
            content: config.groups || [],
            value: m.groups,
            onChange: function (names) {
              m.groups = names;
            },
          })
        );
      }
      var close = body.querySelector(".poll-date");
      if (close) {
        close.appendChild(
          dateTimeInput(t, {
            date: m.autoClose,
            assets: config.assets,
            onChange: function (date) {
              m.autoClose = date;
            },
          })
        );
      }
      var result = body.querySelector(".poll-select:not(.column)");
      if (result) {
        result.appendChild(
          comboBox(t, {
            className: "poll-result",
            content: pollResults(),
            value: m.result,
            onChange: function (value) {
              m.result = value;
            },
          })
        );
      }
    }

    function redraw() {
      body.innerHTML = bodyHtml();
      mountWidgets();
      insertButton.disabled = disableInsert();
      if (advancedButton) {
        advancedButton.querySelector(".d-button-label").textContent = t(
          m.advanced ? "advanced_mode_toggle.simple_mode" : "advanced_mode_toggle.advanced_mode"
        );
      }
      var input = body.querySelector('.poll-option-value input[data-index="' + focusIndex + '"]');
      if (input) {
        input.focus();
        input.setSelectionRange(input.value.length, input.value.length);
      }
    }

    // What typing changes, without drawing the field being typed in again:
    // the tips, the min and max the options bound, and the insert button.
    function refresh() {
      var holder = body.querySelector(m.advanced ? ".poll-options" : ".poll-option-controls");
      if (holder) {
        var old = holder.querySelector(":scope > .tip");
        if (old) {
          old.remove();
        }
        var count = minNumOfOptionsValidation();
        if (showMinNumOfOptionsValidation() && !count.ok) {
          holder.insertAdjacentHTML("beforeend", tip(count));
        }
      }
      var options = body.querySelector(":scope > .options");
      if (options) {
        var next = options.nextElementSibling;
        if (next && next.classList.contains("tip")) {
          next.remove();
        }
        var range = minMaxValueValidation();
        if (!range.ok) {
          options.insertAdjacentHTML("afterend", tip(range));
        }
        [
          [".poll-options-min", m.min],
          [".poll-options-max", m.max],
        ].forEach(function (pair) {
          var input = options.querySelector(pair[0]);
          if (input && input.value !== String(pair[1])) {
            input.value = pair[1];
          }
        });
      }
      insertButton.disabled = disableInsert();
    }

    function addOption(at) {
      if (at === -1) {
        at = m.options.length;
      }
      m.options.splice(at, 0, "");
      focusIndex = at;
      enforceMinMax();
      redraw();
    }

    var close = Discourse.modal.show({
      className: "poll-ui-builder",
      title: t(isEditing ? "poll.ui_builder.edit" : "poll.ui_builder.title"),
      body: function (el) {
        body = el;
      },
      footer: function (el, closeModal) {
        el.innerHTML =
          '<button class="btn btn-icon-text btn-primary insert-poll" type="button">' + svg("chart-bar") +
          '<span class="d-button-label">' + escapeHtml(t(isEditing ? "save" : "poll.ui_builder.insert")) + "</span></button>" +
          '<button class="btn btn-text btn-flat" type="button"><span class="d-button-label">' + escapeHtml(t("cancel")) +
          "</span></button>" +
          (isEditing
            ? ""
            : '<button class="btn btn-icon-text btn-default advanced-mode-btn" type="button">' + svg("gear") +
              '<span class="d-button-label"></span></button>');
        insertButton = el.querySelector(".insert-poll");
        advancedButton = el.querySelector(".advanced-mode-btn");
        insertButton.addEventListener("click", function () {
          if (disableInsert()) {
            return;
          }
          if (edit && edit.onSave) {
            edit.onSave(pollAttrs());
          } else {
            composer.addText(pollOutput());
          }
          closeModal();
        });
        el.querySelector(".btn-flat").addEventListener("click", closeModal);
        if (!advancedButton) {
          return;
        }
        advancedButton.addEventListener("click", function () {
          m.advanced = !m.advanced;
          if (m.advanced) {
            m.optionsText = m.options.join("\n");
          }
          focusIndex = -1;
          redraw();
        });
      },
    });

    // A click outside an open select-kit closes it (the select-kit
    // service's clickOutside).
    var modalEl = body.closest(".d-modal") || body;
    modalEl.addEventListener("click", function (event) {
      // A row or choice the click redrew is no longer in the page.
      if (!event.target.isConnected) {
        return;
      }
      modalEl.querySelectorAll("details.select-kit[open]").forEach(function (kit) {
        if (!kit.contains(event.target)) {
          kit.open = false;
        }
      });
    });

    body.addEventListener("click", function (event) {
      var pill = event.target.closest("[data-poll-type]");
      if (pill) {
        event.preventDefault();
        var type = pill.dataset.pollType;
        if (type === NUMBER && !isNumber()) {
          m.max = maxOptions;
        }
        m.type = type;
        enforceMinMax();
        focusIndex = -1;
        redraw();
        return;
      }
      var remove = event.target.closest("[data-remove]");
      if (remove) {
        m.options.splice(parseInt(remove.dataset.remove, 10), 1);
        focusIndex = -1;
        enforceMinMax();
        redraw();
        return;
      }
      if (event.target.closest(".poll-option-add")) {
        addOption(-1);
        return;
      }
      if (event.target.closest(".poll-toggle-public")) {
        m.publicPoll = !m.publicPoll;
        focusIndex = -1;
        redraw();
        return;
      }
      if (event.target.closest(".poll-toggle-dynamic")) {
        m.dynamic = !m.dynamic;
        focusIndex = -1;
        redraw();
        return;
      }
      var radio = event.target.closest('input[name="poll-chart-type"]');
      if (radio) {
        m.chartType = radio.value;
      }
    });

    body.addEventListener("input", function (event) {
      var el = event.target;
      if (el.matches(".poll-option-value input")) {
        var index = parseInt(el.dataset.index, 10);
        m.options[index] = el.value;
        focusIndex = index;
        enforceMinMax();
        refresh();
      } else if (el.matches(".poll-options textarea")) {
        m.optionsText = el.value;
        m.options = el.value.split("\n");
        enforceMinMax();
        focusIndex = -1;
        refresh();
      } else if (el.matches(".poll-title input")) {
        m.title = el.value;
      } else if (el.matches(".poll-options-min")) {
        m.min = el.value;
        enforceMinMax();
        focusIndex = -1;
        refresh();
      } else if (el.matches(".poll-options-max")) {
        m.max = el.value;
        enforceMinMax();
        focusIndex = -1;
        refresh();
      } else if (el.matches(".poll-options-step")) {
        m.step = el.value;
        focusIndex = -1;
        refresh();
      }
    });

    // onInputKeydown: Enter adds an option after a filled one; mod+Shift+.
    // inserts the current date (discourse-local-dates).
    body.addEventListener("keydown", function (event) {
      var el = event.target;
      if (!el.matches(".poll-option-value input")) {
        return;
      }
      var index = parseInt(el.dataset.index, 10);
      if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        if (el.value !== "") {
          addOption(index + 1);
        }
        return;
      }
      if (
        event.code === "Period" &&
        event.shiftKey &&
        (event.metaKey || event.ctrlKey) &&
        config.localDates &&
        Discourse.localDatesCreate &&
        Discourse.localDatesCreate.currentDateMarkup
      ) {
        event.preventDefault();
        var start = el.selectionStart;
        var end = el.selectionEnd;
        Discourse.localDatesCreate.currentDateMarkup().then(function (markup) {
          if (!markup) {
            return;
          }
          var value = m.options[index] || "";
          m.options[index] = value.slice(0, start) + markup + value.slice(end);
          focusIndex = index;
          enforceMinMax();
          redraw();
          var input = body.querySelector('.poll-option-value input[data-index="' + index + '"]');
          if (input) {
            input.setSelectionRange(start + markup.length, start + markup.length);
          }
        });
      }
    });

    redraw();
    return close;
  }

  // The ComboBox is the poll breakdown's group chooser too (poll.js).
  Discourse.pollBuilder = { show: show, comboBox: comboBox };
})();
