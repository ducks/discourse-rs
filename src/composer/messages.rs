//! `ComposerMessagesFinder` (lib/composer_messages_finder.rb): the message
//! the composer shows above itself when it opens, from the member's posting
//! history and the topic replied to. The checks run in the order the class
//! defines them, then the ones the plugins add (`add_to_class(
//! :composer_messages_finder, :check_*)`, crate::plugins::composer_messages),
//! and the first with a message wins. An edit gets none.
//!
//! Also here: `TopicLink.duplicate_lookup`, the links the topic's posts
//! hold, which the endpoint hands the composer to warn about posting one
//! again (src/routes/composer_messages.rs).

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::pretty_text::{Host, MarkdownOptions};
use crate::session::current::SessionUser;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// The params the finder reads (`params.slice(:composer_action, :topic_id,
/// :post_id)`), as given.
#[derive(Debug, Clone, Default)]
pub struct Details {
    pub composer_action: Option<String>,
    pub topic_id: Option<String>,
    pub post_id: Option<String>,
}

impl Details {
    /// ActiveRecord's cast of an id param: digits, else no record.
    fn id(value: &Option<String>) -> Option<i32> {
        value.as_deref().and_then(|v| v.trim().parse().ok())
    }

    pub fn topic_id(&self) -> Option<i32> {
        Self::id(&self.topic_id)
    }

    pub fn post_id(&self) -> Option<i32> {
        Self::id(&self.post_id)
    }

    /// `@details[:post_id].blank?`
    fn post_id_blank(&self) -> bool {
        self.post_id.as_deref().is_none_or(|v| v.trim().is_empty())
    }
}

/// The topic, when the member can see it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FinderTopic {
    pub id: i32,
    pub user_id: Option<i32>,
    pub archetype: String,
    pub posts_count: i32,
    /// `topic.category&.read_restricted`
    pub read_restricted: Option<bool>,
}

impl FinderTopic {
    pub fn private_message(&self) -> bool {
        self.archetype == "private_message"
    }
}

pub struct Finder<'a> {
    pub state: &'a AppState,
    pub settings: &'a SiteSettings,
    /// `@user.guardian`
    pub guardian: &'a Guardian,
    pub details: Details,
    /// `@topic`: nil unless the member can see it.
    pub topic: Option<FinderTopic>,
}

impl<'a> Finder<'a> {
    /// `ComposerMessagesFinder.new(user, details)`
    pub async fn new(
        conn: &mut PgConnection,
        state: &'a AppState,
        settings: &'a SiteSettings,
        guardian: &'a Guardian,
        details: Details,
    ) -> Result<Finder<'a>, AppError> {
        let mut topic = None;
        if let Some(id) = details.topic_id() {
            topic = sqlx::query_as::<_, FinderTopic>(
                "SELECT t.id, t.user_id, t.archetype, t.posts_count, c.read_restricted \
                 FROM topics t LEFT JOIN categories c ON c.id = t.category_id \
                 WHERE t.id = $1 AND t.deleted_at IS NULL",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            if topic.is_some()
                && guardian
                    .can_see_topic_ids(&mut *conn, settings, &[id])
                    .await?
                    .is_empty()
            {
                topic = None;
            }
        }
        Ok(Finder {
            state,
            settings,
            guardian,
            details,
            topic,
        })
    }

    fn user(&self) -> Result<&SessionUser, AppError> {
        self.guardian
            .user()
            .ok_or_else(|| AppError::from(crate::Unsupported("composer messages for no user")))
    }

    /// `find`: the first check's message, none while editing.
    pub async fn find(&self, conn: &mut PgConnection) -> Result<Option<Value>, AppError> {
        if self.editing_post() {
            return Ok(None);
        }
        // check_methods, in their definition order.
        if let Some(m) = self.check_education_message(conn).await? {
            return Ok(Some(m));
        }
        if let Some(m) = self.check_new_user_many_replies(conn).await? {
            return Ok(Some(m));
        }
        if let Some(m) = self.check_dominating_topic(conn).await? {
            return Ok(Some(m));
        }
        if let Some(m) = self.check_get_a_room(conn, 5).await? {
            return Ok(Some(m));
        }
        if let Some(m) = self.check_dont_feed_the_trolls(conn).await? {
            return Ok(Some(m));
        }
        crate::plugins::composer_messages(conn, self).await
    }

    /// `PrettyText.cook(text)`
    pub async fn cook(&self, text: &str) -> Result<String, AppError> {
        let host = Host::from_state(self.state);
        Ok(crate::pretty_text::cook(&host, text, &MarkdownOptions::default()).await?)
    }

    /// `I18n.t(key, args)`, the key itself when missing.
    pub fn t(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.state
            .i18n
            .t_with(key, args)
            .unwrap_or_else(|| key.to_string())
    }

    /// `I18n.t(key, count:, args)`
    fn t_count(&self, key: &str, count: i64, args: &[(&str, &str)]) -> String {
        self.state
            .i18n
            .t_count(key, count, args)
            .unwrap_or_else(|| key.to_string())
    }

    fn base_path(&self) -> &str {
        self.state.config.globals.relative_url_root()
    }

    /// `@user.topic_count + @user.post_count`'s parts, from user_stats.
    async fn stats(&self, conn: &mut PgConnection) -> Result<(i64, i64), AppError> {
        let row: Option<(i32, i32)> =
            sqlx::query_as("SELECT topic_count, post_count FROM user_stats WHERE user_id = $1")
                .bind(self.user()?.id)
                .fetch_optional(&mut *conn)
                .await?;
        let (topics, posts) = row.unwrap_or_default();
        Ok((i64::from(topics), i64::from(posts)))
    }

    /// Determines whether to show the user education text
    pub async fn check_education_message(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Option<Value>, AppError> {
        if self.topic.as_ref().is_some_and(|t| t.private_message()) {
            return Ok(None);
        }
        let education_key = if self.creating_topic() {
            "education.new-topic"
        } else {
            "education.new-reply"
        };
        let (topics, posts) = self.stats(conn).await?;
        let until = self.settings.get("educate_until_posts")?.to_i();
        if topics + posts >= until {
            return Ok(None);
        }
        let posts_text = self.t_count("education.until_posts", until, &[]);
        let title = self.settings.get("title")?.to_s();
        let text = self.t(
            education_key,
            &[
                ("education_posts_text", &posts_text),
                ("site_name", &title),
                ("base_path", self.base_path()),
            ],
        );
        Ok(Some(json!({
            "id": "education",
            "templateName": "education",
            "wait_for_typing": true,
            "body": self.cook(&text).await?,
        })))
    }

    /// New users have a limited number of replies in a topic
    pub async fn check_new_user_many_replies(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Option<Value>, AppError> {
        if !self.replying()
            || !crate::posting::validate::posted_too_much_in_topic(
                conn,
                self.settings,
                self.guardian,
                self.topic.as_ref().map(|t| t.id),
            )
            .await?
        {
            return Ok(None);
        }
        let max = self
            .settings
            .get("newuser_max_replies_per_topic")?
            .to_i()
            .to_string();
        let text = self.t(
            "education.too_many_replies",
            &[("newuser_max_replies_per_topic", &max)],
        );
        Ok(Some(json!({
            "id": "too_many_replies",
            "templateName": "education",
            "body": self.cook(&text).await?,
        })))
    }

    pub async fn check_dominating_topic(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Option<Value>, AppError> {
        if !self
            .educate_reply(conn, "notified_about_dominating_topic")
            .await?
        {
            return Ok(None);
        }
        let user_id = self.user()?.id;
        let Some(topic) = self.topic.as_ref() else {
            return Ok(None);
        };
        if topic.user_id == Some(user_id)
            || i64::from(topic.posts_count) < self.settings.get("summary_posts_required")?.to_i()
            || topic.private_message()
        {
            return Ok(None);
        }
        let posts_by_user: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM posts WHERE user_id = $1 AND topic_id = $2 AND deleted_at IS NULL",
        )
        .bind(user_id)
        .bind(topic.id)
        .fetch_one(&mut *conn)
        .await?;
        let ratio = posts_by_user as f64 / f64::from(topic.posts_count);
        if ratio
            < self
                .settings
                .get("dominating_topic_minimum_percent")?
                .to_f()
                / 100.0
        {
            return Ok(None);
        }
        // Log the topic notification
        self.log(conn, "notified_about_dominating_topic").await?;
        let text = self.t("education.dominating_topic", &[]);
        Ok(Some(json!({
            "id": "dominating_topic",
            "templateName": "dominating-topic",
            "wait_for_typing": true,
            "extraClass": "education-message dominating-topic-message",
            "body": self.cook(&text).await?,
        })))
    }

    /// `check_get_a_room(min_users_posted: 5)`
    pub async fn check_get_a_room(
        &self,
        conn: &mut PgConnection,
        min_users_posted: i64,
    ) -> Result<Option<Value>, AppError> {
        if !self.guardian.can_send_private_messages(self.settings)? {
            return Ok(None);
        }
        if !self
            .educate_reply(conn, "notified_about_get_a_room")
            .await?
        {
            return Ok(None);
        }
        if self.details.post_id_blank() {
            return Ok(None);
        }
        let Some(topic) = self.topic.as_ref() else {
            return Ok(None);
        };
        if topic.read_restricted == Some(true) {
            return Ok(None);
        }
        let user_id = self.user()?.id;
        let reply_to_user_id: Option<i32> = sqlx::query_scalar::<_, Option<i32>>(
            "SELECT user_id FROM posts WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(self.details.post_id())
        .fetch_optional(&mut *conn)
        .await?
        .flatten();
        let threshold = self.settings.get("get_a_room_threshold")?.to_i();
        // Users's last x posts in the topic
        let last: Vec<Option<i32>> = sqlx::query_scalar(
            "SELECT reply_to_user_id FROM posts \
             WHERE topic_id = $1 AND user_id = $2 AND deleted_at IS NULL \
             ORDER BY created_at DESC LIMIT $3",
        )
        .bind(topic.id)
        .bind(user_id)
        .bind(threshold)
        .fetch_all(&mut *conn)
        .await?;
        let last_x_replies: Vec<Option<i32>> = last
            .into_iter()
            .filter(|uid| *uid != Some(user_id) && *uid == reply_to_user_id)
            .collect();
        if last_x_replies.len() as i64 != threshold {
            return Ok(None);
        }
        let posters: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT user_id) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL",
        )
        .bind(topic.id)
        .fetch_one(&mut *conn)
        .await?;
        if posters < min_users_posted {
            return Ok(None);
        }
        self.log(conn, "notified_about_get_a_room").await?;
        let reply_username: Option<String> =
            sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
                .bind(last_x_replies.first().copied().flatten())
                .fetch_optional(&mut *conn)
                .await?;
        let text = self.t_count(
            "education.get_a_room",
            threshold,
            &[
                ("reply_username", reply_username.as_deref().unwrap_or("")),
                ("base_path", self.base_path()),
            ],
        );
        Ok(Some(json!({
            "id": "get_a_room",
            "templateName": "get-a-room",
            "wait_for_typing": true,
            "reply_username": reply_username,
            "extraClass": "education-message get-a-room",
            "body": self.cook(&text).await?,
        })))
    }

    pub async fn check_dont_feed_the_trolls(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Option<Value>, AppError> {
        if !self.replying() {
            return Ok(None);
        }
        let post_id: Option<i32> = if self.details.post_id.is_some() {
            sqlx::query_scalar("SELECT id FROM posts WHERE id = $1 AND deleted_at IS NULL")
                .bind(self.details.post_id())
                .fetch_optional(&mut *conn)
                .await?
        } else if let Some(topic) = self.topic.as_ref() {
            // @topic&.first_post
            sqlx::query_scalar(
                "SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
            )
            .bind(topic.id)
            .fetch_optional(&mut *conn)
            .await?
        } else {
            None
        };
        let Some(post_id) = post_id else {
            return Ok(None);
        };
        if !can_see_post(conn, self.settings, self.guardian, post_id).await? {
            return Ok(None);
        }
        // post.flags.active.group(:user_id).count
        let flags: Vec<(Option<i32>, i64)> = sqlx::query_as(
            "SELECT user_id, COUNT(*) FROM post_actions \
             WHERE post_id = $1 AND deleted_at IS NULL \
               AND disagreed_at IS NULL AND deferred_at IS NULL AND agreed_at IS NULL \
               AND post_action_type_id IN \
                   (SELECT id FROM flags WHERE NOT score_type AND id <> 2 AND NOT require_message) \
             GROUP BY user_id",
        )
        .bind(post_id)
        .fetch_all(&mut *conn)
        .await?;
        let user_id = self.user()?.id;
        let flagged_by_replier = flags
            .iter()
            .any(|(uid, count)| *uid == Some(user_id) && *count > 0);
        let flagged_by_others = flags.iter().map(|(_, count)| count).sum::<i64>()
            >= self.settings.get("dont_feed_the_trolls_threshold")?.to_i();
        if !flagged_by_replier && !flagged_by_others {
            return Ok(None);
        }
        let text = self.t("education.dont_feed_the_trolls", &[]);
        Ok(Some(json!({
            "id": "dont_feed_the_trolls",
            "templateName": "education",
            "wait_for_typing": false,
            "extraClass": "urgent",
            "body": self.cook(&text).await?,
        })))
    }

    /// `UserHistory.create!(action:, target_user_id: @user.id, topic_id:
    /// @details[:topic_id])`
    async fn log(&self, conn: &mut PgConnection, action: &str) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_histories (action, target_user_id, topic_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, $4, $4)",
        )
        .bind(crate::staff_action_logs::action_id(action))
        .bind(self.user()?.id)
        .bind(self.details.topic_id())
        .bind(crate::clock::now_naive())
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// `educate_reply?(type)`
    async fn educate_reply(&self, conn: &mut PgConnection, action: &str) -> Result<bool, AppError> {
        let Some(topic) = self.topic.as_ref() else {
            return Ok(false);
        };
        if !self.replying() || self.details.topic_id.is_none() || topic.private_message() {
            return Ok(false);
        }
        let (_, posts) = self.stats(conn).await?;
        if posts < self.settings.get("educate_until_posts")?.to_i() {
            return Ok(false);
        }
        Ok(!exists_for_user(conn, self.user()?.id, action, self.details.topic_id()).await?)
    }

    pub fn creating_topic(&self) -> bool {
        self.details.composer_action.as_deref() == Some("createTopic")
    }

    pub fn replying(&self) -> bool {
        self.details.composer_action.as_deref() == Some("reply")
    }

    fn editing_post(&self) -> bool {
        self.details.composer_action.as_deref() == Some("edit")
    }
}

/// `UserHistory.exists_for_user?(user, action_type, topic_id:)`
pub async fn exists_for_user(
    conn: &mut PgConnection,
    user_id: i32,
    action: &str,
    topic_id: Option<i32>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_histories WHERE target_user_id = $1 AND action = $2 \
           AND ($3::int IS NULL OR topic_id = $3))",
    )
    .bind(user_id)
    .bind(crate::staff_action_logs::action_id(action))
    .bind(topic_id)
    .fetch_one(conn)
    .await
}

/// `guardian.can_see?(post)`: the post's topic, then the post.
async fn can_see_post(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i32,
) -> Result<bool, AppError> {
    // Post's default scope: a trashed post is no record.
    let Some(post) = crate::topics::guardian::PostCtx::load(&mut *conn, post_id)
        .await?
        .filter(|p| !p.trashed())
    else {
        return Ok(false);
    };
    let topic_id: Option<i32> = sqlx::query_scalar("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&mut *conn)
        .await?;
    let topic_can_see = match topic_id {
        Some(id) => !guardian
            .can_see_topic_ids(&mut *conn, settings, &[id])
            .await?
            .is_empty(),
        None => false,
    };
    Ok(guardian.can_see_post(settings, &post, topic_can_see)?)
}

/// `ComposerMessagesFinder.user_not_seen_in_a_while(usernames)`: those not
/// seen for pm_warn_user_last_seen_months_ago months, sorted.
pub async fn user_not_seen_in_a_while(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    usernames: &[String],
) -> Result<Vec<String>, AppError> {
    let mut names: Vec<String> = sqlx::query_scalar(
        "SELECT username FROM users WHERE username_lower = ANY($1) AND last_seen_at < $2",
    )
    .bind(usernames)
    .bind(months_ago(settings)?)
    .fetch_all(conn)
    .await?;
    names.sort();
    Ok(names)
}

/// `SiteSetting.pm_warn_user_last_seen_months_ago.months.ago`
pub fn months_ago(settings: &SiteSettings) -> Result<NaiveDateTime, AppError> {
    let months = settings.get("pm_warn_user_last_seen_months_ago")?.to_i();
    let now = crate::clock::now_naive();
    Ok(now
        .checked_sub_months(chrono::Months::new(months.max(0) as u32))
        .unwrap_or(now))
}

/// `TopicLink.duplicate_lookup(topic, guardian)`: the last 200 links of the
/// topic's visible posts, by their URL lowercased without its scheme and
/// trailing slash, leaving out internal links to what the member can't see.
pub async fn duplicate_lookup(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<Map<String, Value>, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        url: String,
        domain: String,
        internal: bool,
        link_topic_id: Option<i32>,
        link_post_id: Option<i32>,
        username_lower: String,
        created_at: NaiveDateTime,
        post_number: i32,
    }
    let mut rows: Vec<Row> = sqlx::query_as(
        "SELECT tl.url, tl.domain, tl.internal, tl.link_topic_id, tl.link_post_id, \
                u.username_lower, p.created_at, p.post_number \
         FROM topic_links tl JOIN posts p ON p.id = tl.post_id JOIN users u ON u.id = tl.user_id \
         WHERE tl.topic_id = $1 AND NOT tl.reflection \
           AND p.deleted_at IS NULL AND NOT p.hidden AND p.post_type = ANY($2) \
         ORDER BY tl.id DESC LIMIT 200",
    )
    .bind(topic_id)
    .bind(guardian.visible_post_types(settings)?)
    .fetch_all(&mut *conn)
    .await?;
    rows.reverse();
    let mut lookup = Map::new();
    for tl in rows {
        if tl.internal {
            if let Some(id) = tl.link_topic_id {
                let seen: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
                )
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
                if !seen
                    || guardian
                        .can_see_topic_ids(&mut *conn, settings, &[id])
                        .await?
                        .is_empty()
                {
                    continue;
                }
            }
            if let Some(id) = tl.link_post_id
                && !can_see_post(conn, settings, guardian, id).await?
            {
                continue;
            }
        }
        let lower = tl.url.to_lowercase();
        let normalized = lower
            .strip_prefix("https://")
            .or_else(|| lower.strip_prefix("http://"))
            .unwrap_or(&lower);
        let normalized = normalized.strip_suffix('/').unwrap_or(normalized);
        lookup.insert(
            normalized.to_string(),
            json!({
                "domain": tl.domain,
                "username": tl.username_lower,
                "posted_at": crate::topics::list::time_json(tl.created_at),
                "post_number": tl.post_number,
            }),
        );
    }
    Ok(lookup)
}
