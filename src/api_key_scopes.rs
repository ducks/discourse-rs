//! ApiKeyScope: the admin API key scope mappings (core's and the bundled
//! plugins', with the urls Rails lists for each) and RouteMatcher for them.
//!
//! A mapping names `controller#action`s, which Rails knows from the route
//! a request is dispatched to. The port has no such names, so the request
//! is recognized against Rails' own route table (first match by verb and
//! path, with each route's requirements), both recorded from the reference
//! into vendor/discourse/config/api_key_scopes.json by
//! scripts/record-api-key-scopes.

use std::sync::LazyLock;

use axum::http::Method;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::Unsupported;

const API_KEY_SCOPES_JSON: &str = include_str!("../vendor/discourse/config/api_key_scopes.json");

/// One mapping (`ApiKeyScope.scope_mappings[resource][action]`).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Mapping {
    pub actions: Option<Vec<String>>,
    pub methods: Option<Vec<String>>,
    pub formats: Option<Vec<String>>,
    pub params: Option<Vec<String>>,
    pub path_params: Option<Vec<String>>,
    pub aliases: Option<Map<String, Value>>,
    #[serde(default)]
    pub urls: Vec<String>,
}

impl Mapping {
    /// `restrictable_parameters`: params, then path params, deduplicated;
    /// None when there are none.
    pub fn restrictable_parameters(&self) -> Option<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for p in self
            .params
            .iter()
            .flatten()
            .chain(self.path_params.iter().flatten())
        {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn alias_for(&self, param: &str) -> Option<String> {
        self.aliases
            .as_ref()?
            .get(param)
            .and_then(Value::as_str)
            .map(str::to_string)
    }
}

#[derive(Deserialize)]
struct RawRoute {
    verb: String,
    path: String,
    #[serde(default)]
    requirements: Map<String, Value>,
    action: String,
    #[serde(default)]
    constraints: Vec<String>,
}

/// A Rails route, compiled.
struct Route {
    verbs: Vec<String>,
    pattern: Regex,
    names: Vec<String>,
    action: String,
    constraints: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    scopes: serde_json::Map<String, Value>,
    routes: Vec<RawRoute>,
}

struct Table {
    /// (resource, [(action, mapping)]) in Rails' order.
    scopes: Vec<(String, Vec<(String, Mapping)>)>,
    routes: Vec<Route>,
}

/// A Ruby regex source as Rust reads it: `\w`, `\d` and `\h` are ASCII
/// in Ruby.
fn ruby_regex(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let Some(n) = chars.next() else {
                    out.push('\\');
                    break;
                };
                let class = match n {
                    'w' => Some("A-Za-z0-9_"),
                    'd' => Some("0-9"),
                    'h' => Some("0-9a-fA-F"),
                    _ => None,
                };
                match class {
                    Some(set) if in_class => out.push_str(set),
                    Some(set) => {
                        out.push('[');
                        out.push_str(set);
                        out.push(']');
                    }
                    None => {
                        out.push('\\');
                        out.push(n);
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                out.push(c);
            }
            ']' if in_class => {
                in_class = false;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A Journey path spec (`/t/:slug/:topic_id(.:format)`, `*path`, optional
/// groups) as an anchored regex, with the names of its captures.
fn compile(spec: &str, requirements: &Map<String, Value>) -> Option<(Regex, Vec<String>)> {
    let mut out = String::from(r"\A");
    let mut names = Vec::new();
    let bytes: Vec<char> = spec.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            '(' => {
                out.push_str("(?:");
                i += 1;
            }
            ')' => {
                out.push_str(")?");
                i += 1;
            }
            ':' | '*' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == '_')
                {
                    end += 1;
                }
                let name: String = bytes[start..end].iter().collect();
                let default = if c == '*' { ".+?" } else { "[^/.?]+" };
                let req = requirements
                    .get(&name)
                    .and_then(Value::as_str)
                    .map(ruby_regex)
                    .unwrap_or_else(|| default.to_string());
                out.push_str(&format!("(?P<p{}>{req})", names.len()));
                names.push(name);
                i = end;
            }
            _ => {
                out.push_str(&regex::escape(&c.to_string()));
                i += 1;
            }
        }
    }
    out.push_str(r"\z");
    Regex::new(&out).ok().map(|r| (r, names))
}

static TABLE: LazyLock<Table> = LazyLock::new(|| {
    let raw: Raw = serde_json::from_str(API_KEY_SCOPES_JSON).expect("api_key_scopes.json");
    let scopes = raw
        .scopes
        .into_iter()
        .map(|(resource, actions)| {
            let actions = actions
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|(name, m)| (name, serde_json::from_value(m).unwrap_or_default()))
                .collect();
            (resource, actions)
        })
        .collect();
    let routes = raw
        .routes
        .into_iter()
        .filter_map(|r| {
            let (pattern, names) = compile(&r.path, &r.requirements)?;
            Some(Route {
                verbs: r
                    .verb
                    .split('|')
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .collect(),
                pattern,
                names,
                action: r.action,
                constraints: r.constraints,
            })
        })
        .collect();
    Table { scopes, routes }
});

/// `ApiKeyScope.scope_mappings`, in order, every plugin's included.
pub fn scope_mappings() -> &'static [(String, Vec<(String, Mapping)>)] {
    &TABLE.scopes
}

/// The recorded resources plugins add (`add_api_key_scope`, a filtered
/// register) and their plugin's enabled setting.
const PLUGIN_RESOURCES: [(&str, &str); 2] =
    [("chat", "chat_enabled"), ("solved", "solved_enabled")];

/// Whether `scope_mappings` has the resource now: core's always, a
/// plugin's while the plugin is enabled. A key's scope on a resource it
/// lacks permits nothing and shows no parameters or urls.
pub fn resource_enabled(
    resource: &str,
    settings: &crate::site_settings::SiteSettings,
) -> Result<bool, crate::site_settings::SettingError> {
    match PLUGIN_RESOURCES.iter().find(|(r, _)| *r == resource) {
        Some((_, setting)) => Ok(settings.get(setting)?.truthy()),
        None => Ok(true),
    }
}

/// `scope_mappings.dig(resource, action)` as the site has them now.
pub fn enabled_mapping(
    resource: &str,
    action: &str,
    settings: &crate::site_settings::SiteSettings,
) -> Result<Option<&'static Mapping>, crate::site_settings::SettingError> {
    if !resource_enabled(resource, settings)? {
        return Ok(None);
    }
    Ok(mapping(resource, action))
}

pub fn mapping(resource: &str, action: &str) -> Option<&'static Mapping> {
    TABLE
        .scopes
        .iter()
        .find(|(r, _)| r == resource)?
        .1
        .iter()
        .find(|(a, _)| a == action)
        .map(|(_, m)| m)
}

/// A recognized route's `controller#action` and path parameters.
pub type Recognized = (&'static str, Map<String, Value>);

/// The route a request reaches: its `controller#action` and path
/// parameters. Routes behind a constraint the port can't evaluate here
/// (the home page's, permalinks', markdown's, impersonation's, procs) are
/// refused; admin and staff constraints are the session layer's (a key
/// that does not let the request in is a 404 there).
pub fn recognize(method: &Method, path: &str) -> Result<Option<Recognized>, Unsupported> {
    let verb = if *method == Method::HEAD {
        "GET"
    } else {
        method.as_str()
    };
    for route in &TABLE.routes {
        if !route.verbs.is_empty() && !route.verbs.iter().any(|v| v == verb) {
            continue;
        }
        let Some(caps) = route.pattern.captures(path) else {
            continue;
        };
        if route
            .constraints
            .iter()
            .any(|c| c != "AdminConstraint" && c != "StaffConstraint")
        {
            return Err(Unsupported(
                "granular API key scopes on a constrained route",
            ));
        }
        let mut params = Map::new();
        for (i, name) in route.names.iter().enumerate() {
            if let Some(m) = caps.name(&format!("p{i}")) {
                params.insert(name.clone(), Value::String(m.as_str().to_string()));
            }
        }
        return Ok(Some((route.action.as_str(), params)));
    }
    Ok(None)
}

/// What a RouteMatcher sees of a request.
pub struct Request<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    pub query: Option<&'a str>,
    /// `request.parameters` less the path's: the query string over the
    /// body.
    pub params: &'a Map<String, Value>,
    /// The request format's symbol (`request.formats.first`).
    pub format: &'a str,
}

/// `RouteMatcher#match?` for a mapping with a scope's allowed parameters.
pub fn matches(
    m: &Mapping,
    allowed: Option<&Map<String, Value>>,
    r: &Request,
) -> Result<bool, Unsupported> {
    let recognized = recognize(r.method, r.path)?;
    let action_allowed = match &m.actions {
        None => true,
        Some(actions) => {
            let fullpath = match r.query {
                Some(q) if !q.is_empty() => format!("{}?{q}", r.path),
                _ => r.path.to_string(),
            };
            static BUS: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"\A/message-bus/.*/poll").unwrap());
            static LOGS: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"\A/logs/.*\.json\z").unwrap());
            (actions.iter().any(|a| a == "message_bus") && BUS.is_match(&fullpath))
                || (actions.iter().any(|a| a == "logster") && LOGS.is_match(&fullpath))
                || recognized
                    .as_ref()
                    .is_some_and(|(action, _)| actions.iter().any(|a| a == action))
        }
    };
    if !action_allowed {
        return Ok(false);
    }
    let empty = Map::new();
    let path_params = recognized.as_ref().map_or(&empty, |(_, p)| p);
    if !params_allowed(m, allowed, r.params, path_params) {
        return Ok(false);
    }
    let method_allowed = m.methods.as_ref().is_none_or(|methods| {
        methods
            .iter()
            .any(|x| x.eq_ignore_ascii_case(r.method.as_str()))
    });
    let format_allowed = m
        .formats
        .as_ref()
        .is_none_or(|formats| formats.iter().any(|f| f == r.format));
    Ok(method_allowed && format_allowed)
}

fn blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Bool(b) => !b,
        Value::Number(_) => false,
    }
}

/// `[allowed_param_values.fetch(param, [])].flatten`
fn allowed_values(allowed: &Map<String, Value>, param: &str) -> Vec<Value> {
    match allowed.get(param) {
        None => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(v) => vec![v.clone()],
    }
}

/// `extract_category_id`
fn extract_category_id(value: &Value) -> Value {
    let Some(s) = value.as_str() else {
        return Value::Null;
    };
    let last = s.split('/').next_back().unwrap_or("");
    if !s.is_empty() && !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) {
        Value::String(last.to_string())
    } else {
        Value::Null
    }
}

/// `params_allowed?`: ordinary params and path params against the
/// allowed values.
fn params_allowed(
    m: &Mapping,
    allowed: Option<&Map<String, Value>>,
    params: &Map<String, Value>,
    path_params: &Map<String, Value>,
) -> bool {
    let Some(allowed) = allowed.filter(|a| !a.is_empty()) else {
        return true;
    };
    let path_names: Vec<&String> = m.path_params.iter().flatten().collect();
    let ordinary: Vec<&String> = m
        .params
        .iter()
        .flatten()
        .filter(|p| !path_names.contains(p))
        .collect();
    // request.parameters: the path's over the rest.
    let mut all = params.clone();
    for (k, v) in path_params {
        all.insert(k.clone(), v.clone());
    }
    for param in ordinary {
        let alias = m.alias_for(param);
        let values = allowed_values(allowed, param);
        let value = all.get(param.as_str()).cloned().unwrap_or(Value::Null);
        let alias_value = alias
            .as_deref()
            .and_then(|a| all.get(a).cloned())
            .unwrap_or(Value::Null);
        if !blank(&value) && !blank(&alias_value) {
            return false;
        }
        let mut value = if value.is_null() { alias_value } else { value };
        if alias.as_deref() == Some("category_slug_path_with_id") {
            value = extract_category_id(&value);
        }
        if !(values.is_empty() || values.contains(&value)) {
            return false;
        }
    }
    for param in path_names {
        let values = allowed_values(allowed, param);
        if values.is_empty() {
            continue;
        }
        let alias = m.alias_for(param);
        let value = path_params.get(param.as_str()).filter(|v| !v.is_null());
        let alias_value = alias
            .as_deref()
            .and_then(|a| path_params.get(a))
            .filter(|v| !v.is_null());
        let requested = match (value, alias_value) {
            (Some(_), Some(_)) | (None, None) => return false,
            (Some(v), None) | (None, Some(v)) => v,
        };
        let scalar = matches!(
            requested,
            Value::String(_) | Value::Number(_) | Value::Bool(_)
        );
        if !scalar || !values.contains(requested) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_compiles() {
        let raw: Raw = serde_json::from_str(API_KEY_SCOPES_JSON).unwrap();
        let failed: Vec<String> = raw
            .routes
            .iter()
            .filter(|r| compile(&r.path, &r.requirements).is_none())
            .map(|r| r.path.clone())
            .collect();
        assert!(failed.is_empty(), "{failed:?}");
    }

    #[test]
    fn recognizes_actions_like_rails() {
        let get = |p| {
            recognize(&Method::GET, p)
                .unwrap()
                .map(|(a, _)| a.to_string())
        };
        assert_eq!(get("/latest.json").as_deref(), Some("list#latest"));
        assert_eq!(get("/u/user1.json").as_deref(), Some("users#show"));
        assert_eq!(
            recognize(&Method::POST, "/posts.json")
                .unwrap()
                .map(|(a, _)| a),
            Some("posts#create")
        );
        let (action, params) = recognize(&Method::PUT, "/t/some-slug/42.json")
            .unwrap()
            .unwrap();
        assert_eq!(action, "topics#update");
        assert_eq!(params["topic_id"], "42");
    }
}
