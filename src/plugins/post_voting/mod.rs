//! discourse-post-voting, on the tables Rails' plugin created
//! (`post_voting_votes`, `post_voting_comments`): topics of the
//! `question_answer` subtype, whose answers the topic view orders by
//! votes, with up and down votes on posts (`votes`), comments on posts
//! and upvotes on those (`comments`), and the keys the plugin adds to
//! posts, the topic view, topic list items, categories, user cards and
//! the current user.

pub mod comments;
pub mod lifecycle;
pub mod view;
pub mod votes;

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::plugins::PluginError;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list::time_json;

/// `Topic::POST_VOTING_SUBTYPE`
pub const SUBTYPE: &str = "question_answer";

/// `TopicView::PRELOAD_COMMENTS_COUNT`: the comments a post shows before
/// "Show more".
pub const PRELOAD_COMMENTS_COUNT: i64 = 5;

/// `TopicView::ACTIVITY_FILTER`: the topic in post order instead of by
/// votes.
pub const ACTIVITY_FILTER: &str = "activity";

/// `PostVoting::CREATE_AS_POST_VOTING_DEFAULT`
pub const CREATE_AS_POST_VOTING_DEFAULT: &str = "create_as_post_voting_default";
/// `PostVoting::ONLY_POST_VOTING_IN_THIS_CATEGORY`
pub const ONLY_POST_VOTING_IN_THIS_CATEGORY: &str = "only_post_voting_in_this_category";

/// `Notification.types[:question_answer_user_commented]`
pub const USER_COMMENTED: i32 = 35;

/// `enabled_site_setting :post_voting_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("post_voting_enabled")?.truthy())
}

/// `Topic#nested_view?`: nested replies show the topic as a tree.
async fn nested_view(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    archetype: &str,
) -> Result<bool, PluginError> {
    if !settings.get("nested_replies_enabled")?.truthy() || archetype == "private_message" {
        return Ok(false);
    }
    if settings.get("nested_replies_default")?.truthy() {
        return Ok(true);
    }
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM nested_topics WHERE topic_id = $1)")
            .bind(i64::from(topic_id))
            .fetch_one(conn)
            .await?,
    )
}

/// `Topic#is_post_voting?` from the topic's subtype and archetype.
pub async fn is_post_voting(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    archetype: &str,
    subtype: Option<&str>,
) -> Result<bool, PluginError> {
    if !enabled(settings)? || subtype != Some(SUBTYPE) {
        return Ok(false);
    }
    Ok(!nested_view(conn, settings, topic_id, archetype).await?)
}

/// `Post#is_post_voting_topic?`: the post's topic is a post voting one
/// (refused for a trashed topic, as `refuse_trashed_topic`).
pub async fn post_in_post_voting_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i32,
) -> Result<bool, PluginError> {
    if !enabled(settings)? {
        return Ok(false);
    }
    let topic: Option<(i32, String, Option<String>, bool)> = sqlx::query_as(
        "SELECT t.id, t.archetype, t.subtype, t.deleted_at IS NOT NULL FROM posts p \
         JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    match topic {
        Some((id, archetype, subtype, trashed)) => {
            refuse_trashed_topic(settings, trashed)?;
            is_post_voting(conn, settings, id, &archetype, subtype.as_deref()).await
        }
        None => Ok(false),
    }
}

/// PostSerializerExtension's include checks call `topic.is_post_voting?` on
/// the post's topic, which is nil once the topic is trashed (`belongs_to`
/// with Topic's default scope): at the vendored ref that raises
/// NoMethodError for every post of a trashed topic (staff's view of it).
/// Refused here while the plugin is on; Rails raises off too, which the
/// port does not follow.
pub fn refuse_trashed_topic(settings: &SiteSettings, trashed: bool) -> Result<(), PluginError> {
    if trashed && enabled(settings)? {
        return Err(PluginError::Unsupported(crate::Unsupported(
            "the posts of a trashed topic with post voting on (Rails raises on topic.is_post_voting?)",
        )));
    }
    Ok(())
}

/// The scope `TopicView.apply_custom_default_scope` gives a post voting
/// topic's view (unless it loads replies to a post or posts by id): the
/// answers alone, and without the activity filter no whispers or small
/// actions, ordered by votes after the question.
pub fn topic_view_scope(activity: bool) -> crate::plugins::PostScope {
    if activity {
        return crate::plugins::PostScope {
            condition: "posts.reply_to_post_number IS NULL",
            order: None,
        };
    }
    crate::plugins::PostScope {
        condition: "posts.reply_to_post_number IS NULL AND posts.post_type NOT IN (3, 4)",
        order: Some(crate::plugins::PostOrder {
            asc: "CASE posts.post_number WHEN 1 THEN 0 ELSE 1 END, posts.qa_vote_count DESC, \
                  posts.post_number ASC",
            // ActiveRecord's reverse_order of the same string.
            desc: "CASE posts.post_number WHEN 1 THEN 0 ELSE 1 END DESC, posts.qa_vote_count ASC, \
                   posts.post_number DESC",
        }),
    }
}

/// A comment as the serializers read it, with its author.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Comment {
    pub id: i64,
    pub post_id: i32,
    pub user_id: i32,
    pub raw: String,
    pub cooked: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub qa_vote_count: Option<i32>,
    pub deleted_at: Option<NaiveDateTime>,
    pub name: Option<String>,
    pub username: Option<String>,
    /// `comment.user` exists.
    pub user_found: bool,
    pub user_staff: bool,
}

/// The columns of [`Comment`], over `post_voting_comments c`.
pub const COMMENT_SELECT: &str = "SELECT c.id, c.post_id, c.user_id, c.raw, c.cooked, c.created_at, \
    c.updated_at, c.qa_vote_count, c.deleted_at, u.name, u.username, u.id IS NOT NULL AS user_found, \
    COALESCE(u.admin OR u.moderator, FALSE) AS user_staff \
    FROM post_voting_comments c LEFT JOIN users u ON u.id = c.user_id";

/// What `can_flag_post_voting_comment?` reads besides the comment, once
/// per response.
#[derive(Debug, Clone)]
pub struct Flagging {
    me: Option<i32>,
    /// `can_flag_post_voting_comments?`
    can_flag: bool,
    allow_flagging_staff: bool,
    /// `PostActionType.flag_types`' names.
    flag_types: Vec<String>,
    enable_names: bool,
}

impl Flagging {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
    ) -> Result<Self, PluginError> {
        let types = crate::post_actions::ActionTypes::load(conn).await?;
        Ok(Flagging {
            me: guardian.user_id(),
            can_flag: guardian.is_authenticated() && can_flag_comments(settings, guardian)?,
            allow_flagging_staff: settings.get("allow_flagging_staff")?.truthy(),
            flag_types: types
                .types
                .into_iter()
                .filter(|(name, _)| name != "like")
                .map(|(name, _)| name)
                .collect(),
            enable_names: settings.get("enable_names")?.truthy(),
        })
    }

    /// `can_flag_post_voting_comment?` for a comment on a post the viewer
    /// can see.
    fn can_flag(&self, c: &Comment) -> bool {
        let Some(me) = self.me else {
            return false;
        };
        if c.deleted_at.is_some() || !c.user_found {
            return false;
        }
        if c.user_staff && !self.allow_flagging_staff {
            return false;
        }
        c.user_id != me && self.can_flag
    }

    /// PostVotingCommentSerializer, with `user_voted` as given
    /// (`comments_user_voted[id]` in a topic view, else whether the
    /// viewer voted on it). `reviewable_id` is always nil: no caller
    /// passes `reviewable_ids`.
    pub fn comment_json(&self, c: &Comment, user_voted: Value) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(c.id));
        out.insert("user_id".into(), json!(c.user_id));
        if self.enable_names {
            out.insert("name".into(), json!(c.name));
        }
        out.insert("username".into(), json!(c.username));
        out.insert("created_at".into(), json!(time_json(c.created_at)));
        out.insert("raw".into(), json!(c.raw));
        out.insert("cooked".into(), json!(c.cooked));
        out.insert("post_voting_vote_count".into(), json!(c.qa_vote_count));
        out.insert("user_voted".into(), user_voted);
        let flags: Vec<&String> = if self.can_flag(c) {
            self.flag_types.iter().collect()
        } else {
            Vec::new()
        };
        out.insert("available_flags".into(), json!(flags));
        out.insert("reviewable_id".into(), Value::Null);
        Value::Object(out)
    }
}

/// `can_flag_post_voting_comments?` for a signed-in user.
pub fn can_flag_comments(
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, SettingError> {
    if guardian.is_silenced() {
        return Ok(false);
    }
    if guardian.is_staff() {
        return Ok(true);
    }
    guardian.in_setting_groups(settings, "flag_posts_voting_comments_allowed_groups")
}

/// The post serializer's view of a post voting topic: what
/// `TopicView.on_preload` loads for the page in a topic view, nothing
/// outside one.
#[derive(Debug, Default)]
pub struct Posts {
    in_topic_view: bool,
    flagging: Option<Flagging>,
    comments: HashMap<i32, Vec<Comment>>,
    comments_counts: HashMap<i32, i64>,
    posts_user_voted: HashMap<i32, String>,
    comments_user_voted: HashSet<i64>,
    posts_voted_on: HashSet<i32>,
}

impl Posts {
    /// A post serialized without a topic view (`@topic_view` nil).
    pub fn outside_topic_view() -> Self {
        Posts::default()
    }

    /// `TopicView.on_preload` for the page's posts.
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        post_ids: &[i32],
    ) -> Result<Self, PluginError> {
        let mut out = Posts {
            in_topic_view: true,
            ..Default::default()
        };
        if post_ids.is_empty() {
            return Ok(out);
        }
        out.flagging = Some(Flagging::load(conn, settings, guardian).await?);
        // Each post's first comments by id.
        let sql = format!(
            "{COMMENT_SELECT} WHERE c.id IN (SELECT id FROM (SELECT id, ROW_NUMBER() OVER \
               (PARTITION BY post_id ORDER BY id) AS n FROM post_voting_comments \
               WHERE post_id = ANY($1) AND deleted_at IS NULL) x WHERE n <= $2) \
             ORDER BY c.id"
        );
        let comments: Vec<Comment> = sqlx::query_as(&sql)
            .bind(post_ids)
            .bind(PRELOAD_COMMENTS_COUNT)
            .fetch_all(&mut *conn)
            .await?;
        for c in comments {
            out.comments.entry(c.post_id).or_default().push(c);
        }
        let counts: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT post_id, count(*) FROM post_voting_comments \
             WHERE post_id = ANY($1) AND deleted_at IS NULL GROUP BY post_id",
        )
        .bind(post_ids)
        .fetch_all(&mut *conn)
        .await?;
        out.comments_counts = counts.into_iter().collect();
        if let Some(uid) = guardian.user_id() {
            let voted: Vec<(i64, String)> = sqlx::query_as(
                "SELECT votable_id, direction FROM post_voting_votes \
                 WHERE user_id = $1 AND votable_type = 'Post' AND votable_id = ANY($2) ORDER BY id",
            )
            .bind(uid)
            .bind(post_ids.iter().map(|&id| i64::from(id)).collect::<Vec<_>>())
            .fetch_all(&mut *conn)
            .await?;
            out.posts_user_voted = voted
                .into_iter()
                .map(|(id, direction)| (id as i32, direction))
                .collect();
            let comments: Vec<i64> = sqlx::query_scalar(
                "SELECT v.votable_id FROM post_voting_votes v \
                 JOIN post_voting_comments comments ON comments.id = v.votable_id \
                 WHERE v.user_id = $1 AND v.votable_type = 'PostVotingComment' \
                   AND comments.post_id = ANY($2)",
            )
            .bind(uid)
            .bind(post_ids)
            .fetch_all(&mut *conn)
            .await?;
            out.comments_user_voted = comments.into_iter().collect();
        }
        let voted_on: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT votable_id FROM post_voting_votes \
             WHERE votable_type = 'Post' AND votable_id = ANY($1)",
        )
        .bind(post_ids.iter().map(|&id| i64::from(id)).collect::<Vec<_>>())
        .fetch_all(&mut *conn)
        .await?;
        out.posts_voted_on = voted_on.into_iter().map(|id| id as i32).collect();
        Ok(out)
    }

    /// PostSerializerExtension's keys for a post of a post voting topic:
    /// `post_voting_vote_count`, `post_voting_user_voted_direction` (in a
    /// topic view where the viewer voted on some post of the page),
    /// `post_voting_has_votes`, `comments` and `comments_count`.
    pub fn post_keys(
        &self,
        post_id: i32,
        qa_vote_count: Option<i32>,
        out: &mut Map<String, Value>,
    ) {
        out.insert("post_voting_vote_count".into(), json!(qa_vote_count));
        if self.in_topic_view && !self.posts_user_voted.is_empty() {
            out.insert(
                "post_voting_user_voted_direction".into(),
                json!(self.posts_user_voted.get(&post_id)),
            );
        }
        out.insert(
            "post_voting_has_votes".into(),
            json!(self.posts_voted_on.contains(&post_id)),
        );
        let comments: Vec<Value> = match &self.flagging {
            Some(flagging) => self
                .comments
                .get(&post_id)
                .into_iter()
                .flatten()
                .map(|c| {
                    let voted = if self.comments_user_voted.contains(&c.id) {
                        Value::Bool(true)
                    } else {
                        Value::Null
                    };
                    flagging.comment_json(c, voted)
                })
                .collect(),
            None => Vec::new(),
        };
        out.insert("comments".into(), Value::Array(comments));
        out.insert(
            "comments_count".into(),
            json!(self.comments_counts.get(&post_id).copied().unwrap_or(0)),
        );
    }

    /// The page's comments on a post, for the HTML page.
    pub fn comments(&self, post_id: i32) -> &[Comment] {
        self.comments.get(&post_id).map_or(&[], Vec::as_slice)
    }

    /// The post's comment count.
    pub fn comments_count(&self, post_id: i32) -> i64 {
        self.comments_counts.get(&post_id).copied().unwrap_or(0)
    }

    /// The viewer's vote on a post, "up" or "down".
    pub fn user_voted(&self, post_id: i32) -> Option<&str> {
        self.posts_user_voted.get(&post_id).map(String::as_str)
    }

    /// The viewer upvoted the comment.
    pub fn comment_user_voted(&self, comment_id: i64) -> bool {
        self.comments_user_voted.contains(&comment_id)
    }

    /// The post has votes.
    pub fn has_votes(&self, post_id: i32) -> bool {
        self.posts_voted_on.contains(&post_id)
    }

    /// `can_flag_post_voting_comment?` for a comment of the page.
    pub fn can_flag(&self, c: &Comment) -> bool {
        self.flagging.as_ref().is_some_and(|f| f.can_flag(c))
    }
}

/// TopicViewSerializerExtension's facts: the topic's answers (posts not
/// replying to another, the first one aside) and comments.
#[derive(Debug, Default)]
pub struct TopicFacts {
    pub answer_count: i64,
    pub last_answered_at: Option<NaiveDateTime>,
    pub last_answer_post_number: Option<i32>,
    /// The last answer's author: `Some(None)` when it has none, which
    /// `User.find` raises on.
    pub last_answerer_id: Option<Option<i32>>,
    pub last_commented_on: Option<NaiveDateTime>,
}

impl TopicFacts {
    pub async fn load(conn: &mut PgConnection, topic_id: i32) -> Result<Self, sqlx::Error> {
        let answer_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM posts WHERE topic_id = $1 AND reply_to_post_number IS NULL \
             AND post_number <> 1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let last: Option<(NaiveDateTime, i32, Option<i32>)> = sqlx::query_as(
            "SELECT created_at, post_number, user_id FROM posts WHERE topic_id = $1 \
             AND reply_to_post_number IS NULL AND post_number <> 1 AND deleted_at IS NULL \
             ORDER BY post_number DESC LIMIT 1",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        let last_commented_on: Option<NaiveDateTime> = sqlx::query_scalar(
            "SELECT c.created_at FROM post_voting_comments c JOIN posts ON posts.id = c.post_id \
             WHERE posts.topic_id = $1 AND posts.deleted_at IS NULL AND c.deleted_at IS NULL \
             ORDER BY c.created_at DESC LIMIT 1",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        Ok(TopicFacts {
            answer_count,
            last_answered_at: last.map(|(at, _, _)| at),
            last_answer_post_number: last.map(|(_, n, _)| n),
            last_answerer_id: last.map(|(_, _, user)| user),
            last_commented_on,
        })
    }

    /// The topic view's keys but `last_answerer`, which the caller adds
    /// as a BasicUser.
    pub fn topic_view_keys(&self, out: &mut Map<String, Value>) {
        out.insert("is_post_voting".into(), json!(true));
        out.insert(
            "last_answered_at".into(),
            json!(self.last_answered_at.map(time_json)),
        );
        out.insert(
            "last_commented_on".into(),
            json!(self.last_commented_on.map(time_json)),
        );
        out.insert("answer_count".into(), json!(self.answer_count));
        out.insert(
            "last_answer_post_number".into(),
            json!(self.last_answer_post_number),
        );
    }
}

/// `User#vote_count` (the user card's key): the votes on the user's posts.
pub async fn user_vote_count(conn: &mut PgConnection, user_id: i32) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(qa_vote_count), 0)::bigint FROM posts \
         WHERE user_id = $1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// BasicCategorySerializer's `create_as_post_voting_default` and
/// `only_post_voting_in_this_category`: each custom field cast to a
/// boolean, nil when unset.
pub async fn category_keys(
    conn: &mut PgConnection,
    category_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), sqlx::Error> {
    let fields = category_fields(conn, &[category_id]).await?;
    category_keys_from(&fields, category_id, out);
    Ok(())
}

/// The two category fields of many categories: (category, name) to the
/// first row's value.
pub async fn category_fields(
    conn: &mut PgConnection,
    category_ids: &[i32],
) -> Result<HashMap<(i32, String), Option<String>>, sqlx::Error> {
    let rows: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT category_id, name, value FROM category_custom_fields \
         WHERE category_id = ANY($1) AND name IN ($2, $3) ORDER BY id DESC",
    )
    .bind(category_ids)
    .bind(CREATE_AS_POST_VOTING_DEFAULT)
    .bind(ONLY_POST_VOTING_IN_THIS_CATEGORY)
    .fetch_all(conn)
    .await?;
    // Descending ids: the first row of a name wins.
    Ok(rows
        .into_iter()
        .map(|(id, name, value)| ((id, name), value))
        .collect())
}

/// [`category_keys`] from loaded fields.
pub fn category_keys_from(
    fields: &HashMap<(i32, String), Option<String>>,
    category_id: i32,
    out: &mut Map<String, Value>,
) {
    for name in [
        CREATE_AS_POST_VOTING_DEFAULT,
        ONLY_POST_VOTING_IN_THIS_CATEGORY,
    ] {
        let value = fields
            .get(&(category_id, name.to_string()))
            .map(|v| crate::ruby::cast_boolean(v.as_deref()));
        out.insert(name.into(), json!(value.flatten()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_activity_filter_keeps_post_order() {
        let votes = topic_view_scope(false);
        assert!(votes.order.is_some());
        assert!(votes.condition.contains("post_type NOT IN (3, 4)"));
        let activity = topic_view_scope(true);
        assert!(activity.order.is_none());
        assert_eq!(activity.condition, "posts.reply_to_post_number IS NULL");
    }
}
