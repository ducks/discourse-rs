//! EmbeddableHost and Embedding (the embedding settings) behind
//! Admin::EmbeddableHostsController and Admin::EmbeddingController.
//! The public /embed endpoints and `EmbeddableHost.url_allowed?` are not
//! here.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// `Embedding.posts_and_topics_settings`, then `crawlers_settings`.
pub const POSTS_AND_TOPICS: [&str; 5] = [
    "embed_by_username",
    "embed_post_limit",
    "embed_title_scrubber",
    "embed_truncate",
    "embed_unlisted",
];
pub const CRAWLERS: [&str; 3] = [
    "allowed_embed_selectors",
    "blocked_embed_selectors",
    "allowed_embed_classnames",
];

/// `Embedding.settings`
pub fn settings() -> impl Iterator<Item = &'static str> {
    POSTS_AND_TOPICS.iter().chain(CRAWLERS.iter()).copied()
}

/// UserHistory actions for the host logs.
pub const EMBEDDABLE_HOST_CREATE: i32 = 73;
pub const EMBEDDABLE_HOST_UPDATE: i32 = 74;
pub const EMBEDDABLE_HOST_DESTROY: i32 = 75;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Host {
    pub id: i32,
    pub host: String,
    pub allowed_paths: Option<String>,
    pub class_name: Option<String>,
    pub category_id: i32,
    pub user_id: Option<i32>,
}

const COLUMNS: &str = "id, host, allowed_paths, class_name, category_id, user_id";

pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Host>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM embeddable_hosts WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await
}

/// `EmbeddableHost.all.order(:host)`
pub async fn all(conn: &mut PgConnection) -> Result<Vec<Host>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM embeddable_hosts ORDER BY host"
    ))
    .fetch_all(conn)
    .await
}

/// EmbeddableHostSerializer
pub async fn serialize(conn: &mut PgConnection, h: &Host) -> Result<Value, sqlx::Error> {
    let tags: Vec<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.name, t.slug FROM embeddable_host_tags eht JOIN tags t ON t.id = eht.tag_id \
         WHERE eht.embeddable_host_id = $1 ORDER BY eht.id",
    )
    .bind(h.id)
    .fetch_all(&mut *conn)
    .await?;
    let user: Option<String> = match h.user_id {
        Some(id) => sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?,
        None => None,
    };
    Ok(json!({
        "id": h.id,
        "host": h.host,
        "allowed_paths": h.allowed_paths,
        "class_name": h.class_name,
        "category_id": h.category_id,
        "tags": tags.into_iter().map(|(id, name, slug)| json!({"id": id, "name": name, "slug": slug})).collect::<Vec<_>>(),
        "user": user,
    }))
}

/// The before_validation callback: the scheme off the front, the path
/// (from a `/` with no newline after it) off the end.
pub fn clean_host(host: &str) -> String {
    let host = host
        .strip_prefix("https://")
        .or_else(|| host.strip_prefix("http://"))
        .unwrap_or(host);
    for (i, c) in host.char_indices() {
        if c == '/' && !host[i + 1..].contains('\n') {
            return host[..i].to_string();
        }
    }
    host.to_string()
}

/// `host_must_be_valid`'s three patterns (a domain with a TLD, an IPv4
/// address, a `localhost`), case-insensitive; `\Z` may leave a newline.
pub fn valid_host(host: &str) -> bool {
    static PATTERNS: std::sync::OnceLock<Vec<regex::Regex>> = std::sync::OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"(?i)\A[a-z0-9]+([\-\.]+[a-z0-9]+)*\.[a-z]{2,24}(:[0-9]{1,5})?(/.*)?\n?\z",
            r"\A(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(:[0-9]{1,5})?(/.*)?\n?\z",
            r"(?i)\A([a-z0-9]+([\-\.][a-z0-9]+)*\.)?localhost(:[0-9]{1,5})?(/.*)?\n?\z",
        ]
        .iter()
        .map(|p| regex::Regex::new(p).expect("a host pattern"))
        .collect()
    });
    patterns.iter().any(|p| p.is_match(host))
}

/// `allowed_paths_must_be_valid`: Ruby's `Regexp.new` compiles it. A
/// pattern the port compiles is valid; unbalanced parentheses or brackets
/// are invalid in Ruby too; anything else is refused.
pub fn valid_allowed_paths(paths: &str) -> Result<bool, Unsupported> {
    if fancy_regex::Regex::new(paths).is_ok() {
        return Ok(true);
    }
    let mut depth: i64 = 0;
    let mut in_class = false;
    let mut chars = paths.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '[' if !in_class => in_class = true,
            ']' if in_class => in_class = false,
            '(' if !in_class => depth += 1,
            ')' if !in_class => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Ok(false);
        }
    }
    if depth != 0 || in_class {
        return Ok(false);
    }
    Err(Unsupported("allowed paths only Ruby's regexp syntax may accept"))
}

/// What `save_host` assigns.
pub struct HostParams {
    pub host: String,
    pub allowed_paths: Option<String>,
    pub category_id: Option<i32>,
    pub user_id: Option<i32>,
    /// `params[:embeddable_host][:tags]`: None when blank.
    pub tag_ids: Option<Vec<i32>>,
}

/// `save_host`: the host saved (create or update), its tags set and the
/// staff log written; Err(messages) when invalid.
pub async fn save(
    conn: &mut PgConnection,
    actor: i32,
    before: Option<&Host>,
    p: &HostParams,
) -> Result<Result<Host, Vec<String>>, AppError> {
    let host = clean_host(&p.host);
    let mut errors = Vec::new();
    if !valid_host(&host) {
        errors.push("Host is invalid".to_string());
    }
    if let Some(paths) = p.allowed_paths.as_deref().filter(|s| !crate::ruby::is_blank(s))
        && !valid_allowed_paths(paths)?
    {
        errors.push("Allowed paths is invalid".to_string());
    }
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    let category_id = p.category_id.unwrap_or(0);
    let id = match before {
        None => {
            let id: i32 = sqlx::query_scalar(
                "INSERT INTO embeddable_hosts (host, allowed_paths, category_id, user_id, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, now(), now()) RETURNING id",
            )
            .bind(&host)
            .bind(&p.allowed_paths)
            .bind(category_id)
            .bind(p.user_id)
            .fetch_one(&mut *conn)
            .await?;
            id
        }
        Some(b) => {
            let changed = b.host != host
                || b.allowed_paths != p.allowed_paths
                || b.category_id != category_id
                || b.user_id != p.user_id;
            if changed {
                sqlx::query(
                    "UPDATE embeddable_hosts SET host = $2, allowed_paths = $3, category_id = $4, \
                       user_id = $5, updated_at = now() WHERE id = $1",
                )
                .bind(b.id)
                .bind(&host)
                .bind(&p.allowed_paths)
                .bind(category_id)
                .bind(p.user_id)
                .execute(&mut *conn)
                .await?;
            }
            b.id
        }
    };
    // manage_tags: blank clears, else the join rows become the tags found.
    let wanted: Vec<i32> = match &p.tag_ids {
        None => Vec::new(),
        Some(ids) => sqlx::query_scalar("SELECT id FROM tags WHERE id = ANY($1) ORDER BY id")
            .bind(ids)
            .fetch_all(&mut *conn)
            .await?,
    };
    sqlx::query("DELETE FROM embeddable_host_tags WHERE embeddable_host_id = $1 AND NOT (tag_id = ANY($2))")
        .bind(id)
        .bind(&wanted)
        .execute(&mut *conn)
        .await?;
    for tag_id in &wanted {
        sqlx::query(
            "INSERT INTO embeddable_host_tags (embeddable_host_id, tag_id, created_at, updated_at) \
             VALUES ($1, $2, now(), now()) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(tag_id)
        .execute(&mut *conn)
        .await?;
    }
    // log_embeddable_host, with saved_changes' "key: value" lists on update.
    let (previous, new) = match before {
        None => (None, None),
        Some(b) => {
            let s = |v: &Option<String>| v.clone().unwrap_or_default();
            let i = |v: Option<i32>| v.map(|v| v.to_string()).unwrap_or_default();
            let mut old = Vec::new();
            let mut new = Vec::new();
            if b.allowed_paths != p.allowed_paths {
                old.push(format!("allowed_paths: {}", s(&b.allowed_paths)));
                new.push(format!("allowed_paths: {}", s(&p.allowed_paths)));
            }
            if b.category_id != category_id {
                old.push(format!("category_id: {}", b.category_id));
                new.push(format!("category_id: {category_id}"));
            }
            if b.host != host {
                old.push(format!("host: {}", b.host));
                new.push(format!("host: {host}"));
            }
            if b.user_id != p.user_id {
                old.push(format!("user_id: {}", i(b.user_id)));
                new.push(format!("user_id: {}", i(p.user_id)));
            }
            (Some(old.join(", ")), Some(new.join(", ")))
        }
    };
    log(
        conn,
        actor,
        if before.is_some() {
            EMBEDDABLE_HOST_UPDATE
        } else {
            EMBEDDABLE_HOST_CREATE
        },
        &host,
        previous.as_deref(),
        new.as_deref(),
    )
    .await?;
    let saved = find(&mut *conn, id)
        .await?
        .ok_or(Unsupported("an embeddable host gone once saved"))?;
    Ok(Ok(saved))
}

/// StaffActionLogger#log_embeddable_host
pub async fn log(
    conn: &mut PgConnection,
    actor: i32,
    action: i32,
    host: &str,
    previous: Option<&str>,
    new: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, context, previous_value, new_value, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action)
    .bind(actor)
    .bind(format!("host: {host}"))
    .bind(previous)
    .bind(new)
    .execute(conn)
    .await?;
    Ok(())
}

/// The embedding's settings as EmbeddingSerializer writes them.
pub fn settings_json(values: &[(&str, crate::site_settings::Value)]) -> Map<String, Value> {
    let mut out = Map::new();
    for (name, value) in values {
        out.insert(
            (*name).into(),
            match value {
                crate::site_settings::Value::Bool(b) => json!(b),
                crate::site_settings::Value::Int(i) => json!(i),
                other => json!(other.to_s()),
            },
        );
    }
    out
}
