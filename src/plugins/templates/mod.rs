//! discourse-templates: the first posts of the topics in the templates
//! categories (and their subcategories), and of the member's own messages
//! tagged as private templates, inserted into a composer. Who may use them
//! (GuardianExtension, UserExtension), what is a template
//! (TopicExtension#template?), the list TemplatesController#index serves
//! (TopicQueryExtension, TemplatesSerializer) and the usage counts kept in
//! `discourse_templates_usage_count`. The controller is
//! src/routes/discourse_templates.rs; the replacement variables and the UI
//! drawn on the server are `variables` and `view`.

pub mod variables;
pub mod view;

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::plugins::PluginError;
use crate::site_settings::{SettingError, SiteSettings};

/// `Group::AUTO_GROUPS[:everyone]`
const EVERYONE: i64 = 0;

/// `enabled_site_setting :discourse_templates_enabled`
pub fn enabled(settings: &SiteSettings) -> Result<bool, SettingError> {
    Ok(settings.get("discourse_templates_enabled")?.truthy())
}

/// A list setting's values, split on `|` (`&.split("|")`), empty values
/// included as Ruby's split keeps the inner ones.
fn split(settings: &SiteSettings, name: &str) -> Result<Vec<String>, SettingError> {
    let value = settings.get(name)?.to_s();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts: Vec<String> = value.split('|').map(str::to_string).collect();
    // Ruby's String#split drops trailing empty fields.
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    Ok(parts)
}

/// `split("|").map(&:to_i)`
fn split_ids(settings: &SiteSettings, name: &str) -> Result<Vec<i64>, SettingError> {
    Ok(split(settings, name)?
        .iter()
        .map(|p| crate::ruby::to_i(p))
        .collect())
}

/// `User#can_use_category_templates?`: the first configured category
/// that is 0 or missing refuses, else any the member can see allows.
pub async fn can_use_category_templates(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, PluginError> {
    if guardian.is_anonymous() {
        return Ok(false);
    }
    for id in split_ids(settings, "discourse_templates_categories")? {
        let Ok(id) = i32::try_from(id) else {
            return Ok(false);
        };
        if id == 0 {
            return Ok(false);
        }
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
        if !exists {
            return Ok(false);
        }
        if crate::onebox::can_see_category(conn, settings, guardian, id).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `User#can_use_private_templates?`: staff, or a member of an allowed
/// group (everyone allows anyone), while private templates, tagging and
/// their tags are set.
pub async fn can_use_private_templates(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, PluginError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(false);
    };
    if !settings
        .get("discourse_templates_enable_private_templates")?
        .truthy()
        || !settings.get("tagging_enabled")?.truthy()
        || split(settings, "discourse_templates_private_templates_tags")?.is_empty()
    {
        return Ok(false);
    }
    if guardian.is_staff() {
        return Ok(true);
    }
    for group_id in split_ids(
        settings,
        "discourse_templates_groups_allowed_private_templates",
    )? {
        if group_id == EVERYONE {
            return Ok(true);
        }
        let Ok(group_id) = i32::try_from(group_id) else {
            continue;
        };
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM group_users WHERE group_id = $1 AND user_id = $2)",
        )
        .bind(group_id)
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        if member {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `User#can_use_templates?` (the current user's `can_use_templates`).
pub async fn can_use_templates(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<bool, PluginError> {
    Ok(can_use_category_templates(conn, settings, guardian).await?
        || can_use_private_templates(conn, settings, guardian).await?)
}

/// `add_to_serializer(:current_user, :can_use_templates)`
pub async fn current_user_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    out.insert(
        "can_use_templates".into(),
        json!(can_use_templates(conn, settings, guardian).await?),
    );
    Ok(())
}

/// The configured categories with their subcategories
/// (`flat_map { Category.subcategory_ids(id) }`).
async fn template_category_ids(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<Vec<i32>, PluginError> {
    let mut out = Vec::new();
    for id in split_ids(settings, "discourse_templates_categories")? {
        // Category.subcategory_ids(id): the id itself, then its
        // descendants down to max_category_nesting.
        let ids: Vec<i64> = sqlx::query_scalar(
            "WITH RECURSIVE subcategories AS ( \
               SELECT $1::bigint id, 1 depth \
               UNION \
               SELECT categories.id, (subcategories.depth + 1) depth \
               FROM categories JOIN subcategories ON subcategories.id = categories.parent_category_id \
               WHERE subcategories.depth < $2 \
             ) SELECT id FROM subcategories",
        )
        .bind(id)
        .bind(settings.get("max_category_nesting")?.to_i() as i32)
        .fetch_all(&mut *conn)
        .await?;
        out.extend(ids.into_iter().filter_map(|id| i32::try_from(id).ok()));
    }
    Ok(out)
}

/// The private templates' tags, while private templates and tagging are
/// on (TopicExtension#template?'s guard); None otherwise.
fn private_template_tags(settings: &SiteSettings) -> Result<Option<Vec<String>>, SettingError> {
    if !settings.get("tagging_enabled")?.truthy()
        || !settings
            .get("discourse_templates_enable_private_templates")?
            .truthy()
    {
        return Ok(None);
    }
    Ok(Some(split(
        settings,
        "discourse_templates_private_templates_tags",
    )?))
}

/// `Topic#template?(user)`: in a templates category or a subcategory, or
/// the user's own message tagged with a private templates tag. False for
/// a topic that is not there (trashed included, Topic's default scope).
pub async fn is_template(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    user_id: Option<i32>,
) -> Result<bool, PluginError> {
    let row: Option<(Option<i32>, String, Option<i32>)> = sqlx::query_as(
        "SELECT category_id, archetype, user_id FROM topics WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((category_id, archetype, author)) = row else {
        return Ok(false);
    };
    if let Some(category_id) = category_id
        && template_category_ids(conn, settings)
            .await?
            .contains(&category_id)
    {
        return Ok(true);
    }
    let Some(tags) = private_template_tags(settings)? else {
        return Ok(false);
    };
    if archetype != "private_message" || author.is_none() || author != user_id {
        return Ok(false);
    }
    let tagged: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM topic_tags tt JOIN tags t ON t.id = tt.tag_id \
         WHERE tt.topic_id = $1 AND t.name = ANY($2))",
    )
    .bind(topic_id)
    .bind(&tags)
    .fetch_one(&mut *conn)
    .await?;
    Ok(tagged)
}

/// `add_to_serializer(:topic_view, :is_template, include_condition:)`:
/// true, only for a template.
pub async fn topic_view_keys(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    user_id: Option<i32>,
    out: &mut Map<String, Value>,
) -> Result<(), PluginError> {
    if is_template(conn, settings, topic_id, user_id).await? {
        out.insert("is_template".into(), json!(true));
    }
    Ok(())
}

/// `Topic#template_item_usage_count`
pub async fn usage_count(conn: &mut PgConnection, topic_id: i32) -> Result<i32, sqlx::Error> {
    let count: Option<i32> = sqlx::query_scalar(
        "SELECT usage_count FROM discourse_templates_usage_count WHERE topic_id = $1",
    )
    .bind(topic_id)
    .fetch_optional(conn)
    .await?;
    Ok(count.unwrap_or(0))
}

/// `Topic#increment_template_item_usage_count!`: the rows written, which
/// `DB.exec` answers and the controller renders.
pub async fn increment_usage_count(
    conn: &mut PgConnection,
    topic_id: i32,
) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query(
        "INSERT INTO discourse_templates_usage_count AS uc \
         (topic_id, usage_count, created_at, updated_at) \
         VALUES ($1, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP) \
         ON CONFLICT (topic_id) DO UPDATE SET \
           usage_count = uc.usage_count + 1, \
           updated_at = CURRENT_TIMESTAMP \
           WHERE uc.topic_id = $1",
    )
    .bind(topic_id)
    .execute(conn)
    .await?
    .rows_affected())
}

/// A template as TemplatesSerializer has it.
#[derive(Debug, Clone)]
pub struct Template {
    pub id: i32,
    pub title: String,
    pub slug: String,
    /// The first post's raw.
    pub content: String,
    /// The visible tags, sorted, while tagging is on.
    pub tags: Option<Vec<String>>,
    pub usages: i32,
}

impl Template {
    /// TemplatesSerializer
    pub fn json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("title".into(), json!(self.title));
        out.insert("slug".into(), json!(self.slug));
        out.insert("content".into(), json!(self.content));
        if let Some(tags) = &self.tags {
            out.insert("tags".into(), json!(tags));
        }
        out.insert("usages".into(), json!(self.usages));
        Value::Object(out)
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i32,
    title: String,
    slug: Option<String>,
    raw: Option<String>,
    usages: Option<i32>,
}

/// `prioritize_pinned_topics` without a category: the globally pinned
/// topics the member has not cleared first, newest pin first, then the
/// rest by title (the plugin's reorder); `tu` is the member's topic_users.
const PINNED_THEN_TITLE: &str = "ORDER BY (topics.pinned_globally AND topics.pinned_at IS NOT NULL \
       AND (topics.pinned_at > tu.cleared_pinned_at OR tu.cleared_pinned_at IS NULL)) DESC, \
     CASE WHEN topics.pinned_globally AND topics.pinned_at IS NOT NULL \
       AND (topics.pinned_at > tu.cleared_pinned_at OR tu.cleared_pinned_at IS NULL) \
       THEN topics.pinned_at END DESC NULLS LAST, \
     topics.title ASC";

/// TemplatesController#index's list for the member (logged in, who may
/// use templates): `list_category_templates` then
/// `list_private_templates`, each limited to
/// discourse_templates_max_replies_fetched.
pub async fn list(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<Vec<Template>, PluginError> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(Vec::new());
    };
    let per_page = settings
        .get("discourse_templates_max_replies_fetched")?
        .to_i();
    let mut rows: Vec<Row> = Vec::new();
    if can_use_category_templates(conn, settings, guardian).await? {
        let category_ids = template_category_ids(conn, settings).await?;
        // filter_allowed_categories over listable topics, without the
        // category definitions, visible and not archived.
        let allowed = if guardian.is_admin()
            && !settings
                .get("suppress_secured_categories_from_admin")?
                .truthy()
        {
            "TRUE".to_string()
        } else {
            format!(
                "(categories.id IS NULL OR categories.id IN ({}))",
                guardian.allowed_category_ids_sql(settings)?
            )
        };
        rows.extend(
            sqlx::query_as::<_, Row>(&format!(
                "SELECT topics.id, topics.title, topics.slug, p.raw, uc.usage_count AS usages \
                 FROM topics \
                 LEFT JOIN categories ON categories.id = topics.category_id \
                 LEFT JOIN topic_users tu ON tu.topic_id = topics.id AND tu.user_id = $1 \
                 LEFT JOIN posts p ON p.topic_id = topics.id AND p.post_number = 1 AND p.deleted_at IS NULL \
                 LEFT JOIN discourse_templates_usage_count uc ON uc.topic_id = topics.id \
                 WHERE topics.deleted_at IS NULL \
                   AND topics.archetype <> 'private_message' \
                   AND {allowed} \
                   AND COALESCE(categories.topic_id, 0) <> topics.id \
                   AND topics.category_id = ANY($2) \
                   AND topics.visible AND NOT topics.archived \
                 {PINNED_THEN_TITLE} LIMIT $3"
            ))
            .bind(user_id)
            .bind(&category_ids)
            .bind(per_page)
            .fetch_all(&mut *conn)
            .await?,
        );
    }
    if can_use_private_templates(conn, settings, guardian).await? {
        let tags = split(settings, "discourse_templates_private_templates_tags")?;
        // private_messages_for(user, :user), not archived by the member,
        // their own, joined to the matching tags (a message with two of
        // them comes twice, as the join gives it).
        let visible = if guardian.is_staff() {
            ""
        } else {
            "AND topics.visible"
        };
        rows.extend(
            sqlx::query_as::<_, Row>(&format!(
                "SELECT topics.id, topics.title, topics.slug, p.raw, uc.usage_count AS usages \
                 FROM topics \
                 LEFT JOIN topic_users tu ON tu.topic_id = topics.id AND tu.user_id = $1 \
                 INNER JOIN topic_tags tt ON tt.topic_id = topics.id \
                 INNER JOIN tags ON tags.id = tt.tag_id \
                 LEFT JOIN user_archived_messages um ON um.user_id = $1 AND um.topic_id = topics.id \
                 LEFT JOIN posts p ON p.topic_id = topics.id AND p.post_number = 1 AND p.deleted_at IS NULL \
                 LEFT JOIN discourse_templates_usage_count uc ON uc.topic_id = topics.id \
                 WHERE topics.deleted_at IS NULL \
                   AND topics.archetype = 'private_message' \
                   AND topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = $1) \
                   {visible} \
                   AND um.user_id IS NULL \
                   AND topics.user_id = $1 \
                   AND tags.name = ANY($2) \
                 {PINNED_THEN_TITLE} LIMIT $3"
            ))
            .bind(user_id)
            .bind(&tags)
            .bind(per_page)
            .fetch_all(&mut *conn)
            .await?,
        );
    }
    let tags = if settings.get("tagging_enabled")?.truthy() {
        let ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
        let pairs: Vec<(i32, String)> = sqlx::query_as(&format!(
            "SELECT tt.topic_id, tags.name FROM topic_tags tt JOIN tags ON tags.id = tt.tag_id \
             WHERE tt.topic_id = ANY($1) AND tags.id IN {}",
            crate::tags::visible_tag_ids_subquery(guardian, settings)?
        ))
        .bind(&ids)
        .fetch_all(&mut *conn)
        .await?;
        Some(pairs)
    } else {
        None
    };
    rows.into_iter()
        .map(|row| {
            // TemplatesSerializer#content reads first_post.raw, which a
            // template whose first post is gone doesn't have.
            let Some(content) = row.raw else {
                return Err(PluginError::Unsupported(crate::Unsupported(
                    "a template without its first post",
                )));
            };
            let tags = tags.as_ref().map(|pairs| {
                let mut names: Vec<String> = pairs
                    .iter()
                    .filter(|(id, _)| *id == row.id)
                    .map(|(_, name)| name.clone())
                    .collect();
                // Ruby's sort: by bytes.
                names.sort();
                names
            });
            Ok(Template {
                id: row.id,
                title: row.title,
                // Topic#slug: the stored slug (every topic the port or
                // Rails creates has one).
                slug: row.slug.unwrap_or_default(),
                content,
                tags,
                usages: row.usages.unwrap_or(0),
            })
        })
        .collect()
}

/// A template's first post raw split as the topic's "New topic from
/// template" button reads it (discourse-templates-button.gjs
/// `parseTemplateOptions`): an optional `<!-- discourse-template ... -->`
/// header in the first 500 characters with `category:` and `tags:`
/// options, then the body.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TemplateOptions {
    pub body: String,
    pub category_slug: Option<String>,
    /// The tags, trimmed and comma joined.
    pub tags: Option<String>,
}

/// `TEMPLATE_OPTIONS_MAX_LENGTH`
const TEMPLATE_OPTIONS_MAX_LENGTH: usize = 500;

pub fn parse_template_options(raw: &str) -> TemplateOptions {
    use std::sync::LazyLock;
    static START: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?i)^\s*<!--\s*discourse-template\s*").expect("start regex")
    });
    static OPTION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^\s*([\w-]+)\s*:\s*(.*?)\s*$").expect("option regex"));
    static SEPARATOR: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^([ \t]*\r?\n){1,2}").expect("separator regex"));
    let whole = || TemplateOptions {
        body: raw.to_string(),
        ..Default::default()
    };
    // raw.slice(0, 500), in UTF-16 code units as JS counts them.
    let header = js_slice(raw, TEMPLATE_OPTIONS_MAX_LENGTH);
    let Some(start) = START.find(header) else {
        return whole();
    };
    let Some(end) = header[start.end()..].find("-->").map(|i| start.end() + i) else {
        return whole();
    };
    let mut category = None;
    let mut tags = None;
    for line in raw[start.end()..end].split('\n') {
        let Some(caps) = OPTION.captures(line) else {
            continue;
        };
        let (key, value) = (&caps[1], &caps[2]);
        if value.is_empty() {
            continue;
        }
        match key {
            "category" => category = Some(value.to_string()),
            "tags" => tags = Some(value.to_string()),
            _ => {}
        }
    }
    TemplateOptions {
        body: SEPARATOR.replace(&raw[end + 3..], "").into_owned(),
        category_slug: category,
        tags: tags.map(|t| {
            t.split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        }),
    }
}

/// `s.slice(0, n)` with n in UTF-16 code units, cut back to a char
/// boundary.
fn js_slice(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// The topic's template controls' "New topic from template" target
/// (`findCategory`): the category the options name by slug path
/// (`parent/child` or `parent:child`), when the member may create topics
/// in it.
pub async fn find_category(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    slug_path: &str,
) -> Result<Option<i32>, PluginError> {
    let slugs: Vec<&str> = slug_path
        .split(['/', ':'])
        .filter(|s| !s.is_empty())
        .collect();
    if slugs.is_empty() {
        return Ok(None);
    }
    let mut parent: Option<i32> = None;
    for slug in slugs {
        let found: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM categories WHERE lower(slug) = lower($1) \
             AND parent_category_id IS NOT DISTINCT FROM $2 ORDER BY id LIMIT 1",
        )
        .bind(slug)
        .bind(parent)
        .fetch_optional(&mut *conn)
        .await?;
        let Some(id) = found else {
            return Ok(None);
        };
        parent = Some(id);
    }
    let Some(id) = parent else {
        return Ok(None);
    };
    let allowed = guardian
        .topic_create_allowed_category_ids(&mut *conn, settings)
        .await?;
    Ok(allowed.contains(&id).then_some(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_options_without_a_header_are_the_whole_raw() {
        let raw = "Hello %{my_username}";
        assert_eq!(
            parse_template_options(raw),
            TemplateOptions {
                body: raw.into(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn template_options_read_category_and_tags() {
        let raw = "<!-- discourse-template\ncategory: support/bugs\ntags: one, two ,, three\n-->\n\nBody\nmore";
        assert_eq!(
            parse_template_options(raw),
            TemplateOptions {
                body: "Body\nmore".into(),
                category_slug: Some("support/bugs".into()),
                tags: Some("one,two,three".into()),
            }
        );
    }

    #[test]
    fn template_options_need_the_header_closed_within_500_chars() {
        let raw = format!(
            "<!-- discourse-template\ncategory: a\n{}-->body",
            "x".repeat(500)
        );
        assert_eq!(parse_template_options(&raw).body, raw);
    }

    #[test]
    fn template_options_strip_at_most_two_blank_lines() {
        let raw = "<!-- DISCOURSE-TEMPLATE tags: a -->\n\n\nBody";
        let parsed = parse_template_options(raw);
        assert_eq!(parsed.body, "\nBody");
        // What follows the start tag on its line is read as options too.
        assert_eq!(parsed.tags.as_deref(), Some("a"));
    }
}
