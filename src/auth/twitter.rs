//! Twitter's login: Auth::TwitterAuthenticator over omniauth-twitter,
//! omniauth-oauth and the oauth gem's OAuth 1.0a consumer (HMAC-SHA1
//! signed requests to https://api.x.com: a request token, the user sent
//! to authenticate it, the verifier exchanged for an access token, the
//! account read from verify_credentials), and the authenticator's
//! `healthy?` that the twitter_login problem check runs.

use serde_json::{Map, Value, json};

use crate::auth::managed::AuthHash;
use crate::auth::oauth2::{Failure, Http, escape};
use crate::{AppError, Unsupported};

/// The strategy's `client_options[:site]`.
pub const SITE: &str = "https://api.x.com";
/// omniauth-oauth's open and read timeouts.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// `OAuth::Client::Helper#amend_user_agent_header` over Net::HTTP's "Ruby".
const USER_AGENT: &str = "OAuth gem v1.1.8";

/// The consumer's key and secret.
pub struct Consumer {
    pub key: String,
    pub secret: String,
}

/// A token and its secret (request or access).
#[derive(Debug, Clone)]
pub struct Token {
    pub token: String,
    pub secret: String,
    /// The rest of the token response (`params`).
    pub params: Vec<(String, String)>,
}

/// `OAuth::Helper.generate_key`: 32 random bytes in Base64, non-word
/// characters dropped.
fn nonce() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::STANDARD
        .encode(bytes)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// `RequestProxy::Base#normalized_uri`: scheme and host lowercased, a
/// port that isn't the scheme's, the path ("/" when empty).
fn normalized_uri(url: &reqwest::Url) -> String {
    let scheme = url.scheme().to_lowercase();
    let host = url.host_str().unwrap_or_default().to_lowercase();
    let port = match url.port() {
        Some(p) if !((scheme == "http" && p == 80) || (scheme == "https" && p == 443)) => {
            format!(":{p}")
        }
        _ => String::new(),
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    format!("{scheme}://{host}{port}{path}")
}

/// `OAuth::Signature::HMAC::SHA1`: the base string (method, URI and the
/// sorted, escaped params) signed with the consumer's and token's
/// escaped secrets.
fn sign(
    method: &str,
    url: &reqwest::Url,
    params: &[(String, String)],
    consumer_secret: &str,
    token_secret: &str,
) -> String {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut pairs: Vec<(String, String)> =
        params.iter().map(|(k, v)| (escape(k), escape(v))).collect();
    pairs.sort();
    let normalized = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let base = [method, &normalized_uri(url), &normalized]
        .iter()
        .map(|v| escape(v))
        .collect::<Vec<_>>()
        .join("&");
    let key = format!("{}&{}", escape(consumer_secret), escape(token_secret));
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(key.as_bytes()).expect("any key length");
    mac.update(base.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// The `Authorization: OAuth ...` header for a request: the oauth params
/// (empty ones left out), signed over them with the query and form
/// params, sorted.
fn authorization(
    method: &str,
    url: &reqwest::Url,
    form: &[(String, String)],
    consumer: &Consumer,
    token: Option<&Token>,
    extra: &[(&str, &str)],
) -> String {
    let timestamp = crate::clock::now().timestamp().to_string();
    let nonce = nonce();
    let mut oauth: Vec<(String, String)> = Vec::new();
    for (k, v) in extra.iter().filter(|(k, _)| *k == "oauth_callback") {
        oauth.push((k.to_string(), v.to_string()));
    }
    oauth.push(("oauth_consumer_key".into(), consumer.key.clone()));
    if let Some(t) = token {
        oauth.push(("oauth_token".into(), t.token.clone()));
    }
    oauth.push(("oauth_signature_method".into(), "HMAC-SHA1".into()));
    oauth.push(("oauth_timestamp".into(), timestamp));
    oauth.push(("oauth_nonce".into(), nonce));
    for (k, v) in extra.iter().filter(|(k, _)| *k == "oauth_verifier") {
        oauth.push((k.to_string(), v.to_string()));
    }
    oauth.push(("oauth_version".into(), "1.0".into()));
    oauth.retain(|(_, v)| !v.is_empty());
    let mut signed = oauth.clone();
    signed.extend(
        url.query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned())),
    );
    signed.extend(form.iter().cloned());
    let token_secret = token.map(|t| t.secret.as_str()).unwrap_or("");
    let signature = sign(method, url, &signed, &consumer.secret, token_secret);
    oauth.push(("oauth_signature".into(), signature));
    oauth.sort();
    let params = oauth
        .iter()
        .map(|(k, v)| format!("{k}=\"{}\"", escape(v)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("OAuth {params}")
}

/// What a token request ended in, as the strategy rescues it.
pub enum Outcome<T> {
    Ok(T),
    /// A 4xx: OAuth::Unauthorized.
    Unauthorized,
    /// A 5xx: Net::HTTPFatalError (`service_unavailable`).
    ServerError,
    /// No answer (`timeout`).
    Timeout(String),
}

/// `Consumer#token_request(:post, path, token, options, form)`: the
/// signed POST, a 2xx's form body read.
async fn token_request(
    http: &Http<'_>,
    consumer: &Consumer,
    path: &str,
    token: Option<&Token>,
    extra: &[(&str, &str)],
    form: &[(String, String)],
) -> Result<Outcome<Token>, AppError> {
    let url = reqwest::Url::parse(&format!("{SITE}{path}"))
        .map_err(|_| Unsupported("a Twitter URL that does not parse"))?;
    let header = authorization("POST", &url, form, consumer, token, extra);
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let fetched = match http
        .request(
            false,
            TIMEOUT,
            reqwest::Method::POST,
            url.as_str(),
            &[
                ("Authorization".into(), header),
                (
                    "Content-Type".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
            ],
            Some(body),
        )
        .await?
    {
        Ok(f) => f,
        Err(f) => return Ok(Outcome::Timeout(f.message)),
    };
    match fetched.status {
        200..=299 => {}
        300..=399 => return Err(Unsupported("a redirected Twitter token request").into()),
        400..=499 => return Ok(Outcome::Unauthorized),
        _ => return Ok(Outcome::ServerError),
    }
    // CGI.parse: the first value of each key.
    let mut params: Vec<(String, String)> = Vec::new();
    for (k, v) in form_urlencoded::parse(fetched.body.as_bytes()) {
        let k = k.trim().to_string();
        if !params.iter().any(|(key, _)| *key == k) {
            params.push((k, v.into_owned()));
        }
    }
    let get = |k: &str| {
        params
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    Ok(Outcome::Ok(Token {
        token: get("oauth_token"),
        secret: get("oauth_token_secret"),
        params,
    }))
}

/// `consumer.get_request_token({oauth_callback:}, request_params)`.
pub async fn request_token(
    http: &Http<'_>,
    consumer: &Consumer,
    callback: &str,
    form: &[(String, String)],
) -> Result<Outcome<Token>, AppError> {
    token_request(
        http,
        consumer,
        "/oauth/request_token",
        None,
        &[("oauth_callback", callback)],
        form,
    )
    .await
}

/// `request_token.get_access_token(oauth_verifier:)` (or the callback
/// again when the request token's wasn't confirmed).
pub async fn access_token(
    http: &Http<'_>,
    consumer: &Consumer,
    request_token: &Token,
    verifier: Option<&str>,
    callback: Option<&str>,
) -> Result<Outcome<Token>, AppError> {
    let mut extra: Vec<(&str, &str)> = Vec::new();
    if let Some(v) = verifier {
        extra.push(("oauth_verifier", v));
    }
    if let Some(c) = callback {
        extra.push(("oauth_callback", c));
    }
    token_request(
        http,
        consumer,
        "/oauth/access_token",
        Some(request_token),
        &extra,
        &[],
    )
    .await
}

/// `RequestToken#authorize_url(params)`: the authorize path on the site,
/// the params and the token in `URI.encode_www_form`.
pub fn authorize_url(path: &str, params: &[(String, String)], token: &str) -> String {
    let mut all: Vec<(String, String)> = params.to_vec();
    all.push(("oauth_token".into(), token.into()));
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(all)
        .finish();
    format!("{SITE}{path}?{query}")
}

/// The strategy's `raw_info`: verify_credentials read with the access
/// token, its body parsed whatever the status (`JSON.load`).
async fn raw_info(
    http: &Http<'_>,
    consumer: &Consumer,
    token: &Token,
) -> Result<Result<Value, Failure>, AppError> {
    let url = reqwest::Url::parse(&format!(
        "{SITE}/1.1/account/verify_credentials.json?include_entities=false&skip_status=true&include_email=true"
    ))
    .map_err(|_| Unsupported("a Twitter URL that does not parse"))?;
    let header = authorization("GET", &url, &[], consumer, Some(token), &[]);
    let fetched = match http
        .request(
            false,
            TIMEOUT,
            reqwest::Method::GET,
            url.as_str(),
            &[("Authorization".into(), header)],
            None,
        )
        .await?
    {
        Ok(f) => f,
        Err(f) => return Ok(Err(f)),
    };
    // JSON.load: nil for an empty body, a parse error is a 500.
    if fetched.body.trim().is_empty() {
        return Ok(Ok(Value::Null));
    }
    serde_json::from_str(&fetched.body)
        .map(Ok)
        .map_err(|_| Unsupported("a verify_credentials body that is not JSON").into())
}

/// The strategy's auth hash: the credentials, the account read, the
/// uid from the access token's user_id. TwitterAuthenticator keeps no
/// extra.
pub async fn auth_hash(
    http: &Http<'_>,
    consumer: &Consumer,
    token: &Token,
) -> Result<Result<AuthHash, Failure>, AppError> {
    let raw = match raw_info(http, consumer, token).await? {
        Ok(v) => v,
        Err(f) => return Ok(Err(f)),
    };
    let at = |k: &str| raw.get(k).cloned().unwrap_or(Value::Null);
    let screen_name = match at("screen_name") {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut info = Map::new();
    info.insert("nickname".into(), at("screen_name"));
    info.insert("name".into(), at("name"));
    info.insert("email".into(), at("email"));
    info.insert("location".into(), at("location"));
    info.insert("image".into(), at("profile_image_url"));
    info.insert("description".into(), at("description"));
    info.insert(
        "urls".into(),
        json!({
            "Website": at("url"),
            "Twitter": format!("https://twitter.com/{screen_name}"),
        }),
    );
    let uid = token
        .params
        .iter()
        .find(|(k, _)| k == "user_id")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    Ok(Ok(AuthHash {
        provider: "twitter".into(),
        uid,
        info,
        credentials: json!({"token": token.token, "secret": token.secret}),
        extra: Map::new(),
    }))
}

/// TwitterAuthenticator#healthy?: a request token for an out-of-band
/// callback. Unauthorized and server errors are unhealthy; no answer
/// raises (the check fails).
pub async fn healthy(http: &Http<'_>, consumer: &Consumer) -> Result<bool, AppError> {
    match request_token(http, consumer, "oob", &[]).await? {
        Outcome::Ok(_) => Ok(true),
        Outcome::Unauthorized | Outcome::ServerError => Ok(false),
        Outcome::Timeout(_) => {
            Err(Unsupported("Twitter's request token request failing to connect").into())
        }
    }
}

/// An Http for the consumer's requests, with the oauth gem's user agent.
pub fn http<'a>(
    settings: &'a crate::site_settings::SiteSettings,
    config: &'a crate::config::Config,
) -> Http<'a> {
    Http {
        fetcher: crate::final_destination::Fetcher {
            settings,
            urls: crate::url::Urls { config, settings },
        },
        user_agent: USER_AGENT.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_as_the_oauth_core_example() {
        // RFC 5849 section 3.4.1's example request, signed with its
        // client and token secrets.
        let url = reqwest::Url::parse("http://example.com/request?b5=%3D%253D&a3=a&c%40=&a2=r%20b")
            .unwrap();
        let mut params: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        params.extend(
            [
                ("oauth_consumer_key", "9djdj82h48djs9d2"),
                ("oauth_token", "kkk9d7dh3k39sjv7"),
                ("oauth_signature_method", "HMAC-SHA1"),
                ("oauth_timestamp", "137131201"),
                ("oauth_nonce", "7d8f3e4a"),
                ("c2", ""),
                ("a3", "2 q"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string())),
        );
        assert_eq!(
            sign("POST", &url, &params, "j49sk3j29djd", "dh893hdasih9"),
            "r6/TJjbCOr97/+UU0NsvSne7s5g="
        );
    }

    #[test]
    fn builds_the_authorize_url() {
        assert_eq!(
            authorize_url(
                "/oauth/authenticate",
                &[("force_login".into(), "true".into())],
                "tok en"
            ),
            "https://api.x.com/oauth/authenticate?force_login=true&oauth_token=tok+en"
        );
        assert_eq!(
            normalized_uri(&reqwest::Url::parse("HTTPS://Api.X.com:443/a?b=c").unwrap()),
            "https://api.x.com/a"
        );
    }
}
