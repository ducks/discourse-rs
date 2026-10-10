//! GET /user-menu: a member's recent notifications as the header menu's
//! HTML. It is the list the Ember client's user menu fetches
//! (`/notifications?recent=true`, 30 at most): the same prioritized list,
//! and the same bump of what the member has seen, whose published
//! notification state clears the header's count over the live stream.

use askama::Template;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::notifications::{self, Notifications};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// The user menu's page size.
const LIMIT: i64 = 30;

#[derive(Template)]
#[template(path = "user_menu.html")]
pub struct UserMenu {
    pub base_path: String,
    pub items: Vec<MenuItem>,
    pub username: String,
    pub csrf_token: String,
}

pub struct MenuItem {
    pub url: String,
    pub actor: String,
    pub verb: String,
    /// HTML: a topic's fancy title is stored escaped; anything else is
    /// escaped here.
    pub title_html: String,
    pub read: bool,
}

/// GET /user-menu
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(user) = guardian.user().cloned() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let votes_released = state
        .i18n
        .t(
            if settings.get("topic_voting_enable_vote_limits")?.truthy() {
                "js.topic_voting.notification_label.vote_released"
            } else {
                "js.topic_voting.notification_label.voting_closed"
            },
        )
        .unwrap_or_default()
        .to_string();
    let query = notifications::Query {
        recent: true,
        silent: false,
        limit: LIMIT,
        offset: 0,
        filter: None,
        types: Vec::new(),
    };
    let (doc, bumped) = Notifications {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
    }
    .index(user.id, &user.username, &query)
    .await?;
    if bumped {
        crate::bus::publish_notifications_state(&state.bus, &mut conn, &settings, user.id).await?;
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let items = doc["notifications"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|n| menu_item(&state.i18n, &base_path, n, &votes_released))
                .collect()
        })
        .unwrap_or_default();
    // Logging out is in the menu, as in the Ember client's profile tab.
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let csrf_token = vs
        .viewer
        .as_ref()
        .map(|v| v.csrf_token.clone())
        .unwrap_or_default();
    let html = UserMenu {
        base_path,
        items,
        username: user.username.clone(),
        csrf_token,
    }
    .render()
    .map_err(crate::html::HtmlError::from)?;
    let response = (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, no-store"),
        ],
        html,
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// `Notification.types[:votes_released]` (discourse-topic-voting)
const VOTES_RELEASED: i64 = 26;

/// One notification as the menu shows it: who, what, where, and the link.
/// `votes_released` is discourse-topic-voting's label for its type's
/// notifications, in the actor's place.
fn menu_item(
    i18n: &crate::i18n::I18n,
    base_path: &str,
    n: &Value,
    votes_released: &str,
) -> MenuItem {
    let data = &n["data"];
    if n["notification_type"] == CHAT_MENTION {
        return chat_mention_item(i18n, base_path, n);
    }
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    // A bookmark reminder is the member's own: no actor, as Discourse shows it.
    let actor = if n["notification_type"] == 24 {
        String::new()
    } else if n["notification_type"] == VOTES_RELEASED {
        votes_released.to_string()
    } else {
        data["display_username"]
            .as_str()
            .or(data["username"].as_str())
            .unwrap_or_default()
            .to_string()
    };
    let title_html = match n["fancy_title"].as_str() {
        Some(fancy) => fancy.to_string(),
        None => match data["topic_title"].as_str().or(data["badge_name"].as_str()) {
            Some(t) => html_escape::encode_text(t).into_owned(),
            None => String::new(),
        },
    };
    let url = match n["topic_id"].as_i64() {
        Some(topic_id) => format!(
            "{base_path}/t/{}/{topic_id}/{}",
            text(&n["slug"]),
            n["post_number"].as_i64().unwrap_or(1)
        ),
        None => format!("{base_path}/u/{}", text(&data["username"])),
    };
    MenuItem {
        url,
        actor,
        verb: crate::html::notification_verb(n["notification_type"].as_i64().unwrap_or(0))
            .to_string(),
        title_html,
        read: n["read"] == true,
    }
}

/// `Notification.types[:chat_mention]`
const CHAT_MENTION: i64 = 29;

/// The chat plugin's chat_mention renderer: who mentioned, whom (them, or
/// the group, @here or @all) in which channel, linking to the message.
fn chat_mention_item(i18n: &crate::i18n::I18n, base_path: &str, n: &Value) -> MenuItem {
    let data = &n["data"];
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let identifier = data["identifier"].as_str().map(|i| format!("@{i}"));
    let prefix = if data["is_direct_message_channel"] == true {
        "js.notifications.popup.direct_message_chat_mention"
    } else {
        "js.notifications.popup.chat_mention"
    };
    let suffix = if identifier.is_some() {
        "other_plain"
    } else {
        "direct"
    };
    let channel = text(&data["chat_channel_title"]);
    let verb = i18n
        .t_with(
            &format!("{prefix}.{suffix}"),
            &[
                ("identifier", identifier.as_deref().unwrap_or_default()),
                ("channel", &channel),
            ],
        )
        .unwrap_or_default();
    let slug = slugify_channel(data["chat_channel_slug"].as_str(), &channel);
    let mut url = format!("{base_path}/chat/c/{slug}/{}", data["chat_channel_id"]);
    match data["chat_thread_id"].as_i64() {
        Some(thread) => url.push_str(&format!("/t/{thread}")),
        None => url.push_str(&format!("/{}", data["chat_message_id"])),
    }
    MenuItem {
        url,
        actor: text(&data["mentioned_by_username"]),
        verb,
        title_html: String::new(),
        read: n["read"] == true,
    }
}

/// chat's slugifyChannel: the slug, else the title slugified (utilities'
/// slugify, else the title lower cased and dashed), a hundred characters
/// at most; "-" for none.
fn slugify_channel(slug: Option<&str>, title: &str) -> String {
    if let Some(slug) = slug.filter(|s| !s.is_empty()) {
        return slug.to_string();
    }
    if title.is_empty() {
        return "-".into();
    }
    let dashed = |s: &str| -> String {
        let lower = s.trim().to_lowercase();
        let mut out = String::new();
        let mut chars = lower.chars().peekable();
        // /\s|_+/g -> "-"
        while let Some(c) = chars.next() {
            if c.is_whitespace() {
                out.push('-');
            } else if c == '_' {
                while chars.peek() == Some(&'_') {
                    chars.next();
                }
                out.push('-');
            } else {
                out.push(c);
            }
        }
        out
    };
    let mut slug: String = dashed(title)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-').to_string();
    let resolved: String = if slug.is_empty() { dashed(title) } else { slug }
        .chars()
        .take(100)
        .collect();
    if resolved.is_empty() {
        "-".into()
    } else {
        resolved
    }
}
