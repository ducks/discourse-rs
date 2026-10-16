//! Write requests replayed against Rails' recording: each case is
//! parity/writes/<area>/<name>.case.json, recorded by scripts/record-writes
//! into <name>.json beside it, and is a test of its own (`<area>::<name>`).
//! It runs on a fresh copy of the seed, and its responses and every row it
//! inserted, updated or deleted must be what Discourse did.
//!
//! Timestamps the case itself wrote (at or after its start) compare as
//! `<now>`. Ids line up because both sides set every sequence to its
//! table's max(id) first. The jobs the requests enqueued are compared,
//! and those the case's `run_jobs` enqueue in turn.

mod common;

use std::collections::{BTreeMap, HashMap};

use axum::body::Body;
use axum::http::{Method, Request, header};
use chrono::{NaiveDateTime, Timelike};
use common::{TestDb, recorded_config, state};
use discourse_rs::AppState;
use http_body_util::BodyExt;
use serde_json::{Map, Value};
use sqlx::PgPool;
use tower::ServiceExt;

/// Not compared: written by the agent's scheduler during a recording
/// (scheduler_stats, top_topics), the login's token row, whose id and
/// random token cannot line up (the seed leaves tokens out; tests/sessions.rs
/// covers it), or the stylesheet the reference compiles the first time a
/// server-rendered page links it (stylesheet_cache).
const BACKGROUND_TABLES: [&str; 4] = [
    "scheduler_stats",
    "stylesheet_cache",
    "top_topics",
    "user_auth_tokens",
];

/// A background table is compared when the case names it in
/// `include_tables` (it runs the job that writes it).
fn background(table: &str, include: &[String]) -> bool {
    BACKGROUND_TABLES.contains(&table) && !include.iter().any(|t| t == table)
}

fn include_tables(case: &Value) -> Vec<String> {
    case["include_tables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_str().map(str::to_string))
        .collect()
}

/// Compared as a set, without ids: tables Rails fills from a query without
/// ORDER BY (sidebar links from `Category.where(id:).pluck(:id)`), whose
/// ids follow each database's heap order, and upload references, which
/// `link_post_uploads` deletes and inserts again within a case (on create,
/// then in the post processor), so their ids count rows the recording no
/// longer shows and the sequence cannot be lined up from it.
const UNORDERED_INSERTS: [&str; 2] = ["sidebar_section_links", "upload_references"];

/// Keys plugins add to the post and topic serializers on the reference.
const PLUGIN_KEYS: [&str; 4] = [
    "event",
    "calendar_details",
    "discourse_zendesk_plugin_zendesk_id",
    "discourse_zendesk_plugin_zendesk_url",
];

/// Category custom fields the reference's plugins preload
/// (Site.preloaded_category_custom_fields), which BasicCategorySerializer
/// then shows as `custom_fields`.
const PLUGIN_CATEGORY_FIELDS: [&str; 12] = [
    "additional_assign_allowed_on_groups",
    "create_as_post_voting_default",
    "disable_topic_resorting",
    "empty_box_on_unsolved",
    "enable_accepted_answers",
    "enable_shared_issues",
    "enable_topic_voting",
    "enable_unassigned_filter",
    "has_chat_enabled",
    "notify_on_staff_accept_solved",
    "only_post_voting_in_this_category",
    "sort_topics_by_event_start_date",
];

/// A category's `custom_fields` holding only plugin fields.
fn only_plugin_category_fields(value: &Value) -> bool {
    value.as_object().is_some_and(|fields| {
        !fields.is_empty()
            && fields
                .keys()
                .all(|k| PLUGIN_CATEGORY_FIELDS.contains(&k.as_str()))
    })
}

/// Keys plugins add to the user serializer and its user_option on the
/// reference (chat, discourse-solved, discourse-calendar).
const PLUGIN_USER_KEYS: [&str; 1] = ["event_reminder_preference"];

/// Keys the reference's unported plugins add to CurrentUserSerializer
/// (discourse-templates).
const PLUGIN_CURRENT_USER_KEYS: [&str; 1] = ["can_use_templates"];

/// User serializer keys that follow what the reference does in the
/// background between seeding and recording (profile views, other
/// sessions, badge grants): not compared.
const DRIFTING_USER_KEYS: [&str; 3] = [
    "profile_view_count",
    "user_auth_tokens",
    "featured_user_badge_ids",
];

/// A serialized user (or current user) in a response, less plugin and
/// drifting keys, with `sidebar_category_ids` (plucked without ORDER BY)
/// sorted.
fn undrift_user(user: &mut Value) {
    let Some(map) = user.as_object_mut() else {
        return;
    };
    map.retain(|k, _| {
        !PLUGIN_USER_KEYS.contains(&k.as_str())
            && !PLUGIN_CURRENT_USER_KEYS.contains(&k.as_str())
            && !DRIFTING_USER_KEYS.contains(&k.as_str())
    });
    if let Some(Value::Object(options)) = map.get_mut("user_option") {
        options.retain(|k, _| !PLUGIN_USER_KEYS.contains(&k.as_str()));
    }
    if let Some(Value::Array(ids)) = map.get_mut("sidebar_category_ids") {
        ids.sort_by_key(|v| v.as_i64());
    }
}

/// Jobs plugins enqueue on the reference (discourse-narrative-bot).
const PLUGIN_JOBS: [&str; 1] = ["bot_input"];

/// Jobs queued at most once a minute through a Redis lock the recorder's
/// cases share, so whether a recording shows one depends on the case
/// recorded before it: a staff current user's refresh_latest_new_feature
/// (DiscourseUpdates.has_unseen_features?).
const THROTTLED_JOBS: [&str; 1] = ["refresh_latest_new_feature"];

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
    /// The last response's rate limit headers, as the recorder keeps them.
    rate_headers: Map<String, Value>,
    /// The headers the case names in `keep_headers`, kept as well.
    keep_headers: Vec<String>,
}

/// A Location as the recorder keeps it: its path and query, the host being
/// each side's own.
fn location_path(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".to_string(),
        },
        None => url.to_string(),
    }
}

/// The response headers recordings keep (when a response has any).
const RATE_HEADERS: [&str; 4] = [
    "Retry-After",
    "Discourse-Rate-Limit-Error-Code",
    "Discourse-Actions-Remaining",
    "Discourse-Actions-Max",
];

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
        self.send_with(method, path, body, &Map::new()).await
    }

    /// A request with the case's own headers. One with an API key goes
    /// without the CSRF token, as an API client would.
    async fn send_with(
        &mut self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        extra: &Map<String, Value>,
    ) -> (u16, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "localhost")
            .header("x-requested-with", "XMLHttpRequest")
            .header(header::ACCEPT, "application/json");
        if !self.cookies.is_empty() {
            let cookie: Vec<String> = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            request = request.header(header::COOKIE, cookie.join("; "));
        }
        let api = extra
            .keys()
            .any(|k| k.eq_ignore_ascii_case("api-key") || k.eq_ignore_ascii_case("user-api-key"));
        // The case's headers replace the defaults, as the recorder's
        // headers.merge does.
        if let Some(headers) = request.headers_mut() {
            for (name, value) in extra {
                let name = header::HeaderName::from_bytes(name.as_bytes()).unwrap();
                headers.insert(name, value.as_str().unwrap_or("").parse().unwrap());
            }
        }
        if let Some(token) = self.csrf.as_ref().filter(|_| !api) {
            request = request.header("x-csrf-token", token.as_str());
        }
        let body = match body {
            Some(Value::Object(params)) if params.values().any(is_fixture) => {
                let (content_type, bytes) = multipart(params);
                request = request.header(header::CONTENT_TYPE, content_type);
                Body::from(bytes)
            }
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = discourse_rs::app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        self.rate_headers = RATE_HEADERS
            .iter()
            .copied()
            .chain(self.keep_headers.iter().map(String::as_str))
            .filter_map(|name| {
                let value = response.headers().get(name)?.to_str().ok()?;
                let value = if name == "Location" {
                    location_path(value)
                } else {
                    value.to_string()
                };
                Some((name.to_string(), Value::String(value)))
            })
            .collect();
        for value in response.headers().get_all(header::SET_COOKIE) {
            let value = value.to_str().unwrap_or("");
            let (pair, attrs) = value.split_once(';').unwrap_or((value, ""));
            let Some((name, v)) = pair.split_once('=') else {
                continue;
            };
            self.cookies.retain(|(n, _)| n != name);
            if !attrs.contains("max-age=0") && !v.is_empty() {
                self.cookies.push((name.to_string(), v.to_string()));
            }
        }
        let zipped = response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes().starts_with(b"application/zip"));
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        if zipped {
            return (status, zip_entries(&bytes));
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn login(&mut self, username: &str) {
        // The case's own database: its user's password is stored with one
        // PBKDF2 iteration instead of 600k, so logging in costs nothing.
        // The login's writes come before the case's snapshot either way.
        let algorithm = "$pbkdf2-sha256$i=1,l=32$";
        let salt: String = sqlx::query_scalar(
            "SELECT p.password_salt FROM user_passwords p JOIN users u ON u.id = p.user_id \
             WHERE u.username_lower = lower($1)",
        )
        .bind(username)
        .fetch_one(&self.state.pool)
        .await
        .unwrap_or_else(|e| panic!("{username} has no password: {e}"));
        let hash =
            discourse_rs::session::token::hash_password("password", &salt, algorithm).unwrap();
        sqlx::query(
            "UPDATE user_passwords SET password_hash = $2, password_algorithm = $3 \
             WHERE user_id = (SELECT id FROM users WHERE username_lower = lower($1))",
        )
        .bind(username)
        .bind(hash)
        .bind(algorithm)
        .execute(&self.state.pool)
        .await
        .unwrap();
        let (_, csrf) = self.send(Method::GET, "/session/csrf.json", None).await;
        self.csrf = csrf["csrf"].as_str().map(str::to_string);
        let body = serde_json::json!({ "login": username, "password": "password" });
        let (status, reply) = self.send(Method::POST, "/session.json", Some(&body)).await;
        assert_eq!(status, 200, "login as {username}: {reply}");
    }
}

/// A zip response as the recorder keeps it: `{"zip": [{name, size,
/// sha256, text (a CSV's)}]}` by name.
fn zip_entries(bytes: &[u8]) -> Value {
    use sha2::Digest;
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        let mut item = serde_json::json!({
            "name": name,
            "size": data.len(),
            "sha256": format!("{:x}", sha2::Sha256::digest(&data)),
        });
        if name.ends_with(".csv") {
            item["text"] = Value::String(String::from_utf8_lossy(&data).into_owned());
        }
        entries.push(item);
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    serde_json::json!({ "zip": entries })
}

/// A `{"fixture": name}` param: a file from parity/writes/files, sent as
/// the recorder sent it, a Rack::Test::UploadedFile in a multipart form.
fn is_fixture(value: &Value) -> bool {
    value.get("fixture").is_some_and(Value::is_string)
}

/// `Rack::Mime.mime_type(File.extname(path), "application/octet-stream")`,
/// the type the recorder's Rack::Test::UploadedFile sends, for the
/// extensions parity/writes/files holds.
fn rack_mime_type(file: &str) -> &'static str {
    let ext = file.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "gz" => "application/x-gzip",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "pem" => "application/x-x509-ca-cert",
        _ => "application/octet-stream",
    }
}

fn multipart(params: &Map<String, Value>) -> (String, Vec<u8>) {
    let boundary = "----discourse-rs-writes-boundary";
    let files = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/writes/files");
    let mut out = Vec::new();
    for (name, value) in params {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        if is_fixture(value) {
            let file = value["fixture"].as_str().unwrap();
            let content_type = rack_mime_type(file);
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                     Content-Type: {content_type}\r\n\r\n"
                )
                .as_bytes(),
            );
            out.extend(std::fs::read(files.join(file)).unwrap());
        } else {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{text}").as_bytes(),
            );
        }
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), out)
}

/// Every table with its primary key columns.
async fn tables(pool: &PgPool, include: &[String]) -> Vec<(String, Vec<String>)> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.relname, \
                COALESCE((SELECT string_agg(a.attname, ',' ORDER BY array_position(i.indkey, a.attnum)) \
                          FROM pg_index i JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
                          WHERE i.indrelid = c.oid AND i.indisprimary), '') \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relkind = 'r' ORDER BY c.relname",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    rows.into_iter()
        .filter(|(t, _)| !background(t, include))
        .map(|(t, pk)| {
            let pk = pk
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            (t, pk)
        })
        .collect()
}

async fn table_rows(pool: &PgPool, table: &str) -> Vec<Value> {
    let rows: Vec<String> =
        sqlx::query_scalar(&format!("SELECT to_jsonb(x)::text FROM \"{table}\" x"))
            .fetch_all(pool)
            .await
            .unwrap();
    rows.iter()
        .map(|r| serde_json::from_str(r).unwrap())
        .collect()
}

async fn checksum(pool: &PgPool, table: &str) -> String {
    sqlx::query_scalar(&format!(
        "SELECT md5(COALESCE(string_agg(x::text, '|' ORDER BY x::text), '')) FROM \"{table}\" x"
    ))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn reset_sequences(pool: &PgPool) {
    let sequences: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT s.relname, t.relname, a.attname FROM pg_class s \
         JOIN pg_depend d ON d.objid = s.oid AND d.deptype = 'a' \
         JOIN pg_class t ON t.oid = d.refobjid \
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid \
         JOIN pg_namespace n ON n.oid = t.relnamespace \
         WHERE s.relkind = 'S' AND n.nspname = 'public'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for (seq, table, column) in sequences {
        let max: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT max(\"{column}\")::bigint FROM \"{table}\""
        ))
        .fetch_one(pool)
        .await
        .unwrap();
        let sql = match max.filter(|m| *m > 0) {
            Some(max) => format!("SELECT setval('\"{seq}\"', {max}, true)"),
            None => format!("SELECT setval('\"{seq}\"', 1, false)"),
        };
        sqlx::query(&sql).execute(pool).await.unwrap();
    }
}

/// A value that reads as a timestamp, in to_jsonb's or JSON's format, or
/// as Ruby's Time#to_s writes it.
fn timestamp(s: &str) -> Option<NaiveDateTime> {
    if let Some(t) = s.strip_suffix(" UTC") {
        return NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").ok();
    }
    // Z, or DateTime#iso8601's +00:00.
    let s = s
        .strip_suffix('Z')
        .or_else(|| s.strip_suffix("+00:00"))
        .unwrap_or(s);
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok()
}

/// What differs on every send: unsubscribe keys (64 hex) and MIME
/// boundaries.
fn unrandom(s: &str) -> String {
    static KEY: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\b[0-9a-f]{64}\b").unwrap());
    static BOUNDARY: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"boundary=\S+").unwrap());
    static HEX32: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^[0-9a-f]{32}$").unwrap());
    static UUID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap()
    });
    // An entrypoint script: Rails' asset digest and per-request CSP nonce
    // (the port sets no CSP).
    static ASSET: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(/assets/js/[a-z-]+?)(-[0-9a-f]{8})?\.js"#).unwrap()
    });
    static NONCE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#" nonce="[^"]*""#).unwrap());
    // UserAnonymizer's `anon<random number>` username, wherever it lands.
    static ANON: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\banon[0-9]{1,8}\b").unwrap());
    if HEX32.is_match(s) {
        return "<hex>".into();
    }
    let s = if s.contains("data-discourse-entrypoint=") {
        let s = ASSET.replace_all(s, "$1.js");
        NONCE.replace_all(&s, "").into_owned()
    } else {
        s.to_string()
    };
    // The reference's own port in its absolute URLs (each agent's
    // reference listens on its own; the replay's config has rs-parity's).
    static PORT: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"//localhost:\d{4}\b").unwrap());
    let s = PORT.replace_all(&s, "//localhost:<port>");
    // LetterAvatar.version: Rails' hashes its ImageMagick and font list,
    // the port's its renderer and font (src/letter_avatar.rs).
    static LETTER_AVATAR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(/letter_avatar/[^/]+/[^/]+/)5_[0-9a-f]{32}\.png").unwrap()
    });
    let s = LETTER_AVATAR.replace_all(&s, "${1}<version>.png");
    // EmailController's cache key for the unsubscribed page.
    static UNSUB: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\bunsub_[0-9a-f]{32}\b").unwrap());
    let s = UNSUB.replace_all(&s, "unsub_<hex>");
    // AdminConfirmation's token (SecureRandom.hex) in its links.
    static CONFIRM_ADMIN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"confirm-admin/[0-9a-f]{32}\b").unwrap());
    let s = CONFIRM_ADMIN.replace_all(&s, "confirm-admin/<hex>");
    // EmailBackupToken (SecureRandom.hex) in a backup download link.
    static BACKUP_TOKEN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\?token=[0-9a-f]{32}\b").unwrap());
    let s = BACKUP_TOKEN.replace_all(&s, "?token=<hex>");
    // A password reset token (SecureRandom.hex) in a mailed link whose
    // job doesn't carry it (InviteMailer's password instructions).
    static RESET_TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"/u/password-reset/[0-9a-f]{32}\b").unwrap()
    });
    let s = RESET_TOKEN.replace_all(&s, "/u/password-reset/<hex>");
    // Jobs::BulkInvite's log lines, stamped `[Time.now]` to the second.
    static LOG_TIME: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} \+0000\]").unwrap()
    });
    let s = LOG_TIME.replace_all(&s, "[<log time>]");
    let s = ANON.replace_all(&s, "anon<n>");
    let s = KEY.replace_all(&s, "<key>");
    let s = UUID.replace_all(&s, "<uuid>");
    BOUNDARY.replace_all(&s, "boundary=<boundary>").into_owned()
}

/// The columns a case names in `approximate` (`table.column`), floats
/// computed through powers and logs, rounded to seven significant digits on
/// both sides: their last digits follow rounding in the arithmetic, the
/// connection's float output (extra_float_digits) and Ruby's sub-microsecond
/// clock against the stored timestamp.
/// `unordered_responses`: JSON pointers to arrays in the recording that
/// Rails returns in heap order (a query without ORDER BY), compared
/// sorted by `id` (or `post_id`) since neither database's heap order can be
/// lined up (rows without either, such as a report's, by their whole JSON).
fn unordered_responses(doc: &mut Value, case: &Value) {
    for pointer in case["unordered_responses"].as_array().into_iter().flatten() {
        let Some(pointer) = pointer.as_str() else {
            continue;
        };
        if let Some(Value::Array(items)) = doc.pointer_mut(pointer) {
            items.sort_by_cached_key(|item| {
                (
                    item["id"].as_i64().or(item["post_id"].as_i64()),
                    item.to_string(),
                )
            });
        }
    }
}

/// `random_delays` (`{job: seconds}`): jobs enqueued with a random part
/// in their delay (schedule_backup's `rand(10.minutes)`), their delay
/// compared rounded down to that window on both sides.
fn random_delays(doc: &mut Value, case: &Value) {
    let Some(windows) = case["random_delays"].as_object() else {
        return;
    };
    for list in ["jobs", "jobs_from_jobs"] {
        for job in doc[list].as_array_mut().into_iter().flatten() {
            let Some(shown) = job[0].as_str() else {
                continue;
            };
            let Some((name, delay)) = shown.split_once(" in ") else {
                continue;
            };
            let (Some(window), Some(seconds)) = (
                windows.get(name).and_then(Value::as_i64),
                delay.strip_suffix('s').and_then(|d| d.parse::<i64>().ok()),
            ) else {
                continue;
            };
            job[0] = Value::String(format!("{name} in {}s", seconds / window * window));
        }
    }
}

/// The case's `ignore` paths again once web hook payloads are expanded
/// (`web_hook_values`), for the paths inside them.
fn ignore_in_payloads(doc: &mut Value, ignore: &[String]) {
    for ptr in ignore.iter().filter(|p| p.contains("/payload/")) {
        discourse_rs::parity::ignore_path(doc, ptr);
    }
}

/// The columns a case names in `digests` (`table.column`): SHA1 digests
/// no two runs share (a theme scss field's value_baked digests CSS whose
/// source map names the compile's temporary directory), compared as
/// `<digest>` wherever they hold one.
fn digests(doc: &mut Value, case: &Value) {
    let mask = |v: &mut Value| {
        if v.as_str()
            .is_some_and(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            *v = Value::String("<digest>".into());
        }
    };
    for spec in case["digests"].as_array().into_iter().flatten() {
        let Some((table, column)) = spec.as_str().and_then(|s| s.split_once('.')) else {
            continue;
        };
        let change = &mut doc["changes"][table];
        for kind in ["inserted", "deleted"] {
            for row in change[kind].as_array_mut().into_iter().flatten() {
                mask(&mut row[column]);
            }
        }
        for row in change["updated"].as_array_mut().into_iter().flatten() {
            for side in ["before", "after"] {
                mask(&mut row[side][column]);
            }
        }
    }
}

fn approximate(doc: &mut Value, case: &Value) {
    let round = |v: &mut Value| {
        if let Some(f) = v.as_f64().filter(|f| *f != 0.0 && f.is_finite()) {
            let digits = 6 - f.abs().log10().floor() as i32;
            let scale = 10f64.powi(digits);
            *v = serde_json::json!((f * scale).round() / scale);
        }
    };
    for spec in case["approximate"].as_array().into_iter().flatten() {
        let Some((table, column)) = spec.as_str().and_then(|s| s.split_once('.')) else {
            continue;
        };
        let change = &mut doc["changes"][table];
        for kind in ["inserted", "deleted"] {
            for row in change[kind].as_array_mut().into_iter().flatten() {
                round(&mut row[column]);
            }
        }
        for row in change["updated"].as_array_mut().into_iter().flatten() {
            for side in ["before", "after"] {
                round(&mut row[side][column]);
            }
        }
    }
}

/// The tables a case names in `unordered` (`{table: [columns]}`): rows
/// Rails inserts in an order it leaves to the database (an unordered
/// lookup), compared without those columns (the ids that order gives).
fn unordered(doc: &mut Value, case: &Value) {
    let Some(tables) = case["unordered"].as_object() else {
        return;
    };
    for (table, columns) in tables {
        let columns: Vec<&str> = columns
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if let Some(Value::Array(rows)) = doc["changes"][table].get_mut("inserted") {
            for row in rows.iter_mut() {
                if let Value::Object(r) = row {
                    for column in &columns {
                        r.remove(*column);
                    }
                }
            }
        }
    }
}

/// The user API key authorization's encrypted answers (a `payload` and a
/// `oneTimePassword`, random RSA padding on every run), decrypted with the
/// cases' fixed private key (parity/writes/files/user_api_key_private.pem):
/// a `payload` body field becomes its JSON, and a redirect carrying them
/// (`redirect_url`, a kept Location) becomes `{url, payload,
/// oneTimePassword}` with the rest of its query left in `url`.
fn decrypt_user_api_payloads(doc: &mut Value) {
    use base64::Engine;
    use rsa::pkcs1::DecodeRsaPrivateKey;
    static KEY: std::sync::LazyLock<Option<rsa::RsaPrivateKey>> = std::sync::LazyLock::new(|| {
        let pem = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("parity/writes/files/user_api_key_private.pem"),
        )
        .ok()?;
        rsa::RsaPrivateKey::from_pkcs1_pem(&pem).ok()
    });
    let Some(key) = KEY.as_ref() else {
        return;
    };
    let decrypt = |b64: &str| -> Option<String> {
        let compact: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(compact)
            .ok()?;
        let plain = key
            .decrypt(rsa::Pkcs1v15Encrypt, &bytes)
            .or_else(|_| key.decrypt(rsa::Oaep::new::<sha1::Sha1>(), &bytes))
            .ok()?;
        String::from_utf8(plain).ok()
    };
    let as_json = |plain: String| serde_json::from_str(&plain).unwrap_or(Value::String(plain));
    let redirect = |url: &str| -> Option<Value> {
        let (base, query) = url.split_once('?')?;
        let mut rest = Vec::new();
        let mut out = Map::new();
        for (k, v) in form_urlencoded::parse(query.as_bytes()) {
            match k.as_ref() {
                "payload" | "oneTimePassword" => {
                    let plain = decrypt(&v)?;
                    out.insert(k.into_owned(), as_json(plain));
                }
                _ => rest.push(format!("{k}={v}")),
            }
        }
        if out.is_empty() {
            return None;
        }
        let url = if rest.is_empty() {
            base.to_string()
        } else {
            format!("{base}?{}", rest.join("&"))
        };
        out.insert("url".into(), Value::String(url));
        Some(Value::Object(out))
    };
    for response in doc["responses"].as_array_mut().into_iter().flatten() {
        let body = &mut response["body"];
        if let Some(plain) = body["payload"].as_str().and_then(decrypt) {
            body["payload"] = as_json(plain);
        }
        if let Some(v) = body["redirect_url"].as_str().and_then(redirect) {
            body["redirect_url"] = v;
        }
        if let Some(v) = response["headers"]["Location"].as_str().and_then(redirect) {
            response["headers"]["Location"] = v;
        }
    }
}

/// Per-run secrets a document carries in its own jobs and responses (email
/// tokens, login codes, honeypots), replaced wherever they appear in it.
fn hide_secrets(doc: &mut Value) {
    let mut secrets: Vec<(String, &str)> = Vec::new();
    for list in ["jobs", "jobs_from_jobs"] {
        for job in doc[list].as_array().cloned().unwrap_or_default() {
            for (key, label) in [("email_token", "<email_token>"), ("code", "<code>")] {
                if let Some(v) = job[1][key].as_str() {
                    secrets.push((v.to_string(), label));
                }
            }
        }
    }
    for response in doc["responses"].as_array().cloned().unwrap_or_default() {
        let body = &response["body"];
        if let Some(v) = body["value"].as_str() {
            secrets.push((v.to_string(), "<honeypot>"));
        }
        if let Some(v) = body["challenge"].as_str() {
            secrets.push((v.to_string(), "<challenge>"));
        }
        // The user API key device flow's random codes.
        if let Some(v) = body["user_code"].as_str() {
            secrets.push((v.to_string(), "<user_code>"));
        }
        if let Some(token) = body["verification_uri_with_request"]
            .as_str()
            .and_then(|u| u.rsplit_once("request="))
            .map(|(_, t)| t.to_string())
        {
            secrets.push((token, "<request_token>"));
        }
        if let Some(token) = body["redirect_url"]
            .as_str()
            .and_then(|u| u.rsplit('/').next())
        {
            secrets.push((token.to_string(), "<email_token>"));
        }
    }
    // Reply keys (dashed in the row, bare in Reply-To) and VERP bounce keys.
    let inserted = |table: &str| {
        doc["changes"][table]["inserted"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    for row in inserted("post_reply_keys") {
        if let Some(v) = row["reply_key"].as_str() {
            secrets.push((v.to_string(), "<reply_key>"));
            secrets.push((v.replace('-', ""), "<reply_key>"));
        }
    }
    // Invite keys (SecureRandom.base58) and email tokens, in the row and
    // in the links and emails that carry them.
    for row in inserted("invites") {
        if let Some(v) = row["invite_key"].as_str() {
            secrets.push((v.to_string(), "<invite_key>"));
        }
        if let Some(v) = row["email_token"].as_str() {
            secrets.push((v.to_string(), "<invite_email_token>"));
        }
    }
    for change in doc["changes"]["invites"]["updated"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if let Some(v) = change["after"]["email_token"].as_str() {
            secrets.push((v.to_string(), "<invite_email_token>"));
        }
    }
    for row in inserted("email_logs") {
        if let Some(v) = row["bounce_key"].as_str() {
            secrets.push((v.to_string(), "<bounce_key>"));
            secrets.push((v.replace('-', ""), "<bounce_key>"));
        }
    }
    // A new admin API key's first four characters (truncated_key): the
    // whole value where it stands alone (the row, a staff log's subject)
    // and inside the log's list of changes.
    for response in doc["responses"].as_array().cloned().unwrap_or_default() {
        let key = &response["body"]["key"];
        if let (Some(full), Some(truncated)) = (key["key"].as_str(), key["truncated_key"].as_str())
            && full.len() == 64
            && full.starts_with(truncated)
        {
            secrets.push((
                format!("truncated_key: {truncated}"),
                "truncated_key: <truncated_key>",
            ));
            let truncated = truncated.to_string();
            fn exact(v: &mut Value, truncated: &str) {
                match v {
                    Value::String(s) if s == truncated => *s = "<truncated_key>".into(),
                    Value::Array(a) => a.iter_mut().for_each(|x| exact(x, truncated)),
                    Value::Object(m) => m.values_mut().for_each(|x| exact(x, truncated)),
                    _ => {}
                }
            }
            exact(doc, &truncated);
        }
    }
    secrets.retain(|(v, _)| v.len() >= 6);
    fn walk(v: &mut Value, secrets: &[(String, &str)]) {
        match v {
            Value::String(s) => {
                for (secret, label) in secrets {
                    if s.contains(secret.as_str()) {
                        *s = s.replace(secret.as_str(), label);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, secrets)),
            Value::Object(m) => m.values_mut().for_each(|x| walk(x, secrets)),
            _ => {}
        }
    }
    walk(doc, &secrets);
}

/// Web hook deliveries: the JSON a payload carries in a string (the
/// `emit_web_hook_event` job's `payload`, `web_hook_events.payload`) is
/// compared as the JSON it holds, so the times in it normalize, and a
/// delivery's duration (milliseconds of a real request) is left out. The
/// signature header, an HMAC over a body with those times, is masked as
/// every 64-hex value is (`unrandom`).
fn web_hook_values(doc: &mut Value) {
    fn expand(v: &mut Value) {
        if let Some(parsed) = v
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .filter(|p| p.is_object() || p.is_array())
        {
            *v = parsed;
        }
        // An anonymized user's letter avatar: its color hashes the random
        // `anon<n>` username.
        if v["username"].as_str().is_some_and(|u| {
            u.strip_prefix("anon")
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        }) {
            static COLOR: std::sync::LazyLock<regex::Regex> =
                std::sync::LazyLock::new(|| regex::Regex::new(r"/letter/a/[0-9a-f]{6}/").unwrap());
            for key in ["avatar_template", "system_avatar_template"] {
                if let Some(Value::String(s)) = v.get_mut(key) {
                    *s = COLOR.replace(s, "/letter/a/<color>/").into_owned();
                }
            }
        }
    }
    for list in ["jobs", "jobs_from_jobs"] {
        for job in doc
            .get_mut(list)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if job[0]
                .as_str()
                .is_some_and(|n| n.starts_with("emit_web_hook_event"))
                && let Some(payload) = job[1].get_mut("payload")
            {
                expand(payload);
            }
        }
    }
    // The events the case delivered (or delivered again).
    let mut delivered: Vec<i64> = Vec::new();
    if let Some(Value::Object(kinds)) = doc
        .get_mut("changes")
        .and_then(|c| c.get_mut("web_hook_events"))
    {
        for kind in ["inserted", "updated"] {
            for row in kinds
                .get(kind)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(id) = row["id"].as_i64().or_else(|| row["after"]["id"].as_i64()) {
                    delivered.push(id);
                }
            }
        }
        let fix = |row: &mut Value| {
            if let Some(payload) = row.get_mut("payload") {
                expand(payload);
            }
            if let Some(duration) = row.get_mut("duration") {
                *duration = Value::String("<duration>".into());
            }
        };
        for (kind, list) in kinds.iter_mut() {
            for row in list.as_array_mut().into_iter().flatten() {
                match (kind.as_str(), row) {
                    ("updated", Value::Object(sides)) => sides.values_mut().for_each(fix),
                    (_, row) => fix(row),
                }
            }
        }
    }
    // AdminWebHookEventSerializer in a response, for a delivered event: its
    // duration too.
    fn walk(v: &mut Value, delivered: &[i64]) {
        match v {
            Value::Object(m) => {
                if m.contains_key("request_url")
                    && m.contains_key("duration")
                    && m.get("id")
                        .and_then(Value::as_i64)
                        .is_some_and(|id| delivered.contains(&id))
                {
                    m.insert("duration".into(), Value::String("<duration>".into()));
                }
                m.values_mut().for_each(|x| walk(x, delivered));
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, delivered)),
            _ => {}
        }
    }
    for response in doc["responses"].as_array_mut().into_iter().flatten() {
        walk(&mut response["body"], &delivered);
    }
}

/// DateTime#to_s times inside a longer text (a staff log's details), in
/// whole seconds, and JSON's millisecond times (a theme log's serialized
/// theme), taken during the case: `<now>`.
fn embedded_now(s: &str, started: NaiveDateTime) -> String {
    // DateTime#to_s, or TimeWithZone#to_s (a staff log's list of changes).
    static ISO: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00|\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC|\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z",
        )
        .unwrap()
    });
    if !ISO.is_match(s) || timestamp(s).is_some() {
        return s.to_string();
    }
    let floor = started.with_nanosecond(0).unwrap_or(started);
    ISO.replace_all(s, |c: &regex::Captures| match timestamp(&c[0]) {
        Some(t) if t >= floor => "<now>".to_string(),
        _ => c[0].to_string(),
    })
    .into_owned()
}

/// Timestamps at or after the case's start become `<now>`, plugin keys
/// go.
fn normalize(value: &Value, started: NaiveDateTime) -> Value {
    match value {
        Value::String(s) => match timestamp(s) {
            Some(t) if t >= started => Value::String("<now>".into()),
            // In whole seconds (DateTime#iso8601), a time taken at the start
            // reads as up to a second before it.
            Some(t) if !s.contains('.') && t >= started.with_nanosecond(0).unwrap_or(started) => {
                Value::String("<now>".into())
            }
            // A setup's `now() - interval ...`: how long before the case
            // began, to the minute, as setup takes its own time on each side.
            Some(t) if started - t < chrono::Duration::days(1) => {
                let minutes = ((started - t).num_seconds() as f64 / 60.0).round();
                Value::String(format!("<now-{minutes}m>"))
            }
            _ => Value::String(today(&embedded_now(&unrandom(s), started), started.date())),
        },
        Value::Array(items) => Value::Array(items.iter().map(|v| normalize(v, started)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !PLUGIN_KEYS.contains(&k.as_str()))
                .filter(|(k, v)| !(k.as_str() == "custom_fields" && only_plugin_category_fields(v)))
                // The reference's poll and post-voting plugins register
                // NewPostManager handlers, which turn queued posts on for
                // every topic view; empty, they say nothing.
                .filter(|(k, v)| {
                    !matches!(
                        (k.as_str(), v),
                        ("pending_posts", Value::Array(a)) if a.is_empty()
                    ) && !(k.as_str() == "queued_posts_count" && v.as_i64() == Some(0))
                })
                .map(|(k, v)| match (k.as_str(), v) {
                    // A session token: random on every run.
                    ("auth_token", Value::String(_)) => {
                        (k.clone(), Value::String("<auth_token>".into()))
                    }
                    // A presence state's place in the bus: Rails' per-channel
                    // MessageBus id (null before any message), the port's
                    // pg-bus position. Neither means anything to the other.
                    ("last_message_id", Value::Null | Value::Number(_) | Value::String(_))
                        if map.contains_key("count") =>
                    {
                        (k.clone(), Value::String("<bus position>".into()))
                    }
                    // AdminUserListSerializer's ages: Time.now less a time,
                    // so they move with the request's timing.
                    ("last_seen_age" | "last_emailed_age" | "created_at_age", Value::Number(_)) => {
                        (k.clone(), Value::String("<age>".into()))
                    }
                    // post_revisions.modifications: Rails reads any YAML,
                    // so the port's is compared as what it holds.
                    ("modifications", Value::String(yaml)) => {
                        (k.clone(), modifications_value(yaml))
                    }
                    _ => (k.clone(), normalize(v, started)),
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// The day the case ran, as a date column holds it (given_daily_likes) or
/// as an email shows it ("October 2"), becomes `<today>`: a recording
/// replayed on a later day is still the same case.
fn today(s: &str, day: chrono::NaiveDate) -> String {
    let iso = day.format("%Y-%m-%d").to_string();
    if s == iso {
        return "<today>".into();
    }
    let shown = day.format("%B %-d").to_string();
    if !s.contains(&shown) {
        return s.to_string();
    }
    regex::Regex::new(&format!(r"\b{}\b", regex::escape(&shown)))
        .unwrap()
        .replace_all(s, "<today>")
        .into_owned()
}

fn modifications_value(yaml: &str) -> Value {
    match discourse_rs::modifications::load(yaml) {
        Ok(fields) => Value::Array(
            fields
                .into_iter()
                .map(|(k, [a, b])| serde_json::json!([k, a, b]))
                .collect(),
        ),
        Err(e) => Value::String(format!("unreadable modifications ({e}): {yaml}")),
    }
}

/// Background tables go, and an updated row becomes the columns that
/// changed: the seed and the reference drift apart on columns no case
/// touches (category stats, unread markers), which is not what is measured.
fn changed_columns(mut doc: Value, include: &[String]) -> Value {
    let Some(Value::Object(tables)) = doc.get_mut("changes") else {
        return doc;
    };
    tables.retain(|t, _| !background(t, include));
    // A table listed with nothing inserted, deleted or updated did not change.
    tables.retain(|_, change| {
        ["inserted", "deleted", "updated"]
            .iter()
            .any(|k| change[*k].as_array().is_some_and(|rows| !rows.is_empty()))
    });
    let identity = |row: &Value| {
        ["id", "user_id", "topic_id", "post_id"]
            .iter()
            .map(|k| row[*k].to_string())
            .collect::<Vec<_>>()
    };
    for (table, change) in tables.iter_mut() {
        // Both sides list rows in their own order; sort by identity.
        for kind in ["inserted", "deleted"] {
            if let Some(Value::Array(rows)) = change.get_mut(kind) {
                if UNORDERED_INSERTS.contains(&table.as_str()) {
                    for row in rows.iter_mut() {
                        if let Value::Object(r) = row {
                            r.remove("id");
                        }
                    }
                    rows.sort_by_key(|r| r.to_string());
                } else {
                    // The whole row breaks ties, for tables without an id
                    // (post_timings).
                    rows.sort_by_key(|r| (identity(r), r.to_string()));
                }
            }
        }
        // A deleted row is measured by which row it was: its contents are
        // what the seed or the reference held before the case (a hot
        // score recalculated on schedule, say), not what the case wrote.
        if let Some(Value::Array(rows)) = change.get_mut("deleted") {
            for row in rows.iter_mut() {
                if let Value::Object(r) = row {
                    r.retain(|k, _| ["id", "user_id", "topic_id", "post_id"].contains(&k.as_str()));
                }
            }
        }
        let Some(Value::Array(updated)) = change.get_mut("updated") else {
            continue;
        };
        updated.sort_by_key(|row| identity(&row["before"]));
        for row in updated.iter_mut() {
            let (Some(Value::Object(before)), Some(Value::Object(after))) =
                (row.get("before"), row.get("after"))
            else {
                continue;
            };
            // A column the case stamped with the current time is measured by
            // that: what it held before is the seed's or the reference's
            // (a category touched on the reference since the snapshot).
            let changed: Map<String, Value> = after
                .iter()
                .filter(|(k, v)| before.get(*k) != Some(*v))
                .map(|(k, v)| {
                    let was = if v == "<now>" {
                        Some(&Value::Null)
                    } else {
                        before.get(k)
                    };
                    (k.clone(), serde_json::json!([was, v]))
                })
                .collect();
            *row = Value::Object(changed);
        }
    }
    doc
}

/// Rows keyed by primary key (the whole row without one).
fn keyed(rows: Vec<Value>, pk: &[String]) -> BTreeMap<String, Value> {
    rows.into_iter()
        .map(|r| {
            let key = if pk.is_empty() {
                r.to_string()
            } else {
                pk.iter()
                    .map(|c| r[c].to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            (key, r)
        })
        .collect()
}

/// What a case changed, as the recorder writes it.
async fn changes(
    pool: &PgPool,
    tables: &[(String, Vec<String>)],
    before_sums: &HashMap<String, String>,
    before_rows: &HashMap<String, Vec<Value>>,
) -> Map<String, Value> {
    let mut out = Map::new();
    for (table, pk) in tables {
        if checksum(pool, table).await == before_sums[table] {
            continue;
        }
        let old = keyed(before_rows[table].clone(), pk);
        let new = keyed(table_rows(pool, table).await, pk);
        let inserted: Vec<Value> = new
            .iter()
            .filter(|(k, _)| !old.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect();
        let deleted: Vec<Value> = old
            .iter()
            .filter(|(k, _)| !new.contains_key(*k))
            .map(|(_, v)| v.clone())
            .collect();
        let updated: Vec<Value> = new
            .iter()
            .filter_map(|(k, v)| {
                let before = old.get(k)?;
                (before != v).then(|| serde_json::json!({ "before": before, "after": v }))
            })
            .collect();
        out.insert(
            table.clone(),
            serde_json::json!({ "inserted": inserted, "deleted": deleted, "updated": updated }),
        );
    }
    out
}

/// The paths where two documents differ, a few of them.
fn differences(path: &str, rails: &Value, ours: &Value, out: &mut Vec<String>) {
    if out.len() >= 12 {
        return;
    }
    match (rails, ours) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in a {
                match b.get(k) {
                    Some(w) => differences(&format!("{path}/{k}"), v, w, out),
                    None => out.push(format!("{path}/{k}: missing (rails {})", short(v))),
                }
            }
            for k in b.keys().filter(|k| !a.contains_key(*k)) {
                out.push(format!("{path}/{k}: not in rails (ours {})", short(&b[k])));
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (v, w)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}/{i}"), v, w, out);
            }
        }
        _ if rails != ours => out.push(format!("{path}: {} (rails {})", short(ours), short(rails))),
        _ => {}
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 160 {
        format!("{}...", s.chars().take(160).collect::<String>())
    } else {
        s
    }
}

/// Replays one recorded case; the differences, empty when it matches.
/// The reference keeps writing in the background (admin notices, badge
/// grants), so a table can be ahead of the seed: each table Rails inserted
/// into starts where Rails' first new row did.
/// The rows each table's `INSERT INTO <table>` setup statements take ids
/// for,
/// from a rolled-back run of the setup; None when the setup can't run yet
/// (it needs the login's rows), and the recording's own evidence is used.
async fn setup_insert_counts(pool: &PgPool, case: &Value) -> Option<HashMap<String, i64>> {
    let mut counts = HashMap::new();
    // apply_settings inserts a site_settings row per setting after the
    // sequences are aligned, as SiteSetting.set did before Rails' requests.
    let settings = case["settings"].as_object().map_or(0, |s| s.len() as i64);
    if settings > 0 {
        counts.insert("site_settings".to_string(), settings);
    }
    let Some(setup) = case["setup"].as_array() else {
        return (!counts.is_empty()).then_some(counts);
    };
    let mut tx = pool.begin().await.ok()?;
    for sql in setup.iter().filter_map(Value::as_str) {
        let rows = sqlx::query(sql)
            .execute(&mut *tx)
            .await
            .ok()?
            .rows_affected() as i64;
        let lower = sql.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("insert into ") else {
            continue;
        };
        let table = rest.split_whitespace().next().unwrap_or_default();
        // Rows given their own ids take none from the sequence.
        let explicit_id = rest
            .split_once('(')
            .and_then(|(_, cols)| cols.split_once(')'))
            .is_some_and(|(cols, _)| cols.split(',').any(|c| c.trim() == "id"));
        if !explicit_id {
            *counts.entry(table.to_string()).or_insert(0) += rows;
        }
    }
    tx.rollback().await.ok()?;
    Some(counts)
}

async fn align_sequences(pool: &PgPool, case: &Value, setup_counts: Option<HashMap<String, i64>>) {
    let Some(changes) = case["changes"].as_object() else {
        return;
    };
    for (table, change) in changes {
        let Some(smallest) = change["inserted"]
            .as_array()
            .and_then(|rows| rows.iter().filter_map(|r| r["id"].as_i64()).min())
        else {
            continue;
        };
        // Where the recording's sequence stood when the requests began
        // (older recordings: the smallest new id, which is off when the
        // case inserts and deletes rows again).
        let first = case["next_ids"][table.as_str()]
            .as_i64()
            .unwrap_or(smallest);
        let sequence: Option<String> =
            sqlx::query_scalar("SELECT pg_get_serial_sequence($1, 'id')")
                .bind(format!("public.{table}"))
                .fetch_one(pool)
                .await
                .unwrap_or(None);
        if let Some(sequence) = sequence {
            // Rows the case's setup inserted into this table take ids
            // before the first new one: as many as the setup writes, else
            // (a setup that needs the login) those the recording shows
            // deleted or updated from where the sequence stands now.
            let inserted_by_setup = case["setup"].as_array().is_some_and(|setup| {
                setup.iter().filter_map(Value::as_str).any(|sql| {
                    sql.to_ascii_lowercase()
                        .starts_with(&format!("insert into {table} "))
                })
            });
            let (last, called): (i64, bool) =
                sqlx::query_as(&format!("SELECT last_value, is_called FROM {sequence}"))
                    .fetch_one(pool)
                    .await
                    .unwrap();
            let next = if called { last + 1 } else { last };
            let setup_rows = match setup_counts.as_ref() {
                Some(counts) => counts.get(table.as_str()).copied().unwrap_or(0),
                None => ["deleted", "updated"]
                    .iter()
                    .flat_map(|k| change[*k].as_array().into_iter().flatten())
                    .filter_map(|r| r["id"].as_i64().or_else(|| r["before"]["id"].as_i64()))
                    .filter(|id| inserted_by_setup && *id >= next && *id < first)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len() as i64,
            };
            sqlx::query("SELECT setval($1, GREATEST($2, 1), $2 > 0)")
                .bind(&sequence)
                .bind(first - 1 - setup_rows)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

/// A url's query value by name.
fn query_param(url: &str, name: &str) -> String {
    url.split_once('?')
        .and_then(|(_, q)| {
            form_urlencoded::parse(q.split('#').next().unwrap_or("").as_bytes())
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_default()
}

/// A case request's `discourse_connect`: its `fields` signed with `secret`
/// as DiscourseConnectBase#payload signs them (Base64 of the query,
/// HMAC-SHA256 in hex), the `sso` and `sig` to add to the path's query.
fn discourse_connect_query(dc: &Value) -> String {
    use base64::Engine;
    use hmac::Mac;
    let query: String = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            dc["fields"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), v)
                }),
        )
        .finish();
    let payload = base64::engine::general_purpose::STANDARD.encode(query);
    let mut mac =
        hmac::Hmac::<sha2::Sha256>::new_from_slice(dc["secret"].as_str().unwrap_or("").as_bytes())
            .unwrap();
    mac.update(payload.as_bytes());
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let escaped: String = form_urlencoded::byte_serialize(payload.as_bytes()).collect();
    format!("sso={escaped}&sig={sig}")
}

/// `{{path.to.value}}` with an optional `|reverse`, `|last_segment`,
/// `|request_param` (a url's `request` query value), `|state_param` (its
/// `state`, an OAuth authorize redirect's) or `|sso_nonce` (the nonce in a
/// DiscourseConnect redirect's signed payload).
fn resolve(value: &Value, state: &Value) -> Value {
    static REF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\{\{([^}|]+)(\|reverse|\|last_segment|\|request_param|\|state_param|\|sso_nonce)?\}\}",
        )
        .unwrap()
    });
    match value {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), resolve(v, state)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| resolve(v, state)).collect()),
        Value::String(s) => Value::String(
            REF.replace_all(s, |c: &regex::Captures| {
                let mut found = state;
                for key in c[1].split('.') {
                    found = match key.parse::<usize>() {
                        Ok(i) if found.is_array() => &found[i],
                        _ => &found[key],
                    };
                }
                let text = match found {
                    Value::String(s) => s.clone(),
                    Value::Null => format!("unresolved-{}", c[1].replace('.', "-")),
                    other => other.to_string(),
                };
                match c.get(2).map(|m| m.as_str()) {
                    Some("|reverse") => text.chars().rev().collect(),
                    Some("|last_segment") => text.rsplit('/').next().unwrap_or("").to_string(),
                    Some("|request_param") => query_param(&text, "request"),
                    Some("|state_param") => query_param(&text, "state"),
                    Some("|sso_nonce") => {
                        use base64::Engine;
                        let payload = query_param(&text, "sso");
                        let decoded = base64::engine::general_purpose::STANDARD
                            .decode(payload.trim())
                            .unwrap_or_default();
                        form_urlencoded::parse(&decoded)
                            .find(|(k, _)| k == "nonce")
                            .map(|(_, v)| v.into_owned())
                            .unwrap_or_default()
                    }
                    _ => text,
                }
            })
            .into_owned(),
        ),
        other => other.clone(),
    }
}

/// Jobs in the queue after `after_id`, as (id, name, `[name, args]` with
/// " in Ns" for a delayed one, as the recorder writes them).
async fn queued_jobs(pool: &PgPool, after_id: i64) -> Vec<(i64, String, Value)> {
    let rows: Vec<(i64, String, Value, f64, bool, bool)> = sqlx::query_as(
        "SELECT id, name, args, EXTRACT(EPOCH FROM run_at - created_at)::float8, at_time, delayed \
         FROM discourse_rs.jobs WHERE id > $1 ORDER BY id",
    )
    .bind(after_id)
    .fetch_all(pool)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(id, name, args, delay, at_time, delayed)| {
            // " at" for Jobs.enqueue_at, as the recorder writes it.
            let shown = if at_time {
                format!("{name} at")
            } else if delayed || delay >= 1.0 {
                // Jobs.enqueue_in, " in 0s" included.
                format!("{name} in {}s", delay.round() as i64)
            } else {
                name.clone()
            };
            (id, name, serde_json::json!([shown, args]))
        })
        .collect()
}

/// A case's `settings`, as `SiteSetting.set` stores them: a row typed by
/// the setting's definition, booleans as "t"/"f".
async fn apply_settings(pool: &PgPool, state: &AppState, case: &Value) {
    let Some(settings) = case["settings"].as_object() else {
        return;
    };
    for (name, value) in settings {
        let def = state
            .site_setting_defs
            .get(name)
            .unwrap_or_else(|| panic!("case sets an unknown setting {name}"));
        let value = match value {
            Value::Bool(true) => "t".to_string(),
            Value::Bool(false) => "f".to_string(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        sqlx::query("DELETE FROM site_settings WHERE name = $1")
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ($1, $2, $3, now(), now())",
        )
        .bind(name)
        .bind(def.data_type as i32)
        .bind(value)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Where `remote` pages are served, on both sides: fixtures.localhost
/// resolves to loopback here and in the reference's container.
const REMOTE_ADDRS: [&str; 2] = ["127.0.0.1:8099", "[::1]:8099"];

/// A case's `remote`: request targets (path and query) on
/// http://fixtures.localhost:8099 and what each answers, `{status,
/// headers, body}`, as scripts/record_writes.rb serves them; anything else
/// is a 404. Cases run side by side, so a case waits for the port.
async fn serve_remote(case: &Value) -> Vec<tokio::task::JoinHandle<()>> {
    let Some(pages) = case["remote"].as_object() else {
        return Vec::new();
    };
    let pages = std::sync::Arc::new(pages.clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    let listeners = loop {
        let mut bound = Vec::new();
        for addr in REMOTE_ADDRS {
            match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => bound.push(l),
                Err(_) => break,
            }
        }
        if bound.len() == REMOTE_ADDRS.len() {
            break bound;
        }
        drop(bound);
        assert!(
            std::time::Instant::now() < deadline,
            "port 8099 stayed busy"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    listeners
        .into_iter()
        .map(|listener| {
            let pages = pages.clone();
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    tokio::spawn(answer_remote(socket, pages.clone()));
                }
            })
        })
        .collect()
}

async fn answer_remote(
    mut socket: tokio::net::TcpStream,
    pages: std::sync::Arc<Map<String, Value>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut request = Vec::new();
    let mut buf = [0u8; 4096];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    // The body too (a web hook's POST): closing the socket with it unread
    // could reset the connection before the client reads the answer.
    let end = request
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(0)
        + 4;
    let length = String::from_utf8_lossy(&request[..end])
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while request.len() - end < length {
        match socket.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let mut line = request.lines().next().unwrap_or("").split(' ');
    let method = line.next().unwrap_or("");
    let target = line.next().unwrap_or("");
    let page = pages.get(target);
    let status = page.and_then(|p| p["status"].as_u64()).unwrap_or(404);
    let body = page.and_then(|p| p["body"].as_str()).unwrap_or("");
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in page
        .and_then(|p| p["headers"].as_object())
        .into_iter()
        .flatten()
    {
        head.push_str(&format!("{name}: {}\r\n", value.as_str().unwrap_or("")));
    }
    head.push_str("\r\n");
    let _ = socket.write_all(head.as_bytes()).await;
    if method != "HEAD" {
        let _ = socket.write_all(body.as_bytes()).await;
    }
    let _ = socket.shutdown().await;
}

async fn replay(case: &Value, run_jobs: &[String], ignore: &[String]) -> Vec<String> {
    let db = TestDb::with_clock().await;
    // The case runs from when Rails ran it, so what counts from now (days
    // ago, new topic windows) answers as it did then.
    if let Some(started) = case["started_at"].as_str().and_then(timestamp) {
        db.shift_clock(started.and_utc() - chrono::Utc::now()).await;
    }
    // A public dir of the case's own, for the files uploads store.
    let public = std::env::temp_dir().join(format!(
        "discourse-rs-writes-{}-{}",
        std::process::id(),
        case["name"].as_str().unwrap()
    ));
    // `files`: fixture files at the case's upload urls.
    for (url, name) in case["files"].as_object().into_iter().flatten() {
        let path = public.join(url.trim_start_matches('/'));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("parity/writes/files")
                .join(name.as_str().unwrap()),
            &path,
        )
        .unwrap();
    }
    let mut config = recorded_config();
    config.public_dir = public.clone();
    // RateLimiter: off, as the recorder runs Rails, unless the case asks.
    config.rate_limits = case["rate_limits"] == true;
    // `remote_hosts`: login providers' fixed endpoints on the fixtures
    // server, as the recorder rewrites Faraday's URLs.
    config.auth_provider_hosts = case["remote_hosts"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(from, to)| {
            (
                from.clone(),
                format!("http://fixtures.localhost:8099{}", to.as_str().unwrap()),
            )
        })
        .collect();
    let app_state = state(db.pool.clone(), config).await;
    // Counted before the sequences are set: the rolled-back run of the
    // setup still advances them.
    let setup_counts = setup_insert_counts(&db.pool, case).await;
    reset_sequences(&db.pool).await;
    align_sequences(&db.pool, case, setup_counts).await;
    // The recorder logs in and runs the setup inside its transaction, so
    // their now() is the transaction's start: pinned there, setup rows
    // carry Rails' very timestamps rather than ones seconds later.
    let setup_at = case["transaction_started_at"].as_str().and_then(timestamp);
    if let Some(at) = setup_at {
        db.pin_clock(Some(at.and_utc())).await;
    }
    // From before the login and setup, as the recorder's transaction
    // (and so their now()) starts before both.
    let started: NaiveDateTime = sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    // To the millisecond, as Rails' is: with the clock pinned, a setup row
    // stamped at that very instant reads in JSON as a little before it.
    let started = started
        .with_nanosecond(started.nanosecond() / 1_000_000 * 1_000_000)
        .unwrap_or(started);
    let mut client = Client {
        state: app_state,
        cookies: Vec::new(),
        csrf: None,
        rate_headers: Map::new(),
        keep_headers: case["keep_headers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| h.as_str().map(str::to_string))
            .collect(),
    };
    if let Some(user) = case["user"].as_str() {
        client.login(user).await;
    } else {
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
    }
    apply_settings(&db.pool, &client.state, case).await;
    // `remote`: the other site's pages the case fetches, served as the
    // recorder served them; held until the case is done.
    let _remote = serve_remote(case).await;
    // `setup`: the case's fixture rows, as the recorder ran them.
    for sql in case["setup"].as_array().into_iter().flatten() {
        sqlx::query(sql.as_str().unwrap())
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("setup {sql}: {e}"));
    }
    // `readonly`: read-only mode's keys, set as the recorder sets them.
    for key in case["readonly"].as_array().into_iter().flatten() {
        let mut conn = db.pool.acquire().await.unwrap();
        discourse_rs::readonly::set_key(&mut conn, key.as_str().unwrap())
            .await
            .unwrap();
    }
    // `redis`: other Redis values the requests read, as the recorder sets
    // them (no TTL): the port keeps them in discourse_rs.cached_values.
    for (key, value) in case["redis"].as_object().into_iter().flatten() {
        let mut conn = db.pool.acquire().await.unwrap();
        discourse_rs::owned_schema::cached_setex(
            &mut conn,
            key,
            100 * 365 * 24 * 3600,
            value.as_str().unwrap(),
        )
        .await
        .unwrap();
    }
    // The recorder drops what the login and setup enqueued (ENQUEUED.clear),
    // such as a staff login's suspicious_login.
    sqlx::query("DELETE FROM discourse_rs.jobs")
        .execute(&db.pool)
        .await
        .unwrap();
    // The requests run on the shifted clock, time passing as it did there.
    if setup_at.is_some() {
        db.pin_clock(None).await;
    }

    let tables = tables(&db.pool, &include_tables(case)).await;
    let mut before_sums = HashMap::new();
    let mut before_rows = HashMap::new();
    for (table, _) in &tables {
        before_sums.insert(table.clone(), checksum(&db.pool, table).await);
        before_rows.insert(table.clone(), table_rows(&db.pool, table).await);
    }
    let mut responses = Vec::new();
    let mail_before_requests = client.state.mailer.sent().len();
    for request in case["requests"].as_array().unwrap() {
        let method = Method::from_bytes(request["method"].as_str().unwrap().as_bytes()).unwrap();
        // {{...}} references, as the recorder resolves them: earlier
        // responses and the last queued job of a name.
        let mut jobs = Map::new();
        for (_, name, shown) in queued_jobs(&db.pool, 0).await {
            jobs.insert(name, shown[1].clone());
        }
        let state = serde_json::json!({ "responses": responses, "jobs": jobs });
        let params = request.get("params").map(|p| resolve(p, &state));
        let body = params.as_ref().filter(|p| p.is_object());
        let mut path = resolve(&request["path"], &state)
            .as_str()
            .unwrap()
            .to_string();
        // `discourse_connect`: a DiscourseConnect provider's signed answer
        // added to the query, as the recorder adds it.
        if let Some(dc) = request.get("discourse_connect") {
            let query = discourse_connect_query(&resolve(dc, &state));
            path = format!(
                "{path}{}{query}",
                if path.contains('?') { "&" } else { "?" }
            );
        }
        let extra = request["headers"].as_object().cloned().unwrap_or_default();
        let (status, body) = client.send_with(method, &path, body, &extra).await;
        let mut response = serde_json::json!({ "status": status, "body": body });
        if !client.rate_headers.is_empty() {
            response["headers"] = Value::Object(client.rate_headers.clone());
        }
        responses.push(response);
    }
    // The mail the requests sent themselves, apart from the jobs'.
    let mail_after_requests = client.state.mailer.sent().len();
    // The jobs the requests enqueued, as the recorder writes them; then
    // the case's run_jobs run, and what they enqueue is listed apart.
    let from_requests = queued_jobs(&db.pool, 0).await;
    let last_id = from_requests.last().map(|(id, _, _)| *id).unwrap_or(0);
    let mut job_failures = Vec::new();
    // Jobs the run jobs enqueue run too when named, as on the recording.
    let mut pending: std::collections::VecDeque<(i64, String)> = from_requests
        .iter()
        .map(|(id, name, _)| (*id, name.clone()))
        .collect();
    let mut seen_id = last_id;
    // `enqueue`: the case's own jobs, after its requests', run as theirs.
    if let Some(list) = case["enqueue"].as_array() {
        let mut conn = db.pool.acquire().await.unwrap();
        for job in list {
            discourse_rs::jobs::enqueue(
                &mut conn,
                job[0].as_str().unwrap(),
                job.get(1).cloned().unwrap_or_else(|| serde_json::json!({})),
            )
            .await
            .unwrap();
        }
        for (id, name, _) in queued_jobs(&db.pool, seen_id).await {
            seen_id = id;
            pending.push_back((id, name));
        }
    }
    // `job_clock` (`table.column`): the jobs run at the time Rails' did, the
    // latest value its run wrote in that column, so what they compute from
    // the clock (a hot score's age) comes out the same.
    let job_clock = case["job_clock"]
        .as_str()
        .and_then(|spec| spec.split_once('.'))
        .and_then(|(table, column)| {
            let change = &case["changes"][table];
            let mut times: Vec<NaiveDateTime> = change["inserted"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(
                    change["updated"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|row| &row["after"]),
                )
                .filter_map(|row| row[column].as_str().and_then(timestamp))
                .collect();
            times.sort();
            times.pop()
        });
    if let Some(at) = job_clock {
        db.pin_clock(Some(at.and_utc())).await;
    }
    let mut from_jobs: Vec<Value> = Vec::new();
    while let Some((id, name)) = pending.pop_front() {
        if !run_jobs.contains(&name) {
            continue;
        }
        discourse_rs::jobs::perform_now(&client.state, id)
            .await
            .unwrap();
        let error: Option<Option<String>> =
            sqlx::query_scalar("SELECT last_error FROM discourse_rs.jobs WHERE id = $1")
                .bind(id)
                .fetch_optional(&db.pool)
                .await
                .unwrap();
        if let Some(Some(error)) = error {
            job_failures.push(format!("{name}: {error}"));
        }
        for (new_id, new_name, shown) in queued_jobs(&db.pool, seen_id).await {
            seen_id = new_id;
            from_jobs.push(shown);
            pending.push_back((new_id, new_name));
        }
    }
    if job_clock.is_some() {
        db.pin_clock(None).await;
    }
    let mut ours = serde_json::json!({
        "responses": responses,
        "jobs": from_requests
            .into_iter()
            .map(|(_, _, j)| j)
            .filter(|j| !THROTTLED_JOBS.contains(&j[0].as_str().unwrap_or("")))
            .collect::<Vec<_>>(),
        "changes": changes(&db.pool, &tables, &before_sums, &before_rows).await,
    });
    let mail_json = |messages: &[discourse_rs::email::Message]| -> Value {
        Value::Array(
            messages
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "headers": m.headers.iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>(),
                        "text": m.text,
                        "html": m.html,
                    })
                })
                .collect(),
        )
    };
    let sent = client.state.mailer.sent();
    if !run_jobs.is_empty() {
        ours["jobs_from_jobs"] = Value::Array(from_jobs);
        ours["emails"] = mail_json(&sent[mail_after_requests..]);
    }
    // Recordings made since the recorder keeps the requests' own mail.
    if case.get("request_emails").is_some() {
        ours["request_emails"] = mail_json(&sent[mail_before_requests..mail_after_requests]);
    }

    // CURRENT_TIMESTAMP on the reference is its rolled-back transaction's
    // start, which opened before the case did.
    let mut rails_started = timestamp(case["started_at"].as_str().unwrap()).unwrap();
    if let Some(t) = case["transaction_started_at"].as_str().and_then(timestamp) {
        rails_started = rails_started.min(t);
    }
    // JSON responses carry milliseconds: a row written at the transaction's
    // start reads as slightly before it.
    rails_started = rails_started
        .with_nanosecond(rails_started.nanosecond() / 1_000_000 * 1_000_000)
        .unwrap_or(rails_started);
    let without_plugin_jobs = |jobs: &Value| -> Vec<Value> {
        jobs.as_array()
            .into_iter()
            .flatten()
            .filter(|j| {
                let name = j[0].as_str().unwrap_or("");
                !PLUGIN_JOBS.contains(&name) && !THROTTLED_JOBS.contains(&name)
            })
            .cloned()
            .collect()
    };
    let rails_jobs = without_plugin_jobs(&case["jobs"]);
    let mut rails = serde_json::json!({ "responses": case["responses"], "changes": case["changes"], "jobs": rails_jobs });
    if let Some(from_jobs) = case.get("jobs_from_jobs") {
        rails["jobs_from_jobs"] = Value::Array(without_plugin_jobs(from_jobs));
        rails["emails"] = case
            .get("emails")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
    }
    if let Some(mail) = case.get("request_emails") {
        rails["request_emails"] = mail.clone();
    }
    // The reference runs in development, where a route that does not match
    // (an admin route for a non-admin) renders the Routing Error page, and
    // a record not found its exception page; the port renders the
    // production 404. The status is still compared.
    for i in 0..rails["responses"].as_array().map_or(0, Vec::len) {
        // A rate limit's wait counts wall-clock seconds, and a replay under
        // load runs a case's requests slower than Rails did: within ten
        // seconds (a wrong window is off by far more), ours takes Rails'
        // (its words and Retry-After too).
        let theirs = &rails["responses"][i];
        if theirs["body"]["error_type"] == "rate_limit"
            && ours["responses"][i]["body"]["error_type"] == "rate_limit"
            && let (Some(a), Some(b)) = (
                theirs["body"]["extras"]["wait_seconds"].as_i64(),
                ours["responses"][i]["body"]["extras"]["wait_seconds"].as_i64(),
            )
            && (a - b).abs() <= 10
        {
            let theirs = theirs.clone();
            let mine = &mut ours["responses"][i];
            mine["body"]["extras"] = theirs["body"]["extras"].clone();
            mine["body"]["errors"] = theirs["body"]["errors"].clone();
            if !theirs["headers"]["Retry-After"].is_null() {
                mine["headers"]["Retry-After"] = theirs["headers"]["Retry-After"].clone();
            }
        }
        let routing_error = rails["responses"][i]["status"] == 404
            && rails["responses"][i]["body"].as_str().is_some_and(|b| {
                b.starts_with("Routing Error") || b.starts_with("ActiveRecord::RecordNotFound in ")
            });
        if routing_error && ours["responses"][i]["status"] == 404 {
            rails["responses"][i]["body"] = Value::String("<not found>".into());
            ours["responses"][i]["body"] = Value::String("<not found>".into());
        }
    }
    // `fragment` ([start, end]): an HTML page is compared from the first
    // `start` through the `end` after it, the view without its layout
    // (theme, plugin stylesheets, the profiler). A form's token is left
    // out: the recorder turns forgery protection off.
    if let Some([start, end]) = case["fragment"].as_array().map(Vec::as_slice) {
        static TOKEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r#"<input type="hidden" name="authenticity_token" value="[^"]*" />"#)
                .unwrap()
        });
        let (start, end) = (start.as_str().unwrap_or(""), end.as_str().unwrap_or(""));
        for doc in [&mut rails, &mut ours] {
            for response in doc["responses"].as_array_mut().into_iter().flatten() {
                let Some(body) = response["body"].as_str() else {
                    continue;
                };
                let Some(from) = body.find(start) else {
                    continue;
                };
                let rest = &body[from..];
                let to = rest.find(end).map_or(rest.len(), |i| i + end.len());
                response["body"] = Value::String(TOKEN.replace_all(&rest[..to], "").into_owned());
            }
        }
    }
    for doc in [&mut rails, &mut ours] {
        for ptr in ignore {
            discourse_rs::parity::ignore_path(doc, ptr);
        }
        for response in doc["responses"].as_array_mut().into_iter().flatten() {
            if let Some(body) = response["body"].as_object_mut() {
                for key in ["user", "current_user"] {
                    if let Some(user) = body.get_mut(key) {
                        undrift_user(user);
                    }
                }
            }
            discourse_rs::parity::unorder(&mut response["body"]);
        }
    }
    if let Ok(dir) = std::env::var("WRITES_DUMP") {
        let name = case["name"].as_str().unwrap_or("case");
        std::fs::write(
            format!("{dir}/{name}.ours.json"),
            serde_json::to_string_pretty(&ours).unwrap(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/{name}.rails.json"),
            serde_json::to_string_pretty(&rails).unwrap(),
        )
        .unwrap();
    }
    let mut out: Vec<String> = job_failures
        .into_iter()
        .map(|f| format!("job failed: {f}"))
        .collect();
    // An image upload whose processed file our encoders wrote differently
    // (src/images.rs) is compared through its byte-derived values mapped
    // onto Rails': its file is checked against its own sha1.
    let encoded = encoded_uploads(&rails, &ours);
    for upload in &encoded {
        let stored = public.join(upload.url.trim_start_matches('/'));
        match std::fs::read(&stored) {
            Ok(bytes) => {
                use sha1::Digest;
                let sha1 = format!("{:x}", sha1::Sha1::digest(&bytes));
                if upload.sha1 != sha1 {
                    out.push(format!("{}: stored file has sha1 {sha1}", upload.url));
                }
            }
            Err(e) => out.push(format!("{}: not stored ({e})", upload.url)),
        }
    }
    ours = map_encoded(&ours, &encoded, None);
    // The files the uploads stored, where Rails stored them.
    for upload in rails["changes"]["uploads"]["inserted"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if encoded.iter().any(|e| e.id == upload["id"]) {
            continue;
        }
        let url = upload["url"].as_str().unwrap_or_default();
        let stored = public.join(url.trim_start_matches('/'));
        match std::fs::read(&stored) {
            Ok(bytes) => {
                use sha1::Digest;
                let sha1 = format!("{:x}", sha1::Sha1::digest(&bytes));
                if upload["sha1"] != sha1.as_str() {
                    out.push(format!("{url}: stored file has sha1 {sha1}"));
                }
            }
            Err(e) => out.push(format!("{url}: not stored ({e})")),
        }
    }
    let _ = std::fs::remove_dir_all(&public);
    differences(
        "",
        &changed_columns(
            normalize(
                &{
                    decrypt_user_api_payloads(&mut rails);
                    hide_secrets(&mut rails);
                    web_hook_values(&mut rails);
                    ignore_in_payloads(&mut rails, ignore);
                    approximate(&mut rails, case);
                    digests(&mut rails, case);
                    unordered(&mut rails, case);
                    unordered_responses(&mut rails, case);
                    random_delays(&mut rails, case);
                    rails
                },
                rails_started,
            ),
            &include_tables(case),
        ),
        &changed_columns(
            normalize(
                &{
                    decrypt_user_api_payloads(&mut ours);
                    hide_secrets(&mut ours);
                    web_hook_values(&mut ours);
                    ignore_in_payloads(&mut ours, ignore);
                    approximate(&mut ours, case);
                    digests(&mut ours, case);
                    unordered(&mut ours, case);
                    unordered_responses(&mut ours, case);
                    random_delays(&mut ours, case);
                    ours
                },
                started,
            ),
            &include_tables(case),
        ),
        &mut out,
    );
    out
}

/// An image upload of ours whose bytes differ from Rails' row of the same
/// id: what its bytes decided, and Rails' values for them.
struct EncodedUpload {
    id: Value,
    url: String,
    sha1: String,
    pairs: Vec<(String, String)>,
    filesize: (Value, Value),
}

/// `Upload.base62_sha1`: the sha1 as a base62 number.
fn base62(sha1: &str) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut n: Vec<u8> = (0..sha1.len() / 2)
        .filter_map(|i| u8::from_str_radix(&sha1[2 * i..2 * i + 2], 16).ok())
        .collect();
    let mut out = Vec::new();
    while n.iter().any(|b| *b != 0) {
        let mut rem = 0u32;
        for b in n.iter_mut() {
            let acc = (rem << 8) | u32::from(*b);
            *b = (acc / 62) as u8;
            rem = acc % 62;
        }
        out.push(DIGITS[rem as usize]);
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// The image uploads (rows with a width) both sides inserted under one id
/// with different sha1s: byte-for-byte equality is not expected of the
/// image encoders, so their sha1, its short form and the file size are
/// mapped onto Rails' before comparing. Everything else about the row
/// (dimensions, colour, names, extension) still compares.
fn encoded_uploads(rails: &Value, ours: &Value) -> Vec<EncodedUpload> {
    let inserted = |v: &Value| -> Vec<Value> {
        v["changes"]["uploads"]["inserted"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let rails_rows = inserted(rails);
    let uploads = inserted(ours)
        .into_iter()
        .filter(|o| !o["width"].is_null())
        .filter_map(|o| {
            let r = rails_rows.iter().find(|r| r["id"] == o["id"])?;
            let (os, rs) = (o["sha1"].as_str()?, r["sha1"].as_str()?);
            if os == rs {
                return None;
            }
            let size = |row: &Value| {
                discourse_rs::uploads::human_size(row["filesize"].as_i64().unwrap_or(0))
            };
            Some(EncodedUpload {
                id: o["id"].clone(),
                url: o["url"].as_str().unwrap_or_default().to_string(),
                sha1: os.to_string(),
                pairs: vec![
                    (os.to_string(), rs.to_string()),
                    (base62(os), base62(rs)),
                    (size(&o), size(r)),
                ],
                filesize: (o["filesize"].clone(), r["filesize"].clone()),
            })
        })
        .collect::<Vec<_>>();
    // Thumbnails (optimized_images) are always our encoder's: their sha1
    // and size map the same way. Their urls are built from the original
    // upload's sha1, so they need nothing.
    let optimized = |v: &Value| -> Vec<Value> {
        v["changes"]["optimized_images"]["inserted"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let rails_thumbnails = optimized(rails);
    let mut out = uploads;
    for o in optimized(ours) {
        let Some(r) = rails_thumbnails.iter().find(|r| r["id"] == o["id"]) else {
            continue;
        };
        let (Some(os), Some(rs)) = (o["sha1"].as_str(), r["sha1"].as_str()) else {
            continue;
        };
        if os == rs {
            continue;
        }
        out.push(EncodedUpload {
            id: Value::Null,
            url: o["url"].as_str().unwrap_or_default().to_string(),
            sha1: os.to_string(),
            pairs: vec![(os.to_string(), rs.to_string())],
            filesize: (o["filesize"].clone(), r["filesize"].clone()),
        });
    }
    out
}

/// `value` with each encoded upload's values replaced by Rails'.
fn map_encoded(value: &Value, encoded: &[EncodedUpload], key: Option<&str>) -> Value {
    if encoded.is_empty() {
        return value.clone();
    }
    match value {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), map_encoded(v, encoded, Some(k))))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|v| map_encoded(v, encoded, key)).collect()),
        Value::String(s) => {
            let mut s = s.clone();
            for e in encoded {
                for (from, to) in &e.pairs {
                    s = s.replace(from, to);
                }
            }
            Value::String(s)
        }
        Value::Number(_) if key == Some("filesize") => encoded
            .iter()
            .find(|e| &e.filesize.0 == value)
            .map(|e| e.filesize.1.clone())
            .unwrap_or_else(|| value.clone()),
        other => other.clone(),
    }
}

/// A recorded case: its definition (`<area>/<name>.case.json`) and what
/// Rails did (`<area>/<name>.json`, written by scripts/record-writes).
struct Case {
    area: String,
    name: String,
    recorded: Value,
    run_jobs: Vec<String>,
    /// Why the port does not do it like Rails yet.
    not_yet: Option<String>,
    /// JSON pointers into the recording left out of the comparison on
    /// both sides (Rails' random suggested topics).
    ignore: Vec<String>,
}

/// Every case under parity/writes, by area and name.
fn cases() -> Vec<Case> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/writes");
    let mut out = Vec::new();
    let mut areas: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir() && e.file_name() != "files")
        .collect();
    areas.sort_by_key(|e| e.file_name());
    for area in areas {
        let mut files: Vec<_> = std::fs::read_dir(area.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().ends_with(".case.json"))
            .collect();
        files.sort();
        for path in files {
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            let name = file.trim_end_matches(".case.json").to_string();
            let definition: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let recording = path.with_file_name(format!("{name}.json"));
            let recorded: Value = match std::fs::read_to_string(&recording) {
                Ok(text) => serde_json::from_str(&text).unwrap(),
                Err(_) => Value::Null,
            };
            out.push(Case {
                area: area.file_name().to_string_lossy().to_string(),
                name,
                recorded,
                run_jobs: definition["run_jobs"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|n| n.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                not_yet: definition["not_yet"].as_str().map(str::to_string),
                ignore: definition["ignore"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|p| p.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    out
}

/// Each case is a test, `<area>::<name>`, so nextest runs them side by
/// side and reports and filters them one by one (`-E 'test(drafts::)'`).
/// A case with `not_yet` passes while it still differs and fails once it
/// matches, so the marker comes off when the port catches up.
fn main() {
    // A 500's cause is only logged; show it next to the diff.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_test_writer()
        .try_init();
    let mut args = libtest_mimic::Arguments::from_args();
    // Run as one process (cargo test), every case copies the template
    // database and holds up to four connections: four at a time.
    if args.test_threads.is_none() {
        args.test_threads = Some(4);
    }
    let trials = cases()
        .into_iter()
        .map(|case| {
            let label = format!("{}::{}", case.area, case.name);
            let kind = if case.not_yet.is_some() {
                "not_yet"
            } else {
                ""
            };
            libtest_mimic::Trial::test(label, move || {
                if case.recorded.is_null() {
                    return Err(format!(
                        "not recorded: run scripts/record-writes for {}/{}",
                        case.area, case.name
                    )
                    .into());
                }
                let diff = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(replay(&case.recorded, &case.run_jobs, &case.ignore));
                match (&case.not_yet, diff.is_empty()) {
                    (None, true) => Ok(()),
                    (None, false) => Err(diff.join("\n").into()),
                    (Some(_), false) => Ok(()),
                    (Some(reason), true) => {
                        Err(format!("matches Rails now: remove its not_yet ({reason})").into())
                    }
                }
            })
            .with_kind(kind)
        })
        .collect();
    libtest_mimic::run(&args, trials).exit();
}
