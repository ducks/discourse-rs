//! The installed plugins pages (templates/admin-plugins.gjs): the list
//! (admin-plugins/index.gjs: DPageHeader, DFilterControls with the
//! status dropdown, AdminPluginsList and AdminPluginsListItem), and a
//! plugin's settings in AdminPluginConfigPage. static/js/admin-plugins.js
//! filters the list and toggles a plugin's enabled setting.

use serde_json::Value;

use crate::admin_ui::{Button, tr};
use crate::admin_view::{AdminCx, Crumb, PageHeader, t};
use crate::topic_list_view::{escape, icon};

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

/// `templates/admin-plugins.gjs`: the page's wrapper around the outlet.
pub fn render(content: &str) -> String {
    format!("<div class=\"admin-config-page -no-header\">{content}</div>")
}

/// AdminPlugin#settingCategoryName's translated category: the plugin's
/// own site settings category when the locale names it.
fn translated_category(cx: &AdminCx, plugin: &Value) -> Option<String> {
    crate::admin_view::t_opt(
        cx.i18n,
        &format!(
            "admin.site_settings.categories.{}",
            s(&plugin["name"]).replace('-', "_")
        ),
    )
}

/// AdminPlugin#nameTitleized: the category's name, else the humanized
/// name, without a leading "discourse".
pub fn name_titleized(cx: &AdminCx, plugin: &Value) -> String {
    let name =
        translated_category(cx, plugin).unwrap_or_else(|| s(&plugin["humanized_name"]).to_string());
    let lower = name.to_lowercase();
    if lower.starts_with("discourse") {
        let rest = &name["discourse".len()..];
        let trimmed = rest.trim_start_matches([' ', '\t', '-']);
        if trimmed.len() < rest.len() {
            return trimmed.to_string();
        }
    }
    name
}

/// AdminPlugin#author
fn author(cx: &AdminCx, plugin: &Value) -> String {
    let who = if plugin["is_official"].as_bool().unwrap_or(false)
        || plugin["is_discourse_owned"].as_bool().unwrap_or(false)
    {
        "Discourse"
    } else {
        s(&plugin["authors"])
    };
    t(cx.i18n, "admin.plugins.author").replace("%{author}", who)
}

/// AdminPlugin#linkUrl
fn link_url(plugin: &Value) -> &str {
    match s(&plugin["meta_url"]) {
        "" => s(&plugin["url"]),
        meta => meta,
    }
}

fn preinstalled(plugin: &Value) -> bool {
    s(&plugin["url"]).contains("/discourse/discourse/tree/main/plugins/")
}

/// DToggleSwitch
fn toggle(on: bool, disabled: bool) -> String {
    format!(
        "<div class=\"d-toggle-switch\"><label class=\"d-toggle-switch__label\"><button aria-checked=\"{on}\" class=\"d-toggle-switch__checkbox\"{} role=\"switch\" type=\"button\"></button><span class=\"d-toggle-switch__checkbox-slider\">{}</span></label></div>",
        if disabled { " disabled=\"\"" } else { "" },
        if on {
            icon("check", None)
        } else {
            String::new()
        }
    )
}

/// AdminPluginsListItem
fn row(cx: &AdminCx, plugin: &Value, admin: bool) -> String {
    let name = s(&plugin["name"]);
    let titleized = name_titleized(cx, plugin);
    let has_settings = plugin["has_settings"].as_bool().unwrap_or(false);
    let only_enabled = plugin["has_only_enabled_setting"]
        .as_bool()
        .unwrap_or(false);
    let show_settings = admin && has_settings;
    let settings_disabled = show_settings && only_enabled;
    let new_show_route = plugin["admin_route"]["use_new_show_route"]
        .as_bool()
        .unwrap_or(false);
    let show_path = format!("{}/admin/plugins/{}", cx.base_path, name);
    // pluginUrl: none while the settings button is disabled; the show
    // route, else the plugin's settings category.
    let category = if translated_category(cx, plugin).is_some() {
        name.replace('-', "_")
    } else {
        "plugins".to_string()
    };
    let category_path = format!(
        "{}/admin/site_settings/category/{category}?filter={}",
        cx.base_path,
        crate::ruby::cgi_escape(&format!("plugin:{name}"))
    );
    let name_html = if settings_disabled {
        format!(
            "<div class=\"d-table__overview-name admin-plugins-list__name\">{}</div>",
            escape(&titleized)
        )
    } else {
        format!(
            "<a class=\"d-table__overview-name admin-plugins-list__name d-table__overview-link\" href=\"{}\">{}</a>",
            escape(if new_show_route {
                &show_path
            } else {
                &category_path
            }),
            escape(&titleized)
        )
    };
    let label = plugin["label"]
        .as_str()
        .filter(|l| !l.is_empty())
        .map(|l| {
            format!(
                "<span class=\"admin-plugins-list__badge\">{}</span>",
                escape(l)
            )
        })
        .unwrap_or_default();
    let link = link_url(plugin);
    let about_link = if link.is_empty() {
        String::new()
    } else {
        format!(
            "<a class=\"admin-plugins-list__about-link\" href=\"{}\" rel=\"noopener noreferrer\" target=\"_blank\">{} {}</a>",
            escape(link),
            icon("up-right-from-square", None),
            escape(&t(cx.i18n, "admin.plugins.learn_more"))
        )
    };
    let version_extra = if preinstalled(plugin) {
        format!(
            "<span class=\"admin-plugins-list__preinstalled\">{}</span>",
            escape(&t(cx.i18n, "admin.plugins.preinstalled"))
        )
    } else {
        match plugin["commit_hash"].as_str().filter(|h| !h.is_empty()) {
            Some(hash) => format!(
                "<a class=\"current commit-hash\" href=\"{}\" rel=\"noopener noreferrer\" target=\"_blank\" title=\"{}\">{}</a>",
                escape(s(&plugin["commit_url"])),
                escape(hash),
                escape(&hash[..hash.len().min(7)])
            ),
            None => String::new(),
        }
    };
    let enabled = plugin["enabled"].as_bool().unwrap_or(false);
    let switch = match plugin["enabled_setting"].as_str() {
        Some(setting) => toggle(enabled, false).replacen(
            "class=\"d-toggle-switch__checkbox\"",
            &format!(
                "class=\"d-toggle-switch__checkbox\" data-enabled-setting=\"{}\"",
                escape(setting)
            ),
            1,
        ),
        None => toggle(enabled, true),
    };
    let settings_button = if show_settings {
        format!(
            "<a class=\"btn btn-default btn-text btn-small{}\" data-plugin-setting-button=\"{}\" href=\"{}\" title=\"{}\">{}</a>",
            if settings_disabled { " disabled" } else { "" },
            escape(name),
            escape(if new_show_route {
                &show_path
            } else {
                &category_path
            }),
            if settings_disabled {
                escape(&t(cx.i18n, "admin.plugins.settings_disabled"))
            } else {
                String::new()
            },
            escape(&t(cx.i18n, "admin.plugins.change_settings_short"))
        )
    } else {
        String::new()
    };
    let mobile = |key: &str| {
        format!(
            "<div class=\"d-table__mobile-label\">{}</div>",
            escape(&t(cx.i18n, key))
        )
    };
    format!(
        "<tr class=\"d-table__row admin-plugins-list__row\" data-plugin-name=\"{}\" data-enabled=\"{enabled}\" data-preinstalled=\"{}\"><td class=\"d-table__cell --overview admin-plugins-list__name-details\"><div class=\"admin-plugins-list__name-with-badges\">{name_html}<div class=\"badges\">{label}</div></div><div class=\"d-table__overview-author admin-plugins-list__author\">{}</div><div class=\"d-table__overview-about admin-plugins-list__about\">{} {about_link}</div></td><td class=\"d-table__cell --detail admin-plugins-list__version\">{}<div class=\"plugin-version\">{}<br>{version_extra}</div></td><td class=\"d-table__cell --detail admin-plugins-list__enabled\">{}{switch}</td><td class=\"d-table__cell --controls admin-plugins-list__settings\">{settings_button}</td></tr>",
        escape(name),
        preinstalled(plugin),
        escape(&author(cx, plugin)),
        escape(s(&plugin["about"])),
        mobile("admin.plugins.version"),
        escape(s(&plugin["version"])),
        mobile("admin.plugins.enabled"),
    )
}

/// The DFilterControls' reset button.
fn reset_button(cx: &AdminCx, disabled: bool) -> String {
    Button {
        class: "btn-default d-filter-controls__reset",
        icon: Some("arrow-rotate-left"),
        title: tr(cx, "filter_controls.reset"),
        disabled,
        attrs: format!(
            " aria-label=\"{}\"",
            escape(&t(cx.i18n, "filter_controls.reset"))
        ),
        ..Default::default()
    }
    .render()
}

/// admin-plugins/index.gjs. The list's text filter and status come from
/// the URL as DFilterControls reads them; the script applies them.
pub fn index(cx: &AdminCx, plugins: &[Value], admin: bool, filter: &str) -> String {
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: t(cx.i18n, "admin.plugins.title"),
                path: "/admin/plugins",
            },
        ],
        title: t(cx.i18n, "admin.config.plugins.title"),
        actions: String::new(),
        description: Some(format!(
            "{}<a class=\"admin-plugins-howto\" href=\"https://meta.discourse.org/t/install-a-plugin/19157\">{}</a>",
            t(cx.i18n, "admin.config.plugins.header_description"),
            t(cx.i18n, "admin.plugins.howto")
        )),
        learn_more_url: None,
        tabs: None,
    }
    .render(cx);
    let mut out = format!("<div class=\"admin-plugins-list-container\">{header}");
    if plugins.is_empty() {
        out.push_str(&format!(
            "<p>{}</p>",
            escape(&t(cx.i18n, "admin.plugins.none_installed"))
        ));
    } else {
        let options: String = ["all", "enabled", "disabled", "preinstalled"]
            .iter()
            .map(|v| {
                format!(
                    "<option class=\"d-native-select__option{}\" value=\"{v}\"{}>{}</option>",
                    if *v == "all" { " --selected" } else { "" },
                    if *v == "all" { " selected" } else { "" },
                    escape(&t(cx.i18n, &format!("admin.plugins.filters.{v}")))
                )
            })
            .collect();
        out.push_str(&format!(
            "<div class=\"d-filter-controls\"><div class=\"d-filter-controls__inputs\"><div class=\"filter-input-container\">{}<input autocapitalize=\"none\" autocomplete=\"off\" autocorrect=\"off\" class=\"filter-input d-filter-controls__input\" placeholder=\"{}\" type=\"text\" value=\"{}\"></div></div><div class=\"d-filter-controls__dropdowns\"><select aria-label=\"{}\" class=\"d-filter-controls__dropdown d-native-select\">{options}</select></div>{}</div>",
            icon("magnifying-glass", Some("-left")),
            escape(&t(cx.i18n, "admin.plugins.filters.search_placeholder")),
            escape(filter),
            escape(&t(cx.i18n, "admin.plugins.filters.all")),
            reset_button(cx, filter.is_empty())
        ));
        out.push_str(&format!(
            "<table class=\"d-table admin-plugins-list\"><thead class=\"d-table__header\"><tr class=\"d-table__row\"><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\">{}</th><th class=\"d-table__header-cell\"></th></tr></thead><tbody class=\"d-table__body\">",
            escape(&t(cx.i18n, "admin.plugins.name")),
            escape(&t(cx.i18n, "admin.plugins.version")),
            escape(&t(cx.i18n, "admin.plugins.enabled"))
        ));
        for plugin in plugins {
            out.push_str(&row(cx, plugin, admin));
        }
        out.push_str("</tbody></table>");
    }
    out.push_str("<span></span></div>");
    out
}

/// AdminPluginConfigPage with the settings tab (AdminAreaSettings for the
/// plugin, drawn by the caller). A plugin with only its settings has no
/// tabs.
pub fn settings(cx: &AdminCx, plugin: &Value, area_settings: &str) -> String {
    settings_with_tabs(cx, plugin, area_settings, None)
}

/// `settings` with the plugin's configuration nav as the header's tabs.
pub fn settings_with_tabs(
    cx: &AdminCx,
    plugin: &Value,
    area_settings: &str,
    tabs: Option<String>,
) -> String {
    let name = s(&plugin["name"]);
    let settings_path = format!("/admin/plugins/{name}/settings");
    // AdminAreaSettings' own crumb.
    config_page(
        cx,
        plugin,
        (t(cx.i18n, "settings"), &settings_path),
        tabs,
        area_settings,
    )
}

/// AdminPluginConfigPage around one of the plugin's pages: the header with
/// the page's crumb last and the configuration nav's tabs, if any, then
/// the page in the config area.
pub fn config_page(
    cx: &AdminCx,
    plugin: &Value,
    crumb: (String, &str),
    tabs: Option<String>,
    content: &str,
) -> String {
    let titleized = name_titleized(cx, plugin);
    let name = s(&plugin["name"]);
    let show = format!("/admin/plugins/{name}");
    let link = link_url(plugin);
    let header = PageHeader {
        crumbs: vec![
            Crumb {
                label: t(cx.i18n, "admin_title"),
                path: "/admin",
            },
            Crumb {
                label: t(cx.i18n, "admin.plugins.title"),
                path: "/admin/plugins",
            },
            Crumb {
                label: titleized.clone(),
                path: &show,
            },
            Crumb {
                label: crumb.0,
                path: crumb.1,
            },
        ],
        title: titleized.clone(),
        actions: crate::plugins::admin_plugin_actions(cx, name),
        description: Some(escape(s(&plugin["about"]))),
        learn_more_url: (!link.is_empty()).then(|| link.to_string()),
        tabs,
    }
    .render(cx);
    format!(
        "<div class=\"admin-plugin-config-page\">{header}<div class=\"admin-plugin-config-page__content\"><div class=\"admin-plugin-config-page__main-area -without-inner-sidebar\"><section class=\"admin-plugin-config-area\">{content}</section></div></div></div>"
    )
}

/// The strings admin-plugins.js shows.
pub fn strings(cx: &AdminCx) -> String {
    crate::admin_ui::strings(
        cx,
        "admin-plugins-strings",
        &[
            "admin.plugins.filters.no_results",
            "filter_controls.reset",
            "generic_error",
        ],
    )
}
