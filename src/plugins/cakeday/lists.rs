//! CakedayController#cakedays_by, behind AnniversariesController#index
//! and BirthdaysController#index: the users celebrating today, tomorrow,
//! in the week after, or in a month, a page at a time, as
//! CakedayUserSerializer documents.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, Unsupported};

/// `CakedayController::PAGE_SIZE`
pub const PAGE_SIZE: i64 = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Anniversaries,
    Birthdays,
}

impl Kind {
    /// The document's key and the route's path segment.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Anniversaries => "anniversaries",
            Kind::Birthdays => "birthdays",
        }
    }
}

/// The list's params as `setup_params` reads them.
#[derive(Clone, Debug, Default)]
pub struct Params {
    /// `params[:filter]`: today, tomorrow, upcoming, anything else a month.
    pub filter: Option<String>,
    /// `params[:page].to_i.clamp(0..)`
    pub page: i64,
    /// `params[:month].to_i.clamp(1..12)`
    pub month: i64,
}

impl Params {
    /// From the raw params, as `to_i` reads them.
    pub fn parse(filter: Option<String>, page: Option<&str>, month: Option<&str>) -> Params {
        Params {
            filter,
            page: page.map(crate::ruby::to_i).unwrap_or(0).max(0),
            month: month.map(crate::ruby::to_i).unwrap_or(0).clamp(1, 12),
        }
    }
}

/// One user of the list (CakedayUserSerializer's fields).
#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: i32,
    username: String,
    name: Option<String>,
    title: Option<String>,
    uploaded_avatar_id: Option<i32>,
    cakedate_at: Option<NaiveDateTime>,
    cakedate_on: Option<NaiveDate>,
}

/// A page of the list: the users, the total and the next page's params.
pub struct Page {
    pub users: Vec<Value>,
    pub total: i64,
    /// `load_more_*`: the next page's path, under the base path.
    pub load_more: String,
}

fn mmdd(d: NaiveDate) -> String {
    d.format("%m%d").to_string()
}

fn leap(year: i32) -> bool {
    NaiveDate::from_ymd_opt(year, 2, 29).is_some()
}

/// `cakedays_by(column, at_least_one_year_old:, apply_timezone:)` for
/// the viewer (`user_option.timezone`, `today` in it).
pub async fn page(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    urls: &Urls<'_>,
    viewer_id: i32,
    kind: Kind,
    params: &Params,
) -> Result<Page, AppError> {
    let timezone: Option<String> =
        sqlx::query_scalar("SELECT timezone FROM user_options WHERE user_id = $1")
            .bind(viewer_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    // Time.zone.now.in_time_zone(@timezone), the server's zone (UTC)
    // when Rails raises for the name; nil reads as no zone.
    let zone = timezone
        .as_deref()
        .and_then(super::zone)
        .unwrap_or(chrono_tz::UTC);
    let today = crate::clock::now().with_timezone(&zone).date_naive();

    let (mut column, at_least_one_year_old, apply_timezone) = match kind {
        Kind::Anniversaries => ("users.created_at".to_string(), true, true),
        Kind::Birthdays => ("users.date_of_birth".to_string(), false, false),
    };
    if apply_timezone
        && let Some(name) = timezone.as_deref().filter(|t| !t.is_empty() && *t != "UTC")
        && let Some(tz) = super::zone(name)
    {
        // The canonical identifier, quoted: tz names carry no quote, but
        // Rails' own names can (Nuku'alofa) and are mapped first.
        column = format!(
            "{column} AT TIME ZONE 'UTC' AT TIME ZONE '{}'",
            tz.name().replace('\'', "''")
        );
    }

    let mut wheres = vec![
        "users.id > 0".to_string(),
        "NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id)".to_string(),
        "users.active".to_string(),
        "NOT users.staged".to_string(),
        "(users.silenced_till IS NULL OR users.silenced_till <= $1)".to_string(),
        "(users.suspended_till IS NULL OR users.suspended_till <= $1)".to_string(),
        "user_options.hide_profile = FALSE".to_string(),
    ];
    // $1 is now; the rest are text, in order.
    let mut binds: Vec<String> = Vec::new();
    fn bind(binds: &mut Vec<String>, v: String) -> String {
        binds.push(v);
        format!("${}", binds.len() + 1)
    }
    if at_least_one_year_old {
        let year = bind(&mut binds, today.year().to_string());
        wheres.push(format!("EXTRACT(YEAR FROM {column}) < {year}::int"));
    }
    // A year without February 29th has it on March 1st.
    let on_day = |binds: &mut Vec<String>, day: NaiveDate| {
        let date = bind(binds, mmdd(day));
        let not_leap = if leap(day.year()) { "FALSE" } else { "TRUE" };
        format!(
            "(TO_CHAR({column}, 'MMDD') = {date} OR ({not_leap} AND {date} = '0301' AND TO_CHAR({column}, 'MMDD') = '0229'))"
        )
    };
    let mut more = Vec::new();
    match params.filter.as_deref() {
        Some("today") => wheres.push(on_day(&mut binds, today)),
        Some("tomorrow") => wheres.push(on_day(&mut binds, today + Duration::days(1))),
        Some("upcoming") => {
            let from = today + Duration::days(2);
            let to = from + Duration::weeks(1);
            let from = bind(&mut binds, mmdd(from));
            let to = bind(&mut binds, mmdd(to));
            wheres.push(format!("TO_CHAR({column}, 'MMDD') BETWEEN {from} AND {to}"));
        }
        _ => {
            let month = bind(&mut binds, params.month.to_string());
            wheres.push(format!("EXTRACT(MONTH FROM {column}) = {month}::int"));
            more.push(("month", params.month.to_string()));
        }
    }
    more.push(("page", (params.page + 1).to_string()));
    if let Some(filter) = &params.filter {
        more.push(("filter", filter.clone()));
    }
    let from_where = format!(
        "FROM users JOIN user_options ON user_options.user_id = users.id WHERE {}",
        wheres.join(" AND ")
    );
    let now = crate::clock::now_naive();

    let count_sql = format!("SELECT count(*) {from_where}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql).bind(now);
    for b in &binds {
        count = count.bind(b);
    }
    let total = count.fetch_one(&mut *conn).await?;

    // The same cakedate in the order the list shows its users.
    let tie_breaker = if settings.get("prioritize_username_in_ux")?.truthy() {
        "users.username_lower"
    } else {
        "COALESCE(NULLIF(LOWER(TRIM(users.name)), ''), users.username_lower) ASC"
    };
    let (at, on) = match kind {
        Kind::Anniversaries => (
            format!("{column} AS cakedate_at"),
            "NULL::date AS cakedate_on".to_string(),
        ),
        Kind::Birthdays => (
            "NULL::timestamp AS cakedate_at".to_string(),
            format!("{column} AS cakedate_on"),
        ),
    };
    let sql = format!(
        "SELECT users.id, users.username, users.name, users.title, users.uploaded_avatar_id, {at}, {on} \
         {from_where} ORDER BY TO_CHAR({column}, 'MMDDYYYY') ASC, {tie_breaker} LIMIT {PAGE_SIZE} OFFSET {}",
        PAGE_SIZE * params.page
    );
    let mut query = sqlx::query_as::<_, Row>(&sql).bind(now);
    for b in &binds {
        query = query.bind(b);
    }
    let rows = query.fetch_all(&mut *conn).await?;

    let ids: Vec<i32> = rows.iter().map(|r| r.id).collect();
    if settings.get("enable_user_status")?.truthy() {
        let with_status: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_statuses WHERE user_id = ANY($1) \
             AND (ends_at IS NULL OR ends_at > $2))",
        )
        .bind(&ids)
        .bind(now)
        .fetch_one(&mut *conn)
        .await?;
        if with_status {
            return Err(Unsupported("user status on BasicUserSerializer").into());
        }
    }
    let names = settings.get("enable_names")?.truthy();
    let logo = crate::admin_users::logo_small_url(&mut *conn, settings).await?;
    let mut users = Vec::with_capacity(rows.len());
    for r in rows {
        let mut u = Map::new();
        u.insert("id".into(), json!(r.id));
        u.insert("username".into(), json!(r.username));
        if names {
            u.insert("name".into(), json!(r.name));
        }
        u.insert(
            "avatar_template".into(),
            json!(crate::avatar::avatar_template(
                urls,
                r.id,
                &r.username,
                r.uploaded_avatar_id,
                logo.as_deref()
            )?),
        );
        u.insert("title".into(), json!(r.title));
        let cakedate = match kind {
            Kind::Anniversaries => json!(r.cakedate_at.map(crate::topic_list::time_json)),
            Kind::Birthdays => json!(r.cakedate_on.map(super::date_json)),
        };
        u.insert("cakedate".into(), cakedate);
        users.push(Value::Object(u));
    }

    // anniversaries_path(more_params): its keys sorted, a nil one left out.
    more.sort_by_key(|(k, _)| *k);
    let query: Vec<String> = more
        .iter()
        .map(|(k, v)| format!("{k}={}", crate::ruby::cgi_escape(v)))
        .collect();
    let load_more = format!(
        "{}/cakeday/{}?{}",
        urls.config.globals.relative_url_root(),
        kind.key(),
        query.join("&")
    );
    Ok(Page {
        users,
        total,
        load_more,
    })
}

/// render_json_dump: `{<kind>: users, total_rows_<kind>:, load_more_<kind>:}`
pub fn document(kind: Kind, page: Page) -> Value {
    let key = kind.key();
    let mut out = Map::new();
    out.insert(key.into(), Value::Array(page.users));
    out.insert(format!("total_rows_{key}"), json!(page.total));
    out.insert(format!("load_more_{key}"), json!(page.load_more));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_read_as_to_i_and_clamp() {
        let p = Params::parse(None, Some("-3"), Some("15"));
        assert_eq!((p.page, p.month), (0, 12));
        let p = Params::parse(None, Some("2abc"), None);
        assert_eq!((p.page, p.month), (2, 1));
    }
}
