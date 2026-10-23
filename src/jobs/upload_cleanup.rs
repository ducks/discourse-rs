//! Uploads nobody uses removed (Jobs::CleanUpUploads, with Upload#destroy)
//! and the tombstone emptied of files past the grace period
//! (Jobs::PurgeDeletedUploads).

use chrono::Duration;
use sqlx::PgPool;

use crate::file_store::FileStore;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::CleanUpUploads`, hourly.
pub const CLEAN_UP_UPLOADS: &str = "clean_up_uploads";
/// `Jobs::PurgeDeletedUploads`, daily.
pub const PURGE_DELETED_UPLOADS: &str = "purge_deleted_uploads";

/// The Redis key CleanUpUploads throttles itself with.
const LAST_UPLOAD_CLEANUP: &str = "LAST_UPLOAD_CLEANUP";

/// `Upload#destroy`: the file to the tombstone, then in one transaction
/// its hotlinked media, optimized images (their files too), optimized
/// videos (`OptimizedVideo#destroy`), user uploads and references gone,
/// badges' and profiles' links to it cleared, the row deleted, and
/// avatars pointing at it cleared.
pub async fn destroy(pool: &PgPool, store: &FileStore, upload_id: i32) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    destroy_in(&mut tx, store, upload_id).await?;
    tx.commit().await?;
    Ok(())
}

/// `Upload#destroy` inside the caller's transaction. An upload an
/// optimized video was made into keeps that video's foreign key to it,
/// which `destroy` does not clear: the delete fails there, as in Rails.
fn destroy_in<'a>(
    tx: &'a mut sqlx::PgConnection,
    store: &'a FileStore,
    upload_id: i32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + 'a>> {
    Box::pin(async move {
        let url: String = sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
            .bind(upload_id)
            .fetch_one(&mut *tx)
            .await?;
        store.remove_file(&url).await?;
        sqlx::query("DELETE FROM post_hotlinked_media WHERE upload_id = $1")
            .bind(upload_id)
            .execute(&mut *tx)
            .await?;
        let optimized: Vec<(i32, String)> =
            sqlx::query_as("SELECT id, url FROM optimized_images WHERE upload_id = $1")
                .bind(upload_id)
                .fetch_all(&mut *tx)
                .await?;
        for (id, optimized_url) in optimized {
            store.remove_file(&optimized_url).await?;
            sqlx::query("DELETE FROM optimized_images WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        // OptimizedVideo#destroy: the optimized upload's file and its
        // reference from this upload removed, the row deleted, then the
        // optimized upload destroyed.
        let videos: Vec<(i64, i32, Option<String>)> = sqlx::query_as(
            "SELECT v.id, v.optimized_upload_id, u.url FROM optimized_videos v \
             LEFT JOIN uploads u ON u.id = v.optimized_upload_id WHERE v.upload_id = $1 ORDER BY v.id",
        )
        .bind(upload_id)
        .fetch_all(&mut *tx)
        .await?;
        for (id, optimized_upload_id, optimized_url) in videos {
            if let Some(optimized_url) = &optimized_url {
                store.remove_file(optimized_url).await?;
            }
            sqlx::query(
                "DELETE FROM upload_references WHERE upload_id = $1 \
                 AND target_type = 'Upload' AND target_id = $2",
            )
            .bind(optimized_upload_id)
            .bind(i64::from(upload_id))
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM optimized_videos WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            if optimized_url.is_some() {
                destroy_in(&mut *tx, store, optimized_upload_id).await?;
            }
        }
        for table in ["user_uploads", "upload_references"] {
            sqlx::query(&format!("DELETE FROM {table} WHERE upload_id = $1"))
                .bind(upload_id)
                .execute(&mut *tx)
                .await?;
        }
        for (table, column) in [
            ("badges", "image_upload_id"),
            ("user_profiles", "card_background_upload_id"),
            ("user_profiles", "profile_background_upload_id"),
        ] {
            sqlx::query(&format!(
                "UPDATE {table} SET {column} = NULL WHERE {column} = $1"
            ))
            .bind(upload_id)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM uploads WHERE id = $1")
            .bind(upload_id)
            .execute(&mut *tx)
            .await?;
        for (table, column) in [
            ("users", "uploaded_avatar_id"),
            ("user_avatars", "gravatar_upload_id"),
            ("user_avatars", "custom_upload_id"),
        ] {
            sqlx::query(&format!(
                "UPDATE {table} SET {column} = NULL WHERE {column} = $1"
            ))
            .bind(upload_id)
            .execute(&mut *tx)
            .await?;
        }
        Ok(())
    })
}

/// `Jobs::CleanUpUploads`: uploads that never got a url destroyed
/// always; then, unless turned off or run in the last half grace period,
/// uploads past the grace period without references (and not secure to a
/// post) destroyed unless chat still uses them, and expired external
/// upload stubs deleted.
pub async fn clean_up_uploads(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = FileStore::for_site(&state.config, &s)?;
    let now = crate::clock::now_naive();
    let grace_hours = s
        .get("clean_orphan_uploads_grace_period_hours")?
        .to_i()
        .max(1);
    let grace_ago = now - Duration::hours(grace_hours);
    let invalid: Vec<i32> = sqlx::query_scalar(
        "SELECT uploads.id FROM uploads WHERE (uploads.id > 0) \
           AND (retain_hours IS NULL OR created_at < now() - interval '1 hour' * retain_hours) \
           AND (created_at < $1) AND uploads.url = '' ORDER BY uploads.id ASC",
    )
    .bind(grace_ago)
    .fetch_all(&mut *conn)
    .await?;
    for id in invalid {
        destroy(&state.pool, &store, id).await?;
    }
    if !s.get("clean_up_uploads")?.truthy() {
        return Ok(());
    }
    // Half the grace period (in whole hours) since the last run.
    if let Some(last) = crate::owned_schema::cached_get(&mut conn, LAST_UPLOAD_CLEANUP).await? {
        let last = crate::ruby::to_i(&last);
        if now.and_utc().timestamp() - last < (grace_hours / 2) * 3600 {
            return Ok(());
        }
    }
    let orphans: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT uploads.id, uploads.sha1 FROM uploads \
         LEFT JOIN upload_references ON upload_references.upload_id = uploads.id \
         LEFT JOIN upload_references ur ON ur.upload_id = uploads.id AND ur.target_type != 'Post' \
         WHERE (uploads.id > 0) \
           AND (uploads.retain_hours IS NULL OR uploads.created_at < now() - interval '1 hour' * uploads.retain_hours) \
           AND (uploads.created_at < $1) \
           AND (((uploads.access_control_post_id IS NULL) OR (uploads.access_control_post_id IS NOT NULL AND NOT uploads.secure))) \
           AND (upload_references.upload_id IS NULL) AND (ur.upload_id IS NULL) \
         ORDER BY uploads.id ASC",
    )
    .bind(grace_ago)
    .fetch_all(&mut *conn)
    .await?;
    for (id, sha1) in orphans {
        // Chat's in_use callback reads the base62 sha1, which a missing
        // sha1 makes raise: the job fails there, as in Rails.
        let Some(sha1) = sha1 else {
            return Err(
                std::io::Error::other("an orphan upload without a sha1 (Base62 of nil)").into(),
            );
        };
        if crate::plugins::chat::upload_in_use(&mut conn, id, &sha1).await? {
            continue;
        }
        // sha1.present? ? destroy : delete (no callbacks, no file moved).
        if sha1.is_empty() {
            sqlx::query("DELETE FROM uploads WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        } else {
            destroy(&state.pool, &store, id).await?;
        }
    }
    // ExternalUploadStub.cleanup!
    let debug = s.get("enable_upload_debug_mode")?.truthy();
    sqlx::query("DELETE FROM external_upload_stubs WHERE (status = 1 AND created_at <= $1)")
        .bind(now - Duration::hours(if debug { 48 } else { 1 }))
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM external_upload_stubs WHERE (status = 2 AND created_at <= $1)")
        .bind(now - Duration::hours(24))
        .execute(&mut *conn)
        .await?;
    crate::owned_schema::cached_setex(
        &mut conn,
        LAST_UPLOAD_CLEANUP,
        7 * 24 * 3600,
        &now.and_utc().timestamp().to_string(),
    )
    .await?;
    Ok(())
}

/// `Jobs::PurgeDeletedUploads`: tombstoned files older than
/// purge_deleted_uploads_grace_period_days removed.
pub async fn purge_deleted_uploads(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let store = FileStore::for_site(&state.config, &s)?;
    let days = s.get("purge_deleted_uploads_grace_period_days")?.to_i();
    store.purge_tombstone(days).await?;
    Ok(())
}
