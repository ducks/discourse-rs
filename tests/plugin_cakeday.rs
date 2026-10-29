//! discourse-cakeday, ported from the plugin's specs: the anniversaries
//! and birthdays lists (spec/integration/cakeday_spec.rb), the post and
//! user serializers' keys (spec/serializers), the date of birth
//! migration (spec/jobs), the emoji on the profile
//! (spec/system/cakeday_user_summary_spec.rb), and the rest of its client
//! on the server-rendered pages.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use chrono::{TimeZone, Utc};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::Value;
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

    async fn request(
        &mut self,
        method: Method,
        path: &str,
        body: &str,
        xhr: bool,
    ) -> (StatusCode, Option<String>, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            request = request.header("x-requested-with", "XMLHttpRequest");
        }
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
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            location,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn logged_in(state: AppState, username: &str) -> Client {
        let mut client = Client::anonymous(state);
        let (_, _, csrf) = client
            .request(Method::GET, "/session/csrf.json", "", true)
            .await;
        let csrf: Value = serde_json::from_str(&csrf).unwrap();
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
        client
            .request(
                Method::POST,
                "/session",
                &format!("login={username}&password=password"),
                true,
            )
            .await;
        client
    }

    async fn json(&mut self, path: &str) -> (StatusCode, Value) {
        let (status, _, body) = self.request(Method::GET, path, "", true).await;
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> (StatusCode, Option<String>, String) {
        self.request(Method::GET, path, "", false).await
    }
}

async fn exec(db: &TestDb, sql: &str) {
    sqlx::query(sql)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// Both of the plugin's lists on.
async fn enable(db: &TestDb) {
    set_setting(&db.pool, "cakeday_enabled", 5, "t").await;
    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "t").await;
}

/// `Fabricate(:user, created_at:, date_of_birth:)`, a copy of user3 with
/// its options; `name` blank for none.
async fn fabricate(
    db: &TestDb,
    id: i32,
    username: &str,
    name: &str,
    created_at: &str,
    dob: Option<&str>,
) {
    let dob = dob.map_or("NULL".to_string(), |d| format!("'{d}'::date"));
    exec(
        db,
        &format!(
            "INSERT INTO users SELECT (jsonb_populate_record(u, jsonb_build_object(\
               'id', {id}, 'username', '{username}', 'username_lower', lower('{username}'), \
               'name', NULLIF('{name}', ''), 'created_at', '{created_at}'::timestamp, \
               'date_of_birth', {dob}))).* FROM users u WHERE u.id = 5"
        ),
    )
    .await;
    exec(
        db,
        &format!(
            "INSERT INTO user_options SELECT (jsonb_populate_record(o, jsonb_build_object('user_id', {id}))).* \
             FROM user_options o WHERE o.user_id = 5"
        ),
    )
    .await;
}

fn ids(doc: &Value, key: &str) -> Vec<i64> {
    doc[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} in {doc}"))
        .iter()
        .map(|u| u["id"].as_i64().unwrap())
        .collect()
}

fn pin(at: chrono::DateTime<Utc>) {
    discourse_rs::clock::pin(Some(at));
}

/// spec/integration: `Time.zone.local(2016, 9, 30)`, a September 30th.
fn september_30() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0).unwrap()
}

#[tokio::test]
async fn an_anonymous_visitor_is_refused() {
    let db = TestDb::new().await;
    enable(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let (status, doc) = Client::anonymous(st)
        .json("/cakeday/anniversaries.json")
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(doc["error_type"], "not_logged_in");
}

#[tokio::test]
async fn a_list_is_not_found_while_its_setting_is_off() {
    let db = TestDb::new().await;
    enable(&db).await;
    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    assert_eq!(
        user1.json("/cakeday/birthdays.json").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        user1.json("/cakeday/anniversaries.json").await.0,
        StatusCode::OK
    );
    set_setting(&db.pool, "cakeday_enabled", 5, "f").await;
    assert_eq!(
        user1.json("/cakeday/anniversaries.json").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn anniversaries_by_month_today_tomorrow_and_upcoming() {
    let db = TestDb::new().await;
    enable(&db).await;
    // created_at = time - 1.year
    fabricate(&db, 101, "anni1", "", "2023-09-30 00:00", None).await;
    fabricate(&db, 102, "anni2", "", "2025-09-29 00:00", None).await;
    fabricate(&db, 103, "anni3", "", "2025-09-30 00:00", None).await;
    fabricate(&db, 104, "anni4", "", "2025-10-01 00:00", None).await;
    fabricate(&db, 105, "anni5", "", "2025-10-02 00:00", None).await;
    fabricate(&db, 106, "anni6", "", "2026-09-30 00:00", None).await;
    fabricate(&db, 107, "hidden", "", "2024-09-30 00:00", None).await;
    exec(
        &db,
        "UPDATE user_options SET hide_profile = TRUE WHERE user_id = 107",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    pin(september_30());

    let (status, doc) = user1.json("/cakeday/anniversaries.json?month=9").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&doc, "anniversaries"), [102, 101, 103]);
    assert_eq!(doc["total_rows_anniversaries"], 3);
    assert_eq!(
        doc["load_more_anniversaries"],
        "/cakeday/anniversaries?month=9&page=1"
    );
    // CakedayUserSerializer: BasicUserSerializer's, the title, the cakedate.
    let keys: Vec<&String> = doc["anniversaries"][0]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "username",
            "name",
            "avatar_template",
            "title",
            "cakedate"
        ]
    );
    assert_eq!(
        doc["anniversaries"][0]["cakedate"],
        "2025-09-29T00:00:00.000Z"
    );

    let (_, doc) = user1.json("/cakeday/anniversaries.json?filter=today").await;
    assert_eq!(ids(&doc, "anniversaries"), [101, 103]);
    assert_eq!(
        doc["load_more_anniversaries"],
        "/cakeday/anniversaries?filter=today&page=1"
    );
    let (_, doc) = user1.json("/cakeday/anniversaries/tomorrow.json").await;
    assert_eq!(ids(&doc, "anniversaries"), [104]);
    let (_, doc) = user1
        .json("/cakeday/anniversaries.json?filter=upcoming")
        .await;
    assert_eq!(ids(&doc, "anniversaries"), [105]);
    // The next page is empty.
    let (_, doc) = user1
        .json("/cakeday/anniversaries.json?filter=today&page=1")
        .await;
    assert_eq!(ids(&doc, "anniversaries"), Vec::<i64>::new());
    assert_eq!(doc["total_rows_anniversaries"], 2);
}

#[tokio::test]
async fn anniversaries_follow_the_viewers_timezone() {
    let db = TestDb::new().await;
    enable(&db).await;
    fabricate(&db, 101, "anni1", "", "2025-09-30 05:00", None).await;
    fabricate(&db, 102, "anni2", "", "2025-09-30 18:20", None).await;
    fabricate(&db, 103, "anni3", "", "2025-09-30 18:40", None).await;
    fabricate(&db, 104, "anni4", "", "2025-10-01 02:00", None).await;
    fabricate(&db, 107, "hidden", "", "2024-09-30 00:00", None).await;
    exec(
        &db,
        "UPDATE user_options SET hide_profile = TRUE WHERE user_id = 107",
    )
    .await;
    // Asia/Kolkata is 5.5 hours ahead of UTC.
    exec(
        &db,
        "UPDATE user_options SET timezone = 'Asia/Kolkata' WHERE user_id = 3",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    pin(september_30());

    let (_, doc) = user1.json("/cakeday/anniversaries.json?filter=today").await;
    let mut today = ids(&doc, "anniversaries");
    today.sort_unstable();
    assert_eq!(today, [101, 102]);
    let (_, doc) = user1
        .json("/cakeday/anniversaries.json?filter=tomorrow")
        .await;
    let mut tomorrow = ids(&doc, "anniversaries");
    tomorrow.sort_unstable();
    assert_eq!(tomorrow, [103, 104]);

    // Nuku'alofa, a Rails name with a quote, is Pacific/Tongatapu.
    exec(
        &db,
        "UPDATE user_options SET timezone = 'Nuku''alofa' WHERE user_id = 3",
    )
    .await;
    assert_eq!(
        user1
            .json("/cakeday/anniversaries.json?filter=today")
            .await
            .0,
        StatusCode::OK
    );
    // An unknown zone falls back to UTC.
    exec(
        &db,
        "UPDATE user_options SET timezone = 'Invalid/Timezone' WHERE user_id = 3",
    )
    .await;
    let (status, doc) = user1
        .json("/cakeday/anniversaries.json?filter=tomorrow")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids(&doc, "anniversaries"), [104]);
}

#[tokio::test]
async fn birthdays_by_month_today_tomorrow_and_upcoming() {
    let db = TestDb::new().await;
    enable(&db).await;
    fabricate(&db, 101, "bday1", "", "2026-01-01", Some("1904-09-28")).await;
    fabricate(&db, 102, "bday2", "", "2026-01-01", Some("1904-09-29")).await;
    fabricate(&db, 103, "bday3", "", "2026-01-01", Some("1904-09-30")).await;
    fabricate(&db, 104, "bday4", "", "2026-01-01", Some("1904-10-01")).await;
    fabricate(&db, 105, "bday5", "", "2026-01-01", Some("1904-10-02")).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    pin(september_30());

    let (_, doc) = user1.json("/cakeday/birthdays.json?month=9").await;
    assert_eq!(ids(&doc, "birthdays"), [101, 102, 103]);
    assert_eq!(doc["birthdays"][0]["cakedate"], "1904-09-28");
    assert_eq!(
        doc["load_more_birthdays"],
        "/cakeday/birthdays?month=9&page=1"
    );
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=today").await;
    assert_eq!(ids(&doc, "birthdays"), [103]);
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=tomorrow").await;
    assert_eq!(ids(&doc, "birthdays"), [104]);
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=upcoming").await;
    assert_eq!(ids(&doc, "birthdays"), [105]);
}

#[tokio::test]
async fn february_29th_is_march_1st_outside_leap_years() {
    let db = TestDb::with_clock().await;
    enable(&db).await;
    fabricate(&db, 101, "leap", "", "2026-01-01", Some("1904-02-29")).await;
    fabricate(&db, 102, "march", "", "2026-01-01", Some("1904-03-01")).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    db.pin_clock(Some(Utc.with_ymd_and_hms(2027, 3, 1, 12, 0, 0).unwrap()))
        .await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=today").await;
    assert_eq!(ids(&doc, "birthdays"), [101, 102]);
    db.pin_clock(Some(Utc.with_ymd_and_hms(2028, 3, 1, 12, 0, 0).unwrap()))
        .await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=today").await;
    assert_eq!(ids(&doc, "birthdays"), [102]);
}

#[tokio::test]
async fn birthdays_order_by_name_or_username_as_the_list_shows_them() {
    let db = TestDb::new().await;
    enable(&db).await;
    fabricate(
        &db,
        101,
        "alpha_zeta",
        "Zeta Alpha",
        "2026-01-01",
        Some("1904-09-30"),
    )
    .await;
    fabricate(
        &db,
        102,
        "zeta_alpha",
        "Alpha Zeta",
        "2026-01-01",
        Some("1904-09-30"),
    )
    .await;
    fabricate(&db, 103, "beta_omega", "", "2026-01-01", Some("1904-09-30")).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    pin(september_30());

    set_setting(&db.pool, "prioritize_username_in_ux", 5, "t").await;
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=today").await;
    assert_eq!(ids(&doc, "birthdays"), [101, 103, 102]);
    set_setting(&db.pool, "prioritize_username_in_ux", 5, "f").await;
    let (_, doc) = user1.json("/cakeday/birthdays.json?filter=today").await;
    assert_eq!(ids(&doc, "birthdays"), [102, 103, 101]);
}

/// Post 35, user0's, as `username` sees it.
async fn post_35(st: &AppState, username: Option<&str>) -> Value {
    let mut client = match username {
        Some(u) => Client::logged_in(st.clone(), u).await,
        None => Client::anonymous(st.clone()),
    };
    let (status, doc) = client.json("/t/35.json").await;
    assert_eq!(status, StatusCode::OK, "{doc}");
    doc["post_stream"]["posts"]
        .as_array()
        .and_then(|posts| posts.iter().find(|p| p["id"] == 35))
        .cloned()
        .expect("post 35")
}

#[tokio::test]
async fn posts_carry_the_posters_cakedate_and_birthdate() {
    let db = TestDb::new().await;
    enable(&db).await;
    exec(
        &db,
        "UPDATE users SET date_of_birth = '2017-04-05' WHERE id = 2",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;

    // user0 joined 2026-07-25 05:59 UTC, the 24th in their America/Denver.
    let own = post_35(&st, Some("user0")).await;
    assert_eq!(own["user_cakedate"], "2026-07-24");
    assert_eq!(own["user_birthdate"], "2017-04-05");
    let keys: Vec<&String> = own.as_object().unwrap().keys().collect();
    let at = keys.iter().position(|k| *k == "user_cakedate").unwrap();
    assert_eq!(keys[at + 1], "user_birthdate");
    let other = post_35(&st, Some("user1")).await;
    assert_eq!(other["user_cakedate"], "2026-07-25");

    // Not for anonymous readers.
    let anonymous = post_35(&st, None).await;
    assert!(anonymous.get("user_cakedate").is_none());
    assert!(anonymous.get("user_birthdate").is_none());

    // A hidden profile keeps them for its owner alone.
    exec(
        &db,
        "UPDATE user_options SET hide_profile = TRUE WHERE user_id = 2",
    )
    .await;
    let own = post_35(&st, Some("user0")).await;
    assert!(own.get("user_cakedate").is_some());
    assert!(own.get("user_birthdate").is_some());
    let other = post_35(&st, Some("user1")).await;
    assert!(other.get("user_cakedate").is_none());
    assert!(other.get("user_birthdate").is_none());
    exec(
        &db,
        "UPDATE user_options SET hide_profile = FALSE WHERE user_id = 2",
    )
    .await;

    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "f").await;
    let own = post_35(&st, Some("user0")).await;
    assert!(own.get("user_cakedate").is_some());
    assert!(own.get("user_birthdate").is_none());
    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "t").await;
    set_setting(&db.pool, "cakeday_enabled", 5, "f").await;
    let own = post_35(&st, Some("user0")).await;
    assert!(own.get("user_cakedate").is_none());
    assert!(own.get("user_birthdate").is_none());
}

#[tokio::test]
async fn users_and_their_cards_carry_the_cakedate_and_birthdate() {
    let db = TestDb::new().await;
    enable(&db).await;
    exec(
        &db,
        "UPDATE users SET date_of_birth = '2017-04-05' WHERE id = 3",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, doc) = user1.json("/u/user1.json").await;
    assert_eq!(doc["user"]["cakedate"], "2026-07-25");
    assert_eq!(doc["user"]["birthdate"], "2017-04-05");
    let (_, doc) = user1.json("/u/user1/card.json").await;
    let keys: Vec<&String> = doc["user"].as_object().unwrap().keys().collect();
    let at = keys.iter().position(|k| *k == "cakedate").unwrap();
    assert_eq!(keys[at + 1], "birthdate");
    // birthdate is there without a date of birth.
    let (_, doc) = user1.json("/u/user2/card.json").await;
    assert_eq!(doc["user"]["birthdate"], Value::Null);
    assert_eq!(doc["user"]["cakedate"], "2026-07-25");

    // Nothing for anonymous visitors.
    let (_, doc) = Client::anonymous(st.clone()).json("/u/user1.json").await;
    assert!(doc["user"].get("cakedate").is_none());
    assert!(doc["user"].get("birthdate").is_none());

    // A hidden profile keeps them for its owner.
    exec(
        &db,
        "UPDATE user_options SET hide_profile = TRUE WHERE user_id = 3",
    )
    .await;
    let (_, doc) = user1.json("/u/user1.json").await;
    assert_eq!(doc["user"]["birthdate"], "2017-04-05");
    let mut user2 = Client::logged_in(st.clone(), "user2").await;
    let (_, doc) = user2.json("/u/user1.json").await;
    assert!(doc["user"].get("cakedate").is_none());

    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "f").await;
    let (_, doc) = user1.json("/u/user1.json").await;
    assert!(doc["user"].get("cakedate").is_some());
    assert!(doc["user"].get("birthdate").is_none());
}

#[tokio::test]
async fn a_member_sets_their_date_of_birth() {
    let db = TestDb::new().await;
    enable(&db).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st, "user1").await;
    let dob = || async {
        sqlx::query_scalar::<_, Option<chrono::NaiveDate>>(
            "SELECT date_of_birth FROM users WHERE id = 3",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    // What the date of birth input sends.
    let (status, _, body) = user1
        .request(Method::PUT, "/u/user1.json", "date_of_birth=1904-9-5", true)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(dob().await, chrono::NaiveDate::from_ymd_opt(1904, 9, 5));
    let (_, doc) = user1.json("/u/user1.json").await;
    assert_eq!(doc["user"]["birthdate"], "1904-09-05");
    // Not a date: none, as ActiveModel casts it.
    user1
        .request(
            Method::PUT,
            "/u/user1.json",
            "date_of_birth=1904-2-30",
            true,
        )
        .await;
    assert_eq!(dob().await, None);
    user1
        .request(
            Method::PUT,
            "/u/user1.json",
            "date_of_birth=1904-12-25",
            true,
        )
        .await;
    user1
        .request(Method::PUT, "/u/user1.json", "date_of_birth=", true)
        .await;
    assert_eq!(dob().await, None);
}

#[tokio::test]
async fn the_onceoffs_move_the_custom_field_to_the_users_table() {
    let db = TestDb::new().await;
    // The fixture ran them already; the specs start from none.
    exec(
        &db,
        "DELETE FROM onceoff_logs WHERE job_name IN ('MigrateDateOfBirthToUsersTable', 'FixInvalidDateOfBirth')",
    )
    .await;
    exec(
        &db,
        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) VALUES \
         (2, 'date_of_birth', '1904-05-02', now(), now()), \
         (3, 'date_of_birth', '02-05-1904', now(), now()), \
         (4, 'date_of_birth', '02/05/1904', now(), now()), \
         (5, 'date_of_birth', '02/13/1904', now(), now()), \
         (1, 'date_of_birth', '', now(), now())",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(
        &mut conn,
        "migrate_date_of_birth_to_users_table",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    drop(conn);
    discourse_rs::jobs::drain(&st).await.unwrap();
    let dobs: Vec<(i32, Option<chrono::NaiveDate>)> =
        sqlx::query_as("SELECT id, date_of_birth FROM users WHERE id BETWEEN 1 AND 5 ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    let may_2 = chrono::NaiveDate::from_ymd_opt(1904, 5, 2);
    assert_eq!(
        dobs,
        [(1, None), (2, may_2), (3, may_2), (4, may_2), (5, None)]
    );
    // The blank one is gone; the others stay.
    let fields: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM user_custom_fields WHERE name = 'date_of_birth' ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(fields, [2, 3, 4, 5]);
    let logged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM onceoff_logs WHERE job_name = 'MigrateDateOfBirthToUsersTable'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(logged, 1);

    // FixInvalidDateOfBirth blanks what isn't a date.
    let mut conn = db.pool.acquire().await.unwrap();
    discourse_rs::jobs::enqueue(
        &mut conn,
        "fix_invalid_date_of_birth",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    drop(conn);
    discourse_rs::jobs::drain(&st).await.unwrap();
    let values: Vec<String> = sqlx::query_scalar(
        "SELECT value FROM user_custom_fields WHERE name = 'date_of_birth' ORDER BY user_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(values, ["1904-05-02", "02-05-1904", "02/05/1904", ""]);
}

/// The day user0 (who joined 2026-07-25) celebrates a year, with their
/// birthday on it too.
async fn celebrating(db: &TestDb) -> AppState {
    enable(db).await;
    exec(
        db,
        "UPDATE users SET date_of_birth = '2000-07-25' WHERE id = 2",
    )
    .await;
    exec(
        db,
        "UPDATE user_options SET timezone = NULL WHERE user_id = 2",
    )
    .await;
    state(db.pool.clone(), config(RailsEnv::Test, &[])).await
}

fn on_the_day() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2027, 7, 25, 12, 0, 0).unwrap()
}

#[tokio::test]
async fn a_poster_celebrating_shows_the_cake_and_the_balloon() {
    let db = TestDb::with_clock().await;
    let st = celebrating(&db).await;
    db.pin_clock(Some(on_the_day())).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (_, _, html) = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    let anniversary = "Today is the anniversary of the day I joined this community!";
    let names = html
        .split("<div class=\"names trigger-user-card\">")
        .nth(1)
        .and_then(|n| n.split("</div>").next())
        .expect("the first post's names");
    assert!(
        names.contains(&format!("<span class=\"poster-icon\" title=\"{anniversary}\"><img width=\"20\" height=\"20\" src='")),
        "{names}"
    );
    assert!(names.contains(&format!(
        "title='{anniversary}' alt='{anniversary}' class='emoji'>"
    )));
    assert!(names.contains("<span class=\"poster-icon\" title=\"Today is my birthday!\">"));
    let cake = names.find(anniversary).unwrap();
    let balloon = names.find("Today is my birthday!").unwrap();
    assert!(cake < balloon, "the anniversary's icon first");

    // The poster's own view says so.
    let mut user0 = Client::logged_in(st.clone(), "user0").await;
    let (_, _, html) = user0.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains("title=\"Today is your birthday!\""));
    assert!(
        html.contains("title=\"Today is the anniversary of the day you joined our community!\"")
    );

    // With emoji off, the cake icon.
    set_setting(&db.pool, "enable_emoji", 5, "f").await;
    let (_, _, html) = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(html.contains(&format!(
        "<span class=\"poster-icon\" title=\"{anniversary}\"><svg class=\"fa d-icon d-icon-cake-candles"
    )));

    // Not the day after.
    db.pin_clock(Some(Utc.with_ymd_and_hms(2027, 7, 26, 12, 0, 0).unwrap()))
        .await;
    let (_, _, html) = user1.page("/t/parity-fixture-replies-and-posters/35").await;
    assert!(!html.contains("poster-icon"));
}

#[tokio::test]
async fn the_profile_and_the_card_show_the_emoji() {
    let db = TestDb::with_clock().await;
    let st = celebrating(&db).await;
    db.pin_clock(Some(on_the_day())).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    // spec/system/cakeday_user_summary_spec.rb
    let (status, _, html) = user1.page("/u/user0/summary").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(
        "<span><div class=\"user-post-names-outlet user-cakeday\"><div class=\"emoji-images\"><div title=\"Today is my birthday!\"><img width=\"20\" height=\"20\" src='"
    ), "{html}");
    assert!(html.contains(
        "<div title=\"Today is the anniversary of the day I joined this community!\"><img width=\"20\" height=\"20\" src='"
    ));
    assert!(html.contains("alt='cake' class='emoji'>"));
    assert!(html.contains("alt='birthday' class='emoji'>"));
    // One's own profile says so; user1 joined the same day, without a
    // birthday.
    let (_, _, html) = user1.page("/u/user1/summary").await;
    assert!(html.contains(
        "<div class=\"user-post-names-outlet user-cakeday\"><div class=\"emoji-images\"><div title=\"Today is the anniversary of the day you joined our community!\">"
    ));
    assert!(!html.contains("title=\"Today is your birthday!\""));

    let (_, _, card) = user1.page("/user-card/user0").await;
    assert!(card.contains(
        "<div><div class=\"user-card-post-names-outlet user-card-cakeday\"><div class=\"emoji-images\"><div title=\"Today is my birthday!\">"
    ), "{card}");
}

#[tokio::test]
async fn the_lists_pages() {
    let db = TestDb::new().await;
    enable(&db).await;
    fabricate(&db, 101, "anni1", "Anni One", "2025-09-30 00:00", None).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    pin(september_30());

    // The index route replaces itself with today's.
    let (status, location, _) = user1.page("/cakeday/anniversaries").await;
    assert!(status.is_redirection());
    assert_eq!(location.as_deref(), Some("/cakeday/anniversaries/today"));

    let (status, _, html) = user1.page("/cakeday/anniversaries/today").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("<title>Anniversaries - "),
        "the route's title"
    );
    assert!(html.contains(
        "<div class=\"container cakeday\"><ul class=\"nav-pills\"><li class=\"nav-item-anniversaries\"><a href=\"/cakeday/anniversaries\" class=\"active\">Anniversaries</a></li><li class=\"nav-item-birthdays\"><a href=\"/cakeday/birthdays\">Birthdays</a></li></ul><div class=\"anniversaries\"><ul class=\"nav-pills\"><li><a href=\"/cakeday/anniversaries/today\" class=\"active\">Today</a></li>"
    ));
    assert!(html.contains("<h2 class=\"cakeday-header\">Anniversaries for September 30th</h2>"));
    assert!(html.contains(
        "<li class=\"user-info-item\"><div class=\"user-info small\" data-username=\"anni1\">"
    ));
    assert!(html.contains("<div class=\"details\"><div>September 30th, 2025</div></div>"));
    // The community section's More links, today's active.
    assert!(html.contains("data-list-item-name=\"anniversaries\""));
    assert!(html.contains("href=\"/cakeday/birthdays/today\""));

    let (_, _, html) = user1.page("/cakeday/anniversaries/tomorrow").await;
    assert!(html.contains("<h2 class=\"cakeday-header\">Anniversaries for October 1st</h2>"));
    assert!(html.contains(
        "<div class=\"user-info-empty-message\"><p>There are no users celebrating their anniversaries tomorrow.</p></div>"
    ));
    let (_, _, html) = user1.page("/cakeday/anniversaries/upcoming").await;
    assert!(html.contains("Anniversaries for October 2nd - October 9th"));
    let (_, _, html) = user1.page("/cakeday/anniversaries/all").await;
    assert!(html.contains("<h2 class=\"cakeday-header\">Anniversaries in the Month of</h2>"));
    assert!(html.contains("data-username=\"anni1\""));
    assert!(html.contains("<a href=\"/cakeday/anniversaries/all?month=10\">"));
    let (_, _, html) = user1.page("/cakeday/anniversaries/all?month=10").await;
    assert!(html.contains("There are no users celebrating their anniversaries this month."));

    // An unknown filter, and birthdays turned off, are Ember's unknown route.
    assert_eq!(
        user1.page("/cakeday/anniversaries/someday").await.0,
        StatusCode::NOT_FOUND
    );
    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "f").await;
    assert_eq!(
        user1.page("/cakeday/birthdays/today").await.0,
        StatusCode::NOT_FOUND
    );
    let (_, _, html) = user1.page("/cakeday/anniversaries/today").await;
    assert!(!html.contains("nav-item-birthdays"));
    assert!(!html.contains("href=\"/cakeday/birthdays/today\""));
}

#[tokio::test]
async fn the_lists_pages_load_more_as_they_scroll() {
    let db = TestDb::new().await;
    enable(&db).await;
    for i in 0..50 {
        fabricate(
            &db,
            200 + i,
            &format!("many{i:02}"),
            "",
            "2025-09-30 00:00",
            None,
        )
        .await;
    }
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    pin(september_30());
    let (_, _, html) = user1.page("/cakeday/anniversaries/today").await;
    assert_eq!(html.matches("class=\"user-info-item\"").count(), 48);
    assert!(html.contains(
        "id=\"cakeday-load-more\" hx-get=\"/cakeday/anniversaries/today?page=1\" hx-trigger=\"revealed\""
    ));
    let (_, _, html) = user1.page("/cakeday/anniversaries/today?page=1").await;
    assert_eq!(html.matches("class=\"user-info-item\"").count(), 2);
    assert!(html.contains(
        "<div aria-hidden=\"true\" class=\"load-more-sentinel\" id=\"cakeday-load-more\"></div>"
    ));
}

#[tokio::test]
async fn the_profile_preferences_offer_the_date_of_birth() {
    let db = TestDb::new().await;
    enable(&db).await;
    exec(
        &db,
        "UPDATE users SET date_of_birth = '1904-09-05' WHERE id = 3",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(st.clone(), "user1").await;
    let (status, _, html) = user1.page("/u/user1/preferences/profile").await;
    assert_eq!(status, StatusCode::OK);
    let input = html
        .split("<div class=\"user-custom-preferences-outlet user-date-of-birth-input\">")
        .nth(1)
        .expect("the date of birth input");
    assert!(input.starts_with(
        "<div class=\"control-group\"><label class=\"control-label\">Date of Birth</label>"
    ));
    assert!(input.contains("id=\"cakeday-date-of-birth-month\""));
    assert!(input.contains("data-name=\"September\" data-value=\"9\""));
    assert!(input.contains("data-name=\"5\" data-value=\"5\""));
    assert!(input.contains("<input type=\"hidden\" data-pref=\"date_of_birth\" value=\"\">"));
    assert!(html.contains("/assets/cakeday.js"));

    set_setting(&db.pool, "cakeday_birthday_enabled", 5, "f").await;
    let (_, _, html) = user1.page("/u/user1/preferences/profile").await;
    assert!(!html.contains("user-date-of-birth-input"));
}
