//! The preferences pages (UsersController#preferences and
//! UsersEmailController#index answer the Ember app, which draws
//! preferences.gjs and the tab's template over the profile), and the
//! endpoints they call that no other page does: users#my_redirect,
//! users#check_username and users#check_emails.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::users::ProfilePage;
use crate::html::Crawler;
use crate::preferences_view::{self as view, PrefTab};
use crate::session::current::{AuthGuardian, Incoming};
use crate::site_settings::SiteSettings;
use crate::topic_list_view::escape;
use crate::url::Urls;
use crate::user_profile_view::Tab;
use crate::users::{User, Users};
use crate::{AppError, AppState, Unsupported};

/// `UrlHelper.encode_component`: a path segment, percent-encoded.
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The request's own origin, as Rails' `redirect_to` writes a path.
fn origin(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .filter(|p| *p == "https")
        .unwrap_or("http");
    format!("{proto}://{host}")
}

fn found(location: String) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

/// GET /my/*path: users#my_redirect. The glob's last extension is its
/// format, not the path.
pub async fn my_redirect(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let path = match path.rsplit_once('.') {
        Some((p, ext)) if !ext.contains('/') => p.to_string(),
        _ => path,
    };
    let valid = !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-/".contains(&b));
    if !valid {
        return Ok(super::topics::not_found_response(&state));
    }
    let base = state.config.globals.relative_url_root();
    match guardian.user() {
        None => {
            let destination = format!("{base}/my/{path}");
            let cookie = format!(
                "destination_url={}; path=/; SameSite=Lax",
                encode_component(&destination)
            );
            let mut response = found(format!("{}{base}/login-preferences", origin(&headers)));
            response.headers_mut().insert(
                header::SET_COOKIE,
                header::HeaderValue::from_str(&cookie)
                    .map_err(|_| Unsupported("a destination cookie that is not a header"))?,
            );
            Ok(response)
        }
        Some(user) => Ok(found(format!(
            "{}{base}/u/{}/{path}",
            origin(&headers),
            encode_component(&user.username)
        ))),
    }
}

/// GET /.well-known/change-password: the routes' redirect to one's
/// security preferences.
pub async fn change_password(State(state): State<AppState>, headers: HeaderMap) -> Response {
    found(format!(
        "{}{}/my/preferences/security",
        origin(&headers),
        state.config.globals.relative_url_root()
    ))
}

#[derive(Deserialize, Default)]
pub struct CheckUsernameParams {
    username: Option<String>,
    email: Option<String>,
    for_user_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CheckEmailParams {
    email: Option<String>,
}

/// GET /u/check_email(.json): users#check_email, the signup page's check
/// of an address as its field is left.
pub async fn check_email(
    State(state): State<AppState>,
    Query(params): Query<CheckEmailParams>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
) -> Result<Response, AppError> {
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    if let Err(refused) =
        crate::rate_limiter::RateLimiter::new(None, &format!("check-email-{ip}"), 10, 60)
            .performed(&state.pool, state.config.rate_limits)
            .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // Email.downcase
    let email = params.email.unwrap_or_default().trim().to_lowercase();
    let success = || Ok(Json(json!({ "success": "OK" })).into_response());
    if email.is_empty() || settings.get("hide_email_address_taken")?.truthy() {
        return success();
    }
    let t = |key: &str| state.i18n.t(key).unwrap_or(key).to_string();
    // User.new.errors.full_message(:email, ...)
    let failed = |message: String| {
        Ok(
            Json(json!({ "failed": "FAILED", "errors": [format!("Email {message}")] }))
                .into_response(),
        )
    };
    if !crate::accounts::valid_email(&email) {
        return failed(t("user.email.invalid"));
    }
    if !crate::signup::email_allowed(&settings, &email)? {
        return failed(t("user.email.not_allowed"));
    }
    if crate::email_updater::should_block(&mut conn, &settings, &email).await? {
        return failed(t("user.email.blocked"));
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_emails e JOIN users u ON u.id = e.user_id \
         WHERE lower(e.email) = $1 AND NOT u.staged)",
    )
    .bind(&email)
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        return failed(t("errors.messages.taken"));
    }
    success()
}

/// GET /u/check_username(.json): users#check_username, the username
/// preference's (and signup's) availability check.
pub async fn check_username(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<CheckUsernameParams>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if guardian.is_anonymous()
        && (!settings.get("allow_new_registrations")?.truthy()
            || settings.get("enable_discourse_connect")?.truthy())
    {
        return Ok(super::search::invalid_access(&state));
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let available_true = |username: Option<&str>| -> Result<Response, AppError> {
        let mut out = json!({ "available": true });
        if let Some(name) = username {
            out["avatar_template"] = json!(
                crate::avatar::default_template(&settings, &urls, name)
                    .map_err(crate::users::UsersError::from)?
            );
        }
        Ok(Json(out).into_response())
    };
    // An advisory check: a throttled caller is told the name is free.
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    let limiter = crate::rate_limiter::RateLimiter::new(
        guardian.user().map(|u| crate::rate_limiter::LimitUser {
            id: u.id,
            staff: guardian.is_staff(),
        }),
        &format!("check-username-{ip}"),
        60,
        60,
    );
    if limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
        .is_err()
    {
        return available_true(None);
    }
    let Some(username) = params.username.filter(|u| !u.trim().is_empty()) else {
        if params.email.as_deref().is_none_or(|e| e.trim().is_empty()) {
            return Ok(super::accounts::param_missing("username"));
        }
        return Ok(Json(json!({ "success": "OK" })).into_response());
    };
    if !username.is_ascii() {
        return Err(Unsupported("unicode_normalize on a non-ASCII username (153)").into());
    }
    // user_from_params_or_current_user
    let target: Option<i32> = match params.for_user_id.as_deref() {
        None => guardian.user_id(),
        Some(id) if guardian.is_admin() => {
            let id = i32::try_from(crate::ruby::to_i(id)).unwrap_or(0);
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?;
            if !exists {
                return Ok(super::topics::not_found_response(&state));
            }
            Some(id)
        }
        Some(id) => {
            if guardian.user_id().map(i64::from) == Some(crate::ruby::to_i(id))
                && let Some(me) = guardian.user_id()
                && !crate::username_changer::can_edit_username(&mut conn, &settings, &guardian, me)
                    .await?
            {
                return Ok(super::search::invalid_access(&state));
            }
            guardian.user_id()
        }
    };
    let target_row: Option<(String, Option<String>)> = match target {
        Some(id) => {
            sqlx::query_as(
                "SELECT u.username, e.email FROM users u \
             LEFT JOIN user_emails e ON e.user_id = u.id AND e.primary \
             WHERE u.id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    // changing_case_of_own_username
    if let Some((current, _)) = &target_row
        && current.to_lowercase() == username.to_lowercase()
    {
        return available_true(Some(&username));
    }
    if let Some(error) = crate::signup::username_format_error(&settings, &state.i18n, &username)? {
        return Ok(Json(json!({ "errors": [error] })).into_response());
    }
    let email = match target_row.and_then(|(_, e)| e) {
        Some(e) => Some(e),
        None if !settings.get("hide_email_address_taken")?.truthy() => {
            params.email.filter(|e| !e.is_empty())
        }
        None => None,
    };
    let available = !crate::signup::clashing_with_existing_route(&username)?
        && super::groups::username_available_for(
            &mut conn,
            &settings,
            &username,
            email.as_deref(),
            guardian.is_admin(),
        )
        .await?;
    if available {
        return Ok(Json(json!({
            "available": true,
            "is_developer": false,
            "avatar_template": crate::avatar::default_template(&settings, &urls, &username).map_err(crate::users::UsersError::from)?,
        }))
        .into_response());
    }
    let suggestion = super::groups::suggest_username(&mut conn, &settings, &username).await?;
    Ok(Json(json!({ "available": false, "suggestion": suggestion })).into_response())
}

/// GET /u/:username/emails(.json): users#check_emails, the addresses
/// staff look up (logged as check_email unless one's own).
pub async fn check_emails(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    username: &str,
    context: Option<String>,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(user) = User::find_any(&mut conn, username).await? else {
        return Ok(super::topics::not_found_response(state));
    };
    if !guardian.is_me(user.id) {
        let can = guardian.is_admin()
            || (guardian.is_staff() && settings.get("moderators_view_emails")?.truthy());
        if !can {
            return Ok((StatusCode::FORBIDDEN, Json(json!({ "failed": "FAILED" }))).into_response());
        }
        // StaffActionLogger#log_check_email: an admin-only action.
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, context, \
                                         admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(crate::staff_action_logs::action_id("check_email"))
        .bind(guardian.user_id())
        .bind(user.id)
        .bind(context)
        .execute(&mut *conn)
        .await?;
    }
    let emails: Vec<String> = sqlx::query_scalar(
        "SELECT email FROM user_emails WHERE user_id = $1 ORDER BY \"primary\" DESC NULLS LAST",
    )
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    let unconfirmed: Vec<String> = sqlx::query_scalar(
        "SELECT new_email FROM email_change_requests WHERE user_id = $1 AND change_state <> 3",
    )
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    // associated_accounts: each authenticator's description for the user.
    let associated = crate::auth::associated_accounts(&mut conn, &state.i18n, user.id).await?;
    let mut emails = emails.into_iter();
    Ok(Json(json!({
        "email": emails.next(),
        "secondary_emails": emails.collect::<Vec<_>>(),
        "unconfirmed_emails": unconfirmed,
        "associated_accounts": associated,
    }))
    .into_response())
}

/// POST and DELETE /u/:username/*rest: the email endpoints, the
/// preference endpoints other items port refused; no other route.
pub async fn post_tail(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Path((username, rest)): Path<(String, String)>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    if matches!(
        rest.as_str(),
        "preferences/revoke-auth-token" | "preferences/revoke-auth-token.json"
    ) {
        return super::user_security::revoke_auth_token(
            state, &incoming, username, headers, uri, body,
        )
        .await;
    }
    if matches!(
        rest.as_str(),
        "preferences/revoke-account" | "preferences/revoke-account.json"
    ) {
        let p = crate::params::parse(uri.query(), &headers, &body);
        let form: Vec<(String, String)> = p
            .iter()
            .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
            .collect();
        if !super::session::csrf_ok(&state, &headers, &form, uri.path(), "POST") {
            return Ok(super::session::bad_csrf());
        }
        if incoming.guardian.is_anonymous() {
            return Ok(super::login_required::not_logged_in(&state));
        }
        return super::omniauth::revoke_account(&state, &incoming.guardian, &username, &p).await;
    }
    tail(
        "POST",
        state,
        incoming.guardian.clone(),
        username,
        rest,
        headers,
        peer,
        uri,
        body,
    )
    .await
}

pub async fn delete_tail(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((username, rest)): Path<(String, String)>,
    headers: HeaderMap,
    super::search::Peer(peer): super::search::Peer,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    tail(
        "DELETE", state, guardian, username, rest, headers, peer, uri, body,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn tail(
    method: &str,
    state: AppState,
    guardian: crate::guardian::Guardian,
    username: String,
    rest: String,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    if let Some(endpoint) = super::users_email::Endpoint::from_tail(method, &rest) {
        return super::users_email::handle(
            state, guardian, username, endpoint, headers, peer, uri, body,
        )
        .await;
    }
    if method == "DELETE"
        && matches!(
            rest.as_str(),
            "preferences/user_image" | "preferences/user_image.json"
        )
    {
        return super::user_avatar_writes::destroy_user_image(
            state, guardian, username, headers, uri, body,
        )
        .await;
    }
    Ok(super::topics::not_found_response(&state))
}

/// `/u/:username/preferences(/<tab>)(.json)`: what the tab path names.
pub enum Route {
    /// `/preferences`: PreferencesIndex redirects to the account tab.
    Index,
    Tab(PrefTab),
    /// A preferences route the Ember app has no route for
    /// (badge_title, username): its not-found page.
    EmberUnknown,
}

/// The tail after `/u/:username/`, for the preferences routes Rails
/// declares; None for one it does not (Rails' 404).
pub fn route(rest: &str) -> Option<Route> {
    let tail = rest.strip_prefix("preferences")?;
    match tail.trim_end_matches('/') {
        "" => Some(Route::Index),
        t => {
            let tab = t.strip_prefix('/')?;
            match tab {
                "badge_title" | "username" => Some(Route::EmberUnknown),
                tab => PrefTab::from_path(tab).map(Route::Tab),
            }
        }
    }
}

/// The JSON twin of a preferences route: users#preferences and
/// users_email#index answer nothing (logged in only).
pub fn json(state: &AppState, guardian: &crate::guardian::Guardian, route: &Route) -> Response {
    if guardian.is_anonymous() {
        return super::login_required::not_logged_in(state);
    }
    match route {
        Route::Tab(PrefTab::Email) => StatusCode::NO_CONTENT.into_response(),
        _ => StatusCode::OK.into_response(),
    }
}

/// The HTML of a preferences route: the profile (users#show, its view
/// tracked as Ember's user route does) with the tab drawn under the
/// preferences navigation. One who may not edit the user goes to their
/// activity (RestrictedUserRoute); chat's tab without chat goes home,
/// the second factor page without a confirmed session to the security
/// tab.
pub async fn page(
    state: AppState,
    incoming: Incoming,
    username: String,
    route: Route,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: Uri,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let auth_token: Option<String> = incoming
        .session
        .as_ref()
        .map(|s| s.token.auth_token.clone());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let noindex = [("x-robots-tag", "noindex")];
    let Some(user) = User::find_active(&mut conn, &username).await? else {
        return Ok((noindex, super::topics::not_found_response(&state)).into_response());
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let username_lower = user.username.to_lowercase();
    let user_path = format!("{base_path}/u/{}", escape(&username_lower));
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut users = Users {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        base_path: &base_path,
        auth_token: auth_token.as_deref(),
    };
    let show = users.show(&user).await?;
    if !guardian.is_me(user.id) {
        let ip = crate::remote_ip::remote_ip(&headers, peer)?;
        users.track_view(&user, &ip).await?;
    }
    if show["user"]["can_edit"] != true {
        return Ok(found(format!("{}{user_path}/activity", origin(&headers))));
    }
    let tab = match route {
        Route::Index => {
            return Ok(found(format!(
                "{}{user_path}/preferences/account",
                origin(&headers)
            )));
        }
        Route::EmberUnknown => {
            return ember_not_found(
                &state,
                &mut conn,
                &settings,
                &guardian,
                &headers,
                bus_position,
            )
            .await;
        }
        Route::Tab(tab) => tab,
    };
    // PreferencesSecondFactor: the viewer's second factors once the
    // session is trusted, else back to the security tab.
    let second_factors = if tab == PrefTab::SecondFactor {
        let viewer = guardian
            .user_id()
            .ok_or(Unsupported("a logged-in viewer without a session"))?;
        if settings.get("enable_discourse_connect")?.truthy()
            || !settings.get("enable_local_logins")?.truthy()
        {
            return Err(Unsupported("the second factor page without local logins").into());
        }
        let mut session = crate::session::forum::load_forum_session(&state, &headers);
        let session_id = session.server_session_id();
        if !super::user_security::session_is_trusted(&mut conn, &session_id, viewer).await? {
            return Ok(found(format!(
                "{}{user_path}/preferences/security",
                origin(&headers)
            )));
        }
        Some(super::user_security::second_factors(&mut conn, viewer).await?)
    } else {
        None
    };

    let session_user = incoming
        .session
        .as_ref()
        .map(|s| &s.user)
        .ok_or(Unsupported("a logged-in viewer without a session"))?;
    let current_user = crate::current_user::serialize(
        &mut conn,
        &state,
        &settings,
        session_user,
        incoming
            .session
            .as_ref()
            .and_then(|s| s.token.authenticated_with_oauth)
            .unwrap_or(false),
    )
    .await?;
    // UserRoute#model: one's own profile is the current user, with
    // findDetails' users#show set over it.
    let mut model = if guardian.is_me(user.id) {
        current_user.clone()
    } else {
        json!({})
    };
    if let (Value::Object(m), Value::Object(shown)) = (&mut model, &show["user"]) {
        for (k, v) in shown {
            m.insert(k.clone(), v.clone());
        }
    }
    let u = &model;
    let site_doc = crate::site::Site {
        conn: &mut conn,
        config: &state.config,
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        guardian: guardian.clone(),
    }
    .preferences_json()
    .await
    .map_err(AppError::from)?;
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let p = view::Prefs {
        cx: &cx,
        s: &settings,
        u,
        user_path: &user_path,
        viewer_id: guardian.user_id(),
        viewer_admin: guardian.is_admin(),
        viewer_staff: guardian.is_staff(),
    };
    let chat_enabled = crate::plugins::chat::enabled(&settings)?;
    let chat_tab = chat_enabled && (u["can_chat"] == true || guardian.is_admin());
    // PreferencesChatRoute: without chat, the homepage.
    if tab == PrefTab::Chat && !chat_tab {
        return Ok(found(format!("{}{base_path}/", origin(&headers))));
    }
    let (content, attrs) = match tab {
        PrefTab::Account => {
            // UserBadge.findByUsername: the title-granting badges' names.
            let mut titles: Vec<String> = u["groups"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| g["title"].as_str().filter(|t| !t.is_empty()))
                .map(str::to_string)
                .collect();
            if settings.get("enable_badges")?.truthy() {
                let doc = super::badges::username_doc(
                    &state, &mut conn, &settings, &guardian, user.id, false,
                )
                .await?;
                for badge in doc["badges"].as_array().into_iter().flatten() {
                    if badge["allow_title"] == true
                        && let Some(name) = badge["name"].as_str()
                    {
                        titles.push(name.to_string());
                    }
                }
            }
            let mut unique: Vec<String> = Vec::new();
            for title in titles {
                if !unique.contains(&title) {
                    unique.push(title);
                }
            }
            unique.sort();
            // selectable_avatars as the client setting holds it: the
            // uploads' urls, in the setting's order.
            let mut selectable_avatars = Vec::new();
            for id in settings.get("selectable_avatars")?.to_s().split('|') {
                let id = i32::try_from(crate::ruby::to_i(id)).unwrap_or(0);
                if let Some(url) =
                    sqlx::query_scalar::<_, String>("SELECT url FROM uploads WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&mut *conn)
                        .await?
                {
                    selectable_avatars.push(url);
                }
            }
            let site_logo_small_url =
                crate::admin_users::logo_small_url(&mut conn, &settings).await?;
            view::account(
                &p,
                &view::Account {
                    titles: unique,
                    viewer_can_delete_account: current_user["can_delete_account"] == true,
                    viewer_can_upload_avatar: current_user["can_upload_avatar"] == true,
                    selectable_avatars,
                    site_logo_small_url,
                },
            )?
        }
        PrefTab::Profile => {
            let fields: Vec<Value> = site_doc["user_fields"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            view::profile::render(
                &p,
                &view::profile::Profile {
                    site_user_fields: &fields,
                    needs_required_fields_check: current_user["needs_required_fields_check"]
                        == true,
                    moment: (
                        crate::assets::url(&base_path, "moment.min.js"),
                        crate::assets::url(
                            &base_path,
                            "moment-timezone-with-data-10-year-range.min.js",
                        ),
                    ),
                },
            )?
        }
        PrefTab::Notifications => {
            let solved = crate::plugins::solved::enabled(&settings)?
                && (settings.get("allow_solved_on_all_topics")?.truthy()
                    || !settings.get("enable_solved_tags")?.to_s().is_empty()
                    || sqlx::query_scalar::<_, bool>(
                        "SELECT EXISTS (SELECT 1 FROM category_custom_fields \
                         WHERE name = 'enable_accepted_answers' AND value = 'true')",
                    )
                    .fetch_one(&mut *conn)
                    .await?);
            view::notifications::render(
                &p,
                &view::notifications::Notifications {
                    solved,
                    chat: chat_enabled && u["user_option"]["chat_enabled"] == true,
                    chat_enabled,
                },
            )?
        }
        PrefTab::Interface => {
            // The session's palettes and the color mode, as the viewer's
            // page draws them (crate::color_mode).
            let options: Option<(Option<i32>, Option<i32>, i32)> = match guardian.user() {
                Some(viewer) => {
                    sqlx::query_as(
                        "SELECT color_scheme_id, dark_scheme_id, interface_color_mode \
                         FROM user_options WHERE user_id = $1",
                    )
                    .bind(viewer.id)
                    .fetch_optional(&mut *conn)
                    .await?
                }
                None => None,
            };
            let options = options.map(|(color_scheme_id, dark_scheme_id, interface_color_mode)| {
                crate::color_mode::Options {
                    color_scheme_id,
                    dark_scheme_id,
                    interface_color_mode,
                }
            });
            let head = crate::color_mode::head(
                &mut conn,
                &settings,
                &state.i18n,
                &base_path,
                &crate::color_mode::cookies(),
                options.as_ref(),
            )
            .await?;
            view::interface::render(
                &p,
                &view::interface::Interface {
                    user_themes: &site_doc["user_themes"],
                    user_color_schemes: &site_doc["user_color_schemes"],
                    text_size_cookie: crate::session::current::cookie(&headers, "text_size"),
                    user_tips: site_doc["user_tips"] == true,
                    default_light_color_scheme: &site_doc["default_light_color_scheme"],
                    default_dark_color_scheme: &site_doc["default_dark_color_scheme"],
                    session_scheme_id: head.user_scheme_id,
                    session_dark_scheme_id: head.user_dark_scheme_id,
                    color_mode: head.selector_mode,
                },
            )?
        }
        PrefTab::Emails => view::emails::render(
            &p,
            &view::emails::Emails {
                chat_enabled,
                policy_enabled: crate::plugins::policy::enabled(&settings)?,
            },
        )?,
        PrefTab::Email => {
            let new = crate::params::parse_query(uri.query().unwrap_or_default())
                .get("new")
                .and_then(crate::params::scalar)
                .is_some_and(|v| !v.is_empty());
            (
                view::email::render(&p, new, guardian.is_admin()),
                serde_json::Map::new(),
            )
        }
        PrefTab::Tracking => {
            // Category.list: the viewer's site categories, with their
            // topic counts for the rows.
            let allowed = guardian.allowed_category_ids(&mut conn, &settings).await?;
            let rows: Vec<(i32, i32)> =
                sqlx::query_as("SELECT id, topic_count FROM categories WHERE id = ANY($1)")
                    .bind(&allowed)
                    .fetch_all(&mut *conn)
                    .await?;
            let chooser: Vec<crate::composer_view::ChooserCategory> = rows
                .into_iter()
                .map(|(id, topic_count)| crate::composer_view::ChooserCategory {
                    id: i64::from(id),
                    topic_count: i64::from(topic_count),
                    description_text: None,
                    minimum_required_tags: 0,
                })
                .collect();
            view::tracking::render(
                &p,
                &view::tracking::SiteCategories { allowed: &chooser },
                guardian.is_admin(),
            )?
        }
        PrefTab::Tags => view::tracking::render_tags(&p)?,
        PrefTab::Users => view::users::render(&p)?,
        PrefTab::NavigationMenu => view::navigation_menu::render(&p),
        PrefTab::Security => {
            let local = !settings.get("enable_discourse_connect")?.truthy()
                && settings.get("enable_local_logins")?.truthy();
            let can_check_emails = guardian.is_me(user.id)
                || guardian.is_admin()
                || (settings.get("moderators_view_emails")?.truthy() && guardian.is_staff());
            (
                view::security::render_security(
                    &p,
                    &view::security::Security {
                        passkeys: local && settings.get("enable_passkeys")?.truthy(),
                        can_check_emails,
                    },
                )?,
                serde_json::Map::new(),
            )
        }
        PrefTab::Apps => (view::security::render_apps(&p), serde_json::Map::new()),
        PrefTab::CalendarSubscriptions => {
            let viewer = guardian
                .user_id()
                .ok_or(Unsupported("a logged-in viewer without a session"))?;
            let has = super::user_security::has_calendar_subscription(&mut conn, viewer).await?;
            (
                view::security::render_calendar(&p, has),
                serde_json::Map::new(),
            )
        }
        PrefTab::Chat => (view::security::render_chat(&p)?, serde_json::Map::new()),
        PrefTab::SecondFactor => {
            let lists = second_factors
                .as_ref()
                .ok_or(Unsupported("the second factor page without its lists"))?;
            (
                view::security::render_second_factor(
                    &p,
                    &view::security::SecondFactorPage {
                        totps: &lists.totps,
                        security_keys: &lists.security_keys,
                    },
                ),
                serde_json::Map::new(),
            )
        }
    };
    // The security pages' dialog strings.
    let content = if matches!(
        tab,
        PrefTab::Security | PrefTab::Apps | PrefTab::CalendarSubscriptions | PrefTab::SecondFactor
    ) {
        let viewer_username = current_user["username"].as_str().unwrap_or_default();
        format!(
            "{content}{}",
            view::security::strings(
                &cx,
                viewer_username,
                &format!("{base_path}/u/{}", escape(&viewer_username.to_lowercase())),
                guardian.is_me(user.id)
                    && settings.get("enable_passkeys")?.truthy()
                    && u["user_passkeys"].as_array().is_some_and(|a| !a.is_empty()),
            )
        )
    } else {
        content
    };
    // The choosers' strings, where there are choosers.
    let content = if matches!(tab, PrefTab::Tracking | PrefTab::Tags | PrefTab::Users) {
        let moment = (
            crate::assets::url(&base_path, "moment.min.js"),
            crate::assets::url(&base_path, "moment-timezone-with-data-10-year-range.min.js"),
        );
        format!(
            "{content}{}",
            view::users::choosers_strings(
                &cx,
                &settings,
                current_user["username"].as_str().unwrap_or_default(),
                current_user["user_option"]["timezone"].as_str(),
                (&moment.0, &moment.1),
            )?
        )
    } else {
        content
    };
    let main = view::shell(&p, tab, chat_tab, &view::form_attrs(&p, &attrs), &content);

    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    site.bus_position = bus_position;
    site.chrome.set_search_context("user", &username_lower);
    let mut page = super::users::profile_page(
        &mut conn,
        site,
        &state.i18n,
        &guardian,
        &user,
        &show,
        &Tab::Preferences(main),
        &settings,
    )
    .await?;
    page.chrome.body_classes.push_str(" user-preferences-page");
    if tab == PrefTab::Tracking {
        page.chrome
            .body_classes
            .push_str(" user-preferences-tracking-page");
    }
    // Preferences#titleToken over the user route's username.
    // The email, tags, apps, calendar, second factor and chat controllers
    // have no subpageTitle.
    page.page_title = if matches!(
        tab,
        PrefTab::Email
            | PrefTab::Tags
            | PrefTab::Apps
            | PrefTab::CalendarSubscriptions
            | PrefTab::Chat
            | PrefTab::SecondFactor
    ) {
        format!(
            "{} - {}",
            crate::topic_list_view::t(&cx, "user.preferences.title"),
            user.username
        )
    } else {
        format!(
            "{} - {} - {}",
            crate::topic_list_view::t(&cx, tab.title_key()),
            crate::topic_list_view::t(&cx, "user.preferences.title"),
            user.username
        )
    };
    let avatar = u["avatar_template"]
        .as_str()
        .map(|t| urls.absolute(&t.replace("{size}", "45")))
        .transpose()?;
    let mut crawler = Crawler::for_request(&urls, &uri, None)?.with_meta(
        urls.settings.emojis(),
        &user.username,
        "",
        avatar,
    );
    crawler.description = settings.get("site_description")?.to_s();
    page.crawler = crawler;
    let response = (
        noindex,
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// Ember's not-found page in the application layout, for a route the app
/// lacks.
pub(super) async fn ember_not_found(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    bus_position: String,
) -> Result<Response, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let not_found = crate::not_found_page::NotFound {
        forbidden: false,
        custom_message: None,
        slug: "",
        guardian,
    };
    let html =
        crate::not_found_page::build(&mut *conn, settings, &state.i18n, &urls, &not_found).await?;
    let vs = super::session::viewer_state(state, headers, settings, guardian)?;
    let mut site = crate::html::Site::from_settings(settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut *conn,
        state,
        settings,
        guardian,
        crate::sidebar::Active::None,
    )
    .await?;
    site.bus_position = bus_position;
    let page = ProfilePage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::default(),
        page_title: String::new(),
        main: format!("<div class=\"container\">{html}</div>"),
    };
    let response = Html(page.render().map_err(crate::html::HtmlError::from)?).into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}
