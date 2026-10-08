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

/// What the activation routes do.
#[derive(Clone, Copy, PartialEq)]
enum AccountAction {
    LogOut,
    Activate,
    Deactivate,
    Approve,
}

/// PUT /admin/users/:id/approve
pub async fn approve(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    account_route(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        AccountAction::Approve,
    )
    .await
}

/// POST /admin/users/:id/log_out
pub async fn log_out(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    account_route(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        AccountAction::LogOut,
    )
    .await
}

/// PUT /admin/users/:id/activate
pub async fn activate(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    account_route(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        AccountAction::Activate,
    )
    .await
}

/// PUT /admin/users/:id/deactivate
pub async fn deactivate(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    account_route(
        state,
        guardian,
        user_id,
        headers,
        uri,
        body,
        AccountAction::Deactivate,
    )
    .await
}

/// UserHistory.actions
const DEACTIVATE_USER: i32 = 39;
const ACTIVATE_USER: i32 = 43;

/// Admin::UsersController#log_out (admins: AdminConstraint), #activate
/// and #deactivate (staff: StaffConstraint), each answering
/// `success_json`.
async fn account_route(
    state: AppState,
    guardian: crate::guardian::Guardian,
    user_id: String,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
    action: AccountAction,
) -> Result<Response, AppError> {
    let method = if action == AccountAction::LogOut {
        "POST"
    } else {
        "PUT"
    };
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), method) {
        return Ok(bad_csrf());
    }
    let allowed = if action == AccountAction::LogOut {
        guardian.is_admin()
    } else {
        guardian.is_staff()
    };
    if !allowed {
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
    #[derive(sqlx::FromRow)]
    struct Target {
        username_lower: String,
        name: Option<String>,
        admin: bool,
        moderator: bool,
        active: bool,
        approved: bool,
        email: Option<String>,
    }
    let target: Option<Target> = sqlx::query_as(
        "SELECT username_lower, name, admin, moderator, active, approved, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\") AS email \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(target) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let acting = guardian.user_id().unwrap_or_default();
    let log = |action: i32, key: &str| {
        let details = state.i18n.t(key).unwrap_or_default().to_string();
        (action, details)
    };
    let (history, details) = match action {
        AccountAction::Approve => {
            if !target.active || target.approved {
                return Ok(super::search::invalid_access(&state));
            }
            let Some(reviewable_id) =
                crate::reviewable_user::find_or_create(&mut tx, &settings, user_id).await?
            else {
                // Rails calls `.reviewable` on the job's nil: a 500.
                return Err(crate::Unsupported(
                    "approving a user without must_approve_users (NoMethodError in Rails)",
                )
                .into());
            };
            let username = guardian
                .user()
                .map(|u| u.username.clone())
                .unwrap_or_default();
            let performer = crate::reviewable_user::Performer {
                id: acting,
                username: &username,
            };
            let approved = crate::reviewable_user::approve(
                &mut tx,
                &settings,
                reviewable_id,
                user_id,
                &performer,
            )
            .await?;
            if !approved {
                return Ok(super::search::invalid_access(&state));
            }
            tx.commit().await?;
            return Ok(StatusCode::OK.into_response());
        }
        AccountAction::LogOut => {
            if settings.get("verbose_auth_token_logging")?.truthy() {
                return Err(crate::Unsupported("verbose auth token logging").into());
            }
            sqlx::query("DELETE FROM user_auth_tokens WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            // logged_out
            state
                .bus
                .publish(
                    &mut tx,
                    &format!("/logout/{user_id}"),
                    &json!(user_id),
                    Some(&[crate::bus::user_tag(user_id)]),
                )
                .await?;
            tx.commit().await?;
            return Ok(Json(json!({ "success": "OK" })).into_response());
        }
        AccountAction::Activate => {
            if target.active {
                return Ok(super::search::invalid_access(&state));
            }
            let Some(email) = target.email.as_deref() else {
                return Err(crate::Unsupported("activating a user without an email").into());
            };
            let valid_hours = settings.get("email_token_valid_hours")?.to_i();
            let active_token: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM email_tokens WHERE user_id = $1 AND NOT expired \
                   AND created_at >= now() - make_interval(hours => $2))",
            )
            .bind(user_id)
            .bind(valid_hours as i32)
            .fetch_one(&mut *tx)
            .await?;
            let signup = crate::accounts::token_scopes::SIGNUP;
            if !active_token {
                crate::accounts::create_email_token(&mut tx, user_id, email, signup).await?;
            }
            // User#activate
            let token =
                crate::accounts::create_email_token(&mut tx, user_id, email, signup).await?;
            super::accounts::confirm_email_token(&mut tx, &settings, &token, signup).await?;
            log(ACTIVATE_USER, "user.activated_by_staff")
        }
        AccountAction::Deactivate => {
            // can_deactivate? is can_suspend?: staff acting on a regular user.
            if target.admin || target.moderator {
                return Ok(super::search::invalid_access(&state));
            }
            let reviewable: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM reviewables WHERE type = 'ReviewableUser' \
                   AND target_type = 'User' AND target_id = $1 AND status = 0)",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            let shadow: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM anonymous_users WHERE master_user_id = $1 AND active)",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if reviewable || shadow {
                return Err(crate::Unsupported(
                    "deactivating a user with a pending review or anonymous shadows",
                )
                .into());
            }
            if target.active {
                sqlx::query(
                    "UPDATE users SET active = FALSE, updated_at = clock_timestamp() WHERE id = $1",
                )
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
                crate::user_updater::after_save(
                    &mut tx,
                    &settings,
                    user_id,
                    false,
                    &target.username_lower,
                    target.name.as_deref(),
                )
                .await?;
            }
            log(DEACTIVATE_USER, "user.deactivated_by_staff")
        }
    };
    let context = (action == AccountAction::Deactivate)
        .then(|| p.get("context").and_then(params::scalar))
        .flatten();
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, details, context, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(history)
    .bind(acting)
    .bind(user_id)
    .bind(details)
    .bind(context)
    .execute(&mut *tx)
    .await?;
    if action == AccountAction::Deactivate {
        // refresh_browser
        state
            .bus
            .publish(
                &mut tx,
                "/file-change",
                &json!(["refresh"]),
                Some(&[crate::bus::user_tag(user_id)]),
            )
            .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// POST /admin/users/:id/groups: Admin::UsersController#add_group.
pub async fn add_group(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let group_id = params::string(&p, "group_id").unwrap_or_default();
    membership(state, guardian, user_id, group_id, headers, uri, p, "POST").await
}

/// DELETE /admin/users/:id/groups/:group_id: #remove_group.
pub async fn remove_group(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((user_id, group_id)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let group_id = group_id
        .strip_suffix(".json")
        .unwrap_or(&group_id)
        .to_string();
    membership(
        state, guardian, user_id, group_id, headers, uri, p, "DELETE",
    )
    .await
}

/// Admins (AdminConstraint) adding a user to a group or removing them:
/// `Group.find(params[:group_id].to_i)`, automatic groups refused with
/// can_not_modify_automatic, then GroupManager and the group log; no body.
#[allow(clippy::too_many_arguments)]
async fn membership(
    state: AppState,
    guardian: crate::guardian::Guardian,
    user_id: String,
    group_id: String,
    headers: HeaderMap,
    uri: Uri,
    p: Map<String, Value>,
    method: &str,
) -> Result<Response, AppError> {
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), method) {
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
    let mut tx = state.pool.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    let group = crate::group_manager::find(&mut tx, crate::ruby::to_i(&group_id) as i32).await?;
    let (true, Some(group)) = (exists, group) else {
        return Ok(super::topics::not_found_response(&state));
    };
    if group.automatic {
        let message = state
            .i18n
            .t("groups.errors.can_not_modify_automatic")
            .unwrap_or_default();
        return Ok(json_error(vec![message.to_string()]));
    }
    let acting = guardian.user_id().unwrap_or_default();
    if method == "POST" {
        let settings =
            SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
        let host = Host::from_state(&state);
        let cx = crate::promotion::Ctx {
            settings: &settings,
            i18n: &state.i18n,
            host: &host,
        };
        crate::group_manager::add(&mut tx, &cx, &group, user_id, acting).await?;
    } else {
        crate::group_manager::remove(&mut tx, &group, user_id, acting).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::OK.into_response())
}

/// PUT /admin/users/:id/primary_group: staff setting a user's primary
/// group to one they belong to (`can_change_primary_group?`), or none.
pub async fn primary_group(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
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
    #[derive(sqlx::FromRow)]
    struct Target {
        username_lower: String,
        name: Option<String>,
        admin: bool,
        active: bool,
        primary_group_id: Option<i32>,
        flair_group_id: Option<i32>,
        title: Option<String>,
    }
    let target: Option<Target> = sqlx::query_as(
        "SELECT username_lower, name, admin, active, primary_group_id, flair_group_id, title \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(t) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let mut new_primary = t.primary_group_id;
    match params::string(&p, "primary_group_id").filter(|v| !crate::ruby::is_blank(v)) {
        None => new_primary = None,
        Some(id) => {
            let id = crate::ruby::to_i(&id) as i32;
            let group: Option<(bool, i32)> =
                sqlx::query_as("SELECT automatic, visibility_level FROM groups WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some((automatic, visibility)) = group else {
                return Ok(super::topics::not_found_response(&state));
            };
            // can_edit_group?: not automatic, and an admin or (with
            // moderators_manage_groups) a moderator who can see it.
            if !guardian.is_admin() && visibility == 4 {
                return Err(crate::Unsupported("moderators and owner-only visible groups").into());
            }
            let can_admin = guardian.is_admin()
                || (settings.get("moderators_manage_groups")?.truthy()
                    && guardian.is_moderator()
                    && id != 1);
            if automatic || !can_admin {
                return Ok(super::search::invalid_access(&state));
            }
            let member: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
            )
            .bind(id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if member {
                new_primary = Some(id);
            }
        }
    }
    if new_primary != t.primary_group_id {
        // match_primary_group_changes: the old primary group's title and
        // flair follow to the new one.
        let old_title_matches: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1 AND title IS NOT DISTINCT FROM $2)",
        )
        .bind(t.primary_group_id)
        .bind(&t.title)
        .fetch_one(&mut *tx)
        .await?;
        let mut title = t.title.clone();
        if old_title_matches {
            title = match new_primary {
                Some(id) => {
                    sqlx::query_scalar("SELECT title FROM groups WHERE id = $1")
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await?
                }
                None => None,
            };
        }
        let flair = if t.flair_group_id == t.primary_group_id {
            new_primary
        } else {
            t.flair_group_id
        };
        if title != t.title {
            // check_if_title_is_badged_granted
            let title_badges: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
                 WHERE ub.user_id = $1 AND b.allow_title)",
            )
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
            if title_badges {
                return Err(crate::Unsupported("titles granted by badges").into());
            }
            sqlx::query(
                "UPDATE user_profiles SET granted_title_badge_id = NULL \
                 WHERE user_id = $1 AND granted_title_badge_id IS NOT NULL",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE users SET primary_group_id = $2, title = $3, flair_group_id = $4, \
                              updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(user_id)
        .bind(new_primary)
        .bind(&title)
        .bind(flair)
        .execute(&mut *tx)
        .await?;
        crate::user_updater::after_save(
            &mut tx,
            &settings,
            user_id,
            t.admin && t.active,
            &t.username_lower,
            t.name.as_deref(),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::OK.into_response())
}
