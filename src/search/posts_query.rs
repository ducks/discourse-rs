//! `Search#posts_query` with `apply_filters` and `apply_order`, and the
//! topic searches over it (`topic_search`, `aggregate_search`).

use super::advanced::{Advanced, Filter};
use super::{
    PostHit, Results, Search, SearchContext, SearchError, TopicContext, TypeFilter, ts_config,
    ts_query_value,
};
use crate::Unsupported;
use crate::user_search::quote;

/// What one search carries between its queries.
pub(super) struct Run {
    pub adv: Advanced,
    /// `@term` after filter_short_terms; empty when only filters were given.
    pub term: String,
    pub context: Option<SearchContext>,
    /// The type filter after the term's keywords (private messages, all
    /// topics).
    pub type_filter: Option<TypeFilter>,
    /// `@category_filter_matched`
    pub category_filter_matched: bool,
    pub limit: i64,
    pub offset: i64,
}

impl Run {
    fn topic(&self) -> Option<TopicContext> {
        match self.context {
            Some(SearchContext::Topic(t)) => Some(t),
            _ => None,
        }
    }

    /// posts_query's `type_filter`: what aggregate_search passes, nil
    /// for a topic's search and in:all-posts.
    fn pms(&self) -> bool {
        self.type_filter == Some(TypeFilter::PrivateMessages)
    }

    fn all_topics(&self) -> bool {
        self.type_filter == Some(TypeFilter::AllTopics)
    }
}

/// `Topic::PRIVATE_MESSAGES_SQL_USER` and `_GROUP` for a user.
fn private_posts_for_user(uid: i32) -> String {
    format!(
        "topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {uid}) \
         OR topics.id IN (SELECT tg.topic_id FROM topic_allowed_groups tg \
           JOIN group_users gu ON gu.user_id = {uid} AND gu.group_id = tg.group_id)"
    )
}

/// A query on posts as ActiveRecord accumulates it.
#[derive(Default)]
pub(super) struct Query {
    pub(super) joins: Vec<String>,
    pub(super) wheres: Vec<String>,
    pub(super) orders: Vec<String>,
}

impl Query {
    pub(super) fn and(&mut self, clause: impl Into<String>) {
        self.wheres.push(clause.into());
    }

    /// `joins(string)`: ActiveRecord keeps one of identical joins.
    pub(super) fn join(&mut self, join: &str) {
        if !self.joins.iter().any(|j| j == join) {
            self.joins.push(join.to_string());
        }
    }

    /// `posts.none` and `where("1 = 0")`: nothing matches.
    pub(super) fn none(&mut self) {
        self.and("1 = 0");
    }

    /// The SQL for `select`, with `{split}` where aggregate_post_sql's
    /// recent-posts condition goes.
    fn to_sql(&self, select: &str, group: bool, limit: i64, offset: i64) -> String {
        let mut sql = format!("SELECT {select} FROM posts");
        for join in &self.joins {
            sql.push(' ');
            sql.push_str(join);
        }
        sql.push_str(" WHERE ");
        sql.push_str(
            &self
                .wheres
                .iter()
                .map(|w| format!("({w})"))
                .collect::<Vec<_>>()
                .join(" AND "),
        );
        sql.push_str("{split}");
        if group {
            sql.push_str(" GROUP BY topics.id");
        }
        if !self.orders.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&self.orders.join(", "));
        }
        sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
        sql
    }

    /// FROM and WHERE with the eager-load joins after the association
    /// joins (post_search_data and topics, the first two), as
    /// ActiveRecord orders them.
    fn eager_from_where(&self, eager: &[&str], extra_where: &str) -> String {
        let mut sql = String::from(" FROM posts");
        for (i, join) in self.joins.iter().enumerate() {
            if i == 2 {
                for e in eager {
                    sql.push(' ');
                    sql.push_str(e);
                }
            }
            sql.push(' ');
            sql.push_str(join);
        }
        sql.push_str(" WHERE ");
        sql.push_str(
            &self
                .wheres
                .iter()
                .map(|w| format!("({w})"))
                .collect::<Vec<_>>()
                .join(" AND "),
        );
        sql.push_str(extra_where);
        sql
    }
}

/// `match.to_i` of a digits capture, as ActiveRecord quotes the integer.
fn int(digits: &str) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// `columns_for_distinct`: an ORDER BY term without its direction.
fn without_direction(order: &str) -> String {
    let re = regex::Regex::new(r"(?i)\s+(?:ASC|DESC)\b").expect("static pattern");
    re.replace_all(order, "").into_owned()
}

fn id_list(ids: &[i32]) -> String {
    ids.iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `Search.ts_query` inlined, as Rails interpolates it.
fn ts_query(config: &str, term: &str, weights: &str, prefix: bool) -> String {
    format!(
        "REGEXP_REPLACE(TO_TSQUERY('{config}', {})::text, '<->|<\\d+>', '&', 'g')::tsquery",
        quote(&ts_query_value(term, weights, prefix))
    )
}

/// `@term.scan(/"([^"]+)"/)`
pub(super) fn quoted_phrases(term: &str) -> Vec<String> {
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

/// posts_query's `term_without_quote` for a topic's search: the term, or
/// what its outermost double quotes enclose, or its outermost single
/// quotes (checked last, so they win).
fn topic_search_term(term: &str) -> String {
    let mut out = term.to_string();
    for quote in ['"', '\''] {
        if let (Some(first), Some(last)) = (term.find(quote), term.rfind(quote))
            && last > first + 1
        {
            out = term[first + 1..last].to_string();
        }
    }
    out
}

impl Search<'_> {
    fn weights(&self, run: &Run) -> Result<&'static str, SearchError> {
        Ok(if run.adv.in_title {
            "A"
        } else if self.settings.get("tagging_enabled")?.truthy() {
            "ABCD"
        } else {
            "ABD"
        })
    }

    /// `posts_query(limit, aggregate_search:)` for the default type filter.
    async fn posts_query(&mut self, run: &mut Run, aggregate: bool) -> Result<Query, SearchError> {
        let config = ts_config(&self.settings.get("default_locale")?.to_s());
        let mut q = Query::default();
        q.joins
            .push("INNER JOIN post_search_data ON post_search_data.post_id = posts.id".into());
        q.joins.push(
            "INNER JOIN topics ON topics.deleted_at IS NULL AND topics.id = posts.topic_id".into(),
        );
        let pms = run.pms();
        if !pms {
            q.joins
                .push("LEFT JOIN categories ON categories.id = topics.category_id".into());
        }
        let pm_topic = run.topic().is_some_and(|t| t.private_message);
        let post_types = id_list(&self.guardian.visible_post_types(self.settings)?);
        q.and("posts.deleted_at IS NULL");
        q.and(format!("posts.post_type IN ({post_types})"));
        q.and("posts.hidden = FALSE");
        let is_topic_search = run.topic().is_some();
        if !is_topic_search && !run.adv.include_invisible {
            q.and("topics.visible");
        }
        if pms || pm_topic {
            q.and("topics.archetype = 'private_message' AND post_search_data.private_message");
            if !self.guardian.is_admin() {
                q.and(private_posts_for_user(
                    self.guardian.user_id().unwrap_or(-1),
                ));
            }
        } else if run.all_topics() {
            // `.or(private_posts)`: the common conditions stay outside.
            q.and(format!(
                "(topics.archetype <> 'private_message' AND NOT post_search_data.private_message) \
                 OR (topics.archetype = 'private_message' AND post_search_data.private_message \
                 AND ({}))",
                private_posts_for_user(self.guardian.user_id().unwrap_or(-1))
            ));
        } else {
            q.and("topics.archetype <> 'private_message' AND NOT post_search_data.private_message");
        }

        if !run.term.is_empty() {
            if is_topic_search {
                q.joins.push("JOIN users u ON u.id = posts.user_id".into());
                let mut fields =
                    "posts.raw || ' ' || post_search_data.raw_data || ' ' || u.username"
                        .to_string();
                if self.settings.get("enable_names")?.truthy() {
                    fields.push_str(" || ' ' || COALESCE(u.name, '')");
                }
                q.and(format!(
                    "{fields} ILIKE {}",
                    quote(&format!("%{}%", topic_search_term(&run.term)))
                ));
            } else {
                if run.adv.in_title {
                    q.and("posts.post_number = 1");
                }
                let weights = self.weights(run)?;
                q.and(format!(
                    "post_search_data.search_data @@ {}",
                    ts_query(config, &run.term, weights, true)
                ));
                for exact in quoted_phrases(&run.term) {
                    let pattern = quote(&format!("%{exact}%"));
                    q.and(format!(
                        "posts.raw ILIKE {pattern} OR topics.title ILIKE {pattern}"
                    ));
                }
            }
        }

        self.apply_filters(&mut q, run).await?;

        match run.context {
            Some(SearchContext::Topic(topic)) => {
                q.and(format!("topics.id = {}", topic.id));
                if run.adv.order.is_none() {
                    q.orders.push("posts.post_number ASC".into());
                }
            }
            Some(SearchContext::User(id)) => {
                if !pms {
                    q.and(format!("posts.user_id = {id}"));
                } else if self.guardian.is_admin() && !run.adv.search_all_pms {
                    q.and(private_posts_for_user(id));
                }
            }
            None => {
                if !run.category_filter_matched {
                    q.and(
                        "categories.search_priority IS NULL OR categories.search_priority IS NOT NULL \
                         AND categories.search_priority <> 1",
                    );
                }
            }
        }

        if !pms {
            let secure_ids = self
                .guardian
                .secure_category_ids(&mut *self.conn, self.settings)
                .await?;
            if secure_ids.is_empty() {
                q.and("(categories.id IS NULL) OR (NOT categories.read_restricted)");
            } else {
                q.and(format!(
                    "(categories.id IS NULL) OR (NOT categories.read_restricted) OR (categories.id IN ({}))",
                    id_list(&secure_ids)
                ));
            }
        }

        self.apply_order(&mut q, run, aggregate, !is_topic_search)?;
        Ok(q)
    }

    /// `apply_order`: the term's order, else relevance (but in a topic);
    /// a plugin's order (topic voting's votes) replaces what came before.
    fn apply_order(
        &self,
        q: &mut Query,
        run: &Run,
        aggregate: bool,
        allow_relevance: bool,
    ) -> Result<(), SearchError> {
        let in_title = run.adv.in_title;
        let pick = |agg: &str, plain: &str| {
            if aggregate {
                agg.to_string()
            } else {
                plain.to_string()
            }
        };
        let order = run.adv.order.as_deref();
        let set: Option<String> = match order {
            Some("latest") if in_title => Some("topics.bumped_at DESC".into()),
            Some("latest") => Some(pick("MAX(posts.created_at) DESC", "posts.created_at DESC")),
            Some("oldest") if in_title => Some("topics.bumped_at ASC".into()),
            Some("oldest") => Some(pick("MAX(posts.created_at) ASC", "posts.created_at ASC")),
            Some("latest_topic") => Some(pick(
                "MAX(topics.created_at) DESC",
                "topics.created_at DESC",
            )),
            Some("oldest_topic") => {
                Some(pick("MAX(topics.created_at) ASC", "topics.created_at ASC"))
            }
            Some("views") => Some(pick("MAX(topics.views) DESC", "topics.views DESC")),
            Some("likes") => Some(pick("MAX(posts.like_count) DESC", "posts.like_count DESC")),
            Some("read") if self.guardian.user_id().is_some() => {
                let uid = self.guardian.user_id().unwrap_or_default();
                q.join(&format!(
                    "JOIN topic_users tu ON tu.topic_id = posts.topic_id AND tu.user_id = {uid}"
                ));
                q.and("NOT (tu.last_visited_at IS NULL)");
                Some(pick(
                    "MAX(tu.last_visited_at) DESC",
                    "tu.last_visited_at DESC",
                ))
            }
            _ => None,
        };
        match set {
            // reorder and order alike: nothing is ordered before a term's
            // order (a topic's post order is only for a term without one).
            Some(o) => q.orders = vec![o],
            None if allow_relevance => self.sort_by_relevance(q, run, aggregate)?,
            None => {}
        }
        // Search.advanced_orders
        if order == Some("votes") && self.settings.get("topic_voting_enabled")?.truthy() {
            q.orders = vec![
                "COALESCE((SELECT dvtvc.votes_count FROM topic_voting_topic_vote_count dvtvc \
                 WHERE dvtvc.topic_id = topics.id), 0) DESC"
                    .into(),
            ];
        }
        Ok(())
    }

    /// `sort_by_relevance` for the default type filter.
    fn sort_by_relevance(
        &self,
        q: &mut Query,
        run: &Run,
        aggregate: bool,
    ) -> Result<(), SearchError> {
        // Private messages have no categories to weigh.
        if run.pms() {
            q.orders.push("topics.bumped_at DESC".into());
            return Ok(());
        }
        let config = ts_config(&self.settings.get("default_locale")?.to_s());
        let wrap = |expr: String| {
            if aggregate {
                format!("MAX({expr}) DESC")
            } else {
                format!("{expr} DESC")
            }
        };
        let normalization = self.settings.get("search_ranking_normalization")?.to_s();
        let low = super::ruby_float(
            self.settings
                .get("category_search_priority_low_weight")?
                .to_f(),
        );
        let high = super::ruby_float(
            self.settings
                .get("category_search_priority_high_weight")?
                .to_f(),
        );
        // search_rank_sort_priorities, highest first: discourse-solved's
        // solved topics with prioritize_solved_topics_in_search, then
        // closed, then archived.
        let solved = if self.settings.get("solved_enabled")?.truthy()
            && self
                .settings
                .get("prioritize_solved_topics_in_search")?
                .truthy()
        {
            "WHEN EXISTS (SELECT 1 FROM discourse_solved_solved_topics \
             WHERE discourse_solved_solved_topics.topic_id = topics.id) THEN 1.1 "
        } else {
            ""
        };
        let category_priority_weights = format!(
            "(CASE categories.search_priority WHEN 3 THEN {low} WHEN 4 THEN {high} ELSE 1.0 END \
             * CASE {solved}WHEN topics.closed THEN 0.9 WHEN topics.archived THEN 0.85 ELSE 1.0 END)"
        );
        q.orders.push(wrap(
            "(CASE categories.search_priority WHEN 5 THEN 3 WHEN 2 THEN 1 ELSE 2 END)".into(),
        ));
        let term_present = !run.term.is_empty();
        if term_present
            && self
                .settings
                .get("prioritize_exact_search_title_match")?
                .truthy()
        {
            let exact_rank = format!(
                "TS_RANK_CD(post_search_data.search_data, {}, {normalization}|32)",
                ts_query(config, &run.term, "A", false)
            );
            q.orders
                .push(wrap(format!("{exact_rank} * {category_priority_weights}")));
        }
        let data_ranking = if term_present {
            let rank = format!(
                "TS_RANK_CD(post_search_data.search_data, {}, {normalization}|32)",
                ts_query(config, &run.term, self.weights(run)?, true)
            );
            format!("({rank} * {category_priority_weights})")
        } else {
            format!("({category_priority_weights})")
        };
        q.orders.push(wrap(data_ranking));
        q.orders.push("topics.bumped_at DESC".into());
        Ok(())
    }

    /// `apply_filters`: each matched filter's block over the query.
    async fn apply_filters(&mut self, q: &mut Query, run: &mut Run) -> Result<(), SearchError> {
        let filters = run.adv.filters.clone();
        for (filter, capture) in filters {
            let capture = capture.unwrap_or_default();
            match filter {
                Filter::Category => self.category_filter(q, run, &capture).await?,
                Filter::Hashtag => self.hashtag_filter(q, run, &capture).await?,
                Filter::At => self.at_filter(q, &capture).await?,
                Filter::User => self.user_filter(q, &capture).await?,
                Filter::CreatedBy => self.created_by_filter(q, &capture).await?,
                Filter::Group => self.group_filter(q, &capture).await?,
                Filter::Badge => self.badge_filter(q, &capture).await?,
                Filter::Tagged => q.and(
                    "EXISTS (SELECT 1 FROM topic_tags WHERE topic_tags.topic_id = posts.topic_id)",
                ),
                Filter::Untagged => {
                    q.join("LEFT JOIN topic_tags ON topic_tags.topic_id = posts.topic_id");
                    q.and("topic_tags.id IS NULL");
                }
                Filter::Bots => q.and("posts.user_id < 0"),
                Filter::Humans => q.and("posts.user_id >= 0"),
                Filter::Whispers => {
                    if self.guardian.can_see_whispers(self.settings)? {
                        q.and("posts.post_type = 4");
                    } else {
                        q.none();
                    }
                }
                Filter::Regular => q.and("posts.post_type = 1"),
                Filter::First => q.and("posts.post_number = 1"),
                Filter::Replies => q.and("posts.post_number > 1"),
                Filter::Pinned => q.and("topics.pinned_at IS NOT NULL"),
                Filter::Wiki => q.and("posts.wiki = TRUE"),
                Filter::Likes
                | Filter::Bookmarks
                | Filter::Posted
                | Filter::Created
                | Filter::WatchingTracking
                | Filter::Seen
                | Filter::Unseen => self.viewer_filter(q, filter, &capture),
                Filter::Polls => {
                    if self.settings.get("poll_enabled")?.truthy() {
                        q.join("INNER JOIN polls ON polls.post_id = posts.id");
                    }
                }
                Filter::StatusOpen => q.and("NOT topics.closed AND NOT topics.archived"),
                Filter::StatusClosed => q.and("topics.closed"),
                Filter::StatusPublic => self.public_filter(q).await?,
                Filter::StatusArchived => q.and("topics.archived"),
                Filter::StatusNoreplies => q.and("topics.posts_count = 1"),
                Filter::StatusSingleUser => q.and("topics.participant_count = 1"),
                Filter::PostsCount => q.and(format!("topics.posts_count = {}", int(&capture))),
                Filter::MinPostCount | Filter::MinPosts => {
                    q.and(format!("topics.posts_count >= {}", int(&capture)))
                }
                Filter::MaxPosts => q.and(format!("topics.posts_count <= {}", int(&capture))),
                Filter::MinViews => q.and(format!("topics.views >= {}", int(&capture))),
                Filter::MaxViews => q.and(format!("topics.views <= {}", int(&capture))),
                Filter::MinVoteCount => q.and(format!(
                    "(SELECT votes_count FROM topic_voting_topic_vote_count \
                     WHERE topic_voting_topic_vote_count.topic_id = posts.topic_id) >= {}",
                    int(&capture)
                )),
                Filter::WithImages => q.and("posts.image_upload_id IS NOT NULL"),
                Filter::Filetypes => self.filetypes_filter(q, &capture),
                Filter::Before | Filter::After => {
                    match super::dates::word_to_date(&capture, crate::clock::now()) {
                        Some(date) => q.and(format!(
                            "posts.created_at {} '{}'",
                            if filter == Filter::Before { "<" } else { ">" },
                            date.format("%Y-%m-%d %H:%M:%S")
                        )),
                        None => q.none(),
                    }
                }
                Filter::Locale => self.locale_filter(q, &capture),
                Filter::Solved => {
                    q.join(
                        "INNER JOIN discourse_solved_solved_topics \
                         ON discourse_solved_solved_topics.topic_id = topics.id",
                    );
                    q.and("NOT (topics.archetype = 'private_message')");
                }
                Filter::Unsolved => self.unsolved_filter(q).await?,
                Filter::PersonalDirect => {
                    if let Some(uid) = self.guardian.user_id() {
                        q.join("LEFT JOIN topic_allowed_groups tg ON posts.topic_id = tg.topic_id");
                        q.and(format!(
                            "tg.id IS NULL AND posts.topic_id IN (SELECT tau.topic_id \
                             FROM topic_allowed_users tau JOIN topic_allowed_users tau2 \
                             ON tau2.topic_id = tau.topic_id AND tau2.id != tau.id \
                             WHERE tau.user_id = {uid} GROUP BY tau.topic_id HAVING COUNT(*) = 1)"
                        ));
                    }
                }
                Filter::AllPms => {
                    if self.guardian.is_admin() {
                        q.and("topics.archetype = 'private_message'");
                    }
                }
                Filter::GroupMessages => self.group_messages_filter(q, &capture).await?,
                Filter::Tags => self.tags_filter(q, &capture, true).await?,
                Filter::NotTags => self.tags_filter(q, &capture, false).await?,
            }
        }
        Ok(())
    }

    /// `categor(y|ies):` a comma list of slugs, names or ids, each with its
    /// subcategories unless `=` prefixed.
    async fn category_filter(
        &mut self,
        q: &mut Query,
        run: &mut Run,
        terms: &str,
    ) -> Result<(), SearchError> {
        // `.map { [term, exact] }.to_h`: the first position, the last value.
        let mut matches: Vec<(String, bool)> = Vec::new();
        for term in crate::ruby::split(terms, ",") {
            let (key, exact) = match term.strip_prefix('=') {
                Some(rest) => (rest.to_string(), true),
                None => (term.to_string(), false),
            };
            match matches.iter_mut().find(|(k, _)| *k == key) {
                Some(entry) => entry.1 = exact,
                None => matches.push((key, exact)),
            }
        }
        let mut category_ids: Vec<i32> = Vec::new();
        if !matches.is_empty() {
            let keys: Vec<&str> = matches.iter().map(|(k, _)| k.as_str()).collect();
            let found: Vec<(i32, String)> = sqlx::query_as(
                "SELECT c.id, term FROM categories c \
                 JOIN unnest($1::text[]) AS term ON \
                   lower(unaccent(c.slug)) ILIKE lower(unaccent(term)) OR \
                   lower(unaccent(c.name)) ILIKE lower(unaccent(term)) OR \
                   (term ~ '^[0-9]{1,10}$' AND c.id = term::int)",
            )
            .bind(&keys)
            .fetch_all(&mut *self.conn)
            .await?;
            for (id, term) in found {
                category_ids.push(id);
                run.category_filter_matched = true;
                let exact = matches
                    .iter()
                    .find(|(k, _)| *k == term)
                    .is_some_and(|(_, e)| *e);
                if !exact {
                    category_ids.extend(self.subcategory_ids(id).await?);
                }
            }
        }
        if category_ids.is_empty() {
            q.none();
        } else {
            let mut unique: Vec<i32> = Vec::new();
            for id in category_ids {
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            q.and(format!("topics.category_id IN ({})", id_list(&unique)));
        }
        Ok(())
    }

    /// `#slug`, `#parent:child`, `#=slug`: a category (with its
    /// subcategories unless exact), else a tag, else a tag group, else the
    /// word searched as it was typed.
    async fn hashtag_filter(
        &mut self,
        q: &mut Query,
        run: &mut Run,
        word: &str,
    ) -> Result<(), SearchError> {
        let parts = crate::ruby::split(word, ":");
        let Some(first) = parts.first() else {
            return Ok(());
        };
        let subcategory_slug = parts.get(1);
        let (category_slug, exact) = match first.strip_prefix('=') {
            Some(rest) => (rest.to_string(), true),
            None => (first.to_string(), false),
        };
        let category_id: Option<i32> =
            match subcategory_slug {
                Some(sub) => sqlx::query_scalar(
                    "SELECT id FROM categories WHERE lower(unaccent(slug)) = lower(unaccent($1)) \
                     AND parent_category_id IN (SELECT id FROM categories \
                       WHERE lower(unaccent(slug)) = lower(unaccent($2))) LIMIT 1",
                )
                .bind(sub)
                .bind(&category_slug)
                .fetch_optional(&mut *self.conn)
                .await?,
                None => sqlx::query_scalar(
                    "SELECT id FROM categories WHERE lower(unaccent(slug)) = lower(unaccent($1)) \
                     ORDER BY case when parent_category_id is null then 0 else 1 end LIMIT 1",
                )
                .bind(&category_slug)
                .fetch_optional(&mut *self.conn)
                .await?,
            };
        if let Some(id) = category_id {
            let mut ids = vec![id];
            if !exact {
                ids.extend(self.subcategory_ids(id).await?);
            }
            run.category_filter_matched = true;
            q.and(format!("topics.category_id IN ({})", id_list(&ids)));
            return Ok(());
        }
        // A tag, or the tag its synonym points to.
        let tag: Option<(i32, Option<i32>)> = sqlx::query_as(
            "SELECT id, target_tag_id FROM tags WHERE lower(tags.name) IN ($1) LIMIT 1",
        )
        .bind(category_slug.to_lowercase())
        .fetch_optional(&mut *self.conn)
        .await?;
        if let Some((id, target)) = tag {
            q.and(format!(
                "topics.id IN (SELECT DISTINCT(tt.topic_id) FROM topic_tags tt WHERE tt.tag_id = {})",
                target.unwrap_or(id)
            ));
            return Ok(());
        }
        if let Some(group_id) = self.tag_group_id_by_slug(&category_slug).await? {
            q.and(format!(
                "topics.id IN (SELECT DISTINCT(tt.topic_id) FROM topic_tags tt \
                 WHERE tt.tag_id in (SELECT tag_id FROM tag_group_memberships WHERE tag_group_id = {group_id}))"
            ));
            return Ok(());
        }
        let min_length = self.settings.get("min_search_term_length")?.to_i().max(0) as usize;
        if word.chars().count() >= min_length {
            let config = ts_config(&self.settings.get("default_locale")?.to_s());
            q.and(format!(
                "posts.id IN (SELECT post_id FROM post_search_data pd1 WHERE pd1.search_data @@ {})",
                ts_query(config, &format!("#{word}"), "", true)
            ));
        }
        Ok(())
    }

    /// `TagGroup.find_id_by_slug`: the first group whose name's
    /// `Slug.for` is the slug.
    async fn tag_group_id_by_slug(&mut self, slug: &str) -> Result<Option<i32>, SearchError> {
        let groups: Vec<(i32, String)> = sqlx::query_as("SELECT id, name FROM tag_groups")
            .fetch_all(&mut *self.conn)
            .await?;
        if groups.is_empty() {
            return Ok(None);
        }
        if self.settings.get("slug_generation_method")?.to_s() != "ascii" {
            return Err(Unsupported(
                "tag group slugs with slug_generation_method other than ascii",
            )
            .into());
        }
        let locale = self.settings.get("default_locale")?.to_s();
        for (id, name) in groups {
            if crate::posting::text::slug_for(&name, &locale)? == slug {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// `Category.subcategory_ids`: the category and its descendants.
    async fn subcategory_ids(&mut self, category_id: i32) -> Result<Vec<i32>, SearchError> {
        Ok(sqlx::query_scalar(
            "WITH RECURSIVE subcategories AS ( \
                 SELECT $1::int AS id, 1 AS depth \
                 UNION \
                 SELECT categories.id, subcategories.depth + 1 \
                 FROM categories JOIN subcategories ON subcategories.id = categories.parent_category_id \
                 WHERE subcategories.depth < $2) \
             SELECT id FROM subcategories",
        )
        .bind(category_id)
        .bind(self.settings.get("max_category_nesting")?.to_i() as i32)
        .fetch_all(&mut *self.conn)
        .await?)
    }

    /// `topic_search`: a topic's matching posts in order, else
    /// `aggregate_search`.
    pub(super) async fn topic_search(
        &mut self,
        run: &mut Run,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let (limit, offset) = (run.limit, run.offset);
        if let Some(topic) = run.topic() {
            let q = self.posts_query(run, false).await?;
            let extra = format!(" AND (posts.topic_id = {})", topic.id);
            results.posts = self.eager_posts(&q, &extra, limit, offset).await?;
            return Ok(());
        }
        if run.adv.all_posts {
            let q = self.posts_query(run, false).await?;
            results.posts = self.eager_posts(&q, "", limit, offset).await?;
            return Ok(());
        }
        self.aggregate_search(run, results).await
    }

    /// posts_query's posts loaded with posts_eager_loads: the
    /// `references(:categories)` turns ActiveRecord's includes into
    /// eager-load joins, so with a limit it picks the distinct ids first and
    /// loads them in a second, ordered query (ties fall as Postgres plans
    /// that one), each post once.
    async fn eager_posts(
        &mut self,
        q: &Query,
        extra_where: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<PostHit>, SearchError> {
        // ActiveRecord's alias tracker renames an eager join whose table a
        // string join already uses; these joins only load associations, so
        // their own aliases leave the results as they are.
        let mut eager = vec![
            "LEFT OUTER JOIN categories categories_topics ON categories_topics.id = topics.category_id",
            "LEFT OUTER JOIN categories parent_categories_categories \
             ON parent_categories_categories.id = categories_topics.parent_category_id",
        ];
        if self.settings.get("tagging_enabled")?.truthy() {
            eager.push("LEFT OUTER JOIN topic_tags eager_topic_tags ON eager_topic_tags.topic_id = topics.id");
            eager
                .push("LEFT OUTER JOIN tags eager_tags ON eager_tags.id = eager_topic_tags.tag_id");
        }
        // discourse-solved's custom_topic_eager_load
        if self.settings.get("solved_enabled")?.truthy() {
            eager.push(
                "LEFT OUTER JOIN discourse_solved_solved_topics eager_solved \
                 ON eager_solved.topic_id = topics.id",
            );
            eager.push(
                "LEFT OUTER JOIN discourse_solved_topic_answers eager_answers \
                 ON eager_answers.solved_topic_id = eager_solved.id",
            );
        }
        eager.push("LEFT OUTER JOIN users eager_users ON eager_users.id = posts.user_id");
        let from_where = q.eager_from_where(&eager, extra_where);
        let order = if q.orders.is_empty() {
            String::new()
        } else {
            format!(" ORDER BY {}", q.orders.join(", "))
        };
        let mut columns: Vec<String> = q
            .orders
            .iter()
            .enumerate()
            .map(|(i, o)| format!("{} AS alias_{i}", without_direction(o)))
            .collect();
        columns.push("posts.id".into());
        let rows = sqlx::query(&format!(
            "SELECT DISTINCT {}{from_where}{order} LIMIT {limit} OFFSET {offset}",
            columns.join(", ")
        ))
        .fetch_all(&mut *self.conn)
        .await?;
        let ids: Vec<i32> = rows
            .iter()
            .map(|r| sqlx::Row::try_get::<i32, _>(r, "id"))
            .collect::<Result<_, _>>()?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let loaded: Vec<PostHit> = sqlx::query_as(&format!(
            "SELECT posts.id, posts.user_id, posts.topic_id, posts.post_number, posts.created_at, \
                    posts.like_count, post_search_data.raw_data, post_search_data.version\
             {from_where} AND posts.id IN ({}){order}",
            id_list(&ids)
        ))
        .fetch_all(&mut *self.conn)
        .await?;
        let mut out: Vec<PostHit> = Vec::new();
        for post in loaded {
            if !out.iter().any(|p| p.id == post.id) {
                out.push(post);
            }
        }
        Ok(out)
    }

    /// `aggregate_search`: one row per topic with its lowest matching post
    /// number, topics ranked by their best post. With
    /// search_recent_regular_posts_offset_post_id set (large sites, by
    /// create_recent_post_search_indexes), `aggregate_post_sql` splits at
    /// that post: topics matched by recent posts first, the older ones only
    /// when those fall short of the limit.
    pub(super) async fn aggregate_search(
        &mut self,
        run: &mut Run,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let (limit, offset) = (run.limit, run.offset);
        // Likes are not aggregated: each matching post with its likes.
        let order = run.adv.order.clone();
        let inner = if order.as_deref() == Some("likes") {
            let q = self.posts_query(run, false).await?;
            q.to_sql("topics.id, posts.post_number", false, limit, offset)
        } else {
            let q = self.posts_query(run, true).await?;
            let min_or_max = if order.as_deref() == Some("latest") {
                "max"
            } else {
                "min"
            };
            q.to_sql(
                &format!("topics.id, {min_or_max}(posts.post_number) post_number"),
                true,
                limit,
                offset,
            )
        };
        let sql = format!(
            "SELECT posts.id, posts.user_id, posts.topic_id, posts.post_number, posts.created_at, \
                    posts.like_count, psd.raw_data, psd.version \
             FROM posts \
             JOIN (SELECT *, row_number() over() row_number FROM ({inner}) xxx) x \
               ON x.id = posts.topic_id AND x.post_number = posts.post_number \
             JOIN post_search_data psd ON psd.post_id = posts.id \
             WHERE posts.deleted_at IS NULL ORDER BY x.row_number"
        );
        // before: and after: search every post.
        let dated = run
            .adv
            .filters
            .iter()
            .any(|(f, _)| matches!(f, Filter::Before | Filter::After));
        // aggregate_post_sql lists "private_message", a type filter that never
        // occurs, so only all_topics (and the dated filters) skip the split.
        let min_id = if dated || run.all_topics() {
            0
        } else {
            self.settings
                .get("search_recent_regular_posts_offset_post_id")?
                .to_i()
        };
        let parts: Vec<String> = if min_id > 0 {
            vec![
                format!(" AND (post_search_data.post_id >= {min_id})"),
                format!(" AND (post_search_data.post_id < {min_id})"),
            ]
        } else {
            vec![String::new()]
        };
        for (i, split) in parts.iter().enumerate() {
            if i > 0 && results.posts.len() as i64 >= limit {
                break;
            }
            let sql = sql.replace("{split}", split);
            let rows: Vec<PostHit> = sqlx::query_as(&sql).fetch_all(&mut *self.conn).await?;
            results.posts.extend(rows);
        }
        Ok(())
    }
}
