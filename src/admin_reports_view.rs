//! The admin reports index (admin-reports.gjs with AdminReports): the
//! reports from /admin/reports.json less dashboard_hidden_reports, in
//! their groups (REPORT_GROUPS, then the other core reports, then each
//! plugin's), under DFilterControls' text filter and group menu.

use serde_json::Value;

use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

/// AdminReports' REPORT_GROUPS.
const REPORT_GROUPS: [(&str, &[&str]); 6] = [
    (
        "engagement",
        &[
            "daily_engaged_users",
            "dau_by_mau",
            "mobile_visits",
            "new_contributors",
            "signups",
            "topic_view_stats",
            "visits",
        ],
    ),
    (
        "traffic",
        &[
            "consolidated_page_views",
            "consolidated_page_views_browser_detection",
            "page_view_anon_browser_reqs",
            "page_view_anon_reqs",
            "page_view_crawler_reqs",
            "page_view_legacy_total_reqs",
            "page_view_logged_in_reqs",
            "page_view_logged_in_browser_reqs",
            "page_view_total_reqs",
            "site_traffic",
            "top_traffic_sources",
        ],
    ),
    (
        "members",
        &[
            "top_ignored_users",
            "top_referrers",
            "top_users_by_likes_received",
            "trust_level_growth",
            "users_by_trust_level",
            "users_by_type",
        ],
    ),
    (
        "content",
        &[
            "posts",
            "time_to_first_response",
            "top_referred_topics",
            "top_uploads",
            "topics",
            "topics_with_no_response",
            "trending_search",
            "user_to_user_private_messages_with_replies",
        ],
    ),
    (
        "moderation_and_security",
        &[
            "admin_logins",
            "associated_accounts_by_provider",
            "consolidated_api_requests",
            "emails",
            "flags",
            "flags_status",
            "moderators_activity",
            "suspicious_logins",
            "user_flagging_ratio",
            "web_crawlers",
            "web_hook_events_daily_aggregate",
        ],
    ),
    ("other", &[]),
];

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// A group of reports: its key, name and the reports' indexes.
struct Group<'a> {
    key: String,
    name: String,
    reports: Vec<&'a Value>,
}

/// AdminReports#groupReports
fn group_reports<'a>(i18n: &crate::i18n::I18n, reports: &[&'a Value]) -> Vec<Group<'a>> {
    let mut groups: Vec<Group> = Vec::new();
    let core: Vec<&Value> = reports
        .iter()
        .copied()
        .filter(|r| r["plugin"].is_null())
        .collect();
    let mut assigned: Vec<&str> = Vec::new();
    for (key, types) in REPORT_GROUPS {
        let members: Vec<&Value> = core
            .iter()
            .copied()
            .filter(|r| types.contains(&s(&r["type"])))
            .collect();
        if !members.is_empty() {
            assigned.extend(members.iter().map(|r| s(&r["type"])));
            groups.push(Group {
                key: key.to_string(),
                name: t(i18n, &format!("admin.reports.group_{key}")),
                reports: members,
            });
        }
    }
    let unassigned: Vec<&Value> = core
        .iter()
        .copied()
        .filter(|r| !assigned.contains(&s(&r["type"])))
        .collect();
    if !unassigned.is_empty() {
        if let Some(other) = groups.iter_mut().find(|g| g.key == "other") {
            other.reports.extend(unassigned);
        } else {
            groups.push(Group {
                key: "other".into(),
                name: t(i18n, "admin.reports.group_other"),
                reports: unassigned,
            });
        }
    }
    let mut plugins: Vec<Group> = Vec::new();
    for report in reports.iter().copied().filter(|r| !r["plugin"].is_null()) {
        let plugin = s(&report["plugin"]);
        let key = format!("plugin-{plugin}");
        match plugins.iter_mut().find(|g| g.key == key) {
            Some(group) => group.reports.push(report),
            None => plugins.push(Group {
                name: report["plugin_display_name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(plugin)
                    .to_string(),
                key,
                reports: vec![report],
            }),
        }
    }
    // localeCompare
    plugins.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    groups.extend(plugins);
    groups
}

/// The page's filters: DFilterControls' `filter` and `group` params.
pub struct Filters<'a> {
    pub filter: &'a str,
    pub group: &'a str,
}

/// The page under the admin reports header.
pub fn index(cx: &AdminCx, reports: &[Value], hidden: &[&str], f: &Filters) -> String {
    let i18n = cx.i18n;
    let title = t(i18n, "admin.config.reports.title");
    let mut out = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/reports",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(i18n, "admin.config.reports.header_description"))),
        learn_more_url: Some("https://meta.discourse.org/t/-/240233".into()),
        tabs: Some(String::new()),
    }
    .render(cx);
    // filterReports
    let visible: Vec<&Value> = reports
        .iter()
        .filter(|r| !hidden.contains(&s(&r["type"])))
        .collect();
    let all_groups = group_reports(i18n, &visible);
    // The URL's group, when it is one of the options.
    let group = if all_groups.iter().any(|g| g.key == f.group) {
        f.group
    } else {
        "all"
    };
    let active = !f.filter.is_empty() || group != "all";
    out.push_str("<div class=\"admin-container admin-config-page__main-area\"><div class=\"admin-config-area__full-width\">");
    let all_label = t(i18n, "admin.reports.all_groups");
    out.push_str(&format!(
        "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" value=\"{}\" placeholder=\"{}\" type=\"text\"></div></div><div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown{} d-native-select\">",
        icon("magnifying-glass", Some("-left")),
        escape(f.filter),
        escape(&t(i18n, "admin.filter_reports")),
        escape(&all_label),
        if group == "all" { "" } else { " --active" }
    ));
    let options = std::iter::once(("all".to_string(), all_label.clone()))
        .chain(all_groups.iter().map(|g| (g.key.clone(), g.name.clone())));
    for (value, label) in options {
        let selected = value == group;
        out.push_str(&format!(
            "<option class=\"d-native-select__option{}\" value=\"{}\"{}>{}</option>",
            if selected { " --selected" } else { "" },
            escape(&value),
            if selected { " selected" } else { "" },
            escape(&label)
        ));
    }
    out.push_str("</select></div>");
    out.push_str(&reset_button(cx, !active, false));
    out.push_str("</div>");
    // Every group is drawn; admin-reports.js hides what the filters leave
    // out, as DFilterControls#filteredData does, and shows the no-results
    // state from its template.
    out.push_str(&format!(
        "<template class=\"admin-reports-no-results\"><div class=\"d-filter-controls__no-results\"><p>{}</p>{}</div></template>",
        escape(&t(i18n, "admin.filter_reports_no_results")),
        reset_button(cx, false, true)
    ));
    for g in &all_groups {
        out.push_str(&format!(
            "<section class=\"admin-reports-group\" data-group=\"{}\"><h2 class=\"admin-reports-group__title\">{}</h2><div class=\"admin-section-landing-wrapper admin-reports-list\">",
            escape(&g.key),
            escape(&g.name)
        ));
        for r in &g.reports {
            out.push_str(&format!(
                "<a class=\"admin-section-landing-item\" href=\"{}/admin/reports/{}\"><div class=\"admin-section-landing-item__content\"><h3 class=\"admin-section-landing-item__title\">{}</h3><p class=\"admin-section-landing-item__description\">{}</p></div></a>",
                cx.base_path,
                escape(s(&r["type"])),
                escape(s(&r["title"])),
                escape(s(&r["description"]))
            ));
        }
        out.push_str("</div></section>");
    }
    out.push_str("</div></div>");
    out
}

/// DFilterControls' ResetButton, with its label in the no-results state.
fn reset_button(cx: &AdminCx, disabled: bool, label: bool) -> String {
    let reset = escape(&t(cx.i18n, "filter_controls.reset"));
    let dis = if disabled { " disabled=\"\"" } else { "" };
    if label {
        format!(
            "<button aria-label=\"{reset}\" class=\"btn btn-icon-text btn-default d-filter-controls__reset\"{dis} title=\"{reset}\" type=\"button\">{}<span class=\"d-button-label\">{reset}</span></button>",
            icon("arrow-rotate-left", None)
        )
    } else {
        format!(
            "<button aria-label=\"{reset}\" class=\"btn no-text btn-icon btn-default d-filter-controls__reset\"{dis} title=\"{reset}\" type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
            icon("arrow-rotate-left", None)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cx_parts() -> crate::i18n::I18n {
        crate::i18n::I18n::vendored().unwrap()
    }

    #[test]
    fn reports_go_to_their_groups_then_other_then_plugins() {
        let i18n = cx_parts();
        let reports = [
            json!({"type": "signups"}),
            json!({"type": "something_new"}),
            json!({"type": "solved", "plugin": "discourse-solved", "plugin_display_name": "Solved"}),
            json!({"type": "assigns", "plugin": "discourse-assign", "plugin_display_name": "Assign"}),
            json!({"type": "visits"}),
        ];
        let refs: Vec<&Value> = reports.iter().collect();
        let keys: Vec<(String, usize)> = group_reports(&i18n, &refs)
            .into_iter()
            .map(|g| (g.key, g.reports.len()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("engagement".to_string(), 2),
                ("other".to_string(), 1),
                ("plugin-discourse-assign".to_string(), 1),
                ("plugin-discourse-solved".to_string(), 1),
            ]
        );
    }
}
