//! Translations from the vendored config/locales/server.en.yml,
//! client.en.yml and the bundled plugins' client.en.yml files.
//!
//! Only the default locale is loaded, and only string leaves are indexed.
//! Discourse's I18n.t has one quirk that matters for parity: interpolation
//! runs only when arguments are passed. With none, `%{name}` placeholders are
//! returned literally; with some, a placeholder that has no argument makes
//! the lookup fail (Discourse's handler raises and the caller's default is
//! used instead).
//!
//! Translation overrides (`translation_overrides`, the admin's site texts)
//! sit over the files as in Discourse's I18n backend: `t` returns the
//! override of a key once a row exists, and interpolation and plural forms
//! run on it. They are read again whenever the table changed
//! ([`I18n::refresh_overrides`], before every request and job).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use serde_yaml_ng::Value as Yaml;

const SERVER_EN_YML: &str = include_str!("../vendor/discourse/config/locales/server.en.yml");
const CLIENT_EN_YML: &str = include_str!("../vendor/discourse/config/locales/client.en.yml");
/// One YAML document per bundled plugin (scripts/vendor-discourse).
const PLUGIN_CLIENT_EN_YML: &str =
    include_str!("../vendor/discourse/config/locales/plugin_client.en.yml");
const PLUGIN_SERVER_EN_YML: &str =
    include_str!("../vendor/discourse/config/locales/plugin_server.en.yml");

/// The locale whose overrides apply: the port serves English only (the
/// places that would translate refuse other locales).
pub const OVERRIDE_LOCALE: &str = "en";

#[derive(Debug)]
pub struct I18n {
    /// The vendored strings, shared by every site of the process.
    strings: Arc<HashMap<String, String>>,
    overrides: RwLock<Overrides>,
}

#[derive(Debug, Default)]
struct Overrides {
    /// What the table held when the map was read.
    fingerprint: Option<String>,
    values: HashMap<String, &'static str>,
}

/// Override values handed out as `&str` for as long as the I18n lives,
/// though a later reload drops them from the map: each distinct value is
/// kept once for the process (admins write a handful).
fn intern(value: String) -> &'static str {
    static VALUES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut guard = VALUES.lock().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    if let Some(existing) = set.get(value.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(value.into_boxed_str());
    set.insert(leaked);
    leaked
}

impl I18n {
    /// server.en.yml plus client.en.yml (whose keys sit under `js.`), the
    /// latter for texts the HTML pages need, e.g. `js.action_codes.*`, then
    /// the bundled plugins' client translations, which cooking reads
    /// (`js.poll.*`), and their server ones (their settings' descriptions,
    /// badge names). Later files win, as in Rails' load path.
    pub fn vendored() -> Result<Self, String> {
        let mut i18n = Self::parse(SERVER_EN_YML)?;
        let client =
            Self::parse(CLIENT_EN_YML).map_err(|e| e.replace("server.en.yml", "client.en.yml"))?;
        Arc::make_mut(&mut i18n.strings).extend(Arc::unwrap_or_clone(client.strings));
        for (name, src) in [
            ("plugin_client.en.yml", PLUGIN_CLIENT_EN_YML),
            ("plugin_server.en.yml", PLUGIN_SERVER_EN_YML),
        ] {
            for document in serde_yaml_ng::Deserializer::from_str(src) {
                let root: Yaml = serde::Deserialize::deserialize(document)
                    .map_err(|e| format!("{name}: {e}"))?;
                let plugin = Self::from_yaml(root).map_err(|e| e.replace("server.en.yml", name))?;
                Arc::make_mut(&mut i18n.strings).extend(Arc::unwrap_or_clone(plugin.strings));
            }
        }
        Ok(i18n)
    }

    pub fn parse(src: &str) -> Result<Self, String> {
        let root: Yaml = serde_yaml_ng::from_str(src).map_err(|e| format!("server.en.yml: {e}"))?;
        Self::from_yaml(root)
    }

    /// The same strings with no overrides read yet, for another site.
    pub fn for_site(&self) -> I18n {
        I18n {
            strings: self.strings.clone(),
            overrides: RwLock::new(Overrides::default()),
        }
    }

    fn new(strings: HashMap<String, String>) -> Self {
        I18n {
            strings: Arc::new(strings),
            overrides: RwLock::new(Overrides::default()),
        }
    }

    /// `I18n.overrides_by_locale`: the overrides read again when the
    /// table's rows for the locale changed since the last read.
    pub async fn refresh_overrides(
        &self,
        conn: &mut sqlx::PgConnection,
    ) -> Result<(), sqlx::Error> {
        let fingerprint: String = sqlx::query_scalar(
            "SELECT count(*)::text || '/' || COALESCE(md5(string_agg(translation_key || chr(31) \
             || value, chr(30) ORDER BY id)), '') FROM translation_overrides WHERE locale = $1",
        )
        .bind(OVERRIDE_LOCALE)
        .fetch_one(&mut *conn)
        .await?;
        {
            let current = self.overrides.read().unwrap_or_else(|e| e.into_inner());
            if current.fingerprint.as_deref() == Some(fingerprint.as_str()) {
                return Ok(());
            }
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT translation_key, value FROM translation_overrides WHERE locale = $1",
        )
        .bind(OVERRIDE_LOCALE)
        .fetch_all(&mut *conn)
        .await?;
        self.set_overrides(rows, Some(fingerprint));
        Ok(())
    }

    /// Replaces the overrides (the read above, and tests).
    pub fn set_overrides(&self, rows: Vec<(String, String)>, fingerprint: Option<String>) {
        let values = rows.into_iter().map(|(k, v)| (k, intern(v))).collect();
        let mut current = self.overrides.write().unwrap_or_else(|e| e.into_inner());
        *current = Overrides {
            fingerprint,
            values,
        };
    }

    /// The override of a key, if an admin wrote one.
    pub fn override_of(&self, key: &str) -> Option<&str> {
        let current = self.overrides.read().unwrap_or_else(|e| e.into_inner());
        current.values.get(key).copied()
    }

    /// The overridden keys.
    pub fn overridden_keys(&self) -> Vec<String> {
        let current = self.overrides.read().unwrap_or_else(|e| e.into_inner());
        current.values.keys().cloned().collect()
    }

    /// `I18n.overrides_disabled { I18n.t(key) }`: the file's string.
    pub fn t_default(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(String::as_str)
    }

    /// `I18n.exists?(key, :en)`: a string leaf or a hash of them.
    pub fn exists(&self, key: &str) -> bool {
        if self.strings.contains_key(key) {
            return true;
        }
        let prefix = format!("{key}.");
        self.strings.keys().any(|k| k.starts_with(&prefix))
    }

    /// Every string of the files, for the site texts search.
    pub fn strings(&self) -> impl Iterator<Item = (&str, &str)> {
        self.strings.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    fn from_yaml(mut root: Yaml) -> Result<Self, String> {
        root.apply_merge()
            .map_err(|e| format!("server.en.yml merge keys: {e}"))?;
        let locale = root
            .as_mapping()
            .and_then(|m| m.get(Yaml::String("en".into())))
            .ok_or("server.en.yml: missing top-level `en`")?;

        let mut strings = HashMap::new();
        flatten(locale, String::new(), &mut strings);
        Ok(I18n::new(strings))
    }

    /// `I18n.t(key)` with no arguments: the raw string, placeholders
    /// intact, the key's override first.
    pub fn t(&self, key: &str) -> Option<&str> {
        self.override_of(key).or_else(|| self.t_default(key))
    }

    /// `I18n.t(key)` for a key holding a hash: its strings, nested as in
    /// the locale file. None when nothing is under the key.
    pub fn subtree(&self, key: &str) -> Option<serde_json::Value> {
        let prefix = format!("{key}.");
        let mut root = serde_json::Map::new();
        for (k, v) in self.strings.iter() {
            let Some(rest) = k.strip_prefix(&prefix) else {
                continue;
            };
            let mut node = &mut root;
            let mut parts = rest.split('.').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    node.insert(part.to_string(), serde_json::Value::String(v.clone()));
                } else {
                    node = node
                        .entry(part.to_string())
                        .or_insert_with(|| serde_json::Value::Object(Default::default()))
                        .as_object_mut()?;
                }
            }
        }
        (!root.is_empty()).then_some(serde_json::Value::Object(root))
    }

    /// The keys under a prefix.
    pub fn keys_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> {
        self.strings
            .keys()
            .filter(move |k| k.starts_with(prefix))
            .map(String::as_str)
    }

    /// `I18n.t(key, **args)`: every `%{name}` must have an argument.
    pub fn t_with(&self, key: &str, args: &[(&str, &str)]) -> Option<String> {
        let raw = self.t(key)?;
        let mut out = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some(start) = rest.find("%{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let end = after.find('}')?;
            let name = &after[..end];
            let value = args.iter().find(|(k, _)| *k == name)?.1;
            out.push_str(value);
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Some(out)
    }

    /// `I18n.t(key, count: n, **args)` in English: the `one` form for 1,
    /// `other` otherwise, `%{count}` filled with `n` unless given.
    pub fn t_count(&self, key: &str, count: i64, args: &[(&str, &str)]) -> Option<String> {
        let form = if count == 1 { "one" } else { "other" };
        let count = count.to_string();
        let mut all: Vec<(&str, &str)> = args.to_vec();
        if !all.iter().any(|(k, _)| *k == "count") {
            all.push(("count", &count));
        }
        self.t_with(&format!("{key}.{form}"), &all)
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

fn flatten(node: &Yaml, prefix: String, out: &mut HashMap<String, String>) {
    match node {
        Yaml::Mapping(m) => {
            for (k, v) in m {
                let Some(k) = k.as_str() else { continue };
                let key = if prefix.is_empty() {
                    k.to_string()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(v, key, out);
            }
        }
        Yaml::String(s) => {
            out.insert(prefix, s.clone());
        }
        // Numbers, booleans, and lists aren't translation strings.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_nested_keys_and_leaves_placeholders_alone() {
        let i = I18n::parse("en:\n  a:\n    b: \"Hi %{name}\"\n  n: 3\n").unwrap();
        assert_eq!(i.t("a.b"), Some("Hi %{name}"));
        assert_eq!(i.t("n"), None);
        assert_eq!(i.t("missing"), None);
    }

    #[test]
    fn interpolates_only_with_arguments() {
        let i =
            I18n::parse("en:\n  a: \"<a href=\\\"%{base_path}/x\\\">%{tos_url}</a>\"\n").unwrap();
        assert_eq!(
            i.t_with("a", &[("base_path", ""), ("tos_url", "/tos")])
                .as_deref(),
            Some("<a href=\"/x\">/tos</a>")
        );
        // A placeholder without an argument fails the lookup.
        assert_eq!(i.t_with("a", &[("base_path", "")]), None);
    }

    #[test]
    fn vendored_locale_loads_with_merge_keys() {
        let i = I18n::vendored().unwrap();
        assert!(i.len() > 3000, "only {} strings", i.len());
        assert_eq!(i.t("archetypes.regular.title"), Some("Regular Topic"));
        assert_eq!(
            i.t("post_action_types.notify_user.title"),
            Some("Send @%{username} a message")
        );
        // `<<: *datetime_formats` merged into the time formats
        assert!(i.t("time.formats.short").is_some());
        assert_eq!(
            i.t("js.action_codes.closed.enabled"),
            Some("Closed %{when}")
        );
    }
}
