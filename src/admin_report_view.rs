//! The admin report page (/admin/reports/:type): templates/admin-reports/
//! show.gjs with its BackButton, and AdminReport drawn as AdminReportBody
//! draws it: the header (DPageSubheader), the filters (the chart
//! groupings' DSegmentedControl, the date range, the report's own
//! filters), the body's average label, error, or no-data alert, the
//! actions (the modes, export, refresh) and, for a chart, the related
//! items. The body's mode (the table, the charts, posters by member
//! type) is drawn by static/js/admin-report.js from the report the page
//! carries, as the component draws it from the bulk response.

use serde_json::{Map, Value, json};

use crate::admin_view::{AdminCx, t, t_with};
use crate::search_advanced_view::PanelCategory;
use crate::topic_list_view::{ListContext, escape, icon};

/// `REPORT_MODES` that draw a chart (`isChartMode`).
const CHART_MODES: [&str; 3] = ["chart", "stacked_chart", "stacked_line_chart"];

/// The back button: the reports, or the dashboard the page came from.
pub struct Back {
    pub label_key: &'static str,
    pub href: String,
}

pub struct ReportPage<'a> {
    /// `@dataSourceName`, the route's type.
    pub kind: &'a str,
    /// The bulk response's report, None when the request failed.
    pub report: Option<&'a Value>,
    /// The route's `mode` and `chart_grouping` query params.
    pub mode: Option<&'a str>,
    pub chart_grouping: Option<&'a str>,
    /// The model's dates, YYYY-MM-DD.
    pub start_date: &'a str,
    pub end_date: &'a str,
    /// The model's `customFilters` (the `filters` query param).
    pub custom_filters: Option<&'a Map<String, Value>>,
    /// In `dashboard_hidden_reports`.
    pub hidden: bool,
    /// `@showRelatedItems`: the viewer is an admin.
    pub admin: bool,
    pub back: Back,
    /// Site#groups (id, name), for the group filter.
    pub groups: &'a [(i64, String)],
    /// The category chooser's categories.
    pub categories: &'a [PanelCategory],
    /// The report's table limit (reportOptions.table.limit).
    pub limit: i64,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `isPresent(model.data)`
fn present(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => !m.is_empty(),
        Value::String(x) => !x.trim().is_empty(),
        Value::Bool(b) => *b,
        Value::Number(_) => true,
    }
}

/// JavaScript's truthiness (`!!value`).
fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(x) => !x.is_empty(),
        _ => true,
    }
}

/// `parseInt(value, 10)`: the leading digits.
fn parse_int(v: &str) -> Option<i64> {
    let v = v.trim_start();
    let (neg, rest) = match v.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let n: i64 = digits.parse().ok()?;
    Some(if neg { -n } else { n })
}

/// `moment.utc(date).format("YYYY-MM-DD")` of an ISO timestamp.
fn day(v: &Value) -> String {
    s(v).get(..10).unwrap_or_default().to_string()
}

impl ReportPage<'_> {
    fn current_mode(&self) -> Option<String> {
        self.mode.map(str::to_string).or_else(|| {
            self.report
                .and_then(|r| r["modes"].as_array())
                .and_then(|m| m.first())
                .and_then(Value::as_str)
                .map(str::to_string)
        })
    }

    fn error(&self) -> Option<&str> {
        self.report.and_then(|r| r["error"].as_str())
    }

    /// `model.reportUrl`
    fn report_url(&self, cx: &AdminCx, report: &Value) -> String {
        format!(
            "{}/admin/reports/{}?start_date={}&end_date={}",
            cx.base_path,
            s(&report["type"]),
            day(&report["start_date"]),
            day(&report["end_date"])
        )
    }

    pub fn render(&self, cx: &AdminCx, list: &ListContext) -> String {
        let back = format!(
            "<a class=\"{}btn btn-transparent back-button\" href=\"{}{}\">{}{}</a>",
            // LinkTo's active class: the reports route is the current one.
            if self.back.href == "/admin/reports" {
                "active "
            } else {
                ""
            },
            cx.base_path,
            escape(&self.back.href),
            icon("chevron-left", None),
            escape(&t(cx.i18n, self.back.label_key))
        );
        format!(
            "<div class=\"admin-container admin-config-page__main-area\">{back}<div class=\"admin-container admin-config-page__main-area\"><div class=\"admin-config-area\">{}{}</div></div></div>",
            self.report_html(cx, list),
            self.data_script(cx)
        )
    }

    fn report_html(&self, cx: &AdminCx, list: &ListContext) -> String {
        let name = if self.kind.is_empty() {
            self.report
                .map(|r| s(&r["type"]).to_string())
                .unwrap_or_else(|| "undefined".into())
        } else {
            self.kind.to_string()
        };
        let mut out = format!(
            "<div class=\"admin-report is-enabled {} {}\">",
            if self.hidden { "hidden" } else { "is-visible" },
            escape(&name.replace('_', "-"))
        );
        if self.hidden {
            out.push_str("</div>");
            return out;
        }
        out.push_str("<div class=\"conditional-loading-section\">");
        let not_found = self.error() == Some("not_found");
        if let Some(r) = self.report
            && r["legacy"] == Value::Bool(true)
        {
            out.push_str(&format!(
                "<div class=\"alert alert-info\">{} <span>{}</span></div>",
                icon("triangle-exclamation", None),
                escape(&t(cx.i18n, "admin.reports.legacy_warning"))
            ));
        }
        out.push_str("<div class=\"header\">");
        if !not_found {
            out.push_str(
                "<div class=\"d-page-subheader\"><div class=\"d-page-subheader__title-row\">",
            );
            if let Some(r) = self.report
                && let Some(title) = r["title"].as_str().filter(|t| !t.is_empty())
            {
                out.push_str(&format!(
                    "<h2 class=\"d-page-subheader__title\"><a class=\"d-page-subheader__title-link\" href=\"{}\">{}</a></h2>",
                    escape(&self.report_url(cx, r)),
                    escape(title)
                ));
            }
            out.push_str("</div></div>");
        }
        out.push_str("</div><div class=\"chart__wrapper\">");
        // _computeReport turns the filtering off for a report not found.
        let filtering = !not_found;
        let mode = self.current_mode();
        let chart_mode = mode.as_deref().is_some_and(|m| CHART_MODES.contains(&m));
        if filtering {
            out.push_str(&self.filters(cx, chart_mode));
        }
        out.push_str("<div class=\"chart__body\">");
        out.push_str(&self.body(cx, filtering));
        out.push_str("</div>");
        if filtering {
            out.push_str(&self.actions(cx, mode.as_deref()));
        }
        out.push_str("</div>");
        if chart_mode && let Some(r) = self.report {
            out.push_str(&self.related_items(cx, list, r));
        }
        out.push_str("</div></div>");
        out
    }

    fn filters(&self, cx: &AdminCx, chart_mode: bool) -> String {
        let mut out = String::from("<div class=\"chart__filters\">");
        let kind = self
            .report
            .map(|r| s(&r["type"]).to_string())
            .unwrap_or_default();
        if chart_mode {
            // The checked grouping, its slider, and the daily one's
            // disabling are admin-report.js's: they come of the options
            // it builds from the filled data.
            out.push_str(&format!(
                "<fieldset class=\"d-segmented-control chart-groupings\"><legend class=\"d-segmented-control__legend\">{}</legend><span class=\"d-segmented-control__slider\"></span>",
                escape(&t(cx.i18n, "admin.dashboard.reports.chart_group_period"))
            ));
            for id in ["daily", "weekly", "monthly"] {
                out.push_str(&format!(
                    "<label class=\"d-segmented-control__label chart-grouping {id}\"><input class=\"d-segmented-control__input\" name=\"chart-grouping-{}\" type=\"radio\" value=\"{id}\"><span class=\"d-segmented-control__text\">{}</span></label>",
                    escape(&kind),
                    escape(&t(cx.i18n, &format!("admin.dashboard.reports.{id}")))
                ));
            }
            out.push_str("</fieldset>");
        }
        if self
            .report
            .is_some_and(|r| r["dates_filtering"] == Value::Bool(true))
        {
            out.push_str(&format!(
                "<div class=\"chart__dates\">{}</div>",
                crate::admin_logs_view::date_range(cx, Some(self.start_date), Some(self.end_date))
            ));
        }
        out.push_str("<div class=\"chart__additional-filters\">");
        let empty = Vec::new();
        let available = self
            .report
            .and_then(|r| r["available_filters"].as_array())
            .unwrap_or(&empty);
        for filter in available {
            let id = s(&filter["id"]);
            out.push_str(&format!(
                "<div class=\"chart__filter --{}\"><div class=\"input\">{}</div></div>",
                escape(id),
                self.filter(cx, filter)
            ));
        }
        out.push_str("</div></div>");
        out
    }

    /// `reportFilterComponent(filter)` drawn.
    fn filter(&self, cx: &AdminCx, filter: &Value) -> String {
        let id = s(&filter["id"]);
        let filter_by = |name: &str| t_with(cx.i18n, "select_kit.filter_by", &[("name", name)]);
        let combo = |classes: &str, name: &str, value: Option<&str>, label: &str, clear: bool| {
            let fb = escape(&filter_by(name));
            let value_attr = value
                .map(|v| format!(" data-value=\"{}\"", escape(v)))
                .unwrap_or_default();
            format!(
                "<details class=\"select-kit single-select combobox combo-box{classes}{}\" data-filter=\"{}\"><summary aria-label=\"{fb}\" name=\"{fb}\" data-name=\"{n}\"{value_attr} tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\"{value_attr} title=\"{n}\"><span class=\"name\">{label}</span></div>{}{}</div></summary><div class=\"select-kit-body\"></div></details>",
                if value.is_some() {
                    " has-selection"
                } else {
                    ""
                },
                escape(id),
                if clear {
                    format!(
                        "<button aria-label=\"{}\" class=\"btn no-text btn-icon btn-clear btn-transparent\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                        escape(&t(cx.i18n, "clear_input")),
                        icon("xmark", None)
                    )
                } else {
                    String::new()
                },
                icon("angle-down", Some("angle-icon")),
                n = escape(name),
            )
        };
        let default = &filter["default"];
        let default_str = match default {
            Value::Null => None,
            Value::String(v) if v.is_empty() => None,
            Value::String(v) => Some(v.clone()),
            other => Some(other.to_string()),
        };
        match s(&filter["type"]) {
            "bool" => format!(
                "<div class=\"ember-view\"><label class=\"chart__filter-label checkbox-label\"><input class=\"ember-checkbox\" type=\"checkbox\" data-filter=\"{}\"{}>{}</label></div>",
                escape(id),
                if js_truthy(default) { " checked" } else { "" },
                escape(&t(
                    cx.i18n,
                    &format!("admin.dashboard.reports.filters.{id}.label")
                ))
            ),
            "category" => {
                let selected = default_str
                    .as_deref()
                    .and_then(|v| v.parse::<i64>().ok())
                    .and_then(|v| self.categories.iter().find(|c| c.id == v));
                let all = t(cx.i18n, "category.all");
                let html = match selected {
                    Some(c) => combo(
                        " category-chooser search-advanced-category-chooser",
                        &c.name,
                        Some(&c.id.to_string()),
                        &c.label,
                        true,
                    ),
                    None => combo(
                        " category-chooser search-advanced-category-chooser",
                        &all,
                        None,
                        &escape(&all),
                        false,
                    ),
                };
                format!("<div class=\"ember-view\">{html}</div>")
            }
            "group" => {
                let selected = default_str
                    .as_deref()
                    .and_then(parse_int)
                    .and_then(|v| self.groups.iter().find(|(id, _)| *id == v));
                let html = match selected {
                    Some((gid, name)) => {
                        combo("", name, Some(&gid.to_string()), &escape(name), false)
                    }
                    None => {
                        let none = t(cx.i18n, "admin.dashboard.reports.groups");
                        combo("", &none, None, &escape(&none), false)
                    }
                };
                format!("<div class=\"group-filter\">{html}</div>")
            }
            "list" => {
                let choices = filter["choices"].as_array().cloned().unwrap_or_default();
                let selected = default_str.as_deref().and_then(|v| {
                    choices.iter().find(|c| match c {
                        Value::Object(_) => crate::params::scalar(&c["id"]).as_deref() == Some(v),
                        other => crate::params::scalar(other).as_deref() == Some(v),
                    })
                });
                let html = match (selected, &default_str) {
                    (Some(c), Some(v)) => {
                        let name = match c {
                            Value::Object(_) => s(&c["name"]).to_string(),
                            other => crate::params::scalar(other).unwrap_or_default(),
                        };
                        combo("", &name, Some(v), &escape(&name), false)
                    }
                    // allowAny: a value not among the choices shows as itself.
                    (None, Some(v)) => combo("", v, Some(v), &escape(v), false),
                    _ => {
                        let none = t(cx.i18n, "admin.dashboard.report_filter_any");
                        combo("", &none, None, &escape(&none), false)
                    }
                };
                format!("<div class=\"ember-view\">{html}</div>")
            }
            "category_list" => {
                let ids: Vec<i64> = match default {
                    Value::Array(a) => a
                        .iter()
                        .filter_map(|v| crate::params::scalar(v).and_then(|v| v.parse().ok()))
                        .collect(),
                    _ => Vec::new(),
                };
                let chosen: Vec<&PanelCategory> = ids
                    .iter()
                    .filter_map(|id| self.categories.iter().find(|c| c.id == *id))
                    .collect();
                let names: Vec<&str> = chosen.iter().map(|c| c.name.as_str()).collect();
                let values: Vec<String> = chosen.iter().map(|c| c.id.to_string()).collect();
                let text = if names.is_empty() {
                    t(cx.i18n, "category.all")
                } else {
                    names.join(", ")
                };
                format!(
                    "<div class=\"ember-view\"><details class=\"select-kit multi-select multiple-categories-selector{}\" data-filter=\"{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details></div>",
                    if chosen.is_empty() {
                        ""
                    } else {
                        " has-selection"
                    },
                    escape(id),
                    escape(&names.join(",")),
                    escape(&values.join(",")),
                    escape(&text),
                    icon("plus", Some("angle-icon"))
                )
            }
            "groups" => format!(
                "<div class=\"ember-view\"><button class=\"btn btn-icon-text btn-default report-filter-groups__button\" type=\"button\" data-filter=\"{}\">{}<span class=\"d-button-label\">{}</span></button></div>",
                escape(id),
                icon("plus", None),
                escape(&t(
                    cx.i18n,
                    "admin.dashboard.sections.engagement.whos_posting.add_group"
                ))
            ),
            _ => String::new(),
        }
    }

    fn body(&self, cx: &AdminCx, filtering: bool) -> String {
        let mut out = String::new();
        let Some(r) = self.report else {
            return format!(
                "<div class=\"alert alert-info report-alert no-data\">{} <span>{}</span></div>",
                icon("chart-pie", None),
                escape(&t(cx.i18n, "admin.dashboard.reports.no_data"))
            );
        };
        if r["average"] == Value::Bool(true) && filtering {
            out.push_str(&format!(
                "<div class=\"average-chart\">{}</div>",
                escape(&t(cx.i18n, "admin.dashboard.reports.average_chart_label"))
            ));
        }
        let alert = |class: &str, key: &str| {
            format!(
                "<div class=\"alert alert-error report-alert {class}\">{} <span>{}</span></div>",
                icon("triangle-exclamation", None),
                escape(&t(cx.i18n, key))
            )
        };
        match self.error() {
            Some("timeout") => out.push_str(&alert("timeout", "admin.dashboard.timeout_error")),
            Some("exception") => {
                out.push_str(&alert("exception", "admin.dashboard.exception_error"))
            }
            Some("not_found") => {
                out.push_str(&alert("not-found", "admin.dashboard.not_found_error"))
            }
            // The mode, drawn by admin-report.js.
            _ if present(&r["data"]) => {}
            _ => {
                let no_data = t(cx.i18n, "admin.dashboard.reports.no_data");
                let text = match r["title"].as_str().filter(|t| !t.is_empty()) {
                    Some(title) => format!("{title} \u{2014} {no_data}"),
                    None => no_data,
                };
                out.push_str(&format!(
                    "<div class=\"alert alert-info report-alert no-data\">{} <a class=\"report-url\" href=\"{}\"><span>{}</span></a></div>",
                    icon("chart-pie", None),
                    escape(&self.report_url(cx, r)),
                    escape(&text)
                ));
            }
        }
        out
    }

    fn actions(&self, cx: &AdminCx, mode: Option<&str>) -> String {
        let mut out = String::from("<div class=\"chart__actions\">");
        let modes: Vec<&str> = self
            .report
            .and_then(|r| r["modes"].as_array())
            .map(|m| m.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if modes.len() > 1 {
            out.push_str("<div class=\"chart__modes\">");
            for m in &modes {
                out.push_str(&format!(
                    "<button class=\"btn no-text btn-icon btn-default mode-btn {}{}\" type=\"button\" data-mode=\"{}\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                    escape(m),
                    if Some(*m) == mode { " btn-primary" } else { "" },
                    escape(m),
                    icon(if *m == "table" { "table" } else { "signal" }, None)
                ));
            }
            out.push_str("</div>");
        }
        let button = |class: &str, icon_name: &str, key: &str| {
            format!(
                "<div class=\"control\"><div class=\"input\"><button class=\"btn btn-icon-text {class}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div></div>",
                icon(icon_name, None),
                escape(&t(cx.i18n, key))
            )
        };
        out.push_str(&button(
            "btn-default export-csv-btn",
            "download",
            "admin.export_csv.button_text",
        ));
        let show_refresh = self.report.is_some_and(|r| {
            r["dates_filtering"] == Value::Bool(true)
                || r["available_filters"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
        });
        if show_refresh {
            out.push_str(&button(
                "refresh-report-btn btn-default",
                "arrows-rotate",
                "admin.dashboard.reports.refresh_report",
            ));
        }
        out.push_str("</div>");
        out
    }

    /// AdminReportRelatedItems, for an admin's chart with related users.
    /// A plugin's own related items (discourse-solved's solved topics)
    /// are refused by admin-report.js.
    fn related_items(&self, cx: &AdminCx, list: &ListContext, r: &Value) -> String {
        if !self.admin {
            return String::new();
        }
        let users = r["related_items"]["users"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if users.is_empty() {
            return String::new();
        }
        let kind = s(&r["type"]);
        let total = r["related_items_totals"]["users"]
            .as_i64()
            .filter(|n| *n != 0)
            .unwrap_or(users.len() as i64);
        let format = t(cx.i18n, "dates.long_with_year_no_time");
        let long = |date: &str| {
            crate::pretty_text::render::local_dates::format_utc(
                format!("{date}T00:00:00Z")
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .unwrap_or_default(),
                &format,
            )
            .unwrap_or_default()
        };
        let mut out = format!(
            "<section class=\"admin-report-related-items\"><h2>{}</h2><p class=\"admin-report-related-items__description\">{}",
            escape(&t(cx.i18n, "admin.reports.related_items.users.title")),
            escape(&t_with(
                cx.i18n,
                &format!("admin.reports.related_items.users.{kind}.description"),
                &[
                    ("startDate", &long(self.start_date)),
                    ("endDate", &long(self.end_date))
                ]
            ))
        );
        if (users.len() as i64) < total {
            out.push_str(&format!(
                " <span class=\"admin-report-related-items__limit\">{}</span>",
                escape(&t_with(
                    cx.i18n,
                    "admin.reports.related_items.showing",
                    &[
                        ("shown", &users.len().to_string()),
                        ("total", &total.to_string())
                    ]
                ))
            ));
        }
        out.push_str(&format!(
            "</p><table class=\"table admin-report-related-items__table\"><thead><tr><th class=\"admin-report-related-items__user-cell\">{}</th><th class=\"admin-report-related-items__timestamp-cell\">{}</th></tr></thead><tbody>",
            escape(&t(cx.i18n, "admin.reports.related_items.labels.user")),
            escape(&t(
                cx.i18n,
                if kind == "new_contributors" {
                    "admin.reports.related_items.labels.first_post"
                } else {
                    "admin.reports.related_items.labels.created_at"
                }
            ))
        ));
        for item in &users {
            let user = &item["user"];
            let username = s(&user["username"]);
            let name = s(&user["name"]);
            let at = s(&item["timestamp"])
                .parse::<chrono::DateTime<chrono::Utc>>()
                .ok();
            out.push_str(&format!(
                "<tr><td class=\"admin-report-related-items__user-cell\"><a class=\"admin-report-related-items__user-link\" href=\"{}/u/{}\"><img alt=\"\" width=\"24\" height=\"24\" src=\"{}\" class=\"avatar admin-report-related-items__avatar\" title=\"{}\"><span class=\"admin-report-related-items__user-identity\"><span class=\"admin-report-related-items__username\">{}</span>{}</span></a></td><td class=\"admin-report-related-items__timestamp-cell\"><time datetime=\"{}\">{}</time></td></tr>",
                cx.base_path,
                escape(&username.to_lowercase()),
                escape(&s(&user["avatar_template"]).replace("{size}", "24")),
                escape(username),
                escape(username),
                if name.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<span class=\"admin-report-related-items__name\">{}</span>",
                        escape(name)
                    )
                },
                escape(s(&item["timestamp"])),
                at.map(|at| crate::user_profile_view::age_medium(list, at, true))
                    .unwrap_or_default()
            ));
        }
        out.push_str("</tbody></table></section>");
        out
    }

    /// What admin-report.js draws the mode with: the report, the options
    /// the route and the component give it, and the strings.
    fn data_script(&self, cx: &AdminCx) -> String {
        let categories: Vec<Value> = self
            .categories
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "name": c.name,
                    "parentSlug": c.parent_slug,
                    "slug": c.slug,
                    "row": c.row,
                    "label": c.label,
                })
            })
            .collect();
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|(id, name)| json!({"value": id, "name": name}))
            .collect();
        let data = json!({
            "type": self.kind,
            "report": self.report,
            "mode": self.mode,
            "chartGrouping": self.chart_grouping,
            "startDate": self.start_date,
            "endDate": self.end_date,
            "customFilters": self.custom_filters,
            "limit": self.limit,
            "admin": self.admin,
            "avatarSizes": cx.settings.get("avatar_sizes").map(|v| v.to_s()).unwrap_or_default(),
            "categories": categories,
            "groups": groups,
            "strings": strings(cx),
        });
        format!(
            "<script type=\"application/json\" id=\"admin-report-data\">{}</script>",
            data.to_string().replace('<', "\\u003c")
        )
    }
}

/// The strings admin-report.js renders with, placeholders intact.
fn strings(cx: &AdminCx) -> Value {
    let mut keys: Vec<String> = [
        "admin.dashboard.reports.value_so_far",
        "admin.reports.chart.total.one",
        "admin.reports.chart.total.other",
        "number.short.thousands",
        "number.short.millions",
        "admin.dashboard.reports.totals_for_sample",
        "admin.dashboard.reports.total",
        "admin.dashboard.reports.average_for_sample",
        "admin.dashboard.reports.sort_button",
        "admin.dashboard.reports.sort_button_current",
        "admin.dashboard.reports.sort_ascending",
        "admin.dashboard.reports.sort_descending",
        "admin.dashboard.reports.no_data",
        "admin.dashboard.timeout_error",
        "admin.dashboard.exception_error",
        "admin.dashboard.not_found_error",
        "admin.dashboard.too_many_requests",
        "admin.dashboard.reports.posters_by_member_type_members.users_count.one",
        "admin.dashboard.reports.posters_by_member_type_members.users_count.other",
        "admin.dashboard.reports.posters_by_member_type_members.posts_count.one",
        "admin.dashboard.reports.posters_by_member_type_members.posts_count.other",
        "admin.dashboard.reports.posters_by_member_type_members.title",
        "admin.dashboard.reports.posters_by_member_type_members.deselect",
        "admin.reports.related_items.table_summary.trigger",
        "admin.reports.related_items.table_summary.users_title",
        "admin.reports.related_items.table_summary.loading",
        "admin.reports.related_items.table_summary.error",
        "admin.reports.related_items.table_summary.empty",
        "admin.reports.related_items.showing",
        "dates.long_with_year_no_time",
        "dates.tiny.less_than_x_seconds.one",
        "dates.tiny.less_than_x_seconds.other",
        "dates.tiny.x_seconds.one",
        "dates.tiny.x_seconds.other",
        "dates.tiny.less_than_x_minutes.one",
        "dates.tiny.less_than_x_minutes.other",
        "dates.tiny.x_minutes.one",
        "dates.tiny.x_minutes.other",
        "dates.tiny.about_x_hours.one",
        "dates.tiny.about_x_hours.other",
        "dates.tiny.x_days.one",
        "dates.tiny.x_days.other",
        "dates.tiny.x_months.one",
        "dates.tiny.x_months.other",
        "dates.tiny.about_x_years.one",
        "dates.tiny.about_x_years.other",
        "dates.tiny.over_x_years.one",
        "dates.tiny.over_x_years.other",
        "dates.tiny.almost_x_years.one",
        "dates.tiny.almost_x_years.other",
        "number.human.storage_units.format",
        "number.human.storage_units.units.byte.one",
        "number.human.storage_units.units.byte.other",
        "number.human.storage_units.units.kb",
        "number.human.storage_units.units.mb",
        "number.human.storage_units.units.gb",
        "number.human.storage_units.units.tb",
        "select_kit.filter_by",
        "select_kit.filter_placeholder",
        "select_kit.no_content",
        "select_kit.results_count.one",
        "select_kit.results_count.other",
        "clear_input",
        "category.all",
        "admin.dashboard.reports.groups",
        "admin.dashboard.report_filter_any",
    ]
    .iter()
    .map(|k| k.to_string())
    .collect();
    keys.sort();
    let map: Map<String, Value> = keys
        .into_iter()
        .map(|k| {
            let v = crate::admin_view::t_opt(cx.i18n, &k).unwrap_or_else(|| format!("[en.{k}]"));
            (k, Value::String(v))
        })
        .collect();
    Value::Object(map)
}
