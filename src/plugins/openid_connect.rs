//! discourse-openid-connect: logins through an OpenID Connect provider
//! (OpenIDConnectAuthenticator and its OmniAuth strategy). The provider
//! is found from its discovery document; the ID token's claims are
//! checked (issuer, audience, expiry, the nonce sent) but not its
//! signature, as the plugin does: it comes straight from the issuer.
//!
//! Refused: mutual TLS client authentication.

use serde_json::{Map, Value};
use sqlx::PgConnection;

use crate::auth::managed::{AuthHash, AuthResult, Behaviour};
use crate::auth::oauth2::{self, AccessToken, AuthScheme, Client, Failure, Http};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `GlobalSetting.openid_connect_request_timeout_seconds`
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// The discovery document's cache lifetime.
const DISCOVERY_TTL_SECONDS: i64 = 600;

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

/// `discovery_document`: fetched through FinalDestination, kept ten
/// minutes in Discourse.cache; None when it can't be had.
pub async fn discovery_document(
    conn: &mut PgConnection,
    s: &SiteSettings,
    http: &Http<'_>,
) -> Result<Option<Value>, AppError> {
    let url = setting(s, "openid_connect_discovery_document")?;
    if url.trim().is_empty() {
        tracing::error!("OIDC Log: No discovery document URL specified");
        return Ok(None);
    }
    let key = format!("openid-connect-discovery-{url}");
    if let Some(cached) = crate::discourse_connect::cache_read(conn, &key).await? {
        // A cached nil is kept too (`fetch` stores the block's nil).
        return Ok(serde_json::from_str::<Value>(&cached)
            .ok()
            .filter(|v| !v.is_null()));
    }
    let fetched = http
        .request(true, TIMEOUT, reqwest::Method::GET, &url, &[], None)
        .await?;
    let doc = match fetched {
        Ok(f) if (200..300).contains(&f.status) => serde_json::from_str::<Value>(&f.body).ok(),
        Ok(f) => {
            tracing::error!(
                "OIDC Log: Fetching discovery document raised error Faraday::Error status {}",
                f.status
            );
            None
        }
        Err(failure) => {
            tracing::error!(
                "OIDC Log: Fetching discovery document raised error {}",
                failure.message
            );
            None
        }
    };
    let stored = doc.clone().unwrap_or(Value::Null).to_string();
    crate::discourse_connect::cache_write(conn, &key, &stored, DISCOVERY_TTL_SECONDS).await?;
    Ok(doc)
}

/// What `discover!` reads from the document.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub authorize_url: String,
    pub token_url: String,
    /// The issuer (`client_options[:site]`).
    pub site: String,
    pub userinfo_endpoint: Option<String>,
    pub auth_scheme: AuthScheme,
}

fn mtls(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(!setting(s, "openid_connect_mtls_client_cert")?
        .trim()
        .is_empty()
        && !setting(s, "openid_connect_mtls_client_key")?
            .trim()
            .is_empty())
}

/// `discover!`: the endpoints, or the discovery error.
pub fn discover(
    doc: Option<&Value>,
    s: &SiteSettings,
) -> Result<Result<Discovered, Failure>, AppError> {
    let Some(doc) = doc else {
        return Ok(Err(Failure::new(
            "openid_connect_discovery_error",
            "Discovery document is missing",
        )));
    };
    let mut values = Vec::new();
    for key in ["authorization_endpoint", "token_endpoint", "issuer"] {
        match &doc[key] {
            Value::String(v) if !v.is_empty() => values.push(v.clone()),
            Value::Null => {
                return Ok(Err(Failure::new(
                    "openid_connect_discovery_error",
                    format!("missing discovery parameter {key}"),
                )));
            }
            Value::String(_) => {
                return Ok(Err(Failure::new(
                    "openid_connect_discovery_error",
                    format!("missing discovery parameter {key}"),
                )));
            }
            other => values.push(other.to_string()),
        }
    }
    let userinfo = match &doc["userinfo_endpoint"] {
        Value::String(v) if !v.is_empty() => Some(v.clone()),
        Value::Null => None,
        Value::String(_) => None,
        other => Some(other.to_string()),
    };
    let mut auth_scheme = if mtls(s)? {
        AuthScheme::TlsClientAuth
    } else {
        AuthScheme::BasicAuth
    };
    if let Some(methods) = doc["token_endpoint_auth_methods_supported"].as_array() {
        let has = |m: &str| methods.iter().any(|v| v == m);
        if !has("client_secret_basic") && has("client_secret_post") {
            auth_scheme = AuthScheme::RequestBody;
        }
    }
    if s.get("openid_connect_use_pkce")?.truthy()
        && setting(s, "openid_connect_client_secret")?.is_empty()
    {
        auth_scheme = AuthScheme::RequestBody;
    }
    Ok(Ok(Discovered {
        authorize_url: values[0].clone(),
        token_url: values[1].clone(),
        site: values[2].clone(),
        userinfo_endpoint: userinfo,
        auth_scheme,
    }))
}

/// The strategy's client after discovery.
pub fn client(s: &SiteSettings, d: &Discovered) -> Result<Client, AppError> {
    let mut token_params = Vec::new();
    let token_scope = setting(s, "openid_connect_token_scope")?;
    if !token_scope.trim().is_empty() {
        token_params.push(("scope".to_string(), token_scope));
    }
    Ok(Client {
        client_id: setting(s, "openid_connect_client_id")?,
        client_secret: setting(s, "openid_connect_client_secret")?,
        site: Some(d.site.clone()),
        authorize_url: d.authorize_url.clone(),
        token_url: d.token_url.clone(),
        token_method: reqwest::Method::POST,
        auth_scheme: d.auth_scheme,
        token_headers: Vec::new(),
        token_params,
        final_destination: true,
        timeout: TIMEOUT,
    })
}

/// The request phase's extra params and what the session keeps: the
/// passthrough authorize params from the request, the claims, the scope
/// and a fresh nonce; the `p` param kept for the token request.
pub struct Authorize {
    pub params: Vec<(String, String)>,
    pub nonce: String,
    /// `omniauth.param.p`
    pub param_p: Option<String>,
}

pub fn authorize_params(
    s: &SiteSettings,
    request: &serde_json::Map<String, Value>,
) -> Result<Authorize, AppError> {
    let mut params = Vec::new();
    let req = |k: &str| crate::params::scalar(request.get(k)?).filter(|v| !v.is_empty());
    let scope = setting(s, "openid_connect_authorize_scope")?;
    // options_for("authorize"): the scope option.
    if !scope.is_empty() {
        params.push(("scope".to_string(), scope.clone()));
    }
    for k in setting(s, "openid_connect_authorize_parameters")?.split('|') {
        if k.is_empty() {
            continue;
        }
        if let Some(v) = req(k) {
            params.push((k.to_string(), v));
        }
    }
    let claims = setting(s, "openid_connect_claims")?;
    if !claims.trim().is_empty() {
        params.push(("claims".to_string(), claims));
    }
    // params[:scope] = options[:scope]
    params.retain(|(k, _)| k != "scope");
    params.push(("scope".to_string(), scope));
    let nonce = oauth2::random_hex(32);
    params.push(("nonce".to_string(), nonce.clone()));
    Ok(Authorize {
        params,
        nonce,
        param_p: req("p"),
    })
}

/// `error_handler`: a provider error whose description names a
/// configured message is sent to that message's url.
pub fn error_redirect(s: &SiteSettings, description: &str) -> Result<Option<String>, AppError> {
    for row in setting(s, "openid_connect_error_redirects")?.split('\n') {
        let parts: Vec<&str> = row.split('|').collect();
        if let (Some(message), Some(url)) = (parts.first(), parts.get(1))
            && description.contains(message)
        {
            return Ok(Some(url.to_string()));
        }
    }
    Ok(None)
}

/// A JWT's payload, unverified (`JWT.decode(token, nil, false)`).
fn decode_jwt(token: &str) -> Option<Map<String, Value>> {
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 2 && parts.len() != 3 {
        return None;
    }
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let header = engine.decode(parts[0]).ok()?;
    serde_json::from_slice::<Value>(&header).ok()?.as_object()?;
    let payload = engine.decode(parts[1]).ok()?;
    serde_json::from_slice::<Value>(&payload)
        .ok()?
        .as_object()
        .cloned()
}

/// `id_token_info`: the ID token's claims, verified (exp, nbf, the
/// issuer, the audience) and its nonce the one the session kept.
pub fn id_token_info(
    id_token: &str,
    issuer: &str,
    client_id: &str,
    session_nonce: Option<&str>,
    now: i64,
) -> Result<Map<String, Value>, Failure> {
    let decode_failed = |m: &str| Failure::new("jwt_decode_failed", m);
    let claims = decode_jwt(id_token).ok_or_else(|| decode_failed("Invalid segment encoding"))?;
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
        return Err(decode_failed("Signature has expired"));
    }
    if let Some(nbf) = claims.get("nbf")
        && to_i(nbf) > now
    {
        return Err(decode_failed("Signature nbf has not been reached"));
    }
    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err(decode_failed("Invalid issuer"));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == client_id,
        Some(Value::Array(a)) => a.iter().any(|v| v == client_id),
        _ => false,
    };
    if !aud_ok {
        return Err(decode_failed("Invalid audience"));
    }
    let nonce = claims.get("nonce").and_then(Value::as_str).unwrap_or("");
    if nonce.is_empty() || Some(nonce) != session_nonce {
        return Err(Failure::new(
            "jwt_nonce_verify_failed",
            "JWT nonce is missing, or does not match",
        ));
    }
    Ok(claims)
}

/// `prune!`: nil and empty values gone, nested hashes too.
fn prune(map: &mut Map<String, Value>) {
    map.retain(|_, v| {
        if let Value::Object(inner) = v {
            prune(inner);
        }
        !match v {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            _ => false,
        }
    });
}

/// The token request for a provider without a userinfo endpoint:
/// `build_access_token`'s own POST, the token being the ID token.
pub async fn token_without_userinfo(
    http: &Http<'_>,
    client: &Client,
    code: &str,
    callback_url: &str,
    extra: &[(String, String)],
) -> Result<Result<AccessToken, Failure>, AppError> {
    let mut params: Vec<(String, String)> = vec![
        ("redirect_uri".into(), callback_url.into()),
        ("grant_type".into(), "authorization_code".into()),
        ("code".into(), code.into()),
        ("client_id".into(), client.client_id.clone()),
        ("client_secret".into(), client.client_secret.clone()),
    ];
    for (k, v) in client.token_params.iter().chain(extra.iter()) {
        match params.iter_mut().find(|(key, _)| key == k) {
            Some(slot) => slot.1 = v.clone(),
            None => params.push((k.clone(), v.clone())),
        }
    }
    let url = oauth2::build_url(client.site.as_deref(), &client.token_url, &[])?;
    let headers = vec![(
        "Content-Type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    )];
    let fetched = match http
        .request(
            true,
            client.timeout,
            reqwest::Method::POST,
            &url,
            &headers,
            Some(oauth2::encode_query(&params)),
        )
        .await?
    {
        Ok(f) => f,
        Err(f) => {
            return Ok(Err(Failure::new(
                "openid_connect_request_failed",
                f.message,
            )));
        }
    };
    if fetched.status >= 400 {
        return Ok(Err(Failure::new(
            "invalid_credentials",
            format!("token request failed: {}", fetched.status),
        )));
    }
    let parsed = oauth2::parse_body(&fetched.content_type, &fetched.body)?.unwrap_or_default();
    let id_token = match parsed.get("id_token") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let mut hash = parsed;
    hash.insert("access_token".into(), Value::String(id_token));
    Ok(oauth2::access_token(hash, crate::clock::now().timestamp()))
}

/// The strategy's auth hash: uid from the ID token, info and raw_info
/// from the userinfo endpoint (or the ID token without one).
pub async fn auth_hash(
    http: &Http<'_>,
    client: &Client,
    d: &Discovered,
    token: &AccessToken,
    session_nonce: Option<&str>,
) -> Result<Result<AuthHash, Failure>, AppError> {
    let id_token = match token.params.get("id_token") {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    let verify = || {
        id_token_info(
            &id_token,
            &d.site,
            &client.client_id,
            session_nonce,
            crate::clock::now().timestamp(),
        )
    };
    // The auth hash's extra comes first: with a userinfo endpoint, its
    // request, then the ID token's checks (userinfo_response); without,
    // the checks.
    let (raw_info, claims): (Map<String, Value>, Map<String, Value>) = match &d.userinfo_endpoint {
        Some(endpoint) => {
            let info = match http
                .get_with_token(client, &token.token, endpoint, &[])
                .await?
            {
                Ok(Value::Object(m)) => m,
                Ok(_) => Map::new(),
                Err(f) => {
                    return Ok(Err(Failure::new(
                        if f.kind == "timeout" {
                            "openid_connect_request_failed"
                        } else {
                            "invalid_credentials"
                        },
                        f.message,
                    )));
                }
            };
            let verified = match verify() {
                Ok(c) => c,
                Err(f) => return Ok(Err(f)),
            };
            if info.get("sub") != verified.get("sub") {
                return Ok(Err(Failure::new(
                    "openid_connect_sub_mismatch",
                    "OIDC `sub` mismatch",
                )));
            }
            (info, verified)
        }
        None => {
            let verified = match verify() {
                Ok(c) => c,
                Err(f) => return Ok(Err(f)),
            };
            (verified.clone(), verified)
        }
    };
    let field = |k: &str| raw_info.get(k).cloned().unwrap_or(Value::Null);
    let mut info = Map::new();
    info.insert("name".into(), field("name"));
    info.insert("email".into(), field("email"));
    info.insert("first_name".into(), field("given_name"));
    info.insert("last_name".into(), field("family_name"));
    info.insert("nickname".into(), field("preferred_username"));
    info.insert("image".into(), field("picture"));
    prune(&mut info);
    let mut extra = Map::new();
    extra.insert("raw_info".into(), Value::Object(raw_info.clone()));
    extra.insert("id_token".into(), Value::String(id_token));
    extra.insert("id_token_info".into(), Value::Object(claims.clone()));
    prune(&mut extra);
    let uid = match claims.get("sub") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    Ok(Ok(AuthHash {
        provider: "oidc".into(),
        uid,
        info,
        credentials: token.credentials(),
        extra,
    }))
}

/// `primary_email_verified?`: the raw info's email_verified, true when
/// it doesn't say.
fn email_verified(auth: &AuthHash) -> bool {
    match auth
        .extra
        .get("raw_info")
        .and_then(|r| r.get("email_verified"))
    {
        None | Some(Value::Null) => true,
        v => crate::auth::managed::true_or_string_true(v),
    }
}

/// `user_field_values_from(auth_token)`: each mapped claim, from the raw
/// info or else the ID token.
fn user_field_values_from(
    s: &SiteSettings,
    auth: &AuthHash,
) -> Result<Map<String, Value>, AppError> {
    let raw = setting(s, "openid_connect_user_field_mappings")?;
    let raw = if raw.trim().is_empty() {
        "[]".to_string()
    } else {
        raw
    };
    let Ok(Value::Array(mappings)) = serde_json::from_str::<Value>(&raw) else {
        return Ok(Map::new());
    };
    let raw_info = auth.extra.get("raw_info").and_then(Value::as_object);
    let id_info = auth.extra.get("id_token_info").and_then(Value::as_object);
    let mut out = Map::new();
    for m in mappings {
        let claim = match &m["claim"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let field = &m["user_field_id"];
        if claim.trim().is_empty() || field.is_null() || field.as_str().is_some_and(str::is_empty) {
            continue;
        }
        let source = if raw_info.is_some_and(|r| r.contains_key(&claim)) {
            raw_info
        } else if id_info.is_some_and(|r| r.contains_key(&claim)) {
            id_info
        } else {
            None
        };
        let Some(source) = source else { continue };
        let text = match &source[&claim] {
            Value::Array(items) => items
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let key = match field {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.insert(key, Value::String(text));
    }
    Ok(out)
}

/// OpenIDConnectAuthenticator#after_authenticate
pub async fn after_authenticate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    auth: &AuthHash,
    existing_account: Option<i32>,
) -> Result<AuthResult, AppError> {
    let behaviour = Behaviour {
        email_verified: email_verified(auth),
        match_by_email: s.get("openid_connect_match_by_email")?.truthy(),
        overrides_email: s.get("openid_connect_overrides_email")?.truthy(),
        can_connect: s.get("openid_connect_allow_association_change")?.truthy(),
        user_by_email: None,
    };
    let mut result =
        crate::auth::managed::after_authenticate(conn, s, auth, &behaviour, existing_account)
            .await?;
    let claim = setting(s, "openid_connect_groups_claim")?;
    if !claim.trim().is_empty() {
        let groups = auth
            .extra
            .get("raw_info")
            .and_then(|r| r.get(&claim))
            .filter(|v| !v.is_null())
            .or_else(|| auth.extra.get("id_token_info").and_then(|r| r.get(&claim)));
        result.associated_groups = Some(match groups {
            Some(Value::Array(items)) => items
                .iter()
                .map(|g| {
                    let v = match g {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (v.clone(), v)
                })
                .collect(),
            _ => Vec::new(),
        });
    }
    result.user_field_values = Some(user_field_values_from(s, auth)?);
    Ok(result)
}

/// The `before_session_destroy` listener's RP-initiated logout: the
/// provider's end session url with the user's ID token, when one is
/// kept.
pub async fn logout_redirect(
    conn: &mut PgConnection,
    s: &SiteSettings,
    http: &Http<'_>,
    user_id: i32,
) -> Result<Option<String>, AppError> {
    if !s.get("openid_connect_rp_initiated_logout")?.truthy() {
        return Ok(None);
    }
    let extra: Option<Value> = sqlx::query_scalar(
        "SELECT extra FROM user_associated_accounts WHERE user_id = $1 AND provider_name = 'oidc' LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(token) = extra
        .as_ref()
        .and_then(|e| e.get("id_token"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
    else {
        return Ok(None);
    };
    let doc = discovery_document(conn, s, http).await?;
    let Some(endpoint) = doc
        .as_ref()
        .and_then(|d| d.get("end_session_endpoint"))
        .and_then(Value::as_str)
        .filter(|e| !e.is_empty())
    else {
        tracing::error!("OIDC Log: Logout: No end_session_endpoint found in discovery document");
        return Ok(None);
    };
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|_| Unsupported("an end_session_endpoint that does not parse"))?;
    let mut params: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    params.push(("id_token_hint".into(), token));
    let client_id = setting(s, "openid_connect_client_id")?;
    if s.get("openid_connect_rp_initiated_logout_include_client_id")?
        .truthy()
        && !client_id.is_empty()
    {
        params.push(("client_id".into(), client_id));
    }
    let post_logout = setting(s, "openid_connect_rp_initiated_logout_redirect")?;
    if !post_logout.trim().is_empty() {
        params.push(("post_logout_redirect_uri".into(), post_logout));
    }
    if s.get("openid_connect_rp_initiated_logout_include_state")?
        .truthy()
    {
        params.push(("state".into(), oauth2::random_hex(16)));
    }
    // URI.encode_www_form
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();
    url.set_query(Some(&query));
    Ok(Some(url.to_string()))
}

/// `generate_code_verifier`'s use: omniauth-oauth2 makes its own
/// (`SecureRandom.hex(64)`) and the plugin's challenge hashes it.
pub fn pkce(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(s.get("openid_connect_use_pkce")?.truthy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: Value) -> String {
        use base64::Engine;
        let e = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.sig",
            e.encode(json!({"alg": "RS256"}).to_string()),
            e.encode(claims.to_string())
        )
    }

    #[test]
    fn checks_id_token_claims() {
        let good = json!({"iss": "https://idp", "aud": ["c1", "c2"], "sub": "u1", "exp": 2000, "nonce": "n"});
        let token = jwt(good.clone());
        assert!(id_token_info(&token, "https://idp", "c1", Some("n"), 1000).is_ok());
        assert_eq!(
            id_token_info(&token, "https://idp", "c1", Some("n"), 2000)
                .unwrap_err()
                .kind,
            "jwt_decode_failed"
        );
        assert_eq!(
            id_token_info(&token, "https://other", "c1", Some("n"), 1000)
                .unwrap_err()
                .kind,
            "jwt_decode_failed"
        );
        assert_eq!(
            id_token_info(&token, "https://idp", "c3", Some("n"), 1000)
                .unwrap_err()
                .kind,
            "jwt_decode_failed"
        );
        assert_eq!(
            id_token_info(&token, "https://idp", "c1", Some("x"), 1000)
                .unwrap_err()
                .kind,
            "jwt_nonce_verify_failed"
        );
        assert_eq!(
            id_token_info("garbage", "https://idp", "c1", Some("n"), 1000)
                .unwrap_err()
                .kind,
            "jwt_decode_failed"
        );
    }
}
