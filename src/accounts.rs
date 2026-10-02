//! The account pieces the signup, activation, password and email login
//! endpoints share: finding a user by username or email, and EmailToken.

use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::AppError;

/// `EmailToken.scopes`
pub mod token_scopes {
    pub const SIGNUP: i32 = 1;
    pub const PASSWORD_RESET: i32 = 2;
    pub const EMAIL_LOGIN: i32 = 3;
}

/// `EmailToken.hash_token`
pub fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// `SecureRandom.hex`: 16 random bytes as hex.
pub fn random_hex() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `user.email_tokens.create!(email:, scope:)`: a fresh token (returned in
/// the clear, stored hashed), and the user's other tokens of the scope
/// expired (after_create).
pub async fn create_email_token(
    conn: &mut PgConnection,
    user_id: i32,
    email: &str,
    scope: i32,
) -> Result<String, AppError> {
    let token = random_hex();
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO email_tokens (user_id, email, token_hash, scope, expired, confirmed, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, FALSE, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(user_id)
    .bind(email.to_lowercase())
    .bind(hash_token(&token))
    .bind(scope)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE email_tokens SET expired = TRUE WHERE user_id = $1 AND (scope IS NULL OR scope = $2) AND id <> $3",
    )
    .bind(user_id)
    .bind(scope)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    Ok(token)
}

/// `User.normalize_username`: NFC, then lowercase. Only ASCII is ported,
/// where NFC changes nothing.
pub fn normalize_username(username: &str) -> Result<String, crate::Unsupported> {
    if !username.is_ascii() {
        return Err(crate::Unsupported(
            "non-ASCII usernames (unicode normalization)",
        ));
    }
    Ok(username.to_ascii_lowercase())
}

/// A user as the account endpoints read one.
#[derive(Debug, sqlx::FromRow)]
pub struct AccountUser {
    pub id: i32,
    pub username: String,
    pub staged: bool,
    pub active: bool,
    pub email: Option<String>,
}

/// `User.real.find_by_username_or_email(login)`: by any of the user's
/// emails when the login has an `@`, else by username.
pub async fn find_by_username_or_email(
    conn: &mut PgConnection,
    login: &str,
) -> Result<Option<AccountUser>, AppError> {
    let select = "SELECT u.id, u.username, u.staged, u.active, \
                  (SELECT email FROM user_emails WHERE user_id = u.id AND \"primary\" LIMIT 1) AS email \
                  FROM users u";
    let real = "u.id > 0 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = u.id)";
    let user = if login.contains('@') {
        sqlx::query_as(&format!(
            "{select} WHERE {real} AND u.id = (SELECT user_id FROM user_emails WHERE lower(email) = $1 LIMIT 1)"
        ))
        .bind(login.trim().to_lowercase())
        .fetch_optional(&mut *conn)
        .await?
    } else {
        sqlx::query_as(&format!("{select} WHERE {real} AND u.username_lower = $1"))
            .bind(normalize_username(login)?)
            .fetch_optional(&mut *conn)
            .await?
    };
    Ok(user)
}
