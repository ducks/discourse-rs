//! A "helpful" mark on posts, drawn as a button in each post's menu: the
//! shape of a Rails plugin's `api.addPostMenuButton` with its controller
//! and `publish_change_to_clients!`, as server-rendered HTML for htmx.

use discourse_plugin::{Input, execute, publish_post_change, query, reply, t};
use serde_json::{Value, json};

discourse_plugin::phases! {
    "outlet.post_menu" => post_menu,
    "route.handle" => handle,
}

/// Each post's button, for the viewer: a toggle on another member's post,
/// the count alone on their own or for a reader who is not logged in
/// (nothing while no one has marked it).
fn post_menu(input: Input) -> Result<Value, String> {
    let request = &input.payload;
    let base = request["base_path"].as_str().unwrap_or("");
    let posts = request["posts"].as_array().cloned().unwrap_or_default();
    let ids: Vec<i64> = posts.iter().filter_map(|p| p["id"].as_i64()).collect();
    let viewer = input.guardian.user_id;
    let marks = marks(&ids, viewer)?;
    let mut out = serde_json::Map::new();
    for post in &posts {
        let Some(id) = post["id"].as_i64() else {
            continue;
        };
        if post["deleted"] == true {
            continue;
        }
        let (count, mine) = marks
            .iter()
            .find(|m| m.0 == id)
            .map_or((0, false), |m| (m.1, m.2));
        let toggles = viewer.is_some() && post["yours"] != true;
        if !toggles && count == 0 {
            continue;
        }
        out.insert(
            id.to_string(),
            json!(button(base, id, count, mine, toggles)),
        );
    }
    Ok(Value::Object(out))
}

/// POST /helpful/posts/:post_id: the viewer's mark on, or off again.
fn handle(input: Input) -> Result<Value, String> {
    let request = &input.payload;
    let Some(viewer) = input.guardian.user_id else {
        return Ok(reply(
            403,
            "text/plain; charset=utf-8",
            "log in to mark posts",
        ));
    };
    let post_id: i64 = request["params"]["post_id"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or("no post id")?;
    let post = query(
        "SELECT user_id FROM posts WHERE id = $1 AND deleted_at IS NULL",
        &[json!(post_id)],
    )?;
    let Some(post) = post.first() else {
        return Ok(reply(404, "text/plain; charset=utf-8", "no such post"));
    };
    if post["user_id"].as_i64() == Some(viewer) {
        return Ok(reply(403, "text/plain; charset=utf-8", "not your own post"));
    }
    let removed = execute(
        "DELETE FROM marks WHERE post_id = $1 AND user_id = $2",
        &[json!(post_id), json!(viewer)],
    )?;
    if removed == 0 {
        execute(
            "INSERT INTO marks (post_id, user_id) VALUES ($1, $2)",
            &[json!(post_id), json!(viewer)],
        )?;
    }
    publish_post_change(post_id);
    let (count, mine) = marks(&[post_id], Some(viewer))?
        .first()
        .map_or((0, false), |m| (m.1, m.2));
    let base = request["base_path"].as_str().unwrap_or("");
    Ok(reply(
        200,
        "text/html; charset=utf-8",
        button(base, post_id, count, mine, true),
    ))
}

/// Each post's marks counted, and whether the viewer's is among them.
fn marks(ids: &[i64], viewer: Option<i64>) -> Result<Vec<(i64, i64, bool)>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = query(
        "SELECT post_id, count(*) AS n, coalesce(bool_or(user_id = $2), false) AS mine \
         FROM marks WHERE post_id IN (SELECT jsonb_array_elements_text($1::jsonb)::int) \
         GROUP BY post_id",
        &[json!(ids), json!(viewer)],
    )?;
    Ok(rows
        .iter()
        .map(|r| {
            (
                r["post_id"].as_i64().unwrap_or(0),
                r["n"].as_i64().unwrap_or(0),
                r["mine"] == true,
            )
        })
        .collect())
}

fn button(base: &str, post_id: i64, count: i64, mine: bool, toggles: bool) -> String {
    let icon = "<svg class=\"fa d-icon d-icon-thumbs-up svg-icon fa-width-auto svg-string\" width=\"1em\" height=\"1em\" aria-hidden=\"true\" xmlns=\"http://www.w3.org/2000/svg\"><use href=\"#thumbs-up\"></use></svg>";
    let label = if count > 0 {
        format!("<span class=\"d-button-label\">{count}</span>")
    } else {
        String::new()
    };
    let marked = if mine { " --marked" } else { "" };
    if !toggles {
        return format!(
            "<button class=\"btn btn-icon-text btn-flat post-action-menu__helpful{marked}\" type=\"button\" disabled title=\"{}\">{icon}{label}</button>",
            escape(&t("helpful.count"))
        );
    }
    let title = t(if mine {
        "helpful.unmark"
    } else {
        "helpful.mark"
    });
    format!(
        "<button class=\"btn btn-icon-text btn-flat post-action-menu__helpful{marked}\" type=\"button\" title=\"{}\" aria-pressed=\"{mine}\" hx-post=\"{}/helpful/posts/{post_id}\" hx-swap=\"outerHTML\">{icon}{label}</button>",
        escape(&title),
        escape(base)
    )
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
