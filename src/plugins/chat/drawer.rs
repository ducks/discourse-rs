//! The chat drawer's routes, server-rendered (ChatDrawerRouter): what the
//! drawer draws for a chat URL, the way its ROUTES table maps a recognized
//! route to a drawer component. static/js/chat-drawer.js keeps the
//! drawer's state (ChatStateManager: open, expanded, the preferred mode,
//! the last chat and forum URLs) and asks for a route's markup here
//! (GET /live/chat/drawer) whenever it opens, navigates, collapses or
//! expands.
//!
//! Each route of ROUTES is drawn: the lists (channels, direct messages,
//! starred channels, browse, the member's threads, search) and a
//! channel's routes (the channel, its settings and members, its pins, its
//! threads and a thread), each with what its full-page counterpart draws.

use serde_json::Value;

use super::channels::{ChannelRow, Context, Found};
use super::info::Tab;
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
        self.back_link(path, "chat.browse.back")
    }

    /// ChatNavbarBackButton with the route's @title.
    pub fn back_link(&self, path: &str, title: &str) -> String {
        if !self.expanded {
            return String::new();
        }
        format!(
            "<a class=\"c-navbar__back-button no-text btn-transparent btn\" title=\"{}\" href=\"{}{path}\">{}</a>",
            escape(&self.t(title)),
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

    /// A channel route's navbar: the back button (`back`: the path and
    /// the title's key), a title with its icon (ChatNavbarTitle; `title`
    /// the attribute and `title_html` the text, both escaped) and the
    /// drawer's actions.
    pub fn titled_navbar(
        &self,
        back: (&str, &str),
        icon: &str,
        title: &str,
        title_html: &str,
    ) -> String {
        self.titled_navbar_with(back, icon, title, title_html, "")
    }

    /// titled_navbar with `outlet`, a plugin outlet's HTML, before the
    /// drawer's actions.
    pub fn titled_navbar_with(
        &self,
        back: (&str, &str),
        icon: &str,
        title: &str,
        title_html: &str,
        outlet: &str,
    ) -> String {
        format!(
            "<div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\">{}<div class=\"c-navbar__title\" title=\"{title}\"><span class=\"c-navbar__title-text\">{}{title_html}</span></div><nav class=\"c-navbar__actions\">{outlet}{}</nav></nav></div>",
            self.back_link(back.0, back.1),
            d_icon(icon, None),
            self.buttons()
        )
    }

    /// The channel info routes' navbar: the back button to the channel,
    /// ChatNavbarChannelTitle (linking to the settings while expanded,
    /// with the star for a member following it) and the drawer's actions.
    pub fn channel_navbar(
        &self,
        c: &serde_json::Value,
        emojis: &crate::emojis::Emojis,
        online: &[i64],
    ) -> String {
        let channel_path = format!(
            "/chat/c/{}/{}",
            escape(&super::page::slugified_title(c)),
            c["id"]
        );
        let title = super::page::channel_title(c, emojis, self.base_path, online);
        let mut out = format!(
            "<div class=\"c-navbar-container -clickable\"><nav class=\"c-navbar\">{}",
            self.back_link(&channel_path, "chat.return_to_channel")
        );
        if self.expanded {
            out.push_str(&format!(
                "<a class=\"c-navbar__channel-title\" href=\"{}{channel_path}/info/settings\">{title}</a>",
                self.base_path
            ));
        } else {
            out.push_str(&format!(
                "<div class=\"c-navbar__channel-title\">{title}</div>"
            ));
        }
        let membership = &c["current_user_membership"];
        if membership["following"] == true && self.expanded {
            let starred = membership["starred"] == true;
            out.push_str(&format!(
                "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger\" data-identifier=\"star-channel\" data-trigger=\"\" role=\"button\" data-tooltip=\"{}\"><span class=\"fk-d-tooltip__trigger-container\"><button class=\"btn no-text btn-icon btn-transparent c-navbar__star-channel-button{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></span></span>",
                escape(&self.t(if starred {
                    "chat.channel_settings.unstar_channel"
                } else {
                    "chat.channel_settings.star_channel"
                })),
                if starred { " --starred" } else { "" },
                d_icon(if starred { "star" } else { "far-star" }, None)
            ));
        }
        out.push_str(&format!(
            "<nav class=\"c-navbar__actions\">{}</nav></nav></div>",
            self.buttons()
        ));
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
/// (lastKnownChatURL's default), and the scripts the drawer's routes need
/// (a channel, a thread, a channel's info, search), loaded when it first
/// opens.
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
    let scripts = [
        "emoji-picker.js",
        "autocomplete.js",
        "chat.js",
        "chat-thread.js",
        "chat-channel-info.js",
        "chat-search.js",
    ]
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
    /// `chat.threads`: the member's threads.
    Threads,
    Search,
    /// `chat.browse` (the open tab) or `chat.browse.<tab>`.
    Browse {
        tab: &'static str,
        index: bool,
    },
    /// `chat.channel`, `chat.channel.near-message` with a message, or
    /// `chat.channel.near-message-with-thread`, which the drawer draws as
    /// the channel around the message (it has no side panel).
    Channel {
        id: String,
        message: Option<i64>,
        with_thread: bool,
    },
    /// `chat.channel.info.settings` or `chat.channel.info.members`.
    ChannelInfo {
        id: String,
        tab: Tab,
    },
    /// `chat.channel.pins`
    ChannelPins {
        id: String,
    },
    /// `chat.channel.threads`
    ChannelThreads {
        id: String,
    },
    /// `chat.channel.thread`, or `chat.channel.thread.near-message` with
    /// a message.
    ChannelThread {
        id: String,
        thread: String,
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
            Route::Threads => "chat.threads".into(),
            Route::Search => "chat.search".into(),
            Route::Browse { index: true, .. } => "chat.browse".into(),
            Route::Browse { tab, .. } => format!("chat.browse.{tab}"),
            Route::Channel {
                with_thread: true, ..
            } => "chat.channel.near-message-with-thread".into(),
            Route::Channel { message: None, .. } => "chat.channel".into(),
            Route::Channel { .. } => "chat.channel.near-message".into(),
            Route::ChannelInfo {
                tab: Tab::Settings, ..
            } => "chat.channel.info.settings".into(),
            Route::ChannelInfo { .. } => "chat.channel.info.members".into(),
            Route::ChannelPins { .. } => "chat.channel.pins".into(),
            Route::ChannelThreads { .. } => "chat.channel.threads".into(),
            Route::ChannelThread { message: None, .. } => "chat.channel.thread".into(),
            Route::ChannelThread { .. } => "chat.channel.thread.near-message".into(),
        }
    }
}

/// `router.recognize(url)` for the chat routes, then ROUTES' keys, the
/// index routes as their parent (`#forceParentRouteForIndex`). `path` is
/// without the base path and query. None when the drawer doesn't handle
/// the route (canHandleRoute is false): the disabled page, the new
/// message route, a message link, a channel's info index, or no chat
/// route at all.
pub fn recognize(path: &str) -> Option<Route> {
    let path = path.trim_end_matches('/');
    let parts: Vec<&str> = path.split('/').skip(1).collect();
    let message = |m: &str| m.parse::<i64>().ok();
    Some(match parts.as_slice() {
        ["chat"] => Route::Chat,
        ["chat", "channels"] => Route::Channels,
        ["chat", "direct-messages"] => Route::DirectMessages,
        ["chat", "starred-channels"] => Route::StarredChannels,
        ["chat", "threads"] => Route::Threads,
        ["chat", "search"] => Route::Search,
        ["chat", "browse"] => Route::Browse {
            tab: "open",
            index: true,
        },
        ["chat", "browse", tab] => {
            let tab = super::page::BROWSE_TABS
                .iter()
                .copied()
                .find(|t| t == tab)?;
            Route::Browse { tab, index: false }
        }
        ["chat", "c", _, id] => Route::Channel {
            id: (*id).to_string(),
            message: None,
            with_thread: false,
        },
        ["chat", "c", _, id, "t"] => Route::ChannelThreads {
            id: (*id).to_string(),
        },
        ["chat", "c", _, id, "pins"] => Route::ChannelPins {
            id: (*id).to_string(),
        },
        ["chat", "c", _, id, "info", tab] => Route::ChannelInfo {
            id: (*id).to_string(),
            tab: match *tab {
                "settings" => Tab::Settings,
                "members" => Tab::Members,
                _ => return None,
            },
        },
        ["chat", "c", _, id, "t", thread] => Route::ChannelThread {
            id: (*id).to_string(),
            thread: (*thread).to_string(),
            message: None,
        },
        ["chat", "c", _, id, "t", thread, m] => Route::ChannelThread {
            id: (*id).to_string(),
            thread: (*thread).to_string(),
            message: Some(message(m)?),
        },
        ["chat", "c", _, id, m, "t", _] => Route::Channel {
            id: (*id).to_string(),
            message: Some(message(m)?),
            with_thread: true,
        },
        ["chat", "c", _, id, m] => Route::Channel {
            id: (*id).to_string(),
            message: Some(message(m)?),
            with_thread: false,
        },
        _ => return None,
    })
}

/// A route as the drawer draws it.
pub struct Drawn {
    pub html: String,
    /// The routes visited on the way (ChatHistory#visit): the one asked
    /// for, then any it redirected to.
    pub routes: Vec<String>,
    /// The channel the drawer shows (data-chat-channel-id): the model's
    /// channel (activeChannelId).
    pub channel_id: Option<i64>,
}

/// `chatDrawerRouter.stateFor(route)` and the component it picks, drawn.
/// `previous` is ChatHistory's previous route, which the channel and
/// thread routes' back buttons follow. None when the drawer doesn't
/// handle the URL.
pub async fn draw(
    cx: &mut Context<'_>,
    path: &str,
    expanded: bool,
    previous: Option<&str>,
) -> Result<Option<Drawn>, AppError> {
    let Some(mut route) = recognize(path) else {
        return Ok(None);
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
    let list = |html: String, routes: Vec<String>| {
        Ok(Some(Drawn {
            html,
            routes,
            channel_id: None,
        }))
    };
    loop {
        routes.push(route.name());
        let id = match &route {
            Route::Chat => {
                // ROUTES.chat's redirect.
                let preferred = settings.get("chat_preferred_index")?.to_s().to_string();
                if preferred == "my_threads"
                    && settings.get("chat_threads_enabled")?.truthy()
                    && cx.has_viewable_threads().await?
                {
                    route = Route::Threads;
                    continue;
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
                return list(cx.drawer_channels_route(&nav("")).await?, routes);
            }
            Route::Channels => {
                return list(cx.drawer_channels_route(&nav("")).await?, routes);
            }
            Route::DirectMessages => {
                return list(cx.direct_messages_route(Some(&nav(""))).await?, routes);
            }
            Route::StarredChannels => {
                if !cx.has_starred_channels().await? {
                    route = Route::Channels;
                    continue;
                }
                return list(cx.drawer_starred_route(&nav("")).await?, routes);
            }
            Route::Threads => {
                return list(cx.drawer_threads_route(&nav("")).await?, routes);
            }
            Route::Search => {
                return list(cx.drawer_search_route(&nav("")).await?, routes);
            }
            Route::Browse { tab, .. } => {
                return list(cx.browse_page(tab, "", Some(&nav(""))).await?, routes);
            }
            Route::Channel { id, .. }
            | Route::ChannelInfo { id, .. }
            | Route::ChannelPins { id }
            | Route::ChannelThreads { id }
            | Route::ChannelThread { id, .. } => id.clone(),
        };
        // The channel routes' model: chatChannelsManager.find, which
        // ignores the slug.
        let channel = match cx.find_joinable(&id).await? {
            Found::Channel(channel) => channel,
            Found::NotFound | Found::Forbidden => {
                return Err(Unsupported("the chat drawer for a channel it can't open").into());
            }
        };
        let html = match route {
            Route::Channel { message, .. } => {
                // backBtnRoute
                let back = match previous {
                    Some("chat.browse") => "/chat/browse",
                    Some("chat.starred-channels") => "/chat/starred-channels",
                    _ if channel.chatable_type == "DirectMessage" => "/chat/direct-messages",
                    _ => "/chat/channels",
                };
                cx.channel_page(&channel, message, Some(&nav(back))).await?
            }
            Route::ChannelInfo { tab, .. } => {
                cx.drawer_channel_info(&channel, tab, &nav("")).await?
            }
            Route::ChannelPins { .. } => pins_route(cx, &channel, &nav("")).await?,
            Route::ChannelThreads { .. } => threads_route(cx, &channel, &nav("")).await?,
            Route::ChannelThread {
                thread, message, ..
            } => thread_route(cx, &channel, &thread, message, previous, &nav("")).await?,
            _ => unreachable!("a list route returned above"),
        };
        return Ok(Some(Drawn {
            html,
            routes,
            channel_id: Some(channel.id),
        }));
    }
}

/// The channel's serialized JSON, with the member's membership.
async fn channel_json(cx: &mut Context<'_>, channel: &ChannelRow) -> Result<Value, AppError> {
    let membership = cx
        .memberships()
        .await?
        .into_iter()
        .find(|m| m.chat_channel_id == channel.id);
    cx.channel(channel, membership.as_ref(), None, None).await
}

/// The channel's path, the way its route models link to it.
fn channel_path(c: &Value) -> String {
    format!(
        "/chat/c/{}/{}",
        escape(&super::page::slugified_title(c)),
        c["id"]
    )
}

/// `<label> - <the channel's title>`, its emoji drawn (the pins and
/// threads routes' title: escapedTitle through dReplaceEmoji).
fn channel_label(cx: &Context<'_>, nav: &Nav<'_>, key: &str, c: &Value) -> String {
    format!(
        "{} - {}",
        escape(&nav.t(key)),
        crate::topic_list_view::emoji_unescape(
            &escape(c["title"].as_str().unwrap_or_default()),
            cx.settings.emojis(),
            cx.base_path
        )
    )
}

/// ChatDrawerRoutesChannelPins: its navbar (back to the channel, the
/// pinned messages title), then while expanded ChatPinnedMessagesList as
/// the full-page pins draw it. The pins API's 404 while
/// chat_pinned_messages is off is refused.
async fn pins_route(
    cx: &mut Context<'_>,
    channel: &ChannelRow,
    nav: &Nav<'_>,
) -> Result<String, AppError> {
    if !cx.settings.get("chat_pinned_messages")?.truthy() {
        return Err(Unsupported("the chat drawer's pins with pinned messages off").into());
    }
    let c = channel_json(cx, channel).await?;
    let title = channel_label(cx, nav, "chat.pinned_messages.title", &c);
    let mut out = format!(
        "<div class=\"c-drawer-routes --channel-pins\">{}",
        nav.titled_navbar(
            (&channel_path(&c), "chat.return_to_list"),
            "thumbtack",
            &escape(&title),
            &title
        )
    );
    if nav.expanded {
        out.push_str(&format!(
            "<div class=\"chat-drawer-content\">{}</div>",
            cx.pinned_messages_list(channel).await?
        ));
    }
    out.push_str("</div>");
    Ok(out)
}

/// ChatDrawerRoutesChannelThreads: its navbar (back to the channel, the
/// threads title), then while expanded ChatThreadList as the full-page
/// threads route draws it.
async fn threads_route(
    cx: &mut Context<'_>,
    channel: &ChannelRow,
    nav: &Nav<'_>,
) -> Result<String, AppError> {
    let c = channel_json(cx, channel).await?;
    let title = channel_label(cx, nav, "chat.threads.list", &c);
    let navbar = nav.titled_navbar(
        (&channel_path(&c), "chat.return_to_list"),
        "discourse-threads",
        &escape(&title),
        &title,
    );
    if !nav.expanded {
        return Ok(format!(
            "<div class=\"c-drawer-routes --channel-threads\">{navbar}</div>"
        ));
    }
    cx.channel_threads_page_in(channel, Some(&navbar)).await
}

/// ChatDrawerRoutesChannelThread: its navbar (back to where the member
/// came from, the thread's title), then while expanded ChatThread as the
/// full-page thread route draws it, around `message`. A thread that
/// isn't the channel's or doesn't show is refused (Ember pops the
/// request's error over an empty route).
async fn thread_route(
    cx: &mut Context<'_>,
    channel: &ChannelRow,
    thread_id: &str,
    message: Option<i64>,
    previous: Option<&str>,
    nav: &Nav<'_>,
) -> Result<String, AppError> {
    let thread = match thread_id.parse::<i64>() {
        Ok(id) => cx.shown_thread(channel, id).await?,
        Err(_) => None,
    };
    let Some(thread) = thread else {
        return Err(Unsupported("the chat drawer for a thread it can't open").into());
    };
    let c = channel_json(cx, channel).await?;
    // backButton
    let channel_path = channel_path(&c);
    let back = match previous {
        Some("chat.channel.threads") => {
            (format!("{channel_path}/t"), "chat.return_to_threads_list")
        }
        Some("chat.threads") => ("/chat/threads".to_string(), "chat.my_threads.title"),
        _ => (channel_path, "chat.return_to_channel"),
    };
    let title = escape(
        &thread
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| nav.t("chat.thread.label")),
    );
    let outlet = cx.thread_navbar_outlet(thread.id, &c, "drawer").await?;
    let navbar = nav.titled_navbar_with(
        (&back.0, back.1),
        "discourse-threads",
        &title,
        &title,
        &outlet,
    );
    if !nav.expanded {
        return Ok(format!(
            "<div class=\"c-drawer-routes --channel-thread\">{navbar}</div>"
        ));
    }
    cx.thread_page_in(channel, &thread, message, Some(&navbar))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_drawer_routes() {
        let route = |p: &str| recognize(p).map(|r| r.name());
        for (path, name) in [
            ("/chat", "chat"),
            ("/chat/", "chat"),
            ("/chat/channels", "chat.channels"),
            ("/chat/threads", "chat.threads"),
            ("/chat/search", "chat.search"),
            ("/chat/browse", "chat.browse"),
            ("/chat/browse/all", "chat.browse.all"),
            ("/chat/c/general/2", "chat.channel"),
            ("/chat/c/general/2/7", "chat.channel.near-message"),
            (
                "/chat/c/general/2/9/t/1",
                "chat.channel.near-message-with-thread",
            ),
            ("/chat/c/general/2/t", "chat.channel.threads"),
            ("/chat/c/general/2/t/1", "chat.channel.thread"),
            (
                "/chat/c/general/2/t/1/9",
                "chat.channel.thread.near-message",
            ),
            ("/chat/c/general/2/pins", "chat.channel.pins"),
            (
                "/chat/c/general/2/info/settings",
                "chat.channel.info.settings",
            ),
            (
                "/chat/c/general/2/info/members",
                "chat.channel.info.members",
            ),
        ] {
            assert_eq!(route(path).as_deref(), Some(name), "{path}");
        }
        for unhandled in [
            "/chat/disabled",
            "/chat/new-message",
            "/chat/message/3",
            "/chat/c/general/2/info",
            "/chat/c/general/2/info/nope",
            "/chat/browse/nope",
            "/latest",
        ] {
            assert_eq!(recognize(unhandled), None, "{unhandled}");
        }
    }
}
