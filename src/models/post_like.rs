use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::schema::post_likes;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, ToSchema)]
#[diesel(table_name = post_likes)]
#[diesel(belongs_to(super::post::Post))]
#[diesel(belongs_to(super::user::User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PostLike {
    pub id: i32,
    pub user_id: i32,
    pub post_id: i32,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize, ToSchema)]
#[diesel(table_name = post_likes)]
pub struct NewPostLike {
    pub user_id: i32,
    pub post_id: i32,
}
