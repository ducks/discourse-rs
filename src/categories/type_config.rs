//! `Categories::Configure` and `Categories::Unconfigure`
//! (app/services/categories/): making a category one of the registered
//! types, as the category editor's type selector and the creator's type
//! do. Each enables the type's plugin, applies the site settings its
//! schema names (SiteSetting::Update, admins only), configures the
//! category and logs it; unconfiguring reverses the category's part.
//!
//! Ported for Discussion (nothing to configure), Ideas (topic voting's
//! category setting: the voting toggle) and Support (solved's custom
//! fields). Events (discourse-events' calendar settings) is refused.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::category_types::{Registry, TYPES, Type};
use crate::category_writes::Cx;
use crate::guardian::Guardian;
use crate::{AppError, Unsupported};

/// `Categories::TypeRegistry.get(id)`
pub(crate) fn parse(id: &str) -> Option<Type> {
    TYPES.into_iter().find(|t| Registry::id(*t) == id)
}

/// `type_class.available_for?(guardian)`: a type whose plugin is off is
/// for admins only.
pub(crate) fn available_for(
    r: &Registry<'_>,
    t: Type,
    guardian: &Guardian,
) -> Result<bool, AppError> {
    Ok(match r.plugin(t)? {
        Some((_, enabled)) => enabled || guardian.is_admin(),
        None => true,
    })
}

/// `String#humanize` for a type id.
fn humanize(id: &str) -> String {
    let base = id.replace('_', " ");
    let mut chars = base.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// `category_type_unavailable_error`
pub(crate) fn unavailable_error(
    i18n: &crate::i18n::I18n,
    r: &Registry<'_>,
    id: &str,
) -> Result<String, AppError> {
    let type_name = humanize(id);
    let plugin = match parse(id) {
        Some(t) => r.plugin(t)?.map(|(name, _)| name),
        None => None,
    };
    Ok(match plugin {
        Some(plugin_name) => i18n
            .t_with(
                "category_types.requires_plugin",
                &[("type_name", &type_name), ("plugin_name", plugin_name)],
            )
            .unwrap_or_default(),
        None => i18n
            .t_with("category_types.not_available", &[("type_name", &type_name)])
            .unwrap_or_default(),
    })
}

/// The custom field keys a type's schema declares
/// (`configuration_schema_keys(:category_custom_fields)`).
pub(crate) async fn custom_field_keys(
    conn: &mut PgConnection,
    r: &Registry<'_>,
    t: Type,
) -> Result<Vec<&'static str>, AppError> {
    Ok(match r.schema(conn, t).await? {
        Some(schema) => schema
            .category_custom_fields
            .iter()
            .map(|f| f.key)
            .collect(),
        None => Vec::new(),
    })
}

/// `enable_plugin`: the plugin's settings turned on (`SiteSetting.x =
/// true`, the override always written).
async fn enable_plugin(conn: &mut PgConnection, cx: &Cx<'_>, t: Type) -> Result<(), AppError> {
    let names: &[&str] = match t {
        Type::Discussion => &[],
        Type::Ideas => &["topic_voting_enabled"],
        Type::Support => &["solved_enabled"],
        Type::Events => &["discourse_events_enabled", "discourse_post_event_enabled"],
    };
    for name in names {
        match crate::site_setting_update::set(
            &mut *conn,
            cx.bus,
            cx.settings,
            &cx.host.site_setting_defs,
            &cx.host.i18n,
            name,
            &json!(true),
        )
        .await?
        {
            Ok(_) => {}
            Err(m) => return Err(Unsupported(Box::leak(m.into_boxed_str())).into()),
        }
    }
    Ok(())
}

/// `configure_site_settings` through SiteSetting::Update: each schema
/// setting at its given or default value, cast for its type, logged when
/// it changes; nothing for a non-admin (the service's admin policy).
async fn configure_site_settings(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &Registry<'_>,
    guardian: &Guardian,
    t: Type,
    values: &Map<String, Value>,
) -> Result<(), AppError> {
    let Some(schema) = r.schema(conn, t).await? else {
        return Ok(());
    };
    if schema.site_settings.is_empty() || !guardian.is_admin() {
        return Ok(());
    }
    let actor = guardian.user_id().unwrap_or(-1);
    for setting in &schema.site_settings {
        let raw = values.get(setting.key).unwrap_or(&setting.default);
        let text = match raw {
            Value::String(s) => s.trim().to_string(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let def = cx
            .host
            .site_setting_defs
            .get(setting.key)
            .ok_or(Unsupported("a category type setting that isn't defined"))?;
        let new_value = match def.data_type {
            crate::site_settings::DataType::Bool => {
                crate::site_settings::Value::Bool(text == "true" || text == "t")
            }
            crate::site_settings::DataType::Integer => {
                let digits: String = text
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '-')
                    .collect();
                crate::site_settings::Value::Int(crate::ruby::to_i(&digits))
            }
            crate::site_settings::DataType::String => crate::site_settings::Value::Str(text),
            _ => {
                return Err(Unsupported("a category type's site setting of this type").into());
            }
        };
        let previous = cx.settings.get(setting.key)?.clone();
        // set_and_log's early return
        if previous == new_value {
            continue;
        }
        crate::site_setting_update::set_and_log(
            &mut *conn, cx.bus, def, &previous, &new_value, actor,
        )
        .await?;
    }
    Ok(())
}

/// `Categories::Configure.call`: Ok(Err(message)) when the type is not
/// available to the guardian (the policy the controllers answer).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn configure(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    r: &Registry<'_>,
    guardian: &Guardian,
    category_id: i32,
    id: &str,
    category_values: &Map<String, Value>,
    site_setting_values: &Map<String, Value>,
) -> Result<Result<(), String>, AppError> {
    // An unknown type fails the contract, which nothing answers.
    let Some(t) = parse(id) else {
        return Ok(Ok(()));
    };
    // policy :can_modify_category
    let Some(c) =
        crate::category_serializer::GuardedCategory::load(&mut *conn, category_id).await?
    else {
        return Ok(Ok(()));
    };
    if !crate::category_serializer::can_edit(&mut *conn, cx.settings, guardian, &c).await? {
        return Ok(Ok(()));
    }
    if !available_for(r, t, guardian)? {
        return Ok(Err(unavailable_error(&cx.host.i18n, r, id)?));
    }
    if t == Type::Events {
        return Err(Unsupported("configuring a category as an Events category").into());
    }
    enable_plugin(conn, cx, t).await?;
    configure_site_settings(conn, cx, r, guardian, t, site_setting_values).await?;
    match t {
        Type::Ideas => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM topic_voting_category_settings WHERE category_id = $1)",
            )
            .bind(category_id)
            .fetch_one(&mut *conn)
            .await?;
            if !exists {
                // before_create: unarchive_votes
                sqlx::query(
                    "UPDATE topic_voting_votes SET archive = false FROM topics \
                     WHERE topics.category_id = $1 AND topics.deleted_at IS NULL \
                       AND NOT topics.closed AND NOT topics.archived \
                       AND topic_voting_votes.topic_id = topics.id",
                )
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
                sqlx::query(
                    "INSERT INTO topic_voting_category_settings (category_id, created_at, updated_at) \
                     VALUES ($1, now(), now())",
                )
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
            }
        }
        Type::Support => {
            // configure_custom_fields over Support's defaults, accepted
            // answers forced on.
            let schema = r
                .schema(conn, t)
                .await?
                .ok_or(Unsupported("Support's schema"))?;
            let mut values = category_values.clone();
            for key in [
                "notify_on_staff_accept_solved",
                "empty_box_on_unsolved",
                "enable_shared_issues",
            ] {
                values.entry(key).or_insert(json!("true"));
            }
            values.insert("enable_accepted_answers".into(), json!("true"));
            let mut fields = Map::new();
            for field in &schema.category_custom_fields {
                let v = values.get(field.key).unwrap_or(&field.default);
                let s = match v {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                fields.insert(field.key.to_string(), json!(s));
            }
            crate::category_writes::write_custom_fields(&mut *conn, category_id, &fields).await?;
        }
        Type::Discussion | Type::Events => {}
    }
    log(conn, guardian, "configure_category_type", category_id, id).await?;
    Ok(Ok(()))
}

/// `Categories::Unconfigure.call`
pub(crate) async fn unconfigure(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    guardian: &Guardian,
    category_id: i32,
    id: &str,
) -> Result<(), AppError> {
    let Some(t) = parse(id) else {
        return Ok(());
    };
    let Some(c) =
        crate::category_serializer::GuardedCategory::load(&mut *conn, category_id).await?
    else {
        return Ok(());
    };
    if !crate::category_serializer::can_edit(&mut *conn, cx.settings, guardian, &c).await? {
        return Ok(());
    }
    match t {
        Type::Ideas => {
            // CategorySetting#destroy!: before_destroy archive_votes.
            sqlx::query(
                "UPDATE topic_voting_votes SET archive = true FROM topics \
                 WHERE topics.category_id = $1 AND topic_voting_votes.topic_id = topics.id",
            )
            .bind(category_id)
            .execute(&mut *conn)
            .await?;
            sqlx::query("DELETE FROM topic_voting_category_settings WHERE category_id = $1")
                .bind(category_id)
                .execute(&mut *conn)
                .await?;
        }
        Type::Support => {
            // custom_fields[enable_accepted_answers] = "false"; save!
            let mut record = crate::category_writes::Record::load(&mut *conn, category_id)
                .await?
                .ok_or(Unsupported(
                    "a category that vanished while unconfiguring it",
                ))?;
            let mut fields = record.custom_fields_loaded().clone();
            fields.insert("enable_accepted_answers".into(), json!("false"));
            record.set_custom_fields(fields);
            match crate::category_writes::save(&mut *conn, cx, &mut record).await? {
                crate::category_writes::Saved::Invalid(_) => {
                    return Err(Unsupported(
                        "a category failing its validations as Support is removed (save!)",
                    )
                    .into());
                }
                crate::category_writes::Saved::Done(_) => {}
            }
            crate::category_writes::after_commit(
                &mut *conn,
                cx,
                category_id,
                "category_updated",
                Some(record.saved_changes.as_slice()),
                true,
            )
            .await?;
            record.run_deferred(&mut *conn).await?;
        }
        Type::Events => {
            return Err(Unsupported("removing the Events type from a category").into());
        }
        Type::Discussion => {}
    }
    log(conn, guardian, "unconfigure_category_type", category_id, id).await?;
    Ok(())
}

async fn log(
    conn: &mut PgConnection,
    guardian: &Guardian,
    custom_type: &str,
    category_id: i32,
    id: &str,
) -> Result<(), AppError> {
    crate::category_writes::log_custom(
        conn,
        guardian.user_id().unwrap_or(-1),
        custom_type,
        category_id,
        &[("category_type", id.to_string())],
    )
    .await
}
