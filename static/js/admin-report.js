// The admin report page in the browser (AdminReport on adminReports.show).
// The server draws the chrome from the report it loaded; this draws the
// report's mode into the body from the report the page carries
// (#admin-report-data), as the component draws it from the bulk
// response: AdminReportTable (computedLabels, sorting, pages, totals,
// the related items' summaries), the charts (admin-report-charts.js),
// AdminReportInlineTable, AdminReportCounters, AdminReportStorageStats
// and posters by member type, whose members report it loads as the
// nested AdminReport does. And the controls, which change the route's
// query params as onParamsChange does, the server drawing the page
// again: the chart groupings, the dates (and the refresh that applies
// them), the report's filters and the modes. Not ported: exporting a
// CSV, the compare groups modal, and a plugin's own mode or related
// items, which say so.
(function () {
  "use strict";

  if (!Discourse.once("admin-report")) {
    return;
  }

  var TABLE_OPTIONS = { perPage: 8, total: true, limit: 20, formatNumbers: true };
  var PAGES_LIMIT = 8;
  var SUMMARY_LIMIT = 5;
  var CHART_MODES = ["chart", "stacked_chart", "stacked_line_chart"];

  var strings = {};
  var data = null;
  var charts = [];
  // The component's state that outlives a refresh of the route (the same
  // AdminReport stays while only the query params change).
  var kept = null;

  function RC() {
    return Discourse.reportCharts;
  }

  function t(key, args) {
    var s = strings[key];
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
    return t(key + "." + (count === 1 ? "one" : "other"), all);
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // Handlebars' escapeExpression
  function escapeExpression(s) {
    var map = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;", "`": "&#x60;", "=": "&#x3D;" };
    if (s === null || s === undefined) {
      return "";
    }
    return String(s).replace(/[&<>"'`=]/g, function (c) {
      return map[c];
    });
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

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  // getURL
  function getURL(url) {
    var b = base();
    if (!url || !b || url.indexOf("/") !== 0 || url.indexOf("//") === 0) {
      return url;
    }
    return url.indexOf(b + "/") === 0 ? url : b + url;
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  // --- the route's query params --------------------------------------

  function isoDate(m) {
    return m.toISOString(true).split("T")[0];
  }

  function startDate() {
    if (kept.dateRangeFrom) {
      return moment(kept.dateRangeFrom);
    }
    return data.startDate ? moment(data.startDate, "YYYY-MM-DD") : moment();
  }

  function endDate() {
    if (kept.dateRangeTo) {
      return moment(kept.dateRangeTo);
    }
    return data.endDate ? moment(data.endDate, "YYYY-MM-DD") : moment();
  }

  function go(params) {
    var keys = Object.keys(params)
      .filter(function (k) {
        return params[k] !== null && params[k] !== undefined && params[k] !== "";
      })
      .sort();
    var query = keys
      .map(function (k) {
        return k + "=" + encodeURIComponent(params[k]);
      })
      .join("&");
    var url = location.pathname + (query ? "?" + query : "");
    if (!window.htmx) {
      location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // refreshReport, through the route's onParamsChange
  function refreshReport(options) {
    options = options || {};
    var start = options.startDate === undefined ? startDate() : options.startDate;
    var end = options.endDate === undefined ? endDate() : options.endDate;
    var filters = options.filters === undefined ? data.customFilters : options.filters;
    go({
      mode: kept.currentMode,
      start_date: start ? isoDate(start) : null,
      chart_grouping: options.chartGrouping,
      filters: filters && Object.keys(filters).length ? JSON.stringify(filters) : null,
      end_date: end ? isoDate(end) : null,
    });
  }

  // updateReportFilters
  function applyFilter(id, value) {
    var filters = Object.assign({}, data.customFilters || {});
    if (value === null || value === undefined) {
      delete filters[id];
    } else {
      filters[id] = value;
    }
    refreshReport({ filters: filters });
  }

  function changeGrouping(grouping) {
    var options = { chartGrouping: grouping };
    if (!kept.userHasCustomDates) {
      var months = { daily: 1, weekly: 3, monthly: 12 }[grouping];
      if (months) {
        var start = moment().subtract(months, "months").startOf("day");
        var end = moment().endOf("day");
        kept.dateRangeFrom = start;
        kept.dateRangeTo = end;
        options.startDate = start;
        options.endDate = end;
      }
    }
    refreshReport(options);
  }

  // --- Report's labels ----------------------------------------------------

  // utilities' toNumber
  function parseNumber(v) {
    return typeof v === "string" ? parseFloat(v) : v;
  }

  function rawAvatarSize(size) {
    var sizes = (data.avatarSizes || "")
      .split("|")
      .map(Number)
      .filter(function (n) {
        return n > 0;
      })
      .sort(function (a, b) {
        return a - b;
      });
    var raw = size * (window.devicePixelRatio || 1);
    for (var i = 0; i < sizes.length; i++) {
      if (sizes[i] >= raw) {
        return sizes[i];
      }
    }
    return sizes.length ? sizes[sizes.length - 1] : size;
  }

  function avatarImg(template, size, extra) {
    var src = getURL(String(template || "").replace(/\{size\}/g, rawAvatarSize(size)));
    return (
      "<img alt='' width='" +
      size +
      "' height='" +
      size +
      "' src='" +
      src +
      "' class='avatar" +
      (extra ? " " + extra : "") +
      "'>"
    );
  }

  // formatter.js's duration, tiny
  function durationTiny(distance) {
    if (typeof distance !== "number") {
      return "&mdash;";
    }
    var minutes = Math.round(distance / 60.0);
    minutes = minutes < 1 ? 1 : minutes;
    var k = function (key, count) {
      return tCount("dates.tiny." + key, count, { count: count });
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

  // I18n.toHumanSize
  function toHumanSize(value) {
    var number = Number(value) || 0;
    var units = ["byte", "kb", "mb", "gb", "tb"];
    var unit;
    var precision;
    if (number < 1024) {
      unit = tCount("number.human.storage_units.units.byte", number, { count: number });
      precision = 0;
    } else {
      var i = 0;
      while (number >= 1024 && i < 4) {
        number /= 1024;
        i++;
      }
      unit = t("number.human.storage_units.units." + units[i]);
      precision = number - Math.floor(number) === 0 ? 0 : 1;
    }
    var n = RC().toNumber(number, precision).replace(/,/g, "");
    return t("number.human.storage_units.format").replace("%n", n).replace("%u", unit);
  }

  function dash(v) {
    return v ? v : "—";
  }

  // Report#computedLabels
  function computedLabels(report) {
    return (report.labels || []).map(function (label) {
      var type = label.type || "string";
      var props = label.properties || {};
      var mainProperty;
      if (label.property) {
        mainProperty = label.property;
      } else if (type === "user") {
        mainProperty = props.username;
      } else if (type === "topic") {
        mainProperty = props.title;
      } else if (type === "post") {
        mainProperty = props.truncated_raw;
      } else {
        mainProperty = props[0];
      }
      return {
        title: label.title,
        htmlTitle: label.html_title,
        sortProperty: label.sort_property || mainProperty,
        mainProperty: mainProperty,
        type: type,
        compute: function (row, opts) {
          opts = opts || {};
          var value = opts.useSortProperty ? row[label.sort_property || mainProperty] : row[mainProperty];
          if (type === "user") {
            var username = row[props.username];
            return {
              value: username,
              formattedValue: username
                ? "<a href='" +
                  getURL("/admin/users/" + row[props.id] + "/" + username) +
                  "'>" +
                  // renderAvatar's tiny image, as the reference draws it.
                  "<img alt='' width='24' height='24' src='" +
                  getURL(String(row[props.avatar] || "").replace(/\{size\}/g, rawAvatarSize(24))) +
                  "' class='avatar' title='" +
                  escapeExpression(username) +
                  "'>" +
                  "<span class='username'>" +
                  username +
                  "</span></a>"
                : "—",
            };
          }
          if (type === "post") {
            var postTitle = row[props.truncated_raw];
            var href = getURL("/t/-/" + row[props.topic_id] + "/" + row[props.number]);
            return {
              property: props.title,
              value: postTitle,
              formattedValue: postTitle && href ? "<a href='" + href + "'>" + escapeExpression(postTitle) + "</a>" : "—",
            };
          }
          if (type === "topic") {
            var topicTitle = row[props.title];
            return {
              value: topicTitle,
              formattedValue: topicTitle
                ? "<a href='" + getURL("/t/-/" + row[props.id]) + "'>" + escapeExpression(topicTitle) + "</a>"
                : "—",
            };
          }
          if (type === "seconds") {
            return { value: parseNumber(value), formattedValue: durationTiny(value) };
          }
          if (type === "link") {
            var url = getURL(row[props[0]]);
            return {
              value: url,
              formattedValue: url
                ? '<a href="' + escapeExpression(url) + '">' + escapeExpression(row[props[1]]) + "</a>"
                : "—",
            };
          }
          if (type === "percent") {
            return {
              value: parseNumber(value),
              formattedValue: value === null || value === undefined ? "—" : value + "%",
            };
          }
          if (type === "bytes") {
            return { value: parseNumber(value), formattedValue: toHumanSize(value) };
          }
          if (type === "number") {
            var format = opts.formatNumbers === undefined || opts.formatNumbers === null || opts.formatNumbers === "" ? true : opts.formatNumbers;
            return {
              value: parseNumber(value),
              formattedValue: value ? (format ? RC().toNumber(value, 0) : value) : "—",
            };
          }
          if (type === "date" || type === "precise_date") {
            var date = moment(value);
            if (date.isValid()) {
              return { value: value, formattedValue: value ? date.format(type === "date" ? "LL" : "LLL") : "—" };
            }
          }
          if (type === "text") {
            return { value: value, formattedValue: value ? escapeExpression(value) : "—" };
          }
          return {
            value: value,
            type: type,
            property: mainProperty,
            formattedValue: value ? escapeExpression(value) : "—",
          };
        },
      };
    });
  }

  // --- AdminReportTable ---------------------------------------------------

  function hasRelatedItems(report, showRelatedItems) {
    if (!showRelatedItems) {
      return false;
    }
    var items = report.related_items || {};
    return Object.keys(items).some(function (k) {
      return (items[k] || []).length > 0;
    });
  }

  function renderTable(container, report, options, ctx) {
    var labels = computedLabels(report);
    var rows = RC().makeArray(report.data);
    var state = { sortLabel: null, sortDirection: 1, page: 0 };
    var perPage = options.perPage;
    var twoColumns = labels.length === 2;

    function totalsRow() {
      if (!rows.length) {
        return {};
      }
      var out = {};
      labels.forEach(function (label) {
        var total = rows.reduce(function (sum, row) {
          if (["seconds", "number", "percent"].indexOf(label.type) === -1) {
            return undefined;
          }
          return sum + Math.round(label.compute(row).value || 0);
        }, 0);
        out[label.mainProperty] = label.type === "percent" ? Math.round(total / rows.length) : total;
      });
      return out;
    }

    function totalsForSample() {
      var row = totalsRow();
      return labels.map(function (label) {
        var computed = label.compute(row);
        computed.type = label.type;
        computed.property = label.mainProperty;
        return computed;
      });
    }

    function sortedData() {
      var list = rows.slice();
      if (state.sortLabel) {
        var label = state.sortLabel;
        list.sort(function (a, b) {
          var av = label.compute(a, { useSortProperty: true }).value;
          var bv = label.compute(b, { useSortProperty: true }).value;
          return (av < bv ? -1 : av > bv ? 1 : 0) * state.sortDirection;
        });
      }
      return list;
    }

    function pages() {
      if (rows.length <= perPage) {
        return [];
      }
      var list = [];
      for (var i = 0; i < Math.ceil(rows.length / perPage); i++) {
        list.push({ page: i + 1, index: i, current: i === state.page });
      }
      if (list.length > PAGES_LIMIT) {
        var before = Math.max(0, state.page - PAGES_LIMIT / 2);
        var after = Math.max(PAGES_LIMIT, state.page + PAGES_LIMIT / 2);
        list = list.slice(before, after);
      }
      return list;
    }

    function header(label) {
      var classes = ["admin-report-table-header"];
      if (label.mainProperty) {
        classes.push(label.mainProperty);
      }
      if (label.type) {
        classes.push(label.type);
      }
      var current = state.sortLabel && state.sortLabel.sortProperty === label.sortProperty;
      if (current) {
        classes.push("is-current-sort");
      }
      var html = '<th class="' + escapeHtml(classes.join(" ")) + '"' + (label.title ? ' title="' + escapeHtml(label.title) + '"' : "") + ">";
      if (rows.length >= 5) {
        var title = current
          ? t("admin.dashboard.reports.sort_button_current", {
              column: label.title,
              direction: t("admin.dashboard.reports.sort_" + (state.sortDirection === 1 ? "ascending" : "descending")),
            })
          : t("admin.dashboard.reports.sort_button", { column: label.title });
        html +=
          '<button class="btn btn-icon-text btn-transparent --primary sort-btn" title="' +
          escapeHtml(title) +
          '" type="button" data-sort="' +
          labels.indexOf(label) +
          '">' +
          icon(state.sortDirection === 1 ? "angle-up" : "angle-down") +
          '<span class="d-button-label">' +
          (label.htmlTitle ? label.htmlTitle : escapeHtml(label.title)) +
          "</span></button>";
      } else if (label.htmlTitle) {
        html += '<span class="title">' + label.htmlTitle + "</span>";
      } else {
        html += '<span class="title">' + escapeHtml(label.title) + "</span>";
      }
      return html + "</th>";
    }

    function cell(row, label) {
      var computed = label.compute(row, options || {});
      var summary =
        ctx.hasRelatedItems && label.type === "number" && label.mainProperty === "y" && computed.value > 0;
      var classes = ["admin-report-table-cell", label.type, label.mainProperty].filter(Boolean).join(" ");
      var title = summary || computed.value === undefined || computed.value === null ? "" : ' title="' + escapeHtml(computed.value) + '"';
      var inner = computed.formattedValue;
      if (summary) {
        if (ctx.relatedRenderer) {
          inner = computed.formattedValue;
        } else {
          var dateLabel = moment.utc(row.x).format(t("dates.long_with_year_no_time"));
          var trigger = t("admin.reports.related_items.table_summary.trigger", {
            count: computed.formattedValue,
            date: dateLabel,
          });
          inner =
            '<button aria-label="' +
            escapeHtml(trigger) +
            '" aria-expanded="false" class="fk-d-menu__trigger admin-report-table-summary-trigger admin-report-table-summary" data-identifier="admin-report-table-summary" data-trigger="" data-date="' +
            escapeHtml(row.x) +
            '">' +
            computed.formattedValue +
            "</button>";
        }
      }
      return "<td class=\"" + escapeHtml(classes) + "\"" + title + ">" + inner + "</td>";
    }

    function draw() {
      var sorted = sortedData();
      var shown = perPage < sorted.length ? sorted.slice(perPage * state.page, perPage * state.page + perPage) : sorted;
      var html =
        '<div class="admin-report-table' + (twoColumns ? " two-columns" : "") + '"><table class="table"><thead><tr>';
      if (labels.length) {
        html += labels.map(header).join("");
      } else {
        html += rows
          .map(function (d) {
            return "<th>" + escapeHtml(d.x) + "</th>";
          })
          .join("");
      }
      html += "</tr></thead><tbody>";
      shown.forEach(function (row) {
        html +=
          '<tr class="admin-report-table-row">' +
          labels
            .map(function (label) {
              return cell(row, label);
            })
            .join("") +
          "</tr>";
      });
      var totals = totalsForSample();
      var sum = totals
        .map(function (x) {
          return x.value;
        })
        .filter(function (v) {
          return v !== null && v !== undefined;
        })
        .reduce(function (s, v) {
          return s + v;
        }, 0);
      if (sum >= 1 && options.total && report.dates_filtering) {
        html +=
          '<tr class="total-row"><td colspan="' +
          totals.length +
          '">' +
          escapeHtml(t("admin.dashboard.reports.totals_for_sample")) +
          '</td></tr><tr class="admin-report-table-row">' +
          totals
            .map(function (x) {
              return '<td class="admin-report-table-cell ' + escapeHtml(x.type) + " " + escapeHtml(x.property) + '">' + escapeHtml(x.formattedValue) + "</td>";
            })
            .join("") +
          "</tr>";
      }
      var last = labels[labels.length - 1];
      if (report.total && options.total && twoColumns) {
        html +=
          '<tr class="total-row"><td colspan="2">' +
          escapeHtml(t("admin.dashboard.reports.total")) +
          '</td></tr><tr class="admin-report-table-row"><td class="admin-report-table-cell date x">—</td><td class="admin-report-table-cell number y">' +
          escapeHtml(last.compute({ y: report.total }).formattedValue) +
          "</td></tr>";
      }
      if (report.average && rows.length > 0 && totals[1] && totals[1].value && twoColumns) {
        html +=
          '<tr class="total-row"><td colspan="2">' +
          escapeHtml(t("admin.dashboard.reports.average_for_sample")) +
          '</td></tr><tr class="admin-report-table-row"><td class="admin-report-table-cell date x">—</td><td class="admin-report-table-cell number y">' +
          escapeHtml(last.compute({ y: (totals[1].value / rows.length).toFixed(0) }).formattedValue) +
          "</td></tr>";
      }
      html += '</tbody></table><div class="pagination">';
      pages().forEach(function (p) {
        html +=
          '<button class="btn btn-text' +
          (p.current ? " is-current" : "") +
          '" type="button" data-page="' +
          p.index +
          '"><span class="d-button-label">' +
          p.page +
          "</span></button>";
      });
      html += "</div></div>";
      var wrap = document.createElement("div");
      wrap.innerHTML = html;
      var table = wrap.firstChild;
      if (drawn) {
        drawn.replaceWith(table);
      } else {
        container.appendChild(table);
      }
      drawn = table;
      table.addEventListener("click", function (event) {
        var sort = event.target.closest("[data-sort]");
        if (sort) {
          var label = labels[Number(sort.dataset.sort)];
          if (state.sortLabel === label) {
            state.sortDirection = state.sortDirection === 1 ? -1 : 1;
          } else {
            state.sortLabel = label;
          }
          draw();
          return;
        }
        var page = event.target.closest("[data-page]");
        if (page) {
          state.page = Number(page.dataset.page);
          draw();
          return;
        }
        var summary = event.target.closest(".admin-report-table-summary");
        if (summary) {
          openSummary(summary, report, ctx);
        }
      });
    }

    var drawn = null;
    draw();
  }

  // AdminReportTableSummary: the day's related users, loaded when shown.
  function openSummary(trigger, report, ctx) {
    var date = moment.utc(trigger.dataset.date).format("YYYY-MM-DD");
    var dateLabel = moment.utc(trigger.dataset.date).format(t("dates.long_with_year_no_time"));
    var content = document.createElement("div");
    content.className = "admin-report-table-summary__body";
    function body(inner) {
      content.innerHTML =
        '<h3 class="admin-report-table-summary__heading">' +
        escapeHtml(t("admin.reports.related_items.table_summary.users_title", { date: dateLabel })) +
        "</h3>" +
        inner;
    }
    function message(key) {
      return '<p class="admin-report-table-summary__message">' + escapeHtml(t(key)) + "</p>";
    }
    body(message("admin.reports.related_items.table_summary.loading"));
    var menu = Discourse.float.menu(trigger, {
      identifier: "admin-report-table-summary",
      ariaLabel: trigger.getAttribute("aria-label"),
      maxWidth: 360,
      fallbackPlacements: ["top-start"],
      placement: "bottom-start",
      modalForMobile: true,
      content: content,
    });
    // DMenu's @contentClass.
    var el = menu && menu.content;
    if (el && el.classList.contains("fk-d-menu")) {
      el.classList.add("admin-report-table-summary-content", "admin-report-table-summary__content");
    }
    var params = new URLSearchParams();
    params.set("start_date", date);
    params.set("end_date", date);
    params.set("limit", String(SUMMARY_LIMIT));
    params.append("facets[]", "related_items");
    params.set("include_related_items", "true");
    Object.keys(ctx.filters || {}).forEach(function (k) {
      appendParam(params, "filters[" + k + "]", ctx.filters[k]);
    });
    fetch(base() + "/admin/reports/" + report.type + "?" + params.toString(), {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (response) {
        if (!response.ok) {
          throw new Error(String(response.status));
        }
        return response.json();
      })
      .then(function (json) {
        var items = (json.report.related_items || {}).users || [];
        var totals = json.report.related_items_totals || {};
        var total = totals.users === undefined || totals.users === null ? items.length : totals.users;
        if (!items.length) {
          body(message("admin.reports.related_items.table_summary.empty"));
          return;
        }
        var html = "";
        if (items.length < total) {
          html +=
            '<p class="admin-report-table-summary__limit">' +
            escapeHtml(t("admin.reports.related_items.showing", { shown: items.length, total: total })) +
            "</p>";
        }
        html += '<ul class="admin-report-table-summary__list">';
        items.forEach(function (item) {
          var user = item.user || {};
          html +=
            '<li class="admin-report-table-summary__item"><a class="admin-report-table-summary__user-link" href="' +
            escapeHtml(getURL("/u/" + String(user.username || "").toLowerCase())) +
            '">' +
            avatarImg(user.avatar_template, 24, "admin-report-table-summary__avatar").replace(
              "class='avatar admin-report-table-summary__avatar'",
              "class='avatar admin-report-table-summary__avatar' title='" + escapeExpression(user.username) + "'"
            ) +
            '<span class="admin-report-table-summary__user-identity"><span class="admin-report-table-summary__username">' +
            escapeHtml(user.username || "") +
            "</span>" +
            (user.name ? '<span class="admin-report-table-summary__name">' + escapeHtml(user.name) + "</span>" : "") +
            "</span></a></li>";
        });
        body(html + "</ul>");
      })
      .catch(function () {
        body(message("admin.reports.related_items.table_summary.error"));
      });
  }

  // jQuery.param's nesting
  function appendParam(params, key, value) {
    if (Array.isArray(value)) {
      value.forEach(function (v) {
        appendParam(params, key + "[]", v);
      });
    } else if (value && typeof value === "object") {
      Object.keys(value).forEach(function (k) {
        appendParam(params, key + "[" + k + "]", value[k]);
      });
    } else {
      params.append(key, value === null || value === undefined ? "" : String(value));
    }
  }

  // --- the other modes ----------------------------------------------------

  function chartBox(classes) {
    var wrap = document.createElement("div");
    wrap.className = classes;
    wrap.innerHTML = '<div class="chart-canvas-container"><canvas class="chart-canvas"></canvas></div>';
    return wrap;
  }

  function keep(chart) {
    if (chart) {
      charts.push(chart);
    }
  }

  function renderInlineTable(container, report) {
    var html = '<div class="admin-report-inline-table"><div class="table-container">';
    RC().makeArray(report.data).forEach(function (d) {
      html +=
        '<a class="table-cell user-' +
        escapeHtml(d.key) +
        '" href="' +
        escapeHtml(d.url || "") +
        '"><span class="label">' +
        (d.icon ? icon(d.icon) : "") +
        escapeHtml(d.x) +
        '</span><span class="value">' +
        escapeHtml(RC().number(d.y)) +
        "</span></a>";
    });
    container.insertAdjacentHTML("beforeend", html + "</div></div>");
  }

  function renderPosters(container, report) {
    var rows = report.data || [];
    var labels = report.labels || [];
    var selected = rows[0] ? rows[0].type : null;
    var memberCount = null;
    var holder = document.createElement("div");
    holder.className = "posters-by-member-type-report admin-report-table";
    container.appendChild(holder);
    var categoryFilter = (report.available_filters || []).find(function (f) {
      return f.id === "category_ids";
    });
    var categoryIds = ((categoryFilter && categoryFilter.default) || []).join(",");

    function selectedRow() {
      return rows.find(function (r) {
        return r.type === selected;
      });
    }

    function draw() {
      var html =
        '<table class="posters-by-member-type-report__table table"><thead><tr><th>' +
        escapeHtml(labels[0] ? labels[0].title : "") +
        '</th><th class="posters-by-member-type-report__numeric">' +
        escapeHtml(labels[1] ? labels[1].title : "") +
        '</th><th class="posters-by-member-type-report__numeric">' +
        escapeHtml(labels[2] ? labels[2].title : "") +
        "</th></tr></thead><tbody>";
      rows.forEach(function (row) {
        html +=
          '<tr class="posters-by-member-type-report__row' +
          (row.type === selected ? " --selected" : "") +
          '" role="button" tabindex="0" data-type="' +
          escapeHtml(row.type) +
          '"><td class="posters-by-member-type-report__name">' +
          escapeHtml(row.name) +
          '</td><td class="posters-by-member-type-report__numeric">' +
          escapeHtml(row.count) +
          '</td><td class="posters-by-member-type-report__numeric">' +
          escapeHtml(row.share_formatted) +
          "</td></tr>";
      });
      html +=
        '<tr class="total-row"><td colspan="3">' +
        escapeHtml(t("admin.dashboard.reports.totals_for_sample")) +
        '</td></tr><tr><td>—</td><td class="posters-by-member-type-report__numeric">' +
        escapeHtml(report.total === undefined ? "" : report.total) +
        '</td><td class="posters-by-member-type-report__numeric">—</td></tr></tbody></table>';
      var row = selectedRow();
      if (row) {
        var users = tCount("admin.dashboard.reports.posters_by_member_type_members.users_count", memberCount, {
          count: memberCount,
        });
        var posts = tCount("admin.dashboard.reports.posters_by_member_type_members.posts_count", row.count || 0, {
          count: row.count || 0,
        });
        html +=
          '<div class="posters-by-member-type-report__members"><div aria-label="' +
          escapeHtml(
            t("admin.dashboard.reports.posters_by_member_type_members.title", { name: row.name, users: users, posts: posts })
          ) +
          '" class="posters-by-member-type-report__members-header"><div class="posters-by-member-type-report__members-heading"><h3 class="posters-by-member-type-report__members-name">' +
          escapeHtml(row.name) +
          "</h3>" +
          (memberCount !== null
            ? '<p class="posters-by-member-type-report__members-stats">' + escapeHtml(users + " · " + posts) + "</p>"
            : "") +
          '</div><button aria-label="' +
          escapeHtml(t("admin.dashboard.reports.posters_by_member_type_members.deselect", { name: row.name })) +
          '" class="btn no-text btn-icon btn-flat posters-by-member-type-report__members-close" type="button">' +
          icon("xmark") +
          '<span aria-hidden="true">​</span></button></div><div class="posters-by-member-type-report__members-report"></div></div>';
      }
      holder.innerHTML = html;
    }

    var loaded = {};
    function loadMembers() {
      var row = selectedRow();
      if (!row) {
        return;
      }
      var slot = holder.querySelector(".posters-by-member-type-report__members-report");
      var key = row.type;
      if (loaded[key]) {
        slot.replaceWith(loaded[key]);
        return;
      }
      var nested = nestedReport(
        "posters_by_member_type_members",
        {
          startDate: moment(report.start_date).format("YYYY-MM-DD"),
          endDate: moment(report.end_date).format("YYYY-MM-DD"),
          customFilters: { group: key, category_ids: categoryIds },
        },
        function (loadedReport) {
          if (selected !== key) {
            return;
          }
          memberCount = loadedReport && loadedReport.data ? loadedReport.data.length : 0;
          var stats = holder.querySelector(".posters-by-member-type-report__members-heading");
          var r = selectedRow();
          var users = tCount("admin.dashboard.reports.posters_by_member_type_members.users_count", memberCount, {
            count: memberCount,
          });
          var posts = tCount("admin.dashboard.reports.posters_by_member_type_members.posts_count", r.count || 0, {
            count: r.count || 0,
          });
          if (stats && !stats.querySelector(".posters-by-member-type-report__members-stats")) {
            stats.insertAdjacentHTML(
              "beforeend",
              '<p class="posters-by-member-type-report__members-stats">' + escapeHtml(users + " · " + posts) + "</p>"
            );
          }
          var header = holder.querySelector(".posters-by-member-type-report__members-header");
          if (header) {
            header.setAttribute(
              "aria-label",
              t("admin.dashboard.reports.posters_by_member_type_members.title", { name: r.name, users: users, posts: posts })
            );
          }
        }
      );
      loaded[key] = nested;
      slot.replaceWith(nested);
    }

    function select(type) {
      selected = type;
      memberCount = null;
      loaded = {};
      draw();
      loadMembers();
    }

    holder.addEventListener("click", function (event) {
      if (event.target.closest(".posters-by-member-type-report__members-close")) {
        selected = null;
        memberCount = null;
        draw();
        return;
      }
      var tr = event.target.closest(".posters-by-member-type-report__row");
      if (tr && !event.target.closest(".posters-by-member-type-report__members")) {
        select(tr.dataset.type);
      }
    });
    holder.addEventListener("keydown", function (event) {
      var tr = event.target.closest(".posters-by-member-type-report__row");
      if (tr && (event.key === "Enter" || event.key === " ")) {
        event.preventDefault();
        select(tr.dataset.type);
      }
    });
    draw();
    loadMembers();
  }

  // A nested AdminReport with neither header nor filtering: loaded through
  // ReportLoader's bulk request, then its body drawn.
  function nestedReport(type, filters, onLoaded) {
    var el = document.createElement("div");
    el.className = "admin-report is-enabled is-visible is-loading description-in-tooltip " + type.replace(/_/g, "-");
    el.innerHTML = '<div class="conditional-loading-section is-loading"><div class="spinner"></div></div>';
    var params = new URLSearchParams();
    var prefix = "reports[" + type + "]";
    appendParam(params, prefix + "[facets]", ["prev_period"]);
    appendParam(params, prefix + "[start_date]", filters.startDate);
    appendParam(params, prefix + "[end_date]", filters.endDate);
    if (filters.customFilters) {
      appendParam(params, prefix + "[filters]", filters.customFilters);
    }
    fetch(base() + "/admin/reports/bulk?" + params.toString(), {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (response) {
        if (!response.ok) {
          return null;
        }
        return response.json();
      })
      .then(function (json) {
        var report =
          json &&
          (json.reports || []).find(function (r) {
            return r.type === type;
          });
        el.classList.remove("is-loading");
        var section = document.createElement("div");
        section.className = "conditional-loading-section";
        var wrapper = document.createElement("div");
        wrapper.className = "chart__wrapper";
        var body = document.createElement("div");
        body.className = "chart__body";
        wrapper.appendChild(body);
        section.appendChild(wrapper);
        el.innerHTML = "";
        el.appendChild(section);
        if (report) {
          RC().fillReport(report);
          drawBody(body, report, (report.modes || [])[0], { filters: filters.customFilters });
        } else {
          drawBody(body, null, null, {});
        }
        if (onLoaded) {
          onLoaded(report);
        }
      });
    return el;
  }

  // The body below the average label: an error, the mode, or no data.
  function drawBody(body, report, mode, ctx) {
    var alert = function (cls, key) {
      body.insertAdjacentHTML(
        "beforeend",
        '<div class="alert alert-error report-alert ' + cls + '">' + icon("triangle-exclamation") + " <span>" + escapeHtml(t(key)) + "</span></div>"
      );
    };
    if (report && report.error === "timeout") {
      return alert("timeout", "admin.dashboard.timeout_error");
    }
    if (report && report.error === "exception") {
      return alert("exception", "admin.dashboard.exception_error");
    }
    if (report && report.error === "not_found") {
      return alert("not-found", "admin.dashboard.not_found_error");
    }
    if (report && present(report.data)) {
      if (mode) {
        drawMode(body, report, mode, Object.assign({ options: Object.assign({}, TABLE_OPTIONS) }, ctx));
      }
      return;
    }
    var noData = escapeHtml(t("admin.dashboard.reports.no_data"));
    if (report) {
      var url =
        "/admin/reports/" +
        report.type +
        "?start_date=" +
        moment.utc(report.start_date).locale("en").format("YYYY-MM-DD") +
        "&end_date=" +
        moment.utc(report.end_date).locale("en").format("YYYY-MM-DD");
      body.insertAdjacentHTML(
        "beforeend",
        '<div class="alert alert-info report-alert no-data">' +
          icon("chart-pie") +
          ' <a class="report-url" href="' +
          escapeHtml(getURL(url)) +
          '"><span>' +
          (report.title ? escapeHtml(report.title) + " — " : "") +
          noData +
          "</span></a></div>"
      );
    } else {
      body.insertAdjacentHTML(
        "beforeend",
        '<div class="alert alert-info report-alert no-data">' + icon("chart-pie") + " <span>" + noData + "</span></div>"
      );
    }
  }

  function present(v) {
    if (v === null || v === undefined) {
      return false;
    }
    if (Array.isArray(v)) {
      return v.length > 0;
    }
    if (typeof v === "object") {
      return Object.keys(v).length > 0;
    }
    if (typeof v === "string") {
      return v.trim().length > 0;
    }
    return true;
  }

  // modeComponent, drawn.
  function drawMode(body, report, mode, ctx) {
    var options = ctx.options || {};
    switch (String(mode).replace(/-/g, "_")) {
      case "table":
        renderTable(body, report, options, ctx);
        break;
      case "inline_table":
        renderInlineTable(body, report);
        break;
      case "chart":
        var box = chartBox("admin-report-chart");
        body.appendChild(box);
        keep(RC().lineChart(box.querySelector("canvas"), report, options.chartGrouping));
        break;
      case "stacked_chart":
        var stacked = chartBox("admin-report-chart admin-report-stacked-chart");
        body.appendChild(stacked);
        keep(RC().stackedChart(stacked.querySelector("canvas"), report, options));
        break;
      case "stacked_line_chart":
        var line = chartBox("admin-report-chart admin-report-stacked-line-chart");
        body.appendChild(line);
        keep(RC().stackedLineChart(line.querySelector("canvas"), report));
        break;
      case "radar":
        var radar = chartBox("admin-report-chart admin-report-radar");
        body.appendChild(radar);
        keep(RC().radarChart(radar.querySelector("canvas"), report));
        break;
      case "posters_by_member_type":
        renderPosters(body, report);
        break;
      default:
        // counters, storage stats and a plugin's own modes
        unsupported("the " + mode + " report mode");
    }
  }

  // _buildOptions
  function buildOptions(mode, report) {
    if (mode === "table") {
      return Object.assign({}, TABLE_OPTIONS, { perPage: 50, limit: data.limit });
    }
    if (mode === "chart") {
      return {
        chartGrouping: data.chartGrouping || report.default_group_by || RC().groupingFor((report.chartData || []).length),
      };
    }
    if (mode === "stacked_chart") {
      var first = (report.chartData && report.chartData[0] && report.chartData[0].data) || [];
      var options = {};
      if (report.type === "site_traffic") {
        options.hiddenLabels = ["page_view_other", "page_view_crawler", "page_view_embed"];
      }
      options.chartGrouping = data.chartGrouping || report.default_group_by || RC().groupingFor(first.length);
      return options;
    }
    return null;
  }

  // --- the chrome's controls ----------------------------------------------

  function setUpGroupings(el, report, options) {
    var fieldset = el.querySelector(".chart-groupings");
    if (!fieldset) {
      return;
    }
    var value = options && options.chartGrouping;
    var length = report.chartData && report.chartData[0] && Array.isArray(report.chartData[0].data)
      ? report.chartData[0].data.length
      : (report.chartData || []).length;
    fieldset.querySelectorAll(".d-segmented-control__label").forEach(function (label) {
      var input = label.querySelector("input");
      var disabled = input.value === "daily" && length >= RC().DAILY_LIMIT_DAYS && kept.userHasCustomDates;
      label.classList.toggle("is-disabled", disabled);
      input.disabled = disabled;
      input.checked = input.value === value;
    });
    positionSlider(fieldset, value);
  }

  // DSegmentedControl's positionSlider
  function positionSlider(fieldset, value) {
    if (!value) {
      return;
    }
    var checked = fieldset.querySelector("input:checked");
    if (!checked) {
      return;
    }
    var label = checked.closest(".d-segmented-control__label");
    var slider = fieldset.querySelector(".d-segmented-control__slider");
    requestAnimationFrame(function () {
      fieldset.style.setProperty("--slider-x", label.offsetLeft + "px");
      fieldset.style.setProperty("--slider-width", label.offsetWidth + "px");
      if (!slider.classList.contains("is-animated")) {
        requestAnimationFrame(function () {
          slider.classList.add("is-animated");
        });
      }
    });
  }

  // --- select-kit for the report's filters --------------------------------

  function float(chooser) {
    var body = chooser.querySelector(".select-kit-body");
    var header = chooser.querySelector("summary");
    var ui = window.FloatingUIDOM;
    if (!ui || !body) {
      return;
    }
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    // The header's width, not under the 220px the reference's bodies keep.
    body.style.minWidth = Math.max(220, Math.round(chooser.getBoundingClientRect().width)) + "px";
    ui.computePosition(header, body, {
      placement: "bottom-start",
      strategy: "fixed",
      middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
    }).then(function (result) {
      body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      body.style.visibility = "visible";
      body.style.pointerEvents = "auto";
    });
  }

  function normalize(s) {
    return String(s || "")
      .normalize("NFD")
      .replace(/[̀-ͯ]/g, "")
      .toLowerCase()
      .trim();
  }

  function rowLi(index, name, value, extra, selected) {
    return (
      '<li aria-checked="' +
      (selected ? "true" : "false") +
      '" role="menuitemradio" data-index="' +
      index +
      '" data-name="' +
      escapeHtml(name) +
      '"' +
      (value === undefined ? "" : ' data-value="' + escapeHtml(value) + '"') +
      ' title="' +
      escapeHtml(name) +
      '" tabindex="0" class="select-kit-row' +
      (extra ? " " + extra : "") +
      (selected ? " is-highlighted is-selected" : "") +
      '"><span class="name">' +
      escapeHtml(name) +
      "</span></li>"
    );
  }

  // A multi-select's filter has no search icon.
  function filterDiv(multi) {
    return (
      '<div class="select-kit-filter' +
      (multi ? " multi-select-filter" : "") +
      ' is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
      escapeHtml(t("select_kit.filter_placeholder")) +
      '" spellcheck="false" tabindex="0" type="search">' +
      (multi ? "" : icon("magnifying-glass", "filter-icon")) +
      "</div>"
    );
  }

  // The collection, its first row highlighted while filtering, and the
  // count of results a filter found.
  function collection(rows, filter) {
    if (filter && rows && rows.indexOf("is-highlighted") === -1) {
      rows = rows.replace("select-kit-row", "select-kit-row is-highlighted");
    }
    var html =
      '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
      (rows || '<p class="no-content">' + escapeHtml(t("select_kit.no_content")) + "</p>") +
      "</ul>";
    if (filter) {
      var count = (rows.match(/data-index="/g) || []).length;
      html +=
        '<span class="results-count" role="alert">' +
        escapeHtml(tCount("select_kit.results_count", count, { count: count })) +
        "</span>";
    }
    return html;
  }

  // The chooser's rows again for the filter's value.
  function refill(chooser, filter) {
    var body = chooser.querySelector(".select-kit-body");
    body.querySelectorAll(".select-kit-collection, .results-count").forEach(function (el) {
      el.remove();
    });
    body.insertAdjacentHTML("beforeend", collection(chooserRows(chooser, filter), filter));
  }

  function chooserRows(chooser, filter) {
    var id = chooser.dataset.filter;
    var spec = filterSpec(id);
    var needle = normalize(filter);
    if (!spec) {
      return "";
    }
    if (spec.type === "category" || spec.type === "category_list") {
      var chosen = spec.type === "category_list" ? (spec.default || []).map(String) : [];
      var selected = spec.type === "category" && spec.default ? String(spec.default) : null;
      var list = (data.categories || []).filter(function (c) {
        if (chosen.indexOf(String(c.id)) !== -1) {
          return false;
        }
        return !needle || normalize(c.name).indexOf(needle) !== -1;
      });
      return list
        .map(function (c, i) {
          var row = c.row.replace('data-name="', 'data-index="' + i + '" data-name="');
          if (String(c.id) === selected) {
            row = row.replace('class="category-row select-kit-row"', 'aria-checked="" class="category-row select-kit-row is-selected is-highlighted"');
          }
          return row;
        })
        .join("");
    }
    var options;
    var noneKey;
    var current = spec.default === null || spec.default === undefined ? null : String(spec.default);
    if (spec.type === "group") {
      options = (data.groups || []).map(function (g) {
        return { name: g.name, value: String(g.value) };
      });
      noneKey = "admin.dashboard.reports.groups";
    } else {
      options = (spec.choices || []).map(function (c) {
        return typeof c === "object" ? { name: c.name, value: String(c.id) } : { name: String(c), value: String(c) };
      });
      noneKey = "admin.dashboard.report_filter_any";
    }
    var html = "";
    var index = 0;
    if (current) {
      html += rowLi(index++, t(noneKey), "", "is-none none", false);
    }
    options
      .filter(function (o) {
        return !needle || normalize(o.name).indexOf(needle) !== -1;
      })
      .forEach(function (o) {
        html += rowLi(index++, o.name, o.value, "", o.value === current);
      });
    return html;
  }

  function filterSpec(id) {
    var report = data && data.report;
    return ((report && report.available_filters) || []).find(function (f) {
      return f.id === id;
    });
  }

  function openChooser(chooser, filterValue) {
    chooser.open = true;
    chooser.classList.add("is-expanded");
    var multi = chooser.classList.contains("multi-select");
    var caret = chooser.querySelector("summary .angle-icon");
    if (caret && !multi) {
      caret.outerHTML = icon("angle-up", "angle-icon");
    }
    var body = chooser.querySelector(".select-kit-body");
    body.innerHTML = filterDiv(multi) + collection(chooserRows(chooser, filterValue), filterValue);
    var input = body.querySelector(".filter-input");
    input.value = filterValue || "";
    input.focus();
    float(chooser);
  }

  function closeChooser(chooser) {
    chooser.open = false;
    chooser.classList.remove("is-expanded");
    var body = chooser.querySelector(".select-kit-body");
    body.innerHTML = "";
    body.removeAttribute("style");
    var caret = chooser.querySelector("summary .angle-icon");
    if (caret && !chooser.classList.contains("multi-select")) {
      caret.outerHTML = icon("angle-down", "angle-icon");
    }
  }

  function closeChoosers(except) {
    document.querySelectorAll(".chart__additional-filters details.select-kit[open]").forEach(function (c) {
      if (c !== except) {
        closeChooser(c);
        if (c.classList.contains("multiple-categories-selector")) {
          applyCategoryList(c);
        }
      }
    });
  }

  // CategoryList's onClose: applied only when the selection changed.
  function applyCategoryList(chooser) {
    var spec = filterSpec(chooser.dataset.filter);
    var picked = chooser._picked;
    if (!spec || !picked) {
      return;
    }
    var current = (spec.default || []).map(Number);
    if (
      picked.length === current.length &&
      picked.every(function (id) {
        return current.indexOf(id) !== -1;
      })
    ) {
      return;
    }
    applyFilter(spec.id, picked.length ? picked : undefined);
  }

  function choose(chooser, row) {
    var spec = filterSpec(chooser.dataset.filter);
    if (!spec) {
      return;
    }
    var value = row.dataset.value;
    if (spec.type === "category_list") {
      chooser._picked = (chooser._picked || (spec.default || []).map(Number)).concat([Number(value)]);
      var names = chooser._picked.map(function (id) {
        var c = (data.categories || []).find(function (x) {
          return x.id === id;
        });
        return c ? c.name : String(id);
      });
      chooser.querySelector(".formatted-selection").textContent = names.join(", ");
      spec.default = chooser._picked.slice();
      var body = chooser.querySelector(".select-kit-body");
      var input = body.querySelector(".filter-input");
      refill(chooser, input ? input.value : "");
      return;
    }
    closeChooser(chooser);
    if (spec.type === "category") {
      applyFilter(spec.id, value ? Number(value) : undefined);
    } else if (spec.type === "group") {
      applyFilter(spec.id, value ? Number(value) : null);
    } else {
      applyFilter(spec.id, value || null);
    }
  }

  // --- the page -------------------------------------------------------------

  document.addEventListener("click", function (event) {
    var el = document.querySelector(".admin-contents.admin-reports .admin-config-area > .admin-report");
    if (!el || !data || !el.contains(event.target)) {
      if (el && !event.target.closest(".select-kit-body")) {
        closeChoosers(null);
      }
      return;
    }
    var target = event.target;
    var chooser = target.closest(".chart__additional-filters details.select-kit");
    closeChoosers(chooser);
    if (chooser) {
      event.preventDefault();
      if (target.closest(".btn-clear")) {
        closeChooser(chooser);
        var cleared = filterSpec(chooser.dataset.filter);
        if (cleared) {
          applyFilter(cleared.id, undefined);
        }
        return;
      }
      var row = target.closest(".select-kit-row");
      if (row) {
        choose(chooser, row);
        return;
      }
      if (target.closest(".select-kit-filter")) {
        return;
      }
      if (chooser.open) {
        closeChooser(chooser);
        if (chooser.classList.contains("multiple-categories-selector")) {
          applyCategoryList(chooser);
        }
      } else {
        openChooser(chooser, "");
      }
      return;
    }
    var grouping = target.closest(".chart-groupings .d-segmented-control__label");
    if (grouping) {
      // The radio's change event does the grouping; the label's click only
      // lets it through.
      return;
    }
    var modeButton = target.closest(".chart__modes .mode-btn");
    if (modeButton) {
      kept.currentMode = modeButton.dataset.mode;
      refreshReport({ chartGrouping: null });
      return;
    }
    if (target.closest(".refresh-report-btn")) {
      refreshReport();
      return;
    }
    if (target.closest(".export-csv-btn")) {
      unsupported("exporting a report as CSV");
      return;
    }
    if (target.closest(".report-filter-groups__button")) {
      unsupported("comparing groups (CompareGroups)");
      return;
    }
    var checkbox = target.closest(".chart__filter-label input[type=checkbox][data-filter]");
    if (checkbox) {
      var spec = filterSpec(checkbox.dataset.filter);
      // Bool's onChange: the box's state before the click, flipped.
      var wasChecked = !!(spec && spec.default);
      applyFilter(checkbox.dataset.filter, !wasChecked || undefined);
    }
  });

  document.addEventListener("input", function (event) {
    var input = event.target.closest(".chart__additional-filters .select-kit-filter .filter-input");
    if (!input) {
      return;
    }
    refill(input.closest("details.select-kit"), input.value);
  });

  // The filter's Enter picks the highlighted row; Escape closes.
  document.addEventListener("keydown", function (event) {
    var input = event.target.closest && event.target.closest(".chart__additional-filters .select-kit-filter .filter-input");
    if (!input) {
      return;
    }
    var chooser = input.closest("details.select-kit");
    if (event.key === "Enter") {
      event.preventDefault();
      var row =
        chooser.querySelector(".select-kit-collection .select-kit-row.is-highlighted") ||
        chooser.querySelector(".select-kit-collection .select-kit-row");
      if (row) {
        choose(chooser, row);
      }
    } else if (event.key === "Escape") {
      closeChoosers(null);
    }
  });

  document.addEventListener("change", function (event) {
    var el = document.querySelector(".admin-contents.admin-reports .admin-config-area > .admin-report");
    if (!el || !data || !el.contains(event.target)) {
      return;
    }
    var radio = event.target.closest(".chart-groupings input[type=radio]");
    if (radio) {
      changeGrouping(radio.value);
      return;
    }
    var date = event.target.closest(".chart__dates .date-picker");
    if (date) {
      // onChangeDateRange
      kept.userHasCustomDates = true;
      var from = el.querySelector(".chart__dates .from .date-picker");
      var to = el.querySelector(".chart__dates .to .date-picker");
      kept.dateRangeFrom = from && from.value ? moment(from.value, "YYYY-MM-DD") : null;
      kept.dateRangeTo = to && to.value ? moment(to.value, "YYYY-MM-DD") : null;
      if (to && from) {
        to.min = from.value;
      }
      var report = data.report;
      if (report) {
        setUpGroupings(el, report, kept.options);
      }
    }
  });

  Discourse.onPage(function () {
    var script = document.getElementById("admin-report-data");
    var el = document.querySelector(".admin-contents.admin-reports .admin-config-area > .admin-report");
    if (!script || !el) {
      kept = null;
      return;
    }
    data = JSON.parse(script.textContent);
    strings = data.strings || {};
    RC().setStrings(strings);
    // The same AdminReport across a change of the query params keeps its
    // custom dates, its back link and, until the type changes, the rest.
    var back = el.parentNode.parentNode.parentNode.querySelector(".back-button");
    if (kept && back && back.getAttribute("href") === base() + "/admin/reports" && kept.back) {
      back.outerHTML = kept.back;
      back = el.parentNode.parentNode.parentNode.querySelector(".back-button");
    }
    kept = {
      back: back ? back.outerHTML : null,
      userHasCustomDates: kept ? kept.userHasCustomDates : false,
      dateRangeFrom: kept ? kept.dateRangeFrom : null,
      dateRangeTo: kept ? kept.dateRangeTo : null,
      currentMode: null,
      options: null,
    };
    var report = data.report;
    if (!report || el.classList.contains("hidden")) {
      return;
    }
    RC().fillReport(report);
    var modes = report.modes || [];
    kept.currentMode = data.mode || modes[0] || null;
    kept.options = buildOptions(kept.currentMode, report);
    setUpGroupings(el, report, kept.options);
    var body = el.querySelector(".chart__body");
    var related = hasRelatedItems(report, data.admin);
    if (related && report.type === "accepted_solutions") {
      unsupported("discourse-solved's related solved topics");
    }
    if (!report.error && present(report.data) && kept.currentMode && body) {
      var options = kept.options || {};
      drawMode(body, report, kept.currentMode, {
        options: options,
        hasRelatedItems: related,
        relatedRenderer: report.type === "accepted_solutions",
        filters: data.customFilters,
      });
    }
    return function () {
      closeChoosers(null);
      charts.forEach(function (chart) {
        chart.destroy();
      });
      charts = [];
    };
  });
})();
