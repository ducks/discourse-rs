//! Web hooks (WebHook): which hooks an event goes to.

use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// `WebHook.translate_event_name_to_type`: the event type a hook subscribes
/// to for an event (a topic's status changes are topic_edited, a
/// reviewable's score and transitions reviewable_updated).
pub fn event_type(event: &str) -> &str {
    let status_updated = event
        .strip_prefix("topic_")
        .and_then(|rest| rest.strip_suffix("_status_updated"))
        .is_some_and(|word| {
            !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    match event {
        _ if status_updated => "topic_edited",
        "reviewable_score_updated" | "reviewable_transitioned_to" => "reviewable_updated",
        other => other,
    }
}

/// `WebHook.active_web_hooks(event)`: the active hooks subscribed to the
/// event's type, or wildcard ones, by id. An INNER JOIN on the event types,
/// so a wildcard hook without any never fires.
pub async fn active_ids(conn: &mut PgConnection, event: &str) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT w.id FROM web_hooks w \
         JOIN web_hook_event_types_hooks h ON h.web_hook_id = w.id \
         JOIN web_hook_event_types t ON t.id = h.web_hook_event_type_id \
         WHERE w.active AND (w.wildcard_web_hook OR t.name = $1) ORDER BY w.id",
    )
    .bind(event_type(event))
    .fetch_all(&mut *conn)
    .await
}

/// `WebHook.active_web_hooks(event).exists?`
pub async fn active(conn: &mut PgConnection, event: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM web_hooks w \
         JOIN web_hook_event_types_hooks h ON h.web_hook_id = w.id \
         JOIN web_hook_event_types t ON t.id = h.web_hook_event_type_id \
         WHERE w.active AND (w.wildcard_web_hook OR t.name = $1))",
    )
    .bind(event_type(event))
    .fetch_one(&mut *conn)
    .await
}

/// An event whose web hooks are not ported yet: refused when a hook would
/// receive it, so nothing is written that Rails would follow with a
/// delivery.
pub async fn refuse(
    conn: &mut PgConnection,
    event: &str,
    what: &'static str,
) -> Result<(), AppError> {
    if active(conn, event).await? {
        return Err(Unsupported(what).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::event_type;

    #[test]
    fn translates_event_names() {
        assert_eq!(event_type("topic_closed_status_updated"), "topic_edited");
        assert_eq!(
            event_type("topic_pinned_globally_status_updated"),
            "topic_edited"
        );
        assert_eq!(event_type("topic__status_updated"), "topic__status_updated");
        assert_eq!(event_type("reviewable_score_updated"), "reviewable_updated");
        assert_eq!(
            event_type("reviewable_transitioned_to"),
            "reviewable_updated"
        );
        assert_eq!(event_type("post_created"), "post_created");
    }
}
