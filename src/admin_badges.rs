//! Admin::BadgesController's model work: a badge created or updated from
//! the admin's params (`update_badge_from_params`, with Badge's
//! validations and callbacks and the staff action logs), destroyed with
//! its grants, the badge groupings saved, a badge query's preview, and the
//! mass award from a CSV of usernames and emails with its two jobs
//! (MassAwardBadge, BulkUserTitleUpdate).
//!
//! Refused: custom descriptions crate::sanitize_field refuses, and CSV
//! files that aren't UTF-8.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::badge_granter::{self, Badge};
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;
/// `BadgeGrouping::Other`, the column's default.
const OTHER_GROUPING: i32 = 5;

/// What a column holds, for casting the admin's params.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Int,
    Bool,
}

/// `Badge.column_names` less id, created_at, updated_at, grant_count and
/// system: the assignable columns, in the table's order.
const ASSIGNABLE: [(&str, Kind); 17] = [
    ("name", Kind::Text),
    ("description", Kind::Text),
    ("badge_type_id", Kind::Int),
    ("allow_title", Kind::Bool),
    ("multiple_grant", Kind::Bool),
    ("icon", Kind::Text),
    ("listable", Kind::Bool),
    ("target_posts", Kind::Bool),
    ("query", Kind::Text),
    ("enabled", Kind::Bool),
    ("auto_revoke", Kind::Bool),
    ("badge_grouping_id", Kind::Int),
    ("trigger", Kind::Int),
    ("show_posts", Kind::Bool),
    ("long_description", Kind::Text),
    ("image_upload_id", Kind::Int),
    ("show_in_post_header", Kind::Bool),
];

/// `Badge.protected_system_fields`
pub const PROTECTED_SYSTEM_FIELDS: [&str; 9] = [
    "name",
    "badge_type_id",
    "multiple_grant",
    "target_posts",
    "show_posts",
    "query",
    "trigger",
    "auto_revoke",
    "listable",
];

/// `StaffActionLogger::BADGE_FIELDS`
const BADGE_FIELDS: [&str; 17] = [
    "id",
    "name",
    "description",
    "long_description",
    "icon",
    "image_upload_id",
    "badge_type_id",
    "badge_grouping_id",
    "query",
    "allow_title",
    "multiple_grant",
    "listable",
    "target_posts",
    "enabled",
    "auto_revoke",
    "show_posts",
    "system",
];

/// A column's value as ActiveRecord holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
}

impl Val {
    fn from_json(v: &Value) -> Val {
        match v {
            Value::Bool(b) => Val::Bool(*b),
            Value::Number(n) => Val::Int(n.as_i64().unwrap_or(0)),
            Value::String(s) => Val::Text(s.clone()),
            _ => Val::Null,
        }
    }

    /// `"#{value}"`
    fn to_s(&self) -> String {
        match self {
            Val::Null => String::new(),
            Val::Bool(b) => b.to_string(),
            Val::Int(i) => i.to_string(),
            Val::Text(s) => s.clone(),
        }
    }

    /// `present?`: false, nil and blank strings are not.
    fn present(&self) -> bool {
        match self {
            Val::Null | Val::Bool(false) => false,
            Val::Text(s) => !ruby_blank(s),
            _ => true,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Val::Text(s) => Some(s),
            _ => None,
        }
    }

    fn int(&self) -> Option<i64> {
        match self {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }

    fn truthy(&self) -> bool {
        !matches!(self, Val::Null | Val::Bool(false))
    }
}

/// `String#blank?`: empty or only whitespace.
fn ruby_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// ActiveModel's integer cast of a string: nil unless it starts like a
/// number (`/\A\s*[+-]?\d/`), else its leading integer.
fn cast_int(s: &str) -> Val {
    let t = s.trim_start();
    let t2 = t.strip_prefix(['+', '-']).unwrap_or(t);
    if t2.starts_with(|c: char| c.is_ascii_digit()) {
        Val::Int(crate::ruby::to_i(s))
    } else {
        Val::Null
    }
}

/// ActiveModel's boolean cast: "" is nil, the FALSE_VALUES false, the
/// rest true.
fn cast_bool(s: &str) -> Val {
    if s.is_empty() {
        Val::Null
    } else {
        Val::Bool(!["0", "f", "F", "false", "FALSE", "off", "OFF"].contains(&s))
    }
}

/// A param cast to the column's type; Err for what the port doesn't cast
/// (arrays and hashes).
fn cast(kind: Kind, v: &Value) -> Result<Val, AppError> {
    Ok(match (kind, v) {
        (_, Value::Null) => Val::Null,
        (Kind::Text, Value::String(s)) => Val::Text(s.clone()),
        (Kind::Text, Value::Number(n)) => Val::Text(n.to_string()),
        (Kind::Text, Value::Bool(b)) => Val::Text(if *b { "t" } else { "f" }.into()),
        (Kind::Int, Value::String(s)) => cast_int(s),
        (Kind::Int, Value::Number(n)) => Val::Int(n.as_f64().map(|f| f as i64).unwrap_or(0)),
        (Kind::Int, Value::Bool(b)) => Val::Int(i64::from(*b)),
        (Kind::Bool, Value::String(s)) => cast_bool(s),
        (Kind::Bool, Value::Number(n)) => Val::Bool(n.as_f64() != Some(0.0)),
        (Kind::Bool, Value::Bool(b)) => Val::Bool(*b),
        _ => return Err(Unsupported("badge params that are arrays or hashes").into()),
    })
}

/// The badge's row as the controller assigns and saves it.
pub struct Record {
    /// The row's id; None for a new badge.
    pub id: Option<i32>,
    pub system: bool,
    /// The assignable columns, in ASSIGNABLE's order.
    values: Vec<Val>,
    /// As loaded, to tell what changed.
    original: Vec<Val>,
}

/// The services' shared inputs.
pub struct Ctx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub bus: &'a pg_bus::Bus,
    pub base_path: &'a str,
}

fn index_of(column: &str) -> usize {
    ASSIGNABLE
        .iter()
        .position(|(c, _)| *c == column)
        .expect("an assignable column")
}

impl Record {
    /// `Badge.new`: the table's defaults.
    pub fn new() -> Record {
        let values = ASSIGNABLE
            .iter()
            .map(|(c, _)| match *c {
                "allow_title"
                | "multiple_grant"
                | "target_posts"
                | "show_posts"
                | "show_in_post_header" => Val::Bool(false),
                "listable" | "enabled" | "auto_revoke" => Val::Bool(true),
                "icon" => Val::Text("certificate".into()),
                "badge_grouping_id" => Val::Int(OTHER_GROUPING.into()),
                _ => Val::Null,
            })
            .collect::<Vec<_>>();
        Record {
            id: None,
            system: false,
            original: values.clone(),
            values,
        }
    }

    /// `Badge.find(id)`
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Record>, AppError> {
        let row: Option<Value> =
            sqlx::query_scalar("SELECT to_jsonb(b) FROM badges b WHERE id = $1")
                .bind(id)
                .fetch_optional(conn)
                .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let values: Vec<Val> = ASSIGNABLE
            .iter()
            .map(|(c, _)| Val::from_json(&row[*c]))
            .collect();
        Ok(Some(Record {
            id: Some(id),
            system: row["system"].as_bool().unwrap_or(false),
            original: values.clone(),
            values,
        }))
    }

    pub fn get(&self, column: &str) -> &Val {
        &self.values[index_of(column)]
    }

    fn set(&mut self, column: &str, v: Val) {
        let i = index_of(column);
        self.values[i] = v;
    }

    fn changed(&self, column: &str) -> bool {
        let i = index_of(column);
        self.values[i] != self.original[i]
    }

    /// The changed columns, in the table's order.
    fn changes(&self) -> Vec<&'static str> {
        ASSIGNABLE
            .iter()
            .enumerate()
            .filter(|(i, _)| self.values[*i] != self.original[*i])
            .map(|(_, (c, _))| *c)
            .collect()
    }

    /// `i18n_name`: the current name, downcased, spaces to underscores.
    fn i18n_name(&self) -> String {
        self.get("name")
            .text()
            .unwrap_or_default()
            .to_lowercase()
            .replace(' ', "_")
    }

    /// `Badge#description` / `#long_description`: the locale's text, else
    /// the column, else "".
    fn text_getter(&self, ctx: &Ctx<'_>, column: &str) -> Result<String, AppError> {
        let column_value = self.get(column).text().map(str::to_string);
        crate::badges_view::badge_text(
            ctx.i18n,
            ctx.settings,
            ctx.base_path,
            &format!("badges.{}.{column}", self.i18n_name()),
            column_value.as_ref(),
        )
    }

    /// `Badge#display_name`
    fn display_name(&self, i18n: &I18n) -> String {
        let name = self.get("name").text().unwrap_or_default().to_string();
        i18n.t(&format!("badges.{}.name", self.i18n_name()))
            .map(str::to_string)
            .unwrap_or(name)
    }

    /// A field as `badge.public_send(f)` reads it for the staff logs.
    fn log_value(&self, ctx: &Ctx<'_>, field: &str) -> Result<Val, AppError> {
        Ok(match field {
            "id" => self.id.map_or(Val::Null, |i| Val::Int(i.into())),
            "system" => Val::Bool(self.system),
            "description" | "long_description" => Val::Text(self.text_getter(ctx, field)?),
            f => self.get(f).clone(),
        })
    }

    /// `"#{f}: #{v}"` of the BADGE_FIELDS that are present, as
    /// log_badge_creation and log_badge_deletion write them.
    fn present_fields(&self, ctx: &Ctx<'_>) -> Result<String, AppError> {
        let mut lines = Vec::new();
        for f in BADGE_FIELDS {
            let v = self.log_value(ctx, f)?;
            if v.present() {
                lines.push(format!("{f}: {}", v.to_s()));
            }
        }
        Ok(lines.join("\n"))
    }
}

impl Default for Record {
    fn default() -> Self {
        Self::new()
    }
}

/// What `update_badge_from_params` came to.
pub enum Saved {
    /// The badge's id, and whether its name changed.
    Done { id: i32, name_changed: bool },
    /// `render_json_error(errors)`: 422.
    Invalid(Vec<String>),
}

/// The full message of a validation: the attribute's human name and the
/// message.
fn full_message(attr: &str, message: &str) -> String {
    format!("{attr} {message}")
}

fn t(i18n: &I18n, key: &str, fallback: &str) -> String {
    i18n.t(key).unwrap_or(fallback).to_string()
}

fn too_long(i18n: &I18n, count: i64) -> String {
    i18n.t_count("errors.messages.too_long", count, &[])
        .unwrap_or_else(|| format!("is too long (maximum is {count} characters)"))
}

/// `BadgeGranter.contract_checks!`: the first contract the query breaks.
pub fn contract_checks(
    sql: Option<&str>,
    target_posts: bool,
    trigger: Option<i64>,
) -> Option<&'static str> {
    let sql = sql?;
    if ruby_blank(sql) {
        return None;
    }
    // Badge::Trigger: PostAction 1, PostRevision 2, TrustLevelChange 4,
    // UserChange 8.
    let uses_post_ids = matches!(trigger, Some(1 | 2));
    let uses_user_ids = matches!(trigger, Some(4 | 8));
    if uses_post_ids {
        if !sql.contains(":post_ids") {
            return Some(
                "Contract violation:\nQuery triggers on posts, but does not reference the ':post_ids' array",
            );
        }
        if sql.contains(":user_ids") {
            return Some(
                "Contract violation:\nQuery triggers on posts, but references the ':user_ids' array",
            );
        }
    }
    if uses_user_ids {
        if !sql.contains(":user_ids") {
            return Some(
                "Contract violation:\nQuery triggers on users, but does not reference the ':user_ids' array",
            );
        }
        if sql.contains(":post_ids") {
            return Some(
                "Contract violation:\nQuery triggers on users, but references the ':post_ids' array",
            );
        }
    }
    if trigger.is_some_and(|t| t != 0) && !sql.contains(":backfill") {
        return Some(
            "Contract violation:\nQuery is triggered, but does not reference the ':backfill' parameter.\n(Hint: if :backfill is TRUE, you should ignore the :post_ids/:user_ids)",
        );
    }
    if target_posts && !sql.contains("post_id") {
        return Some(
            "Contract violation:\nQuery targets posts, but does not return a 'post_id' column",
        );
    }
    if !sql.contains("user_id") {
        return Some("Contract violation:\nQuery does not return a 'user_id' column");
    }
    if !sql.contains("granted_at") {
        return Some("Contract violation:\nQuery does not return a 'granted_at' column");
    }
    // /;\s*\z/
    if sql
        .trim_end_matches(|c: char| c.is_ascii_whitespace())
        .ends_with(';')
    {
        return Some(
            "Contract violation:\nQuery ends with a semicolon. Remove the semicolon; your sql will be used in a subquery.",
        );
    }
    None
}

/// `update_badge_from_params(badge, new:)`: each allowed column the params
/// give (`if params[key]`) assigned, the query's contract checked, and the
/// badge saved with Badge's validations and callbacks.
pub async fn update_from_params(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    record: &mut Record,
    params: &Map<String, Value>,
) -> Result<Saved, AppError> {
    let sql_on = ctx.settings.get("enable_badge_sql")?.truthy();
    for (column, kind) in ASSIGNABLE {
        if record.system && PROTECTED_SYSTEM_FIELDS.contains(&column) {
            continue;
        }
        if column == "query" && !sql_on {
            continue;
        }
        let Some(param) = params
            .get(column)
            .filter(|v| !matches!(v, Value::Null | Value::Bool(false)))
        else {
            continue;
        };
        let value = cast(kind, param)?;
        if column == "description" || column == "long_description" {
            // `self[:col] = val if val != col` (the getter).
            let getter = record.text_getter(ctx, column)?;
            if value.text() == Some(getter.as_str()) {
                continue;
            }
        }
        record.set(column, value);
    }

    if sql_on {
        let violation = contract_checks(
            record.get("query").text(),
            record.get("target_posts").truthy(),
            record.get("trigger").int(),
        );
        if let Some(message) = violation {
            return Ok(Saved::Invalid(vec![message.to_string()]));
        }
    }

    // Validations, in Badge's order.
    let i18n = ctx.i18n;
    let mut errors = Vec::new();
    let name = record.get("name").text().map(str::to_string);
    if name.as_deref().is_none_or(ruby_blank) {
        errors.push(full_message(
            "Name",
            &t(i18n, "errors.messages.blank", "can't be blank"),
        ));
    }
    if let Some(name) = &name {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM badges WHERE name = $1 AND ($2::int IS NULL OR id <> $2))",
        )
        .bind(name)
        .bind(record.id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            errors.push(full_message(
                "Name",
                &t(i18n, "errors.messages.taken", "has already been taken"),
            ));
        }
        if name.chars().count() > 100 {
            errors.push(full_message("Name", &too_long(i18n, 100)));
        }
    }
    let type_exists = match record.get("badge_type_id").int() {
        Some(id) => {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM badge_types WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        None => false,
    };
    if !type_exists {
        errors.push(full_message(
            "Badge type",
            &t(i18n, "errors.messages.blank", "can't be blank"),
        ));
    }
    for (column, human) in [
        ("allow_title", "Allow title"),
        ("multiple_grant", "Multiple grant"),
    ] {
        if !matches!(record.get(column), Val::Bool(_)) {
            errors.push(full_message(
                human,
                &t(
                    i18n,
                    "errors.messages.inclusion",
                    "is not included in the list",
                ),
            ));
        }
    }
    for (column, human, max) in [
        ("description", "Description", 500),
        ("long_description", "Long description", 1000),
    ] {
        if record.text_getter(ctx, column)?.chars().count() > max {
            errors.push(full_message(human, &too_long(i18n, max as i64)));
        }
    }
    if !errors.is_empty() {
        return Ok(Saved::Invalid(errors));
    }

    // before_save sanitize_description
    if record.changed("description") {
        let current = record.text_getter(ctx, "description")?;
        let sanitized = crate::sanitize_field::sanitize_field(&current, &[])?;
        if sanitized != current {
            record.set("description", Val::Text(sanitized));
        }
    }

    let changes = record.changes();
    let id = match record.id {
        None => {
            // before_create ensure_not_system
            let id: i32 =
                sqlx::query_scalar("SELECT GREATEST(COALESCE(MAX(id), 0) + 1, 100) FROM badges")
                    .fetch_one(&mut *conn)
                    .await?;
            insert(conn, record, id).await?;
            record.id = Some(id);
            id
        }
        Some(id) => {
            if !changes.is_empty() {
                update(conn, record, id, &changes).await?;
            }
            id
        }
    };
    // after_save
    if changes.contains(&"image_upload_id") {
        let upload = record.get("image_upload_id").int();
        crate::upload_references::ensure_exist(&mut *conn, "Badge", id.into(), upload).await?;
    }
    // after_commit
    if changes.contains(&"enabled") {
        badge_granter::update_distinct_badge_count(&mut *conn, None).await?;
        badge_granter::update_featured_ranks(&mut *conn, None).await?;
    }
    Ok(Saved::Done {
        id,
        name_changed: changes.contains(&"name"),
    })
}

/// A value bound by its kind.
fn bind_val<'q>(
    q: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    kind: Kind,
    v: &'q Val,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match (kind, v) {
        (Kind::Text, Val::Text(s)) => q.bind(Some(s.as_str())),
        (Kind::Text, _) => q.bind(None::<&str>),
        (Kind::Int, Val::Int(i)) => q.bind(Some(*i as i32)),
        (Kind::Int, _) => q.bind(None::<i32>),
        (Kind::Bool, Val::Bool(b)) => q.bind(Some(*b)),
        (Kind::Bool, _) => q.bind(None::<bool>),
    }
}

async fn insert(conn: &mut PgConnection, record: &Record, id: i32) -> Result<(), AppError> {
    let columns: Vec<&str> = ASSIGNABLE.iter().map(|(c, _)| *c).collect();
    let placeholders: Vec<String> = (0..columns.len()).map(|i| format!("${}", i + 2)).collect();
    let sql = format!(
        "INSERT INTO badges (id, {}, system, grant_count, created_at, updated_at) \
         VALUES ($1, {}, FALSE, 0, now(), now())",
        columns
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", "),
        placeholders.join(", ")
    );
    let mut q = sqlx::query(&sql).bind(id);
    for (i, (_, kind)) in ASSIGNABLE.iter().enumerate() {
        q = bind_val(q, *kind, &record.values[i]);
    }
    q.execute(conn).await?;
    Ok(())
}

async fn update(
    conn: &mut PgConnection,
    record: &Record,
    id: i32,
    changes: &[&str],
) -> Result<(), AppError> {
    let sets: Vec<String> = changes
        .iter()
        .enumerate()
        .map(|(i, c)| format!("\"{c}\" = ${}", i + 2))
        .collect();
    let sql = format!(
        "UPDATE badges SET {}, updated_at = now() WHERE id = $1",
        sets.join(", ")
    );
    let mut q = sqlx::query(&sql).bind(id);
    for c in changes {
        let i = index_of(c);
        q = bind_val(q, ASSIGNABLE[i].1, &record.values[i]);
    }
    q.execute(conn).await?;
    Ok(())
}

/// An admin-only staff action log (`UserHistory.create!`).
async fn log(
    conn: &mut PgConnection,
    action: &str,
    acting_user_id: i32,
    details: &str,
) -> Result<(), AppError> {
    let action_id = crate::staff_action_logs::action_id(action)
        .ok_or(Unsupported("a staff action missing from enums.json"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, details, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, TRUE, now(), now())",
    )
    .bind(action_id)
    .bind(acting_user_id)
    .bind(details)
    .execute(conn)
    .await?;
    Ok(())
}

/// `StaffActionLogger#log_badge_creation`
pub async fn log_creation(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    record: &Record,
    acting_user_id: i32,
) -> Result<(), AppError> {
    let details = record.present_fields(ctx)?;
    log(conn, "create_badge", acting_user_id, &details).await
}

/// `StaffActionLogger#log_badge_change`: the id, then each changed field.
pub async fn log_change(
    conn: &mut PgConnection,
    record: &Record,
    acting_user_id: i32,
) -> Result<(), AppError> {
    let mut lines = vec![format!("id: {}", record.id.unwrap_or_default())];
    for c in record.changes() {
        if BADGE_FIELDS.contains(&c) {
            lines.push(format!("{c}: {}", record.get(c).to_s()));
        }
    }
    log(conn, "change_badge", acting_user_id, &lines.join("\n")).await
}

/// The `bulk_user_title_update` job.
pub const BULK_USER_TITLE_UPDATE: &str = "bulk_user_title_update";
/// The `mass_award_badge` job.
pub const MASS_AWARD_BADGE: &str = "mass_award_badge";

/// The `name_previously_changed?` follow-up of an update: the holders'
/// titles renamed in the background.
pub async fn enqueue_title_update(
    conn: &mut PgConnection,
    i18n: &I18n,
    record: &Record,
) -> Result<(), AppError> {
    crate::jobs::enqueue(
        conn,
        BULK_USER_TITLE_UPDATE,
        json!({
            "new_title": record.display_name(i18n),
            "granted_badge_id": record.id,
            "action": "update",
        }),
    )
    .await?;
    Ok(())
}

/// Admin::BadgesController#destroy for a badge that isn't a system one:
/// logged, the titles it granted cleared, each grant destroyed (with its
/// callbacks), its upload references and the badge deleted.
pub async fn destroy(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    record: &Record,
    acting_user_id: i32,
) -> Result<(), AppError> {
    let id = record
        .id
        .ok_or(Unsupported("destroying an unsaved badge"))?;
    let details = record.present_fields(ctx)?;
    log(conn, "delete_badge", acting_user_id, &details).await?;
    // clear_user_titles!
    sqlx::query(
        "UPDATE users AS u SET title = '', updated_at = now() FROM user_profiles AS up \
         WHERE up.user_id = u.id AND up.granted_title_badge_id = $1",
    )
    .bind(id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE user_profiles AS up SET granted_title_badge_id = NULL WHERE up.granted_title_badge_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    // has_many :user_badges, dependent: :destroy
    let grants: Vec<i32> = sqlx::query_scalar("SELECT id FROM user_badges WHERE badge_id = $1")
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
    for grant in grants {
        badge_granter::destroy_user_badge(&mut *conn, ctx.bus, ctx.settings, grant).await?;
    }
    // has_many :upload_references, dependent: :destroy
    sqlx::query("DELETE FROM upload_references WHERE target_type = 'Badge' AND target_id = $1")
        .bind(i64::from(id))
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM badges WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `save_badge_groupings`: each name saved at its index's position, as
/// the grouping of the same index's id or a new one (a too long name is
/// not saved), then the groupings that aren't system ones and weren't
/// given deleted.
pub async fn save_groupings(
    conn: &mut PgConnection,
    ids: &[i64],
    names: &[String],
) -> Result<(), AppError> {
    let existing: Vec<(i32, String, i32, Option<String>)> = sqlx::query_as(
        "SELECT id, name, position, description FROM badge_groupings ORDER BY position",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (index, name) in names.iter().enumerate() {
        let id = ids.get(index).copied().unwrap_or(0);
        let position = index as i32;
        let found = existing.iter().find(|g| i64::from(g.0) == id);
        // validates :name, length <= 100; :description, length <= 500
        let description_ok = found
            .and_then(|g| g.3.as_ref())
            .is_none_or(|d| d.chars().count() <= 500);
        if name.chars().count() > 100 || !description_ok {
            continue;
        }
        match found {
            Some(g) => {
                if g.1 != *name || g.2 != position {
                    sqlx::query("UPDATE badge_groupings SET name = $2, position = $3, updated_at = now() WHERE id = $1")
                        .bind(g.0)
                        .bind(name)
                        .bind(position)
                        .execute(&mut *conn)
                        .await?;
                }
            }
            None => {
                sqlx::query(
                    "INSERT INTO badge_groupings (name, position, created_at, updated_at) VALUES ($1, $2, now(), now())",
                )
                .bind(name)
                .bind(position)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    for (id, ..) in &existing {
        // system?: ids up to 5
        if *id > 5 && !ids.contains(&i64::from(*id)) {
            sqlx::query("DELETE FROM badge_groupings WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

/// What a CSV of the mass award holds: its entries, emails (with an @)
/// and usernames apart, each stripped and blank ones skipped.
pub struct CsvEntries {
    pub emails: Vec<String>,
    pub usernames: Vec<String>,
}

/// `Admin::BadgesController::MAX_CSV_LINES`
pub const MAX_CSV_LINES: usize = 50_000;

/// `String#strip`: whitespace and NUL off both ends.
fn ruby_strip(s: &str) -> &str {
    s.trim_matches(|c: char| matches!(c, '\0' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' '))
}

/// What reading the mass award's CSV came to.
pub enum Csv {
    Entries(CsvEntries),
    /// More than MAX_CSV_LINES entries.
    TooMany,
    /// `CSV::MalformedCSVError#line_number`: the row it broke on.
    Malformed(usize),
}

/// One row's first field as Ruby's CSV parses it (strict, quote `"`,
/// comma separated): None for an empty unquoted field. `Err` is the
/// malformation.
struct Rows<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    sep: &'static str,
}

enum Row {
    First(Option<String>),
    Malformed,
}

impl Rows<'_> {
    /// At a row separator: consumes it.
    fn at_sep(&mut self, c: char) -> bool {
        match self.sep {
            "\r\n" => {
                if c == '\r' && self.chars.peek() == Some(&'\n') {
                    self.chars.next();
                    true
                } else {
                    false
                }
            }
            "\r" => c == '\r',
            _ => c == '\n',
        }
    }

    /// The next row, None at the end.
    fn next_row(&mut self) -> Option<Row> {
        self.chars.peek()?;
        let mut first: Option<Option<String>> = None;
        loop {
            // A field.
            let mut value: Option<String> = None;
            let ended_row;
            if self.chars.peek() == Some(&'"') {
                self.chars.next();
                let mut text = String::new();
                loop {
                    match self.chars.next() {
                        None => return Some(Row::Malformed),
                        Some('"') if self.chars.peek() == Some(&'"') => {
                            self.chars.next();
                            text.push('"');
                        }
                        Some('"') => break,
                        Some(c) => text.push(c),
                    }
                }
                value = Some(text);
                match self.chars.next() {
                    None => ended_row = true,
                    Some(',') => ended_row = false,
                    Some(c) if self.at_sep(c) => ended_row = true,
                    Some(_) => return Some(Row::Malformed),
                }
            } else {
                let mut text = String::new();
                loop {
                    match self.chars.next() {
                        None => {
                            ended_row = true;
                            break;
                        }
                        Some(',') => {
                            ended_row = false;
                            break;
                        }
                        Some(c) if self.at_sep(c) => {
                            ended_row = true;
                            break;
                        }
                        Some('"' | '\r' | '\n') => return Some(Row::Malformed),
                        Some(c) => text.push(c),
                    }
                }
                if !text.is_empty() {
                    value = Some(text);
                }
            }
            if first.is_none() {
                first = Some(value);
            }
            if ended_row {
                return Some(Row::First(first.flatten()));
            }
        }
    }
}

/// `CSV.foreach(file) { |row| row.first&.strip }` as the mass award
/// reads it: each row's stripped first field, emails (with an @) and
/// usernames apart, blank ones skipped, stopping at more than
/// MAX_CSV_LINES entries or the first malformed row. The row separator
/// is the first line ending in the file; a leading UTF-8 BOM is skipped.
pub fn read_csv(bytes: &[u8]) -> Result<Csv, AppError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Unsupported("mass award CSV files that aren't UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let sep = match text.find(['\r', '\n']) {
        Some(i) if text[i..].starts_with("\r\n") => "\r\n",
        Some(i) if text[i..].starts_with('\r') => "\r",
        _ => "\n",
    };
    let mut rows = Rows {
        chars: text.chars().peekable(),
        sep,
    };
    let mut out = CsvEntries {
        emails: Vec::new(),
        usernames: Vec::new(),
    };
    let mut line = 0;
    while let Some(row) = rows.next_row() {
        line += 1;
        let first = match row {
            Row::Malformed => return Ok(Csv::Malformed(line)),
            Row::First(first) => first,
        };
        let Some(first) = first else {
            continue;
        };
        let entry = ruby_strip(&first);
        if ruby_blank(entry) {
            continue;
        }
        if entry.contains('@') {
            out.emails.push(entry.to_string());
        } else {
            out.usernames.push(entry.to_string());
        }
        if out.emails.len() + out.usernames.len() > MAX_CSV_LINES {
            return Ok(Csv::TooMany);
        }
    }
    Ok(Csv::Entries(out))
}

/// `BadgeGranter.revoke_all(badge)`: the holders whose title is the
/// badge's name (or a custom name of it) lose it, and every grant is
/// deleted without callbacks.
pub async fn revoke_all(conn: &mut PgConnection, badge: &Badge) -> Result<(), AppError> {
    let custom: Vec<String> =
        sqlx::query_scalar("SELECT value FROM translation_overrides WHERE translation_key = $1")
            .bind(badge.translation_key())
            .fetch_all(&mut *conn)
            .await?;
    sqlx::query(
        "UPDATE users SET title = NULL WHERE users.id IN ( \
           SELECT users.id FROM users INNER JOIN user_badges ON user_badges.user_id = users.id \
           WHERE user_badges.badge_id = $1 AND (users.title = $2 OR users.title = ANY($3)))",
    )
    .bind(badge.id)
    .bind(&badge.name)
    .bind(&custom)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM user_badges WHERE badge_id = $1")
        .bind(badge.id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// What `enqueue_mass_grant_for_users` reports.
pub struct MassGrant {
    pub unmatched: Vec<String>,
    pub matched_users_count: usize,
}

/// `BadgeGranter.enqueue_mass_grant_for_users`: the entries matched to
/// users (usernames, then emails, downcased), a MassAwardBadge job per
/// user (once each, or as many times as listed), skipping those already
/// holding it when once.
pub async fn enqueue_mass_grant(
    conn: &mut PgConnection,
    badge_id: i32,
    entries: &CsvEntries,
    once: bool,
) -> Result<MassGrant, AppError> {
    let usernames: Vec<String> = entries.usernames.iter().map(|u| u.to_lowercase()).collect();
    let emails: Vec<String> = entries.emails.iter().map(|e| e.to_lowercase()).collect();
    let by_username: Vec<(String, i32)> = if usernames.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as("SELECT username_lower, id FROM users WHERE username_lower = ANY($1)")
            .bind(&usernames)
            .fetch_all(&mut *conn)
            .await?
    };
    let by_email: Vec<(String, i32)> = if emails.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT LOWER(user_emails.email), users.id FROM users \
             INNER JOIN user_emails ON user_emails.user_id = users.id \
             WHERE lower(user_emails.email) = ANY($1)",
        )
        .bind(&emails)
        .fetch_all(&mut *conn)
        .await?
    };
    // pluck(...).to_h: the last row of a key wins.
    let lookup = |map: &[(String, i32)], key: &str| {
        map.iter().rev().find(|(k, _)| k == key).map(|(_, id)| *id)
    };
    let mut counts: Vec<(i32, i64)> = Vec::new();
    let mut unmatched: Vec<String> = Vec::new();
    for entry in usernames.iter().chain(&emails) {
        let id = lookup(&by_username, entry).or_else(|| lookup(&by_email, entry));
        let Some(id) = id else {
            if !unmatched.contains(entry) {
                unmatched.push(entry.clone());
            }
            continue;
        };
        match counts.iter_mut().find(|(u, _)| *u == id) {
            Some((_, n)) => {
                if !once {
                    *n += 1;
                }
            }
            None => counts.push((id, 1)),
        }
    }
    let holders: Vec<i32> = if once {
        sqlx::query_scalar("SELECT DISTINCT user_id FROM user_badges WHERE badge_id = $1")
            .bind(badge_id)
            .fetch_all(&mut *conn)
            .await?
    } else {
        Vec::new()
    };
    for (user_id, count) in &counts {
        if once && holders.contains(user_id) {
            continue;
        }
        crate::jobs::enqueue(
            &mut *conn,
            MASS_AWARD_BADGE,
            json!({"user": user_id, "badge": badge_id, "count": count}),
        )
        .await?;
    }
    Ok(MassGrant {
        unmatched,
        matched_users_count: counts.len(),
    })
}

/// `Jobs::MassAwardBadge#execute`: `BadgeGranter.mass_grant` of an
/// enabled badge to an existing user, `count` raw grants, one
/// notification stamped on them, and the user's featured ranks.
pub async fn mass_award_badge(state: &crate::AppState, args: &Value) -> Result<(), AppError> {
    let int = |k: &str| match &args[k] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let mut tx = state.pool.begin().await?;
    let user: Option<(i32, String, Option<String>)> =
        match int("user").and_then(|i| i32::try_from(i).ok()) {
            Some(id) => {
                sqlx::query_as("SELECT id, username, locale FROM users WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => None,
        };
    let Some((user_id, username, locale)) = user else {
        return Ok(());
    };
    let badge = match int("badge").and_then(|i| i32::try_from(i).ok()) {
        Some(id) => Badge::find(&mut tx, id).await?.filter(|b| b.enabled),
        None => None,
    };
    let Some(badge) = badge else {
        return Ok(());
    };
    let count = int("count").unwrap_or(0);
    if count < 1 {
        return Err(Unsupported("a mass award with a count below 1 (ArgumentError)").into());
    }
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    for _ in 0..count {
        sqlx::query(
            "INSERT INTO user_badges (granted_at, created_at, granted_by_id, user_id, badge_id, seq) \
             VALUES (now(), now(), $1, $2, $3, \
                     COALESCE((SELECT MAX(seq) + 1 FROM user_badges WHERE badge_id = $3 AND user_id = $2), 0))",
        )
        .bind(SYSTEM_USER_ID)
        .bind(user_id)
        .bind(badge.id)
        .execute(&mut *tx)
        .await?;
    }
    let notification_id = badge_granter::send_notification(
        &mut tx,
        &state.bus,
        &settings,
        &state.i18n,
        &badge,
        user_id,
        &username,
        locale.as_deref(),
    )
    .await?;
    sqlx::query(
        "UPDATE user_badges SET notification_id = $1 \
         WHERE notification_id IS NULL AND user_id = $2 AND badge_id = $3",
    )
    .bind(notification_id)
    .bind(user_id)
    .bind(badge.id)
    .execute(&mut *tx)
    .await?;
    badge_granter::update_featured_ranks(&mut tx, Some(&[user_id])).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::BulkUserTitleUpdate#execute`: the titles a badge granted renamed
/// ("update") or put back to the badge's name ("reset").
pub async fn bulk_user_title_update(state: &crate::AppState, args: &Value) -> Result<(), AppError> {
    let id = match &args["granted_badge_id"] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let Some(id) = id.and_then(|i| i32::try_from(i).ok()) else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM badges WHERE id = $1)")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Ok(());
    }
    match args["action"].as_str() {
        Some("update") => {
            // A missing new_title is nil.
            let title = args["new_title"].as_str();
            sqlx::query(
                "UPDATE users AS u SET title = $2, updated_at = now() FROM user_profiles AS up \
                 WHERE up.user_id = u.id AND up.granted_title_badge_id = $1",
            )
            .bind(id)
            .bind(title)
            .execute(&mut *tx)
            .await?;
        }
        Some("reset") => {
            sqlx::query(
                "UPDATE users AS u SET title = badges.name, updated_at = now() FROM user_profiles AS up \
                 INNER JOIN badges ON badges.id = up.granted_title_badge_id \
                 WHERE up.user_id = u.id AND up.granted_title_badge_id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        _ => {}
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casts_like_active_model() {
        assert_eq!(cast_int(" 12abc"), Val::Int(12));
        assert_eq!(cast_int("abc"), Val::Null);
        assert_eq!(cast_int(""), Val::Null);
        assert_eq!(cast_int("-3"), Val::Int(-3));
        assert_eq!(cast_bool(""), Val::Null);
        assert_eq!(cast_bool("false"), Val::Bool(false));
        assert_eq!(cast_bool("no"), Val::Bool(true));
    }

    #[test]
    fn contract_checks_in_order() {
        assert_eq!(contract_checks(Some("  "), true, Some(1)), None);
        assert!(
            contract_checks(Some("user_id granted_at;\n "), false, None)
                .unwrap()
                .contains("semicolon")
        );
        assert!(
            contract_checks(Some("x"), false, Some(1))
                .unwrap()
                .contains(":post_ids")
        );
        assert_eq!(
            contract_checks(
                Some("user_id granted_at :backfill :user_ids"),
                false,
                Some(8)
            ),
            None
        );
    }

    #[test]
    fn reads_plain_csv() {
        let Csv::Entries(e) = read_csv(b"user1,x\r\n  a@b.c \r\n\r\nUser2\r\n").unwrap() else {
            panic!("entries");
        };
        assert_eq!(e.usernames, ["user1", "User2"]);
        assert_eq!(e.emails, ["a@b.c"]);
    }

    /// Ruby's CSV.foreach on the reference (Ruby 3.4): each input's
    /// non-blank first fields, stripped, and the malformed row's number.
    #[test]
    fn reads_csv_like_ruby() {
        let table: &[(&str, &[&str], Option<usize>)] = &[
            ("a\nb\n", &["a", "b"], None),
            ("\"a\"\nb\n", &["a", "b"], None),
            ("\"a,b\",c\nd\n", &["a,b", "d"], None),
            ("\"a\"\"b\"\nc\n", &["a\"b", "c"], None),
            ("\"a\nb\"\nc\n", &["a\nb", "c"], None),
            ("\"a\nb\"\n\"c\n", &["a\nb"], Some(2)),
            ("a\n\"b\n", &["a"], Some(2)),
            ("a\nb\"c\nd\n", &["a"], Some(2)),
            ("a\n\"b\"c\nd\n", &["a"], Some(2)),
            ("a\r\nb\r\n", &["a", "b"], None),
            ("a\r\nb\nc\r\n", &["a"], Some(2)),
            ("a\nb\rc\n", &["a"], Some(2)),
            ("a\rb\rc", &["a", "b", "c"], None),
            ("\"a\r\nb\"\r\nc\r\n", &["a\r\nb", "c"], None),
            ("  \"a\"  \nb\n", &[], Some(1)),
            ("\"a\" \nb\n", &[], Some(1)),
            ("x,\"y\nz\"\nw\n", &["x", "w"], None),
            ("\"\"\nb\n", &["b"], None),
            ("\"\n", &[], Some(1)),
            ("a,\"b\nc\nd\ne\"f\n", &[], Some(1)),
            ("\u{feff}user1\nuser2\n", &["user1", "user2"], None),
            ("a\n\n\nb\"\n", &["a"], Some(4)),
            ("a\"\n", &[], Some(1)),
            ("\"a\"\n\"b\" \n", &["a"], Some(2)),
            ("u1\n\"\"\"u2\"\"\"\n", &["u1", "\"u2\""], None),
            ("\"a\nb\nc\"\"d\n", &[], Some(1)),
            (",x\n y ,z\n", &["y"], None),
        ];
        for (input, rows, error) in table {
            match read_csv(input.as_bytes()).unwrap() {
                Csv::Entries(e) => {
                    assert_eq!(error, &None, "{input:?}");
                    assert_eq!(&e.usernames, rows, "{input:?}");
                }
                Csv::Malformed(line) => assert_eq!(*error, Some(line), "{input:?}"),
                Csv::TooMany => panic!("{input:?}"),
            }
        }
    }
}
