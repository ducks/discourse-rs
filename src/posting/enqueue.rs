//! `NewPostManager.default_handler` for a post `post_needs_approval?`
//! sends to the review queue: the raw validated on its own, the topic or
//! category checked, then `NewPostManager#enqueue` (a
//! ReviewableQueuedPost with its needs_approval score, when the post or
//! topic creator finds the post valid) and the author silenced for a fast
//! typed or matching first post.

use chrono::{Months, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgPool;

use super::Ctx;
use super::create::{NewPost, Outcome};
use super::text::{TitleOptions, clean_title, normalize_whitespaces};
use super::validate::{self, Approval, PostInput, Reason, TopicInput};
use crate::guardian::Guardian;
use crate::pretty_text::{self, MarkdownOptions};
use crate::topic_guardian::TopicCtx;
use crate::{AppError, Unsupported};

/// `Reviewable.statuses[:pending]`
const PENDING: i32 = 0;
/// `ReviewableHistory.types[:created]`
const HISTORY_CREATED: i32 = 0;
/// `ReviewableScore.types[:needs_approval]`
const NEEDS_APPROVAL: i32 = 9;
const SYSTEM_USER_ID: i32 = -1;

/// A queued post as NewPostResultSerializer answers it.
#[derive(Debug)]
pub struct Enqueued {
    /// The reviewable's id and created_at; none when the creator refused it.
    pub reviewable: Option<(i64, NaiveDateTime)>,
    pub raw: String,
    pub pending_count: i64,
    pub errors: Vec<String>,
}

impl Enqueued {
    /// NewPostResultSerializer (with `backwards_compatible_json`, which
    /// keeps an enqueued result whole): action, success or errors,
    /// pending_count, and TopicPendingPostSerializer's pending_post.
    pub fn json(&self) -> Value {
        let mut out = Map::new();
        out.insert("action".into(), json!("enqueued"));
        if self.errors.is_empty() {
            out.insert("success".into(), json!(true));
        } else {
            out.insert("errors".into(), json!(self.errors));
        }
        out.insert("pending_count".into(), json!(self.pending_count));
        let (id, created_at) = match self.reviewable {
            Some((id, at)) => (json!(id), json!(crate::topic_list::time_json(at))),
            None => (Value::Null, Value::Null),
        };
        out.insert(
            "pending_post".into(),
            json!({ "id": id, "raw": self.raw, "created_at": created_at }),
        );
        Value::Object(out)
    }
}

/// The topic a reply goes to, as the queue reads it.
pub(super) struct QueueTopic {
    pub id: i32,
    pub category_id: Option<i32>,
    pub slow_mode_seconds: i32,
}

/// `default_handler` once `post_needs_approval?` gave a reason. `polls`:
/// the raw holds polls (discourse-poll's handler ran first).
#[allow(clippy::too_many_arguments)]
pub(super) async fn enqueue(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    args: &NewPost,
    approval: &Approval,
    reason: Reason,
    category_id: Option<i32>,
    reply_topic: Option<&QueueTopic>,
    polls: bool,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let user = guardian
        .user()
        .ok_or(Unsupported("enqueueing anonymously"))?
        .clone();
    if args.email.is_some() {
        return Err(Unsupported("enqueueing a post from email").into());
    }
    if user.suspended() {
        return Err(Unsupported("enqueueing a suspended user's post").into());
    }
    if polls {
        return Err(Unsupported("enqueueing a post with polls").into());
    }
    let mut conn = pool.acquire().await?;

    // PostValidator over Post.new(raw:) with the user: its errors[:raw].
    let raw_errors = validate::raw_errors(
        &mut conn,
        ctx,
        guardian,
        &PostInput {
            raw: &args.raw,
            topic_id: None,
            first_post: false,
            private_message: false,
            new_record: true,
            post_id: None,
            user_id: user.id,
        },
    )
    .await?;
    if !raw_errors.is_empty() {
        return Ok(Outcome::InvalidRaw(raw_errors));
    }
    let new_topic = args.topic_id.is_none();
    if !new_topic {
        let can = match reply_topic {
            Some(topic) => can_reply(&mut conn, ctx, guardian, topic.id).await?,
            None => false,
        };
        if !can {
            return Ok(Outcome::Refused {
                action: "created_post",
                errors: vec![ctx.t("topic_not_found")],
            });
        }
    }

    // creator.valid?: PostCreator for a reply, TopicCreator for a topic.
    let errors = if let Some(topic) = reply_topic {
        if topic.slow_mode_seconds > 0 {
            return Err(Unsupported("slow mode").into());
        }
        let raw = normalize_whitespaces(&args.raw.replace('\0', ""))
            .trim_end()
            .to_string();
        drop(conn);
        let cooked = pretty_text::cook(
            ctx.host,
            &raw,
            &MarkdownOptions {
                topic_id: Some(i64::from(topic.id)),
                post_id: None,
                user_id: Some(i64::from(user.id)),
                force_quote_link: false,
                omit_nofollow: false,
                chat: false,
            },
        )
        .await?;
        conn = pool.acquire().await?;
        let base_path = ctx.config.globals.relative_url_root().to_string();
        let analysis = validate::analyze(&cooked, &base_path)?;
        validate::validate_post(
            &mut conn,
            ctx,
            guardian,
            &PostInput {
                raw: &raw,
                topic_id: Some(topic.id),
                first_post: false,
                private_message: false,
                new_record: true,
                post_id: None,
                user_id: user.id,
            },
            &analysis,
        )
        .await?
    } else {
        let title = clean_title(
            args.title.as_deref().unwrap_or(""),
            &TitleOptions {
                prettify: s.get("title_prettify")?.truthy(),
                allow_uppercase_posts: s.get("allow_uppercase_posts")?.truthy(),
                remove_extraneous_space: s.get("title_remove_extraneous_space")?.truthy(),
            },
        );
        let mut errors = validate::validate_topic(
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
        errors.extend(
            super::create::validate_topic_tags(
                &mut conn,
                ctx,
                guardian,
                category_id,
                false,
                &crate::tagging::queued_names(args.tags.as_ref())?,
            )
            .await?,
        );
        errors
    };
    if !errors.is_empty() {
        let count = pending_count(&mut conn, user.id).await?;
        return Ok(Outcome::Enqueued(Enqueued {
            reviewable: None,
            raw: args.raw.clone(),
            pending_count: count,
            errors,
        }));
    }

    // The payload: raw, tags, the composer's numbers, discourse-poll's
    // is_poll (its handler passed), the title of a new topic.
    let mut payload = Map::new();
    payload.insert("raw".into(), json!(args.raw));
    payload.insert(
        "tags".into(),
        args.tags_payload.clone().unwrap_or(Value::Null),
    );
    if let Some(n) = args.typing_duration_msecs {
        payload.insert("typing_duration_msecs".into(), json!(n));
    }
    if let Some(n) = args.composer_open_duration_msecs {
        payload.insert("composer_open_duration_msecs".into(), json!(n));
    }
    if let Some(n) = args.reply_to_post_number {
        payload.insert("reply_to_post_number".into(), json!(n));
    }
    payload.insert("is_poll".into(), json!(true));
    if let Some(title) = args.title.as_deref().filter(|t| !t.trim().is_empty()) {
        payload.insert("title".into(), json!(title));
    }
    // created_new!: a reply takes its topic's category.
    let reviewable_category = match reply_topic {
        Some(topic) => topic.category_id,
        None => category_id,
    };

    crate::web_hooks::refuse(&mut conn, "reviewable_created", "reviewable web hooks").await?;
    crate::web_hooks::refuse(
        &mut conn,
        "reviewable_score_updated",
        "reviewable web hooks",
    )
    .await?;
    let mut tx = sqlx::Acquire::begin(&mut *conn).await?;
    let (id, created_at): (i64, NaiveDateTime) = sqlx::query_as(
        "INSERT INTO reviewables (type, type_source, status, created_by_id, reviewable_by_moderator, \
                                  reviewable_by_group_id, category_id, topic_id, score, potential_spam, \
                                  target_id, target_type, target_created_by_id, payload, version, \
                                  latest_score, force_review, potentially_illegal, created_at, updated_at) \
         VALUES ('ReviewableQueuedPost', 'core', $1, $2, TRUE, NULL, $3, $4, 0, FALSE, NULL, NULL, $5, $6, 0, \
                 NULL, FALSE, FALSE, clock_timestamp(), clock_timestamp()) \
         RETURNING id, created_at",
    )
    .bind(PENDING)
    .bind(SYSTEM_USER_ID)
    .bind(reviewable_category)
    .bind(args.topic_id)
    .bind(user.id)
    .bind(Value::Object(payload))
    .fetch_one(&mut *tx)
    .await?;
    // after_create log_history(:created, created_by)
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(id)
    .bind(HISTORY_CREATED)
    .bind(PENDING)
    .bind(SYSTEM_USER_ID)
    .execute(&mut *tx)
    .await?;
    crate::jobs::enqueue(&mut tx, "notify_reviewable", json!({ "reviewable_id": id })).await?;

    // add_score(system, :needs_approval, reason:, force_review: true): the
    // system user scores as staff (1 + 5), with no accuracy or type bonus.
    let score = 6.0;
    let score_at: NaiveDateTime = sqlx::query_scalar(
        "INSERT INTO reviewable_scores (reviewable_id, user_id, reviewable_score_type, status, score, \
                                        take_action_bonus, user_accuracy_bonus, meta_topic_id, reason, \
                                        context, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 0, 0, NULL, $6, NULL, clock_timestamp(), clock_timestamp()) \
         RETURNING created_at",
    )
    .bind(id)
    .bind(SYSTEM_USER_ID)
    .bind(NEEDS_APPROVAL)
    .bind(PENDING)
    .bind(score)
    .bind(reason.as_str())
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE reviewables SET score = score + $2, latest_score = $3, force_review = TRUE, \
                                updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(id)
    .bind(score)
    .bind(score_at)
    .execute(&mut *tx)
    .await?;
    crate::jobs::enqueue(&mut tx, "notify_reviewable", json!({ "reviewable_id": id })).await?;
    if let Some(topic_id) = args.topic_id {
        crate::reviewables::update_topic_reviewable_score(&mut tx, s, topic_id, score).await?;
    }
    let count = pending_count(&mut tx, user.id).await?;
    tx.commit().await?;
    // after_commit compute_user_stats
    crate::review_posts::update_pending_posts_count(
        &mut conn,
        ctx,
        user.id,
        &user.username.to_lowercase(),
    )
    .await?;
    drop(conn);

    // In the default locale: the first post typed too fast, or matching
    // the auto silence regexp, silences its author.
    let silence = if approval.fast_typer {
        Some(ctx.t("user.new_user_typed_too_fast"))
    } else if approval.auto_silence {
        Some(ctx.t("user.content_matches_auto_silence_regex"))
    } else {
        None
    };
    if let Some(reason) = silence {
        // Boxed: silencing posts a system message, through create.
        Box::pin(auto_silence(pool, ctx, &user, &reason, id)).await?;
    }

    Ok(Outcome::Enqueued(Enqueued {
        reviewable: Some((id, created_at)),
        raw: args.raw.clone(),
        pending_count: count,
        errors: Vec::new(),
    }))
}

/// `guardian.can_create_post_on_topic?(topic)` for a reply's topic.
async fn can_reply(
    conn: &mut sqlx::PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
) -> Result<bool, AppError> {
    let s = ctx.settings;
    let Some(topic) = TopicCtx::load(&mut *conn, s, guardian, topic_id).await? else {
        return Ok(false);
    };
    let can_post_anywhere = guardian.can_create_post_anywhere(&mut *conn, s).await?;
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    Ok(guardian.can_see_topic(s, &topic, true, &secure)?
        && guardian.can_create_post_on_topic(s, &topic, can_post_anywhere)?)
}

/// `UserSilencer.auto_silence(user, system, keep_posts: true, reason:,
/// reviewable_id:)`: silenced for a thousand years, then the moderators
/// told when `notify_mods_when_user_silenced`.
async fn auto_silence(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    user: &crate::session::current::SessionUser,
    reason: &str,
    reviewable_id: i64,
) -> Result<(), AppError> {
    let until = crate::clock::now_naive()
        .checked_add_months(Months::new(12_000))
        .ok_or(Unsupported("a silence past the calendar"))?;
    let silenced = crate::user_penalties::silencer_silence(
        pool,
        ctx,
        SYSTEM_USER_ID,
        user.id,
        until,
        reason,
        Some(reviewable_id),
    )
    .await?;
    if !silenced || !ctx.settings.get("notify_mods_when_user_silenced")?.truthy() {
        return Ok(());
    }
    if !user.username.is_ascii() {
        return Err(Unsupported("user_url for a username to encode").into());
    }
    // user_path(encoded_username(lower: true))
    let user_url = format!(
        "{}/u/{}",
        ctx.config.globals.relative_url_root(),
        user.username.to_lowercase()
    );
    crate::group_message::GroupMessage {
        group_name: "moderators",
        message_type: "user_automatically_silenced_with_reason",
        limit_once_per: None,
        message_params: vec![
            ("reason".to_string(), reason.to_string()),
            ("username".to_string(), user.username.clone()),
            ("user_url".to_string(), user_url),
        ],
    }
    .create(pool, ctx)
    .await?;
    Ok(())
}

/// `ReviewableQueuedPost.where(target_created_by: user).pending.count`
async fn pending_count(conn: &mut sqlx::PgConnection, user_id: i32) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM reviewables WHERE type = 'ReviewableQueuedPost' AND status = $2 \
         AND target_created_by_id = $1",
    )
    .bind(user_id)
    .bind(PENDING)
    .fetch_one(conn)
    .await
}
