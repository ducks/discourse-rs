// The admin's PenalizeUser modal (admin/components/modal/penalize-user.gjs)
// as adminTools.showControlModal shows it, for silencing and suspending:
// DFutureDateInput (the "until" combo box of time shortcuts, or a custom
// date and time), AdminPenaltyReason (the reason presets or a custom one,
// the message emailed to the user), AdminPenaltyPostAction when it comes
// from a post (delete, delete with replies, edit, nothing; deleting every
// post for users below trust level 2), AdminPenaltySimilarUsers (the
// other users at the same IP, from similar-users.json) and the penalty
// history. Submitting PUTs /admin/users/:id/silence or /suspend.
//
//   Discourse.penalizeUser.show({ penaltyType, user, postId, postEdit,
//                                 reviewableId, before, successCallback })
//       `user` is the admin user's JSON (AdminUser); `before` runs once
//       before the penalty (the flag modal's flag); `successCallback`
//       gets the response.
//   Discourse.penalizeUser.find(userId) -> Promise of the admin user JSON
//       (AdminUser.find), for a caller that has only the user's id.
//
// The page's #penalize-user-config holds the strings, the settings and the
// timezone the shortcuts are computed in; moment, moment-timezone and
// Pikaday load when the modal first opens. Not ported, and saying so: the
// delete posts decider after "delete all posts".
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.penalizeUser) {
    return;
  }

  // FutureDateInputSelector's FORMAT.
  var FORMAT = "YYYY-MM-DD HH:mmZ";
  var REASON_KEYS = [
    "not_listening_to_staff",
    "consuming_staff_time",
    "combative",
    "in_wrong_place",
    "no_constructive_purpose",
    "custom",
  ];
  var POST_ACTIONS = ["delete", "delete_all", "delete_replies", "edit", "none"];

  // Read again each time: a page swap brings another page's block.
  function config() {
    var el = document.getElementById("penalize-user-config");
    try {
      return el ? JSON.parse(el.textContent) : null;
    } catch (e) {
      return null;
    }
  }

  function t(key, args) {
    var c = config();
    var s = c && c.strings[key];
    if (s === undefined) {
      return "[en." + key + "]";
    }
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  function tCount(key, count, args) {
    var all = Object.assign({ count: count }, args || {});
    return t(key + (count === 1 ? ".one" : ".other"), all);
  }

  // I18n.messageFormat for the plural forms these strings use.
  function messageFormat(key, values) {
    var src = t(key);
    var out = "";
    var i = 0;
    while (i < src.length) {
      var open = src.indexOf("{", i);
      if (open === -1) {
        out += src.slice(i);
        break;
      }
      out += src.slice(i, open);
      var depth = 0;
      var end = open;
      for (; end < src.length; end++) {
        if (src[end] === "{") {
          depth++;
        } else if (src[end] === "}" && --depth === 0) {
          break;
        }
      }
      var inner = src.slice(open + 1, end);
      var parts = inner.split(",");
      var name = parts[0].trim();
      if (parts.length > 2 && parts[1].trim() === "plural") {
        var n = Number(values[name]) || 0;
        var forms = {};
        var rest = parts.slice(2).join(",");
        var re = /\s*(=\d+|zero|one|two|few|many|other)\s*\{/g;
        var m;
        while ((m = re.exec(rest))) {
          var d = 1;
          var j = re.lastIndex;
          for (; j < rest.length && d > 0; j++) {
            if (rest[j] === "{") {
              d++;
            } else if (rest[j] === "}") {
              d--;
            }
          }
          forms[m[1]] = rest.slice(re.lastIndex, j - 1);
          re.lastIndex = j;
        }
        var form = forms["=" + n] !== undefined ? forms["=" + n] : n === 1 && forms.one !== undefined ? forms.one : forms.other;
        out += (form || "").replace(/#/g, String(n));
      } else {
        out += values[name] === undefined ? "" : String(values[name]);
      }
      i = end + 1;
    }
    return out.trim();
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name, extra) {
    if (Discourse.cooked && Discourse.cooked.icon) {
      return Discourse.cooked.icon(name, extra);
    }
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon svg-string" +
      (extra ? " " + extra : "") +
      '" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    if (meta && meta.content) {
      return meta.content;
    }
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}")["X-CSRF-Token"] || "";
    } catch (e) {
      return "";
    }
  }

  // jQuery's param: undefined and null left out, arrays as key[].
  function encode(data) {
    var params = new URLSearchParams();
    Object.keys(data).forEach(function (key) {
      var value = data[key];
      if (value === undefined || value === null) {
        return;
      }
      if (Array.isArray(value)) {
        value.forEach(function (v) {
          params.append(key + "[]", v);
        });
      } else {
        params.append(key, value);
      }
    });
    return params.toString();
  }

  // discourse/lib/ajax, rejecting with extractError's message.
  function ajax(method, path, data) {
    var options = {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
    };
    if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = encode(data);
    }
    return fetch(base() + path, options).then(
      function (response) {
        return response.text().then(function (text) {
          var json = null;
          try {
            json = text ? JSON.parse(text) : null;
          } catch (e) {
            // Not JSON: the status decides.
          }
          if (!response.ok) {
            var error = new Error(extractError(json, response));
            error.status = response.status;
            throw error;
          }
          return json;
        });
      },
      function () {
        throw new Error(t("generic_error"));
      }
    );
  }

  // ajax-error's extractError
  function extractError(json, response) {
    if (json) {
      if (json.errors && json.errors.length > 1) {
        return t("multiple_errors", {
          errors: json.errors
            .map(function (e, i) {
              return i + 1 + ") " + e;
            })
            .join(" "),
        });
      }
      if (json.errors && json.errors.length) {
        return t("generic_error_with_reason", { error: json.errors[0] });
      }
      if (json.error) {
        return json.error;
      }
      if (json.message) {
        return json.message;
      }
      if (json.failed) {
        return json.failed;
      }
    }
    if (response && response.status >= 400) {
      return response.status + " " + (response.statusText || "");
    }
    return t("generic_error");
  }

  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = src;
      s.onload = resolve;
      s.onerror = reject;
      document.head.appendChild(s);
    });
  }

  var momentLoading = null;
  function loadMoment() {
    if (window.moment && window.moment.tz) {
      return Promise.resolve();
    }
    if (!momentLoading) {
      var assets = config().assets;
      momentLoading = (window.moment ? Promise.resolve() : loadScript(assets.moment)).then(function () {
        return window.moment.tz ? null : loadScript(assets.moment_timezone);
      });
    }
    return momentLoading;
  }

  function timezone() {
    return config().timezone || window.moment.tz.guess();
  }

  // lib/time-shortcut's extendedDefaultTimeShortcuts, custom, then
  // hideDynamicTimeShortcuts, as DFutureDateInput lists them.
  function shortcuts() {
    var m = window.moment;
    var tz = timezone();
    var now = function () {
      return m.tz(tz);
    };
    var startOfDay = function (d) {
      return d.hour(8).startOf("hour");
    };
    var laterToday = function () {
      var later = now().add(3, "hours");
      if (later.hour() >= 18) {
        return later.hour(18).startOf("hour");
      }
      return later.minutes() < 30 ? later.startOf("hour") : later.add(30, "minutes").startOf("hour");
    };
    var day = now().day();
    var list = [
      { id: "later_today", icon: "far-clock", label: "later_today", time: laterToday(), format: "dates.time" },
      { id: "tomorrow", icon: "calendar-day", label: "tomorrow", time: startOfDay(now().add(1, "day")), format: "dates.time_short_day" },
      { id: "later_this_week", icon: "calendar-days", label: "later_this_week", time: startOfDay(now().add(2, "days")), format: "dates.time_short_day" },
      {
        id: "start_of_next_business_week",
        icon: "briefcase",
        label: day === 1 || day === 0 ? "start_of_next_business_week_alt" : "start_of_next_business_week",
        time: startOfDay(now().add(7, "days")).day(1),
        format: "dates.long_no_year",
      },
      { id: "two_weeks", icon: "far-calendar-plus", label: "two_weeks", time: startOfDay(now().add(2, "weeks").day(1)), format: "dates.long_no_year" },
      { id: "next_month", icon: "far-calendar-plus", label: "next_month", time: startOfDay(now().add(1, "month").startOf("month")), format: "dates.long_no_year" },
      { id: "two_months", icon: "far-calendar-plus", label: "two_months", time: startOfDay(now().add(2, "months").startOf("month")), format: "dates.long_no_year" },
      { id: "three_months", icon: "far-calendar-plus", label: "three_months", time: startOfDay(now().add(3, "months").startOf("month")), format: "dates.long_no_year" },
      { id: "four_months", icon: "far-calendar-plus", label: "four_months", time: startOfDay(now().add(4, "months").startOf("month")), format: "dates.long_no_year" },
      { id: "six_months", icon: "far-calendar-plus", label: "six_months", time: startOfDay(now().add(6, "months").startOf("month")), format: "dates.long_with_year" },
      { id: "one_year", icon: "far-calendar-plus", label: "one_year", time: startOfDay(now().add(1, "years").startOf("month")), format: "dates.long_with_year" },
      { id: "forever", icon: "gavel", label: "forever", time: startOfDay(now().add(1000, "years").startOf("month")), format: "dates.long_with_year" },
      { id: "custom", icon: "calendar-days", label: "custom", time: null, format: null },
    ];
    var hide = {};
    if (now().hour() >= 17) {
      hide.later_today = true;
    }
    // `_now.day === MOMENT_SUNDAY` compares the function: only Thursday on.
    if (day >= 4) {
      hide.later_this_week = true;
    }
    return list
      .filter(function (x) {
        return !hide[x.id];
      })
      .map(function (x) {
        return {
          id: x.id,
          name: t("time_shortcut." + x.label),
          icon: x.icon,
          time: x.time,
          timeFormatted: x.time && x.format ? x.time.format(t(x.format)) : null,
        };
      });
  }

  // A select-kit ComboBox (single-select combobox combo-box) of rows
  // {id, name, icon, timeFormatted}: the header shows the selection, the
  // body opens on its header with the rows (and a filter when asked).
  function comboBox(opts) {
    var details = document.createElement("details");
    details.className =
      "select-kit single-select combobox combo-box" + (opts.headerClass ? " " + opts.headerClass : "") + (opts.className ? " " + opts.className : "");
    details.innerHTML =
      '<summary tabindex="0" class="select-kit-header single-select-header combo-box-header' +
      (opts.headerClass ? " " + opts.headerClass + "-header" : "") +
      '"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div>';
    var summary = details.querySelector("summary");
    var body = details.querySelector(".select-kit-body");
    var value = opts.value;

    function drawHeader() {
      var item = opts.rows().find(function (r) {
        return r.id === value;
      });
      var name = item ? item.name : opts.none || "";
      var label = t("select_kit.filter_by", { name: name });
      summary.setAttribute("aria-label", label);
      summary.setAttribute("name", label);
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
        (item && item.timeFormatted
          ? '<span class="future-date-input-selector-datetime">' + escapeHtml(item.timeFormatted) + "</span>"
          : "") +
        icon(details.open ? "angle-up" : "angle-down", "angle-icon");
    }

    function rowHtml(r, index) {
      var selected = r.id === value;
      var inner = opts.rowClass
        ? (r.icon ? '<div class="future-date-input-selector-icons">' + icon(r.icon) + "</div>" : "") +
          '<span class="name">' +
          escapeHtml(r.name) +
          "</span>" +
          (r.timeFormatted ? '<span class="future-date-input-selector-datetime">' + escapeHtml(r.timeFormatted) + "</span>" : "")
        : '<span class="name">' + escapeHtml(r.name) + "</span>";
      return (
        '<li aria-checked="' +
        selected +
        '" role="menuitemradio" data-index="' +
        index +
        '" data-name="' +
        escapeHtml(r.name) +
        '" data-value="' +
        escapeHtml(r.id) +
        '" title="' +
        escapeHtml(r.name) +
        '" tabindex="0" class="select-kit-row' +
        (opts.rowClass ? " " + opts.rowClass : "") +
        (selected ? " is-highlighted is-selected" : "") +
        '">' +
        inner +
        "</li>"
      );
    }

    function drawRows(filter) {
      var term = (filter || "").toLowerCase();
      var list = body.querySelector(".select-kit-collection");
      list.innerHTML = opts
        .rows()
        .filter(function (r) {
          return !term || r.name.toLowerCase().indexOf(term) !== -1;
        })
        .map(rowHtml)
        .join("");
    }

    function float() {
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
          body.style.pointerEvents = "auto";
        });
    }

    details.addEventListener("toggle", function () {
      details.classList.toggle("is-expanded", details.open);
      if (!details.open) {
        body.innerHTML = "";
        drawHeader();
        return;
      }
      body.innerHTML =
        (opts.filter
          ? '<div class="select-kit-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
            escapeHtml(t("select_kit.filter_placeholder")) +
            '" spellcheck="false" tabindex="0" type="search">' +
            icon("magnifying-glass", "filter-icon") +
            "</div>"
          : '<div class="select-kit-filter"></div>') +
        '<ul aria-live="polite" class="select-kit-collection" role="menu"></ul>';
      drawRows("");
      drawHeader();
      float();
      var input = body.querySelector(".filter-input");
      if (input) {
        input.addEventListener("input", function () {
          drawRows(input.value);
        });
        input.focus();
      }
    });
    body.addEventListener("click", function (event) {
      var row = event.target.closest(".select-kit-row");
      if (!row) {
        return;
      }
      value = row.dataset.value;
      details.open = false;
      drawHeader();
      opts.onChange(value);
    });
    body.addEventListener("keydown", function (event) {
      var row = event.target.closest(".select-kit-row");
      if (row && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault();
        row.click();
      }
    });
    document.addEventListener("click", function (event) {
      if (details.open && !details.contains(event.target)) {
        details.open = false;
      }
    });
    drawHeader();
    return {
      el: details,
      set: function (v) {
        value = v;
        drawHeader();
      },
    };
  }

  // dFormatDuration (formatter.js's tiny duration), an em dash for none.
  function durationTiny(distance) {
    if (typeof distance !== "number") {
      return "&mdash;";
    }
    var minutes = Math.round(distance / 60.0);
    minutes = minutes < 1 ? 1 : minutes;
    var k = function (key, count) {
      return escapeHtml(tCount("dates.tiny." + key, count));
    };
    if (distance <= 59) {
      return k("less_than_x_minutes", 1);
    }
    if (minutes <= 44) {
      return k("x_minutes", minutes);
    }
    if (minutes <= 89) {
      return k("about_x_hours", 1);
    }
    if (minutes <= 1409) {
      return k("about_x_hours", Math.round(minutes / 60.0));
    }
    if (minutes <= 2519) {
      return k("x_days", 1);
    }
    if (minutes <= 129599) {
      return k("x_days", Math.round(minutes / 1440.0));
    }
    if (minutes <= 525599) {
      return k("x_months", Math.round(minutes / 43200.0));
    }
    var years = minutes / 525600.0;
    var rest = years % 1;
    if (rest < 0.25) {
      return k("about_x_years", Math.floor(years));
    }
    if (rest < 0.75) {
      return k("over_x_years", Math.floor(years));
    }
    return k("almost_x_years", Math.floor(years) + 1);
  }

  function number(n) {
    return '<span class="number">' + escapeHtml(n == null ? 0 : n) + "</span>";
  }

  function avatar(user) {
    var src = String(user.avatar_template || "").replace("{size}", "24");
    if (src.charAt(0) === "/" && src.charAt(1) !== "/") {
      src = base() + src.replace(new RegExp("^" + base()), "");
    }
    return (
      '<img alt="" width="24" height="24" src="' +
      escapeHtml(src) +
      '" class="avatar" title="' +
      escapeHtml(user.username) +
      '">'
    );
  }

  function button(className, iconName, label, disabled) {
    return (
      '<button class="btn ' +
      (iconName ? "btn-icon-text " : "") +
      className +
      '"' +
      (disabled ? ' disabled=""' : "") +
      ' type="button">' +
      (iconName ? icon(iconName) + " " : "") +
      '<span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  function loadPikaday() {
    if (window.Pikaday) {
      return Promise.resolve();
    }
    return loadScript(config().assets.pikaday);
  }

  // DFutureDateInput: the shortcut combo box, then with "custom" the
  // DatePickerFuture and the time input; onChangeInput(FORMAT string or
  // null).
  function futureDateInput(root, opts) {
    var m = window.moment;
    var list = shortcuts();
    var selection = null;
    var date = null;
    var time = null;
    if (opts.input) {
      var dateTime = m(opts.input);
      var closest = list.find(function (s) {
        if (s.time) {
          var diff = s.time.diff(dateTime);
          return diff >= 0 && diff < 60 * 1000;
        }
        return false;
      });
      if (closest) {
        selection = closest.id;
      } else {
        selection = "custom";
        date = dateTime.format("YYYY-MM-DD");
        time = dateTime.format("HH:mm");
      }
    }

    var wrapper = document.createElement("div");
    wrapper.className = "future-date-input " + opts.className;
    wrapper.innerHTML = '<div class="control-group"><label>' + escapeHtml(opts.label) + "</label></div>";
    root.appendChild(wrapper);

    function dateTimeChanged() {
      var value = date ? m(date + (time ? " " + time : "")) : null;
      opts.onChangeInput(value && value.isValid() ? value.locale("en").format(FORMAT) : null);
    }

    var picker = null;
    function drawCustom() {
      wrapper.querySelectorAll(".future-date-input-date-picker, .future-date-input-time-picker").forEach(function (el) {
        el.remove();
      });
      if (picker) {
        picker.destroy();
        picker = null;
      }
      if (selection !== "custom") {
        return;
      }
      var mobile = config().mobile;
      wrapper.insertAdjacentHTML(
        "beforeend",
        '<div class="control-group future-date-input-date-picker">' +
          icon("calendar-days") +
          '<div class="date-picker-wrapper"><input class="date-picker" autocomplete="off" placeholder="' +
          escapeHtml(t("dates.placeholder")) +
          '" type="' +
          (mobile ? "date" : "text") +
          '"></div></div><div class="control-group future-date-input-time-picker">' +
          icon("far-clock") +
          '<input class="time-input" placeholder="--:--" type="time"' +
          (date ? "" : " disabled") +
          "></div>"
      );
      var dateInput = wrapper.querySelector(".date-picker");
      var timeInput = wrapper.querySelector(".time-input");
      dateInput.value = date || "";
      timeInput.value = time || "";
      var onDate = function (value) {
        date = value ? m(value).format("YYYY-MM-DD") : null;
        if (!date) {
          time = null;
          timeInput.value = "";
        }
        timeInput.disabled = !date;
        dateTimeChanged();
      };
      if (mobile) {
        dateInput.addEventListener("change", function () {
          onDate(dateInput.value);
        });
      } else {
        loadPikaday().then(function () {
          if (!dateInput.isConnected) {
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
              dateInput.value = m(d).format("YYYY-MM-DD");
              onDate(d);
            },
          });
          if (date) {
            picker.setDate(m(date).toDate(), true);
          }
        });
      }
      timeInput.addEventListener("input", function () {
        if (date) {
          time = timeInput.value;
          dateTimeChanged();
        }
      });
    }

    var combo = comboBox({
      headerClass: "future-date-input-selector",
      rowClass: "future-date-input-selector-row",
      filter: true,
      none: t("time_shortcut.select_timeframe"),
      value: selection,
      rows: function () {
        return list;
      },
      onChange: function (value) {
        selection = value;
        if (value !== "custom") {
          var item = list.find(function (s) {
            return s.id === value;
          });
          if (item && item.time) {
            opts.onChangeInput(item.time.locale("en").format(FORMAT));
          }
        }
        drawCustom();
      },
    });
    wrapper.querySelector(".control-group").appendChild(combo.el);
    drawCustom();
  }

  function show(opts) {
    var c = config();
    if (!c || !Discourse.modal) {
      return Promise.resolve();
    }
    return loadMoment().then(function () {
      open(opts);
    });
  }

  function open(model) {
    var c = config();
    var user = model.user;
    var type = model.penaltyType;
    var suspend = type === "suspend";
    var state = {
      penalizeUntil: user.next_penalty || null,
      penalizing: false,
      confirmClose: false,
      otherUserIds: [],
      postAction: "delete",
      postEdit: model.postEdit,
      reason: undefined,
      message: undefined,
      readyToDeleteAll: false,
      beforeCompleted: false,
    };
    if (state.postEdit && c.settings.penalty_include_post_message) {
      state.message = "-------------------\n" + state.postEdit + "\n-------------------";
    }
    // AdminUser#canSuspend / canSilence
    var canPenalize = !(user.admin || user.moderator);
    var submit = null;
    var flash = null;
    var modalEl = null;

    function submitDisabled() {
      return (
        state.penalizing ||
        !state.penalizeUntil ||
        !state.reason ||
        state.reason.length < 1 ||
        (state.postAction === "delete_all" && !state.readyToDeleteAll)
      );
    }

    function update() {
      if (submit) {
        submit.disabled = submitDisabled();
      }
    }

    function setFlash(text) {
      if (flash) {
        flash.remove();
        flash = null;
      }
      if (text) {
        flash = document.createElement("div");
        flash.id = "modal-alert";
        flash.setAttribute("role", "alert");
        flash.className = "alert alert-error";
        flash.textContent = text;
        modalEl.querySelector(".d-modal__body").before(flash);
      }
    }

    var close = null;
    function warnBeforeClosing() {
      if (!state.confirmClose && ((state.reason && state.reason.length) || (state.message && state.message.length))) {
        Discourse.dialog.confirm({
          message: t("admin.user.confirm_cancel_penalty"),
          didConfirm: function () {
            state.confirmClose = true;
            close();
          },
        });
        return false;
      }
      return true;
    }

    function penalizeUser() {
      if (submitDisabled()) {
        return;
      }
      state.penalizing = true;
      state.confirmClose = true;
      update();
      var before =
        model.before && !state.beforeCompleted
          ? Promise.resolve(model.before()).then(function () {
              state.beforeCompleted = true;
            })
          : Promise.resolve();
      before
        .then(function () {
          var data = {
            reason: state.reason,
            message: state.message,
            post_id: model.postId,
            post_action: state.postAction,
            post_edit: state.postEdit,
            other_user_ids: state.otherUserIds,
            reviewable_id: model.reviewableId,
          };
          if (suspend) {
            data.suspend_until = state.penalizeUntil;
          } else {
            data.silenced_till = state.penalizeUntil;
          }
          return ajax("PUT", "/admin/users/" + user.id + "/" + (suspend ? "suspend" : "silence"), data);
        })
        .then(function (result) {
          close();
          var shouldDeleteAllPosts = state.postAction === "delete_all" && state.readyToDeleteAll;
          if (model.successCallback) {
            model.successCallback(Object.assign({}, result, { shouldDeleteAllPosts: shouldDeleteAllPosts }));
          }
          // adminTools.deletePostsDecider: not ported.
          if (shouldDeleteAllPosts) {
            Discourse.dialog.alert("Not ported yet: deleting all of the user's posts.");
          }
        })
        .catch(function (error) {
          setFlash(error && error.message ? error.message : t("generic_error"));
        })
        .then(function () {
          state.penalizing = false;
          update();
        });
    }

    close = Discourse.modal.show({
      className: type + "-user-modal --large",
      title: t(suspend ? "admin.user.suspend_modal_title" : "admin.user.silence_modal_title"),
      beforeClose: warnBeforeClosing,
      body: function (body) {
        modalEl = body.closest(".d-modal");
        if (canPenalize) {
          var duration = document.createElement("div");
          duration.className = "penalty-duration-controls";
          body.appendChild(duration);
          futureDateInput(duration, {
            className: suspend ? "suspend-until" : "silence-until",
            label: t(suspend ? "admin.user.suspend_duration" : "admin.user.silence_duration"),
            input: state.penalizeUntil,
            onChangeInput: function (value) {
              state.penalizeUntil = value;
              update();
            },
          });
          if (suspend) {
            body.insertAdjacentHTML(
              "beforeend",
              '<div class="penalty-reason-visibility">' +
                t(c.settings.hide_suspension_reasons ? "admin.user.suspend_reason_hidden_label" : "admin.user.suspend_reason_label") +
                "</div>"
            );
          }
          drawReason(body);
          if (model.postId) {
            drawPostAction(body);
          }
          if (user.similar_users_count) {
            drawSimilarUsers(body);
          }
        } else {
          body.insertAdjacentHTML(
            "beforeend",
            '<div class="' +
              (suspend ? "cant-suspend" : "cant-silence") +
              '">' +
              escapeHtml(t(suspend ? "admin.user.cant_suspend" : "admin.user.cant_silence")) +
              "</div>"
          );
        }
        var counts = user.penalty_counts || {};
        body.insertAdjacentHTML(
          "beforeend",
          '<div class="penalty-history">' +
            messageFormat("admin.user.penalty_history_MF", {
              SUSPENDED: counts.suspended,
              SILENCED: counts.silenced,
            }) +
            "</div>"
        );
      },
      footer: function (footer) {
        footer.innerHTML =
          button("btn-danger perform-penalize", "ban", t(suspend ? "admin.user.suspend" : "admin.user.silence"), true) +
          button("btn-flat d-modal-cancel", null, t("cancel"));
        submit = footer.querySelector(".perform-penalize");
        submit.addEventListener("click", penalizeUser);
        footer.querySelector(".d-modal-cancel").addEventListener("click", function () {
          close();
        });
        update();
      },
    });

    // AdminPenaltyReason
    function drawReason(body) {
      var controls = document.createElement("div");
      controls.className = "penalty-reason-controls";
      body.appendChild(controls);
      var selected = "custom";
      var custom = "";
      var reasonClass = suspend ? "suspend-reason" : "silence-reason";
      controls.insertAdjacentHTML(
        "beforeend",
        suspend
          ? '<label class="suspend-reason-title">' + escapeHtml(t("admin.user.suspend_reason_title")) + "</label>"
          : '<label class="silence-reason-title">' + t("admin.user.silence_reason_label") + "</label>"
      );
      var setReason = function () {
        state.reason = selected === "custom" ? custom : t("admin.user.suspend_reasons." + selected);
        update();
      };
      var drawCustom = function () {
        var input = controls.querySelector("input." + reasonClass);
        if (selected !== "custom") {
          if (input) {
            input.remove();
          }
          return;
        }
        if (input) {
          return;
        }
        input = document.createElement("input");
        input.placeholder = suspend ? "" : t("admin.user.silence_reason_placeholder");
        input.value = custom;
        input.className = reasonClass;
        input.type = "text";
        input.addEventListener("input", function () {
          custom = input.value;
          setReason();
        });
        controls.appendChild(input);
      };
      var combo = comboBox({
        className: reasonClass,
        value: selected,
        rows: function () {
          return REASON_KEYS.map(function (key) {
            return { id: key, name: t("admin.user.suspend_reasons." + key) };
          });
        },
        onChange: function (value) {
          selected = value;
          setReason();
          drawCustom();
        },
      });
      controls.appendChild(combo.el);
      drawCustom();
      body.insertAdjacentHTML(
        "beforeend",
        '<div class="penalty-message-controls"><label>' +
          escapeHtml(t("admin.user.suspend_message")) +
          '</label><textarea class="suspend-message" placeholder="' +
          escapeHtml(t("admin.user.suspend_message_placeholder")) +
          '"></textarea></div>'
      );
      var message = body.querySelector("textarea.suspend-message");
      message.value = state.message || "";
      message.addEventListener("input", function () {
        state.message = message.value;
      });
    }

    // AdminPenaltyPostAction
    function drawPostAction(body) {
      var holder = document.createElement("div");
      holder.innerHTML =
        '<div class="penalty-post-controls"><label><div class="penalty-post-label">' +
        t("admin.user.penalty_post_actions") +
        "</div></label></div>";
      body.appendChild(holder);
      var actions = POST_ACTIONS.filter(function (id) {
        return !(id === "delete_all" && user.trust_level >= 2);
      }).map(function (id) {
        return { id: id, name: t("admin.user.penalty_post_" + id) };
      });
      var deleteAll = null;
      var draw = function () {
        var edit = holder.querySelector(".penalty-post-edit");
        if (edit) {
          edit.remove();
        }
        if (deleteAll) {
          deleteAll.remove();
          deleteAll = null;
        }
        if (state.postAction === "edit") {
          holder.insertAdjacentHTML("beforeend", '<div class="penalty-post-edit"><textarea class="post-editor"></textarea></div>');
          var editor = holder.querySelector(".post-editor");
          editor.value = state.postEdit || "";
          editor.addEventListener("input", function () {
            state.postEdit = editor.value;
          });
          var modalBody = holder.closest(".d-modal__body");
          if (modalBody) {
            modalBody.scrollTo(0, modalBody.clientHeight);
          }
          editor.focus();
        } else if (state.postAction === "delete_all") {
          var label = (deleteAll = document.createElement("label"));
          label.innerHTML =
            '<input type="checkbox"> ' +
            messageFormat("admin.user.penalty_post_delete_all_confirmation_MF", {
              TOPICS: user.topic_count,
              REPLIES: (user.post_count || 0) - (user.topic_count || 0),
            });
          holder.appendChild(label);
          label.querySelector("input").addEventListener("click", function (event) {
            state.readyToDeleteAll = event.target.checked;
            update();
          });
        }
      };
      var combo = comboBox({
        value: state.postAction,
        rows: function () {
          return actions;
        },
        onChange: function (value) {
          state.postAction = value;
          state.readyToDeleteAll = false;
          draw();
          update();
        },
      });
      holder.querySelector(".penalty-post-controls").appendChild(combo.el);
      draw();
    }

    // AdminPenaltySimilarUsers
    function drawSimilarUsers(body) {
      var field = suspend ? "can_be_suspended" : "can_be_silenced";
      var count = user.similar_users_count;
      var section = document.createElement("div");
      section.className = "penalty-similar-users";
      section.innerHTML =
        '<p class="alert alert-warning">' +
        tCount("admin.user.other_matches", count, { username: escapeHtml(user.username) }) +
        '</p><div class="loading-container"><div class="spinner"></div></div>';
      body.appendChild(section);
      ajax("GET", "/admin/users/" + user.id + "/similar-users.json")
        .then(function (data) {
          var rows = (data && data.users) || [];
          var headers = [
            "",
            t("username"),
            t("last_seen"),
            t("admin.user.topics_entered"),
            t("admin.user.posts_read_count"),
            t("admin.user.time_read"),
            t("created"),
          ];
          section.querySelector(".loading-container").innerHTML =
            '<table class="table"><thead><tr>' +
            headers
              .map(function (h) {
                return "<th>" + escapeHtml(h) + "</th>";
              })
              .join("") +
            "</tr></thead><tbody>" +
            rows
              .map(function (u) {
                return (
                  '<tr><td><input type="checkbox" data-user-id="' +
                  escapeHtml(u.id) +
                  '"' +
                  (u[field] ? "" : " disabled") +
                  "></td><td>" +
                  avatar(u) +
                  " " +
                  escapeHtml(u.username) +
                  "</td><td>" +
                  durationTiny(u.last_seen_age) +
                  "</td><td>" +
                  number(u.topics_entered) +
                  "</td><td>" +
                  number(u.posts_read_count) +
                  "</td><td>" +
                  durationTiny(u.time_read) +
                  "</td><td>" +
                  durationTiny(u.created_at_age) +
                  "</td></tr>"
                );
              })
              .join("") +
            "</tbody></table>";
          section.querySelectorAll("input[data-user-id]").forEach(function (box) {
            box.addEventListener("click", function () {
              var id = Number(box.dataset.userId);
              if (box.checked) {
                state.otherUserIds.push(id);
              } else {
                state.otherUserIds = state.otherUserIds.filter(function (x) {
                  return x !== id;
                });
              }
            });
          });
        })
        .catch(function () {
          section.querySelector(".loading-container").innerHTML = "";
        });
    }
  }

  // AdminUser.find
  function find(userId) {
    return ajax("GET", "/admin/users/" + userId + ".json");
  }

  Discourse.penalizeUser = { show: show, find: find };
})();
