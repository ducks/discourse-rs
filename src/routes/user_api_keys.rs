//! UserApiKeysController#revoke and #undo_revoke, and
//! UserApiKeyClientsController (registering a client with its auth
//! redirect and scopes, and asking whether one is registered).

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::accounts::param_missing;
use super::search::Peer;
use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::rate_limiter::RateLimiter;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// `params.require(name)`: present and not blank.
fn required(p: &Map<String, Value>, name: &str) -> Option<String> {
    match p.get(name) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(v @ (Value::Number(_) | Value::Bool(true))) => params::scalar(v),
        Some(Value::Array(a)) if !a.is_empty() => Some(String::new()),
        Some(Value::Object(o)) if !o.is_empty() => Some(String::new()),
        _ => None,
    }
}

fn success() -> Response {
    Json(json!({"success": "OK"})).into_response()
}

/// `find_key`: UserApiKey.find(params[:id]), the owner's or an admin's.
async fn find_key(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    user_id: i32,
    id: &str,
) -> Result<Result<i32, Response>, AppError> {
    let id = i32::try_from(crate::ruby::to_i(id)).unwrap_or(0);
    let owner: Option<i32> = sqlx::query_scalar("SELECT user_id FROM user_api_keys WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(match owner {
        None => Err(super::topics::not_found_response(state)),
        Some(owner) if guardian.is_admin() || owner == user_id => Ok(id),
        Some(_) => Err(super::search::invalid_access(state)),
    })
}

/// POST /user-api-key/revoke: the key by `id`, or the request's own.
pub async fn revoke(
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
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    // `find_key if params[:id]`: nil and false skip it.
    let mut key = None;
    match p.get("id") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(v) => {
            let id = params::scalar(v).unwrap_or_default();
            match find_key(&state, &guardian, user_id, &id).await? {
                Ok(id) => key = Some(id),
                Err(refused) => return Ok(refused),
            }
        }
    }
    if key.is_none()
        && let Some(current) = headers
            .get(crate::session::user_api_key::HEADER_USER_API_KEY)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.trim().is_empty())
    {
        key = sqlx::query_scalar(
            "SELECT id FROM user_api_keys WHERE key_hash = $1 ORDER BY id LIMIT 1",
        )
        .bind(crate::session::user_api_key::hash_key(current))
        .fetch_optional(&state.pool)
        .await?;
    }
    let Some(key) = key else {
        return Ok(super::topics::not_found_response(&state));
    };
    sqlx::query("UPDATE user_api_keys SET revoked_at = now() WHERE id = $1")
        .bind(key)
        .execute(&state.pool)
        .await?;
    Ok(success())
}

/// POST /user-api-key/undo-revoke
pub async fn undo_revoke(
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
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let id = params::string(&p, "id").unwrap_or_default();
    let key = match find_key(&state, &guardian, user_id, &id).await? {
        Ok(id) => id,
        Err(refused) => return Ok(refused),
    };
    sqlx::query("UPDATE user_api_keys SET revoked_at = NULL WHERE id = $1")
        .bind(key)
        .execute(&state.pool)
        .await?;
    Ok(success())
}

/// GET /user-api-key-client: 200 when a client with an auth redirect is
/// registered under `client_id`.
pub async fn client_show(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &[]);
    let Some(client_id) = required(&p, "client_id") else {
        return Ok(param_missing("client_id"));
    };
    let registered: bool = sqlx::query_scalar(
        "SELECT COALESCE((SELECT auth_redirect IS NOT NULL AND btrim(auth_redirect) <> '' \
                          FROM user_api_key_clients WHERE client_id = $1 ORDER BY id LIMIT 1), false)",
    )
    .bind(client_id)
    .fetch_one(&state.pool)
    .await?;
    if !registered {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    }
    Ok(StatusCode::OK.into_response())
}

/// POST /user-api-key-client: registers a client (no CSRF check), at most
/// user_api_key_clients_create_per_day a day per address.
pub async fn client_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    let skip_limit = settings
        .get("create_user_api_key_client_ip_rate_limit_override_ips")?
        .to_s()
        .split('|')
        .any(|o| o == ip);
    if !skip_limit {
        let limiter = RateLimiter::new(
            None,
            &format!("user-api-key-clients-{ip}"),
            settings.get("user_api_key_clients_create_per_day")?.to_i(),
            24 * 3600,
        );
        if let Err(refused) = limiter
            .performed(&state.pool, state.config.rate_limits)
            .await?
        {
            return Ok(refused.into_response(&state.i18n));
        }
    }
    let mut values = Vec::new();
    for name in [
        "client_id",
        "application_name",
        "public_key",
        "auth_redirect",
        "scopes",
    ] {
        match required(&p, name) {
            Some(v) => values.push(v),
            None => return Ok(param_missing(name)),
        }
    }
    let scopes: Vec<String> = values[4]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if scopes.is_empty() {
        return Ok(super::search::invalid_parameters(&state, "scopes"));
    }
    let allowed = settings.get("allow_user_api_key_client_scopes")?.to_s();
    let allowed: Vec<&str> = allowed.split('|').collect();
    if !scopes.iter().all(|s| allowed.contains(&s.as_str())) {
        return Ok(super::search::invalid_access(&state));
    }
    // OpenSSL::PKey::RSA.new, which raises (a 500) on a key it can't read.
    if crate::user_api_key_crypto::parse_public_key(&values[2]).is_none() {
        return Err(AppError::from(std::io::Error::other(
            "OpenSSL::PKey::RSAError: Neither PUB key nor PRIV key",
        )));
    }
    let mut tx = state.pool.begin().await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_api_key_clients WHERE client_id = $1)",
    )
    .bind(&values[0])
    .fetch_one(&mut *tx)
    .await?;
    if exists {
        return Ok(super::search::invalid_access(&state));
    }
    let client: i64 = sqlx::query_scalar(
        "INSERT INTO user_api_key_clients (client_id, application_name, public_key, auth_redirect, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, now(), now()) RETURNING id",
    )
    .bind(&values[0])
    .bind(&values[1])
    .bind(&values[2])
    .bind(&values[3])
    .fetch_one(&mut *tx)
    .await?;
    for scope in &scopes {
        sqlx::query(
            "INSERT INTO user_api_key_client_scopes (user_api_key_client_id, name, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(client)
        .bind(scope)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(success())
}
