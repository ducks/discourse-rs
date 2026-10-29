//! The user card as the Ember app renders it once loaded
//! (components/user-card-contents.gjs, with UserBadge, DBadgeButton,
//! DUserAvatarFlair and the chat, discourse-cakeday and discourse-solved
//! connectors), from
//! the users#show_card document. static/js/user-card.js floats it by the
//! avatar or mention that opened it.
//!
//! Not drawn: the user status (refused by users#show), suspension and
//! silencing (refused by users#show), the featured topic (refused by
//! users#show), Cancel filter (no user filters on the topic page) and
//! Delete (admin pages only).

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::topic_list_view::{ListContext, escape, icon, t, t_count, t_with};
use crate::user_profile_view::{age_medium, avatar, duration};

/// Who is looking, and from where.
pub struct Viewer {
    pub id: Option<i32>,
    /// chat's `userCanDirectMessage`, for its Chat button.
    pub can_direct_message: bool,
    /// The topic the card was opened on (`viewingTopic`), whose post
    /// count `include_post_count_for` asked for.
    pub topic_id: Option<i32>,
}

/// The site settings the card reads.
pub struct CardSettings {
    pub enable_badges: bool,
    pub allow_profile_backgrounds: bool,
    pub display_local_time_in_user_card: bool,
    pub tl3_links_no_follow: bool,
    /// discourse-user-notes, when it is on.
    pub user_notes: Option<crate::plugins::user_notes::view::UserNotesUi>,
    /// discourse-cakeday's, when it is on.
    pub cakeday: Option<crate::plugins::cakeday::view::CakedayUi>,
}

/// A site user field shown on cards (`site.user_fields` with
/// show_on_user_card), by position.
pub struct CardField {
    pub id: i64,
    pub name: String,
    pub searchable: bool,
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

fn date(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

/// DButton with an icon and a label.
fn button(class: &str, icon_name: &str, label: &str) -> String {
    format!(
        "<button class=\"btn btn-icon-text {class}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
        crate::post_view::d_icon(icon_name, None),
        escape(label)
    )
}

/// `dasherize`: underscores and spaces to dashes, camelCase split.
fn dasherize(s: &str) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 && !out.ends_with('-') {
                out.push('-');
            }
            out.extend(c.to_lowercase());
        } else if c == '_' || c == ' ' {
            out.push('-');
        } else {
            out.push(c);
        }
    }
    out
}

/// DAvatarFlair, for a user with a flair group whose flair is set
/// (DUserAvatarFlair). None when the group has none of its own (the
/// automatic groups' flair, which needs the site's groups).
fn flair(u: &Value) -> Option<String> {
    let url = u["flair_url"].as_str().filter(|v| !v.is_empty());
    let bg = u["flair_bg_color"].as_str().filter(|v| !v.is_empty());
    if url.is_none() && bg.is_none() {
        return None;
    }
    let color = u["flair_color"].as_str().filter(|v| !v.is_empty());
    Some(avatar_flair(s(&u["flair_name"]), url, bg, color))
}

/// DAvatarFlair for a group's flair (`flairName`, `flairUrl`,
/// `flairBgColor`, `flairColor`).
pub(crate) fn avatar_flair(
    name: &str,
    url: Option<&str>,
    bg: Option<&str>,
    color: Option<&str>,
) -> String {
    let is_icon = url.is_some_and(|u| !u.contains('/'));
    let mut css = Vec::new();
    if !is_icon {
        css.push(format!(
            "background-image: url({})",
            escape(url.unwrap_or("undefined"))
        ));
    }
    if let Some(bg) = bg {
        css.push(format!("background-color: #{}", escape(bg)));
    }
    if let Some(color) = color {
        css.push(format!("color: #{}", escape(color)));
    }
    let mut classes = format!("avatar-flair avatar-flair-{}", escape(name));
    if bg.is_some() {
        classes.push_str(" rounded");
    }
    if !is_icon {
        classes.push_str(" avatar-flair-image");
    }
    let style = if css.is_empty() {
        String::new()
    } else {
        format!(" style=\"{}\"", css.join("; "))
    };
    // convertIconClass: "fa-foo" or "far fa-foo" class names to icon ids.
    let inner = match url.filter(|_| is_icon) {
        Some(class) => {
            let class = class.trim();
            let id = match class.split_once(' ') {
                Some((prefix, name)) if prefix.starts_with("fa") => {
                    let name = name.trim_start_matches("fa-");
                    match prefix {
                        "far" => format!("far-{name}"),
                        "fab" => format!("fab-{name}"),
                        _ => name.to_string(),
                    }
                }
                _ => class.trim_start_matches("fa-").to_string(),
            };
            crate::post_view::d_icon(&id, None)
        }
        None => String::new(),
    };
    format!(
        "<div class=\"{classes}\"{style} title=\"{}\">{inner}</div>",
        escape(name)
    )
}

/// `domFromString("<div>" + description + "</div>").innerText`, the
/// badge button's title: the description without its tags.
pub(crate) fn strip_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    html_escape::decode_html_entities(&text).into_owned()
}

/// UserBadge (DBadgeButton inside the badge's link).
fn user_badge(cx: &ListContext, badge: &Value, badge_type: &str, username_lower: &str) -> String {
    let name = s(&badge["name"]);
    let title = badge["description"]
        .as_str()
        .filter(|d| !d.is_empty())
        .map(|d| format!(" title=\"{}\"", escape(&strip_tags(d))))
        .unwrap_or_default();
    // dIconOrImage: the image when there is one, else the icon.
    let image = match badge["image_url"].as_str().filter(|u| !u.is_empty()) {
        Some(url) => format!("<img src=\"{}\" alt=\"\">", escape(url)),
        None => crate::post_view::d_icon(s(&badge["icon"]), None),
    };
    format!(
        "<a class=\"user-card-badge-link\" href=\"{}/badges/{}/{}?username={}\"><span class=\"user-badge badge-type-{}{}\" data-badge-name=\"{}\"{title}>{image} <span class=\"badge-display-name\">{}</span> </span></a>",
        cx.base_path,
        badge["id"],
        escape(s(&badge["slug"])),
        escape(username_lower),
        escape(&badge_type.to_lowercase()),
        if badge["enabled"] == false {
            " disabled"
        } else {
            ""
        },
        escape(name),
        escape(name)
    )
}

/// The `#user-card` element with its loaded contents.
pub fn render(
    cx: &ListContext,
    settings: &CardSettings,
    viewer: &Viewer,
    fields: &[CardField],
    doc: &Value,
    clicked: &str,
) -> String {
    let u = &doc["user"];
    let base = cx.base_path;
    let username = s(&u["username"]);
    let username_lower = username.to_lowercase();
    let name = s(&u["name"]);
    let user_path = format!("{base}/u/{}", escape(&username_lower));
    let content_hidden = u["profile_hidden"] == true || u["inactive"] == true;
    let own = viewer.id.is_some() && viewer.id == u["id"].as_i64().map(|i| i as i32);
    let name_first = cx.settings.prioritize_name && !name.trim().is_empty();

    let mut out = String::new();
    // classNames and classNameBindings: show, showBadges, no-bg,
    // user-card-<username>, group-<primary group>.
    let background = u["card_background_upload_url"]
        .as_str()
        .filter(|b| !b.is_empty());
    let mut classes = vec!["user-card".to_string(), "show".to_string()];
    if settings.enable_badges {
        classes.push("show-badges".into());
    }
    if background.is_none() {
        classes.push("no-bg".into());
    }
    if !clicked.is_empty() {
        classes.push(format!("user-card-{}", escape(clicked)));
    }
    // `group-${primary_group_name}`: "null" when the document says so,
    // "undefined" when it has no such key (InactiveUserSerializer).
    let group = match u.get("primary_group_name") {
        Some(Value::String(name)) => name.as_str(),
        Some(_) => "null",
        None => "undefined",
    };
    classes.push(format!("group-{}", escape(group)));
    let style = match background {
        Some(url) if settings.allow_profile_backgrounds => {
            format!(" style=\"background-image: url({})\"", escape(url))
        }
        _ => String::new(),
    };
    out.push_str(&format!(
        "<div aria-label=\"{}\" id=\"user-card\" class=\"{}\" role=\"dialog\"{style}><div class=\"card-content\">",
        escape(&t(cx, "user.card")),
        classes.join(" ")
    ));

    // The first row: avatar, names, controls.
    out.push_str(
        "<div class=\"card-row first-row\"><div aria-hidden=\"true\" class=\"user-card-avatar\">",
    );
    let huge = avatar(cx, s(&u["avatar_template"]), 144);
    if content_hidden {
        out.push_str(&format!("<span class=\"card-huge-avatar\">{huge}</span>"));
    } else {
        let href = if own {
            format!("{base}/u/{}/preferences/account", escape(&username_lower))
        } else {
            user_path.clone()
        };
        let pencil = if own {
            format!(
                "<span class=\"own-avatar-pencil\"><span class=\"own-avatar-pencil--icon\">{}</span></span>",
                icon("pencil", None)
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<a class=\"card-huge-avatar\" href=\"{href}\" tabindex=\"-1\">{huge}{pencil}</a>"
        ));
    }
    if u["flair_group_id"].is_number()
        && let Some(flair) = flair(u)
    {
        out.push_str(&flair);
    }
    out.push_str("<div></div></div><div class=\"names\">");
    let staff = u["admin"] == true || u["moderator"] == true;
    let mut primary = vec!["names__primary"];
    if staff {
        primary.push("staff");
    }
    if u["trust_level"] == 0 {
        primary.push("new-user");
    }
    primary.push(if name_first { "full-name" } else { "username" });
    let shown = escape(if name_first { name } else { username });
    out.push_str(&format!("<div class=\"{}\">", primary.join(" ")));
    if content_hidden {
        out.push_str(&format!(
            "<span class=\"name-username-wrapper\">{shown}</span>"
        ));
    } else {
        out.push_str(&format!(
            "<a aria-label=\"{}\" class=\"user-profile-link\" href=\"{user_path}\"><span class=\"name-username-wrapper\">{shown}</span></a>",
            escape(&t_with(cx, "user.profile_link", &[("username", username)]))
        ));
    }
    out.push_str("</div>");
    if name_first {
        out.push_str(&format!(
            "<div class=\"names__secondary username\">{}</div>",
            escape(username)
        ));
    } else if !name.is_empty() {
        out.push_str(&format!(
            "<div class=\"names__secondary full-name\">{}</div>",
            escape(name)
        ));
    }
    if let Some(title) = u["title"].as_str().filter(|t| !t.is_empty()) {
        out.push_str(&format!(
            "<div class=\"names__secondary\">{}</div>",
            escape(title)
        ));
    }
    if u["staged"] == true {
        out.push_str(&format!(
            "<div class=\"names__secondary staged\">{}</div>",
            escape(&t(cx, "user.staged"))
        ));
    }
    // The user-card-post-names outlet: the connectors in plugin load
    // order, discourse-cakeday's then discourse-user-notes'.
    out.push_str("<div>");
    if let Some(ui) = &settings.cakeday {
        out.push_str(&crate::plugins::cakeday::view::user_card(
            ui, cx, u, viewer.id,
        ));
    }
    if let Some(ui) = &settings.user_notes {
        out.push_str(&crate::plugins::user_notes::view::card(
            ui, cx.i18n, base, u,
        ));
    }
    out.push_str("</div></div><ul class=\"usercard-controls\">");
    if u["can_send_private_message_to_user"] == true {
        out.push_str(&format!(
            "<li class=\"compose-pm\">{}</li>",
            button("btn-primary", "envelope", &t(cx, "user.private_message"))
        ));
    }
    // chat's user-card-below-message-button connector.
    if u["can_chat_user"] == true && viewer.can_direct_message {
        out.push_str(&format!(
            "<li class=\"user-card-below-message-button chat-button\">{}</li>",
            button(
                "btn-primary chat-direct-message-btn",
                "d-chat",
                &t(cx, "chat.title_capitalized")
            )
        ));
    }
    // showFilter: on a topic, with two posts or more there.
    if let Some(topic_id) = viewer.topic_id {
        let count = u["topic_post_count"][topic_id.to_string()]
            .as_i64()
            .unwrap_or(0);
        if count >= 2 {
            out.push_str(&format!(
                "<li>{}</li>",
                button(
                    "btn-default",
                    "filter",
                    &t_count(cx, "topic.filter_to", count, &[])
                )
            ));
        }
    }
    out.push_str("</ul></div>");

    // The second row: why the profile is not shown, or the bio.
    if u["profile_hidden"] == true {
        out.push_str(&format!(
            "<div class=\"card-row second-row\"><div class=\"profile-hidden\"><span role=\"alert\">{}</span></div></div>",
            escape(&t(cx, "user.profile_hidden"))
        ));
    } else if u["inactive"] == true {
        out.push_str(&format!(
            "<div class=\"card-row second-row\"><div class=\"inactive-user\"><span role=\"alert\">{}</span></div></div>",
            escape(&t(cx, "user.inactive_user"))
        ));
    }
    if let Some(bio) = u["bio_excerpt"].as_str().filter(|b| !b.is_empty()) {
        out.push_str(&format!(
            "<div class=\"card-row second-row\"><div class=\"bio\"><div>{bio}</div></div></div>"
        ));
    }

    // hasLocaleOrWebsite
    let website_name = u["website_name"].as_str().filter(|w| !w.is_empty());
    let location = u["location"].as_str().filter(|l| !l.is_empty());
    let timezone = if settings.display_local_time_in_user_card {
        u["timezone"].as_str().filter(|z| !z.is_empty())
    } else {
        None
    };
    if website_name.is_some() || location.is_some() || timezone.is_some() {
        out.push_str("<div class=\"card-row\"><div class=\"location-and-website\">");
        if let Some(site) = website_name {
            // linkWebsite: not for TL0 (isBasic); nofollow unless TL3+
            // and tl3_links_no_follow is off.
            let link = if u["trust_level"] == 0 {
                format!(
                    "<span title=\"{}\">{}</span>",
                    escape(s(&u["website"])),
                    escape(site)
                )
            } else {
                let remove_nofollow =
                    u["trust_level"].as_i64().unwrap_or(0) > 2 && !settings.tl3_links_no_follow;
                format!(
                    "<a href=\"{}\" rel=\"noopener {}\" target=\"_blank\">{}</a>",
                    escape(s(&u["website"])),
                    if remove_nofollow { "" } else { "nofollow ugc" },
                    escape(site)
                )
            };
            out.push_str(&format!(
                "<span class=\"website-name\">{}{link}</span>",
                icon("globe", None)
            ));
        }
        if let Some(location) = location {
            out.push_str(&format!(
                "<span class=\"location\">{}<span>{}</span></span>",
                icon("location-dot", None),
                escape(location)
            ));
        }
        if settings.display_local_time_in_user_card {
            // moment.tz(timezone).format(dates.time), at the time the
            // card opens; an unknown zone is moment's UTC.
            let zone: chrono_tz::Tz = timezone
                .and_then(|z| z.parse().ok())
                .unwrap_or(chrono_tz::UTC);
            let wall = Utc.from_utc_datetime(&cx.now.with_timezone(&zone).naive_local());
            let time =
                crate::pretty_text::render::local_dates::format_utc(wall, &t(cx, "dates.time"))
                    .unwrap_or_default();
            out.push_str(&format!(
                "<span class=\"local-time\" title=\"{}\">{}<span>{}</span></span>",
                escape(&t(cx, "local_time")),
                icon("far-clock", None),
                escape(&time)
            ));
        }
        out.push_str("<span></span></div></div>");
    }

    // The metadata row.
    out.push_str("<div class=\"card-row metadata-row\">");
    if !content_hidden {
        out.push_str("<div class=\"metadata\">");
        if let Some(at) = date(&u["last_posted_at"]) {
            out.push_str(&format!(
                "<div class=\"metadata__last-posted\"><span class=\"desc\">{}</span> {}</div>",
                escape(&t(cx, "last_post")),
                age_medium(cx, at, true)
            ));
        }
        if let Some(at) = date(&u["created_at"]) {
            out.push_str(&format!(
                "<div class=\"metadata__user-created\"><span class=\"desc\">{}</span> {}</div>",
                escape(&t(cx, "joined")),
                age_medium(cx, at, true)
            ));
        }
        let time_read = u["time_read"].as_i64().unwrap_or(0);
        if time_read != 0 {
            let recent = u["recent_time_read"].as_i64().unwrap_or(0);
            let show_recent = time_read != recent && recent != 0;
            let total = duration(cx, time_read, "tiny");
            let recent_tiny = duration(cx, recent, "tiny");
            let tooltip = if show_recent {
                t_with(
                    cx,
                    "time_read_recently_tooltip",
                    &[("time_read", &total), ("recent_time_read", &recent_tiny)],
                )
            } else {
                t_with(cx, "time_read_tooltip", &[("time_read", &total)])
            };
            out.push_str(&format!(
                "<div class=\"metadata__time-read\" title=\"{}\"><span class=\"desc\">{}</span> {}",
                escape(&tooltip),
                escape(&t(cx, "time_read")),
                total
            ));
            if show_recent {
                out.push_str(&format!(
                    " <span>({})</span>",
                    escape(&t_with(
                        cx,
                        "time_read_recently",
                        &[("time_read", &recent_tiny)]
                    ))
                ));
            }
            out.push_str("</div>");
        }
        // discourse-solved's user-card-metadata connector.
        if let Some(solved) = u.get("accepted_answers") {
            out.push_str("<div class=\"user-card-metadata-outlet accepted-answers\">");
            if solved.as_i64().unwrap_or(0) != 0 {
                out.push_str(&format!(
                    "<span class=\"desc\">{}</span> <span>{solved}</span>",
                    escape(&t(cx, "solutions"))
                ));
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    out.push_str("</div>");

    // publicUserFields
    let values = &u["user_fields"];
    let shown_fields: Vec<(&CardField, &Value)> = fields
        .iter()
        .filter_map(|f| {
            let v = &values[f.id.to_string()];
            let empty = v.is_null()
                || v.as_str().is_some_and(str::is_empty)
                || v.as_array().is_some_and(Vec::is_empty);
            (!empty).then_some((f, v))
        })
        .collect();
    if !shown_fields.is_empty() {
        out.push_str("<div class=\"card-row\"><div class=\"public-user-fields\">");
        for (field, value) in shown_fields {
            out.push_str(&format!(
                "<div class=\"public-user-field public-user-field__{}\"><span class=\"user-field-name\">{}:</span><span class=\"user-field-value\">",
                escape(&dasherize(&field.name)),
                escape(&field.name)
            ));
            match value.as_array() {
                Some(list) => {
                    for v in list {
                        let v = v
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string());
                        let inner = if field.searchable {
                            format!(
                                "<a href=\"{base}/u?name={}\">{}</a>",
                                form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>(),
                                escape(&v)
                            )
                        } else {
                            escape(&v)
                        };
                        out.push_str(&format!(
                            "<span class=\"user-field-value-list-item\">{inner}</span>"
                        ));
                    }
                }
                None => {
                    let v = value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string());
                    out.push_str(&escape(&v));
                }
            }
            out.push_str("</span></div>");
        }
        out.push_str("</div></div>");
    }

    // The featured badges.
    if settings.enable_badges {
        out.push_str("<div class=\"card-row\">");
        let ids: Vec<&Value> = u["featured_user_badge_ids"]
            .as_array()
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        if !ids.is_empty() {
            out.push_str("<div class=\"badge-section\">");
            let empty = Vec::new();
            let user_badges = doc["user_badges"].as_array().unwrap_or(&empty);
            let badges = doc["badges"].as_array().unwrap_or(&empty);
            let types = doc["badge_types"].as_array().unwrap_or(&empty);
            for id in &ids {
                let Some(ub) = user_badges.iter().find(|b| &&b["id"] == id) else {
                    continue;
                };
                let Some(badge) = badges.iter().find(|b| b["id"] == ub["badge_id"]) else {
                    continue;
                };
                let badge_type = types
                    .iter()
                    .find(|t| t["id"] == badge["badge_type_id"])
                    .map(|t| s(&t["name"]))
                    .unwrap_or_default();
                out.push_str(&user_badge(cx, badge, badge_type, &username_lower));
            }
            let more = u["badge_count"].as_i64().unwrap_or(0) - ids.len() as i64;
            if more > 0 {
                out.push_str(&format!(
                    "<span class=\"more-user-badges\"><a href=\"{user_path}/badges\">{}</a></span>",
                    escape(&t_count(cx, "badges.more_badges", more, &[]))
                ));
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    out.push_str("</div></div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dasherizes_field_names() {
        assert_eq!(dasherize("Favorite color"), "favorite-color");
        assert_eq!(dasherize("myField_name"), "my-field-name");
    }

    #[test]
    fn badge_titles_lose_their_tags() {
        assert_eq!(
            strip_tags("<a href=\"https://x\">Granted</a> invitations"),
            "Granted invitations"
        );
    }

    #[test]
    fn flair_of_an_icon_and_of_an_image() {
        let icon_flair = flair(&serde_json::json!({
            "flair_name": "team", "flair_url": "fa-heart", "flair_bg_color": "ff0000"
        }))
        .unwrap();
        assert!(icon_flair.starts_with(
            "<div class=\"avatar-flair avatar-flair-team rounded\" style=\"background-color: #ff0000\" title=\"team\"><svg"
        ));
        let image = flair(&serde_json::json!({
            "flair_name": "g", "flair_url": "/uploads/f.png"
        }))
        .unwrap();
        assert_eq!(
            image,
            "<div class=\"avatar-flair avatar-flair-g avatar-flair-image\" style=\"background-image: url(/uploads/f.png)\" title=\"g\"></div>"
        );
        assert!(flair(&serde_json::json!({ "flair_name": "g" })).is_none());
    }
}
