//! The site settings' values Ruby code computes (SiteSettings::TypeSupervisor
//! enum classes and `choices` expressions): recorded from Rails at the
//! vendored commit (scripts/record-setting-enums) when they are constant,
//! computed here when they read the database or other settings.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{Value as Json, json};
use sqlx::PgConnection;

use crate::admin_site_settings::Context;
use crate::{AppError, Unsupported};

const RECORDED: &str = include_str!("../vendor/discourse/config/setting_enums.json");

fn record() -> &'static HashMap<String, Json> {
    static MAP: OnceLock<HashMap<String, Json>> = OnceLock::new();
    MAP.get_or_init(|| serde_json::from_str(RECORDED).expect("setting_enums.json is JSON"))
}

/// The recorded `valid_values`, `translate_names`, `choices` and
/// `json_schema` of a setting, by name.
pub fn recorded(setting: &str) -> Option<&'static serde_json::Map<String, Json>> {
    if setting.starts_with('_') {
        return None;
    }
    record().get(setting).and_then(Json::as_object)
}

/// The settings plugins register from Ruby rather than in YAML.
pub fn dynamic_settings() -> &'static [Json] {
    record()
        .get("_dynamic")
        .and_then(Json::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// A setting's area when a plugin stored it as a string, not a list.
pub fn string_area(setting: &str) -> Option<&'static str> {
    record().get("_string_areas")?.get(setting)?.as_str()
}

/// SiteSettings::DeprecatedSettings with the plugins' additions: the old
/// names that now mean `setting`.
pub fn deprecated_aliases(setting: &str) -> Vec<String> {
    record()
        .get("_deprecated")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter(|pair| pair[1].as_str() == Some(setting))
        .filter_map(|pair| pair[0].as_str().map(str::to_string))
        .collect()
}

/// `klass.values` and `klass.translate_names?` for the enum classes whose
/// values come from the database or other settings.
pub async fn runtime(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    class: &str,
) -> Result<(Json, bool), AppError> {
    Ok(match class {
        // AiAgent.all_agents(enabled_only: false): `ordered`, at most 500.
        "DiscourseAi::Configuration::AgentEnumerator" => {
            let rows: Vec<(String, i64)> = sqlx::query_as(
                "SELECT name, id::bigint FROM ai_agents ORDER BY priority DESC, lower(name) ASC LIMIT 500",
            )
            .fetch_all(&mut *conn)
            .await?;
            (named(rows), false)
        }
        "DiscourseAi::Configuration::LlmEnumerator" => {
            let rows: Vec<(String, i64)> =
                sqlx::query_as("SELECT display_name, id::bigint FROM llm_models")
                    .fetch_all(&mut *conn)
                    .await?;
            (named(rows), false)
        }
        "DiscourseAi::Configuration::EmbeddingDefsEnumerator" => {
            let rows: Vec<(String, i64)> =
                sqlx::query_as("SELECT display_name, id::bigint FROM embedding_definitions")
                    .fetch_all(&mut *conn)
                    .await?;
            (named(rows), false)
        }
        // DiscourseReactions::Reaction.valid_reactions (the like's reaction
        // and the enabled ones, a set) less those excluded from likes.
        "ReactionForLikeSiteSettingEnum" => {
            let setting = |name: &str| cx.settings.get(name).map(|v| v.to_s());
            let mut reactions =
                vec![setting("discourse_reactions_reaction_for_like")?.replace('-', "")];
            for r in setting("discourse_reactions_enabled_reactions")?.split('|') {
                if !r.is_empty() && !reactions.iter().any(|x| x == r) {
                    reactions.push(r.to_string());
                }
            }
            let excluded = setting("discourse_reactions_excluded_from_like")?;
            let excluded: Vec<&str> = excluded.split('|').collect();
            let values: Vec<Json> = reactions
                .into_iter()
                .filter(|r| !excluded.contains(&r.as_str()))
                .map(|r| json!({ "name": r, "value": r }))
                .collect();
            (json!(values), false)
        }
        // The top menu's default, TopMenu.homepage_choices, then the
        // homepages enabled plugins register (discourse-ai's conversations
        // while the bot is on).
        "HomepageSiteSetting" => {
            let mut values =
                vec![json!({ "name": "admin.homepage.top_menu_default", "value": "" })];
            for choice in homepage_choices(cx)? {
                values.push(json!({ "name": format!("filters.{choice}.title"), "value": choice }));
            }
            if cx.defs.plugin_enabled("discourse-ai", cx.settings)
                && cx.settings.get("ai_bot_enabled")?.truthy()
            {
                values.push(json!({
                    "name": "discourse_ai.ai_bot.conversations.homepage_option",
                    "value": "ai-conversations",
                }));
            }
            (json!(values), true)
        }
        _ => return Err(Unsupported("site setting values of an unported enum class").into()),
    })
}

fn named(rows: Vec<(String, i64)>) -> Json {
    json!(
        rows.into_iter()
            .map(|(name, value)| json!({ "name": name, "value": value }))
            .collect::<Vec<_>>()
    )
}

/// `TopMenu.homepage_choices`: `choices | (Discourse.filters - unread)`.
fn homepage_choices(cx: &Context<'_>) -> Result<Vec<String>, AppError> {
    let mut choices: Vec<String> = [
        "latest",
        "new",
        "unseen",
        "top",
        "categories",
        "read",
        "posted",
        "bookmarks",
        "hot",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if !cx.settings.get("enable_unified_new")?.truthy() {
        choices.push("unread".into());
    }
    // Discourse.filters less unread, with what bundled plugins push at load
    // (discourse-topic-voting's votes, enabled or not).
    for filter in [
        "latest",
        "new",
        "unseen",
        "top",
        "read",
        "posted",
        "bookmarks",
        "hot",
        "votes",
    ] {
        if !choices.iter().any(|c| c == filter) {
            choices.push(filter.into());
        }
    }
    Ok(choices)
}

/// The measured size of one of Discourse's stock images (a seeded upload's
/// url), for when the file is not beside the port.
pub fn stock_upload_dimensions(url: &str) -> Option<(i32, i32)> {
    let size = record().get("_stock_upload_dimensions")?.get(url)?;
    Some((size[0].as_i64()? as i32, size[1].as_i64()? as i32))
}
