//! The groups pages' HTML: the directory (/groups) from groups#index with
//! its cards and filters, and a group's members page (/g/:name) from
//! groups#show and groups#members. The documents are covered by the
//! parity/writes/groups read cases.

use crate::common;

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

/// The members route as a page: Ember's group.members route transitions to
/// group.index, the group's page at its lowercased name (the group route's
/// serialize); a group the viewer can't see is the 404 page. The JSON
/// stays groups#members'.
#[tokio::test]
async fn the_members_page_redirects_to_the_groups_page() {
    let db = TestDb::new().await;
    custom_groups(&db).await;
    sqlx::query("UPDATE groups SET name = 'Testers' WHERE id = 9001")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let get = |path: &'static str, accept: Option<&'static str>| {
        let st = st.clone();
        async move {
            let mut request = Request::builder()
                .uri(path)
                .header(header::HOST, "test.localhost");
            if let Some(accept) = accept {
                request = request.header(header::ACCEPT, accept);
            }
            let response = discourse_rs::app(st)
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = response.status();
            let location = response
                .headers()
                .get(header::LOCATION)
                .map(|l| l.to_str().unwrap().to_string());
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            (
                status,
                location,
                String::from_utf8_lossy(&bytes).into_owned(),
            )
        }
    };
    for path in [
        "/g/testers/members",
        "/g/TESTERS/members",
        "/groups/testers/members",
    ] {
        let (status, location, _) = get(path, None).await;
        assert!(status.is_redirection(), "{path}: {status}");
        assert_eq!(location.as_deref(), Some("/g/testers"), "{path}");
    }
    let (status, location, html) = get("/g/members_only/members", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(location, None);
    assert!(html.contains("<html"), "{html}");
    let (status, _, html) = get("/g/nobody/members", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(html.contains("<html"), "{html}");
    for (path, accept) in [
        ("/g/testers/members.json", None),
        ("/g/testers/members", Some("application/json")),
    ] {
        let (status, _, json) = get(path, accept).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["meta"]["total"], 3, "{path}: {json}");
    }
}

/// The Request button: a member's carries the group's template for
/// RequestGroupMembershipForm with the form's strings on the page, a
/// visitor's page is as it was; and GroupMembershipRequester's guards on
/// the request itself.
#[tokio::test]
async fn a_member_requests_membership_from_the_groups_page() {
    let db = TestDb::new().await;
    custom_groups(&db).await;
    for sql in [
        "INSERT INTO groups (id, name, visibility_level, members_visibility_level, allow_membership_requests, membership_request_template, user_count, created_at, updated_at) \
         VALUES (9004, 'askers', 0, 0, TRUE, 'I am \"<b>\" & ', 1, now(), now())",
        "INSERT INTO group_users (group_id, user_id, owner, created_at, updated_at) VALUES (9004, 1, TRUE, now(), now())",
    ] {
        sqlx::query(sql).execute(&db.pool).await.unwrap();
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    let (status, html) = Client::anonymous(st.clone()).page("/g/askers").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(
        "<div class=\"group-membership-button\"><button class=\"btn btn-icon-text btn-default group-index-request\" data-group-id=\"9004\" data-group-name=\"askers\" type=\"button\">"
    ), "{html}");
    assert!(!html.contains("group-membership-strings"));
    assert!(!html.contains("group-message-button"));

    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let (status, html) = user2.page("/g/askers").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(
        "group-index-request\" data-group-id=\"9004\" data-group-name=\"askers\" data-membership-request-template=\"I am &quot;&lt;b&gt;&quot; &amp; \" type=\"button\">"
    ), "{html}");
    assert!(html.contains("<script type=\"application/json\" id=\"group-membership-strings\">{"));
    assert!(html.contains("\"title\":\"Request to join @%{group_name}\""));
    // The directory's cards have the button too.
    let (_, html) = user2.page("/groups").await;
    assert!(html.contains("data-group-name=\"askers\" data-membership-request-template="));
    assert!(html.contains("id=\"group-membership-strings\""));
    // An owner has no button, and the Message button names its group.
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, html) = admin.page("/g/askers").await;
    assert!(!html.contains("group-index-request"));
    let (_, html) = admin.page("/g/testers").await;
    assert!(html.contains("btn-primary group-message-button\" data-group-name=\"testers\""));

    let requests = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM group_requests WHERE group_id = 9004")
            .fetch_one(&db.pool)
            .await
            .unwrap()
    };
    let path = "/groups/askers/request_membership.json";
    // raise Discourse::InvalidAccess if guardian.is_silenced?
    sqlx::query("UPDATE users SET silenced_till = now() + interval '1 day' WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body) = user2
        .send(Method::POST, path, "reason=please", true, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(requests().await, 0);
    sqlx::query("UPDATE users SET silenced_till = NULL WHERE id = 4")
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, body) = user2.send(Method::POST, path, "", true, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = user2
        .send(Method::POST, path, "reason=please", true, None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let url = json["relative_url"].as_str().unwrap();
    assert!(
        url.starts_with("/t/membership-request-for-askers/"),
        "{body}"
    );
    let allowed: Vec<String> = sqlx::query_scalar(
        "SELECT u.username FROM topic_allowed_users a JOIN users u ON u.id = a.user_id \
         JOIN topics t ON t.id = a.topic_id WHERE t.title = 'Membership Request for @askers' AND t.archetype = 'private_message' ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(allowed, ["admin", "user2"]);
    // AlreadyRequested
    let (status, body) = user2
        .send(Method::POST, path, "reason=again", true, None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body.contains("You have already requested membership for this group."),
        "{body}"
    );
    assert_eq!(requests().await, 1);
    // A member may not ask.
    let (status, _) = admin
        .send(Method::POST, path, "reason=me", true, None)
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
