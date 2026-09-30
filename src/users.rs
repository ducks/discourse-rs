//! Public user profiles for anonymous readers: users_controller#show and
//! #summary (UserSerializer, HiddenProfileSerializer, UserSummary), the
//! badge side-loads (UserBadgeSerializer, BadgeSerializer) and
//! user_actions#index (UserAction.stream, UserActionSerializer).

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::avatar::{self, AvatarError};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list::{LookupUser, TopicListError, TopicListSerializer, time_json};
use crate::url::Urls;

#[derive(Debug)]
pub enum UsersError {
    Db(sqlx::Error),
    Setting(SettingError),
    Unsupported(Unsupported),
    TopicList(TopicListError),
    Avatar(AvatarError),
}

impl std::fmt::Display for UsersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsersError::Db(e) => write!(f, "database: {e}"),
            UsersError::Setting(e) => e.fmt(f),
            UsersError::Unsupported(e) => e.fmt(f),
            UsersError::TopicList(e) => e.fmt(f),
            UsersError::Avatar(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for UsersError {}

impl From<sqlx::Error> for UsersError {
    fn from(e: sqlx::Error) -> Self {
        UsersError::Db(e)
    }
}

impl From<SettingError> for UsersError {
    fn from(e: SettingError) -> Self {
        UsersError::Setting(e)
    }
}

impl From<Unsupported> for UsersError {
    fn from(e: Unsupported) -> Self {
        UsersError::Unsupported(e)
    }
}

impl From<TopicListError> for UsersError {
    fn from(e: TopicListError) -> Self {
        UsersError::TopicList(e)
    }
}

impl From<AvatarError> for UsersError {
    fn from(e: AvatarError) -> Self {
        UsersError::Avatar(e)
    }
}

/// `UserAction` types.
pub const LIKE: i32 = 1;
pub const WAS_LIKED: i32 = 2;
pub const NEW_TOPIC: i32 = 4;
pub const REPLY: i32 = 5;
pub const RESPONSE: i32 = 6;
pub const MENTION: i32 = 7;
pub const QUOTE: i32 = 9;
pub const EDIT: i32 = 11;
/// `UserAction.private_types`
pub const PRIVATE_TYPES: &[i32] = &[WAS_LIKED, RESPONSE, MENTION, QUOTE, EDIT];
/// `UserAction.types.values - private_types`
pub const PUBLIC_TYPES: &[i32] = &[1, 4, 5, 12, 13, 15, 16, 17];

/// `UserSummary::MAX_SUMMARY_RESULTS`, `MAX_BADGES`
const MAX_SUMMARY_RESULTS: i64 = 6;
const MAX_BADGES: i64 = 6;

/// A user with what the profile serializers read from users, user_stats,
/// user_profiles and user_options.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i32,
    pub username: String,
    pub name: Option<String>,
    pub uploaded_avatar_id: Option<i32>,
    pub primary_group_id: Option<i32>,
    pub flair_group_id: Option<i32>,
    pub admin: bool,
    pub moderator: bool,
    pub trust_level: i32,
    pub staged: bool,
    pub title: Option<String>,
    pub last_posted_at: Option<NaiveDateTime>,
    pub last_seen_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub suspended_till: Option<NaiveDateTime>,
    pub silenced_till: Option<NaiveDateTime>,
    pub post_count: Option<i32>,
    pub topic_count: Option<i32>,
    pub distinct_badge_count: Option<i32>,
    pub time_read: Option<i32>,
    pub likes_given: Option<i32>,
    pub likes_received: Option<i32>,
    pub topics_entered: Option<i32>,
    pub posts_read_count: Option<i32>,
    pub days_visited: Option<i32>,
    pub profile_id: Option<i32>,
    pub views: Option<i32>,
    pub bio_raw: Option<String>,
    pub bio_cooked: Option<String>,
    pub website: Option<String>,
    pub location: Option<String>,
    pub card_background_upload_id: Option<i32>,
    pub profile_background_upload_id: Option<i32>,
    pub featured_topic_id: Option<i32>,
    pub hide_profile: Option<bool>,
}

const USER_COLUMNS: &str = "users.id, users.username, users.name, users.uploaded_avatar_id, \
    users.primary_group_id, users.flair_group_id, users.admin, users.moderator, users.trust_level, \
    users.staged, users.title, users.last_posted_at, users.last_seen_at, users.created_at, \
    users.suspended_till, users.silenced_till, \
    us.post_count, us.topic_count, us.distinct_badge_count, us.time_read, us.likes_given, \
    us.likes_received, us.topics_entered, us.posts_read_count, us.days_visited, \
    up.user_id AS profile_id, up.views, up.bio_raw, up.bio_cooked, up.website, up.location, \
    up.card_background_upload_id, up.profile_background_upload_id, up.featured_topic_id, \
    uo.hide_profile";

const USER_FROM: &str = "FROM users \
    LEFT JOIN user_stats us ON us.user_id = users.id \
    LEFT JOIN user_profiles up ON up.user_id = users.id \
    LEFT JOIN user_options uo ON uo.user_id = users.id";

impl User {
    /// `fetch_user_from_params` for an anonymous reader: by username_lower,
    /// active only.
    pub async fn find_active(
        conn: &mut PgConnection,
        username: &str,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {USER_COLUMNS} {USER_FROM} WHERE users.username_lower = $1 AND users.active = TRUE LIMIT 1"
        ))
        .bind(username.to_lowercase())
        .fetch_optional(conn)
        .await
    }

    /// `User#has_trust_level?`
    fn has_trust_level(&self, level: i32) -> bool {
        self.admin || self.moderator || self.staged || self.trust_level >= level
    }

    fn suspended(&self) -> bool {
        self.suspended_till
            .is_some_and(|t| t > chrono::Utc::now().naive_utc())
    }

    fn silenced(&self) -> bool {
        self.silenced_till
            .is_some_and(|t| t > chrono::Utc::now().naive_utc())
    }

    /// `guardian.can_see_profile?(user)` for an anonymous reader.
    pub fn visible_to_anonymous(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        if settings.get("hide_user_profiles_from_public")?.truthy() {
            return Ok(false);
        }
        let profile_hidden = settings.get("allow_users_to_hide_profile")?.truthy()
            && self.hide_profile.unwrap_or(false);
        if (self.admin || self.moderator) && !profile_hidden {
            return Ok(true);
        }
        if settings.get("hide_new_user_profiles")?.truthy()
            && !settings.get("invite_only")?.truthy()
            && !settings.get("must_approve_users")?.truthy()
        {
            if self.post_count.unwrap_or(0) == 0 && !self.has_trust_level(2) {
                return Ok(false);
            }
            return Ok(self.has_trust_level(1) && !profile_hidden);
        }
        Ok(!profile_hidden)
    }

    /// `guardian.restrict_user_fields?(user)`
    pub fn restrict_fields(&self, settings: &SiteSettings) -> Result<bool, SettingError> {
        Ok(self.trust_level == 0 || !self.visible_to_anonymous(settings)?)
    }

    fn lookup_user(&self) -> LookupUser {
        LookupUser {
            id: self.id,
            username: self.username.clone(),
            name: self.name.clone(),
            uploaded_avatar_id: self.uploaded_avatar_id,
            primary_group_id: self.primary_group_id,
            flair_group_id: self.flair_group_id,
            admin: self.admin,
            moderator: self.moderator,
            trust_level: self.trust_level,
        }
    }
}

/// A row of `User#featured_user_badges`.
#[derive(Debug, sqlx::FromRow)]
struct UserBadgeRow {
    id: i32,
    badge_id: i32,
    user_id: i32,
    granted_at: NaiveDateTime,
    granted_by_id: i32,
    created_at: NaiveDateTime,
    count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct BadgeRow {
    id: i32,
    name: String,
    description: Option<String>,
    grant_count: i32,
    allow_title: bool,
    multiple_grant: bool,
    icon: Option<String>,
    image_upload_url: Option<String>,
    listable: bool,
    enabled: bool,
    badge_grouping_id: i32,
    system: bool,
    show_in_post_header: bool,
    badge_type_id: i32,
    badge_type_name: String,
}

/// The side-loaded roots a document accumulates while serializing badges
/// (AMS embeds ids and appends the objects to top-level arrays, deduped).
#[derive(Default)]
pub struct SideLoads {
    pub badges: Vec<Value>,
    pub badge_types: Vec<Value>,
    pub users: Vec<Value>,
    badge_ids: Vec<i32>,
    badge_type_ids: Vec<i32>,
    user_ids: Vec<i32>,
}

pub struct Users<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
    pub base_path: &'a str,
}

impl Users<'_> {
    fn list(&mut self) -> TopicListSerializer<'_> {
        TopicListSerializer {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: self.guardian,
            urls: self.urls,
            more_topics_url: None,
            category_id: None,
        }
    }

    /// `User#avatar_template` (the system user may get the site logo).
    async fn avatar_template(&mut self, user: &LookupUser) -> Result<String, UsersError> {
        let logo = self.list().logo_small_url().await?;
        Ok(avatar::avatar_template(
            self.urls,
            user.id,
            &user.username,
            user.uploaded_avatar_id,
            logo.as_deref(),
        )?)
    }

    /// users#show: `{user_badges, [badges, badge_types, users], user}`, or
    /// the hidden profile document.
    pub async fn show(&mut self, user: &User) -> Result<Value, UsersError> {
        let enable_names = self.settings.get("enable_names")?.truthy();
        let mut out = Map::new();
        if !user.visible_to_anonymous(self.settings)? {
            // HiddenProfileSerializer
            let mut u = Map::new();
            u.insert("id".into(), json!(user.id));
            u.insert("username".into(), json!(user.username));
            if enable_names {
                u.insert("name".into(), json!(user.name));
            }
            u.insert(
                "avatar_template".into(),
                json!(self.avatar_template(&user.lookup_user()).await?),
            );
            u.insert("profile_hidden".into(), json!(true));
            u.insert("title".into(), json!(user.title));
            u.insert(
                "primary_group_name".into(),
                json!(self.group_name(user.primary_group_id).await?),
            );
            u.insert("can_send_private_message_to_user".into(), json!(false));
            out.insert("user".into(), Value::Object(u));
            return Ok(Value::Object(out));
        }

        let mut side = SideLoads::default();
        let rank_limit = {
            let max = self.settings.get("max_favorite_badges")?.to_i();
            if max > 0 { max + 1 } else { 3 }
        };
        let featured = self.featured_badges(user.id, rank_limit, &mut side).await?;
        out.insert("user_badges".into(), Value::Array(featured.clone()));
        if !side.badges.is_empty() {
            out.insert("badges".into(), Value::Array(side.badges.clone()));
            out.insert("badge_types".into(), Value::Array(side.badge_types.clone()));
            out.insert("users".into(), Value::Array(side.users.clone()));
        }

        // UserSerializer
        let mut u = Map::new();
        if self.settings.get("enable_user_status")?.truthy() {
            let has_status: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = $1 AND (ends_at IS NULL OR ends_at > now()))",
            )
            .bind(user.id)
            .fetch_one(&mut *self.conn)
            .await?;
            if has_status {
                return Err(Unsupported("user status").into());
            }
        }
        u.insert("id".into(), json!(user.id));
        u.insert("username".into(), json!(user.username));
        if enable_names {
            u.insert("name".into(), json!(user.name));
        }
        u.insert(
            "avatar_template".into(),
            json!(self.avatar_template(&user.lookup_user()).await?),
        );
        u.insert(
            "last_posted_at".into(),
            json!(user.last_posted_at.map(time_json)),
        );
        u.insert(
            "last_seen_at".into(),
            json!(user.last_seen_at.map(time_json)),
        );
        u.insert("created_at".into(), json!(time_json(user.created_at)));
        u.insert("ignored".into(), json!(false));
        u.insert("muted".into(), json!(false));
        u.insert("can_ignore_user".into(), json!(false));
        u.insert("can_mute_user".into(), json!(false));
        u.insert("can_send_private_messages".into(), json!(false));
        u.insert("can_send_private_message_to_user".into(), json!(false));
        u.insert("trust_level".into(), json!(user.trust_level));
        u.insert("moderator".into(), json!(user.moderator));
        u.insert("admin".into(), json!(user.admin));
        u.insert("title".into(), json!(user.title));
        if user.suspended() || user.silenced() {
            return Err(
                Unsupported("suspended or silenced users (staff reason sanitizing)").into(),
            );
        }
        u.insert(
            "badge_count".into(),
            json!(user.distinct_badge_count.unwrap_or(0)),
        );
        let profile_details = !user.restrict_fields(self.settings)?;
        if profile_details {
            let fields = self.user_fields(user.id).await?;
            if !fields.is_empty() {
                u.insert("user_fields".into(), Value::Object(fields));
            }
        }
        u.insert("custom_fields".into(), self.custom_fields(user.id).await?);
        u.insert("time_read".into(), json!(user.time_read.unwrap_or(0)));
        u.insert(
            "recent_time_read".into(),
            json!(self.recent_time_read(user.id).await?),
        );
        u.insert("primary_group_id".into(), json!(user.primary_group_id));
        u.insert(
            "primary_group_name".into(),
            json!(self.group_name(user.primary_group_id).await?),
        );
        u.insert("flair_group_id".into(), json!(user.flair_group_id));
        let flair = match user.flair_group_id {
            Some(id) => crate::groups::load(&mut *self.conn, &[id])
                .await?
                .remove(&id),
            None => None,
        };
        u.insert(
            "flair_name".into(),
            json!(flair.as_ref().map(|g| g.name.clone())),
        );
        u.insert(
            "flair_url".into(),
            json!(flair.as_ref().map(|g| g.flair_url()).transpose()?.flatten()),
        );
        u.insert(
            "flair_bg_color".into(),
            json!(flair.as_ref().and_then(|g| g.flair_bg_color.clone())),
        );
        u.insert(
            "flair_color".into(),
            json!(flair.as_ref().and_then(|g| g.flair_color.clone())),
        );
        if user.featured_topic_id.is_some() {
            return Err(Unsupported("featured_topic on profiles").into());
        }
        if self
            .settings
            .get("display_local_time_in_user_card")?
            .truthy()
        {
            return Err(Unsupported("display_local_time_in_user_card").into());
        }
        // untrusted_attributes: only with profile details and a value.
        let bio = user.bio_cooked.as_deref().filter(|b| !b.is_empty());
        if profile_details && bio.is_some() {
            return Err(Unsupported("profile bios (bio_excerpt via PrettyText.excerpt)").into());
        }
        if profile_details {
            if let Some(w) = user.website.as_deref().filter(|w| !w.is_empty()) {
                u.insert("website".into(), json!(w));
                if let Some(name) = website_name(w) {
                    u.insert("website_name".into(), json!(name));
                }
            }
            if let Some(l) = user.location.as_deref().filter(|l| !l.is_empty()) {
                u.insert("location".into(), json!(l));
            }
            if let Some(url) = self.upload_url(user.card_background_upload_id).await? {
                u.insert("card_background_upload_url".into(), json!(url));
            }
            if let Some(raw) = user.bio_raw.as_deref().filter(|b| !b.is_empty()) {
                u.insert("bio_raw".into(), json!(raw));
            }
        }
        u.insert("can_edit".into(), json!(false));
        u.insert("can_edit_username".into(), json!(false));
        u.insert("can_edit_email".into(), json!(false));
        u.insert("can_edit_name".into(), json!(false));
        u.insert("uploaded_avatar_id".into(), json!(user.uploaded_avatar_id));
        u.insert("pending_count".into(), json!(0));
        u.insert("profile_view_count".into(), json!(user.views.unwrap_or(0)));
        if profile_details {
            if let Some(url) = self.upload_url(user.profile_background_upload_id).await? {
                u.insert("profile_background_upload_url".into(), json!(url));
            }
        }
        u.insert("can_upload_profile_header".into(), json!(false));
        u.insert("can_upload_user_card_background".into(), json!(false));
        // The gravatar/custom avatar ids leak to anonymous readers because
        // their include_ predicates are redefined after private_attributes.
        let avatars: Option<(Option<i32>, Option<i32>)> = sqlx::query_as(
            "SELECT gravatar_upload_id, custom_upload_id FROM user_avatars WHERE user_id = $1",
        )
        .bind(user.id)
        .fetch_optional(&mut *self.conn)
        .await?;
        if let Some((gravatar, custom)) = avatars {
            if let Some(id) = gravatar {
                u.insert("gravatar_avatar_upload_id".into(), json!(id));
                u.insert(
                    "gravatar_avatar_template".into(),
                    json!(avatar::class_avatar_template(
                        self.urls,
                        &user.username,
                        Some(id)
                    )?),
                );
            }
            if let Some(id) = custom {
                u.insert("custom_avatar_upload_id".into(), json!(id));
                u.insert(
                    "custom_avatar_template".into(),
                    json!(avatar::class_avatar_template(
                        self.urls,
                        &user.username,
                        Some(id)
                    )?),
                );
            }
        }
        u.insert(
            "featured_user_badge_ids".into(),
            json!(featured.iter().map(|b| b["id"].clone()).collect::<Vec<_>>()),
        );
        u.insert("invited_by".into(), self.invited_by(user).await?);
        let groups: Vec<crate::groups::BasicGroup> = sqlx::query_as(&format!(
            "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
             JOIN group_users gu ON gu.group_id = g.id \
             WHERE gu.user_id = $1 AND g.id > 0 AND g.id NOT IN (4, 5) \
             AND g.visibility_level = 0 AND g.members_visibility_level = 0 \
             ORDER BY g.id ASC, g.name ASC",
            crate::groups::BASIC_GROUP_COLUMNS
        ))
        .bind(user.id)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut group_json = Vec::new();
        for g in &groups {
            group_json.push(g.json(self.i18n)?);
        }
        u.insert("groups".into(), Value::Array(group_json));
        out.insert("user".into(), Value::Object(u));
        Ok(Value::Object(out))
    }

    async fn group_name(&mut self, id: Option<i32>) -> Result<Option<String>, UsersError> {
        let Some(id) = id else {
            return Ok(None);
        };
        Ok(sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    async fn upload_url(&mut self, id: Option<i32>) -> Result<Option<String>, UsersError> {
        let Some(id) = id else {
            return Ok(None);
        };
        if self.urls.config.globals.cdn_url().is_some() {
            return Err(Unsupported("profile uploads behind a CDN").into());
        }
        Ok(sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *self.conn)
            .await?)
    }

    /// `user_fields`: the fields shown on the profile or card, keyed by id.
    async fn user_fields(&mut self, user_id: i32) -> Result<Map<String, Value>, UsersError> {
        let fields: Vec<(i32, String)> = sqlx::query_as(
            "SELECT id, field_type FROM user_fields WHERE show_on_profile OR show_on_user_card ORDER BY position, id",
        )
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out = Map::new();
        for (id, field_type) in fields {
            if field_type == "confirm" {
                return Err(Unsupported("confirm user fields").into());
            }
            let value: Option<String> = sqlx::query_scalar(
                "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 LIMIT 1",
            )
            .bind(user_id)
            .bind(format!("user_field_{id}"))
            .fetch_optional(&mut *self.conn)
            .await?;
            out.insert(id.to_string(), json!(value));
        }
        Ok(out)
    }

    /// `custom_fields`: only the names in public_user_custom_fields.
    async fn custom_fields(&mut self, user_id: i32) -> Result<Value, UsersError> {
        let mut out = Map::new();
        let names: Vec<String> = self
            .settings
            .get("public_user_custom_fields")?
            .to_s()
            .split('|')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for name in names {
            let value: Option<String> = sqlx::query_scalar(
                "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id LIMIT 1",
            )
            .bind(user_id)
            .bind(&name)
            .fetch_optional(&mut *self.conn)
            .await?;
            if let Some(v) = value {
                out.insert(name, json!(v));
            }
        }
        Ok(Value::Object(out))
    }

    /// `UserStat#recent_time_read`: the last 60 days of visits.
    async fn recent_time_read(&mut self, user_id: i32) -> Result<i64, UsersError> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(SUM(time_read), 0)::bigint FROM user_visits \
             WHERE user_id = $1 AND visited_at >= now() - interval '60 days'",
        )
        .bind(user_id)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// `invited_by`: the inviter of the invite redeemed at sign-up, as a
    /// BasicUser.
    async fn invited_by(&mut self, user: &User) -> Result<Value, UsersError> {
        let inviter: Option<i32> = sqlx::query_scalar(
            "SELECT invites.invited_by_id FROM invites \
             JOIN invited_users ON invited_users.invite_id = invites.id \
             WHERE invited_users.user_id = $1 AND invited_users.redeemed_at <= $2 \
             ORDER BY invites.id LIMIT 1",
        )
        .bind(user.id)
        .bind(user.created_at + chrono::Duration::seconds(5))
        .fetch_optional(&mut *self.conn)
        .await?;
        let Some(id) = inviter else {
            return Ok(Value::Null);
        };
        let users = self.list().user_lookup_for(&[id]).await?;
        let Some(u) = users.get(&id) else {
            return Ok(Value::Null);
        };
        let mut out = Map::new();
        out.insert("id".into(), json!(u.id));
        out.insert("username".into(), json!(u.username));
        if self.settings.get("enable_names")?.truthy() {
            out.insert("name".into(), json!(u.name));
        }
        let template = self.avatar_template(u).await?;
        out.insert("avatar_template".into(), json!(template));
        Ok(Value::Object(out))
    }

    /// `User#featured_user_badges(limit)`: one row per badge with the grant
    /// count, by featured rank; each serialized (UserBadgeSerializer) with
    /// its badge, badge type and users side-loaded.
    async fn featured_badges(
        &mut self,
        user_id: i32,
        rank_limit: i64,
        side: &mut SideLoads,
    ) -> Result<Vec<Value>, UsersError> {
        let rows: Vec<UserBadgeRow> = sqlx::query_as(
            "SELECT MAX(user_badges.id) AS id, MAX(user_badges.badge_id) AS badge_id, \
                    MAX(user_badges.user_id) AS user_id, MAX(user_badges.granted_at) AS granted_at, \
                    MAX(user_badges.granted_by_id) AS granted_by_id, \
                    MAX(user_badges.created_at) AS created_at, COUNT(*) AS count \
             FROM user_badges \
             WHERE user_badges.user_id = $1 \
             AND (user_badges.badge_id IN (SELECT id FROM badges WHERE enabled)) \
             AND (featured_rank <= $2) \
             GROUP BY user_badges.badge_id, user_badges.user_id \
             ORDER BY MAX(featured_rank) ASC",
        )
        .bind(user_id)
        .bind(rank_limit as i32)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(self.user_badge(&row, side).await?);
        }
        Ok(out)
    }

    async fn user_badge(
        &mut self,
        row: &UserBadgeRow,
        side: &mut SideLoads,
    ) -> Result<Value, UsersError> {
        let mut ub = Map::new();
        ub.insert("id".into(), json!(row.id));
        ub.insert("granted_at".into(), json!(time_json(row.granted_at)));
        ub.insert("created_at".into(), json!(time_json(row.created_at)));
        ub.insert("count".into(), json!(row.count));
        ub.insert("badge_id".into(), json!(row.badge_id));
        ub.insert("user_id".into(), json!(row.user_id));
        ub.insert("granted_by_id".into(), json!(row.granted_by_id));

        if !side.badge_ids.contains(&row.badge_id) {
            let badge: BadgeRow = sqlx::query_as(
                "SELECT b.id, b.name, b.description, b.grant_count, b.allow_title, b.multiple_grant, \
                        b.icon, u.url AS image_upload_url, b.listable, b.enabled, b.badge_grouping_id, \
                        b.system, b.show_in_post_header, b.badge_type_id, bt.name AS badge_type_name \
                 FROM badges b JOIN badge_types bt ON bt.id = b.badge_type_id \
                 LEFT JOIN uploads u ON u.id = b.image_upload_id WHERE b.id = $1",
            )
            .bind(row.badge_id)
            .fetch_one(&mut *self.conn)
            .await?;
            side.badge_ids.push(row.badge_id);
            side.badges.push(self.badge_json(&badge)?);
            if !side.badge_type_ids.contains(&badge.badge_type_id) {
                side.badge_type_ids.push(badge.badge_type_id);
                side.badge_types.push(json!({
                    "id": badge.badge_type_id,
                    "name": badge.badge_type_name,
                    "sort_order": 10 - badge.badge_type_id,
                }));
            }
        }
        for id in [row.user_id, row.granted_by_id] {
            if side.user_ids.contains(&id) {
                continue;
            }
            let users = self.list().user_lookup_for(&[id]).await?;
            let Some(u) = users.get(&id) else {
                continue;
            };
            let groups = crate::groups::load(
                &mut *self.conn,
                &[u.primary_group_id, u.flair_group_id]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>(),
            )
            .await?;
            let logo = self.list().logo_small_url().await?;
            let json = self.list().serialize_user(u, logo.as_deref(), &groups)?;
            side.user_ids.push(id);
            side.users.push(json);
        }
        Ok(Value::Object(ub))
    }

    /// BadgeSerializer: name and description through the badge locale keys.
    fn badge_json(&self, b: &BadgeRow) -> Result<Value, UsersError> {
        let key = b.name.to_lowercase().replace(' ', "_");
        let name = self
            .i18n
            .t(&format!("badges.{key}.name"))
            .unwrap_or(&b.name)
            .to_string();
        let max_likes = self.settings.get("max_likes_per_day")?.to_s();
        let description = self
            .i18n
            .t_with(
                &format!("badges.{key}.description"),
                &[
                    ("base_uri", self.base_path),
                    ("max_likes_per_day", &max_likes),
                ],
            )
            .or_else(|| b.description.clone())
            .unwrap_or_default();
        if self.urls.config.globals.cdn_url().is_some() && b.image_upload_url.is_some() {
            return Err(Unsupported("badge images behind a CDN").into());
        }
        Ok(json!({
            "id": b.id,
            "name": name,
            "description": description,
            "grant_count": b.grant_count,
            "allow_title": b.allow_title,
            "multiple_grant": b.multiple_grant,
            "icon": b.icon,
            "image_url": b.image_upload_url,
            "listable": b.listable,
            "enabled": b.enabled,
            "badge_grouping_id": b.badge_grouping_id,
            "system": b.system,
            "slug": slug_for(&name),
            "manually_grantable": !b.system,
            "show_in_post_header": b.show_in_post_header,
            "badge_type_id": b.badge_type_id,
        }))
    }

    /// users#summary: `{topics, [badges, badge_types, users], user_summary}`.
    pub async fn summary(&mut self, user: &User) -> Result<Value, UsersError> {
        let secured = "topics.deleted_at IS NULL AND (topics.category_id IS NULL OR topics.category_id IN (SELECT id FROM categories WHERE NOT read_restricted))";
        let listable_visible = "topics.deleted_at IS NULL AND topics.archetype != 'private_message' AND topics.visible = TRUE";
        let post_query = format!(
            "FROM posts JOIN topics ON topics.deleted_at IS NULL AND topics.id = posts.topic_id \
             WHERE posts.deleted_at IS NULL AND posts.post_type IN (1) AND {listable_visible} AND {secured} \
             AND posts.user_id = $1"
        );
        let mut side = SideLoads::default();
        let mut topic_ids: Vec<i32> = Vec::new();
        let mut topics: Vec<Value> = Vec::new();

        // topics: the user's own, most liked first.
        let own: Vec<SummaryTopic> = sqlx::query_as(&format!(
            "SELECT {SUMMARY_TOPIC_COLUMNS} FROM topics WHERE {secured} AND {listable_visible} \
             AND topics.user_id = $1 ORDER BY like_count DESC, created_at DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let own_ids: Vec<i32> = own.iter().map(|t| t.id).collect();
        for t in &own {
            if !topic_ids.contains(&t.id) {
                topic_ids.push(t.id);
                topics.push(self.summary_topic(t)?);
            }
        }

        // replies: side-load their topics too.
        let replies: Vec<(i32, i32, NaiveDateTime, i32)> = sqlx::query_as(&format!(
            "SELECT posts.post_number, posts.like_count, posts.created_at, posts.topic_id {post_query} \
             AND (post_number > 1) ORDER BY posts.like_count DESC, posts.created_at DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut replies_json = Vec::new();
        for (post_number, like_count, created_at, topic_id) in &replies {
            replies_json.push(json!({
                "post_number": post_number,
                "like_count": like_count,
                "created_at": time_json(*created_at),
                "topic_id": topic_id,
            }));
            self.side_load_topic(*topic_id, &mut topic_ids, &mut topics)
                .await?;
        }

        // links
        let links: Vec<(String, Option<String>, i32, i32, i32)> = sqlx::query_as(&format!(
            "SELECT topic_links.url, topic_links.title, topic_links.clicks, posts.post_number, topic_links.topic_id \
             FROM topic_links \
             INNER JOIN topics ON topics.deleted_at IS NULL AND topics.id = topic_links.topic_id \
             INNER JOIN posts ON posts.deleted_at IS NULL AND posts.id = topic_links.post_id \
             WHERE posts.user_id = $1 AND posts.hidden = FALSE AND posts.post_type IN (1, 2, 3) \
             AND {listable_visible} AND {secured} \
             AND topic_links.user_id = $1 AND topic_links.internal = FALSE \
             AND topic_links.reflection = FALSE AND topic_links.quote = FALSE \
             ORDER BY clicks DESC, topic_links.created_at DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut links_json = Vec::new();
        for (url, title, clicks, post_number, topic_id) in &links {
            links_json.push(json!({
                "url": url,
                "title": title,
                "clicks": clicks,
                "post_number": post_number,
                "topic_id": topic_id,
            }));
            self.side_load_topic(*topic_id, &mut topic_ids, &mut topics)
                .await?;
        }

        let liked_join = format!(
            "FROM user_actions \
             INNER JOIN topics ON topics.deleted_at IS NULL AND topics.id = user_actions.target_topic_id \
             INNER JOIN posts ON posts.deleted_at IS NULL AND posts.id = user_actions.target_post_id \
             WHERE {listable_visible} AND {secured}"
        );
        let most_liked_by: Vec<(i32, i64)> = sqlx::query_as(&format!(
            "SELECT user_actions.acting_user_id, COUNT(*) {liked_join} \
             AND user_actions.user_id = $1 AND user_actions.action_type = {WAS_LIKED} \
             GROUP BY user_actions.acting_user_id ORDER BY COUNT(*) DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let most_liked: Vec<(i32, i64)> = sqlx::query_as(&format!(
            "SELECT user_actions.user_id, COUNT(*) {liked_join} \
             AND user_actions.action_type = {WAS_LIKED} AND user_actions.acting_user_id = $1 \
             GROUP BY user_actions.user_id ORDER BY COUNT(*) DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let most_replied: Vec<(i32, i64)> = sqlx::query_as(&format!(
            "SELECT replies.user_id, COUNT(*) FROM posts \
             INNER JOIN topics topics_posts ON topics_posts.deleted_at IS NULL AND topics_posts.id = posts.topic_id \
             JOIN posts replies ON posts.topic_id = replies.topic_id AND posts.reply_to_post_number = replies.post_number \
             JOIN topics ON replies.topic_id = topics.id AND topics.archetype <> 'private_message' AND replies.post_type IN (1) \
             WHERE posts.deleted_at IS NULL AND posts.post_type IN (1) AND {listable_visible} AND {secured} \
             AND posts.user_id = $1 AND (replies.user_id <> posts.user_id) \
             GROUP BY replies.user_id ORDER BY COUNT(*) DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let most_liked_by_json = self.users_with_counts(&most_liked_by).await?;
        let most_liked_json = self.users_with_counts(&most_liked).await?;
        let most_replied_json = self.users_with_counts(&most_replied).await?;

        let badges = if self.settings.get("enable_badges")?.truthy() {
            Some(self.featured_badges(user.id, MAX_BADGES, &mut side).await?)
        } else {
            None
        };

        // top_categories
        let ids: Vec<Option<i32>> = sqlx::query_scalar(&format!(
            "SELECT topics.category_id {post_query} GROUP BY topics.category_id ORDER BY count(*) DESC LIMIT $2"
        ))
        .bind(user.id)
        .bind(MAX_SUMMARY_RESULTS)
        .fetch_all(&mut *self.conn)
        .await?;
        let ids: Vec<i32> = ids.into_iter().flatten().collect();
        let mut top_categories = Vec::new();
        if !ids.is_empty() {
            let rows: Vec<TopCategory> = sqlx::query_as(
                "SELECT id, name, color, text_color, style_type, icon, emoji, slug, read_restricted, \
                        parent_category_id FROM categories WHERE id = ANY($1) ORDER BY id",
            )
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?;
            let post_counts: Vec<(Option<i32>, i64)> = sqlx::query_as(&format!(
                "SELECT topics.category_id, COUNT(*) {post_query} AND (post_number > 1) \
                 AND (topics.category_id = ANY($2)) GROUP BY topics.category_id"
            ))
            .bind(user.id)
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?;
            let topic_counts: Vec<(Option<i32>, i64)> = sqlx::query_as(&format!(
                "SELECT category_id, COUNT(*) FROM topics WHERE {listable_visible} AND {secured} \
                 AND (topics.category_id = ANY($2)) AND topics.user_id = $1 GROUP BY topics.category_id"
            ))
            .bind(user.id)
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?;
            let count = |list: &[(Option<i32>, i64)], id: i32| {
                list.iter()
                    .find(|(c, _)| *c == Some(id))
                    .map(|(_, n)| *n)
                    .unwrap_or(0)
            };
            let mut entries: Vec<(i64, Value)> = rows
                .iter()
                .map(|c| {
                    let topic_count = count(&topic_counts, c.id);
                    let post_count = count(&post_counts, c.id);
                    (
                        topic_count + post_count,
                        json!({
                            "topic_count": topic_count,
                            "post_count": post_count,
                            "id": c.id,
                            "name": c.name,
                            "color": c.color,
                            "text_color": c.text_color,
                            "style_type": crate::categories::style_type(c.style_type),
                            "icon": c.icon,
                            "emoji": c.emoji,
                            "slug": c.slug,
                            "read_restricted": c.read_restricted,
                            "parent_category_id": c.parent_category_id,
                        }),
                    )
                })
                .collect();
            // sort_by { -(post_count + topic_count) }; ties keep id order.
            entries.sort_by_key(|(total, _)| -total);
            top_categories = entries.into_iter().map(|(_, v)| v).collect();
        }

        let mut out = Map::new();
        out.insert("topics".into(), Value::Array(topics));
        if !side.badges.is_empty() {
            out.insert("badges".into(), Value::Array(side.badges.clone()));
            out.insert("badge_types".into(), Value::Array(side.badge_types.clone()));
            out.insert("users".into(), Value::Array(side.users.clone()));
        }
        let mut s = Map::new();
        s.insert("likes_given".into(), json!(user.likes_given.unwrap_or(0)));
        s.insert(
            "likes_received".into(),
            json!(user.likes_received.unwrap_or(0)),
        );
        s.insert(
            "topics_entered".into(),
            json!(user.topics_entered.unwrap_or(0)),
        );
        s.insert(
            "posts_read_count".into(),
            json!(user.posts_read_count.unwrap_or(0)),
        );
        s.insert("days_visited".into(), json!(user.days_visited.unwrap_or(0)));
        s.insert("topic_count".into(), json!(user.topic_count.unwrap_or(0)));
        s.insert("post_count".into(), json!(user.post_count.unwrap_or(0)));
        s.insert("time_read".into(), json!(user.time_read.unwrap_or(0)));
        s.insert(
            "recent_time_read".into(),
            json!(self.recent_time_read(user.id).await?),
        );
        s.insert("can_see_summary_stats".into(), json!(true));
        s.insert(
            "can_see_user_actions".into(),
            json!(!self.settings.get("hide_user_activity_tab")?.truthy()),
        );
        s.insert("topic_ids".into(), json!(own_ids));
        s.insert("replies".into(), Value::Array(replies_json));
        s.insert("links".into(), Value::Array(links_json));
        s.insert(
            "most_liked_by_users".into(),
            Value::Array(most_liked_by_json),
        );
        s.insert("most_liked_users".into(), Value::Array(most_liked_json));
        s.insert(
            "most_replied_to_users".into(),
            Value::Array(most_replied_json),
        );
        if let Some(b) = badges {
            s.insert("badges".into(), Value::Array(b));
        }
        s.insert("top_categories".into(), Value::Array(top_categories));
        out.insert("user_summary".into(), Value::Object(s));
        Ok(Value::Object(out))
    }

    async fn side_load_topic(
        &mut self,
        topic_id: i32,
        topic_ids: &mut Vec<i32>,
        topics: &mut Vec<Value>,
    ) -> Result<(), UsersError> {
        if topic_ids.contains(&topic_id) {
            return Ok(());
        }
        let t: Option<SummaryTopic> = sqlx::query_as(&format!(
            "SELECT {SUMMARY_TOPIC_COLUMNS} FROM topics WHERE topics.id = $1"
        ))
        .bind(topic_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        if let Some(t) = t {
            topic_ids.push(topic_id);
            topics.push(self.summary_topic(&t)?);
        }
        Ok(())
    }

    /// UserSummarySerializer::TopicSerializer
    fn summary_topic(&self, t: &SummaryTopic) -> Result<Value, UsersError> {
        let Some(fancy_title) = &t.fancy_title else {
            return Err(Unsupported("topics without a stored fancy_title").into());
        };
        Ok(json!({
            "fancy_title": fancy_title,
            "id": t.id,
            "title": t.title,
            "slug": t.slug,
            "posts_count": t.posts_count,
            "category_id": t.category_id,
            "like_count": t.like_count,
            "created_at": time_json(t.created_at),
        }))
    }

    /// `user_counts` + UserWithCountSerializer: users missing from the
    /// lookup are dropped; ordered by count.
    async fn users_with_counts(&mut self, counts: &[(i32, i64)]) -> Result<Vec<Value>, UsersError> {
        if counts.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<i32> = counts.iter().map(|(id, _)| *id).collect();
        let users = self.list().user_lookup_for(&ids).await?;
        let group_ids: Vec<i32> = users
            .values()
            .flat_map(|u| [u.primary_group_id, u.flair_group_id])
            .flatten()
            .collect();
        let groups = crate::groups::load(&mut *self.conn, &group_ids).await?;
        let enable_names = self.settings.get("enable_names")?.truthy();
        let mut out = Vec::new();
        for (id, count) in counts {
            let Some(u) = users.get(id) else {
                continue;
            };
            let mut j = Map::new();
            j.insert("id".into(), json!(u.id));
            j.insert("username".into(), json!(u.username));
            if enable_names {
                j.insert("name".into(), json!(u.name));
            }
            j.insert("count".into(), json!(count));
            // The class-level template: the system user gets no logo here.
            j.insert(
                "avatar_template".into(),
                json!(avatar::class_avatar_template(
                    self.urls,
                    &u.username,
                    u.uploaded_avatar_id
                )?),
            );
            j.insert("admin".into(), json!(u.admin));
            j.insert("moderator".into(), json!(u.moderator));
            j.insert("trust_level".into(), json!(u.trust_level));
            let flair = u.flair_group_id.and_then(|g| groups.get(&g));
            j.insert("flair_name".into(), json!(flair.map(|g| g.name.clone())));
            j.insert(
                "flair_url".into(),
                json!(flair.map(|g| g.flair_url()).transpose()?.flatten()),
            );
            j.insert(
                "flair_bg_color".into(),
                json!(flair.and_then(|g| g.flair_bg_color.clone())),
            );
            j.insert(
                "flair_color".into(),
                json!(flair.and_then(|g| g.flair_color.clone())),
            );
            j.insert(
                "primary_group_name".into(),
                json!(
                    u.primary_group_id
                        .and_then(|g| groups.get(&g))
                        .map(|g| g.name.clone())
                ),
            );
            out.push(Value::Object(j));
        }
        Ok(out)
    }

    /// `UserAction.stream` + UserActionSerializer for an anonymous reader.
    pub async fn actions(
        &mut self,
        user: &User,
        action_types: &[i32],
        offset: i64,
        limit: i64,
        acting_username: Option<&str>,
    ) -> Result<Vec<Value>, UsersError> {
        let mut sql = String::from(
            "SELECT a.id, t.title, t.slug, a.action_type, a.created_at, t.id AS topic_id, \
                    t.closed AS topic_closed, t.archived AS topic_archived, \
                    a.user_id AS target_user_id, au.name AS target_name, au.username AS target_username, \
                    coalesce(p.post_number, 1) AS post_number, p.id AS post_id, p.reply_to_post_number, \
                    pu.username, pu.name, pu.id AS user_id, pu.uploaded_avatar_id, \
                    u.id AS acting_user_id, u.name AS acting_name, u.username AS acting_username, \
                    u.uploaded_avatar_id AS acting_uploaded_avatar_id, \
                    coalesce(p.cooked, p2.cooked) AS cooked, \
                    CASE WHEN coalesce(p.deleted_at, p2.deleted_at, t.deleted_at) IS NULL THEN false ELSE true END AS deleted, \
                    p.hidden, p.post_type, p.action_code, pc.value AS action_code_who, \
                    pc2.value AS action_code_path, p.edit_reason, t.category_id \
             FROM user_actions AS a \
             JOIN topics t ON t.id = a.target_topic_id \
             LEFT JOIN posts p ON p.id = a.target_post_id \
             JOIN posts p2 ON p2.topic_id = a.target_topic_id AND p2.post_number = 1 \
             JOIN users u ON u.id = a.acting_user_id \
             JOIN users pu ON pu.id = COALESCE(p.user_id, t.user_id) \
             JOIN users au ON au.id = a.user_id \
             LEFT JOIN categories c ON c.id = t.category_id \
             LEFT JOIN post_custom_fields pc ON pc.post_id = a.target_post_id AND pc.name = 'action_code_who' \
             LEFT JOIN post_custom_fields pc2 ON pc2.post_id = a.target_post_id AND pc2.name = 'action_code_path' \
             WHERE (t.deleted_at IS NULL) \
             AND (p.deleted_at IS NULL AND p2.deleted_at IS NULL) \
             AND (NOT COALESCE(p.hidden, p2.hidden, false)) \
             AND (COALESCE(p.post_type, p2.post_type) IN (1, 2, 3)) \
             AND (t.visible) AND (t.archetype <> 'private_message') \
             AND ((c.read_restricted IS NULL OR NOT c.read_restricted)) \
             AND (a.user_id = $1)",
        );
        if !action_types.is_empty() {
            sql.push_str(" AND (a.action_type = ANY($4))");
        }
        if acting_username.is_some() {
            sql.push_str(" AND (u.username_lower = $5)");
        }
        if !self.settings.get("enable_mentions")?.truthy() {
            sql.push_str(&format!(" AND (a.action_type <> {MENTION})"));
        }
        sql.push_str(" ORDER BY a.created_at DESC OFFSET $2 LIMIT $3");
        let rows: Vec<ActionRow> = sqlx::query_as(&sql)
            .bind(user.id)
            .bind(offset)
            .bind(limit)
            .bind(action_types)
            .bind(acting_username.map(|u| u.to_lowercase()))
            .fetch_all(&mut *self.conn)
            .await?;
        let enable_names = self.settings.get("enable_names")?.truthy();
        let mut out = Vec::new();
        for r in &rows {
            let mut j = Map::new();
            let cooked = r.cooked.as_deref().unwrap_or("");
            let excerpt = crate::excerpt::excerpt(
                cooked,
                300,
                &crate::excerpt::Options {
                    keep_emoji_images: true,
                    ..Default::default()
                },
            );
            j.insert("excerpt".into(), json!(excerpt));
            if cooked.chars().count() > 300 {
                j.insert("truncated".into(), json!(true));
            }
            j.insert("action_type".into(), json!(r.action_type));
            j.insert("created_at".into(), json!(time_json(r.created_at)));
            j.insert(
                "avatar_template".into(),
                json!(avatar::class_avatar_template(
                    self.urls,
                    &r.username,
                    r.uploaded_avatar_id
                )?),
            );
            j.insert(
                "acting_avatar_template".into(),
                json!(avatar::class_avatar_template(
                    self.urls,
                    &r.acting_username,
                    r.acting_uploaded_avatar_id
                )?),
            );
            if r.title.as_deref().is_some_and(|t| !t.is_empty()) {
                let Some(slug) = &r.slug else {
                    return Err(Unsupported("topics without a stored slug (Slug.for)").into());
                };
                j.insert("slug".into(), json!(slug));
            }
            j.insert("topic_id".into(), json!(r.topic_id));
            j.insert("target_user_id".into(), json!(r.target_user_id));
            if enable_names {
                j.insert("target_name".into(), json!(r.target_name));
            }
            j.insert("target_username".into(), json!(r.target_username));
            j.insert("post_number".into(), json!(r.post_number));
            j.insert("post_id".into(), json!(r.post_id));
            if r.action_type == REPLY {
                j.insert("reply_to_post_number".into(), json!(r.reply_to_post_number));
            }
            j.insert("username".into(), json!(r.username));
            if enable_names {
                j.insert("name".into(), json!(r.name));
            }
            j.insert("user_id".into(), json!(r.user_id));
            j.insert("acting_username".into(), json!(r.acting_username));
            if enable_names {
                j.insert("acting_name".into(), json!(r.acting_name));
            }
            j.insert("acting_user_id".into(), json!(r.acting_user_id));
            j.insert("title".into(), json!(r.title));
            j.insert("deleted".into(), json!(r.deleted));
            j.insert("hidden".into(), json!(r.hidden));
            j.insert("post_type".into(), json!(r.post_type));
            j.insert("action_code".into(), json!(r.action_code));
            if let Some(who) = &r.action_code_who {
                j.insert("action_code_who".into(), json!(who));
            }
            if let Some(path) = &r.action_code_path {
                j.insert("action_code_path".into(), json!(path));
            }
            if r.action_type == EDIT {
                j.insert("edit_reason".into(), json!(r.edit_reason));
            }
            j.insert("category_id".into(), json!(r.category_id));
            j.insert("closed".into(), json!(r.topic_closed));
            j.insert("archived".into(), json!(r.topic_archived));
            out.push(Value::Object(j));
        }
        Ok(out)
    }

    /// `UserProfileView.add(profile_id, ip, nil)`: one view per profile, IP
    /// and day (Rails keeps that window in redis for
    /// user_profile_view_duration_hours); bumps `user_profiles.views`.
    pub async fn track_view(&mut self, user: &User, ip: &str) -> Result<(), UsersError> {
        let Some(profile_id) = user.profile_id else {
            return Ok(());
        };
        let hours = self
            .settings
            .get("user_profile_view_duration_hours")?
            .to_i()
            .max(1);
        let seen: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_profile_views \
             WHERE user_profile_id = $1 AND ip_address = $2::inet AND user_id IS NULL \
             AND viewed_at::date = now()::date AND viewed_at >= now() - ($3 * interval '1 hour'))",
        )
        .bind(profile_id)
        .bind(ip)
        .bind(hours as f64)
        .fetch_one(&mut *self.conn)
        .await?;
        if seen {
            return Ok(());
        }
        let inserted = sqlx::query(
            "INSERT INTO user_profile_views (user_profile_id, ip_address, viewed_at, user_id) \
             VALUES ($1, $2::inet, now(), NULL)",
        )
        .bind(profile_id)
        .bind(ip)
        .execute(&mut *self.conn)
        .await?;
        if inserted.rows_affected() == 1 {
            sqlx::query("UPDATE user_profiles SET views = views + 1 WHERE user_id = $1")
                .bind(profile_id)
                .execute(&mut *self.conn)
                .await?;
        }
        Ok(())
    }
}

#[derive(Debug, sqlx::FromRow)]
struct SummaryTopic {
    id: i32,
    title: String,
    fancy_title: Option<String>,
    slug: Option<String>,
    posts_count: i32,
    category_id: Option<i32>,
    like_count: i32,
    created_at: NaiveDateTime,
}

const SUMMARY_TOPIC_COLUMNS: &str = "topics.id, topics.title, topics.fancy_title, topics.slug, \
    topics.posts_count, topics.category_id, topics.like_count, topics.created_at";

#[derive(Debug, sqlx::FromRow)]
struct TopCategory {
    id: i32,
    name: String,
    color: String,
    text_color: String,
    style_type: i32,
    icon: Option<String>,
    emoji: Option<String>,
    slug: String,
    read_restricted: bool,
    parent_category_id: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct ActionRow {
    action_type: i32,
    created_at: NaiveDateTime,
    title: Option<String>,
    slug: Option<String>,
    topic_id: i32,
    topic_closed: bool,
    topic_archived: bool,
    target_user_id: i32,
    target_name: Option<String>,
    target_username: String,
    post_number: i32,
    post_id: Option<i32>,
    reply_to_post_number: Option<i32>,
    username: String,
    name: Option<String>,
    user_id: i32,
    uploaded_avatar_id: Option<i32>,
    acting_user_id: i32,
    acting_name: Option<String>,
    acting_username: String,
    acting_uploaded_avatar_id: Option<i32>,
    cooked: Option<String>,
    deleted: bool,
    hidden: Option<bool>,
    post_type: Option<i32>,
    action_code: Option<String>,
    action_code_who: Option<String>,
    action_code_path: Option<String>,
    edit_reason: Option<String>,
    category_id: Option<i32>,
}

/// `Slug.for(name, "-")` for badge names: ASCII-lowercased words joined
/// by hyphens (the default `ascii` method).
fn slug_for(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "badge".to_string()
    } else {
        out
    }
}

/// `UserSerializer#website_name`: the host without `www.` plus the path.
fn website_name(website: &str) -> Option<String> {
    let rest = website
        .strip_prefix("https://")
        .or_else(|| website.strip_prefix("http://"))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if host.is_empty() {
        return None;
    }
    let host = host.strip_prefix("www.").unwrap_or(host);
    Some(format!("{host}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_website_names() {
        assert_eq!(slug_for("Basic"), "basic");
        assert_eq!(slug_for("First Like"), "first-like");
        assert_eq!(
            website_name("https://www.example.com/a/b"),
            Some("example.com/a/b".into())
        );
        assert_eq!(
            website_name("http://example.com"),
            Some("example.com".into())
        );
        assert_eq!(website_name("nope"), None);
    }
}
