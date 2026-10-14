//! `Reviewable#perform` for a ReviewableQueuedPost (a post waiting for
//! approval) and a ReviewablePost (a post review_every_post sent to the
//! queue): the actions `build_combined_actions` offers, their aliases,
//! the version, the perform method, `transition_to` and the flag stats,
//! then notify_reviewable and, for a queued post, the author's
//! pending_posts_count (`compute_user_stats`).
//!
//! Queued posts: approving (`approve_post`, PostCreator from the
//! payload), rejecting (`reject_post` and its aliases), rejecting with
//! feedback (`revise_and_reject_post`, a system message), deleting
//! (`delete`, the author's own) and deleting the author (`delete_user`,
//! `delete_user_block`: UserDestroyer as a spammer, then the
//! account_deleted email). Reviewed posts: approving, restoring,
//! unhiding, rejecting, keeping deleted and deleting.
//!
//! Refused: approving into a closed topic, a payload with tags, an email
//! or plugin fields, a silenced author's approval (UserSilencer.unsilence),
//! unhiding a first post (the topic's visibility), a pending claim on the
//! topic, flag stats past the truncation limit, and moderators reviewing
//! posts (reviewable_post_scope).

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::{DeletableUser, Guardian};
use crate::post_destroyer::Deferred;
use crate::posting::Ctx;
use crate::session::current::SessionUser;
use crate::{AppError, Unsupported};

/// `Reviewable.statuses`
const PENDING: i32 = 0;
const APPROVED: i32 = 1;
const REJECTED: i32 = 2;
const DELETED: i32 = 4;
/// `ReviewableHistory.types[:transitioned]`
const HISTORY_TRANSITIONED: i32 = 1;
/// `ReviewableScore.statuses`
const SCORE_PENDING: i32 = 0;
const SCORE_AGREED: i32 = 1;
const SCORE_DISAGREED: i32 = 2;
/// `ReviewableScore.types[:needs_approval]`
const NEEDS_APPROVAL: i32 = 9;
/// `Post.types`
const REGULAR: i32 = 1;
const WHISPER: i32 = 4;
/// `Group::AUTO_GROUPS[:staff]`
const STAFF_GROUP: i32 = 3;
/// `Jobs::TruncateUserFlagStats.truncate_to`
const FLAG_STATS_TRUNCATE_TO: i64 = 100;

/// The request's `additional_args` (ReviewableQueuedPost's revise ones).
#[derive(Debug, Default, Clone)]
pub struct Args {
    pub revise_reason: Option<String>,
    pub revise_feedback: Option<String>,
    pub revise_custom_reason: Option<String>,
}

/// How a perform ends when it isn't a server error.
pub enum Performed {
    Done {
        version: i64,
        /// `remove_reviewable_ids`
        remove: Vec<i64>,
        /// `reviewable_updates`: the other reviewables of the author it
        /// resolved, with their statuses; None when it resolved none.
        updates: Option<Vec<(i64, i32)>>,
        /// The approved post's id and its topic's
        /// (`created_post_id`, `created_post_topic_id`).
        created: Option<(i32, i32)>,
    },
    /// `Reviewable::InvalidAction`
    InvalidAction,
    /// `Reviewable::UpdateConflict`
    Conflict,
}

/// The reviewable as the actions read it.
#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    #[sqlx(rename = "type")]
    kind: String,
    status: i32,
    topic_id: Option<i32>,
    category_id: Option<i32>,
    target_id: Option<i32>,
    target_type: Option<String>,
    target_created_by_id: Option<i32>,
    created_by_id: i32,
    payload: Option<Value>,
    created_at: NaiveDateTime,
}

/// The reviewable's author (`target_created_by`).
#[derive(sqlx::FromRow)]
struct Author {
    id: i32,
    username: String,
    username_lower: String,
    name: Option<String>,
    admin: bool,
    moderator: bool,
    locale: Option<String>,
    silenced_till: Option<NaiveDateTime>,
    suspended_till: Option<NaiveDateTime>,
}

impl Author {
    fn silenced(&self) -> bool {
        self.silenced_till
            .is_some_and(|t| t > crate::clock::now_naive())
    }

    fn suspended(&self) -> bool {
        self.suspended_till
            .is_some_and(|t| t > crate::clock::now_naive())
    }
}

async fn load_author(conn: &mut PgConnection, id: Option<i32>) -> Result<Option<Author>, AppError> {
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(sqlx::query_as(
        "SELECT id, username, username_lower, name, admin, moderator, locale, silenced_till, suspended_till \
         FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?)
}

/// The reviewable's topic, deleted or not.
#[derive(sqlx::FromRow)]
struct TopicRow {
    id: i32,
    title: String,
    slug: Option<String>,
    closed: bool,
    deleted_at: Option<NaiveDateTime>,
}

/// `build_penalty_actions`: can_suspend? (staff on a regular user).
fn can_penalize(guardian: &Guardian, author: Option<&Author>) -> bool {
    author.is_some_and(|a| guardian.is_staff() && !a.admin && !a.moderator)
}

/// `validate_action!`: the action, or an alias of it, among the built
/// ones.
fn validate(built: &[&str], aliases: &[(&str, &str)], action_id: &str) -> bool {
    built.contains(&action_id)
        || aliases
            .iter()
            .any(|(alias, target)| *target == action_id && built.contains(alias))
}

/// `Reviewable#perform(performer, action_id, version:)` on a queued or
/// reviewed post. `deferred` holds the jobs Rails enqueues from
/// after-commit callbacks when an enclosing transaction (UserDestroyer's)
/// commits; None enqueues them before notify_reviewable, at this
/// perform's own commit.
#[allow(clippy::too_many_arguments)]
pub async fn perform(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    reviewable_id: i64,
    action_id: &str,
    version: Option<i64>,
    args: &Args,
    mailer: Option<&crate::email::Mailer>,
    deferred: Option<&Deferred>,
) -> Result<Performed, AppError> {
    let performer = guardian
        .user()
        .ok_or(Unsupported("reviewing anonymously"))?
        .clone();
    let row: Row = sqlx::query_as(
        "SELECT id, type, status, topic_id, category_id, target_id, target_type, target_created_by_id, \
                created_by_id, payload, created_at \
         FROM reviewables WHERE id = $1",
    )
    .bind(reviewable_id)
    .fetch_one(&mut *conn)
    .await?;
    match row.kind.as_str() {
        "ReviewableQueuedPost" => {
            queued(
                conn,
                ctx,
                guardian,
                &performer,
                &row,
                action_id,
                version,
                args,
                mailer,
                deferred.is_some(),
            )
            .await
        }
        "ReviewablePost" => {
            reviewed(
                conn, ctx, guardian, &performer, &row, action_id, version, deferred,
            )
            .await
        }
        _ => Err(Unsupported("reviewable types other than posts").into()),
    }
}

/// `increment_version!(version)`: the new version, None on a conflict.
async fn increment_version(
    conn: &mut PgConnection,
    id: i64,
    version: Option<i64>,
) -> Result<Option<i32>, sqlx::Error> {
    match version {
        Some(v) => sqlx::query_scalar(
            "UPDATE reviewables SET version = version + 1 WHERE id = $1 AND version = $2 RETURNING version",
        )
        .bind(id)
        .bind(v as i32),
        None => sqlx::query_scalar(
            "UPDATE reviewables SET version = version + 1 WHERE id = $1 RETURNING version",
        )
        .bind(id),
    }
    .fetch_optional(conn)
    .await
}

/// `transition_to(status, performer)`: a manual claim on the topic
/// released, the status saved (with the version this perform holds, as
/// the record's save! writes it), its history, the scores settled.
async fn transition_to(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    row: &Row,
    status: i32,
    version: i32,
    performer_id: i32,
) -> Result<(), AppError> {
    if let Some(topic_id) = row.topic_id
        && row.status == PENDING
    {
        let claimed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM reviewable_claimed_topics WHERE topic_id = $1 AND NOT automatic)",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if claimed {
            return Err(Unsupported("releasing a topic's claim from the review queue").into());
        }
    }
    sqlx::query(
        "UPDATE reviewables SET status = $2, version = $3, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(row.id)
    .bind(status)
    .bind(version)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO reviewable_histories (reviewable_id, reviewable_history_type, status, created_by_id, \
                                           edited, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(row.id)
    .bind(HISTORY_TRANSITIONED)
    .bind(status)
    .bind(performer_id)
    .execute(&mut *conn)
    .await?;
    // ReviewableScore.score_transitions: none for deleted.
    let score_status = match status {
        APPROVED => Some(SCORE_AGREED),
        REJECTED => Some(SCORE_DISAGREED),
        3 => Some(3),
        _ => None,
    };
    if let Some(score_status) = score_status {
        sqlx::query(
            "UPDATE reviewable_scores SET status = $2, reviewed_by_id = $3, reviewed_at = clock_timestamp() \
             WHERE reviewable_id = $1 AND status IN ($4, $5)",
        )
        .bind(row.id)
        .bind(score_status)
        .bind(performer_id)
        .bind(SCORE_PENDING)
        .bind(SCORE_DISAGREED)
        .execute(&mut *conn)
        .await?;
    }
    let _ = ctx;
    Ok(())
}

async fn notify_reviewable(
    conn: &mut PgConnection,
    reviewable_id: i64,
    performer: &SessionUser,
    remove: &[i64],
) -> Result<(), sqlx::Error> {
    crate::jobs::enqueue(
        conn,
        "notify_reviewable",
        json!({
            "reviewable_id": reviewable_id,
            "performing_username": performer.username,
            "updated_reviewable_ids": remove,
        }),
    )
    .await
}

/// `pending_reviewable_ids_for_target_user`: the author's other pending
/// reviewables, which a delete-user action may resolve.
async fn pending_for_author(
    conn: &mut PgConnection,
    row: &Row,
    author: Option<&Author>,
) -> Result<Vec<i64>, sqlx::Error> {
    let Some(author) = author else {
        return Ok(Vec::new());
    };
    sqlx::query_scalar(
        "SELECT id FROM reviewables WHERE status = $1 AND id <> $3 \
           AND (target_created_by_id = $2 OR (target_type = 'User' AND target_id = $2)) ORDER BY id",
    )
    .bind(PENDING)
    .bind(author.id)
    .bind(row.id)
    .fetch_all(conn)
    .await
}

/// `resolved_reviewable_ids` and the serializer's `reviewable_updates`.
async fn resolved(
    conn: &mut PgConnection,
    candidates: &[i64],
) -> Result<Option<Vec<(i64, i32)>>, sqlx::Error> {
    if candidates.is_empty() {
        return Ok(None);
    }
    let still_pending: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM reviewables WHERE id = ANY($1) AND status = $2")
            .bind(candidates)
            .bind(PENDING)
            .fetch_all(&mut *conn)
            .await?;
    let gone: Vec<i64> = candidates
        .iter()
        .copied()
        .filter(|id| !still_pending.contains(id))
        .collect();
    if gone.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        sqlx::query_as("SELECT id, status FROM reviewables WHERE id = ANY($1) ORDER BY id")
            .bind(&gone)
            .fetch_all(conn)
            .await?,
    ))
}

/// ReviewableQueuedPost's `action_aliases`.
const QUEUED_ALIASES: [(&str, &str); 4] = [
    ("discard_post", "reject_post"),
    ("reject_and_silence", "reject_post"),
    ("reject_and_suspend", "reject_post"),
    ("delete_user_block", "delete_and_block_user"),
];

#[allow(clippy::too_many_arguments)]
async fn queued(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    performer: &SessionUser,
    row: &Row,
    action_id: &str,
    version: Option<i64>,
    args: &Args,
    mailer: Option<&crate::email::Mailer>,
    nested: bool,
) -> Result<Performed, AppError> {
    let s = ctx.settings;
    let author = load_author(&mut *conn, row.target_created_by_id).await?;
    let topic: Option<TopicRow> = match row.topic_id {
        Some(id) => {
            sqlx::query_as("SELECT id, title, slug, closed, deleted_at FROM topics WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    // `topic`: the association's default scope leaves a deleted one out.
    let live_topic = topic.as_ref().filter(|t| t.deleted_at.is_none());
    if row.target_type.as_deref() == Some("Post") && !guardian.is_admin() {
        return Err(Unsupported("moderators reviewing approved queued posts").into());
    }

    // build_combined_actions
    let mut built: Vec<&str> = Vec::new();
    if row.status != APPROVED {
        if live_topic.is_some_and(|t| t.closed) {
            built.push("approve_post_closed");
        } else if author.is_some() {
            built.push("approve_post");
        }
    }
    if row.status == PENDING {
        built.push("reject_post");
        if author.is_some() {
            built.push("revise_and_reject_post");
        }
        if can_penalize(guardian, author.as_ref())
            && let Some(a) = &author
        {
            if !a.silenced() {
                built.push("reject_and_silence");
            }
            if !a.suspended() {
                built.push("reject_and_suspend");
            }
        }
        if let Some(a) = &author {
            let deletable = DeletableUser::load(&mut *conn, a.id)
                .await?
                .ok_or(Unsupported("an author who is gone"))?;
            if guardian.can_delete_user(&mut *conn, s, &deletable).await? {
                built.push("delete_user");
                built.push("delete_user_block");
            }
        }
    }
    // can_delete_reviewable_queued_post?: the author's own.
    if row.target_created_by_id == Some(performer.id) {
        built.push("delete");
    }
    if !validate(&built, &QUEUED_ALIASES, action_id) {
        return Ok(Performed::InvalidAction);
    }
    let method = QUEUED_ALIASES
        .iter()
        .find(|(alias, _)| *alias == action_id)
        .map_or(action_id, |(_, target)| *target);
    let delete_user = matches!(method, "delete_user" | "delete_and_block_user");
    match method {
        "approve_post_closed" => {
            return Err(Unsupported("approving a queued reply to a closed topic").into());
        }
        "approve_post"
        | "reject_post"
        | "revise_and_reject_post"
        | "delete"
        | "delete_user"
        | "delete_and_block_user" => {}
        // respond_to?(perform_method) fails.
        _ => return Ok(Performed::InvalidAction),
    }
    if method == "revise_and_reject_post" && topic.is_some() && live_topic.is_none() {
        return Err(Unsupported("revising a queued reply to a deleted topic").into());
    }
    let candidates = if delete_user {
        pending_for_author(&mut *conn, row, author.as_ref()).await?
    } else {
        Vec::new()
    };

    let Some(new_version) = increment_version(&mut *conn, row.id, version).await? else {
        return Ok(Performed::Conflict);
    };
    let mut remove = vec![row.id];
    let mut created = None;
    let transition = match method {
        "approve_post" => {
            let author = author
                .as_ref()
                .ok_or(Unsupported("approving a queued post without its author"))?;
            created = Some(approve(&mut *conn, ctx, performer, row, author).await?);
            APPROVED
        }
        "reject_post" => {
            log_post_rejected(
                &mut *conn,
                ctx,
                performer,
                row,
                author.as_ref(),
                topic.as_ref(),
            )
            .await?;
            REJECTED
        }
        "revise_and_reject_post" => {
            let author = author
                .as_ref()
                .ok_or(Unsupported("revising a queued post without its author"))?;
            revise_message(&mut *conn, ctx, row, author, live_topic, args).await?;
            log_post_rejected(
                &mut *conn,
                ctx,
                performer,
                row,
                Some(author),
                topic.as_ref(),
            )
            .await?;
            REJECTED
        }
        "delete" => DELETED,
        _ => {
            // perform_delete_user / perform_delete_and_block_user: the
            // reviewables the author created go from the queue too.
            if let Some(a) = &author {
                let created: Vec<i64> = sqlx::query_scalar(
                    "SELECT id FROM reviewables WHERE created_by_id = $1 ORDER BY id",
                )
                .bind(a.id)
                .fetch_all(&mut *conn)
                .await?;
                remove.extend(created);
                delete_author(
                    conn,
                    ctx,
                    guardian,
                    row,
                    a,
                    method == "delete_and_block_user",
                    mailer,
                )
                .await?;
            }
            REJECTED
        }
    };
    transition_to(&mut *conn, ctx, row, transition, new_version, performer.id).await?;
    if delete_user {
        // update_column(:target_created_by_id, nil)
        sqlx::query("UPDATE reviewables SET target_created_by_id = NULL WHERE id = $1")
            .bind(row.id)
            .execute(&mut *conn)
            .await?;
    }
    // after_commit compute_user_stats: the status left pending. Inside
    // UserDestroyer the author is gone by the commit.
    if row.status == PENDING
        && !nested
        && let Some(a) = &author
    {
        update_pending_posts(&mut *conn, ctx, a).await?;
    }
    notify_reviewable(&mut *conn, row.id, performer, &remove).await?;
    let updates = resolved(&mut *conn, &candidates).await?;
    Ok(Performed::Done {
        version: i64::from(new_version),
        remove,
        updates,
        created,
    })
}

/// `perform_approve_post`: PostCreator writes the payload as the author
/// (`skip_validations`, `skip_guardian`, `reviewed_queued_post`), the
/// reviewable points at the post, the approval is logged and the author
/// told (`post_approved`), and the creator's jobs follow the commit
/// (`skip_jobs`, then `enqueue_jobs`). The post and its topic's ids.
async fn approve(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    performer: &SessionUser,
    row: &Row,
    author: &Author,
) -> Result<(i32, i32), AppError> {
    let s = ctx.settings;
    let payload = row
        .payload
        .as_ref()
        .and_then(Value::as_object)
        .ok_or(Unsupported("a queued post without a payload"))?;
    // create_options: the payload's keys PostCreator is given.
    for (key, value) in payload {
        match key.as_str() {
            "raw"
            | "title"
            | "typing_duration_msecs"
            | "composer_open_duration_msecs"
            | "reply_to_post_number" => {}
            "tags" if value.as_array().is_none_or(Vec::is_empty) => {}
            _ if value.is_null() => {}
            _ => {
                return Err(Unsupported(
                    "approving a queued post with tags, an email or plugin fields",
                )
                .into());
            }
        }
    }
    let int = |key: &str| payload.get(key).and_then(Value::as_i64);
    let raw = payload
        .get("raw")
        .and_then(Value::as_str)
        .ok_or(Unsupported("a queued post without its raw"))?
        .to_string();
    if author.silenced() {
        return Err(Unsupported("unsilencing the author of an approved post").into());
    }
    let author_user = SessionUser::load(&mut *conn, author.id)
        .await?
        .ok_or(Unsupported("approving a post for a user who is gone"))?;
    let author_guardian = Guardian::for_user(&mut *conn, &author_user).await?;
    let mark: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM discourse_rs.jobs")
        .fetch_one(&mut *conn)
        .await?;
    // PostCreator's own transaction, committed before this perform's,
    // which holds nothing it writes.
    let outcome = crate::posting::create::create(
        &ctx.host.pool,
        ctx,
        &author_guardian,
        crate::posting::create::NewPost {
            raw,
            topic_id: row.topic_id,
            title: payload
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string),
            category: row.category_id.map(|c| c.to_string()),
            reply_to_post_number: int("reply_to_post_number").map(|n| n as i32),
            typing_duration_msecs: int("typing_duration_msecs"),
            composer_open_duration_msecs: int("composer_open_duration_msecs"),
            skip_validations: true,
            reviewed_queued_post: true,
            ..Default::default()
        },
    )
    .await?;
    let crate::posting::create::Outcome::Created { post_id } = outcome else {
        return Err(Unsupported("a queued post PostCreator refuses").into());
    };
    let (topic_id, post_number, slug): (i32, i32, Option<String>) = sqlx::query_as(
        "SELECT p.topic_id, p.post_number, t.slug FROM posts p JOIN topics t ON t.id = p.topic_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    // self.target = created_post; self.topic_id ||= its topic; save
    sqlx::query(
        "UPDATE reviewables SET target_id = $2, target_type = 'Post', topic_id = COALESCE(topic_id, $3), \
                                updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(row.id)
    .bind(post_id)
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    if performer.admin || performer.moderator {
        let action = crate::staff_action_logs::action_id("post_approved")
            .ok_or(Unsupported("UserHistory.actions[:post_approved]"))?;
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, post_id, reviewable_id, admin_only, \
                                         created_at, updated_at) \
             VALUES ($1, $2, $3, $4, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(action)
        .bind(performer.id)
        .bind(post_id)
        .bind(row.id)
        .execute(&mut *conn)
        .await?;
    }
    notify_approved(
        &mut *conn,
        ctx,
        author.id,
        topic_id,
        post_number,
        slug.as_deref(),
    )
    .await?;
    let _ = s;

    // result.after_commit: creator.enqueue_jobs, behind the notification's
    // own commit callbacks.
    sqlx::query(
        "WITH moved AS ( \
           DELETE FROM discourse_rs.jobs WHERE id > $1 \
             AND (args->>'post_id' = $2::text \
                  OR (name = 'feature_topic_users' AND args->>'topic_id' = $3::text)) \
           RETURNING id, name, args, run_at, created_at) \
         INSERT INTO discourse_rs.jobs (name, args, run_at, created_at) \
         SELECT name, args, run_at, created_at FROM moved ORDER BY id",
    )
    .bind(mark)
    .bind(post_id.to_string())
    .bind(topic_id.to_string())
    .execute(&mut *conn)
    .await?;
    Ok((post_id, topic_id))
}

/// `Notification.create!(notification_type: :post_approved, ...)` and its
/// commit callbacks: the author's notification state, and the
/// post_approved email (NotificationEmailer, after email_time_window_mins
/// unless their email level is never).
async fn notify_approved(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    user_id: i32,
    topic_id: i32,
    post_number: i32,
    slug: Option<&str>,
) -> Result<(), AppError> {
    const POST_APPROVED: i32 = 20;
    let data = json!({
        "post_url": format!(
            "/t/{}/{topic_id}/{post_number}",
            slug.filter(|s| !s.is_empty()).unwrap_or("topic")
        ),
    });
    let notification_id: i64 = sqlx::query_scalar(
        "INSERT INTO notifications (notification_type, user_id, topic_id, post_number, data, read, \
                                    high_priority, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(POST_APPROVED)
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(data.to_string())
    .fetch_one(&mut *conn)
    .await?;
    crate::bus::publish_notifications_state(ctx.bus, &mut *conn, ctx.settings, user_id).await?;
    let (dnd, email_level, active, approved, staged): (bool, Option<i32>, bool, bool, bool) =
        sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM do_not_disturb_timings WHERE user_id = $1 \
                              AND starts_at <= now() AND ends_at > now()), \
                    (SELECT email_level FROM user_options WHERE user_id = $1), u.active, u.approved, u.staged \
             FROM users u WHERE u.id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
    if dnd {
        return Err(Unsupported("do not disturb (shelved notifications)").into());
    }
    // The approved post is a regular one: an emailable post type.
    let emailable = email_level != Some(2)
        && (active || staged)
        && (approved || staged || !ctx.settings.get("must_approve_users")?.truthy());
    if emailable {
        crate::jobs::enqueue_in(
            &mut *conn,
            ctx.settings.get("email_time_window_mins")?.to_i() * 60,
            "user_email",
            json!({
                "type": "post_approved",
                "user_id": user_id,
                "notification_id": notification_id,
                "notification_data_hash": data,
                "notification_type": "post_approved",
            }),
        )
        .await?;
    }
    Ok(())
}

/// `UserStat#update_pending_posts` for a user still there: the count of
/// their pending queued posts, and the counters to them and staff.
async fn update_pending_posts(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    author: &Author,
) -> Result<(), AppError> {
    let count: Option<i32> = sqlx::query_scalar(
        "UPDATE user_stats SET pending_posts_count = \
           (SELECT COUNT(*) FROM reviewables WHERE type = 'ReviewableQueuedPost' AND status = $2 \
              AND target_created_by_id = $1)::int \
         WHERE user_id = $1 RETURNING pending_posts_count",
    )
    .bind(author.id)
    .bind(PENDING)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(count) = count else {
        return Ok(());
    };
    ctx.bus
        .publish(
            &mut *conn,
            &format!("/u/{}/counters", author.username_lower),
            &json!({ "pending_posts_count": count }),
            Some(&[
                crate::bus::user_tag(author.id),
                crate::bus::group_tag(STAFF_GROUP),
            ]),
        )
        .await?;
    Ok(())
}

/// `StaffActionLogger#log_post_rejected(reviewable, DateTime.now)`.
async fn log_post_rejected(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    performer: &SessionUser,
    row: &Row,
    author: Option<&Author>,
    topic: Option<&TopicRow>,
) -> Result<(), AppError> {
    if !performer.admin && !performer.moderator {
        return Ok(());
    }
    let unknown = ctx.t("staff_action_logs.unknown");
    let (username, name) = match author {
        Some(a) => (
            a.username.clone(),
            a.name.clone().unwrap_or_else(|| unknown.clone()),
        ),
        None => (unknown.clone(), unknown.clone()),
    };
    let title = topic
        .map(|t| t.title.clone())
        .unwrap_or_else(|| ctx.t("staff_action_logs.not_found"));
    let raw = row
        .payload
        .as_ref()
        .and_then(|p| p.get("raw"))
        .map(|r| match r {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let details = [
        format!(
            "created_at: {}",
            row.created_at.format("%Y-%m-%d %H:%M:%S UTC")
        ),
        format!(
            "rejected_at: {}",
            crate::clock::now_naive().format("%Y-%m-%dT%H:%M:%S+00:00")
        ),
        format!("user: {username} ({name})"),
        format!("topic: {title}"),
        format!("raw: {}", truncate(&raw)),
    ]
    .join("\n");
    let action = crate::staff_action_logs::action_id("post_rejected")
        .ok_or(Unsupported("UserHistory.actions[:post_rejected]"))?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, details, topic_id, category_id, reviewable_id, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(performer.id)
    .bind(details)
    .bind(topic.map(|t| t.id))
    .bind(row.category_id)
    .bind(row.id)
    .execute(conn)
    .await?;
    Ok(())
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

/// `perform_revise_and_reject_post`'s system message to the author, as
/// SystemMessage creates it (its own transaction, committed before this
/// perform's, which holds nothing it writes).
async fn revise_message(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    row: &Row,
    author: &Author,
    topic: Option<&TopicRow>,
    args: &Args,
) -> Result<(), AppError> {
    let s = ctx.settings;
    let default_locale = s.get("default_locale")?.to_s();
    if author
        .locale
        .as_deref()
        .is_some_and(|l| !l.is_empty() && l != default_locale)
        && s.get("allow_user_locale")?.truthy()
    {
        return Err(Unsupported("system messages in a user's own locale").into());
    }
    let has_contact = !s.get("site_contact_username")?.to_s().trim().is_empty();
    let new_topic = topic.is_none();
    let key = match (new_topic, has_contact) {
        (true, true) => "reviewable_queued_post_revise_and_reject_edit_topic",
        (true, false) => "reviewable_queued_post_revise_and_reject_edit_topic_no_reply",
        (false, true) => "reviewable_queued_post_revise_and_reject_edit_post",
        (false, false) => "reviewable_queued_post_revise_and_reject_edit_post_no_reply",
    };
    let payload_str = |name: &str| {
        row.payload
            .as_ref()
            .and_then(|p| p.get(name))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let topic_title = topic
        .map(|t| t.title.clone())
        .or_else(|| payload_str("title"))
        .unwrap_or_default();
    let topic_url = match topic {
        Some(t) => {
            let urls = crate::url::Urls {
                config: ctx.config,
                settings: s,
            };
            format!(
                "{}/t/{}/{}",
                urls.base_url()?,
                t.slug
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("topic"),
                t.id
            )
        }
        None => String::new(),
    };
    let reason = args
        .revise_custom_reason
        .clone()
        .filter(|r| !r.trim().is_empty())
        .or_else(|| args.revise_reason.clone())
        .unwrap_or_default();
    let params = vec![
        ("topic_title".to_string(), topic_title),
        ("topic_url".to_string(), topic_url),
        ("reason".to_string(), reason),
        (
            "feedback".to_string(),
            args.revise_feedback.clone().unwrap_or_default(),
        ),
        (
            "original_post".to_string(),
            payload_str("raw").unwrap_or_default(),
        ),
        ("site_name".to_string(), s.get("title")?.to_s()),
        (
            "edit_instructions".to_string(),
            ctx.t(&format!("system_messages.{key}")),
        ),
    ];
    let message_type = if new_topic {
        "reviewable_queued_post_revise_and_reject_new_topic"
    } else {
        "reviewable_queued_post_revise_and_reject"
    };
    let post_id =
        crate::system_message::create(&ctx.host.pool, ctx, author.id, message_type, &params, None)
            .await?;
    // In Rails the message is created inside the perform's transaction, so
    // its links' after-commit crawl waits for that commit: those jobs move
    // behind the message's own.
    let crawls: Vec<Value> = sqlx::query_scalar(
        "DELETE FROM discourse_rs.jobs WHERE name = 'crawl_topic_link' \
           AND (args->>'topic_link_id')::int IN (SELECT id FROM topic_links WHERE post_id = $1) \
         RETURNING args",
    )
    .bind(post_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut crawls = crawls;
    crawls.sort_by_key(|a| a["topic_link_id"].as_i64());
    for args in crawls {
        crate::jobs::enqueue(&mut *conn, "crawl_topic_link", args).await?;
    }
    Ok(())
}

/// ReviewableActionBuilder#delete_user with the queued post's
/// `delete_opts` (as a spammer, posts and links too; blocking the email
/// and IPs in production only), then the account_deleted email.
async fn delete_author(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    row: &Row,
    author: &Author,
    block: bool,
    mailer: Option<&crate::email::Mailer>,
) -> Result<(), AppError> {
    let email: Option<String> =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(author.id)
            .fetch_optional(&mut *conn)
            .await?;
    let production = ctx.config.rails_env == crate::config::RailsEnv::Production;
    let opts = crate::user_destroyer::Options {
        delete_posts: true,
        delete_as_spammer: true,
        block_urls: true,
        block_email: block && production,
        block_ip: block && production,
        context: Some(ctx.t("reviewables.actions.delete_user.reason")),
        reviewable_id: Some(row.id),
        ..Default::default()
    };
    // Boxed: the destroyer, as a spammer, performs on this reviewable.
    match Box::pin(crate::user_destroyer::destroy(
        conn, ctx, guardian, author.id, &opts,
    ))
    .await?
    {
        crate::user_destroyer::Outcome::Destroyed => {}
        // ensure_can_delete_user! raises; the actions were built for it.
        _ => return Err(Unsupported("a review queue delete the destroyer refuses").into()),
    }

    // UserNotifications.account_deleted(email, reviewable): the reason of
    // the first score's type, else spam's.
    let mailer = mailer.ok_or(Unsupported("the account_deleted email outside a request"))?;
    let email = email.ok_or(Unsupported("a deleted user without an email"))?;
    let score_type: Option<i32> = sqlx::query_scalar(
        "SELECT reviewable_score_type FROM reviewable_scores WHERE reviewable_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(row.id)
    .fetch_optional(&mut *conn)
    .await?;
    let flag_reason = match score_type {
        // flag_reasons.needs_approval wants a base_path it isn't given.
        None | Some(NEEDS_APPROVAL) => ctx.t("flag_reasons.spam"),
        Some(_) => {
            return Err(Unsupported("the account_deleted email for a flag's reason").into());
        }
    };
    let host = ctx.host;
    let sender = crate::email::sender::Ctx {
        host,
        settings: ctx.settings,
        config: ctx.config,
        i18n: ctx.i18n,
        mailer,
    };
    let built = crate::email::account::build_template(
        &mut *conn,
        &sender,
        "user_notifications.account_deleted",
        &email,
        &[("flag_reason", &flag_reason)],
        None,
    )
    .await?;
    crate::email::sender::send(&mut *conn, &sender, built, "account_deleted", None).await?;
    Ok(())
}

/// The reviewed post as the actions read it.
#[derive(sqlx::FromRow)]
struct ReviewedPost {
    id: i32,
    user_id: Option<i32>,
    topic_id: i32,
    post_number: i32,
    post_type: i32,
    hidden: bool,
    created_at: NaiveDateTime,
    deleted_at: Option<NaiveDateTime>,
}

#[allow(clippy::too_many_arguments)]
async fn reviewed(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    performer: &SessionUser,
    row: &Row,
    action_id: &str,
    version: Option<i64>,
    deferred: Option<&Deferred>,
) -> Result<Performed, AppError> {
    let s = ctx.settings;
    // build_actions: nothing unless pending.
    if row.status != PENDING {
        return Ok(Performed::InvalidAction);
    }
    let post: Option<ReviewedPost> = match row.target_id {
        Some(id) => sqlx::query_as(
            "SELECT id, user_id, topic_id, post_number, post_type, hidden, created_at, deleted_at \
                 FROM posts WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?,
        None => None,
    };
    let post = post.ok_or(Unsupported("a post review without its post"))?;
    if !guardian.is_admin() {
        return Err(Unsupported("moderators reviewing posts (reviewable_post_scope)").into());
    }
    let author = load_author(&mut *conn, row.target_created_by_id).await?;
    let trashed = post.deleted_at.is_some();

    // build_combined_actions
    let mut built: Vec<&str> = Vec::new();
    // can_recover_post?: staff moderate every topic.
    if trashed && guardian.is_staff() {
        built.push("approve_and_restore");
    } else if post.hidden {
        built.push("approve_and_unhide");
    } else {
        built.push("approve");
    }
    if trashed {
        built.push("reject_and_keep_deleted");
    } else {
        // can_delete_post_or_topic?
        let access =
            crate::posting::revisions::find_post_with_deleted(&mut *conn, ctx, guardian, post.id)
                .await?;
        let can_delete = match &access {
            Some(a) if post.post_number == 1 => guardian.can_delete_topic(s, &a.topic)?,
            Some(a) => guardian.can_delete_post(s, &a.topic, &a.post, a.can_see_post)?,
            None => false,
        };
        if can_delete {
            built.push("reject_and_delete");
        }
    }
    if can_penalize(guardian, author.as_ref())
        && let Some(a) = &author
    {
        if !a.silenced() {
            built.push("reject_and_silence");
        }
        if !a.suspended() {
            built.push("reject_and_suspend");
        }
    }
    if !validate(&built, &[], action_id) {
        return Ok(Performed::InvalidAction);
    }

    let Some(new_version) = increment_version(&mut *conn, row.id, version).await? else {
        return Ok(Performed::Conflict);
    };
    // The jobs the post's after-commit callbacks enqueue, at this
    // perform's commit unless an enclosing transaction holds them.
    let own = Deferred::default();
    let held = deferred.unwrap_or(&own);
    let transition = match action_id {
        "approve" => APPROVED,
        "approve_and_restore" => {
            crate::post_destroyer::staff_recover_in(
                &mut *conn,
                ctx,
                performer.id,
                post.id,
                Some(held),
            )
            .await?;
            APPROVED
        }
        "approve_and_unhide" => {
            unhide(&mut *conn, &post).await?;
            APPROVED
        }
        "reject_and_delete" => {
            let req = crate::post_destroyer::Request {
                context: None,
                reviewable_id: Some(row.id),
                deferred: Some(held),
            };
            crate::post_destroyer::destroy_by_staff_in(&mut *conn, ctx, guardian, post.id, &req)
                .await?;
            REJECTED
        }
        "reject_and_keep_deleted" | "reject_and_silence" | "reject_and_suspend" => REJECTED,
        _ => return Ok(Performed::InvalidAction),
    };
    transition_to(&mut *conn, ctx, row, transition, new_version, performer.id).await?;
    // update_flag_stats for [created_by_id], self-flags not counted.
    if Some(row.created_by_id) != post.user_id {
        let stat = if transition == APPROVED {
            "flags_agreed"
        } else {
            "flags_disagreed"
        };
        let total: Option<i64> = sqlx::query_scalar(&format!(
            "UPDATE user_stats SET {stat} = {stat} + 1 WHERE user_id = $1 \
             RETURNING (flags_agreed + flags_disagreed + flags_ignored)::int8"
        ))
        .bind(row.created_by_id)
        .fetch_optional(&mut *conn)
        .await?;
        if total.is_some_and(|t| t > FLAG_STATS_TRUNCATE_TO) {
            return Err(Unsupported("truncating flag stats (truncate_user_flag_stats)").into());
        }
    }
    if deferred.is_none() {
        let jobs = own
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (job, args) in jobs {
            crate::jobs::enqueue(&mut *conn, job, args).await?;
        }
    }
    let remove = vec![row.id];
    notify_reviewable(&mut *conn, row.id, performer, &remove).await?;
    Ok(Performed::Done {
        version: i64::from(new_version),
        remove,
        updates: None,
        created: None,
    })
}

/// `Post#unhide!` for a reply: shown again, the author's post count back
/// (UserStatCountUpdater.increment!), the topic's bumped_at back to it
/// when it is the last reply.
async fn unhide(conn: &mut PgConnection, post: &ReviewedPost) -> Result<(), AppError> {
    if post.post_number == 1 {
        return Err(Unsupported("unhiding a topic's first post (its visibility)").into());
    }
    sqlx::query("UPDATE posts SET hidden = FALSE, updated_at = clock_timestamp() WHERE id = $1")
        .bind(post.id)
        .execute(&mut *conn)
        .await?;
    let (highest, archetype): (i32, String) =
        sqlx::query_as("SELECT highest_post_number, archetype FROM topics WHERE id = $1")
            .bind(post.topic_id)
            .fetch_one(&mut *conn)
            .await?;
    if highest == post.post_number && post.post_type != WHISPER {
        sqlx::query(
            "UPDATE topics SET bumped_at = $2, updated_at = clock_timestamp() \
             WHERE id = $1 AND bumped_at IS DISTINCT FROM $2",
        )
        .bind(post.topic_id)
        .bind(post.created_at)
        .execute(&mut *conn)
        .await?;
    }
    if archetype != "private_message"
        && post.post_type == REGULAR
        && let Some(author) = post.user_id
    {
        sqlx::query("UPDATE user_stats SET post_count = post_count + 1 WHERE user_id = $1")
            .bind(author)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `reviewable.perform(actor, action)` from UserDestroyer#agree_with_flags
/// (delete_as_spammer): each of the user's post reviews that offers it
/// rejected and deleted, then each queued post that offers it rejected,
/// in id order.
pub async fn reject_for_spammer(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    actor: &Guardian,
    user_id: i32,
    deferred: &Deferred,
) -> Result<(), AppError> {
    for (kind, action) in [
        ("ReviewablePost", "reject_and_delete"),
        ("ReviewableQueuedPost", "reject_post"),
    ] {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM reviewables WHERE type = $1 AND target_created_by_id = $2 ORDER BY id",
        )
        .bind(kind)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        for id in ids {
            // actions_for(guardian).has?(action): only a perform that
            // offers it runs.
            match perform(
                &mut *conn,
                ctx,
                actor,
                id,
                action,
                None,
                &Args::default(),
                None,
                Some(deferred),
            )
            .await?
            {
                Performed::Done { .. } | Performed::InvalidAction => {}
                Performed::Conflict => {
                    return Err(Unsupported("a version conflict without a version").into());
                }
            }
        }
    }
    Ok(())
}
