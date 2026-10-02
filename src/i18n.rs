//! Translations from the vendored config/locales/server.en.yml,
//! client.en.yml and the bundled plugins' client.en.yml files.
//!
//! Only the default locale is loaded, and only string leaves are indexed.
//! Discourse's I18n.t has one quirk that matters for parity: interpolation
//! runs only when arguments are passed. With none, `%{name}` placeholders are
//! returned literally; with some, a placeholder that has no argument makes
//! the lookup fail (Discourse's handler raises and the caller's default is
//! used instead).

use std::collections::HashMap;

use serde_yaml_ng::Value as Yaml;

const SERVER_EN_YML: &str = include_str!("../vendor/discourse/config/locales/server.en.yml");
const CLIENT_EN_YML: &str = include_str!("../vendor/discourse/config/locales/client.en.yml");
/// One YAML document per bundled plugin (scripts/vendor-discourse).
const PLUGIN_CLIENT_EN_YML: &str =
    include_str!("../vendor/discourse/config/locales/plugin_client.en.yml");

#[derive(Debug)]
pub struct I18n {
    strings: HashMap<String, String>,
}

impl I18n {
    /// server.en.yml plus client.en.yml (whose keys sit under `js.`), the
    /// latter for texts the HTML pages need, e.g. `js.action_codes.*`, then
    /// the bundled plugins' client translations, which cooking reads
    /// (`js.poll.*`). Later files win, as in Rails' load path.
    pub fn vendored() -> Result<Self, String> {
        let mut i18n = Self::parse(SERVER_EN_YML)?;
        let client =
            Self::parse(CLIENT_EN_YML).map_err(|e| e.replace("server.en.yml", "client.en.yml"))?;
        i18n.strings.extend(client.strings);
        for document in serde_yaml_ng::Deserializer::from_str(PLUGIN_CLIENT_EN_YML) {
            let root: Yaml = serde::Deserialize::deserialize(document)
                .map_err(|e| format!("plugin_client.en.yml: {e}"))?;
            let plugin = Self::from_yaml(root)
                .map_err(|e| e.replace("server.en.yml", "plugin_client.en.yml"))?;
            i18n.strings.extend(plugin.strings);
        }
        Ok(i18n)
    }

    pub fn parse(src: &str) -> Result<Self, String> {
        let root: Yaml = serde_yaml_ng::from_str(src).map_err(|e| format!("server.en.yml: {e}"))?;
        Self::from_yaml(root)
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
        Ok(I18n { strings })
    }

    /// `I18n.t(key)` with no arguments: the raw string, placeholders intact.
    pub fn t(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(String::as_str)
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
