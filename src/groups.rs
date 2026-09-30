//! The group columns user serializers need (UserLookup's group select,
//! Group#flair_url), with PrimaryGroupSerializer and FlairGroupSerializer.

use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::Unsupported;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Group {
    pub id: i32,
    pub name: String,
    pub flair_icon: Option<String>,
    pub flair_upload_id: Option<i32>,
    pub flair_bg_color: Option<String>,
    pub flair_color: Option<String>,
    /// Joined from uploads for flair_upload_id.
    pub flair_upload_url: Option<String>,
}

impl Group {
    /// `Group#flair_url`: the icon name, else the upload's URL (no CDN yet).
    pub fn flair_url(&self) -> Result<Option<String>, Unsupported> {
        if let Some(icon) = self.flair_icon.as_deref().filter(|i| !i.is_empty()) {
            return Ok(Some(icon.to_string()));
        }
        match (&self.flair_upload_id, &self.flair_upload_url) {
            (Some(_), Some(url)) => Ok(Some(url.clone())),
            (Some(_), None) => Err(Unsupported("group flair upload without an upload row")),
            (None, _) => Ok(None),
        }
    }

    /// PrimaryGroupSerializer
    pub fn primary_json(&self) -> Value {
        json!({"id": self.id, "name": self.name})
    }

    /// FlairGroupSerializer
    pub fn flair_json(&self) -> Result<Value, Unsupported> {
        Ok(json!({
            "id": self.id,
            "name": self.name,
            "flair_url": self.flair_url()?,
            "flair_bg_color": self.flair_bg_color,
            "flair_color": self.flair_color,
        }))
    }
}

pub async fn load(
    conn: &mut PgConnection,
    ids: &[i32],
) -> Result<HashMap<i32, Group>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let groups: Vec<Group> = sqlx::query_as(
        "SELECT g.id, g.name, g.flair_icon, g.flair_upload_id, g.flair_bg_color, g.flair_color, \
                u.url AS flair_upload_url \
         FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id WHERE g.id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(conn)
    .await?;
    Ok(groups.into_iter().map(|g| (g.id, g)).collect())
}

/// UserFlairMixin's attributes for a user's flair group, each only when
/// present, followed by UserPrimaryGroupMixin's primary_group_name.
pub fn user_group_fields(
    out: &mut serde_json::Map<String, Value>,
    groups: &HashMap<i32, Group>,
    primary_group_id: Option<i32>,
    flair_group_id: Option<i32>,
) -> Result<(), Unsupported> {
    if let Some(flair) = flair_group_id.and_then(|id| groups.get(&id)) {
        out.insert("flair_name".into(), json!(flair.name));
        if let Some(url) = flair.flair_url()? {
            out.insert("flair_url".into(), json!(url));
        }
        if let Some(c) = flair.flair_bg_color.as_deref().filter(|c| !c.is_empty()) {
            out.insert("flair_bg_color".into(), json!(c));
        }
        if let Some(c) = flair.flair_color.as_deref().filter(|c| !c.is_empty()) {
            out.insert("flair_color".into(), json!(c));
        }
    }
    if let Some(id) = flair_group_id {
        out.insert("flair_group_id".into(), json!(id));
    }
    if let Some(primary) = primary_group_id.and_then(|id| groups.get(&id)) {
        out.insert("primary_group_name".into(), json!(primary.name));
    }
    Ok(())
}
