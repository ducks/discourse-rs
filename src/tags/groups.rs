//! TagGroup's writes (app/models/tag_group.rb) and TagGroupSerializer:
//! creating, updating and destroying a group with its tags, parent tag,
//! one_per_topic and permissions, as TagGroupsController assigns them.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `TagGroup.create!(name:)`: the row, with init_permissions' default
/// (everyone may use its tags).
pub async fn create_minimal(conn: &mut PgConnection, name: &str) -> Result<i32, sqlx::Error> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tag_groups (name, one_per_topic, created_at, updated_at) \
         VALUES ($1, FALSE, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(name)
    .fetch_one(&mut *conn)
    .await?;
    insert_permission(conn, id, 0, crate::tagging::PERMISSION_FULL).await?;
    Ok(id)
}

async fn insert_permission(
    conn: &mut PgConnection,
    group_id: i32,
    group: i64,
    permission_type: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO tag_group_permissions (tag_group_id, group_id, permission_type, created_at, updated_at) \
         VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
    )
    .bind(group_id)
    .bind(group)
    .bind(permission_type)
    .execute(conn)
    .await?;
    Ok(())
}

/// TagGroupSerializer.
pub async fn serialize(conn: &mut PgConnection, id: i32) -> Result<Value, AppError> {
    let (name, one_per_topic, parent_tag_id): (String, bool, Option<i32>) =
        sqlx::query_as("SELECT name, one_per_topic, parent_tag_id FROM tag_groups WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let tags: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT tags.id, tags.name, tags.slug FROM tags \
         JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
         WHERE tgm.tag_group_id = $1 AND tags.target_tag_id IS NULL ORDER BY tags.name",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let slug_for_url = |id: i32, slug: &str| {
        if slug.is_empty() {
            format!("{id}-tag")
        } else {
            slug.to_string()
        }
    };
    let parent: Option<(i32, String, String)> = match parent_tag_id {
        Some(pid) => {
            sqlx::query_as("SELECT id, name, slug FROM tags WHERE id = $1")
                .bind(pid)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let permissions: Vec<(i64, i32)> = sqlx::query_as(
        "SELECT group_id, permission_type FROM tag_group_permissions WHERE tag_group_id = $1 ORDER BY ctid",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut perms = Map::new();
    for (group, permission) in &permissions {
        perms.insert(group.to_string(), json!(permission));
    }
    if perms.is_empty() {
        perms.insert("0".into(), json!(crate::tagging::PERMISSION_FULL));
    }
    Ok(json!({
        "id": id,
        "name": name,
        "tags": tags.iter().map(|(id, name, slug)| json!({"id": id, "name": name, "slug": slug_for_url(*id, slug)})).collect::<Vec<_>>(),
        "parent_tag": match parent {
            Some((id, name, slug)) => json!([{"id": id, "name": name, "slug": slug_for_url(id, &slug)}]),
            None => json!([]),
        },
        "one_per_topic": one_per_topic,
        "permissions": perms,
    }))
}

/// What tag_groups_params leaves for the group's tags.
pub enum TagsChoice {
    /// `tag_ids=`: these tags and their synonyms.
    Ids(Vec<i64>),
    /// `tag_names=`: add_or_create_tags_by_name, unlimited.
    Names(Vec<String>),
}

/// What tag_groups_params leaves for the parent tag.
pub enum ParentChoice {
    /// `parent_tag_id=`: the tag, when it exists.
    Id(i64),
    /// `parent_tag_name=`: the first name kept, found or created; none
    /// for an empty list.
    Names(Vec<String>),
}

/// The attributes a create or update assigns.
pub struct GroupParams {
    pub name: Option<String>,
    pub tags: TagsChoice,
    pub parent: ParentChoice,
    pub one_per_topic: bool,
    /// `permissions=`, resolved to group ids; None when not given.
    pub permissions: Option<Vec<(i64, i32)>>,
}

/// `Group.group_id_from_param` for a permissions key: an auto group's
/// name, digits, or another group's name.
pub async fn group_id_from_param(conn: &mut PgConnection, key: &str) -> Result<i64, AppError> {
    let auto = [
        ("everyone", 0),
        ("admins", 1),
        ("moderators", 2),
        ("staff", 3),
        ("anonymous_users", 4),
        ("logged_in_users", 5),
        ("trust_level_0", 10),
        ("trust_level_1", 11),
        ("trust_level_2", 12),
        ("trust_level_3", 13),
        ("trust_level_4", 14),
    ];
    if let Some((_, id)) = auto.iter().find(|(n, _)| *n == key) {
        return Ok(*id);
    }
    if let Ok(id) = key.parse::<i64>()
        && id.to_string() == key
    {
        return Ok(id);
    }
    let id: Option<i32> = sqlx::query_scalar("SELECT id FROM groups WHERE name = $1")
        .bind(key)
        .fetch_optional(conn)
        .await?;
    Ok(i64::from(id.ok_or(Unsupported(
        "a tag group permission for a group that doesn't exist (a NoMethodError in Rails)",
    ))?))
}

/// The tags `tags=` leaves: those named by id, then their synonyms.
async fn tags_with_synonyms(conn: &mut PgConnection, ids: &[i64]) -> Result<Vec<i32>, sqlx::Error> {
    let ids: Vec<i32> = ids.iter().filter_map(|i| i32::try_from(*i).ok()).collect();
    let mut selected: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM tags WHERE id = ANY($1) ORDER BY ctid")
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?;
    let synonyms: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM tags WHERE target_tag_id = ANY($1) AND NOT (id = ANY($1)) ORDER BY ctid",
    )
    .bind(&selected)
    .fetch_all(&mut *conn)
    .await?;
    selected.extend(synonyms);
    Ok(selected)
}

/// The group's tags as the assignment leaves them: None when nothing
/// changes (add_or_create_tags_by_name with the same names).
async fn resolve_tags(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    group_id: Option<i32>,
    choice: &TagsChoice,
) -> Result<Option<Vec<i32>>, AppError> {
    match choice {
        TagsChoice::Ids(ids) => Ok(Some(tags_with_synonyms(conn, ids).await?)),
        TagsChoice::Names(names) => {
            let system = crate::web_hooks::system_guardian(conn).await?;
            let tag_names =
                crate::tagging::tags_for_saving(conn, settings, &system, names, true).await?;
            let mut current: Vec<String> = sqlx::query_scalar(
                "SELECT tags.name FROM tags JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
                 WHERE tgm.tag_group_id = $1",
            )
            .bind(group_id)
            .fetch_all(&mut *conn)
            .await?;
            current.sort();
            let mut wanted = tag_names.clone();
            wanted.sort();
            if current == wanted {
                return Ok(None);
            }
            let lower: Vec<String> = tag_names.iter().map(|n| n.to_lowercase()).collect();
            let found: Vec<(i32, String)> = sqlx::query_as(
                "SELECT id, name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid",
            )
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?;
            let mut ids: Vec<i32> = found.iter().map(|(id, _)| *id).collect();
            let synonyms: Vec<i32> = sqlx::query_scalar(
                "SELECT id FROM tags WHERE target_tag_id = ANY($1) AND NOT (id = ANY($1)) ORDER BY ctid",
            )
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?;
            let new_names: Vec<String> = if found.len() < tag_names.len() {
                tag_names
                    .iter()
                    .filter(|n| !found.iter().any(|(_, f)| f == *n))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            ids.extend(synonyms);
            for name in new_names {
                let tag = crate::tagging::create_tag(conn, settings, &name)
                    .await?
                    .ok_or(Unsupported(
                        "a tag name Tag's validations refuse, in a tag group",
                    ))?;
                crate::tagging::enqueue_tag_created(conn, std::slice::from_ref(&tag)).await?;
                ids.push(tag.id);
            }
            Ok(Some(ids))
        }
    }
}

/// The parent tag the assignment leaves.
async fn resolve_parent(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    choice: &ParentChoice,
) -> Result<Option<i32>, AppError> {
    match choice {
        ParentChoice::Id(id) => {
            let id = i32::try_from(*id).ok();
            Ok(sqlx::query_scalar("SELECT id FROM tags WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?)
        }
        ParentChoice::Names(names) if names.is_empty() => Ok(None),
        ParentChoice::Names(names) => {
            let system = crate::web_hooks::system_guardian(conn).await?;
            let saving =
                crate::tagging::tags_for_saving(conn, settings, &system, names, false).await?;
            let Some(name) = saving.first() else {
                return Ok(None);
            };
            if let Some(tag) = crate::tags::Tag::find_by_name(&mut *conn, name).await? {
                return Ok(Some(tag.id));
            }
            let tag = crate::tagging::create_tag(conn, settings, name)
                .await?
                .ok_or(Unsupported("a parent tag name Tag's validations refuse"))?;
            crate::tagging::enqueue_tag_created(conn, std::slice::from_ref(&tag)).await?;
            Ok(Some(tag.id))
        }
    }
}

/// The group's validations: the name's length and uniqueness, and on
/// update a permissions list that isn't empty.
async fn validate(
    conn: &mut PgConnection,
    i18n: &I18n,
    id: Option<i32>,
    name: &str,
    permissions: Option<&Vec<(i64, i32)>>,
) -> Result<Vec<String>, AppError> {
    let mut errors = Vec::new();
    let message = |key: &str, count: Option<i64>| {
        let text = match count {
            Some(n) => i18n
                .t_count(&format!("activerecord.errors.messages.{key}"), n, &[])
                .or_else(|| i18n.t_count(&format!("errors.messages.{key}"), n, &[]))
                .unwrap_or_default(),
            None => i18n
                .t(&format!("activerecord.errors.messages.{key}"))
                .or_else(|| i18n.t(&format!("errors.messages.{key}")))
                .unwrap_or(key)
                .to_string(),
        };
        format!("Name {text}")
    };
    if name.chars().count() > 100 {
        errors.push(message("too_long", Some(100)));
    }
    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM tag_groups WHERE lower(name) = lower($1) AND id IS DISTINCT FROM $2)",
    )
    .bind(name)
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    if taken {
        errors.push(message("taken", None));
    }
    if id.is_some() && permissions.is_some_and(Vec::is_empty) {
        errors.push(
            i18n.t("tags.groups.errors.empty_permissions")
                .unwrap_or_default()
                .to_string(),
        );
    }
    Ok(errors)
}

/// Replaces the group's tags with `wanted`, the parent left out
/// (remove_parent_from_group), keeping the memberships that stay.
async fn set_tags(
    conn: &mut PgConnection,
    group_id: i32,
    wanted: &[i32],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM tag_group_memberships WHERE tag_group_id = $1 AND NOT (tag_id = ANY($2))",
    )
    .bind(group_id)
    .bind(wanted)
    .execute(&mut *conn)
    .await?;
    let mut added: Vec<i32> = Vec::new();
    for tag_id in wanted {
        if added.contains(tag_id) {
            continue;
        }
        added.push(*tag_id);
        sqlx::query(
            "INSERT INTO tag_group_memberships (tag_id, tag_group_id, created_at, updated_at) \
             SELECT $1, $2, clock_timestamp(), clock_timestamp() \
             WHERE NOT EXISTS (SELECT 1 FROM tag_group_memberships WHERE tag_id = $1 AND tag_group_id = $2)",
        )
        .bind(tag_id)
        .bind(group_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `TagGroup.new(params).save` (Ok with the id) or its errors.
pub async fn create(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    params: &GroupParams,
) -> Result<Result<i32, Vec<String>>, AppError> {
    let name = params.name.clone().unwrap_or_default();
    let tags = resolve_tags(conn, settings, None, &params.tags).await?;
    let parent = resolve_parent(conn, settings, &params.parent).await?;
    let errors = validate(conn, i18n, None, &name, params.permissions.as_ref()).await?;
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tag_groups (name, one_per_topic, parent_tag_id, created_at, updated_at) \
         VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(&name)
    .bind(params.one_per_topic)
    .bind(parent)
    .fetch_one(&mut *conn)
    .await?;
    // apply_permissions, else init_permissions' default.
    match &params.permissions {
        Some(perms) if !perms.is_empty() => {
            for (group, permission) in perms {
                insert_permission(conn, id, *group, *permission).await?;
            }
        }
        _ => insert_permission(conn, id, 0, crate::tagging::PERMISSION_FULL).await?,
    }
    let mut wanted = tags.unwrap_or_default();
    if let Some(p) = parent {
        wanted.retain(|t| *t != p);
    }
    set_tags(conn, id, &wanted).await?;
    Ok(Ok(id))
}

/// `tag_group.update(params)`, in the caller's transaction: Ok or the
/// errors (the caller rolls back).
pub async fn update(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    id: i32,
    params: &GroupParams,
) -> Result<Result<(), Vec<String>>, AppError> {
    let (current_name, current_one, current_parent): (String, bool, Option<i32>) =
        sqlx::query_as("SELECT name, one_per_topic, parent_tag_id FROM tag_groups WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    // The tags are assigned (and written) first, as `tags=` on a saved
    // record does.
    if let Some(tags) = resolve_tags(conn, settings, Some(id), &params.tags).await? {
        set_tags(conn, id, &tags).await?;
    }
    let parent = resolve_parent(conn, settings, &params.parent).await?;
    let name = params.name.clone().unwrap_or(current_name.clone());
    let errors = validate(conn, i18n, Some(id), &name, params.permissions.as_ref()).await?;
    if !errors.is_empty() {
        return Ok(Err(errors));
    }
    if let Some(perms) = &params.permissions {
        sqlx::query("DELETE FROM tag_group_permissions WHERE tag_group_id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        for (group, permission) in perms {
            insert_permission(conn, id, *group, *permission).await?;
        }
    }
    if let Some(p) = parent {
        sqlx::query("DELETE FROM tag_group_memberships WHERE tag_group_id = $1 AND tag_id = $2")
            .bind(id)
            .bind(p)
            .execute(&mut *conn)
            .await?;
    }
    if name != current_name || params.one_per_topic != current_one || parent != current_parent {
        sqlx::query(
            "UPDATE tag_groups SET name = $2, one_per_topic = $3, parent_tag_id = $4, \
             updated_at = clock_timestamp() WHERE id = $1",
        )
        .bind(id)
        .bind(&name)
        .bind(params.one_per_topic)
        .bind(parent)
        .execute(&mut *conn)
        .await?;
    }
    Ok(Ok(()))
}

/// `TagGroup#destroy`: its memberships, category tag groups, required
/// tag groups and permissions, then the row.
pub async fn destroy(conn: &mut PgConnection, id: i32) -> Result<(), sqlx::Error> {
    for table in [
        "tag_group_memberships",
        "category_tag_groups",
        "category_required_tag_groups",
        "tag_group_permissions",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE tag_group_id = $1"))
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    sqlx::query("DELETE FROM tag_groups WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// `StaffActionLogger#log_tag_group_*`: the serialized group before and
/// after, unless one is longer than UserHistory::MAX_JSON_LENGTH.
pub async fn log(
    conn: &mut PgConnection,
    i18n: &I18n,
    acting_user_id: i32,
    action: &str,
    subject: &str,
    previous: Option<&str>,
    new: Option<&str>,
) -> Result<(), AppError> {
    const MAX_JSON_LENGTH: usize = 300_000;
    let too_long = previous.is_some_and(|p| p.len() > MAX_JSON_LENGTH)
        || new.is_some_and(|n| n.len() > MAX_JSON_LENGTH);
    let (previous, new, context) = if too_long {
        (
            None,
            None,
            Some(
                i18n.t("staff_action_logs.json_too_long")
                    .unwrap_or_default()
                    .to_string(),
            ),
        )
    } else {
        (previous, new, None)
    };
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, subject, previous_value, new_value, context, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, FALSE, clock_timestamp(), clock_timestamp())",
    )
    .bind(
        crate::staff_action_logs::action_id(action)
            .ok_or(Unsupported("a staff action missing from the enums"))?,
    )
    .bind(acting_user_id)
    .bind(subject)
    .bind(previous)
    .bind(new)
    .bind(context)
    .execute(conn)
    .await?;
    Ok(())
}
