//! Creating chat channels, ported from chat's request specs: POST
//! /chat/api/channels (Chat::CreateCategoryChannel), the staff permissions
//! hint the create modal reads, and a direct message name's validation
//! errors.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn request(&mut self, method: Method, path: &str, body: Option<Value>) -> (u16, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        let body = match body {
            Some(Value::String(form)) => {
                request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                Body::from(form)
            }
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let pair = v.split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            self.cookies.retain(|(n, _)| n != name);
            if !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn json(&mut self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let (status, text) = self.request(method, path, body).await;
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.json(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                Some(Value::String(format!("login={username}&password=password"))),
            )
            .await;
        client
    }

    /// A page as a browser asks for it: the status, the Location and the
    /// body.
    async fn page(&self, path: &str) -> (u16, Option<String>, String) {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let response = discourse_rs::app(self.state.clone())
            .oneshot(
                Request::get(path)
                    .header(header::HOST, "test.localhost")
                    .header(header::COOKIE, cookie.join("; "))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
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
}

async fn scalar<T>(db: &TestDb, sql: &str) -> T
where
    T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    sqlx::query_scalar(sql).fetch_one(&db.pool).await.unwrap()
}

#[tokio::test]
async fn staff_create_a_category_channel() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"chatable_id": 2, "name": "channel name", "description": "My new channel", "threading_enabled": false}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["channel"]["id"].as_i64().unwrap();
    assert_eq!(body["channel"]["slug"], "channel-name");
    assert_eq!(body["channel"]["title"], "channel name");
    assert_eq!(body["channel"]["description"], "My new channel");
    assert_eq!(body["channel"]["chatable_id"], 2);
    assert_eq!(
        body["channel"]["current_user_membership"]["following"],
        true
    );
    let (auto_join, threading, user_count, kind): (bool, bool, i32, String) = sqlx::query_as(
        "SELECT auto_join_users, threading_enabled, user_count, type FROM chat_channels WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(!auto_join && !threading);
    assert_eq!((user_count, kind.as_str()), (1, "CategoryChannel"));

    // A slug given; the same slug again is a record error; the same
    // name in the category an invalid parameter.
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"chatable_id": 2, "name": "other", "slug": "wow-so-cool"}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["channel"]["slug"], "wow-so-cool");
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"chatable_id": 2, "name": "new name", "slug": "wow-so-cool"}})),
        )
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body,
        json!({"errors": ["Slug is already in use"], "error_type": "record_invalid"})
    );
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"chatable_id": 2, "name": "other"}})),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["error_type"], "invalid_parameters");
    assert!(
        body["errors"][0]
            .as_str()
            .unwrap()
            .contains("A channel already exists for this category and name"),
        "{body}"
    );
    // The contract, a missing category, no channel param.
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"name": "x"}})),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["errors"], json!(["Category can't be blank"]));
    let (status, _) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"chatable_id": 999, "name": "x"}})),
        )
        .await;
    assert_eq!(status, 404);
    let (status, _) = admin
        .json(Method::POST, "/chat/api/channels", Some(json!({})))
        .await;
    assert_eq!(status, 400);

    // Auto join, which adds the category's active users.
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(
                json!({"channel": {"chatable_id": 2, "name": "everyone", "auto_join_users": true}}),
            ),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["channel"]["auto_join_users"], true);
}

#[tokio::test]
async fn only_staff_create_channels_while_public_channels_are_on() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let params = json!({"channel": {"chatable_id": 2, "name": "x"}});
    let (status, _) = user1
        .json(Method::POST, "/chat/api/channels", Some(params.clone()))
        .await;
    assert_eq!(status, 403);
    common::set_setting(&db.pool, "enable_public_channels", 5, "f").await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .json(Method::POST, "/chat/api/channels", Some(params))
        .await;
    assert_eq!(status, 422);
    assert_eq!(
        body,
        json!({"errors": ["Unable to create the channel because public channels are disabled. An admin can enable the 'enable public channels' site setting."]})
    );
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channels").await,
        2
    );
}

#[tokio::test]
async fn staff_read_a_categorys_chat_permissions() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (status, body) = admin
        .json(
            Method::GET,
            "/chat/api/category-chatables/3/permissions",
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({"allowed_groups": ["@staff"], "members_count": 3, "private": true})
    );
    let (_, body) = admin
        .json(
            Method::GET,
            "/chat/api/category-chatables/2/permissions",
            None,
        )
        .await;
    assert_eq!(
        body,
        json!({"allowed_groups": ["@everyone"], "private": false})
    );
    let (status, _) = admin
        .json(
            Method::GET,
            "/chat/api/category-chatables/-99/permissions",
            None,
        )
        .await;
    assert_eq!(status, 404);
    // StaffConstraint: no route for anyone else.
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, _) = user1
        .json(
            Method::GET,
            "/chat/api/category-chatables/2/permissions",
            None,
        )
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn an_invalid_group_dm_name_keeps_the_channel() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let before: i64 = scalar(&db, "SELECT COUNT(*) FROM chat_channels").await;
    let (status, body) = user1
        .json(
            Method::POST,
            "/chat/api/direct-message-channels",
            Some(json!({"target_usernames": ["user0", "user2"], "name": " "})),
        )
        .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body,
        json!({"errors": ["Name can't be blank"], "error_type": "record_invalid"})
    );
    // No transaction around the service: the DM and its channel stay.
    assert_eq!(
        scalar::<i64>(&db, "SELECT COUNT(*) FROM chat_channels").await,
        before + 1
    );
}

#[tokio::test]
async fn every_page_carries_the_channel_menus() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let admin = Client::logged_in(st.clone(), "admin").await;
    let (status, _, html) = admin.page("/latest").await;
    assert_eq!(status, 200);
    assert!(html.contains("/assets/chat-channel-menus.js"), "{html}");
    assert!(html.contains("&quot;staff&quot;:true"), "{html}");
    assert!(html.contains("&quot;create&quot;:&quot;New channel...&quot;"));
    assert!(html.contains(
        "&quot;channels&quot;:{&quot;filter&quot;:&quot;all&quot;,&quot;sort&quot;:&quot;alphabetical&quot;}"
    ));
    // The sidebar's channel menus and the browse page's New channel.
    assert!(html.contains("data-sidebar-action-id=\"channelListOptions\""));
    assert!(html.contains("sidebar-section-hover-button"));
    let (_, _, browse) = admin.page("/chat/browse/open").await;
    assert!(browse.contains("c-navbar__new-channel-button"));
    let user1 = Client::logged_in(st, "user1").await;
    let (_, _, html) = user1.page("/latest").await;
    assert!(html.contains("&quot;staff&quot;:false"), "{html}");
}

#[tokio::test]
async fn a_filter_that_leaves_nothing_offers_to_show_all() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (status, body) = user1
        .json(
            Method::POST,
            "/chat/api/direct-message-channels",
            Some(json!({"target_usernames": ["user0"]})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["channel"]["id"].as_i64().unwrap();
    let row = format!("data-chat-channel-id=\"{id}\"");
    let sidebar_link = format!("sidebar-section-link sidebar-row channel-{id}\"");
    // ChatChannelListFilters' unread (2): the DM has nothing unread.
    sqlx::query("UPDATE user_options SET chat_channel_list_filter_dms = 2 WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let empty_state = "<div class=\"empty-state__title\" data-test-title=\"\">No channels match this filter.</div><div class=\"empty-state__cta\"><button class=\"btn btn-primary\" type=\"button\" data-action=\"showAllChannels\"><span class=\"d-button-label\">Show all channels</span></button></div>";
    let show_all = "chat-channel-list-filter-toggle title-action\" title=\"Show all channels\" type=\"button\"><svg class=\"fa d-icon d-icon-filter-circle-xmark ";
    let apply = "chat-channel-list-filter-toggle title-action\" title=\"Apply filters to channels\" type=\"button\"><svg class=\"fa d-icon d-icon-filter ";
    let sidebar_show_all =
        "data-sidebar-action-id=\"toggleChannelFilter\" title=\"Show all channels\"";
    let sidebar_apply =
        "data-sidebar-action-id=\"toggleChannelFilter\" title=\"Apply filters to channels\"";

    let (status, _, html) = user1.page("/chat/direct-messages").await;
    assert_eq!(status, 200, "{html}");
    assert!(html.contains(empty_state), "{html}");
    assert!(html.contains(show_all), "{html}");
    assert!(html.contains(sidebar_show_all), "{html}");
    assert!(!html.contains(&row), "{html}");
    assert!(!html.contains(&sidebar_link), "{html}");

    // Another section's bypass leaves this one filtered.
    user1
        .cookies
        .push(("chat_list_filter_bypass".into(), "starred.channels".into()));
    let (_, _, html) = user1.page("/chat/direct-messages").await;
    assert!(html.contains(empty_state), "{html}");
    assert!(!html.contains(&row), "{html}");

    // Bypassed (showAllChannels): every DM, in the list and the sidebar,
    // and the toggles apply the filter again. The saved filter stays.
    user1.cookies.pop();
    user1
        .cookies
        .push(("chat_list_filter_bypass".into(), "starred.dms".into()));
    let (status, _, html) = user1.page("/chat/direct-messages").await;
    assert_eq!(status, 200, "{html}");
    assert!(!html.contains("No channels match this filter."), "{html}");
    assert!(html.contains(&row), "{html}");
    assert!(html.contains(&sidebar_link), "{html}");
    assert!(html.contains(apply), "{html}");
    assert!(!html.contains(show_all), "{html}");
    assert!(html.contains(sidebar_apply), "{html}");
    assert!(!html.contains(sidebar_show_all), "{html}");
    let (_, _, latest) = user1.page("/latest").await;
    assert!(latest.contains(&sidebar_link), "{latest}");
    assert!(latest.contains(
        "&quot;dms&quot;:{&quot;filter&quot;:&quot;unread&quot;,&quot;sort&quot;:&quot;priority&quot;}"
    ), "{latest}");

    // The mobile view's navbar holds the toggle
    // (ChatNavbarChannelListOptionsButton).
    user1.cookies.push(("mobile_view".into(), "1".into()));
    let (_, _, html) = user1.page("/chat/direct-messages").await;
    assert!(
        html.contains(&format!("<nav class=\"c-navbar__actions\"><button class=\"btn no-text btn-icon btn-transparent {apply}")),
        "{html}"
    );
    assert!(html.contains(&row), "{html}");
    user1.cookies.pop();

    // Nothing is bypassed once the filter is all (setFilter applies it).
    sqlx::query("UPDATE user_options SET chat_channel_list_filter_dms = 0 WHERE user_id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let (_, _, html) = user1.page("/chat/direct-messages").await;
    assert!(html.contains(&row), "{html}");
    assert!(!html.contains("chat-channel-list-filter-toggle"), "{html}");
    assert!(!html.contains("toggleChannelFilter"), "{html}");
}

#[tokio::test]
async fn the_mobile_channels_list_has_the_filter_toggle() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .json(
            Method::POST,
            "/chat/api/channels",
            Some(json!({"channel": {"name": "Filtered away", "chatable_id": 2}})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let id = body["channel"]["id"].as_i64().unwrap();
    let row = format!("data-chat-channel-id=\"{id}\"");
    // ChatChannelListFilters' mentions (3): the channel has none.
    sqlx::query("UPDATE user_options SET chat_channel_list_filter = 3 WHERE user_id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    admin.cookies.push(("mobile_view".into(), "1".into()));
    let (status, _, html) = admin.page("/chat/channels").await;
    assert_eq!(status, 200, "{html}");
    assert!(
        html.contains("<nav class=\"c-navbar__actions\"><button class=\"btn no-text btn-icon btn-transparent chat-channel-list-filter-toggle title-action\" title=\"Show all channels\""),
        "{html}"
    );
    assert!(html.contains("data-action=\"showAllChannels\""), "{html}");
    assert!(!html.contains(&row), "{html}");

    admin
        .cookies
        .push(("chat_list_filter_bypass".into(), "channels".into()));
    let (_, _, html) = admin.page("/chat/channels").await;
    assert!(
        html.contains("<nav class=\"c-navbar__actions\"><button class=\"btn no-text btn-icon btn-transparent chat-channel-list-filter-toggle title-action\" title=\"Apply filters to channels\""),
        "{html}"
    );
    assert!(!html.contains("No channels match this filter."), "{html}");
    assert!(html.contains(&row), "{html}");
}
