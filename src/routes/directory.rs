//! GET /directory_items (directory_items#index): a period's user
//! directory, a page at a time; and GET /u, the users page drawn from it.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::directory::{PAGE_LIMIT, PAGE_SIZE, Query, Refused};
use crate::params;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// `fetch_int_from_params(key, default:, max:)`, min 0: the value, or the
/// InvalidParameters it raises.
fn int_param(p: &Map<String, Value>, key: &str, default: i64, max: i64) -> Result<i64, ()> {
    match p.get(key).and_then(params::scalar) {
        None => Ok(default),
        Some(v) => match v.trim().parse::<i64>() {
            Ok(n) if (0..=max).contains(&n) => Ok(n),
            _ => Err(()),
        },
    }
}

/// `Hash#to_query`: keys sorted, values form-encoded.
fn to_query(pairs: &mut [(String, String)]) -> String {
    pairs.sort();
    let enc = |s: &str| form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// GET /directory_items
pub async fn index(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    let p = params::parse(uri.query(), &headers, &body);
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("enable_user_directory")?.truthy() {
        return Ok(super::search::invalid_access(&state));
    }
    let Some(period) = params::string(&p, "period").filter(|s| !s.is_empty()) else {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"errors": ["param is missing or the value is empty or invalid: period"]})),
        )
            .into_response());
    };
    if !crate::directory::PERIODS.iter().any(|(n, _)| *n == period) {
        return Ok(super::search::invalid_access(&state));
    }
    let Ok(page) = int_param(&p, "page", 0, PAGE_LIMIT) else {
        return Ok(super::search::invalid_parameters(&state, "page"));
    };
    let Ok(limit) = int_param(&p, "limit", PAGE_SIZE, PAGE_SIZE) else {
        return Ok(super::search::invalid_parameters(&state, "limit"));
    };
    let s = |k: &str| params::string(&p, k);
    let query = Query {
        period: period.clone(),
        group: s("group"),
        exclude_groups: s("exclude_groups"),
        exclude_usernames: s("exclude_usernames"),
        order: s("order"),
        ascending: s("asc").is_some_and(|v| !v.trim().is_empty()),
        name: s("name"),
        username: s("username"),
        page,
        limit,
        user_field_ids: match p.get("user_field_ids") {
            None | Some(Value::Null) => None,
            Some(v) => Some(params::scalar(v).ok_or(Unsupported(
                "user_field_ids that isn't a string (NoMethodError)",
            ))?),
        },
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let (items, total, last_updated_at) =
        match crate::directory::index(&mut conn, &settings, &urls, &guardian, &query).await? {
            Ok(r) => r,
            Err(Refused::GroupNotFound) => {
                return Ok(super::search::invalid_parameters(&state, "group"));
            }
            Err(Refused::InvalidAccess) => return Ok(super::search::invalid_access(&state)),
        };
    let mut more: Vec<(String, String)> =
        ["period", "order", "asc", "group", "user_field_ids", "name"]
            .iter()
            .filter_map(|k| s(k).map(|v| (k.to_string(), v)))
            .collect();
    more.push(("page".into(), (page + 1).to_string()));
    let base = state.config.globals.relative_url_root();
    let load_more = format!("{base}/directory_items.json?{}", to_query(&mut more));
    Ok(Json(json!({
        "directory_items": items,
        "meta": {
            "last_updated_at": last_updated_at,
            "total_rows_directory_items": total,
            "load_more_directory_items": load_more,
        },
    }))
    .into_response())
}

/// The HTML of /u (layout plus the users route's content).
#[derive(askama::Template)]
#[template(path = "users.html")]
pub struct UsersHtml {
    pub site_title: String,
    pub lang: String,
    pub base_path: String,
    pub crawler: crate::html::Crawler,
    pub viewer: Option<crate::html::Viewer>,
    pub bus_position: String,
    pub chrome: crate::html::Chrome,
    /// The route's titleToken.
    pub page_title: String,
    pub main: String,
}

/// `Group.findAll({ ignore_automatic: true })` (groups#search: the
/// visible groups by name) filtered by `can_see_members`: the groups a
/// member's directory filters by, as their names and labels
/// (`full_name || name`). None for a visitor.
async fn group_options(
    conn: &mut sqlx::PgConnection,
    guardian: &crate::guardian::Guardian,
) -> Result<Vec<(String, String)>, AppError> {
    if !guardian.is_authenticated() {
        return Ok(Vec::new());
    }
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT g.name, g.full_name FROM groups g WHERE NOT g.automatic AND {} AND {} ORDER BY g.name",
        crate::groups::visible_groups_where(guardian, "g"),
        crate::groups::members_visible_groups_where(guardian, "g"),
    ))
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, full_name)| {
            let label = full_name
                .filter(|f| !f.is_empty())
                .unwrap_or_else(|| name.clone());
            (name, label)
        })
        .collect())
}

/// GET /u (users#index): Ember's users directory for the query params
/// the route takes (period, order, asc, name, group, exclude_groups,
/// exclude_usernames), its first page loaded as the route loads it from
/// directory_items#index.
pub async fn page(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    use askama::Template;

    let bus_position = crate::bus::page_position(&state.bus).await?;
    let p = params::parse(uri.query(), &headers, &Bytes::new());
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    let base_path = state.config.globals.relative_url_root();
    // The route's beforeModel: anonymous visitors go to the front page.
    if settings.get("hide_user_profiles_from_public")?.truthy() && guardian.is_anonymous() {
        return Ok(axum::response::Redirect::to(&format!("{base_path}/")).into_response());
    }
    // /directory-columns.json: the enabled columns by position.
    // DirectoryColumnSerializer's name: the column's, else its user field's.
    type ColumnRow = (Option<String>, i32, Option<String>, Option<i32>);
    let rows: Vec<ColumnRow> = sqlx::query_as(
        "SELECT COALESCE(dc.name, uf.name), dc.type, dc.icon, dc.user_field_id \
         FROM directory_columns dc LEFT JOIN user_fields uf ON uf.id = dc.user_field_id \
         WHERE dc.enabled ORDER BY dc.position",
    )
    .fetch_all(&mut *conn)
    .await?;
    if rows.iter().any(|(_, kind, _, _)| *kind == 2) {
        return Err(Unsupported("plugin columns in the user directory").into());
    }
    let columns: Vec<crate::directory_view::Column> = rows
        .into_iter()
        .map(
            |(name, kind, icon, user_field_id)| crate::directory_view::Column {
                name: name.unwrap_or_default(),
                icon,
                user_field_id: user_field_id.filter(|_| kind == 1),
            },
        )
        .collect();
    // The controller's user_field_ids: the user field columns' ids.
    let user_field_ids = columns
        .iter()
        .filter_map(|c| c.user_field_id.map(|id| id.to_string()))
        .collect::<Vec<_>>()
        .join("|");
    let groups = group_options(&mut conn, &guardian).await?;
    let s = |k: &str| params::string(&p, k).filter(|v| !v.is_empty());
    // The controller's defaults; the route's first column orders.
    let period = s("period").unwrap_or_else(|| "weekly".into());
    let order = s("order")
        .or_else(|| columns.first().map(|c| c.name.clone()))
        .unwrap_or_else(|| "likes_received".into());
    let asc = s("asc");
    let name = s("name").unwrap_or_default();

    let vs = super::session::viewer_state(&state, &headers, &settings, &guardian)?;
    let mut site = crate::html::Site::from_settings(&settings, base_path)?;
    site.viewer = vs.viewer.clone();
    site.load_chrome(
        &mut conn,
        &state,
        &settings,
        &guardian,
        crate::sidebar::Active::Users,
    )
    .await?;
    site.bus_position = bus_position;
    let categories = crate::topic_list_view::categories(&mut conn).await?;
    let cx = crate::topic_list_view::ListContext {
        i18n: &state.i18n,
        base_path,
        now: crate::clock::now(),
        categories: &categories,
        expand_all_pinned: false,
        member_trust_level: site.viewer.as_ref().map(|v| v.trust_level),
        settings: crate::topic_list_view::ListSettings::load(&settings)?,
    };
    let urls = Urls {
        config: &state.config,
        settings: &settings,
    };
    let crawler = crate::html::Crawler::for_request(&urls, &uri, None)?;

    // directory_items#index for the route's params, as the store asks
    // (blank ones left out). A refused list (a period it doesn't know, the directory turned off,
    // a group not to be seen) leaves the route without a model: the page
    // with nothing loaded and its empty state.
    let loaded = if !settings.get("enable_user_directory")?.truthy()
        || !crate::directory::PERIODS.iter().any(|(n, _)| *n == period)
    {
        None
    } else {
        let query = Query {
            period: period.clone(),
            group: s("group"),
            exclude_groups: s("exclude_groups"),
            exclude_usernames: s("exclude_usernames"),
            order: Some(order.clone()),
            ascending: asc.is_some(),
            name: Some(name.clone()).filter(|n| !n.is_empty()),
            username: None,
            page: 0,
            limit: PAGE_SIZE,
            user_field_ids: Some(user_field_ids.clone()).filter(|ids| !ids.is_empty()),
        };
        crate::directory::index(&mut conn, &settings, &urls, &guardian, &query)
            .await?
            .ok()
    };
    let (items, total, last_updated_at) = loaded.unwrap_or((Vec::new(), 0, Value::Null));
    // meta.load_more_directory_items, as index answers it.
    let mut more: Vec<(String, String)> = vec![
        ("period".into(), period.clone()),
        ("order".into(), order.clone()),
        ("page".into(), "1".into()),
    ];
    for key in ["asc", "group", "name"] {
        if let Some(v) = s(key) {
            more.push((key.into(), v));
        }
    }
    if !user_field_ids.is_empty() {
        more.push(("user_field_ids".into(), user_field_ids.clone()));
    }
    let load_more = format!("{base_path}/directory_items.json?{}", to_query(&mut more));
    let last_updated_at = last_updated_at
        .as_str()
        .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&chrono::Utc));
    // hideApplicationFooter while there is more to load.
    if (items.len() as i64) < total {
        site.chrome.powered_by = false;
    }
    let main = crate::directory_view::render(
        &cx,
        &crate::directory_view::Directory {
            columns: &columns,
            items: &items,
            total,
            last_updated_at,
            load_more_url: &load_more,
            period: &period,
            order: &order,
            asc: asc.is_some(),
            name: &name,
            viewer_id: guardian.user_id().map(i64::from),
            viewer_staff: guardian.is_staff(),
            groups: &groups,
            group: s("group").as_deref(),
        },
    );
    site.chrome.body_classes.push_str(" users-page");
    let page_title = crate::topic_list_view::t(&cx, "directory.title");
    let page = UsersHtml {
        site_title: site.site_title,
        lang: site.lang,
        base_path: site.base_path,
        crawler,
        viewer: site.viewer,
        bus_position: site.bus_position,
        chrome: site.chrome,
        page_title,
        main,
    };
    let body = page.render().map_err(crate::html::HtmlError::from)?;
    Ok(crate::html::crawler_response(
        body,
        &page.crawler,
        &settings,
        &vs,
    )?)
}
