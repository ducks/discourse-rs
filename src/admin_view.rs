//! The admin shell as the Ember client renders it around every admin page:
//! AdminSidebarPanel (lib/sidebar/admin-sidebar.js) with its sections from
//! ADMIN_NAV_MAP (admin-nav-map.js) as AdminNavManager filters them for
//! moderators, the panel header (Back to Forum, the toggle for every
//! section, the admin search or, for moderators, the links' filter),
//! templates/admin.gjs's wrapper, and DPageHeader with DBreadcrumbsItem.
//!
//! The admin search the panel's search opens is crate::admin_search; the
//! design wizard its Appearance section's button launches (90f) is not
//! ported.

use serde_json::Value;

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::topic_list_view::{escape, icon};

/// What the shell reads of the current user (the preloaded currentUser).
#[derive(Clone, Debug, Default)]
pub struct AdminViewer {
    pub admin: bool,
    pub reviewable_count: i64,
    pub has_unseen_features: bool,
    pub has_new_upcoming_changes: bool,
    pub can_run_design_wizard: bool,
    pub id: i64,
    pub staff: bool,
    pub can_invite_to_forum: bool,
    pub can_see_ip: bool,
    pub can_see_emails: bool,
}

impl AdminViewer {
    /// From `/session/current.json`'s `current_user`.
    pub fn from_current_user(user: &Value) -> AdminViewer {
        let flag = |key: &str| user[key].as_bool().unwrap_or(false);
        AdminViewer {
            admin: flag("admin"),
            reviewable_count: user["reviewable_count"].as_i64().unwrap_or(0),
            has_unseen_features: flag("has_unseen_features"),
            has_new_upcoming_changes: flag("has_new_upcoming_changes"),
            can_run_design_wizard: flag("can_run_design_wizard"),
            id: user["id"].as_i64().unwrap_or(0),
            staff: flag("staff"),
            can_invite_to_forum: flag("can_invite_to_forum"),
            can_see_ip: flag("can_see_ip"),
            can_see_emails: flag("can_see_emails"),
        }
    }
}

pub struct AdminCx<'a> {
    pub i18n: &'a I18n,
    pub settings: &'a SiteSettings,
    pub base_path: &'a str,
    pub viewer: &'a AdminViewer,
}

/// `i18n(key)` as staff see it: the client strings with the admin bundle
/// (admin_js) merged in; I18n.js's `[en.key]` for a missing one.
pub fn t(i18n: &I18n, key: &str) -> String {
    t_opt(i18n, key).unwrap_or_else(|| format!("[en.{key}]"))
}

pub fn t_opt(i18n: &I18n, key: &str) -> Option<String> {
    i18n.t(&format!("js.{key}"))
        .or_else(|| i18n.t(&format!("admin_js.{key}")))
        .map(str::to_string)
}

/// `i18n(key, { name: value, ... })`
pub fn t_with(i18n: &I18n, key: &str, args: &[(&str, &str)]) -> String {
    i18n.t_with(&format!("js.{key}"), args)
        .or_else(|| i18n.t_with(&format!("admin_js.{key}"), args))
        .unwrap_or_else(|| format!("[en.{key}]"))
}

/// `i18n(key, { count, ... })`
pub fn t_count(i18n: &I18n, key: &str, count: i64, args: &[(&str, &str)]) -> String {
    i18n.t_count(&format!("js.{key}"), count, args)
        .or_else(|| i18n.t_count(&format!("admin_js.{key}"), count, args))
        .unwrap_or_else(|| format!("[en.{key}]"))
}

/// A link of the nav map.
struct NavLink {
    name: &'static str,
    path: &'static str,
    label: &'static str,
    icon: &'static str,
    moderator: bool,
}

const fn link(
    name: &'static str,
    path: &'static str,
    label: &'static str,
    icon: &'static str,
    moderator: bool,
) -> NavLink {
    NavLink {
        name,
        path,
        label,
        icon,
        moderator,
    }
}

/// A section of the nav map: its name, its header's label (none for the
/// root section, whose header is hidden), and its links.
struct NavSection {
    name: &'static str,
    label: Option<&'static str>,
    links: &'static [NavLink],
}

/// ADMIN_NAV_MAP's links with the paths their routes resolve to (the sub
/// links, which only the admin search lists, are crate::admin_search's).
const NAV_MAP: &[NavSection] = &[
    NavSection {
        name: "root",
        label: None,
        links: &[
            link(
                "admin_home",
                "/admin",
                "admin.config.dashboard.title",
                "house",
                true,
            ),
            link(
                "admin_users",
                "/admin/users",
                "admin.config.users.title",
                "users",
                true,
            ),
            link(
                "admin_groups",
                "/admin/groups",
                "admin.config.groups.title",
                "user-group",
                true,
            ),
            link(
                "admin_all_site_settings",
                "/admin/site_settings/category/all_results",
                "admin.config.site_settings.title",
                "gear",
                false,
            ),
            link(
                "admin_review",
                "/review",
                "sidebar.sections.community.links.review.content",
                "flag",
                true,
            ),
            link(
                "admin_whats_new",
                "/admin/whats-new",
                "admin.config.whats_new.title",
                "gift",
                true,
            ),
        ],
    },
    NavSection {
        name: "reports",
        label: Some("admin.config_sections.reports.title"),
        links: &[link(
            "admin_all_reports",
            "/admin/reports",
            "admin.config.reports.title",
            "chart-bar",
            true,
        )],
    },
    NavSection {
        name: "community",
        label: Some("admin.config_sections.community.title"),
        links: &[
            link(
                "admin_about_your_site",
                "/admin/config/about",
                "admin.config.about.title",
                "gear",
                false,
            ),
            link(
                "admin_content",
                "/admin/config/content",
                "admin.config.content.title",
                "pencil",
                false,
            ),
            link(
                "admin_category_management",
                "/admin/config/category-management",
                "admin.config.category_management.title",
                "folder",
                false,
            ),
            link(
                "admin_badges",
                "/admin/badges",
                "admin.config.badges.title",
                "certificate",
                false,
            ),
            link(
                "admin_login",
                "/admin/config/login-and-authentication",
                "admin.config.login.title",
                "unlock",
                false,
            ),
            link(
                "admin_notifications",
                "/admin/config/notifications",
                "admin.config.notifications.title",
                "bell",
                false,
            ),
            link(
                "admin_localization",
                "/admin/config/localization",
                "admin.config.localization.title",
                "globe",
                false,
            ),
            link(
                "admin_analytics",
                "/admin/config/analytics-and-seo",
                "admin.config.analytics.title",
                "chart-pie",
                false,
            ),
            link(
                "admin_permalinks",
                "/admin/config/permalinks",
                "admin.config.permalinks.title",
                "link",
                false,
            ),
            link(
                "admin_user_defaults",
                "/admin/config/user-defaults",
                "admin.config.user_defaults.title",
                "circle-user",
                false,
            ),
            link(
                "admin_trust_levels",
                "/admin/config/trust-levels",
                "admin.config.trust_levels.title",
                "user-shield",
                false,
            ),
            link(
                "admin_group_permissions",
                "/admin/config/group-permissions",
                "admin.config.group_permissions.title",
                "user-gear",
                false,
            ),
            link(
                "admin_user_fields",
                "/admin/config/user-fields",
                "admin.config.user_fields.title",
                "user-pen",
                false,
            ),
            link(
                "admin_watched_words",
                "/admin/customize/watched_words",
                "admin.config.watched_words.title",
                "eye",
                true,
            ),
            link(
                "admin_site_admin",
                "/admin/config/site-admin",
                "admin.config.site_admin.title",
                "screwdriver-wrench",
                false,
            ),
            link(
                "admin_legal",
                "/admin/config/legal",
                "admin.config.legal.title",
                "gavel",
                false,
            ),
            link(
                "admin_moderation_flags",
                "/admin/config/flags",
                "admin.config.flags.title",
                "flag",
                false,
            ),
        ],
    },
    NavSection {
        name: "appearance",
        label: Some("admin.config_sections.appearance.title"),
        links: &[
            link(
                "admin_logo",
                "/admin/config/logo",
                "admin.config.logo.title",
                "fab-discourse",
                false,
            ),
            link(
                "admin_fonts",
                "/admin/config/fonts",
                "admin.config.fonts.title",
                "font",
                false,
            ),
            link(
                "admin_color_palettes",
                "/admin/config/colors",
                "admin.config.color_palettes.title",
                "palette",
                false,
            ),
            link(
                "admin_interface",
                "/admin/config/interface",
                "admin.config.interface.title",
                "discourse-table",
                false,
            ),
            link(
                "admin_emoji",
                "/admin/config/emoji",
                "admin.config.emoji.title",
                "discourse-emojis",
                false,
            ),
            link(
                "admin_navigation",
                "/admin/config/navigation",
                "admin.config.navigation.title",
                "diagram-project",
                false,
            ),
            link(
                "admin_themes_and_components",
                "/admin/config/customize/themes",
                "admin.config.themes_and_components.title",
                "paintbrush",
                false,
            ),
            link(
                "admin_welcome_banner",
                "/admin/config/welcome-banner",
                "admin.config.welcome_banner.title",
                "sign-hanging",
                false,
            ),
            link(
                "admin_customize_site_texts",
                "/admin/customize/site_texts",
                "admin.config.site_texts.title",
                "language",
                false,
            ),
        ],
    },
    NavSection {
        name: "email_settings",
        label: Some("admin.config_sections.email.title"),
        links: &[
            link(
                "admin_server_setup",
                "/admin/email",
                "admin.config.email.title",
                "gear",
                false,
            ),
            link(
                "admin_appearance",
                "/admin/customize/email_style",
                "admin.config.email_appearance.title",
                "palette",
                false,
            ),
            link(
                "admin_email_logs",
                "/admin/email-logs",
                "admin.config.email_logs.title",
                "square-envelope",
                false,
            ),
        ],
    },
    NavSection {
        name: "security",
        label: Some("admin.config_sections.security.title"),
        links: &[
            link(
                "admin_security",
                "/admin/config/security",
                "admin.config.security.title",
                "lock",
                false,
            ),
            link(
                "admin_spam",
                "/admin/config/spam",
                "admin.config.spam.title",
                "robot",
                false,
            ),
            link(
                "admin_logs_staff_action_logs",
                "/admin/logs",
                "admin.config.staff_action_logs.title",
                "user-shield",
                true,
            ),
        ],
    },
    NavSection {
        name: "plugins",
        label: Some("admin.config_sections.plugins.title"),
        links: &[link(
            "admin_installed_plugins",
            "/admin/plugins",
            "admin.config.plugins.title",
            "puzzle-piece",
            false,
        )],
    },
    NavSection {
        name: "advanced",
        label: Some("admin.config_sections.advanced.title"),
        links: &[
            link(
                "admin_backups",
                "/admin/backups",
                "admin.config.backups.title",
                "box-archive",
                false,
            ),
            link(
                "admin_api_keys",
                "/admin/api/keys",
                "admin.config.api_keys.title",
                "key",
                false,
            ),
            link(
                "admin_webhooks",
                "/admin/api/web_hooks",
                "admin.config.webhooks.title",
                "arrows-rotate",
                false,
            ),
            link(
                "admin_developer",
                "/admin/config/developer",
                "admin.config.developer.title",
                "keyboard",
                false,
            ),
            link(
                "admin_mcp",
                "/admin/config/mcp",
                "admin.config.mcp.title",
                "plug",
                false,
            ),
            link(
                "admin_embedding",
                "/admin/customize/embedding",
                "admin.config.embedding.title",
                "code",
                false,
            ),
            link(
                "admin_problem_checks",
                "/admin/problem-checks",
                "admin.config.problem_checks.title",
                "heart",
                false,
            ),
            link(
                "admin_rate_limits",
                "/admin/config/rate-limits",
                "admin.config.rate_limits.title",
                "rocket",
                false,
            ),
            link(
                "admin_onebox",
                "/admin/config/onebox",
                "admin.config.onebox.title",
                "far-square",
                false,
            ),
            link(
                "admin_files",
                "/admin/config/files",
                "admin.config.files.title",
                "file",
                false,
            ),
            link(
                "admin_search",
                "/admin/config/search",
                "admin.config.search.title",
                "magnifying-glass",
                false,
            ),
            link(
                "admin_experimental",
                "/admin/config/experimental",
                "admin.config.experimental.title",
                "discourse-sparkles",
                false,
            ),
        ],
    },
];

/// A bundled plugin with an admin route, as Discourse.visible_plugins
/// lists them (the reference's /admin/plugins.json): its name (the route's
/// location), the route's label, the humanized name, the setting that
/// enables it, and the icon its admin initializer sets
/// (`setAdminPluginIcon`; gear for none).
struct AdminPlugin {
    name: &'static str,
    label: &'static str,
    humanized_name: &'static str,
    enabled_setting: &'static str,
    icon: &'static str,
}

const fn plugin(
    name: &'static str,
    label: &'static str,
    humanized_name: &'static str,
    enabled_setting: &'static str,
    icon: &'static str,
) -> AdminPlugin {
    AdminPlugin {
        name,
        label,
        humanized_name,
        enabled_setting,
        icon,
    }
}

const PLUGINS: &[AdminPlugin] = &[
    plugin(
        "discourse-adplugin",
        "admin.adplugin.house_ads.title",
        "Ad Plugin",
        "discourse_adplugin_enabled",
        "rectangle-ad",
    ),
    plugin(
        "discourse-affiliate",
        "admin.site_settings.categories.discourse_affiliate",
        "Affiliate",
        "affiliate_enabled",
        "handshake",
    ),
    plugin(
        "discourse-ai",
        "discourse_ai.title",
        "AI",
        "discourse_ai_enabled",
        "discourse-sparkles",
    ),
    plugin(
        "discourse-apple-auth",
        "discourse_apple_auth.title",
        "Apple auth",
        "sign_in_with_apple_enabled",
        "fab-apple",
    ),
    plugin(
        "discourse-assign",
        "admin.site_settings.categories.discourse_assign",
        "Assign",
        "assign_enabled",
        "user-plus",
    ),
    plugin(
        "automation",
        "discourse_automation.title",
        "Automation",
        "discourse_automation_enabled",
        "wand-magic-sparkles",
    ),
    plugin(
        "boards",
        "admin.site_settings.categories.boards",
        "Boards",
        "boards_enabled",
        "boards",
    ),
    plugin(
        "discourse-cakeday",
        "discourse_cakeday.title",
        "Cakeday",
        "cakeday_enabled",
        "cake-candles",
    ),
    plugin(
        "discourse-captcha",
        "admin.site_settings.categories.discourse_captcha",
        "Captcha",
        "discourse_captcha_enabled",
        "hand",
    ),
    plugin("chat", "chat.admin.title", "Chat", "chat_enabled", "d-chat"),
    plugin(
        "discourse-chat-integration",
        "chat_integration.menu_title",
        "Chat Integrations",
        "chat_integration_enabled",
        "plug",
    ),
    plugin(
        "discourse-data-explorer",
        "explorer.title",
        "Data Explorer",
        "data_explorer_enabled",
        "chart-line",
    ),
    plugin(
        "discourse-events",
        "discourse_events.title",
        "Events",
        "discourse_events_enabled",
        "calendar-days",
    ),
    plugin(
        "footnote",
        "footnote.title",
        "Footnote",
        "enable_markdown_footnotes",
        "asterisk",
    ),
    plugin(
        "discourse-gamification",
        "gamification.admin.title",
        "Gamification",
        "discourse_gamification_enabled",
        "trophy",
    ),
    plugin(
        "discourse-github",
        "admin.site_settings.categories.discourse_github",
        "GitHub",
        "enable_discourse_github_plugin",
        "fab-github",
    ),
    plugin(
        "discourse-lazy-videos",
        "discourse_lazy_videos.title",
        "Lazy videos",
        "lazy_videos_enabled",
        "bed",
    ),
    plugin(
        "discourse-local-dates",
        "discourse_local_dates.title",
        "Local dates",
        "discourse_local_dates_enabled",
        "far-clock",
    ),
    plugin(
        "discourse-oauth2-basic",
        "admin.site_settings.categories.login",
        "Login",
        "oauth2_enabled",
        "key",
    ),
    plugin(
        "discourse-login-with-amazon",
        "admin.site_settings.categories.discourse_login_with_amazon",
        "Login with Amazon",
        "enable_login_with_amazon",
        "gear",
    ),
    plugin(
        "discourse-lti",
        "discourse_lti.title",
        "Lti",
        "lti_enabled",
        "graduation-cap",
    ),
    plugin(
        "discourse-math",
        "admin.site_settings.categories.discourse_math",
        "Math",
        "discourse_math_enabled",
        "square-root-variable",
    ),
    plugin(
        "discourse-microsoft-auth",
        "discourse_microsoft_auth.title",
        "Microsoft auth",
        "microsoft_auth_enabled",
        "fab-microsoft",
    ),
    plugin(
        "discourse-narrative-bot",
        "discourse_narrative_bot.title",
        "Narrative bot",
        "discourse_narrative_bot_enabled",
        "robot",
    ),
    plugin(
        "discourse-openid-connect",
        "admin.site_settings.categories.discourse_openid_connect",
        "OpenID Connect",
        "openid_connect_enabled",
        "id-card",
    ),
    plugin(
        "discourse-patreon",
        "patreon.title",
        "Patreon",
        "patreon_enabled",
        "fab-patreon",
    ),
    plugin(
        "discourse-policy",
        "discourse_policy.title",
        "Policy",
        "policy_enabled",
        "file-signature",
    ),
    plugin(
        "poll",
        "admin.site_settings.categories.poll",
        "Poll",
        "poll_enabled",
        "chart-bar",
    ),
    plugin(
        "discourse-post-voting",
        "admin.site_settings.categories.discourse_post_voting",
        "Post Voting",
        "post_voting_enabled",
        "arrow-up",
    ),
    plugin(
        "discourse-presence",
        "discourse_presence.title",
        "Presence",
        "presence_enabled",
        "eye",
    ),
    plugin(
        "discourse-reactions",
        "admin.site_settings.categories.discourse_reactions",
        "Reactions",
        "discourse_reactions_enabled",
        "discourse-emojis",
    ),
    plugin(
        "discourse-rss-polling",
        "rss_polling.title",
        "Rss polling",
        "rss_polling_enabled",
        "rss",
    ),
    plugin(
        "discourse-solved",
        "admin.site_settings.categories.discourse_solved",
        "Solved",
        "solved_enabled",
        "far-square-check",
    ),
    plugin(
        "styleguide",
        "styleguide.title",
        "Styleguide",
        "styleguide_enabled",
        "gear",
    ),
    plugin(
        "discourse-subscriptions",
        "discourse_subscriptions.admin_navigation",
        "Subscriptions",
        "discourse_subscriptions_enabled",
        "far-credit-card",
    ),
    plugin(
        "discourse-templates",
        "admin.site_settings.categories.discourse_templates",
        "Templates",
        "discourse_templates_enabled",
        "far-clipboard",
    ),
    plugin(
        "discourse-topic-voting",
        "admin.site_settings.categories.discourse_topic_voting",
        "Topic Voting",
        "topic_voting_enabled",
        "check-to-slot",
    ),
    plugin(
        "discourse-user-notes",
        "discourse_user_notes.title",
        "User notes",
        "user_notes_enabled",
        "pen-to-square",
    ),
    plugin(
        "voice",
        "voice.admin.title",
        "Voice",
        "voice_enabled",
        "microphone-lines",
    ),
    plugin(
        "discourse-workflows",
        "discourse_workflows.admin.title",
        "Workflows",
        "enable_discourse_workflows",
        "code-branch",
    ),
    plugin(
        "discourse-zendesk-plugin",
        "admin.site_settings.categories.discourse_zendesk",
        "Zendesk",
        "zendesk_enabled",
        "headset",
    ),
];

/// A link as the panel renders it.
struct Link {
    name: String,
    href: String,
    text: String,
    /// The plugins' links carry their humanized name as a title.
    title: Option<String>,
    icon: String,
    /// `labelOrText(link)`, as the admin search labels it: a plugin's
    /// humanized name, else the label's text.
    search_label: String,
    /// A plugin link's description (the plugin's about).
    description: Option<String>,
}

/// A link of the admin nav as the admin search indexes it.
pub struct SearchLink {
    pub name: String,
    pub href: String,
    pub label: String,
    pub icon: String,
    pub description: Option<String>,
}

/// The visible nav's sections, `labelOrText(section)` each (empty for
/// the root), with their links, for the admin search.
pub fn search_sections(cx: &AdminCx) -> Result<Vec<(String, Vec<SearchLink>)>, SettingError> {
    Ok(sections(cx)?
        .into_iter()
        .map(|(section, links)| {
            let label = section.label.map(|l| t(cx.i18n, l)).unwrap_or_default();
            let links = links
                .into_iter()
                .map(|l| SearchLink {
                    name: l.name,
                    href: l.href,
                    label: l.search_label,
                    icon: l.icon,
                    description: l.description,
                })
                .collect();
            (label, links)
        })
        .collect())
}

/// `stripDiscoursePrefix`
fn strip_discourse_prefix(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if !lower.starts_with("discourse") {
        return text.to_string();
    }
    let rest = &text["discourse".len()..];
    let trimmed = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '-');
    if trimmed.len() == rest.len() {
        text.to_string()
    } else {
        trimmed.to_string()
    }
}

/// pluginAdminRouteLinks: the enabled plugins' links, sorted by their text
/// (localeCompare, then the name).
fn plugin_links(cx: &AdminCx) -> Result<Vec<Link>, SettingError> {
    let mut links = Vec::new();
    for p in PLUGINS {
        if !cx.settings.get(p.enabled_setting)?.truthy() {
            continue;
        }
        // `i18n(label, { translatedFallback: humanized_name })`
        let text = t_opt(cx.i18n, p.label).unwrap_or_else(|| p.humanized_name.to_string());
        links.push(Link {
            name: format!("admin_plugin_{}", p.name),
            href: format!("{}/admin/plugins/{}", cx.base_path, p.name),
            text: strip_discourse_prefix(&text),
            title: Some(p.humanized_name.to_string()),
            icon: p.icon.to_string(),
            search_label: p.humanized_name.to_string(),
            description: crate::admin_plugins::about(p.name),
        });
    }
    // localeCompare on these names: case-insensitively, then by case.
    links.sort_by(|a, b| {
        (a.text.to_lowercase(), &a.text, &a.name).cmp(&(b.text.to_lowercase(), &b.text, &b.name))
    });
    Ok(links)
}

fn core_link(cx: &AdminCx, l: &NavLink) -> Link {
    Link {
        name: l.name.to_string(),
        href: format!("{}{}", cx.base_path, l.path),
        text: strip_discourse_prefix(&t(cx.i18n, l.label)),
        title: None,
        icon: l.icon.to_string(),
        search_label: t(cx.i18n, l.label),
        description: None,
    }
}

/// The sections' links after AdminSidebarPanel's amendments (form
/// templates and GIFs under Appearance while enabled, Upcoming changes
/// under the root, the plugins' links), less what moderators may not see.
fn sections(cx: &AdminCx) -> Result<Vec<(&'static NavSection, Vec<Link>)>, SettingError> {
    let mut out = Vec::new();
    for section in NAV_MAP {
        let mut links: Vec<Link> = section
            .links
            .iter()
            .filter(|l| cx.viewer.admin || l.moderator)
            .map(|l| core_link(cx, l))
            .collect();
        match section.name {
            "root" => {
                // Not marked for moderators, so only admins see it.
                if cx.viewer.admin {
                    links.push(core_link(
                        cx,
                        &link(
                            "admin_upcoming_changes",
                            "/admin/config/upcoming-changes",
                            "admin.config.upcoming_changes.title",
                            "flask",
                            false,
                        ),
                    ));
                }
            }
            "appearance" if cx.viewer.admin => {
                if cx.settings.get("enable_form_templates")?.truthy() {
                    links.push(core_link(
                        cx,
                        &link(
                            "admin_customize_form_templates",
                            "/admin/customize/form-templates",
                            "admin.config.form_templates.title",
                            "list",
                            false,
                        ),
                    ));
                }
                if cx.settings.get("enable_gifs")?.truthy() {
                    links.push(core_link(
                        cx,
                        &link(
                            "admin_gifs",
                            "/admin/config/gifs",
                            "admin.config.gifs.title",
                            "gif",
                            false,
                        ),
                    ));
                }
            }
            "plugins" if cx.viewer.admin => links.extend(plugin_links(cx)?),
            _ => {}
        }
        if !links.is_empty() {
            out.push((section, links));
        }
    }
    Ok(out)
}

/// DButton with only an icon.
fn icon_button(class: &str, icon_name: &str, title: Option<&str>) -> String {
    let title = title
        .map(|t| format!(" title=\"{}\"", escape(t)))
        .unwrap_or_default();
    format!(
        "<button class=\"btn no-text btn-icon {class}\"{title} type=\"button\">{}<span aria-hidden=\"true\"> &#8203; </span></button>",
        icon(icon_name, None)
    )
}

/// The panel's sections and footer: the header (Back to Forum, the toggle
/// for every section, the search for admins or the filter for
/// moderators), the root section's links, and the other sections, each
/// collapsed until opened (collapsedByDefault) unless it holds `active`,
/// the current page's link (expandActiveSection). sidebar.js applies the
/// state the member stored.
pub fn panel(cx: &AdminCx, active: &str, color_selector: &str) -> Result<String, SettingError> {
    let mut out = String::from("<div class=\"sidebar-sections admin-panel\">");
    out.push_str("<div class=\"sidebar-panel-header\"><div class=\"sidebar-panel-header__row\">");
    out.push_str(&format!(
        "<a class=\"sidebar-sections__back-to-forum\" href=\"{}/\">{}<span>{}</span></a>",
        cx.base_path,
        icon("arrow-left", None),
        escape(&t(cx.i18n, "sidebar.back_to_forum"))
    ));
    // ToggleAllSections, whose title admin.js swaps with the state.
    out.push_str(
        &icon_button(
            "btn-transparent sidebar-toggle-all-sections",
            "angles-down",
            Some(&t(cx.i18n, "sidebar.expand_all_sections")),
        )
        .replacen(
            " type=\"button\"",
            &format!(
                " type=\"button\" data-expand-title=\"{}\" data-collapse-title=\"{}\"",
                escape(&t(cx.i18n, "sidebar.expand_all_sections")),
                escape(&t(cx.i18n, "sidebar.collapse_all_sections"))
            ),
            1,
        ),
    );
    out.push_str("</div><div class=\"sidebar-panel-header__row\">");
    if cx.viewer.admin {
        // The search opens the admin search's modal (admin-search.js).
        out.push_str(&format!(
            "<div class=\"sidebar-search\"><div class=\"sidebar-search__input-container\">{}<input class=\"sidebar-search__input\" enterkeyhint=\"done\" placeholder=\"{}\" type=\"text\">{}</div></div>",
            icon_button("btn-transparent sidebar-search__icon", "magnifying-glass", None),
            escape(&t(cx.i18n, "sidebar.search")),
            shortcut_hint(cx)
        ));
    } else {
        out.push_str(&format!(
            "<div class=\"sidebar-filter\"><div class=\"sidebar-filter__input-container\"><input class=\"sidebar-filter__input\" enterkeyhint=\"done\" placeholder=\"{}\" type=\"text\" value=\"\"></div></div>",
            escape(&t(cx.i18n, "sidebar.filter_links"))
        ));
    }
    out.push_str("</div></div>");
    let sections = sections(cx)?;
    if !cx.viewer.admin {
        // The links' keywords the filter matches too, by link name.
        let keywords: serde_json::Map<String, serde_json::Value> = sections
            .iter()
            .flat_map(|(_, links)| links)
            .filter_map(|l| {
                let keywords = crate::admin_search::link_keywords(cx, &l.name);
                (!keywords.is_empty()).then(|| (l.name.clone(), serde_json::json!(keywords)))
            })
            .collect();
        out.push_str(&format!(
            "<script type=\"application/json\" class=\"admin-sidebar-keywords\">{}</script>",
            serde_json::Value::Object(keywords)
                .to_string()
                .replace('<', "\\u003c")
        ));
    }
    for (section, links) in sections {
        let mut html = String::new();
        let mut has_active = false;
        for l in &links {
            let is_active = l.name == active;
            has_active |= is_active;
            html.push_str(&link_html(cx, l, is_active));
        }
        let name = format!("admin-{}", section.name);
        let header = section.label.map(|label| t(cx.i18n, label));
        // The design wizard, Appearance's header action.
        let design_wizard = section.name == "appearance" && cx.viewer.can_run_design_wizard;
        let buttons = if design_wizard {
            let title = escape(&t(cx.i18n, "design_wizard.launch"));
            format!(
                "<button aria-label=\"{title}\" class=\"sidebar-section-header-button btn-icon btn-flat\" data-sidebar-action-id=\"design_wizard\" title=\"{title}\" type=\"button\">{}</button>",
                icon("wand-magic", None)
            )
        } else {
            String::new()
        };
        let classes = if design_wizard {
            " sidebar-section--persistent-actions"
        } else {
            ""
        };
        out.push_str(&crate::sidebar::section_html_state(
            &name,
            header.as_deref(),
            &buttons,
            &html,
            header.is_none() || has_active,
            classes,
            cx.i18n,
        ));
    }
    out.push_str("</div>");
    out.push_str(&crate::sidebar::footer(cx.i18n, false, color_selector));
    Ok(out)
}

/// DShortcut's keycaps for `mod+/`, which the device shows only with a
/// keyboard (capabilities.hasKeyboard): admin.js puts them in place when it
/// finds a fine pointer, with Apple's glyph on Apple platforms.
fn shortcut_hint(cx: &AdminCx) -> String {
    format!(
        "<template class=\"sidebar-search__shortcut\"><kbd aria-hidden=\"true\" class=\"d-shortcut sidebar-search__shortcut-hint\" dir=\"ltr\" data-command=\"{}\"><kbd class=\"d-shortcut__key\">{}</kbd><kbd class=\"d-shortcut__key\"><span aria-hidden=\"true\">/</span><span class=\"sr-only\">{}</span></kbd></kbd></template>",
        escape(&t(cx.i18n, "shortcut_modifier_key.command")),
        escape(&t(cx.i18n, "shortcut_modifier_key.ctrl")),
        escape(&t(cx.i18n, "shortcut_modifier_key.slash"))
    )
}

/// SectionLink for a SidebarAdminSectionLink: the review queue's pending
/// count, the dot on What's new and Upcoming changes while there is news.
fn link_html(cx: &AdminCx, l: &Link, active: bool) -> String {
    let mut a = format!(
        "<a class=\"{}sidebar-section-link sidebar-row admin-sidebar-nav-link\"",
        if active { "active " } else { "" }
    );
    if let Some(title) = &l.title {
        a.push_str(&format!(" title=\"{}\"", escape(title)));
    }
    a.push_str(&format!(
        " data-link-name=\"{}\" href=\"{}\">",
        escape(&l.name),
        escape(&l.href)
    ));
    let badge = if l.name == "admin_review" && cx.viewer.reviewable_count > 0 {
        format!(
            "<span class=\"sidebar-section-link-content-badge\">{}</span>",
            escape(&t_count(
                cx.i18n,
                "sidebar.sections.community.links.review.pending_count",
                cx.viewer.reviewable_count,
                &[],
            ))
        )
    } else {
        String::new()
    };
    let dot = (l.name == "admin_whats_new" && cx.viewer.has_unseen_features)
        || (l.name == "admin_upcoming_changes" && cx.viewer.has_new_upcoming_changes);
    let suffix = if dot {
        format!(
            "<span class=\"sidebar-section-link-suffix icon admin-sidebar-nav-link__dot\">{}</span>",
            icon("circle", None)
        )
    } else {
        String::new()
    };
    format!(
        "<li class=\"sidebar-section-link-wrapper\" data-list-item-name=\"{}\">{a}<span class=\"sidebar-section-link-prefix icon\">{}</span><span class=\"sidebar-section-link-content-text\">{}</span>{badge}{suffix}</a></li>",
        escape(&l.name),
        crate::post_view::d_icon(&l.icon, Some("prefix-icon")),
        escape(&l.text)
    )
}

/// The desktop sidebar with the admin panel.
pub fn sidebar(cx: &AdminCx, active: &str, color_selector: &str) -> Result<String, SettingError> {
    Ok(crate::sidebar::wrap_sidebar(
        cx.i18n,
        &panel(cx, active, color_selector)?,
    ))
}

/// The mobile view's header menu with the admin panel.
pub fn hamburger(cx: &AdminCx, active: &str, color_selector: &str) -> Result<String, SettingError> {
    Ok(crate::sidebar::wrap_hamburger(&panel(
        cx,
        active,
        color_selector,
    )?))
}

/// A breadcrumb: its label and path.
pub struct Crumb<'a> {
    pub label: String,
    pub path: &'a str,
}

/// DPageHeader with its breadcrumbs (DBreadcrumbsContainer: the last one
/// the current page), title, actions, description and tabs.
pub struct PageHeader<'a> {
    pub crumbs: Vec<Crumb<'a>>,
    pub title: String,
    /// The actions block's markup, empty for none.
    pub actions: String,
    /// `@descriptionLabel` (HTML), with `@learnMoreUrl`.
    pub description: Option<String>,
    pub learn_more_url: Option<String>,
    /// The tabs (DHorizontalOverflowNav's items), None for `@hideTabs`.
    pub tabs: Option<String>,
}

impl PageHeader<'_> {
    pub fn render(&self, cx: &AdminCx) -> String {
        let mut out = String::from("<div class=\"d-page-header\">");
        if !self.crumbs.is_empty() {
            out.push_str("<div class=\"d-page-header__breadcrumbs\"><ul class=\"d-breadcrumbs\">");
            let last = self.crumbs.len() - 1;
            for (i, crumb) in self.crumbs.iter().enumerate() {
                out.push_str(&format!(
                    "<li{} class=\"d-breadcrumbs__item\"><a class=\"d-breadcrumbs__link\" href=\"{}{}\">{}</a>",
                    if i == last { " aria-current=\"page\"" } else { "" },
                    cx.base_path,
                    escape(crumb.path),
                    escape(&crumb.label)
                ));
                if i != last {
                    out.push_str(&format!(
                        " <span class=\"separator\">{}</span>",
                        icon("angle-right", None)
                    ));
                }
                out.push_str("</li>");
            }
            out.push_str("</ul></div>");
        }
        out.push_str(&format!(
            "<div class=\"d-page-header__title-row\"><h1 class=\"d-page-header__title\">{}</h1>",
            escape(&self.title)
        ));
        if !self.actions.is_empty() {
            out.push_str(&format!(
                "<div class=\"d-page-header__actions\">{}</div>",
                self.actions
            ));
        }
        out.push_str("</div>");
        if let Some(description) = &self.description {
            out.push_str(&format!(
                "<p class=\"d-page-header__description\">{description}"
            ));
            if let Some(url) = &self.learn_more_url {
                out.push_str(&format!(
                    " <span class=\"d-page-header__learn-more\">{}</span>",
                    t_with(cx.i18n, "learn_more_with_link", &[("url", url)])
                ));
            }
            out.push_str("</p>");
        }
        if let Some(tabs) = &self.tabs {
            out.push_str(&format!(
                "<div class=\"d-nav-submenu\"><nav class=\"horizontal-overflow-nav\"><div class=\"d-overflow-controls --owned-scroller horizontal-overflow-nav__controls\"><ul class=\"nav-pills action-list d-nav-submenu__tabs\" data-d-scroll-axis=\"horizontal\">{tabs}</ul></div></nav></div>"
            ));
        }
        out.push_str("</div>");
        out
    }
}

/// A page header's or subheader's action (DPageActionButton).
pub struct PageAction {
    /// The action's own classes (what the page's script finds it by).
    pub class: String,
    /// primary, default or danger.
    pub kind: &'static str,
    pub label: String,
    pub title: Option<String>,
}

/// The actions block of DPageHeader (`identifier` d-page-header) or
/// DPageSubheader (d-page-subheader): the buttons, or in the mobile view
/// (collapseActionsOnMobile) the More options DMenu trigger with the
/// buttons as DPageActionListItems in its template, which admin.js opens;
/// an item clicks the button it stands for, kept hidden beside.
pub fn page_actions(cx: &AdminCx, identifier: &str, actions: &[PageAction]) -> String {
    let buttons: String = actions
        .iter()
        .map(|a| {
            format!(
                "<button class=\"btn d-page-action-button btn-small btn-{} {}\"{} type=\"button\"><span class=\"d-button-label\">{}</span></button>",
                a.kind,
                escape(&a.class),
                a.title
                    .as_ref()
                    .map(|t| format!(" title=\"{}\"", escape(t)))
                    .unwrap_or_default(),
                escape(&a.label)
            )
        })
        .collect();
    if actions.is_empty() || !crate::mobile::mobile_view() {
        return buttons;
    }
    let items: String = actions
        .iter()
        .map(|a| {
            format!(
                "<li class=\"dropdown-menu__item d-page-action-list-item\"><button class=\"btn btn-text d-page-action-button btn-small btn-transparent{}\" type=\"button\" data-page-action=\"{}\"><span class=\"d-button-label\">{}</span></button></li>",
                if a.kind == "danger" { " btn-danger" } else { "" },
                escape(&a.class),
                escape(&a.label)
            )
        })
        .collect();
    let more = t(cx.i18n, "more_options");
    format!(
        "<button aria-expanded=\"false\" class=\"btn no-text btn-icon fk-d-menu__trigger {identifier}-mobile-actions-trigger btn-small btn-default\" title=\"{}\" data-identifier=\"{identifier}-mobile-actions\" data-trigger=\"\" type=\"button\">{}</button><template class=\"d-page-mobile-actions\"><ul class=\"dropdown-menu {identifier}__mobile-actions\">{items}</ul></template><div hidden>{buttons}</div>",
        escape(&more),
        icon("ellipsis-vertical", None)
    )
}

/// templates/admin.gjs around the route's content, with the classes
/// AdminController#adminContentsClassName takes from the route's path.
pub fn contents(classes: &str, inner: &str) -> String {
    format!(
        "<div class=\"row\"><div class=\"full-width\"><div class=\"boxed white admin-content\"><div class=\"admin-contents {}\">{inner}</div></div></div></div>",
        escape(classes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel_for(
        viewer: AdminViewer,
        rows: Vec<(String, i32, Option<String>)>,
        active: &str,
    ) -> String {
        let defs = crate::site_settings::Definitions::vendored().unwrap();
        let globals = crate::config::GlobalSettings::from_vars(Vec::<(String, String)>::new());
        let settings = SiteSettings::resolve(&defs, rows, &globals).unwrap();
        let i18n = I18n::vendored().unwrap();
        let cx = AdminCx {
            i18n: &i18n,
            settings: &settings,
            base_path: "",
            viewer: &viewer,
        };
        panel(&cx, active, "").unwrap()
    }

    #[test]
    fn moderators_get_their_links_and_the_filter() {
        let html = panel_for(AdminViewer::default(), vec![], "admin_users");
        assert!(html.contains("data-link-name=\"admin_users\""), "{html}");
        assert!(html.contains("data-link-name=\"admin_watched_words\""));
        assert!(html.contains("data-link-name=\"admin_logs_staff_action_logs\""));
        assert!(!html.contains("data-link-name=\"admin_all_site_settings\""));
        assert!(!html.contains("data-link-name=\"admin_upcoming_changes\""));
        // Sections left with no link go.
        assert!(!html.contains("data-section-name=\"admin-plugins\""));
        assert!(!html.contains("data-section-name=\"admin-advanced\""));
        assert!(!html.contains("data-section-name=\"admin-appearance\""));
        assert!(html.contains("class=\"sidebar-filter__input\""));
        assert!(!html.contains("class=\"sidebar-search__input\""));
        assert!(html.contains("<a class=\"active sidebar-section-link sidebar-row admin-sidebar-nav-link\" data-link-name=\"admin_users\" href=\"/admin/users\">"));
        // The links' keywords for the filter.
        let start = html
            .find("<script type=\"application/json\" class=\"admin-sidebar-keywords\">")
            .expect("keywords");
        let json = &html[start..];
        let json = &json[json.find('>').unwrap() + 1..json.find("</script>").unwrap()];
        let keywords: serde_json::Value = serde_json::from_str(json).unwrap();
        assert!(
            keywords["admin_logs_staff_action_logs"]
                .as_array()
                .is_some_and(|k| !k.is_empty()),
            "{keywords}"
        );
    }

    #[test]
    fn a_section_holding_the_page_starts_open() {
        let viewer = AdminViewer {
            admin: true,
            ..Default::default()
        };
        let html = panel_for(viewer, vec![], "admin_backups");
        assert!(html.contains("<div class=\"sidebar-section sidebar-section-wrapper sidebar-section--expanded\" data-section-name=\"admin-advanced\">"), "{html}");
        assert!(html.contains("<div class=\"sidebar-section sidebar-section-wrapper sidebar-section--collapsed\" data-section-name=\"admin-reports\">"));
    }

    #[test]
    fn admins_get_the_enabled_plugins_and_the_review_count() {
        let viewer = AdminViewer {
            admin: true,
            reviewable_count: 3,
            can_run_design_wizard: true,
            has_new_upcoming_changes: true,
            ..Default::default()
        };
        // poll is on by default; chat off here.
        let rows = vec![("chat_enabled".to_string(), 5, Some("f".to_string()))];
        let html = panel_for(viewer, rows, "admin_home");
        assert!(html.contains("<a class=\"sidebar-section-link sidebar-row admin-sidebar-nav-link\" title=\"Poll\" data-link-name=\"admin_plugin_poll\" href=\"/admin/plugins/poll\">"), "{html}");
        assert!(!html.contains("admin_plugin_chat"));
        assert!(!html.contains("admin_plugin_discourse-ai"));
        assert!(
            html.contains("<span class=\"sidebar-section-link-content-badge\">3 pending</span>")
        );
        assert!(html.contains("admin-sidebar-nav-link__dot"));
        assert!(html.contains("data-sidebar-action-id=\"design_wizard\""));
        assert!(html.contains("sidebar-section--collapsed sidebar-section--persistent-actions\" data-section-name=\"admin-appearance\""));
    }

    #[test]
    fn strips_a_discourse_prefix_as_the_client_does() {
        assert_eq!(strip_discourse_prefix("Discourse Solved"), "Solved");
        assert_eq!(strip_discourse_prefix("discourse-ai"), "ai");
        assert_eq!(strip_discourse_prefix("Discourses"), "Discourses");
        assert_eq!(strip_discourse_prefix("Chat"), "Chat");
    }
}
