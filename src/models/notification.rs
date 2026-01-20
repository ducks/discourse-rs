use chrono::{DateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::schema::notifications;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize, ToSchema)]
#[diesel(table_name = notifications)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Notification {
    pub id: i64,
    pub user_id: i32,
    pub notification_type: String,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
    pub read: bool,
    pub topic_id: Option<i32>,
    pub post_id: Option<i32>,
    pub acting_user_id: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Insertable, Deserialize, ToSchema)]
#[diesel(table_name = notifications)]
pub struct NewNotification {
    pub user_id: i32,
    pub notification_type: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
    pub topic_id: Option<i32>,
    pub post_id: Option<i32>,
    pub acting_user_id: Option<i32>,
}

impl Notification {
    pub fn is_read(&self) -> bool {
        self.read
    }
}
