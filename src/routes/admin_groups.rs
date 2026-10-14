//! Creating, editing and deleting groups and managing their members:
//! Admin::GroupsController (create, destroy, remove_owner, set_primary,
//! automatic_membership_count) and GroupsController's writes (update,
//! add_members, remove_member, add_owners).
//!
//! Refused: adding members by email who are not users (invites), group
//! SMTP settings, flair uploads, plugin custom fields and associated
//! groups, and the 500s Rails answers
//! (an unknown user id for remove_owner, set_primary without usernames).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::group_manager::{self, ADD_USER_TO_GROUP, REMOVE_USER_FROM_GROUP};
use crate::group_writes::{self as gw, Kind};
use crate::guardian::Guardian;
use crate::params;
use crate::pretty_text::Host;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `GroupHistory.actions`
const MAKE_USER_GROUP_OWNER: i32 = 4;
const REMOVE_USER_AS_GROUP_OWNER: i32 = 5;
/// `UserHistory.actions`
const DELETE_GROUP: i32 = 99;
const CREATE_GROUP: i32 = 132;
/// `GroupsController::ADD_MEMBERS_LIMIT`
const ADD_MEMBERS_LIMIT: usize = 1000;

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

fn json_error(errors: Vec<String>) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "errors": errors })),
    )
        .into_response()
}

fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

fn can_not_modify_automatic(state: &AppState) -> Response {
    json_error(vec![
        state
            .i18n
            .t("groups.errors.can_not_modify_automatic")
            .unwrap_or_default()
            .to_string(),
    ])
}

fn strip_json(s: &str) -> &str {
    s.strip_suffix(".json").unwrap_or(s)
}

/// `params.require(key)`: a present, non-empty value.
fn required<'a>(p: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    p.get(key).filter(|v| match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Object(m) => !m.is_empty(),
        Value::Array(a) => !a.is_empty(),
        _ => true,
    })
}

/// What a group write needs.
struct Req {
    p: Map<String, Value>,
    settings: SiteSettings,
    host: Host,
}

async fn begin(
    state: &AppState,
    headers: &HeaderMap,
    uri: &Uri,
    body: &Bytes,
    method: &str,
) -> Result<Result<Req, Response>, AppError> {
    let p = params::parse(uri.query(), headers, body);
    if !csrf_ok(state, headers, &form_pairs(&p), uri.path(), method) {
        return Ok(Err(bad_csrf()));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(Ok(Req {
        p,
        settings,
        host: Host::from_state(state),
    }))
}

/// `users_from_params`: the users named, ids or emails, in id order; an
/// unknown list is InvalidParameters (Err with the key).
async fn users_from_params(
    conn: &mut PgConnection,
    p: &Map<String, Value>,
) -> Result<Result<Vec<(i32, String)>, String>, sqlx::Error> {
    let get = |k: &str| params::string(p, k).filter(|v| !v.trim().is_empty());
    let (key, users): (&str, Vec<(i32, String)>) = if let Some(names) = get("usernames") {
        let lower: Vec<String> = names.split(',').map(|n| n.to_lowercase()).collect();
        (
            "usernames",
            sqlx::query_as(
                "SELECT id, username FROM users WHERE username_lower = ANY($1) ORDER BY id",
            )
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?,
        )
    } else if let Some(id) = get("user_id") {
        (
            "user_id",
            sqlx::query_as("SELECT id, username FROM users WHERE id = $1")
                .bind(crate::ruby::to_i(&id) as i32)
                .fetch_all(&mut *conn)
                .await?,
        )
    } else if let Some(ids) = get("user_ids") {
        let ids: Vec<i32> = ids
            .split(',')
            .filter_map(|i| i.trim().parse().ok())
            .collect();
        (
            "user_ids",
            sqlx::query_as("SELECT id, username FROM users WHERE id = ANY($1) ORDER BY id")
                .bind(&ids)
                .fetch_all(&mut *conn)
                .await?,
        )
    } else if let Some(emails) = get("user_emails") {
        let emails: Vec<String> = emails.split(',').map(|e| e.trim().to_lowercase()).collect();
        (
            "user_emails",
            sqlx::query_as(
                "SELECT DISTINCT u.id, u.username FROM users u JOIN user_emails e ON e.user_id = u.id \
                 WHERE lower(e.email) = ANY($1) ORDER BY u.id",
            )
            .bind(&emails)
            .fetch_all(&mut *conn)
            .await?,
        )
    } else {
        return Ok(Ok(Vec::new()));
    };
    if users.is_empty() {
        return Ok(Err(key.to_string()));
    }
    Ok(Ok(users))
}

/// A group with the viewer's membership, by id (`Group.find_by(id:)`).
async fn find_group(
    conn: &mut PgConnection,
    guardian: &Guardian,
    id: &str,
) -> Result<Option<(crate::groups::BasicGroup, Option<bool>)>, sqlx::Error> {
    let id = strip_json(id);
    let Ok(id) = id.parse::<i32>() else {
        return Ok(None);
    };
    let Some(group) = crate::groups::find_basic(conn, None, Some(id)).await? else {
        return Ok(None);
    };
    let membership = crate::groups::membership(conn, guardian, group.id).await?;
    Ok(Some((group, membership)))
}

fn manager_cx<'a>(
    state: &'a AppState,
    promotion: &'a crate::promotion::Ctx<'a>,
) -> group_manager::Cx<'a> {
    group_manager::Cx { state, promotion }
}

/// POST /admin/groups(.json) (Admin::GroupsController#create:
/// Groups::Create)
pub async fn create(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "POST").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let Some(Value::Object(g)) = required(&req.p, "group") else {
        return Ok(param_missing("group"));
    };
    for key in ["custom_fields", "associated_group_ids", "flair_upload_id"] {
        if g.contains_key(key) {
            return Err(
                Unsupported("group custom fields, associated groups and flair uploads").into(),
            );
        }
    }
    let failed = || {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "failed": "FAILED" })),
        )
            .into_response()
    };
    // The contract: the defaults, then its validations.
    let mut row = Map::new();
    row.insert("automatic".into(), json!(false));
    row.insert("id".into(), Value::Null);
    for (column, kind) in gw::COLUMNS {
        let default = match column {
            "mentionable_level"
            | "messageable_level"
            | "visibility_level"
            | "members_visibility_level" => json!(0),
            "primary_group"
            | "public_admission"
            | "public_exit"
            | "allow_membership_requests"
            | "publish_read_state" => json!(false),
            "default_notification_level" => json!(3),
            _ => Value::Null,
        };
        let value = match g.get(column) {
            Some(v) => gw::cast(kind, v),
            None => default,
        };
        row.insert(column.to_string(), value);
    }
    row.remove("bio_cooked");
    row.insert("bio_cooked".into(), Value::Null);
    let int = |c: &str| row[c].as_i64();
    let alias_ok = |c: &str| matches!(int(c), Some(0 | 1 | 2 | 3 | 4 | 99));
    let level_ok = |c: &str| row[c].is_null() || matches!(int(c), Some(0..=4));
    if row["name"].as_str().is_none_or(|n| n.trim().is_empty())
        || !matches!(int("default_notification_level"), Some(0..=4))
        || !alias_ok("mentionable_level")
        || !alias_ok("messageable_level")
        || !level_ok("visibility_level")
        || !level_ok("members_visibility_level")
    {
        return Ok(failed());
    }
    if row["allow_membership_requests"].as_bool() != Some(true) {
        row.insert("membership_request_template".into(), Value::Null);
    }
    // policy :can_create_group
    let can_create = guardian.is_admin()
        || (req.settings.get("moderators_manage_groups")?.truthy() && guardian.is_moderator());
    if !can_create {
        return Ok(super::search::invalid_access(&state));
    }
    // ActiveModel's array cast: a string splits on commas.
    let names = |key: &str| -> Vec<String> {
        match g.get(key) {
            Some(Value::String(s)) => s.split(',').map(str::to_string).collect(),
            Some(Value::Array(a)) => a.iter().filter_map(params::scalar).collect(),
            _ => Vec::new(),
        }
    };
    let mut tx = state.pool.begin().await?;
    let owner_ids: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE username = ANY($1) ORDER BY id")
            .bind(names("owner_usernames"))
            .fetch_all(&mut *tx)
            .await?;
    let member_ids: Vec<i32> =
        sqlx::query_scalar::<_, i32>("SELECT id FROM users WHERE username = ANY($1) ORDER BY id")
            .bind(names("usernames"))
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .filter(|id| !owner_ids.contains(id))
            .collect();
    // policy :can_request_access
    if owner_ids.is_empty() && row["allow_membership_requests"].as_bool() == Some(true) {
        let message = state
            .i18n
            .t("groups.errors.cant_allow_membership_requests")
            .unwrap_or_default();
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "failed": "FAILED", "errors": [message] })),
        )
            .into_response());
    }
    let errors = gw::validate(
        &mut tx,
        &gw::Validation {
            settings: &req.settings,
            i18n: &state.i18n,
            before: None,
            has_owner: !owner_ids.is_empty(),
        },
        &mut row,
    )
    .await?;
    if !errors.is_empty() {
        return Ok(json_error(errors));
    }
    gw::before_save(&req.host, &mut row, None).await?;
    let id = gw::insert(&mut tx, &row).await?;
    // The built group_users, owners first.
    let members: Vec<(i32, bool)> = owner_ids
        .iter()
        .map(|id| (*id, true))
        .chain(member_ids.iter().map(|id| (*id, false)))
        .collect();
    for (user_id, owner) in &members {
        sqlx::query(
            "INSERT INTO group_users (group_id, user_id, owner, notification_level, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, now(), now())",
        )
        .bind(id)
        .bind(user_id)
        .bind(owner)
        .bind(row["default_notification_level"].as_i64().unwrap_or(3) as i32)
        .execute(&mut *tx)
        .await?;
    }
    gw::after_save(&mut tx, id, &row, None).await?;
    // GroupActionLogger#log_group_creation
    let acting = guardian.user_id().unwrap_or_default();
    let mut logged = members.clone();
    logged.sort_by_key(|(u, _)| *u);
    for (user_id, owner) in &logged {
        if *owner {
            sqlx::query(
                "INSERT INTO group_histories (group_id, acting_user_id, target_user_id, action, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp())",
            )
            .bind(id)
            .bind(acting)
            .bind(user_id)
            .bind(MAKE_USER_GROUP_OWNER)
            .execute(&mut *tx)
            .await?;
        }
        group_manager::log_many(&mut tx, id, acting, &[*user_id], ADD_USER_TO_GROUP).await?;
    }
    let full_name = row["full_name"].as_str().unwrap_or_default();
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, details, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(CREATE_GROUP)
    .bind(acting)
    .bind(format!(
        "name: {}, full_name: {full_name}, id: {id}",
        row["name"].as_str().unwrap_or_default()
    ))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    // after_commit: the group's, then each member's sync_add_side_effects.
    let mut tx = state.pool.begin().await?;
    gw::after_commit(&mut tx, id, &row).await?;
    // trigger_group_created_event
    crate::web_hooks::enqueue_group_hooks(
        &mut tx,
        &crate::web_hooks::Payloads {
            settings: &req.settings,
            i18n: &state.i18n,
            config: &state.config,
        },
        "group_created",
        id,
    )
    .await?;
    let promotion = crate::promotion::Ctx {
        settings: &req.settings,
        i18n: &state.i18n,
        host: &req.host,
        config: &state.config,
    };
    let cx = manager_cx(&state, &promotion);
    if !members.is_empty() {
        let group = group_manager::find(&mut tx, id)
            .await?
            .ok_or(Unsupported("a group gone before its members"))?;
        for (user_id, _) in &members {
            group_manager::sync_add_side_effects(&mut tx, &cx, &group, &[*user_id]).await?;
        }
    }
    tx.commit().await?;
    let mut conn = state.pool.acquire().await?;
    let basic = crate::groups::find_basic(&mut conn, None, Some(id))
        .await?
        .ok_or(Unsupported("a created group not found"))?;
    let membership = crate::groups::membership(&mut conn, &guardian, id).await?;
    let json = basic.json(
        &state.i18n,
        &guardian,
        &req.settings,
        Some((membership.is_some(), membership == Some(true))),
    )?;
    Ok(Json(json!({ "basic_group": json })).into_response())
}

/// DELETE /admin/groups/:id(.json) (Admin::GroupsController#destroy)
pub async fn destroy(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "DELETE").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if !guardian.is_admin() {
        return Ok(not_found(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Ok(id) = strip_json(&id).parse::<i32>() else {
        return Ok(not_found(&state));
    };
    let Some(group) = group_manager::find(&mut tx, id).await? else {
        return Ok(not_found(&state));
    };
    if group.automatic {
        return Ok(can_not_modify_automatic(&state));
    }
    let name: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    // StaffActionLogger#log_group_deletion
    let mut details = format!("name: {name}, id: {id}");
    if let Some(level) = group.grant_trust_level {
        details.push_str(&format!(", grant_trust_level: {level}"));
    }
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, details, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(DELETE_GROUP)
    .bind(guardian.user_id().unwrap_or_default())
    .bind(&details)
    .execute(&mut *tx)
    .await?;
    let members: Vec<i32> =
        sqlx::query_scalar("SELECT user_id FROM group_users WHERE group_id = $1 ORDER BY id")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    // The dependents (each `dependent: :destroy`), then the group.
    for table in [
        "category_groups",
        "category_moderation_groups",
        "category_posting_review_groups",
        "group_users",
        "group_requests",
        "group_mentions",
        "group_associated_groups",
        "mcp_group_scopes",
        "group_archived_messages",
        "group_histories",
        "group_category_notification_defaults",
        "group_tag_notification_defaults",
        "groups_web_hooks",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE group_id = $1"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM upload_references WHERE target_type = 'Group' AND target_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // HasDestroyedWebHook: the payload after the dependents go, before
    // the group does; enqueued once it has.
    let destroyed_payload = crate::web_hooks::group_destroyed_payload(
        &mut tx,
        &crate::web_hooks::Payloads {
            settings: &req.settings,
            i18n: &state.i18n,
            config: &state.config,
        },
        id,
    )
    .await?;
    sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::web_hooks::enqueue_group_destroyed(&mut tx, id, destroyed_payload).await?;
    // after_destroy: clear_acls
    crate::jobs::enqueue(
        &mut tx,
        "cleanup_acls_for_deleted",
        json!({ "group_id": id }),
    )
    .await?;
    tx.commit().await?;
    // after_commit: each membership's sync_removal_side_effects, then
    // group_destroyed (chat's auto leave).
    let mut tx = state.pool.begin().await?;
    let promotion = crate::promotion::Ctx {
        settings: &req.settings,
        i18n: &state.i18n,
        host: &req.host,
        config: &state.config,
    };
    for user_id in members {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
        if !exists {
            continue;
        }
        group_manager::grant_other_available_title(&mut tx, &state.i18n, &group, &[user_id])
            .await?;
        group_manager::remove_primary_and_flair_group(&mut tx, id, &[user_id]).await?;
        if group.grant_trust_level.is_some_and(|l| l != 0) {
            crate::promotion::recalculate_with(&mut tx, &promotion, user_id, None, true).await?;
        }
    }
    crate::plugins::group_destroyed(&mut tx, &req.settings).await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// PUT /groups/:id/members(.json), PUT /admin/groups/:id/members
/// (GroupsController#add_members)
pub async fn add_members(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let admin_alias = uri.path().starts_with("/admin/");
    if admin_alias && !guardian.is_admin() {
        return Ok(not_found(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some((group, membership)) = find_group(&mut tx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    if !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)? {
        return Ok(super::search::invalid_access(&state));
    }
    let mut users = match users_from_params(&mut tx, &req.p).await? {
        Ok(u) => u,
        Err(key) => return Ok(super::search::invalid_parameters(&state, &key)),
    };
    if let Some(emails) = params::string(&req.p, "emails").filter(|e| !e.is_empty()) {
        for email in emails.split(',') {
            let user: Option<(i32, String)> = sqlx::query_as(
                "SELECT u.id, u.username FROM users u JOIN user_emails e ON e.user_id = u.id \
                 WHERE lower(e.email) = lower($1) ORDER BY e.primary DESC LIMIT 1",
            )
            .bind(email.trim())
            .fetch_optional(&mut *tx)
            .await?;
            match user {
                Some(u) => users.push(u),
                None => {
                    return Err(Unsupported("adding members by email (invites)").into());
                }
            }
        }
    }
    if users.is_empty() {
        let message = state
            .i18n
            .t("groups.errors.usernames_or_emails_required")
            .unwrap_or_default()
            .to_string();
        return Ok(super::search::invalid_parameters(&state, &message));
    }
    if users.len() > ADD_MEMBERS_LIMIT {
        return Ok(json_error(vec![
            state
                .i18n
                .t_with(
                    "groups.errors.adding_too_many_users.other",
                    &[("count", &ADD_MEMBERS_LIMIT.to_string())],
                )
                .unwrap_or_default(),
        ]));
    }
    let ids: Vec<i32> = users.iter().map(|(id, _)| *id).collect();
    let mut already: Vec<String> = sqlx::query_scalar(
        "SELECT u.username FROM users u JOIN group_users gu ON gu.user_id = u.id \
         WHERE gu.group_id = $1 AND u.id = ANY($2)",
    )
    .bind(group.id)
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    if !already.is_empty() && already.len() == users.len() {
        already.sort();
        let count = already.len() as i64;
        let form = if count == 1 { "one" } else { "other" };
        return Ok(json_error(vec![
            state
                .i18n
                .t_with(
                    &format!("groups.errors.member_already_exist.{form}"),
                    &[
                        ("username", &already.join(", ")),
                        ("count", &count.to_string()),
                    ],
                )
                .unwrap_or_default(),
        ]));
    }
    let notify = params::string(&req.p, "notify_users").as_deref() == Some("true");
    let mut uniq: Vec<(i32, String)> = Vec::new();
    for u in users {
        if !uniq.iter().any(|(id, _)| *id == u.0) {
            uniq.push(u);
        }
    }
    let uniq_ids: Vec<i32> = uniq.iter().map(|(id, _)| *id).collect();
    let promotion = crate::promotion::Ctx {
        settings: &req.settings,
        i18n: &state.i18n,
        host: &req.host,
        config: &state.config,
    };
    let cx = manager_cx(&state, &promotion);
    let mgr_group = group_manager::find(&mut tx, group.id)
        .await?
        .ok_or(Unsupported("a group gone"))?;
    let added = group_manager::add_many(&mut tx, &cx, &mgr_group, &uniq_ids, false).await?;
    let acting = guardian.user_id().unwrap_or_default();
    group_manager::log_many(&mut tx, group.id, acting, &added, ADD_USER_TO_GROUP).await?;
    if notify && !added.is_empty() {
        crate::jobs::enqueue(
            &mut tx,
            "notify_users_added_to_group",
            json!({ "user_ids": added, "group_id": group.id }),
        )
        .await?;
    }
    tx.commit().await?;
    let usernames: Vec<String> = uniq.into_iter().map(|(_, n)| n).collect();
    Ok(Json(json!({ "success": "OK", "usernames": usernames, "emails": [] })).into_response())
}

/// DELETE /groups/:id/members(.json), DELETE /admin/groups/:id/members
/// (GroupsController#remove_member)
pub async fn remove_member(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let mut req = match begin(&state, &headers, &uri, &body, "DELETE").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    if uri.path().starts_with("/admin/") && !guardian.is_admin() {
        return Ok(not_found(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some((group, membership)) = find_group(&mut tx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    if !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)? {
        return Ok(super::search::invalid_access(&state));
    }
    // Backwards compatibility: username and user_email.
    for (alias, key) in [("username", "usernames"), ("user_email", "user_emails")] {
        if let Some(v) = params::string(&req.p, alias).filter(|v| !v.trim().is_empty()) {
            req.p.insert(key.into(), json!(v));
        }
    }
    let users = match users_from_params(&mut tx, &req.p).await? {
        Ok(u) => u,
        Err(key) => return Ok(super::search::invalid_parameters(&state, &key)),
    };
    if users.is_empty() {
        return Ok(super::search::invalid_parameters(
            &state,
            "user_ids or usernames or user_emails must be present",
        ));
    }
    let ids: Vec<i32> = users.iter().map(|(id, _)| *id).collect();
    let promotion = crate::promotion::Ctx {
        settings: &req.settings,
        i18n: &state.i18n,
        host: &req.host,
        config: &state.config,
    };
    let cx = manager_cx(&state, &promotion);
    let mgr_group = group_manager::find(&mut tx, group.id)
        .await?
        .ok_or(Unsupported("a group gone"))?;
    let removed = group_manager::remove_many(&mut tx, &cx, &mgr_group, &ids).await?;
    let acting = guardian.user_id().unwrap_or_default();
    group_manager::log_many(&mut tx, group.id, acting, &removed, REMOVE_USER_FROM_GROUP).await?;
    tx.commit().await?;
    let (removed_names, skipped): (Vec<_>, Vec<_>) =
        users.into_iter().partition(|(id, _)| removed.contains(id));
    Ok(Json(json!({
        "success": "OK",
        "usernames": removed_names.into_iter().map(|(_, n)| n).collect::<Vec<_>>(),
        "skipped_usernames": skipped.into_iter().map(|(_, n)| n).collect::<Vec<_>>(),
    }))
    .into_response())
}

/// PUT /groups/:id/owners(.json) (GroupsController#add_owners)
pub async fn add_owners(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some((group, membership)) = find_group(&mut tx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    if group.automatic {
        return Ok(can_not_modify_automatic(&state));
    }
    if !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)? {
        return Ok(super::search::invalid_access(&state));
    }
    let users = match users_from_params(&mut tx, &req.p).await? {
        Ok(u) => u,
        Err(key) => return Ok(super::search::invalid_parameters(&state, &key)),
    };
    let notify = params::string(&req.p, "notify_users").as_deref() == Some("true");
    let promotion = crate::promotion::Ctx {
        settings: &req.settings,
        i18n: &state.i18n,
        host: &req.host,
        config: &state.config,
    };
    let cx = manager_cx(&state, &promotion);
    let mgr_group = group_manager::find(&mut tx, group.id)
        .await?
        .ok_or(Unsupported("a group gone"))?;
    let acting = guardian.user_id().unwrap_or_default();
    for (user_id, _) in &users {
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
        )
        .bind(group.id)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;
        if !member {
            group_manager::add_many(&mut tx, &cx, &mgr_group, &[*user_id], false).await?;
            group_manager::log_many(&mut tx, group.id, acting, &[*user_id], ADD_USER_TO_GROUP)
                .await?;
        }
        sqlx::query("UPDATE group_users SET owner = TRUE WHERE group_id = $1 AND user_id = $2")
            .bind(group.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        log_owner(&mut tx, group.id, acting, *user_id, MAKE_USER_GROUP_OWNER).await?;
    }
    tx.commit().await?;
    if notify {
        let full_name: Option<String> =
            sqlx::query_scalar("SELECT full_name FROM groups WHERE id = $1")
                .bind(group.id)
                .fetch_one(&state.pool)
                .await?;
        let name_full_preferred = full_name
            .filter(|f| !f.trim().is_empty())
            .unwrap_or_else(|| group.name.clone());
        let ctx = crate::posting::Ctx {
            host: &req.host,
            settings: &req.settings,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        for (user_id, _) in &users {
            crate::system_message::create_from_system_user(
                &state.pool,
                &ctx,
                *user_id,
                "user_added_to_group_as_owner",
                &[
                    ("group_name".to_string(), name_full_preferred.clone()),
                    ("group_path".to_string(), format!("/g/{}", group.name)),
                ],
            )
            .await?;
        }
    }
    let usernames: Vec<String> = users.into_iter().map(|(_, n)| n).collect();
    Ok(Json(json!({ "success": "OK", "usernames": usernames })).into_response())
}

async fn log_owner(
    conn: &mut PgConnection,
    group_id: i32,
    acting: i32,
    user_id: i32,
    action: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO group_histories (group_id, acting_user_id, target_user_id, action, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp())",
    )
    .bind(group_id)
    .bind(acting)
    .bind(user_id)
    .bind(action)
    .execute(conn)
    .await?;
    Ok(())
}

/// DELETE /admin/groups/:id/owners(.json)
/// (Admin::GroupsController#remove_owner)
pub async fn remove_owner(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "DELETE").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some((group, membership)) = find_group(&mut tx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    if group.automatic {
        return Ok(can_not_modify_automatic(&state));
    }
    if !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)? {
        return Ok(super::search::invalid_access(&state));
    }
    let users: Vec<i32> = if let Some(user_id) =
        params::string(&req.p, "user_id").filter(|v| !v.trim().is_empty())
    {
        let found: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
            .bind(crate::ruby::to_i(&user_id) as i32)
            .fetch_optional(&mut *tx)
            .await?;
        match found {
            Some(id) => vec![id],
            None => return Err(Unsupported("remove_owner of an unknown user (a 500)").into()),
        }
    } else {
        let Some(Value::Object(g)) = required(&req.p, "group") else {
            return Ok(param_missing("group"));
        };
        match params::string(g, "usernames").filter(|v| !v.trim().is_empty()) {
            Some(names) => {
                let names: Vec<&str> = names.split(',').collect();
                sqlx::query_scalar("SELECT id FROM users WHERE username = ANY($1) ORDER BY array_position($1, username::text), id")
                    .bind(&names)
                    .fetch_all(&mut *tx)
                    .await?
            }
            None => return Ok(super::search::invalid_parameters(&state, "user_id")),
        }
    };
    let acting = guardian.user_id().unwrap_or_default();
    for user_id in users {
        sqlx::query("UPDATE group_users SET owner = FALSE WHERE group_id = $1 AND user_id = $2")
            .bind(group.id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        log_owner(
            &mut tx,
            group.id,
            acting,
            user_id,
            REMOVE_USER_AS_GROUP_OWNER,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// PUT /admin/groups/:id/primary(.json) (Admin::GroupsController#set_primary)
pub async fn set_primary(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let mut tx = state.pool.begin().await?;
    let Some((group, membership)) = find_group(&mut tx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    let Some(Value::Object(g)) = required(&req.p, "group") else {
        return Ok(param_missing("group"));
    };
    let Some(names) = params::string(g, "usernames") else {
        return Err(Unsupported("set_primary without usernames (a 500)").into());
    };
    let names: Vec<&str> = names.split(',').collect();
    let users: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM users WHERE username = ANY($1) ORDER BY id")
            .bind(&names)
            .fetch_all(&mut *tx)
            .await?;
    // ensure_can_change_primary_group! for each user: can_edit_group?.
    if !users.is_empty()
        && !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)?
    {
        return Ok(super::search::invalid_access(&state));
    }
    let primary = params::string(&req.p, "primary").as_deref() == Some("true");
    sqlx::query("UPDATE users SET primary_group_id = $2 WHERE id = ANY($1)")
        .bind(&users)
        .bind(primary.then_some(group.id))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// PUT /admin/groups/automatic_membership_count(.json)
pub async fn automatic_membership_count(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    if !guardian.is_staff() {
        return Ok(not_found(&state));
    }
    let can_create = guardian.is_admin()
        || (req.settings.get("moderators_manage_groups")?.truthy() && guardian.is_moderator());
    if !can_create {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(value) =
        required(&req.p, "automatic_membership_email_domains").and_then(params::scalar)
    else {
        return Ok(param_missing("automatic_membership_email_domains"));
    };
    let (mut domains, _) = gw::valid_email_domains(&value);
    let mut conn = state.pool.acquire().await?;
    let mut user_count = json!(0);
    if !domains.is_empty() {
        if let Some(group_id) = params::string(&req.p, "id").filter(|v| !v.is_empty()) {
            let group: Option<(bool, Option<String>)> = sqlx::query_as(
                "SELECT automatic, automatic_membership_email_domains FROM groups WHERE id = $1",
            )
            .bind(crate::ruby::to_i(&group_id) as i32)
            .fetch_optional(&mut *conn)
            .await?;
            let Some((automatic, existing)) = group else {
                return Ok(not_found(&state));
            };
            if automatic {
                return Ok(can_not_modify_automatic(&state));
            }
            let existing: Vec<String> = existing
                .unwrap_or_default()
                .split('|')
                .map(str::to_string)
                .collect();
            domains.retain(|d| !existing.contains(d));
        }
        user_count = if domains.len() > 10 {
            Value::Null
        } else {
            json!(automatic_membership_users_count(&mut conn, &domains).await?)
        };
    }
    Ok(Json(json!({ "user_count": user_count })).into_response())
}

/// `Group.automatic_membership_users(domains).count`: the activated,
/// unstaged users' emails ending in one of the domains (a row an email).
pub async fn automatic_membership_users_count(
    conn: &mut PgConnection,
    domains: &[String],
) -> Result<i64, sqlx::Error> {
    if domains.is_empty() {
        return Ok(0);
    }
    let pattern = format!("@({})$", domains.join("|").replace('.', "\\."));
    sqlx::query_scalar(
        "SELECT count(*) FROM users u JOIN user_emails e ON e.user_id = u.id \
         WHERE e.email ~* $1 AND u.active AND NOT u.staged",
    )
    .bind(pattern)
    .fetch_one(conn)
    .await
}

/// `NotificationLevels.all`, in its order.
const LEVELS: [(&str, i32); 5] = [
    ("muted", 0),
    ("regular", 1),
    ("tracking", 2),
    ("watching", 3),
    ("watching_first_post", 4),
];

/// The defaults' planned changes: (id, action, old, new).
#[derive(Clone, Debug)]
struct DefaultChange {
    id: i32,
    delete: bool,
    update: bool,
    old: Option<i32>,
    new: Option<i32>,
}

/// PUT /groups/:id(.json), /g/:id (GroupsController#update)
pub async fn update(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let req = match begin(&state, &headers, &uri, &body, "PUT").await? {
        Ok(r) => r,
        Err(r) => return Ok(r),
    };
    let Some(user) = guardian.user() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let (admin, staff) = (user.admin, guardian.is_staff());
    let mut tx = state.pool.begin().await?;
    let Some((group, membership)) = find_group(&mut tx, &guardian, &id).await? else {
        return Ok(not_found(&state));
    };
    let can_admin = crate::groups::can_admin_group(&guardian, &req.settings, &group, membership)?;
    if !can_admin && !crate::groups::can_edit_group(&guardian, &req.settings, &group, membership)? {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(Value::Object(g)) = required(&req.p, "group") else {
        return Ok(param_missing("group"));
    };
    let automatic = group.automatic;
    // group_params
    let mut permitted: Vec<&str> = vec![
        "bio_raw",
        "default_notification_level",
        "messageable_level",
        "mentionable_level",
        "flair_bg_color",
        "flair_color",
        "flair_icon",
        "flair_upload_id",
    ];
    if !automatic {
        permitted.extend([
            "allow_membership_requests",
            "full_name",
            "public_exit",
            "public_admission",
            "membership_request_template",
        ]);
    }
    if staff {
        permitted.extend(["visibility_level", "members_visibility_level"]);
    }
    if !automatic && staff {
        permitted.extend([
            "incoming_email",
            "title",
            "primary_group",
            "name",
            "grant_trust_level",
            "publish_read_state",
        ]);
    }
    if !automatic && admin {
        for key in [
            "smtp_server",
            "smtp_port",
            "smtp_ssl_mode",
            "smtp_enabled",
            "smtp_updated_by",
            "smtp_updated_at",
            "email_username",
            "email_password",
            "email_from_alias",
            "allow_unknown_sender_topic_replies",
        ] {
            if g.contains_key(key) {
                return Err(Unsupported("group SMTP settings").into());
            }
        }
    }
    if !automatic && can_admin {
        permitted.push("automatic_membership_email_domains");
    }
    let defaults_allowed = !automatic || admin;
    let mut attrs = Map::new();
    for key in &permitted {
        if let Some(v) = g.get(*key) {
            if *key == "flair_upload_id" && !v.is_null() {
                return Err(Unsupported("group flair uploads (UploadReference)").into());
            }
            attrs.insert(key.to_string(), v.clone());
        }
    }
    // The category and tag default setters, each level given.
    let list = |key: &str| -> Option<Vec<String>> {
        match g.get(key) {
            Some(Value::Array(a)) => Some(a.iter().filter_map(params::scalar).collect()),
            _ => None,
        }
    };
    let mut category_sets: Vec<(i32, Vec<String>)> = Vec::new();
    let mut tag_sets: Vec<(i32, Vec<String>)> = Vec::new();
    if defaults_allowed {
        for (name, level) in LEVELS {
            if let Some(ids) = list(&format!("{name}_category_ids")) {
                category_sets.push((level, ids));
            }
            if let Some(tags) = list(&format!("{name}_tags")) {
                tag_sets.push((level, tags));
            }
        }
    }
    let before = gw::load(&mut tx, group.id)
        .await?
        .ok_or(Unsupported("a group gone"))?;
    // user_default_notifications
    let mut level_change: Option<(i32, i32)> = None;
    let mut categories: Vec<DefaultChange> = Vec::new();
    let mut tags: Vec<DefaultChange> = Vec::new();
    if defaults_allowed {
        let existing_categories: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT category_id, notification_level FROM group_category_notification_defaults \
             WHERE group_id = $1 ORDER BY id",
        )
        .bind(group.id)
        .fetch_all(&mut *tx)
        .await?;
        let existing_tags: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT tag_id, notification_level FROM group_tag_notification_defaults \
             WHERE group_id = $1 ORDER BY id",
        )
        .bind(group.id)
        .fetch_all(&mut *tx)
        .await?;
        let mut cat_left = existing_categories.clone();
        let mut tag_left = existing_tags.clone();
        for (name, level) in LEVELS {
            for raw in list(&format!("{name}_category_ids")).unwrap_or_default() {
                if raw == "-1" {
                    continue;
                }
                let id = crate::ruby::to_i(&raw) as i32;
                plan(&mut categories, &mut cat_left, id, level);
            }
            let names: Vec<String> = list(&format!("{name}_tags"))
                .unwrap_or_default()
                .into_iter()
                .filter(|t| t != "-1")
                .collect();
            let tag_ids: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM tags WHERE name = ANY($1) ORDER BY id")
                    .bind(&names)
                    .fetch_all(&mut *tx)
                    .await?;
            for id in tag_ids {
                plan(&mut tags, &mut tag_left, id, level);
            }
        }
        for (id, old) in cat_left {
            if !categories.iter().any(|c| c.id == id) {
                categories.push(DefaultChange {
                    id,
                    delete: true,
                    update: false,
                    old: Some(old),
                    new: None,
                });
            }
        }
        for (id, old) in tag_left {
            if !tags.iter().any(|c| c.id == id) {
                tags.push(DefaultChange {
                    id,
                    delete: true,
                    update: false,
                    old: Some(old),
                    new: None,
                });
            }
        }
        let current = before["default_notification_level"].as_i64().unwrap_or(3) as i32;
        if let Some(new) = attrs
            .get("default_notification_level")
            .and_then(|v| gw::cast(Kind::Int, v).as_i64())
            && new as i32 != current
        {
            level_change = Some((current, new as i32));
        }
        let update_existing = params::string(&req.p, "update_existing_users").unwrap_or_default();
        if update_existing.trim().is_empty() {
            let count =
                existing_users(&mut tx, group.id, level_change, &categories, &tags, false).await?;
            if count > 0 {
                let form = if count == 1 { "one" } else { "other" };
                let message = state
                    .i18n
                    .t_with(
                        &format!("groups.errors.update_existing_users_required.{form}"),
                        &[("count", &count.to_string())],
                    )
                    .or_else(|| {
                        state.i18n.t_with(
                            "groups.errors.update_existing_users_required",
                            &[("count", &count.to_string())],
                        )
                    })
                    .unwrap_or_default();
                return Ok((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "user_count": count, "errors": [message] })),
                )
                    .into_response());
            }
        }
    }
    // group.update(attrs)
    let mut row = before.clone();
    gw::assign(&mut row, &attrs);
    let errors = gw::validate(
        &mut tx,
        &gw::Validation {
            settings: &req.settings,
            i18n: &state.i18n,
            before: Some(&before),
            has_owner: false,
        },
        &mut row,
    )
    .await?;
    if !errors.is_empty() {
        return Ok(json_error(errors));
    }
    gw::before_save(&req.host, &mut row, Some(&before)).await?;
    let changed = gw::changes(&before, &row);
    gw::write_changes(&mut tx, group.id, &row, &changed).await?;
    gw::after_save(&mut tx, group.id, &row, Some(&before)).await?;
    tx.commit().await?;
    let mut tx = state.pool.begin().await?;
    // after_commit: automatic membership, then the defaults' batch_set.
    gw::after_commit(&mut tx, group.id, &row).await?;
    // trigger_group_updated_event
    crate::web_hooks::enqueue_group_hooks(
        &mut tx,
        &crate::web_hooks::Payloads {
            settings: &req.settings,
            i18n: &state.i18n,
            config: &state.config,
        },
        "group_updated",
        group.id,
    )
    .await?;
    for (level, values) in &category_sets {
        gw::batch_set(&mut tx, group.id, *level, false, values).await?;
    }
    for (level, values) in &tag_sets {
        gw::batch_set(&mut tx, group.id, *level, true, values).await?;
    }
    // GroupActionLogger#log_change_group_settings
    let acting = guardian.user_id().unwrap_or_default();
    gw::log_changes(&mut tx, group.id, acting, &before, &row, &changed).await?;
    // record_email_setting_changes!: a second save, whose after_commit
    // enqueues the automatic membership again and sets the defaults
    // again (their inserts' conflicts still take ids).
    gw::after_commit(&mut tx, group.id, &row).await?;
    // trigger_group_updated_event
    crate::web_hooks::enqueue_group_hooks(
        &mut tx,
        &crate::web_hooks::Payloads {
            settings: &req.settings,
            i18n: &state.i18n,
            config: &state.config,
        },
        "group_updated",
        group.id,
    )
    .await?;
    for (level, values) in &category_sets {
        gw::batch_set(&mut tx, group.id, *level, false, values).await?;
    }
    for (level, values) in &tag_sets {
        gw::batch_set(&mut tx, group.id, *level, true, values).await?;
    }
    if params::string(&req.p, "update_existing_users").as_deref() == Some("true") {
        existing_users(&mut tx, group.id, level_change, &categories, &tags, true).await?;
    }
    tx.commit().await?;
    // Redirected to the groups when the editor can no longer see it.
    let mut conn = state.pool.acquire().await?;
    let membership = crate::groups::membership(&mut conn, &guardian, group.id).await?;
    let visibility = row["visibility_level"].as_i64().unwrap_or(0) as i32;
    if !crate::groups::can_see_level(&guardian, visibility, membership) {
        return Err(Unsupported("an editor who can no longer see the group (a redirect)").into());
    }
    Ok(Json(json!({ "success": "OK" })).into_response())
}

/// One level's entry in user_default_notifications.
fn plan(changes: &mut Vec<DefaultChange>, left: &mut Vec<(i32, i32)>, id: i32, level: i32) {
    let old = left.iter().find(|(c, _)| *c == id).map(|(_, l)| *l);
    match old {
        None => changes.push(DefaultChange {
            id,
            delete: false,
            update: false,
            old: None,
            new: Some(level),
        }),
        Some(old) if old == level => {
            left.retain(|(c, _)| *c != id);
        }
        Some(old) => changes.push(DefaultChange {
            id,
            delete: false,
            update: true,
            old: Some(old),
            new: Some(level),
        }),
    }
}

/// `count_existing_users` / `update_existing_users`.
async fn existing_users(
    conn: &mut PgConnection,
    group_id: i32,
    level: Option<(i32, i32)>,
    categories: &[DefaultChange],
    tags: &[DefaultChange],
    update: bool,
) -> Result<i64, sqlx::Error> {
    if level.is_none() && categories.is_empty() && tags.is_empty() {
        return Ok(0);
    }
    let mut ids: Vec<i32> = Vec::new();
    if let Some((old, new)) = level {
        if update {
            sqlx::query(
                "UPDATE group_users SET notification_level = $3 WHERE group_id = $1 AND notification_level = $2",
            )
            .bind(group_id)
            .bind(old)
            .bind(new)
            .execute(&mut *conn)
            .await?;
        } else {
            let found: Vec<i32> = sqlx::query_scalar(
                "SELECT user_id FROM group_users WHERE group_id = $1 AND notification_level = $2",
            )
            .bind(group_id)
            .bind(old)
            .fetch_all(&mut *conn)
            .await?;
            ids.extend(found);
        }
    }
    let mut kept_categories: Vec<DefaultChange> = Vec::new();
    let mut kept_tags: Vec<DefaultChange> = Vec::new();
    for (list, table, column, out) in [
        (categories, "category_users", "category_id", true),
        (tags, "tag_users", "tag_id", false),
    ] {
        let mut kept = Vec::new();
        for change in list {
            if change.update || change.delete {
                let where_sql = format!(
                    "{column} = $1 AND notification_level = $2 AND user_id IN (SELECT user_id FROM group_users WHERE group_id = $3)"
                );
                if update {
                    sqlx::query(&format!("DELETE FROM {table} WHERE {where_sql}"))
                        .bind(change.id)
                        .bind(change.old)
                        .bind(group_id)
                        .execute(&mut *conn)
                        .await?;
                } else {
                    let found: Vec<i32> = sqlx::query_scalar(&format!(
                        "SELECT user_id FROM {table} WHERE {where_sql}"
                    ))
                    .bind(change.id)
                    .bind(change.old)
                    .bind(group_id)
                    .fetch_all(&mut *conn)
                    .await?;
                    ids.extend(found);
                }
                if change.delete && update {
                    continue;
                }
            }
            kept.push(change.clone());
        }
        if out {
            kept_categories = kept;
        } else {
            kept_tags = kept;
        }
    }
    insert_defaults(
        conn,
        group_id,
        &kept_categories,
        "category_users",
        "category_id",
        update,
        &mut ids,
    )
    .await?;
    insert_defaults(
        conn,
        group_id,
        &kept_tags,
        "tag_users",
        "tag_id",
        update,
        &mut ids,
    )
    .await?;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids.len() as i64)
}

/// The batch insert of `update_existing_users` (or its count): each
/// member without a set level for the category or tag gets the new one.
async fn insert_defaults(
    conn: &mut PgConnection,
    group_id: i32,
    changes: &[DefaultChange],
    table: &str,
    column: &str,
    update: bool,
    ids: &mut Vec<i32>,
) -> Result<(), sqlx::Error> {
    let members: Vec<i32> =
        sqlx::query_scalar("SELECT user_id FROM group_users WHERE group_id = $1 ORDER BY id")
            .bind(group_id)
            .fetch_all(&mut *conn)
            .await?;
    for change in changes {
        // Counting, a default being removed counts the members too (its
        // level nil), as Rails' count does.
        let new = change.new;
        if new.is_none() && update {
            continue;
        }
        let skip: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT user_id FROM {table} WHERE {column} = $1 AND user_id = ANY($2) AND notification_level IS NOT NULL"
        ))
        .bind(change.id)
        .bind(&members)
        .fetch_all(&mut *conn)
        .await?;
        let todo: Vec<i32> = members
            .iter()
            .copied()
            .filter(|m| !skip.contains(m))
            .collect();
        if todo.is_empty() {
            continue;
        }
        if update {
            let timestamps = table == "tag_users";
            sqlx::query(&format!(
                "INSERT INTO {table} (user_id, {column}, notification_level{}) \
                 SELECT u, $2, $3{} FROM unnest($1::int[]) WITH ORDINALITY AS t(u, n) ORDER BY n",
                if timestamps {
                    ", created_at, updated_at"
                } else {
                    ""
                },
                if timestamps { ", now(), now()" } else { "" },
            ))
            .bind(&todo)
            .bind(change.id)
            .bind(new)
            .execute(&mut *conn)
            .await?;
        } else {
            ids.extend(todo);
        }
    }
    Ok(())
}
