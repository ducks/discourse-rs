//! UpcomingChanges (lib/upcoming_changes.rb) as the upcoming changes page
//! reads and writes it: UpcomingChanges::List's entries (the metadata
//! site_settings.yml declares, ConditionalDisplay, the enabled-for target
//! and its groups) and the site_setting_groups an opt-in group list keeps.

use std::collections::HashMap;

use serde_json::{Map, Value as Json, json};
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::AppError;
use crate::admin::site_settings::Context;
use crate::site_settings::{ChangeStatus, Definition, Definitions, SiteSettings, Value};

/// `Group::AUTO_GROUPS[:staff]`
pub const STAFF_GROUP_ID: i32 = 3;
const ADMINS_GROUP_ID: i32 = 1;
const MODERATORS_GROUP_ID: i32 = 2;
const ANONYMOUS_USERS_GROUP_ID: i32 = 4;
const LOGGED_IN_USERS_GROUP_ID: i32 = 5;

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
        if matches!(
            key,
            "impact" | "allow_enabled_for" | "hide_settings" | "group_setting"
        ) {
            continue;
        }
        out.insert(key.to_string(), yaml_json(value));
    }
    out.insert(
        "group_setting".into(),
        group_setting_name(def).map_or(Json::Null, |n| json!(n)),
    );
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

/// Whether `allow_enabled_for` is exactly staff plus specific_groups (the
/// audience must then include staff).
pub fn staff_and_specific_groups(def: &Definition) -> bool {
    allow_enabled_for(def).is_some_and(|mut allow| {
        allow.sort();
        allow == ["specific_groups", "staff"]
    })
}

/// `UpcomingChanges.group_setting_name`: the group_list setting the
/// change uses as its audience instead of site_setting_groups.
pub fn group_setting_name(def: &Definition) -> Option<&str> {
    declared(def)?
        .get(Yaml::String("group_setting".into()))?
        .as_str()
}

/// `UpcomingChanges.group_setting_data`: the linked setting, when it is a
/// group_list setting, with what the audience picker needs to respect.
pub fn group_setting_data(defs: &Definitions, settings: &SiteSettings, def: &Definition) -> Json {
    let Some(linked) = group_setting_name(def).and_then(|n| defs.get(n)) else {
        return Json::Null;
    };
    if linked.data_type != crate::site_settings::DataType::GroupList {
        return Json::Null;
    }
    let requires_confirmation = linked
        .options
        .as_ref()
        .and_then(|o| o.get(Yaml::String("requires_confirmation".into())))
        .map_or(Json::Null, yaml_json);
    json!({
        "setting": linked.name,
        "value": settings.get(&linked.name).map(Value::to_s).unwrap_or_default(),
        "mandatory_values": linked.mandatory_values,
        "disallowed_groups": crate::site_setting_lists::disallowed_groups(linked),
        "requires_confirmation": requires_confirmation,
    })
}

/// `UpcomingChanges.group_ids_for`: the linked setting's groups, else
/// (unless permanent) the site_setting_groups row's.
pub fn group_ids_for(
    def: &Definition,
    settings: &SiteSettings,
    rows: &HashMap<String, Vec<i32>>,
) -> Vec<i32> {
    if let Some(linked) = group_setting_name(def) {
        return settings
            .group_ids(linked)
            .unwrap_or_default()
            .into_iter()
            .map(|id| id as i32)
            .collect();
    }
    if def.upcoming_change == Some(ChangeStatus::Permanent) {
        return Vec::new();
    }
    rows.get(&def.name).cloned().unwrap_or_default()
}

/// `UpcomingChanges.has_groups?`
pub fn has_groups(def: &Definition, group_ids: &[i32]) -> bool {
    group_setting_name(def).is_some() || !group_ids.is_empty()
}

/// `ROLE_GROUP_KEYS` by the change's impact role: the automatic groups
/// the admin UI defaults the audience to.
fn role_group_ids(def: &Definition) -> &'static [i32] {
    let role = declared(def)
        .and_then(|y| y.get(Yaml::String("impact".into())))
        .and_then(Yaml::as_str)
        .and_then(|i| i.split(',').nth(1))
        .unwrap_or_default();
    match role {
        "staff" => &[STAFF_GROUP_ID],
        "admins" | "developers" => &[ADMINS_GROUP_ID],
        "moderators" => &[MODERATORS_GROUP_ID],
        _ => &[LOGGED_IN_USERS_GROUP_ID, ANONYMOUS_USERS_GROUP_ID],
    }
}

/// `UpcomingChanges.normalize_group_names`: blanks and repeats dropped,
/// then the full default audience (logged in plus anonymous users, or the
/// impact role's groups) stored as no restriction.
pub async fn normalize_group_names(
    conn: &mut PgConnection,
    def: Option<&Definition>,
    names: Vec<String>,
) -> Result<Vec<String>, sqlx::Error> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if !crate::ruby::is_blank(&name) && !out.contains(&name) {
            out.push(name);
        }
    }
    let Some(def) = def.filter(|d| {
        d.is_upcoming_change() && group_setting_name(d).is_none() && target_allowed(d, "everyone")
    }) else {
        return Ok(out);
    };
    let group_names = |ids: Vec<i32>| {
        sqlx::query_scalar::<_, String>("SELECT name FROM groups WHERE id = ANY($1)").bind(ids)
    };
    let everyone = group_names(vec![LOGGED_IN_USERS_GROUP_ID, ANONYMOUS_USERS_GROUP_ID])
        .fetch_all(&mut *conn)
        .await?;
    if everyone.len() == 2 && everyone.iter().all(|n| out.contains(n)) {
        return Ok(Vec::new());
    }
    let mut role = group_names(role_group_ids(def).to_vec())
        .fetch_all(&mut *conn)
        .await?;
    let mut sorted = out.clone();
    role.sort();
    sorted.sort();
    Ok(if !role.is_empty() && role == sorted {
        Vec::new()
    } else {
        out
    })
}

/// `UpcomingChanges.group_scope_allowed?`: an audience of these group
/// names fits `allow_enabled_for` (none is everyone, the staff group
/// alone is staff).
pub fn group_scope_allowed(def: &Definition, names: &[String], staff_name: &str) -> bool {
    let staff = names.iter().any(|n| n == staff_name);
    if allow_enabled_for(def)
        .is_some_and(|a| a.iter().any(|t| t == "staff") && a.iter().any(|t| t == "specific_groups"))
        && !staff
    {
        return false;
    }
    let target = if names.is_empty() {
        "everyone"
    } else if names.len() == 1 && staff {
        "staff"
    } else {
        "specific_groups"
    };
    target_allowed(def, target)
}

/// The staff group's (localized) name.
pub async fn staff_group_name(conn: &mut PgConnection) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT name FROM groups WHERE id = $1")
        .bind(STAFF_GROUP_ID)
        .fetch_one(conn)
        .await
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
    group_ids: &[i32],
    names: &HashMap<i32, String>,
) -> (Json, Option<String>) {
    let staff_only = group_ids == [STAFF_GROUP_ID];
    let group_ids = Some(group_ids).filter(|ids| !ids.is_empty());
    let setting_groups = group_ids.map(|ids| {
        ids.iter()
            .map(|id| names.get(id).cloned().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(",")
    });
    let staff = || names.get(&STAFF_GROUP_ID).map_or(Json::Null, |n| json!(n));
    let enabled_for = if !value {
        json!("no_one")
    } else if group_setting_name(def).is_some() {
        let ids = group_ids.unwrap_or_default();
        if ids.contains(&ANONYMOUS_USERS_GROUP_ID) && ids.contains(&LOGGED_IN_USERS_GROUP_ID) {
            json!("everyone")
        } else if staff_only {
            staff()
        } else {
            json!("groups")
        }
    } else if setting_groups.as_deref().is_none_or(str::is_empty) {
        match allow_enabled_for(def) {
            _ if def.upcoming_change == Some(ChangeStatus::Permanent) => json!("everyone"),
            None => json!("everyone"),
            Some(allow) if allow.iter().any(|a| a == "everyone") => json!("everyone"),
            Some(allow) if allow.iter().any(|a| a == "staff") => staff(),
            Some(_) => json!("groups"),
        }
    } else if staff_only {
        staff()
    } else {
        json!("groups")
    };
    (enabled_for, setting_groups)
}

/// The plugin's humanized name, as `Discourse.plugins_by_name` has it.
fn plugin_humanized_name(plugin: &str) -> String {
    crate::admin::plugins::humanized_name(plugin).unwrap_or_else(|| plugin.to_string())
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
    let rows = site_setting_group_ids(&mut *conn).await?;
    let group_ids: HashMap<&str, Vec<i32>> = cx
        .defs
        .iter()
        .filter(|d| d.is_upcoming_change())
        .map(|d| (d.name.as_str(), group_ids_for(d, cx.settings, &rows)))
        .collect();
    let mut wanted: Vec<i32> = group_ids.values().flatten().copied().collect();
    wanted.push(STAFF_GROUP_ID);
    // load_status_changed_at: each change's latest status change.
    let status_changed_at: HashMap<String, chrono::NaiveDateTime> = sqlx::query_as(
        "SELECT upcoming_change_name, MAX(created_at) FROM upcoming_change_events \
         WHERE event_type = $1 GROUP BY upcoming_change_name",
    )
    .bind(event_types::STATUS_CHANGED)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
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
        let ids = group_ids.get(name).map(Vec::as_slice).unwrap_or_default();
        let (enabled_for, groups) = enabled_for_with_groups(def, value, ids, &names);
        change.insert("enabled_for".into(), enabled_for);
        change.insert(
            "status_changed_at".into(),
            status_changed_at
                .get(name)
                .map_or(Json::Null, |t| json!(crate::topics::list::time_json(*t))),
        );
        let mut entry = Map::new();
        entry.insert("setting".into(), json!(name));
        entry.insert(
            "humanized_name".into(),
            json!(crate::admin::site_settings::humanized_name(name)),
        );
        entry.insert(
            "description".into(),
            json!(crate::admin::site_settings::description(cx, name)),
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
                    .map(|d| crate::admin::site_settings::humanized_name(d))
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
        if group_setting_name(def).is_some() {
            entry.insert(
                "group_setting".into(),
                group_setting_data(cx.defs, cx.settings, def),
            );
        }
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
            enabled_for_with_groups(def, false, &[], &names),
            (json!("no_one"), None)
        );
        assert_eq!(
            enabled_for_with_groups(def, true, &[], &names),
            (json!("everyone"), None)
        );
        assert_eq!(
            enabled_for_with_groups(def, true, &[3], &names),
            (json!("staff"), Some("staff".into()))
        );
        assert_eq!(
            enabled_for_with_groups(def, true, &[3, 11], &names),
            (json!("groups"), Some("staff,trust_level_1".into()))
        );
    }

    #[test]
    fn group_scope_follows_allow_enabled_for() {
        let defs = defs();
        let staff = "staff".to_string();
        let open = defs.get("enable_unified_new").unwrap();
        assert!(group_scope_allowed(open, &[], "staff"));
        assert!(group_scope_allowed(
            open,
            std::slice::from_ref(&staff),
            "staff"
        ));
        let limited = defs.get("remove_and_replace_uncategorized").unwrap();
        assert!(group_scope_allowed(limited, &[], "staff"));
        assert!(!group_scope_allowed(limited, &[staff], "staff"));
        assert!(!staff_and_specific_groups(limited));
    }

    #[test]
    fn a_linked_group_setting_is_the_audience() {
        let defs = defs();
        let live = defs.get("live_enabled").unwrap();
        assert_eq!(group_setting_name(live), Some("live_allowed_groups"));
        assert_eq!(metadata(live)["group_setting"], "live_allowed_groups");
        assert_eq!(
            metadata(defs.get("enable_unified_new").unwrap())["group_setting"],
            Json::Null
        );
        let names: HashMap<i32, String> = [
            (3, "staff".to_string()),
            (4, "anonymous_users".to_string()),
            (5, "logged_in_users".to_string()),
        ]
        .into();
        assert_eq!(
            enabled_for_with_groups(live, true, &[4, 5], &names),
            (
                json!("everyone"),
                Some("anonymous_users,logged_in_users".into())
            )
        );
        assert_eq!(
            enabled_for_with_groups(live, true, &[5], &names).0,
            json!("groups")
        );
        assert_eq!(
            enabled_for_with_groups(live, true, &[3], &names).0,
            json!("staff")
        );
        assert!(has_groups(live, &[]));
        assert!(!has_groups(defs.get("enable_unified_new").unwrap(), &[]));
    }
}
