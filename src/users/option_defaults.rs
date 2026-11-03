//! The default user preferences that are user options
//! (`SiteSettingUpdateExistingUsers.user_options`): how many human users
//! hold a setting's previous value (`Admin::SiteSettingsController
//! #user_count`), and the backfill that moves them to the new one
//! (`SiteSettingUpdateExistingUsers.call`).

use sqlx::PgConnection;

use crate::site_settings::Value;

/// How the setting's value maps onto its column.
#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Int,
    /// `UserOption.text_sizes`
    TextSize,
    /// `UserOption.title_count_modes`
    TitleCountMode,
}

/// `SiteSettingUpdateExistingUsers.user_options`: setting, column, kind.
const OPTIONS: [(&str, &str, Kind); 27] = [
    (
        "default_email_mailing_list_mode",
        "mailing_list_mode",
        Kind::Bool,
    ),
    (
        "default_email_mailing_list_mode_frequency",
        "mailing_list_mode_frequency",
        Kind::Int,
    ),
    ("default_email_level", "email_level", Kind::Int),
    (
        "default_email_messages_level",
        "email_messages_level",
        Kind::Int,
    ),
    (
        "default_topics_automatic_unpin",
        "automatically_unpin_topics",
        Kind::Bool,
    ),
    (
        "default_email_previous_replies",
        "email_previous_replies",
        Kind::Int,
    ),
    ("default_email_in_reply_to", "email_in_reply_to", Kind::Bool),
    ("default_other_enable_quoting", "enable_quoting", Kind::Bool),
    (
        "default_other_enable_smart_lists",
        "enable_smart_lists",
        Kind::Bool,
    ),
    (
        "default_other_external_links_in_new_tab",
        "external_links_in_new_tab",
        Kind::Bool,
    ),
    (
        "default_other_dynamic_favicon",
        "dynamic_favicon",
        Kind::Bool,
    ),
    (
        "default_other_new_topic_duration_minutes",
        "new_topic_duration_minutes",
        Kind::Int,
    ),
    (
        "default_other_auto_track_topics_after_msecs",
        "auto_track_topics_after_msecs",
        Kind::Int,
    ),
    (
        "default_other_notification_level_when_replying",
        "notification_level_when_replying",
        Kind::Int,
    ),
    (
        "default_other_like_notification_frequency",
        "like_notification_frequency",
        Kind::Int,
    ),
    (
        "default_other_skip_new_user_tips",
        "skip_new_user_tips",
        Kind::Bool,
    ),
    (
        "default_other_enable_markdown_monospace_font",
        "enable_markdown_monospace_font",
        Kind::Bool,
    ),
    (
        "default_email_digest_frequency",
        "digest_after_minutes",
        Kind::Int,
    ),
    (
        "default_include_tl0_in_digests",
        "include_tl0_in_digests",
        Kind::Bool,
    ),
    ("default_text_size", "text_size_key", Kind::TextSize),
    (
        "default_title_count_mode",
        "title_count_mode_key",
        Kind::TitleCountMode,
    ),
    ("default_hide_profile", "hide_profile", Kind::Bool),
    ("default_hide_presence", "hide_presence", Kind::Bool),
    (
        "default_sidebar_link_to_filtered_list",
        "sidebar_link_to_filtered_list",
        Kind::Bool,
    ),
    (
        "default_sidebar_show_count_of_new_items",
        "sidebar_show_count_of_new_items",
        Kind::Bool,
    ),
    ("default_composition_mode", "composition_mode", Kind::Int),
    (
        "default_watched_precedence_over_muted",
        "watched_precedence_over_muted",
        Kind::Bool,
    ),
];

/// Whether the setting is one of the user options' defaults.
pub fn is_user_option(name: &str) -> bool {
    OPTIONS.iter().any(|(n, ..)| *n == name)
}

fn option(name: &str) -> Option<(&'static str, Kind)> {
    OPTIONS
        .iter()
        .find(|(n, ..)| *n == name)
        .map(|(_, column, kind)| (*column, *kind))
}

/// The column's value for the setting's (Ruby) value, as text: the
/// keyed enums by their key (nil for a name they lack), the others as
/// Ruby's to_s.
fn column_value(kind: Kind, value: &Value) -> Option<String> {
    let s = value.to_s();
    match kind {
        Kind::TextSize => ["normal", "larger", "largest", "smaller", "smallest"]
            .iter()
            .position(|k| *k == s)
            .map(|i| i.to_string()),
        Kind::TitleCountMode => ["notifications", "contextual"]
            .iter()
            .position(|k| *k == s)
            .map(|i| i.to_string()),
        Kind::Bool => Some(value.truthy().to_string()),
        Kind::Int => Some(value.to_i().to_string()),
    }
}

fn sql_type(kind: Kind) -> &'static str {
    match kind {
        Kind::Bool => "boolean",
        _ => "integer",
    }
}

/// `UserOption.human_users.where(column => previous).count`; None for a
/// setting that isn't a user option's default.
pub async fn count(
    conn: &mut PgConnection,
    name: &str,
    previous: &Value,
) -> Result<Option<i64>, sqlx::Error> {
    let Some((column, kind)) = option(name) else {
        return Ok(None);
    };
    let previous = column_value(kind, previous);
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM user_options WHERE user_id > 0 AND \
         ({column} IS NOT DISTINCT FROM CAST($1 AS {}))",
        sql_type(kind)
    ))
    .bind(previous)
    .fetch_one(conn)
    .await?;
    Ok(Some(count))
}

/// `SiteSettingUpdateExistingUsers.call` for a user option's default:
/// the human users holding the previous value given the new one (and
/// the digest frequency's email_digests with it).
pub async fn backfill(
    conn: &mut PgConnection,
    name: &str,
    new_value: &Value,
    previous: &Value,
) -> Result<(), sqlx::Error> {
    let Some((column, kind)) = option(name) else {
        return Ok(());
    };
    let ty = sql_type(kind);
    let digests = if name == "default_email_digest_frequency" {
        ", email_digests = CAST($1 AS integer) <> 0"
    } else {
        ""
    };
    sqlx::query(&format!(
        "UPDATE user_options SET {column} = CAST($1 AS {ty}){digests} \
         WHERE user_id > 0 AND ({column} IS NOT DISTINCT FROM CAST($2 AS {ty}))"
    ))
    .bind(column_value(kind, new_value))
    .bind(column_value(kind, previous))
    .execute(conn)
    .await?;
    Ok(())
}
