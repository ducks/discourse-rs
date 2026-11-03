//! The badges pages as Ember draws them from the JSON their routes load:
//! the badges index (templates/badges/index.gjs), a badge's page
//! (templates/badges/show.gjs with its grants) and a user's badges tab
//! (templates/user/badges.gjs), with the DBadgeCard they share.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;

use crate::topic_list_view::{ListContext, escape, icon, t, t_count, t_with};
use crate::{AppError, Unsupported};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `dNumber(n)`
fn d_number(n: i64) -> Result<String, AppError> {
    if n.abs() >= 1000 {
        return Err(
            Unsupported("badge counts of a thousand or more (dNumber's short form)").into(),
        );
    }
    Ok(format!("<span class=\"number\">{n}</span>"))
}

/// The client's `sanitize` and `emojiUnescape` of a badge's text: the
/// identity for what the port draws (the locale's texts and sanitized
/// custom ones), refused where an emoji code would be replaced.
fn summary_html(text: &str, emoji: bool) -> Result<String, AppError> {
    if emoji {
        let has_code = text
            .split(':')
            .skip(1)
            .zip(text.split(':').skip(2))
            .any(|(code, _)| {
                !code.is_empty()
                    && code
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
            });
        if has_code {
            return Err(Unsupported("emoji in a badge's long description").into());
        }
    }
    Ok(text.to_string())
}

/// What a DBadgeCard is drawn with.
#[derive(Default)]
pub struct Card<'a> {
    /// "large" draws the name without a link and the long description.
    pub large: bool,
    /// `@count`: None draws the badge's grant count.
    pub count: Option<i64>,
    pub granted: bool,
    /// `@filterUser` with `@username`: the link carries the username.
    pub filter_user: Option<&'a str>,
    /// `@canFavorite`, `@isFavorite`, `@canFavoriteMoreBadges`
    pub can_favorite: bool,
    pub is_favorite: bool,
    pub can_favorite_more: bool,
}

/// `badgeTypeClassName` from the badge's type.
fn type_class(type_name: &str) -> String {
    format!("badge-type-{}", type_name.to_lowercase())
}

/// DBadgeCard (ui-kit/d-badge-card.gjs). `type_name` is its badge type's.
pub fn badge_card(
    cx: &ListContext,
    b: &Value,
    type_name: &str,
    card: &Card<'_>,
) -> Result<String, AppError> {
    let slug = escape(s(&b["slug"]));
    let image = match b["image_url"].as_str().filter(|i| !i.is_empty()) {
        Some(url) => format!("<img src='{}' alt=''>", escape(url)),
        None => match b["icon"].as_str().filter(|i| !i.is_empty()) {
            Some(i) => icon(i, None),
            None => String::new(),
        },
    };
    let display = match card.count {
        None => b["grant_count"].as_i64().filter(|c| *c != 0),
        Some(c) if c > 1 => Some(c),
        Some(_) => None,
    };
    let url = format!("{}/badges/{}/{slug}", cx.base_path, b["id"]);
    let url = match card.filter_user {
        Some(u) => format!("{url}?username={}", escape(u)),
        None => url,
    };
    let mut described = format!("badge-summary-{slug}");
    if display.is_some() {
        described.push_str(&format!(" badge-granted-{slug}"));
    }
    if card.granted {
        described.push_str(&format!(" badge-awarded-{slug}"));
    }
    let name = escape(s(&b["name"]));
    let long = s(&b["long_description"]);
    let summary = if card.large && !long.is_empty() {
        summary_html(long, true)?
    } else {
        summary_html(s(&b["description"]), false)?
    };
    let title = if card.large {
        name
    } else {
        format!(
            "<a aria-describedby=\"{described}\" class=\"badge-link\" href=\"{url}\">{name}</a>"
        )
    };
    let granted_count = match display {
        Some(c) => format!(
            "<div class=\"badge-granted\" id=\"badge-granted-{slug}\">{}</div>",
            t_count(cx, "badges.awarded", c, &[("number", &d_number(c)?)])
        ),
        None => String::new(),
    };
    let mut out = format!(
        "<div class=\"badge-card --badge-{}{}\" data-badge-slug=\"{slug}\"><div class=\"badge-contents\">\
         <span aria-hidden=\"true\" class=\"badge-icon {}\">{image}</span>\
         <div class=\"badge-info\"><div class=\"badge-info-item\"><h3>{title}</h3>\
         <div class=\"badge-summary\" id=\"badge-summary-{slug}\">{summary}</div>{granted_count}</div></div></div>",
        if card.large { "large" } else { "medium" },
        if card.can_favorite {
            " --can-favorite"
        } else {
            ""
        },
        type_class(type_name),
    );
    if card.granted {
        out.push_str(&format!(
            "<div aria-label=\"{}\" class=\"check-display status-checked\" id=\"badge-awarded-{slug}\">{}</div>",
            escape(&t(cx, "notifications.titles.granted_badge")),
            icon("check", None)
        ));
    }
    if card.can_favorite {
        let (icon_name, disabled, title) = if card.is_favorite {
            ("star", false, None)
        } else if card.can_favorite_more {
            ("far-star", false, Some("badges.favorite_max_not_reached"))
        } else {
            ("far-star", true, Some("badges.favorite_max_reached"))
        };
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-default favorite-btn btn-small\"{}{} type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button>",
            if disabled { " disabled" } else { "" },
            title
                .map(|k| format!(" title=\"{}\"", escape(&t(cx, k))))
                .unwrap_or_default(),
            icon(icon_name, None)
        ));
    }
    out.push_str("</div>");
    Ok(out)
}

/// The client's `slugify`: lowercased, accents and anything but letters,
/// digits, spaces and dashes dropped, runs of spaces and dashes as one
/// dash.
fn slugify(text: &str) -> String {
    let lower = text.trim().to_lowercase();
    let mut kept = String::new();
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || c == '-' {
            kept.push(c);
        } else if c.is_whitespace() {
            kept.push(' ');
        }
    }
    let mut out = String::new();
    for c in kept.chars() {
        let c = if c == ' ' { '-' } else { c };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    out
}

/// `badgeGroupingDisplayName(name)`
fn grouping_display_name(cx: &ListContext, name: &str) -> String {
    let key: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .collect();
    cx.i18n
        .t(&format!("js.badges.badge_grouping.{key}.name"))
        .unwrap_or(name)
        .to_string()
}

/// `localeCompare` for the badge keys: case-insensitively, then by case.
fn locale_key(text: &str) -> (String, String) {
    (text.to_lowercase(), text.to_string())
}

/// The badges index (`/badges`): the badges JSON's badges, sorted by the
/// controller's key (grouping position, type, name) and grouped.
pub fn render_index(cx: &ListContext, doc: &Value) -> Result<String, AppError> {
    let empty = Vec::new();
    let badges = doc["badges"].as_array().unwrap_or(&empty);
    let groupings = doc["badge_groupings"].as_array().unwrap_or(&empty);
    let types = doc["badge_types"].as_array().unwrap_or(&empty);
    let grouping = |id: &Value| groupings.iter().find(|g| g["id"] == *id);
    let type_name = |id: &Value| {
        types
            .iter()
            .find(|t| t["id"] == *id)
            .map(|t| s(&t["name"]).to_string())
            .unwrap_or_default()
    };
    // ("000" + pos).slice(-4) + (10 - type) + name
    let key = |b: &Value| {
        let pos = grouping(&b["badge_grouping_id"])
            .map(|g| g["position"].to_string())
            .unwrap_or_else(|| "undefined".into());
        let padded = format!("000{pos}");
        let padded = &padded[padded.len().saturating_sub(4)..];
        let kind = 10 - b["badge_type_id"].as_i64().unwrap_or(0);
        locale_key(&format!("{padded}{kind}{}", s(&b["name"])))
    };
    let mut sorted: Vec<&Value> = badges.iter().collect();
    sorted.sort_by_key(|b| key(b));
    let mut out = format!(
        "<section><div class=\"container badges\"><h1>{}</h1><span></span><div class=\"badge-groups\">",
        escape(&t(cx, "badges.title"))
    );
    let mut i = 0;
    while i < sorted.len() {
        let group_id = &sorted[i]["badge_grouping_id"];
        let mut j = i;
        while j < sorted.len() && sorted[j]["badge_grouping_id"] == *group_id {
            j += 1;
        }
        let name = grouping(group_id)
            .map(|g| grouping_display_name(cx, s(&g["name"])))
            .unwrap_or_default();
        out.push_str(&format!(
            "<div class=\"badge-grouping\"><div class=\"title\"><h2 id=\"{}\">{}</h2></div><div class=\"badge-group-list\">",
            escape(&slugify(&name)),
            escape(&name)
        ));
        for b in &sorted[i..j] {
            out.push_str(&badge_card(
                cx,
                b,
                &type_name(&b["badge_type_id"]),
                &Card {
                    granted: b["has_badge"] == true,
                    ..Default::default()
                },
            )?);
        }
        out.push_str("</div></div>");
        i = j;
    }
    out.push_str("</div></div></section>");
    Ok(out)
}

/// What a badge's page is drawn from.
pub struct Show<'a> {
    /// The badge JSON (`/badges/:id.json`).
    pub badge: &'a Value,
    /// Its grants (`/user_badges.json?badge_id=` with the username).
    pub grants: &'a Value,
    /// `?username=`
    pub username: Option<&'a str>,
    pub enable_badges: bool,
}

/// DUserInfo at the medium size with the `badge-info` class, its details
/// yielded.
fn user_info(cx: &ListContext, u: &Value, details: &str) -> String {
    let username = s(&u["username"]);
    let path = format!("{}/u/{}", cx.base_path, escape(&username.to_lowercase()));
    let name_first =
        cx.settings.prioritize_name && u["name"].as_str().is_some_and(|n| !n.trim().is_empty());
    let name = match u["name"].as_str().filter(|n| !n.is_empty()) {
        Some(n) => format!(
            "<span class=\"name-wrapper\"><a data-user-card=\"{}\" href=\"{path}\"><span class=\"name\">{}</span></a></span>",
            escape(username),
            escape(n)
        ),
        None => String::new(),
    };
    let template = s(&u["avatar_template"]);
    let avatar = format!(
        "<img alt=\"\" width=\"48\" height=\"48\" src=\"{}{}\" class=\"avatar\" title=\"{}\">",
        if template.starts_with('/') && !template.starts_with("//") {
            cx.base_path
        } else {
            ""
        },
        escape(&template.replace("{size}", "48")),
        escape(username)
    );
    format!(
        "<div class=\"user-info medium badge-info\" data-username=\"{0}\"><div class=\"user-image\"><div class=\"user-image-inner\"><a aria-hidden=\"true\" data-user-card=\"{0}\" href=\"{path}\">{avatar}</a></div></div>\
         <div class=\"user-detail\"><div class=\"name-line{1}\"><span class=\"username-wrapper\"><a data-user-card=\"{0}\" href=\"{path}\"><span class=\"username\">{0}</span></a></span>{name}</div>\
         <div class=\"title\">{2}</div><div class=\"details\">{details}</div></div></div>",
        escape(username),
        if name_first { " --name-first" } else { "" },
        escape(s(&u["title"]))
    )
}

fn parse_time(v: &Value) -> Option<DateTime<Utc>> {
    let text = v.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(text.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|t| t.and_utc())
        })
}

/// A badge's page (`/badges/:id/:slug`). The second value is whether the
/// footer hides (more grants to load).
pub fn render_show(cx: &ListContext, v: &Show<'_>) -> Result<(String, bool), AppError> {
    let empty = Vec::new();
    let b = &v.badge["badge"];
    let types = v.badge["badge_types"].as_array().unwrap_or(&empty);
    let type_name = types
        .iter()
        .find(|t| t["id"] == b["badge_type_id"])
        .map(|t| s(&t["name"]).to_string())
        .unwrap_or_default();
    let info = &v.grants["user_badge_info"];
    let grants = info["user_badges"].as_array().unwrap_or(&empty);
    let users = v.grants["users"].as_array().unwrap_or(&empty);
    let topics = v.grants["topics"].as_array().unwrap_or(&empty);
    let user_grant_count = info["grant_count"].as_i64();
    let model_count = b["grant_count"].as_i64().unwrap_or(0);
    let grant_count = if v.username.is_some() {
        user_grant_count.unwrap_or(0)
    } else {
        model_count
    };
    let can_load_more = grant_count > grants.len() as i64;
    if can_load_more {
        return Err(Unsupported("a badge's page with more grants than its first page").into());
    }
    let slug = escape(s(&b["slug"]));
    let has_badge = b["has_badge"] == true;
    let can_select_title = v.enable_badges && b["allow_title"] == true && has_badge;
    let mut out = format!(
        "<div class=\"container show-badge {slug}\"><h1><a href=\"{}/badges\">{}</a>\n/\n{}</h1><div class=\"show-badge-details\">",
        cx.base_path,
        escape(&t(cx, "badges.title")),
        escape(s(&b["name"]))
    );
    out.push_str(&badge_card(
        cx,
        b,
        &type_name,
        &Card {
            large: true,
            count: user_grant_count,
            granted: has_badge,
            ..Default::default()
        },
    )?);
    out.push_str("<div class=\"badge-grant-info \"><div>");
    if can_select_title {
        out.push_str(&format!(
            "<div class=\"grant-info-item\">{}<button class=\"btn no-text btn-icon btn-default pad-left\" type=\"button\">{}<span aria-hidden=\"true\">\u{200b}</span></button></div>",
            escape(&t(cx, "badges.allow_title")),
            icon("pencil", None)
        ));
    }
    if b["multiple_grant"] == true {
        out.push_str(&format!(
            "<div class=\"grant-info-item\">{}</div>",
            escape(&t(cx, "badges.multiple_grant"))
        ));
    }
    out.push_str("</div></div>");
    if can_select_title {
        // The title picker (BadgeTitle), hidden until the pencil opens it.
        return Err(Unsupported("the badge page's title picker (BadgeTitle)").into());
    }
    out.push_str("</div>");
    if !grants.is_empty() {
        out.push_str(&format!(
            "<div class=\"user-badges {slug}\"><div><div class=\"badges-granted\">"
        ));
        for ub in grants {
            let Some(user) = users.iter().find(|u| u["id"] == ub["user_id"]) else {
                continue;
            };
            let granted_at =
                parse_time(&ub["granted_at"]).ok_or(Unsupported("a grant without its time"))?;
            let date = crate::user_profile_view::age_medium(cx, granted_at, false);
            let mut details = format!(
                "<div class=\"granted-on\">{}</div>",
                t_with(cx, "badges.granted_on", &[("date", &date)])
            );
            if let Some(number) = ub["post_number"].as_i64() {
                let topic = topics.iter().find(|t| t["id"] == ub["topic_id"]);
                let (url, title) = match topic {
                    Some(t) => (
                        format!("{}/t/{}/{}", cx.base_path, s(&t["slug"]), t["id"]),
                        s(&t["fancy_title"]).to_string(),
                    ),
                    None => return Err(Unsupported("a grant's post without its topic").into()),
                };
                details.push_str(&format!(
                    "<a class=\"post-link\" href=\"{url}/{number}\">{title}</a>"
                ));
            }
            out.push_str(&user_info(cx, user, &details));
        }
        out.push_str("</div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div>");
        // canShowOthers: a username and more than one grant.
        if v.username.is_some() && model_count > 1 {
            let others = model_count - user_grant_count.unwrap_or(0);
            out.push_str(&format!(
                "<div><a class=\"btn btn-default\" href=\"{}/badges/{}/{slug}\" id=\"show-others-with-badge-link\">{}</a></div>",
                cx.base_path,
                b["id"],
                escape(&t_count(cx, "badges.others_count", others, &[]))
            ));
        }
        out.push_str("</div><div class=\"loading-container\"></div>");
    }
    out.push_str("</div>");
    Ok((out, can_load_more))
}

/// A user's badges tab (`/u/:username/badges`): the grouped grants of
/// `/user-badges/:username.json?grouped=true`, sorted by grouping
/// position, badge type and badge id.
pub fn render_user_badges(
    cx: &ListContext,
    doc: &Value,
    username: &str,
    max_favorite_badges: i64,
) -> Result<String, AppError> {
    let empty = Vec::new();
    let grants = doc["user_badges"].as_array().unwrap_or(&empty);
    let badges = doc["badges"].as_array().unwrap_or(&empty);
    let types = doc["badge_types"].as_array().unwrap_or(&empty);
    let badge = |id: &Value| badges.iter().find(|b| b["id"] == *id);
    let mut sorted: Vec<(&Value, &Value)> = grants
        .iter()
        .filter_map(|ub| badge(&ub["badge_id"]).map(|b| (ub, b)))
        .collect();
    sorted.sort_by_key(|(ub, b)| {
        (
            ub["grouping_position"].as_i64(),
            b["badge_type_id"].as_i64(),
            b["id"].as_i64(),
        )
    });
    let favorites = grants.iter().filter(|ub| ub["is_favorite"] == true).count() as i64;
    let can_favorite_more = favorites < max_favorite_badges;
    let mut out = String::from("<section class=\"user-content\" id=\"user-content\">");
    if max_favorite_badges != 0 {
        out.push_str(&format!(
            "<p class=\"favorite-count\">{}</p>",
            escape(&t_with(
                cx,
                "badges.favorite_count",
                &[
                    ("count", &favorites.to_string()),
                    ("max", &max_favorite_badges.to_string())
                ]
            ))
        ));
    }
    out.push_str("<div class=\"badge-group-list\">");
    for (ub, b) in sorted {
        let type_name = types
            .iter()
            .find(|t| t["id"] == b["badge_type_id"])
            .map(|t| s(&t["name"]).to_string())
            .unwrap_or_default();
        out.push_str(&badge_card(
            cx,
            b,
            &type_name,
            &Card {
                count: Some(ub["count"].as_i64().unwrap_or(0)),
                filter_user: Some(username),
                can_favorite: ub["can_favorite"] == true,
                is_favorite: ub["is_favorite"] == true,
                can_favorite_more,
                ..Default::default()
            },
        )?);
    }
    out.push_str("</div></section>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugifies_like_the_client() {
        assert_eq!(slugify("Getting Started"), "getting-started");
        assert_eq!(slugify("Trust Level"), "trust-level");
        assert_eq!(slugify(" Odd -- Name! "), "odd-name");
    }
}
