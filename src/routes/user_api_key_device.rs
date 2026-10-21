//! The user API key device authorization flow (UserApiKey::DeviceAuth and
//! UserApiKeysController's device actions): an application asks for a
//! device code, the member enters the user code (or opens the request
//! link) at /user-api-key/activate and authorizes or denies, and the
//! application polls until the encrypted key is ready.
//!
//! Rails keeps the grants, their user-code and request-token indexes and
//! the authorization locks in Redis; here they are the same keys in
//! `discourse_rs.cached_values` (locks in `expiring_keys`). The approval
//! tokens Rails keeps in the cookie session are in the browser's server
//! session. The debug traces are log-only and not ported.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::search::Peer;
use super::session::{bad_csrf, csrf_ok};
use super::user_api_key_auth::{
    Refusal, allowed_scopes, current_user_json, find_client, has_push, iso8601, localized_scopes,
    meets_tl, no_store, parse_expires_in_seconds, payload_json, redirect_to_login, refusal,
    split_scopes, validate_padding, wants_json,
};
use super::user_api_keys::{form_pairs, required};
use crate::guardian::Guardian;
use crate::owned_schema::{cached_del, cached_get, cached_setex};
use crate::params;
use crate::rate_limiter::RateLimiter;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `DEVICE_AUTH_TTL`
const DEVICE_AUTH_TTL: i64 = 600;
/// `DEVICE_AUTH_INTERVAL`
const DEVICE_AUTH_INTERVAL: i64 = 5;
/// `DEVICE_AUTHORIZED_PAYLOAD_TTL`
const DEVICE_AUTHORIZED_PAYLOAD_TTL: i64 = 60;
const DEVICE_AUTHORIZATION_LOCK_TTL: i64 = 30;
const USER_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const MAX_DEVICE_GRANT_BYTES: usize = 10_000;
const MAX_CODE_REGISTRY_COLLISION_ATTEMPTS: usize = 20;
const DEVICE_REQUESTS_PER_MINUTE: i64 = 20;
const DEVICE_POLLS_PER_MINUTE: i64 = 60;
const DEVICE_ACTIVATION_ATTEMPTS_PER_MINUTE: i64 = 10;
const DEVICE_ACTIVATION_ATTEMPTS_PER_HOUR: i64 = 30;
const DEVICE_REQUEST_TOKEN_LOOKUPS_PER_MINUTE: i64 = 30;
const DEVICE_REQUEST_TOKEN_LOOKUPS_PER_HOUR: i64 = 120;
const APPROVALS_KEY: &str = "user_api_key_device_approvals";
const MAX_APPROVAL_TOKENS: usize = 10;

fn grant_key(device_code: &str) -> String {
    format!("user_api_key:device:{device_code}")
}
fn user_code_key(user_code: &str) -> String {
    format!("user_api_key:device:code:{user_code}")
}
fn request_token_key(token: &str) -> String {
    format!("user_api_key:device:request:{token}")
}
fn lock_key(device_code: &str) -> String {
    format!("user_api_key:device:lock:{device_code}")
}

/// `DEVICE_CODE_REGEX`
fn valid_device_code(code: &str) -> bool {
    code.len() == 64 && code.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `CodeRegistry.valid_request_token?`
fn valid_request_token(token: &str) -> bool {
    token.len() == 8
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// `CodeRegistry.normalize_user_code`
fn normalize_user_code(value: &str) -> Option<String> {
    let code: String = value
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .collect();
    (code.len() == 8).then(|| format!("{}-{}", &code[..4], &code[4..]))
}

/// UserApiKey::DeviceAuth::Grant, as GrantStore keeps it (`to_h.compact`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Grant {
    status: String,
    device_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    application_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    scopes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    push_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    padding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in_seconds: Option<i64>,
    /// `iso8601(6)`
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    unregistered_client: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorized_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    denied_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorizing_user_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorizing_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorizing_at: Option<String>,
}

fn now_iso() -> String {
    crate::clock::now_naive()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

impl Grant {
    fn pending(&self) -> bool {
        self.status == "pending"
    }
    fn authorized(&self) -> bool {
        self.status == "authorized"
    }
    fn denied(&self) -> bool {
        self.status == "denied"
    }

    /// The expiry as a time (Time.zone.parse of the stored iso8601(6)).
    fn expires_at(&self) -> Option<chrono::NaiveDateTime> {
        let s = self.expires_at.as_deref()?;
        chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f").ok()
    }

    fn bound_to_another_user(&self, user_id: i32) -> bool {
        self.authorizing_user_id.is_some_and(|id| id != user_id)
    }

    /// `bind_to_user!`
    fn bind_to_user(&mut self, user_id: i32, username: &str) -> bool {
        if self.bound_to_another_user(user_id) {
            return false;
        }
        if self.authorizing_user_id == Some(user_id) {
            return true;
        }
        self.authorizing_user_id = Some(user_id);
        self.authorizing_username = Some(username.to_string());
        self.authorizing_at = Some(now_iso());
        true
    }

    fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// `device_auth_json`
    fn device_auth_json(&self, state: &AppState) -> Value {
        json!({
            "application_name": self.application_name,
            "localized_scopes": localized_scopes(state, &self.scopes),
            "write_scope": self.scopes.iter().any(|s| s == "write"),
            "unregistered_client": self.unregistered_client,
            "expires_at": self.expires_at().map(iso8601),
        })
    }
}

/// GrantStore.load
async fn load_grant(conn: &mut PgConnection, device_code: &str) -> Result<Option<Grant>, AppError> {
    if !valid_device_code(device_code) {
        return Ok(None);
    }
    let Some(serialized) = cached_get(conn, &grant_key(device_code)).await? else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<Grant>(&serialized)
        .ok()
        .filter(|g| g.device_code == device_code))
}

async fn save_grant(conn: &mut PgConnection, grant: &Grant, ttl: i64) -> Result<(), AppError> {
    cached_setex(conn, &grant_key(&grant.device_code), ttl, &grant.to_json()).await?;
    Ok(())
}

/// `ttl_for_update`: what is left of the grant's TTL (Redis TTL, rounded),
/// else a fresh one.
async fn ttl_for_update(conn: &mut PgConnection, device_code: &str) -> Result<i64, AppError> {
    let ttl: Option<f64> = sqlx::query_scalar(
        "SELECT EXTRACT(EPOCH FROM expires_at - clock_timestamp())::float8 \
         FROM discourse_rs.cached_values WHERE key = $1 AND expires_at > clock_timestamp()",
    )
    .bind(grant_key(device_code))
    .fetch_optional(&mut *conn)
    .await?;
    let ttl = ttl.map(|t| t.round() as i64).unwrap_or(0);
    Ok(if ttl > 0 { ttl } else { DEVICE_AUTH_TTL })
}

/// Redis `SET key value NX EX ttl`.
async fn set_nx(
    conn: &mut PgConnection,
    key: &str,
    value: &str,
    ttl: i64,
) -> Result<bool, AppError> {
    sqlx::query(
        "DELETE FROM discourse_rs.cached_values WHERE key = $1 AND expires_at <= clock_timestamp()",
    )
    .bind(key)
    .execute(&mut *conn)
    .await?;
    let inserted = sqlx::query(
        "INSERT INTO discourse_rs.cached_values (key, value, expires_at) \
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3)) ON CONFLICT (key) DO NOTHING",
    )
    .bind(key)
    .bind(value)
    .bind(ttl as f64)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    Ok(inserted == 1)
}

/// GrantStore.with_lock!: None when another request holds the lock
/// (InvalidAccess in Rails).
async fn lock(conn: &mut PgConnection, device_code: &str) -> Result<bool, AppError> {
    Ok(
        crate::owned_schema::set_once(conn, &lock_key(device_code), DEVICE_AUTHORIZATION_LOCK_TTL)
            .await?,
    )
}

async fn unlock(conn: &mut PgConnection, device_code: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM discourse_rs.expiring_keys WHERE key = $1")
        .bind(lock_key(device_code))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// CodeRegistry.load_by_user_code
async fn load_by_user_code(
    conn: &mut PgConnection,
    user_code: &str,
) -> Result<Option<Grant>, AppError> {
    let Some(device_code) = cached_get(conn, &user_code_key(user_code))
        .await?
        .filter(|c| !c.is_empty())
    else {
        return Ok(None);
    };
    let grant = load_grant(conn, &device_code).await?;
    if grant.is_none() {
        cached_del(conn, &user_code_key(user_code)).await?;
    }
    Ok(grant)
}

/// CodeRegistry.load_by_request_token
async fn load_by_request_token(
    conn: &mut PgConnection,
    token: &str,
) -> Result<Option<Grant>, AppError> {
    if !valid_request_token(token) {
        return Ok(None);
    }
    let Some(device_code) = cached_get(conn, &request_token_key(token))
        .await?
        .filter(|c| !c.is_empty())
    else {
        return Ok(None);
    };
    let grant = load_grant(conn, &device_code).await?;
    if grant.is_none() {
        cached_del(conn, &request_token_key(token)).await?;
    }
    Ok(grant)
}

/// CodeRegistry.delete_indexes_for
async fn delete_indexes(conn: &mut PgConnection, grant: &Grant) -> Result<(), AppError> {
    if let Some(c) = grant.user_code.as_deref().filter(|c| !c.is_empty()) {
        cached_del(conn, &user_code_key(c)).await?;
    }
    if let Some(t) = grant.request_token.as_deref().filter(|t| !t.is_empty()) {
        cached_del(conn, &request_token_key(t)).await?;
    }
    Ok(())
}

/// `unavailable_reason_for`: None when the grant can be acted on.
fn unavailable(grant: Option<&Grant>, user_id: i32) -> bool {
    match grant {
        None => true,
        Some(g) => !g.pending() || g.bound_to_another_user(user_id),
    }
}

fn random_user_code() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let code: String = (0..8)
        .map(|_| USER_CODE_ALPHABET[rng.gen_range(0..USER_CODE_ALPHABET.len())] as char)
        .collect();
    format!("{}-{}", &code[..4], &code[4..])
}

/// `SecureRandom.urlsafe_base64(6)`: 8 characters.
fn random_request_token() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut b = [0u8; 6];
    rand::thread_rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn random_hex64() -> String {
    format!(
        "{}{}",
        crate::accounts::random_hex(),
        crate::accounts::random_hex()
    )
}

async fn settings(state: &AppState) -> Result<SiteSettings, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `ensure_json_request!`: the JSON format and a JSON body.
fn json_request(uri: &Uri, headers: &HeaderMap) -> bool {
    wants_json(uri, headers)
        && headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                let mime = v.split(';').next().unwrap_or("").trim();
                mime == "application/json" || mime.ends_with("+json")
            })
}

/// `RateLimiter#performed!` for each, in order: the first refusal.
async fn limit(state: &AppState, limiters: Vec<RateLimiter>) -> Result<Option<Response>, AppError> {
    for l in limiters {
        if let Err(refused) = l.performed(&state.pool, state.config.rate_limits).await? {
            return Ok(Some(refused.into_response(&state.i18n)));
        }
    }
    Ok(None)
}

fn hash(value: &str) -> String {
    crate::session::user_api_key::hash_key(value)
}

/// The request's string param, cast as a service's `:string` attribute.
fn string_param(p: &Map<String, Value>, name: &str) -> Option<String> {
    params::string(p, name)
}

fn blank(v: &Option<String>) -> bool {
    v.as_deref().is_none_or(|s| s.trim().is_empty())
}

/// RequestValidator.validate_requested_scopes!
fn validate_requested_scopes(
    settings: &SiteSettings,
    scopes: &[String],
) -> Result<Result<(), Refusal>, AppError> {
    if scopes.is_empty() || scopes.iter().any(|s| s.trim().is_empty()) {
        return Ok(Err(Refusal::Params("scopes".into())));
    }
    if scopes.iter().any(|s| s == "one_time_password") {
        return Ok(Err(Refusal::Params("scopes".into())));
    }
    let allowed = allowed_scopes(settings)?;
    if !scopes.iter().all(|s| allowed.contains(s)) {
        return Ok(Err(Refusal::Access));
    }
    Ok(Ok(()))
}

/// RequestValidator: the RSA key between 2048 and 8192 bits.
fn device_public_key(value: Option<&str>) -> Result<rsa::RsaPublicKey, Refusal> {
    use rsa::traits::PublicKeyParts;
    let key = value
        .and_then(crate::user_api_key_crypto::parse_public_key)
        .ok_or(Refusal::Params("public_key".into()))?;
    let bits = key.n().bits();
    if !(2048..=8192).contains(&bits) {
        return Err(Refusal::Params("public_key".into()));
    }
    Ok(key)
}

/// POST /user-api-key/device: a pending grant and its codes.
pub async fn create_device_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !json_request(&uri, &headers) {
        return Ok(no_store(super::search::invalid_access(&state)));
    }
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    if let Some(refused) = limit(
        &state,
        vec![RateLimiter::new(
            None,
            &format!("user-api-key-device-requests-{ip}"),
            DEVICE_REQUESTS_PER_MINUTE,
            60,
        )],
    )
    .await?
    {
        return Ok(no_store(refused));
    }
    let settings = settings(&state).await?;
    let p = params::parse(uri.query(), &headers, &body);
    match create_grant(&state, &settings, &p).await? {
        Ok(response) => Ok(no_store(Json(response).into_response())),
        Err(r) => Ok(no_store(refusal(&state, r))),
    }
}

async fn create_grant(
    state: &AppState,
    settings: &SiteSettings,
    p: &Map<String, Value>,
) -> Result<Result<Value, Refusal>, AppError> {
    // The service's contract: nonce, scopes and client_id present.
    let get = |k: &str| string_param(p, k);
    if blank(&get("nonce")) || blank(&get("scopes")) || blank(&get("client_id")) {
        return Ok(Err(Refusal::Params("base".into())));
    }
    let client_id = get("client_id").unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let client = find_client(&mut conn, "client_id", &client_id).await?;
    // RequestValidator.validate!: lengths first.
    for (name, max) in [
        ("client_id", 200),
        ("application_name", 200),
        ("nonce", 256),
        ("public_key", 4096),
        ("push_url", 2000),
    ] {
        if get(name).is_some_and(|v| !v.trim().is_empty() && v.len() > max) {
            return Ok(Err(Refusal::Params(name.into())));
        }
    }
    let scopes_param = get("scopes").unwrap_or_default();
    let scopes = split_scopes(&scopes_param);
    if scopes_param.chars().count() > 500 || scopes.len() > 20 {
        return Ok(Err(Refusal::Params("scopes".into())));
    }
    let client_name = client
        .as_ref()
        .map(|c| c.application_name.clone())
        .filter(|n| !n.trim().is_empty());
    if client_name.is_none() && blank(&get("application_name")) {
        return Ok(Err(Refusal::Params("application_name".into())));
    }
    if let Err(r) = validate_requested_scopes(settings, &scopes)? {
        return Ok(Err(r));
    }
    if let Some(c) = &client
        && !c.scopes.is_empty()
        && !scopes.iter().all(|s| c.scopes.contains(s))
    {
        return Ok(Err(Refusal::Access));
    }
    if let Err(r) = validate_padding(p) {
        return Ok(Err(r));
    }
    let public_key_str = client
        .as_ref()
        .and_then(|c| c.public_key.clone())
        .filter(|k| !k.trim().is_empty())
        .or_else(|| get("public_key"));
    let public_key = match device_public_key(public_key_str.as_deref()) {
        Ok(k) => k,
        Err(r) => return Ok(Err(r)),
    };
    // build_grant
    let expires_in_seconds = match parse_expires_in_seconds(settings, p)? {
        Ok(s) => s,
        Err(r) => return Ok(Err(r)),
    };
    let device_code = random_hex64();
    let created_at = crate::clock::now_naive();
    let iso6 = |t: chrono::NaiveDateTime| t.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    let present = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let mut grant = Grant {
        status: "pending".into(),
        device_code: device_code.clone(),
        application_name: match &client {
            Some(_) => client_name.clone().or_else(|| get("application_name")),
            None => get("application_name"),
        },
        client_id: Some(client_id.clone()),
        public_key: public_key_str.clone(),
        nonce: get("nonce"),
        scopes: scopes.clone(),
        push_url: present(get("push_url")),
        padding: present(get("padding")),
        expires_in_seconds,
        expires_at: expires_in_seconds.map(|s| iso6(created_at + chrono::Duration::seconds(s))),
        unregistered_client: client
            .as_ref()
            .is_none_or(|c| c.public_key.as_deref().is_none_or(|k| k.trim().is_empty())),
        created_at: Some(iso6(created_at)),
        ..Default::default()
    };
    // PayloadBuilder.validate_size! with the placeholder key.
    let sample = payload_json(
        &"x".repeat(32),
        grant
            .nonce
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        false,
        grant.expires_at(),
    );
    if let Some(message) = crate::user_api_key_crypto::payload_too_large(
        &sample,
        &public_key,
        grant.padding.as_deref(),
    ) {
        return Ok(Err(Refusal::Params(message)));
    }
    if grant.to_json().len() > MAX_DEVICE_GRANT_BYTES {
        return Ok(Err(Refusal::Params("base".into())));
    }
    // reserve_codes: the request token, then the user code.
    let mut request_token = None;
    for _ in 0..MAX_CODE_REGISTRY_COLLISION_ATTEMPTS {
        let t = random_request_token();
        if set_nx(
            &mut conn,
            &request_token_key(&t),
            &device_code,
            DEVICE_AUTH_TTL,
        )
        .await?
        {
            request_token = Some(t);
            break;
        }
    }
    let Some(request_token) = request_token else {
        return Ok(Err(Refusal::Access));
    };
    let mut user_code = None;
    for _ in 0..MAX_CODE_REGISTRY_COLLISION_ATTEMPTS {
        let c = random_user_code();
        if set_nx(&mut conn, &user_code_key(&c), &device_code, DEVICE_AUTH_TTL).await? {
            user_code = Some(c);
            break;
        }
    }
    let Some(user_code) = user_code else {
        cached_del(&mut conn, &request_token_key(&request_token)).await?;
        return Ok(Err(Refusal::Access));
    };
    grant.user_code = Some(user_code.clone());
    grant.request_token = Some(request_token.clone());
    if grant.to_json().len() > MAX_DEVICE_GRANT_BYTES {
        return Ok(Err(Refusal::Params("base".into())));
    }
    save_grant(&mut conn, &grant, DEVICE_AUTH_TTL).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let verification_uri = format!("{}/user-api-key/activate", urls.base_url()?);
    Ok(Ok(json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": verification_uri,
        "verification_uri_with_request": format!(
            "{verification_uri}?request={}",
            form_urlencoded::byte_serialize(request_token.as_bytes()).collect::<String>()
        ),
        "expires_in": DEVICE_AUTH_TTL,
        "interval": DEVICE_AUTH_INTERVAL,
    })))
}

/// POST /user-api-key/device/poll: pending, denied, expired, or the
/// encrypted payload (once).
pub async fn poll_device_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !json_request(&uri, &headers) {
        return Ok(no_store(super::search::invalid_access(&state)));
    }
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    if let Some(refused) = limit(
        &state,
        vec![RateLimiter::new(
            None,
            &format!("user-api-key-device-poll-ip-{ip}"),
            DEVICE_POLLS_PER_MINUTE,
            60,
        )],
    )
    .await?
    {
        return Ok(no_store(refused));
    }
    let p = params::parse(uri.query(), &headers, &body);
    let Some(device_code) = required(&p, "device_code") else {
        return Ok(no_store(super::accounts::param_missing("device_code")));
    };
    if let Some(refused) = limit(
        &state,
        vec![RateLimiter::new(
            None,
            &format!("user-api-key-device-poll-code-{}", hash(&device_code)),
            DEVICE_POLLS_PER_MINUTE,
            60,
        )],
    )
    .await?
    {
        return Ok(no_store(refused));
    }
    let status = |s: &str| no_store(Json(json!({"status": s})).into_response());
    if !valid_device_code(&device_code) {
        return Ok(status("expired_token"));
    }
    let mut conn = state.pool.acquire().await?;
    let Some(grant) = load_grant(&mut conn, &device_code).await? else {
        return Ok(status("expired_token"));
    };
    if grant.pending() {
        return Ok(status("authorization_pending"));
    }
    if grant.denied() {
        return Ok(status("access_denied"));
    }
    if !grant.authorized() {
        return Ok(status("expired_token"));
    }
    // consume_authorized, under the lock.
    if !lock(&mut conn, &device_code).await? {
        return Ok(status("authorization_pending"));
    }
    let consumed = match load_grant(&mut conn, &device_code).await? {
        Some(g) if g.authorized() => {
            cached_del(&mut conn, &grant_key(&device_code)).await?;
            Some(g)
        }
        _ => None,
    };
    unlock(&mut conn, &device_code).await?;
    Ok(match consumed {
        Some(g) => {
            let mut body = Map::new();
            body.insert("status".into(), json!("authorized"));
            if let Some(payload) = g.payload.filter(|p| !p.is_empty()) {
                body.insert("payload".into(), json!(payload));
            }
            no_store(Json(Value::Object(body)).into_response())
        }
        None => status("expired_token"),
    })
}

/// The member's approval tokens (ApprovalTokenStore, in the browser's
/// server session rather than its cookie): token to `{device_code,
/// user_id, created_at}`, oldest first.
struct Approvals {
    session_id: String,
    tokens: Vec<(String, Value)>,
}

impl Approvals {
    async fn load(conn: &mut PgConnection, session_id: String) -> Result<Approvals, AppError> {
        let stored = crate::session::server::get(conn, &session_id, APPROVALS_KEY).await?;
        let tokens = match stored {
            Some(Value::Array(list)) => list
                .into_iter()
                .filter_map(|e| Some((e[0].as_str()?.to_string(), e[1].clone())))
                .collect(),
            _ => Vec::new(),
        };
        Ok(Approvals { session_id, tokens })
    }

    async fn persist(&self, conn: &mut PgConnection) -> Result<(), AppError> {
        let list: Vec<Value> = self.tokens.iter().map(|(t, v)| json!([t, v])).collect();
        crate::session::server::set(
            conn,
            &self.session_id,
            APPROVALS_KEY,
            &Value::Array(list),
            crate::session::server::EXPIRY_SECONDS,
        )
        .await?;
        Ok(())
    }

    fn expired(approval: &Value) -> bool {
        approval["created_at"]
            .as_str()
            .and_then(|t| chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%SZ").ok())
            .is_none_or(|t| {
                t <= crate::clock::now_naive() - chrono::Duration::seconds(DEVICE_AUTH_TTL)
            })
    }

    /// `create!`
    async fn create(
        &mut self,
        conn: &mut PgConnection,
        device_code: &str,
        user_id: i32,
    ) -> Result<String, AppError> {
        self.tokens.retain(|(_, a)| !Self::expired(a));
        while self.tokens.len() >= MAX_APPROVAL_TOKENS {
            self.tokens.remove(0);
        }
        let token = random_hex64();
        self.tokens.push((
            token.clone(),
            json!({"device_code": device_code, "user_id": user_id, "created_at": now_iso()}),
        ));
        self.persist(conn).await?;
        Ok(token)
    }

    /// `device_code_for`
    fn device_code_for(&self, token: &str, user_id: i32) -> Option<String> {
        let (_, approval) = self.tokens.iter().find(|(t, _)| t == token)?;
        if Self::expired(approval) || approval["user_id"].as_i64() != Some(i64::from(user_id)) {
            return None;
        }
        let code = approval["device_code"].as_str()?.to_string();
        valid_device_code(&code).then_some(code)
    }

    async fn delete(&mut self, conn: &mut PgConnection, token: &str) -> Result<(), AppError> {
        self.tokens.retain(|(t, _)| t != token);
        self.persist(conn).await
    }
}

/// What a device action renders (render_device_activation), JSON or the
/// page.
struct Rendered {
    model: Value,
}

fn enter_code(flag: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("state".into(), json!("enter_code"));
    if let Some(f) = flag {
        m.insert(f.into(), json!(true));
    }
    Value::Object(m)
}

/// `device_authorization_model`
fn authorization_model(
    state: &AppState,
    current_user: &Value,
    grant: &Grant,
    request_token: Option<&str>,
    approval_token: Option<&str>,
    no_trust_level: bool,
    invalid_code: bool,
) -> Value {
    json!({
        "state": "authorize",
        "request_token": request_token,
        "approval_token": approval_token,
        "no_trust_level": no_trust_level,
        "invalid_code": invalid_code,
        "device_auth": grant.device_auth_json(state),
        "current_user": current_user,
    })
}

/// The activation page's limiters on request-token lookups.
fn request_token_limiters(ip: &str, user_id: i32, token: &str) -> Vec<RateLimiter> {
    let token_key = if valid_request_token(token) {
        hash(token)
    } else {
        "invalid".to_string()
    };
    vec![
        RateLimiter::new(
            None,
            &format!("user-api-key-device-request-token-ip-{ip}"),
            DEVICE_REQUEST_TOKEN_LOOKUPS_PER_HOUR,
            3600,
        ),
        RateLimiter::new(
            None,
            &format!("user-api-key-device-request-token-user-{user_id}"),
            DEVICE_REQUEST_TOKEN_LOOKUPS_PER_MINUTE,
            60,
        ),
        RateLimiter::new(
            None,
            &format!("user-api-key-device-request-token-{token_key}"),
            DEVICE_REQUEST_TOKEN_LOOKUPS_PER_MINUTE,
            60,
        ),
    ]
}

/// The activation attempts' limiters, by address, member and code.
fn activation_limiters(ip: &str, user_id: i32, code: Option<&str>) -> Vec<RateLimiter> {
    let normalized = code
        .and_then(normalize_user_code)
        .unwrap_or_else(|| "invalid".to_string());
    vec![
        RateLimiter::new(
            None,
            &format!("user-api-key-device-activate-ip-{ip}"),
            DEVICE_ACTIVATION_ATTEMPTS_PER_HOUR,
            3600,
        ),
        RateLimiter::new(
            None,
            &format!("user-api-key-device-activate-user-{user_id}"),
            DEVICE_ACTIVATION_ATTEMPTS_PER_MINUTE,
            60,
        ),
        RateLimiter::new(
            None,
            &format!("user-api-key-device-activate-code-{}", hash(&normalized)),
            DEVICE_ACTIVATION_ATTEMPTS_PER_MINUTE,
            60,
        ),
    ]
}

/// Everything a device action reads from its request.
struct Ctx {
    state: AppState,
    settings: SiteSettings,
    guardian: Guardian,
    headers: HeaderMap,
    uri: Uri,
    ip: String,
    p: Map<String, Value>,
    /// The `_forum_session` cookie, for the approval tokens.
    session: crate::session::forum::ForumSession,
}

impl Ctx {
    async fn new(
        state: AppState,
        guardian: Guardian,
        headers: HeaderMap,
        peer: Option<std::net::SocketAddr>,
        uri: Uri,
        body: &[u8],
    ) -> Result<Ctx, AppError> {
        let settings = settings(&state).await?;
        let ip = crate::remote_ip::remote_ip(&headers, peer)?;
        let p = params::parse(uri.query(), &headers, body);
        let session = crate::session::forum::load_forum_session(&state, &headers);
        Ok(Ctx {
            state,
            settings,
            guardian,
            headers,
            uri,
            ip,
            p,
            session,
        })
    }

    fn param(&self, name: &str) -> Option<String> {
        params::string(&self.p, name).filter(|v| !v.trim().is_empty())
    }

    async fn approvals(&mut self, conn: &mut PgConnection) -> Result<Approvals, AppError> {
        let id = self.session.server_session_id();
        Approvals::load(conn, id).await
    }

    /// The response: JSON, or the activation page, with the session cookie
    /// when the approval tokens created one.
    async fn render(&self, r: Rendered) -> Result<Response, AppError> {
        let mut response = if wants_json(&self.uri, &self.headers) {
            Json(r.model).into_response()
        } else {
            activation_page(
                &self.state,
                &self.settings,
                &self.guardian,
                &self.headers,
                &self.uri,
                &r.model,
            )
            .await?
        };
        if let Some(cookie) = self.session.set_cookie(&self.state, &self.settings)? {
            response
                .headers_mut()
                .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
        }
        Ok(no_store(response))
    }
}

/// GET and POST /user-api-key/activate: the code entry, the request
/// link's preview, and a manual code's approval token.
pub async fn activate(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut cx = Ctx::new(state, guardian, headers, peer, uri, &body).await?;
    if method == Method::POST
        && !csrf_ok(
            &cx.state,
            &cx.headers,
            &form_pairs(&cx.p),
            cx.uri.path(),
            "POST",
        )
    {
        return Ok(bad_csrf());
    }
    let base_path = cx.state.config.globals.relative_url_root().to_string();
    let Some(user) = cx.guardian.user().cloned() else {
        let token = cx.param("request").unwrap_or_default();
        let destination = if valid_request_token(&token) {
            format!(
                "{base_path}/user-api-key/activate?request={}",
                form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>()
            )
        } else {
            format!("{base_path}/user-api-key/activate")
        };
        return Ok(no_store(redirect_to_login(
            &cx.state,
            &cx.settings,
            &cx.headers,
            &destination,
        )?));
    };
    let current_user = current_user_json(&cx.state, &cx.settings, &user)?;
    let mut conn = cx.state.pool.acquire().await?;
    if method != Method::POST {
        if cx.param("code").is_some() {
            let host = cx
                .headers
                .get(header::HOST)
                .and_then(|h| h.to_str().ok())
                .unwrap_or("localhost");
            return Ok(no_store(
                (
                    StatusCode::FOUND,
                    [(
                        header::LOCATION,
                        format!("http://{host}{base_path}/user-api-key/activate"),
                    )],
                )
                    .into_response(),
            ));
        }
        // device_activation_model
        let Some(token) = cx.param("request") else {
            return cx
                .render(Rendered {
                    model: enter_code(None),
                })
                .await;
        };
        if let Some(refused) =
            limit(&cx.state, request_token_limiters(&cx.ip, user.id, &token)).await?
        {
            return Ok(no_store(refused));
        }
        let grant = load_by_request_token(&mut conn, &token).await?;
        let model = match grant {
            Some(g) if !unavailable(Some(&g), user.id) => authorization_model(
                &cx.state,
                &current_user,
                &g,
                Some(&token),
                None,
                !meets_tl(&cx.settings, &cx.guardian)?,
                false,
            ),
            _ => enter_code(Some("expired_code")),
        };
        return cx.render(Rendered { model }).await;
    }
    let code = cx.param("code");
    if let Some(refused) = limit(
        &cx.state,
        activation_limiters(&cx.ip, user.id, code.as_deref()),
    )
    .await?
    {
        return Ok(no_store(refused));
    }
    // find_manual_code
    let Some(user_code) = code.as_deref().and_then(normalize_user_code) else {
        return cx
            .render(Rendered {
                model: enter_code(Some("invalid_code")),
            })
            .await;
    };
    let Some(grant) = load_by_user_code(&mut conn, &user_code).await? else {
        return cx
            .render(Rendered {
                model: enter_code(Some("invalid_code")),
            })
            .await;
    };
    if unavailable(Some(&grant), user.id) {
        return cx
            .render(Rendered {
                model: enter_code(Some("expired_code")),
            })
            .await;
    }
    if !meets_tl(&cx.settings, &cx.guardian)? {
        let model = authorization_model(&cx.state, &current_user, &grant, None, None, true, false);
        return cx.render(Rendered { model }).await;
    }
    // create_approval_token!: the grant bound to the member, under the lock.
    let device_code = grant.device_code.clone();
    let mut approval = None;
    if lock(&mut conn, &device_code).await? {
        let stored = load_grant(&mut conn, &device_code).await?;
        if !unavailable(stored.as_ref(), user.id)
            && let Some(mut stored) = stored
            && stored.bind_to_user(user.id, &user.username)
        {
            let ttl = ttl_for_update(&mut conn, &device_code).await?;
            save_grant(&mut conn, &stored, ttl).await?;
            let mut approvals = cx.approvals(&mut conn).await?;
            approval = Some(approvals.create(&mut conn, &device_code, user.id).await?);
        }
        unlock(&mut conn, &device_code).await?;
    }
    let model = match approval {
        Some(token) => authorization_model(
            &cx.state,
            &current_user,
            &grant,
            None,
            Some(&token),
            false,
            false,
        ),
        None => enter_code(Some("expired_code")),
    };
    cx.render(Rendered { model }).await
}

/// POST /user-api-key/device/authorize: the key created for the grant
/// (by request link and code, or by approval token) and its payload
/// stored for the poller.
pub async fn authorize_device_request(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut cx = Ctx::new(state, guardian, headers, peer, uri, &body).await?;
    if !csrf_ok(
        &cx.state,
        &cx.headers,
        &form_pairs(&cx.p),
        cx.uri.path(),
        "POST",
    ) {
        return Ok(bad_csrf());
    }
    let Some(user) = cx.guardian.user().cloned() else {
        return Ok(no_store(super::login_required::not_logged_in(&cx.state)));
    };
    if !meets_tl(&cx.settings, &cx.guardian)? {
        return Ok(no_store(super::search::invalid_access(&cx.state)));
    }
    let current_user = current_user_json(&cx.state, &cx.settings, &user)?;
    let mut conn = cx.state.pool.acquire().await?;
    // device_code_for_authorize_request
    let mut device_auth: Option<Grant> = None;
    let mut request_token = None;
    let device_code = if let Some(token) = cx.param("request") {
        if let Some(refused) =
            limit(&cx.state, request_token_limiters(&cx.ip, user.id, &token)).await?
        {
            return Ok(no_store(refused));
        }
        let preview = load_by_request_token(&mut conn, &token).await?;
        if !unavailable(preview.as_ref(), user.id) {
            device_auth = preview;
            request_token = Some(token.clone());
            let code = cx.param("code");
            if let Some(refused) = limit(
                &cx.state,
                activation_limiters(&cx.ip, user.id, code.as_deref()),
            )
            .await?
            {
                return Ok(no_store(refused));
            }
            resolve_request(&mut conn, &token, code.as_deref(), &user, &mut device_auth).await?
        } else {
            None
        }
    } else {
        let Some(approval_token) = required(&cx.p, "approval_token") else {
            return Ok(no_store(super::accounts::param_missing("approval_token")));
        };
        let approvals = cx.approvals(&mut conn).await?;
        approvals.device_code_for(&approval_token, user.id)
    };
    let Some(device_code) = device_code else {
        let model = match (&request_token, &device_auth) {
            (Some(token), Some(g))
                if g.application_name
                    .as_deref()
                    .is_some_and(|n| !n.trim().is_empty()) =>
            {
                authorization_model(&cx.state, &current_user, g, Some(token), None, false, true)
            }
            _ => enter_code(Some("expired_code")),
        };
        return cx.render(Rendered { model }).await;
    };
    // UserApiKey::DeviceAuth::Authorize
    if !lock(&mut conn, &device_code).await? {
        return Ok(no_store(super::search::invalid_access(&cx.state)));
    }
    let outcome = authorize_grant(&cx.state, &cx.settings, &mut conn, &device_code, user.id).await;
    unlock(&mut conn, &device_code).await?;
    match outcome? {
        Err(None) => {
            cx.render(Rendered {
                model: enter_code(Some("expired_code")),
            })
            .await
        }
        Err(Some(r)) => Ok(no_store(refusal(&cx.state, r))),
        Ok(()) => {
            if let Some(token) = cx.param("approval_token") {
                let mut approvals = cx.approvals(&mut conn).await?;
                approvals.delete(&mut conn, &token).await?;
            }
            cx.render(Rendered {
                model: json!({"state": "complete", "denied": false}),
            })
            .await
        }
    }
}

/// `resolve_request_authorize_device_code`: the request link's grant,
/// when the code entered is its user code; bound to the member.
async fn resolve_request(
    conn: &mut PgConnection,
    token: &str,
    code: Option<&str>,
    user: &crate::session::current::SessionUser,
    device_auth: &mut Option<Grant>,
) -> Result<Option<String>, AppError> {
    let grant = load_by_request_token(conn, token).await?;
    if unavailable(grant.as_ref(), user.id) {
        if grant.is_some() {
            *device_auth = grant;
        }
        return Ok(None);
    }
    let Some(grant) = grant else {
        return Ok(None);
    };
    let device_code = grant.device_code.clone();
    if !lock(conn, &device_code).await? {
        *device_auth = Some(grant);
        return Ok(None);
    }
    let result = async {
        let reloaded = load_by_request_token(conn, token).await?;
        if unavailable(reloaded.as_ref(), user.id) {
            if reloaded.is_some() {
                *device_auth = reloaded;
            }
            return Ok(None);
        }
        let Some(mut reloaded) = reloaded else {
            return Ok(None);
        };
        // user_code_matches_grant?
        let matches = match code.and_then(normalize_user_code) {
            Some(c) => cached_get(conn, &user_code_key(&c))
                .await?
                .is_some_and(|d| !d.is_empty() && d == reloaded.device_code),
            None => false,
        };
        if !matches {
            *device_auth = Some(reloaded);
            return Ok(None);
        }
        if !reloaded.bind_to_user(user.id, &user.username) {
            *device_auth = Some(reloaded);
            return Ok(None);
        }
        let ttl = ttl_for_update(conn, &reloaded.device_code).await?;
        save_grant(conn, &reloaded, ttl).await?;
        let code = reloaded.device_code.clone();
        *device_auth = Some(reloaded);
        Ok::<_, AppError>(Some(code))
    }
    .await;
    unlock(conn, &device_code).await?;
    result
}

/// The Authorize service's step, under the lock: Err(None) for a grant
/// gone, not pending or another member's; Err(Some) for what
/// KeyCreator's validation raises.
async fn authorize_grant(
    state: &AppState,
    settings: &SiteSettings,
    conn: &mut PgConnection,
    device_code: &str,
    user_id: i32,
) -> Result<Result<(), Option<Refusal>>, AppError> {
    let Some(mut grant) = load_grant(conn, device_code).await? else {
        return Ok(Err(None));
    };
    if !grant.pending() || grant.bound_to_another_user(user_id) {
        return Ok(Err(None));
    }
    // KeyCreator.validate_grant!: the allowed scopes, then the requested
    // scopes' own checks.
    let allowed = allowed_scopes(settings)?;
    if !grant.scopes.iter().all(|s| allowed.contains(s)) {
        return Ok(Err(Some(Refusal::Access)));
    }
    if let Err(r) = validate_requested_scopes(settings, &grant.scopes)? {
        return Ok(Err(Some(r)));
    }
    let client_id = grant.client_id.clone().unwrap_or_default();
    let client = find_client(conn, "client_id", &client_id).await?;
    if let Some(c) = &client
        && !c.scopes.is_empty()
        && !grant.scopes.iter().all(|s| c.scopes.contains(s))
    {
        return Ok(Err(Some(Refusal::Access)));
    }
    let public_key = match device_public_key(grant.public_key.as_deref()) {
        Ok(k) => k,
        Err(r) => return Ok(Err(Some(r))),
    };
    if let Some(p) = grant.padding.as_deref()
        && p != "pkcs1"
        && p != "oaep"
    {
        return Ok(Err(Some(Refusal::Params("padding".into()))));
    }
    let sample = payload_json(
        &"x".repeat(32),
        grant
            .nonce
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        false,
        grant.expires_at(),
    );
    if let Some(message) = crate::user_api_key_crypto::payload_too_large(
        &sample,
        &public_key,
        grant.padding.as_deref(),
    ) {
        return Ok(Err(Some(Refusal::Params(message))));
    }
    if grant.to_json().len() > MAX_DEVICE_GRANT_BYTES {
        return Ok(Err(Some(Refusal::Params("base".into()))));
    }
    // KeyCreator.create!: a new client takes the grant's name; a known
    // one keeps its own.
    let user = crate::session::current::SessionUser::load(conn, user_id)
        .await?
        .ok_or(Unsupported("authorizing a device for a user that vanished"))?;
    let expires_at = grant.expires_at();
    let mut tx = state.pool.begin().await?;
    let new_key = super::user_api_key_auth::NewKey {
        user_id: user.id,
        client_id: &client_id,
        client: client.as_ref(),
        application_name: if client.is_some() {
            None
        } else {
            grant.application_name.as_deref()
        },
        push_url: grant.push_url.as_deref(),
        scopes: &grant.scopes,
        expires_at,
    };
    let (key, _) = super::user_api_key_auth::create_key(&mut tx, &new_key).await?;
    tx.commit().await?;
    // PayloadBuilder.encrypted_payload!: the key's own expiry, whole
    // seconds.
    let push = has_push(settings, &grant.scopes, grant.push_url.as_deref())?;
    let payload = payload_json(
        &key,
        grant
            .nonce
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        push,
        expires_at,
    );
    if let Some(message) = crate::user_api_key_crypto::payload_too_large(
        &payload,
        &public_key,
        grant.padding.as_deref(),
    ) {
        return Ok(Err(Some(Refusal::Params(message))));
    }
    let Some(encrypted) = crate::user_api_key_crypto::encrypt(
        &public_key,
        payload.as_bytes(),
        grant.padding.as_deref(),
    ) else {
        return Ok(Err(Some(Refusal::Params("public_key".into()))));
    };
    grant.status = "authorized".into();
    grant.payload = Some(crate::user_api_key_crypto::encode64(&encrypted));
    grant.authorized_at = Some(now_iso());
    let ttl = ttl_for_update(conn, device_code)
        .await?
        .min(DEVICE_AUTHORIZED_PAYLOAD_TTL);
    save_grant(conn, &grant, ttl).await?;
    delete_indexes(conn, &grant).await?;
    Ok(Ok(()))
}

/// POST /user-api-key/device/deny: the grant denied, by approval token.
pub async fn deny_device_request(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut cx = Ctx::new(state, guardian, headers, peer, uri, &body).await?;
    if !csrf_ok(
        &cx.state,
        &cx.headers,
        &form_pairs(&cx.p),
        cx.uri.path(),
        "POST",
    ) {
        return Ok(bad_csrf());
    }
    let Some(user) = cx.guardian.user().cloned() else {
        return Ok(no_store(super::login_required::not_logged_in(&cx.state)));
    };
    let expired = Rendered {
        model: enter_code(Some("expired_code")),
    };
    if cx.param("request").is_some() {
        return cx.render(expired).await;
    }
    let Some(approval_token) = required(&cx.p, "approval_token") else {
        return Ok(no_store(super::accounts::param_missing("approval_token")));
    };
    let mut conn = cx.state.pool.acquire().await?;
    let mut approvals = cx.approvals(&mut conn).await?;
    let Some(device_code) = approvals.device_code_for(&approval_token, user.id) else {
        return cx.render(expired).await;
    };
    // UserApiKey::DeviceAuth::Deny
    if !lock(&mut conn, &device_code).await? {
        return cx.render(expired).await;
    }
    let ok = match load_grant(&mut conn, &device_code).await? {
        None => false,
        Some(mut g) if g.pending() => {
            g.status = "denied".into();
            g.denied_at = Some(now_iso());
            let ttl = ttl_for_update(&mut conn, &device_code).await?;
            save_grant(&mut conn, &g, ttl).await?;
            delete_indexes(&mut conn, &g).await?;
            true
        }
        Some(g) => g.denied(),
    };
    unlock(&mut conn, &device_code).await?;
    if !ok {
        return cx.render(expired).await;
    }
    approvals.delete(&mut conn, &approval_token).await?;
    cx.render(Rendered {
        model: json!({"state": "complete", "denied": true}),
    })
    .await
}

#[derive(askama::Template)]
#[template(path = "user_api_key_activate.html")]
struct ActivatePage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    /// enter_code, authorize or complete.
    kind: String,
    heading: String,
    errors: Vec<String>,
    /// The completed page's line, or the code field's description.
    note: String,
    no_trust_level: bool,
    logged_in_as: String,
    avatar_url: String,
    username: String,
    permissions_header: String,
    localized_scopes: Vec<String>,
    notices: Vec<String>,
    expiry: String,
    code_label: String,
    submit_label: String,
    deny_label: String,
    request_token: String,
    approval_token: String,
    show_form: bool,
    csrf_token: String,
}

/// user-api-key-activate as this port draws it, its forms posting to the
/// actions the Ember controller calls.
async fn activation_page(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    model: &Value,
) -> Result<Response, AppError> {
    use askama::Template;
    let i18n = &state.i18n;
    let t = |key: &str| {
        i18n.t(&format!("js.user_api_key.{key}"))
            .unwrap_or_default()
            .to_string()
    };
    let t_with = |key: &str, args: &[(&str, &str)]| {
        i18n.t_with(&format!("js.user_api_key.{key}"), args)
            .unwrap_or_default()
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let vs = super::session::viewer_state(state, headers, settings, guardian)?;
    let mut site = crate::html::Site::from_settings(settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    let mut conn = state.pool.acquire().await?;
    site.load_chrome(
        &mut conn,
        state,
        settings,
        guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    drop(conn);
    site.chrome.sidebar = String::new();
    site.chrome.show_footer = false;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    let csrf_token = site
        .viewer
        .as_ref()
        .map(|v| v.csrf_token.clone())
        .unwrap_or_default();
    let s = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let flag = |k: &str| model[k].as_bool() == Some(true);
    let mut page = ActivatePage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        kind: s(&model["state"]),
        heading: t("device.title"),
        errors: Vec::new(),
        note: t("device.enter_code"),
        no_trust_level: flag("no_trust_level"),
        logged_in_as: t("logged_in_as"),
        avatar_url: s(&model["current_user"]["avatar_template"]).replace("{size}", "24"),
        username: s(&model["current_user"]["username"]),
        permissions_header: String::new(),
        localized_scopes: Vec::new(),
        notices: Vec::new(),
        expiry: String::new(),
        code_label: t("device.code"),
        submit_label: t("device.continue"),
        deny_label: t("deny"),
        request_token: s(&model["request_token"]),
        approval_token: s(&model["approval_token"]),
        show_form: !flag("expired_code"),
        csrf_token,
    };
    if flag("expired_code") {
        page.errors.push(t("device.expired_code"));
    }
    if flag("invalid_code") {
        page.errors.push(t("device.invalid_code"));
    }
    match page.kind.as_str() {
        "complete" => {
            page.heading = if flag("denied") {
                t("device.denied")
            } else {
                t("device.complete")
            };
            page.note = t("device.return_to_cli");
        }
        "authorize" => {
            let auth = &model["device_auth"];
            let app = s(&auth["application_name"]);
            page.heading = t_with("device.authorize_title", &[("application_name", &app)]);
            if page.no_trust_level {
                page.errors.push(t("no_trust_level"));
            }
            page.permissions_header = t_with("permissions_header", &[("application_name", &app)]);
            page.localized_scopes = auth["localized_scopes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect();
            if auth["write_scope"].as_bool() == Some(true) {
                page.notices.push(t("write_scope_warning"));
            }
            if auth["unregistered_client"].as_bool() == Some(true) {
                page.notices.push(t("device.unregistered_app_warning"));
            }
            match auth["expires_at"]
                .as_str()
                .and_then(|v| chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%SZ").ok())
            {
                Some(at) => {
                    let shown = at.format("%B %-d, %Y %-I:%M %p").to_string();
                    page.expiry = t_with(
                        "device.expiry_notice",
                        &[("application_name", &app), ("expires_at", &shown)],
                    );
                }
                None => page.notices.push(t("device.no_expiry_warning")),
            }
            page.submit_label = t("authorize");
        }
        _ => {}
    }
    let html = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::with_viewer_headers(
        axum::response::Html(html).into_response(),
        &vs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_normalize_like_rails() {
        assert_eq!(
            normalize_user_code("abcd-efgh").as_deref(),
            Some("ABCD-EFGH")
        );
        assert_eq!(
            normalize_user_code(" ab cd ef gh ").as_deref(),
            Some("ABCD-EFGH")
        );
        assert_eq!(normalize_user_code("abc"), None);
        let code = random_user_code();
        assert_eq!(code.len(), 9);
        assert_eq!(normalize_user_code(&code), Some(code));
        assert!(valid_request_token(&random_request_token()));
        assert!(valid_device_code(&random_hex64()));
    }
}
