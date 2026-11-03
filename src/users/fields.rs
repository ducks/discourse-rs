//! UserField: the admin-defined profile fields, stored per user as
//! `user_custom_fields` rows named `user_field_<id>`, with
//! Admin::UserFieldsController's writes and UserFieldSerializer.
//!
//! `field_type` is the legacy varchar Rails ignores (rows it creates leave
//! it NULL); the type is `field_type_enum`.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// `HasCustomFields::Helpers::CUSTOM_FIELD_TRUE`
pub const CUSTOM_FIELD_TRUE: &[&str] = &["1", "t", "true", "T", "True", "TRUE"];

/// `UserField.field_type_enums`, by value.
pub const FIELD_TYPES: [&str; 6] = [
    "text",
    "confirm",
    "dropdown",
    "multiselect",
    "textarea",
    "date",
];
pub const CONFIRM: i32 = 1;
pub const DROPDOWN: i32 = 2;
pub const MULTISELECT: i32 = 3;

/// `UserField.requirements`, by value.
pub const REQUIREMENTS: [&str; 3] = ["optional", "for_all_users", "on_signup"];
pub const FOR_ALL_USERS: i32 = 1;
pub const ON_SIGNUP: i32 = 2;

/// `UserField.max_length`
pub const MAX_LENGTH: usize = 2048;

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct UserField {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub field_type_enum: i32,
    pub editable: bool,
    pub requirement: i32,
    pub show_on_profile: bool,
    pub show_on_user_card: bool,
    pub show_on_signup: bool,
    pub searchable: bool,
    pub position: Option<i32>,
}

impl UserField {
    pub fn field_type(&self) -> &'static str {
        FIELD_TYPES
            .get(self.field_type_enum as usize)
            .copied()
            .unwrap_or("text")
    }

    /// `required?`: anything but optional.
    pub fn required(&self) -> bool {
        self.requirement != 0
    }
}

const COLUMNS: &str = "id, name, description, field_type_enum, editable, requirement, \
    show_on_profile, show_on_user_card, show_on_signup, searchable, position";

/// `UserField.order(:position)` (no tie-breaker, as Rails has none).
pub async fn all(conn: &mut PgConnection) -> Result<Vec<UserField>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM user_fields ORDER BY position"
    ))
    .fetch_all(conn)
    .await
}

/// `UserField.all` in the scan's order (no ORDER BY).
pub async fn unordered(conn: &mut PgConnection) -> Result<Vec<UserField>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM user_fields"))
        .fetch_all(conn)
        .await
}

pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<UserField>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM user_fields WHERE id = $1"))
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// `user_field_options.pluck(:value)`
pub async fn options(conn: &mut PgConnection, field_id: i32) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM user_field_options WHERE user_field_id = $1 ORDER BY id")
        .bind(field_id)
        .fetch_all(conn)
        .await
}

/// UserFieldSerializer
pub async fn serialize(conn: &mut PgConnection, f: &UserField) -> Result<Value, sqlx::Error> {
    let mut out = Map::new();
    out.insert("id".into(), json!(f.id));
    out.insert("name".into(), json!(f.name));
    out.insert("description".into(), json!(f.description));
    out.insert("field_type".into(), json!(f.field_type()));
    out.insert("editable".into(), json!(f.editable));
    out.insert("required".into(), json!(f.required()));
    out.insert(
        "requirement".into(),
        json!(REQUIREMENTS.get(f.requirement as usize).copied()),
    );
    out.insert("show_on_profile".into(), json!(f.show_on_profile));
    out.insert("show_on_user_card".into(), json!(f.show_on_user_card));
    out.insert("show_on_signup".into(), json!(f.show_on_signup));
    out.insert("searchable".into(), json!(f.searchable));
    out.insert("position".into(), json!(f.position));
    let options = options(conn, f.id).await?;
    if !options.is_empty() {
        out.insert("options".into(), json!(options));
    }
    Ok(Value::Object(out))
}

/// `user.custom_fields["user_field_<id>"]`: no row is nil, one row its
/// value, several (a multiselect) an array in id order, the way
/// `FieldDescriptor#append_field` folds untyped fields.
pub async fn custom_field_value(
    conn: &mut PgConnection,
    user_id: i32,
    field_id: i32,
) -> Result<Value, sqlx::Error> {
    let values: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id",
    )
    .bind(user_id)
    .bind(format!("user_field_{field_id}"))
    .fetch_all(conn)
    .await?;
    Ok(match values.len() {
        0 => Value::Null,
        1 => json!(values[0]),
        _ => json!(values),
    })
}

/// Ruby's `present?` on a custom field value.
pub fn present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !crate::ruby::is_blank(s),
        Value::Array(a) => !a.is_empty(),
        Value::Bool(b) => *b,
        _ => true,
    }
}

/// `HasCustomFields#save_custom_fields` for one key of a user's: an array
/// replaces the rows when it differs from them, nil removes them, a
/// string keeps the row holding it (or adds one) and drops the rest.
pub async fn save_custom_field(
    conn: &mut PgConnection,
    user_id: i32,
    name: &str,
    value: &Value,
) -> Result<(), AppError> {
    let rows: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id",
    )
    .bind(user_id)
    .bind(name)
    .fetch_all(&mut *conn)
    .await?;
    match value {
        Value::Array(items) => {
            let mut wanted = Vec::new();
            for item in items {
                match item {
                    Value::Null => {}
                    other => wanted.push(Some(serialize_value(other)?)),
                }
            }
            let current: Vec<Option<String>> = rows.iter().map(|(_, v)| v.clone()).collect();
            if wanted != current {
                for (id, _) in &rows {
                    delete_row(conn, *id).await?;
                }
                for v in wanted {
                    sqlx::query(
                        "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
                         VALUES ($1, $2, $3, now(), now())",
                    )
                    .bind(user_id)
                    .bind(name)
                    .bind(v)
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
        Value::Null => {
            for (id, _) in &rows {
                delete_row(conn, *id).await?;
            }
        }
        other => {
            let v = serialize_value(other)?;
            let keep = rows
                .iter()
                .find(|(_, r)| r.as_deref() == Some(v.as_str()))
                .map(|(id, _)| *id);
            for (id, _) in &rows {
                if Some(*id) != keep {
                    delete_row(conn, *id).await?;
                }
            }
            if keep.is_none() {
                // create_singular
                let inserted = sqlx::query(
                    "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
                     VALUES ($1, $2, $3, now(), now()) ON CONFLICT DO NOTHING",
                )
                .bind(user_id)
                .bind(name)
                .bind(&v)
                .execute(&mut *conn)
                .await?
                .rows_affected();
                if inserted == 0 {
                    sqlx::query(
                        "UPDATE user_custom_fields SET value = $3 WHERE user_id = $1 AND name = $2",
                    )
                    .bind(user_id)
                    .bind(name)
                    .bind(&v)
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
    }
    Ok(())
}

async fn delete_row(conn: &mut PgConnection, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM user_custom_fields WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}

/// `FieldDescriptor#serialize` for an untyped field.
fn serialize_value(value: &Value) -> Result<String, Unsupported> {
    Ok(match value {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "t".into(),
        Value::Bool(false) => "f".into(),
        Value::Number(n) => n.to_string(),
        _ => return Err(Unsupported("a user field value that is a hash")),
    })
}

/// `Admin::UserFieldsController.columns`: the permitted attributes.
const PERMITTED: [&str; 10] = [
    "name",
    "field_type",
    "editable",
    "description",
    "requirement",
    "show_on_profile",
    "show_on_signup",
    "show_on_user_card",
    "position",
    "searchable",
];

/// A field being written: the row's attributes, before validation.
#[derive(Clone, Debug)]
pub struct Draft {
    pub name: Option<String>,
    pub description: Option<String>,
    pub field_type_enum: Option<i32>,
    pub editable: Option<bool>,
    pub requirement: Option<i32>,
    pub show_on_profile: Option<bool>,
    pub show_on_user_card: Option<bool>,
    pub show_on_signup: Option<bool>,
    pub searchable: Option<bool>,
    pub position: Option<i32>,
}

impl Draft {
    /// `UserField.new`: the columns' defaults.
    pub fn new_record() -> Draft {
        Draft {
            name: None,
            description: None,
            field_type_enum: None,
            editable: Some(false),
            requirement: Some(0),
            show_on_profile: Some(false),
            show_on_user_card: Some(false),
            show_on_signup: Some(true),
            searchable: Some(false),
            position: Some(0),
        }
    }

    pub fn from_field(f: &UserField) -> Draft {
        Draft {
            name: Some(f.name.clone()),
            description: Some(f.description.clone()),
            field_type_enum: Some(f.field_type_enum),
            editable: Some(f.editable),
            requirement: Some(f.requirement),
            show_on_profile: Some(f.show_on_profile),
            show_on_user_card: Some(f.show_on_user_card),
            show_on_signup: Some(f.show_on_signup),
            searchable: Some(f.searchable),
            position: f.position,
        }
    }

    /// Assigns the permitted params: every one given (nil included) on
    /// create, the non-nil ones on update. An array or hash for a scalar
    /// attribute is unpermitted and dropped.
    pub fn assign(
        &mut self,
        params: &Map<String, Value>,
        skip_nil: bool,
    ) -> Result<(), Unsupported> {
        for key in PERMITTED {
            let Some(raw) = params.get(key) else { continue };
            if matches!(raw, Value::Array(_) | Value::Object(_)) {
                continue;
            }
            if raw.is_null() && skip_nil {
                continue;
            }
            let text = crate::params::scalar(raw);
            match key {
                "name" => self.name = text,
                "description" => self.description = text,
                "field_type" => {
                    self.field_type_enum = Some(
                        enum_value(raw, &FIELD_TYPES)
                            .ok_or(Unsupported("an unknown user field type (ArgumentError)"))?,
                    )
                }
                "requirement" => {
                    self.requirement = Some(enum_value(raw, &REQUIREMENTS).ok_or(Unsupported(
                        "an unknown user field requirement (ArgumentError)",
                    ))?)
                }
                "position" => self.position = integer(raw)?,
                _ => {
                    let b = match raw {
                        Value::Bool(b) => Some(*b),
                        _ => crate::ruby::cast_boolean(text.as_deref()),
                    };
                    match key {
                        "editable" => self.editable = b,
                        "show_on_profile" => self.show_on_profile = b,
                        "show_on_signup" => self.show_on_signup = b,
                        "show_on_user_card" => self.show_on_user_card = b,
                        _ => self.searchable = b,
                    }
                }
            }
        }
        Ok(())
    }

    /// The validations' full messages, in UserField's order.
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self
            .description
            .as_deref()
            .is_none_or(crate::ruby::is_blank)
        {
            errors.push("Description can't be blank".to_string());
        }
        if self.field_type_enum != Some(CONFIRM)
            && self.name.as_deref().is_none_or(crate::ruby::is_blank)
        {
            errors.push("Name can't be blank".to_string());
        }
        errors
    }

    /// The NOT NULL columns Rails would write a NULL to (a 500).
    fn check_not_null(&self) -> Result<(), Unsupported> {
        if self.name.is_none()
            || self.field_type_enum.is_none()
            || self.editable.is_none()
            || self.requirement.is_none()
            || self.show_on_profile.is_none()
            || self.show_on_user_card.is_none()
            || self.show_on_signup.is_none()
            || self.searchable.is_none()
        {
            return Err(Unsupported(
                "a user field with a NOT NULL column left nil (NotNullViolation)",
            ));
        }
        Ok(())
    }
}

/// ActiveModel's integer cast for the plain shapes: numbers, and strings
/// of digits; nil for blank. Anything else refused.
fn integer(raw: &Value) -> Result<Option<i32>, Unsupported> {
    match raw {
        Value::Null => Ok(None),
        Value::Number(n) => Ok(n.as_i64().map(|n| n as i32)),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) if s.trim().parse::<i32>().is_ok() => Ok(s.trim().parse().ok()),
        _ => Err(Unsupported("a user field position that isn't an integer")),
    }
}

/// An enum attribute's assignment: a key, or one of its integer values.
fn enum_value(raw: &Value, names: &[&str]) -> Option<i32> {
    match raw {
        Value::Number(n) => n
            .as_i64()
            .filter(|n| (0..names.len() as i64).contains(n))
            .map(|n| n as i32),
        Value::String(s) => names.iter().position(|n| n == s).map(|i| i as i32),
        _ => None,
    }
}

/// `options.map { |o| { value: o } }.uniq`: the values to add, or None
/// when `options` is not present (nothing to replace).
pub fn options_param(params: &Map<String, Value>) -> Result<Option<Vec<String>>, Unsupported> {
    let Some(raw) = params.get("options") else {
        return Ok(None);
    };
    let items = match raw {
        Value::Null => return Ok(None),
        Value::Array(items) if items.is_empty() => return Ok(None),
        Value::Array(items) => items,
        Value::String(s) if crate::ruby::is_blank(s) => return Ok(None),
        _ => return Err(Unsupported("user field options that aren't a list")),
    };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let v = crate::params::scalar(item)
            .ok_or(Unsupported("a user field option that isn't a string"))?;
        if !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(Some(out))
}

/// `field.save` for a new field (create) or an existing one (update),
/// with the callbacks: sanitize_description, the nested options,
/// update_required_fields_version, queue_index_search and the anon cache.
/// Err(messages) when invalid.
pub async fn save(
    conn: &mut PgConnection,
    bus: &pg_bus::Bus,
    before: Option<&UserField>,
    draft: &Draft,
    new_options: Option<&[String]>,
) -> Result<Result<UserField, Vec<String>>, AppError> {
    let errors = draft.errors();
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    draft.check_not_null()?;
    let mut description = draft.description.clone().unwrap_or_default();
    // before_save :sanitize_description, when it changed.
    if before.is_none_or(|b| b.description != description) {
        description = crate::sanitize_field::sanitize_field(&description, &["target"])?;
    }
    let after = UserField {
        id: before.map(|b| b.id).unwrap_or(0),
        name: draft.name.clone().unwrap_or_default(),
        description,
        field_type_enum: draft.field_type_enum.unwrap_or(0),
        editable: draft.editable.unwrap_or(false),
        requirement: draft.requirement.unwrap_or(0),
        show_on_profile: draft.show_on_profile.unwrap_or(false),
        show_on_user_card: draft.show_on_user_card.unwrap_or(false),
        show_on_signup: draft.show_on_signup.unwrap_or(true),
        searchable: draft.searchable.unwrap_or(false),
        position: draft.position,
    };
    let id = match before {
        None => sqlx::query_scalar::<_, i32>(
            "INSERT INTO user_fields (name, description, field_type_enum, editable, requirement, \
                   show_on_profile, show_on_user_card, show_on_signup, searchable, position, \
                   created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, now(), now()) RETURNING id",
        )
        .bind(&after.name)
        .bind(&after.description)
        .bind(after.field_type_enum)
        .bind(after.editable)
        .bind(after.requirement)
        .bind(after.show_on_profile)
        .bind(after.show_on_user_card)
        .bind(after.show_on_signup)
        .bind(after.searchable)
        .bind(after.position)
        .fetch_one(&mut *conn)
        .await?,
        Some(b) => {
            let changed = b.name != after.name
                || b.description != after.description
                || b.field_type_enum != after.field_type_enum
                || b.editable != after.editable
                || b.requirement != after.requirement
                || b.show_on_profile != after.show_on_profile
                || b.show_on_user_card != after.show_on_user_card
                || b.show_on_signup != after.show_on_signup
                || b.searchable != after.searchable
                || b.position != after.position;
            if changed {
                sqlx::query(
                    "UPDATE user_fields SET name = $2, description = $3, field_type_enum = $4, \
                       editable = $5, requirement = $6, show_on_profile = $7, show_on_user_card = $8, \
                       show_on_signup = $9, searchable = $10, position = $11, updated_at = now() \
                     WHERE id = $1",
                )
                .bind(b.id)
                .bind(&after.name)
                .bind(&after.description)
                .bind(after.field_type_enum)
                .bind(after.editable)
                .bind(after.requirement)
                .bind(after.show_on_profile)
                .bind(after.show_on_user_card)
                .bind(after.show_on_signup)
                .bind(after.searchable)
                .bind(after.position)
                .execute(&mut *conn)
                .await?;
            }
            b.id
        }
    };
    // accepts_nested_attributes_for :user_field_options
    for value in new_options.into_iter().flatten() {
        sqlx::query(
            "INSERT INTO user_field_options (user_field_id, value, created_at, updated_at) \
             VALUES ($1, $2, now(), now())",
        )
        .bind(id)
        .bind(value)
        .execute(&mut *conn)
        .await?;
    }
    // after_create, or after_update when the requirement changed:
    // update_required_fields_version.
    let requirement_changed = before.is_none_or(|b| b.requirement != after.requirement);
    if requirement_changed && after.requirement == FOR_ALL_USERS {
        sqlx::query(
            "INSERT INTO user_required_fields_versions (created_at, updated_at) VALUES (now(), now())",
        )
        .execute(&mut *conn)
        .await?;
        crate::bus::request_refresh(bus, &mut *conn).await?;
    }
    // after_save :queue_index_search
    crate::jobs::enqueue(
        &mut *conn,
        "index_user_fields_for_search",
        json!({ "user_field_id": id }),
    )
    .await?;
    // AnonCacheInvalidator
    crate::bus::clear_anon_cache(bus, &mut *conn).await?;
    let field = find(&mut *conn, id)
        .await?
        .ok_or(Unsupported("a user field gone once saved"))?;
    Ok(Ok(field))
}

/// `field.destroy`: its options and directory column (dependent
/// destroy), the row, the anon cache. Users' values stay.
pub async fn destroy(conn: &mut PgConnection, bus: &pg_bus::Bus, id: i32) -> Result<(), AppError> {
    sqlx::query("DELETE FROM user_field_options WHERE user_field_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM directory_columns WHERE user_field_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM user_fields WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    crate::bus::clear_anon_cache(bus, &mut *conn).await?;
    Ok(())
}

/// UsersController#clean_custom_field_values, then `nil if "false"` and
/// the cut to `UserField.max_length` (characters of a string, items of
/// an array): a dropdown keeps a value only when it is an option, a
/// multiselect drops the values that aren't.
pub async fn clean_value(
    conn: &mut PgConnection,
    field: &UserField,
    raw: Option<&Value>,
) -> Result<Value, AppError> {
    let value = match raw {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if s.is_empty() => json!(""),
        Some(Value::Array(a)) if a.is_empty() => json!([]),
        Some(v @ (Value::String(_) | Value::Array(_))) => {
            if let Value::Array(items) = v
                && items.iter().any(|i| !i.is_string())
            {
                return Err(Unsupported("user field values that aren't strings").into());
            }
            match field.field_type_enum {
                DROPDOWN => {
                    let Value::String(s) = v else {
                        return Err(Unsupported("a list for a dropdown user field").into());
                    };
                    let found: Option<String> = sqlx::query_scalar(
                        "SELECT value FROM user_field_options WHERE user_field_id = $1 AND value = $2 \
                         ORDER BY id LIMIT 1",
                    )
                    .bind(field.id)
                    .bind(s)
                    .fetch_optional(&mut *conn)
                    .await?;
                    json!(found)
                }
                MULTISELECT => {
                    let wanted: Vec<String> = match v {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(items) => items
                            .iter()
                            .filter_map(|i| i.as_str().map(str::to_string))
                            .collect(),
                        _ => Vec::new(),
                    };
                    let allowed = options(&mut *conn, field.id).await?;
                    json!(
                        wanted
                            .into_iter()
                            .filter(|w| allowed.contains(w))
                            .collect::<Vec<_>>()
                    )
                }
                _ => v.clone(),
            }
        }
        Some(_) => return Err(Unsupported("a user field value that isn't a string").into()),
    };
    Ok(match value {
        Value::String(s) if s == "false" => Value::Null,
        Value::String(s) => json!(s.chars().take(MAX_LENGTH).collect::<String>()),
        Value::Array(items) => Value::Array(items.into_iter().take(MAX_LENGTH).collect()),
        other => other,
    })
}

/// Ruby's `blank?` on a cleaned value.
pub fn blank(value: &Value) -> bool {
    !present(value)
}

/// UsersController#update's `user_fields`: the custom fields to merge,
/// by name, or Err(()) for `login.missing_user_field`. Staff may set any
/// field, others the editable ones; only the ids given are touched.
pub async fn update_values(
    conn: &mut PgConnection,
    given: &Map<String, Value>,
    staff: bool,
    user_id: i32,
) -> Result<Result<Vec<(String, Value)>, ()>, AppError> {
    let mut out = Vec::new();
    for field in unordered(&mut *conn).await? {
        if !staff && !field.editable {
            continue;
        }
        let key = field.id.to_string();
        if !given.contains_key(&key) {
            continue;
        }
        let value = clean_value(&mut *conn, &field, given.get(&key)).await?;
        if blank(&value) {
            let required = field.requirement == FOR_ALL_USERS
                || (field.requirement == ON_SIGNUP
                    && present(&custom_field_value(&mut *conn, user_id, field.id).await?));
            if required {
                return Ok(Err(()));
            }
        }
        out.push((format!("user_field_{}", field.id), value));
    }
    Ok(Ok(out))
}

/// UsersController#create's user fields: every field's cleaned value,
/// the blank ones left out, or Err(()) when a required one is blank.
pub async fn signup_values(
    conn: &mut PgConnection,
    given: Option<&Map<String, Value>>,
) -> Result<Result<Vec<(String, Value)>, ()>, AppError> {
    let mut out = Vec::new();
    for field in unordered(&mut *conn).await? {
        let raw = given.and_then(|g| g.get(&field.id.to_string()));
        let value = clean_value(&mut *conn, &field, raw).await?;
        if blank(&value) {
            if field.required() {
                return Ok(Err(()));
            }
            continue;
        }
        out.push((format!("user_field_{}", field.id), value));
    }
    Ok(Ok(out))
}

/// `UserRequiredFieldsVersion.current`
pub async fn current_required_version(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM user_required_fields_versions")
            .fetch_one(conn)
            .await?
            .unwrap_or(0),
    )
}

/// `User#needs_required_fields_check?`
pub async fn needs_required_fields_check(
    conn: &mut PgConnection,
    user_id: i32,
) -> Result<bool, sqlx::Error> {
    let version: Option<i32> =
        sqlx::query_scalar("SELECT required_fields_version FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    Ok(i64::from(version.unwrap_or(0)) < current_required_version(conn).await?)
}

/// Jobs::IndexUserFieldsForSearch: the users with a value for the field
/// queued for reindexing (SearchIndexer.queue_users_reindex).
pub async fn index_for_search(conn: &mut PgConnection, field_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_search_data SET version = 0 WHERE user_id IN \
           (SELECT user_id FROM user_custom_fields WHERE name = $1)",
    )
    .bind(format!("user_field_{field_id}"))
    .execute(conn)
    .await?;
    Ok(())
}
