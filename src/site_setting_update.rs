//! `SiteSetting::Update` for one setting (`Admin::SiteSettingsController
//! #update`): the value cast for its type, the policies, TypeSupervisor's
//! validation, then `set_and_log` (the override saved, the
//! change_site_setting staff log).
//!
//! Ported for core's and the bundled plugins' settings of the plain types
//! (string, integer, float, bool, enum with listed choices), a plugin
//! setting's change running the handlers of the plugins the port
//! has started (crate::plugins::site_setting_changed). Refused: enabling a
//! bundled plugin the port has not started (unless the reference has
//! it on by default), themeable and upcoming change settings, the other
//! types (lists, uploads, groups,
//! categories, objects...), settings with a validator class or a custom
//! validation, default user preferences (their backfill), client strings
//! with markup (sanitize_field), settings shadowed by a global setting,
//! bulk updates, an archived site, and the settings whose change handlers
//! write (the welcome topic for title and site_description, approving
//! users, emoji links, slugs, review priorities, site icons...).

use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::{DataType, Definitions, SiteSettings, Value};
use crate::{AppError, Unsupported};

/// `UserHistory.actions[:change_site_setting]`
const CHANGE_SITE_SETTING: i32 = 3;

/// `SiteSettingUpdateExistingUsers.is_sidebar_default_setting?`: the default
/// user preferences the port updates (and backfills).
pub const SIDEBAR_DEFAULTS: [&str; 2] = [
    "default_navigation_menu_categories",
    "default_navigation_menu_tags",
];

/// `SiteSettings::DeprecatedSettings::SETTINGS`: old name, new name and
/// whether the old one still overrides the new.
const DEPRECATED: &[(&str, &str, bool)] = &[
    ("min_first_post_typing_time", "fast_typing_threshold", false),
    (
        "twitter_summary_large_image",
        "x_summary_large_image",
        false,
    ),
    (
        "external_system_avatars_enabled",
        "external_system_avatars_url",
        false,
    ),
    (
        "allow_duplicate_topic_titles",
        "duplicate_topic_titles",
        false,
    ),
    (
        "allow_duplicate_topic_titles_category",
        "duplicate_topic_titles",
        false,
    ),
];

/// Settings whose `site_setting_changed` handlers (config/initializers
/// /014-track-setting-changes.rb) write more than caches.
const CHANGE_HANDLERS_WRITE: &[&str] = &[
    "title",
    "site_description",
    "must_approve_users",
    "emoji_set",
    "slug_generation_method",
    "reviewable_low_priority_threshold",
    "splash_screen_image",
    "simple_email_subject",
    "content_localization_enabled",
];

/// `SiteSettings::Validations#validate_<name>` and TypeSupervisor's
/// `validate_value` hooks.
const CUSTOM_VALIDATIONS: &[&str] = &[
    "allow_all_users_to_flag_illegal_content",
    "allow_likes_in_anonymous_mode",
    "backup_location",
    "cors_origins",
    "enable_local_logins",
    "enable_page_publishing",
    "enable_s3_uploads",
    "enforce_second_factor",
    "s3_backup_bucket",
    "s3_upload_bucket",
    "secure_uploads",
    "share_quote_buttons",
    "slow_down_crawler_user_agents",
    "strip_image_metadata",
    "x_summary_large_image",
];

/// How an update ends when it isn't a server error.
pub enum Outcome {
    /// `head :no_content`
    Done,
    /// `Discourse::InvalidParameters`: a 422 with the message.
    Invalid(String),
}

/// One of the setting's declared options.
fn opt<'a>(def: &'a crate::site_settings::Definition, key: &str) -> Option<&'a Yaml> {
    def.options.as_ref()?.get(Yaml::String(key.into()))
}

/// `hidden: true` in the setting's definition.
pub fn is_hidden(def: &crate::site_settings::Definition) -> bool {
    opt(def, "hidden").is_some_and(|v| matches!(v, Yaml::Bool(true)))
}

/// ActionView's `number_with_delimiter`.
fn delimited(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// `SiteSetting::Update.call` for `id` set to `raw`.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    defs: &Definitions,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    guardian: &Guardian,
    name: &str,
    raw: &str,
    backfill: bool,
) -> Result<Outcome, AppError> {
    if settings.get("site_archived")?.truthy() && name != "site_archived" {
        return Err(Unsupported("changing settings of an archived site").into());
    }
    let actor = guardian
        .user()
        .ok_or(Unsupported("changing settings anonymously"))?;
    // params' after_validation: the type, which an unknown name has none of.
    let Some(def) = defs.get(name) else {
        return Ok(Outcome::Invalid(format!(
            "No setting named '{name}' exists"
        )));
    };
    // settings_are_not_deprecated: a hard deprecation (no override) refuses.
    if let Some((_, new_name, override_)) = DEPRECATED.iter().find(|(old, _, _)| *old == name) {
        if *override_ {
            return Err(Unsupported("overriding deprecated settings").into());
        }
        let message = i18n
            .t_with(
                "errors.site_settings.site_settings_are_deprecated",
                &[("old_names", name), ("new_names", new_name)],
            )
            .unwrap_or_default();
        return Ok(Outcome::Invalid(message));
    }
    // SiteSettingExtension::InvalidSettingAccess, which the controller's
    // on_exceptions answers.
    if def.themeable {
        return Ok(Outcome::Invalid(format!(
            "{name} cannot be changed like this because it is a themeable setting. Instead, use the \
             ThemeSiteSettingManager service to manage themeable site settings."
        )));
    }
    if def.is_upcoming_change() {
        return Err(Unsupported("upcoming change settings").into());
    }
    if CHANGE_HANDLERS_WRITE.contains(&name) {
        return Err(Unsupported("settings whose change handlers write").into());
    }
    if CUSTOM_VALIDATIONS.contains(&name) || opt(def, "validator").is_some() {
        return Err(Unsupported("settings with custom validations").into());
    }
    if name.starts_with("default_") && !SIDEBAR_DEFAULTS.contains(&name) {
        return Err(Unsupported("default_* settings (user preference backfills)").into());
    }
    if opt(def, "shadowed_by_global").is_some_and(|v| matches!(v, Yaml::Bool(true))) {
        return Err(Unsupported("settings a global setting can shadow").into());
    }

    // The value, stripped and cast for its type, as Ruby holds it.
    let value = raw.trim();
    let new_value = match def.data_type {
        DataType::Integer => {
            let digits: String = value
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            Value::Int(crate::ruby::to_i(&digits))
        }
        DataType::Float => Value::Float(crate::ruby::to_f(value)),
        DataType::Bool => Value::Bool(matches!(value, "t" | "true")),
        DataType::String => Value::Str(value.to_string()),
        DataType::CategoryList | DataType::TagList if SIDEBAR_DEFAULTS.contains(&name) => {
            Value::Str(value.to_string())
        }
        DataType::Enum => match &def.default {
            Value::Int(_) => Value::Int(crate::ruby::to_i(value)),
            _ => Value::Str(value.to_string()),
        },
        _ => {
            return Err(
                Unsupported("setting types other than string, number, bool and enum").into(),
            );
        }
    };

    // settings_are_visible
    let hidden = opt(def, "hidden").is_some_and(|v| matches!(v, Yaml::Bool(true)));
    if hidden && name != "enable_site_owner_onboarding" {
        let message = i18n
            .t_with(
                "errors.site_settings.site_settings_are_hidden",
                &[("setting_names", name)],
            )
            .unwrap_or_default();
        return Ok(Outcome::Invalid(message));
    }

    // set_and_log: nothing when the value is unchanged.
    let previous = settings.get(name)?.clone();
    if previous == new_value {
        return Ok(Outcome::Done);
    }

    // TypeSupervisor#validate_value
    if let Some(message) = invalid(def, &new_value, i18n)? {
        return Ok(Outcome::Invalid(format!("{name}: {message}")));
    }
    // A plugin's enabled setting turned on: the plugin's behaviour, which
    // the port has only for the plugins it has started porting (and runs the
    // plugins the reference enables by default with, as their default).
    if let Some(plugin) = &def.plugin
        && defs.enabled_setting_of(plugin) == Some(name)
        && new_value.truthy()
        && !def.default.truthy()
        && !crate::plugins::STARTED.contains(&plugin.as_str())
    {
        return Err(Unsupported("enabling a bundled plugin the port has not started").into());
    }
    // SiteSetting::Update#save: default user preferences log whether
    // existing users were updated; #backfill enqueues the sidebar job.
    let details = if SIDEBAR_DEFAULTS.contains(&name) {
        let answer = i18n
            .t(if backfill {
                "csv_export.boolean_yes"
            } else {
                "csv_export.boolean_no"
            })
            .unwrap_or_default();
        i18n.t_with(
            "staff_action_logs.site_setting.update_existing_users",
            &[("value", answer)],
        )
    } else {
        None
    };
    set_and_log_details(
        &mut *conn,
        bus,
        def,
        &previous,
        &new_value,
        actor.id,
        details.as_deref(),
    )
    .await?;
    crate::plugins::site_setting_changed(&mut *conn, defs, settings, name, &previous, &new_value)
        .await?;
    if backfill && SIDEBAR_DEFAULTS.contains(&name) {
        crate::jobs::enqueue(
            &mut *conn,
            "backfill_sidebar_site_settings",
            serde_json::json!({
                "setting_name": name,
                "previous_value": previous.to_s(),
                "new_value": new_value.to_s(),
            }),
        )
        .await?;
    }
    Ok(Outcome::Done)
}

/// `add_override!`'s `sanitize_field` of a client setting's string: the
/// value stored and published (the log keeps the given one). Applied to
/// string settings; the port leaves the other string-valued types (lists,
/// enums) as given.
fn stored_value(
    def: &crate::site_settings::Definition,
    new_value: &Value,
) -> Result<Value, AppError> {
    if !def.client || def.data_type != DataType::String {
        return Ok(new_value.clone());
    }
    match new_value {
        Value::Bool(_) | Value::Int(_) | Value::Float(_) => Ok(new_value.clone()),
        other => {
            let text = other.to_s();
            let sanitized = crate::sanitize_field::sanitize_field(&text, &[])?;
            Ok(if sanitized == text {
                new_value.clone()
            } else {
                Value::Str(sanitized)
            })
        }
    }
}

/// `SiteSetting.<name> = value` for a boolean (`add_override!`): the
/// override always saved, and when the value changed, client settings
/// published.
pub async fn save_setting(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    def: &crate::site_settings::Definition,
    previous: &Value,
    value: bool,
) -> Result<(), AppError> {
    save_override(&mut *conn, def, if value { "t" } else { "f" }).await?;
    if previous.truthy() != value && def.client {
        bus.publish(
            &mut *conn,
            "/client_settings",
            &serde_json::json!({ "name": def.name, "value": value, "scoped_to": null }),
            None,
        )
        .await?;
    }
    Ok(())
}

/// `SiteSettingExtension#set_and_log` past its early return: the
/// override saved, client settings published, and the change logged by
/// `actor_id` (hidden settings unlogged, secret ones without values).
pub async fn set_and_log(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    def: &crate::site_settings::Definition,
    previous: &Value,
    new_value: &Value,
    actor_id: i32,
) -> Result<(), AppError> {
    set_and_log_details(conn, bus, def, previous, new_value, actor_id, None).await
}

/// `set_and_log` with the log's details (the backfill answer of default
/// user preference settings).
pub async fn set_and_log_details(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    def: &crate::site_settings::Definition,
    previous: &Value,
    new_value: &Value,
    actor_id: i32,
    details: Option<&str>,
) -> Result<(), AppError> {
    let name = def.name.as_str();
    let stored = stored_value(def, new_value)?;
    let db_value = match &stored {
        Value::Bool(b) => if *b { "t" } else { "f" }.to_string(),
        other => other.to_s(),
    };
    save_override(&mut *conn, def, &db_value).await?;
    // notify_clients!: the new value to every client, for client settings.
    if def.client {
        let value = match &stored {
            Value::Bool(b) => serde_json::json!(b),
            Value::Int(i) => serde_json::json!(i),
            Value::Float(f) => serde_json::json!(f),
            other => serde_json::json!(other.to_s()),
        };
        bus.publish(
            &mut *conn,
            "/client_settings",
            &serde_json::json!({ "name": name, "value": value, "scoped_to": null }),
            None,
        )
        .await?;
    }
    // SiteSettingExtension#log
    if opt(def, "hidden").is_some_and(|v| matches!(v, Yaml::Bool(true))) {
        return Ok(());
    }
    let secret = opt(def, "secret").is_some_and(|v| matches!(v, Yaml::Bool(true)));
    let (previous, new_value) = if secret {
        ("[FILTERED]".to_string(), "[FILTERED]".to_string())
    } else {
        (previous.to_s(), new_value.to_s())
    };
    // StaffActionLogger#log_site_setting_change
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, details, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(CHANGE_SITE_SETTING)
    .bind(actor_id)
    .bind(name)
    .bind(previous)
    .bind(new_value)
    .bind(details)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `SiteSetting.set(name, value)` (`add_override!`, no staff log) for the
/// plain types: `to_db_value`'s cast and validation, the override saved,
/// client settings published when the value changed. Err(message) for
/// the InvalidParameters an invalid value raises.
pub async fn set(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    settings: &SiteSettings,
    defs: &Definitions,
    i18n: &crate::i18n::I18n,
    name: &str,
    raw: &serde_json::Value,
) -> Result<Result<Value, String>, AppError> {
    let def = defs
        .get(name)
        .ok_or(Unsupported("setting an unknown site setting"))?;
    let text = || match raw {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        _ => Err(Unsupported("a site setting value that is a list or hash")),
    };
    let value = match def.data_type {
        DataType::Bool => Value::Bool(
            matches!(raw, serde_json::Value::Bool(true)) || {
                let t = text()?;
                t == "t" || t == "true"
            },
        ),
        DataType::Integer => match raw {
            serde_json::Value::Number(n) if n.is_i64() => Value::Int(n.as_i64().unwrap_or(0)),
            _ => Value::Int(crate::ruby::to_i(&text()?)),
        },
        DataType::String | DataType::Username => Value::Str(text()?),
        _ => return Err(Unsupported("SiteSetting.set for this setting type").into()),
    };
    if def.data_type == DataType::Username {
        let username = value.to_s();
        if !crate::ruby::is_blank(&username) {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = $1)")
                    .bind(&username)
                    .fetch_one(&mut *conn)
                    .await?;
            if !exists {
                let message = i18n
                    .t("site_settings.errors.invalid_username")
                    .unwrap_or("There's no user with that username.");
                return Ok(Err(format!("{name}: {message}")));
            }
        }
    } else if let Some(message) = invalid(def, &value, i18n)? {
        return Ok(Err(format!("{name}: {message}")));
    }
    let stored = stored_value(def, &value)?;
    let db_value = match &stored {
        Value::Bool(b) => if *b { "t" } else { "f" }.to_string(),
        other => other.to_s(),
    };
    // A string set to nil is kept as NULL (to_db_value's implicit cast).
    let nil = raw.is_null() && matches!(def.data_type, DataType::String | DataType::Username);
    save_override_value(&mut *conn, def, (!nil).then_some(db_value.as_str())).await?;
    let previous = settings.get(name)?;
    if def.client && *previous != stored {
        let published = match &stored {
            Value::Bool(b) => serde_json::json!(b),
            Value::Int(i) => serde_json::json!(i),
            other => serde_json::json!(other.to_s()),
        };
        bus.publish(
            &mut *conn,
            "/client_settings",
            &serde_json::json!({ "name": name, "value": published, "scoped_to": null }),
            None,
        )
        .await?;
    }
    Ok(Ok(stored))
}

/// `DbProvider#save`: a new row (even holding the default), else the
/// value and type when either changed; an unchanged row is not written.
pub(crate) async fn save_override(
    conn: &mut PgConnection,
    def: &crate::site_settings::Definition,
    db_value: &str,
) -> Result<(), sqlx::Error> {
    save_override_value(conn, def, Some(db_value)).await
}

/// `save_override` of a value that may be nil (a string setting set to
/// nil keeps a NULL value).
pub(crate) async fn save_override_value(
    conn: &mut PgConnection,
    def: &crate::site_settings::Definition,
    db_value: Option<&str>,
) -> Result<(), sqlx::Error> {
    let row: Option<(Option<String>, i32)> =
        sqlx::query_as("SELECT value, data_type FROM site_settings WHERE name = $1")
            .bind(&def.name)
            .fetch_optional(&mut *conn)
            .await?;
    match row {
        None => {
            sqlx::query(
                "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
                 VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
            )
            .bind(&def.name)
            .bind(def.data_type as i32)
            .bind(db_value)
            .execute(&mut *conn)
            .await?;
        }
        Some((value, data_type))
            if value.as_deref() == db_value && data_type == def.data_type as i32 => {}
        Some(_) => {
            sqlx::query(
                "UPDATE site_settings SET value = $2, data_type = $3, updated_at = clock_timestamp() WHERE name = $1",
            )
            .bind(&def.name)
            .bind(db_value)
            .bind(def.data_type as i32)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// TypeSupervisor#validate_value: the choices of an enum, then the type's
/// validator (IntegerSettingValidator, StringSettingValidator).
fn invalid(
    def: &crate::site_settings::Definition,
    value: &Value,
    i18n: &crate::i18n::I18n,
) -> Result<Option<String>, Unsupported> {
    let number = |key: &str| opt(def, key).and_then(Yaml::as_i64);
    let t = |key: &str, args: &[(&str, &str)]| i18n.t_with(key, args).unwrap_or_default();
    match def.data_type {
        DataType::Enum => {
            let Some(Yaml::Sequence(choices)) = opt(def, "choices") else {
                return Err(Unsupported("enum settings with an enum class"));
            };
            let wanted = value.to_s();
            let listed = choices.iter().any(|c| match c {
                Yaml::String(s) => *s == wanted,
                Yaml::Number(n) => n.to_string() == wanted,
                _ => false,
            });
            if !listed {
                // Discourse::InvalidParameters.new(:value)
                return Ok(Some("value".to_string()));
            }
            string_validator(def, &wanted, i18n)
        }
        DataType::Integer => {
            let hidden = opt(def, "hidden").is_some_and(|v| matches!(v, Yaml::Bool(true)));
            let min = number("min").or((!hidden).then_some(0));
            let max = number("max").or((!hidden).then_some(2_000_000_000));
            let v = match value {
                Value::Int(i) => *i,
                _ => 0,
            };
            if min.is_some_and(|m| m > v) || max.is_some_and(|m| m < v) {
                let (min_s, max_s) = (min.map(delimited), max.map(delimited));
                return Ok(Some(match (min_s, max_s) {
                    (Some(min), Some(max)) => t(
                        "site_settings.errors.invalid_integer_min_max",
                        &[("min", &min), ("max", &max)],
                    ),
                    (Some(min), None) => {
                        t("site_settings.errors.invalid_integer_min", &[("min", &min)])
                    }
                    (None, Some(max)) => {
                        t("site_settings.errors.invalid_integer_max", &[("max", &max)])
                    }
                    (None, None) => t("site_settings.errors.invalid_integer", &[]),
                }));
            }
            Ok(None)
        }
        DataType::String => string_validator(def, &value.to_s(), i18n),
        _ => Ok(None),
    }
}

/// StringSettingValidator: the length bounds; regexes and JSON schemas are
/// not ported.
fn string_validator(
    def: &crate::site_settings::Definition,
    value: &str,
    i18n: &crate::i18n::I18n,
) -> Result<Option<String>, Unsupported> {
    let t = |key: &str, args: &[(&str, &str)]| i18n.t_with(key, args).unwrap_or_default();
    if opt(def, "regex").is_some() || opt(def, "json_schema").is_some() {
        return Err(Unsupported("string settings with a regex or JSON schema"));
    }
    if value.is_empty() {
        return Ok(None);
    }
    let len = value.chars().count() as i64;
    let min = opt(def, "min").and_then(Yaml::as_i64);
    let max = opt(def, "max").and_then(Yaml::as_i64);
    if min.is_some_and(|m| m > len) || max.is_some_and(|m| m < len) {
        let (min_s, max_s) = (min.map(|m| m.to_string()), max.map(|m| m.to_string()));
        return Ok(Some(match (min_s, max_s) {
            (Some(min), Some(max)) => t(
                "site_settings.errors.invalid_string_min_max",
                &[("min", &min), ("max", &max)],
            ),
            (Some(min), None) => t(
                "site_settings.errors.invalid_string_min",
                &[("count", &min)],
            ),
            (_, Some(max)) => t(
                "site_settings.errors.invalid_string_max",
                &[("count", &max)],
            ),
            (None, None) => t("site_settings.errors.invalid_string", &[]),
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::delimited;

    #[test]
    fn delimits_like_number_with_delimiter() {
        assert_eq!(delimited(1), "1");
        assert_eq!(delimited(1000), "1,000");
        assert_eq!(delimited(2_000_000_000), "2,000,000,000");
        assert_eq!(delimited(-1234567), "-1,234,567");
    }
}
