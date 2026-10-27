//! Translation overrides in other locales: read with English's, and
//! applied only to lookups in their own locale.

mod common;

use common::TestDb;
use discourse_rs::i18n::I18n;

#[tokio::test]
async fn overrides_apply_in_their_own_locale() {
    let db = TestDb::new().await;
    sqlx::query(
        "INSERT INTO translation_overrides (locale, translation_key, value, \
         original_translation, status, created_at, updated_at) VALUES \
         ('fr', 'js.log_in', 'Entrer', '', 0, now(), now()), \
         ('en', 'js.sign_up', 'Join', '', 0, now(), now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let i18n = I18n::vendored().unwrap();
    let mut conn = db.pool.acquire().await.unwrap();
    i18n.refresh_overrides(&mut conn).await.unwrap();
    assert_eq!(i18n.locale("fr").t("js.log_in"), Some("Entrer"));
    assert_eq!(i18n.t("js.log_in"), Some("Log In"));
    assert_eq!(i18n.t("js.sign_up"), Some("Join"));
    // English's override reaches French only where French has no string.
    assert_ne!(i18n.locale("fr").t("js.sign_up"), Some("Join"));

    sqlx::query("DELETE FROM translation_overrides WHERE locale = 'fr'")
        .execute(&db.pool)
        .await
        .unwrap();
    i18n.refresh_overrides(&mut conn).await.unwrap();
    assert_ne!(i18n.locale("fr").t("js.log_in"), Some("Entrer"));
}
