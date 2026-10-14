//! The topic timer jobs: Jobs::TopicTimerEnqueuer (every minute, the
//! timers due), and the typed jobs it enqueues through TopicTimerBase:
//! close_topic, open_topic, bump_topic, delete_topic, delete_replies and
//! clear_slow_mode.
//!
//! Refused: publish_topic_to_category (TopicPublisher needs a topic's
//! category change through PostRevisor, TopicTimestampChanger and
//! TopicConverter).

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::posting::Ctx;
use crate::posting::small_action::{self, SmallAction};
use crate::site_settings::SiteSettings;
use crate::topic_status::{self, Status};
use crate::topic_timer::{self, SYSTEM_USER_ID, Timer, types};
use crate::{AppError, AppState, Unsupported};

pub const ENQUEUER: &str = "topic_timer_enqueuer";

/// The typed jobs, by name.
pub const JOBS: &[&str] = &[
    "close_topic",
    "open_topic",
    "publish_topic_to_category",
    "delete_topic",
    "bump_topic",
    "delete_replies",
    "clear_slow_mode",
];

/// `Jobs::TopicTimerEnqueuer`: `TopicTimer.pending_timers.find_each`,
/// each one's typed job enqueued. A reminder timer's job does not exist,
/// and Rails logs the failure and goes on.
pub async fn enqueue_pending(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let pending: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT id, status_type FROM topic_timers \
         WHERE type = 'TopicTimer' AND execute_at <= $1 AND deleted_at IS NULL ORDER BY id",
    )
    .bind(crate::clock::now_naive())
    .fetch_all(&mut *conn)
    .await?;
    for (id, status_type) in pending {
        let job = match status_type {
            types::CLOSE => ("close_topic", json!({ "topic_timer_id": id })),
            types::SILENT_CLOSE => (
                "close_topic",
                json!({ "topic_timer_id": id, "silent": true }),
            ),
            types::OPEN => ("open_topic", json!({ "topic_timer_id": id })),
            types::PUBLISH_TO_CATEGORY => {
                ("publish_topic_to_category", json!({ "topic_timer_id": id }))
            }
            types::DELETE => ("delete_topic", json!({ "topic_timer_id": id })),
            types::BUMP => ("bump_topic", json!({ "topic_timer_id": id })),
            types::DELETE_REPLIES => ("delete_replies", json!({ "topic_timer_id": id })),
            types::CLEAR_SLOW_MODE => ("clear_slow_mode", json!({ "topic_timer_id": id })),
            _ => {
                tracing::warn!(
                    timer = id,
                    "Error when attempting to enqueue topic timer job for timer {id}"
                );
                continue;
            }
        };
        crate::jobs::enqueue(&mut conn, job.0, job.1).await?;
    }
    Ok(())
}

/// One of the typed jobs: `TopicTimerBase#execute`. A trashed or not yet
/// due timer does nothing; one on a deleted topic is destroyed outright.
pub async fn run(state: &AppState, name: &str, args: &Value) -> Result<(), AppError> {
    let Some(timer_id) = args
        .get("topic_timer_id")
        .and_then(Value::as_i64)
        .map(|v| v as i32)
    else {
        return Err(Unsupported("a topic timer job without its topic_timer_id").into());
    };
    let silent = args
        .get("silent")
        .is_some_and(|v| v.as_bool() == Some(true));
    let mut conn = state.pool.acquire().await?;
    let timer: Option<Timer> = sqlx::query_as(
        "SELECT id, execute_at, status_type, user_id, timerable_id, created_at, based_on_last_post, \
                duration_minutes, category_id \
         FROM topic_timers WHERE id = $1 AND type = 'TopicTimer' AND deleted_at IS NULL",
    )
    .bind(timer_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(timer) = timer.filter(|t| t.execute_at <= crate::clock::now_naive()) else {
        return Ok(());
    };
    let topic_live: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(timer.timerable_id)
    .fetch_one(&mut *conn)
    .await?;
    if !topic_live {
        destroy(&mut conn, timer.id).await?;
        return Ok(());
    }
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    // The PostDestroyer ones run their own transactions.
    match name {
        "delete_topic" => return delete_topic(state, &ctx, &timer).await,
        "delete_replies" => return delete_replies(state, &ctx, &timer).await,
        _ => {}
    }
    let mut tx = state.pool.begin().await?;
    match name {
        "close_topic" => close_topic(&mut tx, &ctx, &timer, silent).await?,
        "open_topic" => open_topic(&mut tx, &ctx, &timer).await?,
        "bump_topic" => bump_topic(&mut tx, &ctx, &timer).await?,
        "clear_slow_mode" => clear_slow_mode(&mut tx, &ctx, &timer).await?,
        "publish_topic_to_category" => {
            return Err(Unsupported("publish_topic_to_category (TopicPublisher)").into());
        }
        other => {
            return Err(std::io::Error::other(format!("unknown topic timer job {other}")).into());
        }
    }
    tx.commit().await?;
    Ok(())
}

/// `topic_timer.destroy!`: deleted outright.
async fn destroy(conn: &mut PgConnection, timer_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM topic_timers WHERE id = $1")
        .bind(timer_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// `MessageBus.publish("/topic/:id", {reload_topic: true}, ...)`
async fn reload_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
) -> Result<(), AppError> {
    crate::bus::publish_to_topic(ctx.bus, conn, topic_id, &json!({ "reload_topic": true })).await
}

/// `Jobs::CloseTopic`
async fn close_topic(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    timer: &Timer,
    silent: bool,
) -> Result<(), AppError> {
    let topic_id = timer.timerable_id;
    let closed: bool = sqlx::query_scalar("SELECT closed FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if closed {
        destroy(conn, timer.id).await?;
        return Ok(());
    }
    if !topic_timer::can_set_topic_timer(&mut *conn, ctx.settings, Some(timer.user_id), topic_id)
        .await?
    {
        destroy(&mut *conn, timer.id).await?;
        if timer.based_on_last_post {
            let kind = if silent {
                types::SILENT_CLOSE
            } else {
                types::CLOSE
            };
            topic_timer::inherit_auto_close_from_category(&mut *conn, ctx, topic_id, kind).await?;
        }
        return Ok(());
    }
    // TopicStatusUpdater finishes the timer.
    topic_status::apply(
        &mut *conn,
        ctx,
        topic_id,
        timer.user_id,
        Status::Autoclosed,
        true,
        None,
        silent,
    )
    .await?;
    reload_topic(conn, ctx, topic_id).await
}

/// `Jobs::OpenTopic`
async fn open_topic(conn: &mut PgConnection, ctx: &Ctx<'_>, timer: &Timer) -> Result<(), AppError> {
    let topic_id = timer.timerable_id;
    let closed: bool = sqlx::query_scalar("SELECT closed FROM topics WHERE id = $1")
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
    if !topic_timer::can_set_topic_timer(&mut *conn, ctx.settings, Some(timer.user_id), topic_id)
        .await?
        || !closed
    {
        destroy(&mut *conn, timer.id).await?;
        topic_timer::inherit_auto_close_from_category(&mut *conn, ctx, topic_id, types::CLOSE)
            .await?;
        return Ok(());
    }
    if topic_timer::auto_close_threshold_reached(&mut *conn, ctx.settings, topic_id).await? {
        // Still flagged: the open timer moves on by num_hours_to_close_topic.
        let hours = ctx.settings.get("num_hours_to_close_topic")?.to_i() as f64;
        topic_timer::set_or_create_timer(
            &mut *conn,
            ctx,
            topic_id,
            types::OPEN,
            topic_timer::When::Hours(hours),
            &topic_timer::SetOptions {
                by_user: Some(SYSTEM_USER_ID),
                ..Default::default()
            },
        )
        .await?;
    } else {
        topic_status::apply(
            &mut *conn,
            ctx,
            topic_id,
            timer.user_id,
            Status::Autoclosed,
            false,
            None,
            false,
        )
        .await?;
    }
    topic_timer::inherit_auto_close_from_category(&mut *conn, ctx, topic_id, types::CLOSE).await?;
    reload_topic(conn, ctx, topic_id).await
}

/// The timer user's guardian and the topic as it sees it, None for a
/// user who is gone.
async fn user_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    topic_id: i32,
) -> Result<Option<(crate::guardian::Guardian, crate::topic_guardian::TopicCtx)>, AppError> {
    if crate::session::current::SessionUser::load(&mut *conn, user_id)
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let guardian = crate::onebox::guardian_for(&mut *conn, Some(user_id)).await?;
    let topic = crate::topic_guardian::TopicCtx::load(&mut *conn, settings, &guardian, topic_id)
        .await?
        .ok_or(Unsupported("a timer's topic that went missing"))?;
    Ok(Some((guardian, topic)))
}

/// `Jobs::BumpTopic`
async fn bump_topic(conn: &mut PgConnection, ctx: &Ctx<'_>, timer: &Timer) -> Result<(), AppError> {
    let s = ctx.settings;
    let topic_id = timer.timerable_id;
    let allowed = match user_topic(&mut *conn, s, timer.user_id, topic_id).await? {
        Some((guardian, topic)) => {
            let anywhere = guardian.can_create_post_anywhere(&mut *conn, s).await?;
            guardian.can_create_post_on_topic(s, &topic, anywhere)?
                || topic_timer::can_set_topic_timer(&mut *conn, s, Some(timer.user_id), topic_id)
                    .await?
        }
        None => false,
    };
    if !allowed {
        topic_timer::finish(conn, timer.id, Some(SYSTEM_USER_ID)).await?;
        return Ok(());
    }
    small_action::add(
        &mut *conn,
        ctx,
        &SmallAction {
            user_id: SYSTEM_USER_ID,
            topic_id,
            action_code: "autobumped",
            bump: true,
            who: None,
        },
    )
    .await?;
    topic_timer::finish(conn, timer.id, Some(SYSTEM_USER_ID)).await?;
    Ok(())
}

/// `Guardian.new(timer.user).can_delete?(topic)`
async fn can_delete_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    timer: &Timer,
) -> Result<bool, AppError> {
    Ok(
        match user_topic(conn, settings, timer.user_id, timer.timerable_id).await? {
            Some((guardian, topic)) => guardian.can_delete_topic(settings, &topic)?,
            None => false,
        },
    )
}

/// The newest queued job's id, to tell the jobs enqueued after it.
async fn last_job_id(state: &AppState) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(MAX(id), 0) FROM discourse_rs.jobs")
        .fetch_one(&state.pool)
        .await
}

/// The timer jobs run in one transaction in Rails, so the after_commit
/// work PostDestroyer sets off (Category#index_search) waits for its end:
/// those jobs move behind the rest, once per category.
async fn defer_after_commit_jobs(state: &AppState, since: i64) -> Result<(), sqlx::Error> {
    let mut tx = state.pool.begin().await?;
    let deferred: Vec<Value> = sqlx::query_scalar(
        "SELECT args FROM discourse_rs.jobs WHERE id > $1 AND name = 'index_category_for_search' \
         ORDER BY id",
    )
    .bind(since)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM discourse_rs.jobs WHERE id > $1 AND name = 'index_category_for_search'",
    )
    .bind(since)
    .execute(&mut *tx)
    .await?;
    let mut seen = Vec::new();
    for args in deferred {
        if !seen.contains(&args) {
            crate::jobs::enqueue(&mut tx, "index_category_for_search", args.clone()).await?;
            seen.push(args);
        }
    }
    tx.commit().await
}

/// `Jobs::DeleteTopic`: the first post destroyed by the timer's user.
async fn delete_topic(state: &AppState, ctx: &Ctx<'_>, timer: &Timer) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    if !can_delete_topic(&mut conn, ctx.settings, timer).await? {
        return Ok(());
    }
    let first: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 AND deleted_at IS NULL ORDER BY post_number LIMIT 1",
    )
    .bind(timer.timerable_id)
    .fetch_optional(&mut *conn)
    .await?;
    drop(conn);
    let Some(first) = first else {
        return Err(Unsupported("deleting a topic without posts").into());
    };
    let since = last_job_id(state).await?;
    let context = ctx.t("topic_statuses.auto_deleted_by_timer");
    crate::post_destroyer::destroy_as(&state.pool, ctx, timer.user_id, first, &context).await?;
    let mut conn = state.pool.acquire().await?;
    topic_timer::finish(&mut conn, timer.id, Some(SYSTEM_USER_ID)).await?;
    defer_after_commit_jobs(state, since).await?;
    Ok(())
}

/// `Jobs::DeleteReplies`: replies older than the duration (and with fewer
/// likes than skip_auto_delete_reply_likes) destroyed by the timer's
/// user, then the timer rescheduled from the oldest reply left.
async fn delete_replies(state: &AppState, ctx: &Ctx<'_>, timer: &Timer) -> Result<(), AppError> {
    let s = ctx.settings;
    let mut conn = state.pool.acquire().await?;
    if !can_delete_topic(&mut conn, s, timer).await? {
        topic_timer::finish(&mut conn, timer.id, Some(SYSTEM_USER_ID)).await?;
        return Ok(());
    }
    let skip_likes = s.get("skip_auto_delete_reply_likes")?.to_i();
    let duration = chrono::Duration::minutes(i64::from(timer.duration_minutes.unwrap_or(0)));
    // The replies relation: live replies, under the like limit when set.
    let replies_sql = "FROM posts WHERE topic_id = $1 AND deleted_at IS NULL AND post_number > 1 \
                       AND ($2 <= 0 OR like_count < $2)";
    let old: Vec<i32> = sqlx::query_scalar(&format!("SELECT id {replies_sql} AND created_at < $3"))
        .bind(timer.timerable_id)
        .bind(skip_likes)
        .bind(crate::clock::now_naive() - duration)
        .fetch_all(&mut *conn)
        .await?;
    drop(conn);
    let since = last_job_id(state).await?;
    let context = ctx.t("topic_statuses.auto_deleted_by_timer");
    for post_id in old {
        crate::post_destroyer::destroy_as(&state.pool, ctx, timer.user_id, post_id, &context)
            .await?;
    }
    let mut tx = state.pool.begin().await?;
    let oldest: Option<chrono::NaiveDateTime> =
        sqlx::query_scalar(&format!("SELECT MIN(created_at) {replies_sql}"))
            .bind(timer.timerable_id)
            .bind(skip_likes)
            .fetch_one(&mut *tx)
            .await?;
    let execute_at = oldest.unwrap_or_else(crate::clock::now_naive) + duration;
    // topic_timer.save: a validation failure (not after created_at)
    // leaves it as it was.
    if execute_at > timer.created_at && execute_at != timer.execute_at {
        sqlx::query(
            "UPDATE topic_timers SET execute_at = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(timer.id)
        .bind(execute_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    defer_after_commit_jobs(state, since).await?;
    Ok(())
}

/// `Jobs::ClearSlowMode`: `topic.update!(slow_mode_seconds: 0)`, a
/// validated save, then the timer finished.
async fn clear_slow_mode(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    timer: &Timer,
) -> Result<(), AppError> {
    set_slow_mode_seconds(conn, ctx.settings, timer.timerable_id, 0).await?;
    topic_timer::finish(conn, timer.id, Some(SYSTEM_USER_ID)).await?;
    Ok(())
}

/// `topic.update!(slow_mode_seconds:)`: the title reassigned on the way
/// (slug, fancy_title), written when anything differs.
pub async fn set_slow_mode_seconds(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    seconds: i32,
) -> Result<(), AppError> {
    let slug = crate::posting::topic_save::reassigned_slug(&mut *conn, settings, topic_id).await?;
    sqlx::query(
        "UPDATE topics SET slow_mode_seconds = $2, slug = $3, fancy_title = NULL, \
                           updated_at = clock_timestamp() \
         WHERE id = $1 AND (slow_mode_seconds IS DISTINCT FROM $2 OR slug IS DISTINCT FROM $3 \
                            OR fancy_title IS NOT NULL)",
    )
    .bind(topic_id)
    .bind(seconds)
    .bind(&slug)
    .execute(conn)
    .await?;
    Ok(())
}
