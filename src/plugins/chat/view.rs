//! Chat's pieces of every page's chrome, as the plugin's initializers add
//! them: the header's chat icon (ChatHeaderIcon with its unread
//! indicator) and the sidebar's sections (chat-sidebar.js: search, starred
//! channels, channels, direct messages), from the channels
//! GET /chat/api/me/channels gives (the preloaded channels).
//!
//! The sections' menus (channel list options, a channel's menu) are drawn
//! as their buttons. Direct message channels and threads are refused
//! upstream (channels.rs) until their slices, and only the "never"
//! separate sidebar mode (chat's sections in the main sidebar) is drawn.

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

/// A followed channel as the sidebar reads it.
struct SidebarChannel {
    id: i64,
    title: String,
    slug: String,
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
            // slugifiedTitle: the slug; a channel without one is not drawn.
            slug: c["slug"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(Unsupported("chat channels without a slug"))?
                .to_string(),
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
    let alphabetical =
        |a: &SidebarChannel, b: &SidebarChannel| a.slug.cmp(&b.slug).then(a.id.cmp(&b.id));
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
    channels
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
        let channels: Vec<SidebarChannel> = me["public_channels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| SidebarChannel::from_json(c, tracking))
            .collect::<Result<_, _>>()?;
        let full_page = matches!(active, Active::Chat | Active::ChatChannel(_));

        let header_icon = self.header_icon(&channels, &options, full_page)?;
        let sidebar_sections = match sidebar {
            Some(cx) => {
                self.sidebar_sections(&channels, &options, active, cx)
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
        options: &super::ChatOptions,
        full_page: bool,
    ) -> Result<String, AppError> {
        let mut indicator = String::new();
        if !full_page {
            // ChatHeaderIconUnreadIndicator; no direct messages or threads.
            let preference = super::HEADER_INDICATORS
                .get(usize::try_from(options.chat_header_indicator_preference).unwrap_or(0))
                .copied()
                .unwrap_or("all_new");
            let urgent: i64 = channels.iter().map(SidebarChannel::urgent).sum();
            let mentions: i64 = channels.iter().map(|c| c.mentions).sum();
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

        // Starred channels.
        let starred: Vec<&SidebarChannel> = channels.iter().filter(|c| c.starred).collect();
        let starred_prefs = prefs(
            options.chat_channel_list_filter_starred,
            options.chat_channel_list_sort_starred,
        );
        let starred_links = filter_and_sort(starred.clone(), starred_prefs, active_id);
        if !starred_links.is_empty() || (!starred.is_empty() && starred_prefs.filter != "all") {
            out.push_str(&self.channels_section(
                "chat-starred-channels",
                &self.t("chat.starred_channels"),
                &starred_links,
                starred_prefs,
                active_id,
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
                    cx,
                ));
            }
        }

        // Direct messages: none yet (refused upstream), so the header is
        // hidden and the section offers a new one.
        if super::can_direct_message(settings, g)? {
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
        }
        Ok(out)
    }

    /// A section of channel links with its list options button (and the
    /// show-all toggle when a filter leaves it empty).
    fn channels_section(
        &self,
        name: &str,
        header: &str,
        channels: &[&SidebarChannel],
        p: ListPrefs,
        active: Option<i64>,
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
        let list = crate::topic_list_view::ListSettings::load(self.settings).ok();
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
            let text = match &list {
                Some(l) => {
                    crate::topic_list_view::emoji_unescape(&escape(&c.title), l, self.base_path)
                }
                None => escape(&c.title),
            };
            let link = Link {
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
            };
            // ChatSidebarIndicators
            let suffix_html = if c.unread_total() > 0 {
                format!(
                    "<span class=\"sidebar-section-link-content-badge icon {}\">{}</span>",
                    if c.urgent() > 0 { "urgent" } else { "unread" },
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
        crate::sidebar::section_html_with(name, Some(header), &buttons, &links, cx)
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
