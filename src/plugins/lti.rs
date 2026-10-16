//! discourse-lti: logins started by an LTI 1.3 platform (a learning
//! management system) as a third party initiated OpenID Connect login.
//! The platform sends the browser to /auth/lti/initiate, the tool sends
//! it on to the platform's authorization endpoint with a state and nonce,
//! and the platform posts an RS256 ID token back to the callback, checked
//! against its configured public key (DiscourseLti::LtiOmniauthStrategy).
//! Cross-site POSTs are first resubmitted from the tool's own page so the
//! browser sends the SameSite=Lax session cookie.

use serde_json::{Map, Value, json};

use crate::auth::managed::AuthHash;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The JS that submits the resubmit page's form on load.
pub const SUBMIT_ON_LOAD_JS: &str = include_str!("../../static/js/submit-on-load-lti.js");
pub const SUBMIT_ON_LOAD_PATH: &str = "/plugins/discourse-lti/javascripts/submit-on-load-lti.js";
/// `DiscourseLti::CUSTOM_DATA_CLAIM`
const CUSTOM_DATA_CLAIM: &str = "https://purl.imsglobal.org/spec/lti/claim/custom";

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

fn client_ids(s: &SiteSettings) -> Result<Vec<String>, AppError> {
    Ok(setting(s, "lti_client_ids")?
        .split('|')
        .map(str::to_string)
        .collect())
}

/// `cross_site_post?`: a POST without the resubmit page's marker.
pub fn cross_site_post(post: bool, params: &Map<String, Value>) -> bool {
    post && params.get("samesite").is_none_or(Value::is_null)
}

/// `resubmit_as_samesite(*params)`: a page that posts the given params
/// back to the same URL with `samesite`, from this site.
pub fn resubmit_page(params: &Map<String, Value>, keys: &[&str], script_url: &str) -> String {
    let fields = keys
        .iter()
        .filter_map(|k| {
            let v = crate::params::string(params, k)?;
            let escaped = html_escape::encode_quoted_attribute(&v).replace("&#x27;", "&#39;");
            Some(format!(
                "<input type='hidden' name='{k}' value='{escaped}'/>"
            ))
        })
        .collect::<Vec<_>>()
        .join("\n");
    // The heredoc's indentation, with the fields' first line indented.
    format!(
        "<html>\n  <head>\n    <script src=\"{script_url}\" nonce=\"\"></script>\n  </head>\n  <body>\n    <form method=\"post\">\n      <input type='hidden' name='samesite' value='true'/>\n      {fields}\n      <noscript>\n        <input type=\"submit\" value=\"Continue\"/>\n      </noscript>\n    </form>\n  </body>\n</html>\n"
    )
}

/// What the initiate phase answers.
pub enum Initiate {
    /// A failure, by its message key.
    Fail(&'static str),
    /// The platform's authorization endpoint, with the state and nonce to
    /// keep and the destination.
    Redirect {
        url: String,
        state: String,
        nonce: String,
        destination: String,
    },
}

/// `Hash#to_query`: sorted, CGI-escaped pairs.
fn to_query(params: &[(&str, String)]) -> String {
    let mut pairs: Vec<String> = params
        .iter()
        .map(|(k, v)| {
            let k: String = form_urlencoded::byte_serialize(k.as_bytes()).collect();
            let v: String = form_urlencoded::byte_serialize(v.as_bytes()).collect();
            format!("{k}={v}")
        })
        .collect();
    pairs.sort();
    pairs.join("&")
}

/// `initiate_phase` past the resubmit: the platform's params checked,
/// the authorization request built.
pub fn initiate(
    s: &SiteSettings,
    params: &Map<String, Value>,
    callback_url: &str,
) -> Result<Initiate, AppError> {
    let param = |k: &str| crate::params::string(params, k).filter(|v| !v.trim().is_empty());
    let (Some(iss), Some(login_hint), Some(target_link_uri)) =
        (param("iss"), param("login_hint"), param("target_link_uri"))
    else {
        return Ok(Initiate::Fail("missing_parameters"));
    };
    // `lti_message_hint` only when given, blank or not.
    let lti_message_hint = crate::params::string(params, "lti_message_hint");
    if iss != setting(s, "lti_platform_issuer_id")? {
        return Ok(Initiate::Fail("invalid_issuer"));
    }
    let ids = client_ids(s)?;
    let client_id = match param("client_id") {
        Some(id) if !ids.contains(&id) => return Ok(Initiate::Fail("invalid_client_id")),
        Some(id) => id,
        None if ids.len() > 1 => return Ok(Initiate::Fail("missing_client_id")),
        None => ids.first().cloned().unwrap_or_default(),
    };
    let state = crate::auth::oauth2::random_hex(16);
    let nonce = crate::auth::oauth2::random_hex(16);
    let mut query: Vec<(&str, String)> = vec![
        ("scope", "openid".into()),
        ("response_type", "id_token".into()),
        ("response_mode", "form_post".into()),
        ("prompt", "none".into()),
        ("client_id", client_id),
        ("redirect_uri", callback_url.into()),
        ("login_hint", login_hint),
        ("state", state.clone()),
        ("nonce", nonce.clone()),
    ];
    if let Some(hint) = lti_message_hint {
        query.push(("lti_message_hint", hint));
    }
    Ok(Initiate::Redirect {
        url: format!(
            "{}?{}",
            setting(s, "lti_authorization_endpoint")?,
            to_query(&query)
        ),
        state,
        nonce,
        destination: target_link_uri,
    })
}

/// What the callback phase found.
pub enum Callback {
    Auth(Box<AuthHash>),
    /// fail! with a message key and the exception's message.
    Fail(String, String),
}

fn b64url(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .ok()
}

/// The platform's key: a PEM, or the bare Base64 of one wrapped as a
/// public key.
fn public_key(raw: &str) -> Result<rsa::RsaPublicKey, AppError> {
    use base64::Engine;
    use rsa::pkcs1::DecodeRsaPublicKey;
    use rsa::pkcs8::DecodePublicKey;
    let unreadable =
        || Unsupported("an LTI platform key OpenSSL can't read (OpenSSL::PKey::RSAError)");
    // The PEM's Base64 between its markers, however its lines run.
    let body: String = raw
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .map_err(|_| unreadable())?;
    let pkcs1 = raw.starts_with("-----BEGIN RSA PUBLIC KEY");
    let key = if pkcs1 {
        rsa::RsaPublicKey::from_pkcs1_der(&der).ok()
    } else {
        rsa::RsaPublicKey::from_public_key_der(&der).ok()
    };
    key.ok_or_else(|| unreadable().into())
}

/// `JWT.decode(token, key, true, algorithm: "RS256", ...)`: the
/// signature, then exp, nbf, iss and aud. Err with the reason for a
/// JWT::DecodeError.
pub fn decode_token(
    token: &str,
    key: &rsa::RsaPublicKey,
    issuer: &str,
    audiences: &[String],
    now: i64,
) -> Result<Map<String, Value>, String> {
    use sha2::Digest;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Not enough or too many segments".into());
    }
    let header: Value = b64url(parts[0])
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Invalid segment encoding")?;
    let payload: Value = b64url(parts[1])
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("Invalid segment encoding")?;
    if header["alg"].as_str() != Some("RS256") {
        return Err("Expected a different algorithm".into());
    }
    let signature = b64url(parts[2]).ok_or("Invalid segment encoding")?;
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
    if claims.get("iss").and_then(Value::as_str) != Some(issuer) {
        return Err("Invalid issuer".into());
    }
    let aud: Vec<String> = match claims.get("aud") {
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    };
    if !aud.iter().any(|a| audiences.contains(a)) {
        return Err("Invalid audience".into());
    }
    Ok(claims)
}

/// `callback_phase`, past the resubmit: the error, the state, the token
/// and the nonce checked, then the auth hash.
pub fn callback(
    s: &SiteSettings,
    params: &Map<String, Value>,
    session_state: Option<&str>,
    session_nonce: Option<&str>,
) -> Result<Callback, AppError> {
    let param = |k: &str| crate::params::string(params, k);
    if let Some(error) = param("error") {
        // CallbackError's message; alone, Discourse's on_failure re-raises
        // the strategy's own CallbackError (a 500).
        let message = [
            param("error"),
            param("error_description"),
            param("error_uri"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" | ");
        if message == error {
            return Err(Unsupported("an LTI error re-raised by on_failure (500)").into());
        }
        return Ok(Callback::Fail(error, message));
    }
    let state = param("state").unwrap_or_default();
    if state.is_empty() {
        return Ok(Callback::Fail(
            "state_missing".into(),
            "State parameter was not included in the callback".into(),
        ));
    }
    let token = param("id_token").unwrap_or_default();
    if token.is_empty() {
        return Ok(Callback::Fail(
            "id_token_missing".into(),
            "id_token parameter was not included in the callback".into(),
        ));
    }
    if Some(state.as_str()) != session_state {
        return Ok(Callback::Fail(
            "state_mismatch".into(),
            "State parameter did not match the session".into(),
        ));
    }
    let key = public_key(&setting(s, "lti_platform_public_key")?)?;
    let ids = client_ids(s)?;
    let claims = match decode_token(
        &token,
        &key,
        &setting(s, "lti_platform_issuer_id")?,
        &ids,
        crate::clock::now().timestamp(),
    ) {
        Ok(c) => c,
        Err(reason) => return Ok(Callback::Fail("token_invalid".into(), reason)),
    };
    if claims.get("nonce").and_then(Value::as_str) != session_nonce {
        return Ok(Callback::Fail(
            "nonce_mismatch".into(),
            "Nonce claim did not match the session".into(),
        ));
    }
    if matches!(claims.get("aud"), Some(Value::Array(list)) if list.len() > 1) {
        // `options.client_ids.include(...)`: a NoMethodError upstream.
        return Err(Unsupported("an LTI ID token with several audiences (NoMethodError)").into());
    }
    let at = |k: &str| claims.get(k).cloned().unwrap_or(Value::Null);
    let mut info = Map::new();
    info.insert("name".into(), at("name"));
    info.insert("email".into(), at("email"));
    info.insert("first_name".into(), at("given_name"));
    info.insert("last_name".into(), at("family_name"));
    info.insert("nickname".into(), at("preferred_username"));
    info.insert("image".into(), at("picture"));
    let uid = match at("sub") {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => other.to_string(),
    };
    let mut extra = Map::new();
    extra.insert("raw_info".into(), Value::Object(claims.clone()));
    extra.insert("id_token".into(), json!(token));
    Ok(Callback::Auth(Box::new(AuthHash {
        provider: "lti".into(),
        uid,
        info,
        credentials: json!({}),
        extra,
    })))
}

/// The plugin's first after_auth: a new user's invite link from the
/// launch's custom claim, when it is this site's invite page.
pub fn invite_destination(extra: &Value, host: &str, base_path: &str) -> Option<String> {
    let custom = extra.get("raw_info")?.get(CUSTOM_DATA_CLAIM)?;
    let invite = ["discourse_invite_link", "custom_discourse_invite_link"]
        .iter()
        .find_map(|k| custom.get(*k).filter(|v| !v.is_null()))?;
    let invite = match invite {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    // URI.parse: a relative path, or an absolute URL on this host.
    let (url_host, path) = if invite.starts_with("http://") || invite.starts_with("https://") {
        let parsed = reqwest::Url::parse(&invite).ok()?;
        (
            parsed.host_str().map(str::to_string),
            parsed.path().to_string(),
        )
    } else if invite.starts_with('/') {
        (
            None,
            invite.split(['?', '#']).next().unwrap_or("").to_string(),
        )
    } else {
        return None;
    };
    if url_host.as_deref().is_some_and(|h| h != host) {
        return None;
    }
    // Discourse.route_for: invites#show is GET /invites/:id.
    let path = path.strip_prefix(base_path).unwrap_or(&path);
    let id = path.strip_prefix("/invites/")?;
    if id.is_empty() || id.contains(['/', '.']) {
        return None;
    }
    Some(invite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_invite_links() {
        let extra = json!({"raw_info": {CUSTOM_DATA_CLAIM: {"custom_discourse_invite_link": "https://forum.example/invites/abc123"}}});
        assert_eq!(
            invite_destination(&extra, "forum.example", "").as_deref(),
            Some("https://forum.example/invites/abc123")
        );
        assert_eq!(invite_destination(&extra, "other.example", ""), None);
        let latest = json!({"raw_info": {CUSTOM_DATA_CLAIM: {"discourse_invite_link": "/latest"}}});
        assert_eq!(invite_destination(&latest, "forum.example", ""), None);
    }

    #[test]
    fn queries_sorted() {
        assert_eq!(
            to_query(&[("b", "x y".into()), ("a", "1/2".into())]),
            "a=1%2F2&b=x+y"
        );
    }
}
