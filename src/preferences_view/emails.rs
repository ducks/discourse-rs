//! preferences/emails.gjs, with chat's preferences-chat-emails connector.

use serde_json::{Map, Value};

use super::{Combo, Prefs, checkbox, combo, save_attrs, save_controls, value_string};
use crate::AppError;
use crate::topic_list_view::{escape, t, t_count};

/// EmailsController's EMAIL_LEVELS.ONLY_WHEN_AWAY
const ONLY_WHEN_AWAY: i64 = 1;

/// What the emails tab reads beside the user.
pub struct Emails {
    /// chat_enabled: chat's connector and its saveable option.
    pub chat_enabled: bool,
}

/// `data-show-pref`: the element is there while the control `key` is
/// checked (or has `value`); static/js/preferences.js keeps it so.
fn shown_by(key: &str, value: Option<&str>, shown: bool) -> String {
    format!(
        " data-show-pref=\"{key}\"{}{}",
        value
            .map(|v| format!(" data-show-value=\"{v}\""))
            .unwrap_or_default(),
        if shown {
            ""
        } else {
            " style=\"display: none;\""
        }
    )
}

pub fn render(p: &Prefs, e: &Emails) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let mailing_list_mode =
        !p.setting("disable_mailing_list_mode")? && p.option("mailing_list_mode") == true;
    let mut out = String::new();
    if mailing_list_mode {
        out.push_str(&format!(
            "<div class=\"warning-wrap\"><div class=\"warning\">{}</div></div>",
            escape(&t(cx, "user.mailing_list_mode.warning"))
        ));
    }
    let window = p.s.get("email_time_window_mins")?.to_i();
    let frequency = (window != 0).then(|| {
        t_count(
            cx,
            "user.email.frequency",
            window,
            &[("count", &window.to_string())],
        )
    });
    let levels: Vec<(String, String)> = [(0, "always"), (1, "only_when_away"), (2, "never")]
        .iter()
        .map(|(v, k)| (v.to_string(), t(cx, &format!("user.email_level.{k}"))))
        .collect();
    let level_combo = |id: &str, key: &str| {
        combo(
            cx,
            &Combo {
                id: Some(id),
                component_class: "",
                class: "",
                rows: levels.clone(),
                value: value_string(p.option(key)),
                none: None,
                key: Some(key),
                filterable: false,
            },
        )
    };
    let instructions = |key: &str| match &frequency {
        Some(text) => format!(
            "<div class=\"instructions\"{}>{}</div>",
            shown_by(
                key,
                Some(&ONLY_WHEN_AWAY.to_string()),
                p.option(key).as_i64() == Some(ONLY_WHEN_AWAY)
            ),
            escape(text)
        ),
        None => String::new(),
    };
    out.push_str(&format!(
        "<div class=\"control-group pref-email-settings\"><label class=\"control-label\">{}</label>\
         <div class=\"controls controls-dropdown\" data-setting-name=\"user-email-messages-level\"><label for=\"user-email-messages-level\">{}</label>{}{}</div>\
         <div class=\"controls controls-dropdown\" data-setting-name=\"user-email-level\"><label for=\"user-email-level\">{}</label>{}{}</div>\
         <div class=\"controls controls-dropdown\" data-setting-name=\"user-email-previous-replies\"><label>{}</label>{}</div>{}<span>",
        escape(&t(cx, "user.email_settings")),
        escape(&t(cx, "user.email_messages_level")),
        level_combo("user-email-messages-level", "email_messages_level"),
        instructions("email_messages_level"),
        escape(&t(cx, "user.email_level.title")),
        level_combo("user-email-level", "email_level"),
        instructions("email_level"),
        escape(&t(cx, "user.email_previous_replies.title")),
        combo(
            cx,
            &Combo {
                id: None,
                component_class: "",
                class: "",
                rows: [(0, "always"), (1, "unless_emailed"), (2, "never")]
                    .iter()
                    .map(|(v, k)| {
                        (
                            v.to_string(),
                            t(cx, &format!("user.email_previous_replies.{k}")),
                        )
                    })
                    .collect(),
                value: value_string(p.option("email_previous_replies")),
                none: None,
                key: Some("email_previous_replies"),
                filterable: false,
            }
        ),
        checkbox(
            "",
            Some("user-email-in-reply-to"),
            p.option("email_in_reply_to") == true,
            &t(cx, "user.email_in_reply_to"),
            Some("email_in_reply_to")
        ),
    ));
    if e.chat_enabled {
        let when_away = p.option("chat_email_frequency").as_str() == Some("when_away");
        out.push_str(&format!(
            "<h3>{}</h3><div class=\"control-group chat-setting controls-dropdown\" data-setting-name=\"user_chat_email_frequency\"><label for=\"user_chat_email_frequency\">{}</label>{}<div class=\"control-instructions\"{}>{}</div></div>",
            escape(&t(cx, "chat.heading")),
            escape(&t(cx, "chat.email_frequency.title")),
            combo(
                cx,
                &Combo {
                    id: Some("user_chat_email_frequency"),
                    component_class: "",
                    class: "",
                    rows: ["never", "when_away"]
                        .iter()
                        .map(|v| (v.to_string(), t(cx, &format!("chat.email_frequency.{v}"))))
                        .collect(),
                    value: value_string(p.option("chat_email_frequency")),
                    none: None,
                    key: Some("chat_email_frequency"),
                    filterable: false,
                }
            ),
            shown_by("chat_email_frequency", Some("when_away"), when_away),
            escape(&t(cx, "chat.email_frequency.description")),
        ));
    }
    out.push_str("</span></div>");

    if !p.setting("disable_digest_emails")? {
        let digests = p.option("email_digests") == true;
        // @disabled={{mailing_list_mode}}
        let disabled_checkbox = |html: String| {
            html.replacen(
                "<input ",
                if p.option("mailing_list_mode") == true {
                    "<input disabled data-disabled-by=\"mailing_list_mode\" "
                } else {
                    "<input data-disabled-by=\"mailing_list_mode\" "
                },
                1,
            )
        };
        out.push_str(&format!(
            "<div class=\"control-group pref-activity-summary\"><label class=\"control-label\">{}</label>{}<div class=\"controls controls-dropdown\" data-setting-name=\"user-email-digests-frequency\"{}>{}</div>{}</div>",
            escape(&t(cx, "user.email_activity_summary")),
            disabled_checkbox(checkbox(
                "",
                Some("user-email-digests"),
                digests,
                &t(cx, "user.email_digests.title"),
                Some("email_digests")
            )),
            shown_by("email_digests", None, digests),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "",
                    rows: [
                        (30, "every_30_minutes"),
                        (60, "every_hour"),
                        (1440, "daily"),
                        (10080, "weekly"),
                        (43200, "every_month"),
                        (259200, "every_six_months"),
                    ]
                    .iter()
                    .map(|(v, k)| (v.to_string(), t(cx, &format!("user.email_digests.{k}"))))
                    .collect(),
                    value: value_string(p.option("digest_after_minutes")),
                    none: None,
                    key: Some("digest_after_minutes"),
                    filterable: true,
                }
            ),
            disabled_checkbox(checkbox(
                "",
                Some("user-include-tl0-in-digests"),
                p.option("include_tl0_in_digests") == true,
                &t(cx, "user.include_tl0_in_digests"),
                Some("include_tl0_in_digests")
            ))
            .replacen(
                "<div class=\"controls\"",
                &format!(
                    "<div class=\"controls\"{}",
                    shown_by("email_digests", None, digests)
                ),
                1
            ),
        ));
    }

    if !p.setting("disable_mailing_list_mode")? {
        let estimate = p.u["mailing_list_posts_per_day"].as_i64().unwrap_or(0);
        let estimate_text = if estimate < 2 {
            t(cx, "user.mailing_list_mode.few_per_day")
        } else {
            crate::topic_list_view::t_with(
                cx,
                "user.mailing_list_mode.many_per_day",
                &[("dailyEmailEstimate", &estimate.to_string())],
            )
        };
        out.push_str(&format!(
            "<div class=\"control-group pref-mailing-list-mode\"><label class=\"control-label\">{}</label>{}<div class=\"instructions\">{}</div><div class=\"controls controls-dropdown\" data-setting-name=\"user-mailing-list-mode-options\"{}>{}</div></div>",
            escape(&t(cx, "user.mailing_list_mode.label")),
            checkbox(
                "",
                Some("user-mailing-list-mode-enabled"),
                p.option("mailing_list_mode") == true,
                &t(cx, "user.mailing_list_mode.enabled"),
                Some("mailing_list_mode")
            ),
            t(cx, "user.mailing_list_mode.instructions"),
            shown_by("mailing_list_mode", None, p.option("mailing_list_mode") == true),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "",
                    rows: vec![
                        ("1".to_string(), estimate_text),
                        (
                            "2".to_string(),
                            t(cx, "user.mailing_list_mode.individual_no_echo")
                        ),
                    ],
                    value: value_string(p.option("mailing_list_mode_frequency")),
                    none: None,
                    key: Some("mailing_list_mode_frequency"),
                    filterable: false,
                }
            ),
        ));
    }
    out.push_str("<span></span><br><span></span>");
    out.push_str(&save_controls(cx));

    let plugin_options: Vec<&str> = if e.chat_enabled {
        vec!["chat_email_frequency"]
    } else {
        Vec::new()
    };
    let mut fields = vec![
        "email_level",
        "email_messages_level",
        "mailing_list_mode",
        "mailing_list_mode_frequency",
        "email_digests",
        "email_in_reply_to",
        "email_previous_replies",
        "digest_after_minutes",
        "include_tl0_in_digests",
    ];
    fields.extend(plugin_options.iter().copied());
    let attrs = save_attrs(p.u, &fields, &plugin_options, &[]);
    Ok((out, attrs))
}
