use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::topics;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize)]
#[diesel(table_name = topics)]
#[diesel(belongs_to(super::user::User))]
#[diesel(belongs_to(super::category::Category))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Topic {
    pub id: i32,
    pub title: String,
    pub slug: String,
    pub user_id: i32,
    pub category_id: Option<i32>,
    pub views: i32,
    pub posts_count: i32,
    pub pinned: bool,
    pub closed: bool,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = topics)]
pub struct NewTopic {
    pub title: String,
    pub slug: String,
    pub user_id: i32,
    pub category_id: Option<i32>,
    #[serde(default)]
    pub views: i32,
    #[serde(default)]
    pub posts_count: i32,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub closed: bool,
}

#[derive(Debug, AsChangeset, Deserialize)]
#[diesel(table_name = topics)]
pub struct UpdateTopic {
    pub title: Option<String>,
    pub slug: Option<String>,
    pub category_id: Option<i32>,
    pub pinned: Option<bool>,
    pub closed: Option<bool>,
}

impl Topic {
    pub fn increment_views(&mut self) {
        self.views += 1;
    }

    pub fn increment_posts_count(&mut self) {
        self.posts_count += 1;
    }
}
