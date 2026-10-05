//! `/notification-alert/<id>` (PostAlerter.create_notification_alert):
//! the alert a browser pops up for a new notification.

mod common;

use common::{TestDb, bus_messages, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use serde_json::json;

/// A reply to the topic's first post by someone other than its author:
/// (post id, topic id, post number, replier's username, first post's
/// author).
async fn reply_to_another(st: &AppState) -> (i32, i32, i32, String, i32) {
    let pool = &st.pool;
    let (post_id, topic_id, post_number, username, owner): (i32, i32, i32, String, i32) =
        sqlx::query_as(
            "SELECT p.id, p.topic_id, p.post_number, u.username, f.user_id FROM posts p \
             JOIN users u ON u.id = p.user_id \
             JOIN topics t ON t.id = p.topic_id JOIN categories c ON c.id = t.category_id \
             JOIN posts f ON f.topic_id = p.topic_id AND f.post_number = 1 \
             WHERE t.archetype = 'regular' AND p.post_number > 1 AND p.post_type = 1 \
               AND p.deleted_at IS NULL AND t.deleted_at IS NULL AND NOT c.read_restricted \
               AND p.user_id > 0 AND f.user_id > 0 AND f.user_id <> p.user_id \
             ORDER BY p.id LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE posts SET reply_to_post_number = 1, reply_to_user_id = $2 WHERE id = $1")
        .bind(post_id)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    // A clean slate: no notifications on the topic to collapse into, and the
    // owner at the regular level.
    sqlx::query("DELETE FROM notifications WHERE topic_id = $1")
        .bind(topic_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM topic_users WHERE topic_id = $1 AND user_id = $2")
        .bind(topic_id)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    (post_id, topic_id, post_number, username, owner)
}

async fn run_post_alert(st: &AppState, post_id: i32) {
    let job: i64 = sqlx::query_scalar(
        "INSERT INTO discourse_rs.jobs (name, args) VALUES ('post_alert', $1) RETURNING id",
    )
    .bind(json!({ "post_id": post_id, "new_record": true, "options": null }))
    .fetch_one(&st.pool)
    .await
    .unwrap();
    discourse_rs::jobs::perform_now(st, job).await.unwrap();
    let error: Option<String> =
        sqlx::query_scalar("SELECT last_error FROM discourse_rs.jobs WHERE id = $1")
            .bind(job)
            .fetch_optional(&st.pool)
            .await
            .unwrap()
            .flatten();
    assert_eq!(error, None, "post_alert ran");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_alerts_the_author_it_replies_to() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, topic_id, post_number, username, owner) = reply_to_another(&st).await;
    sqlx::query("UPDATE users SET last_seen_at = now() WHERE id = $1")
        .bind(owner)
        .execute(&db.pool)
        .await
        .unwrap();
    let (title, slug): (String, String) =
        sqlx::query_as("SELECT title, slug FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();

    let from = st.bus.now().await.unwrap();
    run_post_alert(&st, post_id).await;
    let channel = format!("/notification-alert/{owner}");
    let alerts = bus_messages(&st, from, &[&channel]).await;
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    let alert = &alerts[0];
    assert_eq!(alert.audience, Some(vec![format!("user:{owner}")]));
    assert_eq!(alert.data["notification_type"], 2, "replied");
    assert_eq!(alert.data["post_id"], post_id);
    assert_eq!(alert.data["post_number"], post_number);
    assert_eq!(alert.data["topic_id"], topic_id);
    assert_eq!(alert.data["topic_title"], title.as_str());
    assert_eq!(alert.data["username"], username.as_str());
    assert_eq!(
        alert.data["post_url"],
        format!("/t/{slug}/{topic_id}/{post_number}")
    );
    let excerpt = alert.data["excerpt"].as_str().unwrap();
    assert!(!excerpt.is_empty() && !excerpt.contains('<'), "{excerpt}");
    // The notification state goes with it.
    let state_channel = format!("/notification/{owner}");
    assert!(!bus_messages(&st, from, &[&state_channel]).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_alert_for_someone_away_a_month() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (post_id, _, _, _, owner) = reply_to_another(&st).await;
    sqlx::query("UPDATE users SET last_seen_at = now() - interval '31 days' WHERE id = $1")
        .bind(owner)
        .execute(&db.pool)
        .await
        .unwrap();

    let from = st.bus.now().await.unwrap();
    run_post_alert(&st, post_id).await;
    let made: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM notifications WHERE user_id = $1 AND notification_type = 2)",
    )
    .bind(owner)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(made, "the notification is still made");
    let channel = format!("/notification-alert/{owner}");
    assert!(bus_messages(&st, from, &[&channel]).await.is_empty());
}
