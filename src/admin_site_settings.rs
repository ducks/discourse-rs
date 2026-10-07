//! `SiteSetting.all_settings` for Admin::SiteSettingsController#index:
//! every setting an admin can see, with its labels from the locale, its
//! value and default, and its type's details (SiteSettings::TypeSupervisor
//! `type_hash`), filtered by category, plugin or names.
//!
//! Refused: settings of enum classes not ported, objects settings holding
//! uploads, and upcoming changes that hide settings while enabled.

use serde_json::{Map, Value as Json, json};
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::config::GlobalSettings;
use crate::i18n::I18n;
use crate::setting_enums;
use crate::site_settings::{DataType, Definition, Definitions, SiteSettings, Value};
use crate::{AppError, Unsupported};

/// The index's params: `categories`, `plugin` and `names`.
#[derive(Default)]
pub struct Filters {
    pub categories: Option<Vec<String>>,
    pub plugin: Option<String>,
    pub names: Option<Vec<String>>,
}

/// SiteSettings::TypeSupervisor::REQUIRES_CONFIRMATION_TYPES
const REQUIRES_CONFIRMATION: [&str; 4] = [
    "simple",
    "simple_on_enable",
    "simple_on_disable",
    "user_option",
];

pub struct Context<'a> {
    pub defs: &'a Definitions,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub globals: &'a GlobalSettings,
    pub base_path: &'a str,
    pub urls: &'a crate::url::Urls<'a>,
}

/// `SiteSetting.all_settings(filter_categories:, filter_plugin:,
/// filter_names:)` with the other options at their defaults.
pub async fn all_settings(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    filters: &Filters,
) -> Result<Vec<Json>, AppError> {
    let mut out = Vec::new();
    let filtering =
        filters.categories.as_ref().is_some_and(|c| !c.is_empty()) || filters.plugin.is_some();
    if !filtering
        && filters
            .names
            .as_ref()
            .is_none_or(|n| n.iter().any(|n| n == "default_locale"))
    {
        out.push(locale_setting(cx)?);
    }
    for def in cx.defs.iter() {
        if hidden(cx, def)? {
            continue;
        }
        if let Some(categories) = filters.categories.as_ref().filter(|c| !c.is_empty())
            && !categories.contains(&def.category)
        {
            continue;
        }
        if let Some(plugin) = &filters.plugin
            && def.plugin.as_deref() != Some(plugin.as_str())
        {
            continue;
        }
        if let Some(names) = &filters.names
            && !names.contains(&def.name)
        {
            continue;
        }
        out.push(entry(conn, cx, def).await?);
    }
    Ok(out)
}

fn opt<'a>(def: &'a Definition, key: &str) -> Option<&'a Yaml> {
    def.options
        .as_ref()
        .and_then(|o| o.get(Yaml::String(key.into())))
        .filter(|v| !v.is_null())
}

fn opt_str(def: &Definition, key: &str) -> Option<String> {
    opt(def, key).and_then(|v| match v {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Number(n) => Some(n.to_string()),
        Yaml::Bool(b) => Some(b.to_string()),
        _ => None,
    })
}

fn yaml_json(v: &Yaml) -> Json {
    serde_json::to_value(v).unwrap_or(Json::Null)
}

/// HiddenProvider#all: `hidden: true`, shadowed by a present GlobalSetting,
/// or named in the `hide_settings` of an upcoming change that is enabled
/// (UpcomingChanges.settings_hidden_while_enabled).
fn hidden(cx: &Context<'_>, def: &Definition) -> Result<bool, AppError> {
    if opt(def, "hidden").is_some_and(|v| v.as_bool() == Some(true)) {
        return Ok(true);
    }
    if cx.globals.get(&def.name).is_some_and(|v| !v.is_empty()) {
        return Ok(true);
    }
    for change in cx.defs.iter().filter(|d| d.is_upcoming_change()) {
        let hides = opt(change, "upcoming_change")
            .and_then(|c| c.get("hide_settings"))
            .is_some_and(|h| match h {
                Yaml::Sequence(names) => {
                    names.iter().any(|n| n.as_str() == Some(def.name.as_str()))
                }
                Yaml::String(n) => *n == def.name,
                _ => false,
            });
        if hides && cx.settings.get(&change.name)?.truthy() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `areas[name]&.first`: the first of the declared areas, or the first
/// character of an area a plugin stored as a string.
fn primary_area(def: &Definition) -> Option<String> {
    if let Some(area) = setting_enums::string_area(&def.name) {
        return area.chars().next().map(String::from);
    }
    opt_str(def, "area").and_then(|a| a.split('|').next().map(str::to_string))
}

/// The `default_locale` entry all_settings puts first.
fn locale_setting(cx: &Context<'_>) -> Result<Json, AppError> {
    let recorded = setting_enums::recorded("default_locale").ok_or(Unsupported(
        "the default_locale entry without recorded locales",
    ))?;
    let valid_values = recorded.get("valid_values").cloned().unwrap_or_default();
    let translate_names = recorded.get("translate_names").cloned().unwrap_or_default();
    Ok(json!({
        "setting": "default_locale",
        "humanized_name": humanized_name("default_locale"),
        "default": "en",
        "category": "required",
        "primary_area": "localization",
        "description": description(cx, "default_locale"),
        "type": "locale_enum",
        "preview": null,
        "value": cx.settings.get("default_locale")?.to_s(),
        "valid_values": valid_values,
        "translate_names": translate_names,
    }))
}

async fn entry(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    def: &Definition,
) -> Result<Json, AppError> {
    let name = def.name.as_str();
    let mut default = default_for(cx, def)?.to_s();
    let mut value = cx.settings.get(name)?.to_s();
    // A themeable setting reads as the default theme sets it.
    if def.themeable {
        let row: Option<(i32, Option<String>)> = sqlx::query_as(
            "SELECT data_type, value FROM theme_site_settings WHERE theme_id = $1 AND name = $2",
        )
        .bind(cx.settings.get("default_theme_id")?.to_i() as i32)
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((data_type, raw)) = row {
            value =
                crate::site_settings::to_rb_value(cx.defs, name, data_type, raw.as_deref())?.to_s();
        }
    }
    let mut upload = None;
    match def.data_type {
        DataType::Upload => {
            // Seeded uploads (negative ids) default to their url.
            if crate::ruby::to_i(&default) < 0 {
                default = upload_url(conn, crate::ruby::to_i(&default) as i32)
                    .await?
                    .unwrap_or_default();
            }
            let id = crate::ruby::to_i(&value) as i32;
            if id != 0 {
                let row: Option<UploadRow> = sqlx::query_as(
                    "SELECT url, original_filename, filesize, width, height, extension FROM uploads WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
                if let Some(row) = row {
                    let (width, height) = dimensions(cx, &row).await?;
                    upload = Some(json!({
                        "original_filename": row.original_filename,
                        "human_filesize": human_filesize(row.filesize),
                        "width": width,
                        "height": height,
                    }));
                    value = row.url;
                } else {
                    value = String::new();
                }
            }
        }
        DataType::Objects if objects_hold_uploads(def) => {
            return Err(Unsupported("objects settings with uploads").into());
        }
        _ => {}
    }

    let mut e = Map::new();
    e.insert("setting".into(), json!(name));
    e.insert("humanized_name".into(), json!(humanized_name(name)));
    e.insert("description".into(), json!(description(cx, name)));
    e.insert("keywords".into(), json!(keywords(cx, name)));
    e.insert("category".into(), json!(def.category));
    e.insert("primary_area".into(), json!(primary_area(def)));
    e.insert("default".into(), json!(default));
    e.insert("value".into(), json!(value));
    e.insert("preview".into(), json!(opt_str(def, "preview")));
    e.insert(
        "secret".into(),
        json!(opt(def, "secret").is_some_and(|v| v.as_bool() == Some(true))),
    );
    e.insert("placeholder".into(), placeholder(conn, cx, name).await?);
    e.insert("mandatory_values".into(), json!(def.mandatory_values));
    e.insert(
        "disallowed_groups".into(),
        opt(def, "disallowed_groups")
            .map(yaml_json)
            .unwrap_or(Json::Null),
    );
    e.insert(
        "requires_confirmation".into(),
        json!(
            opt_str(def, "requires_confirmation")
                .filter(|r| REQUIRES_CONFIRMATION.contains(&r.as_str()))
        ),
    );
    e.insert("upcoming_change".into(), Json::Null);
    e.insert("themeable".into(), json!(def.themeable));
    // type_supervisor.dependencies: an empty list is still present.
    e.insert("depends_on".into(), json!(def.depends_on));
    e.insert(
        "depends_on_humanized_names".into(),
        json!(
            def.depends_on
                .iter()
                .map(|d| humanized_name(d))
                .collect::<Vec<_>>()
        ),
    );
    e.insert(
        "depends_behavior".into(),
        json!(opt_str(def, "depends_behavior")),
    );
    if !def.depends_on_values.is_empty() {
        e.insert("depends_on_values".into(), json!(def.depends_on_values));
    }
    if let Some(display) = opt_str(def, "dependent_setting_display") {
        e.insert("dependent_setting_display".into(), json!(display));
    }
    if let Some((change, new_default)) = &def.default_override
        && cx.settings.get(change)?.truthy()
    {
        e.insert(
            "upcoming_change_default_override_metadata".into(),
            json!({
                "old_default": def.default.to_s(),
                "new_default": new_default.to_s(),
                "change_setting_name": change,
            }),
        );
    }
    type_hash(conn, cx, def, &mut e).await?;
    if let Some(plugin) = &def.plugin {
        e.insert("plugin".into(), json!(plugin));
    }
    if let Some(upload) = upload {
        e.insert("upload".into(), upload);
    }
    Ok(Json::Object(e))
}

/// `defaults.get(name, default_locale)`, with an enabled upcoming change's
/// default override.
fn default_for<'a>(cx: &Context<'a>, def: &'a Definition) -> Result<&'a Value, AppError> {
    if let Some((change, new_default)) = &def.default_override
        && cx.settings.get(change)?.truthy()
    {
        return Ok(new_default);
    }
    let locale = cx.settings.get("default_locale")?.to_s();
    Ok(def.locale_defaults.get(&locale).unwrap_or(&def.default))
}

async fn upload_url(conn: &mut PgConnection, id: i32) -> Result<Option<String>, AppError> {
    Ok(sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?)
}

fn objects_hold_uploads(def: &Definition) -> bool {
    opt(def, "schema")
        .is_some_and(|s| serde_yaml_ng::to_string(s).is_ok_and(|s| s.contains("type: upload")))
}

/// `ActiveSupport::NumberHelper.number_to_human_size`
fn human_filesize(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["Bytes", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} {}", if bytes == 1 { "Byte" } else { "Bytes" });
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    // Three significant digits, trailing zeros dropped.
    let digits = if size >= 100.0 {
        0
    } else if size >= 10.0 {
        1
    } else {
        2
    };
    let rounded = format!("{size:.digits$}");
    let rounded = if rounded.contains('.') {
        rounded
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        rounded
    };
    format!("{rounded} {}", UNITS[unit])
}

/// TypeSupervisor#type_hash
async fn type_hash(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    def: &Definition,
    e: &mut Map<String, Json>,
) -> Result<(), AppError> {
    let type_name = type_name(def);
    e.insert("type".into(), json!(type_name));
    // valid_values, translate_names, choices and json_schema as Ruby
    // computes them: recorded, or from the database for the enum classes
    // that read it.
    if let Some(recorded) = setting_enums::recorded(&def.name) {
        for (k, v) in recorded {
            e.insert(k.clone(), v.clone());
        }
    } else if let Some(class) = opt_str(def, "enum")
        && (type_name == "enum" || type_name == "list")
    {
        let (values, translate) = setting_enums::runtime(conn, cx, &class).await?;
        e.insert("valid_values".into(), values);
        e.insert("translate_names".into(), json!(translate));
    }
    if type_name == "integer" || type_name == "file_size_restriction" {
        for key in ["min", "max"] {
            if let Some(v) = opt(def, key) {
                e.insert(key.into(), yaml_json(v));
            }
        }
    }
    if type_name == "list" {
        e.insert(
            "allow_any".into(),
            json!(opt(def, "allow_any").and_then(Yaml::as_bool) != Some(false)),
        );
    }
    if type_name == "list"
        && let Some(list_type) = opt_str(def, "list_type")
    {
        e.insert("list_type".into(), json!(list_type));
    }
    for key in [
        "textarea",
        "schema",
        "authorized_extensions",
        "max_file_size_kb",
    ] {
        if let Some(v) = opt(def, key) {
            e.insert(key.into(), yaml_json(v));
        }
    }
    Ok(())
}

/// TypeSupervisor#get_data_type: the declared `type` when it is one of
/// the types (some only name a validation), an enum class's `enum`, else
/// the class of the default (parse_value_type).
fn type_name(def: &Definition) -> String {
    if let Some(t) = opt_str(def, "type").filter(|t| DataType::from_name(t).is_some()) {
        return t;
    }
    if opt(def, "enum").is_some() && opt_str(def, "type").is_none() {
        return "enum".into();
    }
    match &def.default {
        Value::Null => "null",
        Value::Str(_) => "string",
        Value::Int(_) => "integer",
        Value::Float(_) => "float",
        Value::Bool(_) => "bool",
        Value::List(_) => "string",
    }
    .into()
}

/// `CGI.escape`
fn cgi_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// LabelFormatter.humanized_name
pub fn humanized_name(setting: &str) -> String {
    const ACRONYMS: &[&str] = &[
        "2fa", "acl", "ai", "api", "arn", "aws", "bg", "cdn", "cors", "csp", "csrf", "css", "cta",
        "csv", "cx", "db", "dm", "dns", "eu", "faq", "fg", "ga", "gb", "gif", "gpu", "gpt", "gtm",
        "hd", "html", "http", "https", "iam", "id", "imap", "ip", "jpg", "json", "kb", "llm", "mb",
        "mfa", "oauth", "oidc", "pdf", "pm", "png", "pop3", "rest", "rss", "s3", "saml", "smtp",
        "sso", "svg", "tei", "tl", "tl0", "tl1", "tl2", "tl3", "tl4", "tld", "totp", "txt", "ui",
        "url", "ux", "vpc", "xml", "yaml", "yml",
    ];
    const MIXED_CASE: [(&str, &str); 45] = [
        ("apple", "Apple"),
        ("adobe analytics", "Adobe Analytics"),
        ("amazon web services", "Amazon Web Services"),
        ("android", "Android"),
        ("chinese", "Chinese"),
        ("discord", "Discord"),
        ("discourse", "Discourse"),
        ("discourse connect", "Discourse Connect"),
        ("discourse discover", "Discourse Discover"),
        ("discourse narrative bot", "Discourse Narrative Bot"),
        ("facebook", "Facebook"),
        ("foundation", "Foundation"),
        ("github", "GitHub"),
        ("google", "Google"),
        ("google analytics", "Google Analytics"),
        ("google tag manager", "Google Tag Manager"),
        ("gravatar", "Gravatar"),
        ("gravatars", "Gravatars"),
        ("gitter", "Gitter"),
        ("horizon", "Horizon"),
        ("ios", "iOS"),
        ("japanese", "Japanese"),
        ("linkedin", "LinkedIn"),
        ("meta", "Meta"),
        ("mediaconvert", "MediaConvert"),
        ("microsoft", "Microsoft"),
        ("matrix", "Matrix"),
        ("mattermost", "Mattermost"),
        ("oauth2", "OAuth2"),
        ("openid connect", "OpenID Connect"),
        ("openai", "OpenAI"),
        ("opengraph", "OpenGraph"),
        ("powered by discourse", "Powered by Discourse"),
        ("tiktok", "TikTok"),
        ("tos", "ToS"),
        ("twitter", "Twitter"),
        ("telegram", "Telegram"),
        ("teams", "Teams"),
        ("rocketchat", "RocketChat"),
        ("slack", "Slack"),
        ("vimeo", "Vimeo"),
        ("wordpress", "WordPress"),
        ("webex", "WebEx"),
        ("youtube", "YouTube"),
        ("zulip", "Zulip"),
    ];
    let name = setting.replace('_', " ");
    let mut words: Vec<String> = name
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    if let Some(first) = words.first_mut() {
        *first = capitalize(first);
    }
    for word in &mut words {
        let lower = word.to_lowercase();
        if ACRONYMS.contains(&lower.as_str()) {
            *word = word.to_uppercase();
        } else if word.ends_with('s') && ACRONYMS.contains(&&lower[..lower.len() - 1]) {
            *word = format!("{}s", lower[..lower.len() - 1].to_uppercase());
        }
    }
    // HUMANIZED_MIXED_CASE_REGEX, compiled once.
    static MIXED_CASE_REGEX: std::sync::OnceLock<Vec<(regex::Regex, &str)>> =
        std::sync::OnceLock::new();
    let patterns = MIXED_CASE_REGEX.get_or_init(|| {
        MIXED_CASE
            .iter()
            .map(|(key, replacement)| {
                let re = regex::RegexBuilder::new(&format!(r"\b{}\b", regex::escape(key)))
                    .case_insensitive(true)
                    .build()
                    .expect("a literal pattern");
                (re, *replacement)
            })
            .collect()
    });
    let mut result = words.join(" ");
    for (re, replacement) in patterns {
        result = re.replace_all(&result, *replacement).into_owned();
    }
    result
}

/// `String#capitalize`: the first character up, the rest down.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// LabelFormatter.description: the locale's text with `%{base_path}`, and
/// `{{setting:name}}` markers expanded into links.
pub fn description(cx: &Context<'_>, setting: &str) -> String {
    // Only `base_path` is given: other `%{...}` stay as written, and an
    // escaped `%%{` becomes `%{`, as I18n interpolates.
    let text = cx
        .i18n
        .t(&format!("site_settings.{setting}"))
        .unwrap_or_default()
        .replace("%{base_path}", cx.base_path)
        .replace("%%{", "%{");
    expand_setting_links(cx, &text)
}

/// LabelFormatter.expand_setting_links
fn expand_setting_links(cx: &Context<'_>, text: &str) -> String {
    if !text.contains("{{setting") {
        return text.to_string();
    }
    static MANY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static ONE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let many = MANY.get_or_init(|| {
        regex::Regex::new(
            r"\{\{settings:([a-z][a-z0-9_]*(?:,[a-z][a-z0-9_]*)*)(?:\|([^{}|]+))?\}\}",
        )
        .unwrap()
    });
    let one = ONE.get_or_init(|| regex::Regex::new(r"\{\{setting:([a-z][a-z0-9_]*)\}\}").unwrap());
    let text = many.replace_all(text, |c: &regex::Captures| {
        let settings: Vec<&str> = c[1].split(',').collect();
        let label = c
            .get(2)
            .map(|l| l.as_str().trim().to_string())
            .unwrap_or_else(|| {
                settings
                    .iter()
                    .map(|s| humanized_name(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
        let filter = format!("any:{}", settings.join("|"));
        format!(
            r#"<a class="site-setting-link" href="{}">{}</a>"#,
            escape_html(&filter_href(cx, &filter)),
            escape_html(&label)
        )
    });
    one.replace_all(&text, |c: &regex::Captures| linkify(cx, &c[1]))
        .into_owned()
}

fn filter_href(cx: &Context<'_>, filter: &str) -> String {
    format!(
        "{}/admin/site_settings/category/all_results?filter={}",
        cx.base_path,
        cgi_escape(filter)
    )
}

/// LabelFormatter.linkify
fn linkify(cx: &Context<'_>, setting: &str) -> String {
    let def = cx.defs.get(setting);
    let mut attrs = vec![
        ("class", "site-setting-link".to_string()),
        ("href", filter_href(cx, setting)),
        ("data-setting-name", setting.to_string()),
    ];
    if let Some(area) = def.and_then(primary_area) {
        attrs.push(("data-setting-area", area));
    }
    if let Some(def) = def {
        attrs.push(("data-setting-category", def.category.clone()));
        if let Some(plugin) = &def.plugin {
            attrs.push(("data-setting-plugin", plugin.clone()));
        }
    }
    let attrs: Vec<String> = attrs
        .iter()
        .map(|(k, v)| format!(r#"{k}="{}""#, escape_html(v)))
        .collect();
    format!(
        "<a {}>{}</a>",
        attrs.join(" "),
        escape_html(&humanized_name(setting))
    )
}

/// `CGI.escapeHTML`
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// LabelFormatter.keywords: the locale's `|`-separated keywords, then the
/// names the setting was renamed from (DeprecatedSettings).
fn keywords(cx: &Context<'_>, setting: &str) -> Vec<String> {
    let mut words: Vec<String> = cx
        .i18n
        .t(&format!("site_settings.keywords.{setting}"))
        .map(|k| k.split('|').map(str::to_string).collect())
        .unwrap_or_default();
    words.extend(setting_enums::deprecated_aliases(setting));
    words
}

/// LabelFormatter.placeholder: the locale's, else for SiteIconManager's
/// icons the url it resolves (`<icon>_url`, "" for none).
async fn placeholder(
    conn: &mut PgConnection,
    cx: &Context<'_>,
    setting: &str,
) -> Result<Json, AppError> {
    const ICONS: [&str; 8] = [
        "digest_logo",
        "mobile_logo",
        "mobile_logo_dark",
        "large_icon",
        "manifest_icon",
        "favicon",
        "apple_touch_icon",
        "opengraph_image",
    ];
    let key = format!("site_settings.placeholder.{setting}");
    if let Some(p) = cx.i18n.t(&key).filter(|p| !p.is_empty()) {
        return Ok(json!(p));
    }
    // A placeholder may be a hash (a key and a value, for a list of pairs).
    static NESTED: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    let nested = NESTED.get_or_init(|| {
        cx.i18n
            .keys_with_prefix("site_settings.placeholder.")
            .filter_map(|k| {
                let rest = k.strip_prefix("site_settings.placeholder.")?;
                rest.split_once('.').map(|(name, _)| name.to_string())
            })
            .collect()
    });
    if nested.contains(setting)
        && let Some(tree) = cx.i18n.subtree(&key)
    {
        return Ok(tree);
    }
    if ICONS.contains(&setting) {
        let url = crate::site_icons::site_url(conn, cx.urls, setting).await?;
        return Ok(json!(url));
    }
    Ok(Json::Null)
}

#[derive(sqlx::FromRow)]
struct UploadRow {
    url: String,
    original_filename: Option<String>,
    filesize: i64,
    width: Option<i32>,
    height: Option<i32>,
    extension: Option<String>,
}

/// `Upload#width`/`#height` (get_dimension): the stored ones, else, for an
/// image, `fix_dimensions!` measures the file (unsaved): the site's own
/// public directory, then the stock images of a Discourse checkout, then
/// the stock images' recorded sizes.
async fn dimensions(
    cx: &Context<'_>,
    row: &UploadRow,
) -> Result<(Option<i32>, Option<i32>), AppError> {
    if row.width.is_some() && row.height.is_some() {
        return Ok((row.width, row.height));
    }
    let is_image = row
        .extension
        .as_deref()
        .is_some_and(|e| crate::images::Format::from_extension(e).is_some());
    if !is_image || !row.url.starts_with('/') || row.url.starts_with("//") {
        return Ok((row.width, row.height));
    }
    let relative = row.url.trim_start_matches('/');
    let config = cx.urls.config;
    let mut candidates = vec![config.public_dir.join(relative)];
    if let Some(src) = &config.discourse_src {
        candidates.push(src.join("public").join(relative));
    }
    for path in candidates {
        if let Ok(bytes) = tokio::fs::read(&path).await
            && let Some(info) = crate::images::info(&bytes)
        {
            return Ok((Some(info.width as i32), Some(info.height as i32)));
        }
    }
    if let Some((width, height)) = setting_enums::stock_upload_dimensions(&row.url) {
        return Ok((Some(width), Some(height)));
    }
    Err(Unsupported("the dimensions of an upload whose file is missing").into())
}
