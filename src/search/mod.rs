//! Port of lib/search.rb, lib/search/grouped_search_results.rb and
//! app/models/search_log.rb for anonymous users searching plain terms
//! (words and quoted phrases). Advanced syntax (`in:`, `status:`,
//! `category:`, `#`, `@user`, `order:`, ...) is refused until ported.

pub mod blurb;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use regex::Regex;
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::Unsupported;
use crate::avatar::{self, AvatarError};
use crate::categories::{Categories, CategoriesError};
use crate::guardian::Guardian;
use crate::i18n::I18n;
use crate::site_settings::{SettingError, SiteSettings};
use crate::tags::{Tag, VISIBLE_TAGS_WHERE};
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
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

/// `process_advanced_search!`'s tokenizer: a run of non-quote, non-space
/// characters with an optional quoted phrase glued on; unbalanced quotes
/// vanish.
fn words(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r');
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while i < chars.len() && chars[i] != '"' && !is_ws(chars[i]) {
            i += 1;
        }
        if i < chars.len() && chars[i] == '"' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == '"') {
                if close > 0 {
                    i += close + 2;
                }
            }
        }
        if i == start {
            i += 1;
            continue;
        }
        out.push(chars[start..i].iter().collect());
    }
    out
}

/// Every registered `advanced_filter` regex, tested on the word with its
/// quotes stripped (case-insensitive).
const FILTER_PATTERNS: &[&str] = &[
    r"\Ain:personal-direct\z",
    r"\Ain:all-pms\z",
    r"\Ain:tagged\z",
    r"\Ain:bots?\z",
    r"\Ain:humans?\z",
    r"\Ain:whispers?\z",
    r"\Ain:regular\z",
    r"\Ain:untagged\z",
    r"\Astatus:open\z",
    r"\Astatus:closed\z",
    r"\Astatus:public\z",
    r"\Astatus:archived\z",
    r"\Astatus:noreplies\z",
    r"\Astatus:single_user\z",
    r"\Aposts_count:(\d+)\z",
    r"\Amin_post_count:(\d+)\z",
    r"\Amin_posts:(\d+)\z",
    r"\Amax_posts:(\d+)\z",
    r"\Ain:first|^f\z",
    r"\Ain:replies\z",
    r"\Ain:pinned\z",
    r"\Ain:wiki\z",
    r"\Abadge:(.*)\z",
    r"\Ain:(likes)\z",
    r"\Ain:(bookmarks)\z",
    r"\Ain:posted\z",
    r"\Ain:(created|mine)\z",
    r"\Ain:(watching|tracking)\z",
    r"\Ain:seen\z",
    r"\Ain:unseen\z",
    r"\Acreated:@(.*)\z",
    r"\Awith:images\z",
    r"\Acategor(?:y|ies):(.+)\z",
    r"\A\#([\p{L}\p{M}0-9\-:=]+)\z",
    r"\Agroup:(.+)\z",
    r"\Agroup_messages:(.+)\z",
    r"\Auser:(.+)\z",
    r"\A\@(\S+)\z",
    r"\Abefore:(.*)\z",
    r"\Aafter:(.*)\z",
    r"\Atags?:([\p{L}\p{M}0-9,\-_+]+)\z",
    r"\A\-tags?:([\p{L}\p{M}0-9,\-_+]+)\z",
    r"\Afiletypes?:([a-zA-Z0-9,\-_]+)\z",
    r"\Amin_views:(\d+)\z",
    r"\Amax_views:(\d+)\z",
    r"\Alocale:([a-zA-Z0-9_-]+)\z",
];

/// The keyword chain tested on the raw word after the filters.
const KEYWORD_PATTERNS: &[&str] = &[
    r"\Aorder:\w+\z",
    r"\Ain:title\z",
    r"\Atopic:(\d+)\z",
    r"\Ain:all\z",
    r"\Ain:all-posts\z",
    r"\Ain:personal\z",
    r"\Ain:messages\z",
    r"\Ain:personal-direct\z",
    r"\Ain:all-pms\z",
    r"\Agroup_messages:(.+)\z",
    r"\Apersonal_messages:(.+)\z",
    r"\Ainclude:(invisible|unlisted)\z",
];

fn compiled(
    patterns: &'static [&'static str],
    cell: &'static OnceLock<Vec<Regex>>,
) -> &'static [Regex] {
    cell.get_or_init(|| {
        patterns
            .iter()
            .map(|p| {
                regex::RegexBuilder::new(p)
                    .case_insensitive(true)
                    .build()
                    .expect("static search pattern")
            })
            .collect()
    })
}

static FILTERS: OnceLock<Vec<Regex>> = OnceLock::new();
static KEYWORDS: OnceLock<Vec<Regex>> = OnceLock::new();

/// `process_advanced_search!` for the plain case: the words re-joined
/// with single spaces; any filter or keyword is refused.
pub fn process_advanced_search(term: &str) -> Result<String, Unsupported> {
    let filters = compiled(FILTER_PATTERNS, &FILTERS);
    let keywords = compiled(KEYWORD_PATTERNS, &KEYWORDS);
    let mut kept = Vec::new();
    for word in words(term) {
        let cleaned: String = word.chars().filter(|c| *c != '"' && *c != '\'').collect();
        if filters.iter().any(|re| re.is_match(&cleaned)) {
            return Err(Unsupported(
                "advanced search filters (in:, status:, category:, #, @, tags:, ...)",
            ));
        }
        if matches!(word.as_str(), "l" | "r" | "t") || keywords.iter().any(|re| re.is_match(&word))
        {
            return Err(Unsupported(
                "advanced search keywords (order:, in:title, l, t, ...)",
            ));
        }
        kept.push(word);
    }
    Ok(kept.join(" "))
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
fn ts_query_value(term: &str, weights: &str, prefix: bool) -> String {
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
fn ts_query_sql(config: &str, bind: usize) -> String {
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

#[derive(sqlx::FromRow)]
struct GroupHit {
    id: i32,
    automatic: bool,
    name: String,
    mentionable_level: i32,
    messageable_level: i32,
    visibility_level: i32,
    primary_group: bool,
    title: Option<String>,
    grant_trust_level: Option<i32>,
    flair_icon: Option<String>,
    flair_upload_id: Option<i32>,
    flair_upload_url: Option<String>,
    flair_bg_color: Option<String>,
    flair_color: Option<String>,
    bio_cooked: Option<String>,
    public_admission: bool,
    public_exit: bool,
    allow_membership_requests: bool,
    full_name: Option<String>,
    default_notification_level: i32,
    membership_request_template: Option<String>,
    members_visibility_level: i32,
    publish_read_state: bool,
}

/// `Search::GroupedSearchResults` before serialization.
#[derive(Default)]
struct Results {
    posts: Vec<PostHit>,
    users: Vec<UserHit>,
    category_ids: Vec<i32>,
    tags: Vec<Tag>,
    groups: Vec<GroupHit>,
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
    pub async fn execute(&mut self, args: &SearchArgs) -> Result<Option<Value>, SearchError> {
        self.check_unported()?;
        let clean = clean_term(&args.term);
        let after_advanced = process_advanced_search(&clean)?;
        // prepare_data: only URL rewriting is left to do after the tokenizer's
        // squish, and URLs lose their query string there.
        if after_advanced.split_whitespace().any(|w| {
            w.trim_matches('"').starts_with("http://")
                || w.trim_matches('"').starts_with("https://")
        }) {
            return Err(Unsupported("URLs in search terms (query string stripping)").into());
        }
        let blurb_term = (!after_advanced.is_empty()).then(|| after_advanced.clone());
        let original_term = after_advanced.replace('\\', "\\\\");

        let mut results = Results::default();
        if self.settings.get("log_search_queries")?.truthy()
            && args.type_filter != Some(TypeFilter::ExcludeTopics)
        {
            results.search_log_id = self.log(&clean, args).await?;
        }

        let min_length = self.settings.get("min_search_term_length")?.to_i().max(0) as usize;
        let term = filter_short_terms(&after_advanced, min_length);
        if term.trim().is_empty() {
            return Ok(None);
        }

        let limit = if args.full_page {
            self.settings.get("search_page_size")?.to_i() + 1
        } else {
            PER_FACET as i64 + 1
        };
        let per_filter = self.settings.get("search_page_size")?.to_i().max(1) as usize;
        let offset = if args.full_page && args.type_filter.is_some() {
            (args.page - 1) * per_filter as i64
        } else {
            0
        };

        // find_grouped_results
        match args.type_filter {
            Some(TypeFilter::User) => {
                self.user_search(&term, &original_term, &mut results)
                    .await?
            }
            Some(TypeFilter::Category) => self.category_search(&term, &mut results).await?,
            Some(TypeFilter::Tags) => self.tags_search(&term, &mut results).await?,
            Some(TypeFilter::ExcludeTopics) => {
                self.user_search(&term, &original_term, &mut results)
                    .await?;
                self.category_search(&term, &mut results).await?;
                self.tags_search(&term, &mut results).await?;
                self.groups_search(&term, &mut results).await?;
            }
            Some(TypeFilter::Topic) => {
                self.topic_search(&term, limit, offset, &mut results)
                    .await?
            }
            None => {
                self.user_search(&term, &original_term, &mut results)
                    .await?;
                self.category_search(&term, &mut results).await?;
                self.tags_search(&term, &mut results).await?;
                self.groups_search(&term, &mut results).await?;
                self.topic_search(&term, limit, offset, &mut results)
                    .await?;
            }
        }
        // add()'s limit trick: the extra row flips the flag.
        let cap = if args.full_page {
            per_filter
        } else {
            PER_FACET
        };
        if results.posts.len() > cap {
            results.posts.truncate(cap);
            if args.full_page {
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
                blurb_term.as_deref(),
                args.blurb_length,
                &original_term,
            )
            .await?,
        ))
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
        if s.get("search_prefer_recent_posts")?.truthy()
            || s.get("search_recent_regular_posts_offset_post_id")?.to_i() > 0
        {
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
        if s.get("search_default_sort_order")?.to_i() != 0 {
            return Err(Unsupported("search_default_sort_order other than relevance").into());
        }
        Ok(())
    }

    fn config(&self) -> Result<&'static str, SearchError> {
        Ok(ts_config(&self.settings.get("default_locale")?.to_s()))
    }

    /// `topic_search` -> `aggregate_search`: one row per topic with its
    /// lowest matching post number, topics ranked by their best post.
    async fn topic_search(
        &mut self,
        term: &str,
        limit: i64,
        offset: i64,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let config = self.config()?;
        let weights = if self.settings.get("tagging_enabled")?.truthy() {
            "ABCD"
        } else {
            "ABD"
        };
        let normalization = self.settings.get("search_ranking_normalization")?.to_s();
        let low = ruby_float(
            self.settings
                .get("category_search_priority_low_weight")?
                .to_f(),
        );
        let high = ruby_float(
            self.settings
                .get("category_search_priority_high_weight")?
                .to_f(),
        );
        let priority_weights = format!(
            "(CASE categories.search_priority WHEN 3 THEN {low} WHEN 4 THEN {high} ELSE 1.0 END \
             * CASE WHEN topics.closed THEN 0.9 WHEN topics.archived THEN 0.85 ELSE 1.0 END)"
        );
        let rank = format!(
            "TS_RANK_CD(post_search_data.search_data, {}, {normalization}|32)",
            ts_query_sql(config, 1)
        );
        let exact_rank = format!(
            "TS_RANK_CD(post_search_data.search_data, {}, {normalization}|32)",
            ts_query_sql(config, 2)
        );
        let mut order = vec![
            "MAX((CASE categories.search_priority WHEN 5 THEN 3 WHEN 2 THEN 1 ELSE 2 END)) DESC"
                .to_string(),
        ];
        if self
            .settings
            .get("prioritize_exact_search_title_match")?
            .truthy()
        {
            order.push(format!("MAX({exact_rank} * {priority_weights}) DESC"));
        }
        order.push(format!("MAX(({rank} * {priority_weights})) DESC"));
        order.push("topics.bumped_at DESC".to_string());

        // Quoted phrases must also appear verbatim in the raw or the title.
        let phrases: Vec<String> = quoted_phrases(term);
        let mut phrase_clauses = String::new();
        for (i, _) in phrases.iter().enumerate() {
            let bind = 5 + i;
            phrase_clauses.push_str(&format!(
                " AND (posts.raw ILIKE ${bind} OR topics.title ILIKE ${bind})"
            ));
        }
        let inner = format!(
            "SELECT topics.id, min(posts.post_number) post_number FROM posts \
             INNER JOIN post_search_data ON post_search_data.post_id = posts.id \
             INNER JOIN topics ON topics.deleted_at IS NULL AND topics.id = posts.topic_id \
             LEFT JOIN categories ON categories.id = topics.category_id \
             WHERE posts.deleted_at IS NULL AND posts.post_type IN (1, 2, 3) AND posts.hidden = FALSE \
             AND (topics.visible) \
             AND (topics.archetype <> 'private_message' AND NOT post_search_data.private_message) \
             AND (post_search_data.search_data @@ {}){phrase_clauses} \
             AND ((categories.search_priority IS NULL OR categories.search_priority IS NOT NULL AND categories.search_priority <> 1)) \
             AND ((categories.id IS NULL) OR (NOT categories.read_restricted)) \
             GROUP BY topics.id ORDER BY {} LIMIT $3 OFFSET $4",
            ts_query_sql(config, 1),
            order.join(", ")
        );
        let sql = format!(
            "SELECT posts.id, posts.user_id, posts.topic_id, posts.post_number, posts.created_at, \
                    posts.like_count, psd.raw_data, psd.version \
             FROM posts \
             JOIN (SELECT *, row_number() over() row_number FROM ({inner}) xxx) x \
               ON x.id = posts.topic_id AND x.post_number = posts.post_number \
             JOIN post_search_data psd ON psd.post_id = posts.id \
             WHERE posts.deleted_at IS NULL ORDER BY x.row_number"
        );
        let mut query = sqlx::query_as::<_, PostHit>(&sql)
            .bind(ts_query_value(term, weights, true))
            .bind(ts_query_value(term, "A", false))
            .bind(limit)
            .bind(offset);
        for phrase in &phrases {
            query = query.bind(format!("%{phrase}%"));
        }
        results.posts = query.fetch_all(&mut *self.conn).await?;
        Ok(())
    }

    /// `user_search`: active, unstaged, unsuspended users matching in the
    /// simple config, the exact username first, then by last post.
    async fn user_search(
        &mut self,
        term: &str,
        original_term: &str,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        if self
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
        if !self
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

    /// `category_search`: readable categories by their search data, busiest
    /// this month first.
    async fn category_search(
        &mut self,
        term: &str,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let config = self.config()?;
        let sql = format!(
            "SELECT categories.id FROM categories \
             LEFT OUTER JOIN category_search_data ON category_search_data.category_id = categories.id \
             WHERE (category_search_data.search_data @@ {}) AND (NOT categories.read_restricted) \
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
        let sql = format!(
            "SELECT tags.id, tags.name, tags.slug, tags.description, tags.description_cooked, \
                    tags.public_topic_count, tags.pm_topic_count, tags.target_tag_id FROM tags \
             LEFT OUTER JOIN tag_search_data ON tag_search_data.tag_id = tags.id \
             WHERE tags.target_tag_id IS NULL AND {VISIBLE_TAGS_WHERE} \
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
        results.groups = sqlx::query_as(
            "SELECT g.id, g.automatic, g.name, g.mentionable_level, g.messageable_level, \
                    g.visibility_level, g.primary_group, g.title, g.grant_trust_level, \
                    g.flair_icon, g.flair_upload_id, u.url AS flair_upload_url, g.flair_bg_color, \
                    g.flair_color, g.bio_cooked, g.public_admission, g.public_exit, \
                    g.allow_membership_requests, g.full_name, g.default_notification_level, \
                    g.membership_request_template, g.members_visibility_level, g.publish_read_state \
             FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
             WHERE (g.id > 0) AND (g.id NOT IN (4, 5)) AND (g.visibility_level = 0) \
             AND (g.name ILIKE $1 OR g.full_name ILIKE $1) ORDER BY g.name ASC LIMIT $2",
        )
        .bind(format!("%{term}%"))
        .bind(PER_FACET as i64 + 1)
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(())
    }

    /// `SearchLog.log`: a row per search, or the previous row from the same
    /// IP within 5 s updated when the new term extends it.
    async fn log(&mut self, term: &str, args: &SearchArgs) -> Result<Option<i64>, SearchError> {
        if term.trim().is_empty() || args.ip_address.is_empty() {
            return Ok(None);
        }
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
                .get(&args.ip_address)
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
                let crawler = self.crawler(user_agent.as_deref())?;
                sqlx::query_scalar(
                    "INSERT INTO search_logs (term, search_type, ip_address, user_agent, user_id, session_id, \
                                              crawler, likely_crawler, created_at) \
                     VALUES ($1, $2, $3::inet, $4, NULL, $5, $6, FALSE, now()) RETURNING id::bigint",
                )
                .bind(term)
                .bind(search_type)
                .bind(&args.ip_address)
                .bind(user_agent)
                .bind(session_id)
                .bind(crawler)
                .fetch_one(&mut *self.conn)
                .await?
            }
        };
        let mut entries = self
            .log_cache
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        entries.insert(
            args.ip_address.clone(),
            (id, term.to_string(), Instant::now()),
        );
        Ok(Some(id))
    }

    /// `CrawlerDetection.crawler?`: a missing or unfamiliar user agent is a
    /// crawler; a browser-like one is a crawler only when it names one.
    fn crawler(&self, user_agent: Option<&str>) -> Result<bool, SearchError> {
        let Some(ua) = user_agent.filter(|u| !u.is_empty()) else {
            return Ok(true);
        };
        let lower = ua.to_lowercase();
        let list = |name: &str| -> Result<Vec<String>, SearchError> {
            Ok(self
                .settings
                .get(name)?
                .to_s()
                .split('|')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_lowercase())
                .collect())
        };
        let possibly_real = list("non_crawler_user_agents")?
            .iter()
            .any(|s| lower.contains(s.as_str()));
        if !possibly_real {
            return Ok(true);
        }
        if list("crawler_check_bypass_agents")?
            .iter()
            .any(|s| lower.contains(s.as_str()))
        {
            return Ok(false);
        }
        Ok(list("crawler_user_agents")?
            .iter()
            .any(|s| lower.contains(s.as_str())))
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
            let mut topics = Vec::new();
            for id in &topic_ids {
                let Some(t) = rows.iter().find(|t| t.id == *id) else {
                    continue;
                };
                topics.push(
                    list.serialize_topic(t, &[], tagging, Mode::SearchItem)
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
                tags.push(t.serialize(&mut *self.conn).await?);
            }
            out.insert("tags".into(), Value::Array(tags));
        }

        let mut groups = Vec::new();
        for g in &results.groups {
            groups.push(self.basic_group(g)?);
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
            json!(self.guardian.is_authenticated()),
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

    /// BasicGroupSerializer for an anonymous reader.
    fn basic_group(&self, g: &GroupHit) -> Result<Value, SearchError> {
        let mut out = Map::new();
        out.insert("id".into(), json!(g.id));
        out.insert("automatic".into(), json!(g.automatic));
        out.insert("name".into(), json!(g.name));
        if g.automatic {
            out.insert(
                "display_name".into(),
                json!(self.i18n.t(&format!("groups.default_names.{}", g.name))),
            );
        }
        let can_see_members = g.members_visibility_level == 0;
        if can_see_members {
            return Err(Unsupported("group user_count").into());
        }
        out.insert("mentionable_level".into(), json!(g.mentionable_level));
        out.insert("messageable_level".into(), json!(g.messageable_level));
        out.insert("visibility_level".into(), json!(g.visibility_level));
        out.insert("primary_group".into(), json!(g.primary_group));
        out.insert("title".into(), json!(g.title));
        out.insert("grant_trust_level".into(), json!(g.grant_trust_level));
        let flair = crate::groups::Group {
            id: g.id,
            name: g.name.clone(),
            flair_icon: g.flair_icon.clone(),
            flair_upload_id: g.flair_upload_id,
            flair_bg_color: g.flair_bg_color.clone(),
            flair_color: g.flair_color.clone(),
            flair_upload_url: g.flair_upload_url.clone(),
        };
        out.insert("flair_url".into(), json!(flair.flair_url()?));
        out.insert("flair_bg_color".into(), json!(g.flair_bg_color));
        out.insert("flair_color".into(), json!(g.flair_color));
        out.insert("bio_cooked".into(), json!(g.bio_cooked));
        if g.bio_cooked.as_deref().is_some_and(|b| !b.is_empty()) {
            return Err(Unsupported("group bio_excerpt (PrettyText.excerpt)").into());
        }
        out.insert("bio_excerpt".into(), Value::Null);
        out.insert("public_admission".into(), json!(g.public_admission));
        out.insert("public_exit".into(), json!(g.public_exit));
        out.insert(
            "allow_membership_requests".into(),
            json!(g.allow_membership_requests),
        );
        out.insert("full_name".into(), json!(g.full_name));
        out.insert(
            "default_notification_level".into(),
            json!(g.default_notification_level),
        );
        out.insert(
            "membership_request_template".into(),
            json!(g.membership_request_template),
        );
        out.insert(
            "members_visibility_level".into(),
            json!(g.members_visibility_level),
        );
        out.insert("can_see_members".into(), json!(can_see_members));
        out.insert("publish_read_state".into(), json!(g.publish_read_state));
        Ok(Value::Object(out))
    }
}

/// `@term.scan(/"([^"]+)"/)`
fn quoted_phrases(term: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = term;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        match after.find('"') {
            Some(end) => {
                if end > 0 {
                    out.push(after[..end].to_string());
                }
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
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
    fn tokenizes_like_process_advanced_search() {
        assert_eq!(words("hello\tworld"), vec!["hello", "world"]);
        assert_eq!(words("foo\"bar baz\" x"), vec!["foo\"bar baz\"", "x"]);
        assert_eq!(words("\"a b\" c"), vec!["\"a b\"", "c"]);
        assert_eq!(words("a \" b"), vec!["a", "b"]);
        assert_eq!(
            process_advanced_search(" hello    world ").unwrap(),
            "hello world"
        );
        assert_eq!(process_advanced_search("foo:bar").unwrap(), "foo:bar");
        assert_eq!(process_advanced_search("a#b").unwrap(), "a#b");
        for term in [
            "hello #general",
            "f foo",
            "in:title x",
            "order:latest x",
            "@user",
            "l",
            "x category:general",
            "\"in:first\"",
        ] {
            assert!(process_advanced_search(term).is_err(), "{term}");
        }
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
        assert_eq!(quoted_phrases("x \"a b\" y \"c\""), vec!["a b", "c"]);
    }
}
