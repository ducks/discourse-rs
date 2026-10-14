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

/// A param as ActiveModel's `:integer` attribute casts it.
fn integer_param(p: &Map<String, Value>, key: &str) -> Option<i64> {
    match p.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        v => params::scalar(v).and_then(|s| user_penalties::cast_integer(&s)),
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
        other_user_ids: user_penalties::cast_array(p.get("other_user_ids"))?,
        post_id: integer_param(&p, "post_id"),
        post_action: text("post_action"),
        post_edit: text("post_edit"),
        reviewable_id: integer_param(&p, "reviewable_id"),
    };
    let outcome = match action {
        Action::Suspend => {
            user_penalties::suspend(&state.pool, &ctx, &guardian, user_id, &penalty).await?
        }
        Action::Unsuspend => {
            let mut tx = state.pool.begin().await?;
            let outcome = user_penalties::unsuspend(&mut tx, &guardian, user_id).await?;
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
        Outcome::Conflict(message) => (
            StatusCode::CONFLICT,
            Json(json!({ "failed": "FAILED", "message": message })),
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

/// DELETE /admin/users/:id(.json): Admin::UsersController#destroy, for
/// staff. `{deleted: true}`, or a 403 with the posts' count when the user
/// still has posts and they were not to be deleted.
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    // User.find_by(id: params[:id].to_i)
    let id = crate::ruby::to_i(id.strip_suffix(".json").unwrap_or(&id));
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let target = match i32::try_from(id) {
        Ok(id) => crate::guardian::DeletableUser::load(&mut tx, id).await?,
        Err(_) => None,
    };
    // guardian.ensure_can_delete_user!(user)
    let Some(target) = target else {
        return Ok(super::search::invalid_access(&state));
    };
    if !guardian
        .can_delete_user(&mut tx, &settings, &target)
        .await?
    {
        return Ok(super::search::invalid_access(&state));
    }
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let flag = |key: &str| crate::ruby::cast_boolean(scalar(key).as_deref()).unwrap_or(false);
    let opts = crate::user_destroyer::Options {
        delete_posts: flag("delete_posts"),
        // params.slice(:delete_as_spammer): the raw value, any string truthy.
        delete_as_spammer: p.get("delete_as_spammer").is_some_and(|v| !v.is_null()),
        block_email: flag("block_email"),
        block_urls: flag("block_urls"),
        block_ip: flag("block_ip"),
        prepare_for_destroy: true,
        quiet: false,
        context: scalar("context"),
        reviewable_id: None,
    };
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    match crate::user_destroyer::destroy(&mut tx, &ctx, &guardian, target.id, &opts).await? {
        crate::user_destroyer::Outcome::Destroyed => {
            tx.commit().await?;
            Ok(Json(json!({ "deleted": true })).into_response())
        }
        crate::user_destroyer::Outcome::PostsExist => {
            let count = crate::user_destroyer::live_post_count(&mut tx, target.id).await?;
            let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
                .bind(target.id)
                .fetch_one(&mut *tx)
                .await?;
            let form = if count == 1 { "one" } else { "other" };
            let n = count.to_string();
            let message = state
                .i18n
                .t_with(
                    &format!("user.cannot_delete_has_posts.{form}"),
                    &[("username", &username), ("count", &n)],
                )
                .unwrap_or_default();
            Ok((
                StatusCode::FORBIDDEN,
                Json(json!({ "deleted": false, "message": message })),
            )
                .into_response())
        }
        crate::user_destroyer::Outcome::Forbidden => Ok(super::search::invalid_access(&state)),
    }
}

/// DELETE /admin/users/destroy-bulk(.json): Admin::UsersController
/// #destroy_bulk (User::BulkDestroy), for staff: 1 to 100 users, each
/// deletable, deleted in the order given with their posts, the progress
/// published to the actor (/bulk-user-delete).
pub async fn destroy_bulk(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let failed = |status: StatusCode, errors: Option<Vec<String>>| {
        let mut body = json!({ "failed": "FAILED" });
        if let Some(errors) = errors {
            body["errors"] = json!(errors);
        }
        (status, Json(body)).into_response()
    };
    // attribute :user_ids, :array, compact_blank: true
    let ids: Vec<String> = match p.get("user_ids") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(params::scalar)
            .filter(|s| !s.trim().is_empty())
            .collect(),
        Some(Value::Object(_)) => {
            return Err(crate::Unsupported("destroy-bulk user_ids as a hash").into());
        }
        Some(v) => params::scalar(v)
            .filter(|s| !s.trim().is_empty())
            .into_iter()
            .collect(),
        None => Vec::new(),
    };
    if ids.is_empty() || ids.len() > 100 {
        let message = if ids.is_empty() {
            "User ids is too short (minimum is 1 character)"
        } else {
            "User ids is too long (maximum is 100 characters)"
        };
        return Ok(failed(StatusCode::BAD_REQUEST, Some(vec![message.into()])));
    }
    let ids: Vec<i32> = ids
        .iter()
        .filter_map(|s| i32::try_from(crate::ruby::to_i(s)).ok())
        .collect();
    let block = crate::ruby::cast_boolean(
        p.get("block_ip_and_email")
            .and_then(params::scalar)
            .as_deref(),
    )
    .unwrap_or(false);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // The users in the order of the ids given.
    let users: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT id, username, host(ip_address) FROM users WHERE id = ANY($1) \
         ORDER BY array_position($1, id)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    if users.is_empty() {
        return Ok(failed(StatusCode::NOT_FOUND, None));
    }
    for (id, ..) in &users {
        let deletable = crate::guardian::DeletableUser::load(&mut conn, *id)
            .await?
            .ok_or(crate::Unsupported("a user gone while deleting in bulk"))?;
        if !guardian
            .can_delete_user(&mut conn, &settings, &deletable)
            .await?
        {
            let message = state.i18n.t("user.cannot_bulk_delete").unwrap_or_default();
            return Ok(failed(StatusCode::FORBIDDEN, Some(vec![message.into()])));
        }
    }
    let actor = guardian
        .user()
        .ok_or(crate::Unsupported("deleting users anonymously"))?
        .clone();
    let actor_ip: Option<String> =
        sqlx::query_scalar("SELECT host(ip_address) FROM users WHERE id = $1")
            .bind(actor.id)
            .fetch_one(&mut *conn)
            .await?;
    drop(conn);
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let context = ctx.t("staff_action_logs.bulk_user_delete");
    let total = users.len();
    for (position, (id, username, ip)) in users.iter().enumerate() {
        let opts = crate::user_destroyer::Options {
            delete_posts: true,
            prepare_for_destroy: true,
            context: Some(context.clone()),
            block_ip: block && actor_ip != *ip,
            block_email: block,
            ..Default::default()
        };
        let mut tx = state.pool.begin().await?;
        let outcome = crate::user_destroyer::destroy(&mut tx, &ctx, &guardian, *id, &opts).await?;
        let result = if outcome == crate::user_destroyer::Outcome::Destroyed {
            json!({ "success": true })
        } else {
            // UserDestroyer raised (InvalidAccess): reported, the rest go on.
            json!({ "failed": true, "error": "Discourse::InvalidAccess" })
        };
        let mut data = json!({ "position": position + 1, "username": username, "total": total });
        if let (Some(data), Some(result)) = (data.as_object_mut(), result.as_object()) {
            data.extend(result.clone());
        }
        state
            .bus
            .publish(
                &mut tx,
                "/bulk-user-delete",
                &data,
                Some(&[crate::bus::user_tag(actor.id)]),
            )
            .await?;
        tx.commit().await?;
    }
    Ok(Json(json!({ "deleted": true })).into_response())
}

/// DELETE /admin/users/delete-others-with-same-ip(.json):
/// Admin::UsersController#delete_other_accounts_with_same_ip, for staff:
/// up to 50 other users on the user's last (or, with ip_type
/// registration, registration) IP, highest trust level first, deleted as
/// spammers with their posts, emails, links and IPs blocked.
pub async fn delete_others_with_same_ip(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    // params.require(:user_id)
    let Some(user_id) = p
        .get("user_id")
        .and_then(params::scalar)
        .filter(|s| !s.is_empty())
    else {
        return Ok(super::accounts::param_missing("user_id"));
    };
    let column = match p.get("ip_type").and_then(params::scalar).as_deref() {
        None | Some("") | Some("last") => "ip_address",
        Some("registration") => "registration_ip_address",
        // SAME_IP_ADDRESS_COLUMNS.fetch raises KeyError.
        Some(_) => return Err(crate::Unsupported("an unknown ip_type (KeyError)").into()),
    };
    let target_id = i32::try_from(crate::ruby::to_i(&user_id)).unwrap_or(0);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let target: Option<(String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT username, host({column}) FROM users WHERE id = $1"
    ))
    .bind(target_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((target_username, ip)) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    // find_users(50): same IP (none without one), the user excluded,
    // ordered by trust level.
    let ids: Vec<i32> = match &ip {
        Some(ip) => {
            sqlx::query_scalar(
                "SELECT id FROM users WHERE (ip_address = $1::inet OR registration_ip_address = $1::inet) \
                   AND id <> $2 ORDER BY trust_level DESC NULLS LAST, id LIMIT 50",
            )
            .bind(ip)
            .bind(target_id)
            .fetch_all(&mut *conn)
            .await?
        }
        None => Vec::new(),
    };
    let context = match &ip {
        Some(ip) if crate::admin_users::can_see_ip(&settings, &guardian)? => state.i18n.t_with(
            "user.destroy_reasons.same_ip_address",
            &[("ip_address", ip)],
        ),
        _ => state.i18n.t_with(
            "user.destroy_reasons.same_ip_address_user",
            &[("username", &target_username)],
        ),
    }
    .unwrap_or_default();
    drop(conn);
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let opts = crate::user_destroyer::Options {
        delete_posts: true,
        block_email: true,
        block_urls: true,
        block_ip: true,
        delete_as_spammer: true,
        context: Some(context),
        ..Default::default()
    };
    for id in ids {
        let mut tx = state.pool.begin().await?;
        match crate::user_destroyer::destroy(&mut tx, &ctx, &guardian, id, &opts).await? {
            crate::user_destroyer::Outcome::Destroyed => tx.commit().await?,
            // InvalidAccess, the users before it deleted.
            _ => return Ok(super::search::invalid_access(&state)),
        }
    }
    Ok(Json(json!({ "success": "OK" })).into_response())
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
            super::accounts::confirm_email_token(&state, &mut tx, &settings, &token, signup)
                .await?;
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
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let promotion = crate::promotion::Ctx {
        settings: &settings,
        i18n: &state.i18n,
        host: &host,
    };
    let cx = crate::group_manager::Cx {
        state: &state,
        promotion: &promotion,
    };
    if method == "POST" {
        crate::group_manager::add(&mut tx, &cx, &group, user_id, acting).await?;
    } else {
        crate::group_manager::remove(&mut tx, &cx, &group, user_id, acting).await?;
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
            crate::badge_granter::check_if_title_is_badged_granted(
                &mut tx,
                &state.i18n,
                user_id,
                title.as_deref(),
            )
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

/// The `:user_id` of an admin user route, `.json` and all.
fn route_user_id(user_id: &str) -> Option<i32> {
    user_id
        .strip_suffix(".json")
        .unwrap_or(user_id)
        .parse::<i32>()
        .ok()
}

/// PUT /admin/users/:id/anonymize: Admin::UsersController#anonymize.
pub async fn anonymize(
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
    let Some(user_id) = route_user_id(&user_id) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Ok(super::topics::not_found_response(&state));
    }
    if !crate::user_anonymizer::can_anonymize(&mut tx, user_id).await? {
        return Ok(super::search::invalid_access(&state));
    }
    let anonymize_ip = p
        .get("anonymize_ip")
        .and_then(params::scalar)
        .filter(|v| !crate::ruby::is_blank(v));
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let username = crate::user_anonymizer::make_anonymous(
        &mut tx,
        &ctx,
        user_id,
        guardian.user_id(),
        anonymize_ip.as_deref(),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK", "username": username })).into_response())
}

/// POST /admin/users/:id/merge: Admin::UsersController#merge, which
/// enqueues the merge.
pub async fn merge(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "POST") {
        return Ok(bad_csrf());
    }
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(user_id) = route_user_id(&user_id) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut tx = state.pool.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Ok(super::topics::not_found_response(&state));
    }
    // params.require(:target_username)
    let target_username = match p.get("target_username") {
        None => None,
        Some(v) => Some(
            params::scalar(v)
                .ok_or(crate::Unsupported("a target_username that is not a string"))?,
        ),
    };
    let Some(target_username) = target_username.filter(|v| !crate::ruby::is_blank(v)) else {
        return Ok(super::accounts::param_missing("target_username"));
    };
    let target: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
        .bind(crate::accounts::normalize_username(&target_username)?)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(target) = target else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !crate::user_merger::can_merge_user(&mut tx, &guardian, user_id).await? {
        return Ok(super::search::invalid_access(&state));
    }
    crate::jobs::enqueue(
        &mut tx,
        crate::user_merger::MERGE_USER,
        json!({
            "user_id": user_id,
            "target_user_id": target,
            "current_user_id": guardian.user_id(),
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// PUT /admin/users/suspend-bulk(.json): Admin::UsersController#suspend_bulk
/// (User::BulkSuspend): each user suspended in the order given, the
/// progress published to the actor on /bulk-user-suspend.
pub async fn suspend_bulk(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
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
    let failed = |status: StatusCode, errors: Option<Vec<String>>| {
        let mut body = json!({ "failed": "FAILED" });
        if let Some(errors) = errors {
            body["errors"] = json!(errors);
        }
        (status, Json(body)).into_response()
    };
    // attribute :user_ids, :array, compact_blank: true
    let ids: Vec<user_penalties::ArrayItem> = user_penalties::cast_array(p.get("user_ids"))?
        .into_iter()
        .filter(|item| !item.is_blank())
        .collect();
    let text = |k: &str| p.get(k).and_then(params::scalar);
    let (reason, until, message) = (text("reason"), text("suspend_until"), text("message"));
    let until_at = user_penalties::cast_time(until.as_deref())?;
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let host = Host::from_state(&state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut errors = Vec::new();
    if ids.is_empty() {
        errors.push("User ids is too short (minimum is 1 character)".to_string());
    } else if ids.len() > 100 {
        errors.push("User ids is too long (maximum is 100 characters)".to_string());
    }
    errors.extend(user_penalties::contract_errors(
        &ctx,
        reason.as_deref(),
        "Suspend until",
        until_at,
    ));
    if !errors.is_empty() {
        return Ok(failed(StatusCode::BAD_REQUEST, Some(errors)));
    }
    let ids: Vec<i32> = ids
        .iter()
        .map(|item| match item {
            user_penalties::ArrayItem::Int(n) => i32::try_from(*n).ok(),
            // array_position(ARRAY['x'], users.id) fails in Postgres.
            user_penalties::ArrayItem::Text(_) => None,
        })
        .collect::<Option<_>>()
        .ok_or(crate::Unsupported(
            "suspend-bulk user_ids that are not integers",
        ))?;
    let users: Vec<(i32, String, bool, bool, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT id, username, admin, moderator, suspended_till FROM users WHERE id = ANY($1) \
         ORDER BY array_position($1, users.id)",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    if users.is_empty() {
        return Ok(failed(StatusCode::NOT_FOUND, None));
    }
    // can_suspend_users: can_suspend? && !suspended?
    let now = crate::clock::now_naive();
    let suspendable = users.iter().all(|(_, _, admin, moderator, till)| {
        let staff = *admin || *moderator;
        guardian.is_staff() && !staff && !till.is_some_and(|t| t > now)
    });
    if !suspendable {
        let message = state.i18n.t("user.cannot_bulk_suspend").unwrap_or_default();
        return Ok(failed(StatusCode::FORBIDDEN, Some(vec![message.into()])));
    }
    let actor = guardian
        .user()
        .ok_or(crate::Unsupported("suspending users anonymously"))?
        .clone();
    let suspension = user_penalties::Suspension {
        actor_id: actor.id,
        until: until_at.unwrap_or_default(),
        reason: reason.as_deref().unwrap_or_default(),
        message: message.as_deref(),
        post_id: None,
        reviewable_id: None,
    };
    let total = users.len();
    for (position, (id, username, ..)) in users.iter().enumerate() {
        // User::Action::SuspendAndPublish
        user_penalties::suspend_user(&state.pool, *id, &suspension).await?;
        let data = json!({ "position": position + 1, "username": username, "total": total, "success": true });
        let mut conn = state.pool.acquire().await?;
        state
            .bus
            .publish(
                &mut conn,
                "/bulk-user-suspend",
                &data,
                Some(&[crate::bus::user_tag(actor.id)]),
            )
            .await?;
    }
    Ok(Json(json!({ "suspended": true })).into_response())
}

/// DELETE /admin/users/:user_id/penalty_history(.json): the user's
/// suspensions and silences relabeled as removed, so TL3 promotion no
/// longer counts them (they stay in the logs). Admins only; no user
/// lookup, so any id answers success.
pub async fn penalty_history(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    if !csrf_ok(&state, &headers, &form_pairs(&p), uri.path(), "DELETE") {
        return Ok(bad_csrf());
    }
    // AdminConstraint: anyone else misses the route.
    if !guardian.is_admin() {
        return Ok(super::topics::not_found_response(&state));
    }
    let user_id = crate::ruby::to_i(user_id.strip_suffix(".json").unwrap_or(&user_id));
    let Ok(user_id) = i32::try_from(user_id) else {
        return Err(crate::Unsupported("a user id out of range").into());
    };
    sqlx::query(
        "UPDATE user_histories SET action = CASE \
           WHEN action = 30 THEN 60 \
           WHEN action = 31 THEN 62 \
           WHEN action = 10 THEN 61 \
           WHEN action = 11 THEN 63 \
         END \
         WHERE target_user_id = $1 AND action IN (30, 10, 31, 11)",
    )
    .bind(user_id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// GET /admin/users/:user_id/similar-users.json: up to ten other real,
/// non-staff users on the user's last IP, as SimilarAdminUserSerializer.
pub async fn similar_users(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(user_id): Path<String>,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let Some(user_id) = crate::ruby::integer(&user_id).and_then(|n| i32::try_from(n).ok()) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let ip: Option<Option<String>> =
        sqlx::query_scalar("SELECT host(ip_address) FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(ip) = ip else {
        return Ok(super::topics::not_found_response(&state));
    };
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let mut out = Vec::new();
    if ip.is_some() {
        // No ORDER BY in Rails: heap order (the cases compare it unordered).
        let rows: Vec<crate::admin_users::Row> = sqlx::query_as(&format!(
            "{} WHERE users.id > 0 \
               AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
               AND users.id <> $1 \
               AND users.ip_address = (SELECT ip_address FROM users WHERE id = $1) \
               AND NOT users.admin AND NOT users.moderator \
             ORDER BY users.id LIMIT {}",
            crate::admin_users::ROW_SELECT,
            user_penalties::MAX_SIMILAR_USERS
        ))
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        let logo_small_url = crate::admin_users::logo_small_url(&mut conn, &settings).await?;
        let opts = crate::admin_users::EntryOptions {
            emails_desired: false,
            can_be_deleted: false,
            can_be_suspended: false,
            silence_reason: None,
            suspend_reason: None,
        };
        for r in &rows {
            let mut u = crate::admin_users::entry(
                &mut conn,
                &settings,
                &urls,
                &guardian,
                r,
                logo_small_url.as_deref(),
                &opts,
            )
            .await?;
            // can_suspend? and can_silence_user?: staff on a regular user
            // (none of these is staff).
            u.insert("can_be_suspended".into(), json!(guardian.is_staff()));
            u.insert("can_be_silenced".into(), json!(guardian.is_staff()));
            out.push(Value::Object(u));
        }
    }
    Ok(Json(json!({ "users": out })).into_response())
}
