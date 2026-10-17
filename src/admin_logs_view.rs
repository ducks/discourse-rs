//! The admin logs pages (admin-logs.gjs): the Logs & screening header with
//! its tabs, and each tab's content: the staff action logs, the screened
//! emails, IP addresses and URLs; and the search logs (admin-search-logs).

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{ListContext, escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// The logs pages' tabs.
#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    StaffActions,
    ScreenedEmails,
    ScreenedIps,
    ScreenedUrls,
    SearchLogs,
}

/// DNavItem: `active` on the current route's.
fn nav_item(href: &str, label: &str, active: bool) -> String {
    if active {
        format!(
            "<li class=\"active\"><a class=\"active\" href=\"{}\">{}</a></li>",
            escape(href),
            escape(label)
        )
    } else {
        format!(
            "<li><a href=\"{}\">{}</a></li>",
            escape(href),
            escape(label)
        )
    }
}

/// The header admin-logs.gjs draws over every tab: the email and IP tabs
/// for those who can see them, the error logs for admins.
pub fn header(cx: &AdminCx, can_see_emails: bool, tab: Tab) -> String {
    let base = cx.base_path;
    let i18n = cx.i18n;
    let title = t(i18n, "admin.config.staff_action_logs.title");
    let sub = |k: &str| {
        t(
            i18n,
            &format!("admin.config.staff_action_logs.sub_pages.{k}.title"),
        )
    };
    let mut tabs = nav_item(
        &format!("{base}/admin/logs/staff_action_logs"),
        &title,
        tab == Tab::StaffActions,
    );
    if can_see_emails {
        tabs.push_str(&nav_item(
            &format!("{base}/admin/logs/screened_emails"),
            &sub("screened_emails"),
            tab == Tab::ScreenedEmails,
        ));
    }
    if cx.viewer.can_see_ip {
        tabs.push_str(&nav_item(
            &format!("{base}/admin/logs/screened_ip_addresses"),
            &sub("screened_ips"),
            tab == Tab::ScreenedIps,
        ));
    }
    tabs.push_str(&nav_item(
        &format!("{base}/admin/logs/screened_urls"),
        &sub("screened_urls"),
        tab == Tab::ScreenedUrls,
    ));
    tabs.push_str(&nav_item(
        &format!("{base}/admin/logs/search_logs"),
        &sub("search_logs"),
        tab == Tab::SearchLogs,
    ));
    if cx.viewer.admin {
        tabs.push_str(&format!(
            "<li><a data-auto-route=\"true\" href=\"{base}/logs\">{}</a></li>",
            escape(&t(i18n, "admin.logs.logster.title"))
        ));
    }
    PageHeader {
        crumbs: vec![
            Crumb {
                label: t(i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/logs",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(
            i18n,
            "admin.config.staff_action_logs.header_description",
        ))),
        learn_more_url: None,
        tabs: Some(tabs),
    }
    .render(cx)
}

/// The staff action logs' filters, as the route's `filters` param holds
/// them (JSON), and its date range.
#[derive(Default)]
pub struct StaffFilters {
    pub action_name: Option<String>,
    pub acting_user: Option<String>,
    pub target_user: Option<String>,
    pub subject: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl StaffFilters {
    fn exist(&self) -> bool {
        self.action_name.is_some()
            || self.acting_user.is_some()
            || self.target_user.is_some()
            || self.subject.is_some()
    }
}

/// StaffActionLog's format(): the label, its value on the next line when
/// it has several.
fn format_line(cx: &AdminCx, label: &str, value: &str, escape_value: bool) -> String {
    if value.is_empty() {
        return String::new();
    }
    let br = if value.contains('\n') { "<br/>" } else { " " };
    let shown = value.replace('\n', "<br/>");
    format!(
        "<b>{}:</b>{br} {}",
        t(cx.i18n, label),
        if escape_value { escape(&shown) } else { shown }
    )
}

/// A value as Ruby's to_s gives it to the client: strings, numbers.
fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

const CUSTOM_MODAL_ACTIONS: [&str; 5] = [
    "change_theme",
    "delete_theme",
    "tag_group_create",
    "tag_group_destroy",
    "tag_group_change",
];

/// StaffActionLog#useModalForDetails
fn use_modal(log: &Value) -> bool {
    let details = s(&log["details"]);
    details.chars().count() > 100 || details.contains('\n')
}

/// StaffActionLog#formattedDetails
fn formatted_details(cx: &AdminCx, log: &Value) -> String {
    let id = |k: &str| value_text(&log[k]);
    let post_link = match id("post_id") {
        v if v.is_empty() => String::new(),
        v => format!("<a href data-link-post-id=\"{v}\">{v}</a>"),
    };
    let topic_link = match id("topic_id") {
        v if v.is_empty() => String::new(),
        v => format!("<a href data-link-topic-id=\"{v}\">{v}</a>"),
    };
    let reviewable_link = match id("reviewable_id") {
        v if v.is_empty() => String::new(),
        v => format!("<a href=\"{}/review/{v}\">{v}</a>", cx.base_path),
    };
    let custom = CUSTOM_MODAL_ACTIONS.contains(&s(&log["action_name"]));
    let mut lines = vec![
        format_line(cx, "email", &id("email"), true),
        format_line(cx, "admin.logs.ip_address", &id("ip_address"), true),
        format_line(cx, "admin.logs.topic_id", &topic_link, false),
        format_line(cx, "admin.logs.post_id", &post_link, false),
        format_line(cx, "admin.logs.category_id", &id("category_id"), true),
        format_line(cx, "admin.logs.reviewable_id", &reviewable_link, false),
    ];
    if !custom {
        lines.push(format_line(
            cx,
            "admin.logs.staff_actions.new_value",
            &id("new_value"),
            true,
        ));
        lines.push(format_line(
            cx,
            "admin.logs.staff_actions.previous_value",
            &id("previous_value"),
            true,
        ));
    }
    let details = s(&log["details"]);
    if !use_modal(log) && !details.is_empty() {
        lines.extend(escape(details).split('\n').map(str::to_string));
    }
    let joined = lines
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("<br/>");
    if joined.is_empty() {
        joined
    } else {
        format!("{joined}<br/>")
    }
}

/// A user's tiny avatar.
fn avatar(user: &Value) -> String {
    crate::post_view::avatar_img(
        s(&user["avatar_template"]),
        24,
        &format!(" title=\"{}\"", escape(s(&user["username"]))),
    )
}

/// dAgeWithTooltip
fn age(list: &ListContext, at: &Value) -> String {
    let Some(at) = s(at).parse::<chrono::DateTime<chrono::Utc>>().ok() else {
        return String::new();
    };
    format!(
        "<span class=\"relative-date\"{} data-time=\"{}\" data-format=\"tiny\">{}</span>",
        crate::viewer_dates::long_title(list, at),
        at.timestamp_millis(),
        escape(&crate::topic_list_view::relative_age_tiny(list, at))
    )
}

/// A filter's button: its label and value, the xmark that clears it.
fn filter_button(label: &str, key: &str) -> String {
    format!(
        "<button class=\"btn btn-text btn-default filter\" data-clear-filter=\"{key}\" type=\"button\"><span class=\"d-button-label\">{}</span>{}</button>",
        escape(label),
        icon("xmark", None)
    )
}

/// DDateTimeInputRange without times: the from and to dates.
pub fn date_range(cx: &AdminCx, from: Option<&str>, to: Option<&str>) -> String {
    let input = |class: &str, key: &str, value: Option<&str>, min: Option<&str>| {
        format!(
            "<div class=\"d-date-time-input {class}\"><div class=\"d-date-input\"><input class=\"date-picker\" placeholder=\"{}\" type=\"date\"{}{}><div class=\"picker-container\"></div></div></div>",
            escape(&t(cx.i18n, key)),
            value
                .map(|v| format!(" value=\"{}\"", escape(v)))
                .unwrap_or_default(),
            min.map(|v| format!(" min=\"{}\"", escape(v)))
                .unwrap_or_default(),
        )
    };
    fn day(v: Option<&str>) -> Option<&str> {
        v.and_then(|v| v.get(..10))
    }
    format!(
        "<div class=\"d-date-time-input-range\">{}{}</div>",
        input("from", "dates.from_placeholder", day(from), None),
        input("to", "dates.to_placeholder", day(to), day(from)),
    )
}

/// The staff action logs tab: the filters (or the action menu), the date
/// range, export, and the page of logs.
pub fn staff_action_logs(
    cx: &AdminCx,
    list: &ListContext,
    doc: &Value,
    f: &StaffFilters,
    page: i64,
) -> String {
    let i18n = cx.i18n;
    let action_label = |name: &str| t(i18n, &format!("admin.logs.staff_actions.actions.{name}"));
    let mut out = String::from(
        "<div class=\"admin-container\"><div class=\"staff-action-logs-controls\"><div class=\"staff-action-logs-controls__left\">",
    );
    if f.exist() {
        out.push_str(&format!(
            "<div class=\"staff-action-logs-filters\"><button class=\"btn btn-text clear-filters filter btn-default\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
            escape(&t(i18n, "admin.logs.staff_actions.clear_filters"))
        ));
        if let Some(name) = &f.action_name {
            out.push_str(&filter_button(
                &format!("{}: {}", t(i18n, "admin.logs.action"), action_label(name)),
                "actionFilter",
            ));
        }
        for (key, value, label) in [
            (
                "acting_user",
                &f.acting_user,
                "admin.logs.staff_actions.staff_user",
            ),
            (
                "target_user",
                &f.target_user,
                "admin.logs.staff_actions.target_user",
            ),
            ("subject", &f.subject, "admin.logs.staff_actions.subject"),
        ] {
            if let Some(v) = value {
                out.push_str(&filter_button(&format!("{}: {v}", t(i18n, label)), key));
            }
        }
        out.push_str("</div>");
    } else {
        // The action menu: every staff action, by name.
        let mut actions: Vec<Value> = doc["user_history_actions"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|h| {
                        serde_json::json!({
                            "id": s(&h["id"]),
                            "action_id": h["action_id"],
                            "name": action_label(s(&h["id"])),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        actions.sort_by(|a, b| {
            s(&a["name"])
                .to_lowercase()
                .cmp(&s(&b["name"]).to_lowercase())
        });
        let all = t(i18n, "admin.logs.staff_actions.all");
        let aria = t(i18n, "select_kit.filter_by").replace("%{name}", &all);
        out.push_str(&format!(
            "{} <details id=\"staff-action-logs-action-filter\" class=\"select-kit single-select combobox combo-box\" data-options=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\" data-value=\"\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div id=\"staff-action-logs-action-filter-body\" class=\"select-kit-body\"></div></details>",
            escape(&t(i18n, "admin.logs.staff_actions.filter")),
            escape(&Value::Array(actions).to_string()),
            icon("angle-down", Some("angle-icon")),
            a = escape(&aria),
            n = escape(&all),
        ));
    }
    out.push_str(&format!(
        " <div class=\"date-filter-container\">{}</div></div><div class=\"staff-action-logs-controls__right\"><button class=\"btn btn-icon-text btn-default export-staff-action-logs\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div></div><div class=\"clearfix\"></div>",
        date_range(cx, f.start_date.as_deref(), f.end_date.as_deref()),
        icon("download", None),
        escape(&t(i18n, "admin.export_csv.button_text"))
    ));
    // The details modal's title and close button, for admin-logs.js.
    out.push_str(&format!(
        "<div class=\"table staff-actions\" data-modal-title=\"{}\" data-close-label=\"{}\"><div>",
        escape(&t(i18n, "admin.logs.staff_actions.modal_title")),
        escape(&t(i18n, "close"))
    ));
    let logs = doc["staff_action_logs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if logs.is_empty() {
        out.push_str(&escape(&t(i18n, "search.no_results")));
    } else {
        out.push_str(&format!(
            "<table class=\"table staff-logs grid\"><thead><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></thead><tbody>",
            escape(&t(i18n, "admin.logs.staff_actions.staff_user")),
            escape(&t(i18n, "admin.logs.action")),
            escape(&t(i18n, "admin.logs.staff_actions.subject")),
            escape(&t(i18n, "admin.logs.staff_actions.when")),
            escape(&t(i18n, "admin.logs.staff_actions.details")),
            escape(&t(i18n, "admin.logs.staff_actions.context")),
        ));
        for log in &logs {
            out.push_str(&staff_row(cx, list, log));
        }
        out.push_str("</tbody></table>");
    }
    if doc["load_more_staff_action_logs"].is_string()
        && (logs.len() as i64) < doc["total_rows_staff_action_logs"].as_i64().unwrap_or(0)
    {
        out.push_str(&format!(
            "<div aria-hidden=\"true\" class=\"load-more-sentinel\" data-page=\"{page}\"></div>"
        ));
    }
    out.push_str("</div></div></div>");
    out
}

/// One log's row.
pub fn staff_row(cx: &AdminCx, list: &ListContext, log: &Value) -> String {
    let i18n = cx.i18n;
    let base = cx.base_path;
    let id = log["id"].as_i64().unwrap_or_default();
    let acting = &log["acting_user"];
    let staff = if acting.is_object() {
        format!(
            "<a href=\"{base}/admin/users/{}/{}\">{} {}</a>",
            value_text(&acting["id"]),
            escape(s(&acting["username"])),
            avatar(acting),
            escape(s(&acting["username"]))
        )
    } else {
        format!(
            "<span class=\"deleted-user\" title=\"{}\">{}</span>",
            escape(&t(i18n, "admin.user.deleted")),
            icon("trash-can", None)
        )
    };
    let target = &log["target_user"];
    let mut subject = String::new();
    if target.is_object() {
        subject.push_str(&format!(
            "<a href=\"{base}/admin/users/{}/{}\">{}</a> <a href=\"\" data-filter-target-user=\"{u}\">{u}</a>",
            value_text(&target["id"]),
            escape(s(&target["username"])),
            avatar(target),
            u = escape(s(&target["username"]))
        ));
    }
    let subject_text = s(&log["subject"]);
    if !subject_text.is_empty() {
        if !subject.is_empty() {
            subject.push(' ');
        }
        subject.push_str(&format!(
            "<a href=\"\" title=\"{v}\" data-filter-subject=\"{v}\">{v}</a>",
            v = escape(subject_text)
        ));
    }
    let mut details = formatted_details(cx, log);
    let show = format!(
        "{} {}",
        icon("circle-info", None),
        escape(&t(i18n, "admin.logs.staff_actions.show"))
    );
    if CUSTOM_MODAL_ACTIONS.contains(&s(&log["action_name"])) {
        details.push_str(&format!(
            " <a href=\"\" class=\"staff-log-custom-details\">{show}</a>"
        ));
    }
    if use_modal(log) {
        details.push_str(&format!(
            " <a href=\"\" class=\"staff-log-details\" data-details=\"{}\">{show}</a>",
            escape(s(&log["details"]))
        ));
    }
    // showHtmlSafeContext
    let context = if s(&log["action_name"]) == "upcoming_change_toggled" {
        s(&log["context"]).to_string()
    } else {
        escape(s(&log["context"]))
    };
    format!(
        "<tr class=\"admin-list-item\" data-user-history-id=\"{id}\"><td class=\"staff-users\"><div class=\"staff-user\">{staff}</div></td><td class=\"col value action\"><a href=\"\" data-filter-action=\"{}\" data-action-id=\"{}\" data-custom-type=\"{}\">{}</a></td><td class=\"col value subject\"><div class=\"subject\">{subject}</div></td><td class=\"col value created-at\">{}</td><td class=\"col value details\"><div>{details}</div></td><td class=\"col value context\">{context}</td></tr>",
        escape(s(&log["action_name"])),
        value_text(&log["action"]),
        escape(&value_text(&log["custom_type"])),
        escape(&t(
            i18n,
            &format!(
                "admin.logs.staff_actions.actions.{}",
                s(&log["action_name"])
            )
        )),
        age(list, &log["created_at"]),
    )
}

/// DPageSubheader with only its description.
fn described(cx: &AdminCx, key: &str) -> String {
    format!(
        "<div class=\"d-page-subheader\"><div class=\"d-page-subheader__title-row\"></div><p class=\"d-page-subheader__description\">{}</p></div>",
        escape(&t(
            cx.i18n,
            &format!("admin.config.staff_action_logs.sub_pages.{key}.header_description")
        ))
    )
}

/// The CSV export button.
fn export_button(cx: &AdminCx, class: &str, title_key: &str) -> String {
    format!(
        "<button class=\"btn btn-icon-text btn-default{}\" title=\"{}\" type=\"button\" data-export=\"{title_key}\">{}<span class=\"d-button-label\">{}</span></button>",
        if class.is_empty() {
            String::new()
        } else {
            format!(" {class}")
        },
        escape(&t(
            cx.i18n,
            &format!("admin.export_csv.button_title.{title_key}")
        )),
        icon("download", None),
        escape(&t(cx.i18n, "admin.export_csv.button_text"))
    )
}

fn labelled(cx: &AdminCx, key: &str, value: &str) -> String {
    format!(
        "<div class=\"label\">{}</div>{value}",
        escape(&t(cx.i18n, key))
    )
}

/// The screened emails tab.
pub fn screened_emails(cx: &AdminCx, list: &ListContext, rows: &[Value]) -> String {
    let i18n = cx.i18n;
    let mut out = String::from("<div class=\"admin-container\">");
    out.push_str(&described(cx, "screened_emails"));
    if cx.viewer.can_see_ip {
        out.push_str(&export_button(
            cx,
            "screened-email-export",
            "screened_email",
        ));
    }
    out.push_str("<br><div class=\"loading-container\">");
    if rows.is_empty() {
        out.push_str(&escape(&t(i18n, "search.no_results")));
    } else {
        let th = |class: &str, key: &str| {
            format!("<th class=\"{class}\">{}</th>", escape(&t(i18n, key)))
        };
        out.push_str("<table class=\"screened-emails grid\"><thead>");
        out.push_str(&th("first email", "admin.logs.screened_emails.email"));
        out.push_str(&th("action", "admin.logs.action"));
        out.push_str(&th("match_count", "admin.logs.match_count"));
        out.push_str(&th("last_match_at", "admin.logs.last_match_at"));
        out.push_str(&th("created_at", "admin.logs.created_at"));
        if cx.viewer.can_see_ip {
            out.push_str(&th("ip_address", "admin.logs.ip_address"));
        }
        out.push_str("<th class=\"action\"></th></thead><tbody>");
        for r in rows {
            out.push_str(&format!(
                "<tr class=\"admin-list-item\" data-id=\"{}\"><td class=\"col first email\"><div class=\"overflow-ellipsis\" title=\"{e}\">{e}</div></td><td class=\"action\">{}</td><td class=\"match_count\">{}</td><td class=\"last_match_at\">{}</td><td class=\"created_at\">{}</td>",
                value_text(&r["id"]),
                escape(&t(
                    i18n,
                    &format!("admin.logs.screened_actions.{}", s(&r["action"]))
                )),
                labelled(cx, "admin.logs.match_count", &value_text(&r["match_count"])),
                labelled(cx, "admin.logs.last_match_at", &age(list, &r["last_match_at"])),
                labelled(cx, "admin.logs.created_at", &age(list, &r["created_at"])),
                e = escape(s(&r["email"])),
            ));
            if cx.viewer.can_see_ip {
                out.push_str(&format!(
                    "<td class=\"ip_address\">{}</td>",
                    escape(s(&r["ip_address"]))
                ));
            }
            out.push_str(&format!(
                "<td class=\"action\"><button class=\"btn btn-icon-text btn-default screened-email-allow\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></td></tr>",
                icon("check", None),
                escape(&t(i18n, "admin.logs.screened_emails.actions.allow"))
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div></div>");
    out
}

/// The screened URLs tab.
pub fn screened_urls(cx: &AdminCx, list: &ListContext, rows: &[Value]) -> String {
    let i18n = cx.i18n;
    let mut out = String::from("<div class=\"admin-container\">");
    out.push_str(&described(cx, "screened_urls"));
    out.push_str(&export_button(cx, "", "screened_url"));
    out.push_str("<br><div class=\"loading-container\">");
    if rows.is_empty() {
        out.push_str(&escape(&t(i18n, "search.no_results")));
    } else {
        let th = |class: &str, key: &str| {
            format!("<th class=\"{class}\">{}</th>", escape(&t(i18n, key)))
        };
        out.push_str("<table class=\"screened-urls grid\"><thead>");
        out.push_str(&th("first domain", "admin.logs.screened_urls.domain"));
        out.push_str(&th("action", "admin.logs.action"));
        out.push_str(&th("match_count", "admin.logs.match_count"));
        out.push_str(&th("last_match_at", "admin.logs.last_match_at"));
        out.push_str(&th("created_at", "admin.logs.created_at"));
        out.push_str("</thead><tbody>");
        for r in rows {
            out.push_str(&format!(
                "<tr class=\"admin-list-item\"><td class=\"col first domain\"><div class=\"overflow-ellipsis\" title=\"{d}\">{d}</div></td><td class=\"col action\">{}</td><td class=\"col match_count\">{}</td><td class=\"col last_match_at\">{}</td><td class=\"col created_at\">{}</td></tr>",
                escape(&t(
                    i18n,
                    &format!("admin.logs.screened_actions.{}", s(&r["action"]))
                )),
                labelled(cx, "admin.logs.match_count", &value_text(&r["match_count"])),
                labelled(cx, "admin.logs.last_match_at", &age(list, &r["last_match_at"])),
                labelled(cx, "admin.logs.created_at", &age(list, &r["created_at"])),
                d = escape(s(&r["domain"])),
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div></div>");
    out
}

/// The screened IP addresses tab: the filter, the new address form and
/// the list with its edit, delete, allow and block controls.
pub fn screened_ips(cx: &AdminCx, list: &ListContext, rows: &[Value], filter: &str) -> String {
    let i18n = cx.i18n;
    let mut out = String::from("<div class=\"admin-container\">");
    out.push_str(&described(cx, "screened_ips"));
    out.push_str(&format!(
        "<div class=\"screened-ip-controls\"><div class=\"filter-screened-ip-address inline-form\"><input placeholder=\"{}\" autocapitalize=\"off\" autocorrect=\"off\" class=\"ip-address-input\" value=\"{}\" type=\"text\"> {}</div>",
        escape(&t(i18n, "admin.logs.screened_ips.form.filter")),
        escape(filter),
        export_button(cx, "", "screened_ip"),
    ));
    // ScreenedIpAddressForm's actions: allow_admin with the allowlist.
    let mut actions = vec!["block", "do_nothing"];
    if cx
        .settings
        .get("use_admin_ip_allowlist")
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        actions.push("allow_admin");
    }
    let options: Vec<Value> = actions
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a,
                "name": t(i18n, &format!("admin.logs.screened_ips.actions.{a}")),
            })
        })
        .collect();
    let block = t(i18n, "admin.logs.screened_ips.actions.block");
    let aria = t(i18n, "select_kit.filter_by").replace("%{name}", &block);
    out.push_str(&format!(
        "<form class=\"screened-ip-address-form inline-form\"><label>{}</label> <input placeholder=\"{}\" autocapitalize=\"off\" autocorrect=\"off\" class=\"ip-address-input\" type=\"text\"> <details class=\"select-kit single-select combobox combo-box has-selection\" data-options=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\" data-value=\"block\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" data-value=\"block\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details> <button class=\"btn btn-text btn-default\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></form></div>",
        escape(&t(i18n, "admin.logs.screened_ips.form.label")),
        escape(&t(i18n, "admin.logs.screened_ips.form.ip_address")),
        escape(&Value::Array(options).to_string()),
        icon("angle-down", Some("angle-icon")),
        escape(&t(i18n, "admin.logs.screened_ips.form.add")),
        a = escape(&aria),
        n = escape(&block),
    ));
    // The strings admin-logs.js confirms and reports with.
    let strings: serde_json::Map<String, Value> = [
        "admin.logs.screened_ips.delete_confirm",
        "admin.logs.save",
        "cancel",
        "yes_value",
        "no_value",
        "generic_error",
        "generic_error_with_reason",
    ]
    .iter()
    .map(|k| (k.to_string(), Value::String(t(i18n, k))))
    .collect();
    let dialog = serde_json::json!({
        "ok_value": t(i18n, "ok_value"),
        "cancel": t(i18n, "cancel"),
        "modal.close": t(i18n, "modal.close"),
    });
    out.push_str(&format!(
        "<script type=\"application/json\" id=\"admin-logs-strings\">{}</script><script type=\"application/json\" id=\"dialog-strings\">{}</script>",
        Value::Object(strings).to_string().replace('<', "\\u003c"),
        dialog.to_string().replace('<', "\\u003c")
    ));
    out.push_str("<div class=\"loading-container\">");
    if rows.is_empty() {
        out.push_str(&escape(&t(i18n, "search.no_results")));
    } else {
        let th = |class: &str, key: &str| {
            format!(
                "<th class=\"col heading {class}\">{}</th>",
                escape(&t(i18n, key))
            )
        };
        out.push_str("<table class=\"admin-logs-table screened-ip-addresses grid\"><thead class=\"heading-container\">");
        out.push_str(&th("first ip_address", "admin.logs.ip_address"));
        out.push_str(&th("action", "admin.logs.action"));
        out.push_str(&th("match_count", "admin.logs.match_count"));
        out.push_str(&th("created_at", "admin.logs.created_at"));
        out.push_str(&th("last_match_at", "admin.logs.last_match_at"));
        out.push_str("<th class=\"col heading actions\"></th></thead><tbody>");
        for r in rows {
            out.push_str(&ip_row(cx, list, r));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div></div>");
    out
}

/// One screened IP address's row.
fn ip_row(cx: &AdminCx, list: &ListContext, r: &Value) -> String {
    let i18n = cx.i18n;
    let ip = s(&r["ip_address"]);
    let blocked = s(&r["action_name"]) == "block";
    let small = |class: &str, name: &str| {
        format!(
            "<button class=\"btn no-text btn-icon btn-default{class}\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            icon(name, None)
        )
    };
    let toggle = if blocked {
        (
            "allow",
            "check",
            "admin.logs.screened_ips.actions.do_nothing",
        )
    } else {
        ("block", "ban", "admin.logs.screened_ips.actions.block")
    };
    format!(
        "<tr class=\"admin-list-item\" data-id=\"{}\" data-ip-address=\"{ip_e}\" data-action-name=\"{}\"><td class=\"col first ip_address\"><a class=\"inline-editable-field\" href=\"\">{}</a></td><td class=\"col action\">{} {}</td><td class=\"col match_count\">{}</td><td class=\"col created_at\">{}</td><td class=\"col last_match_at\">{}</td><td class=\"col actions\">{} {} <button class=\"btn btn-icon-text btn-default screened-ip-{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></td></tr>",
        value_text(&r["id"]),
        escape(s(&r["action_name"])),
        if ip.find('/').is_some_and(|i| i > 0) {
            format!("<strong>{}</strong>", escape(ip))
        } else {
            escape(ip)
        },
        icon(if blocked { "ban" } else { "check" }, None),
        escape(&t(
            i18n,
            &format!("admin.logs.screened_ips.actions.{}", s(&r["action_name"]))
        )),
        labelled(cx, "admin.logs.match_count", &value_text(&r["match_count"])),
        labelled(cx, "admin.logs.created_at", &age(list, &r["created_at"])),
        if r["last_match_at"].is_null() {
            String::new()
        } else {
            labelled(
                cx,
                "admin.logs.last_match_at",
                &age(list, &r["last_match_at"]),
            )
        },
        small(" btn-danger screened-ip-delete", "trash-can"),
        small(" screened-ip-edit", "pencil"),
        toggle.0,
        icon(toggle.1, None),
        escape(&t(i18n, toggle.2)),
        ip_e = escape(ip),
    )
}

/// The search logs tab: the period and search type menus over the
/// trending terms.
pub fn search_logs(
    cx: &AdminCx,
    list: &ListContext,
    rows: &[Value],
    period: &str,
    search_type: &str,
) -> String {
    let i18n = cx.i18n;
    let mut types = vec![
        ("all", "all_search_types"),
        ("non_staff_only", "non_staff_only"),
    ];
    if cx
        .settings
        .get("improved_crawler_detection")
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        types.push(("human_only", "human_only"));
    }
    types.push(("header", "header"));
    types.push(("full_page", "full_page"));
    let options: Vec<Value> = types
        .iter()
        .map(|(id, key)| {
            serde_json::json!({
                "id": id,
                "name": t(i18n, &format!("admin.logs.search_logs.types.{key}")),
            })
        })
        .collect();
    let selected = options
        .iter()
        .find(|o| o["id"] == search_type)
        .map(|o| s(&o["name"]).to_string())
        .unwrap_or_default();
    let aria = t(i18n, "select_kit.filter_by").replace("%{name}", &selected);
    let mut out = format!(
        "<div class=\"admin-container\"><div class=\"admin-title\">{} <details class=\"select-kit single-select combobox combo-box has-selection search-logs-filter\" data-options=\"{}\"><summary aria-label=\"{a}\" name=\"{a}\" data-name=\"{n}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" data-value=\"{}\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details></div><div class=\"loading-container\">",
        crate::directory_view::period_chooser(list, period),
        escape(&Value::Array(options.clone()).to_string()),
        escape(search_type),
        escape(search_type),
        icon("angle-down", Some("angle-icon")),
        a = escape(&aria),
        n = escape(&selected),
    );
    if rows.is_empty() {
        out.push_str(&escape(&t(i18n, "search.no_results")));
    } else {
        let label = |key: &str| {
            format!(
                "<div class=\"label\">{}</div>",
                escape(&t(i18n, &format!("admin.logs.search_logs.{key}")))
            )
        };
        out.push_str(&format!(
            "<table class=\"search-logs-list grid\"><thead><th class=\"col heading term\">{}</th><th class=\"col heading\">{}</th><th class=\"col heading\">{}</th></thead><tbody>",
            escape(&t(i18n, "admin.logs.search_logs.term")),
            escape(&t(i18n, "admin.logs.search_logs.searches")),
            escape(&t(i18n, "admin.logs.search_logs.click_through_rate")),
        ));
        for r in rows {
            let term = s(&r["term"]);
            let mut query = format!("term={}", urlencoding(term));
            if period != "yearly" {
                query = format!("period={}&{query}", urlencoding(period));
            }
            out.push_str(&format!(
                "<tr class=\"admin-list-item\"><td class=\"col term\"><a class=\"test\" href=\"{}/admin/logs/search_logs/term?{}\">{}</a></td><td class=\"col\">{}{}</td><td class=\"col\">{}{}%</td></tr>",
                cx.base_path,
                escape(&query),
                escape(term),
                label("searches"),
                value_text(&r["searches"]),
                label("click_through_rate"),
                value_text(&r["ctr"]),
            ));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("</div></div>");
    out
}

/// A query parameter's value, encoded as the router's URLs have it.
fn urlencoding(v: &str) -> String {
    form_urlencoded::byte_serialize(v.as_bytes()).collect()
}

/// An email logs column header: its key and colspan.
type Header = (&'static str, Option<u8>);
/// An email logs filter: its param and placeholder key.
type FilterInput = (&'static str, &'static str);

/// The email logs' tabs.
#[derive(Clone, Copy, PartialEq)]
pub enum EmailTab {
    Sent,
    Skipped,
    Bounced,
    Received,
    Rejected,
}

impl EmailTab {
    pub fn key(self) -> &'static str {
        match self {
            EmailTab::Sent => "sent",
            EmailTab::Skipped => "skipped",
            EmailTab::Bounced => "bounced",
            EmailTab::Received => "received",
            EmailTab::Rejected => "rejected",
        }
    }

    /// The tab's headers (key, colspan) and filters (param, placeholder).
    fn columns(self) -> (Vec<Header>, Vec<FilterInput>) {
        let user = ("user", "admin.email.logs.filters.user_placeholder");
        let address = ("address", "admin.email.logs.filters.address_placeholder");
        let kind = ("type", "admin.email.logs.filters.type_placeholder");
        let from = (
            "from",
            "admin.email.incoming_emails.filters.from_placeholder",
        );
        let to = ("to", "admin.email.incoming_emails.filters.to_placeholder");
        let subject = (
            "subject",
            "admin.email.incoming_emails.filters.subject_placeholder",
        );
        match self {
            EmailTab::Sent => (
                vec![
                    ("admin.email.user", None),
                    ("admin.email.to_address", None),
                    ("admin.email.email_type", None),
                    ("admin.email.reply_key", None),
                    ("admin.email.post_link_with_smtp", None),
                ],
                vec![
                    user,
                    address,
                    kind,
                    (
                        "reply_key",
                        "admin.email.logs.filters.reply_key_placeholder",
                    ),
                ],
            ),
            EmailTab::Skipped => (
                vec![
                    ("admin.email.user", None),
                    ("admin.email.to_address", None),
                    ("admin.email.email_type", None),
                    ("admin.email.skipped_reason", None),
                ],
                vec![user, address, kind],
            ),
            EmailTab::Bounced => (
                vec![
                    ("admin.email.user", None),
                    ("admin.email.to_address", None),
                    ("admin.email.bounced", Some(2)),
                ],
                vec![user, address, kind],
            ),
            EmailTab::Received => (
                vec![
                    ("admin.email.incoming_emails.from_address", None),
                    ("admin.email.incoming_emails.to_addresses", None),
                    ("admin.email.incoming_emails.subject", None),
                ],
                vec![from, to, subject],
            ),
            EmailTab::Rejected => (
                vec![
                    ("admin.email.incoming_emails.from_address", None),
                    ("admin.email.incoming_emails.to_addresses", None),
                    ("admin.email.incoming_emails.subject", None),
                    ("admin.email.incoming_emails.error", Some(2)),
                ],
                vec![
                    from,
                    to,
                    subject,
                    (
                        "error",
                        "admin.email.incoming_emails.filters.error_placeholder",
                    ),
                ],
            ),
        }
    }
}

/// admin-email-logs.gjs's header with the five tabs.
pub fn email_header(cx: &AdminCx, tab: EmailTab) -> String {
    let base = cx.base_path;
    let i18n = cx.i18n;
    let title = t(i18n, "admin.config.email_logs.title");
    let mut tabs = String::new();
    for each in [
        EmailTab::Sent,
        EmailTab::Skipped,
        EmailTab::Bounced,
        EmailTab::Received,
        EmailTab::Rejected,
    ] {
        let href = match each {
            EmailTab::Sent => format!("{base}/admin/email-logs"),
            other => format!("{base}/admin/email-logs/{}", other.key()),
        };
        tabs.push_str(&nav_item(
            &href,
            &t(
                i18n,
                &format!("admin.config.email_logs.sub_pages.{}.title", each.key()),
            ),
            each == tab,
        ));
    }
    PageHeader {
        crumbs: vec![
            Crumb {
                label: t(i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/email-logs",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(
            i18n,
            "admin.config.email_logs.header_description",
        ))),
        learn_more_url: None,
        tabs: Some(tabs),
    }
    .render(cx)
}

/// dFormatDate: the tiny date with its full date as the title.
fn format_date(list: &ListContext, at: &Value) -> String {
    age(list, at)
}

fn email_user(cx: &AdminCx, log: &Value) -> String {
    let user = &log["user"];
    if user.is_object() {
        format!(
            "<span class=\"email-logs-user\"><a href=\"{}/admin/users/{}/{}\">{} {}</a></span>",
            cx.base_path,
            value_text(&user["id"]),
            escape(s(&user["username"])),
            avatar(user),
            escape(s(&user["username"]))
        )
    } else {
        "&mdash;".into()
    }
}

/// One log's row, as the tab's template draws it.
fn email_row(cx: &AdminCx, list: &ListContext, tab: EmailTab, log: &Value) -> String {
    let i18n = cx.i18n;
    let id = value_text(&log["id"]);
    let date = format_date(list, &log["created_at"]);
    let to = escape(s(&log["to_address"]));
    match tab {
        EmailTab::Sent => {
            let mut address = String::new();
            if log["bounced"] == Value::Bool(true) {
                address.push_str(&format!(
                    "<span title=\"{}\">{}</span> ",
                    escape(&t(i18n, "admin.email.bounced")),
                    icon("arrow-rotate-right", None)
                ));
            }
            address.push_str(&format!("<a href=\"mailto:{to}\" title=\"TO\">{to}</a>"));
            let ccs: Vec<&str> = log["cc_addresses"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            // CC_ADDRESS_DISPLAY_THRESHOLD: two, the rest behind See more.
            for cc in ccs.iter().take(2) {
                address.push_str(&format!(
                    " <a href=\"mailto:{c}\" title=\"CC\">{c}</a>",
                    c = escape(cc)
                ));
            }
            if ccs.len() > 2 {
                let more: String = ccs[2..]
                    .iter()
                    .map(|cc| {
                        format!(
                            "<li><span><a href=\"mailto:{c}\" title=\"CC\">{c}</a></span></li>",
                            c = escape(cc)
                        )
                    })
                    .collect();
                address.push_str(&format!(
                    " <button class=\"fk-d-tooltip__trigger email-log-cc-more\" type=\"button\"><span class=\"fk-d-tooltip__trigger-container\">{}</span></button><template class=\"email-log-cc-addresses\"><ul>{more}</ul></template>",
                    escape(&t(i18n, "admin.email.logs.email_addresses.see_more"))
                ));
            }
            let mut post = String::new();
            if let Some(url) = log["post_url"].as_str() {
                post.push_str(&format!(
                    "<a href=\"{}\">{}</a><div>{}</div>",
                    escape(url),
                    escape(s(&log["post_description"])),
                    escape(
                        &t(i18n, "admin.email.logs.post_id")
                            .replace("%{post_id}", &value_text(&log["post_id"]))
                    )
                ));
            }
            let smtp = escape(s(&log["smtp_transaction_response"]));
            format!(
                "<tr class=\"sent-email-item\" data-test-email-log-row-id=\"{id}\"><td class=\"sent-email-date\">{date}</td><td class=\"sent-email-username\">{}</td><td class=\"sent-email-address\">{address}</td><td class=\"sent-email-type\">{}</td><td class=\"sent-email-reply-key\"><span class=\"reply-key\" title=\"{k}\">{k}</span></td><td class=\"sent-email-post-link-with-smtp-response\">{post}<code title=\"{smtp}\">{smtp}</code></td></tr>",
                email_user(cx, log),
                escape(s(&log["email_type"])),
                k = escape(s(&log["reply_key"])),
            )
        }
        EmailTab::Skipped => format!(
            "<tr data-test-email-log-row-id=\"{id}\"><td>{date}</td><td>{}</td><td><a href=\"mailto:{to}\">{to}</a></td><td>{}</td><td><div class=\"overflow-ellipsis\" title=\"{r}\">{r}</div></td></tr>",
            email_user(cx, log),
            escape(s(&log["email_type"])),
            r = escape(s(&log["skipped_reason"])),
        ),
        EmailTab::Bounced => format!(
            "<tr data-test-email-log-row-id=\"{id}\"><td>{date}</td><td>{}</td><td><a href=\"mailto:{to}\">{to}</a></td><td>{}</td><td class=\"email-details\">{}</td></tr>",
            email_user(cx, log),
            escape(s(&log["email_type"])),
            if log["has_bounce_key"] == Value::Bool(true) {
                format!(
                    "<a href=\"\" class=\"incoming-email-link\" title=\"{}\">{}</a>",
                    escape(&t(i18n, "admin.email.details_title")),
                    icon("circle-info", None)
                )
            } else {
                String::new()
            },
        ),
        EmailTab::Received | EmailTab::Rejected => {
            let tos = log["to_addresses"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let subject = escape(s(&log["subject"]));
            let subject_cell = match (tab, log["post_url"].as_str()) {
                (EmailTab::Received, Some(url)) => {
                    format!("<a href=\"{}\">{subject}</a>", escape(url))
                }
                _ => subject,
            };
            let mut row = format!(
                "<tr data-test-email-log-row-id=\"{id}\"><td>{date}</td><td>{}</td><td>{}</td><td>{subject_cell}</td>",
                escape(s(&log["from_address"])),
                escape(&tos),
            );
            if tab == EmailTab::Rejected {
                row.push_str(&format!(
                    "<td><a class=\"incoming-email-link\" href=\"\">{}</a></td>",
                    escape(s(&log["error"]))
                ));
            }
            row.push_str("</tr>");
            row
        }
    }
}

/// EmailLogsList: the headers, a filter input each, the logs or none,
/// and the load-more sentinel.
pub fn email_logs(
    cx: &AdminCx,
    list: &ListContext,
    tab: EmailTab,
    logs: &[Value],
    filters: &[(String, String)],
) -> String {
    let i18n = cx.i18n;
    let (headers, filter_columns) = tab.columns();
    let mut out = String::from(
        "<div class=\"admin-container\"><div><div class=\"horizontal-scroll-sync__container\" data-loads-rows><div class=\"horizontal-scroll-sync__top-scroll\"><div class=\"horizontal-scroll-sync__fake-content\"></div></div><div class=\"horizontal-scroll-sync__content\"><table class=\"table email-list\"><thead><tr>",
    );
    out.push_str(&format!(
        "<th>{}</th>",
        escape(&t(i18n, "admin.email.sent_at"))
    ));
    for (key, colspan) in &headers {
        out.push_str(&format!(
            "<th{}>{}</th>",
            colspan
                .map(|c| format!(" colspan=\"{c}\""))
                .unwrap_or_default(),
            escape(&t(i18n, key))
        ));
    }
    out.push_str(&format!(
        "</tr></thead><tbody><tr class=\"filters\"><td><span class=\"sr-only\">{}</span></td>",
        escape(&t(i18n, "admin.email.logs.filters.title"))
    ));
    for (name, placeholder) in &filter_columns {
        let value = filters
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        out.push_str(&format!(
            "<td><input placeholder=\"{}\" data-filter=\"{name}\" value=\"{}\" type=\"text\"></td>",
            escape(&t(i18n, placeholder)),
            escape(value)
        ));
    }
    // The sent tab's empty alignment cell.
    if tab == EmailTab::Sent {
        out.push_str("<td></td>");
    }
    out.push_str("</tr>");
    if logs.is_empty() {
        out.push_str(&format!(
            "<tr><td colspan=\"6\">{}</td></tr>",
            escape(&t(i18n, "admin.email.logs.none"))
        ));
    }
    for log in logs {
        out.push_str(&email_row(cx, list, tab, log));
    }
    out.push_str("</tbody></table></div></div>");
    // A full page of 50 may have more.
    if logs.len() >= 50 {
        out.push_str(
            "<div aria-hidden=\"true\" class=\"load-more-sentinel\" data-offset=\"0\"></div>",
        );
    } else {
        out.push_str("<div aria-hidden=\"true\" class=\"load-more-sentinel\"></div>");
    }
    out.push_str("</div><div class=\"loading-container\"></div></div>");
    out
}
