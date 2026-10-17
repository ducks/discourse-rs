//! Admin::Config::CategoryManagementController#categories
//! (ListAdminCategories with AdminCategorySerializer) and the category
//! management pages (90g6): each type's list, all of them, and the
//! settings tab. The pages' JSON is the config site settings
//! controller's, as Rails routes `category-management(/*path)`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::admin_pages::{Shell, wants_json};
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// `ListAdminCategories::DEFAULT_PER_PAGE` and `MAX_PER_PAGE`
const DEFAULT_PER_PAGE: i64 = 50;
const MAX_PER_PAGE: i64 = 100;

/// The registered types' ids, as `Categories::TypeRegistry.all` holds them.
const TYPE_IDS: [&str; 4] = ["discussion", "events", "support", "ideas"];

/// `find_matches` of a type that matches a subset of the categories.
async fn matching_ids(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    type_id: &str,
) -> Result<Vec<i32>, AppError> {
    Ok(match type_id {
        "events" => settings
            .get("events_calendar_categories")?
            .to_s()
            .split('|')
            .filter_map(|id| id.trim().parse::<i32>().ok())
            .collect(),
        "support" => {
            sqlx::query_scalar(
                "SELECT DISTINCT category_id FROM category_custom_fields \
                 WHERE name = 'enable_accepted_answers' AND value = 'true'",
            )
            .fetch_all(&mut *conn)
            .await?
        }
        "ideas" => {
            sqlx::query_scalar("SELECT DISTINCT category_id FROM topic_voting_category_settings")
                .fetch_all(&mut *conn)
                .await?
        }
        _ => Vec::new(),
    })
}

#[derive(sqlx::FromRow)]
struct Row {
    id: i32,
    name: String,
    slug: String,
    color: String,
    text_color: String,
    style_type: i32,
    icon: Option<String>,
    emoji: Option<String>,
    read_restricted: bool,
    parent_category_id: Option<i32>,
    description: Option<String>,
    topic_count: i32,
}

const COLUMNS: &str = "id, name, slug, color, text_color, style_type, icon, emoji, \
     read_restricted, parent_category_id, description, topic_count";

/// AdminCategoryBadgeSerializer
fn badge(row: &Row, settings: &SiteSettings, i18n: &crate::i18n::I18n) -> Result<Value, AppError> {
    let uncategorized = i64::from(row.id) == settings.get("uncategorized_category_id")?.to_i();
    let name = if uncategorized {
        i18n.t("uncategorized_category_name")
            .map(str::to_string)
            .unwrap_or_else(|| row.name.clone())
    } else {
        row.name.clone()
    };
    Ok(json!({
        "id": row.id,
        "name": name,
        "slug": row.slug,
        "color": row.color,
        "text_color": row.text_color,
        "style_type": crate::categories::style_type(row.style_type),
        "icon": row.icon,
        "emoji": row.emoji,
        "read_restricted": row.read_restricted,
        "parent_category_id": row.parent_category_id,
    }))
}

/// ListAdminCategories for the params, as the controller renders it.
async fn list(
    conn: &mut PgConnection,
    state: &AppState,
    settings: &SiteSettings,
    p: &Map<String, Value>,
) -> Result<Option<Value>, AppError> {
    let param = |k: &str| p.get(k).and_then(crate::params::scalar);
    let type_id = param("type")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "discussion".to_string());
    if type_id != "all" && !TYPE_IDS.contains(&type_id.as_str()) {
        // category_type_exists
        return Ok(None);
    }
    let mut sql = format!("SELECT {COLUMNS} FROM categories WHERE TRUE");
    let mut ids: Vec<i32> = Vec::new();
    match type_id.as_str() {
        "all" => {}
        "discussion" => {
            for other in &TYPE_IDS[1..] {
                ids.extend(matching_ids(conn, settings, other).await?);
            }
            sql.push_str(" AND NOT (id = ANY($1))");
        }
        other => {
            ids = matching_ids(conn, settings, other).await?;
            sql.push_str(" AND id = ANY($1)");
        }
    }
    // matching_name_or_slug_ref
    let filter = param("filter")
        .map(|f| f.trim().trim_start_matches('#').to_string())
        .filter(|f| !f.is_empty());
    let like = filter.as_ref().map(|f| {
        let mut normalized = f.replace('/', ":");
        while normalized.contains("::") {
            normalized = normalized.replace("::", ":");
        }
        format!(
            "%{}%",
            normalized
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        )
    });
    if like.is_some() {
        sql.push_str(
            " AND (lower(unaccent(categories.name)) ILIKE lower(unaccent($2)) \
               OR lower(unaccent(categories.slug)) ILIKE lower(unaccent($2)) \
               OR EXISTS (SELECT 1 FROM categories parent_categories \
                 WHERE parent_categories.id = categories.parent_category_id \
                   AND lower(unaccent(parent_categories.slug || ':' || categories.slug)) \
                     ILIKE lower(unaccent($2))))",
        );
    }
    match param("visibility").as_deref() {
        Some("public") => sql.push_str(" AND NOT read_restricted"),
        Some("restricted") => sql.push_str(" AND read_restricted"),
        _ => {}
    }
    // The controller's per_page (clamped) and fetch_page_from_params(max:).
    let per_page = match param("per_page").filter(|v| !v.is_empty()) {
        Some(v) => crate::ruby::to_i(&v).clamp(1, MAX_PER_PAGE),
        None => DEFAULT_PER_PAGE,
    };
    // Checked by the controller (fetch_page_from_params) first.
    let page = param("page")
        .and_then(|v| crate::ruby::integer(&v))
        .unwrap_or(0);
    sql.push_str(
        " ORDER BY lower(unaccent(categories.name)), categories.id ASC OFFSET $3 LIMIT $4",
    );
    let mut rows: Vec<Row> = sqlx::query_as(&sql)
        .bind(&ids)
        .bind(like.unwrap_or_default())
        .bind(page * per_page)
        .bind(per_page + 1)
        .fetch_all(&mut *conn)
        .await?;
    let has_more = rows.len() as i64 > per_page;
    rows.truncate(per_page as usize);
    // Each category's ancestors, for the badge chains and edit URLs.
    let mut known: Vec<Row> = Vec::new();
    let types = {
        let cx = crate::category_serializer::Cx::of(state);
        let ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
        crate::category_serializer::with_registry(&cx, settings, async |registry| {
            registry.for_categories(&mut *conn, &ids).await
        })
        .await?
    };
    let mut out = Vec::new();
    for (row, types) in rows.iter().zip(types) {
        let mut chain: Vec<Value> = Vec::new();
        let mut slugs = vec![row.slug.clone()];
        let mut parent = row.parent_category_id;
        while let Some(pid) = parent {
            if !known.iter().any(|k| k.id == pid) {
                let found: Option<Row> =
                    sqlx::query_as(&format!("SELECT {COLUMNS} FROM categories WHERE id = $1"))
                        .bind(pid)
                        .fetch_optional(&mut *conn)
                        .await?;
                match found {
                    Some(f) => known.push(f),
                    None => break,
                }
            }
            let Some(p) = known.iter().find(|k| k.id == pid) else {
                break;
            };
            chain.insert(0, badge(p, settings, &state.i18n)?);
            slugs.insert(0, p.slug.clone());
            parent = p.parent_category_id;
        }
        chain.push(badge(row, settings, &state.i18n)?);
        let category_types: Vec<Value> = types
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(id, meta)| json!({ "id": id, "name": meta["name"] }))
                    .collect()
            })
            .unwrap_or_default();
        out.push(json!({
            "id": row.id,
            "slug": row.slug,
            "badge_chain": chain,
            "category_types": category_types,
            "description_text": crate::categories::description_plain_text(row.description.as_deref())?,
            "read_restricted": row.read_restricted,
            "topic_count": row.topic_count,
            "edit_url": format!("/c/{}/edit/general", slugs.join("/")),
        }));
    }
    Ok(Some(json!({ "categories": out, "has_more": has_more })))
}

/// GET /admin/config/category-management/categories(.json)
pub async fn categories(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    if let Some(refused) = super::admin_config_branding::admin_only(&state, &guardian) {
        return Ok(refused);
    }
    let p = crate::params::parse(uri.query(), &headers, &body);
    // fetch_page_from_params raises before the service.
    let per_page = p
        .get("per_page")
        .and_then(crate::params::scalar)
        .filter(|v| !v.is_empty())
        .map(|v| crate::ruby::to_i(&v).clamp(1, MAX_PER_PAGE))
        .unwrap_or(DEFAULT_PER_PAGE);
    if let Some(page) = p.get("page").and_then(crate::params::scalar)
        && !crate::ruby::integer(&page).is_some_and(|n| (0..=per_page).contains(&n))
    {
        return Ok(super::search::invalid_parameters(&state, "page"));
    }
    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;
    Ok(match list(&mut conn, &state, &settings, &p).await? {
        Some(body) => Json(body).into_response(),
        None => super::topics::not_found_response(&state),
    })
}

/// Which tab.
enum Tab {
    Settings,
    Type(String),
}

async fn page(
    state: &AppState,
    guardian: &crate::guardian::Guardian,
    headers: &HeaderMap,
    uri: &Uri,
    body: Bytes,
    tab: Option<Tab>,
) -> Result<Response, AppError> {
    if wants_json(uri, headers) || !guardian.is_admin() {
        return super::admin_site_settings::config_index(
            State(state.clone()),
            AuthGuardian(guardian.clone()),
            headers.clone(),
            uri.clone(),
            body,
        )
        .await;
    }
    // The index route goes on to the list of all.
    let Some(tab) = tab else {
        let to = format!(
            "{}/admin/config/category-management/all",
            state.config.globals.relative_url_root()
        );
        return Ok(axum::response::Redirect::to(&to).into_response());
    };
    let mut conn = state.pool.acquire().await?;
    let shell = Shell::load(
        &mut conn,
        state,
        guardian,
        headers,
        "admin_category_management",
    )
    .await?;
    let cx = shell.cx(state);
    let (outlet, classes, title, script) = match &tab {
        Tab::Settings => (
            super::admin_advanced::area_settings(
                &mut conn,
                state,
                &shell,
                uri,
                json!({ "filter_area": "categories" }),
            )
            .await?,
            "admin-config category-management settings".to_string(),
            Some(crate::admin_view::t(
                cx.i18n,
                "admin.config.category_management.tabs.settings",
            )),
            "site-settings.js",
        ),
        Tab::Type(id) => {
            // The route's model: "all", or a site category type.
            let types = {
                let ccx = crate::category_serializer::Cx::of(state);
                crate::category_serializer::with_registry(&ccx, &shell.settings, async |registry| {
                    registry.list(&mut conn, guardian).await
                })
                .await?
            };
            let model = if id == "all" {
                Some(json!({
                    "id": "all",
                    "name": crate::admin_view::t(cx.i18n, "admin.config.category_management.types.all.title"),
                    "title": crate::admin_view::t(cx.i18n, "admin.config.category_management.types.all.title"),
                    "description": crate::admin_view::t(cx.i18n, "admin.config.category_management.types.all.description"),
                    "available": true,
                }))
            } else {
                types
                    .as_array()
                    .and_then(|list| list.iter().find(|t| t["id"].as_str() == Some(id)))
                    .cloned()
            };
            let Some(model) = model else {
                return Ok(super::topics::not_found_response(state));
            };
            // The list's first page, as the component loads it; with
            // `fragment` the page the script asks for (its filter,
            // visibility and page), as rows or the table.
            let query = crate::params::parse_query(uri.query().unwrap_or(""));
            let fragment = query.contains_key("fragment");
            let mut params = Map::new();
            params.insert("type".into(), json!(id));
            params.insert("per_page".into(), json!("50"));
            let page_no = if fragment {
                for key in ["filter", "visibility"] {
                    if let Some(v) = crate::params::string(&query, key) {
                        params.insert(key.into(), json!(v));
                    }
                }
                crate::params::string(&query, "page")
                    .and_then(|p| p.parse::<i64>().ok())
                    .unwrap_or(0)
                    .clamp(0, 50)
            } else {
                0
            };
            params.insert("page".into(), json!(page_no.to_string()));
            let first = list(&mut conn, state, &shell.settings, &params)
                .await?
                .unwrap_or_else(|| json!({ "categories": [], "has_more": false }));
            if fragment {
                let categories = first["categories"].as_array().cloned().unwrap_or_default();
                let html = if page_no > 0 {
                    categories
                        .iter()
                        .map(|c| crate::admin_category_management_view::row(&cx, c, id == "all"))
                        .collect()
                } else {
                    crate::admin_category_management_view::table(&cx, &categories, id == "all")
                };
                let mut response = axum::response::Html(html).into_response();
                response.headers_mut().insert(
                    "x-has-more",
                    axum::http::HeaderValue::from_static(if first["has_more"] == true {
                        "true"
                    } else {
                        "false"
                    }),
                );
                return Ok(response);
            }
            let title = model["title"].as_str().map(str::to_string);
            (
                crate::admin_category_management_view::list(&cx, &model, &first),
                "admin-config category-management type".to_string(),
                title,
                "admin-category-management.js",
            )
        }
    };
    let types = {
        let ccx = crate::category_serializer::Cx::of(state);
        crate::category_serializer::with_registry(&ccx, &shell.settings, async |registry| {
            registry.list(&mut conn, guardian).await
        })
        .await?
    };
    let active = match &tab {
        Tab::Settings => "settings".to_string(),
        Tab::Type(id) => id.clone(),
    };
    let content = crate::admin_category_management_view::page(&cx, &types, &active, &outlet);
    let strings = crate::admin_ui::strings(
        &cx,
        "admin-category-management-strings",
        crate::admin_category_management_view::STRINGS,
    );
    let main = crate::admin_view::contents(&classes, &format!("{content}{strings}"));
    let section = crate::admin_view::t(cx.i18n, "admin.config.category_management.title");
    let title = match title {
        Some(t) => format!("{t} - {section}"),
        None => section,
    };
    shell.render(state, uri, &title, main, script, StatusCode::OK)
}

pub(super) fn routes(mut router: axum::Router<AppState>) -> axum::Router<AppState> {
    use axum::routing::get;
    for suffix in ["", ".json"] {
        router = router
            .route(
                &format!("/admin/config/category-management/categories{suffix}"),
                get(categories),
            )
            .route(
                &format!("/admin/config/category-management{suffix}"),
                get(
                    |State(state): State<AppState>,
                     AuthGuardian(guardian): AuthGuardian,
                     headers: HeaderMap,
                     uri: Uri,
                     body: Bytes| async move {
                        page(&state, &guardian, &headers, &uri, body, None).await
                    },
                ),
            );
    }
    router.route(
        "/admin/config/category-management/{tab}",
        get(
            |State(state): State<AppState>,
             AuthGuardian(guardian): AuthGuardian,
             Path(tab): Path<String>,
             headers: HeaderMap,
             uri: Uri,
             body: Bytes| async move {
                let name = tab.strip_suffix(".json").unwrap_or(&tab).to_string();
                let tab = if name == "settings" {
                    Tab::Settings
                } else {
                    Tab::Type(name)
                };
                page(&state, &guardian, &headers, &uri, body, Some(tab)).await
            },
        ),
    )
}
