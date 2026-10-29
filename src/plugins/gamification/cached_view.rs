//! LeaderboardCachedView: a leaderboard's positions per period, kept in a
//! materialized view each (`gamification_leaderboard_cache_<id>_<period>`)
//! over its daily scores, its eligible users ranked by the
//! score_ranking_strategy setting. Each view carries its query's
//! signature as its comment, so a view built from an older query reads as
//! stale. The query text is the plugin's, so views Rails built carry the
//! same signature and stay in use.

use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use super::{PERIODS, PluginError};
use crate::site_settings::SiteSettings;

/// `NotReadyError`: the view to read from doesn't exist yet.
#[derive(Debug)]
pub struct NotReady;

/// A position row as `scores` selects it (users' id, name, username,
/// uploaded_avatar_id, then the view's total_score and position).
#[derive(Debug, Clone, sqlx::FromRow, PartialEq)]
pub struct Position {
    pub id: i32,
    pub name: Option<String>,
    pub username: String,
    pub uploaded_avatar_id: Option<i32>,
    pub total_score: i64,
    pub position: i64,
}

/// `PERIOD_INTERVALS`, NULL for all_time.
fn period_start_sql(period: &str) -> &'static str {
    match period {
        "yearly" => "CURRENT_DATE - INTERVAL '1 year'",
        "quarterly" => "CURRENT_DATE - INTERVAL '3 months'",
        "monthly" => "CURRENT_DATE - INTERVAL '1 month'",
        "weekly" => "CURRENT_DATE - INTERVAL '1 week'",
        "daily" => "CURRENT_DATE - INTERVAL '1 day'",
        _ => "NULL",
    }
}

/// `SCORE_RANKING_STRATEGY_MAP[SiteSetting.score_ranking_strategy]`
fn ranking_function(settings: &SiteSettings) -> Result<&'static str, PluginError> {
    Ok(
        match settings.get("score_ranking_strategy")?.to_s().as_str() {
            "row_number" => "ROW_NUMBER()",
            "rank" => "RANK()",
            _ => "DENSE_RANK()",
        },
    )
}

/// `total_scores_query(period)`, `:leaderboard_id` left in place.
pub fn total_scores_query(settings: &SiteSettings, period: &str) -> Result<String, PluginError> {
    let start = period_start_sql(period);
    let ranking = ranking_function(settings)?;
    Ok(format!(
        r#"WITH leaderboard AS (
  SELECT * FROM gamification_leaderboards WHERE id = :leaderboard_id
),

leaderboard_users AS (
  SELECT
    u.id
  FROM
    users u
  INNER JOIN
    user_emails ON user_emails.primary = TRUE AND user_emails.user_id = u.id
  CROSS JOIN
    leaderboard lb
  WHERE NOT
    (user_emails.email LIKE '%@anonymized.invalid%')
  AND
    u.staged = FALSE
  AND
    u.active
  AND
    (u.suspended_till IS NULL OR u.suspended_till < CURRENT_TIMESTAMP)
  AND
    u.id > 0
  AND
    (
      NOT EXISTS(SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id)
    )
  AND
    -- Ensure user is a member of included_groups_ids if it's not empty
    (
      (COALESCE(array_length(lb.included_groups_ids, 1), 0) = 0)
      OR
      (EXISTS (SELECT 1 FROM group_users AS gu WHERE gu.group_id = ANY(lb.included_groups_ids) AND gu.user_id = u.id))
    )
  AND
    -- Ensure user is not a member of excluded_groups_ids if it's not empty
    (
      (COALESCE(array_length(lb.excluded_groups_ids, 1), 0) = 0)
      OR
      (NOT EXISTS (SELECT 1 FROM group_users AS gu WHERE gu.group_id = ANY(lb.excluded_groups_ids) AND gu.user_id = u.id))
    )
),

scores AS (
  SELECT
    gls.*
  FROM
    gamification_leaderboard_scores gls
  CROSS JOIN
    leaderboard lb
  WHERE
    gls.leaderboard_id = :leaderboard_id
  AND
    (CASE
      -- Leaderboard with both "to_date" and "from_date" configured.
      -- Filter scores within the configured date range AND
      -- the relative period window
      WHEN lb.from_date IS NOT NULL AND lb.to_date IS NOT NULL THEN
        gls.date BETWEEN GREATEST(lb.from_date, {start}) AND lb.to_date

      -- Leaderboard with only "from_date" configured.
      -- Filter scores starting from the later of leaderboard's "from_date"
      -- and the relative period start date
      WHEN lb.from_date IS NOT NULL AND lb.to_date IS NULL THEN
        gls.date >= GREATEST(lb.from_date, {start})

      -- Leaderboard with only "to_date" configured.
      -- Filter scores up to leaderboard's "to_date" starting from
      -- the relative period start date
      WHEN lb.from_date IS NULL AND lb.to_date IS NOT NULL THEN
        gls.date >= COALESCE({start}, gls.date) AND gls.date <= lb.to_date

      -- Leaderboard with no "from_date" and "to_date" configured.
      -- Filter scores within the relative period window only
      ELSE
        gls.date >= COALESCE({start}, gls.date)
    END)
    AND gls.date <= CURRENT_DATE -- Ensure scores are not from the future
)

SELECT
 lu.id AS user_id,
 SUM(COALESCE(s.score, 0)) AS total_score,
 {ranking} OVER (ORDER BY SUM(COALESCE(s.score, 0)) DESC) AS position
FROM
  leaderboard_users lu
INNER JOIN
  scores s ON s.user_id = lu.id
GROUP BY
  lu.id
ORDER BY
  position ASC,
  user_id ASC
"#
    ))
}

/// `query_signature`: SHA-256 of the query, trimmed, whitespace runs as
/// single spaces.
pub fn query_signature(query: &str) -> String {
    let normalized = query.split_whitespace().collect::<Vec<_>>().join(" ");
    Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `mview_name(period)`
pub fn mview_name(leaderboard_id: i64, period: &str) -> String {
    format!("gamification_leaderboard_cache_{leaderboard_id}_{period}")
}

/// `mview_exists?(period)`: in the current schema only.
pub async fn mview_exists(
    conn: &mut PgConnection,
    leaderboard_id: i64,
    period: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews \
         WHERE schemaname = current_schema() AND matviewname = $1)",
    )
    .bind(mview_name(leaderboard_id, period))
    .fetch_one(conn)
    .await
}

/// `create`: the views not there yet, each with its unique user index
/// and signature.
pub async fn create(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    leaderboard_id: i64,
) -> Result<(), PluginError> {
    for period in PERIODS {
        if mview_exists(conn, leaderboard_id, period).await? {
            continue;
        }
        let name = mview_name(leaderboard_id, period);
        let select = total_scores_query(settings, period)?;
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        sqlx::query(&format!(
            "CREATE MATERIALIZED VIEW IF NOT EXISTS {name} AS\n{}",
            select.replace(":leaderboard_id", &leaderboard_id.to_string())
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS user_id_{leaderboard_id}_{period}_index ON {name} (user_id)"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::query(&format!(
            "COMMENT ON MATERIALIZED VIEW {name} IS '{}'",
            query_signature(&select)
        ))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}

/// `refresh`: the existing views refreshed concurrently.
pub async fn refresh(conn: &mut PgConnection, leaderboard_id: i64) -> Result<(), PluginError> {
    for period in PERIODS {
        if mview_exists(conn, leaderboard_id, period).await? {
            sqlx::query(&format!(
                "REFRESH MATERIALIZED VIEW CONCURRENTLY {}",
                mview_name(leaderboard_id, period)
            ))
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// `delete`: works for a leaderboard already gone too
/// (DeletedGamificationLeaderboard).
pub async fn delete(conn: &mut PgConnection, leaderboard_id: i64) -> Result<(), PluginError> {
    for period in PERIODS {
        sqlx::query(&format!(
            "DROP MATERIALIZED VIEW IF EXISTS {} CASCADE",
            mview_name(leaderboard_id, period)
        ))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `stale_mview?(period)`: an existing view whose comment isn't the
/// current query's signature.
async fn stale_mview(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    leaderboard_id: i64,
    period: &str,
) -> Result<bool, PluginError> {
    if !mview_exists(conn, leaderboard_id, period).await? {
        return Ok(false);
    }
    let current: Option<String> =
        sqlx::query_scalar("SELECT obj_description($1::regclass::oid, 'pg_class')")
            .bind(mview_name(leaderboard_id, period))
            .fetch_one(&mut *conn)
            .await?;
    Ok(match current {
        None => true,
        Some(sig) => sig != query_signature(&total_scores_query(settings, period)?),
    })
}

/// `stale_mviews`: every period's view once any is stale.
async fn stale_mviews(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    leaderboard_id: i64,
) -> Result<Vec<String>, PluginError> {
    let mut any = false;
    for period in PERIODS {
        if stale_mview(conn, settings, leaderboard_id, period).await? {
            any = true;
            break;
        }
    }
    Ok(if any {
        PERIODS
            .iter()
            .map(|p| mview_name(leaderboard_id, p))
            .collect()
    } else {
        Vec::new()
    })
}

/// `stale?`
pub async fn is_stale(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    leaderboard_id: i64,
) -> Result<bool, PluginError> {
    Ok(!stale_mviews(conn, settings, leaderboard_id)
        .await?
        .is_empty())
}

/// `purge_stale`
pub async fn purge_stale(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    leaderboard_id: i64,
) -> Result<(), PluginError> {
    let list = stale_mviews(conn, settings, leaderboard_id).await?;
    if !list.is_empty() {
        sqlx::query(&format!(
            "DROP MATERIALIZED VIEW IF EXISTS {} CASCADE",
            list.join(", ")
        ))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `scores(period:, for_user_id:, limit:, offset:)`: the positions by
/// position then user id; NotReady without the period's view.
pub async fn scores(
    conn: &mut PgConnection,
    leaderboard_id: i64,
    period: &str,
    for_user_id: Option<i32>,
    limit: Option<i64>,
    offset: i64,
) -> Result<Result<Vec<Position>, NotReady>, PluginError> {
    if !mview_exists(conn, leaderboard_id, period).await? {
        return Ok(Err(NotReady));
    }
    let filter = for_user_id
        .map(|id| format!("WHERE (users.id = {id}) "))
        .unwrap_or_default();
    let limit = limit.map(|l| format!(" LIMIT {l}")).unwrap_or_default();
    let offset = if offset != 0 {
        format!(" OFFSET {offset}")
    } else {
        String::new()
    };
    let rows = sqlx::query_as(&format!(
        "SELECT users.id, users.name, users.username, users.uploaded_avatar_id, p.total_score::bigint AS total_score, p.position \
         FROM users INNER JOIN {} p ON  p.user_id = users.id {filter}\
         ORDER BY \"position\" ASC, \"users\".\"id\" ASC{limit}{offset}",
        mview_name(leaderboard_id, period)
    ))
    .fetch_all(&mut *conn)
    .await?;
    Ok(Ok(rows))
}

/// The leaderboards' ids, in `find_each` order.
async fn all_ids(conn: &mut PgConnection) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM gamification_leaderboards ORDER BY id")
        .fetch_all(conn)
        .await
}

/// `create_all`
pub async fn create_all(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<(), PluginError> {
    for id in all_ids(conn).await? {
        create(conn, settings, id).await?;
    }
    Ok(())
}

/// `refresh_all`
pub async fn refresh_all(conn: &mut PgConnection) -> Result<(), PluginError> {
    for id in all_ids(conn).await? {
        refresh(conn, id).await?;
    }
    Ok(())
}

/// `purge_all_stale`
pub async fn purge_all_stale(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<(), PluginError> {
    for id in all_ids(conn).await? {
        purge_stale(conn, settings, id).await?;
    }
    Ok(())
}

/// `update_all`: the stale views purged and the missing ones created, in
/// one transaction.
pub async fn update_all(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<(), PluginError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    purge_all_stale(&mut tx, settings).await?;
    create_all(&mut tx, settings).await?;
    tx.commit().await?;
    Ok(())
}

/// `regenerate_all`: every view dropped and built again, in one
/// transaction.
pub async fn regenerate_all(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<(), PluginError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    for id in all_ids(&mut tx).await? {
        delete(&mut tx, id).await?;
    }
    create_all(&mut tx, settings).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_ignore_whitespace() {
        assert_eq!(
            query_signature("SELECT  1\n\n"),
            query_signature(" SELECT 1")
        );
        assert_ne!(query_signature("SELECT 1"), query_signature("SELECT 2"));
    }
}
