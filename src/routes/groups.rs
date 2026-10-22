//! The groups directory and a group's pages as JSON (groups_controller's
//! reads): GET /groups (index), /groups/search, /groups/check-name,
//! /g/:name (show), /groups/:name/members, /logs (histories),
//! /mentionable, /messageable and /permissions. The HTML pages are drawn
//! by groups_view.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::groups::{BasicGroup, can_edit_group, can_see_level, insert_before};
use crate::guardian::Guardian;
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// `Discourse::NotFound` as JSON.
fn not_found(state: &AppState) -> Response {
    super::topics::not_found_response(state)
}

/// `fetch_int_from_params(key, default:, max:)` with min 0: the value,
/// or Err for the InvalidParameters it raises.
fn int_param(p: &Map<String, Value>, key: &str, default: i64, max: Option<i64>) -> Result<i64, ()> {
    match p.get(key) {
        None => Ok(default),
        Some(v) => {
            let Some(s) = params::scalar(v) else {
                return Err(());
            };
            match s.trim().parse::<i64>() {
                Ok(n) if n >= 0 && max.is_none_or(|m| n <= m) => Ok(n),
                _ => Err(()),
            }
        }
    }
}

/// `Hash#to_query`: keys sorted, values form-encoded.
fn to_query(pairs: &mut [(String, String)]) -> String {
    pairs.sort();
    let enc = |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `MobileDetection.mobile_device?`
pub(crate) fn mobile_device(headers: &HeaderMap) -> bool {
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    ua.contains("Mobile") && !ua.contains("iPad")
}

/// The groups the viewer is a member of among `group_ids`, and those
/// they own (index's `user_group_ids`, `owner_group_ids`).
async fn viewer_groups(
    conn: &mut PgConnection,
    guardian: &Guardian,
) -> Result<Vec<(i32, bool)>, sqlx::Error> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(Vec::new());
    };
    sqlx::query_as("SELECT group_id, owner FROM group_users WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(conn)
        .await
}

/// BasicGroupSerializer with index's `user_group_ids`/`owner_group_ids`
/// options: `is_group_user` when the viewer is in any listed group,
/// `is_group_owner` when they own any.
#[allow(clippy::too_many_arguments)]
fn listed_group_json(
    state: &AppState,
    guardian: &Guardian,
    settings: &SiteSettings,
    group: &BasicGroup,
    mine: &[(i32, bool)],
    any_member: bool,
    any_owner: bool,
) -> Result<Value, AppError> {
    let membership = mine
        .iter()
        .find(|(id, _)| *id == group.id)
        .map(|(_, owner)| *owner);
    let owner = membership == Some(true);
    let mut out = match group.json(
        &state.i18n,
        guardian,
        settings,
        Some((membership.is_some(), owner)),
    )? {
        Value::Object(m) => m,
        _ => unreachable!(),
    };
    // bio_raw for owners only when the owner_group_ids option says so.
    if !any_owner && !guardian.is_staff() {
        out.remove("bio_raw");
    }
    if any_member {
        insert_before(
            &mut out,
            "members_visibility_level",
            "is_group_user",
            json!(membership.is_some()),
        );
    }
    if any_owner {
        insert_before(
            &mut out,
            "members_visibility_level",
            "is_group_owner",
            json!(owner),
        );
    }
    Ok(Value::Object(out))
}

fn basic_select(where_sql: &str, order: &str) -> String {
    format!(
        "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE {where_sql} ORDER BY {order}",
        crate::groups::BASIC_GROUP_COLUMNS
    )
}

/// GET /groups(.json) (groups#index)
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match index_doc(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &p,
        mobile_device(&headers),
    )
    .await?
    {
        Ok(doc) => Ok(Json(doc).into_response()),
        Err(refused) => Ok(refused),
    }
}

/// groups#index's document, or the response it refuses with.
pub(crate) async fn index_doc(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    p: &Map<String, Value>,
    mobile: bool,
) -> Result<Result<Value, Response>, AppError> {
    if !settings.get("enable_group_directory")?.truthy() && !guardian.is_staff() {
        return Ok(Err(super::search::invalid_access(state)));
    }
    let s = |k: &str| params::string(p, k).filter(|v| !v.is_empty());
    let requested_order = params::string(p, "order").filter(|o| o == "name" || o == "user_count");
    let dir = if params::string(p, "asc").as_deref() == Some("true") {
        "ASC"
    } else {
        "DESC"
    };
    let mut wheres = vec![crate::groups::visible_groups_scope(
        guardian, "g", false, false,
    )];
    let mut type_filters = vec![
        "my",
        "owner",
        "public",
        "close",
        "automatic",
        "non_automatic",
    ];
    if let Some(username) = s("username") {
        let user_id: Option<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1")
                .bind(crate::accounts::normalize_username(&username)?)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(user_id) = user_id else {
            return Ok(Err(not_found(state)));
        };
        wheres.push(crate::groups::members_visible_groups_where(guardian, "g"));
        wheres.push(format!(
            "g.id IN (SELECT group_id FROM group_users WHERE user_id = {user_id})"
        ));
        type_filters.retain(|t| *t != "my" && *t != "owner");
    }
    let filter = s("filter");
    if let Some(filter) = &filter {
        let like = crate::user_search::quote(&format!("%{filter}%"));
        wheres.push(format!("(g.name ILIKE {like} OR g.full_name ILIKE {like})"));
    }
    if !guardian.is_staff() {
        wheres.push("(g.automatic IS FALSE OR g.id = 2)".into());
        type_filters.retain(|t| *t != "automatic");
    }
    let kind = params::string(p, "type");
    if let Some(kind) = &kind {
        let viewer = guardian.user_id();
        let clause = match (kind.as_str(), viewer) {
            ("my", Some(uid)) => {
                format!("g.id IN (SELECT group_id FROM group_users WHERE user_id = {uid})")
            }
            ("owner", Some(uid)) => format!(
                "g.id IN (SELECT group_id FROM group_users WHERE user_id = {uid} AND owner)"
            ),
            ("my" | "owner", None) => return Ok(Err(not_found(state))),
            ("public", _) => "(g.public_admission AND NOT g.automatic)".into(),
            ("close", _) => "(NOT g.public_admission AND NOT g.automatic)".into(),
            ("automatic", _) => "g.automatic".into(),
            ("non_automatic", _) => "NOT g.automatic".into(),
            _ => return Ok(Err(super::search::invalid_parameters(state, "type"))),
        };
        wheres.push(clause);
    }
    let where_sql = wheres.join(" AND ");
    let listed: Vec<i32> =
        sqlx::query_scalar(&format!("SELECT g.id FROM groups g WHERE {where_sql}"))
            .fetch_all(&mut *conn)
            .await?;
    let mine = viewer_groups(conn, guardian).await?;
    let (any_member, any_owner) = if guardian.is_authenticated() {
        let in_listed: Vec<&(i32, bool)> =
            mine.iter().filter(|(id, _)| listed.contains(id)).collect();
        (!in_listed.is_empty(), in_listed.iter().any(|(_, o)| *o))
    } else {
        type_filters.retain(|t| *t != "my" && *t != "owner");
        (false, false)
    };
    type_filters.retain(|t| *t != "non_automatic");
    // group_directory_order: user_count only when every listed group's
    // members are visible.
    let order = match requested_order.as_deref() {
        Some("user_count") => {
            let hidden: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS (SELECT 1 FROM groups g WHERE {where_sql} AND NOT ({}))",
                crate::groups::members_visible_groups_where(guardian, "g")
            ))
            .fetch_one(&mut *conn)
            .await?;
            (!hidden).then_some("user_count")
        }
        other => other,
    };
    let order_sql = match order {
        Some("user_count") => format!("g.user_count {dir}, g.name ASC"),
        Some(_) => format!("g.name {dir}"),
        None => "g.name ASC".into(),
    };
    let total = listed.len() as i64;
    let page = match p.get("page") {
        None => 0,
        Some(_) => match int_param(p, "page", 0, None) {
            Ok(n) => n,
            Err(()) => return Ok(Err(super::search::invalid_parameters(state, "page"))),
        },
    };
    let page_size = if mobile { 15 } else { 36 };
    let groups: Vec<BasicGroup> = sqlx::query_as(&format!(
        "{} LIMIT {page_size} OFFSET {}",
        basic_select(&where_sql, &order_sql),
        page * page_size
    ))
    .fetch_all(&mut *conn)
    .await?;
    let mut list = Vec::new();
    for group in &groups {
        list.push(listed_group_json(
            state, guardian, settings, group, &mine, any_member, any_owner,
        )?);
    }
    let mut more: Vec<(String, String)> = vec![("page".into(), (page + 1).to_string())];
    if let Some(kind) = kind {
        more.push(("type".into(), kind));
    }
    if let Some(order) = order {
        more.push(("order".into(), order.to_string()));
        if let Some(asc) = params::string(p, "asc") {
            more.push(("asc".into(), asc));
        }
    }
    if let Some(filter) = filter {
        more.push(("filter".into(), filter));
    }
    let base = state.config.globals.relative_url_root();
    Ok(Ok(json!({
        "groups": list,
        "extras": {"type_filters": type_filters},
        "total_rows_groups": total,
        "load_more_groups": format!("{base}/groups?{}", to_query(&mut more)),
    })))
}

/// GET /groups/search(.json) (groups#search)
pub async fn search(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let flag = |k: &str| params::string(&p, k).as_deref() == Some("true");
    let include_pseudogroups = flag("include_pseudogroups");
    let include_everyone = (flag("include_everyone") || include_pseudogroups)
        && !settings
            .get("granular_anonymous_and_logged_in_groups_permissions")?
            .truthy();
    let mut wheres = vec![crate::groups::visible_groups_scope(
        &guardian,
        "g",
        include_everyone,
        include_pseudogroups,
    )];
    if let Some(term) = params::string(&p, "term").filter(|t| !t.trim().is_empty()) {
        let term = crate::user_search::quote(&term);
        wheres.push(format!(
            "(position(LOWER({term}) IN LOWER(g.name)) <> 0 OR position(LOWER({term}) IN LOWER(g.full_name)) <> 0)"
        ));
    }
    if params::string(&p, "ignore_automatic").as_deref() == Some("true") {
        wheres.push("g.automatic = FALSE".into());
    }
    let groups: Vec<BasicGroup> = sqlx::query_as(&basic_select(&wheres.join(" AND "), "g.name"))
        .fetch_all(&mut *conn)
        .await?;
    let mine = viewer_groups(&mut conn, &guardian).await?;
    let mut out = Vec::new();
    for group in &groups {
        out.push(listed_group_json(
            &state, &guardian, &settings, group, &mine, false, false,
        )?);
    }
    Ok(Json(Value::Array(out)).into_response())
}

/// `User.username_exists?`: a user or a group by that name.
pub(super) async fn username_exists(
    conn: &mut PgConnection,
    lower: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1) \
             OR EXISTS (SELECT 1 FROM groups WHERE lower(name) = $1)",
    )
    .bind(lower)
    .fetch_one(conn)
    .await
}

/// `User.username_available?(name)` (reserved names not allowed).
pub(crate) async fn username_available(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<bool, AppError> {
    let lower = crate::accounts::normalize_username(name)?;
    if crate::signup::reserved_by_settings(settings, &lower)? {
        return Ok(false);
    }
    Ok(!username_exists(conn, &lower).await?)
}

/// `User.username_available?(name, email, allow_reserved_username:)`: a
/// taken name is free to the staged user whose primary email it is.
pub(crate) async fn username_available_for(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
    email: Option<&str>,
    allow_reserved: bool,
) -> Result<bool, AppError> {
    let lower = crate::accounts::normalize_username(name)?;
    if !allow_reserved && crate::signup::reserved_by_settings(settings, &lower)? {
        return Ok(false);
    }
    if !username_exists(conn, &lower).await? {
        return Ok(true);
    }
    let Some(email) = email.filter(|e| !e.trim().is_empty()) else {
        return Ok(false);
    };
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users u JOIN user_emails e ON e.user_id = u.id \
                        WHERE u.staged AND u.username_lower = $1 AND e.primary AND e.email = $2)",
    )
    .bind(&lower)
    .bind(email)
    .fetch_one(&mut *conn)
    .await?)
}

/// `UserNameSuggester.suggest(name)` for a name that passed the format
/// check: numbered after the name, from the first free number.
pub(crate) async fn suggest_username(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<String, AppError> {
    let max = settings.get("max_username_length")?.to_i() as usize;
    let min = settings.get("min_username_length")?.to_i() as usize;
    // fix_username: rightsize_username (the name is already sanitized).
    let mut name: String = name.chars().take(max).collect();
    while name.ends_with(|c: char| !c.is_ascii_alphanumeric()) {
        name.pop();
    }
    while name.chars().count() < min {
        name.push('1');
    }
    let mut attempt = name.clone();
    let mut offset: Option<i64> = None;
    let mut i = 1i64;
    while !username_available(conn, settings, &attempt).await? {
        if i > 100 {
            return Err(Unsupported("random username suggestions").into());
        }
        let offset = match offset {
            Some(o) => o,
            None => {
                let normalized = crate::accounts::normalize_username(&name)?;
                let count: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM users WHERE username_lower LIKE $1 AND username_lower SIMILAR TO $2",
                )
                .bind(format!("{normalized}%"))
                .bind(format!("{normalized}(0|1|2|3|4|5|6|7|8|9)+"))
                .fetch_one(&mut *conn)
                .await?;
                let o = if count > 0 {
                    let available: Option<i32> = sqlx::query_scalar(
                        "WITH numbers AS (SELECT generate_series(1, $1) AS n) \
                         SELECT n FROM numbers LEFT JOIN users ON (username_lower = $2 || n::varchar) \
                         AND (username_lower <> '') WHERE users.id IS NULL ORDER BY n ASC LIMIT 1",
                    )
                    .bind((count + 10) as i32)
                    .bind(&normalized)
                    .fetch_optional(&mut *conn)
                    .await?;
                    (i64::from(available.unwrap_or(0)) - 1).max(0)
                } else {
                    0
                };
                offset = Some(o);
                o
            }
        };
        let suffix = (i + offset).to_string();
        let keep = max.saturating_sub(suffix.len());
        attempt = format!("{}{suffix}", name.chars().take(keep).collect::<String>());
        i += 1;
    }
    Ok(attempt)
}

/// GET /groups/check-name(.json) (groups#check_name):
/// UsernameCheckerService with reserved names allowed.
pub async fn check_name(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let Some(name) = params::string(&p, "group_name").filter(|n| !n.is_empty()) else {
        return Ok(param_missing("group_name"));
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if let Some(error) = crate::signup::username_format_error(&settings, &state.i18n, &name)? {
        return Ok(Json(json!({"errors": [error]})).into_response());
    }
    let lower = crate::accounts::normalize_username(&name)?;
    let available = !crate::signup::clashing_with_existing_route(&name)?
        && !username_exists(&mut conn, &lower).await?;
    if available {
        return Ok(Json(json!({"available": true, "is_developer": false})).into_response());
    }
    let suggestion = suggest_username(&mut conn, &settings, &name).await?;
    Ok(Json(json!({"available": false, "suggestion": suggestion})).into_response())
}

/// `params.require(key)` failing: 400 ParameterMissing.
fn param_missing(key: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "errors": [format!("param is missing or the value is empty or invalid: {key}")]
        })),
    )
        .into_response()
}

/// `find_group(:name)` / `find_group_for_show`: the group the viewer can
/// see, with their membership of it.
pub(crate) async fn find_visible(
    conn: &mut PgConnection,
    guardian: &Guardian,
    name: Option<&str>,
    id: Option<i32>,
) -> Result<Option<(BasicGroup, Option<bool>)>, sqlx::Error> {
    let Some(group) = crate::groups::find_basic(conn, name, id).await? else {
        return Ok(None);
    };
    let membership = crate::groups::membership(conn, guardian, group.id).await?;
    if !can_see_level(guardian, group.visibility_level, membership) {
        return Ok(None);
    }
    Ok(Some((group, membership)))
}

/// The group name in a path, less the `.json` the route may carry.
fn strip_json(name: &str) -> (&str, bool) {
    match name.strip_suffix(".json") {
        Some(n) => (n, true),
        None => (name, false),
    }
}

/// The columns GroupShowSerializer adds for admins.
#[derive(sqlx::FromRow)]
struct ShowExtra {
    automatic_membership_email_domains: Option<String>,
    smtp_server: Option<String>,
    smtp_port: Option<i32>,
    smtp_ssl_mode: i32,
    smtp_enabled: Option<bool>,
    smtp_updated_at: Option<chrono::NaiveDateTime>,
    smtp_updated_by_id: Option<i32>,
    email_username: Option<String>,
    email_password: Option<String>,
    email_from_alias: Option<String>,
    allow_unknown_sender_topic_replies: bool,
}

/// GroupShowSerializer (root nil) for the viewer.
pub(crate) async fn show_json(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    group: &BasicGroup,
    membership: Option<bool>,
) -> Result<Value, AppError> {
    let owner = membership == Some(true);
    let mut out = match group.json(
        &state.i18n,
        guardian,
        settings,
        Some((membership.is_some(), owner)),
    )? {
        Value::Object(m) => m,
        _ => unreachable!(),
    };
    if guardian.is_authenticated() {
        insert_before(
            &mut out,
            "members_visibility_level",
            "is_group_user",
            json!(membership.is_some()),
        );
        if owner {
            insert_before(
                &mut out,
                "members_visibility_level",
                "is_group_owner",
                json!(true),
            );
        }
        out.insert("is_group_owner_display".into(), json!(owner));
        let uid = guardian.user_id().unwrap_or_default();
        let mentionable: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM groups WHERE ({}) AND groups.id = $1)",
            crate::user_search::mentionable_where(guardian, uid)
        ))
        .bind(group.id)
        .fetch_one(&mut *conn)
        .await?;
        out.insert("mentionable".into(), json!(mentionable));
        out.insert(
            "messageable".into(),
            json!(messageable(conn, settings, guardian, group).await?),
        );
    }
    // include_flair_icon?/include_flair_type?: is_group_owner is always
    // true there, so only presence counts.
    if let Some(icon) = group.flair_icon.as_deref().filter(|i| !i.is_empty()) {
        out.insert("flair_icon".into(), json!(icon));
        out.insert("flair_type".into(), json!("icon"));
    } else if group.flair_upload_id.is_some() {
        if group.flair_upload_url.is_none() {
            return Err(Unsupported("group flair upload without an upload row").into());
        }
        out.insert("flair_type".into(), json!("image"));
    }
    let can_admin = crate::groups::can_admin_group(guardian, settings, group, membership)?;
    let extra: ShowExtra =
        sqlx::query_as(
            "SELECT automatic_membership_email_domains, smtp_server, smtp_port, smtp_ssl_mode, smtp_enabled, \
                    smtp_updated_at, smtp_updated_by_id, email_username, email_password, email_from_alias, \
                    allow_unknown_sender_topic_replies FROM groups WHERE id = $1",
        )
        .bind(group.id)
        .fetch_one(&mut *conn)
        .await?;
    if can_admin {
        out.insert(
            "automatic_membership_email_domains".into(),
            json!(extra.automatic_membership_email_domains),
        );
    }
    if guardian.is_admin() {
        if extra.smtp_updated_by_id.is_some() {
            return Err(Unsupported("group SMTP settings").into());
        }
        out.insert("smtp_server".into(), json!(extra.smtp_server));
        out.insert("smtp_port".into(), json!(extra.smtp_port));
        out.insert("smtp_ssl_mode".into(), json!(extra.smtp_ssl_mode));
        out.insert("smtp_enabled".into(), json!(extra.smtp_enabled));
        out.insert(
            "smtp_updated_at".into(),
            json!(extra.smtp_updated_at.map(time_json)),
        );
        out.insert("smtp_updated_by".into(), Value::Null);
        out.insert("email_username".into(), json!(extra.email_username));
        out.insert("email_password".into(), json!(extra.email_password));
        out.insert("email_from_alias".into(), json!(extra.email_from_alias));
        let message_count: i64 = if group.has_messages {
            sqlx::query_scalar(
                "SELECT count(*) FROM topic_allowed_groups tag JOIN topics t ON t.id = tag.topic_id \
                 WHERE tag.group_id = $1",
            )
            .bind(group.id)
            .fetch_one(&mut *conn)
            .await?
        } else {
            0
        };
        out.insert("message_count".into(), json!(message_count));
        out.insert(
            "allow_unknown_sender_topic_replies".into(),
            json!(extra.allow_unknown_sender_topic_replies),
        );
    }
    if guardian.is_admin() || owner {
        let defaults: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT notification_level, category_id FROM group_category_notification_defaults \
             WHERE group_id = $1 ORDER BY id",
        )
        .bind(group.id)
        .fetch_all(&mut *conn)
        .await?;
        for (key, level) in [
            ("watching_category_ids", 3),
            ("tracking_category_ids", 2),
            ("watching_first_post_category_ids", 4),
            ("regular_category_ids", 1),
            ("muted_category_ids", 0),
        ] {
            let ids: Vec<i32> = defaults
                .iter()
                .filter(|(l, _)| *l == level)
                .map(|(_, c)| *c)
                .collect();
            out.insert(key.into(), json!(ids));
        }
        let tags_shown = settings.get("tagging_enabled")?.truthy() && guardian.is_admin() || owner;
        if tags_shown {
            let tags: Vec<(i32, i32, String, Option<String>)> = sqlx::query_as(
                "SELECT d.notification_level, t.id, t.name, t.slug FROM group_tag_notification_defaults d \
                 JOIN tags t ON t.id = d.tag_id WHERE d.group_id = $1 ORDER BY d.id",
            )
            .bind(group.id)
            .fetch_all(&mut *conn)
            .await?;
            for (key, level) in [
                ("watching_tags", 3),
                ("watching_first_post_tags", 4),
                ("tracking_tags", 2),
                ("regular_tags", 1),
                ("muted_tags", 0),
            ] {
                let list: Vec<Value> = tags
                    .iter()
                    .filter(|(l, ..)| *l == level)
                    .map(|(_, id, name, slug)| json!({"id": id, "name": name, "slug": slug}))
                    .collect();
                out.insert(key.into(), json!(list));
            }
        }
    }
    Ok(Value::Object(out))
}

/// `guardian.can_send_private_message?(group)`
async fn messageable(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    group: &BasicGroup,
) -> Result<bool, AppError> {
    let target = crate::posting::group_messages::AllowedGroup {
        id: group.id,
        name: group.name.clone(),
        user_count: group.user_count,
        has_messages: group.has_messages,
    };
    let can_pm = guardian.can_send_private_messages(settings)?;
    Ok(crate::posting::group_messages::can_send_to(conn, guardian, can_pm, false, &target).await?)
}

/// `Group.visible_groups(current_user)` names, less the automatic ones
/// (moderators aside) for non-staff: show's `visible_group_names`.
pub(crate) async fn visible_group_names(
    conn: &mut PgConnection,
    guardian: &Guardian,
) -> Result<Vec<String>, sqlx::Error> {
    let mut sql = format!(
        "SELECT g.name FROM groups g WHERE {}",
        crate::groups::visible_groups_scope(guardian, "g", false, false)
    );
    if !guardian.is_staff() {
        sql.push_str(" AND (g.automatic IS FALSE OR g.id = 2)");
    }
    sql.push_str(" ORDER BY g.name ASC");
    sqlx::query_scalar(&sql).fetch_all(conn).await
}

/// GET /groups/by-id/:id(.json)
pub async fn show_by_id(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(id): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (id, json_suffix) = strip_json(&id);
    let id = crate::ruby::to_i(id) as i32;
    show_inner(
        &state,
        guardian,
        None,
        Some(id),
        json_suffix,
        &headers,
        &uri,
    )
    .await
}

/// GET /g/:name(.json) and /groups/:name(.json) (groups#show)
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (name, json_suffix) = strip_json(&name);
    show_inner(
        &state,
        guardian,
        Some(name),
        None,
        json_suffix,
        &headers,
        &uri,
    )
    .await
}

async fn show_inner(
    state: &AppState,
    guardian: Guardian,
    name: Option<&str>,
    id: Option<i32>,
    json_suffix: bool,
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Response, AppError> {
    if !json_suffix && !super::wants_json(uri.query(), headers) {
        return show_page(state, guardian, name, id, headers, uri, None).await;
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some((group, membership)) = find_visible(&mut conn, &guardian, name, id).await? else {
        return Ok(not_found(state));
    };
    let doc = show_json(state, &mut conn, &settings, &guardian, &group, membership).await?;
    let names = visible_group_names(&mut conn, &guardian).await?;
    Ok(Json(json!({"group": doc, "extras": {"visible_group_names": names}})).into_response())
}

/// A user as BasicUserSerializer shows them.
async fn basic_user(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<Value, AppError> {
    let (id, username, name, avatar): (i32, String, Option<String>, Option<i32>) =
        sqlx::query_as("SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(conn)
            .await?;
    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert("username".into(), json!(username));
    if settings.get("enable_names")?.truthy() {
        out.insert("name".into(), json!(name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls, id, &username, avatar, None
        )?),
    );
    Ok(Value::Object(out))
}

/// UserStatusMixin: the port has no user status serializer, so a member
/// with one is refused.
async fn refuse_status(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<(), AppError> {
    if !settings.get("enable_user_status")?.truthy() {
        return Ok(());
    }
    let has: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = $1 AND (ends_at IS NULL OR ends_at > now()))",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await?;
    if has {
        return Err(Unsupported("user status").into());
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    id: i32,
    added_at: chrono::NaiveDateTime,
    reason: Option<String>,
    requested_at: Option<chrono::NaiveDateTime>,
}

/// The filter clause of `filter_by_username` (members) or, for admins,
/// `filter_by_username_or_email`.
async fn member_filter(
    conn: &mut PgConnection,
    guardian: &Guardian,
    filter: &str,
) -> Result<String, sqlx::Error> {
    use crate::user_search::quote;
    let list: Option<Vec<&str>> = filter.contains(',').then(|| filter.split(',').collect());
    if guardian.is_admin() {
        if list.is_none() && regex::Regex::new(r".+@.+").unwrap().is_match(filter) {
            let user_id: Option<i32> = sqlx::query_scalar(
                "SELECT user_id FROM user_emails WHERE lower(email) = $1 LIMIT 1",
            )
            .bind(filter.to_lowercase())
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(user_id) = user_id {
                return Ok(format!("users.id = {user_id}"));
            }
        }
        let primary = "(SELECT lower(email) FROM user_emails WHERE user_emails.user_id = users.id AND user_emails.primary LIMIT 1)";
        return Ok(match list {
            Some(list) => {
                let pattern = quote(&format!("({})", list.join("|")));
                format!(
                    "EXISTS (SELECT 1 FROM user_emails pe WHERE pe.user_id = users.id AND pe.primary) \
                     AND (users.username_lower ~* {pattern} OR {primary} SIMILAR TO {pattern})"
                )
            }
            None => {
                let like = quote(&format!("%{filter}%"));
                format!(
                    "EXISTS (SELECT 1 FROM user_emails pe WHERE pe.user_id = users.id AND pe.primary) \
                     AND (users.username_lower ILIKE {like} OR {primary} ILIKE {like})"
                )
            }
        });
    }
    Ok(match list {
        Some(list) => format!(
            "users.username_lower ~* {}",
            quote(&format!("({})", list.join("|")))
        ),
        None => format!(
            "users.username_lower ILIKE {}",
            quote(&format!("%{filter}%"))
        ),
    })
}

/// GET /groups/:name/members(.json) (groups#members)
pub async fn members(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    match members_doc(&state, &mut conn, &settings, &guardian, &name, &p).await? {
        Ok(doc) => Ok(Json(doc).into_response()),
        Err(refused) => Ok(refused),
    }
}

/// groups#members' document, or the response it refuses with.
pub(crate) async fn members_doc(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    name: &str,
    p: &Map<String, Value>,
) -> Result<Result<Value, Response>, AppError> {
    let Some((group, membership)) = find_visible(conn, guardian, Some(name), None).await? else {
        return Ok(Err(not_found(state)));
    };
    if !can_see_level(guardian, group.members_visibility_level, membership) {
        return Ok(Err(super::search::invalid_access(state)));
    }
    let Ok(limit) = int_param(p, "limit", 50, Some(1000)) else {
        return Ok(Err(super::search::invalid_parameters(state, "limit")));
    };
    let offset = params::integer(p, "offset").unwrap_or(0);
    if offset < 0 {
        return Ok(Err(super::search::invalid_parameters(state, "offset")));
    }
    let dir = if params::string(p, "asc").as_deref() == Some("true") {
        "ASC"
    } else {
        "DESC"
    };
    let order_param = params::string(p, "order");
    let filter = params::string(p, "filter").filter(|f| !f.trim().is_empty());
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let enable_names = settings.get("enable_names")?.truthy();

    if p.contains_key("requesters") {
        if !can_edit_group(guardian, settings, &group, membership)? {
            return Ok(Err(super::search::invalid_access(state)));
        }
        let base = format!(
            "FROM users JOIN group_requests gr ON gr.user_id = users.id WHERE gr.group_id = {}",
            group.id
        );
        let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) {base}"))
            .fetch_one(&mut *conn)
            .await?;
        let mut where_sql = String::new();
        if let Some(filter) = &filter {
            where_sql = format!(" AND {}", member_filter(conn, guardian, filter).await?);
        }
        let order = if order_param.as_deref() == Some("requested_at") {
            format!("gr.created_at {dir}, users.username_lower {dir}")
        } else {
            format!("users.username_lower {dir}")
        };
        let rows: Vec<MemberRow> = sqlx::query_as(&format!(
            "SELECT users.id, FALSE AS owner, gr.created_at AS added_at, gr.reason, gr.created_at AS requested_at \
             {base}{where_sql} ORDER BY {order} LIMIT {limit} OFFSET {offset}"
        ))
        .fetch_all(&mut *conn)
        .await?;
        let mut list = Vec::new();
        for row in rows {
            refuse_status(conn, settings, row.id).await?;
            let mut u = match basic_user(conn, &urls, settings, row.id).await? {
                Value::Object(m) => m,
                _ => unreachable!(),
            };
            u.insert("reason".into(), json!(row.reason));
            u.insert(
                "requested_at".into(),
                json!(row.requested_at.map(time_json)),
            );
            list.push(Value::Object(u));
        }
        return Ok(Ok(json!({
            "members": list,
            "meta": {"total": total, "limit": limit, "offset": offset},
        })));
    }

    if p.contains_key("include_custom_fields") || order_param.as_deref() == Some("custom_field") {
        return Err(Unsupported("ordering group members by custom fields").into());
    }
    let order = match order_param.as_deref() {
        Some(o @ ("last_posted_at" | "last_seen_at")) => format!("users.{o} {dir} NULLS LAST"),
        Some("added_at") => format!("gu.created_at {dir}"),
        _ => "NOT gu.owner".to_string(),
    };
    // listed_users: human users (custom and automatic groups alike hide
    // bots, every automatic group being one of AUTO_GROUPS).
    let base = format!(
        "FROM users JOIN group_users gu ON gu.user_id = users.id WHERE gu.group_id = {} AND users.id > 0",
        group.id
    );
    let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) {base}"))
        .fetch_one(&mut *conn)
        .await?;
    let mut where_sql = String::new();
    if let Some(filter) = &filter {
        where_sql = format!(" AND {}", member_filter(conn, guardian, filter).await?);
    }
    let select = format!(
        "SELECT users.id, gu.owner, gu.created_at AS added_at, NULL::text AS reason, \
                NULL::timestamp AS requested_at {base}{where_sql}"
    );
    let rows: Vec<MemberRow> = sqlx::query_as(&format!(
        "{select} ORDER BY {order}, users.username_lower {dir} LIMIT {limit} OFFSET {offset}"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let owners: Vec<MemberRow> = sqlx::query_as(&format!(
        "{select} AND gu.owner ORDER BY {order}, users.username_lower {dir}"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let mut members = Vec::new();
    for row in &rows {
        members.push(member_json(conn, &urls, settings, guardian, enable_names, row).await?);
    }
    let mut owner_list = Vec::new();
    for row in &owners {
        owner_list.push(member_json(conn, &urls, settings, guardian, enable_names, row).await?);
    }
    Ok(Ok(json!({
        "members": members,
        "owners": owner_list,
        "meta": {"total": total, "limit": limit, "offset": offset},
    })))
}

/// GroupUserWithCustomFieldsSerializer
async fn member_json(
    conn: &mut PgConnection,
    urls: &Urls<'_>,
    settings: &SiteSettings,
    guardian: &Guardian,
    enable_names: bool,
    row: &MemberRow,
) -> Result<Value, AppError> {
    let Some(user) = crate::users::User::find_by_id(conn, row.id).await? else {
        return Err(Unsupported("a group member without a user row").into());
    };
    refuse_status(conn, settings, user.id).await?;
    let mut out = Map::new();
    out.insert("id".into(), json!(user.id));
    out.insert("username".into(), json!(user.username));
    if enable_names {
        out.insert("name".into(), json!(user.name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            urls,
            user.id,
            &user.username,
            user.uploaded_avatar_id,
            None
        )?),
    );
    // custom_fields: User.allowed_user_custom_fields(scope)'s values.
    let fields = crate::review_list::allowed_user_custom_fields(settings, guardian)?;
    let mut custom = Map::new();
    if !fields.is_empty() {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT name, value FROM user_custom_fields WHERE user_id = $1 AND name = ANY($2) ORDER BY id",
        )
        .bind(user.id)
        .bind(&fields)
        .fetch_all(&mut *conn)
        .await?;
        for (name, value) in rows {
            if custom.contains_key(&name) {
                return Err(Unsupported("multi-valued user custom fields").into());
            }
            custom.insert(name, json!(value));
        }
    }
    out.insert("custom_fields".into(), Value::Object(custom));
    let ids: Vec<i32> = [user.primary_group_id, user.flair_group_id]
        .into_iter()
        .flatten()
        .collect();
    let groups = crate::groups::load(conn, &ids).await?;
    crate::groups::user_group_fields(
        &mut out,
        &groups,
        user.primary_group_id,
        user.flair_group_id,
    )?;
    if user.admin {
        out.insert("admin".into(), json!(true));
    }
    if user.moderator {
        out.insert("moderator".into(), json!(true));
    }
    out.insert("trust_level".into(), json!(user.trust_level));
    out.insert("title".into(), json!(user.title));
    if user.visible_to(settings, guardian)? {
        out.insert(
            "last_posted_at".into(),
            json!(user.last_posted_at.map(time_json)),
        );
        out.insert(
            "last_seen_at".into(),
            json!(user.last_seen_at.map(time_json)),
        );
    }
    out.insert("added_at".into(), json!(time_json(row.added_at)));
    Ok(Value::Object(out))
}

/// GET /groups/:name/logs(.json) (groups#histories)
pub async fn histories(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some((group, membership)) = find_visible(&mut conn, &guardian, Some(&name), None).await?
    else {
        return Ok(not_found(&state));
    };
    if !crate::groups::can_admin_group(&guardian, &settings, &group, membership)?
        && !can_edit_group(&guardian, &settings, &group, membership)?
    {
        return Ok(super::search::invalid_access(&state));
    }
    let page_size = 25;
    let offset = params::integer(&p, "offset").unwrap_or(0);
    let mut wheres = vec![format!("h.group_id = {}", group.id)];
    if let Some(Value::Object(filters)) = p.get("filters") {
        let f = |k: &str| params::string(filters, k).filter(|v| !v.trim().is_empty());
        if let Some(action) = f("action") {
            let id = match action.as_str() {
                "change_group_setting" => Some(1),
                "add_user_to_group" => Some(2),
                "remove_user_from_group" => Some(3),
                "make_user_group_owner" => Some(4),
                "remove_user_as_group_owner" => Some(5),
                _ => None,
            };
            wheres.push(match id {
                Some(id) => format!("h.action = {id}"),
                None => "h.action IS NULL".into(),
            });
        }
        if let Some(subject) = f("subject") {
            wheres.push(format!(
                "h.subject = {}",
                crate::user_search::quote(&subject)
            ));
        }
        for key in ["acting_user", "target_user"] {
            if let Some(username) = f(key) {
                wheres.push(format!(
                    "h.{key}_id IN (SELECT id FROM users WHERE username_lower = {})",
                    crate::user_search::quote(&username)
                ));
            }
        }
    }
    type Row = (
        i32,
        Option<i32>,
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        chrono::NaiveDateTime,
    );
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT h.acting_user_id, h.target_user_id, h.action, h.subject, h.prev_value, h.new_value, h.created_at \
         FROM group_histories h WHERE {} ORDER BY h.created_at DESC LIMIT {page_size} OFFSET {}",
        wheres.join(" AND "),
        offset * page_size
    ))
    .fetch_all(&mut *conn)
    .await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let can_admin = crate::groups::can_admin_group(&guardian, &settings, &group, membership)?;
    let mut logs = Vec::new();
    for (acting, target, action, subject, prev, new, created_at) in &rows {
        let action = match action {
            1 => "change_group_setting",
            2 => "add_user_to_group",
            3 => "remove_user_from_group",
            4 => "make_user_group_owner",
            5 => "remove_user_as_group_owner",
            _ => return Err(Unsupported("unknown group history actions").into()),
        };
        // redact_email_setting_value
        let redact = |v: &Option<String>| -> Value {
            match v {
                Some(s)
                    if !s.trim().is_empty()
                        && subject.as_deref().is_some_and(|s| {
                            matches!(
                                s,
                                "email_password"
                                    | "email_username"
                                    | "smtp_server"
                                    | "smtp_port"
                                    | "smtp_ssl_mode"
                            )
                        })
                        && !can_admin =>
                {
                    json!(
                        state
                            .i18n
                            .t("staff_action_logs.redacted")
                            .unwrap_or("[REDACTED]")
                    )
                }
                other => json!(other),
            }
        };
        let acting_user = basic_user(&mut conn, &urls, &settings, *acting).await?;
        let target_user = match target {
            Some(id) => basic_user(&mut conn, &urls, &settings, *id).await?,
            None => Value::Null,
        };
        logs.push(json!({
            "action": action,
            "subject": subject,
            "prev_value": redact(prev),
            "new_value": redact(new),
            "created_at": time_json(*created_at),
            "acting_user": acting_user,
            "target_user": target_user,
        }));
    }
    let all_loaded = (rows.len() as i64) < page_size;
    Ok(Json(json!({"logs": logs, "all_loaded": all_loaded})).into_response())
}

/// GET /groups/:name/mentionable(.json)
pub async fn mentionable(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
) -> Result<Response, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let mut conn = state.pool.acquire().await?;
    let Some((group, _)) = find_visible(&mut conn, &guardian, Some(&name), None).await? else {
        return Ok(not_found(&state));
    };
    let mentionable: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM groups WHERE ({}) AND groups.id = $1)",
        crate::user_search::mentionable_where(&guardian, user_id)
    ))
    .bind(group.id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Json(json!({"mentionable": mentionable})).into_response())
}

/// GET /groups/:name/messageable(.json)
pub async fn messageable_route(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
) -> Result<Response, AppError> {
    if guardian.is_anonymous() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some((group, _)) = find_visible(&mut conn, &guardian, Some(&name), None).await? else {
        return Ok(not_found(&state));
    };
    let messageable = messageable(&mut conn, &settings, &guardian, &group).await?;
    Ok(Json(json!({"messageable": messageable})).into_response())
}

/// GET /groups/:name/permissions(.json): the group's category_groups the
/// viewer can see, by category name, each with its category through
/// CategorySerializer and the group through BasicGroupSerializer.
pub async fn permissions(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(name): Path<String>,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some((group, membership)) = find_visible(&mut conn, &guardian, Some(&name), None).await?
    else {
        return Ok(not_found(&state));
    };
    let rows: Vec<(i32, i32, String)> = sqlx::query_as(
        "SELECT cg.category_id, cg.permission_type, c.name FROM category_groups cg \
         JOIN categories c ON c.id = cg.category_id WHERE cg.group_id = $1 ORDER BY cg.id",
    )
    .bind(group.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut visible = Vec::new();
    for (category_id, permission_type, category_name) in rows {
        let Some(c) =
            crate::category_serializer::GuardedCategory::load(&mut conn, category_id).await?
        else {
            continue;
        };
        if crate::category_serializer::can_see(&mut conn, &settings, &guardian, &c).await? {
            visible.push((category_id, permission_type, category_name));
        }
    }
    // sort_by category name (stable).
    visible.sort_by(|a, b| a.2.cmp(&b.2));
    let group_json = group.json(
        &state.i18n,
        &guardian,
        &settings,
        membership.map(|owner| (true, owner)),
    )?;
    let mut out = Vec::with_capacity(visible.len());
    for (category_id, permission_type, _) in visible {
        let category = crate::category_serializer::serialize(
            &mut conn,
            &crate::category_serializer::Cx::of(&state),
            &settings,
            &guardian,
            category_id,
            &Default::default(),
        )
        .await?;
        out.push(json!({
            "permission_type": permission_type,
            "category": category,
            "group": group_json,
        }));
    }
    Ok(Json(Value::Array(out)).into_response())
}

/// The HTML of the groups pages (layout plus the route's content).
#[derive(askama::Template)]
#[template(path = "groups.html")]
pub struct GroupsHtml {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: crate::html::Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: crate::html::Chrome,
    /// The route's title tokens, joined.
    pub page_title: String,
    pub main: String,
}

/// What every groups page loads around its content: the site's chrome
/// for the viewer, the categories the list context reads, the crawler
/// view.
pub(super) struct Frame {
    pub(super) site: crate::html::Site,
    pub(super) vs: crate::html::ViewerState,
    pub(super) categories: std::collections::HashMap<i64, crate::topic_list_view::ListCategory>,
    pub(super) crawler: crate::html::Crawler,
}

pub(super) async fn frame(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    active: crate::sidebar::Active,
) -> Result<Frame, AppError> {
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let base_path = state.config.globals.relative_url_root();
    let vs = super::session::viewer_state(state, headers, settings, guardian)?;
    let mut site = crate::html::Site::from_settings(settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(conn, state, settings, guardian, active)
        .await?;
    site.bus_position = bus_position;
    let categories = crate::topic_list_view::categories(conn).await?;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let crawler = crate::html::Crawler::for_request(&urls, uri, None)?;
    Ok(Frame {
        site,
        vs,
        categories,
        crawler,
    })
}

pub(super) fn finish(
    frame: Frame,
    settings: &SiteSettings,
    page_title: String,
    main: String,
) -> Result<Response, AppError> {
    use askama::Template;
    let Frame {
        site, vs, crawler, ..
    } = frame;
    let page = GroupsHtml {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        page_title,
        main,
    };
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        settings,
        &vs,
    )?)
}

/// GET /groups (groups#index): the directory page, or its JSON when
/// asked for.
pub async fn index_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    if super::wants_json(uri.query(), &headers) {
        return index(State(state), AuthGuardian(guardian), headers, uri).await;
    }
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // The route's model: groups#index for its query params; a refusal
    // leaves the page with the error, as the store's rejection does.
    let doc = match index_doc(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &p,
        mobile_device(&headers),
    )
    .await?
    {
        Ok(doc) => doc,
        Err(refused) => return Ok(refused),
    };
    let mut frame = frame(
        &state,
        &mut conn,
        &settings,
        &guardian,
        &headers,
        &uri,
        crate::sidebar::Active::Groups,
    )
    .await?;
    let base_path = state.config.globals.relative_url_root();
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path,
        now: crate::clock::now(),
        categories: &frame.categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: frame.site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let filter = params::string(&p, "filter").unwrap_or_default();
    let kind = params::string(&p, "type");
    let can_create_group = guardian.is_admin()
        || (settings.get("moderators_manage_groups")?.truthy() && guardian.is_moderator());
    let main = crate::groups_view::render_index(
        &cx,
        &crate::groups_view::Index {
            doc: &doc,
            filter: &filter,
            kind: kind.as_deref(),
            can_create_group,
        },
    );
    let page_title = crate::topic_list_view::t(&cx, "groups.index.title");
    // hideApplicationFooter while there is more to load.
    if (doc["groups"].as_array().map_or(0, Vec::len) as i64)
        < doc["total_rows_groups"].as_i64().unwrap_or(0)
    {
        frame.site.chrome.show_footer = false;
    }
    frame.site.chrome.body_classes.push_str(" groups-page");
    drop(cx);
    finish(frame, &settings, page_title, main)
}

/// GET /g/:name/<tab> (groups#show for the group's routes): the page
/// drawn at that tab, or the group's JSON when asked for.
pub async fn show_tab(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path((name, tab)): Path<(String, String)>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let (tab, json_suffix) = strip_json(&tab);
    if json_suffix || super::wants_json(uri.query(), &headers) {
        return show_inner(&state, guardian, Some(&name), None, true, &headers, &uri).await;
    }
    show_page(
        &state,
        guardian,
        Some(&name),
        None,
        &headers,
        &uri,
        Some(tab),
    )
    .await
}

/// The group's page (groups#show as HTML): the members tab; the others
/// load routes not ported yet.
async fn show_page(
    state: &AppState,
    guardian: Guardian,
    name: Option<&str>,
    id: Option<i32>,
    headers: &HeaderMap,
    uri: &Uri,
    tab: Option<&str>,
) -> Result<Response, AppError> {
    match tab {
        None | Some("") => {}
        Some(_) => return Err(Unsupported("group activity, messages and manage pages").into()),
    }
    let p = params::parse(uri.query(), headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some((group, membership)) = find_visible(&mut conn, &guardian, name, id).await? else {
        return Ok(not_found(state));
    };
    let doc = show_json(state, &mut conn, &settings, &guardian, &group, membership).await?;
    let names = visible_group_names(&mut conn, &guardian).await?;
    // GroupIndexController's query params, as Group.loadMembers sends them.
    let filter = params::string(&p, "filter").unwrap_or_default();
    let order = params::string(&p, "order").unwrap_or_default();
    let asc = params::string(&p, "asc").is_none_or(|a| a == "true");
    let members = if can_see_level(&guardian, group.members_visibility_level, membership) {
        let mut query = Map::new();
        query.insert("offset".into(), json!("0"));
        query.insert("order".into(), json!(order));
        query.insert("asc".into(), json!(asc.to_string()));
        if !filter.is_empty() {
            query.insert("filter".into(), json!(filter));
        }
        members_doc(state, &mut conn, &settings, &guardian, &group.name, &query)
            .await?
            .ok()
    } else {
        None
    };
    let frame = frame(
        state,
        &mut conn,
        &settings,
        &guardian,
        headers,
        uri,
        crate::sidebar::Active::None,
    )
    .await?;
    let base_path = state.config.globals.relative_url_root();
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path,
        now: crate::clock::now(),
        categories: &frame.categories,
        expand_all_pinned: false,
        hide_category: false,
        member_trust_level: frame.site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let viewer = match guardian.user() {
        Some(_) => Some(crate::groups_view::Viewer {
            admin: guardian.is_admin(),
            can_send_private_messages: guardian.can_send_private_messages(&settings)?,
            can_invite_to_forum: guardian.in_setting_groups(&settings, "invite_allowed_groups")?
                && (settings.get("max_invites_per_day")?.to_i() > 0 || guardian.is_staff()),
        }),
        None => None,
    };
    let main = crate::groups_view::render_show(
        &cx,
        &crate::groups_view::Show {
            group: &doc,
            group_names: &names,
            members: members.as_ref(),
            viewer,
            filter: &filter,
            order: &order,
            asc,
        },
    );
    // The route's title tokens: the members tab's, then the group's name.
    let page_title = format!(
        "{} - {}",
        crate::topic_list_view::t(&cx, "groups.members.title"),
        group.name
    );
    drop(cx);
    finish(frame, &settings, page_title, main)
}
