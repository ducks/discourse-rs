//! The scheduled jobs that keep the topic lists' numbers fresh: /top's
//! scores (`TopTopic.refresh_daily!` and `refresh_older!`), /hot's
//! (`TopicHotScore.update_scores`) and the categories' counts
//! (`Category.update_stats`).

use serde_json::json;
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::TopRefreshToday`, hourly.
pub const TOP_REFRESH_TODAY: &str = "top_refresh_today";
/// `Jobs::TopRefreshOlder`, daily.
pub const TOP_REFRESH_OLDER: &str = "top_refresh_older";
/// `Jobs::UpdateTopicHotScores`, every ten minutes.
pub const UPDATE_TOPIC_HOT_SCORES: &str = "update_topic_hot_scores";
/// `Jobs::CategoryStats`, daily.
pub const CATEGORY_STATS: &str = "category_stats";

/// `Discourse.system_user`
const SYSTEM_USER_ID: i32 = -1;
/// `Post.types`
const REGULAR: i32 = 1;
const SMALL_ACTION: i32 = 3;

async fn settings(state: &AppState, conn: &mut PgConnection) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `DistributedMutex.synchronize("update_top_topics")`, for the
/// transaction.
async fn top_topics_mutex(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('update_top_topics'))")
        .execute(conn)
        .await?;
    Ok(())
}

/// `TopTopic.refresh_daily!`: hidden, archived, deleted, private and
/// category definition topics leave top_topics, new visible ones join,
/// and the daily counts and score are computed.
pub async fn top_refresh_today(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    top_topics_mutex(&mut tx).await?;
    sqlx::query(
        "WITH category_definition_topic_ids AS (SELECT COALESCE(topic_id, 0) AS id FROM categories), \
         invisible_topic_ids AS ( \
           SELECT id FROM topics WHERE deleted_at IS NOT NULL OR NOT visible \
             OR archetype = 'private_message' OR archived \
             OR id IN (SELECT id FROM category_definition_topic_ids)) \
         DELETE FROM top_topics WHERE topic_id IN (SELECT id FROM invisible_topic_ids)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "WITH category_definition_topic_ids AS (SELECT COALESCE(topic_id, 0) AS id FROM categories), \
         visible_topics AS ( \
           SELECT t.id FROM topics t LEFT JOIN top_topics tt ON t.id = tt.topic_id \
           WHERE t.deleted_at IS NULL AND t.visible AND t.archetype <> 'private_message' \
             AND NOT t.archived AND t.id NOT IN (SELECT id FROM category_definition_topic_ids) \
             AND tt.topic_id IS NULL) \
         INSERT INTO top_topics (topic_id) SELECT id FROM visible_topics",
    )
    .execute(&mut *tx)
    .await?;
    update_counts_and_compute_scores_for(&mut tx, &s, "daily").await?;
    tx.commit().await?;
    Ok(())
}

/// `TopTopic.refresh_older!`: the weekly to yearly counts and scores,
/// then the all-time score.
pub async fn top_refresh_older(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    top_topics_mutex(&mut tx).await?;
    // periods - [:daily, :all], in TopTopic.periods' order.
    for period in ["yearly", "quarterly", "monthly", "weekly"] {
        update_counts_and_compute_scores_for(&mut tx, &s, period).await?;
    }
    compute_top_score_for(&mut tx, &s, "all").await?;
    tx.commit().await?;
    Ok(())
}

/// `start_of(period)`, as SQL.
fn start_of(period: &str) -> &'static str {
    match period {
        "yearly" => "now() - interval '1 year'",
        "quarterly" => "now() - interval '3 months'",
        "monthly" => "now() - interval '1 month'",
        "weekly" => "now() - interval '1 week'",
        _ => "now() - interval '1 day'",
    }
}

async fn update_counts_and_compute_scores_for(
    conn: &mut PgConnection,
    s: &SiteSettings,
    period: &str,
) -> Result<(), AppError> {
    let from = start_of(period);
    let counts = [
        (
            "posts",
            format!(
                "SELECT topic_id, GREATEST(COUNT(*), 1) AS count FROM posts \
                 WHERE created_at >= {from} AND deleted_at IS NULL AND NOT hidden \
                   AND post_type = {REGULAR} AND user_id <> {SYSTEM_USER_ID} GROUP BY topic_id"
            ),
        ),
        (
            "views",
            format!(
                "SELECT topic_id, COUNT(*) AS count FROM topic_views WHERE viewed_at >= {from} GROUP BY topic_id"
            ),
        ),
        (
            "likes",
            format!(
                "SELECT topic_id, SUM(like_count) AS count FROM posts \
                 WHERE created_at >= {from} AND deleted_at IS NULL AND NOT hidden \
                   AND post_type = {REGULAR} GROUP BY topic_id"
            ),
        ),
        (
            "op_likes",
            format!(
                "SELECT topic_id, like_count AS count FROM posts \
                 WHERE created_at >= {from} AND post_number = 1 AND deleted_at IS NULL \
                   AND NOT hidden AND post_type = {REGULAR}"
            ),
        ),
    ];
    for (sort, inner) in counts {
        // update_top_topics
        sqlx::query(&format!(
            "UPDATE top_topics SET {period}_{sort}_count = c.count FROM top_topics tt \
             INNER JOIN ({inner}) c ON tt.topic_id = c.topic_id \
             WHERE tt.topic_id = top_topics.topic_id AND tt.{period}_{sort}_count <> c.count"
        ))
        .execute(&mut *conn)
        .await?;
    }
    compute_top_score_for(conn, s, period).await
}

/// Ruby's `Float#to_s` for the multipliers interpolated into the SQL; an
/// unset one (0) falls back to its default, an Integer.
fn multiplier(s: &SiteSettings, name: &str, fallback: i64) -> Result<String, AppError> {
    let value = crate::ruby::to_f(&s.get(name)?.to_s());
    Ok(if value == 0.0 {
        fallback.to_string()
    } else if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    })
}

/// `compute_top_score_for(period)`
async fn compute_top_score_for(
    conn: &mut PgConnection,
    s: &SiteSettings,
    period: &str,
) -> Result<(), AppError> {
    let log_views = multiplier(s, "top_topics_formula_log_views_multiplier", 2)?;
    // A 0 fallback that is itself a Float.
    let first_post_likes = {
        let value = crate::ruby::to_f(
            &s.get("top_topics_formula_first_post_likes_multiplier")?
                .to_s(),
        );
        if value == 0.0 {
            "0.5".to_string()
        } else {
            multiplier(s, "top_topics_formula_first_post_likes_multiplier", 0)?
        }
    };
    let least_likes = multiplier(s, "top_topics_formula_least_likes_per_post_multiplier", 3)?;
    let (top_topics, time_filter) = if period == "all" {
        (
            "(SELECT t.like_count all_likes_count, t.id topic_id, t.posts_count all_posts_count, \
               p.like_count all_op_likes_count, t.views all_views_count \
             FROM topics t JOIN posts p ON p.topic_id = t.id AND p.post_number = 1) as top_topics"
                .to_string(),
            "false".to_string(),
        )
    } else {
        (
            "top_topics".to_string(),
            format!("topics.created_at < {}", start_of(period)),
        )
    };
    sqlx::query(&format!(
        "WITH top AS ( \
           SELECT CASE WHEN {time_filter} THEN 0 \
             ELSE log(GREATEST({period}_views_count, 1)) * {log_views} + \
                  {period}_op_likes_count * {first_post_likes} + \
                  CASE WHEN {period}_likes_count > 0 AND {period}_posts_count > 0 \
                    THEN LEAST({period}_likes_count / {period}_posts_count, {least_likes}) ELSE 0 END + \
                  CASE WHEN topics.posts_count < 10 \
                    THEN 0 - ((10 - topics.posts_count) / 20) * {period}_op_likes_count ELSE 10 END + \
                  log(GREATEST({period}_posts_count, 1)) \
             END AS score, topic_id \
           FROM {top_topics} \
           LEFT JOIN topics ON topics.id = top_topics.topic_id AND topics.deleted_at IS NULL) \
         UPDATE top_topics SET {period}_score = top.score FROM top \
         WHERE top_topics.topic_id = top.topic_id AND {period}_score <> top.score"
    ))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::UpdateTopicHotScores`: every ten minutes while /hot is in the
/// top menu, else once in six hours (a Redis key with nx, here an
/// expiring key).
pub async fn update_topic_hot_scores(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let s = settings(state, &mut conn).await?;
    let hot_in_menu = s
        .get("top_menu")?
        .to_s()
        .split('|')
        .any(|item| item == "hot");
    if !hot_in_menu
        && !crate::owned_schema::set_once(&mut conn, "hot_score_6_hourly", 6 * 3600).await?
    {
        return Ok(());
    }
    update_scores(&mut conn, &s, 1000).await
}

/// `TopicHotScore.update_scores(max)`: a row for up to `max` topics that
/// have none, the recent likes and posters of the recently bumped, then
/// the score, (likes - 1) / (age in hours + 2) ^ gravity, of the `max`
/// most recently bumped and the `max` hottest. The hottest topic ids cache
/// it then rebuilds feeds only `is_hot`, which a theme or plugin must turn
/// on; the port has none.
async fn update_scores(
    conn: &mut PgConnection,
    s: &SiteSettings,
    max: i64,
) -> Result<(), AppError> {
    let now = crate::clock::now_naive();
    let gravity = crate::ruby::to_f(&s.get("hot_topics_gravity")?.to_s());
    let recent_days = s.get("hot_topics_recent_days")?.to_i();
    let recent_cutoff = now - chrono::Duration::days(recent_days);
    sqlx::query(
        "INSERT INTO topic_hot_scores (topic_id, score, recent_likes, recent_posters, created_at, updated_at) \
         SELECT topics.id, 0.0, 0, 0, $1, $1 FROM topics \
         LEFT OUTER JOIN topic_hot_scores ON topic_hot_scores.topic_id = topics.id \
         WHERE topic_hot_scores.topic_id IS NULL AND topics.deleted_at IS NULL \
           AND topics.archetype <> 'private_message' AND topics.created_at <= $1 AND topics.visible = true \
         ORDER BY CASE WHEN topics.pinned_at IS NOT NULL THEN 0 ELSE 1 END ASC, topics.bumped_at desc \
         LIMIT $2",
    )
    .bind(now)
    .bind(max)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE topic_hot_scores thsOrig SET \
           recent_likes = COALESCE(new_values.likes_count, 0), \
           recent_posters = COALESCE(new_values.unique_participants, 0), \
           recent_first_bumped_at = COALESCE(new_values.first_bumped_at, ths.recent_first_bumped_at) \
         FROM topic_hot_scores ths LEFT OUTER JOIN ( \
           SELECT t.id AS topic_id, COUNT(DISTINCT p.user_id) AS unique_participants, \
             (SELECT COUNT(distinct pa.user_id) FROM post_actions pa JOIN posts p2 ON p2.id = pa.post_id \
              WHERE p2.topic_id = t.id AND p2.post_type = $3 AND p2.deleted_at IS NULL \
                AND p2.user_deleted = false AND pa.post_action_type_id = 2 \
                AND pa.created_at >= $2 AND pa.deleted_at IS NULL) AS likes_count, \
             MIN(p.created_at) AS first_bumped_at \
           FROM topics t JOIN posts p ON t.id = p.topic_id \
           WHERE p.created_at >= $2 AND t.archetype <> 'private_message' AND t.deleted_at IS NULL \
             AND t.visible = true AND p.deleted_at IS NULL AND p.user_deleted = false \
             AND t.created_at <= $1 AND t.bumped_at >= $2 AND p.created_at < $1 \
             AND p.created_at >= $2 AND p.post_type = $3 \
           GROUP BY t.id) AS new_values ON ths.topic_id = new_values.topic_id \
         WHERE thsOrig.topic_id = ths.topic_id",
    )
    .bind(now)
    .bind(recent_cutoff)
    .bind(REGULAR)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "WITH topic_ids AS ( \
           SELECT topic_id FROM (SELECT th3.topic_id FROM topic_hot_scores th3 \
             JOIN topics t3 on t3.id = th3.topic_id ORDER BY t3.bumped_at DESC LIMIT $4) Y \
           UNION ALL \
           SELECT topic_id FROM (SELECT th2.topic_id FROM topic_hot_scores th2 \
             ORDER BY th2.score DESC, th2.recent_first_bumped_at DESC NULLS LAST LIMIT $4) X) \
         UPDATE topic_hot_scores ths SET score = CASE WHEN topics.visible = false THEN 0 ELSE \
           (( CASE WHEN topics.created_at > $2 THEN ths.recent_likes ELSE topics.like_count END ) / \
             (EXTRACT(EPOCH FROM ($1 - topics.created_at)) / 3600 + 2) ^ $3 \
            + CASE WHEN ths.recent_first_bumped_at IS NULL THEN 0 ELSE \
               (ths.recent_likes + ths.recent_posters - 1) / \
               (EXTRACT(EPOCH FROM ($1 - recent_first_bumped_at)) / 3600 + 2) ^ $3 END \
           ) * CASE WHEN topics.closed THEN 0.5 ELSE 1 END END, \
           updated_at = $1 \
         FROM topics WHERE topics.id IN (SELECT topic_id FROM topic_ids) \
           AND ths.topic_id = topics.id AND topics.created_at <= $1",
    )
    .bind(now)
    .bind(recent_cutoff)
    .bind(gravity)
    .bind(max)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct CategoryRow {
    id: i32,
    topic_id: Option<i32>,
    topics_year: Option<i32>,
    topics_month: Option<i32>,
    topics_week: Option<i32>,
    topics_day: Option<i32>,
    posts_year: Option<i32>,
    posts_month: Option<i32>,
    posts_week: Option<i32>,
    posts_day: Option<i32>,
}

/// `Category.update_stats`: each category's topic and post counts, then
/// (find_each) its counts of the last year, month, week and day, saved
/// when they changed.
pub async fn category_stats(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    sqlx::query(
        "UPDATE categories c SET topic_count = COALESCE(x.topic_count, 0), post_count = COALESCE(x.post_count, 0) \
         FROM (SELECT ccc.id as category_id, stats.topic_count, stats.post_count FROM categories ccc \
               LEFT JOIN (SELECT topics.category_id, COUNT(*) topic_count, SUM(topics.posts_count) post_count \
                          FROM topics WHERE topics.deleted_at IS NULL \
                            AND topics.id NOT IN (select cc.topic_id from categories cc WHERE topic_id IS NOT NULL) \
                            AND topics.visible = TRUE \
                          GROUP BY topics.category_id) stats ON stats.category_id = ccc.id) x \
         WHERE x.category_id = c.id \
           AND (c.topic_count <> COALESCE(x.topic_count, 0) OR c.post_count <> COALESCE(x.post_count, 0))",
    )
    .execute(&mut *conn)
    .await?;
    let categories: Vec<CategoryRow> = sqlx::query_as(
        "SELECT id, topic_id, topics_year, topics_month, topics_week, topics_day, \
                posts_year, posts_month, posts_week, posts_day FROM categories ORDER BY id",
    )
    .fetch_all(&mut *conn)
    .await?;
    for c in categories {
        let topics: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE created_at >= NOW() - INTERVAL '1 year'), \
                    COUNT(*) FILTER (WHERE created_at >= NOW() - INTERVAL '1 month'), \
                    COUNT(*) FILTER (WHERE created_at >= NOW() - INTERVAL '1 week'), \
                    COUNT(*) FILTER (WHERE created_at >= NOW() - INTERVAL '1 day') \
             FROM topics WHERE category_id = $1 AND visible = true AND deleted_at IS NULL \
               AND ($2::int IS NULL OR id != $2)",
        )
        .bind(c.id)
        .bind(c.topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let posts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE posts.created_at >= NOW() - INTERVAL '1 year'), \
                    COUNT(*) FILTER (WHERE posts.created_at >= NOW() - INTERVAL '1 month'), \
                    COUNT(*) FILTER (WHERE posts.created_at >= NOW() - INTERVAL '1 week'), \
                    COUNT(*) FILTER (WHERE posts.created_at >= NOW() - INTERVAL '1 day') \
             FROM posts INNER JOIN topics ON topics.id = posts.topic_id \
             WHERE topics.category_id = $1 AND topics.visible = true AND posts.deleted_at IS NULL \
               AND posts.user_deleted = false AND posts.post_type <> $3 \
               AND ($2::int IS NULL OR topics.id != $2)",
        )
        .bind(c.id)
        .bind(c.topic_id)
        .bind(SMALL_ACTION)
        .fetch_one(&mut *conn)
        .await?;
        let fresh = [
            topics.0, topics.1, topics.2, topics.3, posts.0, posts.1, posts.2, posts.3,
        ]
        .map(|n| Some(n as i32));
        let stored = [
            c.topics_year,
            c.topics_month,
            c.topics_week,
            c.topics_day,
            c.posts_year,
            c.posts_month,
            c.posts_week,
            c.posts_day,
        ];
        // c.save if c.changed?: its after_save_commit indexes it for search.
        if fresh != stored {
            sqlx::query(
                "UPDATE categories SET topics_year = $2, topics_month = $3, topics_week = $4, topics_day = $5, \
                   posts_year = $6, posts_month = $7, posts_week = $8, posts_day = $9, \
                   updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(c.id)
            .bind(fresh[0])
            .bind(fresh[1])
            .bind(fresh[2])
            .bind(fresh[3])
            .bind(fresh[4])
            .bind(fresh[5])
            .bind(fresh[6])
            .bind(fresh[7])
            .execute(&mut *conn)
            .await?;
            crate::jobs::enqueue(
                &mut conn,
                "index_category_for_search",
                json!({ "category_id": c.id, "force": false }),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn periods_start_where_rails_starts_them() {
        assert_eq!(super::start_of("quarterly"), "now() - interval '3 months'");
        assert_eq!(super::start_of("daily"), "now() - interval '1 day'");
    }
}
