//! FormTemplate with its validations (FormTemplateYamlValidator) behind
//! Admin::FormTemplatesController, and FormTemplatesController's list.
//!
//! The YAML is read with serde_yaml_ng, which is YAML 1.2 where Psych is
//! YAML 1.1: the plain scalars the two read differently (1.1's yes/no/on/
//! off booleans, sexagesimal and underscored numbers, dates Psych's
//! safe_load refuses), anchors and non-mapping fields are refused.
//! `process!` re-dumps the template with Psych, which isn't ported: the
//! preview and the public show are refused.

use serde_json::{Value, json};
use serde_yaml_ng::Value as Yaml;
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

const RESERVED_KEYWORDS: [&str; 5] = ["title", "body", "category", "category_id", "tags"];
const ALLOWED_TYPES: [&str; 8] = [
    "checkbox",
    "dropdown",
    "input",
    "multi-select",
    "textarea",
    "upload",
    "tag-chooser",
    "composer",
];

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct FormTemplate {
    pub id: i64,
    pub name: String,
    pub template: String,
}

/// (Admin)FormTemplateSerializer: id, name, template.
pub fn serialize(t: &FormTemplate) -> Value {
    json!({ "id": t.id, "name": t.name, "template": t.template })
}

pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<FormTemplate>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, template FROM form_templates WHERE id = $1")
        .bind(id)
        .fetch_optional(conn)
        .await
}

pub async fn all(conn: &mut PgConnection) -> Result<Vec<FormTemplate>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, template FROM form_templates ORDER BY id")
        .fetch_all(conn)
        .await
}

/// `accessible_form_templates`: those assigned to no category, and those
/// assigned to a category the viewer may see.
pub async fn accessible(
    conn: &mut PgConnection,
    allowed_category_ids: &[i32],
) -> Result<Vec<FormTemplate>, sqlx::Error> {
    let allowed: Vec<i64> = allowed_category_ids.iter().map(|i| i64::from(*i)).collect();
    sqlx::query_as(
        "SELECT id, name, template FROM form_templates \
         WHERE id NOT IN (SELECT form_template_id FROM category_form_templates) \
            OR id IN (SELECT form_template_id FROM category_form_templates WHERE category_id = ANY($1)) \
         ORDER BY id",
    )
    .bind(&allowed)
    .fetch_all(conn)
    .await
}

/// A plain scalar Psych would read otherwise than YAML 1.2.
fn psych_differs(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "y" | "n" | "yes" | "no" | "on" | "off" | "~"
    ) {
        return true;
    }
    let numeric = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '_' | ':' | '.' | '-' | '+'))
        && s.chars().any(|c| c.is_ascii_digit());
    numeric && (s.contains('_') || s.contains(':') || s.matches('-').count() == 2)
}

/// A field's value as Ruby's YAML gives it: strings and the scalars both
/// readings agree on; None for nil.
fn scalar(v: Option<&Yaml>) -> Result<Option<String>, Unsupported> {
    match v {
        None | Some(Yaml::Null) => Ok(None),
        Some(Yaml::String(s)) if psych_differs(s) => Err(Unsupported(
            "form template values YAML 1.1 reads differently (Psych)",
        )),
        Some(Yaml::String(s)) => Ok(Some(s.clone())),
        Some(Yaml::Bool(_)) => Err(Unsupported("form template fields with boolean values")),
        Some(Yaml::Number(n)) => Ok(Some(n.to_string())),
        Some(_) => Err(Unsupported("form template fields with list or hash values")),
    }
}

/// `Sanitize.fragment(description, HTML_SANITIZATION_OPTIONS)`
fn sanitized_description(description: &str) -> Result<String, Unsupported> {
    use crate::sanitize::config::{AttrAllow, Config, Protocols};
    let mut config = Config {
        elements: ["a"].iter().map(|s| s.to_string()).collect(),
        ..Config::default()
    };
    config
        .attributes
        .insert("a".into(), AttrAllow::new(&["href", "target"]));
    config.protocols.insert(
        "a".into(),
        [(
            "href".to_string(),
            Protocols::new(&["http", "https", "mailto"]),
        )]
        .into_iter()
        .collect(),
    );
    crate::sanitize::fragment(description, &config)
        .map_err(|_| Unsupported("a form template description Sanitize refuses"))
}

/// FormTemplate's validations' full messages, in order.
pub async fn errors(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    id: Option<i64>,
    name: Option<&str>,
    template: Option<&str>,
) -> Result<Vec<String>, AppError> {
    let mut out = Vec::new();
    let t = |key: &str, args: &[(&str, &str)]| {
        format!(
            "Template {}",
            i18n.t_with(&format!("form_templates.errors.{key}"), args)
                .unwrap_or_default()
        )
    };
    match name {
        None => out.push("Name can't be blank".to_string()),
        Some(n) if crate::ruby::is_blank(n) => out.push("Name can't be blank".to_string()),
        _ => {}
    }
    if let Some(n) = name {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM form_templates WHERE name = $1 AND id IS DISTINCT FROM $2)",
        )
        .bind(n)
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            out.push("Name has already been taken".to_string());
        }
        let max = settings.get("max_form_template_title_length")?.to_i();
        if n.chars().count() as i64 > max {
            out.push(format!("Name is too long (maximum is {max} characters)"));
        }
    }
    let Some(template) = template else {
        out.push("Template can't be blank".to_string());
        return Ok(out);
    };
    if crate::ruby::is_blank(template) {
        out.push("Template can't be blank".to_string());
    }
    let max = settings.get("max_form_template_content_length")?.to_i();
    if template.chars().count() as i64 > max {
        out.push(format!(
            "Template is too long (maximum is {max} characters)"
        ));
    }
    // Anchors, aliases and tags where a node starts: safe_load refuses
    // some, Psych reads the rest; not compared.
    static NODE_PROPERTY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let property = NODE_PROPERTY.get_or_init(|| {
        regex::Regex::new(r"(?m)(?:^|[:\-,\[\{?])[ \t]*[&*!][^\s]").expect("a regex")
    });
    if property.is_match(template) {
        return Err(Unsupported("form templates with YAML anchors, aliases or tags").into());
    }
    // FormTemplateYamlValidator
    let parsed: Result<Yaml, _> = serde_yaml_ng::from_str(template);
    let fields = match parsed {
        Ok(Yaml::Sequence(fields)) => fields,
        Ok(_) | Err(_) => {
            out.push(t("invalid_yaml", &[]));
            return Ok(out);
        }
    };
    let mut existing_ids: Vec<String> = Vec::new();
    let mut tag_groups: Vec<String> = Vec::new();
    for field in &fields {
        let Yaml::Mapping(map) = field else {
            return Err(Unsupported("form template fields that aren't mappings").into());
        };
        let get = |k: &str| scalar(map.get(Yaml::String(k.to_string())));
        let ftype = get("type")?;
        let fid = get("id")?;
        // check_missing_fields
        if ftype.as_deref().is_none_or(crate::ruby::is_blank) {
            out.push(t("missing_type", &[]));
        }
        if fid.as_deref().is_none_or(crate::ruby::is_blank) {
            out.push(t("missing_id", &[]));
        }
        // check_allowed_types
        if !ftype
            .as_deref()
            .is_some_and(|ty| ALLOWED_TYPES.contains(&ty))
        {
            out.push(t(
                "invalid_type",
                &[
                    ("type", ftype.as_deref().unwrap_or("")),
                    ("valid_types", &ALLOWED_TYPES.join(", ")),
                ],
            ));
        }
        // check_ids
        if let Some(id) = &fid {
            if RESERVED_KEYWORDS.contains(&id.as_str()) {
                out.push(t("reserved_id", &[("id", id)]));
            }
            if existing_ids.contains(id) {
                out.push(t("duplicate_ids", &[]));
            }
            if !crate::ruby::is_blank(id) {
                existing_ids.push(id.clone());
            }
        }
        // check_descriptions_html
        let description = match map.get(Yaml::String("attributes".into())) {
            Some(Yaml::Mapping(attrs)) => scalar(attrs.get(Yaml::String("description".into())))?,
            None | Some(Yaml::Null) => None,
            Some(_) => {
                return Err(Unsupported("form template attributes that aren't a mapping").into());
            }
        };
        if let Some(d) = description.filter(|d| !crate::ruby::is_blank(d)) {
            let sanitized = sanitized_description(&d)?;
            let loofah = crate::sanitize::html5_fragment_to_s(&d)
                .map_err(|_| Unsupported("a form template description Loofah refuses"))?;
            if sanitized != loofah {
                out.push(t("unsafe_description", &[]));
            }
        }
        if let Some(g) = get("tag_group")? {
            tag_groups.push(g);
        }
    }
    // check_tag_groups
    let mut names: Vec<String> = Vec::new();
    for g in tag_groups {
        let lower = g.to_lowercase();
        if !names.contains(&lower) {
            names.push(lower);
        }
    }
    let valid: Vec<String> =
        sqlx::query_scalar("SELECT lower(name) FROM tag_groups WHERE lower(name) = ANY($1)")
            .bind(&names)
            .fetch_all(&mut *conn)
            .await?;
    for name in names.iter().filter(|n| !valid.contains(n)) {
        out.push(t("invalid_tag_group", &[("tag_group_name", name)]));
    }
    Ok(out)
}

pub async fn create(
    conn: &mut PgConnection,
    name: &str,
    template: &str,
) -> Result<FormTemplate, sqlx::Error> {
    sqlx::query_as(
        "INSERT INTO form_templates (name, template, created_at, updated_at) \
         VALUES ($1, $2, now(), now()) RETURNING id, name, template",
    )
    .bind(name)
    .bind(template)
    .fetch_one(conn)
    .await
}

pub async fn update(
    conn: &mut PgConnection,
    before: &FormTemplate,
    name: &str,
    template: &str,
) -> Result<FormTemplate, sqlx::Error> {
    if before.name == name && before.template == template {
        return Ok(before.clone());
    }
    sqlx::query_as(
        "UPDATE form_templates SET name = $2, template = $3, updated_at = now() WHERE id = $1 \
         RETURNING id, name, template",
    )
    .bind(before.id)
    .bind(name)
    .bind(template)
    .fetch_one(conn)
    .await
}

/// `destroy!`: its category assignments (dependent destroy), then it.
pub async fn destroy(conn: &mut PgConnection, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM category_form_templates WHERE form_template_id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM form_templates WHERE id = $1")
        .bind(id)
        .execute(conn)
        .await?;
    Ok(())
}
