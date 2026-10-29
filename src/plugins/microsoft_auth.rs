//! discourse-microsoft-auth: Login with Microsoft (MicrosoftAuthenticator
//! and the plugin's OmniAuth::Strategies::MicrosoftOffice365). An OAuth2
//! code flow against the Microsoft identity platform's v2.0 endpoints for
//! the configured tenant, whose user is Microsoft Graph's /me, and whose
//! emails are verified only when the site says all of them are.
//!
//! MicrosoftAuthRevoker (the `microsoft_auth:revoke` and
//! `microsoft_auth:log_out_users` rake tasks) is `discourse-rs
//! microsoft-auth revoke` / `log-out-users` (src/cli.rs).
//!
//! The plugin's migrations (2021) carried the old office365 settings and
//! oauth2_user_infos rows over; the vendored schema records them as run,
//! and a database from before them went through Rails' migrations first.

use serde_json::{Map, Value};
use sqlx::PgConnection;

use crate::auth::managed::{AuthHash, Behaviour};
use crate::auth::oauth2::{AccessToken, AuthScheme, Client, Failure, Http};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

pub const NAME: &str = "microsoft_office365";

const SITE: &str = "https://login.microsoftonline.com";
const ME_URL: &str = "https://graph.microsoft.com/v1.0/me";
/// The strategy's DEFAULT_SCOPE.
const DEFAULT_SCOPE: &str = "openid email profile https://graph.microsoft.com/User.Read";
/// Net::HTTP's open and read timeouts: Faraday sets none.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

/// The strategy's client options, as the authenticator's setup lambda
/// leaves them: the tenant's authorize and token paths, the client's
/// credentials in a Basic header (oauth2's default auth scheme).
pub fn client(s: &SiteSettings) -> Result<Client, AppError> {
    let tenant = setting(s, "microsoft_auth_tenant_id")?;
    Ok(Client {
        client_id: setting(s, "microsoft_auth_client_id")?,
        client_secret: setting(s, "microsoft_auth_client_secret")?,
        site: Some(SITE.to_string()),
        authorize_url: format!("/{tenant}/oauth2/v2.0/authorize"),
        token_url: format!("/{tenant}/oauth2/v2.0/token"),
        token_method: reqwest::Method::POST,
        auth_scheme: AuthScheme::BasicAuth,
        token_headers: Vec::new(),
        token_params: Vec::new(),
        final_destination: false,
        timeout: TIMEOUT,
    })
}

/// The strategy's `authorize_params`: `display`, `score` and `auth_type`
/// from the request, then the default scope.
pub fn authorize_params(request: &Map<String, Value>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for k in ["display", "score", "auth_type"] {
        if let Some(v) = crate::params::string(request, k) {
            out.push((k.to_string(), v));
        }
    }
    out.push(("scope".into(), DEFAULT_SCOPE.into()));
    out
}

/// `raw_info[key]` of a parsed response that may not be a hash.
fn at<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&Value::Null)
}

/// The strategy's auth hash: Graph's /me with the bearer token. The
/// parsed response is a SnakyHash, so displayName reads as display_name.
pub async fn auth_hash(
    http: &Http<'_>,
    client: &Client,
    token: &AccessToken,
) -> Result<Result<AuthHash, Failure>, AppError> {
    let raw_info = match http
        .get_with_token(client, &token.token, ME_URL, &[])
        .await?
    {
        Ok(v) => v,
        Err(f) => return Ok(Err(f)),
    };
    let or = |a: &str, b: &str| match at(&raw_info, a) {
        Value::Null | Value::Bool(false) => at(&raw_info, b).clone(),
        other => other.clone(),
    };
    let mut info = Map::new();
    info.insert("name".into(), or("display_name", "user_principal_name"));
    info.insert("email".into(), or("mail", "user_principal_name"));
    let uid = match at(&raw_info, "id") {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut extra = Map::new();
    extra.insert("raw_info".into(), raw_info.clone());
    Ok(Ok(AuthHash {
        provider: NAME.into(),
        uid,
        info,
        credentials: token.credentials(),
        extra,
    }))
}

/// MicrosoftAuthenticator over ManagedAuthenticator's defaults:
/// `primary_email_verified?` is the site setting.
pub fn behaviour(s: &SiteSettings) -> Result<Behaviour, AppError> {
    Ok(Behaviour {
        email_verified: s.get("microsoft_auth_email_verified")?.truthy(),
        match_by_email: true,
        overrides_email: false,
        can_connect: true,
        user_by_email: None,
    })
}

/// A login's association needs a provider uid (`provider_uid` can't be
/// blank, a 500 in Rails).
pub fn check_uid(auth: &AuthHash) -> Result<(), AppError> {
    if auth.uid.is_empty() {
        return Err(Unsupported(
            "a Microsoft login without a user id (provider_uid can't be blank)",
        )
        .into());
    }
    Ok(())
}

const ASSOCIATED_USERS: &str =
    "SELECT user_id FROM user_associated_accounts WHERE provider_name = 'microsoft_office365'";

/// MicrosoftAuthRevoker.revoke: every user who logged in with Microsoft
/// deactivated and logged out, their user API keys and the API keys they
/// created revoked, and the Microsoft associations removed.
pub async fn revoke(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "UPDATE users SET active = false WHERE users.id IN ({ASSOCIATED_USERS})"
    ))
    .execute(&mut *conn)
    .await?;
    log_out_users(conn).await?;
    sqlx::query(&format!(
        "UPDATE user_api_keys SET revoked_at = NOW() WHERE user_id IN ({ASSOCIATED_USERS})"
    ))
    .execute(&mut *conn)
    .await?;
    sqlx::query(&format!(
        "UPDATE api_keys SET revoked_at = NOW() WHERE created_by_id IN ({ASSOCIATED_USERS})"
    ))
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM user_associated_accounts WHERE provider_name = 'microsoft_office365'")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// MicrosoftAuthRevoker.log_out_users: their auth tokens deleted.
pub async fn log_out_users(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "DELETE FROM user_auth_tokens WHERE user_id IN ({ASSOCIATED_USERS})"
    ))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_params_come_before_the_default_scope() {
        let request: Map<String, Value> = serde_json::from_value(serde_json::json!({
            "display": "page", "prompt": "login", "auth_type": "x"
        }))
        .unwrap();
        assert_eq!(
            authorize_params(&request),
            vec![
                ("display".to_string(), "page".to_string()),
                ("auth_type".to_string(), "x".to_string()),
                ("scope".to_string(), DEFAULT_SCOPE.to_string()),
            ]
        );
    }
}
