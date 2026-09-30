//! Port of lib/topic_query.rb for anonymous users: `list_latest`, its
//! `default_results` filters, ordering, paging and pinned prioritization.

use chrono::NaiveDateTime;
use sqlx::PgConnection;

use crate::Unsupported;
use crate::guardian::Guardian;
use crate::site_settings::{SettingError, SiteSettings};

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

const TOPIC_COLUMNS: &str = "topics.id, topics.title, topics.fancy_title, topics.slug, topics.posts_count, \
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
}

/// The result of `create_list`: the page of topics plus what TopicList
/// carries for the serializer.
pub struct TopicList {
    pub filter: &'static str,
    pub topics: Vec<TopicRow>,
    pub per_page: i64,
}

impl TopicQuery<'_> {
    fn per_page(&self) -> i64 {
        self.options.per_page.unwrap_or(DEFAULT_PER_PAGE)
    }

    /// `list_latest` -> `create_list(:latest, {}, latest_results)`.
    pub async fn list_latest(&mut self) -> Result<TopicList, TopicQueryError> {
        self.check_unported_filters()?;
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
    fn where_clause(&self) -> String {
        let mut clauses = vec![
            "topics.deleted_at IS NULL".to_string(),
            "(categories.id IS NULL OR categories.id IN (SELECT id FROM categories WHERE NOT read_restricted))".to_string(),
            "topics.archetype <> 'private_message'".to_string(),
        ];
        if self.options.no_definitions {
            clauses.push("COALESCE(categories.topic_id, 0) <> topics.id".to_string());
        }
        clauses.push("topics.visible = TRUE".to_string());
        clauses.join(" AND ")
    }

    /// `apply_ordering`: `SORTABLE_MAPPING` with `order`, default bumped_at
    /// DESC, no tiebreaker.
    fn order_clause(&self) -> Result<String, TopicQueryError> {
        let column = match self.options.order.as_deref() {
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
        let dir = if self.options.ascending {
            "ASC"
        } else {
            "DESC"
        };
        Ok(format!("{column} {dir}"))
    }

    async fn fetch(
        &mut self,
        extra_where: &str,
        order: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TopicRow>, TopicQueryError> {
        let sql = format!(
            "SELECT {TOPIC_COLUMNS} FROM topics \
             LEFT OUTER JOIN categories ON categories.id = topics.category_id \
             WHERE {} AND ({extra_where}) ORDER BY {order} LIMIT $1 OFFSET $2",
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
        let pinned_clause = "pinned_globally AND pinned_at IS NOT NULL";
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
