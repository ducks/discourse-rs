//! discourse-apple-auth: Sign in with Apple (AppleAuthenticator and the
//! plugin's own OmniAuth::Strategies::Apple). An OAuth2 code flow whose
//! client secret is an ES256 JWT signed with the team's key, whose
//! callback arrives as a cross-site POST (sent back to itself as a GET so
//! the session cookie comes along), and whose user is the ID token's,
//! checked against Apple's keys (cached a day).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::auth::managed::AuthHash;
use crate::auth::oauth2::{AccessToken, AuthScheme, Client, Failure, Http};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

const SITE: &str = "https://appleid.apple.com";
const KEYS_URL: &str = "https://appleid.apple.com/auth/keys";
const JWKS_CACHE_KEY: &str = "sign-in-with-apple-jwks";
/// Net::HTTP's open and read timeouts.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `client_secret`: an ES256 JWT for the team, signed with its key, good
/// for a minute.
fn client_secret(s: &SiteSettings) -> Result<String, AppError> {
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::DecodePrivateKey;
    let pem = setting(s, "apple_pem")?;
    let key = p256::ecdsa::SigningKey::from_pkcs8_pem(&pem)
        .ok()
        .or_else(|| {
            p256::SecretKey::from_sec1_pem(&pem)
                .ok()
                .map(p256::ecdsa::SigningKey::from)
        })
        .ok_or(Unsupported(
            "an Apple key OpenSSL can't read (OpenSSL::PKey::ECError)",
        ))?;
    let now = crate::clock::now().timestamp();
    let header = json!({"kid": setting(s, "apple_key_id")?, "alg": "ES256"});
    let payload = json!({
        "iss": setting(s, "apple_team_id")?,
        "aud": SITE,
        "sub": setting(s, "apple_client_id")?,
        "iat": now,
        "exp": now + 60,
    });
    let input = format!(
        "{}.{}",
        b64url(header.to_string().as_bytes()),
        b64url(payload.to_string().as_bytes())
    );
    let signature: p256::ecdsa::Signature = key.sign(input.as_bytes());
    Ok(format!("{input}.{}", b64url(&signature.to_bytes())))
}

/// The strategy's client, its secret made for it (`ensure_client_secret`).
pub fn client(s: &SiteSettings) -> Result<Client, AppError> {
    Ok(Client {
        client_id: setting(s, "apple_client_id")?,
        client_secret: client_secret(s)?,
        site: Some(SITE.to_string()),
        authorize_url: "/auth/authorize".into(),
        token_url: "/auth/token".into(),
        token_method: reqwest::Method::POST,
        auth_scheme: AuthScheme::RequestBody,
        token_headers: Vec::new(),
        token_params: Vec::new(),
        final_destination: false,
        timeout: TIMEOUT,
    })
}

/// The strategy's `authorize_params` option.
pub fn authorize_params() -> Vec<(String, String)> {
    vec![
        ("response_mode".into(), "form_post".into()),
        ("scope".into(), "email name".into()),
    ]
}

/// The callback phase of a POST: back to the callback as a GET, with the
/// code, state and user when there are a code and a state.
pub fn post_redirect(callback_url: &str, params: &Map<String, Value>) -> String {
    let param = |k: &str| crate::params::string(params, k);
    let mut url = callback_url.to_string();
    if let (Some(code), Some(state)) = (param("code"), param("state")) {
        url.push_str(&format!("?code={}", crate::ruby::cgi_escape(&code)));
        url.push_str(&format!("&state={}", crate::ruby::cgi_escape(&state)));
        if let Some(user) = param("user") {
            url.push_str(&format!("&user={}", crate::ruby::cgi_escape(&user)));
        }
    }
    url
}

/// AppleAuthenticator#fetch_jwks: Apple's keys, cached a day; none when
/// they can't be had.
async fn jwks(conn: &mut PgConnection, http: &Http<'_>) -> Result<Option<Value>, AppError> {
    if let Some(cached) = crate::discourse_connect::cache_read(&mut *conn, JWKS_CACHE_KEY).await?
        && let Ok(v) = serde_json::from_str::<Value>(&cached)
    {
        return Ok(Some(v));
    }
    let fetched = http
        .request(false, TIMEOUT, reqwest::Method::GET, KEYS_URL, &[], None)
        .await?;
    let Ok(fetched) = fetched else {
        tracing::error!("Unable to fetch sign-in-with-apple-jwks (connection failed)");
        return Ok(None);
    };
    if !(200..300).contains(&fetched.status) {
        tracing::error!(
            "Unable to fetch sign-in-with-apple-jwks Faraday::Error status {}",
            fetched.status
        );
        return Ok(None);
    }
    let Ok(keys) = serde_json::from_str::<Value>(&fetched.body) else {
        tracing::error!("Unable to fetch sign-in-with-apple-jwks JSON::ParserError");
        return Ok(None);
    };
    crate::discourse_connect::cache_write(&mut *conn, JWKS_CACHE_KEY, &keys.to_string(), 86_400)
        .await?;
    Ok(Some(keys))
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .ok()
}

/// `JWT.decode(id_token, nil, true, algorithms: ["RS256"], jwks:, iss:,
/// aud:, ...)`: the key by the token's kid, the signature, exp, nbf, iss
/// and aud. Err with the reason for a JWT::DecodeError.
pub fn verify(
    token: &str,
    keys: Option<&Value>,
    client_id: &str,
    now: i64,
) -> Result<Map<String, Value>, String> {
    use sha2::Digest;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Not enough or too many segments".into());
    }
    let header: Value = b64url_decode(parts[0])
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Invalid segment encoding")?;
    let payload: Value = b64url_decode(parts[1])
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Invalid segment encoding")?;
    if header["alg"].as_str() != Some("RS256") {
        return Err("Expected a different algorithm".into());
    }
    let kid = header["kid"]
        .as_str()
        .ok_or("No key id (kid) found from token headers")?;
    let jwk = keys
        .and_then(|k| k["keys"].as_array())
        .and_then(|list| list.iter().find(|k| k["kid"].as_str() == Some(kid)))
        .ok_or_else(|| format!("Could not find public key for kid {kid}"))?;
    let n = jwk["n"]
        .as_str()
        .and_then(b64url_decode)
        .ok_or("Invalid JWK")?;
    let e = jwk["e"]
        .as_str()
        .and_then(b64url_decode)
        .ok_or("Invalid JWK")?;
    let key = rsa::RsaPublicKey::new(
        rsa::BigUint::from_bytes_be(&n),
        rsa::BigUint::from_bytes_be(&e),
    )
    .map_err(|_| "Invalid JWK".to_string())?;
    let signature = b64url_decode(parts[2]).ok_or("Invalid segment encoding")?;
    let mut digest_info = vec![
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    let signed = format!("{}.{}", parts[0], parts[1]);
    digest_info.extend_from_slice(&sha2::Sha256::digest(signed.as_bytes()));
    key.verify(
        rsa::Pkcs1v15Sign::new_unprefixed(),
        &digest_info,
        &signature,
    )
    .map_err(|_| "Signature verification failed".to_string())?;
    let Value::Object(claims) = payload else {
        return Err("Invalid payload".into());
    };
    let to_i = |v: &Value| match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Value::String(s) => crate::ruby::to_i(s),
        _ => 0,
    };
    if let Some(exp) = claims.get("exp")
        && to_i(exp) <= now
    {
        return Err("Signature has expired".into());
    }
    if let Some(nbf) = claims.get("nbf")
        && to_i(nbf) > now
    {
        return Err("Signature nbf has not been reached".into());
    }
    if claims.get("iss").and_then(Value::as_str) != Some(SITE) {
        return Err("Invalid issuer".into());
    }
    let aud: Vec<&str> = match claims.get("aud") {
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(s)) => vec![s.as_str()],
        _ => Vec::new(),
    };
    if !aud.contains(&client_id) {
        return Err("Invalid audience".into());
    }
    Ok(claims)
}

/// The strategy's auth hash: the ID token's claims (extra first, as
/// OmniAuth builds it), the name the browser's `user` param brings.
pub async fn auth_hash(
    conn: &mut PgConnection,
    http: &Http<'_>,
    client: &Client,
    token: &AccessToken,
    params: &Map<String, Value>,
) -> Result<Result<AuthHash, Failure>, AppError> {
    let id_token = token
        .params
        .get("id_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let keys = jwks(conn, http).await?;
    let claims = verify(
        &id_token,
        keys.as_ref(),
        &client.client_id,
        crate::clock::now().timestamp(),
    )
    .map_err(|_| Unsupported("an Apple ID token that does not verify (JWT::DecodeError, a 500)"))?;
    // unsafe_user_info: the `user` param's JSON, else nothing.
    let user: Value = crate::params::string(params, "user")
        .and_then(|u| serde_json::from_str(&u).ok())
        .unwrap_or(Value::Null);
    let name_part = |k: &str| {
        user.get("name")
            .and_then(|n| n.get(k))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let mut info = Map::new();
    if let Some(email) = claims
        .get("email")
        .filter(|e| !e.is_null() && **e != Value::Bool(false))
    {
        info.insert("email".into(), email.clone());
    }
    let (first, last) = (name_part("firstName"), name_part("lastName"));
    info.insert("first_name".into(), first.clone());
    info.insert("last_name".into(), last.clone());
    if !first.is_null() && !last.is_null() {
        let s = |v: &Value| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        info.insert("name".into(), json!(format!("{} {}", s(&first), s(&last))));
    }
    let uid = match claims.get("sub") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let mut extra = Map::new();
    extra.insert(
        "raw_info".into(),
        json!({"id_token_info": Value::Object(claims), "id_token": id_token}),
    );
    Ok(Ok(AuthHash {
        provider: "apple".into(),
        uid,
        info,
        credentials: token.credentials(),
        extra,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posts_come_back_as_gets() {
        let params: Map<String, Value> = serde_json::from_value(json!({
            "code": "c/1", "state": "s", "user": "{\"name\":{\"firstName\":\"A\"}}"
        }))
        .unwrap();
        assert_eq!(
            post_redirect("https://f.x/auth/apple/callback", &params),
            "https://f.x/auth/apple/callback?code=c%2F1&state=s&user=%7B%22name%22%3A%7B%22firstName%22%3A%22A%22%7D%7D"
        );
        assert_eq!(
            post_redirect("https://f.x/auth/apple/callback", &Map::new()),
            "https://f.x/auth/apple/callback"
        );
    }
}
