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

/// `Emoji.url_for`
fn url_for(settings: &SiteSettings, base_path: &str, filename: &str) -> Result<String, AppError> {
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
    let set = settings.get("emoji_set")?.to_s();
    let version = crate::emoji::image_version();
    Ok(match settings.get("external_emoji_url")?.presence() {
        Some(external) => format!("{external}/{set}/{name}.png?v={version}"),
        None => format!("{base_path}/images/emoji/{set}/{name}.png?v={version}"),
    })
}

/// `Emoji.denied`: the deny list's names and their aliases.
fn denied(settings: &SiteSettings) -> Result<Vec<String>, AppError> {
    let list = settings.get("emoji_deny_list")?.to_s();
    let mut denied: Vec<String> = list
        .split('|')
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    if denied.is_empty() {
        return Ok(denied);
    }
    let aliases: Map<String, Value> =
        serde_json::from_str(ALIASES_JSON).expect("vendored aliases.json");
    let extra: Vec<String> = denied
        .iter()
        .filter_map(|name| aliases.get(name)?.as_array().cloned())
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    denied.extend(extra);
    Ok(denied)
}

/// `Emoji.grouped`: the allowed emoji (standard, then custom, less the
/// denied) by group, in order of first appearance, the pinned groups
/// (emoji_picker_pinned_groups) first.
pub async fn grouped(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    base_path: &str,
) -> Result<Value, AppError> {
    let denied = denied(settings)?;
    let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
    let mut add = |group: &str, emoji: Value| match groups.iter_mut().find(|(g, _)| g == group) {
        Some((_, list)) => list.push(emoji),
        None => groups.push((group.to_string(), vec![emoji])),
    };
    for e in STANDARD.iter() {
        if denied.contains(&e.name) {
            continue;
        }
        add(
            &e.group,
            json!({
                "name": e.name,
                "tonable": e.tonable,
                "url": url_for(settings, base_path, &e.filename)?,
                "group": e.group,
            }),
        );
    }
    // Emoji.load_custom
    #[derive(sqlx::FromRow)]
    struct Custom {
        name: String,
        url: Option<String>,
        group: Option<String>,
        created_by: Option<String>,
    }
    let custom: Vec<Custom> = sqlx::query_as(
        "SELECT custom_emojis.name, uploads.url, custom_emojis.\"group\", users.username AS created_by \
         FROM custom_emojis LEFT JOIN uploads ON uploads.id = custom_emojis.upload_id \
         LEFT JOIN users ON users.id = custom_emojis.user_id ORDER BY custom_emojis.name",
    )
    .fetch_all(&mut *conn)
    .await?;
    for Custom {
        name,
        url,
        group,
        created_by,
    } in custom
    {
        if denied.contains(&name) {
            continue;
        }
        let group = group.unwrap_or_else(|| DEFAULT_GROUP.to_string());
        add(
            &group,
            json!({ "name": name, "url": url, "group": group, "created_by": created_by }),
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
