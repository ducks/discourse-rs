//! The admin dashboard as the Ember client renders it (templates/admin/
//! dashboard.gjs with RedesignedAdminDashboard and components/dashboard/*)
//! from the document /admin/dashboard.json serves: the page header with
//! the period picker and the configure menu, the site advice (problems),
//! then the sections in their configured order. The charts (the pinned
//! reports, the site traffic) are drawn in the browser from their data by
//! static/js/admin-dashboard.js, as Ember draws them with Chart.js.
//!
//! Not drawn: plugin sections (lookupAdminDashboardSection) and plugin
//! outlets, which the bundled plugins the port runs leave empty.

use chrono::{Datelike, NaiveDate};
use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t, t_count, t_with};
use crate::topic_list_view::{ListContext, escape, icon};
use crate::{AppError, Unsupported};

pub const PERIODS: &[&str] = &[
    "last_7_days",
    "last_30_days",
    "last_3_months",
    "last_6_months",
    "last_year",
];

/// The controller's `range`, `start_date` and `end_date` query params as
/// the dashboard reads them (safePeriod, startDate, endDate), in days.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub period: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Range {
    /// `safePeriod` and the dates it gives on `today`: an unknown range,
    /// or custom without both valid dates, is the last 30 days.
    pub fn from_params(
        range: Option<&str>,
        start: Option<&str>,
        end: Option<&str>,
        today: NaiveDate,
    ) -> Range {
        let parse = |v: Option<&str>| v.and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
        let period = match range {
            Some(r) if PERIODS.contains(&r) => r,
            Some("custom") if parse(start).is_some() && parse(end).is_some() => "custom",
            _ => "last_30_days",
        };
        if period == "custom" {
            return Range {
                period: period.into(),
                start: parse(start).unwrap_or(today),
                end: parse(end).unwrap_or(today),
            };
        }
        let months_back = |n: u32| {
            today
                .checked_sub_months(chrono::Months::new(n))
                .unwrap_or(today)
                + chrono::Duration::days(1)
        };
        let start = match period {
            "last_7_days" => today - chrono::Duration::days(6),
            "last_3_months" => months_back(3),
            "last_6_months" => months_back(6),
            "last_year" => months_back(12),
            _ => today - chrono::Duration::days(29),
        };
        Range {
            period: period.into(),
            start,
            end: today,
        }
    }

    fn query(&self) -> String {
        format!(
            "end_date={}&start_date={}",
            self.end.format("%Y-%m-%d"),
            self.start.format("%Y-%m-%d")
        )
    }
}

/// JavaScript's `Number#toFixed`: the nearest decimal with `digits`
/// places, a tie going away from zero.
pub fn to_fixed(value: f64, digits: usize) -> String {
    // The exact decimal expansion far past the places kept, rounded half up
    // by hand: Rust's own rounding sends ties to even.
    const EXTRA: usize = 30;
    let expanded = format!("{:.*}", digits + EXTRA, value.abs());
    let (kept, rest) = expanded.split_at(expanded.len() - EXTRA);
    let mut out: Vec<u8> = kept.trim_end_matches('.').bytes().collect();
    if rest.as_bytes()[0] >= b'5' {
        let mut i = out.len();
        loop {
            if i == 0 {
                out.insert(0, b'1');
                break;
            }
            i -= 1;
            match out[i] {
                b'.' => continue,
                b'9' => out[i] = b'0',
                d => {
                    out[i] = d + 1;
                    break;
                }
            }
        }
    }
    let s = String::from_utf8(out).unwrap_or_default();
    if value < 0.0 { format!("-{s}") } else { s }
}

/// `I18n.toNumber(value, { precision })`: thousands delimited by commas.
pub fn to_number(value: f64, precision: usize) -> String {
    let fixed = to_fixed(value.abs(), precision);
    let (int, frac) = match fixed.split_once('.') {
        Some((i, f)) => (i.to_string(), Some(f.to_string())),
        None => (fixed.clone(), None),
    };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if let Some(f) = frac {
        grouped.push('.');
        grouped.push_str(&f);
    }
    if value < 0.0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// `I18n.toHumanSize`: bytes in the largest binary unit below them, one
/// decimal unless whole.
pub fn to_human_size(cx: &AdminCx, bytes: f64) -> String {
    let mut size = bytes;
    let mut iterations = 0;
    while size >= 1024.0 && iterations < 4 {
        size /= 1024.0;
        iterations += 1;
    }
    let (unit, precision) = if iterations == 0 {
        (
            t_count(
                cx.i18n,
                "number.human.storage_units.units.byte",
                size as i64,
                &[],
            ),
            0,
        )
    } else {
        let unit = ["", "kb", "mb", "gb", "tb"][iterations];
        (
            t(cx.i18n, &format!("number.human.storage_units.units.{unit}")),
            if size.fract() == 0.0 { 0 } else { 1 },
        )
    };
    let format = t(cx.i18n, "number.human.storage_units.format");
    let number = to_fixed(size, precision);
    format.replace("%u", &unit).replace("%n", &number)
}

/// formatKpiValue
fn format_kpi_value(value: &Value, percentage: bool) -> String {
    match value.as_f64() {
        None => "—".into(),
        Some(v) if percentage => format!("{}%", to_number(v, 1)),
        Some(v) => to_number(v, 0),
    }
}

/// roundDeltaPercent
fn round_delta_percent(value: f64) -> f64 {
    let abs = value.abs();
    if abs > 0.0 && abs < 1.0 {
        let rounded = (abs * 10.0).round() / 10.0;
        return if value < 0.0 { -rounded } else { rounded };
    }
    js_round(value)
}

/// `Math.round`: halves toward positive infinity.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// formatDeltaPercent
fn format_delta_percent(value: f64) -> String {
    let rounded = round_delta_percent(value);
    if value.abs() > 0.0 && value.abs() < 1.0 {
        let sign = if value > 0.0 { "+" } else { "-" };
        return format!("{sign}{}%", to_number(rounded.abs(), 1));
    }
    let sign = if rounded > 0.0 { "+" } else { "" };
    format!("{sign}{}%", to_number(rounded, 0))
}

/// DTooltip's trigger with an icon.
fn tooltip(class: &str, identifier: Option<&str>, content: &str) -> String {
    let identifier = identifier
        .map(|i| format!(" data-identifier=\"{}\"", escape(i)))
        .unwrap_or_default();
    format!(
        " <span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger {class}\"{identifier} data-trigger=\"\" role=\"button\" data-tooltip-content=\"{}\"><span class=\"fk-d-tooltip__trigger-container\"><span class=\"fk-d-tooltip__icon\">{}</span></span></span>",
        escape(content),
        icon("far-circle-question", None)
    )
}

/// DButton with an icon and a label.
fn button(class: &str, icon_name: &str, label: &str, attrs: &str) -> String {
    format!(
        "<button class=\"btn btn-icon-text {class}\"{attrs} type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        icon(icon_name, None),
        escape(label)
    )
}

/// DashboardSection
fn section(
    id: &str,
    title: &str,
    layout: &str,
    bordered: bool,
    header_action: &str,
    description: Option<&str>,
    body: &str,
) -> String {
    let mut out = format!(
        "<section class=\"db-section --{id}\" data-section-id=\"{id}\"><div class=\"db-section__header-row\"><h2 class=\"db-section__header\">{}</h2>{header_action}</div>",
        escape(title)
    );
    if let Some(description) = description {
        out.push_str(&format!(
            "<p class=\"db-section__intro\">{}</p>",
            escape(description)
        ));
    }
    out.push_str(&format!(
        "<div class=\"db-section__wrapper --{layout}{}\">{body}</div></section>",
        if bordered { "" } else { " --no-border" }
    ));
    out
}

pub struct Dashboard<'a> {
    pub admin: &'a AdminCx<'a>,
    pub list: &'a ListContext<'a>,
    pub range: &'a Range,
    /// The document /admin/dashboard.json serves for the range.
    pub doc: &'a Value,
    /// `!SiteSetting.use_legacy_pageviews`, for the traffic's details link.
    pub use_legacy_pageviews: bool,
}

impl Dashboard<'_> {
    fn t(&self, key: &str) -> String {
        t(self.admin.i18n, key)
    }

    /// The page: DPageHeader with its breadcrumbs and actions, then
    /// `db-main`.
    pub fn render(&self) -> Result<String, AppError> {
        let title = self.t("admin.dashboard.title");
        let mut actions = self.date_range_trigger();
        if self.admin.viewer.admin {
            actions.push_str(&format!(
                "<button aria-expanded=\"false\" class=\"btn btn-icon-text fk-d-menu__trigger db-configure-trigger btn-default\" title=\"{}\" data-identifier=\"db-configure\" data-trigger=\"\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                escape(&self.t("admin.dashboard.configure.tooltip")),
                icon("gear", None),
                escape(&self.t("admin.dashboard.configure.button"))
            ));
        }
        let header = PageHeader {
            crumbs: vec![
                Crumb {
                    label: self.t("admin_title"),
                    path: "/admin",
                },
                Crumb {
                    label: title.clone(),
                    path: "/admin",
                },
            ],
            title,
            actions,
            description: None,
            learn_more_url: None,
            tabs: None,
        };
        let mut main = String::from("<div class=\"db-main\">");
        main.push_str(&self.site_advice());
        let empty = Vec::new();
        let sections = self.doc["sections"].as_array().unwrap_or(&empty);
        for s in sections {
            let id = s["id"].as_str().unwrap_or_default();
            let data = &s["data"];
            let error = s["error"].as_bool().unwrap_or(false) || !s["error"].is_null();
            main.push_str(&match id {
                "highlights" => self.highlights(data, error),
                "reports" => self.reports(data, error),
                "traffic" => self.traffic(data, error)?,
                "engagement" => self.engagement(data, error),
                "search" => self.search(data, error),
                "system" => self.system(data, error),
                // A plugin's section (lookupAdminDashboardSection): none of
                // the plugins the port runs registers one.
                _ => return Err(Unsupported("admin dashboard sections from plugins").into()),
            });
        }
        if sections.is_empty() {
            let key = if self.admin.viewer.admin {
                "admin.dashboard.configure.empty_state_admin"
            } else {
                "admin.dashboard.configure.empty_state_moderator"
            };
            main.push_str(&format!(
                "<div aria-live=\"polite\" class=\"db-main__empty\" role=\"status\">{}</div>",
                escape(&self.t(key))
            ));
        }
        main.push_str("</div>");
        if self.admin.viewer.admin {
            main.push_str(&self.configure_menu());
        }
        main.push_str(&format!(
            "<script type=\"application/json\" id=\"db-strings\">{}</script>",
            self.client_strings().replace('<', "\\u003c")
        ));
        Ok(format!("{}{main}", header.render(self.admin)))
    }

    /// The strings admin-dashboard.js renders with, as `i18n` resolves
    /// them for staff, placeholders intact.
    fn client_strings(&self) -> String {
        let mut keys: Vec<String> = [
            "admin.dashboard.reports_section.no_data",
            "admin.dashboard.reports_section.report_error",
            "admin.dashboard.reports.value_so_far",
            "admin.reports.chart.total.one",
            "admin.reports.chart.total.other",
            "number.short.thousands",
            "number.short.millions",
            "admin.dashboard.site_advice.title.one",
            "admin.dashboard.site_advice.title.other",
            "admin.dashboard.dismiss_notice",
            "date_range_picker.presets.label",
            "date_range_picker.start_date",
            "date_range_picker.end_date",
            "date_range_picker.date_placeholder",
            "date_range_picker.apply",
            "dates.previous_month",
            "dates.next_month",
            "cancel",
        ]
        .iter()
        .map(|k| k.to_string())
        .collect();
        keys.sort();
        let map: serde_json::Map<String, Value> = keys
            .into_iter()
            .map(|k| {
                let raw = self
                    .admin
                    .i18n
                    .t(&format!("js.{k}"))
                    .or_else(|| self.admin.i18n.t(&format!("admin_js.{k}")))
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("[en.{k}]"));
                (k, Value::String(raw))
            })
            .collect();
        Value::Object(map).to_string()
    }

    /// ConfigureMenu's content, which admin-dashboard.js opens in the
    /// Configure trigger's DMenu: a row per section, a drag handle on the
    /// desktop and arrows in the mobile view, and its visibility toggle.
    fn configure_menu(&self) -> String {
        let i18n = self.admin.i18n;
        let empty = Vec::new();
        let sections = self.doc["configuration"]["sections"]
            .as_array()
            .unwrap_or(&empty);
        let mobile = crate::mobile::mobile_view();
        let last = sections.len().saturating_sub(1);
        let mut out = format!(
            "<template id=\"db-configure-content\"><div class=\"db-configure\"><ul aria-label=\"{}\" class=\"db-configure__list\">",
            escape(&t(i18n, "admin.dashboard.configure.menu_title"))
        );
        for (i, s) in sections.iter().enumerate() {
            let id = s["id"].as_str().unwrap_or_default();
            let label = t(i18n, &format!("admin.dashboard.sections.{id}.title"));
            let visible = s["visible"].as_bool().unwrap_or(false);
            out.push_str(&format!(
                "<li class=\"db-configure__row\" data-section-id=\"{}\" draggable=\"true\">",
                escape(id)
            ));
            if mobile {
                let arrow = |direction: &str, disabled: bool, icon_name: &str, key: &str| {
                    let title = escape(&t_with(i18n, key, &[("label", &label)]));
                    format!(
                        "<button class=\"btn no-text btn-icon btn-flat db-configure__arrow\" aria-label=\"{title}\" title=\"{title}\"{} type=\"button\" data-direction=\"{direction}\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
                        if disabled { " disabled" } else { "" },
                        icon(icon_name, None)
                    )
                };
                out.push_str(&format!(
                    "<span class=\"db-configure__arrows\">{}{}</span>",
                    arrow(
                        "up",
                        i == 0,
                        "chevron-up",
                        "admin.dashboard.configure.reorder_up"
                    ),
                    arrow(
                        "down",
                        i == last,
                        "chevron-down",
                        "admin.dashboard.configure.reorder_down"
                    )
                ));
            } else {
                out.push_str(&format!(
                    "<span aria-hidden=\"true\" class=\"db-configure__drag-handle\" tabindex=\"-1\" title=\"{}\">{}</span>",
                    escape(&t_with(
                        i18n,
                        "admin.dashboard.configure.drag_handle",
                        &[("label", &label)]
                    )),
                    icon("grip-vertical", None)
                ));
            }
            out.push_str(&format!(
                "<span class=\"db-configure__section-name\">{}</span><div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{visible}\" class=\"d-toggle-switch__checkbox\" role=\"switch\" type=\"button\" aria-label=\"{}\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div></li>",
                escape(&label),
                escape(&t_with(
                    i18n,
                    "admin.dashboard.configure.toggle_visibility",
                    &[("label", &label)]
                )),
                if visible { icon("check", None) } else { String::new() }
            ));
        }
        out.push_str("</ul></div></template>");
        out
    }

    /// DashboardDateRange's DMenu trigger, with what admin-dashboard.js
    /// fills its menu from.
    fn date_range_trigger(&self) -> String {
        date_range_trigger(self.admin, self.range)
    }

    /// DashboardSiteAdvice: the problems by priority, then id.
    fn site_advice(&self) -> String {
        let empty = Vec::new();
        let mut problems: Vec<&Value> = self.doc["problems"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .collect();
        if problems.is_empty() {
            return String::new();
        }
        problems.sort_by(|a, b| {
            let pa = a["priority"].as_str().unwrap_or_default();
            let pb = b["priority"].as_str().unwrap_or_default();
            pa.cmp(pb).then_with(|| {
                a["id"]
                    .as_i64()
                    .unwrap_or(0)
                    .cmp(&b["id"].as_i64().unwrap_or(0))
            })
        });
        let mut out = format!(
            "<section class=\"db-site-advice\" data-admin=\"{}\"><div class=\"db-site-advice__header\"><h3>{}</h3>{}</div><ul class=\"db-site-advice__list\">",
            self.admin.viewer.admin,
            escape(&t_count(
                self.admin.i18n,
                "admin.dashboard.site_advice.title",
                problems.len() as i64,
                &[]
            )),
            button(
                "btn-transparent btn-small",
                "arrows-rotate",
                &self.t("admin.dashboard.refresh_problems"),
                " data-test-site-advice-refresh=\"true\""
            )
        );
        for p in problems {
            let priority = p["priority"].as_str().unwrap_or_default();
            let id = p["id"].as_i64().unwrap_or(0);
            out.push_str(&format!(
                "<li class=\"db-site-advice__item\" data-test-site-advice-problem=\"{id}\">{}<div class=\"db-site-advice__message\">{}</div>",
                icon(
                    if priority == "high" {
                        "triangle-exclamation"
                    } else {
                        "circle-info"
                    },
                    Some(&format!("db-site-advice__icon --{}", escape(priority)))
                ),
                p["message"].as_str().unwrap_or_default()
            ));
            if self.admin.viewer.admin {
                out.push_str(&format!(
                    "<button class=\"btn btn-default btn-small\" data-test-site-advice-ignore=\"true\" data-problem-id=\"{id}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
                    escape(&self.t("admin.dashboard.dismiss_notice"))
                ));
            }
            out.push_str("</li>");
        }
        out.push_str("</ul></section>");
        out
    }

    fn report_href(&self, report: &str, query: Option<&Value>) -> String {
        let query = match query {
            Some(q) => format!(
                "end_date={}&start_date={}",
                q["end_date"].as_str().unwrap_or_default(),
                q["start_date"].as_str().unwrap_or_default()
            ),
            None => self.range.query(),
        };
        format!(
            "{}/admin/reports/{}?{}",
            self.admin.base_path,
            escape(report),
            escape(&query)
        )
    }

    /// Highlights with its KpiTiles.
    fn highlights(&self, data: &Value, error: bool) -> String {
        let mut body = String::new();
        if error {
            body.push_str(&self.error("admin.dashboard.highlights.fetch_error"));
        } else {
            let comparison = if ["last_7_days", "last_30_days", "last_3_months"]
                .contains(&self.range.period.as_str())
            {
                self.t(&format!(
                    "admin.dashboard.highlights.comparison.{}",
                    self.range.period
                ))
            } else {
                self.t("admin.dashboard.highlights.comparison.previous_period")
            };
            for kpi in data["kpis"].as_array().into_iter().flatten() {
                body.push_str(&self.kpi_tile(kpi, &comparison));
            }
        }
        section(
            "highlights",
            &self.t("admin.dashboard.sections.highlights.title"),
            "row",
            true,
            "",
            None,
            &body,
        )
    }

    fn kpi_tile(&self, kpi: &Value, comparison: &str) -> String {
        let kind = kpi["type"].as_str().unwrap_or_default();
        let value = format_kpi_value(&kpi["value"], kind == "dau_mau");
        let label = self.t(&format!("admin.dashboard.highlights.kpi.{kind}.label"));
        let tip = self.t(&format!("admin.dashboard.highlights.kpi.{kind}.tooltip"));
        let mut aria = vec![label.clone(), value.clone()];
        let delta = kpi["percent_change"].as_f64().map(|change| {
            let text = format_delta_percent(change);
            aria.push(if comparison.is_empty() {
                text.clone()
            } else {
                format!("{text} {comparison}")
            });
            format!(
                "<div class=\"db-delta {}\">{}</div>",
                if change >= 0.0 { "--pos" } else { "--neg" },
                escape(&text)
            )
        });
        format!(
            "<a class=\"db-kpi db-section__row-block\" aria-label=\"{}\" href=\"{}\"><div class=\"db-kpi__value\">{}</div><div class=\"db-kpi__label\">{}{}</div>{}<span aria-hidden=\"true\" class=\"db-link-arrow\">{}</span></a>",
            escape(&aria.join(", ")),
            self.report_href(
                kpi["report_type"].as_str().unwrap_or_default(),
                Some(&kpi["report_query"])
            ),
            escape(&value),
            escape(&label),
            tooltip("db-kpi__tooltip", None, &tip),
            delta.unwrap_or_default(),
            icon("arrow-right", None)
        )
    }

    fn error(&self, key: &str) -> String {
        format!(
            "<div class=\"db-section__error\" role=\"alert\">{}</div>",
            escape(&self.t(key))
        )
    }

    /// DashboardReports: the pinned reports' cards, whose charts
    /// admin-dashboard.js loads (loadDashboardReports) and draws.
    fn reports(&self, data: &Value, error: bool) -> String {
        let can_edit = self.admin.viewer.admin;
        let mut body = String::new();
        if error {
            body.push_str(&self.error("admin.dashboard.sections.reports.fetch_error"));
        } else {
            let empty = Vec::new();
            let items = data["items"].as_array().unwrap_or(&empty);
            let layout: Vec<Value> = items
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "source": i["source"],
                        "identifier": i["identifier"],
                        "rows": i["rows"].as_i64().filter(|r| *r > 0).unwrap_or(1),
                        "cols": i["cols"].as_i64().filter(|c| *c > 0).unwrap_or(1),
                    })
                })
                .collect();
            body.push_str(&format!(
                "<div class=\"db-reports\" data-items=\"{}\" data-filters=\"{}\">",
                escape(&Value::Array(layout).to_string()),
                escape(
                    &serde_json::json!({
                        "start_date": self.range.start.format("%Y-%m-%d").to_string(),
                        "end_date": self.range.end.format("%Y-%m-%d").to_string(),
                    })
                    .to_string()
                )
            ));
            // computeColumns: a wide card starts a row; others alternate.
            let mut cursor = 0;
            for item in items {
                let rows = item["rows"].as_i64().unwrap_or(1).max(1);
                let cols = item["cols"].as_i64().unwrap_or(1).max(1);
                let column = if cols > 1 {
                    cursor = 0;
                    None
                } else {
                    let c = cursor;
                    cursor = if cursor == 0 { 1 } else { 0 };
                    Some(c)
                };
                let mut classes = String::from("db-report__card");
                if cols > 1 {
                    classes.push_str(" --wide");
                }
                if rows > 1 {
                    classes.push_str(&format!(" --rows-{rows}"));
                }
                let key = item["key"].as_str().unwrap_or_default();
                let source = item["source"].as_str().unwrap_or_default();
                body.push_str(&format!(
                    "<div class=\"{classes}\"{} data-identifier=\"{}\"><div class=\"db-report__header\"><a class=\"db-report__name\" href=\"{}\">{}</a>",
                    column
                        .map(|c| format!(" data-column=\"{c}\""))
                        .unwrap_or_default(),
                    escape(key),
                    escape(item["url"].as_str().unwrap_or_default()),
                    escape(item["title"].as_str().unwrap_or_default())
                ));
                if let Some(label) = item["label"].as_str().filter(|l| !l.is_empty()) {
                    body.push_str(&format!(
                        "<div class=\"db-report__label --{s}\" data-source=\"{s}\">{}</div>",
                        escape(label),
                        s = escape(source)
                    ));
                }
                // DConditionalLoadingSpinner until its payload loads.
                body.push_str("</div><div class=\"db-report__chart\"><div class=\"loading-container inline-spinner\"><div class=\"spinner small\"></div></div></div>");
                if can_edit {
                    // handleDirectionsFor: the inner corner's handle.
                    let directions: &[&str] = match column {
                        None => &["sw", "se"],
                        Some(1) => &["sw"],
                        Some(_) => &["se"],
                    };
                    for d in directions {
                        body.push_str(&format!(
                            "<span aria-hidden=\"true\" class=\"db-report__resize-handle --{d}\" data-resize-handle=\"{d}\" data-pointer-drag=\"none\"></span>"
                        ));
                    }
                }
                body.push_str("</div>");
            }
            if can_edit && items.len() < 10 {
                let add = self.t("admin.dashboard.reports_section.add");
                body.push_str(&format!(
                    "<button aria-label=\"{a}\" class=\"db-report__add-report\" type=\"button\"><span>{} {a}</span></button>",
                    icon("plus", None),
                    a = escape(&add)
                ));
            }
            body.push_str("</div>");
        }
        let header_action = if can_edit {
            format!(
                "<div class=\"db-section__header-action\">{}</div>",
                button(
                    "btn-default btn-small",
                    "gear",
                    &self.t("admin.dashboard.reports_section.header_action"),
                    ""
                )
            )
        } else {
            String::new()
        };
        section(
            "reports",
            &self.t("admin.dashboard.sections.reports.title"),
            "grid",
            false,
            &header_action,
            Some(&self.t("admin.dashboard.reports_section.description")),
            &body,
        )
    }

    /// DashboardTraffic
    fn traffic(&self, data: &Value, error: bool) -> Result<String, AppError> {
        let i18n = self.admin.i18n;
        let kpi = |key: &str| &data["kpis"][key]["value"];
        let pageviews = kpi("browser_pageviews").as_f64().unwrap_or(0.0);
        let headline_key = match self.range.period.as_str() {
            "last_7_days" | "last_30_days" | "last_3_months" => {
                format!(
                    "admin.dashboard.site_traffic.headline.{}",
                    self.range.period
                )
            }
            _ => "admin.dashboard.site_traffic.headline.selected_period".into(),
        };
        let formatted = format_headline_count(pageviews);
        let mut headline = escape(&t_count(
            i18n,
            &headline_key,
            pageviews as i64,
            &[("formatted_count", &formatted)],
        ));
        let trend = &data["kpis"]["browser_pageviews"];
        if let Some(change) = trend["percent_change"].as_f64() {
            let direction = if change > 0.0 { "up" } else { "down" };
            let abs = change.abs();
            let percent = format!("{}%", to_number(abs, if abs < 1.0 { 1 } else { 0 }));
            headline.push_str(&format!(
                " — <span class=\"db-traffic__trend --{direction}\">{}</span>",
                escape(&t_with(
                    i18n,
                    &format!("admin.dashboard.site_traffic.trend.{direction}"),
                    &[("percent", &percent)]
                ))
            ));
            let tip = self.comparison_tooltip(&trend["comparison_period"]);
            headline.push_str(&tooltip(
                "db-section__info",
                Some("site-traffic-comparison-tooltip"),
                &tip,
            ));
        }
        let mut body = format!(
            "<div class=\"db-traffic \"><div class=\"db-section__subheader\"><div class=\"db-section__subintro\"><h3>{headline}</h3></div>"
        );
        let metric = |number: &str, label_key: &str, tip_key: &str, id: &str, test: &str| {
            format!(
                "<div class=\"db-section__metric\"{test}><div class=\"db-section__metric-number\">{}</div><div class=\"db-section__metric-label\">{}{}</div></div>",
                escape(number),
                escape(&t(i18n, label_key)),
                tooltip("db-section__info", Some(id), &t(i18n, tip_key))
            )
        };
        let has = |key: &str| !kpi(key).is_null();
        let session = data["kpis"]["bounce_rate"].is_object();
        let mut metrics = String::new();
        if has("logged_in_share") {
            metrics.push_str(&metric(
                &format!("{}%", js_number(kpi("logged_in_share"))),
                "admin.dashboard.site_traffic.kpi.logged_in_share.label",
                "admin.dashboard.site_traffic.kpi.logged_in_share.tooltip",
                "site-traffic-logged-in-share-tooltip",
                "",
            ));
        }
        if has("direct_traffic") {
            metrics.push_str(&metric(
                &format!("{}%", js_number(kpi("direct_traffic"))),
                "admin.dashboard.site_traffic.kpi.direct_traffic.label",
                "admin.dashboard.site_traffic.kpi.direct_traffic.tooltip",
                "site-traffic-direct-traffic-tooltip",
                "",
            ));
        }
        if session {
            let empty = kpi("bounce_rate").is_null();
            let tip = |key: &str| {
                if empty {
                    "admin.dashboard.site_traffic.kpi.session_metrics.empty_tooltip".to_string()
                } else {
                    key.to_string()
                }
            };
            let bounce = if empty {
                "—".to_string()
            } else {
                format!("{}%", js_number(kpi("bounce_rate")))
            };
            metrics.push_str(&metric(
                &bounce,
                "admin.dashboard.site_traffic.kpi.bounce_rate.label",
                &tip("admin.dashboard.site_traffic.kpi.bounce_rate.tooltip"),
                "site-traffic-bounce-rate-tooltip",
                " data-test-kpi=\"bounce_rate\"",
            ));
            let duration = match kpi("average_session_duration_seconds").as_f64() {
                Some(seconds) => format_minutes_seconds(self.admin, seconds),
                None => "—".into(),
            };
            metrics.push_str(&metric(
                &duration,
                "admin.dashboard.site_traffic.kpi.average_session_duration.label",
                &tip("admin.dashboard.site_traffic.kpi.average_session_duration.tooltip"),
                "site-traffic-average-session-duration-tooltip",
                " data-test-kpi=\"average_session_duration\"",
            ));
        }
        if !metrics.is_empty() {
            body.push_str(&format!(
                "<div class=\"db-section__metrics\">{metrics}</div>"
            ));
        }
        body.push_str("</div>");
        if error {
            body.push_str(&format!(
                "<div class=\"db-section__traffic-chart\"><div class=\"db-section__traffic-chart-message\" role=\"alert\">{}</div></div>",
                escape(&t(i18n, "admin.dashboard.site_traffic.fetch_error"))
            ));
        } else if !data.is_null() {
            let chart = serde_json::json!({
                "start_date": self.range.start.format("%Y-%m-%d").to_string(),
                "end_date": self.range.end.format("%Y-%m-%d").to_string(),
                "data": data["pageview_series"],
                "total_label": t(i18n, "admin.reports.chart.total.other"),
                "total_label_one": t(i18n, "admin.reports.chart.total.one"),
            });
            body.push_str(&format!(
                "<div class=\"db-section__traffic-chart\"><div class=\"db-section__traffic-chart-canvas admin-report-chart admin-report-stacked-chart\" data-chart=\"{}\"><div class=\"chart-canvas-container\"><canvas class=\"chart-canvas\"></canvas></div></div></div>",
                escape(&chart.to_string())
            ));
            let details = if self.admin.viewer.admin && !self.use_legacy_pageviews {
                let query = match self.range.period.as_str() {
                    "last_30_days" => String::new(),
                    "custom" => format!("?{}&range=custom", self.range.query()),
                    p => format!("?range={p}"),
                };
                format!(
                    "{}/admin/dashboard/site-traffic-explorer{}",
                    self.admin.base_path,
                    escape(&query)
                )
            } else {
                self.report_href("site_traffic", None)
            };
            body.push_str(&format!(
                "<div class=\"db-section__footer\"><a class=\"db-traffic__learn-more\" href=\"https://meta.discourse.org/t/improving-how-we-detect-and-flag-likely-crawlers-in-your-traffic-data/409316/9\" rel=\"noopener noreferrer\" target=\"_blank\">{} {}</a><a class=\"db-traffic__see-details\" href=\"{details}\">{}</a></div>",
                icon("up-right-from-square", None),
                escape(&t(i18n, "admin.dashboard.site_traffic.crawler_learn_more")),
                escape(&t(i18n, "admin.dashboard.site_traffic.see_details"))
            ));
            let lists = ["top_countries", "top_referrers", "top_entry_urls"];
            if lists.iter().any(|l| !data[*l].is_null()) {
                body.push_str("<div class=\"db-section__row\">");
                body.push_str(&self.traffic_list(
                    data,
                    "top_referrers",
                    "top_referrers_by_browser_pageviews",
                )?);
                if !data["top_entry_urls"].is_null() {
                    body.push_str(&self.traffic_list(data, "top_entry_urls", "top_entry_urls")?);
                }
                body.push_str(&self.traffic_list(
                    data,
                    "top_countries",
                    "top_countries_by_browser_pageviews",
                )?);
                body.push_str("</div>");
            }
        } else {
            body.push_str("<div class=\"db-section__traffic-chart\"><div class=\"db-section__traffic-chart-shell\"></div></div>");
        }
        body.push_str("</div>");
        Ok(section(
            "traffic",
            &t(i18n, "admin.dashboard.sections.traffic.title"),
            "column",
            true,
            "",
            None,
            &body,
        ))
    }

    /// `#comparisonTooltip` with `#formatComparisonDates`.
    fn comparison_tooltip(&self, period: &Value) -> String {
        let parse = |v: &Value| {
            v.as_str()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        };
        let (Some(start), Some(end)) = (parse(&period["start_date"]), parse(&period["end_date"]))
        else {
            return String::new();
        };
        let i18n = self.admin.i18n;
        let with_year = |d: NaiveDate| moment_format(d, &t(i18n, "dates.long_with_year_no_time"));
        let without_year = |d: NaiveDate| moment_format(d, &t(i18n, "dates.long_no_year_no_time"));
        let (s, e) = if start == end {
            (with_year(start), with_year(end))
        } else if start.year() == end.year() {
            (without_year(start), with_year(end))
        } else {
            (with_year(start), with_year(end))
        };
        match self.range.period.as_str() {
            "last_7_days" | "last_30_days" | "last_3_months" => {
                let key = match self.range.period.as_str() {
                    "last_7_days" => "previous_7_days",
                    "last_30_days" => "previous_30_days",
                    _ => "previous_3_months",
                };
                t_with(
                    i18n,
                    &format!("admin.dashboard.site_traffic.comparison_tooltip.{key}"),
                    &[("start", &s), ("end", &e)],
                )
            }
            _ => t_count(
                i18n,
                "admin.dashboard.site_traffic.comparison_tooltip.previous_days",
                (end - start).num_days() + 1,
                &[("start", &s), ("end", &e)],
            ),
        }
    }

    fn traffic_list(&self, data: &Value, key: &str, report: &str) -> Result<String, AppError> {
        let i18n = self.admin.i18n;
        let list = &data[key];
        let mut out = format!(
            "<div class=\"db-section__row-block\"><h3 class=\"db-section__row-block-title\"><a href=\"{}\">{} <span aria-hidden=\"true\" class=\"db-link-arrow\">{}</span></a></h3>",
            self.report_href(report, None),
            escape(&t(
                i18n,
                &format!("admin.dashboard.site_traffic.{key}.title")
            )),
            icon("arrow-right", None)
        );
        let rows = list["rows"].as_array().filter(|r| !r.is_empty());
        if !list["error"].is_null() && list["error"] != Value::Bool(false) {
            out.push_str(&format!(
                "<p class=\"db-traffic__list-error\" role=\"status\">{}</p>",
                escape(&t(
                    i18n,
                    &format!("admin.dashboard.site_traffic.{key}.error")
                ))
            ));
        } else if let Some(rows) = rows {
            out.push_str("<ul class=\"db-traffic__list\">");
            for row in rows {
                let metric = format!(
                    "<span class=\"db-traffic__metric\"><span class=\"db-traffic__percent\">{}%</span> <span class=\"db-traffic__count\">({})</span></span>",
                    js_number(&row["percent"]),
                    format_headline_count(row["count"].as_f64().unwrap_or(0.0))
                );
                match key {
                    "top_referrers" => {
                        let referrer = row["normalized_referrer"].as_str().unwrap_or_default();
                        out.push_str(&format!(
                            "<li class=\"db-traffic__list-row\"><a class=\"db-traffic__link\" href=\"https://{r}\" rel=\"noopener noreferrer nofollow ugc\" target=\"_blank\" title=\"{r}\"><span class=\"db-traffic__label\">{r}</span></a>{metric}</li>",
                            r = escape(referrer)
                        ));
                    }
                    "top_entry_urls" => {
                        let url = row["entry_url"].as_str().unwrap_or_default();
                        out.push_str(&format!(
                            "<li class=\"db-traffic__list-row\"><a class=\"db-traffic__link\" href=\"{}{u}\" title=\"{u}\"><span class=\"db-traffic__label\">{u}</span></a>{metric}</li>",
                            self.admin.base_path,
                            u = escape(url)
                        ));
                    }
                    // countryName is Intl.DisplayNames' region name.
                    _ => {
                        return Err(Unsupported(
                            "the dashboard's country names (Intl.DisplayNames)",
                        )
                        .into());
                    }
                }
            }
            out.push_str("</ul>");
        } else {
            out.push_str(&format!(
                "<p class=\"db-traffic__list-empty\">{}</p>",
                escape(&t(
                    i18n,
                    &format!("admin.dashboard.site_traffic.{key}.empty")
                ))
            ));
        }
        out.push_str("</div>");
        Ok(out)
    }

    /// DashboardEngagement
    fn engagement(&self, data: &Value, error: bool) -> String {
        let mut body = String::new();
        if error {
            body.push_str(&self.error("admin.dashboard.sections.engagement.fetch_error"));
        } else {
            if !data.is_null() {
                body.push_str(&self.engagement_headline(&data["kpis"]));
            }
            body.push_str("<div class=\"db-section__row-group\"><div class=\"db-section__row\"><div class=\"db-section__row-block\">");
            if !data["trust_level_pipeline"].is_null() {
                body.push_str(&self.trust_level_pipeline(&data["trust_level_pipeline"]));
            }
            body.push_str("</div><div class=\"db-section__row-block\">");
            body.push_str(&self.whos_posting(&data["posters"]));
            body.push_str("</div></div>");
            if !data["activity_by_category"].is_null() {
                body.push_str(
                    "<div class=\"db-section__row\"><div class=\"db-section__row-block\">",
                );
                body.push_str(&self.activity_by_category(&data["activity_by_category"]));
                body.push_str("</div></div>");
            }
            body.push_str("</div>");
        }
        section(
            "engagement",
            &self.t("admin.dashboard.sections.engagement.title"),
            "column",
            true,
            "",
            None,
            &body,
        )
    }

    /// EngagementHeadline
    fn engagement_headline(&self, kpis: &Value) -> String {
        let empty = Vec::new();
        let kpis = kpis.as_array().unwrap_or(&empty);
        let by_type = |kind: &str| kpis.iter().find(|k| k["type"] == kind);
        let direction = |m: Option<&Value>| -> &'static str {
            let Some(m) = m else { return "unavailable" };
            let Some(value) = m["value"].as_f64() else {
                return "unavailable";
            };
            let previous = m["previous_value"].as_f64();
            if previous.is_none() || previous == Some(0.0) {
                return if value > 0.0 { "improved" } else { "flat" };
            }
            let change = percent_change(m);
            let rounded = round_delta_percent(change);
            if rounded > 0.0 {
                "improved"
            } else if rounded < 0.0 {
                "declined"
            } else {
                "flat"
            }
        };
        let order = ["dau_mau", "daily_engaged_users", "new_signups"];
        let dirs: Vec<&str> = order.iter().map(|k| direction(by_type(k))).collect();
        // The declined metric that fell furthest owns the call to action.
        let mut cta_owner: Option<(&str, f64)> = None;
        for (kind, dir) in order.iter().zip(&dirs) {
            if *dir != "declined" {
                continue;
            }
            let change = round_delta_percent(percent_change(by_type(kind).unwrap_or(&Value::Null)));
            if cta_owner.is_none_or(|(_, most)| change < most) {
                cta_owner = Some((kind, change));
            }
        }
        let (title, summary) =
            crate::admin_dashboard_headlines::engagement(dirs[0], dirs[1], dirs[2]);
        let prefix = "admin.dashboard.sections.engagement.headline";
        let mut summary_text = self.t(&format!("{prefix}.summaries.{summary}"));
        if let Some((owner, _)) = cta_owner {
            summary_text = format!(
                "{summary_text} {}",
                self.t(&format!("{prefix}.cta.{owner}"))
            );
        }
        let mut out = format!(
            "<div class=\"db-section__subheader\"><div class=\"db-section__subintro\"><h3>{}</h3><p>{}</p></div><div class=\"db-section__metrics\">",
            escape(&self.t(&format!("{prefix}.titles.{title}"))),
            escape(&summary_text)
        );
        for m in kpis {
            let kind = m["type"].as_str().unwrap_or_default();
            let value = format_kpi_value(&m["value"], kind == "dau_mau");
            out.push_str(&format!(
                "<div class=\"db-section__metric\"><div class=\"db-section__metric-number\">{}</div><div class=\"db-section__metric-label\"><a href=\"{}\">{}</a>{}</div>",
                escape(&value),
                self.report_href(m["report_type"].as_str().unwrap_or_default(), Some(&m["report_query"])),
                escape(&self.t(&format!("{prefix}.metrics.{kind}"))),
                tooltip(
                    "db-section__info",
                    Some(&format!("engagement-headline-{kind}-tooltip")),
                    &self.t(&format!("admin.dashboard.highlights.kpi.{kind}.tooltip"))
                )
            ));
            if let Some(change) = m["percent_change"].as_f64() {
                if change == 0.0 {
                    out.push_str(&format!(
                        "<span class=\"db-pill\">{}</span>",
                        escape(&self.t("admin.dashboard.stable"))
                    ));
                } else {
                    out.push_str(&format!(
                        "<div class=\"db-delta {}\">{}</div>",
                        if change > 0.0 { "--pos" } else { "--neg" },
                        escape(&format_delta_percent(change))
                    ));
                }
            }
            out.push_str("</div>");
        }
        out.push_str("</div></div>");
        out
    }

    /// TrustLevelPipeline
    fn trust_level_pipeline(&self, data: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.engagement.trust_level_pipeline";
        let empty = Vec::new();
        let rows = data["rows"].as_array().unwrap_or(&empty);
        let n = |row: &Value, key: &str| row[key].as_f64().unwrap_or(0.0);
        let bar_max = rows.iter().fold(0.0f64, |acc, r| {
            acc.max(n(r, "promoted_in")).max(n(r, "demoted_in"))
        });
        let trend = &data["trend"];
        let trend_text = match trend["direction"].as_str() {
            None => None,
            Some("stable") => Some(t(i18n, "admin.dashboard.stable")),
            Some(direction) => Some(t_count(
                i18n,
                &format!("{prefix}.trend.{direction}"),
                trend["net"].as_i64().unwrap_or(0),
                &[],
            )),
        };
        let trend_class = match trend["direction"].as_str() {
            Some("climbing") => "--pos",
            Some("dropping") => "--neg",
            _ => "",
        };
        let mut out = format!(
            "<div class=\"db-tl-pipeline\"><div class=\"db-section__row-block-header\"><a class=\"db-section__row-block-title --label\" href=\"{}/admin/reports/trust_level_pipeline\">{}</a>",
            self.admin.base_path,
            escape(&t(i18n, &format!("{prefix}.title")))
        );
        if let Some(text) = trend_text {
            out.push_str(&format!(
                "<span class=\"db-pill {trend_class}\">{}</span>",
                escape(&text)
            ));
        }
        out.push_str("</div><ol class=\"db-tl-pipeline__rows\">");
        for row in rows {
            let promoted = n(row, "promoted_in");
            let demoted = n(row, "demoted_in");
            let signups = n(row, "signups");
            let bar_down = demoted > promoted;
            let bar_value = if bar_down { demoted } else { promoted };
            let ladder = promoted > 0.0 || demoted > 0.0;
            out.push_str(&format!(
                "<li class=\"db-tl-pipeline__row\"><div class=\"db-tl-pipeline__label\"><span class=\"db-tl-pipeline__name\">{}</span><span class=\"db-tl-pipeline__count\" title=\"{}\">{} ({}%)</span>",
                escape(&t(
                    i18n,
                    &format!("{prefix}.trust_levels.{}", row["trust_level"])
                )),
                escape(&t(i18n, &format!("{prefix}.count_title"))),
                to_number(n(row, "count"), 0),
                to_number(n(row, "share"), 2)
            ));
            if signups > 0.0 {
                out.push_str(&format!(
                    "<span class=\"db-tl-pipeline__signups\" title=\"{}\">{}</span>",
                    escape(&t(i18n, &format!("{prefix}.signups_title"))),
                    escape(&t_count(
                        i18n,
                        &format!("{prefix}.signups"),
                        signups as i64,
                        &[("formattedCount", &to_number(signups, 0))]
                    ))
                ));
            }
            if !ladder && signups <= 0.0 {
                out.push_str(&format!(
                    "<span class=\"db-pill\">{}</span>",
                    escape(&t(i18n, "admin.dashboard.stable"))
                ));
            }
            out.push_str("</div>");
            if ladder {
                out.push_str(
                    "<div class=\"db-tl-pipeline__flow\"><div class=\"db-tl-pipeline__bar-track\">",
                );
                if bar_value > 0.0 {
                    let width = if bar_max == 0.0 {
                        0.0
                    } else {
                        js_round(bar_value / bar_max * 100.0)
                    };
                    out.push_str(&format!(
                        "<span aria-hidden=\"true\" class=\"db-tl-pipeline__bar{}\" style=\"width: {}%\"></span>",
                        if bar_down { " --demoted" } else { "" },
                        width as i64
                    ));
                }
                out.push_str("</div>");
                if promoted > 0.0 {
                    out.push_str(&format!(
                        "<span aria-label=\"{}\" class=\"db-delta --pos\">{} {}</span>",
                        escape(&t_count(
                            i18n,
                            &format!("{prefix}.arrivals_aria"),
                            promoted as i64,
                            &[]
                        )),
                        to_number(promoted, 0),
                        icon("arrow-up", None)
                    ));
                }
                if demoted > 0.0 {
                    out.push_str(&format!(
                        "<span aria-label=\"{}\" class=\"db-delta --neg\">{} {}</span>",
                        escape(&t_count(
                            i18n,
                            &format!("{prefix}.demotions_in_aria"),
                            demoted as i64,
                            &[]
                        )),
                        icon("arrow-down", None),
                        to_number(demoted, 0)
                    ));
                }
                out.push_str("</div>");
            }
            out.push_str("</li>");
        }
        out.push_str("</ol></div>");
        out
    }

    /// MultipleCategoriesSelector's closed header.
    fn categories_selector(&self, ids: &[i64], none: Option<&str>) -> String {
        let names: Vec<&str> = ids
            .iter()
            .filter_map(|id| self.list.categories.get(id))
            .map(|c| c.name.as_str())
            .collect();
        let label = if names.is_empty() {
            none.map(|key| t(self.admin.i18n, key)).unwrap_or_default()
        } else {
            names.join(", ")
        };
        let values: Vec<String> = ids
            .iter()
            .filter(|id| self.list.categories.contains_key(id))
            .map(|id| id.to_string())
            .collect();
        format!(
            "<details class=\"select-kit multi-select multiple-categories-selector{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
            if names.is_empty() {
                ""
            } else {
                " has-selection"
            },
            escape(&names.join(",")),
            values.join(","),
            escape(&label),
            icon("plus", Some("angle-icon"))
        )
    }

    /// WhosPosting
    fn whos_posting(&self, posters: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.engagement.whos_posting";
        let ids: Vec<i64> = posters["category_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .collect();
        let groups: Vec<&str> = posters["groups"]
            .as_array()
            .map(|g| g.iter().filter_map(Value::as_str).collect())
            .unwrap_or_else(|| vec!["new_members", "returning", "staff"]);
        // The report route serializes its filters as JSON; the router
        // writes the query params in name order.
        let mut filters = serde_json::Map::new();
        if !ids.is_empty() {
            let joined: Vec<String> = ids.iter().map(i64::to_string).collect();
            filters.insert("category_ids".into(), Value::String(joined.join(",")));
        }
        if groups != ["new_members", "returning", "staff"] {
            filters.insert("groups".into(), Value::String(groups.join(",")));
        }
        let mut query = vec![format!("end_date={}", self.range.end.format("%Y-%m-%d"))];
        if !filters.is_empty() {
            query.push(format!(
                "filters={}",
                enc(&Value::Object(filters).to_string())
            ));
        }
        query.push(format!(
            "start_date={}",
            self.range.start.format("%Y-%m-%d")
        ));
        let mut out = format!(
            "<div class=\"db-whos-posting\"><div class=\"db-section__row-block-header\"><a class=\"db-section__row-block-title --label\" href=\"{}/admin/reports/posters_by_member_type?{}\">{}</a><div class=\"db-whos-posting__filter\">{}</div></div>",
            self.admin.base_path,
            escape(&query.join("&")),
            escape(&t(i18n, &format!("{prefix}.title"))),
            self.categories_selector(&ids, Some("category.all"))
        );
        let total = posters["total"].as_f64().unwrap_or(0.0);
        if total > 0.0 {
            struct Bar {
                name: String,
                share: String,
                width: String,
                class: String,
            }
            let mut group_index = 0;
            let mut bars = Vec::new();
            for row in posters["rows"].as_array().into_iter().flatten() {
                let class = if row["kind"] == "group" {
                    group_index += 1;
                    format!("--group-{}", (group_index - 1) % 6)
                } else {
                    format!(
                        "--{}",
                        row["type"]
                            .as_str()
                            .unwrap_or_default()
                            .replacen('_', "-", 1)
                    )
                };
                bars.push(Bar {
                    name: row["name"].as_str().unwrap_or_default().to_string(),
                    share: format!("{}%", js_round(row["share"].as_f64().unwrap_or(0.0)) as i64),
                    width: js_number(&row["share"]),
                    class,
                });
            }
            let aria: Vec<String> = bars
                .iter()
                .map(|b| format!("{} {}", b.name, b.share))
                .collect();
            out.push_str(&format!(
                "<div aria-label=\"{}\" class=\"db-whos-posting__bars\" role=\"img\">",
                escape(&aria.join(", "))
            ));
            for b in &bars {
                out.push_str(&format!(
                    "<div class=\"db-whos-posting__bar-row\"><span class=\"db-whos-posting__bar-label\">{}</span><span class=\"db-whos-posting__bar-track\"><span class=\"db-whos-posting__bar-fill {}\" style=\"width: {}%\"></span></span><span class=\"db-whos-posting__bar-share\">{}</span></div>",
                    escape(&b.name),
                    b.class,
                    b.width,
                    b.share
                ));
            }
            out.push_str("</div>");
        } else {
            out.push_str(&format!(
                "<p class=\"db-whos-posting__empty\">{}</p>",
                escape(&t(i18n, &format!("{prefix}.empty")))
            ));
        }
        out.push_str(&button(
            "btn-transparent db-whos-posting__add-group",
            "plus",
            &t(i18n, &format!("{prefix}.add_group")),
            "",
        ));
        out.push_str("</div>");
        out
    }

    /// ActivityByCategory, sorted by share, descending.
    fn activity_by_category(&self, activity: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.engagement.activity_by_category";
        let empty = Vec::new();
        let rows = activity["rows"].as_array().unwrap_or(&empty);
        let ids: Vec<i64> = match activity["category_ids"].as_array() {
            Some(ids) => ids.iter().filter_map(Value::as_i64).collect(),
            None => rows
                .iter()
                .filter_map(|r| r["category_id"].as_i64())
                .collect(),
        };
        let mut out = format!(
            "<div class=\"db-activity\"><div class=\"db-section__row-block-header\"><a class=\"db-section__row-block-title --label\" href=\"{}/admin/reports/activity_by_category\">{}</a>{}</div>",
            self.admin.base_path,
            escape(&t(i18n, &format!("{prefix}.title"))),
            self.categories_selector(&ids, None)
        );
        if rows.is_empty() {
            out.push_str(&format!(
                "<p class=\"db-activity__empty\">{}</p></div>",
                escape(&t(i18n, &format!("{prefix}.empty")))
            ));
            return out;
        }
        out.push_str("<div class=\"db-activity__table-scroll-container\"><table class=\"db-activity-table\"><thead><tr>");
        out.push_str(&format!(
            "<th class=\"db-activity-table__col-category\">{}</th>",
            escape(&t(i18n, &format!("{prefix}.category")))
        ));
        for (field, label) in [
            ("topics", "topics"),
            ("posts", "posts"),
            ("page_views", "page_views"),
            ("share", "share"),
            ("share_change", "vs_prior"),
        ] {
            let sorted = field == "share";
            out.push_str(&format!(
                "<th aria-sort=\"{}\" class=\"db-activity-table__col-number\"><button class=\"db-activity-table__sort-button\" type=\"button\" data-sort=\"{field}\">{} {}</button></th>",
                if sorted { "descending" } else { "none" },
                escape(&t(i18n, &format!("{prefix}.{label}"))),
                icon("chevron-down", if sorted { None } else { Some("is-hidden") })
            ));
        }
        out.push_str("</tr></thead><tbody>");
        let mut sorted: Vec<&Value> = rows.iter().collect();
        // A stable sort, as Array#sort is: equal shares keep their order.
        sorted.sort_by(|a, b| {
            let av = a["share"].as_f64().unwrap_or(0.0);
            let bv = b["share"].as_f64().unwrap_or(0.0);
            bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
        });
        for row in sorted {
            let id = row["category_id"].as_i64().unwrap_or(0);
            let category = self.list.categories.get(&id);
            let badge = match category {
                Some(c) => {
                    let mut chain = Vec::new();
                    let mut parent = c.parent_id.and_then(|p| self.list.categories.get(&p));
                    while let Some(p) = parent {
                        chain.push(p);
                        parent = p.parent_id.and_then(|pp| self.list.categories.get(&pp));
                    }
                    chain.reverse();
                    let mut html: String = chain
                        .iter()
                        .map(|p| {
                            crate::topic_list_view::category_badge_html(self.list, p, false, false)
                        })
                        .collect();
                    html.push_str(&crate::topic_list_view::category_badge_html(
                        self.list, c, false, true,
                    ));
                    html
                }
                None => {
                    let color = row["color"].as_str().unwrap_or_default();
                    let color = if color.len() == 6 && color.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        color
                    } else {
                        "cccccc"
                    };
                    format!(
                        "<span aria-hidden=\"true\" class=\"db-activity-table__swatch\" style=\"background-color: #{color}\"></span> {}",
                        escape(row["name"].as_str().unwrap_or_default())
                    )
                }
            };
            let slug = match category {
                Some(c) => category_slug_for(self.list, c),
                None => row["slug"].as_str().unwrap_or_default().to_string(),
            };
            let n = |key: &str| row[key].as_f64().unwrap_or(0.0);
            let change = n("share_change");
            out.push_str(&format!(
                "<tr data-topics=\"{}\" data-posts=\"{}\" data-page_views=\"{}\" data-share=\"{}\" data-share_change=\"{}\"><td class=\"db-activity-table__cell-category\">{badge}</td><td class=\"db-activity-table__cell-number\"><a href=\"{}\">{}</a></td><td class=\"db-activity-table__cell-number\"><a href=\"{}\">{}</a></td><td class=\"db-activity-table__cell-number\">{}</td><td class=\"db-activity-table__cell-number\">{}</td><td class=\"db-activity-table__cell-number db-delta {}\">{}</td></tr>",
                js_number(&row["topics"]),
                js_number(&row["posts"]),
                js_number(&row["page_views"]),
                js_number(&row["share"]),
                js_number(&row["share_change"]),
                self.filter_href(&slug, "created"),
                to_number(n("topics"), 0),
                self.filter_href(&slug, "activity"),
                to_number(n("posts"), 0),
                crate::topic_list_view::number(self.list, js_round(n("page_views")) as i64),
                escape(row["share_formatted"].as_str().unwrap_or_default()),
                if change > 0.0 {
                    "--pos"
                } else if change < 0.0 {
                    "--neg"
                } else {
                    ""
                },
                escape(row["share_change_formatted"].as_str().unwrap_or_default())
            ));
        }
        out.push_str("</tbody></table></div></div>");
        out
    }

    /// `#topicQuery`: /filter?q= for the category's topics in the range.
    fn filter_href(&self, slug: &str, field: &str) -> String {
        let q = format!(
            "{field}-after:{} {field}-before:{} =category:{slug}",
            self.range.start.format("%Y-%m-%d"),
            (self.range.end + chrono::Duration::days(1)).format("%Y-%m-%d")
        );
        let encoded: String = q
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'_'
                | b'.'
                | b'!'
                | b'~'
                | b'*'
                | b'\''
                | b'('
                | b')' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect();
        format!("{}/filter?q={}", self.admin.base_path, encoded)
    }

    /// DashboardSearch
    fn search(&self, data: &Value, error: bool) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.search";
        let mut body = String::new();
        if error {
            body.push_str(&self.error(&format!("{prefix}.fetch_error")));
        } else if data["logging_enabled"] == Value::Bool(false) {
            let text = if self.admin.viewer.admin {
                t_with(
                    i18n,
                    &format!("{prefix}.logging_disabled"),
                    &[("basePath", self.admin.base_path)],
                )
            } else {
                escape(&t(i18n, &format!("{prefix}.logging_disabled_moderator")))
            };
            body.push_str(&format!("<p class=\"db-section__callout\">{text}</p>"));
        } else if !data.is_null() {
            let total = &data["kpis"]["total_searches"];
            let rate = &data["kpis"]["no_result_rate"];
            let (title, summary) = if total["value"].as_f64() == Some(0.0)
                && total["previous_value"].as_f64() == Some(0.0)
                && rate["value"].is_null()
            {
                ("insufficient_activity", "insufficient_activity".to_string())
            } else {
                let direction = |kpi: &Value, change: &Value| -> &'static str {
                    let Some(value) = kpi["value"].as_f64() else {
                        return "unavailable";
                    };
                    let previous = kpi["previous_value"].as_f64();
                    if previous.is_none() || previous == Some(0.0) {
                        return if value > 0.0 { "up" } else { "flat" };
                    }
                    match change.as_f64() {
                        Some(c) if c > 0.0 => "up",
                        Some(c) if c < 0.0 => "down",
                        _ => "flat",
                    }
                };
                let (title, summary, cta) = crate::admin_dashboard_headlines::search(
                    direction(total, &total["percent_change"]),
                    direction(rate, &rate["point_change"]),
                );
                let mut text = t(i18n, &format!("{prefix}.headline.summaries.{summary}"));
                if let Some(cta) = cta {
                    text = format!(
                        "{text} {}",
                        t(i18n, &format!("{prefix}.headline.cta.{cta}"))
                    );
                }
                (title, text)
            };
            let summary = if title == "insufficient_activity" {
                t(i18n, &format!("{prefix}.headline.summaries.{summary}"))
            } else {
                summary
            };
            body.push_str(&format!(
                "<div class=\"db-section__subheader\"><div class=\"db-section__subintro\"><h3>{}</h3><p>{}</p></div><div class=\"db-section__metrics\">",
                escape(&t(i18n, &format!("{prefix}.headline.titles.{title}"))),
                escape(&summary)
            ));
            let delta = |change: Option<f64>, up_is_good: bool| {
                change
                    .map(|c| {
                        let good = (c > 0.0) == up_is_good;
                        format!(
                            "<div class=\"db-delta {}\">{}</div>",
                            if good { "--pos" } else { "--neg" },
                            escape(&format_delta_percent(c))
                        )
                    })
                    .unwrap_or_default()
            };
            body.push_str(&format!(
                "<div class=\"db-section__metric\" data-test-search-kpi=\"total_searches\"><div class=\"db-section__metric-number\">{}</div><div class=\"db-section__metric-label\">{}{}</div>{}</div>",
                to_number(total["value"].as_f64().unwrap_or(0.0), 0),
                escape(&t(i18n, &format!("{prefix}.kpi.total_searches.label"))),
                tooltip(
                    "db-section__info",
                    Some("search-total-searches-tooltip"),
                    &t(i18n, &format!("{prefix}.kpi.total_searches.tooltip"))
                ),
                delta(total["percent_change"].as_f64(), true)
            ));
            let rate_value = match rate["value"].as_f64() {
                Some(v) => format!("{}%", to_number(v, 0)),
                None => "—".into(),
            };
            body.push_str(&format!(
                "<div class=\"db-section__metric\" data-test-search-kpi=\"no_result_rate\"><div class=\"db-section__metric-number{}\">{}</div><div class=\"db-section__metric-label\">{}{}</div>{}</div>",
                if rate["exceeds_threshold"] == Value::Bool(true) { " --neg" } else { "" },
                escape(&rate_value),
                escape(&t(i18n, &format!("{prefix}.kpi.no_result_rate.label"))),
                tooltip(
                    "db-section__info",
                    Some("search-no-result-rate-tooltip"),
                    &t(i18n, &format!("{prefix}.kpi.no_result_rate.tooltip"))
                ),
                delta(rate["point_change"].as_f64(), false)
            ));
            body.push_str("</div></div><div class=\"db-section__row\">");
            body.push_str(&self.search_trending(data));
            body.push_str(&self.search_gaps(data));
            body.push_str("</div>");
        }
        section(
            "search",
            &t(i18n, &format!("{prefix}.title")),
            "column",
            true,
            "",
            None,
            &body,
        )
    }

    fn search_trending(&self, data: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.search";
        let mut out = format!(
            "<div class=\"db-section__row-block\"><h3 class=\"db-section__row-block-title\">{}{}</h3>",
            escape(&t(i18n, &format!("{prefix}.trending.title"))),
            tooltip(
                "db-section__info",
                Some("search-trending-tooltip"),
                &t(i18n, &format!("{prefix}.trending.tooltip"))
            )
        );
        let rows = data["trending"].as_array().filter(|r| !r.is_empty());
        match rows {
            Some(rows) => {
                let period = if self.range.period == "custom" {
                    "all"
                } else {
                    data["trending_period"].as_str().unwrap_or_default()
                };
                out.push_str(&format!(
                    "<table class=\"db-search-table\"><thead><tr><th>{}</th><th class=\"db-search-table__col-number\">{}</th></tr></thead><tbody>",
                    escape(&t(i18n, &format!("{prefix}.table.term"))),
                    escape(&t(i18n, &format!("{prefix}.table.searches")))
                ));
                for row in rows {
                    let term = row["term"].as_str().unwrap_or_default();
                    let query = format!(
                        "period={}&searchType={}&term={}",
                        enc(period),
                        enc(data["search_type"].as_str().unwrap_or_default()),
                        enc(term)
                    );
                    out.push_str(&format!(
                        "<tr data-test-search-term-row><td><a title=\"{t}\" href=\"{}/admin/logs/search_logs/term?{}\">{t}</a></td><td class=\"db-search-table__cell-number\">{}</td></tr>",
                        self.admin.base_path,
                        escape(&query),
                        to_number(row["searches"].as_f64().unwrap_or(0.0), 0),
                        t = escape(term)
                    ));
                }
                out.push_str("</tbody></table>");
            }
            None => out.push_str(&format!(
                "<p class=\"db-search-table__empty\">{}</p>",
                escape(&t(i18n, &format!("{prefix}.trending.empty")))
            )),
        }
        out.push_str("</div>");
        out
    }

    fn search_gaps(&self, data: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.search";
        let mut out = format!(
            "<div class=\"db-section__row-block\"><h3 class=\"db-section__row-block-title\">{}</h3>",
            escape(&t(i18n, &format!("{prefix}.content_gaps.title")))
        );
        match data["content_gaps"].as_array().filter(|r| !r.is_empty()) {
            Some(rows) => {
                out.push_str(&format!(
                    "<table class=\"db-search-table\"><thead><tr><th>{}</th><th class=\"db-search-table__col-status\">{}</th><th class=\"db-search-table__col-number\">{}</th></tr></thead><tbody>",
                    escape(&t(i18n, &format!("{prefix}.table.term"))),
                    escape(&t(i18n, &format!("{prefix}.table.status"))),
                    escape(&t(i18n, &format!("{prefix}.table.searches")))
                ));
                for row in rows {
                    let term = row["term"].as_str().unwrap_or_default();
                    let status = row["status"].as_str().unwrap_or_default();
                    out.push_str(&format!(
                        "<tr data-test-search-term-row><td><a title=\"{t}\" href=\"{}/search?q={}\">{t}</a></td><td><span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger db-pill --neg\" data-identifier=\"search-gap-badge-tooltip\" data-trigger=\"\" role=\"button\" data-tooltip-content=\"{}\"><span class=\"fk-d-tooltip__trigger-container\">{}</span></span></td><td class=\"db-search-table__cell-number\">{}</td></tr>",
                        self.admin.base_path,
                        escape(&enc(term)),
                        escape(&t(i18n, &format!("{prefix}.content_gaps.badge.{status}_tooltip"))),
                        escape(&t(i18n, &format!("{prefix}.content_gaps.badge.{status}"))),
                        to_number(row["searches"].as_f64().unwrap_or(0.0), 0),
                        t = escape(term)
                    ));
                }
                out.push_str("</tbody></table>");
            }
            None => out.push_str(&format!(
                "<p class=\"db-search-table__empty\">{}</p>",
                escape(&t(i18n, &format!("{prefix}.content_gaps.empty")))
            )),
        }
        out.push_str("</div>");
        out
    }

    /// DashboardSystem
    fn system(&self, data: &Value, error: bool) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.system";
        let mut body = String::new();
        if error {
            body.push_str(&self.error(&format!("{prefix}.fetch_error")));
        } else if !data.is_null() {
            body.push_str(&self.system_version(&data["version"]));
            if !data["storage"].is_null() {
                body.push_str(&self.system_storage(&data["storage"]));
            }
            body.push_str(&self.system_status(data));
        }
        section(
            "system",
            &t(i18n, &format!("{prefix}.title")),
            "row",
            true,
            "",
            None,
            &body,
        )
    }

    /// The version block with VersionCheck's status.
    fn system_version(&self, version: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.system";
        let mut out = format!(
            "<div class=\"db-section__row-block db-system__block\"><div class=\"db-section__row-block-header\"><h3 class=\"db-section__header\">{} {}</h3></div><div class=\"db-system__headline\"><div class=\"db-group\"><div class=\"db-system__value\">{}</div>",
            icon("tag", Some("db-system__block-icon")),
            escape(&t(i18n, "admin.dashboard.version")),
            escape(
                version["installed_version"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .unwrap_or("—")
            )
        );
        let ahead = commits_ahead(version["installed_describe"].as_str());
        if let Some(n) = ahead {
            out.push_str(&format!(
                "<span class=\"db-pill\" title=\"{}\">{}</span>",
                escape(&t_count(i18n, "admin.dashboard.commits_ahead", n, &[])),
                escape(&t_count(i18n, &format!("{prefix}.commits"), n, &[]))
            ));
        }
        out.push_str("</div>");
        if let Some(sha) = version["installed_sha"].as_str().filter(|s| !s.is_empty()) {
            let short: String = sha.chars().take(10).collect();
            out.push_str(&format!(
                "<a class=\"db-system__link\" href=\"https://github.com/discourse/discourse/commits/{}\" rel=\"noopener noreferrer\" target=\"_blank\" title=\"{}\">{} {}</a>",
                escape(sha),
                escape(&t_with(i18n, "admin.dashboard.commit_on_github", &[("sha", &short)])),
                escape(&t(i18n, &format!("{prefix}.view_on_github"))),
                icon("up-right-from-square", None)
            ));
        }
        out.push_str("</div>");
        let stale = version["stale_data"].as_bool().unwrap_or(false);
        let pending = version["version_check_pending"].as_bool().unwrap_or(false);
        let missing = &version["missing_versions_count"];
        let up_to_date = missing.is_null() || missing.as_i64() == Some(0);
        let (status_icon, class, message) = if version["updated_at"].is_null()
            && version.get("updated_at").is_some()
            || (stale && !pending)
        {
            (
                "triangle-exclamation",
                "--critical",
                t(i18n, &format!("{prefix}.update_check_failed")),
            )
        } else if stale {
            (
                "circle-check",
                "--ok",
                t(i18n, "admin.dashboard.version_check_pending"),
            )
        } else if up_to_date {
            let latest = commits_ahead(version["latest_pretty_version"].as_str());
            let newer = latest.map(|l| l - ahead.unwrap_or(0)).unwrap_or(0);
            let message = if newer > 0 {
                t_count(i18n, "admin.dashboard.up_to_date_newer_commits", newer, &[])
            } else {
                t(i18n, "admin.dashboard.up_to_date")
            };
            ("circle-check", "--ok", message)
        } else {
            let message = match version["latest_version"].as_str() {
                Some(v) if !v.is_empty() => t_with(
                    i18n,
                    &format!("{prefix}.update_available"),
                    &[("version", v)],
                ),
                _ => t(i18n, "admin.dashboard.updates_available"),
            };
            ("triangle-exclamation", "--warning", message)
        };
        out.push_str(&format!(
            "<div class=\"db-system__footer {class}\"><span class=\"db-system__label\">{} {}</span></div></div>",
            icon(status_icon, Some("db-system__status-icon")),
            escape(&message)
        ));
        out
    }

    fn system_storage(&self, storage: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.system";
        let backups = &storage["backups"];
        let uploads = &storage["uploads"];
        let entries: Vec<(&str, &Value, &str)> =
            [("backups", backups, ""), ("uploads", uploads, "--alt")]
                .into_iter()
                .filter(|(_, stats, _)| !stats.is_null())
                .collect();
        let local: Vec<&(&str, &Value, &str)> = entries
            .iter()
            .filter(|(_, s, _)| s["remote"] != Value::Bool(true))
            .collect();
        let used: f64 = local
            .iter()
            .map(|(_, s, _)| s["used_bytes"].as_f64().unwrap_or(0.0))
            .sum();
        let free = local.iter().find_map(|(_, s, _)| s["free_bytes"].as_f64());
        let used_space = to_human_size(self.admin, used);
        let free_space = free.map(|f| to_human_size(self.admin, f));
        let mut out = format!(
            "<div class=\"db-section__row-block db-system__block --storage\"><div class=\"db-section__row-block-header\"><h3 class=\"db-section__header\">{} {}</h3></div><div class=\"db-system__headline\"><div class=\"db-system__value\">{}</div>",
            icon("database", Some("db-system__block-icon")),
            escape(&t(i18n, &format!("{prefix}.storage"))),
            escape(&t_with(
                i18n,
                "admin.dashboard.space_used",
                &[("usedSize", &used_space)]
            ))
        );
        if let Some(free_space) = &free_space {
            out.push_str(&format!(
                "<div class=\"db-system__label\">{}</div>",
                escape(&t_with(
                    i18n,
                    &format!("{prefix}.space_free"),
                    &[("size", free_space)]
                ))
            ));
        }
        out.push_str("</div><div>");
        if let Some(free) = free {
            let segments: Vec<&&(&str, &Value, &str)> = local
                .iter()
                .filter(|(_, s, _)| s["used_bytes"].as_f64().is_some_and(|u| u != 0.0))
                .collect();
            if !segments.is_empty() {
                let total: f64 = segments
                    .iter()
                    .map(|(_, s, _)| s["used_bytes"].as_f64().unwrap_or(0.0))
                    .sum::<f64>()
                    + free;
                out.push_str(&format!(
                    "<div aria-label=\"{}\" class=\"db-bar-track\" role=\"img\">",
                    escape(&t_with(
                        i18n,
                        "admin.dashboard.space_used_and_free",
                        &[
                            ("usedSize", &used_space),
                            ("freeSize", free_space.as_deref().unwrap_or_default())
                        ]
                    ))
                ));
                for (key, s, class) in segments {
                    let width =
                        (s["used_bytes"].as_f64().unwrap_or(0.0) / total * 100.0).min(100.0);
                    out.push_str(&format!(
                        "<span class=\"db-bar-fill{}\" style=\"width: {}%\" title=\"{}\"></span>",
                        if class.is_empty() {
                            String::new()
                        } else {
                            format!(" {class}")
                        },
                        to_fixed(width, 1),
                        escape(&t(i18n, &format!("admin.dashboard.{key}")))
                    ));
                }
                out.push_str("</div>");
            }
        }
        out.push_str("<div class=\"db-system__breakdown\">");
        if !backups.is_null() {
            out.push_str(&format!(
                "<span class=\"db-system__breakdown-item\">{}</span> <span class=\"dot-separator\"></span> ",
                escape(&t_count(
                    i18n,
                    &format!("{prefix}.backups_size"),
                    backups["count"].as_i64().unwrap_or(0),
                    &[(
                        "size",
                        &to_human_size(self.admin, backups["used_bytes"].as_f64().unwrap_or(0.0))
                    )]
                ))
            ));
        }
        out.push_str(&format!(
            "<span class=\"db-system__breakdown-item\">{}</span></div></div>",
            escape(&t_with(
                i18n,
                &format!("{prefix}.uploads_size"),
                &[(
                    "size",
                    &to_human_size(self.admin, uploads["used_bytes"].as_f64().unwrap_or(0.0))
                )]
            ))
        ));
        if !backups.is_null() {
            out.push_str("<div class=\"db-system__footer\">");
            if let Some(at) = backups["last_backup_taken_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            {
                let date = crate::user_profile_view::age_medium(self.list, at.to_utc(), true);
                out.push_str(&format!(
                    "<div class=\"db-system__label\"><span>{}</span></div>",
                    t_with(i18n, &format!("{prefix}.latest_backup"), &[("date", &date)])
                ));
            }
            out.push_str(&format!(
                "<a class=\"db-system__link\" href=\"{}/admin/backups\">{}</a></div>",
                self.admin.base_path,
                escape(&t(i18n, &format!("{prefix}.manage_backups")))
            ));
        }
        out.push_str("</div>");
        out
    }

    fn system_status(&self, data: &Value) -> String {
        let i18n = self.admin.i18n;
        let prefix = "admin.dashboard.sections.system";
        let date = |key: &str| {
            data[key]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.to_utc())
        };
        let discourse = date("discourse_updated_at");
        let dashboard = date("dashboard_updated_at");
        if discourse.is_none() && dashboard.is_none() {
            return String::new();
        }
        let mut out = format!(
            "<div class=\"db-section__row-block db-system__block --status\"><div class=\"db-section__row-block-header\"><h3 class=\"db-section__header\">{} {}</h3></div>",
            icon("clock", Some("db-system__block-icon")),
            escape(&t(i18n, &format!("{prefix}.status")))
        );
        for (at, label) in [
            (discourse, "discourse_updated"),
            (dashboard, "dashboard_updated"),
        ] {
            if let Some(at) = at {
                out.push_str(&format!(
                    "<div class=\"db-system__headline\"><div class=\"db-system__value\">{}</div><div class=\"db-system__label\">{}</div></div>",
                    crate::user_profile_view::age_medium(self.list, at, true),
                    escape(&t(i18n, &format!("{prefix}.{label}")))
                ));
            }
        }
        out.push_str(&format!(
            "<div class=\"db-system__footer\"><a class=\"db-system__link\" href=\"{}/admin/whats-new\">{}</a></div></div>",
            self.admin.base_path,
            escape(&t(i18n, "admin.dashboard.whats_new_in_discourse"))
        ));
        out
    }
}

/// `encodeURIComponent`, as Ember's router writes query values.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `Category.slugFor(category, ":")`
fn category_slug_for(cx: &ListContext, c: &crate::topic_list_view::ListCategory) -> String {
    let own = if c.slug.trim().is_empty() {
        format!("{}-category", c.id)
    } else {
        c.slug.clone()
    };
    match c.parent_id.and_then(|p| cx.categories.get(&p)) {
        Some(parent) => format!("{}:{own}", category_slug_for(cx, parent)),
        None => own,
    }
}

/// `commitsAhead`: the count after " +" in a described version.
fn commits_ahead(pretty: Option<&str>) -> Option<i64> {
    let extra = pretty?.split(" +").nth(1)?;
    let digits: String = extra.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// A JSON number as JavaScript prints it in a template.
fn js_number(v: &Value) -> String {
    match v {
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
            (None, Some(f)) => f.to_string(),
            _ => n.to_string(),
        },
        Value::Null => "null".into(),
        other => other.as_str().unwrap_or_default().to_string(),
    }
}

/// `percentChange`: the metric's change, or computed from its values.
fn percent_change(m: &Value) -> f64 {
    m["percent_change"].as_f64().unwrap_or_else(|| {
        let value = m["value"].as_f64().unwrap_or(0.0);
        let previous = m["previous_value"].as_f64().unwrap_or(0.0);
        (value - previous) / previous * 100.0
    })
}

/// DashboardTraffic#formatHeadlineCount
fn format_headline_count(value: f64) -> String {
    if value >= 1_000_000.0 {
        let formatted = to_number(value / 1_000_000.0, 1);
        let formatted = formatted
            .strip_suffix(".0")
            .or_else(|| formatted.strip_suffix(",0"))
            .map(str::to_string)
            .unwrap_or(formatted.clone());
        return format!("{formatted}M");
    }
    if value >= 1_000.0 {
        return format!("{}k", to_number(js_round(value / 1_000.0), 0));
    }
    to_number(value, 0)
}

/// formatMinutesSeconds
fn format_minutes_seconds(cx: &AdminCx, seconds: f64) -> String {
    let total = seconds.floor() as i64;
    if total < 60 {
        return t_count(cx.i18n, "dates.tiny.x_seconds", total, &[]);
    }
    t_with(
        cx.i18n,
        "dates.tiny.x_minutes_seconds",
        &[
            ("minutes", &(total / 60).to_string()),
            ("seconds", &(total % 60).to_string()),
        ],
    )
}

/// DashboardDateRange's DMenu trigger, with what date-range-picker.js
/// fills its menu from: the preset's label, or the range's dates.
pub fn date_range_trigger(cx: &crate::admin_view::AdminCx, range: &Range) -> String {
    let label = if PERIODS.contains(&range.period.as_str()) {
        t(
            cx.i18n,
            &format!("date_range_picker.presets.{}", range.period),
        )
    } else {
        format_range(range.start, range.end)
    };
    let presets: Vec<Value> = PERIODS
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p,
                "label": t(cx.i18n, &format!("date_range_picker.presets.{p}")),
            })
        })
        .collect();
    format!(
        "<button aria-expanded=\"false\" class=\"btn btn-icon-text fk-d-menu__trigger db-date-range-menu-trigger btn-default db-date-range__trigger\" data-identifier=\"db-date-range-menu\" data-trigger=\"\" type=\"button\" data-presets=\"{}\" data-active=\"{}\" data-from=\"{}\" data-to=\"{}\">{}<span class=\"d-button-label\">{}</span></button>",
        escape(&Value::Array(presets).to_string()),
        escape(&range.period),
        range.start.format("%Y-%m-%d"),
        range.end.format("%Y-%m-%d"),
        icon("calendar-days", None),
        escape(&label)
    )
}

/// moment's `format` for the date tokens the dashboard's strings use.
pub fn moment_format(d: NaiveDate, format: &str) -> String {
    crate::pretty_text::render::local_dates::format_utc(
        d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc(),
        format,
    )
    .unwrap_or_default()
}

/// formatRange: moment's `ll` for each end, one date for a single day.
fn format_range(from: NaiveDate, to: NaiveDate) -> String {
    if from == to {
        return moment_format(from, "ll");
    }
    format!(
        "{} – {}",
        moment_format(from, "ll"),
        moment_format(to, "ll")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_rounds_ties_away_from_zero_as_javascript_does() {
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(100.0, 1), "100.0");
        assert_eq!(to_fixed(-2.5, 0), "-3");
    }

    #[test]
    fn numbers_group_thousands() {
        assert_eq!(to_number(1234567.0, 0), "1,234,567");
        assert_eq!(to_number(40.0, 2), "40.00");
        assert_eq!(to_number(-1000.0, 0), "-1,000");
    }

    #[test]
    fn deltas_format_as_the_dashboard_does() {
        assert_eq!(format_delta_percent(62.5), "+63%");
        assert_eq!(format_delta_percent(-0.42), "-0.4%");
        assert_eq!(format_delta_percent(0.0), "0%");
        assert_eq!(format_delta_percent(-12.0), "-12%");
    }

    #[test]
    fn headline_counts_shorten() {
        assert_eq!(format_headline_count(0.0), "0");
        assert_eq!(format_headline_count(1499.0), "1k");
        assert_eq!(format_headline_count(2_000_000.0), "2M");
        assert_eq!(format_headline_count(2_500_000.0), "2.5M");
    }

    #[test]
    fn the_default_range_is_the_last_30_days() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
        let r = Range::from_params(None, None, None, today);
        assert_eq!(r.period, "last_30_days");
        assert_eq!(r.start, NaiveDate::from_ymd_opt(2026, 9, 15).unwrap());
        let r = Range::from_params(Some("last_3_months"), None, None, today);
        assert_eq!(r.start, NaiveDate::from_ymd_opt(2026, 7, 15).unwrap());
        let r = Range::from_params(Some("custom"), Some("2026-01-02"), None, today);
        assert_eq!(r.period, "last_30_days");
        let r = Range::from_params(
            Some("custom"),
            Some("2026-01-02"),
            Some("2026-02-03"),
            today,
        );
        assert_eq!(r.start, NaiveDate::from_ymd_opt(2026, 1, 2).unwrap());
    }

    #[test]
    fn commits_ahead_reads_the_described_version() {
        assert_eq!(commits_ahead(Some("v2026.10.0-latest +114")), Some(114));
        assert_eq!(commits_ahead(Some("v2026.10.0")), None);
    }
}
