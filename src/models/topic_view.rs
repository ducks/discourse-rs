use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::schema::topic_views;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, ToSchema)]
#[diesel(table_name = topic_views)]
#[diesel(belongs_to(super::user::User))]
#[diesel(belongs_to(super::topic::Topic))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TopicView {
    pub id: i32,
    pub user_id: i32,
    pub topic_id: i32,
    pub first_viewed_at: NaiveDateTime,
    pub last_viewed_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize, ToSchema)]
#[diesel(table_name = topic_views)]
pub struct NewTopicView {
    pub user_id: i32,
    pub topic_id: i32,
}
