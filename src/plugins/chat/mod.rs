//! chat: Discourse's chat plugin, a channel per category (CategoryChannel)
//! or per set of users (DirectMessageChannel), on its own tables
//! (`chat_channels`, `user_chat_channel_memberships`, `chat_messages`...).
//!
//! This module holds what the rest of the app asks of it: whether the
//! plugin is on, the guardian's chat checks, the channels a viewer may see
//! (ChannelFetcher.generate_allowed_channel_ids_sql), and the keys it adds
//! to users, the current user and their options.

pub mod actions_view;
pub mod auto_join;
pub mod channels;
pub mod cook;
pub mod create;
pub mod membership;
pub mod messages;
pub mod modify;
pub mod page;
pub mod publisher;
pub mod view;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};
use crate::{AppError, Unsupported};

/// `enabled_site_setting :chat_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("chat_enabled")?.truthy())
}

/// `Guardian#can_chat?`: members of chat_allowed_groups, and bots. A
/// user in anonymous mode chats as their master account; not ported.
pub async fn can_chat(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(false);
    };
    if user_id <= 0 {
        return Ok(true);
    }
    let shadow: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM anonymous_users WHERE user_id = $1)")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if shadow {
        return Err(Unsupported("chat in anonymous mode").into());
    }
    Ok(guardian.in_setting_groups(settings, "chat_allowed_groups")?)
}

/// `Guardian#can_direct_message?`
pub fn can_direct_message(
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, SettingError> {
    guardian.in_setting_groups(settings, "direct_message_enabled_groups")
}

/// `Category.scoped_to_permissions(guardian, permissions)` as a condition
/// on `categories`, the ids written in.
pub(crate) fn categories_scoped_to(guardian: &Guardian, permissions: &str) -> String {
    match guardian.user() {
        _ if guardian.is_admin() => "TRUE".to_string(),
        None => "1 = 0".to_string(),
        Some(user) => format!(
            "(({staged} AND LENGTH(COALESCE(categories.email_in, '')) > 0 AND categories.email_in_allow_strangers) \
              OR categories.id NOT IN (SELECT category_id FROM category_groups) \
              OR categories.id IN (SELECT category_id FROM category_groups \
                 WHERE permission_type IN ({permissions}) \
                   AND (group_id = 0 OR group_id IN (SELECT group_id FROM group_users WHERE user_id = {id}))))",
            staged = user.staged,
            id = user.id,
        ),
    }
}

/// `ChannelFetcher.generate_allowed_channel_ids_sql`: the ids of the
/// category channels whose category the viewer may post in, and (unless
/// excluded) the direct message channels they are in. Anonymous access to
/// public channels is not ported.
pub fn allowed_channel_ids_sql(
    settings: &SiteSettings,
    guardian: &Guardian,
    exclude_dm_channels: bool,
) -> Result<String, AppError> {
    if guardian.is_anonymous()
        && settings.get("enable_public_channels")?.truthy()
        && settings
            .group_ids("chat_allowed_groups")?
            .contains(&crate::guardian::auto_groups::ANONYMOUS_USERS)
    {
        return Err(Unsupported("anonymous access to chat channels").into());
    }
    // POST_CREATION_PERMISSIONS: full (1) and create_post (2).
    let mut sql = format!(
        "SELECT chat_channels.id FROM categories \
         INNER JOIN chat_channels ON chat_channels.chatable_id = categories.id \
           AND chat_channels.chatable_type = 'Category' \
         WHERE {}",
        categories_scoped_to(guardian, "1, 2")
    );
    if let (false, Some(user_id)) = (exclude_dm_channels, guardian.user_id()) {
        sql.push_str(&format!(
            " UNION SELECT chat_channels.id FROM chat_channels \
               INNER JOIN direct_message_channels ON direct_message_channels.id = chat_channels.chatable_id \
                 AND chat_channels.chatable_type = 'DirectMessage' \
               INNER JOIN direct_message_users \
                 ON direct_message_users.direct_message_channel_id = direct_message_channels.id \
               WHERE direct_message_users.user_id = {user_id}"
        ));
    }
    Ok(sql)
}

/// `has_joinable_public_channels`: an open public channel the viewer may
/// join and doesn't follow (ChannelFetcher.secured_public_channel_search).
pub async fn has_joinable_public_channels(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, AppError> {
    if !settings.get("enable_public_channels")?.truthy() {
        return Ok(false);
    }
    let allowed = allowed_channel_ids_sql(settings, guardian, true)?;
    let user_id = guardian.user_id().unwrap_or(0);
    // Chat::Channel.public_channel_chatable_types: Category; status open (0).
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM chat_channels \
         LEFT JOIN categories ON categories.id = chat_channels.chatable_id \
           AND chat_channels.chatable_type = 'Category' \
         WHERE chat_channels.deleted_at IS NULL AND chat_channels.chatable_type = 'Category' \
           AND chat_channels.id IN ({allowed}) AND chat_channels.status = 0 \
           AND chat_channels.id NOT IN (SELECT chat_channel_id FROM user_chat_channel_memberships uccm \
             WHERE uccm.chat_channel_id = chat_channels.id AND following IS TRUE AND user_id = $1))"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?)
}

/// The chat columns of user_options.
#[derive(sqlx::FromRow)]
pub(crate) struct ChatOptions {
    pub(crate) chat_enabled: bool,
    pub(crate) chat_sound: Option<String>,
    pub(crate) ignore_channel_wide_mention: Option<bool>,
    pub(crate) show_thread_title_prompts: bool,
    pub(crate) chat_announce_new_messages: bool,
    pub(crate) chat_channel_list_filter: i32,
    pub(crate) chat_channel_list_sort: i32,
    pub(crate) chat_channel_list_sort_starred: i32,
    pub(crate) chat_channel_list_sort_dms: i32,
    pub(crate) chat_channel_list_filter_starred: i32,
    pub(crate) chat_channel_list_filter_dms: i32,
    pub(crate) chat_new_message_sound: bool,
    pub(crate) chat_email_frequency: i32,
    pub(crate) chat_header_indicator_preference: i32,
    pub(crate) chat_separate_sidebar_mode: i32,
    pub(crate) send_shortcut: i32,
    pub(crate) chat_quick_reaction_type: i32,
    pub(crate) chat_quick_reactions_custom: Option<String>,
    pub(crate) dismissed_channel_retention_reminder: Option<bool>,
}

pub(crate) async fn options(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<Option<ChatOptions>, sqlx::Error> {
    sqlx::query_as(
        "SELECT chat_enabled, chat_sound, ignore_channel_wide_mention, show_thread_title_prompts, \
                chat_announce_new_messages, chat_channel_list_filter, chat_channel_list_sort, \
                chat_channel_list_sort_starred, chat_channel_list_sort_dms, chat_channel_list_filter_starred, \
                chat_channel_list_filter_dms, chat_new_message_sound, chat_email_frequency, \
                chat_header_indicator_preference, chat_separate_sidebar_mode, send_shortcut, \
                chat_quick_reaction_type, chat_quick_reactions_custom, dismissed_channel_retention_reminder \
         FROM user_options WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await
}

/// The enums of Chat::UserOptionExtension (and core's send_shortcut), by
/// stored value.
fn enum_name(names: &[&'static str], value: i32) -> Value {
    names
        .get(usize::try_from(value).unwrap_or(usize::MAX))
        .map(|n| json!(n))
        .unwrap_or(Value::Null)
}
pub(crate) const LIST_FILTERS: [&str; 4] = ["all", "active", "unread", "mentions"];
pub(crate) const LIST_SORTS: [&str; 3] = ["alphabetical", "recent_activity", "priority"];
const EMAIL_FREQUENCIES: [&str; 2] = ["never", "when_away"];
pub(crate) const HEADER_INDICATORS: [&str; 4] =
    ["all_new", "dm_and_mentions", "never", "only_mentions"];
pub(crate) const SIDEBAR_MODES: [&str; 4] = ["default", "never", "always", "fullscreen"];
pub(crate) const SEND_SHORTCUTS: [&str; 2] = ["enter", "meta_enter"];
const QUICK_REACTION_TYPES: [&str; 2] = ["frequent", "custom"];

/// The keys chat adds to UserOptionSerializer, in their order.
pub async fn user_option_keys(
    conn: &mut PgConnection,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), AppError> {
    let Some(o) = options(conn, user_id).await? else {
        return Ok(());
    };
    out.insert("chat_enabled".into(), json!(o.chat_enabled));
    if let Some(sound) = o.chat_sound.filter(|s| !s.trim().is_empty()) {
        out.insert("chat_sound".into(), json!(sound));
    }
    out.insert(
        "ignore_channel_wide_mention".into(),
        json!(o.ignore_channel_wide_mention),
    );
    out.insert(
        "show_thread_title_prompts".into(),
        json!(o.show_thread_title_prompts),
    );
    out.insert(
        "chat_announce_new_messages".into(),
        json!(o.chat_announce_new_messages),
    );
    for (key, value, names) in [
        (
            "chat_channel_list_filter",
            o.chat_channel_list_filter,
            &LIST_FILTERS[..],
        ),
        (
            "chat_channel_list_sort",
            o.chat_channel_list_sort,
            &LIST_SORTS[..],
        ),
        (
            "chat_channel_list_sort_starred",
            o.chat_channel_list_sort_starred,
            &LIST_SORTS[..],
        ),
        (
            "chat_channel_list_sort_dms",
            o.chat_channel_list_sort_dms,
            &LIST_SORTS[..],
        ),
        (
            "chat_channel_list_filter_starred",
            o.chat_channel_list_filter_starred,
            &LIST_FILTERS[..],
        ),
        (
            "chat_channel_list_filter_dms",
            o.chat_channel_list_filter_dms,
            &LIST_FILTERS[..],
        ),
    ] {
        out.insert(key.into(), enum_name(names, value));
    }
    out.insert(
        "chat_new_message_sound".into(),
        json!(o.chat_new_message_sound),
    );
    out.insert(
        "chat_email_frequency".into(),
        enum_name(&EMAIL_FREQUENCIES, o.chat_email_frequency),
    );
    out.insert(
        "chat_header_indicator_preference".into(),
        enum_name(&HEADER_INDICATORS, o.chat_header_indicator_preference),
    );
    out.insert(
        "chat_separate_sidebar_mode".into(),
        enum_name(&SIDEBAR_MODES, o.chat_separate_sidebar_mode),
    );
    out.insert(
        "chat_send_shortcut".into(),
        enum_name(&SEND_SHORTCUTS, o.send_shortcut),
    );
    out.insert(
        "chat_quick_reaction_type".into(),
        enum_name(&QUICK_REACTION_TYPES, o.chat_quick_reaction_type),
    );
    out.insert(
        "chat_quick_reactions_custom".into(),
        json!(o.chat_quick_reactions_custom),
    );
    Ok(())
}

/// The keys chat adds to CurrentUserOptionSerializer, in their order.
pub async fn current_user_option_keys(
    conn: &mut PgConnection,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), AppError> {
    let Some(o) = options(conn, user_id).await? else {
        return Ok(());
    };
    out.insert(
        "show_thread_title_prompts".into(),
        json!(o.show_thread_title_prompts),
    );
    for (key, value, names) in [
        (
            "chat_channel_list_filter",
            o.chat_channel_list_filter,
            &LIST_FILTERS[..],
        ),
        (
            "chat_channel_list_sort",
            o.chat_channel_list_sort,
            &LIST_SORTS[..],
        ),
        (
            "chat_channel_list_sort_starred",
            o.chat_channel_list_sort_starred,
            &LIST_SORTS[..],
        ),
        (
            "chat_channel_list_sort_dms",
            o.chat_channel_list_sort_dms,
            &LIST_SORTS[..],
        ),
        (
            "chat_channel_list_filter_starred",
            o.chat_channel_list_filter_starred,
            &LIST_FILTERS[..],
        ),
        (
            "chat_channel_list_filter_dms",
            o.chat_channel_list_filter_dms,
            &LIST_FILTERS[..],
        ),
    ] {
        out.insert(key.into(), enum_name(names, value));
    }
    out.insert(
        "chat_announce_new_messages".into(),
        json!(o.chat_announce_new_messages),
    );
    out.insert(
        "chat_new_message_sound".into(),
        json!(o.chat_new_message_sound),
    );
    out.insert(
        "chat_header_indicator_preference".into(),
        enum_name(&HEADER_INDICATORS, o.chat_header_indicator_preference),
    );
    out.insert(
        "chat_separate_sidebar_mode".into(),
        enum_name(&SIDEBAR_MODES, o.chat_separate_sidebar_mode),
    );
    out.insert(
        "chat_send_shortcut".into(),
        enum_name(&SEND_SHORTCUTS, o.send_shortcut),
    );
    out.insert(
        "chat_quick_reaction_type".into(),
        enum_name(&QUICK_REACTION_TYPES, o.chat_quick_reaction_type),
    );
    out.insert(
        "chat_quick_reactions_custom".into(),
        json!(o.chat_quick_reactions_custom),
    );
    Ok(())
}

/// The keys chat adds to CurrentUserSerializer, in their order.
pub async fn current_user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    out: &mut Map<String, Value>,
) -> Result<(), AppError> {
    let Some(user) = guardian.user() else {
        return Ok(());
    };
    let user_id = user.id;
    let staff = user.admin || user.moderator;
    let o = options(&mut *conn, user_id).await?;
    let can_chat = enabled(settings)? && can_chat(&mut *conn, settings, guardian).await?;
    let has_chat_enabled = can_chat && o.as_ref().is_some_and(|o| o.chat_enabled);
    if can_chat {
        out.insert("can_chat".into(), json!(true));
    }
    if has_chat_enabled && can_direct_message(settings, guardian)? {
        out.insert("can_direct_message".into(), json!(true));
    }
    if has_chat_enabled {
        out.insert("has_chat_enabled".into(), json!(true));
        if let Some(sound) = o
            .as_ref()
            .and_then(|o| o.chat_sound.clone())
            .filter(|s| !s.is_empty())
        {
            out.insert("chat_sound".into(), json!(sound));
        }
        if staff
            && !o
                .as_ref()
                .and_then(|o| o.dismissed_channel_retention_reminder)
                .unwrap_or(false)
            && settings.get("chat_channel_retention_days")?.to_i() != 0
        {
            out.insert("needs_channel_retention_reminder".into(), json!(true));
        }
        if settings.get("chat_dm_retention_days")?.to_i() != 0 {
            return Err(Unsupported("chat's direct message retention reminder").into());
        }
    }
    out.insert(
        "has_joinable_public_channels".into(),
        json!(has_joinable_public_channels(&mut *conn, settings, guardian).await?),
    );
    if has_chat_enabled {
        let drafts: Vec<(i64, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT chat_channel_id::bigint, data, thread_id::bigint FROM chat_drafts \
             WHERE user_id = $1 ORDER BY updated_at DESC LIMIT 20",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        out.insert(
            "chat_drafts".into(),
            json!(
                drafts
                    .into_iter()
                    .map(|(channel_id, data, thread_id)| json!({
                        "channel_id": channel_id,
                        "data": data,
                        "thread_id": thread_id,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
    }
    Ok(())
}

/// `can_chat_user` on UserCardSerializer and HiddenProfileSerializer:
/// whether the viewer may start a direct message with the user.
pub async fn can_chat_user(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    target_id: i32,
) -> Result<bool, AppError> {
    if !enabled(settings)? {
        return Ok(false);
    }
    let Some(viewer_id) = guardian.user_id() else {
        return Ok(false);
    };
    let enabled_for = |id: i32| {
        sqlx::query_scalar::<_, Option<bool>>(
            "SELECT chat_enabled FROM user_options WHERE user_id = $1",
        )
        .bind(id)
    };
    let viewer_on = enabled_for(viewer_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten()
        .unwrap_or(false);
    let target_on = enabled_for(target_id)
        .fetch_optional(&mut *conn)
        .await?
        .flatten()
        .unwrap_or(false);
    if !viewer_on || !target_on {
        return Ok(false);
    }
    if !can_direct_message(settings, guardian)? {
        return Ok(false);
    }
    // Guardian.new(object).can_chat?
    let Some(target) = crate::session::current::SessionUser::load(&mut *conn, target_id).await?
    else {
        return Ok(false);
    };
    let target_guardian = Guardian::for_user(&mut *conn, &target).await?;
    if !can_chat(&mut *conn, settings, &target_guardian).await? {
        return Ok(false);
    }
    // recipient_allows_direct_messages?
    if guardian.is_staff() {
        return Ok(true);
    }
    let (allows, suspended): (bool, bool) = sqlx::query_as(
        "SELECT COALESCE(uo.allow_private_messages, TRUE), \
                COALESCE(u.suspended_till > now(), FALSE) \
         FROM users u LEFT JOIN user_options uo ON uo.user_id = u.id WHERE u.id = $1",
    )
    .bind(target_id)
    .fetch_one(&mut *conn)
    .await?;
    if !allows || suspended {
        return Ok(false);
    }
    // UserCommScreener#disallowing_pms_from_actor?: the target ignores or
    // mutes the viewer, or only allows some users and not them. The acting
    // user is never among its targets.
    if target_id == viewer_id {
        return Ok(true);
    }
    let disallowing: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM ignored_users WHERE user_id = $1 AND ignored_user_id = $2) \
             OR EXISTS (SELECT 1 FROM muted_users WHERE user_id = $1 AND muted_user_id = $2) \
             OR (COALESCE((SELECT enable_allowed_pm_users FROM user_options WHERE user_id = $1), FALSE) \
                 AND NOT EXISTS (SELECT 1 FROM allowed_pm_users WHERE user_id = $1 AND allowed_pm_user_id = $2))",
    )
    .bind(target_id)
    .bind(viewer_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(!disallowing)
}

/// The chat service's `userCanDirectMessage`: `userCanChat` (the plugin
/// on, and has_chat_enabled) and `can_direct_message`.
pub async fn user_can_direct_message(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, AppError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(false);
    };
    if !enabled(settings)? || !can_chat(&mut *conn, settings, guardian).await? {
        return Ok(false);
    }
    let chat_enabled = options(&mut *conn, user_id)
        .await?
        .is_some_and(|o| o.chat_enabled);
    Ok(chat_enabled && can_direct_message(settings, guardian)?)
}
