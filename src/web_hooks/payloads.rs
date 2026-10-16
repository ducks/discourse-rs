//! The payloads of the user, group membership, badge and tag events:
//! WebHookUserSerializer (and UserSerializer for user_promoted),
//! WebHookGroupUserSerializer, UserBadgeSerializer and TagSerializer, each
//! seen by the system user.

use serde_json::{Map, Value, json};
use sqlx::PgConnection;

use super::{Payloads, system_guardian};
use crate::topic_list::time_json;
use crate::{AppError, Unsupported};

/// UserSerializer's keys WebHookUserSerializer leaves out.
const USER_LEFT_OUT: &[&str] = &[
    "unconfirmed_emails",
    "can_edit",
    "can_edit_username",
    "can_edit_email",
    "can_edit_name",
    "can_send_private_messages",
    "can_send_private_message_to_user",
    "can_ignore_user",
    "can_mute_user",
    "ignored",
    "uploaded_avatar_id",
    "has_title_badges",
    "bio_cooked",
    "custom_fields",
    "can_be_deleted",
    "can_delete_all_posts",
    "system_avatar_upload_id",
    "gravatar_avatar_upload_id",
    "custom_avatar_upload_id",
    "can_change_bio",
    "can_change_location",
    "can_change_website",
    "can_change_tracking_preferences",
    "user_api_keys",
    "user_passkeys",
    "group_users",
    "user_auth_tokens",
    "user_auth_token_logs",
    "use_logo_small_as_avatar",
    "pending_posts_count",
    "status",
    "display_sidebar_tags",
    "sidebar_category_ids",
    "sidebar_tags",
    "can_pick_theme_with_custom_homepage",
];

impl Payloads<'_> {
    /// UserSerializer (root: false) for a user, seen by the system user.
    pub async fn user_serializer(
        &self,
        conn: &mut PgConnection,
        user_id: i32,
    ) -> Result<Map<String, Value>, AppError> {
        let user = crate::users::User::find_by_id(&mut *conn, user_id)
            .await?
            .ok_or(Unsupported("a web hook payload for a missing user"))?;
        let guardian = system_guardian(&mut *conn).await?;
        let urls = crate::url::Urls {
            config: self.config,
            settings: self.settings,
        };
        let doc = crate::users::Users {
            conn: &mut *conn,
            settings: self.settings,
            i18n: self.i18n,
            guardian: &guardian,
            urls: &urls,
            base_path: self.config.globals.relative_url_root(),
            auth_token: None,
        }
        .show(&user)
        .await?;
        match doc.get("user") {
            Some(Value::Object(u)) => Ok(u.clone()),
            _ => Err(Unsupported("a web hook payload for a hidden user").into()),
        }
    }

    /// WebHookUserSerializer: UserSerializer less what it leaves out, the
    /// emails for its admin scope, the SSO external id.
    pub async fn user(&self, conn: &mut PgConnection, user_id: i32) -> Result<Value, AppError> {
        let full = self.user_serializer(&mut *conn, user_id).await?;
        // include_external_id?: the system user's admin scope and a record.
        let external_id: Option<String> =
            sqlx::query_scalar("SELECT external_id FROM single_sign_on_records WHERE user_id = $1")
                .bind(user_id)
                .fetch_optional(&mut *conn)
                .await?;
        let (email, secondary): (Option<String>, Vec<String>) = sqlx::query_as(
            "SELECT (SELECT email FROM user_emails WHERE user_id = $1 AND \"primary\" LIMIT 1), \
                    ARRAY(SELECT email FROM user_emails WHERE user_id = $1 AND NOT \"primary\" ORDER BY ctid)",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await?;
        let mut out = Map::new();
        // The subclass's own attribute comes after UserSerializer's, before
        // its associations.
        let mut external_id = external_id;
        for (key, value) in full {
            if USER_LEFT_OUT.contains(&key.as_str()) || key == "email" || key == "secondary_emails"
            {
                continue;
            }
            if matches!(
                key.as_str(),
                "invited_by" | "groups" | "user_option" | "featured_user_badge_ids"
            ) && let Some(id) = external_id.take()
            {
                out.insert("external_id".into(), json!(id));
            }
            let after_avatar = key == "avatar_template";
            out.insert(key, value);
            if after_avatar {
                out.insert("email".into(), json!(email));
                out.insert("secondary_emails".into(), json!(secondary));
            }
        }
        if let Some(id) = external_id {
            out.insert("external_id".into(), json!(id));
        }
        Ok(Value::Object(out))
    }

    /// WebHookGroupSerializer: BasicGroupSerializer as the system user,
    /// without is_group_user and is_group_owner.
    pub async fn group(&self, conn: &mut PgConnection, group_id: i32) -> Result<Value, AppError> {
        let guardian = system_guardian(&mut *conn).await?;
        let group = crate::groups::find_basic(&mut *conn, None, Some(group_id))
            .await?
            .ok_or(Unsupported("a group web hook without its group"))?;
        Ok(group.json(self.i18n, &guardian, self.settings, None)?)
    }

    /// WebHookGroupUserSerializer: BasicGroupUserSerializer's group, user
    /// and level, then the row's id and creation.
    pub async fn group_user(
        &self,
        conn: &mut PgConnection,
        group_user_id: i64,
    ) -> Result<Value, AppError> {
        let row: Option<(i32, i32, Option<i32>, chrono::NaiveDateTime)> = sqlx::query_as(
            "SELECT group_id, user_id, notification_level, created_at FROM group_users WHERE id = $1",
        )
        .bind(group_user_id)
        .fetch_optional(&mut *conn)
        .await?;
        let Some((group_id, user_id, level, created_at)) = row else {
            return Ok(Value::Null);
        };
        Ok(json!({
            "group_id": group_id,
            "user_id": user_id,
            "notification_level": level,
            "id": group_user_id,
            "created_at": time_json(created_at),
        }))
    }

    /// UserBadgeSerializer, root: false: the badge, user, granter and topic
    /// associations embedded by id.
    pub async fn user_badge(&self, conn: &mut PgConnection, id: i64) -> Result<Value, AppError> {
        user_badge_json(conn, id).await
    }

    /// TagSerializer, as the system user sees it.
    pub async fn tag(&self, conn: &mut PgConnection, tag_id: i32) -> Result<Value, AppError> {
        let tag = crate::tags::Tag::find(&mut *conn, tag_id)
            .await?
            .ok_or(Unsupported("a web hook payload for a missing tag"))?;
        let guardian = system_guardian(&mut *conn).await?;
        Ok(tag.serialize(&mut *conn, &guardian, self.settings).await?)
    }
}

/// UserBadgeSerializer for a row (gone rows can't be serialized: the
/// revoke builds it first).
pub(crate) async fn user_badge_json(conn: &mut PgConnection, id: i64) -> Result<Value, AppError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i32,
        granted_at: chrono::NaiveDateTime,
        created_at: chrono::NaiveDateTime,
        post_id: Option<i32>,
        badge_id: i32,
        user_id: i32,
        granted_by_id: i32,
        show_posts: bool,
        post_number: Option<i32>,
        topic_id: Option<i32>,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT ub.id, ub.granted_at, ub.created_at, ub.post_id, ub.badge_id, ub.user_id, \
                ub.granted_by_id, b.show_posts, p.post_number, p.topic_id \
         FROM user_badges ub JOIN badges b ON b.id = ub.badge_id \
         LEFT JOIN posts p ON p.id = ub.post_id AND p.deleted_at IS NULL \
         WHERE ub.id = $1",
    )
    .bind(i32::try_from(id).unwrap_or(0))
    .fetch_optional(&mut *conn)
    .await?;
    let Some(r) = row else {
        return Ok(Value::Null);
    };
    let mut out = Map::new();
    out.insert("id".into(), json!(r.id));
    out.insert("granted_at".into(), json!(time_json(r.granted_at)));
    out.insert("created_at".into(), json!(time_json(r.created_at)));
    // UserBadgePostAndTopicAttributesMixin: the post's attributes while
    // the badge shows posts and the post is there.
    let post_attributes = r.show_posts && r.post_number.is_some();
    if post_attributes {
        out.insert("post_id".into(), json!(r.post_id));
        out.insert("post_number".into(), json!(r.post_number));
    }
    out.insert("badge_id".into(), json!(r.badge_id));
    out.insert("user_id".into(), json!(r.user_id));
    out.insert("granted_by_id".into(), json!(r.granted_by_id));
    if post_attributes {
        out.insert("topic_id".into(), json!(r.topic_id));
    }
    Ok(Value::Object(out))
}
