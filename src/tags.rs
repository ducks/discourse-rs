//! Tags: lookup (app/models/tag.rb), visibility
//! (lib/discourse_tagging.rb visible_tags / hidden_tags / staff_tag_names)
//! and TagSerializer.

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::guardian::{Guardian, GuardianError};
use crate::site_settings::{SettingError, SiteSettings};

/// `DiscourseTagging.visible_tags(guardian)` as a WHERE fragment over
/// `tags`: everything for admins; otherwise not in a permissioned tag
/// group unless one grants a permitted group (everyone, or one of the
/// user's), and not restricted to categories outside the allowed ones.
pub fn visible_tags_where(
    guardian: &Guardian,
    settings: &SiteSettings,
) -> Result<String, SettingError> {
    if guardian.is_admin() {
        return Ok("TRUE".to_string());
    }
    let permitted_groups = match guardian.user_id() {
        Some(id) => format!("SELECT 0 UNION SELECT group_id FROM group_users WHERE user_id = {id}"),
        None => "SELECT 0".to_string(),
    };
    let allowed = guardian.allowed_category_ids_sql(settings)?;
    Ok(format!(
        "(tags.id NOT IN ( \
            SELECT tgm.tag_id FROM tag_group_memberships tgm \
            JOIN tag_groups tg ON tg.id = tgm.tag_group_id \
            JOIN tag_group_permissions tgp ON tgp.tag_group_id = tg.id) \
         OR tags.id IN ( \
            SELECT tgm.tag_id FROM tag_group_permissions tgp \
            JOIN tag_groups tg ON tg.id = tgp.tag_group_id \
            JOIN tag_group_memberships tgm ON tgm.tag_group_id = tg.id \
            WHERE tgp.group_id IN ({permitted_groups}))) \
        AND (tags.id NOT IN ( \
            SELECT tag_id FROM category_tags \
            UNION SELECT tgm.tag_id FROM tag_group_memberships tgm \
            JOIN category_tag_groups ctg ON ctg.tag_group_id = tgm.tag_group_id) \
         OR tags.id IN ( \
            SELECT tag_id FROM category_tags \
            WHERE category_id IN ({allowed}) \
            UNION SELECT tgm.tag_id FROM tag_group_memberships tgm \
            JOIN category_tag_groups ctg ON ctg.tag_group_id = tgm.tag_group_id \
            AND ctg.category_id IN ({allowed})))"
    ))
}

/// `tags.id IN (visible tag ids)`, for joins from topic_tags.
pub fn visible_tag_ids_subquery(
    guardian: &Guardian,
    settings: &SiteSettings,
) -> Result<String, SettingError> {
    Ok(format!(
        "(SELECT tags.id FROM tags WHERE {})",
        visible_tags_where(guardian, settings)?
    ))
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Tag {
    pub id: i32,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub description_cooked: Option<String>,
    pub public_topic_count: i32,
    pub staff_topic_count: i32,
    pub pm_topic_count: i32,
    pub target_tag_id: Option<i32>,
}

const COLUMNS: &str = "id, name, slug, description, description_cooked, public_topic_count, staff_topic_count, pm_topic_count, target_tag_id";

impl Tag {
    /// `Tag.find_by_name`: case-insensitive.
    pub async fn find_by_name(
        conn: &mut PgConnection,
        name: &str,
    ) -> Result<Option<Tag>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM tags WHERE lower(name) = $1 LIMIT 1"
        ))
        .bind(name.to_lowercase())
        .fetch_optional(conn)
        .await
    }

    pub async fn find(conn: &mut PgConnection, id: i32) -> Result<Option<Tag>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM tags WHERE id = $1"))
            .bind(id)
            .fetch_optional(conn)
            .await
    }

    /// `Tag.where(id: ids)`, by id.
    pub async fn find_all(conn: &mut PgConnection, ids: &[i32]) -> Result<Vec<Tag>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM tags WHERE id = ANY($1) ORDER BY id"
        ))
        .bind(ids)
        .fetch_all(conn)
        .await
    }

    /// `slug_for_url`
    pub fn slug_for_url(&self) -> String {
        match self.slug.as_deref().filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => format!("{}-tag", self.id),
        }
    }

    /// `Tag#url`: `/tag/<slug>/<id>`.
    pub fn url(&self, base_path: &str) -> String {
        format!("{base_path}/tag/{}/{}", self.slug_for_url(), self.id)
    }

    /// `guardian.can_see_tag?`: not among hidden_tag_names.
    pub async fn visible_to_anonymous(&self, conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
        let hidden: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
               SELECT 1 FROM tags WHERE tags.id = $1 \
               AND tags.id IN (SELECT tgm.tag_id FROM tag_group_memberships tgm \
                               JOIN tag_group_permissions tgp ON tgp.tag_group_id = tgm.tag_group_id) \
               AND tags.id NOT IN (SELECT tgm.tag_id FROM tag_group_memberships tgm \
                                   JOIN tag_group_permissions tgp ON tgp.tag_group_id = tgm.tag_group_id \
                                   WHERE tgp.group_id = 0))",
        )
        .bind(self.id)
        .fetch_one(conn)
        .await?;
        Ok(!hidden)
    }

    /// `DiscourseTagging.staff_tag_names.include?(name)`: in a group that is
    /// read-only (permission 3) for everyone.
    pub async fn is_staff(&self, conn: &mut PgConnection) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM tag_group_memberships tgm \
             JOIN tag_group_permissions tgp ON tgp.tag_group_id = tgm.tag_group_id \
             WHERE tgm.tag_id = $1 AND tgp.group_id = 0 AND tgp.permission_type = 3)",
        )
        .bind(self.id)
        .fetch_one(conn)
        .await
    }

    /// TagSerializer for anonymous users (public_topic_count).
    /// TagSerializer: `topic_count` is the staff count for staff.
    pub async fn serialize(
        &self,
        conn: &mut PgConnection,
        guardian: &Guardian,
        settings: &SiteSettings,
    ) -> Result<Value, GuardianError> {
        let topic_count = if guardian.tag_count_column(settings)? == "staff_topic_count" {
            self.staff_topic_count
        } else {
            self.public_topic_count
        };
        Ok(json!({
            "id": self.id,
            "name": self.name,
            "slug": self.slug_for_url(),
            "topic_count": topic_count,
            "staff": self.is_staff(conn).await?,
            "description": self.description,
            "description_cooked": self.description_cooked,
        }))
    }

    /// `tag_counts_json` entry on /tags.json for a base tag.
    pub fn counts_json(&self) -> Value {
        json!({
            "id": self.id,
            "text": self.name,
            "name": self.name,
            "slug": self.slug_for_url(),
            "description": self.description,
            "count": self.public_topic_count,
            "pm_only": self.public_topic_count == 0 && self.pm_topic_count > 0,
            "target_tag": null,
        })
    }
}

/// `TopicQuery#filter_by_tags` resolution: visible tags named (case
/// insensitively) in `names`, each mapped to its synonym target or itself,
/// deduplicated.
pub async fn resolve_tag_ids(
    conn: &mut PgConnection,
    guardian: &Guardian,
    settings: &SiteSettings,
    names: &[String],
) -> Result<Vec<i32>, GuardianError> {
    let lowered: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let rows: Vec<(i32, Option<i32>)> = sqlx::query_as(&format!(
        "SELECT id, target_tag_id FROM tags WHERE {} AND lower(name) = ANY($1)",
        visible_tags_where(guardian, settings)?
    ))
    .bind(&lowered)
    .fetch_all(conn)
    .await?;
    let mut ids: Vec<i32> = rows
        .into_iter()
        .map(|(id, target)| target.unwrap_or(id))
        .collect();
    let mut seen = Vec::new();
    ids.retain(|id| {
        if seen.contains(id) {
            false
        } else {
            seen.push(*id);
            true
        }
    });
    Ok(ids)
}
