//! Admin::SiteTextsController's model side: the English translation tree
//! as I18n searches it (recorded from the reference, gem locales and all,
//! into vendor/discourse/config/locales/i18n_tree.en.json by
//! scripts/record-i18n-tree), TranslationOverride's validations and
//! statuses, SiteTextSerializer, and ThemeTranslationManager's entries.
//!
//! Only English is served: other available locales are refused (the port
//! has no other translations).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

const TREE_JSON: &str = include_str!("../vendor/discourse/config/locales/i18n_tree.en.json");

/// `Admin::SiteTextsController::PREFERRED_KEYS`
pub const PREFERRED_KEYS: [&str; 4] = [
    "education.new-reply",
    "education.new-topic",
    "login_required.welcome_message",
    "system_messages.usage_tips.text_body_template",
];

/// `Admin::SiteTextsController::RESTRICTED_KEYS`
pub const RESTRICTED_KEYS: [&str; 7] = [
    "user_notifications.confirm_old_email.title",
    "user_notifications.confirm_old_email.subject_template",
    "user_notifications.confirm_old_email.text_body_template",
    "user_notifications.confirm_old_email_add.title",
    "user_notifications.confirm_old_email_add.subject_template",
    "user_notifications.confirm_old_email_add.text_body_template",
    "js.powered_by_discourse",
];

/// `TranslationOverride::ALLOWED_CUSTOM_INTERPOLATION_KEYS`
const CUSTOM_INTERPOLATION_KEYS: [(&[&str], &[&str]); 4] = [
    (
        &[
            "user_notifications.user_",
            "user_notifications.only_reply_by_email",
            "user_notifications.reply_by_email",
            "user_notifications.visit_link_to_respond",
            "user_notifications.header_instructions",
            "user_notifications.pm_participants",
            "unsubscribe_mailing_list",
            "unsubscribe_link_and_mail",
            "unsubscribe_link",
        ],
        &[
            "topic_title",
            "topic_title_url_encoded",
            "message",
            "url",
            "post_id",
            "topic_id",
            "context",
            "username",
            "recipient_username",
            "group_name",
            "unsubscribe_url",
            "subject_pm",
            "participants",
            "site_description",
            "site_title",
            "site_title_url_encoded",
            "site_name",
            "optional_re",
            "optional_pm",
            "optional_cat",
            "optional_tags",
        ],
    ),
    (
        &["system_messages.welcome_user"],
        &["username", "name", "name_or_username"],
    ),
    (&["js.welcome_banner.header"], &["site_name"]),
    (&["email_from"], &["site_name"]),
];

/// `I18n.t("i18n.plural.keys")` in English.
pub const PLURAL_KEYS: [&str; 2] = ["one", "other"];

/// The English tree: string leaves in I18n's walk order.
pub struct Tree {
    strings: Vec<(String, String)>,
    index: HashMap<String, usize>,
    /// Every key holding a hash.
    parents: HashSet<String>,
    /// Leaves that are neither strings nor hashes (arrays, numbers).
    others: HashSet<String>,
}

pub fn tree() -> &'static Tree {
    static TREE: OnceLock<Tree> = OnceLock::new();
    TREE.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Raw {
            strings: Vec<(String, String)>,
            other_leaves: Vec<String>,
        }
        let raw: Raw = serde_json::from_str(TREE_JSON).expect("vendored i18n_tree.en.json");
        let mut index = HashMap::with_capacity(raw.strings.len());
        let mut parents = HashSet::new();
        for (i, (key, _)) in raw.strings.iter().enumerate() {
            index.insert(key.clone(), i);
            let mut end = key.len();
            while let Some(dot) = key[..end].rfind('.') {
                if !parents.insert(key[..dot].to_string()) {
                    break;
                }
                end = dot;
            }
        }
        for key in &raw.other_leaves {
            let mut end = key.len();
            while let Some(dot) = key[..end].rfind('.') {
                parents.insert(key[..dot].to_string());
                end = dot;
            }
        }
        Tree {
            strings: raw.strings,
            index,
            parents,
            others: raw.other_leaves.into_iter().collect(),
        }
    })
}

impl Tree {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.index.get(key).map(|&i| self.strings[i].1.as_str())
    }

    /// The string leaves in I18n's walk order.
    pub fn strings_in_order(&self) -> &[(String, String)] {
        &self.strings
    }

    /// `I18n.exists?(key, :en)`
    pub fn exists(&self, key: &str) -> bool {
        self.index.contains_key(key) || self.parents.contains(key) || self.others.contains(key)
    }

    /// The hash under a key, in the tree's order; refused when it holds
    /// other leaves (their Ruby values are not recorded).
    fn subtree(&self, key: &str) -> Result<Option<Value>, Unsupported> {
        if !self.parents.contains(key) {
            return Ok(None);
        }
        let prefix = format!("{key}.");
        if self.others.iter().any(|k| k.starts_with(&prefix)) {
            return Err(Unsupported("site texts of hashes with non-string leaves"));
        }
        let mut root = Map::new();
        for (k, v) in &self.strings {
            let Some(rest) = k.strip_prefix(&prefix) else {
                continue;
            };
            let mut node = &mut root;
            let mut parts = rest.split('.').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    node.insert(part.to_string(), Value::String(v.clone()));
                } else {
                    let entry = node
                        .entry(part.to_string())
                        .or_insert_with(|| Value::Object(Map::new()));
                    let Value::Object(next) = entry else {
                        return Err(Unsupported("site texts of mixed hashes"));
                    };
                    node = next;
                }
            }
        }
        Ok(Some(Value::Object(root)))
    }

    /// `I18n.overrides_disabled { I18n.t(key, locale: :en) }`: the string,
    /// the hash, or I18n's missing translation text.
    pub fn default_value(&self, key: &str) -> Result<Value, Unsupported> {
        if let Some(s) = self.get(key) {
            return Ok(Value::String(s.to_string()));
        }
        if let Some(hash) = self.subtree(key)? {
            return Ok(hash);
        }
        if self.others.contains(key) {
            return Err(Unsupported("site texts of non-string leaves"));
        }
        Ok(Value::String(format!("Translation missing: en.{key}")))
    }
}

/// The site's overrides in a locale, in the table's order
/// (`TranslationOverride.where(locale:).pluck`, no ORDER BY).
pub struct Overrides {
    pub pairs: Vec<(String, String)>,
}

impl Overrides {
    pub async fn load(conn: &mut PgConnection, locale: &str) -> Result<Self, sqlx::Error> {
        let pairs: Vec<(String, String)> = sqlx::query_as(
            "SELECT translation_key, value FROM translation_overrides WHERE locale = $1",
        )
        .bind(locale)
        .fetch_all(&mut *conn)
        .await?;
        Ok(Overrides { pairs })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `I18n.t(key)` in English with the overrides.
    pub fn translate(&self, key: &str) -> Result<Value, Unsupported> {
        match self.get(key) {
            Some(v) => Ok(Value::String(v.to_string())),
            None => tree().default_value(key),
        }
    }
}

/// `TranslationOverride.transform_pluralized_key`
pub fn transform_pluralized_key(key: &str) -> String {
    for form in ["zero", "two", "few", "many"] {
        if let Some(base) = key.strip_suffix(&format!(".{form}")) {
            return format!("{base}.other");
        }
    }
    key.to_string()
}

/// `TranslationOverride.custom_interpolation_keys`
pub fn custom_interpolation_keys(key: &str) -> &'static [&'static str] {
    CUSTOM_INTERPOLATION_KEYS
        .iter()
        .find(|(prefixes, _)| prefixes.iter().any(|p| key.starts_with(p)))
        .map_or(&[], |(_, keys)| keys)
}

/// `I18nInterpolationKeysFinder.find`: I18n's interpolation patterns and
/// `{{name}}`, every capture group of every match, in order, once each.
pub fn interpolation_keys_in(text: &Value) -> Vec<String> {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let Value::String(text) = text else {
        return Vec::new();
    };
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"%%|%\{([a-zA-Z0-9_|]+)\}|%<([a-zA-Z0-9_]+)>([^0-9]*?[0-9]*\.?[0-9]*[bBdiouxXeEfgGcps])|\{\{([a-zA-Z0-9_]+)\}\}",
        )
        .expect("interpolation pattern")
    });
    let mut out: Vec<String> = Vec::new();
    for caps in re.captures_iter(text) {
        for group in caps.iter().skip(1).flatten() {
            let s = group.as_str().to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

/// `record_for`'s interpolation keys: the English default's (overrides
/// off) with the allowed custom ones, sorted.
pub fn record_interpolation_keys(key: &str) -> Result<Vec<String>, Unsupported> {
    let en_key = transform_pluralized_key(key);
    let mut keys = interpolation_keys_in(&tree().default_value(&en_key)?);
    for k in custom_interpolation_keys(&en_key) {
        if !keys.iter().any(|x| x == k) {
            keys.push(k.to_string());
        }
    }
    keys.sort();
    Ok(keys)
}

/// A site text before serialization (`record_for`).
#[derive(Clone)]
pub struct Record {
    pub id: String,
    pub value: Value,
    /// The record's locale: "en", or (as Rails' fix_plural_keys leaves it
    /// for a complete plural group) the value itself.
    pub locale: String,
    pub interpolation_keys: Vec<String>,
}

pub fn record_for(
    overrides: &Overrides,
    key: &str,
    value: Option<Value>,
    locale: &str,
) -> Result<Record, Unsupported> {
    let value = match value {
        Some(v) => v,
        None => overrides.translate(key)?,
    };
    Ok(Record {
        id: key.to_string(),
        value,
        locale: locale.to_string(),
        interpolation_keys: record_interpolation_keys(key)?,
    })
}

/// A `translation_overrides` row.
#[derive(sqlx::FromRow, Clone)]
pub struct OverrideRow {
    pub id: i32,
    pub locale: String,
    pub translation_key: String,
    pub value: String,
    pub original_translation: Option<String>,
    pub status: i32,
}

pub async fn find_override(
    conn: &mut PgConnection,
    locale: &str,
    key: &str,
) -> Result<Option<OverrideRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, locale, translation_key, value, original_translation, status \
         FROM translation_overrides WHERE locale = $1 AND translation_key = $2 LIMIT 1",
    )
    .bind(locale)
    .bind(key)
    .fetch_optional(&mut *conn)
    .await
}

fn status_name(status: i32) -> &'static str {
    match status {
        1 => "outdated",
        2 => "invalid_interpolation_keys",
        3 => "deprecated",
        _ => "up_to_date",
    }
}

/// SiteTextSerializer; `overridden_keys` as the index passes them, else
/// the record's own override decides.
pub async fn serialize(
    conn: &mut PgConnection,
    record: &Record,
    overridden_keys: Option<&[String]>,
) -> Result<Value, AppError> {
    let row = find_override(conn, &record.locale, &record.id).await?;
    let (status, old_default, new_default) = match &row {
        Some(r) => (
            status_name(r.status),
            r.original_translation
                .clone()
                .map(Value::String)
                .unwrap_or(Value::Null),
            tree().default_value(&transform_pluralized_key(&r.translation_key))?,
        ),
        None => ("up_to_date", Value::Null, Value::Null),
    };
    let overridden = match overridden_keys {
        Some(keys) => keys.contains(&record.id),
        None => row.is_some(),
    };
    Ok(json!({
        "id": record.id,
        "value": record.value,
        "status": status,
        "old_default": old_default,
        "new_default": new_default,
        "interpolation_keys": record.interpolation_keys,
        "overridden": overridden,
        "can_revert": overridden,
    }))
}

/// `TranslationOverride#invalid_interpolation_keys` for a value of a key.
pub fn invalid_interpolation_keys(key: &str, value: &str) -> Result<Vec<String>, Unsupported> {
    let transformed = transform_pluralized_key(key);
    let current_default = tree().default_value(&transformed)?;
    let blank = match &current_default {
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Object(m) => m.is_empty(),
        _ => false,
    };
    if blank || crate::ruby::is_blank(value) {
        return Ok(Vec::new());
    }
    let mut allowed = interpolation_keys_in(&current_default);
    allowed.extend(
        custom_interpolation_keys(&transformed)
            .iter()
            .map(|k| k.to_string()),
    );
    static ATTEMPT: OnceLock<regex::Regex> = OnceLock::new();
    let re = ATTEMPT
        .get_or_init(|| regex::Regex::new(r"%\{([^{}]+?)\}").expect("interpolation attempt"));
    let mut attempted: Vec<String> = Vec::new();
    for caps in re.captures_iter(value) {
        let k = caps[1].to_string();
        if !attempted.contains(&k) {
            attempted.push(k);
        }
    }
    Ok(attempted
        .into_iter()
        .filter(|k| !allowed.contains(k))
        .collect())
}

/// `TranslationOverride#refresh_status` (before_update): the default
/// rewritten as the original, then deprecated, invalid or up to date
/// (`original_translation_updated?` compares the two just made equal).
pub fn refreshed_status(key: &str, value: &str) -> Result<(Option<String>, i32), Unsupported> {
    let transformed = transform_pluralized_key(key);
    let current = tree().default_value(&transformed)?;
    let original = match &current {
        Value::String(s) => Some(s.clone()),
        _ => return Err(Unsupported("site text overrides of hashes")),
    };
    let deleted = tree().get(&transformed).is_none();
    let status = if deleted {
        3
    } else if !invalid_interpolation_keys(key, value)?.is_empty() {
        2
    } else {
        0
    };
    Ok((original, status))
}

/// `Badge.find_system_badge_id_from_translation_key`: `badges.<name>.*`
/// whose camelized name is one of Badge's constants.
pub fn system_badge_id(key: &str) -> Option<i32> {
    const BADGES: [(&str, i32); 49] = [
        ("BasicUser", 1),
        ("Member", 2),
        ("Regular", 3),
        ("Leader", 4),
        ("Welcome", 5),
        ("NicePost", 6),
        ("GoodPost", 7),
        ("GreatPost", 8),
        ("Autobiographer", 9),
        ("Editor", 10),
        ("WikiEditor", 48),
        ("FirstLike", 11),
        ("FirstShare", 12),
        ("FirstFlag", 13),
        ("FirstLink", 14),
        ("FirstQuote", 15),
        ("FirstMention", 40),
        ("FirstEmoji", 41),
        ("FirstOnebox", 42),
        ("FirstReplyByEmail", 43),
        ("ReadGuidelines", 16),
        ("Reader", 17),
        ("NiceTopic", 18),
        ("GoodTopic", 19),
        ("GreatTopic", 20),
        ("NiceShare", 21),
        ("GoodShare", 22),
        ("GreatShare", 23),
        ("Anniversary", 24),
        ("Promoter", 25),
        ("Campaigner", 26),
        ("Champion", 27),
        ("PopularLink", 28),
        ("HotLink", 29),
        ("FamousLink", 30),
        ("Appreciated", 36),
        ("Respected", 37),
        ("Admired", 31),
        ("OutOfLove", 33),
        ("HigherLove", 34),
        ("CrazyInLove", 35),
        ("ThankYou", 38),
        ("GivesBack", 32),
        ("Empathetic", 39),
        ("Enthusiast", 45),
        ("Aficionado", 46),
        ("Devotee", 47),
        ("NewUserOfTheMonth", 44),
        ("AutobiographerMinBioLength", 10),
    ];
    if !key.starts_with("badges.") {
        return None;
    }
    let name = key.split('.').nth(1)?;
    // ActiveSupport's camelize: each `_` part capitalized.
    let camel: String = name
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect();
    BADGES.iter().find(|(n, _)| *n == camel).map(|(_, id)| *id)
}

/// `I18n::Backend::DiscourseI18n.create_search_regexp(query, as_string:
/// true)`: Regexp.escape, then the quote and ellipsis variants.
pub fn search_regexp_string(query: &str) -> String {
    let mut escaped = String::new();
    for c in query.chars() {
        match c {
            '[' | ']' | '{' | '}' | '(' | ')' | '|' | '-' | '*' | '.' | '\\' | '?' | '+' | '^'
            | '$' | '#' => {
                escaped.push('\\');
                escaped.push(c);
            }
            ' ' => escaped.push_str("\\ "),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\x0c' => escaped.push_str("\\f"),
            '\x0b' => escaped.push_str("\\v"),
            c => escaped.push(c),
        }
    }
    let mut out = String::new();
    for c in escaped.chars() {
        match c {
            '\'' | '‘' | '’' | '‚' | '‹' | '›' => out.push_str("['‘’‚‹›]"),
            '"' | '“' | '”' | '„' | '«' | '»' => out.push_str("[\"“”„«»]"),
            c => out.push(c),
        }
    }
    out.replace("\\.\\.\\.", "\u{0}")
        .replace('…', "\u{0}")
        .replace('\u{0}', "(?:\\.\\.\\.|…)")
}

/// The same pattern for matching, case-insensitively.
pub fn search_regexp(query: &str) -> Result<regex::Regex, Unsupported> {
    let mut pattern = String::from("(?i)");
    let chars: Vec<char> = query.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '.' && chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'.') {
            pattern.push_str(r"(?:\.\.\.|…)");
            i += 3;
            continue;
        }
        match c {
            '…' => pattern.push_str(r"(?:\.\.\.|…)"),
            '\'' | '‘' | '’' | '‚' | '‹' | '›' => pattern.push_str("['‘’‚‹›]"),
            '"' | '“' | '”' | '„' | '«' | '»' => pattern.push_str("[\"“”„«»]"),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    regex::Regex::new(&pattern).map_err(|_| Unsupported("this site texts query"))
}

/// One of a theme's English translations (ThemeTranslationManager).
pub struct ThemeEntry {
    pub theme_id: i32,
    pub key: String,
    pub default: String,
    pub overridden: Option<String>,
}

impl ThemeEntry {
    pub fn id(&self) -> String {
        format!("js.theme_translations.{}.{}", self.theme_id, self.key)
    }

    pub fn value(&self) -> &str {
        self.overridden.as_deref().unwrap_or(&self.default)
    }

    /// `ThemeTranslationManager#site_text`
    pub fn site_text(&self) -> Value {
        let mut keys = interpolation_keys_in(&Value::String(self.default.clone()));
        keys.sort();
        json!({
            "id": self.id(),
            "value": self.value(),
            "status": "up_to_date",
            "old_default": null,
            "new_default": self.default,
            "overridden": self.overridden.is_some(),
            "can_revert": self.overridden.is_some(),
            "interpolation_keys": keys,
        })
    }
}

/// `Theme#translations` in English: the `en` translations field less
/// `theme_metadata`, flattened in the YAML's order, with the theme's
/// overrides. Empty when the YAML is not a valid translations file.
pub async fn theme_translations(
    conn: &mut PgConnection,
    theme_id: i32,
) -> Result<Vec<ThemeEntry>, AppError> {
    let field: Option<String> = sqlx::query_scalar(
        "SELECT value FROM theme_fields WHERE theme_id = $1 AND target_id = 4 AND name = 'en' \
         ORDER BY id LIMIT 1",
    )
    .bind(theme_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(src) = field.filter(|v| !crate::ruby::is_blank(v)) else {
        return Ok(Vec::new());
    };
    let overrides: Vec<(String, String)> = sqlx::query_as(
        "SELECT translation_key, value FROM theme_translation_overrides \
         WHERE theme_id = $1 AND locale = 'en'",
    )
    .bind(theme_id)
    .fetch_all(&mut *conn)
    .await?;
    let Ok(parsed) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&src) else {
        return Ok(Vec::new());
    };
    let Some(root) = parsed.as_mapping() else {
        return Ok(Vec::new());
    };
    if root.len() != 1 || root.keys().next().and_then(|k| k.as_str()) != Some("en") {
        return Ok(Vec::new());
    }
    let mut flat: Vec<(String, String)> = Vec::new();
    match root.values().next() {
        Some(serde_yaml_ng::Value::Mapping(m)) => {
            if !flatten_theme(m, "", &mut flat) {
                return Ok(Vec::new());
            }
        }
        Some(serde_yaml_ng::Value::Null) => {}
        _ => return Err(Unsupported("theme translations that are not a hash").into()),
    }
    Ok(flat
        .into_iter()
        .filter(|(k, _)| k != "theme_metadata" && !k.starts_with("theme_metadata."))
        .map(|(key, default)| ThemeEntry {
            theme_id,
            overridden: overrides
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone()),
            key,
            default,
        })
        .collect())
}

/// `check_contains_hashes` and `list_from_hash` together: false when a
/// leaf is not a string.
fn flatten_theme(
    map: &serde_yaml_ng::Mapping,
    prefix: &str,
    out: &mut Vec<(String, String)>,
) -> bool {
    for (k, v) in map {
        let key = match k {
            serde_yaml_ng::Value::String(s) => s.clone(),
            serde_yaml_ng::Value::Number(n) => n.to_string(),
            _ => return false,
        };
        let path = if prefix.is_empty() {
            key
        } else {
            format!("{prefix}.{key}")
        };
        match v {
            serde_yaml_ng::Value::String(s) => out.push((path, s.clone())),
            serde_yaml_ng::Value::Mapping(m) => {
                if !flatten_theme(m, &path, out) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// `ThemeTranslationManager.find_site_text`
pub async fn find_theme_text(
    conn: &mut PgConnection,
    id: &str,
) -> Result<Option<ThemeEntry>, AppError> {
    let Some((theme_id, key)) = theme_text_id(id) else {
        return Ok(None);
    };
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM themes WHERE id = $1)")
        .bind(theme_id)
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(None);
    }
    Ok(theme_translations(conn, theme_id)
        .await?
        .into_iter()
        .find(|e| e.key == key))
}

/// `SITE_TEXT_ID_PATTERN`: `js.theme_translations.<id>.<key>`.
pub fn theme_text_id(id: &str) -> Option<(i32, String)> {
    let rest = id.strip_prefix("js.theme_translations.")?;
    let (num, key) = rest.split_once('.')?;
    let digits = num.strip_prefix('-').unwrap_or(num);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || key.is_empty() {
        return None;
    }
    // `(.+)\z` stops at a newline as `.` does.
    if key.contains('\n') {
        return None;
    }
    Some((num.parse().ok()?, key.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_regexp_string_matches_ruby() {
        assert_eq!(
            search_regexp_string("bookmark reminder"),
            "bookmark\\ reminder"
        );
        assert_eq!(search_regexp_string("don't"), "don['‘’‚‹›]t");
        assert_eq!(search_regexp_string("a..."), "a(?:\\.\\.\\.|…)");
        assert_eq!(search_regexp_string("(x)"), "\\(x\\)");
    }

    #[test]
    fn finds_interpolation_keys_like_ruby() {
        let v = Value::String("%{a} %% %<b>s {{c}} %{a}".into());
        assert_eq!(interpolation_keys_in(&v), ["a", "b", "s", "c"]);
    }

    #[test]
    fn camelizes_badge_keys() {
        assert_eq!(system_badge_id("badges.first_like.name"), Some(11));
        assert_eq!(
            system_badge_id("badges.new_user_of_the_month.description"),
            Some(44)
        );
        assert_eq!(system_badge_id("badges.custom_thing.name"), None);
        assert_eq!(system_badge_id("js.badges.first_like.name"), None);
    }

    #[test]
    fn tree_loads() {
        let t = tree();
        assert_eq!(t.get("js.topic.create"), Some("New Topic"));
        assert!(t.exists("js.topic"));
        assert!(!t.exists("no.such.key"));
    }
}
