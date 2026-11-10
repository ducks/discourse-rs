//! `Topic.similar_to` (app/models/topic.rb) and what
//! SimilarTopicsController#index renders from it: the topics whose search
//! data matches a new topic's title and body, ranked by trigram
//! similarity, each with a blurb of its first post.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::posting::search_index::{self, BaseUrls};
use crate::pretty_text::{Host, MarkdownOptions};
use crate::search::{blurb, ts_config, ts_query_value};
use crate::site_settings::SiteSettings;
use crate::topic_list::{Mode, TopicListSerializer, time_json};
use crate::topic_query::{TOPIC_COLUMNS, TopicRow};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// `Topic::MAX_SIMILAR_BODY_LENGTH`
const MAX_SIMILAR_BODY_LENGTH: usize = 200;
/// `Topic::SIMILAR_TOPIC_SEARCH_LIMIT`
const SIMILAR_TOPIC_SEARCH_LIMIT: i64 = 10;
/// `Topic::SIMILAR_TOPIC_LIMIT`
const SIMILAR_TOPIC_LIMIT: i64 = 3;
/// `Topic::SIMILAR_TOPIC_MAX_BLURB_LENGTH`
const SIMILAR_TOPIC_MAX_BLURB_LENGTH: i32 = 500;
/// `Searchable::PRIORITIES[:ignore]`
const SEARCH_PRIORITY_IGNORE: i32 = 1;
/// `Search::GroupedSearchResults::BLURB_LENGTH`
const BLURB_LENGTH: usize = 200;

/// A similar topic and the start of its first post's cooked HTML.
pub struct Similar {
    pub topic: TopicRow,
    pub blurb: String,
}

/// `Topic.count_exceeds_minimum?`
pub async fn count_exceeds_minimum(
    conn: &mut PgConnection,
    settings: &SiteSettings,
) -> Result<bool, AppError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM topics WHERE deleted_at IS NULL")
        .fetch_one(conn)
        .await?;
    Ok(count > settings.get("minimum_topics_similar")?.to_i())
}

/// Ruby's `blank?` for a string.
fn blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `Topic.similar_to(title, raw, user)`
pub async fn similar_to(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    title: &str,
    raw: Option<&str>,
) -> Result<Vec<Similar>, AppError> {
    if blank(title) {
        return Ok(Vec::new());
    }
    // Search.prepare_data segments Chinese and Japanese text.
    let locale = settings.get("default_locale")?.to_s();
    if settings.get("search_tokenize_chinese")?.truthy()
        || settings.get("search_tokenize_japanese")?.truthy()
        || locale.starts_with("zh")
        || locale.starts_with("ja")
    {
        return Err(Unsupported("CJK segmentation for similar topics").into());
    }
    let raw = raw.unwrap_or_default();
    let search_data = search_index::prepare_data(title.trim());
    if blank(&search_data) {
        return Ok(Vec::new());
    }
    let mut term = ts_query_value(&search_data, "A", true);
    if !blank(raw) {
        let head: String = raw.chars().take(MAX_SIMILAR_BODY_LENGTH).collect();
        let host = Host::from_state(state);
        let cooked =
            crate::pretty_text::cook(&host, head.trim(), &MarkdownOptions::default()).await?;
        let prepared = search_index::prepare_data(&scrub(state, settings, &cooked)?);
        if !blank(&prepared) {
            term = format!("{term} & {}", ts_query_value(&prepared, "B", true));
        }
    }
    // Search.to_tsquery(term:, joiner: "|"), the term unaccented when the
    // site ignores accents.
    let bound = if settings.get("search_ignore_accents")?.truthy() {
        "unaccent($1)"
    } else {
        "$1"
    };
    let tsquery = format!(
        "REPLACE(REGEXP_REPLACE(TO_TSQUERY('{}', {bound})::text, '<->|<\\d+>', '&', 'g')::tsquery::text, '&', '|')::tsquery",
        ts_config(&locale)
    );

    // Category.secured(guardian) with the ignore priority, and for a
    // member the categories they muted.
    let allowed = guardian.allowed_category_ids_sql(settings)?;
    let mut excluded = format!(
        "SELECT categories.id FROM categories WHERE categories.id IN ({allowed}) \
         AND categories.search_priority = {SEARCH_PRIORITY_IGNORE}"
    );
    if let Some(user_id) = guardian.user_id() {
        let default_level = i64::from(crate::categories::serializer::default_notification_level(
            settings,
        )?);
        let nesting3 = settings.get("max_category_nesting")?.to_i() == 3;
        excluded = format!(
            "{excluded} UNION {}",
            crate::hashtags::muted_category_ids_query(user_id, default_level, nesting3)
        );
    }

    // Topic.visible.listable_topics.secured(guardian) without the
    // category definitions. No plugin registers
    // similar_topic_candidate_ids here (discourse-ai's embeddings do).
    let candidate_ids: Vec<i32> = sqlx::query_scalar(&format!(
        "SELECT topics.id FROM topics \
         LEFT JOIN categories c ON topics.id = c.topic_id \
         JOIN topic_search_data s ON topics.id = s.topic_id \
         WHERE topics.deleted_at IS NULL AND topics.visible \
           AND topics.archetype <> 'private_message' \
           AND (topics.category_id IS NULL OR topics.category_id IN ({allowed})) \
           AND c.topic_id IS NULL \
           AND topics.category_id NOT IN ({excluded}) \
           AND search_data @@ {tsquery} \
         ORDER BY ts_rank(search_data, {tsquery}) DESC \
         LIMIT {SIMILAR_TOPIC_SEARCH_LIMIT}"
    ))
    .bind(&term)
    .fetch_all(&mut *conn)
    .await?;
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }

    let similarity = if blank(raw) {
        "similarity(topics.title, $2)"
    } else {
        "similarity(topics.title, $2) + similarity(p.raw, $3)"
    };
    let sql = format!(
        "SELECT topics.id, LEFT(p.cooked, {SIMILAR_TOPIC_MAX_BLURB_LENGTH}) AS blurb \
         FROM topics JOIN posts AS p ON p.topic_id = topics.id AND p.post_number = 1 \
         WHERE topics.deleted_at IS NULL AND topics.id = ANY($1) \
           AND {similarity} > 0.2 \
         ORDER BY {similarity} DESC LIMIT {SIMILAR_TOPIC_LIMIT}"
    );
    let mut query = sqlx::query_as(&sql).bind(&candidate_ids).bind(title);
    if !blank(raw) {
        query = query.bind(raw);
    }
    let ranked: Vec<(i32, Option<String>)> = query.fetch_all(&mut *conn).await?;
    let ids: Vec<i32> = ranked.iter().map(|(id, _)| *id).collect();
    let mut rows: Vec<TopicRow> = sqlx::query_as(&format!(
        "SELECT {TOPIC_COLUMNS} FROM topics WHERE topics.id = ANY($1)"
    ))
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(ranked.len());
    for (id, blurb) in ranked {
        if let Some(at) = rows.iter().position(|t| t.id == id) {
            out.push(Similar {
                topic: rows.swap_remove(at),
                blurb: blurb.unwrap_or_default(),
            });
        }
    }
    Ok(out)
}

/// `SearchIndexer::HtmlScrubber.scrub(cooked)`
fn scrub(state: &AppState, settings: &SiteSettings, cooked: &str) -> Result<String, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let base = BaseUrls {
        base_path: state.config.globals.relative_url_root(),
        base_url_no_prefix: urls.base_url_no_prefix()?,
        store: crate::file_store::FileStore::for_site(&state.config, settings)?,
    };
    Ok(search_index::scrub(cooked, &base))
}

/// `SimilarTopic#blurb`: `GroupedSearchResults.blurb_for(cooked:)`, the
/// cooked HTML scrubbed, its media links named, truncated and sanitized.
fn blurb_for(state: &AppState, settings: &SiteSettings, cooked: &str) -> Result<String, AppError> {
    let text = search_index::clean_post_raw_data(&scrub(state, settings, cooked)?)?;
    Ok(blurb::blurb_for(&text, None, BLURB_LENGTH))
}

/// `render_serialized(topics, SimilarTopicSerializer, root:
/// :similar_topics, rest_serializer: true)`: each topic side-loaded as a
/// TopicListItemSerializer with its posters' users and groups.
pub async fn serialize(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    similar: &[Similar],
) -> Result<Value, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let base_url = urls.base_url()?;
    let topics: Vec<TopicRow> = similar.iter().map(|s| s.topic.clone()).collect();
    let mut list = TopicListSerializer {
        conn: &mut *conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        more_topics_url: None,
        category_id: None,
        group_id: None,
        prefetched: Default::default(),
    };
    let lookup = list.user_lookup(&topics).await?;
    let logo_small_url = list.logo_small_url().await?;
    let group_ids: Vec<i32> = lookup
        .values()
        .flat_map(|u| [u.primary_group_id, u.flair_group_id])
        .flatten()
        .collect();
    let groups = crate::groups::load(&mut *list.conn, &group_ids).await?;
    let tagging = settings.get("tagging_enabled")?.truthy();

    let mut users: Vec<Value> = Vec::new();
    let mut seen_users: Vec<i32> = Vec::new();
    let mut primary_groups: Vec<Value> = Vec::new();
    let mut flair_groups: Vec<Value> = Vec::new();
    let mut seen_primary: Vec<i32> = Vec::new();
    let mut seen_flair: Vec<i32> = Vec::new();
    let mut topics_json: Vec<Value> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    let mut any_poster = false;
    for s in similar {
        let t = &s.topic;
        let posters = list.posters(t, &lookup);
        for poster in &posters {
            any_poster = true;
            if !seen_users.contains(&poster.user.id) {
                seen_users.push(poster.user.id);
                users.push(list.serialize_user(
                    &poster.user,
                    logo_small_url.as_deref(),
                    &groups,
                )?);
            }
            if let Some(g) = poster.user.primary_group_id.and_then(|id| groups.get(&id))
                && !seen_primary.contains(&g.id)
            {
                seen_primary.push(g.id);
                primary_groups.push(g.primary_json());
            }
            if let Some(g) = poster.user.flair_group_id.and_then(|id| groups.get(&id))
                && !seen_flair.contains(&g.id)
            {
                seen_flair.push(g.id);
                flair_groups.push(g.flair_json()?);
            }
        }
        topics_json.push(
            list.serialize_topic(t, &posters, tagging, Mode::Bare)
                .await?,
        );
        let Some(slug) = &t.slug else {
            return Err(Unsupported("topics without a stored slug (Slug.for)").into());
        };
        let mut item = Map::new();
        item.insert("id".into(), json!(t.id));
        item.insert("blurb".into(), json!(blurb_for(state, settings, &s.blurb)?));
        item.insert("created_at".into(), json!(time_json(t.created_at)));
        item.insert("url".into(), json!(format!("{base_url}/t/{slug}/{}", t.id)));
        item.insert("topic_id".into(), json!(t.id));
        items.push(Value::Object(item));
    }

    // AMS side-loads first: the topics, then what their posters embed.
    let mut out = Map::new();
    if !topics_json.is_empty() {
        out.insert("topics".into(), Value::Array(topics_json));
    }
    if any_poster {
        out.insert("users".into(), Value::Array(users));
        out.insert("primary_groups".into(), Value::Array(primary_groups));
        out.insert("flair_groups".into(), Value::Array(flair_groups));
    }
    out.insert("similar_topics".into(), Value::Array(items));
    out.insert("__rest_serializer".into(), json!("1"));
    Ok(Value::Object(out))
}
