//! Port of ReviewableClaimedTopicsController (POST
//! /reviewable_claimed_topics, DELETE /reviewable_claimed_topics/:id) and
//! ReviewableNotesController (POST /review/:reviewable_id/notes, DELETE
//! /review/:reviewable_id/notes/:note_id).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// ReviewableNote::MAX_CONTENT_LENGTH
const MAX_NOTE_LENGTH: usize = 2000;

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn success() -> Response {
    (StatusCode::OK, Json(json!({ "success": "OK" }))).into_response()
}

fn errors(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({ "errors": errors }))).into_response()
}

/// What the shared front lets through: the params, or the response that
/// ends the request.
enum Front {
    Params(Map<String, Value>),
    Stop(Response),
}

/// The shared front: CSRF, then requires_login.
fn front(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Front {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Front::Stop(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Front::Stop(super::login_required::not_logged_in(state));
    }
    Front::Params(p)
}

/// The id without its `.json`.
fn id_of(raw: &str) -> i64 {
    crate::ruby::to_i(raw.strip_suffix(".json").unwrap_or(raw))
}

/// `can_claim_reviewable_topic?(topic, automatic)` for a topic that
/// exists (with deleted ones): claiming on, or an automatic claim, and
/// `can_review_topic?`.
async fn can_claim(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
    automatic: bool,
) -> Result<bool, AppError> {
    if settings.get("reviewable_claiming")?.to_s() == "disabled" && !automatic {
        return Ok(false);
    }
    let can_see = guardian.is_staff()
        || crate::onebox::can_see_topic(&mut *conn, settings, guardian, topic_id).await?;
    Ok(guardian.can_review_topic(settings, can_see)?)
}

/// `Topic.with_deleted.find_by(id:)`
async fn topic_exists(conn: &mut sqlx::PgConnection, id: i64) -> Result<Option<i32>, sqlx::Error> {
    let Ok(id) = i32::try_from(id) else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT id FROM topics WHERE id = $1")
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// POST /reviewable_claimed_topics(.json)
pub async fn create_claim(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body, "POST") {
        Front::Params(p) => p,
        Front::Stop(response) => return Ok(response),
    };
    // params[:reviewable_claimed_topic][:topic_id]: a 500 without the hash.
    let Some(Value::Object(claim)) = p.get("reviewable_claimed_topic") else {
        return Err(
            Unsupported("a claim without its reviewable_claimed_topic hash (a 500)").into(),
        );
    };
    let topic_param = claim.get("topic_id").and_then(params::scalar);
    let automatic = claim.get("automatic").and_then(params::scalar).as_deref() == Some("true");
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let topic = match topic_param {
        Some(raw) if !raw.is_empty() => topic_exists(&mut tx, crate::ruby::to_i(&raw)).await?,
        _ => None,
    };
    // ensure_can_claim_reviewable_topic!: can_review_topic?(nil) is false.
    let allowed = match topic {
        Some(id) => can_claim(&mut tx, &settings, &guardian, id, automatic).await?,
        None => false,
    };
    let (Some(topic_id), true) = (topic, allowed) else {
        return Ok(super::search::invalid_access(&state));
    };
    let user = guardian
        .user()
        .ok_or(Unsupported("claiming anonymously"))?
        .clone();
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    if crate::reviewable_claims::claim(&mut tx, &ctx, &user, topic_id, automatic)
        .await?
        .is_none()
    {
        // rescue ActiveRecord::RecordInvalid
        return Ok(errors(
            StatusCode::CONFLICT,
            vec![ctx.t("reviewables.conflict")],
        ));
    }
    tx.commit().await?;
    Ok(success())
}

/// DELETE /reviewable_claimed_topics/:id(.json), :id the topic's.
pub async fn destroy_claim(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = match front(&state, &guardian, &headers, &uri, &body, "DELETE") {
        Front::Params(p) => p,
        Front::Stop(response) => return Ok(response),
    };
    let automatic = p.get("automatic").and_then(params::scalar).as_deref() == Some("true");
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let topic = topic_exists(&mut tx, id_of(&id)).await?;
    let allowed = match topic {
        Some(topic_id) => can_claim(&mut tx, &settings, &guardian, topic_id, automatic).await?,
        None => false,
    };
    let (Some(topic_id), true) = (topic, allowed) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let user = guardian
        .user()
        .ok_or(Unsupported("releasing a claim anonymously"))?
        .clone();
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    if let Some(claim) = crate::reviewable_claims::find(&mut tx, topic_id).await? {
        crate::reviewable_claims::release(&mut tx, &ctx, &claim, &user).await?;
    }
    tx.commit().await?;
    Ok(success())
}

/// ReviewableNotesController's before actions: ensure_can_see (staff;
/// category group moderators are not ported), then find_reviewable
/// (`Reviewable.viewable_by(current_user)`). Err is the response.
async fn find_reviewable(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
    raw_id: &str,
) -> Result<Result<i64, Response>, AppError> {
    if !guardian.is_staff() {
        if settings.get("enable_category_group_moderation")?.truthy() {
            return Err(Unsupported("the review queue for category group moderators").into());
        }
        return Ok(Err(super::search::invalid_access(state)));
    }
    let id = id_of(raw_id);
    if crate::reviewables::is_viewable(&mut *conn, settings, guardian, id).await? {
        Ok(Ok(id))
    } else {
        Ok(Err(super::topics::not_found_response(state)))
    }
}

/// POST /review/:reviewable_id/notes(.json)
pub async fn create_note(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(reviewable_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let id = match find_reviewable(&mut tx, &state, &settings, &guardian, &reviewable_id).await? {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };
    // params.require(:reviewable_note).permit(:content)
    let content = match p.get("reviewable_note") {
        Some(Value::Object(note)) if !note.is_empty() => {
            note.get("content").and_then(params::scalar)
        }
        Some(Value::Object(_)) | None => {
            return Ok(errors(
                StatusCode::BAD_REQUEST,
                vec!["param is missing or the value is empty or invalid: reviewable_note".into()],
            ));
        }
        Some(v) if params::scalar(v).is_some_and(|s| s.trim().is_empty()) => {
            return Ok(errors(
                StatusCode::BAD_REQUEST,
                vec!["param is missing or the value is empty or invalid: reviewable_note".into()],
            ));
        }
        Some(_) => return Err(Unsupported("a reviewable_note that isn't a hash (a 500)").into()),
    };
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("a note without its author"))?;
    // validates :content, presence: true, length: 1..MAX_CONTENT_LENGTH
    let mut messages = Vec::new();
    let text = content.unwrap_or_default();
    if crate::ruby::is_blank(&text) {
        messages.push(format!(
            "Content {}",
            state
                .i18n
                .t("activerecord.errors.messages.blank")
                .or_else(|| state.i18n.t("errors.messages.blank"))
                .unwrap_or("can't be blank")
        ));
    }
    if text.chars().count() > MAX_NOTE_LENGTH {
        let too_long = state
            .i18n
            .t_count(
                "errors.messages.too_long",
                MAX_NOTE_LENGTH as i64,
                &[("count", &MAX_NOTE_LENGTH.to_string())],
            )
            .unwrap_or_else(|| "is too long".to_string());
        messages.push(format!("Content {too_long}"));
    } else if text.is_empty() {
        let too_short = state
            .i18n
            .t_count("errors.messages.too_short", 1, &[("count", "1")])
            .unwrap_or_else(|| "is too short".to_string());
        messages.push(format!("Content {too_short}"));
    }
    if !messages.is_empty() {
        return Ok(errors(StatusCode::UNPROCESSABLE_ENTITY, messages));
    }
    let note: (i64, String, chrono::NaiveDateTime, chrono::NaiveDateTime) = sqlx::query_as(
        "INSERT INTO reviewable_notes (reviewable_id, user_id, content, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now()) RETURNING id, content, created_at, updated_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(&text)
    .fetch_one(&mut *tx)
    .await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let user = crate::user_penalties::basic_user(&mut tx, &ctx, user_id).await?;
    tx.commit().await?;
    // ReviewableNoteSerializer, root: false
    Ok((
        StatusCode::OK,
        Json(json!({
            "id": note.0,
            "content": note.1,
            "created_at": crate::topic_list::time_json(note.2),
            "updated_at": crate::topic_list::time_json(note.3),
            "user": user,
        })),
    )
        .into_response())
}

/// DELETE /review/:reviewable_id/notes/:note_id(.json)
pub async fn destroy_note(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((reviewable_id, note_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let id = match find_reviewable(&mut tx, &state, &settings, &guardian, &reviewable_id).await? {
        Ok(id) => id,
        Err(response) => return Ok(response),
    };
    let author: Option<i32> = sqlx::query_scalar(
        "SELECT user_id::integer FROM reviewable_notes WHERE id = $1 AND reviewable_id = $2",
    )
    .bind(id_of(&note_id))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(author) = author else {
        return Ok(super::topics::not_found_response(&state));
    };
    if Some(author) != guardian.user_id() && !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    sqlx::query("DELETE FROM reviewable_notes WHERE id = $1")
        .bind(id_of(&note_id))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(success())
}
