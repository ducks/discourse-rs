//! lib/highlight_js.rb: the site's highlight.js language bundle, built
//! from the vendored @highlightjs/cdn-assets 11.12.0 language files
//! (static/vendor/README) for `highlighted_languages`, and the versioned
//! path pages load it from.

use std::sync::Mutex;

use sha1::{Digest, Sha1};

include!(concat!(env!("OUT_DIR"), "/highlight_js_languages.rs"));

/// Bumped to invalidate caches following core changes.
const VERSION: u32 = 1;

/// `HighlightJs.languages`: every language there is a file for, sorted.
pub fn languages() -> impl Iterator<Item = &'static str> {
    HIGHLIGHT_JS_LANGUAGES.iter().map(|(name, _)| *name)
}

/// `HighlightJs.bundle`: an ES module registering each of `langs` that has
/// a file, in the given order.
pub fn bundle<'a>(langs: impl IntoIterator<Item = &'a str>) -> String {
    let lang_js: Vec<&str> = langs
        .into_iter()
        .filter_map(|lang| {
            HIGHLIGHT_JS_LANGUAGES
                .iter()
                .find(|(name, _)| *name == lang)
                .map(|(_, js)| *js)
        })
        .collect();
    format!(
        "export default function registerLanguages(hljs) {{\n  {}\n}}\n",
        lang_js.join("\n")
    )
}

/// `HighlightJs.version`'s cache, one per site as Rails keys it by
/// `current_db`: the last (languages, salt, digest).
#[derive(Default)]
pub struct VersionCache(Mutex<Option<(String, String, String)>>);

/// `HighlightJs.version`: the bundle's digest for a `highlighted_languages`
/// value, with `GlobalSetting.asset_url_salt`. The last one is kept.
pub fn version(cache: &VersionCache, lang_string: &str, asset_url_salt: &str) -> String {
    let mut cache = cache.0.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((langs, salt, digest)) = cache.as_ref()
        && langs == lang_string
        && salt == asset_url_salt
    {
        return digest.clone();
    }
    let body = bundle(lang_string.split('|')) + &format!("|{VERSION}|{asset_url_salt}");
    let digest = format!("{:x}", Sha1::digest(body.as_bytes()));
    *cache = Some((lang_string.into(), asset_url_salt.into(), digest.clone()));
    digest
}

/// `HighlightJs.path`, without the base path.
pub fn path(
    cache: &VersionCache,
    hostname: &str,
    lang_string: &str,
    asset_url_salt: &str,
) -> String {
    format!(
        "/highlight-js/{hostname}/{}.js",
        version(cache, lang_string, asset_url_salt)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "bash|c|cpp|csharp|css|diff|go|graphql|ini|java|javascript|json|kotlin|lua|makefile|markdown|objectivec|perl|php|php-template|plaintext|python|python-repl|r|ruby|rust|scss|shell|sql|swift|typescript|xml|yaml|wasm";

    #[test]
    fn lists_every_vendored_language() {
        let all: Vec<&str> = languages().collect();
        assert_eq!(all.len(), 193);
        assert!(all.is_sorted());
        assert!(all.contains(&"php-template"));
    }

    #[test]
    fn bundles_the_known_languages_in_order() {
        let js = bundle(["sql", "nope", "ruby"]);
        assert!(
            js.starts_with(
                "export default function registerLanguages(hljs) {\n  /*! `sql` grammar"
            )
        );
        assert!(js.find("`sql`").unwrap() < js.find("`ruby`").unwrap());
        assert!(js.ends_with("\n}\n"));
        assert_eq!(
            bundle([""]),
            "export default function registerLanguages(hljs) {\n  \n}\n"
        );
    }

    // The digest the reference serves for the default languages with no
    // salt (/highlight-js/localhost/<digest>.js).
    #[test]
    fn digests_as_rails_does() {
        let cache = VersionCache::default();
        assert_eq!(
            version(&cache, DEFAULT, ""),
            "d125c3422119592da271b68fa9500c957dd748c0"
        );
        assert_ne!(
            version(&cache, DEFAULT, "salt"),
            version(&cache, DEFAULT, "")
        );
        assert_eq!(
            path(&cache, "localhost", DEFAULT, ""),
            "/highlight-js/localhost/d125c3422119592da271b68fa9500c957dd748c0.js"
        );
    }
}
