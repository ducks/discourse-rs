//! The header's user menu (components/user-menu/menu.gjs and its lists),
//! rendered as the Ember client renders it: the tabs down its side with
//! their unread counts, and the current tab's panel, a list of items
//! (lib/user-menu/*-item.js and the notification type renderers,
//! lib/notification-types/*.js and the plugins' ones) with its show all
//! and dismiss buttons, an empty state, or the profile tab. In the mobile
//! view the panel slides in over a cloak (UserMenuWrapper).
//! static/js/user-menu.js opens it, switches its tabs and runs its
//! buttons.

use serde_json::Value;

use crate::emojis::Emojis;
use crate::i18n::I18n;
use crate::topic_list_view::{emoji_unescape, escape, icon};

/// What the menu reads besides its items.
pub struct MenuContext<'a> {
    pub i18n: &'a I18n,
    pub base_path: &'a str,
    /// The slide-in mode (GlimmerSiteHeader#slideInMode): the mobile view
    /// or the narrow desktop, where the menu slides in over a cloak.
    pub mobile: bool,
    /// The serialized current user (/session/current.json's), which the
    /// client's menu reads its counts and flags from.
    pub user: &'a Value,
    /// `site.notification_types`, in order.
    pub notification_types: &'a [(&'a str, i64)],
    /// `prioritize_full_name_in_ux`
    pub prioritize_full_name: bool,
    /// `topic_voting_enable_vote_limits` (votes_released's label and icon).
    pub vote_limits: bool,
    /// `enable_user_status`: the profile tab's status row.
    pub enable_user_status: bool,
    /// chat's `userCanChat`: its notifications tab.
    pub can_chat: bool,
    pub emojis: &'a Emojis,
    pub now: chrono::DateTime<chrono::Utc>,
    /// `relative_date_duration`, for the ages it draws.
    pub relative_date_duration: i64,
}

impl MenuContext<'_> {
    fn t(&self, key: &str) -> String {
        self.i18n
            .t(&format!("js.{key}"))
            .map(str::to_string)
            .unwrap_or_else(|| format!("[en.{key}]"))
    }

    fn t_with(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n
            .t_with(&format!("js.{key}"), args)
            .unwrap_or_else(|| format!("[en.{key}]"))
    }

    fn t_count(&self, key: &str, count: i64, args: &[(&str, &str)]) -> String {
        let count_s = count.to_string();
        let mut all = vec![("count", count_s.as_str())];
        all.extend_from_slice(args);
        self.i18n
            .t_count(&format!("js.{key}"), count, &all)
            .unwrap_or_else(|| format!("[en.{key}]"))
    }

    fn username(&self) -> &str {
        self.user["username"].as_str().unwrap_or_default()
    }

    /// `currentUser.path`
    fn user_path(&self) -> String {
        format!("{}/u/{}", self.base_path, self.username().to_lowercase())
    }

    fn type_id(&self, name: &str) -> Option<i64> {
        self.notification_types
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, id)| *id)
    }

    fn type_name(&self, id: i64) -> Option<&str> {
        self.notification_types
            .iter()
            .find(|(_, i)| *i == id)
            .map(|(n, _)| *n)
    }

    /// `UserMenuTab#getUnreadCountForType`
    fn unread(&self, name: &str) -> i64 {
        let Some(id) = self.type_id(name) else {
            return 0;
        };
        let grouped = &self.user["grouped_unread_notifications"];
        grouped[id.to_string()]
            .as_i64()
            .or_else(|| grouped[id as usize].as_i64())
            .unwrap_or(0)
    }

    fn user_option(&self, key: &str) -> &Value {
        &self.user["user_option"][key]
    }
}

/// A tab of the menu: its id, icon, the notification types its list
/// shows and its unread count.
pub struct Tab {
    pub id: &'static str,
    icon: &'static str,
    pub types: Vec<String>,
    count: i64,
    link: Option<String>,
}

/// The menu's tabs, top then bottom (UserMenu#topTabs, #bottomTabs): the
/// core ones the user may see, chat's before the review queue, and the
/// other notifications with every type no other tab lists.
pub fn tabs(cx: &MenuContext) -> (Vec<Tab>, Vec<Tab>) {
    let path = cx.user_path();
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let sum = |list: &[&str]| list.iter().map(|t| cx.unread(t)).sum::<i64>();
    let mut top = vec![
        Tab {
            id: "all-notifications",
            icon: "bell",
            types: Vec::new(),
            count: 0,
            link: Some(format!("{path}/notifications")),
        },
        Tab {
            id: "replies",
            icon: "reply",
            types: names(&[
                "mentioned",
                "group_mentioned",
                "posted",
                "quoted",
                "replied",
            ]),
            count: sum(&[
                "mentioned",
                "group_mentioned",
                "posted",
                "quoted",
                "replied",
            ]),
            link: Some(format!("{path}/notifications/responses")),
        },
    ];
    if cx.user_option("likes_notifications_disabled") != true {
        top.push(Tab {
            id: "likes",
            icon: "heart",
            types: names(&["liked", "liked_consolidated", "reaction"]),
            count: sum(&["liked", "liked_consolidated", "reaction"]),
            link: Some(format!("{path}/notifications/likes-received")),
        });
    }
    if cx.user["can_send_private_messages"] == true {
        top.push(Tab {
            id: "messages",
            icon: "envelope",
            types: names(&["private_message", "group_message_summary"]),
            count: cx.unread("private_message"),
            link: Some(format!("{path}/messages")),
        });
    }
    top.push(Tab {
        id: "bookmarks",
        icon: "bookmark",
        types: names(&["bookmark_reminder"]),
        count: cx.unread("bookmark_reminder"),
        link: Some(format!("{path}/activity/bookmarks")),
    });
    let reviewables = cx.user["reviewable_count"].as_i64().unwrap_or(0);
    let review = (cx.user["can_review"] == true && reviewables > 0).then(|| Tab {
        id: "review-queue",
        icon: "flag",
        types: Vec::new(),
        count: reviewables,
        link: Some(format!("{}/review", cx.base_path)),
    });
    if cx.can_chat {
        top.push(Tab {
            id: "chat-notifications",
            icon: "d-chat",
            types: names(&[
                "chat_invitation",
                "chat_mention",
                "chat_message",
                "chat_quoted",
                "chat_watched_thread",
            ]),
            count: sum(&["chat_mention", "chat_invitation", "chat_watched_thread"]),
            link: None,
        });
    }
    if let Some(review) = review {
        top.push(review);
    }
    let used: Vec<String> = top.iter().flat_map(|t| t.types.clone()).collect();
    let other: Vec<String> = cx
        .notification_types
        .iter()
        .map(|(name, _)| name.to_string())
        .filter(|name| !used.contains(name))
        .collect();
    let other_count = other.iter().map(|t| cx.unread(t)).sum();
    top.push(Tab {
        id: "other-notifications",
        icon: "discourse-other-tab",
        types: other,
        count: other_count,
        link: None,
    });
    let bottom = vec![Tab {
        id: "profile",
        icon: "user",
        types: Vec::new(),
        count: 0,
        link: Some(format!("{path}/summary")),
    }];
    (top, bottom)
}

/// `UserMenuTab#title`
fn tab_title(cx: &MenuContext, tab: &Tab) -> String {
    let id = tab.id.replace('-', "_");
    if tab.count > 0 {
        cx.t_count(&format!("user_menu.tabs.{id}_with_unread"), tab.count, &[])
    } else {
        cx.t(&format!("user_menu.tabs.{id}"))
    }
}

/// The whole menu with the panel's content (`.user-menu-dropdown-wrapper`).
pub fn menu(cx: &MenuContext, current: &str, panel: &str) -> String {
    let (top, bottom) = tabs(cx);
    let tab_html = |tab: &Tab, position: usize| {
        let active = tab.id == current;
        let title = escape(&tab_title(cx, tab));
        format!(
            "<a aria-controls=\"quick-access-{id}\" aria-label=\"{title}\" aria-selected=\"{active}\" class=\"btn btn-flat btn-icon no-text user-menu-tab{}\" data-tab-number=\"{position}\"{} id=\"user-menu-button-{id}\" role=\"tab\" tabindex=\"{}\" title=\"{title}\">{}{}</a>",
            if active { " active" } else { "" },
            tab.link
                .as_ref()
                .map(|l| format!(" href=\"{}\"", escape(l)))
                .unwrap_or_default(),
            if active { "0" } else { "-1" },
            icon(tab.icon, None),
            if tab.count > 0 {
                format!(
                    "<span aria-hidden=\"true\" class=\"badge-notification\">{}</span>",
                    tab.count
                )
            } else {
                String::new()
            },
            id = tab.id,
        )
    };
    let mut out = format!(
        "<div class=\"user-menu-dropdown-wrapper\"><div class=\"user-menu revamped menu-panel {}\" data-max-width=\"320\" data-tab-id=\"{current}\"><div class=\"panel-body\"><div class=\"panel-body-contents\"><div aria-label=\"{}\" aria-orientation=\"vertical\" class=\"menu-tabs-container\" role=\"tablist\"><div class=\"top-tabs tabs-list\">",
        if cx.mobile { "slide-in" } else { "drop-down" },
        escape(&cx.t("user_menu.sr_menu_tabs"))
    );
    for (i, tab) in top.iter().enumerate() {
        out.push_str(&tab_html(tab, i));
    }
    out.push_str("</div><div class=\"bottom-tabs tabs-list\">");
    for (i, tab) in bottom.iter().enumerate() {
        out.push_str(&tab_html(tab, top.len() + i));
    }
    out.push_str(&format!(
        "</div></div><div class=\"quick-access-panel\" id=\"quick-access-{current}\">{panel}</div></div></div></div>"
    ));
    if cx.mobile {
        out.push_str("<div class=\"header-cloak\" style=\"display: block;\"></div>");
    }
    out.push_str("</div>");
    out
}

/// What a notifications list shows at its bottom and when empty, by tab.
pub struct ListOptions {
    pub show_all_href: String,
    pub show_all_title: String,
    pub show_dismiss: bool,
    pub dismiss_title: String,
    /// The types the dismiss button marks read, none for all.
    pub dismiss_types: Vec<String>,
    /// Dismissing asks first while high priority notifications are unread.
    pub confirm_dismiss: Option<String>,
    pub empty_state: String,
}

/// The list options of a tab's panel (UserMenuNotificationsList and its
/// subclasses).
pub fn list_options(cx: &MenuContext, tab: &Tab) -> ListOptions {
    let path = cx.user_path();
    let any_unread = cx.user["grouped_unread_notifications"]
        .as_object()
        .is_some_and(|g| g.values().any(|v| v.as_i64().unwrap_or(0) > 0));
    let high_priority = cx.user["unread_high_priority_notifications"]
        .as_i64()
        .unwrap_or(0);
    let mut options = ListOptions {
        show_all_href: format!("{path}/notifications"),
        show_all_title: cx.t("user_menu.view_all_notifications"),
        show_dismiss: any_unread,
        dismiss_title: cx.t("user.dismiss_notifications_tooltip"),
        dismiss_types: tab.types.clone(),
        confirm_dismiss: None,
        empty_state: empty_state(cx, "user_menu.generic_no_items", None),
    };
    let notifications_empty = || {
        empty_state(
            cx,
            "user.no_notifications_title",
            Some(cx.t_with(
                "user.no_notifications_body",
                &[
                    ("icon", &icon("bell", None)),
                    (
                        "preferencesUrl",
                        &format!("{}/my/preferences/notifications", cx.base_path),
                    ),
                ],
            )),
        )
    };
    match tab.id {
        "all-notifications" => {
            options.dismiss_types = Vec::new();
            options.empty_state = notifications_empty();
            if high_priority > 0 {
                options.confirm_dismiss = Some(cx.t_count(
                    "notifications.dismiss_confirmation.body.default",
                    high_priority,
                    &[],
                ));
            }
        }
        "replies" => options.empty_state = notifications_empty(),
        "likes" => {
            options.empty_state = empty_state(
                cx,
                "user.no_likes_title",
                Some(cx.t_with(
                    "user.no_likes_body",
                    &[(
                        "preferencesUrl",
                        &format!("{}/my/preferences/notifications", cx.base_path),
                    )],
                )),
            )
        }
        "messages" => {
            let unread = cx.unread("private_message");
            options.show_all_href = format!("{path}/messages");
            options.show_all_title = cx.t("user_menu.view_all_messages");
            options.show_dismiss = unread > 0;
            options.dismiss_title = cx.t("user.dismiss_messages_tooltip");
            if high_priority > 0 {
                options.confirm_dismiss = Some(cx.t_count(
                    "notifications.dismiss_confirmation.body.messages",
                    unread,
                    &[],
                ));
            }
            options.empty_state = empty_state(
                cx,
                "user.no_messages_title",
                Some(cx.t_with(
                    "user.no_messages_body",
                    &[
                        ("icon", &icon("envelope", None)),
                        ("aboutUrl", &format!("{}/about", cx.base_path)),
                    ],
                )),
            );
        }
        "bookmarks" => {
            let unread = cx.unread("bookmark_reminder");
            options.show_all_href = format!("{path}/activity/bookmarks");
            options.show_all_title = cx.t("user_menu.view_all_bookmarks");
            options.show_dismiss = unread > 0;
            options.dismiss_title = cx.t("user.dismiss_bookmarks_tooltip");
            if high_priority > 0 {
                options.confirm_dismiss = Some(cx.t_count(
                    "notifications.dismiss_confirmation.body.bookmarks",
                    unread,
                    &[],
                ));
            }
            options.empty_state = empty_state(
                cx,
                "user.no_bookmarks_title",
                Some(cx.t_with(
                    "user.no_bookmarks_body",
                    &[("icon", &icon("bookmark", None))],
                )),
            );
        }
        "chat-notifications" => {
            options.empty_state = empty_state(
                cx,
                "user_menu.no_chat_notifications_title",
                Some(cx.t_with(
                    "user_menu.no_chat_notifications_body",
                    &[(
                        "preferencesUrl",
                        &format!("{}/my/preferences/notifications", cx.base_path),
                    )],
                )),
            )
        }
        // UserMenuReviewablesList: the queue's page, no dismiss.
        "review-queue" => {
            options.show_all_href = format!("{}/review", cx.base_path);
            options.show_all_title = cx.t("user_menu.reviewable.view_all");
            options.show_dismiss = false;
        }
        "other-notifications" => {
            options.empty_state = empty_state(
                cx,
                "user.no_other_notifications_title",
                Some(cx.t("user.no_other_notifications_body")),
            )
        }
        _ => {}
    }
    options
}

/// DEmptyState, text only; the body is HTML (trustHTML).
fn empty_state(cx: &MenuContext, title_key: &str, body: Option<String>) -> String {
    let mut out = format!(
        "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title=\"\">{}</div>",
        escape(&cx.t(title_key))
    );
    if let Some(body) = body {
        out.push_str(&format!(
            "<div class=\"empty-state__body\"><p data-test-body=\"\">{body}</p></div>"
        ));
    }
    out.push_str("</div></div>");
    out
}

/// UserMenuItemsList: the items, then the show all and dismiss buttons;
/// the empty state without items.
pub fn items_list(tab: &Tab, items: &[String], options: &ListOptions) -> String {
    if items.is_empty() {
        return options.empty_state.clone();
    }
    let mut out = format!(
        "<ul aria-labelledby=\"user-menu-button-{id}\" class=\"user-menu-button-{id}\">",
        id = tab.id
    );
    for item in items {
        out.push_str(item);
    }
    out.push_str("</ul><div class=\"panel-body-bottom\">");
    let title = escape(&options.show_all_title);
    out.push_str(&format!(
        "<a aria-label=\"{title}\" class=\"btn no-text btn-default show-all\" href=\"{}\" title=\"{title}\"><svg class=\"fa d-icon d-icon-chevron-down svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"false\" aria-label=\"{title}\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#chevron-down\"></use></svg></a>",
        escape(&options.show_all_href)
    ));
    if options.show_dismiss {
        out.push_str(&format!(
            "<button class=\"btn btn-default notifications-dismiss btn-icon-text\" title=\"{}\" type=\"button\" data-dismiss-types=\"{}\"{}>{} Dismiss </button>",
            escape(&options.dismiss_title),
            escape(&options.dismiss_types.join(",")),
            options
                .confirm_dismiss
                .as_ref()
                .map(|c| format!(" data-confirm=\"{}\"", escape(c)))
                .unwrap_or_default(),
            icon("check", None)
        ));
    }
    out.push_str("</div>");
    out
}

/// The text or markup an item shows.
enum Text {
    Plain(String),
    Html(String),
}

impl Text {
    /// MenuItem's `dReplaceEmoji` on the description; a label is text.
    fn render(&self, cx: &MenuContext, emoji: bool) -> String {
        match self {
            Text::Html(h) => {
                if emoji {
                    emoji_unescape(h, cx.emojis, cx.base_path)
                } else {
                    h.clone()
                }
            }
            Text::Plain(s) => {
                if emoji {
                    emoji_unescape(&escape(s), cx.emojis, cx.base_path)
                } else {
                    escape(s)
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Text::Plain(s) | Text::Html(s) => s.is_empty(),
        }
    }
}

/// One item as UserMenuItem renders it.
struct Item {
    class: String,
    href: Option<String>,
    title: Option<String>,
    icon: String,
    label: Option<String>,
    label_class: String,
    description: Option<Text>,
    description_class: String,
    topic_id: Option<i64>,
    /// The notification's id, which user-menu.js sets the cn cookie to
    /// when an unread one is clicked.
    notification_id: Option<i64>,
}

fn render_item(cx: &MenuContext, item: &Item) -> String {
    render_item_with_end(cx, item, "")
}

/// An item with its endComponent (the notifications page's date) after
/// the texts.
fn render_item_with_end(cx: &MenuContext, item: &Item, end: &str) -> String {
    let mut out = format!(
        "<li class=\"{}\"{}><a",
        escape(&item.class),
        item.notification_id
            .map(|id| format!(" data-notification-id=\"{id}\""))
            .unwrap_or_default()
    );
    if let Some(href) = &item.href {
        out.push_str(&format!(" href=\"{}\"", escape(href)));
    }
    if let Some(title) = &item.title {
        out.push_str(&format!(" title=\"{}\"", escape(title)));
    }
    out.push('>');
    out.push_str(&icon(&resolve_icon(&item.icon), None));
    out.push_str("<div>");
    if let Some(label) = item.label.as_ref().filter(|l| !l.is_empty()) {
        out.push_str(&format!(
            "<span class=\"item-label {}\">{}</span>",
            item.label_class,
            escape(label)
        ));
    }
    if let Some(description) = item.description.as_ref().filter(|d| !d.is_empty()) {
        out.push_str(&format!(
            "<span class=\"item-description {}\"{}>{}</span>",
            item.description_class,
            item.topic_id
                .map(|id| format!(" data-topic-id=\"{id}\""))
                .unwrap_or_default(),
            description.render(cx, true)
        ));
    }
    out.push_str("</div>");
    out.push_str(end);
    out.push_str("</a></li>");
    out
}

/// The icon library's REPLACEMENTS for the names the items use.
fn resolve_icon(name: &str) -> String {
    let replaced = match name {
        "notification.mentioned" => "at",
        "notification.group_mentioned" => "users",
        "notification.quoted" => "quote-right",
        "notification.replied" => "reply",
        "notification.posted" => "discourse-bell-exclamation",
        "notification.watching_category_or_tag" => "discourse-bell-exclamation",
        "notification.edited" => "pencil",
        "notification.bookmark_reminder" => "discourse-bookmark-clock",
        "notification.liked"
        | "notification.liked_2"
        | "notification.liked_many"
        | "notification.liked_consolidated" => "heart",
        "notification.private_message" | "notification.invited_to_private_message" => "envelope",
        "notification.invited_to_topic" => "hand-point-right",
        "notification.invitee_accepted" => "user",
        "notification.moved_post" => "right-from-bracket",
        "notification.linked" | "notification.linked_consolidated" => "link",
        "notification.granted_badge" => "certificate",
        "notification.topic_reminder" => "far-clock",
        "notification.watching_first_post" => "discourse-bell-one",
        "notification.group_message_summary" => "users",
        "notification.post_approved" => "check",
        "notification.membership_request_accepted" => "user-plus",
        "notification.membership_request_consolidated" => "users",
        "notification.reaction" => "bell",
        "notification.votes_released" => "plus",
        "notification.chat_quoted" => "quote-right",
        "user_menu.replies" => "reply",
        "user_menu.drafts" => "pencil",
        // discourse-topic-voting's and chat's replacements.
        "topic_voting.voting_closed" => "far-envelope",
        "d-chat" => "comment",
        other => other,
    };
    replaced.to_string()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `postUrl(slug, topicId, postNumber)`
fn post_url(base_path: &str, slug: &str, topic_id: i64, post_number: Option<i64>) -> String {
    let mut url = format!("{base_path}/t/");
    if slug.is_empty() {
        url.push_str("topic/");
    } else {
        url.push_str(slug);
        url.push('/');
    }
    url.push_str(&topic_id.to_string());
    if let Some(n) = post_number.filter(|n| *n > 1) {
        url.push_str(&format!("/{n}"));
    }
    url
}

/// UserMenuNotificationItem with the notification type's renderer
/// (NotificationTypeBase and its subclasses).
pub fn notification_item(cx: &MenuContext, n: &Value) -> String {
    render_item(cx, &notification(cx, n))
}

/// A notification as the notifications page lists it: its date at the end
/// (DRelativeDate).
pub fn notification_item_with_end(cx: &MenuContext, n: &Value, end: &str) -> String {
    render_item_with_end(cx, &notification(cx, n), end)
}

fn notification(cx: &MenuContext, n: &Value) -> Item {
    let data = &n["data"];
    let name = n["notification_type"]
        .as_i64()
        .and_then(|id| cx.type_name(id))
        .unwrap_or_default()
        .to_string();
    let topic_id = n["topic_id"].as_i64();
    // NotificationTypeBase
    let mut classes = vec![
        "notification".to_string(),
        if n["read"] == true { "read" } else { "unread" }.to_string(),
    ];
    if !name.is_empty() {
        classes.push(name.replace('_', "-"));
    }
    if n["is_warning"] == true {
        classes.push("is-warning".into());
    }
    let username = s(&data["display_username"]).to_string();
    let base_label = if cx.prioritize_full_name {
        n["acting_user_name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| username.clone())
    } else {
        username.clone()
    };
    let base_description = || -> Option<Text> {
        match n["fancy_title"].as_str().filter(|f| !f.is_empty()) {
            Some(fancy) => Some(Text::Html(emoji_unescape(fancy, cx.emojis, cx.base_path))),
            None => data["topic_title"].as_str().map(|t| Text::Plain(t.into())),
        }
    };
    let base_href = || -> Option<String> {
        if let Some(id) = topic_id {
            return Some(post_url(
                cx.base_path,
                s(&n["slug"]),
                id,
                n["post_number"].as_i64(),
            ));
        }
        if !data["group_id"].is_null() {
            return Some(format!(
                "{}/u/{}/messages/{}",
                cx.base_path,
                s(&data["username"]),
                s(&data["group_name"])
            ));
        }
        None
    };
    let base_title = if name.is_empty() {
        String::new()
    } else {
        cx.t(&format!("notifications.titles.{name}"))
    };
    let mut item = Item {
        class: String::new(),
        href: base_href(),
        title: Some(base_title),
        icon: format!("notification.{name}"),
        label: Some(base_label.clone()),
        label_class: String::new(),
        description: base_description(),
        description_class: String::new(),
        topic_id,
        notification_id: n["id"].as_i64(),
    };
    let user_path = |p: &str| format!("{}/u/{p}", cx.base_path);
    let count = data["count"].as_i64().unwrap_or(0);
    match name.as_str() {
        "admin_problems" => {
            item.label = None;
            item.description = Some(Text::Plain(cx.t("notifications.admin_problems")));
            item.href = Some(format!("{}/admin", cx.base_path));
            item.icon = "triangle-exclamation".into();
        }
        "bookmark_reminder" => {
            if let Some(name) = data["bookmark_name"].as_str().filter(|n| !n.is_empty()) {
                item.title = Some(cx.t_with(
                    "notifications.titles.bookmark_reminder_with_name",
                    &[("name", name)],
                ));
            }
            if item.description.as_ref().is_none_or(Text::is_empty) {
                item.description = data["title"].as_str().map(|t| Text::Plain(t.into()));
            }
            item.label = None;
            if item.href.is_none() {
                // getUrl: chat's message URLs carry the base path already.
                item.href = data["bookmarkable_url"].as_str().map(|u| {
                    let prefixed = !cx.base_path.is_empty()
                        && (u == cx.base_path || u.starts_with(&format!("{}/", cx.base_path)));
                    if prefixed || !u.starts_with('/') || u.starts_with("//") {
                        u.to_string()
                    } else {
                        format!("{}{u}", cx.base_path)
                    }
                });
            }
        }
        "custom" => {
            item.title = data["title"].as_str().map(|t| cx.t(t));
            item.icon = format!("notification.{}", s(&data["message"]));
        }
        "granted_badge" => {
            if let Some(badge_id) = data["badge_id"].as_i64() {
                let slug = match data["badge_slug"].as_str().filter(|s| !s.is_empty()) {
                    Some(slug) => slug.to_string(),
                    None => {
                        let re = regex::Regex::new(r"[^A-Za-z0-9_]+").unwrap();
                        re.replace_all(s(&data["badge_name"]), "-").to_lowercase()
                    }
                };
                let username = data["username"]
                    .as_str()
                    .map(|u| format!("?username={}", u.to_lowercase()))
                    .unwrap_or_default();
                item.href = Some(format!(
                    "{}/badges/{badge_id}/{slug}{username}",
                    cx.base_path
                ));
            } else {
                // `super.url`, which the base doesn't have.
                item.href = None;
            }
            item.description = Some(Text::Plain(cx.t_with(
                "notifications.granted_badge",
                &[("description", s(&data["badge_name"]))],
            )));
            item.label = None;
        }
        "group_mentioned" => {
            item.label = Some(format!("{base_label} @{}", s(&data["group_name"])));
            item.label_class = "mention-group notify".into();
        }
        "group_message_summary" => {
            item.description = Some(Text::Plain(cx.t_count(
                "notifications.group_message_summary",
                data["inbox_count"].as_i64().unwrap_or(0),
                &[("group_name", s(&data["group_name"]))],
            )));
            item.label = None;
            item.href = Some(user_path(&format!(
                "{}/messages/group/{}",
                s(&data["username"]),
                s(&data["group_name"])
            )));
        }
        "invitee_accepted" => {
            item.href = Some(user_path(&username));
            item.description = Some(Text::Plain(
                cx.t("notifications.invitee_accepted_your_invitation"),
            ));
        }
        "liked_consolidated" => {
            item.href = Some(user_path(&format!(
                "{}/notifications/likes-received?acting_username={}",
                cx.username(),
                s(&data["username"])
            )));
            item.description = Some(Text::Plain(cx.t_count(
                "notifications.liked_consolidated_description",
                count,
                &[],
            )));
        }
        "liked" => {
            let name_or_username = if cx.prioritize_full_name {
                data["display_name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(s(&data["display_username"]))
            } else {
                s(&data["display_username"])
            };
            if count == 2 {
                let second = if cx.prioritize_full_name {
                    data["name2"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(s(&data["username2"]))
                } else {
                    s(&data["username2"])
                };
                item.label = Some(cx.t_with(
                    "notifications.liked_by_2_users",
                    &[("username", name_or_username), ("username2", second)],
                ));
                item.label_class = "double-user".into();
            } else if count > 2 {
                item.label = Some(cx.t_count(
                    "notifications.liked_by_multiple_users",
                    count - 1,
                    &[("username", name_or_username)],
                ));
                item.label_class = "multi-user".into();
            }
        }
        "linked_consolidated" => {
            item.href = Some(user_path(&format!(
                "{}/notifications/links?acting_username={}",
                cx.username(),
                s(&data["username"])
            )));
            item.description = Some(Text::Plain(cx.t_count(
                "notifications.linked_consolidated_description",
                count,
                &[],
            )));
        }
        "membership_request_accepted" => {
            item.href = Some(format!("{}/g/{}", cx.base_path, s(&data["group_name"])));
            item.description = Some(Text::Plain(cx.t_with(
                "notifications.membership_request_accepted",
                &[("group_name", s(&data["group_name"]))],
            )));
            item.label = None;
        }
        "membership_request_consolidated" => {
            let owner = n["username"].as_str().unwrap_or(cx.username());
            item.href = Some(user_path(&format!("{owner}/messages")));
            item.description = Some(Text::Plain(cx.t_count(
                "notifications.membership_request_consolidated",
                count,
                &[("group_name", s(&data["group_name"]))],
            )));
            item.label = None;
        }
        "moved_post" => {
            item.label =
                Some(cx.t_with("notifications.user_moved_post", &[("username", &username)]));
        }
        "new_features" => {
            item.label = None;
            item.description = Some(Text::Plain(cx.t("notifications.new_features")));
            item.href = Some(format!("{}/admin/whats-new", cx.base_path));
            item.icon = "gift".into();
        }
        "replied" => {
            let bucket = data["reply_to_post_number"].as_i64();
            let consolidated = data["consolidated_count"].as_i64().unwrap_or(0);
            if let (Some(bucket), true) = (bucket, consolidated > 1) {
                if let Some(id) = topic_id {
                    let mut url = format!("{}/t/{}/{id}", cx.base_path, s(&n["slug"]));
                    if bucket != 1 {
                        url.push_str(&format!("/{bucket}"));
                    }
                    item.href = Some(format!("{url}?sort=new&collapse_replies=true"));
                }
                let key = if bucket == 1 {
                    "notifications.replied_consolidated_in_topic"
                } else {
                    "notifications.replied_consolidated_to_post"
                };
                item.label = Some(cx.t_count(key, consolidated, &[]));
            }
        }
        "upcoming_change_available" | "upcoming_change_automatically_promoted" => {
            let promoted = name == "upcoming_change_automatically_promoted";
            let key = if promoted {
                "automatically_promoted"
            } else {
                "available"
            };
            item.label = Some(cx.t(&format!("notifications.upcoming_changes.{key}.title")));
            item.title = Some(cx.t(&format!("notifications.titles.{name}")));
            let humanized: Vec<String> = match data["upcoming_change_humanized_names"].as_array() {
                Some(list) => list.iter().map(|v| s(v).to_string()).collect(),
                None => vec![s(&data["upcoming_change_humanized_name"]).to_string()],
            };
            let total = data["count"]
                .as_i64()
                .filter(|c| *c != 0)
                .unwrap_or(humanized.len() as i64);
            let first = humanized.first().cloned().unwrap_or_default();
            item.description = Some(Text::Plain(match total {
                1 => cx.t_with(
                    &format!("notifications.upcoming_changes.{key}.description"),
                    &[("changeName", &first)],
                ),
                2 => cx.t_with(
                    &format!("notifications.upcoming_changes.{key}.description_two"),
                    &[
                        ("changeName1", &first),
                        (
                            "changeName2",
                            humanized.get(1).map(String::as_str).unwrap_or(""),
                        ),
                    ],
                ),
                _ => cx.t_with(
                    &format!("notifications.upcoming_changes.{key}.description_many"),
                    &[
                        ("changeName", &first),
                        ("otherChangeCount", &(total - 1).to_string()),
                    ],
                ),
            }));
            let names: Vec<String> = match data["upcoming_change_names"].as_array() {
                Some(list) => list.iter().map(|v| s(v).to_string()).collect(),
                None => vec![s(&data["upcoming_change_name"]).to_string()],
            };
            // site.permanent_upcoming_change_names is empty on the port's
            // site, so a promoted change links to its configuration too.
            item.href = Some(format!(
                "{}/admin/config/upcoming-changes?changeNamesFilter={}",
                cx.base_path,
                names.join(",")
            ));
            item.icon = if promoted {
                "discourse-flask-check".into()
            } else {
                "flask".into()
            };
        }
        "watching_first_post" => {
            item.label = Some(cx.t("notifications.watching_first_post_label"));
        }
        // discourse-reactions
        "reaction" => {
            item.title = Some(cx.t("notifications.titles.reaction"));
            if item.href.is_none() {
                let activity = if data["reaction_icon"].is_null() {
                    "reactions-received"
                } else {
                    "likes-received"
                };
                item.href = Some(user_path(&format!(
                    "{}/notifications/{activity}?acting_username={}&include_likes=true",
                    cx.username(),
                    s(&data["display_username"])
                )));
            }
            if let Some(reaction) = data["reaction_icon"].as_str() {
                item.icon = reaction.to_string();
            }
            let name_or_username = if cx.prioritize_full_name {
                data["display_name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&username)
                    .to_string()
            } else {
                username.clone()
            };
            let username2 = s(&data["username2"]);
            item.label = Some(if count <= 1 || username2.is_empty() {
                name_or_username.clone()
            } else if count > 2 {
                cx.t_count(
                    "notifications.reaction_multiple_users",
                    count - 1,
                    &[("username", &name_or_username)],
                )
            } else {
                let second = if cx.prioritize_full_name {
                    data["name2"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .unwrap_or(username2)
                } else {
                    username2
                };
                cx.t_with(
                    "notifications.reaction_2_users",
                    &[("username", &name_or_username), ("username2", second)],
                )
            });
            if !username2.is_empty() {
                item.label_class = if count > 2 {
                    "multi-user".into()
                } else {
                    "double-user".into()
                };
            }
            if count > 1 && username2.is_empty() {
                item.description = Some(Text::Plain(cx.t_count(
                    "notifications.reaction_1_user_multiple_posts",
                    count,
                    &[],
                )));
            }
        }
        // discourse-post-voting: to the comment on the post.
        "question_answer_user_commented" => {
            if let Some(id) = topic_id {
                item.href = Some(format!(
                    "{}#post-voting-comment-{}",
                    post_url(cx.base_path, s(&n["slug"]), id, n["post_number"].as_i64()),
                    data["post_voting_comment_id"]
                ));
            }
            item.icon = "comment".into();
        }
        // discourse-topic-voting
        "votes_released" => {
            let label = if cx.vote_limits {
                cx.t("topic_voting.notification_label.vote_released")
            } else {
                cx.t("topic_voting.notification_label.voting_closed")
            };
            item.title = Some(label.clone());
            item.label = Some(label);
            if !cx.vote_limits {
                item.icon = "topic_voting.voting_closed".into();
            }
        }
        // chat
        "chat_invitation" => {
            item.title = Some(cx.t("notifications.titles.chat_invitation"));
            item.icon = "link".into();
            item.description = Some(Text::Plain(cx.t("notifications.chat_invitation")));
            let slug = crate::routes::user_menu::slugify_channel(
                data["chat_channel_slug"].as_str(),
                s(&data["chat_channel_title"]),
            );
            let mut url = format!("{}/chat/c/{slug}/{}", cx.base_path, data["chat_channel_id"]);
            if let Some(m) = data["chat_message_id"].as_i64() {
                url.push_str(&format!("/{m}"));
            }
            item.href = Some(url);
            item.label = Some(s(&data["invited_by_username"]).to_string());
        }
        "chat_mention" => {
            let slug = crate::routes::user_menu::slugify_channel(
                data["chat_channel_slug"].as_str(),
                s(&data["chat_channel_title"]),
            );
            let mut url = format!("{}/chat/c/{slug}/{}", cx.base_path, data["chat_channel_id"]);
            match data["chat_thread_id"].as_i64() {
                Some(thread) => url.push_str(&format!("/t/{thread}")),
                None => url.push_str(&format!("/{}", data["chat_message_id"])),
            }
            item.href = Some(url);
            item.title = Some(cx.t("notifications.titles.chat_mention"));
            item.icon = "d-chat".into();
            item.label = Some(s(&data["mentioned_by_username"]).to_string());
            let identifier = data["identifier"].as_str().map(|i| format!("@{i}"));
            let prefix = if data["is_direct_message_channel"] == true {
                "notifications.popup.direct_message_chat_mention"
            } else {
                "notifications.popup.chat_mention"
            };
            let suffix = if identifier.is_some() {
                "other_plain"
            } else {
                "direct"
            };
            item.description = Some(Text::Plain(cx.t_with(
                &format!("{prefix}.{suffix}"),
                &[
                    ("identifier", identifier.as_deref().unwrap_or_default()),
                    ("channel", s(&data["chat_channel_title"])),
                ],
            )));
        }
        _ => {}
    }
    item.class = classes.join(" ");
    item
}

/// UserMenuMessageItem: a message from the messages tab's list.
pub fn message_item(cx: &MenuContext, topic: &Value, users: &[Value]) -> String {
    let last_read = topic["last_read_post_number"].as_i64().unwrap_or(0);
    let highest = topic["highest_post_number"].as_i64().unwrap_or(0);
    let next = (last_read + 1).min(highest);
    let last_poster = s(&topic["last_poster_username"]);
    let label = if cx.prioritize_full_name {
        let poster_id = topic["posters"]
            .as_array()
            .and_then(|p| p.last())
            .and_then(|p| p["user_id"].as_i64());
        users
            .iter()
            .find(|u| u["id"].as_i64() == poster_id && poster_id.is_some())
            .and_then(|u| u["name"].as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or(last_poster)
            .to_string()
    } else {
        last_poster.to_string()
    };
    // The list's `unread`, which ListableTopicSerializer keeps at 0 for
    // old themes: a message reads as read.
    let unread = topic["unread"].as_i64().unwrap_or(0) != 0;
    render_item(
        cx,
        &Item {
            class: format!("message {}", if unread { "unread" } else { "read" }),
            href: Some(post_url(
                cx.base_path,
                s(&topic["slug"]),
                topic["id"].as_i64().unwrap_or(0),
                Some(next),
            )),
            title: Some(cx.t("user.private_message")),
            icon: "notification.private_message".into(),
            label: Some(label),
            label_class: String::new(),
            description: Some(Text::Html(emoji_unescape(
                s(&topic["fancy_title"]),
                cx.emojis,
                cx.base_path,
            ))),
            description_class: String::new(),
            topic_id: topic["id"].as_i64(),
            notification_id: None,
        },
    )
}

/// UserMenuReviewableItem with its type's render director
/// (lib/reviewable-types): a pending reviewable from
/// `/review/user-menu-list` or the notifications' pending_reviewables.
pub fn reviewable_item(cx: &MenuContext, r: &Value) -> String {
    let flagger = r["flagger_username"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| cx.t("user_menu.reviewable.deleted_user"));
    let id = r["id"].as_i64().unwrap_or(0);
    let unescape = |title: &str| emoji_unescape(title, cx.emojis, cx.base_path);
    let (icon_name, label, description) = match s(&r["type"]) {
        "ReviewableFlaggedPost" => {
            let description = match (r["topic_fancy_title"].as_str(), r["post_number"].as_i64()) {
                (Some(title), Some(n)) if !title.is_empty() => Text::Html(cx.t_with(
                    "user_menu.reviewable.post_number_with_topic_title",
                    &[("post_number", &n.to_string()), ("title", &unescape(title))],
                )),
                _ => Text::Plain(cx.t("user_menu.reviewable.deleted_post")),
            };
            ("flag", flagger, description)
        }
        "ReviewableQueuedPost" => {
            let title = r["topic_fancy_title"]
                .as_str()
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| escape(s(&r["payload_title"])));
            let title = unescape(&title);
            let description = if r["is_new_topic"] == true {
                Text::Html(title)
            } else if !title.is_empty() {
                Text::Html(cx.t_with(
                    "user_menu.reviewable.new_post_in_topic",
                    &[("title", &title)],
                ))
            } else {
                Text::Plain(cx.t("user_menu.reviewable.new_post_in_deleted_topic"))
            };
            (
                "layer-group",
                cx.t("user_menu.reviewable.queue"),
                description,
            )
        }
        "ReviewableUser" => (
            "user",
            flagger,
            Text::Plain(cx.t_with(
                "user_menu.reviewable.user_requires_approval",
                &[("username", s(&r["username"]))],
            )),
        ),
        _ => (
            "flag",
            flagger,
            Text::Plain(cx.t_with(
                "user_menu.reviewable.default_item",
                &[("reviewable_id", &id.to_string())],
            )),
        ),
    };
    render_item(
        cx,
        &Item {
            class: if r["pending"] == true {
                "reviewable pending".into()
            } else {
                "reviewable reviewed".into()
            },
            href: Some(format!("{}/review/{id}", cx.base_path)),
            // linkTitle: "" (a TODO in Ember).
            title: Some(String::new()),
            icon: icon_name.into(),
            label: Some(label),
            label_class: String::new(),
            description: Some(description),
            description_class: String::new(),
            topic_id: None,
            notification_id: None,
        },
    )
}

/// UserMenuBookmarkItem
pub fn bookmark_item(cx: &MenuContext, b: &Value) -> String {
    let user = &b["user"];
    let label = if cx.prioritize_full_name {
        user["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .or(user["username"].as_str())
    } else {
        user["username"].as_str()
    };
    render_item(
        cx,
        &Item {
            class: "bookmark".into(),
            href: b["bookmarkable_url"].as_str().map(str::to_string),
            title: b["name"].as_str().map(str::to_string),
            icon: "bookmark".into(),
            label: label.map(str::to_string),
            label_class: String::new(),
            description: Some(Text::Html(s(&b["fancy_title"]).to_string())),
            description_class: String::new(),
            topic_id: b["topic_id"].as_i64(),
            notification_id: None,
        },
    )
}

/// UserMenuProfileTabContent
pub fn profile_tab(cx: &MenuContext) -> String {
    let path = cx.user_path();
    let mut out = String::from(
        "<ul aria-labelledby=\"user-menu-button-profile\" class=\"user-menu-button-profile\">",
    );
    let button = |icon_name: &str, label: &str| {
        format!(
            "<button class=\"btn no-text btn-flat profile-tab-btn\" type=\"button\">{}<span class=\"item-label\">{label}</span></button>",
            icon(icon_name, None)
        )
    };
    if cx.enable_user_status {
        // The status button needs the user status modal, not ported.
        out.push_str(&format!(
            "<li class=\"set-user-status\">{}</li>",
            button(
                "circle-plus",
                &escape(&cx.t("user_status.set_custom_status"))
            )
        ));
    }
    let hidden = cx.user_option("hide_presence") == true;
    out.push_str(&format!(
        "<li class=\"presence-toggle{}\" title=\"{}\">{}</li>",
        if hidden { "" } else { " enabled" },
        escape(&cx.t("presence_toggle.title")),
        button(
            if hidden { "toggle-off" } else { "toggle-on" },
            &escape(&cx.t(if hidden {
                "presence_toggle.offline"
            } else {
                "presence_toggle.online"
            }))
        )
    ));
    // isInDoNotDisturb: a do_not_disturb_until still ahead.
    let dnd_until = cx.user["do_not_disturb_until"]
        .as_str()
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
        .filter(|d| *d > cx.now);
    let pause = escape(&cx.t("pause_notifications.label"));
    out.push_str(&format!(
        "<li class=\"do-not-disturb{}\">{}</li>",
        if dnd_until.is_some() { " enabled" } else { "" },
        match dnd_until {
            // The end as dAgeWithTooltip, unless it is DoNotDisturb.forever.
            Some(until) => button(
                "toggle-on",
                &format!(
                    "<span>{pause}</span>{}",
                    if until.format("%Y-%m-%d").to_string() == "3000-01-01" {
                        String::new()
                    } else {
                        // The template's whitespace between the two.
                        format!(" {}", crate::viewer_dates::tiny_span(cx, until))
                    }
                )
            ),
            None => button("toggle-off", &pause),
        }
    ));
    out.push_str("<hr>");
    let link = |class: &str, href: &str, icon_name: &str, label: &str| {
        format!(
            "<li class=\"{class}\"><a href=\"{href}\">{}<span class=\"item-label\">{}</span></a></li>",
            icon(icon_name, None),
            escape(label)
        )
    };
    out.push_str(&link(
        "summary",
        &format!("{path}/summary"),
        "user",
        &cx.t("user.summary.title"),
    ));
    out.push_str(&link(
        "activity",
        &format!("{path}/activity"),
        "bars-staggered",
        &cx.t("user.activity_stream"),
    ));
    if cx.user["can_invite_to_forum"] == true {
        out.push_str(&link(
            "invites",
            &format!("{path}/invited"),
            "user-plus",
            &cx.t("user.invited.title"),
        ));
    }
    let drafts = cx.user["draft_count"].as_i64().unwrap_or(0);
    out.push_str(&link(
        "drafts",
        &format!("{path}/activity/drafts"),
        "pencil",
        &if drafts > 0 {
            cx.t_count("drafts.label_with_count", drafts, &[])
        } else {
            cx.t("drafts.label")
        },
    ));
    out.push_str(&link(
        "preferences",
        &format!("{path}/preferences"),
        "gear",
        &cx.t("user.preferences.title"),
    ));
    if cx.user["can_post_anonymously"] == true || cx.user["is_anonymous"] == true {
        let anonymous = cx.user["is_anonymous"] == true;
        out.push_str(&format!(
            "<li class=\"{}\">{}</li>",
            if anonymous {
                "disable-anonymous"
            } else {
                "enable-anonymous"
            },
            if anonymous {
                button("ban", &escape(&cx.t("switch_from_anon")))
            } else {
                button("user-secret", &escape(&cx.t("switch_to_anon")))
            }
        ));
    }
    out.push_str(&format!(
        "<li class=\"logout\">{}</li></ul>",
        button("right-from-bracket", &escape(&cx.t("user.log_out")))
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_urls_leave_out_the_first_post() {
        assert_eq!(post_url("", "a-topic", 5, Some(1)), "/t/a-topic/5");
        assert_eq!(post_url("", "a-topic", 5, Some(3)), "/t/a-topic/5/3");
        assert_eq!(post_url("/f", "", 5, None), "/f/t/topic/5");
    }

    #[test]
    fn icons_resolve_through_the_replacements() {
        assert_eq!(resolve_icon("notification.mentioned"), "at");
        assert_eq!(
            resolve_icon("notification.bookmark_reminder"),
            "discourse-bookmark-clock"
        );
        assert_eq!(resolve_icon("bell"), "bell");
    }
}
