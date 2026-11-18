//! The routes a router registers, for `discourse-rs routes`
//! (scripts/tally-refusals crawls them). axum keeps no public listing, so
//! this reads the router's Debug output: each route id's path from its
//! node, and each id's methods from the endpoint's Allow header value.

use axum::Router;
use regex::Regex;
use std::collections::HashMap;

/// A registered path and the methods it answers: the Allow header's
/// (`GET,HEAD,POST`), or `*` for a mounted service (nest_service), which
/// takes every method.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Route {
    pub methods: String,
    pub path: String,
}

/// Every route in the router's own table (not its fallbacks), by path.
/// An error when the Debug output has no routes or a route without a
/// path, which is axum's format having changed.
pub fn listing<S>(router: &Router<S>) -> Result<Vec<Route>, String> {
    let debug = format!("{router:?}");
    // The fallback router lists paths of its own ("/", the catch-all).
    let own = debug
        .split(", fallback_router: ")
        .next()
        .ok_or("no path_router in the router's Debug output")?;
    let (endpoints, nodes) = own
        .split_once(", node: Node { paths: {")
        .ok_or("no node paths in the router's Debug output")?;
    let path_re = Regex::new(r#"RouteId\((\d+)\): "((?:[^"\\]|\\.)*)""#).unwrap();
    let paths: HashMap<&str, &str> = path_re
        .captures_iter(nodes)
        .map(|c| (c.get(1).unwrap().as_str(), c.get(2).unwrap().as_str()))
        .collect();
    let endpoint_re = Regex::new(
        r#"RouteId\((\d+)\): (?:Route\(Route\)|MethodRouter\(MethodRouter \{[^{}]*?allow_header: (?:Bytes\(b"([^"]*)"\)|(\w+)))"#,
    )
    .unwrap();
    let mut routes = Vec::new();
    for c in endpoint_re.captures_iter(endpoints) {
        let id = c.get(1).unwrap().as_str();
        let path = paths
            .get(id)
            .ok_or_else(|| format!("route {id} has no path in the router's Debug output"))?;
        let methods = match (c.get(2), c.get(3)) {
            (Some(allow), _) => allow.as_str().to_string(),
            // AllowHeader::None/Skip: no method routes (`any`).
            (None, Some(other)) => format!("({})", other.as_str()),
            (None, None) => "*".to_string(),
        };
        routes.push(Route {
            methods,
            path: path.to_string(),
        });
    }
    if routes.is_empty() || routes.len() != paths.len() {
        return Err(format!(
            "read {} endpoints for {} paths from the router's Debug output",
            routes.len(),
            paths.len()
        ));
    }
    routes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(routes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};

    async fn handler() {}

    #[test]
    fn lists_paths_with_their_methods() {
        let router: Router<()> = Router::new()
            .route("/t/{slug}/{id}", get(handler).post(handler))
            .route("/session", post(handler))
            .merge(Router::new().route("/plugin.json", get(handler)))
            .nest_service("/uploads", tower_http::services::ServeDir::new("."))
            .route_layer(axum::middleware::from_fn(
                |req, next: axum::middleware::Next| async move { next.run(req).await },
            ))
            .fallback(handler);
        let routes = listing(&router).unwrap();
        let line = |r: &Route| format!("{} {}", r.methods, r.path);
        assert_eq!(
            routes.iter().map(line).collect::<Vec<_>>(),
            [
                "GET,HEAD /plugin.json",
                "POST /session",
                "GET,HEAD,POST /t/{slug}/{id}",
                "* /uploads",
                "* /uploads/",
                "* /uploads/{*__private__axum_nest_tail_param}",
            ]
        );
    }
}
