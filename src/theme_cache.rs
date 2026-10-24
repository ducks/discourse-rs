//! What a page draws from the theme tables (the svg sprite, the theme's
//! html fields, its stylesheet links and its color scheme tags), kept in
//! process per site as Rails keeps them in its DistributedCaches
//! (SvgSprite, Theme.lookup_field, Stylesheet::Manager), and built again
//! only when what they are drawn from has changed.
//!
//! Rails expires those caches when a theme, theme field, setting, palette,
//! modifier set or sprite is saved, and when a site setting changes. Here
//! those tables give one fingerprint, read in a single query a page. A
//! write from any path and in any process is seen by the next request, as
//! the site settings' own cache sees its table change (SiteSettings::load).
//! Entries are keyed by the database, so each site of a multisite process
//! keeps its own.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

use sqlx::PgConnection;

/// The database, and each table's latest update and row count
/// (theme_modifier_sets, which has no timestamps, and color_schemes,
/// which update_all writes without them, by their rows).
const QUERY: &str = "SELECT current_database()::text, concat_ws('/', \
     (SELECT max(updated_at)::text || count(*) FROM site_settings), \
     (SELECT max(updated_at)::text || count(*) FROM badges), \
     (SELECT max(updated_at)::text || count(*) FROM groups), \
     (SELECT max(updated_at)::text || count(*) FROM themes), \
     (SELECT max(updated_at)::text || count(*) FROM child_themes), \
     (SELECT max(updated_at)::text || count(*) FROM remote_themes), \
     (SELECT max(updated_at)::text || count(*) FROM theme_fields), \
     (SELECT max(updated_at)::text || count(*) FROM theme_settings), \
     (SELECT max(updated_at)::text || count(*) FROM theme_svg_sprites), \
     (SELECT md5(string_agg(s::text, ',' ORDER BY s.id)) FROM theme_modifier_sets s), \
     (SELECT md5(string_agg(c::text, ',' ORDER BY c.id)) FROM color_schemes c), \
     (SELECT max(updated_at)::text || count(*) FROM color_scheme_colors), \
     (SELECT max(updated_at)::text || count(*) FROM discourse_kanban_columns))";

/// The theme tables' state as one request read it.
pub struct Fingerprint {
    database: String,
    value: String,
}

pub async fn read(conn: &mut PgConnection) -> Result<Fingerprint, sqlx::Error> {
    let (database, value) = sqlx::query_as(QUERY).fetch_one(conn).await?;
    Ok(Fingerprint { database, value })
}

type Map<K, V> = HashMap<(String, K), (String, V)>;

/// Values kept per site and key, each with the fingerprint it was built
/// at; one built at another is not handed out.
pub struct Kept<K, V>(Mutex<Option<Map<K, V>>>);

impl<K: Eq + Hash, V: Clone> Kept<K, V> {
    pub const fn new() -> Self {
        Kept(Mutex::new(None))
    }

    pub fn get(&self, at: &Fingerprint, key: K) -> Option<V> {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref()?.get(&(at.database.clone(), key)) {
            Some((built_at, value)) if *built_at == at.value => Some(value.clone()),
            _ => None,
        }
    }

    pub fn put(&self, at: &Fingerprint, key: K, value: V) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if map.len() >= 256 {
            map.clear();
        }
        map.insert((at.database.clone(), key), (at.value.clone(), value));
    }
}

impl<K: Eq + Hash, V: Clone> Default for Kept<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
