//! The vendored Discourse schema (schema/structure.sql, written by
//! scripts/vendor-schema) and a check that a database matches it.
//!
//! discourse-rs owns no migrations. It runs against a database built from
//! structure.sql, or against a real Discourse database, and refuses to start
//! if any migration it was built against is missing.

use std::collections::HashSet;
use std::fmt;

use sqlx::PgConnection;

const STRUCTURE_SQL: &str = include_str!("../schema/structure.sql");
const DISCOURSE_REF: &str = include_str!("../schema/DISCOURSE_REF");

/// The Discourse commit structure.sql was vendored from.
pub fn discourse_commit() -> &'static str {
    DISCOURSE_REF
        .lines()
        .find_map(|l| l.strip_prefix("commit="))
        .unwrap_or("unknown")
}

/// Schema versions listed in the trailing `INSERT INTO schema_migrations`
/// block of structure.sql, one `('20120311163914'),` per line.
pub fn expected_versions() -> Vec<&'static str> {
    parse_versions(STRUCTURE_SQL)
}

fn parse_versions(sql: &str) -> Vec<&str> {
    sql.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("('")?;
            let inner = rest
                .strip_suffix("'),")
                .or_else(|| rest.strip_suffix("');"))?;
            (!inner.is_empty() && inner.bytes().all(|b| b.is_ascii_digit())).then_some(inner)
        })
        .collect()
}

#[derive(Debug)]
pub struct SchemaReport {
    pub expected: usize,
    /// Versions in the database but not in the vendored schema, e.g. from
    /// newer Discourse or third-party plugins. Tolerated, but reported.
    pub extra: Vec<String>,
}

#[derive(Debug)]
pub enum SchemaError {
    Db(sqlx::Error),
    Missing(Vec<String>),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Db(e) => write!(f, "reading schema_migrations: {e}"),
            SchemaError::Missing(v) => write!(
                f,
                "database is missing {} schema versions from Discourse {} (first: {}); \
                 load schema/structure.sql or migrate the Discourse database",
                v.len(),
                discourse_commit(),
                v.first().map(String::as_str).unwrap_or("-"),
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<sqlx::Error> for SchemaError {
    fn from(e: sqlx::Error) -> Self {
        SchemaError::Db(e)
    }
}

pub async fn verify(conn: &mut PgConnection) -> Result<SchemaReport, SchemaError> {
    let present: Vec<String> = sqlx::query_scalar("SELECT version FROM schema_migrations")
        .fetch_all(conn)
        .await?;
    compare(&expected_versions(), &present)
}

fn compare(expected: &[&str], present: &[String]) -> Result<SchemaReport, SchemaError> {
    let present_set: HashSet<&str> = present.iter().map(String::as_str).collect();
    let mut missing: Vec<String> = expected
        .iter()
        .filter(|v| !present_set.contains(*v))
        .map(|v| v.to_string())
        .collect();
    if !missing.is_empty() {
        missing.sort();
        return Err(SchemaError::Missing(missing));
    }

    let expected_set: HashSet<&str> = expected.iter().copied().collect();
    let mut extra: Vec<String> = present
        .iter()
        .filter(|v| !expected_set.contains(v.as_str()))
        .cloned()
        .collect();
    extra.sort();

    Ok(SchemaReport {
        expected: expected.len(),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_schema_migrations_block() {
        let sql = "INSERT INTO \"schema_migrations\" (version) VALUES\n('20260101000000'),\n('20120311163914'),\n('2000');\n";
        assert_eq!(
            parse_versions(sql),
            vec!["20260101000000", "20120311163914", "2000"]
        );
    }

    #[test]
    fn ignores_non_version_lines() {
        assert!(parse_versions("('abc'),\n  ('1'),\nCREATE TABLE x ();").is_empty());
    }

    #[test]
    fn vendored_schema_has_versions_and_a_commit() {
        assert!(expected_versions().len() > 1000);
        assert_eq!(discourse_commit().len(), 40);
    }

    #[test]
    fn missing_versions_are_an_error() {
        let err = compare(&["1", "2", "3"], &["1".into()]).unwrap_err();
        match err {
            SchemaError::Missing(v) => assert_eq!(v, vec!["2", "3"]),
            other => panic!("expected Missing, got {other:?}"),
        }
    }

    #[test]
    fn extra_versions_are_reported_not_rejected() {
        let report = compare(&["1"], &["1".into(), "9".into()]).unwrap();
        assert_eq!(report.expected, 1);
        assert_eq!(report.extra, vec!["9"]);
    }
}
