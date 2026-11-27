//! The admin's badges page for one user as the Ember client renders it
//! (templates/admin-user/badges.gjs with AdminUserBadgesController's
//! getters) from `/badges.json` and `/user-badges/:username.json`:
//! the grant form over grantableBadges, and the grants grouped per badge.
//! static/js/admin-user-badges.js grants, revokes and expands.

use serde_json::Value;

use crate::admin::view::{AdminCx, t, t_with};
use crate::preferences_view::{Combo, combo_rows};
use crate::topic_list_view::{ListContext, escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// What the page reads.
pub struct Docs<'a> {
    /// BadgesController#index: every badge.
    pub badges: &'a Value,
    /// UserBadgesController#username: the user's grants.
    pub user_badges: &'a Value,
}

/// grantableBadges: the enabled, manually grantable badges the user
/// lacks or may hold again, by name.
fn grantable<'a>(all: &'a [Value], grants: &[Value]) -> Vec<&'a Value> {
    let mut out: Vec<&Value> = all
        .iter()
        .filter(|b| {
            b["enabled"] == true
                && b["manually_grantable"] == true
                && (b["multiple_grant"] == true || !grants.iter().any(|g| g["badge_id"] == b["id"]))
        })
        .collect();
    out.sort_by(|a, b| {
        s(&a["name"])
            .to_lowercase()
            .cmp(&s(&b["name"]).to_lowercase())
    });
    out
}

/// A row of groupedBadges.
struct Row<'a> {
    badge: &'a Value,
    granted_at: &'a str,
    /// The grant itself; none for a group.
    grant: Option<&'a Value>,
    count: usize,
    /// The grants a group stands for.
    members: Vec<&'a Value>,
}

/// groupedBadges: per badge id (ascending, as Object.values reads integer
/// keys), one row for a single grant or an expanded badge, else a group
/// row with the latest grant time and the count; latest first.
fn grouped<'a>(doc: &'a Value, grants: &'a [Value]) -> Vec<Row<'a>> {
    let badges = doc["badges"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let mut ids: Vec<i64> = grants
        .iter()
        .filter_map(|g| g["badge_id"].as_i64())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut rows = Vec::new();
    for id in ids {
        let Some(badge) = badges.iter().find(|b| b["id"].as_i64() == Some(id)) else {
            continue;
        };
        let members: Vec<&Value> = grants
            .iter()
            .filter(|g| g["badge_id"].as_i64() == Some(id))
            .collect();
        if members.len() == 1 {
            rows.push(Row {
                badge,
                granted_at: s(&members[0]["granted_at"]),
                grant: Some(members[0]),
                count: 1,
                members: Vec::new(),
            });
            continue;
        }
        let latest = members
            .iter()
            .map(|g| s(&g["granted_at"]))
            .max()
            .unwrap_or_default();
        rows.push(Row {
            badge,
            granted_at: latest,
            grant: None,
            count: members.len(),
            members,
        });
    }
    rows.sort_by(|a, b| b.granted_at.cmp(a.granted_at));
    rows
}

/// UserBadge with its grant count.
fn user_badge(cx: &AdminCx, doc: &Value, badge: &Value, count: usize) -> String {
    let button = crate::admin::badges_view::badge_button(doc, badge);
    let count = if count > 1 {
        format!("<span class=\"count\">&nbsp;(&times;{count})</span>")
    } else {
        String::new()
    };
    let button = match button.strip_suffix("</span>") {
        Some(open) => format!("{open}{count}</span>"),
        None => button,
    };
    format!(
        "<a class=\"user-card-badge-link\" href=\"{}/badges/{}/{}\">{button}</a>",
        cx.base_path,
        badge["id"],
        escape(s(&badge["slug"]))
    )
}

fn tr(cx: &AdminCx, list: &ListContext, doc: &Value, row: &Row, hidden_of: Option<i64>) -> String {
    let i18n = cx.i18n;
    let users = doc["granted_bies"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let granted_by = row
        .grant
        .and_then(|g| users.iter().find(|u| u["id"] == g["granted_by_id"]));
    let by = match granted_by {
        Some(u) => format!(
            "<a href=\"{}/admin/users/{}/{}\">{} {}</a>",
            cx.base_path,
            u["id"],
            escape(&s(&u["username"]).to_lowercase()),
            crate::post_view::avatar_img(
                s(&u["avatar_template"]),
                20,
                &format!(" title=\"{}\"", escape(s(&u["username"])))
            ),
            escape(s(&u["username"]))
        ),
        None => "<a></a>".into(),
    };
    // postUrl: the reason's post, when the grant names a topic.
    let reason = row
        .grant
        .filter(|g| g["topic_title"].as_str().is_some_and(|t| !t.is_empty()))
        .map(|g| {
            format!(
                "<a href=\"/t/-/{}/{}\">{}</a>",
                g["topic_id"],
                g["post_number"],
                escape(s(&g["topic_title"]))
            )
        })
        .unwrap_or_default();
    // dAgeWithTooltip
    let age = row
        .granted_at
        .parse::<chrono::DateTime<chrono::Utc>>()
        .map(|at| crate::viewer_dates::tiny_span(list, at))
        .unwrap_or_default();
    let action = match row.grant {
        Some(g) => format!(
            "<button class=\"btn btn-text btn-danger revoke-badge\" type=\"button\" data-user-badge-id=\"{}\"><span class=\"d-button-label\">{}</span></button>",
            g["id"],
            t(i18n, "admin.badges.revoke")
        ),
        None => format!(
            "<button class=\"btn btn-text btn-default expand-badge\" type=\"button\" data-badge-id=\"{}\"><span class=\"d-button-label\">{}</span></button>",
            row.badge["id"],
            t(i18n, "admin.badges.expand")
        ),
    };
    let millis = chrono::DateTime::parse_from_rfc3339(row.granted_at)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0);
    let hidden = hidden_of
        .map(|id| format!(" data-grouped-in=\"{id}\" hidden"))
        .unwrap_or_default();
    format!(
        "<tr data-granted-at=\"{millis}\"{hidden}><td>{}</td><td>{by}</td><td class=\"reason\">{reason}</td><td>{age}</td><td>{action}</td></tr>",
        user_badge(cx, doc, row.badge, row.count)
    )
}

pub fn render(cx: &AdminCx, list: &ListContext, u: &Value, docs: &Docs) -> String {
    let i18n = cx.i18n;
    let username = s(&u["username"]);
    let all = docs.badges["badges"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let grants = docs.user_badges["user_badges"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let available = grantable(all, grants);
    let form = if available.is_empty() {
        format!("<p>{}</p>", escape(&t(i18n, "admin.badges.no_badges")))
    } else {
        let rows: Vec<(String, String)> = available
            .iter()
            .map(|b| (b["id"].to_string(), s(&b["name"]).to_string()))
            .collect();
        let icons: Vec<(String, String)> = available
            .iter()
            .map(|b| {
                (
                    b["id"].to_string(),
                    crate::admin::badges_view::combo_icon(s(&b["icon"])),
                )
            })
            .collect();
        let combo = combo_rows(
            list,
            &Combo {
                id: None,
                component_class: "",
                class: "grant-badge-combo",
                rows,
                value: Some(available[0]["id"].to_string()),
                none: None,
                key: None,
                filterable: true,
            },
            "",
            &|value, name, _| {
                let icon = icons
                    .iter()
                    .find(|(id, _)| id == value)
                    .map(|(_, i)| i.as_str())
                    .unwrap_or_default();
                format!("{icon}<span class=\"name\">{}</span>", escape(name))
            },
        );
        format!(
            "<form class=\"form-horizontal grant-badge-form\"><div class=\"control-group\"><label>{}</label>{combo}</div>\
<div class=\"control-group\"><label>{}</label><input class=\"ember-text-field badge-reason\" type=\"text\"><br><small>{}</small></div>\
<button class=\"btn btn-text btn-primary\" type=\"submit\"><span class=\"d-button-label\">{}</span></button></form>",
            escape(&t(i18n, "admin.badges.badge")),
            escape(&t(i18n, "admin.badges.reason")),
            escape(&t(i18n, "admin.badges.reason_help")),
            t(i18n, "admin.badges.grant")
        )
    };
    let doc = docs.user_badges;
    let mut body = String::new();
    let rows = grouped(doc, grants);
    for row in &rows {
        body.push_str(&tr(cx, list, doc, row, None));
        // A group's grants, for expandGroup to show in its place.
        for g in &row.members {
            let single = Row {
                badge: row.badge,
                granted_at: s(&g["granted_at"]),
                grant: Some(g),
                count: 1,
                members: Vec::new(),
            };
            body.push_str(&tr(cx, list, doc, &single, row.badge["id"].as_i64()));
        }
    }
    if rows.is_empty() {
        body.push_str(&format!(
            "<tr><td colspan=\"5\"><p>{}</p></td></tr>",
            escape(&t_with(
                i18n,
                "admin.badges.no_user_badges",
                &[("name", username)]
            ))
        ));
    }
    let strings = serde_json::json!({
        "admin.badges.revoke_confirm": t(i18n, "admin.badges.revoke_confirm"),
        "generic_error": t(i18n, "generic_error"),
    });
    format!(
        "<div class=\"admin-controls\"><nav><ul class=\"nav nav-pills\"><li><a href=\"{}/admin/users/{}/{}\">{} &nbsp;{}</a></li></ul></nav></div>\
<div class=\"admin-container user-badges\" data-username=\"{}\"><h2>{}</h2><br>{form}\
<table id=\"user-badges\"><tbody><tr><th>{}</th><th>{}</th><th class=\"reason\">{}</th><th>{}</th><th></th></tr>{body}</tbody></table></div>\
<script type=\"application/json\" id=\"admin-user-badges-strings\">{}</script>\
<script type=\"application/json\" id=\"dialog-strings\">{}</script>",
        cx.base_path,
        u["id"],
        escape(&username.to_lowercase()),
        icon("angle-left", None),
        escape(username),
        escape(username),
        escape(&t(i18n, "admin.badges.grant_badge")),
        escape(&t(i18n, "admin.badges.badge")),
        escape(&t(i18n, "admin.badges.granted_by")),
        escape(&t(i18n, "admin.badges.reason")),
        escape(&t(i18n, "admin.badges.granted_at")),
        strings.to_string().replace('<', "\\u003c"),
        crate::html::dialog_strings(i18n),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grantable_badges_skip_held_single_grants_and_sort_by_name() {
        let all = vec![
            json!({"id": 1, "name": "zeta", "enabled": true, "manually_grantable": true, "multiple_grant": false}),
            json!({"id": 2, "name": "Alpha", "enabled": true, "manually_grantable": true, "multiple_grant": true}),
            json!({"id": 3, "name": "beta", "enabled": true, "manually_grantable": true, "multiple_grant": false}),
            json!({"id": 4, "name": "off", "enabled": false, "manually_grantable": true, "multiple_grant": false}),
            json!({"id": 5, "name": "auto", "enabled": true, "manually_grantable": false, "multiple_grant": false}),
        ];
        let grants = vec![json!({"badge_id": 1}), json!({"badge_id": 2})];
        let names: Vec<&str> = grantable(&all, &grants)
            .iter()
            .map(|b| b["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Alpha", "beta"]);
    }

    #[test]
    fn grants_of_one_badge_group_under_the_latest() {
        let doc = json!({"badges": [{"id": 7}, {"id": 3}]});
        let grants = vec![
            json!({"id": 1, "badge_id": 7, "granted_at": "2026-01-01T00:00:00.000Z"}),
            json!({"id": 2, "badge_id": 3, "granted_at": "2026-02-01T00:00:00.000Z"}),
            json!({"id": 3, "badge_id": 7, "granted_at": "2026-03-01T00:00:00.000Z"}),
        ];
        let rows = grouped(&doc, &grants);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].badge["id"], 7);
        assert_eq!(rows[0].count, 2);
        assert!(rows[0].grant.is_none());
        assert_eq!(rows[0].granted_at, "2026-03-01T00:00:00.000Z");
        assert_eq!(rows[1].grant.unwrap()["id"], 2);
    }
}
