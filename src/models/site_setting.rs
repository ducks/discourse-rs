use diesel::prelude::*;
use serde::{Deserialize, Serialize};

use crate::schema::site_settings;

#[derive(Queryable, Selectable, Serialize, Deserialize, Debug, Clone)]
#[diesel(table_name = site_settings)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SiteSetting {
    pub key: String,
    pub value: String,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

#[derive(Insertable, AsChangeset, Serialize, Deserialize)]
#[diesel(table_name = site_settings)]
pub struct UpdateSiteSetting {
    pub value: String,
}
