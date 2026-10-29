//! `CategorySerializer` (app/serializers/category_serializer.rb) over
//! SiteCategorySerializer and BasicCategorySerializer, with what the
//! bundled plugins add to it: topic voting's `can_vote` and chat's
//! `has_chat_channels`. Used by categories#show, find_by_slug, create and
//! update, the group permissions page, and `Category#publish_category`
//! (no scope, so an anonymous guardian).
//!
//! Also here: `HasCustomFields#custom_fields` for categories with the
//! field types core and the plugins register, and the category guardian
//! methods the serializer and the controller ask.
//!
//! Not ported: content localization (a category's `locale` is emitted as
//! stored; translated names are the site serializer's), which the
//! serializer only reads when content_localization_enabled is on.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState, Unsupported};

/// `CategoryGroup.permission_types`
pub const FULL: i32 = 1;
pub const CREATE_POST: i32 = 2;
pub const READONLY: i32 = 3;

/// `Group::AUTO_GROUPS[:everyone]`
pub const EVERYONE: i32 = 0;

/// The category custom fields registered with a type (core's
/// skip_category_definition and nested_replies_conversion_completed,
/// then the plugins'), all boolean; anything else is a string.
const BOOLEAN_FIELDS: &[&str] = &[
    "skip_category_definition",
    "nested_replies_conversion_completed",
    "has_chat_enabled",
    "enable_topic_voting",
    "create_as_post_voting_default",
    "only_post_voting_in_this_category",
    "sort_topics_by_event_start_date",
    "disable_topic_resorting",
];

/// Whether a category custom field is registered as boolean.
pub fn is_boolean_field(name: &str) -> bool {
    BOOLEAN_FIELDS.contains(&name)
}

/// `HasCustomFields::Helpers::CUSTOM_FIELD_TRUE`
fn custom_field_true(v: &str) -> bool {
    matches!(v, "1" | "t" | "true" | "T" | "True" | "TRUE")
}

/// `Category#custom_fields`: every row by id, typed by its registered
/// type; a name with several rows becomes an array.
pub async fn custom_fields(
    conn: &mut PgConnection,
    category_id: i32,
) -> Result<Map<String, Value>, sqlx::Error> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, value FROM category_custom_fields WHERE category_id = $1 ORDER BY id",
    )
    .bind(category_id)
    .fetch_all(conn)
    .await?;
    let mut out = Map::new();
    for (name, value) in rows {
        let typed = if BOOLEAN_FIELDS.contains(&name.as_str()) {
            json!(value.as_deref().is_some_and(custom_field_true))
        } else {
            json!(value)
        };
        match out.get_mut(&name) {
            Some(Value::Array(items)) => items.push(typed),
            Some(existing) => {
                let first = existing.take();
                *existing = json!([first, typed]);
            }
            None => {
                out.insert(name, typed);
            }
        }
    }
    Ok(out)
}

/// The columns the guardian methods read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GuardedCategory {
    pub id: i32,
    pub read_restricted: bool,
    pub topic_count: i32,
    pub email_in: Option<String>,
    pub email_in_allow_strangers: bool,
}

impl GuardedCategory {
    pub async fn load(conn: &mut PgConnection, id: i32) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, read_restricted, topic_count, email_in, email_in_allow_strangers \
             FROM categories WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(conn)
        .await
    }
}

/// `Guardian#can_see_category?`
pub async fn can_see(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    c: &GuardedCategory,
) -> Result<bool, AppError> {
    if guardian.is_admin() && !s.get("suppress_secured_categories_from_admin")?.truthy() {
        return Ok(true);
    }
    if !c.read_restricted {
        return Ok(true);
    }
    if guardian.user().is_some_and(|u| u.staged)
        && c.email_in.as_deref().is_some_and(|e| !e.is_empty())
        && c.email_in_allow_strangers
    {
        return Ok(true);
    }
    Ok(guardian.secure_category_ids(conn, s).await?.contains(&c.id))
}

/// `Guardian#can_create_category?`
pub fn can_create(s: &SiteSettings, guardian: &Guardian) -> Result<bool, AppError> {
    Ok(guardian.is_admin()
        || (s.get("moderators_manage_categories")?.truthy() && guardian.is_moderator()))
}

/// `Guardian#can_edit_category?`
pub async fn can_edit(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    c: &GuardedCategory,
) -> Result<bool, AppError> {
    if guardian.is_admin() {
        return Ok(true);
    }
    Ok(s.get("moderators_manage_categories")?.truthy()
        && guardian.is_moderator()
        && can_see(conn, s, guardian, c).await?)
}

/// `Category#has_children?`
pub async fn has_children(conn: &mut PgConnection, id: i32) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM categories WHERE parent_category_id = $1)")
        .bind(id)
        .fetch_one(conn)
        .await
}

/// Chat's `category_channel`: the category's live channel row
/// (Trashable's default scope).
async fn category_channel(conn: &mut PgConnection, id: i32) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id::bigint FROM chat_channels WHERE chatable_type = 'Category' AND chatable_id = $1 \
         AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(id)
    .fetch_optional(conn)
    .await
}

/// `Guardian#can_delete_category?` with chat's `deletable_for_chat?`.
pub async fn can_delete(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    c: &GuardedCategory,
) -> Result<bool, AppError> {
    if !can_edit(conn, s, guardian, c).await?
        || c.topic_count > 0
        || i64::from(c.id) == s.get("uncategorized_category_id")?.to_i()
        || has_children(conn, c.id).await?
    {
        return Ok(false);
    }
    // deletable_for_chat?: no channel, or one without messages.
    let Some(channel) = category_channel(conn, c.id).await? else {
        return Ok(true);
    };
    let has_messages: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM chat_messages WHERE chat_channel_id = $1 AND deleted_at IS NULL)",
    )
    .bind(channel)
    .fetch_one(&mut *conn)
    .await?;
    Ok(!has_messages)
}

/// `Category#cannot_delete_reason`, chat's check first.
pub async fn cannot_delete_reason(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    base_url: &str,
    c: &GuardedCategory,
) -> Result<Option<String>, AppError> {
    let t = |key: &str| -> Result<String, AppError> {
        Ok(i18n
            .t(key)
            .ok_or(Unsupported("a cannot_delete reason without a translation"))?
            .to_string())
    };
    if category_channel(conn, c.id).await?.is_some() {
        return Ok(Some(t("category.cannot_delete.has_chat_channels")?));
    }
    if i64::from(c.id) == s.get("uncategorized_category_id")?.to_i() {
        return Ok(Some(t("category.cannot_delete.uncategorized")?));
    }
    if has_children(conn, c.id).await? {
        return Ok(Some(t("category.cannot_delete.has_subcategories")?));
    }
    if c.topic_count == 0 {
        return Ok(None);
    }
    // The oldest topic other than the definition (Topic's default scope:
    // not deleted).
    let oldest: Option<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.title, t.slug FROM topics t \
         WHERE t.category_id = $1 AND t.deleted_at IS NULL \
           AND t.id IS DISTINCT FROM (SELECT topic_id FROM categories WHERE id = $1) \
         ORDER BY t.created_at ASC LIMIT 1",
    )
    .bind(c.id)
    .fetch_optional(&mut *conn)
    .await?;
    let count = i64::from(c.topic_count);
    let missing = || Unsupported("a cannot_delete reason without a translation");
    Ok(Some(match oldest {
        Some((id, title, slug)) => {
            // Topic#url: absolute, the slug segment only when present.
            let mut url = format!("{base_url}/t/");
            if let Some(slug) = slug.filter(|s| !s.is_empty()) {
                url.push_str(&slug);
                url.push('/');
            }
            url.push_str(&id.to_string());
            let link = format!(
                "<a href=\"{url}\">{}</a>",
                crate::category_badge::html_escape(&title)
            );
            i18n.t_count(
                "category.cannot_delete.topic_exists",
                count,
                &[("topic_link", &link)],
            )
            .ok_or_else(missing)?
        }
        None => i18n
            .t_count("category.cannot_delete.topic_exists_no_oldest", count, &[])
            .ok_or_else(missing)?,
    }))
}

/// What a caller set on the category before serializing it (its
/// attr_accessors): `Category.preload_user_fields!`, or the permission
/// categories#show sets.
#[derive(Debug, Default, Clone)]
pub struct Preloaded {
    pub notification_level: Option<i32>,
    pub permission: Option<i32>,
    pub has_children: Option<bool>,
    pub subcategory_count: Option<i64>,
    /// `includes(:category_setting)`: the association is serialized.
    pub category_setting: bool,
}

/// `Category.preload_user_fields!(guardian, [category])`
pub async fn preload_user_fields(
    conn: &mut PgConnection,
    s: &SiteSettings,
    i18n: &crate::i18n::I18n,
    guardian: &Guardian,
    category_id: i32,
) -> Result<Preloaded, AppError> {
    let base_path = "";
    let mut categories = crate::categories::Categories {
        conn: &mut *conn,
        settings: s,
        i18n,
        guardian,
        base_path,
        topic_url_via_slug: false,
    };
    let levels = categories.notification_levels().await.map_err(map_err)?;
    let level = levels
        .iter()
        .find(|(id, _)| *id == i64::from(category_id))
        .map(|(_, l)| *l as i32)
        .unwrap_or(default_notification_level(s)?);
    let permission = topic_create_allowed(&mut *conn, s, guardian, category_id)
        .await?
        .then_some(FULL);
    // Category.secured(guardian) children, grouped by parent.
    let secure = guardian.secure_category_ids(&mut *conn, s).await?;
    let children: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM categories WHERE parent_category_id = $1 \
           AND (NOT read_restricted OR id = ANY($2))",
    )
    .bind(category_id)
    .bind(&secure)
    .fetch_one(&mut *conn)
    .await?;
    Ok(Preloaded {
        notification_level: Some(level),
        permission,
        has_children: Some(children > 0),
        subcategory_count: (children > 0).then_some(children),
        category_setting: false,
    })
}

fn map_err(e: crate::categories::CategoriesError) -> AppError {
    match e {
        crate::categories::CategoriesError::Db(e) => e.into(),
        crate::categories::CategoriesError::Setting(e) => e.into(),
        crate::categories::CategoriesError::Url(e) => e.into(),
        crate::categories::CategoriesError::Unsupported(e) => e.into(),
    }
}

/// `CategoryUser.default_notification_level`
pub fn default_notification_level(s: &SiteSettings) -> Result<i32, AppError> {
    Ok(if s.get("mute_all_categories_by_default")?.truthy() {
        0
    } else {
        1
    })
}

/// `Category.topic_create_allowed(guardian).where(id:).exists?`
pub async fn topic_create_allowed(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    category_id: i32,
) -> Result<bool, AppError> {
    if guardian.is_admin() {
        return Ok(true);
    }
    if guardian.is_anonymous() {
        return Ok(false);
    }
    Ok(guardian
        .topic_create_allowed_category_ids(conn, s)
        .await?
        .contains(&category_id))
}

/// `Category.set_permission!(guardian, category)`: full where the viewer
/// may create topics, create_post where they may reply, readonly where
/// they may only see it, none otherwise.
pub async fn set_permission(
    conn: &mut PgConnection,
    s: &SiteSettings,
    guardian: &Guardian,
    c: &GuardedCategory,
) -> Result<Option<i32>, AppError> {
    if topic_create_allowed(conn, s, guardian, c.id).await? {
        return Ok(Some(FULL));
    }
    if guardian.is_authenticated() {
        let sql = format!(
            "SELECT EXISTS (SELECT 1 FROM categories WHERE categories.id = $1 AND {})",
            crate::plugins::chat::categories_scoped_to(guardian, "1, 2")
        );
        let may_post: bool = sqlx::query_scalar(&sql)
            .bind(c.id)
            .fetch_one(&mut *conn)
            .await?;
        if may_post {
            return Ok(Some(CREATE_POST));
        }
    }
    if can_see(conn, s, guardian, c).await? {
        return Ok(Some(READONLY));
    }
    Ok(None)
}

/// The columns CategorySerializer reads beyond the site serializer's.
#[derive(sqlx::FromRow)]
struct Extra {
    locale: Option<String>,
    auto_close_hours: Option<f64>,
    auto_close_based_on_last_post: Option<bool>,
    email_in: Option<String>,
    email_in_allow_strangers: Option<bool>,
    mailinglist_mirror: bool,
    all_topics_wiki: bool,
    allow_unlimited_owner_edits_on_first_post: bool,
    allow_badges: bool,
    topic_featured_link_allowed: Option<bool>,
    search_priority: Option<i32>,
    default_slow_mode_seconds: Option<i32>,
    topic_count: i32,
}

/// category_settings: cooldown days, daily bumps, nested replies, the
/// topic and reply review modes.
type SettingRow = (Option<i32>, Option<i32>, bool, i32, i32);

/// `CategorySetting.topic_posting_review_mode`'s enum.
fn review_mode(v: i32) -> &'static str {
    match v {
        1 => "everyone",
        2 => "everyone_except",
        3 => "no_one_except",
        _ => "no_one",
    }
}

/// CategorySerializer for category `id` as `guardian` sees it (an
/// anonymous guardian for no scope).
pub async fn serialize(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    s: &SiteSettings,
    guardian: &Guardian,
    id: i32,
    pre: &Preloaded,
) -> Result<Value, AppError> {
    let base_path = cx.config.globals.relative_url_root().to_string();
    let base_url = crate::url::Urls {
        config: cx.config,
        settings: s,
    }
    .base_url()?;
    let mut row: crate::categories::CategoryRow = sqlx::query_as(&format!(
        "SELECT * FROM ({}) c WHERE c.id = $1",
        crate::categories::CATEGORY_SQL
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(Unsupported("serializing a category that vanished"))?;
    // topic_only_relative_url: the definition through Topic's default
    // scope, so none once it is deleted.
    if let Some(topic_id) = row.topic_id {
        let live: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM topics WHERE id = $1 AND deleted_at IS NULL)",
        )
        .bind(topic_id)
        .fetch_one(&mut *conn)
        .await?;
        if !live {
            row.topic_id = None;
            row.topic_slug = None;
        }
    }
    let extra: Extra = sqlx::query_as(
        "SELECT locale, auto_close_hours, auto_close_based_on_last_post, email_in, \
                email_in_allow_strangers, mailinglist_mirror, all_topics_wiki, \
                allow_unlimited_owner_edits_on_first_post, allow_badges, \
                topic_featured_link_allowed, search_priority, default_slow_mode_seconds, topic_count \
         FROM categories WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await?;
    let guarded = GuardedCategory {
        id,
        read_restricted: row.read_restricted,
        topic_count: extra.topic_count,
        email_in: extra.email_in.clone(),
        email_in_allow_strangers: extra.email_in_allow_strangers.unwrap_or(false),
    };
    let can_edit_category =
        guardian.is_authenticated() && can_edit(conn, s, guardian, &guarded).await?;
    let tagging = s.get("tagging_enabled")?.truthy();
    let can_edit_tags = tagging && can_edit_category;

    let mut basic = {
        let mut categories = crate::categories::Categories {
            conn: &mut *conn,
            settings: s,
            i18n: cx.i18n,
            guardian,
            base_path: &base_path,
            topic_url_via_slug: false,
        };
        let mut out = categories.basic_fields(&row).await.map_err(map_err)?;
        let mut uploads = Map::new();
        categories
            .uploads(&mut uploads, &row)
            .await
            .map_err(map_err)?;
        out.insert("__uploads".into(), Value::Object(uploads));
        out
    };
    let uploads = match basic.remove("__uploads") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    // basic_fields put the plugins' keys (post voting's, then topic
    // voting's can_vote) last; they go after custom_fields here.
    let plugin_keys: Vec<(String, Value)> = [
        crate::plugins::post_voting::CREATE_AS_POST_VOTING_DEFAULT,
        crate::plugins::post_voting::ONLY_POST_VOTING_IN_THIS_CATEGORY,
    ]
    .into_iter()
    .filter_map(|k| basic.shift_remove(k).map(|v| (k.to_string(), v)))
    .collect();
    let can_vote = basic.remove("can_vote");

    let mut out = Map::new();
    for (k, v) in basic {
        match k.as_str() {
            "permission" => {
                out.insert(k, json!(pre.permission));
            }
            "notification_level" => {
                // include_notification_level?: scope.user
                if let Some(user_id) = guardian.user_id() {
                    let level = match pre.notification_level {
                        Some(l) => l,
                        None => {
                            let stored: Option<i32> = sqlx::query_scalar(
                                "SELECT notification_level FROM category_users \
                                 WHERE user_id = $1 AND category_id = $2 ORDER BY id LIMIT 1",
                            )
                            .bind(user_id)
                            .bind(id)
                            .fetch_optional(&mut *conn)
                            .await?;
                            match stored {
                                Some(l) => l,
                                None => default_notification_level(s)?,
                            }
                        }
                    };
                    out.insert(k, json!(level));
                }
                if can_edit_category {
                    out.insert("can_edit".into(), json!(true));
                }
            }
            "has_children" => {
                out.insert(k, json!(pre.has_children));
            }
            "subcategory_count" => {
                out.insert(k, json!(pre.subcategory_count));
            }
            _ => {
                out.insert(k, v);
            }
        }
    }
    out.insert(
        "custom_fields".into(),
        Value::Object(custom_fields(conn, id).await?),
    );
    out.extend(plugin_keys);
    if let Some(v) = can_vote {
        out.insert("can_vote".into(), v);
    }

    // SiteCategorySerializer
    if tagging {
        out.insert("allow_global_tags".into(), json!(row.allow_global_tags));
    }
    out.insert("read_only_banner".into(), json!(row.read_only_banner));
    let form_templates: Vec<i32> = sqlx::query_scalar(
        "SELECT form_template_id FROM category_form_templates WHERE category_id = $1 \
         ORDER BY form_template_id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    out.insert("form_template_ids".into(), json!(form_templates));
    if tagging {
        let required: Vec<(i32, Option<String>)> = sqlx::query_as(
            "SELECT r.min_count, g.name FROM category_required_tag_groups r \
             LEFT JOIN tag_groups g ON g.id = r.tag_group_id \
             WHERE r.category_id = $1 ORDER BY r.\"order\" ASC, r.id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        let required: Vec<Value> = required
            .into_iter()
            .map(|(min_count, name)| {
                let mut e = Map::new();
                e.insert("min_count".into(), json!(min_count));
                if can_edit_tags {
                    e.insert("name".into(), json!(name));
                }
                Value::Object(e)
            })
            .collect();
        out.insert("required_tag_groups".into(), json!(required));
    }
    let category_types = if s.get("enable_simplified_category_creation")?.truthy() {
        with_registry(cx, s, async |r| r.for_category(conn, id).await).await?
    } else {
        json!({})
    };
    out.insert("category_types".into(), category_types);

    // CategorySerializer
    out.insert("locale".into(), json!(extra.locale));
    let group_permissions = if can_edit_category {
        Some(group_permissions(conn, guardian, id, row.read_restricted).await?)
    } else {
        None
    };
    if let Some(perms) = &group_permissions {
        let taken: Vec<&str> = perms
            .iter()
            .filter_map(|p| p["group_name"].as_str())
            .collect();
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM groups ORDER BY name")
            .fetch_all(&mut *conn)
            .await?;
        let available: Vec<String> = names
            .into_iter()
            .filter(|n| !taken.contains(&n.as_str()))
            .collect();
        out.insert("available_groups".into(), json!(available));
    }
    if can_edit_tags {
        let tags: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT t.id, t.name, t.slug FROM category_tags ct JOIN tags t ON t.id = ct.tag_id \
             WHERE ct.category_id = $1 ORDER BY ct.id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        out.insert(
            "allowed_tags".into(),
            json!(
                tags.into_iter()
                    .map(|(id, name, slug)| json!({"id": id, "name": name, "slug": slug}))
                    .collect::<Vec<_>>()
            ),
        );
    }
    if tagging {
        let names: Vec<String> = if can_edit_tags || guardian.is_admin() {
            sqlx::query_scalar(
                "SELECT g.name FROM category_tag_groups ctg JOIN tag_groups g ON g.id = ctg.tag_group_id \
                 WHERE ctg.category_id = $1 ORDER BY ctg.id",
            )
            .bind(id)
            .fetch_all(&mut *conn)
            .await?
        } else {
            // TagGroup.visible(scope || Guardian.new)
            let allowed = guardian.allowed_category_ids_sql(s)?;
            let groups: Vec<i64> = crate::tagging::permitted_group_ids(guardian);
            sqlx::query_scalar(&format!(
                "SELECT name FROM tag_groups WHERE id IN \
                   (SELECT tag_group_id FROM category_tag_groups WHERE category_id = $1) \
                 AND (id IN (SELECT tag_group_id FROM category_tag_groups WHERE category_id IN ({allowed})) \
                      OR id NOT IN (SELECT tag_group_id FROM category_tag_groups)) \
                 AND id IN (SELECT tag_group_id FROM tag_group_permissions WHERE group_id = ANY($2)) \
                 ORDER BY id"
            ))
            .bind(id)
            .bind(&groups)
            .fetch_all(&mut *conn)
            .await?
        };
        out.insert("allowed_tag_groups".into(), json!(names));
    }
    out.insert("auto_close_hours".into(), json!(extra.auto_close_hours));
    out.insert(
        "auto_close_based_on_last_post".into(),
        json!(extra.auto_close_based_on_last_post),
    );
    if let Some(perms) = group_permissions {
        out.insert("group_permissions".into(), Value::Array(perms));
    }
    if can_edit_category {
        out.insert("email_in".into(), json!(extra.email_in));
        out.insert(
            "email_in_allow_strangers".into(),
            json!(extra.email_in_allow_strangers),
        );
    }
    out.insert("mailinglist_mirror".into(), json!(extra.mailinglist_mirror));
    out.insert("all_topics_wiki".into(), json!(extra.all_topics_wiki));
    out.insert(
        "allow_unlimited_owner_edits_on_first_post".into(),
        json!(extra.allow_unlimited_owner_edits_on_first_post),
    );
    let deletable = guardian.is_authenticated() && can_delete(conn, s, guardian, &guarded).await?;
    if deletable {
        out.insert("can_delete".into(), json!(true));
    } else if can_edit_category {
        out.insert(
            "cannot_delete_reason".into(),
            json!(cannot_delete_reason(conn, s, cx.i18n, &base_url, &guarded).await?),
        );
    }
    let special = [
        "meta_category_id",
        "staff_category_id",
        "uncategorized_category_id",
    ];
    let mut is_special = false;
    for name in special {
        if s.get(name)?.to_i() == i64::from(id) {
            is_special = true;
        }
    }
    if is_special {
        out.insert("is_special".into(), json!(true));
    }
    out.insert("allow_badges".into(), json!(extra.allow_badges));
    out.insert(
        "topic_featured_link_allowed".into(),
        json!(extra.topic_featured_link_allowed),
    );
    out.insert("search_priority".into(), json!(extra.search_priority));
    if s.get("enable_category_group_moderation")?.truthy() {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT group_id FROM category_moderation_groups WHERE category_id = $1 ORDER BY id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        out.insert("moderating_group_ids".into(), json!(ids));
    }
    for (key, post_type) in [
        ("topic_posting_review_group_ids", 0),
        ("reply_posting_review_group_ids", 1),
    ] {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT group_id FROM category_posting_review_groups \
             WHERE category_id = $1 AND post_type = $2 ORDER BY id",
        )
        .bind(id)
        .bind(post_type)
        .fetch_all(&mut *conn)
        .await?;
        out.insert(key.into(), json!(ids));
    }
    out.insert(
        "default_slow_mode_seconds".into(),
        json!(extra.default_slow_mode_seconds),
    );
    if can_edit_category {
        let settings = category_type_settings(s, id)?;
        out.insert("category_type_settings".into(), settings);
        out.insert(
            "available_category_types".into(),
            with_registry(cx, s, async |r| r.list(conn, guardian).await).await?,
        );
    }
    // chat: `object.category_channels.exists?`, a has_many on a composite
    // foreign key the category's single key never matches.
    if s.get("chat_enabled")?.truthy() {
        out.insert("has_chat_channels".into(), json!(false));
    }
    for (k, v) in uploads {
        out.insert(k, v);
    }
    if pre.category_setting {
        let setting: Option<SettingRow> = sqlx::query_as(
            "SELECT auto_bump_cooldown_days, num_auto_bump_daily, nested_replies_default, \
                    topic_posting_review_mode, reply_posting_review_mode \
             FROM category_settings WHERE category_id = $1",
        )
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
        out.insert(
            "category_setting".into(),
            match setting {
                None => Value::Null,
                Some((cooldown, daily, nested, topic_mode, reply_mode)) => json!({
                    "auto_bump_cooldown_days": cooldown,
                    "num_auto_bump_daily": daily,
                    "require_reply_approval": reply_mode == 1,
                    "require_topic_approval": topic_mode == 1,
                    "nested_replies_default": nested,
                    "topic_posting_review_mode": review_mode(topic_mode),
                    "reply_posting_review_mode": review_mode(reply_mode),
                }),
            },
        );
    }
    let localizations: Vec<(i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id::bigint, locale, name, description FROM category_localizations \
         WHERE category_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    out.insert(
        "category_localizations".into(),
        json!(
            localizations
                .into_iter()
                .map(|(id, locale, name, description)| json!({
                    "id": id, "locale": locale, "name": name, "description": description,
                }))
                .collect::<Vec<_>>()
        ),
    );
    Ok(Value::Object(out))
}

/// `Category#publish_category`: the category through CategorySerializer
/// (no scope) on /categories, to its groups when read restricted (to
/// nobody without any), to everyone otherwise.
pub async fn publish_category(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    bus: &pg_bus::Bus,
    s: &SiteSettings,
    id: i32,
    pre: &Preloaded,
) -> Result<(), AppError> {
    let read_restricted: bool =
        sqlx::query_scalar("SELECT read_restricted FROM categories WHERE id = $1")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
    let audience = if read_restricted {
        let group_ids: Vec<i32> = sqlx::query_scalar(
            "SELECT g.id FROM category_groups cg JOIN groups g ON g.id = cg.group_id \
             WHERE cg.category_id = $1 ORDER BY cg.id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        if group_ids.is_empty() {
            return Ok(());
        }
        Some(
            group_ids
                .into_iter()
                .map(crate::bus::group_tag)
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let json = serialize(conn, cx, s, &Guardian::anonymous(), id, pre).await?;
    bus.publish(
        &mut *conn,
        "/categories",
        &json!({ "categories": [json] }),
        audience.as_deref(),
    )
    .await?;
    Ok(())
}

/// What serializing a category reads besides the database and settings.
pub struct Cx<'a> {
    pub config: &'a crate::config::Config,
    pub i18n: &'a crate::i18n::I18n,
    pub defs: &'a crate::site_settings::Definitions,
}

impl<'a> Cx<'a> {
    pub fn of(state: &'a AppState) -> Self {
        Cx {
            config: &state.config,
            i18n: &state.i18n,
            defs: &state.site_setting_defs,
        }
    }
}

/// The category type registry over this request's settings.
pub async fn with_registry<T>(
    cx: &Cx<'_>,
    s: &SiteSettings,
    f: impl AsyncFnOnce(&crate::category_types::Registry<'_>) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let urls = crate::url::Urls {
        config: cx.config,
        settings: s,
    };
    let acx = crate::admin_site_settings::Context {
        defs: cx.defs,
        settings: s,
        i18n: cx.i18n,
        globals: &cx.config.globals,
        base_path: cx.config.globals.relative_url_root(),
        urls: &urls,
    };
    let registry = crate::category_types::Registry { cx: &acx };
    f(&registry).await
}

/// `CategorySerializer#group_permissions`: the category's groups the
/// viewer may see, by name, or everyone with full access for a public
/// category without any.
async fn group_permissions(
    conn: &mut PgConnection,
    guardian: &Guardian,
    id: i32,
    read_restricted: bool,
) -> Result<Vec<Value>, AppError> {
    let visible = crate::groups::visible_groups_scope(guardian, "g", true, false);
    let rows: Vec<(i32, String, i32)> = sqlx::query_as(&format!(
        "SELECT cg.permission_type, g.name, cg.group_id FROM category_groups cg \
         JOIN groups g ON g.id = cg.group_id \
         WHERE cg.category_id = $1 AND {visible} ORDER BY g.name ASC"
    ))
    .bind(id)
    .fetch_all(&mut *conn)
    .await?;
    let mut perms: Vec<Value> = rows
        .into_iter()
        .map(|(permission_type, group_name, group_id)| {
            json!({
                "permission_type": permission_type,
                "group_name": group_name,
                "group_id": group_id,
            })
        })
        .collect();
    if perms.is_empty() && !read_restricted {
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM groups WHERE id = 0")
            .fetch_optional(&mut *conn)
            .await?;
        perms.push(json!({
            "permission_type": FULL,
            "group_name": name.filter(|n| !n.is_empty()).unwrap_or_else(|| "everyone".into()),
            "group_id": EVERYONE,
        }));
    }
    Ok(perms)
}

/// `CategorySerializer#category_type_settings`: each matching type's
/// stored category settings; only Events has any
/// (`read_category_settings`, from `calendar_categories`).
fn category_type_settings(s: &SiteSettings, id: i32) -> Result<Value, AppError> {
    let mut out = Map::new();
    let events = s
        .get("events_calendar_categories")?
        .to_s()
        .split('|')
        .any(|v| v.trim().parse::<i32>() == Ok(id));
    if events {
        let calendar = s.get("calendar_categories")?.to_s();
        let marker = format!("categoryId={id}");
        if let Some(entry) = calendar
            .split('|')
            .find(|e| e.split(';').any(|p| p == marker))
        {
            let mut view = None;
            let mut weekends = None;
            for part in entry.split(';') {
                if let Some((k, v)) = part.split_once('=') {
                    match k {
                        "defaultView" => view = Some(v.to_string()),
                        "weekends" => weekends = Some(v.to_string()),
                        _ => {}
                    }
                }
            }
            out.insert(
                "events_calendar_default_view".into(),
                json!(view.unwrap_or_else(|| "month".into())),
            );
            out.insert(
                "events_calendar_display_weekends".into(),
                json!(weekends.as_deref() != Some("false")),
            );
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_field_truth_matches_rails() {
        for v in ["1", "t", "true", "T", "True", "TRUE"] {
            assert!(custom_field_true(v));
        }
        for v in ["0", "f", "false", "yes", ""] {
            assert!(!custom_field_true(v));
        }
    }

    #[test]
    fn review_modes_follow_the_enum() {
        assert_eq!(review_mode(0), "no_one");
        assert_eq!(review_mode(1), "everyone");
        assert_eq!(review_mode(2), "everyone_except");
        assert_eq!(review_mode(3), "no_one_except");
    }
}
