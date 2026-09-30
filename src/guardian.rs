//! Port of lib/guardian.rb, anonymous users only so far
//! (Guardian::AnonymousUser, lib/guardian.rb:39-121).

use sqlx::PgConnection;

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

pub struct Guardian {
    // Only the anonymous guardian exists until sessions are ported; the
    // field keeps call sites honest about which user they're asking for.
    user: Option<()>,
}

impl Guardian {
    pub fn anonymous() -> Self {
        Guardian { user: None }
    }

    pub fn is_anonymous(&self) -> bool {
        self.user.is_none()
    }

    pub fn is_authenticated(&self) -> bool {
        self.user.is_some()
    }

    pub fn is_admin(&self) -> bool {
        false
    }

    pub fn is_staff(&self) -> bool {
        false
    }

    /// `in_any_groups?`: for anonymous users, everyone (0) counts unless
    /// granular_anonymous_and_logged_in_groups_permissions is on, in which
    /// case only anonymous_users (4) does.
    pub fn in_any_groups(
        &self,
        settings: &SiteSettings,
        group_ids: &[i64],
    ) -> Result<bool, SettingError> {
        debug_assert!(self.is_anonymous());
        let granular = settings
            .get("granular_anonymous_and_logged_in_groups_permissions")?
            .truthy();
        Ok(group_ids.contains(&auto_groups::ANONYMOUS_USERS)
            || (!granular && group_ids.contains(&auto_groups::EVERYONE)))
    }

    /// `secure_category_ids`: categories the user may see despite
    /// read_restricted. None for anonymous.
    pub fn secure_category_ids(&self) -> Vec<i32> {
        Vec::new()
    }

    /// `allowed_category_ids`: public categories plus the secure ones.
    pub async fn allowed_category_ids(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Vec<i32>, sqlx::Error> {
        let mut ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM categories WHERE NOT read_restricted")
                .fetch_all(conn)
                .await?;
        ids.extend(self.secure_category_ids());
        Ok(ids)
    }

    /// `can_see_shared_draft?` needs shared_drafts_allowed_groups membership,
    /// never satisfied anonymously.
    pub fn can_see_shared_draft(&self) -> bool {
        false
    }

    /// `can_see_whispers?` requires a whisperer.
    pub fn can_see_whispers(&self) -> bool {
        false
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

    /// TagGuardian#can_tag_pms?: anonymous users can't.
    pub fn can_tag_pms(&self) -> bool {
        false
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
