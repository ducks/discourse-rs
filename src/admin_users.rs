//! Admin::UsersController#index: AdminUserIndexQuery's users (filtered by
//! the list's query, the search, IPs and flags, sorted and paged) through
//! AdminUserListSerializer.
//!
//! Refused: penalty reasons with markup (User.sanitize_staff_reason),
//! sorting by email, and `stats=false`.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The index's params.
#[derive(Default)]
pub struct ListParams {
    pub query: Option<String>,
    pub order: Option<String>,
    pub asc: bool,
    pub page: i64,
    pub show_emails: bool,
    pub email: Option<String>,
    pub filter: Option<String>,
    pub ip: Option<String>,
    pub same_ip_user_id: Option<String>,
    pub ip_type: Option<String>,
    pub exclude: Option<String>,
    pub account_type: Option<String>,
    pub activation: Option<String>,
}

/// UserHistory.actions
const CHECK_EMAIL: i32 = 16;
const SUSPEND_USER: i32 = 10;
const SILENCE_USER: i32 = 30;
/// User::MAX_STAFF_DELETE_POST_COUNT
const MAX_STAFF_DELETE_POST_COUNT: i64 = 5;
/// AdminUserIndexQuery::MAX_FILTER_TERMS
const MAX_FILTER_TERMS: usize = 100;
/// TrustLevel.levels
const TRUST_LEVELS: [(&str, i32); 5] = [
    ("newuser", 0),
    ("basic", 1),
    ("member", 2),
    ("regular", 3),
    ("leader", 4),
];

pub enum Listed {
    Users(Vec<Value>),
    InvalidFilter,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    active: bool,
    admin: bool,
    moderator: bool,
    last_seen_at: Option<NaiveDateTime>,
    last_emailed_at: Option<NaiveDateTime>,
    created_at: NaiveDateTime,
    trust_level: i32,
    manual_locked_trust_level: Option<i32>,
    title: Option<String>,
    approved: bool,
    suspended_at: Option<NaiveDateTime>,
    suspended_till: Option<NaiveDateTime>,
    silenced_till: Option<NaiveDateTime>,
    staged: bool,
    time_read: Option<i32>,
    days_visited: Option<i32>,
    posts_read_count: Option<i32>,
    topics_entered: Option<i32>,
    post_count: Option<i32>,
    topic_count: Option<i32>,
    first_post_created_at: Option<NaiveDateTime>,
    second_factor: bool,
}

/// `normalized_order`: lower case, a trailing direction dropped.
fn normalized_order(order: &str) -> String {
    let lower = order.to_lowercase();
    lower
        .strip_suffix(" asc")
        .or_else(|| lower.strip_suffix(" desc"))
        .unwrap_or(&lower)
        .to_string()
}

/// SORTABLE_MAPPING
fn sort_column(order: &str) -> Result<Option<&'static str>, AppError> {
    Ok(Some(match order {
        "created" => "users.created_at",
        "last_emailed" => "COALESCE(users.last_emailed_at, to_date('1970-01-01', 'YYYY-MM-DD'))",
        "seen" => "COALESCE(users.last_seen_at, to_date('1970-01-01', 'YYYY-MM-DD'))",
        "username" => "users.username",
        "trust_level" => "users.trust_level",
        "days_visited" => "user_stats.days_visited",
        "posts_read" => "user_stats.posts_read_count",
        "topics_viewed" => "user_stats.topics_entered",
        "posts" => "user_stats.post_count",
        "read_time" => "user_stats.time_read",
        "silence_reason" => "silence_reasons.silence_reason",
        "suspend_reason" => "suspend_reasons.suspend_reason",
        "email" => return Err(Unsupported("sorting admin users by email").into()),
        _ => return Ok(None),
    }))
}

/// `AdminUserIndexQuery#find_users(100)` and AdminUserListSerializer with
/// the index's options.
pub async fn list(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
    p: &ListParams,
    path: &str,
) -> Result<Listed, AppError> {
    let mut wheres: Vec<String> = Vec::new();
    let mut joins: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    let mut bind = |v: String| {
        binds.push(v);
        format!("${}", binds.len())
    };
    let query = p.query.as_deref().unwrap_or("");
    let now_sql = format!(
        "'{}'::timestamp",
        crate::clock::now_naive().format("%Y-%m-%d %H:%M:%S%.6f")
    );

    // filter_by_trust
    if let Some((_, level)) = TRUST_LEVELS.iter().find(|(k, _)| *k == query) {
        wheres.push(format!("users.trust_level = {level}"));
    }
    // filter_by_query_classification
    match query {
        "staff" => wheres.push("(users.admin OR users.moderator)".into()),
        "admins" => wheres.push("users.admin".into()),
        "moderators" => wheres.push("users.moderator".into()),
        "silenced" => wheres.push(format!("(users.silenced_till IS NOT NULL AND users.silenced_till > {now_sql})")),
        "suspended" => wheres.push(format!("(users.suspended_till IS NOT NULL AND users.suspended_till > {now_sql})")),
        "pending" => wheres.push(format!(
            "(users.suspended_till IS NULL OR users.suspended_till <= {now_sql}) AND users.approved = FALSE AND users.active"
        )),
        "staged" => wheres.push("users.staged".into()),
        _ => {}
    }
    // filter_by_account_type
    if query == "staff" {
        match p.account_type.as_deref() {
            Some("all") => {}
            Some("bot") => wheres.push("users.id <= 0".into()),
            _ => wheres.push("users.id > 0".into()),
        }
    }
    // filter_by_activation
    match p.activation.as_deref() {
        Some("activated") => wheres.push("users.active".into()),
        Some("not_activated") => wheres.push("NOT users.active".into()),
        _ => {}
    }
    let can_see_ip = can_see_ip(settings, guardian)?;
    let none = "FALSE".to_string();
    // filter_by_ip
    if let Some(ip) = p.ip.as_deref().filter(|ip| !ip.trim().is_empty())
        && p.same_ip_user_id.as_deref().is_none_or(str::is_empty)
    {
        if can_see_ip {
            let b = bind(ip.trim().to_string());
            wheres.push(format!(
                "(users.ip_address = {b}::inet OR users.registration_ip_address = {b}::inet)"
            ));
        } else {
            wheres.push(none.clone());
        }
    }
    // filter_by_same_ip_user
    if let Some(id) = p.same_ip_user_id.as_deref().filter(|s| !s.is_empty()) {
        let column = match p.ip_type.as_deref() {
            Some("registration") => "registration_ip_address",
            _ => "ip_address",
        };
        let ip: Option<String> =
            sqlx::query_scalar(&format!("SELECT {column}::text FROM users WHERE id = $1"))
                .bind(crate::ruby::to_i(id) as i32)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
        match ip {
            Some(ip) => {
                let b = bind(ip);
                wheres.push(format!(
                    "(users.ip_address = {b}::inet OR users.registration_ip_address = {b}::inet)"
                ));
            }
            None => wheres.push(none.clone()),
        }
    }
    // filter_exclude
    if let Some(exclude) = p.exclude.as_deref().filter(|s| !s.is_empty()) {
        wheres.push(format!("users.id <> {}", crate::ruby::to_i(exclude)));
    }
    // filter_by_search
    if let Some(email) = p.email.as_deref().filter(|s| !s.is_empty()) {
        joins.push("JOIN user_emails pe ON pe.user_id = users.id AND pe.primary".into());
        let b = bind(email.to_lowercase());
        wheres.push(format!("pe.email = {b}"));
    } else if let Some(filter) = p.filter.as_deref() {
        static SPLIT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let split = SPLIT.get_or_init(|| regex::Regex::new(r"[,\s]+").unwrap());
        let terms: Vec<&str> = split
            .split(filter)
            .filter(|t| !crate::ruby::is_blank(t))
            .collect();
        if terms.len() > MAX_FILTER_TERMS {
            return Ok(Listed::InvalidFilter);
        }
        if terms.len() == 1 {
            let term = terms[0];
            if parse_ip(term) {
                if p.same_ip_user_id.as_deref().is_none_or(str::is_empty) {
                    if can_see_ip {
                        let b = bind(term.to_string());
                        wheres.push(format!(
                            "(users.ip_address <<= {b}::inet OR users.registration_ip_address <<= {b}::inet)"
                        ));
                    } else {
                        wheres.push(none.clone());
                    }
                }
            } else {
                // filter_by_username_or_email: an exact email first.
                let mut found = None;
                if term.contains('@') && term.find('@').is_some_and(|i| i > 0 && i < term.len() - 1)
                {
                    found = sqlx::query_scalar::<_, i32>(
                        "SELECT user_id FROM user_emails WHERE lower(email) = $1 LIMIT 1",
                    )
                    .bind(term.to_lowercase())
                    .fetch_optional(&mut *conn)
                    .await?;
                }
                match found {
                    Some(id) => wheres.push(format!("users.id = {id}")),
                    None => {
                        joins.push(
                            "JOIN user_emails pe ON pe.user_id = users.id AND pe.primary".into(),
                        );
                        let b = bind(format!("%{term}%"));
                        wheres.push(format!(
                            "(users.username_lower ILIKE {b} OR lower(pe.email) ILIKE {b})"
                        ));
                    }
                }
            }
        } else if terms.len() > 1 {
            // filter_by_multiple_terms
            let terms: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();
            joins.push("JOIN user_emails pe ON pe.user_id = users.id AND pe.primary".into());
            let patterns: Vec<String> = terms
                .iter()
                .map(|t| format!("'%{}%'", t.replace('\'', "''")))
                .collect();
            let mut clause = format!(
                "(users.username_lower ILIKE ANY (ARRAY[{p}]) OR lower(pe.email) ILIKE ANY (ARRAY[{p}])",
                p = patterns.join(",")
            );
            let exact: Vec<String> = terms
                .iter()
                .filter(|t| t.find('@').is_some_and(|i| i > 0 && i < t.len() - 1))
                .map(|t| format!("'{}'", t.replace('\'', "''")))
                .collect();
            if !exact.is_empty() {
                clause.push_str(&format!(
                    " OR users.id IN (SELECT user_id FROM user_emails WHERE lower(user_emails.email) IN ({}))",
                    exact.join(",")
                ));
            }
            clause.push(')');
            wheres.push(clause);
        }
    }

    // initialize_query_with_order
    let order_param = p.order.as_deref().filter(|o| !o.is_empty());
    let mut order: Vec<String> = Vec::new();
    let normalized = order_param.map(normalized_order);
    if let Some(n) = &normalized
        && let Some(column) = sort_column(n)?
    {
        order.push(format!(
            "{column} {} NULLS LAST",
            if p.asc { "ASC" } else { "DESC" }
        ));
    }
    if order_param.is_none() {
        if query == "active" {
            order.push("users.last_seen_at DESC NULLS LAST".into());
        } else {
            order.push("users.created_at DESC".into());
        }
        order.push("users.username".into());
    }
    match normalized.as_deref() {
        Some("silence_reason") => joins.push(penalty_reason_join(
            SILENCE_USER,
            "silenced_till",
            "silence_reason",
        )),
        Some("suspend_reason") => joins.push(penalty_reason_join(
            SUSPEND_USER,
            "suspended_till",
            "suspend_reason",
        )),
        _ => {}
    }

    let page = (p.page - 1).max(0);
    let sql = format!(
        "SELECT users.id, users.username, users.name, users.uploaded_avatar_id, users.active, users.admin, \
                users.moderator, users.last_seen_at, users.last_emailed_at, users.created_at, users.trust_level, \
                users.manual_locked_trust_level, users.title, users.approved, users.suspended_at, users.suspended_till, \
                users.silenced_till, users.staged, user_stats.time_read, user_stats.days_visited, \
                user_stats.posts_read_count, user_stats.topics_entered, user_stats.post_count, user_stats.topic_count, \
                user_stats.first_post_created_at, \
                (EXISTS (SELECT 1 FROM user_second_factors f WHERE f.user_id = users.id AND f.method = 1 AND f.enabled) \
                 OR EXISTS (SELECT 1 FROM user_security_keys k WHERE k.user_id = users.id AND k.enabled AND k.factor_type = 0)) AS second_factor \
         FROM users LEFT JOIN user_stats ON user_stats.user_id = users.id {joins} \
         WHERE {wheres} ORDER BY {order} LIMIT 100 OFFSET {offset}",
        offset = page * 100,
        joins = joins.join(" "),
        wheres = if wheres.is_empty() {
            "TRUE".to_string()
        } else {
            wheres.join(" AND ")
        },
        order = if order.is_empty() {
            "users.id".to_string()
        } else {
            order.join(", ")
        },
    );
    let mut q = sqlx::query_as::<_, Row>(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q.fetch_all(&mut *conn).await?;

    let now = crate::clock::now_naive();
    let silenced: Vec<i32> = rows
        .iter()
        .filter(|r| r.silenced_till.is_some_and(|t| t > now))
        .map(|r| r.id)
        .collect();
    let suspended: Vec<i32> = rows
        .iter()
        .filter(|r| r.suspended_till.is_some_and(|t| t > now))
        .map(|r| r.id)
        .collect();
    let silence_reasons = penalty_reasons(conn, &silenced, SILENCE_USER).await?;
    let suspend_reasons = penalty_reasons(conn, &suspended, SUSPEND_USER).await?;

    if p.show_emails && !rows.is_empty() {
        // StaffActionLogger#log_show_emails
        let details = rows
            .iter()
            .map(|r| format!("[{}] {}", r.id, r.username))
            .collect::<Vec<_>>()
            .join("\n");
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, details, context, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(CHECK_EMAIL)
        .bind(guardian.user_id())
        .bind(details)
        .bind(path)
        .execute(&mut *conn)
        .await?;
    }

    let enable_names = settings.get("enable_names")?.truthy();
    // The system user's avatar may be the small logo.
    let logo_small = settings.get("logo_small")?.to_i();
    let logo_small_url: Option<String> = if logo_small == 0 {
        None
    } else {
        sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
            .bind(i32::try_from(logo_small).unwrap_or(0))
            .fetch_optional(&mut *conn)
            .await?
    };
    let must_approve = settings.get("must_approve_users")?.truthy();
    let local_logins = !settings.get("enable_discourse_connect")?.truthy()
        && settings.get("enable_local_logins")?.truthy();
    let can_check_emails = guardian.is_admin()
        || (guardian.is_staff() && settings.get("moderators_view_emails")?.truthy());
    let age = |t: NaiveDateTime| (now - t).num_microseconds().unwrap_or(0) as f64 / 1e6;

    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut u = Map::new();
        u.insert("id".into(), json!(r.id));
        u.insert("username".into(), json!(r.username));
        if enable_names {
            u.insert("name".into(), json!(r.name));
        }
        u.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls,
                r.id,
                &r.username,
                r.uploaded_avatar_id,
                logo_small_url.as_deref()
            )?),
        );
        // include_email?: staff see their own; staged users or with
        // show_emails, for those who can check emails.
        let email = (guardian.is_staff() && guardian.user_id() == Some(r.id))
            || ((r.staged || p.show_emails) && can_check_emails);
        if email {
            let primary: Option<String> = sqlx::query_scalar(
                "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"",
            )
            .bind(r.id)
            .fetch_optional(&mut *conn)
            .await?;
            let secondary: Vec<String> = sqlx::query_scalar(
                "SELECT email FROM user_emails WHERE user_id = $1 AND NOT \"primary\" ORDER BY id",
            )
            .bind(r.id)
            .fetch_all(&mut *conn)
            .await?;
            u.insert("email".into(), json!(primary));
            u.insert("secondary_emails".into(), json!(secondary));
        }
        u.insert("active".into(), json!(r.active));
        u.insert("admin".into(), json!(r.admin));
        u.insert("moderator".into(), json!(r.moderator));
        u.insert("last_seen_at".into(), json!(r.last_seen_at.map(time_json)));
        u.insert(
            "last_emailed_at".into(),
            json!(r.last_emailed_at.map(time_json)),
        );
        u.insert("created_at".into(), json!(time_json(r.created_at)));
        u.insert("last_seen_age".into(), json!(r.last_seen_at.map(age)));
        u.insert("last_emailed_age".into(), json!(r.last_emailed_at.map(age)));
        u.insert("created_at_age".into(), json!(age(r.created_at)));
        u.insert("trust_level".into(), json!(r.trust_level));
        u.insert(
            "manual_locked_trust_level".into(),
            json!(r.manual_locked_trust_level),
        );
        u.insert("title".into(), json!(r.title));
        if must_approve {
            u.insert("approved".into(), json!(r.approved));
        }
        let is_suspended = r.suspended_till.is_some_and(|t| t > now);
        if is_suspended {
            u.insert("suspended_at".into(), json!(r.suspended_at.map(time_json)));
            u.insert(
                "suspended_till".into(),
                json!(r.suspended_till.map(time_json)),
            );
        }
        if let Some(till) = r.silenced_till {
            u.insert("silenced_till".into(), json!(time_json(till)));
        }
        u.insert("time_read".into(), json!(r.time_read));
        u.insert("staged".into(), json!(r.staged));
        if local_logins && r.second_factor {
            u.insert("second_factor_enabled".into(), json!(true));
        }
        u.insert(
            "can_be_deleted".into(),
            json!(can_delete_user(conn, settings, guardian, r).await?),
        );
        let can_unsuspend =
            guardian.is_staff() && (!(r.admin || r.moderator) || guardian.is_admin());
        u.insert(
            "can_be_suspended".into(),
            json!(can_unsuspend && !(r.admin || r.moderator) && !is_suspended),
        );
        u.insert(
            "silence_reason".into(),
            format_penalty_reason(silence_reasons.get(&r.id))?,
        );
        u.insert(
            "suspend_reason".into(),
            format_penalty_reason(suspend_reasons.get(&r.id))?,
        );
        u.insert("days_visited".into(), json!(r.days_visited));
        u.insert("posts_read_count".into(), json!(r.posts_read_count));
        u.insert("topics_entered".into(), json!(r.topics_entered));
        u.insert("post_count".into(), json!(r.post_count));
        out.push(Value::Object(u));
    }
    Ok(Listed::Users(out))
}

/// `with_penalty_reason`: the latest reason of a user's current penalty.
fn penalty_reason_join(action: i32, till: &str, name: &str) -> String {
    format!(
        "LEFT JOIN LATERAL (SELECT user_histories.details {name} FROM user_histories \
           WHERE user_histories.target_user_id = users.id AND user_histories.action = {action} \
             AND users.{till} IS NOT NULL ORDER BY user_histories.id DESC LIMIT 1) {name}s ON true"
    )
}

/// `penalty_reasons(users, action)`: each user's latest details.
async fn penalty_reasons(
    conn: &mut PgConnection,
    ids: &[i32],
    action: i32,
) -> Result<std::collections::HashMap<i32, Option<String>>, AppError> {
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT DISTINCT ON (target_user_id) target_user_id, details FROM user_histories \
         WHERE action = $1 AND target_user_id = ANY($2) ORDER BY target_user_id, id DESC",
    )
    .bind(action)
    .bind(ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows.into_iter().collect())
}

/// `User.format_penalty_reason`: the sanitized reason's first line.
fn format_penalty_reason(details: Option<&Option<String>>) -> Result<Value, AppError> {
    let Some(Some(details)) = details else {
        return Ok(Value::Null);
    };
    if crate::ruby::is_blank(details) {
        return Ok(Value::Null);
    }
    if details.contains(['<', '>', '&', '"', '\'']) {
        return Err(Unsupported("penalty reasons with markup (sanitize_staff_reason)").into());
    }
    Ok(json!(details.split('\n').next().unwrap_or_default()))
}

/// `can_see_ip?`
fn can_see_ip(settings: &SiteSettings, guardian: &Guardian) -> Result<bool, AppError> {
    Ok(guardian.is_admin()
        || (guardian.is_moderator() && settings.get("moderators_view_ips")?.truthy()))
}

/// `can_delete_user?` for a staff viewer.
async fn can_delete_user(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    r: &Row,
) -> Result<bool, AppError> {
    if r.admin {
        return Ok(false);
    }
    if guardian.user_id() == Some(r.id) {
        if settings.get("enable_discourse_connect")?.truthy() {
            return Ok(false);
        }
        let max = settings.get("delete_user_self_max_post_count")?.to_i();
        return Ok(!has_more_posts_than(conn, r, max).await?);
    }
    if !guardian.is_staff() || (r.moderator && !guardian.is_admin()) {
        return Ok(false);
    }
    let Some(first_post) = r.first_post_created_at else {
        return Ok(true);
    };
    if !has_more_posts_than(conn, r, MAX_STAFF_DELETE_POST_COUNT).await? {
        return Ok(true);
    }
    let days = settings.get("delete_user_max_post_age")?.to_i();
    Ok(first_post > crate::clock::now_naive() - chrono::Duration::days(days))
}

/// `User#has_more_posts_than?`
async fn has_more_posts_than(conn: &mut PgConnection, r: &Row, max: i64) -> Result<bool, AppError> {
    if i64::from(r.topic_count.unwrap_or(0) + r.post_count.unwrap_or(0)) > max {
        return Ok(true);
    }
    if max < 0 {
        return Ok(true);
    }
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(1) FROM (SELECT 1 FROM posts p JOIN topics t ON (p.topic_id = t.id) \
           WHERE p.user_id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL AND \
             (t.archetype <> 'private_message' OR \
              EXISTS (SELECT 1 FROM topic_allowed_users a WHERE a.topic_id = t.id AND a.user_id > 0 AND a.user_id <> $1) OR \
              EXISTS (SELECT 1 FROM topic_allowed_groups g WHERE g.topic_id = p.topic_id)) \
           LIMIT $2) x",
    )
    .bind(r.id)
    .bind(max + 1)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n > max)
}

/// `IPAddr.new(filter)` succeeds: an address, or one with a prefix length.
fn parse_ip(term: &str) -> bool {
    let (addr, prefix) = match term.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (term, None),
    };
    let Ok(ip) = addr.parse::<std::net::IpAddr>() else {
        return false;
    };
    match prefix {
        None => true,
        Some(p) => p
            .parse::<u8>()
            .is_ok_and(|p| p <= if ip.is_ipv4() { 32 } else { 128 }),
    }
}
