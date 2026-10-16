//! `DiscourseWebauthn` (lib/discourse_webauthn.rb): the challenge staged in
//! the server session for a security key or passkey, and the credentials a
//! user may answer it with; AuthenticationService and RegistrationService
//! (lib/discourse_webauthn/), with the cose gem's part in crate::cose.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::config::{Config, RailsEnv};
use crate::cose;
use crate::discourse_connect::decode64;
use crate::i18n::I18n;
use crate::second_factor::Enabled;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `CHALLENGE_EXPIRY`
const CHALLENGE_EXPIRY_SECONDS: i64 = 5 * 60;

/// `UserSecurityKey.factor_types`
pub const SECOND_FACTOR: i32 = 0;
pub const FIRST_FACTOR: i32 = 1;

/// `session_challenge_key(user)`
pub fn session_challenge_key(user_id: Option<i32>) -> String {
    format!(
        "staged-webauthn-challenge-{}",
        user_id.map(|id| id.to_string()).unwrap_or_default()
    )
}

/// `stage_challenge(user, server_session)`: `SecureRandom.hex(30)`, kept
/// for five minutes.
pub async fn stage_challenge(
    conn: &mut PgConnection,
    session_id: &str,
    user_id: Option<i32>,
) -> Result<String, sqlx::Error> {
    use rand::RngCore;
    let mut bytes = [0u8; 30];
    rand::thread_rng().fill_bytes(&mut bytes);
    let challenge: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    crate::session::server::set(
        conn,
        session_id,
        &session_challenge_key(user_id),
        &Value::String(challenge.clone()),
        CHALLENGE_EXPIRY_SECONDS,
    )
    .await?;
    Ok(challenge)
}

/// `challenge(user, server_session)`
pub async fn challenge(
    conn: &mut PgConnection,
    session_id: &str,
    user_id: Option<i32>,
) -> Result<Option<String>, sqlx::Error> {
    let stored =
        crate::session::server::get(conn, session_id, &session_challenge_key(user_id)).await?;
    Ok(match stored {
        Some(Value::String(s)) => Some(s),
        _ => None,
    })
}

/// The enabled credential ids of a factor type, oldest first (pluck's
/// order).
async fn credential_ids(
    conn: &mut PgConnection,
    user_id: i32,
    factor_type: i32,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT credential_id FROM user_security_keys WHERE user_id = $1 AND enabled \
         AND factor_type = $2 ORDER BY id",
    )
    .bind(user_id)
    .bind(factor_type)
    .fetch_all(conn)
    .await
}

/// `allowed_credentials(user, server_session, include_passkeys:)`: the
/// security keys' ids (and the passkeys' when asked), with the staged
/// challenge; empty without either.
pub async fn allowed_credentials(
    conn: &mut PgConnection,
    session_id: &str,
    user_id: i32,
    enabled: &Enabled,
    include_passkeys: bool,
) -> Result<Map<String, Value>, sqlx::Error> {
    let passkeys = include_passkeys && enabled.passkeys;
    let mut out = Map::new();
    if !enabled.security_keys && !passkeys {
        return Ok(out);
    }
    if enabled.security_keys {
        out.insert(
            "allowed_credential_ids".into(),
            json!(credential_ids(conn, user_id, SECOND_FACTOR).await?),
        );
    }
    if passkeys {
        out.insert(
            "passkey_allowed_credential_ids".into(),
            json!(credential_ids(conn, user_id, FIRST_FACTOR).await?),
        );
    }
    out.insert(
        "challenge".into(),
        json!(challenge(conn, session_id, Some(user_id)).await?),
    );
    Ok(out)
}

/// `clear_challenge(user, server_session)`
pub async fn clear_challenge(
    conn: &mut PgConnection,
    session_id: &str,
    user_id: Option<i32>,
) -> Result<(), sqlx::Error> {
    crate::session::server::delete(conn, session_id, &session_challenge_key(user_id)).await
}

/// `SUPPORTED_ALGORITHMS`
pub const SUPPORTED_ALGORITHMS: [i64; 8] = [
    cose::ES256,
    cose::EDDSA,
    cose::ES384,
    cose::ES512,
    cose::PS256,
    cose::PS384,
    cose::PS512,
    cose::RS256,
];

/// `DiscourseWebauthn::SecurityKeyError` and its subclasses, by the
/// `webauthn.validation.*` message each raises with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityKeyError(pub &'static str);

impl SecurityKeyError {
    pub fn message(&self, i18n: &I18n) -> String {
        let key = format!("webauthn.validation.{}", self.0);
        i18n.t(&key).map(str::to_string).unwrap_or(key)
    }
}

const INVALID_TYPE: SecurityKeyError = SecurityKeyError("invalid_type_error");
const CHALLENGE_MISMATCH: SecurityKeyError = SecurityKeyError("challenge_mismatch_error");
const INVALID_ORIGIN: SecurityKeyError = SecurityKeyError("invalid_origin_error");
const INVALID_RP_ID: SecurityKeyError = SecurityKeyError("invalid_relying_party_id_error");
const USER_PRESENCE: SecurityKeyError = SecurityKeyError("user_presence_error");
const USER_VERIFICATION: SecurityKeyError = SecurityKeyError("user_verification_error");
const UNSUPPORTED_ALGORITHM: SecurityKeyError =
    SecurityKeyError("unsupported_public_key_algorithm_error");
const UNSUPPORTED_ATTESTATION: SecurityKeyError =
    SecurityKeyError("unsupported_attestation_format_error");
const CREDENTIAL_IN_USE: SecurityKeyError = SecurityKeyError("credential_id_in_use_error");
const MALFORMED_ATTESTATION: SecurityKeyError = SecurityKeyError("malformed_attestation_error");
const NOT_FOUND: SecurityKeyError = SecurityKeyError("not_found_error");
const MALFORMED_CREDENTIAL: SecurityKeyError =
    SecurityKeyError("malformed_public_key_credential_error");
const OWNERSHIP: SecurityKeyError = SecurityKeyError("ownership_error");
const PUBLIC_KEY: SecurityKeyError = SecurityKeyError("public_key_error");
const UNKNOWN_ALGORITHM: SecurityKeyError = SecurityKeyError("unknown_cose_algorithm_error");

/// The relying party: `rp_id`, `origin` and `rp_name`.
pub struct RelyingParty {
    pub id: String,
    pub origin: String,
    pub name: String,
}

impl RelyingParty {
    /// `rp_id` is the hostname in production, else localhost; `origin` is
    /// http://localhost:3000 in development, else base_url_no_prefix.
    pub fn new(config: &Config, settings: &SiteSettings) -> Result<Self, AppError> {
        let urls = crate::url::Urls { config, settings };
        Ok(Self {
            id: if config.rails_env == RailsEnv::Production {
                urls.current_hostname()?
            } else {
                "localhost".into()
            },
            origin: if config.rails_env == RailsEnv::Development {
                "http://localhost:3000".into()
            } else {
                urls.base_url_no_prefix()?
            },
            name: settings.get("title")?.to_s(),
        })
    }
}

/// `params[key]` as a string.
fn param(params: &Map<String, Value>, key: &str) -> Option<String> {
    crate::params::string(params, key)
}

/// BaseValidationService's client data and the checks over it.
struct ClientData {
    json: Vec<u8>,
    parsed: Value,
}

impl ClientData {
    fn read(params: &Map<String, Value>) -> Result<Self, AppError> {
        let json = decode64(&param(params, "clientData").unwrap_or_default());
        let parsed = serde_json::from_slice(&json)
            .map_err(|_| Unsupported("webauthn client data that is not JSON"))?;
        Ok(Self { json, parsed })
    }

    fn validate_type(&self, want: &str) -> Result<(), SecurityKeyError> {
        if self.parsed.get("type").and_then(Value::as_str) == Some(want) {
            Ok(())
        } else {
            Err(INVALID_TYPE)
        }
    }

    /// `Base64.decode64(client_data["challenge"])` against the staged
    /// challenge (the base64url the browser sends read leniently, as Ruby
    /// reads it).
    async fn validate_challenge(
        &self,
        conn: &mut PgConnection,
        session_id: &str,
        user_id: Option<i32>,
    ) -> Result<Result<(), SecurityKeyError>, AppError> {
        let sent = self
            .parsed
            .get("challenge")
            .and_then(Value::as_str)
            .map(decode64);
        let staged = challenge(conn, session_id, user_id).await?;
        Ok(match (sent, staged) {
            (Some(sent), Some(staged)) if sent == staged.as_bytes() => Ok(()),
            _ => Err(CHALLENGE_MISMATCH),
        })
    }

    fn validate_origin(&self, rp: &RelyingParty) -> Result<(), SecurityKeyError> {
        if self.parsed.get("origin").and_then(Value::as_str) == Some(rp.origin.as_str()) {
            Ok(())
        } else {
            Err(INVALID_ORIGIN)
        }
    }

    fn hash(&self) -> [u8; 32] {
        use sha2::Digest;
        sha2::Sha256::digest(&self.json).into()
    }
}

/// `validate_rp_id_hash`, `validate_user_presence` and (for first factor
/// keys) `validate_user_verification` over authenticator data.
fn validate_auth_data(
    auth_data: &[u8],
    rp: &RelyingParty,
    user_verification: bool,
) -> Result<Result<(), SecurityKeyError>, AppError> {
    use sha2::Digest;
    let rp_hash = sha2::Sha256::digest(rp.id.as_bytes());
    if auth_data.get(..32) != Some(&rp_hash[..]) {
        return Ok(Err(INVALID_RP_ID));
    }
    let flags = *auth_data
        .get(32)
        .ok_or(Unsupported("webauthn authenticator data without flags"))?;
    if flags & 0x01 == 0 {
        return Ok(Err(USER_PRESENCE));
    }
    if user_verification && flags & 0x04 == 0 {
        return Ok(Err(USER_VERIFICATION));
    }
    Ok(Ok(()))
}

/// The security key an assertion went through with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedKey {
    pub id: i64,
    pub user_id: i32,
    pub factor_type: i32,
}

/// `AuthenticationService#authenticate_security_key`: the assertion in
/// `credential` (signature, clientData, authenticatorData, credentialId
/// and, without a current user, userHandle) checked against the staged
/// challenge and the stored key; the key's last_used written and the
/// challenge cleared once it goes through.
pub async fn authenticate(
    conn: &mut PgConnection,
    rp: &RelyingParty,
    session_id: &str,
    current_user: Option<i32>,
    credential: Option<&Value>,
    factor_types: &[i32],
) -> Result<Result<AuthenticatedKey, SecurityKeyError>, AppError> {
    let Some(params) = credential
        .and_then(Value::as_object)
        .filter(|m| !m.is_empty())
    else {
        return Ok(Err(MALFORMED_CREDENTIAL));
    };
    // 6. The credential and its owner.
    let key: Option<(i64, i64, i32, String)> = sqlx::query_as(
        "SELECT id, user_id, factor_type, public_key FROM user_security_keys \
         WHERE credential_id = $1 AND enabled ORDER BY id LIMIT 1",
    )
    .bind(param(params, "credentialId"))
    .fetch_optional(&mut *conn)
    .await?;
    let Some((key_id, key_user_id, factor_type, public_key)) = key else {
        return Ok(Err(NOT_FOUND));
    };
    if !factor_types.contains(&factor_type) {
        return Ok(Err(OWNERSHIP));
    }
    let key_user_id = i32::try_from(key_user_id).map_err(|_| Unsupported("a user id past i32"))?;
    match current_user {
        Some(user_id) => {
            if key_user_id != user_id {
                return Ok(Err(OWNERSHIP));
            }
        }
        None => {
            let secure_identifier: Option<String> =
                sqlx::query_scalar("SELECT secure_identifier FROM users WHERE id = $1")
                    .bind(key_user_id)
                    .fetch_optional(&mut *conn)
                    .await?
                    .flatten();
            let handle = param(params, "userHandle").filter(|h| !h.trim().is_empty());
            let matches = handle
                .is_some_and(|h| secure_identifier.is_some_and(|id| decode64(&h) == id.as_bytes()));
            if !matches {
                return Ok(Err(OWNERSHIP));
            }
        }
    }
    // 9-14
    let client_data = ClientData::read(params)?;
    if let Err(e) = client_data.validate_type("webauthn.get") {
        return Ok(Err(e));
    }
    if let Err(e) = client_data
        .validate_challenge(&mut *conn, session_id, current_user)
        .await?
    {
        return Ok(Err(e));
    }
    if let Err(e) = client_data.validate_origin(rp) {
        return Ok(Err(e));
    }
    // 16-18
    let auth_data = decode64(&param(params, "authenticatorData").unwrap_or_default());
    if let Err(e) = validate_auth_data(&auth_data, rp, factor_type == FIRST_FACTOR)? {
        return Ok(Err(e));
    }
    // 22-23
    let mut signed = auth_data;
    signed.extend_from_slice(&client_data.hash());
    let signature = decode64(&param(params, "signature").unwrap_or_default());
    match cose::Key::deserialize(&decode64(&public_key)) {
        Err(_) => return Ok(Err(PUBLIC_KEY)),
        Ok(key) => match key.verify(&signature, &signed) {
            Ok(()) => {}
            Err(cose::Error::UnknownAlgorithm) => return Ok(Err(UNKNOWN_ALGORITHM)),
            Err(cose::Error::Unported(what)) => return Err(Unsupported(what).into()),
            Err(_) => return Ok(Err(PUBLIC_KEY)),
        },
    }
    // 26
    sqlx::query(
        "UPDATE user_security_keys SET last_used = clock_timestamp(), \
         updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(key_id)
    .execute(&mut *conn)
    .await?;
    clear_challenge(conn, session_id, current_user).await?;
    Ok(Ok(AuthenticatedKey {
        id: key_id,
        user_id: key_user_id,
        factor_type,
    }))
}

/// A registered key: the row's id and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredKey {
    pub id: i64,
    pub name: String,
}

/// `RegistrationService#register_security_key`: the attestation (format
/// "none" only) checked against the staged challenge, and the credential
/// saved for the user with its public key as the attested COSE bytes.
pub async fn register(
    conn: &mut PgConnection,
    rp: &RelyingParty,
    session_id: &str,
    user_id: i32,
    params: &Map<String, Value>,
    factor_type: i32,
) -> Result<Result<RegisteredKey, SecurityKeyError>, AppError> {
    let client_data = ClientData::read(params)?;
    if let Err(e) = client_data.validate_type("webauthn.create") {
        return Ok(Err(e));
    }
    if let Err(e) = client_data
        .validate_challenge(&mut *conn, session_id, Some(user_id))
        .await?
    {
        return Ok(Err(e));
    }
    if let Err(e) = client_data.validate_origin(rp) {
        return Ok(Err(e));
    }
    // 9. The attestation object.
    let attestation = decode64(&param(params, "attestation").unwrap_or_default());
    let Ok(attestation) = ciborium::de::from_reader::<ciborium::Value, _>(&attestation[..]) else {
        return Ok(Err(MALFORMED_ATTESTATION));
    };
    let field = |name: &str| {
        attestation.as_map().and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_text() == Some(name))
                .map(|(_, v)| v)
        })
    };
    let Some(ciborium::Value::Bytes(auth_data)) = field("authData") else {
        return Err(Unsupported("an attestation without its authData bytes").into());
    };
    if let Err(e) = validate_auth_data(auth_data, rp, factor_type == FIRST_FACTOR)? {
        return Ok(Err(e));
    }
    // 13. The attested credential: its id and the public key's CBOR,
    // written back out as the cbor gem encodes what it read.
    let Some((credential_id, public_key_bytes)) = attested_credential(auth_data) else {
        return Ok(Err(MALFORMED_ATTESTATION));
    };
    let Ok(public_key) = cose::Key::deserialize(&public_key_bytes) else {
        return Ok(Err(MALFORMED_ATTESTATION));
    };
    if !public_key
        .alg()
        .is_some_and(|alg| SUPPORTED_ALGORITHMS.contains(&alg))
    {
        return Ok(Err(UNSUPPORTED_ALGORITHM));
    }
    // 15-16
    if field("fmt").and_then(|f| f.as_text()) != Some("none") {
        return Ok(Err(UNSUPPORTED_ATTESTATION));
    }
    // 19
    use base64::Engine;
    let encoded_id = base64::engine::general_purpose::STANDARD.encode(&credential_id);
    let encoded_key = base64::engine::general_purpose::STANDARD.encode(&public_key_bytes);
    let in_use: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_security_keys WHERE credential_id = $1)",
    )
    .bind(&encoded_id)
    .fetch_one(&mut *conn)
    .await?;
    if in_use {
        return Ok(Err(CREDENTIAL_IN_USE));
    }
    // 20. UserSecurityKey.create!, with its validations.
    let name = param(params, "name").unwrap_or_default();
    if name.chars().count() > 300 {
        return Err(Unsupported("a security key name over 300 characters (RecordInvalid)").into());
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM user_security_keys WHERE user_id = $1")
            .bind(i64::from(user_id))
            .fetch_one(&mut *conn)
            .await?;
    if count >= 50 {
        return Err(Unsupported("a 51st security key (RecordInvalid)").into());
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO user_security_keys (user_id, credential_id, public_key, name, factor_type, \
         created_at, updated_at) VALUES ($1, $2, $3, $4, $5, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(i64::from(user_id))
    .bind(&encoded_id)
    .bind(&encoded_key)
    .bind(&name)
    .bind(factor_type)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Ok(RegisteredKey { id, name }))
}

/// `extract_public_key_and_credential_from_attestation`: after the rp id
/// hash, flags and counter (37 bytes) and the aaguid (16), a two byte
/// length, the credential id, then one CBOR item, the public key.
fn attested_credential(auth_data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let data = auth_data.get(37..)?;
    let len = u16::from_be_bytes([*data.get(16)?, *data.get(17)?]) as usize;
    let credential_id = data.get(18..18 + len)?.to_vec();
    let mut rest = data.get(18 + len..)?;
    let key: ciborium::Value = ciborium::de::from_reader(&mut rest).ok()?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&key, &mut bytes).ok()?;
    Some((credential_id, bytes))
}

/// `create_or_fetch_secure_identifier`: `SecureRandom.hex(20)`, saved the
/// first time.
pub async fn secure_identifier(conn: &mut PgConnection, user_id: i32) -> Result<String, AppError> {
    let existing: Option<String> =
        sqlx::query_scalar("SELECT secure_identifier FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if let Some(id) = existing.filter(|s| !s.is_empty()) {
        return Ok(id);
    }
    use rand::RngCore;
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    sqlx::query(
        "UPDATE users SET secure_identifier = $2, updated_at = clock_timestamp() WHERE id = $1",
    )
    .bind(user_id)
    .bind(&id)
    .execute(conn)
    .await?;
    Ok(id)
}
