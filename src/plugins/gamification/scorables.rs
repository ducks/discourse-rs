//! The score sources (lib/discourse_gamification/scorables): each one's
//! query of the points a user earned per day since a date, its weight
//! (the leaderboard's `score_overrides`, else its `<key>_score_value`
//! setting) and its category filter, and
//! GamificationLeaderboardScore.calculate_scores, which rolls them up with
//! the score events into a leaderboard's daily scores.
//!
//! The SQL is the plugin's, with `:since` inlined as the quoted literal
//! ActiveRecord's sanitize_sql_array writes (an untyped literal, so it
//! reads as a date against a date column and as a timestamp against a
//! timestamp).

use chrono::{NaiveDate, NaiveDateTime};
use sqlx::PgConnection;

use super::Leaderboard;
use crate::site_settings::{SettingError, SiteSettings};

/// The scorables, by `scorable_key`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scorable {
    DayVisited,
    FlagCreated,
    LikeGiven,
    LikeReceived,
    PostCreated,
    PostRead,
    Solutions,
    TimeRead,
    TopicCreated,
    UserInvited,
    ReactionGiven,
    ReactionReceived,
    ChatReactionGiven,
    ChatReactionReceived,
    ChatMessageCreated,
}

/// `Scorable.subclasses`, in the order plugin.rb requires them.
pub const ALL: [Scorable; 15] = [
    Scorable::DayVisited,
    Scorable::FlagCreated,
    Scorable::LikeGiven,
    Scorable::LikeReceived,
    Scorable::PostCreated,
    Scorable::PostRead,
    Scorable::Solutions,
    Scorable::TimeRead,
    Scorable::TopicCreated,
    Scorable::UserInvited,
    Scorable::ReactionGiven,
    Scorable::ReactionReceived,
    Scorable::ChatReactionGiven,
    Scorable::ChatReactionReceived,
    Scorable::ChatMessageCreated,
];

/// `GamificationLeaderboard::VALID_SCORABLE_KEYS`
pub const VALID_KEYS: [&str; 15] = [
    "like_given",
    "like_received",
    "solution",
    "user_invited",
    "time_read",
    "post_read",
    "topic_created",
    "post_created",
    "flag_created",
    "day_visited",
    "reaction_received",
    "reaction_given",
    "chat_reaction_received",
    "chat_reaction_given",
    "chat_message_created",
];

/// A `:since` value as Rails passes one: a Date, a Time, or (from a job's
/// arguments) a string.
#[derive(Clone, Debug, PartialEq)]
pub enum Since {
    Date(NaiveDate),
    Time(NaiveDateTime),
    Text(String),
}

impl Since {
    /// `connection.quote(value)`: `'2024-01-01'`, `'2024-01-01 12:00:00'`
    /// (with microseconds when there are any), or the string quoted.
    pub fn literal(&self) -> String {
        let s = match self {
            Since::Date(d) => d.format("%Y-%m-%d").to_string(),
            Since::Time(t) => {
                if t.and_utc().timestamp_subsec_micros() == 0 {
                    t.format("%Y-%m-%d %H:%M:%S").to_string()
                } else {
                    t.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
                }
            }
            Since::Text(s) => s.clone(),
        };
        format!("'{}'", s.replace('\'', "''"))
    }
}

impl Scorable {
    /// `scorable_key`: the class name underscored, `solution` for Solutions.
    pub fn key(self) -> &'static str {
        match self {
            Scorable::DayVisited => "day_visited",
            Scorable::FlagCreated => "flag_created",
            Scorable::LikeGiven => "like_given",
            Scorable::LikeReceived => "like_received",
            Scorable::PostCreated => "post_created",
            Scorable::PostRead => "post_read",
            Scorable::Solutions => "solution",
            Scorable::TimeRead => "time_read",
            Scorable::TopicCreated => "topic_created",
            Scorable::UserInvited => "user_invited",
            Scorable::ReactionGiven => "reaction_given",
            Scorable::ReactionReceived => "reaction_received",
            Scorable::ChatReactionGiven => "chat_reaction_given",
            Scorable::ChatReactionReceived => "chat_reaction_received",
            Scorable::ChatMessageCreated => "chat_message_created",
        }
    }

    /// `score_multiplier(leaderboard:)`: the leaderboard's override, else
    /// the setting.
    pub fn multiplier(
        self,
        settings: &SiteSettings,
        leaderboard: Option<&Leaderboard>,
    ) -> Result<i64, SettingError> {
        if let Some(v) = leaderboard.and_then(|lb| lb.score_override_for(self.key())) {
            return Ok(v);
        }
        Ok(settings.get(&format!("{}_score_value", self.key()))?.to_i())
    }

    /// `enabled?(leaderboard:)`: a positive weight, and for the plugins'
    /// sources their plugin on.
    pub fn enabled(
        self,
        settings: &SiteSettings,
        leaderboard: Option<&Leaderboard>,
    ) -> Result<bool, SettingError> {
        let plugin = match self {
            Scorable::Solutions => Some("solved_enabled"),
            Scorable::ReactionGiven | Scorable::ReactionReceived => {
                Some("discourse_reactions_enabled")
            }
            Scorable::ChatReactionGiven
            | Scorable::ChatReactionReceived
            | Scorable::ChatMessageCreated => Some("chat_enabled"),
            _ => None,
        };
        if let Some(setting) = plugin
            && !settings.get(setting)?.truthy()
        {
            return Ok(false);
        }
        Ok(self.multiplier(settings, leaderboard)? > 0)
    }

    /// The category filter's list: the leaderboard's categories, else the
    /// `scorable_categories` setting; empty for none.
    fn category_list(
        settings: &SiteSettings,
        leaderboard: Option<&Leaderboard>,
    ) -> Result<String, SettingError> {
        Ok(
            match leaderboard.and_then(|lb| lb.scorable_category_ids.as_ref()) {
                Some(ids) => ids
                    .iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                None => settings
                    .get("scorable_categories")?
                    .to_s()
                    .split('|')
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::ruby::to_i(s).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            },
        )
    }

    fn category_filter(
        settings: &SiteSettings,
        leaderboard: Option<&Leaderboard>,
        column: &str,
    ) -> Result<String, SettingError> {
        let list = Self::category_list(settings, leaderboard)?;
        if list.is_empty() {
            return Ok(String::new());
        }
        Ok(format!("AND {column} IN ({list})\n"))
    }

    /// `query(leaderboard:)`, `:since` left in place.
    pub fn query(
        self,
        settings: &SiteSettings,
        leaderboard: Option<&Leaderboard>,
    ) -> Result<String, SettingError> {
        let m = self.multiplier(settings, leaderboard)?;
        let t_filter = || Self::category_filter(settings, leaderboard, "t.category_id");
        Ok(match self {
            Scorable::DayVisited => format!(
                "SELECT\n  uv.user_id AS user_id,\n  date_trunc('day', uv.visited_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  user_visits AS uv\nWHERE\n  uv.visited_at >= :since\nGROUP BY\n  1, 2\n"
            ),
            Scorable::FlagCreated => format!(
                "SELECT\n  r.created_by_id AS user_id,\n  date_trunc('day', r.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  reviewables AS r\nWHERE\n  created_at >= :since AND\n  status = 1 \nGROUP BY\n  1, 2\n"
            ),
            Scorable::LikeGiven | Scorable::LikeReceived => {
                let user = if self == Scorable::LikeGiven {
                    "pa.user_id"
                } else {
                    "p.user_id"
                };
                format!(
                    "SELECT\n  {user} AS user_id,\n  date_trunc('day', pa.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  post_actions AS pa\nINNER JOIN posts AS p\n  ON p.id = pa.post_id\nINNER JOIN topics AS t\n  ON t.id = p.topic_id\n  {}\nWHERE\n  p.deleted_at IS NULL AND\n  t.archetype <> 'private_message' AND\n  p.wiki IS FALSE AND\n  post_action_type_id = 2 AND\n  pa.created_at >= :since\nGROUP BY\n  1, 2\n",
                    t_filter()?
                )
            }
            Scorable::PostCreated => format!(
                "SELECT\n  p.user_id AS user_id,\n  date_trunc('day', p.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  posts AS p\nINNER JOIN topics AS t\n  ON t.id = p.topic_id\n  {}\nWHERE\n  p.deleted_at IS NULL AND\n  t.deleted_at IS NULL AND\n  t.archetype <> 'private_message' AND\n  p.post_number <> 1 AND\n  p.post_type = 1 AND\n  p.wiki IS FALSE AND\n  p.hidden IS FALSE AND\n  p.created_at >= :since\nGROUP BY\n  1, 2\n",
                t_filter()?
            ),
            Scorable::PostRead => format!(
                "SELECT\n  uv.user_id AS user_id,\n  date_trunc('day', uv.visited_at) AS date,\n  SUM(uv.posts_read) / 100 * {m} AS points\nFROM\n  user_visits AS uv\nWHERE\n  uv.visited_at >= :since AND\n  uv.posts_read >= 5\nGROUP BY\n  1, 2\n"
            ),
            Scorable::Solutions => format!(
                "SELECT\n  posts.user_id AS user_id,\n  date_trunc('day', dsta.created_at) AS date,\n  COUNT(dsst.topic_id) * {m} AS points\nFROM\n  discourse_solved_solved_topics dsst\nINNER JOIN topics\n  ON dsst.topic_id = topics.id\n  {}\nINNER JOIN discourse_solved_topic_answers dsta \n  ON dsta.solved_topic_id = dsst.id\nINNER JOIN posts \n  ON posts.id = dsta.answer_post_id\nWHERE\n  posts.deleted_at IS NULL AND\n  topics.deleted_at IS NULL AND\n  topics.archetype <> 'private_message' AND\n  posts.user_id != topics.user_id AND\n  dsta.created_at >= :since\nGROUP BY\n  1, 2\n",
                Self::category_filter(settings, leaderboard, "topics.category_id")?
            ),
            Scorable::TimeRead => format!(
                "SELECT\n  uv.user_id AS user_id,\n  date_trunc('day', uv.visited_at) AS date,\n  SUM(uv.time_read) / 3600 * {m} AS points\nFROM\n  user_visits AS uv\nWHERE\n  uv.visited_at >= :since AND\n  uv.time_read >= 60\nGROUP BY\n  1, 2\n"
            ),
            Scorable::TopicCreated => format!(
                "SELECT\n  t.user_id AS user_id,\n  date_trunc('day', t.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  topics AS t\nWHERE\n  t.deleted_at IS NULL AND\n  t.archetype <> 'private_message' AND\n  t.created_at >= :since\n  {}\nGROUP BY\n  1, 2\n",
                t_filter()?
            ),
            Scorable::UserInvited => format!(
                "SELECT\n  inv.invited_by_id AS user_id,\n  date_trunc('day', inv.created_at) AS date,\n  SUM(inv.redemption_count * {m}) AS points\nFROM\n  invites AS inv\nWHERE\n  inv.created_at >= :since AND\n  inv.redemption_count > 0\nGROUP BY\n  1, 2\n"
            ),
            Scorable::ReactionGiven => format!(
                "SELECT\n  reactions.user_id AS user_id,\n  date_trunc('day', reactions.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  discourse_reactions_reaction_users AS reactions\nINNER JOIN posts AS p\n  ON p.id = reactions.post_id\nINNER JOIN topics AS t\n  ON t.id = p.topic_id\n  {}\nWHERE\n  p.deleted_at IS NULL AND\n  t.deleted_at IS NULL AND\n  p.wiki IS FALSE AND\n  reactions.created_at >= :since\nGROUP BY\n  1, 2\n",
                t_filter()?
            ),
            Scorable::ReactionReceived => format!(
                "SELECT\n  p.user_id AS user_id,\n  date_trunc('day', reactions.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\ndiscourse_reactions_reaction_users AS reactions\nINNER JOIN posts AS p\n  ON p.id = reactions.post_id\nINNER JOIN topics AS t\n  ON t.id = p.topic_id\n  {}\nWHERE\n  p.deleted_at IS NULL AND\n  t.archetype <> 'private_message' AND\n  p.wiki IS FALSE AND\n  reactions.created_at >= :since\nGROUP BY\n  1, 2\n",
                t_filter()?
            ),
            Scorable::ChatReactionGiven | Scorable::ChatReactionReceived => {
                let user = if self == Scorable::ChatReactionGiven {
                    "reactions.user_id"
                } else {
                    "cm.user_id"
                };
                format!(
                    "SELECT\n  {user} AS user_id,\n  date_trunc('day', reactions.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  chat_message_reactions AS reactions\nINNER JOIN chat_messages AS cm\n  ON cm.id = reactions.chat_message_id\nINNER JOIN chat_channels AS cc\n  ON cc.id = cm.chat_channel_id\nWHERE\n  cc.deleted_at IS NULL AND\n  cm.deleted_at IS NULL AND\n  cm.user_id <> reactions.user_id AND\n  reactions.created_at >= :since\nGROUP BY\n  1, 2\n"
                )
            }
            Scorable::ChatMessageCreated => format!(
                "SELECT\n  m.user_id,\n  date_trunc('day', m.created_at) AS date,\n  COUNT(*) * {m} AS points\nFROM\n  chat_messages AS m\nJOIN\n  chat_channels AS c ON c.id = m.chat_channel_id\nLEFT JOIN (\n  SELECT direct_message_channel_id\n  FROM direct_message_users\n  GROUP BY direct_message_channel_id\n  HAVING COUNT(DISTINCT user_id) > 1\n) AS dm ON dm.direct_message_channel_id = c.chatable_id\nWHERE\n  m.created_at >= :since AND\n  m.deleted_at IS NULL AND\n  (c.chatable_type <> 'DirectMessage' OR dm.direct_message_channel_id IS NOT NULL)\nGROUP BY\n  m.user_id, date_trunc('day', m.created_at)\n"
            ),
        })
    }
}

/// A query with `:since` bound.
pub fn bind_since(sql: &str, since: &Since) -> String {
    sql.replace(":since", &since.literal())
}

/// `scorables_queries(leaderboard:)`: the enabled sources' queries in
/// parentheses, joined by UNION ALL.
pub fn scorables_queries(
    settings: &SiteSettings,
    leaderboard: Option<&Leaderboard>,
) -> Result<String, SettingError> {
    let mut parts = Vec::new();
    for s in ALL {
        if s.enabled(settings, leaderboard)? {
            parts.push(format!("( {} )", s.query(settings, leaderboard)?));
        }
    }
    Ok(parts.join(" UNION ALL "))
}

/// `GamificationLeaderboardScore.calculate_scores(leaderboard, since_date:,
/// only_subclass:)`: the leaderboard's scores from the date replaced by
/// what its sources and the score events give, per user and day, for
/// active users not suspended.
pub async fn calculate_scores(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    leaderboard: &Leaderboard,
    since: &Since,
    only: Option<Scorable>,
) -> Result<(), super::PluginError> {
    let queries = match only {
        Some(s) => s.query(settings, Some(leaderboard))?,
        None => scorables_queries(settings, Some(leaderboard))?,
    };
    let lit = since.literal();
    sqlx::query(&format!(
        "DELETE FROM gamification_leaderboard_scores WHERE leaderboard_id = {} AND date >= {lit}",
        leaderboard.id
    ))
    .execute(&mut *conn)
    .await?;
    let score_event_query = "SELECT user_id, date, SUM(points) AS points\nFROM gamification_score_events\nWHERE date >= :since\nGROUP BY 1, 2\n";
    let source = if queries.trim().is_empty() {
        score_event_query.to_string()
    } else {
        format!("{queries} UNION ALL {score_event_query}")
    };
    let sql = format!(
        "INSERT INTO gamification_leaderboard_scores (leaderboard_id, user_id, date, score)\n\
         SELECT {id}, user_id, date, SUM(points) AS score\n\
         FROM (\n  {source}\n) AS source\n\
         JOIN users AS u ON u.id = source.user_id\n\
         WHERE user_id IS NOT NULL\n  \
           AND (u.suspended_till IS NULL OR u.suspended_till < CURRENT_TIMESTAMP)\n  \
           AND u.active\n\
         GROUP BY user_id, date\n\
         ON CONFLICT (leaderboard_id, user_id, date) DO UPDATE\n\
         SET score = EXCLUDED.score;",
        id = leaderboard.id
    );
    sqlx::query(&bind_since(&sql, since))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `GamificationLeaderboardScore.calculate_all(since_date:)`: every
/// leaderboard not over before the date.
pub async fn calculate_all(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    since: &Since,
) -> Result<(), super::PluginError> {
    let leaderboards: Vec<Leaderboard> = sqlx::query_as(&format!(
        "SELECT * FROM gamification_leaderboards WHERE to_date IS NULL OR to_date >= {} ORDER BY id",
        since.literal()
    ))
    .fetch_all(&mut *conn)
    .await?;
    for lb in &leaderboards {
        calculate_scores(conn, settings, lb, since, None).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_literals_are_what_active_record_quotes() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert_eq!(Since::Date(d).literal(), "'2024-01-01'");
        let t = d.and_hms_opt(12, 0, 0).unwrap();
        assert_eq!(Since::Time(t).literal(), "'2024-01-01 12:00:00'");
        let t = d.and_hms_micro_opt(12, 0, 0, 5).unwrap();
        assert_eq!(Since::Time(t).literal(), "'2024-01-01 12:00:00.000005'");
        assert_eq!(Since::Text("it's".into()).literal(), "'it''s'");
    }

    #[test]
    fn every_source_has_a_valid_key() {
        for s in ALL {
            assert!(VALID_KEYS.contains(&s.key()), "{}", s.key());
        }
    }
}
