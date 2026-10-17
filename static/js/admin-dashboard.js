// The admin dashboard's client side (RedesignedAdminDashboard): the
// pinned reports' charts, loaded as loadDashboardReports loads them (POST
// /admin/dashboard/reports/bulk) and drawn as AdminReportChart and
// AdminReportStackedChart draw them (admin-report-charts.js); the site traffic's
// stacked chart from the data the page carries; the site advice's Ignore
// (DELETE /admin/admin_notices/:id) and Refresh (POST
// /admin/dashboard/problems); the period picker (DashboardDateRange with
// DashboardDateRangePicker) and the configure menu (ConfigureMenu, PUT
// /admin/dashboard/configuration), which reload the page for the new
// range or layout; a report card's size toggle (PUT
// /admin/dashboard/reports/layout); the category activity's sorting; the
// tooltips. Not ported: the manage reports, compare groups and category
// pickers, and dragging a card's corner to resize it, which say so when
// used.
(function () {
  "use strict";

  if (!Discourse.once("admin-dashboard")) {
    return;
  }

  var MAX_COLS = 2;

  var strings = {};
  var charts = [];

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

  function escapeHtml(s) {
    return String(s)
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

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : "";
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function ajax(method, path, body) {
    var options = {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
    };
    if (body !== undefined) {
      options.headers["Content-Type"] = "application/json";
      options.body = JSON.stringify(body);
    }
    return fetch(base() + path, options).then(function (response) {
      if (!response.ok) {
        return response.text().then(function (text) {
          var message = text;
          try {
            var json = JSON.parse(text);
            if (json.errors) {
              message = json.errors.join(" ");
            }
          } catch (_) {}
          throw new Error(message);
        });
      }
      var type = response.headers.get("Content-Type") || "";
      return type.indexOf("json") !== -1 ? response.json() : null;
    });
  }

  // popupAjaxError, and the refusal of what is not ported.
  function showError(message) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = message;
      error.hidden = false;
    }
  }

  function unsupported(what) {
    showError("Not ported yet: " + what + ".");
  }

  // The page again for a new layout, or for a new range (the controller's
  // query params): the server draws the sections.
  function reload() {
    location.reload();
  }

  function go(query) {
    location.href = base() + "/admin" + (query ? "?" + query : "");
  }

  // The charts' module (admin-report-charts.js, loaded before this one).
  function charting() {
    return Discourse.reportCharts;
  }

  function keep(chart) {
    if (chart) {
      charts.push(chart);
    }
  }

  // The card's AdminReport @bare: a chart, or the empty or error state.
  function renderCard(card, entry) {
    var chart = card.querySelector(".db-report__chart");
    var container = chart.querySelector(".loading-container");
    if (!entry || entry.error) {
      chart.innerHTML =
        '<div class="db-report__error" role="alert">' +
        icon("triangle-exclamation") +
        "<span>" +
        escapeHtml(t("admin.dashboard.reports_section.report_error")) +
        "</span></div>";
      return;
    }
    var report = entry.data;
    if (report && report.empty) {
      chart.innerHTML =
        '<div class="db-report__empty">' +
        icon("chart-pie") +
        "<span>" +
        escapeHtml(t("admin.dashboard.reports_section.no_data")) +
        "</span></div>";
      return;
    }
    if (!report) {
      return;
    }
    charting().fillReport(report);
    var mode = (report.modes || [])[0];
    var type = String(report.type || "").replace(/_/g, "-");
    var div = document.createElement("div");
    div.className =
      "admin-report --bare is-enabled is-visible" + (report.description ? " description-in-tooltip" : "") + " " + type;
    container.innerHTML = "";
    container.appendChild(div);
    if (mode === "chart" || mode === "stacked_chart") {
      var wrap = document.createElement("div");
      wrap.className = mode === "chart" ? "admin-report-chart" : "admin-report-chart admin-report-stacked-chart";
      wrap.innerHTML = '<div class="chart-canvas-container"><canvas class="chart-canvas"></canvas></div>';
      div.appendChild(wrap);
      var canvas = wrap.querySelector("canvas");
      if (mode === "chart") {
        var grouping =
          report.default_group_by || charting().groupingFor((report.chartData || []).length);
        keep(charting().lineChart(canvas, report, grouping));
      } else {
        var first = (report.chartData && report.chartData[0] && report.chartData[0].data) || [];
        keep(
          charting().stackedChart(canvas, report, {
            chartGrouping: report.default_group_by || charting().groupingFor(first.length),
          })
        );
      }
    } else {
      // The report's other modes (tables, counters, radar, storage) are
      // not ported.
      div.innerHTML =
        '<div class="db-report__error" role="alert">' +
        icon("triangle-exclamation") +
        "<span>Not ported yet: the " +
        escapeHtml(mode || "unknown") +
        " report mode.</span></div>";
    }
  }

  function loadReports(root) {
    var reports = root.querySelector(".db-reports[data-items]");
    if (!reports) {
      return;
    }
    var items = JSON.parse(reports.dataset.items);
    if (!items.length) {
      return;
    }
    ajax("POST", "/admin/dashboard/reports/bulk.json", {
      items: items,
      filters: JSON.parse(reports.dataset.filters),
    })
      .then(function (response) {
        var byKey = {};
        (response.items || []).forEach(function (entry) {
          byKey[entry.key] = entry;
        });
        reports.querySelectorAll(".db-report__card[data-identifier]").forEach(function (card) {
          renderCard(card, byKey[card.dataset.identifier]);
        });
      })
      .catch(function (e) {
        reports.querySelectorAll(".db-report__card[data-identifier]").forEach(function (card) {
          renderCard(card, { error: true });
        });
        showError(e.message);
      });
  }

  function drawTraffic(root) {
    var el = root.querySelector(".db-section__traffic-chart-canvas[data-chart]");
    if (!el) {
      return;
    }
    var chart = JSON.parse(el.dataset.chart);
    keep(
      charting().stackedChart(el.querySelector("canvas"), chart, {
        hideYAxisGridLines: true,
        hiddenLabels: ["page_view_crawler"],
      })
    );
  }

  // DashboardSiteAdvice
  function renderProblems(section, problems) {
    problems = problems.slice().sort(function (a, b) {
      var pa = a.priority || "";
      var pb = b.priority || "";
      if (pa !== pb) {
        return pa < pb ? -1 : 1;
      }
      return (a.id || 0) - (b.id || 0);
    });
    if (!problems.length) {
      section.remove();
      return;
    }
    section.querySelector("h3").textContent = tCount("admin.dashboard.site_advice.title", problems.length);
    var admin = section.dataset.admin === "true";
    section.querySelector(".db-site-advice__list").innerHTML = problems
      .map(function (p) {
        return (
          '<li class="db-site-advice__item" data-test-site-advice-problem="' +
          p.id +
          '">' +
          icon(p.priority === "high" ? "triangle-exclamation" : "circle-info", "db-site-advice__icon --" + escapeHtml(p.priority)) +
          '<div class="db-site-advice__message">' +
          p.message +
          "</div>" +
          (admin
            ? '<button class="btn btn-default btn-small" data-test-site-advice-ignore="true" data-problem-id="' +
              p.id +
              '" type="button"><span class="d-button-label">' +
              escapeHtml(t("admin.dashboard.dismiss_notice")) +
              "</span></button>"
            : "") +
          "</li>"
        );
      })
      .join("");
  }

  function currentProblems(section) {
    return Array.prototype.map.call(section.querySelectorAll(".db-site-advice__item"), function (li) {
      var svg = li.querySelector("svg");
      var priority = /--(\w+)/.exec(svg.getAttribute("class"));
      return {
        id: parseInt(li.dataset.testSiteAdviceProblem, 10),
        priority: priority ? priority[1] : "low",
        message: li.querySelector(".db-site-advice__message").innerHTML,
      };
    });
  }

  function setLoading(button, loading) {
    button.disabled = loading;
    button.classList.toggle("is-loading", loading);
  }

  // ConfigureMenu: each section's toggle and order.
  function configureMenu(template) {
    var el = document.createElement("div");
    el.appendChild(template.content.cloneNode(true));
    function sections() {
      return Array.prototype.map.call(el.querySelectorAll(".db-configure__row"), function (row) {
        return {
          id: row.dataset.sectionId,
          visible: row.querySelector(".d-toggle-switch__checkbox").getAttribute("aria-checked") === "true",
        };
      });
    }
    function save(next) {
      ajax("PUT", "/admin/dashboard/configuration.json", { sections: next })
        .then(function () {
          Discourse.float.close("db-configure");
          reload();
        })
        .catch(function (e) {
          showError(e.message);
        });
    }
    el.addEventListener("click", function (event) {
      var toggle = event.target.closest(".d-toggle-switch__checkbox");
      if (toggle) {
        var id = toggle.closest(".db-configure__row").dataset.sectionId;
        save(
          sections().map(function (s) {
            return s.id === id ? { id: s.id, visible: !s.visible } : s;
          })
        );
        return;
      }
      var arrow = event.target.closest(".db-configure__arrow");
      if (arrow && !arrow.disabled) {
        var list = sections();
        var row = arrow.closest(".db-configure__row");
        var from = Array.prototype.indexOf.call(row.parentNode.children, row);
        var to = from + (arrow.dataset.direction === "up" ? -1 : 1);
        var moved = list.splice(from, 1)[0];
        list.splice(to, 0, moved);
        save(list);
      }
    });
    var dragging = null;
    el.addEventListener("dragstart", function (event) {
      var row = event.target.closest(".db-configure__row");
      dragging = row ? Array.prototype.indexOf.call(row.parentNode.children, row) : null;
    });
    el.addEventListener("dragover", function (event) {
      if (dragging !== null && event.target.closest(".db-configure__row")) {
        event.preventDefault();
      }
    });
    el.addEventListener("drop", function (event) {
      var row = event.target.closest(".db-configure__row");
      if (dragging === null || !row) {
        return;
      }
      event.preventDefault();
      var to = Array.prototype.indexOf.call(row.parentNode.children, row);
      if (to !== dragging) {
        var list = sections();
        var moved = list.splice(dragging, 1)[0];
        list.splice(to, 0, moved);
        save(list);
      }
      dragging = null;
    });
    return el;
  }

  // ActivityByCategory#updateSort
  function sortActivity(button) {
    var table = button.closest("table");
    var field = button.dataset.sort;
    var current = table.dataset.sortBy || "share";
    var dir = table.dataset.sortDir || "desc";
    if (current === field) {
      dir = dir === "desc" ? "asc" : "desc";
    } else {
      dir = "desc";
    }
    table.dataset.sortBy = field;
    table.dataset.sortDir = dir;
    table.querySelectorAll("th[aria-sort]").forEach(function (th) {
      var b = th.querySelector("button");
      var on = b.dataset.sort === field;
      th.setAttribute("aria-sort", on ? (dir === "asc" ? "ascending" : "descending") : "none");
      var svg = b.querySelector("svg");
      var name = dir === "asc" ? "chevron-up" : "chevron-down";
      svg.setAttribute("class", "fa d-icon d-icon-" + name + " svg-icon fa-width-auto" + (on ? "" : " is-hidden") + " svg-string");
      svg.querySelector("use").setAttribute("href", "#" + name);
    });
    var body = table.querySelector("tbody");
    var rows = Array.prototype.slice.call(body.rows);
    var direction = dir === "asc" ? 1 : -1;
    rows.sort(function (a, b) {
      var av = parseFloat(a.dataset[field]) || 0;
      var bv = parseFloat(b.dataset[field]) || 0;
      return av === bv ? 0 : av > bv ? direction : -direction;
    });
    rows.forEach(function (row) {
      body.appendChild(row);
    });
  }

  Discourse.onPage(function () {
    var main = document.querySelector(".db-main");
    if (!main) {
      return;
    }
    var json = document.getElementById("db-strings");
    strings = json ? JSON.parse(json.textContent) : {};
    charting().setStrings(strings);
    loadReports(main);
    drawTraffic(main);
    return function () {
      charts.forEach(function (chart) {
        chart.destroy();
      });
      charts = [];
    };
  });

  document.addEventListener("click", function (event) {
    if (!event.target.closest(".admin-contents.dashboard")) {
      return;
    }
    var tip = event.target.closest(".fk-d-tooltip__trigger[data-tooltip-content]");
    if (tip) {
      // The tooltip lives inside links (a KPI's): it opens, not the link.
      event.preventDefault();
      event.stopPropagation();
      Discourse.float.showTooltip(tip, {
        identifier: tip.dataset.identifier || "db-tooltip",
        content: tip.dataset.tooltipContent,
      });
      return;
    }
    var ignore = event.target.closest("[data-test-site-advice-ignore]");
    if (ignore) {
      var section = ignore.closest(".db-site-advice");
      var id = parseInt(ignore.dataset.problemId, 10);
      setLoading(ignore, true);
      ajax("DELETE", "/admin/admin_notices/" + id)
        .then(function () {
          renderProblems(
            section,
            currentProblems(section).filter(function (p) {
              return p.id !== id;
            })
          );
        })
        .catch(function (e) {
          setLoading(ignore, false);
          showError(e.message);
        });
      return;
    }
    var refresh = event.target.closest("[data-test-site-advice-refresh]");
    if (refresh) {
      var advice = refresh.closest(".db-site-advice");
      setLoading(refresh, true);
      ajax("POST", "/admin/dashboard/problems.json")
        .then(function (model) {
          setLoading(refresh, false);
          renderProblems(advice, model.problems || []);
        })
        .catch(function (e) {
          setLoading(refresh, false);
          showError(e.message);
        });
      return;
    }
    var range = event.target.closest(".db-date-range__trigger");
    if (range) {
      var menu = Discourse.float.menu(range, {
        identifier: "db-date-range-menu",
        contentClass: "db-date-range__popover",
        maxWidth: 800,
        modalForMobile: true,
        placement: "bottom-end",
        content: Discourse.dateRangePicker(range, {
          t: t,
          escapeHtml: escapeHtml,
          icon: icon,
          close: function () {
            Discourse.float.close("db-date-range-menu");
          },
          onPreset: function (id) {
            go("range=" + encodeURIComponent(id));
          },
          onRange: function (start, end) {
            go("end_date=" + end + "&range=custom&start_date=" + start);
          },
        }),
      });
      return menu;
    }
    var configure = event.target.closest(".db-configure-trigger");
    if (configure) {
      var template = document.getElementById("db-configure-content");
      if (template) {
        Discourse.float.menu(configure, {
          identifier: "db-configure",
          modalForMobile: true,
          content: configureMenu(template),
        });
      }
      return;
    }
    var handle = event.target.closest(".db-report__resize-handle");
    if (handle) {
      // A click on the corner toggles the card between one and both
      // columns (onCardResizeEnd without a move).
      var reports = handle.closest(".db-reports");
      var card = handle.closest(".db-report__card");
      var items = JSON.parse(reports.dataset.items);
      var key = card.dataset.identifier;
      var next = items.map(function (i) {
        if (i.source + ":" + i.identifier !== key) {
          return i;
        }
        var normal = i.rows === 1 && i.cols === 1;
        return Object.assign({}, i, normal ? { rows: 1, cols: MAX_COLS } : { rows: 1, cols: 1 });
      });
      ajax("PUT", "/admin/dashboard/reports/layout.json", { items: next })
        .then(reload)
        .catch(function (e) {
          showError(e.message);
        });
      return;
    }
    var sort = event.target.closest(".db-activity-table__sort-button");
    if (sort) {
      sortActivity(sort);
      return;
    }
    if (event.target.closest(".db-report__add-report, .db-section.--reports .db-section__header-action button")) {
      unsupported("the dashboard's manage reports modal");
      return;
    }
    if (event.target.closest(".db-whos-posting__add-group")) {
      unsupported("the dashboard's compare groups modal");
      return;
    }
    if (event.target.closest(".multiple-categories-selector summary")) {
      event.preventDefault();
      unsupported("the dashboard's category picker");
    }
  });

  document.addEventListener("mouseover", function (event) {
    var tip = event.target.closest(
      ".admin-contents.dashboard .fk-d-tooltip__trigger[data-tooltip-content]"
    );
    if (!tip || tip.getAttribute("aria-expanded") === "true") {
      return;
    }
    if (document.documentElement.classList.contains("mobile-view")) {
      return;
    }
    Discourse.float.showTooltip(tip, {
      identifier: tip.dataset.identifier || "db-tooltip",
      content: tip.dataset.tooltipContent,
    });
  });
})();
