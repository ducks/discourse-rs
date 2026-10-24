//! What static/js/penalize-user.js needs for the admin's PenalizeUser modal
//! (admin/components/modal/penalize-user.gjs), as adminTools shows it from
//! the admin user page and the flag modal's take action: the strings as
//! `i18n` resolves them for staff (count keys as their `one` and `other`
//! forms, MessageFormat keys raw), the settings it reads, the member's
//! timezone the time shortcuts are computed in, the date libraries it
//! loads and, on the admin user page, the AdminUser it penalizes.

use serde_json::{Map, Value, json};

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;

const KEYS: &[&str] = &[
    "generic_error",
    "generic_error_with_reason",
    "multiple_errors",
    "cancel",
    "username",
    "last_seen",
    "created",
    "admin.user.suspend_modal_title",
    "admin.user.silence_modal_title",
    "admin.user.suspend",
    "admin.user.silence",
    "admin.user.penalty_history_MF",
    "admin.user.suspend_duration",
    "admin.user.silence_duration",
    "admin.user.suspend_reason_label",
    "admin.user.suspend_reason_hidden_label",
    "admin.user.suspend_reason_title",
    "admin.user.silence_reason_label",
    "admin.user.silence_reason_placeholder",
    "admin.user.suspend_reasons.not_listening_to_staff",
    "admin.user.suspend_reasons.consuming_staff_time",
    "admin.user.suspend_reasons.combative",
    "admin.user.suspend_reasons.in_wrong_place",
    "admin.user.suspend_reasons.no_constructive_purpose",
    "admin.user.suspend_reasons.custom",
    "admin.user.suspend_message",
    "admin.user.suspend_message_placeholder",
    "admin.user.penalty_post_actions",
    "admin.user.penalty_post_delete",
    "admin.user.penalty_post_delete_all",
    "admin.user.penalty_post_delete_replies",
    "admin.user.penalty_post_edit",
    "admin.user.penalty_post_none",
    "admin.user.penalty_post_delete_all_confirmation_MF",
    "admin.user.topics_entered",
    "admin.user.posts_read_count",
    "admin.user.time_read",
    "admin.user.cant_suspend",
    "admin.user.cant_silence",
    "admin.user.confirm_cancel_penalty",
    "time_shortcut.later_today",
    "time_shortcut.tomorrow",
    "time_shortcut.later_this_week",
    "time_shortcut.start_of_next_business_week",
    "time_shortcut.start_of_next_business_week_alt",
    "time_shortcut.two_weeks",
    "time_shortcut.next_month",
    "time_shortcut.two_months",
    "time_shortcut.three_months",
    "time_shortcut.four_months",
    "time_shortcut.six_months",
    "time_shortcut.one_year",
    "time_shortcut.forever",
    "time_shortcut.custom",
    "time_shortcut.select_timeframe",
    "dates.time",
    "dates.time_short_day",
    "dates.long_no_year",
    "dates.long_with_year",
    "dates.placeholder",
    "dates.previous_month",
    "dates.next_month",
    "select_kit.filter_by",
    "select_kit.filter_placeholder",
];

/// Keys with `one` and `other` forms.
const COUNT_KEYS: &[&str] = &[
    "admin.user.other_matches",
    "dates.tiny.less_than_x_minutes",
    "dates.tiny.x_minutes",
    "dates.tiny.about_x_hours",
    "dates.tiny.x_days",
    "dates.tiny.x_months",
    "dates.tiny.about_x_years",
    "dates.tiny.over_x_years",
    "dates.tiny.almost_x_years",
];

/// The fields of AdminUser the modal reads.
const USER_FIELDS: &[&str] = &[
    "id",
    "username",
    "admin",
    "moderator",
    "next_penalty",
    "penalty_counts",
    "similar_users_count",
    "trust_level",
    "topic_count",
    "post_count",
];

pub struct Assets<'a> {
    pub base_path: &'a str,
}

/// The `#penalize-user-config` block. `user` is the admin user's JSON
/// (AdminDetailedUserSerializer) on the admin user page; the flag modal
/// finds the user when it penalizes.
pub fn config(
    i18n: &I18n,
    settings: &SiteSettings,
    assets: &Assets<'_>,
    timezone: Option<&str>,
    user: Option<&Value>,
) -> String {
    let mut strings = Map::new();
    for key in KEYS {
        strings.insert(key.to_string(), json!(crate::admin_view::t(i18n, key)));
    }
    for key in COUNT_KEYS {
        for form in ["one", "other"] {
            let full = format!("{key}.{form}");
            strings.insert(full.clone(), json!(crate::admin_view::t(i18n, &full)));
        }
    }
    let setting = |name: &str| settings.get(name).map(|v| v.truthy()).unwrap_or(false);
    let user = user.map(|u| {
        Value::Object(
            USER_FIELDS
                .iter()
                .map(|f| (f.to_string(), u[*f].clone()))
                .collect(),
        )
    });
    let config = json!({
        "strings": strings,
        "settings": {
            "penalty_include_post_message": setting("penalty_include_post_message"),
            "hide_suspension_reasons": setting("hide_suspension_reasons"),
        },
        "timezone": timezone.filter(|z| !z.is_empty()),
        "mobile": crate::mobile::mobile_view(),
        "assets": {
            "moment": crate::assets::url(assets.base_path, "moment.min.js"),
            "moment_timezone": crate::assets::url(assets.base_path, "moment-timezone-with-data-10-year-range.min.js"),
            "pikaday": crate::assets::url(assets.base_path, "pikaday.js"),
        },
        "user": user,
    });
    format!(
        "<script type=\"application/json\" id=\"penalize-user-config\">{}</script>",
        config.to_string().replace('<', "\\u003c")
    )
}

/// `user_options.timezone` of the member viewing the page.
pub async fn viewer_timezone(
    conn: &mut sqlx::PgConnection,
    user_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, Option<String>>(
        "SELECT timezone FROM user_options WHERE user_id = $1",
    )
    .bind(user_id as i32)
    .fetch_optional(conn)
    .await?
    .flatten())
}
