//! The OAuth2 logins Discourse ships: core's GitHub (omniauth-github),
//! Google (omniauth-google-oauth2), Facebook (omniauth-facebook), Discord,
//! LinkedIn and Discourse ID authenticators, and the bundled
//! discourse-patreon plugin. Each
//! is an OmniAuth OAuth2 strategy (the request and callback phases are
//! routes/omniauth.rs's) with its client options, authorize params, the
//! requests that build its auth hash, and its authenticator's
//! after_authenticate.
//!
//! Refused: a Google id_token handed to the callback (its JWKS signature
//! check), a Google callback with a JSON body, and Facebook's signed
//! request cookie (the JS SDK's code).

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::auth::managed::{AuthHash, AuthResult, Behaviour, truthy};
use crate::auth::oauth2::{AccessToken, AuthScheme, Client, Failure, Http};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// Net::HTTP's open and read timeouts: Faraday sets none.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

const FACEBOOK_API_VERSION: &str = "v19.0";
const GOOGLE_USER_INFO_URL: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
const GOOGLE_TOKEN_INFO_URL: &str = "https://www.googleapis.com/oauth2/v3/tokeninfo";
const GOOGLE_OAUTH2_BASE_URL: &str = "https://oauth2.googleapis.com";
const GOOGLE_GROUPS_URL: &str = "https://admin.googleapis.com/admin/directory/v1/groups";
const GOOGLE_GROUPS_SCOPE: &str = "https://www.googleapis.com/auth/admin.directory.group.readonly";
/// GoogleOauth2's AUTHORIZE_OPTIONS, which request params may override.
const GOOGLE_AUTHORIZE_OPTIONS: [&str; 13] = [
    "access_type",
    "hd",
    "login_hint",
    "prompt",
    "request_visible_actions",
    "scope",
    "state",
    "redirect_uri",
    "include_granted_scopes",
    "enable_granular_consent",
    "openid_realm",
    "device_id",
    "device_name",
];

/// The strategies this module runs.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "github"
            | "google_oauth2"
            | "facebook"
            | "discord"
            | "linkedin_oidc"
            | "discourse_id"
            | "patreon"
    )
}

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

fn client_of(
    s: &SiteSettings,
    id: &str,
    secret: &str,
    site: &str,
    authorize_url: &str,
    token_url: &str,
    auth_scheme: AuthScheme,
) -> Result<Client, AppError> {
    Ok(Client {
        client_id: setting(s, id)?,
        client_secret: setting(s, secret)?,
        site: Some(site.to_string()),
        authorize_url: authorize_url.to_string(),
        token_url: token_url.to_string(),
        token_method: reqwest::Method::POST,
        auth_scheme,
        token_headers: Vec::new(),
        token_params: Vec::new(),
        final_destination: false,
        timeout: TIMEOUT,
    })
}

/// The strategy's client options, as its setup lambda leaves them.
pub fn client(s: &SiteSettings, name: &str) -> Result<Client, AppError> {
    use AuthScheme::{BasicAuth, RequestBody};
    Ok(match name {
        "github" => client_of(
            s,
            "github_client_id",
            "github_client_secret",
            "https://api.github.com",
            "https://github.com/login/oauth/authorize",
            "https://github.com/login/oauth/access_token",
            RequestBody,
        )?,
        "google_oauth2" => {
            // connection_build: FinalDestination's Faraday adapter.
            let mut c = client_of(
                s,
                "google_oauth2_client_id",
                "google_oauth2_client_secret",
                GOOGLE_OAUTH2_BASE_URL,
                "https://accounts.google.com/o/oauth2/auth",
                "/token",
                BasicAuth,
            )?;
            c.final_destination = true;
            c
        }
        "facebook" => client_of(
            s,
            "facebook_app_id",
            "facebook_app_secret",
            &format!("https://graph.facebook.com/{FACEBOOK_API_VERSION}"),
            &format!("https://www.facebook.com/{FACEBOOK_API_VERSION}/dialog/oauth"),
            "oauth/access_token",
            RequestBody,
        )?,
        "discord" => client_of(
            s,
            "discord_client_id",
            "discord_secret",
            "https://discord.com/api",
            "oauth2/authorize",
            "oauth2/token",
            BasicAuth,
        )?,
        "linkedin_oidc" => client_of(
            s,
            "linkedin_oidc_client_id",
            "linkedin_oidc_client_secret",
            "https://api.linkedin.com",
            "https://www.linkedin.com/oauth/v2/authorization?response_type=code",
            "https://www.linkedin.com/oauth/v2/accessToken",
            BasicAuth,
        )?,
        "discourse_id" => {
            let site = s
                .get("discourse_id_provider_url")?
                .presence()
                .unwrap_or_else(|| "https://id.discourse.com".to_string());
            client_of(
                s,
                "discourse_id_client_id",
                "discourse_id_client_secret",
                &site,
                "oauth/authorize",
                "oauth/token",
                BasicAuth,
            )?
        }
        "patreon" => client_of(
            s,
            "patreon_client_id",
            "patreon_client_secret",
            "https://www.patreon.com",
            "https://www.patreon.com/oauth2/authorize",
            if patreon_v2(s)? {
                "https://www.patreon.com/api/oauth2/token"
            } else {
                "https://api.patreon.com/oauth2/token"
            },
            RequestBody,
        )?,
        _ => return Err(Unsupported("an unknown built-in login").into()),
    })
}

/// Sets `key` in an ordered param list, replacing a value it has.
fn put(params: &mut Vec<(String, String)>, key: &str, value: String) {
    match params.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = value,
        None => params.push((key.to_string(), value)),
    }
}

/// The request phase's params besides the state and PKCE:
/// `options_for("authorize")` and the strategy's `authorize_params`
/// override. A `state` here (Google takes one from the request) is the
/// one kept in the session.
pub fn authorize_params(
    s: &SiteSettings,
    name: &str,
    request: &Map<String, Value>,
) -> Result<Vec<(String, String)>, AppError> {
    let param = |k: &str| crate::params::string(request, k);
    let mut out: Vec<(String, String)> = Vec::new();
    match name {
        "github" => {
            out.push(("scope".into(), "user:email".into()));
            for k in ["scope", "client_options"] {
                if let Some(v) = param(k) {
                    put(&mut out, k, v);
                }
            }
        }
        "google_oauth2" => {
            let hd = setting(s, "google_oauth2_hd")?;
            if !hd.trim().is_empty() {
                out.push(("hd".into(), hd));
            }
            let prompt = setting(s, "google_oauth2_prompt")?;
            if !prompt.trim().is_empty() {
                out.push(("prompt".into(), prompt.replace('|', " ")));
            }
            for k in GOOGLE_AUTHORIZE_OPTIONS {
                if let Some(v) = param(k).filter(|v| !v.is_empty()) {
                    put(&mut out, k, v);
                }
            }
            let scope = out
                .iter()
                .find(|(k, _)| k == "scope")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| "email,profile".to_string());
            put(&mut out, "scope", google_scope(&scope));
            if !out.iter().any(|(k, _)| k == "access_type") {
                out.push(("access_type".into(), "offline".into()));
            }
            if let Some(i) = out.iter().position(|(k, _)| k == "openid_realm") {
                let (_, v) = out.remove(i);
                out.push(("openid.realm".into(), v));
            }
        }
        "facebook" => {
            out.push(("scope".into(), "email".into()));
            for k in ["display", "scope", "auth_type", "config_id"] {
                if let Some(v) = param(k) {
                    put(&mut out, k, v);
                }
            }
        }
        "discord" => out.push(("scope".into(), "identify email guilds".into())),
        "linkedin_oidc" => out.push(("scope".into(), "openid profile email".into())),
        "discourse_id" => {
            out.push(("scope".into(), "read".into()));
            if param("signup").as_deref() == Some("true") {
                out.push(("intent".into(), "signup".into()));
                if let Some(email) = param("email").filter(|e| !e.trim().is_empty()) {
                    out.push(("email".into(), email));
                }
            }
        }
        "patreon" => {
            // The API version's authorize params replace the strategy's.
            out.push(("response_type".into(), "code".into()));
            if patreon_v2(s)? {
                out.push((
                    "scope".into(),
                    "identity identity[email] campaigns campaigns.members campaigns.members[email]"
                        .into(),
                ));
            }
        }
        _ => return Err(Unsupported("an unknown built-in login").into()),
    }
    Ok(out)
}

/// `Patreon::ApiVersion.current` is V2.
fn patreon_v2(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(setting(s, "patreon_api_version")? == "2")
}

/// `provider_ignores_state`: Patreon's hidden setting.
pub fn ignores_state(s: &SiteSettings, name: &str) -> Result<bool, AppError> {
    Ok(name == "patreon" && s.get("patreon_login_ignore_state")?.truthy())
}

/// GoogleOauth2#get_scope: commas and spaces split it; scopes other than
/// the base ones and URLs go under googleapis' auth URL.
fn google_scope(raw: &str) -> String {
    raw.split_whitespace()
        .flat_map(|item| item.split(','))
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with("http://")
                || s.starts_with("https://")
                || ["profile", "email", "openid"].contains(&s)
            {
                s.to_string()
            } else {
                format!("https://www.googleapis.com/auth/{s}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The strategy's `callback_url`. Discord, LinkedIn and Patreon add
/// `script_name` to a callback path that has it already;
/// OmniauthBypassMiddleware's compat patch takes the doubled one out, so
/// every one is the origin, the subfolder and the callback path.
pub fn callback_url(
    name: &str,
    base_url_no_prefix: &str,
    script_name: &str,
) -> Result<String, AppError> {
    Ok(format!(
        "{base_url_no_prefix}{script_name}/auth/{name}/callback"
    ))
}

/// What the callback phase needs before the state check: Facebook's
/// `with_authorization_code!`, which fails without a code.
pub fn before_callback(
    name: &str,
    params: &Map<String, Value>,
    client: &Client,
    headers: &axum::http::HeaderMap,
) -> Result<Option<Failure>, AppError> {
    if name != "facebook" || params.contains_key("code") {
        return Ok(None);
    }
    let cookie = format!("fbsr_{}", client.client_id);
    if crate::session::current::cookie(headers, &cookie).is_some() {
        return Err(Unsupported("Facebook's signed request cookie (the JS SDK's code)").into());
    }
    Ok(Some(Failure::new(
        "no_authorization_code",
        "must pass either a `code` (via URL or by an `fbsr_XXX` signed request cookie)",
    )))
}

/// `build_access_token`: the code exchanged, as each strategy does it.
#[allow(clippy::too_many_arguments)]
pub async fn build_access_token(
    http: &Http<'_>,
    s: &SiteSettings,
    name: &str,
    client: &Client,
    params: &Map<String, Value>,
    xhr: bool,
    json_body: bool,
    callback: &str,
) -> Result<Result<AccessToken, Failure>, AppError> {
    let param = |k: &str| crate::params::string(params, k);
    let code = param("code").unwrap_or_default();
    match name {
        "google_oauth2" => {
            let token = if params.contains_key("code") {
                let redirect = param("redirect_uri").unwrap_or_else(|| {
                    if xhr {
                        "postmessage".to_string()
                    } else {
                        callback.to_string()
                    }
                });
                match http.get_token(client, &code, &redirect, &[]).await? {
                    Ok(t) => t,
                    Err(f) => return Ok(Err(f)),
                }
            } else if let Some(given) = param("access_token") {
                if param("id_token").is_some_and(|t| !t.is_empty()) {
                    return Err(Unsupported("a Google id_token handed to the callback").into());
                }
                let info = match google_token_info(http, client, &given).await? {
                    Ok(i) => i,
                    Err(f) => return Ok(Err(f)),
                };
                if info["aud"].as_str() != Some(client.client_id.as_str()) {
                    return Ok(Err(no_credentials()));
                }
                AccessToken {
                    token: given,
                    refresh_token: None,
                    expires_at: None,
                    params: Map::new(),
                }
            } else if json_body {
                return Err(Unsupported("a Google callback with a JSON body").into());
            } else {
                return Ok(Err(no_credentials()));
            };
            // verify_hd
            let hd = setting(s, "google_oauth2_hd")?;
            if !hd.trim().is_empty() {
                let info = match google_user_info(http, client, &token.token).await? {
                    Ok(i) => i,
                    Err(f) => return Ok(Err(f)),
                };
                if hd != "*" && info["hd"].as_str() != Some(hd.as_str()) {
                    return Ok(Err(Failure::new(
                        "invalid_credentials",
                        "invalid_hd | Invalid Hosted Domain",
                    )));
                }
            }
            Ok(Ok(token))
        }
        "patreon" => {
            // options.redirect_uri: Discourse.base_url's callback.
            let redirect = format!("{}/auth/patreon/callback", http.fetcher.urls.base_url()?);
            http.get_token(client, &code, &redirect, &[]).await
        }
        _ => http.get_token(client, &code, callback, &[]).await,
    }
}

fn no_credentials() -> Failure {
    Failure::new(
        "invalid_credentials",
        "invalid_credentials | No valid credentials were supplied in the callback request",
    )
}

async fn google_token_info(
    http: &Http<'_>,
    client: &Client,
    token: &str,
) -> Result<Result<Value, Failure>, AppError> {
    let body = crate::auth::oauth2::encode_query(&[("access_token".into(), token.into())]);
    http.resource(
        client,
        reqwest::Method::POST,
        GOOGLE_TOKEN_INFO_URL,
        &[],
        &[(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        Some(body),
    )
    .await
}

async fn google_user_info(
    http: &Http<'_>,
    client: &Client,
    token: &str,
) -> Result<Result<Value, Failure>, AppError> {
    http.get_with_token(client, token, GOOGLE_USER_INFO_URL, &[])
        .await
}

/// omniauth-google-oauth2 and omniauth-facebook's `prune!`: nil and empty
/// values dropped, nested hashes first.
fn prune(map: &mut Map<String, Value>) {
    map.retain(|_, v| {
        if let Value::Object(inner) = v {
            prune(inner);
        }
        match v {
            Value::Null => false,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
            _ => true,
        }
    });
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// `raw_info[key]` of a parsed response that may not be a hash.
fn at<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.get(key).unwrap_or(&Value::Null)
}

/// `uid`'s value as the association keeps it (`to_s`).
fn uid_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// The OAuth2 strategy's credentials.
fn credentials(token: &AccessToken) -> Value {
    token.credentials()
}

macro_rules! fetch {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(f) => return Ok(Err(f)),
        }
    };
}

/// The strategy's `auth_hash`: credentials and extra first, then uid
/// and info, with the requests they make in that order.
pub async fn auth_hash(
    http: &Http<'_>,
    name: &str,
    client: &Client,
    token: &AccessToken,
) -> Result<Result<AuthHash, Failure>, AppError> {
    let bearer = |path: &'static str| http.get_with_token(client, &token.token, path, &[]);
    let hash = match name {
        "github" => {
            // extra: raw_info, then the emails (user:email is the scope).
            let raw_info = fetch!(bearer("user").await?);
            let emails = fetch!(
                http.get_with_token(
                    client,
                    &token.token,
                    "user/emails",
                    &[("Accept".into(), "application/vnd.github.v3".into())],
                )
                .await?
            );
            let primary = emails.as_array().and_then(|list| {
                list.iter()
                    .find(|e| truthy(e.get("primary")) && truthy(e.get("verified")))
                    .map(|e| at(e, "email").clone())
            });
            let mut info = Map::new();
            info.insert("nickname".into(), at(&raw_info, "login").clone());
            info.insert("email".into(), primary.unwrap_or(Value::Null));
            info.insert("name".into(), at(&raw_info, "name").clone());
            info.insert("image".into(), at(&raw_info, "avatar_url").clone());
            info.insert(
                "urls".into(),
                json!({
                    "GitHub": at(&raw_info, "html_url"),
                    "Blog": at(&raw_info, "blog"),
                }),
            );
            let mut extra = Map::new();
            extra.insert("raw_info".into(), raw_info.clone());
            extra.insert("all_emails".into(), emails);
            extra.insert(
                "scope".into(),
                token.params.get("scope").cloned().unwrap_or(Value::Null),
            );
            AuthHash {
                provider: name.into(),
                uid: uid_string(at(&raw_info, "id")),
                info,
                credentials: credentials(token),
                extra,
            }
        }
        "google_oauth2" => {
            // credentials: the token's scope from tokeninfo.
            let token_info = fetch!(google_token_info(http, client, &token.token).await?);
            let mut creds = obj(credentials(token));
            let mut scope = Map::new();
            scope.insert("scope".into(), at(&token_info, "scope").clone());
            prune(&mut scope);
            creds.extend(scope);
            let raw_info = fetch!(google_user_info(http, client, &token.token).await?);
            let mut extra = Map::new();
            extra.insert(
                "id_token".into(),
                token.params.get("id_token").cloned().unwrap_or(Value::Null),
            );
            extra.insert("raw_info".into(), raw_info.clone());
            prune(&mut extra);
            let verified = truthy(raw_info.get("email_verified"));
            let mut info = Map::new();
            info.insert("name".into(), at(&raw_info, "name").clone());
            info.insert(
                "email".into(),
                if verified {
                    at(&raw_info, "email").clone()
                } else {
                    Value::Null
                },
            );
            info.insert("unverified_email".into(), at(&raw_info, "email").clone());
            info.insert(
                "email_verified".into(),
                at(&raw_info, "email_verified").clone(),
            );
            info.insert("first_name".into(), at(&raw_info, "given_name").clone());
            info.insert("last_name".into(), at(&raw_info, "family_name").clone());
            info.insert(
                "image".into(),
                match at(&raw_info, "picture") {
                    Value::String(p) => Value::String(google_image_url(p)),
                    _ => Value::Null,
                },
            );
            info.insert("urls".into(), json!({"google": at(&raw_info, "profile")}));
            prune(&mut info);
            AuthHash {
                provider: name.into(),
                uid: uid_string(at(&raw_info, "sub")),
                info,
                credentials: Value::Object(creds),
                extra,
            }
        }
        "facebook" => {
            let proof = {
                use hmac::{Hmac, Mac};
                let mut mac = Hmac::<sha2::Sha256>::new_from_slice(client.client_secret.as_bytes())
                    .map_err(|_| Unsupported("an unusable Facebook app secret"))?;
                mac.update(token.token.as_bytes());
                mac.finalize()
                    .into_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            };
            let raw_info = fetch!(
                http.resource(
                    client,
                    reqwest::Method::GET,
                    "me",
                    &[
                        ("appsecret_proof".into(), proof),
                        ("fields".into(), "name,first_name,last_name,email".into()),
                    ],
                    &[("Authorization".into(), format!("OAuth {}", token.token))],
                    None,
                )
                .await?
            );
            let raw_info = match raw_info {
                Value::Null => json!({}),
                other => other,
            };
            let mut extra = Map::new();
            extra.insert("raw_info".into(), raw_info.clone());
            prune(&mut extra);
            let uid = at(&raw_info, "id").clone();
            // image_url: graph.facebook.com's picture over https, sized.
            let image = format!(
                "https://graph.facebook.com/{FACEBOOK_API_VERSION}/{}/picture?{}",
                uid_string(&uid),
                // Rack::Utils.build_query
                form_urlencoded::Serializer::new(String::new())
                    .extend_pairs([
                        ("access_token", token.token.as_str()),
                        ("width", "480"),
                        ("height", "480"),
                    ])
                    .finish()
            );
            let mut info = Map::new();
            info.insert("nickname".into(), at(&raw_info, "username").clone());
            info.insert("email".into(), at(&raw_info, "email").clone());
            info.insert("name".into(), at(&raw_info, "name").clone());
            info.insert("first_name".into(), at(&raw_info, "first_name").clone());
            info.insert("last_name".into(), at(&raw_info, "last_name").clone());
            info.insert("image".into(), Value::String(image));
            info.insert("description".into(), at(&raw_info, "bio").clone());
            info.insert(
                "urls".into(),
                json!({
                    "Facebook": at(&raw_info, "link"),
                    "Website": at(&raw_info, "website"),
                }),
            );
            info.insert(
                "location".into(),
                at(at(&raw_info, "location"), "name").clone(),
            );
            info.insert("verified".into(), at(&raw_info, "verified").clone());
            prune(&mut info);
            AuthHash {
                provider: name.into(),
                uid: uid_string(&uid),
                info,
                credentials: credentials(token),
                extra,
            }
        }
        "discord" => {
            let mut raw_info = obj(fetch!(bearer("users/@me").await?));
            let guilds = fetch!(bearer("users/@me/guilds").await?);
            // Merged into the SnakyHash: its hashes snake_cased too.
            raw_info.insert("guilds".into(), crate::auth::oauth2::snaky(guilds));
            let raw_info = Value::Object(raw_info);
            let id = at(&raw_info, "id").clone();
            let mut info = Map::new();
            info.insert(
                "name".into(),
                match at(&raw_info, "global_name") {
                    Value::Null | Value::Bool(false) => at(&raw_info, "username").clone(),
                    other => other.clone(),
                },
            );
            info.insert("nickname".into(), at(&raw_info, "username").clone());
            info.insert(
                "email".into(),
                if truthy(raw_info.get("verified")) {
                    at(&raw_info, "email").clone()
                } else {
                    Value::Null
                },
            );
            info.insert(
                "image".into(),
                Value::String(format!(
                    "https://cdn.discordapp.com/avatars/{}/{}",
                    ruby_to_s(&id),
                    ruby_to_s(at(&raw_info, "avatar"))
                )),
            );
            let mut extra = Map::new();
            extra.insert("raw_info".into(), raw_info.clone());
            AuthHash {
                provider: name.into(),
                uid: uid_string(&id),
                info,
                credentials: credentials(token),
                extra,
            }
        }
        "linkedin_oidc" => {
            let raw_info = fetch!(bearer("/v2/userinfo").await?);
            let mut info = Map::new();
            info.insert("email".into(), at(&raw_info, "email").clone());
            info.insert("first_name".into(), at(&raw_info, "given_name").clone());
            info.insert("last_name".into(), at(&raw_info, "family_name").clone());
            info.insert("image".into(), at(&raw_info, "picture").clone());
            let mut extra = Map::new();
            extra.insert("raw_info".into(), raw_info.clone());
            AuthHash {
                provider: name.into(),
                uid: uid_string(at(&raw_info, "sub")),
                info,
                credentials: credentials(token),
                extra,
            }
        }
        "discourse_id" => {
            // Everything comes with the token.
            let given = token.params.get("info").cloned().unwrap_or(Value::Null);
            let Value::Object(_) = given else {
                return Err(
                    Unsupported("a Discourse ID token without its info (NoMethodError)").into(),
                );
            };
            let mut info = Map::new();
            info.insert("nickname".into(), at(&given, "username").clone());
            info.insert("name".into(), at(&given, "name").clone());
            info.insert("email".into(), at(&given, "email").clone());
            info.insert("image".into(), at(&given, "image").clone());
            AuthHash {
                provider: name.into(),
                uid: uid_string(at(&given, "uuid")),
                info,
                credentials: credentials(token),
                extra: Map::new(),
            }
        }
        "patreon" => {
            // client.request(:get, identity_url, parse: :json): JSON
            // whatever the content type, an error status raising.
            let url = if patreon_v2(http.fetcher.settings)? {
                "https://www.patreon.com/api/oauth2/v2/identity?fields%5Buser%5D=email,full_name,is_email_verified"
            } else {
                "https://api.patreon.com/oauth2/api/current_user"
            };
            let fetched = fetch!(
                http.request(
                    client.final_destination,
                    client.timeout,
                    reqwest::Method::GET,
                    url,
                    &[("Authorization".into(), format!("Bearer {}", token.token))],
                    None,
                )
                .await?
            );
            if fetched.status >= 400 {
                return Ok(Err(Failure::new(
                    "invalid_credentials",
                    format!("request to {url} failed: {}", fetched.status),
                )));
            }
            let raw_info: Value = serde_json::from_str(&fetched.body)
                .map(crate::auth::oauth2::snaky)
                .map_err(|_| {
                    Unsupported("a Patreon identity that is not JSON (JSON::ParserError)")
                })?;
            let data = at(&raw_info, "data");
            let attributes = at(data, "attributes");
            if !attributes.is_object() {
                return Err(Unsupported(
                    "a Patreon identity without its attributes (NoMethodError)",
                )
                .into());
            }
            let mut info = Map::new();
            info.insert("email".into(), at(attributes, "email").clone());
            info.insert(
                "email_verified".into(),
                at(attributes, "is_email_verified").clone(),
            );
            info.insert("name".into(), at(attributes, "full_name").clone());
            let mut extra = Map::new();
            extra.insert("raw_info".into(), raw_info.clone());
            AuthHash {
                provider: name.into(),
                uid: ruby_to_s(at(data, "id")),
                info,
                credentials: credentials(token),
                extra,
            }
        }
        _ => return Err(Unsupported("an unknown built-in login").into()),
    };
    Ok(Ok(hash))
}

/// Ruby's `to_s` in string interpolation: nil is empty.
fn ruby_to_s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// GoogleOauth2#image_url: the size suffix taken off the path, the `sz`
/// query param dropped.
fn google_image_url(picture: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(picture) else {
        return picture.to_string();
    };
    let re = regex::Regex::new(r"^(.*)(=((w[0-9]*|h[0-9]*|s[0-9]*|c|p)-?)*)$").expect("regex");
    let path = url.path().to_string();
    if let Some(c) = re.captures(&path) {
        let kept = c.get(1).map(|m| m.as_str()).unwrap_or_default().to_string();
        url.set_path(&kept);
    }
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != "sz")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        let encoded = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(kept)
            .finish();
        url.set_query(Some(&encoded));
    }
    url.to_string()
}

/// GoogleOAuth2Authenticator#provides_groups?
pub fn google_provides_groups(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(!setting(s, "google_oauth2_hd")?.trim().is_empty()
        && s.get("google_oauth2_hd_groups")?.truthy()
        && !setting(s, "google_oauth2_hd_groups_service_account_admin_email")?
            .trim()
            .is_empty()
        && !setting(s, "google_oauth2_hd_groups_service_account_json")?
            .trim()
            .is_empty())
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `JWT.encode(payload, key, "RS256", headers)`.
fn jwt_rs256(payload: &Value, pem: &str) -> Result<String, AppError> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    use sha2::Digest;
    let key = rsa::RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| rsa::RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|_| Unsupported("a service account key OpenSSL can't read"))?;
    let header = json!({"alg": "RS256", "typ": "JWT"});
    let signing_input = format!(
        "{}.{}",
        b64url(header.to_string().as_bytes()),
        b64url(payload.to_string().as_bytes())
    );
    // PKCS#1 v1.5 over SHA-256: the DigestInfo prefix, then the digest.
    let mut digest_info = vec![
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    digest_info.extend_from_slice(&sha2::Sha256::digest(signing_input.as_bytes()));
    let signature = key
        .sign(rsa::Pkcs1v15Sign::new_unprefixed(), &digest_info)
        .map_err(|_| Unsupported("signing with the service account key"))?;
    Ok(format!("{signing_input}.{}", b64url(&signature)))
}

/// GoogleOAuth2Authenticator#raw_groups: the user's groups through a
/// service account's token, page by page; none when the token can't be
/// had, as far as it got when a page fails.
async fn google_raw_groups(
    http: &Http<'_>,
    s: &SiteSettings,
    uid: &str,
) -> Result<Option<Vec<Value>>, AppError> {
    let info: Value =
        serde_json::from_str(&setting(s, "google_oauth2_hd_groups_service_account_json")?)
            .map_err(|_| Unsupported("an unreadable service account JSON (JSON::ParserError)"))?;
    let now = crate::clock::now().timestamp();
    let payload = json!({
        "iss": info["client_email"],
        "aud": format!("{GOOGLE_OAUTH2_BASE_URL}/token"),
        "scope": GOOGLE_GROUPS_SCOPE,
        "iat": now,
        "exp": now + 60,
        "sub": setting(s, "google_oauth2_hd_groups_service_account_admin_email")?,
    });
    let jwt = jwt_rs256(&payload, info["private_key"].as_str().unwrap_or_default())?;
    // A plain OAuth2::Client: Basic auth, no FinalDestination.
    let mut client = client(s, "google_oauth2")?;
    client.final_destination = false;
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", client.client_id, client.client_secret));
    let body = crate::auth::oauth2::encode_query(&[
        (
            "grant_type".into(),
            "urn:ietf:params:oauth:grant-type:jwt-bearer".into(),
        ),
        ("assertion".into(), jwt),
    ]);
    let fetched = http
        .request(
            false,
            TIMEOUT,
            reqwest::Method::POST,
            &format!("{GOOGLE_OAUTH2_BASE_URL}/token"),
            &[
                ("Authorization".into(), format!("Basic {basic}")),
                (
                    "Content-Type".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
            ],
            Some(body),
        )
        .await?;
    // Faraday's connection errors are not rescued here: a 500.
    let Ok(fetched) = fetched else {
        return Err(Unsupported("the Google group token request failing to connect").into());
    };
    if fetched.status != 200 {
        tracing::error!(
            "[Discourse Google OAuth2] failed to retrieve group fetch token - status {}",
            fetched.status
        );
        return Ok(None);
    }
    let Some(hash) = crate::auth::oauth2::parse_body(&fetched.content_type, &fetched.body)? else {
        return Err(Unsupported("a group token response that is not a hash").into());
    };
    let token = match crate::auth::oauth2::access_token(hash, now) {
        Ok(t) => t,
        Err(_) => return Err(Unsupported("a group token response without a token").into()),
    };
    let mut groups = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut query = vec![("userKey".to_string(), uid.to_string())];
        if let Some(p) = &page_token {
            query.push(("pageToken".into(), p.clone()));
        }
        let url = crate::auth::oauth2::build_url(None, GOOGLE_GROUPS_URL, &query)?;
        let fetched = http
            .request(
                false,
                TIMEOUT,
                reqwest::Method::GET,
                &url,
                &[("Authorization".into(), format!("Bearer {}", token.token))],
                None,
            )
            .await?;
        let Ok(fetched) = fetched else {
            return Err(Unsupported("the Google groups request failing to connect").into());
        };
        if fetched.status != 200 {
            tracing::error!(
                "[Discourse Google OAuth2] failed to retrieve groups for {uid} - status {}",
                fetched.status
            );
            break;
        }
        let page = crate::auth::oauth2::parsed_value(&fetched.content_type, &fetched.body)?;
        if let Some(list) = page.get("groups").and_then(Value::as_array) {
            groups.extend(list.iter().cloned());
        }
        page_token = match page.get("next_page_token") {
            Some(Value::String(t)) => Some(t.clone()),
            Some(Value::Null) | None => None,
            Some(other) => Some(other.to_string()),
        };
        if page_token.is_none() {
            break;
        }
    }
    Ok(Some(groups))
}

/// GithubAuthenticator#all_github_emails: the primary first.
fn github_emails(auth: &AuthHash) -> Vec<Value> {
    let mut emails: Vec<Value> = auth
        .extra
        .get("all_emails")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(i) = emails.iter().position(|e| truthy(e.get("primary"))) {
        let primary = emails.remove(i);
        emails.insert(0, primary);
    }
    emails
}

/// The authenticator's `after_authenticate`.
pub async fn after_authenticate(
    conn: &mut PgConnection,
    s: &SiteSettings,
    host: &crate::pretty_text::Host,
    i18n: &crate::i18n::I18n,
    http: &Http<'_>,
    mut auth: AuthHash,
    existing_account: Option<i32>,
) -> Result<(AuthResult, AuthHash), AppError> {
    let name = auth.provider.clone();
    // DiscordAuthenticator: members of the trusted guilds only.
    if name == "discord" {
        let allowed: Vec<String> = setting(s, "discord_trusted_guilds")?
            .split('|')
            .filter(|g| !g.is_empty())
            .map(str::to_string)
            .collect();
        if !allowed.is_empty() {
            let member = auth
                .extra
                .get("raw_info")
                .and_then(|r| r.get("guilds"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|g| g.get("id").map(ruby_to_s))
                .any(|id| allowed.contains(&id));
            if !member {
                let reason = i18n
                    .t("discord.not_in_allowed_guild")
                    .unwrap_or("Membership of the Discord guild is required.")
                    .to_string();
                return Ok((AuthResult::failed(reason), auth));
            }
        }
    }
    // GoogleOAuth2Authenticator: the groups fetched first and kept in extra.
    let mut groups = None;
    if name == "google_oauth2" && google_provides_groups(s)? {
        groups = google_raw_groups(http, s, &auth.uid).await?;
        if let Some(g) = &groups {
            auth.extra
                .insert("raw_groups".into(), Value::Array(g.clone()));
        }
    }
    if auth.uid.is_empty() {
        return Err(Unsupported("a login without a user id (provider_uid can't be blank)").into());
    }
    let email_verified = match name.as_str() {
        "google_oauth2" => truthy(
            auth.extra
                .get("raw_info")
                .and_then(|r| r.get("email_verified")),
        ),
        "patreon" => truthy(auth.info.get("email_verified")),
        _ => true,
    };
    // GithubAuthenticator#find_user_by_email: any verified address.
    let user_by_email = if name == "github" {
        let mut found = None;
        for candidate in github_emails(&auth) {
            if !truthy(candidate.get("verified")) {
                continue;
            }
            let Some(email) = candidate.get("email").and_then(Value::as_str) else {
                continue;
            };
            if let Some(user) = crate::auth::managed::user_by_email(conn, email).await? {
                found = Some(user);
                break;
            }
        }
        Some(found)
    } else {
        None
    };
    let behaviour = Behaviour {
        email_verified,
        match_by_email: true,
        overrides_email: false,
        can_connect: true,
        user_by_email,
    };
    let mut result = crate::auth::managed::after_authenticate(
        conn,
        s,
        host,
        &auth,
        &behaviour,
        existing_account,
    )
    .await?;
    if name == "github" && result.user_id.is_none() {
        // A secondary address the domain rules allow.
        for candidate in github_emails(&auth) {
            let Some(email) = candidate.get("email").and_then(Value::as_str) else {
                continue;
            };
            if !crate::signup::email_allowed(s, email)? {
                continue;
            }
            result.set_email(Some(email.to_string()));
            result.email_valid = truthy(candidate.get("verified"));
            break;
        }
    }
    if let Some(groups) = groups {
        result.associated_groups = Some(
            groups
                .iter()
                .map(|g| (ruby_to_s(at(g, "id")), ruby_to_s(at(g, "name"))))
                .collect(),
        );
    }
    Ok((result, auth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_scopes() {
        assert_eq!(google_scope("email,profile"), "email profile");
        assert_eq!(
            google_scope("email admin.directory.group.readonly https://x.y/z"),
            "email https://www.googleapis.com/auth/admin.directory.group.readonly https://x.y/z"
        );
    }

    #[test]
    fn google_images() {
        assert_eq!(
            google_image_url("https://lh3.googleusercontent.com/a/abc=s96-c"),
            "https://lh3.googleusercontent.com/a/abc"
        );
        assert_eq!(
            google_image_url("https://lh3.googleusercontent.com/a/photo.jpg?sz=50"),
            "https://lh3.googleusercontent.com/a/photo.jpg"
        );
        assert_eq!(
            google_image_url("https://x.y/p.jpg?sz=50&a=1"),
            "https://x.y/p.jpg?a=1"
        );
    }

    #[test]
    fn prunes() {
        let mut m = obj(json!({"a": null, "b": "", "c": {"d": null}, "e": false, "f": "x"}));
        prune(&mut m);
        assert_eq!(Value::Object(m), json!({"e": false, "f": "x"}));
    }

    #[test]
    fn callback_urls() {
        assert_eq!(
            callback_url("discord", "https://f.x", "/forum").unwrap(),
            "https://f.x/forum/auth/discord/callback"
        );
        assert_eq!(
            callback_url("github", "https://f.x", "/forum").unwrap(),
            "https://f.x/forum/auth/github/callback"
        );
    }
}
