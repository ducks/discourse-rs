//! discourse-math: `$...$`, `$$...$$`, `\(...\)` and `\[...\]` (and
//! `%...%` asciimath) cooked as `span.math`, `div.math` and
//! `span.asciimath` by the markdown crate (render/math.rs), typeset in the
//! browser by static/js/discourse-math.js with MathJax or KaTeX
//! (`discourse_math_provider`), and the composer's Insert Math item.
//!
//! The renderers are the discourse_math_bundle gem's (1.0.1: MathJax
//! 4.1.0, KaTeX 0.16.27), vendored verbatim under
//! static/vendor/discourse-math-bundle and embedded by build.rs. Rails
//! symlinks them into the plugin's public/<version> and serves them as
//! /plugins/discourse-math/<version>/{mathjax,katex}/...; `serve` answers
//! those paths.

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

include!(concat!(env!("OUT_DIR"), "/math_bundle.rs"));

/// `DiscourseMathBundle::VERSION`, the gem the files come from.
pub const BUNDLE_VERSION: &str = "1.0.1";

/// `DiscourseMath::BundlePaths.public_url_base`, the site serializer's
/// `discourse_math_bundle_url` (no base path, as Rails gives it).
pub fn public_url_base() -> String {
    format!("/plugins/discourse-math/{BUNDLE_VERSION}")
}

/// A vendored bundle file by its path under the version
/// (`mathjax/tex-mml-chtml.js`, `katex/katex.min.css`).
pub fn bundle_file(path: &str) -> Option<&'static [u8]> {
    MATH_BUNDLE
        .binary_search_by(|(name, _)| (*name).cmp(path))
        .ok()
        .map(|i| MATH_BUNDLE[i].1)
}

/// The content type for a bundle file, as nginx's mime.types gives it.
fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

/// GET /plugins/discourse-math/<version>/<path>: the plugin's public
/// files, served whether or not the plugin is on, as Rails links them at
/// boot. nginx.sample.conf's `^/plugins/` location caches them for a year
/// (public, immutable) with `Access-Control-Allow-Origin: *`.
pub async fn serve(Path(path): Path<String>) -> Response {
    let Some(body) = bundle_file(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (header::CONTENT_TYPE, content_type(&path)),
            (header::CACHE_CONTROL, "max-age=31556952, public, immutable"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        body,
    )
        .into_response()
}

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
/// buildDiscourseMathOptions reads, the bundle's url (the site's
/// `discourse_math_bundle_url` under the base path, as getURLWithCDN
/// gives it) and the modal's strings. Empty while the plugin is off,
/// which leaves the script out.
pub fn script_attrs(
    settings: &SiteSettings,
    i18n: &I18n,
    base_path: &str,
) -> Result<String, SettingError> {
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
         data-zoom-on-click=\"{}\" data-bundle-url=\"{}\" data-strings=\"{}\"",
        esc(&settings.get("discourse_math_provider")?.to_s()),
        esc(&settings.get("discourse_math_mathjax_output")?.to_s()),
        flag("discourse_math_enable_menu")?,
        flag("discourse_math_enable_asciimath")?,
        flag("discourse_math_enable_accessibility")?,
        flag("discourse_math_zoom_on_click")?,
        esc(&format!("{base_path}{}", public_url_base())),
        esc(&Value::Object(strings).to_string()),
    ))
}
