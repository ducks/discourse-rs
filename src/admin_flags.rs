//! Flag (the post action types) with FlagSerializer, and the
//! Flags::CreateFlag, UpdateFlag, DestroyFlag, ToggleFlag and ReorderFlag
//! services behind /admin/config/flags. Custom flags are the ids from
//! 1000 up; the system flags below can only be toggled and moved.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::i18n::I18n;
use crate::{AppError, Unsupported};

/// `PostActionType::LIKE_POST_ACTION_ID`
pub const LIKE_POST_ACTION_ID: i64 = 2;
/// `Flag::MAX_SYSTEM_FLAG_ID`
pub const MAX_SYSTEM_FLAG_ID: i64 = 1000;
const MAX_NAME_LENGTH: usize = 200;
const MAX_DESCRIPTION_LENGTH: usize = 1000;

/// `Flag.valid_applies_to_types`: the defaults and chat's.
pub const VALID_APPLIES_TO: [&str; 3] = ["Post", "Topic", "Chat::Message"];

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Flag {
    pub id: i64,
    pub name: Option<String>,
    pub name_key: Option<String>,
    pub description: Option<String>,
    pub applies_to: Vec<String>,
    pub position: i32,
    pub require_message: bool,
    pub enabled: bool,
    pub score_type: bool,
    pub auto_action_type: bool,
}

impl Flag {
    pub fn system(&self) -> bool {
        self.id < MAX_SYSTEM_FLAG_ID
    }
}

pub const COLUMNS: &str = "id, name, name_key, description, applies_to, position, \
    require_message, enabled, score_type, auto_action_type";

/// Flag's default scope: no score types, no like, by position.
const DEFAULT_SCOPE: &str = "NOT score_type AND id <> 2";

/// `Flag.find_by(id:)`, under the default scope.
pub async fn find(conn: &mut PgConnection, id: i64) -> Result<Option<Flag>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM flags WHERE {DEFAULT_SCOPE} AND id = $1 LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(conn)
    .await
}

/// `Flag#used?`
pub async fn used(conn: &mut PgConnection, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM post_actions WHERE post_action_type_id = $1) \
             OR EXISTS (SELECT 1 FROM reviewable_scores WHERE reviewable_score_type = $1)",
    )
    .bind(id as i32)
    .fetch_one(conn)
    .await
}

/// FlagSerializer, with its `post_action` (or `topic_flag`) translations
/// for system flags and the row's own words for custom ones.
pub fn serialize(i18n: &I18n, base_path: &str, flag: &Flag, target: &str, is_used: bool) -> Value {
    let name_key = flag.name_key.clone().unwrap_or_default();
    let prefix = format!("{target}_types.{name_key}");
    let base_path = [("base_path", base_path)];
    // I18n.t without arguments leaves placeholders literal; with
    // base_path passed, a missing placeholder falls back to the default.
    let name = i18n
        .t(&format!("{prefix}.title"))
        .map(str::to_string)
        .or_else(|| flag.name.clone());
    let description = i18n
        .t_with(&format!("{prefix}.description"), &base_path)
        .unwrap_or_else(|| flag.description.clone().unwrap_or_default());
    let short_description = i18n
        .t_with(&format!("{prefix}.short_description"), &base_path)
        .unwrap_or_default();
    json!({
        "id": flag.id,
        "name": name,
        "name_key": flag.name_key,
        "description": description,
        "short_description": short_description,
        "applies_to": flag.applies_to,
        "position": flag.position,
        "require_message": flag.require_message,
        "enabled": flag.enabled,
        "is_flag": !flag.score_type && flag.id != LIKE_POST_ACTION_ID,
        "is_used": is_used,
        "auto_action_type": flag.auto_action_type,
        "system": flag.system(),
    })
}

/// The create and update contract's attributes.
#[derive(Debug, Default)]
pub struct Params {
    pub name: Option<String>,
    pub description: Option<String>,
    pub require_message: Option<bool>,
    pub enabled: Option<bool>,
    pub applies_to: Option<Vec<String>>,
    pub auto_action_type: Option<bool>,
}

impl Params {
    /// The contract's validations' full messages, in order.
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let blank = |v: &Option<String>| v.as_deref().is_none_or(crate::ruby::is_blank);
        if blank(&self.name) {
            errors.push("Name can't be blank".to_string());
        }
        if blank(&self.description) {
            errors.push("Description can't be blank".to_string());
        }
        if self
            .name
            .as_deref()
            .is_some_and(|n| n.chars().count() > MAX_NAME_LENGTH)
        {
            errors.push(format!(
                "Name is too long (maximum is {MAX_NAME_LENGTH} characters)"
            ));
        }
        if self
            .description
            .as_deref()
            .is_some_and(|d| d.chars().count() > MAX_DESCRIPTION_LENGTH)
        {
            errors.push(format!(
                "Description is too long (maximum is {MAX_DESCRIPTION_LENGTH} characters)"
            ));
        }
        let valid = self
            .applies_to
            .as_ref()
            .is_some_and(|a| a.iter().all(|t| VALID_APPLIES_TO.contains(&t.as_str())));
        if !valid {
            errors.push("Applies to is not included in the list".to_string());
        }
        errors
    }

    /// The booleans Rails would write as NULL (a NotNullViolation).
    fn booleans(&self) -> Result<(bool, bool, bool), Unsupported> {
        match (self.require_message, self.enabled, self.auto_action_type) {
            (Some(r), Some(e), Some(a)) => Ok((r, e, a)),
            _ => Err(Unsupported(
                "a flag without require_message, enabled or auto_action_type (NotNullViolation)",
            )),
        }
    }
}

/// `Flag.custom.where(name:)` (less `id`) exists.
pub async fn name_taken(
    conn: &mut PgConnection,
    name: &str,
    except: Option<i64>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT EXISTS (SELECT 1 FROM flags WHERE {DEFAULT_SCOPE} AND id >= 1000 AND name = $1 \
           AND id IS DISTINCT FROM $2)"
    ))
    .bind(name)
    .bind(except)
    .fetch_one(conn)
    .await
}

/// `Flag.custom.count`
pub async fn custom_count(conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM flags WHERE {DEFAULT_SCOPE} AND id >= 1000"
    ))
    .fetch_one(conn)
    .await
}

/// `set_name_key`: `custom_` (for a custom flag) and the name squeezed,
/// spaces as underscores, non-word characters out, downcased; `_2`, `_3`..
/// while another flag has it.
async fn name_key_for(
    conn: &mut PgConnection,
    id: Option<i64>,
    name: &str,
    system: bool,
) -> Result<String, AppError> {
    let mut squeezed = String::new();
    for c in name.chars() {
        if c == ' ' && squeezed.ends_with(' ') {
            continue;
        }
        squeezed.push(c);
    }
    if !name.is_ascii() {
        return Err(Unsupported("a flag name_key from a non-ASCII name (Ruby's \\w)").into());
    }
    let slug: String = squeezed
        .replace(' ', "_")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase();
    let prefix = if system { "" } else { "custom_" };
    let base = if slug.is_empty() {
        format!("{prefix}flag")
    } else {
        format!("{prefix}{slug}")
    };
    let mut candidate = base.clone();
    let mut suffix = 1;
    loop {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM flags WHERE name_key = $1 AND id IS DISTINCT FROM $2)",
        )
        .bind(&candidate)
        .bind(id)
        .fetch_one(&mut *conn)
        .await?;
        if !taken {
            return Ok(candidate);
        }
        suffix += 1;
        candidate = format!("{base}_{suffix}");
    }
}

/// Ruby's `Array#to_s` of strings.
fn inspect_array(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|s| crate::ruby::inspect_string(s))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn details_of(flag: &Flag, with_require_message: bool) -> Vec<(&'static str, String)> {
    let mut details = vec![
        ("name", flag.name.clone().unwrap_or_default()),
        ("description", flag.description.clone().unwrap_or_default()),
        ("applies_to", inspect_array(&flag.applies_to)),
    ];
    if with_require_message {
        details.push(("require_message", flag.require_message.to_string()));
    }
    details.push(("enabled", flag.enabled.to_string()));
    details
}

/// CreateFlag's model and steps: `Flag.new(params.merge(notify_type:
/// true))`, saved, logged.
pub async fn create(conn: &mut PgConnection, actor: i32, p: &Params) -> Result<Flag, AppError> {
    let (require_message, enabled, auto_action_type) = p.booleans()?;
    let name = p.name.clone().unwrap_or_default();
    // before_save :set_position (the default scope's maximum).
    let max: Option<i32> = sqlx::query_scalar(&format!(
        "SELECT MAX(position) FROM flags WHERE {DEFAULT_SCOPE}"
    ))
    .fetch_one(&mut *conn)
    .await?;
    let id: i64 = sqlx::query_scalar("SELECT nextval('flags_id_seq')")
        .fetch_one(&mut *conn)
        .await?;
    // Before the insert the record has no id: `system?` is false and
    // `where.not(id: nil)` takes every flag.
    let name_key = name_key_for(&mut *conn, None, &name, false).await?;
    sqlx::query(
        "INSERT INTO flags (id, name, name_key, description, notify_type, auto_action_type, applies_to, \
           position, enabled, require_message, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, TRUE, $5, $6, $7, $8, $9, now(), now())",
    )
    .bind(id)
    .bind(&name)
    .bind(&name_key)
    .bind(&p.description)
    .bind(auto_action_type)
    .bind(p.applies_to.clone().unwrap_or_default())
    .bind(max.unwrap_or(0) + 1)
    .bind(enabled)
    .bind(require_message)
    .execute(&mut *conn)
    .await?;
    let flag = find(&mut *conn, id)
        .await?
        .ok_or(Unsupported("a flag gone once created"))?;
    crate::staff_action_logs::log_custom(
        conn,
        actor,
        "create_flag",
        &details_of(&flag, true),
        None,
        None,
    )
    .await?;
    Ok(flag)
}

/// UpdateFlag's steps: `update!(**params)` (a new name, a new name_key),
/// logged.
pub async fn update(
    conn: &mut PgConnection,
    actor: i32,
    flag: &Flag,
    p: &Params,
) -> Result<Flag, AppError> {
    let (require_message, enabled, auto_action_type) = p.booleans()?;
    let name = p.name.clone().unwrap_or_default();
    let name_key = if flag.name_key.as_deref().is_some_and(|k| !k.is_empty())
        && flag.name.as_deref() == Some(name.as_str())
    {
        flag.name_key.clone().unwrap_or_default()
    } else {
        name_key_for(&mut *conn, Some(flag.id), &name, flag.system()).await?
    };
    let applies_to = p.applies_to.clone().unwrap_or_default();
    let changed = flag.name.as_deref() != Some(name.as_str())
        || flag.description != p.description
        || flag.require_message != require_message
        || flag.enabled != enabled
        || flag.applies_to != applies_to
        || flag.auto_action_type != auto_action_type
        || flag.name_key.as_deref() != Some(name_key.as_str());
    if changed {
        sqlx::query(
            "UPDATE flags SET name = $2, name_key = $3, description = $4, require_message = $5, \
               enabled = $6, applies_to = $7, auto_action_type = $8, updated_at = now() WHERE id = $1",
        )
        .bind(flag.id)
        .bind(&name)
        .bind(&name_key)
        .bind(&p.description)
        .bind(require_message)
        .bind(enabled)
        .bind(&applies_to)
        .bind(auto_action_type)
        .execute(&mut *conn)
        .await?;
    }
    let flag = find(&mut *conn, flag.id)
        .await?
        .ok_or(Unsupported("a flag gone once updated"))?;
    crate::staff_action_logs::log_custom(
        conn,
        actor,
        "update_flag",
        &details_of(&flag, true),
        None,
        None,
    )
    .await?;
    Ok(flag)
}

/// DestroyFlag's steps.
pub async fn destroy(conn: &mut PgConnection, actor: i32, flag: &Flag) -> Result<(), AppError> {
    sqlx::query("DELETE FROM flags WHERE id = $1")
        .bind(flag.id)
        .execute(&mut *conn)
        .await?;
    crate::staff_action_logs::log_custom(
        conn,
        actor,
        "delete_flag",
        &details_of(flag, false),
        None,
        None,
    )
    .await?;
    Ok(())
}

/// ToggleFlag's steps.
pub async fn toggle(conn: &mut PgConnection, actor: i32, flag: &Flag) -> Result<(), AppError> {
    let enabled = !flag.enabled;
    sqlx::query("UPDATE flags SET enabled = $2, updated_at = now() WHERE id = $1")
        .bind(flag.id)
        .bind(enabled)
        .execute(&mut *conn)
        .await?;
    crate::staff_action_logs::log_custom(
        conn,
        actor,
        "toggle_flag",
        &[
            ("flag", flag.name.clone().unwrap_or_default()),
            ("enabled", enabled.to_string()),
        ],
        None,
        None,
    )
    .await?;
    Ok(())
}

/// ReorderFlag's all_flags, invalid_move policy and move: None when the
/// move is invalid.
pub async fn reorder(
    conn: &mut PgConnection,
    actor: i32,
    flag: &Flag,
    direction: &str,
) -> Result<Option<()>, AppError> {
    // Flag.where.not(name_key: "notify_user").order(:position), under the
    // default scope (a NULL name_key is left out too).
    let all: Vec<(i64, i32)> = sqlx::query_as(&format!(
        "SELECT id, position FROM flags WHERE {DEFAULT_SCOPE} AND name_key <> 'notify_user' \
         ORDER BY position"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let Some(index) = all.iter().position(|(id, _)| *id == flag.id) else {
        return Err(Unsupported("moving a flag outside the list (nil index)").into());
    };
    let up = direction == "up";
    if (up && index == 0) || (!up && index + 1 == all.len()) {
        return Ok(None);
    }
    let target = all[if up { index - 1 } else { index + 1 }];
    for (id, position) in [(flag.id, target.1), (target.0, flag.position)] {
        sqlx::query(
            "UPDATE flags SET position = $2, updated_at = now() WHERE id = $1 AND position <> $2",
        )
        .bind(id)
        .bind(position)
        .execute(&mut *conn)
        .await?;
    }
    crate::staff_action_logs::log_custom(
        conn,
        actor,
        "move_flag",
        &[
            ("flag", flag.name.clone().unwrap_or_default()),
            ("direction", direction.to_string()),
        ],
        None,
        None,
    )
    .await?;
    Ok(Some(()))
}
