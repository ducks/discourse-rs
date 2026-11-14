//! What static/js/desktop-notifications.js reads: lib/desktop-notifications
//! and the desktop-notifications service's strings and site settings.

use serde_json::json;

use crate::i18n::I18n;

/// The script's attributes, for a member: `notifications.popup.*` by
/// type name with `Site#notificationLookup` (`Notification.types`
/// inverted), the title and icon a notification carries
/// (`site_logo_small_url || site_logo_url`), NotificationConsentBanner's
/// strings and its `push_notifications_prompt`.
pub fn script_attrs(
    i18n: &I18n,
    base_path: &str,
    site_title: &str,
    logo_small_url: &str,
    logo_url: &str,
    prompt: bool,
) -> String {
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
    let lookup: serde_json::Map<String, serde_json::Value> = super::TYPES
        .iter()
        .map(|(name, id)| (id.to_string(), json!(name)))
        .collect();
    let config = json!({
        "popup": i18n.subtree("js.notifications.popup").unwrap_or_else(|| json!({})),
        "lookup": lookup,
        "base": base_path,
        "title": site_title,
        "icon": if logo_small_url.is_empty() { logo_url } else { logo_small_url },
        "prompt": prompt,
        "consent_prompt": t("user.desktop_notifications.consent_prompt"),
        "enable": t("user.desktop_notifications.enable"),
        "close": t("banner.close"),
        "xmark": crate::topic_list_view::icon("xmark", None),
    });
    format!(
        " data-config=\"{}\"",
        crate::topic_list_view::escape(&config.to_string())
    )
}
