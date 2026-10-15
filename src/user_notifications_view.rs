//! The notifications pages under the profile as the Ember app renders
//! them: user-notifications.gjs's navigation and Dismiss All, the index
//! (user/notifications-index.gjs: NotificationsFilter and the
//! notifications as the user menu draws them, each with its date), the
//! stream tabs (user/stream.gjs) and discourse-reactions' received
//! reactions.

use serde_json::Value;

use crate::topic_list_view::{ListContext, escape, icon, long_date, relative_age_tiny, t, t_with};
use crate::user_menu_view::MenuContext;

/// The userNotifications routes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Route {
    Index,
    Responses,
    LikesReceived,
    Mentions,
    Edits,
    Links,
    /// discourse-reactions' userNotifications.reactionsReceived.
    ReactionsReceived,
}

impl Route {
    /// `/u/:username/notifications(/<path>)`
    pub fn from_path(path: &str) -> Option<Route> {
        Some(match path {
            "" => Route::Index,
            "responses" => Route::Responses,
            "likes-received" => Route::LikesReceived,
            "mentions" => Route::Mentions,
            "edits" => Route::Edits,
            "links" => Route::Links,
            "reactions-received" => Route::ReactionsReceived,
            _ => return None,
        })
    }

    /// The stream routes' UserAction type.
    pub fn action_type(self) -> Option<i32> {
        match self {
            Route::Responses => Some(6),
            Route::LikesReceived => Some(2),
            Route::Mentions => Some(7),
            Route::Edits => Some(11),
            Route::Links => Some(17),
            Route::Index | Route::ReactionsReceived => None,
        }
    }

    /// The route's titleToken.
    pub fn title_key(self) -> &'static str {
        match self {
            Route::Index => "user.filters.all",
            Route::Responses => "user_action_groups.6",
            Route::LikesReceived => "user_action_groups.1",
            Route::Mentions => "user_action_groups.7",
            Route::Edits => "user_action_groups.11",
            Route::Links => "user_action_groups.17",
            Route::ReactionsReceived => "discourse_reactions.reactions_title",
        }
    }
}

/// The secondary navigation and Dismiss All: `dismiss` is None without
/// notifications, else whether every one is read (the button disabled).
pub fn nav(
    cx: &ListContext,
    user_path: &str,
    route: Route,
    mentions: bool,
    reactions: bool,
    dismiss: Option<bool>,
) -> String {
    let mut out = String::from(
        "<div class=\"user-navigation user-navigation-secondary\">\
         <nav aria-label=\"User secondary - notifications\" class=\"horizontal-overflow-nav\">\
         <div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\">\
         <ul class=\"nav-pills action-list\" data-d-scroll-axis=\"horizontal\">",
    );
    let mut items = vec![
        (
            "user-nav__notifications-all",
            "",
            "bell",
            "user.filters.all",
            Route::Index,
        ),
        (
            "user-nav__notifications-responses",
            "/responses",
            "reply",
            "user_action_groups.5",
            Route::Responses,
        ),
        (
            "user-nav__notifications-likes",
            "/likes-received",
            "heart",
            "user_action_groups.2",
            Route::LikesReceived,
        ),
    ];
    if mentions {
        items.push((
            "user-nav__notifications-mentions",
            "/mentions",
            "at",
            "user_action_groups.7",
            Route::Mentions,
        ));
    }
    items.push((
        "user-nav__notifications-edits",
        "/edits",
        "pencil",
        "user_action_groups.11",
        Route::Edits,
    ));
    items.push((
        "user-nav__notifications-links",
        "/links",
        "link",
        "user_action_groups.17",
        Route::Links,
    ));
    for (class, path, icon_name, label, item) in items {
        let current = item == route;
        out.push_str(&format!(
            "<li{} class=\"{class}\"><a class=\"{}\" href=\"{user_path}/notifications{path}\">{} <span>{}</span></a></li>",
            if current {
                " aria-current=\"location\""
            } else {
                ""
            },
            if current { "active" } else { "" },
            icon(icon_name, None),
            escape(&t(cx, label))
        ));
    }
    // discourse-reactions' user-notifications-bottom connector: the item
    // stays, its link only while the plugin is on.
    out.push_str(
        "<li class=\"user-notifications-bottom-outlet discourse-reactions-user-notification-reactions\">",
    );
    if reactions {
        let current = route == Route::ReactionsReceived;
        out.push_str(&format!(
            "<a class=\"{}\" href=\"{user_path}/notifications/reactions-received\">{} <span>{}</span></a>",
            if current { "active" } else { "" },
            icon("far-face-smile", None),
            escape(&t(cx, "discourse_reactions.reactions_title"))
        ));
    }
    out.push_str("</li></ul></div></nav>");
    if let Some(all_read) = dismiss {
        out.push_str(&format!(
            "<div class=\"navigation-controls\"><button class=\"btn btn-icon-text btn-default dismiss-notifications\"{} title=\"{}\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button></div>",
            if all_read { " disabled" } else { "" },
            escape(&t(cx, "user.dismiss_notifications_tooltip")),
            icon("check", None),
            escape(&t(cx, "user.dismiss_notifications"))
        ));
    }
    out.push_str("</div>");
    out
}

/// DLoadMore's tail.
const LOAD_MORE: &str = "<div class=\"loading-container\"></div><div aria-hidden=\"true\" class=\"load-more-sentinel\"></div>";

/// NotificationsFilter: a DropdownSelectBox whose header reads
/// "Filter By <filter>".
fn filter(cx: &ListContext, value: &str) -> String {
    let name = t(cx, &format!("user.user_notifications.filters.{value}"));
    let filter_by = escape(&t_with(cx, "select_kit.filter_by", &[("name", &name)]));
    let rows: String = ["all", "read", "unread"]
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let label = escape(&t(cx, &format!("user.user_notifications.filters.{id}")));
            let is = *id == value;
            format!(
                "<li aria-checked=\"{is}\" role=\"menuitemradio\" data-index=\"{i}\" data-name=\"{label}\" data-value=\"{id}\" title=\"{label}\" tabindex=\"0\" class=\"select-kit-row dropdown-select-box-row{}\"><div class=\"texts\"><span class=\"name\">{label}</span></div></li>",
                if is { " is-highlighted is-selected" } else { "" }
            )
        })
        .collect();
    format!(
        "<details class=\"select-kit single-select dropdown-select-box notifications-filter has-selection\"><summary aria-label=\"{filter_by}\" name=\"{filter_by}\" data-name=\"{n}\" data-value=\"{value}\" tabindex=\"0\" class=\"select-kit-header single-select-header dropdown-select-box-header notifications-filter-header btn-flat btn btn-icon-text btn-default\"><div class=\"select-kit-header-wrapper\"><span class=\"filter-text\">{}</span> <span class=\"header-text\">{n}</span> {}</div></summary><div class=\"select-kit-body\"><ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\">{rows}</ul></div></details>",
        escape(&t(cx, "user.user_notifications.filters.filter_by")),
        icon("angle-down", Some("angle-icon")),
        n = escape(&name),
    )
}

/// user/notifications-index.gjs: the notifications (notifications#index's
/// `notifications`) under the filter, or the empty states.
pub fn index(
    cx: &ListContext,
    mcx: &MenuContext,
    notifications: &[Value],
    filter_value: &str,
) -> String {
    let filtered = filter_value != "all";
    let mut out = String::from(
        "<section class=\"user-content\" id=\"user-content\"><div class=\"notification-history user-stream\">",
    );
    if !filtered && notifications.is_empty() {
        out.push_str(&format!(
            "<div class=\"empty-state__container\"><div class=\"empty-state\"><div class=\"empty-state__title\" data-test-title>{}</div><div class=\"empty-state__body\"><p data-test-body>{}</p></div></div></div>",
            escape(&t(cx, "user.no_notifications_page_title")),
            t_with(
                cx,
                "user.no_notifications_page_body",
                &[
                    (
                        "preferencesUrl",
                        &format!("{}/my/preferences/notifications", cx.base_path)
                    ),
                    ("icon", &icon("bell", None)),
                ]
            )
        ));
    } else {
        out.push_str(&format!(
            "<div class=\"user-notifications-filter\">{}</div>",
            filter(cx, filter_value)
        ));
        if filtered && notifications.is_empty() {
            out.push_str(&format!(
                "<div class=\"alert alert-info\">{}</div>",
                escape(&t(cx, "notifications.empty"))
            ));
        } else {
            out.push_str("<div class=\"user-notifications-list\">");
            for n in notifications {
                let date = n["created_at"]
                    .as_str()
                    .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
                    .map(|d| d.with_timezone(&chrono::Utc));
                let end = match date {
                    Some(at) => format!(
                        "<span class=\"relative-date\" title=\"{}\" data-time=\"{}\" data-format=\"tiny\">{}</span>",
                        escape(&long_date(cx, at)),
                        at.timestamp_millis(),
                        escape(&relative_age_tiny(cx, at))
                    ),
                    None => String::new(),
                };
                out.push_str(&crate::user_menu_view::notification_item_with_end(
                    mcx, n, &end,
                ));
            }
            out.push_str("<div class=\"loading-container\"></div></div>");
        }
    }
    out.push_str(LOAD_MORE);
    out.push_str("</div></section>");
    out
}

/// user/stream.gjs under the notifications' navigation.
pub fn stream(cx: &ListContext, action_type: i32, stream: &[Value]) -> String {
    let (empty, list) = crate::user_profile_view::stream_parts(
        cx,
        action_type,
        stream,
        &t(cx, "user_activity.no_activity_title"),
    );
    format!(
        "<section class=\"user-content\" id=\"user-content\"><div class=\"notification-history user-stream\">{empty}<div>{list}{LOAD_MORE}</div>{LOAD_MORE}</div></section>"
    )
}

/// discourse-reactions' reactions template for the received ones.
pub fn reactions(
    cx: &ListContext,
    ui: &crate::plugins::reactions::view::ReactionsUi,
    reactions: &[Value],
) -> String {
    format!(
        "<section class=\"user-content\" id=\"user-content\"><div class=\"notification-history user-stream\">{}{LOAD_MORE}</div></section>",
        crate::user_profile_view::reactions_list(cx, ui, reactions)
    )
}
