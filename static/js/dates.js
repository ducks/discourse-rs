// Dates in the viewer's timezone, as Ember draws them: the pages are
// rendered with UTC epoch milliseconds in their markup, so a cached page
// serves every viewer, and this script writes the text in the browser.
//
// It loads in the head, before the body, and watches the document: every
// date the parser (or an htmx swap, a bus update, a fetched user card)
// inserts is drawn in the microtask after its insertion, before the page
// paints. Three kinds of markup:
//
//   <span class="relative-date" data-time="<ms>" data-format="tiny">
//       Ember's autoUpdatingRelativeAge (discourse/lib/formatter): the text
//       is relativeAge in the format (tiny, medium, medium-with-ago,
//       medium-with-ago-and-on), redrawn each minute as Ember's
//       relative-ages initializer does (updateRelativeAge).
//
//   data-tz-text="..."    the element's text, and data-tz-title,
//   data-tz-aria-label,   data-tz-aria-label, data-tz-value and
//   data-tz-value,        data-tz-datetime the attribute without the
//   data-tz-datetime      prefix: a template whose %{<ms>:<format>}
//       placeholders are each a date in the format, a moment format string
//       (the server resolves the locale's key, dates.long_with_year say)
//       or @tiny, @medium, @medium-with-ago or @medium-with-ago-and-on for
//       relativeAge. The data-tz-* attributes go once drawn, so the
//       element ends as Ember renders it.
//
//   data-tz-zone="<IANA>" on the element: the dates in that zone instead
//       of the browser's (Ember's moment.tz(date, user_option.timezone) for
//       bookmark reminders and the do-not-disturb end).
//
// The strings (the dates.* keys and "now", with the site's overrides) and
// relative_date_duration come with the script tag. The moment formats are
// drawn in English, the port's only locale.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.formatDates) {
    return;
  }

  var script = document.currentScript;
  var strings = {};
  try {
    strings = JSON.parse(script.dataset.strings || "{}");
  } catch (e) {
    window.console && console.error("dates strings", e);
  }
  var duration = parseInt(script.dataset.relativeDateDuration, 10);
  if (isNaN(duration)) {
    duration = 30;
  }

  // discourse-i18n for English: the zero form where the key has one, then
  // one for 1 and other for the rest; %{name} filled from the options.
  function i18n(key, opts) {
    var text = strings[key];
    if (opts && opts.count !== undefined && text === undefined) {
      var n = opts.count;
      text =
        (n === 0 && strings[key + ".zero"]) ||
        strings[key + (n === 1 ? ".one" : ".other")];
    }
    if (text === undefined) {
      return "[en." + key + "]";
    }
    return text.replace(/%\{(\w+)\}/g, function (m, name) {
      return opts && opts[name] !== undefined ? String(opts[name]) : m;
    });
  }

  // ---- moment#format in English, in the browser's zone or a named one.

  var MONTHS = [
    "January", "February", "March", "April", "May", "June", "July",
    "August", "September", "October", "November", "December",
  ];
  var WEEKDAYS = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
  ];
  // moment's English longDateFormat.
  var LONG = {
    LTS: "h:mm:ss A",
    LT: "h:mm A",
    L: "MM/DD/YYYY",
    LL: "MMMM D, YYYY",
    LLL: "MMMM D, YYYY h:mm A",
    LLLL: "dddd, MMMM D, YYYY h:mm A",
  };
  var TOKENS =
    /(\[[^\[]*\])|(\\)?(Mo|MM?M?M?|Do|DDDo|DD?D?D?|ddd?d?|do?|w[o|w]?|W[o|W]?|Qo?|YYYYYY|YYYYY|YYYY|YY|gg(ggg?)?|GG(GGG?)?|e|E|a|A|hh?|HH?|kk?|mm?|ss?|S{1,9}|x|X|zz?|ZZ?|.)/g;
  var LONG_TOKENS = /(\[[^\[]*\])|(\\)?(LTS|LT|LL?L?L?|l{1,4})/g;

  function pad(n, width) {
    var s = String(Math.abs(n));
    while (s.length < width) {
      s = "0" + s;
    }
    return (n < 0 ? "-" : "") + s;
  }

  function ordinal(n) {
    var b = n % 10;
    var suffix =
      Math.floor((n % 100) / 10) === 1 ? "th" : b === 1 ? "st" : b === 2 ? "nd" : b === 3 ? "rd" : "th";
    return n + suffix;
  }

  // moment's expandFormat: the long tokens (and their short forms, which
  // English derives by shortening the month and weekday names).
  function expand(format) {
    for (var i = 0; i < 5 && LONG_TOKENS.test(format); i++) {
      LONG_TOKENS.lastIndex = 0;
      format = format.replace(LONG_TOKENS, function (m, literal, escaped, token) {
        if (literal || escaped) {
          return m;
        }
        if (LONG[token]) {
          return LONG[token];
        }
        var upper = LONG[token.toUpperCase()];
        return upper.replace(/MMMM|MM|DD|dddd/g, function (t) {
          return t.slice(1);
        });
      });
      LONG_TOKENS.lastIndex = 0;
    }
    LONG_TOKENS.lastIndex = 0;
    return format;
  }

  var zoneFormats = {};
  function zoneFormat(zone) {
    if (!zoneFormats[zone]) {
      zoneFormats[zone] = new Intl.DateTimeFormat("en-US", {
        timeZone: zone,
        hourCycle: "h23",
        year: "numeric",
        month: "numeric",
        day: "numeric",
        hour: "numeric",
        minute: "numeric",
        second: "numeric",
        timeZoneName: "short",
      });
    }
    return zoneFormats[zone];
  }

  // The wall clock of a moment in the zone: the browser's, or a named one
  // through Intl (an unknown zone falls back to the browser's, as
  // moment.tz does with a warning).
  function fields(ms, zone) {
    if (zone) {
      try {
        var parts = {};
        zoneFormat(zone)
          .formatToParts(new Date(ms))
          .forEach(function (p) {
            parts[p.type] = p.value;
          });
        var wall = Date.UTC(
          +parts.year, +parts.month - 1, +parts.day,
          +parts.hour % 24, +parts.minute, +parts.second
        );
        var d = new Date(wall);
        return {
          year: d.getUTCFullYear(),
          month: d.getUTCMonth(),
          day: d.getUTCDate(),
          weekday: d.getUTCDay(),
          hour: d.getUTCHours(),
          minute: d.getUTCMinutes(),
          second: d.getUTCSeconds(),
          millis: ((ms % 1000) + 1000) % 1000,
          offset: Math.round((wall - (ms - (((ms % 1000) + 1000) % 1000))) / 60000),
          abbr: parts.timeZoneName || "",
          ms: ms,
        };
      } catch (e) {
        // fall through to the browser's zone
      }
    }
    var l = new Date(ms);
    return {
      year: l.getFullYear(),
      month: l.getMonth(),
      day: l.getDate(),
      weekday: l.getDay(),
      hour: l.getHours(),
      minute: l.getMinutes(),
      second: l.getSeconds(),
      millis: l.getMilliseconds(),
      offset: -l.getTimezoneOffset(),
      abbr: "",
      ms: ms,
    };
  }

  function dayOfYear(f) {
    return Math.round((Date.UTC(f.year, f.month, f.day) - Date.UTC(f.year, 0, 1)) / 864e5) + 1;
  }

  // moment's weekOfYear with the locale's dow and doy (English: 0 and 6;
  // ISO: 1 and 4): [week, year].
  function weekOfYear(f, dow, doy) {
    function firstWeekOffset(year) {
      var fwd = 7 + dow - doy;
      var fwdlw = (7 + new Date(Date.UTC(year, 0, fwd)).getUTCDay() - dow) % 7;
      return -fwdlw + fwd - 1;
    }
    function weeksInYear(year) {
      var leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
      return (365 + (leap ? 1 : 0) - firstWeekOffset(year) + firstWeekOffset(year + 1)) / 7;
    }
    var weekOffset = firstWeekOffset(f.year);
    var week = Math.floor((dayOfYear(f) - weekOffset - 1) / 7) + 1;
    if (week < 1) {
      return [week + weeksInYear(f.year - 1), f.year - 1];
    }
    if (week > weeksInYear(f.year)) {
      return [week - weeksInYear(f.year), f.year + 1];
    }
    return [week, f.year];
  }

  function offset(f, separator) {
    var m = f.offset;
    return (m < 0 ? "-" : "+") + pad(Math.floor(Math.abs(m) / 60), 2) + separator + pad(Math.abs(m) % 60, 2);
  }

  function token(f, t) {
    var h12 = f.hour % 12 || 12;
    var weekday = f.weekday;
    switch (t) {
      case "YYYYYY": return (f.year < 0 ? "-" : "+") + pad(Math.abs(f.year), 6);
      case "YYYYY": return pad(f.year, 5);
      case "YYYY": return f.year <= 9999 ? pad(f.year, 4) : "+" + f.year;
      case "YY": return pad(f.year % 100, 2);
      case "MMMM": return MONTHS[f.month];
      case "MMM": return MONTHS[f.month].slice(0, 3);
      case "MM": return pad(f.month + 1, 2);
      case "Mo": return ordinal(f.month + 1);
      case "M": return String(f.month + 1);
      case "DDDD": return pad(dayOfYear(f), 3);
      case "DDDo": return ordinal(dayOfYear(f));
      case "DDD": return String(dayOfYear(f));
      case "DD": return pad(f.day, 2);
      case "Do": return ordinal(f.day);
      case "D": return String(f.day);
      case "dddd": return WEEKDAYS[weekday];
      case "ddd": return WEEKDAYS[weekday].slice(0, 3);
      case "dd": return WEEKDAYS[weekday].slice(0, 2);
      case "do": return ordinal(weekday);
      case "d": case "e": return String(weekday);
      case "E": return String(weekday || 7);
      case "ww": return pad(weekOfYear(f, 0, 6)[0], 2);
      case "wo": return ordinal(weekOfYear(f, 0, 6)[0]);
      case "w": return String(weekOfYear(f, 0, 6)[0]);
      case "WW": return pad(weekOfYear(f, 1, 4)[0], 2);
      case "Wo": return ordinal(weekOfYear(f, 1, 4)[0]);
      case "W": return String(weekOfYear(f, 1, 4)[0]);
      case "gg": return pad(weekOfYear(f, 0, 6)[1] % 100, 2);
      case "gggg": return pad(weekOfYear(f, 0, 6)[1], 4);
      case "ggggg": return pad(weekOfYear(f, 0, 6)[1], 5);
      case "GG": return pad(weekOfYear(f, 1, 4)[1] % 100, 2);
      case "GGGG": return pad(weekOfYear(f, 1, 4)[1], 4);
      case "GGGGG": return pad(weekOfYear(f, 1, 4)[1], 5);
      case "Qo": return ordinal(Math.floor(f.month / 3) + 1);
      case "Q": return String(Math.floor(f.month / 3) + 1);
      case "a": return f.hour < 12 ? "am" : "pm";
      case "A": return f.hour < 12 ? "AM" : "PM";
      case "hh": return pad(h12, 2);
      case "h": return String(h12);
      case "HH": return pad(f.hour, 2);
      case "H": return String(f.hour);
      case "kk": return pad(f.hour || 24, 2);
      case "k": return String(f.hour || 24);
      case "mm": return pad(f.minute, 2);
      case "m": return String(f.minute);
      case "ss": return pad(f.second, 2);
      case "s": return String(f.second);
      case "x": return String(f.ms);
      case "X": return String(Math.floor(f.ms / 1000));
      case "z": case "zz": return f.abbr;
      case "ZZ": return offset(f, "");
      case "Z": return offset(f, ":");
    }
    if (/^S+$/.test(t)) {
      var digits = pad(f.millis, 3);
      return t.length <= 3 ? digits.slice(0, t.length) : digits + new Array(t.length - 2).join("0");
    }
    return t;
  }

  function format(ms, pattern, zone) {
    var f = fields(ms, zone);
    TOKENS.lastIndex = 0;
    return expand(pattern).replace(TOKENS, function (m, literal, escaped, t) {
      if (literal) {
        return literal.slice(1, -1);
      }
      if (escaped) {
        return t;
      }
      return token(f, t);
    });
  }

  // ---- discourse/lib/formatter

  function sameYear(ms, zone) {
    return fields(ms, zone).year === fields(Date.now(), zone).year;
  }

  function shortDate(ms, zone) {
    return format(ms, i18n("dates.medium.date_year"), zone);
  }

  function shortDateNoYear(ms, zone) {
    return format(ms, i18n("dates.tiny.date_month"), zone);
  }

  function tinyDateYear(ms, zone) {
    return format(ms, i18n("dates.tiny.date_year"), zone);
  }

  function smartShortDate(ms, zone, withYear) {
    return sameYear(ms, zone) ? shortDateNoYear(ms, zone) : (withYear || tinyDateYear)(ms, zone);
  }

  function longDate(ms, zone) {
    return format(ms, i18n("dates.long_with_year"), zone);
  }

  // components/topic-timeline/container's timelineDate.
  function timelineDate(ms, zone) {
    var key = sameYear(ms, zone) ? "dates.long_no_year_no_time" : "dates.timeline_date";
    return format(ms, i18n(key), zone);
  }

  // relativeAgeTiny; `timeline` is the timeline's options: addAgo, and
  // timelineDate as the defaultFormat.
  function relativeAgeTiny(ms, zone, timeline) {
    var distance = Math.round(Math.abs(Date.now() - ms) / 1000);
    var minutes = Math.max(1, Math.round(distance / 60.0));
    var t = function (key, count) {
      var result = i18n("dates.tiny." + key, { count: count });
      return timeline ? i18n("dates.wrap_ago", { date: result }) : result;
    };
    if (minutes <= 44) {
      return t("x_minutes", minutes);
    }
    if (minutes <= 89) {
      return t("about_x_hours", 1);
    }
    if (minutes <= 1409) {
      return t("about_x_hours", Math.round(minutes / 60.0));
    }
    if (duration === 0 && minutes <= 525599) {
      return shortDateNoYear(ms, zone);
    }
    if (minutes <= 2519) {
      return t("x_days", 1);
    }
    if (minutes <= (duration || 14) * 1440) {
      return t("x_days", Math.round(minutes / 1440.0));
    }
    return timeline ? timelineDate(ms, zone) : smartShortDate(ms, zone);
  }

  function relativeAgeMediumSpan(distance, leaveAgo) {
    var minutes = Math.round(distance / 60.0);
    var t = function (key, count) {
      return i18n("dates.medium" + (leaveAgo ? "_with_ago" : "") + "." + key, { count: count });
    };
    var formatted;
    if (minutes >= 1 && minutes <= 55) {
      formatted = t("x_minutes", minutes);
    } else if (minutes >= 56 && minutes <= 89) {
      formatted = t("x_hours", 1);
    } else if (minutes >= 90 && minutes <= 1409) {
      formatted = t("x_hours", Math.round(minutes / 60.0));
    } else if (minutes >= 1410 && minutes <= 2519) {
      formatted = t("x_days", 1);
    } else if (minutes >= 2520 && minutes <= 129599) {
      formatted = t("x_days", Math.round(minutes / 1440.0));
    } else if (minutes >= 129600 && minutes <= 525599) {
      formatted = t("x_months", Math.round(minutes / 43200.0));
    } else {
      formatted = t("x_years", Math.round(minutes / 525600.0));
    }
    return formatted || "\u2014";
  }

  // relativeAgeMedium without the span (wrapInSpan: false).
  function relativeAgeMedium(ms, zone, leaveAgo, wrapOn) {
    var distance = Math.round((Date.now() - ms) / 1000);
    if (distance < 60) {
      return i18n("now");
    }
    if (distance > 432000) {
      var date = smartShortDate(ms, zone, shortDate);
      return wrapOn ? i18n("dates.wrap_on", { date: date }) : date;
    }
    return relativeAgeMediumSpan(distance, leaveAgo);
  }

  function relativeAge(ms, fmt, zone) {
    switch (fmt) {
      case "medium":
        return relativeAgeMedium(ms, zone, false, false);
      case "medium-with-ago":
        return relativeAgeMedium(ms, zone, true, false);
      case "medium-with-ago-and-on":
        return relativeAgeMedium(ms, zone, true, true);
      case "tiny":
      case undefined:
      case "":
        return relativeAgeTiny(ms, zone);
    }
    return "UNKNOWN FORMAT";
  }

  // lib/bookmark's formattedReminderTime, capitalized as
  // Bookmark#formattedReminder: in the member's zone where they have one.
  function formattedReminder(ms, zone) {
    var at = fields(ms, zone);
    var now = fields(Date.now(), zone);
    var next = new Date(Date.UTC(now.year, now.month, now.day + 1));
    var time = format(ms, i18n("dates.time"), zone);
    var text;
    if (
      at.year === next.getUTCFullYear() &&
      at.month === next.getUTCMonth() &&
      at.day === next.getUTCDate()
    ) {
      text = i18n("bookmarks.reminders.tomorrow_with_time", { time: time });
    } else if (at.year === now.year && at.month === now.month && at.day === now.day) {
      text = i18n("bookmarks.reminders.today_with_time", { time: time });
    } else {
      text = i18n("bookmarks.reminders.at_time", {
        date_time: format(ms, i18n("dates.long_with_year"), zone),
      });
    }
    return text.charAt(0).toUpperCase() + text.slice(1);
  }

  // A template's @ formats (crate::viewer_dates::named_format):
  //   tiny, medium, medium-with-ago, medium-with-ago-and-on  relativeAge
  //   timeline           timelineDate
  //   timeline-ago       the timeline's last post (addAgo, timelineDate)
  //   medium-span-ago    relativeAgeMediumSpan with ago (no dates)
  //   reminder           Bookmark#formattedReminder
  //   timeline-days-ago  the number is days: timelineDate of today less
  //                      them, as the timeline's scroller counts them
  //   year:A|B           format A in the current year, else B
  function named(ms, name, zone) {
    if (name.indexOf("year:") === 0) {
      var formats = name.slice(5).split("|");
      return format(ms, sameYear(ms, zone) ? formats[0] : formats[1] || formats[0], zone);
    }
    switch (name) {
      case "timeline":
        return timelineDate(ms, zone);
      case "timeline-ago":
        return relativeAgeTiny(ms, zone, true);
      case "reminder":
        return formattedReminder(ms, zone);
      case "medium-span-ago":
        return relativeAgeMediumSpan((Date.now() - ms) / 1000, true);
      case "timeline-days-ago":
        var date = new Date();
        date.setDate(date.getDate() - ms || 0);
        return timelineDate(date.getTime(), zone);
    }
    return relativeAge(ms, name, zone);
  }

  // A template's %{<ms>:<format>} placeholders.
  function fill(template, zone) {
    return template.replace(/%\{(-?\d+):([^}]*)\}/g, function (m, ms, fmt) {
      ms = parseInt(ms, 10);
      return fmt.charAt(0) === "@" ? named(ms, fmt.slice(1), zone) : format(ms, fmt, zone);
    });
  }

  // ---- the page

  var ATTRS = ["title", "aria-label", "value", "datetime"];
  var SELECTOR =
    ".relative-date[data-time], [data-tz-text], " +
    ATTRS.map(function (a) {
      return "[data-tz-" + a + "]";
    }).join(", ");
  // What each drawn element should read, kept so text the parser appends
  // after the element went in (a chunk boundary inside it) is put right.
  var drawnText = new WeakMap();

  // The element's text; one holding elements (an icon) keeps them, its
  // text drawn after them. Written only when it differs, so drawing again
  // on our own mutation settles.
  function setText(el, text) {
    drawnText.set(el, text);
    if (!el.firstElementChild) {
      if (el.textContent !== text) {
        el.textContent = text;
      }
      return;
    }
    var own = Array.prototype.filter.call(el.childNodes, function (n) {
      return n.nodeType === 3;
    });
    if (own.length === 1 && own[0].data === text && el.lastChild === own[0]) {
      return;
    }
    own.forEach(function (n) {
      n.remove();
    });
    el.appendChild(document.createTextNode(text));
  }

  function draw(el) {
    var zone = el.getAttribute("data-tz-zone") || drawnZone.get(el) || "";
    if (zone) {
      drawnZone.set(el, zone);
      el.removeAttribute("data-tz-zone");
    }
    var text = el.getAttribute("data-tz-text");
    if (text !== null) {
      el.removeAttribute("data-tz-text");
      drawnTemplate.set(el, text);
    }
    // A template, where there is one, is the text (the timeline's
    // relative-date draws its first age with options data-format lacks).
    if (drawnTemplate.has(el)) {
      setText(el, fill(drawnTemplate.get(el), zone));
    } else if (el.classList.contains("relative-date") && el.hasAttribute("data-time")) {
      var ms = parseInt(el.getAttribute("data-time"), 10);
      if (!isNaN(ms)) {
        // The text sits in the span, or in its .date span where the
        // server draws one (a post's date).
        var holder =
          el.children.length === 1 && el.firstElementChild.classList.contains("date")
            ? el.firstElementChild
            : el;
        var prefix = el.getAttribute("data-tz-prefix");
        if (prefix !== null) {
          drawnPrefix.set(el, prefix);
          el.removeAttribute("data-tz-prefix");
        } else {
          prefix = drawnPrefix.get(el) || "";
        }
        var age = relativeAge(ms, el.getAttribute("data-format"), zone);
        setText(holder, prefix + age);
        // relativeAgeTinyShowsYear
        if (el.getAttribute("data-format") === "tiny") {
          el.classList.toggle("with-year", /'[\d]{2}$/.test(age));
        }
      }
    }
    ATTRS.forEach(function (name) {
      var template = el.getAttribute("data-tz-" + name);
      if (template === null) {
        return;
      }
      el.removeAttribute("data-tz-" + name);
      var value = fill(template, zone);
      el.setAttribute(name, value);
      if (name === "value" && "value" in el) {
        el.value = value;
      }
    });
  }
  var drawnZone = new WeakMap();
  var drawnPrefix = new WeakMap();
  var drawnTemplate = new WeakMap();

  // Every date under root (root included).
  function formatDates(root) {
    root = root || document;
    if (root.nodeType === 1 && root.matches(SELECTOR)) {
      draw(root);
    }
    if (root.querySelectorAll) {
      Array.prototype.forEach.call(root.querySelectorAll(SELECTOR), draw);
    }
  }

  // Text the parser appended after an element was drawn: drawn again.
  // Once the document is parsed, changes are the page scripts' own (the
  // timeline's scroller label) and stay.
  function redrawHolder(node) {
    if (document.readyState !== "loading") {
      return;
    }
    var el = node && (node.nodeType === 1 ? node : node.parentElement);
    while (el && el.nodeType === 1) {
      if (drawnText.has(el)) {
        var owner = el.classList.contains("relative-date") ? el : el.parentElement;
        if (owner && owner.classList.contains("relative-date") && owner.hasAttribute("data-time")) {
          draw(owner);
        } else {
          draw(el);
        }
        return;
      }
      el = el.parentElement;
    }
  }

  var observer = new MutationObserver(function (records) {
    records.forEach(function (record) {
      if (record.type === "childList") {
        Array.prototype.forEach.call(record.addedNodes, function (node) {
          if (node.nodeType === 1) {
            formatDates(node);
          }
        });
      }
      redrawHolder(record.target);
    });
  });
  observer.observe(document.documentElement, {
    childList: true,
    subtree: true,
    characterData: true,
  });

  // Ember's relative-ages initializer: updateRelativeAge each minute,
  // which draws the age as the span's only content.
  setInterval(function () {
    document.querySelectorAll(".relative-date").forEach(function (el) {
      var ms = parseInt(el.dataset.time, 10);
      if (isNaN(ms)) {
        return;
      }
      var text = relativeAge(ms, el.dataset.format, drawnZone.get(el) || "");
      drawnText.set(el, text);
      drawnTemplate.delete(el);
      if (el.textContent !== text || el.children.length) {
        el.textContent = text;
      }
    });
  }, 60 * 1000);

  Discourse.formatDates = formatDates;
  Discourse.dates = {
    format: format,
    fill: fill,
    relativeAge: relativeAge,
    longDate: longDate,
    timelineDate: timelineDate,
    i18n: i18n,
  };

  formatDates(document);
})();
