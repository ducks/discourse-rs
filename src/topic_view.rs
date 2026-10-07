//! Port of lib/topic_view.rb, TopicViewSerializer, TopicViewDetailsSerializer
//! and PostSerializer (with BasicPostSerializer) for anonymous users.
//!
//! Plugin-added keys (reactions, solved, voting, zendesk, ...) are not
//! emitted; parity/cases ignores them. Suggested topics are served in a
//! deterministic order because Discourse's RandomTopicSelector consumes a
//! Redis list, so their content can never match a recording.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::avatar::{self, AvatarError};
use crate::guardian::{Guardian, GuardianError};
use crate::i18n::I18n;
use crate::post_actions::{self, ActOpts, ActionTypes, TakenAction};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::topic_list::{self, Mode, TopicListError, TopicListSerializer, time_json};
use crate::topic_query::{Filter, TOPIC_COLUMNS, TopicQuery, TopicRow};
use crate::url::{UrlError, Urls};

/// `TopicView::CHUNK_SIZE`
pub const CHUNK_SIZE: i64 = 20;

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

impl From<crate::topic_query::TopicQueryError> for TopicViewError {
    fn from(e: crate::topic_query::TopicQueryError) -> Self {
        use crate::topic_query::TopicQueryError as E;
        match e {
            E::Db(e) => TopicViewError::Db(e),
            E::Setting(e) => TopicViewError::Setting(e),
            E::Unsupported(e) => TopicViewError::Unsupported(e),
        }
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

impl From<GuardianError> for TopicViewError {
    fn from(e: GuardianError) -> Self {
        match e {
            GuardianError::Db(e) => TopicViewError::Db(e),
            GuardianError::Setting(e) => TopicViewError::Setting(e),
            GuardianError::Unsupported(e) => TopicViewError::Unsupported(e),
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
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct PostRow {
    id: i32,
    user_id: Option<i32>,
    topic_id: i32,
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
    hidden_at: Option<NaiveDateTime>,
    locked_by_id: Option<i32>,
    deleted_at: Option<NaiveDateTime>,
    version: i32,
    notify_user_count: i32,
    off_topic_count: i32,
    inappropriate_count: i32,
    spam_count: i32,
    illegal_count: i32,
    notify_moderators_count: i32,
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

/// What a logged-in viewer adds to a TopicView: the guardian's answers
/// about the topic and the per-user rows (`topic_user`, `read_posts_set`,
/// `bookmarks`, `all_post_actions`, the draft sequence).
pub struct Viewer {
    pub ctx: TopicCtx,
    pub can_see: bool,
    pub secure_category_ids: Vec<i32>,
    pub topic_user: Option<TopicUserRow>,
    pub read_post_numbers: Vec<i32>,
    pub bookmarks: Vec<BookmarkRow>,
    pub taken: HashMap<i32, HashMap<i64, TakenAction>>,
    pub can_post_anywhere: bool,
    pub can_create_post: bool,
    pub action_types: ActionTypes,
    pub draft_sequence: Option<i64>,
    pub queue_enabled: bool,
    pub has_deleted: Option<bool>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TopicUserRow {
    pub posted: bool,
    pub last_read_post_number: Option<i32>,
    pub notification_level: i32,
    pub notifications_reason_id: Option<i32>,
    pub cleared_pinned_at: Option<NaiveDateTime>,
    pub last_posted_at: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BookmarkRow {
    pub id: i64,
    pub bookmarkable_id: i64,
    pub bookmarkable_type: String,
    pub reminder_at: Option<NaiveDateTime>,
    pub name: Option<String>,
    pub auto_delete_preference: i32,
    pub post_number: Option<i32>,
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
    /// `Topic.visible_post_types(user)`, set by render.
    pub post_types: Vec<i32>,
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
        // check_and_raise_exceptions: PMs need a login (not ported), and
        // what can_see_topic? refuses is a 404 (detailed_404 off).
        // check_and_raise_exceptions: a PM for a visitor is NotLoggedIn,
        // which topics#show turns into a 404 while detailed_404 is off.
        let pm = topic.archetype == "private_message";
        if pm && self.guardian.is_anonymous() {
            return Err(TopicViewError::NotFound);
        }
        let mut viewer = self.load_viewer(topic.id).await?;
        if !viewer.can_see {
            if self.settings.get("detailed_404")?.truthy() {
                return Err(Unsupported("detailed_404").into());
            }
            return Err(TopicViewError::NotFound);
        }
        if extra.deleted_at.is_some() {
            return Err(Unsupported("viewing deleted topics as staff").into());
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
        // all_post_actions: the viewer's rows on the page's posts.
        viewer.taken = post_actions::taken_actions(
            &mut *self.conn,
            &posts.iter().map(|p| p.id).collect::<Vec<_>>(),
            self.guardian.user_id(),
        )
        .await?;
        let next_page = match (posts.last(), highest_post_number) {
            (Some(last), Some(highest)) if highest > last.post_number => Some(page + 1),
            _ => None,
        };

        let mut out = Map::new();
        let post_ids: Vec<i32> = posts.iter().map(|p| p.id).collect();
        let serialized_posts = self.serialize_posts(&topic, &slug, &posts, &viewer).await?;
        out.insert(
            "post_stream".into(),
            json!({"posts": serialized_posts, "stream": stream.iter().map(|(id, _)| *id).collect::<Vec<_>>()}),
        );
        out.insert(
            "timeline_lookup".into(),
            json!(timeline_lookup(&stream, 300)),
        );
        // related_messages and suggested messages for PM participants in
        // personal_message_enabled_groups; suggested topics otherwise.
        let pm_eligible = pm
            && self.guardian.is_authenticated()
            && self
                .guardian
                .in_setting_groups(self.settings, "personal_message_enabled_groups")?;
        if pm_eligible {
            out.insert(
                "related_messages".into(),
                self.related_messages(&topic).await?,
            );
        }
        if next_page.is_none() {
            if pm {
                if pm_eligible {
                    out.insert(
                        "suggested_topics".into(),
                        self.suggested_messages(&topic).await?,
                    );
                    out.insert(
                        "suggested_group_name".into(),
                        self.suggested_group_name(&topic).await?,
                    );
                }
            } else {
                out.insert(
                    "suggested_topics".into(),
                    self.suggested_topics(&topic).await?,
                );
            }
        }
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        if tagging && (!pm || self.guardian.can_tag_pms(self.settings)?) {
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
        // `draft` is only loaded when the visit is tracked (HTML), which
        // the JSON document never does.
        out.insert("draft".into(), Value::Null);
        out.insert("draft_key".into(), json!(format!("topic_{}", topic.id)));
        out.insert("draft_sequence".into(), json!(viewer.draft_sequence));
        if let Some(tu) = &viewer.topic_user {
            out.insert("posted".into(), json!(tu.posted));
        }
        // PinnedCheck with the viewer's topic_user.
        let unpinned = match (
            topic.pinned_at,
            viewer
                .topic_user
                .as_ref()
                .and_then(|tu| tu.cleared_pinned_at),
        ) {
            (Some(pinned_at), Some(cleared)) => Some(cleared > pinned_at),
            _ => None,
        };
        out.insert("unpinned".into(), json!(unpinned));
        out.insert(
            "pinned".into(),
            json!(topic.pinned_at.is_some() && unpinned != Some(true)),
        );
        if let Some(highest) = highest_post_number {
            out.insert(
                "current_post_number".into(),
                json!(post_number.min(i64::from(highest))),
            );
        }
        out.insert("highest_post_number".into(), json!(highest_post_number));
        if let Some(tu) = &viewer.topic_user {
            out.insert(
                "last_read_post_number".into(),
                json!(tu.last_read_post_number),
            );
            let last_read_post_id: Option<i32> = match tu.last_read_post_number {
                Some(n) => {
                    sqlx::query_scalar(
                        "SELECT id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
                         AND post_number = $3 LIMIT 1",
                    )
                    .bind(topic.id)
                    .bind(&self.post_types)
                    .bind(n)
                    .fetch_optional(&mut *self.conn)
                    .await?
                }
                None => None,
            };
            out.insert("last_read_post_id".into(), json!(last_read_post_id));
        }
        out.insert("deleted_by".into(), Value::Null);
        if let Some(has_deleted) = viewer.has_deleted {
            out.insert("has_deleted".into(), json!(has_deleted));
        }
        let topic_actions = self
            .actions_summary(posts.is_empty(), &viewer, posts.first())
            .await?;
        out.insert("actions_summary".into(), topic_actions.clone());
        out.insert("chunk_size".into(), json!(CHUNK_SIZE));
        out.insert("bookmarked".into(), json!(!viewer.bookmarks.is_empty()));
        if pm {
            out.insert(
                "message_archived".into(),
                json!(self.message_archived(topic.id).await?),
            );
        }
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
        if pm {
            out.insert(
                "pm_with_non_human_user".into(),
                json!(self.pm_with_non_human_user(topic.id).await?),
            );
        }
        if self.guardian.is_staff() && viewer.queue_enabled {
            if !self.guardian.is_admin() {
                return Err(Unsupported("queued_posts_count for moderators").into());
            }
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM reviewables WHERE type = 'ReviewableQueuedPost' AND topic_id = $1 AND status = 0",
            )
            .bind(topic.id)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("queued_posts_count".into(), json!(count));
        }
        out.insert("show_read_indicator".into(), json!(false));
        if topic.image_upload_id.is_some() {
            return Err(Unsupported("topic thumbnails").into());
        }
        out.insert("thumbnails".into(), Value::Null);
        out.insert(
            "slow_mode_enabled_until".into(),
            self.slow_mode_enabled_until(topic.id).await?,
        );
        out.insert(
            "details".into(),
            self.details(&topic, participants, &viewer, &topic_actions)
                .await?,
        );
        if self.guardian.is_authenticated() && viewer.queue_enabled {
            let pending: Vec<(i64, NaiveDateTime)> = sqlx::query_as(
                "SELECT id, created_at FROM reviewables WHERE type = 'ReviewableQueuedPost' AND status = 0 \
                 AND target_created_by_id = $1 AND topic_id = $2 ORDER BY created_at ASC",
            )
            .bind(self.guardian.user_id())
            .bind(topic.id)
            .fetch_all(&mut *self.conn)
            .await?;
            if !pending.is_empty() {
                return Err(Unsupported("pending queued posts (raw from payload)").into());
            }
            out.insert("pending_posts".into(), json!([]));
        }
        out.insert(
            "bookmarks".into(),
            json!(viewer
                .bookmarks
                .iter()
                .map(|b| json!({
                    "id": b.id, "bookmarkable_id": b.bookmarkable_id, "bookmarkable_type": b.bookmarkable_type,
                    "reminder_at": b.reminder_at.map(time_json), "name": b.name,
                    "auto_delete_preference": b.auto_delete_preference, "post_number": b.post_number,
                }))
                .collect::<Vec<_>>()),
        );

        Ok(Rendered {
            json: Value::Object(out),
            slug,
        })
    }

    /// The guardian's view of the topic and the viewer's rows, loaded
    /// before anything is serialized. Sets `post_types`.
    async fn load_viewer(&mut self, topic_id: i32) -> Result<Viewer, TopicViewError> {
        self.post_types = self.guardian.visible_post_types(self.settings)?;
        let ctx = TopicCtx::load(&mut *self.conn, self.settings, self.guardian, topic_id)
            .await?
            .ok_or(TopicViewError::NotFound)?;
        let secure_category_ids = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        let can_see =
            self.guardian
                .can_see_topic(self.settings, &ctx, true, &secure_category_ids)?;
        let action_types = ActionTypes::load(&mut *self.conn).await?;
        let mut viewer = Viewer {
            can_see,
            secure_category_ids,
            topic_user: None,
            read_post_numbers: Vec::new(),
            bookmarks: Vec::new(),
            taken: HashMap::new(),
            can_post_anywhere: false,
            can_create_post: false,
            action_types,
            draft_sequence: None,
            queue_enabled: false,
            has_deleted: None,
            ctx,
        };
        let Some(uid) = self.guardian.user_id() else {
            return Ok(viewer);
        };
        if !can_see {
            return Ok(viewer);
        }
        // Ignored users remove their replies from the stream (with gaps).
        let ignoring: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM ignored_users ig JOIN users u ON u.id = ig.ignored_user_id \
             WHERE ig.user_id = $1 AND ig.ignored_user_id <> $1 AND NOT u.admin AND NOT u.moderator)",
        )
        .bind(uid)
        .fetch_one(&mut *self.conn)
        .await?;
        if ignoring {
            return Err(Unsupported("ignored users in topic views").into());
        }
        if self.guardian.can_see_deleted(self.settings)? {
            // Deleted replies would be served with gaps (or inline with
            // show_deleted); neither is ported.
            let has_deleted: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND deleted_at IS NOT NULL AND post_number > 1)",
            )
            .bind(topic_id)
            .fetch_one(&mut *self.conn)
            .await?;
            if has_deleted {
                return Err(Unsupported("deleted replies for staff (post_stream.gaps)").into());
            }
            viewer.has_deleted = Some(false);
        }
        viewer.topic_user = sqlx::query_as(
            "SELECT posted, last_read_post_number, notification_level, notifications_reason_id, \
                    cleared_pinned_at, last_posted_at \
             FROM topic_users WHERE topic_id = $1 AND user_id = $2 LIMIT 1",
        )
        .bind(topic_id)
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?;
        if viewer.topic_user.is_some() {
            viewer.read_post_numbers = sqlx::query_scalar(
                "SELECT post_number FROM post_timings WHERE topic_id = $1 AND user_id = $2",
            )
            .bind(topic_id)
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
        }
        viewer.bookmarks = sqlx::query_as(
            "SELECT bookmarks.id, bookmarks.bookmarkable_id, bookmarks.bookmarkable_type, bookmarks.reminder_at, \
                    bookmarks.name, bookmarks.auto_delete_preference, posts.post_number \
             FROM bookmarks \
             LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
             LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                              OR (topics.id = posts.topic_id) \
             WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
               AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL ORDER BY bookmarks.id",
        )
        .bind(i64::from(uid))
        .bind(topic_id)
        .fetch_all(&mut *self.conn)
        .await?;
        viewer.can_post_anywhere = self
            .guardian
            .can_create_post_anywhere(&mut *self.conn, self.settings)
            .await?;
        viewer.can_create_post =
            self.guardian
                .can_create_post(self.settings, &viewer.ctx, viewer.can_post_anywhere)?;
        let sequence: Option<i64> = sqlx::query_scalar(
            "SELECT sequence FROM draft_sequences WHERE user_id = $1 AND draft_key = $2",
        )
        .bind(uid)
        .bind(format!("topic_{topic_id}"))
        .fetch_optional(&mut *self.conn)
        .await?;
        viewer.draft_sequence = Some(sequence.unwrap_or(0));
        viewer.queue_enabled = self.queue_enabled(&viewer.ctx).await?;
        Ok(viewer)
    }

    /// `PostSerializer.new(post, scope: guardian)` outside a topic view: no
    /// `read`; link counts when asked for (update sets
    /// `single_post_link_counts`); `raw` and `draft_sequence` for posts#create
    /// and posts#update (render_post_json with add_raw: false has neither).
    /// `with_viewer_actions`: the viewer's own actions on the post (`acted`,
    /// `can_undo`), as posts#show gives the serializer; without them, as
    /// serialize_data does for a list of posts (the posts feed), the
    /// serializer sees none.
    pub async fn serialize_single_post(
        &mut self,
        post_id: i32,
        with_link_counts: bool,
        with_raw_and_draft_sequence: bool,
        with_viewer_actions: bool,
    ) -> Result<Value, TopicViewError> {
        let sql = format!("{} WHERE id = $1", Self::POST_SQL);
        let post: PostRow = sqlx::query_as(&sql)
            .bind(post_id)
            .fetch_optional(&mut *self.conn)
            .await?
            .ok_or(TopicViewError::NotFound)?;
        let (topic, _) = self.find_topic(post.topic_id).await?;
        let mut viewer = self.load_viewer(topic.id).await?;
        if with_viewer_actions {
            viewer.taken =
                post_actions::taken_actions(&mut *self.conn, &[post.id], self.guardian.user_id())
                    .await?;
        }
        let Some(slug) = topic.slug.clone() else {
            return Err(Unsupported("topics without a stored slug (Slug.for)").into());
        };
        let raw: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&mut *self.conn)
            .await?;
        let mut serialized = self
            .serialize_posts(&topic, &slug, std::slice::from_ref(&post), &viewer)
            .await?;
        let Some(Value::Object(mut p)) = serialized.pop() else {
            return Err(TopicViewError::NotFound);
        };
        p.shift_remove("read");
        if !with_link_counts {
            p.shift_remove("link_counts");
        }
        // Without a topic view the reviewable lookup finds nothing as nil.
        if p.get("reviewable_id") == Some(&json!(0)) {
            p.insert("reviewable_id".into(), Value::Null);
        }
        // include_raw?: a hidden post's raw is for staff and its author.
        let raw_visible = !post.hidden
            || self.guardian.is_staff()
            || (self.guardian.user_id().is_some() && self.guardian.user_id() == post.user_id);
        if with_raw_and_draft_sequence {
            if raw_visible {
                p.insert("raw".into(), json!(raw));
            }
            p.insert("draft_sequence".into(), json!(viewer.draft_sequence));
        }
        Ok(Value::Object(p))
    }

    /// `NewPostManager.queue_enabled? || reply_posting_review_required?`:
    /// the settings and watched words that send posts to review. Plugin
    /// handlers (which also enable it) aren't ported.
    async fn queue_enabled(&mut self, ctx: &TopicCtx) -> Result<bool, TopicViewError> {
        let s = self.settings;
        let tl0 = crate::guardian::auto_groups::TRUST_LEVEL_0;
        if s.get("approve_post_count")?.to_i() > 0
            || !s.group_ids("approve_unless_allowed_groups")?.contains(&tl0)
            || !s
                .group_ids("approve_new_topics_unless_allowed_groups")?
                .contains(&tl0)
            || s.get("approve_unless_staged")?.truthy()
        {
            return Ok(true);
        }
        // WatchedWord.actions[:require_approval] = 4
        let watched: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM watched_words WHERE action = 4)")
                .fetch_one(&mut *self.conn)
                .await?;
        if watched {
            return Ok(true);
        }
        if let Some(category_id) = ctx.category_id {
            let review: Option<bool> = sqlx::query_scalar(
                "SELECT reply_posting_review_mode <> 0 FROM category_settings WHERE category_id = $1",
            )
            .bind(category_id)
            .fetch_optional(&mut *self.conn)
            .await?;
            if review == Some(true) {
                return Err(Unsupported("category reply posting review modes").into());
            }
        }
        Ok(false)
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
            group_id: None,
            prefetched: Default::default(),
        }
    }

    /// `Topic.with_deleted.find_by(id:)`.
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
            "SELECT word_count, deleted_at, pinned_until, slow_mode_seconds, external_id \
             FROM topics WHERE id = $1",
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
            "SELECT id, (EXTRACT(EPOCH FROM now() - posts.created_at) / 86400)::INT AS days_ago \
             FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2) \
             ORDER BY sort_order",
        )
        .bind(topic_id)
        .bind(&self.post_types)
        .fetch_all(&mut *self.conn)
        .await?)
    }

    async fn highest_post_number(&mut self, topic_id: i32) -> Result<Option<i32>, TopicViewError> {
        Ok(sqlx::query_scalar(
            "SELECT MAX(post_number) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_type = ANY($2)",
        )
        .bind(topic_id)
        .bind(&self.post_types)
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
        .bind(&self.post_types)
        .bind(post_number)
        .fetch_one(&mut *self.conn)
        .await?;
        Ok(((count - 1).max(0) / CHUNK_SIZE) + 1)
    }

    const POST_SQL: &'static str = "SELECT id, user_id, topic_id, post_number, cooked, created_at, updated_at, \
        reply_to_post_number, reply_count, quote_count, incoming_link_count, reads, score, post_type, \
        hidden, hidden_reason_id, user_deleted, reply_to_user_id, edit_reason, wiki, reply_quoted, \
        public_version, action_code, like_count, hidden_at, locked_by_id, deleted_at, version, \
        notify_user_count, off_topic_count, inappropriate_count, spam_count, illegal_count, \
        notify_moderators_count FROM posts";

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
            .bind(&self.post_types)
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
        .bind(&self.post_types)
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
            .bind(&self.post_types)
            .bind(sort_order)
            .bind(0i64)
            .bind(before)
            .fetch_all(&mut *self.conn)
            .await?;
        let before_len = ids.len() as i64;
        let after: Vec<i32> = sqlx::query_scalar(&ids_where(">=", "ASC"))
            .bind(topic_id)
            .bind(&self.post_types)
            .bind(sort_order)
            .bind(0i64)
            .bind(CHUNK_SIZE - before_len)
            .fetch_all(&mut *self.conn)
            .await?;
        ids.extend(after);
        if (ids.len() as i64) < CHUNK_SIZE {
            let more: Vec<i32> = sqlx::query_scalar(&ids_where("<", "DESC"))
                .bind(topic_id)
                .bind(&self.post_types)
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

    /// TopicView#actions_summary: every topic flag type with a zero count
    /// and whether the viewer may flag the first post with it.
    async fn actions_summary(
        &mut self,
        no_posts: bool,
        viewer: &Viewer,
        first_loaded: Option<&PostRow>,
    ) -> Result<Value, TopicViewError> {
        if no_posts {
            return Ok(json!([]));
        }
        // topic.first_post: the loaded page may start later.
        let first: Option<PostRow> = match first_loaded {
            Some(p) if p.post_number == 1 => Some(p.clone()),
            _ => {
                let sql = format!(
                    "{} WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
                    Self::POST_SQL
                );
                sqlx::query_as(&sql)
                    .bind(viewer.ctx.id)
                    .fetch_optional(&mut *self.conn)
                    .await?
            }
        };
        let mut out = Vec::new();
        for id in &viewer.action_types.topic_flag_ids {
            let key = viewer
                .action_types
                .types
                .iter()
                .find(|(_, i)| i == id)
                .map(|(k, _)| k.as_str())
                .unwrap_or_default();
            let can_act = match (&first, self.guardian.is_authenticated()) {
                (Some(post), true) => {
                    let ctx = self.post_ctx(post).await?;
                    let can_see_post =
                        self.guardian
                            .can_see_post(self.settings, &ctx, viewer.can_see)?;
                    self.guardian.post_can_act(
                        self.settings,
                        &viewer.action_types,
                        (key, *id),
                        &ActOpts {
                            topic: &viewer.ctx,
                            post: &ctx,
                            taken: None,
                            can_see_post,
                            author_missing: post.user_id.is_none(),
                        },
                    )?
                }
                _ => false,
            };
            out.push(json!({"id": id, "count": 0, "hidden": false, "can_act": can_act}));
        }
        Ok(Value::Array(out))
    }

    /// The post's `notice` custom field, parsed.
    async fn notice(&mut self, post_id: i32) -> Result<Option<Value>, TopicViewError> {
        let raw: Option<Option<String>> = sqlx::query_scalar(
            "SELECT value FROM post_custom_fields WHERE post_id = $1 AND name = 'notice' ORDER BY id LIMIT 1",
        )
        .bind(post_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(raw
            .flatten()
            .and_then(|v| serde_json::from_str::<Value>(&v).ok()))
    }
    /// The guardian's view of a post row.
    async fn post_ctx(&mut self, post: &PostRow) -> Result<PostCtx, TopicViewError> {
        let author_staff: bool = match post.user_id {
            Some(id) => sqlx::query_scalar("SELECT admin OR moderator FROM users WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *self.conn)
                .await?
                .unwrap_or(false),
            None => false,
        };
        Ok(PostCtx {
            id: post.id,
            user_id: post.user_id,
            post_number: post.post_number,
            post_type: post.post_type,
            hidden: post.hidden,
            hidden_at: post.hidden_at,
            locked_by_id: post.locked_by_id,
            deleted_at: post.deleted_at,
            user_deleted: post.user_deleted,
            wiki: post.wiki,
            created_at: post.created_at,
            author_staff,
        })
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
        .bind(&self.post_types)
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
                .bind(&self.post_types)
                .fetch_one(&mut *self.conn)
                .await?
            }
        } else {
            participants.len() as i64
        };
        Ok((participants, count))
    }

    /// TopicViewDetailsSerializer: `can_edit` and `notification_level`,
    /// then every `can_*` the guardian grants (`true` only), participants,
    /// created_by and last_poster.
    async fn details(
        &mut self,
        topic: &TopicRow,
        participants: Vec<(PostUser, i64)>,
        viewer: &Viewer,
        topic_actions: &Value,
    ) -> Result<Value, TopicViewError> {
        let logo_small_url = self.list_serializer().logo_small_url().await?;
        let group_ids: Vec<i32> = participants
            .iter()
            .flat_map(|(u, _)| [u.primary_group_id, u.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;
        let enable_names = self.settings.get("enable_names")?.truthy();
        let g = self.guardian;
        let s = self.settings;
        let ctx = &viewer.ctx;
        let can_see = viewer.can_see;
        let mut out = Map::new();
        let can_edit =
            g.is_authenticated() && g.can_edit_topic(s, ctx, can_see, viewer.can_post_anywhere)?;
        out.insert("can_edit".into(), json!(can_edit));
        let tu = viewer.topic_user.as_ref();
        out.insert(
            "notification_level".into(),
            json!(tu.map(|tu| tu.notification_level).unwrap_or(1)),
        );
        if let Some(tu) = tu {
            out.insert(
                "notifications_reason_id".into(),
                json!(tu.notifications_reason_id),
            );
        }
        if g.is_authenticated() {
            let group_mod_action =
                g.can_perform_action_available_to_group_moderators(s, can_see)?;
            let can_create_post_on_topic =
                g.can_create_post_on_topic(s, ctx, viewer.can_post_anywhere)?;
            // can_flag_topic: any topic-level action the viewer may take.
            let can_flag_topic = topic_actions
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x["can_act"] == json!(true)));
            let flags: [(&str, bool); 20] = [
                (
                    "can_move_posts",
                    !g.is_silenced()
                        && group_mod_action
                        && (g.is_staff() || !ctx.private_message()),
                ),
                ("can_delete", g.can_delete_topic(s, ctx)?),
                ("can_permanently_delete", false),
                ("can_recover", g.can_recover_topic(ctx)),
                ("can_remove_allowed_users", g.can_remove_allowed_users(ctx)),
                ("can_invite_to", g.can_invite_to(s, ctx, can_see)?),
                (
                    "can_invite_via_email",
                    g.can_invite_via_email(s, ctx, can_see)?,
                ),
                ("can_create_post", can_see && can_create_post_on_topic),
                ("can_reply_as_new_topic", g.can_reply_as_new_topic()),
                ("can_flag_topic", can_flag_topic),
                (
                    "can_convert_topic",
                    g.can_convert_topic(s, ctx, viewer.can_create_post)?,
                ),
                ("can_review_topic", g.can_review_topic(s, can_see)?),
                (
                    "can_edit_tags",
                    !can_edit && g.can_edit_tags(s, ctx, can_edit, viewer.can_create_post)?,
                ),
                ("can_publish_page", g.can_publish_page(s, ctx, can_see)?),
                ("can_close_topic", group_mod_action),
                ("can_archive_topic", group_mod_action),
                ("can_split_merge_topic", group_mod_action),
                ("can_edit_staff_notes", group_mod_action),
                (
                    "can_toggle_topic_visibility",
                    g.can_moderate(can_see) || group_mod_action,
                ),
                ("can_pin_unpin_topic", group_mod_action),
            ];
            for (key, granted) in flags {
                if granted {
                    out.insert(key.into(), json!(true));
                }
            }
            if g.can_banner_topic(ctx) {
                out.insert("can_banner_topic".into(), json!(true));
            }
            if group_mod_action {
                out.insert("can_moderate_category".into(), json!(true));
            }
            // can_remove_allowed_users?(topic, scope.user): staff, TL2
            // creators, or any non-creator participant of a 2+ user PM.
            let can_remove_self = g.can_remove_allowed_users(ctx)
                || (ctx.private_message()
                    && ctx.pm_member
                    && ctx.recipients > 1
                    && ctx.user_id != g.user_id());
            if can_remove_self {
                out.insert("can_remove_self_id".into(), json!(g.user_id()));
            }
        }
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
        if ctx.private_message() {
            out.insert(
                "allowed_users".into(),
                self.allowed_users(topic.id, logo_small_url.as_deref())
                    .await?,
            );
            out.insert(
                "allowed_groups".into(),
                self.allowed_groups(topic.id).await?,
            );
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
        viewer: &Viewer,
    ) -> Result<Vec<Value>, TopicViewError> {
        let logo_small_url = self.list_serializer().logo_small_url().await?;
        let enable_names = self.settings.get("enable_names")?.truthy();
        let show_badges = self.settings.get("enable_badges")?.truthy()
            && self.settings.get("show_badges_in_post_header")?.truthy();
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

        // TopicView#mentioned_users, serialized with enable_user_status: each
        // post's mentions in order, the unknown ones left out.
        let mut mentioned: HashMap<i32, Vec<i32>> = HashMap::new();
        if self.settings.get("enable_user_status")?.truthy() {
            let mut by_post = Vec::with_capacity(posts.len());
            for post in posts {
                by_post.push((post.id, crate::pretty_text::extract_mentions(&post.cooked)?));
            }
            let usernames: Vec<&String> = by_post.iter().flat_map(|(_, m)| m).collect();
            let found: Vec<(i32, String)> = sqlx::query_as(
                "SELECT id, username_lower FROM users WHERE username_lower = ANY($1)",
            )
            .bind(&usernames)
            .fetch_all(&mut *self.conn)
            .await?;
            let ids: Vec<i32> = found.iter().map(|(id, _)| *id).collect();
            let with_status: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = ANY($1) \
                 AND (ends_at IS NULL OR ends_at > now()))",
            )
            .bind(&ids)
            .fetch_one(&mut *self.conn)
            .await?;
            if with_status {
                return Err(Unsupported("user status on mentioned users").into());
            }
            for (post_id, names) in by_post {
                let users = names
                    .iter()
                    .filter_map(|n| found.iter().find(|(_, u)| u == n).map(|(id, _)| *id))
                    .collect();
                mentioned.insert(post_id, users);
            }
        }

        let mut out = Vec::with_capacity(posts.len());
        for post in posts {
            let u = user(post.user_id);
            let g = self.guardian;
            // BasicPostSerializer#cooked_hidden
            let cooked_hidden = post.hidden && !g.is_staff();
            let s = self.settings;
            let ctx = self.post_ctx(post).await?;
            let yours = match g.user_id() {
                Some(uid) => post.user_id == Some(uid),
                None => post.user_id.is_none(),
            };
            let can_see_post = g.can_see_post(s, &ctx, viewer.can_see)?;
            let taken = viewer.taken.get(&post.id);
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
            if cooked_hidden {
                let mine = g.user_id().is_some() && g.user_id() == post.user_id;
                let message = if mine {
                    self.i18n
                        .t_with("flagging.you_must_edit", &[("path", "/my/messages")])
                } else {
                    self.i18n.t("flagging.user_must_edit").map(str::to_string)
                };
                p.insert("cooked".into(), json!(message));
                p.insert("cooked_hidden".into(), json!(true));
            } else {
                p.insert("cooked".into(), json!(post.cooked));
            }
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
            p.insert("yours".into(), json!(yours));
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
            p.insert(
                "version".into(),
                // Hidden revisions are for staff (can_view_hidden_post_revisions?).
                json!(if post.hidden && !g.is_staff() {
                    1
                } else if g.is_staff() {
                    post.version
                } else {
                    post.public_version
                }),
            );
            p.insert(
                "can_edit".into(),
                json!(
                    g.is_authenticated()
                        && g.can_edit_post(
                            s,
                            &viewer.ctx,
                            &ctx,
                            viewer.can_see,
                            viewer.can_create_post
                        )?
                ),
            );
            p.insert(
                "can_delete".into(),
                json!(
                    g.is_authenticated()
                        && g.can_delete_post(s, &viewer.ctx, &ctx, can_see_post)?
                ),
            );
            p.insert(
                "can_recover".into(),
                json!(g.can_recover_post(s, &ctx, viewer.can_see)?),
            );
            let can_see_hidden_post = g.can_see_hidden_post(s, &ctx)?;
            p.insert("can_see_hidden_post".into(), json!(can_see_hidden_post));
            p.insert("can_wiki".into(), json!(g.can_wiki(s, &ctx)?));
            let link_counts = if post.hidden && !can_see_hidden_post {
                Vec::new()
            } else {
                self.link_counts(post.id).await?
            };
            if !link_counts.is_empty() {
                p.insert("link_counts".into(), Value::Array(link_counts));
            }
            // read?(post_number): anonymous reads everything; a user only
            // what post_timings recorded, and nothing without a topic_users row.
            let read = match g.user_id() {
                None => true,
                Some(_) => viewer.read_post_numbers.contains(&post.post_number),
            };
            p.insert("read".into(), json!(read));
            p.insert("user_title".into(), json!(u.and_then(|u| u.title.clone())));
            if u.and_then(|u| u.title.as_deref())
                .is_some_and(|t| !t.is_empty())
            {
                return Err(Unsupported("title_is_group").into());
            }
            if let Some(reply_to) = post.reply_to_user_id
                && !(suppress_reply_when_quoting && post.reply_quoted)
                && let Some(ru) = user(Some(reply_to))
            {
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
            let bookmark = viewer
                .bookmarks
                .iter()
                .find(|b| b.bookmarkable_type == "Post" && b.bookmarkable_id == i64::from(post.id));
            p.insert("bookmarked".into(), json!(bookmark.is_some()));
            if let Some(b) = bookmark {
                p.insert(
                    "bookmark_reminder_at".into(),
                    json!(b.reminder_at.map(time_json)),
                );
                p.insert("bookmark_id".into(), json!(b.id));
                p.insert("bookmark_name".into(), json!(b.name));
                p.insert(
                    "bookmark_auto_delete_preference".into(),
                    json!(b.auto_delete_preference),
                );
            }
            let counts: HashMap<i64, i32> = viewer
                .action_types
                .types
                .iter()
                .map(|(key, id)| {
                    let count = match key.as_str() {
                        "like" => post.like_count,
                        "notify_user" => post.notify_user_count,
                        "off_topic" => post.off_topic_count,
                        "inappropriate" => post.inappropriate_count,
                        "spam" => post.spam_count,
                        "illegal" => post.illegal_count,
                        "notify_moderators" => post.notify_moderators_count,
                        _ => 0,
                    };
                    (*id, count)
                })
                .collect();
            let actions = g.actions_summary(
                s,
                &viewer.action_types,
                &counts,
                &ActOpts {
                    topic: &viewer.ctx,
                    post: &ctx,
                    taken,
                    can_see_post,
                    author_missing: u.is_none(),
                },
                post.user_id.is_some_and(|id| id < 0),
            )?;
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
                json!(g.can_view_edit_history(s, &ctx, can_see_post)?),
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
                        && !value.is_empty()
                    {
                        p.insert(key.into(), json!(value));
                    }
                }
            }
            // notice: a new/returning-user notice shows to viewers past the
            // notice trust level; custom notices to everyone.
            if let Some(notice) = self.notice(post.id).await? {
                let show = match notice["type"].as_str() {
                    Some("custom") => {
                        return Err(Unsupported("custom post notices").into());
                    }
                    Some("new_user") => {
                        g.is_authenticated()
                            && !yours
                            && g.has_trust_level(s.get("new_user_notice_tl")?.to_i() as i32)
                    }
                    Some("returning_user") => {
                        g.is_authenticated()
                            && !yours
                            && g.has_trust_level(s.get("returning_user_notice_tl")?.to_i() as i32)
                    }
                    _ => false,
                };
                if show {
                    p.insert("notice".into(), notice);
                }
            }
            if post.locked_by_id.is_some() && (yours || g.is_staff()) {
                p.insert("locked".into(), json!(true));
            }
            if g.can_review_topic(s, viewer.can_see)? {
                let counts: Option<(i64, i64, i64)> = sqlx::query_as(
                    "SELECT MAX(r.id), COUNT(*), SUM(CASE WHEN s.status = 0 THEN 1 ELSE 0 END) \
                     FROM reviewables r JOIN reviewable_scores s ON s.reviewable_id = r.id \
                     WHERE r.target_id = $1 AND r.target_type = 'Post' \
                       AND r.type IN ('ReviewableFlaggedPost', 'ReviewableQueuedPost', 'ReviewableUser', 'ReviewablePost') \
                       AND COALESCE(s.reason, '') <> 'category' GROUP BY r.target_id",
                )
                .bind(post.id)
                .fetch_optional(&mut *self.conn)
                .await?;
                let (id, total, pending) = counts.unwrap_or((0, 0, 0));
                p.insert("reviewable_id".into(), json!(id));
                p.insert("reviewable_score_count".into(), json!(total));
                p.insert("reviewable_score_pending_count".into(), json!(pending));
            }
            if u.is_some_and(|u| u.suspended_till.is_some()) {
                return Err(Unsupported("user_suspended").into());
            }
            if let Some(ids) = mentioned.get(&post.id) {
                let mut users = Vec::with_capacity(ids.len());
                for id in ids {
                    users.push(self.basic_user(*id, logo_small_url.as_deref()).await?);
                }
                p.insert("mentioned_users".into(), Value::Array(users));
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

    /// `details.allowed_users`: the PM's direct members (those only
    /// present through an allowed group are left out, the viewer kept),
    /// in the order Rails' unordered join came back with on the reference
    /// (by user id, descending).
    async fn allowed_users(
        &mut self,
        topic_id: i32,
        logo_small_url: Option<&str>,
    ) -> Result<Value, TopicViewError> {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT tau.user_id FROM topic_allowed_users tau \
             WHERE tau.topic_id = $1 AND (tau.user_id = $2 OR tau.user_id NOT IN ( \
                 SELECT gu.user_id FROM group_users gu \
                 WHERE gu.group_id IN (SELECT group_id FROM topic_allowed_groups WHERE topic_id = $1))) \
             ORDER BY tau.user_id DESC",
        )
        .bind(topic_id)
        .bind(self.guardian.user_id().unwrap_or(0))
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.basic_user(id, logo_small_url).await?);
        }
        Ok(Value::Array(out))
    }

    /// `details.allowed_groups`: BasicGroupSerializer for the viewer.
    async fn allowed_groups(&mut self, topic_id: i32) -> Result<Value, TopicViewError> {
        let groups: Vec<crate::groups::BasicGroup> = sqlx::query_as(&format!(
            "SELECT {} FROM topic_allowed_groups tag JOIN groups g ON g.id = tag.group_id \
             LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE tag.topic_id = $1 ORDER BY tag.id",
            crate::groups::BASIC_GROUP_COLUMNS
        ))
        .bind(topic_id)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out = Vec::new();
        for g in &groups {
            let membership: Option<bool> = match self.guardian.user_id() {
                Some(uid) => {
                    sqlx::query_scalar(
                        "SELECT owner FROM group_users WHERE group_id = $1 AND user_id = $2",
                    )
                    .bind(g.id)
                    .bind(uid)
                    .fetch_optional(&mut *self.conn)
                    .await?
                }
                None => None,
            };
            out.push(g.json(
                self.i18n,
                self.guardian,
                self.settings,
                membership.map(|o| (true, o)),
            )?);
        }
        Ok(Value::Array(out))
    }

    /// `Topic#message_archived?(user)`: archived by every group of the
    /// viewer's the PM is addressed to, or by the viewer.
    async fn message_archived(&mut self, topic_id: i32) -> Result<bool, TopicViewError> {
        let Some(uid) = self.guardian.user_id() else {
            return Ok(false);
        };
        let rows: Vec<i32> = sqlx::query_scalar(
            "SELECT 1 WHERE (SELECT count(*) FROM topic_allowed_groups tg \
                 JOIN group_archived_messages gm ON gm.topic_id = tg.topic_id AND gm.group_id = tg.group_id \
                 WHERE tg.group_id IN (SELECT g.group_id FROM group_users g WHERE g.user_id = $1) AND tg.topic_id = $2) \
             = (SELECT CASE WHEN count(*) = 0 THEN -1 ELSE count(*) END FROM topic_allowed_groups tg \
                 WHERE tg.group_id IN (SELECT g.group_id FROM group_users g WHERE g.user_id = $1) AND tg.topic_id = $2) \
             UNION ALL \
             SELECT 1 FROM topic_allowed_users tu JOIN user_archived_messages um \
                 ON um.user_id = tu.user_id AND um.topic_id = tu.topic_id \
             WHERE tu.user_id = $1 AND tu.topic_id = $2",
        )
        .bind(uid)
        .bind(topic_id)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(!rows.is_empty())
    }

    /// `Topic#pm_with_non_human_user?`: a group-less PM with exactly one
    /// human allowed user.
    async fn pm_with_non_human_user(&mut self, topic_id: i32) -> Result<bool, TopicViewError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics LEFT JOIN topic_allowed_groups ON topics.id = topic_allowed_groups.topic_id \
             WHERE topic_allowed_groups.topic_id IS NULL AND topics.archetype = 'private_message' AND topics.id = $1 \
             AND (SELECT COUNT(*) FROM topic_allowed_users WHERE topic_allowed_users.topic_id = $1 AND topic_allowed_users.user_id > 0) = 1)",
        )
        .bind(topic_id)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// `suggested_group_name`: null for direct members, else the name of
    /// one of the viewer's groups the PM is addressed to.
    async fn suggested_group_name(&mut self, topic: &TopicRow) -> Result<Value, TopicViewError> {
        let uid = self.guardian.user_id().unwrap_or(0);
        let direct: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_allowed_users WHERE topic_id = $1 AND user_id = $2)",
        )
        .bind(topic.id)
        .bind(uid)
        .fetch_one(&mut *self.conn)
        .await?;
        if direct {
            return Ok(Value::Null);
        }
        let name: Option<String> = sqlx::query_scalar(
            "SELECT groups.name FROM groups JOIN group_users ON group_users.group_id = groups.id \
             WHERE group_users.group_id IN (SELECT group_id FROM topic_allowed_groups WHERE topic_id = $1) \
             AND group_users.user_id = $2 LIMIT 1",
        )
        .bind(topic.id)
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(json!(name))
    }

    /// `TopicQuery#get_pm_params`: the viewer's groups among the PM's,
    /// every addressed group, and the other addressed users.
    async fn pm_params(
        &mut self,
        topic_id: i32,
    ) -> Result<(Vec<i32>, Vec<i32>, Vec<i32>), TopicViewError> {
        let uid = self.guardian.user_id().unwrap_or(0);
        let my_groups: Vec<i32> = sqlx::query_scalar(
            "SELECT tag.group_id FROM topic_allowed_groups tag \
             LEFT JOIN group_users gu ON tag.group_id = gu.group_id AND gu.user_id = $2 \
             WHERE tag.topic_id = $1 AND gu.group_id IS NOT NULL",
        )
        .bind(topic_id)
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        let target_groups: Vec<i32> =
            sqlx::query_scalar("SELECT group_id FROM topic_allowed_groups WHERE topic_id = $1")
                .bind(topic_id)
                .fetch_all(&mut *self.conn)
                .await?;
        if !my_groups.is_empty() {
            return Err(Unsupported("group private messages (messages_for_groups_or_user)").into());
        }
        let target_users: Vec<i32> = sqlx::query_scalar(
            "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 AND NOT user_id = $2",
        )
        .bind(topic_id)
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok((my_groups, target_groups, target_users))
    }

    /// `list_related_for`: the viewer's other PMs with the same people,
    /// newest first, as suggested topics.
    async fn related_messages(&mut self, topic: &TopicRow) -> Result<Value, TopicViewError> {
        let (_, target_groups, target_users) = self.pm_params(topic.id).await?;
        let uid = self.guardian.user_id().unwrap_or(0);
        if !target_groups.is_empty() {
            return Err(Unsupported("related messages through allowed groups").into());
        }
        let count = self.settings.get("suggested_topics")?.to_i().max(6);
        let sql = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics \
             LEFT JOIN topic_users tu ON topics.id = tu.topic_id AND tu.user_id = $1 \
             LEFT JOIN topic_allowed_users ta ON topics.id = ta.topic_id AND ta.user_id = $1 \
             LEFT JOIN topic_allowed_users ta2 ON topics.id = ta2.topic_id AND ta2.user_id = ANY($2) \
             WHERE topics.deleted_at IS NULL AND topics.archetype = 'private_message' \
             AND ta.topic_id IS NOT NULL AND ta2.topic_id IS NOT NULL AND topics.id <> $3 AND topics.visible = TRUE \
             AND topics.id NOT IN (SELECT topic_id FROM categories WHERE topic_id IS NOT NULL) \
             ORDER BY topics.bumped_at DESC LIMIT $4"
        );
        let rows: Vec<TopicRow> = sqlx::query_as(&sql)
            .bind(uid)
            .bind(&target_users)
            .bind(topic.id)
            .bind(count)
            .fetch_all(&mut *self.conn)
            .await?;
        // Each row once, in order (the ta2 join repeats per shared member).
        let mut seen = Vec::new();
        let rows: Vec<TopicRow> = rows
            .into_iter()
            .filter(|t| {
                if seen.contains(&t.id) {
                    false
                } else {
                    seen.push(t.id);
                    true
                }
            })
            .collect();
        self.suggested_items(&rows).await
    }

    /// `list_suggested_for` on a PM: new messages, then unread ones, up
    /// to suggested_topics; no random fill.
    async fn suggested_messages(&mut self, topic: &TopicRow) -> Result<Value, TopicViewError> {
        let (_, _, _) = self.pm_params(topic.id).await?;
        let uid = self.guardian.user_id().unwrap_or(0);
        let limit = self.settings.get("suggested_topics")?.to_i();
        let min_new =
            chrono::DateTime::from_timestamp(self.settings.get("min_new_topics_time")?.to_i(), 0)
                .map(|t| t.naive_utc())
                .unwrap_or_default();
        let base = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics \
             LEFT JOIN topic_users tu ON topics.id = tu.topic_id AND tu.user_id = $1 \
             LEFT JOIN topic_allowed_users ta ON topics.id = ta.topic_id AND ta.user_id = $1 \
             WHERE topics.deleted_at IS NULL AND topics.archetype = 'private_message' AND ta.topic_id IS NOT NULL"
        );
        let mut rows: Vec<TopicRow> = sqlx::query_as(&format!(
            "{base} AND topics.created_at >= $2 AND tu.last_read_post_number IS NULL \
             AND COALESCE(tu.notification_level, 2) >= 2 AND topics.id <> $3 AND topics.visible = TRUE \
             ORDER BY topics.bumped_at DESC LIMIT $4"
        ))
        .bind(uid)
        .bind(min_new)
        .bind(topic.id)
        .bind(limit)
        .fetch_all(&mut *self.conn)
        .await?;
        let left = limit - rows.len() as i64;
        if left > 0 {
            let first_unread: Option<Option<NaiveDateTime>> =
                sqlx::query_scalar("SELECT first_unread_pm_at FROM user_stats WHERE user_id = $1")
                    .bind(uid)
                    .fetch_optional(&mut *self.conn)
                    .await?;
            let mut excluded: Vec<i32> = rows.iter().map(|t| t.id).collect();
            excluded.push(topic.id);
            let age = match first_unread.flatten() {
                Some(at) => format!(
                    " AND topics.updated_at >= '{}'",
                    at.format("%Y-%m-%d %H:%M:%S%.6f")
                ),
                None => String::new(),
            };
            let unread: Vec<TopicRow> = sqlx::query_as(&format!(
                "{base} AND tu.last_read_post_number < topics.highest_post_number \
                 AND COALESCE(tu.notification_level, 1) >= 2{age} AND NOT (topics.id = ANY($2)) \
                 AND topics.visible = TRUE ORDER BY topics.bumped_at DESC LIMIT $3"
            ))
            .bind(uid)
            .bind(&excluded)
            .bind(left)
            .fetch_all(&mut *self.conn)
            .await?;
            rows.extend(unread);
        }
        self.suggested_items(&rows).await
    }

    /// SuggestedTopicSerializer for a set of rows.
    async fn suggested_items(&mut self, rows: &[TopicRow]) -> Result<Value, TopicViewError> {
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        let mut serializer = self.list_serializer();
        serializer.prefetch(rows).await?;
        let lookup = serializer.user_lookup(rows).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let posters = topic_list::posters_summary(row, &lookup, serializer.i18n);
            out.push(
                serializer
                    .serialize_topic(row, &posters, tagging, Mode::Suggested)
                    .await?,
            );
        }
        Ok(Value::Array(out))
    }
    /// `list_suggested_for` on a regular topic through SuggestedTopicsBuilder:
    /// for a user, unread topics (spliced in at high priority) then new
    /// ones, then the random fill, up to `suggested_topics`.
    async fn suggested_topics(&mut self, topic: &TopicRow) -> Result<Value, TopicViewError> {
        let limit = self.settings.get("suggested_topics")?.to_i();
        if self.settings.get("limit_suggested_to_category")?.truthy() {
            return Err(Unsupported("limit_suggested_to_category").into());
        }
        // Category.topic_ids: the definition topics, never suggested.
        let definitions: Vec<i32> =
            sqlx::query_scalar("SELECT topic_id FROM categories WHERE topic_id IS NOT NULL")
                .fetch_all(&mut *self.conn)
                .await?;
        let mut builder = Suggested {
            results: Vec::new(),
            excluded: vec![topic.id],
            category_id: topic.category_id,
            definitions,
        };
        if self.guardian.is_authenticated() {
            let max_age = self
                .settings
                .get("suggested_topics_unread_max_days_old")?
                .to_i();
            let mut query = TopicQuery {
                conn: &mut *self.conn,
                settings: self.settings,
                guardian: self.guardian,
                options: Default::default(),
                filter: Default::default(),
                category: Default::default(),
                tags: Default::default(),
                user: Default::default(),
            };
            let unified_new = self
                .guardian
                .upcoming_change_enabled(&mut *query.conn, self.settings, "enable_unified_new")
                .await?;
            if unified_new {
                // new_and_unread_results, added at the default priority.
                let rows = query
                    .suggested(
                        Filter::New,
                        topic.category_id,
                        &builder.excluded,
                        max_age,
                        limit - builder.results.len() as i64,
                    )
                    .await?;
                builder.add(rows, false);
            } else {
                let unread = query
                    .suggested(
                        Filter::Unread,
                        topic.category_id,
                        &builder.excluded,
                        max_age,
                        limit - builder.results.len() as i64,
                    )
                    .await?;
                builder.add(unread, true);
            }
            if !unified_new && (builder.results.len() as i64) < limit {
                let same_category = builder
                    .results
                    .iter()
                    .filter(|t| t.category_id == topic.category_id)
                    .count() as i64;
                let new = query
                    .suggested(
                        Filter::New,
                        topic.category_id,
                        &builder.excluded,
                        max_age,
                        limit - same_category,
                    )
                    .await?;
                builder.add(new, false);
            }
        }
        let left = limit - builder.results.len() as i64;
        if left > 0 {
            let random = self
                .random_suggested(topic, &builder.excluded, left)
                .await?;
            builder.add(random, false);
        }
        builder.results.truncate(limit.max(0) as usize);
        self.suggested_items(&builder.results).await
    }

    /// A deterministic stand-in for `random_suggested`: the same filters
    /// (open, unarchived, visible, readable, not a definition, not one
    /// already suggested), same-category first, then by bumped_at. Never
    /// matches Rails' random pick, and does not remove the user's muted
    /// topics as Rails does.
    async fn random_suggested(
        &mut self,
        topic: &TopicRow,
        excluded: &[i32],
        count: i64,
    ) -> Result<Vec<TopicRow>, TopicViewError> {
        let sql = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics LEFT OUTER JOIN categories ON categories.id = topics.category_id \
             WHERE topics.deleted_at IS NULL AND topics.visible AND NOT topics.closed AND NOT topics.archived \
               AND topics.archetype <> 'private_message' AND NOT (topics.id = ANY($1)) \
               AND (categories.id IS NULL OR NOT categories.read_restricted) \
               AND COALESCE(categories.topic_id, 0) <> topics.id \
             ORDER BY CASE WHEN topics.category_id = $2 THEN 0 ELSE 1 END, topics.bumped_at DESC LIMIT $3"
        );
        Ok(sqlx::query_as(&sql)
            .bind(excluded)
            .bind(topic.category_id)
            .bind(count)
            .fetch_all(&mut *self.conn)
            .await?)
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
    /// visible and in a category the viewer may read (`secure_category`),
    /// not muted by a logged-in viewer, external links always, `ORDER BY
    /// reflection, clicks DESC`.
    async fn link_counts(&mut self, post_id: i32) -> Result<Vec<Value>, TopicViewError> {
        let secure = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
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
             LEFT JOIN topic_users tu ON t.id = tu.topic_id AND tu.user_id = $3 \
             WHERE l.post_id = $1 \
               AND t.deleted_at IS NULL \
               AND (t.id IS NULL OR t.visible = true) \
               AND (l.internal = false OR t.id IS NOT NULL) \
               AND (l.link_post_id IS NULL OR (target_posts.id IS NOT NULL AND target_posts.deleted_at IS NULL)) \
               AND COALESCE(t.archetype, 'regular') <> 'private_message' \
               AND ($3::int IS NULL OR COALESCE(tu.notification_level, 1) > 0) \
               AND (NOT COALESCE(c.read_restricted, false) OR c.id = ANY($2)) \
             ORDER BY l.reflection ASC, l.clicks DESC",
        )
        .bind(post_id)
        .bind(&secure)
        .bind(self.guardian.user_id())
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

/// SuggestedTopicsBuilder: the results so far and what they exclude.
struct Suggested {
    results: Vec<TopicRow>,
    excluded: Vec<i32>,
    category_id: Option<i32>,
    /// Category.topic_ids
    definitions: Vec<i32>,
}

impl Suggested {
    /// `add_results`: definition topics and repeats out; at high priority
    /// the topic's category goes in before the first other-category
    /// result, the rest at the end.
    fn add(&mut self, rows: Vec<TopicRow>, high: bool) {
        let mut fresh = Vec::new();
        for row in rows {
            if self.definitions.contains(&row.id) || self.excluded.contains(&row.id) {
                continue;
            }
            self.excluded.push(row.id);
            fresh.push(row);
        }
        match self.category_id {
            Some(category_id) if high => {
                let (same, other): (Vec<TopicRow>, Vec<TopicRow>) = fresh
                    .into_iter()
                    .partition(|r| r.category_id == Some(category_id));
                let at = self
                    .results
                    .iter()
                    .position(|r| r.category_id != Some(category_id))
                    .unwrap_or(self.results.len());
                self.results.splice(at..at, same);
                self.results.extend(other);
            }
            _ => self.results.extend(fresh),
        }
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
