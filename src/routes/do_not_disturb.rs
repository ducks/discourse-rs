//! Port of DoNotDisturbController: POST /do-not-disturb (a timing from
//! now for `duration` minutes, or until the end of the day for
//! "tomorrow") and DELETE /do-not-disturb (ending it, releasing what was
//! shelved).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use chrono::{Duration, NaiveTime, Timelike};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /do-not-disturb
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
    // requires_login
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let duration = p.get("duration");
    let blank = match duration {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => crate::ruby::is_blank(s),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    };
    if blank {
        return Ok(super::search::invalid_parameters(&state, "duration"));
    }
    let text = duration.and_then(params::scalar);
    // Integer(params[:duration]) minutes from now, else "tomorrow": the
    // end of the server's day (the reference runs in UTC).
    let ends_at = match text.as_deref().and_then(crate::ruby::integer) {
        Some(minutes) => {
            let Some(by) = Duration::try_minutes(minutes) else {
                return Err(crate::Unsupported("a do not disturb duration out of range").into());
            };
            crate::clock::now_naive()
                .checked_add_signed(by)
                .ok_or(crate::Unsupported("a do not disturb duration out of range"))?
        }
        None if text.as_deref() == Some("tomorrow") => crate::clock::now_naive()
            .date()
            .and_time(NaiveTime::from_hms_nano_opt(23, 59, 59, 999_999_999).unwrap_or_default()),
        None => return Ok(super::search::invalid_parameters(&state, "duration")),
    };
    // new(starts_at: Time.zone.now, ends_at:), after ends_at was taken.
    let starts_at = crate::clock::now_naive();
    if starts_at > ends_at {
        let message = state
            .i18n
            .t("activerecord.errors.messages.invalid")
            .unwrap_or("is invalid");
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": [format!("Ends at {message}")] })),
        )
            .into_response());
    }
    let mut tx = state.pool.begin().await?;
    // Stored to the microsecond.
    let stored = |t: chrono::NaiveDateTime| {
        t.with_nanosecond(t.nanosecond() / 1_000 * 1_000)
            .unwrap_or(t)
    };
    sqlx::query(
        "INSERT INTO do_not_disturb_timings (user_id, starts_at, ends_at, scheduled) \
         VALUES ($1, $2, $3, FALSE)",
    )
    .bind(user_id)
    .bind(stored(starts_at))
    .bind(stored(ends_at))
    .execute(&mut *tx)
    .await?;
    crate::do_not_disturb::publish(&state.bus, &mut tx, user_id, Some(ends_at)).await?;
    tx.commit().await?;
    Ok((
        StatusCode::OK,
        Json(json!({ "ends_at": time_json(ends_at) })),
    )
        .into_response())
}

/// DELETE /do-not-disturb
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    crate::do_not_disturb::end(&mut tx, &state.bus, &settings, user_id).await?;
    tx.commit().await?;
    Ok((StatusCode::OK, Json(json!({ "success": "OK" }))).into_response())
}
