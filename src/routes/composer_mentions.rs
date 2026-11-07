//! GET /composer/mentions (ComposerController#mentions): for the names a
//! post mentions, which are users and which groups the member may see, why
//! a mention would notify nobody (the topic's access, a mute, a message's
//! invitees, a group that cannot be mentioned), each group's size, and how
//! many an @here would reach. The rich editor turns the mentions it does
//! not hear back about into text, and the composer warns with the reasons.

use std::collections::{HashMap, HashSet};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use crate::guardian::Guardian;
use crate::params;
use crate::session::current::{AuthGuardian, SessionUser};
use crate::site_settings::SiteSettings;
use crate::{AppError, AppState};

/// The topic's part in the checks.
struct Topic {
    id: i32,
    private_message: bool,
}

struct Group {
    id: i32,
    name: String,
    user_count: i32,
}

/// What the action's memoized helpers hold.
struct Mentions<'a> {
    conn: &'a mut PgConnection,
    settings: &'a SiteSettings,
    guardian: &'a Guardian,
    names: Vec<String>,
    topic: Option<Topic>,
    allowed_names: Vec<String>,
    topic_allowed_user_ids: Option<HashSet<i32>>,
    topic_allowed_group_ids: Option<HashSet<i32>>,
    visible_group_ids_for_allowed_check: Option<HashSet<i32>>,
}

fn quote_list(names: &[String]) -> String {
    if names.is_empty() {
        return "NULL".to_string();
    }
    names
        .iter()
        .map(|n| crate::users::search::quote(n))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Mentions<'_> {
    fn user_id(&self) -> i32 {
        self.guardian.user_id().unwrap_or(0)
    }

    async fn ids(&mut self, sql: &str) -> Result<HashSet<i32>, AppError> {
        let ids: Vec<i32> = sqlx::query_scalar(sql).fetch_all(&mut *self.conn).await?;
        Ok(ids.into_iter().collect())
    }

    /// `topic_allowed_user_ids`
    async fn topic_allowed_user_ids(&mut self) -> Result<Option<HashSet<i32>>, AppError> {
        if self.topic_allowed_user_ids.is_none() {
            self.topic_allowed_user_ids = if !self.allowed_names.is_empty() {
                let sql = format!(
                    "SELECT id FROM users WHERE username_lower IN ({})",
                    quote_list(&self.allowed_names)
                );
                Some(self.ids(&sql).await?)
            } else if let Some(topic) = self.topic.as_ref().filter(|t| t.private_message) {
                let sql = format!(
                    "SELECT user_id FROM topic_allowed_users WHERE topic_id = {}",
                    topic.id
                );
                Some(self.ids(&sql).await?)
            } else {
                None
            };
        }
        Ok(self.topic_allowed_user_ids.clone())
    }

    /// `topic_allowed_group_ids`
    async fn topic_allowed_group_ids(&mut self) -> Result<Option<HashSet<i32>>, AppError> {
        if self.topic_allowed_group_ids.is_none() {
            self.topic_allowed_group_ids = if !self.allowed_names.is_empty() {
                let sql = format!(
                    "SELECT groups.id FROM groups WHERE ({}) AND LOWER(groups.name) IN ({})",
                    crate::users::search::messageable_where(self.guardian, self.user_id()),
                    quote_list(&self.allowed_names)
                );
                Some(self.ids(&sql).await?)
            } else if let Some(topic) = self.topic.as_ref().filter(|t| t.private_message) {
                let sql = format!(
                    "SELECT group_id FROM topic_allowed_groups WHERE topic_id = {}",
                    topic.id
                );
                Some(self.ids(&sql).await?)
            } else {
                None
            };
        }
        Ok(self.topic_allowed_group_ids.clone())
    }

    /// `visible_group_ids_for_allowed_check`
    async fn visible_group_ids_for_allowed_check(&mut self) -> Result<HashSet<i32>, AppError> {
        if self.visible_group_ids_for_allowed_check.is_none() {
            let ids = if !self.allowed_names.is_empty() {
                let sql = format!(
                    "SELECT groups.id FROM groups WHERE {} AND LOWER(groups.name) IN ({})",
                    crate::groups::members_visible_groups_where(self.guardian, "groups"),
                    quote_list(&self.allowed_names)
                );
                self.ids(&sql).await?
            } else {
                self.topic_allowed_group_ids().await?.unwrap_or_default()
            };
            self.visible_group_ids_for_allowed_check = Some(ids);
        }
        Ok(self
            .visible_group_ids_for_allowed_check
            .clone()
            .unwrap_or_default())
    }

    /// `is_user_allowed?(user, topic_allowed_user_ids, topic_allowed_group_ids)`
    async fn is_user_allowed(&mut self, user_id: i32) -> Result<bool, AppError> {
        let user_ids = self.topic_allowed_user_ids().await?.unwrap_or_default();
        if user_ids.contains(&user_id) {
            return Ok(true);
        }
        let group_ids = self.topic_allowed_group_ids().await?.unwrap_or_default();
        let visible = self.visible_group_ids_for_allowed_check().await?;
        let user_groups: Vec<i32> =
            sqlx::query_scalar("SELECT group_id FROM group_users WHERE user_id = $1")
                .bind(user_id)
                .fetch_all(&mut *self.conn)
                .await?;
        Ok(user_groups
            .iter()
            .any(|id| group_ids.contains(id) && visible.contains(id)))
    }

    /// `user_reason(user)`
    async fn user_reason(
        &mut self,
        user_id: i32,
        muted_by: &HashSet<i32>,
    ) -> Result<Option<&'static str>, AppError> {
        let reason = if let Some(topic_id) = self.topic.as_ref().map(|t| t.id) {
            let private_message = self.topic.as_ref().is_some_and(|t| t.private_message);
            let can_see = match SessionUser::load(&mut *self.conn, user_id).await? {
                Some(user) => {
                    let guardian = Guardian::for_user(&mut *self.conn, &user).await?;
                    !guardian
                        .can_see_topic_ids(&mut *self.conn, self.settings, &[topic_id])
                        .await?
                        .is_empty()
                }
                None => false,
            };
            if !can_see {
                Some(if private_message {
                    "private"
                } else {
                    "category"
                })
            } else if !self.allowed_names.is_empty() && !self.is_user_allowed(user_id).await? {
                // A new message has no topic for the check above.
                Some("private")
            } else if muted_by.contains(&user_id) {
                Some("muted_topic")
            } else if private_message && !self.is_user_allowed(user_id).await? {
                // Admins can see the message, but only invitees are mentioned.
                Some("not_allowed")
            } else {
                None
            }
        } else if !self.allowed_names.is_empty() && !self.is_user_allowed(user_id).await? {
            Some("private")
        } else {
            None
        };
        // Others than staff learn only that the topic is out of reach.
        Ok(reason.filter(|r| self.guardian.is_staff() || *r == "private" || *r == "category"))
    }

    /// `group_reason(group)`
    async fn group_reason(
        &mut self,
        group_id: i32,
        mentionable: &HashSet<i32>,
    ) -> Result<Option<&'static str>, AppError> {
        if !mentionable.contains(&group_id) {
            return Ok(Some("not_mentionable"));
        }
        let pm = self.topic.as_ref().is_some_and(|t| t.private_message);
        if (pm || !self.allowed_names.is_empty())
            && !self
                .topic_allowed_group_ids()
                .await?
                .unwrap_or_default()
                .contains(&group_id)
        {
            return Ok(Some("not_allowed"));
        }
        Ok(None)
    }

    /// `already_notified_member_count(group)`
    async fn already_notified_member_count(&mut self, group_id: i32) -> Result<i64, AppError> {
        let users: Vec<i32> = self
            .topic_allowed_user_ids()
            .await?
            .unwrap_or_default()
            .into_iter()
            .collect();
        let groups: Vec<i32> = self
            .topic_allowed_group_ids()
            .await?
            .unwrap_or_default()
            .into_iter()
            .collect();
        Ok(sqlx::query_scalar(
            "SELECT COUNT(DISTINCT user_id) FROM group_users WHERE group_id = $1 \
             AND (user_id = ANY($2) OR user_id IN (SELECT user_id FROM group_users WHERE group_id = ANY($3)))",
        )
        .bind(group_id)
        .bind(&users)
        .bind(&groups)
        .fetch_one(&mut *self.conn)
        .await?)
    }

    /// `guardian.can_mention_here?`
    async fn can_mention_here(&mut self) -> Result<bool, AppError> {
        let here = self.settings.get("here_mention")?.to_s();
        if here.trim().is_empty()
            || self.settings.get("max_here_mentioned")?.to_i() < 1
            || self.guardian.user_id().is_none()
        {
            return Ok(false);
        }
        let taken: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE username_lower = $1)")
                .bind(&here)
                .fetch_one(&mut *self.conn)
                .await?;
        if taken {
            return Ok(false);
        }
        Ok(self
            .guardian
            .in_setting_groups(self.settings, "here_mention_allowed_groups")?)
    }

    /// `PostAlerter.new.expand_here_mention(topic.first_post, exclude_ids:
    /// [current_user.id]).size`
    async fn here_count(&mut self, topic_id: i32) -> Result<i64, AppError> {
        let first: Option<(Option<i32>,)> = sqlx::query_as(
            "SELECT user_id FROM posts WHERE topic_id = $1 AND post_number = 1 AND deleted_at IS NULL",
        )
        .bind(topic_id)
        .fetch_optional(&mut *self.conn)
        .await?;
        let author = first.and_then(|(id,)| id);
        let staff: bool = sqlx::query_scalar(
            "SELECT COALESCE((SELECT admin OR moderator FROM users WHERE id = $1), FALSE)",
        )
        .bind(author)
        .fetch_one(&mut *self.conn)
        .await?;
        let mut types = vec![
            crate::posting::post_types::REGULAR,
            crate::posting::post_types::MODERATOR_ACTION,
        ];
        if staff {
            types.push(crate::posting::post_types::WHISPER);
        }
        let ids: Vec<i32> = sqlx::query_scalar(
            "SELECT id FROM users WHERE id > 0 \
               AND NOT EXISTS (SELECT 1 FROM anonymous_users a WHERE a.user_id = users.id) \
               AND id IN (SELECT user_id FROM posts WHERE topic_id = $1 AND post_type = ANY($2) \
                          AND deleted_at IS NULL AND user_id <> $3) \
             LIMIT $4",
        )
        .bind(topic_id)
        .bind(&types)
        .bind(self.user_id())
        .bind(self.settings.get("max_here_mentioned")?.to_i())
        .fetch_all(&mut *self.conn)
        .await?;
        Ok(ids.len() as i64)
    }
}

fn strings(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Array(items) => Some(
            items
                .iter()
                .map(|v| params::scalar(v).unwrap_or_default())
                .collect(),
        ),
        Value::Object(map) => Some(
            map.values()
                .map(|v| params::scalar(v).unwrap_or_default())
                .collect(),
        ),
        _ => None,
    }
}

pub async fn mentions(
    State(state): State<AppState>,
    AuthGuardian(guardian): AuthGuardian,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, AppError> {
    // requires_login
    let Some(current_user) = guardian.user().cloned() else {
        return Ok(super::login_required::not_logged_in(&state));
    };
    let p = params::parse(uri.query(), &headers, &body);
    let names = match p.get("names") {
        None | Some(Value::Null) => return Ok(super::accounts::param_missing("names")),
        Some(Value::String(s)) if s.is_empty() => {
            return Ok(super::accounts::param_missing("names"));
        }
        Some(value) => match value {
            Value::Array(_) => strings(value).unwrap_or_default(),
            _ => return Ok(super::search::invalid_parameters(&state, "names")),
        },
    };
    if names.len() > 20 {
        return Ok(super::search::invalid_parameters(&state, "names"));
    }
    let names: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();

    let mut conn = state.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &state.site_setting_defs, &state.config.globals).await?;

    let topic_param = p
        .get("topic_id")
        .and_then(params::scalar)
        .filter(|s| !s.trim().is_empty());
    let topic = match topic_param {
        Some(raw) => {
            let id = crate::ruby::to_i(&raw) as i32;
            let found: Option<(i32, Option<String>)> =
                sqlx::query_as("SELECT id, archetype FROM topics WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *conn)
                    .await?;
            // ensure_can_see!(nil) as well as an unseen topic
            let Some((id, archetype)) = found else {
                return Ok(super::search::invalid_access(&state));
            };
            if guardian
                .can_see_topic_ids(&mut conn, &settings, &[id])
                .await?
                .is_empty()
            {
                return Ok(super::search::invalid_access(&state));
            }
            Some(Topic {
                id,
                private_message: archetype.as_deref() == Some("private_message"),
            })
        }
        None => None,
    };

    // allowed_names: only for a new message
    let allowed_names = match p.get("allowed_names") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) if s.is_empty() => Vec::new(),
        Some(value @ Value::Array(_)) => {
            let mut list = strings(value).unwrap_or_default();
            if list.is_empty() {
                Vec::new()
            } else {
                list.push(current_user.username.clone());
                list.iter().map(|n| n.to_lowercase()).collect()
            }
        }
        Some(_) => return Ok(super::search::invalid_parameters(&state, "allowed_names")),
    };

    let name_list = quote_list(&names);
    let users: Vec<(i32, String)> = sqlx::query_as(&format!(
        "SELECT id, username_lower FROM users WHERE NOT staged AND username_lower IN ({name_list}) ORDER BY ctid"
    ))
    .fetch_all(&mut *conn)
    .await?;
    let groups: Vec<Group> = sqlx::query_as::<_, (i32, String, i32)>(&format!(
        "SELECT groups.id, groups.name, groups.user_count FROM groups WHERE {} \
         AND LOWER(groups.name) IN ({name_list}) ORDER BY groups.name ASC",
        crate::users::search::visible_groups_where(&guardian)
    ))
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|(id, name, user_count)| Group {
        id,
        name,
        user_count,
    })
    .collect();
    let user_id = current_user.id;
    let mentionable: HashSet<i32> = sqlx::query_scalar::<_, i32>(&format!(
        "SELECT groups.id FROM groups WHERE ({}) AND LOWER(groups.name) IN ({name_list})",
        crate::users::search::mentionable_where_with(&guardian, user_id, false)
    ))
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let members_visible: HashSet<i32> = sqlx::query_scalar::<_, i32>(&format!(
        "SELECT groups.id FROM groups WHERE {} AND LOWER(groups.name) IN ({name_list})",
        crate::groups::members_visible_groups_where(&guardian, "groups")
    ))
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .collect();
    let muted_by: HashSet<i32> = match &topic {
        Some(topic) => {
            let ids: Vec<i32> = users.iter().map(|(id, _)| *id).collect();
            sqlx::query_scalar::<_, i32>(
                "SELECT user_id FROM topic_users WHERE topic_id = $1 AND user_id = ANY($2) \
                 AND notification_level = 0",
            )
            .bind(topic.id)
            .bind(&ids)
            .fetch_all(&mut *conn)
            .await?
            .into_iter()
            .collect()
        }
        None => HashSet::new(),
    };

    let mut m = Mentions {
        conn: &mut conn,
        settings: &settings,
        guardian: &guardian,
        names: names.clone(),
        topic,
        allowed_names,
        topic_allowed_user_ids: None,
        topic_allowed_group_ids: None,
        visible_group_ids_for_allowed_check: None,
    };

    let user_by_name: HashMap<&str, i32> = users.iter().map(|(id, n)| (n.as_str(), *id)).collect();
    let group_by_name: HashMap<String, &Group> =
        groups.iter().map(|g| (g.name.to_lowercase(), g)).collect();
    let mut user_reasons = Map::new();
    let mut group_reasons: Vec<(String, &'static str)> = Vec::new();
    for name in m.names.clone() {
        if let Some(id) = user_by_name.get(name.as_str()) {
            if let Some(reason) = m.user_reason(*id, &muted_by).await? {
                user_reasons.insert(name.clone(), json!(reason));
            }
        } else if let Some(group) = group_by_name.get(&name)
            && let Some(reason) = m.group_reason(group.id, &mentionable).await?
        {
            match group_reasons.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = reason,
                None => group_reasons.push((name.clone(), reason)),
            }
        }
    }

    let here = settings.get("here_mention")?.to_s().to_lowercase();
    let here_count = match m.topic.as_ref().map(|t| t.id) {
        Some(topic_id) if m.names.contains(&here) && m.can_mention_here().await? => {
            Some(m.here_count(topic_id).await?)
        }
        _ => None,
    };

    let pm = m.topic.as_ref().is_some_and(|t| t.private_message);
    let mut serialized_groups = Map::new();
    for group in &groups {
        let name = group.name.to_lowercase();
        let visible = members_visible.contains(&group.id);
        let mut serialized = Map::new();
        if visible {
            serialized.insert("user_count".into(), json!(group.user_count));
        }
        let reason = group_reasons
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| *r);
        if reason == Some("not_allowed") && visible && (pm || !m.allowed_names.is_empty()) {
            let notified = m.already_notified_member_count(group.id).await?;
            if notified > 0 {
                if notified == i64::from(group.user_count) {
                    group_reasons.retain(|(n, _)| *n != name);
                } else {
                    if let Some(slot) = group_reasons.iter_mut().find(|(n, _)| *n == name) {
                        slot.1 = "some_not_allowed";
                    }
                    serialized.insert("notified_count".into(), json!(notified));
                }
            }
        }
        serialized_groups.insert(name, Value::Object(serialized));
    }

    let group_reasons: Map<String, Value> = group_reasons
        .into_iter()
        .map(|(n, r)| (n, json!(r)))
        .collect();
    Ok(Json(json!({
        "users": users.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>(),
        "user_reasons": user_reasons,
        "groups": serialized_groups,
        "group_reasons": group_reasons,
        "here_count": here_count,
        "max_users_notified_per_group_mention": settings.get("max_users_notified_per_group_mention")?.to_i(),
    }))
    .into_response())
}
