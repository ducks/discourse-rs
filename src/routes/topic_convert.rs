//! Port of TopicsController#convert_topic: PUT /t/:id/convert-topic/:type,
//! through TopicConverter (crate::topic_converter).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_converter::{self, Converted};
use crate::topic_guardian::TopicCtx;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /t/:id/convert-topic/:type
pub async fn convert(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((id, kind)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let kind = kind.strip_suffix(".json").unwrap_or(&kind).to_string();
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let s = &settings;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // Topic.find_by(id:), then ensure_can_convert_topic!: a missing or
    // deleted topic can't be converted.
    let topic_id = crate::ruby::to_i(&id) as i32;
    let topic = if guardian.is_authenticated() {
        TopicCtx::load(&mut conn, s, &guardian, topic_id).await?
    } else {
        None
    };
    let can_convert = match &topic {
        Some(topic) => {
            let can_post_anywhere = guardian.can_create_post_anywhere(&mut conn, s).await?;
            let can_create_post = guardian.can_create_post(s, topic, can_post_anywhere)?;
            guardian.can_convert_topic(s, topic, can_create_post)?
        }
        None => false,
    };
    if !can_convert {
        if guardian.is_anonymous() {
            return Ok(super::login_required::not_logged_in(&state));
        }
        return Ok(super::search::invalid_access(&state));
    }
    drop(conn);
    let converted = if kind == "public" {
        let category_id = p
            .get("category_id")
            .and_then(params::scalar)
            .map(|c| crate::ruby::to_i(&c) as i32);
        topic_converter::to_public(&state.pool, &ctx, &guardian, topic_id, category_id, false)
            .await?
    } else {
        topic_converter::to_private(&state.pool, &ctx, &guardian, topic_id, false).await?
    };
    Ok(match converted {
        Converted::Done(url) => {
            (StatusCode::OK, Json(json!({"success": true, "url": url}))).into_response()
        }
        Converted::Invalid(errors) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": errors})),
        )
            .into_response(),
    })
}
