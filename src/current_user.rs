//! CurrentUserSerializer (GET /session/current.json) for a logged-in user,
//! with the slice of Guardian a logged-in user needs for it: group-based
//! permissions (`in_any_groups?`), allowed categories, and the predicates
//! the serializer asks.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::AppState;
use crate::Unsupported;
use crate::session::current::{Session, SessionUser};
use crate::site_settings::SiteSettings;

/// `Group::AUTO_GROUPS`
pub const EVERYONE: i32 = 0;
pub const LOGGED_IN_USERS: i32 = 5;
pub const STAFF: i32 = 3;

/// A logged-in user's permission context.
pub struct UserGuardian<'a> {
    pub user: &'a SessionUser,
    pub settings: &'a SiteSettings,
    /// `belonging_to_group_ids`
    pub group_ids: Vec<i32>,
    pub silenced: bool,
    /// `@secure_category_ids ||=`, shared with the request's Guardian when
    /// built from it.
    secure_category_ids: std::sync::Arc<std::sync::OnceLock<Vec<i32>>>,
}

impl<'a> UserGuardian<'a> {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &'a SiteSettings,
        user: &'a SessionUser,
    ) -> Result<UserGuardian<'a>, AppError> {
        let group_ids: Vec<i32> =
            sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1")
                .bind(user.id)
                .fetch_all(&mut *conn)
                .await?;
        let silenced: bool = sqlx::query_scalar(
            "SELECT silenced_till IS NOT NULL AND silenced_till > now() FROM users WHERE id = $1",
        )
        .bind(user.id)
        .fetch_one(&mut *conn)
        .await?;
        Ok(UserGuardian {
            user,
            settings,
            group_ids,
            silenced,
            secure_category_ids: Default::default(),
        })
    }

    /// The request's guardian as a UserGuardian, with what it already
    /// loaded: memberships, silence and secure category ids.
    pub fn from_guardian(
        settings: &'a SiteSettings,
        guardian: &'a crate::guardian::GuardianUser,
    ) -> UserGuardian<'a> {
        UserGuardian {
            user: &guardian.user,
            settings,
            group_ids: guardian.group_ids.iter().map(|id| *id as i32).collect(),
            silenced: guardian.silenced,
            secure_category_ids: guardian.secure_category_ids.clone(),
        }
    }

    pub fn is_admin(&self) -> bool {
        self.user.admin
    }

    pub fn is_staff(&self) -> bool {
        self.user.admin || self.user.moderator
    }

    /// `SiteSetting.<name>_map`
    pub fn group_map(&self, setting: &str) -> Result<Vec<i32>, AppError> {
        let granular = self
            .settings
            .get("granular_anonymous_and_logged_in_groups_permissions")?
            .truthy();
        Ok(self
            .settings
            .get(setting)?
            .to_s()
            .split('|')
            .filter(|s| !s.is_empty())
            .map(|s| crate::ruby::to_i(s) as i32)
            .map(|id| {
                if id == EVERYONE && granular {
                    LOGGED_IN_USERS
                } else {
                    id
                }
            })
            .collect())
    }

    /// `User#in_any_groups?`
    pub fn in_any_groups(&self, ids: &[i32]) -> Result<bool, AppError> {
        let granular = self
            .settings
            .get("granular_anonymous_and_logged_in_groups_permissions")?
            .truthy();
        if ids.contains(&EVERYONE) && !granular {
            return Ok(true);
        }
        if ids.contains(&LOGGED_IN_USERS) {
            return Ok(true);
        }
        Ok(ids.iter().any(|id| self.group_ids.contains(id)))
    }

    pub fn in_setting_groups(&self, setting: &str) -> Result<bool, AppError> {
        let map = self.group_map(setting)?;
        self.in_any_groups(&map)
    }

    /// `has_trust_level?`
    pub fn has_trust_level(&self, level: i32) -> bool {
        self.user.admin || self.user.moderator || self.user.staged || self.user.trust_level >= level
    }

    /// `allowed_category_ids`: public plus the secure ones this user may read.
    pub async fn allowed_category_ids(
        &self,
        conn: &mut PgConnection,
    ) -> Result<Vec<i32>, AppError> {
        let mut ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM categories WHERE read_restricted = FALSE")
                .fetch_all(&mut *conn)
                .await?;
        if let Some(secure) = self.secure_category_ids.get() {
            ids.extend(secure);
            return Ok(ids);
        }
        let secure: Vec<i32> = if self.is_admin()
            && !self
                .settings
                .get("suppress_secured_categories_from_admin")?
                .truthy()
        {
            sqlx::query_scalar("SELECT id FROM categories WHERE read_restricted = TRUE ORDER BY id")
                .fetch_all(&mut *conn)
                .await?
        } else {
            sqlx::query_scalar(
                "SELECT DISTINCT categories.id FROM categories \
                 INNER JOIN category_groups ON categories.id = category_groups.category_id \
                 INNER JOIN groups ON category_groups.group_id = groups.id \
                 INNER JOIN group_users ON groups.id = group_users.group_id \
                 WHERE group_users.user_id = $1 ORDER BY categories.id",
            )
            .bind(self.user.id)
            .fetch_all(&mut *conn)
            .await?
        };
        ids.extend(self.secure_category_ids.get_or_init(|| secure));
        Ok(ids)
    }

    /// `UpcomingChanges.enabled_for_user?(name, user)` for a stable change:
    /// the admin override wins, else on, gated by its group row.
    pub async fn upcoming_change_enabled(
        &self,
        conn: &mut PgConnection,
        name: &str,
    ) -> Result<bool, AppError> {
        let enabled = match self.settings.get(name) {
            Ok(v) => v.truthy(),
            Err(_) => true,
        };
        if !enabled {
            return Ok(false);
        }
        let groups: Option<Option<String>> =
            sqlx::query_scalar("SELECT group_ids FROM site_setting_groups WHERE name = $1")
                .bind(name)
                .fetch_optional(&mut *conn)
                .await?;
        match groups.flatten() {
            Some(list) => {
                let ids: Vec<i32> = list
                    .split('|')
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::ruby::to_i(s) as i32)
                    .collect();
                self.in_any_groups(&ids)
            }
            None => Ok(true),
        }
    }
}

const COMMUNITY_SECTION: i32 = 0;

/// id, name, slug, description, pm_only
type SidebarTag = (i32, String, Option<String>, Option<String>, bool);

fn ids_json(ids: &[i32]) -> Value {
    json!(ids)
}

/// `CategoryUser.indirectly_muted_category_ids`: subcategories with no
/// level of their own under a muted parent (or grandparent, with three
/// levels of nesting).
pub async fn indirectly_muted_category_ids(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    uid: i32,
) -> Result<Vec<i32>, AppError> {
    let nesting = settings.get("max_category_nesting")?.to_i();
    let default_level = if settings.get("mute_all_categories_by_default")?.truthy() {
        0
    } else {
        1
    };
    let mut sql = String::from(
        "SELECT categories.id FROM categories \
         LEFT JOIN categories categories2 ON categories2.id = categories.parent_category_id \
         LEFT JOIN category_users ON category_users.category_id = categories.id AND category_users.user_id = $1 \
         LEFT JOIN category_users category_users2 ON category_users2.category_id = categories2.id AND category_users2.user_id = $1 ",
    );
    if nesting == 3 {
        sql.push_str(
            "LEFT JOIN categories categories3 ON categories3.id = categories2.parent_category_id \
             LEFT JOIN category_users category_users3 ON category_users3.category_id = categories3.id AND category_users3.user_id = $1 ",
        );
    }
    sql.push_str(
        "WHERE categories.parent_category_id IS NOT NULL \
         AND ((category_users.id IS NULL AND COALESCE(category_users2.notification_level, $2) = 0)",
    );
    if nesting == 3 {
        sql.push_str(
            " OR (category_users.id IS NULL AND category_users2.id IS NULL AND COALESCE(category_users3.notification_level, $2) = 0)",
        );
    }
    sql.push(')');
    Ok(sqlx::query_scalar(&sql)
        .bind(uid)
        .bind(default_level)
        .fetch_all(&mut *conn)
        .await?)
}

/// `/session/current.json`'s `current_user`.
pub async fn serialize(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    session: &Session,
) -> Result<Value, AppError> {
    let user = &session.user;
    let uid = user.id;
    let g = UserGuardian::load(conn, settings, user).await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let enable_names = settings.get("enable_names")?.truthy();
    let tagging = settings.get("tagging_enabled")?.truthy();

    #[derive(sqlx::FromRow)]
    struct Row {
        name: Option<String>,
        uploaded_avatar_id: Option<i32>,
        title: Option<String>,
        previous_visit_at: Option<chrono::NaiveDateTime>,
        seen_notification_id: i64,
        primary_group_id: Option<i32>,
        flair_group_id: Option<i32>,
        required_fields_version: Option<i32>,
        created_at: chrono::NaiveDateTime,

        dismissed_banner_key: Option<i32>,
        skip_new_user_tips: bool,
        draft_count: i32,
        pending_posts_count: i32,
        read_faq: bool,
        topic_count: i32,
        post_count: i32,
        new_since: Option<chrono::NaiveDateTime>,
    }
    let row: Row = sqlx::query_as(
        "SELECT u.name, u.uploaded_avatar_id, u.title, u.previous_visit_at, u.seen_notification_id::bigint AS seen_notification_id, \
                u.primary_group_id, u.flair_group_id, u.required_fields_version, u.created_at, \
                up.dismissed_banner_key, COALESCE(uo.skip_new_user_tips, false) AS skip_new_user_tips, \
                COALESCE(us.draft_count, 0) AS draft_count, COALESCE(us.pending_posts_count, 0) AS pending_posts_count, \
                (us.read_faq IS NOT NULL) AS read_faq, COALESCE(us.topic_count, 0) AS topic_count, \
                COALESCE(us.post_count, 0) AS post_count, us.new_since \
         FROM users u \
         LEFT JOIN user_profiles up ON up.user_id = u.id \
         LEFT JOIN user_options uo ON uo.user_id = u.id \
         LEFT JOIN user_stats us ON us.user_id = u.id \
         WHERE u.id = $1",
    )
    .bind(uid)
    .fetch_one(&mut *conn)
    .await?;
    if settings.get("enable_user_status")?.truthy() {
        return Err(Unsupported("user status on current_user").into());
    }
    if settings.get("allow_anonymous_mode")?.truthy() {
        return Err(Unsupported("anonymous posting mode").into());
    }

    let mut out = Map::new();
    out.insert("id".into(), json!(uid));
    out.insert("username".into(), json!(user.username));
    if enable_names {
        out.insert("name".into(), json!(row.name));
    }
    out.insert(
        "avatar_template".into(),
        json!(crate::avatar::class_avatar_template(
            &urls,
            &user.username,
            row.uploaded_avatar_id
        )?),
    );

    // Notification counters.
    let seen = row.seen_notification_id;
    let unread: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT 1 FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
         WHERE t.deleted_at IS NULL AND n.high_priority = FALSE AND n.user_id = $1 AND n.id > $2 AND NOT read LIMIT 99) x",
    )
    .bind(uid)
    .bind(seen)
    .fetch_one(&mut *conn)
    .await?;
    let unread_high: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
         WHERE t.deleted_at IS NULL AND n.high_priority = TRUE AND n.user_id = $1 AND NOT read",
    )
    .bind(uid)
    .fetch_one(&mut *conn)
    .await?;
    let all_unread: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT 1 FROM notifications n LEFT JOIN topics t ON t.id = n.topic_id \
         WHERE t.deleted_at IS NULL AND n.user_id = $1 AND n.id > $2 AND NOT read LIMIT 99) x",
    )
    .bind(uid)
    .bind(seen)
    .fetch_one(&mut *conn)
    .await?;
    out.insert("unread_notifications".into(), json!(unread));
    out.insert(
        "unread_high_priority_notifications".into(),
        json!(unread_high),
    );
    out.insert("all_unread_notifications_count".into(), json!(all_unread));
    out.insert(
        "read_first_notification".into(),
        json!(seen != 0 || row.skip_new_user_tips),
    );
    out.insert("admin".into(), json!(user.admin));
    out.insert("notification_channel_position".into(), Value::Null);
    out.insert("do_not_disturb_channel_position".into(), json!(0));
    out.insert("moderator".into(), json!(user.moderator));
    out.insert("staff".into(), json!(g.is_staff()));
    let whisperer = {
        let map = g.group_map("whispers_allowed_groups")?;
        !map.is_empty() && (user.admin || map.iter().any(|id| g.group_ids.contains(id)))
    };
    out.insert("whisperer".into(), json!(whisperer));
    out.insert("title".into(), json!(row.title));
    let any_posts: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM posts WHERE deleted_at IS NULL AND user_id = $1)",
    )
    .bind(uid)
    .fetch_one(&mut *conn)
    .await?;
    out.insert("any_posts".into(), json!(any_posts));
    out.insert("trust_level".into(), json!(user.trust_level));
    out.insert(
        "can_send_private_email_messages".into(),
        json!(
            settings.get("enable_staged_users")?.truthy()
                && g.in_setting_groups("personal_message_enabled_groups")?
                && g.in_setting_groups("send_email_messages_allowed_groups")?
        ),
    );
    out.insert(
        "can_send_private_messages".into(),
        json!(can_send_private_messages(&g)?),
    );
    out.insert(
        "can_upload_avatar".into(),
        json!(g.in_setting_groups("uploaded_avatars_allowed_groups")?),
    );
    out.insert("can_edit".into(), json!(true));
    if can_invite_to_forum(&g, settings)? {
        out.insert("can_invite_to_forum".into(), json!(true));
    }
    if user.admin
        && g.upcoming_change_enabled(conn, "enable_invite_modal_with_roles")
            .await?
    {
        out.insert("can_create_admin_invite".into(), json!(true));
    }
    let has_password: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1)")
            .bind(uid)
            .fetch_one(&mut *conn)
            .await?;
    if !has_password {
        out.insert("no_password".into(), json!(true));
    }
    // can_delete_account: can_delete_user?(self)
    if !user.admin && !settings.get("enable_discourse_connect")?.truthy() {
        let max = settings.get("delete_user_self_max_post_count")?.to_i();
        // has_more_posts_than?(max)
        let more = if i64::from(row.topic_count + row.post_count) > max || max < 0 {
            true
        } else {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(1) FROM (SELECT 1 FROM posts p JOIN topics t ON p.topic_id = t.id \
                 WHERE p.user_id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL \
                 AND (t.archetype <> 'private_message' \
                      OR EXISTS (SELECT 1 FROM topic_allowed_users a WHERE a.topic_id = t.id AND a.user_id > 0 AND a.user_id <> $1) \
                      OR EXISTS (SELECT 1 FROM topic_allowed_groups g WHERE g.topic_id = p.topic_id)) \
                 LIMIT $2) x",
            )
            .bind(uid)
            .bind(max + 1)
            .fetch_one(&mut *conn)
            .await?;
            count > max
        };
        if !more {
            out.insert("can_delete_account".into(), json!(true));
        }
    }
    out.insert("can_post_anonymously".into(), json!(false));
    out.insert(
        "can_ignore_users".into(),
        json!(g.is_staff() || g.in_setting_groups("ignore_allowed_groups")?),
    );
    out.insert(
        "can_edit_tags".into(),
        json!(tagging && g.in_setting_groups("edit_tags_allowed_groups")?),
    );
    out.insert(
        "can_delete_all_posts_and_topics".into(),
        json!(g.in_setting_groups("delete_all_posts_and_topics_allowed_groups")?),
    );
    // custom_fields: public_user_custom_fields only (plugin fields ignored).
    let mut custom = Map::new();
    let names: Vec<String> = settings
        .get("public_user_custom_fields")?
        .to_s()
        .split('|')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if !names.is_empty() {
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT name, value FROM user_custom_fields WHERE user_id = $1 AND name = ANY($2) ORDER BY id")
                .bind(uid)
                .bind(&names)
                .fetch_all(&mut *conn)
                .await?;
        for (name, value) in rows {
            if custom.contains_key(&name) {
                return Err(Unsupported("repeated user custom fields (array values)").into());
            }
            custom.insert(name, json!(value));
        }
    }
    out.insert("custom_fields".into(), Value::Object(custom));

    // Category notification levels.
    let levels: Vec<(i32, i32)> = sqlx::query_as(
        "SELECT category_id, notification_level FROM category_users WHERE user_id = $1",
    )
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    let with_level = |level: i32| -> Vec<i32> {
        levels
            .iter()
            .filter(|(_, l)| *l == level)
            .map(|(c, _)| *c)
            .collect()
    };
    out.insert("muted_category_ids".into(), ids_json(&with_level(0)));
    let indirectly = indirectly_muted_category_ids(conn, settings, uid).await?;
    out.insert(
        "indirectly_muted_category_ids".into(),
        ids_json(&indirectly),
    );
    out.insert("regular_category_ids".into(), ids_json(&with_level(1)));
    out.insert("tracked_category_ids".into(), ids_json(&with_level(2)));
    out.insert(
        "watched_first_post_category_ids".into(),
        ids_json(&with_level(4)),
    );
    out.insert("watched_category_ids".into(), ids_json(&with_level(3)));

    // Tag notification levels, visible tags only.
    #[derive(sqlx::FromRow)]
    struct TagLevel {
        id: i32,
        name: String,
        slug: Option<String>,
        notification_level: i32,
    }
    let tag_rows: Vec<TagLevel> = sqlx::query_as(
        "SELECT DISTINCT tags.id, tags.name, tags.slug, tag_users.notification_level FROM tag_users \
         LEFT OUTER JOIN tag_group_memberships ON tag_users.tag_id = tag_group_memberships.tag_id \
         LEFT OUTER JOIN tag_group_permissions ON tag_group_memberships.tag_group_id = tag_group_permissions.tag_group_id \
         LEFT OUTER JOIN group_users ON group_users.user_id = tag_users.user_id \
         INNER JOIN tags ON tags.id = tag_users.tag_id \
         WHERE (tag_group_permissions.group_id IS NULL OR tag_group_permissions.group_id IN (0, group_users.group_id) \
                OR group_users.group_id = 3) \
         AND tag_users.user_id = $1 ORDER BY tags.id",
    )
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    let tags_at = |level: i32| -> Value {
        json!(
            tag_rows
                .iter()
                .filter(|t| t.notification_level == level)
                .map(|t| json!({"id": t.id, "name": t.name, "slug": t.slug}))
                .collect::<Vec<_>>()
        )
    };
    out.insert("watched_tags".into(), tags_at(3));
    out.insert("watching_first_post_tags".into(), tags_at(4));
    out.insert("tracked_tags".into(), tags_at(2));
    out.insert("muted_tags".into(), tags_at(0));
    out.insert("regular_tags".into(), tags_at(1));
    out.insert(
        "dismissed_banner_key".into(),
        json!(row.dismissed_banner_key),
    );
    out.insert("is_anonymous".into(), json!(false));

    // Reviewables.
    let category_moderation = settings.get("enable_category_group_moderation")?.truthy();
    if category_moderation {
        return Err(Unsupported("enable_category_group_moderation").into());
    }
    let (reviewable_count, unseen_reviewable_count) = if g.is_staff() {
        crate::reviewables::staff_counts(&mut *conn, settings, user.id, user.admin, user.moderator)
            .await?
    } else {
        (0, 0)
    };
    out.insert("reviewable_count".into(), json!(reviewable_count));
    out.insert(
        "unseen_reviewable_count".into(),
        json!(unseen_reviewable_count),
    );
    let new_pms: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id = $1 AND id > $2 AND NOT read AND notification_type = 6",
    )
    .bind(uid)
    .bind(seen)
    .fetch_one(&mut *conn)
    .await?;
    out.insert(
        "new_personal_messages_notifications_count".into(),
        json!(new_pms),
    );
    out.insert("read_faq".into(), json!(row.read_faq));
    out.insert(
        "previous_visit_at".into(),
        json!(row.previous_visit_at.map(crate::topic_list::time_json)),
    );
    out.insert("seen_notification_id".into(), json!(seen));
    if let Some(id) = row.primary_group_id {
        out.insert("primary_group_id".into(), json!(id));
    }
    out.insert("flair_group_id".into(), json!(row.flair_group_id));

    // can_create_topic?(nil)
    let can_create_post =
        !g.silenced && (g.has_trust_level(1) || !autosilence_pending(conn, settings, uid).await?);
    let can_create_topic = if g.is_staff() {
        true
    } else if g.in_setting_groups("create_topic_allowed_groups")? && can_create_post {
        let uncategorized = settings.get("uncategorized_category_id")?.to_i() as i32;
        let exclude_uncategorized = !settings.get("allow_uncategorized_topics")?.truthy();
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM categories \
             WHERE (($2 AND LENGTH(COALESCE(email_in, '')) > 0 AND email_in_allow_strangers) \
                    OR categories.id NOT IN (SELECT category_id FROM category_groups) \
                    OR categories.id IN (SELECT category_id FROM category_groups WHERE permission_type IN (1) \
                        AND (group_id = 0 OR group_id IN (SELECT group_id FROM group_users WHERE user_id = $1)))) \
             AND (NOT $3 OR categories.id <> $4))",
        )
        .bind(uid)
        .bind(user.staged)
        .bind(exclude_uncategorized)
        .bind(uncategorized)
        .fetch_one(&mut *conn)
        .await?;
        exists
    } else {
        false
    };
    out.insert("can_create_topic".into(), json!(can_create_topic));
    out.insert(
        "can_set_topic_timer".into(),
        json!(
            !g.silenced
                && (uid == -1
                    || g.is_staff()
                    || g.in_setting_groups("topic_timers_allowed_groups")?)
        ),
    );
    if user.admin || (settings.get("moderators_manage_categories")?.truthy() && user.moderator) {
        out.insert("can_create_category".into(), json!(true));
    }
    if user.admin || (settings.get("moderators_manage_groups")?.truthy() && user.moderator) {
        out.insert("can_create_group".into(), json!(true));
    }
    let link_posting_access = if g.in_setting_groups("post_links_allowed_groups")? {
        "full"
    } else if settings.get("allowed_link_domains")?.presence().is_some() {
        "limited"
    } else {
        "none"
    };
    out.insert("link_posting_access".into(), json!(link_posting_access));
    if settings.get("enable_discourse_connect")?.truthy() {
        return Err(Unsupported("external_id with DiscourseConnect").into());
    }
    if settings.get("include_associated_account_ids")?.truthy() {
        return Err(Unsupported("associated_account_ids").into());
    }
    let top_category_ids: Vec<i32> = sqlx::query_scalar(
        "SELECT category_id FROM category_users WHERE user_id = $1 AND notification_level NOT IN (0, 1) \
         ORDER BY CASE WHEN notification_level = 3 THEN 1 WHEN notification_level = 2 THEN 2 WHEN notification_level = 4 THEN 3 END \
         LIMIT $2",
    )
    .bind(uid)
    .bind(settings.get("header_dropdown_category_count")?.to_i())
    .fetch_all(&mut *conn)
    .await?;
    out.insert("top_category_ids".into(), ids_json(&top_category_ids));

    // groups: the user's visible groups, owned ones flagged.
    let owned: Vec<i32> =
        sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1 AND owner = TRUE")
            .bind(uid)
            .fetch_all(&mut *conn)
            .await?;
    let visibility = if user.admin {
        String::new()
    } else if user.moderator {
        " AND (groups.visibility_level IN (0,1,2,3) OR groups.id IN (SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id AND gu.user_id = $1 AND gu.owner WHERE g.visibility_level = 4))".to_string()
    } else {
        " AND groups.id IN (SELECT id FROM groups WHERE visibility_level IN (0,1) \
           UNION ALL SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id AND gu.user_id = $1 WHERE g.visibility_level = 2 \
           UNION ALL SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id AND gu.user_id = $1 AND gu.owner WHERE g.visibility_level IN (3,4))".to_string()
    };
    let groups: Vec<(i32, String, bool)> = sqlx::query_as(&format!(
        "SELECT groups.id, groups.name, groups.has_messages FROM groups \
         INNER JOIN group_users ON groups.id = group_users.group_id \
         WHERE group_users.user_id = $1 AND groups.id > 0 AND groups.id NOT IN (4, 5){visibility} \
         ORDER BY groups.name ASC"
    ))
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    out.insert(
        "groups".into(),
        json!(
            groups
                .into_iter()
                .map(|(id, name, has_messages)| {
                    let mut g = json!({"id": id, "name": name, "has_messages": has_messages});
                    if owned.contains(&id) {
                        g["owner"] = json!(true);
                    }
                    g
                })
                .collect::<Vec<_>>()
        ),
    );
    let latest_required: Option<i32> =
        sqlx::query_scalar("SELECT MAX(id) FROM user_required_fields_versions")
            .fetch_one(&mut *conn)
            .await?;
    out.insert(
        "needs_required_fields_check".into(),
        json!(row.required_fields_version.unwrap_or(0) < latest_required.unwrap_or(0)),
    );
    let second_factor: bool = sqlx::query_scalar(
        "SELECT (NOT $2::bool AND $3::bool) AND (EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND method = 1 AND enabled) \
          OR EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0))",
    )
    .bind(uid)
    .bind(settings.get("enable_discourse_connect")?.truthy())
    .bind(settings.get("enable_local_logins")?.truthy())
    .fetch_one(&mut *conn)
    .await?;
    out.insert("second_factor_enabled".into(), json!(second_factor));
    let ignored: Vec<String> = sqlx::query_scalar(
        "SELECT users.username FROM ignored_users INNER JOIN users ON users.id = ignored_users.ignored_user_id \
         WHERE ignored_users.user_id = $1",
    )
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    out.insert("ignored_users".into(), json!(ignored));
    let featured: Option<Option<i32>> =
        sqlx::query_scalar("SELECT featured_topic_id FROM user_profiles WHERE user_id = $1")
            .bind(uid)
            .fetch_optional(&mut *conn)
            .await?;
    if featured.flatten().is_some() {
        return Err(Unsupported("featured_topic on current_user").into());
    }
    let dnd: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
        "SELECT MAX(ends_at) FROM do_not_disturb_timings WHERE user_id = $1 AND starts_at <= now() AND ends_at > now()",
    )
    .bind(uid)
    .fetch_one(&mut *conn)
    .await?;
    out.insert(
        "do_not_disturb_until".into(),
        json!(dnd.map(crate::topic_list::time_json)),
    );
    out.insert("can_review".into(), json!(g.is_staff()));
    out.insert("draft_count".into(), json!(row.draft_count));
    out.insert("pending_posts_count".into(), json!(row.pending_posts_count));
    let grouped: Vec<(i32, i64)> = sqlx::query_as(
        "SELECT x.notification_type, COUNT(*) FROM (SELECT n.notification_type FROM notifications n \
         LEFT JOIN topics t ON t.id = n.topic_id WHERE t.deleted_at IS NULL AND n.user_id = $1 AND NOT n.read LIMIT 400) x \
         GROUP BY x.notification_type ORDER BY x.notification_type",
    )
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    let mut grouped_json = Map::new();
    for (t, c) in grouped {
        grouped_json.insert(t.to_string(), json!(c));
    }
    out.insert(
        "grouped_unread_notifications".into(),
        Value::Object(grouped_json),
    );

    out.extend(sidebar_fields(conn, settings, &g, user, tagging).await?);
    out.insert(
        "unified_new_enabled".into(),
        json!(
            g.upcoming_change_enabled(conn, "enable_unified_new")
                .await?
        ),
    );
    out.insert(
        "can_view_raw_email".into(),
        json!(g.in_setting_groups("view_raw_email_allowed_groups")?),
    );
    out.insert(
        "login_method".into(),
        json!(if session.token.authenticated_with_oauth.unwrap_or(false) {
            "oauth"
        } else {
            "local"
        }),
    );
    // AMS never finds `include_can_localize_content??`, so the key is
    // always present; the value needs the setting and the group.
    out.insert(
        "can_localize_content".into(),
        json!(
            settings.get("content_localization_enabled")?.truthy()
                && g.in_setting_groups("content_localization_allowed_groups")?
        ),
    );
    if g.is_staff() {
        out.insert("has_unseen_features".into(), json!(false));
        let last_visited: Option<Option<String>> = sqlx::query_scalar(
            "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = 'last_visited_upcoming_changes_at' ORDER BY id LIMIT 1",
        )
        .bind(uid)
        .fetch_optional(&mut *conn)
        .await?;
        let last_visited = last_visited.flatten().filter(|v| !v.is_empty());
        let site_created: Option<chrono::NaiveDateTime> = sqlx::query_scalar(
            "SELECT created_at FROM schema_migration_details ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&mut *conn)
        .await?;
        let has_new = if last_visited.is_none()
            && site_created.is_some_and(|c| row.created_at < c + chrono::Duration::hours(1))
        {
            false
        } else {
            let cutoff = match &last_visited {
                Some(v) => v
                    .parse::<chrono::DateTime<chrono::Utc>>()
                    .map(|t| t.naive_utc())
                    .unwrap_or(row.created_at),
                None => row.created_at,
            };
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM upcoming_change_events WHERE event_type = 0 \
                 AND (event_data->>'backfilled') IS DISTINCT FROM 'true' AND created_at > $1)",
            )
            .bind(cutoff)
            .fetch_one(&mut *conn)
            .await?
        };
        out.insert("has_new_upcoming_changes".into(), json!(has_new));
        out.insert(
            "can_see_emails".into(),
            json!(
                user.admin || (settings.get("moderators_view_emails")?.truthy() && user.moderator)
            ),
        );
    }
    if user.admin || (user.moderator && settings.get("moderators_view_ips")?.truthy()) {
        out.insert("can_see_ip".into(), json!(true));
    }
    out.insert("is_impersonating".into(), json!(false));
    out.insert("impersonation_expires_at".into(), Value::Null);
    if user.admin
        || (settings.get("moderators_change_post_ownership")?.truthy() && user.moderator)
        || g.in_setting_groups("change_post_ownership_allowed_groups")?
    {
        out.insert("can_change_post_owner".into(), json!(true));
    }
    if settings.get("enable_site_owner_onboarding")?.truthy() && user.admin {
        let first_admin: Option<i32> =
            sqlx::query_scalar("SELECT MIN(id) FROM users WHERE admin = TRUE AND id > 0")
                .fetch_one(&mut *conn)
                .await?;
        let days = settings.get("site_owner_onboarding_max_days")?.to_i();
        if first_admin == Some(uid)
            && row.created_at > crate::clock::now_naive() - chrono::Duration::days(days)
        {
            out.insert("show_site_owner_onboarding".into(), json!(true));
        }
    }
    if user.admin {
        let theme = settings.get("default_theme_id")?.to_i();
        let custom_themes: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM themes WHERE id NOT IN (-1, -2))")
                .fetch_one(&mut *conn)
                .await?;
        if (theme == -1 || theme == -2) && !custom_themes {
            out.insert("can_run_design_wizard".into(), json!(true));
        }
    }
    out.insert(
        "user_option".into(),
        user_option(conn, settings, &row.new_since, row.created_at, uid).await?,
    );
    Ok(Value::Object(out))
}

/// `SpamRule::AutoSilence.should_autosilence?` for a new user.
async fn autosilence_pending(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    uid: i32,
) -> Result<bool, AppError> {
    let needed = settings.get("num_users_to_silence_new_user")?.to_i();
    if needed <= 0 {
        return Ok(false);
    }
    let (total, count): (Option<f64>, i64) = sqlx::query_as(
        "SELECT SUM(rs.score)::float8, COUNT(DISTINCT rs.user_id) FROM reviewables r \
         INNER JOIN reviewable_scores rs ON rs.reviewable_id = r.id \
         WHERE r.target_created_by_id = $1 AND rs.reviewable_score_type = 8 AND rs.status IN (0, 1)",
    )
    .bind(uid)
    .fetch_one(&mut *conn)
    .await?;
    if total.unwrap_or(0.0) > 0.0 && count >= needed {
        return Err(Unsupported("spam auto-silence scoring").into());
    }
    Ok(false)
}

/// `DiscourseTagging.visible_tags(guardian)` for a logged-in user; binds
/// `$1` = user id, `$2` = allowed category ids.
fn visible_tags_where_for_user() -> String {
    "(tags.id NOT IN (SELECT tgm.tag_id FROM tag_group_memberships tgm \
        INNER JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
        INNER JOIN tag_group_permissions tgp ON tgp.tag_group_id = tg.id) \
      OR tags.id IN (SELECT tgm.tag_id FROM tag_group_permissions tgp \
        INNER JOIN tag_groups tg ON tg.id = tgp.tag_group_id \
        INNER JOIN tag_group_memberships tgm ON tgm.tag_group_id = tg.id \
        WHERE tgp.group_id IN (SELECT 0 UNION SELECT group_id FROM group_users WHERE user_id = $1))) \
     AND (tags.id NOT IN (SELECT tag_id FROM category_tags \
          UNION SELECT tgm.tag_id FROM tag_group_memberships tgm \
          INNER JOIN category_tag_groups ctg ON ctg.tag_group_id = tgm.tag_group_id) \
      OR tags.id IN (SELECT tag_id FROM category_tags WHERE category_id = ANY($2) \
          UNION SELECT tgm.tag_id FROM tag_group_memberships tgm \
          INNER JOIN category_tag_groups ctg ON ctg.tag_group_id = tgm.tag_group_id AND ctg.category_id = ANY($2)))"
        .to_string()
}

/// `String#parameterize`
fn parameterize(s: &str) -> String {
    let mut out = String::new();
    let mut dash = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash {
            out.push('-');
            dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

/// What a member's sidebar reads from their current user: the fields of
/// `serialize` its sections and links ask for, and their tracking state.
pub async fn sidebar_member(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::GuardianUser,
    tracking: crate::topic_tracking_report::Tracking,
) -> Result<crate::sidebar::Member, AppError> {
    let g = UserGuardian::from_guardian(settings, guardian);
    let user = &guardian.user;
    let tagging = settings.get("tagging_enabled")?.truthy();
    let fields = sidebar_fields(conn, settings, &g, user, tagging).await?;
    let (draft_count, show_count, link_to_filtered_list): (i32, bool, bool) = sqlx::query_as(
        "SELECT COALESCE(us.draft_count, 0), COALESCE(uo.sidebar_show_count_of_new_items, false), \
                COALESCE(uo.sidebar_link_to_filtered_list, false) \
         FROM users u LEFT JOIN user_stats us ON us.user_id = u.id \
         LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = $1",
    )
    .bind(user.id)
    .fetch_one(&mut *conn)
    .await?;
    let reviewable_count = if g.is_staff() {
        if settings.get("enable_category_group_moderation")?.truthy() {
            return Err(Unsupported("enable_category_group_moderation").into());
        }
        crate::reviewables::staff_counts(&mut *conn, settings, user.id, user.admin, user.moderator)
            .await?
            .0
    } else {
        0
    };
    Ok(crate::sidebar::Member {
        username: user.username.clone(),
        admin: user.admin,
        staff: g.is_staff(),
        can_review: g.is_staff(),
        can_send_private_messages: can_send_private_messages(&g)?,
        can_invite_to_forum: can_invite_to_forum(&g, settings)?,
        draft_count: i64::from(draft_count),
        reviewable_count,
        show_count,
        unified_new: tracking.unified_new,
        link_to_filtered_list,
        tracking,
        fields,
    })
}

/// The serializer's sidebar fields: display_sidebar_tags and sidebar_tags
/// (with tagging), sidebar_category_ids and sidebar_sections.
pub async fn sidebar_fields(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    g: &UserGuardian<'_>,
    user: &SessionUser,
    tagging: bool,
) -> Result<Map<String, Value>, AppError> {
    let uid = user.id;
    let mut out = Map::new();
    let allowed = g.allowed_category_ids(conn).await?;
    if tagging {
        let browsable = if user.admin {
            "SELECT EXISTS (SELECT 1 FROM tags WHERE target_tag_id IS NULL)".to_string()
        } else {
            format!(
                "SELECT EXISTS (SELECT 1 FROM tags WHERE tags.target_tag_id IS NULL AND {})",
                visible_tags_where_for_user()
            )
        };
        let display: bool = sqlx::query_scalar(&browsable)
            .bind(uid)
            .bind(&allowed)
            .fetch_one(&mut *conn)
            .await?;
        out.insert("display_sidebar_tags".into(), json!(display));
        let count_column = if g.is_staff()
            || settings
                .get("include_secure_categories_in_tag_counts")?
                .truthy()
        {
            "staff_topic_count"
        } else {
            "public_topic_count"
        };
        let visible = if user.admin {
            "TRUE".to_string()
        } else {
            visible_tags_where_for_user()
        };
        let sidebar_tags: Vec<SidebarTag> = sqlx::query_as(&format!(
            "SELECT tags.id, tags.name, tags.slug, tags.description, (tags.{count_column} = 0 AND tags.pm_topic_count > 0) \
             FROM tags WHERE tags.target_tag_id IS NULL AND {visible} \
             AND tags.id IN (SELECT linkable_id FROM sidebar_section_links WHERE user_id = $1 AND linkable_type = 'Tag') \
             ORDER BY tags.{count_column} DESC"
        ))
        .bind(uid)
        .bind(&allowed)
        .fetch_all(&mut *conn)
        .await?;
        out.insert(
            "sidebar_tags".into(),
            json!(sidebar_tags
                .into_iter()
                .map(|(id, name, slug, description, pm_only)| {
                    json!({
                        "id": id, "name": name,
                        "slug": slug.filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{id}-tag")),
                        "description": description, "pm_only": pm_only,
                    })
                })
                .collect::<Vec<_>>()),
        );
    }
    let linked: Vec<i32> = sqlx::query_scalar(
        "SELECT linkable_id::int FROM sidebar_section_links WHERE user_id = $1 AND linkable_type = 'Category'",
    )
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    let sidebar_category_ids: Vec<i32> = linked
        .into_iter()
        .filter(|id| allowed.contains(id))
        .collect();
    out.insert(
        "sidebar_category_ids".into(),
        ids_json(&sidebar_category_ids),
    );
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("content_localization_enabled").into());
    }
    #[derive(sqlx::FromRow)]
    struct Section {
        id: i64,
        title: String,
        public: bool,
        section_type: Option<i32>,
        locale: Option<String>,
    }
    let sections: Vec<Section> = sqlx::query_as(
        "SELECT id, title, public, section_type, locale FROM sidebar_sections \
         WHERE (user_id = $1 OR public) ORDER BY (section_type IS NOT NULL) DESC, (public IS TRUE) DESC, id ASC",
    )
    .bind(uid)
    .fetch_all(&mut *conn)
    .await?;
    let mut sections_json = Vec::new();
    for s in sections {
        #[derive(sqlx::FromRow)]
        struct Link {
            id: i64,
            name: String,
            value: String,
            icon: String,
            external: bool,
            segment: i32,
            locale: Option<String>,
        }
        let links: Vec<Link> = sqlx::query_as(
            "SELECT sidebar_urls.id, sidebar_urls.name, sidebar_urls.value, sidebar_urls.icon, sidebar_urls.external, \
                    sidebar_urls.segment, sidebar_urls.locale FROM sidebar_urls \
             INNER JOIN sidebar_section_links ON sidebar_urls.id = sidebar_section_links.linkable_id \
             WHERE sidebar_section_links.sidebar_section_id = $1 AND sidebar_section_links.linkable_type = 'SidebarUrl' \
             ORDER BY sidebar_section_links.position",
        )
        .bind(s.id)
        .fetch_all(&mut *conn)
        .await?;
        sections_json.push(json!({
            "id": s.id,
            "title": s.title,
            "links": links.into_iter().map(|l| json!({
                "id": l.id, "name": l.name, "value": l.value, "icon": l.icon, "external": l.external,
                "segment": if l.segment == 0 { "primary" } else { "secondary" }, "locale": l.locale,
            })).collect::<Vec<_>>(),
            "slug": parameterize(&s.title),
            "public": s.public,
            "section_type": if s.section_type == Some(COMMUNITY_SECTION) { json!("community") } else { Value::Null },
            "locale": s.locale,
        }));
    }
    out.insert("sidebar_sections".into(), Value::Array(sections_json));
    Ok(out)
}

/// CurrentUserOptionSerializer
async fn user_option(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    new_since: &Option<chrono::NaiveDateTime>,
    created_at: chrono::NaiveDateTime,
    uid: i32,
) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Opt {
        mailing_list_mode: bool,
        external_links_in_new_tab: bool,
        enable_quoting: bool,
        enable_smart_lists: bool,
        enable_markdown_monospace_font: bool,
        dynamic_favicon: bool,
        automatically_unpin_topics: bool,
        like_notification_frequency: i32,
        hide_profile_and_presence: bool,
        hide_profile: bool,
        hide_presence: bool,
        title_count_mode_key: i32,
        timezone: Option<String>,
        skip_new_user_tips: bool,
        default_calendar: i32,
        bookmark_auto_delete_preference: i32,
        notify_on_linked_posts: bool,
        new_topic_duration_minutes: Option<i32>,
        sidebar_link_to_filtered_list: bool,
        sidebar_show_count_of_new_items: bool,
        composition_mode: i32,
        interface_color_mode: i32,
        send_shortcut: i32,
        automatically_translate: bool,
        understood_languages: Vec<String>,
        hidden_composer_toolbar_buttons: Vec<String>,
        previous_visit_at: Option<chrono::NaiveDateTime>,
        last_seen_at: Option<chrono::NaiveDateTime>,
        trust_level: i32,
    }
    let o: Opt = sqlx::query_as(
        "SELECT uo.mailing_list_mode, uo.external_links_in_new_tab, uo.enable_quoting, uo.enable_smart_lists, \
                uo.enable_markdown_monospace_font, uo.dynamic_favicon, uo.automatically_unpin_topics, \
                uo.like_notification_frequency, uo.hide_profile_and_presence, uo.hide_profile, uo.hide_presence, \
                uo.title_count_mode_key, uo.timezone, uo.skip_new_user_tips, uo.default_calendar, \
                uo.bookmark_auto_delete_preference, uo.notify_on_linked_posts, uo.new_topic_duration_minutes, \
                uo.sidebar_link_to_filtered_list, uo.sidebar_show_count_of_new_items, uo.composition_mode, \
                uo.interface_color_mode, uo.send_shortcut, uo.automatically_translate, \
                COALESCE(uo.understood_languages, '{}') AS understood_languages, \
                COALESCE(uo.hidden_composer_toolbar_buttons, '{}') AS hidden_composer_toolbar_buttons, \
                u.previous_visit_at, u.last_seen_at, u.trust_level \
         FROM user_options uo JOIN users u ON u.id = uo.user_id WHERE uo.user_id = $1",
    )
    .bind(uid)
    .fetch_one(&mut *conn)
    .await?;
    if settings.get("enable_user_tips")?.truthy() {
        return Err(Unsupported("enable_user_tips (seen_popups)").into());
    }
    // redirected_to_top: only reachable when the top menu has "top" and a
    // full page of top topics exists; refused rather than approximated.
    if settings.get("redirect_users_to_top_page")?.truthy()
        && settings
            .get("top_menu")?
            .to_s()
            .split('|')
            .any(|m| m == "top")
        && (o.trust_level == 0
            || o.last_seen_at
                .is_none_or(|t| t < crate::clock::now_naive() - chrono::Duration::days(30)))
    {
        let per_page = settings.get("topics_per_period_in_top_page")?.to_i();
        for period in ["daily", "weekly", "monthly", "quarterly", "yearly", "all"] {
            let count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM (SELECT 1 FROM top_topics WHERE {period}_score > 0 LIMIT $1) x"
            ))
            .bind(per_page)
            .fetch_one(&mut *conn)
            .await?;
            if count == per_page {
                return Err(Unsupported("redirected_to_top").into());
            }
        }
    }
    // treat_as_new_topic_start_date
    let duration = o.new_topic_duration_minutes.map(i64::from).unwrap_or(
        settings
            .get("default_other_new_topic_duration_minutes")?
            .to_i(),
    );
    let now = crate::clock::now_naive();
    let base = match duration {
        -1 => created_at,
        -2 => o.previous_visit_at.or(*new_since).unwrap_or(created_at),
        minutes => now - chrono::Duration::minutes(minutes),
    };
    let min_new = chrono::DateTime::from_timestamp(settings.get("min_new_topics_time")?.to_i(), 0)
        .map(|t| t.naive_utc())
        .unwrap_or(created_at);
    let start = base.max(created_at).max(min_new);
    let default_calendar = ["none_selected", "ics", "google", "outlook", "apple"]
        .get(o.default_calendar as usize)
        .copied()
        .unwrap_or("none_selected");
    Ok(json!({
        "mailing_list_mode": o.mailing_list_mode,
        "external_links_in_new_tab": o.external_links_in_new_tab,
        "enable_quoting": o.enable_quoting,
        "enable_smart_lists": o.enable_smart_lists,
        "enable_markdown_monospace_font": o.enable_markdown_monospace_font,
        "dynamic_favicon": o.dynamic_favicon,
        "automatically_unpin_topics": o.automatically_unpin_topics,
        "likes_notifications_disabled": o.like_notification_frequency == 3,
        "hide_profile_and_presence": o.hide_profile_and_presence,
        "hide_profile": o.hide_profile,
        "hide_presence": o.hide_presence,
        "title_count_mode": if o.title_count_mode_key == 1 { "contextual" } else { "notifications" },
        "timezone": o.timezone,
        "skip_new_user_tips": o.skip_new_user_tips,
        "default_calendar": default_calendar,
        "bookmark_auto_delete_preference": o.bookmark_auto_delete_preference,
        "notify_on_linked_posts": o.notify_on_linked_posts,
        "should_be_redirected_to_top": false,
        "treat_as_new_topic_start_date": crate::topic_list::time_json(start),
        "sidebar_link_to_filtered_list": o.sidebar_link_to_filtered_list,
        "sidebar_show_count_of_new_items": o.sidebar_show_count_of_new_items,
        "composition_mode": o.composition_mode,
        "interface_color_mode": o.interface_color_mode,
        "show_original_content": !o.automatically_translate,
        "send_shortcut": if o.send_shortcut == 1 { "meta_enter" } else { "enter" },
        "automatically_translate": o.automatically_translate,
        "understood_languages": o.understood_languages,
        "hidden_composer_toolbar_buttons": o.hidden_composer_toolbar_buttons,
    }))
}

/// `can_send_private_messages`
pub fn can_send_private_messages(g: &UserGuardian<'_>) -> Result<bool, AppError> {
    Ok(g.user.id <= 0 || g.in_setting_groups("personal_message_enabled_groups")?)
}

/// `can_invite_to_forum`
pub fn can_invite_to_forum(
    g: &UserGuardian<'_>,
    settings: &SiteSettings,
) -> Result<bool, AppError> {
    Ok(g.in_setting_groups("invite_allowed_groups")?
        && (settings.get("max_invites_per_day")?.to_i() > 0 || g.is_staff()))
}
