//! The group columns user serializers need (UserLookup's group select,
//! Group#flair_url), with PrimaryGroupSerializer and FlairGroupSerializer.

use crate::guardian::Guardian;
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
    g.publish_read_state, g.incoming_email, g.bio_raw, g.has_messages";

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
    pub incoming_email: Option<String>,
    pub bio_raw: Option<String>,
    pub has_messages: bool,
}

impl BasicGroup {
    /// BasicGroupSerializer for `guardian`, with the viewer's membership
    /// of this group (`member`, `owner`) when they have one.
    pub fn json(
        &self,
        i18n: &crate::i18n::I18n,
        guardian: &Guardian,
        settings: &crate::site_settings::SiteSettings,
        membership: Option<(bool, bool)>,
    ) -> Result<Value, crate::guardian::GuardianError> {
        let (member, owner) = membership.unwrap_or((false, false));
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
        // can_see_group_members?
        let level = self.members_visibility_level;
        let can_see_members = guardian.is_admin()
            || level == 0
            || (guardian.is_staff() && (level == 3 || level == 2))
            || (guardian.is_authenticated() && level == 1)
            || owner
            || (member && level == 2);
        if can_see_members {
            out.insert("user_count".into(), json!(self.user_count));
        }
        out.insert("mentionable_level".into(), json!(self.mentionable_level));
        out.insert("messageable_level".into(), json!(self.messageable_level));
        out.insert("visibility_level".into(), json!(self.visibility_level));
        out.insert("primary_group".into(), json!(self.primary_group));
        out.insert("title".into(), json!(self.title));
        out.insert("grant_trust_level".into(), json!(self.grant_trust_level));
        if guardian.is_staff() {
            out.insert("incoming_email".into(), json!(self.incoming_email));
        }
        // can_see_group_messages?
        let sees_messages = guardian.is_admin()
            || (guardian.is_moderator()
                && self.id == crate::guardian::auto_groups::MODERATORS as i32)
            || (member
                && guardian.in_setting_groups(settings, "personal_message_enabled_groups")?);
        if guardian.is_staff() || sees_messages {
            out.insert("has_messages".into(), json!(self.has_messages));
        }
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
        if guardian.is_staff() || owner {
            out.insert("bio_raw".into(), json!(self.bio_raw));
        }
        let bio_cooked = if self.automatic {
            i18n.t(&format!("groups.default_descriptions.{}", self.name))
                .map(str::to_string)
        } else {
            self.bio_cooked.clone()
        };
        out.insert("bio_cooked".into(), json!(bio_cooked));
        let bio_excerpt = bio_cooked.as_deref().filter(|b| !b.is_empty()).map(|b| {
            crate::excerpt::excerpt(
                b,
                200,
                &crate::excerpt::Options {
                    keep_emoji_images: true,
                    ..Default::default()
                },
            )
        });
        out.insert("bio_excerpt".into(), json!(bio_excerpt));
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
        // can_admin_group? / can_edit_group?
        let can_admin = guardian.is_admin()
            || (settings.get("moderators_manage_groups")?.truthy()
                && guardian.is_moderator()
                && self.id != crate::guardian::auto_groups::ADMINS as i32);
        if can_admin {
            out.insert("can_admin_group".into(), json!(true));
        }
        if !self.automatic && (can_admin || owner) {
            out.insert("can_edit_group".into(), json!(true));
        }
        out.insert("publish_read_state".into(), json!(self.publish_read_state));
        Ok(Value::Object(out))
    }
}

/// `Group.members_visible_groups(user)` over the `groups` table aliased
/// `alias`: real groups (not everyone, anonymous or logged-in users)
/// whose members the user may see, by members_visibility_level.
pub fn members_visible_groups_where(guardian: &Guardian, alias: &str) -> String {
    let real = format!("{alias}.id > 0 AND {alias}.id NOT IN (4, 5)");
    let visible = match guardian.user() {
        None => format!("{alias}.members_visibility_level = 0"),
        Some(u) if u.admin => "TRUE".to_string(),
        Some(u) if u.moderator => format!(
            "({alias}.members_visibility_level IN (0, 1, 2, 3) OR {alias}.id IN (\
                SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
                AND gu.user_id = {} AND gu.owner WHERE g.members_visibility_level = 4))",
            u.id
        ),
        Some(u) => format!(
            "{alias}.id IN (\
                SELECT id FROM groups WHERE members_visibility_level IN (0, 1) \
                UNION ALL SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
                AND gu.user_id = {uid} WHERE g.members_visibility_level = 2 \
                UNION ALL SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
                AND gu.user_id = {uid} AND gu.owner WHERE g.members_visibility_level IN (3, 4))",
            uid = u.id
        ),
    };
    format!("{real} AND {visible}")
}

/// `Guardian#can_see_group_and_members?(group)`: `can_see_group?` on
/// its visibility_level and `can_see_group_members?` on its
/// members_visibility_level, which follow the same rule.
pub async fn can_see_group_and_members(
    conn: &mut PgConnection,
    guardian: &Guardian,
    group_id: i32,
    visibility_level: i32,
    members_visibility_level: i32,
) -> Result<bool, sqlx::Error> {
    // public 0, logged_on_users 1, members 2, staff 3, owners 4
    let mut membership: Option<Option<bool>> = None;
    for level in [visibility_level, members_visibility_level] {
        if guardian.is_admin() || level == 0 {
            continue;
        }
        if guardian.is_staff() && (level == 3 || level == 2) {
            continue;
        }
        if guardian.is_authenticated() && level == 1 {
            continue;
        }
        let Some(user_id) = guardian.user_id() else {
            return Ok(false);
        };
        if membership.is_none() {
            membership = Some(
                sqlx::query_scalar(
                    "SELECT owner FROM group_users WHERE group_id = $1 AND user_id = $2",
                )
                .bind(group_id)
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?,
            );
        }
        match membership.flatten() {
            None => return Ok(false),
            Some(true) => continue,
            Some(false) if level == 4 || level == 3 => return Ok(false),
            Some(false) => continue,
        }
    }
    Ok(true)
}

/// `Group.visible_groups(user)`'s visibility clause over the `groups`
/// table aliased `alias`: public groups for anonymous users, everything
/// for admins, by visibility level and membership or ownership otherwise.
pub fn visible_groups_where(guardian: &Guardian, alias: &str) -> String {
    match guardian.user() {
        None => format!("{alias}.visibility_level = 0"),
        Some(u) if u.admin => "TRUE".to_string(),
        Some(u) if u.moderator => format!(
            "({alias}.visibility_level IN (0, 1, 2, 3) OR {alias}.id IN (\
                SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
                AND gu.user_id = {} AND gu.owner WHERE g.visibility_level = 4))",
            u.id
        ),
        Some(u) => format!(
            "{alias}.id IN (\
                SELECT id FROM groups WHERE visibility_level IN (0, 1) \
                UNION ALL SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
                AND gu.user_id = {uid} WHERE g.visibility_level = 2 \
                UNION ALL SELECT g.id FROM groups g JOIN group_users gu ON gu.group_id = g.id \
                AND gu.user_id = {uid} AND gu.owner WHERE g.visibility_level IN (3, 4))",
            uid = u.id
        ),
    }
}

/// `Group.visible_groups(user, order, include_everyone:,
/// include_pseudogroups:)` without its order: the pseudo groups left out
/// unless asked for, then `visible_groups_where`.
pub fn visible_groups_scope(
    guardian: &Guardian,
    alias: &str,
    include_everyone: bool,
    include_pseudogroups: bool,
) -> String {
    let mut parts = Vec::new();
    if !include_everyone {
        parts.push(format!("{alias}.id > 0"));
    }
    if !include_pseudogroups {
        parts.push(format!("{alias}.id NOT IN (4, 5)"));
    }
    parts.push(format!("({})", visible_groups_where(guardian, alias)));
    parts.join(" AND ")
}

/// A group by `LOWER(name)` or by id, with BasicGroup's columns.
pub async fn find_basic(
    conn: &mut PgConnection,
    name: Option<&str>,
    id: Option<i32>,
) -> Result<Option<BasicGroup>, sqlx::Error> {
    let sql = format!(
        "SELECT {BASIC_GROUP_COLUMNS} FROM groups g LEFT JOIN uploads u ON u.id = g.flair_upload_id \
         WHERE ($1::text IS NOT NULL AND LOWER(g.name) = LOWER($1)) OR g.id = $2 LIMIT 1"
    );
    sqlx::query_as(&sql)
        .bind(name)
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// The viewer's `group_users` row on a group, as `Some(owner)`.
pub async fn membership(
    conn: &mut PgConnection,
    guardian: &Guardian,
    group_id: i32,
) -> Result<Option<bool>, sqlx::Error> {
    let Some(user_id) = guardian.user_id() else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT owner FROM group_users WHERE group_id = $1 AND user_id = $2")
        .bind(group_id)
        .bind(user_id)
        .fetch_optional(conn)
        .await
}

/// `can_see_group?` (`level` the visibility_level) and
/// `can_see_group_members?` (`level` the members_visibility_level): the
/// same rule over the viewer's membership.
pub fn can_see_level(guardian: &Guardian, level: i32, membership: Option<bool>) -> bool {
    // public 0, logged_on_users 1, members 2, staff 3, owners 4
    if guardian.is_admin() || level == 0 {
        return true;
    }
    if guardian.is_staff() && (level == 3 || level == 2) {
        return true;
    }
    if guardian.is_authenticated() && level == 1 {
        return true;
    }
    match membership {
        None => false,
        Some(true) => true,
        Some(false) => level != 4 && level != 3,
    }
}

/// `Guardian#can_admin_group?`: admins, or moderators when
/// moderators_manage_groups for a group they can see other than admins.
pub fn can_admin_group(
    guardian: &Guardian,
    settings: &crate::site_settings::SiteSettings,
    group: &BasicGroup,
    membership: Option<bool>,
) -> Result<bool, crate::site_settings::SettingError> {
    Ok(guardian.is_admin()
        || (settings.get("moderators_manage_groups")?.truthy()
            && guardian.is_moderator()
            && can_see_level(guardian, group.visibility_level, membership)
            && group.id != crate::guardian::auto_groups::ADMINS as i32))
}

/// `Guardian#can_edit_group?`: not automatic, and an admin of it or an
/// owner.
pub fn can_edit_group(
    guardian: &Guardian,
    settings: &crate::site_settings::SiteSettings,
    group: &BasicGroup,
    membership: Option<bool>,
) -> Result<bool, crate::site_settings::SettingError> {
    Ok(!group.automatic
        && (can_admin_group(guardian, settings, group, membership)? || membership == Some(true)))
}

/// Inserts `key` before `before` in a serialized object (at the end when
/// `before` is missing), keeping the serializer's attribute order.
pub fn insert_before(
    out: &mut serde_json::Map<String, Value>,
    before: &str,
    key: &str,
    value: Value,
) {
    let mut rebuilt = serde_json::Map::new();
    let mut value = Some(value);
    for (k, v) in std::mem::take(out) {
        if k == before
            && let Some(value) = value.take()
        {
            rebuilt.insert(key.to_string(), value);
        }
        rebuilt.insert(k, v);
    }
    if let Some(value) = value {
        rebuilt.insert(key.to_string(), value);
    }
    *out = rebuilt;
}
