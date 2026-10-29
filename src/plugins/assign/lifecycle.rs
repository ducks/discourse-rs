//! The events discourse-assign listens to (`on(...)` and
//! `add_model_callback` in plugin.rb), each a no-op while the plugin is
//! off, as Rails' handlers check `enabled?`.

use serde_json::json;
use sqlx::PgConnection;

use super::assigner::{self, Assigner, Target};
use super::{Assignment, Permissions, enabled};
use crate::AppError;
use crate::posting::Ctx;
use crate::site_settings::SiteSettings;

/// `MessageBus.publish("/topic/:id", { reload_topic: true,
/// refresh_stream: true }, topic.secure_audience_publish_messages)`
async fn reload_topic(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    topic_id: i32,
) -> Result<(), AppError> {
    crate::bus::publish_to_topic(
        bus,
        conn,
        topic_id,
        &json!({ "reload_topic": true, "refresh_stream": true }),
    )
    .await
}

/// `assignment.reactivate!`: active again (when its target still
/// exists), notified and published.
pub async fn reactivate(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    permissions: &Permissions,
    assignment: &Assignment,
) -> Result<(), AppError> {
    let target_type = assignment.target_type.as_str();
    if Target::load(&mut *conn, target_type, assignment.target_id)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let assignment: Assignment = sqlx::query_as(&format!(
        "UPDATE assignments AS a SET active = TRUE, updated_at = clock_timestamp() WHERE a.id = $1 \
         RETURNING {}",
        super::COLUMNS
    ))
    .bind(assignment.id)
    .fetch_one(&mut *conn)
    .await?;
    super::jobs::enqueue_assign_notification(conn, assignment.id).await?;
    assigner::publish_topic_assignment(conn, ctx, permissions, &assignment).await
}

/// `assignment.deactivate!`: inactive, its notifications taken back.
pub async fn deactivate(conn: &mut PgConnection, assignment: &Assignment) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE assignments SET active = FALSE, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(assignment.id)
    .execute(&mut *conn)
    .await?;
    super::jobs::enqueue_unassign_notification(conn, assignment).await?;
    Ok(())
}

async fn topic_assignments(
    conn: &mut PgConnection,
    topic_id: i32,
    active: bool,
) -> Result<Vec<Assignment>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {} FROM assignments a WHERE a.topic_id = $1 AND a.active IS {} ORDER BY a.id",
        super::COLUMNS,
        if active { "TRUE" } else { "FALSE" }
    ))
    .bind(topic_id)
    .fetch_all(conn)
    .await
}

/// `:post_created` and `:post_edited`: `Assigner.auto_assign(post, force:
/// true)`.
pub async fn post_saved(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
) -> Result<(), AppError> {
    if !enabled(ctx.settings)? {
        return Ok(());
    }
    assigner::auto_assign(conn, ctx, post_id, true).await?;
    Ok(())
}

/// `:topic_status_updated`: closing unassigns the topic and its posts
/// silently, keeping the rows inactive (unassign_on_close); opening
/// reactivates them (reassign_on_open).
pub async fn topic_status_updated(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    status: &str,
    status_enabled: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if !enabled(s)? || !(status == "closed" || status == "autoclosed") {
        return Ok(());
    }
    if s.get("unassign_on_close")?.truthy()
        && status_enabled
        && !topic_assignments(conn, topic_id, true).await?.is_empty()
    {
        let permissions = Permissions::load(&mut *conn, s).await?;
        let (system, guardian) = assigner::system_user(&mut *conn).await?;
        if let Some(target) = Target::topic(&mut *conn, topic_id).await? {
            Assigner {
                ctx,
                target: &target,
                by: &system,
                guardian: &guardian,
                permissions: &permissions,
            }
            .unassign(conn, true, true)
            .await?;
        }
        let posts: Vec<i32> = sqlx::query_scalar(
            "SELECT p.id FROM posts p JOIN assignments a ON a.target_type = 'Post' AND a.target_id = p.id \
             WHERE p.topic_id = $1 AND p.deleted_at IS NULL ORDER BY p.id",
        )
        .bind(topic_id)
        .fetch_all(&mut *conn)
        .await?;
        for post_id in posts {
            if let Some(target) = Target::load(&mut *conn, "Post", post_id).await? {
                Assigner {
                    ctx,
                    target: &target,
                    by: &system,
                    guardian: &guardian,
                    permissions: &permissions,
                }
                .unassign(conn, true, true)
                .await?;
            }
        }
        reload_topic(conn, ctx.bus, topic_id).await?;
    }
    if s.get("reassign_on_open")?.truthy() && !status_enabled {
        let inactive = topic_assignments(conn, topic_id, false).await?;
        if !inactive.is_empty() {
            let permissions = Permissions::load(&mut *conn, s).await?;
            for assignment in &inactive {
                reactivate(conn, ctx, &permissions, assignment).await?;
            }
            reload_topic(conn, ctx.bus, topic_id).await?;
        }
    }
    Ok(())
}

/// `:post_destroyed`: the post's active assignment deactivated, and the
/// small actions and whispers linking to the post destroyed.
pub async fn post_destroyed(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
) -> Result<(), AppError> {
    if !enabled(ctx.settings)? {
        return Ok(());
    }
    if let Some(assignment) = Assignment::for_target(&mut *conn, "Post", post_id)
        .await?
        .filter(|a| a.active)
    {
        deactivate(conn, &assignment).await?;
        reload_topic(conn, ctx.bus, assignment.topic_id).await?;
    }
    let linked: Vec<i32> = sqlx::query_scalar(
        "SELECT p.id FROM post_custom_fields f JOIN posts p ON p.id = f.post_id AND p.deleted_at IS NULL \
         WHERE f.name = 'action_code_post_id' AND f.value = $1 AND p.post_type IN (3, 4) ORDER BY f.id",
    )
    .bind(post_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    for id in linked {
        destroy_small_action(conn, id).await?;
    }
    Ok(())
}

/// `Post#destroy` (not PostDestroyer) for a small action or whisper: the
/// row and its `dependent: :destroy` associations. One with actions or
/// reviewables on it, whose destroy callbacks reach further, is refused.
async fn destroy_small_action(conn: &mut PgConnection, post_id: i32) -> Result<(), AppError> {
    let entangled: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_actions WHERE post_id = $1) \
         OR EXISTS (SELECT 1 FROM reviewables WHERE target_type = 'Post' AND target_id = $1)",
    )
    .bind(post_id)
    .fetch_one(&mut *conn)
    .await?;
    if entangled {
        return Err(crate::Unsupported(
            "destroying a small action with actions or reviewables on it (Post#destroy)",
        )
        .into());
    }
    for sql in [
        "DELETE FROM group_mentions WHERE post_id = $1",
        "DELETE FROM upload_references WHERE target_type = 'Post' AND target_id = $1",
        "DELETE FROM moved_posts WHERE old_post_id = $1 OR new_post_id = $1",
        "DELETE FROM post_hotlinked_media WHERE post_id = $1",
        "DELETE FROM post_custom_fields WHERE post_id = $1",
        "DELETE FROM post_search_data WHERE post_id = $1",
        "DELETE FROM assignments WHERE target_type = 'Post' AND target_id = $1",
        "DELETE FROM posts WHERE id = $1",
    ] {
        sqlx::query(sql).bind(post_id).execute(&mut *conn).await?;
    }
    Ok(())
}

/// `:post_recovered`: reassign_on_open reactivates the post's assignment.
pub async fn post_recovered(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    post_id: i32,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if !enabled(s)? || !s.get("reassign_on_open")?.truthy() {
        return Ok(());
    }
    if let Some(assignment) = Assignment::for_target(&mut *conn, "Post", post_id)
        .await?
        .filter(|a| !a.active)
    {
        let permissions = Permissions::load(&mut *conn, s).await?;
        reactivate(conn, ctx, &permissions, &assignment).await?;
        reload_topic(conn, ctx.bus, assignment.topic_id).await?;
    }
    Ok(())
}

/// `:move_to_inbox` (`group`: a group inbox's): the assignee hears of the
/// message again; unassign_on_group_archive reactivates its assignments.
pub async fn move_to_inbox(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    group: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if !enabled(s)? {
        return Ok(());
    }
    if let Some(a) = Assignment::for_target(&mut *conn, "Topic", topic_id).await? {
        assigner::publish_assigned_private_message(
            ctx.bus,
            conn,
            topic_id,
            &a.assigned_to_type,
            a.assigned_to_id,
        )
        .await?;
    }
    if !s.get("unassign_on_group_archive")?.truthy() || !group {
        return Ok(());
    }
    let permissions = Permissions::load(&mut *conn, s).await?;
    for assignment in topic_assignments(conn, topic_id, false).await? {
        reactivate(conn, ctx, &permissions, &assignment).await?;
    }
    Ok(())
}

/// `:archive_message`: the assignee hears of it; unassign_on_group_archive
/// deactivates a group-archived message's assignments.
pub async fn archive_message(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    group: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if !enabled(s)? {
        return Ok(());
    }
    let Some(a) = Assignment::for_target(&mut *conn, "Topic", topic_id).await? else {
        return Ok(());
    };
    assigner::publish_assigned_private_message(
        ctx.bus,
        conn,
        topic_id,
        &a.assigned_to_type,
        a.assigned_to_id,
    )
    .await?;
    if !s.get("unassign_on_group_archive")?.truthy() || !group {
        return Ok(());
    }
    for assignment in topic_assignments(conn, topic_id, true).await? {
        deactivate(conn, &assignment).await?;
    }
    Ok(())
}

/// `:user_added_to_group`: the group's active assignments notified again,
/// which reaches the new member (the others have theirs already).
pub async fn user_added_to_group(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    _user_id: i32,
    group_id: i32,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM assignments WHERE assigned_to_type = 'Group' AND assigned_to_id = $1 \
         AND active ORDER BY id",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await?;
    for id in ids {
        super::jobs::enqueue_assign_notification(conn, id).await?;
    }
    Ok(())
}

/// The group's assignments, as a subquery of ids (`group.assignments`).
fn group_assignment_ids(group_id: i32) -> String {
    format!(
        "SELECT id FROM assignments WHERE assigned_to_type = 'Group' AND assigned_to_id = {group_id}"
    )
}

/// `:user_removed_from_group`: the user's notifications of the group's
/// assignments go.
pub async fn user_removed_from_group(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    user_id: i32,
    group_id: i32,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    destroy_group_notifications(conn, bus, settings, &[user_id], group_id).await
}

async fn destroy_group_notifications(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    user_ids: &[i32],
    group_id: i32,
) -> Result<(), AppError> {
    let users: Vec<i32> = sqlx::query_scalar(&format!(
        "DELETE FROM notifications WHERE notifications.user_id = ANY($1) AND {} RETURNING user_id",
        super::jobs::for_assignment_where(&group_assignment_ids(group_id))
    ))
    .bind(user_ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut seen = Vec::new();
    for user in users {
        if !seen.contains(&user) {
            seen.push(user);
            crate::bus::publish_notifications_state(bus, &mut *conn, settings, user).await?;
        }
    }
    Ok(())
}

/// `:group_destroyed`: the former members' notifications of the group's
/// assignments go, then its active assignments.
pub async fn group_destroyed(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    group_id: i32,
    user_ids: &[i32],
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    destroy_group_notifications(conn, bus, settings, user_ids, group_id).await?;
    sqlx::query(
        "DELETE FROM assignments WHERE assigned_to_type = 'Group' AND assigned_to_id = $1 AND active",
    )
    .bind(group_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `:post_moved`: a post's assignment follows it to its new topic, as
/// the topic's when it became the first post.
pub async fn post_moved(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    post_id: i32,
    original_topic_id: i32,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let assignment: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM assignments WHERE topic_id = $1 AND target_type = 'Post' AND target_id = $2 \
         ORDER BY id LIMIT 1",
    )
    .bind(original_topic_id)
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(id) = assignment else {
        return Ok(());
    };
    let (topic_id, post_number): (i32, i32) =
        sqlx::query_as("SELECT topic_id, post_number FROM posts WHERE id = $1")
            .bind(post_id)
            .fetch_one(&mut *conn)
            .await?;
    if post_number == 1 {
        sqlx::query(
            "UPDATE assignments SET topic_id = $2, target_type = 'Topic', target_id = $2, \
             updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    } else {
        sqlx::query(
            "UPDATE assignments SET topic_id = $2, updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// discourse-solved's `:accepted_solution` and `:unaccepted_solution`:
/// each of the topic's assignments takes assignment_status_on_solve (or
/// _on_unsolve once the topic has no answer left), reassigned by the
/// assigned user to themselves.
pub async fn solution_changed(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    topic_id: i32,
    accepted: bool,
) -> Result<(), AppError> {
    let s = ctx.settings;
    if !enabled(s)? {
        return Ok(());
    }
    let status = s
        .get(if accepted {
            "assignment_status_on_solve"
        } else {
            "assignment_status_on_unsolve"
        })?
        .to_s();
    if status.is_empty() {
        return Ok(());
    }
    if !accepted {
        let solved: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM discourse_solved_solved_topics WHERE topic_id = $1)",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if solved {
            return Ok(());
        }
    }
    let permissions = Permissions::load(&mut *conn, s).await?;
    let all: Vec<Assignment> = sqlx::query_as(&format!(
        "SELECT {} FROM assignments a WHERE a.topic_id = $1 ORDER BY a.id",
        super::COLUMNS
    ))
    .bind(topic_id)
    .fetch_all(&mut *conn)
    .await?;
    for assignment in all {
        // User.find_by(id: assigned_to_id), whatever the assignee's type.
        let Some(user) =
            crate::session::current::SessionUser::load(&mut *conn, assignment.assigned_to_id)
                .await?
        else {
            // Assigner.new(target, nil): a nil guardian may not assign.
            continue;
        };
        let guardian = crate::guardian::Guardian::for_user(&mut *conn, &user).await?;
        let Some(target) =
            Target::load(&mut *conn, &assignment.target_type, assignment.target_id).await?
        else {
            continue;
        };
        let Some(assign_to) = assigner::Assignee::user(&mut *conn, user.id).await? else {
            continue;
        };
        let _ = Assigner {
            ctx,
            target: &target,
            by: &user,
            guardian: &guardian,
            permissions: &permissions,
        }
        .assign(
            conn,
            &assign_to,
            assigner::AssignOpts {
                status: Some(status.clone()),
                ..Default::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// Group's `before_update` (a renamed group) and `before_destroy`: the
/// assign_allowed_on_groups value rewritten, as plugin.rb does it (a
/// string replace of the old name; the id and a following pipe removed).
pub async fn group_saving(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    defs: &crate::site_settings::Definitions,
    settings: &SiteSettings,
    change: GroupChange<'_>,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let current = settings.get("assign_allowed_on_groups")?.to_s();
    let new = match change {
        GroupChange::Renamed { from, to } => current.replace(from, to),
        GroupChange::Destroyed { id } => {
            let pattern = regex::Regex::new(&format!(r"{id}[|]?")).expect("an id pattern");
            let mut value = pattern.replace_all(&current, "").to_string();
            if value.ends_with('|') {
                value.pop();
            }
            value
        }
    };
    if new == current {
        return Ok(());
    }
    let def = defs
        .get("assign_allowed_on_groups")
        .ok_or(crate::Unsupported(
            "assign_allowed_on_groups without its definition",
        ))?;
    crate::site_setting_update::save_override(&mut *conn, def, &new).await?;
    bus.publish(
        &mut *conn,
        "/client_settings",
        &json!({ "name": "assign_allowed_on_groups", "value": new, "scoped_to": null }),
        None,
    )
    .await?;
    Ok(())
}

/// What happened to a group, for [`group_saving`].
pub enum GroupChange<'a> {
    Renamed { from: &'a str, to: &'a str },
    Destroyed { id: i32 },
}
