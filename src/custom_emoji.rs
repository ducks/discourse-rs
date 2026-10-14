//! Custom emoji: Admin::EmojiController's index, create and destroy
//! (CustomEmoji::Create, CustomEmoji::Destroy), StaffActionLogger's two
//! custom emoji entries and Jobs::RebakeCustomEmojiPosts.
//!
//! The emoji drawn everywhere come from `SiteSettings::emojis`
//! (crate::emojis::Emojis), which reads the table as of its load.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;
use crate::emojis::Emojis;

/// The job CustomEmoji::Destroy enqueues.
pub const REBAKE_JOB: &str = "rebake_custom_emoji_posts";

/// `CustomEmoji::MAX_GROUP_LENGTH`
const MAX_GROUP_LENGTH: usize = 20;

/// GET /admin/config/emoji: `Emoji.custom` with the default group first,
/// then by group and name, case-insensitively, as EmojiSerializer gives
/// them.
pub fn admin_index(emojis: &Emojis) -> Value {
    let mut list: Vec<&crate::emojis::Custom> = emojis.custom.iter().collect();
    list.sort_by_key(|e| {
        (
            u8::from(e.group != "default"),
            e.group.to_lowercase(),
            e.name.to_lowercase(),
        )
    });
    Value::Array(
        list.into_iter()
            .map(|e| {
                json!({
                    "name": e.name,
                    "url": e.cdn_url,
                    "group": e.group,
                    "created_by": e.created_by,
                })
            })
            .collect(),
    )
}

/// `File.basename(name, ".*")`: the last path segment less its extension.
pub fn basename_without_extension(name: &str) -> &str {
    let base = name.trim_end_matches('/');
    let base = base.rsplit('/').next().unwrap_or(base);
    match base.rfind('.') {
        Some(dot) if dot > 0 => &base[..dot],
        _ => base,
    }
}

/// `Emoji.sanitize_emoji_name`: runs of anything but letters, digits, `+`
/// and `-` become one underscore, as do runs of underscores; lowercase.
pub fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '+' || c == '-' {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    let mut squeezed = String::with_capacity(out.len());
    for c in out.chars() {
        if c == '_' && squeezed.ends_with('_') {
            continue;
        }
        squeezed.push(c);
    }
    squeezed.to_lowercase()
}

/// `UserHistory.actions`
fn action(name: &str) -> Result<i32, AppError> {
    crate::staff_action_logs::action_id(name)
        .ok_or_else(|| crate::Unsupported("a UserHistory action missing from enums.json").into())
}

/// StaffActionLogger#log_custom_emoji_create / _destroy (both in
/// `UserHistory.admin_only_action_ids`).
async fn log(
    conn: &mut PgConnection,
    acting_user_id: i32,
    action_name: &str,
    previous_value: Option<&str>,
    new_value: Option<&str>,
    details: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, previous_value, new_value, details, \
                                     admin_only, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(action(action_name)?)
    .bind(acting_user_id)
    .bind(previous_value)
    .bind(new_value)
    .bind(details)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// StaffActionLogger#log_custom_emoji_create: the group in the details
/// when there is one.
pub async fn log_create(
    conn: &mut PgConnection,
    admin_id: i32,
    name: &str,
    group: Option<&str>,
) -> Result<(), AppError> {
    let details = group
        .filter(|g| !crate::ruby::is_blank(g))
        .map(|g| format!("Group: {g}"));
    log(
        conn,
        admin_id,
        "custom_emoji_create",
        None,
        Some(name),
        details.as_deref(),
    )
    .await
}

/// What CustomEmoji::Create's transaction ends with.
pub enum Created {
    Emoji(Value),
    /// The record's validation messages.
    Invalid(Vec<String>),
}

/// CustomEmoji::Create past its upload: the record (validated as the
/// model does), its upload reference and the staff log, in one
/// transaction.
pub async fn create(
    conn: &mut PgConnection,
    i18n: &crate::i18n::I18n,
    admin_id: i32,
    name: &str,
    group: Option<&str>,
    upload_id: i32,
) -> Result<Created, AppError> {
    let t = |key: &str| i18n.t(key).unwrap_or(key).to_string();
    // errors.full_messages: the attribute's human name, then the message.
    let mut errors = Vec::new();
    if crate::ruby::is_blank(name) {
        errors.push(format!("Name {}", t("errors.messages.blank")));
    } else {
        let taken: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM custom_emojis WHERE name = $1)")
                .bind(name)
                .fetch_one(&mut *conn)
                .await?;
        if taken {
            errors.push(format!(
                "Name {}",
                t("activerecord.errors.models.custom_emoji.attributes.name.taken")
            ));
        }
    }
    if group.is_some_and(|g| g.chars().count() > MAX_GROUP_LENGTH) {
        let message = i18n
            .t_count("errors.messages.too_long", MAX_GROUP_LENGTH as i64, &[])
            .unwrap_or_default();
        errors.push(format!("Group {message}"));
    }
    if !errors.is_empty() {
        return Ok(Created::Invalid(errors));
    }
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO custom_emojis (name, upload_id, \"group\", user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(name)
    .bind(upload_id)
    .bind(group)
    .bind(admin_id)
    .fetch_one(&mut *conn)
    .await?;
    // UploadReference.ensure_exist!
    sqlx::query(
        "INSERT INTO upload_references (upload_id, target_type, target_id, created_at, updated_at) \
         VALUES ($1, 'CustomEmoji', $2, clock_timestamp(), clock_timestamp()) \
         ON CONFLICT DO NOTHING",
    )
    .bind(upload_id)
    .bind(id)
    .execute(&mut *conn)
    .await?;
    log_create(conn, admin_id, name, group).await?;
    let url: String = sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
        .bind(upload_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(Created::Emoji(
        json!({ "name": name, "url": url, "group": group }),
    ))
}

/// CustomEmoji::Destroy: the emoji of that name logged and removed with
/// its upload references (the upload stays for clean_up_uploads), and
/// whether or not there was one, the posts using it rebaked.
pub async fn destroy(conn: &mut PgConnection, admin_id: i32, name: &str) -> Result<(), AppError> {
    let id: Option<i32> = sqlx::query_scalar("SELECT id FROM custom_emojis WHERE name = $1")
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(id) = id {
        log(
            conn,
            admin_id,
            "custom_emoji_destroy",
            Some(name),
            None,
            None,
        )
        .await?;
        sqlx::query(
            "DELETE FROM upload_references WHERE target_type = 'CustomEmoji' AND target_id = $1",
        )
        .bind(id)
        .execute(&mut *conn)
        .await?;
        sqlx::query("DELETE FROM custom_emojis WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    crate::jobs::enqueue(conn, REBAKE_JOB, json!({ "name": name })).await?;
    Ok(())
}

/// Jobs::RebakeCustomEmojiPosts: every post whose raw has `:<name>:`
/// (a LIKE pattern, so `_` matches any character) rebaked, by id.
pub async fn rebake_posts(state: &crate::AppState, args: &Value) -> Result<(), AppError> {
    let name = match args.get("name") {
        Some(Value::String(name)) => name.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let mut conn = state.pool.acquire().await?;
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM posts WHERE deleted_at IS NULL AND raw LIKE $1 ORDER BY id",
    )
    .bind(format!("%:{name}:%"))
    .fetch_all(&mut *conn)
    .await?;
    if ids.is_empty() {
        return Ok(());
    }
    let settings = crate::site_settings::SiteSettings::load(
        &mut conn,
        &state.site_setting_defs,
        &state.config.globals,
    )
    .await?;
    drop(conn);
    let host = crate::pretty_text::Host::from_state(state);
    let ctx = crate::posting::Ctx {
        host: &host,
        settings: &settings,
        config: &state.config,
        i18n: &state.i18n,
        bus: &state.bus,
    };
    for id in ids {
        crate::posting::rebake::rebake(&state.pool, &ctx, id, None).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_from_files() {
        assert_eq!(
            basename_without_extension("party parrot.gif"),
            "party parrot"
        );
        assert_eq!(basename_without_extension("a/b/c.tar.gz"), "c.tar");
        assert_eq!(basename_without_extension(".hidden"), ".hidden");
        assert_eq!(sanitize_name("Party  Parrot!!"), "party_parrot_");
        assert_eq!(sanitize_name("a__b+c-D"), "a_b+c-d");
        assert_eq!(sanitize_name("_x_"), "_x_");
    }
}
