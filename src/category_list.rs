//! Port of CategoriesController#index for anonymous users:
//! app/models/category_list.rb with CategoryListSerializer and
//! CategoryDetailedSerializer.
//!
//! Not ported: pagination (sites with more than 1000 categories or lazy
//! loaded categories), the `/c/.../subcategories` route, tag filtering,
//! and the `subcategories_with_featured_topics` page styles. The
//! `include_subcategories` and `parent_category_id` params are.

use chrono::NaiveDateTime;
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
    /// `params[:include_subcategories] == "true"`
    pub include_subcategories_param: bool,
    /// `params[:parent_category_id]`, when present.
    pub parent_category_param: Option<String>,
    /// `params[:page]`, 1 when absent.
    pub page: i64,
    /// The guardian's secure category ids, set by `json`.
    pub secure_ids: Vec<i32>,
}

struct Entry {
    row: CategoryRow,
    /// Unset (null) when listing one parent's children.
    subcategory_ids: Option<Vec<i32>>,
    /// With include_subcategories: the children, nested the same way.
    subcategory_list: Vec<Entry>,
    notification_level: i64,
    /// `CategoryGroup.permission_types[:full]` where the viewer may create topics.
    permission: Option<i64>,
    has_children: bool,
    subcategory_count: Option<i64>,
    topics: Vec<TopicRow>,
}

impl CategoryList<'_> {
    /// `Category.secured(guardian)` as a WHERE fragment.
    fn secured(&self) -> String {
        if self.secure_ids.is_empty() {
            "NOT categories.read_restricted".to_string()
        } else {
            format!(
                "(NOT categories.read_restricted OR categories.id IN ({}))",
                self.secure_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }

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

    fn subcategories_page_style(&self) -> Result<bool, CategoriesError> {
        Ok(
            self.setting_str("desktop_category_page_style")?
                == "subcategories_with_featured_topics"
                || self.setting_str("mobile_category_page_style")?
                    == "subcategories_with_featured_topics",
        )
    }

    /// fetch_category_list's parent_category: by top-level slug
    /// (`Category.find_by_slug`), else by id; its id and
    /// subcategory_list_style.
    async fn parent_category(&mut self) -> Result<Option<(i32, String)>, CategoriesError> {
        let Some(param) = self.parent_category_param.clone() else {
            return Ok(None);
        };
        let slug = param.to_lowercase();
        if !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(Unsupported("parent_category_id slugs needing CGI escapes").into());
        }
        // find_by_slug_path's "<id>-category" slugs match by id too.
        let slug_id: Option<i32> = slug
            .split_once("-category")
            .and_then(|(id, _)| id.parse().ok());
        let by_slug: Option<(i32, String)> = sqlx::query_as(
            "SELECT id, subcategory_list_style FROM categories \
             WHERE parent_category_id IS NULL AND (slug = $1 OR id = $2) ORDER BY id LIMIT 1",
        )
        .bind(&slug)
        .bind(slug_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        if by_slug.is_some() {
            return Ok(by_slug);
        }
        Ok(
            sqlx::query_as("SELECT id, subcategory_list_style FROM categories WHERE id = $1")
                .bind(crate::ruby::to_i(&param) as i32)
                .fetch_optional(&mut *self.conn)
                .await?,
        )
    }

    /// The whole `{"category_list": ...}` document.
    pub async fn json(&mut self) -> Result<Value, CategoriesError> {
        if self.subcategories_page_style()? {
            return Err(Unsupported("the subcategories_with_featured_topics page styles").into());
        }
        let parent = self.parent_category().await?;
        let parent_id = parent.as_ref().map(|(id, _)| *id);
        // Category.secured(guardian) for every query below.
        let secure_ids = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        self.secure_ids = secure_ids;
        // paginate_results?
        let total: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM categories WHERE {} AND ($1::int IS NULL OR parent_category_id = $1)",
            self.secured()
        ))
        .bind(parent_id)
        .fetch_one(&mut *self.conn)
        .await?;
        if total > 1000 || self.guardian.can_lazy_load_categories(self.settings)? {
            return Err(Unsupported("paginated category lists").into());
        }
        let include_topics = self.include_topics()?
            || parent
                .as_ref()
                .is_some_and(|(_, style)| style.ends_with("with_featured_topics"));

        // Every category listed, descendants included, until nested below.
        let mut entries = if self.page > 1 {
            // Without pagination, `page > 1` is `query.none`.
            Vec::new()
        } else {
            self.find_categories(parent_id).await?
        };
        if include_topics {
            self.find_relevant_topics(&mut entries).await?;
            self.sort_unpinned(&mut entries).await?;
        }
        // trim_results
        for e in &mut entries {
            let n = e.row.num_featured_topics.max(0) as usize;
            e.topics.truncate(n);
        }
        let mut entries = nest(entries, parent_id.is_some());
        // prune_empty: drop Uncategorized unless uncategorized topics are allowed.
        if !self.settings.get("allow_uncategorized_topics")?.truthy() {
            let uncategorized = self.settings.get("uncategorized_category_id")?.to_i();
            entries.retain(|e| i64::from(e.row.id) != uncategorized);
        }
        // demote_muted (stable).
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
                "can_create_category": self.guardian.is_admin()
                    || (self.settings.get("moderators_manage_categories")?.truthy()
                        && self.guardian.is_moderator()),
                "can_create_topic": self.guardian.can_create_topic(&mut *self.conn, self.settings).await?,
                "categories": categories,
            }
        }))
    }

    /// `find_categories`: readable categories in featured-activity order
    /// (a parent's children only, when given), then
    /// `Category.preload_user_fields!`. Without a parent, children become
    /// their parents' subcategory_ids and stay listed (for `nest`) only
    /// with include_subcategories.
    async fn find_categories(
        &mut self,
        parent: Option<i32>,
    ) -> Result<Vec<Entry>, CategoriesError> {
        let all = Categories::load_all(&mut *self.conn).await?;
        let visible_ids: Vec<i32> = all
            .iter()
            .filter(|c| !c.read_restricted || self.secure_ids.contains(&c.id))
            .map(|c| c.id)
            .collect();

        let secured = self.secured();
        let ordered_ids: Vec<i32> = if self.settings.get("fixed_category_positions")?.truthy() {
            sqlx::query_scalar(&format!(
                "SELECT id FROM categories WHERE {secured} ORDER BY position, id"
            ))
            .fetch_all(&mut *self.conn)
            .await?
        } else {
            sqlx::query_scalar(&format!(
                "SELECT categories.id FROM categories \
                 LEFT OUTER JOIN category_featured_topics cft ON cft.category_id = categories.id \
                 LEFT OUTER JOIN topics ON topics.deleted_at IS NULL AND topics.id = cft.topic_id \
                 WHERE {secured} \
                   AND (topics.category_id IS NULL OR topics.category_id = ANY($1)) \
                 GROUP BY categories.id \
                 ORDER BY max(topics.bumped_at) DESC NULLS LAST, categories.id ASC"
            ))
            .bind(&visible_ids)
            .fetch_all(&mut *self.conn)
            .await?
        };
        let rows: Vec<CategoryRow> = ordered_ids
            .iter()
            .filter_map(|id| all.iter().find(|c| c.id == *id).cloned())
            .filter(|c| parent.is_none_or(|p| c.parent_category_id == Some(p)))
            .collect();
        let subcategory_ids = |id: i32| -> Vec<i32> {
            rows.iter()
                .filter(|c| c.parent_category_id == Some(id))
                .map(|c| c.id)
                .collect()
        };
        let mut top: Vec<Entry> = Vec::new();
        for row in &rows {
            if parent.is_none()
                && row.parent_category_id.is_some()
                && !self.include_subcategories_param
            {
                continue;
            }
            top.push(Entry {
                row: row.clone(),
                subcategory_ids: parent.is_none().then(|| subcategory_ids(row.id)),
                subcategory_list: Vec::new(),
                notification_level: 1,
                permission: None,
                has_children: false,
                subcategory_count: None,
                topics: Vec::new(),
            });
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
        .notification_levels()
        .await?;
        let default_level = if self
            .settings
            .get("mute_all_categories_by_default")?
            .truthy()
        {
            MUTED
        } else {
            1
        };
        // allowed_topic_create_ids: nil for admins and anonymous users.
        let allowed_topic_create = if self.guardian.is_admin() {
            None
        } else {
            Some(
                self.guardian
                    .topic_create_allowed_category_ids(&mut *self.conn, self.settings)
                    .await?,
            )
        };
        for e in &mut top {
            e.notification_level = levels
                .iter()
                .find(|(id, _)| *id == i64::from(e.row.id))
                .map(|(_, l)| *l)
                .unwrap_or(default_level);
            e.permission = allowed_topic_create
                .as_ref()
                .is_none_or(|ids| ids.contains(&e.row.id))
                .then_some(1);
            let count: i64 = sqlx::query_scalar(&format!(
                "SELECT count(*) FROM categories WHERE parent_category_id = $1 AND ({secured})"
            ))
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
        // A user's muted topics drop out; muted tags too, by the user's
        // tag_users rows and watched-precedence option.
        let (joins, muted) = match self.guardian.user_id() {
            Some(uid) => {
                let muted_tags = self.muted_tags_clause(uid).await?;
                (
                    format!(
                        " LEFT JOIN topic_users tu ON topics.id = tu.topic_id AND tu.user_id = {uid} \
                         LEFT JOIN category_users ON category_users.category_id = topics.category_id AND category_users.user_id = {uid}"
                    ),
                    format!(" AND (COALESCE(tu.notification_level,1) > 0){muted_tags}"),
                )
            }
            None => (String::new(), String::new()),
        };
        let sql = format!(
            "SELECT cft.category_id AS featured_category_id, {TOPIC_COLUMNS} FROM topics \
             INNER JOIN category_featured_topics cft ON topics.id = cft.topic_id{joins} \
             WHERE topics.deleted_at IS NULL AND topics.visible AND topics.archetype <> 'private_message' \
               AND topics.category_id IN (SELECT id FROM categories WHERE {secured}) \
               AND cft.category_id = ANY($1){muted} \
             ORDER BY cft.rank",
            secured = self.secured()
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

    /// `TopicQuery.remove_muted_tags` for the featured topics of a user.
    async fn muted_tags_clause(&mut self, uid: i32) -> Result<String, CategoriesError> {
        if !self.settings.get("tagging_enabled")?.truthy()
            || self.settings.get("remove_muted_tags_from_latest")?.to_s() == "never"
        {
            return Ok(String::new());
        }
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT tag_id FROM tag_users WHERE user_id = $1 AND notification_level = 0 ORDER BY tag_id",
        )
        .bind(uid)
        .fetch_all(&mut *self.conn)
        .await?;
        if ids.is_empty() {
            return Ok(String::new());
        }
        let precedence: Option<bool> = sqlx::query_scalar(
            "SELECT watched_precedence_over_muted FROM user_options WHERE user_id = $1",
        )
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?;
        let watching_or_infinite = if precedence.unwrap_or(false) { 3 } else { 99 };
        let ids = ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        Ok(
            match self
                .settings
                .get("remove_muted_tags_from_latest")?
                .to_s()
                .as_str()
            {
                "always" => format!(
                    " AND (NOT EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.tag_id IN ({ids}) AND tt.topic_id = topics.id \
                 AND COALESCE(category_users.notification_level, 1) < {watching_or_infinite}))"
                ),
                "only_muted" => format!(
                    " AND (EXISTS (SELECT 1 FROM topic_tags tt WHERE (tt.tag_id NOT IN ({ids}) AND tt.topic_id = topics.id) \
                 OR COALESCE(category_users.notification_level, 1) >= {watching_or_infinite}) \
                 OR NOT EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.topic_id = topics.id))"
                ),
                _ => String::new(),
            },
        )
    }

    /// `sort_unpinned`: in categories with more featured topics than are
    /// shown, pins the user cleared move to the end.
    async fn sort_unpinned(&mut self, entries: &mut [Entry]) -> Result<(), CategoriesError> {
        let Some(uid) = self.guardian.user_id() else {
            return Ok(());
        };
        for e in entries.iter_mut() {
            if e.topics.len() <= e.row.num_featured_topics.max(0) as usize {
                continue;
            }
            let ids: Vec<i32> = e.topics.iter().map(|t| t.id).collect();
            let cleared: Vec<(i32, NaiveDateTime)> = sqlx::query_as(
                "SELECT topic_id, cleared_pinned_at FROM topic_users \
                 WHERE topic_id = ANY($1) AND user_id = $2 AND cleared_pinned_at IS NOT NULL",
            )
            .bind(&ids)
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?;
            let unpinned = |t: &TopicRow| {
                t.pinned_at.is_some_and(|pinned_at| {
                    cleared
                        .iter()
                        .any(|(id, at)| *id == t.id && *at > pinned_at)
                })
            };
            let (moved, kept): (Vec<TopicRow>, Vec<TopicRow>) =
                e.topics.drain(..).partition(unpinned);
            e.topics = kept.into_iter().chain(moved).collect();
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
        out.insert("permission".into(), json!(e.permission));
        out.insert("notification_level".into(), json!(e.notification_level));
        // can_edit?(category): admins, or moderators who manage categories.
        if self.guardian.is_admin()
            || (self.settings.get("moderators_manage_categories")?.truthy()
                && self.guardian.is_moderator())
        {
            out.insert("can_edit".into(), json!(true));
        }
        out.insert("has_children".into(), json!(e.has_children));
        out.insert("subcategory_count".into(), json!(e.subcategory_count));
        // custom_fields: plugins only.

        // count_with_subcategories: own count plus each readable direct child's.
        let children: Vec<(i32, i32, i32, i32, i32)> = sqlx::query_as(&format!(
            "SELECT topics_day, topics_week, topics_month, topics_year, topic_count FROM categories \
             WHERE parent_category_id = $1 AND {}",
            self.secured()
        ))
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
                group_id: None,
                prefetched: Default::default(),
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
        if !e.subcategory_list.is_empty() {
            let mut list = Vec::with_capacity(e.subcategory_list.len());
            for child in &e.subcategory_list {
                list.push(Box::pin(self.serialize(child)).await?);
            }
            out.insert("subcategory_list".into(), Value::Array(list));
        }
        Ok(Value::Object(out))
    }
}

/// The listed categories as CategoryList leaves them: the top level, each
/// with its subcategory_list. Listing one parent's children, they are all
/// top level; otherwise a child whose parent is not listed drops out.
fn nest(entries: Vec<Entry>, of_parent: bool) -> Vec<Entry> {
    if of_parent {
        return entries;
    }
    fn attach(parent: &mut Entry, pool: &mut Vec<Entry>) {
        let (mine, rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(pool)
            .into_iter()
            .partition(|c| c.row.parent_category_id == Some(parent.row.id));
        *pool = rest;
        parent.subcategory_list = mine;
        for child in &mut parent.subcategory_list {
            attach(child, pool);
        }
    }
    let (mut top, mut children): (Vec<Entry>, Vec<Entry>) = entries
        .into_iter()
        .partition(|e| e.row.parent_category_id.is_none());
    for e in &mut top {
        attach(e, &mut children);
    }
    top
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
