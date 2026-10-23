//! ReviewableClaimedTopic: a reviewer's claim on a topic, which the review
//! queue's claim button takes (or a confirmation, a modal or a client
//! action takes automatically first) and releases. A manual claim is
//! logged on the topic's pending reviewables as claimed or unclaimed;
//! every change goes to the staff on /reviewable_claimed with their counts
//! refreshed.

use serde_json::json;
use sqlx::PgConnection;

use crate::posting::Ctx;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

/// Reviewable.statuses[:pending]
const PENDING: i32 = 0;
/// ReviewableHistory.types
const HISTORY_CLAIMED: i32 = 3;
const HISTORY_UNCLAIMED: i32 = 4;
/// Group::AUTO_GROUPS[:staff]
const STAFF_GROUP: i64 = 3;

/// A claim row.
#[derive(sqlx::FromRow)]
pub struct Claim {
    pub id: i64,
    pub user_id: i32,
    pub topic_id: i32,
    pub automatic: bool,
}

/// `ReviewableClaimedTopic.create!` and what the controller does after:
/// None when the topic is claimed already (the uniqueness validation).
pub async fn claim(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user: &SessionUser,
    topic_id: i32,
    automatic: bool,
) -> Result<Option<i64>, AppError> {
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO reviewable_claimed_topics (user_id, topic_id, automatic, created_at, updated_at) \
         VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp()) \
         ON CONFLICT (topic_id) DO NOTHING RETURNING id",
    )
    .bind(user.id)
    .bind(topic_id)
    .bind(automatic)
    .fetch_optional(&mut *conn)
    .await?;
    if id.is_none() {
        return Ok(None);
    }
    if !automatic {
        log_topic_history(conn, topic_id, HISTORY_CLAIMED, user.id).await?;
    }
    publish_change(conn, ctx, user, topic_id, automatic, true).await?;
    Ok(id)
}

/// `ReviewableClaimedTopic#release(performed_by)`
pub async fn release(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    claim: &Claim,
    performer: &SessionUser,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM reviewable_claimed_topics WHERE id = $1")
        .bind(claim.id)
        .execute(&mut *conn)
        .await?;
    if ctx.settings.get("reviewable_claiming")?.to_s() != "disabled" && !claim.automatic {
        log_topic_history(conn, claim.topic_id, HISTORY_UNCLAIMED, performer.id).await?;
    }
    publish_change(conn, ctx, performer, claim.topic_id, claim.automatic, false).await
}

/// `ReviewableClaimedTopic.find_by(topic_id:)`
pub async fn find(conn: &mut PgConnection, topic_id: i32) -> Result<Option<Claim>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, user_id, topic_id, automatic FROM reviewable_claimed_topics WHERE topic_id = $1 \
         ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(conn)
    .await
}

/// `ReviewableClaimedTopic.release_manual_claim(topic_id, performed_by)`,
/// as Reviewable#transition_to releases a pending reviewable's topic.
pub async fn release_manual_claim(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    performer: &SessionUser,
) -> Result<(), AppError> {
    let claim: Option<Claim> = sqlx::query_as(
        "SELECT id, user_id, topic_id, automatic FROM reviewable_claimed_topics \
         WHERE topic_id = $1 AND NOT automatic ORDER BY id LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    match claim {
        Some(claim) => release(conn, ctx, &claim, performer).await,
        None => Ok(()),
    }
}

/// `log_topic_history`: the type on each of the topic's pending
/// reviewables, in id order (find_each).
async fn log_topic_history(
    conn: &mut PgConnection,
    topic_id: i32,
    kind: i32,
    performer_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         SELECT id, $2, status, $3, NULL, clock_timestamp(), clock_timestamp() \
         FROM reviewables WHERE status = $4 AND topic_id = $1 ORDER BY id",
    )
    .bind(topic_id)
    .bind(kind)
    .bind(performer_id)
    .bind(PENDING)
    .execute(conn)
    .await?;
    Ok(())
}

/// `publish_change`: /reviewable_claimed for the staff (and the topic's
/// category moderators, not ported) and their counts refreshed.
async fn publish_change(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    performer: &SessionUser,
    topic_id: i32,
    automatic: bool,
    claimed: bool,
) -> Result<(), AppError> {
    if ctx
        .settings
        .get("enable_category_group_moderation")?
        .truthy()
    {
        return Err(Unsupported("claims with category group moderation").into());
    }
    // BasicUserSerializer of the performer.
    let urls = crate::url::Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let avatar_template = crate::avatar::avatar_template(
        &urls,
        performer.id,
        &performer.username,
        performer.uploaded_avatar_id,
        None,
    )?;
    ctx.bus
        .publish(
            &mut *conn,
            "/reviewable_claimed",
            &json!({
                "topic_id": topic_id,
                "user": {
                    "id": performer.id,
                    "username": performer.username,
                    "name": performer.name,
                    "avatar_template": avatar_template,
                },
                "automatic": automatic,
                "claimed": claimed,
            }),
            Some(&[crate::bus::group_tag(STAFF_GROUP as i32)]),
        )
        .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "refresh_users_reviewable_counts",
        json!({ "group_ids": [STAFF_GROUP] }),
    )
    .await?;
    Ok(())
}
