//! Chat::ReviewableMessage's actions from the review queue (PUT
//! /review/:id/perform/:action_id, which review.rs hands this type):
//! build_combined_actions as the check of the action asked, its aliases
//! (agree_and_suspend, agree_and_silence, agree_and_keep_hidden and
//! delete_and_agree are agree_and_delete), the perform methods (the
//! message trashed or recovered, the author unsilenced), then
//! Reviewable#perform's transition, flag stats and score.

use serde_json::json;
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::review::Outcome;
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
/// `UserHistory.actions[:silence_user]`
const SILENCE_USER: i32 = 30;
/// `Jobs::TruncateUserFlagStats.truncate_to`
const FLAG_STATS_TRUNCATE_TO: i64 = 100;

/// `Chat::ReviewableMessage.action_aliases`
fn alias(action: &str) -> &str {
    match action {
        "agree_and_keep_hidden"
        | "agree_and_silence"
        | "agree_and_suspend"
        | "delete_and_agree" => "agree_and_delete",
        other => other,
    }
}

/// The settled result: the reviewable's new status, the flaggers' stat
/// and whether the score is recalculated.
struct Settled {
    status: i32,
    score_status: i32,
    stat: &'static str,
    recalculate: bool,
}

/// `Reviewable#perform` for a Chat::ReviewableMessage.
pub async fn perform(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    reviewable_id: i64,
    action_id: &str,
    version: i64,
) -> Result<Outcome, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(Outcome::Forbidden);
    };
    let (status, target_id): (i32, Option<i32>) =
        sqlx::query_as("SELECT status, target_id FROM reviewables WHERE id = $1")
            .bind(reviewable_id)
            .fetch_one(&mut *conn)
            .await?;
    // chat_message: with_deleted.
    let message: Option<(i64, Option<i32>, bool)> = match target_id {
        Some(id) => {
            sqlx::query_as(
                "SELECT id, user_id, deleted_at IS NOT NULL FROM chat_messages WHERE id = $1",
            )
            .bind(i64::from(id))
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let creator: Option<(i32, bool, bool, bool)> = match message.and_then(|m| m.1) {
        Some(id) => sqlx::query_as(
            "SELECT id, admin, moderator, silenced_till IS NOT NULL AND silenced_till > now() FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?,
        None => None,
    };

    // build_combined_actions: nothing unless pending.
    let mut actions: Vec<&str> = Vec::new();
    if status == PENDING {
        match message {
            None => actions.push("ignore"),
            Some((_, _, deleted)) => {
                if deleted {
                    actions.extend(["agree_and_restore", "agree_and_keep_deleted"]);
                } else {
                    actions.extend(["agree_and_delete", "agree_and_keep_message"]);
                }
                // can_suspend?: staff, over a user who isn't an admin (nor
                // staff, for a moderator).
                if let Some((_, admin, moderator, _)) = creator
                    && guardian.is_staff()
                    && !admin
                    && (guardian.is_admin() || !moderator)
                {
                    actions.extend(["agree_and_suspend", "agree_and_silence"]);
                }
                actions.push(if deleted {
                    "disagree_and_restore"
                } else {
                    "disagree"
                });
                actions.push("ignore");
                if !deleted {
                    actions.push("delete_and_agree");
                }
                // build_unsilence_action: can_unsilence_user?
                if let Some((_, admin, moderator, true)) = creator
                    && guardian.is_staff()
                    && (!(admin || moderator) || guardian.is_admin())
                {
                    actions.push("unsilence_user");
                }
            }
        }
    }
    // validate_action!: the action or one of its aliases.
    let method = alias(action_id);
    let allowed = actions
        .iter()
        .any(|a| *a == action_id || (alias(a) == action_id && *a != action_id));
    if !allowed {
        return Ok(Outcome::Forbidden);
    }

    // increment_version!
    let bumped: Option<i32> = sqlx::query_scalar(
        "UPDATE reviewables SET version = version + 1 WHERE id = $1 AND version = $2 RETURNING version",
    )
    .bind(reviewable_id)
    .bind(version as i32)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(new_version) = bumped else {
        return Ok(Outcome::Conflict);
    };

    let message_id = message.map(|m| m.0);
    let agree = Settled {
        status: APPROVED,
        score_status: SCORE_AGREED,
        stat: "flags_agreed",
        recalculate: true,
    };
    let disagree = Settled {
        status: REJECTED,
        score_status: SCORE_DISAGREED,
        stat: "flags_disagreed",
        recalculate: true,
    };
    let settled = match method {
        "agree_and_keep_message" | "agree_and_keep_deleted" => Some(agree),
        "agree_and_restore" => {
            recover(&mut *conn, message_id).await?;
            Some(agree)
        }
        "agree_and_delete" => {
            trash(&mut *conn, message_id, user.id).await?;
            Some(agree)
        }
        "disagree" | "disagree_and_restore" => {
            if method == "disagree_and_restore" {
                recover(&mut *conn, message_id).await?;
            }
            if silenced_for(&mut *conn, creator.map(|c| (c.0, c.3)), reviewable_id).await? {
                let author = creator.map(|c| c.0).unwrap_or_default();
                crate::user_penalties::lift_silence_for_reviewable(
                    &mut *conn,
                    ctx,
                    user.id,
                    author,
                    reviewable_id,
                )
                .await?;
            }
            Some(disagree)
        }
        "ignore" => Some(Settled {
            status: IGNORED,
            score_status: SCORE_IGNORED,
            stat: "flags_ignored",
            recalculate: false,
        }),
        "unsilence_user" => {
            let author = creator.map(|c| c.0).unwrap_or_default();
            crate::user_penalties::lift_silence_for_reviewable(
                &mut *conn,
                ctx,
                user.id,
                author,
                reviewable_id,
            )
            .await?;
            None
        }
        _ => return Err(Unsupported("chat reviewable actions not ported").into()),
    };

    let hooks = crate::web_hooks::Payloads {
        settings: ctx.settings,
        i18n: ctx.i18n,
        config: ctx.config,
    };
    let mut status_changed = false;
    if let Some(settled) = &settled {
        // transition_to: no topic to release a claim of.
        sqlx::query(
            "UPDATE reviewables SET status = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(reviewable_id)
        .bind(settled.status)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                               edited, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
        )
        .bind(reviewable_id)
        .bind(HISTORY_TRANSITIONED)
        .bind(settled.status)
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
        crate::web_hooks::enqueue_reviewable_hooks(
            &mut *conn,
            &hooks,
            "reviewable_transitioned_to",
            reviewable_id,
        )
        .await?;
        sqlx::query(
            "UPDATE reviewable_scores SET status = $2, reviewed_by_id = $3, reviewed_at = clock_timestamp() \
             WHERE reviewable_id = $1 AND status IN ($4, $5)",
        )
        .bind(reviewable_id)
        .bind(settled.score_status)
        .bind(user.id)
        .bind(PENDING)
        .bind(SCORE_DISAGREED)
        .execute(&mut *conn)
        .await?;
        status_changed = status == PENDING;
        // update_flag_stats over the flaggers (flagged_by_user_ids).
        let totals: Vec<i64> = sqlx::query_scalar(&format!(
            "UPDATE user_stats SET {stat} = {stat} + 1 WHERE user_id IN \
               (SELECT user_id FROM reviewable_scores WHERE reviewable_id = $1) \
             RETURNING (flags_agreed + flags_disagreed + flags_ignored)::int8",
            stat = settled.stat
        ))
        .bind(reviewable_id)
        .fetch_all(&mut *conn)
        .await?;
        if totals.iter().any(|t| *t > FLAG_STATS_TRUNCATE_TO) {
            return Err(Unsupported("truncating flag stats (truncate_user_flag_stats)").into());
        }
        if settled.recalculate {
            reviewables::recalculate_score(&mut *conn, &hooks, reviewable_id).await?;
        }
    }
    // An action that leaves it pending didn't resolve it.
    let remove: Vec<i64> = if settled.is_some() {
        vec![reviewable_id]
    } else {
        Vec::new()
    };
    if status_changed || !remove.is_empty() {
        crate::jobs::enqueue(
            &mut *conn,
            "notify_reviewable",
            json!({
                "reviewable_id": reviewable_id,
                "performing_username": user.username,
                "updated_reviewable_ids": remove,
            }),
        )
        .await?;
    }
    let (count, unseen) = reviewables::staff_counts(&mut *conn, ctx.settings, guardian).await?;
    Ok(Outcome::Done(json!({
        "reviewable_perform_result": {
            "success": true,
            "remove_reviewable_ids": remove,
            "version": new_version,
            "reviewable_count": count,
            "unseen_reviewable_count": unseen,
        }
    })))
}

/// `silenced_for_this_message?`: the author is silenced, by a silence
/// this reviewable brought.
async fn silenced_for(
    conn: &mut PgConnection,
    creator: Option<(i32, bool)>,
    reviewable_id: i64,
) -> Result<bool, AppError> {
    let Some((user_id, true)) = creator else {
        return Ok(false);
    };
    let latest: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT reviewable_id FROM user_histories WHERE target_user_id = $1 AND action = $2 \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(SILENCE_USER)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(latest.flatten() == Some(reviewable_id))
}

/// `chat_message.trash!(performed_by)`
async fn trash(
    conn: &mut PgConnection,
    message_id: Option<i64>,
    by: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE chat_messages SET deleted_at = clock_timestamp(), deleted_by_id = $2 WHERE id = $1",
    )
    .bind(message_id)
    .bind(by)
    .execute(conn)
    .await?;
    Ok(())
}

/// `chat_message.recover!`
async fn recover(conn: &mut PgConnection, message_id: Option<i64>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE chat_messages SET deleted_at = NULL, deleted_by_id = NULL WHERE id = $1")
        .bind(message_id)
        .execute(conn)
        .await?;
    Ok(())
}
