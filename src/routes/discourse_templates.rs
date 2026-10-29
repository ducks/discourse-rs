//! discourse-templates' TemplatesController, mounted at
//! /discourse_templates: GET (the member's templates) and POST /:id/use
//! (the usage count). GET /live/templates is the filterable list's HTML
//! (crate::plugins::templates::view) for static/js/templates.js, filtered
//! by keyword and tag, with the variables of the insert's context
//! replaced.

use std::collections::HashMap;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::plugins::templates::variables::Context;
use crate::plugins::templates::{self, view};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// requires_plugin, ensure_logged_in, ensure_discourse_templates_enabled
/// (`guardian.can_use_templates?`): the refusal, if any.
async fn refused(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<Option<Response>, AppError> {
    if !templates::enabled(settings)? {
        return Ok(Some(super::topics::not_found_response(state)));
    }
    if guardian.is_anonymous() {
        return Ok(Some(super::login_required::not_logged_in(state)));
    }
    if !templates::can_use_templates(conn, settings, guardian).await? {
        return Ok(Some(super::search::invalid_access(state)));
    }
    Ok(None)
}

/// `render_json_error(message, status: 422)`
fn json_error(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": [message] })),
    )
        .into_response()
}

/// GET /discourse_templates: the category templates, then the private
/// ones, as TemplatesSerializer has them.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(r) = refused(&mut conn, &state, &settings, &guardian).await? {
        return Ok(r);
    }
    let list = templates::list(&mut conn, &settings, &guardian).await?;
    let items: Vec<_> = list.iter().map(templates::Template::json).collect();
    Ok(Json(json!({ "templates": items })).into_response())
}

/// POST /discourse_templates/:id/use
pub async fn use_template(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = p
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(r) = refused(&mut tx, &state, &settings, &guardian).await? {
        return Ok(r);
    }
    // Topic.find_by(id:), trashed topics out of its default scope.
    let topic_id = i32::try_from(crate::ruby::to_i(&id)).unwrap_or(0);
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Ok(json_error("Invalid template id"));
    }
    // guardian.ensure_can_see!(topic)
    let ctx = crate::topic_guardian::TopicCtx::load(&mut tx, &settings, &guardian, topic_id)
        .await?
        .ok_or(crate::Unsupported("a topic gone while it was read"))?;
    let secure = guardian.secure_category_ids(&mut tx, &settings).await?;
    if !guardian.can_see_topic(&settings, &ctx, true, &secure)? {
        return Ok(super::search::invalid_access(&state));
    }
    if !templates::is_template(&mut tx, &settings, topic_id, guardian.user_id()).await? {
        return Ok(json_error("Id does not belong to a template"));
    }
    let record = templates::increment_usage_count(&mut tx, topic_id).await?;
    tx.commit().await?;
    Ok(Json(json!(record)).into_response())
}

#[derive(serde::Deserialize, Default)]
pub struct ListParams {
    filter: Option<String>,
    tag: Option<String>,
    /// `composer` or `chat`; any other textarea without.
    context: Option<String>,
    topic_id: Option<String>,
    post_number: Option<String>,
    chat_channel_id: Option<String>,
    chat_reply_to_id: Option<String>,
}

fn int(v: &Option<String>) -> Option<i64> {
    v.as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(crate::ruby::to_i)
        .filter(|i| *i > 0)
}

impl ListParams {
    fn context(&self) -> Context {
        match self.context.as_deref() {
            Some("composer") => Context::Composer {
                topic_id: int(&self.topic_id).and_then(|i| i32::try_from(i).ok()),
                post_number: int(&self.post_number).and_then(|i| i32::try_from(i).ok()),
            },
            Some("chat") => match int(&self.chat_channel_id) {
                Some(channel_id) => Context::Chat {
                    channel_id,
                    reply_to_id: int(&self.chat_reply_to_id),
                },
                None => Context::None,
            },
            _ => Context::None,
        }
    }
}

/// GET /live/templates: the filterable list, loaded (what
/// FilterableList#load fetches from GET /discourse_templates, drawn), for
/// the keyword and tag asked for.
pub async fn list(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(r) = refused(&mut conn, &state, &settings, &guardian).await? {
        return Ok(r);
    }
    let all = templates::list(&mut conn, &settings, &guardian).await?;
    let context = params.context();
    let variables = templates::variables::load(
        &mut conn,
        &settings,
        &state.i18n,
        &state.config,
        &guardian,
        &context,
    )
    .await?;
    drop(conn);
    let tag_style = settings.get("tag_style")?.to_s().to_string();
    let cx = view::Cx {
        i18n: &state.i18n,
        base_path: state.config.globals.relative_url_root(),
        tagging: settings.get("tagging_enabled")?.truthy(),
        tag_style: &tag_style,
    };
    let empty = HashMap::new();
    let mut list = view::List {
        templates: &all,
        filter: params.filter.as_deref().unwrap_or_default(),
        requested_tag: params.tag.as_deref().filter(|t| !t.is_empty()),
        context: &context,
        variables: &variables,
        cooked: &empty,
    };
    // DCookText: the raw content cooked, oneboxes left as links.
    let host = crate::pretty_text::Host::from_state(&state);
    let mut cooked = HashMap::new();
    for id in view::shown_ids(&cx, &list) {
        if let Some(t) = all.iter().find(|t| t.id == id) {
            cooked.insert(
                id,
                crate::pretty_text::cook_without_onebox(&host, &t.content).await?,
            );
        }
    }
    list.cooked = &cooked;
    let html = view::filterable_list(&cx, &list);
    Ok((
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache, no-store"),
        ],
        html,
    )
        .into_response())
}
