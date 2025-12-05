use diesel::prelude::*;

use crate::models::SiteSetting;
use crate::schema::site_settings;
use crate::DbPool;

pub fn require_auth_for_reads(pool: &DbPool) -> bool {
    let mut conn = match pool.get() {
        Ok(conn) => conn,
        Err(_) => return false, // Default to false if can't connect
    };

    match site_settings::table
        .find("require_auth_for_reads")
        .select(SiteSetting::as_select())
        .first(&mut conn)
    {
        Ok(setting) => setting.value == "true",
        Err(_) => false, // Default to false if setting not found
    }
}

pub fn get_setting(pool: &DbPool, key: &str) -> Option<String> {
    let mut conn = pool.get().ok()?;

    site_settings::table
        .find(key)
        .select(SiteSetting::as_select())
        .first(&mut conn)
        .ok()
        .map(|setting: SiteSetting| setting.value)
}
