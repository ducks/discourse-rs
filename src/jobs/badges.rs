//! The badge jobs: BadgeGrant (a backfill of every enabled badge, each
//! its own BackfillBadge job), EnsureBadgesConsistency, and the badges
//! granted by date: Anniversary and New User of the Month.

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::{Value, json};

use crate::badge_granter::{self, Badge, GrantOpts};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `Jobs::BadgeGrant`, daily.
pub const BADGE_GRANT: &str = "badge_grant";
/// `Jobs::BackfillBadge`
pub const BACKFILL_BADGE: &str = "backfill_badge";
/// `Jobs::EnsureBadgesConsistency`, daily.
pub const ENSURE_BADGES_CONSISTENCY: &str = "ensure_badges_consistency";
/// `Jobs::GrantAnniversaryBadges`, daily.
pub const GRANT_ANNIVERSARY_BADGES: &str = "grant_anniversary_badges";
/// `Jobs::GrantNewUserOfTheMonthBadges`, daily.
pub const GRANT_NEW_USER_OF_THE_MONTH_BADGES: &str = "grant_new_user_of_the_month_badges";

/// `Badge::Anniversary`
const ANNIVERSARY: i32 = 24;
/// `Badge::NewUserOfTheMonth`
const NEW_USER_OF_THE_MONTH: i32 = 44;
/// `GrantNewUserOfTheMonthBadges::MAX_AWARDED`
const MAX_AWARDED: i64 = 2;

async fn settings(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
) -> Result<SiteSettings, AppError> {
    Ok(SiteSettings::load(conn, &state.site_setting_defs, &state.config.globals).await?)
}

/// `Jobs::BadgeGrant#execute`: a BackfillBadge job per enabled badge.
pub async fn badge_grant(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("enable_badges")?.truthy() {
        return Ok(());
    }
    // Badge.enabled.pluck(:id): in the table's own order, as Rails reads it.
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM badges WHERE enabled")
        .fetch_all(&mut *tx)
        .await?;
    for id in ids {
        super::enqueue(&mut tx, BACKFILL_BADGE, json!({ "badge_id": id })).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::BackfillBadge#execute`: the badge's full backfill, then the
/// titles of the users it revoked, and the featured ranks and distinct
/// badge counts of everyone it touched.
pub async fn backfill_badge(state: &AppState, args: &Value) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("enable_badges")?.truthy() {
        return Ok(());
    }
    // Badge.enabled.find_by(id: args[:badge_id])
    let id = match &args["badge_id"] {
        Value::Number(n) => n.as_i64(),
        Value::String(v) => Some(crate::ruby::to_i(v)),
        _ => None,
    };
    let Some(id) = id.and_then(|i| i32::try_from(i).ok()) else {
        return Ok(());
    };
    let Some(badge) = Badge::find(&mut tx, id).await?.filter(|b| b.enabled) else {
        return Ok(());
    };
    let done = badge_granter::backfill(&mut tx, &state.bus, &s, &state.i18n, &badge, None).await?;
    let mut revoked = done.revoked;
    revoked.sort_unstable();
    revoked.dedup();
    let mut affected: Vec<i32> = revoked.iter().chain(&done.granted).copied().collect();
    affected.sort_unstable();
    affected.dedup();
    if !revoked.is_empty() {
        badge_granter::revoke_ungranted_titles(&mut tx, Some(&revoked)).await?;
    }
    if !affected.is_empty() {
        badge_granter::update_featured_ranks(&mut tx, Some(&affected)).await?;
        badge_granter::update_distinct_badge_count(&mut tx, Some(&affected)).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `Jobs::EnsureBadgesConsistency#execute`
pub async fn ensure_badges_consistency(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("enable_badges")?.truthy() {
        return Ok(());
    }
    badge_granter::revoke_ungranted_titles(&mut tx, None).await?;
    badge_granter::update_featured_ranks(&mut tx, None).await?;
    badge_granter::update_distinct_badge_count(&mut tx, None).await?;
    tx.commit().await?;
    Ok(())
}

/// `Jobs::GrantAnniversaryBadges#execute` for its default start (a year
/// ago): the Anniversary badge, dated a year after the start, for each
/// user who posted in the year since and hasn't had it for that year.
pub async fn grant_anniversary_badges(state: &AppState) -> Result<(), AppError> {
    let mut tx = state.pool.begin().await?;
    let s = settings(state, &mut tx).await?;
    if !s.get("enable_badges")?.truthy() {
        return Ok(());
    }
    let Some(badge) = Badge::find(&mut tx, ANNIVERSARY)
        .await?
        .filter(|b| b.enabled)
    else {
        return Ok(());
    };
    let now = crate::clock::now_naive();
    let start = years_ago(now);
    let end = start.checked_add_months(Months::new(12)).unwrap_or(start);
    // BadgeQueries.anniversaries(start_date, end_date)
    let user_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT u.id FROM users AS u \
         JOIN posts AS p ON p.user_id = u.id \
         JOIN topics AS t ON p.topic_id = t.id \
         WHERE u.id > 0 AND u.active AND NOT u.staged \
           AND (u.silenced_till IS NULL OR u.silenced_till < $1) \
           AND (u.suspended_till IS NULL OR u.suspended_till < $1) \
           AND u.created_at <= $1 \
           AND NOT p.hidden AND p.deleted_at IS NULL \
           AND p.created_at BETWEEN $1 AND $2 \
           AND t.visible AND t.archetype <> 'private_message' AND t.deleted_at IS NULL \
           AND NOT EXISTS (SELECT 1 FROM user_badges AS ub WHERE ub.user_id = u.id \
                           AND ub.badge_id = $3 AND ub.granted_at BETWEEN $1 AND $2) \
           AND NOT EXISTS (SELECT 1 FROM anonymous_users AS au WHERE au.user_id = u.id) \
         GROUP BY u.id HAVING COUNT(p.id) > 0 ORDER BY u.id",
    )
    .bind(start)
    .bind(end)
    .bind(ANNIVERSARY)
    .fetch_all(&mut *tx)
    .await?;
    for user_id in user_ids {
        badge_granter::grant(
            &mut tx,
            &state.bus,
            &s,
            &state.i18n,
            &badge,
            user_id,
            &GrantOpts {
                created_at: Some(end),
                ..GrantOpts::system(None)
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `1.year.ago`
fn years_ago(now: NaiveDateTime) -> NaiveDateTime {
    now.checked_sub_months(Months::new(12)).unwrap_or(now)
}

/// `1.month.ago.beginning_of_month` and `.end_of_month`.
fn previous_month(now: NaiveDateTime) -> (NaiveDateTime, NaiveDateTime) {
    let month_ago = now.checked_sub_months(Months::new(1)).unwrap_or(now);
    let first =
        NaiveDate::from_ymd_opt(month_ago.year(), month_ago.month(), 1).unwrap_or(month_ago.date());
    let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
    let begin = first.and_time(NaiveTime::MIN);
    let end = next.and_time(NaiveTime::MIN) - chrono::Duration::microseconds(1);
    (begin, end)
}

/// `Jobs::GrantNewUserOfTheMonthBadges#execute`: unless someone got it
/// since the start of last month, the badge for the (at most two) users
/// who joined last month with the best like score, each with the
/// new_user_of_the_month system message.
pub async fn grant_new_user_of_the_month_badges(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    // Badge.find: a missing badge fails the job.
    let badge = Badge::find(&mut conn, NEW_USER_OF_THE_MONTH)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    let s = settings(state, &mut conn).await?;
    if !s.get("enable_badges")?.truthy() || !badge.enabled {
        return Ok(());
    }
    let now = crate::clock::now_naive();
    let (begin, end) = previous_month(now);
    let granted: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_badges WHERE badge_id = $1 AND granted_at BETWEEN $2 AND $3)",
    )
    .bind(badge.id)
    .bind(begin)
    .bind(now)
    .fetch_one(&mut *conn)
    .await?;
    if granted {
        return Ok(());
    }
    // scores(previous_month_beginning, previous_month_end)
    let scores: Vec<(i32, f64)> = sqlx::query_as(
        "SELECT u.id, \
           SUM(CASE WHEN pa.id IS NOT NULL THEN \
                 CASE WHEN liked_by.id <= 0 THEN 0.0 \
                      WHEN liked_by.admin THEN 3.0 \
                      WHEN liked_by.moderator THEN 3.0 \
                      WHEN liked_by.trust_level = 4 THEN 2.0 \
                      WHEN liked_by.trust_level = 3 THEN 1.5 \
                      WHEN liked_by.trust_level = 2 THEN 1.0 \
                      WHEN liked_by.trust_level = 1 THEN 0.25 \
                      WHEN liked_by.trust_level = 0 THEN 0.1 \
                      ELSE 1.0 END \
               ELSE 0 END) / (5 + COUNT(DISTINCT p.id))::float AS score \
         FROM users AS u \
         INNER JOIN user_stats AS us ON u.id = us.user_id \
         LEFT OUTER JOIN posts AS p ON p.user_id = u.id \
         LEFT OUTER JOIN post_actions AS pa ON pa.post_id = p.id AND pa.post_action_type_id = 2 \
         LEFT OUTER JOIN users AS liked_by ON liked_by.id = pa.user_id \
         LEFT OUTER JOIN topics AS t ON t.id = p.topic_id \
         WHERE u.active AND u.id > 0 \
           AND u.id NOT IN (SELECT user_id FROM user_badges WHERE badge_id = $1 UNION SELECT -1) \
           AND NOT u.staged AND NOT u.admin AND NOT u.moderator \
           AND u.suspended_at IS NULL AND u.suspended_till IS NULL \
           AND u.created_at BETWEEN $2 AND $3 \
           AND t.archetype <> 'private_message' AND t.deleted_at IS NULL AND p.deleted_at IS NULL \
         GROUP BY u.id \
         HAVING COUNT(DISTINCT p.id) > 1 AND COUNT(DISTINCT p.topic_id) > 1 AND COUNT(pa.id) > 1 \
         ORDER BY score DESC LIMIT $4",
    )
    .bind(badge.id)
    .bind(begin)
    .bind(end)
    .bind(MAX_AWARDED)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);
    // I18n.l(previous_month_beginning, format: :no_day): "%B %Y" in English.
    if s.get("default_locale")?.to_s() != "en" {
        return Err(crate::Unsupported("new user of the month messages in other locales").into());
    }
    let month_year = begin.format("%B %Y").to_string();
    let base_url = crate::url::Urls {
        config: &state.config,
        settings: &s,
    }
    .base_url()?;
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &s,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    for (user_id, score) in scores {
        if score <= 0.0 {
            continue;
        }
        let mut tx = state.pool.begin().await?;
        // user.badges: the enabled ones.
        let has: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_badges WHERE user_id = $1 AND badge_id = $2)",
        )
        .bind(user_id)
        .bind(badge.id)
        .fetch_one(&mut *tx)
        .await?;
        if has {
            continue;
        }
        badge_granter::grant(
            &mut tx,
            &state.bus,
            &s,
            &state.i18n,
            &badge,
            user_id,
            &GrantOpts {
                created_at: Some(end),
                ..GrantOpts::system(None)
            },
        )
        .await?;
        tx.commit().await?;
        crate::system_message::create(
            &state.pool,
            &ctx,
            user_id,
            "new_user_of_the_month",
            &[
                ("month_year".to_string(), month_year.clone()),
                ("url".to_string(), format!("{base_url}/badges")),
            ],
            None,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_month_spans_the_whole_month() {
        let now = NaiveDate::from_ymd_opt(2026, 3, 31)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let (begin, end) = previous_month(now);
        assert_eq!(begin.to_string(), "2026-02-01 00:00:00");
        assert_eq!(end.to_string(), "2026-02-28 23:59:59.999999");
    }
}
