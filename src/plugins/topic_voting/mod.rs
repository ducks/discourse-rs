//! discourse-topic-voting, on the tables Rails' plugin created
//! (`topic_voting_category_settings`, `topic_voting_votes`,
//! `topic_voting_topic_vote_count`): the keys it adds to topic list
//! items, the topic view, the first post, categories and the current
//! user, the `votes` list order, and voting (`votes`).

pub mod lifecycle;
pub mod votes;

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

/// `DiscourseTopicVoting::BADGE_NAMES`
pub const BADGE_NAMES: [&str; 4] = ["Daydreamer", "Brainstormer", "Innovator", "Visionary"];

/// `enabled_site_setting :topic_voting_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("topic_voting_enabled")?.truthy())
}

/// `Category.can_vote?(category_id)` for one category.
pub async fn category_votes(
    conn: &mut PgConnection,
    category_id: i32,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_voting_category_settings WHERE category_id = $1)",
    )
    .bind(category_id)
    .fetch_one(conn)
    .await
}

/// The voting categories (`Category.can_vote?`, the category settings
/// rows) with their definition topics.
#[derive(Debug, Default)]
pub struct Categories {
    topic_ids: HashMap<i32, Option<i32>>,
}

impl Categories {
    pub async fn load(conn: &mut PgConnection) -> Result<Self, sqlx::Error> {
        let rows: Vec<(i32, Option<i32>)> = sqlx::query_as(
            "SELECT s.category_id, c.topic_id FROM topic_voting_category_settings s \
             JOIN categories c ON c.id = s.category_id",
        )
        .fetch_all(&mut *conn)
        .await?;
        Ok(Categories {
            topic_ids: rows.into_iter().collect(),
        })
    }

    /// `Topic#can_vote?`: a regular topic in a voting category, not the
    /// category's definition topic.
    pub fn can_vote(&self, topic_id: i32, archetype: &str, category_id: Option<i32>) -> bool {
        archetype == "regular"
            && category_id
                .and_then(|c| self.topic_ids.get(&c))
                .is_some_and(|definition| *definition != Some(topic_id))
    }
}

/// The topic list's after-load data: the voting categories, each topic's
/// vote count and those the viewer voted on.
#[derive(Debug, Default)]
pub struct TopicListData {
    pub categories: Categories,
    counts: HashMap<i32, i32>,
    /// `current_user_voted`: any vote of theirs, archived or not.
    voted: HashSet<i32>,
}

impl TopicListData {
    pub async fn load(
        conn: &mut PgConnection,
        topic_ids: &[i32],
        user_id: Option<i32>,
    ) -> Result<Self, sqlx::Error> {
        let categories = Categories::load(conn).await?;
        let counts: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT topic_id, votes_count FROM topic_voting_topic_vote_count WHERE topic_id = ANY($1)",
        )
        .bind(topic_ids)
        .fetch_all(&mut *conn)
        .await?;
        let voted: Vec<i32> = match user_id {
            Some(user_id) => {
                sqlx::query_scalar(
                    "SELECT DISTINCT topic_id FROM topic_voting_votes \
                     WHERE user_id = $1 AND topic_id = ANY($2)",
                )
                .bind(user_id)
                .bind(topic_ids)
                .fetch_all(&mut *conn)
                .await?
            }
            None => Vec::new(),
        };
        Ok(TopicListData {
            categories,
            counts: counts.into_iter().collect(),
            voted: voted.into_iter().collect(),
        })
    }

    /// The topic list item's keys: `vote_count` and `user_voted` for a
    /// topic that can be voted on, `can_vote` for a regular one.
    pub fn list_item_keys(
        &self,
        guardian: &Guardian,
        topic_id: i32,
        archetype: &str,
        category_id: Option<i32>,
        out: &mut Map<String, Value>,
    ) {
        let can_vote = self.categories.can_vote(topic_id, archetype, category_id);
        if can_vote {
            out.insert(
                "vote_count".into(),
                json!(self.counts.get(&topic_id).copied().unwrap_or(0)),
            );
        }
        if archetype == "regular" {
            out.insert("can_vote".into(), json!(can_vote));
        }
        if can_vote {
            let voted = guardian
                .user_id()
                .map(|_| Value::Bool(self.voted.contains(&topic_id)))
                .unwrap_or(Value::Null);
            out.insert("user_voted".into(), voted);
        }
    }
}

/// The topic view's keys: `can_vote`, `vote_count`, `user_voted` (false
/// for an anonymous viewer).
pub async fn topic_view_keys(
    conn: &mut PgConnection,
    guardian: &Guardian,
    topic_id: i32,
    can_vote: bool,
    out: &mut Map<String, Value>,
) -> Result<(), sqlx::Error> {
    let count: Option<i32> = sqlx::query_scalar(
        "SELECT votes_count FROM topic_voting_topic_vote_count WHERE topic_id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let voted = match guardian.user_id() {
        Some(user_id) => sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_voting_votes WHERE user_id = $1 AND topic_id = $2)",
        )
        .bind(user_id)
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?,
        None => false,
    };
    out.insert("can_vote".into(), json!(can_vote));
    out.insert("vote_count".into(), json!(count.unwrap_or(0)));
    out.insert("user_voted".into(), json!(voted));
    Ok(())
}

/// A user's votes against their limit (DiscourseTopicVoting::UserExtension).
#[derive(Debug, Clone, Copy)]
pub struct UserVotes {
    /// `vote_count`: their unarchived votes.
    pub count: i64,
    /// `vote_limit`: nil without vote limits.
    pub limit: Option<i64>,
}

impl UserVotes {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        user_id: i32,
        trust_level: i32,
    ) -> Result<Self, crate::plugins::PluginError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM topic_voting_votes WHERE user_id = $1 AND NOT archive",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        let limit = if settings.get("topic_voting_enable_vote_limits")?.truthy() {
            Some(
                settings
                    .get(&format!("topic_voting_tl{trust_level}_vote_limit"))?
                    .to_i(),
            )
        } else {
            None
        };
        Ok(UserVotes { count, limit })
    }

    /// `reached_voting_limit?`
    pub fn reached(&self) -> bool {
        self.limit.is_some_and(|l| self.count >= l)
    }

    /// `votes_left`
    pub fn left(&self) -> Option<i64> {
        self.limit.map(|l| (l - self.count).max(0))
    }

    /// `alert_low_votes?`
    pub fn alert_low(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        let Some(limit) = self.limit else {
            return Ok(false);
        };
        Ok(limit - self.count <= settings.get("topic_voting_alert_votes_left")?.to_i())
    }
}

/// The current user's keys: `votes_exceeded`, `votes_count`, `votes_left`
/// and `vote_limit`.
pub async fn current_user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    trust_level: i32,
    out: &mut Map<String, Value>,
) -> Result<(), crate::plugins::PluginError> {
    let votes = UserVotes::load(conn, settings, user_id, trust_level).await?;
    out.insert("votes_exceeded".into(), json!(votes.reached()));
    out.insert("votes_count".into(), json!(votes.count));
    out.insert("votes_left".into(), json!(votes.left()));
    out.insert("vote_limit".into(), json!(votes.limit));
    Ok(())
}

/// The `votes` list order (TopicQuery.results_filter_callbacks): by vote
/// count, then bump.
pub fn votes_order(ascending: bool) -> String {
    format!(
        "COALESCE((SELECT votes_count FROM topic_voting_topic_vote_count v \
         WHERE v.topic_id = topics.id), 0) {}, topics.bumped_at DESC",
        if ascending { "ASC" } else { "DESC" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_regular_topics_outside_the_definition_can_be_voted_on() {
        let categories = Categories {
            topic_ids: HashMap::from([(4, Some(3))]),
        };
        assert!(categories.can_vote(35, "regular", Some(4)));
        assert!(!categories.can_vote(3, "regular", Some(4)));
        assert!(!categories.can_vote(35, "private_message", Some(4)));
        assert!(!categories.can_vote(35, "regular", Some(2)));
        assert!(!categories.can_vote(35, "regular", None));
    }
}
