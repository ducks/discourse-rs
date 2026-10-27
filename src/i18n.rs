//! Translations from the vendored config/locales/server.en.yml,
//! client.en.yml and the bundled plugins' client.en.yml files, and the
//! other vendored locales (server.fr.yml, client.fr.yml).
//!
//! English is what core serves; [`I18n::locale`] reads another locale
//! with Discourse's fallbacks (`FallbackLocaleList`: the locale, then
//! English) and its plural rules. String leaves are indexed, and lists of
//! strings (`date.month_names`) apart from them.
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
/// The other locales core carries, server and client files each.
const OTHER_LOCALE_YMLS: &[(&str, &str)] = &[
    (
        "server.fr.yml",
        include_str!("../vendor/discourse/config/locales/server.fr.yml"),
    ),
    (
        "client.fr.yml",
        include_str!("../vendor/discourse/config/locales/client.fr.yml"),
    ),
];

/// The locale whose overrides apply: the port serves English only (the
/// places that would translate refuse other locales).
pub const OVERRIDE_LOCALE: &str = "en";

#[derive(Debug)]
pub struct I18n {
    /// The vendored English strings, shared by every site of the process.
    strings: Arc<HashMap<String, String>>,
    /// English lists of strings.
    lists: Arc<HashMap<String, List>>,
    /// The other locales, by name.
    locales: Arc<HashMap<String, Table>>,
    overrides: RwLock<Overrides>,
}

/// A YAML list of strings; `~` entries are None (`month_names[0]`).
pub type List = Vec<Option<String>>;

/// One locale's strings and lists.
#[derive(Debug, Clone, Default)]
struct Table {
    strings: HashMap<String, String>,
    lists: HashMap<String, List>,
}

#[derive(Debug, Default)]
struct Overrides {
    /// What the table held when the map was read.
    fingerprint: Option<String>,
    /// English's.
    values: HashMap<String, &'static str>,
    /// The other locales', by locale.
    others: HashMap<String, HashMap<String, &'static str>>,
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
    /// Then the other locales' files.
    pub fn vendored() -> Result<Self, String> {
        let mut i18n = Self::parse(SERVER_EN_YML)?;
        i18n.add_yml("client.en.yml", CLIENT_EN_YML)?;
        for (name, src) in [
            ("plugin_client.en.yml", PLUGIN_CLIENT_EN_YML),
            ("plugin_server.en.yml", PLUGIN_SERVER_EN_YML),
        ] {
            for document in serde_yaml_ng::Deserializer::from_str(src) {
                let root: Yaml = serde::Deserialize::deserialize(document)
                    .map_err(|e| format!("{name}: {e}"))?;
                i18n.add(name, root)?;
            }
        }
        for (name, src) in OTHER_LOCALE_YMLS {
            i18n.add_yml(name, src)?;
        }
        Ok(i18n)
    }

    pub fn parse(src: &str) -> Result<Self, String> {
        let mut i18n = I18n::new(HashMap::new());
        i18n.add_yml("server.en.yml", src)?;
        Ok(i18n)
    }

    /// A locale file over what is loaded, later files winning: each of
    /// its top-level keys is a locale (`en:`, `fr:`).
    pub fn add_yml(&mut self, name: &str, src: &str) -> Result<(), String> {
        let root: Yaml = serde_yaml_ng::from_str(src).map_err(|e| format!("{name}: {e}"))?;
        self.add(name, root)
    }

    fn add(&mut self, name: &str, mut root: Yaml) -> Result<(), String> {
        root.apply_merge()
            .map_err(|e| format!("{name} merge keys: {e}"))?;
        let locales = root
            .as_mapping()
            .filter(|m| !m.is_empty())
            .ok_or(format!("{name}: missing a top-level locale"))?;
        for (locale, tree) in locales {
            let locale = locale
                .as_str()
                .ok_or(format!("{name}: a top-level key that is not a locale"))?;
            let mut table = Table::default();
            flatten(tree, String::new(), &mut table);
            if locale == "en" {
                Arc::make_mut(&mut self.strings).extend(table.strings);
                Arc::make_mut(&mut self.lists).extend(table.lists);
            } else {
                let into = Arc::make_mut(&mut self.locales)
                    .entry(locale.to_string())
                    .or_default();
                into.strings.extend(table.strings);
                into.lists.extend(table.lists);
            }
        }
        Ok(())
    }

    /// The same strings with no overrides read yet, for another site.
    pub fn for_site(&self) -> I18n {
        I18n {
            strings: self.strings.clone(),
            lists: self.lists.clone(),
            locales: self.locales.clone(),
            overrides: RwLock::new(Overrides::default()),
        }
    }

    fn new(strings: HashMap<String, String>) -> Self {
        I18n {
            strings: Arc::new(strings),
            lists: Arc::default(),
            locales: Arc::default(),
            overrides: RwLock::new(Overrides::default()),
        }
    }

    /// The loaded locales: English and the others.
    pub fn available_locales(&self) -> Vec<&str> {
        let mut all: Vec<&str> = self.locales.keys().map(String::as_str).collect();
        all.push("en");
        all.sort_unstable();
        all
    }

    /// Lookups in `locale` (`I18n.with_locale`), falling back to English.
    pub fn locale<'a>(&'a self, locale: &'a str) -> Localized<'a> {
        let locale = match locale {
            // LocaleSiteSetting::FALLBACKS
            "en_GB" => "en",
            other => other,
        };
        Localized { i18n: self, locale }
    }

    /// `I18n.overrides_by_locale`: the overrides read again when the
    /// table's rows changed since the last read. Every locale's: English
    /// is the one core serves, the others are read by [`Localized`].
    pub async fn refresh_overrides(
        &self,
        conn: &mut sqlx::PgConnection,
    ) -> Result<(), sqlx::Error> {
        let fingerprint: String = sqlx::query_scalar(
            "SELECT count(*)::text || '/' || COALESCE(md5(string_agg(translation_key || chr(31) \
             || value || chr(31) || locale, chr(30) ORDER BY id)), '') FROM translation_overrides",
        )
        .fetch_one(&mut *conn)
        .await?;
        {
            let current = self.overrides.read().unwrap_or_else(|e| e.into_inner());
            if current.fingerprint.as_deref() == Some(fingerprint.as_str()) {
                return Ok(());
            }
        }
        let rows: Vec<(String, String, String)> =
            sqlx::query_as("SELECT locale, translation_key, value FROM translation_overrides")
                .fetch_all(&mut *conn)
                .await?;
        let mut english = Vec::new();
        let mut others: HashMap<String, HashMap<String, &'static str>> = HashMap::new();
        for (locale, key, value) in rows {
            if locale == OVERRIDE_LOCALE {
                english.push((key, value));
            } else {
                others.entry(locale).or_default().insert(key, intern(value));
            }
        }
        self.set_overrides(english, Some(fingerprint));
        self.overrides
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .others = others;
        Ok(())
    }

    /// Replaces the English overrides (the read above, and tests).
    pub fn set_overrides(&self, rows: Vec<(String, String)>, fingerprint: Option<String>) {
        let values = rows.into_iter().map(|(k, v)| (k, intern(v))).collect();
        let mut current = self.overrides.write().unwrap_or_else(|e| e.into_inner());
        *current = Overrides {
            fingerprint,
            values,
            others: HashMap::new(),
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

    /// `I18n.t(key)` for a list of strings, in English.
    pub fn list(&self, key: &str) -> Option<&List> {
        self.lists.get(key)
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
        interpolate(self.t(key)?, args)
    }

    /// `I18n.t(key, count: n, **args)` in English: [`Localized::t_count`].
    pub fn t_count(&self, key: &str, count: i64, args: &[(&str, &str)]) -> Option<String> {
        self.locale("en").t_count(key, count, args)
    }

    pub fn len(&self) -> usize {
        self.strings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }
}

/// Lookups in one locale, as `I18n.with_locale(locale) { I18n.t(...) }`:
/// each key from the locale's overrides and files, else English's.
pub struct Localized<'a> {
    i18n: &'a I18n,
    locale: &'a str,
}

impl<'a> Localized<'a> {
    pub fn name(&self) -> &'a str {
        self.locale
    }

    fn table(&self) -> Option<&'a Table> {
        self.i18n.locales.get(self.locale)
    }

    /// The locale's own string: its override, else its file's.
    fn own(&self, key: &str) -> Option<&'a str> {
        let overridden = {
            let current = self
                .i18n
                .overrides
                .read()
                .unwrap_or_else(|e| e.into_inner());
            current
                .others
                .get(self.locale)
                .and_then(|o| o.get(key))
                .copied()
        };
        overridden.or_else(|| {
            self.table()
                .and_then(|t| t.strings.get(key))
                .map(String::as_str)
        })
    }

    /// Whether the locale itself has a plural entry under `key`.
    fn has_own_plural(&self, key: &str) -> bool {
        ["zero", "one", "two", "few", "many", "other"]
            .iter()
            .any(|form| self.own(&format!("{key}.{form}")).is_some())
    }

    /// `I18n.t(key)` with no arguments.
    pub fn t(&self, key: &str) -> Option<&'a str> {
        if self.locale == "en" {
            return self.i18n.t(key);
        }
        self.own(key).or_else(|| self.i18n.t(key))
    }

    /// `I18n.t(key, **args)`.
    pub fn t_with(&self, key: &str, args: &[(&str, &str)]) -> Option<String> {
        interpolate(self.t(key)?, args)
    }

    /// `I18n.t(key, count: n, **args)`: the entry from the first locale of
    /// the chain that has one, pluralized by that locale's rule
    /// (I18n::Backend::Pluralization): `zero` for 0 when present, the
    /// explicit `0` and `1` forms, then the rule's category, else `other`.
    pub fn t_count(&self, key: &str, count: i64, args: &[(&str, &str)]) -> Option<String> {
        let own = self.locale != "en" && self.has_own_plural(key);
        let locale = if own { self.locale } else { "en" };
        let form = |f: &str| {
            let k = format!("{key}.{f}");
            if own { self.own(&k) } else { self.i18n.t(&k) }
        };
        let raw = (count == 0)
            .then(|| form("zero"))
            .flatten()
            .or_else(|| {
                (count == 0 || count == 1)
                    .then(|| form(&count.to_string()))
                    .flatten()
            })
            .or_else(|| form(plural_category(locale, count)?))
            .or_else(|| form("other"))?;
        let count = count.to_string();
        let mut all: Vec<(&str, &str)> = args.to_vec();
        if !all.iter().any(|(k, _)| *k == "count") {
            all.push(("count", &count));
        }
        interpolate(raw, &all)
    }

    /// `I18n.t(key)` for a list of strings.
    pub fn list(&self, key: &str) -> Option<&'a List> {
        self.table()
            .and_then(|t| t.lists.get(key))
            .or_else(|| self.i18n.list(key))
    }
}

/// config/locales/plurals.rb's rule for an integer count, for the locales
/// this crate carries; None for another (only `other` then applies).
fn plural_category(locale: &str, n: i64) -> Option<&'static str> {
    let n = n.unsigned_abs();
    match locale {
        "en" => Some(if n == 1 { "one" } else { "other" }),
        "fr" => Some(if n <= 1 {
            "one"
        } else if n.is_multiple_of(1_000_000) {
            "many"
        } else {
            "other"
        }),
        _ => None,
    }
}

/// `%{name}` filled from `args`; a placeholder without one fails.
fn interpolate(raw: &str, args: &[(&str, &str)]) -> Option<String> {
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

fn flatten(node: &Yaml, prefix: String, out: &mut Table) {
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
            out.strings.insert(prefix, s.clone());
        }
        // A list of strings and nulls (`month_names`); others are not
        // translations.
        Yaml::Sequence(items)
            if items
                .iter()
                .all(|i| matches!(i, Yaml::String(_) | Yaml::Null)) =>
        {
            let list = items
                .iter()
                .map(|i| i.as_str().map(str::to_string))
                .collect();
            out.lists.insert(prefix, list);
        }
        // Numbers and booleans aren't translation strings.
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

    const TWO: &str = "\
en:
  greeting: \"Hello\"
  only_en: \"English\"
  songs:
    one: \"%{count} song\"
    other: \"%{count} songs\"
  months: [~, January, February]
fr:
  greeting: \"Bonjour\"
  songs:
    one: \"%{count} chanson\"
    other: \"%{count} chansons\"
";

    #[test]
    fn reads_a_locale_falling_back_to_english() {
        let i = I18n::parse(TWO).unwrap();
        assert_eq!(i.t("greeting"), Some("Hello"));
        let fr = i.locale("fr");
        assert_eq!(fr.t("greeting"), Some("Bonjour"));
        assert_eq!(fr.t("only_en"), Some("English"));
        assert_eq!(fr.t("missing"), None);
        // A locale nothing loaded is English throughout.
        assert_eq!(i.locale("de").t("greeting"), Some("Hello"));
        assert_eq!(i.available_locales(), ["en", "fr"]);
        let months = fr.list("months").unwrap();
        assert_eq!(months[0], None);
        assert_eq!(months[2].as_deref(), Some("February"));
    }

    #[test]
    fn pluralizes_by_the_locales_rule() {
        let i = I18n::parse(TWO).unwrap();
        let fr = i.locale("fr");
        // French: 0 and 1 are `one`; a million is `many`, absent here,
        // so `other`.
        assert_eq!(fr.t_count("songs", 0, &[]).as_deref(), Some("0 chanson"));
        assert_eq!(fr.t_count("songs", 1, &[]).as_deref(), Some("1 chanson"));
        assert_eq!(fr.t_count("songs", 2, &[]).as_deref(), Some("2 chansons"));
        assert_eq!(
            fr.t_count("songs", 1_000_000, &[]).as_deref(),
            Some("1000000 chansons")
        );
        // English: 0 is `other`.
        assert_eq!(i.t_count("songs", 0, &[]).as_deref(), Some("0 songs"));
        assert_eq!(i.t_count("songs", 1, &[]).as_deref(), Some("1 song"));
    }

    #[test]
    fn the_vendored_french_carries_date_names() {
        let i = I18n::vendored().unwrap();
        let fr = i.locale("fr");
        assert_eq!(
            fr.list("date.month_names").unwrap()[8].as_deref(),
            Some("Août")
        );
        assert_eq!(
            i.list("date.month_names").unwrap()[8].as_deref(),
            Some("August")
        );
        assert_ne!(fr.t("js.log_in"), i.t("js.log_in"));
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
