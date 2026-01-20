use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{moderation_actions, user_suspensions};
use crate::DbPool;

// User suspension model
#[derive(Queryable, Selectable, Serialize)]
#[diesel(table_name = user_suspensions)]
pub struct UserSuspension {
    pub id: i64,
    pub user_id: i32,
    pub suspended_by_user_id: i32,
    pub reason: String,
    pub suspended_at: chrono::NaiveDateTime,
    pub suspended_until: chrono::NaiveDateTime,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = user_suspensions)]
pub struct NewUserSuspension {
    pub user_id: i32,
    pub suspended_by_user_id: i32,
    pub reason: String,
    pub suspended_until: chrono::NaiveDateTime,
}

// Moderation action model for audit log
#[derive(Queryable, Selectable, Serialize)]
#[diesel(table_name = moderation_actions)]
pub struct ModerationAction {
    pub id: i64,
    pub action_type: String,
    pub moderator_id: i32,
    pub target_user_id: Option<i32>,
    pub target_topic_id: Option<i32>,
    pub target_post_id: Option<i32>,
    pub details: Option<serde_json::Value>,
    pub created_at: chrono::NaiveDateTime,
}

#[derive(Insertable)]
#[diesel(table_name = moderation_actions)]
pub struct NewModerationAction {
    pub action_type: String,
    pub moderator_id: i32,
    pub target_user_id: Option<i32>,
    pub target_topic_id: Option<i32>,
    pub target_post_id: Option<i32>,
    pub details: Option<serde_json::Value>,
}

// Log a moderation action
pub fn log_moderation_action(
    pool: &DbPool,
    action: NewModerationAction,
) -> Result<ModerationAction, String> {
    use crate::schema::moderation_actions::dsl::*;

    let mut conn = pool.get().map_err(|e| e.to_string())?;

    diesel::insert_into(moderation_actions)
        .values(&action)
        .get_result(&mut conn)
        .map_err(|e| e.to_string())
}
