//! What static/js/chat-channel-menus.js reads: the channel list options
//! menu's and sidebar context menu's strings, the create channel modal's,
//! and the member's list preferences (ChatChannelListPreferences).

use serde_json::json;
use sqlx::PgConnection;

use crate::AppError;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::escape;

/// chat-channel-menus.js's `data-config` for a member who can chat.
pub async fn script_attrs(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    i18n: &I18n,
) -> Result<String, AppError> {
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
    let options = match guardian.user_id() {
        Some(id) => super::options(&mut *conn, id).await?,
        None => None,
    };
    let name = |names: &[&str], v: Option<i32>, default: &str| -> String {
        v.and_then(|v| usize::try_from(v).ok())
            .and_then(|i| names.get(i))
            .map(|n| n.to_string())
            .unwrap_or_else(|| default.to_string())
    };
    let pref = |filter: Option<i32>, sort: Option<i32>| {
        json!({
            "filter": name(&super::LIST_FILTERS, filter, "all"),
            "sort": name(&super::LIST_SORTS, sort, "alphabetical"),
        })
    };
    let o = options.as_ref();
    let mut strings = serde_json::Map::new();
    for (key, path) in [
        ("create", "chat.channels_list_popup.create"),
        ("browse", "chat.channels_list_popup.browse"),
        ("new_dm", "chat.direct_messages.new"),
        ("new_group", "chat.direct_messages.new_group"),
        ("filter", "chat.channel_list.options.filter"),
        ("sort", "chat.channel_list.options.sort"),
        ("current_filter", "chat.channel_list.options.current_filter"),
        ("current_sort", "chat.channel_list.options.current_sort"),
        ("filter_title", "chat.channel_list.filter.title"),
        ("filter_all", "chat.channel_list.filter.all"),
        ("filter_active", "chat.channel_list.filter.active"),
        ("filter_unread", "chat.channel_list.filter.unread"),
        ("filter_mentions", "chat.channel_list.filter.mentions"),
        ("sort_title", "chat.channel_list.sort.title"),
        ("sort_alphabetical", "chat.channel_list.sort.alphabetical"),
        (
            "sort_recent_activity",
            "chat.channel_list.sort.recent_activity",
        ),
        ("sort_priority", "chat.channel_list.sort.priority"),
        (
            "notifications",
            "chat.channel_settings.notification_settings_context",
        ),
        ("settings", "chat.channel_settings.title"),
        ("star", "chat.channel_settings.star_channel"),
        ("unstar", "chat.channel_settings.unstar_channel"),
        ("leave", "chat.channel_settings.leave_channel"),
        ("close_dm", "chat.channel_settings.close_channel"),
        ("level_never", "chat.notification_levels.never"),
        ("level_mention", "chat.notification_levels.mention"),
        ("level_always", "chat.notification_levels.always"),
        ("mute", "chat.settings.mute"),
        ("unmute", "chat.settings.unmute"),
        ("title", "chat.create_channel.title"),
        ("name", "chat.create_channel.name"),
        ("slug", "chat.create_channel.slug"),
        (
            "slug_description",
            "chat.channel_edit_name_slug_modal.slug_description",
        ),
        ("description", "chat.create_channel.description"),
        ("category", "chat.create_channel.choose_category.label"),
        ("category_none", "chat.create_channel.choose_category.none"),
        (
            "hint_1_group",
            "chat.create_channel.choose_category.hint_1_group",
        ),
        (
            "hint_2_groups",
            "chat.create_channel.choose_category.hint_2_groups",
        ),
        (
            "hint_multiple_one",
            "chat.create_channel.choose_category.hint_multiple_groups.one",
        ),
        (
            "hint_multiple_other",
            "chat.create_channel.choose_category.hint_multiple_groups.other",
        ),
        ("emoji", "chat.create_channel.emoji.label"),
        ("emoji_reset", "chat.create_channel.emoji.reset"),
        ("auto_join_label", "chat.settings.auto_join_users_label"),
        ("auto_join_info", "chat.settings.auto_join_users_info"),
        (
            "auto_join_info_no_category",
            "chat.settings.auto_join_users_info_no_category",
        ),
        (
            "public_warning",
            "chat.create_channel.auto_join_users.public_category_warning",
        ),
        (
            "warning_1_group_one",
            "chat.create_channel.auto_join_users.warning_1_group.one",
        ),
        (
            "warning_1_group_other",
            "chat.create_channel.auto_join_users.warning_1_group.other",
        ),
        (
            "warning_2_groups_one",
            "chat.create_channel.auto_join_users.warning_2_groups.one",
        ),
        (
            "warning_2_groups_other",
            "chat.create_channel.auto_join_users.warning_2_groups.other",
        ),
        ("threading", "chat.create_channel.threading.label"),
        (
            "threading_description",
            "chat.settings.channel_threading_description",
        ),
        ("create_button", "chat.create_channel.create"),
        ("required", "form_kit.errors.required"),
        ("generic_error", "generic_error"),
    ] {
        strings.insert(key.into(), json!(t(path)));
    }
    // default_hint: its link and category filled as the modal does.
    strings.insert(
        "default_hint".into(),
        json!(
            t("chat.create_channel.choose_category.default_hint")
                .replace("%{link}", "/categories")
                .replace("%{category}", "category")
        ),
    );
    // warning_multiple_groups_MF, its plural branches spelled out for the
    // English message.
    strings.insert(
        "warning_multiple".into(),
        json!("Automatically add %{userCount} %{users} from %{groupName} and %{groupCount} other %{groups}?"),
    );
    strings.insert("user_one".into(), json!("user"));
    strings.insert("user_other".into(), json!("users"));
    strings.insert("group_one".into(), json!("group"));
    strings.insert("group_other".into(), json!("groups"));
    let config = json!({
        "strings": strings,
        "staff": guardian.is_staff(),
        "username": guardian.user().map(|u| u.username.as_str()),
        "canDirectMessage": guardian.is_staff() || super::can_direct_message(settings, guardian)?,
        "canGroupChat": guardian.is_staff()
            || settings.get("chat_max_direct_message_users")?.to_i() > 1,
        "autoJoinAvailable": settings.get("max_chat_auto_joined_users")?.to_i() > 0,
        "prefs": {
            "channels": pref(o.map(|o| o.chat_channel_list_filter), o.map(|o| o.chat_channel_list_sort)),
            "starred": pref(o.map(|o| o.chat_channel_list_filter_starred), o.map(|o| o.chat_channel_list_sort_starred)),
            "dms": pref(o.map(|o| o.chat_channel_list_filter_dms), o.map(|o| o.chat_channel_list_sort_dms)),
        },
    });
    Ok(format!(" data-config=\"{}\"", escape(&config.to_string())))
}
