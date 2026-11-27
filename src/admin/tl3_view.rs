//! The admin's TL3 requirements page for one user as the Ember client
//! renders it (templates/admin-user/tl3-requirements.gjs over the
//! Tl3Requirements model's getters) from the `tl3_requirements` of
//! Admin::UsersController#show.

use serde_json::Value;

use crate::admin::view::{AdminCx, t, t_count};
use crate::topic_list_view::{escape, icon};

/// A number as Handlebars prints it; nothing for a missing one.
fn n(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => escape(s),
        _ => String::new(),
    }
}

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

/// checkIcon
fn check(met: bool) -> String {
    icon(if met { "check" } else { "xmark" }, None)
}

/// `Math.round(a * 100 / b)`
fn percent(a: i64, b: i64) -> String {
    if b == 0 {
        return "NaN".into();
    }
    ((a as f64 * 100.0) / b as f64 + 0.5).floor().to_string()
}

pub fn render(cx: &AdminCx, u: &Value) -> String {
    let i18n = cx.i18n;
    let r = &u["tl3_requirements"];
    let username = escape(u["username"].as_str().unwrap_or_default());
    let ge = |a: &str, b: &str| r.get(a).is_some() && int(&r[a]) >= int(&r[b]);
    let le = |a: &str, b: &str| r.get(a).is_some() && int(&r[a]) <= int(&r[b]);
    let row = |label: &str, met: bool, value: String, requirement: String| {
        format!(
            "<tr><th>{}</th><td>{}</td><td>{value}</td><td>{requirement}</td></tr>",
            escape(&t(i18n, &format!("admin.user.tl3_requirements.{label}"))),
            check(met)
        )
    };
    let max_of = |key: &str| {
        if r.get(key).is_some() {
            escape(&t_count(i18n, "max_of_count", int(&r[key]), &[]))
        } else {
            String::new()
        }
    };
    let penalty =
        |key: &str| r.get("penalty_counts").is_some() && int(&r["penalty_counts"][key]) == 0;
    let time_period = int(&r["time_period"]);
    let mut rows = String::new();
    rows.push_str(&row(
        "topics_replied_to",
        ge("num_topics_replied_to", "min_topics_replied_to"),
        format!(
            "{}{}",
            if int(&r["num_topics_replied_to"]) > int(&r["min_topics_replied_to"]) {
                "&ge; "
            } else {
                ""
            },
            n(&r["num_topics_replied_to"])
        ),
        n(&r["min_topics_replied_to"]),
    ));
    for (label, value, min) in [
        ("topics_viewed", "topics_viewed", "min_topics_viewed"),
        (
            "topics_viewed_all_time",
            "topics_viewed_all_time",
            "min_topics_viewed_all_time",
        ),
        ("posts_read", "posts_read", "min_posts_read"),
    ] {
        rows.push_str(&row(label, ge(value, min), n(&r[value]), n(&r[min])));
    }
    rows.push_str(&row(
        "posts_read_days",
        ge("days_visited", "min_days_visited"),
        format!(
            "{}% ({} / {} {})",
            percent(int(&r["days_visited"]), time_period),
            n(&r["days_visited"]),
            n(&r["time_period"]),
            escape(&t(i18n, "admin.user.tl3_requirements.days"))
        ),
        format!("{}%", percent(int(&r["min_days_visited"]), time_period)),
    ));
    rows.push_str(&row(
        "posts_read_all_time",
        ge("posts_read_all_time", "min_posts_read_all_time"),
        n(&r["posts_read_all_time"]),
        n(&r["min_posts_read_all_time"]),
    ));
    rows.push_str(&row(
        "flagged_posts",
        le("num_flagged_posts", "max_flagged_posts"),
        n(&r["num_flagged_posts"]),
        max_of("max_flagged_posts"),
    ));
    rows.push_str(&row(
        "flagged_by_users",
        le("num_flagged_by_users", "max_flagged_by_users"),
        n(&r["num_flagged_by_users"]),
        max_of("max_flagged_by_users"),
    ));
    for (label, value, min) in [
        ("likes_given", "num_likes_given", "min_likes_given"),
        ("likes_received", "num_likes_received", "min_likes_received"),
        (
            "likes_received_days",
            "num_likes_received_days",
            "min_likes_received_days",
        ),
        (
            "likes_received_users",
            "num_likes_received_users",
            "min_likes_received_users",
        ),
    ] {
        rows.push_str(&row(label, ge(value, min), n(&r[value]), n(&r[min])));
    }
    for key in ["silenced", "suspended"] {
        rows.push_str(&row(
            key,
            penalty(key),
            n(&r["penalty_counts"][key]),
            "0".into(),
        ));
    }
    // The template branches on `model.istl3`, which nothing defines, so it
    // always reads as a user below TL3.
    let (status_icon, status) = if r["requirements_met"] == true {
        ("check", vec!["qualifies", "will_be_promoted"])
    } else if r["trust_level_locked"] == true {
        ("lock", vec!["locked_will_not_be_promoted"])
    } else {
        ("xmark", vec!["does_not_qualify"])
    };
    let status: Vec<String> = status
        .iter()
        .map(|k| escape(&t(i18n, &format!("admin.user.tl3_requirements.{k}"))))
        .collect();
    let user_href = format!(
        "{}/admin/users/{}/{}",
        cx.base_path,
        int(&u["id"]),
        escape(&u["username"].as_str().unwrap_or_default().to_lowercase())
    );
    format!(
        "<div class=\"admin-controls\"><nav><ul class=\"nav nav-pills\">\
<li><a href=\"{user_href}\">{} &nbsp;{username}</a></li>\
<li><a href=\"{}/admin/users/list/member\">{}</a></li></ul></nav></div>\
<div class=\"admin-container tl3-requirements\"><h2>{username} - {}</h2><br>\
<p>{}</p><table class=\"table\" style=\"width: auto;\"><thead><tr><th></th><th></th><th>{}</th><th>{}</th></tr></thead>\
<tbody>{rows}</tbody></table><br><p>{} {}</p></div>",
        icon("angle-left", None),
        cx.base_path,
        escape(&t(i18n, "admin.user.trust_level_2_users")),
        escape(&t(i18n, "admin.user.tl3_requirements.title")),
        escape(&t_count(
            i18n,
            "admin.user.tl3_requirements.table_title",
            time_period,
            &[]
        )),
        escape(&t(i18n, "admin.user.tl3_requirements.value_heading")),
        escape(&t(i18n, "admin.user.tl3_requirements.requirement_heading")),
        icon(status_icon, None),
        status.join(" "),
    )
}

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn percent_rounds_half_up() {
        assert_eq!(percent(1, 2), "50");
        assert_eq!(percent(1, 3), "33");
        assert_eq!(percent(2, 3), "67");
        assert_eq!(percent(1, 200), "1");
        assert_eq!(percent(1, 0), "NaN");
    }
}
