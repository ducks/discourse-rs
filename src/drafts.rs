//! Drafts: `Draft.set`, `Draft.get` and `Draft.clear` as DraftsController
//! calls them, with the draft sequence that guards them and the user's
//! draft count.
//!
//! Refused: drafts backed up to a message (backup_drafts_to_pm_length),
//! drafts holding uploads (UploadReference), the edit conflict check on a
//! first post's tags, reading a draft at a given sequence, and acting on
//! another user's drafts.

use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// How saving a draft ends.
pub enum Saved {
    /// The draft sequence now.
    Sequence(i64),
    /// `Draft::OutOfSequence`
    OutOfSequence,
}

async fn current_sequence(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
) -> Result<i64, sqlx::Error> {
    let seq: Option<i64> = sqlx::query_scalar(
        "SELECT sequence FROM draft_sequences WHERE user_id = $1 AND draft_key = $2",
    )
    .bind(user_id)
    .bind(key)
    .fetch_optional(conn)
    .await?;
    Ok(seq.unwrap_or(0))
}

/// `UserStat.update_draft_count(user_id)`
async fn update_draft_count(conn: &mut PgConnection, user_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE user_stats SET draft_count = (SELECT COUNT(*) FROM drafts WHERE user_id = $1) WHERE user_id = $1",
    )
    .bind(user_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// What is not ported about a draft's data.
fn refuse_data(data: &str) -> Result<(), Unsupported> {
    if data.contains("upload://") || data.contains("/uploads/") {
        return Err(Unsupported("drafts with uploads (UploadReference)"));
    }
    Ok(())
}

/// What a save sends: the draft's key, the sequence the client holds, the
/// data, its owner and `force_save`.
pub struct NewDraft<'a> {
    pub key: &'a str,
    pub sequence: i64,
    pub data: &'a str,
    pub owner: Option<&'a str>,
    pub force_save: bool,
}

/// `Draft.set(user, key, sequence, data, owner, force_save:)`
pub async fn set(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    draft: &NewDraft<'_>,
) -> Result<Saved, AppError> {
    let (key, sequence, data, owner, force_save) = (
        draft.key,
        draft.sequence,
        draft.data,
        draft.owner,
        draft.force_save,
    );
    // User.human_user_id?
    if user_id <= 0 {
        return Ok(Saved::Sequence(0));
    }
    let backup = s.get("backup_drafts_to_pm_length")?.to_i();
    if backup > 0 && (backup as usize) < data.chars().count() {
        return Err(Unsupported("drafts backed up to a message").into());
    }
    refuse_data(data)?;
    let existing: Option<(i32, Option<String>)> =
        sqlx::query_as("SELECT id, owner FROM drafts WHERE user_id = $1 AND draft_key = $2")
            .bind(user_id)
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?;
    let current = current_sequence(&mut *conn, user_id, key).await?;
    let (draft_id, sequence) = if let Some((id, current_owner)) = existing {
        if !force_save && current != sequence {
            return Ok(Saved::OutOfSequence);
        }
        let sequence = if force_save { current } else { sequence } + 1;
        // The sequence moves on with every save.
        sqlx::query(
            "INSERT INTO draft_sequences (user_id, draft_key, sequence) VALUES ($1, $2, $3) \
             ON CONFLICT (user_id, draft_key) DO UPDATE SET sequence = EXCLUDED.sequence",
        )
        .bind(user_id)
        .bind(key)
        .bind(sequence)
        .execute(&mut *conn)
        .await?;
        sqlx::query(
            "UPDATE drafts SET sequence = $2, data = $3, revisions = revisions + 1, owner = $4, \
                               updated_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(sequence)
        .bind(data)
        .bind(owner.map(str::to_string).or(current_owner))
        .execute(&mut *conn)
        .await?;
        (id, sequence)
    } else if sequence != current {
        return Ok(Saved::OutOfSequence);
    } else {
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO drafts (user_id, draft_key, data, sequence, owner, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, now(), now()) \
             ON CONFLICT (user_id, draft_key) DO UPDATE SET sequence = $4, data = $3, \
               revisions = drafts.revisions + 1, owner = $5, updated_at = now() \
             RETURNING id",
        )
        .bind(user_id)
        .bind(key)
        .bind(data)
        .bind(sequence)
        .bind(owner)
        .fetch_one(&mut *conn)
        .await?;
        update_draft_count(&mut *conn, user_id).await?;
        (id, sequence)
    };
    // UploadReference.ensure_exist!: the draft holds no uploads.
    sqlx::query("DELETE FROM upload_references WHERE target_type = 'Draft' AND target_id = $1")
        .bind(draft_id)
        .execute(&mut *conn)
        .await?;
    Ok(Saved::Sequence(sequence))
}

/// DraftsController#create's `Draft.set`, retried at the current sequence
/// when the draft does not exist (the client's sequence ran behind).
pub async fn create(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    draft: &NewDraft<'_>,
) -> Result<Saved, AppError> {
    let key = draft.key;
    match set(conn, s, user_id, draft).await? {
        Saved::OutOfSequence => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM drafts WHERE user_id = $1 AND draft_key = $2)",
            )
            .bind(user_id)
            .bind(key)
            .fetch_one(&mut *conn)
            .await?;
            if exists {
                return Ok(Saved::OutOfSequence);
            }
            let current = current_sequence(&mut *conn, user_id, key).await?;
            let retry = NewDraft {
                sequence: current,
                force_save: false,
                ..*draft
            };
            set(conn, s, user_id, &retry).await
        }
        saved => Ok(saved),
    }
}

/// `reached_max_drafts_per_user?`
pub async fn reached_max(
    conn: &mut PgConnection,
    s: &crate::site_settings::SiteSettings,
    user_id: i32,
    key: &str,
) -> Result<bool, AppError> {
    let (count, exists): (i64, bool) = sqlx::query_as(
        "SELECT COUNT(*), BOOL_OR(draft_key = $2) IS TRUE FROM drafts WHERE user_id = $1",
    )
    .bind(user_id)
    .bind(key)
    .fetch_one(conn)
    .await?;
    Ok(count >= s.get("max_drafts_per_user")?.to_i() && !exists)
}

/// DraftsController#show: `Draft.get` at the current sequence, and that
/// sequence.
pub async fn show(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
) -> Result<(Option<String>, i64), AppError> {
    let current = current_sequence(&mut *conn, user_id, key).await?;
    if user_id <= 0 {
        return Ok((None, current));
    }
    let draft: Option<(String, i64)> =
        sqlx::query_as("SELECT data, sequence FROM drafts WHERE user_id = $1 AND draft_key = $2")
            .bind(user_id)
            .bind(key)
            .fetch_optional(&mut *conn)
            .await?;
    // Only a draft at the current sequence is the user's draft.
    let data = draft.and_then(|(data, seq)| (seq == current).then_some(data));
    Ok((data, current))
}

/// `Draft.clear(user, key, sequence)`: false when the sequence is not the
/// current one (the controller answers success either way).
pub async fn clear(
    conn: &mut PgConnection,
    user_id: i32,
    key: &str,
    sequence: i64,
) -> Result<bool, AppError> {
    if user_id <= 0 {
        return Err(Unsupported("clearing drafts of non-human users").into());
    }
    if current_sequence(&mut *conn, user_id, key).await? != sequence {
        return Ok(false);
    }
    // destroy_all: the upload references go with each draft, then the
    // count on commit.
    let ids: Vec<i32> =
        sqlx::query_scalar("DELETE FROM drafts WHERE user_id = $1 AND draft_key = $2 RETURNING id")
            .bind(user_id)
            .bind(key)
            .fetch_all(&mut *conn)
            .await?;
    if !ids.is_empty() {
        sqlx::query(
            "DELETE FROM upload_references WHERE target_type = 'Draft' AND target_id = ANY($1)",
        )
        .bind(&ids)
        .execute(&mut *conn)
        .await?;
        update_draft_count(&mut *conn, user_id).await?;
    }
    Ok(true)
}

/// `Draft.stream`: the user's drafts at their key's current sequence (or
/// past it), newest first, as DraftSerializer writes them. `guardian` is
/// the user's.
pub async fn stream(
    conn: &mut PgConnection,
    urls: &crate::url::Urls<'_>,
    guardian: &crate::guardian::Guardian,
    user_id: i32,
    offset: i64,
    limit: i64,
) -> Result<Vec<serde_json::Value>, AppError> {
    use serde_json::{Map, Value, json};
    use std::collections::HashMap;

    let settings = urls.settings;
    type DraftRow = (String, i64, String, chrono::NaiveDateTime);
    let drafts: Vec<DraftRow> = sqlx::query_as(
        "SELECT draft_key, sequence, data, created_at FROM drafts \
         WHERE user_id = $1 AND sequence >= COALESCE( \
           (SELECT sequence FROM draft_sequences \
            WHERE draft_sequences.user_id = drafts.user_id \
            AND draft_sequences.draft_key = drafts.draft_key), 0) \
         ORDER BY updated_at DESC, id DESC OFFSET $2 LIMIT $3",
    )
    .bind(user_id)
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;

    // parsed_data, topic_id and post_id
    let parsed: Vec<Map<String, Value>> = drafts
        .iter()
        .map(|(_, _, data, _)| match serde_json::from_str(data) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        })
        .collect();
    let topic_ids: Vec<Option<i32>> = drafts
        .iter()
        .map(|(key, ..)| {
            key.strip_prefix("topic_")
                .map(|rest| crate::ruby::to_i(rest) as i32)
        })
        .collect();
    let post_ids: Vec<Option<i32>> = parsed
        .iter()
        .map(|data| {
            data.get("postId")
                .and_then(Value::as_i64)
                .map(|id| id as i32)
        })
        .collect();

    // preload_data: the topics and posts the user may see.
    let allowed = guardian
        .listable_or_own_messages(&mut *conn, settings, user_id)
        .await?;
    type TopicRow = (
        i32,
        String,
        String,
        Option<i32>,
        bool,
        bool,
        String,
        Option<i32>,
    );
    let wanted_topics: Vec<i32> = topic_ids.iter().flatten().copied().collect();
    let topics: HashMap<i32, TopicRow> = sqlx::query_as::<_, TopicRow>(&format!(
        "SELECT topics.id, topics.title, topics.slug, topics.category_id, topics.closed, \
                topics.archived, topics.archetype, topics.user_id \
         FROM topics WHERE topics.id = ANY($1) AND ({allowed})"
    ))
    .bind(&wanted_topics)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|t| (t.0, t))
    .collect();
    // Post.secured(guardian) joined to the allowed topics.
    let post_types: Vec<i32> = guardian.visible_post_types(settings)?;
    let wanted_posts: Vec<i32> = post_ids.iter().flatten().copied().collect();
    let posts: HashMap<i32, Option<i32>> = sqlx::query_as::<_, (i32, Option<i32>)>(&format!(
        "SELECT posts.id, posts.user_id FROM posts JOIN topics ON topics.id = posts.topic_id \
         WHERE posts.id = ANY($1) AND posts.deleted_at IS NULL AND posts.post_type = ANY($2) \
         AND ({allowed})"
    ))
    .bind(&wanted_posts)
    .bind(&post_types)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();

    // display_user: the post's author, else the topic's, else the owner.
    let display_ids: Vec<i32> = (0..drafts.len())
        .map(|i| {
            post_ids[i]
                .and_then(|id| posts.get(&id).copied().flatten())
                .or_else(|| {
                    topic_ids[i]
                        .and_then(|id| topics.get(&id))
                        .and_then(|t| t.7)
                })
                .unwrap_or(user_id)
        })
        .collect();
    let mut user_ids = display_ids.clone();
    user_ids.push(user_id);
    type UserRow = (i32, String, String, Option<String>, Option<i32>);
    let users: HashMap<i32, UserRow> = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, username_lower, name, uploaded_avatar_id FROM users WHERE id = ANY($1)",
    )
    .bind(&user_ids)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|u| (u.0, u))
    .collect();
    let logo_small_url: Option<String> = match settings.get("logo_small")?.to_i() {
        0 => None,
        id => {
            sqlx::query_scalar("SELECT url FROM uploads WHERE id = $1")
                .bind(i32::try_from(id).unwrap_or(0))
                .fetch_optional(&mut *conn)
                .await?
        }
    };
    let avatar = |u: &UserRow| {
        crate::avatar::avatar_template(urls, u.0, &u.1, u.4, logo_small_url.as_deref())
    };
    let enable_names = settings.get("enable_names")?.truthy();
    let excerpt_options = crate::excerpt::Options {
        keep_emoji_images: true,
        ..Default::default()
    };
    let Some(owner) = users.get(&user_id) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::with_capacity(drafts.len());
    for (i, (key, sequence, data, created_at)) in drafts.iter().enumerate() {
        let topic = topic_ids[i].and_then(|id| topics.get(&id));
        let display = users.get(&display_ids[i]);
        // PostItemExcerpt over DraftSerializer#cooked: the draft's reply.
        let cooked = parsed[i].get("reply").and_then(Value::as_str).unwrap_or("");
        let mut d = Map::new();
        d.insert(
            "excerpt".into(),
            json!(crate::excerpt::excerpt(cooked, 300, &excerpt_options)),
        );
        if cooked.chars().count() > 300 {
            d.insert("truncated".into(), json!(true));
        }
        d.insert(
            "created_at".into(),
            json!(crate::topic_list::time_json(*created_at)),
        );
        d.insert("draft_key".into(), json!(key));
        d.insert("sequence".into(), json!(sequence));
        d.insert("draft_username".into(), json!(owner.1));
        d.insert("avatar_template".into(), json!(avatar(owner)?));
        d.insert("data".into(), json!(data));
        d.insert("topic_id".into(), json!(topic_ids[i]));
        d.insert("username".into(), json!(display.map(|u| &u.1)));
        d.insert("username_lower".into(), json!(display.map(|u| &u.2)));
        if enable_names {
            d.insert("name".into(), json!(display.and_then(|u| u.3.as_ref())));
        }
        d.insert("user_id".into(), json!(user_id));
        d.insert("title".into(), json!(topic.map(|t| &t.1)));
        if topic.is_some_and(|t| !t.1.trim().is_empty()) {
            d.insert("slug".into(), json!(topic.map(|t| &t.2)));
        }
        if let Some(category_id) = topic.and_then(|t| t.3) {
            d.insert("category_id".into(), json!(category_id));
        }
        if topic.is_some_and(|t| t.4) {
            d.insert("closed".into(), json!(true));
        }
        d.insert("archetype".into(), json!(topic.map(|t| &t.6)));
        if topic.is_some_and(|t| t.5) {
            d.insert("archived".into(), json!(true));
        }
        out.push(Value::Object(d));
    }
    Ok(out)
}
