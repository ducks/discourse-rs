//! `Topic#make_banner!` and `Topic#remove_banner!` (TopicsController
//! #make_banner and #remove_banner, Jobs::RemoveBanner and
//! Topic.ensure_consistency!'s expired banners): the archetype, the small
//! action saying so, the save, and the scheduled removal cancelled.
//!
//! The banner itself (ApplicationLayoutPreloader's banner, drawn atop
//! every page, and its `/site/banner` messages) is not drawn by the port.

use serde_json::Value;
use sqlx::PgConnection;

use crate::AppError;
use crate::posting::Ctx;
use crate::posting::small_action::{self, SmallAction};

/// `Topic#make_banner!(user)`: the banner there is removed first (one at
/// a time), every dismissed banner is shown again, then the topic becomes
/// the banner.
pub async fn make_banner(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
) -> Result<(), AppError> {
    // Topic.where(archetype: banner).first
    let previous: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM topics WHERE archetype = 'banner' AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(previous) = previous {
        remove_banner(conn, ctx, previous, user_id).await?;
    }
    sqlx::query(
        "UPDATE user_profiles SET dismissed_banner_key = NULL WHERE dismissed_banner_key IS NOT NULL",
    )
    .execute(&mut *conn)
    .await?;
    change(conn, ctx, topic_id, user_id, "banner", "banner.enabled").await
}

/// `Topic#remove_banner!(user)`: back to a regular topic, said so even
/// when it wasn't the banner.
pub async fn remove_banner(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
) -> Result<(), AppError> {
    change(conn, ctx, topic_id, user_id, "regular", "banner.disabled").await
}

/// The archetype set and bannered_until cleared (the controller gives no
/// time), the small action added (before
/// the save, on the row as it is), then `save`: what changed, with the
/// slug the title reassigns, and the scheduled removal cancelled.
async fn change(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
    archetype: &str,
    action_code: &str,
) -> Result<(), AppError> {
    small_action::add(
        &mut *conn,
        ctx,
        &SmallAction {
            user_id,
            topic_id,
            action_code,
            bump: false,
            who: None,
        },
    )
    .await?;
    let slug =
        crate::posting::topic_save::reassigned_slug(&mut *conn, ctx.settings, topic_id).await?;
    sqlx::query(
        "UPDATE topics SET archetype = $2, bannered_until = NULL, slug = $3, fancy_title = NULL, \
                           updated_at = clock_timestamp() \
         WHERE id = $1 AND (archetype IS DISTINCT FROM $2 OR bannered_until IS NOT NULL \
                            OR slug IS DISTINCT FROM $3 OR fancy_title IS NOT NULL)",
    )
    .bind(topic_id)
    .bind(archetype)
    .bind(&slug)
    .execute(&mut *conn)
    .await?;
    // Jobs.cancel_scheduled_job(:remove_banner, topic_id:)
    sqlx::query(
        "DELETE FROM discourse_rs.jobs WHERE name = 'remove_banner' AND args->>'topic_id' = $1::text",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs::RemoveBanner`: a topic still there removed as the banner by the
/// system user.
pub async fn remove_banner_job(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    args: &Value,
) -> Result<(), AppError> {
    let topic_id = match &args["topic_id"] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) if !crate::ruby::is_blank(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let Some(topic_id) = topic_id else {
        return Ok(());
    };
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id as i32)
    .fetch_one(&mut *conn)
    .await?;
    if exists {
        remove_banner(conn, ctx, topic_id as i32, -1).await?;
    }
    Ok(())
}
