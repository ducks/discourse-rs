//! The screened emails, IP addresses and URLs under /admin/logs:
//! Admin::ScreenedEmailsController, ScreenedIpAddressesController and
//! ScreenedUrlsController, their serializers, and ScreenedIpAddress's
//! address handling (`IPAddr.handle_wildcards`, `to_cidr_s`) and
//! validations.

use std::net::IpAddr;

use chrono::NaiveDateTime;
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::AppError;

/// `ScreeningModel.actions`
pub const ACTIONS: [(&str, i32); 3] = [("block", 1), ("do_nothing", 2), ("allow_admin", 3)];
pub const ALLOW_ADMIN: i32 = 3;
/// ScreenedIpAddress's `default_action :block`
const BLOCK: i32 = 1;

pub fn action_type(name: &str) -> Option<i32> {
    ACTIONS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

fn action_name(t: i32) -> &'static str {
    ACTIONS.iter().find(|(_, v)| *v == t).map_or("", |(n, _)| n)
}

fn iso(t: Option<NaiveDateTime>) -> Value {
    json!(t.map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()))
}

/// `IPAddr.handle_wildcards(val)`: `1.2.*` as `1.2.0.0/16`; None when
/// blank or when something other than dots follows the first `*`.
pub fn handle_wildcards(val: &str) -> Option<String> {
    if crate::ruby::is_blank(val) {
        return None;
    }
    let wildcards = val.matches('*').count();
    if wildcards == 0 {
        return Some(val.to_string());
    }
    // strip ranges like "/16" from the end
    let v = val.split('/').next().unwrap_or_default();
    let star = v.find('*')?;
    if v[star..].chars().any(|c| c != '.' && c != '*') {
        return None;
    }
    // Ruby's split drops trailing empty fields.
    let mut parts: Vec<&str> = v.split('.').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    while parts.len() < 4 {
        parts.push("*");
    }
    let v = parts.join(".");
    let bits = 32_i64 - (v.matches('*').count() as i64) * 8;
    Some(format!("{}/{bits}", v.replace('*', "0")))
}

/// An address as IPAddr holds it: the network of its prefix.
#[derive(Clone, Copy, PartialEq)]
pub struct Cidr {
    pub addr: IpAddr,
    pub prefix: u8,
}

impl Cidr {
    /// `IPAddr.new(s)`: an address with an optional `/prefix`, masked.
    pub fn parse(s: &str) -> Option<Cidr> {
        let (a, p) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = a.parse().ok()?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match p {
            None => max,
            Some(p) => {
                let n: u8 = p.parse().ok()?;
                if n > max {
                    return None;
                }
                n
            }
        };
        let addr = match addr {
            IpAddr::V4(v) => {
                let bits = u32::from(v);
                let mask = if prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                IpAddr::V4((bits & mask).into())
            }
            IpAddr::V6(v) => {
                let bits = u128::from(v);
                let mask = if prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - prefix)
                };
                IpAddr::V6((bits & mask).into())
            }
        };
        Some(Cidr { addr, prefix })
    }

    /// `IPAddr#to_cidr_s`: without the mask only at 32 bits.
    pub fn to_cidr_s(self) -> String {
        if self.prefix == 32 {
            self.addr.to_string()
        } else {
            format!("{}/{}", self.addr, self.prefix)
        }
    }

    /// As Postgres takes it.
    fn to_inet(self) -> String {
        format!("{}/{}", self.addr, self.prefix)
    }
}

/// `ScreenedIpAddress#ip_address_with_mask`, from the inet column.
const IP_WITH_MASK: &str = "CASE WHEN masklen(ip_address) = 32 THEN host(network(ip_address)) \
     ELSE host(network(ip_address)) || '/' || masklen(ip_address) END";

/// GET /admin/logs/screened_emails: ScreenedEmailSerializer, the 200 last
/// matched.
pub async fn emails(conn: &mut PgConnection, can_see_ip: bool) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        email: String,
        action_type: i32,
        match_count: i32,
        last_match_at: Option<NaiveDateTime>,
        created_at: NaiveDateTime,
        ip_address: Option<String>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, email, action_type, match_count, last_match_at, created_at, \
                host(network(ip_address)) AS ip_address \
         FROM screened_emails ORDER BY last_match_at DESC LIMIT 200",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|r| {
                let mut o = serde_json::Map::new();
                o.insert("email".into(), json!(r.email));
                o.insert("action".into(), json!(action_name(r.action_type)));
                o.insert("match_count".into(), json!(r.match_count));
                o.insert("last_match_at".into(), iso(r.last_match_at));
                o.insert("created_at".into(), iso(Some(r.created_at)));
                if can_see_ip {
                    o.insert("ip_address".into(), json!(r.ip_address));
                }
                o.insert("id".into(), json!(r.id));
                Value::Object(o)
            })
            .collect(),
    ))
}

/// GET /admin/logs/screened_urls: GroupedScreenedUrlSerializer, by domain.
pub async fn urls(conn: &mut PgConnection) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        domain: String,
        match_count: Option<i64>,
        last_match_at: Option<NaiveDateTime>,
        created_at: Option<NaiveDateTime>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT domain, SUM(match_count)::int8 AS match_count, MAX(last_match_at) AS last_match_at, \
                MIN(created_at) AS created_at \
         FROM screened_urls GROUP BY domain ORDER BY MAX(last_match_at) DESC",
    )
    .fetch_all(&mut *conn)
    .await?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|r| {
                json!({
                    "domain": r.domain,
                    "action": "do_nothing",
                    "match_count": r.match_count,
                    "last_match_at": iso(r.last_match_at),
                    "created_at": iso(r.created_at),
                })
            })
            .collect(),
    ))
}

#[derive(sqlx::FromRow)]
struct IpRow {
    id: i32,
    ip_address: String,
    action_type: i32,
    match_count: i32,
    last_match_at: Option<NaiveDateTime>,
    created_at: NaiveDateTime,
}

fn ip_json(r: &IpRow) -> Value {
    json!({
        "id": r.id,
        "ip_address": r.ip_address,
        "action_name": action_name(r.action_type),
        "match_count": r.match_count,
        "last_match_at": iso(r.last_match_at),
        "created_at": iso(Some(r.created_at)),
    })
}

fn ip_select() -> String {
    format!(
        "SELECT id, {IP_WITH_MASK} AS ip_address, action_type, match_count, last_match_at, created_at \
         FROM screened_ip_addresses"
    )
}

/// GET /admin/logs/screened_ip_addresses: the 200 most matched, those
/// overlapping `filter` when given; a filter Postgres can't read as a
/// cidr lists none (Rails rescues the StatementInvalid).
pub async fn ip_addresses(
    conn: &mut PgConnection,
    filter: Option<&str>,
) -> Result<Value, AppError> {
    let filter = filter.and_then(handle_wildcards);
    let rows: Result<Vec<IpRow>, sqlx::Error> = match &filter {
        Some(f) => {
            sqlx::query_as(&format!(
                "{} WHERE $1::cidr >>= ip_address OR ip_address >>= $1::cidr \
                 ORDER BY match_count DESC LIMIT 200",
                ip_select()
            ))
            .bind(f)
            .fetch_all(&mut *conn)
            .await
        }
        None => {
            sqlx::query_as(&format!(
                "{} ORDER BY match_count DESC LIMIT 200",
                ip_select()
            ))
            .fetch_all(&mut *conn)
            .await
        }
    };
    let rows = match rows {
        Ok(rows) => rows,
        Err(sqlx::Error::Database(e)) if e.code().is_some_and(|c| c.starts_with("22")) => {
            Vec::new()
        }
        Err(e) => return Err(e.into()),
    };
    Ok(Value::Array(rows.iter().map(ip_json).collect()))
}

/// What creating or updating a screened IP comes to.
pub enum Saved {
    Ok(Value),
    /// The model's full error messages (422).
    Invalid(Vec<String>),
    NotFound,
}

/// `ScreenedIpAddress.new(params).save` or `#update(params)`: the address
/// through `ip_address=` (wildcards, IPAddr), the default action, the
/// format and presence validations, and `check_for_match` when the
/// address changes.
pub async fn save_ip(
    conn: &mut PgConnection,
    id: Option<i32>,
    ip_address: &str,
    action: Option<i32>,
) -> Result<Saved, AppError> {
    let existing: Option<(String, i32)> = match id {
        Some(id) => {
            let row: Option<(String, i32)> = sqlx::query_as(
                "SELECT host(ip_address) || '/' || masklen(ip_address), action_type \
                 FROM screened_ip_addresses WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
            if row.is_none() {
                return Ok(Saved::NotFound);
            }
            row
        }
        None => None,
    };
    let cidr = handle_wildcards(ip_address).and_then(|v| Cidr::parse(&v));
    let Some(cidr) = cidr else {
        return Ok(Saved::Invalid(vec![
            "Ip address is invalid".into(),
            "Ip address can't be blank".into(),
        ]));
    };
    let action = action
        .or(existing.as_ref().map(|(_, t)| *t))
        .unwrap_or(BLOCK);
    let old = existing.as_ref().and_then(|(ip, _)| Cidr::parse(ip));
    if old != Some(cidr) {
        // check_for_match: the narrowest rule containing it, same action.
        let matched: Option<i32> = sqlx::query_scalar(
            "SELECT action_type FROM screened_ip_addresses WHERE $1::inet <<= ip_address \
             ORDER BY masklen(ip_address) DESC LIMIT 1",
        )
        .bind(cidr.to_cidr_s())
        .fetch_optional(&mut *conn)
        .await?;
        if matched == Some(action) {
            return Ok(Saved::Invalid(vec![
                "Ip address is already included in an existing rule".into(),
            ]));
        }
    }
    let row: IpRow = match id {
        None => {
            sqlx::query_as(&format!(
                "WITH saved AS (INSERT INTO screened_ip_addresses \
                   (ip_address, action_type, match_count, created_at, updated_at) \
                   VALUES ($1::inet, $2, 0, clock_timestamp(), clock_timestamp()) RETURNING *) \
                 SELECT id, {IP_WITH_MASK} AS ip_address, action_type, match_count, last_match_at, created_at \
                 FROM saved"
            ))
            .bind(cidr.to_inet())
            .bind(action)
            .fetch_one(&mut *conn)
            .await?
        }
        Some(id) => {
            let changed = old != Some(cidr) || existing.as_ref().map(|(_, t)| *t) != Some(action);
            sqlx::query_as(&format!(
                "WITH saved AS (UPDATE screened_ip_addresses SET ip_address = $2::inet, action_type = $3, \
                   updated_at = CASE WHEN $4 THEN clock_timestamp() ELSE updated_at END \
                   WHERE id = $1 RETURNING *) \
                 SELECT id, {IP_WITH_MASK} AS ip_address, action_type, match_count, last_match_at, created_at \
                 FROM saved"
            ))
            .bind(id)
            .bind(cidr.to_inet())
            .bind(action)
            .bind(changed)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    Ok(Saved::Ok(ip_json(&row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert_eq!(
            handle_wildcards("198.51.*.*").as_deref(),
            Some("198.51.0.0/16")
        );
        assert_eq!(handle_wildcards("203.0.*").as_deref(), Some("203.0.0.0/16"));
        assert_eq!(handle_wildcards("10.*/8").as_deref(), Some("10.0.0.0/8"));
        assert_eq!(handle_wildcards("1.*.2"), None);
        assert_eq!(handle_wildcards("1.2.3.4").as_deref(), Some("1.2.3.4"));
        assert_eq!(handle_wildcards(""), None);
    }

    #[test]
    fn cidr_strings() {
        let c = |s: &str| Cidr::parse(s).map(|c| c.to_cidr_s());
        assert_eq!(c("203.0.113.5").as_deref(), Some("203.0.113.5"));
        assert_eq!(c("198.51.100.77/24").as_deref(), Some("198.51.100.0/24"));
        assert_eq!(c("2001:db8::1").as_deref(), Some("2001:db8::1/128"));
        assert_eq!(c("nonsense"), None);
    }
}
