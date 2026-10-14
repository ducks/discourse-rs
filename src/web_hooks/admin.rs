//! The WebHook model as Admin::WebHooksController uses it: its rows and
//! associations, AdminWebHookSerializer and AdminWebHookEventSerializer,
//! the permitted params with their casts and validations, and the staff
//! log of each change.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `WebHookEventType` groups (the `group` enum), by value.
const GROUPS: [&str; 18] = [
    "topic",
    "post",
    "user",
    "group",
    "category",
    "tag",
    "reviewable",
    "notification",
    "solved",
    "assign",
    "user_badge",
    "group_user",
    "like",
    "user_promoted",
    "voting",
    "chat",
    "custom",
    "calendar",
];

/// `WebHook.last_delivery_statuses[:disabled]`
const DISABLED: i32 = 4;

/// A `web_hooks` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WebHook {
    pub id: i32,
    pub payload_url: String,
    pub content_type: i32,
    pub last_delivery_status: i32,
    pub status: i32,
    pub secret: Option<String>,
    pub wildcard_web_hook: bool,
    pub verify_certificate: bool,
    pub active: bool,
}

const COLUMNS: &str = "id, payload_url, content_type, last_delivery_status, status, secret, \
                       wildcard_web_hook, verify_certificate, active";

impl WebHook {
    /// `WebHook.find_by(id:)`
    pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<WebHook>, sqlx::Error> {
        let Ok(id) = i32::try_from(id) else {
            return Ok(None);
        };
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM web_hooks WHERE id = $1"))
            .bind(id)
            .fetch_optional(conn)
            .await
    }

    /// `WebHook.limit(limit).offset(offset)`, by id.
    async fn page(
        conn: &mut PgConnection,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WebHook>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM web_hooks ORDER BY id ASC LIMIT $1 OFFSET $2"
        ))
        .bind(limit)
        .bind(offset)
        .fetch_all(conn)
        .await
    }
}

/// A has_and_belongs_to_many association of WebHook: the record's table
/// and the join table with its key.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Habtm {
    EventTypes,
    Groups,
    Categories,
    Tags,
}

impl Habtm {
    fn tables(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Habtm::EventTypes => (
                "web_hook_event_types",
                "web_hook_event_types_hooks",
                "web_hook_event_type_id",
            ),
            Habtm::Groups => ("groups", "groups_web_hooks", "group_id"),
            Habtm::Categories => ("categories", "categories_web_hooks", "category_id"),
            Habtm::Tags => ("tags", "tags_web_hooks", "tag_id"),
        }
    }

    /// WebHookEventType's default scope orders by id; the others load in
    /// whatever order Postgres gives a query without ORDER BY.
    fn ordered(self) -> bool {
        self == Habtm::EventTypes
    }

    /// The hook's records, as `web_hook.<association>` loads them: a hash
    /// join probed by a scan of the records' table, so in its physical
    /// order (or by id for the event types).
    async fn load(self, conn: &mut PgConnection, hook_id: i32) -> Result<Vec<i32>, sqlx::Error> {
        let (table, join, key) = self.tables();
        let order = if self.ordered() { "r.id" } else { "r.ctid" };
        sqlx::query_scalar(&format!(
            "SELECT r.id::int FROM {table} r JOIN {join} j ON j.{key} = r.id \
             WHERE j.web_hook_id = $1 ORDER BY {order}"
        ))
        .bind(hook_id)
        .fetch_all(conn)
        .await
    }

    /// As `includes(...)` preloads them for a page of hooks: the join
    /// rows in their physical order (by id for the event types).
    async fn preload(
        self,
        conn: &mut PgConnection,
        hook_ids: &[i32],
    ) -> Result<Vec<(i32, i32)>, sqlx::Error> {
        let (table, join, key) = self.tables();
        let order = if self.ordered() { "r.id" } else { "j.ctid" };
        sqlx::query_as(&format!(
            "SELECT j.web_hook_id::int, r.id::int FROM {join} j JOIN {table} r ON r.id = j.{key} \
             WHERE j.web_hook_id = ANY($1) ORDER BY {order}"
        ))
        .bind(hook_ids)
        .fetch_all(conn)
        .await
    }

    /// `<association>_ids=` on a saved hook: the join rows the new list
    /// drops deleted, those it adds inserted, and the in-memory target
    /// (the kept records in their order, then the new ones in theirs).
    async fn replace(
        self,
        conn: &mut PgConnection,
        hook_id: i32,
        current: &[i32],
        ids: &[i32],
    ) -> Result<Vec<i32>, sqlx::Error> {
        let (_, join, key) = self.tables();
        let removed: Vec<i32> = current
            .iter()
            .copied()
            .filter(|id| !ids.contains(id))
            .collect();
        if !removed.is_empty() {
            sqlx::query(&format!(
                "DELETE FROM {join} WHERE web_hook_id = $1 AND {key} = ANY($2)"
            ))
            .bind(hook_id)
            .bind(&removed)
            .execute(&mut *conn)
            .await?;
        }
        let mut target: Vec<i32> = current
            .iter()
            .copied()
            .filter(|id| ids.contains(id))
            .collect();
        for id in ids {
            if !target.contains(id) {
                insert_join(conn, join, key, hook_id, *id).await?;
                target.push(*id);
            }
        }
        Ok(target)
    }
}

async fn insert_join(
    conn: &mut PgConnection,
    join: &str,
    key: &str,
    hook_id: i32,
    id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "INSERT INTO {join} (web_hook_id, {key}) VALUES ($1, $2)"
    ))
    .bind(hook_id)
    .bind(id)
    .execute(conn)
    .await?;
    Ok(())
}

/// A hook's associations, as ids in the order the serializer shows them.
#[derive(Debug, Clone, Default)]
pub struct Associations {
    pub event_type_ids: Vec<i32>,
    pub group_ids: Vec<i32>,
    pub category_ids: Vec<i32>,
    pub tag_ids: Vec<i32>,
}

impl Associations {
    /// The hook's associations loaded one by one.
    pub async fn load(conn: &mut PgConnection, hook_id: i32) -> Result<Associations, sqlx::Error> {
        Ok(Associations {
            event_type_ids: Habtm::EventTypes.load(conn, hook_id).await?,
            group_ids: Habtm::Groups.load(conn, hook_id).await?,
            category_ids: Habtm::Categories.load(conn, hook_id).await?,
            tag_ids: Habtm::Tags.load(conn, hook_id).await?,
        })
    }

    /// Each hook's associations preloaded for a page.
    async fn preload(
        conn: &mut PgConnection,
        hook_ids: &[i32],
    ) -> Result<Vec<Associations>, sqlx::Error> {
        let mut out = vec![Associations::default(); hook_ids.len()];
        for habtm in [
            Habtm::EventTypes,
            Habtm::Groups,
            Habtm::Categories,
            Habtm::Tags,
        ] {
            for (hook_id, id) in habtm.preload(conn, hook_ids).await? {
                let Some(i) = hook_ids.iter().position(|h| *h == hook_id) else {
                    continue;
                };
                let a = &mut out[i];
                match habtm {
                    Habtm::EventTypes => a.event_type_ids.push(id),
                    Habtm::Groups => a.group_ids.push(id),
                    Habtm::Categories => a.category_ids.push(id),
                    Habtm::Tags => a.tag_ids.push(id),
                }
            }
        }
        Ok(out)
    }
}

/// What the serializers read besides the rows.
pub struct Cx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub base_path: &'a str,
}

/// WebHookEventTypeSerializer for each id.
async fn event_types(conn: &mut PgConnection, ids: &[i32]) -> Result<Vec<Value>, AppError> {
    let rows: Vec<(i32, String, Option<i32>)> =
        sqlx::query_as("SELECT id, name, \"group\" FROM web_hook_event_types WHERE id = ANY($1)")
            .bind(ids)
            .fetch_all(&mut *conn)
            .await?;
    Ok(ids
        .iter()
        .filter_map(|id| rows.iter().find(|r| r.0 == *id))
        .map(event_type_json)
        .collect())
}

fn event_type_json((id, name, group): &(i32, String, Option<i32>)) -> Value {
    let group = group.and_then(|g| GROUPS.get(g as usize).copied());
    json!({ "id": id, "name": name, "group": group })
}

/// TagSerializer for each id, as the admin sees them.
async fn tags(conn: &mut PgConnection, cx: &Cx<'_>, ids: &[i32]) -> Result<Vec<Value>, AppError> {
    let rows = crate::tags::Tag::find_all(&mut *conn, ids).await?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(tag) = rows.iter().find(|t| t.id == *id) {
            out.push(tag.serialize(&mut *conn, cx.guardian, cx.settings).await?);
        }
    }
    Ok(out)
}

/// AdminWebHookSerializer, without its side-loaded categories.
pub async fn serialize(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    hook: &WebHook,
    a: &Associations,
) -> Result<Value, AppError> {
    let last_delivery_status = if hook.active {
        hook.last_delivery_status
    } else {
        DISABLED
    };
    Ok(json!({
        "id": hook.id,
        "payload_url": hook.payload_url,
        "content_type": hook.content_type,
        "last_delivery_status": last_delivery_status,
        "secret": hook.secret,
        "wildcard_web_hook": hook.wildcard_web_hook,
        "verify_certificate": hook.verify_certificate,
        "active": hook.active,
        "category_ids": a.category_ids,
        "tags": tags(conn, cx, &a.tag_ids).await?,
        "group_ids": a.group_ids,
        "web_hook_event_types": event_types(conn, &a.event_type_ids).await?,
    }))
}

/// The `categories` AMS side-loads for `has_many :categories, embed: :ids,
/// include: true`: BasicCategorySerializer for each, once, in the order
/// the hooks list them.
pub async fn categories(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    ids: &[i32],
) -> Result<Value, AppError> {
    let mut unique: Vec<i32> = Vec::new();
    for id in ids {
        if !unique.contains(id) {
            unique.push(*id);
        }
    }
    let mut out = Vec::new();
    if unique.is_empty() {
        return Ok(Value::Array(out));
    }
    let rows = crate::categories::Categories::load_all(&mut *conn).await?;
    let mut cats = crate::categories::Categories {
        conn: &mut *conn,
        settings: cx.settings,
        i18n: cx.i18n,
        guardian: cx.guardian,
        base_path: cx.base_path,
        topic_url_via_slug: false,
    };
    // can_edit?(category): admins, or moderators who manage categories.
    let can_edit = cx.guardian.is_admin()
        || (cx.settings.get("moderators_manage_categories")?.truthy()
            && cx.guardian.is_moderator());
    for id in unique {
        let Some(row) = rows.iter().find(|r| r.id == id) else {
            continue;
        };
        let fields = cats.basic_fields(row).await?;
        let mut c = Map::new();
        for (k, v) in fields {
            let after_notification_level = k == "notification_level";
            c.insert(k, v);
            if after_notification_level && can_edit {
                c.insert("can_edit".into(), json!(true));
            }
        }
        c.insert("has_children".into(), Value::Null);
        c.insert("subcategory_count".into(), Value::Null);
        cats.uploads(&mut c, row).await?;
        out.push(Value::Object(c));
    }
    Ok(Value::Array(out))
}

/// `Admin::WebHooksController#index`'s JSON.
pub async fn index(conn: &mut PgConnection, cx: &Cx<'_>, offset: i64) -> Result<Value, AppError> {
    const LIMIT: i64 = 50;
    let hooks = WebHook::page(&mut *conn, LIMIT, offset).await?;
    let ids: Vec<i32> = hooks.iter().map(|h| h.id).collect();
    let associations = Associations::preload(&mut *conn, &ids).await?;
    let mut web_hooks = Vec::new();
    let mut category_ids = Vec::new();
    for (hook, a) in hooks.iter().zip(&associations) {
        web_hooks.push(serialize(&mut *conn, cx, hook, a).await?);
        category_ids.extend(a.category_ids.iter().copied());
    }
    let categories = categories(&mut *conn, cx, &category_ids).await?;

    // WebHookEventType.active_grouped: the plugins' types only while their
    // plugin is on.
    let enabled =
        |name: &str| -> bool { cx.settings.get(name).map(|v| v.truthy()).unwrap_or(false) };
    let mut excluded: Vec<i32> = Vec::new();
    for (setting, ids) in [
        ("solved_enabled", &[1101, 1102][..]),
        ("assign_enabled", &[1201, 1202][..]),
        ("topic_voting_enabled", &[1701, 1702][..]),
        ("chat_enabled", &[1801, 1802, 1803, 1804][..]),
        ("discourse_post_event_enabled", &[1901, 1902, 1903][..]),
    ] {
        if !enabled(setting) {
            excluded.extend_from_slice(ids);
        }
    }
    let types: Vec<(i32, String, Option<i32>)> = sqlx::query_as(
        "SELECT id, name, \"group\" FROM web_hook_event_types \
         WHERE NOT (id = ANY($1)) AND \"group\" IS NOT NULL ORDER BY id ASC",
    )
    .bind(&excluded)
    .fetch_all(&mut *conn)
    .await?;
    let mut grouped = Map::new();
    for t in &types {
        let group =
            t.2.and_then(|g| GROUPS.get(g as usize).copied())
                .unwrap_or_default();
        if let Value::Array(list) = grouped
            .entry(group.to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
        {
            list.push(event_type_json(t));
        }
    }
    let default_types = event_types(&mut *conn, &[201, 202, 203, 204]).await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM web_hooks")
        .fetch_one(&mut *conn)
        .await?;
    Ok(json!({
        "web_hooks": web_hooks,
        "extras": {
            "categories": categories,
            "grouped_event_types": grouped,
            "default_event_types": default_types,
            "content_types": [
                {"id": 1, "name": "application/json"},
                {"id": 2, "name": "application/x-www-form-urlencoded"},
            ],
            "delivery_statuses": [
                {"id": 1, "name": "inactive"},
                {"id": 2, "name": "failed"},
                {"id": 3, "name": "successful"},
                {"id": 4, "name": "disabled"},
            ],
        },
        "total_rows_web_hooks": total,
        "load_more_web_hooks": format!(
            "{}/admin/api/web_hooks.json?limit={LIMIT}&offset={}",
            cx.base_path,
            offset + LIMIT
        ),
    }))
}

/// `serialize_data(@web_hook, AdminWebHookSerializer, root: "web_hook")`:
/// the categories and the hook, as AMS lays them out.
pub async fn rooted(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    hook: &WebHook,
    a: &Associations,
) -> Result<(Value, Value), AppError> {
    let body = serialize(&mut *conn, cx, hook, a).await?;
    let categories = categories(&mut *conn, cx, &a.category_ids).await?;
    Ok((categories, body))
}

/// A `web_hook_events` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Event {
    pub id: i64,
    pub web_hook_id: i32,
    pub headers: Option<String>,
    pub payload: Option<String>,
    pub status: Option<i32>,
    pub response_headers: Option<String>,
    pub response_body: Option<String>,
    pub duration: Option<i32>,
    pub created_at: chrono::NaiveDateTime,
    pub redelivering: bool,
}

pub const EVENT_COLUMNS: &str = "e.id, e.web_hook_id, e.headers, e.payload, e.status, \
     e.response_headers, e.response_body, e.duration, e.created_at, \
     EXISTS (SELECT 1 FROM redelivering_webhook_events r WHERE r.web_hook_event_id = e.id) AS redelivering";

impl Event {
    pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<Event>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {EVENT_COLUMNS} FROM web_hook_events e WHERE e.id = $1"
        ))
        .bind(id)
        .fetch_optional(conn)
        .await
    }
}

/// AdminWebHookEventSerializer
pub fn serialize_event(e: &Event, request_url: &str) -> Value {
    json!({
        "id": e.id,
        "web_hook_id": e.web_hook_id,
        "request_url": request_url,
        "headers": e.headers,
        "payload": e.payload,
        "status": e.status,
        "response_headers": e.response_headers,
        "response_body": e.response_body,
        "duration": e.duration,
        "created_at": e.created_at.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        "redelivering": e.redelivering,
    })
}

/// `WebHookEvent.successful` / `.failed`, or every event.
fn status_filter(status: Option<&str>) -> &'static str {
    match status {
        Some("successful") => " AND e.status >= 200 AND e.status <= 299",
        Some("failed") => " AND (e.status < 200 OR e.status > 299)",
        _ => "",
    }
}

/// `Admin::WebHooksController#list_events`'s JSON.
pub async fn list_events(
    conn: &mut PgConnection,
    base_path: &str,
    hook: &WebHook,
    offset: i64,
    status: Option<&str>,
) -> Result<Value, AppError> {
    const LIMIT: i64 = 50;
    let filter = status_filter(status);
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM web_hook_events e WHERE e.web_hook_id = $1{filter}"
    ))
    .bind(hook.id)
    .fetch_one(&mut *conn)
    .await?;
    let events: Vec<Event> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLUMNS} FROM web_hook_events e WHERE e.web_hook_id = $1{filter} \
         ORDER BY e.created_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(hook.id)
    .bind(LIMIT)
    .bind(offset)
    .fetch_all(&mut *conn)
    .await?;
    let mut load_more = format!(
        "{base_path}/admin/api/web_hook_events/{}.json?limit={LIMIT}&offset={}",
        hook.id,
        offset + LIMIT
    );
    if let Some(status) = status {
        load_more.push_str("&status=");
        load_more.extend(form_urlencoded::byte_serialize(status.as_bytes()));
    }
    Ok(json!({
        "web_hook_events": events.iter().map(|e| serialize_event(e, &hook.payload_url)).collect::<Vec<_>>(),
        "total_rows_web_hook_events": total,
        "load_more_web_hook_events": load_more,
        "extras": { "web_hook_id": hook.id },
    }))
}

/// `@web_hook.web_hook_events.where(id: ids)`, newest first.
pub async fn events_by_id(
    conn: &mut PgConnection,
    hook: &WebHook,
    ids: &[i64],
    not_ping: bool,
) -> Result<Vec<Event>, sqlx::Error> {
    let ping = if not_ping { " AND e.status <> 0" } else { "" };
    sqlx::query_as(&format!(
        "SELECT {EVENT_COLUMNS} FROM web_hook_events e \
         WHERE e.web_hook_id = $1 AND e.id = ANY($2){ping} ORDER BY e.created_at DESC"
    ))
    .bind(hook.id)
    .bind(ids)
    .fetch_all(conn)
    .await
}

/// `web_hook_params`: what `params.require(:web_hook).permit(...)` keeps,
/// cast as the attributes take them. None for a field not given.
#[derive(Debug, Default)]
pub struct Params {
    pub payload_url: Option<Option<String>>,
    pub content_type: Option<Option<i32>>,
    pub secret: Option<Option<String>>,
    pub wildcard_web_hook: Option<Option<bool>>,
    pub active: Option<Option<bool>>,
    pub verify_certificate: Option<Option<bool>>,
    pub web_hook_event_type_ids: Option<Vec<String>>,
    pub group_ids: Option<Vec<String>>,
    pub tag_names: Option<Vec<String>>,
    pub tag_ids: Option<Vec<String>>,
    pub category_ids: Option<Vec<String>>,
}

/// ActiveModel's boolean cast: blank is nil, the false values false.
fn cast_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Null => None,
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(!matches!(
            s.as_str(),
            "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF"
        )),
        _ => None,
    }
}

/// ActiveModel's integer cast: blank is nil, else `to_i`.
fn cast_int(v: &Value) -> Option<i32> {
    match v {
        Value::Bool(b) => Some(i32::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .map(|n| n as i32),
        Value::String(s) if s.trim().is_empty() => None,
        Value::String(s) => Some(crate::ruby::to_i(s) as i32),
        _ => None,
    }
}

/// The permitted scalars of an array param (`key: []`).
fn scalars(v: &Value) -> Option<Vec<String>> {
    let items = v.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|i| match i {
                Value::Null => Some(String::new()),
                other => crate::params::scalar(other),
            })
            .collect(),
    )
}

impl Params {
    /// None when `web_hook` is missing or empty (ParameterMissing).
    pub fn permit(p: &Map<String, Value>) -> Option<Params> {
        let hook = p.get("web_hook")?.as_object()?;
        if hook.is_empty() {
            return None;
        }
        let scalar = |k: &str| -> Option<&Value> {
            hook.get(k)
                .filter(|v| !matches!(v, Value::Array(_) | Value::Object(_)))
        };
        let string = |k: &str| scalar(k).map(crate::params::scalar);
        let mut out = Params {
            payload_url: string("payload_url"),
            content_type: scalar("content_type").map(cast_int),
            secret: string("secret"),
            wildcard_web_hook: scalar("wildcard_web_hook").map(cast_bool),
            active: scalar("active").map(cast_bool),
            verify_certificate: scalar("verify_certificate").map(cast_bool),
            web_hook_event_type_ids: hook.get("web_hook_event_type_ids").and_then(scalars),
            group_ids: hook.get("group_ids").and_then(scalars),
            tag_names: hook.get("tag_names").and_then(scalars),
            tag_ids: hook.get("tag_ids").and_then(scalars),
            category_ids: hook.get("category_ids").and_then(scalars),
        };
        // The deprecated tag_names wins over tag_ids when present.
        if out.tag_names.as_ref().is_some_and(|n| !n.is_empty()) {
            out.tag_ids = None;
        } else {
            out.tag_names = None;
        }
        Some(out)
    }
}

/// `ids_writer`'s ids: blanks dropped, each cast to an integer.
fn ids(raw: &[String]) -> Vec<i32> {
    raw.iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| crate::ruby::to_i(s) as i32)
        .collect()
}

/// Why a write did not happen.
pub enum Refused {
    /// The validation errors' full messages (422).
    Invalid(Vec<String>),
    /// An id that names no record (RecordNotFound, 404).
    NotFound,
}

/// The attributes as they will be saved.
#[derive(Debug, Clone)]
struct Attrs {
    payload_url: Option<String>,
    content_type: Option<i32>,
    secret: Option<String>,
    wildcard_web_hook: Option<bool>,
    active: Option<bool>,
    verify_certificate: Option<bool>,
}

/// The ids among `ids` that exist in the association's table, or None
/// when one does not (`ids_writer` raises RecordNotFound).
async fn existing(
    conn: &mut PgConnection,
    habtm: Habtm,
    ids: &[i32],
) -> Result<Option<Vec<i32>>, sqlx::Error> {
    let (table, _, _) = habtm.tables();
    let found: Vec<i32> = sqlx::query_scalar(&format!("SELECT id FROM {table} WHERE id = ANY($1)"))
        .bind(ids)
        .fetch_all(conn)
        .await?;
    if ids.iter().all(|id| found.contains(id)) {
        let mut unique = Vec::new();
        for id in ids {
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        Ok(Some(unique))
    } else {
        Ok(None)
    }
}

/// `tag_ids=`: `Tag.where(id: ids)`, in the table's order.
async fn tag_ids_assigned(conn: &mut PgConnection, ids: &[i32]) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM tags WHERE id = ANY($1) ORDER BY ctid")
        .bind(ids)
        .fetch_all(conn)
        .await
}

/// `DiscourseTagging.clean_tag`
fn clean_tag(settings: &SiteSettings, tag: &str) -> Result<String, AppError> {
    let mut tag = tag.to_string();
    if settings.get("force_lowercase_tags")?.truthy() {
        tag = tag.to_lowercase();
    }
    let tag = tag.trim();
    // [[:space:]]+ to a dash
    let mut dashed = String::new();
    let mut in_space = false;
    for c in tag.chars() {
        if c.is_whitespace() {
            if !in_space {
                dashed.push('-');
            }
            in_space = true;
        } else {
            dashed.push(c);
            in_space = false;
        }
    }
    // [^[:word:][:punct:]]+ dropped, then TAGS_FILTER_REGEXP's characters.
    const FILTERED: &str = "/?#[]@!$&'()*+,;=%\\`^|{}\"<>";
    let kept: String = dashed
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || c.is_ascii_punctuation())
        .filter(|c| !FILTERED.contains(*c) && !c.is_whitespace())
        .collect();
    // squeeze("-")
    let mut squeezed = String::new();
    for c in kept.chars() {
        if c == '-' && squeezed.ends_with('-') {
            continue;
        }
        squeezed.push(c);
    }
    let max = settings.get("max_tag_length")?.to_i().max(0) as usize;
    let truncated: String = squeezed.chars().take(max).collect();
    Ok(truncated.trim_matches('.').to_string())
}

/// `tag_names=`: DiscourseTagging.add_or_create_tags_by_name with the
/// system user, unlimited: the tags named, their synonyms, and new tags for
/// names no tag has (created at once, whatever becomes of the hook). None
/// when the hook's tags already have those names.
async fn tag_names_assigned(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    current: &[i32],
    names: &[String],
) -> Result<Option<Vec<i32>>, AppError> {
    // tags_for_saving: the system user may tag and create tags while
    // tagging is on (it is in the admins group the defaults allow).
    if !settings.get("tagging_enabled")?.truthy() || names.is_empty() {
        return Ok(None);
    }
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let mut tag_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?;
    let existing = tag_names.clone();
    for name in names.iter().filter(|n| !existing.contains(n)) {
        tag_names.push(clean_tag(settings, name)?);
    }
    tag_names.retain(|t| !t.trim().is_empty());
    let mut unique: Vec<String> = Vec::new();
    for t in tag_names {
        if !unique.contains(&t) {
            unique.push(t);
        }
    }
    let tag_names = unique;

    let mut current_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM tags WHERE id = ANY($1)")
            .bind(current)
            .fetch_all(&mut *conn)
            .await?;
    current_names.sort();
    let mut wanted = tag_names.clone();
    wanted.sort();
    if current_names == wanted {
        return Ok(None);
    }
    let lower: Vec<String> = tag_names.iter().map(|n| n.to_lowercase()).collect();
    let found: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?;
    let mut ids: Vec<i32> = found.iter().map(|(id, _)| *id).collect();
    let new_names: Vec<String> = if found.len() < tag_names.len() {
        tag_names
            .iter()
            .filter(|n| !found.iter().any(|(_, f)| f == *n))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let synonyms: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM tags WHERE target_tag_id = ANY($1) AND NOT (id = ANY($1)) ORDER BY ctid",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    ids.extend(synonyms);
    for name in new_names {
        ids.push(create_tag(conn, settings, &name).await?);
    }
    Ok(Some(ids))
}

/// `Tag.create(name:)`: its slug, the row, its search data and its
/// tag_created web hooks.
async fn create_tag(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<i32, AppError> {
    if ["none", "constructor"].contains(&name.trim().to_lowercase().as_str()) {
        return Err(Unsupported("a reserved tag name through a web hook's tag_names").into());
    }
    // ensure_slug: Slug.for(name, ""), none when another tag has it.
    let slug = crate::posting::text::slug_with_default(name, "en", "")?;
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tags WHERE lower(slug) = lower($1))")
            .bind(&slug)
            .fetch_one(&mut *conn)
            .await?;
    let slug = if slug.is_empty() || taken {
        String::new()
    } else {
        slug
    };
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tags (name, slug, created_at, updated_at) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(name)
    .bind(&slug)
    .fetch_one(&mut *conn)
    .await?;
    crate::posting::search_index::index_tag(conn, settings, id, &name.to_lowercase()).await?;
    // after_commit: tag_created.
    if super::active(conn, "tag_created").await? {
        let tag = crate::tags::Tag::find(conn, id)
            .await?
            .ok_or(Unsupported("a tag that vanished"))?;
        let guardian = super::system_guardian(conn).await?;
        let payload = tag.serialize(conn, &guardian, settings).await?;
        let mut opts = Map::new();
        opts.insert("id".into(), json!(id));
        opts.insert("payload".into(), json!(super::to_json(&payload)));
        super::enqueue_hooks(conn, "tag", "tag_created", opts).await?;
    }
    Ok(id)
}

/// `URI.regexp(%w[http https])`, unanchored: an http or https scheme
/// somewhere in the text.
fn url_format_valid(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("http:") || lower.contains("https:")
}

/// `URI(url).hostname`: None when Ruby would fail on it (an invalid URI
/// raises, a URI without a host gives nil): both are a 500 there.
fn hostname(url: &str) -> Option<String> {
    if !crate::final_destination::ruby_uri_valid(url) {
        return None;
    }
    let (_, rest) = url.split_once("://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..end].rsplit('@').next()?;
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next()?.to_string()
    } else {
        authority.split(':').next()?.to_string()
    };
    (!host.is_empty()).then_some(host)
}

/// The validations, in their declaration order: full messages.
async fn validate(
    i18n: &I18n,
    fetcher: &crate::final_destination::Fetcher<'_>,
    a: &Attrs,
    event_types: &[i32],
    url_changed: bool,
) -> Result<Vec<String>, AppError> {
    let t = |key: &str, default: &str| i18n.t(key).unwrap_or(default).to_string();
    let blank = t("errors.messages.blank", "can't be blank");
    let mut errors = Vec::new();
    let url = a.payload_url.as_deref().unwrap_or("");
    if url.trim().is_empty() {
        errors.push(format!("Payload url {blank}"));
    }
    if !url_format_valid(url) {
        errors.push(format!(
            "Payload url {}",
            t(
                "activerecord.errors.models.web_hook.attributes.payload_url.invalid",
                "is invalid"
            )
        ));
    }
    if let Some(secret) = a.secret.as_deref().filter(|s| !s.trim().is_empty())
        && secret.chars().count() < 12
    {
        let msg = i18n
            .t_count("errors.messages.too_short", 12, &[])
            .unwrap_or_else(|| "is too short (minimum is 12 characters)".into());
        errors.push(format!("Secret {msg}"));
    }
    if a.content_type.is_none() {
        errors.push(format!("Content type {blank}"));
    }
    if !a.wildcard_web_hook.unwrap_or(false) && event_types.is_empty() {
        errors.push(format!("Web hook event types {blank}"));
    }
    // ensure_payload_url_allowed, when the url changed.
    if url_changed && !url.trim().is_empty() {
        let Some(host) = hostname(url.trim()) else {
            return Err(Unsupported("a web hook payload url Ruby's URI cannot read").into());
        };
        match fetcher.lookup_and_filter_ips(&host).await? {
            crate::final_destination::Lookup::Allowed(_) => {}
            crate::final_destination::Lookup::Disallowed => errors.push(t(
                "webhooks.payload_url.blocked_or_internal",
                "Payload URL cannot be used because it resolves to a blocked or internal IP",
            )),
            crate::final_destination::Lookup::Failed => {
                return Err(std::io::Error::other("FinalDestination: lookup failed").into());
            }
        }
    }
    Ok(errors)
}

/// `strip_url`: blank is nil.
fn stripped(url: Option<&str>) -> Option<String> {
    url.map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
}

/// What a write needs.
pub struct Ctx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub urls: Urls<'a>,
    pub admin_id: i32,
}

/// The assigned associations of a new hook, checked.
struct Assigned {
    event_types: Vec<i32>,
    groups: Vec<i32>,
    categories: Vec<i32>,
    tags: Vec<i32>,
}

/// `WebHook.new(web_hook_params).save`, then `log_web_hook` (create).
pub async fn create(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    p: &Params,
) -> Result<Result<(WebHook, Associations), Refused>, AppError> {
    let mut assigned = Assigned {
        event_types: Vec::new(),
        groups: Vec::new(),
        categories: Vec::new(),
        tags: Vec::new(),
    };
    for (habtm, raw) in [
        (Habtm::EventTypes, &p.web_hook_event_type_ids),
        (Habtm::Groups, &p.group_ids),
    ] {
        if let Some(raw) = raw {
            let Some(found) = existing(conn, habtm, &ids(raw)).await? else {
                return Ok(Err(Refused::NotFound));
            };
            match habtm {
                Habtm::EventTypes => assigned.event_types = found,
                _ => assigned.groups = found,
            }
        }
    }
    if let Some(names) = &p.tag_names {
        if let Some(tags) = tag_names_assigned(conn, cx.settings, &[], names).await? {
            assigned.tags = tags;
        }
    } else if let Some(raw) = &p.tag_ids {
        assigned.tags = tag_ids_assigned(conn, &ids(raw)).await?;
    }
    if let Some(raw) = &p.category_ids {
        let Some(found) = existing(conn, Habtm::Categories, &ids(raw)).await? else {
            return Ok(Err(Refused::NotFound));
        };
        assigned.categories = found;
    }
    let attrs = Attrs {
        payload_url: p.payload_url.clone().flatten(),
        content_type: p.content_type.unwrap_or(Some(1)),
        secret: p.secret.clone().unwrap_or(Some(String::new())),
        wildcard_web_hook: p.wildcard_web_hook.unwrap_or(Some(false)),
        active: p.active.unwrap_or(Some(false)),
        verify_certificate: p.verify_certificate.unwrap_or(Some(true)),
    };
    let fetcher = crate::final_destination::Fetcher {
        settings: cx.settings,
        urls: Urls {
            config: cx.urls.config,
            settings: cx.settings,
        },
    };
    let url_changed = attrs.payload_url.is_some();
    let errors = validate(
        cx.i18n,
        &fetcher,
        &attrs,
        &assigned.event_types,
        url_changed,
    )
    .await?;
    if !errors.is_empty() {
        return Ok(Err(Refused::Invalid(errors)));
    }
    let hook: WebHook = sqlx::query_as(&format!(
        "INSERT INTO web_hooks (payload_url, content_type, last_delivery_status, status, secret, \
                                wildcard_web_hook, verify_certificate, active, created_at, updated_at) \
         VALUES ($1, $2, 1, 1, $3, $4, $5, $6, clock_timestamp(), clock_timestamp()) \
         RETURNING {COLUMNS}"
    ))
    .bind(stripped(attrs.payload_url.as_deref()))
    .bind(attrs.content_type)
    .bind(&attrs.secret)
    .bind(attrs.wildcard_web_hook)
    .bind(attrs.verify_certificate)
    .bind(attrs.active)
    .fetch_one(&mut *conn)
    .await?;
    // The habtm rows, by the associations' declaration order.
    for (habtm, list) in [
        (Habtm::EventTypes, &assigned.event_types),
        (Habtm::Groups, &assigned.groups),
        (Habtm::Categories, &assigned.categories),
        (Habtm::Tags, &assigned.tags),
    ] {
        let (_, join, key) = habtm.tables();
        for id in list {
            insert_join(conn, join, key, hook.id, *id).await?;
        }
    }
    log(
        conn,
        cx.admin_id,
        "web_hook_create",
        &hook,
        None::<(Vec<String>, Vec<String>)>,
    )
    .await?;
    Ok(Ok((
        hook,
        Associations {
            event_type_ids: assigned.event_types,
            group_ids: assigned.groups,
            category_ids: assigned.categories,
            tag_ids: assigned.tags,
        },
    )))
}

/// `@web_hook.update(web_hook_params)` (in a transaction the caller rolls
/// back when refused), then `log_web_hook` (update) with the saved changes.
pub async fn update(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    hook: &WebHook,
    p: &Params,
) -> Result<Result<(WebHook, Associations), Refused>, AppError> {
    let mut a = Associations::load(conn, hook.id).await?;
    // The habtm writers write as they are assigned.
    if let Some(raw) = &p.web_hook_event_type_ids {
        let Some(found) = existing(conn, Habtm::EventTypes, &ids(raw)).await? else {
            return Ok(Err(Refused::NotFound));
        };
        a.event_type_ids = Habtm::EventTypes
            .replace(conn, hook.id, &a.event_type_ids, &found)
            .await?;
    }
    if let Some(raw) = &p.group_ids {
        let Some(found) = existing(conn, Habtm::Groups, &ids(raw)).await? else {
            return Ok(Err(Refused::NotFound));
        };
        a.group_ids = Habtm::Groups
            .replace(conn, hook.id, &a.group_ids, &found)
            .await?;
    }
    if let Some(names) = &p.tag_names {
        if let Some(tags) = tag_names_assigned(conn, cx.settings, &a.tag_ids, names).await? {
            a.tag_ids = Habtm::Tags
                .replace(conn, hook.id, &a.tag_ids, &tags)
                .await?;
        }
    } else if let Some(raw) = &p.tag_ids {
        let tags = tag_ids_assigned(conn, &ids(raw)).await?;
        a.tag_ids = Habtm::Tags
            .replace(conn, hook.id, &a.tag_ids, &tags)
            .await?;
    }
    if let Some(raw) = &p.category_ids {
        let Some(found) = existing(conn, Habtm::Categories, &ids(raw)).await? else {
            return Ok(Err(Refused::NotFound));
        };
        a.category_ids = Habtm::Categories
            .replace(conn, hook.id, &a.category_ids, &found)
            .await?;
    }
    let attrs = Attrs {
        payload_url: match &p.payload_url {
            Some(v) => v.clone(),
            None => Some(hook.payload_url.clone()),
        },
        content_type: p.content_type.unwrap_or(Some(hook.content_type)),
        secret: p.secret.clone().unwrap_or(hook.secret.clone()),
        wildcard_web_hook: p.wildcard_web_hook.unwrap_or(Some(hook.wildcard_web_hook)),
        active: p.active.unwrap_or(Some(hook.active)),
        verify_certificate: p
            .verify_certificate
            .unwrap_or(Some(hook.verify_certificate)),
    };
    let url_changed = attrs.payload_url.as_deref() != Some(hook.payload_url.as_str());
    let fetcher = crate::final_destination::Fetcher {
        settings: cx.settings,
        urls: Urls {
            config: cx.urls.config,
            settings: cx.settings,
        },
    };
    let errors = validate(cx.i18n, &fetcher, &attrs, &a.event_type_ids, url_changed).await?;
    if !errors.is_empty() {
        return Ok(Err(Refused::Invalid(errors)));
    }
    // saved_changes, less updated_at, by name.
    let new_url = stripped(attrs.payload_url.as_deref());
    let mut changes: Vec<(&str, String, String)> = Vec::new();
    let show = |v: Option<String>| v.unwrap_or_default();
    if new_url.as_deref() != Some(hook.payload_url.as_str()) {
        changes.push((
            "payload_url",
            hook.payload_url.clone(),
            show(new_url.clone()),
        ));
    }
    if attrs.content_type != Some(hook.content_type) {
        changes.push((
            "content_type",
            hook.content_type.to_string(),
            show(attrs.content_type.map(|c| c.to_string())),
        ));
    }
    if attrs.secret != hook.secret {
        changes.push((
            "secret",
            show(hook.secret.clone()),
            show(attrs.secret.clone()),
        ));
    }
    for (name, old, new) in [
        (
            "wildcard_web_hook",
            hook.wildcard_web_hook,
            attrs.wildcard_web_hook,
        ),
        ("active", hook.active, attrs.active),
        (
            "verify_certificate",
            hook.verify_certificate,
            attrs.verify_certificate,
        ),
    ] {
        if new != Some(old) {
            changes.push((name, old.to_string(), show(new.map(|b| b.to_string()))));
        }
    }
    changes.sort_by(|x, y| x.0.cmp(y.0));
    let saved = if changes.is_empty() {
        hook.clone()
    } else {
        sqlx::query_as(&format!(
            "UPDATE web_hooks SET payload_url = $2, content_type = $3, secret = $4, \
                    wildcard_web_hook = $5, verify_certificate = $6, active = $7, \
                    updated_at = clock_timestamp() \
             WHERE id = $1 RETURNING {COLUMNS}"
        ))
        .bind(hook.id)
        .bind(&new_url)
        .bind(attrs.content_type)
        .bind(&attrs.secret)
        .bind(attrs.wildcard_web_hook)
        .bind(attrs.verify_certificate)
        .bind(attrs.active)
        .fetch_one(&mut *conn)
        .await?
    };
    let old: Vec<String> = changes
        .iter()
        .map(|(k, v, _)| format!("{k}: {v}"))
        .collect();
    let new: Vec<String> = changes
        .iter()
        .map(|(k, _, v)| format!("{k}: {v}"))
        .collect();
    log(
        conn,
        cx.admin_id,
        "web_hook_update",
        &saved,
        Some((old, new)),
    )
    .await?;
    Ok(Ok((saved, a)))
}

/// `@web_hook.destroy!`: its events and daily aggregates, its join rows,
/// the row; then `log_web_hook` (destroy).
pub async fn destroy(
    conn: &mut PgConnection,
    admin_id: i32,
    hook: &WebHook,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM web_hook_events WHERE web_hook_id = $1")
        .bind(hook.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM web_hook_events_daily_aggregates WHERE web_hook_id = $1")
        .bind(hook.id)
        .execute(&mut *conn)
        .await?;
    for habtm in [
        Habtm::EventTypes,
        Habtm::Groups,
        Habtm::Categories,
        Habtm::Tags,
    ] {
        let (_, join, _) = habtm.tables();
        sqlx::query(&format!("DELETE FROM {join} WHERE web_hook_id = $1"))
            .bind(hook.id)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("DELETE FROM web_hooks WHERE id = $1")
        .bind(hook.id)
        .execute(&mut *conn)
        .await?;
    log(
        conn,
        admin_id,
        "web_hook_destroy",
        hook,
        None::<(Vec<String>, Vec<String>)>,
    )
    .await
}

/// `StaffActionLogger#log_web_hook`
async fn log(
    conn: &mut PgConnection,
    admin_id: i32,
    action: &str,
    hook: &WebHook,
    changes: Option<(Vec<String>, Vec<String>)>,
) -> Result<(), AppError> {
    let action = crate::staff_action_logs::action_id(action)
        .ok_or(Unsupported("an unknown staff action"))?;
    let context = format!("webhook_id: {}, payload_url: {}", hook.id, hook.payload_url);
    let (old, new) = match changes {
        Some((old, new)) => (Some(old.join(", ")), Some(new.join(", "))),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, context, previous_value, new_value, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(admin_id)
    .bind(context)
    .bind(old)
    .bind(new)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `RedeliveringWebhookEvent.create!` for each event without one; the
/// events' ids.
pub async fn redeliver_failed(
    conn: &mut PgConnection,
    hook: &WebHook,
    ids: &[i64],
) -> Result<Option<Vec<i64>>, AppError> {
    let events = events_by_id(&mut *conn, hook, ids, true).await?;
    if events.is_empty() {
        return Ok(None);
    }
    for e in &events {
        if !e.redelivering {
            sqlx::query(
                "INSERT INTO redelivering_webhook_events (web_hook_event_id, processing, created_at, updated_at) \
                 VALUES ($1, FALSE, clock_timestamp(), clock_timestamp())",
            )
            .bind(e.id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(Some(events.iter().map(|e| e.id).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_format_is_an_unanchored_scheme_match() {
        assert!(url_format_valid("http://x.test/hook"));
        assert!(url_format_valid("  HTTPS://x.test"));
        assert!(url_format_valid("see http:"));
        assert!(!url_format_valid("ftp://x.test/hook"));
        assert!(!url_format_valid(""));
    }

    #[test]
    fn hostnames_as_uri_reads_them() {
        assert_eq!(hostname("http://x.test:8099/a").as_deref(), Some("x.test"));
        assert_eq!(hostname("http://[::1]/a").as_deref(), Some("::1"));
        assert_eq!(hostname("x.test/hook"), None);
    }

    #[test]
    fn casts_like_active_model() {
        assert_eq!(cast_bool(&json!("false")), Some(false));
        assert_eq!(cast_bool(&json!("0")), Some(false));
        assert_eq!(cast_bool(&json!("yes")), Some(true));
        assert_eq!(cast_bool(&json!("")), None);
        assert_eq!(cast_int(&json!("2")), Some(2));
        assert_eq!(cast_int(&json!("")), None);
        assert_eq!(cast_int(&json!("abc")), Some(0));
    }
}
