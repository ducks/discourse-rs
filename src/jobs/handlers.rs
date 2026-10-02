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
        // Publishes the topic's tracking state on MessageBus, which is not
        // ported: there is no one to tell.
        "post_update_topic_tracking_state" => Ok(()),
        "feature_topic_users" => feature_topic_users(state, &job.args).await,
        "process_post" => process_post(state, &job.args).await,
        other => {
            return Err(JobError::Unported(format!(
                "not ported yet: the {other} job"
            )));
        }
    };
    Ok(result?)
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
    // TopicFeaturedUsers.ensure_consistency!(topic_id): two frequent and
    // two recent posters other than the topic's creator and last poster.
    sqlx::query(
        "WITH poster_stats AS ( \
           SELECT t.id, t.user_id AS topic_user_id, t.last_post_user_id, p.user_id, COUNT(*) post_count, \
                  MAX(p.created_at) last_post_date, MAX(p.id) last_post_id, \
                  ROW_NUMBER() OVER (PARTITION BY t.id ORDER BY MAX(p.created_at) DESC, MAX(p.id) DESC) AS recent_rank \
           FROM topics t JOIN posts p ON p.topic_id = t.id \
           WHERE p.deleted_at IS NULL AND NOT p.hidden AND p.post_type IN (1, 2, 3) \
             AND p.user_id <> t.user_id AND p.user_id <> t.last_post_user_id AND t.id = $1 \
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
         AND tt.id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
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

/// `Badge::FirstEmoji`
const FIRST_EMOJI_BADGE: i32 = 41;

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
        has_first_emoji: bool,
        first_emoji_enabled: bool,
    }
    let tl3_no_follow = s.get("tl3_links_no_follow")?.truthy();
    let post: Option<Row> = sqlx::query_as(
        "SELECT p.raw, p.cooked, p.topic_id, p.post_number, p.user_id, p.image_upload_id, \
                t.image_upload_id AS topic_image_upload_id, \
                COALESCE(u.admin OR u.moderator OR (NOT $2 AND (u.staged OR u.trust_level >= 3)), FALSE) AS omit_nofollow, \
                COALESCE(u.admin OR u.moderator OR u.staged, FALSE) AS author_staff_or_staged, \
                EXISTS (SELECT 1 FROM user_badges WHERE user_id = p.user_id AND badge_id = $3) AS has_first_emoji, \
                EXISTS (SELECT 1 FROM badges WHERE id = $3 AND enabled) AS first_emoji_enabled \
         FROM posts p JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .bind(tl3_no_follow)
    .bind(FIRST_EMOJI_BADGE)
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
    let html = cooked_post_processor::post_process(
        &mut conn,
        &s,
        &state.config,
        &post.cooked,
        post.omit_nofollow,
    )
    .await?;
    // update_post_image: images are refused by the post processor, so the
    // post (and a first post's topic) has none.
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
    // grant_badges: the first emoji badge is a write.
    let has_emoji = html.contains("class=\"emoji");
    if has_emoji && post.first_emoji_enabled && !post.has_first_emoji && post.user_id.is_some() {
        return Err(Unsupported("granting the first emoji badge").into());
    }
    if html != post.cooked {
        if post.post_number == 1 {
            return Err(Unsupported("first post caches after post processing").into());
        }
        sqlx::query("UPDATE posts SET cooked = $2 WHERE id = $1")
            .bind(post_id)
            .bind(&html)
            .execute(&mut *conn)
            .await?;
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
        let watched: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words)")
            .fetch_one(&mut *conn)
            .await?;
        if watched {
            return Err(Unsupported("watched words in process_post").into());
        }
    }
    Ok(())
}
