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

/// The columns BasicGroupSerializer reads, aliased for `BasicGroup`.
pub const BASIC_GROUP_COLUMNS: &str = "g.id, g.automatic, g.name, g.user_count, g.mentionable_level, \
    g.messageable_level, g.visibility_level, g.primary_group, g.title, g.grant_trust_level, \
    g.flair_icon, g.flair_upload_id, u.url AS flair_upload_url, g.flair_bg_color, g.flair_color, \
    g.bio_cooked, g.public_admission, g.public_exit, g.allow_membership_requests, g.full_name, \
    g.default_notification_level, g.membership_request_template, g.members_visibility_level, \
    g.publish_read_state";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BasicGroup {
    pub id: i32,
    pub automatic: bool,
    pub name: String,
    pub user_count: i32,
    pub mentionable_level: i32,
    pub messageable_level: i32,
    pub visibility_level: i32,
    pub primary_group: bool,
    pub title: Option<String>,
    pub grant_trust_level: Option<i32>,
    pub flair_icon: Option<String>,
    pub flair_upload_id: Option<i32>,
    pub flair_upload_url: Option<String>,
    pub flair_bg_color: Option<String>,
    pub flair_color: Option<String>,
    pub bio_cooked: Option<String>,
    pub public_admission: bool,
    pub public_exit: bool,
    pub allow_membership_requests: bool,
    pub full_name: Option<String>,
    pub default_notification_level: i32,
    pub membership_request_template: Option<String>,
    pub members_visibility_level: i32,
    pub publish_read_state: bool,
}

impl BasicGroup {
    /// BasicGroupSerializer for an anonymous reader.
    pub fn json(&self, i18n: &crate::i18n::I18n) -> Result<Value, Unsupported> {
        let mut out = serde_json::Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("automatic".into(), json!(self.automatic));
        out.insert("name".into(), json!(self.name));
        if self.automatic {
            out.insert(
                "display_name".into(),
                json!(i18n.t(&format!("groups.default_names.{}", self.name))),
            );
        }
        // can_see_group_members?: public members for anonymous readers.
        let can_see_members = self.members_visibility_level == 0;
        if can_see_members {
            out.insert("user_count".into(), json!(self.user_count));
        }
        out.insert("mentionable_level".into(), json!(self.mentionable_level));
        out.insert("messageable_level".into(), json!(self.messageable_level));
        out.insert("visibility_level".into(), json!(self.visibility_level));
        out.insert("primary_group".into(), json!(self.primary_group));
        out.insert("title".into(), json!(self.title));
        out.insert("grant_trust_level".into(), json!(self.grant_trust_level));
        let flair = Group {
            id: self.id,
            name: self.name.clone(),
            flair_icon: self.flair_icon.clone(),
            flair_upload_id: self.flair_upload_id,
            flair_bg_color: self.flair_bg_color.clone(),
            flair_color: self.flair_color.clone(),
            flair_upload_url: self.flair_upload_url.clone(),
        };
        out.insert("flair_url".into(), json!(flair.flair_url()?));
        out.insert("flair_bg_color".into(), json!(self.flair_bg_color));
        out.insert("flair_color".into(), json!(self.flair_color));
        let bio_cooked = if self.automatic {
            i18n.t(&format!("groups.default_descriptions.{}", self.name))
                .map(str::to_string)
        } else {
            self.bio_cooked.clone()
        };
        out.insert("bio_cooked".into(), json!(bio_cooked));
        if bio_cooked.as_deref().is_some_and(|b| !b.is_empty()) {
            return Err(Unsupported("group bio_excerpt (PrettyText.excerpt)"));
        }
        out.insert("bio_excerpt".into(), Value::Null);
        out.insert("public_admission".into(), json!(self.public_admission));
        out.insert("public_exit".into(), json!(self.public_exit));
        out.insert(
            "allow_membership_requests".into(),
            json!(self.allow_membership_requests),
        );
        out.insert("full_name".into(), json!(self.full_name));
        out.insert(
            "default_notification_level".into(),
            json!(self.default_notification_level),
        );
        out.insert(
            "membership_request_template".into(),
            json!(self.membership_request_template),
        );
        out.insert(
            "members_visibility_level".into(),
            json!(self.members_visibility_level),
        );
        out.insert("can_see_members".into(), json!(can_see_members));
        out.insert("publish_read_state".into(), json!(self.publish_read_state));
        Ok(Value::Object(out))
    }
}
