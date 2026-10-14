//! The scheduled jobs of accounts and email: activation reminders,
//! deactivating inactive admins, revoking old API keys, and purging
//! expired tokens, keys, codes and email logs.

use serde_json::json;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::ActivationReminderEmails`, every 2 hours.
pub const ACTIVATION_REMINDER_EMAILS: &str = "activation_reminder_emails";
/// `Jobs::InvalidateInactiveAdmins`, daily.
pub const INVALIDATE_INACTIVE_ADMINS: &str = "invalidate_inactive_admins";
/// `Jobs::CleanUpApiKeysMaxLife`, daily.
pub const CLEAN_UP_API_KEYS_MAX_LIFE: &str = "clean_up_api_keys_max_life";
/// `Jobs::CleanUpUnusedApiKeys`, daily.
pub const CLEAN_UP_UNUSED_API_KEYS: &str = "clean_up_unused_api_keys";
/// `Jobs::CleanUpUnusedUserApiKeys`, daily.
pub const CLEAN_UP_UNUSED_USER_API_KEYS: &str = "clean_up_unused_user_api_keys";
/// `Jobs::CleanUpUserApiKeysMaxLife`, daily.
pub const CLEAN_UP_USER_API_KEYS_MAX_LIFE: &str = "clean_up_user_api_keys_max_life";
/// `Jobs::CleanUpUnusedRegisteredUserApiKeyClients`, daily.
pub const CLEAN_UP_UNUSED_REGISTERED_USER_API_KEY_CLIENTS: &str =
    "clean_up_unused_registered_user_api_key_clients";
/// `Jobs::CleanUpEmailTokens`, daily.
pub const CLEAN_UP_EMAIL_TOKENS: &str = "clean_up_email_tokens";
/// `Jobs::CleanUpEmailLoginCodes`, daily.
pub const CLEAN_UP_EMAIL_LOGIN_CODES: &str = "clean_up_email_login_codes";
/// `Jobs::CleanUpEmailChangeRequests`, daily.
pub const CLEAN_UP_EMAIL_CHANGE_REQUESTS: &str = "clean_up_email_change_requests";
/// `Jobs::CleanUpUnsubscribeKeys`, daily.
pub const CLEAN_UP_UNSUBSCRIBE_KEYS: &str = "clean_up_unsubscribe_keys";
/// `Jobs::CleanUpPostReplyKeys`, daily.
pub const CLEAN_UP_POST_REPLY_KEYS: &str = "clean_up_post_reply_keys";
/// `Jobs::CleanUpEmailLogs`, daily.
pub const CLEAN_UP_EMAIL_LOGS: &str = "clean_up_email_logs";
/// `Jobs::DeleteRejectedEmails`, weekly.
pub const DELETE_REJECTED_EMAILS: &str = "delete_rejected_emails";

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;
/// `UserHistory.actions`
const DEACTIVATE_USER: i32 = 39;
const API_KEY_UPDATE: i32 = 81;

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `I18n.t(key, count:)`
fn t_count(state: &AppState, key: &str, count: i64) -> String {
    let n = count.to_string();
    state
        .i18n
        .t_count(key, count, &[("count", &n)])
        .unwrap_or_else(|| key.to_string())
}

/// `Jobs::ActivationReminderEmails`: inactive, unstaged users who signed
/// up two to three days ago and were not reminded yet, marked reminded and
/// sent a fresh signup token.
pub async fn activation_reminder_emails(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let users: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT users.id, (SELECT email FROM user_emails e WHERE e.user_id = users.id AND e.\"primary\") \
         FROM users LEFT JOIN user_custom_fields \
           ON users.id = user_id AND user_custom_fields.name = 'activation_reminder' \
         WHERE users.active = FALSE AND users.staged = FALSE AND user_custom_fields.value IS NULL \
           AND users.created_at BETWEEN now() - interval '3 days' AND now() - interval '2 days' \
         ORDER BY users.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for (user_id, email) in users {
        let Some(email) = email else {
            return Err(Unsupported("an activation reminder for a user without an email").into());
        };
        let mut tx = state.pool.begin().await?;
        // save_custom_fields
        sqlx::query(
            "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
             VALUES ($1, 'activation_reminder', 't', clock_timestamp(), clock_timestamp())",
        )
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
        let token = crate::accounts::create_email_token(
            &mut tx,
            user_id,
            &email,
            crate::accounts::token_scopes::SIGNUP,
        )
        .await?;
        super::enqueue(
            &mut tx,
            "user_email",
            json!({ "type": "activation_reminder", "user_id": user_id, "email_token": token }),
        )
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// `Jobs::InvalidateInactiveAdmins`: active admins unseen, without API
/// key use and without posts for invalidate_inactive_admin_email_after_days,
/// deactivated by the system user with their email tokens expired.
pub async fn invalidate_inactive_admins(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let days = s.get("invalidate_inactive_admin_email_after_days")?.to_i();
    if days == 0 {
        return Ok(());
    }
    #[derive(sqlx::FromRow)]
    struct Admin {
        id: i32,
        username_lower: String,
        name: Option<String>,
    }
    let admins: Vec<Admin> = sqlx::query_as(
        "SELECT id, username_lower, name FROM users \
         WHERE users.id > 0 AND admin AND active \
           AND last_seen_at < now() - make_interval(days => $1) \
           AND NOT EXISTS (SELECT 1 FROM api_keys WHERE api_keys.user_id = users.id \
                             AND COALESCE(last_used_at, updated_at) > now() - make_interval(days => $1)) \
           AND NOT EXISTS (SELECT 1 FROM posts WHERE posts.user_id = users.id \
                             AND created_at > now() - make_interval(days => $1))",
    )
    .bind(days as i32)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    let reason = t_count(state, "user.deactivated_by_inactivity", days);
    for admin in admins {
        let mut tx = state.pool.begin().await?;
        // User#deactivate's pending ReviewableUser, the after_save shadow
        // deactivation, and the authenticators' revoke.
        let unported: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM reviewables WHERE type = 'ReviewableUser' \
                              AND target_type = 'User' AND target_id = $1 AND status = 0) \
                 OR EXISTS (SELECT 1 FROM anonymous_users WHERE master_user_id = $1 AND active) \
                 OR EXISTS (SELECT 1 FROM user_associated_accounts WHERE user_id = $1)",
        )
        .bind(admin.id)
        .fetch_one(&mut *tx)
        .await?;
        if unported {
            return Err(Unsupported(
                "deactivating an admin with a pending review, anonymous shadows or associated accounts",
            )
            .into());
        }
        sqlx::query(
            "UPDATE users SET active = FALSE, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(admin.id)
        .execute(&mut *tx)
        .await?;
        crate::user_updater::after_save(
            &mut tx,
            &s,
            admin.id,
            false,
            &admin.username_lower,
            admin.name.as_deref(),
        )
        .await?;
        sqlx::query("UPDATE email_tokens SET confirmed = FALSE, expired = TRUE WHERE user_id = $1")
            .bind(admin.id)
            .execute(&mut *tx)
            .await?;
        // StaffActionLogger#log_user_deactivate
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, details, \
                                         admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(DEACTIVATE_USER)
        .bind(SYSTEM_USER_ID)
        .bind(admin.id)
        .bind(&reason)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// Revokes the API keys, each logged (admin only) as StaffActionLogger#log_api_key
/// logs `saved_changes`: the revoked_at change, updated_at left out.
async fn revoke_api_keys(state: &AppState, ids: Vec<i32>, context: &str) -> Result<(), AppError> {
    for id in ids {
        let mut tx = state.pool.begin().await?;
        // TimeWithZone#to_s
        let (truncated, revoked): (String, String) = sqlx::query_as(
            "UPDATE api_keys SET revoked_at = now, updated_at = now FROM clock_timestamp() AS now \
             WHERE id = $1 \
             RETURNING truncated_key, to_char(now AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS \"UTC\"')",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, subject, context, previous_value, \
                                         new_value, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'revoked_at: ', $5, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(API_KEY_UPDATE)
        .bind(SYSTEM_USER_ID)
        .bind(truncated)
        .bind(context)
        .bind(format!("revoked_at: {revoked}"))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// `ApiKey.revoke_max_life_keys!`: keys created over
/// revoke_api_keys_maxlife_days ago.
pub async fn clean_up_api_keys_max_life(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let days = s.get("revoke_api_keys_maxlife_days")?.to_i();
    if days == 0 {
        return Ok(());
    }
    let context = t_count(
        state,
        "staff_action_logs.api_key.automatic_revoked_max_life",
        days,
    );
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM api_keys WHERE revoked_at IS NULL \
           AND created_at < now() - make_interval(days => $1) ORDER BY id",
    )
    .bind(days as i32)
    .fetch_all(&state.pool)
    .await?;
    revoke_api_keys(state, ids, &context).await
}

/// `ApiKey.revoke_unused_keys!`: keys without activity (use, creation,
/// update, or the api_key_last_used_epoch) for revoke_api_keys_unused_days.
pub async fn clean_up_unused_api_keys(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let days = s.get("revoke_api_keys_unused_days")?.to_i();
    if days == 0 {
        return Ok(());
    }
    let epoch = s.get("api_key_last_used_epoch")?.to_s();
    let epoch = (!epoch.is_empty()).then_some(epoch);
    let context = t_count(state, "staff_action_logs.api_key.automatic_revoked", days);
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM api_keys WHERE revoked_at IS NULL \
           AND GREATEST(last_used_at, created_at, updated_at, $2::timestamp) \
                 < now() - make_interval(days => $1) \
         ORDER BY id",
    )
    .bind(days as i32)
    .bind(epoch)
    .fetch_all(&state.pool)
    .await?;
    revoke_api_keys(state, ids, &context).await
}

/// `UserApiKey.active.where("<column> < ?", days.ago).update_all(revoked_at:)`
/// (no updated_at).
async fn revoke_user_api_keys(state: &AppState, column: &str, days: i64) -> Result<(), AppError> {
    sqlx::query(&format!(
        "UPDATE user_api_keys SET revoked_at = now() \
         WHERE revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now()) \
           AND {column} < now() - make_interval(days => $1)"
    ))
    .bind(days as i32)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// `Jobs::CleanUpUnusedUserApiKeys`: keys unused for
/// revoke_user_api_keys_unused_days.
pub async fn clean_up_unused_user_api_keys(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let days = s.get("revoke_user_api_keys_unused_days")?.to_i();
    if days > 0 {
        revoke_user_api_keys(state, "last_used_at", days).await?;
    }
    Ok(())
}

/// `Jobs::CleanUpUserApiKeysMaxLife`: keys created over
/// revoke_user_api_keys_maxlife_days ago.
pub async fn clean_up_user_api_keys_max_life(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let days = s.get("revoke_user_api_keys_maxlife_days")?.to_i();
    if days > 0 {
        revoke_user_api_keys(state, "created_at", days).await?;
    }
    Ok(())
}

/// `Jobs::CleanUpUnusedRegisteredUserApiKeyClients`: registered clients
/// (with an auth redirect) none of whose keys were used in
/// unused_registered_user_api_key_clients_days, destroyed with their keys,
/// the keys' scopes and their own scopes. Rails' `NOT IN` is kept: one
/// recently used key without a client makes it match nothing.
pub async fn clean_up_unused_registered_user_api_key_clients(
    state: &AppState,
) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let days = s.get("unused_registered_user_api_key_clients_days")?.to_i();
    if days <= 0 {
        return Ok(());
    }
    let mut tx = state.pool.begin().await?;
    let clients: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT id FROM user_api_key_clients WHERE auth_redirect IS NOT NULL \
           AND id NOT IN (SELECT user_api_key_client_id FROM user_api_keys \
                          WHERE user_api_keys.last_used_at > now() - make_interval(days => $1))",
    )
    .bind(days as i32)
    .fetch_all(&mut *tx)
    .await?;
    for sql in [
        "DELETE FROM user_api_key_scopes WHERE user_api_key_id IN \
           (SELECT id FROM user_api_keys WHERE user_api_key_client_id = ANY($1))",
        "DELETE FROM user_api_keys WHERE user_api_key_client_id = ANY($1)",
        "DELETE FROM user_api_key_client_scopes WHERE user_api_key_client_id = ANY($1)",
        "DELETE FROM user_api_key_clients WHERE id = ANY($1)",
    ] {
        sqlx::query(sql).bind(&clients).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn delete(state: &AppState, sql: &str) -> Result<(), AppError> {
    sqlx::query(sql).execute(&state.pool).await?;
    Ok(())
}

/// `Jobs::CleanUpEmailTokens`
pub async fn clean_up_email_tokens(state: &AppState) -> Result<(), AppError> {
    delete(
        state,
        "DELETE FROM email_tokens WHERE NOT confirmed AND expired AND created_at < now() - interval '1 month'",
    )
    .await
}

/// `Jobs::CleanUpEmailLoginCodes`
pub async fn clean_up_email_login_codes(state: &AppState) -> Result<(), AppError> {
    delete(
        state,
        "DELETE FROM email_login_codes WHERE expires_at < now() - interval '1 day'",
    )
    .await
}

/// `Jobs::CleanUpEmailChangeRequests`
pub async fn clean_up_email_change_requests(state: &AppState) -> Result<(), AppError> {
    delete(
        state,
        "DELETE FROM email_change_requests WHERE updated_at < now() - interval '1 month'",
    )
    .await
}

/// `Jobs::CleanUpUnsubscribeKeys`
pub async fn clean_up_unsubscribe_keys(state: &AppState) -> Result<(), AppError> {
    delete(
        state,
        "DELETE FROM unsubscribe_keys WHERE created_at < now() - interval '2 months'",
    )
    .await
}

/// `Jobs::CleanUpPostReplyKeys`: once reply by email is disallowed for
/// them.
pub async fn clean_up_post_reply_keys(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let days = s.get("disallow_reply_by_email_after_days")?.to_i();
    if days <= 0 {
        return Ok(());
    }
    sqlx::query("DELETE FROM post_reply_keys WHERE created_at < now() - make_interval(days => $1)")
        .bind(days as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `Jobs::CleanUpEmailLogs`: sent and skipped email logs older than
/// delete_email_logs_after_days.
pub async fn clean_up_email_logs(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let days = s.get("delete_email_logs_after_days")?.to_i();
    if days <= 0 {
        return Ok(());
    }
    for table in ["email_logs", "skipped_email_logs"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE created_at < now() - make_interval(days => $1)"
        ))
        .bind(days as i32)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Email::Cleaner.delete_rejected!`: rejected incoming emails older than
/// delete_rejected_email_after_days.
pub async fn delete_rejected_emails(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    sqlx::query(
        "DELETE FROM incoming_emails WHERE rejection_message IS NOT NULL \
           AND created_at < now() - make_interval(days => $1)",
    )
    .bind(s.get("delete_rejected_email_after_days")?.to_i() as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::EnqueueDigestEmails`, every 30 minutes.
pub const ENQUEUE_DIGEST_EMAILS: &str = "enqueue_digest_emails";

/// `GlobalSetting.max_digests_enqueued_per_30_mins_per_site`'s default.
const MAX_DIGESTS_ENQUEUED: i64 = 10_000;

/// Jobs::EnqueueDigestEmails: a digest user_email for each real, active,
/// unstaged, unsuspended user with a primary address whose digests are on
/// and due (neither tried nor seen within their frequency, seen within
/// suppress_digest_email_after_days), the longest waiting first, up to the
/// global limit.
pub async fn enqueue_digest_emails(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if s.get("disable_emails")?.to_s() == "yes"
        || s.get("disable_digest_emails")?.truthy()
        || s.get("private_email")?.truthy()
    {
        return Ok(());
    }
    let limit = match state
        .config
        .globals
        .get("max_digests_enqueued_per_30_mins_per_site")
    {
        Some(v) => crate::ruby::to_i(v),
        None => MAX_DIGESTS_ENQUEUED,
    };
    let approval = if s.get("must_approve_users")?.truthy() {
        " AND (approved OR moderator OR admin)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT users.id FROM users \
         INNER JOIN user_options ON user_options.user_id = users.id \
         INNER JOIN user_stats ON user_stats.user_id = users.id \
         INNER JOIN user_emails ON user_emails.user_id = users.id \
         WHERE users.id > 0 \
           AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
           AND users.active = TRUE AND users.staged = FALSE \
           AND (users.suspended_till IS NULL OR users.suspended_till <= $1) \
           AND user_options.email_digests \
           AND COALESCE(user_options.digest_after_minutes, $2) > 0 \
           AND user_stats.bounce_score < $3 \
           AND user_emails.primary \
           AND COALESCE(user_stats.digest_attempted_at, '2010-01-01') <= CURRENT_TIMESTAMP - ('1 MINUTE'::INTERVAL * COALESCE(user_options.digest_after_minutes, $2)) \
           AND COALESCE(last_seen_at, '2010-01-01') <= CURRENT_TIMESTAMP - ('1 MINUTE'::INTERVAL * COALESCE(user_options.digest_after_minutes, $2)) \
           AND COALESCE(last_seen_at, '2010-01-01') >= CURRENT_TIMESTAMP - ('1 DAY'::INTERVAL * $4){approval} \
         ORDER BY user_stats.digest_attempted_at ASC NULLS FIRST \
         LIMIT $5"
    );
    let ids: Vec<i32> = sqlx::query_scalar(&sql)
        .bind(crate::clock::now_naive())
        .bind(s.get("default_email_digest_frequency")?.to_i() as i32)
        .bind(s.get("bounce_score_threshold")?.to_f())
        .bind(s.get("suppress_digest_email_after_days")?.to_i() as i32)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    for user_id in ids {
        crate::jobs::enqueue(
            &mut tx,
            "user_email",
            json!({"type": "digest", "user_id": user_id}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
