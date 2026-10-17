//! `ApplicationController#redirect_to_profile_if_required`: once a user
//! field required for all users exists (a newer
//! `user_required_fields_versions` row than the user's
//! `required_fields_version`), a logged-in request that isn't JSON goes to
//! the user's profile preferences until the required fields are filled
//! in; a user who already has them only gets their version bumped.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::session::current::Incoming;
use crate::{AppError, AppState};

/// `UserHistory.actions[:redirected_to_required_fields]`
const REDIRECTED_TO_REQUIRED_FIELDS: i32 = 108;

/// The actions that `skip_before_action :redirect_to_profile_if_required`,
/// by path (base path removed), plus what nginx serves itself.
fn skipped(path: &str, method: &axum::http::Method) -> bool {
    let get = method == axum::http::Method::GET || method == axum::http::Method::HEAD;
    const PREFIXES: [&str; 27] = [
        // SessionController
        "/session",
        // StaticController, UploadsController, StylesheetsController,
        // ThemeJavascriptsController, SvgSpriteController,
        // ExtraLocalesController, HighlightJsController, UserAvatarsController
        "/assets/",
        "/fonts/",
        "/images/",
        "/uploads/",
        "/secure-uploads/",
        "/secure-media-uploads/",
        "/stylesheets/",
        "/theme-javascripts/",
        "/svg-sprite/",
        "/extra-locales/",
        "/highlight-js/",
        "/user_avatar/",
        "/letter_avatar/",
        "/letter_avatar_proxy/",
        "/favicon",
        "/service-worker",
        // EmailController
        "/email/",
        // InvitesController
        "/invites",
        // PresenceController
        "/presence/",
        // UserApiKeysController's pages, UsersEmailController
        "/user-api-key",
        "/u/confirm-new-email",
        "/u/confirm-old-email",
        "/u/authorize-email",
        // PublishedPagesController#show, Users::OmniauthCallbacksController
        "/pub/",
        "/auth/",
        // FinishInstallationController
        "/finish-installation",
    ];
    if PREFIXES.iter().any(|p| path.starts_with(p)) {
        return true;
    }
    if matches!(
        path,
        "/srv/status"
            | "/robots.txt"
            | "/robots-builder.json"
            | "/site/basic-info"
            | "/site/basic-info.json"
            | "/site/statistics"
            | "/site/statistics.json"
            | "/offline.html"
            | "/manifest.webmanifest"
            | "/manifest.json"
            | "/opensearch.xml"
            | "/login"
            | "/signup"
            | "/password-reset"
            | "/faq"
            | "/guidelines"
            | "/tos"
            | "/privacy"
            | "/rules"
            | "/conduct"
            | "/llms.txt"
    ) {
        return true;
    }
    // UsersController: show (/u/:username), staff_info, update, and the
    // checks open to anyone.
    if let Some(rest) = path.strip_prefix("/u/") {
        let rest = rest.strip_suffix(".json").unwrap_or(rest);
        let segments: Vec<&str> = rest.split('/').collect();
        if segments.len() == 1 && get {
            return true;
        }
        if segments.len() == 1 && method == axum::http::Method::PUT {
            return true;
        }
        if segments.len() == 2 && segments[1] == "staff-info" {
            return true;
        }
        if matches!(
            segments[0],
            "check_username"
                | "check_email"
                | "hp"
                | "password-reset"
                | "activate-account"
                | "account-created"
                | "confirm-admin"
                | "email-login"
                | "admin-login"
        ) {
            return true;
        }
    }
    matches!(path, "/u" | "/u.json") && method == axum::http::Method::POST
}

/// UsersController's `skip_before_action :redirect_to_profile_if_required,
/// only: %i[show staff_info update]`, which leave the login redirect on.
fn profile_redirect_only(path: &str, method: &axum::http::Method) -> bool {
    let Some(rest) = path.strip_prefix("/u/") else {
        return false;
    };
    let rest = rest.strip_suffix(".json").unwrap_or(rest);
    let segments: Vec<&str> = rest.split('/').collect();
    // The actions that skip both.
    if matches!(
        segments[0],
        "check_username"
            | "check_email"
            | "hp"
            | "password-reset"
            | "activate-account"
            | "account-created"
            | "confirm-admin"
            | "email-login"
            | "admin-login"
            | "confirm-email-token"
            | "generate-random-username"
    ) {
        return false;
    }
    match segments.as_slice() {
        [_] => matches!(
            *method,
            axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::PUT
        ),
        [_, "staff-info"] => true,
        _ => false,
    }
}

/// `should_enforce_2fa?`: enforce_second_factor names the user (staff, or
/// everyone) and they have neither an authenticator nor a security key;
/// not for an API request, an anonymous-mode user, or (unless
/// enforce_second_factor_on_external_auth) an external login. The
/// redirect to the second factor page, unless the request is for it.
async fn enforce_second_factor(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    incoming: &Incoming,
    fullpath: &str,
    host: &str,
    user_id: i32,
    staff: bool,
) -> Result<Option<Response>, AppError> {
    let settings = crate::site_settings::SiteSettings::load(
        conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let enforce = settings.get("enforce_second_factor")?.to_s();
    if !(enforce == "all" || (enforce == "staff" && staff)) {
        return Ok(None);
    }
    // is_api?: no session cookie behind the user.
    let Some(session) = &incoming.session else {
        return Ok(None);
    };
    if incoming.guardian.is_anonymous() {
        return Ok(None);
    }
    if !settings
        .get("enforce_second_factor_on_external_auth")?
        .truthy()
        && session.token.authenticated_with_oauth.unwrap_or(false)
    {
        return Ok(None);
    }
    let enabled = crate::second_factor::Enabled::load(&mut *conn, &settings, user_id).await?;
    if enabled.has_any() {
        return Ok(None);
    }
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !username.is_ascii() {
        return Err(crate::Unsupported("encoded_username for a non-ASCII username").into());
    }
    let base_path = state.config.globals.relative_url_root();
    let redirect_path = format!("{base_path}/u/{username}/preferences/second-factor");
    if fullpath.starts_with(&redirect_path) {
        return Ok(None);
    }
    Ok(Some(
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("http://{host}{redirect_path}"))],
            Body::empty(),
        )
            .into_response(),
    ))
}

pub async fn gate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let Some(user_id) = request
        .extensions()
        .get::<Incoming>()
        .and_then(|i| i.guardian.user_id())
    else {
        return Ok(next.run(request).await);
    };
    let staff = request
        .extensions()
        .get::<Incoming>()
        .is_some_and(|i| i.guardian.is_staff());
    let base_path = state.config.globals.relative_url_root().to_string();
    let full_path = request.uri().path().to_string();
    let path = full_path
        .strip_prefix(base_path.as_str())
        .unwrap_or(&full_path)
        .to_string();
    // return if request.format.json?
    if path.ends_with(".json") || super::wants_json(request.uri().query(), request.headers()) {
        return Ok(next.run(request).await);
    }
    let skip = skipped(&path, request.method());
    // redirect_to_login_if_required's tail, which runs first:
    // should_enforce_2fa? sends the user to their second factor page.
    // UsersController#show, #staff_info and #update skip only the profile
    // redirect.
    if !skip || profile_redirect_only(&path, request.method()) {
        let incoming = request.extensions().get::<Incoming>().cloned();
        let fullpath = request
            .uri()
            .path_and_query()
            .map(|p| p.as_str().to_string())
            .unwrap_or_default();
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost")
            .to_string();
        let mut conn = state.pool.acquire().await?;
        let redirect = match &incoming {
            Some(incoming) => {
                enforce_second_factor(
                    &state, &mut conn, incoming, &fullpath, &host, user_id, staff,
                )
                .await?
            }
            None => None,
        };
        if let Some(redirect) = redirect {
            return Ok(redirect);
        }
    }
    if skip {
        return Ok(next.run(request).await);
    }
    let mut conn = state.pool.acquire().await?;
    if !crate::user_fields::needs_required_fields_check(&mut conn, user_id).await? {
        drop(conn);
        return Ok(next.run(request).await);
    }
    // populated_required_custom_fields?: then bump_required_fields_version.
    let required: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM user_fields WHERE requirement = $1")
            .bind(crate::user_fields::FOR_ALL_USERS)
            .fetch_all(&mut *conn)
            .await?;
    let mut populated = true;
    for id in required {
        let value = crate::user_fields::custom_field_value(&mut conn, user_id, id).await?;
        if !crate::user_fields::present(&value) {
            populated = false;
            break;
        }
    }
    if populated {
        let current = crate::user_fields::current_required_version(&mut conn).await?;
        sqlx::query(
            "UPDATE users SET required_fields_version = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(user_id)
        .bind(current as i32)
        .execute(&mut *conn)
        .await?;
        drop(conn);
        return Ok(next.run(request).await);
    }
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    // encoded_username: plain ASCII usernames are left as they are.
    if !username.is_ascii() {
        return Err(crate::Unsupported("encoded_username for a non-ASCII username").into());
    }
    let encoded = username;
    let redirect_path = format!("{base_path}/u/{encoded}/preferences/profile");
    let second_factor_path = format!("{base_path}/u/{encoded}/preferences/second-factor");
    let fullpath = request
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| full_path.clone());
    let allowed = [
        redirect_path.clone(),
        second_factor_path,
        format!("{base_path}/admin"),
        format!("{base_path}/safe-mode"),
    ];
    if allowed.iter().any(|p| fullpath.starts_with(p.as_str())) {
        drop(conn);
        return Ok(next.run(request).await);
    }
    // RateLimiter.new(current_user, "redirect_to_required_fields_log", 1,
    // 24.hours): the log once a day.
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser { id: user_id, staff }),
        "redirect_to_required_fields_log",
        1,
        24 * 3600,
    );
    if limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
        .is_ok()
    {
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(REDIRECTED_TO_REQUIRED_FIELDS)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }
    drop(conn);
    // cookies[:destination_url] = request.original_url
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost")
        .to_string();
    let original_url = format!("http://{host}{fullpath}");
    let cookie = format!(
        "destination_url={}; path=/; SameSite=Lax",
        form_urlencoded::byte_serialize(original_url.as_bytes()).collect::<String>()
    );
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, format!("http://{host}{redirect_path}")),
            (header::SET_COOKIE, cookie),
        ],
        Body::empty(),
    )
        .into_response())
}
