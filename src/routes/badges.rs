//! The badge pages' JSON: BadgesController#index and #show (GET
//! /badges, /badges/:id(/:slug)), UserBadgesController#index (GET
//! /user_badges) and #username (GET /user-badges/:username). Their HTML
//! (and the badge's RSS) is refused.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::badges_view::{
    BadgeRow, BadgesView, GROUPED_COLUMNS, Side, USER_BADGE_COLUMNS, UserBadgeRow, badge_json,
    badge_type_json, detailed_document,
};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::users::User;
use crate::{AppError, AppState, Unsupported};

/// `UserBadgesController::MAX_BADGES`
const MAX_BADGES: i64 = 96;

/// The `.json` a path segment ends with: the segment and whether it did.
fn strip_json(segment: &str) -> (&str, bool) {
    match segment.strip_suffix(".json") {
        Some(s) => (s, true),
        None => (segment, false),
    }
}

/// after_action :add_noindex_header
fn noindex(doc: Value) -> Response {
    ([("x-robots-tag", "noindex")], Json(doc)).into_response()
}

/// GET /badges(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    let json = uri.path().ends_with(".json") || super::wants_json(uri.query(), &headers);
    if !json {
        return Err(Unsupported("the badges page (HTML)").into());
    }
    let p = params::parse(uri.query(), &headers, b"");
    let xhr = headers
        .get("x-requested-with")
        .is_some_and(|v| v == "XMLHttpRequest");
    let only_listable = p
        .get("only_listable")
        .and_then(params::scalar)
        .is_some_and(|v| v == "true");
    // name ILIKE "%#{search}%": the search's own % and _ stay wildcards.
    let search = p.get("search").and_then(params::scalar);
    let listable = if !guardian.is_staff() || only_listable || !xhr {
        "AND b.enabled AND b.listable"
    } else {
        ""
    };
    // Badge.all: in the table's own order (sorted client side), read
    // without the joins that would reorder it.
    let ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT b.id FROM badges b WHERE ($1::text IS NULL OR b.name ILIKE '%' || $1 || '%') {listable}"
    ))
    .bind(search)
    .fetch_all(&mut *conn)
    .await?;
    let mut loaded: Vec<BadgeRow> = sqlx::query_as(&format!(
        "{} WHERE b.id = ANY($1)",
        crate::badges_view::BADGE_ROW_SELECT,
    ))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut rows = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Some(i) = loaded.iter().position(|b| b.id == *id) {
            rows.push(loaded.swap_remove(i));
        }
    }
    // current_user.user_badges (enabled badges), distinct badge ids
    let has: Option<Vec<i32>> = match guardian.user_id() {
        Some(uid) => Some(
            sqlx::query_scalar(
                "SELECT DISTINCT badge_id FROM user_badges WHERE user_id = $1 \
                 AND badge_id IN (SELECT id FROM badges WHERE enabled)",
            )
            .bind(uid)
            .fetch_all(&mut *conn)
            .await?,
        ),
        None => None,
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let base_path = state.config.globals.relative_url_root();
    let mut badges = Vec::new();
    let mut types: Vec<Value> = Vec::new();
    let mut type_ids: Vec<i32> = Vec::new();
    let mut groupings: Vec<Value> = Vec::new();
    let mut grouping_ids: Vec<i32> = Vec::new();
    for b in &rows {
        // BadgeIndexSerializer#has_badge, when there is a current user.
        let has_badge = has.as_ref().map(|h| h.contains(&b.id));
        badges.push(badge_json(
            &state.i18n,
            &settings,
            &urls,
            base_path,
            b,
            true,
            has_badge,
        )?);
        if !type_ids.contains(&b.badge_type_id) {
            type_ids.push(b.badge_type_id);
            types.push(badge_type_json(b.badge_type_id, &b.badge_type_name));
        }
        if !grouping_ids.contains(&b.badge_grouping_id) {
            grouping_ids.push(b.badge_grouping_id);
            if let Some(g) =
                crate::badges_view::badge_grouping_json(&mut conn, b.badge_grouping_id).await?
            {
                groupings.push(g);
            }
        }
    }
    Ok(noindex(json!({
        "badge_types": types,
        "badge_groupings": groupings,
        "badges": badges,
    })))
}

/// GET /badges/:id
pub async fn show(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (id, json) = strip_json(&id);
    show_badge(state, guardian, id.to_string(), json, headers, uri).await
}

/// GET /badges/:id/:slug
pub async fn show_with_slug(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path((id, slug)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (_, json) = strip_json(&slug);
    if slug.ends_with(".rss") {
        return Err(Unsupported("the badge RSS feed").into());
    }
    show_badge(state, guardian, id, json, headers, uri).await
}

/// BadgesController#show: the enabled badge, the viewer's grant
/// notification marked read.
async fn show_badge(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    id: String,
    json: bool,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if id.ends_with(".rss") {
        return Err(Unsupported("the badge RSS feed").into());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    // Badge.enabled.find(params[:id])
    let id = i32::try_from(crate::ruby::to_i(&id)).unwrap_or(0);
    let enabled: Option<bool> = sqlx::query_scalar("SELECT enabled FROM badges WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    if enabled != Some(true) {
        return Ok(super::topics::not_found_response(&state));
    }
    if !json && !super::wants_json(uri.query(), &headers) {
        return Err(Unsupported("the badge page (HTML)").into());
    }
    let mut has_badge = false;
    if let Some(uid) = guardian.user_id() {
        // UserBadge.find_by(user_id:, badge_id:): the first grant.
        let grant: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT notification_id FROM user_badges WHERE user_id = $1 AND badge_id = $2 \
             ORDER BY id LIMIT 1",
        )
        .bind(uid)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(notification_id) = grant {
            has_badge = true;
            if let Some(nid) = notification_id {
                // notification.update(read: true): a change writes and
                // refreshes the count.
                let owner: Option<i32> = sqlx::query_scalar(
                    "UPDATE notifications SET read = TRUE, updated_at = clock_timestamp() \
                     WHERE id = $1 AND NOT read RETURNING user_id",
                )
                .bind(nid)
                .fetch_optional(&mut *tx)
                .await?;
                if let Some(owner) = owner {
                    crate::bus::publish_notifications_state(&state.bus, &mut tx, &settings, owner)
                        .await?;
                }
            }
        }
    }
    let badge = BadgeRow::load(&mut tx, id).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let body = badge_json(
        &state.i18n,
        &settings,
        &urls,
        state.config.globals.relative_url_root(),
        &badge,
        true,
        has_badge.then_some(true),
    )?;
    tx.commit().await?;
    Ok(noindex(json!({
        "badge_types": [badge_type_json(badge.badge_type_id, &badge.badge_type_name)],
        "badge": body,
    })))
}

/// `fetch_badge_from_params`: by badge_name, else the required badge_id;
/// enabled badges only. Err is the response.
async fn fetch_badge(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    p: &Map<String, Value>,
) -> Result<Result<i32, Response>, AppError> {
    let id: Option<i32> = match p.get("badge_name").and_then(params::scalar) {
        Some(name) => {
            sqlx::query_scalar(
                "SELECT id FROM badges WHERE name = $1 AND enabled ORDER BY id LIMIT 1",
            )
            .bind(name)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => {
            let Some(raw) = p
                .get("badge_id")
                .and_then(params::scalar)
                .filter(|v| !v.trim().is_empty())
            else {
                return Ok(Err(super::accounts::param_missing("badge_id")));
            };
            let raw = i32::try_from(crate::ruby::to_i(&raw)).unwrap_or(0);
            sqlx::query_scalar("SELECT id FROM badges WHERE id = $1 AND enabled")
                .bind(raw)
                .fetch_optional(&mut *conn)
                .await?
        }
    };
    Ok(id.ok_or_else(|| super::topics::not_found_response(state)))
}

/// GET /user_badges(.json)
pub async fn user_badges_index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, b"");
    let badge_id = match fetch_badge(&state, &mut conn, &p).await? {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };
    let username = p.get("username").and_then(params::scalar);
    let mut user_id = None;
    let mut grant_count = None;
    if let Some(name) = &username {
        // fetch_user_from_params(include_inactive: true), can_see_profile?
        let name = name.strip_suffix(".json").unwrap_or(name);
        let Some(user) = User::find_any(&mut conn, name).await? else {
            return Ok(super::topics::not_found_response(&state));
        };
        if !user.visible_to(&settings, &guardian)? {
            return Ok(super::topics::not_found_response(&state));
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM user_badges WHERE badge_id = $1 AND user_id = $2",
        )
        .bind(badge_id)
        .bind(user.id)
        .fetch_one(&mut *conn)
        .await?;
        user_id = Some(user.id);
        grant_count = Some(count);
    }
    // fetch_int_from_params(:offset, default: 0)
    let offset = match p.get("offset").and_then(params::scalar) {
        None => 0,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) => n.max(0),
            Err(_) => return Ok(super::search::invalid_parameters(&state, "offset")),
        },
    };
    // granted_before is permitted but never read.
    let rows: Vec<UserBadgeRow> = sqlx::query_as(&format!(
        "SELECT {USER_BADGE_COLUMNS} FROM user_badges \
         WHERE badge_id = $1 AND ($2::int IS NULL OR user_id = $2) \
         ORDER BY granted_at DESC, id DESC LIMIT $3 OFFSET $4"
    ))
    .bind(badge_id)
    .bind(user_id)
    .bind(MAX_BADGES)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut view = BadgesView {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
    };
    let allowed = view.allowed_topic_ids(&rows).await?;
    let mut side = Side::default();
    let mut items = Vec::new();
    for row in &rows {
        items.push(view.user_badge_json(&mut side, row, &allowed).await?);
    }
    let mut doc = Map::new();
    doc.insert("badges".into(), Value::Array(side.badges));
    doc.insert("badge_types".into(), Value::Array(side.badge_types));
    doc.insert("users".into(), Value::Array(side.users));
    if !side.topics.is_empty() {
        doc.insert("topics".into(), Value::Array(side.topics));
    }
    doc.insert(
        "user_badge_info".into(),
        json!({"grant_count": grant_count, "username": username, "user_badges": items}),
    );
    Ok(Json(Value::Object(doc)).into_response())
}

/// GET /user-badges/:username(.json)
pub async fn username(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (username, json) = strip_json(&username);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_badges")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    if !json && !super::wants_json(uri.query(), &headers) {
        return Err(Unsupported("the user badges page (HTML)").into());
    }
    // fetch_user_from_params(include_inactive: staff, or a user with
    // show_inactive_accounts)
    let include_inactive = guardian.is_staff()
        || (guardian.is_authenticated() && settings.get("show_inactive_accounts")?.truthy());
    let user = if include_inactive
        || guardian
            .user()
            .is_some_and(|u| u.username.eq_ignore_ascii_case(username))
    {
        User::find_any(&mut conn, username).await?
    } else {
        User::find_active(&mut conn, username).await?
    };
    let Some(user) = user else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !user.visible_to(&settings, &guardian)? {
        return Ok(super::topics::not_found_response(&state));
    }
    let p = params::parse(uri.query(), &headers, b"");
    let grouped = p
        .get("grouped")
        .and_then(params::scalar)
        .is_some_and(|v| !v.is_empty());
    // user.user_badges: the enabled badges', in the database's order.
    let sql = if grouped {
        format!(
            "SELECT {GROUPED_COLUMNS} FROM user_badges WHERE user_badges.user_id = $1 \
             AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled)) \
             GROUP BY user_badges.badge_id"
        )
    } else {
        format!(
            "SELECT {USER_BADGE_COLUMNS} FROM user_badges WHERE user_badges.user_id = $1 \
             AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled))"
        )
    };
    let rows: Vec<UserBadgeRow> = sqlx::query_as(&sql)
        .bind(user.id)
        .fetch_all(&mut *conn)
        .await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut view = BadgesView {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: state.config.globals.relative_url_root(),
    };
    let allowed = view.allowed_topic_ids(&rows).await?;
    let mut side = Side::default();
    let mut items = Vec::new();
    for row in &rows {
        items.push(
            view.detailed_user_badge(&mut side, row, None, Some(&allowed))
                .await?,
        );
    }
    Ok(Json(detailed_document(side, "user_badges", Value::Array(items))).into_response())
}
