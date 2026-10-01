//! Port of lib/guardian.rb: who the request runs as and what they may see.
//! Anonymous (Guardian::AnonymousUser) or a logged-in user with their
//! group memberships; the per-object predicates live with the models that
//! need them.

use sqlx::PgConnection;

use crate::session::current::SessionUser;
use crate::site_settings::{SettingError, SiteSettings};

/// `Group::AUTO_GROUPS`
pub mod auto_groups {
    pub const EVERYONE: i64 = 0;
    pub const ADMINS: i64 = 1;
    pub const MODERATORS: i64 = 2;
    pub const STAFF: i64 = 3;
    pub const ANONYMOUS_USERS: i64 = 4;
    pub const LOGGED_IN_USERS: i64 = 5;
    pub const TRUST_LEVEL_0: i64 = 10;
}

/// The logged-in half of a guardian.
#[derive(Debug, Clone)]
pub struct GuardianUser {
    pub user: SessionUser,
    /// `belonging_to_group_ids`
    pub group_ids: Vec<i64>,
    pub silenced: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Guardian {
    user: Option<GuardianUser>,
}

impl Guardian {
    pub fn anonymous() -> Self {
        Guardian { user: None }
    }

    /// A guardian for a session's user, with the group memberships every
    /// `in_any_groups?` needs loaded once.
    pub async fn for_user(
        conn: &mut PgConnection,
        user: &SessionUser,
    ) -> Result<Guardian, sqlx::Error> {
        let group_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT group_id::bigint FROM group_users WHERE user_id = $1 ORDER BY group_id",
        )
        .bind(user.id)
        .fetch_all(&mut *conn)
        .await?;
        let silenced: bool = sqlx::query_scalar(
            "SELECT silenced_till IS NOT NULL AND silenced_till > now() FROM users WHERE id = $1",
        )
        .bind(user.id)
        .fetch_one(&mut *conn)
        .await?;
        Ok(Guardian {
            user: Some(GuardianUser {
                user: user.clone(),
                group_ids,
                silenced,
            }),
        })
    }

    pub fn user(&self) -> Option<&SessionUser> {
        self.user.as_ref().map(|u| &u.user)
    }

    pub fn user_id(&self) -> Option<i32> {
        self.user().map(|u| u.id)
    }

    pub fn is_anonymous(&self) -> bool {
        self.user.is_none()
    }

    pub fn is_authenticated(&self) -> bool {
        self.user.is_some()
    }

    pub fn is_admin(&self) -> bool {
        self.user().is_some_and(|u| u.admin)
    }

    pub fn is_moderator(&self) -> bool {
        self.user().is_some_and(|u| u.moderator)
    }

    pub fn is_staff(&self) -> bool {
        self.is_admin() || self.is_moderator()
    }

    pub fn is_silenced(&self) -> bool {
        self.user.as_ref().is_some_and(|u| u.silenced)
    }

    /// `is_me?`
    pub fn is_me(&self, user_id: i32) -> bool {
        self.user_id() == Some(user_id)
    }

    /// `User#has_trust_level?`
    pub fn has_trust_level(&self, level: i32) -> bool {
        self.user()
            .is_some_and(|u| u.admin || u.moderator || u.staged || u.trust_level >= level)
    }

    pub fn group_ids(&self) -> &[i64] {
        self.user
            .as_ref()
            .map(|u| u.group_ids.as_slice())
            .unwrap_or(&[])
    }

    /// `in_any_groups?`: everyone (0) counts for anyone unless the granular
    /// setting is on, logged_in_users (5) for any user, anonymous_users (4)
    /// for anonymous ones; otherwise membership.
    pub fn in_any_groups(
        &self,
        settings: &SiteSettings,
        group_ids: &[i64],
    ) -> Result<bool, SettingError> {
        let granular = settings
            .get("granular_anonymous_and_logged_in_groups_permissions")?
            .truthy();
        if !granular && group_ids.contains(&auto_groups::EVERYONE) {
            return Ok(true);
        }
        match &self.user {
            None => Ok(group_ids.contains(&auto_groups::ANONYMOUS_USERS)),
            Some(u) => Ok(group_ids.contains(&auto_groups::LOGGED_IN_USERS)
                || group_ids.iter().any(|id| u.group_ids.contains(id))),
        }
    }

    /// `in_any_groups?` with a `*_allowed_groups` setting.
    pub fn in_setting_groups(
        &self,
        settings: &SiteSettings,
        setting: &str,
    ) -> Result<bool, SettingError> {
        self.in_any_groups(settings, &settings.group_ids(setting)?)
    }

    /// `secure_category_ids`: categories the user may see despite
    /// read_restricted: every one for admins, group-granted ones otherwise.
    pub async fn secure_category_ids(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<Vec<i32>, GuardianError> {
        let Some(u) = &self.user else {
            return Ok(Vec::new());
        };
        if u.user.admin
            && !settings
                .get("suppress_secured_categories_from_admin")?
                .truthy()
        {
            return Ok(sqlx::query_scalar(
                "SELECT id FROM categories WHERE read_restricted = TRUE ORDER BY id",
            )
            .fetch_all(conn)
            .await?);
        }
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT categories.id FROM categories \
             INNER JOIN category_groups ON categories.id = category_groups.category_id \
             INNER JOIN groups ON category_groups.group_id = groups.id \
             INNER JOIN group_users ON groups.id = group_users.group_id \
             WHERE group_users.user_id = $1 ORDER BY categories.id",
        )
        .bind(u.user.id)
        .fetch_all(conn)
        .await?)
    }

    /// `allowed_category_ids`: public categories plus the secure ones.
    pub async fn allowed_category_ids(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<Vec<i32>, GuardianError> {
        let mut ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM categories WHERE NOT read_restricted")
                .fetch_all(&mut *conn)
                .await?;
        ids.extend(self.secure_category_ids(conn, settings).await?);
        Ok(ids)
    }

    /// The SQL fragment `Topic.secured(guardian)` uses for categories: a
    /// category id list the caller binds.
    pub fn category_clause(&self, allowed: &[i32], column: &str) -> String {
        let list = allowed
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        if list.is_empty() {
            format!("{column} IS NULL")
        } else {
            format!("({column} IS NULL OR {column} IN ({list}))")
        }
    }

    /// `can_see_shared_draft?`
    pub fn can_see_shared_draft(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        self.in_setting_groups(settings, "shared_drafts_allowed_groups")
    }

    /// `can_see_whispers?`: whisperers (and staff within the whisper groups).
    pub fn can_see_whispers(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        self.is_whisperer(settings)
    }

    /// `User#whisperer?`
    pub fn is_whisperer(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        let Some(u) = &self.user else {
            return Ok(false);
        };
        let groups = settings.group_ids("whispers_allowed_groups")?;
        if groups.is_empty() {
            return Ok(false);
        }
        Ok(u.user.admin || groups.iter().any(|g| u.group_ids.contains(g)))
    }

    /// `can_see_deleted_posts?`: staff.
    pub fn can_see_deleted_posts(&self) -> bool {
        self.is_staff()
    }

    /// TagGuardian#can_create_tag?
    pub fn can_create_tag(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        Ok(settings.get("tagging_enabled")?.truthy()
            && self.in_any_groups(settings, &settings.group_ids("create_tag_allowed_groups")?)?)
    }

    /// TagGuardian#can_tag_topics?
    pub fn can_tag_topics(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        Ok(settings.get("tagging_enabled")?.truthy()
            && self.in_any_groups(settings, &settings.group_ids("tag_topic_allowed_groups")?)?)
    }

    /// TagGuardian#can_tag_pms?
    pub fn can_tag_pms(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        Ok(settings.get("tagging_enabled")?.truthy()
            && (self.is_staff()
                || self.in_setting_groups(settings, "pm_tags_allowed_for_groups")?))
    }

    /// `can_search?`: `authenticated? || allow_anonymous_search`
    pub fn can_search(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        Ok(self.is_authenticated() || settings.get("allow_anonymous_search")?.truthy())
    }

    /// `can_lazy_load_categories?`
    pub fn can_lazy_load_categories(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        self.in_any_groups(
            settings,
            &settings.group_ids("lazy_load_categories_groups")?,
        )
    }
}

#[derive(Debug)]
pub enum GuardianError {
    Db(sqlx::Error),
    Setting(SettingError),
}

impl std::fmt::Display for GuardianError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardianError::Db(e) => write!(f, "database: {e}"),
            GuardianError::Setting(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for GuardianError {}

impl From<sqlx::Error> for GuardianError {
    fn from(e: sqlx::Error) -> Self {
        GuardianError::Db(e)
    }
}

impl From<SettingError> for GuardianError {
    fn from(e: SettingError) -> Self {
        GuardianError::Setting(e)
    }
}
