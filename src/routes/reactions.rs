//! discourse-reactions' CustomReactionsController: PUT
//! /discourse-reactions/posts/:post_id/custom-reactions/:reaction/toggle,
//! answering with the post as PostSerializer has it.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::plugins::reactions::toggle::{self, Outcome};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// PUT /discourse-reactions/posts/:post_id/custom-reactions/:reaction/toggle
pub async fn toggle(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((post_id, reaction)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // requires_plugin
    if !crate::plugins::reactions::enabled(&settings)? {
        return Ok(super::topics::not_found_response(&state));
    }
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let post_id = crate::ruby::to_i(&post_id) as i32;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    match toggle::toggle(&mut tx, &ctx, &guardian, post_id, &reaction).await? {
        Outcome::Done(like_count) => {
            tx.commit().await?;
            let mut post =
                super::posts::serialize_post_alone(&state, &settings, &guardian, post_id).await?;
            stale_like_count(&mut post, like_count);
            Ok((StatusCode::OK, Json(post)).into_response())
        }
        Outcome::NotFound => Ok(super::topics::not_found_response(&state)),
        Outcome::InvalidAccess => Ok(super::search::invalid_access(&state)),
        // render_json_error(post): a post without errors gets
        // JsonError.generic_error, the client locale's js.generic_error.
        Outcome::InvalidReaction => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"errors": ["Sorry, an error has occurred."]})),
        )
            .into_response()),
    }
}

/// The service serializes the post it loaded before toggling, whose
/// like_count the like's counters (an update_all) never touched: the like
/// summary counts what was there before, and says nothing at none.
fn stale_like_count(post: &mut Value, like_count: i32) {
    let Some(Value::Array(summary)) = post.get_mut("actions_summary") else {
        return;
    };
    let like = crate::post_actions::LIKE;
    let Some(entry) = summary
        .iter_mut()
        .find(|e| e.get("id").and_then(Value::as_i64) == Some(like))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let mut rebuilt = Map::new();
    for (key, value) in std::mem::take(entry) {
        if key == "count" {
            continue;
        }
        let is_id = key == "id";
        rebuilt.insert(key, value);
        if is_id && like_count > 0 {
            rebuilt.insert("count".into(), json!(like_count));
        }
    }
    *entry = rebuilt;
}
