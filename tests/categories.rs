//! Category behaviour the parity cases cannot record: states the reference
//! never has (a site without the Horizon theme), and what goes out on the
//! bus (MessageBus lives in Redis on Rails, outside the recordings).

mod common;

use common::{TestDb, config, state};
use discourse_rs::config::RailsEnv;
use discourse_rs::guardian::Guardian;
use discourse_rs::session::current::SessionUser;
use discourse_rs::site_settings::SiteSettings;
use serde_json::Value;

async fn admin_guardian(conn: &mut sqlx::PgConnection) -> Guardian {
    let user = SessionUser::load(conn, 1).await.unwrap().unwrap();
    Guardian::for_user(conn, &user).await.unwrap()
}

/// The Support type's category custom field keys, as the registry lists
/// them for an admin.
async fn support_custom_fields(db: &TestDb) -> Vec<String> {
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    let mut conn = db.pool.acquire().await.unwrap();
    let s = SiteSettings::load(&mut conn, &st.site_setting_defs, &st.config.globals)
        .await
        .unwrap();
    let guardian = admin_guardian(&mut conn).await;
    let types: Value = discourse_rs::category_serializer::with_registry(
        &discourse_rs::category_serializer::Cx::of(&st),
        &s,
        async |r| r.list(&mut conn, &guardian).await,
    )
    .await
    .unwrap();
    let support = types
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "support")
        .unwrap();
    support["configuration_schema"]["category_custom_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap().to_string())
        .collect()
}

// A restored backup from before Horizon has no theme -2, so Horizon is
// not the default theme and Support offers the empty box toggle (Rails
// syncs the core themes on migration, so it always has the row).
#[tokio::test]
async fn category_types_without_horizon() {
    let db = TestDb::new().await;
    assert!(
        support_custom_fields(&db)
            .await
            .contains(&"empty_box_on_unsolved".to_string())
    );
    sqlx::query("DELETE FROM theme_fields WHERE theme_id = -2")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM child_themes WHERE parent_theme_id = -2 OR child_theme_id = -2")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM themes WHERE id = -2")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        support_custom_fields(&db)
            .await
            .contains(&"empty_box_on_unsolved".to_string())
    );
}
