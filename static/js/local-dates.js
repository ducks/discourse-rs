// discourse-local-dates in the browser, which the layout loads while
// discourse_local_dates_enabled is on: initializers/discourse-local-dates.js
// with lib/local-date-builder.js and lib/date-with-zone-helper.js, and core's
// lib/download-calendar.js with the download-calendar modal.
//
//   Discourse.applyLocalDates(root) -> Promise
//       draws each .discourse-local-date under root in the viewer's
//       timezone (moment.tz.guess()), as applyLocalDates does: posts (as
//       a cooked decorator, which also sets data-title), the composer's
//       preview and chat messages.
//
// A click on a date opens its tooltip (identifier "local-date") with the
// date in the viewer's, the post's and the listed timezones, and for a
// date to come "Add to calendar". moment and moment-timezone are
// Discourse's own builds (static/vendor/README), loaded the first time a
// page has a date. The script tag carries the settings and strings.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.applyLocalDates) {
    return;
  }

  var script = document.currentScript;
  var config = {
    moment: script.dataset.moment,
    momentTimezone: script.dataset.momentTimezone,
    defaultTimezones: script.dataset.defaultTimezones || "",
    defaultTitle: script.dataset.defaultTitle || "",
    siteTitle: script.dataset.siteTitle || "",
    defaultCalendar: script.dataset.defaultCalendar || "",
    username: script.dataset.username || "",
    strings: {},
  };
  try {
    config.strings = JSON.parse(script.dataset.strings || "{}");
  } catch (e) {
    window.console && console.error("local-dates strings", e);
  }

  function i18n(key, params) {
    var text = config.strings[key];
    if (text === undefined) {
      return key;
    }
    return text.replace(/%\{(\w+)\}/g, function (m, name) {
      return params && params[name] !== undefined ? params[name] : m;
    });
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // renderIcon("string", name) (and iconHTML), quotes as it writes them.
  function iconHTML(name) {
    return (
      "<svg class='fa d-icon d-icon-" + escapeHtml(name) +
      " svg-icon fa-width-auto svg-string' width='1em' height='1em' aria-hidden='true' xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#" +
      escapeHtml(name) + '" /></svg>'
    );
  }

  // load-moment.js: moment with moment-timezone's 10-year-range data, as
  // window.moment.
  var momentLoading = null;
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
  function loadMoment() {
    if (window.moment && window.moment.tz) {
      return Promise.resolve(window.moment);
    }
    if (!momentLoading) {
      momentLoading = loadScript(config.moment)
        .then(function () {
          return loadScript(config.momentTimezone);
        })
        .then(function () {
          return window.moment;
        });
    }
    return momentLoading;
  }

  // Ember's dasherize: decamelize, then spaces and underscores to dashes.
  function dasherize(s) {
    return s
      .replace(/([a-z\d])([A-Z])/g, "$1_$2")
      .toLowerCase()
      .replace(/[ _]/g, "-");
  }

  // utilities.js slugify
  function slugify(string) {
    return string
      .trim()
      .toLowerCase()
      .replace(/\s|_+/g, "-")
      .replace(/[^\w\-]+/g, "")
      .replace(/\-\-+/g, "-")
      .replace(/^-+/, "")
      .replace(/-+$/, "");
  }

  // bbcode-attributes.js bbcodeAttributeDecode
  function bbcodeAttributeDecode(encodedValue) {
    var base64 = encodedValue.replace(/-/g, "+").replace(/_/g, "/").replace(/~/g, "=");
    var binaryString = atob(base64);
    var bytes = new Uint8Array(binaryString.length);
    for (var i = 0; i < binaryString.length; i++) {
      bytes[i] = binaryString.charCodeAt(i);
    }
    return new TextDecoder().decode(bytes);
  }

  // ---- lib/date-with-zone-helper.js

  function DateWithZoneHelper(params) {
    params = params || {};
    this.timezone = params.timezone || "UTC";
    this.localTimezone = params.localTimezone || moment.tz.guess();
    // getProperties: every key, undefined ones included.
    this.datetime = moment.tz(
      {
        year: params.year,
        month: params.month,
        day: params.day,
        hour: params.hour,
        minute: params.minute,
        second: params.second,
      },
      this.timezone
    );
  }

  DateWithZoneHelper.fromDatetime = function (datetime, timezone, localTimezone) {
    return new DateWithZoneHelper({
      year: datetime.year(),
      month: datetime.month(),
      day: datetime.date(),
      hour: datetime.hour(),
      minute: datetime.minute(),
      second: datetime.second(),
      timezone: timezone,
      localTimezone: localTimezone,
    });
  };

  DateWithZoneHelper.prototype.isDST = function () {
    return this.datetime.tz(this.localTimezone).isDST();
  };

  DateWithZoneHelper.prototype.unitRepetitionsBetweenDates = function (duration, date) {
    var parts = duration.split(".");
    var count = parts[0];
    var unit = parts[1];
    // get the diff in the specified units with decimals
    var diff = Math.abs(this.datetime.diff(date, unit, true));
    // get integer count of duration in diff, eg: 4 hours diff is 2 for 2.hours duration
    var integer = Math.trunc(diff / count);
    // get fractional to define if we have to add one "duration"
    var fractional = (diff / count) % 1;

    return integer * parseInt(count, 10) + (fractional > 0 ? parseInt(count, 10) : 0);
  };

  DateWithZoneHelper.prototype.add = function (count, unit) {
    return DateWithZoneHelper.fromDatetime(
      this.datetime.clone().add(count, unit),
      this.timezone,
      this.localTimezone
    );
  };

  DateWithZoneHelper.prototype.subtract = function (count, unit) {
    return DateWithZoneHelper.fromDatetime(
      this.datetime.clone().subtract(count, unit),
      this.timezone,
      this.localTimezone
    );
  };

  DateWithZoneHelper.prototype.datetimeWithZone = function (timezone) {
    return this.datetime.clone().tz(timezone);
  };

  DateWithZoneHelper.prototype.format = function (format) {
    if (format) {
      return this.datetime.tz(this.localTimezone).format(format);
    }
    return this.datetime.tz(this.localTimezone).toISOString(true);
  };

  // ---- lib/local-date-builder.js

  var DATETIME_FORMAT = "LLL";
  var DATE_FORMAT = "LL";
  var FULL_DATETIME_FORMAT = "LLLL";
  var TIME_FORMAT = "h:mm A";
  var DAY_OF_THE_WEEK_FORMAT = "dddd";
  var RANGE_SEPARATOR = "→";
  var TIME_ICON = "clock";
  var SHORT_FORMAT_DAYS_PERIOD = 1;

  function LocalDateBuilder(params, localTimezone) {
    params = params || {};
    this.time = params.time;
    this.date = params.date;
    this.recurring = params.recurring;
    this.sameLocalDayAsFrom = params.sameLocalDayAsFrom;
    this.timezones = Array.from(new Set((params.timezones || []).filter(Boolean)));
    this.timezone = params.timezone || "UTC";
    this.calendar = typeof params.calendar === "undefined" ? true : params.calendar;
    this.displayedTimezone = params.displayedTimezone;
    this.format = params.format || (this.time ? DATETIME_FORMAT : DATE_FORMAT);
    this.countdown = params.countdown;
    this.duration = params.duration;
    this.localTimezone = localTimezone;
  }

  LocalDateBuilder.prototype.build = function () {
    var ymd = this.date.split("-").map(function (x) {
      return parseInt(x, 10);
    });
    var year = ymd[0];
    var month = ymd[1];
    var day = ymd[2];
    var hms = (this.time || "").split(":").map(function (x) {
      return x ? parseInt(x, 10) : undefined;
    });
    var hour = hms[0];
    var minute = hms[1];
    var second = hms[2];

    var displayedTimezone;
    if (this.time) {
      displayedTimezone = this.displayedTimezone || this.localTimezone;
    } else {
      displayedTimezone = this.displayedTimezone || this.timezone || this.localTimezone;
    }

    var localDate = new DateWithZoneHelper({
      year: year,
      month: month ? month - 1 : null,
      day: day,
      hour: hour,
      minute: minute,
      second: second,
      timezone: this.timezone,
      localTimezone: this.localTimezone,
    });

    if (this.recurring && moment().isAfter(localDate.datetime)) {
      var type = this.recurring.split(".")[1];

      var repetitionsForType = localDate.unitRepetitionsBetweenDates(
        this.recurring,
        moment.tz(this.localTimezone)
      );

      localDate = localDate.add(repetitionsForType, type);
    }

    var previews = this._generatePreviews(localDate, displayedTimezone);
    var hasTime = hour !== undefined;
    return {
      pastEvent: !this.recurring && moment.tz(this.localTimezone).isAfter(localDate.datetime),
      formatted: this._applyFormatting(localDate, displayedTimezone, hasTime),
      previews: previews,
      textPreview: this._generateTextPreviews(previews),
    };
  };

  LocalDateBuilder.prototype._generateTextPreviews = function (previews) {
    var self = this;
    return previews
      .map(function (preview) {
        var formattedZone = self._zoneWithoutPrefix(preview.timezone);
        return formattedZone + " " + preview.formatted;
      })
      .join(", ");
  };

  LocalDateBuilder.prototype._generatePreviews = function (localDate, displayedTimezone) {
    var self = this;
    var previewedTimezones = [];

    var timezones = this.timezones.filter(function (timezone) {
      return !self._isEqualZones(timezone, self.localTimezone);
    });

    previewedTimezones.push({
      timezone: this._zoneWithoutPrefix(this.localTimezone),
      current: true,
      formatted: this._createDateTimeRange(
        DateWithZoneHelper.fromDatetime(localDate.datetime, localDate.timezone, this.localTimezone),
        this.time,
        this.duration
      ),
    });

    if (
      this.timezone &&
      displayedTimezone === this.localTimezone &&
      this.timezone !== displayedTimezone &&
      !this._isEqualZones(displayedTimezone, this.timezone) &&
      !this.timezones.some(function (t) {
        return self._isEqualZones(t, self.timezone);
      })
    ) {
      timezones.unshift(this.timezone);
    }

    timezones.forEach(function (timezone) {
      if (self._isEqualZones(timezone, displayedTimezone)) {
        return;
      }

      if (self._isEqualZones(timezone, self.localTimezone)) {
        timezone = self.localTimezone;
      }

      previewedTimezones.push({
        timezone: self._zoneWithoutPrefix(timezone),
        formatted: self._createDateTimeRange(
          DateWithZoneHelper.fromDatetime(localDate.datetime, localDate.timezone, timezone),
          self.time,
          self.duration
        ),
      });
    });

    // uniqueItemsFromArray(previewedTimezones, "timezone"): the first of each.
    var seen = new Set();
    return previewedTimezones.filter(function (preview) {
      if (seen.has(preview.timezone)) {
        return false;
      }
      seen.add(preview.timezone);
      return true;
    });
  };

  LocalDateBuilder.prototype._isEqualZones = function (timezoneA, timezoneB) {
    if ((timezoneA || timezoneB) && (!timezoneA || !timezoneB)) {
      return false;
    }

    if (timezoneA.includes(timezoneB) || timezoneB.includes(timezoneA)) {
      return true;
    }

    return moment.tz(timezoneA).utcOffset() === moment.tz(timezoneB).utcOffset();
  };

  LocalDateBuilder.prototype._createDateTimeRange = function (startRange, time, duration) {
    var dates = this._calculateDatesForRange(startRange, time, duration);
    var startDate = dates[0];
    var endDate = dates[1];
    var formatElements = [
      startDate.format(DAY_OF_THE_WEEK_FORMAT + ", " + DATE_FORMAT),
      this._optionalTimeIcon(startDate, endDate),
      startDate.format(TIME_FORMAT),
    ];
    if (endDate) {
      formatElements = formatElements.concat([
        RANGE_SEPARATOR,
        endDate.format(this._endDateFormat(startDate, endDate)),
      ]);
    }
    return formatElements.filter(Boolean).join(" ");
  };

  LocalDateBuilder.prototype._shortFormat = function (startDate, endDate) {
    return endDate.datetime.diff(startDate.datetime, "days") < SHORT_FORMAT_DAYS_PERIOD;
  };

  LocalDateBuilder.prototype._optionalTimeIcon = function (startDate, endDate) {
    if (!endDate || this._shortFormat(startDate, endDate)) {
      return "<br />" + iconHTML(TIME_ICON);
    }
  };

  LocalDateBuilder.prototype._endDateFormat = function (startDate, endDate) {
    return this._shortFormat(startDate, endDate) ? TIME_FORMAT : FULL_DATETIME_FORMAT;
  };

  LocalDateBuilder.prototype._calculateDatesForRange = function (date, time, duration) {
    // if a time has been given we do not attempt to automatically create a range
    // instead we show only one date with a format showing the time
    if (time && !duration) {
      return [date];
    }
    var dates = [date, duration ? date.add(duration, "minutes") : date.add(24, "hours")];
    return duration < 0 ? dates.reverse() : dates;
  };

  LocalDateBuilder.prototype._applyFormatting = function (localDate, displayedTimezone, hasTime) {
    if (this.countdown) {
      var diffTime = moment.tz(this.localTimezone).diff(localDate.datetime);

      if (diffTime < 0) {
        return moment.duration(diffTime).humanize();
      } else {
        return i18n("discourse_local_dates.relative_dates.countdown.passed");
      }
    }

    var sameTimezone = this._isEqualZones(displayedTimezone, this.localTimezone);

    if (this.calendar) {
      var inCalendarRange = moment
        .tz(this.localTimezone)
        .isBetween(
          localDate.subtract(2, "day").datetime,
          localDate.add(1, "day").datetime.endOf("day")
        );

      if (this.sameLocalDayAsFrom) {
        return this._timeOnlyFormat(localDate, displayedTimezone);
      }

      if (inCalendarRange && sameTimezone) {
        var date = localDate.datetimeWithZone(this.localTimezone);

        if (hasTime && date.hours() === 0 && date.minutes() === 0) {
          return date.format("dddd");
        }

        return date.calendar(
          moment.tz(localDate.timezone),
          this._calendarFormats(this.time ? this.time : null)
        );
      }
    }

    if (!sameTimezone) {
      return this._formatWithZone(localDate, displayedTimezone, this.format);
    }

    return localDate.format(this.format);
  };

  LocalDateBuilder.prototype._calendarFormats = function (time) {
    return {
      sameDay: this._translateCalendarKey(time, "today"),
      nextDay: this._translateCalendarKey(time, "tomorrow"),
      lastDay: this._translateCalendarKey(time, "yesterday"),
      sameElse: "L",
    };
  };

  LocalDateBuilder.prototype._translateCalendarKey = function (time, key) {
    var translated = i18n("discourse_local_dates.relative_dates." + key, { time: "LT" });

    if (time) {
      return translated
        .split("LT")
        .map(function (w) {
          return "[" + w + "]";
        })
        .join("LT");
    } else {
      return "[" + translated.replace(" LT", "") + "]";
    }
  };

  LocalDateBuilder.prototype._formatTimezone = function (timezone) {
    return timezone.replace("_", " ").replace("Etc/", "").split("/");
  };

  LocalDateBuilder.prototype._zoneWithoutPrefix = function (timezone) {
    var parts = this._formatTimezone(timezone);
    return parts[1] || parts[0];
  };

  LocalDateBuilder.prototype._formatWithZone = function (localDate, displayedTimezone, format) {
    var formatted = localDate.datetimeWithZone(displayedTimezone).format(format);
    return formatted + " (" + this._zoneWithoutPrefix(displayedTimezone) + ")";
  };

  LocalDateBuilder.prototype._timeOnlyFormat = function (localTime, displayedTimezone) {
    return this._formatWithZone(localTime, displayedTimezone, "LT");
  };

  // ---- initializers/discourse-local-dates.js

  function applyLocalDates(dates, timezone) {
    var currentUserTZ = timezone || moment.tz.guess();

    dates.forEach(function (element, index, arr) {
      var opts = buildOptionsFromElement(element);

      if (
        element.getAttribute("data-range") === "to" &&
        index !== 0 &&
        arr[index - 1].getAttribute("data-range") === "from"
      ) {
        var fromElement = arr[index - 1];
        if (rangeIsSameLocalDay(fromElement, element)) {
          opts.sameLocalDayAsFrom = true;
        }
      }

      var localDateBuilder = new LocalDateBuilder(opts, currentUserTZ).build();

      element.innerText = "";
      element.insertAdjacentHTML("beforeend", iconHTML("earth-americas") + "\n        ");
      var relativeTime = document.createElement("span");
      relativeTime.classList.add("relative-time");
      relativeTime.textContent = localDateBuilder.formatted;
      element.appendChild(relativeTime);
      element.setAttribute("aria-label", localDateBuilder.textPreview);

      var classes = ["cooked-date"];
      if (localDateBuilder.pastEvent) {
        classes.push("past");
      }
      element.classList.add.apply(element.classList, classes);
    });
  }

  function rangeIsSameLocalDay(fromElement, toElement) {
    if (!fromElement.hasAttribute("data-time") || !toElement.hasAttribute("data-time")) {
      return false;
    }
    var timezone = fromElement.getAttribute("data-timezone");
    var from = moment(getDateFromElement(fromElement)).tz(timezone);
    var to = moment(getDateFromElement(toElement)).tz(timezone);
    return from.isSame(to, "day");
  }

  function getDateFromElement(element) {
    return element.getAttribute("data-date") + "T" + element.getAttribute("data-time");
  }

  function buildOptionsFromElement(element) {
    var opts = {};
    var dataset = element.dataset;

    if (rangeElements(element).length === 2) {
      opts.duration = calculateDuration(element);
    }

    opts.time = dataset.time;
    opts.date = dataset.date;
    opts.recurring = dataset.recurring;
    opts.timezones = (dataset.timezones || config.defaultTimezones || "Etc/UTC")
      .split("|")
      .filter(Boolean);
    opts.timezone = dataset.timezone;
    opts.displayedTimezone = dataset.displayedTimezone;
    opts.format = dataset.format || (opts.time ? "LLL" : "LL");
    opts.countdown = dataset.countdown;
    opts.calendar = dataset.calendar ? dataset.calendar === "on" : !dataset.format;
    return opts;
  }

  function rangeElements(element) {
    if (!element.parentElement) {
      return [];
    }

    if (element.dataset.range) {
      return partitionedRanges(element).find(function (pair) {
        return pair.includes(element);
      });
    }

    return [element];
  }

  function partitionedRanges(element) {
    var partitions = [];
    var ranges = Array.from(element.parentElement.children).filter(function (span) {
      return span.dataset.range;
    });

    while (ranges.length > 0) {
      partitions.push(ranges.splice(0, 2));
    }

    return partitions;
  }

  function buildHtmlPreview(element) {
    var opts = buildOptionsFromElement(element);
    var localDateBuilder = new LocalDateBuilder(opts, moment.tz.guess()).build();

    var htmlPreviews = localDateBuilder.previews.map(function (preview) {
      var previewNode = document.createElement("div");
      previewNode.dataset.timezone = dasherize(preview.timezone);
      previewNode.classList.add("preview");
      if (preview.current) {
        previewNode.classList.add("current");
      }

      var timezoneNode = document.createElement("span");
      timezoneNode.classList.add("timezone");
      timezoneNode.innerText = preview.timezone;
      previewNode.appendChild(timezoneNode);

      var dateTimeNode = document.createElement("span");
      dateTimeNode.classList.add("date-time");
      dateTimeNode.innerHTML = preview.formatted;
      previewNode.appendChild(dateTimeNode);

      return previewNode;
    });

    var previewsNode = document.createElement("div");
    previewsNode.classList.add("locale-dates-previews");
    htmlPreviews.forEach(function (htmlPreview) {
      previewsNode.appendChild(htmlPreview);
    });

    var calendarNode = downloadCalendarNode(element);
    if (calendarNode) {
      previewsNode.appendChild(calendarNode);
    }

    return previewsNode.outerHTML;
  }

  function calculateStartAndEndDate(startDataset, endDataset) {
    var startDate, endDate;
    startDate = moment.tz(
      (startDataset.date + " " + (startDataset.time || "")).trim(),
      startDataset.timezone
    );
    if (endDataset) {
      endDate = moment.tz(
        (endDataset.date + " " + (endDataset.time || "")).trim(),
        endDataset.timezone
      );
    }
    return [startDate, endDate];
  }

  function downloadCalendarNode(element) {
    var datasets = rangeElements(element).map(function (dateElement) {
      return dateElement.dataset;
    });
    var startDataset = datasets[0];
    var endDataset = datasets[1];
    var dates = calculateStartAndEndDate(startDataset, endDataset);
    var startDate = dates[0];
    var endDate = dates[1];

    if (startDate < moment().tz(startDataset.timezone)) {
      return false;
    }

    var node = document.createElement("div");
    node.classList.add("download-calendar");
    node.innerHTML = iconHTML("file") + " " + i18n("download_calendar.add_to_calendar");
    node.setAttribute("data-starts-at", startDate.toISOString());
    if (endDataset) {
      node.setAttribute("data-ends-at", endDate.toISOString());
    }
    if (!startDataset.time && !endDataset) {
      node.setAttribute("data-ends-at", startDate.add(24, "hours").toISOString());
    }
    if (startDataset.title) {
      node.setAttribute("data-title", startDataset.title);
    }
    if (startDataset.timezone) {
      node.setAttribute("data-timezone", startDataset.timezone);
    }

    // If ics data is available, pass it to the download button
    if (startDataset.ics) {
      node.setAttribute("data-ics", startDataset.ics);
    }

    return node;
  }

  function calculateDuration(element) {
    var datasets = rangeElements(element).map(function (dateElement) {
      return dateElement.dataset;
    });
    var startDataset = datasets[0];
    var endDataset = datasets[1];
    var startDateTime = moment((startDataset.date + " " + (startDataset.time || "")).trim());
    var endDateTime = moment((endDataset.date + " " + (endDataset.time || "")).trim());
    var duration = endDateTime.diff(startDateTime, "minutes");

    // negative duration is used when we calculate difference for end date from range
    return element.dataset === startDataset ? duration : -duration;
  }

  // ---- lib/download-calendar.js

  function downloadCalendar(title, dates, options) {
    options = options || {};
    var formattedDates = formatDates(dates);
    title = (title || i18n("download_calendar.default_title")).trim();

    var defaultCalendar = config.defaultCalendar;

    if (["ics", "google", "outlook", "apple"].indexOf(defaultCalendar) !== -1) {
      addToCalendar(defaultCalendar, title, formattedDates, options);
    } else {
      displayModal(title, formattedDates, options);
    }
  }

  function addToCalendar(calendar, title, dates, options) {
    options = options || {};
    switch (calendar) {
      case "google":
        downloadGoogle(title, dates, options);
        break;
      case "outlook":
        downloadOutlook(title, dates, options);
        break;
      case "apple":
      case "ics":
        downloadIcs(title, dates, options);
        break;
    }
  }

  function downloadIcs(title, dates, options) {
    var REMOVE_FILE_AFTER = 20000;
    var file = new File([generateIcsData(title, dates, options || {})], {
      type: "text/plain",
    });

    var a = document.createElement("a");
    document.body.appendChild(a);
    a.style = "display: none";
    a.href = window.URL.createObjectURL(file);
    a.download = title.toLowerCase().replace(/[^\w]/g, "-") + ".ics";
    a.click();
    setTimeout(function () {
      window.URL.revokeObjectURL(file);
    }, REMOVE_FILE_AFTER);
  }

  // getURL: a path under the site's base path; anything else as it is.
  function getURL(url) {
    var base = (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
    if (!url || url.indexOf("/") !== 0 || url.indexOf("//") === 0) {
      return url;
    }
    if (base && url.indexOf(base + "/") === 0) {
      return url;
    }
    return base + url;
  }

  function downloadGoogle(title, dates, options) {
    var parsedRrule = parseRRule(options.rrule);

    dates.forEach(function (date) {
      var link = new URL("https://www.google.com/calendar/event");
      link.searchParams.append("action", "TEMPLATE");
      link.searchParams.append("text", title);

      var dateRange;
      if (date.allDay) {
        var allDay = allDayMoments(date);
        dateRange = allDay.startDate.format("YYYYMMDD") + "/" + allDay.endDate.format("YYYYMMDD");
      } else {
        dateRange =
          formatDateForGoogleApi(date.startsAt, date.timezone) +
          "/" +
          formatDateForGoogleApi(date.endsAt, date.timezone);
      }
      link.searchParams.append("dates", dateRange);

      if (parsedRrule && hasFreq(parsedRrule)) {
        var rrule = date.allDay ? dateOnlyUntil(parsedRrule) : parsedRrule;
        link.searchParams.append("recur", "RRULE:" + rrule);
      }

      if (options.location) {
        link.searchParams.append("location", options.location);
      }

      if (options.details) {
        link.searchParams.append("details", options.details);
      }

      window.open(getURL(link.href).trim(), "_blank", "noopener", "noreferrer");
    });
  }

  function downloadOutlook(title, dates, options) {
    var parsedRrule = parseRRule(options.rrule);

    if (parsedRrule && hasFreq(parsedRrule)) {
      downloadIcs(title, dates, options);
      return;
    }

    dates.forEach(function (date) {
      var link = new URL("https://outlook.live.com/calendar/0/deeplink/compose");
      link.searchParams.append("path", "/calendar/action/compose");
      link.searchParams.append("rru", "addevent");
      link.searchParams.append("subject", title);

      if (date.allDay) {
        var allDay = allDayMoments(date);
        link.searchParams.append("startdt", allDay.startDate.format("YYYY-MM-DD"));
        link.searchParams.append("enddt", allDay.endDate.format("YYYY-MM-DD"));
        link.searchParams.append("allday", "true");
      } else {
        link.searchParams.append("startdt", formatDateForOutlookApi(date.startsAt, date.timezone));
        link.searchParams.append("enddt", formatDateForOutlookApi(date.endsAt, date.timezone));
        link.searchParams.append("allday", "false");
      }

      if (options.location) {
        link.searchParams.append("location", options.location);
      }

      if (options.details) {
        link.searchParams.append("body", options.details);
      }

      window.open(getURL(link.href).trim(), "_blank", "noopener", "noreferrer");
    });
  }

  function formatDates(dates) {
    return dates.map(function (date) {
      var formatted = {
        startsAt: date.startsAt,
        endsAt: date.endsAt
          ? date.endsAt
          : date.allDay
            ? null
            : moment.utc(date.startsAt).add(1, "hours").format(),
      };

      // Preserve timezone if present
      if (date.timezone) {
        formatted.timezone = date.timezone;
      }

      if (date.allDay) {
        formatted.allDay = true;
      }

      return formatted;
    });
  }

  // Escape special characters in ICS field values per RFC 5545
  function escapeIcsValue(value) {
    return value
      .replace(/\\/g, "\\\\")
      .replace(/\r\n|\r|\n/g, "\\n")
      .replace(/;/g, "\\;")
      .replace(/,/g, "\\,");
  }

  // Fold a line to comply with RFC 5545 line length limit (75 octets)
  function foldLine(line) {
    var maxLength = 75;
    if (line.length <= maxLength) {
      return line;
    }

    var result = [];
    var currentLine = line;

    while (currentLine.length > maxLength) {
      result.push(currentLine.substring(0, maxLength));
      currentLine = " " + currentLine.substring(maxLength);
    }
    result.push(currentLine);

    return result.join("\r\n");
  }

  // The RRULE of a string that may hold DTSTART and RRULE (legacy format).
  function parseRRule(rruleString) {
    if (!rruleString) {
      return null;
    }

    var lines = rruleString.split(/\r?\n/);

    for (var i = 0; i < lines.length; i++) {
      var trimmed = lines[i].trim();
      if (trimmed.startsWith("RRULE:")) {
        return trimmed.substring(6);
      } else if (/^FREQ=/i.test(trimmed)) {
        return trimmed;
      }
    }

    if (/^FREQ=/i.test(rruleString.trim())) {
      return rruleString.trim();
    }

    return null;
  }

  function hasFreq(rrule) {
    return /FREQ=/i.test(rrule);
  }

  function dateOnlyUntil(rrule) {
    return rrule.replace(/(UNTIL=\d{8})T\d{6}Z?/i, "$1");
  }

  function allDayMoments(date) {
    var startDate = moment(date.startsAt, "YYYY-MM-DD");
    var endDate = (date.endsAt ? moment(date.endsAt, "YYYY-MM-DD") : startDate.clone()).add(1, "day");
    return { startDate: startDate, endDate: endDate };
  }

  var ICAL_WEEKDAYS = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];

  function formatIcsUtcOffset(offsetMinutes) {
    var totalSeconds = Math.round(offsetMinutes * 60);
    var sign = totalSeconds < 0 ? "-" : "+";
    var absoluteSeconds = Math.abs(totalSeconds);
    var hours = Math.floor(absoluteSeconds / 3600);
    var minutes = Math.floor((absoluteSeconds % 3600) / 60);
    var seconds = absoluteSeconds % 60;

    return (
      sign +
      String(hours).padStart(2, "0") +
      String(minutes).padStart(2, "0") +
      (seconds ? String(seconds).padStart(2, "0") : "")
    );
  }

  function vTimezoneTransition(zone, index) {
    var transitionAt = zone.untils[index];
    var offsetFrom = -zone.offsets[index];
    var offsetTo = -zone.offsets[index + 1];
    var localStart = moment.utc(transitionAt).utcOffset(offsetFrom);
    var day = localStart.date();
    var ordinal = day + 7 > localStart.daysInMonth() ? -1 : Math.ceil(day / 7);

    return {
      localStart: localStart.format("YYYYMMDDTHHmmss"),
      month: localStart.month() + 1,
      byDay: ordinal + ICAL_WEEKDAYS[localStart.day()],
      offsetFrom: offsetFrom,
      offsetTo: offsetTo,
      name: zone.abbrs[index + 1],
    };
  }

  function sameVTimezoneRule(a, b) {
    return (
      a.month === b.month &&
      a.byDay === b.byDay &&
      a.localStart.slice(8) === b.localStart.slice(8) &&
      a.offsetFrom === b.offsetFrom &&
      a.offsetTo === b.offsetTo
    );
  }

  function renderVTimezoneObservance(observance) {
    var type = observance.offsetTo > observance.offsetFrom ? "DAYLIGHT" : "STANDARD";
    var lines = [
      "BEGIN:" + type,
      "DTSTART:" + observance.localStart,
      "RRULE:FREQ=YEARLY;BYMONTH=" + observance.month + ";BYDAY=" + observance.byDay,
      "TZOFFSETFROM:" + formatIcsUtcOffset(observance.offsetFrom),
      "TZOFFSETTO:" + formatIcsUtcOffset(observance.offsetTo),
    ];

    if (observance.name) {
      lines.push("TZNAME:" + escapeIcsValue(observance.name));
    }

    lines.push("END:" + type);

    return lines.map(foldLine).join("\r\n") + "\r\n";
  }

  function fixedVTimezone(timezone, referenceMs) {
    var reference = moment.tz(referenceMs, timezone);
    var offset = reference.utcOffset();
    var start = reference.clone().subtract(1, "year").startOf("year").format("YYYYMMDDTHHmmss");

    return (
      "BEGIN:VTIMEZONE\r\n" +
      foldLine("TZID:" + timezone) +
      "\r\n" +
      "BEGIN:STANDARD\r\n" +
      "DTSTART:" + start + "\r\n" +
      "TZOFFSETFROM:" + formatIcsUtcOffset(offset) + "\r\n" +
      "TZOFFSETTO:" + formatIcsUtcOffset(offset) + "\r\n" +
      foldLine("TZNAME:" + escapeIcsValue(reference.zoneAbbr())) +
      "\r\n" +
      "END:STANDARD\r\n" +
      "END:VTIMEZONE\r\n"
    );
  }

  function generateVTimezone(timezone, referenceMs) {
    var zone = moment.tz.zone(timezone);

    if (!zone) {
      return "";
    }

    var previousIndex = -1;
    var nextIndex = -1;

    for (var i = 0; i < zone.untils.length - 1; i++) {
      var transitionAt = zone.untils[i];

      if (!Number.isFinite(transitionAt)) {
        break;
      }

      if (transitionAt <= referenceMs) {
        previousIndex = i;
      } else {
        nextIndex = i;
        break;
      }
    }

    if (previousIndex === -1 || nextIndex === -1) {
      return fixedVTimezone(timezone, referenceMs);
    }

    var previous = vTimezoneTransition(zone, previousIndex);
    var next = vTimezoneTransition(zone, nextIndex);
    var followingPreviousIndex = previousIndex + 2;
    var followingNextIndex = nextIndex + 2;

    // A current seasonal pair reverses the same two UTC offsets and repeats
    // the same calendar rules in the following cycle. Otherwise, use the
    // current fixed offset rather than inventing an open-ended yearly rule.
    if (
      previous.offsetFrom !== next.offsetTo ||
      previous.offsetTo !== next.offsetFrom ||
      followingNextIndex >= zone.untils.length - 1
    ) {
      return fixedVTimezone(timezone, referenceMs);
    }

    var followingPrevious = vTimezoneTransition(zone, followingPreviousIndex);
    var followingNext = vTimezoneTransition(zone, followingNextIndex);

    if (!sameVTimezoneRule(previous, followingPrevious) || !sameVTimezoneRule(next, followingNext)) {
      return fixedVTimezone(timezone, referenceMs);
    }

    return (
      "BEGIN:VTIMEZONE\r\n" +
      foldLine("TZID:" + timezone) +
      "\r\n" +
      renderVTimezoneObservance(previous) +
      renderVTimezoneObservance(next) +
      "END:VTIMEZONE\r\n"
    );
  }

  function generateIcsData(title, dates, options) {
    options = options || {};
    var data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Discourse//EN\r\n";
    var parsedRrule = parseRRule(options.rrule);

    var timezoneStarts = new Map();

    dates.forEach(function (date) {
      if (date.allDay) {
        return;
      }

      var timezone = date.timezone || options.timezone;

      if (!timezone || !moment.tz.zone(timezone)) {
        return;
      }

      var startsAt = moment.tz(date.startsAt, timezone);

      if (!startsAt.isValid()) {
        return;
      }

      var existing = timezoneStarts.get(timezone);

      if (existing === undefined || startsAt.valueOf() < existing) {
        timezoneStarts.set(timezone, startsAt.valueOf());
      }
    });

    timezoneStarts.forEach(function (startsAt, timezone) {
      data += generateVTimezone(timezone, startsAt);
    });

    dates.forEach(function (date) {
      var rrule = parsedRrule;

      var startDate, endDate, dtStartValue, dtEndValue;

      if (date.allDay) {
        if (rrule) {
          rrule = dateOnlyUntil(rrule);
        }
        var allDay = allDayMoments(date);
        startDate = allDay.startDate;
        endDate = allDay.endDate;
        dtStartValue = "DTSTART;VALUE=DATE:" + startDate.format("YYYYMMDD");
        dtEndValue = "DTEND;VALUE=DATE:" + endDate.format("YYYYMMDD");
      } else {
        var timezone = date.timezone || options.timezone;
        startDate = timezone ? moment.tz(date.startsAt, timezone) : moment(date.startsAt);
        endDate = timezone ? moment.tz(date.endsAt, timezone) : moment(date.endsAt);

        dtStartValue = timezone
          ? "DTSTART;TZID=" + timezone + ":" + startDate.format("YYYYMMDDTHHmmss")
          : "DTSTART:" + startDate.utc().format("YYYYMMDDTHHmmss") + "Z";
        dtEndValue = timezone
          ? "DTEND;TZID=" + timezone + ":" + endDate.format("YYYYMMDDTHHmmss")
          : "DTEND:" + endDate.utc().format("YYYYMMDDTHHmmss") + "Z";
      }

      data = data.concat(
        "BEGIN:VEVENT\r\n" +
          foldLine("UID:" + startDate.valueOf() + "_" + endDate.valueOf()) +
          "\r\n" +
          foldLine("DTSTAMP:" + moment().utc().format("YYYYMMDDTHHmmss") + "Z") +
          "\r\n" +
          foldLine(dtStartValue) +
          "\r\n" +
          foldLine(dtEndValue) +
          "\r\n" +
          (rrule && hasFreq(rrule) ? foldLine("RRULE:" + rrule) + "\r\n" : "") +
          (options.location ? foldLine("LOCATION:" + escapeIcsValue(options.location)) + "\r\n" : "") +
          (options.details ? foldLine("DESCRIPTION:" + escapeIcsValue(options.details)) + "\r\n" : "") +
          foldLine("SUMMARY:" + escapeIcsValue(title)) +
          "\r\n" +
          "END:VEVENT\r\n"
      );
    });
    data = data.concat("END:VCALENDAR");
    return data;
  }

  function formatDateForGoogleApi(date, timezone) {
    var momentDate = timezone ? moment.tz(date, timezone) : moment(date);
    return momentDate.utc().format("YYYYMMDD[T]HHmmss[Z]");
  }

  function formatDateForOutlookApi(date, timezone) {
    var momentDate = timezone ? moment.tz(date, timezone) : moment(date);
    return momentDate.toISOString();
  }

  // ---- components/modal/download-calendar.gjs, its form-kit form.

  var guid = 0;
  function uuid() {
    return window.crypto && crypto.randomUUID ? crypto.randomUUID() : "fk-" + ++guid;
  }

  function radioField(value, icon, label) {
    var id = uuid();
    return (
      '<div class="form-kit__field form-kit__field-radio">' +
      '<label for="' + id + '" class="form-kit__control-radio-label">' +
      '<input class="form-kit__control-radio" id="' + id + '" name="calendar" type="radio" value="' + value + '">' +
      '<span class="form-kit__control-radio-content"><span class="form-kit__control-radio-title">' +
      Discourse.cooked.icon(icon) + " " + escapeHtml(label) +
      "</span></span></label></div>"
    );
  }

  function displayModal(title, dates, options) {
    var calendar = {
      title: title,
      dates: dates,
      rrule: options.rrule,
      location: options.location,
      details: options.details,
      timezone: options.timezone,
    };
    var form;
    function submit(close) {
      var choice = form.querySelector('input[name="calendar"]:checked');
      var data = {
        calendar: choice ? choice.value : "",
        remember: !!(form.querySelector('input[name="remember"]') || {}).checked,
      };
      // @validation="required"
      if (!data.calendar) {
        return;
      }
      if (data.remember) {
        config.defaultCalendar = data.calendar;
        saveDefaultCalendar(data.calendar);
      }
      addToCalendar(data.calendar, calendar.title, calendar.dates, {
        rrule: calendar.rrule,
        location: calendar.location,
        details: calendar.details,
        timezone: calendar.timezone,
      });
      close();
    }
    Discourse.modal.show({
      className: "download-calendar-modal",
      title: i18n("download_calendar.title"),
      body: function (body, close) {
        var groupId = uuid();
        var html =
          '<form class="form-kit" novalidate="">' +
          '<div class="form-kit__container form-kit__field form-kit__field-radio-group --full" data-name="calendar" id="control-calendar" data-control-type="radio-group">' +
          '<label for="' + groupId + '" class="form-kit__container-title"><span>' +
          escapeHtml(i18n("download_calendar.calendar")) +
          "</span></label>" +
          '<div class="form-kit__container-content">' +
          '<fieldset class="form-kit__fieldset form-kit__control-radio-group" name="calendar" id="' + groupId + '">' +
          radioField("ics", "download", i18n("download_calendar.save_ics")) +
          radioField("google", "fab-google", i18n("download_calendar.save_google")) +
          radioField("outlook", "fab-microsoft", i18n("download_calendar.save_outlook")) +
          radioField("apple", "fab-apple", i18n("download_calendar.save_apple")) +
          "</fieldset></div></div>";
        if (config.username) {
          var checkId = uuid();
          html +=
            '<div class="form-kit__container form-kit__field form-kit__field-checkbox --full" data-name="remember" id="control-remember" data-control-type="checkbox">' +
            '<div class="form-kit__container-content">' +
            '<label class="form-kit__control-checkbox-label">' +
            '<input class="form-kit__control-checkbox" id="' + checkId + '" name="remember" type="checkbox">' +
            '<span class="form-kit__control-checkbox-checkmark">' + Discourse.cooked.icon("check") + "</span>" +
            '<span class="form-kit__control-checkbox-content">' +
            '<span class="form-kit__control-checkbox-title"><span>' +
            escapeHtml(i18n("download_calendar.remember")) +
            "</span></span>" +
            '<span class="form-kit__control-checkbox-description">' +
            escapeHtml(i18n("download_calendar.remember_explanation")) +
            "</span></span></label></div></div>";
        }
        html += "</form>";
        body.innerHTML = html;
        form = body.querySelector("form");
        // formData { calendar: "ics" }: checked, not as an attribute.
        form.querySelector('input[value="ics"]').checked = true;
        form.addEventListener("submit", function (event) {
          event.preventDefault();
          submit(close);
        });
      },
      footer: function (footer, close) {
        footer.innerHTML =
          '<button class="btn btn-primary" type="button"><span class="d-button-label">' +
          escapeHtml(i18n("download_calendar.add_to_calendar")) +
          "</span></button>" +
          '<button class="btn btn-flat d-modal-cancel" type="button"><span class="d-button-label">' +
          escapeHtml(i18n("cancel")) +
          "</span></button>";
        footer.querySelector(".btn-primary").addEventListener("click", function () {
          submit(close);
        });
        footer.querySelector(".d-modal-cancel").addEventListener("click", function () {
          close();
        });
      },
    });
  }

  // currentUser.save(["default_calendar"])
  function saveDefaultCalendar(calendar) {
    var csrf = document.querySelector('meta[name="csrf-token"]');
    var body = new URLSearchParams();
    body.append("default_calendar", calendar);
    fetch(getURL("/u/" + encodeURIComponent(config.username) + ".json"), {
      method: "PUT",
      headers: {
        "X-CSRF-Token": csrf ? csrf.content : "",
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json",
      },
      body: body,
      credentials: "same-origin",
    }).catch(function (e) {
      window.console && console.error("default_calendar", e);
    });
  }

  function downloadIcsData(title, icsData) {
    var blob = new Blob([icsData], { type: "text/calendar;charset=utf-8" });
    var url = URL.createObjectURL(blob);
    try {
      var a = document.createElement("a");
      a.href = url;
      a.download = slugify(title) + ".ics";
      // Most browsers allow clicking without attaching to DOM.
      a.click();
    } finally {
      // Revoke on next tick so the download can start.
      setTimeout(function () {
        URL.revokeObjectURL(url);
      }, 0);
    }
  }

  // LocalDatesInit#showDatePopover, on every click on the page.
  function showDatePopover(event) {
    var target = event && event.target;
    if (target && target.classList && target.classList.contains("download-calendar")) {
      var dataset = target.dataset;

      if (dataset.ics) {
        var icsData = bbcodeAttributeDecode(dataset.ics);

        var title;
        if (dataset.title) {
          title = dataset.title;
        } else {
          // Extract event title from ICS SUMMARY field for filename if title missing
          var summaryMatch = icsData.match(/SUMMARY:(.+?)[\r\n]/);
          if (summaryMatch && summaryMatch[1]) {
            title = summaryMatch[1].trim();
          }
        }

        downloadIcsData(title || "event", icsData);
      } else {
        var dates = [{ startsAt: dataset.startsAt, endsAt: dataset.endsAt }];

        // Add timezone if available
        if (dataset.timezone) {
          dates[0].timezone = dataset.timezone;
        }

        downloadCalendar(dataset.title, dates);
      }

      return Discourse.float.close("local-date");
    }

    if (!target || !target.classList || !target.classList.contains("discourse-local-date")) {
      return;
    }

    loadMoment().then(function () {
      Discourse.float.showTooltip(target, {
        identifier: "local-date",
        content: buildHtmlPreview(target),
      });
    });
  }

  if (Discourse.once && !Discourse.once("local-dates")) {
    return;
  }
  window.addEventListener("click", showDatePopover, { passive: true });

  // opts.title: also each date's data-title, as the cooked decorator sets
  // it (chat's decorator does not).
  Discourse.applyLocalDates = function (root, opts) {
    var dates = root ? root.querySelectorAll(".discourse-local-date") : [];
    if (!dates.length) {
      return Promise.resolve();
    }
    var title = opts && opts.title ? topicTitle() || config.defaultTitle : null;
    return loadMoment().then(function () {
      applyLocalDates(Array.prototype.slice.call(dates));
      if (title) {
        dates.forEach(function (date) {
          date.dataset.title = date.dataset.title || title;
        });
      }
    });
  };

  // The topic's title (helper.getModel().topic.title) on a topic page,
  // from its <title>, which it writes as "title - site title".
  function topicTitle() {
    if (!document.getElementById("topic")) {
      return null;
    }
    var suffix = " - " + config.siteTitle;
    var title = document.title;
    if (config.siteTitle && title.slice(-suffix.length) === suffix) {
      return title.slice(0, -suffix.length);
    }
    return null;
  }

  Discourse.decorateCooked("local-dates", function (elem) {
    Discourse.applyLocalDates(elem, { title: true });
  });
})();
