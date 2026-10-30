//! Creating a category channel (`Chat::CreateCategoryChannel` behind POST
//! /chat/api/channels) and the staff hint the create modal reads (GET
//! /chat/api/category-chatables/:id/permissions).

use serde_json::{Map, Value, json};

use super::channels::{CHANNEL_COLUMNS, ChannelRow, Context};
use super::dm_groups::Outcome;
use crate::{AppError, AppState, Unsupported};

/// What the controller permits of `params[:channel]`.
#[derive(Default)]
pub struct Params {
    pub chatable_id: Option<String>,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub auto_join_users: Option<String>,
    pub threading_enabled: Option<String>,
    pub emoji: Option<String>,
}

impl Params {
    pub fn from_map(map: &Map<String, Value>) -> Params {
        let get = |k: &str| map.get(k).and_then(crate::params::scalar);
        Params {
            chatable_id: get("chatable_id"),
            name: get("name"),
            slug: get("slug"),
            description: get("description"),
            auto_join_users: get("auto_join_users"),
            threading_enabled: get("threading_enabled"),
            emoji: get("emoji"),
        }
    }
}

/// How a create ends besides the dm_groups outcomes: the public channels
/// policy (render_json_error, a 422) and an existing channel
/// (Discourse::InvalidParameters).
pub enum Created {
    Outcome(Outcome),
    PublicChannelsDisabled,
    ChannelExists,
}

impl Context<'_> {
    /// `Chat::CreateCategoryChannel`: staff, public channels on; the
    /// contract (a category, a name within max_topic_title_length); the
    /// category the member may post in, without a live channel of that
    /// name; the channel (one member, its auto slug and validations) and
    /// the creator's following membership; auto join when asked for.
    pub async fn create_category_channel(
        &mut self,
        state: &AppState,
        params: &Params,
    ) -> Result<Created, AppError> {
        if !self.guardian.is_staff() {
            return Ok(Created::Outcome(Outcome::Forbidden));
        }
        if !self.settings.get("enable_public_channels")?.truthy() {
            return Ok(Created::PublicChannelsDisabled);
        }
        let cast_bool =
            |v: &Option<String>| crate::ruby::cast_boolean(v.as_deref()).unwrap_or(false);
        let category_id = params
            .chatable_id
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .map(crate::ruby::to_i);
        let max_title = self.settings.get("max_topic_title_length")?.to_i();
        let mut errors = Vec::new();
        if category_id.is_none() {
            errors.push("Category can't be blank".to_string());
        }
        if params
            .name
            .as_deref()
            .is_some_and(|n| n.chars().count() as i64 > max_title)
        {
            errors.push(format!(
                "Name is too long (maximum is {max_title} characters)"
            ));
        }
        if !errors.is_empty() {
            return Ok(Created::Outcome(Outcome::Invalid(errors)));
        }
        let category_id = category_id.unwrap_or(0);
        let Some(category) = self.category(category_id).await? else {
            return Ok(Created::Outcome(Outcome::NotFound));
        };
        if self
            .post_allowed_category_ids(&[category_id])
            .await?
            .is_empty()
            && !self.guardian.is_admin()
        {
            return Ok(Created::Outcome(Outcome::Forbidden));
        }
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM chat_channels WHERE chatable_type = 'Category' \
               AND chatable_id = $1 AND name IS NOT DISTINCT FROM $2 AND deleted_at IS NULL)",
        )
        .bind(category_id)
        .bind(&params.name)
        .fetch_one(&mut *self.conn)
        .await?;
        if exists {
            return Ok(Created::ChannelExists);
        }
        // The record: generate_auto_slug, then its validations.
        let name = params.name.clone();
        let description = params.description.clone();
        let emoji = params.emoji.clone().filter(|e| !e.trim().is_empty());
        let mut slug = params.slug.clone();
        if slug.as_deref().is_none_or(|s| s.trim().is_empty()) {
            let title = name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| category.name.clone());
            let auto = super::channel_settings::auto_slug(self.settings, title.trim())?;
            slug = Some(
                if !auto.is_empty()
                    && super::channel_settings::duplicate_slug(&mut *self.conn, &auto, 0).await?
                {
                    String::new()
                } else {
                    auto
                },
            );
        }
        let mut errors = Vec::new();
        if name
            .as_deref()
            .is_some_and(|n| n.chars().count() as i64 > max_title)
        {
            errors.push(format!(
                "Name is too long (maximum is {max_title} characters)"
            ));
        }
        if name.as_deref().is_some_and(|n| n.trim().is_empty()) {
            errors.push("Name can't be blank".into());
        }
        if description
            .as_deref()
            .is_some_and(|d| d.chars().count() > 500)
        {
            errors.push("Description is too long (maximum is 500 characters)".into());
        }
        if slug.as_deref().is_some_and(|s| s.chars().count() > 100) {
            errors.push("Slug is too long (maximum is 100 characters)".into());
        }
        if emoji.as_deref().is_some_and(|e| e.chars().count() > 100) {
            errors.push("Emoji is too long (maximum is 100 characters)".into());
        }
        if let Some(given) = slug.clone().filter(|s| !s.trim().is_empty()) {
            match super::channel_settings::checked_slug(
                &mut *self.conn,
                self.settings,
                self.i18n,
                &given,
                0,
            )
            .await?
            {
                Ok(normalized) => slug = Some(normalized),
                Err(error) => errors.push(error),
            }
        }
        if !errors.is_empty() {
            return Ok(Created::Outcome(Outcome::RecordInvalid(errors)));
        }
        let now = crate::clock::now_naive();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO chat_channels (chatable_id, chatable_type, type, name, slug, description, \
               auto_join_users, threading_enabled, emoji, user_count, created_at, updated_at) \
             VALUES ($1, 'Category', 'CategoryChannel', $2, $3, $4, $5, $6, $7, 1, $8, $8) RETURNING id",
        )
        .bind(category_id)
        .bind(&name)
        .bind(&slug)
        .bind(&description)
        .bind(cast_bool(&params.auto_join_users))
        .bind(cast_bool(&params.threading_enabled))
        .bind(&emoji)
        .bind(now)
        .fetch_one(&mut *self.conn)
        .await?;
        sqlx::query(
            "INSERT INTO user_chat_channel_memberships (user_id, chat_channel_id, following, created_at, updated_at) \
             VALUES ($1, $2, TRUE, $3, $3)",
        )
        .bind(self.guardian.user_id())
        .bind(id)
        .bind(now)
        .execute(&mut *self.conn)
        .await?;
        let channel: ChannelRow = sqlx::query_as(&format!(
            "SELECT {CHANNEL_COLUMNS} FROM chat_channels WHERE id = $1"
        ))
        .bind(id)
        .fetch_one(&mut *self.conn)
        .await?;
        if channel.auto_join_users {
            super::auto_join::create_memberships(
                &mut *self.conn,
                &state.bus,
                &state.i18n,
                &state.config,
                self.settings,
                None,
                Some(channel.id),
            )
            .await?;
        }
        let membership = self
            .own_membership(channel.id)
            .await?
            .ok_or(Unsupported("a membership gone while creating its channel"))?;
        let body = self
            .channel(&channel, Some(&membership), None, None)
            .await?;
        Ok(Created::Outcome(Outcome::Done(json!({ "channel": body }))))
    }
}

impl Context<'_> {
    /// CategoryChatablesController#permissions: a restricted category's
    /// groups that may post (full or create_post, visible to the viewer,
    /// by name) with their member count, else the everyone group. None
    /// for a missing category, Err for one the viewer can't see.
    pub async fn category_permissions(
        &mut self,
        id: i64,
    ) -> Result<Option<Result<Value, ()>>, AppError> {
        let Some(category) = self.category(id).await? else {
            return Ok(None);
        };
        if !self.can_see_category(id).await? {
            return Ok(Some(Err(())));
        }
        if !category.read_restricted {
            let everyone: String = sqlx::query_scalar("SELECT name FROM groups WHERE id = 0")
                .fetch_one(&mut *self.conn)
                .await?;
            return Ok(Some(Ok(
                json!({ "allowed_groups": [format!("@{everyone}")], "private": false }),
            )));
        }
        let visible = crate::groups::visible_groups_scope(self.guardian, "groups", true, false);
        let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
            "SELECT groups.name, COUNT(group_users.user_id) FROM groups \
             JOIN category_groups ON category_groups.group_id = groups.id \
             LEFT OUTER JOIN group_users ON groups.id = group_users.group_id \
             WHERE {visible} AND category_groups.category_id = $1 \
               AND category_groups.permission_type IN (1, 2) \
             GROUP BY groups.id, groups.name ORDER BY groups.name ASC"
        ))
        .bind(super::channels::category_id(id))
        .fetch_all(&mut *self.conn)
        .await?;
        let names: Vec<String> = rows.iter().map(|(n, _)| format!("@{n}")).collect();
        let members: i64 = rows.iter().map(|(_, c)| c).sum();
        Ok(Some(Ok(json!({
            "allowed_groups": names,
            "members_count": members,
            "private": true,
        }))))
    }
}
