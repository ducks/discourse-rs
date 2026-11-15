//! A multiselect user field on the profile preferences
//! (templates/preferences/profile.gjs over components/user-fields/multiselect.gjs,
//! saved by users#update) and the user fields of the invite page
//! (templates/invites/show.gjs, redeemed through
//! InviteRedeemer.create_user_from_invite). The markup is read from the
//! templates. users#update's multiselect (a list cleaned against the
//! options, an emptied one) is the recorded
//! parity/writes/user_fields/profile_update; no recording redeems an
//! invite with user fields, so those branches are read from
//! app/models/invite_redeemer.rb and app/models/user.rb.

use crate::common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestDb, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const BOOL: i32 = 5;
const PASSWORD: &str = "a-long-enough-pass-phrase";
const TEXT: i32 = 0;
const CONFIRM: i32 = 1;
const DROPDOWN: i32 = 2;
const MULTISELECT: i32 = 3;
const TEXTAREA: i32 = 4;
const OPTIONAL: i32 = 0;
const FOR_ALL_USERS: i32 = 1;
const ON_SIGNUP: i32 = 2;

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
    /// A visitor with the session's CSRF token.
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
            request = request.header(header::COOKIE, self.cookie_header());
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

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The page as a browser asks for it.
    async fn page(&mut self, path: &str) -> Reply {
        let mut request = Request::get(path).header(header::HOST, "test.localhost");
        if !self.cookies.is_empty() {
            request = request.header(header::COOKIE, self.cookie_header());
        }
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }

    /// PUT /invites/show/:key.json as the invite form sends it, with
    /// `fields` the `user_custom_fields[...]` pairs.
    async fn redeem(&mut self, key: &str, username: &str, fields: &[(&str, &str)]) -> Reply {
        let email = format!("{username}@example.com");
        let mut form = vec![
            ("email", email.as_str()),
            ("username", username),
            ("name", "New Member"),
            ("password", PASSWORD),
            ("timezone", "UTC"),
        ];
        form.extend_from_slice(fields);
        self.send(Method::PUT, &format!("/invites/show/{key}.json"), &form)
            .await
    }
}

async fn app() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

struct Field<'a> {
    id: i32,
    name: &'a str,
    description: &'a str,
    kind: i32,
    editable: bool,
    requirement: i32,
    position: i32,
    show_on_signup: bool,
    show_on_profile: bool,
    options: &'a [&'a str],
}

impl Default for Field<'_> {
    fn default() -> Self {
        Field {
            id: 0,
            name: "",
            description: "About it",
            kind: TEXT,
            editable: true,
            requirement: OPTIONAL,
            position: 0,
            show_on_signup: true,
            show_on_profile: false,
            options: &[],
        }
    }
}

async fn field(pool: &PgPool, f: Field<'_>) {
    sqlx::query(
        "INSERT INTO user_fields (id, name, description, field_type_enum, editable, requirement, \
           position, show_on_signup, show_on_profile, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), now())",
    )
    .bind(f.id)
    .bind(f.name)
    .bind(f.description)
    .bind(f.kind)
    .bind(f.editable)
    .bind(f.requirement)
    .bind(f.position)
    .bind(f.show_on_signup)
    .bind(f.show_on_profile)
    .execute(pool)
    .await
    .unwrap();
    for option in f.options {
        sqlx::query(
            "INSERT INTO user_field_options (user_field_id, value, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(f.id)
        .bind(option)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// The user's `user_field_*` custom fields: (name, value) in id order.
async fn stored(pool: &PgPool, username: &str) -> Vec<(String, Option<String>)> {
    sqlx::query_as(
        "SELECT f.name, f.value FROM user_custom_fields f JOIN users u ON u.id = f.user_id \
         WHERE u.username = $1 AND f.name LIKE 'user\\_field\\_%' ORDER BY f.id",
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn rows(pairs: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), Some(v.to_string())))
        .collect()
}

async fn exists(pool: &PgPool, username: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = $1)")
        .bind(username)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_field(pool: &PgPool, username: &str, id: i32, values: &[&str]) {
    for value in values {
        sqlx::query(
            "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
             SELECT id, $2, $3, now(), now() FROM users WHERE username = $1",
        )
        .bind(username)
        .bind(format!("user_field_{id}"))
        .bind(value)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// The component of the field `id`, up to the next one's.
fn component(html: &str, id: i32) -> &str {
    let at = html
        .find(&format!("data-user-field=\"{id}\""))
        .unwrap_or_else(|| panic!("field {id} is drawn: {html}"));
    let start = html[..at].rfind("<div class=\"user-field ").unwrap();
    // Up to the next one, or what follows the list on either page.
    let end = [
        "<div class=\"user-field ",
        "<div class=\"clearfix\">",
        "<div class=\"invitation-cta\">",
    ]
    .iter()
    .filter_map(|next| html[at..].find(next))
    .min()
    .map_or(html.len(), |e| at + e);
    &html[start..end]
}

/// Langs (3), a multiselect, and Staff tags (4), one only staff edit.
async fn multiselects(pool: &PgPool) {
    field(
        pool,
        Field {
            id: 3,
            name: "Langs",
            description: "The <b>languages</b>",
            kind: MULTISELECT,
            position: 1,
            options: &["rust", "ruby", "c & c++"],
            ..Field::default()
        },
    )
    .await;
    field(
        pool,
        Field {
            id: 4,
            name: "Staff tags",
            kind: MULTISELECT,
            editable: false,
            position: 2,
            options: &["vip", "press"],
            ..Field::default()
        },
    )
    .await;
}

#[tokio::test]
async fn the_profile_preferences_draw_a_multiselect_with_its_choices() {
    let (db, st) = app().await;
    multiselects(&db.pool).await;
    set_field(&db.pool, "user1", 4, &["vip"]).await;

    // Nothing chosen: MultiSelect's `none` in the header.
    let mut user1 = Client::logged_in(&st, "user1").await;
    let page = user1.page("/u/user1/preferences/profile").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let html = &page.body;
    let langs = component(html, 3);
    assert!(
        langs.starts_with(
            "<div class=\"user-field multiselect user-field-langs\"><label class=\"control-label alt-placeholder\" for=\"user-field-3-input\">Langs(optional)</label><div class=\"controls\"><details id=\"user-field-3-input\" data-user-field=\"3\" data-none=\"(select an option)\" data-options=\"[&quot;rust&quot;,&quot;ruby&quot;,&quot;c &amp; c++&quot;]\" class=\"select-kit multi-select\" data-chooser=\"options\" data-items=\"[]\">"
        ),
        "{langs}"
    );
    assert!(
        langs.contains("<span class=\"formatted-selection\">(select an option)</span>"),
        "{langs}"
    );
    assert!(
        langs.contains("<div class=\"instructions\">The <b>languages</b></div>"),
        "{langs}"
    );
    assert!(!langs.contains("data-pref"), "{langs}");
    // A member is not shown a field only staff edit.
    assert!(!html.contains("data-user-field=\"4\""), "{html}");
    // The chooser's script, with its strings.
    assert!(html.contains("id=\"prefs-choosers-strings\""), "{html}");
    assert!(html.contains("/assets/prefs-choosers.js?v="), "{html}");

    // One stored row is a string, several a list: both are choices.
    set_field(&db.pool, "user1", 3, &["ruby"]).await;
    let page = user1.page("/u/user1/preferences/profile").await;
    let langs = component(&page.body, 3);
    assert!(
        langs.contains("class=\"select-kit multi-select has-selection\" data-chooser=\"options\" data-items=\"[[&quot;ruby&quot;,&quot;ruby&quot;]]\""),
        "{langs}"
    );
    assert!(
        langs.contains("<span class=\"formatted-selection\">ruby</span>"),
        "{langs}"
    );
    set_field(&db.pool, "user1", 3, &["c & c++"]).await;
    let page = user1.page("/u/user1/preferences/profile").await;
    let langs = component(&page.body, 3);
    assert!(
        langs.contains("data-items=\"[[&quot;ruby&quot;,&quot;ruby&quot;],[&quot;c &amp; c++&quot;,&quot;c &amp; c++&quot;]]\""),
        "{langs}"
    );
    assert!(
        langs.contains("<span class=\"formatted-selection\">ruby, c &amp; c++</span>"),
        "{langs}"
    );

    // Staff are shown the field members cannot edit, with its value.
    let mut admin = Client::logged_in(&st, "admin").await;
    let page = admin.page("/u/user1/preferences/profile").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let tags = component(&page.body, 4);
    assert!(
        tags.contains("data-items=\"[[&quot;vip&quot;,&quot;vip&quot;]]\""),
        "{tags}"
    );
}

#[tokio::test]
async fn a_profile_without_a_multiselect_loads_no_chooser() {
    let (db, st) = app().await;
    field(
        &db.pool,
        Field {
            id: 1,
            name: "Color",
            ..Field::default()
        },
    )
    .await;
    let mut user1 = Client::logged_in(&st, "user1").await;
    let page = user1.page("/u/user1/preferences/profile").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("data-user-field=\"1\""), "{}", page.body);
    assert!(!page.body.contains("prefs-choosers"), "{}", page.body);
}

#[tokio::test]
async fn saving_a_multiselect_stores_a_row_per_option() {
    let (db, st) = app().await;
    multiselects(&db.pool).await;
    let mut user1 = Client::logged_in(&st, "user1").await;

    // A list: the options among it, in the order given, one row each.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("user_fields[3][]", "ruby"),
                ("user_fields[3][]", "cobol"),
                ("user_fields[3][]", "rust"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json()["user"]["user_fields"]["3"],
        json!(["ruby", "rust"])
    );
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[("user_field_3", "ruby"), ("user_field_3", "rust")])
    );
    let user = user1.send(Method::GET, "/u/user1.json", &[]).await.json();
    assert_eq!(user["user"]["user_fields"]["3"], json!(["ruby", "rust"]));

    // One choice comes back as the row's string.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[3][]", "rust")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["user_fields"]["3"], json!("rust"));
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[("user_field_3", "rust")])
    );

    // Emptied, as the page sends it (null: an empty value, which
    // custom_fields keeps as the one row).
    let reply = user1
        .send(Method::PUT, "/u/user1.json", &[("user_fields[3]", "")])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["user_fields"]["3"], json!(""));
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[("user_field_3", "")])
    );
    // The page draws that as nothing chosen.
    let page = user1.page("/u/user1/preferences/profile").await;
    let langs = component(&page.body, 3);
    assert!(
        langs.contains(
            "class=\"select-kit multi-select\" data-chooser=\"options\" data-items=\"[]\""
        ),
        "{langs}"
    );
}

#[tokio::test]
async fn a_member_cannot_set_a_multiselect_only_staff_edit() {
    let (db, st) = app().await;
    multiselects(&db.pool).await;
    let mut user1 = Client::logged_in(&st, "user1").await;
    let reply = user1
        .send(Method::PUT, "/u/user1.json", &[("user_fields[4][]", "vip")])
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(stored(&db.pool, "user1").await, vec![]);

    let mut admin = Client::logged_in(&st, "admin").await;
    let reply = admin
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[4][]", "vip"), ("user_fields[4][]", "press")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[("user_field_4", "vip"), ("user_field_4", "press")])
    );
}

#[tokio::test]
async fn a_required_multiselect_cannot_be_emptied() {
    let (db, st) = app().await;
    field(
        &db.pool,
        Field {
            id: 3,
            name: "Langs",
            kind: MULTISELECT,
            requirement: FOR_ALL_USERS,
            options: &["rust", "ruby"],
            ..Field::default()
        },
    )
    .await;
    // The fields' version the user has met, so the page is the profile's
    // own and not the required fields' notice.
    sqlx::query(
        "UPDATE users SET required_fields_version = \
           (SELECT COALESCE(MAX(id), 0) FROM user_required_fields_versions) WHERE username = 'user1'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    set_field(&db.pool, "user1", 3, &["rust", "ruby"]).await;
    let mut user1 = Client::logged_in(&st, "user1").await;
    let page = user1.page("/u/user1/preferences/profile").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let langs = component(&page.body, 3);
    // Required: no "(optional)".
    assert!(
        langs.contains("for=\"user-field-3-input\">Langs</label>"),
        "{langs}"
    );

    for emptied in [
        vec![("user_fields[3]", "")],
        // Nothing that is an option.
        vec![("user_fields[3][]", "cobol")],
    ] {
        let reply = user1.send(Method::PUT, "/u/user1.json", &emptied).await;
        assert_eq!(
            reply.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            reply.body
        );
        assert_eq!(
            reply.json()["errors"],
            json!(["You have not completed all the required user fields"])
        );
        assert_eq!(
            stored(&db.pool, "user1").await,
            rows(&[("user_field_3", "rust"), ("user_field_3", "ruby")])
        );
    }
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[3][]", "ruby")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[("user_field_3", "ruby")])
    );
}

const INVITES: &str = "INSERT INTO invites (id, invite_key, email, invited_by_id, created_at, updated_at, \
     emailed_status, max_redemptions_allowed, redemption_count, expires_at) VALUES \
     (900, 'Fixture900', NULL, 3, now(), now(), 0, 50, 0, now() + interval '30 days'), \
     (901, 'Fixture901', 'friend@example.com', 3, now(), now(), 4, 1, 0, now() + interval '30 days')";

/// The classic invite form (login codes off) with the invites.
async fn invite_app() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    sqlx::query(INVITES).execute(&db.pool).await.unwrap();
    set_setting(&db.pool, "enable_local_logins_via_code", BOOL, "f").await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

/// A field of each type the invite form draws, one kept off signup.
async fn invite_fields(pool: &PgPool) {
    for f in [
        Field {
            id: 1,
            name: "Color",
            description: "<b>Any</b> color",
            requirement: ON_SIGNUP,
            position: 2,
            ..Field::default()
        },
        Field {
            id: 2,
            description: "I accept",
            kind: CONFIRM,
            requirement: FOR_ALL_USERS,
            position: 1,
            ..Field::default()
        },
        Field {
            id: 3,
            name: "Pet",
            kind: DROPDOWN,
            position: 3,
            options: &["cat", "dog"],
            ..Field::default()
        },
        Field {
            id: 4,
            name: "Langs",
            kind: MULTISELECT,
            position: 4,
            options: &["rust", "ruby"],
            ..Field::default()
        },
        Field {
            id: 5,
            name: "Story",
            kind: TEXTAREA,
            position: 5,
            ..Field::default()
        },
        Field {
            id: 7,
            name: "Internal",
            position: 0,
            show_on_signup: false,
            ..Field::default()
        },
    ] {
        field(pool, f).await;
    }
}

/// The page's #invite-strings.
fn invite_strings(html: &str) -> Value {
    let marker = "<script type=\"application/json\" id=\"invite-strings\">";
    let at = html.find(marker).expect("the page carries its strings") + marker.len();
    let end = html[at..].find("</script>").unwrap();
    serde_json::from_str(&html[at..at + end].replace("<\\/", "</")).unwrap()
}

#[tokio::test]
async fn the_invite_form_draws_the_fields_shown_on_signup() {
    let (db, st) = invite_app().await;
    let mut visitor = Client::new(&st).await;
    // No fields: no group, no scripts, nothing for invite.js.
    let page = visitor.page("/invites/Fixture900").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("<form class=\"invite-signup\""));
    assert!(
        !page.body.contains("class=\"user-fields\""),
        "{}",
        page.body
    );
    assert!(!page.body.contains("combo-box.js"), "{}", page.body);
    assert!(!page.body.contains("prefs-choosers"), "{}", page.body);
    assert!(invite_strings(&page.body)["user_fields"].is_null());

    invite_fields(&db.pool).await;
    let page = visitor.page("/invites/Fixture900").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let html = &page.body;
    // After the account's inputs, before the button; by position.
    let password = html.find("id=\"new-account-password\"").unwrap();
    let fields = html.find("<div class=\"user-fields\">").unwrap();
    let cta = html.find("<div class=\"invitation-cta\">").unwrap();
    assert!(password < fields && fields < cta, "{html}");
    let order: Vec<usize> = [2, 1, 3, 4, 5]
        .iter()
        .map(|id| html.find(&format!("data-user-field=\"{id}\"")).unwrap())
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(!html.contains("data-user-field=\"7\""), "{html}");
    // Each in its input group, with the signup form's markup.
    assert!(
        html.contains("<div class=\"user-fields\"><div class=\"input-group\"><div class=\"user-field confirm\"><div class=\"controls\"><label class=\"control-label checkbox-label\"><input id=\"user-field-2-input\" class=\"ember-checkbox\" type=\"checkbox\" data-user-field=\"2\"> <span>I accept<span class=\"required\">*</span></span></label><div class=\"tip good hidden\"></div></div></div></div>"),
        "{html}"
    );
    assert!(
        html.contains("<div class=\"input-group\"><div class=\"user-field text user-field-color\"><div class=\"controls\"><input id=\"user-field-1-input\" maxlength=\"2048\" class=\"ember-text-field\" type=\"text\" value=\"\" data-user-field=\"1\"><label class=\"control-label alt-placeholder\" for=\"user-field-1-input\">Color</label><div class=\"instructions\"><b>Any</b> color</div></div></div></div>"),
        "{html}"
    );
    let pet = component(html, 3);
    assert!(pet.contains("prefs-combo"), "{pet}");
    let langs = component(html, 4);
    assert!(
        langs.contains("data-chooser=\"options\" data-items=\"[]\""),
        "{langs}"
    );
    assert!(component(html, 5).contains("<textarea"), "{html}");
    // What invite.js validates them from, and the choosers' scripts.
    let strings = invite_strings(html);
    let ids: Vec<i64> = strings["user_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![2, 1, 3, 4, 5]);
    assert_eq!(strings["user_fields"][0]["required"], json!(true));
    assert_eq!(strings["user_fields"][2]["options"], json!(["cat", "dog"]));
    assert!(html.contains("/assets/combo-box.js?v="), "{html}");
    assert!(html.contains("id=\"prefs-choosers-strings\""), "{html}");
    assert!(html.contains("/assets/prefs-choosers.js?v="), "{html}");
    assert!(html.contains("/assets/invite.js?v="), "{html}");

    // An email invite's form draws them too.
    let page = visitor.page("/invites/Fixture901").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("data-user-field=\"1\""), "{}", page.body);

    // A member accepting has no form.
    let mut user2 = Client::logged_in(&st, "user2").await;
    let page = user2.page("/invites/Fixture900").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        !page.body.contains("class=\"user-fields\""),
        "{}",
        page.body
    );
    assert!(!page.body.contains("prefs-choosers"), "{}", page.body);
}

#[tokio::test]
async fn the_bundles_carry_the_shared_validation_helper() {
    let (_db, st) = app().await;
    let mut visitor = Client::new(&st).await;
    for asset in ["/assets/invite.js", "/assets/login.js"] {
        let script = visitor.page(asset).await;
        assert_eq!(script.status, StatusCode::OK);
        assert_eq!(
            script
                .body
                .matches("Discourse.userFields = function")
                .count(),
            1,
            "{asset}"
        );
        assert!(script.body.contains("Discourse.userFields("), "{asset}");
        // One implementation: neither page's script has its own.
        assert_eq!(
            script.body.matches("user_fields.required_checkbox").count(),
            1,
            "{asset}"
        );
    }
}

#[tokio::test]
async fn redeeming_stores_the_fields_as_given() {
    let (db, st) = invite_app().await;
    invite_fields(&db.pool).await;
    let mut visitor = Client::new(&st).await;
    let long = "x".repeat(2100);
    let reply = visitor
        .redeem(
            "Fixture900",
            "fielded",
            &[
                ("user_custom_fields[1]", "  blue  "),
                ("user_custom_fields[2]", "true"),
                // Not an option: InviteRedeemer cleans nothing.
                ("user_custom_fields[3]", "bird"),
                ("user_custom_fields[4][]", "ruby"),
                ("user_custom_fields[4][]", "cobol"),
                ("user_custom_fields[5]", long.as_str()),
                // Every UserField, shown on signup or not.
                ("user_custom_fields[7]", "inside"),
                ("user_custom_fields[99]", "no such field"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    let mut got = stored(&db.pool, "fielded").await;
    got.sort();
    let cut = "x".repeat(2048);
    let mut want = rows(&[
        ("user_field_1", "  blue  "),
        ("user_field_2", "true"),
        ("user_field_3", "bird"),
        ("user_field_4", "ruby"),
        ("user_field_4", "cobol"),
        ("user_field_5", cut.as_str()),
        ("user_field_7", "inside"),
    ]);
    want.sort();
    assert_eq!(got, want);
    // A multiselect's rows keep the order given.
    let langs: Vec<Option<String>> = stored(&db.pool, "fielded")
        .await
        .into_iter()
        .filter(|(k, _)| k == "user_field_4")
        .map(|(_, v)| v)
        .collect();
    assert_eq!(langs, vec![Some("ruby".into()), Some("cobol".into())]);
}

#[tokio::test]
async fn redeeming_requires_no_field_and_skips_the_blank_ones() {
    let (db, st) = invite_app().await;
    invite_fields(&db.pool).await;
    // The form's values untouched: empty, a confirm false, no list.
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .redeem(
            "Fixture900",
            "untouched",
            &[
                ("user_custom_fields[1]", ""),
                ("user_custom_fields[2]", "false"),
                ("user_custom_fields[3]", ""),
                ("user_custom_fields[4]", ""),
                ("user_custom_fields[5]", "   "),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    assert!(exists(&db.pool, "untouched").await);
    assert_eq!(stored(&db.pool, "untouched").await, vec![]);

    // Nothing sent at all.
    let mut visitor = Client::new(&st).await;
    let reply = visitor.redeem("Fixture900", "bare", &[]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    assert_eq!(stored(&db.pool, "bare").await, vec![]);

    // A hash for a value stores nothing (Parameters#[] with a range).
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .redeem(
            "Fixture900",
            "nested",
            &[
                ("user_custom_fields[1][a]", "b"),
                ("user_custom_fields[5]", "kept"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "nested").await,
        rows(&[("user_field_5", "kept")])
    );
}

#[tokio::test]
async fn values_sent_without_fields_defined_are_ignored() {
    let (db, st) = invite_app().await;
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .redeem(
            "Fixture900",
            "nofields",
            &[("user_custom_fields[1]", "blue")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    assert_eq!(stored(&db.pool, "nofields").await, vec![]);
}

#[tokio::test]
async fn a_member_redeeming_sets_no_fields() {
    let (db, st) = invite_app().await;
    invite_fields(&db.pool).await;
    let mut user2 = Client::logged_in(&st, "user2").await;
    let reply = user2
        .send(
            Method::PUT,
            "/invites/show/Fixture900.json",
            &[("timezone", "UTC"), ("user_custom_fields[1]", "blue")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    assert_eq!(stored(&db.pool, "user2").await, vec![]);
}

#[tokio::test]
async fn watched_words_apply_to_the_public_text_fields() {
    let (db, st) = invite_app().await;
    for f in [
        Field {
            id: 1,
            name: "Motto",
            show_on_profile: true,
            ..Field::default()
        },
        Field {
            id: 2,
            name: "Private note",
            ..Field::default()
        },
        Field {
            id: 3,
            name: "Pet",
            kind: DROPDOWN,
            show_on_profile: true,
            options: &["cat"],
            ..Field::default()
        },
    ] {
        field(&db.pool, f).await;
    }
    sqlx::query(
        "INSERT INTO watched_words (word, action, replacement, case_sensitive, created_at, updated_at) VALUES \
         ('forbidden', 1, NULL, false, now(), now()), \
         ('darn', 2, NULL, false, now(), now()), \
         ('colour', 5, 'color', false, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    // A blocked word in a public text field: User's validation, a 412.
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .redeem(
            "Fixture900",
            "blocked",
            &[("user_custom_fields[1]", "a forbidden motto")],
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::PRECONDITION_FAILED,
        "{}",
        reply.body
    );
    assert_eq!(
        reply.json(),
        json!({
            "failed": "FAILED",
            "message": "Validation failed: Sorry, you can't post the word 'forbidden'; it's not allowed.",
        })
    );
    assert!(!exists(&db.pool, "blocked").await);

    // Censored and replaced there; the fields that are not public, or
    // not text, are left alone.
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .redeem(
            "Fixture900",
            "censored",
            &[
                ("user_custom_fields[1]", "a darn fine colour"),
                ("user_custom_fields[2]", "forbidden darn colour"),
                ("user_custom_fields[3]", "forbidden"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], json!("OK"), "{}", reply.body);
    let mut got = stored(&db.pool, "censored").await;
    got.sort();
    assert_eq!(
        got,
        rows(&[
            ("user_field_1", "a ■■■■ fine color"),
            ("user_field_2", "forbidden darn colour"),
            ("user_field_3", "forbidden"),
        ])
    );

    // disable_watched_word_checking_in_user_fields: none of it.
    set_setting(
        &db.pool,
        "disable_watched_word_checking_in_user_fields",
        BOOL,
        "t",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut visitor = Client::new(&st).await;
    let reply = visitor
        .redeem(
            "Fixture900",
            "unchecked",
            &[("user_custom_fields[1]", "a forbidden darn colour")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "unchecked").await,
        rows(&[("user_field_1", "a forbidden darn colour")])
    );
}

/// Motto (1, text, on the profile), Note (2, text, not public), Pet (3,
/// dropdown, on the profile), Story (4, textarea, on the card) and Rank
/// (5, text, on the profile, only staff edit), with a blocked, a second
/// blocked, a censored, a replaced and a linked word.
async fn watched(pool: &PgPool) {
    for (id, name, kind, public, editable, options) in [
        (1, "Motto", TEXT, true, true, &[][..]),
        (2, "Note", TEXT, false, true, &[][..]),
        (3, "Pet", DROPDOWN, true, true, &["forbidden"][..]),
        (4, "Story", TEXTAREA, false, true, &[][..]),
        (5, "Rank", TEXT, true, false, &[][..]),
    ] {
        field(
            pool,
            Field {
                id,
                name,
                kind,
                editable,
                position: id,
                show_on_profile: public,
                options,
                ..Field::default()
            },
        )
        .await;
    }
    sqlx::query("UPDATE user_fields SET show_on_user_card = true WHERE id = 4")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO watched_words (word, action, replacement, case_sensitive, created_at, updated_at) VALUES \
         ('forbidden', 1, NULL, false, now(), now()), \
         ('banned', 1, NULL, false, now(), now()), \
         ('darn', 2, NULL, false, now(), now()), \
         ('colour', 5, 'color', false, now(), now()), \
         ('wiki', 8, 'https://example.com/wiki', false, now(), now())",
    )
    .execute(pool)
    .await
    .unwrap();
}

fn blocked(reply: &Reply, message: &str) {
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert_eq!(reply.json(), json!({ "errors": [message] }));
}

const FORBIDDEN: &str = "Sorry, you can't post the word 'forbidden'; it's not allowed.";

// No recording updates a user field holding a watched word: the tests
// below are read from User's `validates :validatable_user_fields_values,
// watched_words: true`, `before_save :apply_watched_words` and
// `should_skip_user_fields_validation?`, UserUpdater#update and
// users#update's json_result, against spec/models/user_spec.rb
// "#user_fields".
#[tokio::test]
async fn a_blocked_word_in_a_public_text_field_refuses_the_update() {
    let (db, st) = app().await;
    watched(&db.pool).await;
    let mut user1 = Client::logged_in(&st, "user1").await;

    // The user isn't saved; the profile, saved before it, is (no
    // rollback).
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("user_fields[1]", "a forbidden motto"),
                ("location", "Somewhere"),
            ],
        )
        .await;
    blocked(&reply, FORBIDDEN);
    assert_eq!(stored(&db.pool, "user1").await, vec![]);
    let location: Option<String> = sqlx::query_scalar(
        "SELECT p.location FROM user_profiles p JOIN users u ON u.id = p.user_id \
         WHERE u.username = 'user1'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(location.as_deref(), Some("Somewhere"));

    // The textarea on the card too, the values checked joined.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("user_fields[1]", "forbidden"),
                ("user_fields[4]", "a Banned\nstory"),
            ],
        )
        .await;
    blocked(
        &reply,
        "Sorry, you can't post that. Not allowed: Banned, forbidden.",
    );
    assert_eq!(stored(&db.pool, "user1").await, vec![]);

    // Not in a field that isn't public, nor in a dropdown's option.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("user_fields[2]", "forbidden and banned"),
                ("user_fields[3]", "forbidden"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[
            ("user_field_2", "forbidden and banned"),
            ("user_field_3", "forbidden"),
        ])
    );

    // Staff are not exempt, for another user or for themselves.
    let mut admin = Client::logged_in(&st, "admin").await;
    let reply = admin
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[5]", "forbidden")],
        )
        .await;
    blocked(&reply, FORBIDDEN);
    let reply = admin
        .send(
            Method::PUT,
            "/u/admin.json",
            &[("user_fields[1]", "forbidden")],
        )
        .await;
    blocked(&reply, FORBIDDEN);
    assert_eq!(stored(&db.pool, "admin").await, vec![]);

    // disable_watched_word_checking_in_user_fields: no validation.
    set_setting(
        &db.pool,
        "disable_watched_word_checking_in_user_fields",
        BOOL,
        "t",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user2 = Client::logged_in(&st, "user2").await;
    let reply = user2
        .send(
            Method::PUT,
            "/u/user2.json",
            &[("user_fields[1]", "a forbidden motto")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "user2").await,
        rows(&[("user_field_1", "a forbidden motto")])
    );
}

#[tokio::test]
async fn an_update_censors_and_replaces_in_the_public_text_fields() {
    let (db, st) = app().await;
    watched(&db.pool).await;
    let mut user1 = Client::logged_in(&st, "user1").await;

    // The link words replace nothing; the fields that are not public, or
    // not text, are left alone.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[
                ("user_fields[1]", "a Darn fine colour, see the wiki"),
                ("user_fields[2]", "darn colour"),
                ("user_fields[4]", "darn\ncolour"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json()["user"]["user_fields"]["1"],
        json!("a ■■■■ fine color, see the wiki")
    );
    let mut got = stored(&db.pool, "user1").await;
    got.sort();
    assert_eq!(
        got,
        rows(&[
            ("user_field_1", "a ■■■■ fine color, see the wiki"),
            ("user_field_2", "darn colour"),
            ("user_field_4", "■■■■\ncolor"),
        ])
    );

    // A field emptied alongside is stored empty.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[1]", ""), ("user_fields[4]", "oh darn")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let mut got = stored(&db.pool, "user1").await;
    got.sort();
    assert_eq!(
        got,
        rows(&[
            ("user_field_1", ""),
            ("user_field_2", "darn colour"),
            ("user_field_4", "oh ■■■■"),
        ])
    );

    // Staff are not exempt: the field only they edit is censored too.
    let mut admin = Client::logged_in(&st, "admin").await;
    let reply = admin
        .send(
            Method::PUT,
            "/u/user2.json",
            &[("user_fields[5]", "darn colour sergeant")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "user2").await,
        rows(&[("user_field_5", "■■■■ color sergeant")])
    );

    set_setting(
        &db.pool,
        "disable_watched_word_checking_in_user_fields",
        BOOL,
        "t",
    )
    .await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut user2 = Client::logged_in(&st, "user2").await;
    let reply = user2
        .send(
            Method::PUT,
            "/u/user2.json",
            &[("user_fields[1]", "a darn fine colour")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let mut got = stored(&db.pool, "user2").await;
    got.sort();
    assert_eq!(
        got,
        rows(&[
            ("user_field_1", "a darn fine colour"),
            ("user_field_5", "■■■■ color sergeant"),
        ])
    );
}

// `custom_fields_clean?`: the check runs when the merged custom fields
// differ from the stored ones, and then over every public text field the
// user has (`validatable_user_fields`), given or not.
#[tokio::test]
async fn an_update_checks_the_stored_public_fields_once_any_changes() {
    let (db, st) = app().await;
    watched(&db.pool).await;
    // Stored before the words were watched.
    set_field(&db.pool, "user1", 1, &["a forbidden motto"]).await;
    set_field(&db.pool, "user2", 1, &["a darn fine colour"]).await;
    set_field(&db.pool, "user2", 2, &["a note"]).await;

    // The same value again changes nothing: no validation.
    let mut user1 = Client::logged_in(&st, "user1").await;
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[1]", "a forbidden motto")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    // Another field changing validates the stored one too.
    let reply = user1
        .send(
            Method::PUT,
            "/u/user1.json",
            &[("user_fields[2]", "a note")],
        )
        .await;
    blocked(&reply, FORBIDDEN);
    assert_eq!(
        stored(&db.pool, "user1").await,
        rows(&[("user_field_1", "a forbidden motto")])
    );

    // Nor is anything censored without a change; with one, the stored
    // public field is, too.
    let mut user2 = Client::logged_in(&st, "user2").await;
    let reply = user2
        .send(
            Method::PUT,
            "/u/user2.json",
            &[("user_fields[2]", "a note")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "user2").await,
        rows(&[
            ("user_field_1", "a darn fine colour"),
            ("user_field_2", "a note"),
        ])
    );
    let reply = user2
        .send(
            Method::PUT,
            "/u/user2.json",
            &[("user_fields[2]", "another note")],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let mut got = stored(&db.pool, "user2").await;
    got.sort();
    assert_eq!(
        got,
        rows(&[
            ("user_field_1", "a ■■■■ fine color"),
            ("user_field_2", "another note"),
        ])
    );
}
