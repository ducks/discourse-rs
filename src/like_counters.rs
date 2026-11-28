//! The like counters' one shared rule: `Post.recalculate_like_counts!`,
//! `Topic.recalculate_like_counts!`, `UserStat.recalculate_likes!` and
//! `GivenDailyLike.recalculate!`, each recounting its rows from scratch.

use sqlx::PgConnection;

use crate::AppError;
use crate::posting::{post_types, user_actions};
use crate::site_settings::SiteSettings;

/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE: i32 = 2;

/// `Post.recalculate_like_counts!`: like count and score (staff likes
/// weighted) from the posts' live likes, deleted posts included.
pub async fn posts(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_ids: &[i32],
) -> Result<(), AppError> {
    if post_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE posts SET \
           like_count = (SELECT COUNT(*) FROM post_actions WHERE post_actions.post_id = posts.id \
                           AND post_actions.post_action_type_id = $2 AND post_actions.deleted_at IS NULL), \
           like_score = (SELECT COALESCE(SUM(CASE WHEN users.moderator OR users.admin THEN $3 ELSE 1 END), 0) \
                         FROM post_actions INNER JOIN users ON users.id = post_actions.user_id \
                         WHERE post_actions.post_id = posts.id AND post_actions.post_action_type_id = $2 \
                           AND post_actions.deleted_at IS NULL) \
         WHERE posts.id = ANY($1)",
    )
    .bind(post_ids)
    .bind(LIKE)
    .bind(settings.get("staff_like_weight")?.to_i())
    .execute(conn)
    .await?;
    Ok(())
}

/// One topic's like count before and after `topics`.
#[derive(Debug, sqlx::FromRow)]
pub struct TopicCount {
    pub id: i32,
    pub previous_count: i32,
    pub current_count: i32,
}

/// `Topic.recalculate_like_counts!`: the topics' (deleted ones too) like
/// counts summed from their live non-whisper posts, a count crossing a
/// step of ten regenerating a generated og image.
pub async fn topics(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_ids: &[i32],
) -> Result<Vec<TopicCount>, AppError> {
    if topic_ids.is_empty() {
        return Ok(Vec::new());
    }
    let counts: Vec<TopicCount> = sqlx::query_as(
        "WITH previous AS (SELECT id, like_count FROM topics WHERE id = ANY($1)) \
         UPDATE topics SET like_count = ( \
           SELECT COALESCE(SUM(posts.like_count), 0) FROM posts \
           WHERE posts.topic_id = topics.id AND posts.post_type <> $2 AND posts.deleted_at IS NULL) \
         FROM previous WHERE topics.id = previous.id \
         RETURNING topics.id, previous.like_count AS previous_count, topics.like_count AS current_count",
    )
    .bind(topic_ids)
    .bind(post_types::WHISPER)
    .fetch_all(&mut *conn)
    .await?;
    // enqueue_og_image_regeneration_after_counter_change for each topic
    // whose count crossed a step.
    for c in &counts {
        let (previous, current) = (i64::from(c.previous_count), i64::from(c.current_count));
        if crate::og_image::counter_regeneration_needed(previous, current) {
            crate::og_image::after_counter_change(&mut *conn, settings, c.id, previous, current)
                .await?;
        }
    }
    Ok(counts)
}

/// `UserStat.recalculate_likes!`: likes given and received from the
/// users' user actions on countable posts (`COUNTABLE_POST_CONDITIONS`,
/// the directory refresh's visibility rule).
pub async fn user_stats(conn: &mut PgConnection, user_ids: &[i32]) -> Result<(), AppError> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let count = |param: &str| {
        format!(
            "SELECT COUNT(*) FROM user_actions ua \
             INNER JOIN posts p ON p.id = ua.target_post_id \
             INNER JOIN topics t ON t.id = p.topic_id \
             WHERE ua.user_id = user_stats.user_id AND ua.action_type = {param} \
               AND p.deleted_at IS NULL AND p.post_type = $4 AND NOT p.hidden \
               AND t.deleted_at IS NULL AND t.visible AND t.archetype = 'regular'"
        )
    };
    sqlx::query(&format!(
        "UPDATE user_stats SET likes_given = ({}), likes_received = ({}) WHERE user_id = ANY($1)",
        count("$2"),
        count("$3"),
    ))
    .bind(user_ids)
    .bind(user_actions::LIKE)
    .bind(user_actions::WAS_LIKED)
    .bind(post_types::REGULAR)
    .execute(conn)
    .await?;
    Ok(())
}

/// `GivenDailyLike.recalculate!`: the users' daily like counts rebuilt
/// from their live likes.
pub async fn given_daily_likes(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_ids: &[i32],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM given_daily_likes WHERE user_id = ANY($1)")
        .bind(user_ids)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO given_daily_likes (user_id, given_date, likes_given, limit_reached) \
         SELECT user_id, created_at::DATE, COUNT(*), COUNT(*) >= $3 FROM post_actions \
         WHERE user_id = ANY($1) AND post_action_type_id = $2 AND deleted_at IS NULL \
         GROUP BY user_id, created_at::DATE",
    )
    .bind(user_ids)
    .bind(LIKE)
    .bind(settings.get("max_likes_per_day")?.to_i())
    .execute(conn)
    .await?;
    Ok(())
}
