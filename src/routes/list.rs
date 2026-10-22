//! Port of app/controllers/list_controller.rb (latest) with
//! lib/topic_query_params.rb.

use askama::Template;
use axum::Json;
use axum::extract::{Path, Query, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::category::Category;
use crate::guardian::Guardian;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::topic_list::TopicListSerializer;
use crate::topic_query::{Filter, Options, TopicQuery};
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// A list request's query: the params read directly, and every pair for
/// the TopicQuery options read by name (`build_topic_list_options`).
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(from = "Vec<(String, String)>")]
pub struct ListParams {
    pub(super) page: Option<String>,
    pub(super) order: Option<String>,
    pub(super) ascending: Option<String>,
    /// top lists only; not a TopicQuery option, so never echoed unless given
    pub(super) period: Option<String>,
    /// Every query pair, in order, repeated keys and all.
    pub(super) pairs: Vec<(String, String)>,
}

impl From<Vec<(String, String)>> for ListParams {
    fn from(pairs: Vec<(String, String)>) -> Self {
        let last = |key: &str| {
            pairs
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        ListParams {
            page: last("page"),
            order: last("order"),
            ascending: last("ascending"),
            period: last("period"),
            pairs,
        }
    }
}

/// `TopicQuery.public_valid_options` in its order, the plugins' custom
/// filters (discourse-assign's assigned, discourse-events'
/// order_by_event_date, discourse-solved's solved) after them.
const PUBLIC_OPTIONS: [&str; 26] = [
    "page",
    "per_page",
    "before",
    "bumped_before",
    "topic_ids",
    "category",
    "order",
    "ascending",
    "min_posts",
    "max_posts",
    "status",
    "filter",
    "state",
    "search",
    "q",
    "f",
    "subset",
    "group_name",
    "tags",
    "match_all_tags",
    "no_subcategories",
    "no_tags",
    "exclude_tag",
    "assigned",
    "order_by_event_date",
    "solved",
];

/// The params ListController#page_params carries into the next page's
/// URL besides the list options.
const PAGE_PARAMS: [&str; 4] = ["username", "group_name", "groupname", "period"];

/// The list options TagsController's lists handle, the only ones they
/// take (TagsController#build_topic_list_options re-reads the rest raw).
pub(super) const TAG_LIST_OPTIONS: [&str; 10] = [
    "page",
    "per_page",
    "order",
    "ascending",
    "state",
    "subset",
    "tags",
    "match_all_tags",
    "no_subcategories",
    "no_tags",
];

/// TagsController#build_topic_list_options merges topic_ids, category,
/// order, ascending, min_posts, max_posts, status, filter, state, search
/// and q in raw, blank or not, and turns match_all_tags=false into an any
/// tag list: only the options its lists port are taken, and those as
/// ListController takes them.
pub(super) fn refuse_unported_tag_list_params(params: &ListParams) -> Result<(), Unsupported> {
    let raw = rack_params(&params.pairs)?;
    for (key, value) in &raw {
        if PUBLIC_OPTIONS.contains(&key.as_str()) && !TAG_LIST_OPTIONS.contains(&key.as_str()) {
            return Err(Unsupported(
                "list filters on tag lists beyond page, per_page, order, ascending, state, subset and tags",
            ));
        }
        if matches!(key.as_str(), "order" | "ascending" | "state") && !value.present() {
            return Err(Unsupported(
                "a blank order, ascending or state on tag lists",
            ));
        }
        if key == "match_all_tags" && value.as_str() == Some("false") {
            return Err(Unsupported("match_all_tags=false on tag lists"));
        }
    }
    Ok(())
}

/// A query param as Rack parses it: `k=v` a string, `k[]=v` an array,
/// `k[x]=v` (or deeper) a hash.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Param {
    Str(String),
    Arr(Vec<String>),
    Hash,
}

impl Param {
    /// `present?`
    fn present(&self) -> bool {
        match self {
            Param::Str(s) => !crate::ruby::is_blank(s),
            Param::Arr(a) => !a.is_empty(),
            Param::Hash => true,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Param::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// Rack's nested params for the list's keys. A plain `k` overwrites what
/// came before; `k[]` after a string or hash, or `k[x]` after a string or
/// array, is Rack's ParameterTypeError, and an array of hashes is not
/// ported.
pub(super) fn rack_params(
    pairs: &[(String, String)],
) -> Result<std::collections::BTreeMap<String, Param>, Unsupported> {
    let mut out = std::collections::BTreeMap::new();
    for (key, value) in pairs {
        let (name, rest) = match key.find('[') {
            Some(i) => (&key[..i], &key[i..]),
            None => (key.as_str(), ""),
        };
        if !PUBLIC_OPTIONS.contains(&name) && !PAGE_PARAMS.contains(&name) && name != "tag_name" {
            continue;
        }
        match rest {
            "" => {
                out.insert(name.to_string(), Param::Str(value.clone()));
            }
            "[]" => match out
                .entry(name.to_string())
                .or_insert_with(|| Param::Arr(Vec::new()))
            {
                Param::Arr(values) => values.push(value.clone()),
                _ => {
                    return Err(Unsupported(
                        "a list param given both as a value and an array",
                    ));
                }
            },
            r if r.starts_with("[]") => {
                return Err(Unsupported("a list param holding an array of hashes"));
            }
            _ => match out.entry(name.to_string()).or_insert(Param::Hash) {
                Param::Hash => {}
                _ => return Err(Unsupported("a list param given both as a value and a hash")),
            },
        }
    }
    Ok(out)
}

/// The request's list params after `build_topic_list_options`' first
/// step: a present `tag_name` goes in front of `tags`, which becomes an
/// array without repeats.
fn list_params(
    params: &ListParams,
) -> Result<std::collections::BTreeMap<String, Param>, Unsupported> {
    let mut raw = rack_params(&params.pairs)?;
    if let Some(tag_name) = raw.get("tag_name").filter(|p| p.present()) {
        let Param::Str(tag_name) = tag_name else {
            return Err(Unsupported("a tag_name list param that is not a string"));
        };
        let mut tags = vec![tag_name.clone()];
        match raw.get("tags") {
            None => {}
            Some(Param::Str(s)) => tags.push(s.clone()),
            Some(Param::Arr(a)) => tags.extend(a.iter().cloned()),
            Some(Param::Hash) => return Err(Unsupported("tag_name with a tags hash")),
        }
        let mut seen = std::collections::HashSet::new();
        tags.retain(|t| seen.insert(t.clone()));
        raw.insert("tags".to_string(), Param::Arr(tags));
    }
    Ok(raw)
}

/// Discourse::InvalidParameters from a list's params: JSON as Rails
/// renders it; the HTML error page is not ported, so plain text.
pub(super) fn invalid_list_params(state: &AppState, message: &str, json: bool) -> Response {
    if json {
        super::search::invalid_parameters(state, message)
    } else {
        (StatusCode::BAD_REQUEST, message.to_string()).into_response()
    }
}

/// `build_topic_list_options` + `TopicQuery.validate?`: a bad value is
/// Discourse::InvalidParameters (its message the inner error).
pub(super) fn build_options(
    params: &ListParams,
    settings: &SiteSettings,
) -> Result<Result<Options, String>, Unsupported> {
    build_options_for(params, settings, false)
}

/// build_options, for a tag list when `tag_route`: the route's tag_name
/// puts `tags` in an array, whatever the query gave, so it is always
/// valid there (TagsController reads the tags itself).
pub(super) fn build_options_for(
    params: &ListParams,
    settings: &SiteSettings,
    tag_route: bool,
) -> Result<Result<Options, String>, Unsupported> {
    let mut raw = list_params(params)?;
    if tag_route {
        raw.remove("tags");
    }
    if raw.get("assigned").is_some_and(Param::present) {
        return Err(Unsupported("discourse-assign's assigned list filter"));
    }
    for key in PUBLIC_OPTIONS {
        if let Some(value) = raw.get(key).filter(|p| p.present())
            && !valid(key, value, settings)
        {
            return Ok(Err(key.to_string()));
        }
    }
    check_next_url_params(&raw, settings)?;
    Ok(Ok(options_from(&raw, settings)))
}

/// `TopicQuery.validators` for a present value; options without one
/// take anything.
fn valid(key: &str, value: &Param, settings: &SiteSettings) -> bool {
    let int = |p: &Param| -> Option<i128> {
        let s = p.as_str()?;
        let digits = s.strip_prefix('-').unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // Past i128 is past every range below.
        Some(s.parse::<i128>().unwrap_or(i128::MAX))
    };
    let between = |p: &Param, lo: i128, hi: i128| int(p).is_some_and(|n| (lo..=hi).contains(&n));
    const PG_MAX_INT: i128 = 2_147_483_647;
    match key {
        "page" => {
            let max = settings
                .get("max_topic_query_page_param")
                .map(|v| v.to_i())
                .unwrap_or(2000);
            between(value, 0, i128::from(max))
        }
        "per_page" => between(value, 1, 100),
        "before" | "bumped_before" | "min_posts" | "max_posts" => between(value, 0, PG_MAX_INT),
        "topic_ids" | "tags" => matches!(value, Param::Str(_) | Param::Arr(_)),
        "category" | "order" | "status" | "filter" | "state" | "search" | "q" | "f" | "subset"
        | "group_name" | "exclude_tag" => matches!(value, Param::Str(_)),
        "ascending" | "match_all_tags" | "no_subcategories" | "no_tags" => {
            matches!(value.as_str(), Some("true") | Some("false"))
        }
        _ => true,
    }
}

/// The validated params as TopicQuery options.
fn options_from(
    raw: &std::collections::BTreeMap<String, Param>,
    settings: &SiteSettings,
) -> Options {
    let get = |key: &str| raw.get(key).filter(|p| p.present());
    let string = |key: &str| get(key).and_then(Param::as_str).map(str::to_string);
    let number = |key: &str| get(key).and_then(Param::as_str).map(crate::ruby::to_i);
    let flag = |key: &str| string(key).as_deref() == Some("true");
    let mut options = Options {
        no_definitions: !settings
            .get("show_category_definitions_in_topic_lists")
            .map(|v| v.truthy())
            .unwrap_or(false),
        ..Options::default()
    };
    options.page = number("page").unwrap_or(0);
    options.per_page = number("per_page");
    options.before = number("before");
    options.bumped_before = number("bumped_before");
    options.category = string("category");
    // no_definitions only when `params[:category].blank?`.
    if options.category.is_some() {
        options.no_definitions = false;
    }
    options.order = string("order");
    options.ascending = flag("ascending");
    options.min_posts = number("min_posts");
    options.max_posts = number("max_posts");
    options.status = string("status");
    options.filter = string("filter").or_else(|| string("f"));
    options.state = string("state");
    options.search = string("search");
    options.subset = string("subset");
    match get("tags") {
        Some(Param::Str(s)) => {
            // filter_by_tags splits a string on whitespace; remove_muted_tags
            // takes its first character.
            options.tags = s.split_whitespace().map(str::to_string).collect();
            options.first_tag = s.chars().next().map(String::from);
        }
        Some(Param::Arr(a)) => {
            options.tags = a.clone();
            options.first_tag = a.first().cloned();
        }
        _ => {}
    }
    options.match_all_tags = flag("match_all_tags");
    options.no_subcategories = flag("no_subcategories");
    options.no_tags = flag("no_tags");
    options.exclude_tag = string("exclude_tag");
    options.solved = string("solved");
    // param_to_integer_list(:topic_ids), present or not.
    options.topic_ids = match raw.get("topic_ids") {
        Some(Param::Str(s)) => Some(
            crate::ruby::split(s, ",")
                .into_iter()
                .map(crate::ruby::to_i)
                .collect(),
        ),
        Some(Param::Arr(a)) => Some(a.iter().map(|s| crate::ruby::to_i(s)).collect()),
        _ => None,
    };
    options
}

/// `construct_url_with(:next, list_opts)`: the list's path with the list
/// options (as the request gave them, but the route's own) and
/// page_params, page + 1, as Hash#to_query writes them (each pair
/// escaped, sorted). `username_in_path` for the routes whose path holds
/// the username.
pub(super) fn next_url(
    list_path: &str,
    params: &ListParams,
    options: &Options,
    username_in_path: bool,
) -> String {
    // The params were parsed once already (build_options).
    let raw = list_params(params).unwrap_or_default();
    let mut pairs: Vec<String> = Vec::new();
    let mut push = |key: &str, value: &Param| match value {
        Param::Str(s) => pairs.push(format!(
            "{}={}",
            crate::ruby::cgi_escape(key),
            crate::ruby::cgi_escape(s)
        )),
        Param::Arr(values) if !values.is_empty() => pairs.push(
            values
                .iter()
                .map(|v| {
                    format!(
                        "{}={}",
                        crate::ruby::cgi_escape(&format!("{key}[]")),
                        crate::ruby::cgi_escape(v)
                    )
                })
                .collect::<Vec<_>>()
                .join("&"),
        ),
        _ => {}
    };
    for key in PUBLIC_OPTIONS {
        let value = match key {
            "page" => continue,
            // The route's category is in the path.
            "category" if options.category_id.is_some() => continue,
            "per_page" => match (raw.get(key).filter(|p| p.present()), options.per_page) {
                (Some(p), _) => p.clone(),
                (None, Some(n)) => Param::Str(n.to_string()),
                (None, None) => continue,
            },
            "no_subcategories" if options.no_subcategories => Param::Str("true".into()),
            "topic_ids" => match &options.topic_ids {
                Some(ids) => Param::Arr(ids.iter().map(|id| id.to_string()).collect()),
                None => continue,
            },
            // page_params' group_name wins; the same value.
            "group_name" => continue,
            _ => match raw.get(key).filter(|p| p.present()) {
                Some(p) => p.clone(),
                None => continue,
            },
        };
        push(key, &value);
    }
    if options.no_definitions {
        push("no_definitions", &Param::Str("true".into()));
    }
    for key in PAGE_PARAMS {
        if key == "username" && username_in_path {
            continue;
        }
        if let Some(value) = raw.get(key).filter(|p| p.present()) {
            push(key, value);
        }
    }
    push("page", &Param::Str((options.page + 1).to_string()));
    pairs.sort();
    format!("{list_path}?{}", pairs.join("&"))
}

/// Refusals for what build_options takes but next_url can't write as
/// Rails does: page_params' values that aren't strings, and those
/// unicode_usernames encodes first.
fn check_next_url_params(
    raw: &std::collections::BTreeMap<String, Param>,
    settings: &SiteSettings,
) -> Result<(), Unsupported> {
    let unicode = settings
        .get("unicode_usernames")
        .map_err(|_| Unsupported("unicode_usernames"))?
        .truthy();
    for key in PAGE_PARAMS {
        match raw.get(key).filter(|p| p.present()) {
            None => {}
            Some(Param::Str(s)) => {
                if unicode
                    && key != "period"
                    && !s
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                {
                    return Err(Unsupported(
                        "a username or group name param encoded for unicode_usernames",
                    ));
                }
            }
            Some(_) => {
                return Err(Unsupported(
                    "a username, group name or period param that is not a string",
                ));
            }
        }
    }
    for key in ["order_by_event_date", "solved"] {
        if matches!(raw.get(key), Some(Param::Hash)) {
            return Err(Unsupported("a plugin list param given as a hash"));
        }
    }
    Ok(())
}

/// What TagsController does differently from ListController when it
/// builds a list: `tags`/`no_tags`, per_page clamped to 30, the top period
/// straight from the param or top_page_default_timeframe, and the
/// next-page URL carrying `match_all_tags` and `tags[]`.
pub(super) struct TagListRequest {
    pub(super) tags: Vec<String>,
    pub(super) no_tags: bool,
}

/// The /latest document, shared by the JSON and HTML responses.
pub(super) async fn list_document(
    state: &AppState,
    guardian: &Guardian,
    params: &ListParams,
    category: Option<&Category>,
    no_subcategories: bool,
    kind: ListKind,
    list_path: &str,
) -> Result<Result<(serde_json::Value, SiteSettings), String>, AppError> {
    list_document_for(
        state,
        guardian,
        params,
        ListScope {
            category,
            no_subcategories,
            kind,
            list_path,
            tag_request: None,
        },
    )
    .await
}

/// What a list is scoped to, beyond the query params.
pub(super) struct ListScope<'a> {
    pub(super) category: Option<&'a Category>,
    pub(super) no_subcategories: bool,
    pub(super) kind: ListKind,
    pub(super) list_path: &'a str,
    pub(super) tag_request: Option<&'a TagListRequest>,
}

pub(super) async fn list_document_for(
    state: &AppState,
    guardian: &Guardian,
    params: &ListParams,
    scope: ListScope<'_>,
) -> Result<Result<(serde_json::Value, SiteSettings), String>, AppError> {
    let ListScope {
        category,
        no_subcategories,
        kind,
        list_path,
        tag_request,
    } = scope;
    let t0 = std::time::Instant::now();
    let mut conn = state.pool.acquire().await?;
    let t_acquire = t0.elapsed();
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let t_settings = t0.elapsed();
    let mut options = match build_options_for(params, &settings, tag_request.is_some())? {
        Ok(o) => o,
        Err(e) => return Ok(Err(e)),
    };
    let mut for_period: Option<String> = None;
    if let ListKind::Top = kind {
        let period = match params.period.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => p.to_string(),
            None if tag_request.is_some() => settings.get("top_page_default_timeframe")?.to_s(),
            None => {
                let previous_visit_at = guardian
                    .user()
                    .and_then(|u| u.previous_visit_at)
                    .map(|at| at.and_utc());
                best_period_for(
                    &mut conn,
                    &settings,
                    previous_visit_at,
                    category.map(|c| c.id),
                )
                .await?
            }
        };
        if !crate::topic_query::PERIODS.contains(&period.as_str()) {
            return Ok(Err(
                "Invalid period. Valid periods are all, yearly, quarterly, monthly, weekly, daily"
                    .into(),
            ));
        }
        // top_#{period}: per_page defaults to topics_per_period_in_top_page and
        // is always carried in the next-page URL. Tag lists keep the default.
        if options.per_page.is_none() && tag_request.is_none() {
            options.per_page = Some(settings.get("topics_per_period_in_top_page")?.to_i());
        }
        for_period = Some(period);
    }
    // no_definitions only when `params[:category].blank? && filter == :latest`.
    if kind != ListKind::Latest {
        options.no_definitions = false;
    }
    if let Some(c) = category {
        // set_category puts the id in params[:category], which is why
        // no_definitions is never set for category lists.
        // The category's routes merge `category: id` over the param, and
        // the `/none` ones `no_subcategories: true` too.
        options.category_id = Some(c.id);
        options.category = Some(c.id.to_string());
        options.no_definitions = false;
        options.no_subcategories |= no_subcategories;
    }
    if let Some(t) = tag_request {
        options.no_definitions = false;
        options.no_subcategories |= no_subcategories;
        options.per_page = options.per_page.map(|p| p.clamp(1, 30));
        options.tags = t.tags.clone();
        options.first_tag = t.tags.first().cloned();
        // `options[:match_all_tags] ||= true`
        options.match_all_tags = true;
        options.no_tags |= t.no_tags;
    }
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };

    let mut query = TopicQuery {
        conn: &mut conn,
        settings: &settings,
        guardian,
        options: options.clone(),
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    };
    let t_query0 = t0.elapsed();
    let list = query.list(kind.filter(for_period.as_deref())).await?;
    // `@options[:category_id]`, the category param resolved by default_results.
    let category_id = query.options.category_id;

    let t_query = t0.elapsed();
    let more = match tag_request {
        Some(t) => super::tags::next_url(list_path, params, &options, t),
        None => next_url(list_path, params, &options, false),
    };
    let mut json = TopicListSerializer {
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        more_topics_url: Some(more),
        category_id,
        group_id: None,
        prefetched: Default::default(),
    }
    .serialize(&list)
    .await?;
    if let Some(period) = &for_period {
        // TopicList#for_period, emitted after more_topics_url.
        if let Some(list) = json["topic_list"].as_object_mut() {
            let entries: Vec<(String, serde_json::Value)> =
                list.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            list.clear();
            for (k, v) in entries {
                list.insert(k.clone(), v);
                let has_more = list.contains_key("more_topics_url");
                if (k == "filter" && !has_more) || k == "more_topics_url" {
                    list.insert("for_period".into(), serde_json::json!(period));
                }
            }
        }
    }
    // RUST_LOG=discourse_rs=debug shows where a list request spends its time.
    tracing::debug!(
        acquire = ?t_acquire,
        settings = ?(t_settings - t_acquire),
        query = ?(t_query - t_query0),
        serialize = ?(t0.elapsed() - t_query),
        total = ?t0.elapsed(),
        "list phases"
    );
    Ok(Ok((json, settings)))
}

/// GET /latest.json
pub async fn latest_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
) -> Result<Response, AppError> {
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    match list_document(
        &state,
        &guardian,
        &params,
        None,
        false,
        ListKind::Latest,
        &list_path,
    )
    .await?
    {
        Ok((json, _)) => Ok(Json(json).into_response()),
        Err(message) => Ok(invalid_list_params(&state, &message, true)),
    }
}

/// GET /: config/routes.rb's root routes, each under HomePageConstraint,
/// which picks the request's current_homepage: list#<filter> for
/// Discourse.filters (discourse-topic-voting's votes among them),
/// categories#index, finish_installation#index while has_login_hint,
/// home_page#custom and home_page#blank. The routes take JSON as well as
/// HTML, by `?format=json` or the Accept header.
pub async fn front_page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let homepage = {
        let mut conn = state.pool.acquire().await?;
        let settings =
            SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
        // HomePageConstraint: only finish_installation matches meanwhile.
        if settings.get("has_login_hint")?.truthy() {
            return Err(Unsupported("the finish installation page").into());
        }
        crate::homepage::current(&mut conn, &settings, guardian.user_id()).await?
    };
    // Another format is Rails' 406 page, or markdown, neither ported.
    if uri
        .query()
        .into_iter()
        .flat_map(|q| q.split('&'))
        .filter_map(|p| p.strip_prefix("format="))
        .any(|f| f != "json" && f != "html")
    {
        return Err(Unsupported("the front page in a format other than json or html").into());
    }
    let json = super::wants_json(uri.query(), &headers);
    if homepage == "categories" {
        let params = match Query::<CategoriesParams>::try_from_uri(&uri) {
            Ok(Query(params)) => params,
            Err(rejection) => return Ok(rejection.into_response()),
        };
        return categories_response(
            state,
            guardian,
            params,
            json,
            headers,
            (!json).then_some(uri),
        )
        .await;
    }
    let Some(kind) = ListKind::from_name(&homepage) else {
        return Err(match homepage.as_str() {
            // The theme's connector for the custom-homepage outlet.
            "custom" => Unsupported("a theme's custom homepage"),
            // Only for an anonymous request while login is required, which
            // login_required::gate answers first.
            "blank" => Unsupported("the blank homepage"),
            // top_menu must hold latest, which every viewer may have, so
            // HomepageHelper always finds one.
            _ => Unsupported("a front page without a homepage route"),
        }
        .into());
    };
    let params = match Query::<ListParams>::try_from_uri(&uri) {
        Ok(Query(params)) => params,
        Err(rejection) => return Ok(rejection.into_response()),
    };
    match (kind, json) {
        (ListKind::Latest, true) => {
            latest_json(State(state), AuthGuardian(guardian), Query(params)).await
        }
        (ListKind::Latest, false) => {
            latest(
                State(state),
                AuthGuardian(guardian),
                Query(params),
                headers,
                uri,
            )
            .await
        }
        (kind, json) => {
            front_list(
                state,
                guardian,
                params,
                kind,
                json,
                headers,
                (!json).then_some(uri),
            )
            .await
        }
    }
}

/// Whether `Site#categories` holds the category: one the viewer can see
/// (can_see_serialized_category?) whose parent, if any, the viewer can see
/// too.
async fn in_site_categories(
    conn: &mut sqlx::PgConnection,
    secure: &[i32],
    id: i32,
) -> Result<bool, AppError> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        "SELECT p.id, p.read_restricted FROM categories c \
         JOIN categories p ON p.id = c.id OR p.id = c.parent_category_id \
         WHERE c.id = $1",
    )
    .bind(id)
    .fetch_all(conn)
    .await?;
    Ok(!rows.is_empty()
        && rows
            .iter()
            .all(|(id, restricted)| !restricted || secure.contains(id)))
}

/// ApplicationRoute#_collectTitleTokens: the short site description after
/// the site title on the front page.
fn front_page_description(
    uri: Option<&axum::http::Uri>,
    settings: &SiteSettings,
) -> Result<String, AppError> {
    Ok(if uri.is_some_and(|u| u.path() == "/") {
        settings.get("short_site_description")?.to_s()
    } else {
        String::new()
    })
}

/// GET /latest, and the front page when latest is the homepage: the
/// server-rendered list. The "more" link points at the HTML page.
pub async fn latest(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = crate::bus::page_position(&state.bus).await?;
    let list_path = format!("{}/latest", state.config.globals.relative_url_root());
    // Also before: topics bumped after it are news to this page.
    let live_since = crate::clock::now().timestamp_millis().to_string();
    let (json, settings) = match list_document(
        &state,
        &guardian,
        &params,
        None,
        false,
        ListKind::Latest,
        &list_path,
    )
    .await?
    {
        Ok(doc) => doc,
        Err(message) => return Ok(invalid_list_params(&state, &message, false)),
    };
    let mut conn = state.pool.acquire().await?;
    let base_path = state.config.globals.relative_url_root();
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Discovery,
    )
    .await?;
    site.bus_position = bus_position;
    let mut page = crate::html::latest_page(
        &mut conn,
        site,
        &json,
        &state.i18n,
        &settings,
        false,
        highlights_last_visited(ListKind::Latest, &params),
    )
    .await?;
    page.banner = crate::html::welcome_banner(
        &state.i18n,
        &settings,
        page.viewer.as_ref(),
        base_path,
        "latest",
    )?;
    // The front page has no title token; the short site description
    // follows the site title there.
    page.title = if uri.path() == "/" {
        crate::html::document_title(
            None,
            &page.site_title,
            &settings.get("short_site_description")?.to_s(),
        )
    } else {
        let homepage = crate::homepage::current(&mut conn, &settings, guardian.user_id()).await?;
        let token = page.chrome.topics_title_token("latest", &homepage);
        crate::html::document_title(token.as_deref(), &page.site_title, "")
    };
    page.chrome.welcome_banner = !page.banner.is_empty();
    page.breadcrumbs = crate::html::breadcrumbs(&state.i18n, &settings)?;
    page.nav = crate::html::nav_items(
        &state.i18n,
        &settings,
        base_path,
        "latest",
        page.chrome.tracking.as_ref(),
    )?;
    if page.chrome.tracking.is_some() {
        page.chrome.live_param("nav", "latest");
    }
    page.live_filter = "latest".to_string();
    page.live_since = live_since;
    // Strip `no_definitions`, which is what the JSON list carries around
    // but the HTML list applies on its own.
    page.more_url = page.more_url.map(|u| {
        u.replace("no_definitions=true&", "")
            .replace("?no_definitions=true", "")
    });
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, None, None).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_url_sorts_params_and_increments_page() {
        let p = ListParams::from(vec![("per_page".to_string(), "10".to_string())]);
        let o = Options {
            page: 2,
            per_page: Some(10),
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("/latest", &p, &o, false),
            "/latest?no_definitions=true&page=3&per_page=10"
        );
        let o = Options {
            no_definitions: true,
            ..Options::default()
        };
        assert_eq!(
            next_url("/forum/latest", &ListParams::default(), &o, false),
            "/forum/latest?no_definitions=true&page=1"
        );
    }

    fn settings() -> SiteSettings {
        let defs = crate::site_settings::Definitions::vendored().unwrap();
        SiteSettings::resolve(&defs, vec![], &crate::config::GlobalSettings::default()).unwrap()
    }

    fn params(query: &str) -> ListParams {
        ListParams::from(
            form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn rack_params_keep_the_shape_rack_gives() {
        let raw =
            rack_params(&params("tags[]=a&tags[]=b&status=x&status=y&q[k]=v&other[]=1").pairs)
                .unwrap();
        assert_eq!(raw["tags"], Param::Arr(vec!["a".into(), "b".into()]));
        assert_eq!(raw["status"], Param::Str("y".into()));
        assert_eq!(raw["q"], Param::Hash);
        assert!(!raw.contains_key("other"));
        // A plain value overwrites; an array after a value is Rack's error.
        let raw = rack_params(&params("tags[]=a&tags=b").pairs).unwrap();
        assert_eq!(raw["tags"], Param::Str("b".into()));
        assert!(rack_params(&params("tags=b&tags[]=a").pairs).is_err());
        assert!(rack_params(&params("tags[][x]=a").pairs).is_err());
    }

    #[test]
    fn invalid_values_name_the_param() {
        let s = settings();
        let invalid = |q: &str| build_options(&params(q), &s).unwrap().err();
        assert_eq!(invalid("page=-1").as_deref(), Some("page"));
        assert_eq!(invalid("page=%2B1").as_deref(), Some("page"));
        assert_eq!(invalid("per_page=101").as_deref(), Some("per_page"));
        assert_eq!(invalid("before=2147483648").as_deref(), Some("before"));
        assert_eq!(invalid("min_posts=1.5").as_deref(), Some("min_posts"));
        assert_eq!(invalid("topic_ids[a]=1").as_deref(), Some("topic_ids"));
        assert_eq!(invalid("category[]=4").as_deref(), Some("category"));
        assert_eq!(invalid("status[]=closed").as_deref(), Some("status"));
        assert_eq!(
            invalid("match_all_tags=1").as_deref(),
            Some("match_all_tags")
        );
        assert_eq!(invalid("tags[k]=v").as_deref(), Some("tags"));
        // A tag list's route tag_name makes tags an array.
        assert!(
            build_options_for(&params("tags[0]=x"), &s, true)
                .unwrap()
                .is_ok()
        );
        // Blank values are not validated.
        assert_eq!(invalid("page=&per_page=%20&before="), None);
        // Validated in TopicQuery.public_valid_options' order.
        assert_eq!(invalid("ascending=x&page=x").as_deref(), Some("page"));
    }

    #[test]
    fn options_follow_build_topic_list_options() {
        let s = settings();
        let o = build_options(
            &params(
                "tags=howto%20guide&topic_ids=3,x,&f=liked&category=general&no_subcategories=false",
            ),
            &s,
        )
        .unwrap()
        .unwrap();
        assert_eq!(o.tags, ["howto", "guide"]);
        assert_eq!(o.first_tag.as_deref(), Some("h"));
        assert_eq!(o.topic_ids, Some(vec![3, 0]));
        assert_eq!(o.filter.as_deref(), Some("liked"));
        assert_eq!(o.category.as_deref(), Some("general"));
        assert!(!o.no_definitions);
        assert!(!o.no_subcategories);
        let o = build_options(&params("tag_name=guide&tags=howto&topic_ids="), &s)
            .unwrap()
            .unwrap();
        assert_eq!(o.tags, ["guide", "howto"]);
        assert_eq!(o.first_tag.as_deref(), Some("guide"));
        assert_eq!(o.topic_ids, Some(vec![]));
        assert!(o.no_definitions);
        assert!(build_options(&params("assigned=me"), &s).is_err());
    }

    #[test]
    fn next_url_echoes_the_list_options() {
        let s = settings();
        let p = params("per_page=1&q=a%20b&tag_name=guide&tags=howto&topic_ids=1,2&username=bob");
        let o = build_options(&p, &s).unwrap().unwrap();
        assert_eq!(
            next_url("/latest", &p, &o, false),
            "/latest?no_definitions=true&page=1&per_page=1&q=a+b\
             &tags%5B%5D=guide&tags%5B%5D=howto\
             &topic_ids%5B%5D=1&topic_ids%5B%5D=2&username=bob"
        );
        assert_eq!(
            next_url("/topics/created-by/bob", &p, &o, true),
            "/topics/created-by/bob?no_definitions=true&page=1&per_page=1&q=a+b\
             &tags%5B%5D=guide&tags%5B%5D=howto\
             &topic_ids%5B%5D=1&topic_ids%5B%5D=2"
        );
        // A route's category is in its path; `/none` merges no_subcategories.
        let p = params("category=2&no_subcategories=false");
        let mut o = build_options(&p, &s).unwrap().unwrap();
        o.category_id = Some(4);
        o.no_subcategories = true;
        assert_eq!(
            next_url("/c/general/4/none", &p, &o, false),
            "/c/general/4/none?no_subcategories=true&page=1"
        );
    }

    #[test]
    fn tag_lists_refuse_the_options_they_do_not_take() {
        assert!(refuse_unported_tag_list_params(&params("page=1&tags=a&state=muted")).is_ok());
        assert!(refuse_unported_tag_list_params(&params("status=closed")).is_err());
        assert!(refuse_unported_tag_list_params(&params("topic_ids=")).is_err());
        assert!(refuse_unported_tag_list_params(&params("order=")).is_err());
        assert!(refuse_unported_tag_list_params(&params("match_all_tags=false")).is_err());
    }

    #[test]
    fn best_periods_follow_the_previous_visit() {
        use chrono::{Duration, TimeZone, Utc};
        let now = Utc.with_ymd_and_hms(2026, 10, 22, 12, 0, 0).unwrap();
        assert_eq!(best_periods_for(None, "yearly", now), ["yearly", "all"]);
        assert_eq!(best_periods_for(None, "all", now), ["all"]);
        let visit = |days: i64| Some(now - Duration::days(days));
        assert_eq!(
            best_periods_for(visit(7), "yearly", now),
            ["daily", "weekly", "monthly", "quarterly", "yearly", "all"]
        );
        // 1.week + 1.day, 1.month + 1.week, 3.months + 3.weeks,
        // 1.year + 1.month, 3.years.
        assert_eq!(
            best_periods_for(visit(9), "all", now),
            ["weekly", "monthly", "quarterly", "yearly", "all"]
        );
        assert_eq!(
            best_periods_for(visit(40), "all", now),
            ["monthly", "quarterly", "yearly", "all"]
        );
        assert_eq!(
            best_periods_for(visit(120), "all", now),
            ["quarterly", "yearly", "all"]
        );
        assert_eq!(best_periods_for(visit(400), "all", now), ["yearly", "all"]);
        assert_eq!(best_periods_for(visit(1100), "all", now), ["all"]);
    }
}

/// `/c/*category_slug_path_with_id(.json)` -> list#category_default,
/// `.../none` -> list#category_none_default (both in the category's
/// default view), and `.../l/<filter>` with or without `/none` for the
/// ported filters.
#[allow(clippy::too_many_arguments)]
pub async fn category(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Path(path): Path<String>,
    Query(params): Query<ListParams>,
    RawQuery(raw_query): RawQuery,
    headers: HeaderMap,
    uri: axum::http::Uri,
    requested: Option<axum::Extension<super::RequestedUri>>,
) -> Result<Response, AppError> {
    let (path, json) = match path.strip_suffix(".json") {
        Some(p) => (p.to_string(), true),
        None => (path.clone(), false),
    };
    // categories#show, find_by_slug, visible_groups and the editor.
    if let Some(response) =
        super::categories::dispatch(&state, &guardian, &headers, &uri, &path, json).await?
    {
        return Ok(response);
    }
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    // `.../none` (no_subcategories), `.../l/{latest,top,hot}`, or both.
    let mut rest = path.as_str();
    let mut kind = ListKind::Latest;
    let mut action = String::new();
    let mut filtered = false;
    for (name, k) in ListKind::ALL {
        let suffix = format!("/l/{name}");
        if let Some(p) = rest.strip_suffix(suffix.as_str()) {
            rest = p;
            kind = k;
            action = suffix;
            filtered = true;
            break;
        }
    }
    let none = match rest.strip_suffix("/none") {
        Some(p) => {
            rest = p;
            action = format!("/none{action}");
            true
        }
        None => false,
    };
    let slug_path = rest.to_string();
    if slug_path.contains("/l/") || slug_path.ends_with("/all") {
        return Err(crate::Unsupported("unknown category list filter").into());
    }
    if let Some(response) = ensure_logged_in(&state, &guardian, kind) {
        return Ok(response);
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let max_nesting = settings.get("max_category_nesting")?.to_i();
    let base_path = state.config.globals.relative_url_root().to_string();

    // set_category: missing or unreadable -> 404, before any redirect.
    let category = Category::find_by_slug_path_with_id(&mut conn, &slug_path, max_nesting).await?;
    let Some(category) = category else {
        // NotFound with check_permalinks: the full path looked up.
        drop(conn);
        let fullpath = super::fullpath_of(requested.as_ref().map(|r| &r.0), &uri);
        if let Some(redirect) =
            crate::permalinks::check(&state, &guardian, &headers, &fullpath).await?
        {
            return Ok(redirect);
        }
        return Ok(super::topics::not_found_response(&state));
    };
    if category.read_restricted
        && !guardian
            .secure_category_ids(&mut conn, &settings)
            .await?
            .contains(&category.id)
    {
        return Ok(super::topics::not_found_response(&state));
    }
    // slugs_do_not_match: a wrong slug path redirects, query string kept.
    let real_slug = category.full_slug(&mut conn).await?;
    if slug_path.trim_matches('/') != real_slug {
        let query = raw_query.map(|q| format!("?{q}")).unwrap_or_default();
        let format = if json { ".json" } else { "" };
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("localhost");
        let location = format!("http://{host}{base_path}/c/{real_slug}{action}{format}{query}");
        return Ok((
            StatusCode::MOVED_PERMANENTLY,
            [(header::LOCATION, location)],
        )
            .into_response());
    }
    if !json {
        // BuildCategoryRoute#model finds the category among the site's,
        // which leave out a category whose parent the viewer can't see:
        // not found there, the route goes to /404. Lazy loading finds it
        // with /categories/find instead, and titleToken then throws on
        // the missing parent, as it does for a category found whose
        // parent the site's categories leave out.
        let secure = guardian.secure_category_ids(&mut conn, &settings).await?;
        if !in_site_categories(&mut conn, &secure, category.id).await? {
            if guardian.can_lazy_load_categories(&settings)? {
                return Err(Unsupported(
                    "a lazy-loaded category whose parent the viewer can't see",
                )
                .into());
            }
            return Ok(super::user_activity::found(
                &headers,
                format!("{base_path}/404"),
            ));
        }
        if let Some(parent) = category.parent_category_id
            && !in_site_categories(&mut conn, &secure, parent).await?
        {
            return Err(Unsupported(
                "a category whose parent the site's categories leave out (Ember's titleToken throws)",
            )
            .into());
        }
    }
    drop(conn);

    // category_default_view: the category's default_view when anonymous users
    // may list it (latest, top, hot), else latest. The URL stays the
    // category_default one.
    if !filtered {
        // Only a filter the user may list; anonymous users get latest,
        // top, hot (and categories, which isn't a topic list).
        kind = match category
            .default_view
            .as_deref()
            .and_then(ListKind::from_name)
        {
            Some(k) if guardian.is_authenticated() || !k.requires_login() => k,
            _ => ListKind::Latest,
        };
    }

    let list_path = format!("{base_path}/c/{real_slug}{action}");
    let (doc, settings) = match list_document(
        &state,
        &guardian,
        &params,
        Some(&category),
        none,
        kind,
        &list_path,
    )
    .await?
    {
        Ok(doc) => doc,
        Err(message) => return Ok(invalid_list_params(&state, &message, json)),
    };
    if json {
        return Ok(Json(doc).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, &base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Category(category.id),
    )
    .await?;
    site.bus_position = bus_position;
    let mut page = crate::html::latest_page(
        &mut conn,
        site,
        &doc,
        &state.i18n,
        &settings,
        true,
        highlights_last_visited(kind, &params),
    )
    .await?;
    let heading =
        crate::html::category_heading(&mut conn, &base_path, &category, params.page.is_none())
            .await?;
    let token = page.chrome.category_title_token(
        kind.name(),
        &category.name,
        heading.parent.as_ref().map(|p| p.name.as_str()),
    );
    page.title = crate::html::document_title(Some(&token), &page.site_title, "");
    page.heading = Some(heading);
    // The category navigation: breadcrumbs, the category's pills, and a
    // member's notification level in it.
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let list_cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path: &base_path,
        now: chrono::Utc::now(),
        categories: &categories,
        expand_all_pinned: true,
        member_trust_level: None,
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    page.category_nav = true;
    page.breadcrumbs =
        crate::html::category_breadcrumbs(&list_cx, &settings, i64::from(category.id), none)?;
    let can_vote = crate::plugins::topic_voting::enabled(&settings)?
        && crate::plugins::topic_voting::category_votes(&mut conn, category.id).await?;
    page.nav = crate::html::category_nav_items(
        &state.i18n,
        &settings,
        &format!(
            "{base_path}/c/{real_slug}{}",
            if none { "/none" } else { "" }
        ),
        category.id,
        none,
        kind.name(),
        page.chrome.tracking.as_ref(),
        can_vote,
    )?;
    if let Some(user_id) = guardian.user_id() {
        let level: Option<i32> = sqlx::query_scalar(
            "SELECT notification_level FROM category_users WHERE user_id = $1 AND category_id = $2",
        )
        .bind(user_id)
        .bind(category.id)
        .fetch_optional(&mut *conn)
        .await?;
        page.category_controls = crate::html::category_notifications(
            &state.i18n,
            &base_path,
            i64::from(category.id),
            i64::from(level.unwrap_or(1)),
        );
    }
    // canonical_url "#{Discourse.base_url_no_prefix}#{.url}"
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let canonical = format!(
        "{}{}",
        urls.base_url_no_prefix()?,
        category.url(&mut conn, &base_path).await?
    );
    let description = crate::categories::description_plain_text(category.description.as_deref())?
        .map(|d| html_escape::decode_html_entities(&d).into_owned())
        .filter(|d| !d.is_empty());
    let meta = Some((category.name.clone(), description.unwrap_or_default()));
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, Some(canonical), meta).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// GET /categories(.json) -> categories#index for anonymous users.
pub async fn categories(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<CategoriesParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    categories_response(state, guardian, params, false, headers, Some(uri)).await
}

pub async fn categories_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<CategoriesParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    categories_response(state, guardian, params, true, headers, None).await
}

async fn categories_response(
    state: AppState,
    guardian: Guardian,
    params: CategoriesParams,
    json: bool,
    headers: HeaderMap,
    uri: Option<axum::http::Uri>,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let has_parent = params.parent_category_id.is_some();
    let doc = crate::category_list::CategoryList {
        secure_ids: Vec::new(),
        conn: &mut conn,
        settings: &settings,
        i18n: &state.i18n,
        guardian: &guardian,
        urls: &urls,
        include_topics_param: params.include_topics.is_some(),
        include_subcategories_param: params.include_subcategories.as_deref() == Some("true"),
        parent_category_param: params.parent_category_id.filter(|p| !p.trim().is_empty()),
        page: params
            .page
            .as_deref()
            .map(crate::ruby::to_i)
            .filter(|p| *p > 0)
            .unwrap_or(1),
    }
    .json()
    .await?;
    if json {
        return Ok(Json(doc).into_response());
    }
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Categories,
    )
    .await?;
    site.bus_position = bus_position;
    let mut page =
        crate::html::categories_page(&mut conn, &state.i18n, settings.emojis(), site, &doc).await?;
    // DiscoveryCategoriesRoute#titleToken: none when categories is the
    // homepage.
    let homepage = crate::homepage::current(&mut conn, &settings, guardian.user_id()).await?;
    let token = (homepage != "categories").then(|| page.chrome.t("filters.categories.title"));
    page.title = crate::html::document_title(
        token.as_deref(),
        &page.site_title,
        &front_page_description(uri.as_ref(), &settings)?,
    );
    let base_path = state.config.globals.relative_url_root();
    // CategoriesDisplay#style: the mobile view has its own setting.
    let mobile = crate::mobile::mobile_view();
    let style = settings.get("desktop_category_page_style")?.to_s();
    let mobile_style = settings.get("mobile_category_page_style")?.to_s();
    let created_order = style == "categories_and_latest_topics_created_date";
    let ported = if mobile {
        mobile_style == "categories_with_featured_topics" || mobile_style == "categories_only"
    } else {
        style == "categories_and_latest_topics" || created_order
    };
    if ported && !has_parent {
        // CategoriesController#fetch_topic_list: latest, topics_per_page,
        // which the mobile view's styles don't show.
        let latest = if mobile {
            serde_json::Value::Null
        } else {
            let mut latest_pairs = vec![(
                "per_page".to_string(),
                topics_per_page(&mut conn, &settings, &guardian)
                    .await?
                    .to_string(),
            )];
            if created_order {
                latest_pairs.push(("order".to_string(), "created".to_string()));
            }
            let latest_params = ListParams::from(latest_pairs);
            // list_document takes its own connection: this one is put back
            // meanwhile.
            drop(conn);
            let latest = match list_document(
                &state,
                &guardian,
                &latest_params,
                None,
                false,
                ListKind::Latest,
                &format!("{base_path}/latest"),
            )
            .await?
            {
                Ok((latest, _)) => latest,
                Err(message) => return Ok(invalid_list_params(&state, &message, false)),
            };
            conn = state.pool.acquire().await?;
            latest
        };
        let categories = crate::topic_list_view::categories(&mut conn).await?;
        let cx = crate::topic_list_view::ListContext {
            i18n: &state.i18n,
            base_path,
            now: crate::clock::now(),
            categories: &categories,
            expand_all_pinned: false,
            member_trust_level: page.viewer.as_ref().map(|v| v.trust_level),
            settings: crate::topic_list_view::ListSettings::load(&settings)?,
        };
        // The viewer's muted categories (Category#notification_level 0).
        let muted: Vec<i64> = match guardian.user_id() {
            Some(uid) => sqlx::query_scalar(
                "SELECT category_id::int8 FROM category_users WHERE user_id = $1 AND notification_level = 0",
            )
            .bind(uid)
            .fetch_all(&mut *conn)
            .await?,
            None => Vec::new(),
        };
        page.main = if mobile {
            crate::categories_view::render_mobile(
                &cx,
                &doc,
                &muted,
                mobile_style == "categories_with_featured_topics",
            )?
        } else {
            crate::categories_view::render(
                &cx,
                &doc,
                &latest,
                &muted,
                page.chrome.tracking.as_ref(),
            )?
        };
        page.nav = crate::html::nav_items(
            &state.i18n,
            &settings,
            base_path,
            "categories",
            page.chrome.tracking.as_ref(),
        )?;
        page.breadcrumbs = crate::html::breadcrumbs(&state.i18n, &settings)?;
        // DNavigation's showCategoryAdmin: the new category button (with
        // fixed_category_positions, Rails' admin dropdown, not drawn).
        if doc["category_list"]["can_create_category"] == true
            && !settings.get("fixed_category_positions")?.truthy()
        {
            // Labelled "Category" in the mobile view.
            let label = if mobile {
                "js.categories.category"
            } else {
                "js.category.create"
            };
            page.admin_controls = format!(
                "<button class=\"btn btn-icon-text btn-default\" id=\"create-category\" type=\"button\">{}<span class=\"d-button-label\">{}</span></button>",
                crate::topic_list_view::icon("plus", None),
                state.i18n.t(label).unwrap_or_default()
            );
        }
        // discovery/categories.gjs: bodyClass "categories-list"; its
        // Navigation's "navigation-categories".
        page.chrome
            .body_classes
            .push_str(" navigation-categories categories-list");
    }
    page.banner = crate::html::welcome_banner(
        &state.i18n,
        &settings,
        page.viewer.as_ref(),
        state.config.globals.relative_url_root(),
        "categories",
    )?;
    page.chrome.welcome_banner = !page.banner.is_empty();
    let uri = uri.unwrap_or_default();
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, None, None).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

#[derive(Deserialize, Default)]
pub struct CategoriesParams {
    include_topics: Option<String>,
    include_subcategories: Option<String>,
    parent_category_id: Option<String>,
    page: Option<String>,
}

/// The list conditions of TopicList#lastVisitedTopic: not top or hot, in
/// the default order (activity), descending.
pub(crate) fn highlights_last_visited(kind: ListKind, params: &ListParams) -> bool {
    !matches!(kind, ListKind::Top | ListKind::Hot)
        && params
            .order
            .as_deref()
            .is_none_or(|o| o.is_empty() || o == "activity")
        && params.ascending.as_deref() != Some("true")
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Latest,
    Top,
    Hot,
    Unread,
    New,
    Unseen,
    Read,
    Posted,
    Bookmarks,
    /// discourse-topic-voting's
    Votes,
}

impl ListKind {
    /// `Discourse.filters` (`anonymous_filters` first), as route segments.
    pub const ALL: [(&'static str, ListKind); 10] = [
        ("latest", ListKind::Latest),
        ("top", ListKind::Top),
        ("hot", ListKind::Hot),
        ("unread", ListKind::Unread),
        ("new", ListKind::New),
        ("unseen", ListKind::Unseen),
        ("read", ListKind::Read),
        ("posted", ListKind::Posted),
        ("bookmarks", ListKind::Bookmarks),
        ("votes", ListKind::Votes),
    ];

    pub fn name(self) -> &'static str {
        ListKind::ALL
            .iter()
            .find(|(_, k)| *k == self)
            .map(|(n, _)| *n)
            .expect("every kind is listed")
    }

    pub fn from_name(name: &str) -> Option<ListKind> {
        ListKind::ALL
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, k)| *k)
    }

    /// The TopicQuery filter; top needs its resolved period.
    fn filter(self, period: Option<&str>) -> Filter {
        match self {
            ListKind::Latest => Filter::Latest,
            ListKind::Top => Filter::Top(period.unwrap_or("all").to_string()),
            ListKind::Hot => Filter::Hot,
            ListKind::Unread => Filter::Unread,
            ListKind::New => Filter::New,
            ListKind::Unseen => Filter::Unseen,
            ListKind::Read => Filter::Read,
            ListKind::Posted => Filter::Posted,
            ListKind::Bookmarks => Filter::Bookmarks,
            ListKind::Votes => Filter::Votes,
        }
    }

    /// ListController's `ensure_logged_in` filters.
    pub fn requires_login(self) -> bool {
        self.filter(None).requires_login()
    }
}

/// `ListController.best_period_for(current_user&.previous_visit_at,
/// category_id)`: the category's default_top_period (else
/// top_page_default_timeframe), unless a period of best_periods_for has a
/// full page of scored topics.
async fn best_period_for(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    previous_visit_at: Option<chrono::DateTime<chrono::Utc>>,
    category_id: Option<i32>,
) -> Result<String, AppError> {
    let site_default = settings.get("top_page_default_timeframe")?.to_s();
    let mut default = site_default.clone();
    if let Some(id) = category_id {
        let period: Option<Option<String>> =
            sqlx::query_scalar("SELECT default_top_period FROM categories WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *conn)
                .await?;
        if let Some(Some(p)) = period {
            default = p;
        }
    }
    if !crate::topic_query::PERIODS.contains(&default.as_str()) {
        default = site_default;
    }
    let per_page = settings.get("topics_per_period_in_top_page")?.to_i();
    for period in best_periods_for(previous_visit_at, &default, crate::clock::now()) {
        if !crate::topic_query::PERIODS.contains(&period.as_str()) {
            continue;
        }
        let sql = match category_id {
            Some(_) => format!(
                "SELECT count(*) FROM (SELECT 1 FROM top_topics JOIN topics ON topics.id = top_topics.topic_id \
                 WHERE top_topics.{period}_score > 0 AND topics.category_id = $1 LIMIT $2) t"
            ),
            None => format!(
                "SELECT count(*) FROM (SELECT 1 FROM top_topics WHERE {period}_score > 0 AND $1::int IS NULL LIMIT $2) t"
            ),
        };
        let count: i64 = sqlx::query_scalar(&sql)
            .bind(category_id)
            .bind(per_page)
            .fetch_one(&mut *conn)
            .await?;
        if count == per_page {
            return Ok(period);
        }
    }
    Ok(default)
}

/// `ListController.best_periods_for(date, default_period)`: without a
/// previous visit the default then all; with one, each period shorter than
/// the time since it (with some slack), then all.
fn best_periods_for(
    previous_visit_at: Option<chrono::DateTime<chrono::Utc>>,
    default: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<String> {
    let Some(date) = previous_visit_at else {
        let mut periods = vec![default.to_string()];
        if default != "all" {
            periods.push("all".to_string());
        }
        return periods;
    };
    use chrono::{Duration, Months};
    // ActiveSupport's Duration#ago: calendar months and years, then days.
    let ago = |months: u32, days: i64| {
        now.checked_sub_months(Months::new(months)).unwrap_or(now) - Duration::days(days)
    };
    let mut periods = Vec::new();
    for (period, months, days) in [
        ("daily", 0, 8),
        ("weekly", 1, 7),
        ("monthly", 3, 21),
        ("quarterly", 13, 0),
        ("yearly", 36, 0),
    ] {
        if date > ago(months, days) {
            periods.push(period.to_string());
        }
    }
    periods.push("all".to_string());
    periods
}

/// GET /top(.json) and /hot(.json).
pub async fn top(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    front_list(
        state,
        guardian,
        params,
        ListKind::Top,
        false,
        headers,
        Some(uri),
    )
    .await
}

pub async fn top_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    front_list(state, guardian, params, ListKind::Top, true, headers, None).await
}

pub async fn hot(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    front_list(
        state,
        guardian,
        params,
        ListKind::Hot,
        false,
        headers,
        Some(uri),
    )
    .await
}

pub async fn hot_json(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    front_list(state, guardian, params, ListKind::Hot, true, headers, None).await
}

/// `/top/:period(.json)` -> 301 to `/top?period=`.
pub async fn top_period_redirect(
    State(state): State<AppState>,
    Path(period): Path<String>,
    headers: HeaderMap,
) -> Response {
    let (period, format) = match period.strip_suffix(".json") {
        Some(p) => (p.to_string(), ".json"),
        None => (period, ""),
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let location = format!(
        "http://{host}{}/top{format}?period={period}",
        state.config.globals.relative_url_root()
    );
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, location)],
    )
        .into_response()
}

async fn front_list(
    state: AppState,
    guardian: Guardian,
    params: ListParams,
    kind: ListKind,
    json: bool,
    headers: HeaderMap,
    uri: Option<axum::http::Uri>,
) -> Result<Response, AppError> {
    // Before the page's data is read (crate::bus::page_position).
    let bus_position = if json {
        String::new()
    } else {
        crate::bus::page_position(&state.bus).await?
    };
    let live_since = crate::clock::now().timestamp_millis().to_string();
    if let Some(response) = ensure_logged_in(&state, &guardian, kind) {
        return Ok(response);
    }
    let list_path = format!(
        "{}/{}",
        state.config.globals.relative_url_root(),
        kind.name()
    );
    let (doc, settings) =
        match list_document(&state, &guardian, &params, None, false, kind, &list_path).await? {
            Ok(doc) => doc,
            Err(message) => return Ok(invalid_list_params(&state, &message, json)),
        };
    if json {
        return Ok(Json(doc).into_response());
    }
    let mut conn = state.pool.acquire().await?;
    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site =
        crate::html::Site::from_settings(&settings, state.config.globals.relative_url_root())?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Discovery,
    )
    .await?;
    site.bus_position = bus_position;
    let mut page = crate::html::latest_page(
        &mut conn,
        site,
        &doc,
        &state.i18n,
        &settings,
        false,
        highlights_last_visited(kind, &params),
    )
    .await?;
    page.banner = crate::html::welcome_banner(
        &state.i18n,
        &settings,
        page.viewer.as_ref(),
        state.config.globals.relative_url_root(),
        kind.name(),
    )?;
    let homepage = crate::homepage::current(&mut conn, &settings, guardian.user_id()).await?;
    let token = page.chrome.topics_title_token(kind.name(), &homepage);
    page.title = crate::html::document_title(
        token.as_deref(),
        &page.site_title,
        &front_page_description(uri.as_ref(), &settings)?,
    );
    page.chrome.welcome_banner = !page.banner.is_empty();
    page.breadcrumbs = crate::html::breadcrumbs(&state.i18n, &settings)?;
    if matches!(kind, ListKind::Latest | ListKind::New | ListKind::Unread) {
        page.live_filter = kind.name().to_string();
        page.live_since = live_since;
    }
    page.nav = crate::html::nav_items(
        &state.i18n,
        &settings,
        state.config.globals.relative_url_root(),
        kind.name(),
        page.chrome.tracking.as_ref(),
    )?;
    if page.chrome.tracking.is_some() {
        page.chrome.live_param("nav", kind.name());
    }
    let uri = uri.unwrap_or_default();
    page.crawler = list_crawler(&mut conn, &state, &settings, &uri, None, None).await?;
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}

/// The crawler block for a list page: the site's title, description and
/// OpenGraph image unless the page supplies its own.
async fn list_crawler(
    conn: &mut sqlx::PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    uri: &axum::http::Uri,
    canonical: Option<String>,
    meta: Option<(String, String)>,
) -> Result<crate::html::Crawler, AppError> {
    let urls = Urls {
        config: &state.config,
        settings,
    };
    let site_description = settings.get("site_description")?.to_s();
    let (title, description) = meta.unwrap_or_else(|| {
        (
            settings.get("title").map(|t| t.to_s()).unwrap_or_default(),
            site_description.clone(),
        )
    });
    let image = crate::html::site_opengraph_image(conn, &urls).await?;
    let mut crawler = crate::html::Crawler::for_request(&urls, uri, canonical)?.with_meta(
        urls.settings.emojis(),
        &title,
        &description,
        image,
    );
    crawler.description = if description.is_empty() {
        site_description
    } else {
        description
    };
    Ok(crawler)
}

/// ListController's `ensure_logged_in` for the login-only filters: the 403
/// `not_logged_in` body, which a page request gets as the not-found page
/// (not_found::html_errors).
fn ensure_logged_in(state: &AppState, guardian: &Guardian, kind: ListKind) -> Option<Response> {
    if !kind.requires_login() || guardian.is_authenticated() {
        return None;
    }
    Some(super::login_required::not_logged_in(state))
}

/// GET /unread, /new, /unseen, /read, /posted, /bookmarks, /votes (.json):
/// the front lists without their own handler, keyed by the path.
pub async fn user_list(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<Response, AppError> {
    let path = uri.path().trim_start_matches('/');
    let (name, json) = match path.strip_suffix(".json") {
        Some(n) => (n, true),
        None => (path, false),
    };
    let kind = ListKind::from_name(name).ok_or(crate::Unsupported("unknown list filter"))?;
    front_list(
        state,
        guardian,
        params,
        kind,
        json,
        headers,
        (!json).then_some(uri),
    )
    .await
}

/// GET /topics/created-by/:username(.json): ListController#topics_by, the
/// topics the user created. The HTML is the Ember app, which has no route
/// for it: its not-found page.
pub async fn topics_by(
    State(state): State<AppState>,
    axum::Extension(incoming): axum::Extension<crate::session::current::Incoming>,
    Path(username): Path<String>,
    Query(params): Query<ListParams>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let guardian = incoming.guardian.clone();
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let Some(username) = username.strip_suffix(".json") else {
        let bus_position = crate::bus::page_position(&state.bus).await?;
        return super::user_preferences::ember_not_found(
            &state,
            &mut conn,
            &settings,
            &guardian,
            &headers,
            bus_position,
        )
        .await;
    };
    let mut options = match build_options(&params, &settings)? {
        Ok(o) => o,
        Err(message) => return Ok(invalid_list_params(&state, &message, true)),
    };
    // fetch_user_from_params(include_inactive: staff, or a member with
    // show_inactive_accounts), then ensure_can_see_profile!.
    let include_inactive = guardian.is_staff()
        || (guardian.is_authenticated() && settings.get("show_inactive_accounts")?.truthy());
    let user = if include_inactive {
        crate::users::User::find_any(&mut conn, username).await?
    } else {
        crate::users::User::find_active(&mut conn, username).await?
    };
    let Some(user) = user else {
        return Ok(super::topics::not_found_response(&state));
    };
    if !user.visible_to(&settings, &guardian)? {
        return Ok(super::topics::not_found_response(&state));
    }
    options.no_definitions = false;
    if params.period.is_some() {
        return Err(Unsupported("period on the created-by list").into());
    }
    let list_path = format!(
        "{}/topics/created-by/{}",
        state.config.globals.relative_url_root(),
        user.username.to_lowercase()
    );
    let more = next_url(&list_path, &params, &options, true);
    let doc = topics_by_list(
        &state, &mut conn, &settings, &guardian, user.id, options, more,
    )
    .await?;
    Ok(Json(doc).into_response())
}

/// TopicQuery#list_topics_by(user), serialized as a topic list.
pub(crate) async fn topics_by_list(
    state: &AppState,
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
    user_id: i32,
    options: Options,
    more_topics_url: String,
) -> Result<serde_json::Value, AppError> {
    let mut query = TopicQuery {
        conn: &mut *conn,
        settings,
        guardian,
        options,
        category: Default::default(),
        tags: Default::default(),
        filter: Default::default(),
        user: Default::default(),
    };
    let topics = query.list(Filter::TopicsBy(user_id)).await?;
    let category_id = query.options.category_id;
    let urls = Urls {
        config: &state.config,
        settings,
    };
    Ok(TopicListSerializer {
        conn,
        settings,
        i18n: &state.i18n,
        guardian,
        urls: &urls,
        more_topics_url: Some(more_topics_url),
        category_id,
        group_id: None,
        prefetched: Default::default(),
    }
    .serialize(&topics)
    .await?)
}

/// `CategoriesController#topics_per_page`: categories_topics when set,
/// else one and a half times the visible top-level categories, 5 to 100.
async fn topics_per_page(
    conn: &mut sqlx::PgConnection,
    settings: &SiteSettings,
    guardian: &Guardian,
) -> Result<i64, AppError> {
    let set = settings.get("categories_topics")?.to_i();
    if set > 0 {
        return Ok(set);
    }
    let allowed = guardian.allowed_category_ids(&mut *conn, settings).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM categories WHERE parent_category_id IS NULL AND id = ANY($1)",
    )
    .bind(&allowed)
    .fetch_one(&mut *conn)
    .await?;
    Ok(((count as f64 * 1.5) as i64).clamp(5, 100))
}
