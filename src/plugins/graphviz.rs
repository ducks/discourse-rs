//! discourse-graphviz: `[graphviz]` blocks cooked as a placeholder with
//! the DOT source and its engine by the markdown crate (render/bbcode.rs),
//! drawn in the browser by static/js/graphviz.js with the plugin's viz.js,
//! and the composer's Insert Graphviz graph item.

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

/// Where Rails serves the plugin's viz.js (its public/javascripts),
/// embedded here (static/vendor/viz).
pub const VIZ_PATH: &str = "/plugins/discourse-graphviz/javascripts/viz-3.0.1.js";

/// `enabled_site_setting :discourse_graphviz_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_graphviz_enabled")?.truthy())
}

/// graphviz.js's script tag attributes: viz.js's path, the composer's
/// sample graph (`composer.graphviz_sample`) and the fullscreen button's
/// title. Empty while the plugin is off, which leaves the script out.
pub fn script_attrs(
    settings: &SiteSettings,
    i18n: &I18n,
    base_path: &str,
) -> Result<String, SettingError> {
    if !enabled(settings)? {
        return Ok(String::new());
    }
    let esc = crate::topic_list_view::escape;
    Ok(format!(
        " data-viz=\"{}{VIZ_PATH}\" data-sample=\"{}\" data-fullscreen=\"{}\"",
        esc(base_path),
        esc(i18n.t("js.composer.graphviz_sample").unwrap_or("")),
        esc(i18n.t("js.graphviz.fullscreen").unwrap_or("")),
    ))
}
