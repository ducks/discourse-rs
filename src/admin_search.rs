//! The admin search (AdminSearchDataSource, 90g9): its pages from the
//! admin nav (the visible links with the nav map's descriptions, keywords
//! and tabs, recorded from admin-nav-map.js by scripts/record-admin-nav-map
//! into vendor/discourse/config/admin_nav_map.json) and the formatted items
//! of every type (PageLinkFormatter, SettingLinkFormatter, the themes,
//! components, reports and upcoming changes) that admin-search.js scores
//! as the service's `search` does.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::admin_view::{AdminCx, t, t_opt};
use crate::site_settings::SettingError;

/// `SEPARATOR`
const SEPARATOR: &str = ">";

const RECORDED: &str = include_str!("../vendor/discourse/config/admin_nav_map.json");

/// A link of the recorded nav map.
#[derive(Debug, Deserialize, Clone, Default)]
struct NavLink {
    name: Option<String>,
    route: Option<String>,
    label: Option<String>,
    text: Option<String>,
    description: Option<String>,
    keywords: Option<String>,
    icon: Option<String>,
    #[serde(default)]
    links: Vec<NavLink>,
}

#[derive(Debug, Deserialize)]
struct NavSection {
    links: Vec<NavLink>,
}

/// The recorded links by name, the amendments AdminSidebarPanel adds
/// (form templates, GIFs, upcoming changes) and the login tabs the auth
/// plugins add (oauth2, oidc) included.
fn recorded() -> &'static HashMap<String, NavLink> {
    static MAP: OnceLock<HashMap<String, NavLink>> = OnceLock::new();
    MAP.get_or_init(|| {
        let sections: Vec<NavSection> = serde_json::from_str(RECORDED).expect("admin_nav_map.json");
        let mut map: HashMap<String, NavLink> = sections
            .into_iter()
            .flat_map(|s| s.links)
            .filter_map(|l| Some((l.name.clone()?, l)))
            .collect();
        let amended = |name: &str, description: &str, keywords: Option<&str>| NavLink {
            name: Some(name.into()),
            description: Some(description.into()),
            keywords: keywords.map(str::to_string),
            ..Default::default()
        };
        map.insert(
            "admin_customize_form_templates".into(),
            amended(
                "admin_customize_form_templates",
                "admin.config.form_templates.header_description",
                None,
            ),
        );
        map.insert(
            "admin_gifs".into(),
            amended("admin_gifs", "admin.config.gifs.header_description", None),
        );
        map.insert(
            "admin_upcoming_changes".into(),
            amended(
                "admin_upcoming_changes",
                "admin.config.upcoming_changes.header_description",
                Some("admin.config.upcoming_changes.keywords"),
            ),
        );
        if let Some(login) = map.get_mut("admin_login") {
            for id in ["oauth2", "oidc"] {
                login.links.push(NavLink {
                    name: Some(format!("admin_login_{id}")),
                    route: Some("adminConfig.login.plugin-tab".into()),
                    label: Some(format!("admin.config.login.sub_pages.{id}")),
                    icon: Some("unlock".into()),
                    ..Default::default()
                });
            }
        }
        map
    })
}

/// The keywords the panel's filter matches a link by
/// (adminSidebarStateManager.setLinkKeywords of the nav map's keywords).
pub fn link_keywords(cx: &AdminCx, name: &str) -> Vec<String> {
    recorded()
        .get(name)
        .and_then(|l| l.keywords.as_deref())
        .map(|k| t(cx.i18n, k).split('|').map(str::to_lowercase).collect())
        .unwrap_or_default()
}

/// A tab's path, by its route (and for the login plugin tabs its name).
fn tab_path(link: &NavLink) -> Option<String> {
    let route = link.route.as_deref()?;
    Some(
        match route {
            "adminReports.dashboardSettings" => "/admin/reports/dashboard-settings",
            "adminConfig.login.authenticators" => {
                "/admin/config/login-and-authentication/authenticators"
            }
            "adminConfig.login.discourseconnect" => {
                "/admin/config/login-and-authentication/discourseconnect"
            }
            "adminConfig.login.plugin-tab" => {
                return Some(format!(
                    "/admin/config/login-and-authentication/{}",
                    link.name.as_deref()?.strip_prefix("admin_login_")?
                ));
            }
            "adminConfig.customize.themeSiteSettings" => {
                "/admin/config/customize/theme-site-settings"
            }
            "adminEmail.previewDigest" => "/admin/email/preview-digest",
            "adminEmail.advancedTest" => "/admin/email/advanced-test",
            "adminEmailTemplates" => "/admin/email/templates",
            "adminEmailLogs.sent" => "/admin/email-logs",
            "adminEmailLogs.skipped" => "/admin/email-logs/skipped",
            "adminEmailLogs.bounced" => "/admin/email-logs/bounced",
            "adminEmailLogs.received" => "/admin/email-logs/received",
            "adminEmailLogs.rejected" => "/admin/email-logs/rejected",
            "adminLogs.screenedEmails" => "/admin/logs/screened_emails",
            "adminLogs.screenedIpAddresses" => "/admin/logs/screened_ip_addresses",
            "adminLogs.screenedUrls" => "/admin/logs/screened_urls",
            "adminSearchLogs" => "/admin/logs/search_logs",
            "admin.backups.logs" => "/admin/backups/logs",
            other => {
                let tab = other.strip_prefix("adminConfig.mcp.")?;
                return Some(format!("/admin/config/mcp/{tab}"));
            }
        }
        .to_string(),
    )
}

/// The configuration nav tabs of the bundled plugins that register them
/// (chat's and workflows' admin initializers): (label, description, path
/// under the plugin's page).
fn plugin_tabs(plugin: &str) -> &'static [(&'static str, Option<&'static str>, &'static str)] {
    match plugin {
        "chat" => &[(
            "chat.incoming_webhooks.title",
            Some("chat.incoming_webhooks.header_description"),
            "hooks",
        )],
        "discourse-workflows" => &[
            ("discourse_workflows.title", None, "workflows"),
            ("discourse_workflows.variables.title", None, "variables"),
            ("discourse_workflows.credentials.title", None, "credentials"),
            ("discourse_workflows.data_tables.title", None, "data-tables"),
            ("discourse_workflows.executions.title", None, "executions"),
            ("discourse_workflows.templates.title", None, "templates"),
        ],
        _ => &[],
    }
}

/// One item of the data source.
#[derive(Debug, Clone)]
pub struct Item {
    pub label: String,
    pub url: String,
    pub keywords: String,
    pub description: String,
    pub icon: String,
    pub kind: &'static str,
}

impl Item {
    pub fn json(&self) -> Value {
        json!({
            "label": self.label,
            "url": self.url,
            "keywords": self.keywords,
            "description": self.description,
            "icon": self.icon,
            "type": self.kind,
        })
    }
}

/// `buildKeywords`: the parts joined by spaces, lowercased.
fn build_keywords(parts: &[&str]) -> String {
    parts.join(" ").to_lowercase()
}

/// `i18n(key)` of a description, or the text itself when it has a space.
fn described(cx: &AdminCx, description: Option<&str>) -> String {
    match description {
        Some(d) if d.contains(' ') => d.to_string(),
        Some(d) => t(cx.i18n, d),
        None => String::new(),
    }
}

/// A page link as PageLinkFormatter reads it: `labelOrText`, the url, the
/// icon, the nav map's entry (keywords, description) and a description
/// given beside it (a plugin's).
struct PageLink<'a> {
    label: String,
    url: String,
    icon: &'a str,
    meta: Option<&'a NavLink>,
    description: Option<String>,
}

/// PageLinkFormatter#format: under the section's label, or a tab under its
/// parent's.
fn page_item(
    cx: &AdminCx,
    section_label: &str,
    parent_label: Option<&str>,
    link: PageLink,
) -> Item {
    let link_label = link.label.as_str();
    let label = match parent_label {
        Some(parent) => format!("{parent} {SEPARATOR} {link_label}"),
        None if section_label != link_label => format!(
            "{section_label}{}{link_label}",
            if section_label.is_empty() {
                String::new()
            } else {
                format!(" {SEPARATOR} ")
            }
        ),
        None => section_label.to_string(),
    };
    let keywords = link
        .meta
        .and_then(|m| m.keywords.as_deref())
        .map(|k| t(cx.i18n, k).to_lowercase().replace('|', " "))
        .unwrap_or_default();
    let description = link
        .description
        .unwrap_or_else(|| described(cx, link.meta.and_then(|m| m.description.as_deref())));
    static SPACES: OnceLock<regex::Regex> = OnceLock::new();
    let spaces = SPACES.get_or_init(|| regex::Regex::new("  +").expect("regex"));
    let label_words = spaces
        .replace_all(&label.replacen(SEPARATOR, "", 1).to_lowercase(), " ")
        .into_owned();
    Item {
        keywords: build_keywords(&[&keywords, &link.url, &label_words, &description]),
        label,
        url: link.url,
        description,
        icon: link.icon.to_string(),
        kind: "page",
    }
}

/// The pages, from the visible admin nav: each link, then its tabs.
pub fn pages(cx: &AdminCx) -> Result<Vec<Item>, SettingError> {
    let recorded = recorded();
    let mut items = Vec::new();
    for (section_label, links) in crate::admin_view::search_sections(cx)? {
        for link in links {
            let meta = recorded.get(&link.name);
            let item = page_item(
                cx,
                &section_label,
                None,
                PageLink {
                    label: link.label.clone(),
                    url: link.href.clone(),
                    icon: &link.icon,
                    meta,
                    description: link.description.clone(),
                },
            );
            let parent = item.label.clone();
            items.push(item);
            for tab in meta.map(|m| m.links.as_slice()).unwrap_or_default() {
                let Some(path) = tab_path(tab) else {
                    continue;
                };
                let label = tab
                    .text
                    .clone()
                    .or_else(|| tab.label.as_deref().map(|l| t(cx.i18n, l)))
                    .unwrap_or_default();
                items.push(page_item(
                    cx,
                    &section_label,
                    Some(&parent),
                    PageLink {
                        label,
                        url: format!("{}{path}", cx.base_path),
                        icon: tab.icon.as_deref().unwrap_or_default(),
                        meta: Some(tab),
                        description: None,
                    },
                ));
            }
            if let Some(plugin) = link.name.strip_prefix("admin_plugin_") {
                for (label, description, path) in plugin_tabs(plugin) {
                    let tab = NavLink {
                        description: description.map(str::to_string),
                        ..Default::default()
                    };
                    items.push(page_item(
                        cx,
                        &section_label,
                        Some(&parent),
                        PageLink {
                            label: t(cx.i18n, label),
                            url: format!("{}/admin/plugins/{plugin}/{path}", cx.base_path),
                            icon: "gear",
                            meta: Some(&tab),
                            description: None,
                        },
                    ));
                }
            }
        }
    }
    Ok(items)
}

/// SettingLinkFormatter#buildLabel: the root label, then the full one.
fn setting_label(cx: &AdminCx, setting: &Value) -> (String, String) {
    let s = |k: &str| setting[k].as_str().unwrap_or_default();
    let category_label = || {
        t(
            cx.i18n,
            &format!("admin.site_settings.categories.{}", s("category")),
        )
    };
    let root = if let Some(plugin) = setting["plugin"].as_str() {
        match crate::admin_plugins::admin_route_label(cx.settings, &plugin.replace('_', "-")) {
            Some(label) => t(cx.i18n, &label),
            None => t(cx.i18n, "admin.plugins.title"),
        }
    } else if let Some(area) = setting["primary_area"].as_str() {
        t_opt(cx.i18n, &format!("admin.config.{area}.title")).unwrap_or_else(category_label)
    } else {
        category_label()
    };
    let full = format!("{root} {SEPARATOR} {}", s("humanized_name"));
    (root, full)
}

/// The settings' items (SettingLinkFormatter#format).
pub fn setting_items(cx: &AdminCx, settings: &[Value]) -> Vec<Item> {
    settings
        .iter()
        .map(|setting| {
            let s = |k: &str| setting[k].as_str().unwrap_or_default();
            let (root, label) = setting_label(cx, setting);
            let keywords: Vec<&str> = setting["keywords"]
                .as_array()
                .map(|l| l.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let url = crate::admin_setting_links::url_for_setting(
                cx.settings,
                cx.base_path,
                &crate::admin_setting_links::SettingRef {
                    setting: s("setting"),
                    area: setting["primary_area"].as_str(),
                    category: setting["category"].as_str(),
                    plugin: setting["plugin"].as_str(),
                },
            );
            Item {
                keywords: build_keywords(&[
                    s("setting"),
                    s("humanized_name"),
                    s("description"),
                    &keywords.join(" "),
                    &root,
                ]),
                label,
                url,
                description: s("description").to_string(),
                icon: "gear".into(),
                kind: "setting",
            }
        })
        .collect()
}

/// The themes' and components' items, as `#processThemesAndComponents`
/// sorts them by kind.
pub fn theme_items(cx: &AdminCx, themes: &[Value]) -> (Vec<Item>, Vec<Item>) {
    let mut themes_out = Vec::new();
    let mut components = Vec::new();
    for theme in themes {
        let name = theme["name"].as_str().unwrap_or_default();
        let description = theme["description"].as_str().unwrap_or_default();
        let component = theme["component"] == true;
        let (kind, word, path, icon) = if component {
            ("component", "component", "components", "puzzle-piece")
        } else {
            ("theme", "theme", "themes", "paintbrush")
        };
        let item = Item {
            label: name.to_string(),
            description: description.to_string(),
            url: format!("{}/admin/customize/{path}/{}", cx.base_path, theme["id"]),
            keywords: build_keywords(&[word, description, name]),
            icon: icon.into(),
            kind,
        };
        if component {
            components.push(item);
        } else {
            themes_out.push(item);
        }
    }
    (themes_out, components)
}

/// The reports' items.
pub fn report_items(cx: &AdminCx, reports: &[Value]) -> Vec<Item> {
    reports
        .iter()
        .map(|r| {
            let s = |k: &str| r[k].as_str().unwrap_or_default();
            Item {
                label: s("title").to_string(),
                description: s("description").to_string(),
                url: format!("{}/admin/reports/{}", cx.base_path, s("type")),
                icon: "chart-bar".into(),
                keywords: build_keywords(&[s("title"), s("description"), s("type")]),
                kind: "report",
            }
        })
        .collect()
}

/// UpcomingChangeLinkFormatter#format
pub fn upcoming_change_items(cx: &AdminCx, changes: &[Value]) -> Vec<Item> {
    changes
        .iter()
        .map(|c| {
            let s = |k: &str| c[k].as_str().unwrap_or_default();
            Item {
                label: s("humanized_name").to_string(),
                description: s("description").to_string(),
                url: format!(
                    "{}/admin/config/upcoming-changes?changeNamesFilter={}",
                    cx.base_path,
                    s("setting")
                ),
                keywords: build_keywords(&[s("humanized_name"), s("setting")]),
                icon: "flask".into(),
                kind: "upcoming_change",
            }
        })
        .collect()
}
