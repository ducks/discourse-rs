//! `SecondFactorManager#authenticate_second_factor` for a password login:
//! the method chosen, then the TOTP code (ROTP's verify). Backup codes and
//! security keys are not ported.

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sqlx::PgConnection;
use subtle::ConstantTimeEq;

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
        let (totp, backup, security_keys): (bool, bool, bool) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 1), \
                    EXISTS (SELECT 1 FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 2), \
                    EXISTS (SELECT 1 FROM user_security_keys WHERE user_id = $1 AND enabled AND factor_type = 0)",
        )
        .bind(user_id)
        .fetch_one(conn)
        .await?;
        Ok(Self {
            totp: local && totp,
            backup: local && backup,
            security_keys: local && security_keys,
        })
    }

    pub fn any(&self) -> bool {
        self.totp || self.backup || self.security_keys
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
            "multiple_second_factor_methods": self.security_keys && (self.totp || self.backup),
            "used_2fa_method": null,
        })
    }
}

/// The failure payload, or None when the code was right.
pub async fn authenticate(
    conn: &mut PgConnection,
    i18n: &I18n,
    user_id: i32,
    enabled: &Enabled,
    method: Option<&str>,
    token: Option<&str>,
    now: i64,
) -> Result<Option<Value>, AppError> {
    let method = method
        .filter(|m| !m.trim().is_empty())
        .map(crate::ruby::to_i);
    let Some(method @ (TOTP | BACKUP_CODES | SECURITY_KEY | PASSKEY)) = method else {
        return Ok(Some(enabled.failure(
            i18n,
            "login.invalid_second_factor_method",
            "invalid_second_factor_method",
        )));
    };
    let valid_for_user = match method {
        TOTP => enabled.totp,
        BACKUP_CODES => enabled.backup,
        SECURITY_KEY => enabled.security_keys,
        _ => return Err(Unsupported("passkeys as a second factor").into()),
    };
    if !valid_for_user {
        return Ok(Some(enabled.failure(
            i18n,
            "login.not_enabled_second_factor_method",
            "not_enabled_second_factor_method",
        )));
    }
    if enabled.security_keys {
        return Err(Unsupported("security keys (staging the webauthn challenge)").into());
    }
    if method != TOTP {
        return Err(Unsupported("backup codes").into());
    }
    let totps: Vec<(String, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT data, last_used FROM user_second_factors WHERE user_id = $1 AND enabled AND method = 1 \
         ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let token = token.unwrap_or_default();
    for (secret, last_used) in totps {
        let after = last_used.map_or(0, |t| t.and_utc().timestamp());
        if !token.trim().is_empty() && verify(&secret, token, now, after)? {
            return Err(Unsupported("logging in with a second factor").into());
        }
    }
    Ok(Some(enabled.failure(
        i18n,
        "login.invalid_second_factor_code",
        "invalid_second_factor",
    )))
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
        assert_eq!(code(key, 2000000000 / 30), "279037");
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
}
