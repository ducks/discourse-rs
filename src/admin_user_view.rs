//! The admin's page for one user as the Ember client renders it
//! (templates/admin-user/index.gjs with AdminUser's and the controller's
//! getters) from what Admin::UsersController#show serves: the details,
//! permissions, groups, activity, DiscourseConnect and exports sections,
//! and the danger controls. static/js/admin-user.js runs its actions
//! through the admin user JSON endpoints.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::admin_view::{AdminCx, t, t_count, t_with};
use crate::topic_list_view::{ListContext, escape, icon};

/// What the page reads besides the user.
pub struct UserPage<'a> {
    /// The site's user fields (id, name), by position.
    pub user_fields: &'a [(i64, String)],
    pub now: DateTime<Utc>,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// DButton: an icon and a label, either, a title, extra classes.
fn button(
    class: &str,
    icon_name: Option<&str>,
    label: Option<&str>,
    title: Option<&str>,
) -> String {
    let kind = match (icon_name, label) {
        (Some(_), Some(_)) => "btn btn-icon-text",
        (Some(_), None) => "btn no-text btn-icon",
        _ => "btn btn-text",
    };
    let title = title
        .map(|t| format!(" title=\"{}\"", escape(t)))
        .unwrap_or_default();
    let inner = match (icon_name, label) {
        (Some(i), Some(l)) => format!(
            "{}<span class=\"d-button-label\">{}</span>",
            icon(i, None),
            escape(l)
        ),
        (Some(i), None) => format!(
            "{}<span aria-hidden=\"true\"> &#8203; </span>",
            icon(i, None)
        ),
        (None, Some(l)) => format!("<span class=\"d-button-label\">{}</span>", escape(l)),
        (None, None) => String::new(),
    };
    let class = if class.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} {class}")
    };
    format!("<button class=\"{class}\"{title} type=\"button\">{inner}</button>")
}

/// `i18nYesNo`
fn yes_no(cx: &AdminCx, v: bool) -> String {
    escape(&t(cx.i18n, if v { "yes_value" } else { "no_value" }))
}

/// Sibling controls with the whitespace the template leaves between them,
/// which lets them wrap.
fn spaced(html: &str) -> String {
    let mut out = html.to_string();
    for (from, to) in [
        ("</button><button", "</button> <button"),
        ("</button><a ", "</button> <a "),
        ("</a><button", "</a> <button"),
        ("</a><a ", "</a> <a "),
        ("</span><button", "</span> <button"),
        ("</button><details", "</button> <details"),
        ("</details><button", "</details> <button"),
    ] {
        out = out.replace(from, to);
    }
    out
}

fn row(class: &str, field: &str, value: &str, controls: Option<&str>) -> String {
    let class = if class.is_empty() {
        "display-row".to_string()
    } else {
        format!("display-row {class}")
    };
    format!(
        "<div class=\"{class}\"><div class=\"field\">{field}</div><div class=\"value\">{value}</div>{}</div>",
        controls
            .map(|c| format!("<div class=\"controls\">{c}</div>"))
            .unwrap_or_default()
    )
}

/// AdminEditableField, not editing: the value as a link that starts
/// editing, and the pencil.
fn editable(cx: &AdminCx, class: &str, name_key: &str, value: &str, field: &str) -> String {
    format!(
        "<div class=\"display-row{}\" data-editable=\"{field}\"><div class=\"field\">{}</div><div class=\"value\"><a class=\"inline-editable-field\" href=\"\"><span>{}</span></a></div><div class=\"controls\">{}</div></div>",
        if class.is_empty() {
            String::new()
        } else {
            format!(" {class}")
        },
        escape(&t(cx.i18n, name_key)),
        escape(value),
        button("btn-default", Some("pencil"), None, None)
    )
}

/// `isForever`: more than a hundred years away.
fn is_forever(now: DateTime<Utc>, at: DateTime<Utc>) -> bool {
    (at - now).num_days() > 365 * 100
}

fn parse(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.to_utc())
}

/// `dFormatDate(date, leaveAgo)`, empty for none.
fn format_date(list: &ListContext, v: &Value, leave_ago: bool) -> String {
    parse(v)
        .map(|at| crate::user_profile_view::age_medium(list, at, leave_ago))
        .unwrap_or_default()
}

/// A staff member's avatar and name links (suspended or silenced by,
/// approved by).
fn by_links(cx: &AdminCx, user: &Value, size: i64) -> String {
    let id = user["id"].as_i64().unwrap_or(0);
    let username = s(&user["username"]);
    let href = format!(
        "{}/admin/users/{id}/{}",
        cx.base_path,
        escape(&username.to_lowercase())
    );
    format!(
        "<a href=\"{href}\">{}</a> <a href=\"{href}\">{}</a>",
        crate::post_view::avatar_img(
            s(&user["avatar_template"]),
            size,
            &format!(" title=\"{}\"", escape(username))
        ),
        escape(username)
    )
}

pub fn render(cx: &AdminCx, list: &ListContext, page: &UserPage, u: &Value) -> String {
    let i18n = cx.i18n;
    let base = cx.base_path;
    let viewer = cx.viewer;
    let id = u["id"].as_i64().unwrap_or(0);
    let username = s(&u["username"]);
    let username_lower = username.to_lowercase();
    let active = truthy(&u["active"]);
    let staged = truthy(&u["staged"]);
    let admin = truthy(&u["admin"]);
    let moderator = truthy(&u["moderator"]);
    let staff = admin || moderator;
    let preferences_path = format!("{base}/u/{}/preferences", escape(&username_lower));
    let visit_profile = t_with(
        i18n,
        "admin.user.visit_profile",
        &[("url", &preferences_path)],
    );
    let moderators_view_emails = cx
        .settings
        .get("moderators_view_emails")
        .map(|v| v.truthy())
        .unwrap_or(false);
    let can_check_emails =
        id == viewer.id || viewer.admin || (moderators_view_emails && viewer.staff);
    let check_email = button(
        "btn-default",
        Some("envelope"),
        Some(&t(i18n, "admin.users.check_email.text")),
        Some(&t(i18n, "admin.users.check_email.title")),
    );
    let mut out = format!(
        "<section><section class=\"details {}\"><div class=\"user-controls\">",
        if active { "" } else { "not-activated" }
    );
    // canViewProfile
    if active || staged {
        out.push_str(&format!(
            "<a class=\"btn btn-default\" href=\"{base}/u/{}\">{} {}</a>",
            escape(&username_lower),
            icon("user", None),
            escape(&t(i18n, "admin.user.show_public_profile"))
        ));
    }
    if truthy(&u["can_view_action_logs"]) {
        out.push_str(&button(
            "btn-default",
            Some("far-rectangle-list"),
            Some(&t(i18n, "admin.user.action_logs")),
            None,
        ));
    }
    if active && viewer.admin {
        out.push_str(&button(
            "btn-default",
            Some("power-off"),
            Some(&t(i18n, "admin.user.log_out")),
            None,
        ));
    }
    // The admin-user-controls-after outlet: discourse-user-notes'.
    out.push_str(&crate::plugins::user_notes::view::admin_user_controls(
        i18n, u,
    ));
    out.push_str("</div>");
    out.push_str(&editable(
        cx,
        "username",
        "user.username.title",
        username,
        "username",
    ));
    out.push_str(&editable(
        cx,
        "name",
        "user.name.title",
        s(&u["name"]),
        "name",
    ));
    if can_check_emails {
        let email = s(&u["email"]);
        let mut value = String::new();
        if !active {
            value.push_str(&format!(
                "<div class=\"controls\">{}</div>",
                escape(&t(i18n, "admin.users.not_verified"))
            ));
        }
        if email.is_empty() {
            value.push_str(&check_email);
        } else {
            value.push_str(&format!(
                "<a href=\"mailto:{e}\">{e}</a>",
                e = escape(email)
            ));
        }
        let auth_overrides = cx
            .settings
            .get("auth_overrides_email")
            .map(|v| v.truthy())
            .unwrap_or(false);
        let controls = if auth_overrides {
            escape(&t(i18n, "user.email.auth_override_instructions"))
        } else if !email.is_empty() {
            visit_profile.clone()
        } else {
            String::new()
        };
        out.push_str(&row(
            "email",
            &escape(&t(i18n, "user.email.primary")),
            &value,
            Some(&controls),
        ));
        let secondary: Vec<&str> = u["secondary_emails"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let (value, controls) = if email.is_empty() {
            (check_email.clone(), String::new())
        } else if secondary.is_empty() {
            (escape(&t(i18n, "user.email.no_secondary")), String::new())
        } else {
            let items: String = secondary
                .iter()
                .map(|e| format!("<li><a href=\"mailto:{e}\">{e}</a></li>", e = escape(e)))
                .collect();
            let controls = if auth_overrides {
                escape(&t(i18n, "user.email.auth_override_instructions"))
            } else {
                visit_profile.clone()
            };
            (format!("<ul>{items}</ul>"), controls)
        };
        out.push_str(&row(
            "secondary-emails",
            &escape(&t(i18n, "user.email.secondary")),
            &value,
            Some(&controls),
        ));
        let bounce = u["bounce_score"].as_f64().unwrap_or(0.0);
        let threshold = cx
            .settings
            .get("bounce_score_threshold")
            .map(|v| v.to_f())
            .unwrap_or(4.0);
        let bounce_value = if bounce > 0.0 {
            format!(
                "{} - {}",
                js_number(bounce),
                parse(&u["reset_bounce_score_after"])
                    .map(|at| crate::viewer_dates::marker(at, "LL"))
                    .unwrap_or_default()
            )
        } else {
            js_number(bounce)
        };
        let explanation = if bounce == 0.0 {
            "none"
        } else if bounce < threshold {
            "some"
        } else {
            "threshold_reached"
        };
        let mut controls = String::new();
        if bounce > 0.0 {
            controls.push_str(&button(
                "btn-default",
                None,
                Some(&t(i18n, "admin.user.reset_bounce_score.label")),
                Some(&t(i18n, "admin.user.reset_bounce_score.title")),
            ));
        }
        controls.push_str(&escape(&t(
            i18n,
            &format!("admin.user.bounce_score_explanation.{explanation}"),
        )));
        // The reset date in the viewer's zone (crate::viewer_dates).
        let (bounce_tz, bounce_value) = crate::viewer_dates::text(list, &bounce_value);
        out.push_str(
            &row(
                "bounce-score",
                &format!(
                    "<a href=\"{base}/admin/email-logs/bounced\">{}</a>",
                    escape(&t(i18n, "admin.user.bounce_score"))
                ),
                &bounce_value,
                Some(&controls),
            )
            .replacen(
                "<div class=\"value\">",
                &format!("<div class=\"value\"{bounce_tz}>"),
                1,
            ),
        );
        // associated_accounts comes with the email (checkEmail), or with
        // the user when the email is shown.
        let accounts = match u.get("associated_accounts") {
            Some(list) => escape(
                &list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| format!("{} ({})", s(&p["name"]), s(&p["description"])))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            None => check_email.clone(),
        };
        let delete_accounts = (viewer.admin
            && u["associated_accounts"]
                .as_array()
                .is_some_and(|a| !a.is_empty()))
        .then(|| {
            button(
                "btn-danger",
                Some("trash-can"),
                Some(&t(i18n, "admin.users.delete_associated_accounts.text")),
                Some(&t(i18n, "admin.users.delete_associated_accounts.title")),
            )
        });
        out.push_str(&row(
            "associations",
            &escape(&t(i18n, "user.associated_accounts.title")),
            &accounts,
            delete_accounts.as_deref(),
        ));
    }
    out.push_str(&row(
        "",
        &escape(&t(i18n, "user.avatar.title")),
        &crate::post_view::avatar_img(
            s(&u["avatar_template"]),
            48,
            &format!(" title=\"{}\"", escape(username)),
        ),
        Some(&visit_profile),
    ));
    out.push_str(&editable(
        cx,
        "",
        "user.title.title",
        s(&u["title"]),
        "title",
    ));
    if truthy(&u["include_ip"]) {
        for (class, key, label, kind) in [
            ("last-ip", "ip_address", "user.ip_address.title", "last"),
            (
                "registration-ip",
                "registration_ip_address",
                "user.registration_ip_address.title",
                "registration",
            ),
        ] {
            let ip = s(&u[key]);
            let controls = if viewer.can_see_ip && !ip.is_empty() {
                format!(
                    "<button aria-expanded=\"false\" class=\"btn btn-icon-text fk-d-menu__trigger ip-lookup-trigger btn-default\" data-identifier=\"ip-lookup\" data-trigger=\"\" type=\"button\" data-ip=\"{}\" data-ip-type=\"{kind}\">{}<span class=\"d-button-label\">{}</span></button>",
                    escape(ip),
                    icon("globe", None),
                    escape(&t(i18n, "admin.user.ip_lookup"))
                )
            } else {
                String::new()
            };
            out.push_str(&row(
                class,
                &escape(&t(i18n, label)),
                &escape(ip),
                Some(&controls),
            ));
        }
    }
    if cx
        .settings
        .get("enable_badges")
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        out.push_str(&row(
            "",
            &escape(&t(i18n, "admin.badges.title")),
            &escape(&t_count(
                i18n,
                "badges.badge_count",
                u["badge_count"].as_i64().unwrap_or(0),
                &[],
            )),
            Some(&format!(
                "<a class=\"btn btn-default\" href=\"{base}/admin/users/{id}/{}/badges\">{} {}</a>",
                escape(&username_lower),
                icon("certificate", None),
                escape(&t(i18n, "admin.badges.edit_badges"))
            )),
        ));
    }
    let second_factor = truthy(&u["second_factor_enabled"]);
    let controls = if second_factor && truthy(&u["can_disable_second_factor"]) {
        button(
            "btn-default disable-second-factor",
            Some("unlock-keyhole"),
            Some(&t(i18n, "user.second_factor.disable")),
            None,
        )
    } else {
        String::new()
    };
    out.push_str(&row(
        "second-factor",
        &escape(&t(i18n, "user.second_factor.title")),
        &yes_no(cx, second_factor),
        Some(&controls),
    ));
    out.push_str("</section>");

    // userFields
    if !page.user_fields.is_empty() {
        out.push_str("<section class=\"details\">");
        for (field_id, name) in page.user_fields {
            let value = u["user_fields"][field_id.to_string()].clone();
            let value = match &value {
                Value::String(v) if !v.is_empty() => escape(v),
                Value::Array(a) if !a.is_empty() => escape(
                    &a.iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                _ => "&mdash;".to_string(),
            };
            out.push_str(&row("", &escape(name), &value, None));
        }
        out.push_str("</section>");
    }
    out.push_str("<span></span>");

    out.push_str(&permissions(cx, list, page, u, staff));
    if viewer.admin {
        out.push_str(&groups(cx, u));
    }
    out.push_str(&activity(cx, list, u));
    if !u["single_sign_on_record"].is_null() {
        out.push_str(&sso(cx, u));
    }
    if viewer.admin {
        out.push_str(&format!(
            "<section class=\"details\"><h1>{}</h1>",
            escape(&t(i18n, "admin.user.exports.title"))
        ));
        let export = &u["latest_export"]["user_export"];
        let value = if export.is_object() {
            format!(
                "<a class=\"download\" href=\"{}\">{}</a><br>{}<br>{}",
                escape(s(&export["uri"])),
                escape(s(&export["filename"])),
                escape(s(&export["human_filesize"])),
                escape(&t_count(
                    i18n,
                    "admin.user.exports.download.expires_in",
                    export["retain_hours"].as_i64().unwrap_or(0),
                    &[]
                ))
            )
        } else {
            escape(&t(i18n, "admin.user.exports.download.not_available"))
        };
        out.push_str(&row(
            "",
            &escape(&t(i18n, "admin.user.exports.download.description")),
            &value,
            Some(&format!(
                "<div class=\"loading-container\">{}</div>",
                button(
                    "btn-default",
                    Some("download"),
                    Some(&t(i18n, "admin.user.exports.download.button")),
                    None
                )
            )),
        ));
        out.push_str("</section>");
    }
    out.push_str("<span></span>");
    out.push_str(&danger(cx, u, active, admin));
    out.push_str("<div class=\"clearfix\"></div></section>");
    let mut out = spaced(&out);
    out.push_str(&client_strings(cx));
    out
}

/// The strings admin-user.js and dialog.js show, as `i18n` resolves them
/// for staff, placeholders intact; and the delete box's options.
fn client_strings(cx: &AdminCx) -> String {
    const KEYS: &[&str] = &[
        "generic_error",
        "admin.user.action_logs",
        "admin.user.log_out",
        "admin.user.logged_out",
        "admin.user.deactivate_account",
        "admin.user.activate",
        "admin.user.approve",
        "admin.user.revoke_admin",
        "admin.user.grant_admin",
        "admin.user.grant_admin_confirm",
        "admin.user.revoke_moderation",
        "admin.user.grant_moderation",
        "admin.user.lock_trust_level",
        "admin.user.unlock_trust_level",
        "admin.user.trust_level_change_failed",
        "admin.user.unsuspend",
        "admin.user.unsilence",
        "admin.user.clear_penalty_history.title",
        "admin.impersonate.title",
        "admin.impersonate.error",
        "admin.user.anonymize",
        "admin.user.anonymize_confirm",
        "admin.user.anonymize_yes",
        "admin.user.anonymize_failed",
        "admin.user.suspend",
        "admin.user.silence",
        "admin.users.check_email.text",
        "admin.users.check_sso.text",
        "admin.user.reset_bounce_score.label",
        "admin.user.send_activation_email",
        "admin.user.ip_lookup",
        "user.second_factor.disable",
        "admin.user.upcoming_changes.view_modal",
        "admin.user.merge.button",
        "admin.user.delete_posts.button",
        "admin.user.exports.download.button",
        "admin.user.discourse_connect.delete_sso_record",
        "admin.users.delete_associated_accounts.text",
        "admin.user.delete",
        "admin.user.delete_confirm_title",
        "admin.user.delete_confirm",
    ];
    let strings: serde_json::Map<String, Value> = KEYS
        .iter()
        .map(|k| {
            let raw = crate::admin_view::t_opt(cx.i18n, k).unwrap_or_else(|| format!("[en.{k}]"));
            (k.to_string(), Value::String(raw))
        })
        .collect();
    let dialog = serde_json::json!({
        "ok_value": t(cx.i18n, "ok_value"),
        "cancel": t(cx.i18n, "cancel"),
        "modal.close": t(cx.i18n, "modal.close"),
    });
    // adminTools.deleteUserOptions
    let mut options = String::new();
    for (id, icon_name, flags) in [
        ("delete_dont_block", "trash-can", "{}"),
        (
            "delete_and_block_email",
            "envelope",
            "{\"block_email\":true}",
        ),
        (
            "delete_and_block",
            "ban",
            "{\"block_email\":true,\"block_urls\":true,\"block_ip\":true}",
        ),
    ] {
        let name = t(cx.i18n, &format!("admin.user.{id}"));
        options.push_str(&format!(
            "<div class=\"select-kit-row\" role=\"menuitemradio\" data-value=\"{id}\" data-name=\"{n}\" data-flags=\"{}\" tabindex=\"0\">{}<div class=\"texts\"><span class=\"name\">{n}</span><span class=\"desc\">{}</span></div></div>",
            escape(flags),
            icon(icon_name, None),
            escape(&t(cx.i18n, &format!("admin.user.{id}_description"))),
            n = escape(&name)
        ));
    }
    format!(
        "<script type=\"application/json\" id=\"admin-user-strings\">{}</script><script type=\"application/json\" id=\"dialog-strings\">{}</script><template id=\"admin-user-delete-options\"><div class=\"select-kit-collection\" role=\"menu\">{options}</div></template>",
        Value::Object(strings).to_string().replace('<', "\\u003c"),
        dialog.to_string().replace('<', "\\u003c")
    )
}

/// A JavaScript number as a template prints it.
fn js_number(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        f.to_string()
    }
}

/// The Permissions section.
fn permissions(
    cx: &AdminCx,
    list: &ListContext,
    page: &UserPage,
    u: &Value,
    staff: bool,
) -> String {
    let i18n = cx.i18n;
    let base = cx.base_path;
    let mut out = format!(
        "<section class=\"details\"><h1>{}</h1>",
        escape(&t(i18n, "admin.user.permissions"))
    );
    if cx
        .settings
        .get("must_approve_users")
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        let approved = truthy(&u["approved"]);
        let value = if approved {
            format!(
                "{} {}",
                escape(&t(i18n, "admin.user.approved_by")),
                by_links(cx, &u["approved_by"], 24)
            )
        } else {
            yes_no(cx, false)
        };
        let controls = if approved {
            escape(&t(i18n, "admin.user.approve_success"))
        } else if truthy(&u["can_approve"]) {
            button(
                "btn-default",
                Some("check"),
                Some(&t(i18n, "admin.user.approve")),
                None,
            )
        } else {
            String::new()
        };
        out.push_str(&row(
            "",
            &escape(&t(i18n, "admin.users.approved")),
            &value,
            Some(&controls),
        ));
    }
    let active = truthy(&u["active"]);
    let mut controls = String::new();
    if active {
        if truthy(&u["can_deactivate"]) {
            controls.push_str(&button(
                "btn-default",
                None,
                Some(&t(i18n, "admin.user.deactivate_account")),
                None,
            ));
            controls.push(' ');
            controls.push_str(&escape(&t(i18n, "admin.user.deactivate_explanation")));
        }
    } else {
        if truthy(&u["can_send_activation_email"]) {
            controls.push_str(&button(
                "btn-default",
                Some("envelope"),
                Some(&t(i18n, "admin.user.send_activation_email")),
                None,
            ));
        }
        if truthy(&u["can_activate"]) {
            controls.push_str(&button(
                "btn-default",
                Some("check"),
                Some(&t(i18n, "admin.user.activate")),
                None,
            ));
        }
    }
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.users.active")),
        &yes_no(cx, active),
        Some(&controls),
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.staged")),
        &yes_no(cx, truthy(&u["staged"])),
        Some(&escape(&t(i18n, "admin.user.staged_explanation"))),
    ));
    if cx.viewer.admin {
        out.push_str(&row(
            "",
            &escape(&t(i18n, "admin.api.active_keys")),
            &u["api_key_count"].as_i64().unwrap_or(0).to_string(),
            Some(&format!(
                "<a class=\"btn btn-default\" href=\"{base}/admin/api/keys\"><span class=\"d-button-label\">{}</span></a>",
                escape(&t(i18n, "admin.api.manage_keys"))
            )),
        ));
    }
    let mut controls = String::new();
    if truthy(&u["can_revoke_admin"]) {
        controls.push_str(&button(
            "btn-default",
            Some("shield-halved"),
            Some(&t(i18n, "admin.user.revoke_admin")),
            None,
        ));
    }
    if truthy(&u["can_grant_admin"]) {
        controls.push_str(&button(
            "btn-default grant-admin",
            Some("shield-halved"),
            Some(&t(i18n, "admin.user.grant_admin")),
            None,
        ));
    }
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.admin")),
        &yes_no(cx, truthy(&u["admin"])),
        Some(&controls),
    ));
    let mut controls = String::new();
    if truthy(&u["can_revoke_moderation"]) {
        controls.push_str(&button(
            "btn-default",
            Some("shield-halved"),
            Some(&t(i18n, "admin.user.revoke_moderation")),
            None,
        ));
    }
    if truthy(&u["can_grant_moderation"]) {
        controls.push_str(&button(
            "btn-default",
            Some("shield-halved"),
            Some(&t(i18n, "admin.user.grant_moderation")),
            None,
        ));
    }
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.moderator")),
        &yes_no(cx, truthy(&u["moderator"])),
        Some(&controls),
    ));
    out.push_str(&trust_level_row(cx, u));
    out.push_str(&penalty_rows(cx, list, page, u, staff, "suspended"));
    out.push_str(&penalty_rows(cx, list, page, u, staff, "silenced"));
    let total = u["tl3_requirements"]["penalty_counts"]["total"]
        .as_i64()
        .unwrap_or(0);
    if total > 0 {
        let controls = if cx.viewer.admin {
            Some(format!(
                "{} {}",
                button(
                    "btn-default",
                    Some("xmark"),
                    Some(&t(i18n, "admin.user.clear_penalty_history.title")),
                    None
                ),
                escape(&t(i18n, "admin.user.clear_penalty_history.description"))
            ))
        } else {
            None
        };
        out.push_str(&row(
            "clear-penalty-history",
            &escape(&t(i18n, "admin.user.penalty_count")),
            &total.to_string(),
            controls.as_deref(),
        ));
    }
    if cx.viewer.staff && truthy(&u["upcoming_changes_stats"]) {
        out.push_str(&row(
            "upcoming-changes-info",
            &escape(&t(i18n, "admin.user.upcoming_changes.title")),
            &button(
                "btn-default",
                Some("eye"),
                Some(&t(i18n, "admin.user.upcoming_changes.view_modal")),
                None,
            ),
            Some("&nbsp;"),
        ));
    }
    out.push_str("</section>");
    out
}

/// The trust level's ComboBox, the lock and the TL3 requirements link.
fn trust_level_row(cx: &AdminCx, u: &Value) -> String {
    let i18n = cx.i18n;
    let level = u["trust_level"].as_i64().unwrap_or(0);
    let keys = ["newuser", "basic", "member", "regular", "leader"];
    let detailed = |id: i64| {
        let key = keys.get(id as usize).copied().unwrap_or("newuser");
        t_with(
            i18n,
            "trust_levels.detailed_name",
            &[
                ("level", &id.to_string()),
                ("name", &t(i18n, &format!("trust_levels.names.{key}"))),
            ],
        )
    };
    let name = detailed(level);
    let can_change = truthy(&u["can_change_trust_level"]);
    let filter_by = t_with(i18n, "select_kit.filter_by", &[("name", &name)]);
    let options: Vec<Value> = (0..5)
        .map(|id| serde_json::json!({ "id": id, "name": detailed(id) }))
        .collect();
    let value = format!(
        "<details class=\"select-kit single-select combobox combo-box has-selection change-trust-level-dropdown{}\" data-options=\"{}\"><summary aria-label=\"{f}\" name=\"{f}\" data-name=\"{n}\" data-value=\"{level}\" tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" data-value=\"{level}\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
        if can_change { "" } else { " is-disabled" },
        escape(&Value::Array(options).to_string()),
        icon("angle-down", Some("angle-icon")),
        f = escape(&filter_by),
        n = escape(&name)
    );
    let mut controls = String::new();
    if can_change {
        // canLockTrustLevel
        if level < 4 {
            if truthy(&u["manual_locked_trust_level"]) || u["manual_locked_trust_level"].is_number()
            {
                controls.push_str(&format!(
                    "<span class=\"svg-icon-title\" title=\"{}\">{}</span>",
                    escape(&t(i18n, "admin.user.trust_level_locked_tip")),
                    icon("lock", None)
                ));
                controls.push_str(&button(
                    "btn-default",
                    None,
                    Some(&t(i18n, "admin.user.unlock_trust_level")),
                    None,
                ));
            } else {
                controls.push_str(&format!(
                    "<span class=\"svg-icon-title\" title=\"{}\">{}</span>",
                    escape(&t(i18n, "admin.user.trust_level_unlocked_tip")),
                    icon("unlock", None)
                ));
                controls.push_str(&button(
                    "btn-default",
                    None,
                    Some(&t(i18n, "admin.user.lock_trust_level")),
                    None,
                ));
            }
        }
        if !u["tl3_requirements"].is_null() {
            controls.push_str(&format!(
                "<a class=\"btn btn-default\" href=\"{}/admin/users/{}/{}/tl3_requirements\">{}</a>",
                cx.base_path,
                u["id"].as_i64().unwrap_or(0),
                escape(&s(&u["username"]).to_lowercase()),
                escape(&t(i18n, "admin.user.trust_level_3_requirements"))
            ));
        }
    }
    row(
        "",
        &escape(&t(i18n, "trust_level")),
        &value,
        Some(&controls),
    )
}

/// Suspended? or Silenced?, and who did it and why while it lasts.
fn penalty_rows(
    cx: &AdminCx,
    list: &ListContext,
    page: &UserPage,
    u: &Value,
    staff: bool,
    kind: &str,
) -> String {
    let i18n = cx.i18n;
    let suspend = kind == "suspended";
    let till = parse(if suspend {
        &u["suspended_till"]
    } else {
        &u["silenced_till"]
    });
    let on = till.is_some_and(|t| t > page.now);
    // yes_no unescaped: the template is escaped as a whole below.
    let mut value = t(i18n, if on { "yes_value" } else { "no_value" });
    if let Some(till) = till.filter(|_| on)
        && !is_forever(page.now, till)
    {
        value.push(' ');
        value.push_str(&t_with(
            i18n,
            "admin.user.suspended_until",
            &[("until", &crate::viewer_dates::long_marker(list, till))],
        ));
    }
    // The end in the viewer's zone (crate::viewer_dates).
    let (value_tz, value) = crate::viewer_dates::text(list, &value);
    let (class, icon_name, label, explanation) = if suspend {
        (
            "suspend-user",
            "ban",
            "admin.user.suspend",
            "admin.user.suspended_explanation",
        )
    } else {
        (
            "silence-user",
            "microphone-slash",
            "admin.user.silence",
            "admin.user.silence_explanation",
        )
    };
    let mut controls = String::new();
    if on {
        let (class, label) = if suspend {
            ("unsuspend-user", "admin.user.unsuspend")
        } else {
            ("unsilence-user", "admin.user.unsilence")
        };
        controls.push_str(&button(
            &format!("btn-danger {class}"),
            Some(icon_name),
            Some(&t(i18n, label)),
            None,
        ));
        controls.push(' ');
        controls.push_str(&escape(&t(i18n, explanation)));
    } else if !staff {
        controls.push_str(&button(
            &format!("btn-danger {class}"),
            Some(icon_name),
            Some(&t(i18n, label)),
            None,
        ));
        controls.push(' ');
        controls.push_str(&escape(&t(i18n, explanation)));
    }
    let mut out = if suspend {
        format!(
            "<div class=\"user-suspended display-row {}\"><div class=\"field\">{}</div><div class=\"value\"{value_tz}>{value}</div><div class=\"controls\">{controls}</div></div>",
            if on { "highlight-danger" } else { "" },
            escape(&t(i18n, "admin.user.suspended"))
        )
    } else {
        // DConditionalLoadingSpinner around the controls.
        format!(
            "<div class=\"display-row {}\"><div class=\"field\">{}</div><div class=\"value\"{value_tz}>{value}</div><div class=\"controls\"><div class=\"loading-container inline-spinner\">{controls}</div></div></div>",
            if on { "highlight-danger" } else { "" },
            escape(&t(i18n, "admin.user.silenced"))
        )
    };
    if on {
        let by = if suspend {
            &u["suspended_by"]
        } else {
            &u["silenced_by"]
        };
        let reason = if suspend {
            &u["full_suspend_reason"]
        } else {
            &u["full_silence_reason"]
        };
        let (class, field, tag) = if suspend {
            ("suspension-info", "admin.user.suspended_by", "strong")
        } else {
            ("silence-info", "admin.user.silenced_by", "b")
        };
        out.push_str(&format!(
            "<div class=\"display-row highlight-danger {class}\"><div class=\"field\">{}</div><div class=\"value\">{}</div><div class=\"controls\"><{tag}>{}</{tag}>: <div class=\"full-reason\">{}</div></div></div>",
            escape(&t(i18n, field)),
            if by.is_object() { by_links(cx, by, 20) } else { String::new() },
            escape(&t(
                i18n,
                if suspend {
                    "admin.user.suspend_reason"
                } else {
                    "admin.user.silence_reason"
                }
            )),
            s(reason)
        ));
    }
    out
}

/// The Groups section: the automatic groups, the custom groups' chooser,
/// the primary group.
fn groups(cx: &AdminCx, u: &Value) -> String {
    let i18n = cx.i18n;
    let empty = Vec::new();
    let groups = u["groups"].as_array().unwrap_or(&empty);
    let automatic: Vec<String> = groups
        .iter()
        .filter(|g| truthy(&g["automatic"]))
        .map(|g| {
            let name = escape(s(&g["name"]));
            format!("<a href=\"{}/g/{name}\">{name}</a>", cx.base_path)
        })
        .collect();
    let custom: Vec<&Value> = groups.iter().filter(|g| !truthy(&g["automatic"])).collect();
    let mut out = format!(
        "<section class=\"details\"><h1>{}</h1>",
        escape(&t(i18n, "admin.groups.title"))
    );
    out.push_str(&row(
        "admin-user__automatic-groups",
        &escape(&t(i18n, "admin.groups.automatic")),
        &automatic.join(", "),
        None,
    ));
    let names: Vec<&str> = custom.iter().map(|g| s(&g["name"])).collect();
    let ids: Vec<String> = custom
        .iter()
        .map(|g| g["id"].as_i64().unwrap_or(0).to_string())
        .collect();
    let label = if names.is_empty() {
        t(i18n, "select_kit.default_header_text")
    } else {
        names.join(", ")
    };
    out.push_str(&row(
        "admin-user__custom-groups",
        &escape(&t(i18n, "admin.groups.custom")),
        &format!(
            "<details class=\"select-kit multi-select group-chooser{}\"><summary data-name=\"{}\" data-value=\"{}\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">{}</span>{}</div></summary><div class=\"select-kit-body\"></div></details>",
            if names.is_empty() { "" } else { " has-selection" },
            escape(&names.join(",")),
            ids.join(","),
            escape(&label),
            icon("plus", Some("angle-icon"))
        ),
        None,
    ));
    if !custom.is_empty() {
        let primary = u["primary_group_id"].as_i64();
        let selected = custom
            .iter()
            .find(|g| g["id"].as_i64() == primary)
            .map(|g| s(&g["name"]).to_string());
        let name = selected
            .clone()
            .unwrap_or_else(|| t(i18n, "admin.groups.no_primary"));
        let options: Vec<Value> = custom
            .iter()
            .map(|g| serde_json::json!({ "id": g["id"], "name": g["name"] }))
            .collect();
        let filter_by = t_with(i18n, "select_kit.filter_by", &[("name", &name)]);
        out.push_str(&row(
            "",
            &escape(&t(i18n, "admin.groups.primary")),
            &format!(
                "<details class=\"select-kit single-select combobox combo-box{} primary-group-dropdown\" data-options=\"{}\"><summary aria-label=\"{f}\" name=\"{f}\" data-name=\"{n}\"{} tabindex=\"0\" class=\"select-kit-header single-select-header combo-box-header\"><div class=\"select-kit-header-wrapper\"><div class=\"select-kit-selected-name selected-name choice\" data-name=\"{n}\" title=\"{n}\"><span class=\"name\">{n}</span></div>{}</div></summary><div class=\"select-kit-body\"></div></details>",
                if selected.is_some() { " has-selection" } else { "" },
                escape(&Value::Array(options).to_string()),
                primary
                    .filter(|_| selected.is_some())
                    .map(|p| format!(" data-value=\"{p}\""))
                    .unwrap_or_default(),
                icon("angle-down", Some("angle-icon")),
                f = escape(&filter_by),
                n = escape(&name)
            ),
            None,
        ));
    }
    out.push_str("</section>");
    out
}

/// The Activity section.
fn activity(cx: &AdminCx, list: &ListContext, u: &Value) -> String {
    let i18n = cx.i18n;
    let n = |k: &str| js_value(&u[k]);
    let mut out = format!(
        "<section class=\"details\"><h1>{}</h1>",
        escape(&t(i18n, "admin.user.activity"))
    );
    out.push_str(&row(
        "",
        &escape(&t(i18n, "created")),
        &format_date(list, &u["created_at"], true),
        None,
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.users.last_emailed")),
        &format_date(list, &u["last_emailed_at"], false),
        None,
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "last_seen")),
        &format_date(list, &u["last_seen_at"], true),
        None,
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.like_count")),
        &format!("{} / {}", n("like_given_count"), n("like_count")),
        None,
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.topics_entered")),
        &n("topics_entered"),
        None,
    ));
    let post_count = u["post_count"].as_i64().unwrap_or(0);
    let controls = if truthy(&u["can_delete_all_posts"]) {
        if post_count > 0 {
            button(
                "btn-danger",
                Some("trash-can"),
                Some(&t(i18n, "admin.user.delete_posts.button")),
                None,
            )
        } else {
            String::new()
        }
    } else {
        let max_posts = cx
            .settings
            .get("delete_all_posts_max")
            .map(|v| v.to_i())
            .unwrap_or(15);
        let explanation = if truthy(&u["admin"]) {
            t(i18n, "admin.user.delete_posts_forbidden_because_admin")
        } else if post_count > max_posts {
            t_count(
                i18n,
                "admin.user.cant_delete_all_too_many_posts",
                max_posts,
                &[],
            )
        } else {
            t_count(
                i18n,
                "admin.user.cant_delete_all_posts",
                cx.settings
                    .get("delete_user_max_post_age")
                    .map(|v| v.to_i())
                    .unwrap_or(60),
                &[],
            )
        };
        format!(
            "<span class=\"delete-all-posts-explanation\">{}</span>",
            escape(&explanation)
        )
    };
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.post_count")),
        &n("post_count"),
        Some(&controls),
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.posts_read_count")),
        &n("posts_read_count"),
        None,
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.warnings_received_count")),
        &n("warnings_received_count"),
        None,
    ));
    let flags_received = u["flags_received_count"].as_i64().unwrap_or(0);
    let controls = if flags_received > 0 {
        format!(
            "<a class=\"btn btn-default\" href=\"{}/review?status=all&amp;type=ReviewableFlaggedPost&amp;username={}\">{}</a>",
            cx.base_path,
            escape(s(&u["username"])),
            escape(&t(i18n, "admin.user.show_flags_received"))
        )
    } else {
        String::new()
    };
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.flags_given_received_count")),
        &format!("{} / {}", n("flags_given_count"), n("flags_received_count")),
        Some(&controls),
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.private_topics_count")),
        &n("private_topics_count"),
        None,
    ));
    let time_read = match u["time_read"].as_f64() {
        Some(seconds) => escape(&crate::user_profile_view::duration(
            list,
            seconds.round() as i64,
            "tiny",
        )),
        None => "&mdash;".into(),
    };
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.time_read")),
        &time_read,
        None,
    ));
    out.push_str(&row(
        "",
        &escape(&t(i18n, "user.invited.days_visited")),
        &n("days_visited"),
        None,
    ));
    let edits = u["post_edits_count"].as_i64().unwrap_or(0);
    let controls = if edits > 0 {
        let filters = format!("{{\"editor\":\"{}\"}}", s(&u["username"]));
        format!(
            "<a class=\"btn btn-icon btn-default\" href=\"{}/admin/reports/post_edits?filters={}\">{} {}</a>",
            cx.base_path,
            escape(&encode_component(&filters)),
            icon("far-eye", None),
            escape(&t(i18n, "admin.user.view_edits"))
        )
    } else {
        String::new()
    };
    out.push_str(&row(
        "post-edits-count",
        &escape(&t(i18n, "admin.user.post_edits_count")),
        &if edits > 0 {
            edits.to_string()
        } else {
            "0".into()
        },
        Some(&controls),
    ));
    out.push_str("</section>");
    out
}

/// `encodeURIComponent`
fn encode_component(s: &str) -> String {
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

/// A value as a template prints it: empty for null.
fn js_value(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Number(n) => match n.as_f64() {
            Some(f) => js_number(f),
            None => n.to_string(),
        },
        Value::String(s) => escape(s),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// The DiscourseConnect section.
fn sso(cx: &AdminCx, u: &Value) -> String {
    let i18n = cx.i18n;
    let record = &u["single_sign_on_record"];
    let mut out = format!(
        "<section class=\"details\"><h1>{}</h1>",
        escape(&t(i18n, "admin.user.discourse_connect.title"))
    );
    let delete = if truthy(&u["can_delete_sso_record"]) {
        Some(button(
            "btn-danger",
            Some("trash-can"),
            Some(&t(i18n, "admin.user.discourse_connect.delete_sso_record")),
            None,
        ))
    } else {
        None
    };
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.discourse_connect.external_id")),
        &js_value(&record["external_id"]),
        delete.as_deref(),
    ));
    for key in ["external_username", "external_name"] {
        out.push_str(&row(
            "",
            &escape(&t(i18n, &format!("admin.user.discourse_connect.{key}"))),
            &js_value(&record[key]),
            None,
        ));
    }
    let check = |icon_name: &str, label: &str, title: &str| {
        button(
            "btn-default",
            Some(icon_name),
            Some(&t(i18n, label)),
            Some(&t(i18n, title)),
        )
    };
    if cx.viewer.admin {
        out.push_str(&format!(
            "<div class=\"display-row\"><div class=\"field\">{}</div>{}</div>",
            escape(&t(i18n, "admin.user.discourse_connect.external_email")),
            check(
                "envelope",
                "admin.users.check_email.text",
                "admin.users.check_email.title"
            )
        ));
    }
    out.push_str(&row(
        "",
        &escape(&t(i18n, "admin.user.discourse_connect.external_avatar_url")),
        &js_value(&record["external_avatar_url"]),
        None,
    ));
    if cx.viewer.admin {
        out.push_str(&format!(
            "<div class=\"display-row\"><div class=\"field\">{}</div>{}</div>",
            escape(&t(i18n, "admin.user.discourse_connect.last_payload")),
            check(
                "far-rectangle-list",
                "admin.users.check_sso.text",
                "admin.users.check_sso.title"
            )
        ));
    }
    out.push_str("</section>");
    out
}

/// The danger controls: impersonate, anonymize, delete, merge, and why
/// the user can't be deleted.
fn danger(cx: &AdminCx, u: &Value, active: bool, admin: bool) -> String {
    let i18n = cx.i18n;
    let mut out = String::from("<section><hr><div class=\"admin-user__danger-controls\">");
    if active && truthy(&u["can_impersonate"]) {
        out.push_str(&button(
            "btn-danger btn-impersonate",
            Some("crosshairs"),
            Some(&t(i18n, "admin.impersonate.title")),
            Some(&t(i18n, "admin.impersonate.help")),
        ));
    }
    if truthy(&u["can_be_anonymized"]) {
        out.push_str(&button(
            "btn-danger btn-anonymize",
            Some("triangle-exclamation"),
            Some(&t(i18n, "admin.user.anonymize")),
            None,
        ));
    }
    let max_posts = cx
        .settings
        .get("delete_all_posts_max")
        .map(|v| v.to_i())
        .unwrap_or(15);
    let can_be_deleted =
        truthy(&u["can_be_deleted"]) && u["post_count"].as_i64().unwrap_or(0) <= max_posts;
    if can_be_deleted {
        let label = t(i18n, "admin.user.delete");
        let filter_by = t_with(i18n, "select_kit.filter_by", &[("name", &label)]);
        out.push_str(&format!(
            "<details class=\"select-kit single-select dropdown-select-box btn-user-delete\"><summary aria-label=\"{f}\" name=\"{f}\" data-name=\"{l}\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header btn btn-icon-text btn-danger\"><div class=\"select-kit-header-wrapper\">{} <div class=\"select-kit-selected-name selected-name choice\" data-name=\"{l}\" title=\"{l}\"><span class=\"name\">{l}</span></div>{} &#8203;</div></summary><div class=\"select-kit-body\"></div></details>",
            icon("trash-can", None),
            icon("angle-down", Some("angle-icon")),
            f = escape(&filter_by),
            l = escape(&label)
        ));
    }
    if truthy(&u["can_be_merged"]) {
        out.push_str(&button(
            "btn-danger btn-user-merge",
            Some("left-right"),
            Some(&t(i18n, "admin.user.merge.button")),
            None,
        ));
    }
    out.push_str("</div>");
    if !can_be_deleted {
        let explanation = if admin {
            t(i18n, "admin.user.delete_forbidden_because_admin")
        } else {
            t_count(
                i18n,
                "admin.user.delete_forbidden",
                cx.settings
                    .get("delete_user_max_post_age")
                    .map(|v| v.to_i())
                    .unwrap_or(60),
                &[],
            )
        };
        out.push_str(&format!(
            "<div class=\"pull-right\">{} <span class=\"delete-explanation\">{}</span></div>",
            icon("triangle-exclamation", None),
            escape(&explanation)
        ));
    }
    out.push_str("</section>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_penalty_a_century_away_is_forever() {
        let now = DateTime::parse_from_rfc3339("2026-10-14T00:00:00Z")
            .unwrap()
            .to_utc();
        let far = DateTime::parse_from_rfc3339("3026-10-14T00:00:00Z")
            .unwrap()
            .to_utc();
        let soon = DateTime::parse_from_rfc3339("2027-10-14T00:00:00Z")
            .unwrap()
            .to_utc();
        assert!(is_forever(now, far));
        assert!(!is_forever(now, soon));
    }
}
