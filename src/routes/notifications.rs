//! Port of app/controllers/notifications_controller.rb#index.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::notifications::{self, Notifications};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

#[derive(Deserialize, Default)]
pub struct Params {
    recent: Option<String>,
    silent: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    filter: Option<String>,
    username: Option<String>,
    filter_by_types: Option<String>,
    bump_last_seen_reviewable: Option<String>,
}

/// GET /notifications(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<Params>,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let recent = params
        .recent
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty());
    let viewer = guardian.user().expect("authenticated");

    // The target: `username` only without `recent`; admins may read
    // anyone's, exact-case lookup.
    let (user_id, username) = match (&params.username, recent) {
        (Some(name), false) => {
            let row: Option<(i32, String)> =
                sqlx::query_as("SELECT id, username FROM users WHERE username = $1")
                    .bind(name)
                    .fetch_optional(&mut *conn)
                    .await?;
            let Some((id, username)) = row else {
                return Ok(super::topics::not_found_response(&state));
            };
            if !guardian.is_me(id) && !guardian.is_admin() {
                return Ok(super::search::invalid_access(&state));
            }
            (id, username)
        }
        _ => (viewer.id, viewer.username.clone()),
    };

    // fetch_limit_from_params: an integer within 0..=60.
    let default_limit = if recent { 15 } else { 60 };
    let limit = match params.limit.as_deref() {
        None => default_limit,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) if (0..=60).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let offset = params
        .offset
        .as_deref()
        .map(crate::ruby::to_i)
        .unwrap_or(0)
        .max(0);
    let types = match params.filter_by_types.as_deref().filter(|t| !t.is_empty()) {
        Some(raw) => match notifications::parse_types(raw) {
            Ok(types) => types,
            Err(message) => return Ok(super::search::invalid_parameters(&state, &message)),
        },
        None => Vec::new(),
    };
    let query = notifications::Query {
        recent,
        silent: params.silent.is_some(),
        limit,
        offset,
        filter: params.filter.clone(),
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
    .index(user_id, &username, &query)
    .await?;
    if bumped {
        crate::bus::publish_notifications_state(&state.bus, &mut conn, &settings, user_id).await?;
    }
    // bump_last_seen_reviewable! for the all notifications tab, which
    // lists the pending reviewables (Rails defers it past the response).
    if recent
        && query.types.is_empty()
        && !query.silent
        && params.bump_last_seen_reviewable.is_some()
        && guardian.is_staff()
        && !crate::readonly::mode(&mut conn).await?.readonly
    {
        crate::reviewables::bump_last_seen(&mut conn, &state.bus, &settings, &guardian).await?;
    }
    Ok((
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(doc),
    )
        .into_response())
}

/// `Notification.types[:group_message_summary]`
const GROUP_MESSAGE_SUMMARY: i32 = 16;

/// GET /notifications/totals (NotificationsController#totals):
/// UserNotificationTotalSerializer for the current user, with chat's
/// chat_notifications.
pub async fn totals(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let viewer = guardian.user().expect("authenticated");
    let user_id = viewer.id;
    let badge = crate::header_badge::Badge::load(&mut conn, &settings, &guardian).await?;
    let mut out = Map::new();
    out.insert("username".into(), json!(viewer.username));
    out.insert(
        "unread_notifications".into(),
        json!(badge.all_unread - badge.new_pms),
    );
    if guardian.in_setting_groups(&settings, "personal_message_enabled_groups")? {
        out.insert("unread_personal_messages".into(), json!(badge.new_pms));
    }
    if guardian.is_staff() {
        out.insert("unseen_reviewables".into(), json!(badge.unseen_reviewables));
    }
    let tracking = crate::topic_tracking_report::load(&mut conn, &settings, &guardian)
        .await?
        .unwrap_or_default();
    out.insert("topic_tracking".into(), tracking.report_totals());
    // group_inboxes: the unread group message summaries' data.
    let data: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT data FROM notifications WHERE user_id = $1 AND NOT read \
           AND notification_type = $2 ORDER BY id",
    )
    .bind(user_id)
    .bind(GROUP_MESSAGE_SUMMARY)
    .fetch_all(&mut *conn)
    .await?;
    let mut inboxes = Vec::new();
    for raw in data {
        let data: Value = serde_json::from_str(raw.as_deref().unwrap_or(""))?;
        inboxes.push(json!({
            "group_id": data["group_id"],
            "group_name": data["group_name"],
            "count": data["inbox_count"],
        }));
    }
    out.insert("group_inboxes".into(), Value::Array(inboxes));
    // chat's add_to_serializer(:user_notification_total, :chat_notifications)
    if crate::plugins::chat::enabled(&settings)?
        && crate::plugins::chat::can_chat(&mut conn, &settings, &guardian).await?
    {
        let chat_enabled: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT chat_enabled FROM user_options WHERE user_id = $1), TRUE)",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if chat_enabled {
            let total = crate::plugins::chat::channels::Context {
                conn: &mut conn,
                settings: &settings,
                i18n: &state.i18n,
                guardian: &guardian,
                base_path: state.config.globals.relative_url_root(),
                config: &state.config,
            }
            .unreads_total()
            .await?;
            out.insert("chat_notifications".into(), json!(total));
        }
    }
    Ok(Json(Value::Object(out)).into_response())
}
