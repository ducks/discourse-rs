//! Admin::Config::UpcomingChangesController: the upcoming changes page
//! (90g6) and its JSON (UpcomingChanges::List), the opt-in toggle
//! (UpcomingChanges::Toggle) and the opt-in groups
//! (SiteSetting::UpsertGroups). Staff reach it (StaffConstraint); the
//! services' policies leave moderators an InvalidAccess.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sqlx::PgConnection;

use super::admin_pages::{Shell, wants_json};
use super::session::bad_csrf;
use crate::session::current::AuthGuardian;
use crate::site_settings::{Definition, SiteSettings, Value as SettingValue};
use crate::upcoming_changes::{self as changes, event_types};
use crate::{AppError, AppState, Unsupported, params};

/// The statuses the page lists (the route's `filter_statuses`).
const PAGE_STATUSES: [&str; 4] = ["experimental", "alpha", "beta", "stable"];

fn failed_contract(errors: Vec<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "failed": "FAILED", "errors": errors })),
    )
        .into_response()
}

fn success() -> Response {
    Json(json!({ "success": "OK" })).into_response()
}

/// ActiveModel's boolean cast: blank is nil, then "false", "f", "0",
/// "off" (any case) are false and everything else true.
fn cast_bool(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::Null => None,
        Value::Number(n) => Some(n.as_f64() != Some(0.0)),
        other => {
            let s = params::scalar(other)?;
            if s.is_empty() {
                return None;
            }
            Some(!matches!(
                s.to_lowercase().as_str(),
                "false" | "f" | "0" | "off"
            ))
        }
    }
}

/// The settings and the labels' context, for the list.
async fn list_for(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    statuses: &[String],
) -> Result<Vec<Value>, AppError> {
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let cx = crate::admin_site_settings::Context {
        defs: &state.site_setting_defs,
        settings,
        i18n: &state.i18n,
        globals: &state.config.globals,
        base_path: state.config.globals.relative_url_root(),
        urls: &urls,
    };
    changes::list(conn, &cx, statuses).await
}

/// GET /admin/config/upcoming-changes(.json): the list for the page's
/// xhr, else the page.
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(user_id) = guardian.user_id() else {
        return Ok(super::topics::not_found_response(&state));
    };
    let mut conn = state.pool.acquire().await?;
    if wants_json(&uri, &headers) {
        let p = params::parse(uri.query(), &headers, &body);
        let statuses: Vec<String> = params::string(&p, "filter_statuses")
            .map(|s| s.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        let list = list_for(&mut conn, &state, &settings, &statuses).await?;
        changes::update_last_visited(&mut conn, user_id).await?;
        return Ok(Json(json!({ "upcoming_changes": list })).into_response());
    }
    let shell = Shell::load(
        &mut conn,
        &state,
        &guardian,
        &headers,
        "admin_upcoming_changes",
    )
    .await?;
    let statuses: Vec<String> = PAGE_STATUSES.iter().map(|s| s.to_string()).collect();
    let list = list_for(&mut conn, &state, &shell.settings, &statuses).await?;
    changes::update_last_visited(&mut conn, user_id).await?;
    let cx = shell.cx(&state);
    let query = params::parse_query(uri.query().unwrap_or(""));
    let view = crate::admin_upcoming_changes_view::Filters {
        text: params::string(&query, "changeNamesFilter").unwrap_or_default(),
        status: params::string(&query, "status"),
        kind: params::string(&query, "type"),
        impact_role: params::string(&query, "impactRole"),
        enabled: params::string(&query, "enabled"),
    };
    let content = crate::admin_upcoming_changes_view::page(&cx, &list, &view);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-upcoming-changes-strings",
        crate::admin_upcoming_changes_view::STRINGS,
    );
    let main = crate::admin_view::contents(
        "admin-config upcoming-changes",
        &format!("{content}{strings}"),
    );
    let title = crate::admin_view::t(cx.i18n, "admin.config.upcoming_changes.title");
    shell.render(
        &state,
        &uri,
        &title,
        main,
        "admin-upcoming-changes.js",
        StatusCode::OK,
    )
}

/// The change's setting, when it is an upcoming change.
fn change_def<'a>(state: &'a AppState, name: &str) -> Option<&'a Definition> {
    state
        .site_setting_defs
        .get(name)
        .filter(|d| d.is_upcoming_change())
}

/// `SiteSetting.<change> = enabled` (add_override!): the override saved;
/// when the stored value changed, the clients told (unless the change has
/// groups) and the site_setting_changed handlers run.
async fn set_change(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    def: &Definition,
    enabled: bool,
) -> Result<(), AppError> {
    let old = changes::current_value(&mut *conn, def).await?;
    crate::site_setting_update::save_override(&mut *conn, def, if enabled { "t" } else { "f" })
        .await?;
    if old == enabled {
        return Ok(());
    }
    let name = def.name.as_str();
    let has_groups = changes::site_setting_group_ids(&mut *conn)
        .await?
        .get(name)
        .is_some_and(|ids| !ids.is_empty());
    if def.client && !has_groups {
        let fresh =
            SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
        state
            .bus
            .publish(
                &mut *conn,
                "/client_settings",
                &json!({ "name": name, "value": fresh.get(name)?, "scoped_to": null }),
                None,
            )
            .await?;
    }
    // A plugin's enabled_site_setting: its plugin's handlers, and (when
    // turned on) its changes marked as already notified about.
    let Some(plugin) = def.plugin.as_deref() else {
        return Ok(());
    };
    if state.site_setting_defs.enabled_setting_of(plugin) != Some(name) {
        return Ok(());
    }
    match name {
        "voice_enabled" => {
            return Err(Unsupported("toggling voice (its plugin's enable handlers)").into());
        }
        "enable_discourse_workflows" if enabled => {
            return Err(Unsupported(
                "enabling workflows (DiscourseWorkflows::PluginEnableHandler)",
            )
            .into());
        }
        _ => {}
    }
    if enabled {
        let plugin_changes: Vec<&str> = state
            .site_setting_defs
            .upcoming_change_names()
            .into_iter()
            .filter(|n| {
                state
                    .site_setting_defs
                    .get(n)
                    .is_some_and(|d| d.plugin.as_deref() == Some(plugin))
            })
            .collect();
        changes::backfill_notified_events(
            &mut *conn,
            settings,
            &state.site_setting_defs,
            &plugin_changes,
        )
        .await?;
    }
    Ok(())
}

/// SiteSetting::Action::RemoveAndReplaceUncategorizedToggled: enabling
/// demotes the special Uncategorized category in place (the site's state
/// snapshotted onto the opt-in event first), disabling restores the
/// snapshot's state.
async fn uncategorized_toggled(
    conn: &mut PgConnection,
    state: &AppState,
    enabled: bool,
) -> Result<(), AppError> {
    const NAME: &str = "remove_and_replace_uncategorized";
    const SKIP_DEFINITION: &str = "skip_category_definition";
    let defs = &state.site_setting_defs;
    let settings = SiteSettings::load(&mut *conn, defs, &state.config.globals).await?;
    // `order(created_at: :desc).first` follows UpcomingChangeEvent's
    // default_scope `order(:created_at)`, so the oldest event is the one.
    let snapshot: Option<(i64, Value)> = sqlx::query_as(
        "SELECT id, event_data FROM upcoming_change_events WHERE upcoming_change_name = $1 \
         AND event_type = ANY($2) AND event_data IS NOT NULL ORDER BY created_at, id LIMIT 1",
    )
    .bind(NAME)
    .bind([
        event_types::MANUAL_OPT_IN,
        event_types::AUTOMATICALLY_PROMOTED,
    ])
    .fetch_optional(&mut *conn)
    .await?;
    // set_and_log by the system user, a no-op when the value is unchanged.
    async fn set(
        conn: &mut PgConnection,
        state: &AppState,
        name: &str,
        value: SettingValue,
    ) -> Result<(), AppError> {
        let settings =
            SiteSettings::load(&mut *conn, &state.site_setting_defs, &state.config.globals).await?;
        let previous = settings.get(name)?.clone();
        if previous == value {
            return Ok(());
        }
        let def = state
            .site_setting_defs
            .get(name)
            .ok_or_else(|| crate::site_settings::SettingError::Unknown(name.into()))?;
        // SiteSettingExtension#log leaves out hidden settings, those an
        // enabled change hides too.
        if changes::hidden_while_enabled(&state.site_setting_defs, &settings, name) {
            let raw = serde_json::to_value(&value)?;
            return match crate::site_setting_update::set(
                &mut *conn,
                &state.bus,
                &settings,
                &state.site_setting_defs,
                &state.i18n,
                name,
                &raw,
            )
            .await?
            {
                Ok(_) => Ok(()),
                Err(message) => Err(std::io::Error::other(message).into()),
            };
        }
        crate::site_setting_update::set_and_log(&mut *conn, &state.bus, def, &previous, &value, -1)
            .await
    }
    if enabled {
        let former = settings.get("uncategorized_category_id")?.to_i();
        if former == -1 {
            return Ok(());
        }
        if snapshot.is_none() {
            let data = json!({
                "allow_uncategorized_topics": settings.get("allow_uncategorized_topics")?,
                "default_composer_category": settings.get("default_composer_category")?,
                "uncategorized_category_id": former,
            });
            sqlx::query(
                "UPDATE upcoming_change_events SET event_data = $2, updated_at = now() WHERE id = \
                 (SELECT id FROM upcoming_change_events WHERE upcoming_change_name = $1 \
                  AND event_type = ANY($3) ORDER BY created_at, id LIMIT 1)",
            )
            .bind(NAME)
            .bind(&data)
            .bind([
                event_types::MANUAL_OPT_IN,
                event_types::AUTOMATICALLY_PROMOTED,
            ])
            .execute(&mut *conn)
            .await?;
        }
        if settings.get("default_composer_category")?.is_blank() {
            set(
                conn,
                state,
                "default_composer_category",
                SettingValue::Str(former.to_string()),
            )
            .await?;
        }
        set(
            conn,
            state,
            "uncategorized_category_id",
            SettingValue::Int(-1),
        )
        .await?;
        set(
            conn,
            state,
            "allow_uncategorized_topics",
            SettingValue::Bool(false),
        )
        .await?;
        // upsert_custom_fields: the row updated, else created.
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
                .bind(former as i32)
                .fetch_one(&mut *conn)
                .await?;
        if exists {
            let updated = sqlx::query(
                "UPDATE category_custom_fields SET value = 't', updated_at = now() \
                 WHERE category_id = $1 AND name = $2",
            )
            .bind(former as i32)
            .bind(SKIP_DEFINITION)
            .execute(&mut *conn)
            .await?
            .rows_affected();
            if updated == 0 {
                sqlx::query(
                    "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
                     VALUES ($1, $2, 't', now(), now())",
                )
                .bind(former as i32)
                .bind(SKIP_DEFINITION)
                .execute(&mut *conn)
                .await?;
            }
        }
        return Ok(());
    }
    let Some((_, snapshot)) = snapshot else {
        return Ok(());
    };
    if snapshot["allow_uncategorized_topics"] != true {
        return Ok(());
    }
    let former = snapshot["uncategorized_category_id"].as_i64().unwrap_or(-1);
    set(
        conn,
        state,
        "uncategorized_category_id",
        SettingValue::Int(former),
    )
    .await?;
    set(
        conn,
        state,
        "allow_uncategorized_topics",
        SettingValue::Bool(true),
    )
    .await?;
    let default = match &snapshot["default_composer_category"] {
        Value::Number(n) => SettingValue::Int(n.as_i64().unwrap_or_default()),
        other => SettingValue::Str(params::scalar(other).unwrap_or_default()),
    };
    set(conn, state, "default_composer_category", default).await?;
    sqlx::query("DELETE FROM category_custom_fields WHERE category_id = $1 AND name = $2")
        .bind(former as i32)
        .bind(SKIP_DEFINITION)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// PUT /admin/config/upcoming-changes/toggle(.json)
pub async fn toggle(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = super::admin_config_about::params_of(&state, &headers, &uri, &body, "PUT") else {
        return Ok(bad_csrf());
    };
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    // The contract.
    let name = params::string(&p, "setting_name").unwrap_or_default();
    let enabled = cast_bool(p.get("enabled"));
    let mut errors = Vec::new();
    if crate::ruby::is_blank(&name) {
        errors.push("Setting name can't be blank".to_string());
    }
    if enabled.is_none() {
        errors.push("Enabled is not included in the list".to_string());
    }
    let Some(enabled) = enabled.filter(|_| errors.is_empty()) else {
        return Ok(failed_contract(errors));
    };
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(def) = change_def(&state, &name) else {
        return Ok(super::topics::not_found_response(&state));
    };
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("toggling a change without a user"))?;
    let mut tx = state.pool.begin().await?;
    // allowed_enabled_for_target: enabling without groups means everyone.
    if enabled {
        let has_groups: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM site_setting_groups WHERE name = $1)")
                .bind(&name)
                .fetch_one(&mut *tx)
                .await?;
        if !has_groups && !changes::target_allowed(def, "everyone") {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "failed": "FAILED" })),
            )
                .into_response());
        }
    }
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    let previous = settings.get(&name)?.truthy();
    set_change(&mut tx, &state, &settings, def, enabled).await?;
    if !changes::groups_target_allowed(def) {
        sqlx::query("DELETE FROM site_setting_groups WHERE name = $1")
            .bind(&name)
            .execute(&mut *tx)
            .await?;
    }
    // StaffActionLogger#log_upcoming_change_toggle
    let context = state
        .i18n
        .t("staff_action_logs.upcoming_changes.log_manually_toggled")
        .unwrap_or_default()
        .to_string();
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, \
         context, admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, clock_timestamp(), clock_timestamp())",
    )
    .bind(crate::staff_action_logs::action_id(
        "upcoming_change_toggled",
    ))
    .bind(acting)
    .bind(&name)
    .bind(previous.to_string())
    .bind(enabled.to_string())
    .bind(context)
    .bind(crate::staff_action_logs::admin_only(
        "upcoming_change_toggled",
        &settings,
    )?)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO upcoming_change_events (event_type, upcoming_change_name, acting_user_id, \
         created_at, updated_at) VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
    )
    .bind(if enabled {
        event_types::MANUAL_OPT_IN
    } else {
        event_types::MANUAL_OPT_OUT
    })
    .bind(&name)
    .bind(acting)
    .execute(&mut *tx)
    .await?;
    // The :upcoming_change_enabled and :upcoming_change_disabled handlers.
    match name.as_str() {
        "enable_horizon_high_context_topic_cards" => {
            super::themes::horizon_high_context_toggled(&mut tx, &state, enabled).await?;
        }
        "remove_and_replace_uncategorized" => {
            uncategorized_toggled(&mut tx, &state, enabled).await?;
        }
        _ => {}
    }
    tx.commit().await?;
    Ok(success())
}

/// ActiveSupportTypeExtensions::Array's cast of `group_names`, blanks
/// dropped (the contract's before_validation).
fn group_names(v: Option<&Value>) -> Vec<String> {
    let items: Vec<String> = match v {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.iter().filter_map(params::scalar).collect(),
        Some(Value::Object(map)) => map.values().filter_map(params::scalar).collect(),
        Some(other) => {
            let s = params::scalar(other).unwrap_or_default();
            // `split(/,(?!.*\|)|\|(?!.*,)/)`: on commas after the last pipe,
            // or pipes after the last comma.
            let last_pipe = s.rfind('|');
            let last_comma = s.rfind(',');
            let mut parts = Vec::new();
            let mut start = 0;
            for (i, c) in s.char_indices() {
                let split = match c {
                    ',' => last_pipe.is_none_or(|p| p < i),
                    '|' => last_comma.is_none_or(|p| p < i),
                    _ => false,
                };
                if split {
                    parts.push(s[start..i].to_string());
                    start = i + 1;
                }
            }
            parts.push(s[start..].to_string());
            // String#split drops the trailing empties.
            while parts.last().is_some_and(String::is_empty) {
                parts.pop();
            }
            parts
        }
    };
    items.into_iter().filter(|s| !s.is_empty()).collect()
}

/// PUT /admin/config/upcoming-changes/groups(.json)
pub async fn groups(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let Some(p) = super::admin_config_about::params_of(&state, &headers, &uri, &body, "PUT") else {
        return Ok(bad_csrf());
    };
    if !guardian.is_staff() {
        return Ok(super::topics::not_found_response(&state));
    }
    let setting = params::string(&p, "setting").unwrap_or_default();
    if crate::ruby::is_blank(&setting) {
        return Ok(failed_contract(vec!["Setting can't be blank".into()]));
    }
    if !guardian.is_admin() {
        return Ok(super::search::invalid_access(&state));
    }
    let names = group_names(p.get("group_names"));
    // allowed_enabled_for_target; a failure has no handler (no content).
    if !names.is_empty() {
        let target = if names == ["staff"] {
            "staff"
        } else {
            "specific_groups"
        };
        let allowed = state
            .site_setting_defs
            .get(&setting)
            .is_none_or(|def| changes::target_allowed(def, target));
        if !allowed {
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }
    let acting = guardian
        .user_id()
        .ok_or(Unsupported("changing groups without a user"))?;
    // log_site_setting_groups_change raises for an unknown setting.
    if state.site_setting_defs.get(&setting).is_none() {
        return Ok(super::search::invalid_parameters(&state, "setting_name"));
    }
    let mut tx = state.pool.begin().await?;
    let previous: Option<Option<String>> =
        sqlx::query_scalar("SELECT group_ids FROM site_setting_groups WHERE name = $1")
            .bind(&setting)
            .fetch_optional(&mut *tx)
            .await?;
    let new_value = if names.is_empty() {
        sqlx::query("DELETE FROM site_setting_groups WHERE name = $1")
            .bind(&setting)
            .execute(&mut *tx)
            .await?;
        String::new()
    } else {
        let mut ids: Vec<i32> = sqlx::query_scalar("SELECT id FROM groups WHERE name = ANY($1)")
            .bind(&names)
            .fetch_all(&mut *tx)
            .await?;
        if ids.is_empty() {
            return Ok(super::topics::not_found_response(&state));
        }
        ids.sort_unstable();
        let value = ids.iter().map(i32::to_string).collect::<Vec<_>>().join("|");
        sqlx::query(
            "INSERT INTO site_setting_groups (name, group_ids, created_at, updated_at) \
             VALUES ($1, $2, now(), now()) \
             ON CONFLICT (name) DO UPDATE SET group_ids = EXCLUDED.group_ids, updated_at = now()",
        )
        .bind(&setting)
        .bind(&value)
        .execute(&mut *tx)
        .await?;
        value
    };
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    // StaffActionLogger#log_site_setting_groups_change
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, \
         admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp(), clock_timestamp())",
    )
    .bind(crate::staff_action_logs::action_id(
        "change_site_setting_groups",
    ))
    .bind(acting)
    .bind(&setting)
    .bind(previous.flatten())
    .bind(new_value)
    .bind(crate::staff_action_logs::admin_only(
        "change_site_setting_groups",
        &settings,
    )?)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(success())
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::{get, put};
    for suffix in ["", ".json"] {
        router = router
            .route(
                &format!("/admin/config/upcoming-changes{suffix}"),
                get(index),
            )
            .route(
                &format!("/admin/config/upcoming-changes/toggle{suffix}"),
                put(toggle),
            )
            .route(
                &format!("/admin/config/upcoming-changes/groups{suffix}"),
                put(groups),
            );
    }
    router
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_names_split_as_the_array_type_does() {
        assert_eq!(group_names(Some(&json!(["staff", ""]))), vec!["staff"]);
        assert_eq!(group_names(Some(&json!("a,b"))), vec!["a", "b"]);
        assert_eq!(group_names(Some(&json!("a|b"))), vec!["a", "b"]);
        assert!(group_names(Some(&json!(""))).is_empty());
        assert!(group_names(None).is_empty());
    }

    #[test]
    fn enabled_casts_as_activemodel() {
        assert_eq!(cast_bool(Some(&json!("true"))), Some(true));
        assert_eq!(cast_bool(Some(&json!("false"))), Some(false));
        assert_eq!(cast_bool(Some(&json!(""))), None);
        assert_eq!(cast_bool(None), None);
    }
}
