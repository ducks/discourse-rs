//! `NewPostManager#perform` -> `PostCreator#create` (with TopicCreator for
//! a new topic): validation, then one transaction writing the post, its
//! topic, stats, tracking and user actions, then the category's latest
//! post and the search index.

use chrono::NaiveDateTime;
use serde_json::json;
use sqlx::{PgConnection, PgPool};

use super::links::{self, LinkPost};
use super::search_index::{self, BaseUrls, PostIndex};
use super::text::{TitleOptions, clean_title, normalize_whitespaces, slug_for, word_count};
use super::validate::{self, PostInput, TopicInput};
use super::{
    BAKED_VERSION, Ctx, TopicUserAttr, change_topic_user, current_draft_sequence,
    next_draft_sequence, notification_levels, notification_reasons, post_types, record_timing,
    user_actions,
};
use crate::guardian::Guardian;
use crate::pretty_text::{self, MarkdownOptions};
use crate::topic_guardian::TopicCtx;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The `create_params` this slice accepts.
#[derive(Debug, Default)]
pub struct NewPost {
    pub raw: String,
    pub topic_id: Option<i32>,
    pub title: Option<String>,
    /// `params[:category]`, as sent.
    pub category: Option<String>,
    pub reply_to_post_number: Option<i32>,
    pub typing_duration_msecs: Option<i64>,
    pub composer_open_duration_msecs: Option<i64>,
    pub composer_version: Option<i32>,
    pub user_agent: Option<String>,
}

/// How a create ends when it isn't a server error.
#[derive(Debug)]
pub enum Outcome {
    Created {
        post_id: i32,
    },
    /// `NewPostResult` errors: a 422 with `{action: "create_post", errors}`.
    Invalid(Vec<String>),
    /// `Discourse::InvalidAccess` from the guardian.
    Forbidden,
    /// `Discourse::InvalidParameters`
    InvalidParameter(&'static str),
}

/// The topic a reply goes to, as PostCreator reads it.
#[derive(sqlx::FromRow)]
struct ReplyTopic {
    id: i32,
    title: String,
    archetype: String,
    category_id: Option<i32>,
    slow_mode_seconds: i32,
    word_count: Option<i32>,
    closed: bool,
}

/// What a new topic is created with.
struct TopicPlan {
    title: String,
    category_id: i32,
    slow_mode_seconds: Option<i32>,
    all_topics_wiki: bool,
}

/// `BrowserDetection.device(user_agent)`
fn device(user_agent: Option<&str>) -> &'static str {
    let Some(ua) = user_agent else {
        return "unknown";
    };
    let ua = ua.to_lowercase();
    for (needle, device) in [
        ("android", "android"),
        ("cros", "chromebook"),
        ("ipad", "ipad"),
        ("iphone", "iphone"),
        ("ipod", "ipod"),
        ("mobile", "mobile"),
        ("macintosh", "mac"),
        ("linux", "linux"),
        ("windows", "windows"),
    ] {
        if ua.contains(needle) {
            return device;
        }
    }
    "unknown"
}

/// ActiveSupport's `String#truncate(400)`.
fn truncate_400(s: &str) -> String {
    if s.chars().count() <= 400 {
        return s.to_string();
    }
    format!("{}...", s.chars().take(397).collect::<String>())
}

/// `NewPostManager.new(user, params).perform` for a reply or a regular topic.
pub async fn create(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    args: NewPost,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let user = guardian
        .user()
        .ok_or(Unsupported("creating posts anonymously"))?
        .clone();
    if s.get("site_archived")?.truthy() {
        return Err(Unsupported("site_archived").into());
    }
    let mut conn = pool.acquire().await?;
    let new_topic = args.topic_id.is_none();

    // TopicCreator#setup_topic_params: the category, and the guardian's
    // can_create?(Topic, category).
    let category_id = if new_topic {
        match args.category.as_deref().filter(|c| !c.is_empty()) {
            Some(c) if c.chars().all(|ch| ch.is_ascii_digit()) => {
                let id: Option<i32> = c.parse().ok();
                let found: Option<i32> =
                    sqlx::query_scalar("SELECT id FROM categories WHERE id = $1")
                        .bind(id)
                        .fetch_optional(&mut *conn)
                        .await?;
                match found {
                    Some(id) => Some(id),
                    None => {
                        // Category.find_by(id:) misses; InvalidParameters
                        // follows the guardian check below.
                        if !guardian.can_create_topic(&mut conn, s).await? {
                            return Ok(Outcome::Forbidden);
                        }
                        return Ok(Outcome::InvalidParameter("category"));
                    }
                }
            }
            Some(_) => return Ok(Outcome::InvalidParameter("category")),
            None => None,
        }
    } else {
        None
    };
    if new_topic {
        let allowed = match category_id {
            None => guardian.can_create_topic(&mut conn, s).await?,
            Some(id) => {
                guardian.can_create_topic(&mut conn, s).await?
                    && guardian
                        .topic_create_allowed_category_ids(&mut conn, s)
                        .await?
                        .contains(&id)
            }
        };
        if !allowed {
            return Ok(Outcome::Forbidden);
        }
    }

    // The reply's topic, before the review-queue check reads its category.
    let reply_topic: Option<ReplyTopic> =
        match args.topic_id {
            Some(id) => sqlx::query_as(
                "SELECT id, title, archetype, category_id, slow_mode_seconds, word_count, closed \
                 FROM topics WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
            None => None,
        };
    validate::refuse_unported(
        &mut conn,
        ctx,
        guardian,
        new_topic,
        category_id.or(reply_topic.as_ref().and_then(|t| t.category_id)),
        args.typing_duration_msecs.unwrap_or(0),
    )
    .await?;

    // PostCreator#valid?
    if user.suspended() {
        return Ok(Outcome::Invalid(vec![ctx.t("user_is_suspended")]));
    }
    let mut plan = None;
    if new_topic {
        let title_options = TitleOptions {
            prettify: s.get("title_prettify")?.truthy(),
            allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
            remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
        };
        let title = clean_title(args.title.as_deref().unwrap_or(""), &title_options);
        let errors = validate::validate_topic(
            &mut conn,
            ctx,
            guardian,
            &TopicInput {
                title: &title,
                category_id,
                private_message: false,
            },
        )
        .await?;
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
        let uncategorized = s.get("uncategorized_category_id")?.to_i() as i32;
        let category_id = category_id.unwrap_or(uncategorized);
        let (auto_close_hours, slow_mode, all_topics_wiki): (Option<f64>, Option<i32>, bool) =
            sqlx::query_as(
                "SELECT auto_close_hours, default_slow_mode_seconds, all_topics_wiki FROM categories WHERE id = $1",
            )
            .bind(category_id)
            .fetch_one(&mut *conn)
            .await?;
        if auto_close_hours.is_some() {
            return Err(Unsupported("categories that auto-close topics").into());
        }
        plan = Some(TopicPlan {
            title,
            category_id,
            slow_mode_seconds: slow_mode,
            all_topics_wiki,
        });
    } else {
        let Some(topic) = &reply_topic else {
            return Ok(Outcome::Invalid(vec![ctx.t("topic_not_found")]));
        };
        if topic.slow_mode_seconds > 0 && !guardian.is_staff() {
            return Err(Unsupported("slow mode").into());
        }
        let topic_ctx = TopicCtx::load(&mut conn, s, guardian, topic.id).await?;
        let can_post_anywhere = guardian.can_create_post_anywhere(&mut conn, s).await?;
        let can = match &topic_ctx {
            Some(t) => {
                let can_see = guardian.can_see_topic(
                    s,
                    t,
                    true,
                    &guardian.secure_category_ids(&mut conn, s).await?,
                )?;
                can_see && guardian.can_create_post_on_topic(s, t, can_post_anywhere)?
            }
            None => false,
        };
        if !can {
            return Ok(Outcome::Invalid(vec![ctx.t("topic_not_found")]));
        }
        if topic.archetype == "private_message" {
            return Err(Unsupported("replying to messages").into());
        }
        if topic.closed {
            return Err(Unsupported("replying to closed topics as staff").into());
        }
    }

    // setup_post
    let raw = normalize_whitespaces(&args.raw.replace('\0', ""))
        .trim_end()
        .to_string();
    let reply_to_post_number = if new_topic {
        None
    } else {
        args.reply_to_post_number
    };
    let topic_id_for_cook = reply_topic.as_ref().map(|t| t.id);
    let omit_nofollow = omit_nofollow(&mut conn, ctx, user.id).await?;
    drop(conn);
    let cooked = pretty_text::cook(
        ctx.host,
        &raw,
        &MarkdownOptions {
            topic_id: topic_id_for_cook.map(i64::from),
            post_id: None,
            user_id: Some(i64::from(user.id)),
            force_quote_link: false,
            omit_nofollow,
        },
    )
    .await?;
    let base_path = ctx.config.globals.relative_url_root().to_string();
    let analysis = validate::analyze(&cooked, &base_path)?;
    if analysis.has_uploads {
        return Err(Unsupported("posts with uploads").into());
    }
    if analysis.has_quotes {
        return Err(Unsupported("posts with quotes (QuotedPost)").into());
    }
    if raw.contains("[quote=") {
        return Err(Unsupported("posts with quotes (QuotedPost)").into());
    }
    let mut conn = pool.acquire().await?;
    let errors = validate::validate_post(
        &mut conn,
        ctx,
        guardian,
        &PostInput {
            raw: &raw,
            topic_id: topic_id_for_cook,
            first_post: false,
            private_message: false,
            new_record: true,
            post_id: None,
            user_id: user.id,
        },
        &analysis,
    )
    .await?;
    if !errors.is_empty() {
        return Ok(Outcome::Invalid(errors));
    }
    if new_topic {
        // Saving the post validates again with the topic in place, which
        // makes it a first post.
        let errors = validate::validate_post(
            &mut conn,
            ctx,
            guardian,
            &PostInput {
                raw: &raw,
                topic_id: None,
                first_post: true,
                private_message: false,
                new_record: true,
                post_id: None,
                user_id: user.id,
            },
            &analysis,
        )
        .await?;
        if !errors.is_empty() {
            return Ok(Outcome::Invalid(errors));
        }
    }
    drop(conn);

    // PostCreator#create's transaction.
    let mut tx = pool.begin().await?;
    let urls = Urls {
        config: ctx.config,
        settings: s,
    };
    let draft_key = match &reply_topic {
        Some(t) => format!("topic_{}", t.id),
        None => "new_topic".to_string(),
    };
    // build_post_stats
    let sequence = current_draft_sequence(&mut tx, user.id, &draft_key).await?;
    let drafts_saved: i32 = sqlx::query_scalar(
        "SELECT revisions FROM drafts WHERE sequence = $1 AND user_id = $2 AND draft_key = $3 LIMIT 1",
    )
    .bind(sequence)
    .bind(user.id)
    .bind(&draft_key)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0);

    // create_topic
    let (topic_id, topic_title, topic_word_count, category_id, wiki) = match (&reply_topic, &plan) {
        (Some(t), _) => (
            t.id,
            t.title.clone(),
            t.word_count.unwrap_or(0),
            t.category_id,
            false,
        ),
        (None, Some(plan)) => {
            let id = create_topic(&mut tx, ctx, guardian, user.id, plan).await?;
            (
                id,
                plan.title.clone(),
                0,
                Some(plan.category_id),
                plan.all_topics_wiki,
            )
        }
        (None, None) => unreachable!("a new topic always has a plan"),
    };
    // create_post_notice
    let notice = post_notice(&mut tx, ctx, user.id).await?;

    // save_post: before_create_tasks (reply info, word count, post number
    // through Topic.next_post_number, cook, sort order).
    let reply_to_user_id: Option<i32> = match reply_to_post_number {
        Some(n) => {
            let reply: Option<(Option<i32>, i32)> = sqlx::query_as(
                "SELECT user_id, post_type FROM posts WHERE topic_id = $1 AND post_number = $2 FOR UPDATE",
            )
            .bind(topic_id)
            .bind(n)
            .fetch_optional(&mut *tx)
            .await?;
            if reply.is_some_and(|(_, t)| t == post_types::WHISPER) {
                return Err(Unsupported("replies to whispers").into());
            }
            reply.and_then(|(u, _)| u)
        }
        None => None,
    };
    let words = word_count(&raw);
    let highest: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(post_number), 0) FROM posts WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_one(&mut *tx)
            .await?;
    let reply_sql = if reply_to_post_number.is_some() {
        ", reply_count = reply_count + 1"
    } else {
        ""
    };
    let post_number: i32 = sqlx::query_scalar(&format!(
        "UPDATE topics SET highest_staff_post_number = $1 + 1, highest_post_number = $1 + 1{reply_sql}, \
         posts_count = posts_count + 1 WHERE id = $2 RETURNING highest_post_number"
    ))
    .bind(highest)
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    if s.get("generate_topic_og_image")?.truthy() {
        return Err(Unsupported("generate_topic_og_image").into());
    }
    let (post_id, created_at): (i32, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO posts (user_id, topic_id, post_number, raw, cooked, created_at, updated_at, \
                            reply_to_post_number, reply_to_user_id, last_editor_id, word_count, \
                            sort_order, last_version_at, baked_at, baked_version, wiki, quote_count) \
         VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp(), $6, $7, $1, $8, $3, \
                 clock_timestamp(), clock_timestamp(), $9, $10, 0) \
         RETURNING id, created_at",
    )
    .bind(user.id)
    .bind(topic_id)
    .bind(post_number)
    .bind(&raw)
    .bind(&cooked)
    .bind(reply_to_post_number)
    .bind(reply_to_user_id)
    .bind(words)
    .bind(BAKED_VERSION)
    .bind(wiki)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO post_stats (post_id, drafts_saved, typing_duration_msecs, composer_open_duration_msecs, \
                                 writing_device, writing_device_user_agent, composer_version, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp(), clock_timestamp())",
    )
    .bind(post_id)
    .bind(drafts_saved)
    .bind(args.typing_duration_msecs.unwrap_or(0) as i32)
    .bind(args.composer_open_duration_msecs.unwrap_or(0) as i32)
    .bind(device(args.user_agent.as_deref()))
    .bind(args.user_agent.as_deref().map(truncate_400))
    .bind(args.composer_version)
    .execute(&mut *tx)
    .await?;
    if let Some(notice) = notice {
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             VALUES ($1, 'notice', $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(post_id)
        .bind(notice.to_string())
        .execute(&mut *tx)
        .await?;
    }

    // UserActionManager.post_created
    if !new_topic {
        super::log_user_action(
            &mut tx,
            user_actions::REPLY,
            user.id,
            topic_id,
            post_id,
            created_at,
        )
        .await?;
    }
    // extract_links (QuotedPost refused above)
    let hostname = urls.current_hostname()?;
    links::extract_from(
        &mut tx,
        &links::Site {
            hostname: &hostname,
            base_path: &base_path,
        },
        &LinkPost {
            id: post_id,
            user_id: user.id,
            topic_id,
            cooked: &cooked,
        },
    )
    .await?;
    // track_topic
    change_topic_user(
        &mut tx,
        user.id,
        topic_id,
        &[
            TopicUserAttr::Posted(true),
            TopicUserAttr::LastReadPostNumber(post_number),
            TopicUserAttr::LastPostedAtNow,
        ],
    )
    .await?;
    record_timing(&mut tx, topic_id, user.id, post_number, 5000).await?;
    let replying_level: Option<i32> = sqlx::query_scalar(
        "SELECT notification_level_when_replying FROM user_options WHERE user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    super::auto_notification(
        &mut tx,
        user.id,
        topic_id,
        notification_reasons::CREATED_POST,
        replying_level.unwrap_or(notification_levels::TRACKING),
    )
    .await?;
    // update_topic_stats
    let excerpt = if new_topic {
        Some(crate::excerpt::excerpt(
            &cooked,
            s.get("topic_excerpt_maxlength")?.to_i().max(0) as usize,
            &crate::excerpt::Options {
                strip_links: true,
                image_mode: crate::excerpt::ImageMode::Strip,
                ..Default::default()
            },
        ))
    } else {
        None
    };
    sqlx::query(
        "UPDATE topics SET updated_at = clock_timestamp(), last_posted_at = $2, last_post_user_id = $3, \
                word_count = $4, bumped_at = $2, excerpt = COALESCE($5, excerpt) WHERE id = $1",
    )
    .bind(topic_id)
    .bind(created_at)
    .bind(user.id)
    .bind(topic_word_count + words)
    .bind(excerpt)
    .execute(&mut *tx)
    .await?;
    // update_topic_auto_close
    let timers: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_timers WHERE topic_id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    if timers {
        return Err(Unsupported("topic timers").into());
    }
    // update_user_counts
    sqlx::query(
        "UPDATE user_stats SET first_post_created_at = COALESCE(first_post_created_at, $2) WHERE user_id = $1",
    )
    .bind(user.id)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;
    let column = if new_topic {
        "topic_count"
    } else {
        "post_count"
    };
    sqlx::query(&format!(
        "UPDATE user_stats SET {column} = {column} + 1 WHERE user_id = $1"
    ))
    .bind(user.id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE users SET last_posted_at = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user.id)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;
    // delete_owned_bookmarks (on_owner_reply), then topic_users.bookmarked
    sqlx::query(
        "DELETE FROM bookmarks WHERE id IN (SELECT bookmarks.id FROM bookmarks \
           LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
           LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                            OR (topics.id = posts.topic_id) \
           WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
             AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL \
             AND bookmarks.auto_delete_preference = 2)",
    )
    .bind(user.id)
    .bind(topic_id)
    .execute(&mut *tx)
    .await?;
    let bookmarked: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM bookmarks \
           LEFT JOIN posts ON posts.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Post' \
           LEFT JOIN topics ON (topics.id = bookmarks.bookmarkable_id AND bookmarks.bookmarkable_type = 'Topic') \
                            OR (topics.id = posts.topic_id) \
           WHERE bookmarks.user_id = $1 AND (topics.id = $2 OR posts.topic_id = $2) \
             AND posts.deleted_at IS NULL AND topics.deleted_at IS NULL)",
    )
    .bind(user.id)
    .bind(topic_id)
    .fetch_one(&mut *tx)
    .await?;
    change_topic_user(
        &mut tx,
        user.id,
        topic_id,
        &[TopicUserAttr::Bookmarked(bookmarked)],
    )
    .await?;
    // DraftSequence.next!(user, draft_key)
    next_draft_sequence(&mut tx, user.id, &draft_key).await?;
    // save_reply_relationships
    if let Some(n) = reply_to_post_number {
        let parent: Option<i32> =
            sqlx::query_scalar("SELECT id FROM posts WHERE topic_id = $1 AND post_number = $2")
                .bind(topic_id)
                .bind(n)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(parent) = parent {
            let inserted = sqlx::query(
                "INSERT INTO post_replies (post_id, reply_post_id, created_at, updated_at) \
                 VALUES ($1, $2, clock_timestamp(), clock_timestamp()) ON CONFLICT DO NOTHING",
            )
            .bind(parent)
            .bind(post_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if inserted > 0 {
                sqlx::query("UPDATE posts SET reply_count = reply_count + 1 WHERE id = $1")
                    .bind(parent)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    // PostJobsEnqueuer#enqueue_jobs, committed with the post.
    crate::jobs::enqueue(
        &mut tx,
        "post_alert",
        json!({"post_id": post_id, "new_record": true, "options": null}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "feature_topic_users",
        json!({"topic_id": topic_id}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "process_post",
        json!({"bypass_bump": false, "cooking_options": null, "new_post": true, "post_id": post_id, "skip_pull_hotlinked_images": false}),
    )
    .await?;
    crate::jobs::enqueue(
        &mut tx,
        "post_update_topic_tracking_state",
        json!({"post_id": post_id}),
    )
    .await?;
    crate::jobs::enqueue_in(
        &mut tx,
        s.get("email_time_window_mins")?.to_i() * 60,
        "notify_mailing_list_subscribers",
        json!({"post_id": post_id}),
    )
    .await?;
    tx.commit().await?;

    // After the transaction: track_latest_on_category, auto_close, then
    // the deferred search index.
    let mut conn = pool.acquire().await?;
    if let Some(category_id) = category_id {
        if new_topic {
            sqlx::query(
                "UPDATE categories SET latest_topic_id = $2, latest_post_id = $3 WHERE id = $1",
            )
            .bind(category_id)
            .bind(topic_id)
            .bind(post_id)
            .execute(&mut *conn)
            .await?;
        } else {
            sqlx::query("UPDATE categories SET latest_post_id = $2 WHERE id = $1")
                .bind(category_id)
                .bind(post_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    let posts_count: i32 = sqlx::query_scalar("SELECT posts_count FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    let auto_close = s.get("auto_close_topics_post_count")?.to_i();
    if auto_close > 0 && auto_close <= i64::from(posts_count) {
        return Err(Unsupported("auto_close_topics_post_count").into());
    }
    let (category_name, tags): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT (SELECT name FROM categories WHERE id = $2), \
           (SELECT string_agg(name, ' ') FROM (SELECT t.name FROM topic_tags tt JOIN tags t ON t.id = tt.tag_id \
              WHERE tt.topic_id = $1 \
            UNION ALL SELECT s.name FROM tags s WHERE s.target_tag_id IN \
              (SELECT tag_id FROM topic_tags WHERE topic_id = $1)) n)",
    )
    .bind(topic_id)
    .bind(category_id)
    .fetch_one(&mut *conn)
    .await?;
    search_index::index_post(
        &mut conn,
        s,
        &BaseUrls {
            base_path: &base_path,
            base_url_no_prefix: urls.base_url_no_prefix()?,
        },
        &PostIndex {
            post_id,
            topic_id,
            is_first_post: new_topic,
            topic_title: &topic_title,
            category_name: category_name.as_deref(),
            tag_names: tags.as_deref(),
            cooked: &cooked,
            private_message: false,
        },
    )
    .await?;
    Ok(Outcome::Created { post_id })
}

/// `Post#omit_nofollow?` for the author.
pub async fn omit_nofollow(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<bool, AppError> {
    let tl3_no_follow = ctx.settings.get("tl3_links_no_follow")?.truthy();
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(admin OR moderator OR (NOT $2 AND (staged OR trust_level >= 3)), FALSE) \
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .bind(tl3_no_follow)
    .fetch_optional(&mut *conn)
    .await?
    .unwrap_or(false))
}

/// `create_post_notice`: a first post gets the new-user notice, one after
/// a long absence the returning-user notice.
async fn post_notice(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
) -> Result<Option<serde_json::Value>, AppError> {
    if user_id <= 0 {
        return Ok(None);
    }
    let last: Option<NaiveDateTime> = sqlx::query_scalar(
        "SELECT created_at FROM posts WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(last) = last else {
        return Ok(Some(json!({"type": "new_user"})));
    };
    let days = ctx.settings.get("returning_users_days")?.to_i();
    if days > 0 {
        let returning: bool = sqlx::query_scalar("SELECT $1 < now() - make_interval(days => $2)")
            .bind(last)
            .bind(days as i32)
            .fetch_one(&mut *conn)
            .await?;
        if returning {
            return Err(Unsupported("returning-user post notices").into());
        }
    }
    Ok(None)
}

/// `TopicCreator#create` for a regular topic: the row, the category's
/// count and featured topics, the creator watching it, the NEW_TOPIC
/// action and the topic's draft sequence.
async fn create_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    user_id: i32,
    plan: &TopicPlan,
) -> Result<i32, AppError> {
    let s = ctx.settings;
    if s.get("slug_generation_method")?.to_s() != "ascii" {
        return Err(Unsupported("slug_generation_method other than ascii").into());
    }
    let slug = slug_for(&plan.title)?;
    let fancy_title = fancy_title(&plan.title)?;
    let (topic_id, created_at): (i32, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, visible, category_id, \
                             archetype, bumped_at, created_at, updated_at, slow_mode_seconds) \
         VALUES ($1, $2, $3, $4, $4, TRUE, $5, 'regular', clock_timestamp(), clock_timestamp(), \
                 clock_timestamp(), COALESCE($6, 0)) \
         RETURNING id, created_at",
    )
    .bind(&plan.title)
    .bind(&fancy_title)
    .bind(&slug)
    .bind(user_id)
    .bind(plan.category_id)
    .bind(plan.slow_mode_seconds)
    .fetch_one(&mut *conn)
    .await?;
    // after_create: changed_to_category, then the draft sequence.
    let is_definition: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE topic_id = $1)")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if !is_definition {
        sqlx::query("UPDATE categories SET topic_count = topic_count + 1 WHERE id = $1")
            .bind(plan.category_id)
            .execute(&mut *conn)
            .await?;
        feature_topics_for(conn, s, plan.category_id).await?;
    }
    next_draft_sequence(conn, user_id, &format!("topic_{topic_id}")).await?;
    // set_author_notification_level: watch it.
    change_topic_user(
        conn,
        user_id,
        topic_id,
        &[TopicUserAttr::NotificationLevel(
            notification_levels::WATCHING,
            notification_reasons::CREATED_TOPIC,
        )],
    )
    .await?;
    // UserActionManager.topic_created: the PM-typed row goes, the topic's
    // is logged.
    sqlx::query(
        "DELETE FROM user_actions WHERE action_type = $1 AND user_id = $2 AND acting_user_id = $2 \
         AND target_topic_id = $3 AND target_post_id = -1",
    )
    .bind(user_actions::NEW_PRIVATE_MESSAGE)
    .bind(user_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    super::log_user_action(
        conn,
        user_actions::NEW_TOPIC,
        user_id,
        topic_id,
        -1,
        created_at,
    )
    .await?;
    let _ = guardian;
    Ok(topic_id)
}

/// `Topic.fancy_title(title)` for titles HtmlPrettify and the emoji
/// unescape leave alone: the title HTML-escaped.
fn fancy_title(title: &str) -> Result<String, Unsupported> {
    let prettified = title.contains(['\'', '"', '`', '&', '<', '>'])
        || title.contains("--")
        || title.contains("..")
        || title.contains("(c)")
        || title.contains("(r)")
        || title.contains("(tm)")
        || title.contains("<<")
        || title.contains(">>")
        || !title.is_ascii();
    if prettified {
        return Err(Unsupported("fancy titles (HtmlPrettify)"));
    }
    Ok(title.to_string())
}

/// `CategoryFeaturedTopic.feature_topics_for(category)`: the system
/// user's list, then the anonymous one, rewritten in rank order.
pub async fn feature_topics_for(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    category_id: i32,
) -> Result<(), AppError> {
    let (num_featured, definition, sort_order): (i32, Option<i32>, Option<String>) =
        sqlx::query_as(
            "SELECT num_featured_topics, topic_id, sort_order FROM categories WHERE id = $1",
        )
        .bind(category_id)
        .fetch_one(&mut *conn)
        .await?;
    if sort_order.is_some_and(|o| !o.is_empty()) {
        return Err(Unsupported("featured topics for categories with a sort order").into());
    }
    if s.get("suppress_secured_categories_from_admin")?.truthy() {
        return Err(Unsupported("suppress_secured_categories_from_admin").into());
    }
    let nesting = s.get("max_category_nesting")?.to_i() as i32;
    let definitions_listed = s.get("show_category_definitions_in_topic_lists")?.truthy();
    let mut results: Vec<i32> = Vec::new();
    for anonymous in [false, true] {
        let secured = if anonymous {
            "categories.id IN (SELECT id FROM categories WHERE NOT read_restricted)"
        } else {
            "TRUE"
        };
        let mut except = results.clone();
        if let Some(d) = definition {
            except.push(d);
        }
        let base = format!(
            "FROM topics LEFT OUTER JOIN categories ON categories.id = topics.category_id \
             WHERE topics.deleted_at IS NULL AND topics.archetype <> 'private_message' \
               AND topics.category_id IN (WITH RECURSIVE sub AS (SELECT $1::int AS id, 1 AS depth \
                   UNION SELECT c.id, sub.depth + 1 FROM categories c JOIN sub ON sub.id = c.parent_category_id \
                   WHERE sub.depth < $2) SELECT id FROM sub) \
               {definitions} \
               AND {secured} \
               AND COALESCE(categories.topic_id, 0) <> topics.id \
               AND topics.visible \
               AND NOT (topics.id = ANY($3))",
            definitions = if definitions_listed {
                ""
            } else {
                "AND (categories.topic_id IS DISTINCT FROM topics.id OR topics.category_id = $1)"
            },
        );
        let pinned: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT topics.id {base} AND topics.pinned_at IS NOT NULL AND topics.category_id = $1 \
             ORDER BY topics.bumped_at DESC, topics.pinned_at DESC"
        ))
        .bind(category_id)
        .bind(nesting)
        .bind(&except)
        .fetch_all(&mut *conn)
        .await?;
        let rest: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT topics.id {base} AND (topics.pinned_at IS NULL OR topics.category_id <> $1) \
             ORDER BY topics.bumped_at DESC LIMIT $4"
        ))
        .bind(category_id)
        .bind(nesting)
        .bind(&except)
        .bind(i64::from(num_featured))
        .fetch_all(&mut *conn)
        .await?;
        for id in pinned.into_iter().chain(rest) {
            if !results.contains(&id) {
                results.push(id);
            }
        }
    }
    sqlx::query("DELETE FROM category_featured_topics WHERE category_id = $1")
        .bind(category_id)
        .execute(&mut *conn)
        .await?;
    for (rank, topic_id) in results.iter().enumerate() {
        sqlx::query(
            "INSERT INTO category_featured_topics (category_id, topic_id, rank, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(category_id)
        .bind(topic_id)
        .bind(rank as i32)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}
