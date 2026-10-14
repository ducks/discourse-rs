//! UsersController#recent_searches and #reset_recent_searches: GET and
//! DELETE /u/recent-searches(.json), the search menu's recent searches
//! (the member's search log terms, newest first, since they last cleared
//! them) and clearing them.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `UsersController::MAX_RECENT_SEARCHES`
pub const MAX_RECENT_SEARCHES: i64 = 5;

/// The route's `format: "json"` constraint: the .json path, or a request
/// asking for JSON. Anything else matches no route.
fn wants_json(headers: &HeaderMap, uri: &Uri) -> bool {
    uri.path().ends_with(".json")
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.contains("application/json"))
}

/// The member's recent search terms and when each was last searched:
/// their search log grouped by term, newest first, after
/// `oldest_search_log_date`.
pub async fn terms(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Vec<(String, NaiveDateTime)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT term, MAX(created_at) FROM search_logs \
         WHERE user_id = $1 \
           AND (created_at > (SELECT oldest_search_log_date FROM user_options WHERE user_id = $1) \
                OR (SELECT oldest_search_log_date FROM user_options WHERE user_id = $1) IS NULL) \
         GROUP BY term ORDER BY max(created_at) DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(MAX_RECENT_SEARCHES)
    .fetch_all(conn)
    .await
}

/// GET /u/recent-searches(.json)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !wants_json(&headers, &uri) {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("log_search_queries")?.truthy() {
        let error = state
            .i18n
            .t("user_activity.no_log_search_queries")
            .unwrap_or("Search log queries are currently disabled (an administrator can enable them in site settings).")
            .to_string();
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"failed": "FAILED", "error": error})),
        )
            .into_response());
    }
    let rows = terms(&mut conn, user_id).await?;
    let detailed: Vec<Value> = rows
        .iter()
        .map(|(term, at)| {
            json!({
                "term": term,
                "at": DateTime::<Utc>::from_naive_utc_and_offset(*at, Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
            })
        })
        .collect();
    Ok(Json(json!({
        "success": "OK",
        "recent_searches": rows.iter().map(|(term, _)| term).collect::<Vec<_>>(),
        "recent_searches_detailed": detailed,
    }))
    .into_response())
}

/// DELETE /u/recent-searches(.json): `oldest_search_log_date` a second
/// ago.
pub async fn reset(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if !wants_json(&headers, &uri) {
        return Ok(super::topics::not_found_response(&state));
    }
    if !csrf_ok(&state, &headers, &[], uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let at = crate::clock::now() - chrono::Duration::seconds(1);
    sqlx::query("UPDATE user_options SET oldest_search_log_date = $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(at.naive_utc())
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"success": "OK"})).into_response())
}
