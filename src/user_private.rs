//! The parts of UserSerializer only the user themself and staff get:
//! emails, 2FA, the private attribute block (notification buckets,
//! usernames lists, API keys, passkeys, auth tokens, schedule, sidebar),
//! `group_users` and UserOptionSerializer. Called from `Users::show`.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::time_json;
use crate::users::{User, UsersError};

pub struct Private<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    /// The hashed `_t` token of the request, for `is_active` on tokens.
    pub auth_token: Option<&'a str>,
    pub i18n: &'a crate::i18n::I18n,
}

/// `UserNotificationSchedule::DEFAULT`
const DEFAULT_SCHEDULE: [(i32, i32); 7] = [(480, 1020); 7];

impl Private<'_> {
    /// `email`, `secondary_emails`, `unconfirmed_emails` for the viewer's
    /// own profile (staff only see a staged user's, not ported).
    pub async fn emails(
        &mut self,
        user: &User,
        out: &mut Map<String, Value>,
    ) -> Result<(), UsersError> {
        let me = self.guardian.is_me(user.id);
        if !me {
            if user.staged && self.guardian.is_staff() {
                return Err(Unsupported("staged users' emails for staff").into());
            }
            return Ok(());
        }
        let email: Option<String> = sqlx::query_scalar(
            "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\" = TRUE LIMIT 1",
        )
        .bind(user.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let secondary: Vec<String> = sqlx::query_scalar(
            "SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\" = FALSE ORDER BY id",
        )
        .bind(user.id)
        .fetch_all(&mut *self.conn)
        .await?;
        let unconfirmed: Vec<String> = sqlx::query_scalar(
            "SELECT new_email FROM email_change_requests WHERE user_id = $1 AND change_state <> 3 ORDER BY id",
        )
        .bind(user.id)
        .fetch_all(&mut *self.conn)
        .await?;
        out.insert("email".into(), json!(email));
        out.insert("secondary_emails".into(), json!(secondary));
        out.insert("unconfirmed_emails".into(), json!(unconfirmed));
        Ok(())
    }

    /// `pending_posts_count` (self or staff).
    pub async fn pending_posts_count(&mut self, user_id: i32) -> Result<i64, UsersError> {
        let count: Option<i32> =
            sqlx::query_scalar("SELECT pending_posts_count FROM user_stats WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&mut *self.conn)
                .await?;
        Ok(i64::from(count.unwrap_or(0)))
    }

    /// `has_title_badges`
    pub async fn has_title_badges(&mut self, user_id: i32) -> Result<bool, UsersError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM badges INNER JOIN user_badges ON badges.id = user_badges.badge_id \
             WHERE user_badges.user_id = $1 AND user_badges.badge_id IN (SELECT id FROM badges WHERE enabled) \
             AND badges.allow_title = TRUE)",
        )
        .bind(user_id)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// `second_factor_enabled`, and for the user themself the backup-code
    /// keys and `associated_accounts`.
    pub async fn second_factor(
        &mut self,
        user: &User,
        out: &mut Map<String, Value>,
    ) -> Result<(), UsersError> {
        let me = self.guardian.is_me(user.id);
        let local = !self.settings.get("enable_discourse_connect")?.truthy()
            && self.settings.get("enable_local_logins")?.truthy();
        if me || self.guardian.is_admin() {
            let enabled: bool = sqlx::query_scalar(
                "SELECT $2::bool AND (EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND method = 1 AND enabled) \
                 OR EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0))",
            )
            .bind(user.id)
            .bind(local)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("second_factor_enabled".into(), json!(enabled));
        }
        if me {
            let backup: bool = sqlx::query_scalar(
                "SELECT $2::bool AND EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND method = 2 AND enabled)",
            )
            .bind(user.id)
            .bind(local)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("second_factor_backup_enabled".into(), json!(backup));
            if backup {
                return Err(Unsupported("second_factor_remaining_backup_codes").into());
            }
            let accounts: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM user_associated_accounts WHERE user_id = $1",
            )
            .bind(user.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if accounts > 0 {
                return Err(
                    Unsupported("associated_accounts (per-authenticator descriptions)").into(),
                );
            }
            out.insert("associated_accounts".into(), json!([]));
        }
        Ok(())
    }

    /// `no_password` (self or staff, only when there is none) and
    /// `show_mcp_authorizations` (self).
    pub async fn password_and_mcp(
        &mut self,
        user: &User,
        out: &mut Map<String, Value>,
    ) -> Result<(), UsersError> {
        let me = self.guardian.is_me(user.id);
        if me || self.guardian.is_staff() {
            let has_password: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_passwords WHERE user_id = $1)",
            )
            .bind(user.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if !has_password {
                out.insert("no_password".into(), json!(true));
            }
        }
        if me {
            if self
                .settings
                .get("mcp_server_enabled")
                .map(|v| v.truthy())
                .unwrap_or(false)
            {
                return Err(Unsupported("show_mcp_authorizations with the MCP server on").into());
            }
            let any: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM mcp_oauth_authorizations WHERE user_id = $1)",
            )
            .bind(user.id)
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("show_mcp_authorizations".into(), json!(any));
        }
        Ok(())
    }

    /// The staff attributes: `post_count`, `topic_count`, `can_be_deleted`,
    /// `can_delete_all_posts`.
    pub async fn staff_attributes(
        &mut self,
        user: &User,
        out: &mut Map<String, Value>,
    ) -> Result<(), UsersError> {
        let (post_count, topic_count, first_post_created_at): (i32, i32, Option<NaiveDateTime>) =
            sqlx::query_as(
                "SELECT post_count, topic_count, first_post_created_at FROM user_stats WHERE user_id = $1",
            )
            .bind(user.id)
            .fetch_optional(&mut *self.conn)
            .await?
            .unwrap_or((0, 0, None));
        out.insert("post_count".into(), json!(post_count));
        out.insert("topic_count".into(), json!(topic_count));
        let g = self.guardian;
        let s = self.settings;
        let now = chrono::Utc::now().naive_utc();
        let max_age = s.get("delete_user_max_post_age")?.to_i();
        let age_ok =
            |t: Option<NaiveDateTime>| t.is_none_or(|t| t >= now - chrono::Duration::days(max_age));
        // can_delete_user?
        let can_be_deleted = if user.admin {
            false
        } else if g.is_me(user.id) {
            if s.get("enable_discourse_connect")?.truthy() {
                false
            } else {
                !self
                    .has_more_posts_than(
                        user.id,
                        post_count,
                        topic_count,
                        s.get("delete_user_self_max_post_count")?.to_i(),
                    )
                    .await?
            }
        } else if !g.is_staff() || (user.moderator && !g.is_admin()) {
            false
        } else if first_post_created_at.is_none()
            || !self
                .has_more_posts_than(user.id, post_count, topic_count, 5)
                .await?
        {
            true
        } else {
            age_ok(first_post_created_at)
        };
        out.insert("can_be_deleted".into(), json!(can_be_deleted));
        // can_delete_all_posts?
        let can_delete_all = g.is_staff()
            && !user.admin
            && (!user.moderator || g.is_admin())
            && (g.is_admin()
                || (age_ok(first_post_created_at)
                    && i64::from(post_count) <= s.get("delete_all_posts_max")?.to_i()));
        out.insert("can_delete_all_posts".into(), json!(can_delete_all));
        Ok(())
    }

    /// `User#has_more_posts_than?(max)`
    async fn has_more_posts_than(
        &mut self,
        user_id: i32,
        post_count: i32,
        topic_count: i32,
        max: i64,
    ) -> Result<bool, UsersError> {
        if max < 0 || i64::from(topic_count + post_count) > max {
            return Ok(true);
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT 1 FROM posts p JOIN topics t ON t.id = p.topic_id \
             WHERE p.user_id = $1 AND p.deleted_at IS NULL AND t.deleted_at IS NULL LIMIT $2) x",
        )
        .bind(user_id)
        .bind(max + 1)
        .fetch_one(&mut *self.conn)
        .await?;
        Ok(count > max)
    }

    /// The private attribute block, `locale` through
    /// `can_pick_theme_with_custom_homepage`, in declaration order.
    pub async fn private_block(
        &mut self,
        user: &User,
        system_avatar_template: &str,
        out: &mut Map<String, Value>,
    ) -> Result<(), UsersError> {
        let uid = user.id;
        let s = self.settings;
        let g = self.guardian;
        let locale: Option<String> = sqlx::query_scalar("SELECT locale FROM users WHERE id = $1")
            .bind(uid)
            .fetch_one(&mut *self.conn)
            .await?;
        out.insert("locale".into(), json!(locale));

        // CategoryUser.notification_levels_for(target)
        let levels: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT category_id, notification_level FROM category_users WHERE user_id = $1 ORDER BY id",
        )
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        let with_level = |level: i32| -> Vec<i32> {
            levels
                .iter()
                .filter(|(_, l)| *l == level)
                .map(|(id, _)| *id)
                .collect()
        };
        out.insert("muted_category_ids".into(), json!(with_level(0)));
        out.insert("regular_category_ids".into(), json!(with_level(1)));

        // TagUser.notification_levels_for(target): the user's tag_users
        // rows, visible-tag filtered.
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
        .fetch_all(&mut *self.conn)
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
        out.insert("tracked_category_ids".into(), json!(with_level(2)));
        out.insert("watched_category_ids".into(), json!(with_level(3)));
        out.insert(
            "watched_first_post_category_ids".into(),
            json!(with_level(4)),
        );
        out.insert("system_avatar_upload_id".into(), Value::Null);
        out.insert(
            "system_avatar_template".into(),
            json!(system_avatar_template),
        );
        let avatars: Option<(Option<i32>, Option<i32>)> = sqlx::query_as(
            "SELECT gravatar_upload_id, custom_upload_id FROM user_avatars WHERE user_id = $1 LIMIT 1",
        )
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?;
        if let Some((gravatar, custom)) = avatars {
            if gravatar.is_some() || custom.is_some() {
                return Err(Unsupported("gravatar/custom avatar keys on profiles").into());
            }
        }
        let names = |table: &'static str, column: &'static str| {
            format!(
                "SELECT username FROM {table} INNER JOIN users ON users.id = {table}.{column} \
                 WHERE {table}.user_id = $1 ORDER BY {table}.id"
            )
        };
        let muted: Vec<String> = sqlx::query_scalar(&names("muted_users", "muted_user_id"))
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
        out.insert("muted_usernames".into(), json!(muted));
        out.insert("can_mute_users".into(), json!(g.can_mute_users()));
        let ignored: Vec<String> = sqlx::query_scalar(&names("ignored_users", "ignored_user_id"))
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
        out.insert("ignored_usernames".into(), json!(ignored));
        out.insert("can_ignore_users".into(), json!(g.can_ignore_users(s)?));
        let allowed: Vec<String> =
            sqlx::query_scalar(&names("allowed_pm_users", "allowed_pm_user_id"))
                .bind(uid)
                .fetch_all(&mut *self.conn)
                .await?;
        out.insert("allowed_pm_usernames".into(), json!(allowed));
        // mailing_list_posts_per_day: min(estimate, max_emails_per_day_per_user)
        let posts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM topics INNER JOIN posts ON posts.deleted_at IS NULL AND posts.topic_id = topics.id \
             WHERE topics.deleted_at IS NULL AND topics.archetype <> 'private_message' \
             AND (topics.category_id IS NULL OR topics.category_id IN (SELECT id FROM categories WHERE NOT read_restricted)) \
             AND posts.created_at > now() - interval '30 days'",
        )
        .fetch_one(&mut *self.conn)
        .await?;
        out.insert(
            "mailing_list_posts_per_day".into(),
            json!((posts / 30).min(s.get("max_emails_per_day_per_user")?.to_i())),
        );
        let sso = s.get("enable_discourse_connect")?.truthy();
        for (key, setting) in [
            ("can_change_bio", "discourse_connect_overrides_bio"),
            (
                "can_change_location",
                "discourse_connect_overrides_location",
            ),
            ("can_change_website", "discourse_connect_overrides_website"),
        ] {
            out.insert(key.into(), json!(!(sso && s.get(setting)?.truthy())));
        }
        // can_change_tracking_preferences?: can_edit holds here.
        out.insert(
            "can_change_tracking_preferences".into(),
            json!(s.get("allow_changing_staged_user_tracking")?.truthy() || !user.staged),
        );
        let api_keys: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM user_api_keys WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(uid)
        .fetch_one(&mut *self.conn)
        .await?;
        if api_keys > 0 {
            return Err(Unsupported("user_api_keys (scope translations)").into());
        }
        out.insert("user_api_keys".into(), Value::Null);
        if g.is_me(uid) && s.get("enable_passkeys")?.truthy() {
            let keys: Vec<(i64, Option<String>, Option<NaiveDateTime>, NaiveDateTime)> =
                sqlx::query_as(
                    "SELECT id, name, last_used, created_at FROM user_security_keys \
                 WHERE user_id = $1 AND factor_type = 1 ORDER BY created_at ASC",
                )
                .bind(uid)
                .fetch_all(&mut *self.conn)
                .await?;
            out.insert(
                "user_passkeys".into(),
                json!(
                    keys.into_iter()
                        .map(|(id, name, last_used, created_at)| json!({
                            "id": id, "name": name,
                            "last_used": last_used.map(time_json),
                            "created_at": time_json(created_at),
                        }))
                        .collect::<Vec<_>>()
                ),
            );
        }
        out.insert("user_auth_tokens".into(), self.auth_tokens(uid).await?);
        out.insert(
            "user_notification_schedule".into(),
            self.notification_schedule(uid).await?,
        );
        // use_logo_small_as_avatar: system user only.
        out.insert(
            "use_logo_small_as_avatar".into(),
            json!(
                uid == -1
                    && s.get("logo_small")?.presence().is_some()
                    && s.get("use_site_small_logo_as_system_avatar")?.truthy()
            ),
        );
        self.sidebar(uid, out).await?;
        let custom_homepage: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM theme_modifier_sets tms JOIN themes t ON t.id = tms.theme_id \
             WHERE tms.custom_homepage = TRUE AND t.enabled)",
        )
        .fetch_one(&mut *self.conn)
        .await?;
        out.insert(
            "can_pick_theme_with_custom_homepage".into(),
            json!(custom_homepage),
        );
        Ok(())
    }

    /// `sidebar_tags`, `sidebar_category_ids`, `display_sidebar_tags`:
    /// the target's sidebar links as the viewer may see them.
    async fn sidebar(&mut self, uid: i32, out: &mut Map<String, Value>) -> Result<(), UsersError> {
        let s = self.settings;
        let g = self.guardian;
        let tagging = s.get("tagging_enabled")?.truthy();
        let allowed = g.allowed_category_ids(&mut *self.conn, s).await?;
        if tagging {
            let count_column = g.tag_count_column(s)?;
            let visible = crate::tags::visible_tags_where(g, s)?;
            #[derive(sqlx::FromRow)]
            struct SidebarTag {
                id: i32,
                name: String,
                slug: Option<String>,
                description: Option<String>,
                pm_only: bool,
            }
            let tags: Vec<SidebarTag> = sqlx::query_as(&format!(
                "SELECT tags.id, tags.name, tags.slug, tags.description, \
                        (tags.{count_column} = 0 AND tags.pm_topic_count > 0) AS pm_only \
                 FROM tags WHERE tags.target_tag_id IS NULL AND {visible} \
                 AND tags.id IN (SELECT linkable_id FROM sidebar_section_links WHERE user_id = $1 AND linkable_type = 'Tag') \
                 ORDER BY tags.{count_column} DESC"
            ))
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
            out.insert(
                "sidebar_tags".into(),
                json!(tags
                    .into_iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "pm_only": t.pm_only,
                        "id": t.id,
                        "slug": t.slug.filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{}-tag", t.id)),
                    }))
                    .collect::<Vec<_>>()),
            );
        }
        let links: Vec<i32> = sqlx::query_scalar(
            // Rails plucks without ORDER BY; the reference comes back by id.
            "SELECT linkable_id::int FROM sidebar_section_links WHERE user_id = $1 AND linkable_type = 'Category' ORDER BY linkable_id",
        )
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        out.insert(
            "sidebar_category_ids".into(),
            json!(
                links
                    .into_iter()
                    .filter(|id| allowed.contains(id))
                    .collect::<Vec<_>>()
            ),
        );
        if tagging {
            let visible = crate::tags::visible_tags_where(g, s)?;
            let display: bool = sqlx::query_scalar(&format!(
                "SELECT EXISTS (SELECT 1 FROM tags WHERE tags.target_tag_id IS NULL AND {visible})"
            ))
            .fetch_one(&mut *self.conn)
            .await?;
            out.insert("display_sidebar_tags".into(), json!(display));
        }
        Ok(())
    }

    /// UserAuthTokenSerializer for every token of the user.
    async fn auth_tokens(&mut self, uid: i32) -> Result<Value, UsersError> {
        #[derive(sqlx::FromRow)]
        struct Token {
            id: i32,
            client_ip: Option<String>,
            user_agent: Option<String>,
            created_at: NaiveDateTime,
            seen_at: Option<NaiveDateTime>,
            auth_token: String,
        }
        let tokens: Vec<Token> = sqlx::query_as(
            "SELECT id, host(client_ip) AS client_ip, user_agent, created_at, seen_at, auth_token \
             FROM user_auth_tokens WHERE user_id = $1 ORDER BY id",
        )
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        let g = self.guardian;
        let can_see_ip = g.is_admin()
            || (g.is_moderator() && self.settings.get("moderators_view_ips")?.truthy())
            || g.is_me(uid);
        let i18n = self.i18n;
        let mut out = Vec::with_capacity(tokens.len());
        for t in tokens {
            let ua = t.user_agent.as_deref().unwrap_or("");
            let (browser, device, os) = browser_detection(ua);
            let mut j = Map::new();
            j.insert("id".into(), json!(t.id));
            if can_see_ip {
                j.insert("client_ip".into(), json!(t.client_ip.unwrap_or_default()));
            }
            // DiscourseIpInfo needs a MaxMind database; without one Rails
            // says "unknown".
            j.insert(
                "location".into(),
                json!(i18n.t("staff_action_logs.unknown").unwrap_or("unknown")),
            );
            j.insert(
                "browser".into(),
                json!(i18n.t(&format!("browsers.{browser}")).unwrap_or(browser)),
            );
            j.insert(
                "device".into(),
                json!(
                    i18n.t(&format!("user_auth_tokens.device.{device}"))
                        .unwrap_or(device)
                ),
            );
            j.insert(
                "os".into(),
                json!(i18n.t(&format!("user_auth_tokens.os.{os}")).unwrap_or(os)),
            );
            j.insert(
                "icon".into(),
                json!(match os {
                    "android" => "fab-android",
                    "chromeos" => "fab-chrome",
                    "macos" | "ios" => "fab-apple",
                    "linux" => "fab-linux",
                    "windows" => "fab-windows",
                    _ => "question",
                }),
            );
            j.insert("created_at".into(), json!(time_json(t.created_at)));
            j.insert(
                "seen_at".into(),
                json!(time_json(t.seen_at.unwrap_or(t.created_at))),
            );
            j.insert(
                "is_active".into(),
                json!(self.auth_token == Some(t.auth_token.as_str())),
            );
            out.push(Value::Object(j));
        }
        Ok(Value::Array(out))
    }

    /// UserNotificationScheduleSerializer, or the DEFAULT hash.
    async fn notification_schedule(&mut self, uid: i32) -> Result<Value, UsersError> {
        let row: Option<(i64, bool, Vec<i32>)> = sqlx::query_as(
            "SELECT id, enabled, ARRAY[day_0_start_time, day_0_end_time, day_1_start_time, day_1_end_time, \
                    day_2_start_time, day_2_end_time, day_3_start_time, day_3_end_time, day_4_start_time, \
                    day_4_end_time, day_5_start_time, day_5_end_time, day_6_start_time, day_6_end_time] \
             FROM user_notification_schedules WHERE user_id = $1 LIMIT 1",
        )
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?;
        let mut out = Map::new();
        match row {
            Some((id, enabled, times)) => {
                out.insert("id".into(), json!(id));
                out.insert("user_id".into(), json!(uid));
                out.insert("enabled".into(), json!(enabled));
                for day in 0..7 {
                    out.insert(format!("day_{day}_start_time"), json!(times[day * 2]));
                    out.insert(format!("day_{day}_end_time"), json!(times[day * 2 + 1]));
                }
            }
            None => {
                out.insert("enabled".into(), json!(false));
                for (day, (start, end)) in DEFAULT_SCHEDULE.iter().enumerate() {
                    out.insert(format!("day_{day}_start_time"), json!(start));
                    out.insert(format!("day_{day}_end_time"), json!(end));
                }
            }
        }
        Ok(Value::Object(out))
    }

    /// `group_users`: BasicGroupUserSerializer rows, `owner` for the user
    /// themself only.
    pub async fn group_users(&mut self, uid: i32) -> Result<Value, UsersError> {
        let rows: Vec<(i32, i32, bool)> = sqlx::query_as(
            "SELECT group_id, notification_level, owner FROM group_users WHERE user_id = $1 ORDER BY group_id",
        )
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        let me = self.guardian.is_me(uid);
        Ok(json!(
            rows.into_iter()
                .map(|(group_id, level, owner)| {
                    let mut j =
                        json!({"group_id": group_id, "user_id": uid, "notification_level": level});
                    if me {
                        j["owner"] = json!(owner);
                    }
                    j
                })
                .collect::<Vec<_>>()
        ))
    }

    /// UserOptionSerializer, core keys in declaration order (plugin keys
    /// are the plugins' business).
    pub async fn user_option(&mut self, uid: i32) -> Result<Value, UsersError> {
        #[derive(sqlx::FromRow)]
        struct Opt {
            mailing_list_mode: bool,
            mailing_list_mode_frequency: i32,
            email_digests: Option<bool>,
            email_level: i32,
            email_messages_level: i32,
            external_links_in_new_tab: bool,
            color_scheme_id: Option<i32>,
            dark_scheme_id: Option<i32>,
            dynamic_favicon: bool,
            enable_quoting: bool,
            enable_smart_lists: bool,
            enable_markdown_monospace_font: bool,
            digest_after_minutes: Option<i32>,
            automatically_unpin_topics: bool,
            auto_track_topics_after_msecs: Option<i32>,
            notification_level_when_replying: Option<i32>,
            new_topic_duration_minutes: Option<i32>,
            email_previous_replies: i32,
            email_in_reply_to: bool,
            like_notification_frequency: i32,
            notify_on_linked_posts: bool,
            push_notification_level: i32,
            enable_upcoming_change_available_notifications: bool,
            theme_ids: Vec<i32>,
            theme_key_seq: i32,
            allow_private_messages: bool,
            enable_allowed_pm_users: bool,
            homepage_id: Option<i32>,
            hide_profile_and_presence: bool,
            hide_profile: bool,
            hide_presence: bool,
            text_size_key: i32,
            text_size_seq: i32,
            title_count_mode_key: i32,
            bookmark_auto_delete_preference: i32,
            timezone: Option<String>,
            skip_new_user_tips: bool,
            default_calendar: i32,
            oldest_search_log_date: Option<NaiveDateTime>,
            seen_popups: Option<Vec<i32>>,
            sidebar_link_to_filtered_list: bool,
            sidebar_show_count_of_new_items: bool,
            watched_precedence_over_muted: bool,
            composition_mode: i32,
            interface_color_mode: i32,
            send_shortcut: i32,
            automatically_translate: bool,
            understood_languages: Vec<String>,
            hidden_composer_toolbar_buttons: Vec<String>,
        }
        let o: Opt = sqlx::query_as(
            "SELECT mailing_list_mode, mailing_list_mode_frequency, email_digests, email_level, email_messages_level, \
                    external_links_in_new_tab, color_scheme_id, dark_scheme_id, dynamic_favicon, enable_quoting, \
                    enable_smart_lists, enable_markdown_monospace_font, digest_after_minutes, automatically_unpin_topics, \
                    auto_track_topics_after_msecs, notification_level_when_replying, new_topic_duration_minutes, \
                    email_previous_replies, email_in_reply_to, like_notification_frequency, notify_on_linked_posts, \
                    push_notification_level, enable_upcoming_change_available_notifications, \
                    COALESCE(theme_ids, '{}') AS theme_ids, theme_key_seq, allow_private_messages, \
                    enable_allowed_pm_users, homepage_id, hide_profile_and_presence, hide_profile, hide_presence, \
                    text_size_key, text_size_seq, title_count_mode_key, bookmark_auto_delete_preference, timezone, \
                    skip_new_user_tips, default_calendar, oldest_search_log_date, seen_popups, \
                    sidebar_link_to_filtered_list, sidebar_show_count_of_new_items, watched_precedence_over_muted, \
                    composition_mode, interface_color_mode, send_shortcut, automatically_translate, \
                    COALESCE(understood_languages, '{}') AS understood_languages, \
                    COALESCE(hidden_composer_toolbar_buttons, '{}') AS hidden_composer_toolbar_buttons \
             FROM user_options WHERE user_id = $1",
        )
        .bind(uid)
        .fetch_one(&mut *self.conn)
        .await?;
        let s = self.settings;
        fn name(table: &[&'static str], key: i32) -> &'static str {
            table.get(key as usize).copied().unwrap_or(table[0])
        }
        let theme_ids = if o.theme_ids.is_empty() {
            vec![s.get("default_theme_id")?.to_i() as i32]
        } else {
            o.theme_ids.clone()
        };
        let mut o_out = Map::new();
        o_out.insert("user_id".into(), json!(uid));
        o_out.insert(
            "mailing_list_mode".into(),
            json!(o.mailing_list_mode && !s.get("disable_mailing_list_mode")?.truthy()),
        );
        o_out.insert(
            "mailing_list_mode_frequency".into(),
            json!(o.mailing_list_mode_frequency),
        );
        o_out.insert("email_digests".into(), json!(o.email_digests));
        o_out.insert("email_level".into(), json!(o.email_level));
        o_out.insert("email_messages_level".into(), json!(o.email_messages_level));
        o_out.insert(
            "external_links_in_new_tab".into(),
            json!(o.external_links_in_new_tab),
        );
        o_out.insert("color_scheme_id".into(), json!(o.color_scheme_id));
        o_out.insert("dark_scheme_id".into(), json!(o.dark_scheme_id));
        o_out.insert("dynamic_favicon".into(), json!(o.dynamic_favicon));
        o_out.insert("enable_quoting".into(), json!(o.enable_quoting));
        o_out.insert("enable_smart_lists".into(), json!(o.enable_smart_lists));
        o_out.insert(
            "enable_markdown_monospace_font".into(),
            json!(o.enable_markdown_monospace_font),
        );
        o_out.insert("digest_after_minutes".into(), json!(o.digest_after_minutes));
        o_out.insert(
            "automatically_unpin_topics".into(),
            json!(o.automatically_unpin_topics),
        );
        o_out.insert(
            "auto_track_topics_after_msecs".into(),
            json!(
                o.auto_track_topics_after_msecs
                    .map(i64::from)
                    .unwrap_or(s.get("default_other_auto_track_topics_after_msecs")?.to_i())
            ),
        );
        o_out.insert(
            "notification_level_when_replying".into(),
            json!(
                o.notification_level_when_replying.map(i64::from).unwrap_or(
                    s.get("default_other_notification_level_when_replying")?
                        .to_i()
                )
            ),
        );
        o_out.insert(
            "new_topic_duration_minutes".into(),
            json!(
                o.new_topic_duration_minutes
                    .map(i64::from)
                    .unwrap_or(s.get("default_other_new_topic_duration_minutes")?.to_i())
            ),
        );
        o_out.insert(
            "email_previous_replies".into(),
            json!(o.email_previous_replies),
        );
        o_out.insert("email_in_reply_to".into(), json!(o.email_in_reply_to));
        o_out.insert(
            "like_notification_frequency".into(),
            json!(o.like_notification_frequency),
        );
        o_out.insert(
            "notify_on_linked_posts".into(),
            json!(o.notify_on_linked_posts),
        );
        o_out.insert(
            "push_notification_level".into(),
            json!(name(
                &["none", "all", "chat_only"],
                o.push_notification_level
            )),
        );
        o_out.insert(
            "enable_upcoming_change_available_notifications".into(),
            json!(o.enable_upcoming_change_available_notifications),
        );
        o_out.insert("include_tl0_in_digests".into(), json!(false));
        o_out.insert("theme_ids".into(), json!(theme_ids));
        o_out.insert("theme_key_seq".into(), json!(o.theme_key_seq));
        o_out.insert(
            "allow_private_messages".into(),
            json!(o.allow_private_messages),
        );
        o_out.insert(
            "enable_allowed_pm_users".into(),
            json!(o.enable_allowed_pm_users),
        );
        o_out.insert("homepage_id".into(), json!(o.homepage_id));
        o_out.insert(
            "hide_profile_and_presence".into(),
            json!(o.hide_profile_and_presence),
        );
        o_out.insert("hide_profile".into(), json!(o.hide_profile));
        o_out.insert("hide_presence".into(), json!(o.hide_presence));
        o_out.insert(
            "text_size".into(),
            json!(match o.text_size_key {
                4 => "smallest",
                3 => "smaller",
                1 => "larger",
                2 => "largest",
                _ => "normal",
            }),
        );
        o_out.insert("text_size_seq".into(), json!(o.text_size_seq));
        o_out.insert(
            "title_count_mode".into(),
            json!(name(
                &["notifications", "contextual"],
                o.title_count_mode_key
            )),
        );
        o_out.insert(
            "bookmark_auto_delete_preference".into(),
            json!(o.bookmark_auto_delete_preference),
        );
        o_out.insert("timezone".into(), json!(o.timezone));
        o_out.insert("skip_new_user_tips".into(), json!(o.skip_new_user_tips));
        o_out.insert(
            "default_calendar".into(),
            json!(name(
                &["none_selected", "ics", "google", "outlook", "apple"],
                o.default_calendar
            )),
        );
        o_out.insert(
            "oldest_search_log_date".into(),
            json!(o.oldest_search_log_date.map(time_json)),
        );
        o_out.insert("seen_popups".into(), json!(o.seen_popups));
        o_out.insert(
            "sidebar_link_to_filtered_list".into(),
            json!(o.sidebar_link_to_filtered_list),
        );
        o_out.insert(
            "sidebar_show_count_of_new_items".into(),
            json!(o.sidebar_show_count_of_new_items),
        );
        o_out.insert(
            "watched_precedence_over_muted".into(),
            json!(o.watched_precedence_over_muted),
        );
        o_out.insert("composition_mode".into(), json!(o.composition_mode));
        o_out.insert("interface_color_mode".into(), json!(o.interface_color_mode));
        o_out.insert(
            "show_original_content".into(),
            json!(!o.automatically_translate),
        );
        o_out.insert(
            "send_shortcut".into(),
            json!(name(&["enter", "meta_enter"], o.send_shortcut)),
        );
        o_out.insert(
            "automatically_translate".into(),
            json!(o.automatically_translate),
        );
        o_out.insert("understood_languages".into(), json!(o.understood_languages));
        o_out.insert(
            "hidden_composer_toolbar_buttons".into(),
            json!(o.hidden_composer_toolbar_buttons),
        );
        Ok(Value::Object(o_out))
    }
}

/// `BrowserDetection.browser/device/os` keys, first match wins.
fn browser_detection(ua: &str) -> (&'static str, &'static str, &'static str) {
    let has = |needles: &[&str]| {
        needles
            .iter()
            .any(|n| ua.to_lowercase().contains(&n.to_lowercase()))
    };
    let browser = if has(&["Edg"]) {
        "edge"
    } else if has(&["Opera", "OPR"]) {
        "opera"
    } else if has(&["SamsungBrowser/"]) {
        "samsung_browser"
    } else if has(&["UCBrowser/", "UCBrowser ", "UC Browser/", "UC Browser "]) {
        "uc_browser"
    } else if has(&["MQQBrowser/", "QQBrowser/"]) {
        "qq_browser"
    } else if has(&["BIDUBrowser/", "BaiduBrowser/"]) {
        "baidu_browser"
    } else if has(&["KaiOS/"]) {
        "kaios_browser"
    } else if has(&["MSIE", "Trident", "IEMobile"]) {
        "ie"
    } else if has(&["Firefox", "FxiOS"]) {
        "firefox"
    } else if has(&["Chrome", "CriOS"]) {
        "chrome"
    } else if has(&["Android"]) && has(&["Version/"]) && has(&["Safari/"]) {
        "android_browser"
    } else if has(&["Safari"]) {
        "safari"
    } else if has(&["Discourse"]) {
        "discoursehub"
    } else {
        "unknown"
    };
    let device = if has(&["Android"]) {
        "android"
    } else if has(&["CrOS"]) {
        "chromebook"
    } else if has(&["iPad"]) {
        "ipad"
    } else if has(&["iPhone"]) {
        "iphone"
    } else if has(&["iPod"]) {
        "ipod"
    } else if has(&["Mobile"]) {
        "mobile"
    } else if has(&["Macintosh"]) {
        "mac"
    } else if has(&["Linux"]) {
        "linux"
    } else if has(&["Windows"]) {
        "windows"
    } else {
        "unknown"
    };
    let os = if has(&["Android"]) {
        "android"
    } else if has(&["CrOS"]) {
        "chromeos"
    } else if has(&["iPhone", "iPad", "iPod", "Darwin"]) {
        "ios"
    } else if has(&["Macintosh"]) {
        "macos"
    } else if has(&["Linux"]) {
        "linux"
    } else if has(&["Windows"]) {
        "windows"
    } else {
        "unknown"
    };
    (browser, device, os)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_detection_matches_rails_order() {
        assert_eq!(browser_detection(""), ("unknown", "unknown", "unknown"));
        assert_eq!(
            browser_detection(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36"
            ),
            ("chrome", "linux", "linux")
        );
        assert_eq!(
            browser_detection(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
            ),
            ("safari", "mac", "macos")
        );
        assert_eq!(
            browser_detection(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0 Safari/537.36 Edg/120.0"
            ),
            ("edge", "windows", "windows")
        );
        assert_eq!(
            browser_detection(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 Version/17.0 Mobile/15E148 Safari/604.1"
            ),
            ("safari", "iphone", "ios")
        );
    }
}
