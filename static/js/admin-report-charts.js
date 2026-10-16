// The admin reports' charts, shared by the dashboard (admin-dashboard.js)
// and the report page (admin-report.js): Report's collapse and
// fillMissingDates, formatter.js's number and I18n.toNumber, and
// AdminReportChart, AdminReportStackedChart, AdminReportStackedLineChart
// and AdminReportRadar drawn with Chart.js. A chart function returns the
// Chart it drew (null without Chart.js), which its page destroys.
(function () {
  "use strict";

  if (!Discourse.once("admin-report-charts")) {
    return;
  }

  var DOTTED_LINE = [5, 5];
  var DAILY_LIMIT_DAYS = 34;
  var WEEKLY_LIMIT_DAYS = 365;

  // The strings the pages hand over (placeholders intact).
  var strings = {};

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

  function cssColor(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  function remToPx(rem) {
    return rem * parseFloat(getComputedStyle(document.documentElement).fontSize);
  }

  // I18n.toNumber
  function toNumber(value, precision) {
    var negative = value < 0;
    var parts = Math.abs(value).toFixed(precision).split(".");
    var digits = parts[0];
    var buffer = [];
    while (digits.length > 0) {
      var pos = Math.max(0, digits.length - 3);
      buffer.unshift(digits.slice(pos, pos + 3));
      digits = digits.slice(0, -3);
    }
    var out = buffer.join(",");
    if (precision > 0) {
      out += "." + parts[1];
    }
    return negative ? "-" + out : out;
  }

  // formatter.js's number
  function number(val) {
    val = Math.round(parseFloat(val));
    if (isNaN(val)) {
      val = 0;
    }
    if (val > 999999) {
      return t("number.short.millions", { number: toNumber(val / 1000000, 1) });
    }
    if (val > 99999) {
      return t("number.short.thousands", { number: toNumber(Math.floor(val / 1000), 0) });
    }
    if (val > 999) {
      return t("number.short.thousands", { number: toNumber(val / 1000, 1) });
    }
    return val.toString();
  }

  // Report.groupingForDatapoints and unitForGrouping
  function groupingFor(count) {
    if (count < DAILY_LIMIT_DAYS) {
      return "daily";
    }
    return count < WEEKLY_LIMIT_DAYS ? "weekly" : "monthly";
  }

  function unitFor(grouping) {
    return grouping === "monthly" ? "month" : grouping === "weekly" ? "week" : "day";
  }

  // utilities' fillMissingDates
  function fillMissingDates(data, startDate, endDate) {
    var start = moment(startDate, "YYYY-MM-DD");
    var end = moment(endDate, "YYYY-MM-DD");
    var count = end.diff(start, "days");
    var current = start;
    for (var i = 0; i <= count; i++) {
      var date = data[i] ? moment(data[i].x, "YYYY-MM-DD") : null;
      if (i === 0 && (!date || date.isAfter(start))) {
        data.splice(i, 0, { x: start.format("YYYY-MM-DD"), y: 0 });
      } else if (!date || date.isAfter(moment(current))) {
        data.splice(i, 0, { x: current, y: 0 });
      }
      current = moment(current).add(1, "day").format("YYYY-MM-DD");
    }
    return data;
  }

  function dateMoment(value) {
    if (moment.isMoment(value)) {
      return value.clone().startOf("day");
    }
    if (value instanceof Date) {
      return moment(value).startOf("day");
    }
    return moment.utc(value).startOf("day");
  }

  function applyAverage(value, start, end) {
    var count = end.diff(start, "day") + 1;
    return parseFloat((value / count).toFixed(2));
  }

  // Report.collapse
  function collapse(model, data, grouping) {
    grouping = grouping || groupingFor(data.length);
    if (grouping !== "weekly" && grouping !== "monthly") {
      return data;
    }
    var isoKind = grouping === "weekly" ? "isoWeek" : "month";
    var kind = grouping === "weekly" ? "week" : "month";
    var selectedStart = dateMoment(model.start_date);
    var selectedEnd = dateMoment(model.end_date);
    var index = 0;
    var currentStart = selectedStart.clone().startOf(isoKind);
    var currentEnd = selectedStart.clone().endOf(isoKind);
    function bucketStart() {
      return moment.max(currentStart, selectedStart);
    }
    function bucketEnd() {
      return moment.min(currentEnd, selectedEnd);
    }
    function bucket() {
      return {
        x: bucketStart().format("YYYY-MM-DD"),
        y: 0,
        end_date: bucketEnd().format("YYYY-MM-DD"),
      };
    }
    var out = [bucket()];
    var appliedAverage = false;
    data.forEach(function (d) {
      var date = dateMoment(d.x);
      while (date.isAfter(currentEnd)) {
        if (model.average) {
          out[index].y = applyAverage(out[index].y, bucketStart(), bucketEnd());
          appliedAverage = true;
        }
        index += 1;
        currentStart = currentStart.add(1, kind).startOf(isoKind);
        currentEnd = currentEnd.add(1, kind).endOf(isoKind);
        out[index] = bucket();
      }
      out[index].y += d.y;
      appliedAverage = false;
    });
    if (model.average && !appliedAverage) {
      out[index].y = applyAverage(out[index].y, bucketStart(), bucketEnd());
    }
    return out;
  }

  function makeArray(v) {
    if (v === null || v === undefined) {
      return [];
    }
    return Array.isArray(v) ? v : [v];
  }

  function tooltipStyle() {
    return {
      backgroundColor: cssColor("--primary"),
      titleColor: cssColor("--secondary"),
      bodyColor: cssColor("--secondary"),
      footerColor: cssColor("--secondary"),
      titleMarginBottom: 16,
      footerMarginTop: 16,
      padding: { left: 20, right: 20, top: 12, bottom: 12 },
      bodySpacing: 8,
      cornerRadius: 8,
      boxPadding: 4,
    };
  }

  function draw(canvas, config) {
    if (!window.Chart) {
      return null;
    }
    return new window.Chart(canvas.getContext("2d"), config);
  }

  // AdminReportChart: the report's line, its last point dotted while its
  // period is still running, and the previous period dotted beside it.
  function lineChart(canvas, model, grouping) {
    var chartData = collapse(model, makeArray(model.chartData || model.data), grouping);
    var prev = makeArray(model.prevChartData || model.prev_data);
    var last = chartData.length - 1;
    var lastPoint = chartData[last];
    var now = moment.utc();
    var running =
      !!lastPoint &&
      moment.utc(lastPoint.x).isSame(now, grouping === "weekly" ? "week" : grouping === "monthly" ? "month" : "day");
    var incomplete = cssColor("--primary-medium");
    var rgb = cssColor("--tertiary-rgb");
    var primary = "rgb(" + rgb + ")";
    var secondary = "rgba(" + rgb + ", 0.1)";
    var pointColors = primary;
    var segment;
    if (running) {
      pointColors = chartData.map(function () {
        return primary;
      });
      pointColors[last] = incomplete;
      segment = {
        borderDash: function (ctx) {
          return ctx.p1DataIndex === last ? DOTTED_LINE : [];
        },
        borderColor: function (ctx) {
          return ctx.p1DataIndex === last ? incomplete : primary;
        },
      };
    }
    var datasets = [
      {
        data: chartData.map(function (d) {
          return Math.round(parseFloat(d.y));
        }),
        backgroundColor: prev.length ? "transparent" : secondary,
        borderColor: primary,
        pointRadius: 3,
        borderWidth: 2,
        pointBackgroundColor: pointColors,
        pointBorderColor: pointColors,
        pointStyle: "rectRounded",
        borderCapStyle: "round",
        borderJoinStyle: "round",
        segment: segment,
        tension: 0.4,
        fill: "origin",
      },
    ];
    if (prev.length) {
      datasets.push({
        data: prev.map(function (d) {
          return Math.round(parseFloat(d.y));
        }),
        borderColor: primary,
        borderDash: DOTTED_LINE,
        backgroundColor: "transparent",
        borderWidth: 1,
        pointRadius: 0,
        fill: false,
      });
    }
    return draw(canvas, {
      type: "line",
      data: {
        labels: chartData.map(function (d) {
          return d.x;
        }),
        datasets: datasets,
      },
      options: {
        plugins: {
          tooltip: Object.assign(tooltipStyle(), {
            callbacks: {
              title: function (items) {
                return moment(items[0].parsed.x).format("LL");
              },
              label: function (item) {
                var value = item.formattedValue;
                if (running && item.dataIndex === last) {
                  return t("admin.dashboard.reports.value_so_far", { value: value });
                }
                return value;
              },
            },
          }),
          legend: { display: false },
        },
        responsive: true,
        maintainAspectRatio: false,
        responsiveAnimationDuration: 300,
        animation: { duration: 300 },
        layout: { padding: { left: 0, top: 0, right: 0, bottom: 0 } },
        scales: {
          y: {
            display: true,
            title: { display: true, text: model.y_axis_title },
            grid: { color: cssColor("--primary-low") },
            ticks: {
              callback: function (label) {
                return number(label);
              },
              sampleSize: 5,
              maxRotation: 25,
              minRotation: 0,
              precision: 0,
            },
          },
          x: {
            display: true,
            grid: { display: false },
            type: "time",
            time: { unit: unitFor(grouping) },
            ticks: { sampleSize: 5, maxRotation: 50, minRotation: 0 },
          },
        },
      },
    });
  }

  function hexToRgba(hex, alpha) {
    var r = parseInt(hex.slice(1, 3), 16);
    var g = parseInt(hex.slice(3, 5), 16);
    var b = parseInt(hex.slice(5, 7), 16);
    return "rgba(" + r + ", " + g + ", " + b + ", " + alpha + ")";
  }

  var gradientPlugin = {
    id: "gradientPlugin",
    beforeDatasetsDraw: function (chart) {
      var ctx = chart.ctx;
      chart.data.datasets.forEach(function (dataset, i) {
        if (!dataset._baseColor) {
          return;
        }
        var meta = chart.getDatasetMeta(i);
        if (!meta.data.length) {
          return;
        }
        dataset.backgroundColor = meta.data.map(function (bar) {
          var p = bar.getProps(["x", "y", "width", "base"], true);
          if (!p.x || !p.y || !p.width || !p.base) {
            return undefined;
          }
          var gradient = ctx.createLinearGradient(p.x - p.width / 2, p.y, p.x + p.width / 2, p.base);
          gradient.addColorStop(0, dataset._baseColor);
          gradient.addColorStop(1, hexToRgba(dataset._baseColor, 0.7));
          return gradient;
        });
      });
    },
  };

  var emptyTooltipPlugin = {
    id: "emptyTooltipPlugin",
    beforeDraw: function (chart) {
      var tooltip = chart.tooltip;
      if (!tooltip || tooltip.opacity === 0) {
        return;
      }
      var allZero = (tooltip.dataPoints || []).every(function (dp) {
        return !dp.parsed.y;
      });
      if (allZero) {
        tooltip.opacity = 0;
      }
    },
  };

  // chart-legend-icon's dimColor and buildLegendIcon
  function dimColor(color) {
    var value = (color || "").trim();
    if (!value) {
      return color;
    }
    if (value[0] === "#") {
      var hex = value.slice(1);
      if (hex.length === 3) {
        hex = hex
          .split("")
          .map(function (c) {
            return c + c;
          })
          .join("");
      }
      return hexToRgba("#" + hex, 0.6);
    }
    var m = value.match(/^rgba?\(([^)]+)\)$/);
    if (m) {
      var p = m[1].split(",").map(function (s) {
        return s.trim();
      });
      return "rgba(" + p[0] + ", " + p[1] + ", " + p[2] + ", 0.6)";
    }
    return value;
  }

  function legendIcon(color, visible) {
    var size = remToPx(1);
    var canvas = document.createElement("canvas");
    canvas.width = size;
    canvas.height = size;
    var ctx = canvas.getContext("2d");
    ctx.globalAlpha = visible ? 1 : 0.6;
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.roundRect(1, 1, size - 2, size - 2, 4);
    ctx.stroke();
    if (visible) {
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.roundRect(4, 4, size - 8, size - 8, 2);
      ctx.fill();
    }
    return canvas;
  }

  function labelMoment(value) {
    return typeof value === "string" ? moment.utc(value, "YYYY-MM-DD") : moment(value);
  }

  // AdminReportStackedChart
  function stackedChart(canvas, model, options) {
    options = options || {};
    var hidden = options.hiddenLabels || [];
    var source = makeArray(model.chartData || model.data);
    var grouping =
      options.chartGrouping || groupingFor((source[0] && source[0].data && source[0].data.length) || 0);
    var chartData = source.map(function (series) {
      return {
        label: series.label,
        color: series.color,
        data: collapse(model, series.data, grouping),
        req: series.req,
      };
    });
    var labels = chartData[0]
      ? chartData[0].data.map(function (p) {
          return p.x;
        })
      : [];
    var unit = unitFor(grouping);
    var datasets = chartData.map(function (series) {
      return {
        label: series.label,
        data: series.data,
        backgroundColor: series.color,
        _baseColor: series.color,
        hidden: hidden.indexOf(series.req) !== -1,
        borderRadius: 2,
        maxBarThickness: 30,
      };
    });
    var reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    return draw(canvas, {
      type: "bar",
      data: { labels: labels, datasets: datasets },
      plugins: [gradientPlugin, emptyTooltipPlugin],
      options: {
        responsive: true,
        maintainAspectRatio: false,
        hover: { mode: "index" },
        animation: { duration: reduced ? 0 : 300 },
        plugins: {
          legend: {
            display: datasets.length > 1,
            position: options.legendPosition || "bottom",
            onClick: function (e, item, legend) {
              var ci = legend.chart;
              var index = item.datasetIndex;
              var req = chartData[index].req;
              if (ci.isDatasetVisible(index)) {
                ci.hide(index);
                if (hidden.indexOf(req) === -1) {
                  hidden.push(req);
                }
              } else {
                ci.show(index);
                var at = hidden.indexOf(req);
                if (at !== -1) {
                  hidden.splice(at, 1);
                }
              }
            },
            labels: {
              usePointStyle: true,
              padding: remToPx(1),
              font: { size: remToPx(0.75) },
              generateLabels: function (chart) {
                var color = cssColor("--primary-high");
                return chart.data.datasets.map(function (dataset, i) {
                  var visible = chart.isDatasetVisible(i);
                  return {
                    text: dataset.label,
                    fontColor: visible ? color : dimColor(color),
                    hidden: false,
                    datasetIndex: i,
                    pointStyle: legendIcon(dataset._baseColor, visible),
                  };
                });
              },
            },
          },
          tooltip: Object.assign(tooltipStyle(), {
            mode: "index",
            intersect: false,
            callbacks: {
              beforeFooter: function (items) {
                var total = items.reduce(function (sum, item) {
                  return sum + parseInt(item.parsed.y || 0, 10);
                }, 0);
                return tCount("admin.reports.chart.total", total, { count: toNumber(total, 0) });
              },
              title: function (items) {
                var point = items[0].raw;
                var start = point && point.x !== undefined ? point.x : items[0].parsed.x;
                if (point && point.end_date) {
                  var s = labelMoment(start);
                  var e = labelMoment(point.end_date);
                  return s.isSame(e, "day") ? s.format("LL") : s.format("ll") + " - " + e.format("ll");
                }
                return labelMoment(start).format("LL");
              },
            },
          }),
        },
        layout: { padding: { left: 0, top: 0, right: 0, bottom: 0 } },
        scales: {
          y: {
            stacked: true,
            display: true,
            grid: { color: cssColor("--primary-very-low"), display: !options.hideYAxisGridLines },
            ticks: {
              callback: function (_label, index, ticks) {
                var value = ticks[index] && ticks[index].value;
                return Number.isInteger(Number(value)) ? number(value) : null;
              },
              sampleSize: 5,
              maxRotation: 25,
              minRotation: 0,
            },
          },
          x: {
            stacked: true,
            display: true,
            grid: { display: false },
            type: "category",
            ticks: {
              callback: function (value) {
                var label = labels[value] !== undefined ? labels[value] : value;
                var date = labelMoment(label);
                return unit === "month" ? date.format("MMM YYYY") : date.format("D MMM");
              },
              sampleSize: 5,
              maxRotation: 50,
              minRotation: 0,
            },
          },
        },
      },
    });
  }

  // Report.fillMissingDates for a payload
  function fillReport(report) {
    var start = moment.utc(report.start_date).locale("en").format("YYYY-MM-DD");
    var end = moment.utc(report.end_date).locale("en").format("YYYY-MM-DD");
    var mode = (report.modes || [])[0];
    if (Array.isArray(report.data)) {
      if (mode === "stacked_chart" || mode === "stacked_line_chart") {
        report.chartData = report.data.map(function (series) {
          return {
            req: series.req,
            label: series.label,
            color: series.color,
            data: fillMissingDates(JSON.parse(JSON.stringify(series.data)), start, end),
          };
        });
      } else if (mode !== "radar") {
        report.chartData = fillMissingDates(JSON.parse(JSON.stringify(report.data)), start, end);
      }
    }
    // _loadReport's options name the fields `starDate` and, for the end,
    // the value of the absent `prev_endDate`, so the report's own range
    // is the one filled.
    if (Array.isArray(report.prev_data)) {
      report.prevChartData = fillMissingDates(JSON.parse(JSON.stringify(report.prev_data)), start, end);
    }
    return report;
  }

  // Report.unitForDatapoints
  function unitForDatapoints(count) {
    if (count >= DAILY_LIMIT_DAYS && count < WEEKLY_LIMIT_DAYS) {
      return "week";
    }
    return count >= WEEKLY_LIMIT_DAYS ? "month" : "day";
  }

  function seriesData(model) {
    return makeArray(model.chartData || model.data).map(function (series) {
      return { label: series.label, color: series.color, data: collapse(model, series.data) };
    });
  }

  // AdminReportStackedLineChart
  function stackedLineChart(canvas, model) {
    var chartData = seriesData(model);
    return draw(canvas, {
      type: "line",
      data: {
        labels: chartData[0].data.map(function (p) {
          return p.x;
        }),
        datasets: chartData.map(function (series) {
          return {
            label: series.label,
            stack: "pageviews-stack",
            data: series.data,
            fill: true,
            backgroundColor: hexToRgba(series.color, 0.3),
            borderColor: series.color,
            pointBackgroundColor: series.color,
            pointBorderColor: "#fff",
            pointHoverBackgroundColor: "#fff",
            pointHoverBorderColor: series.color,
          };
        }),
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        responsiveAnimationDuration: 0,
        hover: { mode: "index" },
        animation: { duration: 0 },
        plugins: {
          tooltip: {
            mode: "index",
            intersect: false,
            callbacks: {
              beforeFooter: function (items) {
                var total = items.reduce(function (sum, item) {
                  return sum + parseInt(item.parsed.y || 0, 10);
                }, 0);
                return "= " + total;
              },
              title: function (items) {
                return moment(items[0].parsed.x).format("LL");
              },
            },
          },
        },
        layout: { padding: { left: 0, top: 0, right: 0, bottom: 0 } },
        scales: {
          y: {
            stacked: true,
            display: true,
            ticks: {
              callback: function (label) {
                return number(label);
              },
              sampleSize: 5,
              maxRotation: 25,
              minRotation: 25,
            },
          },
          x: {
            stacked: true,
            display: true,
            grid: { display: false },
            type: "time",
            time: { unit: unitForDatapoints(chartData[0].data.length) },
            ticks: { sampleSize: 5, maxRotation: 50, minRotation: 50 },
          },
        },
      },
    });
  }

  // AdminReportRadar
  function radarChart(canvas, model) {
    var chartData = seriesData(model);
    return draw(canvas, {
      type: "radar",
      data: {
        labels: chartData[0].data.map(function (p) {
          return p.x;
        }),
        datasets: chartData.map(function (series) {
          return {
            label: series.label,
            data: series.data.map(function (p) {
              return p.y;
            }),
            fill: true,
            backgroundColor: hexToRgba(series.color, 0.3),
            borderColor: series.color,
            pointBackgroundColor: series.color,
            pointBorderColor: "#fff",
            pointHoverBackgroundColor: "#fff",
            pointHoverBorderColor: series.color,
          };
        }),
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        responsiveAnimationDuration: 0,
        hover: { mode: "index" },
        animation: { duration: 0 },
        plugins: {
          tooltip: {
            mode: "index",
            intersect: false,
            callbacks: {
              // reduce without a start value, as the component has it.
              beforeFooter: function (items) {
                var total = items.reduce(function (sum, item) {
                  return sum + parseInt(item.parsed.r || 0, 10);
                });
                return "= " + total;
              },
            },
          },
        },
      },
    });
  }

  Discourse.reportCharts = {
    DAILY_LIMIT_DAYS: DAILY_LIMIT_DAYS,
    setStrings: function (more) {
      Object.assign(strings, more || {});
    },
    toNumber: toNumber,
    number: number,
    groupingFor: groupingFor,
    unitFor: unitFor,
    unitForDatapoints: unitForDatapoints,
    fillMissingDates: fillMissingDates,
    collapse: collapse,
    makeArray: makeArray,
    fillReport: fillReport,
    lineChart: lineChart,
    stackedChart: stackedChart,
    stackedLineChart: stackedLineChart,
    radarChart: radarChart,
  };
})();
