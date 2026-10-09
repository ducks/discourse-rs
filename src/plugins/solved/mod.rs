//! discourse-solved: what it adds to the topic list, the topic view,
//! posts and users, on the tables Rails' plugin created
//! (`discourse_solved_solved_topics`, `discourse_solved_topic_answers`,
//! `discourse_solved_shared_issues`) and the category custom fields it
//! reads; accepting and unaccepting answers are in `answers`.

pub mod answers;
pub mod by_user;

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{PluginError, PosterInputs};
use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

/// `enabled_site_setting :solved_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("solved_enabled")?.truthy())
}

/// The topic list's after-load data: the topics with an accepted answer,
/// and each one's answerers.
#[derive(Debug, Default)]
pub struct TopicListData {
    /// `topic.topic_answers.any?`
    pub answered: HashSet<i32>,
    /// `accepted_answer_user_ids` (register_topic_list_preload_user_ids):
    /// the users of the topic's answer posts, through Rails' join on posts,
    /// which keeps deleted ones.
    pub answerers: HashMap<i32, Vec<i32>>,
}

impl TopicListData {
    pub async fn load(conn: &mut PgConnection, topic_ids: &[i32]) -> Result<Self, sqlx::Error> {
        let rows: Vec<(i32, Option<i32>)> = sqlx::query_as(
            "SELECT st.topic_id::int, p.user_id \
             FROM discourse_solved_solved_topics st \
             JOIN discourse_solved_topic_answers ta ON ta.solved_topic_id = st.id \
             LEFT JOIN posts p ON p.id = ta.answer_post_id \
             WHERE st.topic_id = ANY($1) ORDER BY ta.id",
        )
        .bind(topic_ids)
        .fetch_all(&mut *conn)
        .await?;
        let mut data = TopicListData::default();
        for (topic_id, user_id) in rows {
            data.answered.insert(topic_id);
            if let Some(user_id) = user_id {
                let users = data.answerers.entry(topic_id).or_default();
                if !users.contains(&user_id) {
                    users.push(user_id);
                }
            }
        }
        Ok(data)
    }

    /// TopicPostersSummaryExtension: the answerers right after the author
    /// (`user_ids`), "Accepted Answer" after their descriptions
    /// (`descriptions_by_id`), and an answering last poster keeps their
    /// place (`last_poster_is_topic_creator?`).
    pub fn rewrite_posters(
        &self,
        i18n: &crate::i18n::I18n,
        topic_id: i32,
        last_post_user_id: i32,
        inputs: &mut PosterInputs,
    ) {
        let Some(answerers) = self.answerers.get(&topic_id).filter(|a| !a.is_empty()) else {
            return;
        };
        let at = inputs.user_ids.len().min(1);
        for (i, id) in answerers.iter().enumerate() {
            inputs.user_ids.insert(at + i, Some(*id));
        }
        let accepted = i18n.t("accepted_answer").unwrap_or("Accepted Answer");
        for id in answerers {
            inputs
                .descriptions
                .entry(*id)
                .or_default()
                .push(accepted.to_string());
        }
        if answerers.contains(&last_post_user_id) {
            inputs.last_poster_stays = true;
        }
    }
}

/// What the permission checks read of a topic.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TopicFacts {
    pub id: i32,
    pub user_id: Option<i32>,
    pub category_id: Option<i32>,
    pub archetype: String,
    pub closed: bool,
    pub archived: bool,
    pub deleted: bool,
}

impl TopicFacts {
    fn private_message(&self) -> bool {
        self.archetype == "private_message"
    }
}

/// `category.custom_fields[name] == "true"` (CategoryExtension).
pub(crate) async fn category_field(
    conn: &mut PgConnection,
    category_id: Option<i32>,
    name: &str,
) -> Result<Option<String>, sqlx::Error> {
    let Some(category_id) = category_id else {
        return Ok(None);
    };
    sqlx::query_scalar(
        "SELECT value FROM category_custom_fields WHERE category_id = $1 AND name = $2 \
         ORDER BY id LIMIT 1",
    )
    .bind(category_id)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await
}

/// `solved_enabled_for_category?`: every topic, a solved tag, or a
/// category with enable_accepted_answers (AcceptedAnswerCache.allowed).
async fn enabled_for_category(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic: &TopicFacts,
) -> Result<bool, PluginError> {
    if settings.get("allow_solved_on_all_topics")?.truthy() {
        return Ok(true);
    }
    let solved_tags = settings.get("enable_solved_tags")?.to_s();
    if !solved_tags.is_empty() {
        let tags: Vec<String> = sqlx::query_scalar(
            "SELECT t.name FROM topic_tags tt JOIN tags t ON t.id = tt.tag_id WHERE tt.topic_id = $1",
        )
        .bind(topic.id)
        .fetch_all(&mut *conn)
        .await?;
        if tags.iter().any(|t| solved_tags.split('|').any(|s| s == t)) {
            return Ok(true);
        }
    }
    Ok(
        category_field(conn, topic.category_id, "enable_accepted_answers")
            .await?
            .as_deref()
            == Some("true"),
    )
}

/// `allow_accepted_answers?(topic)`
pub async fn allow_accepted_answers(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic: &TopicFacts,
) -> Result<bool, PluginError> {
    if settings.get("allow_solved_on_all_topics")?.truthy() {
        return Ok(true);
    }
    if topic.private_message() {
        let groups = settings.group_ids("allow_solved_in_groups")?;
        if groups.is_empty() {
            return Ok(false);
        }
        let groups: Vec<i32> = groups.iter().map(|g| *g as i32).collect();
        return Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_allowed_groups WHERE topic_id = $1 AND group_id = ANY($2))",
        )
        .bind(topic.id)
        .bind(&groups)
        .fetch_one(&mut *conn)
        .await?);
    }
    enabled_for_category(conn, settings, topic).await
}

/// TopicAnswerMixin's keys for a topic in a list: `has_accepted_answer`,
/// and `can_have_answer` where the category shows an empty box.
pub async fn topic_list_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic: &TopicFacts,
    answered: bool,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if !enabled(settings)? {
        return Ok(());
    }
    out.insert("has_accepted_answer".into(), json!(answered));
    if category_field(conn, topic.category_id, "empty_box_on_unsolved")
        .await?
        .as_deref()
        == Some("true")
    {
        let can_have = if settings.get("allow_solved_on_all_topics")?.truthy() {
            true
        } else if !topic.private_message() && (topic.closed || topic.archived) {
            false
        } else {
            allow_accepted_answers(conn, settings, topic).await?
        };
        out.insert("can_have_answer".into(), json!(can_have));
    }
    Ok(())
}

/// The topics among `topic_ids` with an accepted answer.
pub async fn answered_topics(
    conn: &mut PgConnection,
    topic_ids: &[i32],
) -> Result<HashSet<i32>, sqlx::Error> {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT st.topic_id::int FROM discourse_solved_solved_topics st \
         JOIN discourse_solved_topic_answers ta ON ta.solved_topic_id = st.id \
         WHERE st.topic_id = ANY($1)",
    )
    .bind(topic_ids)
    .fetch_all(&mut *conn)
    .await?;
    Ok(ids.into_iter().collect())
}

/// One accepted answer: its post and who accepted it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Answer {
    pub answer_post_id: i64,
    pub accepter_user_id: i64,
}

/// A topic's solved state for the topic view and its posts, and the
/// viewer's answers to the permission checks that don't depend on the
/// post.
pub struct TopicView {
    pub topic: TopicFacts,
    /// `topic.solved.present?`
    pub solved: bool,
    /// `topic.topic_answers`
    pub answers: Vec<Answer>,
    allow: bool,
    /// Staff, an accept-all group member, or the category's group
    /// moderator: may accept any reply.
    accepts_any: bool,
    /// The author may accept on their open topic.
    author_accepts: bool,
    staff: bool,
    /// The solved row's topic timer (auto close).
    pub(crate) solved_id: Option<i64>,
    pub(crate) topic_timer_id: Option<i32>,
}

impl TopicView {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        topic: TopicFacts,
    ) -> Result<Self, PluginError> {
        let solved: Option<(i64, Option<i32>)> = sqlx::query_as(
            "SELECT id, topic_timer_id FROM discourse_solved_solved_topics WHERE topic_id = $1",
        )
        .bind(topic.id)
        .fetch_optional(&mut *conn)
        .await?;
        let solved_id = solved.map(|s| s.0);
        let topic_timer_id = solved.and_then(|s| s.1);
        let answers: Vec<Answer> =
            match solved_id {
                Some(id) => sqlx::query_as(
                    "SELECT answer_post_id, accepter_user_id FROM discourse_solved_topic_answers \
                 WHERE solved_topic_id = $1 ORDER BY id",
                )
                .bind(id)
                .fetch_all(&mut *conn)
                .await?,
                None => Vec::new(),
            };
        let allow = allow_accepted_answers(conn, settings, &topic).await?;
        let staff = guardian.is_staff();
        let mut accepts_any = staff
            || (guardian.is_authenticated()
                && guardian.in_setting_groups(settings, "accept_all_solutions_allowed_groups")?);
        if !accepts_any && !topic.private_message() {
            accepts_any =
                category_group_moderator(conn, settings, guardian, topic.category_id).await?;
        }
        let author_accepts = guardian.is_authenticated()
            && topic.user_id.is_some()
            && topic.user_id == guardian.user_id()
            && !topic.closed
            && settings.get("accept_solutions_topic_author")?.truthy();
        Ok(TopicView {
            topic,
            solved: solved_id.is_some(),
            answers,
            allow,
            accepts_any,
            author_accepts,
            staff,
            solved_id,
            topic_timer_id,
        })
    }

    /// `can_accept_answer?(topic, post)`, for a post the viewer sees.
    pub(crate) fn can_accept(&self, guardian: &Guardian, post_number: i32, whisper: bool) -> bool {
        guardian.is_authenticated()
            && post_number > 1
            && !whisper
            && self.allow
            && (self.accepts_any || self.author_accepts)
    }

    pub(crate) fn accepted(&self, post_id: i32) -> bool {
        self.answers
            .iter()
            .any(|a| a.answer_post_id == i64::from(post_id))
    }

    /// The post serializer's keys: `can_accept_answer`,
    /// `can_unaccept_answer` (`can_unaccept_answer?` and accepted),
    /// `accepted_answer`, `topic_accepted_answer` (true, or nil).
    pub fn post_keys(
        &self,
        guardian: &Guardian,
        post_id: i32,
        post_number: i32,
        whisper: bool,
        out: &mut Map<String, Value>,
    ) {
        let can_accept = self.can_accept(guardian, post_number, whisper);
        let accepted = self.accepted(post_id);
        let can_unaccept = can_accept || (self.staff && accepted);
        out.insert("can_accept_answer".into(), json!(can_accept));
        out.insert(
            "can_unaccept_answer".into(),
            json!(can_unaccept && accepted),
        );
        out.insert("accepted_answer".into(), json!(accepted));
        // `topic&.solved&.present?`: nil without a solved row.
        out.insert(
            "topic_accepted_answer".into(),
            if self.solved {
                json!(true)
            } else {
                Value::Null
            },
        );
    }

    /// The topic view's shared issue keys (`shared_issue_count` and
    /// `user_created_shared_issue` while they're visible, then
    /// `can_create_shared_issue` and `shared_issue_visible`).
    pub async fn shared_issue_keys(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        out: &mut Map<String, Value>,
    ) -> Result<(), PluginError> {
        let t = &self.topic;
        let category_topic: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
                .bind(t.id)
                .fetch_one(&mut *conn)
                .await?;
        // topic_in_support_category? and shared_issues_enabled_for_category?
        let in_support = t.category_id.is_some()
            && category_field(conn, t.category_id, "enable_accepted_answers")
                .await?
                .as_deref()
                == Some("true");
        let shared_enabled = match t.category_id {
            Some(_) => {
                category_field(conn, t.category_id, "enable_shared_issues")
                    .await?
                    .as_deref()
                    != Some("false")
            }
            None => false,
        };
        let upcoming = guardian
            .upcoming_change_enabled(&mut *conn, settings, "enable_solved_shared_issues")
            .await?;
        let base = !t.private_message() && !category_topic && in_support && shared_enabled;
        let visible = base && !t.deleted && upcoming;
        let can_create = base
            && guardian.is_authenticated()
            && t.user_id != guardian.user_id()
            && !t.deleted
            && !t.closed
            && !t.archived
            && !(self.solved && !settings.get("solved_allow_multiple_solutions")?.truthy())
            && upcoming;
        if visible {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM discourse_solved_shared_issues WHERE topic_id = $1",
            )
            .bind(t.id)
            .fetch_one(&mut *conn)
            .await?;
            out.insert("shared_issue_count".into(), json!(count));
            if let Some(user_id) = guardian.user_id() {
                let created: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM discourse_solved_shared_issues \
                     WHERE topic_id = $1 AND user_id = $2)",
                )
                .bind(t.id)
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await?;
                out.insert("user_created_shared_issue".into(), json!(created));
            }
        }
        out.insert("can_create_shared_issue".into(), json!(can_create));
        out.insert("shared_issue_visible".into(), json!(visible));
        Ok(())
    }
}

/// `is_category_group_moderator?(category)`: with category group
/// moderation on, a member of one of the category's moderating groups.
async fn category_group_moderator(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    category_id: Option<i32>,
) -> Result<bool, PluginError> {
    let (Some(category_id), Some(user_id)) = (category_id, guardian.user_id()) else {
        return Ok(false);
    };
    if !settings.get("enable_category_group_moderation")?.truthy() {
        return Ok(false);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_moderation_groups cmg \
         JOIN group_users gu ON gu.group_id = cmg.group_id \
         WHERE cmg.category_id = $1 AND gu.user_id = $2)",
    )
    .bind(category_id)
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `DiscourseSolved::Queries.solved_count`: solved regular topics answered
/// by the user's live posts (the user card's `accepted_answers`, the
/// summary's `solved_count`).
pub async fn solved_count(conn: &mut PgConnection, user_id: i32) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM discourse_solved_solved_topics st \
         JOIN discourse_solved_topic_answers ta ON ta.solved_topic_id = st.id \
         JOIN posts p ON p.id = ta.answer_post_id \
         JOIN topics t ON t.id = st.topic_id \
         WHERE p.user_id = $1 AND p.deleted_at IS NULL \
         AND t.archetype = 'regular' AND t.deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> PosterInputs {
        PosterInputs {
            user_ids: vec![Some(2), Some(5), Some(3)],
            descriptions: HashMap::from([
                (2, vec!["Original Poster".to_string()]),
                (5, vec!["Most Recent Poster".to_string()]),
                (3, vec!["Frequent Poster".to_string()]),
            ]),
            last_poster_stays: false,
        }
    }

    #[test]
    fn answerers_join_the_posters_second() {
        let i18n = crate::i18n::I18n::vendored().unwrap();
        let data = TopicListData {
            answered: HashSet::from([35]),
            answerers: HashMap::from([(35, vec![4])]),
        };
        let mut posters = inputs();
        data.rewrite_posters(&i18n, 35, 5, &mut posters);
        assert_eq!(posters.user_ids, vec![Some(2), Some(4), Some(5), Some(3)]);
        assert_eq!(
            posters.descriptions[&4],
            vec!["Accepted Answer".to_string()]
        );
        assert!(!posters.last_poster_stays);

        // An answering last poster keeps their place.
        let mut posters = inputs();
        let data = TopicListData {
            answered: HashSet::from([35]),
            answerers: HashMap::from([(35, vec![5])]),
        };
        data.rewrite_posters(&i18n, 35, 5, &mut posters);
        assert!(posters.last_poster_stays);
        assert_eq!(
            posters.descriptions[&5],
            vec![
                "Most Recent Poster".to_string(),
                "Accepted Answer".to_string()
            ]
        );

        // Topics without answerers are left alone.
        let mut posters = inputs();
        data.rewrite_posters(&i18n, 36, 5, &mut posters);
        assert_eq!(posters, inputs());
    }
}
