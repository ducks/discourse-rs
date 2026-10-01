//! Port of lib/guardian/topic_guardian.rb, post_guardian.rb and the
//! invite/tag predicates the topic view serializes, over a topic and its
//! posts loaded once (`TopicCtx`, `PostCtx`). Category group moderation
//! and shared drafts are refused rather than ported.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::{Guardian, GuardianError};
use crate::site_settings::SiteSettings;

/// What the topic predicates read about a topic.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TopicCtx {
    pub id: i32,
    pub user_id: Option<i32>,
    pub category_id: Option<i32>,
    pub archetype: String,
    pub subtype: Option<String>,
    pub closed: bool,
    pub archived: bool,
    pub deleted_at: Option<NaiveDateTime>,
    pub posts_count: i32,
    pub created_at: NaiveDateTime,
    pub read_restricted: Option<bool>,
    /// `Category.exists?(topic_id: id)`
    pub is_category_topic: bool,
    pub shared_draft: bool,
    /// `Category.post_create_allowed(guardian).where(id: category_id).exists?`,
    /// true with no category.
    pub post_create_allowed: bool,
    /// `Category.topic_create_allowed(guardian).where(id: category_id)`
    pub topic_create_allowed: bool,
    /// `Discourse.static_doc_topic_ids.include?(id)`
    pub static_doc: bool,
    pub first_post_locked: bool,
    pub first_post_hidden: bool,
    pub first_post_hidden_at: Option<NaiveDateTime>,
    pub first_post_wiki: bool,
    /// `category.allow_unlimited_owner_edits_on_first_post`
    pub unlimited_owner_edits: bool,
    /// The category has non-automatic groups (whose owners may invite).
    pub manual_groups: bool,
}

/// What the post predicates read about a post.
#[derive(Debug, Clone)]
pub struct PostCtx {
    pub id: i32,
    pub user_id: Option<i32>,
    pub post_number: i32,
    pub post_type: i32,
    pub hidden: bool,
    pub hidden_at: Option<NaiveDateTime>,
    pub locked_by_id: Option<i32>,
    pub deleted_at: Option<NaiveDateTime>,
    pub user_deleted: bool,
    pub wiki: bool,
    pub created_at: NaiveDateTime,
    /// The author is staff (flagging staff may be off).
    pub author_staff: bool,
}

impl TopicCtx {
    /// Loads the context for a topic, with the category permission checks
    /// evaluated for this guardian. None when the topic doesn't exist.
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        guardian: &Guardian,
        topic_id: i32,
    ) -> Result<Option<TopicCtx>, GuardianError> {
        let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
        let exclude_uncategorized =
            !settings.get("allow_uncategorized_topics")?.truthy() && !guardian.is_staff();
        let static_docs: Vec<i64> = ["tos_topic_id", "guidelines_topic_id", "privacy_topic_id"]
            .iter()
            .map(|s| settings.get(s).map(|v| v.to_i()))
            .collect::<Result<_, _>>()?;
        let (uid, staged, admin) = match guardian.user() {
            Some(u) => (Some(u.id), u.staged, u.admin),
            None => (None, false, false),
        };
        // Category.scoped_to_permissions for a user; anonymous users may
        // create nothing, admins anything.
        let scoped = |permissions: &str| {
            if admin {
                "TRUE".to_string()
            } else if uid.is_none() {
                "FALSE".to_string()
            } else {
                format!(
                    "(($2 AND LENGTH(COALESCE(c.email_in, '')) > 0 AND c.email_in_allow_strangers) \
                     OR c.id NOT IN (SELECT category_id FROM category_groups) \
                     OR c.id IN (SELECT category_id FROM category_groups WHERE permission_type IN ({permissions}) \
                         AND (group_id = 0 OR group_id IN (SELECT group_id FROM group_users WHERE user_id = $3))))"
                )
            }
        };
        let sql = format!(
            "SELECT t.id, t.user_id, t.category_id, t.archetype, t.subtype, t.closed, t.archived, t.deleted_at, \
                    t.posts_count, t.created_at, c.read_restricted, \
                    EXISTS (SELECT 1 FROM categories WHERE topic_id = t.id) AS is_category_topic, \
                    EXISTS (SELECT 1 FROM shared_drafts WHERE topic_id = t.id) AS shared_draft, \
                    (c.id IS NULL OR {post_create}) AS post_create_allowed, \
                    (c.id IS NOT NULL AND {topic_create} AND (NOT $4 OR c.id <> $5)) AS topic_create_allowed, \
                    t.id = ANY($6) AS static_doc, \
                    COALESCE(fp.locked_by_id IS NOT NULL, FALSE) AS first_post_locked, \
                    COALESCE(fp.hidden, FALSE) AS first_post_hidden, \
                    fp.hidden_at AS first_post_hidden_at, \
                    COALESCE(fp.wiki, FALSE) AS first_post_wiki, \
                    COALESCE(c.allow_unlimited_owner_edits_on_first_post, FALSE) AS unlimited_owner_edits, \
                    EXISTS (SELECT 1 FROM category_groups cg JOIN groups g ON g.id = cg.group_id \
                            WHERE cg.category_id = c.id AND NOT g.automatic) AS manual_groups \
             FROM topics t LEFT JOIN categories c ON c.id = t.category_id \
             LEFT JOIN posts fp ON fp.topic_id = t.id AND fp.post_number = 1 AND fp.deleted_at IS NULL \
             WHERE t.id = $1",
            post_create = scoped("2, 1"),
            topic_create = scoped("1"),
        );
        let static_docs: Vec<i32> = static_docs.into_iter().map(|id| id as i32).collect();
        Ok(sqlx::query_as(&sql)
            .bind(topic_id)
            .bind(staged)
            .bind(uid.unwrap_or(0))
            .bind(exclude_uncategorized)
            .bind(uncategorized)
            .bind(&static_docs)
            .fetch_optional(conn)
            .await?)
    }

    pub fn trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn private_message(&self) -> bool {
        self.archetype == "private_message"
    }
}

impl PostCtx {
    pub fn trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn is_first_post(&self) -> bool {
        self.post_number == 1
    }
}

impl Guardian {
    /// The parts of the guardian this slice doesn't port: category group
    /// moderators, and shared drafts (checked by the list query already).
    fn refuse_unported(&self, settings: &SiteSettings) -> Result<(), GuardianError> {
        if self.is_authenticated() && settings.get("enable_category_group_moderation")?.truthy() {
            return Err(Unsupported("category group moderation").into());
        }
        Ok(())
    }

    /// `can_delete_all_posts_and_topics?`
    pub fn can_delete_all_posts_and_topics(
        &self,
        settings: &SiteSettings,
    ) -> Result<bool, GuardianError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        Ok(self.in_setting_groups(settings, "delete_all_posts_and_topics_allowed_groups")?)
    }

    /// `can_see_deleted_topics?` / `can_see_deleted_posts?(category)`
    pub fn can_see_deleted(&self, settings: &SiteSettings) -> Result<bool, GuardianError> {
        self.refuse_unported(settings)?;
        self.can_delete_all_posts_and_topics(settings)
    }

    /// `can_see_topic?(topic, hide_deleted)` for regular topics (PMs are
    /// refused), with the viewer's secure category ids.
    pub fn can_see_topic(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        hide_deleted: bool,
        secure_category_ids: &[i32],
    ) -> Result<bool, GuardianError> {
        if self.is_admin()
            && !settings
                .get("suppress_secured_categories_from_admin")?
                .truthy()
        {
            return Ok(true);
        }
        if hide_deleted && topic.trashed() && !self.can_see_deleted(settings)? {
            return Ok(false);
        }
        if topic.private_message() {
            return Err(Unsupported("private message topics").into());
        }
        if topic.shared_draft && !self.can_see_shared_draft(settings)? {
            return Ok(false);
        }
        if topic.read_restricted != Some(true) {
            return Ok(true);
        }
        if self.user().is_some_and(|u| u.staged) {
            return Err(Unsupported("staged users on restricted categories").into());
        }
        // can_see_category?: membership in a group granted the category.
        Ok(topic
            .category_id
            .is_some_and(|id| secure_category_ids.contains(&id)))
    }

    /// `trusted_with_post_edits?`
    fn trusted_with_post_edits(&self, settings: &SiteSettings) -> Result<bool, GuardianError> {
        Ok(self.in_setting_groups(settings, "edit_post_allowed_groups")?)
    }

    /// `edit_time_limit_expired?(user)` (LimitedEdit) for something created
    /// at `created_at`.
    fn edit_time_limit_expired(
        &self,
        settings: &SiteSettings,
        created_at: NaiveDateTime,
    ) -> Result<bool, GuardianError> {
        let Some(u) = self.user() else {
            return Ok(true);
        };
        if !self.trusted_with_post_edits(settings)? {
            return Ok(true);
        }
        let limit = if u.trust_level < 2 {
            settings.get("post_edit_time_limit")?.to_i()
        } else {
            settings.get("tl2_post_edit_time_limit")?.to_i()
        };
        let now = chrono::Utc::now().naive_utc();
        Ok(limit > 0 && created_at < now - chrono::Duration::minutes(limit))
    }

    /// `can_create_post?(topic)` (`can_create_post_in_topic?`), given the
    /// spam-rule answer from `can_create_post_anywhere`.
    pub fn can_create_post(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_post_anywhere: bool,
    ) -> Result<bool, GuardianError> {
        if !settings.get("enable_system_message_replies")?.truthy()
            && topic.subtype.as_deref() == Some("system_message")
        {
            return Ok(false);
        }
        if topic.private_message() {
            return Err(Unsupported("private message topics").into());
        }
        if !can_post_anywhere {
            return Ok(false);
        }
        Ok(topic.post_create_allowed)
    }

    /// `can_create_post_on_topic?`
    pub fn can_create_post_on_topic(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_post_anywhere: bool,
    ) -> Result<bool, GuardianError> {
        if topic.trashed() {
            return Ok(false);
        }
        if self.is_admin() {
            return Ok(true);
        }
        self.refuse_unported(settings)?;
        let trusted = (self.is_authenticated() && self.has_trust_level(4)) || self.is_moderator();
        Ok((!(topic.closed || topic.archived) || trusted)
            && self.can_create_post(settings, topic, can_post_anywhere)?)
    }

    /// `can_edit_topic?`
    pub fn can_edit_topic(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_see: bool,
        can_post_anywhere: bool,
    ) -> Result<bool, GuardianError> {
        if topic.static_doc && !self.is_admin() {
            return Ok(false);
        }
        if !can_see {
            return Ok(false);
        }
        if topic.first_post_locked && !self.is_staff() {
            return Ok(false);
        }
        if self.is_admin() {
            return Ok(true);
        }
        let can_create_post = self.can_create_post(settings, topic, can_post_anywhere)?;
        if self.is_moderator() && can_create_post {
            return Ok(true);
        }
        self.refuse_unported(settings)?;
        let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
        if settings.get("allow_uncategorized_topics")?.truthy()
            || topic.category_id != Some(uncategorized)
        {
            // can_create_topic_on_category?: can_create_topic?(nil) first,
            // which the caller folded into topic_create_allowed for users
            // who may create topics at all.
            if !topic.topic_create_allowed {
                return Ok(false);
            }
        }
        if settings.get("shared_drafts_category")?.presence().is_some() {
            return Err(Unsupported("shared drafts").into());
        }
        if self.in_setting_groups(settings, "edit_all_post_groups")?
            && topic.archived
            && !topic.private_message()
            && can_create_post
        {
            return Ok(true);
        }
        let edit_topic_groups = settings.group_ids("edit_all_topic_groups")?;
        if !edit_topic_groups.is_empty()
            && self.in_any_groups(settings, &edit_topic_groups)?
            && !topic.archived
            && !topic.private_message()
            && can_create_post
        {
            return Ok(true);
        }
        if topic.archived {
            return Ok(false);
        }
        Ok(self.is_me_opt(topic.user_id)
            && !self.edit_time_limit_expired(settings, topic.created_at)?
            && !topic.first_post_locked
            && (!topic.first_post_hidden
                || self.can_edit_hidden_post(settings, topic.first_post_hidden_at)?))
    }

    fn is_me_opt(&self, user_id: Option<i32>) -> bool {
        user_id.is_some_and(|id| self.is_me(id))
    }

    /// `can_edit_hidden_post?`
    fn can_edit_hidden_post(
        &self,
        settings: &SiteSettings,
        hidden_at: Option<NaiveDateTime>,
    ) -> Result<bool, GuardianError> {
        let Some(hidden_at) = hidden_at else {
            return Ok(true);
        };
        let cooldown = settings.get("cooldown_minutes_after_hiding_posts")?.to_i();
        Ok(hidden_at < chrono::Utc::now().naive_utc() - chrono::Duration::minutes(cooldown))
    }

    /// `can_moderate?(topic)`
    pub fn can_moderate(&self, topic_can_see: bool) -> bool {
        self.is_authenticated()
            && !self.is_silenced()
            && (self.is_staff() || (self.has_trust_level(4) && topic_can_see))
    }

    /// `can_perform_action_available_to_group_moderators?`
    pub fn can_perform_action_available_to_group_moderators(
        &self,
        settings: &SiteSettings,
        topic_can_see: bool,
    ) -> Result<bool, GuardianError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        if self.is_staff() {
            return Ok(true);
        }
        if !topic_can_see {
            return Ok(false);
        }
        if self.has_trust_level(4) {
            return Ok(true);
        }
        self.refuse_unported(settings)?;
        Ok(false)
    }

    /// `can_delete_topic?`
    pub fn can_delete_topic(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
    ) -> Result<bool, GuardianError> {
        if topic.trashed() || topic.is_category_topic || topic.static_doc {
            return Ok(false);
        }
        self.refuse_unported(settings)?;
        if self.can_delete_all_posts_and_topics(settings)? {
            return Ok(true);
        }
        let day_ago = chrono::Utc::now().naive_utc() - chrono::Duration::hours(24);
        Ok(self.is_me_opt(topic.user_id) && topic.posts_count <= 1 && topic.created_at > day_ago)
    }

    /// `can_recover_topic?` for a live topic: only a deleted one can be
    /// recovered, which this slice never serves.
    pub fn can_recover_topic(&self, topic: &TopicCtx) -> bool {
        debug_assert!(!topic.trashed());
        false
    }

    /// `can_remove_allowed_users?(topic, target = nil)`
    pub fn can_remove_allowed_users(&self, topic: &TopicCtx) -> bool {
        self.is_staff() || (self.is_me_opt(topic.user_id) && self.has_trust_level(2))
    }

    /// `can_invite_to?(topic)`
    pub fn can_invite_to(&self, topic: &TopicCtx, can_see: bool) -> Result<bool, GuardianError> {
        if self.is_anonymous() || !can_see {
            return Ok(false);
        }
        if topic.private_message() {
            return Err(Unsupported("private message topics").into());
        }
        if topic.read_restricted == Some(true) {
            // category.groups.where(automatic: false).any? { can_edit_group? }:
            // admins may; owners of a manual group may (not ported).
            if !topic.manual_groups {
                return Ok(false);
            }
            if self.is_admin() {
                return Ok(true);
            }
            return Err(Unsupported("inviting to restricted categories (group ownership)").into());
        }
        Ok(true)
    }

    /// `can_invite_to_forum?`
    pub fn can_invite_to_forum(&self, settings: &SiteSettings) -> Result<bool, GuardianError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        if !self.in_setting_groups(settings, "invite_allowed_groups")? {
            return Ok(false);
        }
        Ok(settings.get("max_invites_per_day")?.to_i() > 0 || self.is_staff())
    }

    /// `can_invite_via_email?(topic)`
    pub fn can_invite_via_email(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_see: bool,
    ) -> Result<bool, GuardianError> {
        if !self.can_invite_to_forum(settings)? || !self.can_invite_to(topic, can_see)? {
            return Ok(false);
        }
        Ok((settings.get("enable_local_logins")?.truthy()
            || settings.get("enable_discourse_connect")?.truthy())
            && (!settings.get("must_approve_users")?.truthy() || self.is_staff()))
    }

    /// `can_reply_as_new_topic?`
    pub fn can_reply_as_new_topic(&self) -> bool {
        self.is_authenticated() && self.has_trust_level(1)
    }

    /// `can_convert_topic?`
    pub fn can_convert_topic(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_create_post: bool,
    ) -> Result<bool, GuardianError> {
        if topic.trashed() || topic.is_category_topic {
            return Ok(false);
        }
        if self.is_admin() {
            return Ok(true);
        }
        if !self.in_setting_groups(settings, "personal_message_enabled_groups")? {
            return Ok(false);
        }
        Ok(self.is_moderator() && can_create_post)
    }

    /// `can_review_topic?`
    pub fn can_review_topic(
        &self,
        settings: &SiteSettings,
        can_see: bool,
    ) -> Result<bool, GuardianError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        if self.is_staff() {
            return Ok(true);
        }
        if !can_see {
            return Ok(false);
        }
        self.refuse_unported(settings)?;
        Ok(false)
    }

    /// `can_edit_tags?(topic)`
    pub fn can_edit_tags(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_edit_topic: bool,
        can_create_post: bool,
    ) -> Result<bool, GuardianError> {
        if !self.can_tag_topics(settings)? {
            return Ok(false);
        }
        if topic.private_message() && !self.can_tag_pms(settings)? {
            return Ok(false);
        }
        if can_edit_topic {
            return Ok(true);
        }
        if topic.first_post_wiki
            && self.in_setting_groups(settings, "edit_wiki_post_allowed_groups")?
        {
            return Ok(can_create_post);
        }
        Ok(false)
    }

    /// `can_publish_page?`
    pub fn can_publish_page(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        can_see: bool,
    ) -> Result<bool, GuardianError> {
        Ok(settings.get("enable_page_publishing")?.truthy()
            && !settings.get("secure_uploads")?.truthy()
            && !topic.private_message()
            && can_see
            && self.is_staff())
    }

    /// `can_banner_topic?`
    pub fn can_banner_topic(&self, topic: &TopicCtx) -> bool {
        self.is_authenticated()
            && !topic.private_message()
            && topic.read_restricted != Some(true)
            && self.is_staff()
    }

    /// `can_see_all_hidden_posts?`
    pub fn can_see_all_hidden_posts(&self, settings: &SiteSettings) -> Result<bool, GuardianError> {
        if self.is_anonymous()
            && !settings
                .get("granular_anonymous_and_logged_in_groups_permissions")?
                .truthy()
        {
            return Ok(false);
        }
        self.refuse_unported(settings)?;
        Ok(self.in_setting_groups(settings, "hidden_post_visible_groups")?)
    }

    /// `can_see_hidden_post?(post)`
    pub fn can_see_hidden_post(
        &self,
        settings: &SiteSettings,
        post: &PostCtx,
    ) -> Result<bool, GuardianError> {
        if self.can_see_all_hidden_posts(settings)? {
            return Ok(true);
        }
        if self.is_anonymous() {
            return Ok(false);
        }
        Ok(self.is_me_opt(post.user_id))
    }

    /// `can_see_deleted_post?(post)`
    pub fn can_see_deleted_post(&self, post: &PostCtx) -> bool {
        post.trashed() && self.is_authenticated() && self.is_staff()
    }

    /// `can_see_post?(post)` for a post of a topic the viewer can see.
    pub fn can_see_post(
        &self,
        settings: &SiteSettings,
        post: &PostCtx,
        topic_can_see: bool,
    ) -> Result<bool, GuardianError> {
        if self.is_admin() {
            return Ok(true);
        }
        if !topic_can_see {
            return Ok(false);
        }
        if !self.visible_post_types(settings)?.contains(&post.post_type) {
            return Ok(false);
        }
        if self.is_moderator() {
            return Ok(true);
        }
        self.refuse_unported(settings)?;
        Ok((!post.trashed() || self.can_see_deleted_post(post))
            && (!post.hidden || self.can_see_hidden_post(settings, post)?))
    }

    /// `Topic.visible_post_types(user)`
    pub fn visible_post_types(&self, settings: &SiteSettings) -> Result<Vec<i32>, GuardianError> {
        let mut types = vec![1, 2, 3];
        if self.is_whisperer(settings)? {
            types.push(4);
        }
        Ok(types)
    }

    /// `can_edit_post?(post)`
    pub fn can_edit_post(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        post: &PostCtx,
        topic_can_see: bool,
        can_create_post: bool,
    ) -> Result<bool, GuardianError> {
        if topic.static_doc && !self.is_admin() {
            return Ok(false);
        }
        if self.is_admin() {
            return Ok(true);
        }
        if post.locked_by_id.is_some() && !self.is_staff() {
            return Ok(false);
        }
        if self.in_setting_groups(settings, "edit_all_post_groups")? {
            if self.is_staff() || topic_can_see {
                return Ok(can_create_post);
            }
            return Ok(false);
        }
        if !topic_can_see {
            return Ok(false);
        }
        self.refuse_unported(settings)?;
        if topic.archived || post.user_deleted || post.trashed() {
            return Ok(false);
        }
        if settings.get("shared_drafts_category")?.presence().is_some() {
            return Err(Unsupported("shared drafts").into());
        }
        if post.wiki
            && self.is_authenticated()
            && self.in_setting_groups(settings, "edit_wiki_post_allowed_groups")?
        {
            return Ok(can_create_post);
        }
        if self.is_anonymous() || !self.trusted_with_post_edits(settings)? {
            return Ok(false);
        }
        if self.is_me_opt(post.user_id) {
            if !self.is_staff() && !self.visible_post_types(settings)?.contains(&post.post_type) {
                return Ok(false);
            }
            if self.is_silenced() {
                return Ok(false);
            }
            if post.hidden {
                return self.can_edit_hidden_post(settings, post.hidden_at);
            }
            if post.is_first_post() && topic.unlimited_owner_edits {
                return Ok(true);
            }
            return Ok(!self.edit_time_limit_expired(settings, post.created_at)?);
        }
        if topic.is_category_topic {
            // can_edit_category_description?: group moderators, refused above.
            return Ok(false);
        }
        Ok(false)
    }

    /// `can_delete_post?(post)`
    pub fn can_delete_post(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        post: &PostCtx,
        can_see_post: bool,
    ) -> Result<bool, GuardianError> {
        if !can_see_post || post.is_first_post() {
            return Ok(false);
        }
        self.refuse_unported(settings)?;
        if self.can_delete_all_posts_and_topics(settings)? {
            return Ok(true);
        }
        if topic.archived {
            return Ok(false);
        }
        if self.is_me_opt(post.user_id) {
            if settings.get("max_post_deletions_per_minute")?.to_i() < 1
                || settings.get("max_post_deletions_per_day")?.to_i() < 1
            {
                return Ok(false);
            }
            return Ok(!post.user_deleted);
        }
        Ok(false)
    }

    /// `can_recover_post?(post)`
    pub fn can_recover_post(
        &self,
        settings: &SiteSettings,
        post: &PostCtx,
        topic_can_see: bool,
    ) -> Result<bool, GuardianError> {
        // can_moderate_topic? = is_staff? || group-moderator action
        let can_moderate_topic = self.is_staff()
            || self.can_perform_action_available_to_group_moderators(settings, topic_can_see)?;
        if can_moderate_topic && post.trashed() {
            return Ok(true);
        }
        if self.is_me_opt(post.user_id) {
            if settings.get("max_post_deletions_per_minute")?.to_i() < 1
                || settings.get("max_post_deletions_per_day")?.to_i() < 1
            {
                return Ok(false);
            }
            if post.user_deleted && !post.trashed() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `can_wiki?(post)`
    pub fn can_wiki(&self, settings: &SiteSettings, post: &PostCtx) -> Result<bool, GuardianError> {
        if self.is_anonymous() {
            return Ok(false);
        }
        if self.is_staff() || self.has_trust_level(4) {
            return Ok(true);
        }
        if self.in_setting_groups(settings, "self_wiki_allowed_groups")?
            && self.is_me_opt(post.user_id)
        {
            if post.hidden {
                return Ok(false);
            }
            return Ok(!self.edit_time_limit_expired(settings, post.created_at)?);
        }
        Ok(false)
    }

    /// `can_view_edit_history?(post)`
    pub fn can_view_edit_history(
        &self,
        settings: &SiteSettings,
        post: &PostCtx,
        can_see_post: bool,
    ) -> Result<bool, GuardianError> {
        if !post.hidden && (post.wiki || settings.get("edit_history_visible_to_public")?.truthy()) {
            return Ok(true);
        }
        if self.is_anonymous() || !can_see_post {
            return Ok(false);
        }
        if self.is_staff() || self.is_me_opt(post.user_id) {
            return Ok(true);
        }
        self.refuse_unported(settings)?;
        Ok(false)
    }

    /// `can_delete_post_action?(post_action)` for one the viewer owns on a
    /// post of `topic`.
    pub fn can_delete_post_action(
        &self,
        settings: &SiteSettings,
        topic: &TopicCtx,
        action_user_id: i32,
        action_created_at: NaiveDateTime,
    ) -> Result<bool, GuardianError> {
        if !self.is_me(action_user_id) || topic.private_message() {
            return Ok(false);
        }
        if settings.get("allow_anonymous_mode")?.truthy() {
            return Err(Unsupported("anonymous posting mode").into());
        }
        let window = settings.get("post_undo_action_window_mins")?.to_i();
        Ok(
            action_created_at > chrono::Utc::now().naive_utc() - chrono::Duration::minutes(window)
                && !topic.archived,
        )
    }
}
