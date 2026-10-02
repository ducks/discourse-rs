//! Likes: PostActionCreator and PostActionDestroyer for the like action
//! type, with what a like touches (PostAction#update_counters, the topic
//! user's liked flag, GivenDailyLike, UserActionManager's LIKE and
//! WAS_LIKED rows and like counts, PostActionNotifier's liked
//! notification).
//!
//! Flags and other action types are refused, as are likes in messages,
//! anonymous mode and liked notifications that would consolidate. The
//! like rate limit (RateLimiter, Redis) and the badge queue are not
//! ported.

use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::post_actions::{self, ActOpts, ActionTypes, LIKE};
use crate::posting::revisions::find_post;
use crate::posting::{Ctx, post_types, user_actions};
use crate::{AppError, Unsupported};

/// How a like or unlike ends when it isn't a server error.
pub enum Outcome {
    Done,
    /// `Discourse::NotFound`
    NotFound,
    /// `render_json_error(result)` for a forbidden result: the message.
    Forbidden(&'static str),
}

/// The day GivenDailyLike counts in (`Date.today`, the server's UTC day).
const TODAY: &str = "(clock_timestamp() AT TIME ZONE 'UTC')::date";

/// `PostAction#update_counters` for a like: the post's like count and
/// score, the liker's topic user, the topic's like count.
async fn update_counters(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
    user_id: i32,
    topic_id: i32,
) -> Result<(), AppError> {
    let staff_weight = ctx.settings.get("staff_like_weight")?.to_i();
    sqlx::query(
        "UPDATE posts SET \
           like_count = (SELECT COUNT(*) FROM post_actions WHERE post_id = $1 AND post_action_type_id = $2 \
                           AND deleted_at IS NULL), \
           like_score = (SELECT COALESCE(SUM(CASE WHEN u.moderator OR u.admin THEN $3 ELSE 1 END), 0) \
                         FROM post_actions pa JOIN users u ON u.id = pa.user_id \
                         WHERE pa.post_id = $1 AND pa.deleted_at IS NULL) \
         WHERE id = $1",
    )
    .bind(post_id)
    .bind(LIKE as i32)
    .bind(staff_weight)
    .execute(&mut *conn)
    .await?;
    // TopicUser.update_post_action_cache(user_id:, topic_id:)
    sqlx::query(
        "UPDATE topic_users tu SET liked = x.state FROM ( \
           SELECT EXISTS ( \
             SELECT 1 FROM post_actions pa JOIN posts p ON p.id = pa.post_id JOIN topics t ON t.id = p.topic_id \
             WHERE pa.deleted_at IS NULL AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
               AND pa.post_action_type_id = $3 AND tu2.topic_id = t.id AND tu2.user_id = pa.user_id) AS state, \
             tu2.topic_id, tu2.user_id \
           FROM topic_users tu2 WHERE tu2.user_id = $1 AND tu2.topic_id = $2) x \
         WHERE x.topic_id = tu.topic_id AND x.user_id = tu.user_id AND x.state != tu.liked",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(LIKE as i32)
    .execute(&mut *conn)
    .await?;
    // Topic#update_action_counts
    sqlx::query(
        "UPDATE topics SET like_count = (SELECT COALESCE(SUM(like_count), 0) FROM posts \
                                         WHERE topic_id = $1 AND post_type <> $2) \
         WHERE id = $1",
    )
    .bind(topic_id)
    .bind(post_types::WHISPER)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `UserActionManager.post_action_rows`: LIKE for the liker, WAS_LIKED for
/// the author.
fn action_rows(liker: i32, author: Option<i32>) -> Vec<(i32, i32)> {
    let mut rows = vec![(user_actions::LIKE, liker)];
    if let Some(author) = author {
        rows.push((user_actions::WAS_LIKED, author));
    }
    rows
}

/// `UserAction.update_like_count`
async fn update_like_count(
    conn: &mut PgConnection,
    user_id: i32,
    action_type: i32,
    delta: i32,
) -> Result<(), sqlx::Error> {
    let column = match action_type {
        user_actions::LIKE => "likes_given",
        user_actions::WAS_LIKED => "likes_received",
        _ => return Ok(()),
    };
    sqlx::query(&format!(
        "UPDATE user_stats SET {column} = {column} + $2 WHERE user_id = $1"
    ))
    .bind(user_id)
    .bind(delta)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The post a like is about, if the user may see it, and what the checks
/// read about it.
struct Target {
    access: crate::posting::revisions::PostAccess,
}

async fn target(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Option<Target>, AppError> {
    let Some(access) = find_post(&mut *conn, ctx, guardian, post_id).await? else {
        return Ok(None);
    };
    if access.topic.private_message() {
        return Err(Unsupported("likes in messages").into());
    }
    Ok(Some(Target { access }))
}

/// `PostActionCreator.new(user, post, like).perform`
pub async fn like(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Outcome, AppError> {
    let user = guardian
        .user()
        .ok_or(Unsupported("liking anonymously"))?
        .clone();
    let mut tx = pool.begin().await?;
    let Some(Target { access }) = target(&mut tx, ctx, guardian, post_id).await? else {
        return Ok(Outcome::NotFound);
    };
    let post = &access.post;
    let types = ActionTypes::load(&mut tx).await?;
    let taken = post_actions::taken_actions(&mut tx, &[post.id], Some(user.id)).await?;
    let taken = taken.get(&post.id);
    let author_missing = match post.user_id {
        Some(id) => {
            !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?
        }
        None => true,
    };
    let can_act = guardian.post_can_act(
        ctx.settings,
        &types,
        ("like", LIKE),
        &ActOpts {
            topic: &access.topic,
            post,
            taken,
            can_see_post: access.can_see_post,
            author_missing,
        },
    )?;
    if !can_act {
        return Ok(Outcome::Forbidden(
            if taken.is_some_and(|t| t.contains_key(&LIKE)) {
                "action_already_performed"
            } else {
                "invalid_access"
            },
        ));
    }

    // create_post_action: a trashed like that was never reviewed comes
    // back (dated now, as action_attrs' created_at sets it), else a new one.
    let revived: Option<(i32, chrono::NaiveDateTime)> = sqlx::query_as(
        "UPDATE post_actions SET deleted_at = NULL, deleted_by_id = NULL, staff_took_action = FALSE, \
                related_post_id = NULL, targets_topic = FALSE, created_at = clock_timestamp(), \
                updated_at = clock_timestamp() \
         WHERE id = (SELECT id FROM post_actions WHERE post_id = $1 AND user_id = $2 AND post_action_type_id = $3 \
                       AND deleted_at IS NOT NULL AND agreed_at IS NULL AND disagreed_at IS NULL \
                       AND deferred_at IS NULL ORDER BY id LIMIT 1) \
         RETURNING id, created_at",
    )
    .bind(post.id)
    .bind(user.id)
    .bind(LIKE as i32)
    .fetch_optional(&mut *tx)
    .await?;
    let (action_id, created_at) = match revived {
        Some(row) => row,
        None => sqlx::query_as(
            "INSERT INTO post_actions (post_id, user_id, post_action_type_id, staff_took_action, targets_topic, \
                                       created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id, created_at",
        )
        .bind(post.id)
        .bind(user.id)
        .bind(LIKE as i32)
        .fetch_one(&mut *tx)
        .await?,
    };
    // GivenDailyLike.increment_for
    let updated = sqlx::query(&format!(
        "UPDATE given_daily_likes SET likes_given = likes_given + 1 WHERE user_id = $1 AND given_date = {TODAY}"
    ))
    .bind(user.id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        sqlx::query(&format!(
            "INSERT INTO given_daily_likes (user_id, given_date, likes_given, limit_reached) VALUES ($1, {TODAY}, 1, FALSE)"
        ))
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(&format!(
            "UPDATE given_daily_likes SET limit_reached = TRUE WHERE user_id = $1 AND given_date = {TODAY} \
             AND NOT limit_reached AND likes_given >= $2"
        ))
        .bind(user.id)
        .bind(ctx.settings.get("max_likes_per_day")?.to_i() as i32)
        .execute(&mut *tx)
        .await?;
    }
    // after_save update_counters
    update_counters(&mut tx, ctx, post.id, user.id, access.topic_id).await?;
    // UserActionManager.post_action_created: UserAction.log_action! per row.
    for (action_type, user_id) in action_rows(user.id, post.user_id) {
        let inserted = sqlx::query(
            "INSERT INTO user_actions (action_type, user_id, acting_user_id, target_topic_id, target_post_id, \
                                       created_at, updated_at) \
             SELECT $1, $2, $3, $4, $5, $6, clock_timestamp() \
             WHERE NOT EXISTS (SELECT 1 FROM user_actions WHERE action_type = $1 AND user_id = $2 \
               AND acting_user_id = $3 AND target_topic_id = $4 AND target_post_id = $5)",
        )
        .bind(action_type)
        .bind(user_id)
        .bind(user.id)
        .bind(access.topic_id)
        .bind(post.id)
        .bind(created_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted > 0 {
            update_like_count(&mut tx, user_id, action_type, 1).await?;
        }
    }
    // PostActionNotifier.post_action_created
    crate::jobs::post_alert::notify_liked(
        ctx,
        &mut tx,
        post.id,
        user.id,
        &user.username,
        action_id,
    )
    .await?;
    tx.commit().await?;
    Ok(Outcome::Done)
}

/// `PostActionDestroyer.new(user, post, like).perform`
pub async fn unlike(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Outcome, AppError> {
    let user = guardian
        .user()
        .ok_or(Unsupported("unliking anonymously"))?
        .clone();
    if guardian.is_staff() {
        return Err(Unsupported("unliking as staff (deleted likes and posts)").into());
    }
    let mut tx = pool.begin().await?;
    // Post.find_by(id:): the destroyer does not check visibility first.
    let post: Option<(i32, Option<i32>, i32, i32)> = sqlx::query_as(
        "SELECT id, user_id, topic_id, post_number FROM posts WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(post_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((post_id, author, topic_id, post_number)) = post else {
        return Ok(Outcome::NotFound);
    };
    let action: Option<(i32, i32, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT id, user_id, created_at FROM post_actions WHERE user_id = $1 AND post_id = $2 \
         AND post_action_type_id = $3 AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(user.id)
    .bind(post_id)
    .bind(LIKE as i32)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((action_id, action_user, action_created_at)) = action else {
        return Ok(Outcome::NotFound);
    };
    let topic = crate::topic_guardian::TopicCtx::load(&mut tx, ctx.settings, guardian, topic_id)
        .await?
        .ok_or(Unsupported("likes on posts of deleted topics"))?;
    if topic.private_message() {
        return Err(Unsupported("likes in messages").into());
    }
    if !guardian.can_delete_post_action(ctx.settings, &topic, action_user, action_created_at)? {
        return Ok(Outcome::Forbidden("invalid_access"));
    }
    // remove_act!: trash!, then save's update_counters.
    sqlx::query(
        "UPDATE post_actions SET deleted_at = clock_timestamp(), deleted_by_id = $2, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(action_id)
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    update_counters(&mut tx, ctx, post_id, user.id, topic_id).await?;
    // GivenDailyLike.decrement_for
    sqlx::query(&format!(
        "UPDATE given_daily_likes SET likes_given = likes_given - 1 WHERE user_id = $1 AND given_date = {TODAY}"
    ))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(&format!(
        "UPDATE given_daily_likes SET limit_reached = FALSE WHERE user_id = $1 AND given_date = {TODAY} \
         AND limit_reached AND likes_given < $2"
    ))
    .bind(user.id)
    .bind(ctx.settings.get("max_likes_per_day")?.to_i() as i32)
    .execute(&mut *tx)
    .await?;
    // UserActionManager.post_action_destroyed: UserAction.remove_action!
    for (action_type, user_id) in action_rows(user.id, author) {
        sqlx::query(
            "DELETE FROM user_actions WHERE id = (SELECT id FROM user_actions WHERE action_type = $1 \
               AND user_id = $2 AND acting_user_id = $3 AND target_topic_id = $4 AND target_post_id = $5 \
               ORDER BY id LIMIT 1)",
        )
        .bind(action_type)
        .bind(user_id)
        .bind(user.id)
        .bind(topic_id)
        .bind(post_id)
        .execute(&mut *tx)
        .await?;
        update_like_count(&mut tx, user_id, action_type, -1).await?;
    }
    // PostActionNotifier.post_action_deleted: the author's liked
    // notifications on the post go, then refresh_like_notification
    // rebuilds one from the likes of the last day.
    if let Some(author) = author {
        let reads: Vec<bool> = sqlx::query_scalar(
            "DELETE FROM notifications WHERE topic_id = $1 AND user_id = $2 AND post_number = $3 \
             AND notification_type = 5 RETURNING read",
        )
        .bind(topic_id)
        .bind(author)
        .bind(post_number)
        .fetch_all(&mut *tx)
        .await?;
        let read = reads.iter().all(|r| *r);
        let likers: Vec<String> = sqlx::query_scalar(
            "SELECT u.username FROM post_actions pa JOIN users u ON u.id = pa.user_id \
             WHERE pa.post_id = $1 AND pa.post_action_type_id = $2 AND pa.deleted_at IS NULL \
               AND pa.created_at > now() - interval '1 day' ORDER BY pa.created_at DESC",
        )
        .bind(post_id)
        .bind(LIKE as i32)
        .fetch_all(&mut *tx)
        .await?;
        if !likers.is_empty() {
            let _ = read;
            return Err(Unsupported("rebuilding the liked notification from other likers").into());
        }
    }
    tx.commit().await?;
    Ok(Outcome::Done)
}
