//! The user's messages as the Ember app renders them under the profile
//! (templates/user/messages.gjs, user-private-messages/user.gjs and
//! group.gjs, user-topics-list.gjs with its TopicDismissButtons and
//! BasicTopicList, the MessagesDropdown, GroupNotificationsTracking and
//! the routes' empty states), from the message list document and the
//! viewer's PrivateMessageTopicTrackingState; and the exception page
//! (templates/exception.gjs) a list's error leaves in the profile's place.

use serde_json::Value;

use crate::topic_list_view::{ListContext, escape, icon, t, t_count, t_with};

/// The routed list (userPrivateMessages.user.* and .group.*).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Inbox,
    Sent,
    New,
    Unread,
    Archive,
    Warnings,
    GroupInbox,
    GroupNew,
    GroupUnread,
    GroupArchive,
}

impl Route {
    pub fn is_group(self) -> bool {
        matches!(
            self,
            Route::GroupInbox | Route::GroupNew | Route::GroupUnread | Route::GroupArchive
        )
    }

    /// The route's `filter` (build-private-messages-route): the user
    /// list controller's and the title's.
    pub fn filter(self) -> Option<&'static str> {
        match self {
            Route::Inbox | Route::GroupInbox => Some("inbox"),
            Route::Sent => Some("sent"),
            Route::New | Route::GroupNew => Some("new"),
            Route::Unread | Route::GroupUnread => Some("unread"),
            Route::Archive | Route::GroupArchive => Some("archive"),
            Route::Warnings => None,
        }
    }
}

/// A row of the inboxes dropdown (messagesDropdownContent).
pub struct InboxRow {
    /// Its URL, the row's id.
    pub href: String,
    /// The name, `&nbsp;(count)` appended (trusted HTML) when counts show.
    pub name_html: String,
    pub icon: Option<&'static str>,
    pub show_unread_icon: bool,
}

/// What the page draws.
pub struct Messages<'a> {
    /// `/u/<username_lower>` with the base path.
    pub user_path: &'a str,
    pub route: Route,
    /// The group of a group route, and the profile user's level in it
    /// (`group.group_user.notification_level`).
    pub group: Option<(&'a str, Option<i64>)>,
    pub viewing_self: bool,
    /// `currentUser.isAdmin`
    pub viewer_admin: bool,
    /// `showNewPM`
    pub show_new_pm: bool,
    /// `currentUser.can_send_private_messages`, for the empty state.
    pub can_send_private_messages: bool,
    /// `currentUser.canManageTopic`
    pub can_manage_topic: bool,
    /// `currentUser.unified_new_enabled`
    pub unified_new: bool,
    /// The new and unread counts of the routed inbox.
    pub new_count: i64,
    pub unread_count: i64,
    pub dropdown: Vec<InboxRow>,
    /// The URL the dropdown matches its rows against.
    pub current_url: &'a str,
    /// The list document (TopicListSerializer).
    pub list: &'a Value,
}

/// The content under the profile's navigation.
pub fn render(cx: &ListContext, m: &Messages) -> String {
    let mut out = String::new();
    // user/messages.gjs: the secondary navigation.
    out.push_str(
        "<div class=\"user-navigation user-navigation-secondary\"><ol class=\"category-breadcrumb\"><li>",
    );
    out.push_str(&dropdown(cx, m));
    out.push_str(
        "</li></ol><nav aria-label=\"User secondary - messages\" class=\"horizontal-overflow-nav\">\
         <div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\">\
         <ul class=\"nav-pills action-list messages-nav\" id=\"user-navigation-secondary__horizontal-nav\" data-d-scroll-axis=\"horizontal\">",
    );
    out.push_str(&sub_nav(cx, m));
    out.push_str("</ul></div></nav><div class=\"navigation-controls\">");
    if let Some((_, level)) = m.group {
        out.push_str(&group_tracking(cx, level.unwrap_or(1)));
    }
    if m.show_new_pm {
        out.push_str(&format!(
            "<button class=\"btn btn-icon-text btn-primary new-private-message\" id=\"new-private-message-btn\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
            icon("envelope", None),
            escape(&t(cx, "user.new_private_message"))
        ));
    }
    out.push_str("</div></div><section class=\"user-content\" id=\"user-content\">");
    if m.route == Route::Warnings && !m.viewing_self && !m.viewer_admin {
        out.push_str(&format!(
            "<div class=\"alert alert-info\">{}</div>",
            t(cx, "admin.user.warnings_list_warning")
        ));
    }
    let list = topic_list(cx, m);
    match m.group {
        Some((name, _)) => out.push_str(&format!(
            "<div class=\"group-messages group-{}\">{list}</div>",
            escape(name)
        )),
        None => out.push_str(&list),
    }
    out.push_str("</section>");
    out
}

/// MessagesDropdown: the trigger of a DMenu over the inboxes, its menu
/// hidden until static/js/messages-dropdown.js opens it. The selection
/// is the last row whose URL the current one contains.
fn dropdown(cx: &ListContext, m: &Messages) -> String {
    let current = m.current_url.to_lowercase();
    let selected = m
        .dropdown
        .iter()
        .rposition(|row| current.contains(&row.href.to_lowercase()));
    let selection = selected.map(|i| &m.dropdown[i]);
    let icon_html = selection
        .and_then(|row| row.icon)
        .map(|name| icon(name, None))
        .unwrap_or_default();
    let unread = |show: bool| {
        if show {
            icon("circle", Some("d-icon-d-unread"))
        } else {
            String::new()
        }
    };
    let label = selection
        .map(|row| format!("<span class=\"d-button-label\">{}</span>", row.name_html))
        .unwrap_or_default();
    let mut out = format!(
        "<button aria-expanded=\"false\" class=\"btn{} fk-d-menu__trigger messages-dropdown-trigger btn-default\" title=\"{}\" data-identifier=\"messages-dropdown\" data-trigger=\"\" type=\"button\">{icon_html}{label}{}{}</button>",
        if icon_html.is_empty() {
            ""
        } else {
            " btn-icon-text"
        },
        escape(&t(cx, "user.messages.all")),
        unread(selection.is_some_and(|row| row.show_unread_icon)),
        icon("angle-down", None)
    );
    // The menu's DDropdownMenu, each row a button to its inbox.
    out.push_str(
        "<div class=\"fk-d-menu messages-dropdown-content -animated\" data-content=\"\" data-identifier=\"messages-dropdown\" role=\"dialog\" data-strategy=\"absolute\" data-placement=\"bottom-start\" hidden style=\"display: none\">\
         <div class=\"fk-d-menu__inner-content\"><ul class=\"dropdown-menu\">",
    );
    for row in &m.dropdown {
        let is_selected = selection.is_some_and(|s| s.name_html == row.name_html);
        out.push_str(&format!(
            "<li class=\"dropdown-menu__item\"><button class=\"btn {}{}\" data-href=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span>{}</button></li>",
            if row.icon.is_some() {
                "btn-icon-text"
            } else {
                "btn-text"
            },
            if is_selected { " is-selected" } else { "" },
            escape(&row.href),
            row.icon.map(|name| icon(name, None)).unwrap_or_default(),
            row.name_html,
            unread(row.show_unread_icon)
        ));
    }
    out.push_str("</ul></div></div>");
    out
}

/// The routed inbox's secondary navigation (user.gjs or group.gjs, through
/// MessagesSecondaryNav).
fn sub_nav(cx: &ListContext, m: &Messages) -> String {
    let link_text = |kind: &str, count: i64| {
        if count == 0 {
            t(cx, &format!("user.messages.{kind}"))
        } else {
            t_count(cx, &format!("user.messages.{kind}_with_count"), count, &[])
        }
    };
    let item = |class: &str, href: String, icon_name: &str, label: String, current: bool| {
        format!(
            "<li{} class=\"{class}\"><a class=\"{}\" href=\"{}\">{} <span>{}</span></a></li>",
            if current {
                " aria-current=\"location\""
            } else {
                ""
            },
            if current { "active" } else { "" },
            escape(&href),
            icon(icon_name, None),
            escape(&label)
        )
    };
    let base = m.user_path;
    let mut out = String::new();
    match m.group {
        None => {
            out.push_str(&item(
                "user-nav__messages-latest",
                format!("{base}/messages"),
                "envelope",
                t(cx, "categories.latest"),
                m.route == Route::Inbox,
            ));
            out.push_str(&item(
                "user-nav__messages-sent",
                format!("{base}/messages/sent"),
                "reply",
                t(cx, "user.messages.sent"),
                m.route == Route::Sent,
            ));
            if m.viewing_self {
                out.push_str(&item(
                    "user-nav__messages-new",
                    format!("{base}/messages/new"),
                    "circle-exclamation",
                    link_text("new", m.new_count),
                    m.route == Route::New,
                ));
                out.push_str(&item(
                    "user-nav__messages-unread",
                    format!("{base}/messages/unread"),
                    "circle-plus",
                    link_text("unread", m.unread_count),
                    m.route == Route::Unread,
                ));
            }
            out.push_str(&item(
                "user-nav__messages-archive",
                format!("{base}/messages/archive"),
                "box-archive",
                t(cx, "user.messages.archive"),
                m.route == Route::Archive,
            ));
        }
        Some((name, _)) => {
            let group = format!("{base}/messages/group/{}", uri_component(name));
            out.push_str(&item(
                "user-nav__messages-group-latest",
                group.clone(),
                "envelope",
                t(cx, "categories.latest"),
                m.route == Route::GroupInbox,
            ));
            if m.viewing_self {
                out.push_str(&item(
                    "user-nav__messages-group-new",
                    format!("{group}/new"),
                    "circle-exclamation",
                    link_text("new", m.new_count),
                    m.route == Route::GroupNew,
                ));
                out.push_str(&item(
                    "user-nav__messages-group-unread",
                    format!("{group}/unread"),
                    "circle-plus",
                    link_text("unread", m.unread_count),
                    m.route == Route::GroupUnread,
                ));
                out.push_str(&item(
                    "user-nav__messages-group-archive",
                    format!("{group}/archive"),
                    "box-archive",
                    t(cx, "user.messages.archive"),
                    m.route == Route::GroupArchive,
                ));
            }
        }
    }
    out
}

/// GroupNotificationsTracking: the trigger alone, its title the level's.
/// Changing the level (Group#setNotification) is not ported, so no menu.
fn group_tracking(cx: &ListContext, level: i64) -> String {
    let (key, icon_name) = match level {
        0 => ("muted", "d-muted"),
        2 => ("tracking", "d-tracking"),
        3 => ("watching", "d-watching"),
        4 => ("watching_first_post", "d-watching-first"),
        _ => ("regular", "d-regular"),
    };
    let title = t(cx, &format!("groups.notifications.{key}.title"));
    format!(
        "<button class=\"btn btn-default no-text fk-d-menu__trigger notifications-tracking-trigger btn-default btn-icon notifications-tracking-trigger-btn group-notifications-tracking\" title=\"{}\" aria-expanded=\"false\" data-identifier=\"notifications-tracking\" data-trigger=\"\" data-level-id=\"{level}\" data-level-name=\"{key}\">{}</button>",
        escape(&t_with(
            cx,
            "notifications_tracking.tooltip",
            &[("level", &title)]
        )),
        crate::post_view::d_icon(icon_name, None)
    )
}

/// user-topics-list.gjs: the empty state, or the dismiss buttons around
/// the BasicTopicList.
fn topic_list(cx: &ListContext, m: &Messages) -> String {
    let topics: &[Value] = m.list["topic_list"]["topics"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if topics.is_empty() {
        // The route's emptyState.
        let body = if !m.route.is_group() && m.can_send_private_messages {
            t_with(
                cx,
                "user.no_messages_body",
                &[
                    ("aboutUrl", &format!("{}/about", cx.base_path)),
                    ("icon", &icon("envelope", None)),
                ],
            )
        } else {
            String::new()
        };
        let mut out = format!(
            "<div class=\"empty-state__container --text-only\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title=\"\">{}</div>",
            escape(&t(cx, "user.no_messages_title"))
        );
        if !body.is_empty() {
            out.push_str(&format!(
                "<div class=\"empty-state__body\"><p data-test-body=\"\">{}</p></div>",
                body.trim_end()
            ));
        }
        out.push_str("</div></div>");
        return out;
    }
    let filter = m.route.filter();
    let show_reset_new = filter == Some("new");
    let show_dismiss_read = filter == Some("unread");
    let dismiss = |position: &str| {
        if position == "bottom" && topics.len() <= 5 {
            return String::new();
        }
        let mut out = format!("<div class=\"row dismiss-container-{position}\">");
        if show_dismiss_read {
            out.push_str(&format!(
                "<button class=\"btn btn-default dismiss-read\" id=\"dismiss-topics-{position}\" title=\"{}\" type=\"button\"><span class=\"d-button-label\">{}</span></button>",
                escape(&t(cx, "topics.bulk.dismiss_tooltip")),
                escape(&t(cx, "topics.bulk.dismiss_button"))
            ));
        }
        if show_reset_new {
            let label = if m.unified_new {
                t(cx, "topics.bulk.dismiss_all")
            } else {
                t(cx, "topics.bulk.dismiss_new")
            };
            out.push_str(&format!(
                "<button class=\"btn btn-icon-text btn-default dismiss-read\" id=\"dismiss-new-{position}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                icon("check", None),
                escape(&label)
            ));
        }
        out.push_str("</div>");
        out
    };
    let bulk_select = m.can_manage_topic || show_dismiss_read || show_reset_new;
    let users: &[Value] = m.list["users"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let rows: String = topics
        .iter()
        .map(|topic| crate::topic_list_view::row(cx, topic, users))
        .collect();
    let sortable = |class: &str, key: &str| {
        format!(
            "<th class=\"topic-list-data {class} sortable num\" data-sort-order=\"{class}\" scope=\"col\"><button>{}</button></th>",
            escape(&t(cx, key))
        )
    };
    format!(
        "<div class=\"paginated-topics-list\">{}<div><div class=\"loading-container\">\
         <table class=\"topic-list\"><caption class=\"sr-only\">{}</caption><thead class=\"topic-list-header\"><tr>\
         <th class=\"topic-list-data default\" data-sort-order=\"default\" scope=\"col\">{}<span>{}</span></th>\
         <th aria-label=\"{posters}\" class=\"topic-list-data posters\" data-sort-order=\"posters\" scope=\"col\"><span class=\"sr-only\">{posters}</span></th>\
         {}{}{}</tr></thead><tbody class=\"topic-list-body\">{rows}</tbody></table></div></div>{}\
         <div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div></div>",
        dismiss("top"),
        escape(&t(cx, "sr_topic_list_caption")),
        if bulk_select {
            format!(
                "<button class=\"btn-transparent bulk-select no-text\" title=\"{}\">{}</button> ",
                escape(&t(cx, "topics.bulk.toggle")),
                icon("list-check", None)
            )
        } else {
            String::new()
        },
        escape(&t(cx, "topic.title")),
        sortable("posts", "replies"),
        sortable("views", "views"),
        sortable("activity", "activity"),
        dismiss("bottom"),
        posters = escape(&t(cx, "posters")),
    )
}

/// templates/exception.gjs for an ajax error the route's model hit: the
/// reason and description by status, the URL it tried, and its buttons
/// (Go Back, and Try Again after a transition).
pub fn exception(cx: &ListContext, status: u16, request_url: &str) -> String {
    let (reason, desc) = match status {
        404 => ("errors.reasons.not_found", "errors.desc.not_found"),
        403 => ("errors.reasons.forbidden", "errors.desc.forbidden"),
        _ => ("errors.reasons.unknown", "errors.desc.unknown"),
    };
    format!(
        "<div class=\"container\"><div class=\"error-page\"><div class=\"face\">:(</div><div class=\"reason\">{}</div>\
         <div class=\"url\">{} <a data-auto-route=\"true\" href=\"{url}\">{url}</a></div><div class=\"desc\">{}</div>\
         <div class=\"buttons\"><button class=\"btn btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button>\
         <button class=\"btn btn-icon-text btn-primary\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>\
         <div class=\"loading-container\"></div></div></div></div>",
        escape(&t(cx, reason)),
        escape(&t(cx, "errors.prev_page")),
        escape(&t(cx, desc)),
        escape(&t(cx, "errors.buttons.back")),
        icon("arrows-rotate", None),
        escape(&t(cx, "errors.buttons.again")),
        url = escape(request_url),
    )
}

/// `encodeURIComponent`, for a group name in a route URL.
fn uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_names_are_uri_components() {
        assert_eq!(uri_component("moderators"), "moderators");
        assert_eq!(uri_component("a b"), "a%20b");
    }

    #[test]
    fn routes_know_their_filter() {
        assert_eq!(Route::Inbox.filter(), Some("inbox"));
        assert_eq!(Route::GroupArchive.filter(), Some("archive"));
        assert_eq!(Route::Warnings.filter(), None);
        assert!(Route::GroupNew.is_group());
        assert!(!Route::Sent.is_group());
    }
}
