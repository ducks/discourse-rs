//! Themes and components as the admin reads them: Theme's rows and
//! fields, ThemeSettingsParser over the settings YAML, the themeable site
//! settings, and ThemeSerializer with BasicThemeSerializer,
//! ThemeIndexSerializer and ComponentIndexSerializer.
//!
//! Remote themes show through RemoteThemeSerializer, and decide
//! `supported?` from their minimum and maximum Discourse versions.
//!
//! Refused: theme settings of the list, upload, objects and icon types.

use serde_json::{Map, Value, json};
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::color_scheme::ColorScheme;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `Theme.targets`
pub const TARGETS: [(i32, &str); 10] = [
    (0, "common"),
    (1, "desktop"),
    (2, "mobile"),
    (3, "settings"),
    (4, "translations"),
    (5, "extra_scss"),
    (6, "extra_js"),
    (7, "tests_js"),
    (8, "migrations"),
    (9, "about"),
];

/// `ThemeField.types`
pub const FIELD_TYPES: [(i32, &str); 7] = [
    (0, "html"),
    (1, "scss"),
    (2, "theme_upload_var"),
    (5, "yaml"),
    (6, "js"),
    (7, "theme_screenshot_upload_var"),
    (8, "json"),
];

/// `SiteSetting.themeable_site_settings`, sorted.
pub const THEMEABLE_SITE_SETTINGS: [&str; 2] = ["enable_welcome_banner", "search_experience"];

pub fn target_name(id: i32) -> Option<&'static str> {
    TARGETS.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
}

fn type_name(id: i32) -> Option<&'static str> {
    FIELD_TYPES.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
}

/// A `themes` row.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Theme {
    pub id: i32,
    pub name: String,
    pub user_id: i32,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
    pub component: bool,
    pub color_scheme_id: Option<i32>,
    pub dark_color_scheme_id: Option<i32>,
    pub user_selectable: bool,
    pub auto_update: bool,
    pub remote_theme_id: Option<i32>,
    pub enabled: bool,
}

const COLUMNS: &str = "id, name, user_id, created_at, updated_at, component, color_scheme_id, \
    dark_color_scheme_id, user_selectable, auto_update, remote_theme_id, enabled";

impl Theme {
    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Theme>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM themes WHERE id = $1"))
            .bind(id)
            .fetch_optional(conn)
            .await
    }

    /// Themes matching a condition, `order(:name)`.
    pub async fn where_ordered(
        conn: &mut PgConnection,
        condition: &str,
    ) -> Result<Vec<Theme>, sqlx::Error> {
        Self::select(conn, &format!("WHERE {condition} ORDER BY name")).await
    }

    /// Themes by a WHERE, ORDER and LIMIT tail of the caller's.
    pub async fn select(conn: &mut PgConnection, tail: &str) -> Result<Vec<Theme>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM themes {tail}"))
            .fetch_all(conn)
            .await
    }

    pub fn system(&self) -> bool {
        self.id < 0
    }
}

/// A `theme_fields` row.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct Field {
    pub id: i32,
    pub name: String,
    pub target_id: i32,
    pub type_id: i32,
    pub value: String,
    pub error: Option<String>,
    pub upload_id: Option<i32>,
}

/// `theme.theme_fields`, in the table's order (the association has none).
pub async fn fields(conn: &mut PgConnection, theme_id: i32) -> Result<Vec<Field>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, name, target_id, type_id, value, error, upload_id FROM theme_fields \
         WHERE theme_id = $1",
    )
    .bind(theme_id)
    .fetch_all(conn)
    .await
}

/// `ThemeField#file_path` (FILE_MATCHERS' canonical names).
fn file_path(f: &Field, upload_filename: Option<&str>) -> Option<String> {
    let target = target_name(f.target_id)?;
    let kind = type_name(f.type_id)?;
    let name = f.name.as_str();
    match (kind, target) {
        ("html", "mobile" | "desktop" | "common")
            if matches!(
                name,
                "head_tag" | "header" | "after_header" | "body_tag" | "footer"
            ) =>
        {
            Some(format!("{target}/{name}.html"))
        }
        ("scss", "mobile" | "desktop" | "common") if name == "scss" => {
            Some(format!("{target}/{target}.scss"))
        }
        ("scss", "common") if name == "embedded_scss" => Some("common/embedded.scss".into()),
        ("scss", "common") if name == "color_definitions" => {
            Some("common/color_definitions.scss".into())
        }
        ("scss", "extra_scss") => Some(format!("stylesheets/{name}.scss")),
        ("js", "extra_js") => Some(format!("javascripts/{name}")),
        ("js", "tests_js") => Some(format!("test/{name}")),
        ("json", "about") if name == "about" => Some("about.json".into()),
        ("yaml", "settings") if name == "yaml" => Some("settings.yml".into()),
        ("yaml", "translations") => locale_available(name).then(|| format!("locales/{name}.yml")),
        ("theme_upload_var", "common") => {
            let ext = upload_filename
                .and_then(|f| f.rfind('.').filter(|i| *i > 0).map(|i| &f[i..]))
                .unwrap_or("");
            Some(format!("assets/{name}{ext}"))
        }
        ("js", "migrations") => Some(format!("migrations/settings/{name}.js")),
        _ => None,
    }
}

/// `ThemeField#file_path` of a field, its upload's filename looked up.
pub async fn field_file_path(
    conn: &mut PgConnection,
    f: &Field,
) -> Result<Option<String>, sqlx::Error> {
    let filename = match f.upload_id {
        Some(id) => upload_url(conn, id).await?.map(|(_, name)| name),
        None => None,
    };
    Ok(file_path(f, filename.as_deref()))
}

/// `I18n.available_locales` (LocaleSiteSetting.supported_locales).
pub fn locale_available(name: &str) -> bool {
    crate::setting_enums::recorded("default_locale")
        .and_then(|r| r.get("valid_values"))
        .and_then(Value::as_array)
        .is_some_and(|values| {
            values
                .iter()
                .any(|v| v.get("value").and_then(Value::as_str) == Some(name))
        })
}

pub fn json_time(t: chrono::NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// What serializing needs beside the connection.
pub struct Ctx<'a> {
    pub settings: &'a SiteSettings,
    pub i18n: &'a crate::i18n::I18n,
    pub urls: &'a crate::url::Urls<'a>,
    pub defs: &'a crate::site_settings::Definitions,
}

/// BasicThemeSerializer: id, name, description, created_at, updated_at,
/// `default` for the default theme, component.
pub async fn basic(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    t: &Theme,
) -> Result<Map<String, Value>, AppError> {
    let description = crate::site_texts::theme_translations_of(conn, t.id, true)
        .await?
        .into_iter()
        .find(|e| e.key == "theme_metadata.description")
        .map(|e| e.value().to_string());
    let mut out = Map::new();
    out.insert("id".into(), json!(t.id));
    out.insert("name".into(), json!(t.name));
    out.insert("description".into(), json!(description));
    out.insert("created_at".into(), json!(json_time(t.created_at)));
    out.insert("updated_at".into(), json!(json_time(t.updated_at)));
    if i64::from(t.id) == cx.settings.get("default_theme_id")?.to_i() {
        out.insert("default".into(), json!(true));
    }
    out.insert("component".into(), json!(t.component));
    Ok(out)
}

async fn upload_url(
    conn: &mut PgConnection,
    id: i32,
) -> Result<Option<(String, String)>, sqlx::Error> {
    sqlx::query_as("SELECT url, original_filename FROM uploads WHERE id = $1")
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// `screenshot_upload(name)&.url`
async fn screenshot_url(
    conn: &mut PgConnection,
    fields: &[Field],
    name: &str,
) -> Result<Option<String>, sqlx::Error> {
    let Some(upload) = fields
        .iter()
        .find(|f| f.name == name && f.type_id == 7)
        .and_then(|f| f.upload_id)
    else {
        return Ok(None);
    };
    Ok(upload_url(conn, upload).await?.map(|(url, _)| url))
}

/// UserNameSerializer
pub async fn user_name(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    user_id: Option<i32>,
) -> Result<Value, AppError> {
    let Some(user_id) = user_id else {
        return Ok(Value::Null);
    };
    type Row = (String, Option<String>, Option<i32>, Option<String>);
    let row: Option<Row> =
        sqlx::query_as("SELECT username, name, uploaded_avatar_id, title FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((username, name, avatar, title)) = row else {
        return Ok(Value::Null);
    };
    let logo_small = crate::admin_users::logo_small_url(&mut *conn, cx.settings).await?;
    let avatar_template =
        crate::avatar::avatar_template(cx.urls, user_id, &username, avatar, logo_small.as_deref())?;
    Ok(json!({
        "id": user_id,
        "username": username,
        "name": name,
        "avatar_template": avatar_template,
        "title": title,
    }))
}

/// `color_scheme` (ColorSchemeSerializer), null without one.
async fn color_scheme(conn: &mut PgConnection, id: Option<i32>) -> Result<Value, AppError> {
    let Some(id) = id else {
        return Ok(Value::Null);
    };
    match ColorScheme::find(conn, id).await? {
        Some(s) => Ok(s.serialize()?),
        None => Ok(Value::Null),
    }
}

/// The child or parent themes through BasicThemeSerializer, by name.
async fn related(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    theme_id: i32,
    children: bool,
) -> Result<Value, AppError> {
    let condition = if children {
        format!(
            "id IN (SELECT child_theme_id FROM child_themes WHERE parent_theme_id = {theme_id})"
        )
    } else {
        format!(
            "id IN (SELECT parent_theme_id FROM child_themes WHERE child_theme_id = {theme_id})"
        )
    };
    let mut out = Vec::new();
    for t in Theme::where_ordered(conn, &condition).await? {
        out.push(Value::Object(basic(conn, cx, &t).await?));
    }
    Ok(Value::Array(out))
}

/// A `remote_themes` row.
#[derive(sqlx::FromRow, Clone, Debug)]
pub struct RemoteTheme {
    pub id: i32,
    pub remote_url: String,
    pub remote_version: Option<String>,
    pub local_version: Option<String>,
    pub commits_behind: Option<i32>,
    pub branch: Option<String>,
    pub local_compat_ref: Option<String>,
    pub remote_compat_ref: Option<String>,
    pub remote_updated_at: Option<chrono::NaiveDateTime>,
    pub updated_at: chrono::NaiveDateTime,
    pub last_error_text: Option<String>,
    pub license_url: Option<String>,
    pub about_url: Option<String>,
    pub authors: Option<String>,
    pub theme_version: Option<String>,
    pub minimum_discourse_version: Option<String>,
    pub maximum_discourse_version: Option<String>,
    pub private_key: Option<String>,
}

impl RemoteTheme {
    pub async fn find(
        conn: &mut PgConnection,
        id: Option<i32>,
    ) -> Result<Option<RemoteTheme>, sqlx::Error> {
        let Some(id) = id else {
            return Ok(None);
        };
        sqlx::query_as(
            "SELECT id, remote_url, remote_version, local_version, commits_behind, branch, \
             local_compat_ref, remote_compat_ref, remote_updated_at, updated_at, last_error_text, \
             license_url, about_url, authors, theme_version, minimum_discourse_version, \
             maximum_discourse_version, private_key FROM remote_themes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await
    }

    /// `is_git?`
    pub fn is_git(&self) -> bool {
        !crate::ruby::is_blank(&self.remote_url)
    }

    /// `github_repo_url`
    fn github_repo_url(&self) -> Option<String> {
        let url = crate::ruby::strip(&self.remote_url);
        if url.starts_with("https://github.com/") || url.starts_with("http://github.com/") {
            return Some(url.to_string());
        }
        url.strip_prefix("ssh://git@github.com:")
            .map(|rest| format!("https://github.com/{rest}"))
    }

    /// `github_diff_link`
    fn github_diff_link(&self) -> Option<String> {
        let repo = self.github_repo_url().filter(|u| !u.is_empty())?;
        if self.local_version == self.remote_version {
            return None;
        }
        Some(format!(
            "{}/compare/{}...{}",
            repo.strip_suffix(".git").unwrap_or(&repo),
            self.local_version.as_deref().unwrap_or(""),
            self.remote_version.as_deref().unwrap_or("")
        ))
    }

    /// RemoteThemeSerializer
    pub fn serialize(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("remote_url".into(), json!(self.remote_url));
        out.insert("remote_version".into(), json!(self.remote_version));
        out.insert("local_version".into(), json!(self.local_version));
        out.insert("commits_behind".into(), json!(self.commits_behind));
        out.insert("branch".into(), json!(self.branch));
        out.insert("local_compat_ref".into(), json!(self.local_compat_ref));
        out.insert("remote_compat_ref".into(), json!(self.remote_compat_ref));
        out.insert(
            "remote_updated_at".into(),
            json!(self.remote_updated_at.map(json_time)),
        );
        out.insert("updated_at".into(), json!(json_time(self.updated_at)));
        if let Some(link) = self.github_diff_link() {
            out.insert("github_diff_link".into(), json!(link));
        }
        out.insert("last_error_text".into(), json!(self.last_error_text));
        out.insert("is_git".into(), json!(self.is_git()));
        let valid = |u: &Option<String>| u.clone().filter(|u| valid_url(u));
        out.insert("license_url".into(), json!(valid(&self.license_url)));
        out.insert("about_url".into(), json!(valid(&self.about_url)));
        out.insert("authors".into(), json!(self.authors));
        out.insert("theme_version".into(), json!(self.theme_version));
        out.insert(
            "minimum_discourse_version".into(),
            json!(self.minimum_discourse_version),
        );
        out.insert(
            "maximum_discourse_version".into(),
            json!(self.maximum_discourse_version),
        );
        out.insert(
            "has_private_key".into(),
            json!(
                self.private_key
                    .as_deref()
                    .is_some_and(|k| !crate::ruby::is_blank(k))
            ),
        );
        Value::Object(out)
    }
}

/// `UrlHelper.is_valid_url?`: an absolute path, an anchor, a mailto, or
/// an http(s), ftp or ldap url with a host, that Ruby's URI parses.
pub fn valid_url(url: &str) -> bool {
    // URI::RFC3986_Parser: the characters a URI reference may hold.
    let parses = url
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'()*+,;=%".contains(&b))
        && url.matches('#').count() <= 1;
    if !parses {
        return false;
    }
    if url.starts_with('/') || url.starts_with('#') {
        return true;
    }
    let scheme_end = url.find(':');
    let Some(scheme) = scheme_end.map(|i| &url[..i]).filter(|s| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    }) else {
        return false;
    };
    let lower = scheme.to_ascii_lowercase();
    if lower == "mailto" {
        return true;
    }
    let rest = &url[scheme.len()..];
    matches!(lower.as_str(), "http" | "https" | "ftp" | "ldap" | "ldaps")
        && rest.starts_with("://")
        && rest[3..].chars().next().is_some_and(|c| c != '/')
}

/// `Gem::Version.new(current) >= Gem::Version.new(needed)`, false when
/// either is not a version (`VersionCompatibility.has_needed_version?`).
pub fn has_needed_version(current: &str, needed: &str) -> bool {
    match (gem_version(current), gem_version(needed)) {
        (Some(a), Some(b)) => compare_gem_versions(&a, &b) != std::cmp::Ordering::Less,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Num(u64),
    Str(String),
}

/// Gem::Version's canonical segments, None for a malformed version.
fn gem_version(raw: &str) -> Option<Vec<Segment>> {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\A\s*([0-9]+(?:\.[0-9a-zA-Z]+)*(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?)?\s*\z",
        )
        .expect("valid regex")
    });
    if !PATTERN.is_match(raw) {
        return None;
    }
    let version = raw.trim().replace('-', ".pre.");
    let version = if version.is_empty() {
        "0".to_string()
    } else {
        version
    };
    static PART: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"[0-9]+|[a-zA-Z]+").expect("valid regex"));
    let segments: Vec<Segment> = PART
        .find_iter(&version)
        .map(|m| match m.as_str().parse::<u64>() {
            Ok(n) => Segment::Num(n),
            Err(_) => Segment::Str(m.as_str().to_string()),
        })
        .collect();
    // canonical_segments: trailing zeros dropped from the numeric part and
    // from the rest.
    let string_start = segments
        .iter()
        .position(|s| matches!(s, Segment::Str(_)))
        .unwrap_or(segments.len());
    let trim = |part: &[Segment]| -> Vec<Segment> {
        let mut v = part.to_vec();
        while v.last() == Some(&Segment::Num(0)) {
            v.pop();
        }
        v
    };
    let mut canonical = trim(&segments[..string_start]);
    canonical.extend(trim(&segments[string_start..]));
    Some(canonical)
}

fn compare_gem_versions(a: &[Segment], b: &[Segment]) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let len = a.len().max(b.len());
    for i in 0..len {
        let l = a.get(i).cloned().unwrap_or(Segment::Num(0));
        let r = b.get(i).cloned().unwrap_or(Segment::Num(0));
        if l == r {
            continue;
        }
        return match (l, r) {
            (Segment::Str(_), Segment::Num(_)) => Ordering::Less,
            (Segment::Num(_), Segment::Str(_)) => Ordering::Greater,
            (Segment::Num(x), Segment::Num(y)) => x.cmp(&y),
            (Segment::Str(x), Segment::Str(y)) => x.cmp(&y),
        };
    }
    Ordering::Equal
}

/// `Theme#supported?`: the running version within the remote theme's
/// minimum and maximum.
pub fn supported(remote: Option<&RemoteTheme>) -> bool {
    let current = crate::schema::discourse_version();
    let Some(remote) = remote else {
        return true;
    };
    if let Some(min) = &remote.minimum_discourse_version
        && !has_needed_version(current, min)
    {
        return false;
    }
    if let Some(max) = &remote.maximum_discourse_version
        && !has_needed_version(max, current)
    {
        return false;
    }
    true
}

/// `supported?` of a theme row.
pub async fn supported_theme(conn: &mut PgConnection, t: &Theme) -> Result<bool, sqlx::Error> {
    Ok(supported(
        RemoteTheme::find(conn, t.remote_theme_id).await?.as_ref(),
    ))
}

/// `remote_theme` (RemoteThemeSerializer), null without one.
async fn remote_theme(conn: &mut PgConnection, t: &Theme) -> Result<Value, sqlx::Error> {
    Ok(RemoteTheme::find(conn, t.remote_theme_id)
        .await?
        .map_or(Value::Null, |r| r.serialize()))
}

/// `disabled_at` / `disabled_by`: the latest disable_theme_component log.
async fn disabled_log(
    conn: &mut PgConnection,
    t: &Theme,
) -> Result<Option<(chrono::NaiveDateTime, Option<i32>)>, sqlx::Error> {
    let action = crate::staff_action_logs::action_id("disable_theme_component").unwrap_or(-1);
    sqlx::query_as(
        "SELECT created_at, acting_user_id FROM user_histories WHERE action = $1 AND context = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(action)
    .bind(t.id.to_string())
    .fetch_optional(conn)
    .await
}

/// One parsed theme setting (ThemeSettingsParser and its manager).
pub struct Setting {
    pub name: String,
    pub type_name: &'static str,
    pub type_id: i32,
    pub default: Yaml,
    pub description: Option<String>,
    pub choices: Vec<Yaml>,
    pub textarea: bool,
    pub refresh: bool,
    /// `min` and `max` when numeric (integer, float and string settings).
    pub min: Option<Yaml>,
    pub max: Option<Yaml>,
}

/// `Theme#settings`: the settings field parsed, when it has no error.
pub async fn settings_of(conn: &mut PgConnection, theme_id: i32) -> Result<Vec<Setting>, AppError> {
    let field: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT value, error FROM theme_fields WHERE theme_id = $1 AND target_id = 3 AND name = 'yaml' \
         ORDER BY id LIMIT 1",
    )
    .bind(theme_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((src, None)) = field else {
        return Ok(Vec::new());
    };
    if crate::ruby::is_blank(&src) {
        return Ok(Vec::new());
    }
    let parsed: Yaml = serde_yaml_ng::from_str(&src)
        .map_err(|_| Unsupported("theme settings YAML that does not parse"))?;
    let Some(map) = parsed.as_mapping() else {
        return Err(Unsupported("theme settings YAML that is not a hash").into());
    };
    let guess = |v: &Yaml| -> Option<(&'static str, i32)> {
        match v {
            Yaml::Number(n) if n.is_i64() || n.is_u64() => Some(("integer", 0)),
            Yaml::Number(_) => Some(("float", 1)),
            Yaml::String(_) => Some(("string", 2)),
            Yaml::Bool(_) => Some(("bool", 3)),
            _ => None,
        }
    };
    let mut out = Vec::new();
    for (k, v) in map {
        let name = k
            .as_str()
            .ok_or(Unsupported("theme settings with non-string names"))?
            .to_string();
        let (default, kind, opts) = if let Some(kind) = guess(v) {
            (v.clone(), kind, None)
        } else if let Some(h) = v.as_mapping() {
            let default = h.get("default").cloned().unwrap_or(Yaml::Null);
            let kind = match h.get("type") {
                Some(t) => match t.as_str() {
                    Some("integer") => ("integer", 0),
                    Some("float") => ("float", 1),
                    Some("string") => ("string", 2),
                    Some("bool") => ("bool", 3),
                    Some("enum") => ("enum", 5),
                    Some(_) => {
                        return Err(Unsupported(
                            "theme settings of the list, upload, objects and icon types",
                        )
                        .into());
                    }
                    None => return Err(Unsupported("a theme setting without a type").into()),
                },
                None => guess(&default).ok_or(Unsupported("a theme setting without a type"))?,
            };
            (default, kind, Some(h))
        } else {
            return Err(Unsupported("a theme setting without a type").into());
        };
        let opt = |key: &str| opts.and_then(|h| h.get(key));
        let description = match opt("description") {
            Some(Yaml::String(s)) => Some(s.clone()),
            Some(Yaml::Mapping(m)) => m
                .get("en")
                .and_then(Yaml::as_str)
                .map(str::to_string)
                .or_else(|| m.values().find_map(|v| v.as_str().map(str::to_string))),
            _ => None,
        };
        let mut choices: Vec<Yaml> = match opt("choices") {
            Some(Yaml::Sequence(s)) => s.clone(),
            _ => Vec::new(),
        };
        if kind.0 == "enum" && !choices.contains(&default) {
            choices.push(default.clone());
        }
        out.push(Setting {
            name,
            type_name: kind.0,
            type_id: kind.1,
            default,
            description,
            choices,
            textarea: matches!(opt("textarea"), Some(Yaml::Bool(true))),
            refresh: matches!(opt("refresh"), Some(Yaml::Bool(true))),
            min: opt("min").filter(|v| v.is_number()).cloned(),
            max: opt("max").filter(|v| v.is_number()).cloned(),
        });
    }
    Ok(out)
}

pub fn yaml_json(v: &Yaml) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

impl Setting {
    /// `value_for_editing` from the stored row's value, else the default.
    pub fn value(&self, stored: Option<&str>) -> Value {
        let raw = match stored {
            Some(s) => Value::String(s.to_string()),
            None => yaml_json(&self.default),
        };
        match self.type_name {
            "bool" => json!(matches!(&raw, Value::Bool(true)) || raw == json!("true")),
            "integer" => match &raw {
                Value::String(s) => json!(crate::ruby::to_i(s)),
                other => other.clone(),
            },
            "float" => match &raw {
                Value::String(s) => json!(crate::ruby::to_f(s)),
                other => other.clone(),
            },
            "enum" => {
                let as_string = match &raw {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                self.choices
                    .iter()
                    .map(yaml_json)
                    .find(|c| {
                        *c == raw || c.as_str().map_or(c.to_string(), str::to_string) == as_string
                    })
                    .unwrap_or(raw)
            }
            _ => raw,
        }
    }
}

/// `theme_settings` rows of a theme: name, data_type, value.
pub async fn stored_settings(
    conn: &mut PgConnection,
    theme_id: i32,
) -> Result<Vec<(String, i32, Option<String>)>, sqlx::Error> {
    sqlx::query_as("SELECT name, data_type, value FROM theme_settings WHERE theme_id = $1")
        .bind(theme_id)
        .fetch_all(conn)
        .await
}

/// ThemeSettingsSerializer for each setting.
async fn settings_json(conn: &mut PgConnection, theme_id: i32) -> Result<Value, AppError> {
    let settings = settings_of(conn, theme_id).await?;
    let stored = stored_settings(conn, theme_id).await?;
    let internal = crate::site_texts::theme_translations_of(conn, theme_id, true).await?;
    let mut out = Vec::new();
    for s in settings {
        let value = stored
            .iter()
            .find(|(n, t, _)| *n == s.name && *t == s.type_id)
            .and_then(|(_, _, v)| v.as_deref());
        let mut e = Map::new();
        e.insert("setting".into(), json!(s.name));
        e.insert(
            "humanized_name".into(),
            json!(crate::admin_site_settings::humanized_name(&s.name)),
        );
        e.insert("type".into(), json!(s.type_name));
        e.insert("default".into(), yaml_json(&s.default));
        e.insert("value".into(), s.value(value));
        let wanted = [
            format!("theme_metadata.settings.{}", s.name),
            format!("theme_metadata.settings.{}.description", s.name),
        ];
        let description = internal
            .iter()
            .find(|t| wanted.contains(&t.key))
            .map(|t| t.value().to_string())
            .or(s.description.clone());
        if let Some(d) = description.filter(|d| !crate::ruby::is_blank(d)) {
            // I18n.interpolate with base_path, the text as it is when another
            // placeholder has no argument.
            let base_path = "";
            let interpolated = if d.contains("%{") {
                let only_base = d.replace("%{base_path}", base_path);
                if only_base.contains("%{") {
                    d.clone()
                } else {
                    only_base
                }
            } else {
                d.clone()
            };
            e.insert("description".into(), json!(interpolated));
        }
        if s.type_name == "enum" {
            let prefix = format!("theme_metadata.settings.{}.choices.", s.name);
            let labels: Vec<(String, String)> = internal
                .iter()
                .filter_map(|t| {
                    t.key
                        .strip_prefix(&prefix)
                        .map(|k| (k.to_string(), t.value().to_string()))
                })
                .collect();
            let values: Vec<Value> = s
                .choices
                .iter()
                .map(|c| {
                    let key = match c {
                        Yaml::String(s) => s.clone(),
                        other => yaml_json(other).to_string(),
                    };
                    match labels.iter().rev().find(|(k, _)| *k == key) {
                        Some((_, label)) => json!({"name": label, "value": yaml_json(c)}),
                        None => yaml_json(c),
                    }
                })
                .collect();
            e.insert("valid_values".into(), Value::Array(values));
        }
        if s.type_name == "string" {
            e.insert("textarea".into(), json!(s.textarea));
        }
        out.push(Value::Object(e));
    }
    Ok(Value::Array(out))
}

/// `SiteSetting.setting_metadata_hash(name)` for a themeable setting.
pub fn site_setting_metadata(cx: &Ctx<'_>, name: &str) -> Result<Map<String, Value>, AppError> {
    let def = cx.defs.get(name).ok_or(Unsupported(
        "a themeable setting missing from site_settings.yml",
    ))?;
    let mut e = Map::new();
    e.insert("setting".into(), json!(name));
    let default = match &def.default {
        crate::site_settings::Value::Bool(b) => json!(b),
        crate::site_settings::Value::Int(i) => json!(i),
        other => json!(other.to_s()),
    };
    e.insert("default".into(), default);
    let admin_cx = crate::admin_site_settings::Context {
        defs: cx.defs,
        settings: cx.settings,
        i18n: cx.i18n,
        globals: &cx.urls.config.globals,
        base_path: cx.urls.config.globals.relative_url_root(),
        urls: cx.urls,
    };
    let meta = crate::admin_site_settings::setting_meta(&admin_cx, name)
        .ok_or(Unsupported("a themeable setting without metadata"))?;
    e.insert("description".into(), json!(meta.description));
    e.insert("humanized_name".into(), json!(meta.humanized_name));
    e.insert("type".into(), json!(meta.type_name));
    if let Some(recorded) = crate::setting_enums::recorded(name) {
        if let Some(v) = recorded.get("valid_values") {
            e.insert("valid_values".into(), v.clone());
        }
        if let Some(v) = recorded.get("translate_names") {
            e.insert("translate_names".into(), v.clone());
        }
    }
    Ok(e)
}

/// A theme_site_settings row's `setting_rb_value`.
fn site_setting_rb_value(data_type: i32, value: Option<&str>) -> Value {
    let value = value.unwrap_or("");
    match crate::site_settings::DataType::from_id(data_type) {
        Some(crate::site_settings::DataType::Bool) => json!(value == "t" || value == "true"),
        Some(crate::site_settings::DataType::Integer) => json!(crate::ruby::to_i(value)),
        _ => json!(value),
    }
}

/// `themeable_site_settings` (ThemeSiteSettingResolver), default and
/// value as strings, as ThemeSerializer shows them.
async fn themeable_site_settings(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    theme_id: i32,
) -> Result<Value, AppError> {
    let stored: Vec<(String, i32, Option<String>)> = sqlx::query_as(
        "SELECT name, data_type, value FROM theme_site_settings WHERE theme_id = $1",
    )
    .bind(theme_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::new();
    for name in THEMEABLE_SITE_SETTINGS {
        let mut e = site_setting_metadata(cx, name)?;
        let value = match stored.iter().find(|(n, _, _)| n == name) {
            Some((_, t, v)) => site_setting_rb_value(*t, v.as_deref()),
            None => e["default"].clone(),
        };
        let as_string = |v: &Value| match v {
            Value::String(s) => Value::String(s.clone()),
            other => Value::String(other.to_string()),
        };
        let default = as_string(&e["default"]);
        e.insert("default".into(), default);
        e.insert("value".into(), as_string(&value));
        out.push(Value::Object(e));
    }
    Ok(Value::Array(out))
}

/// The value of `enable_welcome_banner` for a theme: its row, else the
/// site setting's default.
pub async fn welcome_banner_value(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    theme_id: i32,
) -> Result<Value, AppError> {
    let stored: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT data_type, value FROM theme_site_settings WHERE theme_id = $1 \
         AND name = 'enable_welcome_banner' LIMIT 1",
    )
    .bind(theme_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match stored {
        Some((t, v)) => site_setting_rb_value(t, v.as_deref()),
        None => site_setting_metadata(cx, "enable_welcome_banner")?["default"].clone(),
    })
}

/// ThemeFieldSerializer
async fn field_json(
    conn: &mut PgConnection,
    f: &Field,
    include_value: bool,
    migrated: bool,
) -> Result<Value, AppError> {
    let upload = match f.upload_id {
        Some(id) => upload_url(conn, id).await?,
        None => None,
    };
    let mut e = Map::new();
    e.insert("name".into(), json!(f.name));
    e.insert("target".into(), json!(target_name(f.target_id)));
    if include_value {
        e.insert("value".into(), json!(f.value));
    }
    if let Some(error) = f.error.as_deref().filter(|e| !crate::ruby::is_blank(e)) {
        e.insert("error".into(), json!(error));
    }
    e.insert("type_id".into(), json!(f.type_id));
    if let Some((url, filename)) = &upload {
        e.insert("upload_id".into(), json!(f.upload_id));
        e.insert("url".into(), json!(url));
        e.insert("filename".into(), json!(filename));
    }
    if target_name(f.target_id) == Some("migrations") {
        e.insert("migrated".into(), json!(migrated));
    }
    e.insert(
        "file_path".into(),
        json!(file_path(f, upload.as_ref().map(|(_, n)| n.as_str()))),
    );
    Ok(Value::Object(e))
}

/// ThemeSerializer (`include_theme_field_values` for the staff logs).
pub async fn serialize(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    t: &Theme,
    include_field_values: bool,
) -> Result<Value, AppError> {
    let fields = fields(conn, t.id).await?;
    let mut out = basic(conn, cx, t).await?;
    out.insert("color_scheme_id".into(), json!(t.color_scheme_id));
    out.insert("dark_color_scheme_id".into(), json!(t.dark_color_scheme_id));
    out.insert("user_selectable".into(), json!(t.user_selectable));
    out.insert("auto_update".into(), json!(t.auto_update));
    out.insert("remote_theme_id".into(), json!(t.remote_theme_id));
    out.insert("settings".into(), settings_json(conn, t.id).await?);
    if !t.component {
        out.insert(
            "themeable_site_settings".into(),
            themeable_site_settings(conn, cx, t.id).await?,
        );
    }
    let errors: Vec<&str> = fields.iter().filter_map(|f| f.error.as_deref()).collect();
    if !errors.is_empty() {
        out.insert("errors".into(), json!(errors));
    }
    out.insert("supported".into(), json!(supported_theme(conn, t).await?));
    out.insert("enabled".into(), json!(t.enabled));
    let disabled = if t.component && !t.enabled {
        disabled_log(conn, t).await?
    } else {
        None
    };
    if t.component && !t.enabled {
        out.insert(
            "disabled_at".into(),
            json!(disabled.map(|(at, _)| json_time(at))),
        );
    }
    let migrated: Vec<i32> = sqlx::query_scalar(
        "SELECT theme_field_id FROM theme_settings_migrations WHERE theme_id = $1",
    )
    .bind(t.id)
    .fetch_all(&mut *conn)
    .await?;
    let include_values = include_field_values || t.remote_theme_id.is_none();
    let mut field_values = Vec::with_capacity(fields.len());
    for f in &fields {
        field_values.push(field_json(conn, f, include_values, migrated.contains(&f.id)).await?);
    }
    out.insert("theme_fields".into(), Value::Array(field_values));
    out.insert(
        "screenshot_dark_url".into(),
        json!(screenshot_url(conn, &fields, "screenshot_dark").await?),
    );
    out.insert(
        "screenshot_light_url".into(),
        json!(screenshot_url(conn, &fields, "screenshot_light").await?),
    );
    out.insert("system".into(), json!(t.system()));
    let only: Option<Option<bool>> = sqlx::query_scalar(
        "SELECT only_theme_color_schemes FROM theme_modifier_sets WHERE theme_id = $1",
    )
    .bind(t.id)
    .fetch_optional(&mut *conn)
    .await?;
    out.insert(
        "only_theme_color_schemes".into(),
        json!(only.flatten().unwrap_or(false)),
    );
    out.insert(
        "color_scheme".into(),
        color_scheme(conn, t.color_scheme_id).await?,
    );
    out.insert("user".into(), user_name(conn, cx, Some(t.user_id)).await?);
    if t.component && !t.enabled {
        let by = disabled.and_then(|(_, by)| by);
        out.insert("disabled_by".into(), user_name(conn, cx, by).await?);
    }
    if !t.component {
        out.insert("child_themes".into(), related(conn, cx, t.id, true).await?);
    }
    out.insert(
        "parent_themes".into(),
        related(conn, cx, t.id, false).await?,
    );
    out.insert("remote_theme".into(), remote_theme(conn, t).await?);
    let translations: Vec<Value> = crate::site_texts::theme_translations(conn, t.id)
        .await?
        .into_iter()
        .map(|e| json!({"key": e.key, "value": e.value(), "default": e.default}))
        .collect();
    out.insert("translations".into(), Value::Array(translations));
    Ok(Value::Object(out))
}

/// ThemeIndexSerializer
pub async fn index_json(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    t: &Theme,
) -> Result<Value, AppError> {
    let fields = fields(conn, t.id).await?;
    let mut out = basic(conn, cx, t).await?;
    out.insert("user_selectable".into(), json!(t.user_selectable));
    out.insert(
        "screenshot_dark_url".into(),
        json!(screenshot_url(conn, &fields, "screenshot_dark").await?),
    );
    out.insert(
        "screenshot_light_url".into(),
        json!(screenshot_url(conn, &fields, "screenshot_light").await?),
    );
    out.insert("remote_theme_id".into(), json!(t.remote_theme_id));
    out.insert("enabled".into(), json!(t.enabled));
    out.insert("supported".into(), json!(supported_theme(conn, t).await?));
    out.insert("system".into(), json!(t.system()));
    out.insert(
        "color_scheme".into(),
        color_scheme(conn, t.color_scheme_id).await?,
    );
    out.insert("remote_theme".into(), remote_theme(conn, t).await?);
    Ok(Value::Object(out))
}

/// ComponentIndexSerializer
pub async fn component_json(
    conn: &mut PgConnection,
    cx: &Ctx<'_>,
    t: &Theme,
) -> Result<Value, AppError> {
    let mut out = basic(conn, cx, t).await?;
    out.insert("remote_theme_id".into(), json!(t.remote_theme_id));
    out.insert("supported".into(), json!(supported_theme(conn, t).await?));
    out.insert("enabled".into(), json!(t.enabled));
    let disabled = if !t.enabled {
        disabled_log(conn, t).await?
    } else {
        None
    };
    if !t.enabled {
        out.insert(
            "disabled_at".into(),
            json!(disabled.map(|(at, _)| json_time(at))),
        );
    }
    out.insert("user".into(), user_name(conn, cx, Some(t.user_id)).await?);
    if !t.enabled {
        let by = disabled.and_then(|(_, by)| by);
        out.insert("disabled_by".into(), user_name(conn, cx, by).await?);
    }
    out.insert(
        "parent_themes".into(),
        related(conn, cx, t.id, false).await?,
    );
    out.insert("remote_theme".into(), remote_theme(conn, t).await?);
    Ok(Value::Object(out))
}
