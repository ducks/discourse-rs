//! The jobs: `Jobs::<Name>#execute(args)` ports, by name.

use serde_json::{Value, json};
use sqlx::PgConnection;

use super::{Job, JobError};
use crate::posting::links::{self, LinkPost};
use crate::pretty_text::cooked_post_processor;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// Runs a job by its Sidekiq name.
pub async fn run(state: &AppState, job: &Job) -> Result<(), JobError> {
    let result = match job.name.as_str() {
        "post_update_topic_tracking_state" => {
            post_update_topic_tracking_state(state, &job.args).await
        }
        // Publishes reviewable counts to staff on MessageBus, likewise.
        "notify_reviewable" => Ok(()),
        // Publishes each staff member's reviewable counts, likewise.
        "refresh_users_reviewable_counts" => Ok(()),
        "cleanup_acls_for_deleted" => cleanup_acls_for_deleted(state, &job.args).await,
        crate::plugins::chat::membership::DELETE_USER_MESSAGES_JOB => {
            chat_delete_user_messages(state, &job.args).await
        }
        crate::user_merger::MERGE_USER => {
            crate::user_merger::merge_user_job(state, &job.args).await
        }
        crate::user_anonymizer::ANONYMIZE_USER => {
            crate::user_anonymizer::anonymize_user_job(state, &job.args).await
        }
        crate::username_changer::UPDATE_USERNAME => {
            crate::username_changer::run_update_username(state, &job.args).await
        }
        "feature_topic_users" => feature_topic_users(state, &job.args).await,
        "sync_topic_user_bookmarked" => sync_topic_user_bookmarked(state, &job.args).await,
        "process_post" => process_post(state, &job.args).await,
        super::badges::BACKFILL_BADGE => super::badges::backfill_badge(state, &job.args).await,
        "post_alert" => post_alert(state, &job.args).await,
        "group_pm_update_summary" => group_pm_update_summary(state, &job.args).await,
        "group_pm_alert" => group_pm_alert(state, &job.args).await,
        "close_poll" => close_poll(state, &job.args).await,
        "delete_inaccessible_notifications" => {
            delete_inaccessible_notifications(state, &job.args).await
        }
        "user_email" => user_email(state, &job.args, false).await,
        "critical_user_email" => user_email(state, &job.args, true).await,
        "send_system_message" => send_system_message(state, &job.args).await,
        "send_email_login_code" => send_email_login_code(state, &job.args).await,
        // Fires user_added_to_group / user_removed_from_group, which core
        // only hands to web hooks (none are ported).
        "publish_group_membership_updates" => {
            let hooks: bool =
                match sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM web_hooks WHERE active)")
                    .fetch_one(&state.pool)
                    .await
                {
                    Ok(hooks) => hooks,
                    Err(e) => return Err(AppError::from(e).into()),
                };
            if hooks {
                Err(Unsupported("web hooks for group membership").into())
            } else {
                Ok(())
            }
        }
        // UserAuthToken.is_suspicious compares the login's location with the
        // user's earlier ones, through MaxMind's databases; without them
        // (no license key to download them) no login is suspicious.
        "suspicious_login" => {
            if std::env::var("DISCOURSE_MAXMIND_LICENSE_KEY").is_ok_and(|k| !k.is_empty()) {
                Err(Unsupported("suspicious logins by location (MaxMind)").into())
            } else {
                Ok(())
            }
        }
        "process_email" => process_email(state, &job.args).await,
        "Jobs::DiscourseTopicVoting::BackfillBadges" => {
            topic_voting_backfill_badges(state, &job.args).await
        }
        "Jobs::DiscourseTopicVoting::VoteRelease" => {
            topic_voting_vote_release(state, &job.args).await
        }
        crate::plugins::chat::auto_join::UPDATE_USER_COUNT_JOB => {
            chat_update_channel_user_count(state, &job.args).await
        }
        crate::plugins::chat::create::PROCESS_MESSAGE_JOB => {
            chat_process_message(state, &job.args).await
        }
        crate::plugins::chat::create::UPDATE_THREAD_REPLY_COUNT_JOB => {
            chat_update_thread_reply_count(state, &job.args).await
        }
        crate::plugins::chat::notify::NOTIFY_MENTIONED_JOB => {
            chat_notify_mentioned(state, &job.args).await
        }
        crate::plugins::chat::notify::NOTIFY_WATCHING_JOB => {
            chat_notify_watching(state, &job.args).await
        }
        crate::plugins::chat::summary_email::EMAIL_NOTIFICATIONS_JOB => {
            crate::plugins::chat::summary_email::send_unread_mentions_summary(state).await
        }
        crate::plugins::chat::scheduled::AUTO_JOIN_USERS => {
            crate::plugins::chat::scheduled::auto_join_users(state, &job.args).await
        }
        crate::plugins::chat::scheduled::KICK_USERS_FROM_CHANNEL => {
            crate::plugins::chat::scheduled::kick_users_from_channel(state, &job.args).await
        }
        name if super::topic_timers::JOBS.contains(&name) => {
            super::topic_timers::run(state, name, &job.args).await
        }
        name if super::is_scheduled(name) => super::run_scheduled(state, name).await,
        "Jobs::DiscourseTopicVoting::VoteReclaim" => {
            let mut tx = match state.pool.begin().await {
                Ok(tx) => tx,
                Err(e) => return Err(AppError::from(e).into()),
            };
            match crate::plugins::topic_voting::lifecycle::vote_reclaim(&mut tx, &job.args).await {
                Ok(()) => tx.commit().await.map_err(AppError::from),
                Err(e) => Err(e),
            }
        }
        other => {
            return Err(JobError::Unported(format!(
                "not ported yet: the {other} job"
            )));
        }
    };
    Ok(result?)
}

/// Jobs::CleanupAclsForDeleted: a deleted group or user taken out of the
/// access control lists, the lists left with no one deleted.
async fn cleanup_acls_for_deleted(state: &AppState, args: &Value) -> Result<(), AppError> {
    let group_id = int_arg(args, "group_id")?;
    let user_id = int_arg(args, "user_id")?;
    if group_id.is_none() && user_id.is_none() {
        return Ok(());
    }
    let mut tx = state.pool.begin().await?;
    for (column, id) in [
        ("allowed_group_ids", group_id),
        ("allowed_user_ids", user_id),
    ] {
        if let Some(id) = id {
            sqlx::query(&format!(
                "UPDATE access_control_lists SET {column} = array_remove({column}, $1::bigint), \
                                                 updated_at = clock_timestamp() \
                 WHERE {column} @> ARRAY[$1::bigint]"
            ))
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query(
        "DELETE FROM access_control_lists \
         WHERE allowed_group_ids = '{}'::bigint[] AND allowed_user_ids = '{}'::bigint[]",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Jobs::Chat::DeleteUserMessages
async fn chat_delete_user_messages(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(user_id) = int_arg(args, "user_id")? else {
        return Ok(());
    };
    crate::plugins::chat::scheduled::delete_user_messages(state, user_id).await
}

/// Jobs::DiscourseTopicVoting::VoteRelease
async fn topic_voting_vote_release(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    crate::plugins::topic_voting::lifecycle::vote_release(&mut tx, &state.bus, &s, args).await?;
    tx.commit().await?;
    Ok(())
}

/// Jobs::Chat::UpdateChannelUserCount
async fn chat_update_channel_user_count(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    crate::plugins::chat::auto_join::update_user_count(&mut tx, &state.bus, &s, args).await?;
    tx.commit().await?;
    Ok(())
}

/// Jobs::DiscourseTopicVoting::BackfillBadges: the voting badges
/// backfilled for the topic's first post.
async fn topic_voting_backfill_badges(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = int_arg(args, "topic_id")? else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("enable_badges")?.truthy() {
        return Ok(());
    }
    let first_post: Option<i32> =
        sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1")
            .bind(topic_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(first_post) = first_post else {
        return Ok(());
    };
    let badges = crate::badge_granter::Badge::enabled_named(
        &mut tx,
        &crate::plugins::topic_voting::BADGE_NAMES,
    )
    .await?;
    for badge in &badges {
        crate::badge_granter::backfill(
            &mut tx,
            &state.bus,
            &s,
            &state.i18n,
            badge,
            Some(crate::badge_granter::Scope::Posts(&[first_post])),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn int_arg(args: &Value, key: &str) -> Result<Option<i32>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v
            .as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        {
            Some(n) => Ok(Some(n as i32)),
            None => Err(Unsupported("job arguments that are not ids").into()),
        },
    }
}

fn bool_arg(args: &Value, key: &str) -> bool {
    matches!(args.get(key), Some(Value::Bool(true)))
}

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::PostUpdateTopicTrackingState`: a regular topic's lists and
/// counts told about a post, in one transaction. Messages
/// (PrivateMessageTopicTrackingState, TopicGroup.new_message_update) are
/// not ported and left alone, as before.
async fn post_update_topic_tracking_state(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(post_id) = args.get("post_id").and_then(Value::as_i64) else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let post: Option<(i32, i32, i32, String)> = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, p.post_type, t.archetype FROM posts p \
         JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id as i32)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((topic_id, post_number, post_type, archetype)) = post else {
        return Ok(());
    };
    if post_type == crate::posting::post_types::SMALL_ACTION {
        return Ok(());
    }
    let s = settings(state, &mut tx).await?;
    if archetype == "private_message" {
        if post_number > 1 {
            crate::pm_tracking_state::publish_unread(&state.bus, &s, &mut tx, post_id as i32)
                .await?;
        }
        // TopicGroup.new_message_update for the topic's last poster.
        let last_poster: Option<i32> =
            sqlx::query_scalar("SELECT last_post_user_id FROM topics WHERE id = $1")
                .bind(topic_id)
                .fetch_one(&mut *tx)
                .await?;
        if let Some(last_poster) = last_poster {
            crate::posting::group_messages::new_message_update(
                &state.bus,
                &mut tx,
                last_poster,
                topic_id,
                post_number,
            )
            .await?;
        }
        tx.commit().await?;
        return Ok(());
    }
    use crate::topic_tracking_state as tracking;
    tracking::publish_unmuted(&state.bus, &mut tx, topic_id).await?;
    if post_number > 1 {
        tracking::publish_muted(&state.bus, &mut tx, topic_id).await?;
        tracking::publish_unread(&state.bus, &s, &mut tx, post_id as i32).await?;
    }
    if post_type != crate::posting::post_types::WHISPER {
        tracking::publish_latest(&state.bus, &s, &mut tx, topic_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::FeatureTopicUsers` -> `TopicFeaturedUsers#choose`: the four
/// featured posters and the participant count.
async fn feature_topic_users(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = int_arg(args, "topic_id")? else {
        return Err(
            Unsupported("feature_topic_users without a topic_id (InvalidParameters)").into(),
        );
    };
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1)")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    choose_featured_users(&mut conn, topic_id).await
}

/// `Jobs::SyncTopicUserBookmarked`: each topic user's bookmarked flag
/// from their live bookmarks in the topic. Flags are cleared only when
/// someone still has one there: the clearing update joins the counts.
async fn sync_topic_user_bookmarked(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = int_arg(args, "topic_id")? else {
        return Err(Unsupported(
            "sync_topic_user_bookmarked without a topic_id (InvalidParameters)",
        )
        .into());
    };
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "CREATE TEMP TABLE tmp_sync_topic_user_bookmarks ON COMMIT DROP AS \
         SELECT bookmarks.user_id, COUNT(*) AS count FROM bookmarks \
         LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
         LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
           OR (topics.id = posts.topic_id) \
         WHERE (topics.id = $1 OR posts.topic_id = $1) \
           AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
         GROUP BY bookmarks.user_id",
    )
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE topic_users SET bookmarked = true FROM tmp_sync_topic_user_bookmarks t \
         WHERE topic_users.user_id = t.user_id AND topic_users.topic_id = $1 AND t.count > 0",
    )
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE topic_users SET bookmarked = false FROM tmp_sync_topic_user_bookmarks \
         WHERE topic_users.topic_id = $1 AND topic_users.bookmarked = true \
           AND topic_users.user_id NOT IN (SELECT user_id FROM tmp_sync_topic_user_bookmarks)",
    )
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `TopicFeaturedUsers#choose` (what Topic#feature_topic_users runs).
pub(crate) async fn choose_featured_users(
    conn: &mut sqlx::PgConnection,
    topic_id: i32,
) -> Result<(), AppError> {
    featured_users(&mut *conn, Some(topic_id)).await?;
    // update_participant_count
    sqlx::query(
        "UPDATE topics SET participant_count = (SELECT COUNT(DISTINCT user_id) FROM posts \
           WHERE topic_id = $1 AND NOT hidden AND post_type IN (1, 2, 3) AND deleted_at IS NULL) \
         WHERE id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `TopicFeaturedUsers.ensure_consistency!(topic_id)` (every topic for
/// None): two frequent and two recent posters other than the topic's
/// creator and last poster.
pub(crate) async fn featured_users(
    conn: &mut sqlx::PgConnection,
    topic_id: Option<i32>,
) -> Result<(), AppError> {
    sqlx::query(
        "WITH poster_stats AS ( \
           SELECT t.id, t.user_id AS topic_user_id, t.last_post_user_id, p.user_id, COUNT(*) post_count, \
                  MAX(p.created_at) last_post_date, MAX(p.id) last_post_id, \
                  ROW_NUMBER() OVER (PARTITION BY t.id ORDER BY MAX(p.created_at) DESC, MAX(p.id) DESC) AS recent_rank \
           FROM topics t JOIN posts p ON p.topic_id = t.id \
           WHERE p.deleted_at IS NULL AND NOT p.hidden AND p.post_type IN (1, 2, 3) \
             AND p.user_id <> t.user_id AND p.user_id <> t.last_post_user_id AND ($1::int IS NULL OR t.id = $1) \
           GROUP BY t.id, t.user_id, t.last_post_user_id, p.user_id), \
         selected_recent_posters AS ( \
           SELECT id, user_id, recent_rank + 2 AS rank FROM poster_stats \
           WHERE topic_user_id = last_post_user_id AND recent_rank <= 2 \
           UNION ALL \
           SELECT id, user_id, recent_rank + 2 AS rank FROM poster_stats \
           WHERE topic_user_id <> last_post_user_id AND recent_rank <= 1), \
         selected_frequent_posters AS ( \
           SELECT id, user_id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY post_count DESC, last_post_date DESC, last_post_id DESC) AS rank \
           FROM poster_stats WHERE topic_user_id = last_post_user_id AND recent_rank > 2 \
           UNION ALL \
           SELECT id, user_id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY post_count DESC, last_post_date DESC, last_post_id DESC) AS rank \
           FROM poster_stats WHERE topic_user_id <> last_post_user_id AND recent_rank > 1), \
         selected_topic_posters AS ( \
           SELECT id, user_id, rank FROM selected_frequent_posters WHERE rank <= 2 \
           UNION ALL \
           SELECT id, user_id, rank FROM selected_recent_posters) \
         UPDATE topics tt SET \
           featured_user1_id = x.featured_user1, featured_user2_id = x.featured_user2, \
           featured_user3_id = x.featured_user3, featured_user4_id = x.featured_user4 \
         FROM topics AS tt2 \
         LEFT OUTER JOIN ( \
           SELECT id, \
             MAX(CASE WHEN rank = 1 THEN user_id END) featured_user1, \
             MAX(CASE WHEN rank = 2 THEN user_id END) featured_user2, \
             MAX(CASE WHEN rank = 3 THEN user_id END) featured_user3, \
             MAX(CASE WHEN rank = 4 THEN user_id END) featured_user4 \
           FROM selected_topic_posters GROUP BY id) x ON x.id = tt2.id \
         WHERE tt.id = tt2.id AND ( \
           COALESCE(tt.featured_user1_id, -99) <> COALESCE(x.featured_user1, -99) OR \
           COALESCE(tt.featured_user2_id, -99) <> COALESCE(x.featured_user2, -99) OR \
           COALESCE(tt.featured_user3_id, -99) <> COALESCE(x.featured_user3, -99) OR \
           COALESCE(tt.featured_user4_id, -99) <> COALESCE(x.featured_user4, -99)) \
         AND ($1::int IS NULL OR tt.id = $1)",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Badge::FirstEmoji`
const FIRST_EMOJI_BADGE: i32 = 41;
/// `Badge::FirstOnebox`
const FIRST_ONEBOX_BADGE: i32 = 42;
/// `Badge::FirstReplyByEmail`
const FIRST_REPLY_BY_EMAIL_BADGE: i32 = 43;

/// `has_emoji?`: an emoji image outside the quotes.
fn has_emoji_outside_quotes(html: &str) -> bool {
    use crate::pretty_text::cleanup::{all_elements, element_name, has_class, parse};
    let dom = parse(html);
    let mut in_quote = std::collections::HashSet::new();
    for e in all_elements(&dom) {
        if has_class(&e, "quote") {
            for inner in crate::pretty_text::cleanup::all_elements_within(&e) {
                in_quote.insert(std::rc::Rc::as_ptr(&inner));
            }
        }
    }
    all_elements(&dom).iter().any(|e| {
        element_name(e) == Some("img")
            && has_class(e, "emoji")
            && !in_quote.contains(&std::rc::Rc::as_ptr(e))
    })
}

/// `Jobs::ProcessPost`: CookedPostProcessor over the post's cooked HTML,
/// the cooked column updated when it changes, then pull_hotlinked_images.
async fn process_post(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(post_id) = int_arg(args, "post_id")? else {
        return Ok(());
    };
    if args
        .get("cook")
        .is_some_and(|c| !c.is_null() && c != &json!(false))
    {
        return Err(Unsupported("process_post with cook (recooking)").into());
    }
    if args.get("image_sizes").is_some() {
        return Err(Unsupported("process_post with image sizes").into());
    }
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    #[derive(sqlx::FromRow)]
    struct Row {
        raw: String,
        cooked: String,
        topic_id: i32,
        post_number: i32,
        user_id: Option<i32>,
        image_upload_id: Option<i64>,
        topic_image_upload_id: Option<i64>,
        omit_nofollow: bool,
        author_staff_or_staged: bool,
        reply_to_post_number: Option<i32>,
        reply_quoted: bool,
        post_type: i32,
        hidden: bool,
        via_email: bool,
    }
    let tl3_no_follow = s.get("tl3_links_no_follow")?.truthy();
    let post: Option<Row> = sqlx::query_as(
        "SELECT p.raw, p.cooked, p.topic_id, p.post_number, p.user_id, p.image_upload_id, \
                t.image_upload_id AS topic_image_upload_id, \
                COALESCE(u.admin OR u.moderator OR (NOT $2 AND (u.staged OR u.trust_level >= 3)), FALSE) AS omit_nofollow, \
                COALESCE(u.admin OR u.moderator OR u.staged, FALSE) AS author_staff_or_staged, \
                p.reply_to_post_number, p.reply_quoted, p.post_type, p.hidden, p.via_email \
         FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .bind(tl3_no_follow)
    .fetch_optional(&mut *conn)
    .await?;
    // Nothing to do for a missing post or one whose topic is gone.
    let Some(post) = post else {
        return Ok(());
    };
    if bool_arg(args, "new_post")
        && s.get("remove_full_quote")?.truthy()
        && post.post_number > 1
        && post.raw.contains("[quote")
    {
        return Err(Unsupported("remove_full_quote_on_direct_reply").into());
    }
    // DiscourseEvent :before_post_process_cooked, on the cooked html as
    // the processor starts with it.
    crate::plugins::local_dates::before_post_process_cooked(&mut conn, &s, post_id, &post.cooked)
        .await?;
    let html = cooked_post_processor::post_process(
        &mut conn,
        &s,
        &state.config,
        &state.i18n,
        &post.cooked,
        post.omit_nofollow,
        Some(post_id),
    )
    .await?;
    // update_post_image: the post's (and a first post's topic's) image.
    let hostname = Urls {
        config: &state.config,
        settings: &s,
    }
    .current_hostname()?;
    let store = crate::file_store::FileStore::for_site(&state.config, &s)?;
    let image =
        crate::upload_references::post_image_upload(&mut conn, &hostname, &store, &html).await?;
    match image {
        Some(upload_id) => {
            sqlx::query("UPDATE posts SET image_upload_id = $2 WHERE id = $1")
                .bind(post_id)
                .bind(i64::from(upload_id))
                .execute(&mut *conn)
                .await?;
            if post.post_number == 1 {
                sqlx::query("UPDATE topics SET image_upload_id = $2 WHERE id = $1")
                    .bind(post.topic_id)
                    .bind(i64::from(upload_id))
                    .execute(&mut *conn)
                    .await?;
                // clear_generated_og_image!
                let og: Option<i64> =
                    sqlx::query_scalar("SELECT og_image_upload_id FROM topics WHERE id = $1")
                        .bind(post.topic_id)
                        .fetch_one(&mut *conn)
                        .await?;
                if let Some(og) = og {
                    sqlx::query("UPDATE topics SET og_image_upload_id = NULL WHERE id = $1")
                        .bind(post.topic_id)
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query(
                        "DELETE FROM upload_references WHERE target_type = 'Topic' AND target_id = $1 \
                         AND upload_id = $2",
                    )
                    .bind(post.topic_id)
                    .bind(og)
                    .execute(&mut *conn)
                    .await?;
                }
                if let Some(upload) =
                    crate::optimized_images::Upload::find(&mut conn, upload_id).await?
                {
                    crate::optimized_images::generate_topic_thumbnails(
                        &mut conn, &s, &store, &upload,
                    )
                    .await?;
                }
            }
        }
        None => {
            if post.image_upload_id.is_some() {
                sqlx::query("UPDATE posts SET image_upload_id = NULL WHERE id = $1")
                    .bind(post_id)
                    .execute(&mut *conn)
                    .await?;
            }
            if post.post_number == 1 {
                if post.topic_image_upload_id.is_some() {
                    sqlx::query("UPDATE topics SET image_upload_id = NULL WHERE id = $1")
                        .bind(post.topic_id)
                        .execute(&mut *conn)
                        .await?;
                }
                if s.get("generate_topic_og_image")?.truthy() {
                    return Err(Unsupported("generate_topic_og_image").into());
                }
            }
        }
    }
    // grant_badges: the first emoji badge is a write.
    // grant_badges: for a post anyone may see.
    if let Some(user_id) = post.user_id {
        let anonymous = crate::guardian::Guardian::anonymous();
        let visible = [1, 2, 3].contains(&post.post_type)
            && !post.hidden
            && crate::onebox::can_see_topic(&mut conn, &s, &anonymous, post.topic_id).await?;
        if visible {
            let mut badges = Vec::new();
            if has_emoji_outside_quotes(&html) {
                badges.push(FIRST_EMOJI_BADGE);
            }
            if crate::onebox::has_oneboxes(&post.cooked, s.get("max_oneboxes_per_post")?.to_i()) {
                badges.push(FIRST_ONEBOX_BADGE);
            }
            // is_reply_by_email?
            if post.via_email && post.post_number > 1 {
                badges.push(FIRST_REPLY_BY_EMAIL_BADGE);
            }
            for badge_id in badges {
                // Badge.find: a missing badge fails the job.
                let badge = crate::badge_granter::Badge::find(&mut conn, badge_id)
                    .await?
                    .ok_or(sqlx::Error::RowNotFound)?;
                crate::badge_granter::grant(
                    &mut conn,
                    &state.bus,
                    &s,
                    &state.i18n,
                    &badge,
                    user_id,
                    &crate::badge_granter::GrantOpts::system(Some(post_id)),
                )
                .await?;
            }
        }
    }
    // @post.link_post_uploads(fragments: @doc)
    crate::upload_references::link_post_uploads(&mut conn, &s, &hostname, &store, post_id, &html)
        .await?;
    if html != post.cooked {
        sqlx::query("UPDATE posts SET cooked = $2 WHERE id = $1")
            .bind(post_id)
            .bind(&html)
            .execute(&mut *conn)
            .await?;
        // sync_first_post_caches: the topic's excerpt from the new html; a
        // category's description is not synced from its definition here.
        if post.post_number == 1 {
            let definition: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
                    .bind(post.topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            if definition {
                return Err(
                    Unsupported("syncing a category description after post processing").into(),
                );
            }
            let excerpt = crate::excerpt::excerpt(
                &html,
                s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
                &crate::excerpt::Options {
                    strip_links: true,
                    image_mode: crate::excerpt::ImageMode::Strip,
                    ..Default::default()
                },
            );
            sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
                .bind(post.topic_id)
                .bind(excerpt)
                .execute(&mut *conn)
                .await?;
        }
        if let Some(user_id) = post.user_id {
            let urls = Urls {
                config: &state.config,
                settings: &s,
            };
            let hostname = urls.current_hostname()?;
            links::extract_from(
                &mut conn,
                &links::Site {
                    hostname: &hostname,
                    base_path: state.config.globals.relative_url_root(),
                    base_url_no_prefix: &urls.base_url_no_prefix()?,
                    settings: &s,
                },
                &LinkPost {
                    id: post_id,
                    user_id,
                    topic_id: post.topic_id,
                    cooked: &html,
                },
            )
            .await?;
        }
        let host = crate::pretty_text::Host::from_state(state);
        let ctx = crate::posting::Ctx {
            host: &host,
            settings: &s,
            config: &state.config,
            i18n: &state.i18n,
            bus: &state.bus,
        };
        // QuotedPost.extract_from: oneboxes may have quoted posts.
        crate::posting::rebake::extract_quoted_posts(
            &mut conn,
            post_id,
            post.topic_id,
            post.post_number,
            post.reply_to_post_number,
            post.reply_quoted,
            &html,
        )
        .await?;
        crate::bus::publish_post_change(
            &ctx,
            &mut conn,
            post_id,
            "revised",
            Default::default(),
            false,
        )
        .await?;
    }
    if !bool_arg(args, "skip_pull_hotlinked_images") {
        // Jobs.cancel_scheduled_job, then enqueue.
        sqlx::query(
            "DELETE FROM discourse_rs.jobs WHERE name = 'pull_hotlinked_images' AND failed_at IS NULL \
             AND locked_until IS NULL AND args = $1",
        )
        .bind(json!({"post_id": post_id}))
        .execute(&mut *conn)
        .await?;
        super::enqueue(
            &mut conn,
            "pull_hotlinked_images",
            json!({"post_id": post_id}),
        )
        .await?;
    }
    // WordWatcher flags (watched words) for non-staff authors.
    if !post.author_staff_or_staged && !bool_arg(args, "bypass_bump") {
        use crate::word_watcher as ww;
        let flag = ww::compiled_for_action(&mut conn, &s, ww::FLAG).await?;
        let mut text = post.raw.clone();
        if post.post_number == 1 {
            let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
                .bind(post.topic_id)
                .fetch_one(&mut *conn)
                .await?;
            text.push(' ');
            text.push_str(&title);
        }
        let words = ww::matches(&text, &flag, true)?;
        if !words.is_empty() {
            // PostActionCreator.create(Discourse.system_user, post,
            // :inappropriate, reason: :watched_word, context: words).
            drop(conn);
            let system =
                crate::session::current::SessionUser::load(&mut *state.pool.acquire().await?, -1)
                    .await?
                    .ok_or(Unsupported("a site without its system user"))?;
            let guardian =
                crate::guardian::Guardian::for_user(&mut *state.pool.acquire().await?, &system)
                    .await?;
            let types =
                crate::post_actions::ActionTypes::load(&mut *state.pool.acquire().await?).await?;
            let type_id = types
                .types
                .iter()
                .find(|(k, _)| k == "inappropriate")
                .map(|(_, id)| *id)
                .ok_or(Unsupported("a site without the inappropriate flag"))?;
            let host = crate::pretty_text::Host::from_state(state);
            let ctx = crate::posting::Ctx {
                host: &host,
                settings: &s,
                config: &state.config,
                i18n: &state.i18n,
                bus: &state.bus,
            };
            let request = crate::flags::FlagRequest {
                type_id,
                take_action: false,
                queue_for_review: false,
                message: None,
                reason: Some("watched_word"),
                context: Some(words.join(",")),
            };
            match crate::flags::flag(&state.pool, &ctx, &guardian, post_id, &request).await? {
                crate::likes::Outcome::Done => {}
                _ => {
                    return Err(Unsupported("a watched word flag PostActionCreator refuses").into());
                }
            }
        }
    }
    Ok(())
}

/// `Jobs::PostAlert`, in one transaction so a refused case writes nothing.
async fn post_alert(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut tx = state.pool.begin().await?;
    super::post_alert::run(&ctx, &mut tx, args).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::GroupPmAlert`, in one transaction.
async fn group_pm_alert(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let id = |key: &str| args.get(key).and_then(Value::as_i64).map(|v| v as i32);
    let (Some(user_id), Some(group_id), Some(post_id)) =
        (id("user_id"), id("group_id"), id("post_id"))
    else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    crate::pm_invite::group_pm_alert(&mut tx, &ctx, user_id, group_id, post_id).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::ClosePoll`: the poll closed by the system user, quietly not
/// when it may not be.
async fn close_poll(state: &AppState, args: &Value) -> Result<(), AppError> {
    let post_id = args.get("post_id").and_then(Value::as_i64);
    let poll_name = args.get("poll_name").and_then(Value::as_str);
    let (Some(post_id), Some(poll_name)) = (post_id, poll_name.filter(|n| !n.is_empty())) else {
        return Err(Unsupported("ClosePoll without its post_id or poll_name").into());
    };
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let system = crate::session::current::SessionUser::load(&mut conn, -1)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    let guardian = crate::guardian::Guardian::for_user(&mut conn, &system).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let mut tx = state.pool.begin().await?;
    crate::plugins::poll::vote::toggle_status(
        &mut tx, &ctx, &guardian, post_id, poll_name, "closed", false,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::DeleteInaccessibleNotifications`: the notifications on a topic
/// of the given users (or of everyone notified on it) who can no longer
/// see it go, and those users' counts are published. For a message,
/// those outside `all_allowed_users` (admins see it unless secured
/// categories are hidden from them); otherwise each user's guardian.
async fn delete_inaccessible_notifications(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(topic_id) = args
        .get("topic_id")
        .and_then(Value::as_i64)
        .map(|v| v as i32)
    else {
        return Err(Unsupported(
            "delete_inaccessible_notifications without a topic (InvalidParameters)",
        )
        .into());
    };
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let mut tx = state.pool.begin().await?;
    // Topic.find_by: Trashable's default scope leaves a deleted topic out
    // (so the job's deleted_at branch never runs).
    let archetype: Option<String> =
        sqlx::query_scalar("SELECT archetype FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(topic_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(archetype) = archetype else {
        return Ok(());
    };
    let given: Option<Vec<i32>> = args.get("user_ids").and_then(Value::as_array).map(|ids| {
        ids.iter()
            .filter_map(Value::as_i64)
            .map(|v| v as i32)
            .collect()
    });
    let user_ids: Vec<i32> = match given.filter(|ids| !ids.is_empty()) {
        Some(ids) => ids,
        None => {
            sqlx::query_scalar("SELECT DISTINCT user_id FROM notifications WHERE topic_id = $1")
                .bind(topic_id)
                .fetch_all(&mut *tx)
                .await?
        }
    };
    if user_ids.is_empty() {
        return Ok(());
    }
    let mut inaccessible: Vec<i32> = Vec::new();
    if archetype == "private_message" {
        let mut accessible: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 AND user_id = ANY($2) \
             UNION SELECT gu.user_id FROM topic_allowed_groups tag JOIN group_users gu ON gu.group_id = tag.group_id \
                   WHERE tag.topic_id = $1 AND gu.user_id = ANY($2)",
        )
        .bind(topic_id)
        .bind(&user_ids)
        .fetch_all(&mut *tx)
        .await?;
        let flagged_or_warning: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND subtype = 'moderator_warning') \
             OR EXISTS (SELECT 1 FROM reviewables WHERE topic_id = $1 \
                        AND type = 'ReviewableFlaggedPost' AND status = 0)",
        )
        .bind(topic_id)
        .fetch_one(&mut *tx)
        .await?;
        if flagged_or_warning {
            return Err(Unsupported("moderators' access to flagged messages and warnings").into());
        }
        if !s.get("suppress_secured_categories_from_admin")?.truthy() {
            let admins: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1) AND admin")
                    .bind(&user_ids)
                    .fetch_all(&mut *tx)
                    .await?;
            accessible.extend(admins);
        }
        inaccessible = user_ids
            .into_iter()
            .filter(|u| !accessible.contains(u))
            .collect();
    } else {
        for user_id in user_ids {
            let Some(user) = crate::session::current::SessionUser::load(&mut tx, user_id).await?
            else {
                continue;
            };
            let guardian = crate::guardian::Guardian::for_user(&mut tx, &user).await?;
            let topic = crate::topic_guardian::TopicCtx::load(&mut tx, &s, &guardian, topic_id)
                .await?
                .ok_or(Unsupported("checking access to a topic that is gone"))?;
            let secure = guardian.secure_category_ids(&mut tx, &s).await?;
            if !guardian.can_see_topic(&s, &topic, true, &secure)? {
                inaccessible.push(user_id);
            }
        }
    }
    if inaccessible.is_empty() {
        return Ok(());
    }
    sqlx::query("DELETE FROM notifications WHERE user_id = ANY($1) AND topic_id = $2")
        .bind(&inaccessible)
        .bind(topic_id)
        .execute(&mut *tx)
        .await?;
    let mut ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM users WHERE id = ANY($1)")
        .bind(&inaccessible)
        .fetch_all(&mut *tx)
        .await?;
    ids.sort_unstable();
    for user_id in ids {
        crate::bus::publish_notifications_state(&state.bus, &mut tx, &s, user_id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::GroupPmUpdateSummary`: the group's members get their default
/// levels on the message (but those with one already), then those
/// tracking by default get their summary of its inbox.
async fn group_pm_update_summary(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let id = |key: &str| args.get(key).and_then(Value::as_i64).map(|v| v as i32);
    let (Some(group_id), Some(topic_id)) = (id("group_id"), id("topic_id")) else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    let group: Option<(String, i32)> =
        sqlx::query_as("SELECT name, user_count FROM groups WHERE id = $1")
            .bind(group_id)
            .fetch_optional(&mut *tx)
            .await?;
    let topic: Option<Option<i32>> =
        sqlx::query_scalar("SELECT user_id FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(topic_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (Some((name, user_count)), Some(author)) = (group, topic) else {
        return Ok(());
    };
    let limit = s.get("group_pm_user_limit")?.to_i();
    if i64::from(user_count) > limit {
        // GroupPmUserLimitExceededError
        let message = state
            .i18n
            .t_with(
                "groups.errors.default_notification_level_users_limit",
                &[("count", &limit.to_string()), ("group_name", &name)],
            )
            .unwrap_or_default();
        return Err(std::io::Error::other(message).into());
    }
    crate::posting::group_messages::apply_default_levels(&mut tx, group_id, topic_id, author, true)
        .await?;
    let tracking: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users INNER JOIN group_users ON users.id = group_users.user_id \
         WHERE group_users.group_id = $1 AND group_users.notification_level = 2 ORDER BY users.id",
    )
    .bind(group_id)
    .fetch_all(&mut *tx)
    .await?;
    let stats = crate::posting::group_messages::group_stats(&mut tx, topic_id).await?;
    let acting = id("acting_user_id");
    for user_id in tracking {
        crate::posting::group_messages::notify_group_summary(
            &ctx, &mut tx, &stats, user_id, topic_id, acting,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::UserEmail`, in one transaction: a refused case writes no
/// unsubscribe key or log, and nothing is delivered before it commits.
async fn user_email(state: &AppState, args: &Value, critical: bool) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let posting = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let mut tx = state.pool.begin().await?;
    super::user_email::run(&mut tx, &ctx, &posting, args, critical).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::SendEmailLoginCode`: EmailLoginCodeMailer's code email, sent
/// with no user (`Email::Sender.new(message, :email_login_code)`).
async fn send_email_login_code(state: &AppState, args: &Value) -> Result<(), AppError> {
    let (Some(to), Some(code)) = (
        args.get("to_address").and_then(Value::as_str),
        args.get("code").and_then(Value::as_str),
    ) else {
        return Err(Unsupported(
            "send_email_login_code without an address or code (InvalidParameters)",
        )
        .into());
    };
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    if !s.get("enable_local_logins_via_code")?.truthy() {
        return Ok(());
    }
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::email::sender::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        mailer: &state.mailer,
    };
    let template = if bool_arg(args, "password_reset") {
        "password_reset_code_mailer"
    } else {
        "email_login_code_mailer"
    };
    let mut tx = state.pool.begin().await?;
    let built = crate::email::account::build_template(
        &mut tx,
        &ctx,
        template,
        to,
        &[("code", code), ("minutes", "10")],
        None,
    )
    .await?;
    crate::email::sender::send(&mut tx, &ctx, built, "email_login_code", None).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::ProcessEmail`: Email::Processor.process!(mail, source:).
async fn process_email(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mail = args.get("mail").and_then(Value::as_str).unwrap_or("");
    let source = args.get("source").and_then(Value::as_str);
    crate::email::receiver::process(state, mail, source).await
}

/// `Jobs::SendSystemMessage`: SystemMessage.create for the user, the
/// message options interpolated into the templates.
async fn send_system_message(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(user_id) = int_arg(args, "user_id")? else {
        return Err(
            Unsupported("send_system_message without a user_id (InvalidParameters)").into(),
        );
    };
    let Some(message_type) = args
        .get("message_type")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    else {
        return Err(
            Unsupported("send_system_message without a message_type (InvalidParameters)").into(),
        );
    };
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    let s = settings(state, &mut conn).await?;
    drop(conn);
    let mut params = Vec::new();
    let mut post_alert_options = None;
    if let Some(options) = args.get("message_options").and_then(Value::as_object) {
        for (key, value) in options {
            if key == "post_alert_options" {
                post_alert_options = Some(value.clone());
                continue;
            }
            // Ruby's string interpolation of the value.
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => {
                    return Err(
                        Unsupported("system message options that are lists or hashes").into(),
                    );
                }
            };
            params.push((key.clone(), text));
        }
    }
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    crate::system_message::create(
        &state.pool,
        &ctx,
        user_id,
        message_type,
        &params,
        post_alert_options,
    )
    .await?;
    Ok(())
}

/// Jobs::Chat::NotifyWatching
async fn chat_notify_watching(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    crate::plugins::chat::notify::notify_watching(state, &mut tx, &s, args).await?;
    tx.commit().await?;
    Ok(())
}

/// Jobs::Chat::NotifyMentioned
async fn chat_notify_mentioned(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    crate::plugins::chat::notify::notify_mentioned(state, &mut tx, &s, args).await?;
    tx.commit().await?;
    Ok(())
}

/// Jobs::Chat::ProcessMessage
async fn chat_process_message(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    crate::plugins::chat::create::process_message(state, &mut tx, &s, args).await?;
    tx.commit().await?;
    Ok(())
}

/// Jobs::Chat::UpdateThreadReplyCount: the thread's replies (its live
/// messages but the original) counted into replies_count. Rails throttles
/// it through Redis and enqueues it again on each update, a run that
/// returns early; here it runs once.
async fn chat_update_thread_reply_count(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(thread_id) = args["thread_id"].as_i64() else {
        return Ok(());
    };
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE chat_threads SET replies_count = (SELECT COUNT(*) FROM chat_messages m \
           WHERE m.thread_id = chat_threads.id AND m.deleted_at IS NULL AND m.id <> chat_threads.original_message_id), \
         updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(thread_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
