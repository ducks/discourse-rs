//! EmojisController: the emoji the picker offers (`Emoji.grouped`, GET
//! /emojis.json) and the words it searches them by (GET
//! /emojis/search-aliases.json), from the vendored discourse-emojis data
//! and the site's custom emoji.

use std::sync::LazyLock;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::SiteSettings;

const EMOJIS_JSON: &str = include_str!("../vendor/discourse-emojis/dist/emojis.json");
const GROUPS_JSON: &str = include_str!("../vendor/discourse-emojis/dist/groups.json");
const ALIASES_JSON: &str = include_str!("../vendor/discourse-emojis/dist/aliases.json");
const SEARCH_ALIASES_JSON: &str =
    include_str!("../vendor/discourse-emojis/dist/search_aliases.json");
const EN_SEARCH_ALIASES_JSON: &str =
    include_str!("../vendor/discourse-emojis/dist/locale_search_aliases/en.json");

/// `Emoji::DEFAULT_GROUP`, a custom emoji's group when it names none.
const DEFAULT_GROUP: &str = "default";

/// A standard emoji: its name, image file and group.
struct Standard {
    name: String,
    filename: String,
    group: String,
    tonable: bool,
}

/// `Emoji.load_standard`: emojis.json's emoji that groups.json places.
static STANDARD: LazyLock<Vec<Standard>> = LazyLock::new(|| {
    let emojis: Vec<Map<String, Value>> =
        serde_json::from_str(EMOJIS_JSON).expect("vendored emojis.json");
    let groups: Vec<Value> = serde_json::from_str(GROUPS_JSON).expect("vendored groups.json");
    let mut group_of = std::collections::HashMap::new();
    for group in &groups {
        let name = group["name"].as_str().unwrap_or_default();
        for icon in group["icons"].as_array().into_iter().flatten() {
            if let Some(icon) = icon["name"].as_str() {
                group_of.insert(icon.to_string(), name.to_string());
            }
        }
    }
    let tonable: Vec<String> =
        serde_json::from_str(discourse_markdown::emoji::TONABLE_JSON).expect("tonable_emojis.json");
    emojis
        .iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_string();
            let group = group_of.get(&name)?.clone();
            Some(Standard {
                filename: e
                    .get("filename")
                    .and_then(Value::as_str)
                    .unwrap_or(&name)
                    .to_string(),
                tonable: tonable.contains(&name),
                name,
                group,
            })
        })
        .collect()
});

/// A custom emoji (`Emoji.load_custom`): its upload's url, as stored and
/// through the CDN (`Emoji#cdn_url`), its group and who made it.
#[derive(Debug, Clone)]
pub struct Custom {
    pub name: String,
    pub url: Option<String>,
    pub cdn_url: Option<String>,
    /// `group || DEFAULT_GROUP`
    pub group: String,
    pub created_by: Option<String>,
}

/// What every place that draws emoji reads: the emoji settings,
/// `Emoji.custom` and `Emoji.denied`. Built once with the site settings
/// (`SiteSettings::emojis`), so a request sees one snapshot, as Rails'
/// cached lists give it.
#[derive(Debug, Clone, Default)]
pub struct Emojis {
    /// enable_emoji
    pub enabled: bool,
    /// enable_emoji_shortcuts
    pub shortcuts: bool,
    /// enable_inline_emoji_translation
    pub inline: bool,
    /// emoji_set
    pub set: String,
    /// external_emoji_url, when present.
    pub external_url: Option<String>,
    /// `Emoji.custom`, by name.
    pub custom: Vec<Custom>,
    /// `Emoji.denied`: the deny list's names, then their aliases.
    pub denied: Vec<String>,
}

impl Emojis {
    /// The settings part, from resolved setting values (a missing one
    /// takes its default).
    pub(crate) fn from_values(get: impl Fn(&str) -> Option<crate::site_settings::Value>) -> Emojis {
        let flag = |name: &str, default: bool| get(name).map_or(default, |v| v.truthy());
        let text = |name: &str| get(name).and_then(|v| v.presence());
        Emojis {
            enabled: flag("enable_emoji", true),
            shortcuts: flag("enable_emoji_shortcuts", true),
            inline: flag("enable_inline_emoji_translation", false),
            set: get("emoji_set").map_or_else(|| "twitter".into(), |v| v.to_s()),
            external_url: text("external_emoji_url"),
            custom: Vec::new(),
            denied: load_denied(&text("emoji_deny_list").unwrap_or_default()),
        }
    }

    /// `Emoji.load_custom`: the custom emoji rows with their uploads'
    /// urls, by name.
    pub(crate) async fn load_custom(
        conn: &mut PgConnection,
        cdn: Option<&str>,
    ) -> Result<Vec<Custom>, sqlx::Error> {
        #[derive(sqlx::FromRow)]
        struct Row {
            name: String,
            url: Option<String>,
            group: Option<String>,
            created_by: Option<String>,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT custom_emojis.name, uploads.url, custom_emojis.\"group\", \
             (SELECT username FROM users WHERE users.id = custom_emojis.user_id) AS created_by \
             FROM custom_emojis LEFT JOIN uploads ON uploads.id = custom_emojis.upload_id \
             ORDER BY custom_emojis.name",
        )
        .fetch_all(conn)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Custom {
                cdn_url: r.url.as_deref().map(|u| local_cdn_url(cdn, u)),
                name: r.name,
                url: r.url,
                group: r.group.unwrap_or_else(|| DEFAULT_GROUP.to_string()),
                created_by: r.created_by,
            })
            .collect())
    }

    /// `Emoji.custom?(name)`: the custom emoji of that exact name.
    pub fn custom(&self, name: &str) -> Option<&Custom> {
        self.custom.iter().find(|e| e.name == name)
    }

    /// `denied.include?(name)`
    pub fn is_denied(&self, name: &str) -> bool {
        self.denied.iter().any(|d| d == name)
    }

    /// `Emoji.gsub_emoji_to_unicode`: a denied code disappears.
    pub fn gsub_to_unicode(&self, s: &str) -> String {
        crate::emoji::gsub_emoji_to_unicode(s, &self.denied)
    }

    /// `Emoji.lookup_unicode`: "" for a denied name.
    pub fn lookup_unicode(&self, name: &str) -> Option<&'static str> {
        crate::emoji::lookup_unicode(name, &self.denied)
    }

    /// The custom emoji map cooking takes (`customEmoji`) and the client
    /// registers (`registerEmoji`, keyed lowercase): its cdn url.
    fn custom_url(&self, code: &str) -> Option<&str> {
        self.custom
            .iter()
            .find(|e| e.name.to_lowercase() == code)
            .and_then(|e| e.cdn_url.as_deref())
    }

    /// `isCustomEmoji(code)`
    pub fn is_custom(&self, code: &str) -> bool {
        let code = code.to_lowercase();
        self.custom.iter().any(|e| e.name.to_lowercase() == code)
    }

    /// `customEmoji` as cooking takes it: name to cdn url.
    pub fn custom_map(&self) -> Vec<(String, String)> {
        self.custom
            .iter()
            .filter_map(|e| Some((e.name.clone(), e.cdn_url.clone()?)))
            .collect()
    }

    /// `buildEmojiUrl(code)`: a custom emoji's upload, else a standard
    /// emoji's or alias's image (a tone as a path segment), with the
    /// image version; None for an unknown code.
    pub fn url(&self, code: &str, base_path: &str) -> Option<String> {
        let code = code.to_lowercase();
        if let Some(url) = self.custom_url(&code) {
            return Some(format!("{url}?v={}", crate::emoji::image_version()));
        }
        let name = code.split(':').next().filter(|n| !n.is_empty())?;
        if self.set.is_empty() || !crate::emoji::DATA.exists(name) {
            return None;
        }
        let base = match &self.external_url {
            Some(external) => external.clone(),
            None => format!("{base_path}/images/emoji"),
        };
        Some(format!(
            "{base}/{}/{}.png?v={}",
            self.set,
            code.replacen(":t", "/", 1),
            crate::emoji::image_version()
        ))
    }

    /// `Emoji.url_for(name)`: a standard emoji's image.
    pub fn url_for(&self, base_path: &str, filename: &str) -> String {
        let (name, tone) = match filename.rsplit_once(":t") {
            Some((name, tone))
                if tone.len() == 1 && ('1'..='6').contains(&tone.chars().next().unwrap_or('0')) =>
            {
                (name, Some(tone))
            }
            _ => (filename, None),
        };
        let name = match tone {
            Some(tone) => format!("{name}/{tone}"),
            None => name.to_string(),
        };
        let version = crate::emoji::image_version();
        match &self.external_url {
            Some(external) => format!("{external}/{}/{name}.png?v={version}", self.set),
            None => format!(
                "{base_path}/images/emoji/{}/{name}.png?v={version}",
                self.set
            ),
        }
    }

    /// A standard emoji's image url with `%{name}` for the name (the
    /// port's scripts fill it in).
    pub fn url_template(&self, base_path: &str) -> String {
        let base = match &self.external_url {
            Some(external) => external.clone(),
            None => format!("{base_path}/images/emoji"),
        };
        format!(
            "{base}/{}/%{{name}}.png?v={}",
            self.set,
            crate::emoji::image_version()
        )
    }

    /// `custom_urls` as JSON text.
    pub fn custom_urls_json(&self) -> String {
        self.custom_urls().to_string()
    }

    /// The custom emoji's image urls by lowercase name (`registerEmoji`),
    /// for the port's scripts.
    pub fn custom_urls(&self) -> Value {
        let map: Map<String, Value> = self
            .custom
            .iter()
            .filter_map(|e| {
                let url = e.cdn_url.as_ref()?;
                Some((
                    e.name.to_lowercase(),
                    json!(format!("{url}?v={}", crate::emoji::image_version())),
                ))
            })
            .collect();
        Value::Object(map)
    }

    /// The custom emoji as EmojiSerializer gives them (`/site/emoji`):
    /// name, cdn url, group and creator.
    pub fn custom_json(&self) -> Value {
        Value::Array(
            self.custom
                .iter()
                .map(|e| {
                    json!({
                        "name": e.name,
                        "url": e.cdn_url,
                        "group": e.group,
                        "created_by": e.created_by,
                    })
                })
                .collect(),
        )
    }
}

/// `Discourse.store.cdn_url` for the local store
/// (`UrlHelper.local_cdn_url`): an upload path behind the CDN.
fn local_cdn_url(cdn: Option<&str>, url: &str) -> String {
    match cdn {
        Some(cdn) if url.starts_with("/uploads/") => format!("{cdn}{url}"),
        _ => url.to_string(),
    }
}

/// `Emoji.load_denied`: the deny list's names and their aliases.
fn load_denied(list: &str) -> Vec<String> {
    let mut denied: Vec<String> = list
        .split('|')
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    if denied.is_empty() {
        return denied;
    }
    static ALIASES: LazyLock<Map<String, Value>> =
        LazyLock::new(|| serde_json::from_str(ALIASES_JSON).expect("vendored aliases.json"));
    let extra: Vec<String> = denied
        .iter()
        .filter_map(|name| ALIASES.get(name)?.as_array().cloned())
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    denied.extend(extra);
    denied
}

/// `Emoji.allowed.map(&:group)`: the groups of the standard and custom
/// emoji the deny list leaves.
pub fn allowed_groups(settings: &SiteSettings) -> Vec<String> {
    let emojis = settings.emojis();
    let standard = STANDARD
        .iter()
        .filter(|e| !emojis.is_denied(&e.name))
        .map(|e| e.group.clone());
    let custom = emojis
        .custom
        .iter()
        .filter(|e| !emojis.is_denied(&e.name))
        .map(|e| e.group.clone());
    standard.chain(custom).collect()
}

/// `Emoji.grouped`: the allowed emoji (standard, then custom, less the
/// denied) by group, in order of first appearance, the pinned groups
/// (emoji_picker_pinned_groups) first.
pub fn grouped(settings: &SiteSettings, base_path: &str) -> Result<Value, AppError> {
    let emojis = settings.emojis();
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    let mut add = |group: &str, emoji: Value| match groups.iter_mut().find(|(g, _)| g == group) {
        Some((_, list)) => list.push(emoji),
        None => groups.push((group.to_string(), vec![emoji])),
    };
    for e in STANDARD.iter() {
        if emojis.is_denied(&e.name) {
            continue;
        }
        add(
            &e.group,
            json!({
                "name": e.name,
                "tonable": e.tonable,
                "url": emojis.url_for(base_path, &e.filename),
                "group": e.group,
            }),
        );
    }
    for e in &emojis.custom {
        if emojis.is_denied(&e.name) {
            continue;
        }
        add(
            &e.group,
            json!({ "name": e.name, "url": e.url, "group": e.group, "created_by": e.created_by }),
        );
    }
    let pinned: Vec<String> = settings
        .get("emoji_picker_pinned_groups")?
        .to_s()
        .split('|')
        .filter(|g| !g.is_empty())
        .map(str::to_string)
        .collect();
    if !pinned.is_empty() {
        groups.sort_by_key(|(g, _)| pinned.iter().position(|p| p == g).unwrap_or(pinned.len()));
    }
    let mut out = Map::new();
    for (group, list) in groups {
        out.insert(group, Value::Array(list));
    }
    Ok(Value::Object(out))
}

/// GET /emojis/search-aliases.json: `Emoji.search_aliases` with the
/// locale's own terms merged in (English's, the port's one locale).
pub fn search_aliases() -> Value {
    static MERGED: LazyLock<Value> = LazyLock::new(|| {
        let mut aliases: Map<String, Value> =
            serde_json::from_str(SEARCH_ALIASES_JSON).expect("vendored search_aliases.json");
        let locale: Map<String, Value> =
            serde_json::from_str(EN_SEARCH_ALIASES_JSON).expect("vendored en.json");
        for (name, terms) in locale {
            match aliases.get_mut(&name) {
                Some(Value::Array(base)) => {
                    let mut unique: Vec<Value> = Vec::new();
                    for term in base.iter().chain(terms.as_array().into_iter().flatten()) {
                        if !unique.contains(term) {
                            unique.push(term.clone());
                        }
                    }
                    *base = unique;
                }
                _ => {
                    let mut unique: Vec<Value> = Vec::new();
                    for term in terms.as_array().into_iter().flatten() {
                        if !unique.contains(term) {
                            unique.push(term.clone());
                        }
                    }
                    aliases.insert(name, Value::Array(unique));
                }
            }
        }
        Value::Object(aliases)
    });
    MERGED.clone()
}
