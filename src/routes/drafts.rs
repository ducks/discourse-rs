//! Port of DraftsController#create, #show, #destroy and #bulk_destroy:
//! POST /drafts, GET /drafts/:id, DELETE /drafts/:id and DELETE
//! /drafts/bulk_destroy (each also with .json).

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
        .ok_or_else(|| Box::new(super::login_required::not_logged_in(state)))
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
        return Ok(super::topics::not_found_response(&state));
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
        // The tags: the draft's original ones (tag objects by id, or names
        // in the old format) against the topic's less those hidden from
        // the editor, whose editor never showed them.
        if !conflict {
            let original: Vec<Value> = match data.get("original_tags") {
                Some(Value::Array(tags)) => tags.clone(),
                None | Some(Value::Null) => Vec::new(),
                Some(_) => return Err(Unsupported("original_tags of another shape").into()),
            };
            let mut ids: Vec<Value> = original
                .iter()
                .filter_map(|t| t.as_object().and_then(|o| o.get("id")).cloned())
                .filter(|id| !id.is_null())
                .collect();
            let names: Vec<String> = original
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect();
            if !names.is_empty() {
                let found: Vec<i32> =
                    sqlx::query_scalar("SELECT id FROM tags WHERE name = ANY($1)")
                        .bind(&names)
                        .fetch_all(&mut *conn)
                        .await?;
                ids.extend(found.into_iter().map(|id| json!(id)));
            }
            let mut current: Vec<Value> = if tagged {
                let hidden = crate::tagging::hidden_tag_names(&mut *conn, guardian).await?;
                sqlx::query_scalar::<_, i32>(
                    "SELECT tags.id FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
                     JOIN posts p ON p.topic_id = topic_tags.topic_id \
                     WHERE p.id = $1 AND NOT (tags.name = ANY($2))",
                )
                .bind(post_id)
                .bind(&hidden)
                .fetch_all(&mut *conn)
                .await?
                .into_iter()
                .map(|id| json!(id))
                .collect()
            } else {
                Vec::new()
            };
            // to_set: compared without order or repeats.
            let key = |v: &Value| v.to_string();
            ids.sort_by_key(key);
            ids.dedup();
            current.sort_by_key(key);
            current.dedup();
            conflict = ids != current;
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
        return Ok(super::login_required::not_logged_in(&state));
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
        return Ok(super::login_required::not_logged_in(&state));
    };
    let key = id.strip_suffix(".json").unwrap_or(&id);
    if key.trim().is_empty() {
        return Ok(super::topics::not_found_response(&state));
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

/// `DraftsController::BULK_DESTROY_LIMIT`
const BULK_DESTROY_LIMIT: usize = 30;

/// DELETE /drafts/bulk_destroy: the user's drafts at the given keys, when
/// every given sequence is the current one.
pub async fn bulk_destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let user_id = match front(&state, &guardian, &headers, &uri, &p, "DELETE") {
        Ok(id) => id,
        Err(response) => return Ok(*response),
    };
    // params.require(:draft_keys)
    let keys: Vec<String> = match p.get("draft_keys") {
        Some(Value::Array(keys)) if !keys.is_empty() => {
            keys.iter().filter_map(params::scalar).collect()
        }
        Some(Value::String(s)) if !s.trim().is_empty() => {
            return Err(Unsupported("draft_keys given as a string").into());
        }
        _ => return Ok(super::accounts::param_missing("draft_keys")),
    };
    if keys.len() > BULK_DESTROY_LIMIT {
        let message = state
            .i18n
            .t_with(
                "draft.bulk_destroy_limit",
                &[("limit", &BULK_DESTROY_LIMIT.to_string())],
            )
            .unwrap_or_default();
        return Ok(super::search::invalid_parameters(&state, &message));
    }
    if p.contains_key("username") || p.contains_key("external_id") {
        return Err(Unsupported("clearing another user's drafts").into());
    }
    let empty = Map::new();
    let sequences = match p.get("sequences") {
        Some(Value::Object(map)) => map,
        None => &empty,
        Some(_) => return Err(Unsupported("sequences given as other than a hash").into()),
    };
    let mut tx = state.pool.begin().await?;
    match drafts::bulk_destroy(&mut tx, user_id, &keys, sequences).await? {
        Ok(count) => {
            tx.commit().await?;
            Ok(Json(json!({ "success": "OK", "deleted_count": count })).into_response())
        }
        Err(conflicts) => Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "failed": "FAILED",
                "errors": format!("Draft sequence conflict for keys: {}", conflicts.join(", ")),
            })),
        )
            .into_response()),
    }
}

/// GET /drafts/bulk_destroy: drafts#show with that id.
pub async fn show_bulk_destroy(
    state: State<AppState>,
    guardian: AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let id = if uri.path().ends_with(".json") {
        "bulk_destroy.json"
    } else {
        "bulk_destroy"
    };
    show(state, guardian, Path(id.to_string()), uri).await
}
