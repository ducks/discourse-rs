//! `Topic#make_banner!` and `Topic#remove_banner!` (TopicsController
//! #make_banner and #remove_banner, Jobs::RemoveBanner and
//! Topic.ensure_consistency!'s expired banners): the archetype, the small
//! action saying so, the save, and the scheduled removal cancelled.
//!
//! The banner itself: ApplicationLayoutPreloader#banner_json (the
//! preloaded `banner`, GET /site/banner) and Topic#banner, which
//! `/site/banner` carries when a banner is made (null when removed);
//! crate::html draws it atop the pages Ember draws DiscourseBanner on.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::small_action::{self, SmallAction};
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

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
    change(conn, ctx, topic_id, user_id, "banner", "banner.enabled").await?;
    // MessageBus.publish("/site/banner", banner): to everyone, as
    // Topic#banner without a guardian.
    let urls = Urls {
        config: ctx.config,
        settings: ctx.settings,
    };
    let message = banner(&mut *conn, ctx.settings, &urls, topic_id, None).await?;
    ctx.bus.publish(&mut *conn, CHANNEL, &message, None).await?;
    Ok(())
}

/// `Topic#remove_banner!(user)`: back to a regular topic, said so even
/// when it wasn't the banner.
pub async fn remove_banner(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
) -> Result<(), AppError> {
    change(conn, ctx, topic_id, user_id, "regular", "banner.disabled").await?;
    ctx.bus
        .publish(&mut *conn, CHANNEL, &Value::Null, None)
        .await?;
    Ok(())
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

/// The channel DiscourseBanner's initializer (instance-initializers/
/// banner.js) hears: the banner made, or null once removed.
pub const CHANNEL: &str = "/site/banner";

/// `Topic#banner(guardian)`: the first post's cooked, the topic's id as
/// the key the dismissal stores, and its url (Topic#url, absolute).
/// A translated first post (ContentLocalization.show_translated_post?
/// for the viewer) is refused.
pub async fn banner(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    topic_id: i32,
    guardian: Option<&Guardian>,
) -> Result<Value, AppError> {
    // ordered_posts.first: the trashed ones left out by Post's scope.
    let row: Option<(String, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT p.cooked, p.locale, p.raw, t.slug FROM topics t \
         JOIN posts p ON p.topic_id = t.id AND p.deleted_at IS NULL \
         WHERE t.id = $1 ORDER BY p.sort_order LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((cooked, locale, raw, slug)) = row else {
        return Err(Unsupported("a banner topic without posts").into());
    };
    if guardian.is_some()
        && settings.get("content_localization_enabled")?.truthy()
        && locale.is_some_and(|l| !l.is_empty())
        && !raw.trim().is_empty()
    {
        return Err(Unsupported("a translated banner (content localization)").into());
    }
    // Topic#slug writes one for a topic without; refused instead.
    let Some(slug) = slug else {
        return Err(Unsupported("a banner topic without a slug").into());
    };
    // Topic.url(id, slug): the slug segment only when present.
    let mut url = format!("{}/t/", urls.base_url()?);
    if !slug.is_empty() {
        url.push_str(&slug);
        url.push('/');
    }
    url.push_str(&topic_id.to_string());
    Ok(json!({ "html": cooked, "key": topic_id, "url": url }))
}

/// `ApplicationLayoutPreloader#banner_json`: nothing for a visitor while
/// login is required, else the first banner topic's banner unless its
/// category is read restricted (an empty object for none). Rails caches it
/// per locale until a banner topic is saved; it is read each time here.
pub async fn preloaded(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    guardian: &Guardian,
) -> Result<Value, AppError> {
    if guardian.is_anonymous() && settings.get("login_required")?.truthy() {
        return Ok(json!({}));
    }
    // Topic.where(archetype: banner).first, by id.
    let topic: Option<(i32, bool)> = sqlx::query_as(
        "SELECT t.id, COALESCE(c.read_restricted, false) FROM topics t \
         LEFT JOIN categories c ON c.id = t.category_id \
         WHERE t.archetype = 'banner' AND t.deleted_at IS NULL ORDER BY t.id LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    match topic {
        Some((id, false)) => banner(conn, settings, urls, id, Some(guardian)).await,
        _ => Ok(json!({})),
    }
}
