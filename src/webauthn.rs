//! `DiscourseWebauthn` (lib/discourse_webauthn.rb): the challenge staged in
//! the server session for a security key or passkey, and the credentials a
//! user may answer it with.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::second_factor::Enabled;

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
