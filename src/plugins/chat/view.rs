//! Chat's pieces of every page's chrome, as the plugin's initializers add
//! them: the header's chat icon (ChatHeaderIcon with its unread
//! indicator) and the sidebar's sections (chat-sidebar.js: search, starred
//! channels, channels, direct messages), from the channels
//! GET /chat/api/me/channels gives (the preloaded channels).
//!
//! The sections' menus (channel list options, a channel's menu) are drawn
//! as their buttons. Threads are refused upstream (channels.rs) until
//! their slice, and only the "never" separate sidebar mode (chat's
//! sections in the main sidebar) is drawn.

use std::cmp::Ordering;

use serde_json::Value;
use sqlx::PgConnection;

use super::channels::Context;
use crate::post_view::d_icon;
use crate::sidebar::{Active, Link, LinkExtra, Prefix};
use crate::topic_list_view::escape;
use crate::{AppError, Unsupported};

/// What chat adds to a page's chrome.
pub struct ChatChrome {
    /// The `<li>` before the current user's in the header.
    pub header_icon: String,
    /// The sidebar sections after the core ones.
    pub sidebar_sections: String,
    /// ChatHeaderIconUnreadIndicator for the channel route's back button
    /// in the mobile view, counting the channels but the open one.
    pub back_indicator: String,
    /// chatChannelsManager.hasStarredChannels, for the drawer's
    /// lastKnownChatURL.
    pub starred: bool,
}

/// A direct message channel's part of its sidebar link.
struct DmLink {
    group: bool,
    /// The chatable's users (the viewer left out unless alone).
    users: Vec<Value>,
    memberships_count: i64,
    unicode_title: Option<String>,
}

/// The preloaded channels: public ones and DMs as the lists read them,
/// and the DMs' JSON.
struct Preloaded {
    channels: Vec<SidebarChannel>,
    dms: Vec<SidebarChannel>,
    dm_json: Vec<Value>,
}

/// A followed channel as the sidebar reads it.
struct SidebarChannel {
    id: i64,
    title: String,
    /// slugifiedTitle: the slug, else the title slugified.
    slug: String,
    dm: Option<DmLink>,
    emoji: Option<String>,
    description: Option<String>,
    color: Option<String>,
    read_restricted: bool,
    muted: bool,
    starred: bool,
    unread: i64,
    mentions: i64,
    watched_threads: i64,
    /// The last message's created_at, when there is one.
    last_activity: Option<String>,
}

impl SidebarChannel {
    fn from_json(c: &Value, tracking: &Value) -> Result<SidebarChannel, AppError> {
        let id = c["id"].as_i64().unwrap_or(0);
        let t = &tracking[id.to_string()];
        let membership = &c["current_user_membership"];
        let last = &c["last_message"];
        Ok(SidebarChannel {
            id,
            title: c["title"].as_str().unwrap_or_default().to_string(),
            slug: crate::routes::user_menu::slugify_channel(
                c["slug"].as_str(),
                &escape(c["title"].as_str().unwrap_or_default()),
            ),
            dm: (c["chatable_type"] == "DirectMessage").then(|| DmLink {
                group: c["chatable"]["group"] == true,
                users: c["chatable"]["users"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                memberships_count: c["memberships_count"].as_i64().unwrap_or(0),
                unicode_title: c["unicode_title"].as_str().map(str::to_string),
            }),
            emoji: c["emoji"].as_str().map(str::to_string),
            description: c["description"].as_str().map(str::to_string),
            color: c["chatable"]["color"].as_str().map(str::to_string),
            read_restricted: c["chatable"]["read_restricted"] == true,
            muted: membership["muted"] == true,
            starred: membership["starred"] == true,
            unread: t["unread_count"].as_i64().unwrap_or(0),
            mentions: t["mention_count"].as_i64().unwrap_or(0),
            watched_threads: t["watched_threads_unread_count"].as_i64().unwrap_or(0),
            last_activity: (last["id"].is_i64())
                .then(|| last["created_at"].as_str().map(str::to_string))
                .flatten(),
        })
    }

    /// `#sidebarUnreadCount`, with no unread threads.
    fn unread_total(&self) -> i64 {
        self.unread + self.mentions + self.watched_threads
    }

    /// `#sidebarUrgentCount`
    fn urgent(&self) -> i64 {
        self.mentions + self.watched_threads
    }

    /// `#compareChannelsAlphabetically`'s name: a DM's title, else the
    /// slug.
    fn sort_name(&self) -> &str {
        if self.dm.is_some() {
            &self.title
        } else {
            &self.slug
        }
    }
}

/// `#compareChannelsAlphabetically`
fn alphabetical(a: &SidebarChannel, b: &SidebarChannel) -> Ordering {
    a.sort_name().cmp(b.sort_name()).then(a.id.cmp(&b.id))
}

/// `#compareDirectMessageChannelsByActivity`, without unread threads
/// (not ported): channels with a message first; urgent ones (any unread
/// in a DM) before the rest, by count, both urgent by their last message;
/// then by the last message.
fn dm_activity(a: &SidebarChannel, b: &SidebarChannel) -> Ordering {
    let (Some(at_a), Some(at_b)) = (&a.last_activity, &b.last_activity) else {
        return match (&a.last_activity, &b.last_activity) {
            (None, None) => Ordering::Equal,
            (None, _) => Ordering::Greater,
            _ => Ordering::Less,
        };
    };
    let urgent = |c: &SidebarChannel| c.unread + c.mentions + c.watched_threads;
    let (ua, ub) = (urgent(a), urgent(b));
    // Only one urgent: it first. Both or neither: the latest message.
    if (ua > 0) != (ub > 0) {
        return ub.cmp(&ua);
    }
    at_b.cmp(at_a)
}

/// A section's list preferences (ChatChannelListPreferences).
#[derive(Clone, Copy)]
struct ListPrefs {
    filter: &'static str,
    sort: &'static str,
}

fn prefs(filter: i32, sort: i32) -> ListPrefs {
    let pick = |names: &[&'static str], v: i32, default: &'static str| {
        usize::try_from(v)
            .ok()
            .and_then(|i| names.get(i).copied())
            .unwrap_or(default)
    };
    ListPrefs {
        filter: pick(&super::LIST_FILTERS, filter, "all"),
        sort: pick(&super::LIST_SORTS, sort, "alphabetical"),
    }
}

/// The page's channel when full-page chat shows it.
fn active_channel(active: &Active) -> Option<i64> {
    match active {
        Active::ChatChannel(id) => Some(*id),
        _ => None,
    }
}

/// `#filterSidebarChannels` and `#compareSidebarChannels`. The active
/// filter's 30 days are measured from the page's clock.
fn filter_and_sort(
    channels: Vec<&SidebarChannel>,
    p: ListPrefs,
    active: Option<i64>,
) -> Vec<&SidebarChannel> {
    let mut channels = filter(channels, p, active);
    sort(&mut channels, p);
    channels
}

/// `#filterSidebarChannels`
fn filter(
    mut channels: Vec<&SidebarChannel>,
    p: ListPrefs,
    active: Option<i64>,
) -> Vec<&SidebarChannel> {
    let cutoff = crate::clock::now() - chrono::Duration::days(30);
    channels.retain(|c| {
        Some(c.id) == active
            || match p.filter {
                "all" => true,
                "active" => c
                    .last_activity
                    .as_deref()
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                    .is_some_and(|at| at >= cutoff),
                _ if c.muted => false,
                "unread" => c.unread_total() > 0,
                "mentions" => c.urgent() > 0,
                _ => true,
            }
    });
    channels
}

/// `#compareSidebarChannels`, sorting stably.
fn sort(channels: &mut [&SidebarChannel], p: ListPrefs) {
    let priority = |c: &SidebarChannel| {
        if c.muted {
            2
        } else if c.urgent() > 0 {
            0
        } else if c.unread_total() > 0 {
            1
        } else {
            2
        }
    };
    let recency =
        |a: &SidebarChannel, b: &SidebarChannel| match (&a.last_activity, &b.last_activity) {
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
            (Some(x), Some(y)) => y.cmp(x),
        };
    channels.sort_by(|a, b| match p.sort {
        "alphabetical" => alphabetical(a, b),
        "priority" => priority(a)
            .cmp(&priority(b))
            .then_with(|| recency(a, b))
            .then_with(|| alphabetical(a, b)),
        _ => recency(a, b).then_with(|| alphabetical(a, b)),
    });
}

impl Context<'_> {
    /// Chat's chrome for the viewer, None when they can't chat
    /// (`userCanChat`: the plugin on, and has_chat_enabled).
    pub async fn chrome(
        &mut self,
        active: &Active,
        sidebar: Option<&crate::sidebar::Context<'_>>,
    ) -> Result<Option<ChatChrome>, AppError> {
        let settings = self.settings;
        let g = self.guardian;
        let Some(user_id) = g.user_id() else {
            return Ok(None);
        };
        if !super::enabled(settings)? || !super::can_chat(&mut *self.conn, settings, g).await? {
            return Ok(None);
        }
        let Some(options) = super::options(&mut *self.conn, user_id).await? else {
            return Ok(None);
        };
        if !options.chat_enabled {
            return Ok(None);
        }
        // getUserChatSeparateSidebarMode: the user's, else the site's.
        let mode = match super::SIDEBAR_MODES
            .get(usize::try_from(options.chat_separate_sidebar_mode).unwrap_or(0))
            .copied()
        {
            Some("default") | None => settings
                .get("chat_separate_sidebar_mode")?
                .to_s()
                .to_string(),
            Some(mode) => mode.to_string(),
        };
        if mode != "never" {
            return Err(Unsupported("chat's separate sidebar").into());
        }

        let me = self.me_channels().await?;
        let tracking = &me["tracking"]["channel_tracking"];
        let read = |key: &str| -> Result<Vec<SidebarChannel>, AppError> {
            me[key]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| SidebarChannel::from_json(c, tracking))
                .collect()
        };
        let channels = read("public_channels")?;
        let dms = read("direct_message_channels")?;
        let full_page = matches!(active, Active::Chat | Active::ChatChannel(_));

        let header_icon = self.header_icon(&channels, &dms, &options, full_page)?;
        let sidebar_sections = match sidebar {
            Some(cx) => {
                self.sidebar_sections(&channels, &dms, &options, active, cx)
                    .await?
            }
            None => String::new(),
        };
        // The mobile channel route's back button: the other channels' count.
        let back_indicator = match active_channel(active) {
            Some(id) => unread_indicator(&channels, &dms, &options, Some(id)),
            None => String::new(),
        };
        Ok(Some(ChatChrome {
            header_icon,
            sidebar_sections,
            back_indicator,
            starred: channels.iter().chain(&dms).any(|c| c.starred),
        }))
    }

    /// The mobile view's channels route (ChatChannelsRoute with
    /// ChannelsListPublic): the navbar, every followed public channel by
    /// the member's channel list preferences, and the chat footer.
    pub async fn channels_route(&mut self) -> Result<String, AppError> {
        let settings = self.settings;
        let Some(user_id) = self.guardian.user_id() else {
            return Err(Unsupported("chat's channels list without a member").into());
        };
        let Some(options) = super::options(&mut *self.conn, user_id).await? else {
            return Err(Unsupported("chat's channels list without chat options").into());
        };
        let me = self.me_channels().await?;
        let tracking = &me["tracking"]["channel_tracking"];
        let json = |key: &str| me[key].as_array().cloned().unwrap_or_default();
        let public = json("public_channels");
        let channels = public
            .iter()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<Vec<_>, _>>()?;
        let dms = json("direct_message_channels")
            .iter()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<Vec<_>, _>>()?;
        let channel_prefs = prefs(
            options.chat_channel_list_filter,
            options.chat_channel_list_sort,
        );
        if channels.is_empty() {
            return Err(Unsupported("chat's empty channels list").into());
        }
        let list = filter_and_sort(channels.iter().collect(), channel_prefs, None);
        if list.is_empty() {
            return Err(Unsupported("chat's filtered channels list empty state").into());
        }

        let title = escape(&self.t("chat.chat_channels"));
        let mut out = format!(
            "<div class=\"c-routes --channels\"><div class=\"c-navbar-container\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"{title}\"><span class=\"c-navbar__title-text\">{title}</span></div><nav class=\"c-navbar__actions\"><button class=\"btn no-text btn-icon btn-transparent chat-channel-list-options-button title-action\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button></nav></nav></div><div class=\"channels-list-container public-channels\" id=\"public-channels\">",
            escape(&self.t("chat.channel_list.options.title")),
            d_icon("ellipsis-vertical", None)
        );
        let emojis = settings.emojis().clone();
        let now = crate::clock::now();
        for channel in list {
            let Some(c) = public.iter().find(|c| c["id"].as_i64() == Some(channel.id)) else {
                continue;
            };
            out.push_str(&self.channel_row(c, channel, &emojis, now));
        }
        out.push_str("</div></div>");

        out.push_str(&self.footer("channels", &channels, &dms).await?);
        Ok(out)
    }

    /// ChatFooter in the mobile view, `route` the active item's id: shown
    /// with more than one of starred, threads (refused upstream while
    /// unported), DMs and public channels.
    async fn footer(
        &mut self,
        route: &str,
        channels: &[SidebarChannel],
        dms: &[SidebarChannel],
    ) -> Result<String, AppError> {
        let settings = self.settings;
        let mut out = String::new();
        let dms_enabled = !dms.is_empty()
            || super::user_can_direct_message(&mut *self.conn, settings, self.guardian).await?;
        let starred = channels.iter().chain(dms).any(|c| c.starred);
        let public = settings.get("enable_public_channels")?.truthy();
        let routes = [starred, dms_enabled, public]
            .iter()
            .filter(|x| **x)
            .count();
        if routes > 1 {
            out.push_str("<nav class=\"c-footer\">");
            let item = |id: &str,
                        active: bool,
                        aria: &str,
                        icon: &str,
                        label: &str,
                        href: &str,
                        indicator: String| {
                format!(
                    "<button aria-label=\"{}\" class=\"btn btn-icon-text btn-transparent c-footer__item{}\" id=\"c-footer-{id}\" type=\"button\" data-href=\"{}{href}\">{}<span class=\"d-button-label\">{}</span>{indicator}</button>",
                    escape(&self.t(aria)),
                    if active { " --active" } else { "" },
                    self.base_path,
                    d_icon(icon, None),
                    escape(&self.t(label))
                )
            };
            if starred {
                let urgent: i64 = channels
                    .iter()
                    .filter(|c| c.starred)
                    .map(|c| c.mentions)
                    .chain(dms.iter().filter(|c| c.starred).map(|c| c.unread))
                    .sum();
                let unread: i64 = channels
                    .iter()
                    .filter(|c| c.starred)
                    .map(|c| c.unread)
                    .sum();
                out.push_str(&item(
                    "starred",
                    route == "starred",
                    "chat.starred",
                    "star",
                    "chat.starred",
                    "/chat/starred-channels",
                    footer_indicator(urgent, unread),
                ));
            }
            let unstarred = channels.iter().filter(|c| !c.starred);
            out.push_str(&item(
                "channels",
                route == "channels",
                "chat.channel_list.aria_label",
                "comments",
                "chat.channel_list.title",
                "/chat/channels",
                footer_indicator(
                    unstarred.clone().map(|c| c.mentions).sum(),
                    unstarred.map(|c| c.unread).sum(),
                ),
            ));
            if dms_enabled {
                out.push_str(&item(
                    "direct-messages",
                    route == "direct-messages",
                    "chat.direct_messages.aria_label",
                    "users",
                    "chat.direct_messages.title",
                    "/chat/direct-messages",
                    footer_indicator(
                        dms.iter()
                            .filter(|c| !c.starred)
                            .map(|c| c.unread + c.mentions)
                            .sum(),
                        0,
                    ),
                ));
            }
            if settings.get("chat_search_enabled")?.truthy() {
                out.push_str(&item(
                    "search",
                    route == "search",
                    "chat.search.aria_label",
                    "magnifying-glass",
                    "chat.search.short_title",
                    "/chat/search",
                    String::new(),
                ));
            }
            out.push_str("</nav>");
        }
        Ok(out)
    }

    /// The preloaded channels (GET /chat/api/me/channels) as the lists
    /// read them: the public ones and the DMs, each with its JSON.
    async fn preloaded(&mut self) -> Result<Preloaded, AppError> {
        let me = self.me_channels().await?;
        let tracking = &me["tracking"]["channel_tracking"];
        let json = |key: &str| me[key].as_array().cloned().unwrap_or_default();
        let public_json = json("public_channels");
        let dm_json = json("direct_message_channels");
        let read = |list: &[Value]| -> Result<Vec<SidebarChannel>, AppError> {
            list.iter()
                .map(|c| SidebarChannel::from_json(c, tracking))
                .collect()
        };
        Ok(Preloaded {
            channels: read(&public_json)?,
            dms: read(&dm_json)?,
            dm_json,
        })
    }

    /// A route's navbar: its title and actions.
    fn route_navbar(&self, title: &str, actions: &str) -> String {
        let title = escape(title);
        format!(
            "<div class=\"c-navbar-container\"><nav class=\"c-navbar\"><div class=\"c-navbar__title\" title=\"{title}\"><span class=\"c-navbar__title-text\">{title}</span></div><nav class=\"c-navbar__actions\">{actions}</nav></nav></div>"
        )
    }

    /// The navbar's OpenDrawerButton, desktop only.
    fn open_drawer_button(&self) -> String {
        if crate::mobile::mobile_view() {
            return String::new();
        }
        format!(
            "<button class=\"btn no-text btn-icon c-navbar__open-drawer-button btn-transparent\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&self.t("chat.close_full_page")),
            d_icon("discourse-compress", None)
        )
    }

    /// ChatChannelListFilterToggle (shown while the section's filter isn't
    /// all) and ChatChannelListOptionsButton.
    fn list_option_buttons(&self, p: ListPrefs) -> String {
        let mut out = String::new();
        if p.filter != "all" {
            out.push_str(&format!(
                "<button class=\"btn no-text btn-icon btn-transparent chat-channel-list-filter-toggle title-action\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                escape(&self.t("chat.channel_list.empty.show_all")),
                d_icon("filter-circle-xmark", None)
            ));
        }
        out.push_str(&format!(
            "<button class=\"btn no-text btn-icon btn-transparent chat-channel-list-options-button title-action\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
            escape(&self.t("chat.channel_list.options.title")),
            d_icon("ellipsis-vertical", None)
        ));
        out
    }

    /// The direct messages route (ChatDirectMessagesRoute, drawing
    /// ChannelsListDirect): the navbar, the section's divider (on desktop,
    /// or in the mobile view without public channels), the member's DMs by
    /// their DM list preferences, and in the mobile view the chat footer.
    /// In the drawer (ChatDrawerRoutesDirectMessages): the drawer's navbar
    /// titled Chat, then while expanded the list and the footer.
    pub async fn direct_messages_route(
        &mut self,
        drawer: Option<&super::drawer::Nav<'_>>,
    ) -> Result<String, AppError> {
        let settings = self.settings;
        let Some(user_id) = self.guardian.user_id() else {
            return Err(Unsupported("chat's direct messages list without a member").into());
        };
        let Some(options) = super::options(&mut *self.conn, user_id).await? else {
            return Err(Unsupported("chat's direct messages list without chat options").into());
        };
        let Preloaded {
            channels,
            dms,
            dm_json,
        } = self.preloaded().await?;
        let mobile = crate::mobile::mobile_view();
        let can_create =
            super::user_can_direct_message(&mut *self.conn, settings, self.guardian).await?;
        // chatChannelsManager.displayPublicChannels
        let display_public = settings.get("enable_public_channels")?.truthy()
            && (!channels.is_empty()
                || self.guardian.is_staff()
                || super::has_joinable_public_channels(&mut *self.conn, settings, self.guardian)
                    .await?);
        let dm_prefs = prefs(
            options.chat_channel_list_filter_dms,
            options.chat_channel_list_sort_dms,
        );
        let title = self.t("chat.direct_messages.title");
        // ChatNavbarChannelListOptionsButton: the section's buttons go in
        // the navbar in the mobile view.
        let actions = if mobile {
            self.list_option_buttons(dm_prefs)
        } else {
            self.open_drawer_button()
        };
        let mut out = match drawer {
            Some(nav) => {
                let navbar = nav.navbar(&self.t("chat.heading"));
                if !nav.expanded {
                    return Ok(format!(
                        "<div class=\"c-drawer-routes --direct-messages\">{navbar}</div>"
                    ));
                }
                format!(
                    "<div class=\"c-drawer-routes --direct-messages\">{navbar}<div class=\"chat-drawer-content\">"
                )
            }
            None => format!(
                "<div class=\"c-routes --direct-messages\">{}",
                self.route_navbar(&title, &actions)
            ),
        };
        if (can_create || !dms.is_empty()) && (!mobile || !display_public) {
            out.push_str(&format!(
                "<div class=\"chat-channel-divider direct-message-channels-section\"><span class=\"channel-title\">{}</span>",
                escape(&title)
            ));
            if !mobile {
                out.push_str(&format!(
                    "<div class=\"chat-channel-divider__actions\">{}</div>",
                    self.list_option_buttons(dm_prefs)
                ));
            }
            out.push_str("</div>");
        }
        out.push_str(&format!(
            "<div class=\"channels-list-container direct-message-channels{}\" id=\"direct-message-channels\">",
            if dms.is_empty() {
                " center-empty-channels-list"
            } else {
                ""
            }
        ));
        if dms.is_empty() {
            // The CTA opens the new message modal (static/js/chat-new-message.js).
            let cta = if can_create {
                format!(
                    "<div class=\"empty-state__cta\"><button class=\"btn btn-primary\" type=\"button\"><span class=\"d-button-label\">{}</span></button></div>",
                    escape(&self.t("chat.no_direct_message_channels_cta"))
                )
            } else {
                String::new()
            };
            out.push_str(&format!(
                "<div class=\"empty-state__container --empty-channels-list --with-image\"><div class=\"empty-state\"><div class=\"empty-state__image\">{}</div><div class=\"empty-state__title\" data-test-title=\"\">{}</div>{cta}</div></div>",
                super::page::CHAT_ZERO,
                escape(&self.t("chat.no_direct_message_channels"))
            ));
        } else {
            // truncatedDirectMessageChannelsByPreference
            let mut list = filter_and_sort(dms.iter().collect(), dm_prefs, None);
            list.truncate(DIRECT_MESSAGE_CHANNELS_LIMIT);
            if list.is_empty() {
                return Err(Unsupported("chat's filtered direct messages empty state").into());
            }
            let online = self.online_ids().await?;
            let emojis = settings.emojis().clone();
            let now = crate::clock::now();
            for channel in list {
                let Some(c) = dm_json
                    .iter()
                    .find(|c| c["id"].as_i64() == Some(channel.id))
                else {
                    continue;
                };
                out.push_str(&self.dm_row(c, channel, &emojis, &online, now));
            }
        }
        out.push_str("</div></div>");
        if drawer.is_some() {
            out.push_str(&self.footer("direct-messages", &channels, &dms).await?);
            out.push_str("</div>");
        } else if mobile {
            out.push_str(&self.footer("direct-messages", &channels, &dms).await?);
        }
        Ok(out)
    }

    /// The drawer's channels route (ChatDrawerRoutesChannels): its navbar
    /// titled Chat, then while expanded ChannelsListPublic on desktop (the
    /// section's divider while public channels show, every followed public
    /// channel by the member's list preferences, or the empty state
    /// offering the browser) and the footer.
    pub async fn drawer_channels_route(
        &mut self,
        nav: &super::drawer::Nav<'_>,
    ) -> Result<String, AppError> {
        let settings = self.settings;
        let navbar = nav.navbar(&self.t("chat.heading"));
        if !nav.expanded {
            return Ok(format!(
                "<div class=\"c-drawer-routes --channels\">{navbar}</div>"
            ));
        }
        let Some(user_id) = self.guardian.user_id() else {
            return Err(Unsupported("chat's channels list without a member").into());
        };
        let Some(options) = super::options(&mut *self.conn, user_id).await? else {
            return Err(Unsupported("chat's channels list without chat options").into());
        };
        let me = self.me_channels().await?;
        let tracking = &me["tracking"]["channel_tracking"];
        let json = |key: &str| me[key].as_array().cloned().unwrap_or_default();
        let public = json("public_channels");
        let channels = public
            .iter()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<Vec<_>, _>>()?;
        let dms = json("direct_message_channels")
            .iter()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<Vec<_>, _>>()?;
        let channel_prefs = prefs(
            options.chat_channel_list_filter,
            options.chat_channel_list_sort,
        );
        // chatChannelsManager.displayPublicChannels
        let display_public = settings.get("enable_public_channels")?.truthy()
            && (!channels.is_empty()
                || self.guardian.is_staff()
                || super::has_joinable_public_channels(&mut *self.conn, settings, self.guardian)
                    .await?);
        let mut out = format!(
            "<div class=\"c-drawer-routes --channels\">{navbar}<div class=\"chat-drawer-content\">"
        );
        if display_public {
            out.push_str(&format!(
                "<div class=\"chat-channel-divider public-channels-section\"><span class=\"channel-title\">{}</span><div class=\"chat-channel-divider__actions\">{}</div></div>",
                escape(&self.t("chat.chat_channels")),
                self.list_option_buttons(channel_prefs)
            ));
        }
        out.push_str(
            "<div class=\"channels-list-container public-channels\" id=\"public-channels\">",
        );
        if channels.is_empty() {
            // DEmptyState, its button to the browser (openBrowseChannels).
            let cta = if display_public {
                format!(
                    "<div class=\"empty-state__cta\"><button class=\"btn btn-primary\" type=\"button\" data-href=\"{}/chat/browse\"><span class=\"d-button-label\">{}</span></button></div>",
                    self.base_path,
                    escape(&self.t("chat.no_public_channels_cta"))
                )
            } else {
                String::new()
            };
            out.push_str(&format!(
                "<div class=\"empty-state__container --empty-channels-list --with-image\"><div class=\"empty-state\"><div class=\"empty-state__image\">{}</div><div class=\"empty-state__title\" data-test-title=\"\">{}</div>{cta}</div></div>",
                super::page::CHAT_ZERO,
                escape(&self.t("chat.no_public_channels"))
            ));
        } else {
            let list = filter_and_sort(channels.iter().collect(), channel_prefs, None);
            if list.is_empty() {
                return Err(Unsupported("chat's filtered channels list empty state").into());
            }
            let emojis = settings.emojis().clone();
            let now = crate::clock::now();
            for channel in list {
                let Some(c) = public.iter().find(|c| c["id"].as_i64() == Some(channel.id)) else {
                    continue;
                };
                out.push_str(&self.channel_row(c, channel, &emojis, now));
            }
        }
        out.push_str("</div></div>");
        out.push_str(&self.footer("channels", &channels, &dms).await?);
        out.push_str("</div>");
        Ok(out)
    }

    /// Whether the member has a starred channel they follow
    /// (chatChannelsManager.hasStarredChannels).
    pub async fn has_starred_channels(&mut self) -> Result<bool, AppError> {
        let Preloaded { channels, dms, .. } = self.preloaded().await?;
        Ok(channels.iter().chain(&dms).any(|c| c.starred))
    }

    /// The drawer's starred channels route
    /// (ChatDrawerRoutesStarredChannels): its navbar, then while expanded
    /// ChannelsListStarred on desktop (the section's divider, the starred
    /// public channels by slug then DMs by title, by the member's starred
    /// list preferences, a DM with its leave button) and the footer.
    pub async fn drawer_starred_route(
        &mut self,
        nav: &super::drawer::Nav<'_>,
    ) -> Result<String, AppError> {
        let navbar = nav.navbar(&self.t("chat.starred"));
        if !nav.expanded {
            return Ok(format!(
                "<div class=\"c-drawer-routes --starred-channels\">{navbar}</div>"
            ));
        }
        let Some(user_id) = self.guardian.user_id() else {
            return Err(Unsupported("chat's starred channels without a member").into());
        };
        let Some(options) = super::options(&mut *self.conn, user_id).await? else {
            return Err(Unsupported("chat's starred channels without chat options").into());
        };
        let me = self.me_channels().await?;
        let tracking = &me["tracking"]["channel_tracking"];
        let json = |key: &str| me[key].as_array().cloned().unwrap_or_default();
        let public = json("public_channels");
        let dm_json = json("direct_message_channels");
        let channels = public
            .iter()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<Vec<_>, _>>()?;
        let dms = dm_json
            .iter()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<Vec<_>, _>>()?;
        let starred_prefs = prefs(
            options.chat_channel_list_filter_starred,
            options.chat_channel_list_sort_starred,
        );
        let mut starred: Vec<&SidebarChannel> = channels.iter().filter(|c| c.starred).collect();
        starred.sort_by(|a, b| a.slug.cmp(&b.slug));
        let mut starred_dms: Vec<&SidebarChannel> = dms.iter().filter(|c| c.starred).collect();
        starred_dms.sort_by(|a, b| a.title.cmp(&b.title));
        starred.extend(starred_dms);
        let mut list = filter(starred.clone(), starred_prefs, None);
        if starred_prefs.sort != "alphabetical" {
            sort(&mut list, starred_prefs);
        }
        if list.is_empty() {
            return Err(Unsupported("chat's filtered starred channels empty state").into());
        }
        let mut out = format!(
            "<div class=\"c-drawer-routes --starred-channels\">{navbar}<div class=\"chat-drawer-content\"><div class=\"chat-channel-divider starred-channels-section\"><span class=\"channel-title\">{}</span><div class=\"chat-channel-divider__actions\">{}</div></div><div class=\"channels-list-container starred-channels\">",
            escape(&self.t("chat.starred_channels")),
            self.list_option_buttons(starred_prefs)
        );
        let emojis = self.settings.emojis().clone();
        let now = crate::clock::now();
        let online = if list.iter().any(|c| c.dm.is_some()) {
            self.online_ids().await?
        } else {
            Vec::new()
        };
        for channel in list {
            if channel.dm.is_some() {
                if let Some(c) = dm_json
                    .iter()
                    .find(|c| c["id"].as_i64() == Some(channel.id))
                {
                    out.push_str(&self.dm_row(c, channel, &emojis, &online, now));
                }
            } else if let Some(c) = public.iter().find(|c| c["id"].as_i64() == Some(channel.id)) {
                out.push_str(&self.channel_row(c, channel, &emojis, now));
            }
        }
        out.push_str("</div></div>");
        out.push_str(&self.footer("starred", &channels, &dms).await?);
        out.push_str("</div>");
        Ok(out)
    }

    /// The threads route (ChatRoutesThreads, drawing UserThreads): the
    /// navbar and the member's threads once GET /chat/api/me/threads
    /// answers (UserThreads), with none the list's empty state; in the
    /// mobile view the chat footer.
    pub async fn threads_route(&mut self) -> Result<String, AppError> {
        let Preloaded { channels, dms, .. } = self.preloaded().await?;
        let mut out = format!(
            "<div class=\"c-routes --threads\">{}",
            self.route_navbar(&self.t("chat.my_threads.title"), &self.open_drawer_button())
        );
        out.push_str(&self.user_threads_list().await?);
        out.push_str("</div>");
        if crate::mobile::mobile_view() {
            out.push_str(&self.footer("threads", &channels, &dms).await?);
        }
        Ok(out)
    }

    /// ChatChannelRow for a DM in the direct messages list: its icon, name
    /// with the unread indicator, the last message's date, and the leave
    /// button on desktop or the last message's excerpt in the mobile view.
    fn dm_row(
        &self,
        c: &Value,
        channel: &SidebarChannel,
        emojis: &crate::emojis::Emojis,
        online: &[i64],
        now: chrono::DateTime<chrono::Utc>,
    ) -> String {
        // ChatChannelUnreadIndicator: a DM's unread messages are urgent.
        let urgent = channel.unread + channel.mentions + channel.watched_threads;
        let indicator = if channel.unread + channel.mentions > 0 {
            format!(
                "<div class=\"chat-channel-unread-indicator -urgent\"><div class=\"chat-channel-unread-indicator__number\">{}</div></div>",
                if urgent > 99 {
                    "99+".to_string()
                } else {
                    urgent.to_string()
                }
            )
        } else {
            String::new()
        };
        let title = c["unicode_title"]
            .as_str()
            .or(c["title"].as_str())
            .unwrap_or(&channel.title);
        let mobile = crate::mobile::mobile_view();
        let tail = if mobile {
            if c["last_message"].is_object() {
                format!(
                    "<div class=\"chat-channel__last-message\">{}</div>",
                    crate::topic_list_view::emoji_unescape(
                        c["last_message"]["excerpt"].as_str().unwrap_or_default(),
                        emojis,
                        self.base_path
                    )
                )
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let leave = if mobile {
            String::new()
        } else {
            format!(
                "<button class=\"btn no-text btn-icon toggle-channel-membership-button -leave btn-flat chat-channel-leave-btn\" title=\"{}\" type=\"button\">{}<span aria-hidden=\"true\">&#8203;</span></button>",
                escape(&self.t("chat.direct_messages.close")),
                d_icon("xmark", None)
            )
        };
        format!(
            "<a class=\"chat-channel-row{} can-leave{}\" tabindex=\"0\" data-chat-channel-id=\"{id}\" href=\"{}/chat/c/{}/{id}\"><div class=\"chat-channel-row__content is-dm\">{}<div class=\"chat-channel-row__info\"><div class=\"chat-channel-row__name-container\"><div class=\"chat-channel-name\"><div class=\"chat-channel-name__label\">{}</div>{indicator}</div></div><div class=\"chat-channel__metadata\">{}</div>{tail}</div>{leave}</div></a>",
            if channel.muted { " muted" } else { "" },
            if channel.unread > 0 {
                " has-unread"
            } else {
                ""
            },
            self.base_path,
            escape(&channel.slug),
            super::page::channel_icon(c, emojis, self.base_path, online),
            crate::topic_list_view::emoji_unescape(&escape(title), emojis, self.base_path),
            self.metadata_date(c, now),
            id = channel.id,
        )
    }

    /// ChatChannelMetadata's date: the last message's, drawn in UTC here
    /// and in the member's zone by chat.js; a dash without one.
    fn metadata_date(&self, c: &Value, now: chrono::DateTime<chrono::Utc>) -> String {
        if c["last_message"]["id"].as_i64().is_none() {
            return "<div class=\"chat-channel__metadata-date\">&ndash;</div>".to_string();
        }
        let at = c["last_message"]["created_at"].as_str().unwrap_or_default();
        let yesterday = self.t("chat.dates.yesterday");
        let text = chrono::DateTime::parse_from_rfc3339(at)
            .map(|at| channel_calendar(at.with_timezone(&chrono::Utc), now, &yesterday))
            .unwrap_or_default();
        format!(
            "<div class=\"chat-channel__metadata-date\" data-at=\"{}\" data-label-yesterday=\"{}\">{}</div>",
            escape(at),
            escape(&yesterday),
            escape(&text)
        )
    }

    /// ChatChannelRow for a public channel in the mobile list: its icon,
    /// name with the unread indicator, and the last message's date
    /// (ChatChannelMetadata, drawn in UTC here and in the member's zone by
    /// chat.js).
    fn channel_row(
        &self,
        c: &Value,
        channel: &SidebarChannel,
        emojis: &crate::emojis::Emojis,
        now: chrono::DateTime<chrono::Utc>,
    ) -> String {
        let has_unread = channel.unread > 0;
        let urgent = channel.mentions + channel.watched_threads;
        let indicator = if channel.unread + channel.mentions > 0 {
            format!(
                "<div class=\"chat-channel-unread-indicator{}\"><div class=\"chat-channel-unread-indicator__number\">{}</div></div>",
                if urgent > 0 { " -urgent" } else { "" },
                match urgent {
                    0 => "&nbsp;".to_string(),
                    n if n > 99 => "99+".to_string(),
                    n => n.to_string(),
                }
            )
        } else {
            String::new()
        };
        let date = self.metadata_date(c, now);
        format!(
            "<a class=\"chat-channel-row{}{}\" tabindex=\"0\" data-chat-channel-id=\"{id}\" href=\"{}/chat/c/{}/{id}\"><div class=\"chat-channel-row__content is-category\">{}<div class=\"chat-channel-row__info\"><div class=\"chat-channel-row__name-container\"><div class=\"chat-channel-name\"><div class=\"chat-channel-name__label\">{}</div>{indicator}</div></div><div class=\"chat-channel__metadata\">{date}</div></div></div></a>",
            if channel.muted { " muted" } else { "" },
            if has_unread { " has-unread" } else { "" },
            self.base_path,
            escape(&channel.slug),
            super::page::channel_icon(c, emojis, self.base_path, &[]),
            escape(c["unicode_title"].as_str().unwrap_or(&channel.title)),
            id = channel.id,
        )
    }

    fn t(&self, key: &str) -> String {
        self.i18n.t(&format!("js.{key}")).unwrap_or(key).to_string()
    }

    /// ChatHeaderIcon, in "never" mode: Chat, to /chat, active on full-page
    /// chat (where its unread indicator is hidden).
    fn header_icon(
        &self,
        channels: &[SidebarChannel],
        dms: &[SidebarChannel],
        options: &super::ChatOptions,
        full_page: bool,
    ) -> Result<String, AppError> {
        // The mobile view's full-page chat has its own header (the chat
        // header, ChatHeader), without the icon.
        if full_page && crate::mobile::mobile_view() {
            return Ok(String::new());
        }
        let indicator = if full_page {
            String::new()
        } else {
            unread_indicator(channels, dms, options, None)
        };
        Ok(format!(
            "<li class=\"header-dropdown-toggle chat-header-icon\"><a class=\"btn no-text icon btn-flat{}\" href=\"{}/chat\" tabindex=\"0\" title=\"{}\">{}{indicator}</a></li>",
            if full_page { " active" } else { "" },
            self.base_path,
            escape(&self.t("chat.title_capitalized")),
            d_icon("d-chat", None)
        ))
    }

    /// The sections chat-sidebar.js adds to the main panel.
    async fn sidebar_sections(
        &mut self,
        channels: &[SidebarChannel],
        dms: &[SidebarChannel],
        options: &super::ChatOptions,
        active: &Active,
        cx: &crate::sidebar::Context<'_>,
    ) -> Result<String, AppError> {
        let settings = self.settings;
        let g = self.guardian;
        let active_id = active_channel(active);
        let mut out = String::new();

        if settings.get("chat_search_enabled")?.truthy() {
            let link = Link {
                link_name: Some("chat-search".into()),
                attributes: String::new(),
                plain: false,
                href: format!("{}/chat/search", self.base_path),
                title: Some(self.t("chat.search.title")),
                content: escape(&self.t("chat.search.title")),
                prefix: Prefix::Icon {
                    name: "magnifying-glass".into(),
                    color: None,
                },
                prefix_badge: None,
                active: false,
                badge: None,
                suffix: None,
            };
            out.push_str(&crate::sidebar::section_html_with(
                "chat-search",
                None,
                "",
                &crate::sidebar::link_html(&link, cx),
                cx,
            ));
        }

        // Presence on /chat/online marks a DM's avatar active.
        let online = if dms.is_empty() {
            Vec::new()
        } else {
            self.online_ids().await?
        };

        // Starred channels: public ones by slug, then DMs by title; left
        // in that order when sorted alphabetically.
        let mut starred: Vec<&SidebarChannel> = channels.iter().filter(|c| c.starred).collect();
        starred.sort_by(|a, b| a.slug.cmp(&b.slug));
        let mut starred_dms: Vec<&SidebarChannel> = dms.iter().filter(|c| c.starred).collect();
        starred_dms.sort_by(|a, b| a.title.cmp(&b.title));
        starred.extend(starred_dms);
        let starred_prefs = prefs(
            options.chat_channel_list_filter_starred,
            options.chat_channel_list_sort_starred,
        );
        let mut starred_links = filter(starred.clone(), starred_prefs, active_id);
        if starred_prefs.sort != "alphabetical" {
            sort(&mut starred_links, starred_prefs);
        }
        if !starred_links.is_empty() || (!starred.is_empty() && starred_prefs.filter != "all") {
            out.push_str(&self.channels_section(
                "chat-starred-channels",
                &self.t("chat.starred_channels"),
                &starred_links,
                starred_prefs,
                active_id,
                &online,
                cx,
            ));
        }

        if settings.get("enable_public_channels")?.truthy() {
            let unstarred: Vec<&SidebarChannel> = channels.iter().filter(|c| !c.starred).collect();
            let channel_prefs = prefs(
                options.chat_channel_list_filter,
                options.chat_channel_list_sort,
            );
            let can_join = g.is_staff()
                || super::has_joinable_public_channels(&mut *self.conn, settings, g).await?;
            if !unstarred.is_empty() || can_join || channel_prefs.filter != "all" {
                let links = filter_and_sort(unstarred, channel_prefs, active_id);
                out.push_str(&self.channels_section(
                    "chat-channels",
                    &self.t("chat.chat_channels"),
                    &links,
                    channel_prefs,
                    active_id,
                    &online,
                    cx,
                ));
            }
        }

        // Direct messages: the unstarred ones by activity, filtered and
        // sorted by the member's DM list preferences, fifty at most (the
        // active one kept); with none, the header hides and the section
        // offers a new one.
        if super::can_direct_message(settings, g)? {
            let mut unstarred: Vec<&SidebarChannel> = dms.iter().filter(|c| !c.starred).collect();
            unstarred.sort_by(|a, b| dm_activity(a, b));
            let dm_prefs = prefs(
                options.chat_channel_list_filter_dms,
                options.chat_channel_list_sort_dms,
            );
            let mut links = filter_and_sort(unstarred.clone(), dm_prefs, active_id);
            if links.len() > DIRECT_MESSAGE_CHANNELS_LIMIT {
                let active = links.iter().copied().find(|c| Some(c.id) == active_id);
                links.truncate(DIRECT_MESSAGE_CHANNELS_LIMIT);
                if let Some(active) = active
                    && !links.iter().any(|c| c.id == active.id)
                {
                    links.pop();
                    links.push(active);
                }
            }
            if !links.is_empty() {
                out.push_str(&self.channels_section(
                    "chat-dms",
                    &self.t("chat.direct_messages.title"),
                    &links,
                    dm_prefs,
                    active_id,
                    &online,
                    cx,
                ));
            } else if unstarred.is_empty() {
                let title = self.t("sidebar.start_new_dm.title");
                let link = Link {
                    link_name: Some("new-chat-dm".into()),
                    attributes: String::new(),
                    plain: false,
                    href: format!("{}/chat/new-message", self.base_path),
                    title: Some(title),
                    content: escape(&self.t("sidebar.start_new_dm.text")),
                    prefix: Prefix::Icon {
                        name: "plus".into(),
                        color: None,
                    },
                    prefix_badge: None,
                    active: false,
                    badge: None,
                    suffix: None,
                };
                out.push_str(&crate::sidebar::section_html_with(
                    "chat-dms",
                    None,
                    "",
                    &crate::sidebar::link_html(&link, cx),
                    cx,
                ));
            } else if dm_prefs.filter != "all" {
                out.push_str(&self.channels_section(
                    "chat-dms",
                    &self.t("chat.direct_messages.title"),
                    &[],
                    dm_prefs,
                    active_id,
                    &online,
                    cx,
                ));
            }
        }
        Ok(out)
    }

    /// A section of channel links with its list options button (and the
    /// show-all toggle when a filter leaves it empty).
    #[allow(clippy::too_many_arguments)]
    fn channels_section(
        &self,
        name: &str,
        header: &str,
        channels: &[&SidebarChannel],
        p: ListPrefs,
        active: Option<i64>,
        online: &[i64],
        cx: &crate::sidebar::Context<'_>,
    ) -> String {
        let mut buttons = String::new();
        if p.filter != "all" && channels.is_empty() {
            let title = self.t("chat.channel_list.empty.show_all");
            buttons.push_str(&format!(
                "<button aria-label=\"{t}\" class=\"sidebar-section-header-button btn-icon btn-flat\" data-sidebar-action-id=\"toggleChannelFilter\" title=\"{t}\" type=\"button\">{}</button>",
                d_icon("filter-circle-xmark", None),
                t = escape(&title)
            ));
        }
        let title = self.t("chat.channel_list.options.title");
        buttons.push_str(&format!(
            "<button aria-label=\"{t}\" class=\"sidebar-section-header-button btn-icon btn-flat\" data-sidebar-action-id=\"channelListOptions\" title=\"{t}\" type=\"button\">{}</button>",
            d_icon("ellipsis-vertical", None),
            t = escape(&title)
        ));
        let links = self.channel_links(channels, active, online, cx);
        crate::sidebar::section_html_with(name, Some(header), &buttons, &links, cx)
    }

    /// The channels' links: a category channel's, or a DM's
    /// (SidebarChatDirectMessagesSectionLink).
    fn channel_links(
        &self,
        channels: &[&SidebarChannel],
        active: Option<i64>,
        online: &[i64],
        cx: &crate::sidebar::Context<'_>,
    ) -> String {
        let list = crate::topic_list_view::ListSettings::load(self.settings).ok();
        let tiny = raw_avatar_size(self.settings, 20);
        let mut links = String::new();
        for c in channels {
            let is_active = Some(c.id) == active;
            let mut classes = Vec::new();
            if c.muted {
                classes.push("sidebar-section-link--muted".to_string());
            }
            if is_active {
                classes.push("sidebar-section-link--active".into());
            }
            classes.push(format!("channel-{}", c.id));
            let link = match &c.dm {
                Some(dm) => self.dm_link(c, dm, is_active, online, tiny),
                None => self.category_link(c, is_active, list.as_ref()),
            };
            // ChatSidebarIndicators: urgent for mentions, watched threads
            // and any unread in a DM.
            let suffix_html = if c.unread_total() > 0 {
                let urgent = c.urgent() > 0 || (c.dm.is_some() && c.unread > 0);
                format!(
                    "<span class=\"sidebar-section-link-content-badge icon {}\">{}</span>",
                    if urgent { "urgent" } else { "unread" },
                    d_icon("circle", None)
                )
            } else {
                String::new()
            };
            let extra = LinkExtra {
                classes: classes.join(" "),
                suffix_html,
                hover: Some(("ellipsis-vertical", self.t("chat.open_channel_menu"))),
            };
            links.push_str(&crate::sidebar::link_html_with(&link, &extra, cx));
        }
        links
    }

    /// A category channel's link.
    fn category_link(
        &self,
        c: &SidebarChannel,
        is_active: bool,
        list: Option<&crate::topic_list_view::ListSettings>,
    ) -> Link {
        let text = match list {
            Some(l) => {
                crate::topic_list_view::emoji_unescape(&escape(&c.title), &l.emojis, self.base_path)
            }
            None => escape(&c.title),
        };
        Link {
            link_name: Some(c.slug.clone()),
            attributes: String::new(),
            plain: false,
            href: format!("{}/chat/c/{}/{}", self.base_path, c.slug, c.id),
            title: Some(match &c.description {
                Some(d) => d.clone(),
                None => format!("{} {}", c.title, self.t("chat.title")),
            }),
            content: text,
            prefix: match &c.emoji {
                Some(emoji) => Prefix::Emoji {
                    name: emoji.clone(),
                    color: c.color.as_deref().map(|c| format!("#{c}")),
                },
                None => Prefix::Icon {
                    name: "d-chat".into(),
                    color: c.color.as_deref().map(|c| format!("#{c}")),
                },
            },
            prefix_badge: c.read_restricted.then_some("lock"),
            active: is_active,
            badge: None,
            suffix: None,
        }
    }

    /// A DM's link: "Chat in" a group or "Chat with" the users; a 1:1's
    /// text is its title without "@"; an avatar prefix (tiny) for a 1:1
    /// or a group of one, else the member count, an emoji when set,
    /// active when the first user is on /chat/online.
    fn dm_link(
        &self,
        c: &SidebarChannel,
        dm: &DmLink,
        is_active: bool,
        online: &[i64],
        tiny: i64,
    ) -> Link {
        let escaped_title = escape(&c.title);
        let title = if dm.group {
            self.i18n.t_with(
                "js.chat.placeholder_channel",
                &[("channelName", &escaped_title)],
            )
        } else {
            self.i18n.t_with(
                "js.chat.placeholder_users",
                &[("commaSeparatedNames", &escaped_title)],
            )
        }
        .unwrap_or_else(|| escaped_title.clone());
        let content = if dm.group {
            escape(dm.unicode_title.as_deref().unwrap_or(&c.title))
        } else {
            escape(&escaped_title.replace('@', ""))
        };
        let first = dm.users.first();
        let class = first
            .and_then(|u| u["id"].as_i64())
            .filter(|id| online.contains(id))
            .map(|_| "active".to_string());
        let avatar = |u: &Value| {
            u["avatar_template"]
                .as_str()
                .unwrap_or_default()
                .replace("{size}", &tiny.to_string())
        };
        let prefix = match (&c.emoji, dm.group, first) {
            (Some(emoji), _, _) => Prefix::Emoji {
                name: emoji.clone(),
                color: None,
            },
            (None, true, Some(u)) if dm.users.len() == 1 => Prefix::Image {
                src: avatar(u),
                class,
            },
            (None, true, _) => Prefix::Text {
                text: dm.memberships_count.to_string(),
                class,
            },
            (None, false, Some(u)) => Prefix::Image {
                src: avatar(u),
                class,
            },
            (None, false, None) => Prefix::None,
        };
        Link {
            link_name: Some(c.slug.clone()),
            attributes: String::new(),
            plain: false,
            href: format!("{}/chat/c/{}/{}", self.base_path, c.slug, c.id),
            title: Some(title),
            content,
            prefix,
            prefix_badge: None,
            active: is_active,
            badge: None,
            suffix: None,
        }
    }
}

/// `getRawAvatarSize(size)` at a device pixel ratio of 1: the first of
/// avatar_sizes at least that size, else the largest.
fn raw_avatar_size(settings: &crate::site_settings::SiteSettings, size: i64) -> i64 {
    let mut sizes: Vec<i64> = settings
        .get("avatar_sizes")
        .map(|v| v.to_s().to_string())
        .unwrap_or_default()
        .split('|')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    sizes.sort_unstable();
    sizes
        .iter()
        .copied()
        .find(|s| *s >= size)
        .or(sizes.last().copied())
        .unwrap_or(size)
}

/// `DIRECT_MESSAGE_CHANNELS_LIMIT`
const DIRECT_MESSAGE_CHANNELS_LIMIT: usize = 50;

/// ChatHeaderIconUnreadIndicator, without threads: a DM's unread messages
/// are urgent; the unread dot counts public channels only. `exclude`
/// leaves a channel out (the channel route's back button counts the other
/// channels).
fn unread_indicator(
    channels: &[SidebarChannel],
    dms: &[SidebarChannel],
    options: &super::ChatOptions,
    exclude: Option<i64>,
) -> String {
    let others = |c: &&SidebarChannel| Some(c.id) != exclude;
    let preference = super::HEADER_INDICATORS
        .get(usize::try_from(options.chat_header_indicator_preference).unwrap_or(0))
        .copied()
        .unwrap_or("all_new");
    let urgent: i64 = channels
        .iter()
        .filter(others)
        .map(SidebarChannel::urgent)
        .sum::<i64>()
        + dms
            .iter()
            .filter(others)
            .map(|c| c.urgent() + c.unread)
            .sum::<i64>();
    let mentions: i64 = channels
        .iter()
        .chain(dms)
        .filter(others)
        .map(|c| c.mentions)
        .sum();
    let unread: i64 = channels.iter().filter(others).map(|c| c.unread).sum();
    let label = |n: i64| {
        if n > 99 {
            "99+".to_string()
        } else {
            n.to_string()
        }
    };
    if preference == "only_mentions" {
        if mentions > 0 {
            return urgent_indicator(&label(mentions));
        }
    } else if preference != "never" {
        if urgent > 0 && (preference == "all_new" || preference == "dm_and_mentions") {
            return urgent_indicator(&label(urgent));
        } else if unread > 0 && preference == "all_new" {
            return "<div class=\"chat-channel-unread-indicator\"></div>".into();
        }
    }
    String::new()
}

/// FooterUnreadIndicator: the urgent count, else a dot for unread ones.
fn footer_indicator(urgent: i64, unread: i64) -> String {
    if urgent > 0 {
        let label = if urgent > 99 {
            "99+".to_string()
        } else {
            urgent.to_string()
        };
        format!(
            "<div class=\"c-unread-indicator -urgent\"><div class=\"c-unread-indicator__number\">{label}</div></div>"
        )
    } else if unread > 0 {
        "<div class=\"c-unread-indicator\"></div>".into()
    } else {
        String::new()
    }
}

/// ChatChannelMetadata's `moment(at).calendar` with sameDay "LT", lastDay
/// Yesterday, lastWeek "dddd" and sameElse "l", in English.
fn channel_calendar(
    at: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
    yesterday: &str,
) -> String {
    let days = (at.date_naive() - now.date_naive()).num_days();
    match days {
        0 => at.format("%-I:%M %p").to_string(),
        -1 => yesterday.to_string(),
        -6..=-2 => at.format("%A").to_string(),
        _ => at.format("%-m/%-d/%Y").to_string(),
    }
}

fn urgent_indicator(label: &str) -> String {
    format!(
        "<div class=\"chat-channel-unread-indicator -urgent\"><div class=\"chat-channel-unread-indicator__number\">{}</div></div>",
        escape(label)
    )
}

/// Chat's chrome for a page, built with its own connection use.
pub async fn load(
    conn: &mut PgConnection,
    state: &crate::AppState,
    settings: &crate::site_settings::SiteSettings,
    guardian: &crate::guardian::Guardian,
    base_path: &str,
    active: &Active,
    sidebar: Option<&crate::sidebar::Context<'_>>,
) -> Result<Option<ChatChrome>, AppError> {
    Context {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        base_path,
        config: &state.config,
    }
    .chrome(active, sidebar)
    .await
}
