//! WebHookPostSerializer and WebHookTopicViewSerializer: a post and a
//! topic as a web hook's payload shows them, seen by the system user,
//! outside any topic view (deleted posts and topics included).

use serde_json::{Map, Value, json};

use super::{PostRow, TopicView, TopicViewError, Viewer};
use crate::Unsupported;
use crate::post_actions::ActionTypes;
use crate::topic_guardian::TopicCtx;
use crate::topic_list::time_json;

/// PostSerializer's keys WebHookPostSerializer leaves out (with those
/// only a topic view has: `read`, `link_counts`).
const LEFT_OUT: &[&str] = &[
    "can_view",
    "can_edit",
    "can_delete",
    "can_permanently_delete",
    "can_recover",
    "can_see_hidden_post",
    "can_wiki",
    "actions_summary",
    "can_view_edit_history",
    "yours",
    "flair_url",
    "flair_bg_color",
    "flair_color",
    "notice",
    "mentioned_users",
    "badges_granted",
    "readers_count",
    "read",
    "link_counts",
];

/// How the caller holds the post Rails serializes (its object in memory).
#[derive(Debug, Clone, Copy, Default)]
pub struct Held {
    /// The post's `topic` association holds its topic, trashed or not
    /// (PostDestroyer had it in hand); else a trashed topic is none.
    pub topic_loaded: bool,
    /// PostCreator's own object: its `reads` still the default.
    pub just_created: bool,
    /// The held topic's `posts_count`, from before the caller changed it.
    pub topic_posts_count: Option<i32>,
}

/// A topic as the caller held it before changing it: TopicView serializes
/// that object, its trash! only setting when and by whom.
#[derive(Clone)]
pub struct HeldTopic {
    row: crate::topic_query::TopicRow,
    extra: super::TopicExtra,
}

impl HeldTopic {
    pub async fn take(
        conn: &mut sqlx::PgConnection,
        topic_id: i32,
    ) -> Result<HeldTopic, TopicViewError> {
        let sql = format!(
            "SELECT {} FROM topics WHERE topics.id = $1",
            crate::topic_query::TOPIC_COLUMNS
        );
        let row = sqlx::query_as(&sql)
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(TopicViewError::NotFound)?;
        let extra = sqlx::query_as(
            "SELECT word_count, deleted_at, pinned_until, slow_mode_seconds, external_id \
             FROM topics WHERE id = $1",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        Ok(HeldTopic { row, extra })
    }

    pub fn posts_count(&self) -> i32 {
        self.row.posts_count
    }
}

impl TopicView<'_> {
    /// The viewer outside a topic view: the guardian's answers about the
    /// topic, its bookmarks, nothing it read or took.
    async fn bare_viewer(&mut self, topic_id: i32) -> Result<Viewer, TopicViewError> {
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
        Ok(Viewer {
            ctx,
            can_see,
            secure_category_ids,
            topic_user: None,
            read_post_numbers: Vec::new(),
            bookmarks: Vec::new(),
            taken: Default::default(),
            can_post_anywhere: false,
            can_create_post: false,
            action_types,
            draft_sequence: None,
            queue_enabled: false,
            has_deleted: None,
        })
    }

    /// WebHookPostSerializer for a post, as the caller holds it.
    pub async fn web_hook_post(
        &mut self,
        post_id: i32,
        held: Held,
    ) -> Result<Value, TopicViewError> {
        let sql = format!("{} WHERE id = $1", Self::POST_SQL);
        let mut post: PostRow = sqlx::query_as(&sql)
            .bind(post_id)
            .fetch_optional(&mut *self.conn)
            .await?
            .ok_or(TopicViewError::NotFound)?;
        if held.just_created {
            // The author's timing counts the read in the row, not in the
            // creator's object.
            post.reads = 0;
        }
        let (topic, extra) = self.find_topic(post.topic_id).await?;
        let topic_present = held.topic_loaded || extra.deleted_at.is_none();
        let mut viewer = self.bare_viewer(topic.id).await?;
        // post_bookmark: Bookmark.find_by(user: scope.user, bookmarkable: post)
        if let Some(uid) = self.guardian.user_id() {
            viewer.bookmarks = sqlx::query_as(
                "SELECT b.id, b.bookmarkable_id, b.bookmarkable_type, b.reminder_at, b.name, \
                        b.auto_delete_preference, p.post_number \
                 FROM bookmarks b JOIN posts p ON p.id = b.bookmarkable_id \
                 WHERE b.user_id = $1 AND b.bookmarkable_type = 'Post' AND b.bookmarkable_id = $2 \
                 LIMIT 1",
            )
            .bind(uid)
            .bind(i64::from(post.id))
            .fetch_all(&mut *self.conn)
            .await?;
        }
        let Some(slug) = topic.slug.clone() else {
            return Err(Unsupported("topics without a stored slug (Slug.for)").into());
        };
        let raw: String = sqlx::query_scalar("SELECT raw FROM posts WHERE id = $1")
            .bind(post.id)
            .fetch_one(&mut *self.conn)
            .await?;
        let deleted_by_id: Option<i32> =
            sqlx::query_scalar("SELECT deleted_by_id FROM posts WHERE id = $1")
                .bind(post.id)
                .fetch_one(&mut *self.conn)
                .await?;
        let solved = self.solved_view(&topic, extra.deleted_at.is_some()).await?;
        let can_vote = self.topic_can_vote(&topic).await?;
        let mut serialized = self
            .serialize_posts(
                &topic,
                &slug,
                std::slice::from_ref(&post),
                &viewer,
                solved.as_ref(),
                can_vote,
            )
            .await?;
        let Some(Value::Object(p)) = serialized.pop() else {
            return Err(TopicViewError::NotFound);
        };

        // Reviewable.where(target: post).first, without a topic view.
        let can_review = self
            .guardian
            .can_review_topic(self.settings, viewer.can_see)?;
        let reviewable: Option<(i64, i64, i64)> = if can_review {
            sqlx::query_as(
                "SELECT r.id, (SELECT count(*) FROM reviewable_scores s WHERE s.reviewable_id = r.id), \
                        (SELECT count(*) FROM reviewable_scores s WHERE s.reviewable_id = r.id AND s.status = 0) \
                 FROM reviewables r WHERE r.target_type = 'Post' AND r.target_id = $1 ORDER BY r.id LIMIT 1",
            )
            .bind(post.id)
            .fetch_optional(&mut *self.conn)
            .await?
        } else {
            None
        };
        let deleted_by = match deleted_by_id.filter(|_| self.guardian.is_staff()) {
            Some(id) => {
                let logo_small_url = self.logo_small_url().await?;
                Some(self.basic_user(id, logo_small_url.as_deref()).await?)
            }
            None => None,
        };

        let mut out = Map::new();
        for (key, value) in p {
            if LEFT_OUT.contains(&key.as_str()) {
                continue;
            }
            match key.as_str() {
                "topic_slug" => {
                    out.insert(key, value);
                    out.insert("topic_title".into(), json!(topic.title));
                    out.insert("category_id".into(), json!(topic.category_id));
                }
                "moderator" => {
                    // include_raw?: the system user is staff.
                    out.insert("raw".into(), json!(raw));
                    out.insert(key, value);
                }
                "deleted_at" => {
                    out.insert(key, json!(post.deleted_at.map(time_json)));
                    if let Some(by) = deleted_by.clone() {
                        out.insert("deleted_by".into(), by);
                    }
                }
                "reviewable_id" => {
                    out.insert(key, json!(reviewable.map(|r| r.0)));
                }
                "reviewable_score_count" => {
                    out.insert(key, json!(reviewable.map_or(0, |r| r.1)));
                }
                "reviewable_score_pending_count" => {
                    out.insert(key, json!(reviewable.map_or(0, |r| r.2)));
                }
                "posts_count" if held.topic_posts_count.is_some() => {
                    out.insert(key, json!(held.topic_posts_count));
                }
                _ => {
                    out.insert(key, value);
                }
            }
        }
        // topic_posts: the topic's posts (not deleted) its authors kept.
        let (count, filtered): (i64, i64) = if topic_present {
            sqlx::query_as(
                "SELECT count(*), count(*) FILTER (WHERE post_type = 1) FROM posts \
                 WHERE topic_id = $1 AND deleted_at IS NULL AND NOT user_deleted",
            )
            .bind(topic.id)
            .fetch_one(&mut *self.conn)
            .await?
        } else {
            (0, 0)
        };
        out.insert("topic_posts_count".into(), json!(count));
        out.insert("topic_filtered_posts_count".into(), json!(filtered));
        out.insert(
            "topic_archetype".into(),
            json!(if topic_present {
                topic.archetype.as_str()
            } else {
                ""
            }),
        );
        if topic_present && let Some(category_id) = topic.category_id {
            let slug: Option<Option<String>> =
                sqlx::query_scalar("SELECT slug FROM categories WHERE id = $1")
                    .bind(category_id)
                    .fetch_optional(&mut *self.conn)
                    .await?;
            if let Some(slug) = slug {
                // Category#slug_for_url
                let slug = slug
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("{category_id}-category"));
                out.insert("category_slug".into(), json!(slug));
            }
        }
        Ok(Value::Object(out))
    }

    /// WebHookLikeSerializer: the post (WebHookPostSerializer) and the
    /// user (BasicUserSerializer) of a like.
    pub async fn web_hook_like(
        &mut self,
        post_id: i32,
        user_id: i32,
    ) -> Result<Value, TopicViewError> {
        let post = self.web_hook_post(post_id, Held::default()).await?;
        let logo_small_url = self.logo_small_url().await?;
        let user = self.basic_user(user_id, logo_small_url.as_deref()).await?;
        Ok(json!({ "post": post, "user": user }))
    }

    /// WebHookTopicViewSerializer for `TopicView.new(topic, system_user)`:
    /// TopicViewSerializer less the post stream, details, suggestions,
    /// drafts, timers and bookmarks, plus who created the topic and who
    /// posted last.
    pub async fn web_hook_topic(
        &mut self,
        topic_id: i32,
        held: Option<HeldTopic>,
    ) -> Result<Value, TopicViewError> {
        let (topic, extra) = match held {
            Some(held) => {
                let (_, now) = self.find_topic(topic_id).await?;
                let mut extra = held.extra;
                extra.deleted_at = now.deleted_at;
                (held.row, extra)
            }
            None => self.find_topic(topic_id).await?,
        };
        let pm = topic.archetype == "private_message";
        let mut viewer = self.bare_viewer(topic.id).await?;
        if !viewer.can_see {
            return Err(TopicViewError::NotFound);
        }
        let Some(slug) = topic.slug.clone() else {
            return Err(Unsupported("topics without a stored slug (Slug.for)").into());
        };
        let uid = self.guardian.user_id();
        if let Some(uid) = uid {
            viewer.topic_user = sqlx::query_as(
                "SELECT posted, last_read_post_number, notification_level, notifications_reason_id, \
                        cleared_pinned_at, last_posted_at \
                 FROM topic_users WHERE topic_id = $1 AND user_id = $2 LIMIT 1",
            )
            .bind(topic.id)
            .bind(uid)
            .fetch_optional(&mut *self.conn)
            .await?;
            // bookmarks: none on a trashed topic.
            if extra.deleted_at.is_none() {
                viewer.bookmarks = sqlx::query_as(
                    "SELECT bookmarks.id, bookmarks.bookmarkable_id, bookmarks.bookmarkable_type, \
                            bookmarks.reminder_at, bookmarks.name, bookmarks.auto_delete_preference, \
                            posts.post_number \
                     FROM bookmarks \
                     LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
                     LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                                      OR (topics.id = posts.topic_id) \
                     WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
                       AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL ORDER BY bookmarks.id",
                )
                .bind(i64::from(uid))
                .bind(topic.id)
                .fetch_all(&mut *self.conn)
                .await?;
            }
            viewer.queue_enabled = self.queue_enabled(&viewer.ctx).await?;
        }
        let see_deleted = self.guardian.can_see_deleted(self.settings)?;
        // The filtered posts: the visible types, deleted ones too for staff.
        let deleted_filter = if see_deleted {
            ""
        } else {
            " AND deleted_at IS NULL"
        };
        let highest: Option<i32> = sqlx::query_scalar(&format!(
            "SELECT MAX(post_number) FROM posts WHERE topic_id = $1 AND post_type = ANY($2){deleted_filter}"
        ))
        .bind(topic.id)
        .bind(&self.post_types)
        .fetch_one(&mut *self.conn)
        .await?;
        let last_on_page: Option<i32> = sqlx::query_scalar(&format!(
            "SELECT MAX(post_number) FROM (SELECT post_number FROM posts \
             WHERE topic_id = $1 AND post_type = ANY($2){deleted_filter} \
             ORDER BY sort_order LIMIT $3) page"
        ))
        .bind(topic.id)
        .bind(&self.post_types)
        .bind(super::CHUNK_SIZE)
        .fetch_one(&mut *self.conn)
        .await?;
        let next_page = matches!((last_on_page, highest), (Some(l), Some(h)) if h > l);

        let mut out = Map::new();
        if pm
            && !next_page
            && self.guardian.is_authenticated()
            && self
                .guardian
                .in_setting_groups(self.settings, "personal_message_enabled_groups")?
        {
            out.insert(
                "related_messages".into(),
                self.related_messages(&topic).await?,
            );
        }
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        if tagging && (!pm || self.guardian.can_tag_pms(self.settings)?) {
            let (tags, descriptions) = self.list_serializer().tags(topic.id).await?;
            out.insert("tags".into(), tags);
            out.insert("tags_descriptions".into(), descriptions);
        }
        let fancy_title = crate::topic_query::fancy_title(
            &mut *self.conn,
            self.settings,
            topic.id,
            &topic.title,
            topic.fancy_title.as_deref(),
        )
        .await?;
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
        out.insert("archetype".into(), json!(topic.archetype));
        out.insert("slug".into(), json!(slug));
        out.insert("category_id".into(), json!(topic.category_id));
        out.insert("word_count".into(), json!(extra.word_count));
        out.insert("deleted_at".into(), json!(extra.deleted_at.map(time_json)));
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
        if let Some(external_id) = &extra.external_id {
            out.insert("external_id".into(), json!(external_id));
        }
        if let Some(reason) = topic.visibility_reason_id {
            out.insert("visibility_reason_id".into(), json!(reason));
        }
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
        out.insert("highest_post_number".into(), json!(highest));
        if let Some(tu) = &viewer.topic_user {
            out.insert(
                "last_read_post_number".into(),
                json!(tu.last_read_post_number),
            );
            let last_read_post_id: Option<i32> = match tu.last_read_post_number {
                Some(n) => {
                    sqlx::query_scalar(&format!(
                        "SELECT id FROM posts WHERE topic_id = $1 AND post_type = ANY($2) \
                         AND post_number = $3{deleted_filter} LIMIT 1"
                    ))
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
        let deleted_by_id: Option<i32> =
            sqlx::query_scalar("SELECT deleted_by_id FROM topics WHERE id = $1")
                .bind(topic.id)
                .fetch_one(&mut *self.conn)
                .await?;
        let logo_small_url = self.logo_small_url().await?;
        let deleted_by = match deleted_by_id {
            Some(id) => self.basic_user(id, logo_small_url.as_deref()).await?,
            None => Value::Null,
        };
        out.insert("deleted_by".into(), deleted_by);
        if see_deleted {
            let has_deleted: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND post_type = ANY($2) \
                 AND deleted_at IS NOT NULL AND post_number > 1)",
            )
            .bind(topic.id)
            .bind(&self.post_types)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("has_deleted".into(), json!(has_deleted));
        }
        if pm && topic.subtype.as_deref() == Some("moderator_warning") {
            out.insert("is_warning".into(), json!(true));
        }
        out.insert("bookmarked".into(), json!(!viewer.bookmarks.is_empty()));
        if pm {
            out.insert(
                "message_archived".into(),
                json!(self.message_archived(topic.id).await?),
            );
            let requested: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topic_custom_fields WHERE topic_id = $1 \
                 AND name = 'requested_group_id')",
            )
            .bind(topic.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if requested {
                return Err(Unsupported("requested_group_name in web hook payloads").into());
            }
        }
        if crate::emoji::has_emoji_code(&topic.title) {
            out.insert(
                "unicode_title".into(),
                json!(self.settings.emojis().gsub_to_unicode(&topic.title)),
            );
        }
        let (_, participant_count) = self.participants(topic.id, &[]).await?;
        out.insert("participant_count".into(), json!(participant_count));
        if self.guardian.is_staff() && viewer.queue_enabled {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM reviewables WHERE type = 'ReviewableQueuedPost' AND topic_id = $1 AND status = 0",
            )
            .bind(topic.id)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("queued_posts_count".into(), json!(count));
        }
        if topic.image_upload_id.is_some() {
            return Err(Unsupported("topic thumbnails").into());
        }
        out.insert("thumbnails".into(), Value::Null);
        if let Some(tu) = viewer
            .topic_user
            .as_ref()
            .filter(|_| extra.slow_mode_seconds > 0)
        {
            out.insert(
                "user_last_posted_at".into(),
                json!(tu.last_posted_at.map(time_json)),
            );
        }
        let created_by = match topic.user_id {
            Some(id) => self.basic_user(id, logo_small_url.as_deref()).await?,
            None => Value::Null,
        };
        out.insert("created_by".into(), created_by);
        let last_poster = self
            .basic_user(topic.last_post_user_id, logo_small_url.as_deref())
            .await?;
        out.insert("last_poster".into(), last_poster);
        // The plugins' keys.
        let solved = self.solved_view(&topic, extra.deleted_at.is_some()).await?;
        // discourse-solved's accepted answer keys are prepended to
        // TopicViewSerializer after the web hook serializer took its
        // attributes; only its add_to_serializer keys reach it.
        if let Some(solved) = &solved {
            solved
                .shared_issue_keys(&mut *self.conn, self.settings, self.guardian, &mut out)
                .await?;
        }
        if crate::plugins::reactions::enabled(self.settings)? {
            out.insert(
                "valid_reactions".into(),
                crate::plugins::reactions::Reactions::load(self.settings)?.valid_json(),
            );
        }
        if let Some(can_vote) = self.topic_can_vote(&topic).await? {
            crate::plugins::topic_voting::topic_view_keys(
                &mut *self.conn,
                self.guardian,
                topic.id,
                can_vote,
                &mut out,
            )
            .await?;
        }
        if self.guardian.is_authenticated() && viewer.queue_enabled {
            let pending: Vec<(i64, Option<String>, chrono::NaiveDateTime)> = sqlx::query_as(
                "SELECT id, payload->>'raw', created_at FROM reviewables \
                 WHERE type = 'ReviewableQueuedPost' AND status = 0 \
                 AND target_created_by_id = $1 AND topic_id = $2 ORDER BY created_at ASC",
            )
            .bind(uid)
            .bind(topic.id)
            .fetch_all(&mut *self.conn)
            .await?;
            let pending: Vec<Value> = pending
                .into_iter()
                .map(|(id, raw, created_at)| {
                    json!({"id": id, "raw": raw, "created_at": time_json(created_at)})
                })
                .collect();
            out.insert("pending_posts".into(), Value::Array(pending));
        }
        Ok(Value::Object(out))
    }
}
