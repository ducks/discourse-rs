//! Port of lib/search.rb, lib/search/grouped_search_results.rb and
//! app/models/search_log.rb: words, quoted phrases and the advanced
//! filters and keywords ported so far (crate::search::advanced); the
//! others are refused.

pub mod advanced;
pub mod blurb;
mod dates;
mod filters;
mod posts_query;
mod uri;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;
use unicode_normalization::UnicodeNormalization;

use crate::Unsupported;
use crate::avatar::{self, AvatarError};
use crate::categories::{Categories, CategoriesError};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::tags::{Tag, visible_tags_where};
use crate::topic_list::{Mode, TopicListError, TopicListSerializer, time_json};
use crate::topic_query::{TOPIC_COLUMNS, TopicRow};
use crate::url::Urls;

/// `Search.per_facet`
const PER_FACET: usize = 5;
/// `SearchLog.search_types`
pub const SEARCH_TYPE_HEADER: i32 = 1;
pub const SEARCH_TYPE_FULL_PAGE: i32 = 2;
/// `Search::GroupedSearchResults::BLURB_LENGTH`
pub const BLURB_LENGTH: usize = 200;

#[derive(Debug)]
pub enum SearchError {
    Db(sqlx::Error),
    Setting(SettingError),
    Unsupported(Unsupported),
    Categories(CategoriesError),
    TopicList(TopicListError),
    Avatar(AvatarError),
    /// `Discourse::InvalidAccess`
    InvalidAccess,
    Url(crate::url::UrlError),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::InvalidAccess => write!(f, "invalid access"),
            SearchError::Url(e) => e.fmt(f),
            SearchError::Db(e) => write!(f, "database: {e}"),
            SearchError::Setting(e) => e.fmt(f),
            SearchError::Unsupported(e) => e.fmt(f),
            SearchError::Categories(e) => e.fmt(f),
            SearchError::TopicList(e) => e.fmt(f),
            SearchError::Avatar(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<sqlx::Error> for SearchError {
    fn from(e: sqlx::Error) -> Self {
        SearchError::Db(e)
    }
}

impl From<SettingError> for SearchError {
    fn from(e: SettingError) -> Self {
        SearchError::Setting(e)
    }
}

impl From<crate::guardian::GuardianError> for SearchError {
    fn from(e: crate::guardian::GuardianError) -> Self {
        match e {
            crate::guardian::GuardianError::Db(e) => SearchError::Db(e),
            crate::guardian::GuardianError::Setting(e) => SearchError::Setting(e),
            crate::guardian::GuardianError::Unsupported(e) => SearchError::Unsupported(e),
        }
    }
}

impl From<Unsupported> for SearchError {
    fn from(e: Unsupported) -> Self {
        SearchError::Unsupported(e)
    }
}

impl From<CategoriesError> for SearchError {
    fn from(e: CategoriesError) -> Self {
        SearchError::Categories(e)
    }
}

impl From<TopicListError> for SearchError {
    fn from(e: TopicListError) -> Self {
        SearchError::TopicList(e)
    }
}

impl From<crate::url::UrlError> for SearchError {
    fn from(e: crate::url::UrlError) -> Self {
        SearchError::Url(e)
    }
}

impl From<AvatarError> for SearchError {
    fn from(e: AvatarError) -> Self {
        SearchError::Avatar(e)
    }
}

/// `type_filter`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeFilter {
    Topic,
    Category,
    User,
    Tags,
    ExcludeTopics,
    PrivateMessages,
    AllTopics,
}

/// The `Search.new` options the controllers pass.
pub struct SearchArgs {
    pub term: String,
    pub type_filter: Option<TypeFilter>,
    /// `search_type: :full_page` (the /search page) vs `:header`.
    pub full_page: bool,
    /// 1-based, full page only.
    pub page: i64,
    pub blurb_length: usize,
    pub ip_address: String,
    pub user_agent: Option<String>,
    pub session_id: Option<String>,
    /// A `search_type` was given: `log_query?` may log it (search_logs#term
    /// passes none).
    pub log: bool,
    /// The `search_context` the controller found.
    pub context: Option<SearchContext>,
    /// `search_for_id`: a topic id or URL as the term finds that topic.
    pub search_for_id: bool,
    /// `restrict_to_archetype` for search_for_id's topic.
    pub restrict_to_archetype: Option<String>,
}

/// `@search_context`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchContext {
    /// A topic (the search menu's "in this topic"): its posts matching the
    /// term, in post order.
    Topic(TopicContext),
    /// A user: their posts, or with private messages, their messages.
    User(i32),
    /// A category: its topics and its subcategories' (one level).
    Category(i32),
    /// A tag: its topics.
    Tag(i32),
}

impl SearchContext {
    /// The record's id, whatever its kind.
    pub fn id(&self) -> i32 {
        match self {
            SearchContext::Topic(t) => t.id,
            SearchContext::User(id) | SearchContext::Category(id) | SearchContext::Tag(id) => *id,
        }
    }
}

/// A topic the search is scoped to (`Topic === @search_context`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopicContext {
    pub id: i32,
    pub private_message: bool,
}

/// `SearchLog.log`'s redis dedupe key, `__SEARCH__LOG_<ip>` with a 5 s
/// TTL, kept in the process.
#[derive(Default)]
pub struct SearchLogCache {
    entries: Mutex<HashMap<String, (i64, String, Instant)>>,
}

const SEARCH_LOG_TTL: Duration = Duration::from_secs(5);

pub struct Search<'a> {
    pub conn: &'a mut PgConnection,
    pub settings: &'a SiteSettings,
    pub i18n: &'a I18n,
    pub guardian: &'a Guardian,
    pub urls: &'a Urls<'a>,
    pub base_path: &'a str,
    pub log_cache: &'a SearchLogCache,
}

/// `Search.clean_term`: zero-width characters out, curly quotes and fancy
/// apostrophes straightened.
pub fn clean_term(term: &str) -> String {
    term.chars()
        .filter(|c| !matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}'))
        .map(|c| match c {
            '\u{201c}' | '\u{201d}' => '"',
            '\u{2b9}' | '\u{2bb}' | '\u{2bc}' | '\u{2bd}' | '\u{2c8}' | '\u{2018}' | '\u{2019}'
            | '\u{201b}' | '\u{2032}' | '\u{ff07}' => '\'',
            c => c,
        })
        .collect()
}

/// Ruby `String#squish`: stripped, whitespace runs one space.
pub fn squish(s: &str) -> String {
    s.split(char::is_whitespace)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `filter_short_terms`: words shorter than min_search_term_length go,
/// quoted phrases stay; the split ignores whitespace inside quotes.
pub fn filter_short_terms(term: &str, min_length: usize) -> String {
    let chars: Vec<char> = term.chars().collect();
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let quotes_after = chars[i + 1..].iter().filter(|&&q| q == '"').count();
        if c.is_whitespace() && quotes_after % 2 == 0 {
            pieces.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    pieces.push(current);
    pieces
        .into_iter()
        .filter(|p| p.starts_with('"') || p.chars().count() >= min_length)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `Search.escape_string`: quotes and backslashes doubled.
fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

/// The literal handed to TO_TSQUERY after Rails' two rounds of escaping
/// and the SQL quoting collapse: `'<term>':*<weights>`.
pub(crate) fn ts_query_value(term: &str, weights: &str, prefix: bool) -> String {
    let inner = format!(
        "'{}':{}{weights}",
        escape_string(term),
        if prefix { "*" } else { "" }
    );
    // The second escape_string doubles the backslashes again; the quotes
    // it doubles are undone by the SQL literal.
    inner.replace('\\', "\\\\")
}

/// `Search.ts_query`'s SQL around a bound value.
pub(crate) fn ts_query_sql(config: &str, bind: usize) -> String {
    format!(
        "REGEXP_REPLACE(TO_TSQUERY('{config}', ${bind})::text, '<->|<\\d+>', '&', 'g')::tsquery"
    )
}

/// `Search.ts_config`
pub fn ts_config(locale: &str) -> &'static str {
    match locale.split('_').next().unwrap_or("") {
        "da" => "danish",
        "nl" => "dutch",
        "en" => "english",
        "fi" => "finnish",
        "fr" => "french",
        "de" => "german",
        "hu" => "hungarian",
        "it" => "italian",
        "nb" => "norwegian",
        "pt" => "portuguese",
        "ro" => "romanian",
        "ru" => "russian",
        "es" => "spanish",
        "sv" => "swedish",
        "tr" => "turkish",
        _ => "simple",
    }
}

/// Ruby `Float#to_s` for the weights the ORDER BY interpolates.
fn ruby_float(f: f64) -> String {
    if f.fract() == 0.0 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

#[derive(sqlx::FromRow)]
struct PostHit {
    id: i32,
    user_id: Option<i32>,
    topic_id: i32,
    post_number: i32,
    created_at: NaiveDateTime,
    like_count: i32,
    raw_data: Option<String>,
    version: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct UserHit {
    id: i32,
    username: String,
    name: Option<String>,
    uploaded_avatar_id: Option<i32>,
}

use crate::groups::BasicGroup;

/// `Search::GroupedSearchResults` before serialization.
#[derive(Default)]
struct Results {
    posts: Vec<PostHit>,
    users: Vec<UserHit>,
    category_ids: Vec<i32>,
    tags: Vec<Tag>,
    groups: Vec<BasicGroup>,
    more_posts: bool,
    more_users: bool,
    more_categories: bool,
    more_full_page_results: bool,
    search_log_id: Option<i64>,
}

impl Search<'_> {
    /// `Search.new(term, args).execute` serialized through
    /// GroupedSearchResultSerializer; `None` is the nil result
    /// (`{"grouped_search_result":null}`).
    /// `Search.new(term, args).execute` serialized through
    /// GroupedSearchResultSerializer; `None` is the nil result
    /// (`{"grouped_search_result":null}`).
    pub async fn execute(&mut self, args: &SearchArgs) -> Result<Option<Value>, SearchError> {
        self.check_unported()?;
        let clean = clean_term(&args.term);
        let adv = advanced::process(&clean, self.settings, self.guardian)?;
        let mut context = args.context;
        let mut search_pms = adv.search_pms;
        for keyword in &adv.context_keywords {
            match keyword {
                advanced::ContextKeyword::Topic(id) => {
                    if let Some(topic) = self.visible_topic(*id).await? {
                        context = Some(SearchContext::Topic(topic));
                    }
                }
                advanced::ContextKeyword::PersonalMessages(name) => {
                    // User.find_by_username
                    let user_id: Option<i32> = sqlx::query_scalar(
                        "SELECT id FROM users WHERE username_lower = $1 LIMIT 1",
                    )
                    .bind(name.nfc().collect::<String>().to_lowercase())
                    .fetch_optional(&mut *self.conn)
                    .await?;
                    if let Some(id) = user_id {
                        search_pms = true;
                        context = Some(SearchContext::User(id));
                    }
                }
            }
        }
        let mut type_filter = args.type_filter;
        if search_pms || adv.search_all_pms || type_filter == Some(TypeFilter::PrivateMessages) {
            type_filter = Some(TypeFilter::PrivateMessages);
            if context.is_none() {
                context = self.guardian.user_id().map(SearchContext::User);
            }
            // can_see_private_messages?(@search_context.id)
            let allowed = context.is_some_and(|c| {
                self.guardian.is_admin() || self.guardian.user_id() == Some(c.id())
            });
            if !allowed {
                return Err(SearchError::InvalidAccess);
            }
        }
        if adv.search_all_topics && self.guardian.user_id().is_some() {
            type_filter = Some(TypeFilter::AllTopics);
        }
        let topic_context = match context {
            Some(SearchContext::Topic(t)) => Some(t),
            _ => None,
        };
        // prepare_data: squished (but in a topic), and URLs lose their
        // query string.
        let mut term = uri::strip_url_queries(&match topic_context {
            Some(_) => adv.term.clone(),
            None => squish(&adv.term),
        });
        // blurb_term is the term before prepare_data, blank or not.
        let blurb_term = adv.term.clone();
        let original_term = (!term.is_empty()).then(|| term.replace('\\', "\\\\"));

        let mut results = Results::default();
        // log_query?: not while the site is read only.
        if args.log
            && self.settings.get("log_search_queries")?.truthy()
            && args.type_filter != Some(TypeFilter::ExcludeTopics)
            && !crate::readonly::mode(&mut *self.conn)
                .await?
                .site_readonly()
        {
            results.search_log_id = self.log(&clean, args).await?;
        }

        // A topic's search keeps its short words (and searches the posts
        // even without any).
        if !args.search_for_id && topic_context.is_none() {
            let min_length = self.settings.get("min_search_term_length")?.to_i().max(0) as usize;
            term = filter_short_terms(&term, min_length);
            if term.trim().is_empty() && adv.filters.is_empty() && adv.order.is_none() {
                return Ok(None);
            }
        }

        // use_full_page_limit: the full page and a topic's search.
        let full_page_limit = args.full_page || topic_context.is_some();
        let limit = if full_page_limit {
            self.settings.get("search_page_size")?.to_i() + 1
        } else {
            PER_FACET as i64 + 1
        };
        let per_filter = self.settings.get("search_page_size")?.to_i().max(1) as usize;
        let offset = if args.full_page && type_filter.is_some() {
            (args.page - 1) * per_filter as i64
        } else {
            0
        };

        let mut run = posts_query::Run {
            adv,
            term,
            context,
            type_filter,
            category_filter_matched: false,
            limit,
            offset,
        };
        // search_for_id: a topic's id or URL finds that topic.
        if args.search_for_id
            && matches!(
                type_filter,
                Some(TypeFilter::Topic | TypeFilter::PrivateMessages | TypeFilter::AllTopics)
            )
        {
            let term = run.term.clone();
            let id = if !term.is_empty() && term.bytes().all(|b| b.is_ascii_digit()) {
                Some(crate::ruby::to_i(&term))
            } else {
                let hostname = self.urls.current_hostname()?;
                uri::topic_id_for(&term, &hostname, self.base_path).filter(|id| *id > 0)
            };
            if let Some(id) = id {
                self.single_topic(id, args.restrict_to_archetype.as_deref(), &mut results)
                    .await?;
            }
        }
        // find_grouped_results rescues ActiveRecord::StatementInvalid: a
        // query Postgres rejects ends the search with what it found so far.
        if results.posts.is_empty() {
            match self
                .find_grouped_results(
                    type_filter,
                    &mut run,
                    original_term.as_deref(),
                    &mut results,
                )
                .await
            {
                Err(SearchError::Db(sqlx::Error::Database(e))) => {
                    tracing::warn!("search query failed: {e}");
                }
                other => other?,
            }
        }
        // add()'s limit trick: the extra row flips the flag.
        let cap = if full_page_limit {
            per_filter
        } else {
            PER_FACET
        };
        if results.posts.len() > cap {
            results.posts.truncate(cap);
            if full_page_limit {
                results.more_full_page_results = true;
            } else {
                results.more_posts = true;
            }
        }
        if results.users.len() > PER_FACET {
            results.users.truncate(PER_FACET);
            results.more_users = true;
        }
        if results.category_ids.len() > PER_FACET {
            results.category_ids.truncate(PER_FACET);
            results.more_categories = true;
        }
        results.tags.truncate(PER_FACET);
        results.groups.truncate(PER_FACET);

        Ok(Some(
            self.serialize(
                results,
                &clean,
                Some(&blurb_term),
                args.blurb_length,
                original_term.as_deref().unwrap_or(""),
                args.full_page,
            )
            .await?,
        ))
    }

    /// `single_topic`: the topic's first post, of the archetype asked for,
    /// when the viewer can see it.
    async fn single_topic(
        &mut self,
        id: i64,
        restrict_to_archetype: Option<&str>,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let Ok(id) = i32::try_from(id) else {
            return Ok(());
        };
        let post_id: Option<i32> = match restrict_to_archetype.filter(|a| !a.is_empty()) {
            Some(archetype) => {
                let archetype = if archetype == "regular" {
                    "regular"
                } else {
                    "private_message"
                };
                sqlx::query_scalar(
                    "SELECT posts.id FROM posts \
                     INNER JOIN topics ON topics.deleted_at IS NULL AND topics.id = posts.topic_id \
                     WHERE posts.deleted_at IS NULL AND topics.id = $1 AND topics.archetype = $2 \
                     AND posts.post_number = 1 LIMIT 1",
                )
                .bind(id)
                .bind(archetype)
                .fetch_optional(&mut *self.conn)
                .await?
            }
            None => {
                sqlx::query_scalar(
                    "SELECT id FROM posts WHERE posts.deleted_at IS NULL \
                     AND posts.topic_id = $1 AND posts.post_number = 1 LIMIT 1",
                )
                .bind(id)
                .fetch_optional(&mut *self.conn)
                .await?
            }
        };
        let Some(post_id) = post_id else {
            return Ok(());
        };
        let Some(post) = crate::topic_guardian::PostCtx::load(&mut *self.conn, post_id).await?
        else {
            return Ok(());
        };
        let topic_can_see = !self
            .guardian
            .can_see_topic_ids(&mut *self.conn, self.settings, &[id])
            .await?
            .is_empty();
        if !self
            .guardian
            .can_see_post(self.settings, &post, topic_can_see)?
        {
            return Ok(());
        }
        results.posts = sqlx::query_as(
            "SELECT posts.id, posts.user_id, posts.topic_id, posts.post_number, posts.created_at, \
                    posts.like_count, psd.raw_data, psd.version \
             FROM posts LEFT JOIN post_search_data psd ON psd.post_id = posts.id WHERE posts.id = $1",
        )
        .bind(post_id)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// `Topic.find_by(id:)` when the guardian can see it.
    pub async fn visible_topic(&mut self, id: i64) -> Result<Option<TopicContext>, SearchError> {
        let Ok(id) = i32::try_from(id) else {
            return Ok(None);
        };
        let archetype: Option<String> =
            sqlx::query_scalar("SELECT archetype FROM topics WHERE id = $1 AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&mut *self.conn)
                .await?;
        let Some(archetype) = archetype else {
            return Ok(None);
        };
        if self
            .guardian
            .can_see_topic_ids(&mut *self.conn, self.settings, &[id])
            .await?
            .is_empty()
        {
            return Ok(None);
        }
        Ok(Some(TopicContext {
            id,
            private_message: archetype == "private_message",
        }))
    }

    /// `find_grouped_results`
    async fn find_grouped_results(
        &mut self,
        type_filter: Option<TypeFilter>,
        run: &mut posts_query::Run,
        original_term: Option<&str>,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let term = run.term.clone();
        let present = !term.trim().is_empty();
        match type_filter {
            Some(TypeFilter::User) => {
                // Rails reads the nil @original_term of a term that was
                // only filters, and raises.
                let Some(original) = original_term else {
                    return Err(Unsupported("type_filter=user with a term of only filters").into());
                };
                self.user_search(&term, original, results).await?
            }
            Some(TypeFilter::Category) => self.category_search(&term, results).await?,
            Some(TypeFilter::Tags) => self.tags_search(&term, results).await?,
            Some(TypeFilter::ExcludeTopics) => {
                if present {
                    self.user_search(&term, original_term.unwrap_or(""), results)
                        .await?;
                    self.category_search(&term, results).await?;
                    self.tags_search(&term, results).await?;
                    self.groups_search(&term, results).await?;
                }
            }
            Some(TypeFilter::Topic) => self.topic_search(run, results).await?,
            Some(TypeFilter::PrivateMessages) => {
                if self.guardian.user_id().is_none() {
                    return Err(SearchError::InvalidAccess);
                }
                self.aggregate_search(run, results).await?
            }
            Some(TypeFilter::AllTopics) => {
                // private_posts_for_user(nil) raises in Rails.
                if self.guardian.user_id().is_none() {
                    return Err(
                        Unsupported("type_filter=all_topics for an anonymous search").into(),
                    );
                }
                self.aggregate_search(run, results).await?
            }
            None => {
                if present && run.context.is_none() {
                    self.user_search(&term, original_term.unwrap_or(""), results)
                        .await?;
                    self.category_search(&term, results).await?;
                    self.tags_search(&term, results).await?;
                    self.groups_search(&term, results).await?;
                }
                self.topic_search(run, results).await?;
            }
        }
        Ok(())
    }

    /// Settings that change the queries in ways not ported.
    fn check_unported(&self) -> Result<(), SearchError> {
        let s = self.settings;
        if s.get("search_ignore_accents")?.truthy() {
            return Err(Unsupported("search_ignore_accents (unaccent)").into());
        }
        if s.get("use_pg_headlines_for_excerpt")?.truthy() {
            return Err(Unsupported("use_pg_headlines_for_excerpt").into());
        }
        if s.get("search_ranking_weights")?.presence().is_some() {
            return Err(Unsupported("search_ranking_weights").into());
        }
        if s.get("search_prefer_recent_posts")?.truthy() {
            return Err(Unsupported("search_prefer_recent_posts").into());
        }
        if s.get("search_tokenize_chinese")?.truthy() || s.get("search_tokenize_japanese")?.truthy()
        {
            return Err(Unsupported("CJK search tokenization").into());
        }
        let locale = s.get("default_locale")?.to_s();
        if locale.starts_with("zh") || locale.starts_with("ja") {
            return Err(Unsupported("CJK search tokenization").into());
        }
        if s.get("content_localization_enabled")?.truthy() {
            return Err(Unsupported("content_localization_enabled").into());
        }
        // GroupedSearchResultSerializer#extra's categories.
        if self.guardian.can_lazy_load_categories(s)? {
            return Err(Unsupported("search results for lazy-loaded categories (extra)").into());
        }
        Ok(())
    }

    fn config(&self) -> Result<&'static str, SearchError> {
        Ok(ts_config(&self.settings.get("default_locale")?.to_s()))
    }

    /// `user_search`: active, unstaged, unsuspended users matching in the
    /// simple config, the exact username first, then by last post.
    async fn user_search(
        &mut self,
        term: &str,
        original_term: &str,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        if self.guardian.is_anonymous()
            && self
                .settings
                .get("hide_user_profiles_from_public")?
                .truthy()
        {
            return Ok(());
        }
        let weight = if self.settings.get("enable_names")?.truthy() {
            ""
        } else {
            "AC"
        };
        let mut sql = format!(
            "SELECT users.id, users.username, users.name, users.uploaded_avatar_id FROM users \
             LEFT OUTER JOIN user_search_data ON user_search_data.user_id = users.id \
             WHERE users.active = TRUE AND users.staged = FALSE \
             AND (user_search_data.search_data @@ {})",
            ts_query_sql("simple", 1)
        );
        if !self.guardian.is_admin()
            && !self
                .settings
                .get("enable_listing_suspended_users_on_search")?
                .truthy()
        {
            sql.push_str(" AND users.suspended_at IS NULL");
        }
        sql.push_str(
            " ORDER BY CASE WHEN username_lower = $2 THEN 0 ELSE 1 END, last_posted_at DESC LIMIT $3",
        );
        results.users = sqlx::query_as(&sql)
            .bind(ts_query_value(term, weight, true))
            .bind(original_term.to_lowercase())
            .bind(PER_FACET as i64 + 1)
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(())
    }

    /// `Category.secured(guardian)`: public categories, plus the secure
    /// ones when the guardian has any.
    async fn category_secured_clause(&mut self) -> Result<String, SearchError> {
        let secure_ids = self
            .guardian
            .secure_category_ids(&mut *self.conn, self.settings)
            .await?;
        Ok(if secure_ids.is_empty() {
            "NOT categories.read_restricted".to_string()
        } else {
            format!(
                "NOT categories.read_restricted OR categories.id IN ({})",
                secure_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    /// The viewer's `(member, owner)` row for a group, if any.
    async fn group_membership(
        &mut self,
        group_id: i32,
    ) -> Result<Option<(bool, bool)>, SearchError> {
        let Some(uid) = self.guardian.user_id() else {
            return Ok(None);
        };
        let owner: Option<bool> = sqlx::query_scalar(
            "SELECT owner FROM group_users WHERE group_id = $1 AND user_id = $2",
        )
        .bind(group_id)
        .bind(uid)
        .fetch_optional(&mut *self.conn)
        .await?;
        Ok(owner.map(|o| (true, o)))
    }

    /// `category_search`: readable categories by their search data, busiest
    /// this month first.
    async fn category_search(
        &mut self,
        term: &str,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let config = self.config()?;
        let secured = self.category_secured_clause().await?;
        let sql = format!(
            "SELECT categories.id FROM categories \
             LEFT OUTER JOIN category_search_data ON category_search_data.category_id = categories.id \
             WHERE (category_search_data.search_data @@ {}) AND ({secured}) \
             ORDER BY topics_month DESC LIMIT $2",
            ts_query_sql(config, 1)
        );
        results.category_ids = sqlx::query_scalar(&sql)
            .bind(ts_query_value(term, "", true))
            .bind(PER_FACET as i64 + 1)
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(())
    }

    /// `tags_search`: browsable (base, visible) tags by name.
    async fn tags_search(&mut self, term: &str, results: &mut Results) -> Result<(), SearchError> {
        if !self.settings.get("tagging_enabled")?.truthy() {
            return Ok(());
        }
        let config = self.config()?;
        let visible = visible_tags_where(self.guardian, self.settings)?;
        let sql = format!(
            "SELECT tags.id, tags.name, tags.slug, tags.description, tags.description_cooked, \
                    tags.public_topic_count, tags.staff_topic_count, tags.pm_topic_count, tags.target_tag_id FROM tags \
             LEFT OUTER JOIN tag_search_data ON tag_search_data.tag_id = tags.id \
             WHERE tags.target_tag_id IS NULL AND {visible} \
             AND (tag_search_data.search_data @@ {}) ORDER BY name asc LIMIT $2",
            ts_query_sql(config, 1)
        );
        results.tags = sqlx::query_as(&sql)
            .bind(ts_query_value(term, "", true))
            .bind(PER_FACET as i64 + 1)
            .fetch_all(&mut *self.conn)
            .await?;
        Ok(())
    }

    /// `groups_search`: public groups whose name or full name contains the
    /// term.
    async fn groups_search(
        &mut self,
        term: &str,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let visible = crate::groups::visible_groups_where(self.guardian, "g");
        results.groups = sqlx::query_as(&format!(
            "SELECT {} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
             WHERE (g.id > 0) AND (g.id NOT IN (4, 5)) AND ({visible}) \
             AND (g.name ILIKE $1 OR g.full_name ILIKE $1) ORDER BY g.name ASC LIMIT $2",
            crate::groups::BASIC_GROUP_COLUMNS
        ))
        .bind(format!("%{term}%"))
        .bind(PER_FACET as i64 + 1)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// `SearchLog.log`: a row per search, or the previous row from the same
    /// IP or user within 5 s updated when the new term extends it.
    async fn log(&mut self, term: &str, args: &SearchArgs) -> Result<Option<i64>, SearchError> {
        if term.trim().is_empty() || args.ip_address.is_empty() {
            return Ok(None);
        }
        // Logged-in rows carry the user, not the address, and dedupe by user.
        let user_id = self.guardian.user_id();
        let key = match user_id {
            Some(id) => format!("user:{id}"),
            None => args.ip_address.clone(),
        };
        let search_type = if args.full_page {
            SEARCH_TYPE_FULL_PAGE
        } else {
            SEARCH_TYPE_HEADER
        };
        let previous = {
            let entries = self
                .log_cache
                .entries
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            entries
                .get(&key)
                .filter(|(_, _, at)| at.elapsed() < SEARCH_LOG_TTL)
                .map(|(id, old_term, _)| (*id, old_term.clone()))
        };
        let id = match previous {
            Some((id, old_term)) if term.starts_with(old_term.as_str()) => {
                sqlx::query(
                    "UPDATE search_logs SET created_at = now(), term = $1 WHERE id = $2::int",
                )
                .bind(term)
                .bind(id)
                .execute(&mut *self.conn)
                .await?;
                id
            }
            _ => {
                let user_agent: Option<String> = args
                    .user_agent
                    .as_deref()
                    .map(|ua| ua.chars().take(2000).collect());
                let session_id: Option<String> = args
                    .session_id
                    .as_deref()
                    .map(|s| s.chars().take(32).collect());
                // `user_agent.present? && CrawlerDetection.crawler?(user_agent)`
                let crawler = match user_agent.as_deref().filter(|u| !u.trim().is_empty()) {
                    Some(ua) => self.crawler(Some(ua))?,
                    None => false,
                };
                sqlx::query_scalar(
                    "INSERT INTO search_logs (term, search_type, ip_address, user_agent, user_id, session_id, \
                                              crawler, likely_crawler, created_at) \
                     VALUES ($1, $2, $3::inet, $4, $7, $5, $6, FALSE, now()) RETURNING id::bigint",
                )
                .bind(term)
                .bind(search_type)
                .bind(user_id.map(|_| None::<String>).unwrap_or(Some(args.ip_address.clone())))
                .bind(user_agent)
                .bind(session_id)
                .bind(crawler)
                .bind(user_id)
                .fetch_one(&mut *self.conn)
                .await?
            }
        };
        let mut entries = self
            .log_cache
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        entries.insert(key, (id, term.to_string(), Instant::now()));
        Ok(Some(id))
    }

    /// `CrawlerDetection.crawler?`
    fn crawler(&self, user_agent: Option<&str>) -> Result<bool, SearchError> {
        Ok(crate::crawler_detection::crawler(
            self.settings,
            user_agent,
            None,
        )?)
    }

    /// GroupedSearchResultSerializer with its side-loaded associations, in
    /// wire order: posts, topics (when any), users, categories, tags,
    /// groups, grouped_search_result.
    async fn serialize(
        &mut self,
        results: Results,
        term: &str,
        blurb_term: Option<&str>,
        blurb_length: usize,
        original_term: &str,
        full_page: bool,
    ) -> Result<Value, SearchError> {
        let tagging = self.settings.get("tagging_enabled")?.truthy();
        let enable_names = self.settings.get("enable_names")?.truthy();
        let mut out = Map::new();

        // posts, then the topics they side-load (deduplicated, in order).
        let user_ids: Vec<i32> = results.posts.iter().filter_map(|p| p.user_id).collect();
        let mut list = TopicListSerializer {
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
        let users = list.user_lookup_for(&user_ids).await?;
        let logo_small_url = list.logo_small_url().await?;
        let mut posts = Vec::new();
        let mut topic_ids: Vec<i32> = Vec::new();
        for p in &results.posts {
            let user = p.user_id.and_then(|id| users.get(&id));
            let mut post = Map::new();
            post.insert("id".into(), json!(p.id));
            if enable_names {
                post.insert("name".into(), json!(user.and_then(|u| u.name.clone())));
            }
            post.insert("username".into(), json!(user.map(|u| u.username.clone())));
            post.insert(
                "avatar_template".into(),
                match user {
                    Some(u) => json!(avatar::avatar_template(
                        self.urls,
                        u.id,
                        &u.username,
                        u.uploaded_avatar_id,
                        logo_small_url.as_deref()
                    )?),
                    None => Value::Null,
                },
            );
            post.insert("created_at".into(), json!(time_json(p.created_at)));
            post.insert("like_count".into(), json!(p.like_count));
            let (Some(raw_data), Some(version)) = (&p.raw_data, p.version) else {
                return Err(Unsupported(
                    "blurbs from cooked posts (post_search_data without raw_data)",
                )
                .into());
            };
            if version < 4 {
                return Err(
                    Unsupported("blurbs from cooked posts (post_search_data version < 4)").into(),
                );
            }
            post.insert(
                "blurb".into(),
                json!(blurb::blurb_for(raw_data, blurb_term, blurb_length)),
            );
            post.insert("post_number".into(), json!(p.post_number));
            post.insert("topic_id".into(), json!(p.topic_id));
            posts.push(Value::Object(post));
            if !topic_ids.contains(&p.topic_id) {
                topic_ids.push(p.topic_id);
            }
        }
        out.insert("posts".into(), Value::Array(posts));
        if !topic_ids.is_empty() {
            let rows: Vec<TopicRow> = sqlx::query_as(&format!(
                "SELECT {TOPIC_COLUMNS} FROM topics WHERE topics.id = ANY($1)"
            ))
            .bind(&topic_ids)
            .fetch_all(&mut *list.conn)
            .await?;
            list.prefetch(&rows).await?;
            let mut topics = Vec::new();
            for id in &topic_ids {
                let Some(t) = rows.iter().find(|t| t.id == *id) else {
                    continue;
                };
                topics.push(
                    list.serialize_topic(
                        t,
                        &[],
                        tagging,
                        Mode::SearchItem {
                            user_data: full_page,
                        },
                    )
                    .await?,
                );
            }
            out.insert("topics".into(), Value::Array(topics));
        }

        // users: SearchResultUserSerializer with searchable custom fields.
        let mut users_json = Vec::new();
        if !results.users.is_empty() {
            let ids: Vec<i32> = results.users.iter().map(|u| u.id).collect();
            let custom: Vec<(i32, String, String)> = sqlx::query_as(
                "SELECT user_custom_fields.user_id, user_fields.name, user_custom_fields.value \
                 FROM user_custom_fields \
                 INNER JOIN user_fields ON user_fields.id = REPLACE(user_custom_fields.name, 'user_field_', '')::INTEGER \
                   AND user_fields.searchable IS TRUE \
                 WHERE user_id = ANY($1) AND user_custom_fields.name LIKE 'user_field_%' \
                   AND user_custom_fields.value ILIKE $2",
            )
            .bind(&ids)
            .bind(format!("%{}%", original_term.to_lowercase()))
            .fetch_all(&mut *self.conn)
            .await?;
            for u in &results.users {
                let mut user = Map::new();
                user.insert("id".into(), json!(u.id));
                user.insert("username".into(), json!(u.username));
                if enable_names {
                    user.insert("name".into(), json!(u.name));
                }
                user.insert(
                    "avatar_template".into(),
                    json!(avatar::avatar_template(
                        self.urls,
                        u.id,
                        &u.username,
                        u.uploaded_avatar_id,
                        logo_small_url.as_deref()
                    )?),
                );
                let data: Vec<Value> = custom
                    .iter()
                    .filter(|(id, _, _)| *id == u.id)
                    .map(|(_, name, value)| json!({"name": name, "value": value}))
                    .collect();
                user.insert("custom_data".into(), Value::Array(data));
                users_json.push(Value::Object(user));
            }
        }
        out.insert("users".into(), Value::Array(users_json));

        // categories: BasicCategorySerializer
        let mut categories_json = Vec::new();
        if !results.category_ids.is_empty() {
            let rows = Categories::load_all(&mut *self.conn).await?;
            let mut cats = Categories {
                conn: &mut *self.conn,
                settings: self.settings,
                i18n: self.i18n,
                guardian: self.guardian,
                base_path: self.base_path,
                topic_url_via_slug: false,
            };
            for id in &results.category_ids {
                let Some(row) = rows.iter().find(|r| r.id == *id) else {
                    continue;
                };
                let mut c = cats.basic_fields(row).await?;
                c.insert("notification_level".into(), Value::Null);
                // can_edit?(category): admins, or moderators who manage categories.
                if self.guardian.is_admin()
                    || (self.settings.get("moderators_manage_categories")?.truthy()
                        && self.guardian.is_moderator())
                {
                    c.insert("can_edit".into(), json!(true));
                }
                c.insert("has_children".into(), Value::Null);
                c.insert("subcategory_count".into(), Value::Null);
                cats.uploads(&mut c, row).await?;
                categories_json.push(Value::Object(c));
            }
        }
        out.insert("categories".into(), Value::Array(categories_json));

        if tagging {
            let mut tags = Vec::new();
            for t in &results.tags {
                tags.push(
                    t.serialize(&mut *self.conn, self.guardian, self.settings)
                        .await?,
                );
            }
            out.insert("tags".into(), Value::Array(tags));
        }

        let mut groups = Vec::new();
        for g in &results.groups {
            let membership = self.group_membership(g.id).await?;
            groups.push(g.json(self.i18n, self.guardian, self.settings, membership)?);
        }
        out.insert("groups".into(), Value::Array(groups));

        let mut grouped = Map::new();
        grouped.insert(
            "more_posts".into(),
            json!(results.more_posts.then_some(true)),
        );
        grouped.insert(
            "more_users".into(),
            json!(results.more_users.then_some(true)),
        );
        grouped.insert(
            "more_categories".into(),
            json!(results.more_categories.then_some(true)),
        );
        grouped.insert("term".into(), json!(term));
        if let Some(id) = results.search_log_id {
            grouped.insert("search_log_id".into(), json!(id));
        }
        grouped.insert(
            "more_full_page_results".into(),
            json!(results.more_full_page_results.then_some(true)),
        );
        grouped.insert(
            "can_create_topic".into(),
            json!(
                self.guardian
                    .can_create_topic(&mut *self.conn, self.settings)
                    .await?
            ),
        );
        grouped.insert("error".into(), Value::Null);
        grouped.insert("extra".into(), json!({}));
        grouped.insert(
            "post_ids".into(),
            json!(results.posts.iter().map(|p| p.id).collect::<Vec<_>>()),
        );
        grouped.insert(
            "user_ids".into(),
            json!(results.users.iter().map(|u| u.id).collect::<Vec<_>>()),
        );
        grouped.insert("category_ids".into(), json!(results.category_ids));
        if tagging {
            grouped.insert(
                "tag_ids".into(),
                json!(results.tags.iter().map(|t| t.id).collect::<Vec<_>>()),
            );
        }
        grouped.insert(
            "group_ids".into(),
            json!(results.groups.iter().map(|g| g.id).collect::<Vec<_>>()),
        );
        out.insert("grouped_search_result".into(), Value::Object(grouped));
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_terms() {
        assert_eq!(clean_term("cap\u{200b}ybara"), "capybara");
        assert_eq!(clean_term("\u{201c}discourse\u{201d}"), "\"discourse\"");
        assert_eq!(clean_term("it\u{2019}s"), "it's");
    }

    #[test]
    fn filters_short_terms() {
        assert_eq!(
            filter_short_terms("abc de \"x y\" fghi", 3),
            "abc \"x y\" fghi"
        );
        assert_eq!(filter_short_terms("ab cd", 3), "");
        assert_eq!(filter_short_terms("\"a b c d\"", 3), "\"a b c d\"");
    }

    #[test]
    fn builds_the_tsquery_literal() {
        assert_eq!(
            ts_query_value("hello world", "ABCD", true),
            "'hello world':*ABCD"
        );
        assert_eq!(ts_query_value("hello", "A", false), "'hello':A");
        assert_eq!(ts_query_value("it's", "ABCD", true), "'it''s':*ABCD");
        assert_eq!(ts_query_value("a\\b", "ABCD", true), "'a\\\\\\\\b':*ABCD");
        assert_eq!(ruby_float(0.8), "0.8");
        assert_eq!(ruby_float(1.0), "1.0");
        assert_eq!(
            posts_query::quoted_phrases("x \"a b\" y \"c\""),
            vec!["a b", "c"]
        );
    }

    #[test]
    fn squishes_like_ruby() {
        assert_eq!(squish("  a \t\"b   c\"\n d "), "a \"b c\" d");
    }
}
