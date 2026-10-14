//! The scheduled jobs that delete users through UserDestroyer, as system:
//! accounts never activated, inactive new users, unused staged users.

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::pretty_text::Host;
use crate::site_settings::SiteSettings;
use crate::user_destroyer::{self, Options, Outcome};
use crate::{AppError, AppState, Unsupported};

/// `Jobs::PurgeUnactivated`, daily.
pub const PURGE_UNACTIVATED: &str = "purge_unactivated";
/// `Jobs::CleanUpInactiveUsers`, daily.
pub const CLEAN_UP_INACTIVE_USERS: &str = "clean_up_inactive_users";
/// `Jobs::CleanUpUnusedStagedUsers`, daily.
pub const CLEAN_UP_UNUSED_STAGED_USERS: &str = "clean_up_unused_staged_users";

/// `UserHistory.actions[:deactivate_user]`
const DEACTIVATE_USER: i32 = 39;
/// `TrustLevel.levels[:newuser]`
const NEWUSER: i32 = 0;
/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE: i32 = 2;

async fn settings(state: &AppState) -> Result<SiteSettings, AppError> {
    let mut conn = state.pool.acquire().await?;
    Ok(SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `UserDestroyer.new(Discourse.system_user)`'s guardian.
async fn system_guardian(state: &AppState) -> Result<Guardian, AppError> {
    let mut conn = state.pool.acquire().await?;
    let system = crate::session::current::SessionUser::load(&mut conn, -1)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    Ok(Guardian::for_user(&mut conn, &system).await?)
}

fn context(state: &AppState, key: &str) -> Result<String, AppError> {
    Ok(state
        .i18n
        .t(key)
        .ok_or(Unsupported("a missing destroy reason translation"))?
        .to_string())
}

/// What UserDestroyer raises where these jobs let it out.
fn raised(outcome: Outcome) -> Result<(), AppError> {
    match outcome {
        Outcome::Destroyed => Ok(()),
        Outcome::PostsExist => Err(std::io::Error::other("UserDestroyer::PostsExistError").into()),
        Outcome::Forbidden => Err(std::io::Error::other("Discourse::InvalidAccess").into()),
    }
}

/// `User.purge_unactivated`: up to 200 inactive users older than the
/// grace period, neither staff, in no message someone started, without
/// posts (deleted ones too) and not deactivated by staff, each destroyed
/// in its own transaction; one the guardian refuses is skipped.
pub async fn purge_unactivated(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let days = s.get("purge_unactivated_users_grace_period_days")?.to_i();
    if days <= 0 {
        return Ok(());
    }
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users \
         LEFT JOIN user_histories ON user_histories.target_user_id = users.id \
           AND action = $2 AND acting_user_id IS NOT NULL \
         WHERE users.active = FALSE AND users.created_at < now() - make_interval(days => $1) \
           AND NOT admin AND NOT moderator \
           AND NOT EXISTS (SELECT 1 FROM topic_allowed_users tu JOIN topics t ON t.id = tu.topic_id \
                           AND t.user_id > 0 WHERE tu.user_id = users.id LIMIT 1) \
           AND NOT EXISTS (SELECT 1 FROM posts p WHERE p.user_id = users.id LIMIT 1) \
           AND user_histories.id IS NULL \
         ORDER BY users.id LIMIT 200",
    )
    .bind(days as i32)
    .bind(DEACTIVATE_USER)
    .fetch_all(&state.pool)
    .await?;
    let opts = Options {
        context: Some(context(state, "purge_reason")?),
        ..Default::default()
    };
    destroy_each(state, &s, &ids, &opts, false, |outcome| match outcome {
        // rescue Discourse::InvalidAccess: keep going
        Outcome::Forbidden => Ok(()),
        other => raised(other),
    })
    .await
}

/// `Jobs::CleanUpInactiveUsers`: up to 1000 new users neither staff
/// nor seen nor posting since the cutoff, with no live posts, topics or
/// bookmarks and no live like on a live post, destroyed 50 to a
/// transaction; any failure rolls its fifty back and fails the job.
pub async fn clean_up_inactive_users(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let days = s.get("clean_up_inactive_users_after_days")?.to_i();
    if days <= 0 {
        return Ok(());
    }
    // where.missing(:posts, :topics, :bookmarks): LEFT JOINs with the
    // models' default scopes (live posts and topics).
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users \
         LEFT OUTER JOIN posts ON posts.deleted_at IS NULL AND posts.user_id = users.id \
         LEFT OUTER JOIN topics ON topics.deleted_at IS NULL AND topics.user_id = users.id \
         LEFT OUTER JOIN bookmarks ON bookmarks.user_id = users.id \
         WHERE users.last_posted_at IS NULL AND users.trust_level = $2 \
           AND users.admin = FALSE AND users.moderator = FALSE \
           AND users.created_at < now() - make_interval(days => $1) \
           AND (users.last_seen_at < now() - make_interval(days => $1) OR users.last_seen_at IS NULL) \
           AND posts.id IS NULL AND topics.id IS NULL AND bookmarks.id IS NULL \
           AND NOT EXISTS (SELECT 1 FROM post_actions pa INNER JOIN posts p ON p.id = pa.post_id \
                           WHERE pa.user_id = users.id AND pa.post_action_type_id = $3 \
                             AND pa.deleted_at IS NULL AND p.deleted_at IS NULL) \
         ORDER BY users.id LIMIT 1000",
    )
    .bind(days as i32)
    .bind(NEWUSER)
    .bind(LIKE)
    .fetch_all(&state.pool)
    .await?;
    let actor = system_guardian(state).await?;
    let host = Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    let opts = Options {
        context: Some(context(state, "user.destroy_reasons.inactive_user")?),
        ..Default::default()
    };
    for slice in ids.chunks(50) {
        let mut tx = state.pool.begin().await?;
        for &id in slice {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            if !exists {
                continue;
            }
            raised(user_destroyer::destroy(&mut tx, &ctx, &actor, id, &opts).await?)?;
        }
        tx.commit().await?;
    }
    Ok(())
}

/// `Jobs::CleanUpUnusedStagedUsers`: staged users older than the cutoff,
/// neither staff, without any post (deleted ones too), each destroyed in
/// its own transaction. Rails logs a failure and goes on; what the port
/// has not ported still fails the job.
pub async fn clean_up_unused_staged_users(state: &AppState) -> Result<(), AppError> {
    let s = settings(state).await?;
    let days = s.get("clean_up_unused_staged_users_after_days")?.to_i();
    if days <= 0 {
        return Ok(());
    }
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT users.id FROM users LEFT JOIN posts ON posts.user_id = users.id \
         WHERE posts.user_id IS NULL AND users.staged = TRUE \
           AND users.admin = FALSE AND users.moderator = FALSE \
           AND users.created_at < now() - make_interval(days => $1) \
         ORDER BY users.id",
    )
    .bind(days as i32)
    .fetch_all(&state.pool)
    .await?;
    let opts = Options {
        context: Some(context(state, "user.destroy_reasons.unused_staged_user")?),
        ..Default::default()
    };
    destroy_each(state, &s, &ids, &opts, true, raised).await
}

/// Each user destroyed in a transaction of its own; `settle` turns an
/// outcome into what the job does with it. With `log_errors` (Rails'
/// `rescue => e; handle_job_exception`), a failure other than an
/// unported one is logged, its user left, and the next one goes on.
async fn destroy_each(
    state: &AppState,
    s: &SiteSettings,
    ids: &[i32],
    opts: &Options,
    log_errors: bool,
    settle: impl Fn(Outcome) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let actor = system_guardian(state).await?;
    let host = Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings: s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    for &id in ids {
        let mut tx = state.pool.begin().await?;
        let result = match user_destroyer::destroy(&mut tx, &ctx, &actor, id, opts).await {
            Ok(outcome) => settle(outcome),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => tx.commit().await?,
            Err(e) => {
                tx.rollback().await?;
                if !log_errors || e.is_unsupported() {
                    return Err(e);
                }
                tracing::warn!(user_id = id, error = %e, "cleaning up a user failed");
            }
        }
    }
    Ok(())
}
