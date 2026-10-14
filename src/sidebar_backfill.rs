//! SidebarSiteSettingsBackfiller: the default navigation menu categories
//! and tags applied to existing users when the settings change (the
//! `backfill_sidebar_site_settings` job), and how many users that touches.

use serde_json::Value;
use sqlx::PgConnection;

use crate::{AppError, Unsupported};

/// The job's name.
pub const JOB: &str = "backfill_sidebar_site_settings";

/// Ruby's `String#split("|")`: nothing for an empty string, trailing empty
/// fields dropped.
fn split_pipes(value: &str) -> Vec<&str> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut parts: Vec<&str> = value.split('|').collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

pub struct Backfiller {
    linkable_type: &'static str,
    added: Vec<i32>,
    removed: Vec<i32>,
}

impl Backfiller {
    pub async fn new(
        conn: &mut PgConnection,
        setting_name: &str,
        previous_value: &str,
        new_value: &str,
    ) -> Result<Self, AppError> {
        let (linkable_type, previous, new): (&'static str, Vec<i32>, Vec<i32>) = match setting_name
        {
            "default_navigation_menu_categories" => {
                let ids = |v: &str| {
                    split_pipes(v)
                        .into_iter()
                        .map(|p| crate::ruby::to_i(p) as i32)
                        .collect()
                };
                ("Category", ids(previous_value), ids(new_value))
            }
            "default_navigation_menu_tags" => {
                // Tag.where(name:).pluck(:id), in the table's order.
                let mut lookup = async |v: &str| -> Result<Vec<i32>, sqlx::Error> {
                    let names: Vec<String> =
                        split_pipes(v).into_iter().map(str::to_string).collect();
                    sqlx::query_scalar("SELECT id FROM tags WHERE name = ANY($1)")
                        .bind(&names)
                        .fetch_all(&mut *conn)
                        .await
                };
                let previous = lookup(previous_value).await?;
                let new = lookup(new_value).await?;
                ("Tag", previous, new)
            }
            _ => return Err(Unsupported("backfilling a setting other than the sidebar's").into()),
        };
        Ok(Backfiller {
            linkable_type,
            added: new
                .iter()
                .filter(|i| !previous.contains(i))
                .copied()
                .collect(),
            removed: previous
                .iter()
                .filter(|i| !new.contains(i))
                .copied()
                .collect(),
        })
    }

    /// `number_of_users_to_backfill`
    pub async fn number_of_users_to_backfill(
        &self,
        conn: &mut PgConnection,
    ) -> Result<i64, sqlx::Error> {
        let mut selects = Vec::new();
        if !self.removed.is_empty() {
            selects.push(format!(
                "SELECT sidebar_section_links.user_id FROM sidebar_section_links \
                 WHERE sidebar_section_links.linkable_type = '{}' \
                 AND sidebar_section_links.linkable_id IN ({})",
                self.linkable_type,
                join(&self.removed)
            ));
        }
        if !self.added.is_empty() {
            selects.push(format!(
                "SELECT users.id FROM users WHERE users.id NOT IN ( \
                   SELECT DISTINCT(sidebar_section_links.user_id) FROM sidebar_section_links \
                   WHERE sidebar_section_links.linkable_type = '{}' \
                   AND sidebar_section_links.linkable_id IN ({})) \
                 AND users.id > 0 AND NOT users.staged",
                self.linkable_type,
                join(&self.added)
            ));
        }
        if selects.is_empty() {
            return Ok(0);
        }
        sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM ({}) AS user_ids",
            selects.join(" UNION DISTINCT ")
        ))
        .fetch_one(conn)
        .await
    }

    /// `backfill!`: by batches of 1000 real, unstaged users, the removed
    /// links deleted and the added ones inserted where missing.
    pub async fn backfill(&self, conn: &mut PgConnection) -> Result<(), sqlx::Error> {
        let mut after = i32::MIN;
        loop {
            let ids: Vec<i32> = sqlx::query_scalar(
                "SELECT id FROM users WHERE users.id > 0 \
                 AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
                 AND NOT staged AND id > $1 ORDER BY id LIMIT 1000",
            )
            .bind(after)
            .fetch_all(&mut *conn)
            .await?;
            let Some(&last) = ids.last() else {
                return Ok(());
            };
            after = last;
            if !self.removed.is_empty() {
                sqlx::query(
                    "DELETE FROM sidebar_section_links WHERE user_id = ANY($1) \
                     AND linkable_id = ANY($2) AND linkable_type = $3",
                )
                .bind(&ids)
                .bind(&self.removed)
                .bind(self.linkable_type)
                .execute(&mut *conn)
                .await?;
            }
            for user_id in &ids {
                for linkable_id in &self.added {
                    sqlx::query(
                        "INSERT INTO sidebar_section_links (user_id, linkable_type, linkable_id, \
                         created_at, updated_at) VALUES ($1, $2, $3, now(), now()) ON CONFLICT DO NOTHING",
                    )
                    .bind(user_id)
                    .bind(self.linkable_type)
                    .bind(linkable_id)
                    .execute(&mut *conn)
                    .await?;
                }
            }
        }
    }
}

fn join(ids: &[i32]) -> String {
    ids.iter().map(i32::to_string).collect::<Vec<_>>().join(",")
}

/// Jobs::BackfillSidebarSiteSettings
pub async fn run(state: &crate::AppState, args: &Value) -> Result<(), AppError> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default();
    let mut tx = state.pool.begin().await?;
    let backfiller = Backfiller::new(
        &mut tx,
        s("setting_name"),
        s("previous_value"),
        s("new_value"),
    )
    .await?;
    backfiller.backfill(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}
