//! The tag groups pages (src/tags/groups_view.rs): the list, a new group's
//! form and a group's, for staff. Rails' statuses are compared in
//! parity/writes/tags/groups_pages; the form's saving in
//! tests/browser/tag-groups.spec.js.

use crate::common;
use crate::common::chat_client;

use chat_client::Client;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;

const BOOL: i32 = 5;

#[tokio::test]
async fn the_list_new_and_edit_pages() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "tagging_enabled", BOOL, "t").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;

    let (status, html) = admin.page("/tag_groups").await;
    assert_eq!(status, 200, "{html}");
    assert!(
        html.contains("<title>Tag groups - Discourse</title>"),
        "{html}"
    );
    assert!(html.contains("Create a new tag group to get started"));
    assert!(html.contains("tag-groups.js"));
    assert!(!html.contains("tag-groups-sidebar"));

    let (_, html) = admin.page("/tag_groups/new").await;
    assert!(html.contains("<h3>Create new group</h3>"));
    assert!(html.contains("value=\"New tag group\""));
    assert!(html.contains("id=\"public-permission\" name=\"tag-permissions-choice\" type=\"radio\" value=\"public\" checked"));
    assert!(html.contains("data-tag-group-delete=\"\" type=\"button\" disabled"));

    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tag_groups (name, one_per_topic, created_at, updated_at) VALUES ('Colors', true, now(), now()) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tag_group_permissions (tag_group_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, 0, 3, now(), now()), ($1, 3, 1, now(), now())",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, html) = admin.page(&format!("/tag_groups/{id}")).await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<h3>Edit tag group</h3>"));
    assert!(html.contains(&format!(
        "<li><a href=\"/tag_groups/{id}\" class=\"active\" aria-current=\"page\">Colors</a></li>"
    )));
    assert!(html.contains("value=\"visible\" checked"));
    assert!(html.contains("name=\"onepertopic\" type=\"checkbox\" checked"));
    // Both GroupChoosers show staff, the one group with FULL.
    assert_eq!(html.matches("data-name=\"staff\"").count(), 2, "{html}");

    let (status, _) = admin.page("/tag_groups/99999").await;
    assert_eq!(status, 404);

    // Not for members.
    let mut member = Client::logged_in(st, "user1").await;
    let (status, _) = member.page("/tag_groups").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn tagging_disabled() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "tagging_enabled", BOOL, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, html) = admin.page("/tag_groups").await;
    assert_eq!(status, 200);
    assert!(html.contains("<div class=\"alert info\">Tagging is disabled. </div>"));
    // tagGroups.new's beforeModel goes back to the list.
    let (status, _) = admin.page("/tag_groups/new").await;
    assert_eq!(status, 303);
}
