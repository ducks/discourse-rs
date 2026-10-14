//! GET /search-menu: the search menu's results area for the state
//! static/js/search-menu.js keeps (crate::search_menu_view), as the Ember
//! client renders it after SearchMenu#perform: the assistant's suggestions
//! (categories from the site's list with `Category.search`, tags as
//! /tags/filter/search finds them, users as /u/search/users and the
//! client's organizeResults rank them), or the term searched as
//! /search/query searches it (routes::search::run_query, its log and rate
//! limits included), or the initial options. The answer is JSON: the
//! markup, and whether it holds topics (the menu's `includesTopics`).

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};

use super::search::{Peer, QueryParams, run_query};
use crate::search_menu_view::{
    Assist, Menu, PageContext, Suggestion, category_search, matches_suggestions, organize_users,
    result_types, skip_user_search, suggestion_shortcuts,
};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list_view::{ListCategory, ListContext, ListSettings};
use crate::{AppError, AppState, Unsupported};

/// `DEFAULT_TYPE_FILTER`
const EXCLUDE_TOPICS: &str = "exclude_topics";

#[derive(Deserialize, Default)]
pub struct MenuParams {
    #[serde(default)]
    term: String,
    /// The menu's typeFilter: exclude_topics, or empty for null.
    #[serde(default)]
    type_filter: String,
    /// The page's search context.
    context_type: Option<String>,
    context_id: Option<String>,
    /// `search.inTopicContext`
    in_topic: Option<String>,
    /// `isPMInboxCleared`
    pm_cleared: Option<String>,
    /// The results shown have topics.
    topics: Option<String>,
    /// capabilities.hasKeyboard and an Apple platform, for the quick tip.
    keyboard: Option<String>,
    apple: Option<String>,
}

fn flag(v: &Option<String>) -> bool {
    v.as_deref() == Some("1")
}

/// GET /search-menu
pub async fn show(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<MenuParams>,
    headers: HeaderMap,
    Peer(peer): Peer,
) -> Result<Response, AppError> {
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    // site.can_search: the menu is there only when the viewer may search.
    if guardian.user_id().is_none() && !settings.get("allow_anonymous_search")?.truthy() {
        return Ok(super::login_required::not_logged_in(&state));
    }
    let member = guardian.user_id().is_some();
    let tagging = settings.get("tagging_enabled")?.truthy();
    let log_search_queries = settings.get("log_search_queries")?.truthy();
    let base_path = state.config.globals.relative_url_root().to_string();
    let categories = crate::topic_list_view::categories(&mut conn).await?;

    let context = match (params.context_type.as_deref(), params.context_id.as_deref()) {
        (Some("topic"), Some(id)) => id.parse().ok().map(PageContext::Topic),
        (Some("category"), Some(id)) => id.parse().ok().map(PageContext::Category),
        (Some("tag"), Some(id)) => Some(PageContext::Tag(id.to_string())),
        (Some("user"), Some(id)) => Some(PageContext::User(id.to_string())),
        (Some("private_messages"), Some(id)) => Some(PageContext::PrivateMessages(id.to_string())),
        _ => None,
    };
    let in_topic = flag(&params.in_topic) && matches!(context, Some(PageContext::Topic(_)));
    let in_pm_inbox =
        !flag(&params.pm_cleared) && matches!(context, Some(PageContext::PrivateMessages(_)));
    let exclude_topics = params.type_filter == EXCLUDE_TOPICS;
    let term = params.term.clone();
    // includesTopics before the search: the shown results' topics, or a
    // type filter.
    let includes_topics = flag(&params.topics) || !exclude_topics;

    let mut assist_categories: Vec<ListCategory> = Vec::new();
    let mut assist: Option<AssistData> = None;
    let mut results: Option<Value> = None;
    let mut invalid_term = false;
    let mut no_results = false;

    // In a topic's context only Enter searches (triggerSearch).
    let searches = !(in_topic && !includes_topics);
    let suggestion = if searches && !includes_topics {
        matches_suggestions(&term)
    } else {
        None
    };
    match suggestion {
        Some(Suggestion::Category(q)) => {
            let sorted =
                sorted_site_categories(&state, &mut conn, &settings, &guardian, &categories)
                    .await?;
            let refs: Vec<&ListCategory> = sorted.iter().collect();
            let found: Vec<ListCategory> =
                category_search(&refs, &q, 5).into_iter().cloned().collect();
            let tags = if found.len() != 5 && tagging {
                tag_search(&mut conn, &settings, &guardian, &q, 5 - found.len() as i64).await?
            } else {
                Vec::new()
            };
            assist_categories = found;
            assist = Some(AssistData::Categories(tags));
            no_results = true;
        }
        Some(Suggestion::Username(q)) => {
            let users = user_suggestions(&state, &mut conn, &settings, &guardian, &q).await?;
            no_results = true;
            if !users.is_empty() {
                assist = Some(AssistData::Users(users));
            }
        }
        Some(Suggestion::Shortcut(keyword)) => {
            let mut plugins = Vec::new();
            if settings.get("poll_enabled")?.truthy() {
                plugins.push("poll");
            }
            if settings.get("solved_enabled")?.truthy() {
                plugins.push("solved");
            }
            if settings.get("topic_voting_enabled")?.truthy() {
                plugins.push("voting");
            }
            assist = Some(AssistData::Shortcuts(
                keyword,
                suggestion_shortcuts(&term, member, tagging, &plugins),
            ));
            no_results = true;
        }
        None if !searches => {}
        None if term.is_empty() => {}
        None if !valid_term(&term, &settings)? => {
            invalid_term = true;
            no_results = true;
        }
        None => {
            // searchForTerm: the context only in a topic or the inbox.
            let (context_type, context_id) = match &context {
                Some(PageContext::Topic(id)) if in_topic => {
                    (Some("topic".to_string()), Some(id.to_string()))
                }
                Some(PageContext::PrivateMessages(u)) if in_pm_inbox => {
                    (Some("private_messages".to_string()), Some(u.clone()))
                }
                _ => (None, None),
            };
            let query = QueryParams {
                term: Some(term.clone()),
                type_filter: (!params.type_filter.is_empty()).then(|| params.type_filter.clone()),
                context_type,
                context_id_nested: context_id,
                ..Default::default()
            };
            match run_query(&state, &guardian, &query, &headers, peer).await? {
                Ok(doc) => {
                    // The group type draws a flair with DAvatarFlair.
                    if doc["groups"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|g| g["flair_url"].as_str().is_some_and(|f| !f.is_empty()))
                    {
                        return Err(Unsupported("a group with a flair in the search menu").into());
                    }
                    no_results = result_types(&doc, in_topic).is_empty();
                    results = Some(doc);
                }
                Err(refused) => return Ok(refused),
            }
        }
    }

    let topics = results
        .as_ref()
        .is_some_and(|r| r["posts"].as_array().is_some_and(|p| !p.is_empty()));
    let recent = if member && log_search_queries {
        super::recent_searches::terms(&mut conn, guardian.user_id().unwrap_or_default())
            .await?
            .into_iter()
            .map(|(term, _)| term)
            .collect()
    } else {
        Vec::new()
    };

    let cx = ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: None,
        settings: ListSettings::load(&settings)?,
    };
    let assist = match &assist {
        Some(AssistData::Categories(tags)) => Some(Assist::Categories {
            categories: assist_categories.iter().collect(),
            tags: tags.clone(),
        }),
        Some(AssistData::Users(users)) => Some(Assist::Users(users.clone())),
        Some(AssistData::Shortcuts(keyword, shortcuts)) => Some(Assist::Shortcuts {
            keyword: keyword.clone(),
            shortcuts: shortcuts.clone(),
        }),
        None => None,
    };
    let mut menu = Menu {
        cx: &cx,
        mobile: crate::mobile::mobile_view(),
        member,
        log_search_queries,
        tagging,
        term: &term,
        context: context.as_ref(),
        in_topic,
        in_pm_inbox,
        search_topics: topics || !exclude_topics,
        invalid_term,
        no_results,
        recent: &recent,
        quick_tip: 0,
        keyboard: flag(&params.keyboard),
        apple: flag(&params.apple),
    };
    menu.quick_tip = rand::thread_rng().gen_range(0..menu.quick_tips().len().max(1));
    let html = menu.results(assist.as_ref(), results.as_ref());
    let log_id = results
        .as_ref()
        .and_then(|r| r["grouped_search_result"]["search_log_id"].as_i64());
    Ok(Json(json!({
        "html": html,
        "topics": topics,
        "search_log_id": log_id,
        "full_search_url": menu.full_search_url(false),
    }))
    .into_response())
}

enum AssistData {
    Categories(Vec<String>),
    Users(Vec<Value>),
    Shortcuts(String, Vec<String>),
}

/// lib/search's isValidSearchTerm: long enough, or using a filter that
/// bypasses the length.
fn valid_term(term: &str, settings: &SiteSettings) -> Result<bool, AppError> {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    let lower = trimmed.to_lowercase();
    let bypass = lower == "l"
        || lower == "r"
        || [
            "order:",
            "category:",
            "categories:",
            "tag:",
            "tags:",
            "before:",
            "after:",
            "status:",
            "user:",
            "group:",
            "badge:",
            "in:",
            "with:",
            "#",
            "@",
        ]
        .iter()
        .any(|k| lower.contains(k));
    let min = settings.get("min_search_term_length")?.to_i();
    Ok(bypass || trimmed.chars().count() as i64 >= min)
}

/// `Site#sortedCategories`: the viewer's categories by topic count (a
/// stable sort), each parent followed by its children.
async fn sorted_site_categories(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    all: &HashMap<i64, ListCategory>,
) -> Result<Vec<ListCategory>, AppError> {
    let mut site = crate::categories::Categories {
        conn: &mut *conn,
        settings,
        i18n: &state.i18n,
        guardian,
        base_path: state.config.globals.relative_url_root(),
        topic_url_via_slug: true,
    }
    .for_site()
    .await?;
    site.sort_by_key(|c| std::cmp::Reverse(c["topic_count"].as_i64().unwrap_or(0)));
    fn add(out: &mut Vec<i64>, all: &[Value], parent: Option<i64>) {
        for c in all
            .iter()
            .filter(|c| c["parent_category_id"].as_i64() == parent)
        {
            if let Some(id) = c["id"].as_i64() {
                out.push(id);
                add(out, all, Some(id));
            }
        }
    }
    let mut ids = Vec::new();
    add(&mut ids, &site, None);
    Ok(ids
        .into_iter()
        .filter_map(|id| all.get(&id).cloned())
        .collect())
}

/// Tags::Search for the menu (`q`, `limit`, no category or selected
/// tags): DiscourseTagging.filter_allowed_tags over the visible tags whose
/// name contains the cleaned term, the exact name first, then by topic
/// count and name; by popularity without a term.
async fn tag_search(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    q: &str,
    limit: i64,
) -> Result<Vec<String>, AppError> {
    if settings.get("content_localization_enabled")?.truthy() {
        return Err(Unsupported("tag search over localized tags").into());
    }
    let max = settings.get("max_tag_search_results")?.to_i();
    let limit = limit.min(max);
    let term = crate::web_hooks::admin::clean_tag(settings, q)?;
    let count = if guardian.is_staff()
        || settings
            .get("include_secure_categories_in_tag_counts")?
            .truthy()
    {
        "staff_topic_count"
    } else {
        "public_topic_count"
    };
    let visible = if guardian.is_admin() {
        "TRUE".to_string()
    } else {
        crate::tags::visible_tags_where(guardian, settings)?
    };
    let (condition, order) = if term.is_empty() {
        ("TRUE".to_string(), format!("tags.{count} DESC, tags.name"))
    } else {
        (
            "position(LOWER($1) IN LOWER(tags.name)) <> 0".to_string(),
            format!("lower(tags.name) = lower($1) DESC, tags.{count} DESC, tags.name"),
        )
    };
    let sql = format!(
        "SELECT tags.name FROM tags WHERE ({visible}) AND ({condition}) ORDER BY {order} LIMIT $2"
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(&term)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?)
}

/// userSearch({ includeGroups: true, limit: 6, term | lastSeenUsers })
/// with organizeResults: the users of the ranked users and groups.
async fn user_suggestions(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &crate::guardian::Guardian,
    q: &str,
) -> Result<Vec<Value>, AppError> {
    use crate::user_search::{Options, Refused, UserSearch};
    let last_seen = q.is_empty();
    if !last_seen && skip_user_search(q) {
        return Ok(Vec::new());
    }
    let options = Options {
        topic_id: None,
        category_id: None,
        prioritized_user_id: None,
        topic_allowed_users: false,
        groups: None,
        include_staged_users: false,
        last_seen_users: last_seen,
        can_review: false,
        limit: Some(6),
    };
    let mut search = match UserSearch::new(&mut *conn, settings, guardian, q, options).await? {
        Ok(search) => search,
        Err(Refused::NotFound | Refused::InvalidAccess) => return Ok(Vec::new()),
    };
    let ids = search.search_ids().await?;
    let urls = crate::url::Urls {
        config: &state.config,
        settings,
    };
    let users = crate::user_search::found_users(&mut *conn, &urls, guardian, &ids).await?;
    let groups: Vec<Value> = if guardian.user_id().is_some() && !q.is_empty() {
        let scope = crate::user_search::visible_groups_where(guardian);
        let sql = crate::user_search::search_groups_sql(&scope, q);
        let rows: Vec<(String, Option<String>)> =
            sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
        rows.into_iter()
            .map(|(name, full_name)| json!({ "name": name, "full_name": full_name }))
            .collect()
    } else {
        Vec::new()
    };
    let term = (!q.is_empty()).then_some(q);
    Ok(organize_users(users, &groups, term, 6))
}
