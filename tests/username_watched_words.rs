//! Watched words in usernames (UsernameValidator#username_not_watched_word?
//! over WATCHED_WORD_ACTIONS, and UserNameSuggester's candidates) and in a
//! profile update's bio and location (UserProfile's `watched_words: true`,
//! rendered by users#update's json_result with the profile's errors).
//!
//! The vendored Discourse predates the username check (Rails a3c292a9b97),
//! so no recording covers it, and none updates a profile with a blocked
//! word: the cases are read from app/models/username_validator.rb,
//! lib/user_name_suggester.rb, app/models/user_profile.rb and
//! app/services/user_updater.rb, against spec/models/username_validator_spec.rb
//! "watched words validation", spec/lib/user_name_suggester_spec.rb and
//! spec/models/user_profile_spec.rb "Validations".

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::admin::create::{AdminCreateError, NewAdmin, Outcome};
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const PASSWORD: &str = "a-long-enough-pass-phrase";
const BLOCK: i32 = 1;
const CENSOR: i32 = 2;
const REQUIRE_APPROVAL: i32 = 3;
const FLAG: i32 = 4;
const REPLACE: i32 = 5;
const TAG: i32 = 6;
const SILENCE: i32 = 7;
const LINK: i32 = 8;

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

struct Reply {
    status: StatusCode,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

impl Client {
    async fn new(state: &AppState) -> Client {
        let mut client = Client {
            state: state.clone(),
            cookies: Vec::new(),
            csrf: None,
        };
        let reply = client.send(Method::GET, "/session/csrf.json", &[]).await;
        client.csrf = reply.json()["csrf"].as_str().map(str::to_string);
        client
    }

    async fn logged_in(state: &AppState, username: &str) -> Client {
        let mut client = Client::new(state).await;
        client
            .send(
                Method::POST,
                "/session",
                &[("login", username), ("password", "password")],
            )
            .await;
        client
    }

    async fn send(&mut self, method: Method, path: &str, form: &[(&str, &str)]) -> Reply {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
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
        if !body.is_empty() {
            request = request.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap_or("");
            let (pair, attrs) = v.split_once(';').unwrap_or((v, ""));
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            self.cookies.retain(|(n, _)| n != name);
            if !attrs.contains("max-age=0") && !value.is_empty() {
                self.cookies.push((name.to_string(), value.to_string()));
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// POST /u.json as the signup form sends it.
    async fn sign_up(&mut self, username: &str, email: &str) -> Reply {
        let hp = self.send(Method::GET, "/session/hp.json", &[]).await.json();
        let value = hp["value"].as_str().unwrap().to_string();
        let challenge: String = hp["challenge"].as_str().unwrap().chars().rev().collect();
        let form = [
            ("name", "New Member"),
            ("email", email),
            ("password", PASSWORD),
            ("username", username),
            ("password_confirmation", value.as_str()),
            ("challenge", challenge.as_str()),
        ];
        self.send(Method::POST, "/u.json", &form).await
    }
}

async fn app() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn watch(pool: &PgPool, words: &[(&str, i32, Option<&str>)]) {
    for (word, action, replacement) in words {
        sqlx::query(
            "INSERT INTO watched_words (word, action, replacement, case_sensitive, created_at, updated_at) \
             VALUES ($1, $2, $3, false, now(), now())",
        )
        .bind(word)
        .bind(action)
        .bind(replacement)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// A setting as SiteSetting.set stores it, typed by its definition.
async fn set(st: &AppState, name: &str, value: &str) {
    let data_type = st
        .site_setting_defs
        .get(name)
        .unwrap_or_else(|| panic!("unknown setting {name}"))
        .data_type as i32;
    common::set_setting(&st.pool, name, data_type, value).await;
}

async fn exists(pool: &PgPool, username: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = $1)")
        .bind(username)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// users#create's answer for a username the validator refuses.
fn refused_signup(reply: &Reply, message: &str) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let body = reply.json();
    assert_eq!(body["success"], json!(false), "{}", reply.body);
    assert_eq!(
        body["errors"],
        json!({ "username": [message] }),
        "{}",
        reply.body
    );
    assert_eq!(body["message"], json!(format!("Username {message}")));
}

fn one(word: &str) -> String {
    format!("contains the blocked word '{word}'")
}

// username_validator_spec: "is invalid when username contains a
// blocked/censored/flagged/silenced/require approval watched word", "with
// wildcard", "is valid when username matches non-moderation watched word
// actions", the word boundaries, and "multiple watched words".
#[tokio::test]
async fn signup_refuses_a_username_with_a_moderation_watched_word() {
    let (db, st) = app().await;
    watch(
        &db.pool,
        &[
            ("*block*", BLOCK, None),
            ("*censor*", CENSOR, None),
            ("*approve*", REQUIRE_APPROVAL, None),
            ("*flag*", FLAG, None),
            ("*silence*", SILENCE, None),
            ("teh", REPLACE, Some("the")),
            ("github", LINK, Some("https://github.com")),
            ("tagme", TAG, Some("support")),
            ("test", BLOCK, None),
            ("bad", BLOCK, None),
            ("evil", BLOCK, None),
        ],
    )
    .await;
    let mut visitor = Client::new(&st).await;

    // The whole wildcard match is the word reported.
    for username in [
        "user_with_block",
        "censor_user",
        "approve_user",
        "flag_user",
        "silence_user",
    ] {
        let reply = visitor
            .sign_up(username, &format!("{username}@example.com"))
            .await;
        refused_signup(&reply, &one(username));
        assert!(!exists(&db.pool, username).await, "{username}");
    }

    // Unique and sorted across the actions, "other" past one.
    let reply = visitor.sign_up("evil-bad", "evilbad@example.com").await;
    refused_signup(&reply, "contains blocked words: bad, evil");

    // A whole word only.
    let reply = visitor.sign_up("test", "test@example.com").await;
    refused_signup(&reply, &one("test"));
    for username in ["testing", "mytest", "teh", "github", "tagme"] {
        let reply = visitor
            .sign_up(username, &format!("{username}@example.com"))
            .await;
        assert_eq!(reply.json()["success"], json!(true), "{}", reply.body);
        assert!(exists(&db.pool, username).await, "{username}");
    }

    // Last of the format checks: an earlier one answers alone.
    let reply = visitor.sign_up("test.png", "png@example.com").await;
    refused_signup(
        &reply,
        "must not end with a confusing suffix like .json or .png etc.",
    );
    // And before uniqueness: a taken name with a watched word.
    watch(&db.pool, &[("user1", BLOCK, None)]).await;
    let reply = visitor.sign_up("user1", "another@example.com").await;
    refused_signup(&reply, &one("user1"));
}

// UsernameCheckerService#check_username: the validator's errors as given.
// The suggestion for a taken name skips a numbered candidate holding a
// watched word (user_name_suggester_spec "checks watched words on
// numbered candidates").
#[tokio::test]
async fn check_username_answers_the_watched_word_and_suggests_past_one() {
    let (db, st) = app().await;
    let mut visitor = Client::new(&st).await;
    let reply = visitor.sign_up("samwise", "samwise@example.com").await;
    assert_eq!(reply.json()["success"], json!(true), "{}", reply.body);
    watch(
        &db.pool,
        &[("blocked", BLOCK, None), ("samwise1", BLOCK, None)],
    )
    .await;

    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .send(Method::GET, "/u/check_username.json?username=blocked", &[])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json(), json!({ "errors": [one("blocked")] }));

    let reply = visitor
        .send(Method::GET, "/u/check_username.json?username=samwise", &[])
        .await;
    assert_eq!(
        reply.json(),
        json!({ "available": false, "suggestion": "samwise2" }),
        "{}",
        reply.body
    );
}

// users#username: render_json_error(user.errors.full_messages.join(",")).
#[tokio::test]
async fn a_username_change_to_a_watched_word_is_refused() {
    let (db, st) = app().await;
    watch(
        &db.pool,
        &[("blocked", BLOCK, None), ("*fudge*", FLAG, None)],
    )
    .await;
    let mut admin = Client::logged_in(&st, "admin").await;
    for (new_username, message) in [
        (
            "blocked",
            "Username contains the blocked word 'blocked'".to_string(),
        ),
        (
            "fudge_it",
            "Username contains the blocked word 'fudge_it'".to_string(),
        ),
        // The suffix is checked first.
        (
            "blocked.png",
            "Username must not end with a confusing suffix like .json or .png etc.".to_string(),
        ),
    ] {
        let reply = admin
            .send(
                Method::PUT,
                "/u/user1/preferences/username.json",
                &[("new_username", new_username)],
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
        assert_eq!(reply.json(), json!({ "errors": [message] }));
    }
    assert!(exists(&db.pool, "user1").await);
    let reply = admin
        .send(
            Method::PUT,
            "/u/user1/preferences/username.json",
            &[("new_username", "unblocked")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert!(exists(&db.pool, "unblocked").await);
}

// Group names are exempt (Group's skip_watched_words: true), but
// groups#check_name goes through UsernameCheckerService, which isn't.
#[tokio::test]
async fn group_names_skip_the_watched_words_but_check_name_does_not() {
    let (db, st) = app().await;
    watch(&db.pool, &[("blocked", BLOCK, None)]).await;
    let mut admin = Client::logged_in(&st, "admin").await;
    let reply = admin
        .send(
            Method::GET,
            "/groups/check-name.json?group_name=blocked",
            &[],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json(), json!({ "errors": [one("blocked")] }));

    let reply = admin
        .send(
            Method::POST,
            "/admin/groups.json",
            &[("group[name]", "blocked")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let made: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM groups WHERE name = 'blocked')")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(made);
}

// InviteRedeemer: a username the validator refuses gives way to
// UserNameSuggester.suggest(email).
#[tokio::test]
async fn an_invitee_with_a_watched_username_gets_one_from_the_email() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO invites (id, invite_key, email, invited_by_id, created_at, updated_at, \
           emailed_status, max_redemptions_allowed, redemption_count, expires_at) VALUES \
         (900, 'Fixture900', NULL, 3, now(), now(), 0, 50, 0, now() + interval '30 days')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    common::set_setting(&db.pool, "enable_local_logins_via_code", 5, "f").await;
    // The email names the suggestion.
    common::set_setting(
        &db.pool,
        "use_email_for_username_and_name_suggestions",
        5,
        "t",
    )
    .await;
    watch(&db.pool, &[("blocked", BLOCK, None)]).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .send(
            Method::PUT,
            "/invites/show/Fixture900.json",
            &[
                ("email", "member@example.com"),
                ("username", "blocked"),
                ("name", "New Member"),
                ("password", PASSWORD),
                ("timezone", "UTC"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    assert!(!exists(&db.pool, "blocked").await);
    assert!(exists(&db.pool, "member").await);
}

// lib/tasks/admin.rake's save, and the suggestion for its email
// (user_name_suggester_spec "suggests a valid username when the input
// matches a watched word").
#[tokio::test]
async fn the_admin_task_refuses_a_watched_username_and_suggests_past_one() {
    let db = TestDb::new().await;
    // The email names the suggestion.
    common::set_setting(
        &db.pool,
        "use_email_for_username_and_name_suggestions",
        5,
        "t",
    )
    .await;
    watch(&db.pool, &[("blocked", BLOCK, None)]).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let password = "correct horse battery staple";
    match discourse_rs::admin::create::create(
        &st,
        &NewAdmin {
            email: "chief@example.org",
            password,
            name: None,
            username: Some("blocked"),
        },
    )
    .await
    .unwrap_err()
    {
        AdminCreateError::Invalid(messages) => {
            assert_eq!(messages, ["Username contains the blocked word 'blocked'"])
        }
        e => panic!("{e}"),
    }
    let outcome = discourse_rs::admin::create::create(
        &st,
        &NewAdmin {
            email: "blocked@example.org",
            password,
            name: None,
            username: None,
        },
    )
    .await
    .unwrap();
    let Outcome::Created { username } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(username, "blocked1");
}

/// The profile's columns that an update writes.
async fn profile(
    pool: &PgPool,
    username: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT p.bio_raw, p.location, p.website FROM user_profiles p \
         JOIN users u ON u.id = p.user_id WHERE u.username = $1",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn refused(reply: &Reply, errors: &[&str]) {
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert_eq!(reply.json(), json!({ "errors": errors }));
}

const BAD: &str = "Sorry, you can't post the word 'bad'; it's not allowed.";

// user_profile_spec "#bio_raw"/"#location" "when it contains watched
// words" of type Block and "when it is > 3000 characters"; UserUpdater
// saves the options before the profile (no rollback) and the user after.
#[tokio::test]
async fn a_blocked_word_in_the_bio_or_location_refuses_the_update() {
    let (db, st) = app().await;
    watch(&db.pool, &[("bad", BLOCK, None), ("worse", BLOCK, None)]).await;
    let before = profile(&db.pool, "user1").await;
    let name: Option<String> =
        sqlx::query_scalar("SELECT name FROM users WHERE username = 'user1'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let mut user1 = Client::logged_in(&st, "user1").await;

    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("bio_raw", "a bad bio"),
                ("name", "Renamed"),
                ("external_links_in_new_tab", "true"),
            ],
        )
        .await;
    refused(&reply, &[BAD]);
    assert_eq!(profile(&db.pool, "user1").await, before);
    let (now_name, new_tab): (Option<String>, bool) = sqlx::query_as(
        "SELECT u.name, o.external_links_in_new_tab FROM users u \
         JOIN user_options o ON o.user_id = u.id WHERE u.username = 'user1'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(now_name, name);
    assert!(new_tab);

    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("location", "Bad town, worse county")],
        )
        .await;
    refused(
        &reply,
        &["Sorry, you can't post that. Not allowed: Bad, worse."],
    );

    // Every validation's message, in the model's order.
    let long_bio = format!("bad {}", "a".repeat(3000));
    let long_location = format!("{} worse", "b".repeat(3000));
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("bio_raw", long_bio.as_str()),
                ("location", long_location.as_str()),
            ],
        )
        .await;
    refused(
        &reply,
        &[
            "About Me is too long (maximum is 3000 characters)",
            BAD,
            "Location is too long (maximum is 3000 characters)",
            "Sorry, you can't post the word 'worse'; it's not allowed.",
        ],
    );
    assert_eq!(profile(&db.pool, "user1").await, before);

    // The profile validates on every save: a stored bio that a word now
    // blocks refuses an update that doesn't touch it.
    sqlx::query(
        "UPDATE user_profiles SET bio_raw = 'my bad past' \
         WHERE user_id = (SELECT id FROM users WHERE username = 'user1')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let reply = user1
        .send(Method::PUT, "/u/user1.json", &[("name", "Renamed")])
        .await;
    refused(&reply, &[BAD]);

    // Not blocked: saved.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("bio_raw", "a good bio"), ("location", "Good town")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let (bio, location, _) = profile(&db.pool, "user1").await;
    assert_eq!(bio.as_deref(), Some("a good bio"));
    assert_eq!(location.as_deref(), Some("Good town"));
}

// user_profile_spec "with website validation": UrlValidator, the length
// and website_domain_validator, on a changed website only.
#[tokio::test]
async fn an_invalid_website_answers_the_profile_errors() {
    let (db, st) = app().await;
    set(
        &st,
        "allowed_user_website_domains",
        "discourse.org|meta.discourse.org",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user1 = Client::logged_in(&st, "user1").await;
    let domains = "Website is invalid. Allowed domains are: discourse.org, meta.discourse.org";

    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("website", "https://google.com")],
        )
        .await;
    refused(&reply, &[domains]);

    // "doesn't blow up with an invalid URI"
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("website", "user - https://forum.example.com/user")],
        )
        .await;
    refused(&reply, &["Website is invalid", domains]);

    let long = format!("discourse.org/{}", "a".repeat(3000));
    let reply = user1
        .send(Method::PUT, "/u/user1.json", &[("website", long.as_str())])
        .await;
    refused(
        &reply,
        &["Website is too long (maximum is 3000 characters)"],
    );
    assert_eq!(profile(&db.pool, "user1").await.2, None);

    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("website", "discourse.org")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        profile(&db.pool, "user1").await.2.as_deref(),
        Some("http://discourse.org")
    );
}
