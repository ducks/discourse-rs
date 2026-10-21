//! `ApplicationController#redirect_to_login_if_required` for anonymous
//! requests: with `login_required` on, JSON gets a 403 `not_logged_in`,
//! HTML is sent to `/login` (the front page shows the login-required page),
//! and only `/srv/status`, `/site/basic-info` and what signing up and
//! logging in need stay open. Also the login flow's pages: login, signup,
//! the login-required front page and login-preferences.

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
            // StaticController skips the gate; /login renders below.
            | "/signup"
            | "/password-reset"
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
        // Plugins' public files, served statically.
        || path.starts_with("/plugins/")
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

/// UsersController's `skip_before_action :redirect_to_login_if_required`
/// actions, under both of its roots: what signing up, activating an
/// account, resetting a password and logging in by email need.
fn users_exempt(path: &str, method: &axum::http::Method) -> bool {
    let Some(rest) = path
        .strip_prefix("/users")
        .or_else(|| path.strip_prefix("/u"))
    else {
        return false;
    };
    // users#create
    if matches!(rest, "" | ".json") {
        return method == axum::http::Method::POST;
    }
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let action = rest.split('/').next().unwrap_or("");
    let action = action.strip_suffix(".json").unwrap_or(action);
    matches!(
        action,
        "check_username"
            | "check_email"
            | "account-created"
            | "activate-account"
            | "update-activation-email"
            | "password-reset"
            | "confirm-email-token"
            | "email-login"
            | "admin-login"
            | "confirm-admin"
            | "generate_random_username"
    ) || rest == "action/send_activation_email"
        || rest == "action/send_activation_email.json"
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
    if exempt(&path) || users_exempt(&path, request.method()) {
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

    // `request.format.json? || request.xhr?`: request_format sent the other
    // JSON pages to their `.json` twins, but not the front page.
    let json = path.ends_with(".json")
        || (path == "/"
            && (super::wants_json(request.uri().query(), request.headers())
                || request
                    .headers()
                    .get("x-requested-with")
                    .is_some_and(|v| v == "XMLHttpRequest")));
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
        None => login_required_page(&state, &settings, &headers, &uri).await,
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

/// The pages of the login flow (login, signup, the login-required front
/// page, password reset): what src/login_page_view.rs draws, with the
/// data and strings static/js/login.js works with.
#[derive(Template)]
#[template(path = "login_required.html")]
pub struct AuthPage {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: Chrome,
    /// The title's own part, before the site's.
    pub title: String,
    pub body: String,
    /// #login-data: the page, its token, the settings the script reads
    /// and its strings.
    pub data: String,
    /// #dialog-strings
    pub dialog_strings: String,
    /// What the page adds to the head.
    pub head: String,
}

/// The client strings static/js/login.js translates with.
const STRINGS: &[&str] = &[
    "log_in",
    "login.title",
    "login.logging_in",
    "login.blank_username_or_password",
    "login.blank_username",
    "login.error",
    "login.rate_limit",
    "login.cookies_error",
    "login.password_expired",
    "login.not_activated",
    "login.resend_title",
    "login.change_email",
    "login.show_password_title",
    "login.hide_password_title",
    "login.second_factor_title",
    "login.second_factor_description",
    "login.second_factor_backup_title",
    "login.second_factor_backup_description",
    "login.second_factor_backup",
    "login.second_factor",
    "login.security_key_description",
    "login.security_key_authenticate",
    "login.security_key_alternative",
    "login.security_key_not_allowed_error",
    "login.security_key_no_matching_credential_error",
    "login.security_key_support_missing_error",
    "login.passkey_2fa_description",
    "login.caps_lock_warning",
    "read_only_mode.login_disabled",
    "staff_writes_only_mode.login_disabled",
    "email_login.complete_email",
    "email_login.complete_username",
    "email_login.complete_email_found",
    "email_login.complete_username_found",
    "email_login.complete_email_not_found",
    "email_login.complete_username_not_found",
    "forgot_password.title",
    "forgot_password.reset",
    "forgot_password.invite",
    "forgot_password.invite_no_username",
    "forgot_password.email-username",
    "forgot_password.button_ok",
    "forgot_password.button_help",
    "forgot_password.help",
    "forgot_password.complete_email",
    "forgot_password.complete_username",
    "forgot_password.complete_email_found",
    "forgot_password.complete_username_found",
    "forgot_password.complete_email_not_found",
    "forgot_password.complete_username_not_found",
    "email",
    "login.email_placeholder",
    "code_login.email_label",
    "user.email.required",
    "user.email.invalid",
    "user.email.ok",
    "user.email.authenticated",
    "user.username.prefilled",
    "user.username.required",
    "user.username.too_short",
    "user.username.too_long",
    "user.username.checking",
    "user.username.available",
    "user.username.not_available",
    "user.username.not_available_no_suggestion",
    "user.name.required",
    "user.password.required",
    "user.password.too_short",
    "user.password.same_as_username",
    "user.password.same_as_name",
    "user.password.same_as_email",
    "user.password.ok",
    "user.password.common",
    "user.change_password.title",
    "user.change_password.verify_identity",
    "user.change_password.choose_new",
    "user.change_password.set_password",
    "user.second_factor.rate_limit",
    "user.second_factor_backup.use",
    "user.second_factor.use",
    "login.not_approved",
    "submit",
    "create_account.failed",
    "create_account.associate",
    "d_otp.screen_reader",
    "login.passkey_security_error",
    "login.security_key_invalid_response_error",
    "form_kit.errors.required",
    "form_kit.errors_summary_title",
    "code_login.check_your_email",
    "code_login.verify_button",
    "code_login.resend",
    "code_login.resend_countdown",
    "code_login.use_different_email",
    "code_login.code_resent",
    "forgot_password.code_instructions",
];

/// AuthPage's chrome: the application layout as these routes leave it
/// (hideApplicationSidebar, hideApplicationHeaderButtons for search, log
/// in, sign up and the menu), with the route's body class.
async fn auth_chrome(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    body_classes: &[&str],
) -> Result<crate::html::Site, AppError> {
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
    site.chrome.sidebar.clear();
    site.chrome.hamburger.clear();
    site.chrome.sidebar_toggle = false;
    site.chrome.hide_search_button = true;
    site.chrome.hide_auth_buttons = true;
    let mut classes: Vec<&str> = site
        .chrome
        .body_classes
        .split_whitespace()
        .filter(|c| *c != "has-sidebar-page")
        .collect();
    classes.extend(body_classes);
    site.chrome.body_classes = classes.join(" ");
    Ok(site)
}

/// The enabled authenticators' buttons, as LoginMethod describes them.
fn auth_buttons(
    state: &AppState,
    settings: &SiteSettings,
) -> Result<Vec<crate::login_page_view::AuthButton>, AppError> {
    let mut out = Vec::new();
    if settings.get("enable_discourse_connect")?.truthy() {
        return Ok(out);
    }
    // discourse-lti hides its button: the platform starts its logins.
    for p in crate::auth::enabled(settings)?
        .into_iter()
        .filter(|p| p.name != "lti")
    {
        let json = p.serialize(settings)?;
        let title_override = json["title_override"]
            .as_str()
            .filter(|t| !t.is_empty())
            .map(str::to_string);
        let t = |key: &str| state.i18n.t(&format!("js.login.{}.{key}", p.name));
        let title = title_override
            .clone()
            .or_else(|| t("title").map(str::to_string))
            .unwrap_or_else(|| format!("[en.login.{}.title]", p.name));
        out.push(crate::login_page_view::AuthButton {
            name: p.name.to_string(),
            sr_title: title_override
                .or_else(|| t("sr_title").map(str::to_string))
                .unwrap_or_else(|| title.clone()),
            title,
            icon: json["icon_override"]
                .as_str()
                .filter(|i| !i.is_empty())
                .unwrap_or("user")
                .to_string(),
        });
    }
    Ok(out)
}

/// What the script needs besides the page: the route, the token its
/// requests carry, settings, and the strings.
fn login_data(
    state: &AppState,
    settings: &SiteSettings,
    page: &str,
    csrf_token: &str,
    extra: serde_json::Map<String, serde_json::Value>,
) -> Result<String, AppError> {
    let t = |k: &str| state.i18n.t(&format!("js.{k}")).map(str::to_string);
    let mut strings = serde_json::Map::new();
    for k in STRINGS {
        // A pluralized string goes as its forms, which the script picks
        // by count.
        match (t(k), t(&format!("{k}.one")), t(&format!("{k}.other"))) {
            (Some(s), _, _) => {
                strings.insert(k.to_string(), json!(s));
            }
            (None, one, Some(other)) => {
                strings.insert(format!("{k}.one"), json!(one.unwrap_or(other.clone())));
                strings.insert(format!("{k}.other"), json!(other));
            }
            (None, _, None) => {
                strings.insert(k.to_string(), json!(format!("[en.{k}]")));
            }
        }
    }
    let mut data = json!({
        "page": page,
        "csrf": csrf_token,
        "strings": strings,
        "settings": {
            "enable_local_logins": settings.get("enable_local_logins")?.truthy(),
            "enable_discourse_connect": settings.get("enable_discourse_connect")?.truthy(),
            "hide_email_address_taken": settings.get("hide_email_address_taken")?.truthy(),
            "min_username_length": settings.get("min_username_length")?.to_i(),
            "max_username_length": settings.get("max_username_length")?.to_i(),
            "min_password_length": settings.get("min_password_length")?.to_i(),
            "min_admin_password_length": settings.get("min_admin_password_length")?.to_i(),
            "must_approve_users": settings.get("must_approve_users")?.truthy(),
            "email_editable": settings.get("email_editable")?.truthy(),
            "full_name_required": settings.get("full_name_requirement")?.to_s() == "required_at_signup",
            "title": settings.get("title")?.to_s(),
        },
    });
    if let Some(obj) = data.as_object_mut() {
        obj.extend(extra);
    }
    Ok(data.to_string().replace('<', "\\u003c"))
}

fn dialog_strings(state: &AppState) -> String {
    let t = |k: &str| {
        state
            .i18n
            .t(&format!("js.{k}"))
            .unwrap_or_default()
            .to_string()
    };
    json!({
        "ok_value": t("ok_value"),
        "cancel": t("cancel"),
        "yes_value": t("yes_value"),
        "no_value": t("no_value"),
        "modal.close": t("modal.close"),
    })
    .to_string()
    .replace('<', "\\u003c")
}

/// Renders `page` never cached, with the anonymous session's cookie.
fn auth_response(page: AuthPage, set_cookie: Option<String>) -> Result<Response, AppError> {
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

/// `UpcomingChanges` enable_local_logins_via_code with its dependencies:
/// login codes by email, with local logins.
async fn code_logins(
    state: &AppState,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
) -> Result<bool, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(guardian
        .upcoming_change_enabled(&mut conn, settings, "enable_local_logins_via_code")
        .await?
        && settings.get("enable_local_logins_via_email")?.truthy()
        && settings.get("enable_local_logins")?.truthy())
}

/// The login page (templates/login.gjs): what `/login` shows, and what the
/// login gate shows for a page that needs a login.
pub async fn login_page(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    login_page_with(state, settings, headers, uri, false).await
}

/// The login page, with the ForgotPassword modal open over it (`forgot`)
/// as the forgot-password route leaves a login_required site.
async fn login_page_with(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
    forgot: bool,
) -> Result<Response, AppError> {
    // The login route's beforeModel: in read-only mode only staff log in,
    // the others get an alert on the homepage.
    let mode = {
        let mut conn = state.pool.acquire().await?;
        crate::readonly::mode(&mut conn).await?
    };
    if mode.readonly && !mode.staff_writes_only {
        return Err(
            Unsupported("the login page in read-only mode (its dialog on the homepage)").into(),
        );
    }
    let anonymous = crate::guardian::Guardian::anonymous();
    let site = auth_chrome(state, settings, &anonymous, &["login-page"]).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(state, headers, settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    crawler.description = site.site_description.clone();
    let code_login = code_logins(state, settings, &anonymous).await?;
    let code_mode = uri.query().is_some_and(|q| {
        form_urlencoded::parse(q.as_bytes()).any(|(k, v)| k == "mode" && v == "code")
    });
    let local = settings.get("enable_local_logins")?.truthy();
    let buttons = auth_buttons(state, settings)?;
    let t = |key: &str| site.chrome.t(key);
    let body = crate::login_page_view::login_page(
        &crate::login_page_view::LoginView {
            cx: crate::login_page_view::Cx {
                t: &t,
                mobile: crate::mobile::mobile_view(),
            },
            local,
            via_email: settings.get("enable_local_logins_via_email")?.truthy(),
            code_login,
            code_mode,
            passkeys: local && settings.get("enable_passkeys")?.truthy(),
            buttons: &buttons,
            can_sign_up: site.chrome.can_signup && !mode.readonly,
        },
        &site.base_path,
    );
    let mut extra = serde_json::Map::new();
    extra.insert("code_login".into(), json!(code_login));
    extra.insert(
        "passkeys".into(),
        json!(local && settings.get("enable_passkeys")?.truthy()),
    );
    extra.insert("forgot_password".into(), json!(forgot));
    // The login route's beforeModel: DiscourseConnect, or the only
    // external login, starts at once.
    extra.insert(
        "single_external".into(),
        json!(if !local && buttons.len() == 1 {
            Some(buttons[0].name.clone())
        } else {
            None
        }),
    );
    let data = login_data(state, settings, "login", &csrf_token, extra)?;
    let page = AuthPage {
        crawler,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        title: String::new(),
        body,
        data,
        dialog_strings: dialog_strings(state),
        head: String::new(),
    };
    auth_response(page, set_cookie)
}

/// The front page while login is required (discovery.login-required):
/// the static login page, with Sign Up and Log In.
pub async fn login_required_page(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let anonymous = crate::guardian::Guardian::anonymous();
    let site = auth_chrome(state, settings, &anonymous, &["login-page", "static-login"]).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(state, headers, settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    crawler.description = site.site_description.clone();
    let content = static_login_content(state, settings, &site.base_path).await?;
    let readonly = {
        let mut conn = state.pool.acquire().await?;
        crate::readonly::mode(&mut conn).await?.readonly
    };
    let t = |key: &str| site.chrome.t(key);
    let body = crate::login_page_view::login_required_page(
        &crate::login_page_view::Cx {
            t: &t,
            mobile: crate::mobile::mobile_view(),
        },
        &content,
        site.chrome.can_signup && !readonly,
    );
    let data = login_data(
        state,
        settings,
        "login-required",
        &csrf_token,
        serde_json::Map::new(),
    )?;
    let page = AuthPage {
        crawler,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        title: String::new(),
        body,
        data,
        dialog_strings: dialog_strings(state),
        head: String::new(),
    };
    auth_response(page, set_cookie)
}

/// static/login.html.erb, the static page StaticPage.find("login") reads.
async fn static_login_content(
    state: &AppState,
    settings: &SiteSettings,
    base_path: &str,
) -> Result<String, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let mut conn = state.pool.acquire().await?;
    let mobile = crate::mobile::on_mobile_device();
    let dark_setting = if mobile {
        "mobile_logo_dark"
    } else {
        "logo_dark"
    };
    if !crate::site_icons::site_url(&mut conn, &urls, dark_setting)
        .await?
        .is_empty()
    {
        return Err(
            Unsupported("the static login page's dark logo (application_logo_dark_url)").into(),
        );
    }
    // application_logo_url: site_mobile_logo_url (the logo when there is
    // no mobile one) on a phone.
    let logo = crate::site_icons::site_url(
        &mut conn,
        &urls,
        if mobile { "mobile_logo" } else { "logo" },
    )
    .await?;
    let title = settings.get("title")?.to_s();
    // waving_hand_url: a skin tone at random.
    let tone = rand::Rng::gen_range(&mut rand::thread_rng(), 2..=6);
    let hand = settings
        .emojis()
        .url_for(base_path, &format!("wave:t{tone}"));
    let store = crate::file_store::FileStore::for_site(&state.config, settings)?;
    let hand =
        crate::pretty_text::cooked_post_processor::cook_url(&hand, &urls, base_path, &store)?;
    let welcome = state
        .i18n
        .t_with("login_required.welcome_message", &[("title", &title)])
        .unwrap_or_default();
    let host = crate::pretty_text::Host::from_state(state);
    let cooked = crate::pretty_text::cook(
        &host,
        &welcome,
        &crate::pretty_text::MarkdownOptions::default(),
    )
    .await?;
    Ok(crate::login_page_view::static_login_content(
        (!logo.is_empty()).then_some((logo.as_str(), None)),
        &hand,
        &title,
        &cooked,
        &settings.get("site_description")?.to_s(),
    ))
}

/// Sets `email` from the query in the cookie the login and signup pages
/// prefill from (StaticController#show, PAGES_WITH_EMAIL_PARAM).
fn email_cookie(uri: &Uri) -> Option<String> {
    let q = uri.query()?;
    let (_, email) = form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "email")?;
    let expires = (crate::clock::now() + chrono::Duration::days(1))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    Some(format!(
        "email={}; path=/; expires={expires}",
        form_urlencoded::byte_serialize(email.as_bytes()).collect::<String>()
    ))
}

fn redirect(location: &str) -> Result<Response, AppError> {
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_str(location)?)],
    )
        .into_response())
}

/// GET /signup: static#show with id "signup" (templates/signup.gjs). A
/// logged in reader goes to the front page.
pub async fn show_signup(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    if !guardian.is_anonymous() {
        return redirect(&format!("{}/", urls.base_url()?));
    }
    let site = auth_chrome(&state, &settings, &guardian, &["signup-page"]).await?;
    // The signup route's beforeModel: read-only mode alerts on the way
    // to the homepage; closed signups go there.
    let mode = {
        let mut conn = state.pool.acquire().await?;
        crate::readonly::mode(&mut conn).await?
    };
    if mode.readonly {
        return Err(
            Unsupported("the signup page in read-only mode (its dialog on the homepage)").into(),
        );
    }
    if !site.chrome.can_signup
        && (settings.get("invite_only")?.truthy() || !settings.get("auth_immediately")?.truthy())
    {
        return redirect(&format!("{}/", urls.base_url()?));
    }
    let (csrf_token, mut set_cookie) = super::session::anonymous_csrf(&state, &headers, &settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;
    crawler.description = site.site_description.clone();
    let local = settings.get("enable_local_logins")?.truthy();
    let code_signup = code_logins(&state, &settings, &guardian).await?;
    let buttons = auth_buttons(&state, &settings)?;
    let user_fields: i64 = {
        let mut conn = state.pool.acquire().await?;
        sqlx::query_scalar("SELECT count(*) FROM user_fields WHERE show_on_signup")
            .fetch_one(&mut *conn)
            .await?
    };
    if user_fields > 0 {
        return Err(Unsupported("the signup page's user fields (UserField)").into());
    }
    let disclaimer = {
        let mut conn = state.pool.acquire().await?;
        let base = state.config.globals.relative_url_root();
        let tos = super::invite_redeem::legal_url(
            &mut conn,
            &settings,
            base,
            "tos_url",
            "tos_topic_id",
            "/tos",
        )
        .await?;
        let privacy = super::invite_redeem::legal_url(
            &mut conn,
            &settings,
            base,
            "privacy_policy_url",
            "privacy_topic_id",
            "/privacy",
        )
        .await?;
        match (tos, privacy) {
            (Some(tos), Some(privacy)) => Some(
                site.chrome
                    .t("create_account.disclaimer")
                    .replace("%{tos_link}", &crate::topic_list_view::escape(&tos))
                    .replace("%{privacy_link}", &crate::topic_list_view::escape(&privacy)),
            ),
            _ => None,
        }
    };
    let full_name = settings.get("full_name_requirement")?.to_s();
    let t = |key: &str| site.chrome.t(key);
    let body = crate::login_page_view::signup_page(
        &crate::login_page_view::SignupView {
            cx: crate::login_page_view::Cx {
                t: &t,
                mobile: crate::mobile::mobile_view(),
            },
            local,
            code_signup,
            buttons: &buttons,
            must_approve: settings.get("must_approve_users")?.truthy(),
            full_name_visible: full_name != "hidden_at_signup",
            full_name_required: full_name == "required_at_signup",
            // SiteSetting.require_invite_code
            require_invite_code: settings.get("invite_code")?.presence().is_some(),
            show_email_instructions: settings
                .get("show_signup_form_email_instructions")?
                .truthy(),
            show_username_instructions: settings
                .get("show_signup_form_username_instructions")?
                .truthy(),
            show_password_instructions: settings
                .get("show_signup_form_password_instructions")?
                .truthy(),
            show_full_name_instructions: settings
                .get("show_signup_form_full_name_instructions")?
                .truthy(),
            min_password_length: settings.get("min_password_length")?.to_i(),
            disclaimer,
        },
        &site.base_path,
    );
    let mut extra = serde_json::Map::new();
    extra.insert("code_login".into(), json!(code_signup));
    extra.insert(
        "single_external".into(),
        json!(if !local && buttons.len() == 1 {
            Some(buttons[0].name.clone())
        } else {
            None
        }),
    );
    let pretty: serde_json::Map<String, serde_json::Value> = buttons
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                json!(
                    state
                        .i18n
                        .t(&format!("js.login.{}.name", b.name))
                        .unwrap_or(&b.name)
                ),
            )
        })
        .collect();
    extra.insert("provider_names".into(), json!(pretty));
    let data = login_data(&state, &settings, "signup", &csrf_token, extra)?;
    if let Some(cookie) = email_cookie(&uri) {
        set_cookie = Some(match set_cookie {
            // auth_response sets one cookie; the session's goes first.
            Some(session) => {
                let mut response = auth_response(
                    AuthPage {
                        crawler,
                        site_title: site.site_title,
                        viewer: site.viewer,
                        bus_position: site.bus_position,
                        chrome: site.chrome,
                        lang: site.lang,
                        base_path: site.base_path,
                        title: String::new(),
                        body,
                        data,
                        dialog_strings: dialog_strings(&state),
                        head: String::new(),
                    },
                    Some(session),
                )?;
                response
                    .headers_mut()
                    .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
                return Ok(response);
            }
            None => cookie,
        });
    }
    let page = AuthPage {
        crawler,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        title: String::new(),
        body,
        data,
        dialog_strings: dialog_strings(&state),
        head: String::new(),
    };
    auth_response(page, set_cookie)
}

/// GET /login: static#show with id "login". A logged in reader goes to
/// the `redirect` param's page; for anyone else that page becomes the
/// destination_url the login returns to.
pub async fn show_login(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    let base_path = state.config.globals.relative_url_root();
    let redirect_param = uri.query().and_then(|q| {
        form_urlencoded::parse(q.as_bytes())
            .find(|(k, _)| k == "redirect")
            .map(|(_, v)| v.into_owned())
    });
    let destination = extract_redirect_param(redirect_param.as_deref(), base_path);
    if !guardian.is_anonymous() {
        let location = if destination == "/" {
            format!("{base_path}/")
        } else {
            destination
        };
        return redirect(&location);
    }
    let mut response = login_page(&state, &settings, &headers, &uri).await?;
    let mut cookies = Vec::new();
    if destination != "/" {
        cookies.push(format!(
            "destination_url={}; path=/",
            form_urlencoded::byte_serialize(destination.as_bytes()).collect::<String>()
        ));
    }
    cookies.extend(email_cookie(&uri));
    for cookie in cookies {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(&cookie)?);
    }
    Ok(response)
}

/// StaticController#extract_redirect_param: a path (and query) on this
/// host, not the login page; "/" for anything else.
fn extract_redirect_param(redirect: Option<&str>, base_path: &str) -> String {
    let Some(redirect) = redirect.filter(|r| !r.is_empty()) else {
        return "/".into();
    };
    let Ok(url) = url::Url::parse("http://placeholder.invalid").and_then(|b| b.join(redirect))
    else {
        return "/".into();
    };
    let path = url.path();
    // A host of its own is another site's.
    let same_host = url.host_str() == Some("placeholder.invalid");
    if !same_host
        || path.is_empty()
        || path.starts_with(&format!("{base_path}/login"))
        || !path.starts_with('/')
        || path.contains(['.', ' ', '\t', '\n'])
    {
        return "/".into();
    }
    match url.query() {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    }
}

/// GET /login-preferences (templates/login-preferences.gjs): log in to
/// reach one's preferences.
pub async fn show_login_preferences(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    {
        let mut conn = state.pool.acquire().await?;
        site.load_chrome(
            &mut conn,
            &state,
            &settings,
            &guardian,
            crate::sidebar::Active::None,
        )
        .await?;
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(&state, &headers, &settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;
    crawler.description = site.site_description.clone();
    let body = format!(
        "<div class=\"container\"><h2>{}</h2><p style=\"margin-top: 1em\">{}</p><button class=\"btn btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div>",
        crate::topic_list_view::escape(&site.chrome.t("login.to_continue")),
        crate::topic_list_view::escape(&site.chrome.t("login.preferences")),
        crate::topic_list_view::escape(&site.chrome.t("log_in")),
    );
    let data = login_data(
        &state,
        &settings,
        "login-preferences",
        &csrf_token,
        serde_json::Map::new(),
    )?;
    let page = AuthPage {
        crawler,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        title: String::new(),
        body,
        data,
        dialog_strings: dialog_strings(&state),
        head: String::new(),
    };
    auth_response(page, set_cookie)
}

/// GET /u/account-created: users#account_created, the page a new account
/// lands on (templates/account-created with its index): the message the
/// signup left in the server session and, for the account the session
/// created, the activation controls. A logged in reader goes on to where
/// they were headed.
pub async fn show_account_created(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    if !guardian.is_anonymous() {
        if settings.get("enable_discourse_connect_provider")?.truthy()
            && crate::session::current::cookie(&headers, "sso_payload").is_some()
        {
            return Err(Unsupported("account_created's DiscourseConnect provider payload").into());
        }
        if let Some(destination) = crate::session::current::cookie(&headers, "destination_url") {
            let destination = form_urlencoded::parse(format!("d={destination}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            let mut response = redirect(&destination)?;
            response.headers_mut().append(
                header::SET_COOKIE,
                HeaderValue::from_static("destination_url=; path=/; max-age=0"),
            );
            return Ok(response);
        }
        return redirect(&format!("{base_path}/"));
    }
    // format.json: success_json, the page's data being the HTML's.
    if uri.path().ends_with(".json") || super::wants_json(uri.query(), &headers) {
        return Ok((
            [(header::CACHE_CONTROL, "no-cache, no-store")],
            Json(json!({ "success": "OK" })),
        )
            .into_response());
    }
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut conn = state.pool.acquire().await?;
    let message = crate::session::server::get(&mut conn, &session_id, "user_created_message")
        .await?
        .and_then(|m| m.as_str().map(str::to_string))
        .unwrap_or_else(|| {
            state
                .i18n
                .t("activation.missing_session")
                .unwrap_or_default()
                .to_string()
        });
    let user_id = match session.map.get("activate_user") {
        Some(crate::session::cookie::Scalar::Int(id)) => Some(*id),
        Some(crate::session::cookie::Scalar::Str(id)) => id.parse().ok(),
        _ => None,
    };
    // show_controls: the account the session created, unless it was staged.
    let mut account: Option<(String, String)> = None;
    let mut show_controls = false;
    if let Some(id) = user_id.and_then(|id| i32::try_from(id).ok()) {
        let row: Option<(String, Option<String>, bool)> = sqlx::query_as(
            "SELECT u.username, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\"), \
                    EXISTS (SELECT 1 FROM user_custom_fields WHERE user_id = u.id AND name = 'from_staged' AND value = 't') \
             FROM users u WHERE u.id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((username, email, from_staged)) = row {
            show_controls = !from_staged;
            account = Some((username, email.unwrap_or_default()));
        }
    }
    drop(conn);
    let site = auth_chrome(&state, &settings, &guardian, &["account-created-page"]).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let (csrf_token, set_cookie) = super::session::anonymous_csrf(&state, &headers, &settings)?;
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;
    crawler.description = site.site_description.clone();
    let t = |key: &str| site.chrome.t(key);
    let body = crate::login_page_view::account_created_page(
        &crate::login_page_view::Cx {
            t: &t,
            mobile: crate::mobile::mobile_view(),
        },
        &settings.get("title")?.to_s(),
        &message,
        show_controls,
        settings.get("must_approve_users")?.truthy(),
        settings.get("enable_local_logins")?.truthy() || settings.get("email_editable")?.truthy(),
    );
    let mut extra = serde_json::Map::new();
    extra.insert(
        "account".into(),
        json!(account.map(|(username, email)| json!({"username": username, "email": email}))),
    );
    let data = login_data(&state, &settings, "account-created", &csrf_token, extra)?;
    let page = AuthPage {
        crawler,
        title: site.chrome.t("create_account.activation_title"),
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        body,
        data,
        dialog_strings: dialog_strings(&state),
        head: String::new(),
    };
    auth_response(page, set_cookie)
}

/// What users#password_reset_show tells the page about the token's user.
struct ResetUser {
    id: i32,
    is_developer: bool,
    admin: bool,
    enabled: crate::second_factor::Enabled,
}

/// GET /u/password-reset/:token(.json): users#password_reset_show. A token
/// that still confirms (or one this session already confirmed) gets the
/// page (templates/password-reset.gjs) with the security keys' challenge
/// staged; any other the no_ember error page, or its message as JSON.
pub async fn password_reset_show(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    axum::extract::Path(token): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let json_format = token.ends_with(".json");
    let token = token.strip_suffix(".json").unwrap_or(&token).to_string();
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    let mut session = crate::session::forum::load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let mut tx = state.pool.begin().await?;
    // password_reset_find_user(token, committing_change: false)
    let valid_hours = settings.get("email_token_valid_hours")?.to_i();
    let confirmable: Option<i32> = sqlx::query_scalar(
        "SELECT t.user_id FROM email_tokens t JOIN users u ON u.id = t.user_id \
         WHERE t.token_hash = $1 AND NOT t.confirmed AND NOT t.expired \
           AND t.created_at >= now() - make_interval(hours => $2) AND (t.scope = $3 OR t.scope IS NULL) \
         ORDER BY (t.scope = $3) DESC NULLS LAST, t.id LIMIT 1",
    )
    .bind(crate::accounts::hash_token(&token))
    .bind(valid_hours as i32)
    .bind(crate::accounts::token_scopes::PASSWORD_RESET)
    .fetch_optional(&mut *tx)
    .await?;
    let user_id = match confirmable {
        Some(id) => {
            crate::session::server::set(
                &mut tx,
                &session_id,
                &format!("password-{token}"),
                &json!(id),
                crate::session::server::EXPIRY_SECONDS,
            )
            .await?;
            Some(id)
        }
        None => match crate::session::server::get(
            &mut tx,
            &session_id,
            &format!("password-{token}"),
        )
        .await?
        {
            Some(id) => sqlx::query_scalar(
                "SELECT t.user_id FROM email_tokens t WHERE t.user_id = $1 AND t.token_hash = $2 \
                     AND (t.scope = $3 OR t.scope IS NULL) AND t.confirmed AND NOT t.expired \
                     AND t.created_at >= now() - make_interval(hours => $4) LIMIT 1",
            )
            .bind(id.as_i64().unwrap_or(0) as i32)
            .bind(crate::accounts::hash_token(&token))
            .bind(crate::accounts::token_scopes::PASSWORD_RESET)
            .bind(valid_hours as i32)
            .fetch_optional(&mut *tx)
            .await?,
            None => None,
        },
    };
    let error = user_id.is_none().then(|| {
        state
            .i18n
            .t_with(
                "password_reset.no_token",
                &[(
                    "base_url",
                    &crate::url::Urls {
                        config: &state.config,
                        settings: &settings,
                    }
                    .base_url()
                    .unwrap_or_default(),
                )],
            )
            .unwrap_or_default()
    });
    let user = match user_id {
        Some(id) => {
            let (admin, email): (bool, Option<String>) = sqlx::query_as(
                "SELECT admin, (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") \
                 FROM users u WHERE id = $1",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            // UsernameCheckerService.is_developer?
            let is_developer = state
                .config
                .globals
                .get("developer_emails")
                .zip(email.as_deref())
                .is_some_and(|(list, email)| list.split(',').any(|d| d.trim() == email));
            let enabled = crate::second_factor::Enabled::load(&mut tx, &settings, id).await?;
            Some(ResetUser {
                id,
                is_developer,
                admin,
                enabled,
            })
        }
        None => None,
    };
    let security = match &user {
        Some(u) => {
            crate::webauthn::stage_challenge(&mut tx, &session_id, Some(u.id)).await?;
            let mut params = serde_json::Map::new();
            params.insert("is_developer".into(), json!(u.is_developer));
            params.insert("admin".into(), json!(u.admin));
            params.insert("second_factor_required".into(), json!(u.enabled.totp));
            params.insert(
                "security_key_required".into(),
                json!(u.enabled.security_keys),
            );
            params.insert("backup_enabled".into(), json!(u.enabled.backup));
            params.insert(
                "multiple_second_factor_methods".into(),
                json!(u.enabled.multiple()),
            );
            params.extend(
                crate::webauthn::allowed_credentials(&mut tx, &session_id, u.id, &u.enabled, false)
                    .await?,
            );
            Some(params)
        }
        None => None,
    };
    tx.commit().await?;
    // The page's token for the PUT, from the same session.
    let csrf_token = crate::session::csrf::mask(&session.csrf_token())
        .ok_or(Unsupported("session csrf token not decodable"))?;
    let session_cookie = session.set_cookie(&state, &settings)?;
    let with_session = |mut response: Response| -> Result<Response, AppError> {
        // expires_now
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store"),
        );
        if let Some(cookie) = &session_cookie {
            response
                .headers_mut()
                .append(header::SET_COOKIE, HeaderValue::from_str(cookie)?);
        }
        Ok(response)
    };
    let title = state
        .i18n
        .t("password_reset.title")
        .unwrap_or("Reset Password")
        .to_string();
    if json_format {
        let body = match (&error, security) {
            (Some(message), _) => json!({ "message": message }),
            (None, Some(params)) => serde_json::Value::Object(params),
            (None, None) => json!({}),
        };
        return with_session(Json(body).into_response());
    }
    let Some(security) = security else {
        let page = super::invite_redeem::error_page(
            &state,
            &settings,
            &guardian,
            &uri,
            error.unwrap_or_default(),
            &title,
            true,
        )
        .await?;
        return with_session(page);
    };
    let site = auth_chrome(&state, &settings, &guardian, &["password-reset-page"]).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;
    crawler.description = site.site_description.clone();
    let flag = |k: &str| {
        security
            .get(k)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    let t = |key: &str| site.chrome.t(key);
    let body = crate::login_page_view::password_reset_page(
        &crate::login_page_view::Cx {
            t: &t,
            mobile: crate::mobile::mobile_view(),
        },
        &crate::login_page_view::ResetView {
            totp: flag("second_factor_required"),
            security_key: flag("security_key_required"),
            backup: flag("backup_enabled"),
            other_method: flag("multiple_second_factor_methods"),
        },
    );
    let mut extra = serde_json::Map::new();
    extra.insert("token".into(), json!(token));
    extra.insert("password_reset".into(), serde_json::Value::Object(security));
    let data = login_data(&state, &settings, "password-reset", &csrf_token, extra)?;
    let page = AuthPage {
        crawler,
        title,
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        body,
        data,
        dialog_strings: dialog_strings(&state),
        head: "<meta name=\"referrer\" content=\"never\">".into(),
    };
    with_session(auth_response(page, None)?)
}

/// GET /u/confirm-email-token/:token(.json): users#confirm_email_token,
/// which the password reset page asks as it loads: the token confirmed
/// for a signup, if it is one.
pub async fn confirm_email_token(
    State(state): State<AppState>,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> Result<Response, AppError> {
    let token = token.strip_suffix(".json").unwrap_or(&token).to_string();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    super::accounts::confirm_email_token(
        &state,
        &mut conn,
        &settings,
        &token,
        crate::accounts::token_scopes::SIGNUP,
    )
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(json!({ "success": "OK" })),
    )
        .into_response())
}

/// GET /password-reset: the forgot-password route, which opens the
/// ForgotPassword modal over the login page when login is required and
/// over the homepage otherwise.
pub async fn show_forgot_password(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    if guardian.is_anonymous() && settings.get("login_required")?.truthy() {
        return login_page_with(&state, &settings, &headers, &uri, true).await;
    }
    Err(Unsupported("the forgot-password route's modal over the homepage").into())
}
