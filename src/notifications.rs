//! Port of NotificationsController#index, Notification.prioritized_list,
//! the accessible-topic and disabled-badge filters, and
//! NotificationSerializer.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::{Guardian, GuardianError};
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list::time_json;
use crate::url::{UrlError, Urls};

/// `Notification.types`
pub const TYPES: &[(&str, i32)] = &[
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

const GRANTED_BADGE: i32 = 12;
/// `Notification.like_types`
const LIKE_TYPES: &str = "5,19,25";

#[derive(Debug)]
pub enum NotificationsError {
    Db(sqlx::Error),
    Setting(SettingError),
    Unsupported(Unsupported),
    /// The pending reviewables' list (crate::reviewables::user_menu_list).
    Review(crate::AppError),
    Url(UrlError),
}

impl std::fmt::Display for NotificationsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotificationsError::Db(e) => write!(f, "loading notifications: {e}"),
            NotificationsError::Setting(e) => e.fmt(f),
            NotificationsError::Unsupported(e) => e.fmt(f),
            NotificationsError::Review(e) => e.fmt(f),
            NotificationsError::Url(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for NotificationsError {}

impl From<sqlx::Error> for NotificationsError {
    fn from(e: sqlx::Error) -> Self {
        NotificationsError::Db(e)
    }
}

impl From<SettingError> for NotificationsError {
    fn from(e: SettingError) -> Self {
        NotificationsError::Setting(e)
    }
}

impl From<Unsupported> for NotificationsError {
    fn from(e: Unsupported) -> Self {
        NotificationsError::Unsupported(e)
    }
}

impl From<UrlError> for NotificationsError {
    fn from(e: UrlError) -> Self {
        NotificationsError::Url(e)
    }
}

impl From<crate::avatar::AvatarError> for NotificationsError {
    fn from(e: crate::avatar::AvatarError) -> Self {
        match e {
            crate::avatar::AvatarError::Setting(e) => NotificationsError::Setting(e),
            crate::avatar::AvatarError::Url(e) => NotificationsError::Url(e),
            crate::avatar::AvatarError::Unsupported(e) => NotificationsError::Unsupported(e),
        }
    }
}

impl From<GuardianError> for NotificationsError {
    fn from(e: GuardianError) -> Self {
        match e {
            GuardianError::Db(e) => NotificationsError::Db(e),
            GuardianError::Setting(e) => NotificationsError::Setting(e),
            GuardianError::Unsupported(e) => NotificationsError::Unsupported(e),
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct Row {
    id: i64,
    user_id: i32,
    notification_type: i32,
    read: bool,
    high_priority: bool,
    created_at: NaiveDateTime,
    post_number: Option<i32>,
    topic_id: Option<i32>,
    data: String,
    /// From the `visible` scope's preloaded topic.
    fancy_title: Option<String>,
    slug: Option<String>,
    subtype: Option<String>,
    /// The user's DiscourseConnect external id.
    external_id: Option<String>,
    /// `populate_acting_user`: None when it did not run, else the acting
    /// user found, if any.
    #[sqlx(skip)]
    acting: Option<Option<ActingUser>>,
}

/// A notification's acting user, as NotificationSerializer shows it.
#[derive(Debug, Clone)]
struct ActingUser {
    avatar_template: String,
    name: Option<String>,
}

const COLUMNS: &str = "notifications.id, notifications.user_id, notifications.notification_type, \
    notifications.read, notifications.high_priority, notifications.created_at, notifications.post_number, \
    notifications.topic_id, notifications.data, topics.fancy_title, topics.slug, topics.subtype, \
    (SELECT external_id FROM single_sign_on_records s WHERE s.user_id = notifications.user_id) AS external_id";

/// What the request asks for.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub recent: bool,
    /// `?silent`: skip the seen-id bump.
    pub silent: bool,
    pub limit: i64,
    pub offset: i64,
    /// "read" / "unread"; anything else is ignored (but echoed).
    pub filter: Option<String>,
    /// `filter_by_types`, resolved to ids.
    pub types: Vec<i32>,
}

pub struct Notifications<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
}

impl Notifications<'_> {
    /// The document for `user_id` (the viewer, or the admin's target), and
    /// whether `seen_notification_id` moved (Rails then publishes the
    /// notification state).
    pub async fn index(
        &mut self,
        user_id: i32,
        username: &str,
        q: &Query,
    ) -> Result<(Value, bool), NotificationsError> {
        if q.recent {
            self.recent(user_id, q).await
        } else {
            Ok((self.paged(user_id, username, q).await?, false))
        }
    }

    /// `Notification.prioritized_list` + the bump of `seen_notification_id`.
    async fn recent(
        &mut self,
        user_id: i32,
        q: &Query,
    ) -> Result<(Value, bool), NotificationsError> {
        let has_option: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_options WHERE user_id = $1)")
                .bind(user_id)
                .fetch_one(&mut *self.conn)
                .await?;
        let rows: Vec<Row> = if !has_option {
            Vec::new()
        } else {
            let never_likes: bool = sqlx::query_scalar(
                "SELECT like_notification_frequency = 3 FROM user_options WHERE user_id = $1",
            )
            .bind(user_id)
            .fetch_one(&mut *self.conn)
            .await?;
            let (types, order) = if !q.types.is_empty() {
                (
                    " AND notifications.notification_type = ANY($3::int[])".to_string(),
                    "NOT notifications.read DESC".to_string(),
                )
            } else if never_likes {
                (
                    " AND notifications.notification_type <> 5 AND notifications.notification_type <> 19 \
                     AND notifications.notification_type <> 25"
                        .to_string(),
                    format!("NOT notifications.read AND notifications.notification_type NOT IN ({LIKE_TYPES}) DESC"),
                )
            } else {
                (
                    String::new(),
                    format!(
                        "NOT notifications.read AND notifications.notification_type NOT IN ({LIKE_TYPES}) DESC"
                    ),
                )
            };
            sqlx::query_as(&format!(
                "SELECT {COLUMNS} FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
                 WHERE notifications.user_id = $1 AND (topics.id IS NULL OR topics.deleted_at IS NULL) \
                 AND (cardinality($3::int[]) = 0 OR TRUE){types} \
                 ORDER BY notifications.high_priority AND NOT notifications.read DESC, {order}, \
                          notifications.created_at DESC, notifications.id DESC LIMIT $2"
            ))
            .bind(user_id)
            .bind(q.limit)
            .bind(&q.types)
            .fetch_all(&mut *self.conn)
            .await?
        };
        // bump_last_seen_notification!: the newest visible notification,
        // before the in-memory filters.
        let mut bumped = false;
        if !rows.is_empty() && !q.silent {
            let max: Option<i64> = sqlx::query_scalar(
                "SELECT MAX(notifications.id) FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
                 WHERE notifications.user_id = $1 AND (topics.id IS NULL OR topics.deleted_at IS NULL) \
                 AND notifications.id > (SELECT seen_notification_id FROM users WHERE id = $1)",
            )
            .bind(user_id)
            .fetch_one(&mut *self.conn)
            .await?;
            if let Some(max) = max {
                sqlx::query(
                    "UPDATE users SET updated_at = now(), seen_notification_id = $2 WHERE id = $1",
                )
                .bind(user_id)
                .bind(max)
                .execute(&mut *self.conn)
                .await?;
                bumped = true;
            }
        }
        let mut rows = self.post_filter(rows).await?;
        self.populate_acting_user(&mut rows).await?;
        let seen: i64 = sqlx::query_scalar("SELECT seen_notification_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *self.conn)
            .await?;
        let mut out = Map::new();
        out.insert("notifications".into(), self.serialize(&rows)?);
        out.insert("seen_notification_id".into(), json!(seen));
        // pending_reviewables for the all notifications tab, when the user
        // can see the review queue.
        if q.types.is_empty() && self.guardian.is_staff() {
            let pending =
                crate::reviewables::user_menu_list(&mut *self.conn, self.settings, self.guardian)
                    .await
                    .map_err(NotificationsError::Review)?;
            out.insert("pending_reviewables".into(), Value::Array(pending));
        }
        Ok((Value::Object(out), bumped))
    }

    /// The paged branch: newest first, counted before the page is cut.
    async fn paged(
        &mut self,
        user_id: i32,
        username: &str,
        q: &Query,
    ) -> Result<Value, NotificationsError> {
        let read = match q.filter.as_deref() {
            Some("read") => " AND notifications.read = TRUE",
            Some("unread") => " AND notifications.read = FALSE",
            _ => "",
        };
        let total: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
             WHERE notifications.user_id = $1 AND (topics.id IS NULL OR topics.deleted_at IS NULL){read}"
        ))
        .bind(user_id)
        .fetch_one(&mut *self.conn)
        .await?;
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
             WHERE notifications.user_id = $1 AND (topics.id IS NULL OR topics.deleted_at IS NULL){read} \
             ORDER BY notifications.created_at DESC, notifications.id DESC LIMIT $2 OFFSET $3"
        ))
        .bind(user_id)
        .bind(q.limit)
        .bind(q.offset)
        .fetch_all(&mut *self.conn)
        .await?;
        let mut rows = self.post_filter(rows).await?;
        self.populate_acting_user(&mut rows).await?;
        let seen: i64 = sqlx::query_scalar("SELECT seen_notification_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *self.conn)
            .await?;
        // notifications_path(username:, offset:, limit:, filter:), params sorted.
        let mut more = String::from("/notifications?");
        if let Some(f) = &q.filter {
            more.push_str(&format!(
                "filter={}&",
                form_urlencoded::byte_serialize(f.as_bytes()).collect::<String>()
            ));
        }
        more.push_str(&format!(
            "limit={}&offset={}&username={}",
            q.limit,
            q.offset + q.limit,
            form_urlencoded::byte_serialize(username.as_bytes()).collect::<String>()
        ));
        Ok(json!({
            "notifications": self.serialize(&rows)?,
            "total_rows_notifications": total,
            "seen_notification_id": seen,
            "load_more_notifications": more,
        }))
    }

    /// `Notification.for_user_menu(user_id, limit:).unread.where(notification_type:)`,
    /// serialized. The user-menu endpoints filter these by what `data`
    /// points at before returning them, so each comes back on its own.
    pub async fn unread_for_user_menu(
        &mut self,
        user_id: i32,
        notification_type: i32,
        limit: i64,
    ) -> Result<Vec<Value>, NotificationsError> {
        self.for_user_menu(user_id, &[notification_type], false, limit)
            .await
    }

    /// `Notification.for_user_menu(user_id, limit:).where(read:,
    /// notification_type: types)`, serialized, unfiltered.
    pub async fn for_user_menu(
        &mut self,
        user_id: i32,
        types: &[i32],
        read: bool,
        limit: i64,
    ) -> Result<Vec<Value>, NotificationsError> {
        let mut rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
             WHERE notifications.user_id = $1 AND (topics.id IS NULL OR topics.deleted_at IS NULL) \
             AND notifications.read = $4 AND notifications.notification_type = ANY($2) \
             ORDER BY notifications.high_priority AND NOT notifications.read DESC, NOT notifications.read DESC, \
                      notifications.created_at DESC, notifications.id DESC LIMIT $3"
        ))
        .bind(user_id)
        .bind(types)
        .bind(limit)
        .bind(read)
        .fetch_all(&mut *self.conn)
        .await?;
        self.populate_acting_user(&mut rows).await?;
        match self.serialize(&rows)? {
            Value::Array(items) => Ok(items),
            _ => Ok(Vec::new()),
        }
    }

    /// `Notification.filter_inaccessible_topic_notifications` on serialized
    /// notifications: the ones whose topic the guardian can't see go.
    pub async fn filter_inaccessible_topics(
        &mut self,
        items: Vec<Value>,
    ) -> Result<Vec<Value>, NotificationsError> {
        let mut topic_ids: Vec<i32> = Vec::new();
        for n in &items {
            if let Some(id) = n["topic_id"].as_i64().map(|id| id as i32)
                && !topic_ids.contains(&id)
            {
                topic_ids.push(id);
            }
        }
        let accessible = self
            .guardian
            .can_see_topic_ids(&mut *self.conn, self.settings, &topic_ids)
            .await?;
        Ok(items
            .into_iter()
            .filter(|n| {
                n["topic_id"]
                    .as_i64()
                    .is_none_or(|id| accessible.contains(&(id as i32)))
            })
            .collect())
    }

    /// `filter_inaccessible_topic_notifications` then
    /// `filter_disabled_badge_notifications`.
    async fn post_filter(&mut self, rows: Vec<Row>) -> Result<Vec<Row>, NotificationsError> {
        let mut topic_ids: Vec<i32> = Vec::new();
        for r in &rows {
            if let Some(id) = r.topic_id
                && !topic_ids.contains(&id)
            {
                topic_ids.push(id);
            }
        }
        let accessible = self
            .guardian
            .can_see_topic_ids(&mut *self.conn, self.settings, &topic_ids)
            .await?;
        let mut rows: Vec<Row> = rows
            .into_iter()
            .filter(|r| r.topic_id.is_none_or(|id| accessible.contains(&id)))
            .collect();
        if rows.is_empty() {
            return Ok(rows);
        }
        if !self.settings.get("enable_badges")?.truthy() {
            rows.retain(|r| r.notification_type != GRANTED_BADGE);
            return Ok(rows);
        }
        let badge_ids: Vec<i64> = rows
            .iter()
            .filter(|r| r.notification_type == GRANTED_BADGE)
            .filter_map(|r| {
                serde_json::from_str::<Value>(&r.data)
                    .ok()?
                    .get("badge_id")?
                    .as_i64()
            })
            .collect();
        if badge_ids.is_empty() {
            return Ok(rows);
        }
        let enabled: Vec<i64> = sqlx::query_scalar(
            "SELECT id::bigint FROM badges WHERE id = ANY($1::bigint[]) AND enabled",
        )
        .bind(&badge_ids)
        .fetch_all(&mut *self.conn)
        .await?;
        rows.retain(|r| {
            r.notification_type != GRANTED_BADGE
                || serde_json::from_str::<Value>(&r.data)
                    .ok()
                    .and_then(|d| d.get("badge_id")?.as_i64())
                    .is_some_and(|id| enabled.contains(&id))
        });
        Ok(rows)
    }

    /// `Notification.populate_acting_user`: with avatars in the user menu
    /// or full names first, each notification's acting user is the user
    /// named by its data's username, display_username,
    /// mentioned_by_username, invited_by_username or original_username,
    /// the first one set.
    async fn populate_acting_user(&mut self, rows: &mut [Row]) -> Result<(), NotificationsError> {
        if !(self.settings.get("show_user_menu_avatars")?.truthy()
            || self.settings.get("prioritize_full_name_in_ux")?.truthy())
        {
            return Ok(());
        }
        let mut names: Vec<Option<String>> = Vec::with_capacity(rows.len());
        for r in rows.iter() {
            let data: Map<String, Value> = serde_json::from_str(&r.data).unwrap_or_default();
            let named = [
                "username",
                "display_username",
                "mentioned_by_username",
                "invited_by_username",
                "original_username",
            ]
            .iter()
            .find_map(|key| {
                data.get(*key)
                    .filter(|v| !matches!(v, Value::Null | Value::Bool(false)))
            });
            names.push(match named {
                None => None,
                Some(Value::String(s)) => Some(s.to_lowercase()),
                // `.downcase` raises on anything but a string.
                Some(_) => {
                    return Err(
                        Unsupported("notification data naming a user by a non-string").into(),
                    );
                }
            });
        }
        let mut wanted: Vec<String> = names.iter().flatten().cloned().collect();
        wanted.sort_unstable();
        wanted.dedup();
        // id, username, username_lower, name, uploaded_avatar_id
        type UserRow = (i32, String, String, Option<String>, Option<i32>);
        let users: Vec<UserRow> = sqlx::query_as(
            "SELECT id, username, username_lower, name, uploaded_avatar_id FROM users \
             WHERE username_lower = ANY($1)",
        )
        .bind(&wanted)
        .fetch_all(&mut *self.conn)
        .await?;
        let logo: Option<String> = if users
            .iter()
            .any(|(id, ..)| *id == crate::avatar::SYSTEM_USER_ID)
        {
            let id = self.settings.get("logo_small")?.to_i();
            if id == 0 {
                None
            } else {
                sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
                    .bind(i32::try_from(id).unwrap_or(0))
                    .fetch_optional(&mut *self.conn)
                    .await?
            }
        } else {
            None
        };
        for (r, name) in rows.iter_mut().zip(names) {
            let found = users
                .iter()
                .find(|(_, _, lower, ..)| Some(lower) == name.as_ref());
            r.acting = Some(match found {
                Some((id, username, _, name, uploaded_avatar_id)) => {
                    // avatar_template_url: absolute, without `http:`.
                    let template = crate::avatar::avatar_template(
                        self.urls,
                        *id,
                        username,
                        *uploaded_avatar_id,
                        logo.as_deref(),
                    )?;
                    let absolute = self.urls.absolute(&template)?;
                    let avatar_template = match absolute.get(..5) {
                        Some(p) if p.eq_ignore_ascii_case("http:") => absolute[5..].to_string(),
                        _ => absolute,
                    };
                    Some(ActingUser {
                        avatar_template,
                        name: name.clone(),
                    })
                }
                None => None,
            });
        }
        Ok(())
    }

    fn serialize(&self, rows: &[Row]) -> Result<Value, NotificationsError> {
        serialize(self.settings, rows)
    }
}

/// `notifications.visible.order("notifications.created_at desc").first`,
/// serialized: the last notification in the published notification state.
pub async fn last_visible(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
) -> Result<Option<Value>, NotificationsError> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
         WHERE notifications.user_id = $1 AND (topics.id IS NULL OR topics.deleted_at IS NULL) \
         ORDER BY notifications.created_at DESC LIMIT 1"
    ))
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match row {
        Some(row) => match serialize(settings, &[row])? {
            Value::Array(mut items) => items.pop(),
            _ => None,
        },
        None => None,
    })
}

/// NotificationSerializer for one notification, its topic as the
/// association loads it (a trashed topic is none).
pub async fn serialize_by_id(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    id: i64,
) -> Result<Option<Value>, NotificationsError> {
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM notifications LEFT JOIN topics ON notifications.topic_id = topics.id \
         AND topics.deleted_at IS NULL WHERE notifications.id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    // A trashed topic: no slug (Slug.for only runs with a topic).
    const NO_TOPIC: &str = "\u{0}";
    Ok(match row {
        Some(mut row) => {
            if row.topic_id.is_some() && row.slug.is_none() {
                row.slug = Some(NO_TOPIC.into());
            }
            match serialize(settings, &[row])? {
                Value::Array(mut items) => items.pop().map(|mut n| {
                    if n["slug"] == NO_TOPIC {
                        n["slug"] = Value::Null;
                    }
                    n
                }),
                _ => None,
            }
        }
        None => None,
    })
}

/// NotificationSerializer for each row.
fn serialize(settings: &SiteSettings, rows: &[Row]) -> Result<Value, NotificationsError> {
    let enable_names = settings.get("enable_names")?.truthy();
    let discourse_connect = settings.get("enable_discourse_connect")?.truthy();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let mut n = Map::new();
        if let Some(title) = r.fancy_title.as_deref().filter(|t| !t.is_empty()) {
            n.insert("fancy_title".into(), json!(title));
        }
        n.insert("id".into(), json!(r.id));
        n.insert("user_id".into(), json!(r.user_id));
        if discourse_connect {
            n.insert("external_id".into(), json!(r.external_id));
        }
        n.insert("notification_type".into(), json!(r.notification_type));
        n.insert("read".into(), json!(r.read));
        n.insert("high_priority".into(), json!(r.high_priority));
        n.insert("created_at".into(), json!(time_json(r.created_at)));
        n.insert("post_number".into(), json!(r.post_number));
        n.insert("topic_id".into(), json!(r.topic_id));
        // Slug.for(topic.title) is the stored slug on every topic here.
        let slug = match (r.topic_id, &r.slug) {
            (Some(_), Some(s)) if !s.is_empty() => json!(s),
            (Some(_), _) => {
                return Err(Unsupported("topics without a stored slug (Slug.for)").into());
            }
            (None, _) => Value::Null,
        };
        n.insert("slug".into(), slug);
        let mut data: Value = serde_json::from_str(&r.data).unwrap_or(json!({}));
        if !enable_names && let Some(obj) = data.as_object_mut() {
            obj.remove("display_name");
        }
        // populate_acting_user sets data_hash[:original_name] with names on.
        if enable_names
            && let Some(acting) = &r.acting
            && let Some(obj) = data.as_object_mut()
        {
            obj.insert(
                "original_name".into(),
                json!(acting.as_ref().and_then(|a| a.name.clone())),
            );
        }
        n.insert("data".into(), data);
        if r.subtype.as_deref() == Some("moderator_warning") {
            n.insert("is_warning".into(), json!(true));
        }
        if let Some(Some(acting)) = &r.acting {
            n.insert(
                "acting_user_avatar_template".into(),
                json!(acting.avatar_template),
            );
            if enable_names {
                n.insert("acting_user_name".into(), json!(acting.name));
            }
        }
        out.push(Value::Object(n));
    }
    Ok(Value::Array(out))
}

/// `filter_by_types=a,b` to ids; the first unknown name is the error.
pub fn parse_types(raw: &str) -> Result<Vec<i32>, String> {
    raw.split(',')
        .filter(|s| !s.is_empty())
        .map(|name| {
            TYPES
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, id)| *id)
                .ok_or_else(|| format!("invalid notification type: {name}"))
        })
        .collect()
}
