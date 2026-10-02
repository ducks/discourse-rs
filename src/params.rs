//! Rails' `params` for a request: the query string and the body merged,
//! a form body read with Rack's nested-key rules (`post[raw]`, `tags[]`),
//! a JSON body as its object. Form values are strings and JSON values keep
//! their types, as in Rails; readers go through `string` and `integer`.

use axum::http::{HeaderMap, header};
use serde_json::{Map, Value};

/// `Rack::Utils.normalize_params` for one key and value.
fn insert(target: &mut Map<String, Value>, key: &str, value: Value) {
    // `name[rest]`: the name, then what follows it.
    let (name, rest) = match key.find('[') {
        Some(0) | None => (key, ""),
        Some(i) => (&key[..i], &key[i..]),
    };
    if name.is_empty() {
        return;
    }
    if rest.is_empty() {
        target.insert(name.to_string(), value);
        return;
    }
    if rest == "[]" {
        let list = target
            .entry(name.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(items) = list {
            items.push(value);
        }
        return;
    }
    // `[child]rest`
    let Some(close) = rest.find(']') else {
        target.insert(key.to_string(), value);
        return;
    };
    let child = &rest[1..close];
    let after = &rest[close + 1..];
    let nested = target
        .entry(name.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !nested.is_object() {
        *nested = Value::Object(Map::new());
    }
    if let Value::Object(map) = nested {
        insert(map, &format!("{child}{after}"), value);
    }
}

/// `parse_nested_query`
pub fn parse_query(query: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        insert(&mut out, &k, Value::String(v.into_owned()));
    }
    out
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"))
}

/// The request's params. A JSON body that is not an object adds nothing,
/// as Rails puts it under `_json`.
pub fn parse(query: Option<&str>, headers: &HeaderMap, body: &[u8]) -> Map<String, Value> {
    let mut out = query.map(parse_query).unwrap_or_default();
    if is_json(headers) {
        if let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(body) {
            out.extend(map);
        }
    } else if !body.is_empty() {
        let form = std::str::from_utf8(body).unwrap_or_default();
        for (k, v) in parse_query(form) {
            out.insert(k, v);
        }
    }
    out
}

/// A param as Rails' `to_s` sees it: strings as they are, numbers and
/// booleans written out, nothing for null, arrays and objects.
pub fn string(params: &Map<String, Value>, key: &str) -> Option<String> {
    scalar(params.get(key)?)
}

pub fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `params[key].to_i`
pub fn integer(params: &Map<String, Value>, key: &str) -> Option<i64> {
    string(params, key).map(|s| crate::ruby::to_i(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_keys_like_rack() {
        let p = parse_query("post[raw]=hi&post[edit_reason]=why&tags[]=a&tags[]=b&topic_id=3");
        assert_eq!(p["post"]["raw"], "hi");
        assert_eq!(p["post"]["edit_reason"], "why");
        assert_eq!(p["tags"], serde_json::json!(["a", "b"]));
        assert_eq!(integer(&p, "topic_id"), Some(3));
    }
}
