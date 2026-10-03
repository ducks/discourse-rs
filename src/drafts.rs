//! Drafts: `Draft.set`, `Draft.get` and `Draft.clear` as DraftsController
//! calls them, with the draft sequence that guards them and the user's
//! draft count.
//!
//! Refused: drafts backed up to a message (backup_drafts_to_pm_length),
//! drafts holding uploads (UploadReference), the edit conflict check on a
//! first post's tags, reading a draft at a given sequence, and acting on
//! another user's drafts.

use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// How saving a draft ends.
pub enum Saved {
    /// The draft sequence now.
    Sequence(i64),
    /// `Draft::OutOfSequence`
    OutOfSequence,
}

async fn current_sequence(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
) -> Result<i64, sqlx::Error> {
    let seq: Option<i64> = sqlx::query_scalar(
        "SELECT sequence FROM draft_sequences WHERE user_id = $1 AND draft_key = $2",
    )
    .bind(user_id)
    .bind(key)
    .fetch_optional(conn)
    .await?;
    Ok(seq.unwrap_or(0))
}

/// `UserStat.update_draft_count(user_id)`
async fn update_draft_count(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_stats SET draft_count = (SELECT COUNT(*) FROM drafts WHERE user_id = $1) WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// What is not ported about a draft's data.
fn refuse_data(data: &str) -> Result<(), Unsupported> {
    if data.contains("upload://") || data.contains("/uploads/") {
        return Err(Unsupported("drafts with uploads (UploadReference)"));
    }
    Ok(())
}

/// What a save sends: the draft's key, the sequence the client holds, the
/// data, its owner and `force_save`.
pub struct NewDraft<'a> {
    pub key: &'a str,
    pub sequence: i64,
    pub data: &'a str,
    pub owner: Option<&'a str>,
    pub force_save: bool,
}

/// `Draft.set(user, key, sequence, data, owner, force_save:)`
pub async fn set(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    draft: &NewDraft<'_>,
) -> Result<Saved, AppError> {
    let (key, sequence, data, owner, force_save) = (
        draft.key,
        draft.sequence,
        draft.data,
        draft.owner,
        draft.force_save,
    );
    // User.human_user_id?
    if user_id <= 0 {
        return Ok(Saved::Sequence(0));
    }
    let backup = s.get("backup_drafts_to_pm_length")?.to_i();
    if backup > 0 && (backup as usize) < data.chars().count() {
        return Err(Unsupported("drafts backed up to a message").into());
    }
    refuse_data(data)?;
    let existing: Option<(i32, Option<String>)> =
        sqlx::query_as("SELECT id, owner FROM drafts WHERE user_id = $1 AND draft_key = $2")
            .bind(user_id)
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?;
    let current = current_sequence(&mut *conn, user_id, key).await?;
    let (draft_id, sequence) = if let Some((id, current_owner)) = existing {
        if !force_save && current != sequence {
            return Ok(Saved::OutOfSequence);
        }
        let sequence = if force_save { current } else { sequence } + 1;
        // The sequence moves on with every save.
        sqlx::query(
            "INSERT INTO draft_sequences (user_id, draft_key, sequence) VALUES ($1, $2, $3) \
             ON CONFLICT (user_id, draft_key) DO UPDATE SET sequence = EXCLUDED.sequence",
        )
        .bind(user_id)
        .bind(key)
        .bind(sequence)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE drafts SET sequence = $2, data = $3, revisions = revisions + 1, owner = $4, \
                               updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(sequence)
        .bind(data)
        .bind(owner.map(str::to_string).or(current_owner))
        .execute(&mut *conn)
        .await?;
        (id, sequence)
    } else if sequence != current {
        return Ok(Saved::OutOfSequence);
    } else {
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO drafts (user_id, draft_key, data, sequence, owner, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, now(), now()) \
             ON CONFLICT (user_id, draft_key) DO UPDATE SET sequence = $4, data = $3, \
               revisions = drafts.revisions + 1, owner = $5, updated_at = now() \
             RETURNING id",
        )
        .bind(user_id)
        .bind(key)
        .bind(data)
        .bind(sequence)
        .bind(owner)
        .fetch_one(&mut *conn)
        .await?;
        update_draft_count(&mut *conn, user_id).await?;
        (id, sequence)
    };
    // UploadReference.ensure_exist!: the draft holds no uploads.
    sqlx::query("DELETE FROM upload_references WHERE target_type = 'Draft' AND target_id = $1")
        .bind(draft_id)
        .execute(&mut *conn)
        .await?;
    Ok(Saved::Sequence(sequence))
}

/// DraftsController#create's `Draft.set`, retried at the current sequence
/// when the draft does not exist (the client's sequence ran behind).
pub async fn create(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    draft: &NewDraft<'_>,
) -> Result<Saved, AppError> {
    let key = draft.key;
    match set(conn, s, user_id, draft).await? {
        Saved::OutOfSequence => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM drafts WHERE user_id = $1 AND draft_key = $2)",
            )
            .bind(user_id)
            .bind(key)
            .fetch_one(&mut *conn)
            .await?;
            if exists {
                return Ok(Saved::OutOfSequence);
            }
            let current = current_sequence(&mut *conn, user_id, key).await?;
            let retry = NewDraft {
                sequence: current,
                force_save: false,
                ..*draft
            };
            set(conn, s, user_id, &retry).await
        }
        saved => Ok(saved),
    }
}

/// `reached_max_drafts_per_user?`
pub async fn reached_max(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    key: &str,
) -> Result<bool, AppError> {
    let (count, exists): (i64, bool) = sqlx::query_as(
        "SELECT COUNT(*), BOOL_OR(draft_key = $2) IS TRUE FROM drafts WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(key)
    .fetch_one(conn)
    .await?;
    Ok(count >= s.get("max_drafts_per_user")?.to_i() && !exists)
}

/// DraftsController#show: `Draft.get` at the current sequence, and that
/// sequence.
pub async fn show(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
) -> Result<(Option<String>, i64), AppError> {
    let current = current_sequence(&mut *conn, user_id, key).await?;
    if user_id <= 0 {
        return Ok((None, current));
    }
    let draft: Option<(String, i64)> =
        sqlx::query_as("SELECT data, sequence FROM drafts WHERE user_id = $1 AND draft_key = $2")
            .bind(user_id)
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?;
    // Only a draft at the current sequence is the user's draft.
    let data = draft.and_then(|(data, seq)| (seq == current).then_some(data));
    Ok((data, current))
}

/// `Draft.clear(user, key, sequence)`: false when the sequence is not the
/// current one (the controller answers success either way).
pub async fn clear(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
    sequence: i64,
) -> Result<bool, AppError> {
    if user_id <= 0 {
        return Err(Unsupported("clearing drafts of non-human users").into());
    }
    if current_sequence(&mut *conn, user_id, key).await? != sequence {
        return Ok(false);
    }
    // destroy_all: the upload references go with each draft, then the
    // count on commit.
    let ids: Vec<i32> =
        sqlx::query_scalar("DELETE FROM drafts WHERE user_id = $1 AND draft_key = $2 RETURNING id")
            .bind(user_id)
            .bind(key)
            .fetch_all(&mut *conn)
            .await?;
    if !ids.is_empty() {
        sqlx::query(
            "DELETE FROM upload_references WHERE target_type = 'Draft' AND target_id = ANY($1)",
        )
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
        update_draft_count(&mut *conn, user_id).await?;
    }
    Ok(true)
}
