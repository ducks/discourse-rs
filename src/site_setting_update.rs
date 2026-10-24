//! `SiteSetting::Update` for one setting (`Admin::SiteSettingsController
//! #update`): the value cast for its type, the policies, TypeSupervisor's
//! validation, then `set_and_log` (the override saved, the
//! change_site_setting staff log).
//!
//! Ported for core's and the bundled plugins' settings of the plain types
//! (string, integer, float, bool, enum with listed, recorded or enum
//! class choices), uploads (by URL: the staff log's URLs, the upload
//! reference, SiteIconManager's optimized icons), the list types (list of
//! every list_type but those of an enum class, url_list, host_list,
//! value_list, simple_list, emoji_list, category_list, group_list,
//! tag_list: their choices, mandatory values, disallowed groups and
//! validator classes, crate::site_setting_lists), a plugin
//! setting's change running the handlers of the plugins the port
//! has started (crate::plugins::site_setting_changed), and the default
//! user preferences that are user options with their backfill
//! (crate::user_option_defaults). Refused: enabling a
//! bundled plugin the port has not started (unless the reference has
//! it on by default), themeable and upcoming change settings, the other
//! types (uploaded image lists, tag group lists, lists of an enum class,
//! objects...), settings with a validator class the port lacks or a custom
//! validation, the category and tag defaults (their backfill), client strings
//! with markup (sanitize_field), settings shadowed by a global setting,
//! an archived site, and the settings whose change handlers
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

/// `SiteSetting::DEFAULT_USER_PREFERENCES`: the defaults whose change can
/// update the existing users.
pub const DEFAULT_USER_PREFERENCES: &[&str] = &[
    "default_email_digest_frequency",
    "default_include_tl0_in_digests",
    "default_email_level",
    "default_email_messages_level",
    "default_email_mailing_list_mode",
    "default_email_mailing_list_mode_frequency",
    "default_email_previous_replies",
    "default_email_in_reply_to",
    "default_hide_profile",
    "default_hide_presence",
    "default_other_new_topic_duration_minutes",
    "default_other_auto_track_topics_after_msecs",
    "default_other_notification_level_when_replying",
    "default_other_external_links_in_new_tab",
    "default_other_enable_quoting",
    "default_other_enable_smart_lists",
    "default_other_dynamic_favicon",
    "default_other_like_notification_frequency",
    "default_other_skip_new_user_tips",
    "default_other_enable_markdown_monospace_font",
    "default_topics_automatic_unpin",
    "default_categories_watching",
    "default_categories_tracking",
    "default_categories_muted",
    "default_categories_watching_first_post",
    "default_categories_normal",
    "default_tags_watching",
    "default_tags_tracking",
    "default_tags_muted",
    "default_tags_watching_first_post",
    "default_text_size",
    "default_title_count_mode",
    "default_navigation_menu_categories",
    "default_navigation_menu_tags",
    "default_sidebar_link_to_filtered_list",
    "default_sidebar_show_count_of_new_items",
    "default_composition_mode",
    "default_watched_precedence_over_muted",
];

/// A default user preference whose backfill the port lacks (the category
/// and tag defaults and the rest that are not user options).
pub fn preference_not_ported(name: &str) -> bool {
    DEFAULT_USER_PREFERENCES.contains(&name)
        && !SIDEBAR_DEFAULTS.contains(&name)
        && !crate::user_option_defaults::is_user_option(name)
}

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
    "must_approve_users",
    "emoji_set",
    "slug_generation_method",
    "reviewable_low_priority_threshold",
    "splash_screen_image",
    "simple_email_subject",
    "content_localization_enabled",
    // config/initializers/020-discourse-mcp.rb: revoke_disallowed_clients!
    "mcp_oauth_client_id_metadata_policy",
    "mcp_oauth_client_id_metadata_domains",
];

/// `SiteSettings::Validations#validate_<name>` and TypeSupervisor's
/// `validate_value` hooks.
const CUSTOM_VALIDATIONS: &[&str] = &[
    "allow_all_users_to_flag_illegal_content",
    "allow_likes_in_anonymous_mode",
    "backup_location",
    "enable_local_logins",
    "enable_page_publishing",
    "enable_s3_uploads",
    "enforce_second_factor",
    "s3_backup_bucket",
    "s3_upload_bucket",
    "secure_uploads",
    "strip_image_metadata",
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

/// CssColorValidator#valid_value?: a 3 or 6 digit hex color, or one of
/// its named colors.
fn css_color(value: &str) -> bool {
    const COLORS: &str = "aliceblue antiquewhite aqua aquamarine azure beige bisque black \
        blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate coral \
        cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod darkgray darkgreen \
        darkgrey darkkhaki darkmagenta darkolivegreen darkorange darkorchid darkred darksalmon \
        darkseagreen darkslateblue darkslategray darkslategrey darkturquoise darkviolet deeppink \
        deepskyblue dimgray dimgrey dodgerblue firebrick floralwhite forestgreen fuchsia \
        gainsboro ghostwhite gold goldenrod gray green greenyellow grey honeydew hotpink \
        indianred indigo ivory khaki lavender lavenderblush lawngreen lemonchiffon lightblue \
        lightcoral lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink \
        lightsalmon lightseagreen lightskyblue lightslategray lightslategrey lightsteelblue \
        lightyellow lime limegreen linen magenta maroon mediumaquamarine mediumblue \
        mediumorchid mediumpurple mediumseagreen mediumslateblue mediumspringgreen \
        mediumturquoise mediumvioletred midnightblue mintcream mistyrose moccasin navajowhite \
        navy oldlace olive olivedrab orange orangered orchid palegoldenrod palegreen \
        paleturquoise palevioletred papayawhip peachpuff peru pink plum powderblue purple red \
        rosybrown royalblue saddlebrown salmon sandybrown seagreen seashell sienna silver \
        skyblue slateblue slategray slategrey snow springgreen steelblue tan teal thistle \
        tomato turquoise violet wheat white whitesmoke yellow yellowgreen";
    let hex = value
        .strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|c| c.is_ascii_hexdigit()));
    hex || COLORS.split_whitespace().any(|c| c == value.to_lowercase())
}

/// An upload setting's Upload's URL (nil for none).
async fn upload_url(conn: &mut PgConnection, value: &Value) -> Result<Option<String>, sqlx::Error> {
    if value.is_blank() {
        return Ok(None);
    }
    sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
        .bind(value.to_i() as i32)
        .fetch_optional(conn)
        .await
}

/// The setting's choices (`@choices[name]`'s values): listed in its
/// definition, or, for a Ruby expression (`BaseFontSetting.values`), as
/// recorded from the reference (crate::setting_enums).
pub(crate) fn choice_values(def: &crate::site_settings::Definition) -> Vec<String> {
    match opt(def, "choices") {
        Some(Yaml::Sequence(choices)) => choices
            .iter()
            .filter_map(|c| match c {
                Yaml::String(s) => Some(s.clone()),
                Yaml::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        _ => crate::setting_enums::recorded(&def.name)
            .and_then(|r| r.get("valid_values").or_else(|| r.get("choices")))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| match v.get("value").unwrap_or(v) {
                        serde_json::Value::String(s) => Some(s.clone()),
                        serde_json::Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
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
    config: &crate::config::Config,
    defs: &Definitions,
    settings: &SiteSettings,
    i18n: &crate::i18n::I18n,
    guardian: &Guardian,
    name: &str,
    raw: &str,
    backfill: bool,
    // `allow_changing_hidden:`, the hidden settings the caller may set.
    allow_hidden: &[&str],
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
    let validator = opt(def, "validator").and_then(Yaml::as_str);
    let css_validator = matches!(
        validator,
        Some("CssColorValidator" | "CssColorWithBlankValidator")
    );
    if CUSTOM_VALIDATIONS.contains(&name) {
        return Err(Unsupported("settings with custom validations").into());
    }
    let list_validator = validator.filter(|v| crate::site_setting_lists::VALIDATORS.contains(v));
    if validator.is_some() && !css_validator && list_validator.is_none() {
        return Err(Unsupported("site settings with a validator class not ported").into());
    }
    if preference_not_ported(name) {
        return Err(Unsupported("default_* settings (user preference backfills)").into());
    }
    if opt(def, "shadowed_by_global").is_some_and(|v| matches!(v, Yaml::Bool(true))) {
        return Err(Unsupported("settings a global setting can shadow").into());
    }

    // The value, stripped and cast for its type, as Ruby holds it.
    let value = crate::ruby::strip(raw);
    let list = crate::site_setting_lists::is_list(def.data_type);
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
        DataType::String
        | DataType::Color
        | DataType::Email
        | DataType::Username
        | DataType::Group => Value::Str(value.to_string()),
        // A list of an enum class's values (the content localization
        // locales) validates through the class.
        DataType::List if opt(def, "enum").is_some() => {
            return Err(Unsupported("list settings of an enum class").into());
        }
        // The list types, `|` separated strings.
        _ if list => Value::Str(value.to_string()),
        DataType::Enum => match &def.default {
            Value::Int(_) => Value::Int(crate::ruby::to_i(value)),
            _ => Value::Str(value.to_string()),
        },
        // `Upload.get_from_url(raw_value) || ""`: the upload's id.
        DataType::Upload => {
            match crate::upload_references::get_from_url(&mut *conn, value).await? {
                Some((id, _)) => Value::Int(i64::from(id)),
                None => Value::Str(String::new()),
            }
        }
        DataType::UploadedImageList => {
            return Err(Unsupported("uploaded image list settings").into());
        }
        DataType::TagGroupList => return Err(Unsupported("tag group list settings").into()),
        DataType::Objects => return Err(Unsupported("objects settings").into()),
        _ => {
            return Err(Unsupported(
                "setting types other than string, number, bool, enum, upload and the lists",
            )
            .into());
        }
    };

    // settings_are_visible
    let hidden = opt(def, "hidden").is_some_and(|v| matches!(v, Yaml::Bool(true)));
    if hidden && name != "enable_site_owner_onboarding" && !allow_hidden.contains(&name) {
        let message = i18n
            .t_with(
                "errors.site_settings.site_settings_are_hidden",
                &[("setting_names", name)],
            )
            .unwrap_or_default();
        return Ok(Outcome::Invalid(message));
    }

    // set_and_log: nothing when the value is unchanged. An upload
    // setting's value is its Upload (nil without one), which "" never
    // equals.
    let previous = settings.get(name)?.clone();
    let unchanged = if def.data_type == DataType::Upload {
        matches!(new_value, Value::Int(id) if !previous.is_blank() && previous.to_i() == id)
    } else {
        previous == new_value
    };
    if unchanged {
        return Ok(Outcome::Done);
    }

    // set's filter_value never applies here: HOSTNAME_SETTINGS holds
    // strings and the service passes the name as a symbol.
    if list {
        let wanted = new_value.to_s();
        // TypeSupervisor#validate_value: a list that allows no other
        // values than its choices raises the invalid choices.
        if def.data_type == DataType::List
            && opt(def, "allow_any").is_some_and(|v| matches!(v, Yaml::Bool(false)))
        {
            let choices = choice_values(def);
            if choices.is_empty() {
                return Err(Unsupported("list settings whose choices are not recorded").into());
            }
            let diff: Vec<&str> = crate::ruby::split(&wanted, "|")
                .into_iter()
                .filter(|v| !choices.iter().any(|c| c == v))
                .collect();
            if !diff.is_empty() {
                let message = i18n
                    .t_count(
                        "errors.site_settings.invalid_choice",
                        diff.len() as i64,
                        &[("name", &diff.join(","))],
                    )
                    .unwrap_or_default();
                return Ok(Outcome::Invalid(message));
            }
        }
        // The setting's validator class, else its type's: a list's
        // StringSettingValidator, a host list's HostListSettingValidator.
        let error = match list_validator {
            Some(class) => {
                crate::site_setting_lists::validate(
                    &mut *conn, settings, defs, i18n, class, &wanted,
                )
                .await?
            }
            None => match def.data_type {
                DataType::List => string_validator(def, &wanted, i18n)?,
                DataType::HostList if !crate::site_setting_lists::host_list_valid(&wanted) => i18n
                    .t("site_settings.errors.invalid_domain_hostname")
                    .map(str::to_string),
                _ => None,
            },
        };
        if let Some(message) = error {
            return Ok(Outcome::Invalid(format!("{name}: {message}")));
        }
        if let Some(message) =
            crate::site_setting_lists::validate_named(name, &wanted, settings, i18n)?
        {
            return Ok(Outcome::Invalid(message));
        }
    }
    // An enum class's valid_value? (recorded values), whose refusal is
    // its own message.
    if def.data_type == DataType::Enum && opt(def, "enum").is_some() {
        let choices = choice_values(def);
        if choices.is_empty() {
            return Err(Unsupported("enum settings with an enum class not recorded").into());
        }
        let wanted = new_value.to_s();
        if !choices.contains(&wanted) {
            return Ok(Outcome::Invalid(format!(
                "Invalid value `{wanted}` for `{name}`"
            )));
        }
    } else if let Some(message) = invalid(def, &new_value, i18n)? {
        return Ok(Outcome::Invalid(format!("{name}: {message}")));
    }
    // The type supervisor's validator: CssColorValidator (blank allowed
    // by CssColorWithBlankValidator).
    if css_validator {
        let v = new_value.to_s();
        let blank_ok = validator == Some("CssColorWithBlankValidator") && crate::ruby::is_blank(&v);
        if !blank_ok && !css_color(&v) {
            let message = i18n
                .t("site_settings.errors.invalid_css_color")
                .unwrap_or_default();
            return Ok(Outcome::Invalid(format!("{name}: {message}")));
        }
    }
    // The type's validator: EmailSettingValidator, UsernameSettingValidator,
    // GroupSettingValidator (each allowing blank).
    let v = new_value.to_s();
    let type_error = match def.data_type {
        DataType::Email if !crate::ruby::is_blank(&v) && !crate::accounts::valid_email(&v) => {
            Some("site_settings.errors.invalid_email")
        }
        DataType::Username if !crate::ruby::is_blank(&v) => {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username = $1)")
                    .bind(&v)
                    .fetch_one(&mut *conn)
                    .await?;
            (!exists).then_some("site_settings.errors.invalid_username")
        }
        DataType::Group if !crate::ruby::is_blank(&v) => {
            let exists = v.chars().all(|c| c.is_ascii_digit())
                && sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1::bigint)",
                )
                .bind(&v)
                .fetch_one(&mut *conn)
                .await?;
            (!exists).then_some("site_settings.errors.invalid_group")
        }
        _ => None,
    };
    if let Some(key) = type_error {
        let message = i18n.t(key).unwrap_or_default();
        return Ok(Outcome::Invalid(format!("{name}: {message}")));
    }
    // SiteSettings::Validations#validate_x_summary_large_image
    if name == "x_summary_large_image" && !new_value.is_blank() {
        let svg: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM uploads WHERE id = $1 AND extension = 'svg')",
        )
        .bind(new_value.to_i() as i32)
        .fetch_one(&mut *conn)
        .await?;
        if svg {
            let message = i18n
                .t("errors.site_settings.x_summary_large_image_no_svg")
                .unwrap_or_default()
                .to_string();
            return Ok(Outcome::Invalid(message));
        }
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
    let user_option = crate::user_option_defaults::is_user_option(name);
    let details = if SIDEBAR_DEFAULTS.contains(&name) || user_option {
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
    let changed = set_and_log_details(
        &mut *conn,
        bus,
        def,
        &previous,
        &new_value,
        actor.id,
        details.as_deref(),
    )
    .await?;
    if changed {
        crate::plugins::site_setting_changed(
            &mut *conn, defs, settings, name, &previous, &new_value,
        )
        .await?;
    }
    // SiteIconManager.ensure_optimized! for the icons' settings, with the
    // settings as they are now.
    if crate::site_icons::watched(name) {
        let now = SiteSettings::load(&mut *conn, defs, &config.globals).await?;
        let store = crate::file_store::FileStore::for_site(config, &now)?;
        crate::site_icons::create_optimized(&mut *conn, &now, &store).await?;
    }
    if backfill && user_option {
        crate::user_option_defaults::backfill(&mut *conn, name, &new_value, &previous).await?;
    }
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

/// `add_override!`'s `sanitize_field` of a client setting's string, then a
/// list's mandatory values and disallowed groups: the value stored (the
/// log keeps the given one). Sanitized for string and list settings; the
/// port leaves enums' strings as given.
fn stored_value(
    def: &crate::site_settings::Definition,
    new_value: &Value,
) -> Result<Value, AppError> {
    let list = crate::site_setting_lists::is_list(def.data_type);
    if list {
        let mut text = new_value.to_s();
        if def.client {
            text = crate::sanitize_field::sanitize_field(&text, &[])?;
        }
        return Ok(Value::Str(
            crate::site_setting_lists::apply_mandatory_and_disallowed(def, &text),
        ));
    }
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

/// A list setting's `current[name]` before the save: its row's value,
/// else its default (the getter's value, `previous`, has the mandatory
/// values merged in).
async fn current_raw(
    conn: &mut PgConnection,
    def: &crate::site_settings::Definition,
    previous: &Value,
) -> Result<String, sqlx::Error> {
    let row: Option<Option<String>> =
        sqlx::query_scalar("SELECT value FROM site_settings WHERE name = $1")
            .bind(&def.name)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(match row {
        Some(value) => value.unwrap_or_default(),
        None if def.mandatory_values.is_some() => def.default.to_s(),
        None => previous.to_s(),
    })
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
    set_and_log_details(conn, bus, def, previous, new_value, actor_id, None).await?;
    Ok(())
}

/// `set_and_log` with the log's details (the backfill answer of default
/// user preference settings). Whether `current[name]` changed, so
/// `site_setting_changed` is triggered: always for the types other than
/// lists, whose stored value is the one given.
pub async fn set_and_log_details(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    def: &crate::site_settings::Definition,
    previous: &Value,
    new_value: &Value,
    actor_id: i32,
    details: Option<&str>,
) -> Result<bool, AppError> {
    let name = def.name.as_str();
    let list = crate::site_setting_lists::is_list(def.data_type);
    let stored = stored_value(def, new_value)?;
    let old_raw = if list {
        Some(current_raw(&mut *conn, def, previous).await?)
    } else {
        None
    };
    let db_value = match &stored {
        Value::Bool(b) => if *b { "t" } else { "f" }.to_string(),
        other => other.to_s(),
    };
    save_override(&mut *conn, def, &db_value).await?;
    let changed = old_raw.is_none_or(|old| old != db_value);
    // An upload setting's value is its Upload, whose to_s is its URL.
    let urls = if def.data_type == DataType::Upload {
        // SiteSetting's after_save: UploadReference.ensure_exist! for the
        // row's upload.
        let row_id: i64 =
            sqlx::query_scalar("SELECT id::bigint FROM site_settings WHERE name = $1")
                .bind(name)
                .fetch_one(&mut *conn)
                .await?;
        let upload_id = (!new_value.is_blank()).then(|| new_value.to_i());
        crate::upload_references::ensure_exist(&mut *conn, "SiteSetting", row_id, upload_id)
            .await?;
        let previous_url = upload_url(&mut *conn, previous).await?;
        let new_url = upload_url(&mut *conn, new_value).await?.unwrap_or_default();
        Some((previous_url, new_url))
    } else {
        None
    };
    // notify_clients!: the getter's new value to every client, for client
    // settings whose value changed.
    if def.client && changed {
        let value = match &stored {
            Value::Int(_) if urls.is_some() => {
                serde_json::json!(urls.as_ref().map(|(_, new)| new.clone()))
            }
            Value::Bool(b) => serde_json::json!(b),
            Value::Int(i) => serde_json::json!(i),
            Value::Float(f) => serde_json::json!(f),
            other if list => serde_json::json!(crate::site_setting_lists::with_mandatory(
                def,
                &other.to_s()
            )),
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
    // set: Discourse.request_refresh! for the settings declared `refresh`.
    if opt(def, "refresh").is_some_and(|v| matches!(v, Yaml::Bool(true))) {
        crate::bus::request_refresh(bus, &mut *conn).await?;
    }
    // SiteSettingExtension#log
    if opt(def, "hidden").is_some_and(|v| matches!(v, Yaml::Bool(true))) {
        return Ok(changed);
    }
    let secret = opt(def, "secret").is_some_and(|v| matches!(v, Yaml::Bool(true)));
    let (previous, new_value) = if secret {
        (Some("[FILTERED]".to_string()), "[FILTERED]".to_string())
    } else if let Some((previous, new_value)) = urls {
        (previous, new_value)
    } else {
        (Some(previous.to_s()), new_value.to_s())
    };
    log_change(
        conn,
        actor_id,
        name,
        previous.as_deref(),
        &new_value,
        details,
    )
    .await?;
    Ok(changed)
}

/// StaffActionLogger#log_site_setting_change.
pub async fn log_site_setting_change(
    conn: &mut PgConnection,
    actor_id: i32,
    name: &str,
    previous: &str,
    new_value: &str,
) -> Result<(), AppError> {
    log_change(conn, actor_id, name, Some(previous), new_value, None).await
}

/// StaffActionLogger#log_site_setting_change with `details:`.
pub async fn log_site_setting_change_details(
    conn: &mut PgConnection,
    actor_id: i32,
    name: &str,
    previous: &str,
    new_value: &str,
    details: &str,
) -> Result<(), AppError> {
    log_change(
        conn,
        actor_id,
        name,
        Some(previous),
        new_value,
        Some(details),
    )
    .await
}

async fn log_change(
    conn: &mut PgConnection,
    actor_id: i32,
    name: &str,
    previous: Option<&str>,
    new_value: &str,
    details: Option<&str>,
) -> Result<(), AppError> {
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
            if opt(def, "enum").is_some() {
                return Err(Unsupported("enum settings with an enum class"));
            }
            let choices = choice_values(def);
            if choices.is_empty() {
                return Err(Unsupported("enum settings without listed choices"));
            }
            let wanted = value.to_s();
            let listed = choices.contains(&wanted);
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
