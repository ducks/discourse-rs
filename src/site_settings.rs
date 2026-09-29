//! Port of Discourse's SiteSetting: lib/site_setting_extension.rb,
//! lib/site_settings/{yaml_loader,type_supervisor,defaults_provider,db_provider}.rb.
//!
//! Definitions and defaults come from the vendored config/site_settings.yml.
//! A value resolves, lowest to highest precedence, from:
//!   1. the YAML default (or its `locale_default` for the site's locale)
//!   2. a `site_settings` row
//!   3. a `DISCOURSE_<NAME>` global setting, which shadows the setting
//!
//! Not yet ported: plugin settings files, upcoming-change default overrides,
//! `mandatory_values`, themeable settings.

use std::collections::HashMap;
use std::fmt;

use serde::Serialize;
use serde::ser::Serializer;
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::config::GlobalSettings;
use crate::ruby;

const SITE_SETTINGS_YML: &str = include_str!("../vendor/discourse/config/site_settings.yml");

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

    fn from_name(name: &str) -> Option<DataType> {
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
            Value::Null | Value::Bool(_) => 0,
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
        }
    }

    pub fn is_blank(&self) -> bool {
        match self {
            Value::Null | Value::Bool(false) => true,
            Value::Str(s) => ruby::is_blank(s),
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
    locale_defaults: HashMap<String, Value>,
}

/// Every setting declared in site_settings.yml, in file order.
#[derive(Debug)]
pub struct Definitions {
    list: Vec<Definition>,
    index: HashMap<String, usize>,
}

impl Definitions {
    pub fn vendored() -> Result<Self, String> {
        Self::parse(SITE_SETTINGS_YML)
    }

    pub fn parse(src: &str) -> Result<Self, String> {
        let root: Yaml =
            serde_yaml_ng::from_str(src).map_err(|e| format!("site_settings.yml: {e}"))?;
        let categories = root
            .as_mapping()
            .ok_or("site_settings.yml: top level must be a mapping of categories")?;

        let mut list = Vec::new();
        let mut index = HashMap::new();
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
                let def = parse_definition(&category, &name, entry)
                    .map_err(|e| format!("setting {name}: {e}"))?;
                if index.insert(name.clone(), list.len()).is_some() {
                    return Err(format!("setting {name} declared twice"));
                }
                list.push(def);
            }
        }
        Ok(Definitions { list, index })
    }

    pub fn get(&self, name: &str) -> Option<&Definition> {
        self.index.get(name).map(|&i| &self.list[i])
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
            locale_defaults: HashMap::new(),
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

    Ok(Definition {
        name: name.into(),
        category: category.into(),
        default,
        data_type,
        client,
        locale_defaults,
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
                let default = if db_locale == DEFAULT_LOCALE {
                    None
                } else {
                    d.locale_defaults.get(&db_locale)
                };
                (d.name.clone(), default.unwrap_or(&d.default).clone())
            })
            .collect();
        values.extend(db);

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

        values
            .entry("default_locale".into())
            .or_insert_with(|| Value::Str(DEFAULT_LOCALE.into()));

        Ok(SiteSettings { values })
    }

    pub async fn load(
        conn: &mut PgConnection,
        defs: &Definitions,
        globals: &GlobalSettings,
    ) -> Result<Self, SettingError> {
        let rows: Vec<(String, i32, Option<String>)> =
            sqlx::query_as("SELECT name, data_type, value FROM site_settings")
                .fetch_all(conn)
                .await?;
        Self::resolve(defs, rows, globals)
    }

    pub fn get(&self, name: &str) -> Result<&Value, SettingError> {
        self.values
            .get(name)
            .ok_or_else(|| SettingError::Unknown(name.into()))
    }
}

/// TypeSupervisor#to_rb_value with the row's data_type as override.
fn to_rb_value(
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
