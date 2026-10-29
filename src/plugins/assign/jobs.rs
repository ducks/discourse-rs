//! discourse-assign's jobs: Jobs::AssignNotification (each assigned user
//! hears of the assignment, DiscourseAssign::CreateNotification),
//! Jobs::UnassignNotification (their assigned notifications go),
//! Jobs::RemindUser (PendingAssignsReminder) and the daily
//! Jobs::EnqueueReminders.

use serde_json::{Value, json};
use sqlx::PgConnection;

use super::{ASSIGNED, Assignment};
use crate::{AppError, AppState, Unsupported};

pub const ASSIGN_NOTIFICATION: &str = "assign_notification";
pub const UNASSIGN_NOTIFICATION: &str = "unassign_notification";
pub const REMIND_USER: &str = "remind_user";
/// Jobs::EnqueueReminders, `every 1.day`.
pub const ENQUEUE_REMINDERS: &str = "enqueue_reminders";

/// `Jobs.enqueue(:assign_notification, assignment_id:)`
pub async fn enqueue_assign_notification(
    conn: &mut PgConnection,
    assignment_id: i64,
) -> Result<(), sqlx::Error> {
    crate::jobs::enqueue(
        conn,
        ASSIGN_NOTIFICATION,
        json!({ "assignment_id": assignment_id }),
    )
    .await
}

/// `Jobs.enqueue(:unassign_notification, topic_id:, assigned_to_id:,
/// assigned_to_type:, assignment_id:)`
pub async fn enqueue_unassign_notification(
    conn: &mut PgConnection,
    assignment: &Assignment,
) -> Result<(), sqlx::Error> {
    crate::jobs::enqueue(
        conn,
        UNASSIGN_NOTIFICATION,
        json!({
            "topic_id": assignment.topic_id,
            "assigned_to_id": assignment.assigned_to_id,
            "assigned_to_type": assignment.assigned_to_type,
            "assignment_id": assignment.id,
        }),
    )
    .await
}

fn arg_i64(args: &Value, key: &str) -> Option<i64> {
    match args.get(key)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    }
}

/// `Notification.for_assignment(ids)`: assigned notifications naming one
/// of the assignments in their data, as a condition over `notifications`.
pub fn for_assignment_where(assignment_ids_sql: &str) -> String {
    format!(
        "notifications.notification_type = {ASSIGNED} AND \
         ((notifications.data::jsonb)->'assignment_id')::bigint IN ({assignment_ids_sql})"
    )
}

/// Jobs::AssignNotification
pub async fn assign_notification(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(assignment_id) = arg_i64(args, "assignment_id") else {
        return Err(Unsupported(
            "assign_notification without an assignment_id (InvalidParameters)",
        )
        .into());
    };
    let mut tx = state.pool.begin().await?;
    let silenced: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM silenced_assignments WHERE assignment_id = $1)",
    )
    .bind(assignment_id)
    .fetch_one(&mut *tx)
    .await?;
    if silenced {
        return Ok(());
    }
    // Assignment.find: RecordNotFound fails the job.
    let Some(assignment) = Assignment::find(&mut tx, assignment_id).await? else {
        return Err(Unsupported(
            "assign_notification for an assignment that is gone (RecordNotFound)",
        )
        .into());
    };
    let settings = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    create_missing_notifications(&mut tx, state, &settings, &assignment).await?;
    tx.commit().await?;
    Ok(())
}

/// `Assignment#create_missing_notifications!`
pub async fn create_missing_notifications(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &crate::site_settings::SiteSettings,
    assignment: &Assignment,
) -> Result<(), AppError> {
    let users = Assignment::assigned_users(
        &mut *conn,
        &assignment.assigned_to_type,
        assignment.assigned_to_id,
    )
    .await?;
    for user in users {
        let exists: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM notifications WHERE user_id = $1 AND {})",
            for_assignment_where("$2")
        ))
        .bind(user)
        .bind(assignment.id)
        .fetch_one(&mut *conn)
        .await?;
        if exists {
            continue;
        }
        create_notification(
            conn,
            state,
            settings,
            assignment,
            user,
            assignment.assigned_by_user_id == user,
        )
        .await?;
    }
    Ok(())
}

/// DiscourseAssign::CreateNotification: the live alert (unless the user
/// assigned themselves) and the assigned notification, high priority,
/// read when they did.
async fn create_notification(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &crate::site_settings::SiteSettings,
    assignment: &Assignment,
    user_id: i32,
    mark_as_read: bool,
) -> Result<(), AppError> {
    // `topic`: the assignment's topic through the default scope.
    let topic: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT title, slug FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(assignment.topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((title, slug)) = topic else {
        return Ok(());
    };
    let Some((post_id, post_number)) = assignment.post(&mut *conn).await? else {
        return Err(Unsupported("an assigned notification for a post that is gone").into());
    };
    super::assigner::publish_topic_tracking_state(&state.bus, conn, assignment.topic_id, user_id)
        .await?;
    let assigned_by: String = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(assignment.assigned_by_user_id)
        .fetch_one(&mut *conn)
        .await?;
    let group_name: Option<String> = if assignment.assigned_to_group() {
        sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
            .bind(assignment.assigned_to_id)
            .fetch_optional(&mut *conn)
            .await?
    } else {
        None
    };
    let assigned_to_name = match &group_name {
        Some(name) => name.clone(),
        None => sqlx::query_scalar::<_, Option<String>>("SELECT name FROM users WHERE id = $1")
            .bind(assignment.assigned_to_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten()
            .unwrap_or_default(),
    };
    let (excerpt_key, message, display_username) = if assignment.assigned_to_group() {
        (
            "discourse_assign.topic_group_assigned_excerpt",
            "discourse_assign.assign_group_notification",
            assigned_to_name.clone(),
        )
    } else {
        (
            "discourse_assign.topic_assigned_excerpt",
            "discourse_assign.assign_notification",
            assigned_by.clone(),
        )
    };
    if !mark_as_read {
        let (suspended, locale): (bool, Option<String>) = sqlx::query_as(
            "SELECT COALESCE(suspended_till > now(), FALSE), locale FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        // PostAlerter.create_notification_alert: nothing for a suspended user.
        if !suspended {
            let locale = match locale.filter(|l| !l.is_empty()) {
                Some(l) if settings.get("allow_user_locale")?.truthy() => l,
                _ => settings.get("default_locale")?.to_s(),
            };
            let excerpt = state
                .i18n
                .locale(&locale)
                .t_with(
                    excerpt_key,
                    &[("title", &title), ("group", &assigned_to_name)],
                )
                .unwrap_or_default();
            let payload = json!({
                "notification_type": ASSIGNED,
                "post_number": post_number,
                "topic_title": title,
                "topic_id": assignment.topic_id,
                "post_id": post_id,
                "excerpt": excerpt,
                "username": assigned_by,
                "post_url": format!(
                    "/t/{}/{}/{}",
                    slug.clone().unwrap_or_default(),
                    assignment.topic_id,
                    post_number
                ),
            });
            crate::bus::publish_notification_alert(&state.bus, &mut *conn, user_id, &payload)
                .await?;
            crate::jobs::post_alert::push_notification(conn, settings, user_id).await?;
        }
    }
    let mut data = serde_json::Map::new();
    data.insert("message".into(), json!(message));
    data.insert("display_username".into(), json!(display_username));
    data.insert("topic_title".into(), json!(title));
    data.insert("assignment_id".into(), json!(assignment.id));
    let mut notification =
        crate::notification_create::NewNotification::new(ASSIGNED, user_id, Value::Object(data))
            .topic(assignment.topic_id, Some(post_number))
            .read(mark_as_read)
            .high_priority(true);
    notification.created_at = Some(assignment.created_at);
    crate::notification_create::create(&mut *conn, &state.bus, settings, &notification).await?;
    Ok(())
}

/// Jobs::UnassignNotification: the assigned users' notifications of the
/// assignment on the topic go, each told on /private-messages/assigned
/// first for a message.
pub async fn unassign_notification(state: &AppState, args: &Value) -> Result<(), AppError> {
    let (Some(topic_id), Some(assigned_to_id), Some(assignment_id)) = (
        arg_i64(args, "topic_id"),
        arg_i64(args, "assigned_to_id"),
        arg_i64(args, "assignment_id"),
    ) else {
        return Err(
            Unsupported("unassign_notification without its arguments (InvalidParameters)").into(),
        );
    };
    let Some(assigned_to_type) = args.get("assigned_to_type").and_then(Value::as_str) else {
        return Err(Unsupported(
            "unassign_notification without assigned_to_type (InvalidParameters)",
        )
        .into());
    };
    let mut tx = state.pool.begin().await?;
    let settings = crate::site_settings::SiteSettings::load(
        &mut tx,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let users =
        Assignment::assigned_users(&mut tx, assigned_to_type, assigned_to_id as i32).await?;
    for user in &users {
        super::assigner::publish_topic_tracking_state(&state.bus, &mut tx, topic_id as i32, *user)
            .await?;
    }
    destroy_notifications(
        &mut tx,
        &state.bus,
        &settings,
        &format!(
            "notifications.user_id = ANY($1) AND notifications.topic_id = $2 AND {}",
            for_assignment_where("$3")
        ),
        &users,
        Some(topic_id as i32),
        assignment_id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `.destroy_all` over notifications: each row goes and its user's counts
/// are republished (Notification's after_commit `refresh_notification_count`).
pub async fn destroy_notifications(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &crate::site_settings::SiteSettings,
    condition: &str,
    user_ids: &[i32],
    topic_id: Option<i32>,
    assignment_id: i64,
) -> Result<(), AppError> {
    let users: Vec<i32> = sqlx::query_scalar(&format!(
        "DELETE FROM notifications WHERE {condition} RETURNING user_id"
    ))
    .bind(user_ids)
    .bind(topic_id)
    .bind(assignment_id)
    .fetch_all(&mut *conn)
    .await?;
    for user in users {
        crate::bus::publish_notifications_state(bus, &mut *conn, settings, user).await?;
    }
    Ok(())
}

/// Jobs::EnqueueReminders: a remind_user job for each user in a group
/// that may assign, with enough assignments of live topics, untouched for
/// their frequency (the user's own, else remind_assigns_frequency), whose
/// last reminder is older than that less two hours.
pub async fn enqueue_reminders(state: &AppState) -> Result<(), AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    let permissions = super::Permissions::load(&mut conn, &settings).await?;
    let groups = permissions.all_assign_allowed_group_ids();
    if !super::enabled(&settings)? || groups.is_empty() {
        return Ok(());
    }
    // REMINDER_BUFFER_MINUTES
    const BUFFER: i64 = 120;
    let global = settings.get("remind_assigns_frequency")?.to_i();
    let threshold = settings.get("pending_assign_reminder_threshold")?.to_i();
    let frequency = format!("COALESCE(user_frequency.value, '{global}')::INT");
    let user_ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT assignments.assigned_to_id FROM assignments \
         LEFT OUTER JOIN user_custom_fields AS last_reminder \
           ON assignments.assigned_to_id = last_reminder.user_id AND last_reminder.name = 'last_reminded_at' \
         LEFT OUTER JOIN user_custom_fields AS user_frequency \
           ON assignments.assigned_to_id = user_frequency.user_id \
           AND user_frequency.name = '{freq_field}' \
         INNER JOIN group_users ON assignments.assigned_to_id = group_users.user_id \
         INNER JOIN topics ON topics.id = assignments.target_id AND assignments.target_type = 'Topic' \
           AND topics.deleted_at IS NULL \
         WHERE group_users.group_id IN ({groups}) \
           AND {frequency} > 0 \
           AND (last_reminder.value IS NULL OR \
                last_reminder.value::TIMESTAMP <= CURRENT_TIMESTAMP - ('1 MINUTE'::INTERVAL * {frequency}) \
                  + ('1 MINUTE'::INTERVAL * {BUFFER})) \
           AND assignments.updated_at::TIMESTAMP <= CURRENT_TIMESTAMP - ('1 MINUTE'::INTERVAL * {frequency}) \
           AND assignments.assigned_to_type = 'User' \
         GROUP BY assignments.assigned_to_id \
         HAVING COUNT(assignments.assigned_to_id) >= {threshold}",
        freq_field = super::REMINDERS_FREQUENCY,
        groups = super::id_list(&groups),
    ))
    .fetch_all(&mut *conn)
    .await?;
    for id in user_ids {
        crate::jobs::enqueue(&mut conn, REMIND_USER, json!({ "user_id": id })).await?;
    }
    Ok(())
}

/// Jobs::RemindUser
pub async fn remind_user(state: &AppState, args: &Value) -> Result<(), AppError> {
    let Some(user_id) = arg_i64(args, "user_id") else {
        return Err(Unsupported("remind_user without a user_id (InvalidParameters)").into());
    };
    // Not one transaction: the reminder is created and the old ones are
    // destroyed by PostCreator and PostDestroyer, each in its own.
    let mut conn = state.pool.acquire().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(user_id as i32)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Err(Unsupported("remind_user for a user that is gone (InvalidParameters)").into());
    }
    super::reminder::remind(&mut conn, state, user_id as i32).await
}
