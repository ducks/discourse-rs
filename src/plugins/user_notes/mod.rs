//! discourse-user-notes: staff notes on users, kept as Rails' plugin
//! keeps them, one JSON array per user in `plugin_store_rows`
//! (`user_notes`, `notes:<user id>`), with the count in the
//! `user_notes_count` user custom field. UserNotesController is
//! src/routes/user_notes.rs, the `user_notes` report is `report`, and the
//! components on the server-rendered pages are `view`.

pub mod report;
pub mod view;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// The plugin's PluginStore name.
const STORE: &str = "user_notes";

/// `DiscourseUserNotes::COUNT_FIELD`, a staff user custom field
/// (`allow_staff_user_custom_field`).
pub const COUNT_FIELD: &str = "user_notes_count";

/// `Discourse::SYSTEM_USER_ID`
const SYSTEM_USER_ID: i32 = -1;

/// `enabled_site_setting :user_notes_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("user_notes_enabled")?.truthy())
}

/// `DiscourseUserNotes.key_for(user_id)`, with the id as it was given.
pub fn key_for(user_id: &str) -> String {
    format!("notes:{user_id}")
}

/// `Guardian#can_delete_user_notes?`
pub fn can_delete(settings: &SiteSettings, guardian: &Guardian) -> Result<bool, SettingError> {
    Ok(
        (settings.get("user_notes_moderators_delete")?.truthy() && guardian.is_staff())
            || guardian.is_admin(),
    )
}

/// `DiscourseUserNotes.notes_for(user_id)`: the stored notes, oldest
/// first, or none.
pub async fn notes_for(
    conn: &mut PgConnection,
    user_id: &str,
) -> Result<Vec<Map<String, Value>>, AppError> {
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT type_name, value FROM plugin_store_rows WHERE plugin_name = $1 AND key = $2",
    )
    .bind(STORE)
    .bind(key_for(user_id))
    .fetch_optional(&mut *conn)
    .await?;
    let Some((type_name, value)) = row else {
        return Ok(Vec::new());
    };
    // PluginStore.cast_value: anything but a JSON array of hashes is not
    // something the plugin wrote.
    let notes = match (type_name.as_deref(), value) {
        (Some("JSON"), Some(value)) => serde_json::from_str::<Value>(&value).ok(),
        _ => None,
    };
    let Some(Value::Array(notes)) = notes else {
        return Err(Unsupported("user notes stored as something other than a JSON array").into());
    };
    notes
        .into_iter()
        .map(|n| match n {
            Value::Object(m) => Ok(m),
            _ => Err(Unsupported("a user note that is not a hash").into()),
        })
        .collect()
}

/// `PluginStore.set` (or `.remove` for no notes) and the count field, as
/// `add_note` and `remove_note` leave them.
async fn store(
    conn: &mut PgConnection,
    user_id: i32,
    notes: &[Map<String, Value>],
) -> Result<(), AppError> {
    let key = key_for(&user_id.to_string());
    if notes.is_empty() {
        sqlx::query("DELETE FROM plugin_store_rows WHERE plugin_name = $1 AND key = $2")
            .bind(STORE)
            .bind(&key)
            .execute(&mut *conn)
            .await?;
    } else {
        let value = serde_json::to_string(notes).expect("JSON values serialize");
        let updated = sqlx::query(
            "UPDATE plugin_store_rows SET type_name = 'JSON', value = $3 \
             WHERE plugin_name = $1 AND key = $2",
        )
        .bind(STORE)
        .bind(&key)
        .bind(&value)
        .execute(&mut *conn)
        .await?;
        if updated.rows_affected() == 0 {
            sqlx::query(
                "INSERT INTO plugin_store_rows (plugin_name, key, type_name, value) \
                 VALUES ($1, $2, 'JSON', $3)",
            )
            .bind(STORE)
            .bind(&key)
            .bind(&value)
            .execute(&mut *conn)
            .await?;
        }
    }
    save_count(conn, user_id, notes.len()).await
}

/// `user.custom_fields[COUNT_FIELD] = n; user.save_custom_fields`: a row
/// already holding the value stays, every other row of the field goes.
async fn save_count(conn: &mut PgConnection, user_id: i32, count: usize) -> Result<(), AppError> {
    let value = count.to_string();
    let kept: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM user_custom_fields WHERE user_id = $1 AND name = $2 AND value = $3 \
         ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(COUNT_FIELD)
    .bind(&value)
    .fetch_optional(&mut *conn)
    .await?;
    sqlx::query(
        "DELETE FROM user_custom_fields WHERE user_id = $1 AND name = $2 \
         AND id IS DISTINCT FROM $3",
    )
    .bind(user_id)
    .bind(COUNT_FIELD)
    .bind(kept)
    .execute(&mut *conn)
    .await?;
    if kept.is_none() {
        sqlx::query(
            "INSERT INTO user_custom_fields (user_id, name, value, created_at, updated_at) \
             VALUES ($1, $2, $3, now(), now()) \
             ON CONFLICT (name, user_id) WHERE name::text = 'user_notes_count'::text \
             DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(user_id)
        .bind(COUNT_FIELD)
        .bind(&value)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `Time.now` as the plugin's record holds it once serialized.
fn now_json() -> String {
    crate::topic_list::time_json(crate::clock::now().naive_utc())
}

/// `DiscourseUserNotes.add_note(user, raw, created_by, opts)`: the new
/// record, after the stored ones. `extras` are merged over the record as
/// given (`post_id`, `reviewable_id`, `topic_id`).
pub async fn add_note(
    conn: &mut PgConnection,
    user_id: i32,
    raw: Value,
    created_by: i32,
    extras: Map<String, Value>,
) -> Result<Map<String, Value>, AppError> {
    let mut notes = notes_for(&mut *conn, &user_id.to_string()).await?;
    let mut record = Map::new();
    record.insert("id".into(), json!(crate::accounts::random_hex()));
    record.insert("user_id".into(), json!(user_id));
    record.insert("raw".into(), raw);
    record.insert("created_by".into(), json!(created_by));
    record.insert("created_at".into(), json!(now_json()));
    for (k, v) in extras {
        record.insert(k, v);
    }
    notes.push(record.clone());
    store(conn, user_id, &notes).await?;
    Ok(record)
}

/// `DiscourseUserNotes.remove_note(user, note_id)`. Rails compares the
/// stored ids with `n[:id] == note_id`, a string against the param.
pub async fn remove_note(
    conn: &mut PgConnection,
    user_id: i32,
    note_id: &str,
) -> Result<(), AppError> {
    let mut notes = notes_for(&mut *conn, &user_id.to_string()).await?;
    notes.retain(|n| n.get("id").and_then(Value::as_str) != Some(note_id));
    store(conn, user_id, &notes).await
}

/// What the serializer reads besides a note.
pub struct Cx<'a> {
    pub settings: &'a SiteSettings,
    pub urls: &'a Urls<'a>,
    pub guardian: &'a Guardian,
    /// The site's small logo, the system user's avatar under
    /// use_site_small_logo_as_system_avatar.
    pub logo_small_url: Option<&'a str>,
}

/// A note's post as the serializer reads it: `post.url` and the topic id
/// and number it falls back to.
struct NotePost {
    url: String,
}

/// `Post.with_deleted.find_by(id:)`, kept only when the guardian can see
/// it (`post && guardian.can_see_post?(post)`).
async fn visible_post(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    post_id: i64,
) -> Result<Option<NotePost>, AppError> {
    let Ok(post_id) = i32::try_from(post_id) else {
        return Ok(None);
    };
    let Some(post) = crate::topic_guardian::PostCtx::load(&mut *conn, post_id).await? else {
        return Ok(None);
    };
    let row: (Option<i32>,) = sqlx::query_as("SELECT topic_id FROM posts WHERE id = $1")
        .bind(post_id)
        .fetch_one(&mut *conn)
        .await?;
    // post.topic: the default scope leaves out a trashed topic.
    let topic: Option<(Option<String>,)> = match row.0 {
        Some(topic_id) => {
            sqlx::query_as("SELECT slug FROM topics WHERE id = $1 AND deleted_at IS NULL")
                .bind(topic_id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    // can_see_post?: an admin sees everything, anyone else a post of a
    // topic they can see (can_see_post_topic?).
    let topic_can_see = match (row.0, &topic) {
        (Some(topic_id), Some(_)) => {
            match crate::topic_guardian::TopicCtx::load(
                &mut *conn,
                cx.settings,
                cx.guardian,
                topic_id,
            )
            .await?
            {
                Some(t) => {
                    let secure = cx
                        .guardian
                        .secure_category_ids(&mut *conn, cx.settings)
                        .await?;
                    cx.guardian.can_see_topic(cx.settings, &t, true, &secure)?
                }
                None => false,
            }
        }
        _ => false,
    };
    if !cx
        .guardian
        .can_see_post(cx.settings, &post, topic_can_see)?
    {
        return Ok(None);
    }
    let base = cx.urls.config.globals.relative_url_root();
    // Post#url, "/404" without a topic, which the serializer turns into
    // the slugless path.
    let url = match (row.0, topic) {
        (Some(topic_id), Some((Some(slug),))) if !slug.is_empty() => {
            format!("{base}/t/{slug}/{topic_id}/{}", post.post_number)
        }
        (Some(_), Some(_)) => {
            return Err(Unsupported("a user note's post in a topic without a stored slug").into());
        }
        (topic_id, _) => format!(
            "{base}/t/{}/{}",
            topic_id.map(|t| t.to_string()).unwrap_or_default(),
            post.post_number
        ),
    };
    Ok(Some(NotePost { url }))
}

/// `BasicUserSerializer` for a note's author: Rails raises on an author
/// no longer there.
async fn created_by(conn: &mut PgConnection, cx: &Cx<'_>, id: &Value) -> Result<Value, AppError> {
    let id = match id {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    let user: Option<(i32, String, Option<String>, Option<i32>)> = match id {
        Some(id) => {
            sqlx::query_as("SELECT id, username, name, uploaded_avatar_id FROM users WHERE id = $1")
                .bind(i32::try_from(id).unwrap_or(0))
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let Some((id, username, name, avatar)) = user else {
        return Err(Unsupported("a user note whose author was deleted (Rails raises)").into());
    };
    let mut u = Map::new();
    u.insert("id".into(), json!(id));
    u.insert("username".into(), json!(username));
    if cx.settings.get("enable_names")?.truthy() {
        u.insert("name".into(), json!(name));
    }
    u.insert(
        "avatar_template".into(),
        json!(crate::avatar::avatar_template(
            cx.urls,
            id,
            &username,
            avatar,
            cx.logo_small_url
        )?),
    );
    Ok(Value::Object(u))
}

/// A post id as given (`o[:post_id].to_i`), visible to the guardian:
/// nil and "" are 0, which no post has.
async fn post_of(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    post_id: Option<&Value>,
) -> Result<Option<NotePost>, AppError> {
    let id = match post_id {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => Some(crate::ruby::to_i(s)),
        _ => None,
    };
    match id {
        Some(id) if id != 0 => visible_post(conn, cx, id).await,
        _ => Ok(None),
    }
}

/// `post && guardian.can_see_post?(post)` for a post id param.
pub async fn post_visible(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    post_id: &Value,
) -> Result<bool, AppError> {
    Ok(post_of(conn, cx, Some(post_id)).await?.is_some())
}

/// `UserNoteSerializer` for one note, its post looked up and kept only
/// when visible.
pub async fn serialize(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    note: &Map<String, Value>,
) -> Result<Value, AppError> {
    let get = |k: &str| note.get(k).cloned().unwrap_or(Value::Null);
    let post = post_of(&mut *conn, cx, note.get("post_id")).await?;
    let mut out = Map::new();
    out.insert("id".into(), get("id"));
    out.insert("user_id".into(), get("user_id"));
    out.insert("raw".into(), get("raw"));
    out.insert(
        "created_by".into(),
        created_by(&mut *conn, cx, &get("created_by")).await?,
    );
    out.insert("created_at".into(), get("created_at"));
    out.insert(
        "can_delete".into(),
        json!(can_delete(cx.settings, cx.guardian)?),
    );
    if let Some(post) = post {
        out.insert("post_id".into(), get("post_id"));
        out.insert("post_url".into(), json!(post.url));
        // `post.try(:title)`: Post has no title.
        out.insert("post_title".into(), Value::Null);
    }
    out.insert("reviewable_id".into(), get("reviewable_id"));
    Ok(Value::Object(out))
}

/// UserNotesController#index's document: the user's notes, newest first.
pub async fn index_doc(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    user_id_param: &str,
    username: &str,
) -> Result<Value, AppError> {
    let notes = notes_for(&mut *conn, user_id_param).await?;
    let mut out = Vec::with_capacity(notes.len());
    for note in notes.iter().rev() {
        out.push(serialize(&mut *conn, cx, note).await?);
    }
    Ok(json!({ "extras": { "username": username }, "user_notes": out }))
}

/// The `user_notes_count` custom field (`custom_fields["user_notes_count"]
/// .to_i`).
pub async fn count(conn: &mut PgConnection, user_id: i32) -> Result<i64, sqlx::Error> {
    let value: Option<Option<String>> = sqlx::query_scalar(
        "SELECT value FROM user_custom_fields WHERE user_id = $1 AND name = $2 ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(COUNT_FIELD)
    .fetch_optional(conn)
    .await?;
    Ok(value.flatten().map(|v| crate::ruby::to_i(&v)).unwrap_or(0))
}

/// `add_to_serializer(:admin_detailed_user, :user_notes_count)`, while
/// the plugin is enabled.
pub async fn admin_detailed_user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    user_id: i32,
    out: &mut Map<String, Value>,
) -> Result<(), AppError> {
    if enabled(settings)? {
        out.insert(COUNT_FIELD.into(), json!(count(conn, user_id).await?));
    }
    Ok(())
}

/// A UserWarning row as its after_commit callback reads it.
pub struct UserWarning {
    pub user_id: i32,
    pub created_by_id: i32,
    pub topic_id: i32,
}

/// `add_model_callback(UserWarning, :after_commit, on: :create)`: the
/// note recording the official warning, in the site's default locale,
/// by the system user. Rails then triggers `:user_warning_created`, which
/// no plugin the port runs listens to.
pub async fn user_warning_created(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    i18n: &crate::i18n::I18n,
    warning: &UserWarning,
) -> Result<(), AppError> {
    if !enabled(settings)? {
        return Ok(());
    }
    let created_by: Option<String> = sqlx::query_scalar("SELECT username FROM users WHERE id = $1")
        .bind(warning.created_by_id)
        .fetch_optional(&mut *conn)
        .await?;
    let topic: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT title, slug FROM topics WHERE id = $1 AND deleted_at IS NULL")
            .bind(warning.topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let user_exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
        .bind(warning.user_id)
        .fetch_one(&mut *conn)
        .await?;
    let (Some(created_by), Some((title, slug)), true) = (created_by, topic, user_exists) else {
        return Err(Unsupported("an official warning without its user, author or topic").into());
    };
    // Topic#url
    let mut url = format!("{}/t/", urls.base_url()?);
    if let Some(slug) = slug.filter(|s| !s.is_empty()) {
        url.push_str(&slug);
        url.push('/');
    }
    url.push_str(&warning.topic_id.to_string());
    let locale = settings.get("default_locale")?.to_s();
    let raw = i18n
        .locale(&locale)
        .t_with(
            "user_notes.official_warning",
            &[
                ("username", &created_by),
                ("warning_link", &format!("[{title}]({url})")),
            ],
        )
        .ok_or(Unsupported("user_notes.official_warning untranslated"))?;
    let mut extras = Map::new();
    extras.insert("topic_id".into(), json!(warning.topic_id));
    add_note(conn, warning.user_id, json!(raw), SYSTEM_USER_ID, extras).await?;
    Ok(())
}

/// The time a note was created, for the report: `Time.parse`.
pub(crate) fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(s).ok().or_else(|| {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
            .ok()
            .map(|t| t.and_utc().fixed_offset())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_the_id_as_given() {
        assert_eq!(key_for("12"), "notes:12");
    }

    #[test]
    fn times_parse_as_ruby_reads_them() {
        let t = parse_time("2026-10-28T12:34:56.789Z").unwrap();
        assert_eq!(t.timestamp_millis() % 1000, 789);
        assert!(parse_time("2026-10-28T12:34:56.789+00:00").is_some());
        assert!(parse_time("nonsense").is_none());
    }
}
