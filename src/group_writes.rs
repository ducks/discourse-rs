//! Saving a group as the Group model does: the attributes cast from the
//! request (ActiveModel's string, integer and boolean casts), the model's
//! validations in their order, the before_save callbacks (incoming email
//! downcased, the bio cooked), the row written, the after_save callbacks
//! (members' primary group and flair, members' titles, the rename job)
//! and the after_commit ones (automatic membership by domain, the
//! notification defaults' batch_set); GroupActionLogger's settings log;
//! and destroying one with its dependents.
//!
//! Refused: flair uploads (UploadReference), group SMTP settings, plugin
//! custom fields and associated groups.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The columns a request may set, with their ActiveModel types, in the
/// table's column order (the order `previous_changes` lists them in).
pub const COLUMNS: [(&str, Kind); 23] = [
    ("name", Kind::Str),
    ("automatic_membership_email_domains", Kind::Str),
    ("primary_group", Kind::Bool),
    ("title", Kind::Str),
    ("grant_trust_level", Kind::Int),
    ("incoming_email", Kind::Str),
    ("flair_bg_color", Kind::Str),
    ("flair_color", Kind::Str),
    ("bio_raw", Kind::Str),
    ("bio_cooked", Kind::Str),
    ("allow_membership_requests", Kind::Bool),
    ("full_name", Kind::Str),
    ("default_notification_level", Kind::Int),
    ("visibility_level", Kind::Int),
    ("public_exit", Kind::Bool),
    ("public_admission", Kind::Bool),
    ("membership_request_template", Kind::Str),
    ("messageable_level", Kind::Int),
    ("mentionable_level", Kind::Int),
    ("publish_read_state", Kind::Bool),
    ("members_visibility_level", Kind::Int),
    ("flair_icon", Kind::Str),
    ("flair_upload_id", Kind::Int),
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Str,
    Int,
    Bool,
}

/// ActiveModel's cast of a request value to a column's type.
pub fn cast(kind: Kind, v: &Value) -> Value {
    match (kind, v) {
        (_, Value::Null) => Value::Null,
        (Kind::Str, Value::String(s)) => json!(s),
        (Kind::Str, Value::Number(n)) => json!(n.to_string()),
        (Kind::Str, Value::Bool(b)) => json!(if *b { "t" } else { "f" }),
        (Kind::Int, Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .map_or(Value::Null, |i| json!(i)),
        (Kind::Int, Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                return Value::Null;
            }
            // String#to_i of a numeric string; "abc" casts to nil.
            match t.parse::<f64>() {
                Ok(f) => json!(f as i64),
                Err(_) => Value::Null,
            }
        }
        (Kind::Int, Value::Bool(b)) => json!(i64::from(*b)),
        (Kind::Bool, Value::Bool(b)) => json!(b),
        (Kind::Bool, Value::String(s)) => {
            if s.is_empty() {
                Value::Null
            } else {
                let l = s.to_lowercase();
                json!(!matches!(l.as_str(), "0" | "f" | "false" | "off"))
            }
        }
        (Kind::Bool, Value::Number(n)) => json!(n.as_f64() != Some(0.0)),
        _ => Value::Null,
    }
}

fn kind_of(column: &str) -> Option<Kind> {
    COLUMNS.iter().find(|(c, _)| *c == column).map(|(_, k)| *k)
}

/// A group's row as JSON (row_to_json), every column.
pub async fn load(
    conn: &mut PgConnection,
    id: i32,
) -> Result<Option<Map<String, Value>>, sqlx::Error> {
    let row: Option<Value> =
        sqlx::query_scalar("SELECT row_to_json(g)::jsonb FROM groups g WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await?;
    Ok(row.and_then(|v| match v {
        Value::Object(m) => Some(m),
        _ => None,
    }))
}

fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Bool(b) => !b,
        _ => false,
    }
}

fn s(v: &Value) -> Option<&str> {
    v.as_str()
}

/// `Group.get_valid_email_domains(value)`: the valid domains (each
/// stripped of scheme, path, user and port, downcased, unique) and the
/// invalid ones.
pub fn valid_email_domains(value: &str) -> (Vec<String>, Vec<String>) {
    static DOMAIN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)^[a-z0-9]+([\-\.]{1}[a-z0-9]+)*\.[a-z]{2,24}$").unwrap()
    });
    static SCHEME: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^https?://").unwrap());
    static PATH: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)/.*$").unwrap());
    static USER: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^.*@").unwrap());
    static PORT: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r":\d+$").unwrap());
    let mut valid: Vec<String> = Vec::new();
    let mut invalid = Vec::new();
    for domain in value.split('|') {
        let d = domain.trim().to_lowercase();
        let d = SCHEME.replace(&d, "").to_string();
        let d = PATH.replace(&d, "").to_string();
        let d = USER.replace(&d, "").to_string();
        let d = PORT.replace(&d, "").to_string();
        if d.trim().is_empty() {
            continue;
        }
        if DOMAIN.is_match(&d) {
            if !valid.contains(&d) {
                valid.push(d);
            }
        } else {
            invalid.push(d);
        }
    }
    (valid, invalid)
}

/// `Rack::Utils.escape_html`
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&#39;")
        .replace('"', "&quot;")
        .replace('/', "&#x2F;")
}

/// What validating a group needs.
pub struct Validation<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    /// The saved row, for an update.
    pub before: Option<&'a Map<String, Value>>,
    /// For a new group, whether it starts with an owner.
    pub has_owner: bool,
}

/// The model's validations, in their order: the errors' full messages.
/// Also normalizes the name and the domains as their validators do.
pub async fn validate(
    conn: &mut PgConnection,
    v: &Validation<'_>,
    row: &mut Map<String, Value>,
) -> Result<Vec<String>, AppError> {
    let mut errors = Vec::new();
    let t = |key: &str| v.i18n.t(key).unwrap_or(key).to_string();
    let changed = |row: &Map<String, Value>, column: &str| match v.before {
        None => true,
        Some(before) => before.get(column) != row.get(column),
    };
    let automatic = row["automatic"].as_bool().unwrap_or(false);
    // name_format_validator
    if changed(row, "name")
        && let Some(name) = s(&row["name"]).map(str::to_string)
    {
        use unicode_normalization::UnicodeNormalization;
        let stripped: String = name.nfc().collect::<String>().trim().to_string();
        if stripped != name {
            row.insert("name".into(), json!(stripped));
        }
        let name = stripped;
        let format_error = if automatic {
            None
        } else {
            crate::signup::username_format_error(v.settings, v.i18n, &name)?
        };
        match format_error {
            Some(e) => errors.push(format!("Name {e}")),
            None => {
                let lower = crate::accounts::normalize_username(&name)?;
                let was = v
                    .before
                    .and_then(|b| s(&b["name"]))
                    .map(crate::accounts::normalize_username)
                    .transpose()?;
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1) \
                         OR EXISTS (SELECT 1 FROM groups WHERE lower(name) = $1)",
                )
                .bind(&lower)
                .fetch_one(&mut *conn)
                .await?;
                if was.as_deref() != Some(lower.as_str()) && exists {
                    errors.push(format!("Name {}", t("errors.messages.taken")));
                }
            }
        }
    }
    // validates :name, presence: true
    if blank(&row["name"]) {
        errors.push(format!("Name {}", t("errors.messages.blank")));
    }
    // automatic_membership_email_domains_validator
    if let Some(domains) = s(&row["automatic_membership_email_domains"])
        .filter(|d| !d.trim().is_empty())
        .map(str::to_string)
    {
        let (valid, invalid) = valid_email_domains(&domains);
        for domain in &invalid {
            errors.push(
                v.i18n
                    .t_with("groups.errors.invalid_domain", &[("domain", domain)])
                    .unwrap_or_default(),
            );
        }
        let max = v
            .settings
            .get("max_automatic_membership_email_domains")?
            .to_i();
        if valid.len() as i64 > max {
            errors.push(
                v.i18n
                    .t_with(
                        "groups.errors.too_many_domains",
                        &[("max", &max.to_string())],
                    )
                    .unwrap_or_default(),
            );
        }
        for domain in &valid {
            if domain.len() > 253 {
                errors.push(
                    v.i18n
                        .t_with("groups.errors.invalid_domain", &[("domain", domain)])
                        .unwrap_or_default(),
                );
            }
        }
        row.insert(
            "automatic_membership_email_domains".into(),
            json!(valid.join("|")),
        );
    }
    // incoming_email_validator
    if !automatic
        && let Some(emails) = s(&row["incoming_email"])
            .filter(|e| !e.trim().is_empty())
            .map(str::to_string)
    {
        let id = row["id"].as_i64().unwrap_or(0);
        for email in emails.split('|') {
            let escaped = escape_html(email);
            if !crate::accounts::valid_email(email) {
                errors.push(
                    v.i18n
                        .t_with(
                            "groups.errors.invalid_incoming_email",
                            &[("email", &escaped)],
                        )
                        .unwrap_or_default(),
                );
                continue;
            }
            let lower = email.to_lowercase();
            let group: Option<String> = sqlx::query_scalar(
                "SELECT name FROM groups WHERE id <> $2 AND (email_username = $1 \
                   OR string_to_array(incoming_email, '|') @> ARRAY[$1]::text[] \
                   OR email_from_alias = $1) ORDER BY id LIMIT 1",
            )
            .bind(&lower)
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(group) = group {
                errors.push(
                    v.i18n
                        .t_with(
                            "groups.errors.email_already_used_in_group",
                            &[("email", &escaped), ("group_name", &escape_html(&group))],
                        )
                        .unwrap_or_default(),
                );
                continue;
            }
            let category: Option<String> = sqlx::query_scalar(
                "SELECT name FROM categories WHERE string_to_array(email_in, '|') @> ARRAY[$1]::text[] \
                 ORDER BY id LIMIT 1",
            )
            .bind(&lower)
            .fetch_optional(&mut *conn)
            .await?;
            if let Some(category) = category {
                errors.push(
                    v.i18n
                        .t_with(
                            "groups.errors.email_already_used_in_category",
                            &[
                                ("email", &escaped),
                                ("category_name", &escape_html(&category)),
                            ],
                        )
                        .unwrap_or_default(),
                );
            }
        }
    }
    // can_allow_membership_requests
    if row["allow_membership_requests"].as_bool() == Some(true) {
        let has_owner = match v.before {
            None => v.has_owner,
            Some(_) => {
                sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND owner)",
                )
                .bind(row["id"].as_i64().unwrap_or(0) as i32)
                .fetch_one(&mut *conn)
                .await?
            }
        };
        if !has_owner {
            errors.push(t("groups.errors.cant_allow_membership_requests"));
        }
    }
    // validate_grant_trust_level
    if changed(row, "grant_trust_level")
        && let Some(level) = row["grant_trust_level"].as_i64()
        && !(0..=4).contains(&level)
    {
        errors.push(
            v.i18n
                .t_with(
                    "groups.errors.grant_trust_level_not_valid",
                    &[("trust_level", &level.to_string())],
                )
                .unwrap_or_default(),
        );
    }
    for (column, label, max) in [
        ("bio_raw", "Bio raw", 3000),
        (
            "membership_request_template",
            "Membership request template",
            5000,
        ),
        ("full_name", "Full name", 100),
    ] {
        if s(&row[column]).is_some_and(|v| v.chars().count() > max) {
            let message = v
                .i18n
                .t_with(
                    "errors.messages.too_long.other",
                    &[("count", &max.to_string())],
                )
                .unwrap_or_default();
            errors.push(format!("{label} {message}"));
        }
    }
    // name_cannot_be_reserved
    if s(&row["name"]).is_some_and(|n| n.to_lowercase() == "by-id") {
        let name = s(&row["name"]).unwrap_or_default().to_string();
        errors.push(format!(
            "Name {}",
            v.i18n
                .t_with("errors.messages.reserved", &[("name", &name)])
                .unwrap_or_default()
        ));
    }
    Ok(errors)
}

/// The before_save callbacks: `downcase_incoming_email` and `cook_bio`.
pub async fn before_save(
    host: &crate::pretty_text::Host,
    row: &mut Map<String, Value>,
    before: Option<&Map<String, Value>>,
) -> Result<(), AppError> {
    let email = s(&row["incoming_email"])
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty());
    row.insert("incoming_email".into(), json!(email));
    match s(&row["bio_raw"]).filter(|b| !b.trim().is_empty()) {
        Some(raw) => {
            let unchanged = before.is_some_and(|b| b.get("bio_raw") == row.get("bio_raw"));
            if !unchanged || before.is_some_and(|b| b["bio_cooked"].is_null()) {
                if raw.contains("upload://") || raw.contains("/uploads/") {
                    return Err(Unsupported("uploads in a group's bio (UploadReference)").into());
                }
                let cooked = crate::pretty_text::cook(
                    host,
                    raw,
                    &crate::pretty_text::MarkdownOptions::default(),
                )
                .await?;
                row.insert("bio_cooked".into(), json!(cooked));
            }
        }
        None => {
            row.insert("bio_cooked".into(), Value::Null);
        }
    }
    Ok(())
}

/// The columns whose values differ, in column order.
pub fn changes(before: &Map<String, Value>, after: &Map<String, Value>) -> Vec<&'static str> {
    COLUMNS
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| before.get(*c) != after.get(*c))
        .collect()
}

/// The row inserted (`save!` of a new group): the request's columns over
/// the defaults, timestamps now. Its id.
pub async fn insert(conn: &mut PgConnection, row: &Map<String, Value>) -> Result<i32, sqlx::Error> {
    let mut given = Map::new();
    for (column, _) in COLUMNS {
        if let Some(v) = row.get(column)
            && !v.is_null()
        {
            given.insert(column.to_string(), v.clone());
        }
    }
    let columns: Vec<&String> = given.keys().collect();
    let list = columns
        .iter()
        .map(|c| c.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query_scalar(&format!(
        "INSERT INTO groups ({list}, created_at, updated_at) \
         SELECT {list}, now(), now() FROM jsonb_populate_record(NULL::groups, $1) RETURNING id"
    ))
    .bind(Value::Object(given.clone()))
    .fetch_one(conn)
    .await
}

/// An update's UPDATE of the changed columns (and updated_at).
pub async fn write_changes(
    conn: &mut PgConnection,
    id: i32,
    row: &Map<String, Value>,
    changed: &[&str],
) -> Result<(), sqlx::Error> {
    if changed.is_empty() {
        return Ok(());
    }
    let list = changed.join(", ");
    sqlx::query(&format!(
        "UPDATE groups SET ({list}, updated_at) = (SELECT {list}, now() FROM jsonb_populate_record(NULL::groups, $2)) \
         WHERE id = $1"
    ))
    .bind(id)
    .bind(Value::Object(row.clone()))
    .execute(conn)
    .await?;
    Ok(())
}

/// The after_save callbacks that write: `update_primary_group` and
/// `update_title` (for a new group only when set), the rename's
/// `update_group_mentions` job.
pub async fn after_save(
    conn: &mut PgConnection,
    id: i32,
    row: &Map<String, Value>,
    before: Option<&Map<String, Value>>,
) -> Result<(), AppError> {
    let was = |c: &str| before.map_or(Value::Null, |b| b[c].clone());
    let new_record = before.is_none();
    let primary = row["primary_group"].as_bool().unwrap_or(false);
    let primary_changed = match before {
        None => primary,
        Some(b) => b["primary_group"] != row["primary_group"],
    };
    if !(new_record && !primary) && primary_changed {
        if primary {
            sqlx::query(
                "UPDATE users SET primary_group_id = $1 \
                 WHERE id IN (SELECT user_id FROM group_users WHERE group_id = $1)",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "UPDATE users SET flair_group_id = $1 \
                 WHERE id IN (SELECT user_id FROM group_users WHERE group_id = $1) AND flair_group_id IS NULL",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
        } else {
            for column in ["primary_group_id", "flair_group_id"] {
                sqlx::query(&format!(
                    "UPDATE users SET {column} = NULL \
                     WHERE id IN (SELECT user_id FROM group_users WHERE group_id = $1) AND {column} = $1"
                ))
                .bind(id)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    let title = row["title"].clone();
    let title_changed = was("title") != title && !(new_record && title.is_null());
    if !(new_record && blank(&title)) && title_changed {
        sqlx::query(
            "UPDATE users SET title = $1 \
             WHERE (title = $2 OR title = '' OR title IS NULL) \
               AND COALESCE(title, '') <> COALESCE($1, '') \
               AND id IN (SELECT user_id FROM group_users WHERE group_id = $3)",
        )
        .bind(s(&title))
        .bind(s(&was("title")))
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(before) = before
        && before["name"] != row["name"]
    {
        crate::jobs::enqueue(
            &mut *conn,
            "update_group_mentions",
            json!({ "previous_name": before["name"], "group_id": id }),
        )
        .await?;
    }
    if was("flair_upload_id") != row["flair_upload_id"] && !row["flair_upload_id"].is_null() {
        return Err(Unsupported("group flair uploads (UploadReference)").into());
    }
    Ok(())
}

/// The after_commit callbacks of a save: `automatic_group_membership`.
pub async fn after_commit(
    conn: &mut PgConnection,
    id: i32,
    row: &Map<String, Value>,
) -> Result<(), AppError> {
    if !blank(&row["automatic_membership_email_domains"]) {
        crate::jobs::enqueue(
            &mut *conn,
            "automatic_group_membership",
            json!({ "group_id": id }),
        )
        .await?;
    }
    Ok(())
}

/// A column's value as GroupHistory's text columns store it.
fn history_value(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "t" } else { "f" }.to_string()),
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// `GroupActionLogger#log_change_group_settings`: a row a changed
/// attribute (not bio_cooked), unless both values are blank.
pub async fn log_changes(
    conn: &mut PgConnection,
    group_id: i32,
    acting_user_id: i32,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    changed: &[&str],
) -> Result<(), sqlx::Error> {
    for column in changed {
        if *column == "bio_cooked" {
            continue;
        }
        let (prev, new) = (&before[*column], &after[*column]);
        let blank_v = |v: &Value| match v {
            Value::Null => true,
            Value::String(s) => s.trim().is_empty(),
            Value::Bool(b) => !b,
            _ => false,
        };
        if blank_v(prev) && blank_v(new) {
            continue;
        }
        sqlx::query(
            "INSERT INTO group_histories (group_id, acting_user_id, action, subject, prev_value, new_value, \
                                          created_at, updated_at) \
             VALUES ($1, $2, 1, $3, $4, $5, clock_timestamp(), clock_timestamp())",
        )
        .bind(group_id)
        .bind(acting_user_id)
        .bind(column)
        .bind(history_value(prev))
        .bind(history_value(new))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Casts the permitted request attributes onto a row.
pub fn assign(row: &mut Map<String, Value>, attrs: &Map<String, Value>) {
    for (key, value) in attrs {
        if let Some(kind) = kind_of(key) {
            row.insert(key.clone(), cast(kind, value));
        }
    }
}

/// `GroupCategoryNotificationDefault.batch_set` /
/// `GroupTagNotificationDefault.batch_set` for one level: the group's
/// defaults at that level made exactly these categories (or tags).
pub async fn batch_set(
    conn: &mut PgConnection,
    group_id: i32,
    level: i32,
    tags: bool,
    values: &[String],
) -> Result<(), sqlx::Error> {
    let (table, column) = if tags {
        ("group_tag_notification_defaults", "tag_id")
    } else {
        ("group_category_notification_defaults", "category_id")
    };
    let ids: Vec<i32> = if tags {
        sqlx::query_scalar("SELECT id FROM tags WHERE name = ANY($1) ORDER BY id")
            .bind(values)
            .fetch_all(&mut *conn)
            .await?
    } else {
        let wanted: Vec<i32> = values
            .iter()
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        sqlx::query_scalar("SELECT id FROM categories WHERE id = ANY($1) ORDER BY id")
            .bind(&wanted)
            .fetch_all(&mut *conn)
            .await?
    };
    if !ids.is_empty() {
        sqlx::query(&format!(
            "UPDATE {table} SET notification_level = $3 WHERE group_id = $1 AND {column} = ANY($2) \
             AND notification_level <> $3"
        ))
        .bind(group_id)
        .bind(&ids)
        .bind(level)
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query(&format!(
        "DELETE FROM {table} WHERE group_id = $1 AND notification_level = $3 AND NOT ({column} = ANY($2))"
    ))
    .bind(group_id)
    .bind(&ids)
    .bind(level)
    .execute(&mut *conn)
    .await?;
    for id in ids {
        sqlx::query(&format!(
            "INSERT INTO {table} (group_id, {column}, notification_level) VALUES ($1, $2, $3) \
             ON CONFLICT DO NOTHING"
        ))
        .bind(group_id)
        .bind(id)
        .bind(level)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
