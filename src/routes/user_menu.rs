//! The header's user menu. GET /user-menu?tab= renders it as the Ember
//! client does once a tab's list has loaded (crate::user_menu_view): the
//! notifications lists come from what `/notifications?recent=true` returns
//! (30 at most, the all-notifications tab bumping what the member has
//! seen, whose published notification state clears the header's count
//! over the live stream; the others `filter_by_types` and silent), the
//! messages tab from UsersController#user_menu_messages, the bookmarks tab
//! from #user_menu_bookmarks. GET /u/:username/user-menu-private-messages
//! is that action's JSON.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::notifications::{self, Notifications};
use crate::session::current::{AuthGuardian, Incoming};
use crate::site_settings::SiteSettings;
use crate::user_menu_view::{self, MenuContext};
use crate::{AppError, AppState, Unsupported};

/// The notifications lists' page size (`setNotificationsLimit`'s default).
const LIMIT: i64 = 30;

/// `UsersController::USER_MENU_LIST_LIMIT`
const USER_MENU_LIST_LIMIT: i64 = 20;

#[derive(Deserialize, Default)]
pub struct MenuParams {
    tab: Option<String>,
}

/// GET /user-menu?tab=
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Query(params): Query<MenuParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let (Some(user), Some(session)) = (guardian.user().cloned(), incoming.session.as_ref()) else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if settings.get("show_user_menu_avatars")?.truthy() {
        return Err(Unsupported("show_user_menu_avatars (the items' avatars)").into());
    }
    let current_user = crate::current_user::serialize(
        &mut conn,
        &state,
        &settings,
        &session.user,
        session.token.authenticated_with_oauth.unwrap_or(false),
    )
    .await?;
    let can_chat = crate::plugins::chat::enabled(&settings)?
        && crate::plugins::chat::can_chat(&mut conn, &settings, &guardian).await?
        && crate::plugins::chat::options(&mut conn, user.id)
            .await?
            .is_some_and(|o| o.chat_enabled);
    let base_path = state.config.globals.relative_url_root().to_string();
    let cx = MenuContext {
        i18n: &state.i18n,
        base_path: &base_path,
        mobile: crate::mobile::mobile_view() || crate::mobile::narrow_desktop_view(),
        user: &current_user,
        notification_types: crate::site::NOTIFICATION_TYPES,
        prioritize_full_name: settings.get("prioritize_full_name_in_ux")?.truthy(),
        vote_limits: settings.get("topic_voting_enable_vote_limits")?.truthy(),
        enable_user_status: settings.get("enable_user_status")?.truthy(),
        can_chat,
        emojis: settings.emojis(),
        now: crate::clock::now(),
        relative_date_duration: settings.get("relative_date_duration")?.to_i(),
    };
    let (top, bottom) = user_menu_view::tabs(&cx);
    let requested = params.tab.as_deref().unwrap_or("all-notifications");
    let Some(tab) = top.iter().chain(bottom.iter()).find(|t| t.id == requested) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let panel = match tab.id {
        "profile" => user_menu_view::profile_tab(&cx),
        "review-queue" => {
            return Err(
                Unsupported("the user menu's review queue (UserMenuReviewablesList)").into(),
            );
        }
        "messages" => {
            let doc = messages_doc(&state, &mut conn, &settings, &guardian, user.id).await?;
            let options = user_menu_view::list_options(&cx, tab);
            user_menu_view::items_list(tab, &message_items(&cx, &doc), &options)
        }
        "bookmarks" => {
            let urls = crate::url::Urls {
                config: &state.config,
                settings: &settings,
            };
            let doc = crate::bookmarks::Bookmarks {
                conn: &mut conn,
                settings: &settings,
                i18n: &state.i18n,
                guardian: &guardian,
                urls: &urls,
            }
            .user_menu(user.id)
            .await?;
            let mut items: Vec<String> = doc["notifications"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|n| user_menu_view::notification_item(&cx, n))
                .collect();
            items.extend(
                doc["bookmarks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| user_menu_view::bookmark_item(&cx, b)),
            );
            let options = user_menu_view::list_options(&cx, tab);
            user_menu_view::items_list(tab, &items, &options)
        }
        _ => {
            // UserMenuNotificationsList#fetchItems
            let types: Vec<i32> = tab
                .types
                .iter()
                .filter_map(|name| {
                    crate::site::NOTIFICATION_TYPES
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, id)| *id as i32)
                })
                .collect();
            let query = notifications::Query {
                recent: true,
                silent: !tab.types.is_empty(),
                limit: LIMIT,
                offset: 0,
                filter: None,
                types,
            };
            let urls = crate::url::Urls {
                config: &state.config,
                settings: &settings,
            };
            let (doc, bumped) = Notifications {
                conn: &mut conn,
                settings: &settings,
                guardian: &guardian,
                urls: &urls,
            }
            .index(user.id, &user.username, &query)
            .await?;
            if bumped {
                crate::bus::publish_notifications_state(&state.bus, &mut conn, &settings, user.id)
                    .await?;
            }
            let items: Vec<String> = doc["notifications"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|n| user_menu_view::notification_item(&cx, n))
                .collect();
            let options = user_menu_view::list_options(&cx, tab);
            user_menu_view::items_list(tab, &items, &options)
        }
    };
    let html = user_menu_view::menu(&cx, tab.id, &panel);
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
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

/// UserMenuMessagesList#fetchItems: the unread message notifications, then
/// the read ones and the messages merged newest first.
fn message_items(cx: &MenuContext, doc: &Value) -> Vec<String> {
    let list = |key: &str| doc[key].as_array().cloned().unwrap_or_default();
    let users = list("users");
    let mut items: Vec<String> = list("unread_notifications")
        .iter()
        .map(|n| user_menu_view::notification_item(cx, n))
        .collect();
    let read = list("read_notifications");
    let topics = list("topics");
    // mergeSortedLists(read, topics, topic bumped after the notification)
    let time = |v: &Value| {
        v.as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis())
    };
    let (mut i, mut j) = (0, 0);
    while i < read.len() || j < topics.len() {
        let take_topic = i == read.len()
            || (j < topics.len()
                && match (time(&topics[j]["bumped_at"]), time(&read[i]["created_at"])) {
                    (Some(t), Some(n)) => t > n,
                    _ => false,
                });
        if take_topic {
            items.push(user_menu_view::message_item(cx, &topics[j], &users));
            j += 1;
        } else {
            items.push(user_menu_view::notification_item(cx, &read[i]));
            i += 1;
        }
    }
    items
}

/// UsersController#user_menu_messages for the current user.
async fn messages_doc(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    user_id: i32,
) -> Result<Value, AppError> {
    const PRIVATE_MESSAGE: i32 = 6;
    const GROUP_MESSAGE_SUMMARY: i32 = 16;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let mut notes = Notifications {
        conn: &mut *conn,
        settings,
        guardian,
        urls: &urls,
    };
    let unread = notes
        .for_user_menu(
            user_id,
            &[PRIVATE_MESSAGE, GROUP_MESSAGE_SUMMARY],
            false,
            USER_MENU_LIST_LIMIT,
        )
        .await?;
    let unread = notes.filter_inaccessible_topics(unread).await?;
    let mut topics = Vec::new();
    let mut users = Vec::new();
    let mut read = Vec::new();
    if (unread.len() as i64) < USER_MENU_LIST_LIMIT {
        let mut exclude: Vec<i32> = Vec::new();
        for n in &unread {
            if let Some(id) = n["topic_id"].as_i64().map(|id| id as i32)
                && !exclude.contains(&id)
            {
                exclude.push(id);
            }
        }
        let limit = USER_MENU_LIST_LIMIT - unread.len() as i64;
        let list = crate::pm_lists::PmQuery {
            conn: &mut *conn,
            settings,
            guardian,
            options: crate::topic_query::Options {
                per_page: Some(limit),
                ..Default::default()
            },
            owner_id: user_id,
        }
        .direct_and_groups_watching(&exclude)
        .await?;
        let urls = crate::url::Urls {
            config: &state.config,
            settings,
        };
        let json = crate::topic_list::TopicListSerializer {
            conn: &mut *conn,
            settings,
            i18n: &state.i18n,
            guardian,
            urls: &urls,
            more_topics_url: None,
            category_id: None,
            group_id: None,
            prefetched: Default::default(),
        }
        .serialize(&list)
        .await?;
        topics = json["topic_list"]["topics"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        // The last posters as BasicUserSerializer has them, unless names
        // and avatars are both off.
        if settings.get("show_user_menu_avatars")?.truthy()
            || !settings.get("prioritize_username_in_ux")?.truthy()
        {
            let all = json["users"].as_array().cloned().unwrap_or_default();
            for t in &topics {
                let Some(id) = t["posters"]
                    .as_array()
                    .and_then(|p| p.last())
                    .and_then(|p| p["user_id"].as_i64())
                else {
                    continue;
                };
                if users.iter().any(|u: &Value| u["id"].as_i64() == Some(id)) {
                    continue;
                }
                if let Some(u) = all.iter().find(|u| u["id"].as_i64() == Some(id)) {
                    let mut basic = serde_json::Map::new();
                    for key in ["id", "username", "name", "avatar_template"] {
                        if let Some(v) = u.get(key) {
                            basic.insert(key.into(), v.clone());
                        }
                    }
                    users.push(Value::Object(basic));
                }
            }
        }
        let mut notes = Notifications {
            conn: &mut *conn,
            settings,
            guardian,
            urls: &urls,
        };
        let r = notes
            .for_user_menu(user_id, &[GROUP_MESSAGE_SUMMARY], true, limit)
            .await?;
        read = notes.filter_inaccessible_topics(r).await?;
    }
    Ok(json!({
        "unread_notifications": unread,
        "read_notifications": read,
        "topics": topics,
        "users": users,
    }))
}

/// GET /u/:username/user-menu-private-messages(.json)
pub async fn messages(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
) -> Result<Response, AppError> {
    let Some(user) = guardian.user().cloned() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let username = username.strip_suffix(".json").unwrap_or(&username);
    // username_equals_to?
    if user.username.to_lowercase() != username.to_lowercase() {
        return Ok(super::search::invalid_access(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !guardian.is_staff()
        && !guardian.in_setting_groups(&settings, "personal_message_enabled_groups")?
    {
        return Ok(super::search::invalid_access(&state));
    }
    let doc = messages_doc(&state, &mut conn, &settings, &guardian, user.id).await?;
    Ok(([(header::CACHE_CONTROL, "no-cache, no-store")], Json(doc)).into_response())
}

/// chat's slugifyChannel: the slug, else the title slugified (utilities'
/// slugify, else the title lower cased and dashed), a hundred characters
/// at most; "-" for none.
pub(crate) fn slugify_channel(slug: Option<&str>, title: &str) -> String {
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
