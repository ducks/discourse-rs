//! FinishInstallationController: the pages a new site shows while it has
//! no admin (`has_login_hint`, set by db:seed's 993_login_hint.rb): index
//! (also the front page meanwhile, HomePageConstraint), register, whose
//! email chooser lists DISCOURSE_DEVELOPER_EMAILS, confirm-email and
//! resend-email. Rails draws them itself (the finish_installation layout),
//! not Ember. The registered account is activated by its signup email,
//! sent through the job queue's mailer, and becomes an admin when it logs
//! in (make_developer_admin, crate::roles).
//!
//! Not ported: DISCOURSE_SKIP_EMAIL_SETUP's Discourse ID setup
//! (`setup_discourse_id`, `redirect_discourse_id`).

use askama::Template;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::session::cookie::Scalar;
use crate::session::current::AuthGuardian;
use crate::session::forum::{ForumSession, load_forum_session};
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// layouts/finish_installation.html.erb's head.
struct Head {
    base_path: String,
    base_url: String,
    site_title: String,
    title: String,
    version: &'static str,
    commit: &'static str,
    color_links: String,
    color_meta: String,
    favicon_url: String,
    favicon_type: &'static str,
    apple_touch_icon_url: String,
    canonical: String,
    can_search: bool,
}

#[derive(Template)]
#[template(path = "finish_installation_index.html")]
struct IndexPage {
    head: Head,
    congratulations: String,
    help: String,
    button: String,
}

#[derive(Template)]
#[template(path = "finish_installation_register.html")]
struct RegisterPage {
    head: Head,
    title: String,
    allowed_emails: Vec<String>,
    selected_email: Option<String>,
    csrf_token: String,
    email_label: String,
    username_label: String,
    username_instructions: String,
    username: Option<String>,
    username_errors: Vec<String>,
    password_label: String,
    password_instructions: String,
    password: Option<String>,
    password_errors: Vec<String>,
    button: String,
    no_emails: String,
}

#[derive(Template)]
#[template(path = "finish_installation_confirm_email.html")]
struct ConfirmEmailPage {
    head: Head,
    title: String,
    message: String,
    button: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "finish_installation_resend_email.html")]
struct ResendEmailPage {
    head: Head,
    title: String,
    message: String,
}

/// The session key confirm-email and resend-email read.
const REGISTERED_EMAIL: &str = "registered_email";

fn t(state: &AppState, key: &str) -> String {
    state.i18n.t(key).unwrap_or(key).to_string()
}

async fn head(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    uri: &Uri,
) -> Result<Head, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let base_path = state.config.globals.relative_url_root().to_string();
    let color = crate::color_mode::head(
        &mut *conn,
        settings,
        &state.i18n,
        &base_path,
        &crate::color_mode::cookies(),
        None,
    )
    .await?;
    let favicon_url = crate::site_icons::site_url(&mut *conn, &urls, "favicon").await?;
    let apple_touch_icon_url =
        crate::site_icons::site_url(&mut *conn, &urls, "apple_touch_icon").await?;
    Ok(Head {
        base_url: urls.base_url()?,
        // canonical_link_tag: the request's url without its query.
        canonical: format!("{}{}", urls.base_url_no_prefix()?, uri.path()),
        site_title: settings.get("title")?.to_s(),
        title: t(state, "wizard.title"),
        version: crate::schema::discourse_version(),
        commit: crate::schema::discourse_commit(),
        color_links: color.links,
        color_meta: color.meta,
        favicon_type: crate::site_icons::mime_type(&favicon_url),
        favicon_url,
        apple_touch_icon_url,
        can_search: guardian.can_search(settings)?,
        base_path,
    })
}

fn page(html: Result<String, askama::Error>, cookie: Option<String>) -> Result<Response, AppError> {
    let html = html.map_err(crate::html::HtmlError::from)?;
    let mut response = Html(html).into_response();
    if let Some(cookie) = cookie {
        response
            .headers_mut()
            .append(header::SET_COOKIE, cookie.parse()?);
    }
    Ok(response)
}

/// `ensure_no_admins`: Discourse::InvalidAccess once the login hint is
/// off, the forbidden page for a page request.
fn forbidden(state: &AppState) -> Response {
    let mut response = super::search::invalid_access(state);
    response
        .extensions_mut()
        .insert(super::not_found::ErrorPage {
            status: StatusCode::FORBIDDEN,
            forbidden: true,
            custom_message: None,
        });
    response
}

async fn load_settings(
    state: &AppState,
    conn: &mut PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `find_allowed_emails`: DISCOURSE_DEVELOPER_EMAILS split and stripped
/// (not downcased).
fn allowed_emails(state: &AppState) -> Vec<String> {
    state
        .config
        .globals
        .get("developer_emails")
        .map(|list| list.split(',').map(|e| e.trim().to_string()).collect())
        .unwrap_or_default()
}

/// GET /finish-installation, and the front page while the login hint is on.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    index_page(&state, &guardian, &uri).await
}

pub(super) async fn index_page(
    state: &AppState,
    guardian: &Guardian,
    uri: &Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(state, &mut conn).await?;
    if !settings.get("has_login_hint")?.truthy() {
        return Ok(forbidden(state));
    }
    if std::env::var("DISCOURSE_SKIP_EMAIL_SETUP").as_deref() == Ok("1") {
        return Err(Unsupported("the finish installation page's Discourse ID setup").into());
    }
    let html = IndexPage {
        head: head(state, &mut conn, &settings, guardian, uri).await?,
        congratulations: t(state, "finish_installation.congratulations"),
        help: t(state, "finish_installation.register.help"),
        button: t(state, "finish_installation.register.button"),
    }
    .render();
    page(html, None)
}

/// GET /finish-installation/redirect-discourse-id
pub async fn redirect_discourse_id() -> Result<Response, AppError> {
    Err(Unsupported("the finish installation page's Discourse ID setup").into())
}

/// What the register form shows: the values given and the errors.
#[derive(Default)]
struct Form {
    email: Option<String>,
    username: Option<String>,
    password: Option<String>,
    username_errors: Vec<String>,
    password_errors: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
async fn register_page(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    allowed_emails: Vec<String>,
    form: Form,
) -> Result<Response, AppError> {
    let mut session = load_forum_session(state, headers);
    let csrf_token = masked_csrf(&mut session)?;
    let min_admin = settings.get("min_admin_password_length")?.to_i();
    let html = RegisterPage {
        head: head(state, conn, settings, guardian, uri).await?,
        title: t(state, "finish_installation.register.title"),
        allowed_emails,
        selected_email: form.email,
        csrf_token,
        email_label: t(state, "js.user.email.title"),
        username_label: t(state, "js.user.username.title"),
        username_instructions: t(state, "js.user.username.instructions"),
        username: form.username,
        username_errors: form.username_errors,
        password_label: t(state, "js.user.password.title"),
        password_instructions: state
            .i18n
            .t_count(
                "js.user.password.instructions",
                min_admin,
                &[("count", &min_admin.to_string())],
            )
            .unwrap_or_default(),
        password: form.password,
        password_errors: form.password_errors,
        button: t(state, "finish_installation.register.button"),
        no_emails: t(state, "finish_installation.register.no_emails"),
    }
    .render();
    page(html, session.set_cookie(state, settings)?)
}

fn masked_csrf(session: &mut ForumSession) -> Result<String, AppError> {
    let stored = session.csrf_token();
    Ok(crate::session::csrf::mask(&stored)
        .ok_or(Unsupported("session csrf token not decodable"))?)
}

/// GET /finish-installation/register
pub async fn register_form(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if !settings.get("has_login_hint")?.truthy() {
        return Ok(forbidden(&state));
    }
    // The form shows the query's values as a POST's.
    let p = uri.query().map(params::parse_query).unwrap_or_default();
    let form = Form {
        email: params::string(&p, "email"),
        username: params::string(&p, "username"),
        password: params::string(&p, "password"),
        ..Form::default()
    };
    let allowed = allowed_emails(&state);
    register_page(
        &state, &mut conn, &settings, &guardian, &headers, &uri, allowed, form,
    )
    .await
}

/// `send_signup_email`: a signup token and its email (critical_user_email),
/// unless the account is active with its email confirmed.
async fn send_signup_email(conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
    let (active, email): (bool, String) = sqlx::query_as(
        "SELECT u.active, e.email FROM users u JOIN user_emails e ON e.user_id = u.id AND e.\"primary\" \
         WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    // email_confirmed?: a confirmed token for the address, or no tokens.
    let confirmed: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $1 AND email = $2 AND confirmed) \
             OR NOT EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $1)",
    )
    .bind(user_id)
    .bind(&email)
    .fetch_one(&mut *conn)
    .await?;
    if active && confirmed {
        return Ok(());
    }
    let token = crate::accounts::create_email_token(
        &mut *conn,
        user_id,
        &email,
        crate::accounts::token_scopes::SIGNUP,
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "critical_user_email",
        json!({"type": "signup", "user_id": user_id, "email_token": token, "to_address": null}),
    )
    .await?;
    Ok(())
}

/// `redirect_confirm(email)`: the email kept in the session, and on to
/// confirm-email.
fn redirect_confirm(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    email: &str,
) -> Result<Response, AppError> {
    let mut session = load_forum_session(state, headers);
    session
        .map
        .insert(REGISTERED_EMAIL.into(), Scalar::Str(email.to_string()));
    session.changed = true;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let location = format!("{}/finish-installation/confirm-email", urls.base_url()?);
    let mut response = (StatusCode::FOUND, [(header::LOCATION, location)]).into_response();
    if let Some(cookie) = session.set_cookie(state, settings)? {
        response
            .headers_mut()
            .append(header::SET_COOKIE, cookie.parse()?);
    }
    Ok(response)
}

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// POST /finish-installation/register
pub async fn register(
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
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    if !settings.get("has_login_hint")?.truthy() {
        return Ok(forbidden(&state));
    }
    let allowed = allowed_emails(&state);
    let Some(email) = params::string(&p, "email").map(|e| e.trim().to_string()) else {
        return Err(Unsupported(
            "finish installation's register without an email (a NoMethodError)",
        )
        .into());
    };
    if !allowed.contains(&email) {
        return Ok(super::search::invalid_parameters(
            &state,
            "Discourse::InvalidParameters",
        ));
    }
    let username = params::string(&p, "username").unwrap_or_default();
    let password = params::string(&p, "password").unwrap_or_default();

    let mut tx = sqlx::Acquire::begin(&mut *conn).await?;
    // User.find_by_email: an account with the address gets the email again.
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT user_id FROM user_emails WHERE lower(email) = $1 ORDER BY \"primary\" DESC LIMIT 1",
    )
    .bind(email.to_lowercase())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(user_id) = existing {
        send_signup_email(&mut tx, user_id).await?;
        tx.commit().await?;
        return redirect_confirm(&state, &settings, &headers, &email);
    }

    // @user.save with password_required!: the username's and the
    // password's errors are shown (a developer's email: the admin length).
    let mut form = Form {
        email: Some(email.clone()),
        username: Some(username.clone()),
        password: Some(password.clone()),
        ..Form::default()
    };
    if let Some(e) = crate::signup::username_format_error(&settings, &state.i18n, &username)? {
        form.username_errors.push(e);
    } else {
        let lower = crate::accounts::normalize_username(&username)?;
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1) \
             OR EXISTS (SELECT 1 FROM groups WHERE lower(name) = $1)",
        )
        .bind(&lower)
        .fetch_one(&mut *tx)
        .await?;
        if taken {
            form.username_errors.push(t(&state, "user.username.unique"));
        }
    }
    let email_lower = email.to_lowercase();
    let password_error = if password.is_empty() {
        Some("blank")
    } else {
        crate::accounts::password_error(
            &settings,
            &password,
            &crate::accounts::PasswordOwner {
                admin: true,
                username: &username,
                name: None,
                email: Some(&email_lower),
                current: None,
            },
        )?
    };
    if let Some(key) = password_error {
        let count = crate::accounts::too_short_count(&settings, true, &password)?;
        form.password_errors
            .push(crate::accounts::password_error_message(
                &state.i18n,
                key,
                count,
            ));
    }
    if !crate::accounts::valid_email(&email_lower) {
        return Err(Unsupported("a developer email that is not a valid address").into());
    }
    if !form.username_errors.is_empty() || !form.password_errors.is_empty() {
        tx.commit().await?;
        return register_page(
            &state, &mut conn, &settings, &guardian, &headers, &uri, allowed, form,
        )
        .await;
    }
    let payloads = crate::web_hooks::Payloads {
        settings: &settings,
        i18n: &state.i18n,
        config: &state.config,
    };
    let trust_level = settings.get("default_trust_level")?.to_i() as i32;
    let user_id = crate::signup::insert_user(
        &mut tx,
        &settings,
        &payloads,
        &crate::signup::NewUser {
            username: &username,
            name: None,
            email: &email_lower,
            password: Some(&password),
            trust_level,
            active: false,
            approved: false,
            moderator: false,
            staged: false,
            locale: None,
            ip: None,
            custom_fields: &[],
        },
    )
    .await?;
    // @user.change_trust_level!(1) if @user.trust_level < 1
    if trust_level < 1 {
        let host = crate::pretty_text::Host::from_state(&state);
        let promotion = crate::promotion::Ctx {
            settings: &settings,
            i18n: &state.i18n,
            host: &host,
            config: &state.config,
        };
        if crate::promotion::change_trust_level(&mut tx, &promotion, user_id, 1, None)
            .await?
            .is_err()
        {
            return Err(Unsupported("a refused trust level change at registration").into());
        }
    }
    send_signup_email(&mut tx, user_id).await?;
    tx.commit().await?;
    redirect_confirm(&state, &settings, &headers, &email_lower)
}

/// GET /finish-installation/confirm-email
pub async fn confirm_email(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    let mut session = load_forum_session(&state, &headers);
    let email = match session.map.get(REGISTERED_EMAIL) {
        Some(Scalar::Str(email)) => email.clone(),
        _ => String::new(),
    };
    let csrf_token = masked_csrf(&mut session)?;
    let html = ConfirmEmailPage {
        head: head(&state, &mut conn, &settings, &guardian, &uri).await?,
        title: t(&state, "finish_installation.confirm_email.title"),
        message: state
            .i18n
            .t_with(
                "finish_installation.confirm_email.message",
                &[("email", &html_escape::encode_text(&email))],
            )
            .unwrap_or_default(),
        button: t(&state, "finish_installation.resend_email.title"),
        csrf_token,
    }
    .render();
    page(html, session.set_cookie(&state, &settings)?)
}

/// PUT /finish-installation/resend-email (the confirm page's button posts
/// with `_method=put`).
pub async fn resend_email(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    method: axum::http::Method,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    // Rack::MethodOverride: a POST is a PUT only with _method=put.
    if method == axum::http::Method::POST
        && !params::string(&p, "_method").is_some_and(|m| m.eq_ignore_ascii_case("put"))
    {
        return Ok(super::topics::not_found_response(&state));
    }
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let mut conn = state.pool.acquire().await?;
    let settings = load_settings(&state, &mut conn).await?;
    let session = load_forum_session(&state, &headers);
    let email = match session.map.get(REGISTERED_EMAIL) {
        Some(Scalar::Str(email)) => email.clone(),
        _ => String::new(),
    };
    let user_id: Option<i32> = sqlx::query_scalar(
        "SELECT user_id FROM user_emails WHERE lower(email) = $1 ORDER BY \"primary\" DESC LIMIT 1",
    )
    .bind(email.to_lowercase())
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(user_id) = user_id {
        send_signup_email(&mut conn, user_id).await?;
    }
    let html = ResendEmailPage {
        head: head(&state, &mut conn, &settings, &guardian, &uri).await?,
        title: t(&state, "finish_installation.resend_email.title"),
        message: state
            .i18n
            .t_with(
                "finish_installation.resend_email.message",
                &[("email", &html_escape::encode_text(&email))],
            )
            .unwrap_or_default(),
    }
    .render();
    page(html, None)
}
