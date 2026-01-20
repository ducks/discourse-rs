use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::schema::posts;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Associations, Serialize, ToSchema)]
#[diesel(table_name = posts)]
#[diesel(belongs_to(super::topic::Topic))]
#[diesel(belongs_to(super::user::User))]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Post {
    pub id: i32,
    pub topic_id: i32,
    pub user_id: i32,
    pub post_number: i32,
    pub raw: String,
    pub cooked: String,
    pub reply_to_post_number: Option<i32>,
    pub deleted_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize, ToSchema)]
#[diesel(table_name = posts)]
pub struct NewPost {
    pub topic_id: i32,
    pub user_id: i32,
    pub post_number: i32,
    pub raw: String,
    pub cooked: String,
    pub reply_to_post_number: Option<i32>,
}

#[derive(Debug, AsChangeset, Deserialize, ToSchema)]
#[diesel(table_name = posts)]
pub struct UpdatePost {
    pub raw: Option<String>,
    pub cooked: Option<String>,
}

/// API input for creating a post (client only provides raw markdown)
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePostInput {
    pub topic_id: i32,
    pub user_id: i32,
    pub post_number: i32,
    pub raw: String,
    pub reply_to_post_number: Option<i32>,
}

impl CreatePostInput {
    pub fn into_new_post(self) -> NewPost {
        let cooked = crate::markdown::render(&self.raw);
        NewPost {
            topic_id: self.topic_id,
            user_id: self.user_id,
            post_number: self.post_number,
            raw: self.raw,
            cooked,
            reply_to_post_number: self.reply_to_post_number,
        }
    }
}

/// API input for updating a post (client only provides raw markdown)
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePostInput {
    pub raw: Option<String>,
}

impl UpdatePostInput {
    pub fn into_update_post(self) -> UpdatePost {
        let cooked = self.raw.as_ref().map(|r| crate::markdown::render(r));
        UpdatePost {
            raw: self.raw,
            cooked,
        }
    }
}

impl Post {
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn is_first_post(&self) -> bool {
        self.post_number == 1
    }
}
