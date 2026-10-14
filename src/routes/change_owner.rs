//! TopicsController#change_post_owners: POST /t/:topic_id/change-owner
//! hands posts of a topic to another user through PostOwnerChanger.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::post_owner_changer::{self, Change, Outcome};
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /t/:topic_id/change-owner
pub async fn change_owner(
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
    // ensure_logged_in
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    // :topic_id is constrained to digits.
    if topic_id.is_empty() || !topic_id.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    // params.require(:post_ids).map(&:to_i)
    let post_ids: Vec<i32> = match p.get("post_ids") {
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
                params::scalar(v)
                    .map(|s| crate::ruby::to_i(&s) as i32)
                    .ok_or(Unsupported("post_ids holding lists or hashes"))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(
                Unsupported("post_ids that are not a list (NoMethodError in Rails)").into(),
            );
        }
    };
    let username = match p.get("username") {
        None => return Ok(super::accounts::param_missing("username")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("username"));
        }
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(Unsupported("a username that is not a string").into()),
    };

    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // ensure_can_change_post_owner!
    let allowed = guardian.is_admin()
        || (settings.get("moderators_change_post_ownership")?.truthy() && guardian.is_staff())
        || guardian.in_setting_groups(&settings, "change_post_ownership_allowed_groups")?;
    if !allowed {
        return Ok(super::search::invalid_access(&state));
    }
    let new_owner: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(&username)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(new_owner) = new_owner else {
        return Ok(failed());
    };
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let change = Change {
        topic_id: i32::try_from(crate::ruby::to_i(&topic_id)).unwrap_or(0),
        post_ids,
        new_owner,
        skip_revision: false,
    };
    let outcome =
        post_owner_changer::change_owner(&state.pool, &mut tx, &ctx, &guardian, &change).await?;
    Ok(match outcome {
        Outcome::Done => {
            tx.commit().await?;
            (StatusCode::OK, Json(json!({"success": "OK"}))).into_response()
        }
        Outcome::ArgumentError => failed(),
        // The posts changed before the refusal were each saved by then.
        Outcome::InvalidAccess => {
            tx.commit().await?;
            super::search::invalid_access(&state)
        }
        Outcome::RateLimited(refused) => {
            tx.commit().await?;
            refused.into_response(&state.i18n)
        }
    })
}

fn failed() -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"failed": "FAILED"})),
    )
        .into_response()
}
