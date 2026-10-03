//! `UserAuthToken` (app/models/user_auth_token.rb) and the password check
//! (app/models/user_password.rb, lib/pbkdf2.rb).

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::NaiveDateTime;
use sha1::{Digest, Sha1};
use sqlx::PgConnection;
use subtle::ConstantTimeEq;

use crate::Unsupported;

/// `UserAuthToken.hash_token`: SHA-1 of the token and the secret, base64.
pub fn hash_token(unhashed: &str, secret_key_base: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(unhashed.as_bytes());
    hasher.update(secret_key_base.as_bytes());
    B64.encode(hasher.finalize())
}

/// `SecureRandom.hex(16)`
pub fn new_unhashed_token() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `UserPassword#confirm_password?` with the stored PBKDF2 parameters;
/// `None` when the user has no password row.
pub async fn confirm_password(
    conn: &mut PgConnection,
    user_id: i32,
    password: &str,
) -> Result<Option<PasswordCheck>, PasswordError> {
    let row: Option<(String, String, String, Option<NaiveDateTime>)> = sqlx::query_as(
        "SELECT password_hash, password_salt, password_algorithm, password_expired_at \
         FROM user_passwords WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await?;
    let Some((hash, salt, algorithm, expired_at)) = row else {
        return Ok(None);
    };
    let computed = hash_password(password, &salt, &algorithm)?;
    let matches = bool::from(computed.as_bytes().ct_eq(hash.as_bytes()));
    Ok(Some(PasswordCheck {
        matches,
        expired: matches && expired_at.is_some(),
    }))
}

pub struct PasswordCheck {
    pub matches: bool,
    /// `User#password_expired?`: the password matches but was expired.
    pub expired: bool,
}

#[derive(Debug)]
pub enum PasswordError {
    Db(sqlx::Error),
    Unsupported(Unsupported),
}

impl std::fmt::Display for PasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasswordError::Db(e) => write!(f, "database: {e}"),
            PasswordError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PasswordError {}

impl From<sqlx::Error> for PasswordError {
    fn from(e: sqlx::Error) -> Self {
        PasswordError::Db(e)
    }
}

/// `PasswordHasher.hash_password`: the algorithm string
/// `$pbkdf2-sha256$i=<iterations>,l=32$`, the salt's characters as bytes,
/// lowercase hex out.
pub fn hash_password(password: &str, salt: &str, algorithm: &str) -> Result<String, PasswordError> {
    let parts: Vec<&str> = algorithm.trim_matches('$').split('$').collect();
    let [id, params] = parts[..] else {
        return Err(PasswordError::Unsupported(Unsupported(
            "password algorithm string not in $id$params$ form",
        )));
    };
    if id != "pbkdf2-sha256" {
        return Err(PasswordError::Unsupported(Unsupported(
            "password algorithms other than pbkdf2-sha256",
        )));
    }
    let mut iterations: u32 = 0;
    let mut length: usize = 0;
    for pair in params.split(',') {
        match pair.split_once('=') {
            Some(("i", v)) => iterations = v.parse().unwrap_or(0),
            Some(("l", v)) => length = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    if iterations < 1 || length != 32 {
        return Err(PasswordError::Unsupported(Unsupported(
            "pbkdf2 parameters other than i>=1, l=32",
        )));
    }
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt.as_bytes(), iterations, &mut out);
    Ok(out.iter().map(|b| format!("{b:02x}")).collect())
}

/// A `user_auth_tokens` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuthToken {
    pub id: i32,
    pub user_id: i32,
    pub auth_token: String,
    pub prev_auth_token: String,
    pub auth_token_seen: bool,
    pub rotated_at: NaiveDateTime,
    pub authenticated_with_oauth: Option<bool>,
}

/// `UserAuthToken.generate!`: a new row and its unhashed token for the
/// cookie, plus the always-written "generate" log line.
pub async fn generate(
    conn: &mut PgConnection,
    user_id: i32,
    secret_key_base: &str,
    user_agent: Option<&str>,
    client_ip: &str,
    path: Option<&str>,
) -> Result<(AuthToken, String), sqlx::Error> {
    let unhashed = new_unhashed_token();
    let hashed = hash_token(&unhashed, secret_key_base);
    let row: AuthToken = sqlx::query_as(
        "INSERT INTO user_auth_tokens (user_id, user_agent, client_ip, auth_token, prev_auth_token, \
                                       rotated_at, authenticated_with_oauth, created_at, updated_at) \
         VALUES ($1, $2, $3::inet, $4, $4, now(), FALSE, now(), now()) \
         RETURNING id, user_id, auth_token, prev_auth_token, auth_token_seen, rotated_at, authenticated_with_oauth",
    )
    .bind(user_id)
    .bind(user_agent)
    .bind(client_ip)
    .bind(&hashed)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO user_auth_token_logs (action, user_auth_token_id, user_id, user_agent, client_ip, path, auth_token, created_at) \
         VALUES ('generate', $1, $2, $3, $4::inet, $5, $6, now())",
    )
    .bind(row.id)
    .bind(user_id)
    .bind(user_agent)
    .bind(client_ip)
    .bind(path)
    .bind(&hashed)
    .execute(&mut *conn)
    .await?;
    Ok((row, unhashed))
}

/// `UserAuthToken.enforce_session_count_limit!`: at most 60 live sessions.
pub async fn enforce_session_count_limit(
    conn: &mut PgConnection,
    user_id: i32,
    max_session_age_hours: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM user_auth_tokens WHERE id IN ( \
           SELECT id FROM user_auth_tokens \
           WHERE user_id = $1 AND rotated_at > now() - ($2 * interval '1 hour') \
           ORDER BY rotated_at DESC OFFSET 60)",
    )
    .bind(user_id)
    .bind(max_session_age_hours as f64)
    .execute(conn)
    .await?;
    Ok(())
}

/// `UserAuthToken.lookup(unhashed, seen: true)`: the live row for a cookie
/// token, marking it seen; a row reached through its previous token after
/// rotation flips `auth_token_seen` off so the next request rotates again.
pub async fn lookup(
    conn: &mut PgConnection,
    unhashed: &str,
    secret_key_base: &str,
    max_session_age_hours: i64,
) -> Result<Option<AuthToken>, sqlx::Error> {
    let hashed = hash_token(unhashed, secret_key_base);
    let row: Option<AuthToken> = sqlx::query_as(
        "SELECT id, user_id, auth_token, prev_auth_token, auth_token_seen, rotated_at, authenticated_with_oauth \
         FROM user_auth_tokens \
         WHERE rotated_at > now() - ($1 * interval '1 hour') AND (auth_token = $2 OR prev_auth_token = $2) \
         LIMIT 1",
    )
    .bind(max_session_age_hours as f64)
    .bind(&hashed)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(mut row) = row else {
        return Ok(None);
    };
    if row.auth_token != hashed && row.prev_auth_token == hashed && row.auth_token_seen {
        sqlx::query(
            "UPDATE user_auth_tokens SET auth_token_seen = false \
             WHERE id = $1 AND prev_auth_token = $2 AND rotated_at < now() - interval '1 minute'",
        )
        .bind(row.id)
        .bind(&hashed)
        .execute(&mut *conn)
        .await?;
    }
    if !row.auth_token_seen && row.auth_token == hashed {
        let changed = sqlx::query(
            "UPDATE user_auth_tokens SET auth_token_seen = true, seen_at = now() \
             WHERE id = $1 AND auth_token = $2",
        )
        .bind(row.id)
        .bind(&hashed)
        .execute(&mut *conn)
        .await?;
        if changed.rows_affected() == 1 {
            row.auth_token_seen = true;
        }
    }
    Ok(Some(row))
}

/// `UserAuthToken#rotate!`: a new token when the current one was seen (or
/// is stale enough); returns the new unhashed token.
pub async fn rotate(
    conn: &mut PgConnection,
    row: &AuthToken,
    secret_key_base: &str,
    user_agent: Option<&str>,
    client_ip: &str,
    path: &str,
) -> Result<Option<String>, sqlx::Error> {
    let unhashed = new_unhashed_token();
    let hashed = hash_token(&unhashed, secret_key_base);
    let changed = sqlx::query(
        "UPDATE user_auth_tokens \
         SET auth_token_seen = false, seen_at = NULL, user_agent = $2, client_ip = $3::inet, \
             prev_auth_token = CASE WHEN auth_token_seen THEN auth_token ELSE prev_auth_token END, \
             auth_token = $4, rotated_at = now() \
         WHERE id = $1 AND (auth_token_seen OR rotated_at < now() - interval '30 seconds')",
    )
    .bind(row.id)
    .bind(user_agent)
    .bind(client_ip)
    .bind(&hashed)
    .execute(&mut *conn)
    .await?;
    if changed.rows_affected() != 1 {
        return Ok(None);
    }
    sqlx::query(
        "INSERT INTO user_auth_token_logs (action, user_auth_token_id, user_id, user_agent, client_ip, path, auth_token, created_at) \
         VALUES ('rotate', $1, $2, $3, $4::inet, $5, $6, now())",
    )
    .bind(row.id)
    .bind(row.user_id)
    .bind(user_agent)
    .bind(client_ip)
    .bind(path)
    .bind(&hashed)
    .execute(&mut *conn)
    .await?;
    Ok(Some(unhashed))
}

/// Whether `refresh_session` rotates: ten minutes after a seen rotation,
/// one minute after an unseen one.
pub fn needs_rotation(row: &AuthToken) -> bool {
    let age = crate::clock::now_naive() - row.rotated_at;
    if row.auth_token_seen {
        age > chrono::Duration::minutes(10)
    } else {
        age > chrono::Duration::minutes(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_tokens_like_rails() {
        // Digest::SHA1.base64digest("ab") == "2iNhTgJGmg18e9G9q1ycR0sZBNw="
        // with an empty secret the token alone is hashed.
        assert_eq!(hash_token("ab", ""), "2iNhTgJGmg18e9G9q1ycR0sZBNw=");
        assert_eq!(hash_token("a", "b"), hash_token("ab", ""));
        assert_eq!(new_unhashed_token().len(), 32);
    }

    #[test]
    fn hashes_passwords_with_the_stored_parameters() {
        // OpenSSL::KDF.pbkdf2_hmac("password", salt: "salt", iterations: 1,
        // length: 32, hash: "sha256") is the RFC 7914 test vector.
        assert_eq!(
            hash_password("password", "salt", "$pbkdf2-sha256$i=1,l=32$").unwrap(),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert!(hash_password("x", "salt", "$bcrypt$x$").is_err());
        assert!(hash_password("x", "salt", "$pbkdf2-sha256$i=1,l=16$").is_err());
    }
}
