//! discourse-user-notes: UserNotesController (Rails' request spec and the
//! guards around it), the notes kept in plugin_store_rows with their
//! count, the official warning's note (user_warning_spec), the
//! `user_notes` report, the keys it adds to users, admin users and posts,
//! and its buttons on the server-rendered pages.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const BOOL: i32 = 5;
const INTEGER: i32 = 3;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::ACCEPT, "application/json");
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
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
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

    async fn json(&mut self, method: Method, path: &str, body: &str) -> (StatusCode, Value) {
        let (status, text) = self.send(method, path, body).await;
        (
            status,
            serde_json::from_str(&text).unwrap_or_else(|_| json!({ "text": text })),
        )
    }

    async fn anonymous(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state).await;
        client
            .send(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
            )
            .await;
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", "").await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> String {
        let cookie: Vec<String> = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let request = Request::get(path)
            .header(header::HOST, "test.localhost")
            .header(header::COOKIE, cookie.join("; "));
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// The plugin on, user3 a moderator (the fixture has none).
async fn setup() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    set_setting(&db.pool, "user_notes_enabled", BOOL, "t").await;
    sqlx::query("UPDATE users SET moderator = TRUE WHERE username = 'user3'")
        .execute(&db.pool)
        .await
        .unwrap();
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

/// The stored notes of a user (`PluginStore.get("user_notes", "notes:<id>")`).
async fn stored(db: &TestDb, user_id: i32) -> Option<Value> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT value FROM plugin_store_rows WHERE plugin_name = 'user_notes' AND key = $1",
    )
    .bind(format!("notes:{user_id}"))
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    value.map(|v| serde_json::from_str(&v).unwrap())
}

async fn count_field(db: &TestDb, user_id: i32) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = 'user_notes_count'",
    )
    .bind(user_id)
    .fetch_all(&db.pool)
    .await
    .unwrap()
}

/// `DiscourseUserNotes.add_note(user, raw, created_by, opts)` as Rails
/// leaves the row, for notes made before the request under test.
async fn seed_note(db: &TestDb, user_id: i32, raw: &str, by: i32, extra: Value) {
    let mut note = json!({
        "id": format!("seeded{by}{}", raw.len()),
        "user_id": user_id,
        "raw": raw,
        "created_by": by,
        "created_at": "2026-10-20T10:00:00.000Z",
    });
    for (k, v) in extra.as_object().unwrap() {
        note[k] = v.clone();
    }
    let mut notes = stored(db, user_id).await.unwrap_or(json!([]));
    notes.as_array_mut().unwrap().push(note);
    sqlx::query("DELETE FROM plugin_store_rows WHERE plugin_name = 'user_notes' AND key = $1")
        .bind(format!("notes:{user_id}"))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO plugin_store_rows (plugin_name, key, type_name, value) VALUES ('user_notes', $1, 'JSON', $2)",
    )
    .bind(format!("notes:{user_id}"))
    .bind(notes.to_string())
    .execute(&db.pool)
    .await
    .unwrap();
    let n = notes.as_array().unwrap().len().to_string();
    sqlx::query("DELETE FROM user_custom_fields WHERE user_id = $1 AND name = 'user_notes_count'")
        .bind(user_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) VALUES ($1, 'user_notes_count', $2, now(), now())",
    )
    .bind(user_id)
    .bind(n)
    .execute(&db.pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn rejects_notes_longer_than_the_maximum_post_length() {
    let (db, st) = setup().await;
    set_setting(&db.pool, "max_post_length", INTEGER, "10").await;
    let mut moderator = Client::logged_in(st, "user3").await;
    let (status, body) = moderator
        .json(
            Method::POST,
            "/user_notes",
            &format!("user_note[user_id]=2&user_note[raw]={}", "a".repeat(11)),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["errors"],
        json!(["User note must be 10 characters or fewer."])
    );
    assert!(stored(&db, 2).await.is_none());
}

#[tokio::test]
async fn a_pm_post_the_moderator_cannot_see_is_not_leaked_or_kept() {
    let (db, st) = setup().await;
    let mut moderator = Client::logged_in(st, "user3").await;
    // Post 50: admin's message to user1 and user2.
    let (status, note) = moderator
        .json(
            Method::POST,
            "/user_notes",
            "user_note[user_id]=2&user_note[raw]=probe+note&user_note[post_id]=50",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{note}");
    assert!(note.get("post_url").is_none());
    assert!(note.get("post_title").is_none());
    let notes = stored(&db, 2).await.unwrap();
    assert!(notes[0].get("post_id").is_none());
    assert_eq!(notes[0]["raw"], "probe note");
}

#[tokio::test]
async fn the_index_hides_posts_the_moderator_cannot_see() {
    let (db, st) = setup().await;
    seed_note(&db, 2, "old note", 1, json!({"post_id": 50})).await;
    let mut moderator = Client::logged_in(st.clone(), "user3").await;
    let (status, body) = moderator
        .json(Method::GET, "/user_notes?user_id=2", "")
        .await;
    assert_eq!(status, StatusCode::OK);
    let note = &body["user_notes"][0];
    assert!(note.get("post_url").is_none());
    assert!(note.get("post_title").is_none());
    assert!(note.get("post_id").is_none());

    // An admin sees every post: the note keeps it, as given.
    let mut admin = Client::logged_in(st, "admin").await;
    let (_, body) = admin.json(Method::GET, "/user_notes?user_id=2", "").await;
    let note = &body["user_notes"][0];
    assert_eq!(note["post_id"], 50);
    assert_eq!(
        note["post_url"],
        "/t/parity-fixture-pm-admin-to-user1-and-user2/43/1"
    );
    assert_eq!(note["post_title"], Value::Null);
}

#[tokio::test]
async fn notes_are_added_listed_newest_first_and_removed() {
    let (db, st) = setup().await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, first) = admin
        .json(
            Method::POST,
            "/user_notes",
            "user_note[user_id]=2&user_note[raw]=first&user_note[post_id]=36",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let keys: Vec<&String> = first.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "id",
            "user_id",
            "raw",
            "created_by",
            "created_at",
            "can_delete",
            "post_id",
            "post_url",
            "post_title",
            "reviewable_id"
        ]
    );
    assert_eq!(first["user_id"], 2);
    assert_eq!(first["created_by"]["username"], "admin");
    assert_eq!(first["created_by"]["id"], 1);
    assert_eq!(first["can_delete"], true);
    // The param as it came, a string from a form.
    assert_eq!(first["post_id"], "36");
    assert_eq!(
        first["post_url"],
        "/t/parity-fixture-replies-and-posters/35/2"
    );
    assert_eq!(first["reviewable_id"], Value::Null);
    assert_eq!(first["id"].as_str().unwrap().len(), 32);

    let (_, second) = admin
        .json(
            Method::POST,
            "/user_notes",
            "user_note[user_id]=2&user_note[raw]=second&user_note[reviewable_id]=7",
        )
        .await;
    assert!(second.get("post_id").is_none());
    assert_eq!(second["reviewable_id"], "7");
    assert_eq!(count_field(&db, 2).await, ["2"]);

    let (status, index) = admin.json(Method::GET, "/user_notes?user_id=2", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(index["extras"], json!({"username": "user0"}));
    let raws: Vec<&Value> = index["user_notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| &n["raw"])
        .collect();
    assert_eq!(raws, [&json!("second"), &json!("first")]);

    let (status, body) = admin
        .json(
            Method::DELETE,
            &format!("/user_notes/{}?user_id=2", first["id"].as_str().unwrap()),
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"success": "OK"}));
    assert_eq!(stored(&db, 2).await.unwrap().as_array().unwrap().len(), 1);
    assert_eq!(count_field(&db, 2).await, ["1"]);

    // The last one goes with its row; the count stays, at zero.
    admin
        .json(
            Method::DELETE,
            &format!("/user_notes/{}?user_id=2", second["id"].as_str().unwrap()),
            "",
        )
        .await;
    assert!(stored(&db, 2).await.is_none());
    assert_eq!(count_field(&db, 2).await, ["0"]);
}

#[tokio::test]
async fn moderators_delete_only_while_the_setting_allows() {
    let (db, st) = setup().await;
    seed_note(&db, 2, "kept", 1, json!({})).await;
    set_setting(&db.pool, "user_notes_moderators_delete", BOOL, "f").await;
    let mut moderator = Client::logged_in(st, "user3").await;
    let (_, index) = moderator
        .json(Method::GET, "/user_notes?user_id=2", "")
        .await;
    assert_eq!(index["user_notes"][0]["can_delete"], false);
    let (status, _) = moderator
        .json(Method::DELETE, "/user_notes/seeded14?user_id=2", "")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(stored(&db, 2).await.is_some());
    // An unknown user is not found before the permission is checked.
    let (status, _) = moderator
        .json(Method::DELETE, "/user_notes/seeded14?user_id=999", "")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn only_staff_reach_the_notes_and_only_while_enabled() {
    let (db, st) = setup().await;
    let mut anon = Client::anonymous(st.clone()).await;
    let (status, _) = anon.json(Method::GET, "/user_notes?user_id=2", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let mut member = Client::logged_in(st.clone(), "user1").await;
    let (status, body) = member.json(Method::GET, "/user_notes?user_id=2", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error_type"], "invalid_access");
    let (status, _) = member
        .json(
            Method::POST,
            "/user_notes",
            "user_note[user_id]=2&user_note[raw]=x",
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = member.send(Method::GET, "/user-notes/2", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let mut admin = Client::logged_in(st, "admin").await;
    let (status, _) = admin.json(Method::GET, "/user_notes?user_id=999", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = admin
        .json(
            Method::POST,
            "/user_notes",
            "user_note[user_id]=999&user_note[raw]=x",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    set_setting(&db.pool, "user_notes_enabled", BOOL, "f").await;
    let (status, _) = admin.json(Method::GET, "/user_notes?user_id=2", "").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_official_warning_leaves_a_note_in_the_site_language() {
    let (db, st) = setup().await;
    let mut conn = db.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    let urls = discourse_rs::url::Urls {
        config: &st.config,
        settings: &settings,
    };
    let warning = discourse_rs::plugins::user_notes::UserWarning {
        user_id: 2,
        created_by_id: 1,
        topic_id: 35,
    };
    discourse_rs::plugins::user_warning_created(&mut conn, &settings, &urls, &st.i18n, &warning)
        .await
        .unwrap();
    // A second, as a French request would make it: the same words.
    discourse_rs::plugins::user_warning_created(&mut conn, &settings, &urls, &st.i18n, &warning)
        .await
        .unwrap();
    let notes = stored(&db, 2).await.unwrap();
    let base = urls.base_url().unwrap();
    assert_eq!(
        notes[0]["raw"],
        format!(
            "Received an official warning from @admin -- [Parity fixture: replies and posters]({base}/t/parity-fixture-replies-and-posters/35)"
        )
    );
    assert_eq!(notes[0]["raw"], notes[1]["raw"]);
    assert_eq!(notes[0]["created_by"], -1);
    assert_eq!(notes[0]["topic_id"], 35);
    assert_eq!(count_field(&db, 2).await, ["2"]);

    // Not while the plugin is off.
    set_setting(&db.pool, "user_notes_enabled", BOOL, "f").await;
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    discourse_rs::plugins::user_warning_created(&mut conn, &settings, &urls, &st.i18n, &warning)
        .await
        .unwrap();
    assert_eq!(stored(&db, 2).await.unwrap().as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn the_report_lists_the_notes_of_the_period() {
    let (db, st) = setup().await;
    seed_note(&db, 2, "in range", 1, json!({})).await;
    let mut admin = Client::logged_in(st, "admin").await;
    let (status, body) = admin
        .json(
            Method::GET,
            "/admin/reports/user_notes.json?start_date=2026-10-01&end_date=2026-10-27",
            "",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let report = &body["report"];
    assert_eq!(report["type"], "user_notes");
    assert_eq!(report["title"], "User notes");
    assert_eq!(report["modes"], json!(["table"]));
    assert_eq!(report["labels"][0]["type"], "user");
    assert_eq!(
        report["labels"][0]["properties"]["avatar"],
        "user_avatar_template"
    );
    assert_eq!(
        report["labels"][2],
        json!({"type": "text", "property": "note", "title": "Note"})
    );
    let row = &report["data"][0];
    let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "created_at",
            "user_id",
            "username",
            "user_avatar_template",
            "moderator_id",
            "moderator_username",
            "moderator_avatar_template",
            "note"
        ]
    );
    assert_eq!(row["created_at"], "2026-10-20T10:00:00.000Z");
    assert_eq!(row["username"], "user0");
    assert_eq!(row["moderator_username"], "admin");
    assert_eq!(row["note"], "in range");

    let (_, body) = admin
        .json(
            Method::GET,
            "/admin/reports/user_notes.json?start_date=2026-10-21&end_date=2026-10-27",
            "",
        )
        .await;
    assert_eq!(body["report"]["data"], json!([]));
}

#[tokio::test]
async fn staff_see_the_count_on_users_admin_users_and_posts() {
    let (db, st) = setup().await;
    seed_note(&db, 2, "a note", 1, json!({})).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;
    let (_, card) = admin.json(Method::GET, "/u/user0/card.json", "").await;
    assert_eq!(card["user"]["custom_fields"]["user_notes_count"], "1");
    let (_, detailed) = admin.json(Method::GET, "/admin/users/2.json", "").await;
    assert_eq!(detailed["user_notes_count"], 1);
    let (_, topic) = admin.json(Method::GET, "/t/35.json", "").await;
    let posts = topic["post_stream"]["posts"].as_array().unwrap();
    let by_user0 = posts.iter().find(|p| p["user_id"] == 2).unwrap();
    assert_eq!(
        by_user0["user_custom_fields"],
        json!({"user_notes_count": "1"})
    );
    let by_user1 = posts.iter().find(|p| p["user_id"] == 3).unwrap();
    assert!(by_user1.get("user_custom_fields").is_none());

    let mut member = Client::logged_in(st, "user1").await;
    let (_, card) = member.json(Method::GET, "/u/user0/card.json", "").await;
    assert!(
        card["user"]["custom_fields"]
            .get("user_notes_count")
            .is_none()
    );
    let (_, topic) = member.json(Method::GET, "/t/35.json", "").await;
    assert!(
        topic["post_stream"]["posts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("user_custom_fields").is_none())
    );

    // Off: no key, and the field is not a staff field any more.
    set_setting(&db.pool, "user_notes_enabled", BOOL, "f").await;
    let (_, detailed) = admin.json(Method::GET, "/admin/users/2.json", "").await;
    assert!(detailed.get("user_notes_count").is_none());
    let (_, card) = admin.json(Method::GET, "/u/user0/card.json", "").await;
    assert!(
        card["user"]["custom_fields"]
            .get("user_notes_count")
            .is_none()
    );
}

#[tokio::test]
async fn the_pages_carry_the_notes_buttons_for_staff() {
    let (db, st) = setup().await;
    seed_note(&db, 2, "Watch **this** one", 1, json!({"post_id": 35})).await;
    let mut admin = Client::logged_in(st.clone(), "admin").await;

    let topic = admin.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(topic.contains("user-notes.js"));
    // The icon by user0's name, and not by user1's.
    assert!(
        topic
            .contains("<span class=\"user-notes-icon\" data-user-notes=\"2\" data-post-id=\"35\">")
    );
    assert!(!topic.contains("data-user-notes=\"3\" data-post-id=\"36\"><img"));
    assert!(topic.contains("btn btn-icon-text btn btn-transparent add-user-note"));
    assert!(topic.contains("data-user-notes=\"3\" data-post-id=\"36\" class=\"btn btn-icon-text btn btn-transparent add-user-note\""));

    let card = admin.page("/user-card/user0").await;
    assert!(card.contains("<div class=\"user-card-post-names-outlet show-user-notes-on-card\"><div class=\"show-user-notes-on-card\"><button class=\"btn-flat btn no-text\" data-user-notes=\"2\""));
    assert!(card.contains("title=\"User notes (1)\""));

    let profile = admin.page("/u/user0/summary").await;
    assert!(profile.contains("<li class=\"user-profile-controls-outlet show-notes-on-profile\">"));
    assert!(profile.contains("<span class=\"d-button-label\">User notes (1)</span>"));
    let other = admin.page("/u/user1/summary").await;
    assert!(other.contains("<span class=\"d-button-label\">User notes</span>"));

    let admin_page = admin.page("/admin/users/2/user0").await;
    assert!(admin_page.contains("show-user-notes-btn"));
    assert!(admin_page.contains("data-user-notes-kind=\"admin\""));

    let modal = admin.page("/user-notes/2?post_id=35").await;
    assert!(modal.contains("data-user-id=\"2\" data-post-id=\"35\" data-title=\"User notes\""));
    assert!(modal.contains("<div class=\"cooked\"><p>Watch <strong>this</strong> one</p></div>"));
    assert!(modal.contains("class=\"btn btn-default btn-small show-post\" href=\"/t/parity-fixture-replies-and-posters/35/1\""));
    assert!(modal.contains("remove-user-note"));
    assert!(modal.contains("<span class=\"d-button-label\">Add user note</span>"));

    // A member sees none of it.
    let mut member = Client::logged_in(st, "user1").await;
    let topic = member
        .page("/t/parity-fixture-replies-and-posters/35")
        .await;
    assert!(!topic.contains("user-notes"));
    assert!(!topic.contains("add-user-note"));
    let card = member.page("/user-card/user0").await;
    assert!(!card.contains("show-user-notes-on-card"));
}
