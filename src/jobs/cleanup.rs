//! Scheduled jobs that clean up what piles up: dismissals past the new
//! window, stale and old drafts, bookmarks on long-deleted posts, rotated
//! auth tokens, old search logs, unused tags, and screened emails and IPs
//! that stopped matching.

use serde_json::json;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `Jobs::CleanDismissedTopicUsers`, daily.
pub const CLEAN_DISMISSED_TOPIC_USERS: &str = "clean_dismissed_topic_users";
/// `Jobs::CleanUpDrafts`, weekly.
pub const CLEAN_UP_DRAFTS: &str = "clean_up_drafts";
/// `Jobs::CleanUpBookmarks`, weekly.
pub const CLEAN_UP_BOOKMARKS: &str = "clean_up_bookmarks";
/// `Jobs::CleanUpUserAuthTokens`, weekly.
pub const CLEAN_UP_USER_AUTH_TOKENS: &str = "clean_up_user_auth_tokens";
/// `Jobs::CleanUpSearchLogs`, weekly.
pub const CLEAN_UP_SEARCH_LOGS: &str = "clean_up_search_logs";
/// `Jobs::CleanUpTags`, daily.
pub const CLEAN_UP_TAGS: &str = "clean_up_tags";
/// `Jobs::CleanUpUnmatchedEmails`, daily.
pub const CLEAN_UP_UNMATCHED_EMAILS: &str = "clean_up_unmatched_emails";
/// `Jobs::CleanUpUnmatchedIPs`, daily (named by class: zeitwerk's IP
/// acronym does not camelize back from the file name).
pub const CLEAN_UP_UNMATCHED_IPS: &str = "Jobs::CleanUpUnmatchedIPs";

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;
/// `UserHistory.actions`
const CUSTOM_STAFF: i32 = 23;
const ROLL_UP: i32 = 20;
/// `ScreeningModel.actions[:block]`
const BLOCK: i32 = 1;
/// `User::NewTopicDuration`
const ALWAYS: i32 = -1;
const LAST_VISIT: i32 = -2;
/// `UserAuthToken::ROTATE_TIME`, in minutes.
const ROTATE_TIME_MINS: i32 = 10;
/// `CleanUpTags::GRACE_PERIOD_MINUTES`
const TAG_GRACE_PERIOD_MINUTES: i32 = 5;
/// `UserHistory::MAX_CONTEXT_LENGTH`
const MAX_CONTEXT_LENGTH: usize = 50_000;

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::CleanDismissedTopicUsers`: dismissals of topics older than the
/// user's new-topic window, then each user's dismissals beyond the
/// max_new_topics newest.
pub async fn clean_dismissed_topic_users(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    sqlx::query(
        "DELETE FROM dismissed_topic_users dtu1 \
         USING dismissed_topic_users dtu2 \
         JOIN topics ON topics.id = dtu2.topic_id \
         JOIN users ON users.id = dtu2.user_id \
         JOIN categories ON categories.id = topics.category_id \
         LEFT JOIN user_stats ON user_stats.user_id = users.id \
         LEFT JOIN user_options ON user_options.user_id = users.id \
         WHERE topics.created_at < GREATEST(CASE \
                 WHEN COALESCE(user_options.new_topic_duration_minutes, $1) = $2 THEN users.created_at \
                 WHEN COALESCE(user_options.new_topic_duration_minutes, $1) = $3 \
                   THEN COALESCE(users.previous_visit_at, users.created_at) \
                 ELSE (now()::timestamp - INTERVAL '1 MINUTE' * COALESCE(user_options.new_topic_duration_minutes, $1)) \
               END, users.created_at, to_timestamp($4)::timestamp) \
           AND dtu1.id = dtu2.id",
    )
    .bind(s.get("default_other_new_topic_duration_minutes")?.to_i() as i32)
    .bind(ALWAYS)
    .bind(LAST_VISIT)
    .bind(s.get("min_new_topics_time")?.to_i() as f64)
    .execute(&mut *conn)
    .await?;
    let user_ids: Vec<i32> =
        sqlx::query_scalar("SELECT DISTINCT user_id FROM dismissed_topic_users")
            .fetch_all(&mut *conn)
            .await?;
    // NOT IN over a list holding NULL (a user without dismissals left)
    // deletes nothing, as in Rails.
    sqlx::query(
        "DELETE FROM dismissed_topic_users \
         WHERE dismissed_topic_users.id NOT IN ( \
           SELECT valid_dtu.id FROM users \
           LEFT JOIN dismissed_topic_users valid_dtu ON valid_dtu.user_id = users.id \
           AND valid_dtu.topic_id IN ( \
             SELECT topic_id FROM dismissed_topic_users dtu2 \
             JOIN topics ON topics.id = dtu2.topic_id \
             WHERE dtu2.user_id = users.id \
             ORDER BY topics.created_at DESC \
             LIMIT $1) \
           WHERE users.id = ANY($2))",
    )
    .bind(s.get("max_new_topics")?.to_i())
    .bind(&user_ids)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Draft.cleanup!`: drafts behind their key's sequence, then drafts not
/// updated in delete_drafts_older_than_n_days, each destroyed (its upload
/// references with it, the owner's count updated and published on
/// commit); then every user's count.
pub async fn clean_up_drafts(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let stale: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT id, user_id FROM drafts WHERE sequence < ( \
           SELECT MAX(s.sequence) FROM draft_sequences s \
           WHERE s.draft_key = drafts.draft_key AND s.user_id = drafts.user_id) \
         ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await?;
    destroy_drafts(state, &mut conn, stale).await?;
    let old: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT id, user_id FROM drafts WHERE updated_at < now() - make_interval(days => $1) ORDER BY id",
    )
    .bind(s.get("delete_drafts_older_than_n_days")?.to_i() as i32)
    .fetch_all(&mut *conn)
    .await?;
    destroy_drafts(state, &mut conn, old).await?;
    sqlx::query(
        "UPDATE user_stats SET draft_count = new_user_stats.draft_count \
         FROM (SELECT user_stats.user_id, COUNT(drafts.id) draft_count \
               FROM user_stats LEFT JOIN drafts ON user_stats.user_id = drafts.user_id \
               GROUP BY user_stats.user_id) new_user_stats \
         WHERE user_stats.user_id = new_user_stats.user_id \
           AND user_stats.draft_count <> new_user_stats.draft_count",
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn destroy_drafts(
    state: &AppState,
    conn: &mut PgConnection,
    drafts: Vec<(i32, i32)>,
) -> Result<(), AppError> {
    for (id, user_id) in drafts {
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        sqlx::query("DELETE FROM upload_references WHERE target_type = 'Draft' AND target_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM drafts WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // UserStat.update_draft_count(user_id)
        let count: Option<i32> = sqlx::query_scalar(
            "UPDATE user_stats SET draft_count = (SELECT COUNT(*) FROM drafts WHERE user_id = $1) \
             WHERE user_id = $1 RETURNING draft_count",
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?;
        state
            .bus
            .publish(
                &mut tx,
                &format!("/user-drafts/{user_id}"),
                &json!({ "draft_count": count }),
                Some(&[crate::bus::user_tag(user_id)]),
            )
            .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// `Bookmark.cleanup!`: each bookmarkable's cleanup_deleted, in
/// registration order: bookmarks on posts and topics deleted over three
/// days ago (their topics' bookmarked flags resynced), then on chat
/// messages.
pub async fn clean_up_bookmarks(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let post_topics: Vec<i32> = sqlx::query_scalar(
        "DELETE FROM bookmarks b USING topics t, posts p \
         WHERE t.id = p.topic_id AND b.bookmarkable_id = p.id AND b.bookmarkable_type = 'Post' \
           AND (t.deleted_at < now() - interval '3 days' OR p.deleted_at < now() - interval '3 days') \
         RETURNING t.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    let topic_topics: Vec<i32> = sqlx::query_scalar(
        "DELETE FROM bookmarks b USING topics t \
         WHERE b.bookmarkable_id = t.id AND b.bookmarkable_type = 'Topic' \
           AND (t.deleted_at < now() - interval '3 days') \
         RETURNING t.id",
    )
    .fetch_all(&mut *tx)
    .await?;
    for ids in [post_topics, topic_topics] {
        // related_topics.map(&:topic_id).uniq: first appearance order.
        let mut seen = std::collections::HashSet::new();
        for topic_id in ids.into_iter().filter(|id| seen.insert(*id)) {
            super::enqueue(
                &mut tx,
                "sync_topic_user_bookmarked",
                json!({ "topic_id": topic_id }),
            )
            .await?;
        }
    }
    sqlx::query(
        "DELETE FROM bookmarks b USING chat_messages cm \
         WHERE b.bookmarkable_id = cm.id AND b.bookmarkable_type = 'ChatMessage' \
           AND (cm.deleted_at < now() - interval '3 days')",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `UserAuthToken.cleanup!`: logs (but suspicious and generate ones) and
/// tokens rotated before maximum_session_age plus the rotate time.
pub async fn clean_up_user_auth_tokens(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let hours = s.get("maximum_session_age")?.to_i() as i32;
    sqlx::query(
        "DELETE FROM user_auth_token_logs \
         WHERE created_at < now() - make_interval(hours => $1, mins => $2) \
           AND action NOT IN ('suspicious', 'generate')",
    )
    .bind(hours)
    .bind(ROTATE_TIME_MINS)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM user_auth_tokens WHERE rotated_at < now() - make_interval(hours => $1, mins => $2)",
    )
    .bind(hours)
    .bind(ROTATE_TIME_MINS)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `SearchLog.clean_up`: logs beyond search_query_log_max_size (the
/// oldest by id), then those past search_query_log_max_retention_days.
pub async fn clean_up_search_logs(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let cutoff: Option<i32> =
        sqlx::query_scalar("SELECT id FROM search_logs ORDER BY id OFFSET $1 LIMIT 1")
            .bind(s.get("search_query_log_max_size")?.to_i())
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(cutoff) = cutoff {
        sqlx::query("DELETE FROM search_logs WHERE id < $1")
            .bind(cutoff)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("DELETE FROM search_logs WHERE created_at < now() - make_interval(days => $1)")
        .bind(s.get("search_query_log_max_retention_days")?.to_i() as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Ruby's `String#inspect` for a tag name (tag names hold no control
/// characters).
fn inspect(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace("#{", "\\#{")
    )
}

/// `Jobs::CleanUpTags` (when automatically_clean_unused_tags): base tags
/// without staff or PM topics and outside tag groups, created over five
/// minutes ago, logged (even when none) and destroyed.
pub async fn clean_up_tags(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("automatically_clean_unused_tags")?.truthy() {
        return Ok(());
    }
    let tags: Vec<(i32, String)> = sqlx::query_as(
        "SELECT tags.id, tags.name FROM tags \
         LEFT JOIN tag_group_memberships tgm ON tags.id = tgm.tag_id \
         WHERE tags.target_tag_id IS NULL AND tags.staff_topic_count = 0 AND tags.pm_topic_count = 0 \
           AND tgm.tag_id IS NULL AND tags.created_at < now() - make_interval(mins => $1) \
         ORDER BY tags.name",
    )
    .bind(TAG_GRACE_PERIOD_MINUTES)
    .fetch_all(&mut *tx)
    .await?;
    // StaffActionLogger#log_custom("deleted_unused_tags", tags: [...])
    let list = tags
        .iter()
        .map(|(_, name)| inspect(name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut value = format!("[{list}]");
    if value.chars().count() > MAX_CONTEXT_LENGTH {
        value = format!(
            "{}...",
            value
                .chars()
                .take(MAX_CONTEXT_LENGTH + 1)
                .collect::<String>()
        );
    }
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, custom_type, details, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, 'deleted_unused_tags', $3, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(CUSTOM_STAFF)
    .bind(SYSTEM_USER_ID)
    .bind(format!("tags: {value}"))
    .execute(&mut *tx)
    .await?;
    let ids: Vec<i32> = tags.iter().map(|(id, _)| *id).collect();
    // The dependents destroy_all takes with each tag. Topic tags on live
    // topics would move counters (TopicTag's after_destroy), and synonyms
    // are tags with dependents of their own.
    let unported: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_tags tt JOIN topics t ON t.id = tt.topic_id \
                        WHERE tt.tag_id = ANY($1) AND t.deleted_at IS NULL) \
             OR EXISTS (SELECT 1 FROM tags WHERE target_tag_id = ANY($1))",
    )
    .bind(&ids)
    .fetch_one(&mut *tx)
    .await?;
    if unported {
        return Err(Unsupported("destroying unused tags with live topic tags or synonyms").into());
    }
    // HasDestroyedWebHook: tag_destroyed.
    crate::web_hooks::refuse(&mut tx, "tag_destroyed", "tag_destroyed web hooks").await?;
    for (table, column) in [
        ("tag_users", "tag_id"),
        ("topic_tags", "tag_id"),
        ("category_tag_stats", "tag_id"),
        ("category_tags", "tag_id"),
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE {column} = ANY($1)"))
            .bind(&ids)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "DELETE FROM sidebar_section_links WHERE linkable_type = 'Tag' AND linkable_id = ANY($1)",
    )
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM tags WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::CleanUpUnmatchedEmails`: blocked emails not matched in
/// max_age_unmatched_emails days.
pub async fn clean_up_unmatched_emails(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    sqlx::query(
        "DELETE FROM screened_emails WHERE action_type = $1 \
           AND last_match_at < now() - make_interval(days => $2)",
    )
    .bind(BLOCK)
    .bind(s.get("max_age_unmatched_emails")?.to_i() as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `ScreenedIpAddress::ROLLED_UP_BLOCKS`: family, the mask lengths rolled
/// up, and the subnet's.
const ROLLED_UP_BLOCKS: [(i32, std::ops::RangeInclusive<i32>, i32); 6] = [
    (4, 32..=32, 24),
    (6, 65..=128, 64),
    (6, 64..=64, 60),
    (6, 60..=60, 56),
    (6, 56..=56, 52),
    (6, 52..=52, 48),
];

/// `Jobs::CleanUpUnmatchedIPs`: ScreenedIpAddress.roll_up, then blocked
/// IPs not matched (or, never matched, not created) in
/// max_age_unmatched_ips days.
pub async fn clean_up_unmatched_ips(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let min_entries = s.get("min_ban_entries_for_roll_up")?.to_i();
    for (family, from, to) in ROLLED_UP_BLOCKS {
        let from: Vec<i32> = from.collect();
        // ScreenedIpAddress.subnets
        let subnets: Vec<String> = sqlx::query_scalar(
            "WITH ips_and_subnets AS ( \
               SELECT ip_address, network(inet(host(ip_address) || '/' || $3))::text subnet \
               FROM screened_ip_addresses \
               WHERE family(ip_address) = $1 AND masklen(ip_address) = ANY($2) AND action_type = $4) \
             SELECT subnet FROM ips_and_subnets GROUP BY subnet HAVING COUNT(*) >= $5",
        )
        .bind(family)
        .bind(&from)
        .bind(to)
        .bind(BLOCK)
        .bind(min_entries)
        .fetch_all(&mut *conn)
        .await?;
        for subnet in subnets {
            let mut tx = sqlx::Connection::begin(&mut *conn).await?;
            let covered: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM screened_ip_addresses WHERE $1::inet <<= ip_address)",
            )
            .bind(&subnet)
            .fetch_one(&mut *tx)
            .await?;
            if covered {
                continue;
            }
            let old: Vec<(i32, String)> = sqlx::query_as(
                "SELECT id, host(network(ip_address)) FROM screened_ip_addresses \
                 WHERE action_type = $1 AND ip_address << $2::inet \
                   AND family(ip_address) = $3 AND masklen(ip_address) = ANY($4) \
                 ORDER BY ip_address",
            )
            .bind(BLOCK)
            .bind(&subnet)
            .bind(family)
            .bind(&from)
            .fetch_all(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO screened_ip_addresses (ip_address, action_type, match_count, \
                                                    last_match_at, created_at, updated_at) \
                 SELECT $1::inet, $2, SUM(match_count), MAX(last_match_at), MIN(created_at), \
                        clock_timestamp() \
                 FROM screened_ip_addresses WHERE id = ANY($3)",
            )
            .bind(&subnet)
            .bind(BLOCK)
            .bind(old.iter().map(|(id, _)| *id).collect::<Vec<_>>())
            .execute(&mut *tx)
            .await?;
            // StaffActionLogger#log_roll_up (admin only), each IPAddr
            // printed as its network address, in the unique index's order.
            let ips = old
                .iter()
                .map(|(_, ip)| ip.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            sqlx::query(
                "INSERT INTO user_histories (action, acting_user_id, details, admin_only, \
                                             created_at, updated_at) \
                 VALUES ($1, $2, $3, TRUE, clock_timestamp(), clock_timestamp())",
            )
            .bind(ROLL_UP)
            .bind(SYSTEM_USER_ID)
            .bind(format!("{subnet} from {ips}"))
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM screened_ip_addresses WHERE id = ANY($1)")
                .bind(old.iter().map(|(id, _)| *id).collect::<Vec<_>>())
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
    }
    sqlx::query(
        "DELETE FROM screened_ip_addresses WHERE action_type = $1 \
           AND (last_match_at < now() - make_interval(days => $2) \
                OR (last_match_at IS NULL AND created_at < now() - make_interval(days => $2)))",
    )
    .bind(BLOCK)
    .bind(s.get("max_age_unmatched_ips")?.to_i() as i32)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
