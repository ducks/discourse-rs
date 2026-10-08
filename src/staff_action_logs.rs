//! Admin::StaffActionLogsController#index: `UserHistory.staff_action_records`
//! with the staff filters, a page of UserHistorySerializer, the total, the
//! next page's URL and every staff action for the filter.
//!
//! UserHistory's actions and which of them are staff actions come from
//! vendor/discourse/config/enums.json (scripts/record-enums), so the
//! bundled plugins' are included.
//!
//! Refused: moderators viewing logs about a topic, post or category (the
//! content redaction runs their guardian over each), dates other than
//! ISO 8601.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{NaiveDate, NaiveDateTime};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

const ENUMS_JSON: &str = include_str!("../vendor/discourse/config/enums.json");

#[derive(Deserialize)]
struct UserHistoryEnums {
    actions: HashMap<String, i32>,
    staff_actions: Vec<String>,
    moderator_visible_actions: Vec<String>,
}

#[derive(Deserialize)]
struct Enums {
    user_history: UserHistoryEnums,
}

static ENUMS: LazyLock<UserHistoryEnums> = LazyLock::new(|| {
    serde_json::from_str::<Enums>(ENUMS_JSON)
        .expect("vendored enums.json")
        .user_history
});

/// `UserHistory.actions[name]`
pub fn action_id(name: &str) -> Option<i32> {
    ENUMS.actions.get(name).copied()
}

fn action_name(id: i32) -> Option<&'static str> {
    ENUMS
        .actions
        .iter()
        .find(|(_, v)| **v == id)
        .map(|(k, _)| k.as_str())
}

/// `UserHistory::CATEGORY_ACTIONS`, `TRUST_LEVEL_ACTIONS`, `EMAIL_ACTIONS`
const CATEGORY_ACTIONS: [&str; 3] = [
    "create_category",
    "change_category_settings",
    "delete_category",
];
const TRUST_LEVEL_ACTIONS: [&str; 3] = [
    "change_trust_level",
    "lock_trust_level",
    "unlock_trust_level",
];
const EMAIL_ACTIONS: [&str; 4] = ["check_email", "add_email", "update_email", "destroy_email"];
/// `new_value_is_json?`
const JSON_VALUE_ACTIONS: [&str; 5] = [
    "change_theme",
    "delete_theme",
    "tag_group_create",
    "tag_group_destroy",
    "tag_group_change",
];

/// `UserHistory.moderator_visible_action_ids`: less what the moderator
/// settings exclude.
fn moderator_visible_action_ids(s: &SiteSettings) -> Result<Vec<i32>, AppError> {
    let mut excluded: Vec<&str> = Vec::new();
    if !s.get("moderators_manage_categories")?.truthy() {
        excluded.extend(CATEGORY_ACTIONS);
    }
    if !s.get("moderators_change_trust_levels")?.truthy() {
        excluded.extend(TRUST_LEVEL_ACTIONS);
    }
    if !s.get("moderators_view_emails")?.truthy() {
        excluded.extend(EMAIL_ACTIONS);
    }
    Ok(ENUMS
        .moderator_visible_actions
        .iter()
        .filter(|a| !excluded.contains(&a.as_str()))
        .filter_map(|a| action_id(a))
        .collect())
}

/// The request's params the index reads.
#[derive(Default)]
pub struct Params {
    pub action_id: Option<String>,
    pub custom_type: Option<String>,
    pub acting_user: Option<String>,
    pub target_user: Option<String>,
    pub subject: Option<String>,
    pub action_name: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub page: Option<String>,
    pub limit: Option<String>,
    /// The permitted staff filters in request order, for the next page's URL.
    pub permitted: Vec<(String, String)>,
}

/// `UserHistory::staff_filters`
pub const STAFF_FILTERS: [&str; 6] = [
    "action_id",
    "custom_type",
    "acting_user",
    "target_user",
    "subject",
    "action_name",
];

pub const INDEX_LIMIT: i64 = 200;

pub enum Outcome {
    Json(Value),
    /// Discourse::InvalidParameters, its key.
    InvalidParameter(&'static str),
}

/// Ruby's `Integer(str)` for plain decimal strings.
fn strict_integer(s: &str) -> Option<i64> {
    let t = s.trim();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}

/// `"..".to_time` for ISO dates and times (the server is UTC).
fn to_time(s: &str) -> Result<NaiveDateTime, Unsupported> {
    let t = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).unwrap_or_default());
    }
    for f in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.fZ",
    ] {
        if let Ok(t) = NaiveDateTime::parse_from_str(t, f) {
            return Ok(t);
        }
    }
    Err(Unsupported("staff action log dates other than ISO 8601"))
}

/// Binds the filter's values onto a query in order.
macro_rules! bind_all {
    ($q:expr, $binds:expr $(,)?) => {{
        let mut q = $q;
        for b in $binds {
            q = match b {
                Bind::Int(v) => q.bind(*v),
                Bind::Ints(v) => q.bind(v),
                Bind::Text(v) => q.bind(v),
                Bind::Time(v) => q.bind(*v),
            };
        }
        q
    }};
}

enum Bind {
    Int(i32),
    Ints(Vec<i32>),
    Text(String),
    Time(NaiveDateTime),
}

pub struct Context<'a> {
    pub settings: &'a SiteSettings,
    pub user_cx: &'a crate::admin_user_show::Context<'a>,
    pub base_path: &'a str,
}

pub async fn index(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    guardian: &Guardian,
    p: &Params,
) -> Result<Outcome, AppError> {
    let s = cx.settings;
    let page = p.page.as_deref().map_or(0, crate::ruby::to_i);
    let page_size = match p.limit.as_deref() {
        None => INDEX_LIMIT,
        Some(l) => match strict_integer(l) {
            Some(v) if (0..=INDEX_LIMIT).contains(&v) => v,
            _ => return Ok(Outcome::InvalidParameter("limit")),
        },
    };

    // staff_action_records
    let custom_staff_id = action_id("custom_staff");
    let mut action_filter = p.action_id.clone();
    let mut custom_type = p.custom_type.clone();
    let custom_staff = p
        .action_id
        .as_deref()
        .is_some_and(|a| Some(crate::ruby::to_i(a) as i32) == custom_staff_id);
    if custom_staff {
        custom_type = p.action_name.clone();
    } else if let Some(name) = &p.action_name {
        action_filter = action_id(name).map(|id| id.to_string());
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();
    // with_filters
    if let Some(a) = action_filter.filter(|a| !crate::ruby::is_blank(a)) {
        binds.push(Bind::Int(crate::ruby::to_i(&a) as i32));
        clauses.push(format!("action = ${}", binds.len()));
    }
    if let Some(c) = custom_type.filter(|c| !crate::ruby::is_blank(c)) {
        binds.push(Bind::Text(c));
        clauses.push(format!("custom_type = ${}", binds.len()));
    }
    for (key, value) in [
        ("acting_user", &p.acting_user),
        ("target_user", &p.target_user),
    ] {
        if let Some(name) = value {
            // `where("#{key}_id = ?", ids)`: no user is `= NULL`, one is `= id`.
            let ids: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM users WHERE username_lower = $1 ORDER BY id")
                    .bind(name.to_lowercase())
                    .fetch_all(&mut *conn)
                    .await?;
            match ids.as_slice() {
                [] => clauses.push("FALSE".into()),
                [id] => {
                    binds.push(Bind::Int(*id));
                    clauses.push(format!("{key}_id = ${}", binds.len()));
                }
                _ => return Err(Unsupported("several users with one username").into()),
            }
        }
    }
    if let Some(subject) = &p.subject {
        binds.push(Bind::Text(subject.clone()));
        clauses.push(format!("subject = ${}", binds.len()));
    }
    // only_staff_actions
    let staff_ids: Vec<i32> = ENUMS
        .staff_actions
        .iter()
        .filter_map(|a| action_id(a))
        .collect();
    binds.push(Bind::Ints(staff_ids));
    clauses.push(format!("action = ANY(${})", binds.len()));
    if !guardian.is_admin() {
        binds.push(Bind::Ints(moderator_visible_action_ids(s)?));
        clauses.push(format!("action = ANY(${})", binds.len()));
    }
    for (date, op) in [(&p.start_date, ">="), (&p.end_date, "<=")] {
        if let Some(d) = date {
            binds.push(Bind::Time(to_time(d)?));
            clauses.push(format!("created_at {op} ${}", binds.len()));
        }
    }
    let filter = clauses.join(" AND ");

    let count_sql = format!("SELECT COUNT(*) FROM user_histories WHERE {filter}");
    let count: i64 = bind_all!(sqlx::query_scalar(&count_sql), &binds)
        .fetch_one(&mut *conn)
        .await?;
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        action: i32,
        acting_user_id: Option<i32>,
        target_user_id: Option<i32>,
        details: Option<String>,
        context: Option<String>,
        ip_address: Option<String>,
        email: Option<String>,
        subject: Option<String>,
        previous_value: Option<String>,
        new_value: Option<String>,
        topic_id: Option<i32>,
        post_id: Option<i32>,
        category_id: Option<i32>,
        reviewable_id: Option<i64>,
        custom_type: Option<String>,
        created_at: NaiveDateTime,
    }
    let rows_sql = format!(
        "SELECT id, action, acting_user_id, target_user_id, details, context, ip_address, \
                    email, subject, previous_value, new_value, topic_id, post_id, category_id, \
                    reviewable_id::int8 AS reviewable_id, custom_type, created_at \
             FROM user_histories WHERE {filter} ORDER BY id DESC OFFSET {} LIMIT {}",
        page.max(0) * page_size,
        page_size
    );
    let rows: Vec<Row> = bind_all!(sqlx::query_as(&rows_sql), &binds)
        .fetch_all(&mut *conn)
        .await?;

    let can_see_ip =
        guardian.is_admin() || (guardian.is_moderator() && s.get("moderators_view_ips")?.truthy());
    let logo = crate::admin_users::logo_small_url(conn, s).await?;
    let json_actions: Vec<i32> = JSON_VALUE_ACTIONS
        .iter()
        .filter_map(|a| action_id(a))
        .collect();
    let mut logs = Vec::with_capacity(rows.len());
    for r in rows {
        let redact = !can_see_content(
            conn,
            guardian,
            s,
            &Refs {
                topic_id: r.topic_id,
                post_id: r.post_id,
                category_id: r.category_id,
            },
        )
        .await?;
        let name = match action_name(r.action) {
            Some("custom" | "custom_staff") => json!(r.custom_type),
            Some(n) => json!(n),
            None => json!(""),
        };
        let value = |v: &Option<String>| -> Result<Value, AppError> {
            match v {
                None => Ok(Value::Null),
                Some(v) if json_actions.contains(&r.action) => serde_json::from_str(v)
                    .map_err(|_| Unsupported("staff logs with invalid JSON values").into()),
                Some(v) => Ok(json!(v)),
            }
        };
        let mut o = Map::new();
        o.insert("action_name".into(), name);
        let redacted = cx
            .user_cx
            .i18n
            .t("staff_action_logs.redacted")
            .unwrap_or_default();
        o.insert(
            "details".into(),
            if redact {
                json!(redacted)
            } else {
                json!(r.details)
            },
        );
        o.insert(
            "context".into(),
            if redact {
                Value::Null
            } else {
                json!(r.context)
            },
        );
        o.insert(
            "ip_address".into(),
            if can_see_ip {
                json!(r.ip_address)
            } else {
                Value::Null
            },
        );
        o.insert("email".into(), json!(r.email));
        o.insert(
            "created_at".into(),
            json!(r.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()),
        );
        o.insert("subject".into(), json!(r.subject));
        let (previous, new) = if redact {
            (Value::Null, Value::Null)
        } else {
            (value(&r.previous_value)?, value(&r.new_value)?)
        };
        o.insert("previous_value".into(), previous);
        o.insert("new_value".into(), new);
        o.insert("topic_id".into(), json!(r.topic_id));
        o.insert("post_id".into(), json!(r.post_id));
        o.insert("category_id".into(), json!(r.category_id));
        if let Some(id) = r.reviewable_id {
            o.insert("reviewable_id".into(), json!(id));
        }
        o.insert("action".into(), json!(r.action));
        o.insert("custom_type".into(), json!(r.custom_type));
        o.insert("id".into(), json!(r.id));
        for (key, id) in [
            ("acting_user", r.acting_user_id),
            ("target_user", r.target_user_id),
        ] {
            let user =
                crate::admin_user_show::basic_user(conn, cx.user_cx, id, logo.as_deref()).await?;
            o.insert(key.into(), user);
        }
        logs.push(Value::Object(o));
    }

    // admin_staff_action_logs_path(permitted filters + page, page_size):
    // to_query sorts the keys.
    let mut query: Vec<(String, String)> = p.permitted.clone();
    query.push(("page".into(), (page + 1).to_string()));
    query.push(("page_size".into(), page_size.to_string()));
    query.sort_by(|a, b| a.0.cmp(&b.0));
    let query: Vec<String> = query
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                crate::ruby::cgi_escape(k),
                crate::ruby::cgi_escape(v)
            )
        })
        .collect();
    let more = format!(
        "{}/admin/logs/staff_action_logs?{}",
        cx.base_path,
        query.join("&")
    );

    let mut actions: Vec<&String> = ENUMS.staff_actions.iter().collect();
    actions.sort();
    let custom = custom_staff_id.unwrap_or_default();
    let available: Vec<Value> = actions
        .into_iter()
        .map(|name| json!({ "id": name, "action_id": action_id(name).unwrap_or(custom) }))
        .collect();

    Ok(Outcome::Json(json!({
        "staff_action_logs": logs,
        "total_rows_staff_action_logs": count,
        "load_more_staff_action_logs": more,
        "extras": { "user_history_actions": available },
    })))
}

/// What a log is about, for its content's visibility.
struct Refs {
    topic_id: Option<i32>,
    post_id: Option<i32>,
    category_id: Option<i32>,
}

/// `can_see_staff_action_log_content?`: admins see everything; others
/// the log's topic, post (its topic, and whispers if they may) and
/// category, a missing one hidden.
async fn can_see_content(
    conn: &mut PgConnection,
    guardian: &Guardian,
    s: &SiteSettings,
    refs: &Refs,
) -> Result<bool, AppError> {
    if guardian.is_admin() {
        return Ok(true);
    }
    // belongs_to goes through the trashable default scope: a deleted topic
    // or post is nil, which nobody can see.
    if let Some(topic_id) = refs.topic_id {
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if !live
            || guardian
                .can_see_topic_ids(conn, s, &[topic_id])
                .await?
                .is_empty()
        {
            return Ok(false);
        }
    }
    if let Some(post_id) = refs.post_id {
        let post: Option<(i32, i32)> = sqlx::query_as(
            "SELECT p.topic_id, p.post_type FROM posts p JOIN topics t ON t.id = p.topic_id \
                 WHERE p.id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL",
        )
        .bind(post_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((topic_id, post_type)) = post else {
            return Ok(false);
        };
        if guardian
            .can_see_topic_ids(conn, s, &[topic_id])
            .await?
            .is_empty()
        {
            return Ok(false);
        }
        // Post.types[:whisper]
        if post_type == 4 && !guardian.can_see_whispers(s)? {
            return Ok(false);
        }
    }
    if let Some(category_id) = refs.category_id
        && !guardian
            .allowed_category_ids(conn, s)
            .await?
            .contains(&category_id)
    {
        return Ok(false);
    }
    Ok(true)
}
