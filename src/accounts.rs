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

/// `EmailAddressValidator.valid_value?`, without the mail gem's decoding
/// (ASCII addresses decode to themselves).
pub fn valid_email(email: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~\-]+(?:\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~\-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?$",
        )
        .unwrap()
    });
    let Some(at) = email.find('@') else {
        return false;
    };
    at <= 64 && email.len() - at - 1 <= 255 && email.is_ascii() && RE.is_match(email)
}

/// `ScreenedIpAddress.actions[:block]`
const SCREENED_BLOCK: i32 = 1;

/// `ScreenedIpAddress.should_block?(ip)`: the most specific screening of
/// the address, its match recorded when it blocks.
pub async fn ip_blocked(conn: &mut PgConnection, ip: &str) -> Result<bool, AppError> {
    let screening: Option<(i32, i32)> = sqlx::query_as(
        "SELECT id, action_type FROM screened_ip_addresses WHERE $1::inet <<= ip_address \
         ORDER BY masklen(ip_address) DESC LIMIT 1",
    )
    .bind(ip)
    .fetch_optional(&mut *conn)
    .await?;
    match screening {
        Some((id, action)) if action == SCREENED_BLOCK => {
            sqlx::query(
                "UPDATE screened_ip_addresses SET match_count = match_count + 1, \
                 last_match_at = clock_timestamp(), updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// `EmailLoginCode.purposes`
pub mod code_purposes {
    pub const LOGIN: i32 = 0;
    pub const PASSWORD_RESET: i32 = 1;
}

/// `EmailLoginCode.hash_code`: HMAC-SHA256 under secret_key_base.
pub fn hash_login_code(code: &str, secret_key_base: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<Sha256>::new_from_slice(secret_key_base.as_bytes())
        .expect("HMAC takes any key length");
    mac.update(code.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `EmailLoginCode.generate!(email:, purpose:)`: a six-digit code valid
/// for ten minutes, replacing the address's earlier ones; (id, code).
pub async fn generate_login_code(
    conn: &mut PgConnection,
    email: &str,
    purpose: i32,
    secret_key_base: &str,
) -> Result<(i64, String), AppError> {
    use rand::Rng;
    let email = email.to_lowercase();
    let code = format!("{:06}", rand::thread_rng().gen_range(0..1_000_000));
    sqlx::query("DELETE FROM email_login_codes WHERE lower(email) = $1 AND purpose = $2")
        .bind(&email)
        .bind(purpose)
        .execute(&mut *conn)
        .await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO email_login_codes (email, purpose, code_hash, expires_at, attempts, created_at, updated_at) \
         VALUES ($1, $2, $3, clock_timestamp() + interval '10 minutes', 0, clock_timestamp(), clock_timestamp()) \
         RETURNING id",
    )
    .bind(&email)
    .bind(purpose)
    .bind(hash_login_code(&code, secret_key_base))
    .fetch_one(&mut *conn)
    .await?;
    Ok((id, code))
}

/// lib/common_passwords/10-char-common-passwords.txt
const COMMON_PASSWORDS: &str =
    include_str!("../vendor/discourse/lib/common_passwords/10-char-common-passwords.txt");

/// Who a password is checked against.
pub struct PasswordOwner<'a> {
    pub admin: bool,
    pub username: &'a str,
    pub name: Option<&'a str>,
    pub email: Option<&'a str>,
    /// (hash, salt, algorithm) of the current password, if any.
    pub current: Option<(&'a str, &'a str, &'a str)>,
}

/// `UserPasswordValidator`: the first rule a new password breaks, as the
/// error key (`too_short`, `common`, ...).
pub fn password_error(
    settings: &crate::site_settings::SiteSettings,
    password: &str,
    owner: &PasswordOwner<'_>,
) -> Result<Option<&'static str>, AppError> {
    let len = password.chars().count() as i64;
    let min = settings.get("min_password_length")?.to_i();
    let min_admin = settings.get("min_admin_password_length")?.to_i();
    if owner.admin && len < min_admin {
        return Ok(Some("too_short"));
    }
    if len < min {
        return Ok(Some("too_short"));
    }
    if !owner.username.is_empty() && password == owner.username {
        return Ok(Some("same_as_username"));
    }
    if owner.name.is_some_and(|n| !n.is_empty() && password == n) {
        return Ok(Some("same_as_name"));
    }
    if owner.email.is_some_and(|e| !e.is_empty() && password == e) {
        return Ok(Some("same_as_email"));
    }
    if let Some((hash, salt, algorithm)) = owner.current {
        if crate::session::token::hash_password(password, salt, algorithm)
            .ok()
            .as_deref()
            == Some(hash)
        {
            return Ok(Some("same_as_current"));
        }
    }
    if settings.get("block_common_passwords")?.truthy()
        && COMMON_PASSWORDS.lines().any(|l| l == password)
    {
        return Ok(Some("common"));
    }
    let mut unique: Vec<char> = password.chars().collect();
    unique.sort_unstable();
    unique.dedup();
    if (unique.len() as i64) < settings.get("password_unique_characters")?.to_i() {
        return Ok(Some("unique_characters"));
    }
    Ok(None)
}

/// `UserPassword::TARGET_PASSWORD_ALGORITHM`
pub const PASSWORD_ALGORITHM: &str = "$pbkdf2-sha256$i=600000,l=32$";

/// `user.password = pw; user.save`: a fresh salt and hash (PBKDF2 at
/// 600k iterations runs off the async threads).
pub async fn set_password(
    conn: &mut PgConnection,
    user_id: i32,
    password: &str,
) -> Result<(), AppError> {
    let salt = random_hex();
    let pw = password.to_string();
    let salt_for_hash = salt.clone();
    let hash = tokio::task::spawn_blocking(move || {
        crate::session::token::hash_password(&pw, &salt_for_hash, PASSWORD_ALGORITHM)
    })
    .await
    .map_err(|e| {
        crate::Unsupported(if e.is_panic() {
            "a panic hashing a password"
        } else {
            "a cancelled password hash"
        })
    })??;
    sqlx::query(
        "INSERT INTO user_passwords (user_id, password_hash, password_salt, password_algorithm, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp()) \
         ON CONFLICT (user_id) DO UPDATE SET password_hash = EXCLUDED.password_hash, \
           password_salt = EXCLUDED.password_salt, password_algorithm = EXCLUDED.password_algorithm, \
           password_expired_at = NULL, updated_at = clock_timestamp()",
    )
    .bind(user_id)
    .bind(hash)
    .bind(salt)
    .bind(PASSWORD_ALGORITHM)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
