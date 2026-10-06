//! TopicTrackingState.report on the fixture, against what the reference
//! reports for the same rows, and the counts taken from it.

mod common;

use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::site_settings::SiteSettings;
use discourse_rs::topic_tracking_report::{Kind, State, Tracking, load};
use sqlx::PgConnection;

async fn report(conn: &mut PgConnection, settings: &SiteSettings, username: &str) -> Tracking {
    let id: i32 = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    let user = discourse_rs::session::current::SessionUser::load(&mut *conn, id)
        .await
        .unwrap()
        .unwrap();
    let guardian = Guardian::for_user(&mut *conn, &user).await.unwrap();
    load(&mut *conn, settings, &guardian)
        .await
        .unwrap()
        .unwrap()
}

fn sorted(mut states: Vec<State>) -> Vec<State> {
    states.sort_by_key(|s| s.topic_id);
    states
}

/// The admin's rows are the reference's (`TopicTrackingState.report`
/// through its serializer): the uncategorized definition and topic 38 new,
/// topic 35 unread. user1 has read less of topic 35 here than on the
/// reference, so it is unread for them too.
#[tokio::test]
async fn the_report_lists_new_and_unread_topics() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let new = |topic_id, category_id, is_category_topic, tags: Vec<i32>| State {
        topic_id,
        highest_post_number: 1,
        last_read_post_number: None,
        category_id,
        is_category_topic,
        notification_level: None,
        created_in_new_period: true,
        tags: Some(tags),
    };
    let unread = |last_read| State {
        topic_id: 35,
        highest_post_number: 5,
        last_read_post_number: Some(last_read),
        category_id: 4,
        is_category_topic: false,
        notification_level: Some(2),
        created_in_new_period: true,
        tags: Some(vec![2]),
    };

    let admin = report(&mut conn, &settings, "admin").await;
    assert_eq!(
        sorted(admin.states.clone()),
        vec![
            new(34, 34, true, vec![]),
            unread(4),
            new(38, 34, false, vec![1])
        ]
    );
    // enable_unified_new, a stable upcoming change, is on for everyone (as
    // on the reference): the new count takes in the unread.
    assert!(admin.unified_new);
    assert_eq!(admin.lookup("new"), 3);
    assert_eq!(admin.count(Kind::New, None, None), 2);
    assert_eq!(admin.lookup("unread"), 1);
    // The category's definition counts for it alone, not its parent's.
    assert_eq!(admin.count(Kind::New, Some(34), None), 2);
    assert_eq!(admin.count(Kind::Unread, Some(4), None), 1);
    assert_eq!(admin.count(Kind::NewAndUnread, None, Some(2)), 1);

    let user1 = report(&mut conn, &settings, "user1").await;
    assert_eq!(
        sorted(user1.states),
        vec![new(34, 34, true, vec![]), unread(2)]
    );

    // Reading the topic to its end takes it off the report.
    sqlx::query(
        "UPDATE topic_users SET last_read_post_number = 5 \
         WHERE topic_id = 35 AND user_id = (SELECT id FROM users WHERE username = 'admin')",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    let admin = report(&mut conn, &settings, "admin").await;
    assert_eq!(admin.lookup("unread"), 0);
}
