//! Write requests replayed against Rails' recording: each case is
//! parity/writes/<area>/<name>.case.json, recorded by scripts/record-writes
//! into <name>.json beside it, and is a test of its own (`<area>::<name>`).
//! It runs on a fresh copy of the seed, and its responses and every row it
//! inserted, updated or deleted must be what Discourse did.
//!
//! Timestamps the case itself wrote (at or after its start) compare as
//! `<now>`. Ids line up because both sides set every sequence to its
//! table's max(id) first. The jobs Rails enqueued are not compared: the
//! port has no job runner yet.

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
/// (scheduler_stats, top_topics), or the login's token row, whose id and
/// random token cannot line up (the seed leaves tokens out); tests/sessions.rs
/// covers it.
const BACKGROUND_TABLES: [&str; 3] = ["scheduler_stats", "top_topics", "user_auth_tokens"];

/// Compared as a set, without ids: tables Rails fills from a query without
/// ORDER BY (sidebar links from `Category.where(id:).pluck(:id)`), whose
/// ids follow each database's heap order, and upload references, which
/// `link_post_uploads` deletes and inserts again within a case (on create,
/// then in the post processor), so their ids count rows the recording no
/// longer shows and the sequence cannot be lined up from it.
const UNORDERED_INSERTS: [&str; 2] = ["sidebar_section_links", "upload_references"];

/// Keys plugins add to the post and topic serializers on the reference.
const PLUGIN_KEYS: [&str; 19] = [
    "has_accepted_answer",
    "event",
    "calendar_details",
    "accepted_answer",
    "can_accept_answer",
    "can_unaccept_answer",
    "topic_accepted_answer",
    "reactions",
    "current_user_reaction",
    "reaction_users_count",
    "current_user_used_main_reaction",
    "can_vote",
    "vote_count",
    "user_voted",
    "valid_reactions",
    "can_create_shared_issue",
    "shared_issue_visible",
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

/// The topic view details whose order Rails leaves to the database:
/// participants by post count with ties unordered (post_counts_by_user
/// sorts by count only), and allowed_users (an unordered association).
/// Ties go by id, on both sides.
fn unorder_topic_details(details: &mut Value) {
    let id = |v: &Value| v["id"].as_i64().unwrap_or(0);
    if let Some(participants) = details
        .get_mut("participants")
        .and_then(Value::as_array_mut)
    {
        participants.sort_by_key(|p| (-p["post_count"].as_i64().unwrap_or(0), id(p)));
    }
    if let Some(users) = details
        .get_mut("allowed_users")
        .and_then(Value::as_array_mut)
    {
        users.sort_by_key(id);
    }
}

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
const PLUGIN_USER_KEYS: [&str; 21] = [
    "can_chat_user",
    "accepted_answers",
    "chat_enabled",
    "ignore_channel_wide_mention",
    "show_thread_title_prompts",
    "chat_announce_new_messages",
    "chat_channel_list_filter",
    "chat_channel_list_sort",
    "chat_channel_list_sort_starred",
    "chat_channel_list_sort_dms",
    "chat_channel_list_filter_starred",
    "chat_channel_list_filter_dms",
    "chat_new_message_sound",
    "chat_email_frequency",
    "chat_header_indicator_preference",
    "chat_separate_sidebar_mode",
    "chat_send_shortcut",
    "chat_quick_reaction_type",
    "chat_quick_reactions_custom",
    "event_reminder_preference",
    "notify_on_solved",
];

/// User serializer keys that follow what the reference does in the
/// background between seeding and recording (profile views, other
/// sessions, badge grants): not compared.
const DRIFTING_USER_KEYS: [&str; 3] = [
    "profile_view_count",
    "user_auth_tokens",
    "featured_user_badge_ids",
];

/// A serialized user in a response, less plugin and drifting keys, with
/// `sidebar_category_ids` (plucked without ORDER BY) sorted.
fn undrift_user(user: &mut Value) {
    let Some(map) = user.as_object_mut() else {
        return;
    };
    map.retain(|k, _| {
        !PLUGIN_USER_KEYS.contains(&k.as_str()) && !DRIFTING_USER_KEYS.contains(&k.as_str())
    });
    if let Some(Value::Object(options)) = map.get_mut("user_option") {
        options.retain(|k, _| !PLUGIN_USER_KEYS.contains(&k.as_str()));
    }
    if let Some(Value::Array(ids)) = map.get_mut("sidebar_category_ids") {
        ids.sort_by_key(|v| v.as_i64());
    }
}

/// Jobs plugins enqueue on the reference (discourse-narrative-bot, and
/// discourse-topic-voting on topic_status_updated).
const PLUGIN_JOBS: [&str; 3] = [
    "bot_input",
    "Jobs::DiscourseTopicVoting::VoteRelease",
    "Jobs::DiscourseTopicVoting::VoteReclaim",
];

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

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
        let api = extra.keys().any(|k| k.eq_ignore_ascii_case("api-key"));
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
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
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

/// A `{"fixture": name}` param: a file from parity/writes/files, sent as
/// the recorder sent it, a Rack::Test::UploadedFile in a multipart form.
fn is_fixture(value: &Value) -> bool {
    value.get("fixture").is_some_and(Value::is_string)
}

fn multipart(params: &Map<String, Value>) -> (String, Vec<u8>) {
    let boundary = "----discourse-rs-writes-boundary";
    let files = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/writes/files");
    let mut out = Vec::new();
    for (name, value) in params {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        if is_fixture(value) {
            let file = value["fixture"].as_str().unwrap();
            out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                     Content-Type: application/octet-stream\r\n\r\n"
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
async fn tables(pool: &PgPool) -> Vec<(String, Vec<String>)> {
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
        .filter(|(t, _)| !BACKGROUND_TABLES.contains(&t.as_str()))
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

/// A value that reads as a timestamp, in to_jsonb's or JSON's format.
fn timestamp(s: &str) -> Option<NaiveDateTime> {
    let s = s.strip_suffix('Z').unwrap_or(s);
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
    if HEX32.is_match(s) {
        return "<hex>".into();
    }
    let s = if s.contains("data-discourse-entrypoint=") {
        let s = ASSET.replace_all(s, "$1.js");
        NONCE.replace_all(&s, "").into_owned()
    } else {
        s.to_string()
    };
    let s = KEY.replace_all(&s, "<key>");
    let s = UUID.replace_all(&s, "<uuid>");
    BOUNDARY.replace_all(&s, "boundary=<boundary>").into_owned()
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
    for row in inserted("email_logs") {
        if let Some(v) = row["bounce_key"].as_str() {
            secrets.push((v.to_string(), "<bounce_key>"));
            secrets.push((v.replace('-', ""), "<bounce_key>"));
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

/// Timestamps at or after the case's start become `<now>`, plugin keys
/// go.
fn normalize(value: &Value, started: NaiveDateTime) -> Value {
    match value {
        Value::String(s) => match timestamp(s) {
            Some(t) if t >= started => Value::String("<now>".into()),
            _ => Value::String(today(&unrandom(s), started.date())),
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
fn changed_columns(mut doc: Value) -> Value {
    let Some(Value::Object(tables)) = doc.get_mut("changes") else {
        return doc;
    };
    tables.retain(|t, _| !BACKGROUND_TABLES.contains(&t.as_str()));
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
async fn align_sequences(pool: &PgPool, case: &Value) {
    let Some(changes) = case["changes"].as_object() else {
        return;
    };
    for (table, change) in changes {
        let Some(first) = change["inserted"]
            .as_array()
            .and_then(|rows| rows.iter().filter_map(|r| r["id"].as_i64()).min())
        else {
            continue;
        };
        let sequence: Option<String> =
            sqlx::query_scalar("SELECT pg_get_serial_sequence($1, 'id')")
                .bind(format!("public.{table}"))
                .fetch_one(pool)
                .await
                .unwrap_or(None);
        if let Some(sequence) = sequence {
            sqlx::query("SELECT setval($1, GREATEST($2, 1), $2 > 0)")
                .bind(&sequence)
                .bind(first - 1)
                .execute(pool)
                .await
                .unwrap();
        }
    }
}

/// `{{path.to.value}}` with an optional `|reverse` or `|last_segment`.
fn resolve(value: &Value, state: &Value) -> Value {
    static REF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\{\{([^}|]+)(\|reverse|\|last_segment)?\}\}").unwrap()
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
    let rows: Vec<(i64, String, Value, f64, bool)> = sqlx::query_as(
        "SELECT id, name, args, EXTRACT(EPOCH FROM run_at - created_at)::float8, at_time \
         FROM discourse_rs.jobs WHERE id > $1 ORDER BY id",
    )
    .bind(after_id)
    .fetch_all(pool)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(id, name, args, delay, at_time)| {
            // " at" for Jobs.enqueue_at, as the recorder writes it.
            let shown = if at_time {
                format!("{name} at")
            } else if delay >= 1.0 {
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

async fn replay(case: &Value, run_jobs: &[String]) -> Vec<String> {
    let db = TestDb::new().await;
    // A public dir of the case's own, for the files uploads store.
    let public = std::env::temp_dir().join(format!(
        "discourse-rs-writes-{}-{}",
        std::process::id(),
        case["name"].as_str().unwrap()
    ));
    let mut config = recorded_config();
    config.public_dir = public.clone();
    let app_state = state(db.pool.clone(), config).await;
    reset_sequences(&db.pool).await;
    align_sequences(&db.pool, case).await;
    let mut client = Client {
        state: app_state,
        cookies: Vec::new(),
        csrf: None,
    };
    if let Some(user) = case["user"].as_str() {
        client.login(user).await;
    } else {
        let (_, csrf) = client.send(Method::GET, "/session/csrf.json", None).await;
        client.csrf = csrf["csrf"].as_str().map(str::to_string);
    }
    apply_settings(&db.pool, &client.state, case).await;
    // `setup`: the case's fixture rows, as the recorder ran them.
    for sql in case["setup"].as_array().into_iter().flatten() {
        sqlx::query(sql.as_str().unwrap())
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("setup {sql}: {e}"));
    }

    let tables = tables(&db.pool).await;
    let mut before_sums = HashMap::new();
    let mut before_rows = HashMap::new();
    for (table, _) in &tables {
        before_sums.insert(table.clone(), checksum(&db.pool, table).await);
        before_rows.insert(table.clone(), table_rows(&db.pool, table).await);
    }
    let started: NaiveDateTime = sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let mut responses = Vec::new();
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
        let path = resolve(&request["path"], &state);
        let extra = request["headers"].as_object().cloned().unwrap_or_default();
        let (status, body) = client
            .send_with(method, path.as_str().unwrap(), body, &extra)
            .await;
        responses.push(serde_json::json!({ "status": status, "body": body }));
    }
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
    let mut ours = serde_json::json!({
        "responses": responses,
        "jobs": from_requests.into_iter().map(|(_, _, j)| j).collect::<Vec<_>>(),
        "changes": changes(&db.pool, &tables, &before_sums, &before_rows).await,
    });
    if !run_jobs.is_empty() {
        ours["jobs_from_jobs"] = Value::Array(from_jobs);
        ours["emails"] = Value::Array(
            client
                .state
                .mailer
                .sent()
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "headers": m.headers.iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>(),
                        "text": m.text,
                        "html": m.html,
                    })
                })
                .collect(),
        );
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
            .filter(|j| !PLUGIN_JOBS.contains(&j[0].as_str().unwrap_or("")))
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
    // The reference runs in development, where a route that does not match
    // (an admin route for a non-admin) renders the Routing Error page; the
    // port renders the production 404. The status is still compared.
    for i in 0..rails["responses"].as_array().map_or(0, Vec::len) {
        let routing_error = rails["responses"][i]["status"] == 404
            && rails["responses"][i]["body"]
                .as_str()
                .is_some_and(|b| b.starts_with("Routing Error"));
        if routing_error && ours["responses"][i]["status"] == 404 {
            rails["responses"][i]["body"] = Value::String("<not found>".into());
            ours["responses"][i]["body"] = Value::String("<not found>".into());
        }
    }
    for doc in [&mut rails, &mut ours] {
        for response in doc["responses"].as_array_mut().into_iter().flatten() {
            if let Some(user) = response["body"]
                .as_object_mut()
                .and_then(|b| b.get_mut("user"))
            {
                undrift_user(user);
            }
            if let Some(details) = response["body"]
                .as_object_mut()
                .and_then(|b| b.get_mut("details"))
            {
                unorder_topic_details(details);
            }
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
        &changed_columns(normalize(
            &{
                hide_secrets(&mut rails);
                rails
            },
            rails_started,
        )),
        &changed_columns(normalize(
            &{
                hide_secrets(&mut ours);
                ours
            },
            started,
        )),
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
                    .block_on(replay(&case.recorded, &case.run_jobs));
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
