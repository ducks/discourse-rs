//! `UserUpdater#update` as UsersController#update calls it: the user's
//! options (discourse-solved's and chat's included), profile (bio,
//! location, website, the profile and card backgrounds), name, locale,
//! title, primary and flair groups, category tracking, muted users and
//! allowed PM senders, the notification schedule, with the model
//! callbacks that follow (the bio cooked, the search index, the name
//! change logged, the display name job, the title's badge, the profile's
//! upload references and the schedule's do not disturb timings).
//!
//! Refused: what failing validations would answer (the errors JSON),
//! external ids, date of birth, tag tracking, sidebar links, user status,
//! uploads in the bio, and gravatar downloads.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::params;
use crate::posting::Ctx;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The user being updated.
#[derive(sqlx::FromRow)]
pub struct Target {
    pub id: i32,
    pub username: String,
    pub username_lower: String,
    pub name: Option<String>,
    pub locale: Option<String>,
    pub admin: bool,
    pub moderator: bool,
    pub staged: bool,
    pub active: bool,
    pub trust_level: i32,
    pub title: Option<String>,
    pub primary_group_id: Option<i32>,
    pub flair_group_id: Option<i32>,
}

impl Target {
    /// `User#has_trust_level?`
    fn has_trust_level(&self, level: i32) -> bool {
        self.admin || self.moderator || self.staged || self.trust_level >= level
    }
}

/// `fetch_user_from_params`: the user by username, active unless it is
/// the current user or the viewer is staff.
pub async fn find_target(
    conn: &mut PgConnection,
    guardian: &Guardian,
    username: &str,
) -> Result<Option<Target>, sqlx::Error> {
    let username_lower = username.to_lowercase();
    let username_lower = username_lower
        .strip_suffix(".json")
        .unwrap_or(&username_lower);
    sqlx::query_as(
        "SELECT id, username, username_lower, name, locale, admin, moderator, staged, active, \
                trust_level, title, primary_group_id, flair_group_id \
         FROM users WHERE username_lower = $1 AND (active OR id = $2 OR $3) LIMIT 1",
    )
    .bind(username_lower)
    .bind(guardian.user_id().unwrap_or(0))
    .bind(guardian.is_staff())
    .fetch_optional(conn)
    .await
}

/// `can_edit_user?`
pub fn can_edit_user(guardian: &Guardian, user: &Target) -> bool {
    guardian.is_me(user.id) || guardian.is_staff()
}

/// `can_edit_name?`
fn can_edit_name(s: &SiteSettings, guardian: &Guardian, user: &Target) -> Result<bool, AppError> {
    if s.get("auth_overrides_name")?.truthy() {
        return Ok(false);
    }
    if guardian.is_admin() {
        return Ok(true);
    }
    if !s.get("enable_names")?.truthy() {
        return Ok(false);
    }
    if guardian.is_moderator() {
        return Ok(true);
    }
    Ok(!guardian.is_anonymous() && can_edit_user(guardian, user))
}

/// `NotificationLevels.all`
const MUTED: i32 = 0;
const REGULAR: i32 = 1;
const TRACKING: i32 = 2;
const WATCHING: i32 = 3;
const WATCHING_FIRST_POST: i32 = 4;

/// `TopicUser.notification_reasons`
const USER_CHANGED: i32 = 2;
const AUTO_WATCH_CATEGORY: i32 = 6;
const AUTO_TRACK_CATEGORY: i32 = 8;

/// `UserHistory.actions[:change_name]`
const CHANGE_NAME: i32 = 48;
/// `UserHistory.actions[:filled_in_required_fields]`
const FILLED_IN_REQUIRED_FIELDS: i32 = 109;

/// `UserUpdater::CATEGORY_IDS`
const CATEGORY_IDS: [(&str, i32); 5] = [
    ("watched_first_post_category_ids", WATCHING_FIRST_POST),
    ("watched_category_ids", WATCHING),
    ("tracked_category_ids", TRACKING),
    ("regular_category_ids", REGULAR),
    ("muted_category_ids", MUTED),
];

/// `UserUpdater::TAG_NAMES`
const TAG_NAMES: [&str; 4] = [
    "watching_first_post_tags",
    "watched_tags",
    "tracked_tags",
    "muted_tags",
];

/// How an option's column takes a param.
#[derive(Clone, Copy)]
enum Kind {
    /// `to_s == "true"` (a boolean column holding true or false).
    Bool,
    /// An integer column, cast as ActiveModel casts a string.
    Int,
    /// An integer column behind an ActiveRecord enum.
    Enum(&'static [(&'static str, i64)]),
    Text,
    /// An array column: theme_ids (by name), seen_popups (integers), the
    /// rest strings.
    Array,
}

const DEFAULT_CALENDARS: &[(&str, i64)] = &[
    ("none_selected", 0),
    ("ics", 1),
    ("google", 2),
    ("outlook", 3),
    ("apple", 4),
];
const PUSH_NOTIFICATION_LEVELS: &[(&str, i64)] = &[("none", 0), ("all", 1), ("chat_only", 2)];
const SEND_SHORTCUTS: &[(&str, i64)] = &[("enter", 0), ("meta_enter", 1)];

/// `UserOption.text_sizes`
const TEXT_SIZES: &[(&str, i64)] = &[
    ("normal", 0),
    ("larger", 1),
    ("largest", 2),
    ("smaller", 3),
    ("smallest", 4),
];
/// `UserOption.title_count_modes`
const TITLE_COUNT_MODES: &[(&str, i64)] = &[("notifications", 0), ("contextual", 1)];

/// `UserUpdater::OPTION_ATTR` with the columns they write. `text_size`
/// and `title_count_mode` are handled apart, by their key columns.
const OPTION_ATTR: &[(&str, Kind)] = &[
    ("mailing_list_mode", Kind::Bool),
    ("mailing_list_mode_frequency", Kind::Int),
    ("email_digests", Kind::Bool),
    ("email_level", Kind::Int),
    ("email_messages_level", Kind::Int),
    ("external_links_in_new_tab", Kind::Bool),
    ("enable_quoting", Kind::Bool),
    ("enable_smart_lists", Kind::Bool),
    ("enable_markdown_monospace_font", Kind::Bool),
    ("color_scheme_id", Kind::Int),
    ("dark_scheme_id", Kind::Int),
    ("interface_color_mode", Kind::Int),
    ("dynamic_favicon", Kind::Bool),
    ("automatically_unpin_topics", Kind::Bool),
    ("digest_after_minutes", Kind::Int),
    ("new_topic_duration_minutes", Kind::Int),
    ("auto_track_topics_after_msecs", Kind::Int),
    ("notification_level_when_replying", Kind::Int),
    ("email_previous_replies", Kind::Int),
    ("email_in_reply_to", Kind::Bool),
    ("like_notification_frequency", Kind::Int),
    ("notify_on_linked_posts", Kind::Bool),
    (
        "push_notification_level",
        Kind::Enum(PUSH_NOTIFICATION_LEVELS),
    ),
    ("enable_upcoming_change_available_notifications", Kind::Bool),
    ("include_tl0_in_digests", Kind::Bool),
    ("theme_ids", Kind::Array),
    ("allow_private_messages", Kind::Bool),
    ("enable_allowed_pm_users", Kind::Bool),
    ("homepage_id", Kind::Int),
    ("hide_profile", Kind::Bool),
    ("hide_presence", Kind::Bool),
    ("text_size", Kind::Enum(TEXT_SIZES)),
    ("title_count_mode", Kind::Enum(TITLE_COUNT_MODES)),
    ("timezone", Kind::Text),
    ("skip_new_user_tips", Kind::Bool),
    ("seen_popups", Kind::Array),
    ("default_calendar", Kind::Enum(DEFAULT_CALENDARS)),
    ("bookmark_auto_delete_preference", Kind::Int),
    ("sidebar_link_to_filtered_list", Kind::Bool),
    ("sidebar_show_count_of_new_items", Kind::Bool),
    ("watched_precedence_over_muted", Kind::Bool),
    ("composition_mode", Kind::Int),
    ("send_shortcut", Kind::Enum(SEND_SHORTCUTS)),
    ("automatically_translate", Kind::Bool),
    ("understood_languages", Kind::Array),
    ("hidden_composer_toolbar_buttons", Kind::Array),
    // discourse-solved's and chat's, pushed by their plugin.rb.
    ("notify_on_solved", Kind::Bool),
    ("chat_enabled", Kind::Bool),
    (
        "chat_quick_reaction_type",
        Kind::Enum(CHAT_QUICK_REACTION_TYPES),
    ),
    ("chat_quick_reactions_custom", Kind::Text),
    ("chat_sound", Kind::Text),
    ("ignore_channel_wide_mention", Kind::Bool),
    ("show_thread_title_prompts", Kind::Bool),
    ("chat_announce_new_messages", Kind::Bool),
    ("chat_channel_list_filter", Kind::Enum(CHAT_LIST_FILTERS)),
    (
        "chat_channel_list_filter_starred",
        Kind::Enum(CHAT_LIST_FILTERS),
    ),
    (
        "chat_channel_list_filter_dms",
        Kind::Enum(CHAT_LIST_FILTERS),
    ),
    ("chat_channel_list_sort", Kind::Enum(CHAT_LIST_SORTS)),
    (
        "chat_channel_list_sort_starred",
        Kind::Enum(CHAT_LIST_SORTS),
    ),
    ("chat_channel_list_sort_dms", Kind::Enum(CHAT_LIST_SORTS)),
    ("chat_new_message_sound", Kind::Bool),
    ("chat_email_frequency", Kind::Enum(CHAT_EMAIL_FREQUENCIES)),
    (
        "chat_header_indicator_preference",
        Kind::Enum(CHAT_HEADER_INDICATORS),
    ),
    ("chat_separate_sidebar_mode", Kind::Enum(CHAT_SIDEBAR_MODES)),
    ("chat_send_shortcut", Kind::Enum(SEND_SHORTCUTS)),
];

/// Chat::UserOptionExtension's enums.
const CHAT_QUICK_REACTION_TYPES: &[(&str, i64)] = &[("frequent", 0), ("custom", 1)];
const CHAT_LIST_FILTERS: &[(&str, i64)] =
    &[("all", 0), ("active", 1), ("unread", 2), ("mentions", 3)];
const CHAT_LIST_SORTS: &[(&str, i64)] =
    &[("alphabetical", 0), ("recent_activity", 1), ("priority", 2)];
const CHAT_EMAIL_FREQUENCIES: &[(&str, i64)] = &[("never", 0), ("when_away", 1)];
const CHAT_HEADER_INDICATORS: &[(&str, i64)] = &[
    ("all_new", 0),
    ("dm_and_mentions", 1),
    ("never", 2),
    ("only_mentions", 3),
];
const CHAT_SIDEBAR_MODES: &[(&str, i64)] = &[
    ("default", 0),
    ("never", 1),
    ("always", 2),
    ("fullscreen", 3),
];

/// What the update refuses outright, by param.
const REFUSED: &[(&str, &str)] = &[
    ("custom_fields", "updating custom fields"),
    ("external_ids", "updating external ids (191)"),
    (
        "date_of_birth",
        "updating the date of birth (discourse-cakeday, 202)",
    ),
    (
        "sidebar_category_ids",
        "updating sidebar category links (156)",
    ),
    ("sidebar_tag_names", "updating sidebar tag links (156)"),
    ("status", "updating the user status (130)"),
];

/// `UsersController#user_params`' scalar keys beside OPTION_ATTR's.
const PERMITTED: &[&str] = &[
    "name",
    "email",
    "password",
    "username",
    "title",
    "date_of_birth",
    "muted_usernames",
    "allowed_pm_usernames",
    "theme_ids",
    "locale",
    "bio_raw",
    "location",
    "website",
    "dismissed_banner_key",
    "profile_background_upload_url",
    "card_background_upload_url",
    "primary_group_id",
    "flair_group_id",
    "featured_topic_id",
    "show_original_content",
    "watching_first_post_tags",
    "watched_tags",
    "tracked_tags",
    "muted_tags",
];

/// `UserUpdater::NOTIFICATION_SCHEDULE_ATTRS`' keys.
const SCHEDULE_KEYS: &[&str] = &[
    "enabled",
    "day_0_start_time",
    "day_0_end_time",
    "day_1_start_time",
    "day_1_end_time",
    "day_2_start_time",
    "day_2_end_time",
    "day_3_start_time",
    "day_3_end_time",
    "day_4_start_time",
    "day_4_end_time",
    "day_5_start_time",
    "day_5_end_time",
    "day_6_start_time",
    "day_6_end_time",
];

/// `UsersController#user_params`: what `params.permit` keeps of the
/// params. A permitted scalar takes no list or hash, a list key takes
/// only a list of scalars; unknown keys go. No bundled plugin the port
/// runs registers an editable custom field, so `custom_fields` goes too.
pub fn user_params(
    mut p: Map<String, Value>,
    s: &SiteSettings,
) -> Result<Map<String, Value>, AppError> {
    let tagging = s.get("tagging_enabled")?.truthy();
    for key in ["sidebar_category_ids", "sidebar_tag_names"] {
        if (key == "sidebar_category_ids" || tagging)
            && p.get(key)
                .is_some_and(|v| params::scalar(v).is_some_and(|v| crate::ruby::is_blank(&v)))
        {
            p.insert(key.into(), json!([]));
        }
    }
    let scalar = |v: &Value| !matches!(v, Value::Array(_) | Value::Object(_));
    let list = |v: &Value| matches!(v, Value::Array(items) if items.iter().all(scalar));
    let mut out = Map::new();
    for (key, value) in p {
        let keep = match key.as_str() {
            "understood_languages" | "hidden_composer_toolbar_buttons" | "sidebar_category_ids" => {
                list(&value)
            }
            "sidebar_tag_names" => tagging && list(&value),
            k if CATEGORY_IDS.iter().any(|(c, _)| *c == k) => list(&value),
            "theme_ids" | "seen_popups" => scalar(&value) || list(&value),
            "user_notification_schedule" => {
                if let Value::Object(given) = &value {
                    let kept: Map<String, Value> = given
                        .iter()
                        .filter(|(k, v)| SCHEDULE_KEYS.contains(&k.as_str()) && scalar(v))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    out.insert(key, Value::Object(kept));
                }
                continue;
            }
            "status" if s.get("enable_user_status")?.truthy() => match &value {
                Value::Object(given) => {
                    let kept: Map<String, Value> = given
                        .iter()
                        .filter(|(k, v)| {
                            ["emoji", "description", "ends_at"].contains(&k.as_str()) && scalar(v)
                        })
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    out.insert(key, Value::Object(kept));
                    continue;
                }
                v => scalar(v),
            },
            k if PERMITTED.contains(&k) || OPTION_ATTR.iter().any(|(o, _)| *o == k) => {
                scalar(&value)
            }
            _ => false,
        };
        if keep {
            out.insert(key, value);
        }
    }
    Ok(out)
}

/// A param as `attributes[key]`: its string, or nil.
fn text(attrs: &Map<String, Value>, key: &str) -> Option<String> {
    attrs.get(key).and_then(params::scalar)
}

/// `blank?`
fn blank(v: Option<&str>) -> bool {
    v.is_none_or(|v| v.trim().is_empty())
}

/// `ActiveModel::Type::Boolean` on a param: blank is nil.
fn cast_bool(value: &Value) -> Value {
    match params::scalar(value).as_deref() {
        None | Some("") => Value::Null,
        Some("0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF") => json!(false),
        Some(_) => json!(true),
    }
}

/// `ActiveModel::Type::Integer` on a param: blank is nil, the rest `to_i`.
fn cast_int(value: &Value) -> Value {
    match params::scalar(value) {
        Some(v) if !v.is_empty() => json!(crate::ruby::to_i(&v)),
        _ => Value::Null,
    }
}

/// `UserUpdater#format_url`
fn format_url(website: Option<&str>) -> Option<String> {
    let website = website.filter(|w| !blank(Some(w)))?;
    let has_scheme = website.split_once(':').is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    });
    Some(if has_scheme {
        website.to_string()
    } else {
        format!("http://{website}")
    })
}

/// The host of an http(s) URL (`URI.parse(...).host` for a `URI::HTTP`).
fn http_host(website: &str) -> Option<String> {
    let uri: axum::http::Uri = website.parse().ok()?;
    matches!(uri.scheme_str(), Some("http" | "https"))
        .then(|| uri.host().map(str::to_string))
        .flatten()
}

/// `UrlValidator`: an http(s) URL whose host has a dot.
fn valid_url(website: &str) -> bool {
    http_host(website).is_some_and(|h| h.contains('.'))
}

/// The title, primary group and flair group an update leaves the user
/// with.
struct Groups {
    title: Option<String>,
    primary_group_id: Option<i32>,
    flair_group_id: Option<i32>,
}

/// `Guardian#can_grant_title?(user, title)`
async fn can_grant_title(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    actor: &Guardian,
    user: &Target,
    title: &str,
) -> Result<bool, AppError> {
    if actor.is_staff() || title.is_empty() {
        return Ok(true);
    }
    if !actor.is_me(user.id) {
        return Ok(false);
    }
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT b.name FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
         WHERE ub.user_id = $1 AND b.allow_title",
    )
    .bind(user.id)
    .fetch_all(&mut *conn)
    .await?;
    // Badge.display_name(name)
    let badged = names.iter().any(|name| {
        let key = format!("badges.{}.name", name.to_lowercase().replace(' ', "_"));
        i18n.t(&key).unwrap_or(name) == title
    });
    if badged {
        return Ok(true);
    }
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users gu JOIN groups g ON g.id = gu.group_id \
                        WHERE gu.user_id = $1 AND g.title = $2)",
    )
    .bind(user.id)
    .bind(title)
    .fetch_one(&mut *conn)
    .await?)
}

/// `Guardian#can_use_primary_group?` and `#can_use_flair_group?`: the
/// user is in the group, which is not automatic (primary) or has a flair.
async fn can_use_group(
    conn: &mut PgConnection,
    user_id: i32,
    group_id: i32,
    flair: bool,
) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users gu JOIN groups g ON g.id = gu.group_id \
                        WHERE gu.user_id = $1 AND g.id = $2 \
                          AND CASE WHEN $3 THEN (COALESCE(btrim(g.flair_icon), '') <> '' \
                                                 OR g.flair_upload_id IS NOT NULL) \
                                   ELSE NOT g.automatic END)",
    )
    .bind(user_id)
    .bind(group_id)
    .bind(flair)
    .fetch_one(&mut *conn)
    .await?)
}

/// UserUpdater's title, primary group and flair group, then
/// User#match_primary_group_changes: a new primary group carries its
/// title over a title the old one gave, and its id over a flair the old
/// one was.
async fn titles_and_groups(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    actor: &Guardian,
    user: &Target,
    attrs: &Map<String, Value>,
) -> Result<Groups, AppError> {
    let mut title = user.title.clone();
    if let Some(t) = text(attrs, "title")
        && Some(&t) != user.title.as_ref()
        && can_grant_title(&mut *conn, i18n, actor, user, &t).await?
    {
        title = Some(t);
    }
    let id = |v: &str| i32::try_from(crate::ruby::to_i(v)).unwrap_or(0);
    let mut primary_group_id = user.primary_group_id;
    if s.get("user_selected_primary_groups")?.truthy()
        && let Some(v) = text(attrs, "primary_group_id")
    {
        if can_use_group(&mut *conn, user.id, id(&v), false).await? {
            primary_group_id = Some(id(&v));
        } else if blank(Some(&v)) {
            primary_group_id = None;
        }
    }
    let mut flair_group_id = user.flair_group_id;
    if let Some(v) = text(attrs, "flair_group_id") {
        if blank(Some(&v)) {
            flair_group_id = None;
        } else if can_use_group(&mut *conn, user.id, id(&v), true).await? {
            flair_group_id = Some(id(&v));
        }
    }
    if primary_group_id != user.primary_group_id {
        let was_title: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1 AND title IS NOT DISTINCT FROM $2)",
        )
        .bind(user.primary_group_id)
        .bind(&title)
        .fetch_one(&mut *conn)
        .await?;
        if was_title {
            title = match primary_group_id {
                Some(gid) => sqlx::query_scalar::<_, Option<String>>(
                    "SELECT title FROM groups WHERE id = $1",
                )
                .bind(gid)
                .fetch_optional(&mut *conn)
                .await?
                .flatten(),
                None => None,
            };
        }
        if flair_group_id == user.primary_group_id {
            flair_group_id = primary_group_id;
        }
    }
    Ok(Groups {
        title,
        primary_group_id,
        flair_group_id,
    })
}

/// `UserUpdater#update(attributes)`: whether it saved. `custom_fields`
/// is `attributes[:custom_fields]` as the controller builds it from
/// `user_fields` (None when that param wasn't given).
pub async fn update(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    actor: &Guardian,
    user: &Target,
    attrs: &Map<String, Value>,
    custom_fields: Option<&[(String, Value)]>,
) -> Result<bool, AppError> {
    let s = ctx.settings;
    for (key, what) in REFUSED {
        if attrs.contains_key(*key) {
            return Err(Unsupported(what).into());
        }
    }
    let groups = titles_and_groups(&mut *conn, s, ctx.i18n, actor, user, attrs).await?;

    // LEGACY_SHOW_ORIGINAL_CONTENT_ATTR
    let mut attrs = attrs.clone();
    if let Some(legacy) = attrs.remove("show_original_content")
        && !attrs.contains_key("automatically_translate")
    {
        let translate = params::scalar(&legacy).is_none_or(|v| v != "true");
        attrs.insert(
            "automatically_translate".into(),
            json!(translate.to_string()),
        );
    }
    if text(&attrs, "homepage_id").as_deref() == Some("-1") {
        attrs.insert("homepage_id".into(), Value::Null);
    }

    if can_edit_user(actor, user)
        && (s.get("allow_changing_staged_user_tracking")?.truthy() || !user.staged)
    {
        for (key, level) in CATEGORY_IDS {
            if let Some(Value::Array(ids)) = attrs.get(key) {
                let ids: Vec<i32> = ids
                    .iter()
                    .filter_map(params::scalar)
                    .filter(|v| !v.is_empty())
                    .map(|v| crate::ruby::to_i(&v) as i32)
                    .collect();
                batch_set_categories(&mut *conn, user.id, level, &ids).await?;
            }
        }
        if TAG_NAMES.iter().any(|k| attrs.contains_key(*k)) {
            return Err(Unsupported("updating tag tracking").into());
        }
    }

    let options = update_options(&mut *conn, user.id, &attrs).await?;

    if attrs.contains_key("muted_usernames") {
        set_user_list(
            &mut *conn,
            user,
            "muted_users",
            "muted_user_id",
            text(&attrs, "muted_usernames"),
        )
        .await?;
    }
    if attrs.contains_key("allowed_pm_usernames") {
        set_user_list(
            &mut *conn,
            user,
            "allowed_pm_users",
            "allowed_pm_user_id",
            text(&attrs, "allowed_pm_usernames"),
        )
        .await?;
    }

    // user_option.save: update_tracked_topics
    if let Some(Some(threshold)) = options {
        tracked_topics_updater(&mut *conn, user.id, threshold).await?;
    }

    // user_notification_schedule.save: the user's row or a new one,
    // assigned the permitted params. Invalid, it fails the update there:
    // what was written before stays (no rollback), the profile and user
    // aren't saved.
    let schedule = match attrs.get("user_notification_schedule") {
        Some(Value::Object(params)) => {
            let before = crate::notification_schedule::Schedule::find(&mut *conn, user.id).await?;
            let mut schedule = before
                .clone()
                .unwrap_or_else(crate::notification_schedule::Schedule::new_record);
            schedule.assign(params)?;
            if !schedule.valid() {
                return Ok(false);
            }
            schedule.save(&mut *conn, user.id, before.as_ref()).await?;
            Some(schedule)
        }
        _ => None,
    };
    update_profile(&mut *conn, ctx, actor, user, &attrs).await?;

    // user.save
    let old_name = user.name.clone().unwrap_or_default();
    let mut name = user.name.clone();
    if can_edit_name(s, actor, user)? && attrs.contains_key("name") {
        name = text(&attrs, "name");
    }
    let locale = if attrs.contains_key("locale") {
        text(&attrs, "locale")
    } else {
        user.locale.clone()
    };
    let name_changed = name != user.name;
    if name_changed {
        validate_name(&mut *conn, s, user.id, name.as_deref()).await?;
    }
    let title_changed = groups.title != user.title;
    if title_changed {
        crate::badge_granter::check_if_title_is_badged_granted(
            &mut *conn,
            ctx.i18n,
            user.id,
            groups.title.as_deref(),
        )
        .await?;
    }
    if name_changed
        || locale != user.locale
        || title_changed
        || groups.primary_group_id != user.primary_group_id
        || groups.flair_group_id != user.flair_group_id
    {
        sqlx::query(
            "UPDATE users SET name = $2, locale = $3, title = $4, primary_group_id = $5, \
                              flair_group_id = $6, updated_at = now() WHERE id = $1",
        )
        .bind(user.id)
        .bind(&name)
        .bind(&locale)
        .bind(&groups.title)
        .bind(groups.primary_group_id)
        .bind(groups.flair_group_id)
        .execute(&mut *conn)
        .await?;
    }
    if name_changed {
        // after_update :change_display_name
        crate::jobs::enqueue(
            &mut *conn,
            "change_display_name",
            json!({ "user_id": user.id, "old_name": user.name, "new_name": name }),
        )
        .await?;
    }
    // after_save :save_custom_fields
    for (key, value) in custom_fields.into_iter().flatten() {
        crate::user_fields::save_custom_field(&mut *conn, user.id, key, value).await?;
    }
    user_after_save(&mut *conn, s, user, name.as_deref()).await?;

    let new_name = name.clone().unwrap_or_default();
    if name_changed && old_name.to_lowercase() != new_name.to_lowercase() {
        // StaffActionLogger#log_name_change
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, target_user_id, previous_value, \
                                         new_value, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(CHANGE_NAME)
        .bind(actor.user_id())
        .bind(user.id)
        .bind(&old_name)
        .bind(&new_name)
        .execute(&mut *conn)
        .await?;
    }

    // Once saved: an enabled schedule's timings rebuilt, a disabled one's
    // scheduled timings gone.
    if let Some(schedule) = schedule {
        if schedule.enabled == Some(true) {
            crate::notification_schedule::create_do_not_disturb_timings(
                &mut *conn, ctx.bus, user.id, &schedule, true,
            )
            .await?;
        } else {
            crate::notification_schedule::destroy_scheduled_timings(&mut *conn, user.id).await?;
        }
    }
    // DiscourseEvent :user_updated: its web hooks.
    crate::web_hooks::enqueue_user_hooks(
        &mut *conn,
        &crate::web_hooks::Payloads {
            settings: ctx.settings,
            i18n: ctx.i18n,
            config: ctx.config,
        },
        "user_updated",
        user.id,
    )
    .await?;
    if custom_fields.is_some_and(|c| !c.is_empty())
        && crate::user_fields::needs_required_fields_check(&mut *conn, user.id).await?
    {
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, admin_only, created_at, updated_at) \
             VALUES ($1, $2, FALSE, clock_timestamp(), clock_timestamp())",
        )
        .bind(FILLED_IN_REQUIRED_FIELDS)
        .bind(user.id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(true)
}

/// The name's validations: no longer than 255, present when the site
/// requires it, and not the user's password.
async fn validate_name(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    name: Option<&str>,
) -> Result<(), AppError> {
    let invalid = || Err(Unsupported("an invalid name (the errors JSON)").into());
    let Some(name) = name.filter(|n| !blank(Some(n))) else {
        if s.get("full_name_requirement")?.to_s() == "required_at_signup" {
            return invalid();
        }
        return Ok(());
    };
    if name.chars().count() > 255 {
        return invalid();
    }
    // name_validator: the name is not the password.
    let name_pw: String = name.chars().take(200).collect();
    for candidate in [name_pw.clone(), name_pw.to_lowercase()] {
        let check = crate::session::token::confirm_password(&mut *conn, user_id, &candidate)
            .await
            .map_err(|e| match e {
                crate::session::token::PasswordError::Db(e) => AppError::from(e),
                crate::session::token::PasswordError::Unsupported(u) => AppError::from(u),
            })?;
        if check.is_some_and(|c| c.matches) {
            return invalid();
        }
    }
    Ok(())
}

/// User's after_save callbacks: `refresh_avatar`, the login hint and
/// `index_search`.
async fn user_after_save(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user: &Target,
    name: Option<&str>,
) -> Result<(), AppError> {
    after_save(
        conn,
        s,
        user.id,
        user.admin && user.active,
        &user.username_lower,
        name,
    )
    .await
}

/// User's after_save callbacks on any save; `active_admin` for the login
/// hint.
pub(crate) async fn after_save(
    conn: &mut PgConnection,
    s: &SiteSettings,
    user_id: i32,
    active_admin: bool,
    username_lower: &str,
    name: Option<&str>,
) -> Result<(), AppError> {
    // clear_global_notice_if_needed
    if active_admin && s.get("has_login_hint")?.truthy() {
        return Err(Unsupported("clearing the login hint").into());
    }
    // badge_grant
    crate::badge_granter::queue_user_change(conn, s, user_id).await?;
    // refresh_avatar
    let attempted: Option<bool> = sqlx::query_scalar(
        "SELECT last_gravatar_download_attempt IS NOT NULL FROM user_avatars WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    match attempted {
        None => return Err(Unsupported("creating a missing user avatar").into()),
        Some(false) if s.get("automatically_download_gravatars")?.truthy() => {
            return Err(Unsupported("gravatar downloads").into());
        }
        Some(_) => {}
    }
    crate::posting::search_index::index_user(conn, s, user_id, username_lower, name).await
}

/// `CategoryUser.batch_set(user, level, category_ids)`
async fn batch_set_categories(
    conn: &mut PgConnection,
    user_id: i32,
    level: i32,
    ids: &[i32],
) -> Result<(), sqlx::Error> {
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM categories WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(&mut *conn)
        .await?;
    let mut changed = false;
    if !ids.is_empty() {
        let updated = sqlx::query(
            "UPDATE category_users SET notification_level = $3 \
             WHERE user_id = $1 AND category_id = ANY($2) AND notification_level <> $3",
        )
        .bind(user_id)
        .bind(&ids)
        .bind(level)
        .execute(&mut *conn)
        .await?;
        changed |= updated.rows_affected() > 0;
    }
    let deleted = sqlx::query(
        "DELETE FROM category_users \
         WHERE user_id = $1 AND notification_level = $3 AND NOT (category_id = ANY($2))",
    )
    .bind(user_id)
    .bind(&ids)
    .bind(level)
    .execute(&mut *conn)
    .await?;
    changed |= deleted.rows_affected() > 0;
    for id in &ids {
        let inserted = sqlx::query(
            "INSERT INTO category_users (user_id, category_id, notification_level) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(id)
        .bind(level)
        .execute(&mut *conn)
        .await?;
        changed |= inserted.rows_affected() > 0;
    }
    if changed {
        auto_watch(&mut *conn, user_id).await?;
        auto_track(&mut *conn, user_id).await?;
    }
    Ok(())
}

/// `CategoryUser.auto_watch(user_id:)`
pub(crate) async fn auto_watch(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    auto_watch_many(conn, &[user_id]).await
}

/// `CategoryUser.auto_watch(user_ids:)`
pub(crate) async fn auto_watch_many(
    conn: &mut PgConnection,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topic_users tu \
         SET notification_level = CASE WHEN should_track THEN $2 WHEN should_watch THEN $3 \
                                       ELSE notification_level END, \
             notifications_reason_id = CASE WHEN should_track THEN NULL \
                                            WHEN should_watch THEN $5 \
                                            ELSE notifications_reason_id END \
         FROM ( \
           SELECT tu1.topic_id, tu1.user_id, \
                  CASE WHEN cu.user_id IS NULL AND tu1.notification_level = $3 \
                            AND tu1.notifications_reason_id = $5 THEN true ELSE false END should_track, \
                  CASE WHEN cu.user_id IS NOT NULL AND tu1.notification_level IN ($4, $2) \
                       THEN true ELSE false END should_watch \
           FROM topic_users tu1 \
           JOIN topics t ON t.id = tu1.topic_id \
           LEFT JOIN category_users cu ON cu.category_id = t.category_id \
                AND cu.user_id = tu1.user_id AND cu.notification_level = $3 \
           WHERE COALESCE(tu1.notifications_reason_id, 0) <> $6 AND tu1.user_id = ANY($1) \
         ) AS X \
         WHERE X.topic_id = tu.topic_id AND X.user_id = tu.user_id \
           AND (should_watch OR should_track) AND tu.user_id = ANY($1)",
    )
    .bind(user_ids)
    .bind(TRACKING)
    .bind(WATCHING)
    .bind(REGULAR)
    .bind(AUTO_WATCH_CATEGORY)
    .bind(USER_CHANGED)
    .execute(conn)
    .await?;
    Ok(())
}

/// `CategoryUser.auto_track(user_id:)`
pub(crate) async fn auto_track(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    auto_track_many(conn, &[user_id]).await
}

/// `CategoryUser.auto_track(user_ids:)`
pub(crate) async fn auto_track_many(
    conn: &mut PgConnection,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE topic_users tu SET notification_level = $2, notifications_reason_id = $4 \
         FROM topics t, category_users cu \
         WHERE tu.topic_id = t.id AND cu.category_id = t.category_id AND cu.user_id = tu.user_id \
           AND cu.notification_level = $2 AND tu.notification_level = $3 \
           AND COALESCE(tu.notifications_reason_id, 0) <> $5 AND tu.user_id = ANY($1)",
    )
    .bind(user_ids)
    .bind(TRACKING)
    .bind(REGULAR)
    .bind(AUTO_TRACK_CATEGORY)
    .bind(USER_CHANGED)
    .execute(conn)
    .await?;
    Ok(())
}

/// `TrackedTopicsUpdater#call`
pub(crate) async fn tracked_topics_updater(
    conn: &mut PgConnection,
    user_id: i32,
    threshold: i64,
) -> Result<(), sqlx::Error> {
    if threshold < 0 {
        sqlx::query(
            "UPDATE topic_users SET notification_level = $2 \
             WHERE notifications_reason_id IS NULL AND user_id = $1",
        )
        .bind(user_id)
        .bind(REGULAR)
        .execute(conn)
        .await?;
    } else {
        sqlx::query(
            "UPDATE topic_users \
             SET notification_level = CASE WHEN total_msecs_viewed < $2 THEN $3 ELSE $4 END \
             WHERE notifications_reason_id IS NULL AND user_id = $1",
        )
        .bind(user_id)
        .bind(threshold)
        .bind(REGULAR)
        .bind(TRACKING)
        .execute(conn)
        .await?;
    }
    Ok(())
}

/// `update_muted_users` and `update_allowed_pm_users`: the list becomes
/// the named users, less the user themself.
async fn set_user_list(
    conn: &mut PgConnection,
    user: &Target,
    table: &str,
    column: &str,
    usernames: Option<String>,
) -> Result<(), sqlx::Error> {
    let usernames = usernames.unwrap_or_default();
    let desired: Vec<&str> = usernames
        .split(',')
        .filter(|u| !u.is_empty() && *u != user.username)
        .collect();
    let ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username = ANY($1)")
        .bind(&desired)
        .fetch_all(&mut *conn)
        .await?;
    sqlx::query(&format!(
        "DELETE FROM {table} WHERE user_id = $1 AND NOT ({column} = ANY($2))"
    ))
    .bind(user.id)
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    if !ids.is_empty() {
        sqlx::query(&format!(
            "INSERT INTO {table} (user_id, {column}, created_at, updated_at) \
             SELECT $1, id, now, now FROM users, clock_timestamp() AS now \
             WHERE id = ANY($2) ORDER BY id ON CONFLICT DO NOTHING"
        ))
        .bind(user.id)
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// The options half of the update and `user_option.save`: `None` when no
/// option was given, else the new `auto_track_topics_after_msecs` when it
/// changed.
async fn update_options(
    conn: &mut PgConnection,
    user_id: i32,
    attrs: &Map<String, Value>,
) -> Result<Option<Option<i64>>, AppError> {
    if !OPTION_ATTR.iter().any(|(k, _)| attrs.contains_key(*k)) {
        return Ok(None);
    }
    let row: Option<Value> =
        sqlx::query_scalar("SELECT to_jsonb(uo) FROM user_options uo WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(Value::Object(before)) = row else {
        return Err(Unsupported("a user without user options").into());
    };
    let mut after = before.clone();
    let text_size = |key: i64| {
        TEXT_SIZES
            .iter()
            .find(|(_, v)| *v == key)
            .map(|(n, _)| *n)
            .unwrap_or("")
    };
    if let Some(value) = attrs.get("text_size") {
        let current = text_size(before["text_size_key"].as_i64().unwrap_or(-1));
        if params::scalar(value).as_deref() != Some(current) {
            let seq = before["text_size_seq"].as_i64().unwrap_or(0);
            after.insert("text_size_seq".into(), json!(seq + 1));
        }
    }
    for (key, kind) in OPTION_ATTR {
        let Some(value) = attrs.get(*key) else {
            continue;
        };
        let column = match *key {
            "text_size" => "text_size_key",
            "title_count_mode" => "title_count_mode_key",
            "chat_send_shortcut" => "send_shortcut",
            k => k,
        };
        let new = match kind {
            Kind::Array if *key == "theme_ids" => {
                let ids: Vec<i64> = match value {
                    Value::Array(items) => items
                        .iter()
                        .filter_map(params::scalar)
                        .filter(|s| !crate::ruby::is_blank(s))
                        .map(|s| crate::ruby::to_i(&s))
                        .collect(),
                    _ => return Err(Unsupported("theme_ids that are not a list").into()),
                };
                // Guardian#allow_themes?; refused themes leave the option.
                if !allow_themes(&mut *conn, &ids).await? {
                    continue;
                }
                let new = json!(ids);
                if before["theme_ids"] != new {
                    let seq = before["theme_key_seq"].as_i64().unwrap_or(0);
                    after.insert("theme_key_seq".into(), json!(seq + 1));
                }
                new
            }
            // A string is a Postgres array literal; the empty one is
            // what the preferences send for seen_popups' nil.
            Kind::Array => {
                let Value::Array(items) = value else {
                    match params::scalar(value).as_deref() {
                        Some("") => {
                            after.insert(column.to_string(), json!([]));
                            continue;
                        }
                        _ => return Err(Unsupported("an array option given as a string").into()),
                    }
                };
                let items = items.iter().filter_map(params::scalar);
                if *key == "seen_popups" {
                    json!(items.map(|s| crate::ruby::to_i(&s)).collect::<Vec<_>>())
                } else {
                    json!(items.collect::<Vec<_>>())
                }
            }
            // `[true, false].include?(value)`: `to_s == "true"`; an unset
            // (nil) column takes the param as ActiveModel casts it.
            Kind::Bool => match &before[column] {
                Value::Bool(_) => json!(params::scalar(value).as_deref() == Some("true")),
                _ => cast_bool(value),
            },
            Kind::Int => cast_int(value),
            Kind::Text => match value {
                Value::Null => Value::Null,
                v => params::scalar(v).map(Value::from).unwrap_or(Value::Null),
            },
            Kind::Enum(values) => {
                let v = params::scalar(value).unwrap_or_default();
                let found = values
                    .iter()
                    .find(|(n, i)| *n == v || i.to_string() == v)
                    .map(|(_, i)| *i);
                match found {
                    Some(i) => json!(i),
                    None => return Err(Unsupported("an invalid option (the errors JSON)").into()),
                }
            }
        };
        after.insert(column.to_string(), new);
    }
    if attrs.contains_key("skip_new_user_tips") && after["skip_new_user_tips"] == json!(true) {
        after.insert("seen_popups".into(), json!([-1]));
    }
    // automatically disable digests when mailing_list_mode is enabled
    if after["mailing_list_mode"] == json!(true) {
        after.insert("email_digests".into(), json!(false));
    }
    // update_hide_profile_and_presence
    if after["hide_profile"] != before["hide_profile"]
        || after["hide_presence"] != before["hide_presence"]
    {
        let hidden = after["hide_profile"] == json!(true) || after["hide_presence"] == json!(true);
        after.insert("hide_profile_and_presence".into(), json!(hidden));
    }
    validate_options(&mut *conn, &before, &after).await?;

    let changed: Vec<&String> = after
        .keys()
        .filter(|k| after[k.as_str()] != before[k.as_str()])
        .collect();
    if !changed.is_empty() {
        let sets = changed
            .iter()
            .map(|c| format!("{c} = r.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        sqlx::query(&format!(
            "UPDATE user_options SET {sets} \
             FROM jsonb_populate_record(NULL::user_options, $2) r \
             WHERE user_options.user_id = $1"
        ))
        .bind(user_id)
        .bind(Value::Object(after.clone()))
        .execute(&mut *conn)
        .await?;
    }
    let auto_track = after["auto_track_topics_after_msecs"].as_i64();
    Ok(Some(
        (auto_track != before["auto_track_topics_after_msecs"].as_i64())
            .then_some(auto_track)
            .flatten(),
    ))
}

/// `Guardian#allow_themes?`: the first id a user selectable theme (or the
/// default one), the rest its components. No allowed theme repos are
/// configured, so the remote allowlist never applies.
async fn allow_themes(conn: &mut PgConnection, ids: &[i64]) -> Result<bool, sqlx::Error> {
    let Some((parent, components)) = ids.split_first() else {
        return Ok(true);
    };
    // Theme.user_theme_ids: user selectable, or the default (a hidden
    // setting, -1 unless overridden).
    let selectable: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1 AND (user_selectable OR id = \
         COALESCE((SELECT value::int FROM site_settings WHERE name = 'default_theme_id'), -1)))",
    )
    .bind(*parent as i32)
    .fetch_one(&mut *conn)
    .await?;
    if !selectable {
        return Ok(false);
    }
    let children: Vec<i32> =
        sqlx::query_scalar("SELECT child_theme_id FROM child_themes WHERE parent_theme_id = $1")
            .bind(*parent as i32)
            .fetch_all(&mut *conn)
            .await?;
    Ok(components.iter().all(|c| children.contains(&(*c as i32))))
}

/// UserOption's validations.
async fn validate_options(
    conn: &mut PgConnection,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Result<(), AppError> {
    let invalid = || Err(Unsupported("an invalid option (the errors JSON)").into());
    let changed = |key: &str| after.get(key) != before.get(key);
    let strings = |key: &str| -> Vec<String> {
        after[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            })
            .collect()
    };
    // hidden_composer_toolbar_buttons_are_bounded
    if changed("hidden_composer_toolbar_buttons") {
        let buttons = strings("hidden_composer_toolbar_buttons");
        if buttons.len() > 50 || buttons.iter().any(|b| b.chars().count() > 100) {
            return invalid();
        }
    }
    // understood_languages_are_supported
    if changed("understood_languages")
        && !strings("understood_languages")
            .iter()
            .all(|l| crate::themes::locale_available(l))
    {
        return invalid();
    }
    for key in ["email_level", "email_messages_level"] {
        if !after[key].as_i64().is_some_and(|l| (0..=2).contains(&l)) {
            return invalid();
        }
    }
    if let Some(tz) = after["timezone"].as_str().filter(|t| !blank(Some(t))) {
        let known: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_timezone_names WHERE name = $1)")
                .bind(tz)
                .fetch_one(conn)
                .await?;
        if !known {
            return Err(Unsupported("timezones outside the IANA database").into());
        }
    }
    Ok(())
}

/// The profile half and `user_profile.save` with its callbacks.
async fn update_profile(
    conn: &mut PgConnection,
    ctx: &Ctx<'_>,
    actor: &Guardian,
    user: &Target,
    attrs: &Map<String, Value>,
) -> Result<(), AppError> {
    let s = ctx.settings;
    #[derive(sqlx::FromRow)]
    struct Profile {
        bio_raw: Option<String>,
        bio_cooked: Option<String>,
        location: Option<String>,
        website: Option<String>,
        dismissed_banner_key: Option<i32>,
        profile_background_upload_id: Option<i32>,
        card_background_upload_id: Option<i32>,
    }
    let Some(before) = sqlx::query_as::<_, Profile>(
        "SELECT bio_raw, bio_cooked, location, website, dismissed_banner_key, \
                profile_background_upload_id, card_background_upload_id \
         FROM user_profiles WHERE user_id = $1",
    )
    .bind(user.id)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Err(Unsupported("a user without a profile").into());
    };
    let sso = s.get("enable_discourse_connect")?.truthy();
    let overrides = |field: &str| -> Result<bool, AppError> {
        Ok(sso
            && s.get(&format!("discourse_connect_overrides_{field}"))?
                .truthy())
    };
    let fetch = |key: &str, current: &Option<String>| {
        if attrs.contains_key(key) {
            text(attrs, key)
        } else {
            current.clone()
        }
    };
    let mut banner = before.dismissed_banner_key;
    if let Some(key) = text(attrs, "dismissed_banner_key").filter(|v| !blank(Some(v))) {
        banner = Some(crate::ruby::to_i(&key) as i32);
    }
    let bio_raw = if overrides("bio")? {
        before.bio_raw.clone()
    } else {
        fetch("bio_raw", &before.bio_raw)
    };
    let location = if overrides("location")? {
        before.location.clone()
    } else {
        fetch("location", &before.location)
    };
    let website = if overrides("website")? {
        before.website.clone()
    } else {
        format_url(fetch("website", &before.website).as_deref())
    };
    // The backgrounds: "" (or a user the guardian refuses) clears one, a
    // known upload's url sets it, anything else leaves it.
    let mut backgrounds = [
        before.profile_background_upload_id,
        before.card_background_upload_id,
    ];
    for (i, (key, groups)) in [
        (
            "profile_background_upload_url",
            "profile_background_allowed_groups",
        ),
        (
            "card_background_upload_url",
            "user_card_background_allowed_groups",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if !attrs.contains_key(key) {
            continue;
        }
        let allowed =
            actor.is_staff() || (actor.is_me(user.id) && actor.in_setting_groups(s, groups)?);
        match text(attrs, key) {
            Some(url) if url.is_empty() || !allowed => backgrounds[i] = None,
            Some(url) => {
                if let Some((id, _)) =
                    crate::upload_references::get_from_url(&mut *conn, &url).await?
                {
                    backgrounds[i] = Some(id);
                }
            }
            None if !allowed => backgrounds[i] = None,
            None => {}
        }
    }
    let [profile_background, card_background] = backgrounds;

    // validations
    let invalid = || Err(Unsupported("an invalid profile (the errors JSON)").into());
    let too_long = |v: &Option<String>| v.as_ref().is_some_and(|v| v.chars().count() > 3000);
    if too_long(&bio_raw) || too_long(&location) {
        return invalid();
    }
    // watched_words: true on bio_raw and location.
    for v in [&bio_raw, &location].into_iter().flatten() {
        if crate::word_watcher::block_error(&mut *conn, s, ctx.i18n, v)
            .await?
            .is_some()
        {
            return invalid();
        }
    }
    if website != before.website
        && let Some(w) = website.as_deref().filter(|w| !blank(Some(w)))
    {
        if w.chars().count() > 3000 || !valid_url(w) {
            return invalid();
        }
        let allowed = s.get("allowed_user_website_domains")?.to_s();
        if !allowed.is_empty() {
            let host = http_host(w);
            if !allowed.split('|').any(|d| Some(d) == host.as_deref()) {
                return invalid();
            }
        }
    }

    // before_save :apply_watched_words, if: :location?
    let location = match location {
        Some(l) if !blank(Some(&l)) => {
            Some(crate::word_watcher::apply_to_text(&mut *conn, s, &l).await?)
        }
        l => l,
    };
    // before_save :cook
    let bio_changed = bio_raw != before.bio_raw;
    let mut bio_cooked = before.bio_cooked.clone();
    let mut cooked_version = false;
    match bio_raw.as_deref().filter(|b| !blank(Some(b))) {
        Some(raw) if bio_changed => {
            if raw.contains("upload://") || raw.contains("/uploads/") {
                return Err(Unsupported("uploads in the bio (UploadReference)").into());
            }
            let opts = crate::pretty_text::MarkdownOptions {
                omit_nofollow: user.has_trust_level(3) && !s.get("tl3_links_no_follow")?.truthy(),
                ..Default::default()
            };
            bio_cooked = Some(crate::pretty_text::cook(ctx.host, raw, &opts).await?);
            cooked_version = true;
        }
        Some(_) => {}
        None => bio_cooked = None,
    }

    if bio_changed
        || bio_cooked != before.bio_cooked
        || location != before.location
        || website != before.website
        || banner != before.dismissed_banner_key
        || profile_background != before.profile_background_upload_id
        || card_background != before.card_background_upload_id
    {
        sqlx::query(
            "UPDATE user_profiles SET bio_raw = $2, bio_cooked = $3, location = $4, website = $5, \
                    dismissed_banner_key = $6, \
                    bio_cooked_version = CASE WHEN $7 THEN 1 ELSE bio_cooked_version END, \
                    profile_background_upload_id = $8, card_background_upload_id = $9 \
             WHERE user_id = $1",
        )
        .bind(user.id)
        .bind(&bio_raw)
        .bind(&bio_cooked)
        .bind(&location)
        .bind(&website)
        .bind(banner)
        .bind(cooked_version)
        .bind(profile_background)
        .bind(card_background)
        .execute(&mut *conn)
        .await?;
    }
    let backgrounds_changed = profile_background != before.profile_background_upload_id
        || card_background != before.card_background_upload_id;
    if bio_changed {
        // pull_hotlinked_image
        let grace = s.get("editing_grace_period")?.to_i();
        crate::jobs::enqueue_in(
            &mut *conn,
            grace,
            "pull_user_profile_hotlinked_images",
            json!({ "user_id": user.id }),
        )
        .await?;
    }
    // UploadReference.ensure_exist!: the backgrounds and the bio's
    // uploads (none: refused above).
    if bio_changed || backgrounds_changed {
        let ids: Vec<i64> = [profile_background, card_background]
            .into_iter()
            .flatten()
            .map(i64::from)
            .collect();
        crate::upload_references::ensure_exist_many(
            &mut *conn,
            "UserProfile",
            user.id.into(),
            &ids,
        )
        .await?;
    }
    Ok(())
}
