//! Roleable's moderation grant, as the first admin's first login makes it
//! (`DefaultCurrentUserProvider#bootstrap_first_admin`), and
//! `Group.refresh_automatic_group!` for the staff groups.
//!
//! Refused: a pending ReviewableUser for the user (auto_approve_user would
//! perform it), category or tag notification defaults on the moderators or
//! staff group, and staff groups Rails would create, rename or relevel.

use serde_json::json;
use sqlx::{Acquire, PgConnection};

use crate::i18n::I18n;
use crate::{AppError, Unsupported};

/// `UserHistory.actions[:grant_moderation]`
const GRANT_MODERATION: i32 = 34;
/// `Group::AUTO_GROUPS`: the staff groups, with who belongs in each.
const STAFF_GROUPS: [(i32, &str, &str); 3] = [
    (1, "admins", "admin"),
    (2, "moderators", "moderator"),
    (3, "staff", "(admin OR moderator)"),
];
/// `Group::AUTO_GROUP_IDS`: every automatic group.
const AUTO_GROUP_IDS: &str = "0,1,2,3,4,5,10,11,12,13,14";

/// `bootstrap_first_admin(user)`, from log_on_user: the site's only admin,
/// logging in for the first time, is made a moderator too, logged as the
/// system user's grant. True when it granted.
pub async fn bootstrap_first_admin(
    conn: &mut PgConnection,
    i18n: &I18n,
    user_id: i32,
) -> Result<bool, AppError> {
    let (admin, moderator, never_seen): (bool, bool, bool) =
        sqlx::query_as("SELECT admin, moderator, last_seen_at IS NULL FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if !admin || moderator || !never_seen {
        return Ok(false);
    }
    // is_singular_admin?: no other human admin.
    let singular: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM users WHERE admin AND id > 0 AND id <> $1)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !singular {
        return Ok(false);
    }
    grant_moderation(conn, i18n, user_id).await?;
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES ($1, -1, $2, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(GRANT_MODERATION)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(true)
}

/// `grant_moderation!` for a user who is not a moderator yet.
pub async fn grant_moderation(
    conn: &mut PgConnection,
    i18n: &I18n,
    user_id: i32,
) -> Result<(), AppError> {
    // set_permission: save_and_refresh_staff_groups!, in a transaction.
    let mut tx = conn.begin().await?;
    sqlx::query("UPDATE users SET moderator = TRUE, updated_at = clock_timestamp() WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    refresh_staff_groups(&mut tx, i18n).await?;
    tx.commit().await?;

    // auto_approve_user
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM reviewables WHERE type = 'ReviewableUser' \
         AND target_type = 'User' AND target_id = $1 AND status = 0)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if pending {
        return Err(Unsupported("approving a pending user on granting moderation").into());
    }
    // ReviewableUser.set_approved_fields!, by the system user, then save!.
    sqlx::query(
        "UPDATE users SET approved = TRUE, approved_by_id = COALESCE(approved_by_id, -1), \
                          approved_at = COALESCE(approved_at, clock_timestamp()), \
                          updated_at = clock_timestamp() \
         WHERE id = $1 AND (NOT approved OR approved_by_id IS NULL OR approved_at IS NULL)",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    // enqueue_staff_welcome_message(:moderator): none for the singular
    // admin (no other human admin).
    let singular: bool = sqlx::query_scalar(
        "SELECT NOT EXISTS (SELECT 1 FROM users WHERE admin AND id > 0 AND id <> $1)",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    if !singular {
        crate::jobs::enqueue(
            &mut *conn,
            "send_system_message",
            json!({
                "user_id": user_id,
                "message_type": "welcome_staff",
                "message_options": { "role": "moderator" },
            }),
        )
        .await?;
    }

    // set_default_notification_levels(:moderators), and :staff.
    let defaults: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_category_notification_defaults WHERE group_id IN (2, 3)) \
             OR EXISTS (SELECT 1 FROM group_tag_notification_defaults WHERE group_id IN (2, 3))",
    )
    .fetch_one(&mut *conn)
    .await?;
    if defaults {
        return Err(Unsupported("staff group category or tag notification defaults").into());
    }
    Ok(())
}

/// `Group.refresh_automatic_groups!(:admins, :moderators, :staff)`: each
/// group loses the users who no longer belong and gains those who do, with
/// publish_group_membership_updates for each change, and its user count
/// reset.
pub async fn refresh_staff_groups(conn: &mut PgConnection, i18n: &I18n) -> Result<(), AppError> {
    #[derive(sqlx::FromRow)]
    struct GroupRow {
        name: String,
        full_name: Option<String>,
        visibility_level: i32,
        messageable_level: i32,
        title: Option<String>,
    }
    for (group_id, name, member) in STAFF_GROUPS {
        let group: Option<GroupRow> = sqlx::query_as(
            "SELECT name, full_name, visibility_level, messageable_level, title FROM groups WHERE id = $1",
        )
        .bind(group_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(group) = group else {
            return Err(Unsupported("creating a missing staff group").into());
        };
        let title = group.title;
        // The name and full name from the locale, a public group made
        // logged-on only, and moderators messageable by everyone.
        let default_name = i18n.t(&format!("groups.default_names.{name}"));
        let default_full_name = i18n.t(&format!("groups.default_full_names.{name}"));
        if default_name != Some(group.name.as_str())
            || default_full_name != group.full_name.as_deref()
            || group.visibility_level == 0
            || (group_id == 2 && group.messageable_level != 99)
        {
            return Err(Unsupported("renaming or releveling a staff group").into());
        }

        // remove_users_from_automatic_group
        let removed: Vec<i32> = sqlx::query_scalar(&format!(
            "DELETE FROM group_users USING (SELECT id FROM users WHERE NOT {member} OR staged) x \
             WHERE group_id = $1 AND user_id = x.id RETURNING group_users.user_id"
        ))
        .bind(group_id)
        .fetch_all(&mut *conn)
        .await?;
        if !removed.is_empty() {
            sqlx::query(
                "UPDATE users SET flair_group_id = NULL WHERE id = ANY($1) AND flair_group_id = $2",
            )
            .bind(&removed)
            .bind(group_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "UPDATE users SET primary_group_id = NULL WHERE id = ANY($1) AND primary_group_id = $2",
            )
            .bind(&removed)
            .bind(group_id)
            .execute(&mut *conn)
            .await?;
            if let Some(title) = title.filter(|t| !t.is_empty()) {
                sqlx::query("UPDATE users SET title = NULL WHERE id = ANY($1) AND title = $2")
                    .bind(&removed)
                    .bind(title)
                    .execute(&mut *conn)
                    .await?;
            }
            crate::jobs::enqueue(
                &mut *conn,
                "publish_group_membership_updates",
                json!({ "user_ids": removed, "group_id": group_id, "type": "remove" }),
            )
            .await?;
        }

        let added: Vec<i32> = sqlx::query_scalar(&format!(
            "INSERT INTO group_users (group_id, user_id, created_at, updated_at) \
             SELECT $1, x.id, clock_timestamp(), clock_timestamp() FROM group_users \
             RIGHT JOIN (SELECT id FROM users WHERE {member} AND NOT staged) x \
               ON x.id = user_id AND group_id = $1 \
             WHERE user_id IS NULL RETURNING group_users.user_id"
        ))
        .bind(group_id)
        .fetch_all(&mut *conn)
        .await?;
        if !added.is_empty() {
            crate::jobs::enqueue(
                &mut *conn,
                "publish_group_membership_updates",
                json!({ "user_ids": added, "group_id": group_id, "type": "add" }),
            )
            .await?;
        }

        // reset_user_count
        sqlx::query(&format!(
            "WITH tally AS ( \
               SELECT g.id AS group_id, COUNT(gu.user_id) AS users FROM groups g \
               LEFT JOIN group_users gu ON gu.group_id = g.id \
                 AND (gu.user_id > 0 OR (g.automatic AND g.id NOT IN ({AUTO_GROUP_IDS}))) \
               WHERE g.id = $1 GROUP BY g.id) \
             UPDATE groups SET user_count = tally.users FROM tally \
             WHERE id = tally.group_id AND user_count <> tally.users"
        ))
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `revoke_moderation!` / `revoke_admin!`: set_permission to false, the
/// staff groups refreshed in the same transaction.
pub async fn revoke(
    conn: &mut PgConnection,
    i18n: &I18n,
    user_id: i32,
    permission: Permission,
) -> Result<(), AppError> {
    let column = match permission {
        Permission::Admin => "admin",
        Permission::Moderator => "moderator",
    };
    let mut tx = conn.begin().await?;
    sqlx::query(&format!(
        "UPDATE users SET {column} = FALSE, updated_at = clock_timestamp() WHERE id = $1 AND {column}"
    ))
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    refresh_staff_groups(&mut tx, i18n).await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Clone, Copy)]
pub enum Permission {
    Admin,
    Moderator,
}

/// StaffActionLogger's grant and revoke logs (`log_grant_moderation`,
/// `log_revoke_moderation`, `log_revoke_admin`).
pub async fn log(
    conn: &mut PgConnection,
    acting_user_id: i32,
    target_user_id: i32,
    action: i32,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, target_user_id, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(acting_user_id)
    .bind(target_user_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// UserHistory.actions
pub const REVOKE_ADMIN: i32 = 33;
pub const REVOKE_MODERATION: i32 = 35;
pub const LOG_GRANT_MODERATION: i32 = GRANT_MODERATION;
