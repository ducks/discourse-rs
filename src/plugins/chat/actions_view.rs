//! ChatMessageActionsDesktop and ChatComposerMessageDetails, drawn once
//! per channel page as a template that static/js/chat.js clones for the
//! message under the pointer, as Ember renders the toolbar for the active
//! message. What Ember decides per message (ChatMessageInteractor: the
//! quick reactions, from the browser's own emoji usage, and which
//! secondary actions apply) chat.js decides from the template's data and
//! the message's.

use serde_json::Value;

use crate::post_view::d_icon;
use crate::topic_list_view::escape;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// What the template needs besides the channel's payload.
pub struct Toolbar<'a> {
    pub t: &'a dyn Fn(&str) -> String,
    pub base_path: &'a str,
    pub emoji_set: &'a str,
    pub viewer_id: i64,
    pub staff: bool,
    /// `chat_quick_reactions_custom` when the member's
    /// chat_quick_reaction_type is custom.
    pub quick_custom: Option<&'a str>,
    pub default_reactions: &'a str,
    /// The chat_pinned_messages setting.
    pub pins: bool,
}

/// A select-kit row of the secondary actions (DropdownSelectBox).
fn row(value: &str, name: &str, icon: &str) -> String {
    format!(
        "<li aria-checked=\"false\" role=\"menuitemradio\" data-name=\"{n}\" data-value=\"{value}\" title=\"{n}\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row\"><div class=\"icons\"><span class=\"selection-indicator\"></span>{}</div><div class=\"texts\"><span class=\"name\">{n}</span></div></li>",
        d_icon(icon, None),
        n = escape(name)
    )
}

/// The toolbar's template: the channel's permissions (canModifyMessages,
/// canModerate, canDeleteSelf/Others, canFlag, canManagePins) and the
/// strings chat.js fills in.
pub fn template(tb: &Toolbar, c: &Value, following: bool) -> String {
    let t = tb.t;
    let meta = &c["meta"];
    let status = s(&c["status"]);
    let can_modify = status == "open" || (tb.staff && status == "closed");
    let flag = |b: bool| if b { "true" } else { "false" };
    let mut out = format!(
        "<template class=\"chat-message-actions-template\" data-viewer-id=\"{}\" data-staff=\"{}\" data-can-modify=\"{}\" data-following=\"{}\" data-can-moderate=\"{}\" data-can-delete-self=\"{}\" data-can-delete-others=\"{}\" data-can-flag=\"{}\" data-can-pin=\"{}\" data-quick-custom=\"{}\" data-quick-defaults=\"{}\" data-tonable=\"{}\" data-emoji-url=\"{}\" data-label-add=\"{}\" data-label-remove=\"{}\" data-label-bookmark=\"{}\" data-label-bookmark-edit=\"{}\" data-label-created-generic=\"{}\" data-label-created-reminder=\"{}\" data-label-reminder-today=\"{}\" data-label-reminder-tomorrow=\"{}\" data-label-reminder-at=\"{}\" data-label-link-copied=\"{}\" data-label-too-long=\"{}\">",
        tb.viewer_id,
        flag(tb.staff),
        flag(can_modify),
        flag(following),
        flag(meta["can_moderate"] == true),
        flag(meta["can_delete_self"] == true),
        flag(meta["can_delete_others"] == true),
        flag(meta["can_flag"] == true),
        flag(tb.pins && meta["can_manage_pins"] == true),
        escape(tb.quick_custom.unwrap_or_default()),
        escape(tb.default_reactions),
        escape(&tonable_json()),
        escape(&format!(
            "{}/images/emoji/{}/%{{name}}.png?v={}",
            tb.base_path,
            tb.emoji_set,
            crate::emoji::image_version()
        )),
        escape(&t("chat.reactions.add")),
        escape(&t("chat.reactions.remove")),
        escape(&t("chat.bookmark_message")),
        escape(&t("chat.bookmark_message_edit")),
        escape(&t("bookmarks.created_generic")),
        escape(&t("bookmarks.created_with_reminder_generic")),
        escape(&t("bookmarks.reminders.today_with_time")),
        escape(&t("bookmarks.reminders.tomorrow_with_time")),
        escape(&t("bookmarks.reminders.at_time")),
        escape(&t("chat.link_copied")),
        escape(&t("chat.message_too_long")),
    );
    out.push_str(&format!(
        "<div class=\"chat-message-actions-container is-size-full\"><div aria-label=\"{}\" class=\"chat-message-actions\" role=\"toolbar\">",
        escape(&t("chat.message_actions"))
    ));
    out.push_str(&format!(
        "<button class=\"btn no-text btn-icon btn-flat react-btn\" title=\"{}\" type=\"button\" tabindex=\"-1\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        escape(&t("chat.react")),
        d_icon("discourse-emojis", None)
    ));
    // BookmarkIcon's three states; chat.js keeps the message's.
    out.push_str(&format!(
        "<button class=\"btn no-text btn-flat bookmark-btn\" type=\"button\" tabindex=\"-1\"><span class=\"svg-icon-title\" data-state=\"none\" title=\"{}\">{}</span><span class=\"svg-icon-title\" data-state=\"bookmarked\">{}</span><span class=\"svg-icon-title\" data-state=\"reminder\">{}</span></button>",
        escape(&t("bookmarks.create")),
        d_icon("far-bookmark", Some("bookmark-icon")),
        d_icon("bookmark", Some("bookmark-icon bookmark-icon__bookmarked")),
        d_icon(
            "discourse-bookmark-clock",
            Some("bookmark-icon bookmark-icon__bookmarked")
        ),
    ));
    out.push_str(&format!(
        "<button class=\"btn no-text btn-icon btn-flat reply-btn\" title=\"{}\" type=\"button\" tabindex=\"-1\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
        escape(&t("chat.reply")),
        d_icon("reply", None)
    ));
    out.push_str(&format!(
        "<details class=\"select-kit single-select dropdown-select-box more-buttons secondary-actions more-actions-chat\"><summary aria-label=\"{}\" name=\"Select a value to filter\" data-name=\"\" tabindex=\"-1\" class=\"select-kit-header single-select-header dropdown-select-box-header btn btn-icon-text btn-flat\"><div class=\"select-kit-header-wrapper\">{}<div class=\"select-kit-selected-name selected-name choice\" title=\"\"><span class=\"name\"></span></div>&#8203;</div></summary><div class=\"select-kit-body\"><div class=\"select-kit-filter\"></div><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">",
        escape(&t("chat.more_message_actions")),
        d_icon("ellipsis-vertical", None)
    ));
    for (value, key, icon) in [
        ("copyLink", "chat.copy_link", "link"),
        ("edit", "chat.edit", "pencil"),
        ("select", "chat.select", "list-check"),
        ("pin", "chat.pin_message", "thumbtack"),
        ("unpin", "chat.unpin_message", "thumbtack"),
        ("flag", "chat.flag", "flag"),
        ("delete", "chat.delete", "trash-can"),
        ("restore", "chat.restore", "arrow-rotate-left"),
        ("rebake", "chat.rebake_message", "rotate"),
    ] {
        out.push_str(&row(value, &t(key), icon));
    }
    out.push_str("</ul></div></details></div></div>");
    out.push_str(&details_bar(t, "", 0, "", "", ""));
    out.push_str("</template>");
    out.push_str(&toast_template());
    out
}

/// The success toast (DToasts with DDefaultToast) chat.js shows for a
/// copied link.
fn toast_template() -> String {
    format!(
        "<template class=\"chat-toast-template\"><output class=\"fk-d-toast\" data-test-duration=\"3000\" role=\"status\"><div class=\"fk-d-default-toast -success\"><div class=\"fk-d-default-toast__icon-container\">{}</div><div class=\"fk-d-default-toast__main-container\"><div class=\"fk-d-default-toast__texts\"><div class=\"fk-d-default-toast__message\"></div></div></div><div class=\"fk-d-default-toast__close-container\"><button class=\"btn no-text btn-icon btn-transparent\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div></div></output></template>",
        d_icon("check", None),
        d_icon("xmark", None)
    )
}

/// ChatComposerMessageDetails: the message being edited (`edit`) or
/// replied to (`reply`), its author's avatar (ChatUserAvatar, as HTML),
/// name and excerpt. Without an action it is chat.js's prototype, holding
/// both icons. `draft` is what chat.js saves of it in the draft (the
/// excerpt and the author, as data attributes).
pub fn details_bar(
    t: &dyn Fn(&str) -> String,
    action: &str,
    id: i64,
    draft: &str,
    avatar: &str,
    username_and_excerpt: &str,
) -> String {
    let icon = match action {
        "edit" => d_icon("pencil", None),
        "reply" => d_icon("reply", None),
        _ => format!(
            "<template data-action=\"edit\">{}</template><template data-action=\"reply\">{}</template>",
            d_icon("pencil", None),
            d_icon("reply", None)
        ),
    };
    format!(
        "<div class=\"chat-composer-message-details\" data-action=\"{action}\" data-id=\"{id}\"{draft}><div class=\"chat-reply\">{icon}{avatar}{username_and_excerpt}</div><button class=\"btn no-text btn-icon btn-flat cancel-message-action\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></div>",
        escape(&t("cancel")),
        d_icon("circle-xmark", None)
    )
}

/// `Emoji.tonable_emojis`, compact.
fn tonable_json() -> String {
    serde_json::from_str::<Vec<String>>(discourse_markdown::emoji::TONABLE_JSON)
        .map(|v| v.join("|"))
        .unwrap_or_default()
}
