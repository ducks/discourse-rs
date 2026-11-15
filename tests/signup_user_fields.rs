//! The signup form's user fields (templates/signup.gjs over UserField and
//! components/user-fields/*, UserFieldsValidationHelper's fields) and
//! users#create's `user_fields` (UserCreator.assign_signup_fields and
//! clean_custom_field_values). The markup is read from the templates. The
//! recorded parity/writes/user_fields/signup case covers a missing
//! required field and a signup whose multiselect, dropdown and confirm
//! values are cleaned; the other branches here are read from
//! lib/user_creator.rb.

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
const DATE: i32 = 5;
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

    /// The page's #login-data.
    fn data(&self) -> Value {
        let marker = "<script type=\"application/json\" id=\"login-data\">";
        let at = self.body.find(marker).expect("the page carries its data") + marker.len();
        let end = self.body[at..].find("</script>").unwrap();
        serde_json::from_str(&self.body[at..at + end]).unwrap()
    }
}

impl Client {
    /// A client with the session's CSRF token.
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

    /// POST /u.json as the signup form sends it, with `fields` the
    /// `user_fields[...]` pairs.
    async fn sign_up(&mut self, username: &str, fields: &[(&str, &str)]) -> Reply {
        let hp = self.send(Method::GET, "/session/hp.json", &[]).await.json();
        let value = hp["value"].as_str().unwrap().to_string();
        let challenge: String = hp["challenge"].as_str().unwrap().chars().rev().collect();
        let email = format!("{username}@example.com");
        let mut form = vec![
            ("name", "New Member"),
            ("email", email.as_str()),
            ("password", PASSWORD),
            ("username", username),
            ("password_confirmation", value.as_str()),
            ("challenge", challenge.as_str()),
        ];
        form.extend_from_slice(fields);
        self.send(Method::POST, "/u.json", &form).await
    }
}

async fn page(st: &AppState, path: &str) -> Reply {
    let response = discourse_rs::app(st.clone())
        .oneshot(
            Request::get(path)
                .header(header::HOST, "test.localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    Reply {
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn app() -> (TestDb, AppState) {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    (db, st)
}

async fn code_logins(pool: &PgPool, on: bool) {
    set_setting(
        pool,
        "enable_local_logins_via_code",
        BOOL,
        if on { "t" } else { "f" },
    )
    .await;
}

struct Field<'a> {
    id: i32,
    name: &'a str,
    description: &'a str,
    kind: i32,
    requirement: i32,
    position: i32,
    show_on_signup: bool,
    options: &'a [&'a str],
}

impl Default for Field<'_> {
    fn default() -> Self {
        Field {
            id: 0,
            name: "",
            description: "About it",
            kind: TEXT,
            requirement: OPTIONAL,
            position: 0,
            show_on_signup: true,
            options: &[],
        }
    }
}

async fn field(pool: &PgPool, f: Field<'_>) {
    sqlx::query(
        "INSERT INTO user_fields (id, name, description, field_type_enum, editable, requirement, \
           position, show_on_signup, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, true, $5, $6, $7, now(), now())",
    )
    .bind(f.id)
    .bind(f.name)
    .bind(f.description)
    .bind(f.kind)
    .bind(f.requirement)
    .bind(f.position)
    .bind(f.show_on_signup)
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

async fn exists(pool: &PgPool, username: &str) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = $1)")
        .bind(username)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn missing(reply: &Reply) {
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(
        reply.json(),
        json!({
            "success": false,
            "message": "You have not completed all the required user fields",
        })
    );
}

/// One field of each type, a second confirm with a name, and one kept
/// off the signup form.
async fn every_type(pool: &PgPool) {
    for f in [
        Field {
            id: 1,
            name: "Favorite Color!",
            description: "<b>Any</b> color",
            requirement: ON_SIGNUP,
            position: 2,
            ..Field::default()
        },
        Field {
            id: 2,
            description: "I accept the <a href=\"/tos\">terms</a>",
            kind: CONFIRM,
            requirement: FOR_ALL_USERS,
            position: 1,
            ..Field::default()
        },
        Field {
            id: 3,
            name: "Pet",
            description: "Your pet",
            kind: DROPDOWN,
            position: 3,
            options: &["cat", "dog & co"],
            ..Field::default()
        },
        Field {
            id: 4,
            name: "Langs",
            description: "Languages",
            kind: MULTISELECT,
            position: 4,
            options: &["rust", "ruby"],
            ..Field::default()
        },
        Field {
            id: 5,
            name: "Story",
            description: "Tell us",
            kind: TEXTAREA,
            requirement: ON_SIGNUP,
            position: 5,
            ..Field::default()
        },
        Field {
            id: 6,
            name: "Birthday",
            description: "When",
            kind: DATE,
            position: 6,
            ..Field::default()
        },
        Field {
            id: 7,
            name: "Internal",
            position: 0,
            show_on_signup: false,
            ..Field::default()
        },
        Field {
            id: 8,
            name: "Rules",
            description: "Read them",
            kind: CONFIRM,
            position: 7,
            ..Field::default()
        },
    ] {
        field(pool, f).await;
    }
}

#[tokio::test]
async fn the_signup_form_draws_each_field_shown_on_signup() {
    let (db, st) = app().await;
    code_logins(&db.pool, false).await;

    // No fields: no list, no scripts of theirs.
    let none = page(&st, "/signup").await;
    assert_eq!(none.status, StatusCode::OK);
    assert!(
        !none.body.contains("class=\"user-fields\""),
        "{}",
        none.body
    );
    assert!(!none.body.contains("combo-box.js"), "{}", none.body);
    assert!(!none.body.contains("prefs-choosers"), "{}", none.body);
    assert_eq!(none.data()["user_fields"], json!([]));

    every_type(&db.pool).await;
    let reply = page(&st, "/signup").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    assert!(!html.contains("[en."), "every string is translated");

    // After the form's own inputs, by position; the one kept off signup
    // is not drawn.
    let start = html.find("<div class=\"user-fields\">").expect("the list");
    let end = html[start..].find("</form>").unwrap();
    let list = &html[start..start + end];
    assert!(!html.contains("Internal"), "{html}");
    let confirm = "<div class=\"input-group\"><div class=\"user-field confirm\"><div class=\"controls\"><label class=\"control-label checkbox-label\"><input id=\"user-field-2-input\" class=\"ember-checkbox\" type=\"checkbox\" data-user-field=\"2\"> <span>I accept the <a href=\"/tos\">terms</a><span class=\"required\">*</span></span></label><div class=\"tip good hidden\"></div></div></div></div>";
    let text = "<div class=\"input-group\"><div class=\"user-field text user-field-favorite-color\"><div class=\"controls\"><input id=\"user-field-1-input\" maxlength=\"2048\" class=\"ember-text-field\" type=\"text\" value=\"\" data-user-field=\"1\"><label class=\"control-label alt-placeholder\" for=\"user-field-1-input\">Favorite Color!</label><div class=\"instructions\"><b>Any</b> color</div></div></div></div>";
    let dropdown_start = "<div class=\"input-group\"><div class=\"user-field dropdown user-field-pet\"><label class=\"control-label alt-placeholder\" for=\"user-field-3-input\">Pet(optional)</label><div class=\"controls\"><details data-user-field=\"3\" id=\"user-field-3-input\" class=\"select-kit single-select combobox combo-box prefs-combo\"><summary aria-label=\"Filter by: (select an option)\" name=\"Filter by: (select an option)\" data-name=\"(select an option)\" tabindex=\"0\" id=\"user-field-3-input-header\" class=\"select-kit-header single-select-header combo-box-header\">";
    let dropdown_rows = "<ul aria-live=\"polite\" class=\"select-kit-collection\" role=\"menu\"><li aria-checked=\"true\" role=\"menuitemradio\" data-index=\"0\" data-name=\"(select an option)\" data-value=\"\" title=\"(select an option)\" tabindex=\"0\" class=\"select-kit-row none is-highlighted\"><span class=\"name\">(select an option)</span></li><li aria-checked=\"false\" role=\"menuitemradio\" data-index=\"1\" data-name=\"cat\" data-value=\"cat\" title=\"cat\" tabindex=\"0\" class=\"select-kit-row\"><span class=\"name\">cat</span></li><li aria-checked=\"false\" role=\"menuitemradio\" data-index=\"2\" data-name=\"dog &amp; co\" data-value=\"dog &amp; co\" title=\"dog &amp; co\" tabindex=\"0\" class=\"select-kit-row\"><span class=\"name\">dog &amp; co</span></li></ul></div></details><div class=\"instructions\">Your pet</div></div></div></div>";
    let multiselect = "<div class=\"input-group\"><div class=\"user-field multiselect user-field-langs\"><label class=\"control-label alt-placeholder\" for=\"user-field-4-input\">Langs(optional)</label><div class=\"controls\"><details id=\"user-field-4-input\" data-user-field=\"4\" data-none=\"(select an option)\" data-options=\"[&quot;rust&quot;,&quot;ruby&quot;]\" class=\"select-kit multi-select\" data-chooser=\"options\" data-items=\"[]\"><summary data-name=\"\" data-value=\"\" tabindex=\"0\" class=\"select-kit-header multi-select-header\"><div class=\"select-kit-header-wrapper\"><span class=\"formatted-selection\">(select an option)</span>";
    let multiselect_end = "</div></summary><div class=\"select-kit-body\"></div></details><div class=\"instructions\">Languages</div></div></div></div>";
    let textarea = "<div class=\"input-group\"><div class=\"user-field textarea user-field-story\"><div class=\"controls\"><textarea class=\"form-template-field__textarea ember-text-area\" id=\"user-field-5-input\" maxlength=\"2048\" data-user-field=\"5\"></textarea><label class=\"control-label alt-placeholder\" for=\"user-field-5-input\">Story</label><div class=\"instructions\">Tell us</div></div></div></div>";
    let date = "<div class=\"input-group\"><div class=\"user-field date user-field-birthday\"><div class=\"controls\"><input id=\"user-field-6-input\" class=\"ember-text-field\" type=\"date\" value=\"\" data-user-field=\"6\"><label class=\"control-label alt-placeholder\" for=\"user-field-6-input\">Birthday(optional)</label><div class=\"instructions\">When</div></div></div></div>";
    let named_confirm = "<div class=\"input-group\"><div class=\"user-field confirm user-field-rules\"><label class=\"control-label\">Rules(optional)</label><div class=\"controls\"><label class=\"control-label checkbox-label\"><input id=\"user-field-8-input\" class=\"ember-checkbox\" type=\"checkbox\" data-user-field=\"8\"> <span>Read them</span></label><div class=\"tip good hidden\"></div></div></div></div>";
    let mut at = 0;
    for needle in [
        confirm,
        text,
        dropdown_start,
        dropdown_rows,
        multiselect,
        multiselect_end,
        textarea,
        date,
        named_confirm,
    ] {
        let found = list[at..]
            .find(needle)
            .unwrap_or_else(|| panic!("{needle}\n{}", &list[at..]));
        at += found + needle.len();
    }
    assert_eq!(&list[at..], "</div>", "nothing after the last field");
    assert!(
        list.starts_with(&format!("<div class=\"user-fields\">{confirm}{text}")),
        "{list}"
    );

    // The dropdown's and the multiselect's scripts, with select-kit's
    // strings; what the validation reads of each field.
    assert!(html.contains("/assets/combo-box.js?v="), "{html}");
    assert!(html.contains("/assets/prefs-choosers.js?v="), "{html}");
    let marker = "<script type=\"application/json\" id=\"prefs-choosers-strings\">";
    let strings_at = html.find(marker).expect("the strings") + marker.len();
    let strings: Value = serde_json::from_str(
        &html[strings_at..strings_at + html[strings_at..].find("</script>").unwrap()],
    )
    .unwrap();
    assert_eq!(
        strings,
        json!({
            "default_header": "(select an option)",
            "filter_placeholder": "Search…",
            "results_one": "%{count} result",
            "results_other": "%{count} results",
            "delete_item": "Delete %{name}",
        })
    );
    let data = reply.data();
    assert_eq!(
        data["user_fields"],
        json!([
            {"id": 2, "name": "", "field_type": "confirm", "required": true},
            {"id": 1, "name": "Favorite Color!", "field_type": "text", "required": true},
            {"id": 3, "name": "Pet", "field_type": "dropdown", "required": false},
            {"id": 4, "name": "Langs", "field_type": "multiselect", "required": false},
            {"id": 5, "name": "Story", "field_type": "textarea", "required": true},
            {"id": 6, "name": "Birthday", "field_type": "date", "required": false},
            {"id": 8, "name": "Rules", "field_type": "confirm", "required": false},
        ])
    );
    for key in [
        "user_fields.required",
        "user_fields.required_checkbox",
        "user_fields.required_select",
        "user_fields.same_as_password",
    ] {
        assert!(data["strings"][key].is_string(), "{key}");
    }
    assert_eq!(
        data["strings"]["user_fields.required"],
        "Please enter a value for \"%{name}\""
    );
}

#[tokio::test]
async fn only_the_scripts_the_fields_need_are_loaded() {
    let (db, st) = app().await;
    code_logins(&db.pool, false).await;
    field(
        &db.pool,
        Field {
            id: 1,
            name: "Color",
            ..Field::default()
        },
    )
    .await;
    let html = page(&st, "/signup").await.body;
    assert!(html.contains("data-user-field=\"1\""), "{html}");
    assert!(!html.contains("combo-box.js"), "{html}");
    assert!(!html.contains("prefs-choosers"), "{html}");
}

#[tokio::test]
async fn the_code_signup_keeps_the_fields_for_an_external_logins_form() {
    let (db, st) = app().await;
    code_logins(&db.pool, true).await;
    every_type(&db.pool).await;
    let reply = page(&st, "/signup").await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let html = &reply.body;
    // The email step has no fields; the create-account form an external
    // login's data draws has them.
    let template = html.find("<template id=\"signup-auth-form\">").unwrap();
    let list = html.find("<div class=\"user-fields\">").expect("the list");
    assert!(list > template, "{html}");
    assert_eq!(html.matches("<div class=\"user-fields\">").count(), 1);
    assert_eq!(reply.data()["user_fields"].as_array().unwrap().len(), 7);
}

#[tokio::test]
async fn a_signup_stores_each_fields_cleaned_value() {
    let (db, st) = app().await;
    every_type(&db.pool).await;
    let mut client = Client::new(&st).await;
    let long = "x".repeat(2100);
    let reply = client
        .sign_up(
            "fieldsfull",
            &[
                ("user_fields[1]", &long),
                ("user_fields[2]", "true"),
                ("user_fields[3]", "dog & co"),
                ("user_fields[4][]", "ruby"),
                ("user_fields[4][]", "go"),
                ("user_fields[4][]", "rust"),
                ("user_fields[5]", "Once\nupon a time"),
                ("user_fields[6]", "2001-02-03"),
                // Not on the form, and still taken: every field is read.
                ("user_fields[7]", "kept"),
                ("user_fields[8]", "false"),
            ],
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
    let mut rows = stored(&db.pool, "fieldsfull").await;
    rows.sort();
    let s = |v: &str| Some(v.to_string());
    assert_eq!(
        rows,
        vec![
            // Cut to UserField.max_length.
            ("user_field_1".to_string(), s(&"x".repeat(2048))),
            ("user_field_2".to_string(), s("true")),
            ("user_field_3".to_string(), s("dog & co")),
            // The values that are options, in the order sent.
            ("user_field_4".to_string(), s("ruby")),
            ("user_field_4".to_string(), s("rust")),
            ("user_field_5".to_string(), s("Once\nupon a time")),
            ("user_field_6".to_string(), s("2001-02-03")),
            ("user_field_7".to_string(), s("kept")),
        ]
    );
    let order: Vec<Option<String>> = stored(&db.pool, "fieldsfull")
        .await
        .into_iter()
        .filter(|(name, _)| name == "user_field_4")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(order, vec![s("ruby"), s("rust")]);
}

#[tokio::test]
async fn optional_fields_left_empty_or_invalid_store_nothing() {
    let (db, st) = app().await;
    every_type(&db.pool).await;
    let mut client = Client::new(&st).await;
    // As the form sends an untouched field: empty. A dropdown value that
    // is no option, a multiselect with none that are, an unchecked
    // confirm.
    let reply = client
        .sign_up(
            "fieldsbare",
            &[
                ("user_fields[1]", "Blue"),
                ("user_fields[2]", "true"),
                ("user_fields[3]", "horse"),
                ("user_fields[4][]", "go"),
                ("user_fields[5]", "Short"),
                ("user_fields[6]", ""),
                ("user_fields[8]", "false"),
            ],
        )
        .await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "fieldsbare").await,
        vec![
            ("user_field_1".to_string(), Some("Blue".to_string())),
            ("user_field_2".to_string(), Some("true".to_string())),
            ("user_field_5".to_string(), Some("Short".to_string())),
        ]
    );

    // A multiselect given one value outside a list is wrapped.
    let mut client = Client::new(&st).await;
    let reply = client
        .sign_up(
            "fieldsone",
            &[
                ("user_fields[1]", "Blue"),
                ("user_fields[2]", "true"),
                ("user_fields[4]", "rust"),
                ("user_fields[5]", "Short"),
            ],
        )
        .await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);
    assert_eq!(
        stored(&db.pool, "fieldsone").await,
        vec![
            ("user_field_1".to_string(), Some("Blue".to_string())),
            ("user_field_2".to_string(), Some("true".to_string())),
            ("user_field_4".to_string(), Some("rust".to_string())),
            ("user_field_5".to_string(), Some("Short".to_string())),
        ]
    );
}

#[tokio::test]
async fn a_required_field_left_blank_refuses_the_signup() {
    let (db, st) = app().await;
    every_type(&db.pool).await;
    let complete = [
        ("user_fields[1]", "Blue"),
        ("user_fields[2]", "true"),
        ("user_fields[5]", "Short"),
    ];
    let without = |key: &str, value: Option<&'static str>| {
        let mut fields: Vec<(&str, &str)> = complete
            .iter()
            .copied()
            .filter(|(k, _)| *k != key)
            .collect();
        if let Some(value) = value {
            fields.push((complete.iter().find(|(k, _)| *k == key).unwrap().0, value));
        }
        fields
    };
    let cases: Vec<(&str, Vec<(&str, &str)>)> = vec![
        // No user_fields at all.
        ("none", Vec::new()),
        // on_signup: absent, empty, whitespace.
        ("textabsent", without("user_fields[1]", None)),
        ("textempty", without("user_fields[1]", Some(""))),
        ("textblank", without("user_fields[1]", Some(" \t"))),
        // for_all_users: an unchecked confirm is "false", which is nil.
        ("confirmfalse", without("user_fields[2]", Some("false"))),
        ("confirmabsent", without("user_fields[2]", None)),
        ("areaempty", without("user_fields[5]", Some(""))),
    ];
    for (name, fields) in cases {
        let mut client = Client::new(&st).await;
        let username = format!("refused{name}");
        let reply = client.sign_up(&username, &fields).await;
        missing(&reply);
        assert!(!exists(&db.pool, &username).await, "{name}");
    }

    // A required dropdown and multiselect: a value that is no option is
    // as blank as none.
    sqlx::query("UPDATE user_fields SET requirement = $1 WHERE id IN (3, 4)")
        .bind(ON_SIGNUP)
        .execute(&db.pool)
        .await
        .unwrap();
    for (name, extra) in [
        (
            "dropbad",
            vec![("user_fields[3]", "horse"), ("user_fields[4][]", "rust")],
        ),
        ("dropnone", vec![("user_fields[4][]", "rust")]),
        (
            "multibad",
            vec![("user_fields[3]", "cat"), ("user_fields[4][]", "go")],
        ),
        ("multinone", vec![("user_fields[3]", "cat")]),
    ] {
        let mut fields = complete.to_vec();
        fields.extend(extra);
        let mut client = Client::new(&st).await;
        let username = format!("refused{name}");
        missing(&client.sign_up(&username, &fields).await);
        assert!(!exists(&db.pool, &username).await, "{name}");
    }
    let mut fields = complete.to_vec();
    fields.extend([("user_fields[3]", "cat"), ("user_fields[4][]", "rust")]);
    let mut client = Client::new(&st).await;
    let reply = client.sign_up("acceptedall", &fields).await;
    assert_eq!(reply.json()["success"], true, "{}", reply.body);

    // A required field the form does not show is required all the same
    // (UserField.all).
    sqlx::query("UPDATE user_fields SET requirement = $1 WHERE id = 7")
        .bind(FOR_ALL_USERS)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut client = Client::new(&st).await;
    missing(&client.sign_up("refusedhidden", &fields).await);
    assert!(!exists(&db.pool, "refusedhidden").await);
}
