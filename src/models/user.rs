use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::users;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize)]
#[diesel(table_name = users)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: i32,
    pub username: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub admin: bool,
    pub moderator: bool,
    pub trust_level: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub username: String,
    pub email: String,
    pub password_hash: String,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub moderator: bool,
    #[serde(default)]
    pub trust_level: i32,
}

#[derive(Debug, AsChangeset, Deserialize)]
#[diesel(table_name = users)]
pub struct UpdateUser {
    pub username: Option<String>,
    pub email: Option<String>,
    pub admin: Option<bool>,
    pub moderator: Option<bool>,
    pub trust_level: Option<i32>,
}

impl User {
    pub fn is_staff(&self) -> bool {
        self.admin || self.moderator
    }
}
