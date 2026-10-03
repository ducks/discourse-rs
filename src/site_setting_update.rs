//! `SiteSetting::Update` for one setting (`Admin::SiteSettingsController
//! #update`): the value cast for its type, the policies, TypeSupervisor's
//! validation, then `set_and_log` (the override saved, the
//! change_site_setting staff log).
//!
//! Ported for core settings of the plain types (string, integer, float,
//! bool, enum with listed choices). Refused: plugin settings, themeable and
//! upcoming change settings, the other types (lists, uploads, groups,
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
pub async fn update(
    conn: &mut PgConnection,
    defs: &Definitions,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    guardian: &Guardian,
    name: &str,
    raw: &str,
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
    if def.plugin.is_some() {
        return Err(Unsupported("plugin settings").into());
    }
    if def.themeable || def.is_upcoming_change() {
        return Err(Unsupported("themeable and upcoming change settings").into());
    }
    if CHANGE_HANDLERS_WRITE.contains(&name) {
        return Err(Unsupported("settings whose change handlers write").into());
    }
    if CUSTOM_VALIDATIONS.contains(&name) || opt(def, "validator").is_some() {
        return Err(Unsupported("settings with custom validations").into());
    }
    if name.starts_with("default_") {
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
    let db_value = match &new_value {
        Value::Bool(b) => if *b { "t" } else { "f" }.to_string(),
        other => other.to_s(),
    };
    if def.client && def.data_type == DataType::String && db_value.contains(['<', '>', '&']) {
        return Err(Unsupported("client string settings with markup (sanitize_field)").into());
    }

    // add_override!: provider.save
    let updated = sqlx::query(
        "UPDATE site_settings SET value = $2, data_type = $3, updated_at = clock_timestamp() WHERE name = $1",
    )
    .bind(name)
    .bind(&db_value)
    .bind(def.data_type as i32)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    if updated == 0 {
        sqlx::query(
            "INSERT INTO site_settings (name, data_type, value, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
        )
        .bind(name)
        .bind(def.data_type as i32)
        .bind(&db_value)
        .execute(&mut *conn)
        .await?;
    }
    // StaffActionLogger#log_site_setting_change
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(CHANGE_SITE_SETTING)
    .bind(actor.id)
    .bind(name)
    .bind(previous.to_s())
    .bind(new_value.to_s())
    .execute(&mut *conn)
    .await?;
    Ok(Outcome::Done)
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
