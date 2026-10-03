//! The review queue's bookkeeping that flagging goes through: Reviewable
//! (`needs_review!` for a new reviewable, `add_score`, `log_history`), the
//! score of a flag (ReviewableScore), and the thresholds flags are held to
//! (`sensitivity_score` and `min_score_for_priority`, from the priorities
//! Jobs::ReviewablePriorities keeps in the plugin store).

use serde_json::json;
use sqlx::PgConnection;

use crate::posting::text::{TitleOptions, clean_title, slug_for};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Reviewable.statuses[:pending]`, also `ReviewableScore.statuses[:pending]`
pub const PENDING: i32 = 0;
/// `ReviewableScore.statuses[:agreed]`
pub const AGREED: i32 = 1;
/// `ReviewableHistory.types[:created]`
const HISTORY_CREATED: i32 = 0;

/// `Reviewable.typical_sensitivity`
const TYPICAL_SENSITIVITY: f64 = 12.5;
/// `Reviewable.sensitivities[:low]`
const LOW_SENSITIVITY: f64 = 9.0;

/// `PluginStore.get("reviewables", "priority_#{id}")` as `to_f` reads it.
async fn priority(conn: &mut PgConnection, id: i32) -> Result<Option<f64>, sqlx::Error> {
    let value: Option<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM plugin_store_rows WHERE plugin_name = 'reviewables' AND key = $1",
    )
    .bind(format!("priority_{id}"))
    .fetch_optional(conn)
    .await?;
    Ok(value.flatten().map(|v| crate::ruby::to_f(&v)))
}

/// `Reviewable.min_score_for_priority` for reviewable_default_visibility.
pub async fn min_score_for_priority(
    conn: &mut PgConnection,
    s: &SiteSettings,
) -> Result<f64, AppError> {
    let id = match s.get("reviewable_default_visibility")?.to_s().as_str() {
        "low" => 0,
        "medium" => 5,
        "high" => 10,
        _ => return Ok(0.0),
    };
    Ok(priority(conn, id).await?.unwrap_or(0.0))
}

/// `Reviewable.sensitivity_score(SiteSetting.<setting>, scale:)`
pub async fn sensitivity_score(
    conn: &mut PgConnection,
    s: &SiteSettings,
    setting: &str,
    scale: f64,
) -> Result<f64, AppError> {
    let sensitivity = s.get(setting)?.to_i() as f64;
    let value = if sensitivity == 0.0 {
        f64::MAX
    } else {
        let high = priority(conn, 10).await?.unwrap_or(TYPICAL_SENSITIVITY);
        ((high * (sensitivity / LOW_SENSITIVITY)) * scale * 100.0).trunc() / 100.0
    };
    Ok(value.max(min_score_for_priority(conn, s).await?))
}

/// `ReviewableScore.calc_user_accuracy_bonus(agreed, disagreed)`
fn accuracy_bonus(agreed: i32, disagreed: i32) -> f64 {
    let total = f64::from(agreed + disagreed);
    if total <= 5.0 {
        return 0.0;
    }
    let axis = 0.7;
    let correct = f64::from(agreed) / total;
    let positive = correct >= axis;
    let (bottom, top) = if positive { (axis, 1.0) } else { (0.0, axis) };
    let distance = if positive {
        correct - bottom
    } else {
        top - correct
    };
    let sign = if positive { 1.0 } else { -1.0 };
    let bonus = distance * (1.0 / (top - bottom)) * sign * (total.ln() / 4f64.ln() * 5.0);
    (bonus * 100.0).round() / 100.0
}

/// A flagger as ReviewableScore weighs them.
pub struct Flagger {
    pub id: i32,
    pub staff: bool,
    pub trust_level: i32,
}

/// `ReviewableScore.user_accuracy_bonus(user)`: none for bots or without
/// user stats.
async fn user_accuracy_bonus(conn: &mut PgConnection, user: &Flagger) -> Result<f64, sqlx::Error> {
    if user.id < 0 {
        return Ok(0.0);
    }
    let stats: Option<(i32, i32)> =
        sqlx::query_as("SELECT flags_agreed, flags_disagreed FROM user_stats WHERE user_id = $1")
            .bind(user.id)
            .fetch_optional(conn)
            .await?;
    Ok(stats.map_or(0.0, |(a, d)| accuracy_bonus(a, d)))
}

/// A new ReviewableFlaggedPost (`needs_review!` when the post has none).
pub struct NewFlaggedPost {
    pub created_by_id: i32,
    pub post_id: i32,
    pub topic_id: i32,
    pub category_id: Option<i32>,
    pub post_user_id: Option<i32>,
    pub potential_spam: bool,
    pub potentially_illegal: bool,
    pub targets_topic: bool,
}

/// `ReviewableFlaggedPost.needs_review!` for a post with no reviewable:
/// saved, its `created` history logged, notify_reviewable enqueued on
/// commit as it is pending.
pub async fn create_flagged_post(
    conn: &mut PgConnection,
    r: &NewFlaggedPost,
) -> Result<i64, AppError> {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO reviewables (type, type_source, status, created_by_id, reviewable_by_moderator, \
                                  reviewable_by_group_id, category_id, topic_id, score, potential_spam, \
                                  target_id, target_type, target_created_by_id, payload, version, \
                                  latest_score, force_review, potentially_illegal, created_at, updated_at) \
         VALUES ('ReviewableFlaggedPost', 'core', $1, $2, TRUE, NULL, $3, $4, 0, $5, $6, 'Post', $7, $8, 0, \
                 NULL, FALSE, $9, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(PENDING)
    .bind(r.created_by_id)
    .bind(r.category_id)
    .bind(r.topic_id)
    .bind(r.potential_spam)
    .bind(r.post_id)
    .bind(r.post_user_id)
    .bind(json!({ "targets_topic": r.targets_topic }))
    .bind(r.potentially_illegal)
    .fetch_one(&mut *conn)
    .await?;
    // after_create log_history(:created, created_by)
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(id)
    .bind(HISTORY_CREATED)
    .bind(PENDING)
    .bind(r.created_by_id)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({ "reviewable_id": id }),
    )
    .await?;
    Ok(id)
}

/// `Reviewable#add_score(user, type, created_at:)` without taking action,
/// a reason, a meta topic or forcing review. Returns the score added.
pub async fn add_score(
    conn: &mut PgConnection,
    reviewable_id: i64,
    topic_id: i32,
    user: &Flagger,
    score_type: i64,
    created_at: chrono::NaiveDateTime,
    s: &SiteSettings,
) -> Result<f64, AppError> {
    let type_bonus: Option<f64> =
        sqlx::query_scalar("SELECT score_bonus FROM post_action_types WHERE id = $1")
            .bind(score_type as i32)
            .fetch_optional(&mut *conn)
            .await?;
    let accuracy = user_accuracy_bonus(&mut *conn, user).await?;
    // ReviewableScore.calculate_score: user_flag_score + type bonus
    let user_flag_score =
        1.0 + if user.staff {
            5.0
        } else {
            f64::from(user.trust_level)
        } + accuracy;
    let score = (user_flag_score + type_bonus.unwrap_or(0.0)).max(0.0);
    sqlx::query(
        "INSERT INTO reviewable_scores (reviewable_id, user_id, reviewable_score_type, status, score, \
                                        take_action_bonus, user_accuracy_bonus, meta_topic_id, reason, \
                                        context, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 0, $6, NULL, NULL, NULL, $7, clock_timestamp())",
    )
    .bind(reviewable_id)
    .bind(user.id)
    .bind(score_type as i32)
    .bind(PENDING)
    .bind(score)
    .bind(accuracy)
    .bind(created_at)
    .execute(&mut *conn)
    .await?;
    // update(score:, latest_score:, force_review:); still pending, so
    // notify_reviewable again on commit.
    sqlx::query(
        "UPDATE reviewables SET score = score + $2, latest_score = $3, updated_at = clock_timestamp() \
         WHERE id = $1",
    )
    .bind(reviewable_id)
    .bind(score)
    .bind(created_at)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_reviewable",
        json!({ "reviewable_id": reviewable_id }),
    )
    .await?;
    // topic.update(reviewable_score: topic.reviewable_score + rs.score)
    update_topic_reviewable_score(conn, s, topic_id, score).await?;
    Ok(score)
}

/// `topic.update(reviewable_score: ...)`: a validated save, so Topic's
/// before_validation assigns the cleaned title back through `title=`, which
/// recomputes the slug and clears fancy_title (read back lazily later).
/// The row is written when anything changed.
async fn update_topic_reviewable_score(
    conn: &mut PgConnection,
    s: &SiteSettings,
    topic_id: i32,
    delta: f64,
) -> Result<(), AppError> {
    let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let options = TitleOptions {
        prettify: s.get("title_prettify")?.truthy(),
        allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
        remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
    };
    if clean_title(&title, &options) != title {
        return Err(Unsupported("saving a topic whose title the cleaner changes").into());
    }
    if s.get("slug_generation_method")?.to_s() != "ascii" {
        return Err(Unsupported("slug_generation_method other than ascii").into());
    }
    let new_slug = slug_for(&title)?;
    sqlx::query(
        "UPDATE topics SET reviewable_score = reviewable_score + $2, slug = $3, fancy_title = NULL, \
                           updated_at = clock_timestamp() \
         WHERE id = $1 AND ($2 <> 0 OR slug IS DISTINCT FROM $3 OR fancy_title IS NOT NULL)",
    )
    .bind(topic_id)
    .bind(delta)
    .bind(&new_slug)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::accuracy_bonus;

    #[test]
    fn accuracy_bonus_like_rails() {
        assert_eq!(accuracy_bonus(3, 2), 0.0);
        // 10 agreed of 10: log4(10) * 5
        assert_eq!(accuracy_bonus(10, 0), 8.3);
        // 3 of 10: (0.7 - 0.3) / 0.7 * -log4(10) * 5
        assert_eq!(accuracy_bonus(3, 7), -4.75);
    }
}
