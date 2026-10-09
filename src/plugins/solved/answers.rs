//! discourse-solved's AcceptAnswer and UnacceptAnswer services (POST
//! /solution/accept and /solution/unaccept): the topic's answers, the
//! answerer's SOLVED user action, and the notifications to the answerer,
//! the topic's owner and those tracking it. The caller holds the
//! transaction, then renders and publishes the accepted answers.
//!
//! Not ported: the controller's rate limits (RateLimiter, Redis, as for
//! likes), web hooks for accepted_solution and unaccepted_solution, and
//! the auto-close topic timer a solved topic gets when
//! solved_topics_auto_close_hours is set; each is refused when it would
//! apply.

use serde_json::json;
use sqlx::PgConnection;

use super::{TopicFacts, TopicView, category_field};
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_guardian::{PostCtx, TopicCtx};
use crate::{AppError, Unsupported};

/// `UserAction::SOLVED`
const SOLVED: i32 = 15;
/// `Notification.types[:custom]`
const CUSTOM: i32 = 14;

pub enum Outcome {
    /// The topic whose accepted answers the response shows.
    Done(i32),
    /// `Discourse::NotFound`
    NotFound,
    /// `Discourse::InvalidAccess`
    Forbidden,
    /// A policy other than the permission failed (the post is already an
    /// accepted answer): `failed_json`, 422.
    Failed,
}

/// `Post.find_by(id)` (deleted posts too for `with_deleted`) and its
/// topic id.
async fn find_post(
    conn: &mut PgConnection,
    post_id: i32,
    with_deleted: bool,
) -> Result<Option<(PostCtx, i32)>, sqlx::Error> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        user_id: Option<i32>,
        post_number: i32,
        post_type: i32,
        hidden: bool,
        hidden_at: Option<chrono::NaiveDateTime>,
        locked_by_id: Option<i32>,
        deleted_at: Option<chrono::NaiveDateTime>,
        user_deleted: bool,
        wiki: bool,
        created_at: chrono::NaiveDateTime,
        author_staff: Option<bool>,
        topic_id: i32,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT p.id, p.user_id, p.post_number, p.post_type, p.hidden, p.hidden_at, p.locked_by_id, \
                p.deleted_at, p.user_deleted, p.wiki, p.created_at, (u.admin OR u.moderator) AS author_staff, \
                p.topic_id \
         FROM posts p LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1 AND ($2 OR p.deleted_at IS NULL)",
    )
    .bind(post_id)
    .bind(with_deleted)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(|r| {
        (
            PostCtx {
                id: r.id,
                user_id: r.user_id,
                post_number: r.post_number,
                post_type: r.post_type,
                hidden: r.hidden,
                hidden_at: r.hidden_at,
                locked_by_id: r.locked_by_id,
                deleted_at: r.deleted_at,
                user_deleted: r.user_deleted,
                wiki: r.wiki,
                created_at: r.created_at,
                author_staff: r.author_staff.unwrap_or(false),
            },
            r.topic_id,
        )
    }))
}

/// The post, its topic (deleted topics for staff), and solved's state.
struct Target {
    post: PostCtx,
    topic: TopicCtx,
    title: String,
    view: TopicView,
    can_see_post: bool,
}

async fn target(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i32,
    post_with_deleted: bool,
) -> Result<Option<Target>, AppError> {
    let Some((post, topic_id)) = find_post(conn, post_id, post_with_deleted).await? else {
        return Ok(None);
    };
    // fetch_topic: Topic.with_deleted for staff, else post.topic.
    let Some(topic) = TopicCtx::load(&mut *conn, settings, guardian, topic_id).await? else {
        return Ok(None);
    };
    if topic.deleted_at.is_some() && !guardian.is_staff() {
        return Ok(None);
    }
    let title: String = sqlx::query_scalar("SELECT title FROM topics WHERE id = $1")
        .bind(topic.id)
        .fetch_one(&mut *conn)
        .await?;
    let facts = TopicFacts {
        id: topic.id,
        user_id: topic.user_id,
        category_id: topic.category_id,
        archetype: topic.archetype.clone(),
        closed: topic.closed,
        archived: topic.archived,
        deleted: topic.deleted_at.is_some(),
    };
    let view = TopicView::load(conn, settings, guardian, facts).await?;
    let secure = guardian.secure_category_ids(&mut *conn, settings).await?;
    let can_see_topic = guardian.can_see_topic(settings, &topic, true, &secure)?;
    let can_see_post = guardian.can_see_post(settings, &post, can_see_topic)?;
    Ok(Some(Target {
        post,
        topic,
        title,
        view,
        can_see_post,
    }))
}

/// `can_accept_answer?(topic, post)`
fn can_accept(t: &Target, guardian: &Guardian) -> bool {
    t.can_see_post
        && t.view
            .can_accept(guardian, t.post.post_number, t.post.post_type == 4)
}

/// `WebHook.active_web_hooks(event).exists?`
/// The topic's row, locked (`lock(:topic)`).
async fn lock_topic(conn: &mut PgConnection, topic_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT id FROM topics WHERE id = $1 FOR UPDATE")
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// The solved row and its answers, gone (`solved.destroy!`, its answers
/// and topic timer with it).
async fn destroy_solved(conn: &mut PgConnection, view: &TopicView) -> Result<(), AppError> {
    let Some(solved_id) = view.solved_id else {
        return Ok(());
    };
    if view.topic_timer_id.is_some() {
        return Err(Unsupported("a solved topic's auto-close timer").into());
    }
    sqlx::query("DELETE FROM discourse_solved_topic_answers WHERE solved_topic_id = $1")
        .bind(solved_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM discourse_solved_solved_topics WHERE id = $1")
        .bind(solved_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `UserCommScreener#ignoring_or_muting_actor?`: the user ignores or mutes
/// the acting user (nobody screens staff).
async fn screened(
    conn: &mut PgConnection,
    guardian: &Guardian,
    user_id: i32,
) -> Result<bool, sqlx::Error> {
    if guardian.is_staff() {
        return Ok(false);
    }
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM muted_users WHERE user_id = $1 AND muted_user_id = $2) \
         OR EXISTS (SELECT 1 FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2)",
    )
    .bind(user_id)
    .bind(guardian.user_id())
    .fetch_one(&mut *conn)
    .await
}

/// `UserOption.exists?(user_id:, notify_on_solved: true)` (the user
/// existing).
async fn wants_notice(conn: &mut PgConnection, user_id: i32) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users u JOIN user_options o ON o.user_id = u.id \
         WHERE u.id = $1 AND o.notify_on_solved)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await
}

/// `Notification.create!` of a custom notification, and its after_commit
/// notification state.
async fn notify(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    user_id: i32,
    topic_id: i32,
    post_number: i32,
    data: serde_json::Value,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO notifications (notification_type, user_id, topic_id, post_number, data, read, \
                                    high_priority, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(CUSTOM)
    .bind(user_id)
    .bind(topic_id)
    .bind(post_number)
    .bind(data.to_string())
    .execute(&mut *conn)
    .await?;
    crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
    Ok(())
}

/// `DiscourseSolved::AcceptAnswer`
pub async fn accept(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Outcome, AppError> {
    let Some(t) = target(conn, settings, guardian, post_id, false).await? else {
        return Ok(Outcome::NotFound);
    };
    if !can_accept(&t, guardian) {
        return Ok(Outcome::Forbidden);
    }
    // answer_is_acceptable
    if t.view.accepted(t.post.id) {
        return Ok(Outcome::Failed);
    }
    if crate::plugins::web_hooks_active(conn, "accepted_solution").await? {
        return Err(Unsupported("accepted_solution web hooks").into());
    }
    let me = guardian
        .user_id()
        .ok_or(Unsupported("accepting anonymously"))?;
    let Some(author) = t.post.user_id else {
        return Err(Unsupported("accepting a post without a user").into());
    };
    let topic_id = t.topic.id;
    lock_topic(conn, topic_id).await?;

    // revoke_previous_accepted_answer
    if !settings.get("solved_allow_multiple_solutions")?.truthy() && !t.view.answers.is_empty() {
        let post_ids: Vec<i32> = t
            .view
            .answers
            .iter()
            .map(|a| a.answer_post_id as i32)
            .collect();
        sqlx::query("DELETE FROM user_actions WHERE action_type = $1 AND target_post_id = ANY($2)")
            .bind(SOLVED)
            .bind(&post_ids)
            .execute(&mut *conn)
            .await?;
        destroy_solved(conn, &t.view).await?;
    }
    // credit_post_author
    crate::posting::log_user_action_by(
        conn,
        SOLVED,
        author,
        me,
        topic_id,
        t.post.id,
        crate::clock::now_naive(),
    )
    .await?;
    // find_or_create_solved_topic (a new one gets the auto-close timer).
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM discourse_solved_solved_topics WHERE topic_id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let solved_id = match existing {
        Some(id) => id,
        None => {
            let category_hours =
                category_field(conn, t.topic.category_id, "solved_topics_auto_close_hours")
                    .await?
                    .map(|v| crate::ruby::to_i(&v))
                    .unwrap_or(0);
            let hours = if category_hours == 0 {
                settings.get("solved_topics_auto_close_hours")?.to_i()
            } else {
                category_hours
            };
            if hours != 0 && !t.topic.closed {
                return Err(Unsupported("solved_topics_auto_close_hours (topic timers)").into());
            }
            sqlx::query_scalar(
                "INSERT INTO discourse_solved_solved_topics (topic_id, created_at, updated_at) \
                 VALUES ($1, clock_timestamp(), clock_timestamp()) RETURNING id",
            )
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    // create_topic_answer
    sqlx::query(
        "INSERT INTO discourse_solved_topic_answers (solved_topic_id, answer_post_id, accepter_user_id, \
                                                     created_at, updated_at) \
         VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
    )
    .bind(solved_id)
    .bind(i64::from(t.post.id))
    .bind(i64::from(me))
    .execute(&mut *conn)
    .await?;

    let username = guardian
        .user()
        .map(|u| u.username.clone())
        .unwrap_or_default();
    let accepted = json!({
        "message": "solved.accepted_notification",
        "display_username": username,
        "topic_title": t.title,
        "title": "solved.notification.title",
    });
    // notify_post_author
    if me != author
        && wants_notice(conn, author).await?
        && !screened(conn, guardian, author).await?
    {
        notify(
            conn,
            bus,
            settings,
            author,
            topic_id,
            t.post.post_number,
            accepted.clone(),
        )
        .await?;
    }
    // notify_topic_owner
    if let Some(owner) = t.topic.user_id
        && category_field(conn, t.topic.category_id, "notify_on_staff_accept_solved")
            .await?
            .as_deref()
            == Some("true")
        && me != owner
        && wants_notice(conn, owner).await?
        && !screened(conn, guardian, owner).await?
    {
        notify(
            conn,
            bus,
            settings,
            owner,
            topic_id,
            t.post.post_number,
            accepted,
        )
        .await?;
    }
    // notify_tracking_and_watching_users
    let mut skip = vec![me, author];
    skip.extend(t.topic.user_id);
    let trackers: Vec<i32> = sqlx::query_scalar(
        "SELECT tu.user_id FROM topic_users tu \
         JOIN users u ON u.id = tu.user_id JOIN user_options o ON o.user_id = u.id \
         WHERE tu.topic_id = $1 AND tu.notification_level >= 2 \
         AND NOT (tu.user_id = ANY($2)) AND o.notify_on_solved ORDER BY tu.id",
    )
    .bind(topic_id)
    .bind(&skip)
    .fetch_all(&mut *conn)
    .await?;
    let solved = json!({
        "message": "solved.topic_solved_notification",
        "display_username": username,
        "topic_title": t.title,
        "title": "solved.notification.topic_solved_title",
    });
    for user_id in trackers {
        if !screened(conn, guardian, user_id).await? {
            notify(
                conn,
                bus,
                settings,
                user_id,
                topic_id,
                t.post.post_number,
                solved.clone(),
            )
            .await?;
        }
    }
    Ok(Outcome::Done(topic_id))
}

/// `DiscourseSolved::UnacceptAnswer`
pub async fn unaccept(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    guardian: &Guardian,
    post_id: i32,
) -> Result<Outcome, AppError> {
    let Some(t) = target(conn, settings, guardian, post_id, guardian.is_staff()).await? else {
        return Ok(Outcome::NotFound);
    };
    let accepted = t.view.accepted(t.post.id);
    // can_unaccept_answer?
    if !(can_accept(&t, guardian) || (guardian.is_staff() && accepted)) {
        return Ok(Outcome::Forbidden);
    }
    let topic_id = t.topic.id;
    if !accepted {
        return Ok(Outcome::Done(topic_id));
    }
    if crate::plugins::web_hooks_active(conn, "unaccepted_solution").await? {
        return Err(Unsupported("unaccepted_solution web hooks").into());
    }
    lock_topic(conn, topic_id).await?;
    // revoke_solved_credit
    sqlx::query("DELETE FROM user_actions WHERE action_type = $1 AND target_post_id = $2")
        .bind(SOLVED)
        .bind(t.post.id)
        .execute(&mut *conn)
        .await?;
    // remove_accepted_answer_notification: the answerer's, one.
    let mut removed: Vec<i32> = sqlx::query_scalar(
        "DELETE FROM notifications WHERE id = (SELECT id FROM notifications \
           WHERE topic_id = $1 AND notification_type = $2 AND user_id = $3 AND post_number = $4 \
           ORDER BY id LIMIT 1) RETURNING user_id",
    )
    .bind(topic_id)
    .bind(CUSTOM)
    .bind(t.post.user_id)
    .bind(t.post.post_number)
    .fetch_all(&mut *conn)
    .await?;
    // remove_topic_solved_notifications
    removed.extend(
        sqlx::query_scalar::<_, i32>(
            "DELETE FROM notifications WHERE topic_id = $1 AND notification_type = $2 \
             AND post_number = $3 AND data::jsonb @> '{\"message\": \"solved.topic_solved_notification\"}' \
             RETURNING user_id",
        )
        .bind(topic_id)
        .bind(CUSTOM)
        .bind(t.post.post_number)
        .fetch_all(&mut *conn)
        .await?,
    );
    // unmark_as_solved: the answer, and the solved row once none are left.
    sqlx::query("DELETE FROM discourse_solved_topic_answers WHERE answer_post_id = $1")
        .bind(i64::from(t.post.id))
        .execute(&mut *conn)
        .await?;
    if t.view.answers.len() == 1 {
        destroy_solved(conn, &t.view).await?;
    }
    // Each destroyed notification's after_commit notification state.
    for user_id in removed {
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, user_id).await?;
    }
    Ok(Outcome::Done(topic_id))
}
