//! GET /about.json (about#index): the about page's document.

use axum::Json;
use axum::extract::State;
use serde_json::Value;

use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState};

/// GET /about.json
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<Json<Value>, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    Ok(Json(
        crate::about::document(&mut conn, &settings, &urls, &guardian).await?,
    ))
}

/// GET /about/live_post_counts (about#live_post_counts): the public
/// topics, and their replies, outside category definitions; members once
/// every ten minutes, staff as often as they like.
pub async fn live_post_counts(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let limiter = crate::rate_limiter::RateLimiter::new(
        Some(crate::rate_limiter::LimitUser {
            id: user_id,
            staff: guardian.is_staff(),
        }),
        "live_post_counts",
        1,
        600,
    );
    if let Err(refused) = limiter
        .performed(&state.pool, state.config.rate_limits)
        .await?
    {
        return Ok(refused.into_response(&state.i18n));
    }
    // Topic.listable_topics.visible.secured(Guardian.new(nil))
    let (topics, posts): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(t.posts_count), 0)::bigint FROM topics t \
         WHERE t.deleted_at IS NULL AND t.archetype <> 'private_message' AND t.visible \
           AND (t.category_id IS NULL OR t.category_id IN (SELECT id FROM categories WHERE NOT read_restricted)) \
           AND t.id NOT IN (SELECT topic_id FROM categories WHERE topic_id IS NOT NULL)",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(serde_json::json!({
        "public_topic_count": topics,
        "public_post_count": posts - topics,
    }))
    .into_response())
}
