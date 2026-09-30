//! Response parity against real Discourse.
//!
//! A case is a request (`parity/cases`). Rails responses are recorded as
//! golden files (`parity/golden/*.json`) and compared against discourse-rs,
//! either live over HTTP (`parity` binary) or in-process (tests/parity.rs).
//!
//! Comparison covers status, media type, and body. JSON bodies are compared
//! structurally (key order ignored) after removing each case's `ignore`
//! pointers, for fields that legitimately differ between runs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use similar::TextDiff;

/// Headers sent with every case, to Rails and discourse-rs alike. Matches
/// what the Ember app's ajax calls send, since that is the client to satisfy.
pub const REQUEST_HEADERS: &[(&str, &str)] = &[
    ("Accept", "application/json, text/javascript, */*; q=0.01"),
    ("X-Requested-With", "XMLHttpRequest"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub method: String,
    pub path: String,
    /// RFC 6901 JSON pointers to drop before comparing; `*` matches every
    /// array element or object key at that level.
    pub ignore: Vec<String>,
}

impl Case {
    pub fn label(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    /// File name under the golden dir, e.g. `GET /srv/status?cluster=x`
    /// becomes `get_srv_status_cluster_x.json`.
    pub fn golden_name(&self) -> String {
        let raw = format!("{}_{}", self.method, self.path).to_ascii_lowercase();
        let mut slug = String::with_capacity(raw.len());
        for c in raw.chars() {
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.ends_with('_') {
                slug.push('_');
            }
        }
        format!("{}.json", slug.trim_end_matches('_'))
    }
}

/// Parses the cases file: one `METHOD /path [ignore=/ptr,/ptr]` per line,
/// blank lines and `#` comments skipped.
pub fn parse_cases(src: &str) -> Result<Vec<Case>, String> {
    let mut cases: Vec<Case> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let lineno = i + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts
            .next()
            .ok_or(format!("line {lineno}: expected `METHOD /path`"))?
            .to_string();

        // Request bodies and auth arrive with the first write slice.
        if method != "GET" {
            return Err(format!(
                "line {lineno}: only GET is supported, got {method}"
            ));
        }
        if !path.starts_with('/') {
            return Err(format!("line {lineno}: path must start with /"));
        }

        let mut ignore = Vec::new();
        for opt in parts {
            let ptrs = opt
                .strip_prefix("ignore=")
                .ok_or(format!("line {lineno}: unknown option {opt:?}"))?;
            for ptr in ptrs.split(',') {
                if !ptr.starts_with('/') {
                    return Err(format!(
                        "line {lineno}: ignore pointer {ptr:?} must start with /"
                    ));
                }
                ignore.push(ptr.to_string());
            }
        }

        let case = Case {
            method,
            path,
            ignore,
        };
        if let Some(dup) = cases.iter().find(|c| c.golden_name() == case.golden_name()) {
            return Err(format!(
                "line {lineno}: {} collides with {} (golden file {})",
                case.label(),
                dup.label(),
                case.golden_name()
            ));
        }
        cases.push(case);
    }
    Ok(cases)
}

/// Parses parity/environment: `KEY=VALUE` lines describing the process env
/// of the Discourse the golden files came from.
pub fn parse_environment(src: &str) -> Result<Vec<(String, String)>, String> {
    src.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'))
        .map(|(lineno, l)| {
            l.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .ok_or(format!("line {lineno}: expected KEY=VALUE"))
        })
        .collect()
}

pub fn load_environment(path: &Path) -> Result<Vec<(String, String)>, String> {
    let src = fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    parse_environment(&src).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_cases(path: &Path) -> Result<Vec<Case>, String> {
    let src = fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    parse_cases(&src).map_err(|e| format!("{}: {e}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recorded {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Golden {
    pub request: String,
    /// Where the response came from: the Rails URL it was recorded against,
    /// or a note if it was written by hand.
    pub source: String,
    #[serde(flatten)]
    pub response: Recorded,
}

fn golden_path(dir: &Path, case: &Case) -> PathBuf {
    dir.join(case.golden_name())
}

pub fn read_golden(dir: &Path, case: &Case) -> Result<Option<Golden>, String> {
    let path = golden_path(dir, case);
    match fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| format!("parsing {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

pub fn write_golden(dir: &Path, case: &Case, golden: &Golden) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = golden_path(dir, case);
    let mut json = serde_json::to_string_pretty(golden).map_err(|e| e.to_string())?;
    json.push('\n');
    fs::write(&path, json).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(path)
}

/// Compares `actual` against `expected`. On mismatch returns a report with
/// one section per differing aspect, labelled with `names` (expected, actual).
pub fn compare(
    case: &Case,
    expected: &Recorded,
    actual: &Recorded,
    names: (&str, &str),
) -> Result<(), String> {
    let mut problems = Vec::new();

    if expected.status != actual.status {
        problems.push(format!(
            "status: {} {} vs {} {}",
            names.0, expected.status, names.1, actual.status
        ));
    }

    let expected_type = media_type(expected.content_type.as_deref());
    let actual_type = media_type(actual.content_type.as_deref());
    if expected_type != actual_type {
        problems.push(format!(
            "content-type: {} {:?} vs {} {:?}",
            names.0, expected_type, names.1, actual_type
        ));
    }

    let body_diff = if expected_type.as_deref().is_some_and(is_json) {
        match (
            normalize_json(&expected.body, &case.ignore),
            normalize_json(&actual.body, &case.ignore),
        ) {
            (Ok(e), Ok(a)) if e == a => None,
            (Ok(e), Ok(a)) => Some(diff(&pretty(&e), &pretty(&a), names)),
            (Err(e), _) => Some(format!("{} body is not JSON: {e}", names.0)),
            (_, Err(e)) => Some(format!("{} body is not JSON: {e}", names.1)),
        }
    } else if expected.body != actual.body {
        Some(diff(&expected.body, &actual.body, names))
    } else {
        None
    };
    if let Some(d) = body_diff {
        problems.push(format!("body:\n{d}"));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// `text/plain; charset=utf-8` -> `text/plain`. Charset is not compared:
/// Rails always appends one, axum does not for JSON, and clients ignore it.
fn media_type(content_type: Option<&str>) -> Option<String> {
    content_type.map(|ct| {
        ct.split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    })
}

fn is_json(media: &str) -> bool {
    media == "application/json" || media.ends_with("+json")
}

fn normalize_json(body: &str, ignore: &[String]) -> Result<Value, String> {
    let mut value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    for ptr in ignore {
        let segments: Vec<String> = ptr
            .split('/')
            .skip(1)
            .map(|s| s.replace("~1", "/").replace("~0", "~"))
            .collect();
        remove_path(&mut value, &segments);
    }
    Ok(value)
}

fn remove_path(value: &mut Value, segments: &[String]) {
    let Some((head, rest)) = segments.split_first() else {
        return;
    };
    let last = rest.is_empty();

    match value {
        Value::Object(map) => {
            if head == "*" {
                if last {
                    map.clear();
                } else {
                    map.values_mut().for_each(|v| remove_path(v, rest));
                }
            } else if last {
                map.remove(head);
            } else if let Some(v) = map.get_mut(head) {
                remove_path(v, rest);
            }
        }
        Value::Array(items) => {
            if head == "*" {
                if last {
                    items.clear();
                } else {
                    items.iter_mut().for_each(|v| remove_path(v, rest));
                }
            } else if let Ok(i) = head.parse::<usize>() {
                if last {
                    if i < items.len() {
                        items.remove(i);
                    }
                } else if let Some(v) = items.get_mut(i) {
                    remove_path(v, rest);
                }
            }
        }
        _ => {}
    }
}

/// serde_json's default Map is a BTreeMap, so keys print sorted and the diff
/// only shows real differences.
fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

fn diff(expected: &str, actual: &str, names: (&str, &str)) -> String {
    TextDiff::from_lines(expected, actual)
        .unified_diff()
        .context_radius(3)
        .header(names.0, names.1)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(ignore: &[&str]) -> Case {
        Case {
            method: "GET".into(),
            path: "/x".into(),
            ignore: ignore.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn json(body: &str) -> Recorded {
        Recorded {
            status: 200,
            content_type: Some("application/json; charset=utf-8".into()),
            body: body.into(),
        }
    }

    const NAMES: (&str, &str) = ("rails", "rs");

    #[test]
    fn parses_cases_with_comments_and_ignores() {
        let cases = parse_cases(
            "# comment\n\nGET /srv/status\nGET /latest.json ignore=/topic_list/topics/*/bumped_at,/x\n",
        )
        .unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].path, "/srv/status");
        assert!(cases[0].ignore.is_empty());
        assert_eq!(
            cases[1].ignore,
            vec!["/topic_list/topics/*/bumped_at", "/x"]
        );
    }

    #[test]
    fn rejects_unsupported_methods_bad_paths_and_options() {
        assert!(parse_cases("POST /posts").unwrap_err().contains("only GET"));
        assert!(
            parse_cases("GET srv")
                .unwrap_err()
                .contains("must start with /")
        );
        assert!(
            parse_cases("GET /x foo=1")
                .unwrap_err()
                .contains("unknown option")
        );
        assert!(
            parse_cases("GET /x ignore=a")
                .unwrap_err()
                .contains("pointer")
        );
    }

    #[test]
    fn rejects_cases_that_share_a_golden_file() {
        let err = parse_cases("GET /a/b\nGET /a_b").unwrap_err();
        assert!(err.contains("collides"), "{err}");
    }

    #[test]
    fn golden_names_are_filesystem_safe() {
        let c = Case {
            method: "GET".into(),
            path: "/srv/status?cluster=a.b".into(),
            ignore: vec![],
        };
        assert_eq!(c.golden_name(), "get_srv_status_cluster_a_b.json");
    }

    #[test]
    fn json_compares_structurally() {
        let a = json(r#"{"a":1,"b":[1,2]}"#);
        let b = json(r#"{ "b": [1, 2], "a": 1 }"#);
        assert!(compare(&case(&[]), &a, &b, NAMES).is_ok());
    }

    #[test]
    fn json_mismatch_shows_a_diff() {
        let err = compare(&case(&[]), &json(r#"{"a":1}"#), &json(r#"{"a":2}"#), NAMES).unwrap_err();
        assert!(err.contains("-  \"a\": 1"), "{err}");
        assert!(err.contains("+  \"a\": 2"), "{err}");
    }

    #[test]
    fn ignore_pointers_drop_fields_including_wildcards() {
        let a = json(r#"{"list":[{"id":1,"at":"x"},{"id":2,"at":"y"}],"now":1}"#);
        let b = json(r#"{"list":[{"id":1,"at":"z"},{"id":2,"at":"w"}],"now":2}"#);
        assert!(compare(&case(&[]), &a, &b, NAMES).is_err());
        assert!(compare(&case(&["/list/*/at", "/now"]), &a, &b, NAMES).is_ok());
    }

    #[test]
    fn ignore_pointer_unescapes_rfc6901() {
        let a = json(r#"{"a/b":1,"c~d":1,"k":0}"#);
        let b = json(r#"{"a/b":2,"c~d":2,"k":0}"#);
        assert!(compare(&case(&["/a~1b", "/c~0d"]), &a, &b, NAMES).is_ok());
    }

    #[test]
    fn charset_is_not_compared_but_media_type_is() {
        let mut a = json("{}");
        let mut b = json("{}");
        b.content_type = Some("application/json".into());
        assert!(compare(&case(&[]), &a, &b, NAMES).is_ok());

        a.content_type = Some("text/html; charset=utf-8".into());
        a.body = "{}".into();
        let err = compare(&case(&[]), &a, &b, NAMES).unwrap_err();
        assert!(err.contains("content-type"), "{err}");
    }

    #[test]
    fn status_and_text_bodies_compare_exactly() {
        let a = Recorded {
            status: 200,
            content_type: Some("text/plain; charset=utf-8".into()),
            body: "ok".into(),
        };
        let mut b = a.clone();
        assert!(compare(&case(&[]), &a, &b, NAMES).is_ok());

        b.status = 500;
        b.body = "ok\n".into();
        let err = compare(&case(&[]), &a, &b, NAMES).unwrap_err();
        assert!(err.contains("status: rails 200 vs rs 500"), "{err}");
        assert!(err.contains("body:"), "{err}");
    }

    #[test]
    fn non_json_bodies_under_a_json_type_are_reported() {
        let err = compare(&case(&[]), &json("{}"), &json("<html>"), NAMES).unwrap_err();
        assert!(err.contains("rs body is not JSON"), "{err}");
    }
}
