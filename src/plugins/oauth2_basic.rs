//! discourse-oauth2-basic: logins through any OAuth2 provider the
//! `oauth2_*` settings describe (OAuth2BasicAuthenticator and its
//! OmniAuth strategy). The user's id and details come from the token
//! response (`oauth2_callback_*` paths) or, by default, from a user JSON
//! endpoint walked with the `oauth2_json_*` paths.
//!
//! Refused: the plugin registers no additional or required JSON paths
//! here (`DiscoursePluginRegistry.oauth2_basic_*_json_paths` is filled by
//! other plugins only).

use serde_json::{Map, Value};

use crate::auth::managed::{AuthHash, AuthResult, Behaviour};
use crate::auth::oauth2::{AccessToken, AuthScheme, Client, Http};
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `GlobalSetting.oauth2_request_timeout_seconds`
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn setting(s: &SiteSettings, name: &str) -> Result<String, AppError> {
    Ok(s.get(name)?.to_s())
}

/// The strategy's options as its setup lambda sets them.
pub fn client(s: &SiteSettings, signup: bool) -> Result<Client, AppError> {
    let mut authorize_url = setting(s, "oauth2_authorize_url")?;
    let signup_url = setting(s, "oauth2_authorize_signup_url")?;
    if !signup_url.trim().is_empty() && signup {
        authorize_url = signup_url;
    }
    let header = s.get("oauth2_send_auth_header")?.truthy();
    let body = s.get("oauth2_send_auth_body")?.truthy();
    let client_id = setting(s, "oauth2_client_id")?;
    let client_secret = setting(s, "oauth2_client_secret")?;
    let mut token_headers = Vec::new();
    let auth_scheme = if header && body {
        // Both, for compatibility: the body's params and a Basic header.
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{client_id}:{client_secret}"));
        token_headers.push(("Authorization".to_string(), format!("Basic {encoded}")));
        AuthScheme::RequestBody
    } else if header {
        AuthScheme::BasicAuth
    } else {
        AuthScheme::RequestBody
    };
    let token_method = match setting(s, "oauth2_token_url_method")?.as_str() {
        "GET" => reqwest::Method::GET,
        "PUT" => reqwest::Method::PUT,
        _ => reqwest::Method::POST,
    };
    Ok(Client {
        client_id,
        client_secret,
        site: None,
        authorize_url,
        token_url: setting(s, "oauth2_token_url")?,
        token_method,
        auth_scheme,
        token_headers,
        token_params: Vec::new(),
        final_destination: true,
        timeout: TIMEOUT,
    })
}

/// The request phase's params besides state: `options_for("authorize")`
/// over `oauth2_authorize_options` (only `scope` has a value here).
pub fn authorize_params(s: &SiteSettings) -> Result<Vec<(String, String)>, AppError> {
    let mut out = Vec::new();
    let scope = setting(s, "oauth2_scope")?;
    let options = setting(s, "oauth2_authorize_options")?;
    for option in options.split('|') {
        if option == "scope" && !scope.trim().is_empty() {
            out.push(("scope".to_string(), scope.clone()));
        }
    }
    Ok(out)
}

/// `provider_ignores_state`
pub fn ignores_state(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(s.get("oauth2_disable_csrf")?.truthy())
}

/// `recurse(access_token, keys)`: the token's own attributes, then into
/// its params.
fn recurse(token: &AccessToken, keys: &[&str]) -> Option<Value> {
    let (first, rest) = keys.split_first()?;
    let value = match *first {
        "token" => Value::String(token.token.clone()),
        "refresh_token" => token
            .refresh_token
            .clone()
            .map(Value::String)
            .unwrap_or(Value::Null),
        "expires_at" => token.expires_at.map(Value::from).unwrap_or(Value::Null),
        "params" => Value::Object(token.params.clone()),
        other => token.params.get(other).cloned().unwrap_or(Value::Null),
    };
    let mut current = value;
    for k in rest {
        current = match current {
            Value::Object(m) => m.get(*k).cloned().unwrap_or(Value::Null),
            Value::Null => return None,
            _ => return None,
        };
    }
    Some(current)
}

/// The strategy's auth hash: uid and info from the token response's
/// paths, the credentials, no extra.
pub fn auth_hash(s: &SiteSettings, token: &AccessToken) -> Result<AuthHash, AppError> {
    let id_path = setting(s, "oauth2_callback_user_id_path")?;
    let path: Vec<&str> = id_path.split('.').filter(|p| !p.is_empty()).collect();
    let uid = if path.is_empty() {
        None
    } else {
        recurse(token, &path)
    };
    let mut info = Map::new();
    for p in setting(s, "oauth2_callback_user_info_paths")?.split('|') {
        let segments: Vec<&str> = p.split(':').collect();
        if segments.len() == 2 {
            let keys: Vec<&str> = segments[1].split('.').collect();
            info.insert(
                segments[0].to_string(),
                recurse(token, &keys).unwrap_or(Value::Null),
            );
        }
    }
    Ok(AuthHash {
        provider: "oauth2_basic".into(),
        uid: match uid {
            Some(Value::String(s)) => s,
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        },
        info,
        credentials: token.credentials(),
        extra: Map::new(),
    })
}

/// ActiveSupport's `blank?` of a JSON value.
fn blank(v: &Value) -> bool {
    match v {
        Value::Null | Value::Bool(false) => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// `parse_segments(path)`: dots split it, double quotes keep them, a
/// backslash escapes the next character.
pub fn parse_segments(path: &str) -> Vec<String> {
    let mut segments = vec![String::new()];
    let mut quoted = false;
    let mut escaped = false;
    for c in path.chars() {
        let mut next_escaped = false;
        if !escaped && c == '"' {
            quoted = !quoted;
        } else if !escaped && !quoted && c == '.' {
            segments.push(String::new());
        } else if !escaped && c == '\\' {
            next_escaped = true;
        } else {
            segments.last_mut().expect("a segment").push(c);
        }
        escaped = next_escaped;
    }
    segments
}

/// `walk_path(fragment, segments, seg_index)`
pub fn walk_path(
    fragment: &Value,
    segments: &[String],
    seg_index: usize,
) -> Result<Value, AppError> {
    let Some(first) = segments.get(seg_index) else {
        return Ok(Value::Null);
    };
    if first.trim().is_empty() || blank(fragment) {
        return Ok(Value::Null);
    }
    if !fragment.is_object() && !fragment.is_array() {
        return Ok(Value::Null);
    }
    let has_digit = |s: &str| s.chars().any(|c| c.is_ascii_digit() || c == '+');
    let key = if has_digit(first) {
        first.split('[').next().unwrap_or("").to_string()
    } else {
        first.clone()
    };
    let deref = match fragment {
        Value::Object(m) => m.get(&key).cloned().unwrap_or(Value::Null),
        Value::Array(items) => {
            let mut index = 0usize;
            if seg_index > 0 {
                // The previous segment's first digit.
                if let Some(d) = segments[seg_index - 1]
                    .chars()
                    .find(|c| c.is_ascii_digit() || *c == '+')
                {
                    index = d.to_digit(10).unwrap_or(0) as usize;
                }
            }
            if !items.is_empty() && items.len() as i64 >= index as i64 - 1 {
                match items.get(index) {
                    Some(Value::Object(m)) => m.get(&key).cloned().unwrap_or(Value::Null),
                    Some(Value::Array(_)) => {
                        return Err(Unsupported("indexing an array by name (TypeError)").into());
                    }
                    _ => {
                        return Err(Unsupported(
                            "a user JSON path past the array's end (NoMethodError)",
                        )
                        .into());
                    }
                }
            } else {
                Value::Null
            }
        }
        _ => Value::Null,
    };
    if blank(&deref) || seg_index == segments.len() - 1 {
        Ok(deref)
    } else {
        walk_path(&deref, segments, seg_index + 1)
    }
}

/// `json_walk(result, user_json, prop)`: the value at the setting's path,
/// a blank one as nil unless it is false (or an empty string or hash).
fn json_walk(path: &str, user_json: &Value) -> Result<Option<Value>, AppError> {
    if path.trim().is_empty() {
        return Ok(None);
    }
    let path = path.replace(".[].", ".").replace(".[", "[");
    let segments = parse_segments(&path);
    let val = walk_path(user_json, &segments, 0)?;
    Ok(Some(if !blank(&val) {
        val
    } else if val == Value::Array(Vec::new()) {
        Value::Null
    } else {
        val
    }))
}

/// What fetch_user_details found.
#[derive(Default)]
struct Details {
    values: Map<String, Value>,
    user_field_values: Map<String, Value>,
    groups: Option<Vec<Value>>,
}

/// `groups_from(user_json)`
fn groups_from(s: &SiteSettings, user_json: &Value) -> Result<Vec<Value>, AppError> {
    let path = setting(s, "oauth2_json_groups_path")?;
    let expanded = path.replace(".[].", ".").replace(".[", "[");
    let value = walk_path(user_json, &parse_segments(&expanded), 0)?;
    Ok(match value {
        Value::Array(items) => items,
        _ => Vec::new(),
    })
}

/// `user_field_values_from(user_json)`: each mapping's value (arrays
/// joined with commas), by field id.
fn user_field_values_from(
    s: &SiteSettings,
    user_json: &Value,
) -> Result<Map<String, Value>, AppError> {
    let raw = setting(s, "oauth2_user_field_mappings")?;
    let raw = if raw.trim().is_empty() {
        "[]".to_string()
    } else {
        raw
    };
    let Ok(Value::Array(mappings)) = serde_json::from_str::<Value>(&raw) else {
        return Ok(Map::new());
    };
    let mut out = Map::new();
    for mapping in mappings {
        let path = match &mapping["path"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let field_id = &mapping["user_field_id"];
        if path.trim().is_empty() || blank(field_id) {
            continue;
        }
        let expanded = path.replace(".[].", ".").replace(".[", "[");
        let value = walk_path(user_json, &parse_segments(&expanded), 0)?;
        if value.is_null() {
            continue;
        }
        let text = match value {
            Value::Array(items) => items
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
            Value::String(s) => s,
            other => other.to_string(),
        };
        let key = match field_id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.insert(key, Value::String(text));
    }
    Ok(out)
}

/// `fetch_user_details(token, id)`: the user JSON, walked; None when the
/// request fails or does not answer 200.
async fn fetch_user_details(
    s: &SiteSettings,
    http: &Http<'_>,
    token: &str,
    id: &str,
) -> Result<Option<Details>, AppError> {
    let url = setting(s, "oauth2_user_json_url")?
        .replacen(":token", token, 1)
        .replacen(":id", id, 1);
    let method = match setting(s, "oauth2_user_json_url_method")?.as_str() {
        "POST" => reqwest::Method::POST,
        _ => reqwest::Method::GET,
    };
    let headers = vec![
        ("Authorization".to_string(), format!("Bearer {token}")),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    let fetched = match http
        .request(true, TIMEOUT, method, &url, &headers, None)
        .await?
    {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    if fetched.status != 200 {
        return Ok(None);
    }
    let user_json: Value = serde_json::from_str(&fetched.body)
        .map_err(|_| Unsupported("a user JSON response that is not JSON (JSON::ParserError)"))?;
    let mut details = Details::default();
    if !blank(&user_json) {
        for prop in [
            "user_id",
            "username",
            "name",
            "email",
            "email_verified",
            "avatar",
        ] {
            let path = setting(s, &format!("oauth2_json_{prop}_path"))?;
            if let Some(v) = json_walk(&path, &user_json)? {
                details.values.insert(prop.to_string(), v);
            }
        }
        details.user_field_values = user_field_values_from(s, &user_json)?;
        if provides_groups(s)? {
            details.groups = Some(groups_from(s, &user_json)?);
        }
    }
    Ok(Some(details))
}

/// `provides_groups?`
pub fn provides_groups(s: &SiteSettings) -> Result<bool, AppError> {
    Ok(!setting(s, "oauth2_json_groups_path")?.trim().is_empty())
}

/// `primary_email_verified?(auth)`
fn email_verified(s: &SiteSettings, auth: &AuthHash) -> Result<bool, AppError> {
    if s.get("oauth2_email_verified")?.truthy() {
        return Ok(true);
    }
    Ok(crate::auth::managed::true_or_string_true(
        auth.info.get("email_verified"),
    ))
}

/// OAuth2BasicAuthenticator#after_authenticate: the user details fetched
/// into the auth hash, then ManagedAuthenticator's, with the user field
/// values and groups found.
pub async fn after_authenticate(
    conn: &mut sqlx::PgConnection,
    s: &SiteSettings,
    host: &crate::pretty_text::Host,
    i18n: &crate::i18n::I18n,
    http: &Http<'_>,
    mut auth: AuthHash,
    existing_account: Option<i32>,
) -> Result<(AuthResult, AuthHash), AppError> {
    let mut fetched: Option<Details> = None;
    if s.get("oauth2_fetch_user_details")?.truthy()
        && !setting(s, "oauth2_user_json_url")?.trim().is_empty()
    {
        let token = auth.credentials["token"].as_str().unwrap_or("").to_string();
        match fetch_user_details(s, http, &token, &auth.uid).await? {
            Some(details) => {
                let truthy = |v: &Value| !matches!(v, Value::Null | Value::Bool(false));
                if let Some(id) = details.values.get("user_id").filter(|v| truthy(v)) {
                    auth.uid = match id {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                }
                if let Some(v) = details.values.get("username").filter(|v| truthy(v)) {
                    auth.info.insert("nickname".into(), v.clone());
                }
                if let Some(v) = details.values.get("avatar").filter(|v| truthy(v)) {
                    auth.info.insert("image".into(), v.clone());
                }
                for prop in ["name", "email", "email_verified"] {
                    if let Some(v) = details.values.get(prop).filter(|v| truthy(v)) {
                        auth.info.insert(prop.into(), v.clone());
                    }
                }
                fetched = Some(details);
            }
            None => {
                return Ok((
                    AuthResult::failed(
                        i18n.t("login.authenticator_error_fetch_user_details")
                            .unwrap_or_default(),
                    ),
                    auth,
                ));
            }
        }
    }
    if auth.uid.is_empty() {
        return Err(
            Unsupported("an OAuth2 login without a user id (provider_uid can't be blank)").into(),
        );
    }
    let behaviour = Behaviour {
        email_verified: email_verified(s, &auth)?,
        match_by_email: true,
        overrides_email: s.get("oauth2_overrides_email")?.truthy(),
        can_connect: s.get("oauth2_allow_association_change")?.truthy(),
        user_by_email: None,
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
    if let Some(details) = fetched {
        result.user_field_values = Some(details.user_field_values);
        if provides_groups(s)? {
            let groups = details.groups.unwrap_or_default();
            result.associated_groups = Some(
                groups
                    .iter()
                    .map(|g| {
                        let v = match g {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        (v.clone(), v)
                    })
                    .collect(),
            );
        }
    }
    Ok((result, auth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn walks_paths() {
        let user =
            json!({"user": {"id": 7, "emails": [{"value": "a@b.c"}], "name": ""}, "ok": false});
        let walk = |p: &str| json_walk(p, &user).unwrap();
        assert_eq!(walk("user.id"), Some(json!(7)));
        assert_eq!(walk("user.emails[0].value"), Some(json!("a@b.c")));
        assert_eq!(walk("user.emails.[0].value"), Some(json!("a@b.c")));
        assert_eq!(walk("user.name"), Some(json!("")));
        assert_eq!(walk("ok"), Some(json!(false)));
        assert_eq!(walk("missing"), Some(Value::Null));
        assert_eq!(walk(""), None);
        assert_eq!(
            parse_segments(r#"a."b.c".d\.e"#),
            vec!["a".to_string(), "b.c".into(), "d.e".into()]
        );
    }
}
