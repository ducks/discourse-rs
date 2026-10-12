//! `Jobs::DiscourseReactions::ScheduledLikeSynchronizer` with
//! `ReactionLikeSynchronizer#sync!`: hourly, every reaction that counts as
//! a like has a like post action (created, or recovered from the trash)
//! with its user actions, every like whose reaction is excluded from
//! likes is trashed with its user actions, and the counts these feed
//! (posts', topics', user stats', given daily likes, topic users' liked)
//! are recomputed for what changed. All in one transaction.

use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

pub const JOB: &str = "Jobs::DiscourseReactions::ScheduledLikeSynchronizer";

/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE: i32 = 2;
/// `UserAction::LIKE`, `UserAction::WAS_LIKED`
const UA_LIKE: i32 = 1;
const UA_WAS_LIKED: i32 = 2;

pub async fn run(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("discourse_reactions_enabled")?.truthy()
        || !settings
            .get("discourse_reactions_like_sync_enabled")?
            .truthy()
    {
        return Ok(());
    }
    sync(&mut tx, &settings).await?;
    tx.commit().await?;
    Ok(())
}

async fn sync(conn: &mut PgConnection, settings: &SiteSettings) -> Result<(), AppError> {
    // Reaction.reactions_excluded_from_like
    let excluded: Vec<String> = settings
        .get("discourse_reactions_excluded_from_like")?
        .to_s()
        .split('|')
        .map(str::to_string)
        .collect();
    let excluded = if excluded == [""] {
        Vec::new()
    } else {
        excluded
    };
    let not_excluded = if excluded.is_empty() {
        ""
    } else {
        " AND discourse_reactions_reactions.reaction_value <> ALL($2)"
    };

    // create_missing_post_actions
    let inserted: Vec<i32> = sqlx::query_scalar(&format!(
        "INSERT INTO post_actions (post_id, user_id, post_action_type_id, created_at, updated_at) \
         SELECT ru.post_id, ru.user_id, $1, ru.created_at, ru.updated_at \
         FROM discourse_reactions_reaction_users ru \
         INNER JOIN discourse_reactions_reactions ON discourse_reactions_reactions.id = ru.reaction_id \
         LEFT JOIN post_actions ON post_actions.user_id = ru.user_id AND post_actions.post_id = ru.post_id \
         WHERE post_actions.id IS NULL{not_excluded} \
         RETURNING post_actions.id"
    ))
    .bind(LIKE)
    .bind(&excluded)
    .fetch_all(&mut *conn)
    .await?;
    // recover_trashed_post_actions: the update has no RETURNING, so the
    // ids it gives back (DB.query_single) are always none.
    sqlx::query(&format!(
        "UPDATE post_actions SET deleted_at = NULL, deleted_by_id = NULL, updated_at = NOW() \
         FROM discourse_reactions_reaction_users ru \
         INNER JOIN discourse_reactions_reactions ON discourse_reactions_reactions.id = ru.reaction_id \
         WHERE post_actions.deleted_at IS NOT NULL AND post_actions.user_id = ru.user_id \
           AND post_actions.post_id = ru.post_id AND post_actions.post_action_type_id = $1{not_excluded}"
    ))
    .bind(LIKE)
    .bind(&excluded)
    .execute(&mut *conn)
    .await?;
    let mut created = inserted;
    created.dedup();

    // create_missing_user_actions
    if !created.is_empty() {
        for (action_type, user) in [
            (UA_LIKE, "post_actions.user_id"),
            (UA_WAS_LIKED, "posts.user_id"),
        ] {
            sqlx::query(&format!(
                "INSERT INTO user_actions (action_type, user_id, acting_user_id, target_post_id, \
                                           target_topic_id, created_at, updated_at) \
                 SELECT $1, {user}, post_actions.user_id, post_actions.post_id, posts.topic_id, \
                        post_actions.created_at, post_actions.created_at \
                 FROM post_actions INNER JOIN posts ON posts.id = post_actions.post_id \
                 WHERE post_actions.id = ANY($2) AND posts.user_id IS NOT NULL \
                 ON CONFLICT DO NOTHING"
            ))
            .bind(action_type)
            .bind(&created)
            .execute(&mut *conn)
            .await?;
        }
    }

    // trash_excluded_post_actions
    let trashed: Vec<i32> = if excluded.is_empty() {
        Vec::new()
    } else {
        sqlx::query_scalar(
            "UPDATE post_actions SET deleted_at = NOW() FROM discourse_reactions_reaction_users ru \
             INNER JOIN discourse_reactions_reactions ON discourse_reactions_reactions.id = ru.reaction_id \
             WHERE post_actions.user_id = ru.user_id AND post_actions.post_id = ru.post_id \
               AND post_actions.post_action_type_id = $1 \
               AND discourse_reactions_reactions.reaction_value = ANY($2) \
             RETURNING post_actions.id",
        )
        .bind(LIKE)
        .bind(&excluded)
        .fetch_all(&mut *conn)
        .await?
    };
    // delete_excluded_user_actions
    if !trashed.is_empty() {
        sqlx::query(
            "DELETE FROM user_actions WHERE id IN ( \
               SELECT user_actions.id FROM user_actions \
               INNER JOIN post_actions ON user_actions.target_post_id = post_actions.post_id \
                 AND user_actions.acting_user_id = post_actions.user_id \
               WHERE post_actions.id = ANY($1) AND user_actions.action_type = $2 \
               UNION \
               SELECT user_actions.id FROM user_actions user_actions \
               INNER JOIN post_actions ON user_actions.target_post_id = post_actions.post_id \
               INNER JOIN posts ON posts.id = post_actions.post_id \
               WHERE post_actions.id = ANY($1) AND user_actions.action_type = $3 \
                 AND user_actions.user_id = posts.user_id \
                 AND user_actions.acting_user_id = post_actions.user_id)",
        )
        .bind(&trashed)
        .bind(UA_LIKE)
        .bind(UA_WAS_LIKED)
        .execute(&mut *conn)
        .await?;
    }

    let mut affected = created;
    for id in trashed {
        if !affected.contains(&id) {
            affected.push(id);
        }
    }

    // update_post_like_counts
    sqlx::query(
        "UPDATE posts SET like_count = (SELECT COUNT(*) FROM post_actions \
           WHERE post_actions.post_id = posts.id AND post_action_type_id = 2 \
             AND post_actions.deleted_at IS NULL) \
         WHERE posts.id IN (SELECT DISTINCT post_id FROM post_actions WHERE id = ANY($1))",
    )
    .bind(&affected)
    .execute(&mut *conn)
    .await?;
    // update_topic_like_counts
    sqlx::query(
        "UPDATE topics SET like_count = (SELECT SUM(like_count) FROM posts WHERE posts.topic_id = topics.id) \
         WHERE topics.id IN (SELECT DISTINCT topic_id FROM posts \
           INNER JOIN post_actions ON post_actions.post_id = posts.id WHERE post_actions.id = ANY($1))",
    )
    .bind(&affected)
    .execute(&mut *conn)
    .await?;
    if !affected.is_empty() {
        update_user_stats(conn, &affected).await?;
        upsert_given_daily_likes(conn, &affected).await?;
    }
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
    .bind(
        sqlx::query_scalar::<_, i32>("SELECT DISTINCT post_id FROM post_actions WHERE id = ANY($1)")
            .bind(&affected)
            .fetch_all(&mut *conn)
            .await?,
    )
    .bind(LIKE)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `update_user_stats`: likes received of the liked posts' authors and
/// likes given of the likers, from their user actions; 0 for those with
/// none.
async fn update_user_stats(conn: &mut PgConnection, affected: &[i32]) -> Result<(), AppError> {
    let received_users: Vec<Option<i32>> = sqlx::query_scalar(
        "SELECT DISTINCT posts.user_id FROM posts INNER JOIN post_actions ON post_actions.post_id = posts.id \
         WHERE post_actions.id = ANY($1)",
    )
    .bind(affected)
    .fetch_all(&mut *conn)
    .await?;
    let received_users: Vec<i32> = received_users.into_iter().flatten().collect();
    if !received_users.is_empty() {
        let changed: Vec<i32> = sqlx::query_scalar(
            "WITH likes_received_cte AS ( \
               SELECT posts.user_id AS user_id, COUNT(user_actions.id) AS new_likes_received \
               FROM user_actions INNER JOIN posts ON user_actions.target_post_id = posts.id \
               WHERE user_actions.action_type = $2 AND posts.user_id = ANY($1) GROUP BY posts.user_id) \
             UPDATE user_stats SET likes_received = lrc.new_likes_received FROM likes_received_cte lrc \
             WHERE user_stats.user_id = lrc.user_id RETURNING user_stats.user_id",
        )
        .bind(&received_users)
        .bind(UA_WAS_LIKED)
        .fetch_all(&mut *conn)
        .await?;
        let rest: Vec<i32> = received_users
            .into_iter()
            .filter(|u| !changed.contains(u))
            .collect();
        sqlx::query("UPDATE user_stats SET likes_received = 0 WHERE user_id = ANY($1)")
            .bind(&rest)
            .execute(&mut *conn)
            .await?;
    }
    let given_users: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT user_id FROM post_actions WHERE post_actions.id = ANY($1)",
    )
    .bind(affected)
    .fetch_all(&mut *conn)
    .await?;
    if !given_users.is_empty() {
        let changed: Vec<i32> = sqlx::query_scalar(
            "WITH likes_given_cte AS ( \
               SELECT user_actions.acting_user_id AS user_id, COUNT(user_actions.id) AS new_likes_given \
               FROM user_actions WHERE user_actions.action_type = $2 \
                 AND user_actions.acting_user_id = ANY($1) GROUP BY user_actions.acting_user_id) \
             UPDATE user_stats SET likes_given = lgc.new_likes_given FROM likes_given_cte lgc \
             WHERE user_stats.user_id = lgc.user_id RETURNING user_stats.user_id",
        )
        .bind(&given_users)
        .bind(UA_LIKE)
        .fetch_all(&mut *conn)
        .await?;
        let rest: Vec<i32> = given_users
            .into_iter()
            .filter(|u| !changed.contains(u))
            .collect();
        sqlx::query("UPDATE user_stats SET likes_given = 0 WHERE user_id = ANY($1)")
            .bind(&rest)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `upsert_given_daily_likes`: the likers' and the authors' daily like
/// counts recounted from their user actions, the days left with none
/// deleted.
async fn upsert_given_daily_likes(
    conn: &mut PgConnection,
    affected: &[i32],
) -> Result<(), AppError> {
    let user_ids: Vec<Option<i32>> = sqlx::query_scalar(
        "SELECT user_id FROM post_actions WHERE post_actions.id = ANY($1) \
         UNION SELECT posts.user_id FROM posts INNER JOIN post_actions ON post_actions.post_id = posts.id \
         WHERE post_actions.id = ANY($1)",
    )
    .bind(affected)
    .fetch_all(&mut *conn)
    .await?;
    let user_ids: Vec<i32> = user_ids.into_iter().flatten().collect();
    if user_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO given_daily_likes (user_id, given_date, likes_given) \
         SELECT user_actions.acting_user_id, DATE(user_actions.created_at) AS given_date, COUNT(*) AS likes_given \
         FROM user_actions WHERE user_actions.action_type = $2 AND user_actions.acting_user_id = ANY($1) \
         GROUP BY user_actions.acting_user_id, DATE(user_actions.created_at) \
         ON CONFLICT (user_id, given_date) DO UPDATE SET likes_given = EXCLUDED.likes_given",
    )
    .bind(&user_ids)
    .bind(UA_LIKE)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM given_daily_likes gdl WHERE NOT EXISTS ( \
           SELECT 1 FROM user_actions WHERE user_actions.acting_user_id = gdl.user_id \
             AND user_actions.action_type = $2 AND DATE(user_actions.created_at) = gdl.given_date) \
         AND gdl.user_id = ANY($1)",
    )
    .bind(&user_ids)
    .bind(UA_LIKE)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
