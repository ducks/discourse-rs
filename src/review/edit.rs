//! Editing a reviewable: ReviewablesController#update (PUT
//! /review/:reviewable_id) with `Reviewable#editable_for` and
//! `#update_fields`. Only a pending queued post has editable fields
//! (ReviewableQueuedPost#build_editable_fields): a new topic's title,
//! category (staff) and tags, and the raw. The fields are merged into the
//! payload and the category set, the version raised, the edit logged with
//! its changes, the raw's uploads referenced and notify_reviewable
//! enqueued; the answer echoes the params with the new version, the
//! payload's cooked raw and its escaped title.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::reviewables::{self, PENDING};
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::{AppError, Unsupported};

/// `ReviewableHistory.types[:edited]`
const HISTORY_EDITED: i32 = 2;

/// How an update ends when it isn't a server error.
pub enum Updated {
    Done(Value),
    NotFound,
    Forbidden,
    Conflict,
    ClaimError(&'static str),
}

#[derive(sqlx::FromRow)]
struct Row {
    #[sqlx(rename = "type")]
    kind: String,
    status: i32,
    topic_id: Option<i32>,
    category_id: Option<i32>,
    payload: Option<Value>,
}

/// `editable_for(guardian)`: the fields' ids.
fn editable_fields(row: &Row, guardian: &Guardian) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if row.kind != "ReviewableQueuedPost" || row.status != PENDING {
        return fields;
    }
    if row.topic_id.is_none() {
        fields.push("payload.title");
        if guardian.is_staff() {
            fields.push("category_id");
        }
        fields.push("payload.tags");
    }
    fields.push("payload.raw");
    fields
}

/// `self.category_id = value`: ActiveModel's integer cast (blank to nil,
/// a string by `to_i`).
fn category_cast(value: &Value) -> Result<Option<i32>, Unsupported> {
    let n = match value {
        Value::Null => return Ok(None),
        Value::String(s) if crate::ruby::is_blank(s) => return Ok(None),
        Value::String(s) => crate::ruby::to_i(s),
        Value::Number(n) => n
            .as_i64()
            .ok_or(Unsupported("a category id that isn't a whole number"))?,
        Value::Bool(b) => i64::from(*b),
        _ => return Err(Unsupported("a category id that isn't a scalar")),
    };
    i32::try_from(n)
        .map(Some)
        .map_err(|_| Unsupported("a category id past the integer range"))
}

/// `PUT /review/:reviewable_id` past its filters, for the staff member
/// `guardian` is, with `params[:reviewable]` (None when not given).
pub async fn update(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    reviewable_id: i64,
    version: i64,
    edit: Option<&Value>,
) -> Result<Updated, AppError> {
    let s = ctx.settings;
    let Some(user) = guardian.user() else {
        return Ok(Updated::Forbidden);
    };
    // find_reviewable: Reviewable.viewable_by(current_user)
    if !reviewables::is_viewable(&mut *conn, s, guardian, reviewable_id).await? {
        return Ok(Updated::NotFound);
    }
    let row: Row = sqlx::query_as(
        "SELECT type, status, topic_id, category_id, payload FROM reviewables WHERE id = $1",
    )
    .bind(reviewable_id)
    .fetch_one(&mut *conn)
    .await?;
    if let Some(key) = super::claim_error(&mut *conn, s, user.id, row.topic_id).await? {
        return Ok(Updated::ClaimError(key));
    }
    let editable = editable_fields(&row, guardian);
    if editable.is_empty() {
        return Ok(Updated::Forbidden);
    }
    // params[:reviewable] || {}
    let empty = Map::new();
    let edit = match edit {
        None | Some(Value::Null) => &empty,
        Some(Value::Object(map)) => map,
        Some(_) => return Err(Unsupported("a reviewable param that isn't a hash").into()),
    };
    // Every param editable: a hash's keys as `name.key`.
    for (name, value) in edit {
        let allowed = match value {
            Value::Object(nested) => nested
                .keys()
                .all(|key| editable.contains(&format!("{name}.{key}").as_str())),
            _ => editable.contains(&name.as_str()),
        };
        if !allowed {
            return Ok(Updated::Forbidden);
        }
    }

    let mut payload = row.payload.clone();
    let mut category_id = row.category_id;
    let version = if edit.is_empty() {
        // update_fields: nothing to do.
        sqlx::query_scalar::<_, i32>("SELECT version FROM reviewables WHERE id = $1")
            .bind(reviewable_id)
            .fetch_one(&mut *conn)
            .await?
    } else {
        // can_review_target?: a pending queued post has no target post.
        if let Some(fields) = edit.get("payload").and_then(Value::as_object) {
            let Some(Value::Object(stored)) = payload.as_mut() else {
                return Err(Unsupported("editing a queued post without a payload").into());
            };
            for (key, value) in fields {
                if value.is_object() {
                    return Err(Unsupported("a payload field that is a hash").into());
                }
                stored.insert(key.clone(), value.clone());
            }
        }
        if let Some(value) = edit.get("category_id") {
            category_id = category_cast(value)?;
        }
        // increment_version!(version)
        let Some(version) = sqlx::query_scalar::<_, i32>(
            "UPDATE reviewables SET version = version + 1 WHERE id = $1 AND version = $2 RETURNING version",
        )
        .bind(reviewable_id)
        .bind(version as i32)
        .fetch_optional(&mut *conn)
        .await?
        else {
            return Ok(Updated::Conflict);
        };
        // changes.as_json less the version.
        let mut changes = Map::new();
        if payload != row.payload {
            changes.insert("payload".into(), json!([row.payload, payload]));
        }
        if category_id != row.category_id {
            changes.insert("category_id".into(), json!([row.category_id, category_id]));
        }
        // save, with set_type_source.
        sqlx::query(
            "UPDATE reviewables SET payload = $2, category_id = $3, version = $4, type_source = 'core', \
                    updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(reviewable_id)
        .bind(&payload)
        .bind(category_id)
        .bind(version)
        .execute(&mut *conn)
        .await?;
        // log_history(:edited, performed_by, edited: changes_json)
        sqlx::query(
            "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                               edited, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp())",
        )
        .bind(reviewable_id)
        .bind(HISTORY_EDITED)
        .bind(row.status)
        .bind(user.id)
        .bind(Value::Object(changes))
        .execute(&mut *conn)
        .await?;
        // after_save: the raw's uploads, when the payload changed.
        if payload != row.payload
            && let Some(Value::String(raw)) = payload.as_ref().and_then(|p| p.get("raw"))
            && !crate::ruby::is_blank(raw)
        {
            let ids = crate::upload_references::extract_upload_ids(&mut *conn, raw).await?;
            crate::upload_references::ensure_exist_many(
                &mut *conn,
                "ReviewableQueuedPost",
                reviewable_id,
                &ids,
            )
            .await?;
        }
        // after_commit: notify_reviewable while pending.
        crate::jobs::enqueue(
            &mut *conn,
            "notify_reviewable",
            json!({ "reviewable_id": reviewable_id }),
        )
        .await?;
        version
    };

    // edit_params.merge(version:), then the payload's cooked and title.
    let mut result = edit.clone();
    result.insert("version".into(), json!(version));
    let stored = payload.as_ref().and_then(Value::as_object);
    if let Some(stored) = stored.filter(|p| !p.is_empty()) {
        match stored.get("raw") {
            None | Some(Value::Null) => {}
            Some(Value::String(raw)) => {
                let cooked = crate::pretty_text::cook(
                    ctx.host,
                    raw,
                    &crate::pretty_text::MarkdownOptions::default(),
                )
                .await?;
                result.insert("cooked".into(), json!(cooked));
            }
            Some(_) => {
                return Err(Unsupported("cooking a queued post's raw that isn't text").into());
            }
        }
        match stored.get("title") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => {}
            Some(Value::String(title)) => {
                result.insert(
                    "fancy_title".into(),
                    json!(crate::categories::badge::html_escape(title)),
                );
            }
            Some(_) => {
                return Err(Unsupported("escaping a queued post's title that isn't text").into());
            }
        }
    }
    Ok(Updated::Done(Value::Object(result)))
}
