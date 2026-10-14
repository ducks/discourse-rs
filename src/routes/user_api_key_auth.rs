//! The user API key authorization flow: UserApiKeysController#new,
//! #create, #otp and #create_otp, and SessionController#one_time_password
//! (the login a one-time password makes).
//!
//! The application sends the member to /user-api-key/new with its client
//! id, nonce, scopes and RSA public key; the member authorizes; the key
//! goes back to the application encrypted to that key, in a redirect or
//! for the member to paste.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::accounts::param_missing;
use super::session::{bad_csrf, csrf_ok};
use super::user_api_keys::{form_pairs, required};
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::{AuthGuardian, SessionUser};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `AUTH_API_VERSION`
const AUTH_API_VERSION: i64 = 4;

/// `set_device_auth_no_store`
pub(super) fn no_store(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(header::EXPIRES, HeaderValue::from_static("0"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response
}

/// Why a request was turned away before it did anything.
pub(super) enum Refusal {
    /// ActionController::ParameterMissing
    Missing(&'static str),
    /// Discourse::InvalidAccess
    Access,
    /// Discourse::InvalidParameters, with its message.
    Params(String),
}

/// A refusal raised from an action (no rescue): 400 or 403.
pub(super) fn refusal(state: &AppState, r: Refusal) -> Response {
    match r {
        Refusal::Missing(name) => param_missing(name),
        Refusal::Access => super::search::invalid_access(state),
        Refusal::Params(m) => super::search::invalid_parameters(state, &m),
    }
}

/// The registered client a request names (find_client or
/// find_client_by_public_key).
pub(super) struct Client {
    pub id: i64,
    pub application_name: String,
    pub public_key: Option<String>,
    pub auth_redirect: Option<String>,
    pub scopes: Vec<String>,
}

pub(super) async fn find_client(
    conn: &mut sqlx::PgConnection,
    column: &str,
    value: &str,
) -> Result<Option<Client>, AppError> {
    let row: Option<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(&format!(
        "SELECT id, application_name, public_key, auth_redirect FROM user_api_key_clients \
         WHERE {column} = $1 ORDER BY id LIMIT 1"
    ))
    .bind(value)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((id, application_name, public_key, auth_redirect)) = row else {
        return Ok(None);
    };
    let scopes: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM user_api_key_client_scopes WHERE user_api_key_client_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    Ok(Some(Client {
        id,
        application_name,
        public_key,
        auth_redirect,
        scopes,
    }))
}

fn present(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// The request's checked parameters.
struct Authorization {
    p: Map<String, Value>,
    client: Option<Client>,
    scopes: String,
    public_key: Option<rsa::RsaPublicKey>,
    expires_in_seconds: Option<i64>,
}

impl Authorization {
    fn param(&self, name: &str) -> Option<String> {
        params::string(&self.p, name)
    }

    fn raw(&self, name: &str) -> Value {
        self.p.get(name).cloned().unwrap_or(Value::Null)
    }

    fn padding(&self) -> Option<String> {
        self.param("padding").filter(|p| !p.trim().is_empty())
    }
}

/// `UserApiKey.allowed_scopes` (allow_user_api_key_scopes).
pub(super) fn allowed_scopes(settings: &SiteSettings) -> Result<Vec<String>, AppError> {
    Ok(settings
        .get("allow_user_api_key_scopes")?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Ruby's `split(",")`: trailing empty pieces dropped.
pub(super) fn split_scopes(scopes: &str) -> Vec<String> {
    let mut out: Vec<String> = scopes.split(',').map(str::to_string).collect();
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// `validate_padding`
pub(super) fn validate_padding(p: &Map<String, Value>) -> Result<(), Refusal> {
    match params::string(p, "padding") {
        Some(v) if !v.trim().is_empty() && v != "pkcs1" && v != "oaep" => {
            Err(Refusal::Params("padding".into()))
        }
        _ => Ok(()),
    }
}

/// `validate_auth_redirect`: when the request names one, it must be the
/// registered client's (if it has one) and match
/// allowed_user_api_auth_redirects.
pub(super) fn validate_auth_redirect(
    settings: &SiteSettings,
    p: &Map<String, Value>,
    client: Option<&Client>,
) -> Result<Result<(), Refusal>, AppError> {
    if !p.contains_key("auth_redirect") {
        return Ok(Ok(()));
    }
    let wanted = params::string(p, "auth_redirect");
    if let Some(c) = client
        && present(&c.auth_redirect)
        && wanted != c.auth_redirect
    {
        return Ok(Err(Refusal::Access));
    }
    let allowed = settings.get("allowed_user_api_auth_redirects")?.to_s();
    let ok = wanted.as_deref().is_some_and(|w| {
        allowed
            .split('|')
            .filter(|u| !u.is_empty())
            .any(|u| crate::wildcard_url::check_url(u, w))
    });
    Ok(if ok { Ok(()) } else { Err(Refusal::Access) })
}

/// `UserApiKey::Expiry.parse_seconds!`
pub(super) fn parse_expires_in_seconds(
    settings: &SiteSettings,
    p: &Map<String, Value>,
) -> Result<Result<Option<i64>, Refusal>, AppError> {
    let Some(value) = params::string(p, "expires_in_seconds").filter(|v| !v.trim().is_empty())
    else {
        return Ok(Ok(None));
    };
    let refused = Ok(Err(Refusal::Params("expires_in_seconds".into())));
    if value.len() > 10 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return refused;
    }
    let Ok(seconds) = value.parse::<i64>() else {
        return refused;
    };
    let max = settings.get("max_user_api_key_expiry_days")?.to_i() * 86_400;
    if seconds <= 0 || max <= 0 || seconds > max {
        return refused;
    }
    Ok(Ok(Some(seconds)))
}

/// require_params, find_client, require_client_params, validate_params,
/// validate_auth_redirect and parse_expires_in_seconds!, in that order.
async fn authorization(
    state: &AppState,
    settings: &SiteSettings,
    p: Map<String, Value>,
) -> Result<Result<Authorization, Refusal>, AppError> {
    for name in ["nonce", "scopes", "client_id"] {
        if required(&p, name).is_none() {
            return Ok(Err(Refusal::Missing(name)));
        }
    }
    let client_id = params::string(&p, "client_id").unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let client = find_client(&mut conn, "client_id", &client_id).await?;
    drop(conn);
    if !client.as_ref().is_some_and(|c| present(&c.public_key))
        && required(&p, "public_key").is_none()
    {
        return Ok(Err(Refusal::Missing("public_key")));
    }
    if !client
        .as_ref()
        .is_some_and(|c| !c.application_name.trim().is_empty())
        && required(&p, "application_name").is_none()
    {
        return Ok(Err(Refusal::Missing("application_name")));
    }
    // validate_params
    let scopes = params::string(&p, "scopes").unwrap_or_default();
    let requested = split_scopes(&scopes);
    let allowed = allowed_scopes(settings)?;
    if !requested.iter().all(|s| allowed.contains(s)) {
        return Ok(Err(Refusal::Access));
    }
    if let Some(c) = &client
        && !c.scopes.is_empty()
        && !requested.iter().all(|s| c.scopes.contains(s))
    {
        return Ok(Err(Refusal::Access));
    }
    let key_str = client
        .as_ref()
        .and_then(|c| c.public_key.clone())
        .filter(|k| !k.trim().is_empty())
        .or_else(|| params::string(&p, "public_key"));
    let public_key = match key_str.filter(|k| !k.trim().is_empty()) {
        Some(k) => match crate::user_api_key_crypto::parse_public_key(&k) {
            Some(key) => Some(key),
            None => return Ok(Err(Refusal::Params("public_key".into()))),
        },
        None => None,
    };
    if let Err(r) = validate_padding(&p) {
        return Ok(Err(r));
    }
    if let Err(r) = validate_auth_redirect(settings, &p, client.as_ref())? {
        return Ok(Err(r));
    }
    let expires_in_seconds = match parse_expires_in_seconds(settings, &p)? {
        Ok(s) => s,
        Err(r) => return Ok(Err(r)),
    };
    Ok(Ok(Authorization {
        p,
        client,
        scopes,
        public_key,
        expires_in_seconds,
    }))
}

/// `meets_tl?`: staff, or in user_api_key_allowed_groups.
pub(super) fn meets_tl(settings: &SiteSettings, guardian: &Guardian) -> Result<bool, AppError> {
    Ok(guardian.is_staff()
        || guardian.in_setting_groups(settings, "user_api_key_allowed_groups")?)
}

/// `redirect_anonymous_to_login`: the destination kept in
/// destination_url, then the login page.
pub(super) fn redirect_to_login(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    destination: &str,
) -> Result<Response, AppError> {
    if settings.get("enable_discourse_connect")?.truthy() {
        return Err(Unsupported("DiscourseConnect logins").into());
    }
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let base_path = state.config.globals.relative_url_root();
    let cookie = format!(
        "destination_url={}; path=/; SameSite=Lax",
        form_urlencoded::byte_serialize(destination.as_bytes()).collect::<String>()
    );
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, format!("http://{host}{base_path}/login")),
            (header::SET_COOKIE, cookie),
        ],
    )
        .into_response())
}

/// `current_user_json`
pub(super) fn current_user_json(
    state: &AppState,
    settings: &SiteSettings,
    user: &SessionUser,
) -> Result<Value, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let avatar = crate::avatar::avatar_template(
        &urls,
        user.id,
        &user.username,
        user.uploaded_avatar_id,
        None,
    )?;
    Ok(json!({"username": user.username, "avatar_template": avatar}))
}

/// `redirect_uri_for`: the redirect's host (and port unless 80 or 443),
/// none for the app's own scheme.
fn redirect_uri_for(auth_redirect: Option<&str>) -> Value {
    let Some(url) = auth_redirect.filter(|u| !u.trim().is_empty()) else {
        return Value::Null;
    };
    if url == "discourse://auth_redirect" {
        return Value::Null;
    }
    let Some(uri) = crate::wildcard_url::parse(url) else {
        return Value::Null;
    };
    let Some(host) = uri.host else {
        return Value::Null;
    };
    match uri.port {
        None | Some(80) | Some(443) => json!(host),
        Some(port) => json!(format!("{host}:{port}")),
    }
}

/// `Time#iso8601` in UTC, whole seconds.
pub(super) fn iso8601(t: chrono::NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `I18n.t("user_api_key.scopes.<name>")` for each scope.
pub(super) fn localized_scopes(state: &AppState, scopes: &[String]) -> Vec<String> {
    scopes
        .iter()
        .map(|s| {
            let key = format!("user_api_key.scopes.{s}");
            state
                .i18n
                .t(&key)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Translation missing: en.{key}"))
        })
        .collect()
}

pub(super) fn wants_json(uri: &Uri, headers: &HeaderMap) -> bool {
    uri.path().ends_with(".json") || super::wants_json(uri.query(), headers)
}

async fn load_settings(state: &AppState) -> Result<SiteSettings, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// GET (and HEAD) /user-api-key/new: what the application asks for, for
/// the member to authorize.
pub async fn new(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if method == Method::HEAD {
        let mut response = no_store(StatusCode::OK.into_response());
        response
            .headers_mut()
            .insert("auth-api-version", HeaderValue::from(AUTH_API_VERSION));
        response
            .headers_mut()
            .insert("auth-api-device-code", HeaderValue::from_static("true"));
        return Ok(response);
    }
    let settings = load_settings(&state).await?;
    let p = params::parse(uri.query(), &headers, &[]);
    let model = match authorization(&state, &settings, p).await? {
        Err(Refusal::Missing(name)) => return Ok(no_store(param_missing(name))),
        Err(Refusal::Params(m)) if m == "padding" => {
            return Ok(no_store(super::search::invalid_parameters(&state, &m)));
        }
        Err(_) => json!({"state": "generic_error"}),
        Ok(a) => {
            let Some(user) = guardian.user() else {
                let fullpath = uri.path_and_query().map_or("/", |p| p.as_str());
                return Ok(no_store(redirect_to_login(
                    &state, &settings, &headers, fullpath,
                )?));
            };
            let application_name = a
                .param("application_name")
                .or_else(|| a.client.as_ref().map(|c| c.application_name.clone()));
            let current_user = current_user_json(&state, &settings, user)?;
            if !meets_tl(&settings, &guardian)? {
                json!({
                    "state": "no_trust_level",
                    "application_name": application_name,
                    "current_user": current_user,
                })
            } else {
                let expires_at = a
                    .expires_in_seconds
                    .map(|s| crate::clock::now_naive() + chrono::Duration::seconds(s));
                let public_key = a
                    .param("public_key")
                    .or_else(|| a.client.as_ref().and_then(|c| c.public_key.clone()));
                let scopes = split_scopes(&a.scopes);
                json!({
                    "state": "ready",
                    "application_name": application_name,
                    "public_key": public_key,
                    "nonce": a.raw("nonce"),
                    "client_id": a.raw("client_id"),
                    "auth_redirect": a.raw("auth_redirect"),
                    "redirect_uri": redirect_uri_for(a.param("auth_redirect").as_deref()),
                    "push_url": a.raw("push_url"),
                    "localized_scopes": localized_scopes(&state, &scopes),
                    "scopes": a.scopes,
                    "write_scope": scopes.iter().any(|s| s == "write"),
                    "padding": a.raw("padding"),
                    "expires_in_seconds": a.expires_in_seconds,
                    "expires_at": expires_at.map(iso8601),
                    "current_user": current_user,
                })
            }
        }
    };
    if wants_json(&uri, &headers) {
        return Ok(no_store(Json(model).into_response()));
    }
    page(
        &state,
        &settings,
        &guardian,
        &headers,
        &uri,
        Page::Authorize(model),
    )
    .await
}

/// `UserApiKey#has_push?`: allowed_user_api_push_urls is matched as a
/// string (`include?`).
pub(super) fn has_push(
    settings: &SiteSettings,
    scopes: &[String],
    push_url: Option<&str>,
) -> Result<bool, AppError> {
    let Some(url) = push_url.filter(|u| !u.trim().is_empty()) else {
        return Ok(false);
    };
    Ok(scopes.iter().any(|s| s == "push" || s == "notifications")
        && settings
            .get("allowed_user_api_push_urls")?
            .to_s()
            .contains(url))
}

/// What creating a key needs from the request.
pub(super) struct NewKey<'a> {
    pub user_id: i32,
    pub client_id: &'a str,
    pub client: Option<&'a Client>,
    pub application_name: Option<&'a str>,
    pub push_url: Option<&'a str>,
    pub scopes: &'a [String],
    pub expires_at: Option<chrono::NaiveDateTime>,
}

/// The client found or created (named anew when the request names it
/// otherwise), the user's earlier keys for it destroyed with their
/// scopes, and the new key with its scopes; returns the key and the
/// client's id.
pub(super) async fn create_key(
    conn: &mut sqlx::PgConnection,
    k: &NewKey<'_>,
) -> Result<(String, i64), AppError> {
    let client = match k.client {
        None => {
            sqlx::query_scalar::<_, i64>(
                "INSERT INTO user_api_key_clients (client_id, application_name, created_at, updated_at) \
                 VALUES ($1, $2, now(), now()) RETURNING id",
            )
            .bind(k.client_id)
            .bind(k.application_name)
            .fetch_one(&mut *conn)
            .await?
        }
        Some(c) => {
            if let Some(name) = k.application_name
                && name != c.application_name
            {
                sqlx::query(
                    "UPDATE user_api_key_clients SET application_name = $2, updated_at = now() WHERE id = $1",
                )
                .bind(c.id)
                .bind(name)
                .execute(&mut *conn)
                .await?;
            }
            c.id
        }
    };
    sqlx::query(
        "DELETE FROM user_api_key_scopes WHERE user_api_key_id IN \
           (SELECT id FROM user_api_keys WHERE user_api_key_client_id = $1 AND user_id = $2)",
    )
    .bind(client)
    .bind(k.user_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM user_api_keys WHERE user_api_key_client_id = $1 AND user_id = $2")
        .bind(client)
        .bind(k.user_id)
        .execute(&mut *conn)
        .await?;
    let key = crate::accounts::random_hex();
    let key_id: i32 = sqlx::query_scalar(
        "INSERT INTO user_api_keys (user_id, push_url, created_at, updated_at, key_hash, user_api_key_client_id, expires_at) \
         VALUES ($1, $2, now(), now(), $3, $4, $5) RETURNING id",
    )
    .bind(k.user_id)
    .bind(k.push_url)
    .bind(crate::session::user_api_key::hash_key(&key))
    .bind(client)
    .bind(k.expires_at)
    .fetch_one(&mut *conn)
    .await?;
    for scope in k.scopes {
        sqlx::query(
            "INSERT INTO user_api_key_scopes (user_api_key_id, name, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(key_id)
        .bind(scope)
        .execute(&mut *conn)
        .await?;
    }
    Ok((key, client))
}

/// The payload the application decrypts: `{key, nonce, push, api,
/// expires_at}` as JSON.
pub(super) fn payload_json(
    key: &str,
    nonce: Value,
    push: bool,
    expires_at: Option<chrono::NaiveDateTime>,
) -> String {
    let mut payload = Map::new();
    payload.insert("key".into(), json!(key));
    payload.insert("nonce".into(), nonce);
    payload.insert("push".into(), json!(push));
    payload.insert("api".into(), json!(AUTH_API_VERSION));
    if let Some(t) = expires_at {
        payload.insert("expires_at".into(), json!(iso8601(t)));
    }
    Value::Object(payload).to_string()
}

/// POST /user-api-key: the key, destroying the member's earlier keys for
/// the client, its payload encrypted to the application's public key.
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
    let Some(user) = guardian.user().cloned() else {
        return Ok(no_store(super::login_required::not_logged_in(&state)));
    };
    let settings = load_settings(&state).await?;
    let a = match authorization(&state, &settings, p).await? {
        Ok(a) => a,
        Err(r) => return Ok(no_store(refusal(&state, r))),
    };
    if !meets_tl(&settings, &guardian)? {
        return Ok(no_store(super::search::invalid_access(&state)));
    }
    let scopes = split_scopes(&a.scopes);
    let expires_at = a
        .expires_in_seconds
        .map(|s| crate::clock::now_naive() + chrono::Duration::seconds(s));
    let client_id = a.param("client_id").unwrap_or_default();
    let application_name = a.param("application_name").filter(|n| !n.trim().is_empty());
    let push_url = a.param("push_url");
    let mut tx = state.pool.begin().await?;
    let (key, client) = create_key(
        &mut tx,
        &NewKey {
            user_id: user.id,
            client_id: &client_id,
            client: a.client.as_ref(),
            application_name: application_name.as_deref(),
            push_url: push_url.as_deref(),
            scopes: &scopes,
            expires_at,
        },
    )
    .await?;
    tx.commit().await?;
    let push = has_push(&settings, &scopes, push_url.as_deref())?;
    let payload = payload_json(&key, a.raw("nonce"), push, expires_at);
    let public_key = a
        .public_key
        .as_ref()
        .ok_or(Unsupported("a key request without a public key"))?;
    let padding = a.padding();
    if let Some(message) =
        crate::user_api_key_crypto::payload_too_large(&payload, public_key, padding.as_deref())
    {
        return Ok(no_store(super::search::invalid_parameters(
            &state, &message,
        )));
    }
    let Some(encrypted) =
        crate::user_api_key_crypto::encrypt(public_key, payload.as_bytes(), padding.as_deref())
    else {
        return Ok(no_store(super::search::invalid_parameters(
            &state,
            "public_key",
        )));
    };
    let payload = crate::user_api_key_crypto::encode64(&encrypted);
    let otp = if scopes.iter().any(|s| s == "one_time_password") {
        match one_time_password(
            &state,
            &settings,
            public_key,
            padding.as_deref(),
            &user.username,
        )
        .await?
        {
            Ok(otp) => Some(otp),
            Err(r) => return Ok(no_store(refusal(&state, r))),
        }
    } else {
        None
    };
    let json = wants_json(&uri, &headers);
    let Some(auth_redirect) = a.param("auth_redirect") else {
        let application_name: String =
            sqlx::query_scalar("SELECT application_name FROM user_api_key_clients WHERE id = $1")
                .bind(client)
                .fetch_one(&state.pool)
                .await?;
        let instructions = state
            .i18n
            .t_with(
                "user_api_key.instructions",
                &[("application_name", &application_name)],
            )
            .unwrap_or_default();
        let result = json!({"payload": payload, "instructions": instructions});
        if json {
            return Ok(no_store(Json(result).into_response()));
        }
        return page(
            &state,
            &settings,
            &guardian,
            &headers,
            &uri,
            Page::Result(result),
        )
        .await;
    };
    let mut extra = format!("payload={}", cgi_escape(&payload));
    if let Some(otp) = otp {
        extra.push_str(&format!("&oneTimePassword={}", cgi_escape(&otp)));
    }
    let redirect_url = crate::wildcard_url::append_query(&auth_redirect, &extra);
    if json {
        return Ok(no_store(
            Json(json!({"redirect_url": redirect_url})).into_response(),
        ));
    }
    Ok(no_store(found(&redirect_url)))
}

/// `CGI.escape` (for base64 text: `+`, `/`, `=` and newlines escaped).
fn cgi_escape(s: &str) -> String {
    form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn found(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, location.to_string())],
    )
        .into_response()
}

/// `one_time_password`: a token the application can log the member in
/// with once in the next 10 minutes (a ServerSession row standing in for
/// the `otp_<token>` Redis key), encrypted to its key.
async fn one_time_password(
    state: &AppState,
    settings: &SiteSettings,
    public_key: &rsa::RsaPublicKey,
    padding: Option<&str>,
    username: &str,
) -> Result<Result<String, Refusal>, AppError> {
    if !allowed_scopes(settings)?
        .iter()
        .any(|s| s == "one_time_password")
    {
        return Ok(Err(Refusal::Access));
    }
    let otp = crate::accounts::random_hex();
    let mut conn = state.pool.acquire().await?;
    crate::session::server::set(&mut conn, "", &format!("otp_{otp}"), &json!(username), 600)
        .await?;
    let Some(encrypted) = crate::user_api_key_crypto::encrypt(public_key, otp.as_bytes(), padding)
    else {
        return Ok(Err(Refusal::Params("public_key".into())));
    };
    Ok(Ok(crate::user_api_key_crypto::encode64(&encrypted)))
}

/// require_params_otp, find_client_by_public_key, validate_params_otp and
/// validate_auth_redirect.
async fn otp_params(
    state: &AppState,
    settings: &SiteSettings,
    p: &Map<String, Value>,
) -> Result<Result<rsa::RsaPublicKey, Refusal>, AppError> {
    for name in ["public_key", "auth_redirect", "application_name"] {
        if required(p, name).is_none() {
            return Ok(Err(Refusal::Missing(name)));
        }
    }
    let public_key = params::string(p, "public_key").unwrap_or_default();
    let mut conn = state.pool.acquire().await?;
    let client = find_client(&mut conn, "public_key", &public_key).await?;
    drop(conn);
    let Some(key) = crate::user_api_key_crypto::parse_public_key(&public_key) else {
        return Ok(Err(Refusal::Params("public_key".into())));
    };
    if let Err(r) = validate_padding(p) {
        return Ok(Err(r));
    }
    if let Err(r) = validate_auth_redirect(settings, p, client.as_ref())? {
        return Ok(Err(r));
    }
    Ok(Ok(key))
}

/// GET /user-api-key/otp: the application asking to log the member in.
pub async fn otp(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let settings = load_settings(&state).await?;
    let p = params::parse(uri.query(), &headers, &[]);
    if let Err(r) = otp_params(&state, &settings, &p).await? {
        return Ok(no_store(refusal(&state, r)));
    }
    if guardian.user().is_none() {
        let fullpath = uri.path_and_query().map_or("/", |p| p.as_str());
        return Ok(no_store(redirect_to_login(
            &state, &settings, &headers, fullpath,
        )?));
    }
    let get = |k: &str| p.get(k).cloned().unwrap_or(Value::Null);
    let model = json!({
        "application_name": get("application_name"),
        "public_key": get("public_key"),
        "auth_redirect": get("auth_redirect"),
        "padding": get("padding"),
    });
    if wants_json(&uri, &headers) {
        return Ok(no_store(Json(model).into_response()));
    }
    page(
        &state,
        &settings,
        &guardian,
        &headers,
        &uri,
        Page::Otp(model),
    )
    .await
}

/// POST /user-api-key/otp: the one-time password, sent to the
/// application's redirect.
pub async fn create_otp(
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
    let Some(user) = guardian.user().cloned() else {
        return Ok(no_store(super::login_required::not_logged_in(&state)));
    };
    let settings = load_settings(&state).await?;
    let key = match otp_params(&state, &settings, &p).await? {
        Ok(k) => k,
        Err(r) => return Ok(no_store(refusal(&state, r))),
    };
    if !meets_tl(&settings, &guardian)? {
        return Ok(no_store(super::search::invalid_access(&state)));
    }
    let padding = params::string(&p, "padding").filter(|v| !v.trim().is_empty());
    let otp = match one_time_password(&state, &settings, &key, padding.as_deref(), &user.username)
        .await?
    {
        Ok(otp) => otp,
        Err(r) => return Ok(no_store(refusal(&state, r))),
    };
    let auth_redirect = params::string(&p, "auth_redirect").unwrap_or_default();
    let redirect = crate::wildcard_url::append_query(
        &auth_redirect,
        &format!("oneTimePassword={}", cgi_escape(&otp)),
    );
    if wants_json(&uri, &headers) {
        return Ok(no_store(
            Json(json!({"redirect_url": redirect})).into_response(),
        ));
    }
    Ok(no_store(found(&redirect)))
}

/// What the authorization page shows.
enum Page {
    Authorize(Value),
    Result(Value),
    Otp(Value),
}

#[derive(askama::Template)]
#[template(path = "user_api_key.html")]
struct UserApiKeyPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    kind: &'static str,
    heading: String,
    error: String,
    instructions: String,
    payload: String,
    logged_in_as: String,
    avatar_url: String,
    username: String,
    permissions_header: String,
    localized_scopes: Vec<String>,
    write_warning: String,
    expiry_notice: String,
    redirect_warning: String,
    redirect_uri: String,
    authorize_label: String,
    deny_label: String,
    csrf_token: String,
    /// The hidden fields the form posts back (payloadParameters).
    fields: Vec<(String, String)>,
}

/// The Ember pages as this port draws them (user-api-key-new, -result
/// and -otp, with the sidebar and footer hidden); the buttons are forms
/// posting to the actions the Ember controllers call.
async fn page(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    page: Page,
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
    site.chrome.powered_by = false;
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
    let s = |v: &Value, k: &str| match &v[k] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut out = UserApiKeyPage {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        kind: "authorize",
        heading: String::new(),
        error: String::new(),
        instructions: String::new(),
        payload: String::new(),
        logged_in_as: t("logged_in_as"),
        avatar_url: String::new(),
        username: String::new(),
        permissions_header: String::new(),
        localized_scopes: Vec::new(),
        write_warning: String::new(),
        expiry_notice: String::new(),
        redirect_warning: t("redirect_warning"),
        redirect_uri: String::new(),
        authorize_label: t("authorize"),
        deny_label: t("deny"),
        csrf_token,
        fields: Vec::new(),
    };
    match &page {
        Page::Result(v) => {
            out.kind = "result";
            out.heading = i18n
                .t("js.user_api_key.device.title")
                .unwrap_or_default()
                .to_string();
            out.instructions = s(v, "instructions");
            out.payload = s(v, "payload");
        }
        Page::Otp(v) => {
            out.kind = "otp";
            out.heading = t_with(
                "otp_description",
                &[("application_name", &s(v, "application_name"))],
            );
            for k in ["application_name", "public_key", "auth_redirect", "padding"] {
                let value = s(v, k);
                if k != "padding" || !value.is_empty() {
                    out.fields.push((k.to_string(), value));
                }
            }
        }
        Page::Authorize(v) => {
            let app = s(v, "application_name");
            out.heading = if app.is_empty() {
                i18n.t("js.user_api_key.device.title")
                    .unwrap_or_default()
                    .to_string()
            } else {
                t_with("title", &[("application_name", &app)])
            };
            match v["state"].as_str() {
                Some("no_trust_level") => {
                    out.kind = "no_trust_level";
                    out.error = t("no_trust_level");
                }
                Some("ready") => {
                    out.username = s(&v["current_user"], "username");
                    out.avatar_url =
                        s(&v["current_user"], "avatar_template").replace("{size}", "24");
                    out.permissions_header =
                        t_with("permissions_header", &[("application_name", &app)]);
                    out.localized_scopes = v["localized_scopes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect();
                    if v["write_scope"].as_bool() == Some(true) {
                        out.write_warning = t("write_scope_warning");
                    }
                    if let Some(at) = v["expires_at"].as_str().and_then(|t| {
                        chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%SZ").ok()
                    }) {
                        // longDate: moment's "LLL", in UTC.
                        let shown = at.format("%B %-d, %Y %-I:%M %p").to_string();
                        out.expiry_notice = t_with(
                            "expiry_notice",
                            &[("application_name", &app), ("expires_at", &shown)],
                        );
                    }
                    out.redirect_uri = s(v, "redirect_uri");
                    for k in [
                        "application_name",
                        "nonce",
                        "client_id",
                        "push_url",
                        "public_key",
                        "scopes",
                    ] {
                        out.fields.push((k.to_string(), s(v, k)));
                    }
                    for k in ["auth_redirect", "padding", "expires_in_seconds"] {
                        let value = s(v, k);
                        if !value.is_empty() {
                            out.fields.push((k.to_string(), value));
                        }
                    }
                }
                _ => {
                    out.kind = "generic_error";
                    out.error = t("generic_error");
                }
            }
        }
    }
    let html = out.render().map_err(crate::html::HtmlError::from)?;
    let response = axum::response::Html(html).into_response();
    Ok(no_store(crate::html::with_viewer_headers(response, &vs)))
}

/// GET and POST /session/otp/:token: the login a one-time password makes.
/// The member it names is logged in on POST (the form the GET shows),
/// already-logged-in as them goes home; an unknown token shows the error.
#[allow(clippy::too_many_arguments)]
pub async fn session_otp(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: Method,
    Path(token): Path<String>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // :token is constrained to hex.
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(super::topics::not_found_response(&state));
    }
    if method == Method::POST {
        let p = params::parse(uri.query(), &headers, &body);
        if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
            return Ok(bad_csrf());
        }
    }
    let settings = load_settings(&state).await?;
    let key = format!("otp_{token}");
    let mut conn = state.pool.acquire().await?;
    let username = crate::session::server::get(&mut conn, "", &key)
        .await?
        .and_then(|v| v.as_str().map(str::to_string));
    let user = match &username {
        Some(name) => {
            let id: Option<i32> =
                sqlx::query_scalar("SELECT id FROM users WHERE username_lower = lower($1) LIMIT 1")
                    .bind(name)
                    .fetch_optional(&mut *conn)
                    .await?;
            match id {
                Some(id) => SessionUser::load(&mut conn, id).await?,
                None => None,
            }
        }
        None => None,
    };
    let base_path = state.config.globals.relative_url_root();
    let home = format!("{base_path}/");
    if let (Some(name), Some(user)) = (&username, user) {
        if guardian.user().is_some_and(|u| u.username == *name) {
            crate::session::server::delete(&mut conn, "", &key).await?;
            return Ok(found(&home));
        }
        if method == Method::POST {
            let cookie =
                log_on_user(&state, &settings, &mut conn, &headers, peer, &uri, &user).await?;
            crate::session::server::delete(&mut conn, "", &key).await?;
            let mut response = found(&home);
            response
                .headers_mut()
                .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
            return Ok(response);
        }
    }
    drop(conn);
    otp_page(
        &state,
        &settings,
        &guardian,
        &headers,
        &uri,
        username.as_deref(),
    )
    .await
}

/// `log_on_user(user)`: a new auth token and its cookie, the user
/// unstaged, the first admin's moderation, the session count kept.
async fn log_on_user(
    state: &AppState,
    settings: &SiteSettings,
    conn: &mut sqlx::PgConnection,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: &Uri,
    user: &SessionUser,
) -> Result<String, AppError> {
    if crate::session::current::cookie(headers, "anonymous_action").is_some() {
        return Err(Unsupported("replaying an anonymous action after a login").into());
    }
    let ip = crate::remote_ip::remote_ip(headers, peer)?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let (_, unhashed) = crate::session::token::generate(
        &mut *conn,
        user.id,
        &state.keys.secret_key_base,
        user_agent.as_deref(),
        &ip,
        Some(uri.path()),
    )
    .await?;
    if user.staged {
        sqlx::query("UPDATE users SET staged = false WHERE id = $1")
            .bind(user.id)
            .execute(&mut *conn)
            .await?;
    }
    crate::roles::bootstrap_first_admin(&mut *conn, &state.i18n, user.id).await?;
    crate::session::token::enforce_session_count_limit(
        &mut *conn,
        user.id,
        settings.get("maximum_session_age")?.to_i(),
    )
    .await?;
    crate::session::current::auth_cookie(&state.keys, settings, user, &unhashed)
}

#[derive(askama::Template)]
#[template(path = "session_otp.html")]
struct SessionOtpPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    error: String,
    confirm_title: String,
    logging_in_as: String,
    confirm_button: String,
    action: String,
    csrf_token: String,
}

/// session/one_time_password.html.erb in the no_ember layout.
async fn otp_page(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    username: Option<&str>,
) -> Result<Response, AppError> {
    use askama::Template;
    let i18n = &state.i18n;
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut site = crate::html::Site::from_settings(settings, &base_path)?;
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
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    // form_tag's authenticity token, for the session the form posts from.
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(state, headers, settings)?;
    let error = if username.is_some() {
        String::new()
    } else {
        i18n.t("user_api_key.invalid_token")
            .unwrap_or_default()
            .to_string()
    };
    let page = SessionOtpPage {
        confirm_title: i18n
            .t_with(
                "user_api_key.otp_confirmation.confirm_title",
                &[("site_name", &site.site_title)],
            )
            .unwrap_or_default(),
        logging_in_as: i18n
            .t_with(
                "user_api_key.otp_confirmation.logging_in_as",
                &[("username", username.unwrap_or_default())],
            )
            .unwrap_or_default(),
        confirm_button: i18n
            .t("user_api_key.otp_confirmation.confirm_button")
            .unwrap_or_default()
            .to_string(),
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: None,
        bus_position: String::new(),
        chrome: site.chrome,
        error,
        action: uri.path().to_string(),
        csrf_token,
    };
    let html = page.render().map_err(crate::html::HtmlError::from)?;
    let mut response = axum::response::Html(html).into_response();
    if let Some(cookie) = set_cookie {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}
