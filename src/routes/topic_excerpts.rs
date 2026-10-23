//! GET /t/:topic_id/excerpts(.json): TopicsController#excerpts, the posts'
//! excerpts the mobile view's fullscreen timeline shows for the post at
//! its scroller (PostStream#backfillExcerpts).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// A post's id, cooked, author, action code and creation.
type ExcerptRow = (
    i32,
    String,
    Option<String>,
    Option<String>,
    chrono::NaiveDateTime,
);

/// GET /t/{topic_id}/excerpts
pub async fn excerpts(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(topic_id): Path<String>,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = crate::params::parse_query(uri.query().unwrap_or_default());
    // params.require(:post_ids): a blank one is missing too.
    let post_ids = match p.get("post_ids") {
        None => return Ok(super::accounts::param_missing("post_ids")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("post_ids"));
        }
        Some(Value::Array(list)) if list.is_empty() => {
            return Ok(super::accounts::param_missing("post_ids"));
        }
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                crate::params::scalar(v)
                    .map(|s| crate::ruby::to_i(&s))
                    .unwrap_or(0)
            })
            .collect::<Vec<i64>>(),
        Some(_) => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"errors": ["Expecting post_ids to contain a list of posts ids"]})),
            )
                .into_response());
        }
    };
    if post_ids.len() > 100 {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": ["Requested a chunk that is too big"]})),
        )
            .into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // Topic.with_deleted.where(id:).first, then ensure_can_see!.
    let topic_id = crate::ruby::to_i(&topic_id) as i32;
    let visible =
        match crate::topic_guardian::TopicCtx::load(&mut conn, &settings, &guardian, topic_id)
            .await?
        {
            Some(topic) => {
                let secure = guardian.secure_category_ids(&mut conn, &settings).await?;
                guardian.can_see_topic(&settings, &topic, true, &secure)?
            }
            None => false,
        };
    if !visible {
        return Ok(super::search::invalid_access(&state));
    }
    // Post.secured: whispers for whisperers only.
    let whisperer = guardian.is_whisperer(&settings)?;
    let rows: Vec<ExcerptRow> = sqlx::query_as(
        "SELECT posts.id, posts.cooked, u.username, posts.action_code, posts.created_at \
             FROM posts LEFT JOIN users u ON u.id = posts.user_id \
             WHERE posts.hidden = FALSE AND posts.deleted_at IS NULL AND posts.topic_id = $1 \
             AND posts.id = ANY($2) AND ($3 OR posts.post_type <> 4) ORDER BY posts.id",
    )
    .bind(topic_id)
    .bind(post_ids.iter().map(|id| *id as i32).collect::<Vec<i32>>())
    .bind(whisperer)
    .fetch_all(&mut *conn)
    .await?;
    let plugins = crate::excerpt::Plugins::from_settings(&settings)?;
    let options = crate::excerpt::Options {
        keep_emoji_images: true,
        ..Default::default()
    };
    let out: Vec<Value> = rows
        .into_iter()
        .map(|(id, cooked, username, action_code, created_at)| {
            let mut attrs = serde_json::Map::new();
            attrs.insert("post_id".into(), json!(id));
            attrs.insert("username".into(), json!(username));
            attrs.insert(
                "excerpt".into(),
                json!(crate::excerpt::excerpt(&cooked, 800, &options, plugins)),
            );
            if let Some(code) = action_code {
                attrs.insert("action_code".into(), json!(code));
                attrs.insert(
                    "created_at".into(),
                    json!(crate::topic_list::time_json(created_at)),
                );
            }
            Value::Object(attrs)
        })
        .collect();
    Ok(Json(Value::Array(out)).into_response())
}
