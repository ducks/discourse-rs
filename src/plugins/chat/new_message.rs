//! Chat's new message modal (ChatModalNewMessage over MessageCreator) and
//! the openers that start a direct message without it (the profile's and
//! the user card's ChatDirectMessageButton). static/js/chat-new-message.js
//! draws the modal; its script tag carries what the modal reads of the
//! site and the member, rendered here.

use serde_json::json;

use crate::AppError;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::escape;

/// chat-new-message.js's `data-config` for a member who can chat: the
/// modal's strings, chatGuardian.canUseGroupChat,
/// chat_max_direct_message_users, chat.userCanDirectMessage,
/// enable_public_channels (the modal's shouldRender), the display name
/// settings ChatUserDisplayName and ChannelName read, the member (a DM
/// with no one else is titled after them), and the emoji URLs a
/// channel's emoji is drawn with.
pub fn script_attrs(
    settings: &SiteSettings,
    guardian: &Guardian,
    i18n: &I18n,
    base_path: &str,
) -> Result<String, AppError> {
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
    let user = guardian.user();
    let emojis = settings.emojis();
    let config = json!({
        "strings": {
            "filter": t("chat.new_message_modal.filter"),
            "newGroupChat": t("chat.new_message_modal.new_group_chat"),
            "disabledUser": t("chat.new_message_modal.disabled_user"),
            "groupTooMany": t("chat.new_message_modal.group_with_too_many_members"),
            "cantAddMore": t("chat.new_message_modal.cant_add_more_members"),
            "addMembers": t("chat.new_message_modal.user_search_placeholder"),
            "createGroup": t("chat.new_message_modal.create_new_group_chat"),
            "groupName": t("chat.direct_message_creator.group_name"),
            "membersOne": t("chat.direct_message_creator.members_counter.one"),
            "membersOther": t("chat.direct_message_creator.members_counter.other"),
            "cancel": t("cancel"),
            "title": t("chat.new_message_modal.title"),
        },
        "canGroupChat": guardian.is_staff()
            || settings.get("chat_max_direct_message_users")?.to_i() > 1,
        "maxDmUsers": settings.get("chat_max_direct_message_users")?.to_i(),
        "canDirectMessage": guardian.is_staff() || super::can_direct_message(settings, guardian)?,
        "publicChannels": settings.get("enable_public_channels")?.truthy(),
        "prioritizeUsername": settings.get("prioritize_username_in_ux")?.truthy(),
        "prefersName": settings.get("enable_names")?.truthy()
            && settings.get("display_name_on_posts")?.truthy()
            && !settings.get("prioritize_username_in_ux")?.truthy(),
        "username": user.map(|u| u.username.as_str()),
        "emojiUrl": emojis.url_template(base_path),
        "customEmoji": emojis.custom_urls(),
    });
    Ok(format!(" data-config=\"{}\"", escape(&config.to_string())))
}
