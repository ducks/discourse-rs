//! A fresh install: `discourse-rs setup` (the schema and db:seed into an
//! empty database), `discourse-rs admin create` (rake admin:create), the
//! keys db:seed makes on every deploy and GlobalSetting's kept
//! secret_key_base, and FinishInstallationController's pages while the
//! site has no admin.

mod common;

use std::collections::BTreeMap;
use std::str::FromStr;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state, test_database_url};
use discourse_rs::AppState;
use discourse_rs::admin_create::{AdminCreateError, NewAdmin, Outcome};
use discourse_rs::config::RailsEnv;
use discourse_rs::setup::SetupError;
use discourse_rs::site_settings::Definitions;
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use tower::ServiceExt;

const BOOL: i32 = 5;
const STRING: i32 = 1;
const DEVELOPERS: &str = "dev1@example.com, dev2@example.com";

/// An empty database of its own on the test server, dropped afterwards.
struct EmptyDb {
    name: String,
    admin: PgConnectOptions,
}

impl EmptyDb {
    async fn new(tag: &str) -> EmptyDb {
        let template = PgConnectOptions::from_str(&test_database_url()).unwrap();
        let name = format!(
            "{}_setup_{tag}_{}",
            template.get_database().unwrap(),
            std::process::id()
        );
        let admin = template.database("postgres");
        let mut conn = PgConnection::connect_with(&admin).await.unwrap();
        conn.execute(format!(r#"DROP DATABASE IF EXISTS "{name}""#).as_str())
            .await
            .unwrap();
        conn.execute(
            format!(r#"CREATE DATABASE "{name}" TEMPLATE template0 LOCALE 'en_US.UTF-8'"#).as_str(),
        )
        .await
        .unwrap();
        EmptyDb { name, admin }
    }

    fn url(&self) -> String {
        let url = test_database_url();
        let template = PgConnectOptions::from_str(&url)
            .unwrap()
            .get_database()
            .unwrap()
            .to_string();
        url.replacen(&format!("/{template}"), &format!("/{}", self.name), 1)
    }

    async fn drop_it(self) {
        let mut conn = PgConnection::connect_with(&self.admin).await.unwrap();
        conn.execute(format!(r#"DROP DATABASE IF EXISTS "{}" WITH (FORCE)"#, self.name).as_str())
            .await
            .unwrap();
    }
}

fn production(globals: &[(&str, &str)], database_url: String) -> discourse_rs::config::Config {
    let mut c = config(RailsEnv::Production, globals);
    c.site.database_url = database_url;
    c
}

#[tokio::test]
async fn setup_loads_an_empty_database_once() {
    let db = EmptyDb::new("once").await;
    let config = production(
        &[
            ("hostname", "forum.example.com"),
            ("force_https", "true"),
            ("developer_emails", "dev@example.com"),
        ],
        db.url(),
    );
    let defs = Definitions::vendored().unwrap();
    let mut conn = PgConnection::connect(&db.url()).await.unwrap();
    let report = discourse_rs::setup::setup(&mut conn, &defs, &config, None)
        .await
        .unwrap();
    assert!(report.tables > 380, "{report:?}");
    assert_eq!(
        report.versions,
        discourse_rs::schema::expected_versions().len()
    );

    let scalar = async |conn: &mut PgConnection, sql: &str| -> String {
        sqlx::query_scalar::<_, String>(sql)
            .fetch_one(&mut *conn)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    };
    // db:seed's users only, each with a random password and a signup token.
    assert_eq!(
        scalar(
            &mut conn,
            "SELECT string_agg(username, ',' ORDER BY id) FROM users"
        )
        .await,
        "discobot,system"
    );
    assert_eq!(
        scalar(
            &mut conn,
            "SELECT string_agg(user_id::text, ',' ORDER BY user_id) FROM user_passwords"
        )
        .await,
        "-2,-1"
    );
    assert_eq!(
        scalar(
            &mut conn,
            "SELECT count(*)::text FROM email_tokens WHERE scope = 1 AND NOT confirmed"
        )
        .await,
        "2"
    );
    // The schema verifies and the port's own schema is there.
    discourse_rs::schema::verify(&mut conn).await.unwrap();
    assert_eq!(
        scalar(&mut conn, "SELECT count(*)::text FROM discourse_rs.jobs").await,
        "0"
    );
    // The seeded categories and topics.
    assert_eq!(
        scalar(
            &mut conn,
            "SELECT string_agg(name, ',' ORDER BY id) FROM categories"
        )
        .await,
        "Uncategorized,Site Feedback,Staff,General"
    );
    assert_eq!(
        scalar(&mut conn, "SELECT count(*)::text FROM topics").await,
        "6"
    );
    // The site's own urls in place of the placeholder's.
    assert_eq!(
        scalar(&mut conn, "SELECT count(*)::text FROM posts WHERE raw LIKE '%discourse-rs.invalid%' OR cooked LIKE '%discourse-rs.invalid%'").await,
        "0"
    );
    assert!(
        scalar(&mut conn, "SELECT raw FROM posts WHERE topic_id = (SELECT value::int FROM site_settings WHERE name = 'admin_quick_start_topic_id')")
            .await
            .contains("https://forum.example.com/admin/email/server-settings")
    );
    // The settings db:seed derives from the site or makes at random.
    let setting = async |conn: &mut PgConnection, name: &str| -> String {
        sqlx::query_scalar::<_, Option<String>>("SELECT value FROM site_settings WHERE name = $1")
            .bind(name)
            .fetch_optional(&mut *conn)
            .await
            .unwrap()
            .flatten()
            .unwrap_or_default()
    };
    assert_eq!(
        setting(&mut conn, "notification_email").await,
        "noreply@forum.example.com"
    );
    assert_eq!(
        setting(&mut conn, "vapid_base_url").await,
        "https://forum.example.com"
    );
    assert_eq!(setting(&mut conn, "vapid_public_key").await.len(), 88);
    assert_eq!(setting(&mut conn, "push_api_secret_key").await.len(), 32);
    assert!(
        !setting(&mut conn, "api_key_last_used_epoch")
            .await
            .is_empty()
    );
    assert_eq!(setting(&mut conn, "has_login_hint").await, "t");
    assert_eq!(
        setting(&mut conn, "global_notice").await,
        "Congratulations, you installed Discourse! Register a new admin account with dev@example.com to finalize configuration."
    );

    // Again: refused, nothing changed.
    let vapid = setting(&mut conn, "vapid_public_key").await;
    match discourse_rs::setup::setup(&mut conn, &defs, &config, None).await {
        Err(SetupError::NotEmpty { tables }) => assert_eq!(tables, report.tables),
        other => panic!("{other:?}"),
    }
    assert_eq!(setting(&mut conn, "vapid_public_key").await, vapid);
    conn.close().await.unwrap();
    db.drop_it().await;
}

#[tokio::test]
async fn setup_refuses_a_database_with_tables() {
    let db = TestDb::new().await;
    let defs = Definitions::vendored().unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    let err = discourse_rs::setup::setup(&mut conn, &defs, &config(RailsEnv::Test, &[]), None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, SetupError::NotEmpty { tables } if tables > 380),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("only loads an empty database"),
        "{err}"
    );
}

#[tokio::test]
async fn setup_refuses_an_owner_that_is_not_a_role() {
    let db = EmptyDb::new("owner").await;
    let defs = Definitions::vendored().unwrap();
    let mut conn = PgConnection::connect(&db.url()).await.unwrap();
    let err = discourse_rs::setup::setup(
        &mut conn,
        &defs,
        &production(&[], db.url()),
        Some("no_such_role_for_setup"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SetupError::Owner(_)), "{err:?}");
    let tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE schemaname = 'public'")
            .fetch_one(&mut conn)
            .await
            .unwrap();
    assert_eq!(tables, 0);
    conn.close().await.unwrap();
    db.drop_it().await;
}

async fn login_hint_on(pool: &PgPool) {
    set_setting(pool, "has_login_hint", BOOL, "t").await;
    set_setting(
        pool,
        "global_notice",
        STRING,
        "Congratulations, you installed Discourse!",
    )
    .await;
}

async fn setting(pool: &PgPool, name: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT value FROM site_settings WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .unwrap()
        .flatten()
}

#[tokio::test]
async fn admin_create_makes_an_active_admin() {
    let db = TestDb::new().await;
    login_hint_on(&db.pool).await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let password = "correct horse battery staple";
    let outcome = discourse_rs::admin_create::create(
        &st,
        &NewAdmin {
            email: " New.Admin@Example.org ",
            password,
            name: None,
        },
    )
    .await
    .unwrap();
    let Outcome::Created { username } = outcome else {
        panic!("{outcome:?}");
    };
    let (id, admin, active, approved, trust_level): (i32, bool, bool, bool, i32) = sqlx::query_as(
        "SELECT id, admin, active, approved, trust_level FROM users WHERE username = $1",
    )
    .bind(&username)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(admin && active && approved, "{username}");
    assert_eq!(trust_level, 1);
    let email: String =
        sqlx::query_scalar("SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\"")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(email, "new.admin@example.org");
    let unconfirmed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM email_tokens WHERE user_id = $1 AND NOT confirmed",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(unconfirmed, 0);
    let in_admins: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = 1 AND user_id = $1)",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(in_admins);
    let (hash, salt, algorithm): (String, String, String) = sqlx::query_as(
        "SELECT password_hash, password_salt, password_algorithm FROM user_passwords WHERE user_id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        discourse_rs::session::token::hash_password(password, &salt, &algorithm).unwrap(),
        hash
    );
    // grant_admin!'s save cleared the login hint.
    assert_eq!(
        setting(&db.pool, "has_login_hint").await.as_deref(),
        Some("f")
    );
    assert_eq!(
        setting(&db.pool, "global_notice").await.as_deref(),
        Some("")
    );
}

#[tokio::test]
async fn admin_create_refuses_a_short_password() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let err = discourse_rs::admin_create::create(
        &st,
        &NewAdmin {
            email: "short@example.org",
            // min_password_length (10), under min_admin_password_length.
            password: "tenletters1",
            name: None,
        },
    )
    .await
    .unwrap_err();
    match err {
        AdminCreateError::Invalid(messages) => assert_eq!(
            messages,
            ["Password is too short (minimum is 15 characters)"]
        ),
        other => panic!("{other:?}"),
    }
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_emails WHERE email = 'short@example.org')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn admin_create_promotes_an_existing_account() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let outcome = discourse_rs::admin_create::create(
        &st,
        &NewAdmin {
            email: "USER0@example.com",
            password: "another long enough password",
            name: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Updated {
            username: "user0".into()
        }
    );
    let (admin, active): (bool, bool) =
        sqlx::query_as("SELECT admin, active FROM users WHERE username = 'user0'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(admin && active);
    let tokens: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM user_auth_tokens WHERE user_id = (SELECT id FROM users WHERE username = 'user0')",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(tokens, 0, "the password change logged the account out");
}

#[tokio::test]
async fn push_keys_are_made_when_missing_or_the_base_url_moves() {
    let db = TestDb::new().await;
    let defs = Definitions::vendored().unwrap();
    let config = production(&[("hostname", "forum.example.com")], test_database_url());
    let mut conn = db.pool.acquire().await.unwrap();
    sqlx::query(
        "DELETE FROM site_settings WHERE name IN ('vapid_public_key', 'vapid_private_key', \
         'vapid_public_key_bytes', 'vapid_base_url', 'push_api_secret_key')",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    let set = discourse_rs::setup::ensure_push_keys(&mut conn, &defs, &config)
        .await
        .unwrap();
    assert_eq!(set.len(), 5, "{set:?}");
    assert_eq!(
        setting(&db.pool, "vapid_base_url").await.as_deref(),
        Some("http://forum.example.com")
    );
    let key = setting(&db.pool, "vapid_public_key").await.unwrap();
    let secret = setting(&db.pool, "push_api_secret_key").await.unwrap();
    // Nothing missing: kept.
    let set = discourse_rs::setup::ensure_push_keys(&mut conn, &defs, &config)
        .await
        .unwrap();
    assert!(set.is_empty(), "{set:?}");
    // Another host: a new pair, the subscriptions bound to the old one gone.
    let moved = production(&[("hostname", "new.example.com")], test_database_url());
    let set = discourse_rs::setup::ensure_push_keys(&mut conn, &defs, &moved)
        .await
        .unwrap();
    assert_eq!(set.len(), 4, "{set:?}");
    assert_ne!(setting(&db.pool, "vapid_public_key").await.unwrap(), key);
    assert_eq!(
        setting(&db.pool, "push_api_secret_key").await.unwrap(),
        secret
    );
}

#[tokio::test]
async fn a_generated_secret_key_base_is_kept() {
    use discourse_rs::session::current::{stored_secret_key_base, valid_secret_key};
    let db = TestDb::new().await;
    let mut conn = db.pool.acquire().await.unwrap();
    let first = stored_secret_key_base(&mut conn).await.unwrap();
    assert!(valid_secret_key(&first), "{first}");
    assert_eq!(stored_secret_key_base(&mut conn).await.unwrap(), first);
    sqlx::query(
        "UPDATE discourse_rs.persistent_values SET value = 'not hex' WHERE key = 'SECRET_TOKEN'",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    let replaced = stored_secret_key_base(&mut conn).await.unwrap();
    assert!(valid_secret_key(&replaced) && replaced != first);
    assert!(!valid_secret_key(&"A".repeat(128)));
}

/// A browser: its cookies kept between requests.
struct Client {
    st: AppState,
    cookies: BTreeMap<String, String>,
}

struct Reply {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The page's authenticity_token field.
    fn token(&self) -> String {
        let marker = "name=\"authenticity_token\" value=\"";
        let at = self.body.find(marker).expect("a form") + marker.len();
        let end = self.body[at..].find('"').unwrap();
        self.body[at..at + end].to_string()
    }
}

impl Client {
    fn new(st: &AppState) -> Client {
        Client {
            st: st.clone(),
            cookies: BTreeMap::new(),
        }
    }

    async fn send(
        &mut self,
        method: Method,
        path: &str,
        form: Option<&[(&str, &str)]>,
        xhr: bool,
    ) -> Reply {
        let app = discourse_rs::app(self.st.clone());
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "test.localhost");
        if xhr {
            builder = builder
                .header("x-requested-with", "XMLHttpRequest")
                .header(header::ACCEPT, "application/json");
        } else {
            builder = builder.header(header::ACCEPT, "text/html");
        }
        if !self.cookies.is_empty() {
            let jar: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            builder = builder.header(header::COOKIE, jar.join("; "));
        }
        let body = match form {
            Some(pairs) => {
                builder = builder.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
                form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(pairs)
                    .finish()
            }
            None => String::new(),
        };
        let response = app
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        for (k, v) in &headers {
            if k == "set-cookie"
                && let Some((name, rest)) = v.split_once('=')
            {
                let value = rest.split(';').next().unwrap_or("");
                self.cookies.insert(name.to_string(), value.to_string());
            }
        }
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, None, false).await
    }

    async fn post(&mut self, path: &str, form: &[(&str, &str)]) -> Reply {
        self.send(Method::POST, path, Some(form), false).await
    }
}

#[tokio::test]
async fn the_pages_are_forbidden_once_there_is_an_admin() {
    let db = TestDb::new().await;
    let st = state(
        db.pool.clone(),
        config(RailsEnv::Test, &[("developer_emails", DEVELOPERS)]),
    )
    .await;
    let mut client = Client::new(&st);
    for path in ["/finish-installation", "/finish-installation/register"] {
        let reply = client.get(path).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
        assert!(!reply.body.contains("wizard-container"), "{path}");
    }
    let reply = client
        .send(Method::GET, "/finish-installation", None, true)
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let json: Value = serde_json::from_str(&reply.body).unwrap();
    assert_eq!(json["error_type"], "invalid_access");
    // The front page is the list again.
    let reply = client.get("/").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.body.contains("finish-installation"));
}

#[tokio::test]
async fn index_is_the_front_page_while_the_login_hint_is_on() {
    let db = TestDb::new().await;
    login_hint_on(&db.pool).await;
    // login_required doesn't send the visitor to /login: the controller
    // skips the gate.
    set_setting(&db.pool, "login_required", BOOL, "t").await;
    let st = state(
        db.pool.clone(),
        config(RailsEnv::Test, &[("developer_emails", DEVELOPERS)]),
    )
    .await;
    let mut client = Client::new(&st);
    for path in ["/", "/finish-installation"] {
        let reply = client.get(path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.body);
        let body = &reply.body;
        assert!(body.contains("<title>Discourse Setup</title>"), "{body}");
        assert!(body.contains("<body class='wizard'>"));
        assert!(body.contains("<div class='wizard-container-contents finish-installation'>"));
        assert!(body.contains("/assets/wizard.css?v="), "{body}");
        assert!(
            body.contains(
                "<img src=\"/images/wizard/tada.svg\" alt=\"tada emoji\" class=\"tada\">"
            )
        );
        assert!(body.contains("<h1>Congratulations, you installed Discourse!</h1>"));
        assert!(body.contains("<p>Register a new account to get started.</p>"));
        assert!(body.contains(
            "<a class=\"wizard-container__button btn-primary finish-installation__register\" href=\"/finish-installation/register\">"
        ));
        assert!(body.contains("<meta name=\"generator\" content=\"Discourse "));
    }
    // The wizard stylesheet carries the page's rules.
    let reply = client.get("/assets/wizard.css").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(
        reply
            .body
            .contains(".wizard-container-contents.finish-installation")
    );
}

#[tokio::test]
async fn register_lists_the_developer_emails_only() {
    let db = TestDb::new().await;
    login_hint_on(&db.pool).await;
    let st = state(
        db.pool.clone(),
        config(RailsEnv::Test, &[("developer_emails", DEVELOPERS)]),
    )
    .await;
    let mut client = Client::new(&st);
    let reply = client.get("/finish-installation/register").await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = &reply.body;
    assert!(body.contains("<h1>Register Admin Account</h1>"), "{body}");
    assert!(body.contains(
        "<select name=\"email\" id=\"email\" class=\"combobox wizard-container__combobox\"><option value=\"dev1@example.com\">dev1@example.com</option>\n<option value=\"dev2@example.com\">dev2@example.com</option></select>"
    ), "{body}");
    assert!(
        body.contains("<div class='wizard-container__description'>Unique, no spaces, short</div>")
    );
    assert!(
        body.contains("<div class='wizard-container__description'>At least 15 characters</div>")
    );
    assert!(body.contains(
        "<input type=\"submit\" name=\"commit\" value=\"Register\" class=\"wizard-container__button btn-primary\" data-disable-with=\"Register\" />"
    ));

    // Another address is refused.
    let token = reply.token();
    let reply = client
        .post(
            "/finish-installation/register",
            &[
                ("authenticity_token", &token),
                ("email", "someone@example.com"),
                ("username", "someone"),
                ("password", "correct horse battery staple"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    // Without the form's token: Rails' BAD CSRF.
    let reply = client
        .post(
            "/finish-installation/register",
            &[("email", "dev1@example.com")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.body, "[\"BAD CSRF\"]");

    // No developer emails: the page says so.
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let reply = Client::new(&st).get("/finish-installation/register").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.body.contains("<form"));
    assert!(
        reply
            .body
            .contains("Unfortunately, no administrator emails were defined during setup")
    );
}

#[tokio::test]
async fn register_shows_its_errors_then_registers_the_first_admin() {
    let db = TestDb::new().await;
    login_hint_on(&db.pool).await;
    let st = state(
        db.pool.clone(),
        config(RailsEnv::Test, &[("developer_emails", DEVELOPERS)]),
    )
    .await;
    let mut client = Client::new(&st);
    let token = client.get("/finish-installation/register").await.token();

    let reply = client
        .post(
            "/finish-installation/register",
            &[
                ("authenticity_token", &token),
                ("email", "dev2@example.com"),
                ("username", "x"),
                ("password", "short"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body = &reply.body;
    assert!(
        body.contains(
            "<option selected=\"selected\" value=\"dev2@example.com\">dev2@example.com</option>"
        ),
        "{body}"
    );
    assert!(body.contains("<input type=\"text\" name=\"username\" id=\"username\" value=\"x\" class=\"wizard-container__text-input\" />"));
    assert!(
        body.contains("<div class='field-error-description'>must be at least 3 characters</div>")
    );
    assert!(body.contains(
        "<div class='field-error-description'>is too short (minimum is 15 characters)</div>"
    ));
    assert_eq!(body.matches("text-field invalid'>").count(), 2, "{body}");

    let password = "correct horse battery staple";
    let reply = client
        .post(
            "/finish-installation/register",
            &[
                ("authenticity_token", &token),
                ("email", "dev2@example.com"),
                ("username", "firstadmin"),
                ("password", password),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::FOUND, "{}", reply.body);
    assert_eq!(
        reply.header("location"),
        Some("http://test.localhost/finish-installation/confirm-email")
    );
    let (id, active, admin, trust_level): (i32, bool, bool, i32) = sqlx::query_as(
        "SELECT id, active, admin, trust_level FROM users WHERE username = 'firstadmin'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(!active && !admin);
    assert_eq!(trust_level, 1);
    let signup_emails = async || -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM discourse_rs.jobs WHERE name = 'critical_user_email' \
             AND args->>'type' = 'signup' AND (args->>'user_id')::int = $1",
        )
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
    };
    assert_eq!(signup_emails().await, 1);

    let reply = client.get("/finish-installation/confirm-email").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains("<h1>Confirm your Email</h1>"));
    assert!(
        reply
            .body
            .contains("<p>We sent an activation mail to <b>dev2@example.com</b>.")
    );
    assert!(
        reply
            .body
            .contains("<input type=\"hidden\" name=\"_method\" value=\"put\" />")
    );
    let token = reply.token();
    let reply = client
        .post(
            "/finish-installation/resend-email",
            &[("authenticity_token", &token), ("_method", "put")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.body.contains("<h1>Resend Activation Email</h1>"));
    assert!(
        reply
            .body
            .contains("<p>We've re-sent the activation email to <b>dev2@example.com</b>")
    );
    assert_eq!(signup_emails().await, 2);

    // Activated by the email's link, the account logs in and becomes the
    // admin (make_developer_admin), which turns the login hint off.
    sqlx::query("UPDATE users SET active = TRUE WHERE id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE email_tokens SET confirmed = TRUE WHERE user_id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    let csrf = client
        .send(Method::GET, "/session/csrf.json", None, true)
        .await;
    let csrf: Value = serde_json::from_str(&csrf.body).unwrap();
    let csrf = csrf["csrf"].as_str().unwrap().to_string();
    let app = discourse_rs::app(st.clone());
    let jar: Vec<String> = client
        .cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs([("login", "firstadmin"), ("password", password)])
        .finish();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/session")
                .header(header::HOST, "test.localhost")
                .header("x-requested-with", "XMLHttpRequest")
                .header(header::ACCEPT, "application/json")
                .header("x-csrf-token", csrf)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, jar.join("; "))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json.get("error").is_none(), "{json}");
    let (admin, moderator): (bool, bool) =
        sqlx::query_as("SELECT admin, moderator FROM users WHERE id = $1")
            .bind(id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(admin, "make_developer_admin");
    // The fixture site has an admin already, so this one is not singular.
    assert!(!moderator);
    assert_eq!(
        setting(&db.pool, "has_login_hint").await.as_deref(),
        Some("f")
    );
    assert_eq!(
        setting(&db.pool, "global_notice").await.as_deref(),
        Some("")
    );
    let reply = client.get("/finish-installation").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}
