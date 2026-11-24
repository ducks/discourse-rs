//! The review actions that penalize the author (agree_and_suspend,
//! agree_and_silence, reject_and_silence) run the penalty inside
//! Reviewable#perform's transaction: when the action fails after it, the
//! suspension or silence, its staff action log, the jobs it enqueued and
//! the silenced_by_staff message roll back with the reviewable's changes.

use crate::common;

use common::chat_client::Client;
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;

async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

/// Post 52 (user0's, id 2) flagged off topic by user1, pending review at
/// version 0, as the recorded review_agree_and_suspend sets it up.
async fn flagged_post(db: &TestDb) {
    for sql in [
        "INSERT INTO post_actions (id, post_id, user_id, post_action_type_id, created_at, updated_at, staff_took_action, targets_topic) VALUES (9301, 52, 3, 3, '2026-10-02 10:00:00', '2026-10-02 10:00:00', FALSE, FALSE)",
        "UPDATE posts SET off_topic_count = 1 WHERE id = 52",
        "INSERT INTO reviewables (id, type, type_source, status, created_by_id, reviewable_by_moderator, category_id, topic_id, score, potential_spam, target_id, target_type, target_created_by_id, payload, version, latest_score, force_review, potentially_illegal, created_at, updated_at) VALUES (9301, 'ReviewableFlaggedPost', 'core', 0, 3, TRUE, 4, 35, 3, FALSE, 52, 'Post', 2, '{\"targets_topic\":false}', 0, '2026-10-02 10:00:00', FALSE, FALSE, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "INSERT INTO reviewable_scores (id, reviewable_id, user_id, reviewable_score_type, status, score, take_action_bonus, user_accuracy_bonus, created_at, updated_at) VALUES (9301, 9301, 3, 3, 0, 3, 0, 0, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "INSERT INTO reviewable_histories (id, reviewable_id, reviewable_history_type, status, created_by_id, created_at, updated_at) VALUES (9301, 9301, 0, 0, 3, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "UPDATE topics SET reviewable_score = 3 WHERE id = 35",
    ] {
        exec(db, sql).await;
    }
}

/// A queued reply by user1 (id 3), pending at version 0, as the recorded
/// queued_reject_and_silence sets it up.
async fn queued_post(db: &TestDb) {
    for sql in [
        "INSERT INTO reviewables (id, type, type_source, status, created_by_id, reviewable_by_moderator, category_id, topic_id, score, potential_spam, target_id, target_type, target_created_by_id, payload, version, latest_score, force_review, potentially_illegal, created_at, updated_at) VALUES (9311, 'ReviewableQueuedPost', 'core', 0, -1, TRUE, 4, 35, 6, FALSE, NULL, NULL, 3, '{\"raw\": \"This reply waits for approval in the queue.\", \"tags\": null, \"typing_duration_msecs\": 3000, \"composer_open_duration_msecs\": 12000}', 0, '2026-10-02 10:00:00', TRUE, FALSE, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "INSERT INTO reviewable_scores (id, reviewable_id, user_id, reviewable_score_type, status, score, take_action_bonus, user_accuracy_bonus, reason, created_at, updated_at) VALUES (9311, 9311, -1, 9, 0, 6, 0, 0, 'fast_typer', '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
        "INSERT INTO reviewable_histories (id, reviewable_id, reviewable_history_type, status, created_by_id, created_at, updated_at) VALUES (9311, 9311, 0, 0, -1, '2026-10-02 10:00:00', '2026-10-02 10:00:00')",
    ] {
        exec(db, sql).await;
    }
}

/// A concurrent perform taking the version between the penalty and the
/// action's increment_version!: the bump matches no row.
async fn version_taken(db: &TestDb) {
    exec(
        db,
        "CREATE FUNCTION test_version_taken() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.version <> OLD.version THEN RETURN NULL; END IF; RETURN NEW; END $$",
    )
    .await;
    exec(
        db,
        "CREATE TRIGGER test_version_taken BEFORE UPDATE ON reviewables \
         FOR EACH ROW EXECUTE FUNCTION test_version_taken()",
    )
    .await;
}

/// A database error in the action after the penalty: the transition's
/// history row refused.
async fn transition_fails(db: &TestDb) {
    exec(
        db,
        "CREATE FUNCTION test_transition_fails() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'transition refused'; END $$",
    )
    .await;
    exec(
        db,
        "CREATE TRIGGER test_transition_fails BEFORE INSERT ON reviewable_histories \
         FOR EACH ROW EXECUTE FUNCTION test_transition_fails()",
    )
    .await;
}

/// The penalty's traces for the user: suspended or silenced, the staff
/// action log rows (UserHistory suspend_user, silence_user), the
/// critical_user_email jobs (none for a silence since Discourse
/// ea0efdc105e), the system messages sent to them.
async fn penalty_traces(db: &TestDb, user_id: i32) -> (bool, bool, i64, i64, i64) {
    let (suspended, silenced): (bool, bool) = sqlx::query_as(
        "SELECT suspended_till IS NOT NULL, silenced_till IS NOT NULL FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let logged = count(
        db,
        &format!(
            "SELECT COUNT(*) FROM user_histories WHERE target_user_id = {user_id} AND action IN (10, 30)"
        ),
    )
    .await;
    let emails = count(
        db,
        &format!(
            "SELECT COUNT(*) FROM discourse_rs.jobs WHERE name IN ('critical_user_email', 'Jobs::CriticalUserEmail') \
               AND (args->>'user_id')::int = {user_id}"
        ),
    )
    .await;
    let messages = count(
        db,
        &format!(
            "SELECT COUNT(*) FROM topics t JOIN topic_allowed_users a ON a.topic_id = t.id \
             WHERE t.archetype = 'private_message' AND t.subtype = 'system_message' AND a.user_id = {user_id}"
        ),
    )
    .await;
    (suspended, silenced, logged, emails, messages)
}

/// The reviewable's status and version.
async fn reviewable(db: &TestDb, id: i64) -> (i32, i32) {
    sqlx::query_as("SELECT status, version FROM reviewables WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suspension_rolls_back_with_a_conflicting_agree() {
    let (db, st) = setup().await;
    flagged_post(&db).await;
    version_taken(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let path = "/review/9301/perform/agree_and_suspend";
    let body = "version=0&reason=Off+topic&message=Please+stop&suspend_until=2099-01-01";

    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 409, "{json}");
    assert_eq!(penalty_traces(&db, 2).await, (false, false, 0, 0, 0));
    assert_eq!(reviewable(&db, 9301).await, (0, 0));

    // Without the conflict the same request suspends: the failure above
    // came after the penalty.
    exec(&db, "DROP TRIGGER test_version_taken ON reviewables").await;
    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 200, "{json}");
    assert_eq!(penalty_traces(&db, 2).await, (true, false, 1, 1, 0));
    assert_eq!(reviewable(&db, 9301).await, (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silence_rolls_back_with_a_failed_agree() {
    let (db, st) = setup().await;
    flagged_post(&db).await;
    transition_fails(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let path = "/review/9301/perform/agree_and_silence";
    let body = "version=0&reason=Off+topic&silenced_till=2099-01-01";
    let before = penalty_traces(&db, 2).await;
    assert_eq!(before, (false, false, 0, 0, before.4));

    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 500, "{json}");
    assert_eq!(penalty_traces(&db, 2).await, before);
    assert_eq!(reviewable(&db, 9301).await, (0, 0));

    exec(
        &db,
        "DROP TRIGGER test_transition_fails ON reviewable_histories",
    )
    .await;
    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 200, "{json}");
    assert_eq!(
        penalty_traces(&db, 2).await,
        (false, true, 1, 0, before.4 + 1)
    );
    assert_eq!(reviewable(&db, 9301).await, (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_silence_rolls_back_with_a_failed_queued_reject() {
    let (db, st) = setup().await;
    queued_post(&db).await;
    transition_fails(&db).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let path = "/review/9311/perform/reject_and_silence";
    let body = "version=0&reason=Spam&silenced_till=2099-01-01";
    let before = penalty_traces(&db, 3).await;
    assert_eq!(before, (false, false, 0, 0, before.4));

    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 500, "{json}");
    assert_eq!(penalty_traces(&db, 3).await, before);
    assert_eq!(reviewable(&db, 9311).await, (0, 0));

    exec(
        &db,
        "DROP TRIGGER test_transition_fails ON reviewable_histories",
    )
    .await;
    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 200, "{json}");
    assert_eq!(
        penalty_traces(&db, 3).await,
        (false, true, 1, 0, before.4 + 1)
    );
    assert_eq!(reviewable(&db, 9311).await, (2, 1));
}

/// The flagged post's raw and version.
async fn post(db: &TestDb, id: i32) -> (String, i32) {
    sqlx::query_as("SELECT raw, version FROM posts WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// The penalty dialog's post action edit (TriggerPostAction#edit) runs
/// PostRevisor in the same transaction: rolled back with a failed agree,
/// applied with the suspension when it goes through.
#[tokio::test(flavor = "multi_thread")]
async fn a_penalty_post_edit_rolls_back_with_a_conflicting_agree() {
    let (db, st) = setup().await;
    flagged_post(&db).await;
    version_taken(&db).await;
    let original = post(&db, 52).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let path = "/review/9301/perform/agree_and_suspend";
    let body = "version=0&reason=Off+topic&suspend_until=2099-01-01\
                &post_action=edit&post_edit=Edited+by+the+moderator";

    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 409, "{json}");
    assert_eq!(post(&db, 52).await, original);
    assert_eq!(penalty_traces(&db, 2).await, (false, false, 0, 0, 0));
    assert_eq!(reviewable(&db, 9301).await, (0, 0));

    exec(&db, "DROP TRIGGER test_version_taken ON reviewables").await;
    let (status, json) = admin.request("PUT", path, body).await;
    assert_eq!(status, 200, "{json}");
    assert_eq!(post(&db, 52).await.0, "Edited by the moderator");
    assert_eq!(penalty_traces(&db, 2).await, (true, false, 1, 0, 0));
    assert_eq!(reviewable(&db, 9301).await, (1, 1));
}
