//! Port of GroupManager (lib/group_manager.rb) for one user at a time, as
//! `Group#add` and `Group#remove` use it, with GroupActionLogger's
//! membership logs.
//!
//! Refused: groups with categories (the /categories update published to
//! the member), groups with category or tag notification defaults
//! (CategoryUser/TagUser auto watch and track), active web hooks, and a
//! removed group title another group or badge would replace
//! (`next_best_title`).

use serde_json::json;
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// `GroupHistory.actions`
const ADD_USER_TO_GROUP: i32 = 2;
const REMOVE_USER_FROM_GROUP: i32 = 3;
/// Group::AUTO_GROUPS ids: automatic groups hide bots from user_count.
const MAX_AUTO_GROUP_ID: i32 = 14;

#[derive(sqlx::FromRow)]
pub struct Group {
    pub id: i32,
    pub automatic: bool,
    title: Option<String>,
    primary_group: bool,
    grant_trust_level: Option<i32>,
    default_notification_level: i32,
}

pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, automatic, title, primary_group, grant_trust_level, default_notification_level \
         FROM groups WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await
}

impl Group {
    fn title(&self) -> Option<&str> {
        self.title.as_deref().filter(|t| !t.is_empty())
    }

    /// `hides_bot_members?`
    fn counted(&self, user_id: i32) -> i32 {
        let hides = !self.automatic || self.id <= MAX_AUTO_GROUP_ID;
        i32::from(!hides || user_id > 0)
    }
}

async fn refusals(conn: &mut PgConnection, group: &Group) -> Result<(), AppError> {
    let (categories, defaults, hooks): (bool, bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM category_groups WHERE group_id = $1), \
                EXISTS (SELECT 1 FROM group_category_notification_defaults WHERE group_id = $1) \
                  OR EXISTS (SELECT 1 FROM group_tag_notification_defaults WHERE group_id = $1), \
                EXISTS (SELECT 1 FROM web_hooks WHERE active)",
    )
    .bind(group.id)
    .fetch_one(&mut *conn)
    .await?;
    if categories {
        return Err(Unsupported("membership changes in groups with categories").into());
    }
    if defaults {
        return Err(Unsupported("groups with notification defaults").into());
    }
    if hooks {
        return Err(Unsupported("web hooks for group membership").into());
    }
    Ok(())
}

async fn log(
    conn: &mut PgConnection,
    group_id: i32,
    acting_user_id: i32,
    user_id: i32,
    action: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO group_histories (group_id, acting_user_id, target_user_id, action, subject, \
                                      created_at, updated_at) \
         VALUES ($1, $2, $3, $4, NULL, clock_timestamp(), clock_timestamp())",
    )
    .bind(group_id)
    .bind(acting_user_id)
    .bind(user_id)
    .bind(action)
    .execute(conn)
    .await?;
    Ok(())
}

/// `group.add(user)` then `log_add_user_to_group`. Already a member: no
/// change, still logged, as the controller logs regardless.
pub async fn add(
    conn: &mut PgConnection,
    cx: &crate::promotion::Ctx<'_>,
    group: &Group,
    user_id: i32,
    acting_user_id: i32,
) -> Result<(), AppError> {
    refusals(conn, group).await?;
    // bulk_add_transaction
    let added: Option<i32> = sqlx::query_scalar(
        "INSERT INTO group_users (group_id, user_id, notification_level, created_at, updated_at) \
         SELECT $1, u.id, $3, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP FROM users u \
         WHERE u.id = $2 AND NOT EXISTS (SELECT 1 FROM group_users gu \
           WHERE gu.user_id = u.id AND gu.group_id = $1) \
         ON CONFLICT (group_id, user_id) DO NOTHING RETURNING user_id",
    )
    .bind(group.id)
    .bind(user_id)
    .bind(group.default_notification_level)
    .fetch_optional(&mut *conn)
    .await?;
    if added.is_some() {
        // sync_add_side_effects: update_title
        if let Some(title) = group.title() {
            sqlx::query(
                "UPDATE users SET title = $2 WHERE id = $1 AND (title IS NULL OR title = '')",
            )
            .bind(user_id)
            .bind(title)
            .execute(&mut *conn)
            .await?;
        }
        // set_primary_group_and_update_flair
        if group.primary_group {
            sqlx::query(
                "UPDATE users SET flair_group_id = $2 WHERE id = $1 \
                 AND flair_group_id IS NOT DISTINCT FROM primary_group_id",
            )
            .bind(user_id)
            .bind(group.id)
            .execute(&mut *conn)
            .await?;
            sqlx::query(
                "UPDATE users u SET title = $2 WHERE u.id = $1 AND u.primary_group_id IS NOT NULL \
                 AND EXISTS (SELECT 1 FROM groups g WHERE g.id = u.primary_group_id AND g.title = u.title)",
            )
            .bind(user_id)
            .bind(&group.title)
            .execute(&mut *conn)
            .await?;
            sqlx::query("UPDATE users SET primary_group_id = $2 WHERE id = $1")
                .bind(user_id)
                .bind(group.id)
                .execute(&mut *conn)
                .await?;
        }
        // grant_trust_level: TrustLevelGranter.grant for one user.
        if let Some(level) = group.grant_trust_level.filter(|l| *l != 0) {
            let current: i32 = sqlx::query_scalar("SELECT trust_level FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await?;
            if current < level
                && let Err(message) =
                    crate::promotion::change_trust_level(conn, cx, user_id, level, None).await?
            {
                return Err(std::io::Error::other(message).into());
            }
        }
        // increase_group_user_count
        sqlx::query("UPDATE groups SET user_count = user_count + $2 WHERE id = $1")
            .bind(group.id)
            .bind(group.counted(user_id))
            .execute(&mut *conn)
            .await?;
    }
    log(conn, group.id, acting_user_id, user_id, ADD_USER_TO_GROUP).await?;
    Ok(())
}

/// `group.remove(user)` and, when it removed them,
/// `log_remove_user_from_group`.
pub async fn remove(
    conn: &mut PgConnection,
    group: &Group,
    user_id: i32,
    acting_user_id: i32,
) -> Result<(), AppError> {
    refusals(conn, group).await?;
    // bulk_remove_transaction
    let removed = sqlx::query("DELETE FROM group_users WHERE group_id = $1 AND user_id = $2")
        .bind(group.id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if removed == 0 {
        return Ok(());
    }
    // remove_primary_and_flair_group
    sqlx::query("UPDATE users SET primary_group_id = NULL WHERE id = $1 AND primary_group_id = $2")
        .bind(user_id)
        .bind(group.id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE users SET flair_group_id = NULL WHERE id = $1 AND flair_group_id = $2")
        .bind(user_id)
        .bind(group.id)
        .execute(&mut *conn)
        .await?;
    // grant_other_available_title
    if let Some(title) = group.title() {
        sqlx::query(
            "UPDATE users u SET title = NULL WHERE u.id = $1 AND u.title = $2 \
             AND NOT EXISTS (SELECT 1 FROM group_users gu JOIN groups g ON g.id = gu.group_id \
               WHERE gu.user_id = u.id AND g.title IS NOT NULL AND g.title <> '') \
             AND NOT EXISTS (SELECT 1 FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
               WHERE ub.user_id = u.id AND b.allow_title = true)",
        )
        .bind(user_id)
        .bind(title)
        .execute(&mut *conn)
        .await?;
        let kept: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND title = $2)")
                .bind(user_id)
                .bind(title)
                .fetch_one(&mut *conn)
                .await?;
        if kept {
            return Err(Unsupported("replacing a removed group title (next_best_title)").into());
        }
    }
    // decrease_group_user_count
    sqlx::query("UPDATE groups SET user_count = user_count - $2 WHERE id = $1")
        .bind(group.id)
        .bind(group.counted(user_id))
        .execute(&mut *conn)
        .await?;
    // recalculate_trust_level
    if group.grant_trust_level.is_some_and(|l| l != 0) {
        crate::jobs::enqueue(
            &mut *conn,
            "bulk_grant_trust_level",
            json!({ "user_ids": [user_id], "recalculate": true }),
        )
        .await?;
    }
    // enqueue_pm_notification_cleanup
    let topics: Vec<i32> = sqlx::query_scalar(
        "SELECT DISTINCT n.topic_id FROM notifications n \
         WHERE n.user_id = $1 AND n.topic_id IN (SELECT topic_id FROM topic_allowed_groups WHERE group_id = $2) \
         ORDER BY n.topic_id",
    )
    .bind(user_id)
    .bind(group.id)
    .fetch_all(&mut *conn)
    .await?;
    for topic_id in topics {
        crate::jobs::enqueue(
            &mut *conn,
            "delete_inaccessible_notifications",
            json!({ "topic_id": topic_id, "user_ids": [user_id] }),
        )
        .await?;
    }
    log(
        conn,
        group.id,
        acting_user_id,
        user_id,
        REMOVE_USER_FROM_GROUP,
    )
    .await?;
    Ok(())
}
