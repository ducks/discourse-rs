//! `ApplicationController#redirect_to_login_if_required` for anonymous
//! requests: with `login_required` on, JSON gets a 403 `not_logged_in`,
//! HTML is sent to `/login` (the front page renders the login page itself),
//! and only `/srv/status` and `/site/basic-info` stay open.

use askama::Template;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::{Json, RequestExt};
use serde_json::json;

use crate::html::Chrome;
use crate::html::Crawler;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// Paths served without a session even when login is required: the
/// controllers that `skip_before_action :redirect_to_login_if_required`,
/// plus the static files nginx serves in front of Rails.
fn exempt(path: &str) -> bool {
    matches!(
        path,
        "/srv/status"
            | "/site/basic-info"
            | "/site/basic-info.json"
            | "/robots.txt"
            | "/robots-builder.json"
            // StaticController skips the gate; /login renders below.
            | "/signup"
    ) || path.starts_with("/assets/")
        || path.starts_with("/fonts/")
        || path.starts_with("/highlight-js/")
        || path.starts_with("/letter_avatar_proxy/")
        // UserAvatarsController#show and #show_letter
        || path.starts_with("/letter_avatar/")
        || (path.starts_with("/user_avatar/") && !path.contains("/refresh_gravatar"))
        || path.starts_with("/images/")
        || path.starts_with("/uploads/")
        // EmailController
        || path.starts_with("/email/unsubscribe")
        // OmniAuth's middleware and Users::OmniauthCallbacksController
        || path.starts_with("/auth/")
        // UserApiKeysController#new, #otp, #activate and the device
        // request and poll (create_otp needs a login of its own).
        || matches!(
            path,
            "/user-api-key/new"
                | "/user-api-key/new.json"
                | "/user-api-key/otp"
                | "/user-api-key/otp.json"
                | "/user-api-key/activate"
                | "/user-api-key/activate.json"
                | "/user-api-key/device"
                | "/user-api-key/device.json"
                | "/user-api-key/device/poll"
                | "/user-api-key/device/poll.json"
        )
}

pub async fn gate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let base_path = state.config.globals.relative_url_root().to_string();
    let path = request
        .uri()
        .path()
        .strip_prefix(base_path.as_str())
        .unwrap_or(request.uri().path())
        .to_string();
    if exempt(&path) {
        return Ok(next.run(request).await);
    }
    // The connection goes back to the pool before the handler runs: a
    // request must never hold two, or concurrent requests starve each
    // other on the pool.
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    if !settings.get("login_required")?.truthy() {
        return Ok(next.run(request).await);
    }
    // A current user passes, by session or API key.
    if request
        .extensions()
        .get::<crate::session::current::Incoming>()
        .is_some_and(|i| i.session.is_some() || i.guardian.user_id().is_some())
    {
        return Ok(next.run(request).await);
    }
    // /session is skip_before_action'd so people can log in.
    if path == "/session" || path == "/session.json" || path.starts_with("/session/") {
        return Ok(next.run(request).await);
    }

    let json = path.ends_with(".json");
    if json || request.method() != axum::http::Method::GET {
        // topics#show adds the not-found page, as 403 forbids it.
        if request.method() == axum::http::Method::GET
            && let Some(slug) = crate::not_found_page::topic_show_slug(&path)
        {
            let anonymous = crate::guardian::Guardian::anonymous();
            let extras =
                super::topics::not_found_extras(&state, &anonymous, &slug, true, None).await?;
            let mut response = not_logged_in(&state);
            let body = serde_json::json!({
                "errors": [state.i18n.t("not_logged_in").unwrap_or("You need to be logged in to do that.")],
                "error_type": "not_logged_in",
                "extras": extras,
            });
            *response.body_mut() = Body::from(body.to_string());
            return Ok(response);
        }
        return Ok(not_logged_in(&state));
    }
    let headers: HeaderMap = request.extract_parts().await?;
    let uri: Uri = request.uri().clone();
    if path == "/login" {
        return login_page(&state, &settings, &headers, &uri).await;
    }
    match redirect_to_login(&state, &settings, &headers, &uri).await? {
        Some(response) => Ok(response),
        // The front page renders the login page itself.
        None => login_page(&state, &settings, &headers, &uri).await,
    }
}

/// `ApplicationController#redirect_to_login`: straight to DiscourseConnect
/// (the url kept in the server session), or to the only external login
/// when local logins are off (unless a signup through it is under way),
/// else to /login with the url in the destination_url cookie. None for
/// the front page, which shows the login page itself.
pub(super) async fn redirect_to_login(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Option<Response>, AppError> {
    let base_path = state.config.globals.relative_url_root();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let original_url = format!(
        "http://{host}{}",
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
    );
    let destination_cookie = format!(
        "destination_url={}; path=/; SameSite=Lax",
        form_urlencoded::byte_serialize(original_url.as_bytes()).collect::<String>()
    );
    let redirect = |location: String, cookies: Vec<String>| -> Result<Response, AppError> {
        let mut response = (
            StatusCode::FOUND,
            [
                (header::LOCATION, location),
                (header::CACHE_CONTROL, "no-cache, no-store".to_string()),
            ],
            Body::empty(),
        )
            .into_response();
        for c in cookies {
            response
                .headers_mut()
                .append(header::SET_COOKIE, HeaderValue::from_str(&c)?);
        }
        Ok(response)
    };
    let authentication_data =
        crate::session::current::cookie(headers, "authentication_data").is_some();
    let auth_immediately = settings.get("auth_immediately")?.truthy();
    if auth_immediately && settings.get("enable_discourse_connect")?.truthy() {
        // server_session[:destination_url] = destination_url
        let mut session = crate::session::forum::load_forum_session(state, headers);
        let session_id = session.server_session_id();
        let mut conn = state.pool.acquire().await?;
        crate::session::server::set(
            &mut conn,
            &session_id,
            "destination_url",
            &json!(original_url),
            crate::session::server::EXPIRY_SECONDS,
        )
        .await?;
        let cookies = session.set_cookie(state, settings)?.into_iter().collect();
        return Ok(Some(redirect(
            format!("http://{host}{base_path}/session/sso"),
            cookies,
        )?));
    }
    if auth_immediately && !settings.get("enable_local_logins")?.truthy() && !authentication_data {
        let enabled = crate::auth::enabled(settings)?;
        if let [only] = enabled.as_slice() {
            return Ok(Some(redirect(
                format!("http://{host}{base_path}/auth/{}", only.name),
                vec![destination_cookie],
            )?));
        }
    }
    let front =
        uri.path() == format!("{base_path}/") || (base_path.is_empty() && uri.path() == "/");
    if front && !authentication_data {
        return Ok(None);
    }
    // `cookies[:destination_url] = destination_url`; javascript redirects
    // there after login.
    Ok(Some(redirect(
        format!("http://{host}{base_path}/login"),
        vec![destination_cookie],
    )?))
}

/// Marks a `not_logged_in` response, which a page request (a GET that is
/// neither JSON nor XHR) gets as the not-found page instead
/// (not_found::html_errors), as Rails rescues NotLoggedIn.
#[derive(Clone, Copy)]
pub(super) struct NotLoggedIn;

/// `rescue_discourse_actions(:not_logged_in, 403)`. (topics#show's extras
/// are added by the login_required gate, the one place it raises this.)
pub(super) fn not_logged_in(state: &AppState) -> Response {
    let i18n = &state.i18n;
    let mut body = serde_json::Map::new();
    body.insert(
        "errors".into(),
        json!([i18n
            .t("not_logged_in")
            .unwrap_or("You need to be logged in to do that.")]),
    );
    body.insert("error_type".into(), json!("not_logged_in"));
    let mut response = (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(serde_json::Value::Object(body)),
    )
        .into_response();
    response.extensions_mut().insert(NotLoggedIn);
    response
}

#[derive(Template)]
#[template(path = "login_required.html")]
pub struct LoginRequiredPage {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    pub welcome: String,
    /// The note that reading needs an account.
    pub login_required: bool,
    /// Username and password logins are on (the form); external logins are
    /// not ported.
    pub local_logins: bool,
    /// DiscourseConnect is on: the login route sends the browser to it.
    pub discourse_connect: bool,
    /// The enabled external logins' buttons (login-buttons).
    pub external_logins: Vec<LoginButton>,
    pub csrf_token: String,
}

/// An external login's button (the login-buttons component).
pub struct LoginButton {
    pub name: String,
    /// `title_override`, else `login.<name>.title`.
    pub title: String,
    /// The icon, inlined.
    pub icon: String,
}

/// The enabled authenticators' buttons, as LoginMethod describes them.
fn login_buttons(state: &AppState, settings: &SiteSettings) -> Result<Vec<LoginButton>, AppError> {
    let mut out = Vec::new();
    if settings.get("enable_discourse_connect")?.truthy() {
        return Ok(out);
    }
    for p in crate::auth::enabled(settings)? {
        let title = match p.title_setting {
            Some(setting) => settings.get(setting)?.to_s(),
            None => state
                .i18n
                .t(&format!("js.login.{}.title", p.name))
                .unwrap_or(p.name)
                .to_string(),
        };
        let icon = crate::svg_sprite::raw_svg(p.icon.unwrap_or("right-to-bracket"))?;
        out.push(LoginButton {
            name: p.name.to_string(),
            title,
            icon,
        });
    }
    Ok(out)
}

/// The login page: what `/login` shows, and what the front page shows
/// while login is required.
pub async fn login_page(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut site = crate::html::Site::from_settings(settings, &base_path)?;
    let mut conn = state.pool.acquire().await?;
    site.load_chrome(
        &mut conn,
        state,
        settings,
        &crate::guardian::Guardian::anonymous(),
        crate::sidebar::Active::None,
    )
    .await?;
    // The login route is one of ALL_PAGES_EXCLUDED_ROUTES.
    site.chrome.hide_search_button = true;
    let welcome = state
        .i18n
        .t_with(
            "login_required.welcome_message",
            &[("title", &site.site_title)],
        )
        .unwrap_or_else(|| format!("Welcome to {}", site.site_title));
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    // The form posts to /session, which checks this token; the page is never
    // cached, so it can carry one.
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(state, headers, settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    crawler.description = site.site_description.clone();
    let page = LoginRequiredPage {
        crawler,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        welcome: welcome.trim_start_matches("# ").to_string(),
        login_required: settings.get("login_required")?.truthy(),
        local_logins: settings.get("enable_local_logins")?.truthy()
            && !settings.get("enable_discourse_connect")?.truthy(),
        discourse_connect: settings.get("enable_discourse_connect")?.truthy(),
        external_logins: login_buttons(state, settings)?,
        csrf_token,
    };
    let mut response = (
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, must-revalidate, private, max-age=0"),
        )],
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    if let Some(cookie) = set_cookie {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

#[derive(Template)]
#[template(path = "signup.html")]
pub struct SignupPage {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    /// Username and password signups are on; an external login's new
    /// account is taken either way.
    pub local_logins: bool,
    pub unavailable: String,
    pub csrf_token: String,
}

/// GET /signup: static#show with id "signup", the create-account form.
/// A logged in reader goes to the front page.
pub async fn show_signup(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    if !guardian.is_anonymous() {
        let location = format!("{}/", urls.base_url()?);
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, HeaderValue::from_str(&location)?)],
        )
            .into_response());
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(&state, &headers, &settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;
    crawler.description = site.site_description.clone();
    let page = SignupPage {
        crawler,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        local_logins: settings.get("enable_local_logins")?.truthy()
            && !settings.get("enable_discourse_connect")?.truthy(),
        unavailable: state
            .i18n
            .t("login.new_registrations_disabled")
            .unwrap_or("New account registrations are not allowed at this time.")
            .to_string(),
        csrf_token,
    };
    let mut response = (
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, must-revalidate, private, max-age=0"),
        )],
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    if let Some(cookie) = set_cookie {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// GET /login
pub async fn show_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    login_page(&state, &settings, &headers, &uri).await
}
