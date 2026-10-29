//! The guest side of discourse-rs' WebAssembly plugin ABI
//! (docs/plugin-wasm.md): JSON over linear memory.
//!
//! A plugin is a `cdylib` built for `wasm32-unknown-unknown` that names
//! its phases with [`phases!`]:
//!
//! ```ignore
//! fn after_serialize(input: discourse_plugin::Input) -> Result<serde_json::Value, String> { ... }
//! discourse_plugin::phases! { "topic_list.after_serialize" => after_serialize }
//! ```
//!
//! and reaches the host through the functions here, each a synchronous
//! call into the host that the host answers before the guest continues.

use serde::Deserialize;
use serde_json::{Value, json};

/// A phase's input: the payload its pipeline defines, and who is asking.
#[derive(Debug, Deserialize)]
pub struct Input {
    pub payload: Value,
    pub guardian: Guardian,
}

#[derive(Debug, Deserialize)]
pub struct Guardian {
    pub user_id: Option<i64>,
    pub staff: bool,
    pub admin: bool,
}

#[link(wasm_import_module = "discourse")]
unsafe extern "C" {
    #[link_name = "rows"]
    fn host_rows(ptr: *const u8, len: usize) -> u64;
    #[link_name = "core_rows"]
    fn host_core_rows(ptr: *const u8, len: usize) -> u64;
    #[link_name = "setting"]
    fn host_setting(ptr: *const u8, len: usize) -> u64;
    #[link_name = "t"]
    fn host_t(ptr: *const u8, len: usize) -> u64;
    #[link_name = "query"]
    fn host_query(ptr: *const u8, len: usize) -> u64;
    #[link_name = "execute"]
    fn host_execute(ptr: *const u8, len: usize) -> u64;
    #[link_name = "users"]
    fn host_users(ptr: *const u8, len: usize) -> u64;
    #[link_name = "can_see_category"]
    fn host_can_see_category(ptr: *const u8, len: usize) -> u64;
    #[link_name = "publish_post_change"]
    fn host_publish_post_change(ptr: *const u8, len: usize) -> u64;
}

/// Memory for `len` bytes the host writes into: a phase's input or a host
/// call's answer, handed back with [`take`].
pub fn alloc(len: u32) -> u32 {
    let buf = vec![0u8; len as usize].into_boxed_slice();
    Box::into_raw(buf) as *mut u8 as u32
}

/// The bytes at a packed `ptr << 32 | len` that [`alloc`] made.
fn take(packed: u64) -> Vec<u8> {
    let (ptr, len) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
    // SAFETY: the host wrote `len` bytes into memory `alloc(len)` returned.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len)) }.into_vec()
}

/// Leaks `bytes` and packs where they are, for the host to read. The
/// instance is dropped after the call, its memory with it.
fn give(bytes: Vec<u8>) -> u64 {
    let len = bytes.len() as u64;
    let ptr = Box::into_raw(bytes.into_boxed_slice()) as *mut u8 as usize as u64;
    (ptr << 32) | len
}

/// A phase export's body: decode, run, encode `{"ok"}` or `{"error"}`.
pub fn run(ptr: u32, len: u32, phase: fn(Input) -> Result<Value, String>) -> u64 {
    let bytes = take((u64::from(ptr) << 32) | u64::from(len));
    let result = serde_json::from_slice::<Input>(&bytes)
        .map_err(|e| format!("input: {e}"))
        .and_then(phase);
    let out = match result {
        Ok(ok) => json!({ "ok": ok }),
        Err(error) => json!({ "error": error }),
    };
    give(serde_json::to_vec(&out).unwrap_or_default())
}

fn call(f: unsafe extern "C" fn(*const u8, usize) -> u64, request: &[u8]) -> Vec<u8> {
    // SAFETY: the host reads `request` and answers with memory from alloc.
    take(unsafe { f(request.as_ptr(), request.len()) })
}

fn rows_from(
    f: unsafe extern "C" fn(*const u8, usize) -> u64,
    table: &str,
    column: &str,
    ids: &[i64],
) -> Vec<Value> {
    if ids.is_empty() {
        return Vec::new();
    }
    let request = json!({ "table": table, "column": column, "ids": ids });
    let answer = call(f, request.to_string().as_bytes());
    serde_json::from_slice(&answer).unwrap_or_default()
}

/// The rows of a table this plugin owns whose `column` is one of `ids`,
/// with the columns its manifest lists, ordered by `column` then `id`.
pub fn rows(table: &str, column: &str, ids: &[i64]) -> Vec<Value> {
    rows_from(host_rows, table, column, ids)
}

/// The same for a core table, limited to the columns the manifest's
/// `[core_reads]` lists.
pub fn core_rows(table: &str, column: &str, ids: &[i64]) -> Vec<Value> {
    rows_from(host_core_rows, table, column, ids)
}

/// A site setting the manifest lists.
pub fn setting(name: &str) -> Value {
    serde_json::from_slice(&call(host_setting, name.as_bytes())).unwrap_or(Value::Null)
}

/// A server translation (the key itself when there is none).
pub fn t(key: &str) -> String {
    String::from_utf8(call(host_t, key.as_bytes())).unwrap_or_else(|_| key.to_string())
}

/// The plugin's exports: `dp_alloc` and one function per phase, named
/// `<pipeline>.<phase>`.
#[macro_export]
macro_rules! phases {
    ($($name:literal => $f:path),* $(,)?) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn dp_alloc(len: u32) -> u32 {
            $crate::alloc(len)
        }
        $(
            const _: () = {
                #[unsafe(export_name = $name)]
                pub extern "C" fn phase(ptr: u32, len: u32) -> u64 {
                    $crate::run(ptr, len, $f)
                }
            };
        )*
    };
}

/// `route.handle`'s answer: a JSON document.
pub fn json_reply(status: u16, body: &Value) -> Value {
    json!({
        "status": status,
        "content_type": "application/json; charset=utf-8",
        "body": body.to_string(),
    })
}

/// `route.handle`'s answer: text of a content type (`text/html;
/// charset=utf-8` for a page).
pub fn reply(status: u16, content_type: &str, body: impl Into<String>) -> Value {
    json!({ "status": status, "content_type": content_type, "body": body.into() })
}

fn sql(
    f: unsafe extern "C" fn(*const u8, usize) -> u64,
    sql: &str,
    params: &[Value],
) -> Result<Value, String> {
    let request = json!({ "sql": sql, "params": params });
    let answer: Value = serde_json::from_slice(&call(f, request.to_string().as_bytes()))
        .map_err(|e| format!("answer: {e}"))?;
    match answer.get("error") {
        Some(error) => Err(error.as_str().unwrap_or("error").to_string()),
        None => Ok(answer["ok"].clone()),
    }
}

/// Rows of a statement on the plugin's own connection (a SELECT, or a
/// write with RETURNING), as objects. Parameters bind by their JSON type
/// (integers as bigint, strings as text, null untyped); cast the rest in
/// the statement (`$1::date`). The plugin's schema is on the search path;
/// core tables are `public.<table>`, readable where `[core_reads]` grants.
/// A statement's error comes back here; after one, the hook's transaction
/// is aborted and later statements fail too.
pub fn query(statement: &str, params: &[Value]) -> Result<Vec<Value>, String> {
    let rows = sql(host_query, statement, params)?;
    Ok(rows.as_array().cloned().unwrap_or_default())
}

/// A statement's rows affected. Only in routes (not under GET or HEAD,
/// whose transaction is read only) and jobs.
pub fn execute(statement: &str, params: &[Value]) -> Result<u64, String> {
    let answer = sql(host_execute, statement, params)?;
    Ok(answer["rows_affected"].as_u64().unwrap_or(0))
}

/// `application/x-www-form-urlencoded` pairs (a form's body, a query).
pub fn form(encoded: &str) -> Vec<(String, String)> {
    fn hex(b: u8) -> Option<u8> {
        (b as char).to_digit(16).map(|d| d as u8)
    }
    fn decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi << 4 | lo);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                },
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    encoded
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(k), decode(v))
        })
        .collect()
}

/// A translation in `locale` (None: the site's default), with `%{name}`
/// interpolated from `args`; the key itself when there is none.
pub fn t_with(key: &str, locale: Option<&str>, args: &[(&str, &str)]) -> String {
    let args: serde_json::Map<String, Value> = args
        .iter()
        .map(|(k, v)| (k.to_string(), Value::from(*v)))
        .collect();
    let request = json!({ "key": key, "locale": locale, "args": args });
    String::from_utf8(call(host_t, request.to_string().as_bytes()))
        .unwrap_or_else(|_| key.to_string())
}

/// Users' public cards (id, username, name, avatar_template), by id.
pub fn users(ids: &[i64]) -> Vec<Value> {
    if ids.is_empty() {
        return Vec::new();
    }
    let answer = call(host_users, json!(ids).to_string().as_bytes());
    serde_json::from_slice(&answer).unwrap_or_default()
}

/// Whether the viewer may see a category (`can_see_category?`).
pub fn can_see_category(id: i64) -> bool {
    let answer = call(host_can_see_category, id.to_string().as_bytes());
    serde_json::from_slice(&answer).unwrap_or(false)
}

/// `post.publish_change_to_clients!(:acted)`: once the hook commits, every
/// page showing the post redraws it for its viewer, this plugin's
/// `outlet.post_menu` included. Routes and jobs only.
pub fn publish_post_change(post_id: i64) {
    call(host_publish_post_change, post_id.to_string().as_bytes());
}

/// Rails' params from a route's input: the query and a form body, with
/// `a[b][c]=v` nested into objects and `a[]=v` into arrays; a later
/// scalar wins.
pub fn params(request: &Value) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    let query = request["query"].as_str().unwrap_or("");
    let form_body = request["content_type"]
        .as_str()
        .is_some_and(|c| c.starts_with("application/x-www-form-urlencoded"));
    let body = if form_body {
        request["body"].as_str().unwrap_or("")
    } else {
        ""
    };
    for (key, value) in form(query).into_iter().chain(form(body)) {
        let (head, rest) = match key.find('[') {
            Some(i) => (&key[..i], &key[i..]),
            None => (key.as_str(), ""),
        };
        let mut path: Vec<&str> = vec![head];
        let mut rest = rest;
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            path.push(&stripped[..end]);
            rest = &stripped[end + 1..];
        }
        insert(&mut out, &path, Value::String(value));
    }
    out
}

fn insert(map: &mut serde_json::Map<String, Value>, path: &[&str], value: Value) {
    let Some((first, rest)) = path.split_first() else {
        return;
    };
    match rest.first() {
        None => {
            map.insert(first.to_string(), value);
        }
        Some(&"") => {
            let entry = map.entry(first.to_string()).or_insert_with(|| json!([]));
            if !entry.is_array() {
                *entry = json!([]);
            }
            if let Some(list) = entry.as_array_mut() {
                list.push(value);
            }
        }
        Some(_) => {
            let entry = map.entry(first.to_string()).or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            if let Some(inner) = entry.as_object_mut() {
                insert(inner, rest, value);
            }
        }
    }
}
