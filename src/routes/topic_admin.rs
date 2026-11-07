//! The topic admin menu's TopicsController actions: GET
//! /topics/feature_stats (the FeatureTopic modal's counts), PUT
//! /t/:topic_id/make-banner and remove-banner, PUT /t/:topic_id/recover,
//! PUT /t/:topic_id/change-timestamp and PUT
//! /t/:id/reset-bump-date(/:post_id).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::TopicCtx;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// A `\d+` route segment, or None (no route: not found).
fn digits(raw: &str) -> Option<i32> {
    (!raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()))
        .then(|| crate::ruby::to_i(raw) as i32)
}

/// The request's params, None when its CSRF token is refused.
fn params_of(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Option<Map<String, Value>> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), "PUT") {
        return None;
    }
    Some(p)
}

async fn settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// GET /topics/feature_stats(.json): how many topics are pinned globally,
/// in the category asked about, and banners, among the topics the viewer
/// may list.
pub async fn feature_stats(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse_query(uri.query().unwrap_or_default());
    // params[:category_id]&.to_i
    let category_id = p
        .get("category_id")
        .and_then(params::scalar)
        .map(|c| crate::ruby::to_i(&c) as i32);
    let mut conn = state.pool.acquire().await?;
    let s = settings(&state, &mut conn).await?;
    let allowed = guardian.allowed_category_ids(&mut conn, &s).await?;
    // Topic.listable_topics.secured(guardian)
    let listable = format!(
        "deleted_at IS NULL AND archetype <> 'private_message' AND {}",
        guardian.category_clause(&allowed, "category_id")
    );
    let (globally, banners, in_category): (i64, i64, i64) = sqlx::query_as(&format!(
        "SELECT \
           (SELECT COUNT(*) FROM topics WHERE {listable} AND visible AND pinned_globally \
              AND pinned_at IS NOT NULL), \
           (SELECT COUNT(*) FROM topics WHERE {listable} AND archetype = 'banner'), \
           (SELECT COUNT(*) FROM topics WHERE {listable} AND visible AND category_id = $1 \
              AND NOT pinned_globally AND pinned_at IS NOT NULL)"
    ))
    .bind(category_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut body = json!({
        "pinned_globally_count": globally,
        "banner_count": banners,
    });
    if category_id.is_some() {
        body["pinned_in_category_count"] = json!(in_category);
    }
    Ok(Json(body).into_response())
}

/// PUT /t/:topic_id/make-banner
pub async fn make_banner(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    banner(state, guardian, topic_id, headers, uri, body, true).await
}

/// PUT /t/:topic_id/remove-banner
pub async fn remove_banner(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    banner(state, guardian, topic_id, headers, uri, body, false).await
}

/// `Topic.find_by(id:)`, `ensure_can_banner_topic!`, then make_banner! or
/// remove_banner!; no login is required (the guardian refuses visitors).
async fn banner(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    topic_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    make: bool,
) -> Result<Response, AppError> {
    if params_of(&state, &headers, &uri, &body).is_none() {
        return Ok(bad_csrf());
    }
    let Some(topic_id) = digits(&topic_id) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    let s = settings(&state, &mut tx).await?;
    let topic = TopicCtx::load(&mut tx, &s, &guardian, topic_id)
        .await?
        .filter(|t| !t.trashed());
    let (Some(topic), Some(user)) = (topic, guardian.user()) else {
        return Ok(super::search::invalid_access(&state));
    };
    if !guardian.can_banner_topic(&topic) {
        return Ok(super::search::invalid_access(&state));
    }
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    if make {
        crate::topics::banner::make_banner(&mut tx, &ctx, topic.id, user.id).await?;
    } else {
        crate::topics::banner::remove_banner(&mut tx, &ctx, topic.id, user.id).await?;
    }
    tx.commit().await?;
    // render body: nil
    Ok(StatusCode::OK.into_response())
}

/// PUT /t/:topic_id/recover: the topic's first post recovered, and the
/// topic with it.
pub async fn recover(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = params_of(&state, &headers, &uri, &body) else {
        return Ok(bad_csrf());
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let Some(topic_id) = digits(&topic_id) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let context = p.get("context").and_then(params::scalar);
    let mut tx = state.pool.begin().await?;
    let s = settings(&state, &mut tx).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    use crate::post_destroyer::Outcome;
    let outcome = crate::post_destroyer::recover_topic(
        &mut tx,
        &ctx,
        &guardian,
        topic_id,
        context.as_deref(),
    )
    .await?;
    Ok(match outcome {
        Outcome::Done => {
            tx.commit().await?;
            // render body: nil
            StatusCode::OK.into_response()
        }
        Outcome::NotFound => super::topics::not_found_response(&state),
        Outcome::Forbidden => super::search::invalid_access(&state),
        Outcome::RateLimited(refused) => refused.into_response(&state.i18n),
    })
}

/// PUT /t/:topic_id/change-timestamp (TopicsController#change_timestamps):
/// staff move the topic and its posts to `timestamp`, seconds since the
/// epoch.
pub async fn change_timestamps(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = params_of(&state, &headers, &uri, &body) else {
        return Ok(bad_csrf());
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let Some(topic_id) = digits(&topic_id) else {
        return Ok(super::topics::not_found_response(&state));
    };
    // params.require(:timestamp).to_f
    let Some(timestamp) = p
        .get("timestamp")
        .and_then(params::scalar)
        .filter(|t| !t.is_empty())
    else {
        return Ok(super::accounts::param_missing("timestamp"));
    };
    let timestamp = crate::ruby::to_f(&timestamp);
    // ensure_can_change_post_timestamps!
    if !guardian.is_staff() {
        return Ok(super::search::invalid_access(&state));
    }
    let user = guardian
        .user()
        .ok_or(crate::Unsupported("changing timestamps anonymously"))?;
    let mut tx = state.pool.begin().await?;
    let s = settings(&state, &mut tx).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    use crate::topics::timestamp::Outcome;
    Ok(
        match crate::topics::timestamp::change(&mut tx, &ctx, user.id, topic_id, timestamp).await? {
            Outcome::Done => {
                tx.commit().await?;
                Json(json!({"success": "OK"})).into_response()
            }
            Outcome::NotFound => super::topics::not_found_response(&state),
            Outcome::Invalid => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"failed": "FAILED"})),
            )
                .into_response(),
        },
    )
}

/// PUT /t/:id/reset-bump-date
pub async fn reset_bump_date(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    reset_bump(state, guardian, id, None, headers, uri, body).await
}

/// PUT /t/:id/reset-bump-date/:post_id
pub async fn reset_bump_date_to_post(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path((id, post_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    reset_bump(state, guardian, id, Some(post_id), headers, uri, body).await
}

/// `ensure_can_update_bumped_at!`, the topic found and seen, then
/// `Topic#reset_bumped_at(post_id)`: the bump date the post's (any post
/// the id names), or else the last regular post's that is neither hidden
/// nor deleted by its author, or the first post's.
#[allow(clippy::too_many_arguments)]
async fn reset_bump(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    id: String,
    post_id: Option<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if params_of(&state, &headers, &uri, &body).is_none() {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let (Some(topic_id), post_id) = (
        digits(&id),
        match post_id.as_deref().map(digits) {
            None => None,
            Some(Some(post_id)) => Some(post_id),
            Some(None) => return Ok(super::topics::not_found_response(&state)),
        },
    ) else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !(guardian.is_staff() || guardian.has_trust_level(4)) {
        return Ok(super::search::invalid_access(&state));
    }
    let mut tx = state.pool.begin().await?;
    let s = settings(&state, &mut tx).await?;
    let Some(topic) = TopicCtx::load(&mut tx, &s, &guardian, topic_id)
        .await?
        .filter(|t| !t.trashed())
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    let secure = guardian.secure_category_ids(&mut tx, &s).await?;
    if !guardian.can_see_topic(&s, &topic, true, &secure)? {
        return Ok(super::search::invalid_access(&state));
    }
    match post_id {
        None => crate::post_destroyer::reset_bumped_at(&mut tx, topic.id).await?,
        Some(post_id) => {
            // save(validate: false): only when the date changes.
            sqlx::query(
                "UPDATE topics t SET bumped_at = p.created_at, updated_at = clock_timestamp() \
                 FROM posts p WHERE p.id = $2 AND p.deleted_at IS NULL \
                   AND t.id = $1 AND t.bumped_at IS DISTINCT FROM p.created_at",
            )
            .bind(topic.id)
            .bind(post_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    // render body: nil
    Ok(StatusCode::OK.into_response())
}
