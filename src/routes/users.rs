//! Port of users_controller#show and #summary and user_actions#index for
//! anonymous readers, plus the server-rendered profile page.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::search::Peer;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::users::{PRIVATE_TYPES, PUBLIC_TYPES, User, Users};
use crate::{AppError, AppState, Unsupported};

#[derive(Deserialize, Default)]
pub struct ShowParams {
    skip_track_visit: Option<String>,
    include_post_count_for: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ActionsParams {
    username: Option<String>,
    filter: Option<String>,
    offset: Option<String>,
    limit: Option<String>,
    acting_username: Option<String>,
}

/// `Discourse::InvalidAccess` as JSON.
fn invalid_access(state: &AppState) -> Response {
    let text = state
        .i18n
        .t("invalid_access")
        .unwrap_or("You are not permitted to view the requested resource.");
    (
        StatusCode::FORBIDDEN,
        Json(json!({"errors": [text], "error_type": "invalid_access"})),
    )
        .into_response()
}

enum Action {
    Show,
    Summary,
}

/// `/u/:username(/...)`: which action the tail selects, and whether the
/// request wants JSON.
fn route(username: &str, rest: Option<&str>) -> Result<(String, Action, bool), Unsupported> {
    let (username, json) = match username.strip_suffix(".json") {
        Some(u) => (u.to_string(), true),
        None => (username.to_string(), false),
    };
    let Some(rest) = rest else {
        return Ok((username, Action::Show, json));
    };
    let (rest, json) = match rest.strip_suffix(".json") {
        Some(r) => (r, true),
        None => (rest, json),
    };
    let action = match rest {
        "summary" => Action::Summary,
        "badges" | "notifications" | "messages" | "private-messages" | "deleted-posts" => {
            Action::Show
        }
        "activity" if json => return Err(Unsupported("/u/:username/activity.json (posts feed)")),
        "activity" => Action::Show,
        r if r.starts_with("activity/")
            || r.starts_with("notifications/")
            || r.starts_with("messages/")
            || r.starts_with("private-messages/") =>
        {
            Action::Show
        }
        "card" => return Err(Unsupported("/u/:username/card.json")),
        _ => return Err(Unsupported("unknown /u/:username route")),
    };
    Ok((username, action, json))
}

/// GET /u/{username}
pub async fn show(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, AppError> {
    let (username, action, json) = route(&username, None)?;
    respond(state, username, action, json, params, headers, peer).await
}

/// GET /u/{username}/{*rest}
pub async fn show_with_tail(
    State(state): State<AppState>,
    Path((username, rest)): Path<(String, String)>,
    Query(params): Query<ShowParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, AppError> {
    let (username, action, json) = route(&username, Some(&rest))?;
    respond(state, username, action, json, params, headers, peer).await
}

async fn respond(
    state: AppState,
    username: String,
    action: Action,
    json: bool,
    params: ShowParams,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
) -> Result<Response, AppError> {
    if params.include_post_count_for.is_some() {
        return Err(Unsupported("include_post_count_for").into());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    // ensure_public_can_see_profiles!
    if settings.get("hide_user_profiles_from_public")?.truthy() {
        return Ok((noindex, invalid_access(&state)).into_response());
    }
    let not_found = || (noindex, super::topics::not_found_response(&state, false)).into_response();
    let Some(user) = User::find_active(&mut conn, &username).await? else {
        return Ok(not_found());
    };
    let guardian = Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root();
    let mut users = Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path,
    };
    let doc = match action {
        Action::Show => {
            let doc = users.show(&user).await?;
            if params.skip_track_visit.is_none() {
                let ip = super::search::remote_ip(&headers, peer);
                users.track_view(&user, &ip).await?;
            }
            doc
        }
        Action::Summary => {
            if !user.visible_to_anonymous(&settings)? {
                return Ok(not_found());
            }
            users.summary(&user).await?
        }
    };
    if json {
        return Ok((noindex, Json(doc)).into_response());
    }
    // The HTML profile draws on the show document, the summary and the
    // public activity stream.
    let show_doc = match action {
        Action::Show => doc,
        Action::Summary => users.show(&user).await?,
    };
    let visible = user.visible_to_anonymous(&settings)?;
    let summary_doc = if visible {
        Some(users.summary(&user).await?)
    } else {
        None
    };
    let actions = if visible && !settings.get("hide_user_activity_tab")?.truthy() {
        users.actions(&user, PUBLIC_TYPES, 0, 30, None).await?
    } else {
        Vec::new()
    };
    let site = crate::html::Site::from_settings(&settings, base_path)?;
    let page = profile_page(
        site,
        &user,
        &show_doc,
        summary_doc.as_ref(),
        &actions,
        &settings,
    )?;
    Ok((
        noindex,
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response())
}

/// GET /user_actions.json
pub async fn actions(
    State(state): State<AppState>,
    Query(params): Query<ActionsParams>,
) -> Result<Response, AppError> {
    let Some(username) = params.username.as_deref().filter(|u| !u.is_empty()) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(
                json!({"errors": ["param is missing or the value is empty or invalid: username"]}),
            ),
        )
            .into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(user) = User::find_active(&mut conn, username).await? else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let offset = params
        .offset
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0)
        .max(0);
    let mut action_types: Vec<i32> = params
        .filter
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| crate::ruby::to_i(s) as i32)
        .collect();
    let limit = params
        .limit
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(30)
        .min(100);
    if limit < 0 {
        return Err(Unsupported("negative user_actions limit").into());
    }
    // ensure_user_actions_visible!: hidden profiles and private types 404.
    if !user.visible_to_anonymous(&settings)?
        || settings.get("hide_user_activity_tab")?.truthy()
        || action_types.iter().any(|t| PRIVATE_TYPES.contains(t))
    {
        return Ok(super::topics::not_found_response(&state, false));
    }
    if action_types.is_empty() {
        action_types = PUBLIC_TYPES.to_vec();
    }
    let guardian = Guardian::anonymous();
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut users = Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
    };
    let list = users
        .actions(
            &user,
            &action_types,
            offset,
            limit,
            params.acting_username.as_deref().filter(|u| !u.is_empty()),
        )
        .await?;
    Ok(Json(json!({"user_actions": list})).into_response())
}

#[derive(Template)]
#[template(path = "user.html")]
pub struct ProfilePage {
    pub site_title: String,
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub username: String,
    pub name: Option<String>,
    pub title: Option<String>,
    pub hidden: bool,
    pub joined: String,
    pub last_posted: Option<String>,
    pub stats: Vec<(String, String)>,
    pub badges: Vec<BadgeItem>,
    pub top_categories: Vec<CategoryCount>,
    pub activity: Vec<ActivityItem>,
}

pub struct BadgeItem {
    pub name: String,
    pub description: String,
}

pub struct CategoryCount {
    pub name: String,
    pub url: String,
    pub count: i64,
}

pub struct ActivityItem {
    pub kind: String,
    pub title: String,
    pub url: String,
    pub excerpt: String,
    pub created_at: String,
}

fn profile_page(
    site: crate::html::Site,
    user: &User,
    show: &Value,
    summary: Option<&Value>,
    actions: &[Value],
    settings: &SiteSettings,
) -> Result<ProfilePage, AppError> {
    let base = site.base_path.clone();
    let u = &show["user"];
    let hidden = u["profile_hidden"] == json!(true);
    let mut stats = Vec::new();
    let mut badges = Vec::new();
    let mut top_categories = Vec::new();
    if let Some(s) = summary {
        let us = &s["user_summary"];
        for (label, key) in [
            ("Topics created", "topic_count"),
            ("Posts created", "post_count"),
            ("Likes given", "likes_given"),
            ("Likes received", "likes_received"),
            ("Days visited", "days_visited"),
        ] {
            stats.push((label.to_string(), us[key].to_string()));
        }
        for c in us["top_categories"].as_array().into_iter().flatten() {
            let id = c["id"].as_i64().unwrap_or(0);
            let slug = c["slug"].as_str().unwrap_or("");
            top_categories.push(CategoryCount {
                name: c["name"].as_str().unwrap_or("").to_string(),
                url: format!("{base}/c/{slug}/{id}"),
                count: c["topic_count"].as_i64().unwrap_or(0)
                    + c["post_count"].as_i64().unwrap_or(0),
            });
        }
        for b in s["badges"].as_array().into_iter().flatten() {
            badges.push(BadgeItem {
                name: b["name"].as_str().unwrap_or("").to_string(),
                description: b["description"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    let activity = actions
        .iter()
        .map(|a| {
            let kind = match a["action_type"].as_i64() {
                Some(1) => "liked",
                Some(4) => "created",
                Some(5) => "replied",
                _ => "activity",
            };
            let slug = a["slug"].as_str().unwrap_or("topic");
            let topic_id = a["topic_id"].as_i64().unwrap_or(0);
            let post_number = a["post_number"].as_i64().unwrap_or(1);
            let mut url = format!("{base}/t/{slug}/{topic_id}");
            if post_number > 1 {
                url.push_str(&format!("/{post_number}"));
            }
            ActivityItem {
                kind: kind.to_string(),
                title: a["title"].as_str().unwrap_or("").to_string(),
                url,
                excerpt: a["excerpt"].as_str().unwrap_or("").to_string(),
                created_at: a["created_at"].as_str().unwrap_or("").to_string(),
            }
        })
        .collect();
    let _ = settings;
    Ok(ProfilePage {
        site_title: site.site_title,
        site_description: site.site_description,
        lang: site.lang,
        base_path: site.base_path,
        username: user.username.clone(),
        name: u["name"].as_str().map(str::to_string),
        title: user.title.clone(),
        hidden,
        joined: crate::topic_list::time_json(user.created_at),
        last_posted: user.last_posted_at.map(crate::topic_list::time_json),
        stats,
        badges,
        top_categories,
        activity,
    })
}
