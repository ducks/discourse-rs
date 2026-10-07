//! Port of lib/bookmark_query.rb, app/models/user_bookmark_list.rb, the
//! list queries of app/services/post_bookmarkable.rb and
//! topic_bookmarkable.rb, and UserBookmarkListSerializer with
//! UserPostBookmarkSerializer and UserTopicBookmarkSerializer.
//!
//! Only core's two bookmarkables are ported. A user holding a bookmark of
//! a plugin-registered type (chat messages) is refused rather than shown a
//! list without it.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::avatar::{AvatarError, avatar_template};
use crate::guardian::{Guardian, GuardianError};
use crate::i18n::I18n;
use crate::notifications::{Notifications, NotificationsError};
use crate::search::{ts_config, ts_query_sql, ts_query_value};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::topic_list::{TopicListError, TopicListSerializer, time_json};
use crate::url::{UrlError, Urls};

/// `UserBookmarkList::PER_PAGE`
pub const PER_PAGE: i64 = 20;
/// `UsersController::USER_MENU_LIST_LIMIT`
pub const USER_MENU_LIST_LIMIT: i64 = 20;
/// `Notification.types[:bookmark_reminder]`
const BOOKMARK_REMINDER: i32 = 24;

#[derive(Debug)]
pub enum BookmarksError {
    Db(sqlx::Error),
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for BookmarksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BookmarksError::Db(e) => write!(f, "loading bookmarks: {e}"),
            BookmarksError::Setting(e) => e.fmt(f),
            BookmarksError::Url(e) => e.fmt(f),
            BookmarksError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for BookmarksError {}

impl From<sqlx::Error> for BookmarksError {
    fn from(e: sqlx::Error) -> Self {
        BookmarksError::Db(e)
    }
}

impl From<SettingError> for BookmarksError {
    fn from(e: SettingError) -> Self {
        BookmarksError::Setting(e)
    }
}

impl From<UrlError> for BookmarksError {
    fn from(e: UrlError) -> Self {
        BookmarksError::Url(e)
    }
}

impl From<Unsupported> for BookmarksError {
    fn from(e: Unsupported) -> Self {
        BookmarksError::Unsupported(e)
    }
}

impl From<GuardianError> for BookmarksError {
    fn from(e: GuardianError) -> Self {
        match e {
            GuardianError::Db(e) => BookmarksError::Db(e),
            GuardianError::Setting(e) => BookmarksError::Setting(e),
            GuardianError::Unsupported(e) => BookmarksError::Unsupported(e),
        }
    }
}

impl From<TopicListError> for BookmarksError {
    fn from(e: TopicListError) -> Self {
        match e {
            TopicListError::Db(e) => BookmarksError::Db(e),
            TopicListError::Setting(e) => BookmarksError::Setting(e),
            TopicListError::Url(e) => BookmarksError::Url(e),
            TopicListError::Unsupported(e) => BookmarksError::Unsupported(e),
        }
    }
}

impl From<AvatarError> for BookmarksError {
    fn from(e: AvatarError) -> Self {
        match e {
            AvatarError::Setting(e) => BookmarksError::Setting(e),
            AvatarError::Url(e) => BookmarksError::Url(e),
            AvatarError::Unsupported(e) => BookmarksError::Unsupported(e),
        }
    }
}

impl From<NotificationsError> for BookmarksError {
    fn from(e: NotificationsError) -> Self {
        match e {
            NotificationsError::Db(e) => BookmarksError::Db(e),
            NotificationsError::Setting(e) => BookmarksError::Setting(e),
            NotificationsError::Unsupported(e) => BookmarksError::Unsupported(e),
        }
    }
}

/// A row of `bookmarks`.
#[derive(Debug, Clone, sqlx::FromRow)]
struct BookmarkRow {
    id: i64,
    created_at: NaiveDateTime,
    updated_at: NaiveDateTime,
    name: Option<String>,
    reminder_at: Option<NaiveDateTime>,
    pinned: Option<bool>,
    bookmarkable_id: Option<i64>,
    bookmarkable_type: Option<String>,
}

/// The preloaded bookmarkable: the post (a topic bookmark's first post),
/// its topic and its author.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Bookmarkable {
    bookmark_id: i64,
    topic_id: i32,
    title: String,
    fancy_title: Option<String>,
    slug: Option<String>,
    category_id: Option<i32>,
    closed: bool,
    archived: bool,
    archetype: String,
    highest_post_number: i32,
    highest_staff_post_number: i32,
    bumped_at: NaiveDateTime,
    topic_deleted_at: Option<NaiveDateTime>,
    post_id: i32,
    post_user_id: Option<i32>,
    post_number: i32,
    post_type: i32,
    hidden: bool,
    hidden_at: Option<NaiveDateTime>,
    locked_by_id: Option<i32>,
    post_deleted_at: Option<NaiveDateTime>,
    user_deleted: bool,
    wiki: bool,
    post_created_at: NaiveDateTime,
    cooked: String,
    author_staff: bool,
    username: Option<String>,
    user_name: Option<String>,
    uploaded_avatar_id: Option<i32>,
    /// The viewer's `topic_users.last_read_post_number`.
    last_read_post_number: Option<i32>,
}

impl Bookmarkable {
    fn post_ctx(&self) -> PostCtx {
        PostCtx {
            id: self.post_id,
            user_id: self.post_user_id,
            post_number: self.post_number,
            post_type: self.post_type,
            hidden: self.hidden,
            hidden_at: self.hidden_at,
            locked_by_id: self.locked_by_id,
            deleted_at: self.post_deleted_at,
            user_deleted: self.user_deleted,
            wiki: self.wiki,
            created_at: self.post_created_at,
            author_staff: self.author_staff,
        }
    }
}

/// The columns of `Bookmarkable` after `bookmark_id`; `$2` is the viewer.
const BOOKMARKABLE_COLUMNS: &str = "t.id AS topic_id, t.title, t.fancy_title, t.slug, t.category_id, t.closed, \
        t.archived, t.archetype, t.highest_post_number, t.highest_staff_post_number, t.bumped_at, \
        t.deleted_at AS topic_deleted_at, \
        p.id AS post_id, p.user_id AS post_user_id, p.post_number, p.post_type, p.hidden, p.hidden_at, \
        p.locked_by_id, p.deleted_at AS post_deleted_at, p.user_deleted, p.wiki, p.created_at AS post_created_at, \
        p.cooked, COALESCE(u.admin OR u.moderator, FALSE) AS author_staff, u.username, u.name AS user_name, \
        u.uploaded_avatar_id, tu.last_read_post_number";
const BOOKMARKABLE_JOINS: &str = "JOIN topics t ON t.id = p.topic_id \
     LEFT JOIN users u ON u.id = p.user_id \
     LEFT JOIN topic_users tu ON tu.topic_id = t.id AND tu.user_id = $2";

/// What `UserBookmarkList.new` and its `load` block take.
#[derive(Debug, Clone, Default)]
pub struct ListQuery<'q> {
    /// `params[:q]`
    pub search_term: Option<&'q str>,
    pub page: i64,
    /// `per_page`, already capped by the caller's limit check.
    pub per_page: Option<i64>,
    /// The `where.not(id:)` of users#user_menu_bookmarks.
    pub exclude_ids: &'q [i64],
}

/// A loaded `UserBookmarkList`.
pub struct BookmarkList {
    rows: Vec<BookmarkRow>,
    pub has_more: bool,
}

impl BookmarkList {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

pub struct Bookmarks<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    /// The viewer: the list's owner, or an admin reading someone's list.
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
}

impl Bookmarks<'_> {
    /// `BookmarkQuery#count_all`: how many of `user_id`'s bookmarks the
    /// guardian may see.
    pub async fn count_all(&mut self, user_id: i32) -> Result<i64, BookmarksError> {
        self.refuse_plugin_types(user_id).await?;
        let union = self.list_queries(user_id, false).await?;
        Ok(
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM ({union}) AS bookmarks"))
                .bind(i64::from(user_id))
                .bind("")
                .bind("")
                .fetch_one(&mut *self.conn)
                .await?,
        )
    }

    /// Bookmarks of plugin-registered types (chat messages) are not ported.
    async fn refuse_plugin_types(&mut self, user_id: i32) -> Result<(), BookmarksError> {
        let plugin_types: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM bookmarks WHERE user_id = $1 \
             AND bookmarkable_type NOT IN ('Post', 'Topic'))",
        )
        .bind(i64::from(user_id))
        .fetch_one(&mut *self.conn)
        .await?;
        if plugin_types {
            return Err(Unsupported("bookmarks of plugin-registered types (chat messages)").into());
        }
        Ok(())
    }

    /// `UserBookmarkList#load`: `BookmarkQuery#list_all` for `user_id`'s
    /// bookmarks as the guardian may see them.
    pub async fn load(
        &mut self,
        user_id: i32,
        q: &ListQuery<'_>,
    ) -> Result<BookmarkList, BookmarksError> {
        let per_page = q.per_page.unwrap_or(PER_PAGE).min(PER_PAGE);
        self.refuse_plugin_types(user_id).await?;

        let search = q.search_term.filter(|t| !crate::ruby::is_blank(t));
        let union = self.list_queries(user_id, search.is_some()).await?;
        let (ts_value, pattern) = match search {
            Some(term) => (ts_query_value(term, "", true), format!("%{term}%")),
            None => (String::new(), String::new()),
        };

        let count: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM ({union}) AS bookmarks"))
                .bind(i64::from(user_id))
                .bind(&ts_value)
                .bind(&pattern)
                .fetch_one(&mut *self.conn)
                .await?;
        let offset = if q.page > 0 { q.page * per_page } else { 0 };
        let rows: Vec<BookmarkRow> = sqlx::query_as(&format!(
            "SELECT bookmarks.id, bookmarks.created_at, bookmarks.updated_at, bookmarks.name, \
                    bookmarks.reminder_at, bookmarks.pinned, bookmarks.bookmarkable_id, bookmarks.bookmarkable_type \
             FROM ({union}) AS bookmarks WHERE bookmarks.id <> ALL($4) \
             ORDER BY (CASE WHEN bookmarks.pinned THEN 0 ELSE 1 END), bookmarks.reminder_at ASC, \
                      bookmarks.updated_at DESC \
             LIMIT $5 OFFSET $6"
        ))
        .bind(i64::from(user_id))
        .bind(&ts_value)
        .bind(&pattern)
        .bind(q.exclude_ids)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(BookmarkList {
            rows,
            has_more: (q.page + 1) * per_page < count,
        })
    }

    /// `build_list_queries`: PostBookmarkable.list_query and
    /// TopicBookmarkable.list_query, joined by UNION. `$1` is the owner;
    /// with `search`, `$2` is the tsquery literal and `$3` the ILIKE
    /// pattern (the placeholders are referenced either way, so the binds
    /// keep their types).
    async fn list_queries(&mut self, user_id: i32, search: bool) -> Result<String, BookmarksError> {
        let settings = self.settings;
        let guardian = self.guardian;
        // The categories are the viewer's, the messages the owner's.
        let topics = guardian
            .listable_or_own_messages(&mut *self.conn, settings, user_id)
            .await?;
        // Post.secured(guardian)
        let post_types = guardian
            .visible_post_types(settings)?
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let posts = format!("posts.deleted_at IS NULL AND posts.post_type IN ({post_types})");
        // filter_hidden_posts, which only the topic bookmarks go through.
        let hidden = if guardian.can_see_all_hidden_posts(settings)? {
            String::new()
        } else {
            let viewer = guardian.user_id().unwrap_or(0);
            format!(" AND (posts.hidden = false OR posts.user_id = {viewer})")
        };
        // filter_allowed_categories
        let admin_sees_all = guardian.is_admin()
            && !settings
                .get("suppress_secured_categories_from_admin")?
                .truthy();
        let allowed = if admin_sees_all {
            String::new()
        } else {
            let ids = guardian
                .allowed_category_ids(&mut *self.conn, settings)
                .await?;
            format!(
                " AND {}",
                guardian.category_clause(&ids, "topics.category_id")
            )
        };
        let (post_search_join, topic_search_join, search_where) = if search {
            let config = ts_config(&settings.get("default_locale")?.to_s());
            (
                " LEFT JOIN post_search_data ON post_search_data.post_id = bookmarks.bookmarkable_id \
                  AND bookmarks.bookmarkable_type = 'Post'",
                " LEFT JOIN post_search_data ON post_search_data.post_id = posts.id",
                format!(
                    " AND ({} @@ post_search_data.search_data OR bookmarks.name ILIKE $3)",
                    ts_query_sql(config, 2)
                ),
            )
        } else {
            (
                "",
                "",
                " AND $2::text IS NOT NULL AND $3::text IS NOT NULL".to_string(),
            )
        };
        Ok(format!(
            "SELECT bookmarks.* FROM bookmarks \
               INNER JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
               LEFT JOIN topics ON topics.id = posts.topic_id \
               LEFT JOIN topic_users ON topic_users.topic_id = topics.id{post_search_join} \
             WHERE bookmarks.user_id = $1 AND bookmarks.bookmarkable_type = 'Post' \
               AND topic_users.user_id = {user_id} AND {topics} AND {posts}{allowed}{search_where} \
             UNION \
             SELECT bookmarks.* FROM bookmarks \
               INNER JOIN topics ON topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic' \
               INNER JOIN posts ON posts.topic_id = topics.id AND posts.post_number = 1 \
               LEFT JOIN topic_users ON topic_users.topic_id = topics.id{topic_search_join} \
             WHERE bookmarks.user_id = $1 AND bookmarks.bookmarkable_type = 'Topic' \
               AND topic_users.user_id = {user_id} AND {topics} AND {posts}{hidden}{allowed}{search_where}"
        ))
    }

    /// The bookmarks array of UserBookmarkListSerializer: each row through
    /// its bookmarkable's serializer. `link_to_first_unread_post` is the
    /// user menu's option.
    pub async fn serialize(
        &mut self,
        list: &BookmarkList,
        link_to_first_unread_post: bool,
    ) -> Result<Vec<Value>, BookmarksError> {
        if self.guardian.can_lazy_load_categories(self.settings)? {
            return Err(
                Unsupported("categories on the bookmark list (lazy_load_categories)").into(),
            );
        }
        if self.settings.get("content_localization_enabled")?.truthy() {
            return Err(Unsupported("content localization on bookmarks").into());
        }
        let bookmarkables = self
            .bookmarkables(&list.rows.iter().map(|r| r.id).collect::<Vec<_>>())
            .await?;
        let secure = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        let logo_small_url = self.list_serializer().logo_small_url().await?;
        let mut out = Vec::with_capacity(list.rows.len());
        for row in &list.rows {
            let Some(b) = bookmarkables.get(&row.id) else {
                // The list query only returns bookmarks whose post and
                // topic exist and are not deleted.
                continue;
            };
            out.push(
                self.item(
                    row,
                    b,
                    &secure,
                    logo_small_url.as_deref(),
                    link_to_first_unread_post,
                )
                .await?,
            );
        }
        Ok(out)
    }

    /// `BookmarkQuery.preload`: the bookmarkables by bookmark id.
    async fn bookmarkables(
        &mut self,
        ids: &[i64],
    ) -> Result<HashMap<i64, Bookmarkable>, BookmarksError> {
        // Post and Topic carry Trashable's default scope: a deleted post
        // is no bookmarkable, nor is a deleted topic for a topic bookmark
        // (a post bookmark still reaches its post in a deleted topic).
        let rows: Vec<Bookmarkable> = sqlx::query_as(&format!(
            "SELECT b.id AS bookmark_id, {BOOKMARKABLE_COLUMNS} FROM bookmarks b \
             JOIN posts p ON (b.bookmarkable_type = 'Post' AND p.id = b.bookmarkable_id) \
                OR (b.bookmarkable_type = 'Topic' AND p.topic_id = b.bookmarkable_id AND p.post_number = 1) \
             {BOOKMARKABLE_JOINS} \
             WHERE b.id = ANY($1) AND p.deleted_at IS NULL \
               AND (b.bookmarkable_type = 'Post' OR t.deleted_at IS NULL)"
        ))
        .bind(ids)
        .bind(self.guardian.user_id().unwrap_or(0))
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(rows.into_iter().map(|b| (b.bookmark_id, b)).collect())
    }

    /// The bookmarkable a reminder names once its bookmark is gone: the
    /// post, or the topic's first post.
    async fn bookmarkable_of(
        &mut self,
        topic: bool,
        id: i64,
    ) -> Result<Option<Bookmarkable>, BookmarksError> {
        let subject = if topic {
            "p.topic_id = $1 AND p.post_number = 1 AND t.deleted_at IS NULL"
        } else {
            "p.id = $1"
        };
        Ok(sqlx::query_as(&format!(
            "SELECT 0::bigint AS bookmark_id, {BOOKMARKABLE_COLUMNS} FROM posts p {BOOKMARKABLE_JOINS} \
             WHERE {subject} AND p.deleted_at IS NULL"
        ))
        .bind(id)
        .bind(self.guardian.user_id().unwrap_or(0))
        .fetch_optional(&mut *self.conn)
        .await?)
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

    /// `guardian.can_see_post?` for a bookmarkable's post.
    async fn can_see_post(
        &mut self,
        b: &Bookmarkable,
        secure: &[i32],
    ) -> Result<bool, BookmarksError> {
        let Some(topic) =
            TopicCtx::load(&mut *self.conn, self.settings, self.guardian, b.topic_id).await?
        else {
            return Ok(false);
        };
        let topic_can_see = self
            .guardian
            .can_see_topic(self.settings, &topic, true, secure)?;
        Ok(self
            .guardian
            .can_see_post(self.settings, &b.post_ctx(), topic_can_see)?)
    }

    /// UserPostBookmarkSerializer or UserTopicBookmarkSerializer.
    async fn item(
        &mut self,
        row: &BookmarkRow,
        b: &Bookmarkable,
        secure: &[i32],
        logo_small_url: Option<&str>,
        link_to_first_unread_post: bool,
    ) -> Result<Value, BookmarksError> {
        let settings = self.settings;
        let is_topic = row.bookmarkable_type.as_deref() == Some("Topic");
        let slug = match b.slug.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => return Err(Unsupported("topics without a stored slug (Slug.for)").into()),
        };
        let mut out = Map::new();
        out.insert("id".into(), json!(row.id));
        out.insert("created_at".into(), json!(time_json(row.created_at)));
        out.insert("updated_at".into(), json!(time_json(row.updated_at)));
        out.insert("name".into(), json!(row.name));
        out.insert("reminder_at".into(), json!(row.reminder_at.map(time_json)));
        if let Some(at) = row.reminder_at {
            // Bookmark#reminder_at_ics, an hour apart.
            let format = self
                .i18n
                .t("datetime_formats.formats.calendar_ics")
                .unwrap_or("%Y%m%dT%H%M%SZ");
            out.insert(
                "reminder_at_ics_start".into(),
                json!(at.format(format).to_string()),
            );
            out.insert(
                "reminder_at_ics_end".into(),
                json!((at + chrono::Duration::hours(1)).format(format).to_string()),
            );
        }
        out.insert("pinned".into(), json!(row.pinned));
        out.insert("title".into(), json!(b.title));
        // LocalizedFancyTopicTitleMixin: only a stored fancy_title is
        // served; computing one is HtmlPrettify.
        match b.fancy_title.as_deref() {
            Some(f) if !f.is_empty() => out.insert("fancy_title".into(), json!(f)),
            _ => {
                return Err(
                    Unsupported("computing fancy_title (HtmlPrettify + emoji unescape)").into(),
                );
            }
        };
        // PostItemExcerpt
        let can_see_post = self.can_see_post(b, secure).await?;
        if can_see_post {
            out.insert(
                "excerpt".into(),
                json!(crate::excerpt::excerpt(
                    &b.cooked,
                    300,
                    &crate::excerpt::Options {
                        keep_emoji_images: true,
                        ..Default::default()
                    },
                )),
            );
        }
        out.insert("bookmarkable_id".into(), json!(row.bookmarkable_id));
        out.insert("bookmarkable_type".into(), json!(row.bookmarkable_type));
        let base_url = self.urls.base_url()?;
        let url = if is_topic {
            // Topic.url(id, slug, post_number): the number only past 1.
            let next = b.last_read_post_number.unwrap_or(0) + 1;
            if link_to_first_unread_post && next > 1 {
                format!("{base_url}/t/{slug}/{}/{next}", b.topic_id)
            } else {
                format!("{base_url}/t/{slug}/{}", b.topic_id)
            }
        } else {
            // Post#full_url
            format!("{base_url}/t/{slug}/{}/{}", b.topic_id, b.post_number)
        };
        out.insert("bookmarkable_url".into(), json!(url));
        // TopicTagsMixin, under can_see_tags?(topic).
        if settings.get("tagging_enabled")?.truthy()
            && (b.archetype != "private_message" || self.guardian.can_tag_pms(settings)?)
        {
            let (tags, descriptions) = self.list_serializer().tags(b.topic_id).await?;
            out.insert("tags".into(), tags);
            out.insert("tags_descriptions".into(), descriptions);
        }
        if can_see_post && b.cooked.chars().count() > 300 {
            out.insert("truncated".into(), json!(true));
        }
        out.insert("topic_id".into(), json!(b.topic_id));
        out.insert(
            "linked_post_number".into(),
            json!(if is_topic { 1 } else { b.post_number }),
        );
        out.insert(
            "deleted".into(),
            json!(b.topic_deleted_at.is_some() || b.post_deleted_at.is_some()),
        );
        out.insert("hidden".into(), json!(b.hidden));
        out.insert("category_id".into(), json!(b.category_id));
        out.insert("closed".into(), json!(b.closed));
        out.insert("archived".into(), json!(b.archived));
        out.insert("archetype".into(), json!(b.archetype));
        let highest = if self.guardian.is_whisperer(settings)? {
            b.highest_staff_post_number
        } else {
            b.highest_post_number
        };
        out.insert("highest_post_number".into(), json!(highest));
        out.insert("bumped_at".into(), json!(time_json(b.bumped_at)));
        out.insert("slug".into(), json!(slug));
        if is_topic {
            out.insert(
                "last_read_post_number".into(),
                json!(b.last_read_post_number),
            );
        }
        // BasicUserSerializer of the post's author.
        let user = match (b.post_user_id, b.username.as_deref()) {
            (Some(id), Some(username)) => {
                let mut user = Map::new();
                user.insert("id".into(), json!(id));
                user.insert("username".into(), json!(username));
                if settings.get("enable_names")?.truthy() {
                    user.insert("name".into(), json!(b.user_name));
                }
                user.insert(
                    "avatar_template".into(),
                    json!(avatar_template(
                        self.urls,
                        id,
                        username,
                        b.uploaded_avatar_id,
                        logo_small_url,
                    )?),
                );
                Value::Object(user)
            }
            _ => Value::Null,
        };
        out.insert("user".into(), user);
        Ok(Value::Object(out))
    }

    /// users#user_menu_bookmarks: the unread reminder notifications the
    /// viewer can still see the subject of, then their bookmarks minus the
    /// ones those reminders are about, up to the menu's limit together.
    pub async fn user_menu(&mut self, user_id: i32) -> Result<Value, BookmarksError> {
        let unread = Notifications {
            conn: &mut *self.conn,
            settings: self.settings,
            guardian: self.guardian,
        }
        .unread_for_user_menu(user_id, BOOKMARK_REMINDER, USER_MENU_LIST_LIMIT.min(100))
        .await?;
        let secure = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        let mut notifications = Vec::new();
        for n in unread {
            if self
                .can_see_notification_bookmark(user_id, &n["data"], &secure)
                .await?
            {
                notifications.push(n);
            }
        }
        let mut bookmarks = Vec::new();
        let remaining = USER_MENU_LIST_LIMIT - notifications.len() as i64;
        if remaining > 0 {
            let exclude: Vec<i64> = notifications
                .iter()
                .filter_map(|n| n["data"]["bookmark_id"].as_i64())
                .collect();
            let list = self
                .load(
                    user_id,
                    &ListQuery {
                        per_page: Some(remaining),
                        exclude_ids: &exclude,
                        ..Default::default()
                    },
                )
                .await?;
            bookmarks = self.serialize(&list, true).await?;
        }
        Ok(json!({"notifications": notifications, "bookmarks": bookmarks}))
    }

    /// `can_see_notification_bookmark?`: through the bookmark while it
    /// exists, else through the bookmarkable the notification names.
    async fn can_see_notification_bookmark(
        &mut self,
        user_id: i32,
        data: &Value,
        secure: &[i32],
    ) -> Result<bool, BookmarksError> {
        let Some(bookmark_id) = data["bookmark_id"].as_i64() else {
            return Ok(false);
        };
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM bookmarks WHERE id = $1 AND user_id = $2)",
        )
        .bind(bookmark_id)
        .bind(i64::from(user_id))
        .fetch_one(&mut *self.conn)
        .await?;
        if owned {
            return match self.bookmarkables(&[bookmark_id]).await?.get(&bookmark_id) {
                Some(b) => self.can_see_post(b, secure).await,
                None => Ok(false),
            };
        }
        // The bookmark is gone (auto-deleted with its reminder): the
        // notification still shows while its subject is visible.
        let topic = match data["bookmarkable_type"].as_str() {
            Some("Post") => false,
            Some("Topic") => true,
            Some(_) => {
                return Err(
                    Unsupported("bookmarks of plugin-registered types (chat messages)").into(),
                );
            }
            None => return Ok(false),
        };
        let Some(id) = data["bookmarkable_id"].as_i64() else {
            return Ok(false);
        };
        match self.bookmarkable_of(topic, id).await? {
            Some(b) => self.can_see_post(&b, secure).await,
            None => Ok(false),
        }
    }
}
