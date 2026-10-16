//! SessionController#sso and #sso_login: logging in through a
//! DiscourseConnect provider. /session/sso sends the browser to the
//! provider with a signed nonce; the provider sends it back to
//! /session/sso_login with the signed user, who is found or created
//! (crate::discourse_connect) and logged on, redeeming the invite the
//! visitor came from.
//!
//! Refused: suspended users (the staff reason's sanitizing, as the
//! password login refuses it).

use askama::Template;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::json;

use super::search::Peer;
use crate::discourse_connect::{self, LookupError, NonceStore, ParseError};
use crate::session::current::Incoming;
use crate::session::forum::load_forum_session;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

fn form_pairs(map: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| crate::params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// A cookie value as Rack reads it: percent-decoded, "+" as a space.
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = crate::session::current::cookie(headers, name)?;
    form_urlencoded::parse(format!("v={raw}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
}

/// `cookies.delete(name)`: only for a cookie the request sent.
pub(super) fn delete_cookie(headers: &HeaderMap, name: &str, base_path: &str) -> Option<String> {
    crate::session::current::cookie(headers, name)?;
    let path = if base_path.is_empty() { "/" } else { base_path };
    Some(format!(
        "{name}=; path={path}; max-age=0; expires=Thu, 01 Jan 1970 00:00:00 GMT"
    ))
}

/// `URI.parse(url)`'s path and query, "path?query".
fn path_and_query(url: &str) -> String {
    let rest = match url.split_once("://") {
        Some((_, rest)) => match rest.find('/') {
            Some(i) => &rest[i..],
            None => "",
        },
        None => url,
    };
    let rest = rest.split('#').next().unwrap_or("");
    rest.to_string()
}

fn host(headers: &HeaderMap) -> &str {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost")
}

fn redirect(location: &str, cookies: &[String]) -> Result<Response, AppError> {
    let mut response = (
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_str(location)?)],
    )
        .into_response();
    for c in cookies {
        response
            .headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(c)?);
    }
    Ok(response)
}

/// GET /session/sso
pub async fn sso(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_discourse_connect")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let params = crate::params::parse(uri.query(), &headers, &axum::body::Bytes::new());
    let home = format!("{base_path}/");
    let destination_url = match cookie_value(&headers, "destination_url") {
        Some(url) => Some(url),
        None => crate::session::server::get(&mut conn, &session_id, "destination_url")
            .await?
            .and_then(|v| v.as_str().map(str::to_string)),
    };
    let mut return_path = crate::params::string(&params, "return_path").unwrap_or(home.clone());
    if let Some(destination) = destination_url
        && return_path == home
    {
        return_path = path_and_query(&destination);
    }
    crate::session::server::delete(&mut conn, &session_id, "destination_url").await?;
    let mut cookies = Vec::new();
    cookies.extend(delete_cookie(&headers, "destination_url", &base_path));
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let store = NonceStore {
        csrf_protection: settings.get("discourse_connect_csrf_protection")?.truthy(),
        session_id: &session_id,
    };
    let sso =
        discourse_connect::generate_sso(&mut conn, &store, &urls.base_url()?, &return_path).await?;
    if settings.get("verbose_discourse_connect_logging")?.truthy() {
        tracing::warn!(
            "Verbose SSO log: Started SSO process\n\n{}",
            sso.diagnostics()
        );
    }
    let url = sso.to_url(
        &settings.get("discourse_connect_url")?.to_s(),
        &settings.get("discourse_connect_secret")?.to_s(),
    );
    if let Some(cookie) = session.set_cookie(&state, &settings)? {
        cookies.push(cookie);
    }
    redirect(&url, &cookies)
}

#[derive(Template)]
#[template(path = "sso_error.html")]
struct SsoErrorPage {
    site_title: String,
    lang: String,
    base_path: String,
    crawler: crate::html::Crawler,
    viewer: Option<crate::html::Viewer>,
    bus_position: String,
    chrome: crate::html::Chrome,
    title: String,
    error: String,
}

/// `render_sso_error(text:, status:)`: session/sso_login.html.erb in the
/// no_ember layout (the text is HTML).
/// `validate_invitation!(sso)`: the invite the server session kept from
/// the invite page, or the Invite::ValidationFailed message.
async fn validate_invitation(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    session_id: &str,
    sso: &discourse_connect::Sso,
) -> Result<Result<Option<crate::invites::Invite>, String>, AppError> {
    let key = match crate::session::server::get(&mut *conn, session_id, "invite-key").await? {
        Some(serde_json::Value::String(key)) => key,
        Some(serde_json::Value::Null) | None => return Ok(Ok(None)),
        Some(other) => other.to_string(),
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let base_url = urls.base_url()?;
    let site_name = settings.get("title")?.to_s();
    let t_with =
        |key: &str, args: &[(&str, &str)]| state.i18n.t_with(key, args).unwrap_or_default();
    let Some(invite) = crate::invites::Invite::find_by_key(&mut *conn, &key).await? else {
        return Ok(Err(t_with("invite.not_found", &[("base_url", &base_url)])));
    };
    if invite.redeemable(&mut *conn).await? {
        if invite.is_email_invite() && sso.get("email") != invite.email() {
            return Ok(Err(t(state, "invite.not_matching_email")));
        }
    } else if invite.expired() {
        return Ok(Err(t_with("invite.expired", &[("base_url", &base_url)])));
    } else if invite.redeemed(&mut *conn).await? {
        return Ok(Err(t_with(
            "invite.not_found_template",
            &[("site_name", &site_name), ("base_url", &base_url)],
        )));
    }
    Ok(Ok(Some(invite)))
}

/// `redeem_invitation(invite, sso, user)`: InviteRedeemer for the
/// DiscourseConnect user, then the session's invite forgotten. Err is
/// the page's text: Invite::UserExists's message, or invite_redeem_failed
/// for the RecordInvalid and RecordNotSaved it rescues.
#[allow(clippy::too_many_arguments)]
async fn redeem_invitation(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    session_id: &str,
    sso: &discourse_connect::Sso,
    invite: &crate::invites::Invite,
    user_id: i32,
    ip: &str,
) -> Result<Result<(), String>, AppError> {
    use super::invite_redeem::{RedeemArgs, Redeemed};
    let args = RedeemArgs {
        email: sso.get("email").map(str::to_string),
        username: sso.get("username").map(str::to_string),
        name: sso.get("name").map(str::to_string),
        ip: Some(ip.to_string()),
        redeeming_user: Some(user_id),
        ..RedeemArgs::default()
    };
    if let Redeemed::Failed(message) =
        super::invite_redeem::redeem(&mut *conn, state, settings, invite, &args).await?
    {
        let exists = t(state, "invite.existing_user_already_redemeed");
        return Ok(Err(if message == exists {
            message
        } else {
            t(state, "discourse_connect.invite_redeem_failed")
        }));
    }
    crate::session::server::delete(&mut *conn, session_id, "invite-key").await?;
    Ok(Ok(()))
}

async fn sso_error(
    state: &AppState,
    settings: &SiteSettings,
    uri: &Uri,
    status: u16,
    text: &str,
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
    drop(conn);
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let page = SsoErrorPage {
        crawler: crate::html::Crawler::for_request(&urls, uri, None)?,
        title: state
            .i18n
            .t("discourse_connect.login_error")
            .unwrap_or("Login Error")
            .to_string(),
        error: text.to_string(),
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        viewer: None,
        bus_position: String::new(),
        chrome: site.chrome,
    };
    let html = page.render().map_err(crate::html::HtmlError::from)?;
    Ok((
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Html(html),
    )
        .into_response())
}

fn t(state: &AppState, key: &str) -> String {
    state.i18n.t(key).unwrap_or(key).to_string()
}

/// `domain_redirect_allowed?(hostname)`
fn domain_redirect_allowed(settings: &SiteSettings, hostname: &str) -> Result<bool, AppError> {
    let allowed = settings
        .get("discourse_connect_allowed_redirect_domains")?
        .to_s();
    if allowed.trim().is_empty() {
        return Ok(false);
    }
    Ok(allowed.split('|').any(|d| d == "*" || d == hostname))
}

/// GET /session/sso_login
pub async fn sso_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Peer(peer): Peer,
    uri: Uri,
    axum::Extension(incoming): axum::Extension<Incoming>,
    mode: Option<axum::Extension<crate::readonly::Mode>>,
) -> Result<Response, AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_discourse_connect")?.truthy() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mode = mode.map(|m| m.0);
    if mode.is_some_and(|m| m.readonly && !m.staff_writes_only) {
        return Err(crate::readonly::ReadOnly.into());
    }
    let query = uri.query().unwrap_or("");
    let params = crate::params::parse(uri.query(), &headers, &axum::body::Bytes::new());
    for required in ["sso", "sig"] {
        if crate::params::string(&params, required).is_none_or(|v| v.is_empty()) {
            return Ok(super::accounts::param_missing(required));
        }
    }
    let verbose = settings.get("verbose_discourse_connect_logging")?.truthy();
    let sso =
        match discourse_connect::parse(query, &settings.get("discourse_connect_secret")?.to_s())? {
            Ok(sso) => sso,
            Err(ParseError::Payload) => {
                if verbose {
                    tracing::warn!("Verbose SSO log: Payload is not base64");
                }
                return sso_error(
                    &state,
                    &settings,
                    &uri,
                    422,
                    &t(&state, "discourse_connect.payload_parse_error"),
                )
                .await;
            }
            Err(ParseError::BlankSecret) => {
                if verbose {
                    tracing::warn!("Verbose SSO log: blank secret detected");
                }
                return sso_error(
                    &state,
                    &settings,
                    &uri,
                    422,
                    &t(&state, "discourse_connect.signature_error"),
                )
                .await;
            }
            Err(ParseError::Signature) => {
                if verbose {
                    tracing::warn!("Verbose SSO log: Signature verification failed");
                }
                return sso_error(
                    &state,
                    &settings,
                    &uri,
                    422,
                    &t(&state, "discourse_connect.signature_error"),
                )
                .await;
            }
        };
    let mut session = load_forum_session(&state, &headers);
    let session_id = session.server_session_id();
    let store = NonceStore {
        csrf_protection: settings.get("discourse_connect_csrf_protection")?.truthy(),
        session_id: &session_id,
    };
    let nonce = sso.get("nonce");
    if !store.valid(&mut tx, nonce).await? {
        if verbose {
            let error = store.error(&mut tx, nonce).await?;
            tracing::warn!("Verbose SSO log: {error}\n\n{}", sso.diagnostics());
        }
        return sso_error(
            &state,
            &settings,
            &uri,
            419,
            &t(&state, "discourse_connect.timeout_expired"),
        )
        .await;
    }
    let ip = crate::remote_ip::remote_ip(&headers, peer)?;
    if crate::accounts::ip_blocked(&mut tx, &ip).await? {
        tx.commit().await?;
        return sso_error(
            &state,
            &settings,
            &uri,
            500,
            &t(&state, "discourse_connect.unknown_error"),
        )
        .await;
    }
    let mut return_path = store.return_path(&mut tx, nonce).await?;
    store.expire(&mut tx, nonce).await?;
    // validate_invitation!: the invite a visitor came from.
    let invite = match validate_invitation(&mut tx, &state, &settings, &session_id, &sso).await? {
        Ok(invite) => invite,
        Err(text) => {
            tx.commit().await?;
            return sso_error(&state, &settings, &uri, 400, &text).await;
        }
    };
    let cx = discourse_connect::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        state: &state,
    };
    let user_id =
        match discourse_connect::lookup_or_create_user(&mut tx, &cx, &sso, Some(&ip)).await? {
            Ok(id) => id,
            Err(e) => {
                // The nonce stays used: the rescue answers after expire_nonce!.
                let text = match e {
                    LookupError::BlankExternalId => t(&state, "discourse_connect.blank_id_error"),
                    LookupError::Email(None) => t(&state, "discourse_connect.no_email"),
                    LookupError::Email(Some(email)) => state
                        .i18n
                        .t_with(
                            "discourse_connect.email_error",
                            &[("email", &html_escape::encode_safe(&email))],
                        )
                        .unwrap_or_default(),
                    LookupError::Banned | LookupError::Unknown => {
                        t(&state, "discourse_connect.unknown_error")
                    }
                };
                drop(tx);
                let mut tx = state.pool.begin().await?;
                store.expire(&mut tx, nonce).await?;
                tx.commit().await?;
                return sso_error(&state, &settings, &uri, 500, &text).await;
            }
        };
    #[derive(sqlx::FromRow)]
    struct U {
        admin: bool,
        moderator: bool,
        active: bool,
        approved: bool,
        suspended: bool,
    }
    let user: U = sqlx::query_as(
        "SELECT admin, moderator, active, approved, \
                (suspended_till IS NOT NULL AND suspended_till > now()) AS suspended \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    if mode.is_some_and(|m| m.staff_writes_only) && !(user.admin || user.moderator) {
        return Err(crate::readonly::ReadOnly.into());
    }
    if user.suspended {
        return Err(Unsupported(
            "suspended users at a DiscourseConnect login (staff reason sanitizing)",
        )
        .into());
    }
    let base_path = state.config.globals.relative_url_root().to_string();
    let mut cookies: Vec<String> = Vec::new();
    // An invite to redeem when the user has redeemed none yet (the same
    // user visiting an invite link again keeps theirs).
    let invite = match invite {
        Some(invite) => {
            let redeemed_one: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM invited_users WHERE user_id = $1)",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            (!redeemed_one).then_some(invite)
        }
        None => None,
    };
    if settings.get("must_approve_users")?.truthy() && !user.approved {
        if let Some(invite) = &invite
            && let Err(text) = redeem_invitation(
                &mut tx,
                &state,
                &settings,
                &session_id,
                &sso,
                invite,
                user_id,
                &ip,
            )
            .await?
        {
            tx.commit().await?;
            return sso_error(&state, &settings, &uri, 412, &text).await;
        }
        tx.commit().await?;
        let not_approved = settings.get("discourse_connect_not_approved_url")?.to_s();
        if !not_approved.trim().is_empty() {
            return redirect(&not_approved, &cookies);
        }
        return sso_error(
            &state,
            &settings,
            &uri,
            403,
            &t(&state, "discourse_connect.account_not_approved"),
        )
        .await;
    } else if let Some(invite) = &invite {
        if let Err(text) = redeem_invitation(
            &mut tx,
            &state,
            &settings,
            &session_id,
            &sso,
            invite,
            user_id,
            &ip,
        )
        .await?
        {
            tx.commit().await?;
            return sso_error(&state, &settings, &uri, 412, &text).await;
        }
        // user.activate: the provider vouches for the account, so no
        // activation email.
        let email: String =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        super::invite_redeem::activate(&mut tx, &state, &settings, user_id, &email).await?;
        if incoming.guardian.user_id() != Some(user_id) {
            if verbose {
                tracing::warn!(
                    "Verbose SSO log: User was logged on\n\n{}",
                    sso.diagnostics()
                );
            }
            cookies.push(
                crate::session::login::log_on_user(
                    &state,
                    &settings,
                    &mut tx,
                    &headers,
                    peer,
                    uri.path(),
                    user_id,
                    false,
                )
                .await?,
            );
        }
        // path(topic.relative_url), the base path given twice as Rails does.
        let topic: Option<(i32, Option<String>)> = sqlx::query_as(
            "SELECT t.id, t.slug FROM topic_invites ti JOIN topics t ON t.id = ti.topic_id \
             WHERE ti.invite_id = $1 AND t.deleted_at IS NULL ORDER BY ti.id LIMIT 1",
        )
        .bind(invite.id)
        .fetch_optional(&mut *tx)
        .await?;
        return_path = match topic {
            Some((id, slug)) => {
                let slug = slug
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("{s}/"))
                    .unwrap_or_default();
                format!("{base_path}{base_path}/t/{slug}{id}")
            }
            None => format!("{base_path}/"),
        };
    } else if !user.active {
        // UserActivator#finish: EmailActivator, its message kept for the
        // account-created page.
        let email: String =
            sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
                .bind(user_id)
                .fetch_one(&mut *tx)
                .await?;
        let token = crate::accounts::create_email_token(
            &mut tx,
            user_id,
            &email,
            crate::accounts::token_scopes::SIGNUP,
        )
        .await?;
        crate::jobs::enqueue(
            &mut tx,
            "critical_user_email",
            json!({"type": "signup", "user_id": user_id, "email_token": token, "to_address": null}),
        )
        .await?;
        let message = state
            .i18n
            .t_with(
                "login.activate_email",
                &[("email", &html_escape::encode_text(&email))],
            )
            .unwrap_or_default();
        crate::session::server::set(
            &mut tx,
            &session_id,
            "user_created_message",
            &json!(message),
            crate::session::server::EXPIRY_SECONDS,
        )
        .await?;
        tx.commit().await?;
        if let Some(cookie) = session.set_cookie(&state, &settings)? {
            cookies.push(cookie);
        }
        return redirect(
            &format!("http://{}{base_path}/u/account-created", host(&headers)),
            &cookies,
        );
    } else if incoming.guardian.user_id() != Some(user_id) {
        // login_sso_user
        if verbose {
            tracing::warn!(
                "Verbose SSO log: User was logged on\n\n{}",
                sso.diagnostics()
            );
        }
        cookies.push(
            crate::session::login::log_on_user(
                &state,
                &settings,
                &mut tx,
                &headers,
                peer,
                uri.path(),
                user_id,
                false,
            )
            .await?,
        );
    }
    tx.commit().await?;

    // If it's not a relative URL check the host.
    let relative =
        return_path.starts_with('/') && return_path.chars().nth(1).is_some_and(|c| c != '/');
    if !relative {
        let urls = crate::url::Urls {
            config: &state.config,
            settings: &settings,
        };
        // `URI(return_path).hostname`: none for a relative reference.
        let hostname = if let Some(rest) = return_path.strip_prefix("//") {
            crate::final_destination::Uri::parse(&format!("http://{rest}")).map(|u| u.host)
        } else {
            crate::final_destination::Uri::parse(&return_path).map(|u| u.host)
        };
        if hostname.as_deref() == Some(urls.current_hostname()?.as_str()) {
            // uri.to_s: the url as given.
        } else if !domain_redirect_allowed(&settings, hostname.as_deref().unwrap_or(""))? {
            return_path = format!("{base_path}/");
        }
    }
    if return_path.contains(&format!("{base_path}/session/sso")) {
        return_path = format!("{base_path}/");
    }
    let location = if return_path.starts_with('/') && !return_path.starts_with("//") {
        format!("http://{}{return_path}", host(&headers))
    } else if return_path.starts_with("//") {
        format!("http:{return_path}")
    } else {
        return_path
    };
    if let Some(cookie) = session.set_cookie(&state, &settings)? {
        cookies.push(cookie);
    }
    redirect(&location, &cookies)
}

/// GET /u/:username/sso-email and /sso-payload (users#check_sso_email
/// and #check_sso_payload): the DiscourseConnect record's external email
/// or last payload, for admins, the look logged when it is another's.
pub async fn check_sso(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    username: &str,
    context: Option<String>,
    email: bool,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let lower = username.to_lowercase();
    let user: Option<(i32, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT u.id, r.external_email, r.last_payload FROM users u \
         LEFT JOIN single_sign_on_records r ON r.user_id = u.id WHERE u.username_lower = $1 LIMIT 1",
    )
    .bind(&lower)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((user_id, external_email, last_payload)) = user else {
        return Ok(super::topics::not_found_response(state));
    };
    if !guardian.is_admin() {
        return Ok((
            StatusCode::FORBIDDEN,
            axum::Json(json!({ "failed": "FAILED" })),
        )
            .into_response());
    }
    if !guardian.is_me(user_id) {
        // StaffActionLogger#log_check_email
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, context, \
                                         admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp())",
        )
        .bind(crate::staff_action_logs::action_id("check_email"))
        .bind(guardian.user_id())
        .bind(user_id)
        .bind(context)
        .bind(crate::staff_action_logs::admin_only(
            "check_email",
            &settings,
        )?)
        .execute(&mut *conn)
        .await?;
    }
    let none = || t(state, "user.email.does_not_exist");
    let (key, value) = if email {
        ("email", external_email)
    } else {
        ("payload", last_payload)
    };
    let value = value.filter(|v| !v.trim().is_empty()).unwrap_or_else(none);
    Ok(axum::Json(json!({ key: value })).into_response())
}

/// The admin user routes' user: `User.find_by(id:)`, `.json` aside.
async fn admin_target(conn: &mut sqlx::PgConnection, raw: &str) -> Result<Option<i32>, AppError> {
    let Ok(id) = raw.strip_suffix(".json").unwrap_or(raw).parse::<i32>() else {
        return Ok(None);
    };
    Ok(sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?)
}

/// DELETE /admin/users/:user_id/sso_record (Admin::UsersController#sso_record)
pub async fn destroy_sso_record(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(super::session::bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some(user_id) = admin_target(&mut tx, &user_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // can_delete_sso_record?
    if !(settings.get("enable_discourse_connect")?.truthy() && guardian.is_admin()) {
        return Ok(super::search::invalid_access(&state));
    }
    let removed = sqlx::query("DELETE FROM single_sign_on_records WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    if removed.rows_affected() == 0 {
        // `@user.single_sign_on_record.destroy!` on nil.
        return Err(Unsupported(
            "deleting a DiscourseConnect record the user lacks (NoMethodError)",
        )
        .into());
    }
    tx.commit().await?;
    Ok(axum::Json(json!({ "success": "OK" })).into_response())
}

/// Ruby's `inspect` of a JSON value (Ruby 3.4's `{"k" => v}`).
fn ruby_inspect(v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Null => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => ruby_string(s),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(ruby_inspect)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".into();
            }
            format!(
                "{{{}}}",
                map.iter()
                    .map(|(k, v)| format!("{} => {}", ruby_string(k), ruby_inspect(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    }
}

/// `String#inspect`: double quoted, with Ruby's escapes.
fn ruby_string(s: &str) -> String {
    let mut out = String::from("\"");
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{1b}' => out.push_str("\\e"),
            // "#{", "#$" and "#@" escaped so the literal reads back.
            '#' if matches!(chars.get(i + 1), Some('{' | '$' | '@')) => out.push_str("\\#"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// PUT /admin/users/:user_id/delete_associated_accounts
pub async fn delete_associated_accounts(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(super::session::bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some(user_id) = admin_target(&mut tx, &user_id).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    // can_delete_user_associated_accounts?
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let accounts: Vec<(String, String, serde_json::Value)> = sqlx::query_as(
        "SELECT provider_name, provider_uid, info FROM user_associated_accounts WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    let previous_value = accounts
        .iter()
        .map(|(provider, uid, info)| {
            format!(
                "{{provider: {}, uid: {}, info: {}}}",
                ruby_string(provider),
                ruby_string(uid),
                ruby_inspect(info)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let context = crate::params::string(&p, "context");
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, previous_value, context, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp(), clock_timestamp())",
    )
    .bind(crate::staff_action_logs::action_id("delete_associated_accounts"))
    .bind(guardian.user_id())
    .bind(user_id)
    .bind(&previous_value)
    .bind(context)
    .bind(crate::staff_action_logs::admin_only(
        "delete_associated_accounts",
        &settings,
    )?)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM user_associated_accounts WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(axum::Json(json!({ "success": "OK" })).into_response())
}

/// POST /admin/users/sync_sso (admins): a provider's payload applied to
/// the user it names, created when new; AdminDetailedUserSerializer.
pub async fn sync_sso(
    State(state): State<AppState>,
    crate::session::current::AuthGuardian(guardian): crate::session::current::AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let p = crate::params::parse(uri.query(), &headers, &body);
    if !super::session::csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(super::session::bad_csrf());
    }
    // AdminConstraint
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_discourse_connect")?.truthy() {
        return Ok((StatusCode::NOT_FOUND, "").into_response());
    }
    let failed = |message: String| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            axum::Json(json!({ "failed": "FAILED", "message": message })),
        )
            .into_response()
    };
    // Rack::Utils.build_query(sso:, sig:)
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("sso", &crate::params::string(&p, "sso").unwrap_or_default())
        .append_pair("sig", &crate::params::string(&p, "sig").unwrap_or_default())
        .finish();
    let secret = settings.get("discourse_connect_secret")?.to_s();
    let sso = match discourse_connect::parse(&query, &secret)? {
        Ok(sso) => sso,
        Err(_) => return Ok(failed(t(&state, "discourse_connect.login_error"))),
    };
    let cx = discourse_connect::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        state: &state,
    };
    let user_id = match discourse_connect::lookup_or_create_user(&mut tx, &cx, &sso, None).await? {
        Ok(id) => id,
        Err(LookupError::BlankExternalId) => {
            return Ok(failed(t(&state, "discourse_connect.blank_id_error")));
        }
        Err(LookupError::Banned) => {
            return Ok(failed(t(&state, "discourse_connect.banned_id_error")));
        }
        Err(LookupError::Email(_)) => {
            return Err(Unsupported("sync_sso's validation messages (RecordInvalid)").into());
        }
        Err(LookupError::Unknown) => {
            return Err(Unsupported("sync_sso on an archived site").into());
        }
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let show = crate::admin_user_show::Context {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    let doc = crate::admin_user_show::show(
        &mut tx,
        &show,
        &guardian,
        user_id,
        crate::admin_user_show::ShowOptions {
            show: false,
            detailed: true,
        },
    )
    .await?
    .ok_or(Unsupported("a synced user that vanished"))?;
    tx.commit().await?;
    Ok(axum::Json(doc).into_response())
}

/// SessionController#sso_provider's rescues: plain text.
fn provider_refusal(state: &AppState, e: discourse_connect::ProviderError) -> Response {
    use discourse_connect::ProviderError;
    let (status, text) = match e {
        ProviderError::BlankSecret => (
            StatusCode::BAD_REQUEST,
            t(state, "discourse_connect.missing_secret"),
        ),
        // Not the parse error's text: it would give away the signature.
        ProviderError::Parse => (
            StatusCode::UNPROCESSABLE_ENTITY,
            t(state, "discourse_connect.login_error"),
        ),
        ProviderError::BlankReturnUrl => (
            StatusCode::BAD_REQUEST,
            "return_sso_url is blank, it must be provided".to_string(),
        ),
        ProviderError::InvalidParameter(param) => (
            StatusCode::BAD_REQUEST,
            state
                .i18n
                .t_with(
                    "discourse_connect.invalid_parameter_value",
                    &[("param", param)],
                )
                .unwrap_or_default(),
        ),
    };
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

fn is_xhr(headers: &HeaderMap) -> bool {
    headers
        .get("x-requested-with")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("XMLHttpRequest"))
}

/// GET /session/sso_provider
pub async fn sso_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: Uri,
    axum::Extension(incoming): axum::Extension<Incoming>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    provider(
        &state,
        &settings,
        &headers,
        &uri,
        &incoming,
        None,
        Vec::new(),
        false,
    )
    .await
}

/// `sso_provider(payload, confirmed_2fa_during_login)` after a login
/// (SessionController#login with an sso_payload cookie): the user just
/// logged on, the provider's answer for them.
#[allow(clippy::too_many_arguments)]
pub(super) async fn provider_after_login(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
    user_id: i32,
    payload: &str,
    cookies: Vec<String>,
    confirmed_2fa_during_login: bool,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let user = crate::session::current::SessionUser::load(&mut conn, user_id)
        .await?
        .ok_or(Unsupported("a user that vanished at login"))?;
    let guardian = crate::guardian::Guardian::for_user(&mut conn, &user).await?;
    drop(conn);
    let incoming = Incoming {
        guardian,
        had_token_cookie: false,
        session: None,
    };
    provider(
        state,
        settings,
        headers,
        uri,
        &incoming,
        Some(payload.to_string()),
        cookies,
        confirmed_2fa_during_login,
    )
    .await
}

/// SessionController#sso_provider through run_second_factor! with
/// SecondFactor::Actions::DiscourseConnectProvider.
#[allow(clippy::too_many_arguments)]
async fn provider(
    state: &AppState,
    settings: &SiteSettings,
    headers: &HeaderMap,
    uri: &Uri,
    incoming: &Incoming,
    payload: Option<String>,
    mut cookies: Vec<String>,
    confirmed_2fa_during_login: bool,
) -> Result<Response, AppError> {
    if !settings.get("enable_discourse_connect_provider")?.truthy() {
        return Ok(super::topics::not_found_response(state));
    }
    let params = crate::params::parse(uri.query(), headers, &axum::body::Bytes::new());
    let base_path = state.config.globals.relative_url_root().to_string();
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let user_id = incoming.guardian.user_id();
    let mut conn = state.pool.acquire().await?;
    let mut session = load_forum_session(state, headers);
    let session_id = session.server_session_id();
    let finish = |mut response: Response, cookies: &[String]| -> Result<Response, AppError> {
        for c in cookies {
            response
                .headers_mut()
                .append(header::SET_COOKIE, HeaderValue::from_str(c)?);
        }
        Ok(response)
    };
    // The payload: the login's, else the request's (params.require(:sso)).
    let payload_of = |payload: &Option<String>| -> Option<String> {
        match payload {
            Some(p) => Some(p.clone()),
            None => uri.query().map(str::to_string),
        }
    };
    let nonce = crate::params::string(&params, "second_factor_nonce").filter(|n| !n.is_empty());
    if let Some(nonce) = nonce {
        // verify_second_factor_auth_completed
        let challenge =
            match crate::second_factor_auth::find(&mut conn, &session_id, &nonce, user_id).await? {
                Err(bad) => return Ok(bad.response(&state.i18n)),
                Ok(c) => c,
            };
        if challenge.get("successful") != Some(&serde_json::Value::Bool(true)) {
            return Ok(crate::second_factor_auth::BadChallenge {
                key: "second_factor_auth.challenge_not_completed",
                status: StatusCode::UNAUTHORIZED,
            }
            .response(&state.i18n));
        }
        crate::second_factor_auth::delete(&mut conn, &session_id).await?;
        let payload = challenge
            .get("callback_params")
            .and_then(|c| c.get("payload"))
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        let mut request = match discourse_connect::provider_parse(&payload, settings)? {
            Ok(r) => r,
            Err(e) => return Ok(provider_refusal(state, e)),
        };
        let user_id = user_id.ok_or(Unsupported("a completed challenge without a user"))?;
        discourse_connect::populate_user_data(
            &mut conn,
            settings,
            &urls,
            &mut request.sso,
            user_id,
        )
        .await?;
        request
            .sso
            .set("confirmed_2fa", discourse_connect::Field::Bool(true));
        let url = sso_return_url(&request);
        return finish(
            axum::Json(json!({"success": "OK", "redirect_url": url})).into_response(),
            &cookies,
        );
    }
    if payload.is_none() && crate::params::string(&params, "sso").is_none_or(|s| s.is_empty()) {
        return Ok(super::accounts::param_missing("sso"));
    }
    let payload_text = payload_of(&payload).unwrap_or_default();
    let mut request = match discourse_connect::provider_parse(&payload_text, settings)? {
        Ok(r) => r,
        Err(e) => return Ok(provider_refusal(state, e)),
    };
    let logout = request.sso.flag("logout").unwrap_or(false);
    let require_2fa = request.sso.flag("require_2fa").unwrap_or(false);
    // skip_second_factor_auth?
    if user_id.is_none() || logout || !require_2fa || confirmed_2fa_during_login {
        if logout {
            // destroy(trusted_return_url: return_sso_url)
            let return_url = request.sso.get("return_sso_url").unwrap_or("").to_string();
            let response =
                super::session::log_off(state, &mut conn, settings, headers, incoming, &return_url)
                    .await?;
            return finish(response, &cookies);
        }
        let Some(user_id) = user_id else {
            if request.sso.get("prompt") == Some("none") {
                request
                    .sso
                    .set("failed", discourse_connect::Field::Bool(true));
                let url = sso_return_url(&request);
                return finish(redirect_response(&url)?, &cookies);
            }
            // cookies[:sso_payload] = payload; to the login page.
            cookies.push(format!(
                "sso_payload={}; path={}",
                form_urlencoded::byte_serialize(payload_text.as_bytes()).collect::<String>(),
                if base_path.is_empty() {
                    "/"
                } else {
                    &base_path
                }
            ));
            return finish(
                redirect_response(&format!("http://{}{base_path}/login", host(headers)))?,
                &cookies,
            );
        };
        discourse_connect::populate_user_data(
            &mut conn,
            settings,
            &urls,
            &mut request.sso,
            user_id,
        )
        .await?;
        if confirmed_2fa_during_login {
            request
                .sso
                .set("confirmed_2fa", discourse_connect::Field::Bool(true));
        }
        let url = sso_return_url(&request);
        if is_xhr(headers) {
            cookies.push(format!(
                "sso_destination_url={}; path={}",
                form_urlencoded::byte_serialize(url.as_bytes()).collect::<String>(),
                if base_path.is_empty() {
                    "/"
                } else {
                    &base_path
                }
            ));
            return finish(
                axum::Json(json!({"success": "OK", "redirect_url": url})).into_response(),
                &cookies,
            );
        }
        return finish(redirect_response(&url)?, &cookies);
    }
    let user_id = user_id.expect("a user");
    let mut any = false;
    for method in crate::second_factor_auth::DEFAULT_ALLOWED_METHODS {
        any |=
            crate::second_factor_auth::valid_method_for_user(&mut conn, settings, user_id, method)
                .await?;
    }
    if !any {
        // no_second_factors_enabled!
        discourse_connect::populate_user_data(
            &mut conn,
            settings,
            &urls,
            &mut request.sso,
            user_id,
        )
        .await?;
        request
            .sso
            .set("no_2fa_methods", discourse_connect::Field::Bool(true));
        let url = sso_return_url(&request);
        if is_xhr(headers) {
            return Err(Unsupported(
                "an xhr provider request without second factors (no template)",
            )
            .into());
        }
        return finish(redirect_response(&url)?, &cookies);
    }
    // second_factor_auth_required!: the challenge staged.
    let hostname = crate::wildcard_url::parse(request.sso.get("return_sso_url").unwrap_or(""))
        .and_then(|u| u.host)
        .unwrap_or_default();
    let nonce = crate::second_factor_auth::new_nonce();
    let mut challenge = serde_json::Map::new();
    challenge.insert("nonce".into(), json!(nonce));
    challenge.insert("callback_method".into(), json!("GET"));
    challenge.insert(
        "callback_path".into(),
        json!(format!("{base_path}/session/sso_provider")),
    );
    challenge.insert("callback_params".into(), json!({ "payload": payload_text }));
    challenge.insert(
        "allowed_methods".into(),
        json!(crate::second_factor_auth::DEFAULT_ALLOWED_METHODS),
    );
    challenge.insert(
        "generated_at".into(),
        json!(crate::clock::now().timestamp()),
    );
    challenge.insert("target_user_id".into(), json!(user_id));
    challenge.insert(
        "description".into(),
        json!(
            state
                .i18n
                .t_with(
                    "second_factor_auth.actions.discourse_connect_provider.description",
                    &[("hostname", &hostname)],
                )
                .unwrap_or_default()
        ),
    );
    crate::second_factor_auth::store(&mut conn, &session_id, &challenge).await?;
    if let Some(cookie) = session.set_cookie(state, settings)? {
        cookies.push(cookie);
    }
    let response =
        crate::second_factor_auth::required_response(is_xhr(headers), &urls.base_url()?, &nonce);
    finish(response, &cookies)
}

/// `sso.to_url(sso.return_sso_url)`, signed with the request's secret.
fn sso_return_url(request: &discourse_connect::ProviderRequest) -> String {
    request.sso.to_url(
        request.sso.get("return_sso_url").unwrap_or(""),
        &request.secret,
    )
}

fn redirect_response(location: &str) -> Result<Response, AppError> {
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, HeaderValue::from_str(location)?)],
    )
        .into_response())
}
