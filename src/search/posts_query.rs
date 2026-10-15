//! `Search#posts_query` with `apply_filters` and `apply_order`, and the
//! topic searches over it (`topic_search`, `aggregate_search`).

use super::advanced::{Advanced, Filter};
use super::{PostHit, Results, Search, SearchError, TopicContext, ts_config, ts_query_value};
use crate::Unsupported;
use crate::user_search::quote;

/// What one search carries between its queries.
pub(super) struct Run {
    pub adv: Advanced,
    /// `@term` after filter_short_terms; empty when only filters were given.
    pub term: String,
    pub topic: Option<TopicContext>,
    /// `@category_filter_matched`
    pub category_filter_matched: bool,
    pub limit: i64,
    pub offset: i64,
}

/// A query on posts as ActiveRecord accumulates it.
#[derive(Default)]
struct Query {
    joins: Vec<String>,
    wheres: Vec<String>,
    orders: Vec<String>,
}

impl Query {
    fn and(&mut self, clause: impl Into<String>) {
        self.wheres.push(clause.into());
    }

    /// `posts.none` and `where("1 = 0")`: nothing matches.
    fn none(&mut self) {
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
        q.joins
            .push("LEFT JOIN categories ON categories.id = topics.category_id".into());
        let pm_topic = run.topic.is_some_and(|t| t.private_message);
        let post_types = id_list(&self.guardian.visible_post_types(self.settings)?);
        q.and("posts.deleted_at IS NULL");
        q.and(format!("posts.post_type IN ({post_types})"));
        q.and("posts.hidden = FALSE");
        let is_topic_search = run.topic.is_some();
        if !is_topic_search {
            q.and("topics.visible");
        }
        if pm_topic {
            q.and("topics.archetype = 'private_message' AND post_search_data.private_message");
            if !self.guardian.is_admin() {
                let uid = self.guardian.user_id().unwrap_or(-1);
                q.and(format!(
                    "topics.id IN (SELECT topic_id FROM topic_allowed_users WHERE user_id = {uid}) \
                     OR topics.id IN (SELECT tg.topic_id FROM topic_allowed_groups tg \
                       JOIN group_users gu ON gu.user_id = {uid} AND gu.group_id = tg.group_id)"
                ));
            }
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

        match run.topic {
            Some(topic) => {
                q.and(format!("topics.id = {}", topic.id));
                if run.adv.order.is_none() {
                    q.orders.push("posts.post_number ASC".into());
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

        if !is_topic_search {
            self.sort_by_relevance(&mut q, run, aggregate)?;
        }
        Ok(q)
    }

    /// `sort_by_relevance` for the default type filter.
    fn sort_by_relevance(
        &self,
        q: &mut Query,
        run: &Run,
        aggregate: bool,
    ) -> Result<(), SearchError> {
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
        let category_priority_weights = format!(
            "(CASE categories.search_priority WHEN 3 THEN {low} WHEN 4 THEN {high} ELSE 1.0 END \
             * CASE WHEN topics.closed THEN 0.9 WHEN topics.archived THEN 0.85 ELSE 1.0 END)"
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
                _ => return Err(Unsupported("an advanced search filter not ported").into()),
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
        if let Some(topic) = run.topic {
            let q = self.posts_query(run, false).await?;
            let sql = q
                .to_sql(
                    "posts.id, posts.user_id, posts.topic_id, posts.post_number, posts.created_at, \
                     posts.like_count, post_search_data.raw_data, post_search_data.version",
                    false,
                    limit,
                    offset,
                )
                .replace("{split}", &format!(" AND (posts.topic_id = {})", topic.id));
            results.posts = sqlx::query_as(&sql).fetch_all(&mut *self.conn).await?;
            return Ok(());
        }
        self.aggregate_search(run, results).await
    }

    /// `aggregate_search`: one row per topic with its lowest matching post
    /// number, topics ranked by their best post. With
    /// search_recent_regular_posts_offset_post_id set (large sites, by
    /// create_recent_post_search_indexes), `aggregate_post_sql` splits at
    /// that post: topics matched by recent posts first, the older ones only
    /// when those fall short of the limit.
    async fn aggregate_search(
        &mut self,
        run: &mut Run,
        results: &mut Results,
    ) -> Result<(), SearchError> {
        let (limit, offset) = (run.limit, run.offset);
        let q = self.posts_query(run, true).await?;
        let inner = q.to_sql(
            "topics.id, min(posts.post_number) post_number",
            true,
            limit,
            offset,
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
        let min_id = self
            .settings
            .get("search_recent_regular_posts_offset_post_id")?
            .to_i();
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
