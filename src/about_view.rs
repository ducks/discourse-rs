//! The about page as the Ember app renders it (templates/about.gjs with
//! components/about-page.gjs and about-page-users.gjs), from the
//! about#index document (crate::about::document) the route loads.
//!
//! Not drawn: the extra groups (about_page_extra_groups loads each group
//! and its members over routes not ported) and the EU visitor stats
//! (refused by the document).

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

use crate::topic_list_view::{ListContext, escape, icon, t, t_count};
use crate::{AppError, Unsupported};

/// What the page reads besides the document.
pub struct AboutPage<'a> {
    pub doc: &'a Value,
    /// `currentUser.admin`: the Edit this page link.
    pub viewer_admin: bool,
    /// The site settings the navigation reads.
    pub faq_url: &'a str,
    pub rename_faq_to_guidelines: bool,
    /// `site.tos_url` and `site.privacy_policy_url`.
    pub tos_url: Option<&'a str>,
    pub privacy_policy_url: Option<&'a str>,
    /// chat's addAboutPageActivity, registered while chat is enabled.
    pub chat_enabled: bool,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn int(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

/// `I18n.toNumber(n, { precision: 0 })`: grouped by thousands.
fn to_number(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// The about route's model: the users the ids name, admins and moderators
/// seen in the last year (`moment(last_seen_at) > yearAgo`; never seen
/// is left out).
fn staff<'a>(users: &'a [Value], ids: &Value, now: DateTime<Utc>) -> Vec<&'a Value> {
    let year_ago = now - Duration::days(365);
    ids.as_array()
        .into_iter()
        .flatten()
        .filter_map(|id| users.iter().find(|u| u["id"] == *id))
        .filter(|u| {
            u["last_seen_at"]
                .as_str()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at.with_timezone(&Utc) > year_ago)
        })
        .collect()
}

/// AboutPageUsers with `@truncateAt={{6}}`: the first six, and the View
/// more button that shows the rest (templates/about.html's script).
fn users_list(cx: &ListContext, users: &[&Value]) -> String {
    let mut out = String::from("<div class=\"about-page-users-list\">");
    for (i, u) in users.iter().enumerate() {
        let info = crate::user_info_view::user_info(cx, u, None);
        if i >= 6 {
            // Hidden until expanded.
            out.push_str(&info.replacen(
                "<div class=\"user-info small\"",
                "<div hidden class=\"user-info small\"",
                1,
            ));
        } else {
            out.push_str(&info);
        }
    }
    out.push_str("</div>");
    if users.len() > 6 {
        out.push_str(&format!(
            "<button class=\"btn btn-icon-text btn-flat about-page-users-list__expand-button\" type=\"button\" data-view-more=\"{}\" data-view-less=\"{}\">{}<span class=\"d-button-label\">{}</span></button>",
            escape(&t(cx, "about.view_more")),
            escape(&t(cx, "about.view_less")),
            icon("chevron-down", None),
            escape(&t(cx, "about.view_more")),
        ));
    }
    out
}

/// `AboutPage#siteAgeString`: months of 30 days, rounded.
fn site_age(cx: &ListContext, created: Option<DateTime<Utc>>) -> String {
    // `new Date(null)` is the epoch.
    let created = created.unwrap_or(DateTime::UNIX_EPOCH);
    let months = (cx.now - created).num_milliseconds() as f64 / (1000.0 * 3600.0 * 24.0 * 30.0);
    if months < 1.0 {
        t(cx, "about.site_age.less_than_one_month")
    } else if months < 12.0 {
        let n = months.round() as i64;
        t_count(cx, "about.site_age.month", n, &[("count", &n.to_string())])
    } else {
        let n = (months / 12.0).round() as i64;
        t_count(cx, "about.site_age.year", n, &[("count", &n.to_string())])
    }
}

/// `AboutPage#contactInfo`: the contact URL, else the email, as a link.
fn contact_info(cx: &ListContext, about: &Value) -> Option<String> {
    let url = escape(s(&about["contact_url"]));
    let email = escape(s(&about["contact_email"]));
    let link = if !url.is_empty() {
        // contactURLHref
        let scheme = url.split_once(':').is_some_and(|(p, _)| {
            !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        let href = if url.starts_with('/') || scheme {
            url.clone()
        } else {
            format!("//{url}")
        };
        format!("<a href='{href}' target='_blank'>{url}</a>")
    } else if !email.is_empty() {
        format!("<a href=\"mailto:{email}\">{email}</a>")
    } else {
        return None;
    };
    Some(
        cx.i18n
            .t_with("js.about.contact_info", &[("contact_info", &link)])
            .unwrap_or_default(),
    )
}

/// The `#main-outlet` content of /about.
pub fn render(cx: &ListContext, page: &AboutPage) -> Result<String, AppError> {
    let about = &page.doc["about"];
    let users = page.doc["users"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let stats = &about["stats"];
    if !s(&about["banner_image"]).is_empty() {
        return Err(Unsupported("the about page's banner image").into());
    }
    let admins = staff(users, &about["admin_ids"], cx.now);
    let moderators = staff(users, &about["moderator_ids"], cx.now);
    let base = cx.base_path;

    // The navigation pills.
    let pill = |class: &str, href: &str, label: &str, active: bool| {
        format!(
            "<li class=\"{class}\"><a{} href=\"{href}\">{}</a></li>",
            if active { " class=\"active\"" } else { "" },
            escape(label)
        )
    };
    let mut nav = pill(
        "nav-item-about",
        &format!("{base}/about"),
        &t(cx, "about.simple_title"),
        true,
    );
    let guidelines = || {
        pill(
            "nav-item-guidelines",
            &format!("{base}/guidelines"),
            &t(cx, "guidelines"),
            false,
        )
    };
    let faq = || pill("nav-item-faq", &format!("{base}/faq"), &t(cx, "faq"), false);
    if !page.faq_url.is_empty() {
        nav.push_str(&guidelines());
        nav.push_str(&faq());
    } else if page.rename_faq_to_guidelines {
        nav.push_str(&guidelines());
    } else {
        nav.push_str(&faq());
    }
    if page.tos_url.is_some() {
        nav.push_str(&pill(
            "nav-item-tos",
            &format!("{base}/tos"),
            &t(cx, "tos"),
            false,
        ));
    }
    if page.privacy_policy_url.is_some() {
        nav.push_str(&pill(
            "nav-item-privacy",
            &format!("{base}/privacy"),
            &t(cx, "privacy"),
            false,
        ));
    }

    let mut out = format!(
        "<section><div class=\"container\"><div class=\"contents clearfix body-page\"><ul class=\"nav-pills\">{nav}</ul>"
    );
    if page.viewer_admin {
        out.push_str(&format!(
            "<p><a class=\"edit-about-page\" href=\"{base}/admin/config/about\">{} <span>{}</span></a></p>",
            icon("pencil", None),
            escape(&t(cx, "about.edit"))
        ));
    }
    out.push_str(&format!(
        "<section class=\"about__header\"><h1>{}</h1><p class=\"short-description\">{}</p></section>",
        escape(s(&about["title"])),
        escape(s(&about["description"]))
    ));

    // The stats line.
    let count_text =
        |key: &str, n: i64| t_count(cx, key, n, &[("formatted_number", &to_number(n))]);
    let created = about["site_creation_date"]
        .as_str()
        .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.with_timezone(&Utc));
    let stat_items = [
        (
            "members",
            "users",
            true,
            count_text("about.member_count", int(&stats["users_count"])),
        ),
        (
            "admins",
            "shield-halved",
            !admins.is_empty(),
            count_text("about.admin_count", admins.len() as i64),
        ),
        (
            "moderators",
            "shield-halved",
            !moderators.is_empty(),
            count_text("about.moderator_count", moderators.len() as i64),
        ),
        (
            "site-creation-date",
            "calendar-days",
            true,
            site_age(cx, created),
        ),
    ];
    out.push_str("<div class=\"about__main-content\"><div class=\"about__left-side\"><div class=\"about__stats\">");
    for (class, name, display, text) in stat_items {
        if display {
            out.push_str(&format!(
                "<span class=\"about__stats-item {class}\">{} <span>{}</span></span> ",
                icon(name, None),
                escape(&text)
            ));
        }
    }
    out.push_str("</div>");
    let extended = s(&about["extended_site_description"]);
    if !extended.is_empty() {
        out.push_str(&format!(
            "<h2>{}</h2><div>{extended}</div>",
            escape(&t(cx, "about.simple_title"))
        ));
    }
    if !admins.is_empty() {
        out.push_str(&format!(
            "<section class=\"about__admins\"><h2>{}</h2>{}</section>",
            escape(&t(cx, "about.our_admins")),
            users_list(cx, &admins)
        ));
    }
    if !moderators.is_empty() {
        out.push_str(&format!(
            "<section class=\"about__moderators\"><h2>{}</h2>{}</section>",
            escape(&t(cx, "about.our_moderators")),
            users_list(cx, &moderators)
        ));
    }
    out.push_str("</div>");

    // The right side: contact and site activity.
    out.push_str(&format!(
        "<div class=\"about__right-side\"><h2>{}</h2>",
        escape(&t(cx, "about.contact"))
    ));
    if let Some(info) = contact_info(cx, about) {
        out.push_str(&format!("<p class=\"about__contact-info\">{info}</p>"));
    }
    out.push_str(&format!(
        "<p>{}</p><h2>{}</h2><div class=\"about__activities\">",
        escape(&t(cx, "about.report_inappropriate_content")),
        escape(&t(cx, "about.site_activity"))
    ));
    let number = |n: i64| crate::topic_list_view::number(cx, n);
    let activity = |key: &str, n: i64| {
        t_count(
            cx,
            &format!("about.activities.{key}"),
            n,
            &[("formatted_number", &number(n))],
        )
    };
    let last_7_days = t(cx, "about.activities.periods.last_7_days");
    let mut activities = vec![
        (
            "topics",
            "scroll",
            activity("topics", int(&stats["topics_7_days"])),
            last_7_days.clone(),
        ),
        (
            "posts",
            "pencil",
            activity("posts", int(&stats["posts_last_day"])),
            t(cx, "about.activities.periods.today"),
        ),
        (
            "active-users",
            "user-group",
            activity("active_users", int(&stats["active_users_7_days"])),
            last_7_days.clone(),
        ),
        (
            "sign-ups",
            "user-plus",
            activity("sign_ups", int(&stats["users_7_days"])),
            last_7_days.clone(),
        ),
        (
            "likes",
            "heart",
            activity("likes", int(&stats["likes_count"])),
            t(cx, "about.activities.periods.all_time"),
        ),
    ];
    // siteActivitiesFromPlugins: chat's messages over the last 7 days,
    // when there are any.
    let chat_messages = int(&stats["chat_messages_7_days"]);
    if page.chat_enabled && stats.get("chat_messages_7_days").is_some() && chat_messages != 0 {
        activities.push((
            "chat-messages",
            "comment-dots",
            activity("chat_messages", chat_messages),
            last_7_days,
        ));
    }
    for (class, name, text, period) in activities {
        out.push_str(&format!(
            "<div class=\"about__activities-item {class}\"><span class=\"about__activities-item-icon\">{}</span> \
             <span class=\"about__activities-item-type\"><div class=\"about__activities-item-count\">{}</div>\
             <div class=\"about__activities-item-period\">{}</div></span></div>",
            icon(name, None),
            escape(&text),
            escape(&period)
        ));
    }
    out.push_str("</div></div></div></div></div></section>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_number_groups_thousands() {
        assert_eq!(to_number(5), "5");
        assert_eq!(to_number(1234), "1,234");
        assert_eq!(to_number(1234567), "1,234,567");
        assert_eq!(to_number(-1000), "-1,000");
    }
}
