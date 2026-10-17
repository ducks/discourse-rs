//! The bundled plugins as Admin::PluginsController lists them: what the
//! reference's AdminPluginSerializer says of each (recorded by
//! scripts/record-admin-plugins into vendor/discourse/config/
//! admin_plugins.json), with `enabled` read from the plugin's enabled
//! setting as `Plugin::Instance#enabled?` reads it.

use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::AppError;
use crate::site_settings::SiteSettings;

const RECORDED: &str = include_str!("../vendor/discourse/config/admin_plugins.json");

/// The recorded plugins, in Discourse.plugins_sorted_by_name order.
fn recorded() -> &'static [Value] {
    static PLUGINS: OnceLock<Vec<Value>> = OnceLock::new();
    PLUGINS.get_or_init(|| serde_json::from_str(RECORDED).expect("admin_plugins.json"))
}

/// AdminPluginSerializer for a recorded plugin: its fields with `enabled`
/// after `admin_route`, as the serializer orders them, and without the
/// recording's own `config_nav`.
fn serialize(settings: &SiteSettings, plugin: &Value) -> Result<Value, AppError> {
    let enabled = match plugin["enabled_setting"].as_str() {
        Some(setting) => settings.get(setting)?.truthy(),
        None => true,
    };
    let mut out = Map::new();
    for (key, value) in plugin.as_object().into_iter().flatten() {
        if key == "config_nav" {
            continue;
        }
        // Before enabled_setting, or has_settings when there is none.
        if (key == "enabled_setting" || key == "has_settings") && !out.contains_key("enabled") {
            out.insert("enabled".into(), Value::Bool(enabled));
        }
        out.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(out))
}

/// `/admin/plugins.json`'s plugins.
pub fn all(settings: &SiteSettings) -> Result<Vec<Value>, AppError> {
    recorded().iter().map(|p| serialize(settings, p)).collect()
}

/// Admin::PluginsController#show's lookup: the plugin by name, or by the
/// name with the `discourse-` prefix.
pub fn find(settings: &SiteSettings, name: &str) -> Result<Option<Value>, AppError> {
    let found = recorded().iter().find(|p| p["name"] == name).or_else(|| {
        recorded()
            .iter()
            .find(|p| p["name"] == format!("discourse-{name}").as_str())
    });
    found.map(|p| serialize(settings, p)).transpose()
}

/// The admin page's location of an enabled plugin with one, by name
/// (AdminSearchDataSource#buildPluginsMap's plugins).
pub fn admin_route_location(settings: &SiteSettings, name: &str) -> Option<String> {
    let plugin = recorded().iter().find(|p| p["name"] == name)?;
    let location = plugin["admin_route"]["location"].as_str()?;
    let enabled = match plugin["enabled_setting"].as_str() {
        Some(setting) => settings.get(setting).is_ok_and(|v| v.truthy()),
        None => true,
    };
    enabled.then(|| location.to_string())
}

/// `Plugin::Instance#humanized_name` of a bundled plugin, by name.
pub fn humanized_name(name: &str) -> Option<String> {
    recorded()
        .iter()
        .find(|p| p["name"] == name)
        .and_then(|p| p["humanized_name"].as_str())
        .map(str::to_string)
}

/// Whether the plugin's admin initializers add a configuration nav, whose
/// pages are the plugin's own.
pub fn has_config_nav(id: &str) -> bool {
    recorded()
        .iter()
        .any(|p| p["id"] == id && p["config_nav"].as_bool().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recording_is_the_reference_list() {
        let plugins = recorded();
        assert_eq!(plugins.len(), 46);
        assert_eq!(plugins[0]["id"], "discourse-adplugin");
        assert!(has_config_nav("chat"));
        assert!(!has_config_nav("poll"));
    }
}
