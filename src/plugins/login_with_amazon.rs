//! discourse-login-with-amazon: Login with Amazon
//! (Auth::LoginWithAmazonAuthenticator and the plugin's
//! OmniAuth::Strategies::Amazon). An OAuth2 code flow against Amazon's
//! authorize and token endpoints, the scope a site setting, whose user is
//! Amazon's profile; the authenticator's settings decide whether emails
//! are verified, whether the login overrides the account's email, and
//! whether users may connect and revoke it.

use serde_json::{Map, Value};

use crate::auth::managed::{AuthHash, Behaviour};
use crate::auth::oauth2::{AccessToken, AuthScheme, Client, Failure, Http};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

pub const NAME: &str = "amazon";

/// Net::HTTP's open and read timeouts: Faraday sets none.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

/// The strategy's client options, the credentials the setup lambda sets
/// (in a Basic header, oauth2's default auth scheme).
pub fn client(s: &SiteSettings) -> Result<Client, AppError> {
    Ok(Client {
        client_id: setting(s, "login_with_amazon_client_id")?,
        client_secret: setting(s, "login_with_amazon_client_secret")?,
        site: Some("https://www.amazon.com/".to_string()),
        authorize_url: "https://www.amazon.com/ap/oa".into(),
        token_url: "https://api.amazon.com/auth/o2/token".into(),
        token_method: reqwest::Method::POST,
        auth_scheme: AuthScheme::BasicAuth,
        token_headers: Vec::new(),
        token_params: Vec::new(),
        final_destination: false,
        timeout: TIMEOUT,
    })
}

/// The request phase's params: the setup lambda's `scope` option, which
/// replaces the strategy's own (`profile postal_code`).
pub fn authorize_params(s: &SiteSettings) -> Result<Vec<(String, String)>, AppError> {
    Ok(vec![(
        "scope".into(),
        setting(s, "login_with_amazon_scope")?,
    )])
}

/// The strategy's `callback_url`: REDIRECT_URL_ORIGIN when set, else the
/// request's host, with the subfolder and the callback path.
pub fn callback_url(base_url_no_prefix: &str, script_name: &str) -> String {
    let origin =
        std::env::var("REDIRECT_URL_ORIGIN").unwrap_or_else(|_| base_url_no_prefix.to_string());
    format!("{origin}{script_name}/auth/amazon/callback")
}

/// The strategy's `build_access_token`: the callback without its query as
/// the redirect, and the client's id and secret in the form as well.
pub async fn build_access_token(
    http: &Http<'_>,
    client: &Client,
    params: &Map<String, Value>,
    callback: &str,
) -> Result<Result<AccessToken, Failure>, AppError> {
    let code = crate::params::string(params, "code").unwrap_or_default();
    let redirect = callback.split('?').next().unwrap_or_default();
    let extra = [
        ("client_id".to_string(), client.client_id.clone()),
        ("client_secret".to_string(), client.client_secret.clone()),
    ];
    http.get_token(client, &code, redirect, &extra).await
}

/// `raw_info[key]` of a parsed response that may not be a hash.
fn at<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&Value::Null)
}

/// The strategy's auth hash: GET /ap/user/profile on the client's site
/// with the token in the query. The parsed response is a SnakyHash, so
/// `Profile.CustomerId` reads as `profile.customer_id`.
pub async fn auth_hash(
    http: &Http<'_>,
    client: &Client,
    token: &AccessToken,
) -> Result<Result<AuthHash, Failure>, AppError> {
    let raw_info = match http
        .resource(
            client,
            reqwest::Method::GET,
            "/ap/user/profile",
            &[("access_token".into(), token.token.clone())],
            &[],
            None,
        )
        .await?
    {
        Ok(v) => v,
        Err(f) => return Ok(Err(f)),
    };
    let profile = at(&raw_info, "profile");
    if !profile.is_object() {
        return Err(Unsupported("an Amazon profile without its Profile (NoMethodError)").into());
    }
    let uid = match at(profile, "customer_id") {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut extra = Map::new();
    extra.insert("postal_code".into(), at(profile, "postal_code").clone());
    let mut info = Map::new();
    info.insert("email".into(), at(profile, "primary_email").clone());
    info.insert("name".into(), at(profile, "name").clone());
    Ok(Ok(AuthHash {
        provider: NAME.into(),
        uid,
        info,
        credentials: token.credentials(),
        extra,
    }))
}

/// Auth::LoginWithAmazonAuthenticator over ManagedAuthenticator's
/// defaults: `primary_email_verified?`, `always_update_user_email?` and
/// `can_connect_existing_user?` from the site settings.
pub fn behaviour(s: &SiteSettings) -> Result<Behaviour, AppError> {
    Ok(Behaviour {
        email_verified: s.get("login_with_amazon_email_verified")?.truthy(),
        match_by_email: true,
        overrides_email: s.get("login_with_amazon_overrides_email")?.truthy(),
        can_connect: s.get("login_with_amazon_user_can_connect")?.truthy(),
        user_by_email: None,
    })
}

/// A login's association needs a provider uid (`provider_uid` can't be
/// blank, a 500 in Rails).
pub fn check_uid(auth: &AuthHash) -> Result<(), AppError> {
    if auth.uid.is_empty() {
        return Err(Unsupported(
            "an Amazon login without a customer id (provider_uid can't be blank)",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_callback_is_the_request_origin_without_redirect_url_origin() {
        if std::env::var("REDIRECT_URL_ORIGIN").is_err() {
            assert_eq!(
                callback_url("https://f.x", "/forum"),
                "https://f.x/forum/auth/amazon/callback"
            );
        }
    }
}
