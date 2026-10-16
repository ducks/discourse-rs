//! The admin problem checks page (templates/admin-problem-checks.gjs):
//! the page header (its tab strip empty) over ProblemChecksList, a
//! ProblemCheckItem row per tracker. static/js/admin-problem-checks.js
//! ignores and watches them.

use serde_json::Value;

use crate::admin_ui::{Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::escape;

/// The page for the trackers' document (/admin/problem_checks.json).
pub fn render(cx: &AdminCx, trackers: &[Value]) -> String {
    let title = t(cx.i18n, "admin.config.problem_checks.title");
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: title.clone(),
                path: "/admin/problem-checks",
            },
        ],
        title,
        actions: String::new(),
        description: Some(escape(&t(
            cx.i18n,
            "admin.config.problem_checks.header_description",
        ))),
        learn_more_url: None,
        // No tabs block given, but DPageHeader draws the strip.
        tabs: Some(String::new()),
    }
    .render(cx);
    let label = |key: &str| escape(&t(cx.i18n, &format!("admin.config.problem_checks.{key}")));
    let mut out = format!(
        "<div class=\"admin-problem-checks admin-config-page\">{header}<div class=\"admin-container admin-config-page__main-area\"><table class=\"d-table admin-problem-checks__items\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\"></th></tr></thead><tbody class=\"d-table__body\">",
        label("status"),
        label("identifier"),
        label("target"),
        label("last_run_at"),
    );
    for tracker in trackers {
        out.push_str(&row(cx, tracker));
    }
    out.push_str("</tbody></table></div></div>");
    out
}

/// ProblemCheckItem.
fn row(cx: &AdminCx, tracker: &Value) -> String {
    let label = |key: &str| escape(&t(cx.i18n, &format!("admin.config.problem_checks.{key}")));
    let status = tracker["status"].as_str().unwrap_or_default();
    let class = match status {
        "passing" => "--success",
        "failing" => "--critical",
        _ => "",
    };
    let target = tracker["target"].as_str().filter(|t| *t != "__NULL__");
    let last_run = crate::admin_ui::time(&tracker["last_run_at"])
        .map(|at| crate::admin_ui::format_date(cx, at, true))
        .unwrap_or_default();
    let ignored = tracker["ignored"].as_bool().unwrap_or(false);
    let button = Button {
        class: "btn-default btn-small",
        icon: Some(if ignored { "eye" } else { "eye-slash" }),
        label: tr(
            cx,
            if ignored {
                "admin.config.problem_checks.watch"
            } else {
                "admin.config.problem_checks.ignore"
            },
        ),
        attrs: format!(
            " data-problem-check-id=\"{}\" data-action=\"{}\"",
            tracker["id"],
            if ignored { "watch" } else { "ignore" }
        ),
        ..Default::default()
    }
    .render();
    format!(
        "<tr class=\"d-table__row --{}\"><td class=\"d-table__cell --overview\"><div class=\"status-label {class}\"><div class=\"status-label-indicator\"></div><div class=\"status-label-text\">{}</div></div></td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div>{}</td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div>{}</td><td class=\"d-table__cell --detail\"><div class=\"d-table__mobile-label\">{}</div>{last_run}</td><td class=\"d-table__cell --controls\"><div class=\"d-table__cell-actions\">{button}</div></td></tr>",
        escape(status),
        if status.is_empty() {
            String::new()
        } else {
            label(status)
        },
        label("identifier"),
        escape(tracker["identifier"].as_str().unwrap_or_default()),
        label("target"),
        escape(target.unwrap_or_default()),
        label("last_run_at"),
    )
}
