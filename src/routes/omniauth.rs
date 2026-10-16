//! External logins: OmniAuth's middleware for the enabled authenticators
//! (Middleware::OmniauthBypassMiddleware and config/initializers/
//! 009-omniauth.rb) and Users::OmniauthCallbacksController.
//!
//! /auth/:provider starts a login (POST, or GET where interaction isn't
//! required), checking the CSRF token and keeping where to return; the
//! provider sends the browser back to /auth/:provider/callback, where the
//! strategy checks the state and exchanges the code, the authenticator
//! finds or remembers the account, and the user is logged on, or the
//! browser is told (the authentication_data cookie) to sign up.
//! Failures go to /auth/failure. GET /auth/:provider without GET allowed
//! shows the confirmation page that posts to it.
//!
//! Strategies ported: discourse-oauth2-basic's, discourse-openid-connect's
//! and the OAuth2 ones Discourse ships (auth/builtin.rs). Refused: the
//! others, and invite-only sites' invite check.

use askama::Template;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::search::Peer;
use crate::auth::managed::{AuthHash, AuthResult};
use crate::auth::oauth2::{self, Client, Failure, Http};
use crate::session::cookie::Scalar;
use crate::session::current::Incoming;
use crate::session::forum::{ForumSession, load_forum_session};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// The strategies ported.
fn ported(name: &str) -> bool {
    matches!(name, "oauth2_basic" | "oidc") || crate::auth::builtin::is_builtin(name)
}

fn host(headers: &HeaderMap) -> &str {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost")
}

fn base_path(state: &AppState) -> String {
    state.config.globals.relative_url_root().to_string()
}

fn http<'a>(state: &'a AppState, settings: &'a SiteSettings) -> Http<'a> {
    Http {
        fetcher: crate::final_destination::Fetcher {
            settings,
            urls: crate::url::Urls {
                config: &state.config,
                settings,
            },
        },
        user_agent: crate::schema::user_agent(),
    }
}

/// `Rack::Response#redirect` as OmniAuth's `redirect(uri)` sends it.
fn omniauth_redirect(location: &str, cookies: &[String]) -> Result<Response, AppError> {
    let mut response = (
        StatusCode::FOUND,
        [
            (header::LOCATION, HeaderValue::from_str(location)?),
            (header::CONTENT_TYPE, HeaderValue::from_static("text/html")),
        ],
        format!("Redirecting to {location}..."),
    )
        .into_response();
    for c in cookies {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(c)?);
    }
    Ok(response)
}

fn with_cookies(mut response: Response, cookies: &[String]) -> Result<Response, AppError> {
    for c in cookies {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(c)?);
    }
    Ok(response)
}

/// `OmniAuth::FailureEndpoint#redirect_to_failure`: /auth/failure with
/// the error, the origin and the strategy.
fn failure_redirect(
    state: &AppState,
    kind: &str,
    origin: Option<&str>,
    strategy: &str,
    cookies: &[String],
) -> Result<Response, AppError> {
    let esc = |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    let mut location = format!("{}/auth/failure?message={}", base_path(state), esc(kind));
    if let Some(origin) = origin {
        location.push_str(&format!("&origin={}", esc(origin)));
    }
    location.push_str(&format!("&strategy={strategy}"));
    let mut response = (
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_str(&location)?)],
        "302 Moved",
    )
        .into_response();
    for c in cookies {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(c)?);
    }
    Ok(response)
}

fn session_take(session: &mut ForumSession, key: &str) -> Option<String> {
    let value = session.map.remove(key);
    if value.is_some() {
        session.changed = true;
    }
    match value {
        Some(Scalar::Str(s)) => Some(s),
        _ => None,
    }
}

fn session_set(session: &mut ForumSession, key: &str, value: Scalar) {
    session.map.insert(key.to_string(), value);
    session.changed = true;
}

/// The callback url the strategies send: the site's base url with the
/// callback path (OmniAuthStrategyCompatPatch keeps one base path).
fn callback_url(state: &AppState, settings: &SiteSettings, name: &str) -> Result<String, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    if crate::auth::builtin::is_builtin(name) {
        return crate::auth::builtin::callback_url(
            name,
            &urls.base_url_no_prefix()?,
            state.config.globals.relative_url_root(),
        );
    }
    Ok(format!("{}/auth/{name}/callback", urls.base_url()?))
}

/// GET or POST /auth/:provider
pub async fn request(
    State(state): State<AppState>,
    method: Method,
    Path(provider): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    axum::Extension(incoming): axum::Extension<Incoming>,
    body: Bytes,
) -> Result<Response, AppError> {
    if provider == "failure" {
        return failure(&state, &headers, &uri, &body).await;
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let enabled = crate::auth::enabled(&settings)?;
    // OmniAuth matches the path case-insensitively, a trailing slash aside.
    let name = provider.trim_end_matches('/').to_lowercase();
    let authenticator = enabled.iter().find(|p| p.name == name).copied();
    let only_one = !settings.get("enable_local_logins")?.truthy() && enabled.len() == 1;
    let allow_get = only_one || !settings.get("auth_require_interaction")?.truthy();
    let allowed = method == Method::POST || (method == Method::GET && allow_get);
    let Some(authenticator) = authenticator.filter(|_| allowed) else {
        // Rails' own routes: the confirmation page for GET, none for POST.
        if method == Method::GET {
            return confirm_request(&state, &settings, &headers, &uri, &incoming, &provider).await;
        }
        return Ok(super::topics::not_found_response(&state));
    };
    let name = authenticator.name;
    if !ported(name) {
        return Err(Unsupported("logging in with this provider (191e)").into());
    }
    let params = crate::params::parse(uri.query(), &headers, &body);
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut cookies: Vec<String> = Vec::new();
    // before_request_phase: the CSRF token for a POST.
    if method != Method::GET {
        let form: Vec<(String, String)> = params
            .iter()
            .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
            .collect();
        if !super::session::csrf_ok(&state, &headers, &form, uri.path(), "POST") {
            cookies.extend(session.set_cookie(&state, &settings)?);
            return failure_redirect(&state, "csrf_detected", None, name, &cookies);
        }
    }
    let reconnect = params.contains_key("reconnect");
    session_set(&mut session, "auth_reconnect", Scalar::Bool(reconnect));
    let origin = crate::params::string(&params, "origin").filter(|o| !o.is_empty());
    if let Some(origin) = &origin {
        crate::session::server::set(
            &mut conn,
            &session_id,
            "destination_url",
            &json!(origin),
            crate::session::server::EXPIRY_SECONDS,
        )
        .await?;
    }
    // OmniAuth's origin: the param, else the referer.
    let request_path = format!("{}/auth/{name}", base_path(&state));
    if let Some(origin) = &origin {
        session_set(&mut session, "omniauth.origin", Scalar::Str(origin.clone()));
    } else if let Some(referer) = headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .filter(|r| !r.ends_with(&request_path))
    {
        session_set(
            &mut session,
            "omniauth.origin",
            Scalar::Str(referer.to_string()),
        );
    }
    let http = http(&state, &settings);
    let callback = callback_url(&state, &settings, name)?;
    let state_param = oauth2::random_hex(24);
    let (client, mut extra, pkce) = match name {
        "oauth2_basic" => {
            let signup = params
                .get("signup")
                .and_then(crate::params::scalar)
                .is_some_and(|v| !v.trim().is_empty());
            (
                crate::plugins::oauth2_basic::client(&settings, signup)?,
                crate::plugins::oauth2_basic::authorize_params(&settings)?,
                false,
            )
        }
        "oidc" => {
            let doc =
                crate::plugins::openid_connect::discovery_document(&mut conn, &settings, &http)
                    .await?;
            let discovered =
                match crate::plugins::openid_connect::discover(doc.as_ref(), &settings)? {
                    Ok(d) => d,
                    Err(f) => {
                        cookies.extend(session.set_cookie(&state, &settings)?);
                        return failure_redirect(&state, &f.kind, None, name, &cookies);
                    }
                };
            let authorize = crate::plugins::openid_connect::authorize_params(&settings, &params)?;
            session_set(&mut session, "omniauth.nonce", Scalar::Str(authorize.nonce));
            if let Some(p) = authorize.param_p {
                session_set(&mut session, "omniauth.param.p", Scalar::Str(p));
            }
            (
                crate::plugins::openid_connect::client(&settings, &discovered)?,
                authorize.params,
                crate::plugins::openid_connect::pkce(&settings)?,
            )
        }
        n => (
            crate::auth::builtin::client(&settings, n)?,
            crate::auth::builtin::authorize_params(&settings, n, &params)?,
            false,
        ),
    };
    // Google takes a state from the request; it is the one kept.
    let state_param = match extra.iter().find(|(k, _)| k == "state") {
        Some((_, given)) => given.clone(),
        None => {
            extra.insert(0, ("state".to_string(), state_param.clone()));
            state_param
        }
    };
    if pkce {
        let verifier = oauth2::random_hex(64);
        extra.push(("code_challenge".into(), oauth2::code_challenge(&verifier)));
        extra.push(("code_challenge_method".into(), "S256".into()));
        session_set(
            &mut session,
            "omniauth.pkce.verifier",
            Scalar::Str(verifier),
        );
    }
    session_set(&mut session, "omniauth.state", Scalar::Str(state_param));
    let url = oauth2::authorize_url(&client, &callback, &extra)?;
    cookies.extend(session.set_cookie(&state, &settings)?);
    omniauth_redirect(&url, &cookies)
}

#[derive(Template)]
#[template(path = "omniauth_confirm.html")]
struct ConfirmPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    title: String,
    subtitle: String,
    provider_name: String,
    provider_description: String,
    button: String,
    footer: String,
    csrf_token: String,
    plug_icon: String,
}

/// Users::OmniauthCallbacksController#confirm_request: the page whose
/// button posts to /auth/:provider.
async fn confirm_request(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
    incoming: &Incoming,
    provider: &str,
) -> Result<Response, AppError> {
    // find_authenticator
    let enabled = crate::auth::enabled(settings)?;
    if settings.get("enable_discourse_connect")?.truthy()
        || !enabled.iter().any(|p| p.name == provider)
    {
        return Ok(super::search::invalid_access_with(
            state,
            "authenticator_not_found",
        ));
    }
    let i18n = &state.i18n;
    let provider_name = i18n
        .t(&format!("js.login.{provider}.name"))
        .map(str::to_string)
        .unwrap_or_else(|| titleize(provider));
    let base = base_path(state);
    let mut site = crate::html::Site::from_settings(settings, &base)?;
    let mut conn = state.pool.acquire().await?;
    site.load_chrome(
        &mut conn,
        state,
        settings,
        &incoming.guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    drop(conn);
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(state, headers, settings)?;
    let title = site.site_title.clone();
    let page = ConfirmPage {
        crawler: crate::html::Crawler::for_request(&urls, uri, None)?,
        title: i18n
            .t_with(
                "login.omniauth_confirm_title",
                &[("provider", &provider_name)],
            )
            .unwrap_or_default(),
        subtitle: i18n
            .t_with(
                "login.omniauth_confirm_description",
                &[("provider", &provider_name), ("site", &title)],
            )
            .unwrap_or_default(),
        provider_description: i18n
            .t("login.omniauth_confirm_provider_description")
            .unwrap_or_default()
            .to_string(),
        button: i18n
            .t("login.omniauth_confirm_button")
            .unwrap_or_default()
            .to_string(),
        footer: i18n
            .t_with(
                "login.omniauth_confirm_footer_html",
                &[(
                    "site",
                    &format!("<strong>{}</strong>", html_escape::encode_text(&title)),
                )],
            )
            .unwrap_or_default(),
        provider_name,
        csrf_token,
        plug_icon: crate::svg_sprite::raw_svg("plug")?,
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        viewer: None,
        bus_position: String::new(),
        chrome: site.chrome,
    };
    let html = page.render().map_err(crate::html::HtmlError::from)?;
    let mut response = Html(html).into_response();
    if let Some(cookie) = set_cookie {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// ActiveSupport's `titleize` for a provider name.
fn titleize(s: &str) -> String {
    s.split(['_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Template)]
#[template(path = "omniauth_failure.html")]
struct FailurePage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    /// The flash error, HTML.
    error: String,
}

/// users/omniauth_callbacks/failure.html.erb in the no_ember layout.
async fn failure_page(
    state: &AppState,
    settings: &SiteSettings,
    uri: &Uri,
    error_html: String,
) -> Result<Response, AppError> {
    let base = base_path(state);
    let mut site = crate::html::Site::from_settings(settings, &base)?;
    let mut conn = state.pool.acquire().await?;
    site.load_chrome(
        &mut conn,
        state,
        settings,
        &crate::guardian::Guardian::anonymous(),
        crate::sidebar::Active::None,
    )
    .await?;
    drop(conn);
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let page = FailurePage {
        crawler: crate::html::Crawler::for_request(&urls, uri, None)?,
        error: error_html,
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        viewer: None,
        bus_position: String::new(),
        chrome: site.chrome,
    };
    let html = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(Html(html).into_response())
}

/// GET or POST /auth/failure (Users::OmniauthCallbacksController#failure)
async fn failure(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let params = crate::params::parse(uri.query(), headers, body);
    let error_name: String = crate::params::string(&params, "message")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let i18n = &state.i18n;
    let message = match error_name.as_str() {
        "csrf_detected" | "request_error" | "invalid_iat" | "unauthorized" => i18n
            .t(&format!("login.omniauth_error.{error_name}"))
            .unwrap_or_default()
            .to_string(),
        _ => {
            let provider_name = crate::params::string(&params, "provider")
                .filter(|p| !p.is_empty())
                .or_else(|| crate::params::string(&params, "strategy").filter(|p| !p.is_empty()));
            let enabled = crate::auth::enabled(&settings)?;
            let mut provider = provider_name.and_then(|n| {
                enabled
                    .iter()
                    .find(|p| p.name == n)
                    .map(|p| p.display_name())
            });
            if provider.is_none() && enabled.len() == 1 {
                provider = Some(enabled[0].display_name());
            }
            match provider {
                Some(p) => i18n
                    .t_with(
                        "login.omniauth_error.generic_with_provider",
                        &[("provider", p)],
                    )
                    .unwrap_or_default(),
                None => i18n
                    .t("login.omniauth_error.generic_without_provider")
                    .unwrap_or_default()
                    .to_string(),
            }
        }
    };
    failure_page(
        state,
        &settings,
        uri,
        html_escape::encode_text(&message).into_owned(),
    )
    .await
}

/// What a strategy's callback phase found.
enum Callback {
    Auth(Box<AuthHash>),
    Failed(Failure),
    /// openid-connect's error handler: a redirect.
    Redirect(String),
}

/// The strategy's `callback_phase`: the state checked, the provider's
/// error, the code exchanged, the auth hash built.
async fn callback_phase(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    session: &mut ForumSession,
    name: &str,
    params: &Map<String, Value>,
    headers: &HeaderMap,
) -> Result<Callback, AppError> {
    let http = http(state, settings);
    let param = |k: &str| crate::params::string(params, k);
    let mut oidc: Option<crate::plugins::openid_connect::Discovered> = None;
    let (client, ignores_state): (Client, bool) = match name {
        "oauth2_basic" => (
            crate::plugins::oauth2_basic::client(settings, false)?,
            crate::plugins::oauth2_basic::ignores_state(settings)?,
        ),
        "oidc" => {
            if let (Some(_), Some(description)) = (param("error"), param("error_description"))
                && let Some(url) =
                    crate::plugins::openid_connect::error_redirect(settings, &description)?
            {
                return Ok(Callback::Redirect(url));
            }
            let doc =
                crate::plugins::openid_connect::discovery_document(conn, settings, &http).await?;
            let d = match crate::plugins::openid_connect::discover(doc.as_ref(), settings)? {
                Ok(d) => d,
                Err(f) => return Ok(Callback::Failed(f)),
            };
            let client = crate::plugins::openid_connect::client(settings, &d)?;
            oidc = Some(d);
            (client, false)
        }
        n => {
            let client = crate::auth::builtin::client(settings, n)?;
            if let Some(f) = crate::auth::builtin::before_callback(n, params, &client, headers)? {
                return Ok(Callback::Failed(f));
            }
            (client, false)
        }
    };
    // omniauth-oauth2's callback_phase.
    let error = param("error_reason").or_else(|| param("error"));
    if !ignores_state {
        let given = param("state").unwrap_or_default();
        let stored = session_take(session, "omniauth.state");
        let Some(stored) = stored else {
            if given.is_empty() {
                return Ok(Callback::Failed(Failure::new(
                    "csrf_detected",
                    "CSRF detected",
                )));
            }
            return Err(
                Unsupported("an OAuth callback without a session state (NoMethodError)").into(),
            );
        };
        if given.is_empty()
            || !bool::from(subtle::ConstantTimeEq::ct_eq(
                given.as_bytes(),
                stored.as_bytes(),
            ))
        {
            return Ok(Callback::Failed(Failure::new(
                "csrf_detected",
                "CSRF detected",
            )));
        }
    }
    if let Some(error) = error {
        // CallbackError's message: error, the description (or reason) and
        // the uri. When it is the error alone, Discourse's on_failure takes
        // the failure for a rescued exception and re-raises it, which the
        // callback phase rescues as invalid_credentials (and the second
        // time, a 500).
        let message = [
            param("error"),
            param("error_description").or_else(|| param("error_reason")),
            param("error_uri"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" | ");
        if message == error {
            if error == "invalid_credentials" {
                return Err(Unsupported("an OAuth error re-raised by on_failure (500)").into());
            }
            return Ok(Callback::Failed(Failure::new(
                "invalid_credentials",
                message,
            )));
        }
        return Ok(Callback::Failed(Failure::new(&error, message)));
    }
    let code = param("code").unwrap_or_default();
    let callback = callback_url(state, settings, name)?;
    // token_params: PKCE's verifier, openid-connect's `p`.
    let mut extra: Vec<(String, String)> = Vec::new();
    if let Some(p) = session_take(session, "omniauth.param.p").filter(|p| !p.is_empty()) {
        extra.push(("p".into(), p));
    }
    if name == "oidc" && crate::plugins::openid_connect::pkce(settings)? {
        let verifier = session_take(session, "omniauth.pkce.verifier");
        extra.push(("code_verifier".into(), verifier.unwrap_or_default()));
    }
    let token = match &oidc {
        _ if crate::auth::builtin::is_builtin(name) => {
            let xhr = headers
                .get("x-requested-with")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v == "XMLHttpRequest");
            let json_body = headers
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.to_lowercase().contains("json"));
            crate::auth::builtin::build_access_token(
                &http, settings, name, &client, params, xhr, json_body, &callback,
            )
            .await?
        }
        Some(d) if d.userinfo_endpoint.is_none() => {
            crate::plugins::openid_connect::token_without_userinfo(
                &http, &client, &code, &callback, &extra,
            )
            .await?
        }
        _ => http.get_token(&client, &code, &callback, &extra).await?,
    };
    let token = match token {
        Ok(t) => t,
        Err(f) => {
            // openid-connect re-raises Faraday's errors as request failures.
            if name == "oidc" && f.kind == "timeout" {
                return Ok(Callback::Failed(Failure::new(
                    "openid_connect_request_failed",
                    f.message,
                )));
            }
            if name == "oauth2_basic" && f.kind == "timeout" {
                return Ok(Callback::Failed(Failure::new(
                    "oauth2_basic_request_failed",
                    f.message,
                )));
            }
            return Ok(Callback::Failed(f));
        }
    };
    if token
        .expires_at
        .is_some_and(|at| at <= crate::clock::now().timestamp())
    {
        return Err(Unsupported("refreshing an expired OAuth token").into());
    }
    let auth = match name {
        "oauth2_basic" => crate::plugins::oauth2_basic::auth_hash(settings, &token)?,
        "oidc" => {
            let nonce = session_take(session, "omniauth.nonce");
            let d = oidc.as_ref().expect("discovered");
            match crate::plugins::openid_connect::auth_hash(
                &http,
                &client,
                d,
                &token,
                nonce.as_deref(),
            )
            .await?
            {
                Ok(a) => a,
                Err(f) => return Ok(Callback::Failed(f)),
            }
        }
        n => match crate::auth::builtin::auth_hash(&http, n, &client, &token).await? {
            Ok(a) => a,
            Err(f) => return Ok(Callback::Failed(f)),
        },
    };
    Ok(Callback::Auth(Box::new(auth)))
}

/// The authenticator's `after_authenticate(auth, existing_account:)`.
pub(super) async fn after_authenticate(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    auth: AuthHash,
    existing_account: Option<i32>,
) -> Result<(AuthResult, AuthHash), AppError> {
    match auth.provider.as_str() {
        "oauth2_basic" => {
            let http = http(state, settings);
            crate::plugins::oauth2_basic::after_authenticate(
                conn,
                settings,
                &state.i18n,
                &http,
                auth,
                existing_account,
            )
            .await
        }
        "oidc" => {
            let result = crate::plugins::openid_connect::after_authenticate(
                conn,
                settings,
                &auth,
                existing_account,
            )
            .await?;
            Ok((result, auth))
        }
        n if crate::auth::builtin::is_builtin(n) => {
            let http = http(state, settings);
            crate::auth::builtin::after_authenticate(
                conn,
                settings,
                &state.i18n,
                &http,
                auth,
                existing_account,
            )
            .await
        }
        _ => Err(Unsupported("logging in with this provider").into()),
    }
}

/// `persist_auth_token(auth)`: the auth kept ten minutes for
/// /associate/:token, the path returned.
async fn persist_auth_token(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    session_id: &str,
    auth: &AuthHash,
) -> Result<String, AppError> {
    let secret = oauth2::random_hex(16);
    crate::session::server::set(
        conn,
        session_id,
        &format!("omniauth_reconnect_{secret}"),
        &json!(auth.to_json().to_string()),
        600,
    )
    .await?;
    Ok(format!("{}/associate/{secret}", base_path(state)))
}

/// `valid_origin?(uri)`: on this host, not the auth or login paths.
fn valid_origin(
    state: &AppState,
    settings: &SiteSettings,
    origin: &str,
) -> Result<Option<String>, AppError> {
    let base = base_path(state);
    let parsed = match crate::wildcard_url::parse(origin) {
        Some(u) => u,
        None => return Ok(None),
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    if let Some(h) = parsed.host.as_deref().filter(|h| !h.is_empty())
        && h != urls.current_hostname()?
    {
        return Ok(None);
    }
    if parsed.path.starts_with(&format!("{base}/auth/"))
        || parsed.path.starts_with(&format!("{base}/login"))
    {
        return Ok(None);
    }
    let mut out = parsed.path.clone();
    if let Some(q) = &parsed.query {
        out.push('?');
        out.push_str(q);
    }
    Ok(Some(out))
}

/// GET or POST /auth/:provider/callback
#[allow(clippy::too_many_arguments)]
pub async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    axum::Extension(incoming): axum::Extension<Incoming>,
    mode: Option<axum::Extension<crate::readonly::Mode>>,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let enabled = crate::auth::enabled(&settings)?;
    let name = provider.to_lowercase();
    let Some(authenticator) = enabled.iter().find(|p| p.name == name).copied() else {
        // Not OmniAuth's: users/omniauth_callbacks#complete with no auth.
        return Ok(super::topics::not_found_response(&state));
    };
    let name = authenticator.name;
    if !ported(name) {
        return Err(Unsupported("logging in with this provider (191e)").into());
    }
    let params = crate::params::parse(uri.query(), &headers, &body);
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    // callback_call: the origin and params taken out of the session.
    let origin = session_take(&mut session, "omniauth.origin").filter(|o| !o.is_empty());
    let found = callback_phase(
        &state,
        &mut tx,
        &settings,
        &mut session,
        name,
        &params,
        &headers,
    )
    .await?;
    let auth = match found {
        Callback::Auth(auth) => *auth,
        Callback::Redirect(url) => {
            tx.commit().await?;
            let cookies: Vec<String> = session.set_cookie(&state, &settings)?.into_iter().collect();
            return omniauth_redirect(&url, &cookies);
        }
        Callback::Failed(f) => {
            tracing::error!("(oauth) Authentication failure! {}: {}", f.kind, f.message);
            tx.commit().await?;
            let cookies: Vec<String> = session.set_cookie(&state, &settings)?.into_iter().collect();
            return failure_redirect(&state, &f.kind, origin.as_deref(), name, &cookies);
        }
    };
    let mode = mode.map(|m| m.0);
    if mode.is_some_and(|m| m.readonly && !m.staff_writes_only) {
        return Err(crate::readonly::ReadOnly.into());
    }
    // find_authenticator
    if settings.get("enable_discourse_connect")?.truthy() {
        return Ok(super::search::invalid_access_with(
            &state,
            "authenticator_not_found",
        ));
    }
    let can_connect = (authenticator.can_connect)(&settings)?;
    let current_user = incoming.guardian.user_id();
    let mut cookies: Vec<String> = Vec::new();
    let reconnect = matches!(
        session.map.remove("auth_reconnect"),
        Some(Scalar::Bool(true))
    );
    session.changed = true;
    if reconnect && can_connect && current_user.is_some() {
        let path = persist_auth_token(&state, &mut tx, &session_id, &auth).await?;
        tx.commit().await?;
        cookies.extend(session.set_cookie(&state, &settings)?);
        return with_cookies(
            (
                StatusCode::FOUND,
                [(header::LOCATION, format!("http://{}{path}", host(&headers)))],
            )
                .into_response(),
            &cookies,
        );
    }
    let auth_json = auth.clone();
    let (mut result, auth) = after_authenticate(&state, &mut tx, &settings, auth, None).await?;
    let _ = &auth;
    // Staged users are treated as unregistered.
    if let Some(user_id) = result.user_id {
        let staged: bool = sqlx::query_scalar("SELECT staged FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        if staged {
            result.user_id = None;
        }
    }
    // preferred_origin: the server session's destination, the provider
    // request's payload, the destination_url cookie, else OmniAuth's.
    let mut preferred = origin.clone();
    let destination: Option<String> =
        crate::session::server::get(&mut tx, &session_id, "destination_url")
            .await?
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|v| !v.trim().is_empty());
    let base = base_path(&state);
    if let Some(d) = destination {
        preferred = Some(d);
    } else if settings.get("enable_discourse_connect_provider")?.truthy()
        && let Some(payload) = crate::session::current::cookie(&headers, "sso_payload")
    {
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        let payload = form_urlencoded::parse(format!("v={payload}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default();
        preferred = Some(format!(
            "{}/session/sso_provider?{payload}",
            urls.base_url()?
        ));
        cookies.push(format!(
            "sso_payload=; path={}; max-age=0; expires=Thu, 01 Jan 1970 00:00:00 GMT",
            if base.is_empty() { "/" } else { &base }
        ));
    } else if let Some(cookie) =
        crate::session::current::cookie(&headers, "destination_url").filter(|c| !c.is_empty())
    {
        preferred = form_urlencoded::parse(format!("v={cookie}").as_bytes())
            .next()
            .map(|(_, v)| v.into_owned());
    }
    crate::session::server::delete(&mut tx, &session_id, "destination_url").await?;
    cookies.extend(super::discourse_connect::delete_cookie(
        &headers,
        "destination_url",
        &base,
    ));
    let mut origin_path = match &preferred {
        Some(p) => valid_origin(&state, &settings, p)?,
        None => None,
    };
    if origin_path.as_deref().is_none_or(|o| o.trim().is_empty()) {
        origin_path = Some(format!("{base}/"));
    }
    let origin_path = origin_path.expect("an origin");
    result.destination_url = Some(origin_path.clone());
    result.authenticator_name = Some(name.to_string());
    if result.failed {
        tx.commit().await?;
        cookies.extend(session.set_cookie(&state, &settings)?);
        let reason = discourse_markdown::sanitizer::sanitize(
            result.failed_reason.as_deref().unwrap_or(""),
            &discourse_markdown::sanitizer::AllowList::new(),
        );
        let page = failure_page(&state, &settings, &uri, reason).await?;
        return with_cookies(page, &cookies);
    }
    if mode.is_some_and(|m| m.staff_writes_only) {
        let staff = match result.user_id {
            Some(id) => {
                sqlx::query_scalar::<_, bool>("SELECT admin OR moderator FROM users WHERE id = $1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?
            }
            None => false,
        };
        if !staff {
            return Err(crate::readonly::ReadOnly.into());
        }
    }
    // complete_response_data
    let mut logged_on: Option<String> = None;
    if let Some(user_id) = result.user_id {
        logged_on = user_found(
            &state,
            &mut tx,
            &settings,
            &headers,
            peer,
            &uri,
            &session_id,
            &mut result,
            user_id,
        )
        .await?;
    } else if settings.get("invite_only")?.truthy() {
        return Err(Unsupported("external logins on invite-only sites (192)").into());
    } else {
        crate::session::server::set(
            &mut tx,
            &session_id,
            "authentication",
            &result.session_data(),
            crate::session::server::EXPIRY_SECONDS,
        )
        .await?;
    }
    if result.failed {
        tx.commit().await?;
        cookies.extend(session.set_cookie(&state, &settings)?);
        let reason = discourse_markdown::sanitizer::sanitize(
            result.failed_reason.as_deref().unwrap_or(""),
            &discourse_markdown::sanitizer::AllowList::new(),
        );
        let page = failure_page(&state, &settings, &uri, reason).await?;
        return with_cookies(page, &cookies);
    }
    let mut client_hash = result
        .to_client_hash(&mut tx, &settings, &state.i18n)
        .await?;
    if can_connect && (settings.get("enable_local_logins")?.truthy() || enabled.len() > 1) {
        let path = persist_auth_token(&state, &mut tx, &session_id, &auth_json).await?;
        client_hash.insert("associate_url".into(), json!(path));
    }
    tx.commit().await?;
    let cookie_path = if base.is_empty() {
        "/".to_string()
    } else {
        format!("{base}/")
    };
    cookies.push(format!(
        "_bypass_cache=true; path={}",
        if base.is_empty() { "/" } else { &base }
    ));
    cookies.push(format!(
        "authentication_data={}; path={cookie_path}",
        form_urlencoded::byte_serialize(Value::Object(client_hash).to_string().as_bytes())
            .collect::<String>()
    ));
    if let Some(cookie) = logged_on {
        cookies.push(cookie);
    }
    cookies.extend(session.set_cookie(&state, &settings)?);
    let logged_in = current_user.is_some() || result.authenticated;
    let location = if !logged_in && origin_path.starts_with(&format!("{base}/user-api-key/new")) {
        format!("{base}/")
    } else {
        origin_path
    };
    with_cookies(
        (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                HeaderValue::from_str(&format!("http://{}{location}", host(&headers)))?,
            )],
        )
            .into_response(),
        &cookies,
    )
}

/// `user_found(user)`: the second factor rule, else
/// handle_account_activation; the auth cookie when logged on.
#[allow(clippy::too_many_arguments)]
async fn user_found(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    headers: &HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: &Uri,
    session_id: &str,
    result: &mut AuthResult,
    user_id: i32,
) -> Result<Option<String>, AppError> {
    let enabled = crate::second_factor::Enabled::load(&mut *conn, settings, user_id).await?;
    if enabled.any()
        && settings
            .get("enforce_second_factor_on_external_auth")?
            .truthy()
    {
        result.omniauth_disallow_totp = true;
        let email: Option<String> =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
        result.set_email(email);
        return Ok(None);
    }
    // handle_account_activation
    #[derive(sqlx::FromRow)]
    struct U {
        active: bool,
        approved: bool,
        admin: bool,
        moderator: bool,
        email: Option<String>,
        registration_ip_address: Option<String>,
    }
    let user: U = sqlx::query_as(
        "SELECT active, approved, admin, moderator, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") AS email, \
                host(registration_ip_address) AS registration_ip_address \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let ip = crate::remote_ip::remote_ip(headers, peer)?;
    let mut active = user.active;
    if result.email_valid && result.email() == user.email {
        let confirmed = crate::group_manager::email_confirmed(conn, user_id).await?;
        if !user.active || !confirmed {
            // user.update!(password: SecureRandom.hex), then activate.
            crate::accounts::set_password(&mut *conn, user_id, &oauth2::random_hex(16)).await?;
            let email = user.email.clone().unwrap_or_default();
            let has_token: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM email_tokens WHERE email = $1 AND confirmed) \
                 OR EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $2 AND email = $1 AND NOT expired)",
            )
            .bind(&email)
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
            if !has_token {
                crate::accounts::create_email_token(
                    &mut *conn,
                    user_id,
                    &email,
                    crate::accounts::token_scopes::SIGNUP,
                )
                .await?;
            }
            // user.activate
            let token = crate::accounts::create_email_token(
                &mut *conn,
                user_id,
                &email,
                crate::accounts::token_scopes::SIGNUP,
            )
            .await?;
            super::accounts::confirm_email_token(
                state,
                conn,
                settings,
                &token,
                crate::accounts::token_scopes::SIGNUP,
            )
            .await?;
            active = true;
        }
        if user.registration_ip_address.is_none() {
            sqlx::query(
                "UPDATE users SET registration_ip_address = $2::inet, updated_at = clock_timestamp() WHERE id = $1",
            )
                .bind(user_id)
                .bind(&ip)
                .execute(&mut *conn)
                .await?;
        }
    }
    let can_access = !settings.get("must_approve_users")?.truthy()
        || user.approved
        || user.admin
        || user.moderator;
    if crate::accounts::ip_blocked(&mut *conn, &ip).await? {
        result.not_allowed_from_ip_address = true;
        return Ok(None);
    }
    if user.admin && settings.get("use_admin_ip_allowlist")?.truthy() {
        return Err(Unsupported("the admin IP allowlist at an external login").into());
    }
    if can_access && active {
        let cx = crate::discourse_connect::Ctx {
            settings,
            i18n: &state.i18n,
            state,
        };
        crate::auth::managed::apply_user_attributes(conn, &cx, result, user_id).await?;
        let name = result.authenticator_name.as_deref().unwrap_or("");
        crate::auth::managed::apply_associated_attributes(
            conn,
            settings,
            result,
            user_id,
            provides_groups(settings, name)?,
        )
        .await?;
        let cookie = crate::session::login::log_on_user(
            state,
            settings,
            conn,
            headers,
            peer,
            uri.path(),
            user_id,
            true,
        )
        .await?;
        // Invite.invalidate_for_email
        if let Some(email) = &user.email {
            sqlx::query(
                "UPDATE invites SET invalidated_at = now() WHERE lower(email) = lower($1) AND invalidated_at IS NULL",
            )
            .bind(email)
            .execute(&mut *conn)
            .await?;
        }
        crate::session::server::delete(&mut *conn, session_id, "authentication").await?;
        result.authenticated = true;
        return Ok(Some(cookie));
    }
    if settings.get("must_approve_users")?.truthy() && !user.approved {
        result.awaiting_approval = true;
    } else {
        result.awaiting_activation = true;
    }
    Ok(None)
}

/// `authenticator.provides_groups?`
pub(super) fn provides_groups(settings: &SiteSettings, name: &str) -> Result<bool, AppError> {
    Ok(match name {
        "oauth2_basic" => crate::plugins::oauth2_basic::provides_groups(settings)?,
        "oidc" => !settings
            .get("openid_connect_groups_claim")?
            .to_s()
            .trim()
            .is_empty(),
        "google_oauth2" => crate::auth::builtin::google_provides_groups(settings)?,
        _ => false,
    })
}

/// Users::AssociateAccountsController's auth hash: the one the callback
/// kept for this token, with its authenticator (enabled, and one users
/// may connect).
async fn associate_auth(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    session_id: &str,
    token: &str,
) -> Result<Result<(AuthHash, &'static crate::auth::Provider), Response>, AppError> {
    let stored =
        crate::session::server::get(conn, session_id, &format!("omniauth_reconnect_{token}"))
            .await?;
    let Some(json) = stored.and_then(|v| v.as_str().map(str::to_string)) else {
        return Ok(Err(super::topics::not_found_response(state)));
    };
    let value: Value =
        serde_json::from_str(&json).map_err(|_| Unsupported("an unreadable kept auth hash"))?;
    let auth = AuthHash::from_json(&value);
    let Some(authenticator) = crate::auth::enabled(settings)?
        .into_iter()
        .find(|p| p.name == auth.provider)
    else {
        return Ok(Err(super::search::invalid_access_with(
            state,
            "authenticator_not_found",
        )));
    };
    if !(authenticator.can_connect)(settings)? {
        return Ok(Err(super::search::invalid_access_with(
            state,
            "authenticator_no_connect",
        )));
    }
    Ok(Ok((auth, authenticator)))
}

/// The user's account with a provider described, "" without one
/// (`description_for_user`).
async fn description_for_user(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    provider: &str,
    user_id: i32,
) -> Result<String, AppError> {
    let info: Option<Value> = sqlx::query_scalar(
        "SELECT info FROM user_associated_accounts WHERE provider_name = $1 AND user_id = $2 LIMIT 1",
    )
    .bind(provider)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match info {
        None => String::new(),
        Some(info) => crate::auth::description(&state.i18n, &info),
    })
}

/// The token's segment: 32 hex digits, `.json` or not.
fn associate_token(raw: &str) -> Option<(String, bool)> {
    let (token, json) = match raw.strip_suffix(".json") {
        Some(t) => (t, true),
        None => (raw, false),
    };
    (token.len() == 32 && token.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| (token.to_string(), json))
}

/// GET /associate/:token (Users::AssociateAccountsController#connect_info)
pub async fn connect_info(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
    axum::Extension(incoming): axum::Extension<Incoming>,
) -> Result<Response, AppError> {
    let Some((token, json_path)) = associate_token(&raw) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let json = json_path || super::session::is_xhr_or_json(&headers, &raw);
    if !json {
        // check_xhr: the page the client draws the confirmation on.
        return Err(Unsupported("the associate account page (Ember)").into());
    }
    let Some(user_id) = incoming.guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let (auth, _) = match associate_auth(&state, &mut conn, &settings, &session_id, &token).await? {
        Ok(found) => found,
        Err(response) => return Ok(response),
    };
    let account_description = {
        let info = Value::Object(auth.info.clone());
        let d = crate::auth::description(&state.i18n, &info);
        // description_for_auth_hash: nil for no info; the email, nickname
        // or name.
        let has = ["email", "nickname", "name"]
            .iter()
            .any(|k| crate::auth::managed::truthy(auth.info.get(*k)));
        if has { json!(d) } else { Value::Null }
    };
    let existing = description_for_user(&state, &mut conn, &auth.provider, user_id).await?;
    let mut response = axum::Json(json!({
        "token": token,
        "provider_name": auth.provider,
        "account_description": account_description,
        "existing_account_description": if existing.is_empty() { Value::Null } else { json!(existing) },
    }))
    .into_response();
    if let Some(cookie) = session.set_cookie(&state, &settings)? {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// POST /associate/:token (Users::AssociateAccountsController#connect):
/// the kept account connected to the current user, replacing theirs.
pub async fn connect(
    State(state): State<AppState>,
    Path(raw): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    axum::Extension(incoming): axum::Extension<Incoming>,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some((token, _)) = associate_token(&raw) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let params = crate::params::parse(uri.query(), &headers, &body);
    let form: Vec<(String, String)> = params
        .iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect();
    if !super::session::csrf_ok(&state, &headers, &form, uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    let Some(user_id) = incoming.guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let (auth, authenticator) =
        match associate_auth(&state, &mut tx, &settings, &session_id, &token).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
    if !description_for_user(&state, &mut tx, &auth.provider, user_id)
        .await?
        .is_empty()
        && (authenticator.can_revoke)(&settings)?
    {
        // authenticator.revoke(current_user)
        sqlx::query(
            "DELETE FROM user_associated_accounts WHERE provider_name = $1 AND user_id = $2",
        )
        .bind(&auth.provider)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    after_authenticate(&state, &mut tx, &settings, auth, Some(user_id)).await?;
    crate::session::server::delete(&mut tx, &session_id, &format!("omniauth_reconnect_{token}"))
        .await?;
    tx.commit().await?;
    let mut response = axum::Json(json!({"success": "OK"})).into_response();
    if let Some(cookie) = session.set_cookie(&state, &settings)? {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// POST /u/:username/preferences/revoke-account (users#revoke_account):
/// the user's account with a provider (enabled or not) removed, unless it
/// is their only way to log in.
pub(super) async fn revoke_account(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    username: &str,
    params: &Map<String, Value>,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let Some(target) = crate::user_updater::find_target(&mut tx, guardian, username).await? else {
        return Ok(super::topics::not_found_response(state));
    };
    if !crate::user_updater::can_edit_user(guardian, &target) {
        return Ok(super::search::invalid_access(state));
    }
    let Some(provider_name) =
        crate::params::string(params, "provider_name").filter(|p| !p.is_empty())
    else {
        return Ok(super::accounts::param_missing("provider_name"));
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let Some(authenticator) = crate::auth::find(&provider_name) else {
        return Ok(super::topics::not_found_response(state));
    };
    if !(authenticator.can_revoke)(&settings)? {
        return Ok(super::topics::not_found_response(state));
    }
    let accounts = crate::auth::associated_accounts(&mut tx, &state.i18n, target.id).await?;
    if accounts.as_array().is_some_and(|a| a.len() == 1) {
        let (has_password, passkeys): (bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1), \
                    EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND factor_type = 1)",
        )
        .bind(target.id)
        .fetch_one(&mut *tx)
        .await?;
        if !has_password && !passkeys {
            return Ok(axum::Json(json!({
                "success": false,
                "message": state.i18n.t("user.cannot_remove_all_auth").unwrap_or_default(),
            }))
            .into_response());
        }
    }
    if provider_name == "discourse_id" {
        return Err(Unsupported("revoking a Discourse ID account (the remote revoke)").into());
    }
    // ManagedAuthenticator#revoke
    let removed = sqlx::query(
        "DELETE FROM user_associated_accounts WHERE provider_name = $1 AND user_id = $2",
    )
    .bind(&provider_name)
    .bind(target.id)
    .execute(&mut *tx)
    .await?;
    if removed.rows_affected() == 0 {
        return Ok(super::topics::not_found_response(state));
    }
    tx.commit().await?;
    Ok(axum::Json(json!({"success": "OK"})).into_response())
}
