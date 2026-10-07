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
use crate::{AppError, AppState, Unsupported};

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
    ) || path.starts_with("/assets/")
        || path.starts_with("/fonts/")
        || path.starts_with("/letter_avatar_proxy/")
        || path.starts_with("/images/")
        || path.starts_with("/uploads/")
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
        return Ok(not_logged_in(&state, &path));
    }
    let headers: HeaderMap = request.extract_parts().await?;
    let uri: Uri = request.uri().clone();
    if path == "/" || path == "/login" {
        check_auth_immediately(&settings)?;
        return login_page(&state, &settings, &headers, &uri).await;
    }
    redirect_to_login(&state, &settings, &headers, &uri)
}

/// redirect_to_login's auth_immediately branches send readers straight to
/// DiscourseConnect, or to the only external login when local logins are
/// off; neither exists here yet.
fn check_auth_immediately(settings: &SiteSettings) -> Result<(), AppError> {
    if settings.get("auth_immediately")?.truthy()
        && (settings.get("enable_discourse_connect")?.truthy()
            || !settings.get("enable_local_logins")?.truthy())
    {
        return Err(
            Unsupported("auth_immediately (DiscourseConnect or single external login)").into(),
        );
    }
    Ok(())
}

/// `ApplicationController#redirect_to_login` away from the front page.
pub(super) fn redirect_to_login(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    check_auth_immediately(settings)?;
    let base_path = state.config.globals.relative_url_root();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let original_url = format!(
        "http://{host}{}",
        uri.path_and_query().map(|p| p.as_str()).unwrap_or("/")
    );
    // `cookies[:destination_url] = destination_url`; javascript redirects
    // there after login.
    let cookie = format!(
        "destination_url={}; path=/; SameSite=Lax",
        form_urlencoded::byte_serialize(original_url.as_bytes()).collect::<String>()
    );
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, format!("http://{host}{base_path}/login")),
            (header::SET_COOKIE, cookie),
            (header::CACHE_CONTROL, "no-cache, no-store".to_string()),
        ],
        Body::empty(),
    )
        .into_response())
}

/// `rescue_discourse_actions(:not_logged_in, 403)`; topics#show adds the
/// not-found extras.
pub(super) fn not_logged_in(state: &AppState, path: &str) -> Response {
    let i18n = &state.i18n;
    let mut body = serde_json::Map::new();
    body.insert(
        "errors".into(),
        json!([i18n
            .t("not_logged_in")
            .unwrap_or("You need to be logged in to do that.")]),
    );
    body.insert("error_type".into(), json!("not_logged_in"));
    if path.starts_with("/t/") {
        body.insert(
            "extras".into(),
            json!({
                "title": i18n.t("page_not_found.page_title").unwrap_or("Page Not Found"),
                "html": "",
                "group": null,
            }),
        );
    }
    (
        StatusCode::FORBIDDEN,
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(serde_json::Value::Object(body)),
    )
        .into_response()
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
    pub csrf_token: String,
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
    site.load_chrome(
        state,
        settings,
        &crate::guardian::Guardian::anonymous(),
        crate::sidebar::Active::None,
    )
    .await?;
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
        local_logins: settings.get("enable_local_logins")?.truthy(),
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
