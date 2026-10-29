//! discourse-assign, on the tables Rails' plugin created (`assignments`,
//! `silenced_assignments`), the category custom field naming the groups
//! that may assign in a category, `groups.assignable_level` and
//! `user_options.notification_level_when_assigned`: who may assign
//! (DiscourseAssign::AssignmentPermissions and the guardian extension),
//! assigning (`assigner`), the notification jobs (`jobs`), the events it
//! listens to (`lifecycle`) and the keys it adds to serializers (`keys`).

pub mod assigner;
pub mod bulk;
pub mod jobs;
pub mod keys;
pub mod lifecycle;
pub mod lists;
pub mod reminder;
pub mod view;

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

/// `DiscourseAssign::PLUGIN_NAME`
pub const PLUGIN_NAME: &str = "discourse-assign";

/// `AssignmentPermissions::CATEGORY_ADDITIONAL_ASSIGN_ALLOWED_GROUPS`
pub const CATEGORY_ADDITIONAL_ASSIGN_ALLOWED_GROUPS: &str = "additional_assign_allowed_on_groups";

/// `Notification.types[:assigned]`
pub const ASSIGNED: i32 = 34;

/// `PendingAssignsReminder::REMINDERS_FREQUENCY`, a user custom field.
pub const REMINDERS_FREQUENCY: &str = "remind_assigns_frequency";

/// `DiscourseCalendar::HOLIDAY_CUSTOM_FIELD`
pub const HOLIDAY_CUSTOM_FIELD: &str = "on_holiday";

/// `enabled_site_setting :assign_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("assign_enabled")?.truthy())
}

/// `SiteSetting.assigns_public`
pub fn public(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("assigns_public")?.truthy())
}

/// `SiteSetting.enable_assign_status`
pub fn status_enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("enable_assign_status")?.truthy())
}

/// `Assignment.statuses`
pub fn statuses(settings: &SiteSettings) -> Result<Vec<String>, SettingError> {
    Ok(settings
        .get("assign_statuses")?
        .to_s()
        .split('|')
        .map(str::to_string)
        .collect())
}

/// `Assignment.default_status`: the first status.
pub fn default_status(settings: &SiteSettings) -> Result<Option<String>, SettingError> {
    Ok(statuses(settings)?
        .into_iter()
        .next()
        .filter(|s| !s.is_empty()))
}

/// An `assignments` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Assignment {
    pub id: i64,
    pub topic_id: i32,
    pub assigned_to_id: i32,
    pub assigned_by_user_id: i32,
    pub assigned_to_type: String,
    pub target_id: i32,
    pub target_type: String,
    /// The column is nullable; only TRUE is active (`where(active: true)`).
    pub active: bool,
    pub note: Option<String>,
    pub status: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// The columns an [`Assignment`] reads, over `assignments` aliased `a`.
pub const COLUMNS: &str = "a.id, a.topic_id, a.assigned_to_id, a.assigned_by_user_id, \
    a.assigned_to_type, a.target_id, a.target_type, a.active IS TRUE AS active, a.note, a.status, \
    a.created_at, a.updated_at";

impl Assignment {
    pub fn assigned_to_user(&self) -> bool {
        self.assigned_to_type == "User"
    }

    pub fn assigned_to_group(&self) -> bool {
        self.assigned_to_type == "Group"
    }

    pub fn on_post(&self) -> bool {
        self.target_type == "Post"
    }

    pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM assignments a WHERE a.id = $1"
        ))
        .bind(id)
        .fetch_optional(conn)
        .await
    }

    /// `target.assignment` (`has_one :assignment, as: :target`).
    pub async fn for_target(
        conn: &mut PgConnection,
        target_type: &str,
        target_id: i32,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM assignments a WHERE a.target_type = $1 AND a.target_id = $2 \
             ORDER BY a.id LIMIT 1"
        ))
        .bind(target_type)
        .bind(target_id)
        .fetch_optional(conn)
        .await
    }

    /// `assignment.post`: the topic's first post for a topic, else the
    /// post assigned. (id, post_number), None when it is gone.
    pub async fn post(&self, conn: &mut PgConnection) -> Result<Option<(i32, i32)>, sqlx::Error> {
        if self.on_post() {
            sqlx::query_as("SELECT id, post_number FROM posts WHERE id = $1 AND deleted_at IS NULL")
                .bind(self.target_id)
                .fetch_optional(conn)
                .await
        } else {
            sqlx::query_as(
                "SELECT id, post_number FROM posts WHERE topic_id = $1 AND post_number = 1 \
                 AND deleted_at IS NULL",
            )
            .bind(self.target_id)
            .fetch_optional(conn)
            .await
        }
    }

    /// `assigned_users`: a group's members, or the user.
    pub async fn assigned_users(
        conn: &mut PgConnection,
        assigned_to_type: &str,
        assigned_to_id: i32,
    ) -> Result<Vec<i32>, sqlx::Error> {
        if assigned_to_type == "Group" {
            sqlx::query_scalar(
                "SELECT gu.user_id FROM group_users gu JOIN users u ON u.id = gu.user_id \
                 WHERE gu.group_id = $1 ORDER BY gu.id",
            )
            .bind(assigned_to_id)
            .fetch_all(conn)
            .await
        } else {
            sqlx::query_scalar("SELECT id FROM users WHERE id = $1")
                .bind(assigned_to_id)
                .fetch_all(conn)
                .await
        }
    }
}

/// Where an assignment's target sits, for the category scoped
/// permissions (`AssignmentPermissions.category_id_for`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scope {
    /// No target: assigning anywhere.
    Anywhere,
    /// A topic or post, by its topic's category (None for a message).
    Category(Option<i32>),
}

/// DiscourseAssign::AssignmentPermissions, its inputs read once: the
/// assign_allowed_on_groups setting and the categories' additional groups.
#[derive(Debug, Clone, Default)]
pub struct Permissions {
    global: Vec<i32>,
    /// (category_id, the field's value), by row id.
    scoped: Vec<(i32, String)>,
}

/// `group_ids_from_value`: the pipe list's non-zero integers.
fn group_ids_from_value(value: &str) -> Vec<i32> {
    value
        .split('|')
        .map(|v| crate::ruby::to_i(v) as i32)
        .filter(|id| *id != 0)
        .collect()
}

fn union(a: &[i32], b: &[i32]) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    for id in a.iter().chain(b) {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

fn intersect(a: &[i32], b: &[i64]) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    for id in a {
        if b.contains(&i64::from(*id)) && !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

/// An id list for SQL, `NULL` when empty (`IN (NULL)` matches nothing).
pub(crate) fn id_list(ids: &[i32]) -> String {
    if ids.is_empty() {
        "NULL".to_string()
    } else {
        ids.iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Permissions {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<Self, super::PluginError> {
        let global = settings
            .group_ids("assign_allowed_on_groups")?
            .into_iter()
            .map(|id| id as i32)
            .collect();
        let scoped: Vec<(i32, Option<String>)> = sqlx::query_as(
            "SELECT category_id, value FROM category_custom_fields WHERE name = $1 ORDER BY id",
        )
        .bind(CATEGORY_ADDITIONAL_ASSIGN_ALLOWED_GROUPS)
        .fetch_all(conn)
        .await?;
        Ok(Permissions {
            global,
            scoped: scoped
                .into_iter()
                .map(|(c, v)| (c, v.unwrap_or_default()))
                .collect(),
        })
    }

    /// `global_group_ids` (`SiteSetting.assign_allowed_on_groups_map`).
    pub fn global_group_ids(&self) -> &[i32] {
        &self.global
    }

    /// `all_scoped_group_ids`
    pub fn all_scoped_group_ids(&self) -> Vec<i32> {
        let mut out = Vec::new();
        for (_, value) in &self.scoped {
            for id in group_ids_from_value(value) {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// `all_assign_allowed_group_ids`
    pub fn all_assign_allowed_group_ids(&self) -> Vec<i32> {
        union(&self.global, &self.all_scoped_group_ids())
    }

    /// `scoped_group_ids_for_category`: the first field row's groups.
    pub fn scoped_group_ids_for_category(&self, category_id: Option<i32>) -> Vec<i32> {
        let Some(category_id) = category_id else {
            return Vec::new();
        };
        self.scoped
            .iter()
            .find(|(c, _)| *c == category_id)
            .map(|(_, v)| group_ids_from_value(v))
            .unwrap_or_default()
    }

    /// `scoped_group_ids_for_user`
    pub fn scoped_group_ids_for_user(&self, guardian: &Guardian) -> Vec<i32> {
        if guardian.is_anonymous() {
            return Vec::new();
        }
        intersect(&self.all_scoped_group_ids(), guardian.group_ids())
    }

    /// `Guardian#can_assign_globally?`
    pub fn can_assign_globally(&self, guardian: &Guardian) -> bool {
        if guardian.is_anonymous() {
            return false;
        }
        guardian.is_admin() || !intersect(&self.global, guardian.group_ids()).is_empty()
    }

    /// `Guardian#can_assign?` without a target.
    pub fn can_assign(&self, guardian: &Guardian) -> bool {
        if guardian.is_anonymous() {
            return false;
        }
        self.can_assign_globally(guardian) || !self.scoped_group_ids_for_user(guardian).is_empty()
    }

    /// `Guardian#can_assign?(target)`, by the target's category.
    pub fn can_assign_in(&self, guardian: &Guardian, category_id: Option<i32>) -> bool {
        if guardian.is_anonymous() {
            return false;
        }
        if self.can_assign_globally(guardian) {
            return true;
        }
        if category_id.is_none() {
            return false;
        }
        !intersect(
            &self.scoped_group_ids_for_category(category_id),
            guardian.group_ids(),
        )
        .is_empty()
    }

    /// The groups whose members may be assigned:
    /// `assign_allowed_groups_for_target`'s, else
    /// `assign_allowed_groups_for_user`'s (before their visibility).
    pub fn allowed_group_ids(&self, scope: Scope, assigner: &Guardian) -> Vec<i32> {
        match scope {
            Scope::Category(category_id) => union(
                &self.global,
                &self.scoped_group_ids_for_category(category_id),
            ),
            Scope::Anywhere if self.can_assign_globally(assigner) => {
                self.all_assign_allowed_group_ids()
            }
            Scope::Anywhere => union(&self.global, &self.scoped_group_ids_for_user(assigner)),
        }
    }

    /// `allowed_user_ids_for_target(target)`, or without a target
    /// `assignable_user_ids_for_user(assigner)`, as a subquery of user
    /// ids: human admins and the members of the groups that may be
    /// assigned.
    pub fn assignable_user_ids_sql(&self, scope: Scope, assigner: Option<&Guardian>) -> String {
        let group_ids = match scope {
            Scope::Category(category_id) => union(
                &self.global,
                &self.scoped_group_ids_for_category(category_id),
            ),
            Scope::Anywhere if assigner.is_some_and(|g| self.can_assign_globally(g)) => {
                self.all_assign_allowed_group_ids()
            }
            Scope::Anywhere => union(
                &self.global,
                &assigner
                    .map(|g| self.scoped_group_ids_for_user(g))
                    .unwrap_or_default(),
            ),
        };
        format!(
            "SELECT users.id FROM users WHERE users.id > 0 AND users.admin \
             UNION SELECT group_users.user_id FROM group_users WHERE group_users.group_id IN ({})",
            id_list(&group_ids)
        )
    }

    /// `allowed_group_ids_for_target(assigner, target)` as a condition over
    /// `groups`: `Group.assignable(assigner)`, or a group allowed in the
    /// target's category; without a target, `Group.assignable` alone.
    pub fn assignable_groups_where(&self, scope: Scope, assigner: &Guardian) -> String {
        let assignable = assignable_where(assigner);
        match scope {
            Scope::Anywhere => assignable,
            Scope::Category(category_id) => format!(
                "(({assignable}) OR groups.id IN ({}))",
                id_list(&self.scoped_group_ids_for_category(category_id))
            ),
        }
    }
}

/// `Group.assignable(user)` (GroupExtension) over `groups`.
pub fn assignable_where(guardian: &Guardian) -> String {
    // Group.alias_levels(user)
    let levels = if guardian.is_admin() {
        "99, 1, 2, 3, 4"
    } else if guardian.is_moderator() {
        "99, 2, 3, 4"
    } else {
        "99"
    };
    let user_id = guardian
        .user_id()
        .map_or("NULL".to_string(), |id| id.to_string());
    format!(
        "groups.assignable_level IN ({levels}) OR \
         (groups.assignable_level = 3 AND groups.id IN (\
            SELECT group_id FROM group_users AS gu WHERE gu.user_id = {user_id})) OR \
         (groups.assignable_level = 4 AND groups.id IN (\
            SELECT group_id FROM group_users AS gu WHERE gu.user_id = {user_id} AND gu.owner IS TRUE))"
    )
}

/// `Group#can_show_assigned_tab?`: an assignable level above nobody.
pub fn can_show_assigned_tab(assignable_level: i32) -> bool {
    assignable_level > 0
}

/// The category of a topic, for [`Scope::Category`].
pub async fn topic_scope(conn: &mut PgConnection, topic_id: i32) -> Result<Scope, sqlx::Error> {
    let category_id: Option<Option<i32>> =
        sqlx::query_scalar("SELECT category_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(conn)
            .await?;
    Ok(Scope::Category(category_id.flatten()))
}

/// The users who may be assigned in a scope, for publishing to them
/// (`assignable_user_ids(target).pluck(:id)`).
pub async fn assignable_user_ids(
    conn: &mut PgConnection,
    permissions: &Permissions,
    scope: Scope,
) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT id FROM ({}) AS users ORDER BY id",
        permissions.assignable_user_ids_sql(scope, None)
    ))
    .fetch_all(conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_ids_from_a_field_value_drop_zero_and_junk() {
        assert_eq!(group_ids_from_value("3|41|x|0|7"), vec![3, 41, 7]);
        assert_eq!(group_ids_from_value(""), Vec::<i32>::new());
    }

    #[test]
    fn category_scope_reads_the_first_row_and_users_their_groups() {
        let p = Permissions {
            global: vec![3],
            scoped: vec![(4, "41|42".into()), (5, "43".into()), (4, "99".into())],
        };
        assert_eq!(p.scoped_group_ids_for_category(Some(4)), vec![41, 42]);
        assert_eq!(p.scoped_group_ids_for_category(None), Vec::<i32>::new());
        assert_eq!(p.all_scoped_group_ids(), vec![41, 42, 43, 99]);
        assert_eq!(p.all_assign_allowed_group_ids(), vec![3, 41, 42, 43, 99]);
    }
}
