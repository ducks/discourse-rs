//! User status: UserStatus (an emoji and a description, until an optional
//! end), User#set_status!, #clear_status!, #publish_user_status and
//! #has_status?, UserGuardian#can_see_user_status?, UserStatusSerializer
//! and UserStatusMixin's `status` for the serializers that include it.
//!
//! Rails has no job clearing ended statuses: an ended one stays in
//! user_statuses, has_status? is false for it, and the next set replaces it.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, Timelike};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// UserStatus::MAX_DESCRIPTION_LENGTH
pub const MAX_DESCRIPTION_LENGTH: usize = 100;

/// `Group::AUTO_GROUPS[:trust_level_0]` and `[:staff]`.
const TRUST_LEVEL_0: i32 = 10;
const STAFF: i32 = 3;

/// `validate!`'s full messages, raised as ActiveRecord::RecordInvalid:
/// the caller renders them as a 422 `record_invalid` (and nothing the
/// request wrote stays, as Rails' transaction rolls back).
#[derive(Debug)]
pub struct RecordInvalid(pub Vec<String>);

impl std::fmt::Display for RecordInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user status invalid: {}", self.0.join(", "))
    }
}

impl std::error::Error for RecordInvalid {}

/// `render_json_error(e, type: :record_invalid, status: 422)`.
pub fn record_invalid_response(errors: &[String]) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        axum::Json(json!({ "errors": errors, "error_type": "record_invalid" })),
    )
        .into_response()
}

/// `Time#iso8601`: whole seconds in UTC.
fn iso8601(t: NaiveDateTime) -> String {
    t.and_utc().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// UserStatusSerializer. `MessageBus.last_id("/user-status")`: pg-bus has
/// one position for every channel, not an id per channel, so it is 0 (what
/// Rails says for a channel nothing was published to).
pub fn serialize(description: &str, emoji: &str, ends_at: Option<NaiveDateTime>) -> Value {
    json!({
        "description": description,
        "emoji": emoji,
        "ends_at": ends_at.map(iso8601),
        "message_bus_last_id": 0,
    })
}

#[derive(sqlx::FromRow)]
struct Row {
    user_id: i32,
    description: String,
    emoji: String,
    ends_at: Option<NaiveDateTime>,
    hide_profile: bool,
    silenced: bool,
}

/// UserStatusMixin#include_status? for each user, with `viewer` as the
/// scope (`scope || Guardian.new`: pass `Guardian::anonymous()` for a
/// serializer given none): the site setting, a status that has not ended
/// (`has_status?`), a profile not hidden, and can_see_user_status? (a
/// silenced user's status only to them and staff). What it serializes,
/// by user id.
pub async fn visible(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    viewer: &Guardian,
    user_ids: &[i32],
) -> Result<HashMap<i32, Value>, AppError> {
    Ok(load(conn, settings, user_ids)
        .await?
        .into_iter()
        .filter(|r| {
            !r.hide_profile && (!r.silenced || viewer.is_me(r.user_id) || viewer.is_staff())
        })
        .map(|r| (r.user_id, serialize(&r.description, &r.emoji, r.ends_at)))
        .collect())
}

/// The statuses of the users that have not ended, when the site has them.
async fn load(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_ids: &[i32],
) -> Result<Vec<Row>, AppError> {
    if user_ids.is_empty() || !settings.get("enable_user_status")?.truthy() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT s.user_id, s.description, s.emoji, s.ends_at, \
                COALESCE(uo.hide_profile, FALSE) AS hide_profile, \
                COALESCE(u.silenced_till > now(), FALSE) AS silenced \
         FROM user_statuses s \
         JOIN users u ON u.id = s.user_id \
         LEFT JOIN user_options uo ON uo.user_id = s.user_id \
         WHERE s.user_id = ANY($1) AND (s.ends_at IS NULL OR s.ends_at >= now())",
    )
    .bind(user_ids)
    .fetch_all(&mut *conn)
    .await?)
}

/// One user's [`visible`] status.
pub async fn visible_one(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    viewer: &Guardian,
    user_id: i32,
) -> Result<Option<Value>, AppError> {
    Ok(visible(conn, settings, viewer, &[user_id])
        .await?
        .remove(&user_id))
}

/// The status a user sees of their own (CurrentUserSerializer): as
/// [`visible`], to a viewer who may see it when silenced.
pub async fn visible_to_self(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<Option<Value>, AppError> {
    let rows = load(conn, settings, &[user_id]).await?;
    Ok(rows
        .into_iter()
        .find(|r| !r.hide_profile)
        .map(|r| serialize(&r.description, &r.emoji, r.ends_at)))
}

/// The `status` attribute where UserStatusMixin puts it: first, as the
/// mixin is included before BasicUserSerializer's attributes.
pub fn prepend(user: &mut Map<String, Value>, status: Value) {
    let rest = std::mem::take(user);
    user.insert("status".into(), status);
    user.extend(rest.into_iter().filter(|(k, _)| k != "status"));
}

/// [`prepend`] on each serialized user (by its `id`) that has a visible
/// status.
pub fn decorate(users: &mut [Value], statuses: &HashMap<i32, Value>) {
    for user in users {
        let id = user["id"].as_i64().map(|id| id as i32);
        if let (Some(id), Value::Object(map)) = (id, user)
            && let Some(status) = statuses.get(&id)
        {
            prepend(map, status.clone());
        }
    }
}

/// [`visible`] for the serialized users given, [`decorate`]d in place.
pub async fn include(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    viewer: &Guardian,
    users: &mut [Value],
) -> Result<(), AppError> {
    let ids: Vec<i32> = users
        .iter()
        .filter_map(|u| u["id"].as_i64())
        .map(|id| id as i32)
        .collect();
    let statuses = visible(conn, settings, viewer, &ids).await?;
    decorate(users, &statuses);
    Ok(())
}

/// The current user's own status, ended or not (`current_user.user_status`,
/// as GET /user-status answers it).
pub async fn own(conn: &mut PgConnection, user_id: i32) -> Result<Option<Value>, AppError> {
    let row: Option<(String, String, Option<NaiveDateTime>)> =
        sqlx::query_as("SELECT description, emoji, ends_at FROM user_statuses WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.map(|(d, e, t)| serialize(&d, &e, t)))
}

/// `status.blank?` of PUT /u's status param.
pub fn blank_param(status: &Value) -> bool {
    match status {
        Value::Null => true,
        Value::String(v) => crate::ruby::is_blank(v),
        Value::Object(o) => o.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Bool(b) => !b,
        Value::Number(_) => false,
    }
}

/// An `ends_at` param as ActiveRecord casts it for a datetime column:
/// blank is nil; ISO 8601 (with `T` or a space, a fraction kept to the
/// microsecond, `Z` or an offset, else UTC as the reference's zone).
/// Anything else Rails' fallback parser reads its own way, so it is
/// refused.
pub fn cast_ends_at(value: Option<&str>) -> Result<Option<NaiveDateTime>, AppError> {
    let Some(value) = value.filter(|v| !crate::ruby::is_blank(v)) else {
        return Ok(None);
    };
    let v = value.trim();
    let parsed = DateTime::parse_from_rfc3339(&v.replacen(' ', "T", 1))
        .map(|t| t.naive_utc())
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%.f").ok())
        .or_else(|| NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S%.f").ok())
        .ok_or(Unsupported("a user status ends_at that is not ISO 8601"))?;
    Ok(Some(micros(parsed)))
}

/// Stored to the microsecond, as Postgres keeps a timestamp.
fn micros(t: NaiveDateTime) -> NaiveDateTime {
    t.with_nanosecond(t.nanosecond() / 1_000 * 1_000)
        .unwrap_or(t)
}

/// ActiveModel's `humanize` of an attribute without a translated name.
fn human(attr: &str) -> String {
    let base = attr.replace('_', " ");
    let mut chars = base.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// UserStatus' validations, in their order: the emoji exists, the
/// description at most 100 characters, an end not before set_at.
fn validate(
    settings: &SiteSettings,
    i18n: &I18n,
    description: &str,
    emoji: &str,
    set_at: NaiveDateTime,
    ends_at: Option<NaiveDateTime>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if !crate::categories::writes::emoji_exists(settings, emoji) {
        let m = i18n.t("errors.messages.invalid").unwrap_or("is invalid");
        errors.push(format!("{} {m}", human("emoji")));
    }
    if description.chars().count() > MAX_DESCRIPTION_LENGTH {
        let m = i18n
            .t_count(
                "errors.messages.too_long",
                MAX_DESCRIPTION_LENGTH as i64,
                &[],
            )
            .unwrap_or_default();
        errors.push(format!("{} {m}", human("description")));
    }
    if ends_at.is_some_and(|e| set_at > e) {
        let m = i18n
            .t("user_status.errors.ends_at_should_be_greater_than_set_at")
            .unwrap_or_default();
        errors.push(format!("{} {m}", human("ends_at")));
    }
    errors
}

/// `User#set_status!(description, emoji, ends_at)`: validated, upserted
/// with set_at now, and published.
#[allow(clippy::too_many_arguments)]
pub async fn set(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    i18n: &I18n,
    user_id: i32,
    description: &str,
    emoji: &str,
    ends_at: Option<NaiveDateTime>,
) -> Result<(), AppError> {
    let set_at = micros(crate::clock::now_naive());
    let errors = validate(settings, i18n, description, emoji, set_at, ends_at);
    if !errors.is_empty() {
        return Err(RecordInvalid(errors).into());
    }
    sqlx::query(
        "INSERT INTO user_statuses (user_id, description, emoji, set_at, ends_at) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (user_id) DO UPDATE SET description = EXCLUDED.description, \
           emoji = EXCLUDED.emoji, set_at = EXCLUDED.set_at, ends_at = EXCLUDED.ends_at",
    )
    .bind(user_id)
    .bind(description)
    .bind(emoji)
    .bind(set_at)
    .bind(ends_at)
    .execute(&mut *conn)
    .await?;
    let payload = json!({
        "description": description,
        "emoji": emoji,
        "ends_at": ends_at.map(iso8601),
    });
    publish(conn, bus, user_id, payload).await
}

/// `User#clear_status!`: the row destroyed, and null published.
pub async fn clear(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    user_id: i32,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM user_statuses WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    publish(conn, bus, user_id, Value::Null).await
}

/// `User#publish_user_status`: `{id => payload}` on /user-status to
/// trust_level_0 when an anonymous viewer may see the status, else to the
/// user and staff when the user may (silenced), else to no one (a hidden
/// profile).
async fn publish(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    user_id: i32,
    payload: Value,
) -> Result<(), AppError> {
    let (hide_profile, silenced): (bool, bool) = sqlx::query_as(
        "SELECT COALESCE((SELECT hide_profile FROM user_options WHERE user_id = u.id), FALSE), \
                COALESCE(silenced_till > now(), FALSE) \
         FROM users u WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if hide_profile {
        return Ok(());
    }
    let tags = if silenced {
        vec![crate::bus::user_tag(user_id), crate::bus::group_tag(STAFF)]
    } else {
        vec![crate::bus::group_tag(TRUST_LEVEL_0)]
    };
    let mut message = Map::new();
    message.insert(user_id.to_string(), payload);
    bus.publish(conn, "/user-status", &Value::Object(message), Some(&tags))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_at_casts_iso8601() {
        let t = |s| cast_ends_at(Some(s)).unwrap().map(iso8601);
        assert_eq!(
            t("2099-01-01T10:30:00.000Z").as_deref(),
            Some("2099-01-01T10:30:00Z")
        );
        assert_eq!(
            t("2099-01-01T12:30:00+02:00").as_deref(),
            Some("2099-01-01T10:30:00Z")
        );
        assert_eq!(
            t("2099-01-01 10:30:00").as_deref(),
            Some("2099-01-01T10:30:00Z")
        );
        assert_eq!(cast_ends_at(Some("  ")).unwrap(), None);
        assert_eq!(cast_ends_at(None).unwrap(), None);
        assert!(cast_ends_at(Some("tomorrow")).is_err());
    }

    #[test]
    fn status_goes_first() {
        let mut user = Map::new();
        user.insert("id".into(), json!(1));
        user.insert("username".into(), json!("a"));
        prepend(&mut user, json!({"emoji": "tooth"}));
        let keys: Vec<&String> = user.keys().collect();
        assert_eq!(keys, ["status", "id", "username"]);
    }
}
