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

/// What the payload serializers read.
#[derive(Clone, Copy)]
pub struct Payloads<'a> {
    pub settings: &'a crate::site_settings::SiteSettings,
    pub i18n: &'a crate::i18n::I18n,
    pub config: &'a crate::config::Config,
}

impl Payloads<'_> {
    /// A TopicView seen by the system user.
    async fn with_view<T>(
        &self,
        conn: &mut PgConnection,
        f: impl AsyncFnOnce(
            &mut crate::topic_view::TopicView<'_>,
        ) -> Result<T, crate::topic_view::TopicViewError>,
    ) -> Result<T, AppError> {
        let guardian = system_guardian(&mut *conn).await?;
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        let mut view = crate::topic_view::TopicView {
            conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: &guardian,
            urls: &urls,
            options: crate::topic_view::Options {
                page: 0,
                post_number: None,
            },
            post_types: Vec::new(),
        };
        Ok(f(&mut view).await?)
    }

    /// WebHookPostSerializer's JSON for a post.
    pub async fn post(
        &self,
        conn: &mut PgConnection,
        post_id: i32,
        held: crate::topic_view::Held,
    ) -> Result<Value, AppError> {
        self.with_view(conn, async |v| v.web_hook_post(post_id, held).await)
            .await
    }

    /// WebHookLikeSerializer: the liked post and who liked it.
    pub async fn like(
        &self,
        conn: &mut PgConnection,
        post_id: i32,
        user_id: i32,
    ) -> Result<Value, AppError> {
        self.with_view(conn, async |v| v.web_hook_like(post_id, user_id).await)
            .await
    }

    /// WebHookTopicViewSerializer's JSON for a topic.
    pub async fn topic(
        &self,
        conn: &mut PgConnection,
        topic_id: i32,
        held: Option<crate::topic_view::HeldTopic>,
    ) -> Result<Value, AppError> {
        self.with_view(conn, async |v| v.web_hook_topic(topic_id, held).await)
            .await
    }
}

/// The topic's tag ids as `topic.tags.pluck(:id)` gives them.
async fn topic_tag_ids(conn: &mut PgConnection, topic_id: i32) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT tags.id FROM tags JOIN topic_tags ON topic_tags.tag_id = tags.id \
         WHERE topic_tags.topic_id = $1 ORDER BY tags.ctid",
    )
    .bind(topic_id)
    .fetch_all(conn)
    .await
}

/// `WebHook.enqueue_post_hooks(event, post, payload)`: with the post's
/// topic (deleted or not) and its category and tags.
pub async fn enqueue_post_hooks(
    conn: &mut PgConnection,
    p: &Payloads<'_>,
    event: &str,
    post_id: i32,
    held: crate::topic_view::Held,
) -> Result<(), AppError> {
    if !active(&mut *conn, event).await? {
        return Ok(());
    }
    let topic: Option<(i32, Option<i32>)> = sqlx::query_as(
        "SELECT t.id, t.category_id FROM posts p JOIN topics t ON t.id = p.topic_id WHERE p.id = $1",
    )
    .bind(post_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, category_id)) = topic else {
        return Ok(());
    };
    let payload = to_json(&p.post(&mut *conn, post_id, held).await?);
    let mut opts = Map::new();
    opts.insert("id".into(), json!(post_id));
    opts.insert("category_id".into(), json!(category_id));
    opts.insert(
        "tag_ids".into(),
        json!(topic_tag_ids(conn, topic_id).await?),
    );
    opts.insert("payload".into(), json!(payload));
    enqueue_hooks(conn, "post", event, opts).await
}

/// `WebHook.enqueue_topic_hooks(event, topic, payload)`
pub async fn enqueue_topic_hooks(
    conn: &mut PgConnection,
    p: &Payloads<'_>,
    event: &str,
    topic_id: i32,
    held: Option<crate::topic_view::HeldTopic>,
) -> Result<(), AppError> {
    if !active(&mut *conn, event).await? {
        return Ok(());
    }
    let category_id: Option<Option<i32>> =
        sqlx::query_scalar("SELECT category_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(category_id) = category_id else {
        return Ok(());
    };
    let payload = to_json(&p.topic(&mut *conn, topic_id, held).await?);
    let mut opts = Map::new();
    opts.insert("id".into(), json!(topic_id));
    opts.insert("category_id".into(), json!(category_id));
    opts.insert(
        "tag_ids".into(),
        json!(topic_tag_ids(conn, topic_id).await?),
    );
    opts.insert("payload".into(), json!(payload));
    enqueue_hooks(conn, "topic", event, opts).await
}

/// `WebHook.enqueue_object_hooks(type, object, event, serializer, opts)`:
/// the payload built only when a hook will receive it.
pub async fn enqueue_object_hooks(
    conn: &mut PgConnection,
    kind: &str,
    event: &str,
    id: i64,
    payload: impl AsyncFnOnce(&mut PgConnection) -> Result<Value, AppError>,
    opts: Map<String, Value>,
) -> Result<(), AppError> {
    if !active(&mut *conn, event).await? {
        return Ok(());
    }
    let payload = to_json(&payload(&mut *conn).await?);
    let mut all = opts;
    all.insert("id".into(), json!(id));
    all.insert("payload".into(), json!(payload));
    enqueue_hooks(conn, kind, event, all).await
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
