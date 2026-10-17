//! linkifySettingLinks (admin/modifiers/linkify-setting-links.js): a
//! setting description's `{{setting:name}}` links, which the server draws
//! pointing at the all settings page, rewritten to the setting's own page
//! as the admin search's data source resolves it
//! (SettingLinkFormatter#buildURL over its settingPageMap). The JSON keeps
//! the server's links; the pages draw the rewritten ones.

use std::sync::LazyLock;

use regex::Regex;

use crate::site_settings::SiteSettings;

/// AdminSearchDataSource's settingPageMap areas: each area's page, from
/// the admin nav map's first link with that `settings_area` (`/settings`
/// after a multi-tabbed link's route).
const AREAS: &[(&str, &str)] = &[
    ("reports", "/admin/reports/dashboard-settings"),
    ("about", "/admin/config/about"),
    ("categories_and_tags", "/admin/config/content/settings"),
    (
        "authenticators",
        "/admin/config/login-and-authentication/authenticators",
    ),
    (
        "discourseconnect",
        "/admin/config/login-and-authentication/discourseconnect",
    ),
    ("oauth2", "/admin/config/login-and-authentication/oauth2"),
    ("oidc", "/admin/config/login-and-authentication/oidc"),
    ("notifications", "/admin/config/notifications"),
    ("localization", "/admin/config/localization"),
    ("analytics", "/admin/config/analytics-and-seo"),
    ("permalinks", "/admin/config/permalinks/settings"),
    ("user_defaults", "/admin/config/user-defaults"),
    ("trust_levels", "/admin/config/trust-levels"),
    ("group_permissions", "/admin/config/group-permissions"),
    ("site_admin", "/admin/config/site-admin"),
    ("flags", "/admin/config/flags/settings"),
    ("interface", "/admin/config/interface"),
    ("emojis", "/admin/config/emoji/settings"),
    ("navigation", "/admin/config/navigation"),
    ("embedding", "/admin/customize/embedding"),
];

/// The settingPageMap's categories, from the links with a
/// `settings_category`.
const CATEGORIES: &[(&str, &str)] = &[
    ("security", "/admin/config/security"),
    ("spam", "/admin/config/spam"),
    ("backups", "/admin/backups/settings"),
    ("developer", "/admin/config/developer"),
    ("rate_limits", "/admin/config/rate-limits"),
    ("onebox", "/admin/config/onebox"),
    ("files", "/admin/config/files"),
    ("search", "/admin/config/search"),
    ("experimental", "/admin/config/experimental"),
];

/// What a setting link carries.
pub struct SettingRef<'a> {
    pub setting: &'a str,
    pub area: Option<&'a str>,
    pub category: Option<&'a str>,
    pub plugin: Option<&'a str>,
}

/// SettingLinkFormatter#buildURL: a plugin's settings page when the plugin
/// has its admin page (enabled), else its settings category; then the
/// setting's area page, its category page, or all settings.
pub fn url_for_setting(settings: &SiteSettings, base_path: &str, s: &SettingRef<'_>) -> String {
    let filter = format!("?filter={}", s.setting);
    if let Some(plugin) = s.plugin {
        let name = plugin.replace('_', "-");
        let enabled = crate::admin_plugins::admin_route_location(settings, &name);
        return match enabled {
            Some(location) => format!("{base_path}/admin/plugins/{location}/settings{filter}"),
            None => format!(
                "{base_path}/admin/site_settings/category/{}{filter}",
                plugin.replace('-', "_")
            ),
        };
    }
    // The GIFs link (and so its area) is in the nav while GIFs are enabled.
    if s.area == Some("gifs") && settings.get("enable_gifs").is_ok_and(|v| v.truthy()) {
        return format!("{base_path}/admin/config/gifs{filter}");
    }
    let page = s
        .area
        .and_then(|a| AREAS.iter().find(|(k, _)| *k == a))
        .or_else(|| {
            s.category
                .and_then(|c| CATEGORIES.iter().find(|(k, _)| *k == c))
        });
    match page {
        Some((_, path)) => format!("{base_path}{path}{filter}"),
        None => format!("{base_path}/admin/site_settings/category/all_results{filter}"),
    }
}

/// The modifier over a description's HTML: each
/// `a.site-setting-link[data-setting-name]`'s href.
pub fn linkify(settings: &SiteSettings, base_path: &str, html: &str) -> String {
    static ANCHOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<a ([^>]*class="site-setting-link"[^>]*)>"#).expect("regex")
    });
    static ATTR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"([a-z-]+)="([^"]*)""#).expect("regex"));
    if !html.contains("site-setting-link") {
        return html.to_string();
    }
    ANCHOR
        .replace_all(html, |c: &regex::Captures| {
            let attrs = &c[1];
            let get = |name: &str| {
                ATTR.captures_iter(attrs)
                    .find(|a| &a[1] == name)
                    .map(|a| a[2].to_string())
            };
            let Some(setting) = get("data-setting-name") else {
                return c[0].to_string();
            };
            let (area, category, plugin) = (
                get("data-setting-area"),
                get("data-setting-category"),
                get("data-setting-plugin"),
            );
            let url = url_for_setting(
                settings,
                base_path,
                &SettingRef {
                    setting: &setting,
                    area: area.as_deref(),
                    category: category.as_deref(),
                    plugin: plugin.as_deref(),
                },
            );
            let rewritten = ATTR.replace_all(attrs, |a: &regex::Captures| {
                if &a[1] == "href" {
                    format!(
                        r#"href="{}""#,
                        crate::admin_site_settings::escape_html(&url)
                    )
                } else {
                    a[0].to_string()
                }
            });
            format!("<a {rewritten}>")
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GlobalSettings;
    use crate::site_settings::Definitions;

    fn settings() -> SiteSettings {
        let defs = Definitions::vendored().unwrap();
        SiteSettings::resolve(&defs, vec![], &GlobalSettings::default()).unwrap()
    }

    #[test]
    fn links_go_to_the_settings_page() {
        let s = settings();
        let html = r#"configure <a class="site-setting-link" href="/admin/site_settings/category/all_results?filter=slow_down_crawler_rate" data-setting-name="slow_down_crawler_rate" data-setting-category="security">Slow</a> and <a class="site-setting-link" href="/x?filter=faq_url" data-setting-name="faq_url" data-setting-area="legal" data-setting-category="legal">FAQ</a>"#;
        let out = linkify(&s, "", html);
        assert!(
            out.contains(r#"href="/admin/config/security?filter=slow_down_crawler_rate""#),
            "{out}"
        );
        assert!(
            out.contains(r#"href="/admin/site_settings/category/all_results?filter=faq_url""#),
            "{out}"
        );
    }

    #[test]
    fn plugin_settings_follow_the_plugin() {
        let s = settings();
        // chat is enabled by default: its settings page.
        let chat = url_for_setting(
            &s,
            "",
            &SettingRef {
                setting: "chat_allowed_groups",
                area: None,
                category: None,
                plugin: Some("chat"),
            },
        );
        assert_eq!(
            chat,
            "/admin/plugins/chat/settings?filter=chat_allowed_groups"
        );
        let poll = url_for_setting(
            &s,
            "/f",
            &SettingRef {
                setting: "poll_maximum_options",
                area: None,
                category: None,
                plugin: Some("poll"),
            },
        );
        assert!(poll.starts_with("/f/admin/"), "{poll}");
    }
}
