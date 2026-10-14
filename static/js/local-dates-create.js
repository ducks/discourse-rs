// discourse-local-dates' insert date modal (modal/local-dates-create.gjs)
// and the composer's Ctrl/Cmd+Shift+. that inserts the current date
// (generateCurrentDateMarkup). The modal: the from and to date-times with
// DCalendarDateTimeInput's Pikaday calendar (vendored, loaded on first
// use) and time input, the timezone's select-kit combo box, and in the
// advanced mode the recurrence, the timezones to display and the format;
// while the timezone differs from the member's, a preview of the date as
// the post will show it. Insert puts the [date] or [date-range] markup in
// the composer at the cursor. composer.js opens it from the toolbar's
// options menu (Discourse.localDatesCreate.show(insert)).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.localDatesCreate) {
    return;
  }

  var script = document.currentScript;
  var pikadayUrl = script ? script.dataset.pikaday : "";

  var TIME_FORMAT = "HH:mm:ss";
  var DATE_FORMAT = "YYYY-MM-DD";

  function ld() {
    return Discourse.localDates;
  }

  function t(key, params) {
    return ld() ? ld().i18n(key, params) : key;
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
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

  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var el = document.createElement("script");
      el.src = src;
      el.onload = resolve;
      el.onerror = function () {
        reject(new Error("could not load " + src));
      };
      document.head.appendChild(el);
    });
  }

  var pikadayLoading = null;
  function loadPikaday() {
    if (window.Pikaday) {
      return Promise.resolve(window.Pikaday);
    }
    if (!pikadayLoading) {
      pikadayLoading = loadScript(pikadayUrl).then(function () {
        return window.Pikaday;
      });
    }
    return pikadayLoading;
  }

  // lib/text's buildBBCodeAttrs: the present values, quoted when they
  // hold a space.
  function bbcodeAttrs(attrs) {
    return Object.keys(attrs)
      .filter(function (k) {
        var v = attrs[k];
        return v !== undefined && v !== null && v !== "";
      })
      .map(function (k) {
        var v = String(attrs[k]);
        return k + "=" + (/\s/.test(v) ? '"' + v + '"' : v);
      })
      .join(" ");
  }

  function isEmpty(v) {
    return v === undefined || v === null || v === "" || (Array.isArray(v) && !v.length);
  }

  // local-date-markup-generator.js
  function generateDateMarkup(from, options, isRange, to) {
    var attrs = {};
    if (!isRange) {
      attrs.time = from.time;
    }
    attrs.format = from.format;
    attrs.timezone = options.timezone;
    attrs.countdown = options.countdown;
    attrs.displayedTimezone = options.displayedTimezone;
    attrs.timezones = Array.isArray(options.timezones) ? options.timezones.join("|") : options.timezones;
    if (!isRange) {
      attrs.recurring = options.recurring;
    }
    var attrsStr = bbcodeAttrs(attrs);
    var suffix = attrsStr ? " " + attrsStr : "";
    if (isRange) {
      var f = [from.date, from.time].filter(function (e) { return !isEmpty(e); }).join("T");
      var tt = [to.date, to.time].filter(function (e) { return !isEmpty(e); }).join("T");
      return "[date-range from=" + f + " to=" + tt + suffix + "]";
    }
    return "[date=" + from.date + suffix + "]";
  }

  function userTimezone() {
    var tz = ld() && ld().config.userTimezone;
    return tz || window.moment.tz.guess();
  }

  // generateCurrentDateMarkup
  function currentDateMarkup() {
    var moment = window.moment;
    return generateDateMarkup(
      { date: moment().format(DATE_FORMAT), time: moment().format(TIME_FORMAT) },
      { timezone: userTimezone() },
      false
    );
  }

  function mobileView() {
    return document.documentElement.classList.contains("mobile-view");
  }

  // A select-kit combo box (or multi select): a <details> whose summary
  // shows the value, opened into a filter and the rows.
  function SelectKit(opts) {
    this.opts = opts;
    this.el = document.createElement("details");
    this.el.className = opts.classes;
    this.value = opts.value;
    this.render();
    var self = this;
    this.el.addEventListener("click", function (event) {
      var summary = event.target.closest("summary");
      if (summary && summary.parentElement === self.el) {
        event.preventDefault();
        self.toggle();
        return;
      }
      var row = event.target.closest(".select-kit-row");
      if (row) {
        self.select(row.dataset.value === "__none__" ? null : row.dataset.value);
        return;
      }
      var choice = event.target.closest(".selected-choice");
      if (choice) {
        self.remove(choice.dataset.value);
      }
    });
    this.el.addEventListener("input", function (event) {
      if (event.target.classList.contains("filter-input")) {
        self.renderRows(event.target.value);
      }
    });
    this.el.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && self.el.open) {
        event.stopPropagation();
        self.close();
      }
    });
    this.outside = function (event) {
      if (self.el.open && !self.el.contains(event.target)) {
        self.close();
      }
    };
    document.addEventListener("mousedown", this.outside);
  }

  SelectKit.prototype.nameOf = function (value) {
    var found = (this.opts.content || []).filter(function (c) {
      return c.id === value;
    })[0];
    return found ? found.name : value;
  };

  SelectKit.prototype.render = function () {
    var o = this.opts;
    var head;
    if (o.multi) {
      var values = this.value || [];
      var label = values.length
        ? ""
        : '<span class="formatted-selection">\n' + escapeHtml(t("select_kit.default_header_text")) + "\n</span>";
      var choices = values
        .map(function (v) {
          return (
            '<button class="btn btn-default selected-choice" data-value="' + escapeHtml(v) + '" title="' +
            escapeHtml(v) + '" type="button">' + icon("xmark") + '<span class="d-button-label">' +
            escapeHtml(v) + "</span></button>"
          );
        })
        .join("");
      head =
        '<summary data-name="' + escapeHtml(values.join(",")) + '" data-value="' + escapeHtml(values.join(",")) +
        '" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper">' +
        (values.length ? '<div class="choices">' + choices + "</div>" : label) +
        icon("plus") + "</div></summary>";
    } else {
      var name = this.value == null ? o.none : this.nameOf(this.value);
      var valueAttr = this.value == null ? "" : ' data-value="' + escapeHtml(this.value) + '"';
      head =
        '<summary aria-label="' + escapeHtml(t("select_kit.filter_by", { name: name })) + '" name="' +
        escapeHtml(t("select_kit.filter_by", { name: name })) + '" data-name="' + escapeHtml(name) + '"' + valueAttr +
        ' tabindex="0" class="select-kit-header single-select-header combo-box-header"><div class="select-kit-header-wrapper">\n' +
        (o.icon ? icon(o.icon) + "\n" : "") +
        '<div class="select-kit-selected-name selected-name choice" data-name="' + escapeHtml(name) + '"' + valueAttr +
        ' title="' + escapeHtml(name) + '">\n<span class="name">\n' + escapeHtml(name) + "\n</span>\n</div>\n" +
        icon(this.el.open ? "angle-up" : "angle-down", "angle-icon") + "\n</div></summary>";
    }
    this.el.classList.toggle("has-selection", !o.multi && this.value != null);
    this.el.innerHTML = head + '<div class="select-kit-body"></div>';
  };

  SelectKit.prototype.toggle = function () {
    if (this.el.open) {
      this.close();
    } else {
      this.open();
    }
  };

  SelectKit.prototype.open = function () {
    this.el.open = true;
    this.el.classList.add("is-expanded");
    this.render();
    var body = this.el.querySelector(".select-kit-body");
    var filter = this.opts.filterable
      ? '<div class="select-kit-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
        escapeHtml(t("select_kit.filter_placeholder")) + '" spellcheck="false" tabindex="0" type="search">' +
        icon("magnifying-glass") + "</div>"
      : "";
    body.innerHTML = filter + '<ul aria-live="polite" class="select-kit-collection" role="menu"></ul>';
    this.renderRows("");
    this.position(body);
    var input = body.querySelector(".filter-input");
    if (input) {
      input.focus();
    }
    // _scrollToRow: focusing the selected row brings it to the middle of
    // the list when it is out of view.
    var selected = body.querySelector(".select-kit-row.is-selected");
    if (selected) {
      var list = body.querySelector(".select-kit-collection");
      var top = selected.offsetTop - list.offsetTop;
      if (top + selected.offsetHeight > list.scrollTop + list.clientHeight) {
        list.scrollTop = top - (list.clientHeight - selected.offsetHeight) / 2;
      }
    }
  };

  // SelectKit's computePosition: below the header (above when it does
  // not fit), 3px off, at least the header's width (220px at least),
  // fixed on desktop.
  SelectKit.prototype.position = function (body) {
    var F = window.FloatingUIDOM;
    var header = this.el.querySelector("summary");
    if (!F || !header) {
      return;
    }
    var strategy = mobileView() ? "absolute" : "fixed";
    body.style.position = strategy;
    var headerOffset =
      parseInt(document.documentElement.style.getPropertyValue("--header-offset"), 10) || 0;
    var self = this;
    if (this.stopPosition) {
      this.stopPosition();
    }
    // Kept in place while open (the modal settling from its animation).
    this.stopPosition = F.autoUpdate(header, body, function () {
      self.place(F, header, body, strategy, headerOffset);
    }, { animationFrame: true });
  };

  SelectKit.prototype.place = function (F, header, body, strategy, headerOffset) {
    var minWidth;
    F.computePosition(header, body, {
      placement: "bottom-start",
      strategy: strategy,
      middleware: [
        F.size({
          apply: function (state) {
            minWidth = Math.max(Math.round(state.rects.reference.width), 220) + "px";
          },
        }),
        F.flip({ padding: { top: 3 + headerOffset, bottom: 3 }, fallbackStrategy: "initialPlacement" }),
        F.shift({ limiter: F.limitShift() }),
        F.offset(3),
        F.hide(),
      ],
    }).then(function (result) {
      var dpr = window.devicePixelRatio || 1;
      var round = function (v) {
        return Math.round(v * dpr) / dpr;
      };
      body.style.minWidth = minWidth;
      body.style.top = "0";
      body.style.left = "0";
      body.style.transform = "translate(" + round(result.x) + "px," + round(result.y) + "px)";
      var hidden = result.middlewareData.hide && result.middlewareData.hide.referenceHidden;
      body.style.visibility = hidden ? "hidden" : "visible";
      body.style.pointerEvents = hidden ? "none" : "auto";
    });
  };

  SelectKit.prototype.close = function () {
    if (this.stopPosition) {
      this.stopPosition();
      this.stopPosition = null;
    }
    this.el.open = false;
    this.el.classList.remove("is-expanded");
    this.render();
  };

  SelectKit.prototype.renderRows = function (filter) {
    var self = this;
    var list = this.el.querySelector(".select-kit-collection");
    if (!list) {
      return;
    }
    var term = (filter || "").toLowerCase();
    var rows = (this.opts.content || []).filter(function (c) {
      if (self.opts.multi && (self.value || []).indexOf(c.id) !== -1) {
        return false;
      }
      return !term || c.name.toLowerCase().indexOf(term) !== -1;
    });
    if (this.opts.max && (this.value || []).length >= this.opts.max) {
      list.innerHTML =
        '<li class="select-kit-row no-content">' +
        escapeHtml(t("select_kit.max_content_reached." + (this.opts.max === 1 ? "one" : "other"), { count: this.opts.max })) +
        "</li>";
      return;
    }
    var html = "";
    if (this.opts.none && !term) {
      html +=
        '<li aria-checked="' + (this.value == null) + '" role="menuitemradio" data-name="' + escapeHtml(this.opts.none) +
        '" data-value="__none__" title="' + escapeHtml(this.opts.none) + '" tabindex="0" class="select-kit-row none' +
        (this.value == null ? " is-selected" : "") + '"><span class="name">\n' + escapeHtml(this.opts.none) + "\n</span></li>";
    }
    rows.forEach(function (c, i) {
      var selected = !self.opts.multi && c.id === self.value;
      html +=
        '<li aria-checked="' + selected + '" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(c.name) +
        '" data-value="' + escapeHtml(c.id) + '" title="' + escapeHtml(c.name) + '" tabindex="0" class="select-kit-row' +
        (selected ? " is-highlighted is-selected" : "") + '"> <span class="name">\n' + escapeHtml(c.name) + "\n</span></li>";
    });
    if (!html) {
      html = '<li class="select-kit-row no-content">' + escapeHtml(t("select_kit.no_content")) + "</li>";
    }
    list.innerHTML = html;
  };

  SelectKit.prototype.select = function (value) {
    if (this.opts.multi) {
      var values = (this.value || []).slice();
      if (value != null && values.indexOf(value) === -1) {
        values.push(value);
      }
      this.value = values;
    } else {
      // TimezoneInput: IST is ambiguous.
      if (this.opts.timezone && value === "IST") {
        return;
      }
      this.value = value;
    }
    this.close();
    this.opts.onChange(this.value);
  };

  SelectKit.prototype.remove = function (value) {
    this.value = (this.value || []).filter(function (v) {
      return v !== value;
    });
    this.render();
    this.opts.onChange(this.value);
  };

  SelectKit.prototype.destroy = function () {
    if (this.stopPosition) {
      this.stopPosition();
    }
    document.removeEventListener("mousedown", this.outside);
  };

  function recurringOptions() {
    var key = "discourse_local_dates.create.form.recurring";
    return [
      ["every_day", "1.days"],
      ["every_week", "1.weeks"],
      ["every_two_weeks", "2.weeks"],
      ["every_month", "1.months"],
      ["every_two_months", "2.months"],
      ["every_three_months", "3.months"],
      ["every_six_months", "6.months"],
      ["every_year", "1.years"],
    ].map(function (o) {
      return { name: t(key + "." + o[0]), id: o[1] };
    });
  }

  // TimezoneInput's content: the localized names while moment's locale
  // is not English and the translations are loaded.
  function timezoneContent() {
    var moment = window.moment;
    if (moment.locale() !== "en" && typeof moment.tz.localizedNames === "function") {
      return moment.tz.localizedNames().map(function (z) {
        return { id: z.value, name: z.name };
      });
    }
    return moment.tz.names().map(function (z) {
      return { id: z, name: z };
    });
  }

  function show(insertDate) {
    var moment = window.moment;
    var config = (ld() && ld().config) || {};
    var state = {
      date: moment().format(DATE_FORMAT),
      time: null,
      toDate: null,
      toTime: null,
      timezone: userTimezone(),
      timezones: [],
      format: null,
      formats: (config.defaultFormats || "").split("|").filter(Boolean),
      recurring: null,
      advancedMode: false,
      fromSelected: true,
      toSelected: false,
      minDate: null,
    };
    var currentUserTimezone = userTimezone();
    var picker = null;
    var kits = [];
    var previewTimer = null;
    var currentPreview = "";

    function isRange() {
      return !!(state.date && (state.toDate || state.toTime));
    }

    function fromConfig() {
      var time = state.time;
      var timeInferred = !time;
      var dateTime = timeInferred
        ? moment.tz(state.date, state.timezone)
        : moment.tz(state.date + " " + time, state.timezone);
      if (!timeInferred) {
        time = dateTime.format(TIME_FORMAT);
      }
      var format = state.format;
      if (timeInferred && (format === "LLL" || format === "LLLL")) {
        format = "LL";
      }
      return { date: dateTime.format(DATE_FORMAT), time: time, dateTime: dateTime, format: format, range: isRange() ? "start" : false };
    }

    function toConfig() {
      var date = state.toDate;
      var time = state.toTime;
      var timeInferred = !time;
      if (time && !date) {
        date = moment().format(DATE_FORMAT);
      }
      var dateTime = timeInferred
        ? moment.tz(date, state.timezone).endOf("day")
        : moment.tz(date + " " + time, state.timezone);
      if (!timeInferred) {
        time = dateTime.format(TIME_FORMAT);
      }
      var format = state.format;
      if (timeInferred && (format === "LLL" || format === "LLLL")) {
        format = "LL";
      }
      return { date: dateTime.format(DATE_FORMAT), time: time, dateTime: dateTime, format: format, range: isRange() ? "end" : false };
    }

    function options() {
      return {
        recurring: state.recurring,
        timezones: state.timezones,
        timezone: state.timezone,
        format: state.format,
      };
    }

    function isValid() {
      var from = fromConfig();
      if (!from.dateTime || !from.dateTime.isValid()) {
        return false;
      }
      if (isRange()) {
        var to = toConfig();
        if (!to.dateTime || !to.dateTime.isValid() || to.dateTime.diff(from.dateTime) < 0) {
          return false;
        }
      }
      return true;
    }

    function markup() {
      if (!isValid()) {
        return null;
      }
      var to = toConfig();
      if (to.range) {
        return generateDateMarkup(fromConfig(), options(), isRange(), to);
      }
      return generateDateMarkup(fromConfig(), options(), isRange());
    }

    function formattedCurrentUserTimezone() {
      return currentUserTimezone.replace("_", " ").replace("Etc/", "").replace("/", ", ");
    }

    var bodyEl, footerEl, closeModal;

    function renderAlert() {
      var holder = bodyEl.querySelector(".form-alert");
      var html = "";
      if (isValid()) {
        if (currentUserTimezone !== state.timezone) {
          html =
            '<div class="preview alert alert-info">\n' +
            escapeHtml(t("discourse_local_dates.create.form.current_timezone")) +
            "\n<b>" + escapeHtml(formattedCurrentUserTimezone()) + "</b>" + currentPreview + "</div>";
        }
      } else {
        html =
          '<div class="validation-error alert alert-error">\n' +
          escapeHtml(t("discourse_local_dates.create.form.invalid_date")) + "\n</div>";
      }
      holder.innerHTML = html;
      if (html && currentPreview && Discourse.applyLocalDates) {
        Discourse.applyLocalDates(holder.querySelector(".preview"));
      }
    }

    function renderControls() {
      var from = bodyEl.querySelector(".date-time-control.from");
      from.className =
        "date-time-control from" + (state.fromSelected ? " is-selected" : "") + (!isEmpty(state.date) ? " is-filled" : "");
      from.querySelector(".d-button-label").textContent = fromConfig().dateTime.format("LLLL");
      var to = bodyEl.querySelector(".date-time-control.to");
      var toFilled = !isEmpty(state.toDate);
      to.className =
        "date-time-control to" + (state.toSelected ? " is-selected" : "") + (toFilled ? " is-filled" : "");
      var toDt = toConfig().dateTime;
      to.querySelector(".date-time .d-button-label").textContent =
        toDt && toDt.isValid() ? toDt.format("LLLL") : t("discourse_local_dates.create.form.until");
      var del = to.querySelector(".delete-to-date");
      if (toFilled && !del) {
        to.insertAdjacentHTML(
          "beforeend",
          '<button class="btn no-text btn-icon delete-to-date" type="button">' + icon("xmark") +
            '<span aria-hidden="true">​</span></button>'
        );
      } else if (!toFilled && del) {
        del.remove();
      }
    }

    function renderFooter() {
      if (!footerEl) {
        return;
      }
      var advanced = state.advancedMode
        ? "advanced_mode_toggle.simple_mode"
        : "advanced_mode_toggle.advanced_mode";
      var sm = window.matchMedia("(min-width: 40rem)").matches;
      var label = escapeHtml(t(advanced));
      footerEl.innerHTML =
        (isValid()
          ? '<button class="btn btn-primary" type="button"><span class="d-button-label">' +
            escapeHtml(t("discourse_local_dates.create.form.insert")) + "</span></button>"
          : "") +
        '<button class="btn btn-flat" type="button"><span class="d-button-label">' + escapeHtml(t("cancel")) +
        "</span></button>" +
        (sm
          ? '<button aria-label="' + label + '" class="btn btn-icon-text btn-default advanced-mode-btn" type="button">' +
            icon("gear") + '<span class="d-button-label">' + label + "</span></button>"
          : '<button aria-label="' + label + '" class="btn no-text btn-icon btn-default advanced-mode-btn" type="button">' +
            icon("gear") + '<span aria-hidden="true">​</span></button>');
    }

    function renderAdvanced() {
      var holder = bodyEl.querySelector(".advanced-holder");
      kits = kits.filter(function (k) {
        if (k.advanced) {
          k.destroy();
          return false;
        }
        return true;
      });
      if (!state.advancedMode) {
        holder.innerHTML = "";
        return;
      }
      var formats = state.formats
        .map(function (f) {
          return (
            '<li class="format">\n<a class="moment-format" href="" data-format="' + escapeHtml(f) + '">\n' +
            escapeHtml(f) + '\n</a>\n<span class="previewed-format">\n' + escapeHtml(moment().format(f)) +
            "\n</span>\n</li>"
          );
        })
        .join("\n");
      holder.innerHTML =
        '<div class="advanced-options">\n' +
        (isRange()
          ? ""
          : '<div class="control-group recurrence">\n<label class="control-label">\n' +
            escapeHtml(t("discourse_local_dates.create.form.recurring_title")) + "\n</label>\n<p>" +
            t("discourse_local_dates.create.form.recurring_description") +
            '</p>\n<div class="controls"></div>\n</div>\n') +
        '<div class="control-group timezones">\n<label>' +
        escapeHtml(t("discourse_local_dates.create.form.timezones_title")) + "</label>\n<p>" +
        escapeHtml(t("discourse_local_dates.create.form.timezones_description")) +
        '</p>\n<div class="controls"></div>\n</div>\n' +
        '<div class="control-group format">\n<label>' +
        escapeHtml(t("discourse_local_dates.create.form.format_title")) + "</label>\n<p>\n" +
        escapeHtml(t("discourse_local_dates.create.form.format_description")) +
        '\n<a href="https://momentjs.com/docs/#/parsing/string-format/" rel="noopener noreferrer" target="_blank">\n' +
        icon("circle-question") + '\n</a>\n</p>\n<div class="controls">\n<input placeholder="" class="format-input" type="text">\n</div>\n</div>\n' +
        '<div class="control-group">\n<ul class="formats">\n' + formats + "\n</ul>\n</div>\n</div>";
      var recurrence = holder.querySelector(".recurrence .controls");
      if (recurrence) {
        var rk = new SelectKit({
          classes: "select-kit single-select combobox combo-box recurrence-input",
          content: recurringOptions(),
          value: state.recurring,
          none: t("discourse_local_dates.create.form.recurring_none"),
          onChange: function (v) {
            state.recurring = v;
            update();
          },
        });
        rk.advanced = true;
        kits.push(rk);
        recurrence.appendChild(rk.el);
      }
      var tzk = new SelectKit({
        classes: "select-kit multi-select timezones-input",
        content: window.moment.tz.names().map(function (z) {
          return { id: z, name: z };
        }),
        value: state.timezones.slice(),
        multi: true,
        filterable: true,
        max: 5,
        onChange: function (v) {
          state.timezones = v;
          update();
        },
      });
      tzk.advanced = true;
      kits.push(tzk);
      holder.querySelector(".timezones .controls").appendChild(tzk.el);
      var formatInput = holder.querySelector(".format-input");
      formatInput.value = state.format || "";
      formatInput.addEventListener("input", function () {
        state.format = formatInput.value;
        update();
      });
    }

    function syncPicker() {
      if (!picker) {
        return;
      }
      var date = state.fromSelected ? state.date : state.toDate;
      if (date && moment(date, DATE_FORMAT).isValid()) {
        picker.setDate(moment.utc(date).format("YYYY/MM/DD"), true);
      } else {
        picker.setDate(null);
      }
      if (state.minDate && moment(state.minDate, DATE_FORMAT).isValid()) {
        picker.setMinDate(moment(state.minDate, DATE_FORMAT).toDate());
      } else {
        picker.setMinDate(null);
      }
      var timeInput = bodyEl.querySelector(".time-picker");
      var time = state.fromSelected ? state.time : state.toTime;
      timeInput.value = time && /^([01][0-9]|2[0-3]):[0-5][0-9]/.test(time) ? time.slice(0, 5) : "";
    }

    // _renderPreview, debounced: the markup cooked, its dates decorated.
    function schedulePreview() {
      clearTimeout(previewTimer);
      previewTimer = setTimeout(function () {
        var m = markup();
        if (!m || !Discourse.composerCook) {
          return;
        }
        Discourse.composerCook(m).then(function (html) {
          currentPreview = html;
          renderAlert();
        });
      }, 250);
    }

    function update() {
      renderAlert();
      renderControls();
      renderFooter();
      schedulePreview();
    }

    function focusFrom() {
      state.fromSelected = true;
      state.toSelected = false;
      state.minDate = null;
      syncPicker();
      update();
    }

    function focusTo() {
      state.toSelected = true;
      state.fromSelected = false;
      state.minDate = fromConfig().date;
      syncPicker();
      update();
    }

    closeModal = Discourse.modal.show({
      className: "discourse-local-dates-create-modal --large",
      title: t("discourse_local_dates.title"),
      body: function (body) {
        bodyEl = body;
        body.innerHTML =
          '<div class="form">\n<div class="form-alert"></div>\n' +
          '<div class="date-time-configuration">\n<div class="inputs-panel">\n' +
          '<div class="date-time-control from">\n' + icon("calendar-days") +
          '\n<button class="btn date-time" id="from-date-time" autofocus="" type="button"><span class="d-button-label"></span></button>\n</div>\n' +
          '<div class="date-time-control to">\n' + icon("calendar-days") +
          '\n<button class="btn date-time" type="button"><span class="d-button-label"></span></button>\n</div>\n' +
          "</div>\n" +
          '<div class="picker-panel">\n<div class="calendar-date-time-input">\n<input class="fake-input" type="text">\n' +
          '<div class="date-picker" id="picker-container-local-date-create-form"></div>\n' +
          '<div class="time-pickers">\n' + icon("far-clock") +
          '\n<input class="time-picker" maxlength="5" placeholder="hh:mm" type="time">\n</div>\n</div>\n</div>\n' +
          "</div>\n" +
          '<div class="advanced-holder"></div>\n</div>';
        // The timezone: in the inputs on desktop, after the picker on a
        // phone.
        var tz = new SelectKit({
          classes: "select-kit single-select combobox combo-box timezone-input",
          content: timezoneContent(),
          value: state.timezone,
          icon: "globe",
          filterable: true,
          timezone: true,
          onChange: function (v) {
            state.timezone = v;
            update();
          },
        });
        kits.push(tz);
        if (mobileView()) {
          body.querySelector(".date-time-configuration").appendChild(tz.el);
        } else {
          body.querySelector(".inputs-panel").appendChild(tz.el);
        }
        body.addEventListener("click", function (event) {
          if (event.target.closest(".date-time-control.from .date-time")) {
            focusFrom();
          } else if (event.target.closest(".date-time-control.to .date-time")) {
            focusTo();
          } else if (event.target.closest(".delete-to-date")) {
            state.toDate = null;
            state.toTime = null;
            focusFrom();
          } else if (event.target.closest(".moment-format")) {
            event.preventDefault();
            state.format = event.target.closest(".moment-format").dataset.format;
            var input = body.querySelector(".format-input");
            if (input) {
              input.value = state.format;
            }
            update();
          }
        });
        body.querySelector(".time-picker").addEventListener("input", function (event) {
          if (state.fromSelected) {
            state.time = event.target.value;
          } else {
            state.toTime = event.target.value;
          }
          update();
        });
        loadPikaday().then(function (Pikaday) {
          picker = new Pikaday({
            field: body.querySelector(".fake-input"),
            container: body.querySelector("#picker-container-local-date-create-form"),
            bound: false,
            format: "YYYY-MM-DD",
            reposition: false,
            firstDay: 1,
            setDefaultDate: true,
            keyboardInput: false,
            i18n: {
              previousMonth: t("dates.previous_month"),
              nextMonth: t("dates.next_month"),
              months: moment.months(),
              weekdays: moment.weekdays(),
              weekdaysShort: moment.weekdaysMin(),
            },
            onSelect: function (date) {
              var formatted = moment(date).format("YYYY-MM-DD");
              if (state.fromSelected) {
                state.date = formatted;
              } else {
                state.toDate = formatted;
              }
              update();
            },
          });
          syncPicker();
        });
        update();
      },
      footer: function (footer, close) {
        footerEl = footer;
        footer.addEventListener("click", function (event) {
          if (event.target.closest(".btn-primary")) {
            var m = markup();
            if (m) {
              close();
              insertDate(m);
            }
          } else if (event.target.closest(".btn-flat")) {
            close();
          } else if (event.target.closest(".advanced-mode-btn")) {
            state.advancedMode = !state.advancedMode;
            renderAdvanced();
            renderFooter();
          }
        });
        renderFooter();
      },
      onClose: function () {
        clearTimeout(previewTimer);
        kits.forEach(function (k) {
          k.destroy();
        });
        if (picker) {
          picker.destroy();
        }
      },
    });
    focusFrom();
    var from = bodyEl && bodyEl.querySelector("#from-date-time");
    if (from) {
      from.focus();
    }
    return closeModal;
  }

  Discourse.localDatesCreate = {
    // The modal, once moment and its timezones (and locale) are in.
    show: function (insertDate) {
      if (!ld()) {
        return Promise.resolve();
      }
      return ld()
        .loadMoment()
        .then(function () {
          return show(insertDate);
        });
    },
    // The shortcut's markup for now in the member's timezone.
    currentDateMarkup: function () {
      if (!ld()) {
        return Promise.resolve(null);
      }
      return ld()
        .loadMoment()
        .then(function () {
          return currentDateMarkup();
        });
    },
  };
})();
