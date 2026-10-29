//! discourse-assign's AssignController, mounted at /assign: PUT assign
//! and unassign (`target_type`, `target_id`), GET suggestions (who the
//! assign modal offers), GET assigned (admins) and GET
//! members/:group_name (a group's members by their assignments). Every
//! action wants the plugin on, a user and `can_assign?`. The routes'
//! `PUT /claim/:topic_id` has no action behind it: a 404 here as there.
//! Its list routes are here too.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::session::{bad_csrf, csrf_ok};
use crate::guardian::Guardian;
use crate::params;
use crate::plugins::assign::assigner::{self, AssignOpts, Assignee, Assigner, Target};
use crate::plugins::assign::{self as assign, Permissions, Scope};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

fn form_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    map.iter()
        .filter_map(|(k, v)| params::scalar(v).map(|v| (k.clone(), v)))
        .collect()
}

fn success() -> Response {
    Json(json!({ "success": "OK" })).into_response()
}

/// A param's `presence`.
fn present(p: &Map<String, Value>, key: &str) -> Option<String> {
    params::string(p, key).filter(|v| !crate::ruby::is_blank(v))
}

/// The refusals before an action: `requires_plugin`, `requires_login`,
/// `ensure_assign_allowed`. The site's settings and the permissions when
/// the action may go on.
async fn guard(
    state: &AppState,
    conn: &mut PgConnection,
    guardian: &Guardian,
) -> Result<Result<(SiteSettings, Permissions), Response>, AppError> {
    let settings =
        SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
    if !assign::enabled(&settings)? {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    if guardian.is_anonymous() {
        return Ok(Err(super::login_required::not_logged_in(state)));
    }
    let permissions = Permissions::load(&mut *conn, &settings).await?;
    if !permissions.can_assign(guardian) {
        return Ok(Err(super::search::invalid_access(state)));
    }
    Ok(Ok((settings, permissions)))
}

/// The actions' target: `target_id` and `target_type` required, a topic
/// or post the guardian can see.
async fn target_from_params(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Result<Target, Response>, AppError> {
    let Some(target_id) = present(p, "target_id") else {
        return Ok(Err(super::accounts::param_missing("target_id")));
    };
    let Some(target_type) = present(p, "target_type") else {
        return Ok(Err(super::accounts::param_missing("target_type")));
    };
    if !["topic", "post"].contains(&target_type.to_lowercase().as_str()) {
        return Ok(Err(super::topics::not_found_response(state)));
    }
    let id = crate::ruby::to_i(&target_id);
    let Ok(id) = i32::try_from(id) else {
        return Ok(Err(super::topics::not_found_response(state)));
    };
    // `target_type.constantize`: "topic" isn't a class.
    if target_type != "Topic" && target_type != "Post" {
        return Err(crate::Unsupported("a target_type that isn't a class name").into());
    }
    match Target::load(&mut *conn, &target_type, id).await? {
        Some(target) if target.visible_to(&mut *conn, settings, guardian).await? => Ok(Ok(target)),
        _ => Ok(Err(super::topics::not_found_response(state))),
    }
}

/// The target, refused (InvalidAccess) to who may not assign it.
async fn assignable_target_from_params(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    permissions: &Permissions,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Result<Target, Response>, AppError> {
    let target = match target_from_params(state, conn, settings, guardian, p).await? {
        Ok(target) => target,
        Err(response) => return Ok(Err(response)),
    };
    if !permissions.can_assign_in(guardian, target.category_id) {
        return Ok(Err(super::search::invalid_access(state)));
    }
    Ok(Ok(target))
}

/// `assignment_target_from_params` with the suggestions' check: none
/// without `target_id` and `target_type`.
async fn optional_assignable_target(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    permissions: &Permissions,
    guardian: &Guardian,
    p: &Map<String, Value>,
) -> Result<Result<Option<Target>, Response>, AppError> {
    if present(p, "target_id").is_none() && present(p, "target_type").is_none() {
        return Ok(Ok(None));
    }
    Ok(
        assignable_target_from_params(state, conn, settings, permissions, guardian, p)
            .await?
            .map(Some),
    )
}

/// The guardian's session user, for an action past `requires_login`.
fn user(guardian: &Guardian) -> Result<&crate::session::current::SessionUser, AppError> {
    guardian
        .user()
        .ok_or_else(|| crate::Unsupported("an anonymous assigner").into())
}

/// PUT /assign/assign
pub async fn assign_target(
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
    let mut tx = state.pool.begin().await?;
    let (settings, permissions) = match guard(&state, &mut tx, &guardian).await? {
        Ok(ok) => ok,
        Err(response) => return Ok(response),
    };
    let username = present(&p, "username");
    let group_name = present(&p, "group_name");
    let note = present(&p, "note");
    let status = present(&p, "status");
    let should_notify =
        match params::string(&p, "should_notify").filter(|v| !crate::ruby::is_blank(v)) {
            Some(v) => v == "true",
            None => true,
        };
    let target = match target_from_params(&state, &mut tx, &settings, &guardian, &p).await? {
        Ok(target) => target,
        Err(response) => return Ok(response),
    };
    let assign_to = match resolve_assignee(&state, &mut tx, &guardian, username, group_name).await?
    {
        Ok(assignee) => assignee,
        Err(response) => return Ok(response),
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let by = user(&guardian)?;
    let assigner = Assigner {
        ctx: &ctx,
        target: &target,
        by,
        guardian: &guardian,
        permissions: &permissions,
    };
    let result = assigner
        .assign(
            &mut tx,
            &assign_to,
            AssignOpts {
                note,
                status,
                should_notify,
                ..Default::default()
            },
        )
        .await?;
    match result {
        Ok(()) => {
            tx.commit().await?;
            Ok(success())
        }
        Err(assigner::Reason::InvalidStatus) => Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "errors": ["Status is invalid"], "error_type": "record_invalid" })),
        )
            .into_response()),
        Err(reason) => Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": assigner::failure_message(&state.i18n, &settings, reason, Some(&assign_to))?
            })),
        )
            .into_response()),
    }
}

/// `DiscourseAssign::AssigneeResolver.resolve!`
pub(crate) async fn resolve_assignee(
    state: &AppState,
    conn: &mut PgConnection,
    guardian: &Guardian,
    username: Option<String>,
    group_name: Option<String>,
) -> Result<Result<Assignee, Response>, AppError> {
    use crate::plugins::assign::assigner::Resolve;
    Ok(
        match assigner::resolve(conn, guardian, username, group_name).await? {
            Ok(assignee) => Ok(assignee),
            Err(Resolve::InvalidParameters) => {
                Err(super::search::invalid_parameters(state, "assignee"))
            }
            Err(Resolve::NotFound) => Err(super::topics::not_found_response(state)),
            Err(Resolve::InvalidAccess) => Err(super::search::invalid_access(state)),
        },
    )
}

/// PUT /assign/unassign
pub async fn unassign_target(
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
    let mut tx = state.pool.begin().await?;
    let (settings, permissions) = match guard(&state, &mut tx, &guardian).await? {
        Ok(ok) => ok,
        Err(response) => return Ok(response),
    };
    let target = match assignable_target_from_params(
        &state,
        &mut tx,
        &settings,
        &permissions,
        &guardian,
        &p,
    )
    .await?
    {
        Ok(target) => target,
        Err(response) => return Ok(response),
    };
    let host = crate::pretty_text::Host::from_state(&state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    Assigner {
        ctx: &ctx,
        target: &target,
        by: user(&guardian)?,
        guardian: &guardian,
        permissions: &permissions,
    }
    .unassign(&mut tx, false, false)
    .await?;
    tx.commit().await?;
    Ok(success())
}

/// The names of the visible groups (`Group.visible_groups(user)`, by name)
/// among `ids_sql`.
async fn visible_group_names(
    conn: &mut PgConnection,
    guardian: &Guardian,
    ids_sql: &str,
) -> Result<Vec<String>, AppError> {
    Ok(sqlx::query_scalar(&format!(
        "SELECT groups.name FROM groups WHERE {} AND groups.id IN ({ids_sql}) ORDER BY groups.name ASC",
        crate::user_search::visible_groups_where(guardian),
    ))
    .fetch_all(conn)
    .await?)
}

/// GET /assign/suggestions: the groups whose members may be assigned
/// (`assign_allowed_on_groups`), the groups that may be
/// (`assign_allowed_for_groups`), and the viewer with their five latest
/// assignees still under max_assigned_topics and not on holiday.
pub async fn suggestions(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let (settings, permissions) = match guard(&state, &mut conn, &guardian).await? {
        Ok(ok) => ok,
        Err(response) => return Ok(response),
    };
    let target =
        match optional_assignable_target(&state, &mut conn, &settings, &permissions, &guardian, &p)
            .await?
        {
            Ok(target) => target,
            Err(response) => return Ok(response),
        };
    let me = user(&guardian)?.id;
    let scope = target.as_ref().map_or(Scope::Anywhere, Target::scope);
    let assignable = permissions.assignable_user_ids_sql(scope, Some(&guardian));
    let max = settings.get("max_assigned_topics")?.to_i();
    let recent: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT users.id FROM users \
         JOIN (SELECT assigned_to_id user_id, MAX(created_at) last_assigned FROM assignments \
               WHERE assignments.assigned_to_type = 'User' GROUP BY assigned_to_id \
               HAVING COUNT(*) < {max}) AS x ON x.user_id = users.id \
         LEFT JOIN (SELECT DISTINCT ON (user_id) name, user_id FROM user_custom_fields \
                    WHERE name = '{holiday}') AS ucf ON ucf.user_id = users.id \
         WHERE users.id <> $1 AND users.id IN ({assignable}) AND ucf.name IS NULL \
         ORDER BY x.last_assigned DESC LIMIT 5",
        holiday = assign::HOLIDAY_CUSTOM_FIELD,
    ))
    .bind(me)
    .fetch_all(&mut *conn)
    .await?;
    let mut ids = vec![me];
    ids.extend(recent);
    // assign_allowed_groups_for_target / _for_user
    let allowed_on = permissions.allowed_group_ids(scope, &guardian);
    let assign_allowed_on_groups =
        visible_group_names(&mut conn, &guardian, &assign::id_list(&allowed_on)).await?;
    // allowed_group_ids_for_target, else Group.assignable(user)
    let assignable_groups = format!(
        "SELECT groups.id FROM groups WHERE {}",
        permissions.assignable_groups_where(scope, &guardian)
    );
    let assign_allowed_for_groups =
        visible_group_names(&mut conn, &guardian, &assignable_groups).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let users = crate::user_search::found_users(&mut conn, &urls, &guardian, &ids).await?;
    Ok(Json(json!({
        "assign_allowed_on_groups": assign_allowed_on_groups,
        "assign_allowed_for_groups": assign_allowed_for_groups,
        "suggestions": users,
    }))
    .into_response())
}

/// GET /assign/members/:group_name: the group's members with active
/// assignments, by how many, with the group's own count.
pub async fn group_members(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(group_name): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let group_name = group_name
        .strip_suffix(".json")
        .unwrap_or(&group_name)
        .to_string();
    let mut conn = state.pool.acquire().await?;
    let (settings, permissions) = match guard(&state, &mut conn, &guardian).await? {
        Ok(ok) => ok,
        Err(response) => return Ok(response),
    };
    let limit = params::string(&p, "limit").map_or(50, |l| crate::ruby::to_i(&l));
    let offset = params::string(&p, "offset").map_or(0, |o| crate::ruby::to_i(&o));
    if !(0..=1000).contains(&limit) {
        return Ok(super::search::invalid_parameters(&state, "limit"));
    }
    if offset < 0 {
        return Ok(super::search::invalid_parameters(&state, "offset"));
    }
    let group: Option<(i32, i32, i32)> = sqlx::query_as(
        "SELECT id, visibility_level, members_visibility_level FROM groups WHERE name = $1",
    )
    .bind(&group_name)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((group_id, visibility, members_visibility)) = group else {
        // ensure_can_see_group_and_members!(nil)
        return Ok(super::search::invalid_access(&state));
    };
    if !crate::groups::can_see_group_and_members(
        &mut conn,
        &guardian,
        group_id,
        visibility,
        members_visibility,
    )
    .await?
        || !permissions.can_assign_globally(&guardian)
    {
        return Ok(super::search::invalid_access(&state));
    }
    let filter = match params::string(&p, "filter") {
        Some(filter) => {
            let pattern = crate::user_search::quote(&format!("%{filter}%"));
            if settings.get("enable_names")?.truthy() {
                format!(" AND (users.name ILIKE {pattern} OR users.username_lower ILIKE {pattern})")
            } else {
                format!(" AND users.username_lower ILIKE {pattern}")
            }
        }
        None => String::new(),
    };
    #[derive(sqlx::FromRow)]
    struct Member {
        id: i32,
        username: String,
        username_lower: String,
        name: Option<String>,
        uploaded_avatar_id: Option<i32>,
        assignments_count: i64,
    }
    let members: Vec<Member> = sqlx::query_as(&format!(
        "SELECT users.id, users.username, users.username_lower, users.name, users.uploaded_avatar_id, \
                COUNT(users.id) AS assignments_count \
         FROM users \
         LEFT OUTER JOIN group_users g ON g.user_id = users.id \
         LEFT OUTER JOIN assignments a ON a.assigned_to_id = users.id AND a.assigned_to_type = 'User' \
         LEFT OUTER JOIN topics t ON t.id = a.target_id AND a.target_type = 'Topic' \
         WHERE g.group_id = $1 AND users.id > 0 AND t.deleted_at IS NULL \
           AND a.assigned_to_id IS NOT NULL AND a.active{filter} \
         GROUP BY users.id ORDER BY COUNT(users.id) DESC LIMIT $2 OFFSET $3"
    ))
    .bind(group_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    let group_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM assignments WHERE active AND assigned_to_type = 'Group' \
         AND assigned_to_id = $1",
    )
    .bind(group_id)
    .fetch_one(&mut *conn)
    .await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let logo = crate::admin_users::logo_small_url(&mut conn, &settings).await?;
    let cx = crate::plugins::assign::keys::Cx {
        settings: &settings,
        urls: &urls,
        logo: logo.as_deref(),
    };
    let mut users_count = 0;
    let mut out = Vec::new();
    for m in members {
        users_count += m.assignments_count;
        let mut user = match cx.basic_user(&crate::plugins::assign::keys::UserRef {
            id: m.id,
            username: m.username,
            name: m.name,
            uploaded_avatar_id: m.uploaded_avatar_id,
        })? {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        user.insert("assignments_count".into(), json!(m.assignments_count));
        user.insert("username_lower".into(), json!(m.username_lower));
        out.push(Value::Object(user));
    }
    Ok(Json(json!({
        "members": out,
        "assignment_count": users_count + group_count,
        "group_assignment_count": group_count,
    }))
    .into_response())
}

/// GET /assign/assigned: admins only, the assigned topics by assignee then
/// bump, as AssignedTopicSerializer.
pub async fn assigned(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let (settings, _) = match guard(&state, &mut conn, &guardian).await? {
        Ok(ok) => ok,
        Err(response) => return Ok(response),
    };
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let offset = params::string(&p, "offset").map_or(0, |o| crate::ruby::to_i(&o));
    let limit = params::string(&p, "limit").map_or(100, |l| crate::ruby::to_i(&l));
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let topics = crate::plugins::assign::lists::assigned_topics(
        &mut conn, &state, &settings, &urls, &guardian, offset, limit,
    )
    .await?;
    Ok(Json(json!({ "topics": topics })).into_response())
}

/// GET /topics/messages-assigned/:username(.json): ListController
/// #messages_assigned, the topics and messages assigned to the user (and,
/// unless `direct=true`, to their groups), for those who may assign
/// anywhere.
pub async fn messages_assigned(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(username): Path<String>,
    axum::extract::Query(params): axum::extract::Query<super::list::ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(username) = username.strip_suffix(".json").map(str::to_string) else {
        return ember_page(&state, &guardian, &headers).await;
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let user: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
        .bind(username.to_lowercase())
        .fetch_optional(&mut *conn)
        .await?;
    let Some(user_id) = user else {
        return Ok(super::topics::not_found_response(&state));
    };
    let permissions = Permissions::load(&mut conn, &settings).await?;
    if !permissions.can_assign_globally(&guardian) {
        return Ok(super::search::invalid_access(&state));
    }
    let path = format!(
        "{}/topics/messages-assigned/{username}",
        state.config.globals.relative_url_root()
    );
    assigned_list(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &params,
        &path,
        |direct| crate::plugins::assign::lists::AssignedList::User {
            id: user_id,
            direct,
        },
    )
    .await
}

/// GET /topics/group-topics-assigned/:groupname(.json): ListController
/// #group_topics_assigned, the topics assigned to the group (and, unless
/// `direct=true`, to its members).
pub async fn group_topics_assigned(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(groupname): Path<String>,
    axum::extract::Query(params): axum::extract::Query<super::list::ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let Some(groupname) = groupname.strip_suffix(".json").map(str::to_string) else {
        return ember_page(&state, &guardian, &headers).await;
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let group: Option<(i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT id, visibility_level, members_visibility_level, assignable_level FROM groups \
         WHERE name = $1",
    )
    .bind(&groupname)
    .fetch_optional(&mut *conn)
    .await?;
    // ensure_can_see_group_and_members! first: a missing group is no access.
    let Some((group_id, visibility, members, assignable_level)) = group else {
        return Ok(super::search::invalid_access(&state));
    };
    if !crate::groups::can_see_group_and_members(
        &mut conn, &guardian, group_id, visibility, members,
    )
    .await?
    {
        return Ok(super::search::invalid_access(&state));
    }
    let permissions = Permissions::load(&mut conn, &settings).await?;
    if !permissions.can_assign_globally(&guardian)
        || !assign::can_show_assigned_tab(assignable_level)
    {
        return Ok(super::search::invalid_access(&state));
    }
    let path = format!(
        "{}/topics/group-topics-assigned/{groupname}",
        state.config.globals.relative_url_root()
    );
    assigned_list(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &params,
        &path,
        |direct| crate::plugins::assign::lists::AssignedList::Group {
            id: group_id,
            direct,
        },
    )
    .await
}

/// The Ember app's page for a list path it has no route for.
async fn ember_page(
    state: &AppState,
    guardian: &Guardian,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let bus_position = crate::bus::page_position(&state.bus).await?;
    super::user_preferences::ember_not_found(
        state,
        &mut conn,
        &settings,
        guardian,
        headers,
        bus_position,
    )
    .await
}

/// The first page of the user's assigned topics as the user activity tab
/// loads it; None for a viewer who may not assign anywhere.
pub(crate) async fn messages_assigned_doc(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    user_id: i32,
) -> Result<Option<Value>, AppError> {
    let permissions = Permissions::load(&mut *conn, settings).await?;
    if !permissions.can_assign_globally(guardian) {
        return Ok(None);
    }
    let mut query = crate::topic_query::TopicQuery {
        conn: &mut *conn,
        settings,
        guardian,
        options: Default::default(),
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    };
    let topics = query
        .list(crate::topic_query::Filter::Assigned(
            crate::plugins::assign::lists::AssignedList::User {
                id: user_id,
                direct: false,
            },
        ))
        .await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    Ok(Some(
        crate::topic_list::TopicListSerializer {
            conn,
            settings,
            i18n: &state.i18n,
            guardian,
            urls: &urls,
            more_topics_url: None,
            category_id: None,
            group_id: None,
            prefetched: Default::default(),
        }
        .serialize(&topics)
        .await?,
    ))
}

/// `build_topic_list_options` (with `filter: :direct` for `direct=true`),
/// `generate_list_for`, then `respond_with_list`.
async fn assigned_list(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    params: &super::list::ListParams,
    path: &str,
    list: impl FnOnce(bool) -> crate::plugins::assign::lists::AssignedList,
) -> Result<Response, AppError> {
    let mut options = match super::list::build_options(params, settings)? {
        Ok(o) => o,
        Err(message) => return Ok(super::list::invalid_list_params(state, &message, true)),
    };
    if params.period.is_some() {
        return Err(crate::Unsupported("period on the assigned lists").into());
    }
    let direct = params.last("direct") == Some("true");
    if direct {
        options.filter = Some("direct".to_string());
    }
    let more = super::list::next_url(path, params, &options, true);
    let mut query = crate::topic_query::TopicQuery {
        conn: &mut *conn,
        settings,
        guardian,
        options,
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    };
    let topics = query
        .list(crate::topic_query::Filter::Assigned(list(direct)))
        .await?;
    let category_id = query.options.category_id;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let json = crate::topic_list::TopicListSerializer {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        more_topics_url: Some(more),
        category_id,
        group_id: None,
        prefetched: Default::default(),
    }
    .serialize(&topics)
    .await?;
    Ok(Json(json).into_response())
}
