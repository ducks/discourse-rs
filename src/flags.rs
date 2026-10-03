//! Flags: PostActionCreator for the flag types that go to review without
//! a message (off topic, inappropriate, spam): the post action and the
//! post's count of it (PostAction#update_counters), the post's
//! ReviewableFlaggedPost and the flag's score, and the rules a flag is held
//! to (auto close, auto hide, auto silence). Staff taking action hide the
//! post and agree with its flags.
//!
//! Refused: flags that send a message (notify user, notify moderators,
//! illegal), queueing for review as staff, flagging topics, flags on posts
//! already in review, flags in messages, and the rules when they would
//! close the topic, hide on a trusted spam flag or silence the author. The
//! flag rate limit (RateLimiter, Redis) and the badge queue are not
//! ported; flags write no user actions or notifications.

use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::likes::Outcome;
use crate::post_actions::{self, ActOpts, ActionTypes};
use crate::posting::Ctx;
use crate::posting::revisions::find_post;
use crate::reviewables::{self, Flagger, NewFlaggedPost, NewScore};
use crate::{AppError, Unsupported};

/// What a flag request asked for beyond the type.
pub struct FlagRequest {
    pub type_id: i64,
    pub take_action: bool,
    pub queue_for_review: bool,
    pub message: Option<String>,
}

/// The post's author as the rules read them.
struct Author {
    staff: bool,
    staged: bool,
    trust_level: i32,
}

async fn author(
    conn: &mut PgConnection,
    user_id: Option<i32>,
) -> Result<Option<Author>, sqlx::Error> {
    let Some(id) = user_id else {
        return Ok(None);
    };
    let row: Option<(bool, bool, bool, i32)> =
        sqlx::query_as("SELECT admin, moderator, staged, trust_level FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await?;
    Ok(row.map(|(admin, moderator, staged, trust_level)| Author {
        staff: admin || moderator,
        staged,
        trust_level,
    }))
}

/// `PostAction#update_counters` for a flag: the post's `<type>_count`.
pub async fn update_counters(
    conn: &mut PgConnection,
    types: &ActionTypes,
    post_id: i32,
    type_id: i64,
) -> Result<(), AppError> {
    let name = types
        .name(type_id)
        .ok_or(Unsupported("counting an unknown post action type"))?;
    sqlx::query(&format!(
        "UPDATE posts SET {name}_count = (SELECT COUNT(*) FROM post_actions WHERE post_id = $1 \
                                          AND post_action_type_id = $2 AND deleted_at IS NULL) \
         WHERE id = $1"
    ))
    .bind(post_id)
    .bind(type_id as i32)
    .execute(conn)
    .await?;
    Ok(())
}

/// `PostActionCreator.new(user, post, type, ...).perform` for a flag.
pub async fn flag(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    req: &FlagRequest,
) -> Result<Outcome, AppError> {
    let user = guardian
        .user()
        .ok_or(Unsupported("flagging anonymously"))?
        .clone();
    let staff = guardian.is_staff();
    let mut tx = pool.begin().await?;
    // @created_at = Time.zone.now, when the creator is made.
    let created_at: chrono::NaiveDateTime =
        sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
            .fetch_one(&mut *tx)
            .await?;
    let Some(access) = find_post(&mut tx, ctx, guardian, post_id).await? else {
        return Ok(Outcome::NotFound);
    };
    if access.topic.private_message() {
        return Err(Unsupported("flags in messages").into());
    }
    let post = &access.post;
    let types = ActionTypes::load(&mut tx).await?;
    let Some(name) = types.name(req.type_id).map(str::to_string) else {
        return Err(Unsupported("post action types that are neither likes nor flags").into());
    };
    if types.requires_message(req.type_id)
        || req.message.as_deref().is_some_and(|m| !m.trim().is_empty())
    {
        return Err(Unsupported(
            "flags that send a message (notify user, notify moderators, illegal)",
        )
        .into());
    }
    // @take_action = take_action && guardian.is_staff?
    let take_action = req.take_action && staff;

    let taken = post_actions::taken_actions(&mut tx, &[post.id], Some(user.id)).await?;
    let taken = taken.get(&post.id);
    let post_author = author(&mut tx, post.user_id).await?;
    let can_act = guardian.post_can_act(
        ctx.settings,
        &types,
        (&name, req.type_id),
        &ActOpts {
            topic: &access.topic,
            post,
            taken,
            can_see_post: access.can_see_post,
            author_missing: post_author.is_none(),
        },
    )?;
    if !can_act || (req.queue_for_review && !staff) {
        return Ok(Outcome::Forbidden(
            if taken.is_some_and(|t| t.contains_key(&req.type_id)) {
                "action_already_performed"
            } else {
                "invalid_access"
            },
        ));
    }
    if req.queue_for_review {
        return Err(Unsupported("queueing a post for review as staff").into());
    }
    let in_review: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reviewables WHERE target_type = 'Post' AND target_id = $1)",
    )
    .bind(post.id)
    .fetch_one(&mut *tx)
    .await?;
    if in_review {
        return Err(Unsupported("flags on posts already in review").into());
    }

    // create_post_action: a trashed flag that was never reviewed comes
    // back, else a new one.
    let revived = sqlx::query(
        "UPDATE post_actions SET deleted_at = NULL, deleted_by_id = NULL, staff_took_action = $5, \
                related_post_id = NULL, targets_topic = FALSE, created_at = $4, updated_at = clock_timestamp() \
         WHERE id = (SELECT id FROM post_actions WHERE post_id = $1 AND user_id = $2 AND post_action_type_id = $3 \
                       AND deleted_at IS NOT NULL AND agreed_at IS NULL AND disagreed_at IS NULL \
                       AND deferred_at IS NULL ORDER BY id LIMIT 1)",
    )
    .bind(post.id)
    .bind(user.id)
    .bind(req.type_id as i32)
    .bind(created_at)
    .bind(take_action)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if revived == 0 {
        sqlx::query(
            "INSERT INTO post_actions (post_id, user_id, post_action_type_id, staff_took_action, targets_topic, \
                                       created_at, updated_at) \
             VALUES ($1, $2, $3, $5, FALSE, $4, clock_timestamp())",
        )
        .bind(post.id)
        .bind(user.id)
        .bind(req.type_id as i32)
        .bind(created_at)
        .bind(take_action)
        .execute(&mut *tx)
        .await?;
    }
    // after_save
    update_counters(&mut tx, &types, post.id, req.type_id).await?;

    // create_reviewable, for flags that go to review; none for bots' posts.
    let mut reviewable_score = None;
    if types.is_notify_flag(req.type_id) && post.user_id.is_some_and(|id| id >= 0) {
        let reviewable_id = reviewables::create_flagged_post(
            &mut tx,
            &NewFlaggedPost {
                created_by_id: user.id,
                post_id: post.id,
                topic_id: access.topic_id,
                category_id: access.topic.category_id,
                post_user_id: post.user_id,
                potential_spam: name == "spam",
                potentially_illegal: name == "illegal",
                targets_topic: false,
            },
        )
        .await?;
        let flagger = Flagger {
            id: user.id,
            staff,
            trust_level: user.trust_level,
        };
        reviewables::add_score(
            &mut tx,
            reviewable_id,
            access.topic_id,
            &flagger,
            &NewScore {
                score_type: req.type_id,
                created_at,
                take_action,
            },
            ctx.settings,
        )
        .await?;
        reviewable_score =
            sqlx::query_scalar::<_, f64>("SELECT score FROM reviewables WHERE id = $1")
                .bind(reviewable_id)
                .fetch_optional(&mut *tx)
                .await?;
    }

    enforce_rules(
        &mut tx,
        ctx,
        guardian,
        &access,
        post_author.as_ref(),
        &name,
        req.type_id,
        &types,
        reviewable_score,
        take_action,
    )
    .await?;
    // Taking action agrees with the post's flags, this one included.
    if take_action {
        crate::review::agree_with_flags(&mut tx, ctx, user.id, &user.username, post.id).await?;
        update_counters(&mut tx, &types, post.id, req.type_id).await?;
    }
    tx.commit().await?;
    Ok(Outcome::Done)
}

/// `enforce_rules`: auto close, auto hide, auto silence. Hiding is ported;
/// closing the topic, hiding by a trusted spam flagger and silencing are
/// refused when they would act.
#[allow(clippy::too_many_arguments)]
async fn enforce_rules(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    access: &crate::posting::revisions::PostAccess,
    post_author: Option<&Author>,
    name: &str,
    type_id: i64,
    types: &ActionTypes,
    reviewable_score: Option<f64>,
    take_action: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    // auto_close_if_threshold_reached: Topic#auto_close_threshold_reached?
    let topic_owner_staff = author(&mut *conn, access.topic.user_id)
        .await?
        .is_some_and(|a| a.staff);
    if !access.topic.closed && !topic_owner_staff {
        let min = reviewables::min_score_for_priority(&mut *conn, s).await?;
        let (flaggers, total): (i64, f64) = sqlx::query_as(
            "SELECT COUNT(DISTINCT rs.user_id), COALESCE(SUM(rs.score), 0.0) \
             FROM reviewable_scores rs JOIN reviewables r ON r.id = rs.reviewable_id \
             WHERE rs.status = $1 AND rs.score >= $2 AND r.topic_id = $3",
        )
        .bind(reviewables::PENDING)
        .bind(min)
        .bind(access.topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let to_close =
            reviewables::sensitivity_score(&mut *conn, s, "auto_close_topic_sensitivity", 2.5)
                .await?;
        if flaggers >= s.get("num_flaggers_to_close_topic")?.to_i() && total >= to_close {
            return Err(Unsupported("closing a topic on flags").into());
        }
    }

    // auto_hide_if_needed
    let flagger_staff = guardian.is_staff();
    let author_staff = post_author.is_some_and(|a| a.staff);
    if !access.post.hidden && (flagger_staff || !author_staff) && types.is_auto_action(type_id) {
        let trusted_spam_flagger = s.get("high_trust_flaggers_auto_hide_posts")?.truthy()
            && name == "spam"
            && guardian.has_trust_level(3)
            && post_author.is_some_and(|a| a.trust_level == 0);
        if trusted_spam_flagger {
            return Err(Unsupported("hiding a post flagged as spam by a trusted user").into());
        }
        let to_hide =
            reviewables::sensitivity_score(&mut *conn, s, "hide_post_sensitivity", 1.0).await?;
        if reviewable_score.unwrap_or(0.0) >= to_hide || take_action {
            crate::review::hide_post(&mut *conn, ctx, types, access.post.id, type_id).await?;
        }
    }

    // SpamRule::AutoSilence for the post's author
    if let (Some(author), Some(author_id)) = (post_author, access.post.user_id)
        && !author.staged
        && !(author.staff || author.trust_level >= 1)
        && s.get("num_users_to_silence_new_user")?.to_i() > 0
    {
        let (total, users): (f64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(rs.score), 0)::float8, COUNT(DISTINCT rs.user_id) \
             FROM reviewables r JOIN reviewable_scores rs ON rs.reviewable_id = r.id \
             WHERE r.target_created_by_id = $1 AND rs.reviewable_score_type = $2 \
               AND rs.status IN ($3, $4)",
        )
        .bind(author_id)
        .bind(
            types
                .types
                .iter()
                .find(|(k, _)| k == "spam")
                .map_or(8, |(_, id)| *id) as i32,
        )
        .bind(reviewables::PENDING)
        .bind(reviewables::AGREED)
        .fetch_one(&mut *conn)
        .await?;
        let to_silence =
            reviewables::sensitivity_score(&mut *conn, s, "silence_new_user_sensitivity", 0.6)
                .await?;
        if total >= to_silence && users >= s.get("num_users_to_silence_new_user")?.to_i() {
            return Err(Unsupported("silencing a new user on spam flags").into());
        }
    }
    Ok(())
}
