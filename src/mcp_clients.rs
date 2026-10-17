//! The MCP OAuth clients as Admin::McpClientsController reads and writes
//! them: McpOauthClientsPage, the serializer, a pre-registered client's
//! creation with McpOauthClient's validations, blocking (its active
//! authorizations revoked), and DiscourseMcp::OAuth::ClientResolver's
//! refresh of a client ID metadata document (fetched through
//! crate::final_destination, so a client ID can't reach the site's own
//! network).

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

use crate::AppError;
use crate::final_destination::{Fetcher, Options, Read};
use crate::site_settings::SiteSettings;

/// `ClientResolver::MAX_METADATA_BYTES`
const MAX_METADATA_BYTES: usize = 64 * 1024;
/// `McpOauthClient::LOCAL_REDIRECT_HOSTS`
const LOCAL_REDIRECT_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

#[derive(sqlx::FromRow)]
pub struct Client {
    pub id: i64,
    pub client_id: String,
    pub name: String,
    pub registration_type: String,
    pub trust_state: String,
    pub metadata_uri: Option<String>,
    pub metadata_hash: Option<String>,
    pub redirect_uris: Vec<String>,
    pub created_at: NaiveDateTime,
    pub last_seen_at: Option<NaiveDateTime>,
    pub authorization_count: i64,
}

const COLUMNS: &str = "c.id, c.client_id, c.name, c.registration_type, c.trust_state, \
     c.metadata_uri, c.metadata_hash, c.redirect_uris, c.created_at, c.last_seen_at, \
     (SELECT count(*) FROM mcp_oauth_authorizations a WHERE a.mcp_oauth_client_id = c.id) \
     AS authorization_count";

/// `URI.parse(value).host`: None for a relative or unparsable value.
fn uri_host(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    url.host_str().map(str::to_string)
}

/// Admin::McpClientsController#serialize
pub fn serialize(c: &Client) -> Value {
    let mut hosts: Vec<String> = Vec::new();
    for host in c.redirect_uris.iter().filter_map(|u| uri_host(u)) {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    let time = |t: Option<NaiveDateTime>| {
        t.map_or(Value::Null, |t| json!(crate::topic_list::time_json(t)))
    };
    json!({
        "id": c.id,
        "client_id": c.client_id,
        "name": c.name,
        "domain": uri_host(&c.client_id),
        "registration_type": c.registration_type,
        "trust_state": c.trust_state,
        "blocked": c.trust_state == "blocked",
        "redirect_uris": c.redirect_uris,
        "redirect_hosts": hosts,
        "metadata_uri": c.metadata_uri,
        "first_seen_at": time(Some(c.created_at)),
        "last_seen_at": time(c.last_seen_at),
        "authorization_count": c.authorization_count,
    })
}

pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<Client>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM mcp_oauth_clients c WHERE c.id = $1"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await
}

/// McpOauthClientsPage: newest first, after the cursor, filtered by
/// name, client ID or registration type; the next cursor when more.
pub async fn page(
    conn: &mut PgConnection,
    limit: i64,
    cursor: Option<i64>,
    filter: &str,
) -> Result<Value, AppError> {
    let mut sql = format!("SELECT {COLUMNS} FROM mcp_oauth_clients c WHERE TRUE");
    if cursor.is_some() {
        sql.push_str(" AND c.id < $2");
    }
    if !filter.is_empty() {
        sql.push_str(
            " AND (c.name ILIKE $1 OR c.client_id ILIKE $1 OR c.registration_type ILIKE $1)",
        );
    }
    sql.push_str(" ORDER BY c.id DESC LIMIT $3");
    let pattern = format!("%{}%", crate::ruby::sanitize_sql_like(filter));
    let mut rows: Vec<Client> = sqlx::query_as(&sql)
        .bind(&pattern)
        .bind(cursor.unwrap_or(0))
        .bind(limit + 1)
        .fetch_all(&mut *conn)
        .await?;
    let next_cursor = if rows.len() as i64 > limit {
        rows.pop();
        rows.last().map(|r| r.id)
    } else {
        None
    };
    Ok(json!({
        "clients": rows.iter().map(serialize).collect::<Vec<_>>(),
        "meta": { "next_cursor": next_cursor },
    }))
}

/// `McpOauthClient.valid_redirect_uri?`: absolute with a host, no user
/// info or fragment; https, or http to the local machine.
pub fn valid_redirect_uri(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if host.is_empty()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    if url.scheme() == "https" {
        return true;
    }
    let hostname = host.trim_matches(['[', ']']).to_lowercase();
    url.scheme() == "http" && LOCAL_REDIRECT_HOSTS.contains(&hostname.as_str())
}

/// The fields a client is created with.
pub struct NewClient {
    pub client_id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}

/// McpOauthClient's validations, as full messages.
async fn validation_errors(
    conn: &mut PgConnection,
    c: &NewClient,
) -> Result<Vec<String>, sqlx::Error> {
    let mut errors = Vec::new();
    if crate::ruby::is_blank(&c.client_id) {
        errors.push("Client can't be blank".to_string());
    } else {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM mcp_oauth_clients WHERE client_id = $1)",
        )
        .bind(&c.client_id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            errors.push("Client has already been taken".to_string());
        }
    }
    if c.client_id.chars().count() > 1000 {
        errors.push("Client is too long (maximum is 1000 characters)".to_string());
    }
    if crate::ruby::is_blank(&c.name) {
        errors.push("Name can't be blank".to_string());
    }
    if c.name.chars().count() > 255 {
        errors.push("Name is too long (maximum is 255 characters)".to_string());
    }
    if c.redirect_uris.is_empty() {
        errors.push("Redirect uris can't be blank".to_string());
    } else if !c.redirect_uris.iter().all(|u| valid_redirect_uri(u)) {
        errors.push("Redirect uris is invalid".to_string());
    }
    Ok(errors)
}

/// `McpOauthClient.create!` of a pre-registered, approved client, logged.
/// Err holds the validation messages.
pub async fn create(
    conn: &mut PgConnection,
    actor: i32,
    c: &NewClient,
) -> Result<Result<Client, Vec<String>>, AppError> {
    let errors = validation_errors(&mut *conn, c).await?;
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_oauth_clients (client_id, name, registration_type, trust_state, \
         redirect_uris, created_at, updated_at) \
         VALUES ($1, $2, 'pre_registered', 'approved', $3, now(), now()) RETURNING id",
    )
    .bind(&c.client_id)
    .bind(&c.name)
    .bind(&c.redirect_uris)
    .fetch_one(&mut *conn)
    .await?;
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_client_created",
        &[("client_id", id.to_string())],
        None,
        None,
    )
    .await?;
    find(conn, id)
        .await?
        .map(Ok)
        .ok_or_else(|| std::io::Error::other("the client just created").into())
}

/// Admin::McpClientsController#block: the trust state, the active
/// authorizations revoked when blocked, logged.
pub async fn block(
    conn: &mut PgConnection,
    actor: i32,
    client: &Client,
    blocked: bool,
) -> Result<Client, AppError> {
    let trust = if blocked { "blocked" } else { "approved" };
    if trust != client.trust_state {
        sqlx::query(
            "UPDATE mcp_oauth_clients SET trust_state = $2, updated_at = now() WHERE id = $1",
        )
        .bind(client.id)
        .bind(trust)
        .execute(&mut *conn)
        .await?;
    }
    if blocked {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM mcp_oauth_authorizations WHERE mcp_oauth_client_id = $1 \
             AND status = 'active' AND revoked_at IS NULL ORDER BY id",
        )
        .bind(client.id)
        .fetch_all(&mut *conn)
        .await?;
        for id in ids {
            crate::mcp::revoke(&mut *conn, id, Some(actor), "client_blocked").await?;
        }
    }
    crate::staff_action_logs::log_custom(
        &mut *conn,
        actor,
        "mcp_client_trust_updated",
        &[
            ("client_id", client.id.to_string()),
            ("trust_state", trust.to_string()),
        ],
        None,
        None,
    )
    .await?;
    find(conn, client.id)
        .await?
        .ok_or_else(|| std::io::Error::other("the client just blocked").into())
}

/// Why ClientResolver.resolve! refused.
pub enum Refused {
    /// Discourse::InvalidAccess
    InvalidAccess,
    /// RateLimiter::LimitExceeded
    RateLimited(crate::rate_limiter::LimitExceeded),
}

/// `ClientResolver.valid_client_id_uri?`: https with a host and a path
/// other than `/`, no user info, fragment or dot segments.
fn valid_client_id(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    // url::Url resolves dot segments: read them from the raw path.
    let after_scheme = value.split_once("://")?.1;
    let raw_path = after_scheme
        .find('/')
        .map(|i| &after_scheme[i..])
        .unwrap_or("");
    let raw_path = raw_path.split(['?', '#']).next().unwrap_or("");
    let dot_segment = raw_path.split('/').any(|segment| {
        let s = segment.to_lowercase().replace("%2e", ".");
        s == "." || s == ".."
    });
    let ok = url.scheme() == "https"
        && url.host_str().is_some_and(|h| !h.is_empty())
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && !raw_path.is_empty()
        && raw_path != "/"
        && !dot_segment;
    ok.then_some(url)
}

fn setting(settings: &SiteSettings, name: &str) -> String {
    settings.get(name).map(|v| v.to_s()).unwrap_or_default()
}

/// `ClientResolver.approved_domain?`
fn approved_domain(settings: &SiteSettings, host: &str) -> bool {
    setting(settings, "mcp_oauth_client_id_metadata_domains")
        .split('|')
        .any(|d| d.eq_ignore_ascii_case(host))
}

/// `ClientResolver.supports_public_client_authentication?`
fn public_client(metadata: &serde_json::Map<String, Value>) -> bool {
    let preferred = match metadata.get("token_endpoint_auth_method") {
        None => Value::String("none".into()),
        Some(v) => v.clone(),
    };
    let Some(supported) = metadata.get("token_endpoint_auth_methods_supported") else {
        return preferred == "none";
    };
    let Some(list) = supported.as_array() else {
        return false;
    };
    list.iter().all(Value::is_string)
        && list.contains(&preferred)
        && list.iter().any(|m| m == "none")
}

/// `ClientResolver.validate_metadata!`
fn valid_metadata(metadata: &serde_json::Map<String, Value>, client_id: &str) -> bool {
    let Some(uris) = metadata.get("redirect_uris").and_then(Value::as_array) else {
        return false;
    };
    metadata.get("client_id").and_then(Value::as_str) == Some(client_id)
        && metadata
            .get("client_name")
            .and_then(Value::as_str)
            .is_some_and(|n| n.len() <= 255)
        && !uris.is_empty()
        && uris.len() <= 20
        && public_client(metadata)
        && !metadata.contains_key("client_secret")
        && !metadata.contains_key("client_secret_expires_at")
        && uris
            .iter()
            .all(|u| u.as_str().is_some_and(valid_redirect_uri))
}

/// `ClientResolver.canonicalize`: objects' keys sorted, at every depth.
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            for k in keys {
                out.insert(k.clone(), canonicalize(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        other => other.clone(),
    }
}

/// What the refresh needs besides the connection.
pub struct ResolveCx<'a> {
    pub pool: &'a sqlx::PgPool,
    pub rate_limits: bool,
    pub fetcher: Fetcher<'a>,
    pub user: crate::rate_limiter::LimitUser,
}

/// `ClientResolver.resolve!(client_id, force: true, user:)` for a metadata
/// client: the policy and URI checks, the rate limits, the document
/// fetched and validated, the client updated (a changed document sends
/// its active authorizations back to consent).
pub async fn refresh(
    conn: &mut PgConnection,
    cx: &ResolveCx<'_>,
    client: &Client,
) -> Result<Result<Client, Refused>, AppError> {
    let settings = cx.fetcher.settings;
    if client.trust_state == "blocked" {
        return Ok(Err(Refused::InvalidAccess));
    }
    let policy = setting(settings, "mcp_oauth_client_id_metadata_policy");
    if policy == "disabled" {
        return Ok(Err(Refused::InvalidAccess));
    }
    let Some(url) = valid_client_id(&client.client_id) else {
        return Ok(Err(Refused::InvalidAccess));
    };
    let host = url.host_str().unwrap_or_default().to_string();
    if policy == "approved_domains" && !approved_domain(settings, &host) {
        return Ok(Err(Refused::InvalidAccess));
    }
    // enforce_rate_limits!: per admin, then per host.
    let normalized = host.to_lowercase();
    let normalized = normalized.strip_suffix('.').unwrap_or(&normalized);
    let limiters = [
        crate::rate_limiter::RateLimiter::new(Some(cx.user), "mcp-cimd-metadata", 10, 60),
        crate::rate_limiter::RateLimiter::new(
            None,
            &format!(
                "mcp-cimd-metadata-host-{:x}",
                Sha256::digest(normalized.as_bytes())
            ),
            30,
            60,
        ),
    ];
    for limiter in limiters {
        if let Err(refused) = limiter
            .apply_limit_to_staff()
            .performed(cx.pool, cx.rate_limits)
            .await?
        {
            return Ok(Err(Refused::RateLimited(refused)));
        }
    }
    // fetch_metadata
    let opts = Options {
        timeout: std::time::Duration::from_secs(5),
        max_redirects: 3,
        https_redirect_ignore_limit: false,
        stop_at_blocked_pages: false,
        headers: Vec::new(),
    };
    let read = cx
        .fetcher
        .get(
            &mut *conn,
            &client.client_id,
            &opts,
            |_| MAX_METADATA_BYTES,
            |_| false,
        )
        .await?;
    let Read::Page {
        body, content_type, ..
    } = read
    else {
        return Ok(Err(Refused::InvalidAccess));
    };
    static JSON_TYPE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?i)\Aapplication/(?:[a-z0-9.+-]+\+)?json\b").expect("regex")
    });
    if body.len() > MAX_METADATA_BYTES
        || !content_type
            .as_deref()
            .is_some_and(|t| JSON_TYPE.is_match(t))
    {
        return Ok(Err(Refused::InvalidAccess));
    }
    let Ok(Value::Object(metadata)) = serde_json::from_slice::<Value>(&body) else {
        return Ok(Err(Refused::InvalidAccess));
    };
    if !valid_metadata(&metadata, &client.client_id) {
        return Ok(Err(Refused::InvalidAccess));
    }
    let hash = format!(
        "{:x}",
        Sha256::digest(
            canonicalize(&Value::Object(metadata.clone()))
                .to_string()
                .as_bytes()
        )
    );
    let trust = match policy.as_str() {
        "any_domain" => "approved",
        "approved_domains" if approved_domain(settings, &host) => "approved",
        _ => "pending",
    };
    let name = metadata
        .get("client_name")
        .and_then(Value::as_str)
        .filter(|n| !crate::ruby::is_blank(n))
        .map(str::to_string)
        .unwrap_or_else(|| host.clone());
    let uris: Vec<String> = metadata["redirect_uris"]
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(|u| u.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    sqlx::query(
        "UPDATE mcp_oauth_clients SET name = $2, registration_type = 'cimd', trust_state = $3, \
         metadata_uri = client_id, metadata_hash = $4, \
         metadata_expires_at = now() + interval '1 hour', metadata = $5, redirect_uris = $6, \
         last_seen_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(client.id)
    .bind(&name)
    .bind(trust)
    .bind(&hash)
    .bind(Value::Object(metadata))
    .bind(&uris)
    .execute(&mut *conn)
    .await?;
    if client
        .metadata_hash
        .as_deref()
        .is_some_and(|h| !h.is_empty() && h != hash)
    {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM mcp_oauth_authorizations WHERE mcp_oauth_client_id = $1 \
             AND status = 'active' AND revoked_at IS NULL ORDER BY id",
        )
        .bind(client.id)
        .fetch_all(&mut *conn)
        .await?;
        for id in ids {
            sqlx::query(
                "UPDATE mcp_oauth_authorizations SET status = 'consent_required', updated_at = now() \
                 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
            for table in ["mcp_oauth_access_tokens", "mcp_oauth_refresh_tokens"] {
                sqlx::query(&format!(
                    "UPDATE {table} SET revoked_at = now() \
                     WHERE mcp_oauth_authorization_id = $1 AND revoked_at IS NULL"
                ))
                .bind(id)
                .execute(&mut *conn)
                .await?;
            }
        }
    }
    if trust != "approved" {
        return Ok(Err(Refused::InvalidAccess));
    }
    Ok(Ok(find(conn, client.id).await?.ok_or_else(|| {
        std::io::Error::other("the client just refreshed")
    })?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uris_are_https_or_local_http() {
        assert!(valid_redirect_uri("https://example.com/cb"));
        assert!(valid_redirect_uri("http://localhost:3000/cb"));
        assert!(valid_redirect_uri("http://[::1]:3000/cb"));
        assert!(!valid_redirect_uri("http://example.com/cb"));
        assert!(!valid_redirect_uri("https://u:p@example.com/cb"));
        assert!(!valid_redirect_uri("https://example.com/cb#x"));
        assert!(!valid_redirect_uri("example.com/cb"));
    }

    #[test]
    fn client_ids_are_https_documents() {
        assert!(valid_client_id("https://example.com/client.json").is_some());
        assert!(valid_client_id("https://example.com/").is_none());
        assert!(valid_client_id("https://example.com").is_none());
        assert!(valid_client_id("http://example.com/client.json").is_none());
        assert!(valid_client_id("https://example.com/a/../client.json").is_none());
        assert!(valid_client_id("https://example.com/a/%2E%2E/c").is_none());
    }

    #[test]
    fn metadata_needs_a_public_client() {
        let m = |v: Value| v.as_object().unwrap().clone();
        let base = json!({
            "client_id": "https://e.com/c.json",
            "client_name": "E",
            "redirect_uris": ["https://e.com/cb"],
        });
        assert!(valid_metadata(&m(base.clone()), "https://e.com/c.json"));
        let mut secret = base.clone();
        secret["client_secret"] = json!("x");
        assert!(!valid_metadata(&m(secret), "https://e.com/c.json"));
        let mut methods = base.clone();
        methods["token_endpoint_auth_methods_supported"] = json!(["client_secret_basic"]);
        assert!(!valid_metadata(&m(methods), "https://e.com/c.json"));
    }
}
