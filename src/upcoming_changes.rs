//! UpcomingChanges (lib/upcoming_changes.rb) as the upcoming changes page
//! reads and writes it: UpcomingChanges::List's entries (the metadata
//! site_settings.yml declares, ConditionalDisplay, the enabled-for target
//! and its groups) and the site_setting_groups an opt-in group list keeps.

use std::collections::HashMap;

use serde_json::{Map, Value as Json, json};
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::AppError;
use crate::admin_site_settings::Context;
use crate::site_settings::{ChangeStatus, Definition, Definitions, SiteSettings, Value};

/// `Group::AUTO_GROUPS[:staff]`
pub const STAFF_GROUP_ID: i32 = 3;

/// `Theme::CORE_THEMES["horizon"]`
const HORIZON_THEME_ID: i32 = -2;

/// `UpcomingChangeEvent.event_types`
pub mod event_types {
    pub const ADDED: i32 = 0;
    pub const AUTOMATICALLY_PROMOTED: i32 = 2;
    pub const MANUAL_OPT_IN: i32 = 3;
    pub const MANUAL_OPT_OUT: i32 = 4;
    pub const STATUS_CHANGED: i32 = 5;
    pub const ADMINS_NOTIFIED_AVAILABLE_CHANGE: i32 = 6;
    pub const ADMINS_NOTIFIED_AUTOMATIC_PROMOTION: i32 = 7;
}

/// The setting's `upcoming_change:` mapping as declared.
fn declared(def: &Definition) -> Option<&serde_yaml_ng::Mapping> {
    def.options
        .as_ref()?
        .get(Yaml::String("upcoming_change".into()))?
        .as_mapping()
}

fn yaml_json(v: &Yaml) -> Json {
    serde_json::to_value(v).unwrap_or(Json::Null)
}

/// `Array(x).map(&:to_sym)` of a list the metadata declares, nil when
/// absent.
fn symbols(v: Option<&Yaml>) -> Json {
    match v {
        None | Some(Yaml::Null) => Json::Null,
        Some(Yaml::Sequence(items)) => Json::Array(items.iter().map(yaml_json).collect()),
        Some(other) => Json::Array(vec![yaml_json(other)]),
    }
}

/// `SiteSetting.upcoming_change_metadata[name]`: the declared keys but
/// impact, allow_enabled_for and hide_settings, then those normalized
/// (`impact` split into its type and role).
pub fn metadata(def: &Definition) -> Map<String, Json> {
    let mut out = Map::new();
    let Some(yml) = declared(def) else {
        return out;
    };
    let get = |key: &str| yml.get(Yaml::String(key.into()));
    for (key, value) in yml {
        let Some(key) = key.as_str() else {
            continue;
        };
        if matches!(key, "impact" | "allow_enabled_for" | "hide_settings") {
            continue;
        }
        out.insert(key.to_string(), yaml_json(value));
    }
    let impact = get("impact").and_then(Yaml::as_str).unwrap_or_default();
    let mut parts = impact.split(',');
    let mut part = || {
        parts
            .next()
            .filter(|p| !p.is_empty())
            .map_or(Json::Null, |p| json!(p))
    };
    out.insert("impact_type".into(), part());
    out.insert("impact_role".into(), part());
    if let Some(status) = def.upcoming_change {
        out.insert("status".into(), json!(status.as_str()));
    }
    out.insert(
        "allow_enabled_for".into(),
        symbols(get("allow_enabled_for")),
    );
    out.insert(
        "body_class".into(),
        get("body_class").map_or(Json::Null, yaml_json),
    );
    out.insert(
        "permanent_warning".into(),
        json!(!matches!(get("permanent_warning"), Some(Yaml::Bool(false)))),
    );
    out.insert("hide_settings".into(), symbols(get("hide_settings")));
    out
}

/// `UpcomingChanges.allow_enabled_for`: None when every target is allowed.
pub fn allow_enabled_for(def: &Definition) -> Option<Vec<String>> {
    match symbols(declared(def).and_then(|y| y.get(Yaml::String("allow_enabled_for".into())))) {
        Json::Array(items) => Some(
            items
                .iter()
                .filter_map(|i| i.as_str().map(str::to_string))
                .collect(),
        ),
        _ => None,
    }
}

/// `UpcomingChanges.target_allowed?`
pub fn target_allowed(def: &Definition, target: &str) -> bool {
    if target == "no_one" {
        return true;
    }
    allow_enabled_for(def).is_none_or(|allow| allow.iter().any(|a| a == target))
}

/// `UpcomingChanges.groups_target_allowed?`
pub fn groups_target_allowed(def: &Definition) -> bool {
    allow_enabled_for(def)
        .is_none_or(|allow| allow.iter().any(|a| a == "staff" || a == "specific_groups"))
}

/// Whether `UpcomingChanges.settings_hidden_while_enabled` names the
/// setting: an enabled change lists it in its `hide_settings`.
pub fn hidden_while_enabled(defs: &Definitions, settings: &SiteSettings, name: &str) -> bool {
    defs.iter()
        .filter(|d| d.is_upcoming_change())
        .any(|change| {
            let hides =
                match declared(change).and_then(|y| y.get(Yaml::String("hide_settings".into()))) {
                    Some(Yaml::Sequence(names)) => names.iter().any(|n| n.as_str() == Some(name)),
                    Some(Yaml::String(n)) => n == name,
                    _ => false,
                };
            hides && truthy(settings, &change.name)
        })
}

/// `SiteSetting.site_setting_group_ids`: each row's ids, `to_i` of the
/// `|`-separated list.
pub async fn site_setting_group_ids(
    conn: &mut PgConnection,
) -> Result<HashMap<String, Vec<i32>>, sqlx::Error> {
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT name, group_ids FROM site_setting_groups")
            .fetch_all(&mut *conn)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(name, ids)| {
            let ids = ids
                .unwrap_or_default()
                .split('|')
                .filter(|s| !s.is_empty())
                .map(|s| crate::ruby::to_i(s) as i32)
                .collect();
            (name, ids)
        })
        .collect())
}

/// `SiteSetting.current[name]`: the stored value, else the default,
/// before UpcomingChanges.enabled? resolves it.
pub async fn current_value(conn: &mut PgConnection, def: &Definition) -> Result<bool, sqlx::Error> {
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT value FROM site_settings WHERE name = $1")
            .bind(&def.name)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(match stored {
        Some(value) => matches!(value.as_deref(), Some("t" | "true")),
        None => def.default.truthy(),
    })
}

fn truthy(settings: &SiteSettings, name: &str) -> bool {
    settings.get(name).is_ok_and(Value::truthy)
}

/// `UpcomingChanges::ConditionalDisplay.should_display?`: the owning
/// plugin enabled (unless the change opts out), then the change's own
/// rule, if core defines one.
pub async fn should_display(
    conn: &mut PgConnection,
    defs: &Definitions,
    settings: &SiteSettings,
    def: &Definition,
) -> Result<bool, AppError> {
    let name = def.name.as_str();
    if !defs.change_plugin_enabled(name, settings) {
        return Ok(false);
    }
    Ok(match name {
        // Themes::Action::HorizonHighContextTopicCardsToggled: Horizon
        // enabled and the default or user selectable, with the setting the
        // legacy migration left.
        "enable_horizon_high_context_topic_cards" => {
            let default_theme = settings
                .get("default_theme_id")
                .map(Value::to_i)
                .unwrap_or_default();
            sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM themes t WHERE t.id = $1 AND t.enabled \
                   AND (t.user_selectable OR t.id = $2) \
                   AND EXISTS (SELECT 1 FROM theme_settings s WHERE s.theme_id = t.id \
                               AND s.name = 'topic_card_high_context'))",
            )
            .bind(HORIZON_THEME_ID)
            .bind(default_theme as i32)
            .fetch_one(&mut *conn)
            .await?
        }
        "remove_and_replace_uncategorized" => {
            truthy(settings, "allow_uncategorized_topics") || truthy(settings, name)
        }
        "enable_local_logins_via_code" => {
            current_value(conn, def).await?
                || truthy(settings, name)
                || (truthy(settings, "enable_local_logins")
                    && truthy(settings, "enable_local_logins_via_email")
                    && !truthy(settings, "enable_discourse_connect"))
        }
        "update_pending_users_reminder_default" => truthy(settings, "must_approve_users"),
        _ => true,
    })
}

/// `UpcomingChanges.change_dependencies_met?`
fn dependencies_met(settings: &SiteSettings, def: &Definition) -> bool {
    def.depends_on.iter().all(|dep| {
        let value = settings.get(dep).cloned().unwrap_or(Value::Null);
        match def.depends_on_values.get(dep) {
            Some(allowed) => allowed.contains(&value.to_s()),
            None => value == Value::Bool(true),
        }
    })
}

/// `UpcomingChanges.enabled_for_with_groups`: the dropdown's target and
/// the change's groups by name (comma separated, None without groups).
fn enabled_for_with_groups(
    def: &Definition,
    value: bool,
    group_ids: Option<&Vec<i32>>,
    names: &HashMap<i32, String>,
) -> (Json, Option<String>) {
    let group_ids = group_ids.filter(|ids| !ids.is_empty());
    let setting_groups = group_ids.map(|ids| {
        ids.iter()
            .map(|id| names.get(id).cloned().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(",")
    });
    let staff = || names.get(&STAFF_GROUP_ID).map_or(Json::Null, |n| json!(n));
    let enabled_for = if !value {
        json!("no_one")
    } else if setting_groups.as_deref().is_none_or(str::is_empty) {
        match allow_enabled_for(def) {
            None => json!("everyone"),
            Some(allow) if allow.iter().any(|a| a == "everyone") => json!("everyone"),
            Some(allow) if allow.iter().any(|a| a == "staff") => staff(),
            Some(_) => json!("groups"),
        }
    } else if group_ids.is_some_and(|ids| ids.as_slice() == [STAFF_GROUP_ID]) {
        staff()
    } else {
        json!("groups")
    };
    (enabled_for, setting_groups)
}

/// The plugin's humanized name, as `Discourse.plugins_by_name` has it.
fn plugin_humanized_name(plugin: &str) -> String {
    crate::admin_plugins::humanized_name(plugin).unwrap_or_else(|| plugin.to_string())
}

/// `UpcomingChanges.image_data`: the recorded image of the change, its
/// url under the site's base url.
fn image(base_url: &str, name: &str) -> Option<Json> {
    let (path, width, height) = crate::setting_enums::upcoming_change_image(name)?;
    Some(json!({
        "url": format!("{base_url}/{path}"),
        "width": width,
        "height": height,
    }))
}

/// UpcomingChanges::List's upcoming changes (past conceptual, displayed,
/// of the given statuses when any), sorted by setting.
pub async fn list(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    filter_statuses: &[String],
) -> Result<Vec<Json>, AppError> {
    let base_url = cx.urls.base_url()?;
    let group_ids = site_setting_group_ids(&mut *conn).await?;
    let mut wanted: Vec<i32> = group_ids.values().flatten().copied().collect();
    wanted.push(STAFF_GROUP_ID);
    let names: HashMap<i32, String> =
        sqlx::query_as::<_, (i32, String)>("SELECT id, name FROM groups WHERE id = ANY($1)")
            .bind(&wanted)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect();
    let mut out: Vec<(String, Json)> = Vec::new();
    for def in cx.defs.iter() {
        let Some(status) = def.upcoming_change else {
            continue;
        };
        if status == ChangeStatus::Conceptual {
            continue;
        }
        if !filter_statuses.is_empty() && !filter_statuses.iter().any(|s| s == status.as_str()) {
            continue;
        }
        if !should_display(&mut *conn, cx.defs, cx.settings, def).await? {
            continue;
        }
        let name = def.name.as_str();
        let value = truthy(cx.settings, name);
        let mut change = metadata(def);
        if let Some(image) = image(&base_url, name) {
            change.insert("image".into(), image);
        }
        let (enabled_for, groups) =
            enabled_for_with_groups(def, value, group_ids.get(name), &names);
        change.insert("enabled_for".into(), enabled_for);
        let mut entry = Map::new();
        entry.insert("setting".into(), json!(name));
        entry.insert(
            "humanized_name".into(),
            json!(crate::admin_site_settings::humanized_name(name)),
        );
        entry.insert(
            "description".into(),
            json!(crate::admin_site_settings::description(cx, name)),
        );
        entry.insert("value".into(), json!(value));
        entry.insert("upcoming_change".into(), Json::Object(change));
        if let Some(plugin) = &def.plugin {
            entry.insert("plugin".into(), json!(plugin_humanized_name(plugin)));
        }
        entry.insert("depends_on".into(), json!(def.depends_on));
        entry.insert(
            "depends_on_humanized_names".into(),
            json!(
                def.depends_on
                    .iter()
                    .map(|d| crate::admin_site_settings::humanized_name(d))
                    .collect::<Vec<_>>()
            ),
        );
        // DependencyGraph#dependents: the settings depending on it, in
        // the order they were declared.
        let dependents: Vec<&str> = cx
            .defs
            .iter()
            .filter(|d| d.depends_on.iter().any(|dep| dep == name))
            .map(|d| d.name.as_str())
            .collect();
        entry.insert("dependents".into(), json!(dependents));
        entry.insert(
            "depends_on_met".into(),
            json!(dependencies_met(cx.settings, def)),
        );
        entry.insert(
            "overriding_defaults".into(),
            json!(
                cx.defs
                    .iter()
                    .any(|d| d.default_override.as_ref().is_some_and(|(c, _)| c == name))
            ),
        );
        entry.insert("groups".into(), groups.map_or(Json::Null, |g| json!(g)));
        out.push((name.to_string(), Json::Object(entry)));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out.into_iter().map(|(_, e)| e).collect())
}

/// UpcomingChanges::List#update_last_visited: the admin's
/// `last_visited_upcoming_changes_at`, unless the system user or a bot.
pub async fn update_last_visited(conn: &mut PgConnection, user_id: i32) -> Result<(), AppError> {
    if user_id <= 0 {
        return Ok(());
    }
    let now = crate::clock::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    crate::user_fields::save_custom_field(
        conn,
        user_id,
        "last_visited_upcoming_changes_at",
        &json!(now),
    )
    .await
}

/// UpcomingChanges::Action::BackfillNotifiedEvents: the changes marked
/// added, and notified about as far as their status reaches, without
/// notifying anyone (the once-off events a change already has kept).
pub async fn backfill_notified_events(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    defs: &Definitions,
    names: &[&str],
) -> Result<(), AppError> {
    let promote = settings.get("promote_upcoming_changes_on_status")?.to_s();
    let promote = ChangeStatus::parse(&promote).unwrap_or(ChangeStatus::Stable);
    // UpcomingChanges.previous_status: the status ranked just before.
    let available = [
        ChangeStatus::Conceptual,
        ChangeStatus::Experimental,
        ChangeStatus::Alpha,
        ChangeStatus::Beta,
        ChangeStatus::Stable,
        ChangeStatus::Permanent,
        ChangeStatus::Never,
    ]
    .into_iter()
    .rfind(|s| *s < promote)
    .unwrap_or(ChangeStatus::Conceptual);
    for name in names {
        let Some(status) = defs.change_status(name) else {
            continue;
        };
        let mut types = vec![event_types::ADDED];
        if status >= promote {
            types.push(event_types::ADMINS_NOTIFIED_AUTOMATIC_PROMOTION);
        } else if status >= available {
            types.push(event_types::ADMINS_NOTIFIED_AVAILABLE_CHANGE);
        }
        for event_type in types {
            sqlx::query(
                "INSERT INTO upcoming_change_events (event_type, event_data, upcoming_change_name, \
                 created_at, updated_at) VALUES ($1, '{\"backfilled\":true}'::jsonb, $2, now(), now()) \
                 ON CONFLICT (upcoming_change_name, event_type) \
                 WHERE event_type = ANY (ARRAY[0, 1, 6, 7]) DO NOTHING",
            )
            .bind(event_type)
            .bind(name)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Definitions {
        Definitions::vendored().expect("vendored settings")
    }

    #[test]
    fn metadata_normalizes_the_declared_mapping() {
        let defs = defs();
        let def = defs.get("remove_and_replace_uncategorized").unwrap();
        let m = metadata(def);
        assert_eq!(m["status"], "stable");
        assert_eq!(m["impact_type"], "other");
        assert_eq!(m["impact_role"], "all_members");
        assert_eq!(m["allow_enabled_for"], json!(["everyone"]));
        assert_eq!(
            m["hide_settings"],
            json!(["allow_uncategorized_topics", "suppress_uncategorized_badge"])
        );
        assert_eq!(m["permanent_warning"], true);
        assert_eq!(m["body_class"], Json::Null);
        assert!(!m.contains_key("impact"));
    }

    #[test]
    fn targets_follow_allow_enabled_for() {
        let defs = defs();
        let limited = defs.get("remove_and_replace_uncategorized").unwrap();
        assert!(target_allowed(limited, "everyone"));
        assert!(target_allowed(limited, "no_one"));
        assert!(!target_allowed(limited, "staff"));
        assert!(!groups_target_allowed(limited));
        let open = defs.get("enable_unified_new").unwrap();
        assert!(target_allowed(open, "specific_groups"));
        assert!(groups_target_allowed(open));
    }

    #[test]
    fn enabled_for_reads_the_groups() {
        let defs = defs();
        let def = defs.get("enable_unified_new").unwrap();
        let names: HashMap<i32, String> =
            [(3, "staff".to_string()), (11, "trust_level_1".to_string())].into();
        assert_eq!(
            enabled_for_with_groups(def, false, None, &names),
            (json!("no_one"), None)
        );
        assert_eq!(
            enabled_for_with_groups(def, true, None, &names),
            (json!("everyone"), None)
        );
        assert_eq!(
            enabled_for_with_groups(def, true, Some(&vec![3]), &names),
            (json!("staff"), Some("staff".into()))
        );
        assert_eq!(
            enabled_for_with_groups(def, true, Some(&vec![3, 11]), &names),
            (json!("groups"), Some("staff,trust_level_1".into()))
        );
    }
}
