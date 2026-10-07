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
    pub verb: &'static str,
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
        .map(|list| list.iter().map(|n| menu_item(&base_path, n)).collect())
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

/// One notification as the menu shows it: who, what, where, and the link.
fn menu_item(base_path: &str, n: &Value) -> MenuItem {
    let data = &n["data"];
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    // A bookmark reminder is the member's own: no actor, as Discourse shows it.
    let actor = if n["notification_type"] == 24 {
        String::new()
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
        verb: crate::html::notification_verb(n["notification_type"].as_i64().unwrap_or(0)),
        title_html,
        read: n["read"] == true,
    }
}
