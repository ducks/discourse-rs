//! Securing a database freshly loaded from seed/*.sql, the parity
//! fixture's install: its secrets are in the repo and its users share the
//! password "password". [`secure_seed_sql`] is the SQL that, run in the
//! same transaction as the load, gives the site secrets of its own, the
//! admin a password of the operator's, and locks the fixture users out.
//! src/bin/discourse-rs-fresh-site.rs prints it for the NixOS module's
//! discourse-rs-db-init.

use base64::Engine;

/// The seed's members, all with the password "password".
pub const FIXTURE_USERS: [&str; 4] = ["user0", "user1", "user2", "user3"];

/// The seed's admin.
pub const ADMIN_USER_ID: i32 = 1;

/// Rails' `min_admin_password_length` default (config/site_settings.yml).
pub const MIN_ADMIN_PASSWORD_LENGTH: usize = 15;

/// A VAPID key pair as `WebPush.generate_key` gives it and the push
/// notifications initializer stores it: the uncompressed P-256 public
/// point and the private scalar, URL-safe base64 with padding, and
/// `vapid_public_key_bytes`, the public key's bytes joined by `|`.
pub struct VapidKeys {
    pub public_key: String,
    pub private_key: String,
    pub public_key_bytes: String,
}

impl VapidKeys {
    pub fn generate() -> Self {
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        let secret = p256::SecretKey::random(&mut rand::rngs::OsRng);
        let public = secret.public_key().to_encoded_point(false);
        let b64 = base64::engine::general_purpose::URL_SAFE;
        VapidKeys {
            public_key: b64.encode(public.as_bytes()),
            private_key: b64.encode(secret.to_bytes()),
            public_key_bytes: public
                .as_bytes()
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join("|"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum FreshSiteError {
    PasswordTooShort,
    Hash(String),
}

impl std::fmt::Display for FreshSiteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FreshSiteError::PasswordTooShort => write!(
                f,
                "the admin password must be at least {MIN_ADMIN_PASSWORD_LENGTH} characters"
            ),
            FreshSiteError::Hash(e) => write!(f, "hashing the admin password: {e}"),
        }
    }
}

impl std::error::Error for FreshSiteError {}

/// What the operator gives a fresh site.
pub struct FreshSite<'a> {
    pub admin_password: &'a str,
    /// The admin's address, in place of the seed's admin@example.com.
    pub admin_email: Option<&'a str>,
    /// `Discourse.base_url`, stored as `vapid_base_url` with the keys.
    pub base_url: &'a str,
}

/// A SQL string literal.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The SQL that secures a database just loaded from the seed:
///
/// - new VAPID keys and push_api_secret_key (the seed's are public);
/// - the admin's password, hashed as `user.password=` hashes it
///   ([`crate::accounts::PASSWORD_ALGORITHM`], a fresh salt), and
///   optionally its address;
/// - every other password removed (system, discobot and the fixture
///   users), so only the admin can log in with one;
/// - the fixture users deactivated and suspended, so a password reset
///   can't bring them back either;
/// - the seed's tokens and keys dropped: sessions, API keys, invites,
///   email tokens and unsubscribe keys (unsubscribe keys are stored raw).
pub fn secure_seed_sql(site: &FreshSite<'_>) -> Result<String, FreshSiteError> {
    if site.admin_password.chars().count() < MIN_ADMIN_PASSWORD_LENGTH {
        return Err(FreshSiteError::PasswordTooShort);
    }
    let salt = crate::accounts::random_hex();
    let hash = crate::session::token::hash_password(
        site.admin_password,
        &salt,
        crate::accounts::PASSWORD_ALGORITHM,
    )
    .map_err(|e| FreshSiteError::Hash(e.to_string()))?;
    let vapid = VapidKeys::generate();
    let push_secret = crate::accounts::random_hex();

    let mut sql = String::new();
    for (name, value) in [
        ("vapid_public_key", vapid.public_key.as_str()),
        ("vapid_private_key", vapid.private_key.as_str()),
        ("vapid_public_key_bytes", vapid.public_key_bytes.as_str()),
        ("vapid_base_url", site.base_url),
        ("push_api_secret_key", push_secret.as_str()),
    ] {
        // data_type 1: a string.
        sql.push_str(&format!(
            "INSERT INTO public.site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ({}, 1, {}, now(), now()) \
             ON CONFLICT (name) DO UPDATE SET value = EXCLUDED.value, updated_at = now();\n",
            quote(name),
            quote(value)
        ));
    }
    sql.push_str(&format!(
        "DELETE FROM public.user_passwords WHERE user_id <> {ADMIN_USER_ID};\n\
         INSERT INTO public.user_passwords (user_id, password_hash, password_salt, password_algorithm, created_at, updated_at) \
         VALUES ({ADMIN_USER_ID}, {}, {}, {}, now(), now()) \
         ON CONFLICT (user_id) DO UPDATE SET password_hash = EXCLUDED.password_hash, \
           password_salt = EXCLUDED.password_salt, password_algorithm = EXCLUDED.password_algorithm, \
           password_expired_at = NULL, updated_at = now();\n",
        quote(&hash),
        quote(&salt),
        quote(crate::accounts::PASSWORD_ALGORITHM),
    ));
    if let Some(email) = site.admin_email {
        sql.push_str(&format!(
            "UPDATE public.user_emails SET email = {}, updated_at = now() \
             WHERE user_id = {ADMIN_USER_ID} AND \"primary\";\n",
            quote(&email.trim().to_lowercase())
        ));
    }
    let fixtures = FIXTURE_USERS
        .iter()
        .map(|u| quote(u))
        .collect::<Vec<_>>()
        .join(", ");
    sql.push_str(&format!(
        "UPDATE public.users SET active = false, suspended_at = now(), \
           suspended_till = now() + interval '1000 years', updated_at = now() \
         WHERE username_lower IN ({fixtures});\n"
    ));
    for table in [
        "user_auth_tokens",
        "user_api_keys",
        "api_keys",
        "invites",
        "email_tokens",
        "unsubscribe_keys",
    ] {
        sql.push_str(&format!("DELETE FROM public.{table};\n"));
    }
    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";

    fn site() -> FreshSite<'static> {
        FreshSite {
            admin_password: PASSWORD,
            admin_email: Some("Admin@Example.org"),
            base_url: "https://forum.example.org",
        }
    }

    #[test]
    fn vapid_keys_are_webpush_shaped() {
        let keys = VapidKeys::generate();
        let b64 = base64::engine::general_purpose::URL_SAFE;
        let public = b64.decode(&keys.public_key).unwrap();
        assert_eq!(public.len(), 65);
        assert_eq!(public[0], 4, "an uncompressed point");
        assert_eq!(b64.decode(&keys.private_key).unwrap().len(), 32);
        let bytes: Vec<u8> = keys
            .public_key_bytes
            .split('|')
            .map(|b| b.parse().unwrap())
            .collect();
        assert_eq!(bytes, public);
        assert_ne!(keys.public_key, VapidKeys::generate().public_key);
    }

    #[test]
    fn the_admin_hash_verifies_with_the_login_check() {
        let sql = secure_seed_sql(&site()).unwrap();
        let values = sql
            .lines()
            .find(|l| l.contains("INSERT INTO public.user_passwords"))
            .unwrap();
        let quoted: Vec<&str> = values.split('\'').skip(1).step_by(2).collect();
        let (hash, salt, algorithm) = (quoted[0], quoted[1], quoted[2]);
        assert_eq!(algorithm, crate::accounts::PASSWORD_ALGORITHM);
        assert_eq!(
            crate::session::token::hash_password(PASSWORD, salt, algorithm).unwrap(),
            hash
        );
        assert_ne!(
            crate::session::token::hash_password("password", salt, algorithm).unwrap(),
            hash
        );
    }

    #[test]
    fn secrets_and_fixtures_are_covered() {
        let sql = secure_seed_sql(&site()).unwrap();
        for name in [
            "vapid_public_key",
            "vapid_private_key",
            "vapid_public_key_bytes",
            "push_api_secret_key",
        ] {
            assert!(sql.contains(&format!("'{name}'")), "{name}");
        }
        assert!(sql.contains("'admin@example.org'"));
        assert!(sql.contains("'user0', 'user1', 'user2', 'user3'"));
        assert!(sql.contains("DELETE FROM public.unsubscribe_keys;"));
    }

    #[test]
    fn a_short_password_is_refused() {
        let short = FreshSite {
            admin_password: "password",
            ..site()
        };
        assert_eq!(
            secure_seed_sql(&short).err(),
            Some(FreshSiteError::PasswordTooShort)
        );
    }

    #[test]
    fn values_are_quoted() {
        assert_eq!(quote("it's"), "'it''s'");
    }
}
