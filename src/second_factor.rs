//! `SecondFactorManager#authenticate_second_factor` (app/models/concerns/
//! second_factor_manager.rb): the method chosen, then the TOTP code (ROTP's
//! verify) or a backup code. Security keys and passkeys are refused.

use hmac::{Hmac, Mac};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use subtle::ConstantTimeEq;

use crate::config::RailsEnv;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `UserSecondFactor.methods`
pub const TOTP: i64 = 1;
pub const BACKUP_CODES: i64 = 2;
pub const SECURITY_KEY: i64 = 3;
pub const PASSKEY: i64 = 4;

/// `SecondFactorManager::TOTP_ALLOWED_DRIFT_SECONDS`
const DRIFT_SECONDS: i64 = 30;
const INTERVAL: i64 = 30;

/// The user's second factors, as `*_enabled?` sees them.
pub struct Enabled {
    pub totp: bool,
    pub backup: bool,
    pub security_keys: bool,
    /// `passkeys_available_as_second_factor?`
    pub passkeys: bool,
}

impl Enabled {
    pub async fn load(
        conn: &mut PgConnection,
        settings: &SiteSettings,
        user_id: i32,
    ) -> Result<Self, AppError> {
        // Every *_enabled? first needs local logins without DiscourseConnect.
        let local = !settings.get("enable_discourse_connect")?.truthy()
            && settings.get("enable_local_logins")?.truthy();
        let (totp, backup, security_keys, passkeys): (bool, bool, bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 1), \
                    EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 2), \
                    EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0), \
                    EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 1)",
        )
        .bind(user_id)
        .fetch_one(conn)
        .await?;
        let passkeys = passkeys
            && local
            && settings.get("allow_passkeys_for_2fa")?.truthy()
            && settings.get("enable_passkeys")?.truthy();
        Ok(Self {
            totp: local && totp,
            backup: local && backup,
            security_keys: local && security_keys,
            passkeys,
        })
    }

    /// `security_keys_enabled? || totp_or_backup_codes_enabled?`: what a
    /// password login asks for.
    pub fn any(&self) -> bool {
        self.totp || self.backup || self.security_keys
    }

    /// `has_any_second_factor_methods_enabled?`: TOTP or security keys,
    /// backup codes and passkeys left out.
    pub fn has_any(&self) -> bool {
        self.totp || self.security_keys
    }

    /// `has_multiple_second_factor_methods?`
    pub fn multiple(&self) -> bool {
        self.security_keys && (self.totp || self.backup)
    }

    /// `valid_second_factor_method_for_user?(method)`
    pub fn valid_method(&self, method: i64) -> bool {
        match method {
            TOTP => self.totp,
            BACKUP_CODES => self.backup,
            SECURITY_KEY => self.security_keys,
            PASSKEY => self.passkeys,
            _ => false,
        }
    }

    /// `invalid_second_factor_authentication_result(...).to_h`, merged
    /// into failed_json.
    pub fn failure(&self, i18n: &I18n, key: &str, reason: &str) -> Value {
        json!({
            "failed": "FAILED",
            "ok": false,
            "error": i18n.t(key).unwrap_or(key),
            "reason": reason,
            "backup_enabled": self.backup,
            "security_key_enabled": self.security_keys,
            "totp_enabled": self.totp,
            "multiple_second_factor_methods": self.multiple(),
            "used_2fa_method": null,
        })
    }
}

/// `hash_backup_code`'s PBKDF2 iterations: fewer under RAILS_ENV=test.
pub fn backup_code_iterations(env: RailsEnv) -> u32 {
    if env == RailsEnv::Test { 10 } else { 64_000 }
}

/// `authenticate_second_factor(params, server_session)`: the failure
/// payload, or the method that went through (None when the user has no
/// second factor to ask for). A TOTP's last_used and a backup code's
/// spending are written here, as Rails writes them before it goes on.
#[allow(clippy::too_many_arguments)]
pub async fn authenticate(
    conn: &mut PgConnection,
    i18n: &I18n,
    user_id: i32,
    enabled: &Enabled,
    method: Option<&str>,
    token: Option<&str>,
    now: i64,
    backup_iterations: u32,
) -> Result<Result<Option<i64>, Value>, AppError> {
    let method = method.filter(|m| !m.trim().is_empty());
    if !enabled.security_keys
        && !enabled.totp
        && !enabled.backup
        && (!enabled.passkeys || method.is_none())
    {
        return Ok(Ok(None));
    }
    let method = method.map(crate::ruby::to_i);
    let Some(method @ (TOTP | BACKUP_CODES | SECURITY_KEY | PASSKEY)) = method else {
        return Ok(Err(enabled.failure(
            i18n,
            "login.invalid_second_factor_method",
            "invalid_second_factor_method",
        )));
    };
    if !enabled.valid_method(method) {
        return Ok(Err(enabled.failure(
            i18n,
            "login.not_enabled_second_factor_method",
            "not_enabled_second_factor_method",
        )));
    }
    let token = token.unwrap_or_default();
    let ok = match method {
        TOTP => authenticate_totp(conn, user_id, token, now).await?,
        BACKUP_CODES => authenticate_backup_code(conn, user_id, token, backup_iterations).await?,
        SECURITY_KEY => {
            return Err(Unsupported("security keys as a second factor (webauthn)").into());
        }
        _ => return Err(Unsupported("passkeys as a second factor (webauthn)").into()),
    };
    if ok {
        return Ok(Ok(Some(method)));
    }
    Ok(Err(enabled.failure(
        i18n,
        "login.invalid_second_factor_code",
        "invalid_second_factor",
    )))
}

/// SessionController#authenticate_second_factor's failure payload: for a
/// user with security keys, a webauthn challenge staged and the
/// credentials to answer it merged in.
pub async fn stage_failure(
    conn: &mut PgConnection,
    session_id: &str,
    user_id: i32,
    enabled: &Enabled,
    mut failure: Value,
) -> Result<Value, sqlx::Error> {
    if enabled.security_keys {
        crate::webauthn::stage_challenge(&mut *conn, session_id, Some(user_id)).await?;
        let allowed =
            crate::webauthn::allowed_credentials(conn, session_id, user_id, enabled, false).await?;
        if let Value::Object(out) = &mut failure {
            out.extend(allowed);
        }
    }
    Ok(failure)
}

/// `authenticate_totp(token)`: the first enabled TOTP the code is right
/// for gets its last_used.
async fn authenticate_totp(
    conn: &mut PgConnection,
    user_id: i32,
    token: &str,
    now: i64,
) -> Result<bool, AppError> {
    if token.trim().is_empty() {
        return Ok(false);
    }
    let totps: Vec<(i64, String, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT id, data, last_used FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 1 \
         ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for (id, secret, last_used) in totps {
        let after = last_used.map_or(0, |t| t.and_utc().timestamp());
        if verify(&secret, token, now, after)? {
            sqlx::query(
                "UPDATE user_second_factors SET last_used = clock_timestamp(), \
                 updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// `authenticate_backup_code(backup_code)`: a match is spent (disabled,
/// with its last_used).
async fn authenticate_backup_code(
    conn: &mut PgConnection,
    user_id: i32,
    code: &str,
    iterations: u32,
) -> Result<bool, AppError> {
    if code.trim().is_empty() {
        return Ok(false);
    }
    let codes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, data FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 2 \
         ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    for (id, data) in codes {
        let parsed: Map<String, Value> = serde_json::from_str(&data)
            .map_err(|_| Unsupported("a backup code whose data is not a JSON object"))?;
        let field = |k: &str| match parsed.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let hash = hash_backup_code(code, &field("salt"), iterations);
        if bool::from(hash.as_bytes().ct_eq(field("code_hash").as_bytes())) {
            sqlx::query(
                "UPDATE user_second_factors SET enabled = false, last_used = clock_timestamp(), \
                 updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// `hash_backup_code(code, salt)`: Pbkdf2.hash_password over SHA-256, hex.
pub fn hash_backup_code(code: &str, salt: &str, iterations: u32) -> String {
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(code.as_bytes(), salt.as_bytes(), iterations, &mut out);
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// `remaining_backup_codes`: the enabled ones.
pub async fn remaining_backup_codes(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT count(*) FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 2",
    )
    .bind(user_id)
    .fetch_one(conn)
    .await
}

/// `ROTP::TOTP#verify(otp, drift_ahead: 30, drift_behind: 30, after:)`.
fn verify(secret: &str, otp: &str, now: i64, after: i64) -> Result<bool, AppError> {
    let key = base32_decode(secret).ok_or(Unsupported("TOTP secrets that are not base32"))?;
    let first = (now - DRIFT_SECONDS) / INTERVAL;
    let last = (now + DRIFT_SECONDS) / INTERVAL;
    let after = (after > 0).then_some(after / INTERVAL);
    Ok((first..=last)
        .filter(|t| after.is_none_or(|a| *t > a))
        .any(|t| bool::from(code(&key, t).as_bytes().ct_eq(otp.as_bytes()))))
}

/// `ROTP::TOTP#at(now)`: the code a base32 secret gives at a time.
pub fn code_at(secret: &str, now: i64) -> Option<String> {
    Some(code(&base32_decode(secret)?, now / INTERVAL))
}

/// ROTP::OTP#generate_otp: HOTP over SHA-1, six digits.
fn code(key: &[u8], counter: i64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!("{:06}", binary % 1_000_000)
}

/// `ROTP::Base32.decode`: RFC 4648 letters, padding dropped.
fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let value = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | u32::from(value);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_sha1_codes() {
        // RFC 6238 appendix B, the SHA-1 seed, last six digits.
        let key = b"12345678901234567890";
        assert_eq!(code(key, 59 / 30), "287082");
        assert_eq!(code(key, 1111111109 / 30), "081804");
        assert_eq!(code(key, 1111111111 / 30), "050471");
        assert_eq!(code(key, 1234567890 / 30), "005924");
        assert_eq!(code(key, 2000000000 / 30), "279037");
        assert_eq!(code(key, 20000000000 / 30), "353130");
    }

    #[test]
    fn rfc4226_hotp_codes() {
        // RFC 4226 appendix D: HOTP over the same seed, counters 0-9.
        let key = b"12345678901234567890";
        let want = [
            "755224", "287082", "359152", "969429", "338314", "254676", "287922", "162583",
            "399871", "520489",
        ];
        for (counter, want) in want.iter().enumerate() {
            assert_eq!(code(key, counter as i64), *want);
        }
    }

    #[test]
    fn verify_allows_one_step_of_drift_and_rejects_reuse() {
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"; // base32 of the RFC seed
        assert!(verify(secret, "081804", 1111111109, 0).unwrap());
        assert!(verify(secret, "081804", 1111111109 + 30, 0).unwrap());
        assert!(!verify(secret, "081804", 1111111109 + 90, 0).unwrap());
        assert!(!verify(secret, "081804", 1111111109, 1111111109).unwrap());
        assert!(!verify(secret, "00000000", 1111111109, 0).unwrap());
    }

    #[test]
    fn backup_code_hash_is_pbkdf2_sha256_hex() {
        // RFC 7914 section 11's PBKDF2-HMAC-SHA256 vector ("passwd",
        // "salt", 1 iteration), the first 32 bytes, as Pbkdf2.hash_password
        // gives them.
        assert_eq!(
            hash_backup_code("passwd", "salt", 1),
            "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc"
        );
    }
}
