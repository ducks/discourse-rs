//! OmniAuth's OAuth2 strategy (omniauth-oauth2) over the oauth2 gem's
//! client: the authorize redirect with its state (and PKCE) kept in the
//! browser's session, and the callback's state check and code exchange.
//! URLs are built as Faraday builds them: joined to the site, their query
//! merged with the extra params and sorted.

use serde_json::{Map, Value};

use crate::{AppError, Unsupported};

/// `Faraday::Utils.escape`: unreserved bytes kept, spaces as "%20" (the
/// oauth2 gem sets `Faraday::Utils.default_space_encoding` to it).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'~' | b'_' | b'-' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `Faraday::NestedParamsEncoder.encode` of flat params: sorted, escaped.
pub fn encode_query(params: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort();
    sorted
        .iter()
        .map(|(k, v)| format!("{}={}", escape(k), escape(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `NestedParamsEncoder.decode` of a flat query: later keys win.
fn decode_query(query: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        let (k, v) = (k.into_owned(), v.into_owned());
        match out.iter_mut().find(|(key, _)| *key == k) {
            Some(slot) => slot.1 = v,
            None => out.push((k, v)),
        }
    }
    out
}

/// `Connection#build_url(url, params)` on a connection to `site`: the
/// url joined to the site (a relative one under its path), its query
/// merged with `params` and re-encoded.
pub fn build_url(
    site: Option<&str>,
    url: &str,
    params: &[(String, String)],
) -> Result<String, AppError> {
    let absolute = url.starts_with("http://") || url.starts_with("https://");
    let mut parsed = if absolute {
        reqwest::Url::parse(url).map_err(|_| Unsupported("an OAuth url that does not parse"))?
    } else {
        let site = site.ok_or(Unsupported("a relative OAuth url without a site"))?;
        let mut base = reqwest::Url::parse(site)
            .map_err(|_| Unsupported("an OAuth site that does not parse"))?;
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let relative = if url.starts_with('/') || url.starts_with("./") || url.starts_with("../") {
            url.to_string()
        } else {
            format!("./{url}")
        };
        base.join(&relative)
            .map_err(|_| Unsupported("an OAuth url that does not join"))?
    };
    let mut merged = parsed.query().map(decode_query).unwrap_or_default();
    for (k, v) in params {
        match merged.iter_mut().find(|(key, _)| key == k) {
            Some(slot) => slot.1 = v.clone(),
            None => merged.push((k.clone(), v.clone())),
        }
    }
    if merged.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.set_query(Some(&encode_query(&merged)));
    }
    Ok(parsed.to_string())
}

/// How the client authenticates at the token endpoint (`auth_scheme`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AuthScheme {
    /// An `Authorization: Basic` header.
    BasicAuth,
    /// client_id and client_secret in the params.
    RequestBody,
    /// Only client_id (mutual TLS; refused).
    TlsClientAuth,
}

/// A strategy's client options, resolved from settings.
#[derive(Debug, Clone)]
pub struct Client {
    pub client_id: String,
    pub client_secret: String,
    pub site: Option<String>,
    pub authorize_url: String,
    pub token_url: String,
    /// `token_method`: GET, POST or PUT.
    pub token_method: reqwest::Method,
    pub auth_scheme: AuthScheme,
    /// Headers sent with the token request (`token_params[:headers]`).
    pub token_headers: Vec<(String, String)>,
    /// Extra token params (`token_params`).
    pub token_params: Vec<(String, String)>,
    /// Requests made through FinalDestination (the SSRF check).
    pub final_destination: bool,
    pub timeout: std::time::Duration,
}

/// `SecureRandom.hex(n)`
pub fn random_hex(bytes: usize) -> String {
    (0..bytes)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

/// PKCE's `code_challenge` for a verifier: URL-safe Base64 of its
/// SHA-256, unpadded.
pub fn code_challenge(verifier: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier))
}

/// `client.auth_code.authorize_url({redirect_uri:}.merge(authorize_params))`:
/// the strategy's params with the state (and PKCE challenge), then
/// response_type and client_id.
pub fn authorize_url(
    client: &Client,
    callback_url: &str,
    params: &[(String, String)],
) -> Result<String, AppError> {
    let mut all: Vec<(String, String)> = vec![("redirect_uri".into(), callback_url.into())];
    for (k, v) in params {
        match all.iter_mut().find(|(key, _)| key == k) {
            Some(slot) => slot.1 = v.clone(),
            None => all.push((k.clone(), v.clone())),
        }
    }
    for (k, v) in [
        ("response_type", "code".to_string()),
        ("client_id", client.client_id.clone()),
    ] {
        match all.iter_mut().find(|(key, _)| key == k) {
            Some(slot) => slot.1 = v,
            None => all.push((k.to_string(), v)),
        }
    }
    build_url(client.site.as_deref(), &client.authorize_url, &all)
}

/// An `OAuth2::AccessToken`: the token, what the response said of its
/// expiry and refresh, and the rest of it (`params`).
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub params: Map<String, Value>,
}

impl AccessToken {
    /// `credentials`: the token, the refresh token of one that expires,
    /// and when.
    pub fn credentials(&self) -> Value {
        let mut out = Map::new();
        out.insert("token".into(), Value::String(self.token.clone()));
        if self.expires_at.is_some()
            && let Some(r) = &self.refresh_token
        {
            out.insert("refresh_token".into(), Value::String(r.clone()));
        }
        if let Some(at) = self.expires_at {
            out.insert("expires_at".into(), Value::from(at));
        }
        out.insert("expires".into(), Value::Bool(self.expires_at.is_some()));
        Value::Object(out)
    }
}

/// Why the callback phase failed, as OmniAuth's `fail!` names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// `omniauth.error.type`
    pub kind: String,
    /// The exception's message, for the log.
    pub message: String,
}

impl Failure {
    pub fn new(kind: &str, message: impl Into<String>) -> Failure {
        Failure {
            kind: kind.to_string(),
            message: message.into(),
        }
    }
}

/// SnakyHash's key conversion: camelCase to snake_case.
fn snake(key: &str) -> String {
    let chars: Vec<char> = key.replace("::", "/").chars().collect();
    let mut out = String::with_capacity(key.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev.is_ascii_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_ascii_uppercase() && next_lower)
            {
                out.push('_');
            }
        }
        out.push(if c == '-' { '_' } else { c });
    }
    out.to_lowercase()
}

pub fn snaky(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (snake(&k), snaky(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(snaky).collect()),
        other => other,
    }
}

/// `OAuth2::Response#parsed`: JSON or a form body by content type, keys
/// snake_cased; anything else is not a hash.
pub fn parse_body(content_type: &str, body: &str) -> Result<Option<Map<String, Value>>, AppError> {
    let ct = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let value = match ct.as_str() {
        "application/json"
        | "text/javascript"
        | "application/hal+json"
        | "application/vnd.collection+json"
        | "application/vnd.api+json"
        | "application/problem+json" => {
            if body.is_empty() {
                return Ok(None);
            }
            serde_json::from_str::<Value>(body).map_err(|_| {
                Unsupported("an OAuth response that is not JSON (JSON::ParserError)")
            })?
        }
        "application/x-www-form-urlencoded" => Value::Object(
            decode_query(body)
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        ),
        _ => return Ok(None),
    };
    Ok(match snaky(value) {
        Value::Object(m) => Some(m),
        _ => None,
    })
}

/// `AccessToken.from_hash`: the first token key present, the expiry and
/// refresh keys taken out, the rest kept as params.
pub fn access_token(mut hash: Map<String, Value>, now: i64) -> Result<AccessToken, Failure> {
    let string = |v: Option<Value>| match v {
        Some(Value::String(s)) => Some(s),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    };
    let key = [
        "access_token",
        "id_token",
        "token",
        "accessToken",
        "idToken",
    ]
    .into_iter()
    .find(|k| hash.contains_key(*k));
    let token = key.and_then(|k| string(hash.remove(k))).unwrap_or_default();
    let refresh_token = string(hash.remove("refresh_token"));
    let mut expires_in = hash.remove("expires_in");
    let expires_at = hash.remove("expires_at");
    let expires_latency = hash.remove("expires_latency");
    if token.is_empty() && refresh_token.as_deref().is_none_or(str::is_empty) {
        return Err(Failure::new(
            "invalid_credentials",
            "OAuth2::AccessToken has no token",
        ));
    }
    if expires_in.is_none() {
        expires_in = hash.remove("expires");
    }
    let to_i = |v: &Value| match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let expires_in = expires_in.as_ref().and_then(to_i);
    let mut expires_at = expires_at.as_ref().and_then(to_i);
    if expires_at.is_none()
        && let Some(e) = expires_in.filter(|e| *e != 0)
    {
        expires_at = Some(now + e);
    }
    if let (Some(at), Some(latency)) = (expires_at, expires_latency.as_ref().and_then(to_i)) {
        expires_at = Some(at - latency);
    }
    Ok(AccessToken {
        token,
        refresh_token,
        expires_at,
        params: hash,
    })
}

/// A response from the provider.
pub struct Fetched {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

/// What a strategy's HTTP requests need.
pub struct Http<'a> {
    pub fetcher: crate::final_destination::Fetcher<'a>,
    pub user_agent: String,
}

impl Http<'_> {
    /// A provider's fixed endpoint where the config says it is reached
    /// (`Config::auth_provider_hosts`, set by tests only).
    pub fn rewrite(&self, url: &str) -> String {
        for (from, to) in &self.fetcher.urls.config.auth_provider_hosts {
            if let Some(rest) = url.strip_prefix(from.as_str()) {
                return format!("{to}{rest}");
            }
        }
        url.to_string()
    }

    /// One request, through FinalDestination's SSRF check or straight.
    /// Err with the failure OmniAuth would name.
    pub async fn request(
        &self,
        final_destination: bool,
        timeout: std::time::Duration,
        method: reqwest::Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<String>,
    ) -> Result<Result<Fetched, Failure>, AppError> {
        let url = &self.rewrite(url);
        if final_destination {
            let body = body.unwrap_or_default();
            let posted = self
                .fetcher
                .send(
                    method,
                    &crate::final_destination::Post {
                        url,
                        headers,
                        body: body.as_bytes(),
                        verify_certificate: true,
                        timeout,
                        user_agent: &self.user_agent,
                    },
                )
                .await?;
            return Ok(match posted {
                crate::final_destination::Posted::Response {
                    status,
                    headers,
                    body,
                } => Ok(Fetched {
                    status,
                    content_type: headers
                        .iter()
                        .find(|(n, _)| n == "content-type")
                        .map(|(_, v)| v.clone())
                        .unwrap_or_default(),
                    body,
                }),
                crate::final_destination::Posted::Blocked => Err(Failure::new(
                    "timeout",
                    "FinalDestination: all resolved IPs were disallowed",
                )),
                crate::final_destination::Posted::Failed(e) => Err(Failure::new("timeout", e)),
            });
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()?;
        let mut req = client.request(method, url);
        req = req.header(reqwest::header::USER_AGENT, self.user_agent.as_str());
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(body) = body {
            req = req.body(body);
        }
        let response = match req.send().await {
            Ok(r) => r,
            Err(e) => return Ok(Err(Failure::new("timeout", e.to_string()))),
        };
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => return Ok(Err(Failure::new("timeout", e.to_string()))),
        };
        Ok(Ok(Fetched {
            status,
            content_type,
            body,
        }))
    }

    /// `client.auth_code.get_token(code, {redirect_uri:}.merge(token_params))`
    pub async fn get_token(
        &self,
        client: &Client,
        code: &str,
        callback_url: &str,
        extra: &[(String, String)],
    ) -> Result<Result<AccessToken, Failure>, AppError> {
        let mut params: Vec<(String, String)> = vec![
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), code.into()),
            ("redirect_uri".into(), callback_url.into()),
        ];
        for (k, v) in client.token_params.iter().chain(extra.iter()) {
            match params.iter_mut().find(|(key, _)| key == k) {
                Some(slot) => slot.1 = v.clone(),
                None => params.push((k.clone(), v.clone())),
            }
        }
        self.token_request(client, params, client.token_headers.clone())
            .await
    }

    /// `AccessToken#refresh!`: the refresh token exchanged for a new
    /// token, which keeps the old refresh token when it brings none.
    pub async fn refresh(
        &self,
        client: &Client,
        token: &AccessToken,
    ) -> Result<Result<AccessToken, Failure>, AppError> {
        let Some(refresh) = token.refresh_token.clone().filter(|r| !r.is_empty()) else {
            return Ok(Err(Failure::new(
                "invalid_credentials",
                "A refresh_token is not available",
            )));
        };
        let params = vec![
            ("grant_type".to_string(), "refresh_token".to_string()),
            ("refresh_token".to_string(), refresh.clone()),
        ];
        Ok(
            match self.token_request(client, params, Vec::new()).await? {
                Ok(mut fresh) => {
                    if fresh.refresh_token.is_none() {
                        fresh.refresh_token = Some(refresh);
                    }
                    Ok(fresh)
                }
                Err(f) => Err(f),
            },
        )
    }

    /// `client.get_token(params)`: the client's authentication applied, the
    /// request sent by the token method, the answer read as a token.
    async fn token_request(
        &self,
        client: &Client,
        mut params: Vec<(String, String)>,
        mut headers: Vec<(String, String)>,
    ) -> Result<Result<AccessToken, Failure>, AppError> {
        match client.auth_scheme {
            AuthScheme::BasicAuth => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", client.client_id, client.client_secret));
                // basic_auth_header.merge(headers): the given ones win.
                if !headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                {
                    headers.insert(0, ("Authorization".into(), format!("Basic {encoded}")));
                }
            }
            AuthScheme::RequestBody => {
                let mut auth = vec![
                    ("client_id".to_string(), client.client_id.clone()),
                    ("client_secret".to_string(), client.client_secret.clone()),
                ];
                for p in params.drain(..) {
                    match auth.iter_mut().find(|(k, _)| *k == p.0) {
                        Some(slot) => slot.1 = p.1,
                        None => auth.push(p),
                    }
                }
                params = auth;
            }
            AuthScheme::TlsClientAuth => {
                return Err(Unsupported("mutual TLS client authentication").into());
            }
        }
        let post = client.token_method == reqwest::Method::POST;
        let (url, body) = if post {
            headers.push((
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            ));
            (
                build_url(client.site.as_deref(), &client.token_url, &[])?,
                Some(encode_query(&params)),
            )
        } else {
            (
                build_url(client.site.as_deref(), &client.token_url, &params)?,
                None,
            )
        };
        let fetched = match self
            .request(
                client.final_destination,
                client.timeout,
                client.token_method.clone(),
                &url,
                &headers,
                body,
            )
            .await?
        {
            Ok(f) => f,
            Err(failure) => return Ok(Err(failure)),
        };
        if (300..400).contains(&fetched.status) && fetched.status != 304 {
            return Err(Unsupported("a redirected OAuth token request").into());
        }
        if fetched.status >= 400 {
            return Ok(Err(Failure::new(
                "invalid_credentials",
                format!("token request failed: {}", fetched.status),
            )));
        }
        let Some(hash) =
            parse_body(&fetched.content_type, &fetched.body)?.filter(|h| !h.is_empty())
        else {
            return Ok(Err(Failure::new(
                "invalid_credentials",
                "the token response is not a hash",
            )));
        };
        Ok(access_token(hash, crate::clock::now().timestamp()))
    }

    /// `access_token.get(path).parsed`: a request with the bearer token,
    /// relative paths under the client's site.
    pub async fn get_with_token(
        &self,
        client: &Client,
        token: &str,
        path: &str,
        extra_headers: &[(String, String)],
    ) -> Result<Result<Value, Failure>, AppError> {
        let mut headers = vec![("Authorization".to_string(), format!("Bearer {token}"))];
        headers.extend(extra_headers.iter().cloned());
        self.resource(client, reqwest::Method::GET, path, &[], &headers, None)
            .await
    }

    /// `client.request(verb, path, params:, headers:, body:).parsed`: a
    /// request under the client's site, an error status failing as
    /// OAuth2::Error does (invalid_credentials).
    pub async fn resource(
        &self,
        client: &Client,
        method: reqwest::Method,
        path: &str,
        query: &[(String, String)],
        headers: &[(String, String)],
        body: Option<String>,
    ) -> Result<Result<Value, Failure>, AppError> {
        let url = build_url(client.site.as_deref(), path, query)?;
        let fetched = match self
            .request(
                client.final_destination,
                client.timeout,
                method,
                &url,
                headers,
                body,
            )
            .await?
        {
            Ok(f) => f,
            Err(failure) => return Ok(Err(failure)),
        };
        if fetched.status >= 400 {
            return Ok(Err(Failure::new(
                "invalid_credentials",
                format!("request to {url} failed: {}", fetched.status),
            )));
        }
        Ok(Ok(parsed_value(&fetched.content_type, &fetched.body)?))
    }
}

/// `OAuth2::Response#parsed` for any body: a JSON hash snake_cased, a JSON
/// array as it is, anything else as a string.
pub fn parsed_value(content_type: &str, body: &str) -> Result<Value, AppError> {
    if let Some(m) = parse_body(content_type, body)? {
        return Ok(Value::Object(m));
    }
    let ct = content_type.split(';').next().unwrap_or("").trim();
    let json = ct.is_empty() || ct.eq_ignore_ascii_case("application/json");
    Ok(match serde_json::from_str::<Value>(body) {
        Ok(v @ Value::Array(_)) if json => v,
        Ok(v) if ct.is_empty() => v,
        _ => Value::String(body.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_urls_as_faraday() {
        assert_eq!(
            build_url(
                Some("https://discord.com/api"),
                "oauth2/authorize",
                &[("b".into(), "x y".into()), ("a".into(), "1".into())]
            )
            .unwrap(),
            "https://discord.com/api/oauth2/authorize?a=1&b=x%20y"
        );
        assert_eq!(
            build_url(
                Some("https://api.linkedin.com"),
                "https://www.linkedin.com/oauth/v2/authorization?response_type=code",
                &[("client_id".into(), "c".into())]
            )
            .unwrap(),
            "https://www.linkedin.com/oauth/v2/authorization?client_id=c&response_type=code"
        );
        assert_eq!(
            build_url(Some("https://github.com"), "/login/oauth/authorize", &[]).unwrap(),
            "https://github.com/login/oauth/authorize"
        );
        assert_eq!(escape("a~b*c/d"), "a~b%2Ac%2Fd");
    }

    #[test]
    fn reads_tokens() {
        let mut hash = Map::new();
        hash.insert("accessToken".into(), Value::String("t".into()));
        hash.insert("expires_in".into(), Value::from(3600));
        hash.insert("scope".into(), Value::String("email".into()));
        let parsed = match snaky(Value::Object(hash)) {
            Value::Object(m) => m,
            _ => unreachable!(),
        };
        let token = access_token(parsed, 1000).unwrap();
        assert_eq!(token.token, "t");
        assert_eq!(token.expires_at, Some(4600));
        assert_eq!(
            token.params.get("scope"),
            Some(&Value::String("email".into()))
        );
        assert!(access_token(Map::new(), 0).is_err());
    }

    #[test]
    fn challenges() {
        // RFC 7636 appendix B.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
