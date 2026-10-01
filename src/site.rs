//! Port of app/models/site.rb (Site.json_for) and
//! app/serializers/site_serializer.rb for anonymous users.
//!
//! Attributes are emitted in the serializer's declaration order. Plugin
//! contributions (topic-voting's `votes` filter, chat's hashtag and markdown
//! entries, category custom fields) are not ported; those keys are listed
//! with `ignore=` in parity/cases until a plugin layer exists. Categories are
//! a slice of their own and are left out too.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::categories::{Categories, CategoriesError};
use crate::color_scheme::ColorScheme;
use crate::config::Config;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{Definitions, SettingError, SiteSettings};
use crate::tags::visible_tag_ids_subquery;
use crate::url::UrlError;

/// `Archetype.default`
const DEFAULT_ARCHETYPE: &str = "regular";

/// `Notification.types` (app/models/notification.rb), plugin-reserved ids
/// included since they're declared in core.
const NOTIFICATION_TYPES: &[(&str, i64)] = &[
    ("mentioned", 1),
    ("replied", 2),
    ("quoted", 3),
    ("edited", 4),
    ("liked", 5),
    ("private_message", 6),
    ("invited_to_private_message", 7),
    ("invitee_accepted", 8),
    ("posted", 9),
    ("moved_post", 10),
    ("linked", 11),
    ("granted_badge", 12),
    ("invited_to_topic", 13),
    ("custom", 14),
    ("group_mentioned", 15),
    ("group_message_summary", 16),
    ("watching_first_post", 17),
    ("topic_reminder", 18),
    ("liked_consolidated", 19),
    ("post_approved", 20),
    ("code_review_commit_approved", 21),
    ("membership_request_accepted", 22),
    ("membership_request_consolidated", 23),
    ("bookmark_reminder", 24),
    ("reaction", 25),
    ("votes_released", 26),
    ("event_reminder", 27),
    ("event_invitation", 28),
    ("chat_mention", 29),
    ("chat_message", 30),
    ("chat_invitation", 31),
    ("chat_group_mention", 32),
    ("chat_quoted", 33),
    ("assigned", 34),
    ("question_answer_user_commented", 35),
    ("watching_category_or_tag", 36),
    ("new_features", 37),
    ("admin_problems", 38),
    ("linked_consolidated", 39),
    ("chat_watched_thread", 40),
    ("upcoming_change_available", 41),
    ("upcoming_change_automatically_promoted", 42),
    ("boost", 43),
    ("suggested_edit_created", 44),
    ("suggested_edit_accepted", 45),
    ("following", 800),
    ("following_created_topic", 801),
    ("following_replied", 802),
    ("circles_activity", 900),
    ("voice_invitation", 1000),
];

/// `Post.types`
const POST_TYPES: &[(&str, i64)] = &[
    ("regular", 1),
    ("moderator_action", 2),
    ("small_action", 3),
    ("whisper", 4),
];

/// `User.user_tips`
const USER_TIPS: &[(&str, i64)] = &[
    ("first_notification", 1),
    ("topic_timeline", 2),
    ("post_menu", 3),
    ("topic_notification_levels", 4),
    ("suggested_topics", 5),
];

/// `TrustLevel.levels`
const TRUST_LEVELS: &[(&str, i64)] = &[
    ("newuser", 0),
    ("basic", 1),
    ("member", 2),
    ("regular", 3),
    ("leader", 4),
];

/// `Discourse.filters` (lib/discourse.rb), core only.
const FILTERS: &[&str] = &[
    "latest",
    "unread",
    "new",
    "unseen",
    "top",
    "read",
    "posted",
    "bookmarks",
    "hot",
];

/// `Discourse.anonymous_filters`
const ANONYMOUS_FILTERS: &[&str] = &["latest", "top", "categories", "hot"];

/// `TopTopic.periods`
const PERIODS: &[&str] = &["all", "yearly", "quarterly", "monthly", "weekly", "daily"];

/// `UserField.max_length`
const USER_FIELD_MAX_LENGTH: i64 = 2048;

/// `DiscourseTagging::TAGS_FILTER_REGEXP.source`; Ruby drops the `\/`
/// lexer escape from the literal.
const TAGS_FILTER_REGEXP: &str = r#"[/\?#\[\]@!\$&'\(\)\*\+,;=%\\`^\s|\{\}"<>]+"#;

/// `PostActionType::LIKE_POST_ACTION_ID`
const LIKE_POST_ACTION_ID: i64 = 2;

/// `Archetype.list` minus private_message.
const ARCHETYPES: &[&str] = &["regular", "banner"];

/// The `enable_*` settings behind `Discourse::BUILTIN_AUTH`.
const BUILTIN_AUTH_SETTINGS: &[&str] = &[
    "enable_discourse_id",
    "enable_facebook_logins",
    "enable_google_oauth2_logins",
    "enable_github_logins",
    "enable_twitter_logins",
    "enable_discord_logins",
    "enable_linkedin_oidc_logins",
];

#[derive(Debug)]
pub enum SiteError {
    Db(sqlx::Error),
    Setting(SettingError),
    Url(UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for SiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SiteError::Db(e) => write!(f, "building site.json: {e}"),
            SiteError::Setting(e) => e.fmt(f),
            SiteError::Url(e) => e.fmt(f),
            SiteError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SiteError {}

impl From<sqlx::Error> for SiteError {
    fn from(e: sqlx::Error) -> Self {
        SiteError::Db(e)
    }
}

impl From<SettingError> for SiteError {
    fn from(e: SettingError) -> Self {
        SiteError::Setting(e)
    }
}

impl From<UrlError> for SiteError {
    fn from(e: UrlError) -> Self {
        SiteError::Url(e)
    }
}

impl From<Unsupported> for SiteError {
    fn from(e: Unsupported) -> Self {
        SiteError::Unsupported(e)
    }
}

pub struct Site<'a> {
    pub conn: &'a mut PgConnection,
    pub config: &'a Config,
    pub settings: &'a SiteSettings,
    pub defs: &'a Definitions,
    pub i18n: &'a I18n,
    pub guardian: Guardian,
}

fn enum_object(pairs: &[(&str, i64)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect(),
    )
}

fn str_list(items: &[&str]) -> Value {
    json!(items)
}

/// `String#parameterize` for ASCII titles.
fn parameterize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

impl Site<'_> {
    fn setting(&self, name: &str) -> Result<&crate::site_settings::Value, SiteError> {
        Ok(self.settings.get(name)?)
    }

    fn truthy(&self, name: &str) -> Result<bool, SiteError> {
        Ok(self.setting(name)?.truthy())
    }

    /// `Discourse.base_path`
    fn base_path(&self) -> &str {
        self.config.globals.relative_url_root()
    }

    /// `Site.json_for(guardian)`: the full serializer, or the reduced
    /// login_required document for anonymous users.
    pub async fn json_for(&mut self) -> Result<Value, SiteError> {
        if self.guardian.is_anonymous() && self.truthy("login_required")? {
            return self.login_required_json().await;
        }

        let mut out = Map::new();
        out.insert("default_archetype".into(), json!(DEFAULT_ARCHETYPE));
        out.insert("notification_types".into(), enum_object(NOTIFICATION_TYPES));
        out.insert("post_types".into(), enum_object(POST_TYPES));
        if self.truthy("enable_user_tips")? {
            out.insert("user_tips".into(), enum_object(USER_TIPS));
        }
        out.insert("trust_levels".into(), enum_object(TRUST_LEVELS));
        out.insert("groups".into(), self.groups().await?);
        out.insert("filters".into(), str_list(FILTERS));
        out.insert(
            "anonymous_list_filters".into(),
            self.anonymous_list_filters(),
        );
        out.insert("homepage_choices".into(), self.homepage_choices()?);
        // DiscoursePluginRegistry.homepage_options: plugins only.
        out.insert("homepage_options".into(), json!([]));
        out.insert("periods".into(), str_list(PERIODS));
        out.insert("top_menu_items".into(), self.top_menu_items());
        out.insert(
            "anonymous_top_menu_items".into(),
            self.anonymous_top_menu_items(),
        );
        out.insert(
            "uncategorized_category_id".into(),
            json!(self.setting("uncategorized_category_id")?),
        );
        out.insert("user_field_max_length".into(), json!(USER_FIELD_MAX_LENGTH));
        let used_flag_ids = self.used_flag_ids().await?;
        out.insert(
            "post_action_types".into(),
            self.post_action_types(&used_flag_ids).await?,
        );
        out.insert(
            "topic_flag_types".into(),
            self.topic_flag_types(&used_flag_ids).await?,
        );
        out.insert(
            "can_create_tag".into(),
            json!(self.guardian.can_create_tag(self.settings)?),
        );
        out.insert(
            "can_search".into(),
            json!(self.guardian.can_search(self.settings)?),
        );
        out.insert(
            "can_tag_topics".into(),
            json!(self.guardian.can_tag_topics(self.settings)?),
        );
        out.insert(
            "can_tag_pms".into(),
            json!(self.guardian.can_tag_pms(self.settings)?),
        );
        let tagging = self.truthy("tagging_enabled")?;
        if tagging {
            out.insert("tags_filter_regexp".into(), json!(TAGS_FILTER_REGEXP));
            let top_tags = self.top_tags().await?;
            let nav_tags = self.navigation_menu_site_top_tags(&top_tags).await?;
            out.insert("top_tags".into(), Value::Array(top_tags));
            out.insert("navigation_menu_site_top_tags".into(), nav_tags);
        }
        if self.guardian.is_admin() {
            // AssociatedGroup.has_provider?: an enabled authenticator that
            // provides groups (plugin authenticators only).
            out.insert("can_associate_groups".into(), json!(false));
        }
        if self.wizard_required().await? {
            out.insert("wizard_required".into(), json!(true));
        }
        if self.truthy("topic_featured_link_enabled")? {
            out.insert(
                "topic_featured_link_allowed_category_ids".into(),
                self.topic_featured_link_allowed_category_ids().await?,
            );
        }
        out.insert("user_themes".into(), self.user_themes().await?);
        out.insert(
            "user_color_schemes".into(),
            self.user_color_schemes().await?,
        );
        out.insert(
            "default_light_color_scheme".into(),
            self.default_color_scheme("color_scheme_id").await?,
        );
        out.insert(
            "default_dark_color_scheme".into(),
            self.default_color_scheme("dark_color_scheme_id").await?,
        );
        out.insert(
            "censored_regexp".into(),
            self.watched_words("censor").await?.unwrap_or(json!([])),
        );
        if let Some(id) = self.settings.get("shared_drafts_category")?.presence() {
            if self.guardian.can_see_shared_draft(self.settings)? {
                out.insert(
                    "shared_drafts_category_id".into(),
                    json!(crate::ruby::to_i(&id)),
                );
            }
        }
        // Plugin::CustomEmoji.translations: plugins only.
        out.insert("custom_emoji_translation".into(), json!({}));
        out.insert(
            "watched_words_replace".into(),
            self.watched_words("replace").await?.unwrap_or(Value::Null),
        );
        out.insert(
            "watched_words_link".into(),
            self.watched_words("link").await?.unwrap_or(Value::Null),
        );
        let categories = Categories {
            conn: self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: &self.guardian,
            base_path: self.config.globals.relative_url_root(),
            topic_url_via_slug: true,
        }
        .for_site()
        .await?;
        if !categories.is_empty() {
            out.insert("categories".into(), Value::Array(categories));
        }
        // Site.markdown_additional_options: plugins only.
        out.insert("markdown_additional_options".into(), json!({}));
        out.insert(
            "hashtag_configurations".into(),
            self.hashtag_configurations(tagging),
        );
        out.insert("hashtag_icons".into(), self.hashtag_icons(tagging));
        if self.guardian.is_anonymous() {
            if let Some(tags) = self.anonymous_default_navigation_menu_tags(tagging).await? {
                out.insert("anonymous_default_navigation_menu_tags".into(), tags);
            }
            out.insert(
                "anonymous_sidebar_sections".into(),
                self.anonymous_sidebar_sections().await?,
            );
        }
        if self.guardian.can_see_whispers(self.settings)? {
            let ids = self.settings.group_ids("whispers_allowed_groups")?;
            let names: Vec<String> =
                sqlx::query_scalar("SELECT name FROM groups WHERE id = ANY($1::bigint[])")
                    .bind(&ids)
                    .fetch_all(&mut *self.conn)
                    .await?;
            out.insert("whispers_allowed_groups_names".into(), json!(names));
        }
        if let Some(denied) = self.denied_emojis()? {
            out.insert("denied_emojis".into(), denied);
        }
        if let Some(url) = self.tos_url().await? {
            out.insert("tos_url".into(), json!(url));
        }
        if let Some(url) = self.privacy_policy_url().await? {
            out.insert("privacy_policy_url".into(), json!(url));
        }
        if self.truthy("show_user_menu_avatars")? {
            return Err(Unsupported("system_user_avatar_template (avatar templates)").into());
        }
        if self.guardian.can_lazy_load_categories(self.settings)? {
            out.insert("lazy_load_categories".into(), json!(true));
        }
        if self.guardian.is_admin() {
            // Flag.valid_applies_to_types: core's, plus what plugins add.
            out.insert(
                "valid_flag_applies_to_types".into(),
                json!(["Post", "Topic"]),
            );
            // DiscoursePluginRegistry.admin_config_login_routes: plugins only.
            out.insert("admin_config_login_routes".into(), json!([]));
        }
        out.insert(
            "full_name_required_for_signup".into(),
            json!(self.full_name_requirement()? == "required_at_signup"),
        );
        out.insert(
            "full_name_visible_in_signup".into(),
            json!(self.full_name_requirement()? != "hidden_at_signup"),
        );
        out.insert(
            "email_configured".into(),
            json!(self.config.globals.get("smtp_address").is_some()),
        );
        out.insert(
            "upcoming_changes_with_css".into(),
            json!(self.defs.upcoming_changes_with_css()),
        );
        if self.guardian.is_staff() {
            out.insert(
                "permanent_upcoming_change_names".into(),
                json!(self.defs.permanent_upcoming_change_names()),
            );
        }
        // AclTarget registry: plugins only.
        out.insert(
            "access_control".into(),
            json!({"mandatory_acl": {}, "banned_acl": {}}),
        );
        if self.guardian.is_staff() {
            out.insert("category_types".into(), self.category_types());
        }
        out.insert("archetypes".into(), self.archetypes());
        out.insert("user_fields".into(), self.user_fields().await?);
        out.insert("auth_providers".into(), self.auth_providers()?);

        Ok(Value::Object(out))
    }

    /// `Wizard.user_requires_completion?(user)`: the wizard is on, the
    /// site has at most 15 topics, and the viewer is the first admin to
    /// have logged in and hasn't finished it. (Rails flips
    /// bypass_wizard_check on when the topic count passes 15; the port
    /// only reads.) A partly completed wizard isn't ported.
    async fn wizard_required(&mut self) -> Result<bool, SiteError> {
        let Some(user) = self.guardian.user() else {
            return Ok(false);
        };
        if !self.truthy("wizard_enabled")? || self.truthy("bypass_wizard_check")? {
            return Ok(false);
        }
        let topics: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (SELECT 1 FROM topics WHERE deleted_at IS NULL LIMIT 16) t",
        )
        .fetch_one(&mut *self.conn)
        .await?;
        if topics > 15 {
            return Ok(false);
        }
        // User.first_login_admin_id
        let first_admin: Option<i32> = sqlx::query_scalar(
            "SELECT users.id FROM users JOIN user_auth_tokens ON user_auth_tokens.user_id = users.id \
             WHERE users.admin AND users.id > 0 ORDER BY user_auth_tokens.created_at LIMIT 1",
        )
        .fetch_optional(&mut *self.conn)
        .await?;
        if first_admin != Some(user.id) {
            return Ok(false);
        }
        // UserHistory.actions[:wizard_step] = 40
        let started: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_histories WHERE action = 40)")
                .fetch_one(&mut *self.conn)
                .await?;
        if started {
            return Err(Unsupported("wizard completion (Wizard::Builder steps)").into());
        }
        Ok(true)
    }

    /// `Categories::TypeRegistry.list(only_visible: true)`: core registers
    /// the Discussion type; plugins add theirs.
    fn category_types(&self) -> Value {
        let t = |key: &str, default: &str| {
            self.i18n
                .t(&format!("category_types.discussion.{key}"))
                .unwrap_or(default)
                .to_string()
        };
        json!([{
            "id": "discussion",
            "name": t("name", "Discussion"),
            "title": t("title", "discussion"),
            "description": t("description", ""),
            "icon": "memo",
            "available": true,
            "visible": true,
            "configuration_schema": {},
        }])
    }
    /// Site.json_for's login_required branch (site.rb:245-268).
    async fn login_required_json(&mut self) -> Result<Value, SiteError> {
        let mut out = Map::new();
        out.insert("periods".into(), str_list(PERIODS));
        out.insert("filters".into(), str_list(FILTERS));
        out.insert(
            "anonymous_list_filters".into(),
            self.anonymous_list_filters(),
        );
        out.insert("user_fields".into(), self.user_fields().await?);
        out.insert("auth_providers".into(), self.auth_providers()?);
        out.insert(
            "full_name_required_for_signup".into(),
            json!(self.full_name_requirement()? == "required_at_signup"),
        );
        out.insert(
            "full_name_visible_in_signup".into(),
            json!(self.full_name_requirement()? != "hidden_at_signup"),
        );
        out.insert("tos_url".into(), json!(self.tos_url().await?));
        out.insert(
            "privacy_policy_url".into(),
            json!(self.privacy_policy_url().await?),
        );
        out.insert(
            "upcoming_changes_with_css".into(),
            json!(self.defs.upcoming_changes_with_css()),
        );
        Ok(Value::Object(out))
    }

    /// `Discourse.filters & Discourse.anonymous_filters`, in filters order.
    fn anonymous_list_filters(&self) -> Value {
        json!(
            FILTERS
                .iter()
                .filter(|f| ANONYMOUS_FILTERS.contains(f))
                .collect::<Vec<_>>()
        )
    }

    /// `HomepageSiteSetting.choices` = `TopMenu.homepage_choices` (no
    /// plugin homepages): `TopMenu.choices | (filters - ["unread"])`.
    fn homepage_choices(&self) -> Result<Value, SiteError> {
        let mut choices: Vec<&str> = vec![
            "latest",
            "new",
            "unseen",
            "top",
            "categories",
            "read",
            "posted",
            "bookmarks",
            "hot",
        ];
        if !self.truthy("enable_unified_new")? {
            choices.push("unread");
        }
        for f in FILTERS {
            if *f != "unread" && !choices.contains(f) {
                choices.push(f);
            }
        }
        Ok(json!(choices))
    }

    /// `Discourse.top_menu_items`: filters + categories.
    fn top_menu_items(&self) -> Value {
        json!(
            FILTERS
                .iter()
                .chain(["categories"].iter())
                .collect::<Vec<_>>()
        )
    }

    /// `Discourse.anonymous_top_menu_items`: anonymous_filters + [categories,
    /// top]; the duplicates are real.
    fn anonymous_top_menu_items(&self) -> Value {
        json!(
            ANONYMOUS_FILTERS
                .iter()
                .chain(["categories", "top"].iter())
                .collect::<Vec<_>>()
        )
    }

    /// `Site#groups` for anonymous: public groups, custom ones only when
    /// the granular permissions change is on.
    async fn groups(&mut self) -> Result<Value, SiteError> {
        let include_everyone =
            !self.truthy("granular_anonymous_and_logged_in_groups_permissions")?;
        #[derive(sqlx::FromRow)]
        struct Row {
            id: i32,
            name: String,
            full_name: Option<String>,
            flair_icon: Option<String>,
            flair_upload_id: Option<i32>,
            flair_bg_color: Option<String>,
            flair_color: Option<String>,
            automatic: bool,
        }
        let visible = crate::groups::visible_groups_where(&self.guardian, "groups");
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT id, name, full_name, flair_icon, flair_upload_id, flair_bg_color, flair_color, automatic \
             FROM groups WHERE ($1 OR id > 0) AND {visible} ORDER BY name ASC"
        ))
        .bind(include_everyone)
        .fetch_all(&mut *self.conn)
        .await?;

        let mut groups = Vec::with_capacity(rows.len());
        for Row {
            id,
            name,
            full_name,
            flair_icon,
            flair_upload_id,
            flair_bg_color,
            flair_color,
            automatic,
        } in rows
        {
            let display = full_name
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| name.clone());
            let flair_url = match (flair_icon.filter(|i| !i.is_empty()), flair_upload_id) {
                (Some(icon), _) => Some(icon),
                (None, Some(_)) => return Err(Unsupported("group flair uploads").into()),
                (None, None) => None,
            };
            groups.push(json!({
                "id": id,
                "name": name,
                "full_name": display,
                "display_name": display,
                "flair_url": flair_url,
                "flair_bg_color": flair_bg_color,
                "flair_color": flair_color,
                "automatic": automatic,
            }));
        }
        Ok(Value::Array(groups))
    }

    /// `Flag.used_flag_ids`: flags with any post_action or reviewable score.
    async fn used_flag_ids(&mut self) -> Result<Vec<i64>, SiteError> {
        Ok(sqlx::query_scalar(
            "SELECT DISTINCT post_action_type_id::bigint FROM post_actions \
             UNION SELECT DISTINCT reviewable_score_type::bigint FROM reviewable_scores",
        )
        .fetch_all(&mut *self.conn)
        .await?)
    }

    /// FlagSerializer over `Flag.unscoped.order(:position).where(score_type: false)`.
    async fn post_action_types(&mut self, used: &[i64]) -> Result<Value, SiteError> {
        let flags = self
            .flags("SELECT id, name, name_key, description, applies_to, position, require_message, enabled, score_type, auto_action_type \
                    FROM flags WHERE NOT score_type ORDER BY position")
            .await?;
        Ok(Value::Array(
            flags
                .iter()
                .map(|f| self.serialize_flag(f, "post_action", used))
                .collect(),
        ))
    }

    /// Same, restricted to flags that apply to topics.
    async fn topic_flag_types(&mut self, used: &[i64]) -> Result<Value, SiteError> {
        let flags = self
            .flags("SELECT id, name, name_key, description, applies_to, position, require_message, enabled, score_type, auto_action_type \
                    FROM flags WHERE 'Topic' = ANY(applies_to) AND NOT score_type ORDER BY position")
            .await?;
        Ok(Value::Array(
            flags
                .iter()
                .map(|f| self.serialize_flag(f, "topic_flag", used))
                .collect(),
        ))
    }

    async fn flags(&mut self, sql: &str) -> Result<Vec<Flag>, SiteError> {
        Ok(sqlx::query_as(sql).fetch_all(&mut *self.conn).await?)
    }

    fn serialize_flag(&self, flag: &Flag, target: &str, used: &[i64]) -> Value {
        let prefix = format!("{target}_types.{}", flag.name_key);
        let base_path = [("base_path", self.base_path())];
        // I18n.t without arguments leaves placeholders literal; with
        // base_path passed, a missing placeholder falls back to the default.
        let name = self
            .i18n
            .t(&format!("{prefix}.title"))
            .unwrap_or(&flag.name);
        let description = self
            .i18n
            .t_with(&format!("{prefix}.description"), &base_path)
            .unwrap_or_else(|| flag.description.clone().unwrap_or_default());
        let short_description = self
            .i18n
            .t_with(&format!("{prefix}.short_description"), &base_path)
            .unwrap_or_default();
        json!({
            "id": flag.id,
            "name": name,
            "name_key": flag.name_key,
            "description": description,
            "short_description": short_description,
            "applies_to": flag.applies_to,
            "position": flag.position,
            "require_message": flag.require_message,
            "enabled": flag.enabled,
            "is_flag": !flag.score_type && flag.id != LIKE_POST_ACTION_ID,
            "is_used": used.contains(&flag.id),
            "auto_action_type": flag.auto_action_type,
            "system": flag.id < 1000,
        })
    }

    /// `Tag.top_tags(guardian:)`: visible tags with topics in allowed
    /// categories.
    async fn top_tags(&mut self) -> Result<Vec<Value>, SiteError> {
        let category_ids = self
            .guardian
            .allowed_category_ids(self.conn, self.settings)
            .await?;
        if category_ids.is_empty() {
            return Ok(Vec::new());
        }

        let limit = self.setting("max_tags_in_filter_list")?.to_i() + 1;
        let rows: Vec<(i32, String, Option<String>)> = sqlx::query_as(&format!(
            "SELECT tags.id, tags.name, tags.slug \
             FROM category_tag_stats stats \
             JOIN tags ON stats.tag_id = tags.id AND stats.topic_count > 0 \
             WHERE stats.category_id = ANY($1) AND tags.target_tag_id IS NULL \
             AND tags.id IN {visible} \
             GROUP BY tags.id \
             ORDER BY SUM(stats.topic_count) DESC, tags.name ASC \
             LIMIT $2",
            visible = visible_tag_ids_subquery(&self.guardian, self.settings)?,
        ))
        .bind(&category_ids)
        .bind(limit)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, name, slug)| {
                let slug = slug
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("{id}-tag"));
                json!({"id": id, "name": name, "slug": slug})
            })
            .collect())
    }

    /// The first five top tags through SidebarTagSerializer, in top-tag
    /// order. `pm_only` uses public_topic_count for non-staff.
    async fn navigation_menu_site_top_tags(
        &mut self,
        top_tags: &[Value],
    ) -> Result<Value, SiteError> {
        const SIDEBAR_TOP_TAGS_TO_SHOW: usize = 5;
        let ids: Vec<i32> = top_tags
            .iter()
            .take(SIDEBAR_TOP_TAGS_TO_SHOW)
            .filter_map(|t| t["id"].as_i64().and_then(|i| i32::try_from(i).ok()))
            .collect();
        if ids.is_empty() {
            return Ok(json!([]));
        }
        #[derive(sqlx::FromRow)]
        struct Tag {
            id: i32,
            name: String,
            slug: Option<String>,
            description: Option<String>,
            public_topic_count: i32,
            pm_topic_count: i32,
        }
        let tags: Vec<Tag> = sqlx::query_as(
            "SELECT id, name, slug, description, public_topic_count, pm_topic_count \
             FROM tags WHERE id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out: Vec<(usize, Value)> = tags
            .into_iter()
            .map(|t| {
                let slug = t
                    .slug
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| format!("{}-tag", t.id));
                let pos = ids.iter().position(|id| *id == t.id).unwrap_or(usize::MAX);
                (
                    pos,
                    json!({
                        "id": t.id,
                        "name": t.name,
                        "slug": slug,
                        "description": t.description,
                        "pm_only": t.public_topic_count == 0 && t.pm_topic_count > 0,
                    }),
                )
            })
            .collect();
        out.sort_by_key(|(pos, _)| *pos);
        Ok(Value::Array(out.into_iter().map(|(_, v)| v).collect()))
    }

    async fn topic_featured_link_allowed_category_ids(&mut self) -> Result<Value, SiteError> {
        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM categories WHERE topic_featured_link_allowed")
                .fetch_all(&mut *self.conn)
                .await?;
        Ok(json!(ids))
    }

    /// `user_themes`: the default theme plus user-selectable ones.
    async fn user_themes(&mut self) -> Result<Value, SiteError> {
        let default_id = self.setting("default_theme_id")?.to_i();
        #[derive(sqlx::FromRow)]
        struct Theme {
            id: i32,
            name: String,
            color_scheme_id: Option<i32>,
            dark_color_scheme_id: Option<i32>,
            only: Option<bool>,
        }
        let rows: Vec<Theme> = sqlx::query_as(
            "SELECT t.id, t.name, t.color_scheme_id, t.dark_color_scheme_id, m.only_theme_color_schemes AS only \
             FROM themes t \
             LEFT JOIN theme_modifier_sets m ON m.theme_id = t.id \
             WHERE t.id = $1 OR t.user_selectable \
             ORDER BY lower(t.name)",
        )
        .bind(i32::try_from(default_id).unwrap_or(0))
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(json!(
            rows.into_iter()
                .map(
                    |Theme {
                         id,
                         name,
                         color_scheme_id,
                         dark_color_scheme_id,
                         only,
                     }| json!({
                        "theme_id": id,
                        "name": name,
                        "default": i64::from(id) == default_id,
                        "color_scheme_id": color_scheme_id,
                        "dark_color_scheme_id": dark_color_scheme_id,
                        "only_theme_color_schemes": only.unwrap_or(false),
                    })
                )
                .collect::<Vec<_>>()
        ))
    }

    /// `user_color_schemes`: user-selectable schemes. Serializing one needs
    /// ColorScheme#resolved_colors, not ported yet.
    async fn user_color_schemes(&mut self) -> Result<Value, SiteError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM color_schemes cs \
             WHERE NOT cs.remote_copy AND (cs.user_selectable OR cs.theme_id IN \
               (SELECT theme_id FROM theme_modifier_sets WHERE only_theme_color_schemes))",
        )
        .fetch_one(&mut *self.conn)
        .await?;
        if count > 0 {
            return Err(Unsupported("user_color_schemes (ColorSchemeSelectableSerializer)").into());
        }
        Ok(json!([]))
    }

    /// `default_light_color_scheme` / `default_dark_color_scheme`:
    /// `ColorScheme.find_by_id(Theme.find_default&.<column>)` through
    /// ColorSchemeSerializer, null when the theme has none.
    async fn default_color_scheme(&mut self, column: &str) -> Result<Value, SiteError> {
        let default_id = i32::try_from(self.setting("default_theme_id")?.to_i()).unwrap_or(0);
        let sql = format!("SELECT {column} FROM themes WHERE id = $1");
        let scheme_id: Option<Option<i32>> = sqlx::query_scalar(&sql)
            .bind(default_id)
            .fetch_optional(&mut *self.conn)
            .await?;
        let Some(scheme_id) = scheme_id.flatten() else {
            return Ok(Value::Null);
        };
        match ColorScheme::find(self.conn, scheme_id).await? {
            None => Ok(Value::Null),
            Some(scheme) => Ok(scheme.serialize()?),
        }
    }

    /// WordWatcher regexps for an action: None when there are no words.
    /// Building the regexps isn't ported, so present words are refused.
    async fn watched_words(&mut self, action: &str) -> Result<Option<Value>, SiteError> {
        let action_id = match action {
            "censor" => 2,
            "replace" => 5,
            "link" => 8,
            _ => return Err(Unsupported("watched word action").into()),
        };
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM watched_words WHERE action = $1")
            .bind(action_id)
            .fetch_one(&mut *self.conn)
            .await?;
        if count > 0 {
            return Err(Unsupported("watched words (WordWatcher regexps)").into());
        }
        Ok(None)
    }

    /// `HashtagAutocompleteService.contexts_with_ordered_types`, core data
    /// sources only (category, tag).
    fn hashtag_configurations(&self, tagging: bool) -> Value {
        let mut types = vec!["category"];
        if tagging {
            types.push("tag");
        }
        json!({"topic-composer": types})
    }

    fn hashtag_icons(&self, tagging: bool) -> Value {
        let mut icons = Map::new();
        icons.insert("category".into(), json!("folder"));
        if tagging {
            icons.insert("tag".into(), json!("tag"));
        }
        Value::Object(icons)
    }

    async fn anonymous_default_navigation_menu_tags(
        &mut self,
        tagging: bool,
    ) -> Result<Option<Value>, SiteError> {
        if !tagging
            || self
                .setting("default_navigation_menu_tags")?
                .presence()
                .is_none()
        {
            return Ok(None);
        }
        Err(Unsupported("anonymous_default_navigation_menu_tags (SidebarTagSerializer)").into())
    }

    /// Public sidebar sections with their links (SidebarSectionSerializer).
    async fn anonymous_sidebar_sections(&mut self) -> Result<Value, SiteError> {
        #[derive(sqlx::FromRow)]
        struct Section {
            id: i64,
            title: String,
            public: bool,
            section_type: Option<i32>,
            locale: Option<String>,
        }
        let sections: Vec<Section> = sqlx::query_as(
            "SELECT id, title, public, section_type, locale FROM sidebar_sections WHERE public \
             ORDER BY (section_type IS NOT NULL) DESC, (public IS TRUE) DESC, id",
        )
        .fetch_all(&mut *self.conn)
        .await?;

        let mut out = Vec::with_capacity(sections.len());
        for Section {
            id,
            title,
            public,
            section_type,
            locale,
        } in sections
        {
            #[derive(sqlx::FromRow)]
            struct Link {
                id: i64,
                name: String,
                value: String,
                icon: Option<String>,
                external: bool,
                segment: i32,
                locale: Option<String>,
            }
            let links: Vec<Link> = sqlx::query_as(
                "SELECT u.id, u.name, u.value, u.icon, u.external, u.segment, u.locale \
                 FROM sidebar_section_links l \
                 JOIN sidebar_urls u ON u.id = l.linkable_id AND l.linkable_type = 'SidebarUrl' \
                 WHERE l.sidebar_section_id::bigint = $1 ORDER BY l.position",
            )
            .bind(id)
            .fetch_all(&mut *self.conn)
            .await?;
            let links: Vec<Value> = links
                .into_iter()
                .map(
                    |Link {
                         id,
                         name,
                         value,
                         icon,
                         external,
                         segment,
                         locale,
                     }| {
                        json!({
                            "id": id,
                            "name": name,
                            "value": value,
                            "icon": icon,
                            "external": external,
                            "segment": if segment == 0 { "primary" } else { "secondary" },
                            "locale": locale,
                        })
                    },
                )
                .collect();
            out.push(json!({
                "id": id,
                "title": title,
                "links": links,
                "slug": parameterize(&title),
                "public": public,
                "section_type": section_type.map(|_| "community"),
                "locale": locale,
            }));
        }
        Ok(Value::Array(out))
    }

    /// `Emoji.denied`: nil unless emoji_deny_list has entries.
    fn denied_emojis(&self) -> Result<Option<Value>, SiteError> {
        match self.setting("emoji_deny_list")?.presence() {
            None => Ok(None),
            Some(_) => Err(Unsupported("denied_emojis (Emoji aliases)").into()),
        }
    }

    /// `Discourse.tos_url`: the tos_url setting, else /tos when the ToS topic exists.
    async fn tos_url(&mut self) -> Result<Option<String>, SiteError> {
        self.legal_url("tos_url", "tos_topic_id", "/tos").await
    }

    async fn privacy_policy_url(&mut self) -> Result<Option<String>, SiteError> {
        self.legal_url("privacy_policy_url", "privacy_topic_id", "/privacy")
            .await
    }

    async fn legal_url(
        &mut self,
        url_setting: &str,
        topic_setting: &str,
        path: &str,
    ) -> Result<Option<String>, SiteError> {
        if let Some(url) = self.setting(url_setting)?.presence() {
            return Ok(Some(url));
        }
        let topic_id = self.setting(topic_setting)?.to_i();
        if topic_id <= 0 {
            return Ok(None);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1)")
            .bind(i32::try_from(topic_id).unwrap_or(0))
            .fetch_one(&mut *self.conn)
            .await?;
        Ok(exists.then(|| format!("{}{path}", self.base_path())))
    }

    fn full_name_requirement(&self) -> Result<String, SiteError> {
        Ok(self.setting("full_name_requirement")?.to_s())
    }

    /// ArchetypeSerializer over `Archetype.list` minus private_message.
    fn archetypes(&self) -> Value {
        json!(
            ARCHETYPES
                .iter()
                .map(|id| json!({
                    "id": id,
                    "name": self.i18n.t(&format!("archetypes.{id}.title")),
                    "options": [],
                }))
                .collect::<Vec<_>>()
        )
    }

    /// UserFieldSerializer over `UserField.order(:position)`.
    async fn user_fields(&mut self) -> Result<Value, SiteError> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM user_fields")
            .fetch_one(&mut *self.conn)
            .await?;
        if count > 0 {
            return Err(Unsupported("user_fields (UserFieldSerializer)").into());
        }
        Ok(json!([]))
    }

    /// `Discourse.enabled_auth_providers`: none unless a builtin provider is
    /// enabled, and serializing an enabled one isn't ported.
    fn auth_providers(&self) -> Result<Value, SiteError> {
        for name in BUILTIN_AUTH_SETTINGS {
            if self.truthy(name)? {
                return Err(Unsupported("auth_providers (AuthProviderSerializer)").into());
            }
        }
        Ok(json!([]))
    }
}

#[derive(sqlx::FromRow)]
struct Flag {
    id: i64,
    name: String,
    name_key: String,
    description: Option<String>,
    applies_to: Vec<String>,
    position: Option<i32>,
    require_message: bool,
    enabled: bool,
    score_type: bool,
    auto_action_type: bool,
}

impl From<CategoriesError> for SiteError {
    fn from(e: CategoriesError) -> Self {
        match e {
            CategoriesError::Db(e) => SiteError::Db(e),
            CategoriesError::Setting(e) => SiteError::Setting(e),
            CategoriesError::Url(e) => SiteError::Url(e),
            CategoriesError::Unsupported(e) => SiteError::Unsupported(e),
        }
    }
}

impl From<crate::guardian::GuardianError> for SiteError {
    fn from(e: crate::guardian::GuardianError) -> Self {
        match e {
            crate::guardian::GuardianError::Db(e) => SiteError::Db(e),
            crate::guardian::GuardianError::Setting(e) => SiteError::Setting(e),
            crate::guardian::GuardianError::Unsupported(e) => SiteError::Unsupported(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterize_matches_rails_for_ascii() {
        assert_eq!(parameterize("Community"), "community");
        assert_eq!(parameterize("My  Links!"), "my-links");
        assert_eq!(parameterize("-a_b-"), "a_b");
    }
}
