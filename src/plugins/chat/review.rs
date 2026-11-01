//! Chat::ReviewableMessage in the review queue: build_combined_actions
//! (the queue's bundles and the check of an action asked), its aliases
//! (agree_and_suspend, agree_and_silence, agree_and_keep_hidden and
//! delete_and_agree are agree_and_delete), penalty_effect_for, and
//! Reviewable#perform (PUT /review/:id/perform/:action_id, which review.rs
//! hands this type) with the perform methods (the message trashed or
//! recovered, the author unsilenced), the transition, flag stats and score.

use chrono::NaiveDateTime;
use serde_json::json;
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::review::Outcome;
use crate::reviewables::{self, PENDING};

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

/// The flagged message (`chat_message`, with deleted).
pub struct Message {
    pub id: i64,
    pub user_id: Option<i32>,
    pub deleted: bool,
}

/// Its author (`chat_message_creator`).
#[derive(sqlx::FromRow)]
pub struct Creator {
    pub id: i32,
    pub admin: bool,
    pub moderator: bool,
    pub silenced_till: Option<NaiveDateTime>,
}

impl Creator {
    /// `User#silenced?`
    pub fn silenced(&self) -> bool {
        self.silenced_till
            .is_some_and(|t| t > crate::clock::now_naive())
    }
}

/// A Chat::ReviewableMessage as its actions read it.
pub struct Target {
    pub id: i64,
    pub status: i32,
    pub message: Option<Message>,
    pub creator: Option<Creator>,
}

/// One action build_combined_actions adds: its id and icon, its bundle
/// (`agree` or `disagree`, else its own), client action and whether it
/// is set apart (secondary).
pub struct Built {
    pub id: &'static str,
    pub icon: &'static str,
    pub bundle: Option<&'static str>,
    pub client_action: Option<&'static str>,
    pub secondary: bool,
}

/// The bundles the actions go in: id suffix, icon and label key.
pub const BUNDLES: [(&str, &str, &str); 2] = [
    ("agree", "thumbs-up", "reviewables.actions.agree.title"),
    (
        "disagree",
        "far-eye",
        "reviewables.actions.disagree_bundle.title",
    ),
];

impl Target {
    pub async fn load(conn: &mut PgConnection, reviewable_id: i64) -> Result<Target, AppError> {
        let (status, target_id): (i32, Option<i32>) =
            sqlx::query_as("SELECT status, target_id FROM reviewables WHERE id = $1")
                .bind(reviewable_id)
                .fetch_one(&mut *conn)
                .await?;
        let message: Option<(i64, Option<i32>, bool)> =
            match target_id {
                Some(id) => sqlx::query_as(
                    "SELECT id, user_id, deleted_at IS NOT NULL FROM chat_messages WHERE id = $1",
                )
                .bind(i64::from(id))
                .fetch_optional(&mut *conn)
                .await?,
                None => None,
            };
        let message = message.map(|(id, user_id, deleted)| Message {
            id,
            user_id,
            deleted,
        });
        let creator: Option<Creator> = match message.as_ref().and_then(|m| m.user_id) {
            Some(id) => {
                sqlx::query_as(
                    "SELECT id, admin, moderator, silenced_till FROM users WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
            }
            None => None,
        };
        Ok(Target {
            id: reviewable_id,
            status,
            message,
            creator,
        })
    }

    /// `build_combined_actions`, then `build_unsilence_action`.
    pub fn actions(&self, guardian: &Guardian) -> Vec<Built> {
        let mut out = Vec::new();
        if self.status != PENDING {
            return out;
        }
        let action = |id, icon, bundle, client_action| Built {
            id,
            icon,
            bundle,
            client_action,
            secondary: false,
        };
        let Some(message) = &self.message else {
            out.push(action("ignore", "up-right-from-square", None, None));
            return out;
        };
        let agree = Some("agree");
        if message.deleted {
            out.push(action("agree_and_restore", "far-eye", agree, None));
            out.push(action("agree_and_keep_deleted", "thumbs-up", agree, None));
        } else {
            out.push(action("agree_and_delete", "trash-can", agree, None));
            out.push(action("agree_and_keep_message", "far-eye", agree, None));
        }
        // can_suspend?: staff, over a regular user.
        if let Some(c) = &self.creator
            && guardian.is_staff()
            && !c.admin
            && !c.moderator
        {
            out.push(action("agree_and_suspend", "ban", agree, Some("suspend")));
            out.push(action(
                "agree_and_silence",
                "microphone-slash",
                agree,
                Some("silence"),
            ));
        }
        let disagree = Some("disagree");
        if message.deleted {
            out.push(action("disagree_and_restore", "far-eye", disagree, None));
        } else {
            out.push(action("disagree", "far-eye", disagree, None));
        }
        out.push(action("ignore", "xmark", disagree, None));
        if !message.deleted {
            out.push(action("delete_and_agree", "trash-can", disagree, None));
        }
        // build_unsilence_action: can_unsilence_user?
        if let Some(c) = &self.creator
            && c.silenced()
            && guardian.is_staff()
            && (!(c.admin || c.moderator) || guardian.is_admin())
        {
            out.push(Built {
                secondary: true,
                ..action("unsilence_user", "microphone-slash", None, None)
            });
        }
        out
    }

    /// `silenced_for_this_message?`: the author is silenced, by a silence
    /// this reviewable brought.
    pub async fn silenced_for_this_message(
        &self,
        conn: &mut PgConnection,
    ) -> Result<bool, AppError> {
        let Some(c) = self.creator.as_ref().filter(|c| c.silenced()) else {
            return Ok(false);
        };
        let latest: Option<Option<i64>> = sqlx::query_scalar(
            "SELECT reviewable_id FROM user_histories WHERE target_user_id = $1 AND action = $2 \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(c.id)
        .bind(SILENCE_USER)
        .fetch_optional(&mut *conn)
        .await?;
        Ok(latest.flatten() == Some(self.id))
    }
}

/// `penalty_effect_for(action_id)` while the author has penalties.
pub fn penalty_effect(action_id: &str, silenced_for_this_message: bool) -> &'static str {
    let lifts = match action_id {
        "disagree" | "disagree_and_restore" => silenced_for_this_message,
        "unsilence_user" => true,
        _ => false,
    };
    if lifts {
        "lifts_penalty"
    } else {
        "retains_penalty"
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

const AGREE: Settled = Settled {
    status: APPROVED,
    score_status: SCORE_AGREED,
    stat: "flags_agreed",
    recalculate: true,
};
const DISAGREE: Settled = Settled {
    status: REJECTED,
    score_status: SCORE_DISAGREED,
    stat: "flags_disagreed",
    recalculate: true,
};
const IGNORE: Settled = Settled {
    status: IGNORED,
    score_status: SCORE_IGNORED,
    stat: "flags_ignored",
    recalculate: false,
};

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
    let target = Target::load(&mut *conn, reviewable_id).await?;
    // validate_action!: the action or one of its aliases' targets.
    let built = target.actions(guardian);
    let allowed = built
        .iter()
        .any(|a| a.id == action_id || (alias(a.id) == action_id && a.id != action_id));
    if !allowed {
        return Ok(Outcome::Forbidden);
    }
    let Some(new_version) =
        run(conn, ctx, user.id, &target, alias(action_id), Some(version)).await?
    else {
        return Ok(Outcome::Conflict);
    };
    let settled = alias(action_id) != "unsilence_user";
    let remove: Vec<i64> = if settled {
        vec![reviewable_id]
    } else {
        Vec::new()
    };
    if settled {
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

/// The perform method (`method`, the alias resolved) inside
/// Reviewable#perform's transaction: increment_version! (None when the
/// version is stale), the method's work, the transition and its history,
/// the scores settled, update_flag_stats and recalculate_score. The new
/// version.
pub async fn run(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    performer_id: i32,
    target: &Target,
    method: &str,
    version: Option<i64>,
) -> Result<Option<i64>, AppError> {
    // increment_version!: a version given must match.
    let bumped: Option<i32> = sqlx::query_scalar(
        "UPDATE reviewables SET version = version + 1 WHERE id = $1 AND ($2::int IS NULL OR version = $2) \
         RETURNING version",
    )
    .bind(target.id)
    .bind(version.map(|v| v as i32))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(new_version) = bumped else {
        return Ok(None);
    };

    let message_id = target.message.as_ref().map(|m| m.id);
    let author = target.creator.as_ref().map(|c| c.id).unwrap_or_default();
    let settled = match method {
        "agree_and_keep_message" | "agree_and_keep_deleted" => Some(AGREE),
        "agree_and_restore" => {
            recover(&mut *conn, message_id).await?;
            Some(AGREE)
        }
        "agree_and_delete" => {
            trash(&mut *conn, message_id, performer_id).await?;
            Some(AGREE)
        }
        "disagree" | "disagree_and_restore" => {
            if method == "disagree_and_restore" {
                recover(&mut *conn, message_id).await?;
            }
            if target.silenced_for_this_message(&mut *conn).await? {
                crate::user_penalties::lift_silence_for_reviewable(
                    &mut *conn,
                    ctx,
                    performer_id,
                    author,
                    target.id,
                )
                .await?;
            }
            Some(DISAGREE)
        }
        "ignore" => Some(IGNORE),
        "delete_and_ignore" => {
            trash(&mut *conn, message_id, performer_id).await?;
            Some(IGNORE)
        }
        "unsilence_user" => {
            crate::user_penalties::lift_silence_for_reviewable(
                &mut *conn,
                ctx,
                performer_id,
                author,
                target.id,
            )
            .await?;
            None
        }
        _ => None,
    };

    if let Some(settled) = &settled {
        let hooks = crate::web_hooks::Payloads {
            settings: ctx.settings,
            i18n: ctx.i18n,
            config: ctx.config,
        };
        // transition_to: no topic to release a claim of.
        sqlx::query(
            "UPDATE reviewables SET status = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(target.id)
        .bind(settled.status)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                               edited, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
        )
        .bind(target.id)
        .bind(HISTORY_TRANSITIONED)
        .bind(settled.status)
        .bind(performer_id)
        .execute(&mut *conn)
        .await?;
        crate::web_hooks::enqueue_reviewable_hooks(
            &mut *conn,
            &hooks,
            "reviewable_transitioned_to",
            target.id,
        )
        .await?;
        sqlx::query(
            "UPDATE reviewable_scores SET status = $2, reviewed_by_id = $3, reviewed_at = clock_timestamp() \
             WHERE reviewable_id = $1 AND status IN ($4, $5)",
        )
        .bind(target.id)
        .bind(settled.score_status)
        .bind(performer_id)
        .bind(PENDING)
        .bind(SCORE_DISAGREED)
        .execute(&mut *conn)
        .await?;
        // update_flag_stats over flagged_by_user_ids (`post` is nil, so
        // no flag is a self-flag).
        let flaggers: Vec<i32> =
            sqlx::query_scalar("SELECT user_id FROM reviewable_scores WHERE reviewable_id = $1")
                .bind(target.id)
                .fetch_all(&mut *conn)
                .await?;
        reviewables::update_flag_stats(&mut *conn, settled.stat, &flaggers).await?;
        if settled.recalculate {
            reviewables::recalculate_score(&mut *conn, &hooks, target.id).await?;
        }
    }
    Ok(Some(i64::from(new_version)))
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
