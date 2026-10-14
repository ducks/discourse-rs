//! PostDestroyer for staff: deleting a reply or a whole topic (its first
//! post), and recovering a deleted reply.
//!
//! Refused: an author deleting their own post (marked for deletion,
//! through PostRevisor), permanent deletion, posts in messages, posts with
//! links or a pending flag, posts quoted or linked elsewhere,
//! published pages and embeds, and recovering topics or replies with
//! quotes, likes or a reply target to restore.

use chrono::NaiveDateTime;
use serde_json::json;
use sqlx::{PgConnection, PgPool};

use crate::guardian::Guardian;
use crate::posting::revisions::PostAccess;
use crate::posting::{Ctx, post_types, user_actions};
use crate::{AppError, Unsupported};

/// `UserHistory.actions`
const DELETE_POST: i32 = 17;
const DELETE_TOPIC: i32 = 18;
const RECOVER_POST: i32 = 126;

/// The post being deleted or recovered, as PostDestroyer reads it.
#[derive(sqlx::FromRow)]
struct Post {
    id: i32,
    user_id: Option<i32>,
    topic_id: i32,
    post_number: i32,
    post_type: i32,
    created_at: NaiveDateTime,
    raw: String,
    reply_to_post_number: Option<i32>,
}

impl Post {
    fn is_first_post(&self) -> bool {
        self.post_number == 1
    }
}

async fn load_post(conn: &mut PgConnection, id: i32) -> Result<Post, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, user_id, topic_id, post_number, post_type, created_at, raw, reply_to_post_number \
         FROM posts WHERE id = $1",
    )
    .bind(id)
    .fetch_one(conn)
    .await
}

#[derive(sqlx::FromRow)]
struct Topic {
    id: i32,
    user_id: Option<i32>,
    title: String,
    created_at: NaiveDateTime,
    category_id: Option<i32>,
    archetype: String,
    visible: bool,
    highest_post_number: i32,
    closed: bool,
    archived: bool,
}

async fn load_topic(conn: &mut PgConnection, id: i32) -> Result<Topic, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, user_id, title, created_at, category_id, archetype, visible, highest_post_number, \
                closed, archived \
         FROM topics WHERE id = $1",
    )
    .bind(id)
    .fetch_one(conn)
    .await
}

/// `"#{user.username} (#{user.name})"` for a staff log, or None without a
/// user.
async fn user_label(
    conn: &mut PgConnection,
    user_id: Option<i32>,
) -> Result<Option<String>, sqlx::Error> {
    let Some(id) = user_id else {
        return Ok(None);
    };
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT username, name FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(conn)
            .await?;
    Ok(row.map(|(username, name)| format!("{username} ({})", name.unwrap_or_default())))
}

/// Ruby's `Time#to_s` for a UTC time.
fn ruby_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

/// `StaffActionLogger#truncate`
fn truncate(s: &str) -> String {
    const MAX: usize = 50_000;
    if s.chars().count() > MAX {
        format!("{}...", s.chars().take(MAX + 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// What the request asked for, as the controller hands it over.
pub struct Request<'a> {
    pub context: Option<&'a str>,
    /// The review that deletes it (`opts[:reviewable_id]`), logged with it.
    pub reviewable_id: Option<i64>,
    /// Inside a caller's transaction: the jobs Rails enqueues from
    /// after-commit callbacks wait here for its commit. None: enqueued
    /// with the destroy's own transaction.
    pub deferred: Option<&'a Deferred>,
}

/// Jobs held for an enclosing transaction's commit, in callback order.
pub type Deferred = std::sync::Mutex<Vec<(&'static str, serde_json::Value)>>;

/// A job Rails enqueues from an after-commit callback.
async fn after_commit_job(
    conn: &mut PgConnection,
    deferred: Option<&Deferred>,
    name: &'static str,
    args: serde_json::Value,
) -> Result<(), sqlx::Error> {
    match deferred {
        Some(jobs) => {
            // A record saved again in the transaction runs its commit
            // callbacks once.
            let mut jobs = jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !jobs.iter().any(|(n, a)| *n == name && *a == args) {
                jobs.push((name, args));
            }
            Ok(())
        }
        None => crate::jobs::enqueue(conn, name, args).await,
    }
}

/// How a destroy or recover ends when it isn't a server error.
pub enum Outcome {
    Done,
    NotFound,
    Forbidden,
    /// A delete or edit limit refused it.
    RateLimited(crate::rate_limiter::LimitExceeded),
}

/// PostsController#destroy's and #recover's limits, unless the user can
/// moderate the topic (`can_moderate_topic?`).
async fn delete_limits(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    can_see_topic: bool,
) -> Result<Result<(), crate::rate_limiter::LimitExceeded>, AppError> {
    let Some(user) = guardian.user() else {
        return Ok(Ok(()));
    };
    if !ctx.config.rate_limits
        || guardian.can_perform_action_available_to_group_moderators(ctx.settings, can_see_topic)?
    {
        return Ok(Ok(()));
    }
    let user = crate::rate_limiter::LimitUser {
        id: user.id,
        staff: guardian.is_staff(),
    };
    for limiter in crate::posting::limits::delete_limiters(ctx.settings, user)? {
        if let Err(refused) = limiter.performed(pool, true).await? {
            return Ok(Err(refused));
        }
    }
    Ok(Ok(()))
}

/// `PostsController#destroy` from the guardian check on.
pub async fn destroy_post(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    access: &PostAccess,
    req: &Request<'_>,
) -> Result<Outcome, AppError> {
    if !guardian.can_delete_post(
        ctx.settings,
        &access.topic,
        &access.post,
        access.can_see_post,
    )? {
        return Ok(Outcome::Forbidden);
    }
    if let Err(refused) = delete_limits(pool, ctx, guardian, access.can_see_topic).await? {
        return Ok(Outcome::RateLimited(refused));
    }
    destroy(
        pool,
        ctx,
        guardian,
        access.post.id,
        access.can_see_topic,
        req,
    )
    .await?;
    Ok(Outcome::Done)
}

/// `TopicsController#destroy`: the topic's first post, destroyed.
pub async fn destroy_topic(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    req: &Request<'_>,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let mut conn = pool.acquire().await?;
    let Some(topic) =
        crate::topic_guardian::TopicCtx::load(&mut conn, s, guardian, topic_id).await?
    else {
        return Ok(Outcome::Forbidden);
    };
    if topic.trashed() {
        return Err(Unsupported("deleting a deleted topic").into());
    }
    if !guardian.can_delete_topic(s, &topic)? {
        return Ok(Outcome::Forbidden);
    }
    let first: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 ORDER BY sort_order, post_number LIMIT 1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(first) = first else {
        return Err(Unsupported("deleting a topic without posts").into());
    };
    let secure = guardian.secure_category_ids(&mut conn, s).await?;
    let can_see_topic = guardian.can_see_topic(s, &topic, true, &secure)?;
    drop(conn);
    destroy(pool, ctx, guardian, first, can_see_topic, req).await?;
    Ok(Outcome::Done)
}

/// `PostDestroyer.new(Discourse.system_user, post, context:).destroy`, as
/// the scheduled jobs delete old stubs and hidden posts.
pub async fn destroy_as_system(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    post_id: i32,
    context: &str,
) -> Result<(), AppError> {
    let mut conn = pool.acquire().await?;
    let system = crate::session::current::SessionUser::load(&mut conn, -1)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    let guardian = Guardian::for_user(&mut conn, &system).await?;
    drop(conn);
    // The system user is an admin: it sees every topic.
    destroy(
        pool,
        ctx,
        &guardian,
        post_id,
        true,
        &Request {
            context: Some(context),
            reviewable_id: None,
            deferred: None,
        },
    )
    .await
}

/// `PostDestroyer#destroy`: deleted outright by those who moderate the
/// topic (or anyone, with delete_removed_posts_after below 1), marked for
/// deletion when the author removes their own.
async fn destroy(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    can_see_topic: bool,
    req: &Request<'_>,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let user = guardian.user().ok_or(Unsupported("deleting anonymously"))?;
    let mut conn = pool.acquire().await?;
    let post = load_post(&mut conn, post_id).await?;
    let topic = load_topic(&mut conn, post.topic_id).await?;
    let should_reset_bumped_at = topic.highest_post_number == post.post_number
        && post.post_number != 1
        && post.post_type != post_types::WHISPER;
    // can_moderate_this_topic? || post_is_reviewable? (staff)
    let can_moderate = guardian.is_staff()
        || guardian.can_perform_action_available_to_group_moderators(s, can_see_topic)?
        || guardian.can_delete_all_posts_and_topics(s)?;
    let own = post.user_id == Some(user.id);
    let perform = can_moderate || s.get("delete_removed_posts_after")?.to_i() < 1;
    if !perform && !own {
        // Rails does nothing here; the guardian lets no one else through.
        return Err(Unsupported("deleting another's post without moderating the topic").into());
    }
    refuse_unported_deletion(&mut conn, &post, &topic).await?;
    // resolve_reviewables_for_author_deletion
    if own {
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM reviewables WHERE target_type = 'Post' AND target_id = $1 AND status = 0)",
        )
        .bind(post.id)
        .fetch_one(&mut *conn)
        .await?;
        if pending {
            return Err(Unsupported("authors deleting posts under review").into());
        }
    }
    drop(conn);

    if !perform {
        mark_for_deletion(pool, ctx, guardian, &post).await?;
    }
    let mut tx = pool.begin().await?;
    finish(
        &mut tx,
        ctx,
        user.id,
        &post,
        &topic,
        req,
        perform,
        should_reset_bumped_at,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `PostDestroyer.new(staff, post, context:, reviewable_id:).destroy` on
/// the caller's transaction (UserDestroyer deleting a user's posts): a
/// staff member's delete is always performed.
pub async fn destroy_by_staff_in(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post_id: i32,
    req: &Request<'_>,
) -> Result<(), AppError> {
    let user = guardian.user().ok_or(Unsupported("deleting anonymously"))?;
    if !guardian.is_staff() {
        return Err(Unsupported("deleting posts in a transaction as a non-staff user").into());
    }
    let post = load_post(&mut *conn, post_id).await?;
    let topic = load_topic(&mut *conn, post.topic_id).await?;
    let should_reset_bumped_at = topic.highest_post_number == post.post_number
        && post.post_number != 1
        && post.post_type != post_types::WHISPER;
    refuse_unported_deletion(&mut *conn, &post, &topic).await?;
    if post.user_id == Some(user.id) {
        return Err(Unsupported("staff deleting their own posts in a transaction").into());
    }
    finish(
        conn,
        ctx,
        user.id,
        &post,
        &topic,
        req,
        true,
        should_reset_bumped_at,
    )
    .await
}

/// What PostDestroyer#destroy does once it decided: the deletion (when it
/// performs one) and what follows any destroy.
#[allow(clippy::too_many_arguments)]
async fn finish(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    post: &Post,
    topic: &Topic,
    req: &Request<'_>,
    perform: bool,
    should_reset_bumped_at: bool,
) -> Result<(), AppError> {
    let pm = topic.archetype == "private_message";
    let tx = conn;
    if perform {
        perform_delete(&mut *tx, ctx, user_id, post, topic, req).await?;
        // Permanent deletion (:destroyed) is not ported.
        crate::bus::publish_post_change(
            ctx,
            &mut *tx,
            post.id,
            "deleted",
            Default::default(),
            false,
        )
        .await?;
        if post.is_first_post() {
            crate::topic_tracking_state::publish_delete(ctx.bus, &mut *tx, topic.id).await?;
        }
    }

    // UserActionManager.post_destroyed: a reply's rows (already gone with
    // the post's user actions when deleted outright).
    if !post.is_first_post()
        && let Some(author) = post.user_id
    {
        for (action_type, user_id) in post_rows(&mut *tx, topic, author).await? {
            remove_user_action(
                &mut *tx,
                action_type,
                user_id,
                author,
                topic.id,
                post.id,
                pm,
            )
            .await?;
        }
    }
    crate::jobs::enqueue(
        &mut *tx,
        "sync_topic_user_bookmarked",
        json!({ "topic_id": topic.id }),
    )
    .await?;
    if post.is_first_post() {
        sqlx::query(
            "UPDATE user_profiles SET featured_topic_id = NULL WHERE featured_topic_id = $1",
        )
        .bind(topic.id)
        .execute(&mut *tx)
        .await?;
        // UserActionManager.topic_destroyed: topic_rows first removes the
        // row of the other kind (normally none), then the topic's own: the
        // NEW_TOPIC row, or a message's NEW_PRIVATE_MESSAGE and each other
        // recipient's GOT_PRIVATE_MESSAGE.
        if let Some(owner) = topic.user_id {
            let (own, other) = if pm {
                (user_actions::NEW_PRIVATE_MESSAGE, user_actions::NEW_TOPIC)
            } else {
                (user_actions::NEW_TOPIC, user_actions::NEW_PRIVATE_MESSAGE)
            };
            remove_user_action(&mut *tx, other, owner, owner, topic.id, -1, pm).await?;
            remove_user_action(&mut *tx, own, owner, owner, topic.id, -1, pm).await?;
            if pm {
                let recipients: Vec<i32> = sqlx::query_scalar(
                    "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 AND user_id <> $2 ORDER BY id",
                )
                .bind(topic.id)
                .bind(owner)
                .fetch_all(&mut *tx)
                .await?;
                for recipient in recipients {
                    remove_user_action(
                        &mut *tx,
                        user_actions::GOT_PRIVATE_MESSAGE,
                        recipient,
                        owner,
                        topic.id,
                        -1,
                        pm,
                    )
                    .await?;
                }
            }
        }
    }
    if should_reset_bumped_at {
        reset_bumped_at(&mut *tx, topic.id).await?;
    }
    Ok(())
}

/// `PostDestroyer#mark_for_deletion`: the raw replaced through PostRevisor
/// (outside a transaction, as Rails does), then the post marked
/// user-deleted and, for a first post, its topic closed.
async fn mark_for_deletion(
    pool: &PgPool,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    post: &Post,
) -> Result<(), AppError> {
    let key = if post.is_first_post() {
        "js.topic.deleted_by_author_simple"
    } else {
        "js.post.deleted_by_author_simple"
    };
    let outcome = crate::posting::revise::revise(
        pool,
        ctx,
        guardian,
        post.id,
        crate::posting::revise::Changes {
            raw: Some(ctx.t(key)),
            edit_reason: None,
            force_new_version: true,
            skip_validations: true,
        },
    )
    .await?;
    // PostDestroyer lets EditRateLimiter's refusal out of its transaction.
    if let crate::posting::revise::Outcome::RateLimited(_) = outcome {
        return Err(Unsupported("deleting a post the edit limit refuses marking deleted").into());
    }
    if let crate::posting::revise::Outcome::Invalid(errors) = outcome {
        return Err(Unsupported(if errors.is_empty() {
            "marking a post deleted failed"
        } else {
            "marking a post deleted failed validation"
        })
        .into());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE posts SET user_deleted = TRUE WHERE id = $1")
        .bind(post.id)
        .execute(&mut *tx)
        .await?;
    // The post's links: refused above, so none to destroy.
    if post.is_first_post() {
        sqlx::query("UPDATE topics SET closed = TRUE WHERE id = $1")
            .bind(post.topic_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// What PostDestroyer would do here that is not ported.
async fn refuse_unported_deletion(
    conn: &mut PgConnection,
    post: &Post,
    topic: &Topic,
) -> Result<(), AppError> {
    let (flagged, embed, published): (bool, bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM reviewables WHERE target_type = 'Post' AND target_id = $1 \
                        AND type = 'ReviewableFlaggedPost' AND status = 0), \
                EXISTS (SELECT 1 FROM topic_embeds WHERE topic_id = $2), \
                EXISTS (SELECT 1 FROM published_pages WHERE topic_id = $2)",
    )
    .bind(post.id)
    .bind(topic.id)
    .fetch_one(conn)
    .await?;
    let reason = if flagged {
        "deleting flagged posts (agreeing with the flags)"
    } else if embed || published {
        "deleting embedded or published topics"
    } else {
        return Ok(());
    };
    Err(Unsupported(reason).into())
}

/// `TopicLink#destroy` for each link whose `column` is `post_id`: its
/// clicks go with it, and its after_commit crawl_link_title enqueues
/// crawl_topic_link for the gone row (Rails' callback has no `on:`).
async fn destroy_topic_links(
    conn: &mut PgConnection,
    column: &str,
    post_id: i32,
    deferred: Option<&Deferred>,
) -> Result<(), sqlx::Error> {
    let ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT id FROM topic_links WHERE {column} = $1 ORDER BY id"
    ))
    .bind(post_id)
    .fetch_all(&mut *conn)
    .await?;
    for id in ids {
        sqlx::query("DELETE FROM topic_link_clicks WHERE topic_link_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        sqlx::query("DELETE FROM topic_links WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        after_commit_job(
            &mut *conn,
            deferred,
            "crawl_topic_link",
            json!({ "topic_link_id": id }),
        )
        .await?;
    }
    Ok(())
}

/// `UserActionManager.post_rows(post)` for a reply by `author`: the REPLY
/// row, or in a message one row per allowed user (NEW_PRIVATE_MESSAGE for
/// the author, GOT_PRIVATE_MESSAGE for the rest), as (type, user).
async fn post_rows(
    conn: &mut PgConnection,
    topic: &Topic,
    author: i32,
) -> Result<Vec<(i32, i32)>, sqlx::Error> {
    if topic.archetype != "private_message" {
        return Ok(vec![(user_actions::REPLY, author)]);
    }
    let allowed: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM topic_allowed_users WHERE topic_id = $1 ORDER BY id",
    )
    .bind(topic.id)
    .fetch_all(conn)
    .await?;
    Ok(allowed
        .into_iter()
        .map(|user_id| {
            let kind = if user_id == author {
                user_actions::NEW_PRIVATE_MESSAGE
            } else {
                user_actions::GOT_PRIVATE_MESSAGE
            };
            (kind, user_id)
        })
        .collect())
}

/// `UserAction.remove_action!(row)`: the first matching row goes, and the
/// like counts follow outside messages.
async fn remove_user_action(
    conn: &mut PgConnection,
    action_type: i32,
    user_id: i32,
    acting_user_id: i32,
    topic_id: i32,
    post_id: i32,
    private_message: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM user_actions WHERE id = (SELECT id FROM user_actions WHERE action_type = $1 \
           AND user_id = $2 AND acting_user_id = $3 AND target_topic_id = $4 AND target_post_id = $5 \
           ORDER BY id LIMIT 1)",
    )
    .bind(action_type)
    .bind(user_id)
    .bind(acting_user_id)
    .bind(topic_id)
    .bind(post_id)
    .execute(&mut *conn)
    .await?;
    if !private_message {
        crate::likes::update_like_count(conn, user_id, action_type, -1).await?;
    }
    Ok(())
}

/// `PostDestroyer#perform_delete`, not permanent.
async fn perform_delete(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    acting_user_id: i32,
    post: &Post,
    topic: &Topic,
    req: &Request<'_>,
) -> Result<(), AppError> {
    // Post#trash!: its links destroyed, the notice goes, then the row is
    // trashed.
    destroy_topic_links(&mut *conn, "post_id", post.id, req.deferred).await?;
    sqlx::query("DELETE FROM post_custom_fields WHERE post_id = $1 AND name = 'notice'")
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "UPDATE posts SET deleted_at = clock_timestamp(), deleted_by_id = $2 WHERE id = $1",
    )
    .bind(post.id)
    .bind(acting_user_id)
    .execute(&mut *conn)
    .await?;

    // make_previous_post_the_last_one: saved without validation, so only
    // when something changed.
    let last: Option<(NaiveDateTime, i32, i32)> = sqlx::query_as(
        "SELECT created_at, user_id, post_number FROM posts WHERE topic_id = $1 AND id <> $2 \
           AND deleted_at IS NULL AND user_id IS NOT NULL AND post_type NOT IN ($3, $4) \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(post.topic_id)
    .bind(post.id)
    .bind(post_types::WHISPER)
    .bind(post_types::SMALL_ACTION)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((created_at, user_id, post_number)) = last {
        sqlx::query(
            "UPDATE topics SET last_posted_at = $2, last_post_user_id = $3, highest_post_number = $4, \
                               updated_at = clock_timestamp() \
             WHERE id = $1 AND (last_posted_at IS DISTINCT FROM $2 OR last_post_user_id IS DISTINCT FROM $3 \
                                OR highest_post_number <> $4)",
        )
        .bind(topic.id)
        .bind(created_at)
        .bind(user_id)
        .bind(post_number)
        .execute(&mut *conn)
        .await?;
    }
    // mark_topic_changed
    sqlx::query("UPDATE topics SET updated_at = clock_timestamp() WHERE id = $1")
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
    // clear_user_posted_flag
    if let Some(author) = post.user_id {
        sqlx::query(
            "UPDATE topic_users SET posted = FALSE WHERE topic_id = $1 AND user_id = $2 \
               AND NOT EXISTS (SELECT 1 FROM posts WHERE topic_id = $1 AND user_id = $2 AND id <> $3 \
                               AND deleted_at IS NULL)",
        )
        .bind(topic.id)
        .bind(author)
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    }
    // trash_public_post_actions: the live likes trashed, their ids kept
    // in deleted_public_actions (one row each, any earlier ones replaced),
    // the public counts zeroed.
    let likes: Vec<i32> = sqlx::query_scalar(
        "UPDATE post_actions SET deleted_at = clock_timestamp(), deleted_by_id = $2 \
         WHERE id IN (SELECT id FROM post_actions WHERE post_id = $1 AND post_action_type_id = $3 \
                        AND deleted_at IS NULL) \
         RETURNING id",
    )
    .bind(post.id)
    .bind(acting_user_id)
    .bind(crate::post_actions::LIKE as i32)
    .fetch_all(&mut *conn)
    .await?;
    let mut likes = likes;
    likes.sort_unstable();
    sqlx::query(
        "DELETE FROM post_custom_fields WHERE post_id = $1 AND name = 'deleted_public_actions'",
    )
    .bind(post.id)
    .execute(&mut *conn)
    .await?;
    for id in likes {
        sqlx::query(
            "INSERT INTO post_custom_fields (post_id, name, value, created_at, updated_at) \
             VALUES ($1, 'deleted_public_actions', $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(post.id)
        .bind(id.to_string())
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query("UPDATE posts SET like_count = 0 WHERE id = $1")
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    // trash_user_actions
    let actions: Vec<(i32, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT action_type, user_id, acting_user_id, target_topic_id, target_post_id FROM user_actions \
         WHERE target_post_id = $1 ORDER BY id",
    )
    .bind(post.id)
    .fetch_all(&mut *conn)
    .await?;
    for (action_type, user_id, acting_user_id, topic_id, post_id) in actions {
        remove_user_action(
            &mut *conn,
            action_type,
            user_id,
            acting_user_id,
            topic_id,
            post_id,
            topic.archetype == "private_message",
        )
        .await?;
    }
    // remove_associated_replies
    let parents: Vec<i32> =
        sqlx::query_scalar("DELETE FROM post_replies WHERE reply_post_id = $1 RETURNING post_id")
            .bind(post.id)
            .fetch_all(&mut *conn)
            .await?;
    for parent in parents {
        sqlx::query(
            "UPDATE posts SET reply_count = (SELECT COUNT(*) FROM post_replies pr JOIN posts r ON r.id = pr.reply_post_id \
                                             WHERE pr.post_id = $1 AND r.deleted_at IS NULL) WHERE id = $1",
        )
        .bind(parent)
        .execute(&mut *conn)
        .await?;
    }
    // remove_associated_notifications
    sqlx::query("DELETE FROM notifications WHERE topic_id = $1 AND post_number = $2")
        .bind(post.topic_id)
        .bind(post.post_number)
        .execute(&mut *conn)
        .await?;

    // The staff log, when someone else's post goes.
    if post.user_id != Some(acting_user_id) {
        if post.is_first_post() {
            let user = user_label(&mut *conn, topic.user_id)
                .await?
                .unwrap_or_else(|| "(deleted user)".into());
            let details = [
                format!("id: {}", topic.id),
                format!("created_at: {}", ruby_time(topic.created_at)),
                format!("user: {user}"),
                format!("title: {}", topic.title),
                format!("raw: {}", truncate(&post.raw)),
            ]
            .join("\n");
            log(
                &mut *conn,
                DELETE_TOPIC,
                acting_user_id,
                Some(topic.id),
                None,
                req.context,
                req.reviewable_id,
                &details,
            )
            .await?;
        } else {
            let unknown = ctx.t("staff_action_logs.unknown");
            let (username, name): (Option<String>, Option<String>) = match post.user_id {
                Some(id) => sqlx::query_as("SELECT username, name FROM users WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?
                    .unwrap_or((None, None)),
                None => (None, None),
            };
            let details = [
                format!("id: {}", post.id),
                format!("created_at: {}", ruby_time(post.created_at)),
                format!(
                    "user: {} ({})",
                    username.unwrap_or_else(|| unknown.clone()),
                    name.unwrap_or(unknown)
                ),
                format!("topic: {}", topic.title),
                format!("post_number: {}", post.post_number),
                format!("raw: {}", truncate(&post.raw)),
            ]
            .join("\n");
            log(
                &mut *conn,
                DELETE_POST,
                acting_user_id,
                None,
                Some(post.id),
                req.context,
                req.reviewable_id,
                &details,
            )
            .await?;
        }
    }

    // Topic#trash! with its first post.
    if post.is_first_post() {
        if topic.visible
            && let Some(category_id) = topic.category_id
        {
            sqlx::query(
                "UPDATE categories SET topic_count = topic_count - 1 \
                 WHERE id = $1 AND (topic_id <> $2 OR topic_id IS NULL)",
            )
            .bind(category_id)
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
        }
        // CategoryTagStat.topic_deleted
        if let Some(category_id) = topic.category_id {
            sqlx::query(
                "UPDATE category_tag_stats SET topic_count = topic_count - 1 \
                 WHERE category_id = $1 AND topic_count > 0 \
                   AND tag_id IN (SELECT tag_id FROM topic_tags WHERE topic_id = $2)",
            )
            .bind(category_id)
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
        }
        sqlx::query(
            "UPDATE topics SET deleted_at = clock_timestamp(), deleted_by_id = $2 WHERE id = $1",
        )
        .bind(topic.id)
        .bind(acting_user_id)
        .execute(&mut *conn)
        .await?;
        crate::plugins::topic_trashed(
            &mut *conn,
            ctx.settings,
            topic.id,
            topic.closed,
            topic.archived,
        )
        .await?;
    }

    // TopicLink.where(link_post_id:).destroy_all
    destroy_topic_links(&mut *conn, "link_post_id", post.id, req.deferred).await?;
    update_category_latest(&mut *conn, post, topic, req.deferred).await?;
    update_user_counts(&mut *conn, post, topic).await?;
    // TopicUser.update_post_action_cache(post_id:): the likers of the
    // post (trashed likes too) in its topic.
    sqlx::query(
        "UPDATE topic_users tu SET liked = x.state FROM ( \
           SELECT EXISTS ( \
             SELECT 1 FROM post_actions pa JOIN posts p ON p.id = pa.post_id JOIN topics t ON t.id = p.topic_id \
             WHERE pa.deleted_at IS NULL AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
               AND pa.post_action_type_id = $2 AND tu2.topic_id = t.id AND tu2.user_id = pa.user_id) AS state, \
             tu2.topic_id, tu2.user_id \
           FROM topic_users tu2 \
           WHERE tu2.topic_id IN (SELECT topic_id FROM posts WHERE id = $1) \
             AND tu2.user_id IN (SELECT user_id FROM post_actions WHERE post_id = $1 \
                                 AND post_action_type_id = $2)) x \
         WHERE x.topic_id = tu.topic_id AND x.user_id = tu.user_id AND x.state != tu.liked",
    )
    .bind(post.id)
    .bind(crate::post_actions::LIKE as i32)
    .execute(&mut *conn)
    .await?;

    // After commit: Topic.reset_highest.
    reset_highest(&mut *conn, topic.id).await?;
    crate::jobs::enqueue(
        &mut *conn,
        "feature_topic_users",
        json!({ "topic_id": topic.id }),
    )
    .await?;
    Ok(())
}

/// `StaffActionLogger` rows for deleting and recovering.
#[allow(clippy::too_many_arguments)]
async fn log(
    conn: &mut PgConnection,
    action: i32,
    acting_user_id: i32,
    topic_id: Option<i32>,
    post_id: Option<i32>,
    context: Option<&str>,
    reviewable_id: Option<i64>,
    details: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, topic_id, post_id, context, reviewable_id, \
                                     details, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(acting_user_id)
    .bind(topic_id)
    .bind(post_id)
    .bind(context)
    .bind(reviewable_id)
    .bind(details)
    .execute(conn)
    .await?;
    Ok(())
}

/// `update_associated_category_latest_topic` -> `Category#update_latest`
/// when the post was the category's latest (or its topic the latest
/// topic): saved when it changes, which reindexes the category.
async fn update_category_latest(
    conn: &mut PgConnection,
    post: &Post,
    topic: &Topic,
    deferred: Option<&Deferred>,
) -> Result<(), AppError> {
    let Some(category_id) = topic.category_id else {
        return Ok(());
    };
    let (latest_post_id, latest_topic_id): (Option<i32>, Option<i32>) =
        sqlx::query_as("SELECT latest_post_id, latest_topic_id FROM categories WHERE id = $1")
            .bind(category_id)
            .fetch_one(&mut *conn)
            .await?;
    if latest_post_id != Some(post.id)
        && !(post.is_first_post() && latest_topic_id == Some(topic.id))
    {
        return Ok(());
    }
    category_update_latest(conn, category_id, deferred).await
}

/// `Category#update_latest`
async fn category_update_latest(
    conn: &mut PgConnection,
    category_id: i32,
    deferred: Option<&Deferred>,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE categories c SET latest_post_id = x.post_id, latest_topic_id = x.topic_id, \
                                 updated_at = clock_timestamp() \
         FROM (SELECT \
                 (SELECT p.id FROM posts p JOIN topics t ON t.id = p.topic_id \
                  WHERE NOT p.hidden AND p.deleted_at IS NULL AND t.category_id = $1 \
                  ORDER BY p.created_at DESC LIMIT 1) AS post_id, \
                 (SELECT t.id FROM topics t WHERE t.visible AND t.deleted_at IS NULL AND t.category_id = $1 \
                  ORDER BY t.created_at DESC LIMIT 1) AS topic_id) x \
         WHERE c.id = $1 AND (c.latest_post_id IS DISTINCT FROM x.post_id \
                              OR c.latest_topic_id IS DISTINCT FROM x.topic_id)",
    )
    .bind(category_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if changed > 0 {
        // after_save_commit :index_search
        after_commit_job(
            conn,
            deferred,
            "index_category_for_search",
            json!({ "category_id": category_id, "force": false }),
        )
        .await?;
    }
    Ok(())
}

/// `update_user_counts`: the author's first post date, post or topic
/// count, last posted date, and for a topic the repliers' post counts.
async fn update_user_counts(
    conn: &mut PgConnection,
    post: &Post,
    topic: &Topic,
) -> Result<(), AppError> {
    let Some(author) = post.user_id else {
        return Ok(());
    };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(author)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(());
    }
    sqlx::query(
        "UPDATE user_stats SET first_post_created_at = \
           (SELECT created_at FROM posts WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at LIMIT 1) \
         WHERE user_id = $1 AND first_post_created_at = $2",
    )
    .bind(author)
    .bind(post.created_at)
    .execute(&mut *conn)
    .await?;
    // UserStatCountUpdater.decrement!
    let column = if post.is_first_post() {
        Some("topic_count")
    } else if post.post_type == post_types::REGULAR {
        Some("post_count")
    } else {
        None
    };
    if let Some(column) = column {
        sqlx::query(&format!(
            "UPDATE user_stats SET {column} = {column} - 1 WHERE user_id = $1 AND {column} >= 1"
        ))
        .bind(author)
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query(
        "UPDATE users SET last_posted_at = \
           (SELECT created_at FROM posts WHERE user_id = $1 AND deleted_at IS NULL ORDER BY created_at DESC LIMIT 1) \
         WHERE id = $1 AND last_posted_at = $2",
    )
    .bind(author)
    .bind(post.created_at)
    .execute(&mut *conn)
    .await?;
    if post.is_first_post() && topic.archetype != "private_message" {
        // update_post_counts(:decrement): UserStatCountUpdater.set!, not
        // below zero.
        sqlx::query(
            "UPDATE user_stats us SET post_count = GREATEST(us.post_count - c.count, 0) \
             FROM (SELECT user_id, COUNT(*) AS count FROM posts WHERE post_type = $2 AND topic_id = $1 \
                     AND post_number > 1 AND deleted_at IS NULL GROUP BY user_id) c \
             WHERE us.user_id = c.user_id",
        )
        .bind(topic.id)
        .bind(post_types::REGULAR)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Topic.reset_highest(topic_id)`, with readers' last read post numbers
/// brought down to it.
async fn reset_highest(conn: &mut PgConnection, topic_id: i32) -> Result<(), sqlx::Error> {
    let public = format!(
        "post_type NOT IN ({}, {})",
        post_types::SMALL_ACTION,
        post_types::WHISPER
    );
    let staff = format!("post_type <> {}", post_types::SMALL_ACTION);
    let highest: i32 = sqlx::query_scalar(&format!(
        "UPDATE topics SET \
           highest_staff_post_number = (SELECT COALESCE(MAX(post_number), 0) FROM posts \
             WHERE topic_id = $1 AND deleted_at IS NULL AND {staff}), \
           highest_post_number = (SELECT COALESCE(MAX(post_number), 0) FROM posts \
             WHERE topic_id = $1 AND deleted_at IS NULL AND {public}), \
           posts_count = (SELECT count(*) FROM posts WHERE deleted_at IS NULL AND topic_id = $1 AND {public}), \
           word_count = (SELECT SUM(COALESCE(posts.word_count, 0)) FROM posts \
             WHERE topic_id = $1 AND deleted_at IS NULL AND {public}), \
           last_posted_at = (SELECT MAX(created_at) FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND {public}), \
           last_post_user_id = COALESCE((SELECT user_id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
             AND {public} ORDER BY created_at DESC LIMIT 1), last_post_user_id) \
         WHERE id = $1 RETURNING highest_post_number"
    ))
    .bind(topic_id)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE topic_users SET last_read_post_number = $2 WHERE topic_id = $1 AND last_read_post_number > $2",
    )
    .bind(topic_id)
    .bind(highest)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Topic#reset_bumped_at`: the last visible regular post's date, or the
/// first post's; saved without validation when it changed.
async fn reset_bumped_at(conn: &mut PgConnection, topic_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topics t SET bumped_at = p.created_at, updated_at = clock_timestamp() \
         FROM (SELECT created_at FROM posts WHERE topic_id = $1 AND deleted_at IS NULL \
                 AND ((NOT user_deleted AND NOT hidden AND post_type = $2) OR post_number = 1) \
               ORDER BY (NOT user_deleted AND NOT hidden AND post_type = $2) DESC, \
                        CASE WHEN NOT user_deleted AND NOT hidden AND post_type = $2 THEN sort_order END DESC, \
                        sort_order \
               LIMIT 1) p \
         WHERE t.id = $1 AND t.bumped_at IS DISTINCT FROM p.created_at",
    )
    .bind(topic_id)
    .bind(post_types::REGULAR)
    .execute(conn)
    .await?;
    Ok(())
}

/// `PostsController#recover` from the guardian check on: a deleted reply,
/// recovered by staff.
pub async fn recover_post(
    pool: &PgPool,
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    access: &PostAccess,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    if !guardian.can_recover_post(s, &access.post, access.can_see_topic)? {
        return Ok(Outcome::Forbidden);
    }
    if let Err(refused) = delete_limits(pool, ctx, guardian, access.can_see_topic).await? {
        return Ok(Outcome::RateLimited(refused));
    }
    let user = guardian
        .user()
        .ok_or(Unsupported("recovering anonymously"))?;
    let post = load_post(&mut *conn, access.post.id).await?;
    let topic = load_topic(&mut *conn, post.topic_id).await?;
    let can_moderate = guardian.is_staff()
        || guardian.can_perform_action_available_to_group_moderators(s, access.can_see_topic)?
        || guardian.can_delete_all_posts_and_topics(s)?;
    if !(can_moderate && access.post.deleted_at.is_some()) {
        return Err(Unsupported("authors recovering their own posts (user_recovered)").into());
    }
    if post.is_first_post() {
        return Err(Unsupported("recovering topics").into());
    }
    let pm = topic.archetype == "private_message";
    let (deleted_likes, quotes): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM post_custom_fields WHERE post_id = $1 AND name = 'deleted_public_actions'), \
                $2 LIKE '%[quote=%'",
    )
    .bind(post.id)
    .bind(&post.raw)
    .fetch_one(&mut *conn)
    .await?;
    if deleted_likes {
        return Err(Unsupported("recovering a post's likes (deleted_public_actions)").into());
    }
    if quotes || post.reply_to_post_number.is_some() {
        return Err(Unsupported("recovering replies and posts with quotes").into());
    }
    let cooked: String = sqlx::query_scalar("SELECT cooked FROM posts WHERE id = $1")
        .bind(post.id)
        .fetch_one(&mut *conn)
        .await?;

    // staff_recovered: user_deleted cleared, the row recovered; Post#recover!
    // extracts its links again, then updates the category's latest post.
    sqlx::query("UPDATE posts SET user_deleted = FALSE, deleted_at = NULL, deleted_by_id = NULL WHERE id = $1")
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    if let Some(author) = post.user_id {
        let urls = crate::url::Urls {
            config: ctx.config,
            settings: s,
        };
        let hostname = urls.current_hostname()?;
        crate::posting::links::extract_from(
            &mut *conn,
            &crate::posting::links::Site {
                hostname: &hostname,
                base_path: ctx.config.globals.relative_url_root(),
                base_url_no_prefix: &urls.base_url_no_prefix()?,
                settings: s,
            },
            &crate::posting::links::LinkPost {
                id: post.id,
                user_id: author,
                topic_id: topic.id,
                cooked: &cooked,
            },
        )
        .await?;
    } else {
        return Err(Unsupported("recovering posts without an author").into());
    }
    if let Some(category_id) = topic.category_id {
        category_update_latest(&mut *conn, category_id, None).await?;
    }
    // mark_topic_changed
    sqlx::query("UPDATE topics SET updated_at = clock_timestamp() WHERE id = $1")
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
    // No counts for messages.
    if !pm && let Some(author) = post.user_id {
        sqlx::query("UPDATE user_stats SET post_count = post_count + 1 WHERE user_id = $1")
            .bind(author)
            .execute(&mut *conn)
            .await?;
    }
    if topic.user_id.is_none() {
        sqlx::query("UPDATE topics SET user_id = -1 WHERE id = $1")
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
    }
    // Topic#update_statistics!: reset_highest, feature_topic_users (inline),
    // update_action_counts.
    reset_highest(&mut *conn, topic.id).await?;
    crate::jobs::choose_featured_users(&mut *conn, topic.id).await?;
    sqlx::query(
        "UPDATE topics SET like_count = (SELECT COALESCE(SUM(like_count), 0) FROM posts \
                                         WHERE topic_id = $1 AND post_type <> $2 AND deleted_at IS NULL) \
         WHERE id = $1",
    )
    .bind(topic.id)
    .bind(post_types::WHISPER)
    .execute(&mut *conn)
    .await?;
    // reset_bumped_at(post) for the last reply: the post's own date.
    let highest: i32 = sqlx::query_scalar("SELECT highest_post_number FROM topics WHERE id = $1")
        .bind(topic.id)
        .fetch_one(&mut *conn)
        .await?;
    if highest == post.post_number && post.post_type != post_types::WHISPER {
        sqlx::query(
            "UPDATE topics SET bumped_at = $2, updated_at = clock_timestamp() \
             WHERE id = $1 AND bumped_at IS DISTINCT FROM $2",
        )
        .bind(topic.id)
        .bind(post.created_at)
        .execute(&mut *conn)
        .await?;
    }
    // UserActionManager.post_created: the reply's rows again.
    if let Some(author) = post.user_id {
        for (action_type, user_id) in post_rows(&mut *conn, &topic, author).await? {
            crate::posting::log_user_action_by(
                &mut *conn,
                action_type,
                user_id,
                author,
                topic.id,
                post.id,
                post.created_at,
            )
            .await?;
        }
    }
    // skip_topic_stats, as Rails does here.
    crate::bus::publish_post_change(
        ctx,
        &mut *conn,
        post.id,
        "recovered",
        Default::default(),
        true,
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "sync_topic_user_bookmarked",
        json!({ "topic_id": topic.id }),
    )
    .await?;
    crate::jobs::enqueue(
        &mut *conn,
        "notify_mailing_list_subscribers",
        json!({ "post_id": post.id }),
    )
    .await?;
    if post.user_id != Some(user.id) {
        let author = user_label(&mut *conn, post.user_id)
            .await?
            .unwrap_or_else(|| "(deleted user)".into());
        let details = [
            format!("id: {}", post.id),
            format!("created_at: {}", ruby_time(post.created_at)),
            format!("user: {author}"),
            format!("raw: {}", truncate(&post.raw)),
        ]
        .join("\n");
        log(
            &mut *conn,
            RECOVER_POST,
            user.id,
            Some(topic.id),
            Some(post.id),
            None,
            None,
            &details,
        )
        .await?;
    }
    Ok(Outcome::Done)
}
