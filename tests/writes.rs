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

/// Keys plugins add to the post serializer on the reference.
const PLUGIN_KEYS: [&str; 9] = [
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

/// Cases the port does not do like Rails yet. The list only shrinks.
const NOT_YET: &[&str] = &[
    "reply",
    "reply_to_post",
    "reply_by_tl1",
    "reply_with_markdown",
    "new_topic",
    "reply_too_short",
    "reply_to_restricted_topic",
    "reply_to_closed_topic",
    "reply_anonymous",
    "edit",
    "edit_someone_elses_post",
    "edit_conflict",
    "edit_by_admin",
    "revision_not_found",
];

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
         WHERE s.relkind = 'S'",
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

/// Timestamps at or after the case's start become `<now>`, plugin keys
/// go.
fn normalize(value: &Value, started: NaiveDateTime) -> Value {
    match value {
        Value::String(s) => match timestamp(s) {
            Some(t) if t >= started => Value::String("<now>".into()),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(|v| normalize(v, started)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !PLUGIN_KEYS.contains(&k.as_str()))
                .map(|(k, v)| match (k.as_str(), v) {
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
async fn replay(case: &Value) -> Vec<String> {
    let db = TestDb::new().await;
    let app_state = state(db.pool.clone(), recorded_config());
    reset_sequences(&db.pool).await;
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
        let body = request.get("params").filter(|p| p.is_object());
        let (status, body) = client
            .send(method, request["path"].as_str().unwrap(), body)
            .await;
        responses.push(serde_json::json!({ "status": status, "body": body }));
    }
    let ours = serde_json::json!({
        "responses": responses,
        "changes": changes(&db.pool, &tables, &before_sums, &before_rows).await,
    });

    let rails_started = timestamp(case["started_at"].as_str().unwrap()).unwrap();
    let rails = serde_json::json!({ "responses": case["responses"], "changes": case["changes"] });
    let mut out = Vec::new();
    differences(
        "",
        &normalize(&rails, rails_started),
        &normalize(&ours, started),
        &mut out,
    );
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_match_rails() {
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
        let diff = replay(&recorded).await;
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
