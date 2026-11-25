//! A page load of /form-templates(/:id): Rails' check_xhr boots the app
//! once the member is logged in, before the setting or the id are looked
//! at, and Ember only routes form templates under admin. Rails' statuses
//! are compared in parity/writes/form_templates/html_page*.

use crate::common;
use crate::common::chat_client;

use chat_client::Client;
use common::{TestDb, config, set_setting, state};
use discourse_rs::config::RailsEnv;

const BOOL: i32 = 5;

#[tokio::test]
async fn a_members_page_load_is_embers_not_found() {
    let db = TestDb::new().await;
    set_setting(&db.pool, "enable_form_templates", BOOL, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut member = Client::logged_in(st, "user1").await;
    for path in ["/form-templates", "/form-templates/1", "/form-templates/99"] {
        let (status, body) = member.page(path).await;
        assert_eq!(status, 200, "{path}: {body}");
        assert!(body.contains("page-not-found"), "{path}: {body}");
    }
    // The API still answers the setting.
    let (status, _) = member.get("/form-templates.json").await;
    assert_eq!(status, 403);
}
