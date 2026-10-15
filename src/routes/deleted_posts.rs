//! PostsController#deleted_posts: GET /posts/:username/deleted(.json),
//! a user's deleted posts for staff (AdminUserActionSerializer), which
//! the user.deletedPosts page (/u/:username/deleted-posts) lists.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// PostsController::DELETED_POSTS_MAX_LIMIT
const MAX_LIMIT: i64 = 100;
/// The route's and UserPostsStream's page size.
pub const DEFAULT_LIMIT: i64 = 60;

#[derive(sqlx::FromRow)]
struct Row {
    id: i32,
    created_at: NaiveDateTime,
    post_number: i32,
    cooked: String,
    topic_id: i32,
    hidden: bool,
    post_type: i32,
    deleted_at: Option<NaiveDateTime>,
    deleted_by_id: Option<i32>,
    reply_to_post_number: Option<i32>,
    slug: Option<String>,
    title: Option<String>,
    category_id: Option<i32>,
}

/// `user_posts(guardian, user_id, offset:, limit:).where.not(deleted_at:
/// nil)`: newest first; a moderator who isn't an admin sees those in
/// topics they may see, messages left out.
pub async fn posts(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    user_id: i32,
    offset: i64,
    limit: i64,
) -> Result<Vec<Value>, AppError> {
    if settings.get("enable_user_status")?.truthy() {
        return Err(Unsupported("user status on BasicUserSerializer (130)").into());
    }
    let moderator_only = guardian.is_moderator() && !guardian.is_admin();
    let allowed = if moderator_only {
        guardian.allowed_category_ids(&mut *conn, settings).await?
    } else {
        Vec::new()
    };
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT p.id, p.created_at, p.post_number, p.cooked, p.topic_id, p.hidden, p.post_type, \
                p.deleted_at, p.deleted_by_id, p.reply_to_post_number, t.slug, t.title, t.category_id \
         FROM posts p LEFT JOIN topics t ON t.id = p.topic_id \
         WHERE p.user_id = $1 AND p.deleted_at IS NOT NULL \
           AND (NOT $2 OR (t.archetype <> 'private_message' \
                AND (t.category_id IS NULL OR t.category_id = ANY($3)))) \
         ORDER BY p.created_at DESC OFFSET $4 LIMIT $5",
    )
    .bind(user_id)
    .bind(moderator_only)
    .bind(&allowed)
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let author: (String, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let enable_names = settings.get("enable_names")?.truthy();
    let author_avatar = crate::avatar::avatar_template(urls, user_id, &author.0, author.2, None)
        .map_err(crate::users::UsersError::from)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let mut p = Map::new();
        // PostItemExcerpt
        p.insert(
            "excerpt".into(),
            json!(crate::excerpt::excerpt(
                &r.cooked,
                300,
                &crate::excerpt::Options {
                    keep_emoji_images: true,
                    ..Default::default()
                },
            )),
        );
        if r.cooked.chars().count() > 300 {
            p.insert("truncated".into(), json!(true));
        }
        p.insert("id".into(), json!(r.id));
        p.insert("created_at".into(), json!(time_json(r.created_at)));
        p.insert("post_number".into(), json!(r.post_number));
        p.insert("post_id".into(), json!(r.id));
        if enable_names {
            p.insert("name".into(), json!(author.1));
        }
        p.insert("username".into(), json!(author.0));
        p.insert("avatar_template".into(), json!(author_avatar));
        p.insert("topic_id".into(), json!(r.topic_id));
        p.insert("slug".into(), json!(r.slug));
        p.insert("title".into(), json!(r.title));
        p.insert("category_id".into(), json!(r.category_id));
        p.insert("hidden".into(), json!(r.hidden));
        // Post.types: moderator_action 2, small_action 3
        p.insert(
            "moderator_action".into(),
            json!(r.post_type == 2 || r.post_type == 3),
        );
        p.insert("deleted".into(), json!(r.deleted_at.is_some()));
        p.insert("deleted_at".into(), json!(r.deleted_at.map(time_json)));
        p.insert(
            "deleted_by".into(),
            basic_user(&mut *conn, urls, enable_names, r.deleted_by_id).await?,
        );
        p.insert("reply_to_post_number".into(), json!(r.reply_to_post_number));
        // The post's first reply or response action (REPLY 5, RESPONSE 6).
        let action_type: Option<i32> = sqlx::query_scalar(
            "SELECT action_type FROM user_actions WHERE target_post_id = $1 AND action_type IN (5, 6) \
             ORDER BY id LIMIT 1",
        )
        .bind(r.id)
        .fetch_optional(&mut *conn)
        .await?;
        p.insert("action_type".into(), json!(action_type));
        out.push(Value::Object(p));
    }
    Ok(out)
}

/// BasicUserSerializer, null for nobody.
async fn basic_user(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    enable_names: bool,
    user_id: Option<i32>,
) -> Result<Value, AppError> {
    let Some(id) = user_id else {
        return Ok(Value::Null);
    };
    let row: Option<(String, Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((username, name, avatar)) = row else {
        return Ok(Value::Null);
    };
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("username".into(), json!(username));
    if enable_names {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(
            crate::avatar::avatar_template(urls, id, &username, avatar, None)
                .map_err(crate::users::UsersError::from)?
        ),
    );
    Ok(Value::Object(out))
}

/// GET /posts/:username/deleted(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let json_request = uri.path().ends_with(".json")
        || headers
            .get("x-requested-with")
            .is_some_and(|v| v.as_bytes() == b"XMLHttpRequest")
        || headers
            .get(axum::http::header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/json"));
    if !json_request {
        let bus_position = crate::bus::page_position(&state.bus).await?;
        return super::user_preferences::ember_not_found(
            &state,
            &mut conn,
            &settings,
            &guardian,
            &headers,
            bus_position,
        )
        .await;
    }
    // ensure_can_see_deleted_posts_for_user!
    if !guardian.is_staff() {
        return Ok(super::search::invalid_access(&state));
    }
    let p = crate::params::parse(uri.query(), &headers, &[]);
    let int = |key: &str| crate::params::string(&p, key);
    let offset = int("offset")
        .map(|v| crate::ruby::to_i(&v))
        .unwrap_or(0)
        .max(0);
    // fetch_limit_from_params(default: 60, max: 100)
    let limit = match int("limit") {
        None => DEFAULT_LIMIT,
        Some(v) => match v.trim().parse::<i64>() {
            Ok(n) if (0..=MAX_LIMIT).contains(&n) => n,
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };
    let username = username.strip_suffix(".json").unwrap_or(&username);
    let Some(user) = crate::user_updater::find_target(&mut conn, &guardian, username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let posts = posts(
        &mut conn, &settings, &urls, &guardian, user.id, offset, limit,
    )
    .await?;
    Ok(Json(Value::Array(posts)).into_response())
}
