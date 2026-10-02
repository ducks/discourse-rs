//! Port of app/controllers/posts_controller.rb#create, #update,
//! #revisions and #latest_revision.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::posting::create::{self, NewPost, Outcome};
use crate::posting::revise::{self, Changes};
use crate::posting::revisions::{self, RevisionResult, Which};
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_view::{Options, TopicView};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// `create_params` that this slice does not write: refused, not ignored.
const UNPORTED_CREATE_PARAMS: [&str; 22] = [
    "archetype",
    "target_recipients",
    "target_usernames",
    "tags",
    "whisper",
    "no_bump",
    "shared_draft",
    "is_warning",
    "image_sizes",
    "embed_url",
    "created_at",
    "external_id",
    "auto_track",
    "visible",
    "unlist_topic",
    "draft_key",
    "locale",
    "topic_custom_fields",
    "meta_data",
    "nested_post",
    "skip_validations",
    "featured_link",
];

fn json_error(status: StatusCode, errors: Vec<String>) -> Response {
    (status, Json(json!({"errors": errors}))).into_response()
}

/// The scalar params, for the CSRF token lookup.
fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /posts(.json)
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
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    }
    if let Some(key) = UNPORTED_CREATE_PARAMS.iter().find(|k| p.contains_key(**k)) {
        tracing::warn!(param = key, "posts#create param not ported");
        return Err(
            Unsupported("posts#create params beyond raw, topic, title and category").into(),
        );
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let integer = |k: &str| params::integer(&p, k);
    let api = headers.contains_key(crate::session::api_key::HEADER_API_KEY);
    let args = NewPost {
        raw: params::string(&p, "raw").unwrap_or_default(),
        topic_id: integer("topic_id").map(|v| v as i32),
        title: params::string(&p, "title"),
        category: params::string(&p, "category"),
        reply_to_post_number: integer("reply_to_post_number").map(|v| v as i32),
        typing_duration_msecs: integer("typing_duration_msecs"),
        composer_open_duration_msecs: integer("composer_open_duration_msecs"),
        composer_version: integer("composer_version").map(|v| v as i32),
        user_agent: headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        email: None,
        // is_api?: an admin API key (the session middleware checked it).
        advance_draft: !api,
        first_post_checks: !api,
    };
    let cook_host = Host::from_state(&state);
    let ctx = Ctx {
        host: &cook_host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
    };
    match create::create(&state.pool, &ctx, &guardian, args).await? {
        Outcome::Created { post_id } => {
            let mut post =
                serialize_post(&state, &settings, &guardian, post_id, false, true).await?;
            // Rails serializes the post object it created, whose reads the
            // creator's post timing bumped only in the database.
            if let Value::Object(p) = &mut post {
                p.insert("reads".into(), json!(0));
                p.insert("readers_count".into(), json!(0));
            }
            Ok((StatusCode::OK, Json(post)).into_response())
        }
        Outcome::Invalid(errors) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"action": "create_post", "errors": errors})),
        )
            .into_response()),
        Outcome::Forbidden => Ok(super::search::invalid_access(&state)),
        Outcome::InvalidParameter(name) => Ok(super::search::invalid_parameters(&state, name)),
    }
}

/// PostSerializer for one post, as create and update answer.
pub(super) async fn serialize_post(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    post_id: i32,
    with_link_counts: bool,
    with_raw_and_draft_sequence: bool,
) -> Result<Value, AppError> {
    let mut conn = state.pool.acquire().await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let mut view = TopicView {
        conn: &mut conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        options: Options {
            page: 0,
            post_number: None,
        },
        post_types: Vec::new(),
    };
    Ok(view
        .serialize_single_post(post_id, with_link_counts, with_raw_and_draft_sequence)
        .await?)
}

/// Rails' `(.:format)` on the last segment.
fn strip_format(segment: &str) -> &str {
    segment.strip_suffix(".json").unwrap_or(segment)
}

/// PUT /posts/:id(.json)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let id = strip_format(&id).to_string();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state, uri.path()));
    }
    let Some(Value::Object(post_params)) = p.get("post").filter(|v| match v {
        Value::Object(m) => !m.is_empty(),
        _ => false,
    }) else {
        return Ok(json_error(
            StatusCode::BAD_REQUEST,
            vec!["param is missing or the value is empty or invalid: post".into()],
        ));
    };
    for key in post_params.keys() {
        if !matches!(
            key.as_str(),
            "raw" | "edit_reason" | "original_text" | "raw_old"
        ) {
            return Err(Unsupported("posts#update fields beyond raw and edit_reason").into());
        }
    }
    for key in ["title", "image_sizes", "bypass_bump"] {
        if p.contains_key(key) {
            return Err(Unsupported("posts#update title, image sizes and bypass_bump").into());
        }
    }
    let Ok(post_id) = id.parse::<i32>() else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let cook_host = Host::from_state(&state);
    let ctx = Ctx {
        host: &cook_host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
    };
    let exists: Option<(Option<i32>, bool, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT user_id, deleted_at IS NOT NULL, created_at FROM posts WHERE id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((author, deleted, created_at)) = exists else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    if deleted {
        if guardian.is_staff() {
            return Err(Unsupported("editing deleted posts").into());
        }
        return Ok(super::topics::not_found_response(&state, false));
    }
    // can_edit?: the visibility the guardian needs, without the 404.
    let can_edit = match revisions::find_post(&mut conn, &ctx, &guardian, post_id).await? {
        Some(access) => access.can_edit(&ctx, &guardian)?,
        None => false,
    };
    if !can_edit
        && author == guardian.user_id()
        && guardian.edit_time_limit_expired(&settings, created_at)?
    {
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            vec![ctx.t("too_late_to_edit")],
        ));
    }
    if !can_edit {
        return Ok(super::search::invalid_access(&state));
    }
    let raw = post_params.get("raw").and_then(params::scalar);
    let original_text = post_params
        .get("original_text")
        .or_else(|| post_params.get("raw_old"))
        .and_then(params::scalar)
        .filter(|t| !t.trim().is_empty());
    if let Some(original) = original_text {
        let current: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;
        if original != current {
            return Ok(json_error(
                StatusCode::CONFLICT,
                vec![ctx.t("edit_conflict")],
            ));
        }
    }
    drop(conn);
    let changes = Changes {
        raw,
        edit_reason: post_params.get("edit_reason").and_then(params::scalar),
    };
    match revise::revise(&state.pool, &ctx, &guardian, post_id, changes).await? {
        revise::Outcome::Invalid(errors) => {
            Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, errors))
        }
        revise::Outcome::Revised => {
            let post = serialize_post(&state, &settings, &guardian, post_id, true, true).await?;
            Ok((StatusCode::OK, Json(json!({"post": post}))).into_response())
        }
    }
}

/// GET /posts/:id/revisions/:revision(.json) and .../revisions/latest(.json)
pub async fn revision(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((id, revision)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let revision = strip_format(&revision);
    let which = if revision == "latest" {
        Which::Latest
    } else if !revision.is_empty() && revision.chars().all(|c| c.is_ascii_digit()) {
        Which::Number(revision.to_string())
    } else {
        // The route's `revision: /\d+/` constraint: no route matches.
        return Ok(super::topics::not_found_response(&state, false));
    };
    let Ok(post_id) = id.parse::<i32>() else {
        return Ok(super::topics::not_found_response(&state, false));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let cook_host = Host::from_state(&state);
    let ctx = Ctx {
        host: &cook_host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
    };
    Ok(
        match revisions::show(&mut conn, &ctx, &guardian, post_id, which).await? {
            RevisionResult::Found(json) => (StatusCode::OK, Json(json)).into_response(),
            RevisionResult::NotFound => super::topics::not_found_response(&state, false),
            RevisionResult::InvalidRevision => {
                super::search::invalid_parameters(&state, "revision")
            }
            RevisionResult::Forbidden => super::search::invalid_access(&state),
        },
    )
}
