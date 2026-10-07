//! Port of lib/topic_query.rb: the `list_*` methods, `default_results`
//! and `remove_muted` for anonymous and logged-in users, ordering, paging
//! and pinned prioritization.

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
    pub highest_staff_post_number: i32,
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
    pub subtype: Option<String>,
}

pub const TOPIC_COLUMNS: &str = "topics.id, topics.title, topics.fancy_title, topics.slug, topics.posts_count, \
    topics.reply_count, topics.highest_post_number, topics.highest_staff_post_number, topics.image_upload_id, topics.created_at, \
    topics.last_posted_at, topics.bumped_at, topics.archetype, topics.pinned_at, \
    topics.pinned_globally, topics.excerpt, topics.visible, topics.closed, topics.archived, \
    topics.views, topics.like_count, topics.has_summary, topics.user_id, topics.last_post_user_id, \
    topics.featured_user1_id, topics.featured_user2_id, topics.featured_user3_id, \
    topics.featured_user4_id, topics.category_id, topics.featured_link, topics.visibility_reason_id, \
    topics.subtype";

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
    /// `options[:tags]`: tag names the topics must all carry
    /// (`match_all_tags`, which TagsController always sets).
    pub tags: Vec<String>,
    /// `options[:no_tags]`: only untagged topics (`/tag/none`).
    pub no_tags: bool,
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

impl From<crate::guardian::GuardianError> for TopicQueryError {
    fn from(e: crate::guardian::GuardianError) -> Self {
        match e {
            crate::guardian::GuardianError::Db(e) => TopicQueryError::Db(e),
            crate::guardian::GuardianError::Setting(e) => TopicQueryError::Setting(e),
            crate::guardian::GuardianError::Unsupported(e) => TopicQueryError::Unsupported(e),
        }
    }
}

pub struct TopicQuery<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub guardian: &'a Guardian,
    pub options: Options,
    /// Set by `list`.
    pub filter: Filter,
    /// Set by `list` from `options.category_id`.
    pub category: CategoryScope,
    /// Set by `list` from `options.tags` (filter_by_tags).
    pub tags: TagScope,
    /// Set by `list` for a logged-in user.
    pub user: UserScope,
}

/// `filter_by_tags`: the WHERE clause and the resolved ids that become
/// `options[:tag_ids]` (TopicList#tags).
#[derive(Debug, Clone, Default)]
pub struct TagScope {
    clause: Option<String>,
    pub tag_ids: Vec<i32>,
}

/// The per-user inputs `default_results`, `remove_muted` and the
/// login-only filters read, fetched once per list.
#[derive(Debug, Clone, Default)]
pub struct UserScope {
    /// `TopicUser.notification_levels[:muted]` tag_users rows.
    muted_tag_ids: Vec<i32>,
    watched_precedence_over_muted: bool,
    treat_as_new_topic_start_date: Option<NaiveDateTime>,
    /// `user_stats.first_unread_at`
    first_unread_at: Option<NaiveDateTime>,
    first_seen_at: Option<NaiveDateTime>,
    whisperer: bool,
    unified_new: bool,
}

/// Which list is being built: changes joins, filters, order and pinning.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Latest,
    /// `list_top_for(period)`
    Top(String),
    /// `list_hot`
    Hot,
    /// `list_unread`, logged in only
    Unread,
    /// `list_new`, logged in only
    New,
    /// `list_unseen`, logged in only
    Unseen,
    /// `list_read`, logged in only
    Read,
    /// `list_posted`, logged in only
    Posted,
    /// `list_bookmarks`, logged in only
    Bookmarks,
}

impl Filter {
    /// `TopicList#filter`
    pub fn name(&self) -> &'static str {
        match self {
            Filter::Latest => "latest",
            Filter::Top(_) => "top",
            Filter::Hot => "hot",
            Filter::Unread => "unread",
            Filter::New => "new",
            Filter::Unseen => "unseen",
            Filter::Read => "read",
            Filter::Posted => "posted",
            Filter::Bookmarks => "bookmarks",
        }
    }

    /// `Discourse.filters - Discourse.anonymous_filters`: ListController's
    /// `ensure_logged_in` lists.
    pub fn requires_login(&self) -> bool {
        !matches!(self, Filter::Latest | Filter::Top(_) | Filter::Hot)
    }

    /// `create_list(..., unordered: true)`: the list brings its own order
    /// and skips `apply_ordering` and pinning.
    fn unordered(&self) -> bool {
        matches!(
            self,
            Filter::Top(_)
                | Filter::Hot
                | Filter::Unread
                | Filter::New
                | Filter::Unseen
                | Filter::Read
        )
    }

    /// Lists that go through `remove_muted`.
    fn removes_muted(&self) -> bool {
        matches!(
            self,
            Filter::Latest | Filter::Top(_) | Filter::Hot | Filter::New | Filter::Unseen
        )
    }
}

/// The result of `create_list`: the page of topics plus what TopicList
/// carries for the serializer.
pub struct TopicList {
    pub filter: &'static str,
    pub topics: Vec<TopicRow>,
    pub per_page: i64,
    /// `options[:tag_ids]` after filter_by_tags.
    pub tag_ids: Vec<i32>,
}

impl TopicQuery<'_> {
    fn per_page(&self) -> i64 {
        self.options.per_page.unwrap_or(DEFAULT_PER_PAGE)
    }

    fn user_id(&self) -> Option<i32> {
        self.guardian.user_id()
    }

    /// `list_<filter>` -> `create_list`: resolve the scopes, run the
    /// filter's query, page it.
    pub async fn list(&mut self, filter: Filter) -> Result<TopicList, TopicQueryError> {
        if let Filter::Top(period) = &filter {
            // The period names a column; only TopTopic.periods may reach the SQL.
            if !PERIODS.contains(&period.as_str()) {
                return Err(Unsupported("unknown top period").into());
            }
        }
        if filter.requires_login() && self.guardian.is_anonymous() {
            return Err(Unsupported("login-only list for anonymous").into());
        }
        self.check_unported_filters()?;
        self.filter = filter;
        self.category = self.category_scope().await?;
        self.tags = self.tag_scope().await?;
        self.user = self.user_scope().await?;
        let per_page = self.per_page();
        let topics = match &self.filter {
            Filter::Top(_) | Filter::Unseen => {
                let order = self.order_clause()?;
                self.fetch("TRUE", &order, per_page, self.options.page * per_page)
                    .await?
            }
            Filter::Unread => {
                let clause = format!("{} AND {}", self.unread_filter(), self.max_age_clause());
                let order = "CASE WHEN topics.user_id = tu.user_id THEN 1 ELSE 2 END, topics.bumped_at DESC";
                self.fetch(&clause, order, per_page, self.options.page * per_page)
                    .await?
            }
            Filter::New => {
                let new = format!(
                    "{} AND {} AND dismissed_topic_users.id IS NULL",
                    self.new_filter(),
                    self.remove_muted_clause()?
                );
                let (clause, order) = if self.user.unified_new {
                    (
                        format!(
                            "(({new}) OR ({} AND {}))",
                            self.unread_filter(),
                            self.max_age_clause()
                        ),
                        "CASE WHEN topics.user_id = tu.user_id THEN 1 ELSE 2 END, topics.bumped_at DESC",
                    )
                } else {
                    (new, "topics.bumped_at DESC")
                };
                self.fetch(&clause, order, per_page, self.options.page * per_page)
                    .await?
            }
            Filter::Read => {
                self.fetch(
                    "tu.last_visited_at IS NOT NULL",
                    "tu.last_visited_at DESC",
                    per_page,
                    self.options.page * per_page,
                )
                .await?
            }
            Filter::Latest | Filter::Hot | Filter::Posted | Filter::Bookmarks => {
                self.prioritize_pinned_topics().await?
            }
        };
        Ok(TopicList {
            filter: self.filter.name(),
            topics,
            per_page,
            tag_ids: self.tags.tag_ids.clone(),
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
        if self.guardian.is_anonymous()
            && s.get("tagging_enabled")?.truthy()
            && s.get("remove_muted_tags_from_latest")?.to_s() != "never"
            && s.get("default_tags_muted")?.presence().is_some()
        {
            return Err(Unsupported("default_tags_muted in topic lists").into());
        }
        if s.get("shared_drafts_category")?.presence().is_some() {
            return Err(Unsupported("shared_drafts_category in topic lists").into());
        }
        if s.get("max_category_nesting")?.to_i() > 2 && self.guardian.is_authenticated() {
            return Err(
                Unsupported("three-level category nesting in muted category checks").into(),
            );
        }
        Ok(())
    }

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
        // The category's default sort applies to ordered lists (latest,
        // posted, bookmarks) when no order was given; the rest keep their own.
        let mut order = None;
        if self.options.order.is_none() && !self.filter.unordered() {
            let sort: Option<(Option<String>, Option<bool>)> =
                sqlx::query_as("SELECT sort_order, sort_ascending FROM categories WHERE id = $1")
                    .bind(category_id)
                    .fetch_optional(&mut *self.conn)
                    .await?;
            if let Some((Some(sort_order), ascending)) = sort
                && !sort_order.is_empty()
            {
                order = Some((sort_order, ascending.unwrap_or(false)));
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

    /// `filter_by_tags` with `match_all_tags`: every named tag must exist
    /// and be visible (synonyms resolve to their target), else the list is
    /// empty (`result.none`); `no_tags` keeps untagged topics only.
    async fn tag_scope(&mut self) -> Result<TagScope, TopicQueryError> {
        if self.options.tags.is_empty() {
            if self.options.no_tags {
                return Ok(TagScope {
                    clause: Some(
                        "topics.id NOT IN (SELECT DISTINCT topic_id FROM topic_tags)".to_string(),
                    ),
                    tag_ids: Vec::new(),
                });
            }
            return Ok(TagScope::default());
        }
        let tag_ids = crate::tags::resolve_tag_ids(
            &mut *self.conn,
            self.guardian,
            self.settings,
            &self.options.tags,
        )
        .await?;
        let clause = if tag_ids.len() == self.options.tags.len() {
            let mut joined = String::new();
            for (index, id) in tag_ids.iter().enumerate() {
                let subquery =
                    format!("(SELECT topic_id FROM topic_tags WHERE tag_id = {id}) t{index}");
                if index == 0 {
                    joined = subquery;
                } else {
                    joined = format!(
                        "{joined} INNER JOIN {subquery} ON t{index}.topic_id = t0.topic_id"
                    );
                }
            }
            format!("topics.id IN (SELECT t0.topic_id FROM {joined})")
        } else {
            "FALSE".to_string()
        };
        Ok(TagScope {
            clause: Some(clause),
            tag_ids,
        })
    }

    /// The user's muting, new-topic and tracking inputs, in two queries.
    async fn user_scope(&mut self) -> Result<UserScope, TopicQueryError> {
        let Some(uid) = self.user_id() else {
            return Ok(UserScope::default());
        };
        let row: (Option<bool>, Option<NaiveDateTime>, Option<NaiveDateTime>) = sqlx::query_as(
            "SELECT uo.watched_precedence_over_muted, us.first_unread_at, u.first_seen_at \
             FROM users u LEFT JOIN user_options uo ON uo.user_id = u.id \
             LEFT JOIN user_stats us ON us.user_id = u.id WHERE u.id = $1",
        )
        .bind(uid)
        .fetch_one(&mut *self.conn)
        .await?;
        let muted_tag_ids = if self.settings.get("tagging_enabled")?.truthy()
            && self.settings.get("remove_muted_tags_from_latest")?.to_s() != "never"
        {
            sqlx::query_scalar(
                "SELECT tag_id FROM tag_users WHERE user_id = $1 AND notification_level = 0 ORDER BY tag_id",
            )
            .bind(uid)
            .fetch_all(&mut *self.conn)
            .await?
        } else {
            Vec::new()
        };
        Ok(UserScope {
            muted_tag_ids,
            watched_precedence_over_muted: row.0.unwrap_or(false),
            treat_as_new_topic_start_date: self
                .guardian
                .treat_as_new_topic_start_date(&mut *self.conn, self.settings)
                .await?,
            first_unread_at: row.1,
            first_seen_at: row.2,
            whisperer: self.guardian.is_whisperer(self.settings)?,
            unified_new: self
                .guardian
                .upcoming_change_enabled(&mut *self.conn, self.settings, "enable_unified_new")
                .await?,
        })
    }

    /// The joins after `categories`: `tu` for a user, `category_users`
    /// when muting applies, `dismissed_topic_users` for new, then the
    /// filter's own table.
    fn joins(&self) -> String {
        let mut joins = String::new();
        if let Some(uid) = self.user_id() {
            joins.push_str(&format!(
                " LEFT OUTER JOIN topic_users AS tu ON (topics.id = tu.topic_id AND tu.user_id = {uid})"
            ));
            if self.filter.removes_muted() {
                joins.push_str(&format!(
                    " LEFT JOIN category_users ON category_users.category_id = topics.category_id AND category_users.user_id = {uid}"
                ));
            }
            if self.filter == Filter::New {
                joins.push_str(&format!(
                    " LEFT JOIN dismissed_topic_users ON dismissed_topic_users.topic_id = topics.id AND dismissed_topic_users.user_id = {uid}"
                ));
            }
        }
        match &self.filter {
            Filter::Top(_) => {
                joins.push_str(" INNER JOIN top_topics ON top_topics.topic_id = topics.id")
            }
            Filter::Hot => {
                joins.push_str(" JOIN topic_hot_scores ON topics.id = topic_hot_scores.topic_id")
            }
            _ => {}
        }
        joins
    }

    /// The WHERE clause of `default_results`: not deleted, category
    /// readable (or none; admins skip this), not a PM, not a category
    /// definition topic (no_definitions), the category and tag scopes,
    /// visible unless the user may see unlisted topics; then the filter's
    /// own clause and `remove_muted` where the list applies it.
    fn where_clause(&self) -> Result<String, TopicQueryError> {
        let mut clauses = vec!["topics.deleted_at IS NULL".to_string()];
        let admin_sees_all = self.guardian.is_admin()
            && !self
                .settings
                .get("suppress_secured_categories_from_admin")?
                .truthy();
        if !admin_sees_all {
            clauses.push(format!(
                "(categories.id IS NULL OR categories.id IN ({}))",
                self.guardian.allowed_category_ids_sql(self.settings)?
            ));
        }
        clauses.push("topics.archetype <> 'private_message'".to_string());
        if self.options.no_definitions {
            clauses.push("COALESCE(categories.topic_id, 0) <> topics.id".to_string());
        }
        clauses.extend(self.category.clauses.iter().cloned());
        clauses.extend(self.tags.clause.iter().cloned());
        if !self.guardian.can_see_unlisted_topics() {
            clauses.push("topics.visible = TRUE".to_string());
        }
        match &self.filter {
            Filter::Top(period) => clauses.push(format!("top_topics.{period}_score > 0")),
            Filter::Unseen => {
                let col = self.highest_column();
                clauses.push(match self.user.first_seen_at {
                    Some(t) => format!("topics.bumped_at >= '{}'", sql_time(t)),
                    None => "FALSE".to_string(),
                });
                clauses.push(format!(
                    "(tu.last_read_post_number IS NULL OR tu.last_read_post_number < topics.{col})"
                ));
            }
            Filter::Posted => clauses.push("tu.posted".to_string()),
            Filter::Bookmarks => clauses.push("tu.bookmarked".to_string()),
            _ => {}
        }
        // New applies remove_muted inside its own (possibly OR'd) clause.
        if self.filter.removes_muted() && self.filter != Filter::New {
            clauses.push(self.remove_muted_clause()?);
        }
        Ok(clauses.join(" AND "))
    }

    fn highest_column(&self) -> &'static str {
        if self.user.whisperer {
            "highest_staff_post_number"
        } else {
            "highest_post_number"
        }
    }

    /// `TopicQuery.unread_filter`
    fn unread_filter(&self) -> String {
        format!(
            "(tu.last_read_post_number < topics.{}) AND (COALESCE(tu.notification_level, 1) >= 2)",
            self.highest_column()
        )
    }

    /// `TopicQuery.new_filter`
    fn new_filter(&self) -> String {
        let start = match self.user.treat_as_new_topic_start_date {
            Some(t) => format!("topics.created_at >= '{}'", sql_time(t)),
            None => "FALSE".to_string(),
        };
        format!(
            "({start}) AND (tu.last_read_post_number IS NULL) AND (COALESCE(tu.notification_level, 2) >= 2)"
        )
    }

    /// `apply_max_age_limit` without `max_age`: `topics.updated_at >=
    /// user_stats.first_unread_at`, which with no stat compares against
    /// NULL and matches nothing.
    fn max_age_clause(&self) -> String {
        match self.user.first_unread_at {
            Some(t) => format!("(topics.updated_at >= '{}')", sql_time(t)),
            None => "FALSE".to_string(),
        }
    }

    /// `remove_muted` for a user: muted topics, muted categories (unless
    /// the list is scoped to one, watched tags take precedence, or the
    /// topic is tracked/watched) and muted tags. Anonymous lists have
    /// nothing to remove (the default_* settings are refused up front).
    fn remove_muted_clause(&self) -> Result<String, TopicQueryError> {
        let Some(uid) = self.user_id() else {
            return Ok("TRUE".to_string());
        };
        let mut clauses = vec!["(COALESCE(tu.notification_level,1) > 0)".to_string()];
        let category_id = self.options.category_id.unwrap_or(-1);
        let watched = if self.user.watched_precedence_over_muted {
            format!(
                " OR EXISTS (SELECT 1 FROM topic_tags watched_topic_tags \
                 WHERE watched_topic_tags.topic_id = topics.id AND watched_topic_tags.tag_id IN \
                 (SELECT tag_id FROM tag_users WHERE user_id = {uid} AND notification_level >= 3))"
            )
        } else {
            String::new()
        };
        // indirectly_muted_category_ids: subcategories with no row of their
        // own whose parent the user muted (two nesting levels).
        let indirectly_muted = format!(
            "SELECT categories.id FROM categories \
             LEFT JOIN categories categories2 ON categories2.id = categories.parent_category_id \
             LEFT JOIN category_users ON category_users.category_id = categories.id AND category_users.user_id = {uid} \
             LEFT JOIN category_users category_users2 ON category_users2.category_id = categories2.id AND category_users2.user_id = {uid} \
             WHERE categories.parent_category_id IS NOT NULL \
             AND (category_users.id IS NULL AND COALESCE(category_users2.notification_level, 1) = 0)"
        );
        clauses.push(format!(
            "(topics.category_id = {category_id} \
             OR (COALESCE(category_users.notification_level, 1) <> 0 \
             AND (topics.category_id IS NULL OR topics.category_id NOT IN ({indirectly_muted}))){watched} \
             OR tu.notification_level > 1)"
        ));
        if let Some(muted_tags) = self.muted_tags_clause()? {
            clauses.push(muted_tags);
        }
        Ok(clauses.join(" AND "))
    }

    /// `TopicQuery.remove_muted_tags` for a user.
    fn muted_tags_clause(&self) -> Result<Option<String>, TopicQueryError> {
        if self.user.muted_tag_ids.is_empty() {
            return Ok(None);
        }
        // A list filtered by a tag the user muted shows it anyway.
        if let Some(first) = self.tags.tag_ids.first()
            && !self.options.no_tags
            && self.user.muted_tag_ids.contains(first)
        {
            return Ok(None);
        }
        let ids = self
            .user
            .muted_tag_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let watching_or_infinite = if self.user.watched_precedence_over_muted {
            3
        } else {
            99
        };
        let mode = self.settings.get("remove_muted_tags_from_latest")?.to_s();
        Ok(Some(match mode.as_str() {
            "always" => format!(
                "(NOT EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.tag_id IN ({ids}) AND tt.topic_id = topics.id \
                 AND COALESCE(category_users.notification_level, 1) < {watching_or_infinite}))"
            ),
            "only_muted" => format!(
                "(EXISTS (SELECT 1 FROM topic_tags tt WHERE (tt.tag_id NOT IN ({ids}) AND tt.topic_id = topics.id) \
                 OR COALESCE(category_users.notification_level, 1) >= {watching_or_infinite}) \
                 OR NOT EXISTS (SELECT 1 FROM topic_tags tt WHERE tt.topic_id = topics.id))"
            ),
            _ => return Ok(None),
        }))
    }

    /// The list's ORDER BY: `apply_ordering`'s column (the request's order,
    /// or the category default for ordered lists), then for top/hot the
    /// block's own order, which is the whole order when nothing was
    /// requested.
    fn order_clause(&self) -> Result<String, TopicQueryError> {
        let block = match &self.filter {
            Filter::Top(period) => Some(format!(
                "COALESCE(top_topics.{period}_score, 0) DESC, topics.bumped_at DESC"
            )),
            Filter::Hot => Some("topic_hot_scores.score DESC".to_string()),
            _ => None,
        };
        if let Some(block) = &block
            && self.options.order.is_none()
        {
            return Ok(block.clone());
        }
        let (order, ascending) = match &self.category.order {
            Some((o, a)) => (Some(o.as_str()), *a),
            None => (self.options.order.as_deref(), self.options.ascending),
        };
        let ordered = sortable_order(order, ascending, self.settings)?;
        Ok(match block {
            Some(block) => format!("{ordered}, {block}"),
            None => ordered,
        })
    }

    /// `unread_results` / `new_results` as `list_suggested_for` runs them
    /// for a logged-in user: unordered default_results, `suggested_ordering`
    /// (the topic's category first, then bumped_at), and the builder's
    /// exclusions (the topic, those already suggested, unlisted ones)
    /// applied before the limit. Unread uses `max_age` days on bumped_at.
    pub async fn suggested(
        &mut self,
        filter: Filter,
        topic_category_id: Option<i32>,
        exclude: &[i32],
        max_age_days: i64,
        per_page: i64,
    ) -> Result<Vec<TopicRow>, TopicQueryError> {
        if self.guardian.is_anonymous() {
            return Err(Unsupported("suggested unread or new for anonymous").into());
        }
        self.check_unported_filters()?;
        self.filter = filter.clone();
        self.category = self.category_scope().await?;
        self.tags = self.tag_scope().await?;
        self.user = self.user_scope().await?;
        let category_first = topic_category_id
            .map(|id| format!("CASE WHEN topics.category_id = {id} THEN 0 ELSE 1 END, "))
            .unwrap_or_default();
        let excluded = exclude
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let builder = format!("topics.id <> ALL(ARRAY[{excluded}]::int[]) AND topics.visible");
        // apply_max_age_limit: the later of first_unread_at and max_age
        // days ago.
        let max_age = crate::clock::now_naive() - chrono::Duration::days(max_age_days);
        let since = self
            .user
            .first_unread_at
            .map_or(max_age, |t| t.max(max_age));
        let unread = format!(
            "{} AND topics.bumped_at >= '{}'",
            self.unread_filter(),
            sql_time(since)
        );
        let new = format!(
            "{} AND {} AND dismissed_topic_users.id IS NULL",
            self.new_filter(),
            self.remove_muted_clause()?
        );
        let own_first = "CASE WHEN topics.user_id = tu.user_id THEN 1 ELSE 2 END, ";
        let (clause, order) = match filter {
            Filter::Unread => (
                format!("{unread} AND {builder}"),
                format!("{own_first}{category_first}topics.bumped_at DESC"),
            ),
            // new_and_unread_results, with unified new.
            Filter::New if self.user.unified_new => (
                format!("(({new}) OR ({unread})) AND {builder}"),
                format!("{own_first}{category_first}topics.bumped_at DESC"),
            ),
            Filter::New => (
                format!("{new} AND {builder}"),
                format!("{category_first}topics.bumped_at DESC"),
            ),
            _ => return Err(Unsupported("suggested topics from other lists").into()),
        };
        self.fetch(&clause, &order, per_page, 0).await
    }

    /// A deterministic stand-in for `random_suggested`: default_results
    /// (with remove_muted) of open, unarchived, visible topics not
    /// excluded, the topic's category first, then by bumped_at. Never
    /// matches Rails' random pick.
    ///
    /// The candidates are RandomTopicSelector's: its global cache is filled
    /// anonymously (public categories only), its cache for the topic's
    /// category as the system user (that category and its subcategories,
    /// restricted or not), both from topics younger than
    /// suggested_topics_max_days_old.
    pub async fn random_suggested(
        &mut self,
        topic_category_id: Option<i32>,
        exclude: &[i32],
        count: i64,
    ) -> Result<Vec<TopicRow>, TopicQueryError> {
        self.check_unported_filters()?;
        self.filter = Filter::Latest;
        self.options.no_definitions = true;
        self.category = self.category_scope().await?;
        self.tags = self.tag_scope().await?;
        self.user = self.user_scope().await?;
        let excluded = exclude
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let max_age = crate::clock::now_naive()
            - chrono::Duration::days(self.settings.get("suggested_topics_max_days_old")?.to_i());
        let category = topic_category_id.unwrap_or(-1);
        let clause = format!(
            "topics.id <> ALL(ARRAY[{excluded}]::int[]) AND topics.visible \
             AND NOT topics.closed AND NOT topics.archived \
             AND topics.created_at > '{}' \
             AND (categories.id IS NULL OR NOT categories.read_restricted \
                  OR categories.id IN (WITH RECURSIVE own(id) AS ( \
                    SELECT id FROM categories WHERE id = {category} \
                    UNION SELECT c.id FROM categories c JOIN own ON c.parent_category_id = own.id) \
                  SELECT id FROM own))",
            sql_time(max_age)
        );
        let order = format!(
            "CASE WHEN topics.category_id = {category} THEN 0 ELSE 1 END, topics.bumped_at DESC"
        );
        self.fetch(&clause, &order, count, 0).await
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
             LEFT OUTER JOIN categories ON categories.id = topics.category_id{joins} \
             WHERE {} AND ({extra_where}) ORDER BY {order} LIMIT $1 OFFSET $2",
            self.where_clause()?,
            joins = self.joins(),
        );
        Ok(sqlx::query_as(&sql)
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *self.conn)
            .await?)
    }

    /// `prioritize_pinned_topics`: pinned topics (globally, or in the
    /// category; not ones the user cleared) first, newest pin first, then
    /// the rest in list order.
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
        let mut pinned_clause = self
            .category
            .pinned_clause
            .clone()
            .unwrap_or_else(|| "pinned_globally AND pinned_at IS NOT NULL".to_string());
        if self.user_id().is_some() {
            pinned_clause.push_str(
                " AND (topics.pinned_at > tu.cleared_pinned_at OR tu.cleared_pinned_at IS NULL)",
            );
        }
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

/// `apply_ordering`'s SORTABLE_MAPPING for a requested order: the column
/// and direction, bumped_at DESC by default.
pub fn sortable_order(
    order: Option<&str>,
    ascending: bool,
    settings: &SiteSettings,
) -> Result<String, TopicQueryError> {
    let column = match order {
        None | Some("default") | Some("activity") => "topics.bumped_at".to_string(),
        Some("likes") => "topics.like_count".to_string(),
        Some("op_likes") => {
            "(SELECT like_count FROM posts p3 WHERE p3.topic_id = topics.id AND p3.post_number = 1)"
                .to_string()
        }
        Some("views") => "topics.views".to_string(),
        Some("posts") => "topics.posts_count".to_string(),
        Some("posters") => "topics.participant_count".to_string(),
        Some("created") => "topics.created_at".to_string(),
        Some("category") => {
            let uncategorized = settings.get("uncategorized_category_id")?.to_i();
            format!("CASE WHEN categories.id = {uncategorized} THEN '' ELSE categories.name END")
        }
        Some(_) => return Err(Unsupported("unknown topic list order").into()),
    };
    let dir = if ascending { "ASC" } else { "DESC" };
    Ok(format!("{column} {dir}"))
}
/// A timestamp as a Postgres literal (UTC, microseconds).
fn sql_time(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%d %H:%M:%S%.6f").to_string()
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
            highest_staff_post_number: 0,
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
            subtype: None,
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
