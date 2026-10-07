//! Port of Discourse's SiteSetting: lib/site_setting_extension.rb,
//! lib/site_settings/{yaml_loader,type_supervisor,defaults_provider,db_provider}.rb.
//!
//! Definitions and defaults come from the vendored config/site_settings.yml.
//! A value resolves, lowest to highest precedence, from:
//!   1. the YAML default (or its `locale_default` for the site's locale)
//!   2. a `site_settings` row
//!   3. a `DISCOURSE_<NAME>` global setting, which shadows the setting
//!
//! Upcoming-change settings (`upcoming_change:` in the YAML) resolve through a
//! port of UpcomingChanges.enabled?: promoted by status unless an admin stored
//! a choice, gated on `depends_on`.
//!
//! The bundled plugins' settings (vendored config/plugin_settings.yml) are
//! declared after core's, as Rails loads them, each owned by its plugin.
//!
//! Not yet ported: themeable settings.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Mutex;

use serde::Serialize;
use serde::ser::Serializer;
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::config::GlobalSettings;
use crate::ruby;

const SITE_SETTINGS_YML: &str = include_str!("../vendor/discourse/config/site_settings.yml");
/// One YAML document per bundled plugin (scripts/vendor-discourse).
const PLUGIN_SETTINGS_YML: &str = include_str!("../vendor/discourse/config/plugin_settings.yml");

/// `DefaultsProvider::DEFAULT_LOCALE`.
const DEFAULT_LOCALE: &str = "en";

/// `SiteSettings::TypeSupervisor.types`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataType {
    String = 1,
    Time = 2,
    Integer = 3,
    Float = 4,
    Bool = 5,
    Null = 6,
    Enum = 7,
    List = 8,
    UrlList = 9,
    HostList = 10,
    CategoryList = 11,
    ValueList = 12,
    Regex = 13,
    Email = 14,
    Username = 15,
    Category = 16,
    UploadedImageList = 17,
    Upload = 18,
    Group = 19,
    GroupList = 20,
    TagList = 21,
    Color = 22,
    SimpleList = 23,
    EmojiList = 24,
    HtmlDeprecated = 25,
    TagGroupList = 26,
    FileSizeRestriction = 27,
    Objects = 28,
    LocaleEnum = 29,
    Topic = 30,
    Datetime = 31,
    Icon = 32,
    Date = 33,
}

impl DataType {
    const ALL: [(DataType, &'static str); 33] = [
        (DataType::String, "string"),
        (DataType::Time, "time"),
        (DataType::Integer, "integer"),
        (DataType::Float, "float"),
        (DataType::Bool, "bool"),
        (DataType::Null, "null"),
        (DataType::Enum, "enum"),
        (DataType::List, "list"),
        (DataType::UrlList, "url_list"),
        (DataType::HostList, "host_list"),
        (DataType::CategoryList, "category_list"),
        (DataType::ValueList, "value_list"),
        (DataType::Regex, "regex"),
        (DataType::Email, "email"),
        (DataType::Username, "username"),
        (DataType::Category, "category"),
        (DataType::UploadedImageList, "uploaded_image_list"),
        (DataType::Upload, "upload"),
        (DataType::Group, "group"),
        (DataType::GroupList, "group_list"),
        (DataType::TagList, "tag_list"),
        (DataType::Color, "color"),
        (DataType::SimpleList, "simple_list"),
        (DataType::EmojiList, "emoji_list"),
        (DataType::HtmlDeprecated, "html_deprecated"),
        (DataType::TagGroupList, "tag_group_list"),
        (DataType::FileSizeRestriction, "file_size_restriction"),
        (DataType::Objects, "objects"),
        (DataType::LocaleEnum, "locale_enum"),
        (DataType::Topic, "topic"),
        (DataType::Datetime, "datetime"),
        (DataType::Icon, "icon"),
        (DataType::Date, "date"),
    ];

    pub fn from_id(id: i32) -> Option<DataType> {
        Self::ALL
            .iter()
            .find(|(t, _)| *t as i32 == id)
            .map(|(t, _)| *t)
    }

    pub(crate) fn from_name(name: &str) -> Option<DataType> {
        Self::ALL.iter().find(|(_, n)| *n == name).map(|(t, _)| *t)
    }
}

/// A setting value with Ruby's dynamic typing: what `SiteSetting.<name>`
/// returns. Serializes like the Ruby value would in `render json:`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A YAML sequence default (`default: []` on an objects setting).
    List(Vec<Value>),
}

impl Value {
    /// Ruby truthiness: everything but nil and false.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Null | Value::Bool(false))
    }

    /// `value.to_i`, as used on upload ids and `port`.
    pub fn to_i(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            Value::Str(s) => ruby::to_i(s),
            Value::Float(f) => *f as i64,
            Value::Null | Value::Bool(_) | Value::List(_) => 0,
        }
    }

    /// `value.to_f`, as used on the search ranking weights.
    pub fn to_f(&self) -> f64 {
        match self {
            Value::Float(f) => *f,
            Value::Int(i) => *i as f64,
            Value::Str(s) => ruby::to_f(s),
            Value::Null | Value::Bool(_) | Value::List(_) => 0.0,
        }
    }

    /// `value.presence` for string-ish settings: None when nil or blank.
    pub fn presence(&self) -> Option<String> {
        match self {
            Value::Null => None,
            Value::Str(s) if ruby::is_blank(s) => None,
            Value::Str(s) => Some(s.clone()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Int(i) => Some(i.to_string()),
            Value::Float(f) => Some(f.to_string()),
            Value::List(l) if l.is_empty() => None,
            Value::List(_) => Some(self.to_s()),
        }
    }

    /// Ruby `to_s`.
    pub fn to_s(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) if f.fract() == 0.0 && f.is_finite() => format!("{f:.1}"),
            Value::Float(f) => f.to_string(),
            Value::Str(s) => s.clone(),
            // Array#to_s (inspect) of the elements' own to_s; only ever
            // empty in the vendored files.
            Value::List(l) => format!(
                "[{}]",
                l.iter().map(Value::to_s).collect::<Vec<_>>().join(", ")
            ),
        }
    }

    pub fn is_blank(&self) -> bool {
        match self {
            Value::Null | Value::Bool(false) => true,
            Value::Str(s) => ruby::is_blank(s),
            Value::List(l) => l.is_empty(),
            _ => false,
        }
    }

    fn type_of(&self) -> DataType {
        match self {
            Value::Null => DataType::Null,
            Value::Bool(_) => DataType::Bool,
            Value::Int(_) => DataType::Integer,
            Value::Float(_) => DataType::Float,
            Value::Str(_) => DataType::String,
            Value::List(_) => DataType::List,
        }
    }

    /// `GlobalSetting` provider coercion (BaseProvider#coerce): "true" and
    /// "false" become booleans, integers become Integer, anything else stays
    /// a string. Not the TypeSupervisor conversion.
    fn from_global(raw: &str) -> Value {
        match raw {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ if is_ruby_integer(raw) => raw
                .parse()
                .map(Value::Int)
                .unwrap_or(Value::Str(raw.into())),
            _ => Value::Str(raw.into()),
        }
    }
}

/// `/\A(-?[0-9]+)\z/`
fn is_ruby_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => s.serialize_none(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(i) => s.serialize_i64(*i),
            Value::Float(f) => s.serialize_f64(*f),
            Value::Str(v) => s.serialize_str(v),
            Value::List(l) => s.collect_seq(l),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Definition {
    pub name: String,
    pub category: String,
    pub default: Value,
    pub data_type: DataType,
    pub client: bool,
    /// `themeable: true`: themes override it, so it is served per theme
    /// and left out of the client settings.
    pub themeable: bool,
    /// The plugin whose settings.yml declares it; None for core's.
    pub plugin: Option<String>,
    pub(crate) locale_defaults: HashMap<String, Value>,
    /// `upcoming_change: {status: ...}`; the setting's value is then decided
    /// by UpcomingChanges.enabled? rather than read directly.
    pub(crate) upcoming_change: Option<ChangeStatus>,
    /// `upcoming_change.body_class`: the change adds a body CSS class, so the
    /// client is told about it (upcoming_changes_with_css).
    change_body_class: bool,
    pub(crate) depends_on: Vec<String>,
    /// `depends_on_values`: per dependency, the values that count as met.
    pub(crate) depends_on_values: HashMap<String, Vec<String>>,
    /// `mandatory_values`: entries a list setting always carries.
    pub(crate) mandatory_values: Option<String>,
    /// `upcoming_change_default_override`: the upcoming change and the
    /// default this setting takes while that change is enabled.
    pub(crate) default_override: Option<(String, Value)>,
    /// An upcoming change of a plugin only takes effect while the plugin
    /// is enabled, unless it opts out (`requires_plugin_enabled: false`).
    change_requires_plugin: bool,
    /// The setting's options as declared (min, max, hidden, choices, regex,
    /// validator...), for validating a new value.
    pub options: Option<serde_yaml_ng::Mapping>,
}

impl Definition {
    /// Declared as an upcoming change (its value comes from UpcomingChanges).
    pub fn is_upcoming_change(&self) -> bool {
        self.upcoming_change.is_some()
    }
}

/// `UpcomingChanges.statuses`, ordered by their numeric rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeStatus {
    Conceptual = -100,
    Experimental = 0,
    Alpha = 100,
    Beta = 200,
    Stable = 300,
    Permanent = 500,
    Never = 9999,
}

impl ChangeStatus {
    fn parse(s: &str) -> Option<ChangeStatus> {
        Some(match s {
            "conceptual" => ChangeStatus::Conceptual,
            "experimental" => ChangeStatus::Experimental,
            "alpha" => ChangeStatus::Alpha,
            "beta" => ChangeStatus::Beta,
            "stable" => ChangeStatus::Stable,
            "permanent" => ChangeStatus::Permanent,
            "never" => ChangeStatus::Never,
            _ => return None,
        })
    }
}

/// Every setting declared in site_settings.yml, in file order.
#[derive(Debug)]
pub struct Definitions {
    list: Vec<Definition>,
    index: HashMap<String, usize>,
    /// Per plugin, the setting its plugin.rb names with
    /// `enabled_site_setting` (None: always enabled).
    plugin_enabled_settings: HashMap<String, Option<String>>,
    /// The last resolved settings with the table fingerprint they came
    /// from (`max(updated_at)`, row count): what Rails keeps in its
    /// process cache and invalidates over MessageBus.
    cache: Mutex<Option<(String, SiteSettings)>>,
}

impl Definitions {
    /// Core's settings, then each bundled plugin's, then those plugins
    /// register from Ruby (recorded, crate::setting_enums).
    pub fn vendored() -> Result<Self, String> {
        let mut defs = Self::parse(SITE_SETTINGS_YML)?;
        defs.add_plugins(PLUGIN_SETTINGS_YML)?;
        defs.add_dynamic(crate::setting_enums::dynamic_settings())?;
        Ok(defs)
    }

    /// Settings a plugin registers with `SiteSetting.setting` from Ruby,
    /// as recorded: each is declared as its options would be in YAML.
    pub fn add_dynamic(&mut self, settings: &[serde_json::Value]) -> Result<(), String> {
        for s in settings {
            let name = s["name"]
                .as_str()
                .ok_or("a dynamic setting without a name")?;
            let category = s["category"].as_str().unwrap_or("uncategorized");
            let mut entry = serde_json::Map::new();
            for key in [
                "default",
                "type",
                "enum",
                "area",
                "depends_on",
                "depends_behavior",
            ] {
                if !s[key].is_null() {
                    entry.insert(key.into(), s[key].clone());
                }
            }
            let entry: Yaml = serde_yaml_ng::to_value(serde_json::Value::Object(entry))
                .map_err(|e| format!("dynamic setting {name}: {e}"))?;
            let mut def = parse_definition(category, name, &entry)
                .map_err(|e| format!("dynamic setting {name}: {e}"))?;
            def.plugin = s["plugin"].as_str().map(str::to_string);
            if self.index.contains_key(name) {
                return Err(format!("dynamic setting {name} is declared in YAML too"));
            }
            self.index.insert(name.to_string(), self.list.len());
            self.list.push(def);
        }
        Ok(())
    }

    pub fn parse(src: &str) -> Result<Self, String> {
        let root: Yaml =
            serde_yaml_ng::from_str(src).map_err(|e| format!("site_settings.yml: {e}"))?;
        let mut defs = Definitions {
            list: Vec::new(),
            index: HashMap::new(),
            plugin_enabled_settings: HashMap::new(),
            cache: Mutex::new(None),
        };
        defs.add_categories(&root, None)?;
        Ok(defs)
    }

    /// `load_settings(file, plugin:)` for every document of the vendored
    /// plugin settings: `{plugin, enabled_site_setting, settings}`.
    pub fn add_plugins(&mut self, src: &str) -> Result<(), String> {
        for document in serde_yaml_ng::Deserializer::from_str(src) {
            let doc: Yaml = serde::Deserialize::deserialize(document)
                .map_err(|e| format!("plugin_settings.yml: {e}"))?;
            let field = |key: &str| doc.get(Yaml::String(key.into()));
            let plugin = field("plugin")
                .and_then(Yaml::as_str)
                .ok_or("plugin_settings.yml: a document without a plugin name")?
                .to_string();
            let enabled = field("enabled_site_setting")
                .and_then(Yaml::as_str)
                .map(str::to_string);
            // A plugin's settings.yml may be empty.
            if let Some(settings) = field("settings").filter(|s| !s.is_null()) {
                self.add_categories(settings, Some(&plugin))
                    .map_err(|e| format!("plugin {plugin}: {e}"))?;
            }
            self.plugin_enabled_settings.insert(plugin, enabled);
        }
        Ok(())
    }

    fn add_categories(&mut self, root: &Yaml, plugin: Option<&str>) -> Result<(), String> {
        let categories = root
            .as_mapping()
            .ok_or("top level must be a mapping of categories")?;
        for (category, settings) in categories {
            let category = yaml_key(category)?;
            let Some(settings) = settings.as_mapping() else {
                return Err(format!(
                    "category {category}: expected a mapping of settings"
                ));
            };
            for (name, entry) in settings {
                let name = yaml_key(name)?;
                if name == "default_locale" {
                    return Err("default_locale cannot be declared in YAML".into());
                }
                let mut def = parse_definition(&category, &name, entry)
                    .map_err(|e| format!("setting {name}: {e}"))?;
                def.plugin = plugin.map(str::to_string);
                match self.index.get(&name) {
                    // `setting()` called again redefines: a plugin may
                    // take over a core setting (narrative-bot does).
                    Some(&i) if plugin.is_some() => self.list[i] = def,
                    Some(_) => return Err(format!("setting {name} declared twice")),
                    None => {
                        self.index.insert(name, self.list.len());
                        self.list.push(def);
                    }
                }
            }
        }
        Ok(())
    }

    /// `owning_plugin_enabled?`: true for core's settings and for plugins
    /// without an `enabled_site_setting`.
    fn owning_plugin_enabled(&self, def: &Definition, values: &HashMap<String, Value>) -> bool {
        let Some(plugin) = &def.plugin else {
            return true;
        };
        match self.plugin_enabled_settings.get(plugin) {
            Some(Some(setting)) => values.get(setting).is_some_and(Value::truthy),
            _ => true,
        }
    }

    /// `Plugin::Instance#enabled?` for a bundled plugin: its
    /// `enabled_site_setting` is on, or it names none.
    pub fn plugin_enabled(&self, plugin: &str, settings: &SiteSettings) -> bool {
        match self.plugin_enabled_settings.get(plugin) {
            Some(Some(setting)) => settings.get(setting).is_ok_and(Value::truthy),
            _ => true,
        }
    }

    pub fn get(&self, name: &str) -> Option<&Definition> {
        self.index.get(name).map(|&i| &self.list[i])
    }

    /// `UpcomingChanges.including_css`: upcoming changes whose metadata has
    /// `body_class: true`, enabled or not, sorted by name.
    pub fn upcoming_changes_with_css(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .list
            .iter()
            .filter(|d| d.upcoming_change.is_some() && d.change_body_class)
            .map(|d| d.name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    /// `UpcomingChanges.permanent_upcoming_change_names`: the settings
    /// whose upcoming change is permanent, by name.
    pub fn permanent_upcoming_change_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .list
            .iter()
            .filter(|d| d.upcoming_change == Some(ChangeStatus::Permanent))
            .map(|d| d.name.as_str())
            .collect();
        names.sort_unstable();
        names
    }

    pub fn iter(&self) -> impl Iterator<Item = &Definition> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

fn yaml_key(v: &Yaml) -> Result<String, String> {
    v.as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("expected a string key, got {v:?}"))
}

/// Psych (YAML 1.1) reads unquoted `150_000` as an Integer; YAML 1.2 parsers
/// read a string. serde_yaml_ng can't tell quoted from unquoted, so this
/// only applies to strings of digits with underscores, which the vendored
/// file never quotes (checked by a test).
fn yaml_scalar(v: &Yaml) -> Result<Value, String> {
    Ok(match v {
        Yaml::Null => Value::Null,
        Yaml::Bool(b) => Value::Bool(*b),
        Yaml::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().ok_or("number out of range")?),
        },
        Yaml::String(s) if is_underscored_integer(s) => Value::Int(ruby::to_i(s)),
        Yaml::String(s) => Value::Str(s.clone()),
        Yaml::Sequence(items) => {
            Value::List(items.iter().map(yaml_scalar).collect::<Result<_, _>>()?)
        }
        other => return Err(format!("expected a scalar, got {other:?}")),
    })
}

fn is_underscored_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    digits.contains('_')
        && digits.starts_with(|c: char| c.is_ascii_digit())
        && digits.ends_with(|c: char| c.is_ascii_digit())
        && digits.bytes().all(|b| b.is_ascii_digit() || b == b'_')
        && !digits.contains("__")
}

fn parse_definition(category: &str, name: &str, entry: &Yaml) -> Result<Definition, String> {
    let Some(opts) = entry.as_mapping() else {
        let default = yaml_scalar(entry)?;
        return Ok(Definition {
            name: name.into(),
            category: category.into(),
            data_type: default.type_of(),
            default,
            client: false,
            themeable: false,
            plugin: None,
            locale_defaults: HashMap::new(),
            upcoming_change: None,
            change_body_class: false,
            depends_on: Vec::new(),
            depends_on_values: HashMap::new(),
            mandatory_values: None,
            default_override: None,
            change_requires_plugin: true,
            options: None,
        });
    };

    let opt = |key: &str| opts.get(Yaml::String(key.into()));

    // yaml_loader.rb raises on a nil default.
    let default = match opt("default") {
        None | Some(Yaml::Null) => return Err("missing default".into()),
        Some(v) => yaml_scalar(v)?,
    };

    // type_supervisor.rb: `enum:` implies `type: enum`; a declared type only
    // counts if it's a real type (e.g. not a validator-only name), otherwise
    // the type is inferred from the default.
    let declared = opt("type")
        .and_then(Yaml::as_str)
        .or_else(|| opt("enum").map(|_| "enum"));
    let data_type = declared
        .and_then(DataType::from_name)
        .unwrap_or_else(|| default.type_of());

    let client = matches!(opt("client"), Some(Yaml::Bool(true)));

    let mut locale_defaults = HashMap::new();
    if let Some(Yaml::Mapping(m)) = opt("locale_default") {
        for (locale, value) in m {
            locale_defaults.insert(yaml_key(locale)?, yaml_scalar(value)?);
        }
    }

    let change_body_class = matches!(
        opt("upcoming_change").and_then(|c| c.get(Yaml::String("body_class".into()))),
        Some(Yaml::Bool(true))
    );
    let upcoming_change = match opt("upcoming_change") {
        None => None,
        Some(Yaml::Mapping(m)) => {
            let status = m
                .get(Yaml::String("status".into()))
                .and_then(Yaml::as_str)
                .ok_or("upcoming_change needs a status")?;
            Some(
                ChangeStatus::parse(status)
                    .ok_or(format!("unknown upcoming_change status {status}"))?,
            )
        }
        Some(other) => return Err(format!("upcoming_change must be a mapping, got {other:?}")),
    };

    let depends_on = match opt("depends_on") {
        None => Vec::new(),
        Some(Yaml::Sequence(deps)) => deps.iter().map(yaml_key).collect::<Result<_, _>>()?,
        Some(Yaml::String(dep)) => vec![dep.clone()],
        Some(other) => return Err(format!("depends_on must be a list, got {other:?}")),
    };

    let default_override = match opt("upcoming_change_default_override") {
        None => None,
        Some(o) => {
            let field = |key: &str| o.get(Yaml::String(key.into()));
            let change = field("upcoming_change")
                .and_then(Yaml::as_str)
                .ok_or("upcoming_change_default_override needs an upcoming_change")?;
            let new_default = field("new_default")
                .ok_or("upcoming_change_default_override needs a new_default")?;
            Some((change.to_string(), yaml_scalar(new_default)?))
        }
    };

    let mut depends_on_values = HashMap::new();
    if let Some(Yaml::Mapping(m)) = opt("depends_on_values") {
        for (dep, allowed) in m {
            let allowed = match allowed {
                Yaml::Sequence(vals) => vals
                    .iter()
                    .map(|v| yaml_scalar(v).map(|v| v.to_s()))
                    .collect::<Result<_, _>>()?,
                other => return Err(format!("depends_on_values must be lists, got {other:?}")),
            };
            depends_on_values.insert(yaml_key(dep)?, allowed);
        }
    }

    Ok(Definition {
        name: name.into(),
        category: category.into(),
        default,
        data_type,
        client,
        themeable: matches!(opt("themeable"), Some(Yaml::Bool(true))),
        plugin: None,
        locale_defaults,
        upcoming_change,
        change_body_class,
        depends_on,
        depends_on_values,
        mandatory_values: opt("mandatory_values")
            .and_then(Yaml::as_str)
            .map(str::to_string),
        default_override,
        change_requires_plugin: !matches!(
            opt("upcoming_change")
                .and_then(|c| c.get(Yaml::String("requires_plugin_enabled".into()))),
            Some(Yaml::Bool(false))
        ),
        options: Some(opts.clone()),
    })
}

#[derive(Debug)]
pub enum SettingError {
    Db(sqlx::Error),
    Unknown(String),
    /// A row whose data_type isn't a known type; Discourse raises here too.
    BadRow {
        name: String,
        data_type: i32,
    },
    /// A value that Discourse itself would choke on, e.g. an unknown
    /// promote_upcoming_changes_on_status.
    Invalid {
        name: String,
        value: String,
    },
    DependencyCycle(String),
    /// A setting whose client form isn't ported.
    Unsupported {
        name: String,
        what: &'static str,
    },
}

impl fmt::Display for SettingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingError::Db(e) => write!(f, "loading site settings: {e}"),
            SettingError::Unknown(name) => write!(f, "unknown site setting {name}"),
            SettingError::BadRow { name, data_type } => {
                write!(
                    f,
                    "site_settings row {name} has unknown data_type {data_type}"
                )
            }
            SettingError::Invalid { name, value } => {
                write!(f, "site setting {name} has invalid value {value:?}")
            }
            SettingError::DependencyCycle(name) => {
                write!(f, "upcoming change {name} has a dependency cycle")
            }
            SettingError::Unsupported { name, what } => {
                write!(f, "not ported yet: {what} (site setting {name})")
            }
        }
    }
}

impl std::error::Error for SettingError {}

impl From<sqlx::Error> for SettingError {
    fn from(e: sqlx::Error) -> Self {
        SettingError::Db(e)
    }
}

/// Resolved settings for one site, the equivalent of SiteSetting.current.
#[derive(Debug, Clone)]
pub struct SiteSettings {
    values: HashMap<String, Value>,
}

impl SiteSettings {
    /// SiteSettingExtension#refresh!: defaults for the site's locale, then
    /// DB rows, then global-setting shadows.
    pub fn resolve(
        defs: &Definitions,
        rows: Vec<(String, i32, Option<String>)>,
        globals: &GlobalSettings,
    ) -> Result<Self, SettingError> {
        let mut db = HashMap::with_capacity(rows.len());
        for (name, data_type, value) in rows {
            let value = to_rb_value(defs, &name, data_type, value.as_deref())?;
            db.insert(name, value);
        }

        // Defaults use the locale stored in the DB, even if default_locale
        // itself is shadowed by a global setting.
        let db_locale = match db.get("default_locale") {
            Some(Value::Str(l)) => l.clone(),
            _ => DEFAULT_LOCALE.to_string(),
        };

        let mut values: HashMap<String, Value> = defs
            .iter()
            .map(|d| {
                // DefaultsProvider: a locale default (`en` too) overrides the
                // plain one for that locale.
                let default = d.locale_defaults.get(&db_locale);
                (d.name.clone(), default.unwrap_or(&d.default).clone())
            })
            .collect();
        let modified: HashSet<String> = db.keys().cloned().collect();
        values.extend(db);

        // Upcoming-change settings resolve through UpcomingChanges.enabled?
        // rather than their stored value.
        let changes: Vec<&Definition> = defs
            .iter()
            .filter(|d| d.upcoming_change.is_some())
            .collect();
        for def in &changes {
            let enabled = upcoming_change_enabled(defs, &values, &modified, &def.name, 0)?;
            values.insert(def.name.clone(), Value::Bool(enabled));
        }

        // upcoming_change_default_override: while the change is enabled
        // and nothing is stored for the setting, its default is the
        // override's.
        for def in defs.iter() {
            if let Some((change, new_default)) = &def.default_override
                && !modified.contains(&def.name)
                && values.get(change).is_some_and(Value::truthy)
            {
                values.insert(def.name.clone(), new_default.clone());
            }
        }

        // site_setting_extension.rb `setting()`: a setting is shadowed when
        // GlobalSetting responds to its name with a present value.
        for key in globals.keys() {
            if key != "default_locale" && defs.get(key).is_none() {
                continue;
            }
            if let Some(raw) = globals.get(key) {
                values.insert(key.to_string(), Value::from_global(raw));
            }
        }

        // The getter merges `mandatory_values` in front of a list value.
        for def in defs.iter() {
            if let Some(mandatory) = &def.mandatory_values {
                let current = values.get(&def.name).map(Value::to_s).unwrap_or_default();
                let mut merged: Vec<&str> = mandatory.split('|').collect();
                for item in current.split('|').filter(|s| !s.is_empty()) {
                    if !merged.contains(&item) {
                        merged.push(item);
                    }
                }
                values.insert(def.name.clone(), Value::Str(merged.join("|")));
            }
        }

        values
            .entry("default_locale".into())
            .or_insert_with(|| Value::Str(DEFAULT_LOCALE.into()));

        Ok(SiteSettings { values })
    }

    /// The settings as of now: resolved again only when a row of
    /// `site_settings` changed since the last load, else a clone of the
    /// cached resolution (one fingerprint query instead of the table and
    /// the resolve).
    pub async fn load(
        conn: &mut PgConnection,
        defs: &Definitions,
        globals: &GlobalSettings,
    ) -> Result<Self, SettingError> {
        let fingerprint: String = sqlx::query_scalar(
            "SELECT COALESCE(max(updated_at)::text, '') || '/' || count(*)::text FROM site_settings",
        )
        .fetch_one(&mut *conn)
        .await?;
        {
            let cache = defs.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((cached, settings)) = cache.as_ref()
                && *cached == fingerprint
            {
                return Ok(settings.clone());
            }
        }
        let rows: Vec<(String, i32, Option<String>)> =
            sqlx::query_as("SELECT name, data_type, value FROM site_settings")
                .fetch_all(conn)
                .await?;
        let settings = Self::resolve(defs, rows, globals)?;
        let mut cache = defs.cache.lock().unwrap_or_else(|e| e.into_inner());
        *cache = Some((fingerprint, settings.clone()));
        Ok(settings)
    }

    /// `SiteSetting.<name>_map` for group_list settings: `split("|")` then
    /// `to_i`, so a malformed entry becomes 0 (everyone). Empty when blank.
    /// with its `mandatory_values` already merged in.
    pub fn group_ids(&self, name: &str) -> Result<Vec<i64>, SettingError> {
        let raw = self.get(name)?.to_s();
        Ok(raw
            .split('|')
            .filter(|s| !s.is_empty())
            .map(ruby::to_i)
            .collect())
    }

    /// `SiteSetting.client_settings_hash`: `default_locale` and every
    /// setting declared `client: true` (themeable ones aside), by name.
    /// Upload settings become the upload's URL.
    pub async fn client_settings_hash(
        &self,
        conn: &mut PgConnection,
        defs: &Definitions,
    ) -> Result<serde_json::Map<String, serde_json::Value>, SettingError> {
        let mut out = serde_json::Map::new();
        out.insert(
            "default_locale".into(),
            serde_json::json!(self.get("default_locale")?),
        );
        for def in defs.iter().filter(|d| d.client && !d.themeable) {
            let value = self.get(&def.name)?;
            let json = match def.data_type {
                // Upload#to_s is its url; nil.to_s the empty string.
                DataType::Upload => {
                    let url: Option<String> =
                        sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
                            .bind(value.to_i() as i32)
                            .fetch_optional(&mut *conn)
                            .await?;
                    serde_json::json!(url.unwrap_or_default())
                }
                // A blank list is served as the empty array it is in Ruby.
                DataType::UploadedImageList if value.is_blank() => serde_json::json!([]),
                DataType::UploadedImageList => {
                    return Err(SettingError::Unsupported {
                        name: def.name.clone(),
                        what: "uploaded_image_list settings on the client",
                    });
                }
                DataType::Objects if !value.is_blank() => {
                    return Err(SettingError::Unsupported {
                        name: def.name.clone(),
                        what: "objects settings on the client",
                    });
                }
                _ => serde_json::json!(value),
            };
            out.insert(def.name.clone(), json);
        }
        Ok(out)
    }

    pub fn get(&self, name: &str) -> Result<&Value, SettingError> {
        self.values
            .get(name)
            .ok_or_else(|| SettingError::Unknown(name.into()))
    }
}

/// `UpcomingChanges.enabled?` (lib/upcoming_changes.rb). Plugin-owned
/// changes aren't ported (no plugin settings yet), so the plugin guards
/// don't apply.
fn upcoming_change_enabled(
    defs: &Definitions,
    values: &HashMap<String, Value>,
    modified: &HashSet<String>,
    name: &str,
    depth: usize,
) -> Result<bool, SettingError> {
    let def = defs
        .get(name)
        .ok_or_else(|| SettingError::Unknown(name.into()))?;
    let status = def
        .upcoming_change
        .ok_or_else(|| SettingError::Unknown(name.into()))?;
    if depth > 16 {
        return Err(SettingError::DependencyCycle(name.into()));
    }
    // A change of a disabled plugin does not take effect, whatever its
    // status or the stored choice.
    if def.change_requires_plugin && !defs.owning_plugin_enabled(def, values) {
        return Ok(false);
    }

    // change_dependencies_met?: each dependency, itself possibly an upcoming
    // change, must be true (or one of the depends_on_values).
    for dep in &def.depends_on {
        let value = match defs.get(dep).and_then(|d| d.upcoming_change) {
            Some(_) => Value::Bool(upcoming_change_enabled(
                defs,
                values,
                modified,
                dep,
                depth + 1,
            )?),
            None => values.get(dep).cloned().unwrap_or(Value::Null),
        };
        let met = match def.depends_on_values.get(dep) {
            Some(allowed) => allowed.contains(&value.to_s()),
            None => value == Value::Bool(true),
        };
        if !met {
            return Ok(false);
        }
    }

    let promote_on = values
        .get("promote_upcoming_changes_on_status")
        .map(Value::to_s)
        .unwrap_or_default();
    let promote_on = ChangeStatus::parse(&promote_on).ok_or_else(|| SettingError::Invalid {
        name: "promote_upcoming_changes_on_status".into(),
        value: promote_on.clone(),
    })?;

    Ok(
        if modified.contains(name) && status != ChangeStatus::Permanent {
            // An admin's stored choice wins, unless the change is permanent.
            values.get(name).is_some_and(Value::truthy)
        } else if status >= promote_on || status == ChangeStatus::Permanent {
            true
        } else {
            def.default.truthy()
        },
    )
}

/// TypeSupervisor#to_rb_value with the row's data_type as override.
pub(crate) fn to_rb_value(
    defs: &Definitions,
    name: &str,
    data_type: i32,
    value: Option<&str>,
) -> Result<Value, SettingError> {
    let bad_row = || SettingError::BadRow {
        name: name.into(),
        data_type,
    };
    let ty = DataType::from_id(data_type).ok_or_else(bad_row)?;
    let raw = value.unwrap_or("");

    Ok(match ty {
        DataType::Float => Value::Float(ruby::to_f(raw)),
        DataType::Integer | DataType::FileSizeRestriction => Value::Int(ruby::to_i(raw)),
        DataType::Bool => Value::Bool(raw == "t" || raw == "true"),
        DataType::Null => Value::Null,
        DataType::Enum => match defs.get(name).map(|d| &d.default) {
            Some(Value::Int(_)) => Value::Int(ruby::to_i(raw)),
            _ => Value::Str(raw.into()),
        },
        // Every other type returns the stored value untouched (nil stays nil).
        _ => match value {
            Some(v) => Value::Str(v.into()),
            None if matches!(
                ty,
                DataType::String | DataType::Datetime | DataType::Icon | DataType::Date
            ) =>
            {
                Value::Str(String::new())
            }
            None => Value::Null,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const YML: &str = r#"
basic:
  title:
    default: "Discourse"
    client: true
  site_description: ""
  min_post_length:
    default: 20
    locale_default:
      ja: 8
  trim_spaces:
    default: false
    locale_default:
      en: true
  max_size:
    default: 150_000
  quoted_digits:
    default: "12"
  ratio: 1.5
  login_required: false
  logo:
    default: -5
    type: upload
  tl_mode:
    default: 1
    enum: "SomeEnum"
  mode:
    default: "a"
    enum: "OtherEnum"
  weird_type:
    default: 3
    type: not_a_real_type
  port:
    default: ""
  promote_upcoming_changes_on_status:
    default: "beta"
    enum: "X"
  change_beta:
    default: false
    upcoming_change:
      status: "beta"
  change_alpha:
    default: false
    upcoming_change:
      status: "alpha"
  change_permanent:
    default: false
    upcoming_change:
      status: "permanent"
  change_gated:
    default: false
    depends_on:
      - login_required
    upcoming_change:
      status: "stable"
  change_enum_gated:
    default: false
    depends_on:
      - mode
    depends_on_values:
      mode:
        - b
    upcoming_change:
      status: "stable"
  change_chained:
    default: false
    depends_on:
      - change_beta
    upcoming_change:
      status: "stable"
"#;

    fn defs() -> Definitions {
        Definitions::parse(YML).unwrap()
    }

    fn resolve(rows: &[(&str, i32, Option<&str>)], globals: &[(&str, &str)]) -> SiteSettings {
        let rows = rows
            .iter()
            .map(|(n, t, v)| (n.to_string(), *t, v.map(str::to_string)))
            .collect();
        SiteSettings::resolve(
            &defs(),
            rows,
            &GlobalSettings::from_vars(globals.iter().copied()),
        )
        .unwrap()
    }

    #[test]
    fn infers_and_declares_types() {
        let d = defs();
        let ty = |n: &str| d.get(n).unwrap().data_type;
        assert_eq!(ty("title"), DataType::String);
        assert_eq!(ty("min_post_length"), DataType::Integer);
        assert_eq!(ty("ratio"), DataType::Float);
        assert_eq!(ty("login_required"), DataType::Bool);
        assert_eq!(ty("logo"), DataType::Upload);
        assert_eq!(ty("tl_mode"), DataType::Enum);
        assert_eq!(ty("weird_type"), DataType::Integer);
        assert!(d.get("title").unwrap().client);
        assert!(!d.get("ratio").unwrap().client);
    }

    #[test]
    fn underscored_integers_parse_like_psych() {
        let d = defs();
        assert_eq!(d.get("max_size").unwrap().default, Value::Int(150_000));
        assert_eq!(
            d.get("quoted_digits").unwrap().default,
            Value::Str("12".into())
        );
    }

    #[test]
    fn rejects_nil_default_and_default_locale() {
        assert!(Definitions::parse("a:\n  b:\n    client: true\n").is_err());
        assert!(Definitions::parse("a:\n  default_locale: en\n").is_err());
    }

    #[test]
    fn defaults_apply_without_rows() {
        let s = resolve(&[], &[]);
        assert_eq!(s.get("title").unwrap(), &Value::Str("Discourse".into()));
        assert_eq!(s.get("min_post_length").unwrap(), &Value::Int(20));
        assert_eq!(s.get("default_locale").unwrap(), &Value::Str("en".into()));
        assert!(s.get("nope").is_err());
    }

    #[test]
    fn locale_default_follows_the_db_locale() {
        let s = resolve(&[("default_locale", 1, Some("ja"))], &[]);
        assert_eq!(s.get("min_post_length").unwrap(), &Value::Int(8));
        assert_eq!(s.get("default_locale").unwrap(), &Value::Str("ja".into()));
    }

    #[test]
    fn an_en_locale_default_applies_to_english_sites() {
        // title_remove_extraneous_space: false, but true for en.
        let s = resolve(&[], &[]);
        assert_eq!(s.get("trim_spaces").unwrap(), &Value::Bool(true));
        let s = resolve(&[("default_locale", 1, Some("ja"))], &[]);
        assert_eq!(s.get("trim_spaces").unwrap(), &Value::Bool(false));
    }

    #[test]
    fn rows_are_typed_by_their_data_type() {
        let s = resolve(
            &[
                ("title", 1, Some("Hammer Time")),
                ("login_required", 5, Some("t")),
                ("min_post_length", 3, Some("15")),
                ("ratio", 4, Some("2.5")),
                ("logo", 18, Some("42")),
                ("tl_mode", 7, Some("3")),
                ("mode", 7, Some("b")),
                ("not_in_yaml", 1, Some("kept")),
            ],
            &[],
        );
        assert_eq!(s.get("title").unwrap(), &Value::Str("Hammer Time".into()));
        assert_eq!(s.get("login_required").unwrap(), &Value::Bool(true));
        assert_eq!(s.get("min_post_length").unwrap(), &Value::Int(15));
        assert_eq!(s.get("ratio").unwrap(), &Value::Float(2.5));
        assert_eq!(s.get("logo").unwrap(), &Value::Str("42".into()));
        assert_eq!(s.get("logo").unwrap().to_i(), 42);
        assert_eq!(s.get("tl_mode").unwrap(), &Value::Int(3));
        assert_eq!(s.get("mode").unwrap(), &Value::Str("b".into()));
        assert_eq!(s.get("not_in_yaml").unwrap(), &Value::Str("kept".into()));
    }

    #[test]
    fn bool_rows_accept_only_t_and_true() {
        for (raw, want) in [
            ("t", true),
            ("true", true),
            ("f", false),
            ("1", false),
            ("", false),
        ] {
            let s = resolve(&[("login_required", 5, Some(raw))], &[]);
            assert_eq!(
                s.get("login_required").unwrap(),
                &Value::Bool(want),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn unknown_row_type_is_an_error() {
        let rows = vec![("title".to_string(), 99, Some("x".to_string()))];
        let err = SiteSettings::resolve(&defs(), rows, &GlobalSettings::default()).unwrap_err();
        assert!(matches!(err, SettingError::BadRow { data_type: 99, .. }));
    }

    #[test]
    fn globals_shadow_rows_with_provider_coercion() {
        let s = resolve(
            &[("title", 1, Some("From DB"))],
            &[
                ("title", "From Env"),
                ("login_required", "true"),
                ("port", "8443"),
                ("site_description", "  "),
                ("not_a_setting", "ignored"),
                ("default_locale", "fr"),
            ],
        );
        assert_eq!(s.get("title").unwrap(), &Value::Str("From Env".into()));
        assert_eq!(s.get("login_required").unwrap(), &Value::Bool(true));
        assert_eq!(s.get("port").unwrap(), &Value::Int(8443));
        // Blank globals don't shadow.
        assert_eq!(s.get("site_description").unwrap(), &Value::Str("".into()));
        assert!(s.get("not_a_setting").is_err());
        assert_eq!(s.get("default_locale").unwrap(), &Value::Str("fr".into()));
    }

    #[test]
    fn value_serializes_like_ruby() {
        let json = serde_json::to_string(&[
            Value::Null,
            Value::Bool(true),
            Value::Int(-5),
            Value::Str("x".into()),
        ])
        .unwrap();
        assert_eq!(json, r#"[null,true,-5,"x"]"#);
    }

    #[test]
    fn upcoming_changes_promote_by_status() {
        let s = resolve(&[], &[]);
        assert_eq!(s.get("change_beta").unwrap(), &Value::Bool(true));
        assert_eq!(s.get("change_alpha").unwrap(), &Value::Bool(false));
        assert_eq!(s.get("change_permanent").unwrap(), &Value::Bool(true));

        // Raising the promotion bar demotes beta changes.
        let s = resolve(
            &[("promote_upcoming_changes_on_status", 7, Some("stable"))],
            &[],
        );
        assert_eq!(s.get("change_beta").unwrap(), &Value::Bool(false));
        assert_eq!(s.get("change_permanent").unwrap(), &Value::Bool(true));
    }

    #[test]
    fn admins_stored_choice_wins_unless_permanent() {
        let s = resolve(
            &[
                ("change_beta", 5, Some("f")),
                ("change_alpha", 5, Some("t")),
                ("change_permanent", 5, Some("f")),
            ],
            &[],
        );
        assert_eq!(s.get("change_beta").unwrap(), &Value::Bool(false));
        assert_eq!(s.get("change_alpha").unwrap(), &Value::Bool(true));
        assert_eq!(s.get("change_permanent").unwrap(), &Value::Bool(true));
    }

    #[test]
    fn upcoming_changes_are_gated_on_dependencies() {
        let s = resolve(&[], &[]);
        assert_eq!(s.get("change_gated").unwrap(), &Value::Bool(false));
        assert_eq!(s.get("change_enum_gated").unwrap(), &Value::Bool(false));
        assert_eq!(s.get("change_chained").unwrap(), &Value::Bool(true));

        let s = resolve(
            &[
                ("login_required", 5, Some("t")),
                ("mode", 7, Some("b")),
                ("change_beta", 5, Some("f")),
            ],
            &[],
        );
        assert_eq!(s.get("change_gated").unwrap(), &Value::Bool(true));
        assert_eq!(s.get("change_enum_gated").unwrap(), &Value::Bool(true));
        // The dependency is itself an upcoming change, resolved the same way.
        assert_eq!(s.get("change_chained").unwrap(), &Value::Bool(false));
    }

    #[test]
    fn unknown_promotion_status_is_an_error() {
        let rows = vec![(
            "promote_upcoming_changes_on_status".to_string(),
            7,
            Some("bogus".to_string()),
        )];
        let err = SiteSettings::resolve(&defs(), rows, &GlobalSettings::default()).unwrap_err();
        assert!(matches!(err, SettingError::Invalid { .. }), "{err}");
    }

    #[test]
    fn vendored_upcoming_changes_resolve_as_discourse_does() {
        let defs = Definitions::vendored().unwrap();
        let s = SiteSettings::resolve(&defs, vec![], &GlobalSettings::default()).unwrap();
        // beta status, promoted by the default "beta" bar
        assert_eq!(
            s.get("granular_anonymous_and_logged_in_groups_permissions")
                .unwrap(),
            &Value::Bool(true)
        );
        assert_eq!(s.get("enable_unified_new").unwrap(), &Value::Bool(true));
    }

    #[test]
    fn vendored_yaml_loads() {
        let d = Definitions::vendored().unwrap();
        assert!(d.len() > 1000, "only {} settings", d.len());
        let get = |n: &str| d.get(n).unwrap();
        assert_eq!(get("title").default, Value::Str("Discourse".into()));
        assert_eq!(get("logo").default, Value::Int(-5));
        assert_eq!(get("logo").data_type, DataType::Upload);
        assert_eq!(get("login_required").default, Value::Bool(false));
        assert_eq!(get("default_theme_id").default, Value::Int(-1));
        assert_eq!(get("port").default, Value::Str("".into()));
    }

    #[test]
    fn vendored_yaml_never_quotes_underscored_integers() {
        // Guards the Psych emulation in yaml_scalar.
        for line in SITE_SETTINGS_YML.lines() {
            let quoted = line.contains("\"") || line.contains('\'');
            let has = line
                .split(['"', '\''])
                .skip(1)
                .step_by(2)
                .any(is_underscored_integer);
            assert!(!(quoted && has), "quoted underscored integer: {line}");
        }
    }
}
