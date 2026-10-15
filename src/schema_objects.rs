//! SchemaSettingsObjectValidator: objects settings (a theme's `objects`
//! settings, site settings of the objects type) checked against their
//! schema, with the ids they name looked up, and their uploads turned
//! from urls to ids (normalize) and back (hydrate).
//!
//! A schema is the setting's `schema` option as JSON: `{name,
//! properties: {name: {type, required, validations, choices, schema}}}`.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::AppError;

/// The properties of a schema, in their order.
fn properties(schema: &Value) -> Vec<(&String, &Value)> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|m| m.iter().collect())
        .unwrap_or_default()
}

fn kind(attributes: &Value) -> Option<&str> {
    attributes.get("type").and_then(Value::as_str)
}

/// `property_values_of_type(schema:, objects:, type:)`: the values of
/// every property of the type, nested objects included, each once in the
/// order met.
pub fn property_values_of_type(schema: &Value, objects: &[Value], kind_wanted: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for object in objects {
        values_of_type(schema, object, kind_wanted, &mut out);
    }
    out
}

fn values_of_type(schema: &Value, object: &Value, wanted: &str, out: &mut Vec<Value>) {
    for (name, attributes) in properties(schema) {
        let value = object.get(name.as_str()).unwrap_or(&Value::Null);
        if kind(attributes) == Some(wanted) {
            // Array(value): nil none, an array its items, else itself.
            let items: Vec<&Value> = match value {
                Value::Null => Vec::new(),
                Value::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            for item in items {
                if !out.contains(item) {
                    out.push(item.clone());
                }
            }
        } else if kind(attributes) == Some("objects")
            && let Value::Array(children) = value
        {
            let child_schema = attributes.get("schema").unwrap_or(&Value::Null);
            for child in children {
                values_of_type(child_schema, child, wanted, out);
            }
        }
    }
}

/// Each upload property's value through `f`, nested objects included.
fn transform_uploads(objects: &mut [Value], schema: &Value, f: &mut dyn FnMut(&Value) -> Value) {
    for object in objects.iter_mut() {
        let Value::Object(map) = object else {
            continue;
        };
        for (name, attributes) in properties(schema) {
            if !map.contains_key(name.as_str()) {
                continue;
            }
            match kind(attributes) {
                Some("upload") => {
                    let new = f(&map[name.as_str()]);
                    map.insert(name.clone(), new);
                }
                Some("objects") => {
                    if let Some(Value::Array(children)) = map.get_mut(name.as_str()) {
                        let child_schema = attributes.get("schema").cloned().unwrap_or(Value::Null);
                        transform_uploads(children, &child_schema, f);
                    }
                }
                _ => {}
            }
        }
    }
}

/// `upload_ids`: the upload ids an objects value names (an id, or the
/// upload a url resolves to).
pub async fn upload_ids(
    conn: &mut PgConnection,
    schema: &Value,
    objects: &[Value],
) -> Result<Vec<i64>, AppError> {
    let mut out = Vec::new();
    for value in property_values_of_type(schema, objects, "upload") {
        match value {
            Value::Number(n) if n.is_i64() => out.push(n.as_i64().unwrap_or_default()),
            Value::String(s) if !crate::ruby::is_blank(&s) => {
                if let Some((id, _)) = crate::upload_references::get_from_url(conn, &s).await? {
                    out.push(i64::from(id));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// `normalize_uploads`: upload urls replaced by their uploads' ids.
pub async fn normalize_uploads(
    conn: &mut PgConnection,
    schema: &Value,
    objects: &[Value],
) -> Result<Vec<Value>, AppError> {
    let mut resolved: Vec<(String, Option<i32>)> = Vec::new();
    for value in property_values_of_type(schema, objects, "upload") {
        if let Value::String(s) = value
            && !crate::ruby::is_blank(&s)
        {
            let id = crate::upload_references::get_from_url(conn, &s)
                .await?
                .map(|(id, _)| id);
            resolved.push((s, id));
        }
    }
    let mut out = objects.to_vec();
    transform_uploads(&mut out, schema, &mut |value| match value {
        Value::String(s) => resolved
            .iter()
            .find(|(url, _)| url == s)
            .and_then(|(_, id)| *id)
            .map_or_else(|| value.clone(), |id| json!(id)),
        other => other.clone(),
    });
    Ok(out)
}

/// `hydrate_uploads`: upload ids replaced by their uploads' urls (through
/// the CDN when asked).
pub async fn hydrate_uploads(
    conn: &mut PgConnection,
    store: &crate::file_store::FileStore,
    schema: &Value,
    objects: &[Value],
    cdn: bool,
) -> Result<Vec<Value>, AppError> {
    let ids: Vec<i32> = property_values_of_type(schema, objects, "upload")
        .iter()
        .filter_map(|v| v.as_i64())
        .filter_map(|id| i32::try_from(id).ok())
        .collect();
    if objects.is_empty() || ids.is_empty() {
        return Ok(objects.to_vec());
    }
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT id, url FROM uploads WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?;
    let mut out = objects.to_vec();
    transform_uploads(&mut out, schema, &mut |value| {
        let found = value
            .as_i64()
            .and_then(|id| rows.iter().find(|(r, _)| i64::from(*r) == id));
        match found {
            Some((_, url)) if cdn => json!(store.cdn_url(url)),
            Some((_, url)) => json!(url),
            None => value.clone(),
        }
    });
    Ok(out)
}

/// The ids (or tag names) of a type that exist, looked up once per type.
struct ValidIds<'a> {
    schema: &'a Value,
    objects: &'a [Value],
    found: Vec<(String, Vec<Value>)>,
}

impl ValidIds<'_> {
    async fn contains(
        &mut self,
        conn: &mut PgConnection,
        kind: &str,
        value: &Value,
    ) -> Result<bool, AppError> {
        if !self.found.iter().any(|(k, _)| k == kind) {
            let values = property_values_of_type(self.schema, self.objects, kind);
            let existing: Vec<Value> = if kind == "tags" {
                let names: Vec<String> = values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                sqlx::query_scalar::<_, String>("SELECT name FROM tags WHERE name = ANY($1)")
                    .bind(&names)
                    .fetch_all(&mut *conn)
                    .await?
                    .into_iter()
                    .map(Value::String)
                    .collect()
            } else {
                // Topic and Post are Trashable: their default scope leaves
                // the deleted out.
                let (table, scope) = match kind {
                    "categories" => ("categories", ""),
                    "topic" => ("topics", " AND deleted_at IS NULL"),
                    "post" => ("posts", " AND deleted_at IS NULL"),
                    "groups" => ("groups", ""),
                    _ => ("uploads", ""),
                };
                let ids: Vec<i64> = values.iter().filter_map(Value::as_i64).collect();
                sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT id::bigint FROM {table} WHERE id = ANY($1){scope}"
                ))
                .bind(&ids)
                .fetch_all(&mut *conn)
                .await?
                .into_iter()
                .map(|id| json!(id))
                .collect()
            };
            self.found.push((kind.to_string(), existing));
        }
        Ok(self
            .found
            .iter()
            .find(|(k, _)| k == kind)
            .is_some_and(|(_, vs)| vs.contains(value)))
    }

    /// `@valid_ids_lookup["upload"] << id`
    fn add_upload(&mut self, id: i64) {
        if let Some((_, vs)) = self.found.iter_mut().find(|(k, _)| k == "upload") {
            vs.push(json!(id));
        } else {
            self.found.push(("upload".into(), vec![json!(id)]));
        }
    }
}

/// One error at a JSON pointer: its key and interpolations.
struct ObjectError {
    key: String,
    args: Vec<(String, String)>,
    count: Option<i64>,
}

/// Ruby's `to_s` of an interpolated value: arrays and hashes inspected.
fn interpolation(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => inspect(value),
        other => other.to_string(),
    }
}

/// `inspect` of a parsed YAML or JSON value, string keys as `"k" => v`.
pub fn inspect(value: &Value) -> String {
    match value {
        Value::Null => "nil".into(),
        Value::String(s) => crate::ruby::inspect_string(s),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(inspect).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{} => {}", crate::ruby::inspect_string(k), inspect(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

/// `blank?` of a JSON value, as `absent?` asks it (false is present).
fn absent(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(_) | Value::Number(_) => false,
        Value::String(s) => crate::ruby::is_blank(s),
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
    }
}

/// `DatetimeSettingValidator#valid_value?` for a present value: ISO 8601
/// with a `T` and a zone.
fn valid_datetime(value: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:[.,]\d+)?)?(?:Z|[+-]\d{2}:\d{2})\z",
        )
        .expect("valid regex")
    });
    if !RE.is_match(value) {
        return false;
    }
    let date = &value[..10];
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
        && value[11..13].parse::<u32>().is_ok_and(|h| h < 24)
        && value[14..16].parse::<u32>().is_ok_and(|m| m < 60)
}

/// `SchemaSettingsObjectValidator.validate_objects`: the humanized errors
/// of each object, by its index and pointer.
pub async fn validate_objects(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    schema: &Value,
    objects: &[Value],
) -> Result<Vec<String>, AppError> {
    let mut lookup = ValidIds {
        schema,
        objects,
        found: Vec::new(),
    };
    let mut messages = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        let mut errors: Vec<(String, Vec<ObjectError>)> = Vec::new();
        validate_object(conn, schema, object, "", &mut lookup, &mut errors).await?;
        for (pointer, list) in errors {
            for error in list {
                let pointer = format!("/{index}{pointer}");
                let mut args: Vec<(&str, &str)> = error
                    .args
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect();
                args.push(("property_json_pointer", &pointer));
                let key = format!("themes.settings_errors.objects.humanize_{}", error.key);
                let message = match error.count {
                    Some(count) => i18n.t_count(&key, count, &args),
                    None => i18n.t_with(&key, &args),
                };
                messages.push(message.unwrap_or_else(|| format!("Translation missing: en.{key}")));
            }
        }
    }
    Ok(messages)
}

fn add_error(
    errors: &mut Vec<(String, Vec<ObjectError>)>,
    prefix: &str,
    property: &str,
    error: ObjectError,
) {
    let pointer = format!("/{prefix}{property}");
    match errors.iter_mut().find(|(p, _)| *p == pointer) {
        Some((_, list)) => list.push(error),
        None => errors.push((pointer, vec![error])),
    }
}

fn simple(key: &str) -> ObjectError {
    ObjectError {
        key: key.to_string(),
        args: Vec::new(),
        count: None,
    }
}

/// A validator over one object (`#validate`).
async fn validate_object(
    conn: &mut PgConnection,
    schema: &Value,
    object: &Value,
    prefix: &str,
    lookup: &mut ValidIds<'_>,
    errors: &mut Vec<(String, Vec<ObjectError>)>,
) -> Result<(), AppError> {
    let Value::Object(object) = object else {
        return Err(std::io::Error::other("NoMethodError: with_indifferent_access").into());
    };
    let mut object = object.clone();
    for (name, attributes) in properties(schema) {
        if kind(attributes) == Some("objects") {
            let children = object.get(name.as_str()).cloned().unwrap_or(Value::Null);
            if absent(&children) {
                continue;
            }
            let Value::Array(children) = children else {
                return Err(crate::Unsupported("objects properties that are not lists").into());
            };
            let child_schema = attributes.get("schema").unwrap_or(&Value::Null);
            for (index, child) in children.iter().enumerate() {
                let child_prefix = format!("{prefix}{name}/{index}/");
                Box::pin(validate_object(
                    conn,
                    child_schema,
                    child,
                    &child_prefix,
                    lookup,
                    errors,
                ))
                .await?;
            }
            continue;
        }
        validate_property(conn, &mut object, name, attributes, prefix, lookup, errors).await?;
    }
    Ok(())
}

async fn validate_property(
    conn: &mut PgConnection,
    object: &mut Map<String, Value>,
    name: &str,
    attributes: &Value,
    prefix: &str,
    lookup: &mut ValidIds<'_>,
    errors: &mut Vec<(String, Vec<ObjectError>)>,
) -> Result<(), AppError> {
    let kind = kind(attributes).unwrap_or("");
    let value = object.get(name).cloned().unwrap_or(Value::Null);
    // has_valid_property_value_type?
    if !value.is_null() {
        let valid = match kind {
            "string" | "datetime" | "icon" => value.is_string(),
            "integer" | "topic" | "post" => value.is_i64() || value.is_u64(),
            "upload" => match &value {
                Value::String(url) => {
                    match crate::upload_references::get_from_url(conn, url).await? {
                        Some((id, _)) => {
                            object.insert(name.to_string(), json!(id));
                            lookup.add_upload(i64::from(id));
                            true
                        }
                        None => false,
                    }
                }
                other => other.is_i64() || other.is_u64(),
            },
            "float" => value.is_number(),
            "boolean" => value.is_boolean(),
            "enum" => attributes
                .get("choices")
                .and_then(Value::as_array)
                .is_some_and(|c| c.contains(&value)),
            "categories" | "groups" => value
                .as_array()
                .is_some_and(|a| a.iter().all(|v| v.is_i64() || v.is_u64())),
            "tags" => value
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string)),
            _ => {
                add_error(
                    errors,
                    prefix,
                    name,
                    ObjectError {
                        key: "invalid_type".into(),
                        args: vec![("type".into(), kind.to_string())],
                        count: None,
                    },
                );
                return Ok(());
            }
        };
        if !valid {
            // The property's attributes are the message's interpolations.
            let args = attributes
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| (k.clone(), interpolation(v)))
                        .collect()
                })
                .unwrap_or_default();
            add_error(
                errors,
                prefix,
                name,
                ObjectError {
                    key: format!("not_valid_{kind}_value"),
                    args,
                    count: None,
                },
            );
            return Ok(());
        }
    }
    let value = object.get(name).cloned().unwrap_or(Value::Null);
    if absent(&value) {
        if attributes.get("required").is_some_and(truthy) {
            add_error(errors, prefix, name, simple("required"));
        }
        return Ok(());
    }
    // has_valid_property_value?
    let validations = attributes.get("validations");
    let validation = |key: &str| {
        validations
            .and_then(|v| v.get(key))
            .filter(|v| !v.is_null())
    };
    match kind {
        "topic" | "upload" | "post" => {
            if !lookup.contains(conn, kind, &value).await? {
                add_error(
                    errors,
                    prefix,
                    name,
                    simple(&format!("not_valid_{kind}_value")),
                );
            }
        }
        "tags" | "categories" | "groups" => {
            let items = value.as_array().cloned().unwrap_or_default();
            for item in &items {
                if !lookup.contains(conn, kind, item).await? {
                    add_error(
                        errors,
                        prefix,
                        name,
                        simple(&format!("not_valid_{kind}_value")),
                    );
                    return Ok(());
                }
            }
            if let Some(min) = validation("min").and_then(Value::as_i64)
                && (items.len() as i64) < min
            {
                add_error(
                    errors,
                    prefix,
                    name,
                    counted(&format!("{kind}_value_not_valid_min"), min),
                );
                return Ok(());
            }
            if let Some(max) = validation("max").and_then(Value::as_i64)
                && (items.len() as i64) > max
            {
                add_error(
                    errors,
                    prefix,
                    name,
                    counted(&format!("{kind}_value_not_valid_max"), max),
                );
            }
        }
        "datetime" => {
            if !value.as_str().is_some_and(valid_datetime) {
                add_error(errors, prefix, name, simple("not_valid_datetime_value"));
            }
        }
        "string" => {
            let text = value.as_str().unwrap_or_default();
            let length = text.chars().count() as i64;
            if let Some(min) = validation("min_length").and_then(Value::as_i64)
                && length < min
            {
                add_error(
                    errors,
                    prefix,
                    name,
                    counted("string_value_not_valid_min", min),
                );
                return Ok(());
            }
            if let Some(max) = validation("max_length").and_then(Value::as_i64)
                && length > max
            {
                add_error(
                    errors,
                    prefix,
                    name,
                    counted("string_value_not_valid_max", max),
                );
                return Ok(());
            }
            if validation("url").is_some_and(truthy) && !crate::themes::valid_url(text) {
                add_error(errors, prefix, name, simple("string_value_not_valid_url"));
            }
        }
        "integer" | "float" => {
            let number = value.as_f64().unwrap_or_default();
            if let Some(min) = validation("min").filter(|v| v.is_number())
                && number < min.as_f64().unwrap_or_default()
            {
                add_error(
                    errors,
                    prefix,
                    name,
                    ObjectError {
                        key: "number_value_not_valid_min".into(),
                        args: vec![("min".into(), interpolation(min))],
                        count: None,
                    },
                );
                return Ok(());
            }
            if let Some(max) = validation("max").filter(|v| v.is_number())
                && number > max.as_f64().unwrap_or_default()
            {
                add_error(
                    errors,
                    prefix,
                    name,
                    ObjectError {
                        key: "number_value_not_valid_max".into(),
                        args: vec![("max".into(), interpolation(max))],
                        count: None,
                    },
                );
            }
        }
        _ => {}
    }
    Ok(())
}

fn counted(key: &str, count: i64) -> ObjectError {
    ObjectError {
        key: key.to_string(),
        args: Vec::new(),
        count: Some(count),
    }
}

/// Ruby truthiness of a parsed value.
fn truthy(value: &Value) -> bool {
    !matches!(value, Value::Null | Value::Bool(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({"name": "link", "properties": {
            "title": {"type": "string"},
            "image": {"type": "upload"},
            "children": {"type": "objects", "schema": {"properties": {
                "icon": {"type": "upload"},
                "label": {"type": "string"}
            }}}
        }})
    }

    #[test]
    fn values_of_a_type_include_nested_objects_once() {
        let objects = vec![
            json!({"title": "a", "image": 5, "children": [{"icon": 6}, {"icon": 5}]}),
            json!({"title": "b", "image": "/uploads/x.png"}),
        ];
        assert_eq!(
            property_values_of_type(&schema(), &objects, "upload"),
            vec![json!(5), json!(6), json!("/uploads/x.png")]
        );
        assert_eq!(
            property_values_of_type(&schema(), &objects, "string"),
            vec![json!("a"), json!("b")]
        );
    }

    #[test]
    fn hashes_inspect_with_string_keys() {
        assert_eq!(
            inspect(&json!([{"a": 1, "b": ["x", null]}])),
            r#"[{"a" => 1, "b" => ["x", nil]}]"#
        );
    }

    #[test]
    fn datetimes_need_a_time_and_a_zone() {
        assert!(valid_datetime("2026-10-14T10:00:00Z"));
        assert!(valid_datetime("2026-10-14T10:00+02:00"));
        assert!(!valid_datetime("2026-10-14"));
        assert!(!valid_datetime("2026-10-14T10:00:00"));
        assert!(!valid_datetime("2026-13-14T10:00:00Z"));
    }
}
