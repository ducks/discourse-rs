//! Port of lib/guardian.rb: who the request runs as and what they may see.
//! Anonymous (Guardian::AnonymousUser) or a logged-in user with their
//! group memberships; the per-object predicates live with the models that
//! need them.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::Unsupported;
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
        // One round-trip: the memberships and the silence in a row.
        let (group_ids, silenced): (Vec<i64>, bool) = sqlx::query_as(
            "SELECT COALESCE((SELECT array_agg(group_id::bigint ORDER BY group_id) FROM group_users WHERE user_id = $1), '{}'), \
                    silenced_till IS NOT NULL AND silenced_till > now() \
             FROM users WHERE id = $1",
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

    /// `allowed_category_ids` as a subquery, for composing into other SQL.
    pub fn allowed_category_ids_sql(
        &self,
        settings: &SiteSettings,
    ) -> Result<String, SettingError> {
        let public = "SELECT id FROM categories WHERE NOT read_restricted";
        let Some(u) = &self.user else {
            return Ok(public.to_string());
        };
        if u.user.admin
            && !settings
                .get("suppress_secured_categories_from_admin")?
                .truthy()
        {
            return Ok("SELECT id FROM categories".to_string());
        }
        Ok(format!(
            "{public} UNION SELECT categories.id FROM categories \
             INNER JOIN category_groups ON categories.id = category_groups.category_id \
             INNER JOIN group_users ON category_groups.group_id = group_users.group_id \
             WHERE group_users.user_id = {}",
            u.user.id
        ))
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

    /// `private_message_topic_scope`'s clause: PMs the user is allowed
    /// into directly or through a group. Moderators also see warnings and
    /// flagged PMs, which this slice refuses when any pending flag exists.
    pub fn private_message_clause(&self) -> Option<String> {
        let uid = self.user_id()?;
        Some(format!(
            "(topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {uid}) \
             OR topics.id IN (SELECT tg.topic_id FROM topic_allowed_groups tg \
                JOIN group_users gu ON gu.user_id = {uid} AND gu.group_id = tg.group_id))"
        ))
    }

    /// `can_see_topic_ids(topic_ids:)`: the given ids the guardian may
    /// see (`visible_topic_scope`), in the order given. Admins see all.
    pub async fn can_see_topic_ids(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
        topic_ids: &[i32],
    ) -> Result<Vec<i32>, GuardianError> {
        if topic_ids.is_empty() {
            return Ok(Vec::new());
        }
        let suppress = settings
            .get("suppress_secured_categories_from_admin")?
            .truthy();
        if self.is_admin() && !suppress {
            return Ok(topic_ids.to_vec());
        }
        if self.is_moderator() {
            let pending: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM reviewables WHERE type = 'ReviewableFlaggedPost' AND status = 0)",
            )
            .fetch_one(&mut *conn)
            .await?;
            if pending {
                return Err(Unsupported("moderators' view of flagged private messages").into());
            }
        }
        let mut clauses = vec![
            "topics.id = ANY($1)".to_string(),
            "topics.deleted_at IS NULL".to_string(),
        ];
        if self.is_staff() {
            // Staff keep deleted topics in the scope; the serializers
            // that follow refuse them anyway.
            clauses.pop();
        }
        if !self.can_see_shared_draft(settings)? {
            clauses.push("shared_drafts.id IS NULL".to_string());
        }
        let regular = format!(
            "(topics.archetype <> 'private_message' AND (topics.category_id IS NULL OR topics.category_id IN ({})))",
            self.allowed_category_ids_sql(settings)?
        );
        let visible = match self.private_message_clause() {
            Some(pm) => {
                let warning = if self.is_moderator() {
                    " OR topics.subtype = 'moderator_warning'"
                } else {
                    ""
                };
                format!("({regular} OR (topics.archetype = 'private_message' AND {pm}){warning})")
            }
            None => regular,
        };
        clauses.push(visible);
        let ids: Vec<i32> = sqlx::query_scalar(&format!(
            "SELECT topics.id FROM topics LEFT OUTER JOIN shared_drafts ON shared_drafts.topic_id = topics.id \
             WHERE {}",
            clauses.join(" AND ")
        ))
        .bind(topic_ids)
        .fetch_all(conn)
        .await?;
        Ok(topic_ids
            .iter()
            .copied()
            .filter(|id| ids.contains(id))
            .collect())
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

    /// `can_see_unlisted_topics?`: staff or TL4.
    pub fn can_see_unlisted_topics(&self) -> bool {
        self.is_staff() || self.has_trust_level(4)
    }

    /// `!SpamRule::AutoSilence.prevent_posting?`: silenced users and new
    /// users the flag score would auto-silence cannot post anywhere.
    pub async fn can_create_post_anywhere(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<bool, GuardianError> {
        let Some(u) = &self.user else {
            return Ok(false);
        };
        if u.silenced {
            return Ok(false);
        }
        if self.has_trust_level(1) || u.user.staged {
            return Ok(true);
        }
        let needed = settings.get("num_users_to_silence_new_user")?.to_i();
        if needed <= 0 {
            return Ok(true);
        }
        let (total, count): (Option<f64>, i64) = sqlx::query_as(
            "SELECT SUM(rs.score)::float8, COUNT(DISTINCT rs.user_id) FROM reviewables r \
             INNER JOIN reviewable_scores rs ON rs.reviewable_id = r.id \
             WHERE r.target_created_by_id = $1 AND rs.reviewable_score_type = 8 AND rs.status IN (0, 1)",
        )
        .bind(u.user.id)
        .fetch_one(&mut *conn)
        .await?;
        if total.unwrap_or(0.0) > 0.0 && count >= needed {
            return Err(Unsupported("spam auto-silence scoring").into());
        }
        Ok(true)
    }

    /// `can_create_topic?(nil)`: staff, or a member of
    /// create_topic_allowed_groups who may post and has some category
    /// (`Category.topic_create_allowed`) to post in.
    pub async fn can_create_topic(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<bool, GuardianError> {
        let Some(u) = &self.user else {
            return Ok(false);
        };
        if self.is_staff() {
            return Ok(true);
        }
        if !self.in_setting_groups(settings, "create_topic_allowed_groups")?
            || !self.can_create_post_anywhere(&mut *conn, settings).await?
        {
            return Ok(false);
        }
        let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
        let exclude_uncategorized = !settings.get("allow_uncategorized_topics")?.truthy();
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM categories \
             WHERE (($2 AND LENGTH(COALESCE(email_in, '')) > 0 AND email_in_allow_strangers) \
                    OR categories.id NOT IN (SELECT category_id FROM category_groups) \
                    OR categories.id IN (SELECT category_id FROM category_groups WHERE permission_type IN (1) \
                        AND (group_id = 0 OR group_id IN (SELECT group_id FROM group_users WHERE user_id = $1)))) \
             AND (NOT $3 OR categories.id <> $4))",
        )
        .bind(u.user.id)
        .bind(u.user.staged)
        .bind(exclude_uncategorized)
        .bind(uncategorized)
        .fetch_one(conn)
        .await?)
    }

    /// `Category.topic_create_allowed(guardian).pluck(:id)`: none for
    /// anonymous users, every category for admins.
    pub async fn topic_create_allowed_category_ids(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<Vec<i32>, GuardianError> {
        let Some(u) = &self.user else {
            return Ok(Vec::new());
        };
        if u.user.admin {
            return Ok(sqlx::query_scalar("SELECT id FROM categories ORDER BY id")
                .fetch_all(conn)
                .await?);
        }
        let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
        let exclude_uncategorized =
            !settings.get("allow_uncategorized_topics")?.truthy() && !self.is_staff();
        Ok(sqlx::query_scalar(
            "SELECT id FROM categories \
             WHERE (($2 AND LENGTH(COALESCE(email_in, '')) > 0 AND email_in_allow_strangers) \
                    OR categories.id NOT IN (SELECT category_id FROM category_groups) \
                    OR categories.id IN (SELECT category_id FROM category_groups WHERE permission_type IN (1) \
                        AND (group_id = 0 OR group_id IN (SELECT group_id FROM group_users WHERE user_id = $1)))) \
             AND (NOT $3 OR categories.id <> $4) ORDER BY id",
        )
        .bind(u.user.id)
        .bind(u.user.staged)
        .bind(exclude_uncategorized)
        .bind(uncategorized)
        .fetch_all(conn)
        .await?)
    }

    /// `UserOption#treat_as_new_topic_start_date`; None for anonymous.
    pub async fn treat_as_new_topic_start_date(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<Option<NaiveDateTime>, GuardianError> {
        let Some(u) = &self.user else {
            return Ok(None);
        };
        let row: Option<NewTopicRow> = sqlx::query_as(
            "SELECT u.created_at, u.previous_visit_at, us.new_since, uo.new_topic_duration_minutes \
             FROM users u LEFT JOIN user_stats us ON us.user_id = u.id \
             LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = $1",
        )
        .bind(u.user.id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((created_at, previous_visit_at, new_since, minutes)) = row else {
            return Ok(None);
        };
        let duration = minutes.map(i64::from).unwrap_or(
            settings
                .get("default_other_new_topic_duration_minutes")?
                .to_i(),
        );
        let now = crate::clock::now_naive();
        let base = match duration {
            -1 => created_at,
            -2 => previous_visit_at.or(new_since).unwrap_or(created_at),
            minutes => now - chrono::Duration::minutes(minutes),
        };
        let min_new =
            chrono::DateTime::from_timestamp(settings.get("min_new_topics_time")?.to_i(), 0)
                .map(|t| t.naive_utc())
                .unwrap_or(created_at);
        Ok(Some(base.max(created_at).max(min_new)))
    }

    /// `UpcomingChanges.enabled_for_user?(name, user)`: the setting is on
    /// and, when it has a group list in site_setting_groups, the user is
    /// in one of them.
    pub async fn upcoming_change_enabled(
        &self,
        conn: &mut PgConnection,
        settings: &SiteSettings,
        name: &str,
    ) -> Result<bool, GuardianError> {
        if !settings.get(name)?.truthy() {
            return Ok(false);
        }
        let groups: Option<Option<String>> =
            sqlx::query_scalar("SELECT group_ids FROM site_setting_groups WHERE name = $1")
                .bind(name)
                .fetch_optional(&mut *conn)
                .await?;
        match groups.flatten() {
            Some(list) => {
                let ids: Vec<i64> = list
                    .split('|')
                    .filter(|s| !s.is_empty())
                    .map(crate::ruby::to_i)
                    .collect();
                Ok(self.in_any_groups(settings, &ids)?)
            }
            None => Ok(true),
        }
    }

    /// `Tag.topic_count_column(guardian)`: the staff count for staff or
    /// when secure categories count for everyone.
    pub fn tag_count_column(&self, settings: &SiteSettings) -> Result<&'static str, SettingError> {
        Ok(
            if self.is_staff()
                || settings
                    .get("include_secure_categories_in_tag_counts")?
                    .truthy()
            {
                "staff_topic_count"
            } else {
                "public_topic_count"
            },
        )
    }

    /// `can_mute_users?`: staff or TL1+.
    pub fn can_mute_users(&self) -> bool {
        self.user()
            .is_some_and(|u| u.admin || u.moderator || u.trust_level >= 1)
    }

    /// `can_ignore_users?`: staff or ignore_allowed_groups.
    pub fn can_ignore_users(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        Ok(self.is_staff() || self.in_setting_groups(settings, "ignore_allowed_groups")?)
    }

    /// `can_send_private_messages?`: bots and system aside, members of
    /// personal_message_enabled_groups.
    pub fn can_send_private_messages(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        let Some(u) = self.user() else {
            return Ok(false);
        };
        if u.id < 0 {
            return Ok(true);
        }
        self.in_setting_groups(settings, "personal_message_enabled_groups")
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

    /// TagGuardian#can_tag_pms?: the system user, else pm_tags_allowed_for_groups.
    pub fn can_tag_pms(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        if self.user_id() == Some(-1) {
            return Ok(settings.get("tagging_enabled")?.truthy());
        }
        Ok(settings.get("tagging_enabled")?.truthy()
            && self.in_setting_groups(settings, "pm_tags_allowed_for_groups")?)
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

type NewTopicRow = (
    NaiveDateTime,
    Option<NaiveDateTime>,
    Option<NaiveDateTime>,
    Option<i32>,
);

#[derive(Debug)]
pub enum GuardianError {
    Db(sqlx::Error),
    Setting(SettingError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for GuardianError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuardianError::Db(e) => write!(f, "database: {e}"),
            GuardianError::Setting(e) => e.fmt(f),
            GuardianError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl From<Unsupported> for GuardianError {
    fn from(e: Unsupported) -> Self {
        GuardianError::Unsupported(e)
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
