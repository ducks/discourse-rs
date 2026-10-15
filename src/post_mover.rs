//! Port of app/models/post_mover.rb (with Topic#move_posts): posts moved
//! from one topic to a new topic (splitting) or an existing one
//! (merging), sequentially or in chronological order: the posts renumbered
//! into the destination, moved_posts, the references that follow them
//! (notifications, reply counts and links, quotes, timings, topic users,
//! incoming emails, bookmarks, reviewables, user actions), the split_topic
//! small action left where they were, both topics' statistics, a full
//! move closing the original and scheduling its deletion, and the jobs
//! after it.
//!
//! A first post is not moved but copied, as PostCreator writes a reply as
//! its author (`create_first_post`).
//!
//! Refused: moving to or between messages (the whisper left behind and
//! the participants), tags on the new topic (189), a
//! copied first post with polls or reactions, votes moving with a merge.

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::revise_topic::Deferred;
use crate::posting::small_action::{self, SmallAction};
use crate::posting::text::{EmojiEscape, TitleOptions, clean_title, fancy_title, slug_for};
use crate::posting::validate::{self, Persisted, TopicInput};
use crate::posting::{
    Ctx, TopicUserAttr, change_topic_user, notification_levels, notification_reasons, post_types,
};
use crate::topic_guardian::TopicCtx;
use crate::{AppError, Unsupported};

/// What TopicsController#move_posts and #merge_topic hand Topic#move_posts.
#[derive(Debug, Default, Clone)]
pub struct MoveArgs {
    pub post_ids: Vec<i32>,
    pub title: Option<String>,
    pub destination_topic_id: Option<i32>,
    pub category_id: Option<i32>,
    pub chronological_order: bool,
    pub freeze_original: bool,
    /// `archetype: "private_message"`
    pub move_to_pm: bool,
    pub participants: Option<String>,
    /// `tag_ids` or `tags` for the new topic.
    pub tags: bool,
}

/// How a move ends.
pub enum Moved {
    /// The destination topic: `{success: true, url:}`.
    Done(i32),
    /// Neither a destination nor a title: `{success: false}`.
    Nothing,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// `Discourse::InvalidParameters`
    InvalidParameters(&'static str),
    /// `ActiveRecord::RecordInvalid` creating the new topic.
    Invalid(Vec<String>),
}

#[derive(sqlx::FromRow, Clone)]
struct MovePost {
    id: i32,
    topic_id: i32,
    post_number: i32,
    user_id: Option<i32>,
    created_at: NaiveDateTime,
    reply_to_post_number: Option<i32>,
    raw: String,
    cook_method: i32,
    via_email: bool,
    raw_email: Option<String>,
    version: i32,
    public_version: i32,
    locale: Option<String>,
}

#[derive(sqlx::FromRow, Clone)]
struct Dest {
    id: i32,
    title: String,
    category_id: Option<i32>,
    highest_post_number: i32,
    highest_staff_post_number: i32,
}

/// The state of one move as PostMover's instance variables hold it.
struct Mover<'a, 'b> {
    ctx: &'a Ctx<'b>,
    guardian: &'a Guardian,
    user_id: i32,
    original_id: i32,
    original_title: String,
    original_category_id: Option<i32>,
    original_archetype: String,
    creating_new_topic: bool,
    chronological: bool,
    /// The ids the moved posts have afterwards (a copied first post's
    /// copy in place of the original).
    post_ids_after_move: Vec<i32>,
    deferred: Deferred,
    /// The jobs `@post_creator.enqueue_jobs` sends for a copied first
    /// post, after the move.
    creator_jobs: Vec<(&'static str, Value, Option<i64>)>,
    /// `freeze_original`: the posts are copied, the original left closed.
    freeze: bool,
    /// The ids asked for (`@post_ids`).
    post_ids: Vec<i32>,
}

/// `Topic#move_posts(moved_by, post_ids, opts)`
pub async fn move_posts(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    original_id: i32,
    args: &MoveArgs,
) -> Result<Moved, AppError> {
    let user_id = guardian
        .user_id()
        .ok_or(Unsupported("moving posts anonymously"))?;
    if args.move_to_pm {
        return Err(Unsupported("moving posts to a message (its whisper and participants)").into());
    }
    if ctx.config.rate_limits && args.destination_topic_id.is_none() && args.title.is_some() {
        return Err(Unsupported("the topic rate limits of a split's new topic").into());
    }
    let (original_title, original_category_id, original_archetype): (String, Option<i32>, String) =
        sqlx::query_as("SELECT title, category_id, archetype FROM topics WHERE id = $1")
            .bind(original_id)
            .fetch_one(pool)
            .await?;
    let mut mover = Mover {
        ctx,
        guardian,
        user_id,
        original_id,
        original_title,
        original_category_id,
        original_archetype,
        creating_new_topic: false,
        chronological: args.chronological_order,
        post_ids_after_move: args.post_ids.clone(),
        deferred: Deferred::default(),
        creator_jobs: Vec::new(),
        freeze: args.freeze_original,
        post_ids: args.post_ids.clone(),
    };
    if let Some(destination_id) = args.destination_topic_id {
        mover.move_to_topic(pool, destination_id).await
    } else if let Some(title) = &args.title {
        if args.tags {
            return Err(Unsupported("tags on a split's new topic (189)").into());
        }
        mover.move_to_new_topic(pool, title, args.category_id).await
    } else {
        Ok(Moved::Nothing)
    }
}

impl Mover<'_, '_> {
    /// `PostMover#to_topic`
    async fn move_to_topic(
        &mut self,
        pool: &PgPool,
        destination_id: i32,
    ) -> Result<Moved, AppError> {
        let s = self.ctx.settings;
        let mut conn = pool.acquire().await?;
        // ensure_can_create_post_on_topic!(Topic.find_by_id(id))
        let Some(topic) = TopicCtx::load(&mut conn, s, self.guardian, destination_id).await? else {
            return Ok(Moved::Forbidden);
        };
        let can_post_anywhere = self.guardian.can_create_post_anywhere(&mut conn, s).await?;
        if !self
            .guardian
            .can_create_post_on_topic(s, &topic, can_post_anywhere)?
        {
            return Ok(Moved::Forbidden);
        }
        if topic.archetype != self.original_archetype
            && (topic.private_message() || self.original_archetype == "private_message")
        {
            return Ok(Moved::InvalidParameters("Discourse::InvalidParameters"));
        }
        if topic.private_message() {
            return Err(Unsupported("moving posts into a message").into());
        }
        self.topic_merged(&mut conn).await?;
        drop(conn);
        let mut tx = pool.begin().await?;
        let dest = load_dest(&mut tx, destination_id).await?;
        if let Some(refused) = self.move_posts_to(&mut tx, &dest).await? {
            tx.rollback().await?;
            return Ok(refused);
        }
        let deferred = std::mem::take(&mut self.deferred);
        deferred.enqueue(&mut tx).await?;
        tx.commit().await?;
        let mut conn = pool.acquire().await?;
        self.enqueue_jobs(&mut conn, destination_id).await?;
        Ok(Moved::Done(destination_id))
    }

    /// `PostMover#to_new_topic`
    async fn move_to_new_topic(
        &mut self,
        pool: &PgPool,
        title: &str,
        category_id: Option<i32>,
    ) -> Result<Moved, AppError> {
        self.creating_new_topic = true;
        let first_id = self.post_ids_after_move.first().copied();
        let post: Option<(Option<i32>, NaiveDateTime)> =
            match first_id {
                Some(id) => sqlx::query_as(
                    "SELECT user_id, created_at FROM posts WHERE id = $1 AND deleted_at IS NULL",
                )
                .bind(id)
                .fetch_optional(pool)
                .await?,
                None => None,
            };
        let Some((author_id, created_at)) = post else {
            return Ok(Moved::InvalidParameters("Discourse::InvalidParameters"));
        };
        let mut tx = pool.begin().await?;
        let topic_id = match self
            .create_topic(&mut tx, title, category_id, author_id, created_at)
            .await?
        {
            Ok(id) => id,
            Err(errors) => {
                tx.rollback().await?;
                return Ok(Moved::Invalid(errors));
            }
        };
        let dest = load_dest(&mut tx, topic_id).await?;
        if let Some(refused) = self.move_posts_to(&mut tx, &dest).await? {
            tx.rollback().await?;
            return Ok(refused);
        }
        self.watch_new_topic(&mut tx, topic_id).await?;
        // update_topic_excerpt: the new first post's excerpt.
        let cooked: Option<String> = sqlx::query_scalar(
            "SELECT cooked FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *tx)
        .await?;
        let excerpt = cooked.map(|c| {
            crate::excerpt::excerpt(
                &c,
                self.ctx
                    .settings
                    .get("topic_excerpt_maxlength")
                    .map(|v| v.to_i().max(0) as usize)
                    .unwrap_or(220),
                &crate::excerpt::Options {
                    strip_links: true,
                    image_mode: crate::excerpt::ImageMode::Strip,
                    ..Default::default()
                },
            )
        });
        sqlx::query("UPDATE topics SET excerpt = $2 WHERE id = $1")
            .bind(topic_id)
            .bind(excerpt)
            .execute(&mut *tx)
            .await?;
        let deferred = std::mem::take(&mut self.deferred);
        deferred.enqueue(&mut tx).await?;
        tx.commit().await?;
        let mut conn = pool.acquire().await?;
        self.enqueue_jobs(&mut conn, topic_id).await?;
        // DiscourseEvent :topic_created: its web hooks.
        crate::web_hooks::enqueue_topic_hooks(
            &mut conn,
            &crate::web_hooks::Payloads {
                settings: self.ctx.settings,
                i18n: self.ctx.i18n,
                config: self.ctx.config,
            },
            "topic_created",
            topic_id,
            None,
        )
        .await?;
        Ok(Moved::Done(topic_id))
    }
}

async fn load_dest(conn: &mut PgConnection, topic_id: i32) -> Result<Dest, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, title, category_id, highest_post_number, \
                highest_staff_post_number FROM topics WHERE id = $1",
    )
    .bind(topic_id)
    .fetch_one(conn)
    .await
}

impl Mover<'_, '_> {
    /// `Topic.create!(user: post.user, acting_user:, title:, category_id:,
    /// created_at: post.created_at, archetype:)`: the topic's validations
    /// and callbacks; Err with its errors when it isn't valid.
    async fn create_topic(
        &self,
        conn: &mut PgConnection,
        title: &str,
        category_id: Option<i32>,
        author_id: Option<i32>,
        created_at: NaiveDateTime,
    ) -> Result<Result<i32, Vec<String>>, AppError> {
        let ctx = self.ctx;
        let s = ctx.settings;
        let author_id = author_id.ok_or(Unsupported("splitting off a post without its author"))?;
        let title = clean_title(
            title,
            &TitleOptions {
                prettify: s.get("title_prettify")?.truthy(),
                allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
                remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
            },
        );
        let author_admin: bool =
            sqlx::query_scalar("SELECT COALESCE((SELECT admin FROM users WHERE id = $1), FALSE)")
                .bind(author_id)
                .fetch_one(&mut *conn)
                .await?;
        let errors = validate::validate_topic_with(
            &mut *conn,
            ctx,
            self.guardian,
            &TopicInput {
                title: &title,
                category_id,
                private_message: false,
            },
            Some(&Persisted {
                topic_id: 0,
                author_admin,
                check_title: true,
                check_category: true,
            }),
        )
        .await?;
        if !errors.is_empty() {
            return Ok(Err(errors));
        }
        if s.get("slug_generation_method")?.to_s() != "ascii" {
            return Err(Unsupported("slug_generation_method other than ascii").into());
        }
        let category_id = category_id.unwrap_or(s.get("uncategorized_category_id")?.to_i() as i32);
        let slug = slug_for(&title, &s.get("default_locale")?.to_s())?;
        let fancy = fancy_title(&title, EmojiEscape::from_settings(s)?)?;
        let slow_mode: Option<i32> =
            sqlx::query_scalar("SELECT default_slow_mode_seconds FROM categories WHERE id = $1")
                .bind(category_id)
                .fetch_optional(&mut *conn)
                .await?
                .flatten();
        let topic_id: i32 = sqlx::query_scalar(
            "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, category_id, \
                                 archetype, created_at, updated_at, bumped_at, slow_mode_seconds) \
             VALUES ($1, $2, $3, $4, $4, $5, 'regular', $6, clock_timestamp(), clock_timestamp(), \
                     COALESCE($7, 0)) \
             RETURNING id",
        )
        .bind(&title)
        .bind(&fancy)
        .bind(&slug)
        .bind(author_id)
        .bind(category_id)
        .bind(created_at)
        .bind(slow_mode)
        .fetch_one(&mut *conn)
        .await?;
        crate::topic_timer::inherit_auto_close_from_category(
            &mut *conn,
            ctx,
            topic_id,
            crate::topic_timer::types::CLOSE,
        )
        .await?;
        // after_create: changed_to_category(category).
        let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
                .bind(category_id)
                .fetch_one(&mut *conn)
                .await?;
        if exists && (category_id != uncategorized || s.get("allow_uncategorized_topics")?.truthy())
        {
            sqlx::query("UPDATE categories SET topic_count = topic_count + 1 WHERE id = $1")
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
            crate::posting::create::feature_topics_for(&mut *conn, s, category_id, None).await?;
        }
        Ok(Ok(topic_id))
    }

    /// `PostMover#move_posts_to(topic)`. Some when the move is refused.
    async fn move_posts_to(
        &mut self,
        conn: &mut PgConnection,
        dest: &Dest,
    ) -> Result<Option<Moved>, AppError> {
        let s = self.ctx.settings;
        // Guardian.new(user).ensure_can_see!(topic)
        let topic_ctx = TopicCtx::load(&mut *conn, s, self.guardian, dest.id)
            .await?
            .ok_or(Unsupported("moving posts to a missing topic"))?;
        let secure = self.guardian.secure_category_ids(&mut *conn, s).await?;
        if !self.guardian.can_see_topic(s, &topic_ctx, true, &secure)? {
            return Ok(Some(Moved::Forbidden));
        }
        sqlx::query("SELECT id FROM topics WHERE id = $1 FOR UPDATE")
            .bind(self.original_id)
            .execute(&mut *conn)
            .await?;
        // posts: by the given ids, no small actions, no empty raw, by
        // creation.
        let posts: Vec<MovePost> = sqlx::query_as(
            "SELECT id, topic_id, post_number, user_id, created_at, reply_to_post_number, raw, \
                    cook_method, via_email, raw_email, version, public_version, locale \
             FROM posts WHERE deleted_at IS NULL AND topic_id = $1 AND id = ANY($2) \
               AND post_type <> $3 AND raw <> '' ORDER BY created_at",
        )
        .bind(self.original_id)
        .bind(&self.post_ids_after_move)
        .bind(post_types::SMALL_ACTION)
        .fetch_all(&mut *conn)
        .await?;
        if posts.is_empty() {
            return Ok(Some(Moved::InvalidParameters("post_ids")));
        }
        let preventing: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE deleted_at IS NULL AND topic_id = $1 AND post_type IN ($2, $3) \
               AND raw <> '' AND action_code IS DISTINCT FROM 'split_topic'",
        )
        .bind(self.original_id)
        .bind(post_types::REGULAR)
        .bind(post_types::WHISPER)
        .fetch_all(&mut *conn)
        .await?;
        let moving: Vec<i32> = posts.iter().map(|p| p.id).collect();
        let full_move = preventing.iter().all(|id| moving.contains(id));
        let mut first_post_number_moved = if posts[0].post_number == 1 {
            posts.get(1).map(|p| p.post_number)
        } else {
            Some(posts[0].post_number)
        };
        if self.freeze {
            // The small action goes after the copied posts: past every post
            // (deleted ones too) for a full move, else right after the last
            // copied one, the posts after it shifted up one.
            let last = posts[posts.len() - 1].post_number;
            first_post_number_moved = Some(if full_move {
                max_post_number(conn, self.original_id).await? + 1
            } else {
                let after: Vec<(i32, i32)> = sqlx::query_as(
                    "SELECT id, post_number FROM posts WHERE topic_id = $1 AND post_number > $2 \
                     ORDER BY post_number DESC",
                )
                .bind(self.original_id)
                .bind(last)
                .fetch_all(&mut *conn)
                .await?;
                for (id, number) in after {
                    sqlx::query("UPDATE posts SET post_number = $2 WHERE id = $1")
                        .bind(id)
                        .bind(number + 1)
                        .execute(&mut *conn)
                        .await?;
                }
                last + 1
            });
        }

        sqlx::query(
            "CREATE TEMPORARY TABLE temp_moved_posts (old_topic_id INTEGER, old_post_id INTEGER, \
               old_post_number INTEGER, new_topic_id INTEGER, new_topic_title VARCHAR, \
               new_post_id INTEGER, new_post_number INTEGER) ON COMMIT DROP",
        )
        .execute(&mut *conn)
        .await?;
        if self.chronological {
            self.move_each_post_chronological(conn, dest, &posts, full_move)
                .await?;
        } else {
            self.move_each_post_sequential(conn, dest, &posts, full_move)
                .await?;
        }
        self.handle_moved_references(conn, dest).await?;
        self.create_moderator_post(conn, dest, &posts, first_post_number_moved)
            .await?;
        // update_statistics
        update_statistics(conn, dest.id).await?;
        update_statistics(conn, self.original_id).await?;
        update_post_action_cache(conn, &[self.original_id, dest.id], &self.post_ids).await?;
        // update_user_actions
        synchronize_target_topic_ids(conn, &moving).await?;
        self.update_last_post_stats(conn, dest.id).await?;
        self.deferred
            .push("update_topic_upload_security", json!({"topic_id": dest.id}));
        self.deferred.push(
            "sync_topic_user_bookmarked",
            json!({"topic_id": self.original_id}),
        );
        self.deferred
            .push("sync_topic_user_bookmarked", json!({"topic_id": dest.id}));
        // update_reviewables
        sqlx::query(
            "UPDATE reviewables SET topic_id = $1, category_id = $2 \
             WHERE target_type = 'Post' AND target_id = ANY($3) AND topic_id = $4 \
               AND category_id IS NOT DISTINCT FROM $5",
        )
        .bind(dest.id)
        .bind(dest.category_id)
        .bind(&moving)
        .bind(self.original_id)
        .bind(self.original_category_id)
        .execute(&mut *conn)
        .await?;
        if full_move {
            self.close_topic_and_schedule_deletion(conn).await?;
        }
        // The moved posts' after_commit search index (their topic
        // changed).
        let ctx = self.ctx;
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        let base = crate::posting::search_index::BaseUrls {
            base_path: ctx.config.globals.relative_url_root(),
            base_url_no_prefix: urls.base_url_no_prefix()?,
            store: crate::file_store::FileStore::for_site(ctx.config, s)?,
        };
        let reindex: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM posts WHERE id = ANY($1) AND topic_id = $2 AND raw <> '' ORDER BY id",
        )
        .bind(&self.post_ids_after_move)
        .bind(dest.id)
        .fetch_all(&mut *conn)
        .await?;
        for post_id in reindex {
            crate::posting::search_index::reindex_post(&mut *conn, s, &base, post_id).await?;
        }
        Ok(None)
    }

    async fn move_each_post_sequential(
        &mut self,
        conn: &mut PgConnection,
        dest: &Dest,
        posts: &[MovePost],
        full_move: bool,
    ) -> Result<(), AppError> {
        let max_post_number = max_post_number(conn, dest.id).await? + 1;
        let mut move_map: Vec<(i32, i32)> = Vec::new();
        let mut reply_count: Vec<(i32, i32)> = Vec::new();
        for (offset, post) in posts.iter().enumerate() {
            move_map.push((post.post_number, offset as i32 + max_post_number));
            if let Some(n) = post.reply_to_post_number {
                bump(&mut reply_count, n);
            }
        }
        for post in posts {
            let new_number = lookup(&move_map, post.post_number).unwrap_or(post.post_number);
            let new_id = if post.post_number == 1 {
                self.create_first_post(conn, dest, post, new_number, &reply_count)
                    .await?
            } else {
                self.move_post(conn, dest, post, &move_map, &reply_count)
                    .await?
            };
            self.store_movement(conn, dest, post, new_id, new_number, full_move)
                .await?;
        }
        Ok(())
    }

    async fn move_each_post_chronological(
        &mut self,
        conn: &mut PgConnection,
        dest: &Dest,
        posts: &[MovePost],
        full_move: bool,
    ) -> Result<(), AppError> {
        // The destination's posts, deleted ones too, by number.
        let destination_posts: Vec<MovePost> = sqlx::query_as(
            "SELECT id, topic_id, post_number, user_id, created_at, reply_to_post_number, raw, \
                    cook_method, via_email, raw_email, version, public_version, locale \
             FROM posts WHERE topic_id = $1 ORDER BY post_number",
        )
        .bind(dest.id)
        .fetch_all(&mut *conn)
        .await?;
        let min_created_at = posts[0].created_at;
        let mut moved_posts: Vec<MovePost> = destination_posts
            .into_iter()
            .skip_while(|p| p.created_at <= min_created_at)
            .collect();
        let initial_post_number = match moved_posts.first() {
            Some(p) => p.post_number,
            None => max_post_number(conn, dest.id).await? + 1,
        };
        let mut last_index = 0;
        for post in posts {
            while last_index < moved_posts.len()
                && moved_posts[last_index].created_at <= post.created_at
            {
                last_index += 1;
            }
            moved_posts.insert(last_index, post.clone());
        }
        let mut move_map: Vec<(i32, i32)> = Vec::new();
        let mut shift_map: Vec<(i32, i32)> = Vec::new();
        let mut reply_count: Vec<(i32, i32)> = Vec::new();
        let mut next_post_number = initial_post_number;
        for post in &moved_posts {
            if post.topic_id == dest.id {
                if post.post_number > next_post_number {
                    next_post_number = post.post_number;
                }
                shift_map.push((post.post_number, next_post_number));
            } else {
                move_map.push((post.post_number, next_post_number));
                if let Some(n) = post.reply_to_post_number {
                    bump(&mut reply_count, n);
                }
            }
            next_post_number += 1;
        }
        for post in moved_posts.iter().rev() {
            if post.topic_id == dest.id {
                let new_number = lookup(&shift_map, post.post_number).unwrap_or(post.post_number);
                // move_same_topic
                let reply_to = post
                    .reply_to_post_number
                    .and_then(|n| lookup(&shift_map, n))
                    .or(post.reply_to_post_number);
                sqlx::query(
                    "UPDATE posts SET post_number = $2, sort_order = $2, baked_version = NULL, \
                            reply_to_post_number = $3, updated_at = clock_timestamp() \
                     WHERE id = $1 AND (post_number <> $2 OR sort_order IS DISTINCT FROM $2 \
                       OR baked_version IS NOT NULL OR reply_to_post_number IS DISTINCT FROM $3)",
                )
                .bind(post.id)
                .bind(new_number)
                .bind(reply_to)
                .execute(&mut *conn)
                .await?;
                self.store_movement(conn, dest, post, post.id, new_number, full_move)
                    .await?;
            } else {
                let new_number = lookup(&move_map, post.post_number).unwrap_or(post.post_number);
                let new_id = if post.post_number == 1 {
                    self.create_first_post(conn, dest, post, new_number, &reply_count)
                        .await?
                } else {
                    self.move_post(conn, dest, post, &move_map, &reply_count)
                        .await?
                };
                self.store_movement(conn, dest, post, new_id, new_number, full_move)
                    .await?;
            }
        }
        if initial_post_number == 1 {
            sqlx::query("UPDATE topics SET user_id = $2 WHERE id = $1")
                .bind(dest.id)
                .bind(posts[0].user_id)
                .execute(&mut *conn)
                .await?;
        }
        Ok(())
    }

    /// `PostMover#move(post)`: the post renumbered into the destination,
    /// its links along with it.
    async fn move_post(
        &mut self,
        conn: &mut PgConnection,
        dest: &Dest,
        post: &MovePost,
        move_map: &[(i32, i32)],
        reply_count: &[(i32, i32)],
    ) -> Result<i32, AppError> {
        let new_number = lookup(move_map, post.post_number).unwrap_or(post.post_number);
        let reply_to = post.reply_to_post_number.and_then(|n| lookup(move_map, n));
        if self.freeze {
            // post.dup saved: a new row with the post's attributes (fresh
            // timestamps), placed in the destination, cooked as it was and
            // marked baked by before_save.
            let copy: i32 = sqlx::query_scalar(
                "INSERT INTO posts (user_id, topic_id, post_number, raw, cooked, created_at, updated_at, \
                   reply_to_post_number, reply_count, quote_count, deleted_at, off_topic_count, like_count, \
                   incoming_link_count, bookmark_count, score, reads, post_type, sort_order, last_editor_id, \
                   hidden, hidden_reason_id, notify_moderators_count, spam_count, illegal_count, \
                   inappropriate_count, last_version_at, user_deleted, reply_to_user_id, percent_rank, \
                   notify_user_count, like_score, deleted_by_id, edit_reason, word_count, version, \
                   cook_method, wiki, baked_at, baked_version, hidden_at, self_edits, reply_quoted, \
                   via_email, raw_email, public_version, action_code, locked_by_id, image_upload_id, \
                   qa_vote_count, outbound_message_id, locale) \
                 SELECT user_id, $5, $3, raw, cooked, clock_timestamp(), clock_timestamp(), \
                   $4, $2, quote_count, deleted_at, off_topic_count, like_count, \
                   incoming_link_count, bookmark_count, score, reads, post_type, $3, last_editor_id, \
                   hidden, hidden_reason_id, notify_moderators_count, spam_count, illegal_count, \
                   inappropriate_count, last_version_at, user_deleted, \
                   CASE WHEN $4 IS NULL THEN NULL ELSE reply_to_user_id END, percent_rank, \
                   notify_user_count, like_score, deleted_by_id, edit_reason, word_count, version, \
                   cook_method, wiki, clock_timestamp(), $6, hidden_at, self_edits, reply_quoted, \
                   via_email, raw_email, public_version, action_code, locked_by_id, image_upload_id, \
                   qa_vote_count, outbound_message_id, locale \
                 FROM posts WHERE id = $1 RETURNING id",
            )
            .bind(post.id)
            .bind(lookup(reply_count, post.post_number).unwrap_or(0))
            .bind(new_number)
            .bind(reply_to)
            .bind(dest.id)
            .bind(crate::posting::BAKED_VERSION)
            .fetch_one(&mut *conn)
            .await?;
            self.post_ids_after_move = self
                .post_ids_after_move
                .iter()
                .map(|id| if *id == post.id { copy } else { *id })
                .collect();
            self.post_moved(conn, copy, post.id).await?;
            return Ok(copy);
        }
        sqlx::query(
            "UPDATE posts SET reply_count = $2, post_number = $3, reply_to_post_number = $4, \
                    topic_id = $5, sort_order = $3, baked_version = NULL, \
                    reply_to_user_id = CASE WHEN $4 IS NULL THEN NULL ELSE reply_to_user_id END, \
                    updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(post.id)
        .bind(lookup(reply_count, post.post_number).unwrap_or(0))
        .bind(new_number)
        .bind(reply_to)
        .bind(dest.id)
        .execute(&mut *conn)
        .await?;
        self.post_moved(conn, post.id, post.id).await?;
        sqlx::query("UPDATE topic_links SET topic_id = $2 WHERE post_id = $1")
            .bind(post.id)
            .bind(dest.id)
            .execute(&mut *conn)
            .await?;
        Ok(post.id)
    }

    /// DiscourseEvent `:post_moved` to the plugins listening: a post
    /// whose id changed takes its polls along (discourse-poll).
    async fn post_moved(
        &self,
        conn: &mut PgConnection,
        new_id: i32,
        old_id: i32,
    ) -> Result<(), AppError> {
        if new_id != old_id {
            let polls: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM polls WHERE post_id = $1)")
                    .bind(i64::from(old_id))
                    .fetch_one(&mut *conn)
                    .await?;
            if polls {
                return Err(Unsupported("moving a copied post's polls").into());
            }
        }
        Ok(())
    }

    /// `store_movement`: the temporary table PostMover's SQL reads and the
    /// moved_posts row.
    async fn store_movement(
        &self,
        conn: &mut PgConnection,
        dest: &Dest,
        post: &MovePost,
        new_post_id: i32,
        new_post_number: i32,
        full_move: bool,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO temp_moved_posts (old_topic_id, old_post_id, old_post_number, new_topic_id, \
                                           new_topic_title, new_post_id, new_post_number) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(post.topic_id)
        .bind(post.id)
        .bind(post.post_number)
        .bind(dest.id)
        .bind(&dest.title)
        .bind(new_post_id)
        .bind(new_post_number)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO moved_posts (old_topic_id, old_topic_title, old_post_id, old_post_number, \
                                      post_user_id, user_id, full_move, new_topic_id, new_topic_title, \
                                      new_post_id, new_post_number, created_new_topic, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, clock_timestamp(), clock_timestamp())",
        )
        .bind(post.topic_id)
        .bind(&self.original_title)
        .bind(post.id)
        .bind(post.post_number)
        .bind(post.user_id)
        .bind(self.user_id)
        .bind(full_move)
        .bind(dest.id)
        .bind(&dest.title)
        .bind(new_post_id)
        .bind(new_post_number)
        .bind(self.creating_new_topic)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }
}

fn lookup(map: &[(i32, i32)], key: i32) -> Option<i32> {
    map.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn bump(map: &mut Vec<(i32, i32)>, key: i32) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some((_, v)) => *v += 1,
        None => map.push((key, 1)),
    }
}

/// `topic.max_post_number`: deleted posts count.
async fn max_post_number(conn: &mut PgConnection, topic_id: i32) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(MAX(post_number), 0) FROM posts WHERE topic_id = $1")
        .bind(topic_id)
        .fetch_one(conn)
        .await
}

/// `Topic#update_statistics!`: reset_highest, the featured users, the
/// like count.
async fn update_statistics(conn: &mut PgConnection, topic_id: i32) -> Result<(), AppError> {
    crate::post_destroyer::reset_highest(&mut *conn, topic_id).await?;
    crate::jobs::handlers::choose_featured_users(&mut *conn, topic_id).await?;
    let og: Option<i64> = sqlx::query_scalar("SELECT og_image_upload_id FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if og.is_some() {
        return Err(Unsupported("og image regeneration after a move's like count").into());
    }
    sqlx::query(
        "UPDATE topics SET like_count = (SELECT COALESCE(SUM(like_count), 0) FROM posts \
           WHERE topic_id = $1 AND deleted_at IS NULL AND post_type <> $2) WHERE id = $1",
    )
    .bind(topic_id)
    .bind(post_types::WHISPER)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `TopicUser.update_post_action_cache(topic_id:, post_id:)` for likes.
async fn update_post_action_cache(
    conn: &mut PgConnection,
    topic_ids: &[i32],
    post_ids: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topic_users tu SET liked = x.state \
         FROM (SELECT CASE WHEN EXISTS (SELECT 1 FROM post_actions pa \
                 JOIN posts p ON p.id = pa.post_id JOIN topics t ON t.id = p.topic_id \
                 WHERE pa.deleted_at IS NULL AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
                   AND pa.post_action_type_id = 2 AND tu2.topic_id = t.id AND tu2.user_id = pa.user_id \
                 LIMIT 1) THEN true ELSE false END state, tu2.topic_id, tu2.user_id \
               FROM topic_users tu2 \
               WHERE tu2.topic_id = ANY($1) \
                 AND tu2.user_id IN (SELECT user_id FROM post_actions WHERE post_id = ANY($2) \
                                     AND post_action_type_id = 2)) x \
         WHERE x.topic_id = tu.topic_id AND x.user_id = tu.user_id AND x.state != tu.liked",
    )
    .bind(topic_ids)
    .bind(post_ids)
    .execute(conn)
    .await?;
    Ok(())
}

/// `UserAction.synchronize_target_topic_ids(post_ids)`: duplicates gone,
/// then each action's topic from its post.
async fn synchronize_target_topic_ids(
    conn: &mut PgConnection,
    post_ids: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM user_actions USING user_actions ua2 \
         WHERE user_actions.action_type = ua2.action_type AND user_actions.user_id = ua2.user_id \
           AND user_actions.acting_user_id = ua2.acting_user_id \
           AND user_actions.target_post_id = ua2.target_post_id AND user_actions.target_post_id > 0 \
           AND user_actions.id > ua2.id AND user_actions.target_post_id = ANY($1)",
    )
    .bind(post_ids)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE user_actions SET target_topic_id = (SELECT topic_id FROM posts WHERE posts.id = target_post_id) \
         WHERE target_topic_id <> (SELECT topic_id FROM posts WHERE posts.id = target_post_id) \
           AND target_post_id = ANY($1)",
    )
    .bind(post_ids)
    .execute(conn)
    .await?;
    Ok(())
}

impl Mover<'_, '_> {
    /// `create_first_post`: a first post isn't moved; PostCreator writes a
    /// copy into the destination as its author (no validations, no bump,
    /// its jobs held until the move is done), which takes the original's
    /// actions, revisions, custom fields, email logs and bookmarks.
    async fn create_first_post(
        &mut self,
        conn: &mut PgConnection,
        dest: &Dest,
        post: &MovePost,
        new_number: i32,
        reply_count: &[(i32, i32)],
    ) -> Result<i32, AppError> {
        let ctx = self.ctx;
        let s = ctx.settings;
        let author = post
            .user_id
            .ok_or(Unsupported("copying a first post without its author"))?;
        if post.cook_method != 1 {
            return Err(Unsupported("copying a first post with another cook method").into());
        }
        let reactions: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM discourse_reactions_reactions WHERE post_id = $1)",
        )
        .bind(post.id)
        .fetch_one(&mut *conn)
        .await?;
        if reactions {
            return Err(
                Unsupported("copying a first post's reactions (discourse-reactions)").into(),
            );
        }
        let (staged, author_admin_or_mod): (bool, bool) =
            sqlx::query_as("SELECT staged, admin OR moderator FROM users WHERE id = $1")
                .bind(author)
                .fetch_one(&mut *conn)
                .await?;
        let _ = author_admin_or_mod;
        let topic_id = dest.id;
        let cooked = crate::pretty_text::cook(
            ctx.host,
            &post.raw,
            &crate::pretty_text::MarkdownOptions {
                topic_id: Some(i64::from(topic_id)),
                post_id: None,
                user_id: Some(i64::from(author)),
                force_quote_link: false,
                omit_nofollow: false,
                chat: false,
            },
        )
        .await?;
        let base_path = ctx.config.globals.relative_url_root().to_string();
        let analysis = validate::analyze(&cooked, &base_path)?;
        if analysis.has_quotes {
            return Err(Unsupported("copying a first post with quotes (QuotedPost)").into());
        }
        // build_post_stats
        let draft_key = format!("topic_{topic_id}");
        let sequence =
            crate::posting::current_draft_sequence(&mut *conn, author, &draft_key).await?;
        let drafts_saved: i32 = sqlx::query_scalar(
            "SELECT revisions FROM drafts WHERE sequence = $1 AND user_id = $2 AND draft_key = $3 LIMIT 1",
        )
        .bind(sequence)
        .bind(author)
        .bind(&draft_key)
        .fetch_optional(&mut *conn)
        .await?
        .unwrap_or(0);
        let notice = crate::posting::create::post_notice(&mut *conn, ctx, author).await?;
        // save_post: Topic.next_post_number.
        let words = crate::posting::text::word_count(&post.raw);
        let highest: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(post_number), 0) FROM posts WHERE topic_id = $1",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let post_number: i32 = sqlx::query_scalar(
            "UPDATE topics SET highest_staff_post_number = $1 + 1, highest_post_number = $1 + 1, \
             posts_count = posts_count + 1 WHERE id = $2 RETURNING highest_post_number",
        )
        .bind(highest)
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        let (new_id, created_at): (i32, NaiveDateTime) = sqlx::query_as(
            "INSERT INTO posts (user_id, topic_id, post_number, raw, cooked, created_at, updated_at, \
                                last_editor_id, word_count, sort_order, last_version_at, baked_at, \
                                baked_version, wiki, quote_count, via_email, raw_email, cook_method) \
             VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp(), $1, $7, $3, clock_timestamp(), \
                     clock_timestamp(), $8, FALSE, 0, $9, $10, $11) \
             RETURNING id, created_at",
        )
        .bind(author)
        .bind(topic_id)
        .bind(post_number)
        .bind(&post.raw)
        .bind(&cooked)
        .bind(post.created_at)
        .bind(words)
        .bind(crate::posting::BAKED_VERSION)
        .bind(post.via_email)
        .bind(&post.raw_email)
        .bind(post.cook_method)
        .fetch_one(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO post_stats (post_id, drafts_saved, typing_duration_msecs, \
                                     composer_open_duration_msecs, created_at, updated_at) \
             VALUES ($1, $2, 0, 0, clock_timestamp(), clock_timestamp())",
        )
        .bind(new_id)
        .bind(drafts_saved)
        .execute(&mut *conn)
        .await?;
        if let Some(notice) = notice {
            sqlx::query(
                "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
                 VALUES ($1, 'notice', $2, clock_timestamp(), clock_timestamp())",
            )
            .bind(new_id)
            .bind(notice.to_string())
            .execute(&mut *conn)
            .await?;
        }
        // UserActionManager.post_created: nothing for a first post.
        if post_number != 1 {
            crate::posting::log_user_action(
                &mut *conn,
                crate::posting::user_actions::REPLY,
                author,
                topic_id,
                new_id,
                created_at,
            )
            .await?;
        }
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        let hostname = urls.current_hostname()?;
        let crawl_before = crawl_job_ids(&mut *conn).await?;
        crate::posting::links::extract_from(
            &mut *conn,
            &crate::posting::links::Site {
                hostname: &hostname,
                base_path: &base_path,
                base_url_no_prefix: &urls.base_url_no_prefix()?,
                settings: s,
            },
            &crate::posting::links::LinkPost {
                id: new_id,
                user_id: author,
                topic_id,
                cooked: &cooked,
            },
        )
        .await?;
        self.defer_new_crawl_jobs(conn, &crawl_before).await?;
        // track_topic
        change_topic_user(
            &mut *conn,
            author,
            topic_id,
            &[
                TopicUserAttr::Posted(true),
                TopicUserAttr::LastReadPostNumber(post_number),
                TopicUserAttr::LastPostedAtNow,
            ],
        )
        .await?;
        crate::posting::record_timing(&mut *conn, topic_id, author, post_number, 5000).await?;
        if staged {
            change_topic_user(
                &mut *conn,
                author,
                topic_id,
                &[TopicUserAttr::NotificationLevel(
                    notification_levels::WATCHING,
                    notification_reasons::AUTO_WATCH,
                )],
            )
            .await?;
        } else {
            let replying_level: Option<i32> = sqlx::query_scalar(
                "SELECT notification_level_when_replying FROM user_options WHERE user_id = $1",
            )
            .bind(author)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
            crate::posting::auto_notification(
                &mut *conn,
                author,
                topic_id,
                notification_reasons::CREATED_POST,
                replying_level.unwrap_or(notification_levels::TRACKING),
            )
            .await?;
        }
        // update_topic_stats, without a bump.
        let excerpt = (post_number == 1).then(|| {
            crate::excerpt::excerpt(
                &cooked,
                s.get("topic_excerpt_maxlength")
                    .map(|v| v.to_i().max(0) as usize)
                    .unwrap_or(220),
                &crate::excerpt::Options {
                    strip_links: true,
                    image_mode: crate::excerpt::ImageMode::Strip,
                    ..Default::default()
                },
            )
        });
        sqlx::query(
            "UPDATE topics SET updated_at = clock_timestamp(), last_posted_at = $2, last_post_user_id = $3, \
                    word_count = COALESCE(word_count, 0) + $4, excerpt = COALESCE($5, excerpt) WHERE id = $1",
        )
        .bind(topic_id)
        .bind(created_at)
        .bind(author)
        .bind(words)
        .bind(excerpt)
        .execute(&mut *conn)
        .await?;
        crate::topic_timer::update_topic_auto_close(&mut *conn, ctx, topic_id).await?;
        // update_user_counts
        let first = sqlx::query(
            "UPDATE user_stats SET first_post_created_at = $2 \
             WHERE user_id = $1 AND first_post_created_at IS NULL",
        )
        .bind(author)
        .bind(created_at)
        .execute(&mut *conn)
        .await?
        .rows_affected()
            > 0;
        crate::badge_granter::queue_user_change(&mut *conn, s, author).await?;
        let _ = first;
        let column = if post_number == 1 {
            "topic_count"
        } else {
            "post_count"
        };
        sqlx::query(&format!(
            "UPDATE user_stats SET {column} = {column} + 1 WHERE user_id = $1"
        ))
        .bind(author)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE users SET last_posted_at = $2, updated_at = clock_timestamp() \
             WHERE id = $1 AND last_posted_at IS DISTINCT FROM $2",
        )
        .bind(author)
        .bind(created_at)
        .execute(&mut *conn)
        .await?;
        crate::upload_references::link_post_uploads(
            &mut *conn,
            s,
            &hostname,
            &crate::file_store::FileStore::for_site(ctx.config, s)?,
            new_id,
            &cooked,
        )
        .await?;
        // delete_owned_bookmarks, then topic_users.bookmarked
        sqlx::query(
            "DELETE FROM bookmarks WHERE id IN (SELECT bookmarks.id FROM bookmarks \
               LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
               LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                                OR (topics.id = posts.topic_id) \
               WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
                 AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
                 AND bookmarks.auto_delete_preference = 2)",
        )
        .bind(author)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
        let bookmarked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM bookmarks \
               LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
               LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                                OR (topics.id = posts.topic_id) \
               WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
                 AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL)",
        )
        .bind(author)
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        change_topic_user(
            &mut *conn,
            author,
            topic_id,
            &[TopicUserAttr::Bookmarked(bookmarked)],
        )
        .await?;
        // After PostCreator's transaction: publish (replies only), the
        // category's latest, post_created's web hooks; its jobs later.
        if post_number != 1 {
            crate::bus::publish_post_change(
                ctx,
                &mut *conn,
                new_id,
                "created",
                Default::default(),
                false,
            )
            .await?;
        }
        if let Some(category_id) = dest.category_id {
            if post_number == 1 {
                sqlx::query(
                    "UPDATE categories SET latest_topic_id = $2, latest_post_id = $3 WHERE id = $1",
                )
                .bind(category_id)
                .bind(topic_id)
                .bind(new_id)
                .execute(&mut *conn)
                .await?;
            } else {
                sqlx::query("UPDATE categories SET latest_post_id = $2 WHERE id = $1")
                    .bind(category_id)
                    .bind(new_id)
                    .execute(&mut *conn)
                    .await?;
            }
        }
        crate::web_hooks::enqueue_post_hooks(
            &mut *conn,
            &crate::web_hooks::Payloads {
                settings: s,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            "post_created",
            new_id,
            crate::topic_view::Held::default(),
        )
        .await?;
        crate::badge_granter::queue(
            &mut *conn,
            s,
            crate::badge_granter::Trigger::PostRevision,
            &[new_id],
        )
        .await?;
        // PostJobsEnqueuer, sent after the move.
        self.creator_jobs.push((
            "post_alert",
            json!({"post_id": new_id, "new_record": true, "options": null}),
            None,
        ));
        self.creator_jobs
            .push(("feature_topic_users", json!({"topic_id": topic_id}), None));
        self.creator_jobs.push((
            "process_post",
            json!({"bypass_bump": false, "cooking_options": null, "new_post": true, "post_id": new_id, "skip_pull_hotlinked_images": false}),
            None,
        ));
        self.creator_jobs.push((
            "post_update_topic_tracking_state",
            json!({"post_id": new_id}),
            None,
        ));
        self.creator_jobs.push((
            "notify_mailing_list_subscribers",
            json!({"post_id": new_id}),
            Some(s.get("email_time_window_mins")?.to_i() * 60),
        ));
        // The copy takes what the original had.
        sqlx::query("UPDATE email_logs SET post_id = $2 WHERE post_id = $1")
            .bind(post.id)
            .bind(new_id)
            .execute(&mut *conn)
            .await?;
        copy_post_actions(conn, ctx, post.id, new_id, topic_id).await?;
        copy_post_revisions(conn, post.id, new_id).await?;
        let renumber = new_number != post_number;
        sqlx::query(
            "UPDATE posts SET reply_count = $2, version = $3, public_version = $4, \
                    post_number = CASE WHEN $5 THEN $6 ELSE post_number END, \
                    sort_order = CASE WHEN $5 THEN $6 ELSE sort_order END WHERE id = $1",
        )
        .bind(new_id)
        .bind(lookup(reply_count, 1).unwrap_or(0))
        .bind(post.version)
        .bind(post.public_version)
        .bind(renumber)
        .bind(new_number)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             SELECT $2, name, value, clock_timestamp(), clock_timestamp() FROM post_custom_fields \
             WHERE post_id = $1 ORDER BY id",
        )
        .bind(post.id)
        .bind(new_id)
        .execute(&mut *conn)
        .await?;
        self.post_ids_after_move = self
            .post_ids_after_move
            .iter()
            .map(|id| if *id == post.id { new_id } else { *id })
            .collect();
        self.post_moved(conn, new_id, post.id).await?;
        sqlx::query(
            "UPDATE bookmarks SET bookmarkable_id = $2 WHERE bookmarkable_type = 'Post' \
               AND bookmarkable_id = $1",
        )
        .bind(post.id)
        .bind(new_id)
        .execute(&mut *conn)
        .await?;
        let _ = post.locale.as_ref();
        Ok(new_id)
    }

    /// `handle_moved_references`
    async fn handle_moved_references(
        &self,
        conn: &mut PgConnection,
        dest: &Dest,
    ) -> Result<(), AppError> {
        let statements: [&str; 9] = [
            // move_incoming_emails
            "UPDATE incoming_emails ie SET topic_id = mp.new_topic_id, post_id = mp.new_post_id \
             FROM temp_moved_posts mp \
             WHERE ie.topic_id = mp.old_topic_id AND ie.post_id = mp.old_post_id \
               AND mp.old_topic_id <> mp.new_topic_id",
            // move_notifications
            "UPDATE notifications n SET topic_id = mp.new_topic_id, post_number = mp.new_post_number, \
               data = (data::JSONB || jsonb_strip_nulls(jsonb_build_object('topic_title', \
                 CASE WHEN data::JSONB ->> 'topic_title' IS NULL THEN NULL ELSE mp.new_topic_title END)))::JSON \
             FROM temp_moved_posts mp \
             WHERE n.topic_id = mp.old_topic_id AND n.post_number = mp.old_post_number \
               AND n.notification_type <> 17",
            // update_reply_counts
            "UPDATE posts p SET reply_count = GREATEST(0, reply_count - x.moved_reply_count) \
             FROM (SELECT r.post_id, mp.new_topic_id, COUNT(1) AS moved_reply_count \
                   FROM temp_moved_posts mp JOIN post_replies r ON (mp.old_post_id = r.reply_post_id) \
                   GROUP BY r.post_id, mp.new_topic_id) x \
             WHERE x.post_id = p.id AND x.new_topic_id <> p.topic_id",
            // update_quotes
            "UPDATE posts p SET raw = REPLACE(p.raw, \
                ', post:' || mp.old_post_number || ', topic:' || mp.old_topic_id, \
                ', post:' || mp.new_post_number || ', topic:' || mp.new_topic_id), \
               baked_version = NULL \
             FROM temp_moved_posts mp, quoted_posts qp \
             WHERE p.id = qp.post_id AND mp.old_post_id = qp.quoted_post_id",
            // move_first_post_replies
            "UPDATE post_replies pr SET post_id = mp.new_post_id FROM temp_moved_posts mp \
             WHERE mp.old_post_id <> mp.new_post_id AND pr.post_id = mp.old_post_id \
               AND EXISTS (SELECT 1 FROM temp_moved_posts mr WHERE mr.new_post_id = pr.reply_post_id)",
            // delete_post_replies
            "DELETE FROM post_replies pr USING temp_moved_posts mp \
             WHERE (SELECT topic_id FROM posts WHERE id = pr.post_id) <> \
                   (SELECT topic_id FROM posts WHERE id = pr.reply_post_id) \
               AND (pr.reply_post_id = mp.old_post_id OR pr.post_id = mp.old_post_id)",
            // copy_shifted_post_timings_to_temp
            "CREATE TEMPORARY TABLE temp_post_timings ON COMMIT DROP AS ( \
               SELECT pt.topic_id, mp.new_post_number AS post_number, pt.user_id, pt.msecs \
               FROM post_timings pt JOIN temp_moved_posts mp ON mp.old_topic_id = pt.topic_id \
                 AND mp.old_post_number = pt.post_number AND mp.old_topic_id = mp.new_topic_id)",
            // delete_invalid_post_timings
            "DELETE FROM post_timings pt USING temp_moved_posts mp \
             WHERE pt.topic_id = mp.new_topic_id AND pt.post_number = mp.new_post_number",
            // copy_shifted_post_timings_from_temp
            "INSERT INTO post_timings (topic_id, user_id, post_number, msecs) \
             SELECT DISTINCT ON (topic_id, post_number, user_id) topic_id, user_id, post_number, msecs \
             FROM temp_post_timings ORDER BY topic_id, post_number, user_id, msecs DESC \
             ON CONFLICT (topic_id, post_number, user_id) DO UPDATE \
               SET msecs = GREATEST(post_timings.msecs, excluded.msecs)",
        ];
        for sql in statements {
            sqlx::query(sql).execute(&mut *conn).await?;
        }
        // move_post_timings
        sqlx::query(
            "UPDATE post_timings pt SET topic_id = mp.new_topic_id, post_number = mp.new_post_number \
             FROM temp_moved_posts mp \
             WHERE pt.topic_id = mp.old_topic_id AND pt.post_number = mp.old_post_number \
               AND mp.old_post_id = mp.new_post_id AND mp.old_topic_id <> mp.new_topic_id \
               AND mp.new_post_id = ANY($1)",
        )
        .bind(&self.post_ids_after_move)
        .execute(&mut *conn)
        .await?;
        // copy_first_post_timings
        sqlx::query(
            "INSERT INTO post_timings (topic_id, user_id, post_number, msecs) \
             SELECT mp.new_topic_id, pt.user_id, mp.new_post_number, pt.msecs \
             FROM post_timings pt \
             JOIN temp_moved_posts mp ON (pt.topic_id = mp.old_topic_id AND pt.post_number = mp.old_post_number) \
             WHERE mp.old_post_id <> mp.new_post_id AND mp.new_post_id = ANY($1) \
             ON CONFLICT (topic_id, post_number, user_id) DO UPDATE \
               SET msecs = GREATEST(post_timings.msecs, excluded.msecs)",
        )
        .bind(&self.post_ids_after_move)
        .execute(&mut *conn)
        .await?;
        // copy_topic_users, with the destination's highest numbers as
        // they were loaded.
        sqlx::query(
            "INSERT INTO topic_users(user_id, topic_id, posted, last_read_post_number, \
                last_emailed_post_number, first_visited_at, last_visited_at, notification_level, \
                notifications_changed_at, notifications_reason_id) \
             SELECT tu.user_id, $2 AS topic_id, \
               EXISTS(SELECT 1 FROM posts p WHERE p.topic_id = $2 AND p.user_id = tu.user_id LIMIT 1) AS posted, \
               (SELECT MAX(lr.new_post_number) FROM temp_moved_posts lr \
                  WHERE lr.old_topic_id = tu.topic_id AND lr.old_post_number <= tu.last_read_post_number \
                    AND lr.old_topic_id <> lr.new_topic_id) AS last_read_post_number, \
               (SELECT MAX(le.new_post_number) FROM temp_moved_posts le \
                  WHERE le.old_topic_id = tu.topic_id AND le.old_post_number <= tu.last_emailed_post_number \
                    AND le.old_topic_id <> le.new_topic_id) AS last_emailed_post_number, \
               GREATEST(tu.first_visited_at, t.created_at) AS first_visited_at, \
               GREATEST(tu.last_visited_at, t.created_at) AS last_visited_at, \
               tu.notification_level, tu.notifications_changed_at, tu.notifications_reason_id \
             FROM topic_users tu JOIN topics t ON (t.id = $2) \
             WHERE tu.topic_id = $1 \
               AND GREATEST(tu.last_read_post_number, tu.last_emailed_post_number) >= \
                   (SELECT MIN(mp.old_post_number) FROM temp_moved_posts mp WHERE mp.old_topic_id <> mp.new_topic_id) \
             ON CONFLICT (topic_id, user_id) DO UPDATE \
               SET posted = excluded.posted, \
                   last_read_post_number = CASE \
                     WHEN topic_users.last_read_post_number = $4 OR ($3 < $4 \
                       AND topic_users.last_read_post_number = $3 \
                       AND NOT EXISTS(SELECT 1 FROM users u WHERE u.id = topic_users.user_id AND (admin OR moderator))) \
                     THEN GREATEST(topic_users.last_read_post_number, excluded.last_read_post_number) \
                     ELSE topic_users.last_read_post_number END, \
                   last_emailed_post_number = CASE \
                     WHEN topic_users.last_emailed_post_number = $4 OR ($3 < $4 \
                       AND topic_users.last_emailed_post_number = $3 \
                       AND NOT EXISTS(SELECT 1 FROM users u WHERE u.id = topic_users.user_id AND (admin OR moderator))) \
                     THEN GREATEST(topic_users.last_emailed_post_number, excluded.last_emailed_post_number) \
                     ELSE topic_users.last_emailed_post_number END, \
                   first_visited_at = LEAST(topic_users.first_visited_at, excluded.first_visited_at), \
                   last_visited_at = GREATEST(topic_users.last_visited_at, excluded.last_visited_at)",
        )
        .bind(self.original_id)
        .bind(dest.id)
        .bind(dest.highest_post_number)
        .bind(dest.highest_staff_post_number)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// `create_moderator_post_in_original_topic`: the split_topic small
    /// action linking to where the posts went, put where they were.
    async fn create_moderator_post(
        &mut self,
        conn: &mut PgConnection,
        dest: &Dest,
        posts: &[MovePost],
        first_post_number_moved: Option<i32>,
    ) -> Result<(), AppError> {
        let ctx = self.ctx;
        let (slug, title): (Option<String>, String) =
            sqlx::query_as("SELECT slug, title FROM topics WHERE id = $1")
                .bind(dest.id)
                .fetch_one(&mut *conn)
                .await?;
        let slug = slug.unwrap_or_default();
        let base = ctx.config.globals.relative_url_root();
        // posts.first as it is now: moved into the destination, or left
        // where it was when it was copied (a first post, freeze_original).
        let (first_topic, first_number, first_slug): (i32, i32, Option<String>) = sqlx::query_as(
            "SELECT p.topic_id, p.post_number, t.slug FROM posts p JOIN topics t ON t.id = p.topic_id \
             WHERE p.id = $1",
        )
        .bind(posts[0].id)
        .fetch_one(&mut *conn)
        .await?;
        let link = if first_number == 1 {
            if slug.is_empty() {
                format!("[{title}]({base}/t/{})", dest.id)
            } else {
                format!("[{title}]({base}/t/{slug}/{})", dest.id)
            }
        } else {
            format!(
                "[{title}]({base}/t/{}/{first_topic}/{first_number})",
                first_slug.unwrap_or_default()
            )
        };

        let kind = if self.creating_new_topic {
            "new_topic"
        } else {
            "existing_topic"
        };
        let count = posts.len() as i64;
        let form = if count == 1 { "one" } else { "other" };
        let message = ctx
            .i18n
            .t_with(
                &format!("move_posts.{kind}_moderator_post.{form}"),
                &[("count", &count.to_string()), ("topic_link", &link)],
            )
            .ok_or(Unsupported("move_posts moderator post text"))?;
        // PostCreator extracts the small action's links, and this extracts
        // them again; their titles are crawled once the move commits.
        let crawl_before: Vec<i64> = crawl_job_ids(&mut *conn).await?;
        let post_id = small_action::add_with_message(
            &mut *conn,
            ctx,
            &SmallAction {
                user_id: self.user_id,
                topic_id: self.original_id,
                action_code: "split_topic",
                bump: false,
                who: None,
            },
            Some(&message),
            false,
        )
        .await?;
        if let Some(n) = first_post_number_moved {
            sqlx::query(
                "UPDATE posts SET post_number = $2, sort_order = $2, updated_at = clock_timestamp() \
                 WHERE id = $1 AND (post_number <> $2 OR sort_order IS DISTINCT FROM $2)",
            )
            .bind(post_id)
            .bind(n)
            .execute(&mut *conn)
            .await?;
            // Post#index_search runs after the commit on the last save's
            // changes, which update! replaced (with none, when the number
            // stays): no cooked change, so the small action isn't indexed.
            sqlx::query("DELETE FROM post_search_data WHERE post_id = $1")
                .bind(post_id)
                .execute(&mut *conn)
                .await?;
        }
        // TopicLink.extract_from(new_post)
        let (cooked, user_id): (String, i32) =
            sqlx::query_as("SELECT cooked, user_id FROM posts WHERE id = $1")
                .bind(post_id)
                .fetch_one(&mut *conn)
                .await?;
        let s = ctx.settings;
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        crate::posting::links::extract_from(
            &mut *conn,
            &crate::posting::links::Site {
                hostname: &urls.current_hostname()?,
                base_path: base,
                base_url_no_prefix: &urls.base_url_no_prefix()?,
                settings: s,
            },
            &crate::posting::links::LinkPost {
                id: post_id,
                user_id,
                topic_id: self.original_id,
                cooked: &cooked,
            },
        )
        .await?;
        self.defer_new_crawl_jobs(conn, &crawl_before).await?;
        Ok(())
    }

    /// The crawl_topic_link jobs extract_from just enqueued, taken back to
    /// go out with the commit as TopicLink's after_commit sends them.
    async fn defer_new_crawl_jobs(
        &mut self,
        conn: &mut PgConnection,
        before: &[i64],
    ) -> Result<(), AppError> {
        let jobs: Vec<(i64, Value)> = sqlx::query_as(
            "SELECT id, args FROM discourse_rs.jobs WHERE name = 'crawl_topic_link' ORDER BY id",
        )
        .fetch_all(&mut *conn)
        .await?;
        for (id, args) in jobs {
            if before.contains(&id) {
                continue;
            }
            sqlx::query("DELETE FROM discourse_rs.jobs WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            self.deferred.push("crawl_topic_link", args);
        }
        Ok(())
    }

    /// `update_last_post_stats`: the destination's last post, when it is
    /// one of the moved ones, is its last post and bumps it.
    async fn update_last_post_stats(
        &self,
        conn: &mut PgConnection,
        topic_id: i32,
    ) -> Result<(), sqlx::Error> {
        let last: Option<(i32, NaiveDateTime, Option<i32>)> = sqlx::query_as(
            "SELECT id, created_at, user_id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
               AND post_type <> $2 ORDER BY post_number DESC LIMIT 1",
        )
        .bind(topic_id)
        .bind(post_types::WHISPER)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((id, created_at, user_id)) = last
            && self.post_ids_after_move.contains(&id)
        {
            sqlx::query(
                "UPDATE topics SET last_posted_at = $2, last_post_user_id = $3, \
                        bumped_at = clock_timestamp(), updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(topic_id)
            .bind(created_at)
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    /// `close_topic_and_schedule_deletion`: a topic left with nothing to
    /// read is closed, and deleted later (a timer) or at once.
    async fn close_topic_and_schedule_deletion(
        &mut self,
        conn: &mut PgConnection,
    ) -> Result<(), AppError> {
        let ctx = self.ctx;
        crate::topic_status::apply(
            &mut *conn,
            ctx,
            self.original_id,
            self.user_id,
            crate::topic_status::Status::Closed,
            true,
            &Default::default(),
        )
        .await?;
        if self.freeze {
            return Ok(());
        }
        let days = ctx
            .settings
            .get("delete_merged_stub_topics_after_days")?
            .to_i();
        if days == 0 {
            let s = ctx.settings;
            let topic = TopicCtx::load(&mut *conn, s, self.guardian, self.original_id)
                .await?
                .ok_or(Unsupported("deleting a merged topic that is gone"))?;
            if self.guardian.can_delete_topic(s, &topic)? {
                // PostDestroyer on the first post trashes the topic; the
                // mover's own trash! then finds it trashed and only stamps
                // it again, as the system user.
                let first_post: i32 = sqlx::query_scalar(
                    "SELECT id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
                     ORDER BY post_number LIMIT 1",
                )
                .bind(self.original_id)
                .fetch_one(&mut *conn)
                .await?;
                let context = ctx.t("topic_statuses.auto_deleted_by_merge");
                let held = crate::post_destroyer::Deferred::default();
                crate::post_destroyer::destroy_by_staff_in(
                    &mut *conn,
                    ctx,
                    self.guardian,
                    first_post,
                    &crate::post_destroyer::Request {
                        context: Some(&context),
                        reviewable_id: None,
                        deferred: Some(&held),
                    },
                )
                .await?;
                for (name, args) in held
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                {
                    self.deferred.push(name, args);
                }
                let embedded: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM topic_embeds WHERE topic_id = $1 AND deleted_at IS NULL)",
                )
                .bind(self.original_id)
                .fetch_one(&mut *conn)
                .await?;
                if embedded {
                    return Err(Unsupported("trashing a merged topic's embed").into());
                }
                sqlx::query(
                    "UPDATE topics SET deleted_at = clock_timestamp(), deleted_by_id = -1 WHERE id = $1",
                )
                .bind(self.original_id)
                .execute(&mut *conn)
                .await?;
            }
            return Ok(());
        }
        if days > 0 {
            crate::topic_timer::set_or_create_timer(
                &mut *conn,
                ctx,
                self.original_id,
                crate::topic_timer::types::DELETE,
                crate::topic_timer::When::Hours((days * 24) as f64),
                &crate::topic_timer::SetOptions {
                    by_user: Some(self.user_id),
                    based_on_last_post: false,
                    category_id: None,
                    duration_minutes: None,
                },
            )
            .await?;
        }
        Ok(())
    }

    /// `watch_new_topic`: the new topic's author watches it.
    async fn watch_new_topic(
        &self,
        conn: &mut PgConnection,
        topic_id: i32,
    ) -> Result<(), AppError> {
        let owner: Option<i32> = sqlx::query_scalar("SELECT user_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
        let Some(owner) = owner else {
            return Ok(());
        };
        change_topic_user(
            &mut *conn,
            owner,
            topic_id,
            &[TopicUserAttr::NotificationLevel(
                notification_levels::WATCHING,
                notification_reasons::CREATED_TOPIC,
            )],
        )
        .await?;
        crate::bus::publish_notification_level_change(
            self.ctx.bus,
            &mut *conn,
            owner,
            topic_id,
            notification_levels::WATCHING,
            Some(notification_reasons::CREATED_TOPIC),
        )
        .await?;
        Ok(())
    }

    /// `enqueue_jobs`: a copied first post's jobs, notify_moved_posts and
    /// delete_inaccessible_notifications.
    async fn enqueue_jobs(
        &mut self,
        conn: &mut PgConnection,
        topic_id: i32,
    ) -> Result<(), AppError> {
        for (name, args, delay) in std::mem::take(&mut self.creator_jobs) {
            match delay {
                Some(seconds) => crate::jobs::enqueue_in(&mut *conn, seconds, name, args).await?,
                None => crate::jobs::enqueue(&mut *conn, name, args).await?,
            }
        }
        crate::jobs::enqueue(
            &mut *conn,
            "notify_moved_posts",
            json!({"post_ids": self.post_ids_after_move, "moved_by_id": self.user_id}),
        )
        .await?;
        crate::jobs::enqueue(
            &mut *conn,
            "delete_inaccessible_notifications",
            json!({"topic_id": topic_id}),
        )
        .await?;
        Ok(())
    }

    /// DiscourseEvent `:topic_merged` after a move to an existing topic:
    /// discourse-topic-voting's TopicMerger moves a closed original's
    /// votes. Checked before the move, as a move taking every post closes
    /// the original: any votes there are refused.
    async fn topic_merged(&self, conn: &mut PgConnection) -> Result<(), AppError> {
        if !crate::plugins::topic_voting::enabled(self.ctx.settings)? {
            return Ok(());
        }
        let votes: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_voting_votes WHERE topic_id = $1)",
        )
        .bind(self.original_id)
        .fetch_one(&mut *conn)
        .await?;
        if votes {
            return Err(Unsupported("topic voting's merge of a topic's votes").into());
        }
        Ok(())
    }
}

async fn crawl_job_ids(conn: &mut PgConnection) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM discourse_rs.jobs WHERE name = 'crawl_topic_link'")
        .fetch_all(conn)
        .await
}

/// `PostAction.copy(original_post, target_post)`: every action, deleted
/// ones too, onto the copy, then each live one's counters (likes; other
/// actions' counters aren't ported).
async fn copy_post_actions(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    from: i32,
    to: i32,
    topic_id: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO post_actions (post_id, user_id, post_action_type_id, deleted_at, created_at, \
                                   updated_at, deleted_by_id, related_post_id, staff_took_action, \
                                   deferred_by_id, targets_topic, agreed_at, agreed_by_id, deferred_at, \
                                   disagreed_at, disagreed_by_id) \
         SELECT $2, user_id, post_action_type_id, deleted_at, created_at, updated_at, deleted_by_id, \
                related_post_id, staff_took_action, deferred_by_id, targets_topic, agreed_at, agreed_by_id, \
                deferred_at, disagreed_at, disagreed_by_id \
         FROM post_actions WHERE post_id = $1 ORDER BY id",
    )
    .bind(from)
    .bind(to)
    .execute(&mut *conn)
    .await?;
    let live: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT user_id, post_action_type_id FROM post_actions \
         WHERE post_id = $1 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(to)
    .fetch_all(&mut *conn)
    .await?;
    for (user_id, action_type) in live {
        if action_type != crate::post_actions::LIKE as i32 {
            return Err(Unsupported("copying a first post's flags (PostAction.copy)").into());
        }
        crate::likes::update_counters(&mut *conn, ctx, to, user_id, topic_id).await?;
    }
    Ok(())
}

/// `PostRevision.copy(original_post, target_post)`
async fn copy_post_revisions(
    conn: &mut PgConnection,
    from: i32,
    to: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO post_revisions (post_id, user_id, modifications, number, created_at, updated_at, hidden) \
         SELECT $2, user_id, modifications, number, created_at, updated_at, hidden \
         FROM post_revisions WHERE post_id = $1 ORDER BY id",
    )
    .bind(from)
    .bind(to)
    .execute(conn)
    .await?;
    Ok(())
}
