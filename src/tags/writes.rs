//! Tag's own writes (app/models/tag.rb) behind TagsController and the
//! jobs: `destroy` with its dependents, saving a renamed or redescribed
//! tag (`ensure_slug`, the validations, `cook_description`, the search
//! index, the hashtag remap and the tag_updated web hooks), and synonyms
//! (`add_or_create_synonyms`, removing one).

use serde_json::{Map, json};
use sqlx::PgConnection;

use crate::web_hooks::Payloads;
use crate::{AppError, Unsupported};

/// `Tag#destroy`: HasDestroyedWebHook's payload taken first; then the
/// dependents in the order Tag declares them (its search data and
/// localizations, tag users, topic tags with TopicTag's counters, category
/// stats and tags, tag group memberships, its synonyms each destroyed in
/// turn, sidebar links); the row; then its tag_destroyed hooks, after its
/// synonyms'.
pub async fn destroy(
    conn: &mut PgConnection,
    payloads: &Payloads<'_>,
    tag_id: i32,
) -> Result<(), AppError> {
    let payload = if crate::web_hooks::active(conn, "tag_destroyed").await? {
        Some(payloads.tag(conn, tag_id).await?)
    } else {
        None
    };
    let embedded: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM embeddable_host_tags WHERE tag_id = $1)")
            .bind(tag_id)
            .fetch_one(&mut *conn)
            .await?;
    if embedded {
        return Err(Unsupported("destroying a tag embeddable hosts use").into());
    }
    for table in ["tag_search_data", "tag_localizations", "tag_users"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE tag_id = $1"))
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }
    // TopicTag's after_destroy, for each topic tag of a topic not deleted.
    let topic_tags: Vec<(i32, i32)> =
        sqlx::query_as("SELECT id, topic_id FROM topic_tags WHERE tag_id = $1 ORDER BY ctid")
            .bind(tag_id)
            .fetch_all(&mut *conn)
            .await?;
    for (id, topic_id) in topic_tags {
        sqlx::query("DELETE FROM topic_tags WHERE id = $1")
            .bind(id)
            .execute(&mut *conn)
            .await?;
        let topic: Option<(String, Option<i32>)> = sqlx::query_as(
            "SELECT archetype, category_id FROM topics WHERE id = $1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *conn)
        .await?;
        if let Some((archetype, category_id)) = topic {
            let saved = crate::tagging::SavedTopic {
                id: topic_id,
                private_message: archetype == "private_message",
                category_id,
            };
            crate::tagging::topic_tag_destroyed(conn, &saved, tag_id).await?;
        }
    }
    for table in [
        "category_tag_stats",
        "category_tags",
        "tag_group_memberships",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE tag_id = $1"))
            .bind(tag_id)
            .execute(&mut *conn)
            .await?;
    }
    let synonyms: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM tags WHERE target_tag_id = $1 ORDER BY ctid")
            .bind(tag_id)
            .fetch_all(&mut *conn)
            .await?;
    for synonym in synonyms {
        Box::pin(destroy(conn, payloads, synonym)).await?;
    }
    sqlx::query(
        "DELETE FROM sidebar_section_links WHERE linkable_type = 'Tag' AND linkable_id = $1",
    )
    .bind(tag_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("DELETE FROM tags WHERE id = $1")
        .bind(tag_id)
        .execute(&mut *conn)
        .await?;
    if let Some(payload) = payload {
        let mut opts = Map::new();
        opts.insert("id".into(), json!(tag_id));
        opts.insert("payload".into(), json!(crate::web_hooks::to_json(&payload)));
        crate::web_hooks::enqueue_hooks(conn, "tag", "tag_destroyed", opts).await?;
    }
    Ok(())
}

/// What a save of a tag changes: each attribute when it is assigned.
#[derive(Debug, Default)]
pub struct TagChanges {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<Option<String>>,
}

/// The tag as a save reads it.
#[derive(sqlx::FromRow)]
struct TagRow {
    name: String,
    slug: String,
    description: Option<String>,
}

/// `errors.full_messages` for a Tag attribute.
fn tag_error(i18n: &crate::i18n::I18n, attribute: &str, key: &str, count: Option<i64>) -> String {
    let message = match count {
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
    format!("{attribute} {message}")
}

/// `String#parameterize`, transliterated first.
fn parameterize(s: &str) -> String {
    crate::site::parameterize(&crate::posting::text::transliterate(s))
}

/// The outcome of saving a tag: its errors, or the name it had when the
/// name changed.
pub enum Saved {
    Invalid(Vec<String>),
    Saved { renamed_from: Option<String> },
}

/// `Tag#save` with the changes assigned: ensure_slug, the validations,
/// cook_description, the row, the search index for a new name; then, as
/// after_commit would (the caller's transaction is the commit), the
/// tag_updated hooks and the hashtag remap a rename asks for.
pub async fn save(
    conn: &mut PgConnection,
    payloads: &Payloads<'_>,
    host: &crate::pretty_text::Host,
    tag_id: i32,
    changes: &TagChanges,
) -> Result<Saved, AppError> {
    let i18n = payloads.i18n;
    let current: TagRow = sqlx::query_as("SELECT name, slug, description FROM tags WHERE id = $1")
        .bind(tag_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(Unsupported("saving a tag that vanished"))?;
    let name = changes.name.clone().unwrap_or_else(|| current.name.clone());
    let description = changes
        .description
        .clone()
        .unwrap_or_else(|| current.description.clone());
    let name_changed = name != current.name;
    let mut slug = changes.slug.clone().unwrap_or_else(|| current.slug.clone());
    let slug_changed = slug != current.slug;
    let mut errors = Vec::new();
    // ensure_slug
    if !name.is_empty() {
        if !slug.is_empty() && slug_changed && slug != parameterize(&slug) {
            errors.push(tag_error(i18n, "Slug", "invalid", None));
        } else if slug.is_empty() || (name_changed && !slug_changed) {
            slug = crate::posting::text::slug_with_default(&name, "en", "")?;
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM tags WHERE lower(slug) = lower($1) AND id <> $2)",
            )
            .bind(&slug)
            .bind(tag_id)
            .fetch_one(&mut *conn)
            .await?;
            if slug.is_empty() || duplicate {
                slug = String::new();
            }
        }
    }
    // validations, in their order
    if name.trim().is_empty() {
        errors.push(tag_error(i18n, "Name", "blank", None));
    } else {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tags WHERE lower(name) = lower($1) AND id <> $2)",
        )
        .bind(&name)
        .bind(tag_id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            errors.push(tag_error(i18n, "Name", "taken", None));
        }
    }
    if !slug.is_empty() {
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tags WHERE lower(slug) = lower($1) AND id <> $2)",
        )
        .bind(&slug)
        .bind(tag_id)
        .fetch_one(&mut *conn)
        .await?;
        if taken {
            errors.push(tag_error(i18n, "Slug", "taken", None));
        }
    }
    if crate::tagging::RESERVED_TAGS.contains(&name.trim().to_lowercase().as_str()) {
        errors.push(tag_error(i18n, "Name", "invalid", None));
    }
    if description
        .as_deref()
        .is_some_and(|d| d.chars().count() > 1000)
    {
        errors.push(tag_error(i18n, "Description", "too_long", Some(1000)));
    }
    if !errors.is_empty() {
        return Ok(Saved::Invalid(errors));
    }
    let description_changed = description != current.description;
    if name_changed || slug != current.slug || description_changed {
        if description_changed {
            // cook_description: nil for a blank one.
            let cooked = match description.as_deref().filter(|d| !d.trim().is_empty()) {
                Some(d) => Some(crate::pretty_text::cook_without_onebox(host, d).await?),
                None => None,
            };
            sqlx::query(
                "UPDATE tags SET description_cooked = $2, description_cooked_version = 1 WHERE id = $1",
            )
            .bind(tag_id)
            .bind(cooked)
            .execute(&mut *conn)
            .await?;
        }
        sqlx::query(
            "UPDATE tags SET name = $2, slug = $3, description = $4, updated_at = clock_timestamp() \
             WHERE id = $1",
        )
        .bind(tag_id)
        .bind(&name)
        .bind(&slug)
        .bind(&description)
        .execute(&mut *conn)
        .await?;
        if name_changed {
            crate::posting::search_index::index_tag(
                conn,
                payloads.settings,
                tag_id,
                &name.to_lowercase(),
            )
            .await?;
        }
    }
    updated_hooks(conn, payloads, tag_id).await?;
    // enqueue_tag_hashtag_remap, unless only the case changed.
    if name_changed && !current.name.eq_ignore_ascii_case(&name) {
        crate::jobs::enqueue(
            &mut *conn,
            "remap_hashtag",
            json!({"remaps": [{"type": "tag", "id": tag_id, "old_ref": current.name}]}),
        )
        .await?;
    }
    Ok(Saved::Saved {
        renamed_from: name_changed.then_some(current.name),
    })
}

/// `trigger_tag_updated_event`: the tag_updated web hooks with the saved
/// tag's payload.
pub async fn updated_hooks(
    conn: &mut PgConnection,
    payloads: &Payloads<'_>,
    tag_id: i32,
) -> Result<(), AppError> {
    if crate::web_hooks::active(conn, "tag_updated").await? {
        let payload = payloads.tag(conn, tag_id).await?;
        let mut opts = Map::new();
        opts.insert("id".into(), json!(tag_id));
        opts.insert("payload".into(), json!(crate::web_hooks::to_json(&payload)));
        crate::web_hooks::enqueue_hooks(conn, "tag", "tag_updated", opts).await?;
    }
    Ok(())
}

/// `StaffActionLogger#log_custom(type, subject:)` or `(type, tags:)`: the
/// subject a column, other details as `key: value` text.
pub async fn log_custom(
    conn: &mut PgConnection,
    acting_user_id: i32,
    custom_type: &str,
    subject: Option<&str>,
    details: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO user_histories (action, acting_user_id, custom_type, details, subject, admin_only, \
                                     created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, TRUE, clock_timestamp(), clock_timestamp())",
    )
    .bind(crate::staff_action_logs::action_id("custom_staff"))
    .bind(acting_user_id)
    .bind(custom_type)
    .bind(details)
    .bind(subject)
    .execute(conn)
    .await?;
    Ok(())
}

/// `Tag.update_topic_counts` (Tag.ensure_consistency!): each tag's staff,
/// public and message counts from its live topics.
pub async fn update_topic_counts(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tags t SET staff_topic_count = x.topic_count FROM ( \
           SELECT COUNT(topics.id) AS topic_count, tags.id AS tag_id FROM tags \
           LEFT JOIN topic_tags ON tags.id = topic_tags.tag_id \
           LEFT JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
             AND topics.archetype != 'private_message' \
           GROUP BY tags.id) x \
         WHERE x.tag_id = t.id AND x.topic_count <> t.staff_topic_count",
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE tags t SET public_topic_count = x.topic_count FROM ( \
           WITH tags_with_public_topics AS ( \
             SELECT COUNT(topics.id) AS topic_count, tags.id AS tag_id FROM tags \
             INNER JOIN topic_tags ON tags.id = topic_tags.tag_id \
             INNER JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
               AND topics.archetype != 'private_message' \
             INNER JOIN categories ON categories.id = topics.category_id AND NOT categories.read_restricted \
             GROUP BY tags.id) \
           SELECT COALESCE(tags_with_public_topics.topic_count, 0) AS topic_count, tags.id AS tag_id \
           FROM tags LEFT JOIN tags_with_public_topics ON tags_with_public_topics.tag_id = tags.id) x \
         WHERE x.tag_id = t.id AND x.topic_count <> t.public_topic_count",
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE tags t SET pm_topic_count = x.pm_topic_count FROM ( \
           SELECT COUNT(topics.id) AS pm_topic_count, tags.id AS tag_id FROM tags \
           LEFT JOIN topic_tags ON tags.id = topic_tags.tag_id \
           LEFT JOIN topics ON topics.id = topic_tags.topic_id AND topics.deleted_at IS NULL \
             AND topics.archetype = 'private_message' \
           GROUP BY tags.id) x \
         WHERE x.tag_id = t.id AND x.pm_topic_count <> t.pm_topic_count",
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `DiscourseTagging.add_or_create_synonyms(target, synonym_tag_ids:,
/// new_synonym_names:)`: the names that failed with why, None when none
/// did. Tags with synonyms of their own can't become synonyms; new names
/// become tags (insert_all: no callbacks); the synonyms take the
/// target's tag groups and categories, and its topics.
pub async fn add_or_create_synonyms(
    conn: &mut PgConnection,
    settings: &crate::site_settings::SiteSettings,
    i18n: &crate::i18n::I18n,
    target_id: i32,
    synonym_ids: &[i32],
    new_names: &[String],
) -> Result<Option<Vec<(String, String)>>, AppError> {
    if synonym_ids.is_empty() && new_names.is_empty() {
        return Ok(None);
    }
    let target_name: String = sqlx::query_scalar("SELECT name FROM tags WHERE id = $1")
        .bind(target_id)
        .fetch_one(&mut *conn)
        .await?;
    let ids: Vec<i32> = synonym_ids
        .iter()
        .copied()
        .filter(|i| *i != target_id)
        .collect();
    let system = crate::web_hooks::system_guardian(conn).await?;
    let cleaned: Vec<String> =
        crate::tagging::tags_for_saving(conn, settings, &system, new_names, false)
            .await?
            .into_iter()
            .filter(|n| *n != target_name)
            .collect();
    let lower: Vec<String> = cleaned.iter().map(|n| n.to_lowercase()).collect();
    let candidates: Vec<(i32, String, bool)> = sqlx::query_as(
        "SELECT t.id, t.name, EXISTS(SELECT 1 FROM tags s WHERE s.target_tag_id = t.id) \
         FROM tags t WHERE (LOWER(t.name) = ANY($1) OR t.id = ANY($2)) AND t.id != $3 ORDER BY t.ctid",
    )
    .bind(&lower)
    .bind(&ids)
    .bind(target_id)
    .fetch_all(&mut *conn)
    .await?;
    let mut valid: Vec<i32> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut existing: Vec<String> = Vec::new();
    for (id, name, has_synonyms) in candidates {
        existing.push(name.to_lowercase());
        if has_synonyms {
            failed.push((
                name,
                i18n.t("tags.synonyms_exist")
                    .unwrap_or_default()
                    .to_string(),
            ));
        } else {
            valid.push(id);
        }
    }
    sqlx::query("UPDATE tags SET target_tag_id = $2 WHERE id = ANY($1)")
        .bind(&valid)
        .bind(target_id)
        .execute(&mut *conn)
        .await?;
    for name in cleaned
        .iter()
        .filter(|n| !existing.contains(&n.to_lowercase()))
    {
        let slug = crate::posting::text::slug_with_default(name, "en", "")?;
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO tags (name, slug, target_tag_id, created_at, updated_at) \
             VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp()) RETURNING id",
        )
        .bind(name)
        .bind(&slug)
        .bind(target_id)
        .fetch_one(&mut *conn)
        .await?;
        valid.push(id);
    }
    let failed = (!failed.is_empty()).then_some(failed);
    if valid.is_empty() {
        return Ok(failed);
    }
    sqlx::query(
        "INSERT INTO tag_group_memberships (tag_id, tag_group_id, created_at, updated_at) \
         SELECT s.id, tgm.tag_group_id, NOW(), NOW() FROM unnest($1::int[]) s(id) \
         JOIN tag_group_memberships tgm ON tgm.tag_id = $2 ON CONFLICT DO NOTHING",
    )
    .bind(&valid)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "INSERT INTO category_tags (tag_id, category_id, created_at, updated_at) \
         SELECT s.id, ct.category_id, NOW(), NOW() FROM unnest($1::int[]) s(id) \
         JOIN category_tags ct ON ct.tag_id = $2 ON CONFLICT DO NOTHING",
    )
    .bind(&valid)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "WITH keep AS (SELECT MIN(id) AS id FROM topic_tags WHERE tag_id = ANY($1) \
           AND topic_id NOT IN (SELECT topic_id FROM topic_tags WHERE tag_id = $2) GROUP BY topic_id) \
         DELETE FROM topic_tags WHERE tag_id = ANY($1) AND id NOT IN (SELECT id FROM keep)",
    )
    .bind(&valid)
    .bind(target_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE topic_tags SET tag_id = $2 WHERE tag_id = ANY($1)")
        .bind(&valid)
        .bind(target_id)
        .execute(&mut *conn)
        .await?;
    // Scheduler::Defer.later: Tag.ensure_consistency!
    update_topic_counts(conn).await?;
    Ok(failed)
}

/// `TagUser.change(user_id, tag_id, level)`: on the synonym's target, the
/// row created or its level changed (nothing for the same level), then
/// the user's topics auto watched and tracked.
pub async fn change_tag_user(
    conn: &mut PgConnection,
    user_id: i32,
    tag_id: i32,
    level: i32,
) -> Result<(), sqlx::Error> {
    let target: Option<i32> = sqlx::query_scalar("SELECT target_tag_id FROM tags WHERE id = $1")
        .bind(tag_id)
        .fetch_one(&mut *conn)
        .await?;
    let tag_id = target.unwrap_or(tag_id);
    let existing: Option<(i32, i32)> = sqlx::query_as(
        "SELECT id, notification_level FROM tag_users WHERE user_id = $1 AND tag_id = $2 ORDER BY id LIMIT 1",
    )
    .bind(user_id)
    .bind(tag_id)
    .fetch_optional(&mut *conn)
    .await?;
    match existing {
        Some((_, current)) if current == level => return Ok(()),
        Some((id, _)) => {
            sqlx::query(
                "UPDATE tag_users SET notification_level = $2, updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(id)
            .bind(level)
            .execute(&mut *conn)
            .await?;
        }
        None => {
            sqlx::query(
                "INSERT INTO tag_users (user_id, tag_id, notification_level, created_at, updated_at) \
                 VALUES ($1, $2, $3, clock_timestamp(), clock_timestamp())",
            )
            .bind(user_id)
            .bind(tag_id)
            .bind(level)
            .execute(&mut *conn)
            .await?;
        }
    }
    crate::tagging::auto_watch(conn, None, &[user_id]).await?;
    crate::tagging::auto_track(conn, None, &[user_id]).await?;
    Ok(())
}
