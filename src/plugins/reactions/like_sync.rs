//! `Jobs::DiscourseReactions::ScheduledLikeSynchronizer` (daily: enqueues
//! the sync) and `Jobs::DiscourseReactions::LikeSynchronizer` with
//! `ReactionLikeSynchronizer#sync!`: every reaction that counts as a like
//! on a live post by someone else has a like post action (created, or its
//! newest trashed one recovered) with its user actions, every live like
//! whose reaction is excluded from likes is trashed with its user
//! actions, and the counters these feed are recalculated by the shared
//! rule (`crate::like_counters`) for what changed. In batches of
//! reaction users by id, one transaction each.

use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

pub const SCHEDULED_JOB: &str = "Jobs::DiscourseReactions::ScheduledLikeSynchronizer";
pub const JOB: &str = "Jobs::DiscourseReactions::LikeSynchronizer";

/// `ReactionLikeSynchronizer::BATCH_SIZE`
const BATCH_SIZE: i64 = 2_000;
/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE: i32 = 2;
/// `UserAction::LIKE`, `UserAction::WAS_LIKED`
const UA_LIKE: i32 = 1;
const UA_WAS_LIKED: i32 = 2;

/// `ScheduledLikeSynchronizer#execute`
pub async fn run_scheduled(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    crate::jobs::enqueue(&mut conn, JOB, serde_json::json!({})).await?;
    Ok(())
}

/// `LikeSynchronizer#execute`. The worker runs one job at a time, as
/// `cluster_concurrency 1` asks.
pub async fn run(state: &AppState) -> Result<(), AppError> {
    let settings = {
        let mut conn = state.pool.acquire().await?;
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?
    };
    if !settings.get("discourse_reactions_enabled")?.truthy()
        || !settings
            .get("discourse_reactions_like_sync_enabled")?
            .truthy()
    {
        return Ok(());
    }
    // Reaction.reactions_excluded_from_like
    let excluded: Vec<String> = settings
        .get("discourse_reactions_excluded_from_like")?
        .to_s()
        .split('|')
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
    let (min_id, max_id): (Option<i64>, Option<i64>) =
        sqlx::query_as("SELECT MIN(id), MAX(id) FROM discourse_reactions_reaction_users")
            .fetch_one(&state.pool)
            .await?;
    let (min_id, max_id) = (min_id.unwrap_or(0), max_id.unwrap_or(0));
    let (mut inserted, mut recovered, mut trashed) = (0, 0, 0);
    let mut batch_min = min_id;
    while batch_min <= max_id {
        let ids = (batch_min, batch_min + BATCH_SIZE - 1);
        let mut tx = state.pool.begin().await?;
        let counts = sync_batch(&mut tx, &settings, &excluded, ids).await?;
        tx.commit().await?;
        inserted += counts.0;
        recovered += counts.1;
        trashed += counts.2;
        batch_min += BATCH_SIZE;
    }
    tracing::info!(
        "[ReactionLikeSynchronizer] Inserted {inserted}, recovered {recovered} and trashed {trashed} post action likes."
    );
    Ok(())
}

/// `eligible_reaction_users_sql`: the batch's reaction users that count
/// as likes (`<> ALL` of no excluded reactions holds), on live posts by
/// someone else. $2 the excluded reactions, $3 and $4 the batch's id
/// range.
const ELIGIBLE: &str = "SELECT ru.user_id, ru.post_id, ru.created_at, ru.updated_at \
     FROM discourse_reactions_reaction_users ru \
     INNER JOIN discourse_reactions_reactions ON discourse_reactions_reactions.id = ru.reaction_id \
     INNER JOIN posts ON posts.id = ru.post_id \
     WHERE ru.id BETWEEN $3 AND $4 AND posts.deleted_at IS NULL \
       AND posts.user_id <> ru.user_id \
       AND discourse_reactions_reactions.reaction_value <> ALL($2)";

/// `sync_batch`: the inserted, recovered and trashed counts.
async fn sync_batch(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    excluded: &[String],
    (min_id, max_id): (i64, i64),
) -> Result<(usize, usize, usize), AppError> {
    // create_missing_post_actions
    let inserted: Vec<i32> = sqlx::query_scalar(&format!(
        "WITH eligible AS ({ELIGIBLE}) \
         INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at) \
         SELECT eligible.post_id, eligible.user_id, $1, eligible.created_at, eligible.updated_at \
         FROM eligible \
         LEFT JOIN post_actions ON post_actions.user_id = eligible.user_id \
           AND post_actions.post_id = eligible.post_id AND post_actions.post_action_type_id = $1 \
         WHERE post_actions.id IS NULL \
         ON CONFLICT DO NOTHING \
         RETURNING post_actions.id"
    ))
    .bind(LIKE)
    .bind(excluded)
    .bind(min_id)
    .bind(max_id)
    .fetch_all(&mut *conn)
    .await?;
    // recover_trashed_post_actions: the newest like of a user and post
    // that has only trashed ones.
    let recovered: Vec<i32> = sqlx::query_scalar(&format!(
        "WITH eligible AS ({ELIGIBLE}), \
         recoverable AS ( \
           SELECT MAX(post_actions.id) AS id FROM post_actions \
           INNER JOIN eligible ON eligible.user_id = post_actions.user_id \
             AND eligible.post_id = post_actions.post_id \
           WHERE post_actions.post_action_type_id = $1 \
           GROUP BY post_actions.user_id, post_actions.post_id \
           HAVING BOOL_AND(post_actions.deleted_at IS NOT NULL)) \
         UPDATE post_actions SET deleted_at = NULL, deleted_by_id = NULL, updated_at = NOW() \
         FROM recoverable WHERE post_actions.id = recoverable.id \
         RETURNING post_actions.id"
    ))
    .bind(LIKE)
    .bind(excluded)
    .bind(min_id)
    .bind(max_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut created = inserted.clone();
    created.extend(&recovered);

    // create_missing_user_actions: LIKE for the liker, WAS_LIKED for the
    // author.
    if !created.is_empty() {
        sqlx::query(
            "INSERT INTO user_actions (action_type, user_id, acting_user_id, target_post_id, \
                                       target_topic_id, created_at, updated_at) \
             SELECT types.action_type, \
                    CASE types.action_type WHEN $2 THEN post_actions.user_id ELSE posts.user_id END, \
                    post_actions.user_id, post_actions.post_id, posts.topic_id, \
                    post_actions.created_at, post_actions.created_at \
             FROM post_actions INNER JOIN posts ON posts.id = post_actions.post_id \
             CROSS JOIN (VALUES ($2), ($3)) AS types(action_type) \
             WHERE post_actions.id = ANY($1) AND posts.user_id IS NOT NULL \
             ON CONFLICT DO NOTHING",
        )
        .bind(&created)
        .bind(UA_LIKE)
        .bind(UA_WAS_LIKED)
        .execute(&mut *conn)
        .await?;
    }

    // trash_excluded_post_actions: live likes only.
    let trashed: Vec<i32> = if excluded.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            "UPDATE post_actions SET deleted_at = NOW() FROM discourse_reactions_reaction_users ru \
             INNER JOIN discourse_reactions_reactions ON discourse_reactions_reactions.id = ru.reaction_id \
             WHERE post_actions.deleted_at IS NULL AND post_actions.user_id = ru.user_id \
               AND post_actions.post_id = ru.post_id AND post_actions.post_action_type_id = $1 \
               AND discourse_reactions_reactions.reaction_value = ANY($2) \
               AND ru.id BETWEEN $3 AND $4 \
             RETURNING post_actions.id",
        )
        .bind(LIKE)
        .bind(excluded)
        .bind(min_id)
        .bind(max_id)
        .fetch_all(&mut *conn)
        .await?
    };
    // delete_excluded_user_actions
    if !trashed.is_empty() {
        sqlx::query(
            "DELETE FROM user_actions USING post_actions \
             WHERE post_actions.id = ANY($1) \
               AND user_actions.target_post_id = post_actions.post_id \
               AND user_actions.acting_user_id = post_actions.user_id \
               AND user_actions.action_type IN ($2, $3)",
        )
        .bind(&trashed)
        .bind(UA_LIKE)
        .bind(UA_WAS_LIKED)
        .execute(&mut *conn)
        .await?;
    }

    let mut affected = created;
    affected.extend(&trashed);
    recalculate_counters(conn, settings, &affected).await?;
    Ok((inserted.len(), recovered.len(), trashed.len()))
}

/// `recalculate_counters`: the posts', topics', likers' and authors'
/// like counters, the likers' daily likes and the topic users' liked.
async fn recalculate_counters(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_action_ids: &[i32],
) -> Result<(), AppError> {
    if post_action_ids.is_empty() {
        return Ok(());
    }
    let rows: Vec<(i32, i32, i32, Option<i32>)> = sqlx::query_as(
        "SELECT post_actions.post_id, posts.topic_id, post_actions.user_id, posts.user_id \
         FROM post_actions INNER JOIN posts ON posts.id = post_actions.post_id \
         WHERE post_actions.id = ANY($1)",
    )
    .bind(post_action_ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut post_ids = Vec::new();
    let mut topic_ids = Vec::new();
    let mut liker_ids = Vec::new();
    let mut author_ids = Vec::new();
    for (post_id, topic_id, liker_id, author_id) in rows {
        push_unique(&mut post_ids, post_id);
        push_unique(&mut topic_ids, topic_id);
        push_unique(&mut liker_ids, liker_id);
        if let Some(author_id) = author_id {
            push_unique(&mut author_ids, author_id);
        }
    }
    let mut stat_user_ids = liker_ids.clone();
    for id in author_ids {
        push_unique(&mut stat_user_ids, id);
    }

    crate::like_counters::posts(&mut *conn, settings, &post_ids).await?;
    crate::like_counters::topics(&mut *conn, settings, &topic_ids).await?;
    crate::like_counters::user_stats(&mut *conn, &stat_user_ids).await?;
    crate::like_counters::given_daily_likes(&mut *conn, settings, &liker_ids).await?;
    // TopicUser.update_post_action_cache(post_id:)
    sqlx::query(
        "UPDATE topic_users tu SET liked = x.state FROM ( \
           SELECT CASE WHEN EXISTS ( \
             SELECT 1 FROM post_actions pa JOIN posts p on p.id = pa.post_id JOIN topics t ON t.id = p.topic_id \
             WHERE pa.deleted_at IS NULL AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
               AND pa.post_action_type_id = $2 AND tu2.topic_id = t.id AND tu2.user_id = pa.user_id LIMIT 1 \
           ) THEN true ELSE false END state, tu2.topic_id, tu2.user_id \
           FROM topic_users tu2 \
           WHERE tu2.topic_id IN (SELECT topic_id FROM posts WHERE id = ANY($1)) \
             AND tu2.user_id IN (SELECT user_id FROM post_actions WHERE post_id = ANY($1) \
                                 AND post_action_type_id = $2) \
         ) x WHERE x.topic_id = tu.topic_id AND x.user_id = tu.user_id AND x.state != tu.liked",
    )
    .bind(&post_ids)
    .bind(LIKE)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn push_unique(ids: &mut Vec<i32>, id: i32) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}
