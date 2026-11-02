//! Runtime-packaged stylesheets and scripts, served under
//! /assets/<name> with the browser cache in mind, as Discourse serves its
//! digested assets: each carries a digest of its content, pages link it as
//! `/assets/<name>?v=<digest>` (`url`), and a request for a retained
//! digest is cached for good (`immutable`). Unknown versions and requests
//! without `v` get `no-cache`, and every response an ETag, so a browser holding
//! the file revalidates for a 304 instead of fetching it again.

use std::collections::HashMap;
use std::sync::OnceLock;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

/// The first 16 hex digits of a body's SHA-256.
pub fn digest_of(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    assets: Vec<Entry>,
    #[serde(default)]
    history: Vec<Entry>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    name: String,
    content_type: String,
    files: Vec<String>,
    #[serde(default)]
    separator: String,
    digest: Option<String>,
}

struct Asset {
    content_type: &'static str,
    body: axum::body::Bytes,
    digest: String,
}

struct Store {
    current: HashMap<String, String>,
    versions: HashMap<(String, String), Asset>,
}

impl Store {
    fn load(root: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        use std::path::{Component, Path};
        let root = root.canonicalize()?;
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
        if manifest.version != 1 || manifest.assets.is_empty() {
            return Err("unsupported or empty frontend asset manifest".into());
        }
        let mut store = Self {
            current: HashMap::new(),
            versions: HashMap::new(),
        };
        for (entries, current) in [(manifest.assets, true), (manifest.history, false)] {
            for entry in entries {
                if entry.name.is_empty()
                    || !Path::new(&entry.name)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
                    || entry.name.contains('\\')
                    || entry.files.is_empty()
                {
                    return Err(format!("invalid asset name or files: {}", entry.name).into());
                }
                let content_type = match entry.content_type.as_str() {
                    "css" => "text/css; charset=utf-8",
                    "js" => "text/javascript; charset=utf-8",
                    _ => return Err(format!("invalid content type: {}", entry.name).into()),
                };
                let mut body = Vec::new();
                for (index, file) in entry.files.iter().enumerate() {
                    if !Path::new(file)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
                    {
                        return Err(format!("invalid asset path: {file}").into());
                    }
                    let path = root.join(file).canonicalize()?;
                    if !path.starts_with(&root) {
                        return Err(format!("asset escapes package: {file}").into());
                    }
                    if index > 0 {
                        body.extend_from_slice(entry.separator.as_bytes());
                    }
                    body.extend_from_slice(&std::fs::read(path)?);
                }
                std::str::from_utf8(&body)?;
                let digest = digest_of(&body);
                if entry
                    .digest
                    .as_ref()
                    .is_some_and(|expected| expected != &digest)
                {
                    return Err(format!("asset digest mismatch: {}", entry.name).into());
                }
                if !current && entry.digest.is_none() {
                    return Err("historical assets require a digest".into());
                }
                if current
                    && store
                        .current
                        .insert(entry.name.clone(), digest.clone())
                        .is_some()
                {
                    return Err(format!("duplicate asset: {}", entry.name).into());
                }
                store.versions.insert(
                    (entry.name, digest.clone()),
                    Asset {
                        content_type,
                        body: body.into(),
                        digest,
                    },
                );
            }
        }
        Ok(store)
    }

    fn asset(&self, name: &str, query: Option<&str>) -> Option<(&Asset, bool)> {
        let requested = query
            .into_iter()
            .flat_map(|q| q.split('&'))
            .find_map(|pair| pair.strip_prefix("v="));
        if let Some(digest) = requested
            && let Some(asset) = self.versions.get(&(name.to_owned(), digest.to_owned()))
        {
            return Some((asset, true));
        }
        let current = self.current.get(name)?;
        self.versions
            .get(&(name.to_owned(), current.clone()))
            .map(|a| (a, false))
    }

    fn serve(&self, name: &str, query: Option<&str>, headers: &HeaderMap) -> Response {
        match self.asset(name, query) {
            Some((asset, immutable)) => cached(
                headers,
                asset.content_type,
                asset.body.clone(),
                &asset.digest,
                immutable,
            ),
            None => StatusCode::NOT_FOUND.into_response(),
        }
    }
}

static STORE: OnceLock<Store> = OnceLock::new();

fn default_directory() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("DISCOURSE_FRONTEND_DIR") {
        return path.into();
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(prefix) = executable.parent().and_then(|bin| bin.parent())
    {
        let installed = prefix.join("share/discourse-rs/frontend");
        if installed.join("manifest.json").is_file() {
            return installed;
        }
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static")
}

/// Validate all assets before listening. A restart activates a new package.
pub fn initialize() -> Result<(), Box<dyn std::error::Error>> {
    if STORE.get().is_none() {
        let directory = default_directory();
        let store = Store::load(&directory)
            .map_err(|e| format!("frontend assets at {}: {e}", directory.display()))?;
        let _ = STORE.set(store);
    }
    Ok(())
}

fn store() -> &'static Store {
    // Library users and tests don't necessarily enter through cli::serve.
    initialize().expect("valid frontend asset package");
    STORE.get().expect("frontend assets initialized")
}

/// An asset's url with its digest, for a page to link.
pub fn url(base_path: &str, name: &str) -> String {
    match store().current.get(name) {
        Some(digest) => format!("{base_path}/assets/{name}?v={digest}"),
        None => format!("{base_path}/assets/{name}"),
    }
}

/// Inline script consumers share the same validated package.
pub fn text(name: &str) -> &'static str {
    let (asset, _) = store().asset(name, None).expect("known frontend asset");
    std::str::from_utf8(&asset.body).expect("asset UTF-8 validated at startup")
}

/// The request names this digest in `If-None-Match`.
fn not_modified(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        })
}

/// A response for `body` under `etag`: a 304 when the browser holds it,
/// else the body, cached for good when `immutable`.
pub fn cached(
    headers: &HeaderMap,
    content_type: &'static str,
    body: impl IntoResponse,
    digest: &str,
    immutable: bool,
) -> Response {
    let etag = format!("\"{digest}\"");
    let cache_control = if immutable {
        "max-age=31556952, public, immutable"
    } else {
        "no-cache"
    };
    let mut response = if not_modified(headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], body).into_response()
    };
    let h = response.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    if let Ok(etag) = HeaderValue::from_str(&etag) {
        h.insert(header::ETAG, etag);
    }
    response
}

/// Every asset's name; each needs its /assets/<name> route.
pub fn names() -> impl Iterator<Item = &'static str> {
    store().current.keys().map(String::as_str)
}

/// GET /assets/<name>
pub fn serve(name: &str, query: Option<&str>, headers: &HeaderMap) -> Response {
    store().serve(name, query, headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_digest_is_cached_for_good_and_others_revalidate() {
        let url = url("", "topic.js");
        let query = url.split_once('?').map(|(_, q)| q);
        let fresh = serve("topic.js", query, &HeaderMap::new());
        assert_eq!(fresh.status(), StatusCode::OK);
        assert!(
            fresh.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("immutable")
        );

        let stale = serve("topic.js", Some("v=0"), &HeaderMap::new());
        assert_eq!(stale.headers()[header::CACHE_CONTROL], "no-cache");

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, stale.headers()[header::ETAG].clone());
        assert_eq!(
            serve("topic.js", None, &headers).status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(
            serve("nope.js", None, &headers).status(),
            StatusCode::NOT_FOUND
        );
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "drs-assets-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn manifest(&self, value: serde_json::Value) {
            std::fs::write(
                self.0.join("manifest.json"),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[tokio::test]
    async fn packaged_versions_keep_old_pages_working_and_support_rollback() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("old.js"), "old script").unwrap();
        std::fs::write(fixture.0.join("new.js"), "new script").unwrap();
        let old_digest = digest_of(b"old script");
        let new_digest = digest_of(b"new script");
        let old = serde_json::json!({"name":"topic.js", "content_type":"js", "files":["old.js"], "digest":old_digest});
        let new = serde_json::json!({"name":"topic.js", "content_type":"js", "files":["new.js"], "digest":new_digest});
        fixture.manifest(serde_json::json!({"version":1,"assets":[old.clone()]}));
        let before = Store::load(&fixture.0).unwrap();
        fixture.manifest(serde_json::json!({"version":1,"assets":[new],"history":[old]}));
        let after = Store::load(&fixture.0).unwrap();
        for (store, query, expected) in [
            (&before, None, "old script"),
            (&after, None, "new script"),
            (&after, Some(format!("v={old_digest}")), "old script"),
        ] {
            let response = store.serve("topic.js", query.as_deref(), &HeaderMap::new());
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/javascript; charset=utf-8"
            );
            if query.is_some() {
                assert!(
                    response.headers()[header::CACHE_CONTROL]
                        .to_str()
                        .unwrap()
                        .contains("immutable")
                );
            }
            let body = axum::body::to_bytes(response.into_body(), 100)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), expected.as_bytes());
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            format!("W/\"{old_digest}\"").parse().unwrap(),
        );
        assert_eq!(
            after
                .serve("topic.js", Some(&format!("v={old_digest}")), &headers)
                .status(),
            StatusCode::NOT_MODIFIED
        );
        assert_eq!(after.current["topic.js"], new_digest);
        assert_eq!(before.current["topic.js"], old_digest);
        std::fs::write(fixture.0.join("new.js"), "corrupt").unwrap();
        assert!(Store::load(&fixture.0).is_err());
    }

    #[test]
    fn removed_assets_are_still_served_by_their_retained_digest() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("script.js"), "script").unwrap();
        let digest = digest_of(b"script");
        fixture.manifest(serde_json::json!({"version":1,
            "assets":[{"name":"new.js","content_type":"js","files":["script.js"]}],
            "history":[{"name":"old.js","content_type":"js","files":["script.js"],"digest":digest}]}));
        let store = Store::load(&fixture.0).unwrap();
        assert_eq!(
            store
                .serve("old.js", Some(&format!("v={digest}")), &HeaderMap::new())
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            store.serve("old.js", None, &HeaderMap::new()).status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn missing_files_invalid_manifests_and_path_escapes_fail_at_load() {
        let fixture = Fixture::new();
        for file in ["missing.js", "../outside.js", "/etc/passwd"] {
            fixture.manifest(serde_json::json!({"version":1,"assets":[{"name":"topic.js","content_type":"js","files":[file]}]}));
            assert!(Store::load(&fixture.0).is_err(), "{file}");
        }
        fixture.manifest(serde_json::json!({"version":2,"assets":[]}));
        assert!(Store::load(&fixture.0).is_err());
    }
}
