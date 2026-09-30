//! Port of lib/topic_query.rb for anonymous users: `list_latest`, its
//! `default_results` filters, ordering, paging and pinned prioritization.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

/// `TopTopic.periods`
pub const PERIODS: [&str; 6] = ["all", "yearly", "quarterly", "monthly", "weekly", "daily"];

/// `TopicQuery::DEFAULT_PER_PAGE_COUNT`
pub const DEFAULT_PER_PAGE: i64 = 30;

/// The `topics` columns the list serializers read.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TopicRow {
    pub id: i32,
    pub title: String,
    pub fancy_title: Option<String>,
    pub slug: Option<String>,
    pub posts_count: i32,
    pub reply_count: i32,
    pub highest_post_number: i32,
    pub image_upload_id: Option<i64>,
    pub created_at: NaiveDateTime,
    pub last_posted_at: Option<NaiveDateTime>,
    pub bumped_at: NaiveDateTime,
    pub archetype: String,
    pub pinned_at: Option<NaiveDateTime>,
    pub pinned_globally: bool,
    pub excerpt: Option<String>,
    pub visible: bool,
    pub closed: bool,
    pub archived: bool,
    pub views: i32,
    pub like_count: i32,
    pub has_summary: bool,
    pub user_id: Option<i32>,
    pub last_post_user_id: i32,
    pub featured_user1_id: Option<i32>,
    pub featured_user2_id: Option<i32>,
    pub featured_user3_id: Option<i32>,
    pub featured_user4_id: Option<i32>,
    pub category_id: Option<i32>,
    pub featured_link: Option<String>,
    pub visibility_reason_id: Option<i32>,
}

pub const TOPIC_COLUMNS: &str = "topics.id, topics.title, topics.fancy_title, topics.slug, topics.posts_count, \
    topics.reply_count, topics.highest_post_number, topics.image_upload_id, topics.created_at, \
    topics.last_posted_at, topics.bumped_at, topics.archetype, topics.pinned_at, \
    topics.pinned_globally, topics.excerpt, topics.visible, topics.closed, topics.archived, \
    topics.views, topics.like_count, topics.has_summary, topics.user_id, topics.last_post_user_id, \
    topics.featured_user1_id, topics.featured_user2_id, topics.featured_user3_id, \
    topics.featured_user4_id, topics.category_id, topics.featured_link, topics.visibility_reason_id";

/// The list options a request can set (TopicQuery.public_valid_options
/// subset ported so far), already validated.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub page: i64,
    pub per_page: Option<i64>,
    pub order: Option<String>,
    pub ascending: bool,
    /// ListController sets this for /latest without a category.
    pub no_definitions: bool,
    /// `options[:category]` resolved to an id: the list is scoped to it and
    /// its subcategories.
    pub category_id: Option<i32>,
    pub no_subcategories: bool,
}

#[derive(Debug)]
pub enum TopicQueryError {
    Db(sqlx::Error),
    Setting(SettingError),
    Unsupported(Unsupported),
}

impl std::fmt::Display for TopicQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TopicQueryError::Db(e) => write!(f, "querying topics: {e}"),
            TopicQueryError::Setting(e) => e.fmt(f),
            TopicQueryError::Unsupported(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for TopicQueryError {}

impl From<sqlx::Error> for TopicQueryError {
    fn from(e: sqlx::Error) -> Self {
        TopicQueryError::Db(e)
    }
}

impl From<SettingError> for TopicQueryError {
    fn from(e: SettingError) -> Self {
        TopicQueryError::Setting(e)
    }
}

impl From<Unsupported> for TopicQueryError {
    fn from(e: Unsupported) -> Self {
        TopicQueryError::Unsupported(e)
    }
}

pub struct TopicQuery<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    pub options: Options,
    /// Set by the list_* methods.
    pub filter: Filter,
    /// Set by list_latest from `options.category_id`.
    pub category: CategoryScope,
}

/// The result of `create_list`: the page of topics plus what TopicList
/// carries for the serializer.
/// Which list is being built: changes joins, order and pinning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Latest,
    /// `list_top_for(period)`
    Top(String),
    /// `list_hot`
    Hot,
}

pub struct TopicList {
    pub filter: &'static str,
    pub topics: Vec<TopicRow>,
    pub per_page: i64,
}

impl TopicQuery<'_> {
    fn per_page(&self) -> i64 {
        self.options.per_page.unwrap_or(DEFAULT_PER_PAGE)
    }

    /// `list_top_for(period)`: create_list(:top, unordered: true), joined to
    /// top_topics with a positive period score, ordered by that score then
    /// bumped_at; no pinned prioritization.
    pub async fn list_top_for(&mut self, period: &str) -> Result<TopicList, TopicQueryError> {
        // The period names a column; only TopTopic.periods may reach the SQL.
        if !PERIODS.contains(&period) {
            return Err(Unsupported("unknown top period").into());
        }
        self.check_unported_filters()?;
        self.filter = Filter::Top(period.to_string());
        self.category = self.category_scope().await?;
        let per_page = self.per_page();
        let order = self.order_clause()?;
        let topics = self
            .fetch("TRUE", &order, per_page, self.options.page * per_page)
            .await?;
        Ok(TopicList {
            filter: "top",
            topics,
            per_page,
        })
    }

    /// `list_hot`: create_list(:hot, unordered: true, prioritize_pinned: true),
    /// joined to topic_hot_scores, by score, pinned topics first.
    pub async fn list_hot(&mut self) -> Result<TopicList, TopicQueryError> {
        self.check_unported_filters()?;
        self.filter = Filter::Hot;
        self.category = self.category_scope().await?;
        let topics = self.prioritize_pinned_topics().await?;
        Ok(TopicList {
            filter: "hot",
            topics,
            per_page: self.per_page(),
        })
    }

    /// `list_latest` -> `create_list(:latest, {}, latest_results)`.
    pub async fn list_latest(&mut self) -> Result<TopicList, TopicQueryError> {
        self.check_unported_filters()?;
        self.category = self.category_scope().await?;
        let topics = self.prioritize_pinned_topics().await?;
        Ok(TopicList {
            filter: "latest",
            topics,
            per_page: self.per_page(),
        })
    }

    /// Settings that add filters `default_results` doesn't port yet.
    fn check_unported_filters(&self) -> Result<(), TopicQueryError> {
        let s = self.settings;
        if s.get("mute_all_categories_by_default")?.truthy() {
            return Err(Unsupported("mute_all_categories_by_default in topic lists").into());
        }
        if s.get("default_categories_muted")?.presence().is_some() {
            return Err(Unsupported("default_categories_muted in topic lists").into());
        }
        if s.get("tagging_enabled")?.truthy()
            && s.get("remove_muted_tags_from_latest")?.to_s() != "never"
            && s.get("default_tags_muted")?.presence().is_some()
        {
            return Err(Unsupported("default_tags_muted in topic lists").into());
        }
        if s.get("shared_drafts_category")?.presence().is_some() {
            return Err(Unsupported("shared_drafts_category in topic lists").into());
        }
        Ok(())
    }

    /// The WHERE clause of `default_results` for an anonymous user:
    /// not deleted, category readable (or none), not a PM, not a category
    /// definition topic (no_definitions), visible.
    /// The category clauses of `default_results`, computed up front since
    /// they need the subcategory ids and the category's default sort.
    async fn category_scope(&mut self) -> Result<CategoryScope, TopicQueryError> {
        let Some(category_id) = self.options.category_id else {
            return Ok(CategoryScope::default());
        };
        let ids: Vec<i32> = if self.options.no_subcategories {
            vec![category_id]
        } else {
            // Category.subcategory_ids: descendants up to max_category_nesting.
            let nesting = self.settings.get("max_category_nesting")?.to_i();
            sqlx::query_scalar(
                "WITH RECURSIVE subcategories AS ( \
                     SELECT $1::int AS id, 1 AS depth \
                     UNION \
                     SELECT categories.id, subcategories.depth + 1 \
                     FROM categories JOIN subcategories ON subcategories.id = categories.parent_category_id \
                     WHERE subcategories.depth < $2) \
                 SELECT id FROM subcategories",
            )
            .bind(category_id)
            .bind(nesting as i32)
            .fetch_all(&mut *self.conn)
            .await?
        };
        let mut clauses = vec![format!(
            "topics.category_id IN ({})",
            ids.iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",")
        )];
        // Subcategory definition topics are hidden, the category's own shown.
        if !self.options.no_subcategories
            && !self
                .settings
                .get("show_category_definitions_in_topic_lists")?
                .truthy()
        {
            clauses.push(format!(
                "(categories.topic_id IS DISTINCT FROM topics.id OR topics.category_id = {category_id})"
            ));
        }
        // The category's default sort applies to latest (default/unseen
        // filters) when no order was given; top and hot keep their own.
        let mut order = None;
        if self.options.order.is_none() && self.filter == Filter::Latest {
            let sort: Option<(Option<String>, Option<bool>)> =
                sqlx::query_as("SELECT sort_order, sort_ascending FROM categories WHERE id = $1")
                    .bind(category_id)
                    .fetch_optional(&mut *self.conn)
                    .await?;
            if let Some((Some(sort_order), ascending)) = sort {
                if !sort_order.is_empty() {
                    order = Some((sort_order, ascending.unwrap_or(false)));
                }
            }
        }
        Ok(CategoryScope {
            clauses,
            order,
            pinned_clause: Some(format!(
                "topics.category_id = {category_id} AND pinned_at IS NOT NULL"
            )),
        })
    }

    fn where_clause(&self) -> String {
        let mut clauses = vec![
            "topics.deleted_at IS NULL".to_string(),
            "(categories.id IS NULL OR categories.id IN (SELECT id FROM categories WHERE NOT read_restricted))".to_string(),
            "topics.archetype <> 'private_message'".to_string(),
        ];
        if self.options.no_definitions {
            clauses.push("COALESCE(categories.topic_id, 0) <> topics.id".to_string());
        }
        clauses.extend(self.category.clauses.iter().cloned());
        clauses.push("topics.visible = TRUE".to_string());
        clauses.join(" AND ")
    }

    /// `apply_ordering`: `SORTABLE_MAPPING` with `order`, default bumped_at
    /// DESC, no tiebreaker.
    /// The list's ORDER BY: `apply_ordering`'s column (the request's order,
    /// or the category default for latest), then for top/hot the block's
    /// own order, which is the whole order when nothing was requested.
    fn order_clause(&self) -> Result<String, TopicQueryError> {
        let block = match &self.filter {
            Filter::Latest => None,
            Filter::Top(period) => Some(format!(
                "COALESCE(top_topics.{period}_score, 0) DESC, topics.bumped_at DESC"
            )),
            Filter::Hot => Some("topic_hot_scores.score DESC".to_string()),
        };
        if let Some(block) = &block {
            if self.options.order.is_none() {
                return Ok(block.clone());
            }
        }
        let (order, ascending) = match &self.category.order {
            Some((o, a)) => (Some(o.as_str()), *a),
            None => (self.options.order.as_deref(), self.options.ascending),
        };
        let column = match order {
            None | Some("default") | Some("activity") => "topics.bumped_at".to_string(),
            Some("likes") => "topics.like_count".to_string(),
            Some("op_likes") => "(SELECT like_count FROM posts p3 WHERE p3.topic_id = topics.id AND p3.post_number = 1)".to_string(),
            Some("views") => "topics.views".to_string(),
            Some("posts") => "topics.posts_count".to_string(),
            Some("posters") => "topics.participant_count".to_string(),
            Some("created") => "topics.created_at".to_string(),
            Some("category") => {
                let uncategorized = self.settings.get("uncategorized_category_id")?.to_i();
                format!("CASE WHEN categories.id = {uncategorized} THEN '' ELSE categories.name END")
            }
            Some(_) => return Err(Unsupported("unknown topic list order").into()),
        };
        let dir = if ascending { "ASC" } else { "DESC" };
        Ok(match block {
            Some(block) => format!("{column} {dir}, {block}"),
            None => format!("{column} {dir}"),
        })
    }

    async fn fetch(
        &mut self,
        extra_where: &str,
        order: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TopicRow>, TopicQueryError> {
        let (join, filter_where) = match &self.filter {
            Filter::Latest => (String::new(), "TRUE".to_string()),
            Filter::Top(period) => (
                "INNER JOIN top_topics ON top_topics.topic_id = topics.id".to_string(),
                format!("top_topics.{period}_score > 0"),
            ),
            Filter::Hot => (
                "JOIN topic_hot_scores ON topics.id = topic_hot_scores.topic_id".to_string(),
                "TRUE".to_string(),
            ),
        };
        let sql = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics \
             LEFT OUTER JOIN categories ON categories.id = topics.category_id {join} \
             WHERE {} AND ({filter_where}) AND ({extra_where}) ORDER BY {order} LIMIT $1 OFFSET $2",
            self.where_clause()
        );
        Ok(sqlx::query_as(&sql)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// `prioritize_pinned_topics` without a category: globally pinned
    /// topics first (newest pin first), then the rest in list order.
    async fn prioritize_pinned_topics(&mut self) -> Result<Vec<TopicRow>, TopicQueryError> {
        let per_page = self.per_page();
        let page = self.options.page;
        let order = self.order_clause()?;
        // apply_pinning: only with the default/activity order (the request's,
        // not the category's default sort).
        let apply_pinning = matches!(
            self.options.order.as_deref(),
            None | Some("activity") | Some("default")
        );
        if !apply_pinning {
            return self.fetch("TRUE", &order, per_page, page * per_page).await;
        }
        let pinned_clause = self
            .category
            .pinned_clause
            .clone()
            .unwrap_or_else(|| "pinned_globally AND pinned_at IS NOT NULL".to_string());
        let pinned_clause = pinned_clause.as_str();
        let unpinned_clause = format!("NOT ({pinned_clause})");

        if page == 0 {
            let mut topics = self
                .fetch(pinned_clause, "topics.pinned_at DESC", per_page, 0)
                .await?;
            let unpinned = self.fetch(&unpinned_clause, &order, per_page, 0).await?;
            topics.extend(unpinned);
            topics.truncate(per_page as usize);
            Ok(topics)
        } else {
            let pinned_count: i64 = self
                .fetch(pinned_clause, "topics.pinned_at DESC", per_page, 0)
                .await?
                .len() as i64;
            let offset = (page * per_page - pinned_count).max(0);
            self.fetch(&unpinned_clause, &order, per_page, offset).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_page_defaults_to_thirty() {
        assert_eq!(DEFAULT_PER_PAGE, 30);
        let o = Options::default();
        assert_eq!(o.per_page.unwrap_or(DEFAULT_PER_PAGE), 30);
        assert!(!o.ascending);
    }
}

/// `Topic#fancy_title`: the stored column, else `Topic.fancy_title(title)`
/// computed on read. Only the trivial case is ported: a title with nothing
/// for HtmlPrettify (quotes, dashes, ellipses, backticks, entities) or the
/// emoji unescape to rewrite comes back HTML-escaped, which for such titles
/// is the title itself.
pub fn fancy_title(t: &TopicRow) -> Result<String, Unsupported> {
    if let Some(f) = &t.fancy_title {
        return Ok(f.clone());
    }
    let plain = t.title.chars().all(|c| {
        c.is_alphanumeric() && c.is_ascii()
            || " ,.:;!?()/_[]{}%#@+=*$^|~".contains(c)
            || c.is_alphabetic()
    });
    if !plain || crate::emoji::has_emoji_code(&t.title) {
        return Err(Unsupported(
            "computing fancy_title (HtmlPrettify + emoji unescape)",
        ));
    }
    Ok(t.title.clone())
}

#[cfg(test)]
mod fancy_title_tests {
    use super::*;
    use chrono::NaiveDateTime;

    fn row(title: &str, fancy: Option<&str>) -> TopicRow {
        let t = NaiveDateTime::parse_from_str("2026-01-01 00:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        TopicRow {
            id: 1,
            title: title.into(),
            fancy_title: fancy.map(str::to_string),
            slug: None,
            posts_count: 0,
            reply_count: 0,
            highest_post_number: 0,
            image_upload_id: None,
            created_at: t,
            last_posted_at: None,
            bumped_at: t,
            archetype: "regular".into(),
            pinned_at: None,
            pinned_globally: false,
            excerpt: None,
            visible: true,
            closed: false,
            archived: false,
            views: 0,
            like_count: 0,
            has_summary: false,
            user_id: None,
            last_post_user_id: 1,
            featured_user1_id: None,
            featured_user2_id: None,
            featured_user3_id: None,
            featured_user4_id: None,
            category_id: None,
            featured_link: None,
            visibility_reason_id: None,
        }
    }

    #[test]
    fn stored_or_trivially_computed() {
        assert_eq!(fancy_title(&row("x", Some("stored"))).unwrap(), "stored");
        assert_eq!(
            fancy_title(&row("Parity fixture: unlisted topic", None)).unwrap(),
            "Parity fixture: unlisted topic"
        );
        assert!(fancy_title(&row("it's \"quoted\"", None)).is_err());
        assert!(fancy_title(&row("dash -- dash", None)).is_err());
        assert!(fancy_title(&row("Hi :wave:", None)).is_err());
    }
}

/// What a category adds to the query.
#[derive(Debug, Clone, Default)]
pub struct CategoryScope {
    clauses: Vec<String>,
    /// The category's `sort_order`/`sort_ascending`, when it has one and
    /// the request gave no order.
    order: Option<(String, bool)>,
    pinned_clause: Option<String>,
}
