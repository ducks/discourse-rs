//! Port of UsersController#search_users: GET /u/search/users(.json), what
//! the `@` autocomplete and the user pickers ask.

use axum::Json;
use axum::extract::State;
use axum::http::Uri;
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::params;
use crate::ruby::cast_boolean;
use crate::session::current::AuthGuardian;
use crate::site_settings::SiteSettings;
use crate::user_search::{self, Options, Refused, UserSearch, quote};
use crate::{AppError, AppState, Unsupported};

/// `UsersController::SEARCH_USERS_LIMIT`
const SEARCH_USERS_LIMIT: i64 = 50;

/// GET /u/search/users
pub async fn search_users(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    uri: Uri,
) -> Result<Response, AppError> {
    let p = params::parse_query(uri.query().unwrap_or_default());
    let scalar = |key: &str| p.get(key).and_then(params::scalar);
    let present = |key: &str| scalar(key).filter(|v| !crate::ruby::is_blank(v));
    let term = crate::ruby::strip(&scalar("term").unwrap_or_default()).to_string();
    // String#split(","): trailing empty fields go.
    let usernames: Vec<String> = match scalar("usernames") {
        Some(list) => {
            let mut names: Vec<String> = list
                .split(',')
                .map(|u| crate::ruby::strip(&u.to_lowercase()).to_string())
                .collect();
            while names.last().is_some_and(|n| n.is_empty()) {
                names.pop();
            }
            names
        }
        None => Vec::new(),
    };
    let id_param = |key: &str| present(key).map(|v| crate::ruby::to_i(&v) as i32);

    // fetch_limit_from_params(default: nil, max: SEARCH_USERS_LIMIT)
    let limit = match p.get("limit") {
        None => None,
        Some(value) => match params::scalar(value).and_then(|v| crate::ruby::integer(&v)) {
            Some(n) if (0..=SEARCH_USERS_LIMIT).contains(&n) => Some(n),
            _ => return Ok(super::search::invalid_parameters(&state, "limit")),
        },
    };

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;

    // groups / group: Group.where(name: ...)
    let mut group_names: Vec<String> = match p.get("groups") {
        None => Vec::new(),
        Some(Value::Array(items)) => items.iter().filter_map(params::scalar).collect(),
        Some(_) => return Err(Unsupported("a groups param that is not a list").into()),
    };
    if let Some(group) = scalar("group") {
        group_names.push(group);
    }
    let groups = if group_names.is_empty() {
        None
    } else {
        Some(
            sqlx::query_scalar::<_, i32>("SELECT id FROM groups WHERE name = ANY($1)")
                .bind(&group_names)
                .fetch_all(&mut *conn)
                .await?,
        )
    };

    let options = Options {
        topic_id: id_param("topic_id"),
        category_id: id_param("category_id"),
        prioritized_user_id: id_param("prioritized_user_id"),
        topic_allowed_users: scalar("topic_allowed_users").as_deref() == Some("true"),
        groups,
        include_staged_users: cast_boolean(scalar("include_staged_users").as_deref()) == Some(true),
        last_seen_users: cast_boolean(scalar("last_seen_users").as_deref()) == Some(true),
        can_review: cast_boolean(scalar("can_review").as_deref()) == Some(true),
        limit,
    };
    let mut search = match UserSearch::new(&mut conn, &settings, &guardian, &term, options).await? {
        Ok(search) => search,
        Err(Refused::NotFound) => return Ok(super::topics::not_found_response(&state)),
        Err(Refused::InvalidAccess) => return Ok(super::search::invalid_access(&state)),
    };
    let ids = if usernames.is_empty() {
        search.search_ids().await?
    } else {
        search.by_usernames(&usernames, limit).await?
    };
    let urls = crate::url::Urls {
        config: &state.config,
        settings: &settings,
    };
    let users = user_search::found_users(&mut conn, &urls, &guardian, &ids).await?;
    let mut out = Map::new();
    out.insert("users".into(), json!(users));

    // A blank term never brings groups.
    if let Some(user_id) = guardian.user_id()
        && (!term.is_empty() || !usernames.is_empty())
    {
        let scope = if scalar("include_groups").as_deref() == Some("true") {
            Some(user_search::visible_groups_where(&guardian))
        } else if scalar("include_mentionable_groups").as_deref() == Some("true") {
            Some(user_search::mentionable_where(&guardian, user_id))
        } else if scalar("include_messageable_groups").as_deref() == Some("true") {
            Some(user_search::messageable_where(&guardian, user_id))
        } else {
            None
        };
        if let Some(scope) = scope {
            let sql = if usernames.is_empty() {
                user_search::search_groups_sql(&scope, &term)
            } else {
                let names: Vec<String> = usernames.iter().map(|n| quote(n)).collect();
                let mut sql = format!(
                    "SELECT groups.name, groups.full_name FROM groups WHERE ({scope}) AND groups.name IN ({}) ORDER BY groups.name ASC",
                    names.join(", ")
                );
                if let Some(limit) = limit {
                    sql.push_str(&format!(" LIMIT {limit}"));
                }
                sql
            };
            let groups: Vec<(String, Option<String>)> =
                sqlx::query_as(&sql).fetch_all(&mut *conn).await?;
            let groups: Vec<Value> = groups
                .into_iter()
                .map(|(name, full_name)| json!({ "name": name, "full_name": full_name }))
                .collect();
            out.insert("groups".into(), json!(groups));
        }
    }
    Ok(Json(Value::Object(out)).into_response())
}
