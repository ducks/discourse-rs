//! ComposerMessagesController: GET /composer_messages, the message the
//! composer shows as it opens (src/composer/messages.rs) with the topic's
//! links for the duplicate link warning, and GET
//! /composer_messages/user_not_seen_in_a_while, the recipients of a new
//! message who have not been seen for a while.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::composer::messages::{self, Details, Finder};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// A scalar param as Rails reads it, None when absent.
fn scalar(p: &Map<String, Value>, key: &str) -> Option<String> {
    p.get(key).and_then(params::scalar)
}

/// GET /composer_messages(.json)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // requires_login
    if guardian.user().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let details = Details {
        composer_action: scalar(&p, "composer_action"),
        topic_id: scalar(&p, "topic_id"),
        post_id: scalar(&p, "post_id"),
    };
    let topic_id = details
        .topic_id
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .map(str::to_string);
    let finder = Finder::new(&mut conn, &state, &settings, &guardian, details).await?;
    let found = finder.find(&mut conn).await?;
    let mut json = Map::new();
    json.insert(
        "composer_messages".into(),
        json!(found.into_iter().collect::<Vec<_>>()),
    );

    // Topic.where(id:).first, then guardian.can_see?(topic)
    if topic_id.is_some() {
        let topic = crate::ruby::to_i(topic_id.as_deref().unwrap_or_default()) as i32;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
        )
        .bind(topic)
        .fetch_one(&mut *conn)
        .await?;
        if exists
            && !guardian
                .can_see_topic_ids(&mut conn, &settings, &[topic])
                .await?
                .is_empty()
        {
            let lookup = messages::duplicate_lookup(&mut conn, &settings, &guardian, topic).await?;
            json.insert("extras".into(), json!({ "duplicate_lookup": lookup }));
        }
    }
    // render_json_dump(json, rest_serializer: true)
    json.insert("__rest_serializer".into(), json!("1"));
    Ok(Json(Value::Object(json)).into_response())
}

/// GET /composer_messages/user_not_seen_in_a_while(.json)
pub async fn user_not_seen_in_a_while(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if guardian.user().is_none() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &body);
    // params.require(:usernames)
    let usernames: Vec<String> = match p.get("usernames") {
        Some(Value::Array(items)) => items.iter().filter_map(params::scalar).collect(),
        Some(Value::Object(map)) => map.values().filter_map(params::scalar).collect(),
        Some(value) => params::scalar(value)
            .filter(|s| !s.is_empty())
            .into_iter()
            .collect(),
        None => Vec::new(),
    };
    if usernames.is_empty() {
        return Ok(super::accounts::param_missing("usernames"));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let users = messages::user_not_seen_in_a_while(&mut conn, &settings, &usernames).await?;
    let time_ago = crate::age_words::time_ago_verbose(
        &state.i18n,
        messages::months_ago(&settings)?,
        crate::clock::now_naive(),
    );
    Ok(Json(json!({
        "user_count": users.len(),
        "usernames": users,
        "time_ago": time_ago,
    }))
    .into_response())
}
