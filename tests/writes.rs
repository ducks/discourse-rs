//! Write requests replayed against Rails' recording (parity/writes, written
//! by scripts/record-writes): each case runs on a fresh copy of the seed,
//! and its responses and every row it inserted, updated or deleted must be
//! what Discourse did.
//!
//! Timestamps the case itself wrote (at or after its start) compare as
//! `<now>`. Ids line up because both sides set every sequence to its
//! table's max(id) first. The jobs Rails enqueued are not compared: the
//! port has no job runner yet.

mod common;

use std::collections::{BTreeMap, HashMap};

use axum::body::Body;
use axum::http::{Method, Request, header};
use chrono::NaiveDateTime;
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

/// Tables Rails fills from a query without ORDER BY (sidebar links from
/// `Category.where(id:).pluck(:id)`), so the rows' ids follow each
/// database's heap order: compared as a set, without ids.
const UNORDERED_INSERTS: [&str; 1] = ["sidebar_section_links"];

/// Keys plugins add to the post serializer on the reference.
const PLUGIN_KEYS: [&str; 11] = [
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
];

/// Jobs plugins enqueue on the reference (discourse-narrative-bot).
const PLUGIN_JOBS: [&str; 1] = ["bot_input"];

/// Cases the port does not do like Rails yet. The list only shrinks.
const NOT_YET: &[&str] = &[];

struct Client {
    state: AppState,
    cookies: Vec<(String, String)>,
    csrf: Option<String>,
}

impl Client {
    async fn send(&mut self, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
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
        if let Some(token) = &self.csrf {
            request = request.header("x-csrf-token", token.as_str());
        }
        let body = match body {
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
        let (_, csrf) = self.send(Method::GET, "/session/csrf.json", None).await;
        self.csrf = csrf["csrf"].as_str().map(str::to_string);
        let body = serde_json::json!({ "login": username, "password": "password" });
        let (status, reply) = self.send(Method::POST, "/session.json", Some(&body)).await;
        assert_eq!(status, 200, "login as {username}: {reply}");
    }
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
    if HEX32.is_match(s) {
        return "<hex>".into();
    }
    let s = KEY.replace_all(s, "<key>");
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
            _ => Value::String(unrandom(s)),
        },
        Value::Array(items) => Value::Array(items.iter().map(|v| normalize(v, started)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !PLUGIN_KEYS.contains(&k.as_str()))
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
                    rows.sort_by_key(identity);
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
            let changed: Map<String, Value> = after
                .iter()
                .filter(|(k, v)| before.get(*k) != Some(*v))
                .map(|(k, v)| (k.clone(), serde_json::json!([before.get(k), v])))
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
    let rows: Vec<(i64, String, Value, f64)> = sqlx::query_as(
        "SELECT id, name, args, EXTRACT(EPOCH FROM run_at - created_at)::float8 \
         FROM discourse_rs.jobs WHERE id > $1 ORDER BY id",
    )
    .bind(after_id)
    .fetch_all(pool)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(id, name, args, delay)| {
            let shown = if delay >= 1.0 {
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
    let app_state = state(db.pool.clone(), recorded_config());
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
        let (status, body) = client.send(method, path.as_str().unwrap(), body).await;
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

#[tokio::test(flavor = "multi_thread")]
async fn writes_match_rails() {
    // A 500's cause is only logged; show it next to the diff.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .with_test_writer()
        .try_init();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("parity/writes");
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let only = std::env::var("WRITES_ONLY").ok();
    let mut failing = Vec::new();
    let mut report = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        if only.as_deref().is_some_and(|o| !name.contains(o)) {
            continue;
        }
        let recorded: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap(),
        )
        .unwrap();
        let run_jobs: Vec<String> = case["run_jobs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let diff = replay(&recorded, &run_jobs).await;
        if !diff.is_empty() {
            failing.push(name.to_string());
            if !NOT_YET.contains(&name) || std::env::var("WRITES_DIFF").is_ok() {
                report.push(format!("{name}\n  {}", diff.join("\n  ")));
            }
        }
    }
    eprintln!(
        "{} of {} write cases match Rails",
        cases.len() - failing.len(),
        cases.len()
    );
    if std::env::var("WRITES_DIFF").is_ok() {
        eprintln!("{}", report.join("\n"));
    }
    if only.is_none() {
        assert_eq!(
            failing,
            NOT_YET,
            "the cases that differ from Rails changed:\n{}",
            report.join("\n")
        );
    }
}
