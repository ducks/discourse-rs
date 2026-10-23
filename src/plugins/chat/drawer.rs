//! The chat drawer's routes, server-rendered (ChatDrawerRouter): what the
//! drawer draws for a chat URL, the way its ROUTES table maps a recognized
//! route to a drawer component. static/js/chat-drawer.js keeps the
//! drawer's state (ChatStateManager: open, expanded, the preferred mode,
//! the last chat and forum URLs) and asks for a route's markup here
//! (GET /live/chat/drawer) whenever it opens, navigates, collapses or
//! expands.
//!
//! The channels, direct messages, starred channels, browse and channel
//! routes are drawn; the others the drawer handles (threads, a thread, a
//! channel's threads, pins, info, search) are refused until their slice.

use super::channels::{Context, Found};
use crate::post_view::d_icon;
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// The drawer's navbar state: expanded or collapsed, and where the channel
/// route's back button goes.
pub struct Nav<'a> {
    pub expanded: bool,
    pub back: &'a str,
    pub i18n: &'a crate::i18n::I18n,
    pub base_path: &'a str,
}

impl Nav<'_> {
    fn t(&self, key: &str) -> String {
        self.i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
    }

    fn button(&self, class: &str, title: &str, icon: &str) -> String {
        format!(
            "<button class=\"btn no-text btn-icon btn-transparent no-text {class}\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&self.t(title)),
            d_icon(icon, None)
        )
    }

    /// ChatNavbarBackButton, shown while the drawer is expanded.
    pub fn back_button(&self, path: &str) -> String {
        if !self.expanded {
            return String::new();
        }
        format!(
            "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{}{path}\">{}</a>",
            escape(&self.t("chat.browse.back")),
            self.base_path,
            d_icon("chevron-left", None)
        )
    }

    /// The drawer's actions: ToggleDrawerButton, FullPageButton (while
    /// expanded) and CloseDrawerButton.
    pub fn buttons(&self) -> String {
        let mut out = if self.expanded {
            self.button("c-navbar__toggle-drawer-button", "chat.collapse", "minus")
        } else {
            self.button("c-navbar__toggle-drawer-button", "chat.expand", "angles-up")
        };
        if self.expanded {
            out.push_str(&self.button(
                "c-navbar__full-page-button",
                "chat.open_full_page",
                "discourse-expand",
            ));
        }
        out.push_str(&self.button("c-navbar__close-drawer-button", "chat.close", "xmark"));
        out
    }

    /// The list routes' navbar: clickable (toggling the drawer), the
    /// title and the drawer's actions.
    pub fn navbar(&self, title: &str) -> String {
        let title = escape(title);
        format!(
            "<div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"{title}\"><span class=\"c-navbar__title-text\">{title}</span></div><nav class=\"c-navbar__actions\">{}</nav></nav></div>",
            self.buttons()
        )
    }
}

/// chat-drawer.js's attributes for a viewer who can chat: the homepage
/// (`discoveryHomepageRoute`, where lastKnownAppURL falls back to), the
/// member's hide_presence for chat.js, whether they have starred channels
/// (lastKnownChatURL's default), and the scripts a channel in the drawer
/// needs, loaded when it first opens.
pub async fn script_attrs(
    conn: &mut sqlx::PgConnection,
    settings: &crate::site_settings::SiteSettings,
    guardian: &crate::guardian::Guardian,
    base_path: &str,
    starred: bool,
) -> Result<String, AppError> {
    let top_menu = settings.get("top_menu")?.to_s().to_string();
    let first = top_menu
        .split('|')
        .next()
        .and_then(|i| i.split(',').next())
        .unwrap_or("latest");
    let hide_presence: bool = match guardian.user_id() {
        Some(id) => sqlx::query_scalar("SELECT hide_presence FROM user_options WHERE user_id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false),
        None => false,
    };
    let scripts = ["emoji-picker.js", "autocomplete.js", "chat.js"]
        .iter()
        .map(|name| crate::assets::url(base_path, name))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!(
        " data-homepage=\"{}\" data-hide-presence=\"{hide_presence}\" data-starred=\"{starred}\" data-scripts=\"{}\"",
        escape(&format!("/{first}")),
        escape(&scripts)
    ))
}

/// A route the drawer draws.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// `chat`: the channels, after ROUTES.chat's redirect.
    Chat,
    Channels,
    DirectMessages,
    StarredChannels,
    /// `chat.browse` (the open tab) or `chat.browse.<tab>`.
    Browse {
        tab: &'static str,
        index: bool,
    },
    /// `chat.channel`, or `chat.channel.near-message` with a message.
    Channel {
        id: String,
        message: Option<i64>,
    },
}

impl Route {
    fn name(&self) -> String {
        match self {
            Route::Chat => "chat".into(),
            Route::Channels => "chat.channels".into(),
            Route::DirectMessages => "chat.direct-messages".into(),
            Route::StarredChannels => "chat.starred-channels".into(),
            Route::Browse { index: true, .. } => "chat.browse".into(),
            Route::Browse { tab, .. } => format!("chat.browse.{tab}"),
            Route::Channel { message: None, .. } => "chat.channel".into(),
            Route::Channel { .. } => "chat.channel.near-message".into(),
        }
    }
}

/// What the router recognizes in a chat URL, as the drawer sees it.
#[derive(Debug, PartialEq)]
pub enum Recognized {
    /// A route the drawer draws.
    Route(Route),
    /// A route the drawer handles in Ember but this port doesn't draw yet.
    Refused(&'static str),
    /// Not a route the drawer handles (canHandleRoute is false): the
    /// disabled page, the new message route, a message link, a channel's
    /// info index, or no chat route at all.
    Unhandled,
}

/// `router.recognize(url)` for the chat routes, then ROUTES' keys, the
/// index routes as their parent (`#forceParentRouteForIndex`). `path` is
/// without the base path and query.
pub fn recognize(path: &str) -> Recognized {
    let path = path.trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').skip(1).collect();
    let refused = Recognized::Refused;
    match parts.as_slice() {
        ["chat"] => Recognized::Route(Route::Chat),
        ["chat", "channels"] => Recognized::Route(Route::Channels),
        ["chat", "direct-messages"] => Recognized::Route(Route::DirectMessages),
        ["chat", "starred-channels"] => Recognized::Route(Route::StarredChannels),
        ["chat", "threads"] => refused("the chat drawer's threads route"),
        ["chat", "search"] => refused("the chat drawer's search route"),
        ["chat", "browse"] => Recognized::Route(Route::Browse {
            tab: "open",
            index: true,
        }),
        ["chat", "browse", tab] => {
            match super::page::BROWSE_TABS.iter().copied().find(|t| t == tab) {
                Some(tab) => Recognized::Route(Route::Browse { tab, index: false }),
                None => Recognized::Unhandled,
            }
        }
        ["chat", "c", _, id] => Recognized::Route(Route::Channel {
            id: (*id).to_string(),
            message: None,
        }),
        ["chat", "c", _, _, "t"] => refused("the chat drawer's channel threads route"),
        ["chat", "c", _, _, "pins"] => refused("the chat drawer's pinned messages route"),
        ["chat", "c", _, _, "info", "settings"] => {
            refused("the chat drawer's channel settings route")
        }
        ["chat", "c", _, _, "info", "members"] => {
            refused("the chat drawer's channel members route")
        }
        ["chat", "c", _, _, "t", _] | ["chat", "c", _, _, "t", _, _] => {
            refused("the chat drawer's thread route")
        }
        ["chat", "c", _, _, _, "t", _] => refused("the chat drawer's message in a thread"),
        ["chat", "c", _, id, message] => match message.parse::<i64>() {
            Ok(m) => Recognized::Route(Route::Channel {
                id: (*id).to_string(),
                message: Some(m),
            }),
            Err(_) => Recognized::Unhandled,
        },
        _ => Recognized::Unhandled,
    }
}

/// A route as the drawer draws it.
pub struct Drawn {
    pub html: String,
    /// The routes visited on the way (ChatHistory#visit): the one asked
    /// for, then any it redirected to.
    pub routes: Vec<String>,
    /// The channel the drawer shows (data-chat-channel-id).
    pub channel_id: Option<i64>,
}

/// `chatDrawerRouter.stateFor(route)` and the component it picks, drawn.
/// `previous` is ChatHistory's previous route, which the channel route's
/// back button follows. None when the drawer doesn't handle the URL.
pub async fn draw(
    cx: &mut Context<'_>,
    path: &str,
    expanded: bool,
    previous: Option<&str>,
) -> Result<Option<Drawn>, AppError> {
    let mut route = match recognize(path) {
        Recognized::Route(route) => route,
        Recognized::Refused(what) => return Err(Unsupported(what).into()),
        Recognized::Unhandled => return Ok(None),
    };
    let settings = cx.settings;
    let (i18n, base_path) = (cx.i18n, cx.base_path);
    let mut routes = Vec::new();
    let nav = |back: &'static str| Nav {
        expanded,
        back,
        i18n,
        base_path,
    };
    loop {
        routes.push(route.name());
        match route {
            Route::Chat => {
                // ROUTES.chat's redirect.
                let preferred = settings.get("chat_preferred_index")?.to_s().to_string();
                if preferred == "my_threads"
                    && settings.get("chat_threads_enabled")?.truthy()
                    && cx.has_viewable_threads().await?
                {
                    return Err(Unsupported("the chat drawer's threads route").into());
                }
                // chat.userCanAccessDirectMessages
                let dms = preferred == "direct_messages"
                    && (super::user_can_direct_message(&mut *cx.conn, settings, cx.guardian)
                        .await?
                        || !cx.followed_dm_channels(false).await?.is_empty());
                if dms || !settings.get("enable_public_channels")?.truthy() {
                    route = Route::DirectMessages;
                    continue;
                }
                let html = cx.drawer_channels_route(&nav("")).await?;
                return Ok(Some(Drawn {
                    html,
                    routes,
                    channel_id: None,
                }));
            }
            Route::Channels => {
                let html = cx.drawer_channels_route(&nav("")).await?;
                return Ok(Some(Drawn {
                    html,
                    routes,
                    channel_id: None,
                }));
            }
            Route::DirectMessages => {
                let html = cx.direct_messages_route(Some(&nav(""))).await?;
                return Ok(Some(Drawn {
                    html,
                    routes,
                    channel_id: None,
                }));
            }
            Route::StarredChannels => {
                if !cx.has_starred_channels().await? {
                    route = Route::Channels;
                    continue;
                }
                let html = cx.drawer_starred_route(&nav("")).await?;
                return Ok(Some(Drawn {
                    html,
                    routes,
                    channel_id: None,
                }));
            }
            Route::Browse { tab, .. } => {
                let html = cx.browse_page(tab, "", Some(&nav(""))).await?;
                return Ok(Some(Drawn {
                    html,
                    routes,
                    channel_id: None,
                }));
            }
            Route::Channel { id, message } => {
                // chatChannelsManager.find, which ignores the slug.
                let channel = match cx.find_joinable(&id).await? {
                    Found::Channel(channel) => channel,
                    Found::NotFound | Found::Forbidden => {
                        return Err(
                            Unsupported("the chat drawer for a channel it can't open").into()
                        );
                    }
                };
                // backBtnRoute
                let back = match previous {
                    Some("chat.browse") => "/chat/browse",
                    Some("chat.starred-channels") => "/chat/starred-channels",
                    _ if channel.chatable_type == "DirectMessage" => "/chat/direct-messages",
                    _ => "/chat/channels",
                };
                let html = cx.channel_page(&channel, message, Some(&nav(back))).await?;
                return Ok(Some(Drawn {
                    html,
                    routes,
                    channel_id: Some(channel.id),
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_drawer_routes() {
        let route = |p: &str| match recognize(p) {
            Recognized::Route(r) => Some(r.name()),
            _ => None,
        };
        assert_eq!(route("/chat").as_deref(), Some("chat"));
        assert_eq!(route("/chat/").as_deref(), Some("chat"));
        assert_eq!(route("/chat/channels").as_deref(), Some("chat.channels"));
        assert_eq!(route("/chat/browse").as_deref(), Some("chat.browse"));
        assert_eq!(
            route("/chat/browse/all").as_deref(),
            Some("chat.browse.all")
        );
        assert_eq!(route("/chat/c/general/2").as_deref(), Some("chat.channel"));
        assert_eq!(
            route("/chat/c/general/2/7").as_deref(),
            Some("chat.channel.near-message")
        );
        for unhandled in [
            "/chat/disabled",
            "/chat/new-message",
            "/chat/message/3",
            "/chat/c/general/2/info",
            "/chat/browse/nope",
            "/latest",
        ] {
            assert_eq!(recognize(unhandled), Recognized::Unhandled, "{unhandled}");
        }
        for refused in [
            "/chat/threads",
            "/chat/search",
            "/chat/c/general/2/t",
            "/chat/c/general/2/t/1",
            "/chat/c/general/2/t/1/9",
            "/chat/c/general/2/9/t/1",
            "/chat/c/general/2/pins",
            "/chat/c/general/2/info/settings",
            "/chat/c/general/2/info/members",
        ] {
            assert!(
                matches!(recognize(refused), Recognized::Refused(_)),
                "{refused}"
            );
        }
    }
}
