//! discourse-math: `$...$`, `$$...$$`, `\(...\)` and `\[...\]` (and
//! `%...%` asciimath) cooked as `span.math`, `div.math` and
//! `span.asciimath` by the markdown crate (render/math.rs), typeset in the
//! browser by static/js/discourse-math.js with MathJax or KaTeX
//! (`discourse_math_provider`), and the composer's Insert Math item.
//!
//! Rails serves the renderers from the discourse_math_bundle gem under
//! /plugins/discourse-math/<version>; the port does not vendor them (the
//! MathJax bundle loads its components and fonts by path, a directory
//! tree, not a file), so the browser takes them from a CDN at the
//! versions below.

use serde_json::Value;

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

/// Where MathJax's components load from (its `loader.paths.mathjax`):
/// MathJax 4.1, which the plugin names.
pub const MATHJAX_URL: &str = "https://cdn.jsdelivr.net/npm/mathjax@4.1.0";
/// KaTeX's dist directory (katex.min.js, katex.min.css, contrib/).
pub const KATEX_URL: &str = "https://cdn.jsdelivr.net/npm/katex@0.16.22/dist";

/// The client strings the insert modal shows, by their js.* key.
const STRINGS: &[&str] = &[
    "discourse_math.insert_modal.title_inline",
    "discourse_math.insert_modal.title_block",
    "discourse_math.insert_modal.label",
    "discourse_math.insert_modal.placeholder",
    "discourse_math.insert_modal.insert",
    "discourse_math.insert_modal.block_mode",
    "cancel",
    "form_kit.errors.required",
];

/// `enabled_site_setting :discourse_math_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_math_enabled")?.truthy())
}

/// discourse-math.js's script tag attributes: the siteSettings
/// buildDiscourseMathOptions reads, where the renderers load from and the
/// modal's strings. Empty while the plugin is off, which leaves the
/// script out.
pub fn script_attrs(settings: &SiteSettings, i18n: &I18n) -> Result<String, SettingError> {
    if !enabled(settings)? {
        return Ok(String::new());
    }
    let esc = crate::topic_list_view::escape;
    let flag = |name: &str| -> Result<&'static str, SettingError> {
        Ok(if settings.get(name)?.truthy() {
            "true"
        } else {
            "false"
        })
    };
    let strings: serde_json::Map<String, Value> = STRINGS
        .iter()
        .map(|key| {
            let text = i18n.t(&format!("js.{key}")).unwrap_or(key);
            ((*key).to_string(), Value::String(text.to_string()))
        })
        .collect();
    Ok(format!(
        " data-provider=\"{}\" data-mathjax-output=\"{}\" data-enable-menu=\"{}\" \
         data-enable-asciimath=\"{}\" data-enable-accessibility=\"{}\" \
         data-zoom-on-click=\"{}\" data-mathjax-url=\"{MATHJAX_URL}\" \
         data-katex-url=\"{KATEX_URL}\" data-strings=\"{}\"",
        esc(&settings.get("discourse_math_provider")?.to_s()),
        esc(&settings.get("discourse_math_mathjax_output")?.to_s()),
        flag("discourse_math_enable_menu")?,
        flag("discourse_math_enable_asciimath")?,
        flag("discourse_math_enable_accessibility")?,
        flag("discourse_math_zoom_on_click")?,
        esc(&Value::Object(strings).to_string()),
    ))
}
