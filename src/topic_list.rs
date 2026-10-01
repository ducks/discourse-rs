//! Port of TopicListSerializer, TopicListItemSerializer (with its
//! ListableTopicSerializer/BasicTopicSerializer parents), TopicPosterSerializer
//! and the side-loaded PosterSerializer users, for anonymous users.
//! Also TopicPostersSummary (app/models/topic_posters_summary.rb) and
//! TopicList#load_topics' user lookup.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::avatar::{self, AvatarError};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::tags::visible_tag_ids_subquery;
use crate::topic_query::{TopicList, TopicRow};
use crate::url::{UrlError, Urls};

/// Which serializer a topic is rendered with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// TopicListItemSerializer
    ListItem,
    /// SuggestedTopicSerializer (ListableTopicSerializer + tags, posters with users)
    Suggested,
    /// ListableTopicSerializer with an embedded last_poster (featured topics
    /// in /categories.json)
    Listable,
    /// SearchTopicListItemSerializer: Listable without image_url, plus tags
    /// and category_id; `user_data` only when the controller ran
    /// `find_user_data` (/search.json, not /search/query.json)
    SearchItem { user_data: bool },
}

/// `Topic.share_thumbnail_size`
const SHARE_THUMBNAIL_SIZE: (i32, i32) = (1024, 1024);

#[derive(Debug)]
pub enum TopicListError {
    Db(sqlx::Error),
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for TopicListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopicListError::Db(e) => write!(f, "serializing topic list: {e}"),
            TopicListError::Setting(e) => e.fmt(f),
            TopicListError::Url(e) => e.fmt(f),
            TopicListError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for TopicListError {}

impl From<sqlx::Error> for TopicListError {
    fn from(e: sqlx::Error) -> Self {
        TopicListError::Db(e)
    }
}

impl From<SettingError> for TopicListError {
    fn from(e: SettingError) -> Self {
        TopicListError::Setting(e)
    }
}

impl From<UrlError> for TopicListError {
    fn from(e: UrlError) -> Self {
        TopicListError::Url(e)
    }
}

impl From<Unsupported> for TopicListError {
    fn from(e: Unsupported) -> Self {
        TopicListError::Unsupported(e)
    }
}

impl From<AvatarError> for TopicListError {
    fn from(e: AvatarError) -> Self {
        match e {
            AvatarError::Setting(e) => TopicListError::Setting(e),
            AvatarError::Url(e) => TopicListError::Url(e),
            AvatarError::Unsupported(e) => TopicListError::Unsupported(e),
        }
    }
}

/// `UserLookup`'s user columns.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LookupUser {
    pub id: i32,
    pub username: String,
    pub name: Option<String>,
    pub uploaded_avatar_id: Option<i32>,
    pub primary_group_id: Option<i32>,
    pub flair_group_id: Option<i32>,
    pub admin: bool,
    pub moderator: bool,
    pub trust_level: i32,
}

/// Rails `TimeWithZone#as_json` in UTC: ISO 8601, milliseconds truncated.
pub fn time_json(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub struct TopicListSerializer<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
    /// `more_topics_url` as ListController computed it (with base path).
    pub more_topics_url: Option<String>,
    /// `TopicList#category`: scopes top_tags to the category and its
    /// direct subcategories.
    pub category_id: Option<i32>,
    /// Per-topic lookups fetched for the whole page at once (see
    /// `prefetch`); the per-topic methods fall back to their own query
    /// for topics not in it.
    pub prefetched: Prefetched,
}

/// What `prefetch` loads for a page of topics in one query each, where
/// the per-topic serializer would otherwise issue three round-trips per
/// topic: the visible tags, the share thumbnail or image upload, and the
/// first post's like count.
#[derive(Debug, Default)]
pub struct Prefetched {
    topic_ids: Vec<i32>,
    tags: HashMap<i32, Vec<TagRow>>,
    thumbnails: HashMap<i64, String>,
    uploads: HashMap<i64, (String, bool)>,
    op_like_counts: HashMap<i32, i32>,
    /// `TopicUser.lookup_for(user, topics)`
    topic_users: HashMap<i32, TopicUserRow>,
    /// `DismissedTopicUser` topic ids for the user
    dismissed: Vec<i32>,
    treat_as_new_topic_start_date: Option<NaiveDateTime>,
}

/// The topic_users columns the list item reads (`user_data`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TopicUserRow {
    pub topic_id: i32,
    pub last_read_post_number: Option<i32>,
    pub notification_level: i32,
    pub cleared_pinned_at: Option<NaiveDateTime>,
    pub liked: Option<bool>,
    pub bookmarked: Option<bool>,
}

type TagRow = (i32, String, Option<String>, Option<String>);
/// topic_id plus a TagRow
type TopicTagRow = (i32, i32, String, Option<String>, Option<String>);

impl TopicListSerializer<'_> {
    /// The whole response: side-loaded `users`, `primary_groups`,
    /// `flair_groups` (present once any poster was serialized), then
    /// `topic_list`.
    pub async fn serialize(&mut self, list: &TopicList) -> Result<Value, TopicListError> {
        self.prefetch(&list.topics).await?;
        let lookup = self.user_lookup(&list.topics).await?;
        let logo_small_url = self.logo_small_url().await?;
        let group_ids: Vec<i32> = lookup
            .values()
            .flat_map(|u| [u.primary_group_id, u.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;
        let tagging = self.settings.get("tagging_enabled")?.truthy();

        let mut users: Vec<Value> = Vec::new();
        let mut seen_users: Vec<i32> = Vec::new();
        let mut primary_groups: Vec<Value> = Vec::new();
        let mut flair_groups: Vec<Value> = Vec::new();
        let mut seen_primary: Vec<i32> = Vec::new();
        let mut seen_flair: Vec<i32> = Vec::new();
        let mut topics: Vec<Value> = Vec::with_capacity(list.topics.len());
        let mut any_poster = false;
        for topic in &list.topics {
            let posters = posters_summary(topic, &lookup, self.i18n);
            for poster in &posters {
                any_poster = true;
                if !seen_users.contains(&poster.user.id) {
                    seen_users.push(poster.user.id);
                    users.push(self.serialize_user(
                        &poster.user,
                        logo_small_url.as_deref(),
                        &groups,
                    )?);
                }
                // Side-loaded once each, in first-seen order, like AMS's embed :ids.
                if let Some(g) = poster.user.primary_group_id.and_then(|id| groups.get(&id)) {
                    if !seen_primary.contains(&g.id) {
                        seen_primary.push(g.id);
                        primary_groups.push(g.primary_json());
                    }
                }
                if let Some(g) = poster.user.flair_group_id.and_then(|id| groups.get(&id)) {
                    if !seen_flair.contains(&g.id) {
                        seen_flair.push(g.id);
                        flair_groups.push(g.flair_json()?);
                    }
                }
            }
            topics.push(
                self.serialize_topic(topic, &posters, tagging, Mode::ListItem)
                    .await?,
            );
        }

        let mut out = Map::new();
        if any_poster {
            out.insert("users".into(), Value::Array(users));
            out.insert("primary_groups".into(), Value::Array(primary_groups));
            out.insert("flair_groups".into(), Value::Array(flair_groups));
        }

        let mut topic_list = Map::new();
        topic_list.insert(
            "can_create_topic".into(),
            json!(
                self.guardian
                    .can_create_topic(&mut *self.conn, self.settings)
                    .await?
            ),
        );
        topic_list.insert("filter".into(), json!(list.filter));
        if let Some(url) = &self.more_topics_url {
            if list.topics.len() as i64 == list.per_page {
                topic_list.insert("more_topics_url".into(), json!(url));
            }
        }
        topic_list.insert("per_page".into(), json!(list.per_page));
        if tagging {
            topic_list.insert("top_tags".into(), self.top_tags().await?);
            // `TopicList#tags`: Tag.where(id: tag_ids), included when present.
            if !list.tag_ids.is_empty() {
                let mut tags = Vec::new();
                for tag in crate::tags::Tag::find_all(&mut *self.conn, &list.tag_ids).await? {
                    tags.push(
                        tag.serialize(&mut *self.conn, self.guardian, self.settings)
                            .await?,
                    );
                }
                topic_list.insert("tags".into(), Value::Array(tags));
            }
        }
        topic_list.insert("topics".into(), Value::Array(topics));
        out.insert("topic_list".into(), Value::Object(topic_list));
        Ok(Value::Object(out))
    }

    /// `TopicList#load_topics`: every author, last poster and featured
    /// poster across the page, through UserLookup.
    pub(crate) async fn user_lookup(
        &mut self,
        topics: &[TopicRow],
    ) -> Result<HashMap<i32, LookupUser>, TopicListError> {
        let mut ids: Vec<i32> = Vec::new();
        for t in topics {
            ids.extend(t.user_id);
            ids.push(t.last_post_user_id);
            ids.extend(
                [
                    t.featured_user1_id,
                    t.featured_user2_id,
                    t.featured_user3_id,
                    t.featured_user4_id,
                ]
                .into_iter()
                .flatten(),
            );
        }
        self.user_lookup_for(&ids).await
    }

    /// UserLookup for an explicit id list.
    pub(crate) async fn user_lookup_for(
        &mut self,
        ids: &[i32],
    ) -> Result<HashMap<i32, LookupUser>, TopicListError> {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let users: Vec<LookupUser> = sqlx::query_as(
            "SELECT id, username, name, uploaded_avatar_id, primary_group_id, flair_group_id, \
                    admin, moderator, trust_level FROM users WHERE id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(users.into_iter().map(|u| (u.id, u)).collect())
    }

    pub(crate) async fn logo_small_url(&mut self) -> Result<Option<String>, TopicListError> {
        let id = self.settings.get("logo_small")?.to_i();
        if id == 0 {
            return Ok(None);
        }
        Ok(sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
            .bind(i32::try_from(id).unwrap_or(0))
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// PosterSerializer (BasicUserSerializer + flair/primary group mixins).
    pub(crate) fn serialize_user(
        &self,
        user: &LookupUser,
        logo_small_url: Option<&str>,
        groups: &std::collections::HashMap<i32, crate::groups::Group>,
    ) -> Result<Value, TopicListError> {
        let mut out = Map::new();
        out.insert("id".into(), json!(user.id));
        out.insert("username".into(), json!(user.username));
        if self.settings.get("enable_names")?.truthy() {
            out.insert("name".into(), json!(user.name));
        }
        out.insert(
            "avatar_template".into(),
            json!(avatar::avatar_template(
                self.urls,
                user.id,
                &user.username,
                user.uploaded_avatar_id,
                logo_small_url
            )?),
        );
        crate::groups::user_group_fields(
            &mut out,
            groups,
            user.primary_group_id,
            user.flair_group_id,
        )?;
        if user.admin {
            out.insert("admin".into(), json!(true));
        }
        if user.moderator {
            out.insert("moderator".into(), json!(true));
        }
        out.insert("trust_level".into(), json!(user.trust_level));
        Ok(Value::Object(out))
    }

    /// TopicListItemSerializer in attribute order.
    pub(crate) async fn serialize_topic(
        &mut self,
        t: &TopicRow,
        posters: &[Poster],
        tagging: bool,
        mode: Mode,
    ) -> Result<Value, TopicListError> {
        let user_data = match mode {
            Mode::SearchItem { user_data: false } => None,
            _ => self.user_data(t.id).await?,
        };
        // PinnedCheck: a pin the user cleared after it was set is unpinned.
        let unpinned = match (
            t.pinned_at,
            user_data.as_ref().and_then(|u| u.cleared_pinned_at),
        ) {
            (Some(pinned_at), Some(cleared)) => Some(cleared > pinned_at),
            _ => None,
        };
        let pinned = t.pinned_at.is_some() && unpinned != Some(true);
        let whisperer = self.guardian.is_whisperer(self.settings)?;
        let highest = if whisperer && t.highest_staff_post_number != 0 {
            t.highest_staff_post_number
        } else {
            t.highest_post_number
        };
        let mut out = Map::new();
        let fancy_title = crate::topic_query::fancy_title(t)?;
        out.insert("fancy_title".into(), json!(fancy_title));
        out.insert("id".into(), json!(t.id));
        out.insert("title".into(), json!(t.title));
        let Some(slug) = &t.slug else {
            return Err(Unsupported("topics without a stored slug (Slug.for)").into());
        };
        out.insert("slug".into(), json!(slug));
        out.insert("posts_count".into(), json!(t.posts_count));
        out.insert("reply_count".into(), json!(t.reply_count));
        out.insert("highest_post_number".into(), json!(highest));
        if !matches!(mode, Mode::SearchItem { .. }) {
            out.insert("image_url".into(), self.image_url(t).await?);
        }
        out.insert("created_at".into(), json!(time_json(t.created_at)));
        out.insert(
            "last_posted_at".into(),
            json!(t.last_posted_at.map(time_json)),
        );
        out.insert("bumped".into(), json!(t.created_at < t.bumped_at));
        out.insert("bumped_at".into(), json!(time_json(t.bumped_at)));
        out.insert("archetype".into(), json!(t.archetype));
        out.insert(
            "unseen".into(),
            json!(!self.seen(t, user_data.as_ref()).await?),
        );
        if let Some(u) = &user_data {
            out.insert(
                "last_read_post_number".into(),
                json!(u.last_read_post_number),
            );
            out.insert("unread".into(), json!(0));
            // Unread#unread_posts
            let unread_posts = match u.last_read_post_number {
                Some(read) if u.notification_level > 1 && read <= highest => highest - read,
                _ => 0,
            };
            out.insert("new_posts".into(), json!(unread_posts));
            out.insert("unread_posts".into(), json!(unread_posts));
        }
        out.insert("pinned".into(), json!(pinned));
        out.insert("unpinned".into(), json!(unpinned));
        if mode == Mode::Suggested
            || pinned
            || self.settings.get("always_include_topic_excerpts")?.truthy()
        {
            out.insert("excerpt".into(), json!(t.excerpt));
        }
        out.insert("visible".into(), json!(t.visible));
        out.insert("closed".into(), json!(t.closed));
        out.insert("archived".into(), json!(t.archived));
        if let Some(u) = &user_data {
            out.insert("notification_level".into(), json!(u.notification_level));
        }
        out.insert(
            "bookmarked".into(),
            json!(user_data.as_ref().and_then(|u| u.bookmarked)),
        );
        out.insert(
            "liked".into(),
            json!(user_data.as_ref().and_then(|u| u.liked)),
        );
        if crate::emoji::has_emoji_code(&t.title) {
            out.insert(
                "unicode_title".into(),
                json!(crate::emoji::gsub_emoji_to_unicode(&t.title)),
            );
        }
        if let Some(reason) = t.visibility_reason_id {
            out.insert("visibility_reason_id".into(), json!(reason));
        }
        if mode != Mode::Listable && tagging && t.archetype != "private_message" {
            let (tags, descriptions) = self.tags(t.id).await?;
            out.insert("tags".into(), tags);
            out.insert("tags_descriptions".into(), descriptions);
        }
        if mode == Mode::Listable {
            return self.finish_listable(out, t, posters).await;
        }
        if matches!(mode, Mode::SearchItem { .. }) {
            out.insert("category_id".into(), json!(t.category_id));
            return Ok(Value::Object(out));
        }
        if mode == Mode::Suggested {
            return self.finish_suggested(out, t, posters).await;
        }
        out.insert("views".into(), json!(t.views));
        out.insert("like_count".into(), json!(t.like_count));
        out.insert("has_summary".into(), json!(t.has_summary));
        let last_poster = posters
            .iter()
            .find(|p| p.user.id == t.last_post_user_id)
            .map(|p| p.user.username.clone());
        out.insert("last_poster_username".into(), json!(last_poster));
        out.insert("category_id".into(), json!(t.category_id));
        out.insert("op_like_count".into(), self.op_like_count(t.id).await?);
        out.insert("pinned_globally".into(), json!(t.pinned_globally));
        if self.settings.get("topic_featured_link_enabled")?.truthy() {
            out.insert("featured_link".into(), json!(t.featured_link));
            if t.featured_link.as_deref().is_some_and(|l| !l.is_empty()) {
                return Err(Unsupported("featured_link_root_domain").into());
            }
        }
        out.insert(
            "posters".into(),
            Value::Array(
                posters
                    .iter()
                    .map(|p| {
                        json!({
                            "extras": p.extras,
                            "description": p.description,
                            "user_id": p.user.id,
                            "primary_group_id": p.user.primary_group_id,
                            "flair_group_id": p.user.flair_group_id,
                        })
                    })
                    .collect(),
            ),
        );
        Ok(Value::Object(out))
    }

    /// `Topic#image_url` through `UrlHelper.cook_url` for the local store:
    /// the 1024x1024 thumbnail, else the image upload, made absolute and
    /// schemeless.
    /// ListableTopicSerializer's tail: `last_poster` embedded as a BasicUser.
    async fn finish_listable(
        &mut self,
        mut out: Map<String, Value>,
        t: &TopicRow,
        posters: &[Poster],
    ) -> Result<Value, TopicListError> {
        let logo_small_url = self.logo_small_url().await?;
        let last = posters.iter().find(|p| p.user.id == t.last_post_user_id);
        let last_poster = match last {
            Some(p) => {
                let mut u = Map::new();
                u.insert("id".into(), json!(p.user.id));
                u.insert("username".into(), json!(p.user.username));
                if self.settings.get("enable_names")?.truthy() {
                    u.insert("name".into(), json!(p.user.name));
                }
                u.insert(
                    "avatar_template".into(),
                    json!(avatar::avatar_template(
                        self.urls,
                        p.user.id,
                        &p.user.username,
                        p.user.uploaded_avatar_id,
                        logo_small_url.as_deref()
                    )?),
                );
                Value::Object(u)
            }
            None => Value::Null,
        };
        out.insert("last_poster".into(), last_poster);
        Ok(Value::Object(out))
    }

    /// SuggestedTopicSerializer's tail after the Listable attributes:
    /// like_count, views, category_id, featured_link, op_like_count and
    /// posters with embedded users.
    async fn finish_suggested(
        &mut self,
        mut out: Map<String, Value>,
        t: &TopicRow,
        posters: &[Poster],
    ) -> Result<Value, TopicListError> {
        out.insert("like_count".into(), json!(t.like_count));
        out.insert("views".into(), json!(t.views));
        out.insert("category_id".into(), json!(t.category_id));
        if self.settings.get("topic_featured_link_enabled")?.truthy() {
            out.insert("featured_link".into(), json!(t.featured_link));
            if t.featured_link.as_deref().is_some_and(|l| !l.is_empty()) {
                return Err(Unsupported("featured_link_root_domain").into());
            }
        }
        out.insert("op_like_count".into(), self.op_like_count(t.id).await?);
        let logo_small_url = self.logo_small_url().await?;
        let group_ids: Vec<i32> = posters
            .iter()
            .flat_map(|p| [p.user.primary_group_id, p.user.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;
        let mut list = Vec::with_capacity(posters.len());
        for p in posters {
            list.push(json!({
                "extras": p.extras,
                "description": p.description,
                "user": self.serialize_user(&p.user, logo_small_url.as_deref(), &groups)?,
            }));
        }
        out.insert("posters".into(), Value::Array(list));
        Ok(Value::Object(out))
    }

    /// The per-topic lookups for a page, one query each.
    pub async fn prefetch(&mut self, topics: &[TopicRow]) -> Result<(), TopicListError> {
        let topic_ids: Vec<i32> = topics.iter().map(|t| t.id).collect();
        let upload_ids: Vec<i64> = topics.iter().filter_map(|t| t.image_upload_id).collect();
        let mut p = Prefetched {
            topic_ids: topic_ids.clone(),
            ..Prefetched::default()
        };
        if topic_ids.is_empty() {
            self.prefetched = p;
            return Ok(());
        }

        let order = self.tag_order()?;
        let rows: Vec<TopicTagRow> = sqlx::query_as(&format!(
            "SELECT tt.topic_id, t.id, t.name, t.slug, t.description FROM topic_tags tt \
             JOIN tags t ON t.id = tt.tag_id \
             WHERE tt.topic_id = ANY($1) AND t.id IN {visible} ORDER BY tt.topic_id, {order}",
            visible = visible_tag_ids_subquery(self.guardian, self.settings)?,
        ))
        .bind(&topic_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        for (topic_id, id, name, slug, description) in rows {
            p.tags
                .entry(topic_id)
                .or_default()
                .push((id, name, slug, description));
        }

        if !upload_ids.is_empty() {
            let thumbnails: Vec<(i64, String)> = sqlx::query_as(
                "SELECT DISTINCT ON (tt.upload_id) tt.upload_id::bigint, oi.url \
                 FROM topic_thumbnails tt JOIN optimized_images oi ON oi.id = tt.optimized_image_id \
                 WHERE tt.upload_id = ANY($1::bigint[]) AND tt.max_width = $2 AND tt.max_height = $3 \
                 ORDER BY tt.upload_id, tt.id",
            )
            .bind(&upload_ids)
            .bind(SHARE_THUMBNAIL_SIZE.0)
            .bind(SHARE_THUMBNAIL_SIZE.1)
            .fetch_all(&mut *self.conn)
            .await?;
            p.thumbnails = thumbnails.into_iter().collect();
            let uploads: Vec<(i64, String, bool)> = sqlx::query_as(
                "SELECT id::bigint, url, secure FROM uploads WHERE id = ANY($1::bigint[])",
            )
            .bind(&upload_ids)
            .fetch_all(&mut *self.conn)
            .await?;
            p.uploads = uploads
                .into_iter()
                .map(|(id, url, secure)| (id, (url, secure)))
                .collect();
        }

        let counts: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT topic_id, like_count FROM posts \
             WHERE topic_id = ANY($1) AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(&topic_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        p.op_like_counts = counts.into_iter().collect();

        // TopicList#load_topics' per-user lookups.
        if let Some(uid) = self.guardian.user_id() {
            let rows: Vec<TopicUserRow> = sqlx::query_as(
                "SELECT topic_id, last_read_post_number, notification_level, cleared_pinned_at, liked, bookmarked \
                 FROM topic_users WHERE topic_id = ANY($1) AND user_id = $2",
            )
            .bind(&topic_ids)
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
            p.topic_users = rows.into_iter().map(|r| (r.topic_id, r)).collect();
            p.dismissed = sqlx::query_scalar(
                "SELECT topic_id FROM dismissed_topic_users WHERE topic_id = ANY($1) AND user_id = $2",
            )
            .bind(&topic_ids)
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
            p.treat_as_new_topic_start_date = self
                .guardian
                .treat_as_new_topic_start_date(&mut *self.conn, self.settings)
                .await?;
        }
        self.prefetched = p;
        Ok(())
    }

    /// `user_data` for a topic: its topic_users row for the current user.
    /// Topics outside the prefetched page (suggested, search) look it up.
    async fn user_data(&mut self, topic_id: i32) -> Result<Option<TopicUserRow>, TopicListError> {
        let Some(uid) = self.guardian.user_id() else {
            return Ok(None);
        };
        if self.prefetched_for(topic_id) {
            return Ok(self.prefetched.topic_users.get(&topic_id).cloned());
        }
        Ok(sqlx::query_as(
            "SELECT topic_id, last_read_post_number, notification_level, cleared_pinned_at, liked, bookmarked \
             FROM topic_users WHERE topic_id = $1 AND user_id = $2",
        )
        .bind(topic_id)
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?)
    }

    /// `ListableTopicSerializer#seen`: true for anonymous, for a read
    /// topic, a dismissed one, or one older than the user's new-topic
    /// start date.
    async fn seen(
        &mut self,
        t: &TopicRow,
        user_data: Option<&TopicUserRow>,
    ) -> Result<bool, TopicListError> {
        let Some(uid) = self.guardian.user_id() else {
            return Ok(true);
        };
        if user_data.is_some_and(|u| u.last_read_post_number.is_some()) {
            return Ok(true);
        }
        let dismissed = if self.prefetched_for(t.id) {
            self.prefetched.dismissed.contains(&t.id)
        } else {
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM dismissed_topic_users WHERE topic_id = $1 AND user_id = $2)",
            )
            .bind(t.id)
            .bind(uid)
            .fetch_one(&mut *self.conn)
            .await?
        };
        if dismissed {
            return Ok(true);
        }
        let start = if self.prefetched_for(t.id) {
            self.prefetched.treat_as_new_topic_start_date
        } else {
            self.guardian
                .treat_as_new_topic_start_date(&mut *self.conn, self.settings)
                .await?
        };
        Ok(start.is_some_and(|s| t.created_at < s))
    }

    fn prefetched_for(&self, topic_id: i32) -> bool {
        self.prefetched.topic_ids.contains(&topic_id)
    }

    /// The per-topic tag order: by name, or by `Tag.topic_count_column`
    /// (the staff count for staff or when secure categories count), with
    /// the original (id) order reversed among ties like Ruby's sort.reverse.
    fn tag_order(&self) -> Result<String, TopicListError> {
        if self.settings.get("tags_sort_alphabetically")?.truthy() {
            return Ok("t.name ASC".to_string());
        }
        let column = self.guardian.tag_count_column(self.settings)?;
        Ok(format!("t.{column} DESC, t.id DESC"))
    }

    pub(crate) async fn image_url(&mut self, t: &TopicRow) -> Result<Value, TopicListError> {
        let thumbnail: Option<String> = if self.prefetched_for(t.id) {
            t.image_upload_id
                .and_then(|id| self.prefetched.thumbnails.get(&id).cloned())
        } else {
            sqlx::query_scalar(
                "SELECT oi.url FROM topic_thumbnails tt JOIN optimized_images oi ON oi.id = tt.optimized_image_id \
                 WHERE tt.upload_id = $1 AND tt.max_width = $2 AND tt.max_height = $3 ORDER BY tt.id LIMIT 1",
            )
            .bind(t.image_upload_id)
            .bind(SHARE_THUMBNAIL_SIZE.0)
            .bind(SHARE_THUMBNAIL_SIZE.1)
            .fetch_optional(&mut *self.conn)
            .await?
        };
        let raw = match (thumbnail, t.image_upload_id) {
            (Some(url), _) => Some(url),
            (None, Some(upload_id)) => {
                let row: Option<(String, bool)> = if self.prefetched_for(t.id) {
                    self.prefetched.uploads.get(&upload_id).cloned()
                } else {
                    sqlx::query_as("SELECT url, secure FROM uploads WHERE id = $1")
                        .bind(upload_id)
                        .fetch_optional(&mut *self.conn)
                        .await?
                };
                match row {
                    Some((_, true)) if self.settings.get("secure_uploads")?.truthy() => {
                        return Err(Unsupported("secure uploads").into());
                    }
                    Some((url, _)) => Some(url),
                    None => None,
                }
            }
            (None, None) => None,
        };
        let Some(raw) = raw else {
            return Ok(Value::Null);
        };
        if self.urls.config.globals.cdn_url().is_some() {
            return Err(Unsupported("topic images behind a CDN").into());
        }
        // absolute_without_cdn, then schemaless.
        let absolute = self.urls.absolute(&raw)?;
        let schemeless = match absolute.get(..5).map(|s| s.eq_ignore_ascii_case("http:")) {
            Some(true) => absolute[5..].to_string(),
            _ => absolute,
        };
        Ok(json!(schemeless))
    }

    /// `topic.visible_tags(guardian)` sorted by public_topic_count desc,
    /// as `[{id, name, slug}]` plus `tags_descriptions`.
    pub(crate) async fn tags(&mut self, topic_id: i32) -> Result<(Value, Value), TopicListError> {
        let rows: Vec<TagRow> = if self.prefetched_for(topic_id) {
            self.prefetched
                .tags
                .get(&topic_id)
                .cloned()
                .unwrap_or_default()
        } else {
            let order = self.tag_order()?;
            let sql = format!(
                "SELECT t.id, t.name, t.slug, t.description FROM topic_tags tt JOIN tags t ON t.id = tt.tag_id \
                 WHERE tt.topic_id = $1 AND t.id IN {visible} ORDER BY {order}",
                visible = visible_tag_ids_subquery(self.guardian, self.settings)?,
            );
            sqlx::query_as(&sql)
                .bind(topic_id)
                .fetch_all(&mut *self.conn)
                .await?
        };
        let mut descriptions = Map::new();
        let tags: Vec<Value> = rows
            .into_iter()
            .map(|(id, name, slug, description)| {
                if let Some(d) = description.filter(|d| !d.is_empty()) {
                    descriptions.insert(name.clone(), json!(truncate(&d, 80)));
                }
                let slug = slug
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("{id}-tag"));
                json!({"id": id, "name": name, "slug": slug})
            })
            .collect();
        Ok((Value::Array(tags), Value::Object(descriptions)))
    }

    /// `first_post&.like_count`: post_number 1, not deleted.
    async fn op_like_count(&mut self, topic_id: i32) -> Result<Value, TopicListError> {
        if self.prefetched_for(topic_id) {
            return Ok(json!(self.prefetched.op_like_counts.get(&topic_id)));
        }
        let count: Option<i32> = sqlx::query_scalar(
            "SELECT like_count FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(topic_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(json!(count))
    }

    /// `Tag.top_tags` as in site.json's top_tags.
    async fn top_tags(&mut self) -> Result<Value, TopicListError> {
        let mut category_ids = self
            .guardian
            .allowed_category_ids(&mut *self.conn, self.settings)
            .await?;
        if let Some(category_id) = self.category_id {
            // `allowed & ([category.id] + category.subcategories.pluck(:id))`
            let mut scope: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM categories WHERE parent_category_id = $1")
                    .bind(category_id)
                    .fetch_all(&mut *self.conn)
                    .await?;
            scope.insert(0, category_id);
            category_ids.retain(|id| scope.contains(id));
        }
        if category_ids.is_empty() {
            return Ok(json!([]));
        }
        let limit = self.settings.get("max_tags_in_filter_list")?.to_i() + 1;
        let rows: Vec<(i32, String, Option<String>)> = sqlx::query_as(&format!(
            "SELECT tags.id, tags.name, tags.slug FROM category_tag_stats stats \
             JOIN tags ON stats.tag_id = tags.id AND stats.topic_count > 0 \
             WHERE stats.category_id = ANY($1) AND tags.target_tag_id IS NULL \
             AND tags.id IN {visible} \
             GROUP BY tags.id ORDER BY SUM(stats.topic_count) DESC, tags.name ASC LIMIT $2",
            visible = visible_tag_ids_subquery(self.guardian, self.settings)?,
        ))
        .bind(&category_ids)
        .bind(limit)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(json!(
            rows.into_iter()
                .map(|(id, name, slug)| {
                    let slug = slug
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("{id}-tag"));
                    json!({"id": id, "name": name, "slug": slug})
                })
                .collect::<Vec<_>>()
        ))
    }
}

/// Ruby `String#truncate(n)`: at most n chars including the "..." omission.
fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    let head: String = chars[..n.saturating_sub(3)].iter().collect();
    format!("{head}...")
}

pub(crate) struct Poster {
    pub(crate) user: LookupUser,
    pub(crate) extras: Option<&'static str>,
    pub(crate) description: String,
}

/// `TopicPostersSummary#summary`: up to five posters from author, last
/// poster and featured users, the last poster shuffled to the back.
pub(crate) fn posters_summary(
    t: &TopicRow,
    lookup: &HashMap<i32, LookupUser>,
    i18n: &I18n,
) -> Vec<Poster> {
    let mut user_ids: Vec<Option<i32>> = vec![t.user_id, Some(t.last_post_user_id)];
    user_ids.extend(featured_user_ids(t).into_iter().map(Some));

    let mut top: Vec<&LookupUser> = Vec::new();
    for id in user_ids.iter().flatten() {
        if let Some(u) = lookup.get(id) {
            if !top.iter().any(|x| x.id == u.id) {
                top.push(u);
            }
        }
    }
    top.truncate(5);

    // shuffle_last_poster_to_back_in
    if t.user_id != Some(t.last_post_user_id) {
        top.retain(|u| u.id != t.last_post_user_id);
        if let Some(last) = lookup.get(&t.last_post_user_id) {
            top.push(last);
        }
    }

    let mut sorted_uniq = user_ids.clone();
    sorted_uniq.sort();
    sorted_uniq.dedup();
    let single = sorted_uniq.len() == 1;

    let descriptions = descriptions_by_id(t, &user_ids, i18n);
    top.into_iter()
        .map(|u| {
            let is_last = u.id == t.last_post_user_id;
            let extras = if is_last && single {
                Some("latest single")
            } else if is_last {
                Some("latest")
            } else {
                None
            };
            Poster {
                user: u.clone(),
                extras,
                description: descriptions.get(&u.id).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

fn featured_user_ids(t: &TopicRow) -> Vec<i32> {
    let mut ids: Vec<i32> = [
        t.featured_user1_id,
        t.featured_user2_id,
        t.featured_user3_id,
        t.featured_user4_id,
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut seen = Vec::new();
    ids.retain(|id| {
        if seen.contains(id) {
            false
        } else {
            seen.push(*id);
            true
        }
    });
    ids
}

/// `descriptions_by_id`: the first id is the original poster, the second the
/// most recent poster, later ones frequent or recent posters, joined with
/// ", ". Stops at the first nil.
fn descriptions_by_id(t: &TopicRow, user_ids: &[Option<i32>], i18n: &I18n) -> HashMap<i32, String> {
    let recent: Vec<i32> = [t.featured_user3_id, t.featured_user4_id]
        .into_iter()
        .flatten()
        .collect();
    let joiner = i18n.t("poster_description_joiner").unwrap_or(", ");
    let mut parts: HashMap<i32, Vec<&str>> = HashMap::new();
    let mut order: Vec<i32> = Vec::new();
    for (i, id) in user_ids.iter().enumerate() {
        let Some(id) = id else { break };
        let key = match i {
            0 => "original_poster",
            1 => "most_recent_poster",
            _ if recent.contains(id) => "recent_poster",
            _ => "frequent_poster",
        };
        if !order.contains(id) {
            order.push(*id);
        }
        parts
            .entry(*id)
            .or_default()
            .push(i18n.t(key).unwrap_or(key));
    }
    parts
        .into_iter()
        .map(|(id, p)| (id, p.join(joiner)))
        .collect()
}

impl From<crate::guardian::GuardianError> for TopicListError {
    fn from(e: crate::guardian::GuardianError) -> Self {
        match e {
            crate::guardian::GuardianError::Db(e) => TopicListError::Db(e),
            crate::guardian::GuardianError::Setting(e) => TopicListError::Setting(e),
            crate::guardian::GuardianError::Unsupported(e) => TopicListError::Unsupported(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_serialize_like_rails() {
        let t = NaiveDateTime::parse_from_str("2026-07-25 05:58:28.320456", "%Y-%m-%d %H:%M:%S%.f")
            .unwrap();
        assert_eq!(time_json(t), "2026-07-25T05:58:28.320Z");
        let whole =
            NaiveDateTime::parse_from_str("2026-07-25 05:58:28", "%Y-%m-%d %H:%M:%S").unwrap();
        assert_eq!(time_json(whole), "2026-07-25T05:58:28.000Z");
    }

    #[test]
    fn truncate_matches_ruby() {
        assert_eq!(truncate("short", 80), "short");
        assert_eq!(
            truncate(&"x".repeat(100), 80),
            format!("{}...", "x".repeat(77))
        );
    }
}
