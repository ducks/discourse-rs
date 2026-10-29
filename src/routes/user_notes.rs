//! discourse-user-notes' UserNotesController, mounted at /user_notes:
//! GET (a user's notes), POST (a new note) and DELETE /:id, for staff
//! while the plugin is enabled. GET /user-notes/:user_id is the notes
//! modal's HTML (crate::plugins::user_notes::view) for
//! static/js/user-notes.js, from the same document as GET /user_notes.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::plugins::user_notes::{self, Cx};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (k, v) in map {
        match v {
            Value::Object(inner) => {
                for (ik, iv) in inner {
                    if let Some(iv) = params::scalar(iv) {
                        out.push((format!("{k}[{ik}]"), iv));
                    }
                }
            }
            v => {
                if let Some(v) = params::scalar(v) {
                    out.push((k.clone(), v));
                }
            }
        }
    }
    out
}

/// requires_plugin, ensure_logged_in, ensure_staff: the refusal, if any.
fn refused(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<Option<Response>, AppError> {
    if !user_notes::enabled(settings)? {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    if guardian.is_anonymous() {
        return Ok(Some(super::login_required::not_logged_in(state)));
    }
    if !guardian.is_staff() {
        return Ok(Some(super::search::invalid_access(state)));
    }
    Ok(None)
}

/// `User.where(id: param).first`: the id and username.
async fn find_user(
    conn: &mut sqlx::PgConnection,
    param: Option<&Value>,
) -> Result<Option<(i32, String)>, AppError> {
    let Some(id) = param
        .and_then(params::scalar)
        .map(|s| crate::ruby::to_i(&s))
        .and_then(|i| i32::try_from(i).ok())
    else {
        return Ok(None);
    };
    Ok(
        sqlx::query_as("SELECT id, username FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await?,
    )
}

async fn settings_of(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// The index document for a user id param, or the response refusing it.
async fn index_doc(
    state: &AppState,
    guardian: &Guardian,
    user_id: Option<&Value>,
) -> Result<Result<Value, Response>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = settings_of(&mut conn, state).await?;
    if let Some(r) = refused(state, &settings, guardian)? {
        return Ok(Err(r));
    }
    let Some((_, username)) = find_user(&mut conn, user_id).await? else {
        return Ok(Err(super::topics::not_found_response(state)));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let logo = crate::admin_users::logo_small_url(&mut conn, &settings).await?;
    let cx = Cx {
        settings: &settings,
        urls: &urls,
        guardian,
        logo_small_url: logo.as_deref(),
    };
    // notes_for(params[:user_id]): the key from the param as given.
    let param = user_id.and_then(params::scalar).unwrap_or_default();
    Ok(Ok(
        user_notes::index_doc(&mut conn, &cx, &param, &username).await?
    ))
}

/// GET /user_notes
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    Ok(
        match index_doc(&state, &guardian, p.get("user_id")).await? {
            Ok(doc) => Json(doc).into_response(),
            Err(r) => r,
        },
    )
}

/// POST /user_notes
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings = settings_of(&mut tx, &state).await?;
    if let Some(r) = refused(&state, &settings, &guardian)? {
        return Ok(r);
    }
    // `params[:user_note][:raw]`: NoMethodError without user_note.
    let Some(Value::Object(note)) = p.get("user_note") else {
        return Err(std::io::Error::other("undefined method '[]' for nil").into());
    };
    let raw = note.get("raw").cloned().unwrap_or(Value::Null);
    let raw_text = params::scalar(&raw).unwrap_or_default();
    let max = settings.get("max_post_length")?.to_i();
    if raw_text.chars().count() as i64 > max {
        let message = state
            .i18n
            .t_with("user_notes.note_too_long", &[("max", &max.to_string())])
            .unwrap_or_default();
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [message] })),
        )
            .into_response());
    }
    let Some((user_id, _)) = find_user(&mut tx, note.get("user_id")).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let logo = crate::admin_users::logo_small_url(&mut tx, &settings).await?;
    let cx = Cx {
        settings: &settings,
        urls: &urls,
        guardian: &guardian,
        logo_small_url: logo.as_deref(),
    };
    let mut extras = Map::new();
    // `if post_id = params[:user_note][:post_id]`: kept as given when the
    // post is there for the guardian to see.
    if let Some(post_id) = note.get("post_id").filter(|v| !v.is_null())
        && user_notes::post_visible(&mut tx, &cx, post_id).await?
    {
        extras.insert("post_id".into(), post_id.clone());
    }
    if let Some(reviewable_id) = note.get("reviewable_id").filter(|v| !v.is_null()) {
        extras.insert("reviewable_id".into(), reviewable_id.clone());
    }
    let record = user_notes::add_note(
        &mut tx,
        user_id,
        raw,
        guardian.user_id().unwrap_or_default(),
        extras,
    )
    .await?;
    let doc = user_notes::serialize(&mut tx, &cx, &record).await?;
    tx.commit().await?;
    Ok(Json(doc).into_response())
}

/// DELETE /user_notes/:id
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings = settings_of(&mut tx, &state).await?;
    if let Some(r) = refused(&state, &settings, &guardian)? {
        return Ok(r);
    }
    let Some((user_id, _)) = find_user(&mut tx, p.get("user_id")).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !user_notes::can_delete(&settings, &guardian)? {
        return Ok(super::search::invalid_access(&state));
    }
    // `/:id(.:format)`: the id stops at the first dot.
    let id = id.split('.').next().unwrap_or_default();
    user_notes::remove_note(&mut tx, user_id, id).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// GET /user-notes/{user_id}: the notes modal's HTML, `post_id` the post
/// it was opened from.
pub async fn modal(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let doc = match index_doc(&state, &guardian, Some(&json!(user_id))).await? {
        Ok(doc) => doc,
        Err(r) => return Ok(r),
    };
    let settings = {
        let mut conn = state.pool.acquire().await?;
        settings_of(&mut conn, &state).await?
    };
    let Some(ui) = user_notes::view::UserNotesUi::load(&settings, guardian.is_staff())? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let post_id = params::integer(&p, "post_id").filter(|id| *id > 0);
    // DCookText: each note's raw cooked as the composer preview would.
    let host = crate::pretty_text::Host::from_state(&state);
    let opts = crate::pretty_text::MarkdownOptions::default();
    let mut cooked = Vec::new();
    for n in doc["user_notes"].as_array().into_iter().flatten() {
        let raw = params::scalar(&n["raw"]).unwrap_or_default();
        cooked.push(crate::pretty_text::cook(&host, &raw, &opts).await?);
    }
    let html = user_notes::view::modal(
        &state.i18n,
        &ui,
        settings.get("relative_date_duration")?.to_i(),
        state.config.globals.relative_url_root(),
        &user_id,
        post_id,
        &doc,
        &cooked,
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response())
}
