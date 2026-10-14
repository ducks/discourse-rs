//! The groups pages' HTML: the directory (/groups) from groups#index with
//! its cards and filters, and a group's members page (/g/:name) from
//! groups#show and groups#members. The documents are covered by the
//! parity/writes/groups read cases.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    fn anonymous(state: AppState) -> Client {
        Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        }
    }

    async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
        accept: Option<&str>,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
            if let Some(token) = &self.csrf {
                request = request.header("x-csrf-token", token.as_str());
            }
        }
        if let Some(accept) = accept {
            request = request.header(header::ACCEPT, accept);
        }
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state);
        let (_, csrf) = client
            .send(Method::GET, "/session/csrf.json", "", true, None)
            .await;
        let csrf: serde_json::Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
                true,
                None,
            )
            .await;
        client
    }

    async fn page(&mut self, path: &str) -> (StatusCode, String) {
        self.send(Method::GET, path, "", false, None).await
    }
}

/// The parity read cases' groups: a public one with a bio and flair
/// (admin owner, user1 and user2 members), a members-only one user1 owns
/// with a request from user0, one whose members only its owners see.
async fn custom_groups(db: &TestDb) {
    for sql in [
        "INSERT INTO groups (id, name, full_name, bio_raw, bio_cooked, visibility_level, members_visibility_level, public_admission, public_exit, flair_icon, flair_bg_color, mentionable_level, messageable_level, user_count, created_at, updated_at) \
         VALUES (9001, 'testers', 'test team', 'We test things.', '<p>We test <a href=\"https://example.com\">things</a>.</p>', 0, 0, TRUE, TRUE, 'heart', 'ff0000', 99, 99, 3, now(), now())",
        "INSERT INTO groups (id, name, visibility_level, members_visibility_level, allow_membership_requests, user_count, created_at, updated_at) \
         VALUES (9002, 'members_only', 2, 2, TRUE, 1, now(), now())",
        "INSERT INTO groups (id, name, full_name, visibility_level, members_visibility_level, user_count, created_at, updated_at) \
         VALUES (9003, 'hidden_members', 'Hidden Members', 1, 4, 2, now(), now())",
        "INSERT INTO group_users (group_id, user_id, owner, created_at, updated_at) VALUES \
         (9001, 1, TRUE, now(), now()), (9001, 3, FALSE, now(), now()), (9001, 4, FALSE, now(), now()), \
         (9002, 3, TRUE, now(), now()), (9003, 2, FALSE, now(), now()), (9003, 4, TRUE, now(), now())",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
async fn visitors_see_the_public_groups_cards() {
    let db = TestDb::new().await;
    custom_groups(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, html) = Client::anonymous(st).page("/groups").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Groups - "), "{html}");
    assert!(html.contains("groups-page\""));
    assert!(html.contains(
        "<a class=\"group-box\" data-group-name=\"testers\" href=\"/g/testers/members\">"
    ));
    // Only the public group: the others need a login.
    assert!(!html.contains("data-group-name=\"moderators\""));
    assert!(html.contains("<span class=\"group-info-name\">test team</span><span class=\"group-info-mention-name\">@testers</span>"));
    // A description with a link waits in a template for groups.js.
    assert!(html.contains("<template class=\"group-description-html\">We test <a href=\"https://example.com\">things</a>.</template>"));
    assert!(html.contains("group-index-join"));
    // Only public and closed to filter by, and no New Group button.
    assert!(html.contains("data-value=\"public\""));
    assert!(!html.contains("data-value=\"my\""));
    assert!(!html.contains("groups-header-new"));
    assert!(html.contains("/assets/groups.js"));
}

#[tokio::test]
async fn members_see_their_groups_and_staff_create_them() {
    let db = TestDb::new().await;
    custom_groups(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, html) = user1.page("/groups?type=my").await;
    assert!(html.contains("data-group-name=\"members_only\""), "{html}");
    assert!(html.contains("<span class=\"is-group-owner\">"));
    assert!(html.contains("group-index-leave"));
    assert!(!html.contains("data-group-name=\"hidden_members\""));
    assert!(html.contains(
        "select-kit-row is-highlighted is-selected\"><span class=\"name\">My Groups</span>"
    ));
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, html) = admin.page("/groups").await;
    assert!(html.contains("groups-header-new"));
    assert!(html.contains("<span class=\"d-button-label\">New Group</span>"));
    assert!(html.contains("data-group-name=\"trust_level_0\""));
    // The sidebar's Groups link is current.
    assert!(html.contains("data-list-item-name=\"groups\"><a class=\"active sidebar-section-link"));
}

#[tokio::test]
async fn a_groups_page_lists_its_members() {
    let db = TestDb::new().await;
    custom_groups(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, html) = user1.page("/g/testers").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<title>Members - testers - "), "{html}");
    assert!(html.contains("<div class=\"container group group-testers\">"));
    assert!(html.contains("<div class=\"group-bio\"><p>We test"));
    assert!(html.contains("group-message-button"));
    assert!(html.contains("<a class=\"active members\" title=\"Members\" href=\"/g/testers\">"));
    assert!(html.contains("<span class=\"count\">(3)</span>"));
    // Owners first, then by username.
    let body = &html[html.find("directory-table__body").unwrap()..];
    let admin = body.find("data-username=\"admin\"").unwrap();
    let u1 = body.find("data-username=\"user1\"").unwrap();
    let u2 = body.find("data-username=\"user2\"").unwrap();
    assert!(admin < u1 && u1 < u2);
    assert!(html.contains("Owner<br>"));
    // A member manages nothing.
    assert!(!html.contains("group-members-add"));
    // The owner of a group manages its members.
    let (_, html) = user1.page("/g/members_only").await;
    assert!(html.contains("group-members-add"), "{html}");
    assert!(html.contains("<a class=\"requests\""));
    // Hidden members: the forbidden message.
    let (_, html) = user1.page("/g/hidden_members").await;
    assert!(
        html.contains("<br><div>You&#x27;re not allowed to view the members.</div>"),
        "{html}"
    );
    // The JSON when asked for it.
    let (status, json) = user1
        .send(
            Method::GET,
            "/g/testers",
            "",
            false,
            Some("application/json"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json["group"]["name"], "testers");
    let mut visitor = Client::anonymous(st);
    let (status, html) = visitor.page("/g/members_only").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(html.contains("<html"));
}
