//! Port of lib/topic_view.rb, TopicViewSerializer, TopicViewDetailsSerializer
//! and PostSerializer (with BasicPostSerializer) for anonymous users.
//!
//! Plugin-added keys (reactions, solved, voting, zendesk, ...) are not
//! emitted; parity/cases ignores them. Suggested topics are served in a
//! deterministic order because Discourse's RandomTopicSelector consumes a
//! Redis list, so their content can never match a recording.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::avatar::{self, AvatarError};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list::{self, Mode, TopicListError, TopicListSerializer, time_json};
use crate::topic_query::{TOPIC_COLUMNS, TopicRow};
use crate::url::{UrlError, Urls};

/// `TopicView::CHUNK_SIZE`
pub const CHUNK_SIZE: i64 = 20;

/// `Topic.visible_post_types(nil)`: regular, moderator_action, small_action.
const VISIBLE_POST_TYPES: [i32; 3] = [1, 2, 3];

/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE: i64 = 2;

#[derive(Debug)]
pub enum TopicViewError {
    Db(sqlx::Error),
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
    /// `Discourse::NotFound`: missing, deleted, or not visible to the viewer.
    NotFound,
}

impl std::fmt::Display for TopicViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopicViewError::Db(e) => write!(f, "loading topic: {e}"),
            TopicViewError::Setting(e) => e.fmt(f),
            TopicViewError::Url(e) => e.fmt(f),
            TopicViewError::Unsupported(e) => e.fmt(f),
            TopicViewError::NotFound => write!(f, "topic not found"),
        }
    }
}

impl std::error::Error for TopicViewError {}

impl From<sqlx::Error> for TopicViewError {
    fn from(e: sqlx::Error) -> Self {
        TopicViewError::Db(e)
    }
}

impl From<SettingError> for TopicViewError {
    fn from(e: SettingError) -> Self {
        TopicViewError::Setting(e)
    }
}

impl From<UrlError> for TopicViewError {
    fn from(e: UrlError) -> Self {
        TopicViewError::Url(e)
    }
}

impl From<Unsupported> for TopicViewError {
    fn from(e: Unsupported) -> Self {
        TopicViewError::Unsupported(e)
    }
}

impl From<TopicListError> for TopicViewError {
    fn from(e: TopicListError) -> Self {
        match e {
            TopicListError::Db(e) => TopicViewError::Db(e),
            TopicListError::Setting(e) => TopicViewError::Setting(e),
            TopicListError::Url(e) => TopicViewError::Url(e),
            TopicListError::Unsupported(e) => TopicViewError::Unsupported(e),
        }
    }
}

impl From<AvatarError> for TopicViewError {
    fn from(e: AvatarError) -> Self {
        match e {
            AvatarError::Setting(e) => TopicViewError::Setting(e),
            AvatarError::Url(e) => TopicViewError::Url(e),
            AvatarError::Unsupported(e) => TopicViewError::Unsupported(e),
        }
    }
}

/// The extra `topics` columns the view reads beyond TopicRow.
#[derive(sqlx::FromRow)]
struct TopicExtra {
    word_count: Option<i32>,
    deleted_at: Option<NaiveDateTime>,
    pinned_until: Option<NaiveDateTime>,
    slow_mode_seconds: i32,
    external_id: Option<String>,
    read_restricted: Option<bool>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PostRow {
    id: i32,
    user_id: Option<i32>,
    post_number: i32,
    cooked: String,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
    reply_to_post_number: Option<i32>,
    reply_count: i32,
    quote_count: i32,
    incoming_link_count: i32,
    reads: i32,
    score: Option<f64>,
    post_type: i32,
    hidden: bool,
    hidden_reason_id: Option<i32>,
    user_deleted: bool,
    reply_to_user_id: Option<i32>,
    edit_reason: Option<String>,
    wiki: bool,
    reply_quoted: bool,
    public_version: i32,
    action_code: Option<String>,
    like_count: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PostUser {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    primary_group_id: Option<i32>,
    flair_group_id: Option<i32>,
    admin: bool,
    moderator: bool,
    trust_level: i32,
    title: Option<String>,
    suspended_till: Option<NaiveDateTime>,
}

pub struct Options {
    /// `params[:page]`, 0 when absent.
    pub page: i64,
    pub post_number: Option<i64>,
}

pub struct TopicView<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
    pub options: Options,
}

/// What the controller needs from the view besides the JSON.
pub struct Rendered {
    pub json: Value,
    pub slug: String,
}

impl TopicView<'_> {
    /// `TopicView.new` + `TopicViewSerializer`: the full document, or
    /// NotFound. `Redirect` when `page` is past the end.
    pub async fn render(&mut self, topic_id: i32) -> Result<Rendered, TopicViewError> {
        let (topic, extra) = self.find_topic(topic_id).await?;
        // can_see_topic? for anonymous users: not deleted, not a PM,
        // category readable.
        if extra.deleted_at.is_some()
            || topic.archetype == "private_message"
            || extra.read_restricted == Some(true)
        {
            return Err(TopicViewError::NotFound);
        }
        let Some(slug) = topic.slug.clone() else {
            return Err(Unsupported("topics without a stored slug (Slug.for)").into());
        };

        let stream = self.filtered_post_stream(topic.id).await?;
        let highest_post_number = self.highest_post_number(topic.id).await?;
        let post_number = self.options.post_number.unwrap_or(1).max(1);
        let page = if self.options.page > 1 {
            self.options.page
        } else {
            self.calculate_page(topic.id, post_number).await?
        };
        let posts = match self.options.post_number {
            Some(_) => self.filter_posts_near(topic.id, post_number).await?,
            None => self.filter_posts_paged(topic.id, page).await?,
        };
        let next_page = match (posts.last(), highest_post_number) {
            (Some(last), Some(highest)) if highest > last.post_number => Some(page + 1),
            _ => None,
        };

        let mut out = Map::new();
        let post_ids: Vec<i32> = posts.iter().map(|p| p.id).collect();
        let serialized_posts = self.serialize_posts(&topic, &slug, &posts).await?;
        out.insert(
            "post_stream".into(),
            json!({"posts": serialized_posts, "stream": stream.iter().map(|(id, _)| *id).collect::<Vec<_>>()}),
        );
        out.insert(
            "timeline_lookup".into(),
            json!(timeline_lookup(&stream, 300)),
        );
        if next_page.is_none() {
            out.insert(
                "suggested_topics".into(),
                self.suggested_topics(&topic).await?,
            );
        }
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        if tagging {
            let (tags, descriptions) = self.list_serializer().tags(topic.id).await?;
            out.insert("tags".into(), tags);
            out.insert("tags_descriptions".into(), descriptions);
        }
        let fancy_title = crate::topic_query::fancy_title(&topic)?;
        out.insert("fancy_title".into(), json!(fancy_title));
        out.insert("id".into(), json!(topic.id));
        out.insert("title".into(), json!(topic.title));
        out.insert("posts_count".into(), json!(topic.posts_count));
        out.insert("created_at".into(), json!(time_json(topic.created_at)));
        out.insert("views".into(), json!(topic.views));
        out.insert("reply_count".into(), json!(topic.reply_count));
        out.insert("like_count".into(), json!(topic.like_count));
        out.insert(
            "last_posted_at".into(),
            json!(topic.last_posted_at.map(time_json)),
        );
        out.insert("visible".into(), json!(topic.visible));
        out.insert("closed".into(), json!(topic.closed));
        out.insert("archived".into(), json!(topic.archived));
        out.insert("has_summary".into(), json!(topic.has_summary));
        out.insert("archetype".into(), json!(topic.archetype));
        out.insert("slug".into(), json!(slug));
        out.insert("category_id".into(), json!(topic.category_id));
        out.insert("word_count".into(), json!(extra.word_count));
        out.insert("deleted_at".into(), Value::Null);
        out.insert("user_id".into(), json!(topic.user_id));
        if self.settings.get("topic_featured_link_enabled")?.truthy() {
            out.insert("featured_link".into(), json!(topic.featured_link));
            if topic
                .featured_link
                .as_deref()
                .is_some_and(|l| !l.is_empty())
            {
                return Err(Unsupported("featured_link_root_domain").into());
            }
        }
        out.insert("pinned_globally".into(), json!(topic.pinned_globally));
        out.insert("pinned_at".into(), json!(topic.pinned_at.map(time_json)));
        out.insert(
            "pinned_until".into(),
            json!(extra.pinned_until.map(time_json)),
        );
        out.insert(
            "image_url".into(),
            self.list_serializer().image_url(&topic).await?,
        );
        out.insert("slow_mode_seconds".into(), json!(extra.slow_mode_seconds));
        if let Some(external_id) = &extra.external_id {
            out.insert("external_id".into(), json!(external_id));
        }
        if let Some(reason) = topic.visibility_reason_id {
            out.insert("visibility_reason_id".into(), json!(reason));
        }
        out.insert("draft".into(), Value::Null);
        out.insert("draft_key".into(), json!(format!("topic_{}", topic.id)));
        out.insert("draft_sequence".into(), Value::Null);
        out.insert("unpinned".into(), Value::Null);
        out.insert("pinned".into(), json!(topic.pinned_at.is_some()));
        if let Some(highest) = highest_post_number {
            out.insert(
                "current_post_number".into(),
                json!(post_number.min(i64::from(highest))),
            );
        }
        out.insert("highest_post_number".into(), json!(highest_post_number));
        out.insert("deleted_by".into(), Value::Null);
        out.insert(
            "actions_summary".into(),
            self.actions_summary(posts.is_empty()).await?,
        );
        out.insert("chunk_size".into(), json!(CHUNK_SIZE));
        out.insert("bookmarked".into(), json!(false));
        out.insert("topic_timer".into(), self.topic_timer(topic.id).await?);
        if crate::emoji::has_emoji_code(&topic.title) {
            out.insert(
                "unicode_title".into(),
                json!(crate::emoji::gsub_emoji_to_unicode(&topic.title)),
            );
        }
        // MessageBus.last_id("/topic/:id"): no message bus yet.
        out.insert("message_bus_last_id".into(), json!(0));
        let (participants, participant_count) = self.participants(topic.id, &post_ids).await?;
        out.insert("participant_count".into(), json!(participant_count));
        out.insert("show_read_indicator".into(), json!(false));
        if topic.image_upload_id.is_some() {
            return Err(Unsupported("topic thumbnails").into());
        }
        out.insert("thumbnails".into(), Value::Null);
        out.insert(
            "slow_mode_enabled_until".into(),
            self.slow_mode_enabled_until(topic.id).await?,
        );
        out.insert("details".into(), self.details(&topic, participants).await?);
        out.insert("bookmarks".into(), json!([]));

        Ok(Rendered {
            json: Value::Object(out),
            slug,
        })
    }

    fn list_serializer(&mut self) -> TopicListSerializer<'_> {
        TopicListSerializer {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: self.guardian,
            urls: self.urls,
            more_topics_url: None,
            category_id: None,
            prefetched: Default::default(),
        }
    }

    /// `Topic.with_deleted.find_by(id:)` plus the category's restriction.
    async fn find_topic(&mut self, id: i32) -> Result<(TopicRow, TopicExtra), TopicViewError> {
        let sql = format!("SELECT {TOPIC_COLUMNS} FROM topics WHERE topics.id = $1");
        let topic: Option<TopicRow> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(topic) = topic else {
            return Err(TopicViewError::NotFound);
        };
        let extra: TopicExtra = sqlx::query_as(
            "SELECT t.word_count, t.deleted_at, t.pinned_until, t.slow_mode_seconds, t.external_id, \
                    c.read_restricted \
             FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
        )
        .bind(id)
        .fetch_one(&mut *self.conn)
        .await?;
        Ok((topic, extra))
    }

    /// `filtered_post_stream`: every visible post id with its age in days.
    async fn filtered_post_stream(
        &mut self,
        topic_id: i32,
    ) -> Result<Vec<(i32, i32)>, TopicViewError> {
        Ok(sqlx::query_as(
            "SELECT id, (EXTRACT(EPOCH FROM CURRENT_TIMESTAMP - posts.created_at) / 86400)::INT AS days_ago \
             FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
             ORDER BY sort_order",
        )
        .bind(topic_id)
        .bind(&VISIBLE_POST_TYPES[..])
        .fetch_all(&mut *self.conn)
        .await?)
    }

    async fn highest_post_number(&mut self, topic_id: i32) -> Result<Option<i32>, TopicViewError> {
        Ok(sqlx::query_scalar(
            "SELECT MAX(post_number) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2)",
        )
        .bind(topic_id)
        .bind(&VISIBLE_POST_TYPES[..])
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// `calculate_page`: the page holding post_number.
    async fn calculate_page(
        &mut self,
        topic_id: i32,
        post_number: i64,
    ) -> Result<i64, TopicViewError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
             AND post_number <= $3",
        )
        .bind(topic_id)
        .bind(&VISIBLE_POST_TYPES[..])
        .bind(post_number)
        .fetch_one(&mut *self.conn)
        .await?;
        Ok(((count - 1).max(0) / CHUNK_SIZE) + 1)
    }

    const POST_SQL: &'static str = "SELECT id, user_id, post_number, cooked, created_at, updated_at, \
        reply_to_post_number, reply_count, quote_count, incoming_link_count, reads, score, post_type, \
        hidden, hidden_reason_id, user_deleted, reply_to_user_id, edit_reason, wiki, reply_quoted, \
        public_version, action_code, like_count FROM posts";

    /// `filter_posts_paged`: chunk `page` of the visible posts.
    async fn filter_posts_paged(
        &mut self,
        topic_id: i32,
        page: i64,
    ) -> Result<Vec<PostRow>, TopicViewError> {
        let sql = format!(
            "{} WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
             ORDER BY sort_order OFFSET $3 LIMIT $4",
            Self::POST_SQL
        );
        Ok(sqlx::query_as(&sql)
            .bind(topic_id)
            .bind(&VISIBLE_POST_TYPES[..])
            .bind(CHUNK_SIZE * (page - 1).max(0))
            .bind(CHUNK_SIZE)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// `filter_posts_near`: up to 5 posts before the target's sort order,
    /// then forward to fill the chunk, then further back if still short.
    async fn filter_posts_near(
        &mut self,
        topic_id: i32,
        post_number: i64,
    ) -> Result<Vec<PostRow>, TopicViewError> {
        let before = (CHUNK_SIZE / 4).max(1);
        let sort_order: Option<i32> = sqlx::query_scalar(
            "SELECT sort_order FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
             ORDER BY abs(post_number - $3) LIMIT 1",
        )
        .bind(topic_id)
        .bind(&VISIBLE_POST_TYPES[..])
        .bind(post_number as i32)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(sort_order) = sort_order else {
            return Ok(Vec::new());
        };

        let ids_where = |cmp: &str, order: &str| {
            format!(
                "SELECT id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
                 AND sort_order {cmp} $3 ORDER BY sort_order {order} OFFSET $4 LIMIT $5"
            )
        };
        let mut ids: Vec<i32> = sqlx::query_scalar(&ids_where("<", "DESC"))
            .bind(topic_id)
            .bind(&VISIBLE_POST_TYPES[..])
            .bind(sort_order)
            .bind(0i64)
            .bind(before)
            .fetch_all(&mut *self.conn)
            .await?;
        let before_len = ids.len() as i64;
        let after: Vec<i32> = sqlx::query_scalar(&ids_where(">=", "ASC"))
            .bind(topic_id)
            .bind(&VISIBLE_POST_TYPES[..])
            .bind(sort_order)
            .bind(0i64)
            .bind(CHUNK_SIZE - before_len)
            .fetch_all(&mut *self.conn)
            .await?;
        ids.extend(after);
        if (ids.len() as i64) < CHUNK_SIZE {
            let more: Vec<i32> = sqlx::query_scalar(&ids_where("<", "DESC"))
                .bind(topic_id)
                .bind(&VISIBLE_POST_TYPES[..])
                .bind(sort_order)
                .bind(before_len)
                .bind(CHUNK_SIZE - ids.len() as i64)
                .fetch_all(&mut *self.conn)
                .await?;
            ids.extend(more);
        }

        // filter_posts_by_ids: reloaded in sort order.
        let sql = format!(
            "{} WHERE topic_id = $1 AND id = ANY($2) ORDER BY sort_order",
            Self::POST_SQL
        );
        Ok(sqlx::query_as(&sql)
            .bind(topic_id)
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// TopicView#actions_summary: every topic flag type with a zero count.
    async fn actions_summary(&mut self, no_posts: bool) -> Result<Value, TopicViewError> {
        if no_posts {
            return Ok(json!([]));
        }
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM flags WHERE 'Topic' = ANY(applies_to) AND NOT score_type ORDER BY position",
        )
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(json!(
            ids.into_iter()
                .map(|id| json!({"id": id, "count": 0, "hidden": false, "can_act": false}))
                .collect::<Vec<_>>()
        ))
    }

    /// `topic.topic_timers.find_by(public_type: true)`; serializing one
    /// isn't ported.
    async fn topic_timer(&mut self, topic_id: i32) -> Result<Value, TopicViewError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM topic_timers WHERE topic_id = $1 AND public_type AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_one(&mut *self.conn)
        .await?;
        if count > 0 {
            return Err(Unsupported("topic timers (TopicTimerSerializer)").into());
        }
        Ok(Value::Null)
    }

    /// The clear_slow_mode timer's execute_at, if any (status_type 9).
    async fn slow_mode_enabled_until(&mut self, topic_id: i32) -> Result<Value, TopicViewError> {
        let at: Option<NaiveDateTime> = sqlx::query_scalar(
            "SELECT execute_at FROM topic_timers WHERE topic_id = $1 AND status_type = 9 AND deleted_at IS NULL \
             ORDER BY id LIMIT 1",
        )
        .bind(topic_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(json!(at.map(time_json)))
    }

    /// `post_counts_by_user` (top 24 by post count) joined with users, and
    /// `participant_count`.
    async fn participants(
        &mut self,
        topic_id: i32,
        _post_ids: &[i32],
    ) -> Result<(Vec<(PostUser, i64)>, i64), TopicViewError> {
        let counts: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT user_id, count(*) AS count_all FROM posts \
             WHERE topic_id = $1 AND post_type = ANY($2) AND user_id IS NOT NULL \
               AND deleted_at IS NULL AND action_code IS NULL \
             GROUP BY user_id ORDER BY count_all DESC LIMIT 24",
        )
        .bind(topic_id)
        .bind(&VISIBLE_POST_TYPES[..])
        .fetch_all(&mut *self.conn)
        .await?;
        let ids: Vec<i32> = counts.iter().map(|(id, _)| *id).collect();
        let users: Vec<PostUser> = sqlx::query_as(
            "SELECT id, username, name, uploaded_avatar_id, primary_group_id, flair_group_id, admin, moderator, \
                    trust_level, title, suspended_till FROM users WHERE id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let participants: Vec<(PostUser, i64)> = counts
            .iter()
            .filter_map(|(id, n)| users.iter().find(|u| u.id == *id).map(|u| (u.clone(), *n)))
            .collect();
        // participant_count: with a full page of 24, count distinct posters
        // (the topic's cached participant_count only for huge topics).
        let count = if participants.len() == 24 {
            let posts_count: i32 =
                sqlx::query_scalar("SELECT posts_count FROM topics WHERE id = $1")
                    .bind(topic_id)
                    .fetch_one(&mut *self.conn)
                    .await?;
            if posts_count > 500 {
                sqlx::query_scalar::<_, i32>("SELECT participant_count FROM topics WHERE id = $1")
                    .bind(topic_id)
                    .fetch_one(&mut *self.conn)
                    .await? as i64
            } else {
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(DISTINCT user_id) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2)",
                )
                .bind(topic_id)
                .bind(&VISIBLE_POST_TYPES[..])
                .fetch_one(&mut *self.conn)
                .await?
            }
        } else {
            participants.len() as i64
        };
        Ok((participants, count))
    }

    /// TopicViewDetailsSerializer for anonymous users.
    async fn details(
        &mut self,
        topic: &TopicRow,
        participants: Vec<(PostUser, i64)>,
    ) -> Result<Value, TopicViewError> {
        let logo_small_url = self.list_serializer().logo_small_url().await?;
        let group_ids: Vec<i32> = participants
            .iter()
            .flat_map(|(u, _)| [u.primary_group_id, u.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;
        let enable_names = self.settings.get("enable_names")?.truthy();
        let mut out = Map::new();
        out.insert("can_edit".into(), json!(false));
        out.insert("notification_level".into(), json!(1));
        if !participants.is_empty() {
            let mut list = Vec::with_capacity(participants.len());
            for (user, post_count) in &participants {
                let mut p = Map::new();
                p.insert("id".into(), json!(user.id));
                p.insert("username".into(), json!(user.username));
                if enable_names {
                    p.insert("name".into(), json!(user.name));
                }
                // Hash-wrapped users take the class-level template.
                p.insert(
                    "avatar_template".into(),
                    json!(avatar::class_avatar_template(
                        self.urls,
                        &user.username,
                        user.uploaded_avatar_id
                    )?),
                );
                p.insert("post_count".into(), json!(post_count));
                let primary = user.primary_group_id.and_then(|id| groups.get(&id));
                let flair = user.flair_group_id.and_then(|id| groups.get(&id));
                p.insert(
                    "primary_group_name".into(),
                    json!(primary.map(|g| g.name.clone())),
                );
                p.insert("flair_name".into(), json!(flair.map(|g| g.name.clone())));
                p.insert(
                    "flair_url".into(),
                    json!(flair.map(|g| g.flair_url()).transpose()?.flatten()),
                );
                p.insert(
                    "flair_color".into(),
                    json!(flair.and_then(|g| g.flair_color.clone())),
                );
                p.insert(
                    "flair_bg_color".into(),
                    json!(flair.and_then(|g| g.flair_bg_color.clone())),
                );
                p.insert("flair_group_id".into(), json!(user.flair_group_id));
                if user.admin {
                    p.insert("admin".into(), json!(true));
                }
                if user.moderator {
                    p.insert("moderator".into(), json!(true));
                }
                p.insert("trust_level".into(), json!(user.trust_level));
                list.push(Value::Object(p));
            }
            out.insert("participants".into(), Value::Array(list));
        }
        let created_by = match topic.user_id {
            Some(id) => self.basic_user(id, logo_small_url.as_deref()).await?,
            None => Value::Null,
        };
        out.insert("created_by".into(), created_by);
        out.insert(
            "last_poster".into(),
            self.basic_user(topic.last_post_user_id, logo_small_url.as_deref())
                .await?,
        );
        let links: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM topic_links WHERE topic_id = $1 AND clicks > 0",
        )
        .bind(topic.id)
        .fetch_one(&mut *self.conn)
        .await?;
        if links > 0 {
            return Err(Unsupported("topic links with clicks (TopicLink.topic_map)").into());
        }
        Ok(Value::Object(out))
    }

    fn avatar(
        &self,
        user: &PostUser,
        logo_small_url: Option<&str>,
    ) -> Result<String, TopicViewError> {
        Ok(avatar::avatar_template(
            self.urls,
            user.id,
            &user.username,
            user.uploaded_avatar_id,
            logo_small_url,
        )?)
    }

    /// BasicUserSerializer: id, username, name (enable_names), avatar_template.
    async fn basic_user(
        &mut self,
        id: i32,
        logo_small_url: Option<&str>,
    ) -> Result<Value, TopicViewError> {
        let user: Option<PostUser> = sqlx::query_as(
            "SELECT id, username, name, uploaded_avatar_id, primary_group_id, flair_group_id, admin, moderator, \
                    trust_level, title, suspended_till FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(user) = user else {
            return Ok(Value::Null);
        };
        let mut out = Map::new();
        out.insert("id".into(), json!(user.id));
        out.insert("username".into(), json!(user.username));
        if self.settings.get("enable_names")?.truthy() {
            out.insert("name".into(), json!(user.name));
        }
        out.insert(
            "avatar_template".into(),
            json!(self.avatar(&user, logo_small_url)?),
        );
        Ok(Value::Object(out))
    }

    /// PostSerializer for each post of the chunk.
    async fn serialize_posts(
        &mut self,
        topic: &TopicRow,
        slug: &str,
        posts: &[PostRow],
    ) -> Result<Vec<Value>, TopicViewError> {
        let logo_small_url = self.list_serializer().logo_small_url().await?;
        let enable_names = self.settings.get("enable_names")?.truthy();
        let show_badges = self.settings.get("enable_badges")?.truthy()
            && self.settings.get("show_badges_in_post_header")?.truthy();
        let edit_history_public = self
            .settings
            .get("edit_history_visible_to_public")?
            .truthy();
        let suppress_reply_when_quoting =
            self.settings.get("suppress_reply_when_quoting")?.truthy();

        let user_ids: Vec<i32> = posts
            .iter()
            .filter_map(|p| p.user_id)
            .chain(posts.iter().filter_map(|p| p.reply_to_user_id))
            .collect();
        let users: Vec<PostUser> = sqlx::query_as(
            "SELECT id, username, name, uploaded_avatar_id, primary_group_id, flair_group_id, admin, moderator, \
                    trust_level, title, suspended_till FROM users WHERE id = ANY($1)",
        )
        .bind(&user_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let group_ids: Vec<i32> = users
            .iter()
            .flat_map(|u| [u.primary_group_id, u.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;
        let user = |id: Option<i32>| id.and_then(|id| users.iter().find(|u| u.id == id));

        let mut out = Vec::with_capacity(posts.len());
        for post in posts {
            let u = user(post.user_id);
            if post.hidden {
                return Err(Unsupported("hidden posts").into());
            }
            // UserBadge.for_post_header_badges: badges granted to the author
            // for this post; serializing one isn't ported.
            let badges: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM user_badges ub \
                 WHERE ub.post_id = $1 AND ub.user_id = $2 AND ub.badge_id IN \
                   (SELECT id FROM badges WHERE show_posts AND enabled AND listable AND show_in_post_header)",
            )
            .bind(post.id)
            .bind(post.user_id)
            .fetch_one(&mut *self.conn)
            .await?;
            if show_badges && badges > 0 {
                return Err(Unsupported("badges_granted (BasicUserBadgeSerializer)").into());
            }

            let mut p = Map::new();
            p.insert("id".into(), json!(post.id));
            if enable_names {
                p.insert("name".into(), json!(u.and_then(|u| u.name.clone())));
            }
            p.insert("username".into(), json!(u.map(|u| u.username.clone())));
            let avatar = match u {
                Some(u) => Some(self.avatar(u, logo_small_url.as_deref())?),
                None => None,
            };
            p.insert("avatar_template".into(), json!(avatar));
            p.insert("created_at".into(), json!(time_json(post.created_at)));
            p.insert("cooked".into(), json!(post.cooked));
            p.insert("post_number".into(), json!(post.post_number));
            p.insert("post_type".into(), json!(post.post_type));
            p.insert("posts_count".into(), json!(topic.posts_count));
            p.insert("updated_at".into(), json!(time_json(post.updated_at)));
            p.insert("reply_count".into(), json!(post.reply_count));
            p.insert(
                "reply_to_post_number".into(),
                json!(post.reply_to_post_number),
            );
            p.insert("quote_count".into(), json!(post.quote_count));
            p.insert(
                "incoming_link_count".into(),
                json!(post.incoming_link_count),
            );
            p.insert("reads".into(), json!(post.reads));
            p.insert("readers_count".into(), json!((post.reads - 1).max(0)));
            p.insert(
                "score".into(),
                match post.score {
                    Some(s) => json!(s),
                    None => json!(0),
                },
            );
            p.insert("yours".into(), json!(post.user_id.is_none()));
            p.insert("topic_id".into(), json!(topic.id));
            p.insert("topic_slug".into(), json!(slug));
            if enable_names {
                p.insert(
                    "display_username".into(),
                    json!(u.and_then(|u| u.name.clone())),
                );
            }
            let primary = u
                .and_then(|u| u.primary_group_id)
                .and_then(|id| groups.get(&id));
            let flair = u
                .and_then(|u| u.flair_group_id)
                .and_then(|id| groups.get(&id));
            p.insert(
                "primary_group_name".into(),
                json!(primary.map(|g| g.name.clone())),
            );
            p.insert("flair_name".into(), json!(flair.map(|g| g.name.clone())));
            p.insert(
                "flair_url".into(),
                json!(flair.map(|g| g.flair_url()).transpose()?.flatten()),
            );
            p.insert(
                "flair_bg_color".into(),
                json!(flair.and_then(|g| g.flair_bg_color.clone())),
            );
            p.insert(
                "flair_color".into(),
                json!(flair.and_then(|g| g.flair_color.clone())),
            );
            p.insert(
                "flair_group_id".into(),
                json!(u.and_then(|u| u.flair_group_id)),
            );
            p.insert("badges_granted".into(), json!([]));
            p.insert("version".into(), json!(post.public_version));
            p.insert("can_edit".into(), json!(false));
            p.insert("can_delete".into(), json!(false));
            p.insert("can_recover".into(), json!(false));
            p.insert("can_see_hidden_post".into(), json!(false));
            p.insert("can_wiki".into(), json!(false));
            let link_counts = self.link_counts(post.id).await?;
            if !link_counts.is_empty() {
                p.insert("link_counts".into(), Value::Array(link_counts));
            }
            p.insert("read".into(), json!(true));
            p.insert("user_title".into(), json!(u.and_then(|u| u.title.clone())));
            if u.and_then(|u| u.title.as_deref())
                .is_some_and(|t| !t.is_empty())
            {
                return Err(Unsupported("title_is_group").into());
            }
            if let Some(reply_to) = post.reply_to_user_id {
                if !(suppress_reply_when_quoting && post.reply_quoted) {
                    if let Some(ru) = user(Some(reply_to)) {
                        let mut r = Map::new();
                        r.insert("id".into(), json!(ru.id));
                        r.insert("username".into(), json!(ru.username));
                        if enable_names {
                            r.insert("name".into(), json!(ru.name));
                        }
                        r.insert(
                            "avatar_template".into(),
                            json!(self.avatar(ru, logo_small_url.as_deref())?),
                        );
                        p.insert("reply_to_user".into(), Value::Object(r));
                    }
                }
            }
            p.insert("bookmarked".into(), json!(false));
            let actions = if post.like_count > 0 {
                json!([{"id": LIKE, "count": post.like_count}])
            } else {
                json!([])
            };
            p.insert("actions_summary".into(), actions);
            p.insert("moderator".into(), json!(u.is_some_and(|u| u.moderator)));
            p.insert("admin".into(), json!(u.is_some_and(|u| u.admin)));
            p.insert(
                "staff".into(),
                json!(u.is_some_and(|u| u.admin || u.moderator)),
            );
            p.insert("user_id".into(), json!(post.user_id));
            p.insert("hidden".into(), json!(post.hidden));
            if let Some(reason) = post.hidden_reason_id {
                p.insert("hidden_reason_id".into(), json!(reason));
            }
            p.insert("trust_level".into(), json!(u.map(|u| u.trust_level)));
            p.insert("deleted_at".into(), Value::Null);
            p.insert("user_deleted".into(), json!(post.user_deleted));
            p.insert("edit_reason".into(), json!(post.edit_reason));
            p.insert(
                "can_view_edit_history".into(),
                json!(post.wiki || edit_history_public),
            );
            p.insert("wiki".into(), json!(post.wiki));
            if let Some(code) = &post.action_code {
                p.insert("action_code".into(), json!(code));
                let fields: Vec<(String, Option<String>)> = sqlx::query_as(
                    "SELECT name, value FROM post_custom_fields WHERE post_id = $1 \
                     AND name IN ('action_code_who', 'action_code_path') ORDER BY id",
                )
                .bind(post.id)
                .fetch_all(&mut *self.conn)
                .await?;
                for key in ["action_code_who", "action_code_path"] {
                    if let Some(value) = fields
                        .iter()
                        .find(|(n, _)| n == key)
                        .and_then(|(_, v)| v.clone())
                    {
                        if !value.is_empty() {
                            p.insert(key.into(), json!(value));
                        }
                    }
                }
            }
            if post.user_id.is_none() {
                p.insert("locked".into(), json!(false));
            }
            if u.is_some_and(|u| u.suspended_till.is_some()) {
                return Err(Unsupported("user_suspended").into());
            }
            p.insert(
                "post_url".into(),
                json!(format!(
                    "{}/t/{slug}/{}/{}",
                    self.urls.config.globals.relative_url_root(),
                    topic.id,
                    post.post_number
                )),
            );
            out.push(Value::Object(p));
        }
        Ok(out)
    }

    /// A deterministic stand-in for `random_suggested`: the same filters
    /// (open, unarchived, visible, readable, not a definition, not this
    /// topic), same-category first, then by bumped_at; capped at
    /// `suggested_topics`. Never matches Rails' random pick.
    async fn suggested_topics(&mut self, topic: &TopicRow) -> Result<Value, TopicViewError> {
        let limit = self.settings.get("suggested_topics")?.to_i();
        let sql = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics LEFT OUTER JOIN categories ON categories.id = topics.category_id \
             WHERE topics.deleted_at IS NULL AND topics.visible AND NOT topics.closed AND NOT topics.archived \
               AND topics.archetype <> 'private_message' AND topics.id <> $1 \
               AND (categories.id IS NULL OR NOT categories.read_restricted) \
               AND COALESCE(categories.topic_id, 0) <> topics.id \
             ORDER BY CASE WHEN topics.category_id = $2 THEN 0 ELSE 1 END, topics.bumped_at DESC LIMIT $3"
        );
        let rows: Vec<TopicRow> = sqlx::query_as(&sql)
            .bind(topic.id)
            .bind(topic.category_id)
            .bind(limit)
            .fetch_all(&mut *self.conn)
            .await?;
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        let mut serializer = self.list_serializer();
        let lookup = serializer.user_lookup(&rows).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let posters = topic_list::posters_summary(row, &lookup, serializer.i18n);
            out.push(
                serializer
                    .serialize_topic(row, &posters, tagging, Mode::Suggested)
                    .await?,
            );
        }
        Ok(Value::Array(out))
    }
}

/// `TimelineLookup.build`: sampled `[post index, days ago]` pairs, at most
/// `max_values` of them, one per distinct day.
pub fn timeline_lookup(stream: &[(i32, i32)], max_values: usize) -> Vec<[i64; 2]> {
    let len = stream.len();
    if len == 0 {
        return Vec::new();
    }
    let every = (len as f64 / max_values as f64).ceil().max(1.0) as usize;
    let mut last = -1;
    let mut out = Vec::new();
    for (idx, (_, days_ago)) in stream.iter().enumerate() {
        if idx != len - 1 && idx % every != 0 {
            continue;
        }
        if *days_ago != last {
            out.push([idx as i64 + 1, i64::from(*days_ago)]);
            last = *days_ago;
        }
    }
    out
}

impl TopicView<'_> {
    /// `TopicLink.counts_for` for one visible (non-hidden) source post, as
    /// PostSerializer#link_counts shapes it: links whose target topic is
    /// visible and readable, external links always, `ORDER BY reflection,
    /// clicks DESC`.
    async fn link_counts(&mut self, post_id: i32) -> Result<Vec<Value>, TopicViewError> {
        #[derive(sqlx::FromRow)]
        struct Link {
            url: String,
            clicks: i32,
            title: Option<String>,
            internal: bool,
            reflection: Option<bool>,
        }
        let links: Vec<Link> = sqlx::query_as(
            "SELECT l.url, l.clicks, COALESCE(t.title, l.title) AS title, l.internal, l.reflection \
             FROM topic_links l \
             LEFT JOIN topics t ON t.id = l.link_topic_id \
             LEFT JOIN categories c ON c.id = t.category_id \
             LEFT JOIN posts target_posts ON l.link_post_id = target_posts.id \
             WHERE l.post_id = $1 \
               AND t.deleted_at IS NULL \
               AND (t.id IS NULL OR t.visible = true) \
               AND (l.internal = false OR t.id IS NOT NULL) \
               AND (l.link_post_id IS NULL OR (target_posts.id IS NOT NULL AND target_posts.deleted_at IS NULL)) \
               AND COALESCE(t.archetype, 'regular') <> 'private_message' \
               AND (c.id IS NULL OR NOT c.read_restricted) \
             ORDER BY l.reflection ASC, l.clicks DESC",
        )
        .bind(post_id)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(links
            .into_iter()
            .map(|l| {
                let mut out = Map::new();
                out.insert("url".into(), json!(l.url));
                out.insert("internal".into(), json!(l.internal));
                out.insert("reflection".into(), json!(l.reflection));
                if let Some(title) = l.title.filter(|t| !t.is_empty()) {
                    out.insert("title".into(), json!(title));
                }
                out.insert("clicks".into(), json!(l.clicks));
                Value::Object(out)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_lookup_samples_days() {
        assert_eq!(
            timeline_lookup(&[(1, 0), (2, 0), (3, 0), (4, 0)], 300),
            vec![[1, 0]]
        );
        assert_eq!(
            timeline_lookup(&[(1, 5), (2, 5), (3, 2), (4, 0)], 300),
            vec![[1, 5], [3, 2], [4, 0]]
        );
        assert!(timeline_lookup(&[], 300).is_empty());
        // 600 posts sampled every 2, last index always considered.
        let stream: Vec<(i32, i32)> = (0..600).map(|i| (i, 600 - i)).collect();
        let lookup = timeline_lookup(&stream, 300);
        assert_eq!(lookup.len(), 301);
        assert_eq!(lookup[0], [1, 600]);
        assert_eq!(lookup.last().unwrap(), &[600, 1]);
    }
}
