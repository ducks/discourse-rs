//! DiscourseTagging's writes (lib/discourse_tagging.rb): tagging a topic
//! with `tag_topic` / `tag_topic_by_names` and the validations
//! TopicCreator runs first (`validate_category_tags`), `tags_for_saving`,
//! `clean_tag`, `filter_allowed_tags`, creating tags (`Tag.create`),
//! TopicTag's counters and TagUser's auto watching and tracking.
//!
//! Tagging is two steps, as `topic.tags = tags` is for a topic Rails has
//! not saved yet: `tag_topic` works out the topic's tags (creating the new
//! ones), and `apply` writes them once the topic row exists, followed by
//! `after_tags_saved` when the topic is saved (Topic's after_save with
//! `tags_changed`). PostRevisor's tag edits (188) call the same three on a
//! saved topic.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::{AppError, Unsupported};

/// `TagGroupPermission.permission_types`
pub const PERMISSION_FULL: i32 = 1;
pub const PERMISSION_READONLY: i32 = 3;

/// `Tag::RESERVED_TAGS`
pub const RESERVED_TAGS: [&str; 2] = ["none", "constructor"];

/// A tag object from the composer (`{id, name}`); a missing or blank id
/// marks a new tag.
#[derive(Debug, Clone, PartialEq)]
pub struct TagObject {
    /// `t[:id]&.to_i`
    pub id: Option<i64>,
    /// Whether `t[:id].blank?`
    pub id_blank: bool,
    pub name: Option<String>,
}

/// The `tags` param, as names (deprecated) or tag objects.
#[derive(Debug, Clone, PartialEq)]
pub enum TagsParam {
    Names(Vec<String>),
    Objects(Vec<TagObject>),
}

impl TagsParam {
    pub fn is_empty(&self) -> bool {
        match self {
            TagsParam::Names(n) => n.is_empty(),
            TagsParam::Objects(o) => o.is_empty(),
        }
    }

    /// The names for the payload of a queued post (ReviewableQueuedPost
    /// keeps the param as sent).
    pub fn to_value(&self) -> Value {
        match self {
            TagsParam::Names(n) => json!(n),
            TagsParam::Objects(o) => Value::Array(
                o.iter()
                    .map(|t| {
                        let mut m = Map::new();
                        if let Some(id) = t.id {
                            m.insert("id".into(), json!(id));
                        }
                        if let Some(name) = &t.name {
                            m.insert("name".into(), json!(name));
                        }
                        Value::Object(m)
                    })
                    .collect(),
            ),
        }
    }
}

/// `"12abc".to_i`
fn ruby_to_i(s: &str) -> i64 {
    let s = s.trim_start();
    let (sign, digits) = match s.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, s.strip_prefix('+').unwrap_or(s)),
    };
    let n: String = digits.chars().take_while(char::is_ascii_digit).collect();
    n.parse::<i64>().map(|n| n * sign).unwrap_or(0)
}

/// The `tags` param as Rails reads it, or None when blank. A form hash
/// (`tags[0][name]`), a lone string or a list mixing strings and objects
/// is a NoMethodError in TopicCreator: refused.
pub fn parse_param(value: Option<&Value>) -> Result<Option<TagsParam>, Unsupported> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::Array(items) if items.is_empty() => Ok(None),
        Value::Array(items) => {
            if items.iter().all(Value::is_string) {
                Ok(Some(TagsParam::Names(
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                )))
            } else if items.iter().all(Value::is_object) {
                let mut out = Vec::new();
                for item in items {
                    let id = item.get("id");
                    let (id, id_blank) = match id {
                        None | Some(Value::Null) => (None, true),
                        Some(Value::Number(n)) => (n.as_i64(), false),
                        Some(Value::String(s)) => (Some(ruby_to_i(s)), s.trim().is_empty()),
                        Some(_) => return Err(Unsupported("a tag object's id of another shape")),
                    };
                    let id = if id_blank && id == Some(0) { None } else { id };
                    let name = match item.get("name") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(_) => return Err(Unsupported("a tag object's name of another shape")),
                    };
                    out.push(TagObject { id, id_blank, name });
                }
                Ok(Some(TagsParam::Objects(out)))
            } else {
                Err(Unsupported("a tags param mixing names and tag objects"))
            }
        }
        Value::Object(map) if map.is_empty() => Ok(None),
        Value::Object(_) => Err(Unsupported(
            "tags as a form hash (a NoMethodError in TopicCreator)",
        )),
        _ => Err(Unsupported("tags as a single value")),
    }
}

/// `tag_param_count`: an array's length or a hash's keys, else 0.
pub fn param_count(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Array(items)) => items.len(),
        Some(Value::Object(map)) => map.len(),
        _ => 0,
    }
}

/// `reject_too_many_tags!`: the error when the param has more tags than a
/// topic may.
pub fn too_many_tags(
    settings: &SiteSettings,
    i18n: &I18n,
    value: Option<&Value>,
) -> Result<Option<String>, AppError> {
    let max = settings.get("max_tags_per_topic")?.to_i();
    if value.is_some() && param_count(value) as i64 > max {
        return Ok(Some(
            i18n.t_count("tags.too_many_tags_for_topic", max, &[])
                .unwrap_or_default(),
        ));
    }
    Ok(None)
}

/// `Tag.where(id: ids).pluck(:name)`, in the table's order.
async fn names_of_ids(conn: &mut PgConnection, ids: &[i64]) -> Result<Vec<String>, sqlx::Error> {
    let ids: Vec<i32> = ids
        .iter()
        .filter_map(|id| i32::try_from(*id).ok())
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar("SELECT name FROM tags WHERE id = ANY($1) ORDER BY ctid")
        .bind(&ids)
        .fetch_all(conn)
        .await
}

/// `DiscourseTagging.tag_topic`'s names: names as given, or the new names
/// of tag objects then the names of those with ids.
pub async fn names_for_tagging(
    conn: &mut PgConnection,
    param: &TagsParam,
) -> Result<Vec<String>, sqlx::Error> {
    match param {
        TagsParam::Names(n) => Ok(n.clone()),
        TagsParam::Objects(objects) => {
            let ids: Vec<i64> = objects.iter().filter_map(|t| t.id).collect();
            let mut names: Vec<String> = objects
                .iter()
                .filter(|t| t.id_blank)
                .filter_map(|t| t.name.clone().filter(|n| !n.trim().is_empty()))
                .collect();
            names.extend(names_of_ids(conn, &ids).await?);
            Ok(names)
        }
    }
}

/// TopicCreator#valid?'s names: those of tag objects with ids first, then
/// the new names.
pub async fn names_for_validation(
    conn: &mut PgConnection,
    param: &TagsParam,
) -> Result<Vec<String>, sqlx::Error> {
    match param {
        TagsParam::Names(n) => Ok(n.clone()),
        TagsParam::Objects(objects) => {
            let ids: Vec<i64> = objects.iter().filter_map(|t| t.id).collect();
            let mut names = names_of_ids(conn, &ids).await?;
            names.extend(
                objects
                    .iter()
                    .filter(|t| t.id_blank)
                    .filter_map(|t| t.name.clone().filter(|n| !n.trim().is_empty())),
            );
            Ok(names)
        }
    }
}

/// ReviewableQueuedPost#create_options' tags: the names as kept, a tag
/// object by its name. An object without one is a nil name, a
/// NoMethodError down the line: refused.
pub fn queued_names(param: Option<&TagsParam>) -> Result<Vec<String>, Unsupported> {
    match param {
        None => Ok(Vec::new()),
        Some(TagsParam::Names(n)) => Ok(n.clone()),
        Some(TagsParam::Objects(o)) => o
            .iter()
            .map(|t| {
                t.name
                    .clone()
                    .ok_or(Unsupported("a queued tag object without a name"))
            })
            .collect(),
    }
}

/// `DiscourseTagging.clean_tag(tag, truncate:)`
pub fn clean_tag(settings: &SiteSettings, tag: &str) -> Result<String, AppError> {
    clean_tag_with(settings, tag, true)
}

pub fn clean_tag_with(
    settings: &SiteSettings,
    tag: &str,
    truncate: bool,
) -> Result<String, AppError> {
    use std::sync::LazyLock;
    // [^[:word:][:punct:]]+, then TAGS_FILTER_REGEXP. Onigmo's [[:punct:]]
    // takes the ASCII symbols too.
    static NOT_WORD_OR_PUNCT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"[^\w\p{P}$+<=>^`|~]+").expect("a valid regex"));
    static FILTERED: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"[/?#\[\]@!$&'()*+,;=%\\`^\s|{}"<>]+"#).expect("a valid regex")
    });
    static SPACES: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"[[:space:]\p{Zs}\u{85}]+").expect("a valid regex"));
    let mut tag = tag.to_string();
    if settings.get("force_lowercase_tags")?.truthy() {
        tag = tag.to_lowercase();
    }
    let tag = tag.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\0');
    let tag = SPACES.replace_all(tag, "-");
    let tag = NOT_WORD_OR_PUNCT.replace_all(&tag, "");
    let tag = FILTERED.replace_all(&tag, "");
    let mut squeezed = String::new();
    for c in tag.chars() {
        if c == '-' && squeezed.ends_with('-') {
            continue;
        }
        squeezed.push(c);
    }
    let tag = if truncate {
        let max = settings.get("max_tag_length")?.to_i().max(0) as usize;
        squeezed.chars().take(max).collect()
    } else {
        squeezed
    };
    Ok(tag.trim_matches('.').to_string())
}

/// `permitted_group_ids(guardian)`: everyone, plus the user's groups.
pub fn permitted_group_ids(guardian: &Guardian) -> Vec<i64> {
    let mut ids: Vec<i64> = vec![0];
    if guardian.user_id().is_some() {
        ids.extend(guardian.group_ids().iter().copied().filter(|g| *g != 0));
    }
    ids
}

/// `DiscourseTagging.readonly_tag_names(guardian)`: tags in a group with a
/// read-only permission (for anyone); none for admins.
pub async fn readonly_tag_names(
    conn: &mut PgConnection,
    guardian: Option<&Guardian>,
) -> Result<Vec<String>, sqlx::Error> {
    if guardian.is_some_and(Guardian::is_admin) {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        "SELECT tags.name FROM tags \
         JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
         JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
         JOIN tag_group_permissions tgp ON tgp.tag_group_id = tg.id \
         WHERE tgp.permission_type = $1 ORDER BY tgp.ctid, tgm.ctid",
    )
    .bind(PERMISSION_READONLY)
    .fetch_all(conn)
    .await
}

/// `DiscourseTagging.permitted_tag_names(guardian)`: tags a full
/// permission lets the user's groups use.
pub async fn permitted_tag_names(
    conn: &mut PgConnection,
    guardian: &Guardian,
) -> Result<Vec<String>, sqlx::Error> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT tags.name FROM tags \
         JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
         JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
         JOIN tag_group_permissions tgp ON tgp.tag_group_id = tg.id \
         WHERE tgp.group_id = ANY($1) AND tgp.permission_type = $2 ORDER BY tgp.ctid, tgm.ctid",
    )
    .bind(permitted_group_ids(guardian))
    .bind(PERMISSION_FULL)
    .fetch_all(conn)
    .await?;
    Ok(uniq(names))
}

/// `DiscourseTagging.hidden_tag_names(guardian)`: tags in a permissioned
/// group none of whose permissions are for the user's groups.
pub async fn hidden_tag_names(
    conn: &mut PgConnection,
    guardian: &Guardian,
) -> Result<Vec<String>, sqlx::Error> {
    if guardian.is_admin() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        "SELECT name FROM tags \
         WHERE id IN (SELECT tgm.tag_id FROM tag_group_memberships tgm \
                      JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
                      JOIN tag_group_permissions tgp ON tgp.tag_group_id = tg.id) \
           AND id NOT IN (SELECT tgm.tag_id FROM tag_group_permissions tgp \
                          JOIN tag_groups tg ON tg.id = tgp.tag_group_id \
                          JOIN tag_group_memberships tgm ON tgm.tag_group_id = tg.id \
                          WHERE tgp.group_id = ANY($1)) \
         ORDER BY ctid",
    )
    .bind(permitted_group_ids(guardian))
    .fetch_all(conn)
    .await
}

fn uniq<T: PartialEq>(items: Vec<T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::with_capacity(items.len());
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// Ruby's `a & b`: a's items in b, without duplicates.
fn intersect(a: &[String], b: &[String]) -> Vec<String> {
    uniq(a.iter().filter(|x| b.contains(x)).cloned().collect())
}

/// Ruby's `a - b`
fn minus(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|x| !b.contains(x)).cloned().collect()
}

/// `guardian.can_tag?(topic)`
pub fn can_tag(
    guardian: &Guardian,
    settings: &SiteSettings,
    private_message: bool,
) -> Result<bool, AppError> {
    Ok(if private_message {
        guardian.can_tag_pms(settings)?
    } else {
        guardian.can_tag_topics(settings)?
    })
}

/// `DiscourseTagging.tags_for_saving(tags_arg, guardian, unlimited:)`: the
/// existing tags named, then, for those who may create tags, the other
/// names cleaned; at most max_tags_per_topic.
pub async fn tags_for_saving(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    names: &[String],
    unlimited: bool,
) -> Result<Vec<String>, AppError> {
    if !guardian.can_tag_topics(settings)? || names.is_empty() {
        return Ok(Vec::new());
    }
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let mut tag_names: Vec<String> =
        sqlx::query_scalar("SELECT name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?;
    if guardian.can_create_tag(settings)? {
        let existing = tag_names.clone();
        for name in minus(names, &existing) {
            tag_names.push(clean_tag(settings, &name)?);
        }
        tag_names.retain(|t| !t.trim().is_empty());
        tag_names = uniq(tag_names);
    }
    if !unlimited {
        let max = settings.get("max_tags_per_topic")?.to_i().max(0) as usize;
        tag_names.truncate(max);
    }
    Ok(tag_names)
}

/// What filter_allowed_tags and the validations read of a category.
#[derive(Debug, Clone)]
pub struct TagCategory {
    pub id: i32,
    pub name: String,
    pub allow_global_tags: bool,
    pub minimum_required_tags: i32,
    pub tag_count: i64,
    pub tag_group_count: i64,
}

impl TagCategory {
    pub async fn load(conn: &mut PgConnection, id: i32) -> Result<Option<Self>, sqlx::Error> {
        let row: Option<(i32, String, bool, Option<i32>, i64, i64)> = sqlx::query_as(
            "SELECT id, name, allow_global_tags, minimum_required_tags, \
               (SELECT count(*) FROM category_tags WHERE category_id = c.id), \
               (SELECT count(*) FROM category_tag_groups WHERE category_id = c.id) \
             FROM categories c WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await?;
        Ok(row.map(
            |(id, name, allow_global_tags, min, tag_count, tag_group_count)| TagCategory {
                id,
                name,
                allow_global_tags,
                minimum_required_tags: min.unwrap_or(0),
                tag_count,
                tag_group_count,
            },
        ))
    }

    /// `has_restricted_tags?`
    pub fn has_restricted_tags(&self) -> bool {
        self.tag_count > 0 || self.tag_group_count > 0
    }
}

/// A row of filter_allowed_tags.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AllowedTag {
    pub id: i32,
    pub name: String,
    pub slug: Option<String>,
    pub topic_count: i32,
    pub pm_topic_count: i32,
    pub description: Option<String>,
    pub tgm_id: Option<i32>,
    pub tag_group_id: Option<i32>,
    pub parent_tag_id: Option<i32>,
    pub one_per_topic: Option<bool>,
    pub target_tag_id: Option<i32>,
}

/// `filter_allowed_tags`'s options. `guardian: None` filters as for an
/// admin (the caller doesn't want guardian rules).
#[derive(Default)]
pub struct FilterOpts<'a> {
    pub guardian: Option<&'a Guardian>,
    pub term: Option<String>,
    pub term_starts_with: bool,
    pub limit: Option<i64>,
    pub category: Option<&'a TagCategory>,
    pub for_input: bool,
    pub for_topic: bool,
    /// `selected_tag_ids`, or `selected_tags` looked up by name.
    pub selected_tag_ids: Vec<i32>,
    pub only_tag_names: Option<Vec<String>>,
    pub exclude_synonyms: bool,
    pub exclude_has_synonyms: bool,
    pub order_search_results: bool,
    pub order_popularity: bool,
    pub order_recent_tag_ids: Vec<i32>,
    pub excluded_tag_names: Vec<String>,
}

/// `with_context`'s required tag group: its name and min_count.
#[derive(Debug, Clone, PartialEq)]
pub struct RequiredTagGroup {
    pub name: String,
    pub min_count: i32,
}

/// `Tag.topic_count_column(guardian)`
pub fn topic_count_column(
    guardian: Option<&Guardian>,
    settings: &SiteSettings,
) -> Result<&'static str, AppError> {
    Ok(match guardian {
        Some(g) => g.tag_count_column(settings)?,
        None => {
            if settings
                .get("include_secure_categories_in_tag_counts")?
                .truthy()
            {
                "staff_topic_count"
            } else {
                "public_topic_count"
            }
        }
    })
}

fn int_list(ids: &[i32]) -> String {
    ids.iter().map(i32::to_string).collect::<Vec<_>>().join(",")
}

/// `category.category_required_tag_groups` (by their order): each group's
/// id, name and min_count.
async fn required_tag_groups(
    conn: &mut PgConnection,
    category_id: i32,
) -> Result<Vec<(i32, String, i32)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT tg.id, tg.name, crtg.min_count FROM category_required_tag_groups crtg \
         JOIN tag_groups tg ON tg.id = crtg.tag_group_id \
         WHERE crtg.category_id = $1 ORDER BY crtg.\"order\", crtg.id",
    )
    .bind(category_id)
    .fetch_all(conn)
    .await
}

/// `DiscourseTagging.filter_allowed_tags(guardian, opts)`, with its
/// context (the required tag group shown to the chooser).
pub async fn filter_allowed_tags(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    opts: &FilterOpts<'_>,
) -> Result<(Vec<AllowedTag>, Option<RequiredTagGroup>), AppError> {
    let category = opts.category;
    let category_has_tag_groups = category.is_some_and(|c| c.tag_group_count > 0);
    let category_has_restricted_tags =
        category_has_tag_groups || category.is_some_and(|c| c.tag_count > 0);
    let filter_for_non_admin = opts.guardian.is_some_and(|g| !g.is_admin());
    let selected = &opts.selected_tag_ids;
    let selected_sql = int_list(selected);

    let term = opts.term.as_deref().filter(|t| !t.trim().is_empty());
    if term.is_some() && settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("tag search over localized tags").into());
    }
    let name_match = if opts.term_starts_with {
        "starts_with(LOWER(t.name), LOWER($2))"
    } else {
        "position(LOWER($2) IN LOWER(t.name)) <> 0"
    };
    let and_name_like = if term.is_some() {
        format!("AND {name_match}")
    } else {
        String::new()
    };

    let mut sql = format!(
        "WITH tag_group_restrictions AS ( \
           SELECT t.id as tag_id, tgm.id as tgm_id, tg.id as tag_group_id, tg.parent_tag_id as parent_tag_id, \
             tg.one_per_topic as one_per_topic \
           FROM tags t \
           LEFT OUTER JOIN tag_group_memberships tgm ON tgm.tag_id = t.id {and_name_like} \
           LEFT OUTER JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
         ), category_restrictions AS ( \
           SELECT t.id as tag_id, ct.id as ct_id, ct.category_id as category_id, NULL AS category_tag_group_id \
           FROM tags t \
           INNER JOIN category_tags ct ON t.id = ct.tag_id {and_name_like} \
           UNION \
           SELECT t.id as tag_id, ctg.id as ctg_id, ctg.category_id as category_id, ctg.tag_group_id AS category_tag_group_id \
           FROM tags t \
           INNER JOIN tag_group_memberships tgm ON tgm.tag_id = t.id {and_name_like} \
           INNER JOIN category_tag_groups ctg ON tgm.tag_group_id = ctg.tag_group_id \
         )"
    );
    let with_permitted = (opts.for_input || opts.for_topic) && filter_for_non_admin;
    if let (true, Some(g)) = (with_permitted, opts.guardian) {
        let groups = permitted_group_ids(g)
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            ", permitted_tag_groups AS ( \
               SELECT tg.id as tag_group_id, tgp.group_id as group_id, tgp.permission_type as permission_type \
               FROM tags t \
               INNER JOIN tag_group_memberships tgm ON tgm.tag_id = t.id {and_name_like} \
               INNER JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
               INNER JOIN tag_group_permissions tgp \
               ON tg.id = tgp.tag_group_id AND group_id IN ({groups}) \
               AND tgp.permission_type = {PERMISSION_FULL} \
             )"
        ));
    }

    let outer_join = category.is_none_or(|c| c.allow_global_tags || !category_has_restricted_tags);
    let count_column = topic_count_column(opts.guardian, settings)?;
    let recent = int_list(&opts.order_recent_tag_ids);
    let distinct = if !opts.order_recent_tag_ids.is_empty() {
        format!("DISTINCT ON (array_position(ARRAY[{recent}]::int[], t.id), {count_column}, name)")
    } else if opts.order_popularity {
        format!("DISTINCT ON ({count_column}, name)")
    } else if opts.order_search_results && term.is_some() {
        format!("DISTINCT ON (lower(name) = lower($2), {count_column}, name)")
    } else {
        String::new()
    };
    sql.push_str(&format!(
        " SELECT {distinct} t.id, t.name, t.slug, t.{count_column} AS topic_count, t.pm_topic_count, \
           t.description, tgr.tgm_id as tgm_id, tgr.tag_group_id as tag_group_id, \
           tgr.parent_tag_id as parent_tag_id, tgr.one_per_topic as one_per_topic, t.target_tag_id \
         FROM tags t \
         INNER JOIN tag_group_restrictions tgr ON tgr.tag_id = t.id \
         {} JOIN category_restrictions cr ON t.id = cr.tag_id \
           AND (tgr.tag_group_id = cr.category_tag_group_id OR cr.category_tag_group_id IS NULL)",
        if outer_join { "LEFT OUTER" } else { "INNER" }
    ));

    let mut wheres: Vec<String> = Vec::new();
    if !opts.for_topic && !selected.is_empty() {
        wheres.push(format!("id NOT IN ({selected_sql})"));
    }
    if opts.only_tag_names.is_some() {
        wheres.push("LOWER(name) IN (SELECT unnest($1::text[]))".to_string());
    }
    if opts.for_input {
        wheres.push(if selected.is_empty() {
            "tgm_id IS NULL OR parent_tag_id IS NULL".to_string()
        } else {
            format!("tgm_id IS NULL OR parent_tag_id IS NULL OR parent_tag_id IN ({selected_sql})")
        });
    }
    match category {
        Some(c) if category_has_restricted_tags => wheres.push(if c.allow_global_tags {
            format!("category_id = {} OR category_id IS NULL", c.id)
        } else {
            format!("category_id = {}", c.id)
        }),
        _ if category.is_some() || opts.for_input || opts.for_topic => {
            wheres.push("category_id IS NULL".to_string());
        }
        _ => {}
    }
    if with_permitted {
        wheres.push(
            "tag_group_id IS NULL OR tag_group_id IN (SELECT tag_group_id FROM permitted_tag_groups)"
                .to_string(),
        );
    }
    if term.is_some() {
        wheres.push(name_match.to_string());
    }

    // The required tag group a category asks the chooser to show first.
    let mut required_tag_ids: Option<Vec<i32>> = None;
    let mut required_group: Option<RequiredTagGroup> = None;
    if let Some(c) = category
        && opts.for_input
        && (filter_for_non_admin || term.is_none())
    {
        let groups = required_tag_groups(conn, c.id).await?;
        if let Some((group_id, name, min_count)) = groups.into_iter().next() {
            let group_tags: Vec<i32> = sqlx::query_scalar(
                "SELECT tag_id FROM tag_group_memberships WHERE tag_group_id = $1 ORDER BY ctid",
            )
            .bind(group_id)
            .fetch_all(&mut *conn)
            .await?;
            let chosen = group_tags.iter().filter(|id| selected.contains(id)).count();
            if (chosen as i32) < min_count
                && (filter_for_non_admin || group_tags.len() as i64 >= opts.limit.unwrap_or(0))
            {
                wheres.push(format!("id IN ({})", int_list(&group_tags)));
                required_group = Some(RequiredTagGroup { name, min_count });
                required_tag_ids = Some(group_tags);
            }
        }
    }
    if let (true, Some(g)) = (filter_for_non_admin, opts.guardian) {
        wheres.push(format!(
            "t.id IN {}",
            crate::tags::visible_tag_ids_subquery(g, settings)?
        ));
    }
    if !selected.is_empty() && (opts.for_input || opts.for_topic) {
        let mut one_sql = format!(
            "SELECT DISTINCT(tg.id) FROM tag_groups tg \
             INNER JOIN tag_group_memberships tgm ON tg.id = tgm.tag_group_id AND tgm.tag_id IN ({selected_sql}) \
             WHERE tg.one_per_topic"
        );
        if let (true, Some(c)) = (category_has_tag_groups, category) {
            one_sql.push_str(&if c.allow_global_tags {
                format!(
                    " AND (tg.id IN (SELECT tag_group_id FROM category_tag_groups WHERE category_id = {0}) \
                     OR tg.id NOT IN (SELECT tag_group_id FROM category_tag_groups))",
                    c.id
                )
            } else {
                format!(
                    " AND tg.id IN (SELECT tag_group_id FROM category_tag_groups WHERE category_id = {})",
                    c.id
                )
            });
        }
        let one_ids: Vec<i32> = sqlx::query_scalar(&one_sql).fetch_all(&mut *conn).await?;
        if !one_ids.is_empty() {
            wheres.push(format!(
                "t.id NOT IN (SELECT DISTINCT tag_id FROM tag_group_restrictions WHERE tag_group_id IN ({})) \
                 OR id IN ({selected_sql})",
                int_list(&one_ids)
            ));
        }
    }
    if opts.exclude_synonyms {
        wheres.push("target_tag_id IS NULL".to_string());
    }
    if opts.exclude_has_synonyms {
        wheres.push(
            "id NOT IN (SELECT target_tag_id FROM tags WHERE target_tag_id IS NOT NULL)"
                .to_string(),
        );
    }
    if !opts.excluded_tag_names.is_empty() {
        wheres.push("name NOT IN (SELECT unnest($3::text[]))".to_string());
    }
    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(
            &wheres
                .iter()
                .map(|w| format!("({w})"))
                .collect::<Vec<_>>()
                .join(" AND "),
        );
    }
    if !opts.order_recent_tag_ids.is_empty() {
        sql.push_str(&format!(
            " ORDER BY array_position(ARRAY[{recent}]::int[], t.id) ASC NULLS LAST, {count_column} DESC, name"
        ));
    } else if opts.order_popularity {
        sql.push_str(&format!(" ORDER BY {count_column} DESC, name"));
    } else if opts.order_search_results && term.is_some() {
        sql.push_str(&format!(
            " ORDER BY lower(name) = lower($2) DESC, {count_column} DESC, name"
        ));
    }
    if let Some(limit) = opts.limit {
        let limit = match (&required_tag_ids, term) {
            (Some(ids), None) => ids.len() as i64,
            _ => limit,
        };
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    let only: Vec<String> = opts
        .only_tag_names
        .iter()
        .flatten()
        .map(|n| n.to_lowercase())
        .collect();
    let rows: Vec<AllowedTag> = sqlx::query_as(&sql)
        .bind(&only)
        .bind(term.unwrap_or(""))
        .bind(&opts.excluded_tag_names)
        .fetch_all(&mut *conn)
        .await?;
    let mut seen: Vec<i32> = Vec::new();
    let rows = rows
        .into_iter()
        .filter(|r| {
            if seen.contains(&r.id) {
                false
            } else {
                seen.push(r.id);
                true
            }
        })
        .collect();
    Ok((rows, required_group))
}

/// A tag as tagging holds it: its id (None for a tag Tag.create failed to
/// save) and name.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldTag {
    pub id: Option<i32>,
    pub name: String,
}

/// `filter_tags_violating_one_tag_from_group_per_topic`: the one per
/// topic groups with more than one of the tags, each with its tags by
/// name.
async fn tags_violating_one_per_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: Option<&Guardian>,
    category: Option<&TagCategory>,
    names: &[String],
) -> Result<Vec<Vec<AllowedTag>>, AppError> {
    if names.len() < 2 {
        return Ok(Vec::new());
    }
    let (allowed, _) = filter_allowed_tags(
        conn,
        settings,
        &FilterOpts {
            guardian,
            category,
            only_tag_names: Some(names.to_vec()),
            for_topic: true,
            order_search_results: true,
            ..Default::default()
        },
    )
    .await?;
    if allowed.len() < 2 {
        return Ok(Vec::new());
    }
    let mut sorted = allowed;
    sorted.sort_by(|a, b| {
        (a.tag_group_id.unwrap_or(-1), &a.name).cmp(&(b.tag_group_id.unwrap_or(-1), &b.name))
    });
    let mut groups: Vec<(Option<i32>, Vec<AllowedTag>)> = Vec::new();
    for tag in sorted {
        if tag.one_per_topic != Some(true) {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == tag.tag_group_id) {
            Some((_, list)) => list.push(tag),
            None => groups.push((tag.tag_group_id, vec![tag])),
        }
    }
    Ok(groups
        .into_iter()
        .filter(|(_, tags)| tags.len() > 1)
        .map(|(_, tags)| tags)
        .collect())
}

/// `validate_one_tag_from_group_per_topic`
async fn validate_one_tag_from_group_per_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    guardian: Option<&Guardian>,
    category: Option<&TagCategory>,
    names: &[String],
    errors: &mut Vec<String>,
) -> Result<bool, AppError> {
    let groups = tags_violating_one_per_topic(conn, settings, guardian, category, names).await?;
    if groups.is_empty() || guardian.is_some_and(Guardian::is_admin) {
        return Ok(true);
    }
    for tags in groups {
        let mut names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        names.sort();
        errors.push(
            i18n.t_with(
                "tags.limited_to_one_tag_from_group",
                &[("tags", &names.join(", "))],
            )
            .unwrap_or_default(),
        );
    }
    Ok(false)
}

/// `validate_min_required_tags_for_category`
fn validate_min_required_tags(
    i18n: &I18n,
    guardian: &Guardian,
    category: Option<&TagCategory>,
    count: usize,
    errors: &mut Vec<String>,
) -> bool {
    match category {
        Some(c)
            if !guardian.is_admin()
                && c.minimum_required_tags > 0
                && (count as i32) < c.minimum_required_tags =>
        {
            errors.push(
                i18n.t_count(
                    "tags.minimum_required_tags",
                    i64::from(c.minimum_required_tags),
                    &[],
                )
                .unwrap_or_default(),
            );
            false
        }
        _ => true,
    }
}

/// `validate_required_tags_from_group`: each of the category's required
/// groups must have min_count of the (saved) tags.
async fn validate_required_tags_from_group(
    conn: &mut PgConnection,
    i18n: &I18n,
    guardian: &Guardian,
    category: Option<&TagCategory>,
    tags: &[HeldTag],
    errors: &mut Vec<String>,
) -> Result<bool, AppError> {
    let Some(category) = category else {
        return Ok(true);
    };
    if guardian.is_admin() {
        return Ok(true);
    }
    let ids: Vec<i32> = tags.iter().filter_map(|t| t.id).collect();
    let mut success = true;
    for (group_id, name, min_count) in required_tag_groups(conn, category.id).await? {
        let in_group: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM tag_group_memberships WHERE tag_group_id = $1 AND tag_id = ANY($2)",
        )
        .bind(group_id)
        .bind(&ids)
        .fetch_one(&mut *conn)
        .await?;
        if (tags.len() as i64) < i64::from(min_count) || in_group < i64::from(min_count) {
            success = false;
            let names: Vec<String> = sqlx::query_scalar(
                "SELECT tags.name FROM tags JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
                 WHERE tgm.tag_group_id = $1 ORDER BY tags.id",
            )
            .bind(group_id)
            .fetch_all(&mut *conn)
            .await?;
            errors.push(
                i18n.t_count(
                    "tags.required_tags_from_group",
                    i64::from(min_count),
                    &[("tag_group_name", &name), ("tags", &names.join(", "))],
                )
                .unwrap_or_default(),
            );
        }
    }
    Ok(success)
}

/// `restricted_category_ids_by_tag_name`: the categories each named tag is
/// restricted to (through its tag groups, then directly), by name.
async fn restricted_category_ids_by_tag_name(
    conn: &mut PgConnection,
    names: &[String],
) -> Result<Vec<(String, Vec<i32>)>, sqlx::Error> {
    let mut out: Vec<(String, Vec<i32>)> = Vec::new();
    if names.is_empty() {
        return Ok(out);
    }
    let via_groups: Vec<(String, i32)> = sqlx::query_as(
        "SELECT tags.name, categories.id FROM tags \
         JOIN tag_group_memberships tgm ON tgm.tag_id = tags.id \
         JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
         JOIN category_tag_groups ctg ON ctg.tag_group_id = tg.id \
         JOIN categories ON categories.id = ctg.category_id \
         WHERE tags.name = ANY($1) ORDER BY tgm.ctid, ctg.ctid",
    )
    .bind(names)
    .fetch_all(&mut *conn)
    .await?;
    let direct: Vec<(String, i32)> = sqlx::query_as(
        "SELECT tags.name, categories.id FROM tags \
         JOIN category_tags ct ON ct.tag_id = tags.id \
         JOIN categories ON categories.id = ct.category_id \
         WHERE tags.name = ANY($1) ORDER BY ct.ctid",
    )
    .bind(names)
    .fetch_all(&mut *conn)
    .await?;
    for (name, category_id) in via_groups.into_iter().chain(direct) {
        match out.iter_mut().find(|(n, _)| *n == name) {
            Some((_, ids)) => {
                if !ids.contains(&category_id) {
                    ids.push(category_id);
                }
            }
            None => out.push((name, vec![category_id])),
        }
    }
    Ok(out)
}

/// `tag_names_not_allowed_in_category`
async fn tag_names_not_allowed_in_category(
    conn: &mut PgConnection,
    category: Option<&TagCategory>,
    names: &[String],
) -> Result<Vec<String>, sqlx::Error> {
    let Some(category) = category else {
        return Ok(Vec::new());
    };
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let restricted = restricted_category_ids_by_tag_name(conn, names).await?;
    Ok(names
        .iter()
        .filter(|n| {
            restricted
                .iter()
                .any(|(r, ids)| r == *n && !ids.contains(&category.id))
        })
        .cloned()
        .collect())
}

/// `validate_category_restricted_tags`
async fn validate_category_restricted_tags(
    conn: &mut PgConnection,
    i18n: &I18n,
    guardian: &Guardian,
    category: Option<&TagCategory>,
    names: &[String],
    errors: &mut Vec<String>,
) -> Result<bool, AppError> {
    let Some(category) = category else {
        return Ok(true);
    };
    if names.is_empty() {
        return Ok(true);
    }
    let restricted = restricted_category_ids_by_tag_name(conn, names).await?;
    let mut unallowed: Vec<&str> = restricted
        .iter()
        .filter(|(_, ids)| !ids.contains(&category.id))
        .map(|(n, _)| n.as_str())
        .collect();
    if !unallowed.is_empty() {
        if guardian.is_admin() {
            return Ok(true);
        }
        let count = unallowed.len() as i64;
        unallowed.sort();
        errors.push(
            i18n.t_count(
                "tags.forbidden.restricted_tags_cannot_be_used_in_category",
                count,
                &[
                    ("tags", &unallowed.join(", ")),
                    ("category", &category.name),
                ],
            )
            .unwrap_or_default(),
        );
        return Ok(false);
    }
    if !category.allow_global_tags && category.has_restricted_tags() {
        let keys: Vec<String> = restricted.iter().map(|(n, _)| n.clone()).collect();
        let mut unrestricted = minus(names, &keys);
        if !unrestricted.is_empty() {
            if guardian.is_admin() {
                return Ok(true);
            }
            let count = unrestricted.len() as i64;
            unrestricted.sort();
            errors.push(
                i18n.t_count(
                    "tags.forbidden.category_does_not_allow_tags",
                    count,
                    &[
                        ("tags", &unrestricted.join(", ")),
                        ("category", &category.name),
                    ],
                )
                .unwrap_or_default(),
            );
            return Ok(false);
        }
    }
    Ok(true)
}

/// `Tag.where(name: names)` (exact names), in the table's order.
async fn tags_named_exactly(
    conn: &mut PgConnection,
    names: &[String],
) -> Result<Vec<HeldTag>, sqlx::Error> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM tags WHERE name = ANY($1) ORDER BY ctid")
            .bind(names)
            .fetch_all(conn)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| HeldTag { id: Some(id), name })
        .collect())
}

/// `DiscourseTagging.validate_category_tags` (TopicCreator#valid?, with
/// a category and a guardian that may tag the topic): the errors, empty
/// when the tags are fine. Stops at the first failing check.
pub async fn validate_category_tags(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    category: &TagCategory,
    names: &[String],
) -> Result<Vec<String>, AppError> {
    let existing = tags_named_exactly(conn, names).await?;
    let can_create = guardian.can_create_tag(settings)?;
    let valid_names: Vec<String> = if can_create {
        names.to_vec()
    } else {
        existing.iter().map(|t| t.name.clone()).collect()
    };
    let mut errors = Vec::new();
    let category = Some(category);
    if !validate_min_required_tags(i18n, guardian, category, valid_names.len(), &mut errors) {
        return Ok(errors);
    }
    if !validate_required_tags_from_group(conn, i18n, guardian, category, &existing, &mut errors)
        .await?
    {
        return Ok(errors);
    }
    if !validate_category_restricted_tags(conn, i18n, guardian, category, &valid_names, &mut errors)
        .await?
    {
        return Ok(errors);
    }
    validate_one_tag_from_group_per_topic(
        conn,
        settings,
        i18n,
        Some(guardian),
        category,
        &valid_names,
        &mut errors,
    )
    .await?;
    Ok(errors)
}

/// A tag Tag.create saved, with its tag_created payload (TagSerializer of
/// the new row, counts and all at zero), for the web hooks after commit.
#[derive(Debug, Clone)]
pub struct CreatedTag {
    pub id: i32,
    pub payload: Option<String>,
}

/// `Tag.create(name:)`: None when its validations refuse it (a blank or
/// reserved name, or a name another tag has); else the row, its slug
/// (ensure_slug) and search data, and its tag_created payload when a web
/// hook listens.
pub async fn create_tag(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    name: &str,
) -> Result<Option<CreatedTag>, AppError> {
    if name.trim().is_empty() || RESERVED_TAGS.contains(&name.trim().to_lowercase().as_str()) {
        return Ok(None);
    }
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tags WHERE lower(name) = lower($1))")
            .bind(name)
            .fetch_one(&mut *conn)
            .await?;
    if taken {
        return Ok(None);
    }
    // ensure_slug: Slug.for(name, ""), none when another tag has it.
    let slug = crate::posting::text::slug_with_default(name, "en", "")?;
    let slug_taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tags WHERE lower(slug) = lower($1))")
            .bind(&slug)
            .fetch_one(&mut *conn)
            .await?;
    let slug = if slug.is_empty() || slug_taken {
        String::new()
    } else {
        slug
    };
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO tags (name, slug, created_at, updated_at) \
         VALUES ($1, $2, clock_timestamp(), clock_timestamp()) RETURNING id",
    )
    .bind(name)
    .bind(&slug)
    .fetch_one(&mut *conn)
    .await?;
    crate::posting::search_index::index_tag(conn, settings, id, &name.to_lowercase()).await?;
    let payload = if crate::web_hooks::active(conn, "tag_created").await? {
        let tag = crate::tags::Tag::find(conn, id)
            .await?
            .ok_or(Unsupported("a tag that vanished"))?;
        let guardian = crate::web_hooks::system_guardian(conn).await?;
        let serialized = tag.serialize(conn, &guardian, settings).await?;
        Some(crate::web_hooks::to_json(&serialized))
    } else {
        None
    };
    Ok(Some(CreatedTag { id, payload }))
}

/// Why Tag's validations refuse a name, as `errors.full_messages`:
/// presence, uniqueness (ignoring case), then the reserved names.
pub async fn tag_name_errors(
    conn: &mut PgConnection,
    i18n: &I18n,
    name: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let message = |key: &str| {
        format!(
            "Name {}",
            i18n.t(&format!("activerecord.errors.messages.{key}"))
                .or_else(|| i18n.t(&format!("errors.messages.{key}")))
                .unwrap_or(key)
        )
    };
    let mut errors = Vec::new();
    if name.trim().is_empty() {
        errors.push(message("blank"));
        return Ok(errors);
    }
    let taken: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tags WHERE lower(name) = lower($1))")
            .bind(name)
            .fetch_one(conn)
            .await?;
    if taken {
        errors.push(message("taken"));
    }
    if RESERVED_TAGS.contains(&name.trim().to_lowercase().as_str()) {
        errors.push(message("invalid"));
    }
    Ok(errors)
}

/// `DiscourseTagging.find_or_create_tags!(names, guardian)`: the tags for
/// the names tags_for_saving keeps, the missing ones created
/// (`Tag.create!`, its tag_created web hooks at once), by `where_name`;
/// Err with the errors of the first tag Tag.create! refuses.
pub async fn find_or_create_tags(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    names: &[String],
) -> Result<Result<Vec<(i32, String)>, Vec<String>>, AppError> {
    let tag_names = tags_for_saving(conn, settings, guardian, names, false).await?;
    if tag_names.is_empty() {
        return Ok(Ok(Vec::new()));
    }
    let lower: Vec<String> = tag_names.iter().map(|n| n.to_lowercase()).collect();
    let existing: Vec<String> =
        sqlx::query_scalar("SELECT name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?;
    for name in minus(&tag_names, &existing) {
        match create_tag(conn, settings, &name).await? {
            Some(tag) => enqueue_tag_created(conn, std::slice::from_ref(&tag)).await?,
            None => return Ok(Err(tag_name_errors(conn, i18n, &name).await?)),
        }
    }
    Ok(Ok(sqlx::query_as(
        "SELECT id, name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid",
    )
    .bind(&lower)
    .fetch_all(&mut *conn)
    .await?))
}

/// `trigger_tag_created_event` after commit: the tag_created web hooks.
pub async fn enqueue_tag_created(
    conn: &mut PgConnection,
    created: &[CreatedTag],
) -> Result<(), AppError> {
    for tag in created {
        if let Some(payload) = &tag.payload {
            let mut opts = Map::new();
            opts.insert("id".into(), json!(tag.id));
            opts.insert("payload".into(), json!(payload));
            crate::web_hooks::enqueue_hooks(conn, "tag", "tag_created", opts).await?;
        }
    }
    Ok(())
}

/// The topic being tagged: a saved one (`id`) or one TopicCreator is
/// about to save, with the category it was given (`topic.category` before
/// the save fills in uncategorized).
#[derive(Debug, Clone)]
pub struct TaggedTopic {
    pub id: Option<i32>,
    pub private_message: bool,
    pub category_id: Option<i32>,
}

/// What tagging a topic settled on: its tags in order, and the tags it
/// created on the way.
#[derive(Debug, Clone, Default)]
pub struct Tagging {
    pub tags: Vec<HeldTag>,
    pub created: Vec<CreatedTag>,
    /// The names before and after (topic_tags_changed's payload).
    pub old_tag_names: Vec<String>,
}

impl Tagging {
    /// A tag Tag.create couldn't save: the topic's save fails with "Tags
    /// is invalid" (its autosaved association).
    pub fn has_invalid_tag(&self) -> bool {
        self.tags.iter().any(|t| t.id.is_none())
    }

    pub fn tag_ids(&self) -> Vec<i32> {
        self.tags.iter().filter_map(|t| t.id).collect()
    }
}

/// Why tagging stopped: the topic's errors, and the tags it had created
/// by then (saved, their web hooks still due).
#[derive(Debug, Clone, Default)]
pub struct Rejected {
    pub errors: Vec<String>,
    pub created: Vec<CreatedTag>,
}

impl Rejected {
    fn new(errors: Vec<String>) -> Self {
        Rejected {
            errors,
            created: Vec::new(),
        }
    }
}

/// `"Tags is invalid"`: the topic's error for an unsaved tag.
pub fn invalid_tags_message(i18n: &I18n) -> String {
    format!(
        "Tags {}",
        i18n.t("activerecord.errors.messages.invalid")
            .or_else(|| i18n.t("errors.messages.invalid"))
            .unwrap_or("is invalid")
    )
}

/// The topic's current tag names, by its topic tags.
async fn current_tag_names(
    conn: &mut PgConnection,
    topic_id: Option<i32>,
) -> Result<Vec<String>, sqlx::Error> {
    let Some(id) = topic_id else {
        return Ok(Vec::new());
    };
    sqlx::query_scalar(
        "SELECT tags.name FROM topic_tags JOIN tags ON tags.id = topic_tags.tag_id \
         WHERE topic_tags.topic_id = $1 ORDER BY topic_tags.id",
    )
    .bind(id)
    .fetch_all(conn)
    .await
}

/// `DiscourseTagging.tag_topic(topic, guardian, tags_param)`
pub async fn tag_topic(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    topic: &TaggedTopic,
    param: &TagsParam,
) -> Result<Result<Tagging, Rejected>, AppError> {
    let names = names_for_tagging(conn, param).await?;
    tag_topic_by_names(conn, settings, i18n, guardian, topic, &names, false).await
}

/// `DiscourseTagging.tag_topic_by_names(topic, guardian, tag_names, append:)`:
/// the topic's tags (creating new ones where allowed), or the errors that
/// stop them. Nothing about the topic is written: `apply` does that.
pub async fn tag_topic_by_names(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    i18n: &I18n,
    guardian: &Guardian,
    topic: &TaggedTopic,
    names: &[String],
    append: bool,
) -> Result<Result<Tagging, Rejected>, AppError> {
    if !can_tag(guardian, settings, topic.private_message)? {
        return Ok(Err(Rejected::new(vec![
            i18n.t("tags.user_not_permitted")
                .unwrap_or_default()
                .to_string(),
        ])));
    }
    let mut tag_names = tags_for_saving(conn, settings, guardian, names, false).await?;
    if !tag_names.is_empty() {
        // Synonyms give way to their target.
        let lower: Vec<String> = tag_names.iter().map(|n| n.to_lowercase()).collect();
        let synonyms: Vec<(String, String)> = sqlx::query_as(
            "SELECT tags.name, target.name FROM tags JOIN tags target ON target.id = tags.target_tag_id \
             WHERE lower(tags.name) = ANY($1) ORDER BY tags.ctid",
        )
        .bind(&lower)
        .fetch_all(&mut *conn)
        .await?;
        for (name, target) in synonyms {
            let at = tag_names
                .iter()
                .position(|n| *n == name)
                .ok_or(Unsupported(
                    "a synonym named in another case (a TypeError in Rails)",
                ))?;
            tag_names[at] = target;
        }
    }
    let old_tag_names = current_tag_names(conn, topic.id).await?;
    let new_tag_names = minus(&tag_names, &old_tag_names);
    let removed_tag_names = minus(&old_tag_names, &tag_names);

    let mut readonly = readonly_tag_names(conn, Some(guardian)).await?;
    let hidden = hidden_tag_names(conn, guardian).await?;
    let permitted = permitted_tag_names(conn, guardian).await?;
    if !permitted.is_empty() {
        readonly = minus(&readonly, &permitted);
    }
    let mut disallowed = intersect(&new_tag_names, &readonly);
    disallowed.extend(intersect(&new_tag_names, &hidden));
    if !disallowed.is_empty() {
        return Ok(Err(Rejected::new(vec![
            i18n.t_with(
                "tags.restricted_tag_disallowed",
                &[("tag", &disallowed.join(" "))],
            )
            .unwrap_or_default(),
        ])));
    }
    let removed_readonly = intersect(&removed_tag_names, &readonly);
    if !removed_readonly.is_empty() {
        return Ok(Err(Rejected::new(vec![
            i18n.t_with(
                "tags.restricted_tag_remove_disallowed",
                &[("tag", &removed_readonly.join(" "))],
            )
            .unwrap_or_default(),
        ])));
    }
    tag_names.extend(intersect(&removed_tag_names, &hidden));

    let category = match topic.category_id {
        Some(id) => TagCategory::load(conn, id).await?,
        None => None,
    };
    let category = category.as_ref();
    if append {
        tag_names.extend(old_tag_names.iter().cloned());
    }

    let mut errors = Vec::new();
    let mut created = Vec::new();
    let tags: Vec<HeldTag> = if !tag_names.is_empty() {
        // guardian nil: the tags that passed the checks above stay.
        let lower: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
                .bind(
                    tag_names
                        .iter()
                        .map(|n| n.to_lowercase())
                        .collect::<Vec<_>>(),
                )
                .fetch_all(&mut *conn)
                .await?;
        let (allowed, _) = filter_allowed_tags(
            conn,
            settings,
            &FilterOpts {
                guardian: None,
                for_topic: true,
                category,
                selected_tag_ids: lower,
                only_tag_names: Some(tag_names.clone()),
                ..Default::default()
            },
        )
        .await?;
        let mut ids: Vec<i32> = allowed.iter().map(|t| t.id).collect();
        // Keep the topic's tags the user can't use.
        let kept = tags_named_exactly(conn, &intersect(&old_tag_names, &tag_names)).await?;
        ids.extend(kept.iter().filter_map(|t| t.id));
        let mut tags: Vec<HeldTag> = if ids.is_empty() {
            Vec::new()
        } else {
            let rows: Vec<(i32, String)> =
                sqlx::query_as("SELECT id, name FROM tags WHERE id = ANY($1) ORDER BY ctid")
                    .bind(&ids)
                    .fetch_all(&mut *conn)
                    .await?;
            rows.into_iter()
                .map(|(id, name)| HeldTag { id: Some(id), name })
                .collect()
        };
        if tags.len() < tag_names.len()
            && category
                .is_none_or(|c| c.allow_global_tags || (c.tag_count == 0 && c.tag_group_count == 0))
        {
            for name in &tag_names {
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM tags WHERE lower(name) = lower($1))",
                )
                .bind(name)
                .fetch_one(&mut *conn)
                .await?;
                if exists {
                    continue;
                }
                match create_tag(conn, settings, name).await? {
                    Some(tag) => {
                        tags.push(HeldTag {
                            id: Some(tag.id),
                            name: name.clone(),
                        });
                        created.push(tag);
                    }
                    None => tags.push(HeldTag {
                        id: None,
                        name: name.clone(),
                    }),
                }
            }
        }

        let tag_name_list: Vec<String> = tags.iter().map(|t| t.name.clone()).collect();
        if !validate_one_tag_from_group_per_topic(
            conn,
            settings,
            i18n,
            None,
            category,
            &tag_name_list,
            &mut errors,
        )
        .await?
        {
            return Ok(Err(Rejected { errors, created }));
        }

        // Mandatory parent tags.
        let tag_ids: Vec<i32> = tags.iter().filter_map(|t| t.id).collect();
        let mut parent_sql = "SELECT tgm.tag_id, tg.parent_tag_id FROM tag_groups tg \
             INNER JOIN tag_group_memberships tgm ON tgm.tag_group_id = tg.id \
             WHERE tg.parent_tag_id IS NOT NULL AND tgm.tag_id = ANY($1)"
            .to_string();
        if let Some(c) = category {
            if c.has_restricted_tags() {
                parent_sql.push_str(&if c.allow_global_tags {
                    format!(
                        " AND (tg.id NOT IN (SELECT tag_group_id FROM category_tag_groups) \
                         OR tg.id IN (SELECT tag_group_id FROM category_tag_groups WHERE category_id = {}))",
                        c.id
                    )
                } else {
                    format!(
                        " AND tg.id IN (SELECT tag_group_id FROM category_tag_groups WHERE category_id = {})",
                        c.id
                    )
                });
            } else {
                parent_sql
                    .push_str(" AND tg.id NOT IN (SELECT tag_group_id FROM category_tag_groups)");
            }
        }
        parent_sql.push_str(" ORDER BY tgm.ctid");
        let parent_rows: Vec<(i32, i32)> = sqlx::query_as(&parent_sql)
            .bind(&tag_ids)
            .fetch_all(&mut *conn)
            .await?;
        let mut parent_map: Vec<(i32, Vec<i32>)> = Vec::new();
        for (tag_id, parent_id) in parent_rows {
            match parent_map.iter_mut().find(|(t, _)| *t == tag_id) {
                Some((_, parents)) => parents.push(parent_id),
                None => parent_map.push((tag_id, vec![parent_id])),
            }
        }
        let mut missing: Vec<i32> = Vec::new();
        for (_, parents) in &parent_map {
            if !parents.iter().any(|p| tag_ids.contains(p)) {
                for p in parents {
                    if !missing.contains(p) {
                        missing.push(*p);
                    }
                }
            }
        }
        if !missing.is_empty() {
            let rows: Vec<(i32, String)> =
                sqlx::query_as("SELECT id, name FROM tags WHERE id = ANY($1) ORDER BY ctid")
                    .bind(&missing)
                    .fetch_all(&mut *conn)
                    .await?;
            tags.extend(
                rows.into_iter()
                    .map(|(id, name)| HeldTag { id: Some(id), name }),
            );
        }
        let all_names: Vec<String> = tags.iter().map(|t| t.name.clone()).collect();
        let conflicts =
            tags_violating_one_per_topic(conn, settings, None, category, &all_names).await?;
        if !conflicts.is_empty() {
            // The child each added parent came in for.
            let mut parent_child: Vec<(String, String)> = Vec::new();
            for (tag_id, parents) in &parent_map {
                if parents.iter().any(|p| tag_ids.contains(p)) {
                    continue;
                }
                let Some(child) = tags.iter().find(|t| t.id == Some(*tag_id)) else {
                    continue;
                };
                for parent_id in parents {
                    if let Some(parent) = tags.iter().find(|t| t.id == Some(*parent_id)) {
                        match parent_child.iter_mut().find(|(p, _)| *p == parent.name) {
                            Some((_, c)) => *c = child.name.clone(),
                            None => parent_child.push((parent.name.clone(), child.name.clone())),
                        }
                    }
                }
            }
            for group in conflicts {
                let mut names: Vec<String> = group
                    .iter()
                    .map(|t| {
                        parent_child
                            .iter()
                            .find(|(p, _)| *p == t.name)
                            .map(|(_, c)| c.clone())
                            .filter(|c| !c.is_empty())
                            .unwrap_or_else(|| t.name.clone())
                    })
                    .collect();
                names = uniq(names);
                names.sort();
                errors.push(
                    i18n.t_with(
                        "tags.limited_to_one_tag_from_group",
                        &[("tags", &names.join(", "))],
                    )
                    .unwrap_or_default(),
                );
            }
            return Ok(Err(Rejected { errors, created }));
        }

        if !validate_min_required_tags(i18n, guardian, category, tags.len(), &mut errors) {
            return Ok(Err(Rejected { errors, created }));
        }
        if !validate_required_tags_from_group(conn, i18n, guardian, category, &tags, &mut errors)
            .await?
        {
            return Ok(Err(Rejected { errors, created }));
        }
        if tags.is_empty() {
            let not_allowed =
                tag_names_not_allowed_in_category(conn, category, &new_tag_names).await?;
            if let (false, Some(c)) = (not_allowed.is_empty(), category) {
                let mut sorted = not_allowed.clone();
                sorted.sort();
                errors.push(
                    i18n.t_count(
                        "tags.forbidden.tag_not_allowed_in_category",
                        not_allowed.len() as i64,
                        &[("tags", &sorted.join(", ")), ("category", &c.name)],
                    )
                    .unwrap_or_default(),
                );
            } else {
                errors.push(
                    i18n.t_count("tags.forbidden.invalid", new_tag_names.len() as i64, &[])
                        .unwrap_or_default(),
                );
            }
            return Ok(Err(Rejected { errors, created }));
        }
        tags
    } else {
        if !validate_min_required_tags(i18n, guardian, category, 0, &mut errors) {
            return Ok(Err(Rejected { errors, created }));
        }
        if !validate_required_tags_from_group(conn, i18n, guardian, category, &[], &mut errors)
            .await?
        {
            return Ok(Err(Rejected { errors, created }));
        }
        Vec::new()
    };
    topic_tags_changed_unsupported(conn, settings).await?;
    Ok(Ok(Tagging {
        tags,
        created,
        old_tag_names,
    }))
}

/// `DiscourseTagging.add_or_create_tags_by_name(topic, names)` for a topic
/// TopicCreator hasn't saved (no tags yet): as the system user, the tags
/// named, their synonyms, and new tags for the names no tag has.
pub async fn add_or_create_tags_by_name(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    names: &[String],
) -> Result<Tagging, AppError> {
    let system = crate::web_hooks::system_guardian(conn).await?;
    let tag_names = tags_for_saving(conn, settings, &system, names, false).await?;
    let mut tagging = Tagging::default();
    if tag_names.is_empty() {
        return Ok(tagging);
    }
    let lower: Vec<String> = tag_names.iter().map(|n| n.to_lowercase()).collect();
    let found: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM tags WHERE lower(name) = ANY($1) ORDER BY ctid")
            .bind(&lower)
            .fetch_all(&mut *conn)
            .await?;
    let new_names: Vec<String> = if found.len() < tag_names.len() {
        let found_names: Vec<String> = found.iter().map(|(_, n)| n.clone()).collect();
        minus(&tag_names, &found_names)
    } else {
        Vec::new()
    };
    let ids: Vec<i32> = found.iter().map(|(id, _)| *id).collect();
    let synonyms: Vec<(i32, String)> = sqlx::query_as(
        "SELECT id, name FROM tags WHERE target_tag_id = ANY($1) AND NOT (id = ANY($1)) ORDER BY ctid",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    tagging.tags = found
        .into_iter()
        .chain(synonyms)
        .map(|(id, name)| HeldTag { id: Some(id), name })
        .collect();
    for name in new_names {
        match create_tag(conn, settings, &name).await? {
            Some(tag) => {
                tagging.tags.push(HeldTag {
                    id: Some(tag.id),
                    name,
                });
                tagging.created.push(tag);
            }
            None => tagging.tags.push(HeldTag { id: None, name }),
        }
    }
    Ok(tagging)
}

/// DiscourseEvent :topic_tags_changed's listeners that would act:
/// discourse-automation's automations on it and the boards plugin's sync
/// aren't ported.
async fn topic_tags_changed_unsupported(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<(), AppError> {
    let automations: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM discourse_automation_automations \
         WHERE enabled AND trigger = 'topic_tags_changed')",
    )
    .fetch_one(conn)
    .await?;
    if automations {
        return Err(Unsupported("automations on topic_tags_changed").into());
    }
    if settings.get("boards_enabled").is_ok_and(|v| v.truthy()) {
        return Err(Unsupported("the boards plugin's topic sync").into());
    }
    Ok(())
}

/// TopicCreator#setup_tags' watched words: the topic's tag names plus the
/// replacements of the tag words in its text, or None when there are no
/// tag words.
pub async fn watched_word_tags(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    text: &str,
    current: &[String],
) -> Result<Option<Vec<String>>, AppError> {
    let words =
        crate::word_watcher::words_for_action(conn, settings, crate::word_watcher::TAG).await?;
    if words.is_empty() {
        return Ok(None);
    }
    let mut names = current.to_vec();
    for w in words {
        let re = crate::word_watcher::ruby_regex(&w.regexp(true), w.case_sensitive, w.regex)?;
        if re
            .is_match(text)
            .map_err(|_| Unsupported("watched word regexp backtracking limit"))?
        {
            names.extend(
                w.replacement
                    .as_deref()
                    .unwrap_or("")
                    .split(',')
                    .map(str::to_string),
            );
        }
    }
    Ok(Some(names))
}

/// The topic as TopicTag's callbacks read it.
pub struct SavedTopic {
    pub id: i32,
    pub private_message: bool,
    pub category_id: Option<i32>,
}

/// `topic.tags = tags`: the topic tags no longer wanted destroyed, then
/// the new ones created, each with TopicTag's counter callbacks.
pub async fn apply(
    conn: &mut PgConnection,
    topic: &SavedTopic,
    tagging: &Tagging,
) -> Result<(), AppError> {
    let wanted = tagging.tag_ids();
    let current: Vec<(i32, i32)> =
        sqlx::query_as("SELECT id, tag_id FROM topic_tags WHERE topic_id = $1 ORDER BY id")
            .bind(topic.id)
            .fetch_all(&mut *conn)
            .await?;
    for (id, tag_id) in &current {
        if !wanted.contains(tag_id) {
            sqlx::query("DELETE FROM topic_tags WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
            topic_tag_destroyed(conn, topic, *tag_id).await?;
        }
    }
    let mut added: Vec<i32> = Vec::new();
    for tag_id in wanted {
        if current.iter().any(|(_, t)| *t == tag_id) || added.contains(&tag_id) {
            continue;
        }
        added.push(tag_id);
        sqlx::query(
            "INSERT INTO topic_tags (topic_id, tag_id, created_at, updated_at) \
             VALUES ($1, $2, clock_timestamp(), clock_timestamp())",
        )
        .bind(topic.id)
        .bind(tag_id)
        .execute(&mut *conn)
        .await?;
        topic_tag_created(conn, topic, tag_id).await?;
    }
    Ok(())
}

async fn public_category(
    conn: &mut PgConnection,
    category_id: Option<i32>,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM categories WHERE id = $1 AND read_restricted = FALSE)",
    )
    .bind(category_id)
    .fetch_one(conn)
    .await
}

/// TopicTag's after_create: the tag's counts and its category stat.
async fn topic_tag_created(
    conn: &mut PgConnection,
    topic: &SavedTopic,
    tag_id: i32,
) -> Result<(), AppError> {
    if topic.private_message {
        sqlx::query(
            "UPDATE tags SET pm_topic_count = COALESCE(pm_topic_count, 0) + 1 WHERE id = $1",
        )
        .bind(tag_id)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }
    let public = public_category(conn, topic.category_id).await?;
    sqlx::query(
        "UPDATE tags SET staff_topic_count = COALESCE(staff_topic_count, 0) + 1, \
           public_topic_count = COALESCE(public_topic_count, 0) + CASE WHEN $2 THEN 1 ELSE 0 END \
         WHERE id = $1",
    )
    .bind(tag_id)
    .bind(public)
    .execute(&mut *conn)
    .await?;
    let updated = sqlx::query(
        "UPDATE category_tag_stats SET topic_count = COALESCE(topic_count, 0) + 1 \
         WHERE id = (SELECT id FROM category_tag_stats WHERE tag_id = $1 AND category_id IS NOT DISTINCT FROM $2 \
                     ORDER BY id LIMIT 1)",
    )
    .bind(tag_id)
    .bind(topic.category_id)
    .execute(&mut *conn)
    .await?;
    if updated.rows_affected() == 0 {
        sqlx::query(
            "INSERT INTO category_tag_stats (tag_id, category_id, topic_count) VALUES ($1, $2, 1)",
        )
        .bind(tag_id)
        .bind(topic.category_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// TopicTag's after_destroy: its category stat, then the tag's counts.
pub(crate) async fn topic_tag_destroyed(
    conn: &mut PgConnection,
    topic: &SavedTopic,
    tag_id: i32,
) -> Result<(), AppError> {
    if topic.private_message {
        sqlx::query(
            "UPDATE tags SET pm_topic_count = COALESCE(pm_topic_count, 0) - 1 WHERE id = $1",
        )
        .bind(tag_id)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }
    let stat: Option<(i32, i32)> = sqlx::query_as(
        "SELECT id, topic_count FROM category_tag_stats \
         WHERE tag_id = $1 AND category_id IS NOT DISTINCT FROM $2 ORDER BY id LIMIT 1",
    )
    .bind(tag_id)
    .bind(topic.category_id)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((id, count)) = stat {
        if count == 1 {
            sqlx::query("DELETE FROM category_tag_stats WHERE id = $1")
                .bind(id)
                .execute(&mut *conn)
                .await?;
        } else {
            sqlx::query(
                "UPDATE category_tag_stats SET topic_count = topic_count - 1 WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *conn)
            .await?;
        }
    }
    let public = public_category(conn, topic.category_id).await?;
    sqlx::query(
        "UPDATE tags SET staff_topic_count = COALESCE(staff_topic_count, 0) - 1, \
           public_topic_count = COALESCE(public_topic_count, 0) - CASE WHEN $2 THEN 1 ELSE 0 END \
         WHERE id = $1",
    )
    .bind(tag_id)
    .bind(public)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Topic's after_save with `tags_changed`: its posts queued for a reindex
/// (SearchIndexer.queue_post_reindex), then TagUser.auto_watch and
/// auto_track for the topic.
pub async fn after_tags_saved(conn: &mut PgConnection, topic_id: i32) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE post_search_data SET version = $2 FROM posts \
         WHERE post_search_data.post_id = posts.id AND posts.topic_id = $1",
    )
    .bind(topic_id)
    .bind(REINDEX_VERSION)
    .execute(&mut *conn)
    .await?;
    auto_watch(conn, Some(topic_id), &[]).await?;
    auto_track(conn, Some(topic_id), &[]).await?;
    Ok(())
}

/// `SearchIndexer::REINDEX_VERSION`
const REINDEX_VERSION: i32 = 0;
/// `NotificationLevels`
const REGULAR: i32 = 1;
const TRACKING: i32 = 2;
const WATCHING: i32 = 3;
use crate::posting::notification_reasons::{AUTO_TRACK_TAG, AUTO_WATCH_TAG, USER_CHANGED};

/// `TagUser.auto_watch(topic_id:, user_ids:)`
pub async fn auto_watch(
    conn: &mut PgConnection,
    topic_id: Option<i32>,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    let mut wheres = vec![
        format!("tu.notification_level IN ({TRACKING}, {REGULAR}, {WATCHING})"),
        format!("COALESCE(tu.notifications_reason_id, 0) != {USER_CHANGED}"),
    ];
    if topic_id.is_some() {
        wheres.push("tu.topic_id = $1".to_string());
    }
    if !user_ids.is_empty() {
        wheres.push("tu.user_id = ANY($2)".to_string());
    }
    let sql = format!(
        "UPDATE topic_users \
            SET notification_level = CASE WHEN should_watch THEN {WATCHING} ELSE {TRACKING} END, \
                notifications_reason_id = CASE WHEN should_watch THEN {AUTO_WATCH_TAG} ELSE NULL END \
           FROM ( \
             SELECT tu.topic_id, tu.user_id, \
                    CASE WHEN MAX(tag_users.notification_level) = {WATCHING} THEN true ELSE false END should_watch, \
                    CASE WHEN MAX(tag_users.notification_level) IS NULL AND tu.notification_level = {WATCHING} \
                         AND tu.notifications_reason_id = {AUTO_WATCH_TAG} THEN true ELSE false END should_track \
               FROM topic_users tu \
          LEFT JOIN topic_tags ON tu.topic_id = topic_tags.topic_id \
          LEFT JOIN tag_users ON tag_users.user_id = tu.user_id AND topic_tags.tag_id = tag_users.tag_id \
                    AND tag_users.notification_level = {WATCHING} \
              WHERE {} \
           GROUP BY tu.topic_id, tu.user_id, tu.notification_level, tu.notifications_reason_id \
           ) AS X \
          WHERE X.topic_id = topic_users.topic_id AND X.user_id = topic_users.user_id \
            AND (should_track OR should_watch)",
        wheres
            .iter()
            .map(|w| format!("({w})"))
            .collect::<Vec<_>>()
            .join(" AND ")
    );
    sqlx::query(&sql)
        .bind(topic_id)
        .bind(user_ids)
        .execute(conn)
        .await?;
    Ok(())
}

/// `TagUser.auto_track(topic_id:, user_ids:)`
pub async fn auto_track(
    conn: &mut PgConnection,
    topic_id: Option<i32>,
    user_ids: &[i32],
) -> Result<(), sqlx::Error> {
    let mut wheres = Vec::new();
    if topic_id.is_some() {
        wheres.push("(tu.topic_id = $1)".to_string());
    }
    if !user_ids.is_empty() {
        wheres.push("(tu.user_id = ANY($2))".to_string());
    }
    let where_sql = if wheres.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wheres.join(" AND "))
    };
    let sql = format!(
        "UPDATE topic_users SET notification_level = {TRACKING}, notifications_reason_id = {AUTO_TRACK_TAG} \
           FROM ( \
             SELECT DISTINCT tu.topic_id, tu.user_id FROM topic_users tu \
               JOIN topic_tags ON tu.topic_id = topic_tags.topic_id \
               JOIN tag_users ON tag_users.user_id = tu.user_id AND topic_tags.tag_id = tag_users.tag_id \
                    AND tag_users.notification_level = {TRACKING} \
             {where_sql} \
           ) AS X \
          WHERE topic_users.notification_level = {REGULAR} \
            AND COALESCE(topic_users.notifications_reason_id, 0) != {USER_CHANGED} \
            AND topic_users.topic_id = X.topic_id AND topic_users.user_id = X.user_id"
    );
    sqlx::query(&sql)
        .bind(topic_id)
        .bind(user_ids)
        .execute(conn)
        .await?;
    Ok(())
}

/// `notification_level_visible([level])` over the topic's tag users: the
/// users at that level for one of its tags whose tag group lets them see
/// it.
pub async fn tag_watchers(
    conn: &mut PgConnection,
    topic_id: i32,
    level: i32,
) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT tag_users.user_id FROM tag_users \
         JOIN topic_tags ON topic_tags.tag_id = tag_users.tag_id AND topic_tags.topic_id = $1 \
         LEFT OUTER JOIN tag_group_memberships ON tag_users.tag_id = tag_group_memberships.tag_id \
         LEFT OUTER JOIN tag_group_permissions ON tag_group_memberships.tag_group_id = tag_group_permissions.tag_group_id \
         LEFT OUTER JOIN group_users ON group_users.user_id = tag_users.user_id \
         WHERE (tag_group_permissions.group_id IS NULL \
                OR tag_group_permissions.group_id IN (0, group_users.group_id) \
                OR group_users.group_id = 3) \
           AND tag_users.notification_level = $2 \
         ORDER BY tag_users.user_id",
    )
    .bind(topic_id)
    .bind(level)
    .fetch_all(conn)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_objects() {
        assert_eq!(parse_param(None).unwrap(), None);
        assert_eq!(parse_param(Some(&json!([]))).unwrap(), None);
        assert_eq!(
            parse_param(Some(&json!(["a", "b"]))).unwrap(),
            Some(TagsParam::Names(vec!["a".into(), "b".into()]))
        );
        assert_eq!(
            parse_param(Some(
                &json!([{"id": 2, "name": "guide"}, {"name": "new"}, {"id": "", "name": "x"}])
            ))
            .unwrap(),
            Some(TagsParam::Objects(vec![
                TagObject {
                    id: Some(2),
                    id_blank: false,
                    name: Some("guide".into())
                },
                TagObject {
                    id: None,
                    id_blank: true,
                    name: Some("new".into())
                },
                TagObject {
                    id: None,
                    id_blank: true,
                    name: Some("x".into())
                },
            ]))
        );
        assert!(parse_param(Some(&json!({"0": {"name": "a"}}))).is_err());
        assert!(parse_param(Some(&json!(["a", {"name": "b"}]))).is_err());
        assert_eq!(param_count(Some(&json!({"0": {}, "1": {}}))), 2);
        assert_eq!(ruby_to_i("12abc"), 12);
    }
}
