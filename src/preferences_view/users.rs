//! preferences/users.gjs: IgnoredUserList (its Add… opening the ignore
//! duration modal of static/js/prefs-choosers.js), the muted users'
//! UserChooser and who may send messages.

use serde_json::{Map, Value};

use super::tracking::multi_select;
use super::{Prefs, button, checkbox, save_attrs, save_controls};
use crate::AppError;
use crate::topic_list_view::{escape, icon, t};

fn names(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(|n| (n.to_string(), n.to_string())))
        .collect()
}

/// What static/js/prefs-choosers.js says and reads: select-kit's strings,
/// the tag search's settings, and for the ignore duration modal its
/// strings, the viewer's timezone, the time shortcuts and moment.js.
pub fn choosers_strings(
    cx: &crate::topic_list_view::ListContext,
    s: &crate::site_settings::SiteSettings,
    current_username: &str,
    timezone: Option<&str>,
    moment: (&str, &str),
) -> Result<String, AppError> {
    use serde_json::json;
    let shortcut = |k: &str| t(cx, &format!("time_shortcut.{k}"));
    let strings = json!({
        "default_header": t(cx, "select_kit.default_header_text"),
        "filter_placeholder": t(cx, "select_kit.filter_placeholder"),
        "tags_placeholder": t(cx, "tagging.choose_for_topic"),
        "results_one": crate::topic_list_view::t_count(cx, "select_kit.results_count", 1, &[("count", "1")]),
        "results_other": t(cx, "select_kit.results_count.other"),
        "delete_item": t(cx, "select_kit.delete_item"),
        "filter_by": t(cx, "select_kit.filter_by"),
        "max_tag_search_results": s.get("max_tag_search_results")?.to_i(),
        "tags_sort_alphabetically": s.get("tags_sort_alphabetically")?.truthy(),
        "tag_style": s.get("tag_style")?.to_s(),
        "current_username": current_username,
        "moment": moment.0,
        "moment_timezone": moment.1,
        "timezone": timezone,
        "suggest_weekends": s.get("suggest_weekends_in_date_pickers")?.truthy(),
        "select_timeframe": shortcut("select_timeframe"),
        "time_shortcut": {
            "later_today": shortcut("later_today"),
            "tomorrow": shortcut("tomorrow"),
            "later_this_week": shortcut("later_this_week"),
            "this_weekend": shortcut("this_weekend"),
            "start_of_next_business_week": shortcut("start_of_next_business_week"),
            "start_of_next_business_week_alt": shortcut("start_of_next_business_week_alt"),
            "two_weeks": shortcut("two_weeks"),
            "next_month": shortcut("next_month"),
            "two_months": shortcut("two_months"),
            "three_months": shortcut("three_months"),
            "four_months": shortcut("four_months"),
            "six_months": shortcut("six_months"),
            "one_year": shortcut("one_year"),
            "forever": shortcut("forever"),
        },
        "dates": {
            "time": t(cx, "dates.time"),
            "time_short_day": t(cx, "dates.time_short_day"),
            "long_no_year": t(cx, "dates.long_no_year"),
            "long_with_year": t(cx, "dates.long_with_year"),
        },
        "ignore_title": t(cx, "user.user_notifications.ignore_duration_title"),
        "ignore_username": t(cx, "user.user_notifications.ignore_duration_username"),
        "ignore_when": t(cx, "user.user_notifications.ignore_duration_when"),
        "ignore_note": t(cx, "user.user_notifications.ignore_duration_note"),
        "ignore_save": t(cx, "user.user_notifications.ignore_duration_save"),
        "ignore_required": t(cx, "user.user_notifications.ignore_duration_time_frame_required"),
    });
    Ok(format!(
        "<script type=\"application/json\" id=\"prefs-choosers-strings\">{}</script>",
        strings.to_string().replace('<', "\\u003c")
    ))
}

/// IgnoredUserListItem
pub(crate) fn ignored_item(username: &str) -> String {
    format!(
        "<div><div class=\"ignored-user-list-item\"><span class=\"ignored-user-name\">{}</span>{}</div></div>",
        escape(username),
        button("remove-ignored-user", Some("xmark"), None, "")
    )
}

pub fn render(p: &Prefs) -> Result<(String, Map<String, Value>), AppError> {
    let cx = p.cx;
    let u = p.u;
    let mut out = format!(
        "<label class=\"control-label\">{}</label>",
        escape(&t(cx, "user.users"))
    );
    if u["can_ignore_users"] == true {
        let items: String = u["ignored_usernames"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(ignored_item)
            .collect();
        out.push_str(&format!(
            "<div class=\"control-group user-ignore\" data-setting-name=\"user-ignored-users\"><div class=\"controls tracking-controls user-notifications\"><label>{} {}</label><div><div class=\"ignored-list\" data-empty=\"{}\">{}</div><div class=\"instructions\">{}</div><div>{}</div></div></div></div>",
            icon("far-eye-slash", Some("icon")),
            escape(&t(cx, "user.ignored_users")),
            escape(&t(cx, "user.user_notifications.ignore_no_users")),
            if items.is_empty() {
                escape(&t(cx, "user.user_notifications.ignore_no_users"))
            } else {
                items
            },
            escape(&t(cx, "user.ignored_users_instructions")),
            button(
                "btn-default add-ignored-user",
                Some("plus"),
                Some(&t(cx, "user.user_notifications.add_ignored_user")),
                ""
            ),
        ));
    }
    if u["can_mute_users"] == true {
        out.push_str(&format!(
            "<div class=\"control-group user-mute\" data-setting-name=\"user-muted-users\"><div class=\"controls tracking-controls\"><label>{} <span>{}</span></label>{}</div><div class=\"instructions\">{}</div></div>",
            crate::post_view::d_icon("d-muted", Some("icon")),
            escape(&t(cx, "user.muted_users")),
            multi_select(cx, "user-chooser", "user", "muted_usernames", &names(&u["muted_usernames"])),
            escape(&t(cx, "user.muted_users_instructions")),
        ));
    }
    if u["can_send_private_messages"] == true {
        // disableAllowPmUsersSetting: off while messages are refused.
        let allow = p.option("allow_private_messages") == true;
        let specific = p.option("enable_allowed_pm_users") == true;
        let mut specific_box = checkbox(
            "",
            None,
            specific,
            &t(cx, "user.allow_private_messages_from_specific_users"),
            Some("enable_allowed_pm_users"),
        )
        .replacen(
            "<input ",
            "<input data-disabled-unless=\"allow_private_messages\" ",
            1,
        );
        if !allow {
            specific_box = specific_box.replacen("<input ", "<input disabled ", 1);
        }
        out.push_str(&format!(
            "<div class=\"control-group private-messages\"><label class=\"control-label\">{}</label><div class=\"control-group user-allow-pm\" data-setting-name=\"user-allow-private-messages\"><div class=\"controls\">{}</div></div><div class=\"control-group user-allow-pm\" data-setting-name=\"user-allow-private-messages-from-specific-users\"><div class=\"controls\">{}</div><div class=\"controls tracking-controls\" data-show-pref=\"enable_allowed_pm_users\" data-show-also=\"allow_private_messages\"{}>{}</div><div class=\"instructions\" data-show-pref=\"enable_allowed_pm_users\" data-show-also=\"allow_private_messages\"{}>{}</div></div></div>",
            escape(&t(cx, "user.private_messages")),
            checkbox(
                "",
                None,
                allow,
                &t(cx, "user.allow_private_messages"),
                Some("allow_private_messages")
            ),
            specific_box,
            if allow && specific { "" } else { " style=\"display: none;\"" },
            multi_select(
                cx,
                "user-chooser",
                "user",
                "allowed_pm_usernames",
                &names(&u["allowed_pm_usernames"])
            ),
            if allow && specific { "" } else { " style=\"display: none;\"" },
            escape(&t(cx, "user.allowed_pm_users_instructions")),
        ));
    }
    out.push_str("<span></span>");
    out.push_str(&save_controls(cx));
    let attrs = save_attrs(
        u,
        &[
            "muted_usernames",
            "allowed_pm_usernames",
            "enable_allowed_pm_users",
            "allow_private_messages",
        ],
        &[],
        &[],
    );
    Ok((out, attrs))
}
