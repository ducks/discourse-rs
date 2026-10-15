//! preferences/notifications.gjs, with PushNotificationSelect,
//! UserNotificationSchedule and the connectors of discourse-solved
//! (solved-notifications) and chat (chat-notifications).

use serde_json::{Map, Value};

use super::{Combo, Prefs, button, checkbox, combo, save_attrs, save_controls, value_string};
use crate::AppError;
use crate::topic_list_view::{ListContext, escape, t};

/// What the notifications tab reads beside the user.
pub struct Notifications {
    /// discourse-solved's SolvedNotifications.shouldRender.
    pub solved: bool,
    /// chat's ChatNotifications.shouldRender and its saveable options.
    pub chat: bool,
    /// chat_enabled (the site's), for PushNotificationSelect's chat_only.
    pub chat_enabled: bool,
}

const DAYS: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

/// UserNotificationSchedule#buildTimeOptions: every half hour from
/// `start_at` as moment's `LT` writes it in English, Midnight at 1440.
pub(crate) fn time_options(
    cx: &ListContext,
    start_at: i64,
    include_none: bool,
    show_midnight: bool,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if include_none {
        out.push(("-1".to_string(), t(cx, "user.notification_schedule.none")));
    }
    let mut minutes = start_at;
    while minutes <= 1440 {
        let hours = minutes / 60;
        if hours == 24 {
            if show_midnight {
                out.push((
                    "1440".to_string(),
                    t(cx, "user.notification_schedule.midnight"),
                ));
            }
            break;
        }
        let twelve = if hours % 12 == 0 { 12 } else { hours % 12 };
        out.push((
            minutes.to_string(),
            format!(
                "{twelve}:{:02} {}",
                minutes % 60,
                if hours < 12 { "AM" } else { "PM" }
            ),
        ));
        minutes += 30;
    }
    out
}

/// preferences/notifications.gjs
pub fn render(p: &Prefs, n: &Notifications) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let u = p.u;
    let mut out = format!(
        "<div class=\"control-group notifications\"><label class=\"control-label\">{}</label><div class=\"controls controls-dropdown\" data-setting-name=\"user-like-notification-frequency\"><label>{}</label>{}</div>{}</div>",
        escape(&t(cx, "user.notifications")),
        escape(&t(cx, "user.like_notification_frequency.title")),
        combo(
            cx,
            &Combo {
                id: None,
                component_class: "",
                class: "",
                rows: [
                    (0, "always"),
                    (1, "first_time_and_daily"),
                    (2, "first_time"),
                    (3, "never")
                ]
                .iter()
                .map(|(v, k)| {
                    (
                        v.to_string(),
                        t(cx, &format!("user.like_notification_frequency.{k}")),
                    )
                })
                .collect(),
                value: value_string(p.option("like_notification_frequency")),
                none: None,
                key: Some("like_notification_frequency"),
                filterable: false,
            }
        ),
        checkbox(
            "pref-notify-on-linked-posts",
            Some("user-notify-on-linked-posts"),
            p.option("notify_on_linked_posts") == true,
            &t(cx, "user.notify_on_linked_posts"),
            Some("notify_on_linked_posts")
        )
    );

    // PushNotificationSelect: the push levels; static/js/preferences.js
    // swaps in the not-supported or denied button from the browser's
    // permission, and subscribing (DesktopNotifications#enable) is 186's.
    let mut levels = vec![
        (
            "none".to_string(),
            t(cx, "user.desktop_notifications.push_level.none"),
        ),
        (
            "all".to_string(),
            t(cx, "user.desktop_notifications.push_level.all"),
        ),
    ];
    let chat_available = n.chat_enabled && p.option("chat_enabled") == true;
    if chat_available {
        levels.push((
            "chat_only".to_string(),
            t(cx, "user.desktop_notifications.push_level.chat_only"),
        ));
    }
    out.push_str(&format!(
        "<div class=\"control-group desktop-notifications\" data-setting-name=\"user-desktop-notifications\"><div class=\"push-notification-config\" data-labels=\"{}\" data-unported=\"subscribing to push notifications (186)\"><label class=\"sr-only\">{}</label>{}</div><div class=\"instructions\">{}</div><span></span></div>",
        escape(
            &serde_json::json!({
                "not_supported": t(cx, "user.desktop_notifications.not_supported"),
                "perm_denied_btn": t(cx, "user.desktop_notifications.perm_denied_btn"),
                "perm_denied_expl": t(cx, "user.desktop_notifications.perm_denied_expl"),
            })
            .to_string()
        ),
        escape(&t(cx, "user.desktop_notifications.push_level.title")),
        combo(
            cx,
            &Combo {
                id: None,
                component_class: "",
                class: "push-notification-select",
                rows: levels,
                // `value`: none until the browser is subscribed.
                value: Some("none".to_string()),
                none: None,
                key: None,
                filterable: false,
            }
        ),
        escape(&t(cx, "user.desktop_notifications.each_browser_note"))
    ).replacen(
        "<div class=\"control-group desktop-notifications\" data-setting-name=\"user-desktop-notifications\">",
        &format!(
            "<div class=\"control-group desktop-notifications\" data-setting-name=\"user-desktop-notifications\"><label class=\"control-label\">{}</label>",
            escape(&t(cx, "user.desktop_notifications.label"))
        ),
        1,
    ));

    if u["admin"] == true {
        out.push_str(&format!(
            "<div class=\"control-group upcoming-changes\"><label class=\"control-label\">{}</label>{}</div>",
            escape(&t(cx, "user.upcoming_changes.title")),
            checkbox(
                "pref-enable-upcoming-change-available-notifications",
                Some("user-enable-upcoming-change-available-notifications"),
                p.option("enable_upcoming_change_available_notifications") == true,
                &t(cx, "user.upcoming_changes.enable_available_notifications"),
                Some("enable_upcoming_change_available_notifications")
            )
        ));
    }

    // UserNotificationSchedule
    let schedule = &u["user_notification_schedule"];
    let enabled = schedule["enabled"] == true;
    out.push_str(&format!(
        "<div class=\"control-group notification-schedule\" data-setting-name=\"user-notification-schedule\" data-midnight=\"{}\" data-to=\"{}\"><label class=\"control-label\">{}</label>{}",
        escape(&t(cx, "user.notification_schedule.midnight")),
        escape(&t(cx, "user.notification_schedule.to")),
        escape(&t(cx, "user.notification_schedule.title")),
        checkbox(
            "",
            None,
            enabled,
            &t(cx, "user.notification_schedule.label"),
            Some("user_notification_schedule.enabled")
        )
    ));
    out.push_str(&format!(
        "<div class=\"notification-schedule-body\"{}><div class=\"instruction\">{}</div><table class=\"notification-schedule-table\"><tbody class=\"notification-schedule-tbody\">",
        if enabled { "" } else { " hidden" },
        escape(&t(cx, "user.notification_schedule.tip"))
    ));
    let starts = time_options(cx, 0, true, false);
    for (i, day) in DAYS.iter().enumerate() {
        let label = t(cx, &format!("user.notification_schedule.{day}"));
        let start = schedule[format!("day_{i}_start_time")]
            .as_i64()
            .unwrap_or(-1);
        let end = schedule[format!("day_{i}_end_time")].as_i64();
        out.push_str(&format!(
            "<tr class=\"day {l}\"><td class=\"day-label\">{l}</td><td class=\"starts-at\">{}</td>",
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "",
                    rows: starts.clone(),
                    value: Some(start.to_string()),
                    none: None,
                    key: Some(&format!("user_notification_schedule.day_{i}_start_time")),
                    filterable: false,
                }
            ),
            l = escape(&label)
        ));
        if start != -1 {
            out.push_str(&format!(
                "<td class=\"to\">{}</td><td class=\"ends-at\">{}</td>",
                escape(&t(cx, "user.notification_schedule.to")),
                combo(
                    cx,
                    &Combo {
                        id: None,
                        component_class: "",
                        class: "",
                        rows: time_options(cx, start + 30, false, true),
                        value: end.map(|e| e.to_string()),
                        none: None,
                        key: Some(&format!("user_notification_schedule.day_{i}_end_time")),
                        filterable: false,
                    }
                )
            ));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div></div>");

    // The user-preferences-notifications outlet.
    out.push_str("<span>");
    if n.solved {
        out.push_str(&format!(
            "<div class=\"control-group solved-notifications\"><label class=\"control-label\">{}</label>{}</div>",
            escape(&t(cx, "solved.title")),
            checkbox(
                "pref-notify-on-solved",
                Some("user-notify-on-solved"),
                p.option("notify_on_solved") == true,
                &t(cx, "solved.notify_on_solved"),
                Some("notify_on_solved")
            )
        ));
    }
    if n.chat {
        let sound = p
            .option("chat_sound")
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| {
                if ["classic", "soft", "retro", "bubble", "ethereal"].contains(&s) {
                    s.to_string()
                } else {
                    "classic".to_string()
                }
            });
        let sounds = ["classic", "soft", "retro", "bubble", "ethereal"]
            .iter()
            .map(|v| (v.to_string(), t(cx, &format!("chat.sounds.{v}"))))
            .collect();
        let preview = if sound.is_some() {
            button(
                "btn-flat chat-sound-preview",
                Some("play"),
                None,
                &format!(
                    " title=\"{}\" data-unported=\"chat's notification sounds (162)\"",
                    escape(&t(cx, "chat.sound.preview"))
                ),
            )
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<div class=\"control-group chat-notifications\"><label class=\"control-label\">{}</label>{}<div class=\"controls controls-dropdown\"><label>{}</label><div class=\"chat-sound-controls\">{}{preview}</div></div><div class=\"controls controls-dropdown\"><label>{}</label>{}</div></div>",
            escape(&t(cx, "chat.chat_notifications_title")),
            checkbox(
                "pref-chat-ignore-channel-wide-mention",
                Some("chat-ignore-channel-wide-mention"),
                p.option("ignore_channel_wide_mention") == true,
                &t(cx, "chat.ignore_channel_wide_mention.title"),
                Some("ignore_channel_wide_mention")
            ),
            escape(&t(cx, "chat.sound.title")),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "chat-sound",
                    rows: sounds,
                    value: sound,
                    none: Some(t(cx, "chat.sounds.none")),
                    key: Some("chat_sound"),
                    filterable: false,
                }
            ),
            escape(&t(cx, "chat.header_indicator_preference.title")),
            combo(
                cx,
                &Combo {
                    id: None,
                    component_class: "",
                    class: "chat-header-indicator-preference",
                    rows: ["all_new", "dm_and_mentions", "only_mentions", "never"]
                        .iter()
                        .map(|v| (
                            v.to_string(),
                            t(cx, &format!("chat.header_indicator_preference.{v}"))
                        ))
                        .collect(),
                    value: value_string(p.option("chat_header_indicator_preference")),
                    none: None,
                    key: Some("chat_header_indicator_preference"),
                    filterable: false,
                }
            ),
        ));
    }
    out.push_str("</span><br><span></span>");
    out.push_str(&save_controls(cx));

    // addSaveableUserOption: solved's and chat's notifications options.
    let mut plugin_options: Vec<&str> = Vec::new();
    if p.setting("solved_enabled")? {
        plugin_options.push("notify_on_solved");
    }
    if n.chat_enabled {
        plugin_options.extend([
            "chat_header_indicator_preference",
            "chat_sound",
            "ignore_channel_wide_mention",
        ]);
    }
    let mut fields = vec![
        "allow_private_messages",
        "auto_track_topics_after_msecs",
        "enable_allowed_pm_users",
        "like_notification_frequency",
        "muted_usernames",
        "new_topic_duration_minutes",
        "notification_level_when_replying",
        "notify_on_linked_posts",
        "push_notification_level",
        "enable_upcoming_change_available_notifications",
        "user_notification_schedule",
    ];
    fields.extend(plugin_options.iter().copied());
    let attrs = save_attrs(u, &fields, &plugin_options, &[]);
    Ok((out, attrs))
}
