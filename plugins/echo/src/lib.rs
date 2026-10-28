//! tests/plugin_api.rs' echo plugin as a WebAssembly guest: its routes,
//! jobs, keys on the current user and the topic view, its outlet above a
//! topic's posts and its homepage, each one export (plugin.toml declares
//! them; settings.yml, locales/ and public/ are files beside it).

use discourse_plugin::{Input, execute, form, json_reply, query, reply, setting, t};
use serde_json::{Value, json};

discourse_plugin::phases! {
    "route.handle" => handle,
    "job.run" => run_job,
    "serializer.current_user" => current_user,
    "serializer.topic_view" => topic_view,
    "outlet.topic_above_posts" => topic_above_posts,
    "homepage.available" => available,
}

fn handle(input: Input) -> Result<Value, String> {
    let request = &input.payload;
    Ok(match request["route"].as_str().unwrap_or("") {
        "/echo.json" => json_reply(
            200,
            &json!({ "greeting": setting("echo_greeting"), "hello": t("echo.hello") }),
        ),
        // The host checked the session's token before the call.
        "/echo/write" => reply(200, "text/plain; charset=utf-8", ""),
        "/echo/token" => reply(
            200,
            "text/plain; charset=utf-8",
            request["csrf_token"].as_str().unwrap_or(""),
        ),
        "/echo" => reply(
            200,
            "text/plain; charset=utf-8",
            format!("echo home {}", request["query"].as_str().unwrap_or("")),
        ),
        "/echo/echoes" => {
            let fields = form(request["body"].as_str().unwrap_or(""));
            let body = field(&fields, "body").ok_or("no body")?;
            let rows = query(
                "INSERT INTO echoes (body, user_id) VALUES ($1, $2) RETURNING id",
                &[json!(body), json!(input.guardian.user_id)],
            )?;
            // A route that fails after writing: its write is rolled back.
            if body == "fail" {
                return Err("told to fail after writing".into());
            }
            json_reply(200, &json!({ "id": rows[0]["id"] }))
        }
        "/echo/echoes.json" => json_reply(
            200,
            &json!(query(
                "SELECT id, body, user_id FROM echoes ORDER BY id",
                &[]
            )?),
        ),
        "/echo/probe" => {
            let source = match request["method"].as_str() {
                Some("GET") => request["query"].as_str().unwrap_or(""),
                _ => request["body"].as_str().unwrap_or(""),
            };
            let results: Vec<Value> = form(source)
                .into_iter()
                .filter(|(k, _)| k == "sql")
                .map(|(_, sql)| {
                    let result = match sql.strip_prefix('!') {
                        Some(sql) => execute(sql, &[]).map(Value::from),
                        None => query(&sql, &[]).map(Value::from),
                    };
                    match result {
                        Ok(ok) => json!({ "ok": ok }),
                        Err(error) => json!({ "error": error }),
                    }
                })
                .collect();
            json_reply(200, &json!(results))
        }
        other => return Err(format!("no route {other}")),
    })
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// A regular job fails when its args ask it to, so its outcome shows in
/// the queue.
fn run_job(input: Input) -> Result<Value, String> {
    let job = &input.payload;
    match job["name"].as_str() {
        Some("echo_scheduled") => Ok(Value::Null),
        Some("echo_job") if job["args"]["fail"] == true => {
            Err(format!("echo_job told to fail: {}", job["args"]))
        }
        Some("echo_job") => {
            if let Some(body) = job["args"]["body"].as_str() {
                execute("INSERT INTO echoes (body) VALUES ($1)", &[json!(body)])?;
            }
            Ok(Value::Null)
        }
        other => Err(format!("no job {other:?}")),
    }
}

fn current_user(input: Input) -> Result<Value, String> {
    Ok(json!({ "echo_user": input.guardian.user_id }))
}

fn topic_view(input: Input) -> Result<Value, String> {
    let id = input.payload["topic_id"]
        .as_i64()
        .ok_or("topic_view without a topic_id")?;
    // Through the column grant: public.topics (id, title) only.
    let topic = query(
        "SELECT title FROM public.topics WHERE id = $1",
        &[json!(id)],
    )?
    .into_iter()
    .next()
    .ok_or("no such topic")?;
    Ok(json!({ "echo_topic": { "id": id, "title": topic["title"] } }))
}

fn topic_above_posts(input: Input) -> Result<Value, String> {
    let id = &input.payload["view"]["echo_topic"]["id"];
    Ok(json!(format!("<div class=\"echo-card\">{id}</div>")))
}

/// Closed while the greeting is "closed".
fn available(_: Input) -> Result<Value, String> {
    Ok(json!(setting("echo_greeting") != json!("closed")))
}
