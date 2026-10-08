//! Port of Admin::UsersController's penalties: PUT
//! /admin/users/:user_id/{suspend,unsuspend,silence,unsilence}(.json).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use super::session::{bad_csrf, csrf_ok};
use crate::params;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::user_penalties::{self, Outcome, Penalty};
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

/// Which penalty a route applies.
#[derive(Clone, Copy)]
enum Action {
    Suspend,
    Unsuspend,
    Silence,
    Unsilence,
}

/// The params the services take that are not ported.
fn unported(p: &Map<String, Value>) -> Option<&'static str> {
    let present = |k: &str| {
        p.get(k)
            .is_some_and(|v| !v.is_null() && params::scalar(v).is_none_or(|s| !s.is_empty()))
    };
    if present("other_user_ids") {
        Some("penalizing several users at once (other_user_ids)")
    } else if present("post_id") || present("post_action") || present("post_edit") {
        Some("acting on a post with a penalty")
    } else if present("reviewable_id") {
        Some("penalties from the review queue")
    } else {
        None
    }
}

async fn handle(
    state: AppState,
    guardian: crate::guardian::Guardian,
    user_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: Action,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    // The admin namespace is routed for staff only.
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let user_id = crate::ruby::to_i(user_id.strip_suffix(".json").unwrap_or(&user_id)) as i32;
    let mut conn = state.pool.acquire().await?;
    // The controller's fetch_user before_action, ahead of the service's
    // params contract.
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(super::topics::not_found_response(&state));
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let until_param = match action {
        Action::Silence => "silenced_till",
        _ => "suspend_until",
    };
    let (reason, message, until) = (text("reason"), text("message"), text(until_param));
    let penalty = Penalty {
        reason: reason.as_deref(),
        message: message.as_deref(),
        until: until.as_deref(),
        unported: unported(&p),
    };
    // Suspending is one transaction; silencing sends a system message,
    // which PostCreator commits on its own.
    let outcome = match action {
        Action::Suspend | Action::Unsuspend => {
            let mut tx = state.pool.begin().await?;
            let outcome = match action {
                Action::Suspend => {
                    user_penalties::suspend(&mut tx, &ctx, &guardian, user_id, &penalty).await?
                }
                _ => user_penalties::unsuspend(&mut tx, &guardian, user_id).await?,
            };
            if matches!(outcome, Outcome::Done(_)) {
                tx.commit().await?;
            }
            outcome
        }
        Action::Silence => {
            user_penalties::silence(&state.pool, &ctx, &guardian, user_id, &penalty).await?
        }
        Action::Unsilence => {
            user_penalties::unsilence(&state.pool, &ctx, &guardian, user_id).await?
        }
    };
    Ok(match outcome {
        Outcome::Done(body) => (StatusCode::OK, Json(body)).into_response(),
        Outcome::NotFound => super::topics::not_found_response(&state),
        Outcome::Forbidden => super::search::invalid_access(&state),
        Outcome::Invalid(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "failed": "FAILED", "errors": errors })),
        )
            .into_response(),
    })
}

/// PUT /admin/users/:user_id/suspend
pub async fn suspend(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Suspend,
    )
    .await
}

/// PUT /admin/users/:user_id/unsuspend
pub async fn unsuspend(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Unsuspend,
    )
    .await
}

/// PUT /admin/users/:user_id/silence
pub async fn silence(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Silence,
    )
    .await
}

/// PUT /admin/users/:user_id/unsilence
pub async fn unsilence(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    handle(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        Action::Unsilence,
    )
    .await
}

/// GET /admin/users/list(/:query)(.json): Admin::UsersController#index,
/// for staff (StaffConstraint).
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    path: Option<Path<String>>,
    uri: Uri,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let q: Vec<(String, String)> = form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    let get = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let p = crate::admin_users::ListParams {
        query: path.map(|Path(s)| s.strip_suffix(".json").unwrap_or(&s).to_string()),
        order: get("order"),
        asc: get("asc").is_some_and(|v| !v.is_empty()),
        page: get("page").map(|v| crate::ruby::to_i(&v)).unwrap_or(0),
        show_emails: get("show_emails").as_deref() == Some("true"),
        email: get("email"),
        filter: get("filter"),
        ip: get("ip"),
        same_ip_user_id: get("same_ip_user_id"),
        ip_type: get("ip_type"),
        exclude: get("exclude"),
        account_type: get("account_type"),
        activation: get("activation"),
    };
    if get("stats").as_deref() == Some("false") {
        return Err(crate::Unsupported("admin user lists without stats").into());
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    Ok(
        match crate::admin_users::list(&mut conn, &settings, &urls, &guardian, &p, uri.path())
            .await?
        {
            crate::admin_users::Listed::Users(users) => Json(Value::Array(users)).into_response(),
            crate::admin_users::Listed::InvalidFilter => {
                super::search::invalid_parameters(&state, "filter")
            }
        },
    )
}

/// GET /admin/users/:id(.json): Admin::UsersController#show, for staff.
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let id = id.strip_suffix(".json").unwrap_or(&id);
    // User.find_by(id:): a non-numeric id finds nobody.
    let Ok(id) = id.parse::<i32>() else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = crate::admin_user_show::Context {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    Ok(
        match crate::admin_user_show::show(
            &mut conn,
            &cx,
            &guardian,
            id,
            crate::admin_user_show::ShowOptions {
                show: true,
                detailed: true,
            },
        )
        .await?
        {
            Some(user) => Json(user).into_response(),
            None => super::topics::not_found_response(&state),
        },
    )
}

/// Which role a route grants or revokes.
#[derive(Clone, Copy)]
enum RoleChange {
    GrantModeration,
    RevokeModeration,
    RevokeAdmin,
}

/// PUT /admin/users/:id/grant_moderation
pub async fn grant_moderation(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    change_role(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        RoleChange::GrantModeration,
    )
    .await
}

/// PUT /admin/users/:id/revoke_moderation
pub async fn revoke_moderation(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    change_role(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        RoleChange::RevokeModeration,
    )
    .await
}

/// PUT /admin/users/:id/revoke_admin
pub async fn revoke_admin(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    change_role(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        RoleChange::RevokeAdmin,
    )
    .await
}

/// Admin::UsersController#grant_moderation, #revoke_moderation and
/// #revoke_admin: for admins (AdminConstraint), the guardian's check, the
/// change, the staff log, then the user's AdminDetailedUserSerializer.
async fn change_role(
    state: AppState,
    guardian: crate::guardian::Guardian,
    user_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    change: RoleChange,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Ok(user_id) = user_id
        .strip_suffix(".json")
        .unwrap_or(&user_id)
        .parse::<i32>()
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let target: Option<(bool, bool)> =
        sqlx::query_as("SELECT admin, moderator FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((admin, moderator)) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    // can_administer?: a real user; can_administer_user?: not oneself.
    let administer = user_id > 0;
    let allowed = match change {
        RoleChange::GrantModeration => administer && !moderator,
        RoleChange::RevokeModeration => administer && moderator,
        RoleChange::RevokeAdmin => administer && guardian.user_id() != Some(user_id) && admin,
    };
    if !allowed {
        return Ok(super::search::invalid_access(&state));
    }
    let acting = guardian.user_id().unwrap_or_default();
    match change {
        RoleChange::GrantModeration => {
            crate::roles::grant_moderation(&mut conn, &state.i18n, user_id).await?;
            crate::roles::log(
                &mut conn,
                acting,
                user_id,
                crate::roles::LOG_GRANT_MODERATION,
            )
            .await?;
        }
        RoleChange::RevokeModeration => {
            crate::roles::revoke(
                &mut conn,
                &state.i18n,
                user_id,
                crate::roles::Permission::Moderator,
            )
            .await?;
            crate::roles::log(&mut conn, acting, user_id, crate::roles::REVOKE_MODERATION).await?;
        }
        RoleChange::RevokeAdmin => {
            crate::roles::revoke(
                &mut conn,
                &state.i18n,
                user_id,
                crate::roles::Permission::Admin,
            )
            .await?;
            crate::roles::log(&mut conn, acting, user_id, crate::roles::REVOKE_ADMIN).await?;
        }
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let cx = crate::admin_user_show::Context {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    Ok(
        match crate::admin_user_show::show(
            &mut conn,
            &cx,
            &guardian,
            user_id,
            crate::admin_user_show::ShowOptions {
                show: false,
                detailed: true,
            },
        )
        .await?
        {
            Some(user) => Json(user).into_response(),
            None => super::topics::not_found_response(&state),
        },
    )
}

/// What the trust level routes act on.
#[derive(Clone, Copy)]
enum TrustLevelAction {
    Change,
    Lock,
}

/// PUT /admin/users/:id/trust_level
pub async fn trust_level(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    trust_level_route(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        TrustLevelAction::Change,
    )
    .await
}

/// PUT /admin/users/:id/trust_level_lock
pub async fn trust_level_lock(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    trust_level_route(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        TrustLevelAction::Lock,
    )
    .await
}

/// Admin::UsersController#trust_level and #trust_level_lock: for staff
/// (StaffConstraint), guardian.can_change_trust_level?, then the change
/// (AdminUserSerializer) or the lock and a recalculation (no body).
async fn trust_level_route(
    state: AppState,
    guardian: crate::guardian::Guardian,
    user_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: TrustLevelAction,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "PUT") {
        return Ok(bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Ok(user_id) = user_id
        .strip_suffix(".json")
        .unwrap_or(&user_id)
        .parse::<i32>()
    else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    let target: Option<(bool, bool, Option<i32>)> = sqlx::query_as(
        "SELECT admin, moderator, manual_locked_trust_level FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((admin, moderator, lock)) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let can_change = guardian.is_admin()
        || (guardian.is_moderator()
            && settings.get("moderators_change_trust_levels")?.truthy()
            && !(admin || moderator));
    let host = Host::from_state(&state);
    let cx = crate::promotion::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        host: &host,
    };
    let acting = guardian.user_id().unwrap_or_default();
    let s = &settings;
    match action {
        TrustLevelAction::Change => {
            // The action rescues the guardian's InvalidAccess into a 422.
            if !can_change {
                return Ok(json_error(vec!["can_change_trust_level? failed".into()]));
            }
            let level = p
                .get("level")
                .and_then(params::scalar)
                .map_or(0, |l| crate::ruby::to_i(&l)) as i32;
            if lock.is_none() {
                let lock_it = if (0..=2).contains(&level) {
                    crate::promotion::tl_met(&mut tx, s, user_id, level + 1).await? == Some(true)
                } else {
                    level == 3 && crate::promotion::tl3_lost(&mut tx, s, user_id).await?
                };
                if lock_it {
                    crate::promotion::save_lock(&mut tx, s, user_id, Some(level)).await?;
                }
            }
            if let Err(message) =
                crate::promotion::change_trust_level(&mut tx, &cx, user_id, level, Some(acting))
                    .await?
            {
                return Ok(json_error(vec![message]));
            }
        }
        TrustLevelAction::Lock => {
            if !can_change {
                return Ok(super::search::invalid_access(&state));
            }
            let locked = p.get("locked").and_then(params::scalar).unwrap_or_default();
            if !locked.contains("true") && !locked.contains("false") {
                return Ok(json_error(vec![
                    "Translation missing: en.errors.invalid_boolean".into(),
                ]));
            }
            let trust_level: i32 =
                sqlx::query_scalar("SELECT trust_level FROM users WHERE id = $1")
                    .bind(user_id)
                    .fetch_one(&mut *tx)
                    .await?;
            let new_lock = (locked == "true").then_some(trust_level);
            crate::promotion::save_lock(&mut tx, s, user_id, new_lock).await?;
            crate::promotion::log_lock(&mut tx, acting, user_id, new_lock.is_some()).await?;
            crate::promotion::recalculate(&mut tx, &cx, user_id, Some(acting)).await?;
            tx.commit().await?;
            return Ok(StatusCode::OK.into_response());
        }
    }
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let show_cx = crate::admin_user_show::Context {
        settings: &settings,
        defs: &state.site_setting_defs,
        i18n: &state.i18n,
        urls: &urls,
        globals: &state.config.globals,
        development: state.config.rails_env == crate::config::RailsEnv::Development,
    };
    let user = crate::admin_user_show::show(
        &mut tx,
        &show_cx,
        &guardian,
        user_id,
        crate::admin_user_show::ShowOptions {
            show: false,
            detailed: false,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "admin_user": user })).into_response())
}

fn json_error(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}
