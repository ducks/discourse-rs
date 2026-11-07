//! Where Ember draws a user status: DUserStatusMessage (by a poster's
//! name, in the account preferences), UserStatusBubble (on the header's
//! avatar), the user card's and the user menu's rows. Each takes the
//! status as UserStatusSerializer serializes it. static/js/user-status.js
//! opens the tooltips and draws the dates in the viewer's zone.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::emojis::Emojis;
use crate::topic_list_view::{emoji_unescape, escape};

/// The status' emoji: `emojiUnescape(":<emoji>:")`, with `skipTitle`
/// dropping its title.
pub fn emoji(emojis: &Emojis, base_path: &str, code: &str, skip_title: bool) -> String {
    let html = emoji_unescape(&format!(":{}:", escape(code)), emojis, base_path);
    if !skip_title {
        return html;
    }
    let title = format!(" title='{}'", escape(code));
    html.replacen(&title, "", 1)
}

/// The status' end as UTC milliseconds, for the scripts that draw it.
pub fn ends_at(status: &Value) -> Option<DateTime<Utc>> {
    status["ends_at"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
}

/// DUserStatusMessage: the emoji (and with `show_description`, the
/// description) in a DTooltip trigger whose content, the emoji, the
/// description and the "until", user-status.js draws on hover.
pub fn message(emojis: &Emojis, base_path: &str, status: &Value, show_description: bool) -> String {
    message_with_class(emojis, base_path, status, show_description, None)
}

/// [`message`] given a class (`...attributes`), after its own.
pub fn message_with_class(
    emojis: &Emojis,
    base_path: &str,
    status: &Value,
    show_description: bool,
    class: Option<&str>,
) -> String {
    let code = status["emoji"].as_str().unwrap_or_default();
    let description = status["description"].as_str().unwrap_or_default();
    let mut inner = emoji(emojis, base_path, code, true);
    if show_description {
        inner.push_str(&format!(
            "<span class=\"user-status-message-description\">{}</span>",
            escape(description)
        ));
    }
    let ends = ends_at(status)
        .map(|t| format!(" data-ends-at=\"{}\"", t.timestamp_millis()))
        .unwrap_or_default();
    let class = class.map(|c| format!(" {}", escape(c))).unwrap_or_default();
    format!(
        "<span aria-expanded=\"false\" class=\"fk-d-tooltip__trigger user-status-message{class}\" data-identifier=\"user-status-message-tooltip\" data-trigger=\"\" role=\"button\" data-emoji=\"{}\" data-description=\"{}\"{ends}><span class=\"fk-d-tooltip__trigger-container\">{inner}</span></span>",
        escape(code),
        escape(description),
    )
}

/// lib/user-status-message's UserStatusMessage, which chat's mentions and
/// the autocomplete put in the page themselves: the emoji (and with
/// `show_description`, the description) in a plain span, its tooltip
/// registered on it. The data attributes are what user-status.js draws the
/// tooltip from.
pub fn plain_message(
    emojis: &Emojis,
    base_path: &str,
    status: &Value,
    show_description: bool,
) -> String {
    let code = status["emoji"].as_str().unwrap_or_default();
    let description = status["description"].as_str().unwrap_or_default();
    let mut inner = emoji(emojis, base_path, code, true);
    if show_description {
        inner.push_str(&format!(
            "<span class=\"user-status-message-description\">{}</span>",
            escape(description)
        ));
    }
    let ends = ends_at(status)
        .map(|t| format!(" data-ends-at=\"{}\"", t.timestamp_millis()))
        .unwrap_or_default();
    format!(
        "<span class=\"user-status-message\" data-identifier=\"user-status-message-tooltip\" data-emoji=\"{}\" data-description=\"{}\"{ends}>{inner}</span>",
        escape(code),
        escape(description),
    )
}

/// `html` put at the end of each mention of `username` in cooked HTML
/// (`a.mention[href="<base>/u/<username lowercased>"]`).
pub fn in_mentions(cooked: &str, base_path: &str, username: &str, html: &str) -> String {
    let href = format!("{base_path}/u/{}", username.to_lowercase());
    let open = format!("<a class=\"mention\" href=\"{}\">", escape(&href));
    let mut out = cooked.to_string();
    let mut from = 0;
    while let Some(at) = out[from..].find(&open).map(|i| i + from) {
        let Some(close) = out[at..].find("</a>").map(|i| i + at) else {
            break;
        };
        out.insert_str(close, html);
        from = close + html.len() + "</a>".len();
    }
    out
}

/// UserStatusBubble: the emoji on a round background, titled with the
/// description (and, with an end, "Until" and its date, which
/// user-status.js writes in the member's timezone).
pub fn bubble(emojis: &Emojis, base_path: &str, status: &Value) -> String {
    let code = status["emoji"].as_str().unwrap_or_default();
    let description = status["description"].as_str().unwrap_or_default();
    let ends = ends_at(status)
        .map(|t| format!(" data-ends-at=\"{}\"", t.timestamp_millis()))
        .unwrap_or_default();
    // dEmoji with its title and alt=":<emoji>:".
    let img = emoji(emojis, base_path, code, true)
        .replacen(
            "<img ",
            &format!("<img title=\"{}\" ", escape(description)),
            1,
        )
        .replacen(
            &format!("alt='{}'", escape(code)),
            &format!("alt=':{}:'", escape(code)),
            1,
        );
    format!(
        "<div class=\"user-status-background\" data-description=\"{}\"{ends}>{img}</div>",
        escape(description)
    )
}

/// What static/js/user-status.js reads from its script tag: the strings
/// of the tooltips and, for a member, of UserStatusModal (with
/// DTimeShortcutPicker's tiles), the member's timezone, the emoji image
/// urls, and the scripts it loads on demand (moment for the shortcuts,
/// the emoji picker).
pub fn script_attrs(
    i18n: &crate::i18n::I18n,
    base_path: &str,
    emojis: &Emojis,
    member: Option<&str>,
) -> String {
    let t = |key: &str| i18n.t(&format!("js.{key}")).unwrap_or(key).to_string();
    let mut strings = serde_json::json!({
        "until": t("until"),
        "long_no_year_no_time": t("dates.long_no_year_no_time"),
        "long_date_without_year": t("dates.long_date_without_year"),
    });
    if let Some(timezone) = member {
        let more = serde_json::json!({
            "title": t("user_status.set_custom_status"),
            "what_are_you_doing": t("user_status.what_are_you_doing"),
            "pause_notifications": t("user_status.pause_notifications"),
            "remove_status": t("user_status.remove_status"),
            "save": t("user_status.save"),
            "cancel": t("cancel"),
            "in_one_hour": t("time_shortcut.in_one_hour"),
            "in_two_hours": t("time_shortcut.in_two_hours"),
            "tomorrow": t("time_shortcut.tomorrow"),
            "custom": t("time_shortcut.custom"),
            "never": t("time_shortcut.never"),
            "time": t("dates.time"),
            "time_short_day": t("dates.time_short_day"),
            "timezone": timezone,
            "icons": {
                "trash-can": crate::topic_list_view::icon("trash-can", None),
                "far-face-smile": crate::topic_list_view::icon("far-face-smile", None),
            },
            "moment": crate::assets::url(base_path, "moment.min.js"),
            "moment_timezone": crate::assets::url(base_path, "moment-timezone-with-data-10-year-range.min.js"),
            "emoji_picker": crate::assets::url(base_path, "emoji-picker.js"),
        });
        if let (Some(all), serde_json::Value::Object(more)) = (strings.as_object_mut(), more) {
            all.extend(more);
        }
    }
    format!(
        " data-strings=\"{}\" data-emoji-url=\"{}\" data-custom-emoji=\"{}\"",
        escape(&strings.to_string()),
        escape(&emojis.url_template(base_path)),
        escape(&emojis.custom_urls_json()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ends_at_reads_the_serializers_iso8601() {
        let status = json!({"ends_at": "2099-01-01T10:00:00Z"});
        assert_eq!(
            ends_at(&status).map(|t| t.timestamp_millis()),
            Some(4_070_944_800_000)
        );
        assert_eq!(ends_at(&json!({"ends_at": null})), None);
    }
}
