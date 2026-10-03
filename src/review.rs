//! Acting on the review queue: `Reviewable#perform` for a
//! ReviewableFlaggedPost (agree and keep, agree and hide, disagree,
//! ignore), with the flag actions it settles, the transition, the flaggers'
//! stats, and `Post#hide!`.
//!
//! Refused: other reviewable types, posts that are deleted or hidden
//! (restoring and unhiding), deleting posts, silencing or suspending from
//! the queue, editing, deleting users, claiming, flags with a message
//! (their moderator replies) and category group moderation.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::post_actions::ActionTypes;
use crate::posting::Ctx;
use crate::reviewables::{self, PENDING};
use crate::{AppError, Unsupported};

/// `Reviewable.statuses`
const APPROVED: i32 = 1;
const REJECTED: i32 = 2;
const IGNORED: i32 = 3;
/// `ReviewableHistory.types[:transitioned]`
const HISTORY_TRANSITIONED: i32 = 1;
/// `ReviewableScore.statuses`
const SCORE_AGREED: i32 = 1;
const SCORE_DISAGREED: i32 = 2;
const SCORE_IGNORED: i32 = 3;
/// `Post.hidden_reasons`
const FLAG_THRESHOLD_REACHED: i32 = 1;
const FLAG_THRESHOLD_REACHED_AGAIN: i32 = 2;
/// `Post.types`
const REGULAR: i32 = 1;
const WHISPER: i32 = 4;
/// `Jobs::TruncateUserFlagStats.truncate_to`
const FLAG_STATS_TRUNCATE_TO: i64 = 100;

/// The ported perform methods of ReviewableFlaggedPost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlagAction {
    AgreeAndKeep,
    AgreeAndHide,
    Disagree,
    Ignore,
}

/// How a perform request ends when it isn't a server error.
pub enum Outcome {
    /// The ReviewablePerformResultSerializer body.
    Done(Value),
    /// `Discourse::NotFound`
    NotFound,
    /// `Reviewable::InvalidAction` or not being able to see the queue:
    /// `Discourse::InvalidAccess`.
    Forbidden,
    /// `Reviewable::UpdateConflict`
    Conflict,
}

/// The perform method an action id runs, given whether the post is
/// hidden, as `validate_action!` finds it among the built actions and
/// their aliases. None for an action that isn't built.
fn flag_action(action_id: &str, hidden: bool) -> Result<Option<FlagAction>, Unsupported> {
    Ok(match action_id {
        "agree_and_keep" => Some(FlagAction::AgreeAndKeep),
        "agree_and_keep_hidden" => hidden.then_some(FlagAction::AgreeAndKeep),
        "agree_and_hide" => (!hidden).then_some(FlagAction::AgreeAndHide),
        "disagree" | "ignore_and_do_nothing" | "ignore" if hidden => {
            return Err(Unsupported(
                "disagreeing with or ignoring flags on hidden posts",
            ));
        }
        "disagree" => Some(FlagAction::Disagree),
        "ignore_and_do_nothing" | "ignore" => Some(FlagAction::Ignore),
        "agree_and_edit"
        | "agree_and_silence"
        | "agree_and_suspend"
        | "agree_and_restore"
        | "agree_and_keep_deleted"
        | "disagree_and_restore"
        | "disagree_and_keep_deleted"
        | "delete_and_agree"
        | "delete_and_agree_replies"
        | "delete_and_ignore"
        | "delete_and_ignore_replies"
        | "delete_user"
        | "delete_user_block"
        | "delete_and_block_user"
        | "unsilence_user"
        | "unsilence_user_and_ignore" => {
            return Err(Unsupported(
                "review actions that delete, edit, restore or penalize",
            ));
        }
        _ => None,
    })
}

/// The reviewable a perform is about.
#[derive(sqlx::FromRow)]
struct Reviewable {
    id: i64,
    #[sqlx(rename = "type")]
    kind: String,
    status: i32,
    target_id: Option<i32>,
    topic_id: Option<i32>,
    reviewable_by_moderator: bool,
}

/// The flagged post as the actions read it.
#[derive(sqlx::FromRow)]
struct FlaggedPost {
    id: i32,
    user_id: Option<i32>,
    topic_id: i32,
    post_number: i32,
    post_type: i32,
    hidden: bool,
    hidden_at: Option<chrono::NaiveDateTime>,
    deleted_at: Option<chrono::NaiveDateTime>,
    user_deleted: bool,
}

async fn flagged_post(
    conn: &mut PgConnection,
    id: i32,
) -> Result<Option<FlaggedPost>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, user_id, topic_id, post_number, post_type, hidden, hidden_at, deleted_at, user_deleted \
         FROM posts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await
}

/// `ReviewablesController#perform` with the given version.
pub async fn perform(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    reviewable_id: i64,
    action_id: &str,
    version: i64,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let Some(user) = guardian.user() else {
        return Ok(Outcome::Forbidden);
    };
    // ensure_can_see_review_queue!
    if s.get("enable_category_group_moderation")?.truthy() {
        return Err(Unsupported("category group moderation").into());
    }
    if !guardian.is_staff() {
        return Ok(Outcome::Forbidden);
    }
    // find_reviewable: Reviewable.viewable_by(current_user)
    let reviewable: Option<Reviewable> = sqlx::query_as(
        "SELECT id, type, status, target_id, topic_id, reviewable_by_moderator FROM reviewables WHERE id = $1",
    )
    .bind(reviewable_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(reviewable) = reviewable else {
        return Ok(Outcome::NotFound);
    };
    if !guardian.is_admin() && !reviewable.reviewable_by_moderator {
        return Err(Unsupported("reviewables for groups").into());
    }
    // claim_error?
    if s.get("reviewable_claiming")?.to_s() != "disabled" && reviewable.topic_id.is_some() {
        return Err(Unsupported("reviewable claiming").into());
    }
    if reviewable.kind != "ReviewableFlaggedPost" {
        return Err(Unsupported("reviewable types other than flagged posts").into());
    }
    let post = match reviewable.target_id {
        Some(id) => flagged_post(&mut *conn, id).await?,
        None => None,
    };
    // build_actions: nothing unless pending, with a post.
    let Some(post) = post.filter(|_| reviewable.status == PENDING) else {
        return Ok(Outcome::Forbidden);
    };
    if post.deleted_at.is_some() || post.user_deleted {
        return Err(Unsupported("reviewing flags on deleted posts").into());
    }
    let Some(action) = flag_action(action_id, post.hidden)? else {
        return Ok(Outcome::Forbidden);
    };
    let Some(version) = run(
        conn,
        ctx,
        user.id,
        &user.username,
        &reviewable,
        &post,
        action,
        Some(version),
    )
    .await?
    else {
        return Ok(Outcome::Conflict);
    };
    let (count, unseen) =
        reviewables::staff_counts(&mut *conn, s, user.id, user.admin, user.moderator).await?;
    Ok(Outcome::Done(json!({
        "reviewable_perform_result": {
            "success": true,
            "remove_reviewable_ids": [reviewable.id],
            "version": version,
            "reviewable_count": count,
            "unseen_reviewable_count": unseen,
        }
    })))
}

/// `Reviewable#perform(performed_by, action)` for a pending flagged post
/// on a live post: the version, the perform method, the transition and
/// the flag stats, then notify_reviewable. None on a version conflict.
#[allow(clippy::too_many_arguments)]
async fn run(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    performer_id: i32,
    performer_username: &str,
    reviewable: &Reviewable,
    post: &FlaggedPost,
    action: FlagAction,
    version: Option<i64>,
) -> Result<Option<i64>, AppError> {
    // increment_version!
    let version: Option<i32> = match version {
        Some(v) => sqlx::query_scalar(
            "UPDATE reviewables SET version = version + 1 WHERE id = $1 AND version = $2 RETURNING version",
        )
        .bind(reviewable.id)
        .bind(v as i32),
        None => sqlx::query_scalar(
            "UPDATE reviewables SET version = version + 1 WHERE id = $1 RETURNING version",
        )
        .bind(reviewable.id),
    }
    .fetch_optional(&mut *conn)
    .await?;
    let Some(version) = version else {
        return Ok(None);
    };

    let types = ActionTypes::load(&mut *conn).await?;
    // PostAction.active on the post, of the types that went to review.
    let actions: Vec<(i32, i32, i32, Option<i32>)> = sqlx::query_as(
        "SELECT id, user_id, post_action_type_id, related_post_id FROM post_actions \
         WHERE post_id = $1 AND agreed_at IS NULL AND disagreed_at IS NULL AND deferred_at IS NULL \
           AND deleted_at IS NULL ORDER BY id",
    )
    .bind(post.id)
    .fetch_all(&mut *conn)
    .await?;
    let actions: Vec<_> = actions
        .into_iter()
        .filter(|(_, _, t, _)| types.is_notify_flag(i64::from(*t)))
        .collect();
    if actions.iter().any(|(_, _, _, related)| related.is_some()) {
        return Err(Unsupported("settling flags that sent a message (moderator replies)").into());
    }
    let (column, transition, score_status, stat) = match action {
        FlagAction::AgreeAndKeep | FlagAction::AgreeAndHide => {
            ("agreed", APPROVED, SCORE_AGREED, "flags_agreed")
        }
        FlagAction::Disagree => ("disagreed", REJECTED, SCORE_DISAGREED, "flags_disagreed"),
        FlagAction::Ignore => ("deferred", IGNORED, SCORE_IGNORED, "flags_ignored"),
    };
    for (id, _, type_id, _) in &actions {
        sqlx::query(&format!(
            "UPDATE post_actions SET {column}_at = clock_timestamp(), {column}_by_id = $2, \
                                     updated_at = clock_timestamp() WHERE id = $1"
        ))
        .bind(id)
        .bind(performer_id)
        .execute(&mut *conn)
        .await?;
        // after_save update_counters
        crate::flags::update_counters(&mut *conn, &types, post.id, i64::from(*type_id)).await?;
    }
    match action {
        FlagAction::AgreeAndHide => {
            if let Some((_, _, type_id, _)) = actions.first() {
                hide(&mut *conn, ctx, &types, post, i64::from(*type_id)).await?;
            }
        }
        FlagAction::Disagree => {
            // The post's counts of every flag that goes to review, zeroed.
            let zeroed: Vec<String> = types
                .types
                .iter()
                .filter(|(_, id)| types.is_notify_flag(*id))
                .map(|(key, _)| format!("{key}_count = 0"))
                .collect();
            sqlx::query(&format!(
                "UPDATE posts SET {} WHERE id = $1",
                zeroed.join(", ")
            ))
            .bind(post.id)
            .execute(&mut *conn)
            .await?;
        }
        FlagAction::AgreeAndKeep | FlagAction::Ignore => {}
    }

    // transition_to: the status saved, its history, the scores settled.
    sqlx::query("UPDATE reviewables SET status = $2, updated_at = clock_timestamp() WHERE id = $1")
        .bind(reviewable.id)
        .bind(transition)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(reviewable.id)
    .bind(HISTORY_TRANSITIONED)
    .bind(transition)
    .bind(performer_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE reviewable_scores SET status = $2, reviewed_by_id = $3, reviewed_at = clock_timestamp() \
         WHERE reviewable_id = $1 AND status IN ($4, $5)",
    )
    .bind(reviewable.id)
    .bind(score_status)
    .bind(performer_id)
    .bind(PENDING)
    .bind(SCORE_DISAGREED)
    .execute(&mut *conn)
    .await?;

    // update_flag_stats, self-flags not counted.
    let flaggers: Vec<i32> = actions
        .iter()
        .map(|(_, user_id, _, _)| *user_id)
        .filter(|id| Some(*id) != post.user_id)
        .collect();
    if !flaggers.is_empty() {
        let totals: Vec<i64> = sqlx::query_scalar(&format!(
            "UPDATE user_stats SET {stat} = {stat} + 1 WHERE user_id = ANY($1) \
             RETURNING (flags_agreed + flags_disagreed + flags_ignored)::int8"
        ))
        .bind(&flaggers)
        .fetch_all(&mut *conn)
        .await?;
        if totals.iter().any(|t| *t > FLAG_STATS_TRUNCATE_TO) {
            return Err(Unsupported("truncating flag stats (truncate_user_flag_stats)").into());
        }
    }

    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({
            "reviewable_id": reviewable.id,
            "performing_username": performer_username,
            "updated_reviewable_ids": [reviewable.id],
        }),
    )
    .await?;
    Ok(Some(i64::from(version)))
}

/// `post.reviewable_flag.perform(created_by, :agree_and_keep)`: the
/// agreeing that taking action on a flag does, without a version.
pub async fn agree_with_flags(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    performer_id: i32,
    performer_username: &str,
    post_id: i32,
) -> Result<(), AppError> {
    let reviewable: Option<Reviewable> = sqlx::query_as(
        "SELECT id, type, status, target_id, topic_id, reviewable_by_moderator FROM reviewables \
         WHERE type = 'ReviewableFlaggedPost' AND status = $2 AND target_type = 'Post' AND target_id = $1",
    )
    .bind(post_id)
    .bind(PENDING)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(reviewable) = reviewable else {
        return Ok(());
    };
    let post = flagged_post(&mut *conn, post_id)
        .await?
        .ok_or(Unsupported("agreeing with flags on a missing post"))?;
    if post.deleted_at.is_some() || post.user_deleted {
        return Err(Unsupported("reviewing flags on deleted posts").into());
    }
    run(
        conn,
        ctx,
        performer_id,
        performer_username,
        &reviewable,
        &post,
        FlagAction::AgreeAndKeep,
        None,
    )
    .await?;
    Ok(())
}

/// `Post#hide!(post_action_type_id)` for a reply: hidden with the reason,
/// the author's post count down, the post_hidden system message, and the
/// topic's bumped_at reset when it was the last reply. Hiding a first
/// post, or the last visible one, hides the topic, which is not ported.
pub async fn hide_post(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    types: &ActionTypes,
    post_id: i32,
    type_id: i64,
) -> Result<(), AppError> {
    let post = flagged_post(&mut *conn, post_id)
        .await?
        .ok_or(Unsupported("hiding a missing post"))?;
    hide(conn, ctx, types, &post, type_id).await
}

async fn hide(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    types: &ActionTypes,
    post: &FlaggedPost,
    type_id: i64,
) -> Result<(), AppError> {
    if post.hidden {
        return Ok(());
    }
    let hiding_again = post.hidden_at.is_some();
    let reason = if hiding_again {
        FLAG_THRESHOLD_REACHED_AGAIN
    } else {
        FLAG_THRESHOLD_REACHED
    };
    let (highest, slug): (i32, Option<String>) =
        sqlx::query_as("SELECT highest_post_number, slug FROM topics WHERE id = $1")
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    // is_last_reply? && !whisper?
    let reset_bumped_at =
        highest == post.post_number && post.post_number != 1 && post.post_type != WHISPER;
    sqlx::query(
        "UPDATE posts SET hidden = TRUE, hidden_at = clock_timestamp(), hidden_reason_id = $2, \
                          updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(post.id)
    .bind(reason)
    .execute(&mut *conn)
    .await?;
    let any_visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND NOT hidden AND post_type = $2 \
                        AND deleted_at IS NULL)",
    )
    .bind(post.topic_id)
    .bind(REGULAR)
    .fetch_one(&mut *conn)
    .await?;
    if post.post_number == 1 || !any_visible {
        return Err(Unsupported(
            "hiding a topic's first or last visible post (unlisting the topic)",
        )
        .into());
    }
    // UserStatCountUpdater.decrement!: a regular reply's post count, not
    // below zero.
    if let Some(user_id) = post.user_id
        && post.post_type == REGULAR
    {
        sqlx::query("UPDATE user_stats SET post_count = post_count - 1 WHERE user_id = $1 AND post_count >= 1")
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
    }
    // Inform the author.
    let author_exists = match post.user_id {
        Some(id) => {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
        None => false,
    };
    if let (true, Some(user_id)) = (author_exists, post.user_id) {
        let base_path = ctx.config.globals.relative_url_root();
        let key = types.name(type_id).unwrap_or_default();
        let flag_reason = ctx
            .i18n
            .t_with(&format!("flag_reasons.{key}"), &[("base_path", base_path)])
            .ok_or(Unsupported("flag reasons without a translation"))?;
        let url = format!(
            "{base_path}/t/{}/{}/{}",
            slug.as_deref().filter(|s| !s.is_empty()).unwrap_or("topic"),
            post.topic_id,
            post.post_number
        );
        crate::jobs::enqueue_in(
            &mut *conn,
            5,
            "send_system_message",
            json!({
                "user_id": user_id,
                "message_type": if hiding_again { "post_hidden_again" } else { "post_hidden" },
                "message_options": {
                    "url": url,
                    "edit_delay": ctx.settings.get("cooldown_minutes_after_hiding_posts")?.to_i(),
                    "flag_reason": flag_reason,
                },
            }),
        )
        .await?;
    }
    if reset_bumped_at {
        // Topic#reset_bumped_at: the last visible regular post's date (one
        // exists, checked above); saved without validation when it changed.
        sqlx::query(
            "UPDATE topics t SET bumped_at = p.created_at, updated_at = clock_timestamp() \
             FROM (SELECT created_at FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
                     AND NOT user_deleted AND NOT hidden AND post_type = $2 \
                   ORDER BY sort_order DESC LIMIT 1) p \
             WHERE t.id = $1 AND t.bumped_at IS DISTINCT FROM p.created_at",
        )
        .bind(post.topic_id)
        .bind(REGULAR)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
