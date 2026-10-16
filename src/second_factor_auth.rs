//! `SecondFactor::AuthManager` (lib/second_factor/auth_manager.rb): an
//! action that needs the acting user's second factor stages a challenge
//! in the server session, the browser completes it at /session/2fa, and
//! the action is replayed with the challenge's nonce.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The server session key the challenge lives under.
pub const CHALLENGE_KEY: &str = "current_second_factor_auth_challenge";
/// `MAX_CHALLENGE_AGE`
const MAX_CHALLENGE_AGE_SECONDS: i64 = 5 * 60;

/// `SecondFactor::BadChallenge`: its translation key and status.
pub struct BadChallenge {
    pub key: &'static str,
    pub status: StatusCode,
}

impl BadChallenge {
    fn not_found() -> Self {
        Self {
            key: "second_factor_auth.challenge_not_found",
            status: StatusCode::NOT_FOUND,
        }
    }

    pub fn message(&self, i18n: &I18n) -> String {
        i18n.t(self.key).unwrap_or(self.key).to_string()
    }

    /// The `rescue_from SecondFactor::BadChallenge` response.
    pub fn response(&self, i18n: &I18n) -> Response {
        (self.status, Json(json!({ "error": self.message(i18n) }))).into_response()
    }
}

/// `AuthManager.find_second_factor_challenge(nonce:, server_session:,
/// target_user:)`
pub async fn find(
    conn: &mut PgConnection,
    session_id: &str,
    nonce: &str,
    target_user_id: Option<i32>,
) -> Result<Result<Map<String, Value>, BadChallenge>, AppError> {
    let stored = crate::session::server::get(conn, session_id, CHALLENGE_KEY).await?;
    // Stored as JSON text, as Rails keeps it.
    let challenge = match stored {
        Some(Value::String(text)) if !text.trim().is_empty() => {
            serde_json::from_str::<Value>(&text)
                .map_err(|_| Unsupported("an unreadable second factor challenge"))?
        }
        _ => return Ok(Err(BadChallenge::not_found())),
    };
    let Value::Object(challenge) = challenge else {
        return Err(Unsupported("an unreadable second factor challenge").into());
    };
    if challenge.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return Ok(Err(BadChallenge::not_found()));
    }
    if let Some(target) = target_user_id
        && challenge.get("target_user_id").and_then(Value::as_i64) != Some(i64::from(target))
    {
        return Ok(Err(BadChallenge::not_found()));
    }
    let generated_at = challenge
        .get("generated_at")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if generated_at < crate::clock::now().timestamp() - MAX_CHALLENGE_AGE_SECONDS {
        return Ok(Err(BadChallenge {
            key: "second_factor_auth.challenge_expired",
            status: StatusCode::UNAUTHORIZED,
        }));
    }
    Ok(Ok(challenge))
}

/// `server_session[CHALLENGE_KEY] = challenge.to_json`
pub async fn store(
    conn: &mut PgConnection,
    session_id: &str,
    challenge: &Map<String, Value>,
) -> Result<(), sqlx::Error> {
    let text = Value::Object(challenge.clone()).to_string();
    crate::session::server::set(
        conn,
        session_id,
        CHALLENGE_KEY,
        &Value::String(text),
        crate::session::server::EXPIRY_SECONDS,
    )
    .await
}

pub async fn delete(conn: &mut PgConnection, session_id: &str) -> Result<(), sqlx::Error> {
    crate::session::server::delete(conn, session_id, CHALLENGE_KEY).await
}

/// `UserSecondFactor.methods` the default AuthManager allows: TOTP,
/// security keys and passkeys (backup codes only when an action allows
/// them).
pub const DEFAULT_ALLOWED_METHODS: [i64; 3] = [
    crate::second_factor::TOTP,
    crate::second_factor::SECURITY_KEY,
    crate::second_factor::PASSKEY,
];

/// `valid_second_factor_method_for_user?(method)`
pub async fn valid_method_for_user(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    method: i64,
) -> Result<bool, AppError> {
    let enabled = crate::second_factor::Enabled::load(&mut *conn, settings, user_id).await?;
    Ok(enabled.valid_method(method))
}

/// `SecondFactorRequired`'s response: the nonce to an xhr request, else
/// a redirect to the 2FA page.
pub fn required_response(xhr: bool, base_path: &str, nonce: &str) -> Response {
    if xhr {
        (
            StatusCode::FORBIDDEN,
            Json(json!({ "second_factor_challenge_nonce": nonce })),
        )
            .into_response()
    } else {
        (
            StatusCode::FOUND,
            [(
                axum::http::header::LOCATION,
                format!("{base_path}/session/2fa?nonce={nonce}"),
            )],
        )
            .into_response()
    }
}

/// `SecureRandom.alphanumeric(32)`
pub fn new_nonce() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}
