//! Security keys and passkeys (DiscourseWebauthn) against real payloads:
//! parity/webauthn/reference.json holds what Chromium's virtual
//! authenticator sent the reference (development: origin
//! http://localhost:3000, rp_id localhost) to register a security key and
//! a passkey for user1 and to log in with each, the challenges Rails
//! staged, and the rows Rails saved. The authenticator's private keys are
//! there too, so the logins through the port's own challenges are signed
//! as the authenticator signs them.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use base64::Engine;
use common::{TestDb, config, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use discourse_rs::webauthn::{self, FIRST_FACTOR, RelyingParty, SECOND_FACTOR};
use http_body_util::BodyExt;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const STANDARD: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn reference() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/parity/webauthn/reference.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn object(v: &Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

async fn relying_party(pool: &PgPool, env: RailsEnv) -> RelyingParty {
    let app = state(pool.clone(), config(env, &[])).await;
    let mut conn = pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &app.site_setting_defs,
        &app.config.globals,
    )
    .await
    .unwrap();
    RelyingParty::new(&app.config, &settings).unwrap()
}

async fn stage(pool: &PgPool, session_id: &str, user_id: Option<i32>, challenge: &str) {
    let mut conn = pool.acquire().await.unwrap();
    discourse_rs::session::server::set(
        &mut conn,
        session_id,
        &webauthn::session_challenge_key(user_id),
        &json!(challenge),
        300,
    )
    .await
    .unwrap();
}

/// The rows Rails saved for user1, and its secure identifier.
async fn insert_reference_keys(pool: &PgPool) {
    let r = reference();
    for which in ["security_key", "passkey"] {
        let row = &r[which]["row"];
        sqlx::query(
            "INSERT INTO user_security_keys (id, user_id, credential_id, public_key, factor_type, \
             enabled, name, created_at, updated_at) VALUES ($1, 3, $2, $3, $4, true, $5, now(), now())",
        )
        .bind(row["id"].as_i64().unwrap() + 9000)
        .bind(row["credential_id"].as_str().unwrap())
        .bind(row["public_key"].as_str().unwrap())
        .bind(row["factor_type"].as_i64().unwrap() as i32)
        .bind(row["name"].as_str().unwrap())
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE users SET secure_identifier = $1 WHERE id = 3")
        .bind(r["secure_identifier"].as_str().unwrap())
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn registers_the_recorded_credentials_as_rails_saved_them() {
    let db = TestDb::new().await;
    let rp = relying_party(&db.pool, RailsEnv::Development).await;
    let r = reference();
    for (which, factor_type) in [("security_key", SECOND_FACTOR), ("passkey", FIRST_FACTOR)] {
        stage(
            &db.pool,
            "sess",
            Some(3),
            r[which]["challenge"].as_str().unwrap(),
        )
        .await;
        let mut conn = db.pool.acquire().await.unwrap();
        let params = object(&r[which]["register_params"]);
        let key = webauthn::register(&mut conn, &rp, "sess", 3, &params, factor_type)
            .await
            .unwrap()
            .unwrap_or_else(|e| panic!("{which}: {e:?}"));
        let (credential_id, public_key, name, saved_type): (String, String, String, i32) =
            sqlx::query_as(
                "SELECT credential_id, public_key, name, factor_type FROM user_security_keys WHERE id = $1",
            )
            .bind(key.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let row = &r[which]["row"];
        assert_eq!(credential_id, row["credential_id"], "{which}");
        assert_eq!(public_key, row["public_key"], "{which}");
        assert_eq!(name, row["name"], "{which}");
        assert_eq!(saved_type, factor_type, "{which}");

        // The same credential again: credential_id_in_use_error.
        let again = webauthn::register(&mut conn, &rp, "sess", 3, &params, factor_type)
            .await
            .unwrap();
        assert_eq!(
            again,
            Err(webauthn::SecurityKeyError("credential_id_in_use_error"))
        );
    }
}

#[tokio::test]
async fn registration_refusals() {
    let db = TestDb::new().await;
    let r = reference();
    let params = object(&r["security_key"]["register_params"]);
    let mut conn = db.pool.acquire().await.unwrap();
    let dev = relying_party(&db.pool, RailsEnv::Development).await;
    // Nothing staged.
    assert_eq!(
        webauthn::register(&mut conn, &dev, "sess", 3, &params, SECOND_FACTOR)
            .await
            .unwrap(),
        Err(webauthn::SecurityKeyError("challenge_mismatch_error"))
    );
    stage(
        &db.pool,
        "sess",
        Some(3),
        r["security_key"]["challenge"].as_str().unwrap(),
    )
    .await;
    // Another origin.
    let test_rp = relying_party(&db.pool, RailsEnv::Test).await;
    assert_eq!(
        webauthn::register(&mut conn, &test_rp, "sess", 3, &params, SECOND_FACTOR)
            .await
            .unwrap(),
        Err(webauthn::SecurityKeyError("invalid_origin_error"))
    );
    // An assertion's client data is not a registration's.
    let mut wrong = params.clone();
    wrong.insert(
        "clientData".into(),
        r["security_key"]["assertion"]["clientData"].clone(),
    );
    assert_eq!(
        webauthn::register(&mut conn, &dev, "sess", 3, &wrong, SECOND_FACTOR)
            .await
            .unwrap(),
        Err(webauthn::SecurityKeyError("invalid_type_error"))
    );
    let mut wrong = params.clone();
    wrong.insert("attestation".into(), json!("bm90IGNib3I="));
    assert_eq!(
        webauthn::register(&mut conn, &dev, "sess", 3, &wrong, SECOND_FACTOR)
            .await
            .unwrap(),
        Err(webauthn::SecurityKeyError("malformed_attestation_error"))
    );
}

#[tokio::test]
async fn authenticates_the_recorded_assertions() {
    let db = TestDb::new().await;
    insert_reference_keys(&db.pool).await;
    let rp = relying_party(&db.pool, RailsEnv::Development).await;
    let r = reference();
    let mut conn = db.pool.acquire().await.unwrap();

    // The security key, for the user logging in.
    let assertion = r["security_key"]["assertion"].clone();
    let challenge = r["security_key"]["login_challenge"].as_str().unwrap();
    stage(&db.pool, "sess", Some(3), challenge).await;
    // A passkey ceremony does not take a second factor key.
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &rp,
            "sess",
            Some(3),
            Some(&assertion),
            &[FIRST_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("ownership_error"))
    );
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &rp,
            "sess",
            Some(2),
            Some(&assertion),
            &[SECOND_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("ownership_error"))
    );
    let mut tampered = assertion.clone();
    let mut signature = STANDARD
        .decode(assertion["signature"].as_str().unwrap())
        .unwrap();
    let last = signature.len() - 1;
    signature[last] ^= 1;
    tampered["signature"] = json!(STANDARD.encode(&signature));
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &rp,
            "sess",
            Some(3),
            Some(&tampered),
            &[SECOND_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("public_key_error"))
    );
    let key = webauthn::authenticate(
        &mut conn,
        &rp,
        "sess",
        Some(3),
        Some(&assertion),
        &[SECOND_FACTOR],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!((key.user_id, key.factor_type), (3, SECOND_FACTOR));
    // Spent: the challenge is cleared.
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &rp,
            "sess",
            Some(3),
            Some(&assertion),
            &[SECOND_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("challenge_mismatch_error"))
    );
    let used: bool =
        sqlx::query_scalar("SELECT last_used IS NOT NULL FROM user_security_keys WHERE id = $1")
            .bind(key.id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(used);

    // The passkey, with no user yet: its user handle names them.
    let assertion = r["passkey"]["assertion"].clone();
    let challenge = r["passkey"]["login_challenge"].as_str().unwrap();
    stage(&db.pool, "sess", None, challenge).await;
    let mut wrong_handle = assertion.clone();
    wrong_handle["userHandle"] = json!(STANDARD.encode("someone else"));
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &rp,
            "sess",
            None,
            Some(&wrong_handle),
            &[FIRST_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("ownership_error"))
    );
    let test_rp = relying_party(&db.pool, RailsEnv::Test).await;
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &test_rp,
            "sess",
            None,
            Some(&assertion),
            &[FIRST_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("invalid_origin_error"))
    );
    let key = webauthn::authenticate(
        &mut conn,
        &rp,
        "sess",
        None,
        Some(&assertion),
        &[FIRST_FACTOR],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!((key.user_id, key.factor_type), (3, FIRST_FACTOR));

    // Not a credential at all.
    for bad in [json!("000000"), json!({}), Value::Null] {
        assert_eq!(
            webauthn::authenticate(
                &mut conn,
                &rp,
                "sess",
                Some(3),
                Some(&bad),
                &[SECOND_FACTOR]
            )
            .await
            .unwrap(),
            Err(webauthn::SecurityKeyError(
                "malformed_public_key_credential_error"
            ))
        );
    }
    let unknown = json!({"credentialId": "bm9wZQ==", "signature": "", "clientData": "", "authenticatorData": ""});
    assert_eq!(
        webauthn::authenticate(
            &mut conn,
            &rp,
            "sess",
            Some(3),
            Some(&unknown),
            &[SECOND_FACTOR]
        )
        .await
        .unwrap(),
        Err(webauthn::SecurityKeyError("not_found_error"))
    );
}

/// What the virtual authenticator does with a challenge: client data for
/// the origin, authenticator data for localhost with the flags, the
/// signature over both with the credential's key.
fn assert_with(credential: &Value, challenge: &str, flags: u8, user_handle: Option<&str>) -> Value {
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::DecodePrivateKey;
    use sha2::Digest;
    let der = STANDARD
        .decode(credential["privateKey"].as_str().unwrap())
        .unwrap();
    let key = p256::ecdsa::SigningKey::from_pkcs8_der(&der).unwrap();
    let client_data = json!({
        "type": "webauthn.get",
        "challenge": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(challenge),
        "origin": "http://localhost:3000",
        "crossOrigin": false,
    })
    .to_string();
    let mut auth_data = sha2::Sha256::digest(b"localhost").to_vec();
    auth_data.push(flags);
    auth_data.extend_from_slice(&7u32.to_be_bytes());
    let mut signed = auth_data.clone();
    signed.extend_from_slice(&sha2::Sha256::digest(client_data.as_bytes()));
    let signature: p256::ecdsa::Signature = key.sign(&signed);
    let mut out = json!({
        "signature": STANDARD.encode(signature.to_der().as_bytes()),
        "clientData": STANDARD.encode(client_data.as_bytes()),
        "authenticatorData": STANDARD.encode(&auth_data),
        "credentialId": credential["credentialId"],
    });
    if let Some(handle) = user_handle {
        out["userHandle"] = json!(STANDARD.encode(handle));
    }
    out
}

/// The reference's credential for a factor type, by its Rails row.
fn credential(r: &Value, which: &str) -> Value {
    let id = r[which]["row"]["credential_id"].as_str().unwrap();
    r["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["credentialId"] == id)
        .cloned()
        .unwrap()
}

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
    async fn send(&mut self, method: Method, path: &str, body: &str) -> Reply {
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
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        for v in response.headers().get_all(header::SET_COOKIE) {
            let v = v.to_str().unwrap();
            let (pair, attrs) = v.split_once(';').unwrap_or((v, ""));
            let (name, value) = pair.split_once('=').unwrap();
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

    async fn new(state: AppState) -> Client {
        let mut client = Client {
            state,
            cookies: Vec::new(),
            csrf: None,
        };
        let reply = client.send(Method::GET, "/session/csrf.json", "").await;
        client.csrf = reply.json()["csrf"].as_str().map(str::to_string);
        client
    }
}

/// jQuery.param's nesting for an object param.
fn nested(name: &str, value: &Value) -> String {
    value
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            format!(
                "{name}%5B{k}%5D={}",
                form_urlencoded::byte_serialize(v.as_str().unwrap().as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[tokio::test]
async fn logs_in_with_a_security_key_and_with_a_passkey() {
    let db = TestDb::new().await;
    insert_reference_keys(&db.pool).await;
    let r = reference();
    let app = state(db.pool.clone(), config(RailsEnv::Development, &[])).await;

    // The password login stages the challenge and names the key.
    let mut client = Client::new(app.clone()).await;
    let login = "login=user1&password=password";
    let reply = client.send(Method::POST, "/session.json", login).await;
    let body = reply.json();
    assert_eq!(body["reason"], "invalid_second_factor_method", "{body}");
    assert_eq!(body["security_key_enabled"], true);
    assert_eq!(
        body["allowed_credential_ids"],
        json!([r["security_key"]["row"]["credential_id"]])
    );
    let challenge = body["challenge"].as_str().unwrap().to_string();
    assert_eq!(challenge.len(), 60);

    // Without user presence: the error is the key's.
    let assertion = assert_with(&credential(&r, "security_key"), &challenge, 0x00, None);
    let form = format!(
        "{login}&second_factor_method=3&{}",
        nested("second_factor_token", &assertion)
    );
    let reply = client.send(Method::POST, "/session.json", &form).await;
    assert_eq!(reply.json()["reason"], "invalid_security_key");
    assert_eq!(reply.json()["error"], "User presence is required.");

    // The failure staged a new challenge.
    let challenge = reply.json()["challenge"].as_str().unwrap().to_string();
    let assertion = assert_with(&credential(&r, "security_key"), &challenge, 0x01, None);
    let form = format!(
        "{login}&second_factor_method=3&{}",
        nested("second_factor_token", &assertion)
    );
    let reply = client.send(Method::POST, "/session.json", &form).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["username"], "user1", "{}", reply.body);

    // A passkey login from a fresh browser.
    let mut client = Client::new(app.clone()).await;
    let reply = client
        .send(Method::GET, "/session/passkey/challenge.json", "")
        .await;
    let challenge = reply.json()["challenge"].as_str().unwrap().to_string();
    let handle = r["secure_identifier"].as_str().unwrap();
    // Without user verification.
    let assertion = assert_with(&credential(&r, "passkey"), &challenge, 0x01, Some(handle));
    let form = nested("publicKeyCredential", &assertion);
    let reply = client
        .send(Method::POST, "/session/passkey/auth.json", &form)
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        reply.json(),
        json!({"errors": ["User verification is required."]})
    );
    let assertion = assert_with(&credential(&r, "passkey"), &challenge, 0x05, Some(handle));
    let form = nested("publicKeyCredential", &assertion);
    let reply = client
        .send(Method::POST, "/session/passkey/auth.json", &form)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["user"]["username"], "user1");
    let reply = client
        .send(Method::POST, "/session/passkey/auth.json", "")
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn registers_keys_and_confirms_the_session_with_a_passkey() {
    let db = TestDb::new().await;
    let app = state(db.pool.clone(), config(RailsEnv::Development, &[])).await;
    let mut client = Client::new(app.clone()).await;
    let reply = client
        .send(
            Method::POST,
            "/session.json",
            "login=user1&password=password",
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let reply = client.send(Method::GET, "/session/csrf.json", "").await;
    client.csrf = reply.json()["csrf"].as_str().map(str::to_string);

    // Unconfirmed: the security key needs the session confirmed.
    let reply = client
        .send(
            Method::POST,
            "/u/create_second_factor_security_key.json",
            "",
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    // create_passkey does not.
    let reply = client
        .send(Method::POST, "/u/create_passkey.json", "")
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let body = reply.json();
    assert_eq!(body["rp_id"], "localhost");
    assert_eq!(body["rp_name"], "Discourse");
    assert_eq!(
        body["supported_algorithms"],
        json!([-7, -8, -35, -36, -37, -38, -39, -257])
    );
    assert_eq!(body["existing_passkey_credential_ids"], json!([]));
    let secure_id = body["user_secure_id"].as_str().unwrap().to_string();
    assert_eq!(secure_id.len(), 40);

    let reply = client
        .send(Method::POST, "/u/confirm-session.json", "password=password")
        .await;
    assert_eq!(reply.json(), json!({"success": "OK"}));
    let reply = client
        .send(
            Method::POST,
            "/u/create_second_factor_security_key.json",
            "",
        )
        .await;
    let body = reply.json();
    assert_eq!(body["user_secure_id"], secure_id.as_str());
    assert_eq!(body["existing_active_credential_ids"], json!([]));
    // A registration the challenge was not staged for.
    let r = reference();
    let params = &r["security_key"]["register_params"];
    let form = params
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}={}",
                form_urlencoded::byte_serialize(v.as_str().unwrap().as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    let reply = client
        .send(
            Method::POST,
            "/u/register_second_factor_security_key.json",
            &form,
        )
        .await;
    assert_eq!(
        reply.json(),
        json!({
            "failed": "FAILED",
            "error": "The provided challenge does not match the challenge generated by the authentication server."
        })
    );
    let reply = client
        .send(Method::POST, "/u/register_passkey.json", &form)
        .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);

    // Confirming the session with the reference's passkey, its rows in.
    insert_reference_keys(&db.pool).await;
    let reply = client
        .send(Method::GET, "/session/passkey/challenge.json", "")
        .await;
    let challenge = reply.json()["challenge"].as_str().unwrap().to_string();
    let assertion = assert_with(&credential(&r, "passkey"), &challenge, 0x05, None);
    let form = nested("publicKeyCredential", &assertion);
    let reply = client
        .send(Method::POST, "/u/confirm-session.json", &form)
        .await;
    assert_eq!(reply.json(), json!({"success": "OK"}), "{}", reply.body);

    // Renaming, disabling and deleting.
    let reply = client
        .send(Method::PUT, "/u/rename_passkey/9002.json", "name=Laptop")
        .await;
    assert_eq!(reply.json(), json!({"success": "OK"}));
    let reply = client
        .send(
            Method::PUT,
            "/u/security_key.json",
            "id=9001&name=Yubi&disable=true",
        )
        .await;
    assert_eq!(reply.json(), json!({"success": "OK"}));
    let rows: Vec<(i64, String, bool)> =
        sqlx::query_as("SELECT id, name, enabled FROM user_security_keys ORDER BY id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        rows,
        vec![(9001, "Yubi".into(), false), (9002, "Laptop".into(), true)]
    );
    let reply = client
        .send(Method::DELETE, "/u/delete_passkey/9002.json", "")
        .await;
    assert_eq!(reply.json(), json!({"success": "OK"}));
}

#[tokio::test]
async fn the_2fa_page_offers_security_keys() {
    let db = TestDb::new().await;
    insert_reference_keys(&db.pool).await;
    sqlx::query("UPDATE users SET admin = true WHERE id = 3")
        .execute(&db.pool)
        .await
        .unwrap();
    let app = state(db.pool.clone(), config(RailsEnv::Development, &[])).await;
    let mut client = Client::new(app).await;
    // A password login with the key.
    let reply = client
        .send(
            Method::POST,
            "/session.json",
            "login=user1&password=password",
        )
        .await;
    let challenge = reply.json()["challenge"].as_str().unwrap().to_string();
    let r = reference();
    let assertion = assert_with(&credential(&r, "security_key"), &challenge, 0x01, None);
    let form = format!(
        "login=user1&password=password&second_factor_method=3&{}",
        nested("second_factor_token", &assertion)
    );
    let reply = client.send(Method::POST, "/session.json", &form).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    let reply = client.send(Method::GET, "/session/csrf.json", "").await;
    client.csrf = reply.json()["csrf"].as_str().map(str::to_string);

    // Granting admin asks for the second factor.
    let reply = client
        .send(Method::PUT, "/admin/users/2/grant_admin.json", "")
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
    let nonce = reply.json()["second_factor_challenge_nonce"]
        .as_str()
        .unwrap()
        .to_string();
    let reply = client
        .send(Method::GET, &format!("/session/2fa.json?nonce={nonce}"), "")
        .await;
    let body = reply.json();
    assert_eq!(body["security_keys_enabled"], true, "{body}");
    assert_eq!(body["passkeys_enabled"], false);
    assert_eq!(
        body["allowed_credential_ids"],
        json!([r["security_key"]["row"]["credential_id"]])
    );
    let challenge = body["challenge"].as_str().unwrap().to_string();
    let assertion = assert_with(&credential(&r, "security_key"), &challenge, 0x01, None);
    let form = format!(
        "nonce={nonce}&second_factor_method=3&{}",
        nested("second_factor_token", &assertion)
    );
    let reply = client.send(Method::POST, "/session/2fa.json", &form).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    assert_eq!(reply.json()["ok"], true);
}
