use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::{moderation_actions, user_suspensions};
use crate::DbPool;

// Trust levels
pub const TRUST_LEVEL_NEW_USER: i32 = 0;
pub const TRUST_LEVEL_BASIC: i32 = 1;
pub const TRUST_LEVEL_MEMBER: i32 = 2;
pub const TRUST_LEVEL_REGULAR: i32 = 3;
pub const TRUST_LEVEL_LEADER: i32 = 4;

// Check if user is moderator (trust level 4)
pub fn is_moderator(trust_level: i32) -> bool {
    trust_level >= TRUST_LEVEL_LEADER
}

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

// Check if user is currently suspended
pub fn is_user_suspended(pool: &DbPool, user_id: i32) -> Result<bool, String> {
    use crate::schema::user_suspensions::dsl::*;

    let mut conn = pool.get().map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().naive_utc();

    let suspended = user_suspensions
        .filter(user_id.eq(user_id))
        .filter(suspended_until.gt(now))
        .select(diesel::dsl::count_star())
        .first::<i64>(&mut conn)
        .map_err(|e| e.to_string())?;

    Ok(suspended > 0)
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
