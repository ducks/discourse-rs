//! Port of DraftsController#create, #show and #destroy: POST /drafts,
//! GET /drafts/:id and DELETE /drafts/:id (each also with .json).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::drafts::{self, NewDraft, Saved};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `render_json_error(title, status:, extras: {description:})`
fn error_with_description(
    state: &AppState,
    status: StatusCode,
    key: &str,
    description: &str,
) -> Response {
    let title = state
        .i18n
        .t(&format!("{key}.title"))
        .unwrap_or(key)
        .to_string();
    (
        status,
        Json(json!({ "errors": [title], "extras": { "description": description } })),
    )
        .into_response()
}

/// The shared front: CSRF and requires_login. The user's id, or the
/// response that ends the request.
fn front(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    p: &Map<String, Value>,
    method: &str,
) -> Result<i32, Box<Response>> {
    if !csrf_ok(state, headers, &form_pairs(p), uri.path(), method) {
        return Err(Box::new(bad_csrf()));
    }
    guardian
        .user_id()
        .ok_or_else(|| Box::new(super::login_required::not_logged_in(state, uri.path())))
}

/// POST /drafts
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let user_id = match front(&state, &guardian, &headers, &uri, &p, "POST") {
        Ok(id) => id,
        Err(response) => return Ok(*response),
    };
    let Some(key) = p
        .get("draft_key")
        .and_then(params::scalar)
        .filter(|k| !k.trim().is_empty())
    else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // data: a JSON string no longer than max_draft_length.
    let data = match p.get("data") {
        Some(Value::String(d)) if d.chars().count() as i64 <= s.get("max_draft_length")?.to_i() => {
            d.clone()
        }
        _ => return Ok(super::search::invalid_parameters(&state, "data")),
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&data) else {
        return Ok(super::search::invalid_parameters(&state, "data"));
    };
    if drafts::reached_max(&mut tx, &s, user_id, &key).await? {
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &s,
        };
        let description = state
            .i18n
            .t_with(
                "draft.too_many_drafts.description",
                &[("base_url", &urls.base_url()?)],
            )
            .unwrap_or_default();
        return Ok(error_with_description(
            &state,
            StatusCode::FORBIDDEN,
            "draft.too_many_drafts",
            &description,
        ));
    }
    let sequence = p
        .get("sequence")
        .and_then(params::scalar)
        .map(|v| crate::ruby::to_i(&v))
        .unwrap_or(0);
    let owner = p.get("owner").and_then(params::scalar);
    let force_save = p
        .get("force_save")
        .and_then(params::scalar)
        .is_some_and(|v| v == "true");
    let saved = drafts::create(
        &mut tx,
        &s,
        user_id,
        &NewDraft {
            key: &key,
            sequence,
            data: &data,
            owner: owner.as_deref(),
            force_save,
        },
    )
    .await?;
    Ok(match saved {
        Saved::Sequence(sequence) => {
            let mut body = json!({ "success": "OK", "draft_sequence": sequence });
            if let Some(user) = edit_conflict(&mut tx, &state, &s, &guardian, &parsed).await? {
                body["conflict_user"] = user;
            }
            tx.commit().await?;
            (StatusCode::OK, Json(body)).into_response()
        }
        Saved::OutOfSequence => {
            let description = state
                .i18n
                .t("draft.sequence_conflict_error.description")
                .unwrap_or_default()
                .to_string();
            error_with_description(
                &state,
                StatusCode::CONFLICT,
                "draft.sequence_conflict_error",
                &description,
            )
        }
    })
}

/// The edit conflict check for a draft of an edit: the post's last editor
/// when the post changed since the edit began (its raw, and for a first
/// post its title and tags).
async fn edit_conflict(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    s: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    data: &Value,
) -> Result<Option<Value>, AppError> {
    let text = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    };
    let editing = data
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|a| a.starts_with("edit"));
    let Some(post_id) = data.get("postId").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Some(original_text) = text("original_text").or_else(|| text("originalText")) else {
        return Ok(None);
    };
    if !editing {
        return Ok(None);
    }
    let post_id = match post_id {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(v) => crate::ruby::to_i(v),
        _ => 0,
    } as i32;
    // Draft.allowed_draft_posts_for_user: posts the user may see.
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    if crate::posting::revisions::find_post(&mut *conn, &ctx, guardian, post_id)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let (raw, post_number, last_editor_id, title, tagged): (
        String,
        i32,
        Option<i32>,
        String,
        bool,
    ) = sqlx::query_as(
        "SELECT p.raw, p.post_number, p.last_editor_id, t.title, \
                    EXISTS (SELECT 1 FROM topic_tags WHERE topic_id = t.id) \
             FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    let mut conflict = original_text != raw;
    if post_number == 1 {
        conflict = conflict || text("original_title").is_some_and(|t| t != title);
        if !conflict {
            let original_tags = data
                .get("original_tags")
                .and_then(Value::as_array)
                .is_some_and(|tags| !tags.is_empty());
            if tagged || original_tags {
                return Err(Unsupported("the edit conflict check on tags").into());
            }
        }
    }
    if !conflict {
        return Ok(None);
    }
    let Some(editor) = last_editor_id else {
        return Err(Unsupported("an edit conflict without a last editor").into());
    };
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    Ok(Some(
        crate::user_penalties::basic_user(&mut *conn, &ctx, editor).await?,
    ))
}

/// `DraftsController::INDEX_LIMIT`
const INDEX_LIMIT: i64 = 50;

/// GET /drafts: the user's drafts (`Draft.stream`), as the drafts menu
/// and the drafts page read them.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    };
    let query = params::parse_query(uri.query().unwrap_or_default());
    // fetch_limit_from_params(default: nil, max: INDEX_LIMIT); Draft.stream
    // takes 30 without one.
    let limit = match query.get("limit").and_then(params::scalar) {
        None => 30,
        Some(raw) => match raw.trim().parse::<i64>() {
            Ok(n) if (0..=INDEX_LIMIT).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let offset = query
        .get("offset")
        .and_then(params::scalar)
        .map(|o| crate::ruby::to_i(&o))
        .unwrap_or(0);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // ContentLocalization.translated_topic_title and the categories that
    // come along when they are lazy loaded.
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("draft titles with content localization").into());
    }
    if guardian.can_lazy_load_categories(&settings)? {
        return Err(Unsupported("the drafts' categories when lazy loaded").into());
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let drafts = drafts::stream(&mut conn, &urls, &guardian, user_id, offset, limit).await?;
    Ok(Json(json!({ "drafts": drafts })).into_response())
}

/// GET /drafts/:id
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    uri: Uri,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    };
    let key = id.strip_suffix(".json").unwrap_or(&id);
    if key.trim().is_empty() {
        return Ok(super::topics::not_found_response(&state, false));
    }
    if params::parse_query(uri.query().unwrap_or_default()).contains_key("sequence") {
        return Err(Unsupported("reading a draft at a given sequence").into());
    }
    let mut conn = state.pool.acquire().await?;
    let (draft, sequence) = drafts::show(&mut conn, user_id, key).await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "draft": draft, "draft_sequence": sequence })),
    )
        .into_response())
}

/// DELETE /drafts/:id
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let user_id = match front(&state, &guardian, &headers, &uri, &p, "DELETE") {
        Ok(id) => id,
        Err(response) => return Ok(*response),
    };
    // fetch_target_user: another user's drafts, for admins over the API.
    if p.contains_key("username") || p.contains_key("external_id") {
        return Err(Unsupported("clearing another user's drafts").into());
    }
    let key = id.strip_suffix(".json").unwrap_or(&id);
    let sequence = p
        .get("sequence")
        .and_then(params::scalar)
        .map(|v| crate::ruby::to_i(&v))
        .unwrap_or(0);
    let mut tx = state.pool.begin().await?;
    // Draft::OutOfSequence is rescued: success either way.
    drafts::clear(&mut tx, user_id, key, sequence).await?;
    tx.commit().await?;
    Ok((StatusCode::OK, Json(json!({ "success": "OK" }))).into_response())
}
