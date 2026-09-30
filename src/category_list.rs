//! Port of CategoriesController#index for anonymous users:
//! app/models/category_list.rb with CategoryListSerializer and
//! CategoryDetailedSerializer.
//!
//! Not ported: pagination (sites with more than 1000 categories or lazy
//! loaded categories), a parent category (`/c/.../subcategories`), tag
//! filtering, and `subcategory_list` nesting (only emitted for the
//! `subcategories_with_featured_topics` page styles).

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::categories::{Categories, CategoriesError, CategoryRow};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::SiteSettings;
use crate::topic_list::{self, Mode, TopicListSerializer};
use crate::topic_query::{TOPIC_COLUMNS, TopicRow};
use crate::url::Urls;

/// `CategoryUser.notification_levels`
const MUTED: i64 = 0;

pub struct CategoryList<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
    /// `params[:include_topics]` present (any value counts as true).
    pub include_topics_param: bool,
    /// `params[:page]`, 1 when absent.
    pub page: i64,
}

struct Entry {
    row: CategoryRow,
    subcategory_ids: Vec<i32>,
    notification_level: i64,
    has_children: bool,
    subcategory_count: Option<i64>,
    topics: Vec<TopicRow>,
}

impl CategoryList<'_> {
    fn setting_str(&self, name: &str) -> Result<String, CategoriesError> {
        Ok(self.settings.get(name)?.to_s())
    }

    /// CategoriesController#index's `include_topics` rule.
    fn include_topics(&self) -> Result<bool, CategoriesError> {
        let desktop = self.setting_str("desktop_category_page_style")?;
        let mobile = self.setting_str("mobile_category_page_style")?;
        Ok(self.include_topics_param
            || [
                "categories_with_featured_topics",
                "subcategories_with_featured_topics",
                "categories_boxes_with_topics",
                "categories_with_top_topics",
            ]
            .contains(&desktop.as_str())
            || [
                "categories_with_featured_topics",
                "categories_boxes_with_topics",
                "subcategories_with_featured_topics",
            ]
            .contains(&mobile.as_str()))
    }

    fn include_subcategories(&self) -> Result<bool, CategoriesError> {
        Ok(
            self.setting_str("desktop_category_page_style")?
                == "subcategories_with_featured_topics"
                || self.setting_str("mobile_category_page_style")?
                    == "subcategories_with_featured_topics",
        )
    }

    /// The whole `{"category_list": ...}` document.
    pub async fn json(&mut self) -> Result<Value, CategoriesError> {
        if self.include_subcategories()? {
            return Err(Unsupported(
                "subcategory_list (subcategories_with_featured_topics styles)",
            )
            .into());
        }
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM categories WHERE NOT read_restricted")
                .fetch_one(&mut *self.conn)
                .await?;
        if total > 1000 || self.guardian.can_lazy_load_categories(self.settings)? {
            return Err(Unsupported("paginated category lists").into());
        }

        let mut entries = if self.page > 1 {
            // Without pagination, `page > 1` is `query.none`.
            Vec::new()
        } else {
            self.find_categories().await?
        };
        if self.include_topics()? {
            self.find_relevant_topics(&mut entries).await?;
        }
        // prune_empty: drop Uncategorized unless uncategorized topics are allowed.
        if !self.settings.get("allow_uncategorized_topics")?.truthy() {
            let uncategorized = self.settings.get("uncategorized_category_id")?.to_i();
            entries.retain(|e| i64::from(e.row.id) != uncategorized);
        }
        // trim_results, then demote_muted (stable).
        for e in &mut entries {
            let n = e.row.num_featured_topics.max(0) as usize;
            e.topics.truncate(n);
        }
        let (muted, rest): (Vec<Entry>, Vec<Entry>) = entries
            .into_iter()
            .partition(|e| e.notification_level == MUTED);
        let entries: Vec<Entry> = rest.into_iter().chain(muted).collect();

        let mut categories = Vec::with_capacity(entries.len());
        for e in &entries {
            categories.push(self.serialize(e).await?);
        }
        Ok(json!({
            "category_list": {
                "can_create_category": false,
                "can_create_topic": self.guardian.is_authenticated(),
                "categories": categories,
            }
        }))
    }

    /// `find_categories`: readable categories in featured-activity order,
    /// children folded into their parents' subcategory_ids, then
    /// `Category.preload_user_fields!`.
    async fn find_categories(&mut self) -> Result<Vec<Entry>, CategoriesError> {
        let all = Categories::load_all(&mut *self.conn).await?;
        let visible_ids: Vec<i32> = all
            .iter()
            .filter(|c| !c.read_restricted)
            .map(|c| c.id)
            .collect();

        let ordered_ids: Vec<i32> = if self.settings.get("fixed_category_positions")?.truthy() {
            sqlx::query_scalar(
                "SELECT id FROM categories WHERE NOT read_restricted ORDER BY position, id",
            )
            .fetch_all(&mut *self.conn)
            .await?
        } else {
            sqlx::query_scalar(
                "SELECT categories.id FROM categories \
                 LEFT OUTER JOIN category_featured_topics cft ON cft.category_id = categories.id \
                 LEFT OUTER JOIN topics ON topics.id = cft.topic_id \
                 WHERE NOT categories.read_restricted \
                   AND (topics.category_id IS NULL OR topics.category_id = ANY($1)) \
                 GROUP BY categories.id \
                 ORDER BY max(topics.bumped_at) DESC NULLS LAST, categories.id ASC",
            )
            .bind(&visible_ids)
            .fetch_all(&mut *self.conn)
            .await?
        };
        let rows: Vec<CategoryRow> = ordered_ids
            .iter()
            .filter_map(|id| all.iter().find(|c| c.id == *id).cloned())
            .collect();

        // Children become their parents' subcategory_ids and drop out of
        // the top level, in load order.
        let mut top: Vec<Entry> = Vec::new();
        let mut children: Vec<(i32, i32)> = Vec::new();
        for row in rows {
            match row.parent_category_id {
                Some(parent) => children.push((parent, row.id)),
                None => top.push(Entry {
                    row,
                    subcategory_ids: Vec::new(),
                    notification_level: 1,
                    has_children: false,
                    subcategory_count: None,
                    topics: Vec::new(),
                }),
            }
        }
        for (parent, child) in &children {
            if let Some(e) = top.iter_mut().find(|e| e.row.id == *parent) {
                e.subcategory_ids.push(*child);
            }
        }

        // preload_user_fields!
        let levels = Categories {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: self.guardian,
            base_path: self.urls.config.globals.relative_url_root(),
            topic_url_via_slug: false,
        }
        .notification_levels()?;
        let default_level = if self
            .settings
            .get("mute_all_categories_by_default")?
            .truthy()
        {
            MUTED
        } else {
            1
        };
        for e in &mut top {
            e.notification_level = levels
                .iter()
                .find(|(id, _)| *id == i64::from(e.row.id))
                .map(|(_, l)| *l)
                .unwrap_or(default_level);
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM categories WHERE parent_category_id = $1 AND NOT read_restricted",
            )
            .bind(e.row.id)
            .fetch_one(&mut *self.conn)
            .await?;
            e.has_children = count > 0;
            e.subcategory_count = (count > 0).then_some(count);
        }
        Ok(top)
    }

    /// `find_relevant_topics`: featured topics by rank, grouped by the
    /// featured row's category.
    async fn find_relevant_topics(&mut self, entries: &mut [Entry]) -> Result<(), CategoriesError> {
        let ids: Vec<i32> = entries.iter().map(|e| e.row.id).collect();
        if self.settings.get("tagging_enabled")?.truthy()
            && self.settings.get("remove_muted_tags_from_latest")?.to_s() != "never"
            && self
                .settings
                .get("default_tags_muted")?
                .presence()
                .is_some()
        {
            return Err(Unsupported("default_tags_muted in category lists").into());
        }
        #[derive(sqlx::FromRow)]
        struct Featured {
            featured_category_id: i32,
            #[sqlx(flatten)]
            topic: TopicRow,
        }
        let sql = format!(
            "SELECT cft.category_id AS featured_category_id, {TOPIC_COLUMNS} FROM topics \
             INNER JOIN category_featured_topics cft ON topics.id = cft.topic_id \
             WHERE topics.deleted_at IS NULL AND topics.visible AND topics.archetype <> 'private_message' \
               AND topics.category_id IN (SELECT id FROM categories WHERE NOT read_restricted) \
               AND cft.category_id = ANY($1) \
             ORDER BY cft.rank"
        );
        let featured: Vec<Featured> = sqlx::query_as(&sql)
            .bind(&ids)
            .fetch_all(&mut *self.conn)
            .await?;
        for f in featured {
            if let Some(e) = entries
                .iter_mut()
                .find(|e| e.row.id == f.featured_category_id)
            {
                e.topics.push(f.topic);
            }
        }
        Ok(())
    }

    /// CategoryDetailedSerializer.
    async fn serialize(&mut self, e: &Entry) -> Result<Value, CategoriesError> {
        let base_path = self.urls.config.globals.relative_url_root();
        let mut cats = Categories {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: self.guardian,
            base_path,
            topic_url_via_slug: false,
        };
        let mut out = cats.basic_fields(&e.row).await?;
        out.insert("notification_level".into(), json!(e.notification_level));
        out.insert("has_children".into(), json!(e.has_children));
        out.insert("subcategory_count".into(), json!(e.subcategory_count));
        // custom_fields: plugins only.

        // count_with_subcategories: own count plus each readable direct child's.
        let children: Vec<(i32, i32, i32, i32, i32)> = sqlx::query_as(
            "SELECT topics_day, topics_week, topics_month, topics_year, topic_count FROM categories \
             WHERE parent_category_id = $1 AND NOT read_restricted",
        )
        .bind(e.row.id)
        .fetch_all(&mut *self.conn)
        .await?;
        let sum = |own: i32, pick: fn(&(i32, i32, i32, i32, i32)) -> i32| {
            i64::from(own) + children.iter().map(|c| i64::from(pick(c))).sum::<i64>()
        };
        out.insert("topics_day".into(), json!(sum(e.row.topics_day, |c| c.0)));
        out.insert("topics_week".into(), json!(sum(e.row.topics_week, |c| c.1)));
        out.insert(
            "topics_month".into(),
            json!(sum(e.row.topics_month, |c| c.2)),
        );
        out.insert("topics_year".into(), json!(sum(e.row.topics_year, |c| c.3)));
        out.insert(
            "topics_all_time".into(),
            json!(sum(e.row.topic_count, |c| c.4)),
        );
        if i64::from(e.row.id) == self.settings.get("uncategorized_category_id")?.to_i() {
            out.insert("is_uncategorized".into(), json!(true));
        }
        out.insert("subcategory_ids".into(), json!(e.subcategory_ids));
        let mut cats = Categories {
            conn: &mut *self.conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: self.guardian,
            base_path,
            topic_url_via_slug: false,
        };
        cats.uploads(&mut out, &e.row).await?;

        if !e.topics.is_empty() {
            let mut serializer = TopicListSerializer {
                conn: &mut *self.conn,
                settings: self.settings,
                i18n: self.i18n,
                guardian: self.guardian,
                urls: self.urls,
                more_topics_url: None,
                category_id: None,
            };
            let lookup = serializer.user_lookup(&e.topics).await?;
            let tagging = self.settings.get("tagging_enabled")?.truthy();
            let mut topics = Vec::with_capacity(e.topics.len());
            for t in &e.topics {
                let posters = topic_list::posters_summary(t, &lookup, self.i18n);
                topics.push(
                    serializer
                        .serialize_topic(t, &posters, tagging, Mode::Listable)
                        .await?,
                );
            }
            out.insert("topics".into(), Value::Array(topics));
        }
        Ok(Value::Object(out))
    }
}

impl From<crate::topic_list::TopicListError> for CategoriesError {
    fn from(e: crate::topic_list::TopicListError) -> Self {
        match e {
            crate::topic_list::TopicListError::Db(e) => CategoriesError::Db(e),
            crate::topic_list::TopicListError::Setting(e) => CategoriesError::Setting(e),
            crate::topic_list::TopicListError::Url(e) => CategoriesError::Url(e),
            crate::topic_list::TopicListError::Unsupported(e) => CategoriesError::Unsupported(e),
        }
    }
}
