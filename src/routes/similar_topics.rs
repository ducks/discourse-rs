//! SimilarTopicsController: GET /topics/similar_to and /similar_topics,
//! the topics like the one being written (src/topics/similar.rs), and GET
//! /composer/similar-topics, the same topics as ComposerMessages' similar
//! topics popup draws them (composer-messages/similar-topics.gjs) for
//! static/js/composer-messages.js.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::guardian::Guardian;
use crate::params;
use crate::search_menu_view::Menu;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::{ListContext, ListSettings};
use crate::topics::similar;
use crate::{AppError, AppState};

/// The title and raw params, the title None when `params.require` raises.
fn title_and_raw(p: &Map<String, Value>) -> (Option<String>, Option<String>) {
    let title = p
        .get("title")
        .and_then(params::scalar)
        .filter(|t| !t.is_empty());
    (title, p.get("raw").and_then(params::scalar))
}

/// `SimilarTopicsController#index`'s document.
async fn find(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    title: &str,
    raw: Option<&str>,
) -> Result<Value, AppError> {
    let min = settings.get("min_title_similar_length")?.to_i();
    if (title.chars().count() as i64) < min
        || !similar::count_exceeds_minimum(&mut *conn, settings).await?
    {
        // render json: [] under the controller's root.
        return Ok(json!({ "similar_topics": [] }));
    }
    let topics = similar::similar_to(state, &mut *conn, settings, guardian, title, raw).await?;
    similar::serialize(state, conn, settings, guardian, &topics).await
}

/// GET /topics/similar_to(.json), /similar_topics(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let (Some(title), raw) = title_and_raw(&p) else {
        return Ok(super::accounts::param_missing("title"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let doc = find(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &title,
        raw.as_deref(),
    )
    .await?;
    Ok(Json(doc).into_response())
}

/// GET /composer/similar-topics: the popup's rows and how many.
pub async fn popup(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // The composer is a member's.
    if guardian.user().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let (Some(title), raw) = title_and_raw(&p) else {
        return Ok(super::accounts::param_missing("title"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let doc = find(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &title,
        raw.as_deref(),
    )
    .await?;
    let base_path = state.config.globals.relative_url_root().to_string();
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: None,
        settings: ListSettings::load(&settings)?,
    };
    let menu = Menu {
        cx: &cx,
        mobile: crate::mobile::mobile_view(),
        member: true,
        log_search_queries: false,
        tagging: settings.get("tagging_enabled")?.truthy(),
        term: "",
        context: None,
        in_topic: false,
        in_pm_inbox: false,
        search_topics: true,
        invalid_term: false,
        no_results: false,
        recent: &[],
        quick_tip: 0,
        keyboard: false,
        apple: false,
    };
    let empty = Vec::new();
    let items = doc["similar_topics"].as_array().unwrap_or(&empty);
    let topics = doc["topics"].as_array().unwrap_or(&empty);
    let mut html = String::new();
    for item in items {
        if let Some(topic) = topics.iter().find(|t| t["id"] == item["topic_id"]) {
            html.push_str(&menu.similar_topic(item, topic));
        }
    }
    Ok(Json(json!({ "count": items.len(), "html": html })).into_response())
}
