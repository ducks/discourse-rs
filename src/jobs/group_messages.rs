//! The scheduled jobs that message the moderators group: the summary of a
//! user many others ignore, and the reminder of a waiting review queue.

use crate::group_message::GroupMessage;
use crate::guardian::Guardian;
use crate::posting::create::{self, NewPost};
use crate::session::current::{SESSION_USER_COLUMNS, SessionUser};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::IgnoredUsersSummary`, daily.
pub const IGNORED_USERS_SUMMARY: &str = "ignored_users_summary";
/// `Jobs::PendingReviewablesReminder`, every 15 minutes.
pub const PENDING_REVIEWABLES_REMINDER: &str = "pending_reviewables_reminder";

/// `PendingReviewablesReminder.last_notified_key`, set without expiry.
const LAST_NOTIFIED_KEY: &str = "last_notified_reviewable_id";
/// A redis SET without a TTL, as a cached value: a century.
const NO_EXPIRY_SECONDS: i64 = 100 * 365 * 24 * 3600;

async fn settings(state: &AppState) -> Result<SiteSettings, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Group[:moderators].name`
async fn moderators_name(state: &AppState) -> Result<String, AppError> {
    Ok(sqlx::query_scalar("SELECT name FROM groups WHERE id = 2")
        .fetch_one(&state.pool)
        .await?)
}

/// `Jobs::IgnoredUsersSummary`: each user ignored by at least
/// ignored_users_count_message_threshold users, not summarized within
/// ignored_users_message_gap_days, gets a message to the moderators
/// (PostCreator without validations), and their ignores are stamped.
pub async fn ignored_users_summary(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let threshold = s.get("ignored_users_count_message_threshold")?.to_i();
    let gap = s.get("ignored_users_message_gap_days")?.to_i();
    // DB.query_single interpolates the integers into the quoted literals.
    let user_ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT ignored_user_id FROM ignored_users \
         WHERE COALESCE(summarized_at, CURRENT_TIMESTAMP + '{} DAYS'::INTERVAL) - '{gap} DAYS'::INTERVAL > CURRENT_TIMESTAMP \
         GROUP BY ignored_user_id HAVING COUNT(ignored_user_id) >= $1",
        gap + 1
    ))
    .bind(threshold)
    .fetch_all(&state.pool)
    .await?;
    let mut users: Vec<i32> = sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1)")
        .bind(&user_ids)
        .fetch_all(&state.pool)
        .await?;
    users.sort_unstable();
    let moderators = moderators_name(state).await?;
    for user_id in users {
        let mut params =
            crate::system_message::defaults(&state.pool, &ctx, user_id, "ignored_users_summary")
                .await?;
        params.push(("ignores_threshold".to_string(), threshold.to_string()));
        let args: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let t = |key: &str, args: &[(&str, &str)]| {
            state
                .i18n
                .t_with(
                    &format!("system_messages.ignored_users_summary.{key}"),
                    args,
                )
                .ok_or(crate::Unsupported("system messages without a translation"))
        };
        let title = t("subject_template", &[])?;
        let raw = t("text_body_template", &args)?;
        let mut conn = state.pool.acquire().await?;
        let system: SessionUser = sqlx::query_as(&format!(
            "SELECT {SESSION_USER_COLUMNS} FROM users WHERE users.id = -1"
        ))
        .fetch_one(&mut *conn)
        .await?;
        let guardian = Guardian::for_user(&mut conn, &system).await?;
        drop(conn);
        // PostCreator.create: a refusal is not raised.
        create::create(
            &state.pool,
            &ctx,
            &guardian,
            NewPost {
                raw,
                title: Some(title),
                pm_recipients: Some(Vec::new()),
                pm_groups: vec![moderators.to_lowercase()],
                subtype: Some("system_message".to_string()),
                skip_validations: true,
                ..Default::default()
            },
        )
        .await?;
        sqlx::query("UPDATE ignored_users SET summarized_at = $2 WHERE ignored_user_id = $1")
            .bind(user_id)
            .bind(crate::clock::now_naive())
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

/// `Jobs::PendingReviewablesReminder`: with notify_about_reviewable_item_after
/// set, once a pending reviewable above the priority threshold has waited
/// that many hours and is newer than the last one reminded of, the
/// moderators get a reminder mentioning up to three moderators (the
/// earlier reminder replaced), and the newest id is remembered.
pub async fn pending_reviewables_reminder(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let after = s.get("notify_about_reviewable_item_after")?;
    if after.to_f() <= 0.0 {
        return Ok(());
    }
    let mut conn = state.pool.acquire().await?;
    let min_score = crate::reviewables::min_score_for_priority(&mut conn, &s).await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM reviewables WHERE status = 0 AND score >= $1 \
         AND latest_score < now() - make_interval(secs => $2) ORDER BY id DESC",
    )
    .bind(min_score)
    .bind(after.to_f() * 3600.0)
    .fetch_all(&mut *conn)
    .await?;
    let Some(newest) = ids.first().copied() else {
        return Ok(());
    };
    let last = crate::owned_schema::cached_get(&mut conn, LAST_NOTIFIED_KEY)
        .await?
        .map_or(0, |v| crate::ruby::to_i(&v));
    if last >= newest {
        return Ok(());
    }
    let usernames: Vec<String> = sqlx::query_scalar(
        "SELECT username FROM users WHERE moderator AND id > 0 ORDER BY last_seen_at DESC LIMIT 3",
    )
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    let mentions = if usernames.is_empty() {
        String::new()
    } else {
        format!("@{} ", usernames.join(", @"))
    };
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let moderators = moderators_name(state).await?;
    let message = GroupMessage {
        group_name: &moderators,
        message_type: "reviewables_reminder",
        limit_once_per: None,
        message_params: vec![
            ("mentions".to_string(), mentions),
            ("count".to_string(), after.to_s()),
        ],
    };
    message.delete_previous(&state.pool, &ctx, false).await?;
    let created = message.create(&state.pool, &ctx).await?;
    let mut conn = state.pool.acquire().await?;
    if let Some(post_id) = created {
        // Both run in a Topic.transaction, so TopicLink's after_commit
        // (crawl_topic_link) waits for its commit, past the jobs
        // PostCreator enqueued.
        let crawls: Vec<(i64, serde_json::Value)> = sqlx::query_as(
            "SELECT id, args FROM discourse_rs.jobs WHERE name = 'crawl_topic_link' \
             AND (args->>'topic_link_id')::int IN (SELECT id FROM topic_links WHERE post_id = $1) \
             ORDER BY id",
        )
        .bind(post_id)
        .fetch_all(&mut *conn)
        .await?;
        let ids: Vec<i64> = crawls.iter().map(|(id, _)| *id).collect();
        sqlx::query("DELETE FROM discourse_rs.jobs WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&mut *conn)
            .await?;
        for (_, args) in crawls {
            super::enqueue(&mut conn, "crawl_topic_link", args).await?;
        }
    }
    crate::owned_schema::cached_setex(
        &mut conn,
        LAST_NOTIFIED_KEY,
        NO_EXPIRY_SECONDS,
        &newest.to_string(),
    )
    .await?;
    Ok(())
}
