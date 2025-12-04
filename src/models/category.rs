use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::categories;

#[derive(Debug, Clone, Queryable, Selectable, Identifiable, Serialize)]
#[diesel(table_name = categories)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Category {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: String,
    pub position: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable, Deserialize)]
#[diesel(table_name = categories)]
pub struct NewCategory {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub position: i32,
}

#[derive(Debug, AsChangeset, Deserialize)]
#[diesel(table_name = categories)]
pub struct UpdateCategory {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub position: Option<i32>,
}

fn default_color() -> String {
    "0088CC".to_string()
}
