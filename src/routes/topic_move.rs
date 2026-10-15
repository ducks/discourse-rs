//! Port of TopicsController#move_posts: POST /t/:topic_id/move-posts,
//! through PostMover (crate::post_mover).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::post_mover::{self, MoveArgs, Moved};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::TopicCtx;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn json_error(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({"errors": errors}))).into_response()
}

/// A param's value as `params[key].present?` sees it.
fn present(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key)
        .and_then(params::scalar)
        .filter(|v| !v.trim().is_empty())
}

/// `params[key]` as a list of ids (`.map(&:to_i)`).
fn id_list(p: &Map<String, Value>, key: &str) -> Result<Option<Vec<i32>>, AppError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.is_empty() => Ok(None),
        Some(Value::Array(items)) => Ok(Some(
            items
                .iter()
                .map(|v| {
                    params::scalar(v)
                        .map(|s| crate::ruby::to_i(&s) as i32)
                        .ok_or(Unsupported("post ids of another shape"))
                })
                .collect::<Result<_, _>>()?,
        )),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(_) => Err(Unsupported("post ids that aren't a list").into()),
    }
}

/// `guardian.can_move_posts?(topic)`
pub(crate) async fn can_move_posts(
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<bool, AppError> {
    if guardian.is_silenced() {
        return Ok(false);
    }
    let Some(topic) = TopicCtx::load(&mut *conn, s, guardian, topic_id).await? else {
        return Ok(false);
    };
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let can_see = guardian.can_see_topic(s, &topic, true, &secure)?;
    if !guardian.can_perform_action_available_to_group_moderators(s, can_see)? {
        return Ok(false);
    }
    Ok(!(topic.private_message() && !guardian.is_staff()))
}

/// The answer to a move: render_topic_changes, or the error a raise
/// becomes.
pub(crate) async fn moved_response(state: &AppState, moved: Moved) -> Result<Response, AppError> {
    Ok(match moved {
        Moved::Done(topic_id) => {
            let slug: Option<String> = sqlx::query_scalar("SELECT slug FROM topics WHERE id = $1")
                .bind(topic_id)
                .fetch_one(&state.pool)
                .await?;
            let base = state.config.globals.relative_url_root();
            let url = match slug.filter(|s| !s.is_empty()) {
                Some(slug) => format!("{base}/t/{slug}/{topic_id}"),
                None => format!("{base}/t/{topic_id}"),
            };
            (StatusCode::OK, Json(json!({"success": true, "url": url}))).into_response()
        }
        Moved::Nothing => (StatusCode::OK, Json(json!({"success": false}))).into_response(),
        Moved::Forbidden => super::search::invalid_access(state),
        Moved::InvalidParameters(what) => super::search::invalid_parameters(state, what),
        Moved::Invalid(errors) => json_error(StatusCode::UNPROCESSABLE_ENTITY, errors),
    })
}

/// POST /t/:topic_id/move-posts(.json)
pub async fn move_posts(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let Some(post_ids) = id_list(&p, "post_ids")? else {
        return Ok(super::accounts::param_missing("post_ids"));
    };
    let topic_id = crate::ruby::to_i(&topic_id) as i32;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let s = &settings;
    if !can_move_posts(&mut conn, s, &guardian, topic_id).await? {
        return Ok(super::search::invalid_access(&state));
    }
    let title = present(&p, "title");
    if title.is_some() {
        // A new topic starts with a regular post.
        let first_type: Option<i32> = sqlx::query_scalar(
            "SELECT post_type FROM posts WHERE topic_id = $1 AND id = ANY($2) AND deleted_at IS NULL \
             ORDER BY post_number LIMIT 1",
        )
        .bind(topic_id)
        .bind(&post_ids)
        .fetch_optional(&mut *conn)
        .await?;
        if first_type != Some(crate::posting::post_types::REGULAR) {
            return Ok(json_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                vec![
                    "When moving posts to a new topic, the first post must be a regular post."
                        .into(),
                ],
            ));
        }
        if let Some(category) = present(&p, "category_id") {
            let category_id = crate::ruby::to_i(&category) as i32;
            if !crate::posting::revise_topic::can_create_topic_on_category(
                &mut conn,
                s,
                &guardian,
                category_id,
            )
            .await?
            {
                return Ok(super::search::invalid_access(&state));
            }
        }
    }
    // post_ids_including_replies
    let mut post_ids = post_ids;
    if let Some(reply_parents) = id_list(&p, "reply_post_ids")? {
        let replies: Vec<i32> =
            sqlx::query_scalar("SELECT reply_post_id FROM post_replies WHERE post_id = ANY($1)")
                .bind(&reply_parents)
                .fetch_all(&mut *conn)
                .await?;
        for id in replies {
            if !post_ids.contains(&id) {
                post_ids.push(id);
            }
        }
    }
    let archetype = present(&p, "archetype");
    let args = MoveArgs {
        post_ids,
        title,
        destination_topic_id: present(&p, "destination_topic_id")
            .map(|d| crate::ruby::to_i(&d) as i32),
        category_id: if archetype.is_some() {
            None
        } else {
            present(&p, "category_id").map(|c| crate::ruby::to_i(&c) as i32)
        },
        chronological_order: p
            .get("chronological_order")
            .and_then(params::scalar)
            .as_deref()
            == Some("true"),
        freeze_original: p.get("freeze_original").and_then(params::scalar).as_deref()
            == Some("true"),
        move_to_pm: archetype.as_deref() == Some("private_message"),
        participants: present(&p, "participants"),
        tags: p.get("tag_ids").is_some_and(|v| !blank(v))
            || p.get("tags").is_some_and(|v| !blank(v)),
    };
    drop(conn);
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let moved = post_mover::move_posts(&state.pool, &ctx, &guardian, topic_id, &args).await?;
    moved_response(&state, moved).await
}

/// `blank?` for a param.
fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}
