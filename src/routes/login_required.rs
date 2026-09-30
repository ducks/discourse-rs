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

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// Paths served without a session even when login is required: the
/// controllers that `skip_before_action :redirect_to_login_if_required`,
/// plus the static files nginx serves in front of Rails.
fn exempt(path: &str) -> bool {
    matches!(
        path,
        "/srv/status" | "/site/basic-info" | "/site/basic-info.json" | "/assets/site.css"
    ) || path.starts_with("/images/")
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
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("login_required")?.truthy() {
        return Ok(next.run(request).await);
    }
    drop(conn);

    let json = path.ends_with(".json");
    if json || request.method() != axum::http::Method::GET {
        return Ok(not_logged_in(&state, &path));
    }
    // redirect_to_login: auth_immediately sends readers straight to
    // DiscourseConnect, or to the only external login when local logins
    // are off; neither exists here yet.
    if settings.get("auth_immediately")?.truthy()
        && (settings.get("enable_discourse_connect")?.truthy()
            || !settings.get("enable_local_logins")?.truthy())
    {
        return Err(
            Unsupported("auth_immediately (DiscourseConnect or single external login)").into(),
        );
    }
    let headers: HeaderMap = request.extract_parts().await?;
    let uri: Uri = request.uri().clone();
    if path == "/" || path == "/login" {
        let site = crate::html::Site::from_settings(&settings, &base_path)?;
        let welcome = state
            .i18n
            .t_with(
                "login_required.welcome_message",
                &[("title", &site.site_title)],
            )
            .unwrap_or_else(|| format!("Welcome to {}", site.site_title));
        let page = LoginRequiredPage {
            site_title: site.site_title,
            site_description: site.site_description,
            lang: site.lang,
            base_path: site.base_path,
            welcome: welcome.trim_start_matches("# ").to_string(),
        };
        return Ok((
            [(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store, must-revalidate, private, max-age=0"),
            )],
            Html(page.render().map_err(crate::html::HtmlError::from)?),
        )
            .into_response());
    }
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
fn not_logged_in(state: &AppState, path: &str) -> Response {
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
    pub site_description: String,
    pub lang: String,
    pub base_path: String,
    pub welcome: String,
}
