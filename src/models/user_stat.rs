use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::schema::user_stats;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, ToSchema)]
#[diesel(table_name = user_stats)]
#[diesel(primary_key(user_id))]
#[diesel(belongs_to(super::user::User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserStat {
    pub user_id: i32,
    pub post_count: i32,
    pub topic_count: i32,
    pub time_read: i32,
    pub posts_read_count: i32,
    pub topics_entered: i32,
    pub days_visited: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize, ToSchema)]
#[diesel(table_name = user_stats)]
pub struct NewUserStat {
    pub user_id: i32,
}
