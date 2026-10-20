//! `Category#save` (app/models/category.rb) for categories#create and
//! update: CategoriesController#category_params, the attributes cast as
//! ActiveRecord casts them, the validations in declaration order, the
//! before_save normalizations, the row, its associations (permissions,
//! settings, custom fields, tags, form templates, posting review groups)
//! and the callbacks: the definition topic on create
//! (`create_category_definition`, also what ensure_db_consistency calls),
//! the permalink bookkeeping, publish_category, the search index job,
//! the web hooks and the staff action log.
//!
//! A record is held as a JSON object of its columns, written through
//! `jsonb_populate_record`, so Postgres casts each value to its column.
//!
//! Refused: content localization (locale, category_localizations),
//! category group moderation (moderating_group_ids), uploads as logos or
//! backgrounds, email_in addresses, the Events category type
//! (crate::category_type_config) and the automations on
//! category_created_edited.

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// The column kinds ActiveRecord casts to.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Str,
    Bool,
    Int,
    Float,
}

/// The scalar attributes `category_params` permits, in its order, with
/// their column kinds (position, handled by the controller, first).
const PERMITTED: &[(&str, Kind)] = &[
    ("name", Kind::Str),
    ("position", Kind::Int),
    ("color", Kind::Str),
    ("text_color", Kind::Str),
    ("style_type", Kind::Int),
    ("emoji", Kind::Str),
    ("icon", Kind::Str),
    ("email_in", Kind::Str),
    ("email_in_allow_strangers", Kind::Bool),
    ("mailinglist_mirror", Kind::Bool),
    ("all_topics_wiki", Kind::Bool),
    ("allow_unlimited_owner_edits_on_first_post", Kind::Bool),
    ("default_slow_mode_seconds", Kind::Int),
    ("parent_category_id", Kind::Int),
    ("auto_close_hours", Kind::Float),
    ("auto_close_based_on_last_post", Kind::Bool),
    ("uploaded_logo_id", Kind::Int),
    ("uploaded_logo_dark_id", Kind::Int),
    ("uploaded_background_id", Kind::Int),
    ("uploaded_background_dark_id", Kind::Int),
    ("slug", Kind::Str),
    ("allow_badges", Kind::Bool),
    ("topic_template", Kind::Str),
    ("topic_title_placeholder", Kind::Str),
    ("description", Kind::Str),
    ("sort_order", Kind::Str),
    ("sort_ascending", Kind::Bool),
    ("topic_featured_link_allowed", Kind::Bool),
    ("show_subcategory_list", Kind::Bool),
    ("num_featured_topics", Kind::Int),
    ("default_view", Kind::Str),
    ("subcategory_list_style", Kind::Str),
    ("default_top_period", Kind::Str),
    ("minimum_required_tags", Kind::Int),
    ("navigate_to_first_post_after_read", Kind::Bool),
    ("search_priority", Kind::Int),
    ("allow_global_tags", Kind::Bool),
    ("read_only_banner", Kind::Str),
    ("default_list_filter", Kind::Str),
];

/// Columns that are NOT NULL without a default the record keeps: a nil
/// assigned to one fails the INSERT or UPDATE in Rails (a 500).
const NOT_NULL: &[&str] = &[
    "name",
    "color",
    "text_color",
    "style_type",
    "slug",
    "mailinglist_mirror",
    "all_topics_wiki",
    "allow_unlimited_owner_edits_on_first_post",
    "allow_badges",
    "minimum_required_tags",
    "navigate_to_first_post_after_read",
    "allow_global_tags",
];

/// The new record's column defaults (the schema's).
fn defaults() -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in [
        ("allow_global_tags", json!(false)),
        ("allow_unlimited_owner_edits_on_first_post", json!(false)),
        ("auto_close_based_on_last_post", json!(false)),
        ("color", json!("0088CC")),
        ("default_list_filter", json!("all")),
        ("default_top_period", json!("all")),
        ("email_in_allow_strangers", json!(false)),
        ("mailinglist_mirror", json!(false)),
        ("minimum_required_tags", json!(0)),
        ("navigate_to_first_post_after_read", json!(false)),
        ("num_featured_topics", json!(3)),
        ("read_restricted", json!(false)),
        ("search_priority", json!(0)),
        ("show_subcategory_list", json!(false)),
        ("style_type", json!(0)),
        ("subcategory_list_style", json!("rows_with_featured_topics")),
        ("text_color", json!("FFFFFF")),
        ("topic_count", json!(0)),
        ("topic_featured_link_allowed", json!(true)),
        ("allow_badges", json!(true)),
        ("all_topics_wiki", json!(false)),
    ] {
        m.insert(k.into(), v);
    }
    m
}

/// `TopTopic.periods`
const TOP_PERIODS: [&str; 6] = ["all", "yearly", "quarterly", "monthly", "weekly", "daily"];
/// `Searchable::PRIORITIES` values.
const SEARCH_PRIORITIES: [i64; 6] = [1, 2, 3, 0, 4, 5];
/// `Category::RESERVED_SLUGS`
const RESERVED_SLUGS: [&str; 1] = ["none"];
/// `CategoriesController::MAX_DESCRIPTION_PARAM_LENGTH`
pub const MAX_DESCRIPTION_PARAM_LENGTH: usize = 1000;
/// `HasCustomFields::CUSTOM_FIELDS_MAX_ITEMS`
const CUSTOM_FIELDS_MAX_ITEMS: usize = 100;
/// `Group::AUTO_GROUPS`
const AUTO_GROUPS: [(&str, i32); 13] = [
    ("everyone", 0),
    ("admins", 1),
    ("moderators", 2),
    ("staff", 3),
    ("trust_level_0", 10),
    ("trust_level_1", 11),
    ("trust_level_2", 12),
    ("trust_level_3", 13),
    ("trust_level_4", 14),
    ("anonymous_users", 4),
    ("logged_in_users", 5),
    ("trusted_users", 6),
    ("all_users", 7),
];

/// What the save fails with before touching anything: ActiveRecord's
/// ArgumentError on assignment (an unknown enum value or group), which
/// the controller renders as a 422 with its message.
#[derive(Debug)]
pub enum Rejected {
    Argument(String),
    Invalid(Vec<String>),
}

/// `category_params`: the permitted keys as given.
#[derive(Debug, Default, Clone)]
pub struct Params {
    /// Scalar attributes in the permit order (position included).
    pub attrs: Vec<(&'static str, Value)>,
    /// `permissions`: group name or id to permission type (`to_i`).
    pub permissions: Option<Vec<(String, i64)>>,
    pub custom_fields: Option<Map<String, Value>>,
    pub category_setting_attributes: Option<Map<String, Value>>,
    pub allowed_tags: Option<Vec<String>>,
    pub allowed_tag_groups: Option<Vec<String>>,
    pub required_tag_groups: Option<Vec<(String, i64)>>,
    pub form_template_ids: Option<Vec<i64>>,
    pub topic_posting_review_group_ids: Option<Vec<i64>>,
    pub reply_posting_review_group_ids: Option<Vec<i64>>,
}

impl Params {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn has(&self, key: &str) -> bool {
        self.attrs.iter().any(|(k, _)| *k == key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.attrs.iter().position(|(k, _)| *k == key)?;
        Some(self.attrs.remove(i).1)
    }

    /// The keys in category_params' order (a log reads them so).
    pub fn keys(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = self.attrs.iter().map(|(k, _)| *k).collect();
        if self.topic_posting_review_group_ids.is_some() {
            out.push("topic_posting_review_group_ids");
        }
        if self.reply_posting_review_group_ids.is_some() {
            out.push("reply_posting_review_group_ids");
        }
        if self.category_setting_attributes.is_some() {
            out.push("category_setting_attributes");
        }
        if self.custom_fields.is_some() {
            out.push("custom_fields");
        }
        if self.permissions.is_some() {
            out.push("permissions");
        }
        if self.allowed_tags.is_some() {
            out.push("allowed_tags");
        }
        if self.allowed_tag_groups.is_some() {
            out.push("allowed_tag_groups");
        }
        if self.required_tag_groups.is_some() {
            out.push("required_tag_groups");
        }
        if self.form_template_ids.is_some() {
            out.push("form_template_ids");
        }
        out
    }
}

/// A permitted scalar (`permit` drops arrays and hashes for scalar keys).
fn permitted_scalar(v: &Value) -> bool {
    !matches!(v, Value::Array(_) | Value::Object(_))
}

/// An array of permitted scalars (`key: []`), none for another shape.
fn scalar_list(v: &Value) -> Option<Vec<Value>> {
    match v {
        Value::Array(items) => Some(
            items
                .iter()
                .filter(|i| permitted_scalar(i))
                .cloned()
                .collect(),
        ),
        _ => None,
    }
}

fn ids(items: Vec<Value>) -> Vec<i64> {
    items
        .iter()
        .filter_map(|v| match v {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) if !s.trim().is_empty() => Some(crate::ruby::to_i(s)),
            _ => None,
        })
        .collect()
}

fn strings(items: Vec<Value>) -> Vec<String> {
    items.iter().filter_map(crate::params::scalar).collect()
}

/// `category_params` over the request's params.
pub fn permit(p: &Map<String, Value>, s: &SiteSettings) -> Result<Params, AppError> {
    let mut out = Params::default();
    if s.get("enable_category_group_moderation")?.truthy() && p.contains_key("moderating_group_ids")
    {
        return Err(Unsupported("category group moderation (moderating_group_ids)").into());
    }
    if s.get("content_localization_enabled")?.truthy()
        && (p.contains_key("locale") || p.contains_key("category_localizations"))
    {
        return Err(Unsupported("category localizations").into());
    }
    for (key, _) in PERMITTED {
        if let Some(v) = p.get(*key)
            && permitted_scalar(v)
        {
            out.attrs.push((key, v.clone()));
        }
    }
    let tagging = s.get("tagging_enabled")?.truthy();
    // tagging_enabled: a blank list param becomes [] before the permit.
    let list_param = |key: &str| -> Option<Vec<Value>> {
        let v = p.get(key)?;
        if tagging && blank_param(v) {
            return Some(Vec::new());
        }
        scalar_list(v)
    };
    out.topic_posting_review_group_ids = p
        .get("topic_posting_review_group_ids")
        .and_then(scalar_list)
        .map(ids);
    out.reply_posting_review_group_ids = p
        .get("reply_posting_review_group_ids")
        .and_then(scalar_list)
        .map(ids);
    if let Some(Value::Object(attrs)) = p.get("category_setting_attributes") {
        let mut kept = Map::new();
        for key in [
            "auto_bump_cooldown_days",
            "num_auto_bump_daily",
            "require_reply_approval",
            "require_topic_approval",
            "nested_replies_default",
            "topic_posting_review_mode",
            "reply_posting_review_mode",
        ] {
            if let Some(v) = attrs.get(key)
                && permitted_scalar(v)
            {
                kept.insert(key.into(), v.clone());
            }
        }
        out.category_setting_attributes = Some(kept);
    }
    // custom_fields: {} permits nothing (a hash of scalars only with
    // permit!; Rails' `custom_fields: {}` keeps scalar values).
    if let Some(Value::Object(fields)) = p.get("custom_fields") {
        let kept: Map<String, Value> = fields
            .iter()
            .filter(|(_, v)| permitted_scalar(v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.custom_fields = Some(kept);
    }
    if let Some(Value::Object(perms)) = p.get("permissions") {
        let mut kept = Vec::new();
        for (group, v) in perms {
            if !permitted_scalar(v) {
                continue;
            }
            // `p[k] = v.to_i`
            let level = match v {
                Value::Number(n) => n
                    .as_i64()
                    .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
                Value::String(s) => crate::ruby::to_i(s),
                Value::Bool(_) => {
                    return Err(Unsupported("a category permission given as a boolean").into());
                }
                _ => 0,
            };
            kept.push((group.clone(), level));
        }
        out.permissions = Some(kept);
    }
    out.allowed_tags = list_param("allowed_tags").map(strings);
    out.allowed_tag_groups = list_param("allowed_tag_groups").map(strings);
    if let Some(v) = p.get("required_tag_groups") {
        let groups = if tagging && blank_param(v) {
            Some(Vec::new())
        } else {
            match v {
                Value::Array(items) => {
                    let mut kept = Vec::new();
                    for item in items {
                        let Value::Object(g) = item else { continue };
                        let name = g
                            .get("name")
                            .and_then(crate::params::scalar)
                            .unwrap_or_default();
                        let min = g
                            .get("min_count")
                            .and_then(crate::params::scalar)
                            .map(|v| crate::ruby::to_i(&v))
                            .unwrap_or(0);
                        kept.push((name, min));
                    }
                    Some(kept)
                }
                Value::Object(_) => {
                    // Not an Array once permitted: InvalidParameters.
                    return Err(Unsupported("required_tag_groups given as a hash").into());
                }
                _ => None,
            }
        };
        out.required_tag_groups = groups;
    }
    out.form_template_ids = p.get("form_template_ids").and_then(scalar_list).map(ids);
    Ok(out)
}

/// A value as ActiveRecord casts it to the column's kind.
fn cast(kind: Kind, v: &Value) -> Value {
    match kind {
        Kind::Str => match v {
            Value::Null => Value::Null,
            Value::String(s) => json!(s),
            Value::Bool(b) => json!(if *b { "t" } else { "f" }),
            Value::Number(n) => json!(n.to_string()),
            _ => Value::Null,
        },
        Kind::Bool => match v {
            Value::Null => Value::Null,
            Value::Bool(b) => json!(b),
            Value::String(s) => json!(crate::ruby::cast_boolean(Some(s))),
            Value::Number(n) => json!(n.as_f64() != Some(0.0)),
            _ => Value::Null,
        },
        Kind::Int => match v {
            Value::Null => Value::Null,
            Value::Bool(b) => json!(i64::from(*b)),
            Value::String(s) if s.is_empty() => Value::Null,
            Value::String(s) => json!(crate::ruby::to_i(s)),
            Value::Number(n) => json!(
                n.as_i64()
                    .unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64)
            ),
            _ => Value::Null,
        },
        Kind::Float => match v {
            Value::Null => Value::Null,
            Value::Bool(b) => json!(if *b { 1.0 } else { 0.0 }),
            Value::String(s) if s.is_empty() => Value::Null,
            Value::String(s) => json!(crate::ruby::to_f(s)),
            Value::Number(n) => json!(n.as_f64().unwrap_or(0.0)),
            _ => Value::Null,
        },
    }
}

/// `Category.style_types` (an enum: an unknown value is an ArgumentError).
fn style_type(v: &Value) -> Result<Value, Rejected> {
    let named = |s: &str| match s {
        "square" | "0" => Some(0),
        "icon" | "1" => Some(1),
        "emoji" | "2" => Some(2),
        _ => None,
    };
    match v {
        Value::Null => Ok(Value::Null),
        Value::String(s) if s.is_empty() => Ok(Value::Null),
        Value::String(s) => named(s)
            .map(|i| json!(i))
            .ok_or_else(|| Rejected::Argument(format!("'{s}' is not a valid style_type"))),
        Value::Number(n) => match n.as_i64() {
            Some(i @ 0..=2) => Ok(json!(i)),
            _ => Err(Rejected::Argument(format!(
                "'{n}' is not a valid style_type"
            ))),
        },
        other => Err(Rejected::Argument(format!(
            "'{}' is not a valid style_type",
            crate::params::scalar(other).unwrap_or_default()
        ))),
    }
}

/// A category being saved: its columns as JSON, as loaded and as
/// assigned, and the raw values the numericality validations read.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: Option<i32>,
    pub original: Map<String, Value>,
    pub values: Map<String, Value>,
    raw: Map<String, Value>,
    /// `@permissions`: resolved group ids and types to apply on save.
    permissions: Option<Vec<(i32, i64)>>,
    /// The group names of `@permissions` as given, for the logs.
    pub permissions_given: Option<Vec<(String, i64)>>,
    custom_fields: Option<Map<String, Value>>,
    custom_fields_original: Map<String, Value>,
    setting: Option<Map<String, Value>>,
    setting_exists: bool,
    review_groups: [Option<Vec<i64>>; 2],
    form_template_ids: Option<Vec<i64>>,
    allowed_tags: Option<Vec<String>>,
    allowed_tag_groups: Option<Vec<String>>,
    required_tag_groups: Option<Vec<(String, i64)>>,
    /// `saved_changes` after a save: each changed column, before and after.
    pub saved_changes: Vec<(String, Value, Value)>,
    /// Jobs enqueued with DB.after_commit, after the model's own
    /// after_commit callbacks (`run_deferred`).
    deferred: Vec<(&'static str, Value)>,
}

impl Record {
    pub fn new(user_id: i32) -> Record {
        let mut values = defaults();
        values.insert("user_id".into(), json!(user_id));
        Record {
            id: None,
            original: Map::new(),
            values,
            raw: Map::new(),
            permissions: None,
            permissions_given: None,
            custom_fields: None,
            custom_fields_original: Map::new(),
            setting: None,
            setting_exists: false,
            review_groups: [None, None],
            form_template_ids: None,
            allowed_tags: None,
            allowed_tag_groups: None,
            required_tag_groups: None,
            saved_changes: Vec::new(),
            deferred: Vec::new(),
        }
    }

    pub async fn load(conn: &mut PgConnection, id: i32) -> Result<Option<Record>, AppError> {
        let row: Option<Value> =
            sqlx::query_scalar("SELECT to_jsonb(c) FROM categories c WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        let Some(Value::Object(values)) = row else {
            return Ok(None);
        };
        let setting_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM category_settings WHERE category_id = $1)",
        )
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        let custom = crate::category_serializer::custom_fields(&mut *conn, id).await?;
        let mut r = Record::new(0);
        r.id = Some(id);
        r.original = values.clone();
        r.values = values;
        r.setting_exists = setting_exists;
        r.custom_fields_original = custom;
        Ok(Some(r))
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(Value::as_str)
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        self.values.get(key).and_then(Value::as_i64)
    }

    pub fn changed(&self, key: &str) -> bool {
        self.values.get(key).unwrap_or(&Value::Null)
            != self.original.get(key).unwrap_or(&Value::Null)
    }

    fn is_new(&self) -> bool {
        self.id.is_none()
    }

    /// The columns whose values differ from the loaded ones, as
    /// `[before, after]`.
    pub fn changes(&self) -> Vec<(String, Value, Value)> {
        self.values
            .iter()
            .filter(|(k, v)| self.original.get(*k).unwrap_or(&Value::Null) != *v)
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.original.get(k).cloned().unwrap_or(Value::Null),
                    v.clone(),
                )
            })
            .collect()
    }

    /// `assign_attributes(category_params)`, the scalars then the
    /// association writers.
    pub async fn assign(
        &mut self,
        conn: &mut PgConnection,
        params: &Params,
    ) -> Result<Result<(), Rejected>, AppError> {
        for (key, v) in &params.attrs {
            if *key == "position" {
                continue;
            }
            let kind = PERMITTED
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, kind)| *kind)
                .unwrap_or(Kind::Str);
            let value = if *key == "style_type" {
                match style_type(v) {
                    Ok(v) => v,
                    Err(e) => return Ok(Err(e)),
                }
            } else {
                cast(kind, v)
            };
            if value.is_null() && NOT_NULL.contains(key) && *key != "name" && *key != "slug" {
                return Err(Unsupported("nil for a required category column").into());
            }
            self.raw.insert((*key).to_string(), v.clone());
            self.values.insert((*key).to_string(), value);
        }
        if params.has("uploaded_logo_id")
            || params.has("uploaded_logo_dark_id")
            || params.has("uploaded_background_id")
            || params.has("uploaded_background_dark_id")
        {
            for key in [
                "uploaded_logo_id",
                "uploaded_logo_dark_id",
                "uploaded_background_id",
                "uploaded_background_dark_id",
            ] {
                if self.changed(key) {
                    return Err(Unsupported("category logos and backgrounds").into());
                }
            }
        }
        if params.has("email_in")
            && self.changed("email_in")
            && self.str("email_in").is_some_and(|e| !e.trim().is_empty())
        {
            return Err(Unsupported("category incoming email addresses (email_in)").into());
        }
        if let Some(perms) = &params.permissions {
            // set_permissions: Category.resolve_permissions
            let mut mapped = Vec::with_capacity(perms.len());
            for (group, level) in perms {
                let id = match group_id_from_param(conn, group).await? {
                    Some(id) => id,
                    None => return Ok(Err(Rejected::Argument("unknown group".into()))),
                };
                mapped.push((id, *level));
            }
            let mut read_restricted = true;
            let mut everyone_full = false;
            for (group, level) in &mapped {
                if *group == 0 && *level == 1 {
                    everyone_full = true;
                    break;
                }
                if *group == 0 {
                    read_restricted = false;
                }
            }
            if everyone_full {
                self.values.insert("read_restricted".into(), json!(false));
                self.permissions = Some(Vec::new());
            } else {
                self.values
                    .insert("read_restricted".into(), json!(read_restricted));
                self.permissions = Some(mapped);
            }
            self.permissions_given = Some(perms.clone());
        }
        if let Some(fields) = &params.custom_fields {
            // custom_fields = data: replaced.
            let mut merged = Map::new();
            for (k, v) in fields {
                merged.insert(k.clone(), v.clone());
            }
            self.custom_fields = Some(merged);
        }
        if let Some(attrs) = &params.category_setting_attributes {
            self.setting = Some(attrs.clone());
        }
        self.review_groups = [
            params.topic_posting_review_group_ids.clone(),
            params.reply_posting_review_group_ids.clone(),
        ];
        self.form_template_ids = params.form_template_ids.clone();
        self.allowed_tags = params.allowed_tags.clone();
        self.allowed_tag_groups = params.allowed_tag_groups.clone();
        self.required_tag_groups = params.required_tag_groups.clone();
        Ok(Ok(()))
    }
}

/// `Group.group_id_from_param`: an id given as digits, an automatic
/// group's name, else a group by exact name (ArgumentError without one).
async fn group_id_from_param(
    conn: &mut PgConnection,
    group: &str,
) -> Result<Option<i32>, AppError> {
    if let Ok(id) = group.parse::<i32>()
        && id.to_string() == group
    {
        return Ok(Some(id));
    }
    if let Some((_, id)) = AUTO_GROUPS.iter().find(|(name, _)| *name == group) {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        if !exists {
            return Err(Unsupported("an automatic group missing from the groups table").into());
        }
        return Ok(Some(*id));
    }
    Ok(sqlx::query_scalar("SELECT id FROM groups WHERE name = $1")
        .bind(group)
        .fetch_optional(&mut *conn)
        .await?)
}

/// What a save reads besides the database: the site (config, setting
/// definitions, translations, as cooking takes them), its bus and
/// settings.
pub struct Cx<'a> {
    pub host: &'a crate::pretty_text::Host,
    pub bus: &'a pg_bus::Bus,
    pub settings: &'a SiteSettings,
}

impl<'a> Cx<'a> {
    /// From a post's context (the user destroyer, the owner changer).
    pub fn from_posting(ctx: &'a crate::posting::Ctx<'a>) -> Cx<'a> {
        Cx {
            host: ctx.host,
            bus: ctx.bus,
            settings: ctx.settings,
        }
    }

    fn i18n(&self) -> &I18n {
        &self.host.i18n
    }

    fn t(&self, key: &str) -> String {
        self.host.i18n.t(key).unwrap_or(key).to_string()
    }

    /// The serializer's context.
    pub fn ser(&self) -> crate::category_serializer::Cx<'_> {
        crate::category_serializer::Cx {
            config: &self.host.config,
            i18n: &self.host.i18n,
            defs: &self.host.site_setting_defs,
        }
    }
}

/// ActiveRecord's `human_attribute_name` for a category attribute.
fn human_attribute(i18n: &I18n, attr: &str) -> String {
    if let Some(name) = i18n.t(&format!("activerecord.attributes.category.{attr}")) {
        return name.to_string();
    }
    // humanize: underscores to spaces, a trailing _id dropped, the first
    // letter upcased.
    let base = attr.strip_suffix("_id").unwrap_or(attr).replace('_', " ");
    let mut chars = base.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The errors a validation adds, as full messages, in order.
struct Errors<'a> {
    i18n: &'a I18n,
    list: Vec<String>,
}

impl Errors<'_> {
    fn add(&mut self, attr: &str, message: String) {
        self.list
            .push(format!("{} {message}", human_attribute(self.i18n, attr)));
    }

    fn base(&mut self, message: String) {
        self.list.push(message);
    }

    fn message(&self, key: &str, args: &[(&str, &str)]) -> String {
        self.i18n
            .t_with(&format!("errors.messages.{key}"), args)
            .unwrap_or_else(|| key.to_string())
    }

    fn message_count(&self, key: &str, count: i64) -> String {
        self.i18n
            .t_count(&format!("errors.messages.{key}"), count, &[])
            .unwrap_or_else(|| key.to_string())
    }
}

/// A number as NumericalityValidator parses the raw value: None for "is
/// not a number"; `integer` when it has no fractional part as given.
struct Number {
    value: f64,
    integer: bool,
}

fn parse_number(raw: &Value) -> Option<Number> {
    match raw {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(Number {
                    value: i as f64,
                    integer: true,
                })
            } else {
                Some(Number {
                    value: n.as_f64()?,
                    integer: false,
                })
            }
        }
        Value::String(s) => {
            let t = s.trim();
            let int_like = {
                let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
                !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
            };
            // Kernel.Float: digits with an optional fraction or exponent.
            let value: f64 = t.replace('_', "").parse().ok()?;
            if t.is_empty() || t.contains("inf") || t.contains("nan") || t.contains("Inf") {
                return None;
            }
            Some(Number {
                value,
                integer: int_like,
            })
        }
        Value::Bool(_) => None,
        _ => None,
    }
}

impl Record {
    /// The value NumericalityValidator reads: the raw one when assigned
    /// from params, else the column's.
    fn numeric_source(&self, key: &str) -> Value {
        self.raw
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.values.get(key).cloned().unwrap_or(Value::Null))
    }
}

/// `validates numericality:` with only_integer and bounds.
#[allow(clippy::too_many_arguments)]
fn numericality(
    errors: &mut Errors<'_>,
    r: &Record,
    attr: &str,
    allow_nil: bool,
    only_integer: bool,
    greater_than: Option<f64>,
    less_than_or_equal_to: Option<f64>,
) {
    let raw = r.numeric_source(attr);
    if raw.is_null() || raw.as_str().is_some_and(|s| s.is_empty()) {
        if !allow_nil {
            let m = errors.message("not_a_number", &[]);
            errors.add(attr, m);
        }
        return;
    }
    let Some(n) = parse_number(&raw) else {
        let m = errors.message("not_a_number", &[]);
        errors.add(attr, m);
        return;
    };
    if only_integer && !n.integer {
        let m = errors.message("not_an_integer", &[]);
        errors.add(attr, m);
        return;
    }
    let show = |f: f64| -> String {
        if f.fract() == 0.0 {
            format!("{}", f as i64)
        } else {
            f.to_string()
        }
    };
    if let Some(g) = greater_than
        && n.value <= g
    {
        let m = errors.message("greater_than", &[("count", &show(g))]);
        errors.add(attr, m);
    }
    if let Some(l) = less_than_or_equal_to
        && n.value > l
    {
        let m = errors.message("less_than_or_equal_to", &[("count", &show(l))]);
        errors.add(attr, m);
    }
}

/// `Emoji.exists?(name)`: a standard emoji or alias, or a custom one,
/// with the colons and a skin tone stripped.
fn emoji_exists(s: &SiteSettings, name: &str) -> bool {
    let trimmed = name.strip_prefix(':').unwrap_or(name);
    let trimmed = trimmed.strip_suffix(':').unwrap_or(trimmed);
    let base = match trimmed.rsplit_once(":t") {
        Some((base, "1" | "2" | "3" | "4" | "5" | "6")) => base,
        _ => trimmed,
    };
    discourse_markdown::emoji::DATA.exists(base) || s.emojis().custom(base).is_some()
}

/// `Slug.for(string, "", method: :encoded)`
fn encoded_slug(s: &str) -> String {
    const FILTER: &str = ":/?#[]@!$&'()*+,;=_.~%\\`^|{}\"<>";
    let mut collapsed = String::new();
    let mut in_space = false;
    for c in crate::ruby::strip(s).chars() {
        if c.is_whitespace() {
            if !in_space {
                collapsed.push('-');
            }
            in_space = true;
        } else {
            in_space = false;
            collapsed.push(c);
        }
    }
    let filtered: String = collapsed
        .chars()
        .filter(|c| !FILTER.contains(*c) && !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    let escaped = crate::ruby::cgi_escape(&filtered);
    prettify(&escaped)
}

/// `Slug.prettify_slug` without truncation, then the blank or numeric
/// default ("").
fn prettify(slug: &str) -> String {
    if slug.bytes().all(|b| b.is_ascii_digit()) {
        return String::new();
    }
    let mut out = String::new();
    for c in slug.chars().map(|c| if c == '_' { '-' } else { c }) {
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() || out.bytes().all(|b| b.is_ascii_digit()) {
        String::new()
    } else {
        out
    }
}

/// `CGI.unescape`
fn cgi_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len()
                && bytes[i + 1].is_ascii_hexdigit()
                && bytes[i + 2].is_ascii_hexdigit() =>
            {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `Category#ensure_slug` (a validation that also sets the slug).
async fn ensure_slug(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Record,
    errors: &mut Errors<'_>,
) -> Result<(), AppError> {
    let Some(name) = r.str("name").filter(|n| !crate::ruby::is_blank(n)) else {
        return Ok(());
    };
    let name = crate::ruby::strip(name).to_string();
    r.values.insert("name".into(), json!(name));
    let method = cx.settings.get("slug_generation_method")?.to_s();
    let given = r
        .str("slug")
        .filter(|s| !crate::ruby::is_blank(s))
        .map(str::to_string);
    if let Some(given) = given {
        let unescaped = if method == "encoded" {
            cgi_unescape(&given)
        } else {
            given
        };
        let slug = encoded_slug(&unescaped);
        r.values.insert("slug".into(), json!(slug));
        if slug.is_empty() {
            let m = errors.message("invalid", &[]);
            errors.add("slug", m);
        } else if method == "ascii" && !cgi_unescape(&slug).is_ascii() {
            errors.add(
                "slug",
                cx.t("category.errors.slug_contains_non_ascii_chars"),
            );
        } else if duplicate_slug(conn, r, &slug).await? {
            errors.add("slug", cx.t("category.errors.is_already_in_use"));
        }
    } else {
        let slug = match method.as_str() {
            "ascii" => crate::posting::text::slug_with_default(
                &name,
                &cx.settings.get("default_locale")?.to_s(),
                "",
            )?,
            "encoded" => encoded_slug(&name),
            _ => String::new(),
        };
        let slug = if !slug.is_empty() && duplicate_slug(conn, r, &slug).await? {
            String::new()
        } else {
            slug
        };
        r.values.insert("slug".into(), json!(slug));
    }
    // Only the category's own id may name it `<id>-category`.
    let slug = r.str("slug").unwrap_or("").to_string();
    if let Some(rest) = slug.split_once("-category").map(|(head, _)| head)
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
        && slug.starts_with(&format!("{rest}-category"))
        && (r.is_new() || r.id.map(|id| id.to_string()) != Some(rest.to_string()))
    {
        let m = errors.message("invalid", &[]);
        errors.add("slug", m);
    }
    Ok(())
}

/// `Category#duplicate_slug?`
async fn duplicate_slug(conn: &mut PgConnection, r: &Record, slug: &str) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories WHERE slug = $1 \
           AND parent_category_id IS NOT DISTINCT FROM $2 AND id IS DISTINCT FROM $3)",
    )
    .bind(slug)
    .bind(r.int("parent_category_id").map(|i| i as i32))
    .bind(r.id)
    .fetch_one(&mut *conn)
    .await?)
}

/// `height_of_ancestors` and `depth_of_descendants` for the record's
/// parent, as the validation computes them before saving.
async fn nesting(conn: &mut PgConnection, r: &Record, max: i64) -> Result<i64, AppError> {
    let parent = r.int("parent_category_id").map(|i| i as i32);
    let id = r.id;
    if parent.is_some() && parent == id {
        return Ok(max + 1 + max);
    }
    let height: Option<i32> = sqlx::query_scalar(
        "WITH RECURSIVE ancestors(parent_category_id, height) AS ( \
           SELECT $2::integer, 0 \
           UNION ALL \
           SELECT categories.parent_category_id, \
                  CASE WHEN categories.parent_category_id = $1 THEN $3::integer ELSE ancestors.height + 1 END \
           FROM categories, ancestors \
           WHERE categories.id = ancestors.parent_category_id AND ancestors.height < $3) \
         SELECT max(height) FROM ancestors",
    )
    .bind(id)
    .bind(parent)
    .bind(max as i32)
    .fetch_one(&mut *conn)
    .await?;
    let depth: Option<i32> = sqlx::query_scalar(
        "WITH RECURSIVE descendants(id, depth) AS ( \
           SELECT $1::integer, 0 \
           UNION ALL \
           SELECT categories.id, \
                  CASE WHEN categories.id = $2 THEN $3::integer ELSE descendants.depth + 1 END \
           FROM categories, descendants \
           WHERE categories.parent_category_id = descendants.id AND descendants.depth < $3) \
         SELECT max(depth) FROM descendants",
    )
    .bind(id)
    .bind(parent)
    .bind(max as i32)
    .fetch_one(&mut *conn)
    .await?;
    Ok(i64::from(height.unwrap_or(0)) + 1 + i64::from(depth.unwrap_or(0)))
}

/// `check_permissions_compatibility(parent, child)`
async fn permission_conflict(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    errors: &mut Errors<'_>,
    parent: &[(i32, i64)],
    child: &[(i32, i64)],
) -> Result<(), AppError> {
    let parent_groups: Vec<i32> = parent.iter().map(|(g, _)| *g).collect();
    if parent_groups.contains(&0) {
        return Ok(());
    }
    let only_child: Vec<i32> = child
        .iter()
        .map(|(g, _)| *g)
        .filter(|g| !parent_groups.contains(g))
        .collect();
    if only_child.is_empty() {
        return Ok(());
    }
    let names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM groups WHERE id = ANY($1) ORDER BY id")
            .bind(&only_child)
            .fetch_all(&mut *conn)
            .await?;
    errors.base(
        cx.i18n()
            .t_with(
                "category.errors.permission_conflict",
                &[("group_names", &names.join(", "))],
            )
            .unwrap_or_default(),
    );
    Ok(())
}

/// The validations in declaration order, after before_validation's
/// normalizations.
async fn validate(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Record,
) -> Result<Vec<String>, AppError> {
    // before_validation
    if !r
        .str("default_top_period")
        .is_some_and(|p| TOP_PERIODS.contains(&p))
    {
        r.values.insert("default_top_period".into(), Value::Null);
    }
    if r.values
        .get("minimum_required_tags")
        .is_none_or(Value::is_null)
    {
        r.values.insert("minimum_required_tags".into(), json!(0));
    }
    let i18n = cx.i18n();
    let mut errors = Errors {
        i18n,
        list: Vec::new(),
    };
    // HasCustomFields, when the fields changed.
    if let Some(fields) = &r.custom_fields
        && fields.len() > CUSTOM_FIELDS_MAX_ITEMS
    {
        errors.base(
            i18n.t_with(
                "custom_fields.validations.max_items",
                &[("max_items_number", &CUSTOM_FIELDS_MAX_ITEMS.to_string())],
            )
            .unwrap_or_default(),
        );
    }
    // name: on create, or when it or the parent changes.
    if r.is_new() || r.changed("name") || r.changed("parent_category_id") {
        let name = r.str("name").map(str::to_string);
        if name.as_deref().is_none_or(crate::ruby::is_blank) {
            let m = errors.message("blank", &[]);
            errors.add("name", m);
        }
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM categories WHERE LOWER(name) = LOWER($1) \
               AND parent_category_id IS NOT DISTINCT FROM $2 AND id IS DISTINCT FROM $3)",
        )
        .bind(&name)
        .bind(r.int("parent_category_id").map(|i| i as i32))
        .bind(r.id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            let m = errors.message("taken", &[]);
            errors.add("name", m);
        }
        let len = name.as_deref().map(|n| n.chars().count()).unwrap_or(0) as i64;
        if len < 1 {
            let m = errors.message_count("too_short", 1);
            errors.add("name", m);
        } else if len > 50 {
            let m = errors.message_count("too_long", 50);
            errors.add("name", m);
        }
    }
    numericality(
        &mut errors,
        r,
        "num_featured_topics",
        false,
        true,
        Some(0.0),
        None,
    );
    if !r
        .int("search_priority")
        .is_some_and(|p| SEARCH_PRIORITIES.contains(&p))
    {
        let m = errors.message("inclusion", &[]);
        errors.add("search_priority", m);
    }
    // emoji_exists
    if let Some(emoji) = r.str("emoji").filter(|e| !crate::ruby::is_blank(e))
        && !emoji_exists(cx.settings, emoji)
    {
        let m = errors.message("invalid", &[]);
        errors.add("emoji", m);
    }
    // parent_category_validator
    if let Some(parent) = r.int("parent_category_id") {
        let uncategorized = cx.settings.get("uncategorized_category_id")?.to_i();
        if r.id.map(i64::from) == Some(uncategorized) {
            errors.base(cx.t("category.errors.uncategorized_parent"));
        }
        if r.id.map(i64::from) == Some(parent) {
            errors.base(cx.t("category.errors.self_parent"));
        }
        let max = cx.settings.get("max_category_nesting")?.to_i();
        if nesting(conn, r, max).await? > max {
            errors.base(cx.t("category.errors.depth"));
        }
    }
    // email_in_validator: email_in is refused on assignment.
    ensure_slug(conn, cx, r, &mut errors).await?;
    // permissions_compatibility_validator
    if let Some(perms) = r.permissions.clone() {
        if let Some(parent) = r.int("parent_category_id") {
            let parent_perms: Vec<(i32, i64)> = sqlx::query_as::<_, (i32, i32)>(
                "SELECT group_id, permission_type FROM category_groups WHERE category_id = $1 ORDER BY id",
            )
            .bind(parent as i32)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .map(|(g, p)| (g, i64::from(p)))
            .collect();
            if !parent_perms.is_empty() {
                let child = if perms.is_empty() {
                    vec![(0, 1)]
                } else {
                    perms.clone()
                };
                permission_conflict(conn, cx, &mut errors, &parent_perms, &child).await?;
            }
        } else if let Some(id) = r.id
            && !perms.is_empty()
        {
            let has_subcategories: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM categories WHERE parent_category_id = $1)",
            )
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
            if has_subcategories {
                let child: Vec<(i32, i64)> = sqlx::query_as::<_, (i32, i32)>(
                    "SELECT DISTINCT * FROM ( \
                       SELECT category_groups.group_id, category_groups.permission_type \
                       FROM categories, category_groups \
                       WHERE categories.parent_category_id = $1 \
                         AND categories.id = category_groups.category_id \
                       UNION \
                       SELECT 0, 1 FROM categories \
                       WHERE categories.parent_category_id = $1 \
                         AND (SELECT DISTINCT 1 FROM category_groups \
                              WHERE category_groups.category_id = categories.id) IS NULL) x",
                )
                .bind(id)
                .fetch_all(&mut *conn)
                .await?
                .into_iter()
                .map(|(g, p)| (g, i64::from(p)))
                .collect();
                permission_conflict(conn, cx, &mut errors, &perms, &child).await?;
            }
        }
    }
    // posting_review_groups_validator
    let (topic_mode, reply_mode) = review_modes(conn, r).await?;
    for (post_type, mode, slot) in [("topic", topic_mode, 0), ("reply", reply_mode, 1)] {
        if mode == 2 || mode == 3 {
            let groups = match &r.review_groups[slot] {
                Some(ids) => ids.clone(),
                None => match r.id {
                    Some(id) => sqlx::query_scalar::<_, i32>(
                        "SELECT group_id FROM category_posting_review_groups \
                         WHERE category_id = $1 AND post_type = $2",
                    )
                    .bind(id)
                    .bind(slot as i32)
                    .fetch_all(&mut *conn)
                    .await?
                    .into_iter()
                    .map(i64::from)
                    .collect(),
                    None => Vec::new(),
                },
            };
            if groups.is_empty() {
                errors.base(
                    i18n.t_with(
                        "category.errors.groups_required_for_mode",
                        &[("setting", &format!("{post_type}_posting_review_mode"))],
                    )
                    .unwrap_or_default(),
                );
            }
        }
    }
    numericality(
        &mut errors,
        r,
        "default_slow_mode_seconds",
        true,
        true,
        Some(0.0),
        None,
    );
    numericality(
        &mut errors,
        r,
        "auto_close_hours",
        true,
        false,
        Some(0.0),
        Some(87_600.0),
    );
    if r.str("slug").is_some_and(|s| RESERVED_SLUGS.contains(&s)) {
        let m = errors.message("exclusion", &[]);
        errors.add("slug", m);
    }
    let hex = |v: Option<&str>| {
        v.is_some_and(|c| {
            (c.len() == 6 || c.len() == 3) && c.bytes().all(|b| b.is_ascii_hexdigit())
        })
    };
    if !hex(r.str("color")) {
        let m = errors.message("invalid", &[]);
        errors.add("color", m);
    }
    if !hex(r.str("text_color")) {
        let m = errors.message("invalid", &[]);
        errors.add("text_color", m);
    }
    // The autosaved category_setting's own validations.
    if let Some(attrs) = &r.setting {
        for key in ["num_auto_bump_daily", "auto_bump_cooldown_days"] {
            let Some(raw) = attrs.get(key) else { continue };
            if raw.is_null() || raw.as_str().is_some_and(str::is_empty) {
                continue;
            }
            let human = format!("Category setting {}", key.replace('_', " "));
            match parse_number(raw) {
                None => errors
                    .list
                    .push(format!("{human} {}", errors.message("not_a_number", &[]))),
                Some(n) if !n.integer => errors
                    .list
                    .push(format!("{human} {}", errors.message("not_an_integer", &[]))),
                Some(n) if n.value < 0.0 => errors.list.push(format!(
                    "{human} {}",
                    errors.message("greater_than_or_equal_to", &[("count", "0")])
                )),
                _ => {}
            }
        }
    }
    Ok(errors.list)
}

/// The category's review modes as the save will leave them.
async fn review_modes(conn: &mut PgConnection, r: &Record) -> Result<(i32, i32), AppError> {
    let stored: Option<(i32, i32)> = match r.id {
        Some(id) => {
            sqlx::query_as(
                "SELECT topic_posting_review_mode, reply_posting_review_mode \
                 FROM category_settings WHERE category_id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?
        }
        None => None,
    };
    let (mut topic, mut reply) = stored.unwrap_or((0, 0));
    if let Some(attrs) = &r.setting {
        if let Some(v) = attrs.get("require_topic_approval") {
            topic = if cast(Kind::Bool, v).as_bool() == Some(true) {
                1
            } else {
                0
            };
        }
        if let Some(v) = attrs.get("require_reply_approval") {
            reply = if cast(Kind::Bool, v).as_bool() == Some(true) {
                1
            } else {
                0
            };
        }
        if let Some(v) = attrs.get("topic_posting_review_mode") {
            topic = review_mode_value(v)?;
        }
        if let Some(v) = attrs.get("reply_posting_review_mode") {
            reply = review_mode_value(v)?;
        }
    }
    Ok((topic, reply))
}

/// A posting review mode as given (its name or number).
fn review_mode_value(v: &Value) -> Result<i32, AppError> {
    let s = crate::params::scalar(v).unwrap_or_default();
    Ok(match s.as_str() {
        "no_one" | "0" => 0,
        "everyone" | "1" => 1,
        "everyone_except" | "2" => 2,
        "no_one_except" | "3" => 3,
        _ => return Err(Unsupported("an unknown posting review mode").into()),
    })
}

/// The outcome of a save.
pub enum Saved {
    Done(i32),
    Invalid(Vec<String>),
}

/// `Category#save` in a transaction the caller holds.
pub async fn save(conn: &mut PgConnection, cx: &Cx<'_>, r: &mut Record) -> Result<Saved, AppError> {
    let errors = validate(conn, cx, r).await?;
    if !errors.is_empty() {
        return Ok(Saved::Invalid(errors));
    }
    let is_new = r.is_new();
    // before_save: Positionable, then the category's own.
    if r.values.get("position").is_none_or(Value::is_null) {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM categories")
            .fetch_one(&mut *conn)
            .await?;
        r.values.insert("position".into(), json!(count));
    }
    let email = r
        .str("email_in")
        .map(|e| crate::ruby::strip(e).to_lowercase())
        .filter(|e| !e.is_empty());
    r.values.insert("email_in".into(), json!(email));
    if let Some(name) = r.str("name").map(str::to_lowercase) {
        r.values.insert("name_lower".into(), json!(name));
    }
    let (topic_mode, reply_mode) = review_modes(conn, r).await?;
    for (slot, mode) in [(0, topic_mode), (1, reply_mode)] {
        if mode != 2 && mode != 3 {
            r.review_groups[slot] = Some(Vec::new());
        }
    }
    let changes = r.changes();
    if !is_new
        && changes.iter().any(|(k, _, _)| k == "description")
        && definition_has_first_post(conn, r.id.unwrap_or_default()).await?
    {
        return Err(Unsupported(
            "revising a category's definition post when its description changes",
        )
        .into());
    }
    let id = if is_new {
        let cols: Vec<String> = r.values.keys().cloned().collect();
        let list = cols.join(", ");
        let select = cols
            .iter()
            .map(|c| format!("r.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let id: i32 = sqlx::query_scalar(&format!(
            "INSERT INTO categories ({list}, created_at, updated_at) \
             SELECT {select}, now(), now() FROM jsonb_populate_record(NULL::categories, $1) r \
             RETURNING id"
        ))
        .bind(Value::Object(r.values.clone()))
        .fetch_one(&mut *conn)
        .await?;
        r.id = Some(id);
        id
    } else {
        let id = r.id.unwrap_or_default();
        let changed: Vec<String> = r.changes().into_iter().map(|(k, _, _)| k).collect();
        if !changed.is_empty() {
            let sets = changed
                .iter()
                .map(|c| format!("{c} = r.{c}"))
                .collect::<Vec<_>>()
                .join(", ");
            sqlx::query(&format!(
                "UPDATE categories SET {sets}, updated_at = now() \
                 FROM jsonb_populate_record(NULL::categories, $1) r WHERE categories.id = $2"
            ))
            .bind(Value::Object(r.values.clone()))
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }
        id
    };
    // The associations autosaved before the after_create callbacks:
    // category_groups (apply_permissions), the posting review groups,
    // then the category_setting.
    if let Some(perms) = r.permissions.take() {
        sqlx::query("DELETE FROM category_groups WHERE category_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        for (group_id, level) in perms {
            sqlx::query(
                "INSERT INTO category_groups (category_id, group_id, permission_type, created_at, updated_at) \
                 VALUES ($1, $2, $3, now(), now())",
            )
            .bind(id)
            .bind(group_id)
            .bind(level as i32)
            .execute(&mut *conn)
            .await?;
        }
    }
    for slot in 0..2 {
        let Some(groups) = r.review_groups[slot].take() else {
            continue;
        };
        let current: Vec<i32> = sqlx::query_scalar(
            "SELECT group_id FROM category_posting_review_groups \
             WHERE category_id = $1 AND post_type = $2 ORDER BY id",
        )
        .bind(id)
        .bind(slot as i32)
        .fetch_all(&mut *conn)
        .await?;
        let wanted: Vec<i32> = groups.iter().map(|g| *g as i32).collect();
        sqlx::query(
            "DELETE FROM category_posting_review_groups \
             WHERE category_id = $1 AND post_type = $2 AND NOT (group_id = ANY($3))",
        )
        .bind(id)
        .bind(slot as i32)
        .bind(&wanted)
        .execute(&mut *conn)
        .await?;
        for g in wanted {
            if current.contains(&g) {
                continue;
            }
            sqlx::query(
                "INSERT INTO category_posting_review_groups \
                   (category_id, group_id, post_type, created_at, updated_at) \
                 VALUES ($1, $2, $3, now(), now())",
            )
            .bind(id)
            .bind(g)
            .bind(slot as i32)
            .execute(&mut *conn)
            .await?;
        }
    }
    save_setting(conn, r, id, topic_mode, reply_mode).await?;
    if is_new {
        // after_create
        create_category_definition(conn, cx, id).await?;
        delete_category_permalink(conn, id).await?;
    } else {
        after_update(conn, cx, id, &changes, &mut r.deferred).await?;
    }
    // The has_many :through writers (autosaved after the callbacks).
    save_tags(conn, cx, r, id).await?;
    save_form_templates(conn, r, id).await?;
    // after_save: HasCustomFields, then the category's.
    save_custom_fields(conn, r, id).await?;
    crate::bus::clear_anon_cache(cx.bus, &mut *conn).await?;
    crate::category_serializer::publish_category(
        &mut *conn,
        &cx.ser(),
        cx.bus,
        cx.settings,
        id,
        &crate::category_serializer::Preloaded {
            category_setting: true,
            ..Default::default()
        },
    )
    .await?;
    r.saved_changes = changes;
    r.original = r.values.clone();
    Ok(Saved::Done(id))
}

/// The has_one category_setting: built when missing
/// (ensure_category_setting), its nested attributes assigned.
async fn save_setting(
    conn: &mut PgConnection,
    r: &mut Record,
    id: i32,
    topic_mode: i32,
    reply_mode: i32,
) -> Result<(), AppError> {
    let attrs = r.setting.take();
    let int = |key: &str| -> Option<Option<i64>> {
        let v = attrs.as_ref()?.get(key)?;
        Some(cast(Kind::Int, v).as_i64())
    };
    let nested = attrs
        .as_ref()
        .and_then(|a| a.get("nested_replies_default"))
        .map(|v| cast(Kind::Bool, v).as_bool());
    if !r.setting_exists {
        sqlx::query(
            "INSERT INTO category_settings (category_id, num_auto_bump_daily, auto_bump_cooldown_days, \
               topic_posting_review_mode, reply_posting_review_mode, nested_replies_default, \
               created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now(), now())",
        )
        .bind(id)
        .bind(int("num_auto_bump_daily").unwrap_or(Some(0)).map(|v| v as i32))
        .bind(int("auto_bump_cooldown_days").unwrap_or(Some(1)).map(|v| v as i32))
        .bind(topic_mode)
        .bind(reply_mode)
        .bind(nested.flatten().unwrap_or(false))
        .execute(&mut *conn)
        .await?;
        r.setting_exists = true;
        return Ok(());
    }
    if attrs.is_none() {
        return Ok(());
    }
    let current: (Option<i32>, Option<i32>, i32, i32, bool) = sqlx::query_as(
        "SELECT num_auto_bump_daily, auto_bump_cooldown_days, topic_posting_review_mode, \
                reply_posting_review_mode, nested_replies_default \
         FROM category_settings WHERE category_id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    let daily = int("num_auto_bump_daily").map_or(current.0, |v| v.map(|v| v as i32));
    let cooldown = int("auto_bump_cooldown_days").map_or(current.1, |v| v.map(|v| v as i32));
    let nested_value = match nested {
        Some(Some(b)) => b,
        Some(None) => return Err(Unsupported("nil for nested_replies_default").into()),
        None => current.4,
    };
    if (daily, cooldown, topic_mode, reply_mode, nested_value)
        == (current.0, current.1, current.2, current.3, current.4)
    {
        return Ok(());
    }
    if nested_value != current.4 && !nested_value {
        // after_save: clear_nested_replies_conversion_completed
        sqlx::query(
            "DELETE FROM category_custom_fields WHERE category_id = $1 \
               AND name = 'nested_replies_conversion_completed'",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    sqlx::query(
        "UPDATE category_settings SET num_auto_bump_daily = $2, auto_bump_cooldown_days = $3, \
           topic_posting_review_mode = $4, reply_posting_review_mode = $5, \
           nested_replies_default = $6, updated_at = now() \
         WHERE category_id = $1",
    )
    .bind(id)
    .bind(daily)
    .bind(cooldown)
    .bind(topic_mode)
    .bind(reply_mode)
    .bind(nested_value)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `HasCustomFields::FieldDescriptor#serialize` for a category field.
fn serialize_custom_field(name: &str, v: &Value) -> Option<String> {
    let boolean = crate::category_serializer::is_boolean_field(name);
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "t" } else { "f" }.into()),
        Value::String(s) if boolean => Some(
            if matches!(s.as_str(), "1" | "t" | "true" | "T" | "True" | "TRUE") {
                "t"
            } else {
                "f"
            }
            .into(),
        ),
        Value::String(s) => Some(s.clone()),
        Value::Number(_) if boolean => Some("t".into()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(_) => Some(v.to_string()),
        Value::Array(_) => None,
    }
}

/// `save_custom_fields` after the save: each changed key's rows replaced.
async fn save_custom_fields(
    conn: &mut PgConnection,
    r: &mut Record,
    id: i32,
) -> Result<(), AppError> {
    let Some(fields) = r.custom_fields.take() else {
        return Ok(());
    };
    // custom_fields = data replaced the hash: keys left out are deleted.
    let mut keys: Vec<String> = r.custom_fields_original.keys().cloned().collect();
    for k in fields.keys() {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    for key in keys {
        let rows: Vec<(i32, Option<String>)> = sqlx::query_as(
            "SELECT id, value FROM category_custom_fields WHERE category_id = $1 AND name = $2 ORDER BY id",
        )
        .bind(id)
        .bind(&key)
        .fetch_all(&mut *conn)
        .await?;
        // An array: its serialized items, the rows replaced unless equal.
        if let Some(Value::Array(items)) = fields.get(&key) {
            let wanted: Vec<String> = items
                .iter()
                .filter(|v| !v.is_null())
                .filter_map(|v| match v {
                    Value::Array(_) => Some(v.to_string()),
                    other => serialize_custom_field(&key, other),
                })
                .collect();
            let have: Vec<String> = rows
                .iter()
                .map(|(_, v)| v.clone().unwrap_or_default())
                .collect();
            if wanted != have {
                sqlx::query(
                    "DELETE FROM category_custom_fields WHERE category_id = $1 AND name = $2",
                )
                .bind(id)
                .bind(&key)
                .execute(&mut *conn)
                .await?;
                for value in wanted {
                    sqlx::query(
                        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
                         VALUES ($1, $2, $3, now(), now())",
                    )
                    .bind(id)
                    .bind(&key)
                    .bind(&value)
                    .execute(&mut *conn)
                    .await?;
                }
            }
            continue;
        }
        let value = fields
            .get(&key)
            .and_then(|v| serialize_custom_field(&key, v));
        match value {
            None => {
                sqlx::query(
                    "DELETE FROM category_custom_fields WHERE category_id = $1 AND name = $2",
                )
                .bind(id)
                .bind(&key)
                .execute(&mut *conn)
                .await?;
            }
            Some(value) => {
                let keep = rows
                    .iter()
                    .find(|(_, v)| v.as_deref() == Some(value.as_str()));
                for (row_id, _) in &rows {
                    if keep.is_some_and(|(k, _)| k == row_id) {
                        continue;
                    }
                    sqlx::query("DELETE FROM category_custom_fields WHERE id = $1")
                        .bind(row_id)
                        .execute(&mut *conn)
                        .await?;
                }
                if keep.is_none() {
                    sqlx::query(
                        "INSERT INTO category_custom_fields (category_id, name, value, created_at, updated_at) \
                         VALUES ($1, $2, $3, now(), now())",
                    )
                    .bind(id)
                    .bind(&key)
                    .bind(&value)
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
    }
    Ok(())
}

/// allowed_tags=, allowed_tag_groups= and required_tag_groups=.
async fn save_tags(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &mut Record,
    id: i32,
) -> Result<(), AppError> {
    if let Some(names) = r.allowed_tags.take() {
        // DiscourseTagging.add_or_create_tags_by_name(self, names,
        // unlimited: true), as the system user.
        let system = crate::web_hooks::system_guardian(conn).await?;
        let tag_names =
            crate::tagging::tags_for_saving(conn, cx.settings, &system, &names, true).await?;
        let mut current: Vec<String> = sqlx::query_scalar(
            "SELECT t.name FROM category_tags ct JOIN tags t ON t.id = ct.tag_id WHERE ct.category_id = $1",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        current.sort();
        let mut wanted = tag_names.clone();
        wanted.sort();
        if current != wanted {
            let tagging =
                crate::tagging::add_or_create_tags_by_name(conn, cx.settings, &names).await?;
            sqlx::query("DELETE FROM category_tags WHERE category_id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            for tag in &tagging.tags {
                let Some(tag_id) = tag.id else { continue };
                sqlx::query(
                    "INSERT INTO category_tags (category_id, tag_id, created_at, updated_at) \
                     VALUES ($1, $2, now(), now())",
                )
                .bind(id)
                .bind(tag_id)
                .execute(&mut *conn)
                .await?;
            }
            crate::tagging::enqueue_tag_created(conn, &tagging.created).await?;
        }
    }
    if let Some(names) = r.allowed_tag_groups.take() {
        let ids: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM tag_groups WHERE name = ANY($1) ORDER BY id")
                .bind(&names)
                .fetch_all(&mut *conn)
                .await?;
        sqlx::query(
            "DELETE FROM category_tag_groups WHERE category_id = $1 AND NOT (tag_group_id = ANY($2))",
        )
        .bind(id)
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "INSERT INTO category_tag_groups (category_id, tag_group_id, created_at, updated_at) \
             SELECT $1, g, now(), now() FROM unnest($2::int[]) g \
             WHERE NOT EXISTS (SELECT 1 FROM category_tag_groups WHERE category_id = $1 AND tag_group_id = g)",
        )
        .bind(id)
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
    }
    if let Some(groups) = r.required_tag_groups.take() {
        let names: Vec<String> = groups.iter().map(|(n, _)| n.clone()).collect();
        let found: Vec<(i32, String)> =
            sqlx::query_as("SELECT id, name FROM tag_groups WHERE name = ANY($1) ORDER BY id")
                .bind(&names)
                .fetch_all(&mut *conn)
                .await?;
        // The map by name keeps a name's last entry and its index.
        let mut wanted: Vec<(i32, i64, i64)> = found
            .iter()
            .filter_map(|(gid, name)| {
                let (i, (_, min)) = groups.iter().enumerate().rfind(|(_, (n, _))| n == name)?;
                Some((*gid, *min, i as i64))
            })
            .collect();
        wanted.sort_by_key(|(_, _, order)| *order);
        sqlx::query("DELETE FROM category_required_tag_groups WHERE category_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        for (gid, min, order) in wanted {
            sqlx::query(
                "INSERT INTO category_required_tag_groups \
                   (category_id, tag_group_id, min_count, \"order\", created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, now(), now())",
            )
            .bind(id)
            .bind(gid)
            .bind(min as i32)
            .bind(order as i32)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// form_template_ids=
async fn save_form_templates(
    conn: &mut PgConnection,
    r: &mut Record,
    id: i32,
) -> Result<(), AppError> {
    let Some(ids) = r.form_template_ids.take() else {
        return Ok(());
    };
    let ids: Vec<i32> = ids.into_iter().map(|i| i as i32).collect();
    let found: Vec<i32> = sqlx::query_scalar("SELECT id FROM form_templates WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?;
    if found.len() != {
        let mut u = ids.clone();
        u.sort_unstable();
        u.dedup();
        u.len()
    } {
        return Err(Unsupported("form templates that don't exist (RecordNotFound)").into());
    }
    sqlx::query(
        "DELETE FROM category_form_templates WHERE category_id = $1 AND NOT (form_template_id = ANY($2))",
    )
    .bind(id)
    .bind(&ids)
    .execute(&mut *conn)
    .await?;
    for fid in ids {
        sqlx::query(
            "INSERT INTO category_form_templates (category_id, form_template_id, created_at, updated_at) \
             SELECT $1, $2, now(), now() WHERE NOT EXISTS \
               (SELECT 1 FROM category_form_templates WHERE category_id = $1 AND form_template_id = $2)",
        )
        .bind(id)
        .bind(fid)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Category#delete_category_permalink`: the permalink at the category's
/// path, destroyed.
async fn delete_category_permalink(conn: &mut PgConnection, id: i32) -> Result<(), AppError> {
    let Some(category) = crate::category::Category::find(&mut *conn, id).await? else {
        return Ok(());
    };
    let path = category.slug_path(&mut *conn).await?;
    let url = format!("c/{}", path.join("/"));
    // Permalink.find_by_url normalizes the url as it stores them.
    sqlx::query("DELETE FROM permalinks WHERE url = $1")
        .bind(&url)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `Category.post_template`
fn post_template(i18n: &I18n) -> Result<String, AppError> {
    Ok(i18n
        .t_with(
            "category.post_template",
            &[(
                "replace_paragraph",
                i18n.t("category.replace_paragraph").unwrap_or(""),
            )],
        )
        .ok_or(Unsupported("category.post_template"))?)
}

/// `Category#create_category_definition`: unless the category skips it,
/// a pinned "About the X category" topic by the category's user with the
/// description (or the template) as its first post, saved without
/// validations or the topic's callbacks; the category then points at it
/// and takes the cooked description.
pub async fn create_category_definition(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
) -> Result<(), AppError> {
    let skip: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM category_custom_fields WHERE category_id = $1 \
           AND name = 'skip_category_definition' AND value IN ('1', 't', 'true', 'T', 'True', 'TRUE'))",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if skip {
        return Ok(());
    }
    let (name, user_id, description, slow_mode): (String, i32, Option<String>, Option<i32>) =
        sqlx::query_as(
            "SELECT name, user_id, description, default_slow_mode_seconds FROM categories WHERE id = $1",
        )
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let s = cx.settings;
    let i18n = cx.i18n();
    let title = i18n
        .t_with("category.topic_prefix", &[("category", &name)])
        .ok_or(Unsupported("category.topic_prefix"))?;
    let slug = crate::posting::text::slug_for(&title, &s.get("default_locale")?.to_s())?;
    let fancy_title = crate::posting::text::fancy_title(
        &title,
        crate::posting::text::EmojiEscape::from_settings(s)?,
    );
    // pinned_at: Time.now before the save; bumped_at, then the row.
    let pinned_at: NaiveDateTime = sqlx::query_scalar("SELECT clock_timestamp()::timestamp")
        .fetch_one(&mut *conn)
        .await?;
    let topic_id: i32 = sqlx::query_scalar(
        "INSERT INTO topics (title, fancy_title, slug, user_id, last_post_user_id, category_id, \
                             pinned_at, bumped_at, created_at, updated_at, slow_mode_seconds) \
         VALUES ($1, $2, $3, $4, $4, $5, $6, clock_timestamp(), clock_timestamp(), clock_timestamp(), \
                 COALESCE($7, 0)) RETURNING id",
    )
    .bind(&title)
    .bind(&fancy_title)
    .bind(&slug)
    .bind(user_id)
    .bind(id)
    .bind(pinned_at)
    .bind(slow_mode)
    .fetch_one(&mut *conn)
    .await?;
    sqlx::query("UPDATE categories SET topic_id = $2 WHERE id = $1")
        .bind(id)
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    // The first post: before_create_tasks (word count, the topic's
    // numbers, cooked), then the row.
    let raw = match &description {
        Some(d) => d.clone(),
        None => post_template(i18n)?,
    };
    let cooked = crate::pretty_text::cook(
        cx.host,
        &raw,
        &crate::pretty_text::MarkdownOptions {
            topic_id: Some(i64::from(topic_id)),
            post_id: None,
            user_id: Some(i64::from(user_id)),
            force_quote_link: false,
            omit_nofollow: false,
            chat: false,
        },
    )
    .await?;
    let words = crate::posting::text::word_count(&raw);
    sqlx::query(
        "UPDATE topics SET highest_staff_post_number = 1, highest_post_number = 1, \
                           posts_count = posts_count + 1 WHERE id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    let post_id: i32 = sqlx::query_scalar(
        "INSERT INTO posts (user_id, topic_id, post_number, raw, cooked, created_at, updated_at, \
                            last_editor_id, word_count, sort_order, last_version_at, baked_at, \
                            baked_version) \
         VALUES ($1, $2, 1, $3, $4, clock_timestamp(), clock_timestamp(), $1, $5, 1, \
                 clock_timestamp(), clock_timestamp(), $6) RETURNING id",
    )
    .bind(user_id)
    .bind(topic_id)
    .bind(&raw)
    .bind(&cooked)
    .bind(words)
    .bind(crate::posting::BAKED_VERSION)
    .fetch_one(&mut *conn)
    .await?;
    if description.as_deref().is_some_and(|d| !d.is_empty()) {
        sqlx::query("UPDATE categories SET description = $2 WHERE id = $1")
            .bind(id)
            .bind(&cooked)
            .execute(&mut *conn)
            .await?;
    }
    // after_commit index_search: the post's and the topic's rows.
    let urls = crate::url::Urls {
        config: &cx.host.config,
        settings: s,
    };
    let base_path = cx.host.config.globals.relative_url_root().to_string();
    let base = crate::posting::search_index::BaseUrls {
        base_path: &base_path,
        base_url_no_prefix: urls.base_url_no_prefix()?,
        store: crate::file_store::FileStore::for_site(urls.config, s)?,
    };
    crate::posting::search_index::index_post(
        &mut *conn,
        s,
        &base,
        &crate::posting::search_index::PostIndex {
            post_id,
            topic_id,
            is_first_post: true,
            topic_title: &title,
            category_name: Some(&name),
            tag_names: None,
            cooked: &cooked,
            private_message: false,
        },
    )
    .await?;
    Ok(())
}

/// `Positionable#move_to`
pub async fn move_to(conn: &mut PgConnection, id: i32, position: i64) -> Result<(), AppError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM categories")
        .fetch_one(&mut *conn)
        .await?;
    let position = position.max(0).min(count - 1);
    let current: Option<i32> = sqlx::query_scalar("SELECT position FROM categories WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    match current {
        // No position yet: the shifts compare against NULL and match none.
        None => {}
        Some(c) if position > i64::from(c) => {
            sqlx::query(
                "UPDATE categories SET position = position - 1 \
                 WHERE position > $1 AND position <= $2",
            )
            .bind(c)
            .bind(position as i32)
            .execute(&mut *conn)
            .await?;
        }
        Some(c) if position < i64::from(c) => {
            sqlx::query(
                "UPDATE categories SET position = position + 1 \
                 WHERE position >= $2 AND position < $1",
            )
            .bind(c)
            .bind(position as i32)
            .execute(&mut *conn)
            .await?;
        }
        Some(_) => return Ok(()),
    }
    sqlx::query("UPDATE categories SET position = $2 WHERE id = $1")
        .bind(id)
        .bind(position as i32)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `Category#url`
pub async fn url(conn: &mut PgConnection, base_path: &str, id: i32) -> Result<String, AppError> {
    let category = crate::category::Category::find(&mut *conn, id)
        .await?
        .ok_or(Unsupported("a category that vanished"))?;
    Ok(category.url(&mut *conn, base_path).await?)
}

/// The category's created_at as Ruby's Time#to_s in UTC.
async fn created_at_to_s(conn: &mut PgConnection, id: i32) -> Result<String, AppError> {
    let at: NaiveDateTime = sqlx::query_scalar("SELECT created_at FROM categories WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(at.format("%Y-%m-%d %H:%M:%S UTC").to_string())
}

/// `StaffActionLogger#log_category_creation`
pub async fn log_creation(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    acting_user_id: i32,
    id: i32,
) -> Result<(), AppError> {
    let name: String = sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
    let details = format!(
        "created_at: {}\nname: {name}",
        created_at_to_s(conn, id).await?
    );
    let context = url(conn, cx.host.config.globals.relative_url_root(), id).await?;
    insert_history(
        conn,
        "create_category",
        acting_user_id,
        id,
        &details,
        &context,
    )
    .await
}

async fn insert_history(
    conn: &mut PgConnection,
    action: &str,
    acting_user_id: i32,
    category_id: i32,
    details: &str,
    context: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, category_id, details, context, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(
        crate::staff_action_logs::action_id(action)
            .ok_or(Unsupported("a staff action missing from the enums"))?,
    )
    .bind(acting_user_id)
    .bind(category_id)
    .bind(details)
    .bind(context)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The after_commit callbacks of a save: the category_created or
/// category_updated event (its web hooks; the automations on it are
/// refused) and the search index job. `force` is whether the save changed
/// the name (a reloaded category has no saved changes); `setting_loaded`
/// whether the category_setting association is still loaded (not after
/// a reload). True when a web hook payload was serialized, which leaves
/// the category's `has_children?` memoized.
pub async fn after_commit(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
    event: &str,
    saved: Option<&[(String, Value, Value)]>,
    setting_loaded: bool,
) -> Result<bool, AppError> {
    let saved = saved.unwrap_or(&[]);
    let changed = |key: &str| saved.iter().any(|(k, _, _)| k == key);
    let update = event == "category_updated";
    // enqueue_upload_security_updates (on update)
    if update && changed("read_restricted") {
        crate::jobs::enqueue(
            &mut *conn,
            "update_category_upload_security",
            json!({ "category_id": id }),
        )
        .await?;
    }
    let automations: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_automation_automations \
         WHERE enabled AND trigger = 'category_created_edited')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if automations {
        return Err(Unsupported("automations on category_created_edited").into());
    }
    let hooked = crate::web_hooks::active(&mut *conn, event).await?;
    if hooked {
        let payload = web_hook_payload(conn, cx, id, setting_loaded).await?;
        let mut opts = Map::new();
        opts.insert("id".into(), json!(id));
        opts.insert("payload".into(), json!(payload));
        crate::web_hooks::enqueue_hooks(&mut *conn, "category", event, opts).await?;
    }
    if update {
        category_updated_plugins(conn, cx, id, changed("name") || changed("description")).await?;
    }
    crate::jobs::enqueue(
        &mut *conn,
        "index_category_for_search",
        json!({ "category_id": id, "force": changed("name") }),
    )
    .await?;
    Ok(hooked)
}

/// The plugins' `:category_updated` listeners: chat's auto join and
/// leave for the category's channel; discourse-ai's translations of a
/// renamed or redescribed category are refused.
pub async fn category_updated_plugins(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
    name_or_description_changed: bool,
) -> Result<(), AppError> {
    let s = cx.settings;
    let on = |name: &str| -> Result<bool, AppError> {
        Ok(s.get(name).map(|v| v.truthy()).unwrap_or(false))
    };
    if name_or_description_changed && on("discourse_ai_enabled")? && on("ai_translation_enabled")? {
        return Err(Unsupported(
            "translating a changed category (discourse-ai localize_categories)",
        )
        .into());
    }
    if !crate::plugins::chat::enabled(s)? {
        return Ok(());
    }
    let channel: Option<(i64, bool)> = sqlx::query_as(
        "SELECT id::bigint, auto_join_users FROM chat_channels \
         WHERE chatable_type = 'Category' AND chatable_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((channel_id, auto_join)) = channel else {
        return Ok(());
    };
    if auto_join {
        crate::plugins::chat::auto_join::create_memberships(
            &mut *conn,
            cx.bus,
            &cx.host.i18n,
            &cx.host.config,
            s,
            None,
            Some(channel_id),
        )
        .await?;
    }
    crate::plugins::chat::scheduled::auto_leave_category(&mut *conn, s, "category_updated", id)
        .await?;
    Ok(())
}

/// WebHookCategorySerializer as the system user sees it: CategorySerializer
/// less can_edit, notification_level and available_groups.
pub async fn web_hook_payload(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
    setting_loaded: bool,
) -> Result<String, AppError> {
    let system = crate::web_hooks::system_guardian(conn).await?;
    let mut json = crate::category_serializer::serialize(
        conn,
        &cx.ser(),
        cx.settings,
        &system,
        id,
        &crate::category_serializer::Preloaded {
            category_setting: setting_loaded,
            ..Default::default()
        },
    )
    .await?;
    if let Value::Object(m) = &mut json {
        m.remove("can_edit");
        m.remove("notification_level");
        m.remove("available_groups");
    }
    Ok(crate::web_hooks::to_json(&json))
}

/// `StaffActionLogger#log_custom` with the category as a base attribute
/// (a column) and the rest as details.
pub async fn log_custom(
    conn: &mut PgConnection,
    acting_user_id: i32,
    custom_type: &str,
    category_id: i32,
    details: &[(&str, String)],
) -> Result<(), AppError> {
    let details = details
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, custom_type, category_id, details, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(
        crate::staff_action_logs::action_id("custom_staff")
            .ok_or(Unsupported("a staff action missing from the enums"))?,
    )
    .bind(acting_user_id)
    .bind(custom_type)
    .bind(category_id)
    .bind(details)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `blank?` for a param value.
pub fn blank_param(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        Value::Bool(b) => !b,
        Value::Number(_) => false,
    }
}

/// `revise_category_definition`'s guard: a live definition topic with a
/// first post.
async fn definition_has_first_post(conn: &mut PgConnection, id: i32) -> Result<bool, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories c JOIN topics t ON t.id = c.topic_id AND t.deleted_at IS NULL \
         JOIN posts p ON p.topic_id = t.id AND p.post_number = 1 AND p.deleted_at IS NULL WHERE c.id = $1)",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?)
}

fn change<'a>(changes: &'a [(String, Value, Value)], key: &str) -> Option<(&'a Value, &'a Value)> {
    changes
        .iter()
        .find(|(k, _, _)| k == key)
        .map(|(_, before, after)| (before, after))
}

/// The after_update callbacks: the definition's title follows a rename,
/// an old slug keeps a permalink, and a slug or parent change remaps
/// hashtags.
async fn after_update(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
    changes: &[(String, Value, Value)],
    deferred: &mut Vec<(&'static str, Value)>,
) -> Result<(), AppError> {
    if let Some((old, new)) = change(changes, "name") {
        rename_category_definition(
            conn,
            cx,
            id,
            old.as_str().unwrap_or(""),
            new.as_str().unwrap_or(""),
        )
        .await?;
    }
    if let Some((old, _)) = change(changes, "slug") {
        create_category_permalink(conn, id, old.as_str().unwrap_or("")).await?;
    }
    let slug_change = change(changes, "slug");
    let parent_change = change(changes, "parent_category_id");
    if (slug_change.is_some() || parent_change.is_some())
        && let Some(job) = category_hashtag_remap(conn, id, slug_change, parent_change).await?
    {
        deferred.push(("remap_hashtag", job));
    }
    Ok(())
}

/// `rename_category_definition`: a definition still titled for the old
/// name takes the new one (`update_attribute`: the slug and fancy title,
/// then its search rows).
async fn rename_category_definition(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
    old_name: &str,
    new_name: &str,
) -> Result<(), AppError> {
    let topic: Option<(i32, String)> = sqlx::query_as(
        "SELECT t.id, t.title FROM categories c JOIN topics t ON t.id = c.topic_id AND t.deleted_at IS NULL \
         WHERE c.id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((topic_id, title)) = topic else {
        return Ok(());
    };
    let i18n = cx.i18n();
    let old_title = i18n
        .t_with("category.topic_prefix", &[("category", old_name)])
        .ok_or(Unsupported("category.topic_prefix"))?;
    if title != old_title {
        return Ok(());
    }
    let new_title = i18n
        .t_with("category.topic_prefix", &[("category", new_name)])
        .ok_or(Unsupported("category.topic_prefix"))?;
    let s = cx.settings;
    let slug = crate::posting::text::slug_for(&new_title, &s.get("default_locale")?.to_s())?;
    let fancy_title = crate::posting::text::fancy_title(
        &new_title,
        crate::posting::text::EmojiEscape::from_settings(s)?,
    );
    sqlx::query(
        "UPDATE topics SET title = $2, slug = $3, fancy_title = $4, updated_at = now() WHERE id = $1",
    )
    .bind(topic_id)
    .bind(&new_title)
    .bind(&slug)
    .bind(&fancy_title)
    .execute(&mut *conn)
    .await?;
    // after_save: SearchIndexer.queue_post_reindex, then the topic's
    // first post and its row.
    sqlx::query(
        "UPDATE post_search_data SET version = 0 FROM posts \
         WHERE post_search_data.post_id = posts.id AND posts.topic_id = $1",
    )
    .bind(topic_id)
    .execute(&mut *conn)
    .await?;
    let first_post: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(post_id) = first_post {
        let urls = crate::url::Urls {
            config: &cx.host.config,
            settings: s,
        };
        let base_path = cx.host.config.globals.relative_url_root().to_string();
        crate::posting::search_index::reindex_post(
            &mut *conn,
            s,
            &crate::posting::search_index::BaseUrls {
                base_path: &base_path,
                base_url_no_prefix: urls.base_url_no_prefix()?,
                store: crate::file_store::FileStore::for_site(urls.config, s)?,
            },
            post_id,
        )
        .await?;
    }
    Ok(())
}

/// `create_category_permalink`: the old slug under the current parent's
/// path, pointed at the category (an existing permalink re-pointed).
async fn create_category_permalink(
    conn: &mut PgConnection,
    id: i32,
    old_slug: &str,
) -> Result<(), AppError> {
    let parent: Option<i32> =
        sqlx::query_scalar("SELECT parent_category_id FROM categories WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let mut url = String::from("c");
    if let Some(parent) = parent
        && let Some(p) = crate::category::Category::find(&mut *conn, parent).await?
    {
        url.push('/');
        url.push_str(&p.slug_path(&mut *conn).await?.join("/"));
    }
    url.push_str(&format!("/{old_slug}/{id}"));
    let updated = sqlx::query("UPDATE permalinks SET category_id = $2 WHERE url = $1")
        .bind(&url)
        .bind(id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    if updated == 0 {
        sqlx::query(
            "INSERT INTO permalinks (url, category_id, created_at, updated_at) VALUES ($1, $2, now(), now())",
        )
        .bind(&url)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Category.hashtag_ref_from`
async fn hashtag_ref_from(
    conn: &mut PgConnection,
    slug: &str,
    parent: Option<i64>,
) -> Result<String, AppError> {
    let parent_slug: Option<String> = match parent {
        Some(p) => {
            sqlx::query_scalar("SELECT slug FROM categories WHERE id = $1")
                .bind(p as i32)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    Ok(match parent_slug {
        Some(p) => format!("{p}:{slug}"),
        None => slug.to_string(),
    })
}

/// `enqueue_category_hashtag_remap` through HashtagRemapper.enqueue: the
/// remaps whose old ref no longer is the current one, as a remap_hashtag
/// job.
async fn category_hashtag_remap(
    conn: &mut PgConnection,
    id: i32,
    slug_change: Option<(&Value, &Value)>,
    parent_change: Option<(&Value, &Value)>,
) -> Result<Option<Value>, AppError> {
    let (slug, parent): (String, Option<i32>) =
        sqlx::query_as("SELECT slug, parent_category_id FROM categories WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let old_slug = slug_change
        .and_then(|(old, _)| old.as_str().map(str::to_string))
        .unwrap_or_else(|| slug.clone());
    let old_parent = match parent_change {
        Some((old, _)) => old.as_i64(),
        None => parent.map(i64::from),
    };
    let mut remaps = vec![(id, hashtag_ref_from(conn, &old_slug, old_parent).await?)];
    if slug_change.is_some() {
        let subs: Vec<(i32, String)> = sqlx::query_as(
            "SELECT id, slug FROM categories WHERE parent_category_id = $1 ORDER BY id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        for (sub_id, sub_slug) in subs {
            remaps.push((sub_id, format!("{old_slug}:{sub_slug}")));
        }
    }
    let mut usable = Vec::new();
    for (rid, old_ref) in remaps {
        // CategoryHashtagDataSource.ref_for: slug_ref(depth: 1).
        let current: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT c.slug, p.slug FROM categories c LEFT JOIN categories p ON p.id = c.parent_category_id \
             WHERE c.id = $1",
        )
        .bind(rid)
        .fetch_optional(&mut *conn)
        .await?;
        let current = current.map(|(slug, parent)| match parent {
            Some(p) => format!("{p}:{slug}"),
            None => slug,
        });
        let unchanged = current
            .as_deref()
            .filter(|c| !c.is_empty())
            .is_some_and(|c| c.to_lowercase() == old_ref.to_lowercase());
        if !unchanged {
            usable.push(json!({ "type": "category", "id": rid, "old_ref": old_ref }));
        }
    }
    if usable.is_empty() {
        return Ok(None);
    }
    Ok(Some(json!({ "remaps": usable })))
}

/// `Category#permissions_params`: each category group's group name and
/// permission type; `{everyone: 1}` for none.
pub async fn permissions_params(
    conn: &mut PgConnection,
    id: i32,
) -> Result<Vec<(String, i64)>, AppError> {
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT g.name, cg.permission_type FROM category_groups cg JOIN groups g ON g.id = cg.group_id \
         WHERE cg.category_id = $1 ORDER BY cg.id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut out: Vec<(String, i64)> = Vec::new();
    for (name, level) in rows {
        // A hash: a later row for the same name replaces the value in place.
        match out.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = i64::from(level),
            None => out.push((name, i64::from(level))),
        }
    }
    Ok(out)
}

/// A hash of names to levels as `to_json` writes it.
fn permissions_json(perms: &[(String, i64)]) -> String {
    let mut m = Map::new();
    for (k, v) in perms {
        m.insert(k.clone(), json!(v));
    }
    crate::web_hooks::to_json(&Value::Object(m))
}

/// A value as a UserHistory text column stores it (ActiveModel's string
/// cast: booleans as t and f, floats as Ruby prints them).
fn history_value(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(b) => Some(if *b { "t" } else { "f" }.into()),
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(match n.as_i64() {
            Some(i) => i.to_string(),
            None => crate::ruby::float_to_s(n.as_f64().unwrap_or(0.0)),
        }),
        other => Some(other.to_string()),
    }
}

/// `StaffActionLogger#log_category_settings_change`: a row per changed
/// attribute among the params' keys, the permissions when they differ
/// from the old ones, and each custom field given that changed.
#[allow(clippy::too_many_arguments)]
pub async fn log_settings_change(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    acting_user_id: i32,
    id: i32,
    params: &Params,
    saved_changes: &[(String, Value, Value)],
    old_permissions: &[(String, i64)],
    old_custom_fields: &Map<String, Value>,
) -> Result<(), AppError> {
    let mut rows: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    for key in params.keys() {
        if let Some((before, after)) = change(saved_changes, key) {
            rows.push((key.to_string(), history_value(before), history_value(after)));
        }
    }
    let given = params.permissions_given_json();
    let same = |a: &[(String, i64)], b: &[(String, i64)]| {
        let mut a = a.to_vec();
        let mut b = b.to_vec();
        a.sort();
        b.sort();
        a == b
    };
    if !old_permissions.is_empty() && !given.as_deref().is_some_and(|g| same(g, old_permissions)) {
        rows.push((
            "permissions".into(),
            Some(permissions_json(old_permissions)),
            Some(match &given {
                Some(p) => permissions_json(p),
                None => "null".into(),
            }),
        ));
    }
    if let Some(fields) = &params.custom_fields {
        for (key, value) in fields {
            let old = old_custom_fields.get(key).cloned().unwrap_or(Value::Null);
            if old == *value {
                continue;
            }
            rows.push((
                format!("custom_fields[{key}]"),
                history_value(&old),
                history_value(value),
            ));
        }
    }
    let context = url(conn, cx.host.config.globals.relative_url_root(), id).await?;
    let action = crate::staff_action_logs::action_id("change_category_settings")
        .ok_or(Unsupported("a staff action missing from the enums"))?;
    for (subject, previous, new) in rows {
        sqlx::query(
            "INSERT INTO user_histories (action, acting_user_id, category_id, context, subject, \
                                         previous_value, new_value, admin_only, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, TRUE, clock_timestamp(), clock_timestamp())",
        )
        .bind(action)
        .bind(acting_user_id)
        .bind(id)
        .bind(&context)
        .bind(&subject)
        .bind(previous)
        .bind(new)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

impl Params {
    /// `category_params[:permissions].to_h` with each name kept once.
    fn permissions_given_json(&self) -> Option<Vec<(String, i64)>> {
        let perms = self.permissions.as_ref()?;
        let mut out: Vec<(String, i64)> = Vec::new();
        for (k, v) in perms {
            match out.iter_mut().find(|(n, _)| n == k) {
                Some(e) => e.1 = *v,
                None => out.push((k.clone(), *v)),
            }
        }
        Some(out)
    }
}

/// `merge_pending_custom_fields!`: blank values delete the key, booleans
/// become strings, the rest is kept as given.
pub fn merge_pending_custom_fields(
    current: &Map<String, Value>,
    pending: &Map<String, Value>,
) -> Map<String, Value> {
    let mut out = current.clone();
    for (key, value) in pending {
        match value {
            Value::Null => {
                out.remove(key);
            }
            Value::String(s) if s.is_empty() => {
                out.remove(key);
            }
            Value::Bool(b) => {
                out.insert(key.clone(), json!(b.to_string()));
            }
            other => {
                out.insert(key.clone(), other.clone());
            }
        }
    }
    out
}

impl Record {
    /// The custom fields as loaded (typed).
    pub fn custom_fields_loaded(&self) -> &Map<String, Value> {
        &self.custom_fields_original
    }

    /// Sets the custom fields to save, when they differ from the loaded
    /// ones (custom_fields_clean? otherwise).
    pub fn set_custom_fields(&mut self, fields: Map<String, Value>) {
        if fields != self.custom_fields_original {
            self.custom_fields = Some(fields);
        }
    }
}

impl Record {
    /// The jobs a save left for DB.after_commit, enqueued after the
    /// after_commit callbacks.
    pub async fn run_deferred(&mut self, conn: &mut PgConnection) -> Result<(), AppError> {
        for (job, args) in std::mem::take(&mut self.deferred) {
            crate::jobs::enqueue(&mut *conn, job, args).await?;
        }
        Ok(())
    }
}

impl Record {
    /// Assigns a column directly (as the model's writer would).
    pub fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_string(), value);
    }
}

/// `Category#destroy`: the category_destroyed web hook's payload taken
/// first (HasDestroyedWebHook), the `dependent:` associations in their
/// declaration order, the row, then the after_destroy callbacks (the
/// definition trashed, the site settings naming it cleared, the
/// deletion published) and the hooks enqueued. The category's chat
/// channel (Chat::Channel#destroy) is refused.
pub async fn destroy(conn: &mut PgConnection, cx: &Cx<'_>, id: i32) -> Result<(), AppError> {
    let channel: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE chatable_type = 'Category' AND chatable_id = $1)",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if channel {
        return Err(Unsupported("destroying a category's chat channel").into());
    }
    let embeddable: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM embeddable_hosts WHERE category_id = $1)")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    if embeddable {
        return Err(Unsupported("destroying a category's embeddable hosts").into());
    }
    // Searchable's and HasCustomFields' dependents (included before
    // HasDestroyedWebHook's around_destroy).
    for sql in [
        "DELETE FROM category_search_data WHERE category_id = $1",
        "DELETE FROM category_custom_fields WHERE category_id = $1",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *conn).await?;
    }
    let payload = if crate::web_hooks::active(&mut *conn, "category_destroyed").await? {
        // ensure_can_delete! memoized has_children? on the category.
        let system = crate::web_hooks::system_guardian(conn).await?;
        let pre = crate::category_serializer::Preloaded {
            has_children: Some(crate::category_serializer::has_children(&mut *conn, id).await?),
            ..Default::default()
        };
        let mut json =
            crate::category_serializer::serialize(conn, &cx.ser(), cx.settings, &system, id, &pre)
                .await?;
        if let Value::Object(m) = &mut json {
            m.remove("can_edit");
            m.remove("notification_level");
            m.remove("available_groups");
        }
        Some(crate::web_hooks::to_json(&json))
    } else {
        None
    };
    let topic: Option<(i32, bool, bool)> = sqlx::query_as(
        "SELECT t.id, t.closed, t.archived FROM categories c \
         JOIN topics t ON t.id = c.topic_id AND t.deleted_at IS NULL WHERE c.id = $1",
    )
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?;
    // The model's own dependent associations in declaration order, then
    // the plugins'.
    for sql in [
        "DELETE FROM category_localizations WHERE category_id = $1",
        "DELETE FROM category_groups WHERE category_id = $1",
        "DELETE FROM category_moderation_groups WHERE category_id = $1",
        "DELETE FROM category_posting_review_groups WHERE category_id = $1",
        "DELETE FROM topic_timers WHERE category_id = $1",
        "DELETE FROM upload_references WHERE target_type = 'Category' AND target_id = $1",
        "DELETE FROM category_settings WHERE category_id = $1",
        "DELETE FROM categories_web_hooks WHERE category_id = $1",
        "DELETE FROM category_tags WHERE category_id = $1",
        "DELETE FROM category_tag_groups WHERE category_id = $1",
        "DELETE FROM category_required_tag_groups WHERE category_id = $1",
        "DELETE FROM sidebar_section_links WHERE linkable_type = 'Category' AND linkable_id = $1",
        "DELETE FROM category_form_templates WHERE category_id = $1",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *conn).await?;
    }
    // topic voting's category setting: before_destroy archive_votes.
    let voting: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_voting_category_settings WHERE category_id = $1)",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if voting {
        sqlx::query(
            "UPDATE topic_voting_votes SET archive = true FROM topics \
             WHERE topics.category_id = $1 AND topic_voting_votes.topic_id = topics.id",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM topic_voting_category_settings WHERE category_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("DELETE FROM categories WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    if let Some(payload) = payload {
        let mut opts = Map::new();
        opts.insert("id".into(), json!(id));
        opts.insert("payload".into(), json!(payload));
        crate::web_hooks::enqueue_hooks(&mut *conn, "category", "category_destroyed", opts).await?;
    }
    // after_destroy: trash_category_definition (Topic#trash!: its
    // category is gone, so no count moves).
    if let Some((topic_id, closed, archived)) = topic {
        let tagged: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM topic_tags WHERE topic_id = $1)")
                .bind(topic_id)
                .fetch_one(&mut *conn)
                .await?;
        let embed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topic_embeds WHERE topic_id = $1 AND deleted_at IS NULL)",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if tagged || embed {
            return Err(Unsupported("trashing a tagged or embedded category definition").into());
        }
        if cx.settings.get("boards_enabled").is_ok_and(|v| v.truthy()) {
            return Err(Unsupported("the boards plugin's topic_trashed sync").into());
        }
        sqlx::query("UPDATE topics SET deleted_at = now(), deleted_by_id = NULL WHERE id = $1")
            .bind(topic_id)
            .execute(&mut *conn)
            .await?;
        if crate::plugins::topic_voting::enabled(cx.settings)? {
            crate::plugins::topic_voting::lifecycle::topic_trashed(
                &mut *conn, topic_id, closed, archived,
            )
            .await?;
        }
    }
    // clear_related_site_settings, then remove_site_settings.
    if cx.settings.get("general_category_id")?.to_i() == i64::from(id) {
        set_setting(conn, cx, "general_category_id", json!(-1)).await?;
    }
    remove_site_settings(conn, cx, id).await?;
    cx.bus
        .publish(
            &mut *conn,
            "/categories",
            &json!({ "deleted_categories": [id] }),
            None,
        )
        .await?;
    crate::bus::clear_anon_cache(cx.bus, &mut *conn).await?;
    Ok(())
}

/// `SiteSetting.set(name, value)`
async fn set_setting(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    name: &str,
    value: Value,
) -> Result<(), AppError> {
    match crate::site_setting_update::set(
        &mut *conn,
        cx.bus,
        cx.settings,
        &cx.host.site_setting_defs,
        &cx.host.i18n,
        name,
        &value,
    )
    .await?
    {
        Ok(_) => Ok(()),
        Err(_) => Err(Unsupported("a site setting a category deletion cannot clear").into()),
    }
}

/// `remove_site_settings`: `SiteSetting.all_settings` (measuring the
/// upload settings' images on the way), then each category setting
/// naming the category set to blank.
async fn remove_site_settings(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    id: i32,
) -> Result<(), AppError> {
    let urls = crate::url::Urls {
        config: &cx.host.config,
        settings: cx.settings,
    };
    let acx = crate::admin_site_settings::Context {
        defs: &cx.host.site_setting_defs,
        settings: cx.settings,
        i18n: &cx.host.i18n,
        globals: &cx.host.config.globals,
        base_path: cx.host.config.globals.relative_url_root(),
        urls: &urls,
    };
    crate::admin_site_settings::fix_upload_dimensions(&mut *conn, &acx, false).await?;
    let mut names = Vec::new();
    for def in cx.host.site_setting_defs.iter() {
        if !matches!(def.data_type, crate::site_settings::DataType::Category)
            || crate::admin_site_settings::is_hidden(&acx, def)?
        {
            continue;
        }
        if crate::ruby::to_i(&cx.settings.get(&def.name)?.to_s()) == i64::from(id) {
            names.push(def.name.clone());
        }
    }
    for name in names {
        set_setting(conn, cx, &name, json!("")).await?;
    }
    Ok(())
}

/// `StaffActionLogger#log_category_deletion` from what was read before
/// the category went: its creation time, name, permissions (always
/// empty, the groups destroyed with it), parent and url.
pub struct Deleted {
    pub details: String,
    pub context: String,
}

impl Deleted {
    pub async fn read(conn: &mut PgConnection, cx: &Cx<'_>, id: i32) -> Result<Deleted, AppError> {
        let (name, parent): (String, Option<i32>) =
            sqlx::query_as("SELECT name, parent_category_id FROM categories WHERE id = $1")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
        let mut details = format!(
            "created_at: {}\nname: {name}\npermissions: {{}}",
            created_at_to_s(conn, id).await?
        );
        if let Some(parent) = parent {
            let parent_name: Option<String> =
                sqlx::query_scalar("SELECT name FROM categories WHERE id = $1")
                    .bind(parent)
                    .fetch_optional(&mut *conn)
                    .await?;
            if let Some(p) = parent_name {
                details.push_str(&format!("\nparent_category: {p}"));
            }
        }
        let context = url(conn, cx.host.config.globals.relative_url_root(), id).await?;
        Ok(Deleted { details, context })
    }

    pub async fn log(
        &self,
        conn: &mut PgConnection,
        acting_user_id: i32,
        id: i32,
    ) -> Result<(), AppError> {
        insert_history(
            conn,
            "delete_category",
            acting_user_id,
            id,
            &self.details,
            &self.context,
        )
        .await
    }
}

/// `category.custom_fields[k] = v` for each field, then
/// `save_custom_fields` (Categories::Types::Base#configure_custom_fields).
pub async fn write_custom_fields(
    conn: &mut PgConnection,
    id: i32,
    fields: &Map<String, Value>,
) -> Result<(), AppError> {
    let mut record = Record::load(&mut *conn, id)
        .await?
        .ok_or(Unsupported("a category that vanished while configuring it"))?;
    let mut merged = record.custom_fields_loaded().clone();
    for (k, v) in fields {
        merged.insert(k.clone(), v.clone());
    }
    record.set_custom_fields(merged);
    save_custom_fields(conn, &mut record, id).await
}
