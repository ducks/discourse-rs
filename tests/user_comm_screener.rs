//! UserCommScreener against the test database: the targets' preferences
//! towards the actor, the actor's towards them (the 'disallowed_pm' join
//! as Rails writes it), the staff short circuit and unknown ids.

mod common;

use common::TestDb;
use discourse_rs::user_comm_screener::{Actor, UserCommScreener};

const USER0: i32 = 2;
const USER1: i32 = 3;
const USER2: i32 = 4;
const USER3: i32 = 5;

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql).execute(&db.pool).await.unwrap();
}

async fn screener(db: &TestDb, actor: i32, staff: bool, targets: &[i32]) -> UserCommScreener {
    let mut conn = db.pool.acquire().await.unwrap();
    UserCommScreener::new(&mut conn, Actor { id: actor, staff }, targets)
        .await
        .unwrap()
}

#[tokio::test]
async fn targets_screening_the_actor() {
    let db = TestDb::new().await;
    exec(&db, "INSERT INTO ignored_users (user_id, ignored_user_id, created_at, updated_at, expiring_at) VALUES (4, 3, now(), now(), now() + interval '1 day')").await;
    exec(&db, "INSERT INTO muted_users (user_id, muted_user_id, created_at, updated_at) VALUES (5, 3, now(), now())").await;
    exec(
        &db,
        "UPDATE user_options SET allow_private_messages = false WHERE user_id = 2",
    )
    .await;
    let s = screener(&db, USER1, false, &[USER0, USER1, USER2, USER3]).await;
    let mut preventing = s.preventing_actor_communication();
    preventing.sort();
    assert_eq!(preventing, vec![USER0, USER2, USER3]);
    assert!(s.allowing_actor_communication().is_empty());
    assert!(s.ignoring_or_muting_actor(USER2).unwrap());
    assert!(s.ignoring_or_muting_actor(USER3).unwrap());
    assert!(!s.ignoring_or_muting_actor(USER0).unwrap());
    assert!(s.disallowing_pms_from_actor(USER0).unwrap());
    // The actor itself is never a target, but asking about it is allowed.
    assert!(!s.ignoring_or_muting_actor(USER1).unwrap());
    // An id outside the targets is Discourse::NotFound.
    assert!(s.ignoring_or_muting_actor(1).is_err());

    // Nobody screens staff.
    let staff = screener(&db, USER1, true, &[USER0, USER2, USER3]).await;
    assert!(staff.preventing_actor_communication().is_empty());
    assert_eq!(staff.allowing_actor_communication().len(), 3);
    assert!(!staff.disallowing_pms_from_actor(USER0).unwrap());
}

#[tokio::test]
async fn allowed_pm_lists_on_both_sides() {
    let db = TestDb::new().await;
    // user2 takes messages only from user3.
    exec(
        &db,
        "UPDATE user_options SET enable_allowed_pm_users = true WHERE user_id IN (3, 4)",
    )
    .await;
    exec(&db, "INSERT INTO allowed_pm_users (user_id, allowed_pm_user_id, created_at, updated_at) VALUES (4, 5, now(), now())").await;
    let s = screener(&db, USER1, false, &[USER2]).await;
    assert!(s.disallowing_pms_from_actor(USER2).unwrap());
    let s = screener(&db, USER3, false, &[USER2]).await;
    assert!(!s.disallowing_pms_from_actor(USER2).unwrap());

    // The actor's side: user1's list allows nobody, so the quirky join
    // counts user0 and user2 (named by nobody's list) as disallowed, and
    // user3 (on user2's list, not user1's) as allowed.
    let s = screener(&db, USER1, false, &[USER0, USER2, USER3]).await;
    assert!(s.actor_disallowing_pms(USER0).unwrap());
    assert!(s.actor_disallowing_pms(USER2).unwrap());
    assert!(!s.actor_disallowing_pms(USER3).unwrap());
    // Without the list turned on, nobody is disallowed by it.
    exec(
        &db,
        "UPDATE user_options SET enable_allowed_pm_users = false WHERE user_id = 3",
    )
    .await;
    let s = screener(&db, USER1, false, &[USER0, USER2, USER3]).await;
    assert!(!s.actor_disallowing_pms(USER0).unwrap());
    // allow_private_messages off disallows everyone.
    exec(
        &db,
        "UPDATE user_options SET allow_private_messages = false WHERE user_id = 3",
    )
    .await;
    let s = screener(&db, USER1, false, &[USER3]).await;
    assert!(s.actor_disallowing_all_pms());
    assert!(s.actor_disallowing_pms(USER3).unwrap());
}

#[tokio::test]
async fn the_actors_ignores_and_mutes() {
    let db = TestDb::new().await;
    exec(&db, "INSERT INTO ignored_users (user_id, ignored_user_id, created_at, updated_at, expiring_at) VALUES (3, 4, now(), now(), now() + interval '1 day')").await;
    exec(&db, "INSERT INTO muted_users (user_id, muted_user_id, created_at, updated_at) VALUES (3, 5, now(), now())").await;
    let s = screener(&db, USER1, false, &[USER0, USER2, USER3]).await;
    assert!(s.actor_ignoring(USER2).unwrap());
    assert!(s.actor_muting(USER3).unwrap());
    assert!(!s.actor_ignoring(USER0).unwrap());
    let mut preventing = s.actor_preventing_communication();
    preventing.sort();
    assert_eq!(preventing, vec![USER2, USER3]);
    assert_eq!(s.actor_allowing_communication(), vec![USER0]);
}
