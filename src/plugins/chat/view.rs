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
}

/// A direct message channel's part of its sidebar link.
struct DmLink {
    group: bool,
    /// The chatable's users (the viewer left out unless alone).
    users: Vec<Value>,
    memberships_count: i64,
    unicode_title: Option<String>,
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
        Ok(Some(ChatChrome {
            header_icon,
            sidebar_sections,
        }))
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
        let mut indicator = String::new();
        if !full_page {
            // ChatHeaderIconUnreadIndicator, without threads: a DM's
            // unread messages are urgent; the unread dot counts public
            // channels only.
            let preference = super::HEADER_INDICATORS
                .get(usize::try_from(options.chat_header_indicator_preference).unwrap_or(0))
                .copied()
                .unwrap_or("all_new");
            let urgent: i64 = channels.iter().map(SidebarChannel::urgent).sum::<i64>()
                + dms.iter().map(|c| c.urgent() + c.unread).sum::<i64>();
            let mentions: i64 = channels.iter().chain(dms).map(|c| c.mentions).sum();
            let unread: i64 = channels.iter().map(|c| c.unread).sum();
            let label = |n: i64| {
                if n > 99 {
                    "99+".to_string()
                } else {
                    n.to_string()
                }
            };
            if preference == "only_mentions" {
                if mentions > 0 {
                    indicator = urgent_indicator(&label(mentions));
                }
            } else if preference != "never" {
                if urgent > 0 && (preference == "all_new" || preference == "dm_and_mentions") {
                    indicator = urgent_indicator(&label(urgent));
                } else if unread > 0 && preference == "all_new" {
                    indicator = "<div class=\"chat-channel-unread-indicator\"></div>".into();
                }
            }
        }
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
            Some(l) => crate::topic_list_view::emoji_unescape(&escape(&c.title), l, self.base_path),
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
