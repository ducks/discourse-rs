//! `Topic#update_status` -> TopicStatusUpdater for closing, opening,
//! archiving, unarchiving, listing, unlisting, pinning and unpinning a
//! topic: the column, the small action post saying so, what the change
//! takes with it (featured topics, the category's count, hot scores), and
//! the staff action log. Pinning until a time schedules the unpin job
//! (`Topic#update_pinned`, `Jobs::UnpinTopic`).

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::Ctx;
use crate::posting::small_action::{self, SmallAction};
use crate::{AppError, Unsupported};

/// `UserHistory.actions`
const TOPIC_CLOSED: i32 = 91;
const TOPIC_OPENED: i32 = 92;
const TOPIC_ARCHIVED: i32 = 93;
const TOPIC_UNARCHIVED: i32 = 94;
/// `Topic.visibility_reasons`
const MANUALLY_UNLISTED: i32 = 3;
const MANUALLY_RELISTED: i32 = 4;

/// `TopicStatusUpdater::Status` names the controller takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Closed,
    Archived,
    Visible,
    Pinned,
    PinnedGlobally,
    /// `autoclosed`: the timer jobs' close and open, not a status the
    /// controller takes.
    Autoclosed,
}

impl Status {
    /// `check_for_status_presence`
    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "closed" => Status::Closed,
            "archived" => Status::Archived,
            "visible" => Status::Visible,
            "pinned" => Status::Pinned,
            "pinned_globally" => Status::PinnedGlobally,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Status::Closed => "closed",
            Status::Archived => "archived",
            Status::Visible => "visible",
            Status::Pinned => "pinned",
            Status::PinnedGlobally => "pinned_globally",
            Status::Autoclosed => "autoclosed",
        }
    }
}

/// `update_status`'s options.
#[derive(Default)]
pub struct StatusOpts<'a> {
    /// `until:` for a pin.
    pub until: Option<&'a str>,
    /// `silent:`: the small action leaves the bump alone.
    pub silent: bool,
    /// `message:`: the small action's raw text.
    pub message: Option<&'a str>,
    /// `visibility_reason_id:` for a visibility change (manually unlisted
    /// or relisted when not given).
    pub visibility_reason_id: Option<i32>,
}

/// How a status update ends when it isn't a server error.
pub enum Outcome {
    Done,
    /// `Discourse::InvalidAccess` from the guardian (also for a missing
    /// topic, which the guardian refuses).
    Forbidden,
}

#[derive(sqlx::FromRow)]
struct Topic {
    id: i32,
    archetype: String,
    category_id: Option<i32>,
}

/// `TopicsController#status` from the topic lookup on.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    guardian: &Guardian,
    topic_id: i32,
    category_id: Option<i32>,
    status: Status,
    enabled: bool,
    until: Option<&str>,
) -> Result<Outcome, AppError> {
    let s = ctx.settings;
    let topic: Option<Topic> = sqlx::query_as(
        "SELECT id, archetype, category_id FROM topics WHERE id = $1 AND deleted_at IS NULL \
           AND ($2::int IS NULL OR category_id = $2)",
    )
    .bind(topic_id)
    .bind(category_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(topic) = topic else {
        return Ok(Outcome::Forbidden);
    };
    let topic_ctx = crate::topic_guardian::TopicCtx::load(&mut *conn, s, guardian, topic.id)
        .await?
        .ok_or(Unsupported("loading a topic being moderated"))?;
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let can_see = guardian.can_see_topic(s, &topic_ctx, true, &secure)?;
    let allowed = match status {
        Status::Closed | Status::Archived | Status::Pinned => {
            guardian.can_perform_action_available_to_group_moderators(s, can_see)?
        }
        Status::Visible => {
            guardian.can_moderate(can_see)
                || guardian.can_perform_action_available_to_group_moderators(s, can_see)?
        }
        Status::PinnedGlobally | Status::Autoclosed => guardian.can_moderate(can_see),
    };
    if !allowed {
        return Ok(Outcome::Forbidden);
    }
    let user = guardian
        .user()
        .ok_or(Unsupported("changing a topic's status anonymously"))?;
    let opts = StatusOpts {
        until,
        ..Default::default()
    };
    apply(conn, ctx, topic.id, user.id, status, enabled, &opts).await?;
    Ok(Outcome::Done)
}

/// `Topic#update_status(status, enabled, user)`: TopicStatusUpdater, the
/// staff action log, and a closed message's group archives, for a topic
/// that exists.
pub async fn apply(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    user_id: i32,
    status: Status,
    enabled: bool,
    opts: &StatusOpts<'_>,
) -> Result<(), AppError> {
    let until = opts.until;
    let s = ctx.settings;
    let topic: Topic =
        sqlx::query_as("SELECT id, archetype, category_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&mut *conn)
            .await?;
    // @topic_timer: closing and opening act on the public timer.
    let topic_timer = crate::topic_timer::public_topic_timer(&mut *conn, topic.id).await?;
    // previous_pin, for a pin's change.
    let pin_sql = "SELECT pinned_at, pinned_globally, pinned_until FROM topics WHERE id = $1";
    type Pin = (
        Option<chrono::NaiveDateTime>,
        bool,
        Option<chrono::NaiveDateTime>,
    );
    let is_pin = matches!(status, Status::Pinned | Status::PinnedGlobally);
    let previous_pin: Option<Pin> = if is_pin {
        Some(
            sqlx::query_as(pin_sql)
                .bind(topic.id)
                .fetch_one(&mut *conn)
                .await?,
        )
    } else {
        None
    };

    // change
    let updated = match status {
        Status::Pinned | Status::PinnedGlobally => {
            update_pinned(
                &mut *conn,
                topic.id,
                enabled,
                status == Status::PinnedGlobally,
                until,
            )
            .await?;
            true
        }
        Status::Closed | Status::Archived | Status::Visible | Status::Autoclosed => {
            let column = match status {
                Status::Autoclosed => "closed",
                _ => status.name(),
            };
            sqlx::query(&format!(
                "UPDATE topics SET {column} = $2 WHERE id = $1 AND {column} = NOT $2"
            ))
            .bind(topic.id)
            .bind(enabled)
            .execute(&mut *conn)
            .await?
            .rows_affected()
                > 0
        }
    };
    // CategoryFeaturedTopic.where(topic_id:).delete_all, for a topic
    // closed, archived or unlisted.
    let unfeature = (enabled
        && matches!(
            status,
            Status::Closed | Status::Archived | Status::Autoclosed
        ))
        || (!enabled && status == Status::Visible);
    if status == Status::Visible && !enabled {
        // UserProfile.remove_featured_topic_from_all_profiles
        sqlx::query(
            "UPDATE user_profiles SET featured_topic_id = NULL WHERE featured_topic_id = $1",
        )
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
    }
    if status == Status::Visible && updated {
        // update_category_topic_count_by, but not for the category's
        // definition topic.
        if let Some(category_id) = topic.category_id {
            sqlx::query(
                "UPDATE categories SET topic_count = topic_count + $3 \
                 WHERE id = $1 AND (topic_id <> $2 OR topic_id IS NULL)",
            )
            .bind(category_id)
            .bind(topic.id)
            .bind(if enabled { 1 } else { -1 })
            .execute(&mut *conn)
            .await?;
        }
        // The featured rows (below in Rails) go while only the category's
        // row is locked, before the author's user_stats: a new topic's
        // order (see small_action::add_unfeaturing).
        if unfeature {
            unfeature_topic(&mut *conn, topic.id).await?;
        }
        // UserStatCountUpdater on the first post: its author's topic count,
        // not below zero.
        let author: Option<Option<i32>> =
            sqlx::query_scalar("SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL")
                .bind(topic.id)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(Some(author)) = author {
            let sql = if enabled {
                "UPDATE user_stats SET topic_count = topic_count + 1 WHERE user_id = $1"
            } else {
                "UPDATE user_stats SET topic_count = topic_count - 1 WHERE user_id = $1 AND topic_count >= 1"
            };
            sqlx::query(sql).bind(author).execute(&mut *conn).await?;
        }
    }
    if status == Status::Visible {
        // topic.update(visibility_reason_id:), a validated save.
        let slug = crate::posting::topic_save::reassigned_slug(&mut *conn, s, topic.id).await?;
        sqlx::query(
            "UPDATE topics SET visibility_reason_id = $2, slug = $3, fancy_title = NULL, \
                               updated_at = clock_timestamp() \
             WHERE id = $1 AND (visibility_reason_id IS DISTINCT FROM $2 OR slug IS DISTINCT FROM $3 \
                                OR fancy_title IS NOT NULL)",
        )
        .bind(topic.id)
        .bind(opts.visibility_reason_id.unwrap_or(if enabled {
            MANUALLY_RELISTED
        } else {
            MANUALLY_UNLISTED
        }))
        .bind(&slug)
        .execute(&mut *conn)
        .await?;
    }
    // The public timer: closing finishes close timers, opening finishes
    // the open timer and takes the category's auto close again.
    if topic_timer.is_some() && matches!(status, Status::Closed | Status::Autoclosed) {
        use crate::topic_timer::{delete_topic_timer, types};
        let by = Some(user_id);
        if enabled {
            delete_topic_timer(&mut *conn, topic.id, types::CLOSE, by).await?;
            delete_topic_timer(&mut *conn, topic.id, types::SILENT_CLOSE, by).await?;
        } else {
            delete_topic_timer(&mut *conn, topic.id, types::OPEN, by).await?;
            crate::topic_timer::inherit_auto_close_from_category(
                &mut *conn,
                ctx,
                topic.id,
                types::CLOSE,
            )
            .await?;
        }
    }
    // Featured topics go when a topic is closed, archived or unlisted:
    // here when nothing changed, else once the category's row is locked
    // (above for an unlisting, in the small action otherwise).
    if unfeature && !updated {
        unfeature_topic(&mut *conn, topic.id).await?;
    }
    if status == Status::Visible && !enabled {
        sqlx::query("DELETE FROM topic_hot_scores WHERE topic_id = $1")
            .bind(topic.id)
            .execute(&mut *conn)
            .await?;
    }

    // create_moderator_post_for
    if updated {
        let action_code = format!(
            "{}.{}",
            status.name(),
            if enabled { "enabled" } else { "disabled" }
        );
        // opts[:message] || message_for(status)
        let message = match (opts.message, status) {
            (Some(m), _) => Some(m.to_string()),
            (None, Status::Autoclosed) => Some(
                autoclosed_message(&mut *conn, ctx, topic.id, enabled, topic_timer.as_ref())
                    .await?,
            ),
            _ => None,
        };
        let action = SmallAction {
            user_id,
            topic_id: topic.id,
            action_code: &action_code,
            bump: matches!(status, Status::Closed | Status::Autoclosed) && !enabled,
            who: None,
        };
        if unfeature && status != Status::Visible {
            small_action::add_unfeaturing(
                &mut *conn,
                ctx,
                &action,
                message.as_deref(),
                opts.silent,
            )
            .await?;
        } else {
            small_action::add_with_message(
                &mut *conn,
                ctx,
                &action,
                message.as_deref(),
                opts.silent,
            )
            .await?;
        }
    }
    // DiscourseEvent :topic_status_updated when it changed (a pin compares
    // the pin; unpinning names the pin it was): the web hooks first.
    let changed = match &previous_pin {
        Some(previous) => {
            let now: Pin = sqlx::query_as(pin_sql)
                .bind(topic.id)
                .fetch_one(&mut *conn)
                .await?;
            *previous != now
        }
        None => updated,
    };
    if changed {
        let event_status = match &previous_pin {
            Some((_, globally, _)) if !enabled => {
                if *globally {
                    "pinned_globally"
                } else {
                    "pinned"
                }
            }
            _ => status.name(),
        };
        crate::web_hooks::enqueue_topic_hooks(
            &mut *conn,
            &crate::web_hooks::Payloads {
                settings: s,
                i18n: ctx.i18n,
                config: ctx.config,
            },
            &format!("topic_{event_status}_status_updated"),
            topic.id,
            None,
        )
        .await?;
    }
    // No plugin listens for pins.
    if updated && !is_pin {
        crate::plugins::topic_status_updated(&mut *conn, s, topic.id, status.name(), enabled)
            .await?;
        crate::plugins::assign::lifecycle::topic_status_updated(
            &mut *conn,
            ctx,
            topic.id,
            status.name(),
            enabled,
        )
        .await?;
    }

    // StaffActionLogger
    let logged = match status {
        Status::Closed => Some(if enabled { TOPIC_CLOSED } else { TOPIC_OPENED }),
        Status::Archived => Some(if enabled {
            TOPIC_ARCHIVED
        } else {
            TOPIC_UNARCHIVED
        }),
        _ => None,
    };
    if let Some(action) = logged {
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, topic_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(action)
        .bind(user_id)
        .bind(topic.id)
        .execute(&mut *conn)
        .await?;
    }
    // Topic#update_status: a message closed is archived for each of the
    // closer's groups it is to (with no acting user).
    if enabled
        && matches!(status, Status::Closed | Status::Autoclosed)
        && topic.archetype == "private_message"
    {
        let groups: Vec<i32> = sqlx::query_scalar(
            "SELECT groups.id FROM groups \
             INNER JOIN topic_allowed_groups ON groups.id = topic_allowed_groups.group_id \
             WHERE topic_allowed_groups.topic_id = $1 \
               AND topic_allowed_groups.group_id IN (SELECT group_id FROM group_users WHERE user_id = $2)",
        )
        .bind(topic.id)
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        for group_id in groups {
            if crate::archived_messages::group_archive(
                ctx.bus, &mut *conn, group_id, topic.id, None,
            )
            .await?
            {
                crate::plugins::assign::lifecycle::archive_message(&mut *conn, ctx, topic.id, true)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Discourse::InvalidParameters for a param: a 400 naming it.
#[derive(Debug)]
pub struct InvalidParameters(pub &'static str);

impl std::fmt::Display for InvalidParameters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Discourse::InvalidParameters: {}", self.0)
    }
}

impl std::error::Error for InvalidParameters {}

/// `Time.parse(s)` of a time the pin and banner forms send
/// (FutureDateInputSelector's FORMAT, "YYYY-MM-DD HH:mmZ", or ISO 8601),
/// in UTC. What Date._parse finds nothing in raises ArgumentError
/// (InvalidParameters for `param`); any other form is refused.
pub fn parse_time(s: &str, param: &'static str) -> Result<NaiveDateTime, AppError> {
    let v = s.trim();
    for format in [
        "%Y-%m-%d %H:%M%:z",
        "%Y-%m-%d %H:%M%z",
        "%Y-%m-%d %H:%M:%S%:z",
    ] {
        if let Ok(t) = chrono::DateTime::parse_from_str(v, format) {
            return Ok(t.naive_utc());
        }
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(v) {
        return Ok(t.naive_utc());
    }
    let h = crate::ruby_date::parse(v);
    let nothing = h.year.is_none()
        && h.yday.is_none()
        && h.mon.is_none()
        && h.mday.is_none()
        && h.hour.is_none()
        && h.min.is_none()
        && h.sec.is_none()
        && !h.sec_fraction;
    if v.is_ascii() && nothing {
        return Err(InvalidParameters(param).into());
    }
    Err(Unsupported("a time Time.parse reads that isn't the pin form's").into())
}

/// `CategoryFeaturedTopic.where(topic_id:).delete_all`
async fn unfeature_topic(conn: &mut PgConnection, topic_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM category_featured_topics WHERE topic_id = $1")
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `Topic#update_pinned(status, global, pinned_until)`: the columns, then
/// the scheduled unpin cancelled and, with a time, scheduled again.
pub async fn update_pinned(
    conn: &mut PgConnection,
    topic_id: i32,
    enabled: bool,
    global: bool,
    until: Option<&str>,
) -> Result<(), AppError> {
    let until = until.map(|u| parse_time(u, "pinned_until")).transpose()?;
    sqlx::query(
        "UPDATE topics SET pinned_at = CASE WHEN $2 THEN clock_timestamp() END, \
                           pinned_globally = $3, pinned_until = $4 WHERE id = $1",
    )
    .bind(topic_id)
    .bind(enabled)
    .bind(global)
    .bind(until)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM discourse_rs.jobs WHERE name = 'unpin_topic' AND args->>'topic_id' = $1::text",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    if let Some(at) = until {
        crate::jobs::enqueue_at(
            &mut *conn,
            at,
            "unpin_topic",
            json!({ "topic_id": topic_id }),
        )
        .await?;
    }
    Ok(())
}

/// `Jobs::UnpinTopic`: a topic that is still there unpinned (its columns
/// only; no small action).
pub async fn unpin_topic_job(conn: &mut PgConnection, args: &Value) -> Result<(), AppError> {
    let topic_id = match &args["topic_id"] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) if !crate::ruby::is_blank(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let Some(topic_id) = topic_id else {
        return Ok(());
    };
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
    )
    .bind(topic_id as i32)
    .fetch_one(&mut *conn)
    .await?;
    if exists {
        update_pinned(conn, topic_id as i32, false, false, None).await?;
    }
    Ok(())
}

/// TopicStatusUpdater#message_for an autoclosed change: how long the
/// topic stayed open (or closed), from the timer's duration when based
/// on the last post, else since the timer (or the topic) was created.
async fn autoclosed_message(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    enabled: bool,
    timer: Option<&crate::topic_timer::Timer>,
) -> Result<String, AppError> {
    let lastpost = timer.is_some_and(|t| t.based_on_last_post);
    let key = format!(
        "topic_statuses.autoclosed_{}{}",
        if enabled { "enabled" } else { "disabled" },
        if lastpost { "_lastpost" } else { "" }
    );
    let now = crate::clock::now_naive();
    let seconds: f64 = match timer {
        Some(t) if t.based_on_last_post => f64::from(t.duration_minutes.unwrap_or(0)) * 60.0,
        Some(t) => (now - t.created_at).num_microseconds().unwrap_or(0) as f64 / 1e6,
        None => {
            let created: chrono::NaiveDateTime =
                sqlx::query_scalar("SELECT created_at FROM topics WHERE id = $1")
                    .bind(topic_id)
                    .fetch_one(&mut *conn)
                    .await?;
            (now - created).num_microseconds().unwrap_or(0) as f64 / 1e6
        }
    };
    let minutes = (seconds / 60.0).round() as i64;
    // Duration / Duration divides integers.
    Ok(if minutes * 60 >= 2 * 86_400 {
        ctx.t_count(&format!("{key}_days"), minutes * 60 / 86_400)
    } else if minutes / 60 >= 2 {
        ctx.t_count(&format!("{key}_hours"), minutes / 60)
    } else {
        ctx.t_count(&format!("{key}_minutes"), minutes)
    })
}

/// The response's body: success, with `TopicTimer.find_by(topic:)`, any
/// live timer of the topic (private ones too).
pub async fn response(conn: &mut PgConnection, topic_id: i32) -> Result<Value, AppError> {
    let timer = crate::topic_timer::any_topic_timer(conn, topic_id).await?;
    Ok(json!({
        "success": "OK",
        "topic_status_update": timer.map(|t| t.serialize()),
    }))
}
