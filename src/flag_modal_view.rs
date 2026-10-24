//! What static/js/flag-modal.js needs for the flag modal (modal/flag.gjs)
//! on a topic page: Site's post_action_types and topic_flag_types (each
//! flag's name, description and short description as FlagSerializer
//! gives them), the topic's id, author and actions summary (TopicFlag),
//! whether the viewer is staff, the settings the modal reads and its
//! strings as `i18n` resolves them (count keys as their `one` and `other`
//! forms).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::admin_flags::Flag;
use crate::html::HtmlError;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;

const KEYS: &[&str] = &[
    "generic_error",
    "generic_error_with_reason",
    "multiple_errors",
    "cancel",
    "select_kit.filter_by",
    "flagging.title",
    "flagging.review_process_description",
    "flagging.action",
    "flagging.notify_action",
    "flagging.take_action",
    "flagging.take_action_options.default.title",
    "flagging.take_action_options.default.details",
    "flagging.take_action_options.suspend.title",
    "flagging.take_action_options.suspend.details",
    "flagging.take_action_options.silence.title",
    "flagging.take_action_options.silence.details",
    "flagging.official_warning",
    "flagging.delete_spammer",
    "flagging.flag_for_review",
    "flagging.delete_confirm_MF",
    "flagging.yes_delete_spammer",
    "flagging.ip_address_missing",
    "flagging.hidden_email_address",
    "flagging.submit_tooltip",
    "flagging.cant",
    "flagging.notify_staff",
    "flagging.formatted_name.off_topic",
    "flagging.formatted_name.inappropriate",
    "flagging.formatted_name.spam",
    "flagging.formatted_name.illegal",
    "flagging.custom_placeholder_notify_user",
    "flagging.notify_user_textarea_label",
    "flagging.custom_placeholder_notify_moderators",
    "flagging.notify_moderators_textarea_label",
    "flagging.custom_placeholder_illegal",
    "flagging.confirmation_illegal",
    "flagging_topic.title",
    "flagging_topic.action",
    "flagging_topic.notify_action",
    "admin.user.delete_failed",
];

const COUNT_KEYS: &[&str] = &[
    "flagging.custom_message.at_least",
    "flagging.custom_message.more",
    "flagging.custom_message.left",
];

const FLAG_COLUMNS: &str = "id, name, name_key, description, applies_to, position, require_message, enabled, score_type, auto_action_type";

/// FlagSerializer over the flags (`Flag.unscoped.order(:position)` without
/// score types), for `target` ("post_action" or "topic_flag").
async fn flag_types(
    conn: &mut PgConnection,
    i18n: &I18n,
    base_path: &str,
    target: &str,
) -> Result<Value, HtmlError> {
    let filter = if target == "topic_flag" {
        "'Topic' = ANY(applies_to) AND NOT score_type"
    } else {
        "NOT score_type"
    };
    let flags: Vec<Flag> = sqlx::query_as(&format!(
        "SELECT {FLAG_COLUMNS} FROM flags WHERE {filter} ORDER BY position"
    ))
    .fetch_all(&mut *conn)
    .await?;
    Ok(Value::Array(
        flags
            .iter()
            .map(|f| crate::admin_flags::serialize(i18n, base_path, f, target, false))
            .collect(),
    ))
}

/// The `#flag-modal-config` block, or nothing for an anonymous viewer.
pub async fn config(
    conn: &mut PgConnection,
    i18n: &I18n,
    settings: &SiteSettings,
    base_path: &str,
    view: &Value,
    staff: bool,
) -> Result<String, HtmlError> {
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
    let config = json!({
        "strings": strings,
        "post_action_types": flag_types(conn, i18n, base_path, "post_action").await?,
        "topic_flag_types": flag_types(conn, i18n, base_path, "topic_flag").await?,
        "topic": {
            "id": view["id"],
            "user_id": view["user_id"],
            "archetype": view["archetype"],
            "actions_summary": view["actions_summary"],
        },
        "staff": staff,
        "mobile": crate::mobile::mobile_view(),
        "min_personal_message_post_length": settings.get("min_personal_message_post_length")?.to_i(),
        "moderators_view_emails": settings.get("moderators_view_emails")?.truthy(),
    });
    Ok(format!(
        "<script type=\"application/json\" id=\"flag-modal-config\">{}</script>",
        config.to_string().replace('<', "\\u003c")
    ))
}
