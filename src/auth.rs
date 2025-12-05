use bcrypt::{hash, verify, DEFAULT_COST};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

const JWT_SECRET: &str = "your-secret-key-change-this-in-production";
const JWT_EXPIRATION_HOURS: i64 = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub user_id: i32,
    pub username: String,
    pub exp: i64,
}

pub fn hash_password(password: &str) -> Result<String, bcrypt::BcryptError> {
    hash(password, DEFAULT_COST)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, bcrypt::BcryptError> {
    verify(password, hash)
}

pub fn generate_token(user_id: i32, username: String) -> Result<String, jsonwebtoken::errors::Error> {
    let expiration = Utc::now()
        .checked_add_signed(Duration::hours(JWT_EXPIRATION_HOURS))
        .expect("valid timestamp")
        .timestamp();

    let claims = Claims {
        user_id,
        username,
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET.as_ref()),
    )
}

pub fn verify_token(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET.as_ref()),
        &Validation::default(),
    )?;

    Ok(token_data.claims)
}
