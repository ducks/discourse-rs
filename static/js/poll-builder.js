// discourse-poll's builder (components/modal/poll-ui-builder.gjs with
// lib/poll-settings.js), the composer options menu's Build poll: the
// poll's type, its options (one input each, or one per line in advanced
// mode), its min, max and step, who voted shown, and in advanced mode its
// title, dynamic options, groups, automatic close, results and chart,
// inserted at the cursor as the [poll] markdown it builds. Loaded by
// static/js/composer.js the first time it is asked for, with the strings
// and settings the composer's data-poll-builder carries.
//
//   Discourse.pollBuilder.show(config, { getText, addText })
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

  function show(config, composer) {
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
      close: { date: "", time: "" },
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
      return m.options.filter(function (o) {
        return o.trim();
      }).length;
    };

    function minNumOfOptionsValidation() {
      if (!isNumber()) {
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

    // The automatic close, as DDateTimeInput gives it (local time).
    function closeIso() {
      if (!m.close.date) {
        return null;
      }
      var at = new Date(m.close.date + "T" + (m.close.time || "00:00"));
      return isNaN(at.getTime()) ? null : at.toISOString();
    }

    // pollAttrsFromSettings, for a new poll.
    function pollAttrs() {
      return {
        name: null,
        type: m.type || null,
        results: m.result || null,
        min: hasRange() ? rangeAttr(m.min) : null,
        max: hasRange() ? rangeAttr(m.max) : null,
        step: isNumber() ? String(Math.max(Number(m.step) || 1, 1)) : null,
        public: String(m.publicPoll),
        chartType: !isNumber() && m.chartType ? m.chartType : null,
        dynamic: m.dynamic ? "true" : null,
        groups: m.groups.length ? m.groups.join(",") : null,
        close: closeIso(),
        status: null,
        order: null,
      };
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
      if (m.advanced) {
        html +=
          '<div class="input-group poll-title"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_title.label")) + '</label><input type="text" value="' +
          escapeHtml(m.title) + '"></div>';
      }
      if (!isNumber()) {
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
          escapeHtml(t("poll.ui_builder.poll_groups.label")) + '</label><select class="group-chooser" multiple>' +
          (config.groups || [])
            .map(function (g) {
              return (
                '<option value="' + escapeHtml(g) + '"' + (m.groups.indexOf(g) !== -1 ? " selected" : "") + ">" +
                escapeHtml(g) + "</option>"
              );
            })
            .join("") +
          "</select></div>";
        html +=
          '<div class="input-group poll-date"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.automatic_close.label")) +
          '</label><div class="d-date-time-input"><div class="d-date-input"><input class="date-picker" type="date" value="' +
          escapeHtml(m.close.date) + '"></div><div class="d-time-input"><input class="time-picker" type="time" value="' +
          escapeHtml(m.close.time) + '"></div></div></div>';
        var results = [
          [ALWAYS, "always"],
          ["on_vote", "vote"],
          ["on_close", "closed"],
        ];
        if (config.staff) {
          results.push(["staff_only", "staff"]);
        }
        html +=
          '<div class="input-group poll-select"><label class="input-group-label">' +
          escapeHtml(t("poll.ui_builder.poll_result.label")) + '</label><select class="poll-result combo-box">' +
          results
            .map(function (r) {
              return (
                '<option value="' + r[0] + '"' + (m.result === r[0] ? " selected" : "") + ">" +
                escapeHtml(t("poll.ui_builder.poll_result." + r[1])) + "</option>"
              );
            })
            .join("") +
          "</select></div>";
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

    function redraw() {
      body.innerHTML = bodyHtml();
      insertButton.disabled = disableInsert();
      advancedButton.querySelector(".d-button-label").textContent = t(
        m.advanced ? "advanced_mode_toggle.simple_mode" : "advanced_mode_toggle.advanced_mode"
      );
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
      title: t("poll.ui_builder.title"),
      body: function (el) {
        body = el;
      },
      footer: function (el, closeModal) {
        el.innerHTML =
          '<button class="btn btn-icon-text btn-primary insert-poll" type="button">' + svg("chart-bar") +
          '<span class="d-button-label">' + escapeHtml(t("poll.ui_builder.insert")) + "</span></button>" +
          '<button class="btn btn-text btn-flat" type="button"><span class="d-button-label">' + escapeHtml(t("cancel")) +
          "</span></button>" +
          '<button class="btn btn-icon-text btn-default advanced-mode-btn" type="button">' + svg("gear") +
          '<span class="d-button-label"></span></button>';
        insertButton = el.querySelector(".insert-poll");
        advancedButton = el.querySelector(".advanced-mode-btn");
        insertButton.addEventListener("click", function () {
          if (disableInsert()) {
            return;
          }
          composer.addText(pollOutput());
          closeModal();
        });
        el.querySelector(".btn-flat").addEventListener("click", closeModal);
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
      } else if (el.matches(".date-picker")) {
        m.close.date = el.value;
      } else if (el.matches(".time-picker")) {
        m.close.time = el.value;
      }
    });

    body.addEventListener("change", function (event) {
      var el = event.target;
      if (el.matches(".group-chooser")) {
        m.groups = Array.prototype.filter
          .call(el.options, function (o) {
            return o.selected;
          })
          .map(function (o) {
            return o.value;
          });
      } else if (el.matches(".poll-result")) {
        m.result = el.value;
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

  Discourse.pollBuilder = { show: show };
})();
