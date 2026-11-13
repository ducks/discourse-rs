// The topic admin menu's modals (static/js/topic-admin-menu.js opens
// them with the topic's state from its template's data-topic):
//
//   Discourse.topicAdminModals.featureTopic(topic)
//       FeatureTopic (components/modal/feature-topic.gjs): the counts from
//       /topics/feature_stats.json, pinning in the category or globally
//       until a date (PinOptionSection's form, the date required and in
//       the future, a confirmation past four global pins), unpinning, and
//       making or removing the banner (PUT make-banner, remove-banner).
//   Discourse.topicAdminModals.changeTimestamp(topic)
//       ChangeTimestamp: a past date (DatePickerPast, Pikaday in
//       #date-container) and time, PUT /t/:id/change-timestamp.
//   Discourse.topicAdminModals.slowMode(topic)
//       EditSlowMode: the durations' ComboBox (custom hours, minutes and
//       seconds), the enabled until DFutureDateInput, PUT /t/:id/slow_mode.
//   Discourse.topicAdminModals.makePublic(topic)
//       ConvertToPublicTopic: a copy of the composer's CategoryChooser,
//       PUT /t/:id/convert-topic/public.
//   Discourse.topicAdminModals.editTopicTimer(topic)
//       EditTopicTimer (components/modal/edit-topic-timer.gjs with
//       EditTopicTimerForm): the timer types the topic's state offers,
//       DTimeShortcutPicker or DRelativeTimePicker (bookmark-modal.js's
//       Discourse.timeShortcutPicker) by type, the category for
//       publishing, the TopicTimerInfo preview, POST /t/:id/timer to set
//       or remove. Not ported: pausing the keyboard shortcuts, the
//       preview's periodic redraw.
//
// The page is drawn again once a change is saved, where Ember re-renders
// in place. The date inputs are penalize-user.js's (Discourse
// .futureDateInput, its #penalize-user-config holding the time shortcuts'
// strings, the member's timezone and the date libraries).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.topicAdminModals) {
    return;
  }

  // PinOptionSection's MAX_GLOBALLY_PINNED_TOPICS
  var MAX_GLOBALLY_PINNED_TOPICS = 4;

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

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return Discourse.cooked ? Discourse.cooked.icon(name) : "";
  }

  // The topic's strings (data-topic's), interpolated as I18n does.
  function t(topic, key, args) {
    var s = (topic.strings || {})[key];
    if (s === undefined) {
      return "[en." + key + "]";
    }
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  function tCount(topic, key, count, args) {
    return t(topic, key + (count === 1 ? ".one" : ".other"), Object.assign({ count: count }, args || {}));
  }

  // discourse/lib/ajax: a form-encoded request, rejecting with the
  // response for a failure.
  function ajax(method, path, data) {
    var headers = memberHeaders();
    headers.Accept = "application/json";
    var init = { method: method, headers: headers, credentials: "same-origin" };
    if (data) {
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      var params = new URLSearchParams();
      Object.keys(data).forEach(function (key) {
        if (data[key] !== undefined && data[key] !== null) {
          params.append(key, data[key]);
        }
      });
      init.body = params.toString();
    }
    return fetch(base() + path, init).then(function (r) {
      if (!r.ok) {
        return Promise.reject(r);
      }
      return r.text().then(function (text) {
        try {
          return text ? JSON.parse(text) : null;
        } catch (e) {
          return null;
        }
      });
    });
  }

  // ajax-error's extractError for a failed response.
  function extractError(topic, r) {
    if (!(r instanceof Response)) {
      return Promise.resolve(t(topic, "generic_error"));
    }
    return r
      .json()
      .catch(function () {
        return null;
      })
      .then(function (json) {
        if (json && json.errors && json.errors.length > 1) {
          return t(topic, "multiple_errors", {
            errors: json.errors
              .map(function (e, i) {
                return i + 1 + ") " + e;
              })
              .join(" "),
          });
        }
        if (json && json.errors && json.errors.length) {
          return t(topic, "generic_error_with_reason", { error: json.errors[0] });
        }
        if (json && (json.error || json.message || json.failed)) {
          return json.error || json.message || json.failed;
        }
        return r.status + " " + (r.statusText || "");
      });
  }

  function reload() {
    location.reload();
  }

  // DButton
  function button(className, label, iconName, type) {
    var b = document.createElement("button");
    b.className = "btn " + (iconName ? "btn-icon-text " : "btn-text ") + className;
    b.type = type || "button";
    b.innerHTML = (iconName ? icon(iconName) : "") + '<span class="d-button-label"></span>';
    b.querySelector(".d-button-label").textContent = label;
    return b;
  }

  // DModal's @flash: the alert above the body.
  function setFlash(body, text) {
    var modal = body.closest(".d-modal");
    var flash = modal.querySelector("#modal-alert");
    if (!text) {
      if (flash) {
        flash.remove();
      }
      return;
    }
    if (!flash) {
      flash = document.createElement("div");
      flash.id = "modal-alert";
      flash.setAttribute("role", "alert");
      flash.className = "alert alert-error";
      body.parentNode.insertBefore(flash, body);
    }
    flash.textContent = text;
  }

  function dates() {
    return Discourse.futureDateInput;
  }

  // DConditionalLoadingSpinner @size="small"
  function spinner() {
    return '<div class="spinner small"></div>';
  }

  // ---- FeatureTopic ----

  // PinOptionSection: its form, the date required in the future, a
  // confirmation first when asked. `topic` holds the strings; opts:
  // { stats (the stats paragraph), note, pinMessage, buttonLabel,
  // confirmMessage(), onPin(until) }. FeatureTopic's and the bulk
  // actions' BulkPinOptions (topic-list-bulk-select.js).
  function pinOptionSection(topic, opts) {
    var m = window.moment;
    var form = document.createElement("form");
    form.className = "form-kit feature-section";
    form.setAttribute("novalidate", "");
    var summary = document.createElement("div");
    form.appendChild(summary);
    var description = document.createElement("div");
    description.className = "feature-section__description";
    form.appendChild(description);
    if (opts.stats) {
      description.appendChild(opts.stats);
    }
    var note = document.createElement("p");
    note.textContent = opts.note;
    description.appendChild(note);
    var pinMessage = document.createElement("p");
    pinMessage.innerHTML = opts.pinMessage + " " + icon("far-clock");
    pinMessage.className = "feature-section__pin-message";
    description.appendChild(pinMessage);

    var value = null;
    var field = document.createElement("div");
    field.className = "form-kit__container form-kit__field form-kit__field-custom";
    field.id = "control-pinUntil";
    field.dataset.name = "pinUntil";
    field.dataset.controlType = "custom";
    field.innerHTML = '<div class="form-kit__container-content"><div class="form-kit__control-custom"></div></div>';
    description.appendChild(field);
    dates().draw(field.querySelector(".form-kit__control-custom"), {
      className: "pin-until",
      label: "",
      onChangeInput: function (v) {
        value = v;
        if (field.classList.contains("has-error")) {
          validate();
        }
      },
    });
    var submit = button("btn-primary form-kit__button", opts.buttonLabel, null, "submit");
    description.appendChild(submit);

    // validatePinUntil, with FormKit's errors summary and the field's
    // error.
    function validate() {
      var parsed = m(value, dates().FORMAT);
      var valid = !!value && parsed.isValid() && parsed > m();
      field.classList.toggle("has-error", !valid);
      var error = field.querySelector(".form-kit__errors");
      if (error) {
        error.remove();
      }
      summary.innerHTML = "";
      summary.className = "";
      if (!valid) {
        var message = t(topic, "topic.feature_topic.pin_validation");
        var title = t(topic, "topic.feature_topic.pin_until");
        error = document.createElement("p");
        error.setAttribute("aria-live", "assertive");
        error.className = "form-kit__errors";
        error.innerHTML = "<span>" + icon("triangle-exclamation") + " " + escapeHtml(message) + "</span>";
        field.appendChild(error);
        summary.className = "form-kit__errors-summary";
        summary.setAttribute("aria-live", "assertive");
        summary.innerHTML =
          '<h2 class="form-kit__errors-summary-title">' +
          icon("triangle-exclamation") +
          " " +
          escapeHtml(tCount(topic, "form_kit.errors_summary_title", 1)) +
          '</h2><ul class="form-kit__errors-summary-list"><li><a href="#control-pinUntil" rel="noopener noreferrer">' +
          escapeHtml(title) +
          "</a>: " +
          escapeHtml(message) +
          "</li></ul>";
      }
      return valid;
    }

    form.addEventListener("submit", function (event) {
      event.preventDefault();
      if (!validate()) {
        return;
      }
      var confirm = opts.confirmMessage && opts.confirmMessage();
      if (confirm && Discourse.dialog) {
        Discourse.dialog.yesNoConfirm({
          message: confirm,
          didConfirm: function () {
            opts.onPin(value);
          },
        });
      } else {
        opts.onPin(value);
      }
    });
    return form;
  }

  function featureTopic(topic) {
    if (!dates() || !Discourse.modal) {
      return;
    }
    dates()
      .load()
      .then(function () {
        openFeatureTopic(topic);
      });
  }

  function openFeatureTopic(topic) {
    var m = window.moment;
    var stats = null;
    var categoryLink = topic.categoryLink || "";
    var canPinGlobally = topic.canManageTopic && topic.can_pin_unpin_topic;
    var isBanner = topic.archetype === "banner";

    function alreadyPinnedMessage() {
      var count = stats.pinned_in_category_count;
      if (count === 0) {
        return t(topic, "topic.feature_topic.not_pinned", { categoryLink: categoryLink });
      }
      return tCount(topic, "topic.feature_topic.already_pinned", count, { categoryLink: categoryLink });
    }

    function pinGloballyStatsMessage() {
      if (stats.pinned_globally_count) {
        return tCount(topic, "topic.feature_topic.already_pinned_globally", stats.pinned_globally_count);
      }
      return t(topic, "topic.feature_topic.not_pinned_globally");
    }

    function unPinMessage() {
      var name = "topic.feature_topic.unpin";
      if (topic.pinned_globally) {
        name += "_globally";
      }
      if (m(topic.pinned_until) > m()) {
        name += "_until";
      }
      return t(topic, name, {
        categoryLink: categoryLink,
        until: m(topic.pinned_until).format("LL"),
      });
    }

    // The stats paragraphs, a spinner while the counts load.
    var statsParagraphs = [];
    function statsParagraph(render) {
      var p = document.createElement("p");
      p.innerHTML = stats ? render() : spinner();
      statsParagraphs.push({ el: p, render: render });
      return p;
    }

    function paragraph(html) {
      var p = document.createElement("p");
      p.innerHTML = html;
      return p;
    }

    function text(s) {
      var p = document.createElement("p");
      p.textContent = s;
      return p;
    }

    // Topic#saveStatus
    function saveStatus(status, enabled, until) {
      return ajax("PUT", "/t/" + topic.id + "/status", {
        status: status,
        enabled: String(enabled),
        until: until || undefined,
      })
        .then(reload)
        .catch(function (e) {
          console.error(e);
        });
    }

    Discourse.modal.show({
      className: "feature-topic",
      title: t(topic, "topic.feature_topic.title"),
      body: function (body, close) {
        if (topic.pinned_at) {
          var section = document.createElement("div");
          section.className = "feature-section";
          var description = document.createElement("div");
          description.className = "feature-section__description";
          section.appendChild(description);
          if (topic.pinned_globally) {
            description.appendChild(statsParagraph(pinGloballyStatsMessage));
            description.appendChild(text(t(topic, "topic.feature_topic.global_pin_note")));
          } else {
            description.appendChild(statsParagraph(alreadyPinnedMessage));
            description.appendChild(text(t(topic, "topic.feature_topic.pin_note")));
          }
          description.appendChild(paragraph(unPinMessage()));
          var unpin = button("btn-primary", t(topic, "topic.feature.unpin"), "thumbtack");
          var p = document.createElement("p");
          p.appendChild(unpin);
          description.appendChild(p);
          // togglePinned: the pin the topic has, off.
          unpin.addEventListener("click", function () {
            saveStatus("pinned", false);
            close();
          });
          body.appendChild(section);
        } else {
          body.appendChild(
            pinOptionSection(topic, {
              buttonLabel: t(topic, "topic.feature.pin"),
              note: t(topic, "topic.feature_topic.pin_note"),
              pinMessage: t(topic, "topic.feature_topic.pin", { categoryLink: categoryLink }),
              stats: statsParagraph(alreadyPinnedMessage),
              onPin: function (until) {
                saveStatus("pinned", true, until);
                close();
              },
            })
          );
          if (canPinGlobally) {
            body.appendChild(document.createElement("hr"));
            body.appendChild(
              pinOptionSection(topic, {
                buttonLabel: t(topic, "topic.feature.pin_globally"),
                note: t(topic, "topic.feature_topic.global_pin_note"),
                pinMessage: t(topic, "topic.feature_topic.pin_globally"),
                stats: statsParagraph(pinGloballyStatsMessage),
                confirmMessage: function () {
                  if (stats && stats.pinned_globally_count >= MAX_GLOBALLY_PINNED_TOPICS) {
                    return tCount(topic, "topic.feature_topic.confirm_pin_globally", stats.pinned_globally_count);
                  }
                  return null;
                },
                onPin: function (until) {
                  saveStatus("pinned_globally", true, until);
                  close();
                },
              })
            );
          }
        }
        if (topic.can_banner_topic) {
          body.appendChild(document.createElement("hr"));
          var banner = document.createElement("div");
          banner.className = "feature-section";
          var bannerDescription = document.createElement("div");
          bannerDescription.className = "feature-section__description";
          banner.appendChild(bannerDescription);
          bannerDescription.appendChild(
            statsParagraph(function () {
              return t(topic, stats.banner_count ? "topic.feature_topic.banner_exists" : "topic.feature_topic.no_banner_exists");
            })
          );
          bannerDescription.appendChild(text(t(topic, "topic.feature_topic.banner_note")));
          bannerDescription.appendChild(
            text(t(topic, isBanner ? "topic.feature_topic.remove_banner" : "topic.feature_topic.make_banner"))
          );
          var action = isBanner
            ? button("btn-primary", t(topic, "topic.feature.remove_banner"), "thumbtack")
            : button("btn-primary make-banner", t(topic, "topic.feature.make_banner"), "thumbtack");
          var holder = document.createElement("p");
          holder.appendChild(action);
          bannerDescription.appendChild(holder);
          // Topic#makeBanner and #removeBanner leave a failure unhandled.
          action.addEventListener("click", function () {
            ajax("PUT", "/t/" + topic.id + (isBanner ? "/remove-banner" : "/make-banner"))
              .then(reload)
              .catch(function (e) {
                console.error(e);
              });
            close();
          });
          body.appendChild(banner);
        }
      },
    });

    // loadFeatureStats
    var query = topic.category_id ? "?category_id=" + encodeURIComponent(topic.category_id) : "";
    ajax("GET", "/topics/feature_stats.json" + query)
      .then(function (result) {
        stats = {
          pinned_in_category_count: result.pinned_in_category_count,
          pinned_globally_count: result.pinned_globally_count,
          banner_count: result.banner_count,
        };
      })
      .catch(function (e) {
        console.error(e);
        stats = stats || { pinned_in_category_count: 0, pinned_globally_count: 0, banner_count: 0 };
      })
      .then(function () {
        statsParagraphs.forEach(function (s) {
          s.el.innerHTML = s.render();
        });
      });
  }

  // ---- ChangeTimestamp ----

  function changeTimestamp(topic) {
    if (!dates() || !Discourse.modal) {
      return;
    }
    dates()
      .load()
      .then(function () {
        openChangeTimestamp(topic);
      });
  }

  function openChangeTimestamp(topic) {
    var m = window.moment;
    var date = m().format("YYYY-MM-DD");
    var time;
    var saving = false;
    var submit = null;
    var invalid = null;
    var picker = null;

    function createdAt() {
      return m(date + " " + time, "YYYY-MM-DD HH:mm:ss");
    }

    // validTimestamp: a time still to come (the alert's condition).
    function future() {
      return m().diff(createdAt(), "minutes") < 0;
    }

    function update() {
      if (invalid) {
        invalid.classList.toggle("hidden", !future());
      }
      if (submit) {
        submit.disabled = saving || future() || !date;
        submit.querySelector(".d-button-label").textContent = t(
          topic,
          saving ? "saving" : "topic.change_timestamp.action"
        );
      }
    }

    var bodyEl = null;
    Discourse.modal.show({
      title: t(topic, "topic.change_timestamp.title"),
      onClose: function () {
        if (picker) {
          picker.destroy();
          picker = null;
        }
      },
      body: function (body) {
        bodyEl = body;
        body.classList.add("change-timestamp");
        var instructions = document.createElement("p");
        instructions.textContent = t(topic, "topic.change_timestamp.instructions");
        body.appendChild(instructions);
        invalid = document.createElement("p");
        invalid.className = "alert alert-error hidden";
        invalid.textContent = t(topic, "topic.change_timestamp.invalid_timestamp");
        body.appendChild(invalid);
        var form = document.createElement("form");
        var mobile = document.documentElement.classList.contains("mobile-view");
        form.innerHTML =
          '<div class="date-picker-wrapper"><input autocomplete="off" class="date-picker" type="' +
          (mobile ? "date" : "text") +
          '"></div><input type="time">';
        body.appendChild(form);
        var container = document.createElement("div");
        container.id = "date-container";
        body.appendChild(container);
        var dateInput = form.querySelector(".date-picker");
        dateInput.placeholder = dates().t("dates.placeholder");
        dateInput.value = date;
        var timeInput = form.querySelector('input[type="time"]');
        form.addEventListener("submit", function (event) {
          event.preventDefault();
        });
        timeInput.addEventListener("input", function () {
          time = timeInput.value;
          update();
        });
        var onSelect = function (value) {
          date = value ? m(value).format("YYYY-MM-DD") : "";
          dateInput.value = date;
          update();
        };
        if (mobile) {
          dateInput.addEventListener("change", function () {
            onSelect(dateInput.value);
          });
        } else {
          // DatePickerPast: Pikaday in #date-container, nothing after
          // today.
          dates()
            .loadPikaday()
            .then(function () {
              if (!dateInput.isConnected) {
                return;
              }
              picker = new window.Pikaday({
                field: dateInput,
                container: container,
                bound: false,
                format: "YYYY-MM-DD",
                firstDay: 1,
                maxDate: new Date(),
                defaultDate: m(date, "YYYY-MM-DD").toDate(),
                setDefaultDate: false,
                i18n: {
                  previousMonth: dates().t("dates.previous_month"),
                  nextMonth: dates().t("dates.next_month"),
                  months: m.months(),
                  weekdays: m.weekdays(),
                  weekdaysShort: m.weekdaysMin(),
                },
                onSelect: onSelect,
              });
              picker.setDate(m(date).toDate(), true);
            });
        }
      },
      footer: function (footer, close) {
        submit = button("btn-primary", t(topic, "topic.change_timestamp.action"));
        footer.appendChild(submit);
        update();
        submit.addEventListener("click", function () {
          saving = true;
          update();
          setFlash(bodyEl, null);
          ajax("PUT", "/t/" + topic.id + "/change-timestamp", { timestamp: String(createdAt().unix()) })
            .then(function (result) {
              if (!result || !result.success) {
                throw new Error("error updating timestamp of topic");
              }
              close();
              reload();
            })
            .catch(function () {
              setFlash(bodyEl, t(topic, "topic.change_timestamp.error"));
            })
            .then(function () {
              saving = false;
              update();
            });
        });
      },
    });
  }

  // ---- EditSlowMode ----

  var SLOW_MODE_OPTIONS = [
    ["600", "10_minutes"],
    ["900", "15_minutes"],
    ["1800", "30_minutes"],
    ["2700", "45_minutes"],
    ["3600", "1_hour"],
    ["7200", "2_hours"],
    ["14400", "4_hours"],
    ["28800", "8_hours"],
    ["43200", "12_hours"],
    ["86400", "24_hours"],
    ["custom", "custom"],
  ];

  // helpers/slow-mode's fromSeconds and toSeconds
  function fromSeconds(seconds) {
    var rest = seconds;
    var hours = Math.trunc(rest / 3600);
    if (hours >= 1) {
      rest -= 3600 * hours;
    } else {
      hours = 0;
    }
    var minutes = Math.trunc(rest / 60);
    if (minutes >= 1) {
      rest -= 60 * minutes;
    } else {
      minutes = 0;
    }
    return { hours: hours, minutes: minutes, seconds: rest };
  }

  function slowMode(topic) {
    if (!dates() || !Discourse.modal) {
      return;
    }
    dates()
      .load()
      .then(function () {
        openSlowMode(topic);
      });
  }

  function openSlowMode(topic) {
    var modes = SLOW_MODE_OPTIONS.map(function (o) {
      return { id: o[0], name: t(topic, "topic.slow_mode_update.durations." + o[1]) };
    });
    var current = parseInt(topic.slow_mode_seconds, 10);
    var state = { selected: null, hours: null, minutes: null, seconds: null };
    var enabledUntil = topic.slow_mode_enabled_until;
    function setFromSeconds(seconds) {
      var parts = fromSeconds(seconds);
      state.hours = parts.hours;
      state.minutes = parts.minutes;
      state.seconds = parts.seconds;
    }
    if (current) {
      state.selected = modes.some(function (mode) {
        return mode.id === String(current);
      })
        ? String(current)
        : "custom";
      setFromSeconds(current);
    }
    var bodyEl = null;
    var custom = null;

    function parse(v) {
      return parseInt(v, 10) || 0;
    }

    function drawCustom() {
      if (custom) {
        custom.remove();
        custom = null;
      }
      if (state.selected !== "custom") {
        return;
      }
      custom = document.createElement("div");
      custom.className = "control-group";
      ["hours", "minutes", "seconds"].forEach(function (unit) {
        var label = document.createElement("label");
        label.className = "slow-mode-label";
        label.textContent = t(topic, "topic.slow_mode_update." + unit);
        var input = document.createElement("input");
        input.className = "input-small";
        input.type = "number";
        input.value = state[unit] === null || state[unit] === undefined ? "" : state[unit];
        input.addEventListener("input", function () {
          state[unit] = input.value;
        });
        custom.appendChild(label);
        custom.appendChild(input);
      });
      bodyEl.querySelector(".slow-mode-type").closest(".control-group").after(custom);
    }

    function setSlowMode(seconds, until) {
      return ajax("PUT", "/t/" + topic.id + "/slow_mode", { seconds: String(seconds), enabled_until: until || "" });
    }

    Discourse.modal.show({
      className: "edit-slow-mode-modal",
      title: t(topic, "topic.slow_mode_update.title"),
      body: function (body) {
        bodyEl = body;
        body.innerHTML =
          '<div class="control-group"><label class="slow-mode-label"></label></div>' +
          '<div class="control-group"><label class="slow-mode-label"></label></div>' +
          '<div class="control-group enabled-until-group"></div>';
        var labels = body.querySelectorAll(".slow-mode-label");
        labels[0].textContent = t(topic, "topic.slow_mode_update.description");
        labels[1].textContent = t(topic, "topic.slow_mode_update.select");
        var combo = dates().comboBox({
          className: "slow-mode-type",
          value: state.selected,
          none: "",
          rows: function () {
            return modes;
          },
          onChange: function (value) {
            // setSlowModeDuration
            if (value !== "custom") {
              setFromSeconds(parseInt(value, 10));
            }
            state.selected = value;
            drawCustom();
          },
        });
        labels[1].parentNode.appendChild(combo.el);
        drawCustom();
        dates().draw(body.querySelector(".enabled-until-group"), {
          className: "enabled-until",
          label: t(topic, "topic.slow_mode_update.enabled_until"),
          labelClasses: "slow-mode-label",
          input: enabledUntil,
          customShortcuts: [
            "later_today",
            "tomorrow",
            "two_days",
            { id: "start_of_next_business_week", label: "next_week" },
            "two_weeks",
            "next_month",
            "two_months",
          ],
          onChangeInput: function (v) {
            enabledUntil = v;
          },
        });
      },
      footer: function (footer, close) {
        var save = button(
          "btn-primary",
          t(topic, current ? "topic.slow_mode_update.update" : "topic.slow_mode_update.enable"),
          "hourglass-start"
        );
        footer.appendChild(save);
        save.addEventListener("click", function () {
          save.disabled = true;
          var seconds = parse(state.seconds) + parse(state.hours) * 3600 + parse(state.minutes) * 60;
          setSlowMode(seconds, enabledUntil)
            .then(function () {
              close();
              reload();
            })
            .catch(function () {
              setFlash(bodyEl, t(topic, "generic_error"));
            })
            .then(function () {
              save.disabled = false;
            });
        });
        if (current) {
          var remove = button("btn-danger", t(topic, "topic.slow_mode_update.remove"));
          footer.appendChild(remove);
          remove.addEventListener("click", function () {
            remove.disabled = true;
            setSlowMode(0)
              .then(function () {
                close();
                reload();
              })
              .catch(function (r) {
                return extractError(topic, r).then(function (message) {
                  setFlash(bodyEl, message);
                });
              })
              .then(function () {
                remove.disabled = false;
              });
          });
        }
      },
    });
  }

  // ---- ConvertToPublicTopic ----

  // The select-kit body floated under its header (select-kit's
  // placement).
  function floatBody(details) {
    var body = details.querySelector(".select-kit-body");
    var summary = details.querySelector("summary");
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
      });
  }

  // CategoryChooser: a copy of the composer's, nothing chosen. opts:
  // { excludeCategoryId, value } (a category left out, the one chosen).
  function categoryChooser(topic, onChange, opts) {
    opts = opts || {};
    var source = document.querySelector("#reply-control details.category-chooser");
    if (!source) {
      return null;
    }
    var chooser = source.cloneNode(true);
    if (opts.excludeCategoryId) {
      chooser.querySelectorAll('.category-row[data-value="' + Number(opts.excludeCategoryId) + '"]').forEach(function (row) {
        row.remove();
      });
    }
    chooser.open = false;
    chooser.classList.remove("is-expanded", "has-selection");
    var filter = chooser.querySelector(".filter-input");
    var header = chooser.querySelector(".select-kit-selected-name");
    var summary = chooser.querySelector("summary");
    summary.dataset.value = "";
    // The composer's header may name its category: the modal's starts
    // with none.
    header.innerHTML = '<span class="name"></span>';
    header.querySelector(".name").textContent = t(topic, "category.choose").replace("&hellip;", "…");
    delete header.dataset.value;
    delete header.dataset.name;
    header.removeAttribute("title");
    filter.value = "";
    chooser.querySelectorAll(".category-row").forEach(function (row) {
      row.hidden = false;
    });
    chooser.addEventListener("toggle", function () {
      chooser.classList.toggle("is-expanded", chooser.open);
      if (chooser.open) {
        floatBody(chooser);
        filter.focus();
      } else {
        chooser.querySelector(".select-kit-body").removeAttribute("style");
      }
    });
    filter.addEventListener("input", function () {
      var term = filter.value.toLowerCase();
      chooser.querySelectorAll(".category-row").forEach(function (row) {
        row.hidden = term !== "" && row.dataset.name.toLowerCase().indexOf(term) < 0;
      });
    });
    function choose(row) {
      header.innerHTML = "";
      var name = document.createElement("span");
      name.className = "name";
      row.querySelectorAll(".category-status > .badge-category__wrapper").forEach(function (b) {
        var badge = b.cloneNode(true);
        var count = badge.querySelector(".topic-count");
        if (count) {
          count.remove();
        }
        name.appendChild(badge);
      });
      header.appendChild(name);
      header.dataset.value = row.dataset.value;
      header.dataset.name = row.dataset.name;
      header.title = row.dataset.name;
      summary.dataset.value = row.dataset.value;
      chooser.classList.add("has-selection");
    }
    if (opts.value) {
      var chosen = chooser.querySelector('.category-row[data-value="' + Number(opts.value) + '"]');
      if (chosen) {
        choose(chosen);
      }
    }
    chooser.addEventListener("click", function (event) {
      var row = event.target.closest(".category-row");
      if (!row) {
        return;
      }
      event.preventDefault();
      choose(row);
      chooser.open = false;
      onChange(parseInt(row.dataset.value, 10));
    });
    document.addEventListener("click", function (event) {
      if (chooser.open && !chooser.contains(event.target)) {
        chooser.open = false;
      }
    });
    return chooser;
  }

  function makePublic(topic) {
    if (!Discourse.modal) {
      return;
    }
    var categoryId;
    var bodyEl = null;
    Discourse.modal.show({
      className: "convert-to-public-topic",
      title: t(topic, "topic.make_public.title"),
      body: function (body) {
        bodyEl = body;
        var instructions = document.createElement("div");
        instructions.className = "instructions";
        instructions.textContent = t(topic, "topic.make_public.choose_category");
        body.appendChild(instructions);
        var chooser = categoryChooser(topic, function (id) {
          categoryId = id;
        });
        if (chooser) {
          body.appendChild(chooser);
        }
      },
      footer: function (footer, close) {
        var ok = button("btn-primary", t(topic, "composer.modal_ok"));
        var cancel = button("btn-flat d-modal-cancel", t(topic, "cancel"));
        footer.appendChild(ok);
        footer.appendChild(cancel);
        cancel.addEventListener("click", close);
        ok.addEventListener("click", function () {
          ok.disabled = true;
          // Topic#convertTopic("public", { categoryId })
          ajax("PUT", "/t/" + topic.id + "/convert-topic/public", { category_id: categoryId })
            .then(function () {
              close();
              reload();
            })
            .catch(function (r) {
              return extractError(topic, r).then(function (message) {
                setFlash(bodyEl, message);
              });
            })
            .then(function () {
              ok.disabled = false;
            });
        });
      },
    });
  }

  // ---- EditTopicTimer ----

  // future-date-input-selector's FORMAT
  var TIMER_FORMAT = "YYYY-MM-DD HH:mmZ";
  // d-native-select's NO_VALUE_OPTION
  var NO_VALUE_OPTION = "__NONE__";
  var AFTER_LAST_POST = ["close_after_last_post", "delete_after_last_post"];

  // publicTimerTypes
  function timerTypes(topic) {
    var types = [];
    function add(id, key) {
      types.push({ id: id, name: t(topic, "topic." + key + ".title") });
    }
    if (!topic.closed) {
      add("close", "auto_close");
      add("close_after_last_post", "auto_close_after_last_post");
    }
    if (topic.closed) {
      add("open", "auto_reopen");
    }
    if (topic.can_delete) {
      add("delete", "auto_delete");
      add("delete_after_last_post", "auto_delete_after_last_post");
    }
    add("bump", "auto_bump");
    if (topic.can_delete) {
      add("delete_replies", "auto_delete_replies");
    }
    if (topic.closed) {
      add("close", "temp_open");
    }
    if (!topic.closed) {
      add("open", "temp_close");
    }
    if (topic.category_read_restricted || topic.private_message || !topic.visible) {
      add("publish_to_category", "publish_to_category");
    }
    return types;
  }

  // Category.findById, of the categories the chooser lists: its slug and
  // url (the row's data-url).
  function timerCategory(id) {
    var row = id && document.querySelector('#reply-control details.category-chooser .category-row[data-value="' + Number(id) + '"]');
    if (!row || !row.dataset.url) {
      return null;
    }
    var parts = row.dataset.url.split("/");
    return { slug: parts[parts.length - 2], url: row.dataset.url };
  }

  // TopicTimerInfo (components/topic-timer-info.gjs) as the modal draws
  // it, without the edit and remove buttons: the notice, or nothing.
  function topicTimerInfo(topic, a) {
    var m = window.moment;
    var el = document.createElement("div");
    el.className = "topic-timer-info";
    if (!a.executeAt) {
      return el;
    }
    var deleteReplies = a.statusType === "delete_replies";
    var left = m.duration(m(a.executeAt) - m());
    if (!(left.asMinutes() > 0 || deleteReplies || a.basedOnLastPost)) {
      return el;
    }
    var options = {
      timeLeft: left.humanize(true),
      duration: m.duration(parseInt(a.durationMinutes, 10) || 0, "minutes").humanize({ s: 60, m: 60, h: 24 }),
    };
    var category = timerCategory(a.categoryId);
    if (category) {
      options.categoryName = escapeHtml(category.slug);
      options.categoryUrl = escapeHtml(category.url);
    }
    // _noticeKey
    var statusType = a.statusType === "silent_close" ? "close" : a.statusType;
    if (a.basedOnLastPost && (statusType === "close" || statusType === "delete")) {
      statusType += "_after_last_post";
    }
    el.innerHTML =
      '<h3 class="topic-timer-heading"><span>' +
      icon("far-clock") +
      " " +
      t(topic, "topic.status_update_notice.auto_" + statusType, options) +
      '</span><div class="topic-timer-modify"></div></h3>';
    el.querySelector("span").title = m(a.executeAt).format("LLLL");
    return el;
  }

  function editTopicTimer(topic) {
    var pickers = Discourse.timeShortcutPicker;
    if (!pickers || !Discourse.modal) {
      return;
    }
    pickers.load().then(function () {
      openEditTopicTimer(topic);
    });
  }

  function openEditTopicTimer(topic) {
    var m = window.moment;
    var pickers = Discourse.timeShortcutPicker;
    var types = timerTypes(topic);
    // The topic's timer, else createDefaultTimer.
    var timer = topic.topic_timer ? Object.assign({}, topic.topic_timer) : { status_type: types[0].id };
    // The picker's last choice (EditTopicTimerForm's timerType).
    var timerType = null;
    var bodyEl = null;
    var form = null;
    var select = null;
    var picker = null;
    var marks = {};
    var parts = {};

    function statusType() {
      if (timer.based_on_last_post && (timer.status_type === "close" || timer.status_type === "delete")) {
        return timer.status_type + "_after_last_post";
      }
      return timer.status_type;
    }

    function useDuration() {
      return AFTER_LAST_POST.concat("delete_replies").indexOf(statusType()) !== -1;
    }

    function showFutureDateInput() {
      return ["open", "delete", "bump", "publish_to_category", "close"].indexOf(statusType()) !== -1;
    }

    function executeAt() {
      if (useDuration()) {
        return m().add(parseFloat(timer.duration_minutes), "minutes").format(TIMER_FORMAT);
      }
      return timer.updateTime;
    }

    // willCloseImmediately, willDeleteImmediately
    function immediately(type) {
      return (
        statusType() === type &&
        !!timer.duration_minutes &&
        m(topic.last_posted_at).add(timer.duration_minutes, "minutes") < m()
      );
    }

    function showTopicTimerInfo() {
      if (!statusType() || immediately("close_after_last_post") || immediately("delete_after_last_post")) {
        return false;
      }
      if (statusType() === "publish_to_category" && !timer.category_id) {
        return false;
      }
      if (timerType === "custom" && timer.updateTime) {
        if (m(timer.updateTime) < m()) {
          return false;
        }
      } else if (useDuration()) {
        return !!timer.duration_minutes;
      }
      return !!timer.updateTime;
    }

    // A part of the form drawn before its mark, or taken out.
    function place(name, shown) {
      var el = parts[name];
      if (shown && el.parentNode !== form) {
        form.insertBefore(el, marks[name]);
      } else if (!shown && el.parentNode === form) {
        el.remove();
      }
      return shown;
    }

    // The warnings and the TopicTimerInfo preview.
    function drawNotices() {
      form.querySelectorAll(":scope > .warning, :scope > .modal-topic-timer-info").forEach(function (el) {
        el.remove();
      });
      [
        ["close_after_last_post", "topic.auto_close_immediate"],
        ["delete_after_last_post", "topic.auto_delete_immediate"],
      ].forEach(function (pair) {
        if (!immediately(pair[0])) {
          return;
        }
        var hours = Math.round((new Date() - new Date(topic.last_posted_at)) / (1000 * 60 * 60));
        var warning = document.createElement("div");
        warning.className = "warning";
        warning.innerHTML = icon("triangle-exclamation") + " " + escapeHtml(tCount(topic, pair[1], hours));
        form.appendChild(warning);
      });
      if (!showTopicTimerInfo()) {
        return;
      }
      var info = document.createElement("div");
      info.className = "alert alert-info modal-topic-timer-info";
      info.appendChild(
        topicTimerInfo(topic, {
          basedOnLastPost: timer.based_on_last_post,
          categoryId: timer.category_id,
          durationMinutes: timer.duration_minutes,
          executeAt: executeAt(),
          statusType: statusType(),
        })
      );
      form.appendChild(info);
    }

    function drawOptions() {
      var value = statusType();
      var none = value ? "none_placeholder" : "select_placeholder";
      select.innerHTML = [{ id: NO_VALUE_OPTION, name: t(topic, none) }]
        .concat(types)
        .map(function (type) {
          var selected = type.id === (value || NO_VALUE_OPTION);
          return (
            '<option class="d-native-select__option' +
            (selected ? ' --selected" selected="' : "") +
            '" value="' +
            type.id +
            '">' +
            escapeHtml(type.name) +
            "</option>"
          );
        })
        .join("");
      select.value = value || NO_VALUE_OPTION;
    }

    // The form's parts for the timer's type, as its template's ifs.
    function draw() {
      if (place("publish", statusType() === "publish_to_category") && !parts.publish.querySelector("details")) {
        var chooser = categoryChooser(
          topic,
          function (id) {
            timer.category_id = id;
            drawNotices();
          },
          // excludeCategoryId
          { excludeCategoryId: topic.visible ? topic.category_id : null, value: timer.category_id }
        );
        if (chooser) {
          parts.publish.appendChild(chooser);
        }
      }
      place("whenLabel", showFutureDateInput());
      if (place("when", showFutureDateInput())) {
        if (!picker) {
          picker = pickers.mount(parts.when, {
            timezone: pickers.timezone(),
            prefilledDatetime: timer.execute_at || null,
            // hiddenTimeShortcutOptions
            hiddenOptions: ["none", "later_today", "later_this_week"],
            // timeOptions
            timeShortcuts: function (shortcuts) {
              return [
                shortcuts.laterToday(),
                shortcuts.tomorrow(),
                shortcuts.laterThisWeek(),
                shortcuts.thisWeekend(),
                shortcuts.monday(),
                shortcuts.twoWeeks(),
                shortcuts.nextMonth(),
                shortcuts.sixMonths(),
              ];
            },
            // onTimeSelected, onChangeInput
            onTimeSelected: function (type, time) {
              timerType = type;
              timer.updateTime = m.isMoment(time) ? time.format(TIMER_FORMAT) : time;
              drawNotices();
            },
          });
        }
      } else if (picker) {
        picker.destroy();
        picker = null;
        parts.when.innerHTML = "";
      }
      if (place("duration", useDuration())) {
        if (!parts.duration.querySelector(".relative-time-picker")) {
          parts.duration.appendChild(
            pickers.relativeTimePicker({
              durationMinutes: timer.duration_minutes,
              // changeDuration
              onChange: function (minutes) {
                timer.duration_minutes = minutes;
                drawNotices();
              },
            })
          );
        }
      } else {
        var relative = parts.duration.querySelector(".relative-time-picker");
        if (relative) {
          relative.remove();
        }
      }
      if (parts.publish.parentNode !== form) {
        var old = parts.publish.querySelector("details");
        if (old) {
          old.remove();
        }
      }
      drawNotices();
    }

    function group(name, label) {
      var el = document.createElement("div");
      el.className = "control-group";
      el.innerHTML = '<label class="control-label"></label>';
      el.firstChild.textContent = t(topic, "topic.topic_status_update." + label);
      parts[name] = el;
    }

    // TopicTimer.update, the page drawn again after. popupAjaxError on a
    // failure.
    function setTimer(button, close, data) {
      button.disabled = true;
      return ajax("POST", "/t/" + topic.id + "/timer", data)
        .then(function () {
          close();
          reload();
        })
        .catch(function (r) {
          button.disabled = false;
          return extractError(topic, r).then(function (message) {
            if (Discourse.dialog) {
              Discourse.dialog.popupAjaxError(message);
            } else {
              window.alert(message);
            }
          });
        });
    }

    // The type the server knows, for the after last post ones.
    function serverType() {
      var type = timer.status_type;
      return AFTER_LAST_POST.indexOf(type) !== -1 ? type.replace("_after_last_post", "") : type;
    }

    Discourse.modal.show({
      className: "edit-topic-timer-modal",
      attributes: { id: "topic-timer-modal" },
      title: t(topic, "topic.topic_status_update.title"),
      body: function (body) {
        bodyEl = body;
        form = document.createElement("form");
        form.addEventListener("submit", function (event) {
          event.preventDefault();
        });
        form.innerHTML = '<div class="control-group"><select class="d-native-select timer-type"></select></div>';
        select = form.querySelector("select");
        ["publish", "whenLabel", "when", "duration"].forEach(function (name) {
          marks[name] = document.createComment(name);
          form.appendChild(marks[name]);
        });
        group("publish", "publish_to");
        group("duration", "duration");
        parts.whenLabel = document.createElement("label");
        parts.whenLabel.className = "control-label";
        parts.whenLabel.textContent = t(topic, "topic.topic_status_update.when");
        parts.when = document.createElement("div");
        body.appendChild(form);
        drawOptions();
        // onChangeStatusType
        select.addEventListener("input", function () {
          var value = select.value === NO_VALUE_OPTION ? null : select.value;
          timer.based_on_last_post = AFTER_LAST_POST.indexOf(value) !== -1;
          timer.status_type = value;
          drawOptions();
          draw();
        });
        draw();
      },
      footer: function (footer, close) {
        var save = button("btn-primary", t(topic, "topic.topic_status_update.save"));
        footer.appendChild(save);
        // saveTimer
        save.addEventListener("click", function () {
          var flash = null;
          if (!timer.updateTime && !timer.duration_minutes) {
            flash = "time_frame_required";
          } else if (timer.duration_minutes && !timer.updateTime && timer.duration_minutes <= 0) {
            flash = "min_duration";
          } else if (timer.duration_minutes && !timer.updateTime && timer.duration_minutes > 20 * 365 * 1440) {
            // No more than 20 years.
            flash = "max_duration";
          } else if (timer.status_type === "publish_to_category" && !timer.category_id) {
            flash = "category_required";
          }
          setFlash(bodyEl, flash && t(topic, "topic.topic_status_update." + flash));
          if (flash) {
            return;
          }
          // jQuery sends a null time as an empty one.
          setTimer(save, close, {
            time: timer.updateTime || "",
            status_type: serverType(),
            based_on_last_post: timer.based_on_last_post || undefined,
            category_id: timer.category_id || undefined,
            duration_minutes: timer.duration_minutes || undefined,
          });
        });
        if (timer.execute_at) {
          var remove = button("btn-danger", t(topic, "topic.topic_status_update.remove"));
          footer.appendChild(remove);
          // removeTimer
          remove.addEventListener("click", function () {
            setTimer(remove, close, { time: "", status_type: serverType() });
          });
        }
      },
      onClose: function () {
        if (picker) {
          picker.destroy();
          picker = null;
        }
      },
    });
  }

  Discourse.topicAdminModals = {
    editTopicTimer: editTopicTimer,
    featureTopic: featureTopic,
    changeTimestamp: changeTimestamp,
    slowMode: slowMode,
    makePublic: makePublic,
    // For the MoveToTopic modal (select-posts.js) and BulkTopicActions
    // (topic-list-bulk-select.js).
    categoryChooser: categoryChooser,
    floatBody: floatBody,
    pinOptionSection: pinOptionSection,
    MAX_GLOBALLY_PINNED_TOPICS: MAX_GLOBALLY_PINNED_TOPICS,
  };
})();
