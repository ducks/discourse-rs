//! Port of Site#categories / Site.all_categories_cache (app/models/site.rb)
//! with SiteCategorySerializer and BasicCategorySerializer, for anonymous
//! users.
//!
//! Not ported: plugin custom fields (`custom_fields` is only emitted when
//! plugins register some, so core emits nothing), plugin category types,
//! content localization, and category descriptions with markup beyond
//! plain paragraphs (ExcerptParser), which are refused.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};

/// `CategoryUser.notification_levels`
const MUTED: i64 = 0;
const REGULAR: i64 = 1;

/// `Category.style_types`
pub(crate) fn style_type(id: i32) -> &'static str {
    match id {
        1 => "icon",
        2 => "emoji",
        _ => "square",
    }
}

#[derive(Debug)]
pub enum CategoriesError {
    Db(sqlx::Error),
    Setting(SettingError),
    Url(crate::url::UrlError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for CategoriesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CategoriesError::Db(e) => write!(f, "loading categories: {e}"),
            CategoriesError::Setting(e) => e.fmt(f),
            CategoriesError::Url(e) => e.fmt(f),
            CategoriesError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for CategoriesError {}

impl From<sqlx::Error> for CategoriesError {
    fn from(e: sqlx::Error) -> Self {
        CategoriesError::Db(e)
    }
}

impl From<SettingError> for CategoriesError {
    fn from(e: SettingError) -> Self {
        CategoriesError::Setting(e)
    }
}

impl From<Unsupported> for CategoriesError {
    fn from(e: Unsupported) -> Self {
        CategoriesError::Unsupported(e)
    }
}

/// `categories.*, t.slug topic_slug`, the columns the serializer reads.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CategoryRow {
    pub(crate) id: i32,
    pub(crate) name: String,
    pub(crate) color: String,
    pub(crate) text_color: String,
    pub(crate) style_type: i32,
    pub(crate) icon: Option<String>,
    pub(crate) emoji: Option<String>,
    pub(crate) slug: String,
    pub(crate) topic_count: i32,
    pub(crate) post_count: i32,
    pub(crate) position: Option<i32>,
    pub(crate) description: Option<String>,
    pub(crate) topic_id: Option<i32>,
    pub(crate) topic_slug: Option<String>,
    pub(crate) read_restricted: bool,
    pub(crate) parent_category_id: Option<i32>,
    pub(crate) topic_template: Option<String>,
    pub(crate) topic_title_placeholder: Option<String>,
    pub(crate) sort_order: Option<String>,
    pub(crate) sort_ascending: Option<bool>,
    pub(crate) show_subcategory_list: bool,
    pub(crate) num_featured_topics: i32,
    pub(crate) default_view: Option<String>,
    pub(crate) subcategory_list_style: String,
    pub(crate) default_top_period: String,
    pub(crate) default_list_filter: String,
    pub(crate) minimum_required_tags: i32,
    pub(crate) navigate_to_first_post_after_read: bool,
    pub(crate) allow_global_tags: bool,
    pub(crate) read_only_banner: Option<String>,
    pub(crate) uploaded_logo_id: Option<i32>,
    pub(crate) uploaded_logo_dark_id: Option<i32>,
    pub(crate) uploaded_background_id: Option<i32>,
    pub(crate) uploaded_background_dark_id: Option<i32>,
    pub(crate) topics_day: i32,
    pub(crate) topics_week: i32,
    pub(crate) topics_month: i32,
    pub(crate) topics_year: i32,
}

pub struct Categories<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    /// `Discourse.base_path`
    pub base_path: &'a str,
    /// Whether topic_url is built from the selected topic_slug (Site cache)
    /// rather than the association (CategoryList).
    pub topic_url_via_slug: bool,
}

impl Categories<'_> {
    /// `Site#categories`: every category the guardian can see, serialized,
    /// with the per-user fields filled in. Ordered by position.
    /// `categories.*, t.slug topic_slug`: every category by position.
    pub(crate) async fn load_all(conn: &mut PgConnection) -> Result<Vec<CategoryRow>, sqlx::Error> {
        sqlx::query_as(CATEGORY_SQL).fetch_all(conn).await
    }

    pub async fn for_site(&mut self) -> Result<Vec<Value>, CategoriesError> {
        let rows = Self::load_all(&mut *self.conn).await?;

        // can_see_serialized_category?: public, or in the guardian's secure ids.
        let secure = self.guardian.secure_category_ids();
        let visible: Vec<&CategoryRow> = rows
            .iter()
            .filter(|c| !c.read_restricted || secure.contains(&c.id))
            .collect();
        let visible_ids: Vec<i32> = visible.iter().map(|c| c.id).collect();
        let with_children: Vec<i32> = visible
            .iter()
            .filter_map(|c| c.parent_category_id)
            .collect();

        let notification_levels = self.notification_levels()?;
        let default_level = if self
            .settings
            .get("mute_all_categories_by_default")?
            .truthy()
        {
            MUTED
        } else {
            REGULAR
        };

        let mut out = Vec::with_capacity(visible.len());
        for category in visible {
            // Children of a hidden parent are dropped.
            if let Some(parent) = category.parent_category_id {
                if !visible_ids.contains(&parent) {
                    continue;
                }
            }
            let mut json = self.serialize(category).await?;
            let level = notification_levels
                .iter()
                .find(|(id, _)| *id == i64::from(category.id))
                .map(|(_, l)| *l)
                .unwrap_or(default_level);
            json.insert("notification_level".into(), json!(level));
            // permission stays null: anonymous users can't create topics.
            json.insert(
                "has_children".into(),
                json!(with_children.contains(&category.id)),
            );
            // can_edit_serialized_category? is false for anonymous users.
            json.insert("can_edit".into(), json!(false));
            out.push(Value::Object(json));
        }
        Ok(out)
    }

    /// `CategoryUser.notification_levels_for(nil)`: default categories from
    /// settings are regular, default muted ones muted.
    pub(crate) fn notification_levels(&self) -> Result<Vec<(i64, i64)>, CategoriesError> {
        let mut levels = Vec::new();
        for name in [
            "default_categories_watching",
            "default_categories_tracking",
            "default_categories_watching_first_post",
            "default_categories_normal",
        ] {
            levels.extend(
                self.settings
                    .group_ids(name)?
                    .into_iter()
                    .map(|id| (id, REGULAR)),
            );
        }
        levels.extend(
            self.settings
                .group_ids("default_categories_muted")?
                .into_iter()
                .map(|id| (id, MUTED)),
        );
        // Hash[*flatten]: a later entry for the same id wins.
        levels.reverse();
        levels.dedup_by_key(|(id, _)| *id);
        levels.reverse();
        Ok(levels)
    }

    /// SiteCategorySerializer as cached by Site.all_categories_cache (no
    /// scope), in attribute order. notification_level, has_children and
    /// can_edit are filled in by for_site.
    async fn serialize(&mut self, c: &CategoryRow) -> Result<Map<String, Value>, CategoriesError> {
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        let mut out = self.basic_fields(c).await?;
        // custom_fields: only present when plugins register category fields.
        if tagging {
            out.insert("allow_global_tags".into(), json!(c.allow_global_tags));
        }
        out.insert("read_only_banner".into(), json!(c.read_only_banner));
        out.insert(
            "form_template_ids".into(),
            self.form_template_ids(c.id).await?,
        );
        if tagging {
            out.insert(
                "required_tag_groups".into(),
                self.required_tag_groups(c.id).await?,
            );
        }
        out.insert("category_types".into(), self.category_types()?);
        self.uploads(&mut out, c).await?;
        Ok(out)
    }

    /// CategoryUploadSerializer for the four category images.
    pub(crate) async fn uploads(
        &mut self,
        out: &mut Map<String, Value>,
        c: &CategoryRow,
    ) -> Result<(), CategoriesError> {
        for (key, upload_id) in [
            ("uploaded_logo", c.uploaded_logo_id),
            ("uploaded_logo_dark", c.uploaded_logo_dark_id),
            ("uploaded_background", c.uploaded_background_id),
            ("uploaded_background_dark", c.uploaded_background_dark_id),
        ] {
            out.insert(key.into(), self.upload(upload_id).await?);
        }
        Ok(())
    }

    /// BasicCategorySerializer's attributes through
    /// navigate_to_first_post_after_read, with notification_level and
    /// has_children left null for the caller to fill.
    pub(crate) async fn basic_fields(
        &mut self,
        c: &CategoryRow,
    ) -> Result<Map<String, Value>, CategoriesError> {
        let uncategorized =
            i64::from(c.id) == self.settings.get("uncategorized_category_id")?.to_i();

        let (description, description_text, description_excerpt) = if uncategorized {
            let text = self
                .i18n
                .t("category.uncategorized_description")
                .map(str::to_string);
            (text.clone(), text.clone(), text)
        } else {
            let text = description_plain_text(c.description.as_deref())?;
            let excerpt = description_excerpt(c.description.as_deref())?;
            (c.description.clone(), text, excerpt)
        };
        let name = if uncategorized {
            self.i18n
                .t("uncategorized_category_name")
                .map(str::to_string)
                .unwrap_or_else(|| c.name.clone())
        } else {
            c.name.clone()
        };

        // Topic.relative_url(topic_id, topic_slug) when the row carries
        // `topic_slug` (the site cache query), even with no topic ("/t/");
        // the plain association path gives nil without a topic.
        let topic_url: Option<String> = if c.topic_id.is_none() && !self.topic_url_via_slug {
            None
        } else {
            let mut url = format!("{}/t/", self.base_path);
            if let Some(slug) = c.topic_slug.as_deref().filter(|s| !s.is_empty()) {
                url.push_str(slug);
                url.push('/');
            }
            if let Some(id) = c.topic_id {
                url.push_str(&id.to_string());
            }
            Some(url)
        };

        let mut out = Map::new();
        out.insert("id".into(), json!(c.id));
        out.insert("name".into(), json!(name));
        out.insert("color".into(), json!(c.color));
        out.insert("text_color".into(), json!(c.text_color));
        out.insert("style_type".into(), json!(style_type(c.style_type)));
        out.insert("icon".into(), json!(c.icon));
        out.insert("emoji".into(), json!(c.emoji));
        out.insert("slug".into(), json!(c.slug));
        out.insert("topic_count".into(), json!(c.topic_count));
        out.insert("post_count".into(), json!(c.post_count));
        out.insert("position".into(), json!(c.position));
        out.insert("description".into(), json!(description));
        out.insert("description_text".into(), json!(description_text));
        out.insert("description_excerpt".into(), json!(description_excerpt));
        out.insert("topic_url".into(), json!(topic_url));
        out.insert("read_restricted".into(), json!(c.read_restricted));
        out.insert("permission".into(), Value::Null);
        if let Some(parent) = c.parent_category_id {
            out.insert("parent_category_id".into(), json!(parent));
        }
        out.insert("notification_level".into(), Value::Null);
        out.insert("topic_template".into(), json!(c.topic_template));
        out.insert(
            "topic_title_placeholder".into(),
            json!(c.topic_title_placeholder),
        );
        out.insert("has_children".into(), Value::Null);
        // attr_accessor never set on cached categories
        out.insert("subcategory_count".into(), Value::Null);
        out.insert("sort_order".into(), json!(c.sort_order));
        out.insert("sort_ascending".into(), json!(c.sort_ascending));
        out.insert(
            "show_subcategory_list".into(),
            json!(c.show_subcategory_list),
        );
        out.insert("num_featured_topics".into(), json!(c.num_featured_topics));
        out.insert("default_view".into(), json!(c.default_view));
        out.insert(
            "subcategory_list_style".into(),
            json!(c.subcategory_list_style),
        );
        out.insert("default_top_period".into(), json!(c.default_top_period));
        out.insert("default_list_filter".into(), json!(c.default_list_filter));
        out.insert(
            "minimum_required_tags".into(),
            json!(c.minimum_required_tags),
        );
        out.insert(
            "navigate_to_first_post_after_read".into(),
            json!(c.navigate_to_first_post_after_read),
        );
        Ok(out)
    }

    /// `object.form_template_ids.sort`
    async fn form_template_ids(&mut self, category_id: i32) -> Result<Value, CategoriesError> {
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT form_template_id FROM category_form_templates WHERE category_id = $1 \
             ORDER BY form_template_id",
        )
        .bind(category_id)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(json!(ids))
    }

    /// `required_tag_groups` without `name`, which needs can_edit.
    async fn required_tag_groups(&mut self, category_id: i32) -> Result<Value, CategoriesError> {
        let counts: Vec<i32> = sqlx::query_scalar(
            "SELECT min_count FROM category_required_tag_groups WHERE category_id = $1 \
             ORDER BY \"order\" ASC",
        )
        .bind(category_id)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(json!(
            counts
                .into_iter()
                .map(|min_count| json!({"min_count": min_count}))
                .collect::<Vec<_>>()
        ))
    }

    /// `Category#category_types` with the core Discussion type, which
    /// matches every category. Plugin types aren't registered.
    fn category_types(&self) -> Result<Value, CategoriesError> {
        if !self
            .settings
            .get("enable_simplified_category_creation")?
            .truthy()
        {
            return Ok(json!({}));
        }
        let name = self
            .i18n
            .t("category_types.discussion.name")
            .unwrap_or("Discussion");
        Ok(json!({
            "discussion": {
                "id": "discussion",
                "name": name,
                "title": self.i18n.t("category_types.discussion.title").unwrap_or(name),
                "description": self.i18n.t("category_types.discussion.description").unwrap_or(""),
                "icon": "memo",
                "available": true,
                "visible": true,
                "configuration_schema": {},
            }
        }))
    }

    /// CategoryUploadSerializer, null when unset.
    async fn upload(&mut self, id: Option<i32>) -> Result<Value, CategoriesError> {
        let Some(id) = id else {
            return Ok(Value::Null);
        };
        let row: Option<(i32, String, Option<i32>, Option<i32>)> =
            sqlx::query_as("SELECT id, url, width, height FROM uploads WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *self.conn)
                .await?;
        Ok(match row {
            Some((id, url, width, height)) => {
                json!({"id": id, "url": url, "width": width, "height": height})
            }
            None => Value::Null,
        })
    }
}

/// `ERB::Util.html_escape`
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `Category#description_text`: escaped plain text, nil without a
/// description (or when it has no text).
pub fn description_plain_text(description: Option<&str>) -> Result<Option<String>, Unsupported> {
    let Some(html) = description else {
        return Ok(None);
    };
    let text = crate::excerpt::fragment_text(html);
    Ok(Some(html_escape(text.trim())))
}

/// `PrettyText.excerpt(description, 300)`
pub(crate) fn description_excerpt(
    description: Option<&str>,
) -> Result<Option<String>, Unsupported> {
    Ok(description
        .map(|html| crate::excerpt::excerpt(html, 300, &crate::excerpt::Options::default())))
}

pub(crate) const CATEGORY_SQL: &str = "SELECT categories.id, categories.name, categories.color, categories.text_color, \
    categories.style_type, categories.icon, categories.emoji, categories.slug, \
    categories.topic_count, categories.post_count, categories.position, \
    categories.description, categories.topic_id, t.slug AS topic_slug, \
    categories.read_restricted, categories.parent_category_id, \
    categories.topic_template, categories.topic_title_placeholder, \
    categories.sort_order, categories.sort_ascending, \
    categories.show_subcategory_list, categories.num_featured_topics, \
    categories.default_view, categories.subcategory_list_style, \
    categories.default_top_period, categories.default_list_filter, \
    categories.minimum_required_tags, categories.navigate_to_first_post_after_read, \
    categories.allow_global_tags, categories.read_only_banner, \
    categories.uploaded_logo_id, categories.uploaded_logo_dark_id, \
    categories.uploaded_background_id, categories.uploaded_background_dark_id, \
    categories.topics_day, categories.topics_week, categories.topics_month, categories.topics_year \
    FROM categories LEFT JOIN topics t ON t.id = categories.topic_id \
    ORDER BY categories.position";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paragraph_descriptions() {
        let html = Some("<p>Discussion about this site, &amp; how we can improve it.</p>");
        assert_eq!(
            description_plain_text(html).unwrap().as_deref(),
            Some("Discussion about this site, &amp; how we can improve it.")
        );
        assert_eq!(
            description_excerpt(html).unwrap().as_deref(),
            Some("Discussion about this site, &amp; how we can improve it.")
        );
        assert_eq!(description_plain_text(None).unwrap(), None);
        assert_eq!(description_excerpt(Some("")).unwrap().as_deref(), Some(""));
    }

    #[test]
    fn long_descriptions_get_an_ellipsis() {
        let long = format!("<p>{}</p>", "x".repeat(301));
        let excerpt = description_excerpt(Some(&long)).unwrap().unwrap();
        assert!(excerpt.ends_with("&hellip;"));
        assert_eq!(excerpt.chars().count(), 300 + "&hellip;".len());
    }

    #[test]
    fn markup_is_reduced_like_pretty_text() {
        assert_eq!(
            description_plain_text(Some("<p>Hi <b>there</b> &amp; you</p>")).unwrap(),
            Some("Hi there &amp; you".into())
        );
        assert_eq!(
            description_excerpt(Some("<p><a href=\"x\">y</a> <b>z</b></p>")).unwrap(),
            Some("<a href=\"x\">y</a> z".into())
        );
    }

    #[test]
    fn style_types_match_the_enum() {
        assert_eq!(style_type(0), "square");
        assert_eq!(style_type(1), "icon");
        assert_eq!(style_type(2), "emoji");
    }
}
