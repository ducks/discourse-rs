//! The users directory (/u) as the Ember app renders it (templates/users.gjs
//! with the PeriodChooser, DirectoryTable, DirectoryItem and
//! DTableHeaderToggle components), from the directory_items#index page the
//! route loads. static/js/users-directory.js does the rest in the browser:
//! the period menu, sorting, the name filter and loading more rows.

use chrono::{DateTime, Duration, Months, Utc};
use serde_json::{Value, json};

use crate::pretty_text::render::local_dates::format_utc;
use crate::topic_list_view::{ListContext, escape, icon, t, t_count, t_with};

/// `site.periods` (TopTopic.periods), in the period chooser's order.
pub const PERIODS: [&str; 6] = ["all", "yearly", "quarterly", "monthly", "weekly", "daily"];

/// An enabled directory column (`/directory-columns.json`): an automatic
/// one, or a user field's (named after the field).
pub struct Column {
    pub name: String,
    pub icon: Option<String>,
    pub user_field_id: Option<i32>,
}

/// `encodeURIComponent`, as Ember's router writes a query param.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// DirectoryItemUserFieldValue: the item's values for the field (split on
/// commas, dashes as spaces), linked to the directory filtered by each
/// when the field is searchable, else written as one list; `-` without.
fn user_field_value(cx: &ListContext, item: &Value, field_id: i32) -> String {
    let data = &item["user"]["user_fields"][field_id.to_string()];
    let joined = match &data["value"] {
        Value::Array(values) if !values.is_empty() => values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .collect::<Vec<_>>()
            .join(","),
        _ => return "-".into(),
    };
    let values: Vec<String> = joined
        .split(',')
        .map(|v| v.replace('-', " ").trim().to_string())
        .collect();
    if data["searchable"].as_bool() == Some(true) {
        values
            .iter()
            .map(|v| {
                format!(
                    "<a class=\"directory-value-list-item\" href=\"{}/u?name={}\">{}</a>",
                    cx.base_path,
                    encode_uri_component(v),
                    escape(v)
                )
            })
            .collect()
    } else {
        escape(&values.join(","))
    }
}

/// The users route's model and the controller's query params.
pub struct Directory<'a> {
    pub columns: &'a [Column],
    pub items: &'a [Value],
    /// `totalRows`
    pub total: i64,
    /// `resultSetMeta.last_updated_at`
    pub last_updated_at: Option<DateTime<Utc>>,
    /// `resultSetMeta.load_more_directory_items`
    pub load_more_url: &'a str,
    pub period: &'a str,
    pub order: &'a str,
    pub asc: bool,
    pub name: &'a str,
    /// The current user's id, staff flag.
    pub viewer_id: Option<i64>,
    pub viewer_staff: bool,
    /// `groupOptions` for a member: each group's name and label
    /// (`full_name || name`); none for a visitor.
    pub groups: &'a [(String, String)],
    /// The `group` query param.
    pub group: Option<&'a str>,
}

/// `I18n.t("filters.top." + TITLE_SUBS[period])`
fn period_name(cx: &ListContext, period: &str) -> String {
    let sub = match period {
        "all" => "all_time",
        "yearly" => "this_year",
        "quarterly" => "this_quarter",
        "monthly" => "this_month",
        "daily" => "today",
        _ => "this_week",
    };
    t(cx, &format!("filters.top.{sub}"))
}

/// `periodTitle(period, {showDateRange: true, fullDay: false})`: the
/// period's name and its dates up to now.
fn period_title(cx: &ListContext, period: &str) -> String {
    let finish = cx.now;
    let fmt = |at: DateTime<Utc>, key: &str| format_utc(at, &t(cx, key)).unwrap_or_default();
    let range =
        |start: DateTime<Utc>, key: &str| format!("{} – {}", fmt(start, key), fmt(finish, key));
    let months = |n: u32| finish.checked_sub_months(Months::new(n)).unwrap_or(finish);
    let dates = match period {
        "yearly" => range(months(12), "dates.long_with_year_no_time"),
        "quarterly" => range(months(3), "dates.long_no_year_no_time"),
        "weekly" => range(finish - Duration::days(6), "dates.long_no_year_no_time"),
        "monthly" => range(months(1), "dates.long_no_year_no_time"),
        "daily" => fmt(finish, "dates.full_no_year_no_time"),
        _ => String::new(),
    };
    format!(
        "<span class=\"date-section\">{}</span><span class=\"top-date-string\">{}</span>",
        escape(&period_name(cx, period)),
        escape(&dates)
    )
}

/// The group filter, a ComboBox with `none: "directory.group.all"`: the
/// selected group's label, or all groups; its rows the groups, after the
/// none row once one is selected.
fn group_selector(cx: &ListContext, groups: &[(String, String)], group: Option<&str>) -> String {
    let all = t(cx, "directory.group.all");
    let selected = group.and_then(|g| groups.iter().find(|(name, _)| name == g));
    let (label, value) = match selected {
        Some((name, label)) => (escape(label), format!(" data-value=\"{}\"", escape(name))),
        None => (escape(&all), String::new()),
    };
    let filter_by = escape(&t_with(cx, "select_kit.filter_by", &[("name", &label)]));
    let mut rows = String::new();
    let mut index = 0;
    if selected.is_some() {
        rows.push_str(&format!(
            "<li aria-checked=\"false\" role=\"menuitemradio\" data-index=\"0\" data-name=\"{all}\" title=\"{all}\" tabindex=\"0\" class=\"select-kit-row is-none none\"><span class=\"name\">{all}</span></li>",
            all = escape(&all)
        ));
        index = 1;
    }
    for (name, row_label) in groups {
        let is_selected = Some(name.as_str()) == group;
        rows.push_str(&format!(
            "<li aria-checked=\"{is_selected}\" role=\"menuitemradio\" data-index=\"{index}\" data-name=\"{l}\" data-value=\"{n}\" title=\"{l}\" tabindex=\"0\" class=\"select-kit-row{}\"><span class=\"name\">{l}</span></li>",
            if is_selected { " is-highlighted is-selected" } else { "" },
            l = escape(row_label),
            n = escape(name),
        ));
        index += 1;
    }
    format!(
        "<details class=\"select-kit single-select combobox combo-box{} directory-group-selector\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"{label}\"{value} tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{label}\"{value} title=\"{label}\"><span class=\"name\">{label}</span></div> {}</div></summary>\
         <div class=\"select-kit-body\"><div class=\"select-kit-filter\"></div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        if selected.is_some() {
            " has-selection"
        } else {
            ""
        },
        icon("angle-down", Some("angle-icon")),
    )
}

/// PeriodChooser (a DropdownSelectBox): its header, and its rows for
/// users-directory.js to open.
fn period_chooser(cx: &ListContext, period: &str) -> String {
    let mut rows = String::new();
    for (i, p) in PERIODS.iter().enumerate() {
        let selected = *p == period;
        rows.push_str(&format!(
            "<li aria-checked=\"{selected}\" role=\"menuitemradio\" data-index=\"{i}\" data-name=\"{p}\" data-value=\"{p}\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row period-chooser-row{}\"><span class=\"selection-indicator\"></span> <span class=\"period-title\">{}</span></li>",
            if selected { " is-highlighted is-selected" } else { "" },
            period_title(cx, p)
        ));
    }
    // The selection's name: none for a period not among the rows.
    let period = escape(period);
    let name = if PERIODS.contains(&period.as_str()) {
        period.as_str()
    } else {
        ""
    };
    format!(
        "<details class=\"select-kit single-select dropdown-select-box period-chooser has-selection\"><summary aria-label=\"{}\" name=\"{}\" data-name=\"{name}\" data-value=\"{period}\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header period-chooser-header btn-flat btn btn-icon-text\"><h2 class=\"selected-name\">{}</h2> {}</summary>\
         <div class=\"select-kit-body\"><div class=\"select-kit-filter\"></div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        escape(&t(cx, "period_chooser.aria_label")),
        escape(&t_with(cx, "select_kit.filter_by", &[("name", name)])),
        period_title(cx, &period),
        icon("angle-down", Some("angle-icon")),
    )
}

/// The label a column's header and cells show: `directory.<name>_long`,
/// else `directory.<name>` (DTableHeaderToggle#label).
fn header_label(cx: &ListContext, field: &str) -> String {
    let key = if field == "username" {
        field.to_string()
    } else {
        format!("directory.{field}")
    };
    cx.i18n
        .t(&format!("js.{key}_long"))
        .map(str::to_string)
        .unwrap_or_else(|| t(cx, &key))
}

/// DTableHeaderToggle
/// (`translated`: a user field's column, labelled with the field's name.)
fn header_toggle(
    cx: &ListContext,
    d: &Directory,
    field: &str,
    icon_name: Option<&str>,
    translated: bool,
) -> String {
    let current = d.order == field;
    let (aria_sort, pressed, chevron) = match (current, d.asc) {
        (true, true) => ("ascending", "mixed", Some("chevron-up")),
        (true, false) => ("descending", "true", Some("chevron-down")),
        _ => ("none", "false", None),
    };
    let label = if translated {
        field.to_string()
    } else {
        header_label(cx, field)
    };
    let mut criteria = String::new();
    if icon_name == Some("heart") {
        criteria.push_str(&t_count(cx, "likes_lowercase", 2, &[]));
        criteria.push(' ');
    }
    criteria.push_str(&label);
    // A click sorts by the field, descending, or by the current field the
    // other way round (toggleProperties).
    let next_asc = current && !d.asc;
    let field_attr = escape(field);
    format!(
        "<div aria-sort=\"{aria_sort}\" class=\"directory-table__column-header sortable\" role=\"columnheader\" data-order=\"{field_attr}\"{}><div aria-label=\"{}\" aria-pressed=\"{pressed}\" class=\"header-contents\" id=\"table-header-toggle-{}\" role=\"button\" tabindex=\"0\"><span class=\"text\">{}{}{}</span></div></div>",
        if next_asc { " data-asc=\"true\"" } else { "" },
        escape(&t_with(
            cx,
            "directory.sort.label",
            &[("criteria", &criteria)]
        )),
        escape(&field.split_whitespace().collect::<String>()),
        icon_name
            .map(|i| format!("{} ", icon(i, None)))
            .unwrap_or_default(),
        escape(&label),
        chevron
            .map(|c| format!(" {}", icon(c, None)))
            .unwrap_or_default(),
    )
}

/// A cell's value: `number`, the short form (a missing one is 0).
fn value(cx: &ListContext, v: &Value) -> String {
    crate::topic_list_view::number(cx, v.as_i64().unwrap_or(0))
}

/// DirectoryItem
pub fn row(cx: &ListContext, d: &Directory, item: &Value) -> String {
    let me = d.viewer_id.is_some() && item["user"]["id"].as_i64() == d.viewer_id;
    let mut out = format!(
        "<div class=\"directory-table__row{}\" role=\"row\"><div class=\"directory-table__cell\" role=\"rowheader\">{}</div>",
        if me { " me" } else { "" },
        crate::user_info_view::user_info(cx, &item["user"], Some(3))
    );
    for c in d.columns {
        if let Some(field_id) = c.user_field_id {
            out.push_str(&format!(
                "<div class=\"directory-table__cell--user-field\" role=\"cell\"><span class=\"directory-table__label\"><span>{}</span></span> <span class=\"directory-table__value--user-field\">{}</span></div>",
                escape(&c.name),
                user_field_value(cx, item, field_id)
            ));
            continue;
        }
        let count = item[&c.name].as_i64().unwrap_or(0);
        out.push_str(&format!(
            "<div class=\"directory-table__cell\" role=\"cell\"><span class=\"directory-table__label\"><span>{}{}</span></span> <span class=\"directory-table__value\">{}</span></div>",
            c.icon
                .as_deref()
                .map(|i| format!("{} ", icon(i, None)))
                .unwrap_or_default(),
            escape(&cell_label(cx, &c.name, count)),
            value(cx, &item[&c.name]),
        ));
    }
    if d.period == "all" {
        let time_read = match item["time_read"].as_i64() {
            Some(seconds) => crate::user_profile_view::duration(cx, seconds, "tiny"),
            None => "&mdash;".into(),
        };
        out.push_str(&format!(
            "<div class=\"directory-table__cell time-read\" role=\"cell\"><span class=\"directory-table__label\"><span>{}</span></span> <span class=\"directory-table__value\">{time_read}</span></div>",
            escape(&t(cx, "directory.time_read"))
        ));
    }
    out.push_str("</div>");
    out
}

/// directoryItemLabel: `directory.<name>` with the count.
fn cell_label(cx: &ListContext, name: &str, count: i64) -> String {
    let key = format!("js.directory.{name}");
    let count_s = count.to_string();
    cx.i18n
        .t_with(&key, &[("count", &count_s)])
        .or_else(|| cx.i18n.t_count(&key, count, &[]))
        .unwrap_or(key)
}

/// What users-directory.js needs to draw the rows it loads as the server
/// draws them: the columns with their labels, the strings, the paging.
fn client_config(cx: &ListContext, d: &Directory) -> Value {
    let string = |key: &str| t(cx, key);
    let tiny = |key: &str| {
        json!({
            "one": string(&format!("dates.tiny.{key}.one")),
            "other": string(&format!("dates.tiny.{key}.other")),
        })
    };
    json!({
        "basePath": cx.base_path,
        "columns": d.columns.iter().map(|c| json!({
            "name": c.name,
            "icon": c.icon,
            "userFieldId": c.user_field_id,
            "label": json!({
                "one": cell_label(cx, &c.name, 1),
                "other": cell_label(cx, &c.name, 2),
            }),
        })).collect::<Vec<_>>(),
        "showTimeRead": d.period == "all",
        "timeRead": string("directory.time_read"),
        "meId": d.viewer_id,
        "prioritizeName": cx.settings.prioritize_name,
        "total": d.total,
        "loaded": d.items.len(),
        "loadMoreUrl": d.load_more_url,
        "thousands": string("number.short.thousands"),
        "millions": string("number.short.millions"),
        "totalRows": json!({
            "one": string("directory.total_rows.one"),
            "other": string("directory.total_rows.other"),
        }),
        "noResultsWithSearch": string("directory.no_results_with_search"),
        "tiny": {
            "less_than_x_minutes": tiny("less_than_x_minutes"),
            "x_minutes": tiny("x_minutes"),
            "about_x_hours": tiny("about_x_hours"),
            "x_days": tiny("x_days"),
            "x_months": tiny("x_months"),
            "about_x_years": tiny("about_x_years"),
            "over_x_years": tiny("over_x_years"),
            "almost_x_years": tiny("almost_x_years"),
        },
    })
}

/// The `#main-outlet` content of /u.
pub fn render(cx: &ListContext, d: &Directory) -> String {
    let mut out = String::from(
        "<section><div><div class=\"container\"><div class=\"users-directory directory\"><span></span><div class=\"directory-controls\"><div class=\"period-controls\">",
    );
    out.push_str(&period_chooser(cx, d.period));
    if let Some(at) = d.last_updated_at {
        out.push_str(&format!(
            "<div class=\"directory-last-updated\">{} {}</div>",
            escape(&t(cx, "directory.last_updated")),
            escape(&crate::topic_list_view::long_date(cx, at))
        ));
    }
    out.push_str("</div><div class=\"inline-form\"><label class=\"total-rows\">");
    if d.total > 0 {
        out.push_str(&escape(&t_count(cx, "directory.total_rows", d.total, &[])));
    }
    out.push_str(&format!(
        "</label> <input class=\"filter-name no-blur\" placeholder=\"{}\" value=\"{}\" type=\"text\">",
        escape(&t(cx, "directory.filter_name")),
        escape(d.name)
    ));
    // showGroupFilter: a member with groups to filter by.
    if d.viewer_id.is_some() && !d.groups.is_empty() {
        out.push(' ');
        out.push_str(&group_selector(cx, d.groups, d.group));
    }
    if d.viewer_staff {
        // The Edit Directory Columns modal is not drawn yet.
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-default open-edit-columns-btn\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            icon("wrench", None)
        ));
    }
    out.push_str("</div></div><div class=\"loading-container\">");
    if d.items.is_empty() {
        let body = if !d.name.is_empty() {
            escape(&t(cx, "directory.no_results_with_search"))
        } else if d.viewer_staff {
            t_with(
                cx,
                "directory.no_results.extra_body",
                &[("basePath", cx.base_path)],
            )
        } else {
            escape(&t(cx, "directory.no_results.body"))
        };
        out.push_str(&format!(
            "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__body\"><p data-test-body>{body}</p></div></div></div>"
        ));
    } else {
        // DirectoryTable#setupTable's grid columns.
        let count = d.columns.len() + usize::from(d.period == "all");
        out.push_str(&format!(
            "<div class=\"horizontal-scroll-sync__container directory-table-container\"><div class=\"horizontal-scroll-sync__top-scroll\"><div class=\"horizontal-scroll-sync__fake-content\"></div></div><div class=\"horizontal-scroll-sync__content\"><div class=\"directory-table\" role=\"table\" style=\"grid-template-columns: minmax(15em, 3fr) repeat({count}, minmax(max-content, 1fr));\"><div class=\"directory-table__header\">"
        ));
        out.push_str(&header_toggle(cx, d, "username", None, false));
        for c in d.columns {
            out.push_str(&header_toggle(
                cx,
                d,
                &c.name,
                c.icon.as_deref(),
                c.user_field_id.is_some(),
            ));
        }
        if d.period == "all" {
            out.push_str(&format!(
                "<div class=\"directory-table__column-header\"><div class=\"header-contents\">{}</div></div>",
                escape(&t(cx, "directory.time_read"))
            ));
        }
        out.push_str("</div><div class=\"directory-table__body\">");
        for item in d.items {
            out.push_str(&row(cx, d, item));
        }
        out.push_str("</div></div></div></div><div class=\"loading-container\"></div>");
    }
    out.push_str("</div></div></div>");
    // DLoadMore's sentinel, and the client's configuration.
    out.push_str("<div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div></section>");
    out.push_str(&format!(
        "<script type=\"application/json\" id=\"users-directory-config\">{}</script>",
        client_config(cx, d).to_string().replace("</", "<\\/")
    ));
    out
}
