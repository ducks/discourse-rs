//! Web hooks (WebHook): which hooks an event goes to, the admin's CRUD
//! (admin), the delivery (emit) and the scheduled jobs (jobs).

pub mod admin;
pub mod emit;
pub mod jobs;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
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

/// `WebHook.enqueue_hooks(type, event, opts)`: an emit_web_hook_event job
/// for each active hook, its args the opts then the hook, event and type.
pub async fn enqueue_hooks(
    conn: &mut PgConnection,
    kind: &str,
    event: &str,
    opts: Map<String, Value>,
) -> Result<(), AppError> {
    for id in active_ids(&mut *conn, event).await? {
        let mut args = opts.clone();
        args.insert("web_hook_id".into(), json!(id));
        args.insert("event_name".into(), json!(event));
        args.insert("event_type".into(), json!(kind));
        crate::jobs::enqueue(&mut *conn, emit::EMIT_WEB_HOOK_EVENT, Value::Object(args)).await?;
    }
    Ok(())
}

/// A serializer's `to_json`: ActiveSupport's encoder escapes `<`, `>`
/// and `&` (and the JavaScript line separators) in its strings.
pub fn to_json(value: &Value) -> String {
    let mut out = String::new();
    for c in value.to_string().chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

/// `Guardian.new(Discourse.system_user)`, the payloads' scope.
pub async fn system_guardian(conn: &mut PgConnection) -> Result<Guardian, AppError> {
    let system = crate::session::current::SessionUser::load(&mut *conn, -1)
        .await?
        .ok_or(Unsupported("a site without its system user"))?;
    Ok(Guardian::for_user(&mut *conn, &system).await?)
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
