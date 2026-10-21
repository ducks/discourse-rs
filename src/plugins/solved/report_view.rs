//! discourse-solved's related items renderer for its accepted_solutions
//! report (registerAdminReportRelatedItemsRenderer): the solved topics
//! under the report's chart (SolvedAdminReportRelatedItems). The table's
//! per-day summary (SolvedAdminReportTableSummaryItem) is drawn by
//! static/js/admin-report.js.

use serde_json::Value;

use crate::admin_view::{AdminCx, t, t_with};
use crate::topic_list_view::{ListContext, escape};

/// The report the renderer is registered for.
pub const REPORT: &str = "accepted_solutions";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `dAvatar(user, imageSize="small", extraClasses)`: titled with the
/// display name (the name when names come first, else the username).
fn avatar(list: &ListContext, user: &Value, class: &str) -> String {
    let username = s(&user["username"]);
    let template = s(&user["avatar_template"]);
    if username.is_empty() || template.is_empty() {
        return String::new();
    }
    let name = s(&user["name"]);
    let title = if list.settings.prioritize_name && !name.trim().is_empty() {
        name
    } else {
        username
    };
    format!(
        "<img alt=\"\" width=\"24\" height=\"24\" src=\"{}\" class=\"avatar {class}\" title=\"{}\">",
        escape(&template.replace("{size}", "24")),
        escape(title)
    )
}

/// `dCategoryBadge(item.category)` of the report's plain category (id,
/// name, slug, color, textColor): an unlinked square badge without
/// parent or restriction, as categoryBadgeHTML draws an object that
/// isn't a Category.
fn category_badge(list: &ListContext, category: &Value) -> String {
    let id = category["id"].as_i64().unwrap_or_default();
    if id == list.settings.uncategorized_category_id && list.settings.suppress_uncategorized_badge {
        return String::new();
    }
    let mut style = String::new();
    if !s(&category["color"]).is_empty() {
        style.push_str(&format!(
            "--category-badge-color: #{};",
            s(&category["color"])
        ));
    }
    if !s(&category["textColor"]).is_empty() {
        style.push_str(&format!(
            "--category-badge-text-color: #{};",
            s(&category["textColor"])
        ));
    }
    let dir = if list.settings.support_mixed_text_direction {
        " dir=\"auto\""
    } else {
        ""
    };
    format!(
        "<span class=\"badge-category__wrapper\"{}><span data-category-id=\"{id}\" data-drop-close=\"true\" class=\"badge-category --style-square\"><span class=\"badge-category__name\"{dir}>{}</span></span></span>",
        if style.is_empty() {
            String::new()
        } else {
            format!(" style=\"{}\"", escape(&style))
        },
        escape(s(&category["name"]))
    )
}

/// SolvedAdminReportRelatedItems: the report's solved topics, with who
/// solved them and their category; nothing without any.
pub fn related_items(
    cx: &AdminCx,
    list: &ListContext,
    r: &Value,
    start: &str,
    end: &str,
) -> String {
    let empty = Vec::new();
    let topics = r["related_items"]["solved_topics"]
        .as_array()
        .unwrap_or(&empty);
    if topics.is_empty() {
        return String::new();
    }
    let total = r["related_items_totals"]["solved_topics"]
        .as_i64()
        .filter(|n| *n != 0)
        .unwrap_or(topics.len() as i64);
    let prefix = "admin.reports.related_items";
    let mut out = format!(
        "<section class=\"admin-report-related-items solved-admin-report-related-items\"><h2>{}</h2><p class=\"admin-report-related-items__description\">{}",
        escape(&t(cx.i18n, &format!("{prefix}.solved_topics.title"))),
        escape(&t_with(
            cx.i18n,
            &format!("{prefix}.solved_topics.description"),
            &[("startDate", start), ("endDate", end)]
        ))
    );
    if (topics.len() as i64) < total {
        out.push_str(&format!(
            " <span class=\"admin-report-related-items__limit\">{}</span>",
            escape(&t_with(
                cx.i18n,
                &format!("{prefix}.showing"),
                &[
                    ("shown", &topics.len().to_string()),
                    ("total", &total.to_string())
                ]
            ))
        ));
    }
    out.push_str(&format!(
        "</p><table class=\"table admin-report-related-items__table solved-admin-report-related-items__table\"><thead><tr><th class=\"solved-admin-report-related-items__topic-cell\">{}</th><th class=\"solved-admin-report-related-items__solved-by-cell\">{}</th><th class=\"solved-admin-report-related-items__category-cell\">{}</th></tr></thead><tbody>",
        escape(&t(cx.i18n, &format!("{prefix}.labels.topic"))),
        escape(&t(cx.i18n, &format!("{prefix}.labels.solved_by"))),
        escape(&t(cx.i18n, &format!("{prefix}.labels.category")))
    ));
    for item in topics {
        out.push_str(&format!(
            "<tr><td class=\"solved-admin-report-related-items__topic-cell\"><a class=\"solved-admin-report-related-items__topic-link\" href=\"{}\">{}</a></td><td class=\"solved-admin-report-related-items__solved-by-cell\">",
            escape(s(&item["topic"]["url"])),
            escape(s(&item["topic"]["title"]))
        ));
        let users = item["solved_by_users"].as_array().unwrap_or(&empty);
        if !users.is_empty() {
            out.push_str("<div class=\"solved-admin-report-related-items__solved-by-users\">");
            for user in users {
                let username = s(&user["username"]);
                out.push_str(&format!(
                    "<a class=\"admin-report-related-items__user-link\" href=\"{}/u/{}\">{}<span class=\"admin-report-related-items__username\">{}</span></a>",
                    cx.base_path,
                    escape(&username.to_lowercase()),
                    avatar(list, user, "admin-report-related-items__avatar"),
                    escape(username)
                ));
            }
            out.push_str("</div>");
        }
        out.push_str("</td><td class=\"solved-admin-report-related-items__category-cell\">");
        if item["category"].is_object() {
            out.push_str(&category_badge(list, &item["category"]));
        } else {
            out.push_str(&escape(&t(cx.i18n, "category.none")));
        }
        out.push_str("</td></tr>");
    }
    out.push_str("</tbody></table></section>");
    out
}
