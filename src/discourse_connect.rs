//! DiscourseConnect, the consumer side: DiscourseConnectBase's signed
//! payloads (lib/discourse_connect_base.rb) and DiscourseConnect
//! (app/models/discourse_connect.rb): the nonce a login starts with,
//! kept in the server session (or Discourse.cache without
//! discourse_connect_csrf_protection), and `lookup_or_create_user`, which
//! finds or creates the user the provider vouches for and brings their
//! record, profile, avatar jobs and groups in line with the payload.
//!
//! Refused: unstaging a staged user, an SSO record whose user is gone
//! while another record holds the external id, repeated payload keys,
//! user validation errors other than the email's.

use hmac::Mac;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `DiscourseConnectBase::ACCESSORS`, in their order (the payload's).
pub const ACCESSORS: [&str; 29] = [
    "add_groups",
    "admin",
    "avatar_force_update",
    "avatar_url",
    "bio",
    "card_background_url",
    "confirmed_2fa",
    "email",
    "external_id",
    "failed",
    "groups",
    "locale",
    "locale_force_update",
    "location",
    "logout",
    "moderator",
    "name",
    "no_2fa_methods",
    "nonce",
    "prompt",
    "profile_background_url",
    "remove_groups",
    "require_2fa",
    "require_activation",
    "return_sso_url",
    "suppress_welcome_message",
    "title",
    "username",
    "website",
];

/// `DiscourseConnectBase::BOOLS`
const BOOLS: [&str; 11] = [
    "admin",
    "avatar_force_update",
    "confirmed_2fa",
    "failed",
    "locale_force_update",
    "logout",
    "moderator",
    "no_2fa_methods",
    "require_2fa",
    "require_activation",
    "suppress_welcome_message",
];

/// `DiscourseConnectBase.nonce_expiry_time`
pub const NONCE_EXPIRY_SECONDS: i64 = 30 * 60;
/// `DiscourseConnectBase.used_nonce_expiry_time`
const USED_NONCE_EXPIRY_SECONDS: i64 = 24 * 3600;

/// `DiscourseConnect::BANNED_EXTERNAL_IDS`
const BANNED_EXTERNAL_IDS: [&str; 4] = ["none", "nil", "blank", "null"];

const SYSTEM_USER_ID: i32 = -1;

/// An accessor's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    Str(String),
    Bool(bool),
}

/// A DiscourseConnect payload: the accessors that are set, and the
/// `custom.*` fields.
#[derive(Debug, Clone, Default)]
pub struct Sso {
    values: Vec<(&'static str, Field)>,
    pub custom_fields: Vec<(String, String)>,
}

impl Sso {
    /// A string accessor (`nil` when unset).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| match v {
                Field::Str(s) => Some(s.as_str()),
                Field::Bool(_) => None,
            })
    }

    /// A string accessor that is present (`.present?`).
    pub fn present(&self, key: &str) -> Option<&str> {
        self.get(key).filter(|v| !v.trim().is_empty())
    }

    /// A boolean accessor (`nil` unless "true" or "false" was sent).
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| match v {
                Field::Bool(b) => Some(*b),
                Field::Str(_) => None,
            })
    }

    /// An accessor set to nil.
    pub fn unset(&mut self, key: &str) {
        self.values.retain(|(k, _)| *k != key);
    }

    pub fn set(&mut self, key: &'static str, value: Field) {
        if let Some(slot) = self.values.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.values.push((key, value));
        }
    }

    /// `to_h`: the accessors that are set in ACCESSORS order, then the
    /// custom fields.
    pub fn pairs(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for key in ACCESSORS {
            if let Some((_, v)) = self.values.iter().find(|(k, _)| *k == key) {
                let v = match v {
                    Field::Str(s) => s.clone(),
                    Field::Bool(b) => b.to_string(),
                };
                out.push((key.to_string(), v));
            }
        }
        for (k, v) in &self.custom_fields {
            out.push((format!("custom.{k}"), v.clone()));
        }
        out
    }

    /// `unsigned_payload`: `Rack::Utils.build_query(to_h)`.
    pub fn unsigned_payload(&self) -> String {
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(self.pairs())
            .finish()
    }

    /// `payload(secret)`: the Base64 payload, escaped, and its signature.
    pub fn payload(&self, secret: &str) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::STANDARD.encode(self.unsigned_payload());
        let escaped: String = form_urlencoded::byte_serialize(payload.as_bytes()).collect();
        format!("sso={escaped}&sig={}", sign(&payload, secret))
    }

    /// `to_url(base_url)`
    pub fn to_url(&self, base: &str, secret: &str) -> String {
        let sep = if base.contains('?') { "&" } else { "?" };
        format!("{base}{sep}{}", self.payload(secret))
    }

    /// `diagnostics`, for the verbose log.
    pub fn diagnostics(&self) -> String {
        ACCESSORS
            .iter()
            .map(|k| {
                let v = match self.values.iter().find(|(key, _)| key == k) {
                    Some((_, Field::Str(s))) => s.clone(),
                    Some((_, Field::Bool(b))) => b.to_string(),
                    None => String::new(),
                };
                format!("{k}: {v}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `OpenSSL::HMAC.hexdigest("sha256", secret, payload)`
pub fn sign(payload: &str, secret: &str) -> String {
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC takes a key of any size");
    mac.update(payload.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why a payload was not taken (DiscourseConnectBase's parse errors).
#[derive(Debug, PartialEq)]
pub enum ParseError {
    /// PayloadParseError: `sso` is not Base64.
    Payload,
    /// SignatureError
    Signature,
    /// BlankSecretError
    BlankSecret,
}

/// `Rack::Utils.parse_query`: decoded pairs, a key sent twice refused (it
/// would be an array there).
fn parse_query(query: &[u8]) -> Result<Vec<(String, String)>, AppError> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in form_urlencoded::parse(query) {
        if out.iter().any(|(key, _)| *key == k) {
            return Err(Unsupported("repeated keys in a DiscourseConnect payload").into());
        }
        out.push((k.into_owned(), v.into_owned()));
    }
    Ok(out)
}

/// Ruby's `Base64.decode64` (`unpack1("m")`): characters outside the
/// alphabet skipped, decoding up to the padding, a lone trailing
/// character dropped.
fn decode64(s: &str) -> Vec<u8> {
    use base64::Engine;
    let mut clean: String = s
        .chars()
        .take_while(|c| *c != '=')
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/')
        .collect();
    if clean.len() % 4 == 1 {
        clean.pop();
    }
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine.decode(clean).unwrap_or_default()
}

/// `DiscourseConnectBase.parse(payload, secret)` over a query string.
pub fn parse(query: &str, secret: &str) -> Result<Result<Sso, ParseError>, AppError> {
    if secret.trim().is_empty() {
        return Ok(Err(ParseError::BlankSecret));
    }
    let parsed = parse_query(query.as_bytes())?;
    let get = |k: &str| {
        parsed
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    let sso = get("sso").unwrap_or("");
    if sso
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || "=\r\n/+".contains(c)))
    {
        return Ok(Err(ParseError::Payload));
    }
    let decoded = decode64(sso);
    let decoded = parse_query(&decoded)?;
    let expected = sign(sso, secret);
    let sig = get("sig").unwrap_or("");
    if !bool::from(subtle::ConstantTimeEq::ct_eq(
        expected.as_bytes(),
        sig.as_bytes(),
    )) {
        return Ok(Err(ParseError::Signature));
    }
    let mut out = Sso::default();
    for key in ACCESSORS {
        let Some((_, value)) = decoded.iter().find(|(k, _)| k == key) else {
            continue;
        };
        if BOOLS.contains(&key) {
            match value.as_str() {
                "true" => out.set(key, Field::Bool(true)),
                "false" => out.set(key, Field::Bool(false)),
                _ => {}
            }
        } else {
            out.set(key, Field::Str(value.clone()));
        }
    }
    for (k, v) in &decoded {
        if let Some(field) = k.strip_prefix("custom.").filter(|f| !f.is_empty()) {
            out.custom_fields.push((field.to_string(), v.clone()));
        }
    }
    Ok(Ok(out))
}

/// Where a nonce lives: the browser's server session with
/// discourse_connect_csrf_protection, else Discourse.cache.
pub struct NonceStore<'a> {
    pub csrf_protection: bool,
    pub session_id: &'a str,
}

fn nonce_key(nonce: &str) -> String {
    format!("SSO_NONCE_{nonce}")
}

fn used_nonce_key(nonce: &str) -> String {
    format!("USED_SSO_NONCE_{nonce}")
}

/// Discourse.cache's key for a value (its namespace in
/// discourse_rs.cached_values).
fn cache_key(key: &str) -> String {
    format!("_CACHE:{key}")
}

pub async fn cache_read(conn: &mut PgConnection, key: &str) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT value FROM discourse_rs.cached_values WHERE key = $1 AND expires_at > clock_timestamp()",
    )
    .bind(cache_key(key))
    .fetch_optional(&mut *conn)
    .await?)
}

pub async fn cache_write(
    conn: &mut PgConnection,
    key: &str,
    value: &str,
    expires_seconds: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO discourse_rs.cached_values (key, value, expires_at) \
         VALUES ($1, $2, clock_timestamp() + make_interval(secs => $3)) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, expires_at = EXCLUDED.expires_at",
    )
    .bind(cache_key(key))
    .bind(value)
    .bind(expires_seconds as f64)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub async fn cache_delete(conn: &mut PgConnection, key: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM discourse_rs.cached_values WHERE key = $1")
        .bind(cache_key(key))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

impl NonceStore<'_> {
    async fn read(&self, conn: &mut PgConnection, nonce: &str) -> Result<Option<String>, AppError> {
        if self.csrf_protection {
            let value =
                crate::session::server::get(&mut *conn, self.session_id, &nonce_key(nonce)).await?;
            Ok(value.map(|v| match v {
                Value::String(s) => s,
                other => other.to_string(),
            }))
        } else {
            cache_read(conn, &nonce_key(nonce)).await
        }
    }

    /// `register_nonce(return_path)`
    pub async fn register(
        &self,
        conn: &mut PgConnection,
        nonce: &str,
        return_path: &str,
    ) -> Result<(), AppError> {
        if self.csrf_protection {
            crate::session::server::set(
                &mut *conn,
                self.session_id,
                &nonce_key(nonce),
                &json!(return_path),
                NONCE_EXPIRY_SECONDS,
            )
            .await?;
        } else {
            cache_write(conn, &nonce_key(nonce), return_path, NONCE_EXPIRY_SECONDS).await?;
        }
        Ok(())
    }

    /// `nonce_valid?`
    pub async fn valid(
        &self,
        conn: &mut PgConnection,
        nonce: Option<&str>,
    ) -> Result<bool, AppError> {
        let Some(nonce) = nonce else {
            return Ok(false);
        };
        Ok(self
            .read(conn, nonce)
            .await?
            .is_some_and(|v| !v.trim().is_empty()))
    }

    /// `nonce_error`, for the verbose log.
    pub async fn error(
        &self,
        conn: &mut PgConnection,
        nonce: Option<&str>,
    ) -> Result<&'static str, AppError> {
        if let Some(nonce) = nonce
            && cache_read(conn, &used_nonce_key(nonce))
                .await?
                .is_some_and(|v| !v.is_empty())
        {
            return Ok("Nonce has already been used");
        }
        Ok(if self.csrf_protection {
            "Nonce is incorrect, was generated in a different browser session, or has expired"
        } else {
            "Nonce is incorrect, or has expired"
        })
    }

    /// `return_path`
    pub async fn return_path(
        &self,
        conn: &mut PgConnection,
        nonce: Option<&str>,
    ) -> Result<String, AppError> {
        Ok(match nonce {
            Some(nonce) => self.read(conn, nonce).await?,
            None => None,
        }
        .unwrap_or_else(|| "/".to_string()))
    }

    /// `expire_nonce!`
    pub async fn expire(
        &self,
        conn: &mut PgConnection,
        nonce: Option<&str>,
    ) -> Result<(), AppError> {
        let Some(nonce) = nonce else {
            return Ok(());
        };
        if self.csrf_protection {
            crate::session::server::delete(&mut *conn, self.session_id, &nonce_key(nonce)).await?;
        } else {
            cache_delete(conn, &nonce_key(nonce)).await?;
        }
        let path = self.return_path(conn, Some(nonce)).await?;
        cache_write(
            conn,
            &used_nonce_key(nonce),
            &path,
            USED_NONCE_EXPIRY_SECONDS,
        )
        .await
    }
}

/// `DiscourseConnect.generate_sso(return_path)`: a fresh nonce
/// registered with where to go after, and the url to come back to.
pub async fn generate_sso(
    conn: &mut PgConnection,
    store: &NonceStore<'_>,
    base_url: &str,
    return_path: &str,
) -> Result<Sso, AppError> {
    let nonce: String = (0..16)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    store.register(conn, &nonce, return_path).await?;
    let mut sso = Sso::default();
    sso.set("nonce", Field::Str(nonce));
    sso.set(
        "return_sso_url",
        Field::Str(format!("{base_url}/session/sso_login")),
    );
    Ok(sso)
}

/// What `lookup_or_create_user` could not do, as sso_login answers it.
#[derive(Debug, PartialEq)]
pub enum LookupError {
    /// BlankExternalId
    BlankExternalId,
    /// A RecordInvalid on the user's primary email: blank (`None`) or
    /// refused.
    Email(Option<String>),
    /// BannedExternalId (sso_login answers it as an unknown error).
    Banned,
    /// SiteArchived, which sso_login rescues as an unknown error.
    Unknown,
}

/// What lookup_or_create_user needs besides the connection.
pub struct Ctx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a crate::i18n::I18n,
    pub state: &'a crate::AppState,
}

#[derive(sqlx::FromRow)]
struct Record {
    id: i32,
    user_id: i32,
    external_avatar_url: Option<String>,
    external_profile_background_url: Option<String>,
    external_card_background_url: Option<String>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: i32,
    username: String,
    name: Option<String>,
    active: bool,
    staged: bool,
    admin: bool,
    moderator: bool,
    title: Option<String>,
    uploaded_avatar_id: Option<i32>,
    email: Option<String>,
}

async fn load_user(conn: &mut PgConnection, id: i32) -> Result<Option<UserRow>, AppError> {
    Ok(sqlx::query_as(
        "SELECT id, username, name, active, staged, admin, moderator, title, uploaded_avatar_id, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) AS email \
         FROM users u WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?)
}

async fn load_record(
    conn: &mut PgConnection,
    sql: &str,
    bind: Value,
) -> Result<Option<Record>, AppError> {
    let q = sqlx::query_as::<_, Record>(sql);
    let q = match bind {
        Value::String(s) => q.bind(s),
        Value::Number(n) => q.bind(n.as_i64().unwrap_or(0) as i32),
        _ => q,
    };
    Ok(q.fetch_optional(&mut *conn).await?)
}

const RECORD_COLUMNS: &str = "id, user_id, external_avatar_url, \
     external_profile_background_url, external_card_background_url";

/// The SSO record's attributes as lookup_or_create_user leaves them, saved
/// once at the end (`sso_record.save!`).
struct RecordChanges {
    last_payload: String,
    external_id: String,
}

/// `lookup_or_create_user(ip_address)`: the user's id.
pub async fn lookup_or_create_user(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    sso: &Sso,
    ip: Option<&str>,
) -> Result<Result<i32, LookupError>, AppError> {
    // `self.external_id.to_s`: a blank one fails.
    let external_id = sso.get("external_id").unwrap_or("").to_string();
    if external_id.trim().is_empty() {
        return Ok(Err(LookupError::BlankExternalId));
    }
    if BANNED_EXTERNAL_IDS.contains(&external_id.to_lowercase().as_str()) {
        return Ok(Err(LookupError::Banned));
    }
    // DistributedMutex "sso_lookup_or_create_user_#{external_id}", for
    // the request's transaction.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("sso_lookup_or_create_user_{external_id}"))
        .execute(&mut *conn)
        .await?;
    let s = cx.settings;
    let payloads = crate::web_hooks::Payloads {
        settings: s,
        i18n: cx.i18n,
        config: &cx.state.config,
    };
    let unsigned = sso.unsigned_payload();

    let found = load_record(
        conn,
        &format!("SELECT {RECORD_COLUMNS} FROM single_sign_on_records WHERE external_id = $1"),
        json!(external_id),
    )
    .await?;
    let found_user = match &found {
        Some(r) => load_user(conn, r.user_id).await?.map(|u| (r.id, u)),
        None => None,
    };
    let (record, user_id) = match found_user {
        Some((_, user)) => (found.expect("a record"), user.id),
        None => {
            if found.is_some() {
                return Err(Unsupported("a DiscourseConnect record whose user is gone").into());
            }
            let user_id = match match_email_or_create_user(conn, cx, &payloads, sso, ip).await? {
                Ok(id) => id,
                Err(e) => return Ok(Err(e)),
            };
            let record = load_record(
                conn,
                &format!("SELECT {RECORD_COLUMNS} FROM single_sign_on_records WHERE user_id = $1"),
                json!(user_id),
            )
            .await?
            .ok_or(Unsupported("a DiscourseConnect user without a record"))?;
            (record, user_id)
        }
    };
    let mut changes = RecordChanges {
        last_payload: unsigned.clone(),
        external_id: external_id.clone(),
    };

    let mut user = load_user(conn, user_id)
        .await?
        .ok_or(Unsupported("a DiscourseConnect user that vanished"))?;
    // user.unstage!
    if user.staged {
        return Err(Unsupported("unstaging a user through DiscourseConnect").into());
    }

    // change_external_attributes_and_override
    let mut email_changed = false;
    let require_activation = sso.flag("require_activation").unwrap_or(false);
    let mut new_email: Option<String> = None;
    let mut set_active: Option<bool> = None;
    if s.get("auth_overrides_email")?.truthy()
        && let Some(email) = sso.present("email")
        && user.email.as_deref() != Some(email.to_lowercase().as_str())
    {
        new_email = Some(email.to_lowercase());
        if require_activation {
            set_active = Some(false);
        }
        email_changed = true;
    }
    if s.get("auth_overrides_username")?.truthy()
        && let Some(username) = sso.present("username")
    {
        override_username(conn, cx, &user, username).await?;
        user = load_user(conn, user_id)
            .await?
            .ok_or(Unsupported("a DiscourseConnect user that vanished"))?;
    }
    let mut new_name: Option<String> = None;
    if s.get("auth_overrides_name")?.truthy()
        && let Some(name) = sso.present("name")
        && user.name.as_deref() != Some(name)
    {
        new_name = Some(name.to_string());
    }
    let mut new_locale: Option<String> = None;
    if sso.flag("locale_force_update").unwrap_or(false)
        && s.get("allow_user_locale")?.truthy()
        && let Some(locale) = sso.present("locale")
        && valid_locale(locale)
    {
        new_locale = Some(locale.to_string());
    }
    let avatar_missing = match user.uploaded_avatar_id {
        None => true,
        Some(id) => {
            !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM uploads WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?
        }
    };
    let overrides_avatar = s.get("discourse_connect_overrides_avatar")?.truthy();
    let force_avatar = sso.flag("avatar_force_update").unwrap_or(false);
    if let Some(avatar_url) = sso.present("avatar_url")
        && (avatar_missing || force_avatar || overrides_avatar)
    {
        let changed = record.external_avatar_url.as_deref() != Some(avatar_url);
        if force_avatar || changed || avatar_missing {
            crate::jobs::enqueue(
                &mut *conn,
                "download_avatar_from_url",
                json!({"url": avatar_url, "user_id": user_id, "override_gravatar": overrides_avatar}),
            )
            .await?;
        }
    }
    for (key, card, overrides, previous) in [
        (
            "profile_background_url",
            false,
            "discourse_connect_overrides_profile_background",
            record.external_profile_background_url.as_deref(),
        ),
        (
            "card_background_url",
            true,
            "discourse_connect_overrides_card_background",
            record.external_card_background_url.as_deref(),
        ),
    ] {
        let Some(url) = sso.present(key) else {
            continue;
        };
        let column = if card {
            "card_background_upload_id"
        } else {
            "profile_background_upload_id"
        };
        let missing: bool = sqlx::query_scalar(&format!(
            "SELECT NOT EXISTS (SELECT 1 FROM user_profiles p JOIN uploads u ON u.id = p.{column} \
             WHERE p.user_id = $1)"
        ))
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if missing || s.get(overrides)?.truthy() {
            let changed = previous != Some(url);
            if changed || missing {
                crate::jobs::enqueue(
                    &mut *conn,
                    "download_profile_background_from_url",
                    json!({"url": url, "user_id": user_id, "is_card_background": card}),
                )
                .await?;
            }
        }
    }

    // A changed email is validated when the user is saved.
    if let Some(email) = &new_email
        && let Some(e) = email_error(conn, s, email, Some(user_id)).await?
    {
        return Ok(Err(LookupError::Email(e)));
    }
    let mut pending = Pending {
        email: new_email,
        name: new_name,
        locale: new_locale,
        active: set_active,
        ..Default::default()
    };
    // The record's user activated: saved with the changes so far, welcomed
    // and given their automatic groups.
    if !user.active && pending.active != Some(false) && !require_activation {
        pending.active = Some(true);
        save_user(conn, cx, &user, &pending).await?;
        pending = Pending::default();
        user = load_user(conn, user_id)
            .await?
            .ok_or(Unsupported("a DiscourseConnect user that vanished"))?;
        // enqueue_welcome_message("welcome_user")
        if !sso.flag("suppress_welcome_message").unwrap_or(false)
            && s.get("send_welcome_message")?.truthy()
        {
            crate::jobs::enqueue(
                &mut *conn,
                "send_system_message",
                json!({"user_id": user_id, "message_type": "welcome_user"}),
            )
            .await?;
        }
        set_automatic_groups(conn, cx, user_id).await?;
    }
    // The payload's custom fields, the ip address, admin, moderator and
    // title, then user.save!.
    pending.custom_fields = sso.custom_fields.clone();
    pending.ip = Some(ip.map(str::to_string));
    pending.admin = sso.flag("admin");
    pending.moderator = sso.flag("moderator");
    pending.title = sso.get("title").map(str::to_string);
    save_user(conn, cx, &user, &pending).await?;
    let admin = pending.admin;
    let moderator = pending.moderator;

    let user_now = load_user(conn, user_id)
        .await?
        .ok_or(Unsupported("a DiscourseConnect user that vanished"))?;
    if email_changed && user_now.active {
        set_automatic_groups(conn, cx, user_id).await?;
    }
    // create_reviewable
    if s.get("must_approve_users")?.truthy() || s.get("invite_only")?.truthy() {
        let approved: bool = sqlx::query_scalar("SELECT approved FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
        if !approved {
            crate::jobs::enqueue(
                &mut *conn,
                "create_user_reviewable",
                json!({"user_id": user_id}),
            )
            .await?;
        }
    }
    // bio, website, location
    if let Some(bio) = sso.get("bio") {
        let blank: bool = sqlx::query_scalar(
            "SELECT COALESCE(btrim(bio_raw), '') = '' FROM user_profiles WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if blank || s.get("discourse_connect_overrides_bio")?.truthy() {
            save_bio(conn, cx, user_id, bio).await?;
        }
    }
    if let Some(website) = sso.get("website") {
        sqlx::query("UPDATE user_profiles SET website = $2 WHERE user_id = $1 AND website IS DISTINCT FROM $2")
            .bind(user_id)
            .bind(website)
            .execute(&mut *conn)
            .await?;
    }
    if let Some(location) = sso.get("location") {
        sqlx::query("UPDATE user_profiles SET location = $2 WHERE user_id = $1 AND location IS DISTINCT FROM $2")
            .bind(user_id)
            .bind(location)
            .execute(&mut *conn)
            .await?;
    }
    // Group.refresh_automatic_groups!(:admins, :moderators, :staff)
    if admin.is_some() || moderator.is_some() {
        crate::roles::refresh_staff_groups(conn, cx.i18n).await?;
    }

    // sso_record.save!
    changes.external_id = external_id;
    sqlx::query(
        "UPDATE single_sign_on_records SET last_payload = $2, external_id = $3, external_username = $4, \
                external_email = $5, external_name = $6, external_avatar_url = $7, \
                external_profile_background_url = $8, external_card_background_url = $9, \
                updated_at = clock_timestamp() \
         WHERE id = $1 AND (last_payload, external_id, external_username, external_email, external_name, \
                            external_avatar_url, external_profile_background_url, external_card_background_url) \
               IS DISTINCT FROM ($2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(record.id)
    .bind(&changes.last_payload)
    .bind(&changes.external_id)
    .bind(sso.get("username"))
    .bind(sso.get("email"))
    .bind(sso.get("name"))
    .bind(sso.get("avatar_url"))
    .bind(sso.get("profile_background_url"))
    .bind(sso.get("card_background_url"))
    .execute(&mut *conn)
    .await?;

    apply_group_rules(conn, cx, sso, user_id).await?;
    Ok(Ok(user_id))
}

/// `LocaleSiteSetting.valid_value?`
fn valid_locale(locale: &str) -> bool {
    crate::themes::locale_available(locale)
}

/// `user_profile.bio_raw = bio; user_profile.save!`: cooked before the
/// save, the hotlinked images pulled after the grace period.
async fn save_bio(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user_id: i32,
    bio: &str,
) -> Result<(), AppError> {
    let s = cx.settings;
    let before: Option<String> =
        sqlx::query_scalar("SELECT bio_raw FROM user_profiles WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if before.as_deref() == Some(bio) {
        return Ok(());
    }
    if bio.contains("upload://") || bio.contains("/uploads/") {
        return Err(Unsupported("uploads in the bio (UploadReference)").into());
    }
    let cooked = if bio.trim().is_empty() {
        None
    } else {
        let trust_level: i32 = sqlx::query_scalar("SELECT trust_level FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
        let opts = crate::pretty_text::MarkdownOptions {
            omit_nofollow: trust_level >= 3 && !s.get("tl3_links_no_follow")?.truthy(),
            ..Default::default()
        };
        let host = crate::pretty_text::Host::from_state(cx.state);
        Some(crate::pretty_text::cook(&host, bio, &opts).await?)
    };
    sqlx::query(
        "UPDATE user_profiles SET bio_raw = $2, bio_cooked = $3, \
                bio_cooked_version = CASE WHEN $3 IS NULL THEN bio_cooked_version ELSE 1 END \
         WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(bio)
    .bind(&cooked)
    .execute(&mut *conn)
    .await?;
    crate::jobs::enqueue_in(
        &mut *conn,
        s.get("editing_grace_period")?.to_i(),
        "pull_user_profile_hotlinked_images",
        json!({ "user_id": user_id }),
    )
    .await?;
    Ok(())
}

/// The attributes lookup_or_create_user set on the user before a save!.
#[derive(Default)]
struct Pending {
    email: Option<String>,
    name: Option<String>,
    locale: Option<String>,
    active: Option<bool>,
    /// Set when given (None: not touched), to null too.
    ip: Option<Option<String>>,
    custom_fields: Vec<(String, String)>,
    admin: Option<bool>,
    moderator: Option<bool>,
    title: Option<String>,
}

/// `user.save!` with what changed: the custom fields that differ
/// (HasCustomFields), the primary email (its old address's tokens
/// destroyed, UserEmail's before_save), the columns that differ with
/// updated_at, `change_display_name` for a new name, then the after_save
/// callbacks, which run on every save.
async fn save_user(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    before: &UserRow,
    p: &Pending,
) -> Result<(), AppError> {
    let user_id = before.id;
    for (k, v) in &p.custom_fields {
        let existing: Vec<String> = sqlx::query_scalar(
            "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id",
        )
        .bind(user_id)
        .bind(k)
        .fetch_all(&mut *conn)
        .await?;
        if existing.len() == 1 && existing[0] == *v {
            continue;
        }
        sqlx::query("DELETE FROM user_custom_fields WHERE user_id = $1 AND name = $2")
            .bind(user_id)
            .bind(k)
            .execute(&mut *conn)
            .await?;
        sqlx::query(
            "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(user_id)
        .bind(k)
        .bind(v)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(email) = &p.email
        && before.email.as_deref() != Some(email.as_str())
    {
        if let Some(old) = &before.email {
            sqlx::query("DELETE FROM email_tokens WHERE email = $1")
                .bind(old)
                .execute(&mut *conn)
                .await?;
        }
        sqlx::query(
            "UPDATE user_emails SET email = $2, normalized_email = $3, updated_at = clock_timestamp() \
             WHERE user_id = $1 AND \"primary\"",
        )
        .bind(user_id)
        .bind(email)
        .bind(crate::signup::normalize_email(email))
        .execute(&mut *conn)
        .await?;
    }
    let current_ip: Option<String> =
        sqlx::query_scalar("SELECT host(ip_address) FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let current_locale: Option<String> =
        sqlx::query_scalar("SELECT locale FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    let name_changed = p
        .name
        .as_deref()
        .is_some_and(|n| before.name.as_deref() != Some(n));
    let changed = name_changed
        || p.locale
            .as_deref()
            .is_some_and(|l| current_locale.as_deref() != Some(l))
        || p.active.is_some_and(|a| a != before.active)
        || p.ip.as_ref().is_some_and(|ip| current_ip != *ip)
        || p.admin.is_some_and(|a| a != before.admin)
        || p.moderator.is_some_and(|m| m != before.moderator)
        || p.title
            .as_deref()
            .is_some_and(|t| before.title.as_deref() != Some(t));
    if changed {
        sqlx::query(
            "UPDATE users SET name = COALESCE($2, name), locale = COALESCE($3, locale), \
                    active = COALESCE($4, active), \
                    ip_address = CASE WHEN $9 THEN $5::inet ELSE ip_address END, \
                    admin = COALESCE($6, admin), moderator = COALESCE($7, moderator), \
                    title = COALESCE($8, title), updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(user_id)
        .bind(&p.name)
        .bind(&p.locale)
        .bind(p.active)
        .bind(p.ip.clone().flatten())
        .bind(p.admin)
        .bind(p.moderator)
        .bind(&p.title)
        .bind(p.ip.is_some())
        .execute(&mut *conn)
        .await?;
    }
    // after_update :change_display_name
    if name_changed {
        crate::jobs::enqueue(
            &mut *conn,
            "change_display_name",
            json!({"user_id": user_id, "old_name": before.name, "new_name": p.name}),
        )
        .await?;
    }
    let after = load_user(conn, user_id)
        .await?
        .ok_or(Unsupported("a DiscourseConnect user that vanished"))?;
    crate::user_updater::after_save(
        conn,
        cx.settings,
        user_id,
        after.admin && after.active,
        &crate::accounts::normalize_username(&after.username)?,
        after.name.as_deref(),
    )
    .await
}

/// `UsernameChanger.override(user, username)`
async fn override_username(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user: &UserRow,
    new_username: &str,
) -> Result<(), AppError> {
    let host = crate::pretty_text::Host::from_state(cx.state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: cx.settings,
        config: &cx.state.config,
        i18n: cx.i18n,
        bus: &cx.state.bus,
    };
    let equal = crate::accounts::normalize_username(&user.username)?
        == crate::accounts::normalize_username(new_username)?;
    if equal {
        crate::username_changer::change(
            conn,
            &ctx,
            user.id,
            new_username,
            Some(user.id),
            crate::username_changer::Update::Enqueue,
        )
        .await?;
        return Ok(());
    }
    let fixed =
        crate::user_name_suggester::fix_username(cx.settings, cx.i18n, Some(new_username), true)?;
    if fixed.as_deref() != Some(user.username.as_str()) {
        let suggested = crate::user_name_suggester::suggest(
            conn,
            cx.settings,
            cx.i18n,
            &[Some(new_username)],
            Some(&user.username),
            true,
        )
        .await?
        .unwrap_or_default();
        crate::username_changer::change(
            conn,
            &ctx,
            user.id,
            &suggested,
            Some(user.id),
            crate::username_changer::Update::Enqueue,
        )
        .await?;
    }
    Ok(())
}

async fn set_automatic_groups(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user_id: i32,
) -> Result<(), AppError> {
    let host = crate::pretty_text::Host::from_state(cx.state);
    let promotion = crate::promotion::Ctx {
        settings: cx.settings,
        i18n: cx.i18n,
        host: &host,
        config: &cx.state.config,
    };
    let gm = crate::group_manager::Cx {
        state: cx.state,
        promotion: &promotion,
    };
    crate::group_manager::set_automatic_groups(conn, &gm, user_id).await
}

/// The first reason the primary email is refused (`Some(None)` when it is
/// blank), as User's validation of it would.
async fn email_error(
    conn: &mut PgConnection,
    s: &SiteSettings,
    email: &str,
    except_user: Option<i32>,
) -> Result<Option<Option<String>>, AppError> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(Some(None));
    }
    if !crate::accounts::valid_email(&email) || !crate::signup::email_allowed(s, &email)? {
        return Ok(Some(Some(email)));
    }
    let screened: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM screened_emails WHERE email = $1)")
            .bind(&email)
            .fetch_one(&mut *conn)
            .await?;
    if screened {
        return Err(Unsupported("screened emails through DiscourseConnect").into());
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM user_emails WHERE lower(email) = $1 AND user_id IS DISTINCT FROM $2)",
    )
    .bind(&email)
    .bind(except_user)
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        return Ok(Some(Some(email)));
    }
    Ok(None)
}

/// `match_email_or_create_user(ip_address)`
async fn match_email_or_create_user(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    payloads: &crate::web_hooks::Payloads<'_>,
    sso: &Sso,
    ip: Option<&str>,
) -> Result<Result<i32, LookupError>, AppError> {
    let s = cx.settings;
    let email = sso.get("email").unwrap_or("");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("discourse_single_sign_on_{email}"))
        .execute(&mut *conn)
        .await?;
    let require_activation = sso.flag("require_activation").unwrap_or(false);
    // User.find_by_email (any of the user's addresses).
    let mut user_id: Option<i32> = if require_activation {
        None
    } else {
        sqlx::query_scalar(
            "SELECT user_id FROM user_emails WHERE lower(email) = lower($1) ORDER BY \"primary\" DESC, id LIMIT 1",
        )
        .bind(email.trim())
        .fetch_optional(&mut *conn)
        .await?
    };
    if user_id.is_none() {
        if s.get("site_archived")?.truthy() {
            return Ok(Err(LookupError::Unknown));
        }
        if let Some(e) = email_error(conn, s, email, None).await? {
            return Ok(Err(LookupError::Email(e)));
        }
        let name = resolve_name(s, sso);
        let username = resolve_username(conn, cx, sso).await?;
        let locale = sso
            .present("locale")
            .filter(|l| valid_locale(l))
            .filter(|_| s.get("allow_user_locale").is_ok_and(|v| v.truthy()));
        if s.get("must_approve_users")?.truthy() {
            return Err(Unsupported(
                "approving users created by DiscourseConnect (can_auto_approve_user?)",
            )
            .into());
        }
        let email = email.to_lowercase();
        let id = crate::signup::insert_user(
            conn,
            s,
            payloads,
            &crate::signup::NewUser {
                username: &username,
                name: Some(name.as_str()),
                email: &email,
                password: None,
                active: false,
                locale,
                ip,
                custom_fields: &[],
            },
        )
        .await?;
        user_id = Some(id);
    }
    let user_id = user_id.expect("a user");
    let existing: Option<i32> =
        sqlx::query_scalar("SELECT id FROM single_sign_on_records WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    if existing.is_none() {
        let overrides_avatar = s.get("discourse_connect_overrides_avatar")?.truthy();
        if let Some(url) = sso.present("avatar_url") {
            crate::jobs::enqueue(
                &mut *conn,
                "download_avatar_from_url",
                json!({"url": url, "user_id": user_id, "override_gravatar": overrides_avatar}),
            )
            .await?;
        }
        if let Some(url) = sso.present("profile_background_url") {
            crate::jobs::enqueue(
                &mut *conn,
                "download_profile_background_from_url",
                json!({"url": url, "user_id": user_id, "is_card_background": false}),
            )
            .await?;
        }
        if let Some(url) = sso.present("card_background_url") {
            crate::jobs::enqueue(
                &mut *conn,
                "download_profile_background_from_url",
                json!({"url": url, "user_id": user_id, "is_card_background": true}),
            )
            .await?;
        }
        sqlx::query(
            "INSERT INTO single_sign_on_records (user_id, external_id, last_payload, created_at, updated_at, \
                external_username, external_email, external_name, external_avatar_url, \
                external_profile_background_url, external_card_background_url) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp(), $4, $5, $6, $7, $8, $9)",
        )
        .bind(user_id)
        .bind(sso.get("external_id").unwrap_or(""))
        .bind(sso.unsigned_payload())
        .bind(sso.get("username"))
        .bind(sso.get("email"))
        .bind(sso.get("name"))
        .bind(sso.get("avatar_url"))
        .bind(sso.get("profile_background_url"))
        .bind(sso.get("card_background_url"))
        .execute(&mut *conn)
        .await?;
    }
    Ok(Ok(user_id))
}

/// `resolve_name`
fn resolve_name(s: &SiteSettings, sso: &Sso) -> String {
    if let Some(name) = sso.present("name") {
        return name.to_string();
    }
    let mut input = sso.present("username").map(str::to_string);
    if input.is_none()
        && s.get("use_email_for_username_and_name_suggestions")
            .is_ok_and(|v| v.truthy())
    {
        input = sso.get("email").map(str::to_string);
    }
    crate::user_name_suggester::suggest_name(input.as_deref().unwrap_or(""))
}

/// `resolve_username`
async fn resolve_username(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    sso: &Sso,
) -> Result<String, AppError> {
    let s = cx.settings;
    let mut input = vec![sso.get("username")];
    if s.get("use_name_for_username_suggestions")?.truthy() {
        input.push(sso.get("name"));
    }
    if s.get("use_email_for_username_and_name_suggestions")?
        .truthy()
    {
        input.push(sso.get("email"));
    }
    Ok(
        crate::user_name_suggester::suggest(conn, s, cx.i18n, &input, None, true)
            .await?
            .unwrap_or_default(),
    )
}

/// The non-automatic groups whose lowercase names are in a list, by id.
async fn groups_named(conn: &mut PgConnection, list: &str) -> Result<Vec<i32>, AppError> {
    let names: Vec<String> = list.split(',').map(|n| n.to_lowercase()).collect();
    Ok(sqlx::query_scalar(
        "SELECT id FROM groups WHERE lower(name) = ANY($1) AND NOT automatic ORDER BY id",
    )
    .bind(&names)
    .fetch_all(&mut *conn)
    .await?)
}

/// `apply_group_rules(user)`
async fn apply_group_rules(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    sso: &Sso,
    user_id: i32,
) -> Result<(), AppError> {
    let s = cx.settings;
    let member_of: Vec<i32> = sqlx::query_scalar(
        "SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
         WHERE gu.user_id = $1 AND NOT g.automatic ORDER BY g.id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let (to_add, to_remove): (Vec<i32>, Vec<i32>) =
        if s.get("discourse_connect_overrides_groups")?.truthy() {
            // synchronize_groups
            let desired = groups_named(conn, sso.get("groups").unwrap_or("")).await?;
            (
                desired
                    .iter()
                    .filter(|g| !member_of.contains(g))
                    .copied()
                    .collect(),
                member_of
                    .iter()
                    .filter(|g| !desired.contains(g))
                    .copied()
                    .collect(),
            )
        } else {
            let mut to_add = Vec::new();
            if let Some(add) = sso.get("add_groups")
                && !add.is_empty()
            {
                let all: Vec<i32> =
                    sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1")
                        .bind(user_id)
                        .fetch_all(&mut *conn)
                        .await?;
                to_add = groups_named(conn, add)
                    .await?
                    .into_iter()
                    .filter(|g| !all.contains(g))
                    .collect();
            }
            let mut to_remove = Vec::new();
            if let Some(remove) = sso.get("remove_groups")
                && !remove.is_empty()
            {
                to_remove = groups_named(conn, remove)
                    .await?
                    .into_iter()
                    .filter(|g| member_of.contains(g))
                    .collect();
            }
            (to_add, to_remove)
        };
    if to_add.is_empty() && to_remove.is_empty() {
        return Ok(());
    }
    let host = crate::pretty_text::Host::from_state(cx.state);
    let promotion = crate::promotion::Ctx {
        settings: s,
        i18n: cx.i18n,
        host: &host,
        config: &cx.state.config,
    };
    let gm = crate::group_manager::Cx {
        state: cx.state,
        promotion: &promotion,
    };
    // add_user_to_groups: GroupUser.create! and the add log, each group.
    for group_id in to_add {
        let Some(group) = crate::group_manager::find(conn, group_id).await? else {
            continue;
        };
        crate::group_manager::add(conn, &gm, &group, user_id, SYSTEM_USER_ID).await?;
    }
    // remove_user_from_groups: the memberships destroyed, then a log each.
    for group_id in &to_remove {
        let Some(group) = crate::group_manager::find(conn, *group_id).await? else {
            continue;
        };
        crate::group_manager::remove(conn, &gm, &group, user_id, SYSTEM_USER_ID).await?;
    }
    Ok(())
}

/// Why DiscourseConnectProvider refused a request.
#[derive(Debug, PartialEq)]
pub enum ProviderError {
    /// BlankSecret: no secret for the return url's host.
    BlankSecret,
    /// ParseError: no return url, or the payload or signature is bad.
    Parse,
    /// BlankReturnUrl
    BlankReturnUrl,
    /// InvalidParameterValueError(param)
    InvalidParameter(&'static str),
}

/// A request this site answers as a DiscourseConnect provider, with the
/// secret it was signed with (the answer is signed with it too).
pub struct ProviderRequest {
    pub sso: Sso,
    pub secret: String,
}

/// `WildcardDomainChecker.check_domain(domain, external_domain)`
pub fn check_domain(domain: &str, external: &str) -> bool {
    let pattern = match domain.strip_prefix('*') {
        Some(rest) => format!(r"(?i)\A\S*{}\z", regex::escape(rest)),
        None => format!(r"(?i)\A{}\z", regex::escape(domain)),
    };
    regex::Regex::new(&pattern).is_ok_and(|r| r.is_match(external))
}

/// `DiscourseConnectProvider.parse(payload)` with `get_sso`'s check:
/// the secret found from the return url's host
/// (discourse_connect_provider_secrets, the domains in reverse order, the
/// first whose secret signs the payload, else the first that matches).
pub fn provider_parse(
    query: &str,
    settings: &SiteSettings,
) -> Result<Result<ProviderRequest, ProviderError>, AppError> {
    let parsed = parse_query(query.as_bytes())?;
    let get = |k: &str| {
        parsed
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    let sso_param = get("sso").unwrap_or("");
    let decoded = parse_query(&decode64(sso_param))?;
    let Some(return_sso_url) = decoded
        .iter()
        .find(|(k, _)| k == "return_sso_url")
        .map(|(_, v)| v.clone())
    else {
        return Ok(Err(ProviderError::Parse));
    };
    let mut secret: Option<String> = None;
    if settings.get("enable_discourse_connect_provider")?.truthy() {
        let host = crate::wildcard_url::parse(&return_sso_url)
            .and_then(|u| u.host)
            .ok_or(Unsupported("a DiscourseConnect return url without a host"))?;
        let mut rows: Vec<(String, Option<String>)> = settings
            .get("discourse_connect_provider_secrets")?
            .to_s()
            .split('\n')
            .map(|row| match row.split_once('|') {
                Some((d, s)) => (d.to_string(), Some(s.to_string())),
                None => (row.to_string(), None),
            })
            .collect();
        rows.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        rows.reverse();
        let sig = get("sig").unwrap_or("");
        let mut first_match: Option<Option<String>> = None;
        for (domain, configured) in &rows {
            if !check_domain(domain, &host) {
                continue;
            }
            if first_match.is_none() {
                first_match = Some(configured.clone());
            }
            let Some(configured) = configured else {
                return Err(Unsupported("a provider secret row without a secret").into());
            };
            if bool::from(subtle::ConstantTimeEq::ct_eq(
                sign(sso_param, configured).as_bytes(),
                sig.as_bytes(),
            )) {
                secret = Some(configured.clone());
                first_match = None;
                break;
            }
        }
        if secret.is_none() {
            secret = first_match.flatten();
        }
    }
    let Some(secret) = secret.filter(|s| !s.trim().is_empty()) else {
        if settings
            .get("verbose_discourse_connect_logging")
            .is_ok_and(|v| v.truthy())
        {
            tracing::warn!(
                "SSO failed; website is not in the `discourse_connect_provider_secrets` site settings"
            );
        }
        return Ok(Err(ProviderError::BlankSecret));
    };
    let sso = match parse(query, &secret)? {
        Ok(sso) => sso,
        Err(_) => return Ok(Err(ProviderError::Parse)),
    };
    if sso.get("prompt").is_some_and(|p| p != "none") {
        return Ok(Err(ProviderError::InvalidParameter("prompt")));
    }
    if sso
        .get("return_sso_url")
        .is_none_or(|u| u.trim().is_empty())
    {
        return Ok(Err(ProviderError::BlankReturnUrl));
    }
    Ok(Ok(ProviderRequest { sso, secret }))
}

/// `populate_user_data(sso)`: what the claims let this site tell the
/// requesting site about the user.
pub async fn populate_user_data(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &crate::url::Urls<'_>,
    sso: &mut Sso,
    user_id: i32,
) -> Result<(), AppError> {
    let claims_setting = settings.get("discourse_connect_provider_claims")?.to_s();
    let claims: Vec<&str> = claims_setting.split('|').collect();
    #[derive(sqlx::FromRow)]
    struct U {
        name: Option<String>,
        username: String,
        email: Option<String>,
        admin: bool,
        moderator: bool,
        avatar_url: Option<String>,
        profile_background_url: Option<String>,
        card_background_url: Option<String>,
    }
    let u: U = sqlx::query_as(
        "SELECT u.name, u.username, \
                (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) AS email, \
                u.admin, u.moderator, \
                (SELECT url FROM uploads WHERE id = u.uploaded_avatar_id) AS avatar_url, \
                (SELECT up.url FROM user_profiles p JOIN uploads up ON up.id = p.profile_background_upload_id \
                  WHERE p.user_id = u.id) AS profile_background_url, \
                (SELECT up.url FROM user_profiles p JOIN uploads up ON up.id = p.card_background_upload_id \
                  WHERE p.user_id = u.id) AS card_background_url \
         FROM users u WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *conn)
    .await?;
    let set = |sso: &mut Sso, key: &'static str, value: Option<String>| match value {
        Some(v) => sso.set(key, Field::Str(v)),
        None => sso.unset(key),
    };
    if claims.contains(&"name") {
        set(sso, "name", u.name.clone());
    }
    sso.set("username", Field::Str(u.username.clone()));
    if claims.contains(&"email") {
        set(sso, "email", u.email.clone());
    }
    sso.set("external_id", Field::Str(user_id.to_string()));
    if claims.contains(&"admin") {
        sso.set("admin", Field::Bool(u.admin));
    }
    if claims.contains(&"moderator") {
        sso.set("moderator", Field::Bool(u.moderator));
    }
    if claims.contains(&"groups") {
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT g.name FROM groups g JOIN group_users gu ON gu.group_id = g.id \
             WHERE gu.user_id = $1 ORDER BY gu.id",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await?;
        sso.set("groups", Field::Str(names.join(",")));
    }
    for (claim, url) in [
        ("avatar_url", u.avatar_url),
        ("profile_background_url", u.profile_background_url),
        ("card_background_url", u.card_background_url),
    ] {
        if claims.contains(&claim)
            && let Some(url) = url
        {
            let key = ACCESSORS
                .iter()
                .find(|k| **k == claim)
                .copied()
                .expect("an accessor");
            sso.set(key, Field::Str(urls.full_cdn_url(&url)?));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_domains() {
        assert!(check_domain("*.example.com", "a.example.com"));
        assert!(check_domain("*.example.com", "a.b.example.com"));
        assert!(!check_domain("*.example.com", "example.org"));
        assert!(check_domain("Example.com", "example.COM"));
        assert!(!check_domain("example.com", "xexample.com"));
    }

    fn signed(fields: &[(&str, &str)], secret: &str) -> String {
        use base64::Engine;
        let query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields.iter().copied())
            .finish();
        let payload = base64::engine::general_purpose::STANDARD.encode(query);
        let escaped: String = form_urlencoded::byte_serialize(payload.as_bytes()).collect();
        format!("sso={escaped}&sig={}", sign(&payload, secret))
    }

    #[test]
    fn parses_a_signed_payload() {
        let q = signed(
            &[
                ("nonce", "abc"),
                ("external_id", "42"),
                ("email", "a@example.com"),
                ("admin", "true"),
                ("moderator", "yes"),
                ("custom.shoe_size", "11"),
            ],
            "s3cret",
        );
        let sso = parse(&q, "s3cret").unwrap().unwrap();
        assert_eq!(sso.get("external_id"), Some("42"));
        assert_eq!(sso.flag("admin"), Some(true));
        // Neither "true" nor "false": nil.
        assert_eq!(sso.flag("moderator"), None);
        assert_eq!(sso.custom_fields, vec![("shoe_size".into(), "11".into())]);
    }

    #[test]
    fn refuses_bad_signatures_and_payloads() {
        let q = signed(&[("nonce", "abc")], "s3cret");
        assert_eq!(
            parse(&q, "other").unwrap().unwrap_err(),
            ParseError::Signature
        );
        assert_eq!(parse(&q, "").unwrap().unwrap_err(), ParseError::BlankSecret);
        assert_eq!(
            parse("sso=not%20base64&sig=x", "s3cret")
                .unwrap()
                .unwrap_err(),
            ParseError::Payload
        );
    }

    #[test]
    fn payload_round_trips() {
        let mut sso = Sso::default();
        sso.set(
            "return_sso_url",
            Field::Str("http://x/session/sso_login".into()),
        );
        sso.set("nonce", Field::Str("n1".into()));
        // ACCESSORS order, whatever the order set.
        assert_eq!(
            sso.unsigned_payload(),
            "nonce=n1&return_sso_url=http%3A%2F%2Fx%2Fsession%2Fsso_login"
        );
        let url = sso.to_url("https://idp.example/sso", "k");
        let query = url.split_once('?').unwrap().1;
        let back = parse(query, "k").unwrap().unwrap();
        assert_eq!(back.get("nonce"), Some("n1"));
    }

    #[test]
    fn decodes_like_ruby() {
        assert_eq!(decode64("bm9uY2U9YQ=="), b"nonce=a");
        assert_eq!(decode64("bm9uY2U9YQ"), b"nonce=a");
        assert_eq!(decode64("bm9u\nY2U9YQ=="), b"nonce=a");
    }
}
