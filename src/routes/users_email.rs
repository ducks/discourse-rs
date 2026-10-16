//! The email endpoints of the preferences: UsersEmailController#create
//! and #update (EmailUpdater#change_to), its confirmation links
//! (show_confirm_old_email, confirm_old_email, show_confirm_new_email,
//! confirm_new_email, the HTML being Ember's confirm pages), and
//! UsersController#update_primary_email and #destroy_email.

use askama::Template;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::email_updater::{self as updater, Confirmed, Owner};
use crate::html::Crawler;
use crate::params;
use crate::session::current::{AuthGuardian, Incoming};
use crate::site_settings::SiteSettings;
use crate::topic_list_view::escape;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn json_error(status: StatusCode, messages: Vec<String>) -> Response {
    (status, Json(json!({ "errors": messages }))).into_response()
}

fn failed(status: StatusCode) -> Response {
    (status, Json(json!({ "failed": "FAILED" }))).into_response()
}

/// `fetch_user_from_params` for the email endpoints: oneself, else an
/// active user (staff find inactive ones too).
async fn fetch_user(
    conn: &mut sqlx::PgConnection,
    guardian: &crate::guardian::Guardian,
    username: &str,
) -> Result<Option<i32>, sqlx::Error> {
    let lower = username.trim_end_matches(".json").to_lowercase();
    if let Some(me) = guardian.user()
        && me.username.to_lowercase() == lower
    {
        return Ok(Some(me.id));
    }
    sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1 AND (active OR $2)")
        .bind(&lower)
        .bind(guardian.is_staff())
        .fetch_optional(conn)
        .await
}

/// What the email endpoints under `/u/:username/` are.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// POST preferences/email: users_email#create
    Add,
    /// PUT preferences/email: users_email#update
    Change,
    /// PUT preferences/primary-email: users#update_primary_email
    Primary,
    /// DELETE preferences/email: users#destroy_email
    Destroy,
}

impl Endpoint {
    pub fn from_tail(method: &str, rest: &str) -> Option<Endpoint> {
        let rest = rest.strip_suffix(".json").unwrap_or(rest);
        Some(match (method, rest) {
            ("POST", "preferences/email") => Endpoint::Add,
            ("PUT", "preferences/email") => Endpoint::Change,
            ("PUT", "preferences/primary-email") => Endpoint::Primary,
            ("DELETE", "preferences/email") => Endpoint::Destroy,
            _ => return None,
        })
    }

    fn method(self) -> &'static str {
        match self {
            Endpoint::Add => "POST",
            Endpoint::Change | Endpoint::Primary => "PUT",
            Endpoint::Destroy => "DELETE",
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn handle(
    state: AppState,
    guardian: crate::guardian::Guardian,
    username: String,
    endpoint: Endpoint,
    headers: HeaderMap,
    peer: Option<std::net::SocketAddr>,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(
        &state,
        &headers,
        &form_pairs(&p),
        uri.path(),
        endpoint.method(),
    ) {
        return Ok(bad_csrf());
    }
    // requires_login: users_email's create and update; users#
    // update_primary_email and destroy_email reach the guardian first.
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if endpoint != Endpoint::Change && !s.get("enable_secondary_emails")?.truthy() {
        return Ok(failed(StatusCode::GONE));
    }
    // params.require(:email)
    let Some(email) = params::string(&p, "email").filter(|e| !e.trim().is_empty()) else {
        return Ok(super::accounts::param_missing("email"));
    };
    let Some(user_id) = fetch_user(&mut tx, &guardian, &username).await? else {
        return Ok(super::topics::not_found_response(&state));
    };
    // fetch_user_from_params: the route's username always wins over an
    // external_id param.
    match endpoint {
        Endpoint::Add | Endpoint::Change => {
            let owner = Owner::load(&mut tx, user_id).await?;
            // RateLimiter.new(user, "email-hr-#{ip}") and "email-min-#{ip}"
            let ip = crate::remote_ip::remote_ip(&headers, peer)?;
            for (kind, max, secs) in [("email-hr", 6, 3600), ("email-min", 3, 60)] {
                let limiter = crate::rate_limiter::RateLimiter::new(
                    Some(crate::rate_limiter::LimitUser {
                        id: user_id,
                        staff: owner.admin || owner.moderator,
                    }),
                    &format!("{kind}-{ip}"),
                    max,
                    secs,
                );
                if limiter
                    .performed(&state.pool, state.config.rate_limits)
                    .await?
                    .is_err()
                {
                    let text = state.i18n.t("rate_limiter.slow_down").unwrap_or_default();
                    return Ok(json_error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        vec![text.to_string()],
                    ));
                }
            }
            // ensure_can_edit_email!
            if !updater::can_edit_email(&s, &guardian, user_id)? {
                return Ok(super::search::invalid_access(&state));
            }
            let errors = updater::change_to(
                &mut tx,
                &s,
                &state.i18n,
                &guardian,
                &owner,
                &email,
                endpoint == Endpoint::Add,
            )
            .await?;
            if !errors.is_empty() {
                return Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, errors));
            }
            tx.commit().await?;
            Ok(StatusCode::OK.into_response())
        }
        Endpoint::Primary => {
            if !updater::can_edit_email(&s, &guardian, user_id)? {
                return Ok(super::search::invalid_access(&state));
            }
            let owner = Owner::load(&mut tx, user_id).await?;
            if owner.email == email {
                return Ok(Json(json!({ "success": "OK" })).into_response());
            }
            let new_primary: Option<i32> = sqlx::query_scalar(
                "SELECT id FROM user_emails WHERE user_id = $1 AND email = $2 ORDER BY id LIMIT 1",
            )
            .bind(user_id)
            .bind(&email)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(new_primary) = new_primary else {
                let text = state
                    .i18n
                    .t("change_email.doesnt_exist")
                    .unwrap_or_default();
                return Ok((
                    StatusCode::PRECONDITION_REQUIRED,
                    Json(json!({ "failed": "FAILED", "errors": [text] })),
                )
                    .into_response());
            };
            sqlx::query(
                "UPDATE user_emails SET \"primary\" = FALSE, updated_at = now() \
                 WHERE user_id = $1 AND \"primary\"",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            updater::destroy_email_tokens(&mut tx, &owner.email).await?;
            sqlx::query(
                "UPDATE user_emails SET \"primary\" = TRUE, updated_at = now() WHERE id = $1",
            )
            .bind(new_primary)
            .execute(&mut *tx)
            .await?;
            updater::destroy_email_tokens(&mut tx, &email).await?;
            user_updated(&state, &mut tx, &s, user_id).await?;
            updater::log(&mut tx, &guardian, user_id, updater::actions::UPDATE_EMAIL).await?;
            tx.commit().await?;
            Ok(Json(json!({ "success": "OK" })).into_response())
        }
        Endpoint::Destroy => {
            if !updater::can_edit_email(&s, &guardian, user_id)? {
                return Ok(super::search::invalid_access(&state));
            }
            // An unconfirmed address's requests, else a secondary address.
            let requests: Vec<(i32, Option<i32>, Option<i32>)> = sqlx::query_as(
                "SELECT id, old_email_token_id, new_email_token_id FROM email_change_requests \
                 WHERE user_id = $1 AND new_email = $2 ORDER BY id",
            )
            .bind(user_id)
            .bind(&email)
            .fetch_all(&mut *tx)
            .await?;
            if !requests.is_empty() {
                // destroy_all, its tokens with it (dependent: :destroy).
                for (id, old_token, new_token) in requests {
                    sqlx::query("DELETE FROM email_change_requests WHERE id = $1")
                        .bind(id)
                        .execute(&mut *tx)
                        .await?;
                    for token in [old_token, new_token].into_iter().flatten() {
                        sqlx::query("DELETE FROM email_tokens WHERE id = $1")
                            .bind(token)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
            } else {
                let gone: Vec<String> = sqlx::query_scalar(
                    "DELETE FROM user_emails WHERE user_id = $1 AND email = $2 AND NOT \"primary\" RETURNING email",
                )
                .bind(user_id)
                .bind(&email)
                .fetch_all(&mut *tx)
                .await?;
                if gone.is_empty() {
                    return Ok(failed(StatusCode::PRECONDITION_REQUIRED));
                }
                for address in &gone {
                    updater::destroy_email_tokens(&mut tx, address).await?;
                }
                user_updated(&state, &mut tx, &s, user_id).await?;
            }
            updater::log(&mut tx, &guardian, user_id, updater::actions::DESTROY_EMAIL).await?;
            tx.commit().await?;
            Ok(Json(json!({ "success": "OK" })).into_response())
        }
    }
}

/// DiscourseEvent :user_updated: its web hooks.
async fn user_updated(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    crate::web_hooks::enqueue_user_hooks(
        conn,
        &crate::web_hooks::Payloads {
            settings: s,
            i18n: &state.i18n,
            config: &state.config,
        },
        "user_updated",
        user_id,
    )
    .await?;
    Ok(())
}

/// Which confirmation link: the old address's or the new one's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Old,
    New,
}

/// GET /u/confirm-{old,new}-email/:token(.json): the request as JSON, or
/// Ember's confirm page.
pub async fn show_confirm(
    state: AppState,
    incoming: Incoming,
    link: Link,
    token: String,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (token, json) = match token.strip_suffix(".json") {
        Some(t) => (t.to_string(), true),
        None => (token, false),
    };
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let request =
        match load_request(&state, &mut conn, &s, &incoming.guardian, &token, link).await? {
            Ok(request) => request,
            // The JSON's error; for the page, the Ember route's model fails
            // with it.
            Err(response) => return Ok(*response),
        };
    if !json {
        return page(
            &state, &mut conn, &s, &incoming, link, &request, &headers, &uri,
        )
        .await;
    }
    Ok((
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Json(json!({
            "new_email": request.new_email,
            "old_email": request.old_email,
            "token": token,
        })),
    )
        .into_response())
}

/// UsersEmailController#load_change_request: 404 without the token's
/// request, 403 for another logged-in user's link.
async fn load_request(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    token: &str,
    link: Link,
) -> Result<Result<updater::Request, Box<Response>>, AppError> {
    let Some((token_id, user_id)) = updater::confirmable(&mut *conn, s, token).await? else {
        return Ok(Err(Box::new(super::topics::not_found_response(state))));
    };
    if guardian.user_id().is_some_and(|me| me != user_id) {
        return Ok(Err(Box::new(super::search::invalid_access(state))));
    }
    match updater::load_request(&mut *conn, token_id, user_id, link == Link::Old).await? {
        Some(request) => Ok(Ok(request)),
        None => Ok(Err(Box::new(super::topics::not_found_response(state)))),
    }
}

/// PUT /u/confirm-{old,new}-email/:token(.json)
pub async fn confirm(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    link: Link,
    token: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    let token = token.strip_suffix(".json").unwrap_or(&token).to_string();
    let mut tx = state.pool.begin().await?;
    let s = SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let request = match load_request(&state, &mut tx, &s, &guardian, &token, link).await? {
        Ok(request) => request,
        Err(response) => return Ok(*response),
    };
    let already_done = || {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": state.i18n.t("change_email.already_done").unwrap_or_default() })),
        )
            .into_response()
    };
    let mut cookie = None;
    if link == Link::New {
        // run_second_factor!(SecondFactor::Actions::ConfirmEmail, target_user:
        // change_request.user)
        use crate::second_factor_auth as auth;
        let mut session = crate::session::forum::load_forum_session(&state, &headers);
        let session_id = session.server_session_id();
        let nonce = p
            .get("second_factor_nonce")
            .and_then(params::scalar)
            .filter(|n| !crate::ruby::is_blank(n));
        if let Some(nonce) = nonce {
            // verify_second_factor_auth_completed
            let challenge =
                match auth::find(&mut tx, &session_id, &nonce, Some(request.user_id)).await? {
                    Err(bad) => return Ok(bad.response(&state.i18n)),
                    Ok(challenge) => challenge,
                };
            if challenge.get("successful") != Some(&Value::Bool(true)) {
                return Ok(auth::BadChallenge {
                    key: "second_factor_auth.challenge_not_completed",
                    status: StatusCode::UNAUTHORIZED,
                }
                .response(&state.i18n));
            }
            auth::delete(&mut tx, &session_id).await?;
        } else {
            let mut any = false;
            for method in auth::DEFAULT_ALLOWED_METHODS {
                any |= auth::valid_method_for_user(&mut tx, &s, request.user_id, method).await?;
            }
            if any {
                let base_path = state.config.globals.relative_url_root();
                let nonce = auth::initiate(
                    &mut tx,
                    &session_id,
                    auth::Required {
                        callback_method: "PUT",
                        callback_path: uri.path(),
                        callback_params: json!({ "token": token }),
                        target_user_id: request.user_id,
                        description: None,
                        redirect_url: Some(if guardian.user_id().is_some() {
                            format!("{base_path}/my/preferences/account")
                        } else {
                            format!("{base_path}/login")
                        }),
                    },
                )
                .await?;
                tx.commit().await?;
                let xhr = headers
                    .get("x-requested-with")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.eq_ignore_ascii_case("XMLHttpRequest"));
                let urls = crate::url::Urls {
                    config: &state.config,
                    settings: &s,
                };
                let response = auth::required_response(xhr, &urls.base_url()?, &nonce);
                let cookie = session.set_cookie(&state, &s)?;
                return super::accounts::with_session_cookie(response, cookie);
            }
        }
        cookie = session.set_cookie(&state, &s)?;
    }
    let result = updater::confirm(&mut tx, &s, &token).await?;
    match (link, result) {
        (Link::Old, Confirmed::AuthorizingNew) => {}
        (Link::New, Confirmed::Complete) => {
            user_updated(&state, &mut tx, &s, request.user_id).await?;
            let host = crate::pretty_text::Host::from_state(&state);
            let promotion = crate::promotion::Ctx {
                settings: &s,
                i18n: &state.i18n,
                host: &host,
                config: &state.config,
            };
            let cx = crate::group_manager::Cx {
                state: &state,
                promotion: &promotion,
            };
            crate::group_manager::set_automatic_groups(&mut tx, &cx, request.user_id).await?;
            // user_stat.reset_bounce_score!
            sqlx::query(
                "UPDATE user_stats SET bounce_score = 0, reset_bounce_score_after = NULL WHERE user_id = $1",
            )
            .bind(request.user_id)
            .execute(&mut *tx)
            .await?;
        }
        _ => {
            tx.commit().await?;
            return super::accounts::with_session_cookie(already_done(), cookie);
        }
    }
    tx.commit().await?;
    super::accounts::with_session_cookie(Json(json!({ "success": "OK" })).into_response(), cookie)
}

/// Ember's confirm-old-email and confirm-new-email pages.
#[allow(clippy::too_many_arguments)]
async fn page(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    incoming: &Incoming,
    link: Link,
    request: &updater::Request,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    let guardian = &incoming.guardian;
    let t = |key: &str| {
        state
            .i18n
            .t(&format!("js.{key}"))
            .unwrap_or(key)
            .to_string()
    };
    let t_with = |key: &str, args: &[(&str, &str)]| {
        state
            .i18n
            .t_with(&format!("js.{key}"), args)
            .unwrap_or_else(|| key.to_string())
    };
    let (class, title, body) = match link {
        Link::New => {
            let description = if request.old_email.is_some() {
                t("user.change_email.authorizing_new.description")
            } else {
                t("user.change_email.authorizing_new.description_add")
            };
            (
                "confirm-new-email",
                t("user.change_email.title"),
                format!(
                    "<p>{}</p><p>{}</p>",
                    escape(&description),
                    escape(&request.new_email)
                ),
            )
        }
        Link::Old => {
            let description = if request.old_email.is_some() {
                t("user.change_email.authorizing_old.description")
            } else {
                t("user.change_email.authorizing_old.description_add")
            };
            let mut body = format!("<p>{}</p>", escape(&description));
            if let Some(old) = &request.old_email {
                body.push_str(&format!(
                    "<p>{}</p>",
                    escape(&t_with(
                        "user.change_email.authorizing_old.old_email",
                        &[("email", old)]
                    ))
                ));
            }
            body.push_str(&format!(
                "<p>{}</p>",
                escape(&t_with(
                    "user.change_email.authorizing_old.new_email",
                    &[("email", &request.new_email)]
                ))
            ));
            (
                "confirm-old-email",
                t("user.change_email.authorizing_old.title"),
                body,
            )
        }
    };
    let success = match link {
        Link::New => t("user.change_email.confirm_success"),
        Link::Old => t("user.change_email.authorizing_old.confirm_success"),
    };
    let main = format!(
        "<div id=\"simple-container\"><div class=\"{class}\" data-confirm-success=\"{}\" data-account-path=\"{}\"><h2>{}</h2>{body}<button class=\"btn btn-text btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div></div>\
         <script type=\"application/json\" id=\"dialog-strings\">{}</script>",
        escape(&success),
        escape(&match guardian.user() {
            Some(me) => format!(
                "{}/u/{}/preferences/account",
                state.config.globals.relative_url_root(),
                me.username.to_lowercase()
            ),
            None => String::new(),
        }),
        escape(&title),
        escape(&t("user.change_email.confirm")),
        json!({
            "ok_value": t("ok_value"),
            "cancel": t("cancel_value"),
            "modal.close": t("modal.close"),
        })
        .to_string()
        .replace('<', "\\u003c"),
    );
    let base_path = state.config.globals.relative_url_root().to_string();
    let vs = super::session::viewer_state(state, headers, s, guardian)?;
    let mut site = crate::html::Site::from_settings(s, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(&mut *conn, state, s, guardian, crate::sidebar::Active::None)
        .await?;
    site.bus_position = crate::bus::page_position(&state.bus).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: s,
    };
    let page = super::users::ProfilePage {
        site_title: site.site_title,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        lang: site.lang,
        base_path: site.base_path,
        crawler: Crawler::for_request(&urls, uri, None)?,
        page_title: match link {
            Link::New => t("user.change_email.title"),
            Link::Old => String::new(),
        },
        main,
    };
    let response = (
        [(header::CACHE_CONTROL, "no-cache, no-store")],
        Html(page.render().map_err(crate::html::HtmlError::from)?),
    )
        .into_response();
    Ok(crate::html::with_viewer_headers(response, &vs))
}

/// The routes' extractors over [show_confirm] and [confirm].
pub async fn show_confirm_old(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Path(token): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    show_confirm(state, incoming, Link::Old, token, headers, uri).await
}

pub async fn show_confirm_new(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<Incoming>,
    Path(token): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    show_confirm(state, incoming, Link::New, token, headers, uri).await
}

pub async fn confirm_old(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(token): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    confirm(state, guardian, Link::Old, token, headers, uri, body).await
}

pub async fn confirm_new(
    state: State<AppState>,
    guardian: AuthGuardian,
    Path(token): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    confirm(state, guardian, Link::New, token, headers, uri, body).await
}
