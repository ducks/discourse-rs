//! Restoring a backup, which the write cases can't record (it replaces
//! the whole public schema under the recorder's transaction): a backup
//! the port made, put back over a changed database, compared row by row;
//! the rollback; the refusals; and DropBackupSchema's week.

mod common;

use common::{TestDb, bus_messages, recorded_config, state};
use discourse_rs::backup_restore::{self, creator, restorer};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "discourse-rs-restore-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn scalar<T>(pool: &sqlx::PgPool, sql: &str) -> T
where
    T: Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// What the restore must bring back, per table: its rows, as text, in a
/// stable order.
async fn table_digest(pool: &sqlx::PgPool, schema: &str, table: &str) -> String {
    sqlx::query_scalar(&format!(
        "SELECT md5(COALESCE(string_agg(x::text, '|' ORDER BY x::text), '')) FROM {schema}.\"{table}\" x"
    ))
    .fetch_one(pool)
    .await
    .unwrap()
}

/// theme_fields, the digests the basic SCSS fields bake left out.
async fn theme_fields_but_scss_digests(pool: &sqlx::PgPool) -> String {
    sqlx::query_scalar(
        "SELECT md5(string_agg(x::text, '|' ORDER BY x.id)) FROM ( \
           SELECT id, theme_id, target_id, name, value, compiler_version, error, upload_id, \
             type_id, created_at, updated_at, \
             CASE WHEN type_id = 1 AND target_id IN (0, 1, 2) \
               AND name IN ('scss', 'embedded_scss', 'color_definitions') \
               THEN NULL ELSE value_baked END AS value_baked \
           FROM theme_fields) x",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The basic SCSS fields' baked digests, by id.
async fn scss_digests(pool: &sqlx::PgPool) -> Vec<(i32, Option<String>)> {
    sqlx::query_as(
        "SELECT id, value_baked FROM theme_fields WHERE type_id = 1 AND target_id IN (0, 1, 2) \
         AND name IN ('scss', 'embedded_scss', 'color_definitions') ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_backup_restored_over_a_changed_site_brings_it_back() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch("roundtrip-public");
    config.tmp_dir = scratch("roundtrip-tmp");
    let uploads = config.public_dir.join("uploads/default/original/1X");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("kept.png"), "png").unwrap();
    let st = state(db.pool.clone(), config).await;
    sqlx::query("UPDATE users SET name = 'Before backup' WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();

    // The posts the backup holds (not the messages sent after).
    let last_post: i32 = scalar(&db.pool, "SELECT max(id) FROM posts").await;
    let backup = creator::run(
        &st,
        1,
        creator::Options {
            with_uploads: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(backup.success, "{:#?}", backup.logs);
    assert!(backup.filename.ends_with(".tar.gz"));

    // The tables as backed up (backup_metadata as the dump has it).
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename::text FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let mut before = Vec::new();
    for table in &tables {
        before.push(table_digest(&db.pool, "public", table).await);
    }
    let fields_before = theme_fields_but_scss_digests(&db.pool).await;
    let scss_before = scss_digests(&db.pool).await;
    assert!(!scss_before.is_empty());

    // The site moves on: a renamed admin, a new topic, an upload gone.
    sqlx::query("UPDATE users SET name = 'After backup' WHERE id = 1")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM topic_links")
        .execute(&db.pool)
        .await
        .unwrap();
    std::fs::remove_file(uploads.join("kept.png")).unwrap();

    let from = st.bus.now().await.unwrap();
    let outcome = restorer::run(
        &st,
        1,
        restorer::Options {
            filename: Some(backup.filename.clone()),
            client_id: Some("abc".into()),
            disable_emails: true,
        },
    )
    .await
    .unwrap();
    assert!(outcome.success, "{:#?}", outcome.logs);
    let lines: Vec<&str> = outcome
        .logs
        .iter()
        .map(|l| l.split_once("] ").map_or(l.as_str(), |(_, m)| m))
        .collect();
    for expected in [
        "[STARTED]",
        "'admin' has started the restore!",
        "Marking restore as running...",
        "Copying archive to tmp directory...",
        "Unzipping archive, this may take a while...",
        "Extracting dump file...",
        "Validating metadata...",
        "  Current version: 20260925054715",
        "  Restored version: 20260925054715",
        "Enabling readonly mode...",
        "Pausing sidekiq...",
        "Waiting up to 60 seconds for Sidekiq to finish running jobs...",
        "Creating missing functions in the discourse_functions schema...",
        "Restoring dump file... (this may take a while)",
        "Migrating the database...",
        "Reconnecting to the database...",
        "Reloading site settings...",
        "Disabling outgoing emails for non-staff users...",
        "Running seed fu...",
        "Failed to run seed fu",
        "Disabling readonly mode...",
        "Remapping uploads...",
        "Restoring uploads, this may take a while...",
        "Optimizing site icons...",
        "Clear theme cache",
        "Executing the after_restore_hook...",
        "Cleaning stuff up...",
        "Unpausing sidekiq...",
        "Marking restore as finished...",
        "Notifying 'admin' of the end of the restore...",
        "Finished!",
        "[SUCCESS]",
    ] {
        assert!(lines.contains(&expected), "no {expected:?} in {lines:#?}");
    }
    assert_eq!(lines.last(), Some(&"[SUCCESS]"));
    assert!(
        !lines.contains(&"Something went wrong while clearing theme cache."),
        "{lines:#?}"
    );

    // Every table as backed up, but what the restore itself writes.
    let changed_by_restore = [
        // last_restore_date
        "backup_metadata",
        // disable_emails set and logged
        "site_settings",
        "user_histories",
        // emptied, then the site icons made again
        "optimized_images",
        // posts with uploads marked for a rebake, and the restore's
        // message with its log.txt.zip
        "posts",
        "topics",
        "topic_allowed_users",
        "topic_users",
        "topic_links",
        "post_search_data",
        "topic_search_data",
        "post_stats",
        "user_actions",
        "uploads",
        "upload_references",
        "user_uploads",
        // ThemeField.force_recompilation!: what Rails changes on the seed
        // (measured on the reference in a rolled back transaction) is the
        // digest each SCSS field bakes, of its compile with a source map
        // naming a temporary directory, so new on every compile.
        "theme_fields",
        // and the translation fields' scripts, made again (the seed has
        // none: Rails rebuilds the cache).
        "javascript_caches",
    ];
    for (table, digest) in tables.iter().zip(&before) {
        if changed_by_restore.contains(&table.as_str()) {
            continue;
        }
        assert_eq!(
            &table_digest(&db.pool, "public", table).await,
            digest,
            "{table} differs from the backup"
        );
    }
    assert_eq!(theme_fields_but_scss_digests(&db.pool).await, fields_before);
    let scss_after = scss_digests(&db.pool).await;
    assert_eq!(scss_after.len(), scss_before.len());
    for (b, a) in scss_before.iter().zip(&scss_after) {
        assert_eq!(a.0, b.0);
        assert_ne!(a.1, b.1, "field {} was not compiled again", a.0);
    }
    // The scripts Rails' force_recompilation! makes on the seed, measured
    // on the reference (parity/backup_restore): field, theme, digest, a
    // SHA1 of the content and whether the source map is nil.
    let caches: Vec<(i64, Option<i64>, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT theme_field_id, theme_id, digest, content, source_map FROM javascript_caches \
         ORDER BY theme_field_id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let ours: Vec<String> = caches
        .iter()
        .map(|(field, theme, digest, content, map)| {
            use sha1::Digest;
            format!(
                "{field} {} {} {:x} {}",
                theme.map_or("nil".to_string(), |t| t.to_string()),
                digest.as_deref().unwrap_or_default(),
                sha1::Sha1::digest(content.as_bytes()),
                map.is_none()
            )
        })
        .collect();
    let rails = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("parity/backup_restore/recompiled_javascript_caches.txt"),
    )
    .unwrap();
    assert_eq!(ours, rails.lines().collect::<Vec<_>>());
    let name: String = scalar(&db.pool, "SELECT name FROM users WHERE id = 1").await;
    assert_eq!(name, "Before backup");
    // The old tables wait in `backup`.
    let old: String = scalar(&db.pool, "SELECT name FROM backup.users WHERE id = 1").await;
    assert_eq!(old, "After backup");
    let mut conn = db.pool.acquire().await.unwrap();
    assert!(backup_restore::can_rollback(&mut conn).await.unwrap());
    assert!(
        !backup_restore::is_operation_running(&mut conn)
            .await
            .unwrap()
    );
    assert!(
        !discourse_rs::readonly::mode(&mut conn)
            .await
            .unwrap()
            .site_readonly()
    );
    assert!(
        discourse_rs::owned_schema::cached_get(&mut conn, restorer::PAUSED_KEY)
            .await
            .unwrap()
            .is_none()
    );
    let restored_at: String = scalar(
        &db.pool,
        "SELECT value FROM backup_metadata WHERE name = 'last_restore_date'",
    )
    .await;
    assert!(restored_at.ends_with('Z'), "{restored_at}");
    let disable_emails: String = scalar(
        &db.pool,
        "SELECT value FROM site_settings WHERE name = 'disable_emails'",
    )
    .await;
    assert_eq!(disable_emails, "non-staff");
    // The uploads are back, and posts with uploads wait for a rebake.
    assert!(uploads.join("kept.png").exists());
    // (Not the restore's own message, with its log.txt.zip.)
    let unbaked: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts WHERE id IN \
           (SELECT target_id FROM upload_references WHERE target_type = 'Post') \
         AND baked_version IS NOT NULL AND id <= $1",
    )
    .bind(last_post)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(unbaked, 0);
    // Read only on and off again, for everyone.
    let readonly: Vec<serde_json::Value> =
        bus_messages(&st, from, &[discourse_rs::readonly::CHANNEL])
            .await
            .into_iter()
            .map(|m| m.data)
            .collect();
    assert_eq!(
        readonly,
        vec![serde_json::json!(true), serde_json::json!(false)]
    );
    let title: String = scalar(
        &db.pool,
        "SELECT t.title FROM topics t JOIN topic_allowed_users tu ON tu.topic_id = t.id \
         AND tu.user_id = 1 WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .await;
    assert_eq!(title, "Restore completed successfully");

    // POST /admin/backups/rollback's move: the site as it was before.
    restorer::move_tables_between_schemas(&st, "backup", "public")
        .await
        .unwrap();
    let name: String = scalar(&db.pool, "SELECT name FROM users WHERE id = 1").await;
    assert_eq!(name, "After backup");
    assert!(!backup_restore::can_rollback(&mut conn).await.unwrap());
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
    let _ = std::fs::remove_dir_all(&st.config.tmp_dir);
}

/// A gzipped SQL file in the store, as a `.sql.gz` backup.
fn sql_backup(public: &std::path::Path, name: &str, sql: &str) {
    use std::io::Write;
    let dir = public.join("backups/default");
    std::fs::create_dir_all(&dir).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(sql.as_bytes()).unwrap();
    std::fs::write(dir.join(name), gz.finish().unwrap()).unwrap();
}

#[tokio::test]
async fn restores_that_fail_leave_the_site_as_it_was() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch("fail-public");
    config.tmp_dir = scratch("fail-tmp");
    let st = state(db.pool.clone(), config).await;
    let options = |filename: &str| restorer::Options {
        filename: Some(filename.to_string()),
        client_id: None,
        disable_emails: true,
    };
    let message =
        |outcome: &restorer::Outcome, text: &str| outcome.logs.iter().any(|l| l.ends_with(text));

    // A newer schema: MigrationRequiredError, nothing touched.
    sql_backup(
        &st.config.public_dir,
        "newer-2026-10-14-000000-v29990101000000.sql.gz",
        "SELECT 1;\n",
    );
    let outcome = restorer::run(
        &st,
        1,
        options("newer-2026-10-14-000000-v29990101000000.sql.gz"),
    )
    .await
    .unwrap();
    assert!(!outcome.success);
    assert!(message(
        &outcome,
        "] EXCEPTION: You're trying to restore a more recent version of the schema. You should migrate first!"
    ));
    assert!(message(&outcome, "] There was no need to rollback"));
    assert!(message(&outcome, "] [FAILED]"));

    // No version in the name.
    sql_backup(&st.config.public_dir, "nameless.sql.gz", "SELECT 1;\n");
    let outcome = restorer::run(&st, 1, options("nameless.sql.gz"))
        .await
        .unwrap();
    assert!(message(
        &outcome,
        "] EXCEPTION: Migration version is missing from the filename."
    ));

    // A dump psql refuses: the tables moved back.
    sql_backup(
        &st.config.public_dir,
        "broken-2026-10-14-000000-v20260925054715.sql.gz",
        "CREATE TABLE public.posts (id integer);\nTHIS IS NOT SQL;\n",
    );
    let posts: i64 = scalar(&db.pool, "SELECT COUNT(*) FROM posts WHERE user_id <> -1").await;
    let outcome = restorer::run(
        &st,
        1,
        options("broken-2026-10-14-000000-v20260925054715.sql.gz"),
    )
    .await
    .unwrap();
    assert!(!outcome.success);
    assert!(
        outcome
            .logs
            .iter()
            .any(|l| l.contains("] EXCEPTION: psql failed: ")),
        "{:#?}",
        outcome.logs
    );
    assert!(message(&outcome, "] Rolling back..."));
    let after: i64 = scalar(&db.pool, "SELECT COUNT(*) FROM posts WHERE user_id <> -1").await;
    assert_eq!(after, posts);
    let mut conn = db.pool.acquire().await.unwrap();
    assert!(!backup_restore::can_rollback(&mut conn).await.unwrap());
    assert!(
        !discourse_rs::readonly::mode(&mut conn)
            .await
            .unwrap()
            .site_readonly()
    );
    // The failure reached the admin.
    let title: String = scalar(
        &db.pool,
        "SELECT t.title FROM topics t JOIN topic_allowed_users tu ON tu.topic_id = t.id \
         AND tu.user_id = 1 WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .await;
    assert_eq!(title, "Restore failed");

    // Production without allow_restore: Restorer.new refuses.
    let mut production = recorded_config();
    production.rails_env = discourse_rs::config::RailsEnv::Production;
    production.public_dir = st.config.public_dir.clone();
    let prod = state(db.pool.clone(), production).await;
    let refused = restorer::run(&prod, 1, options("nameless.sql.gz")).await;
    assert_eq!(
        refused.unwrap_err().to_string(),
        "BackupRestore::RestoreDisabledError"
    );
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
    let _ = std::fs::remove_dir_all(&st.config.tmp_dir);
}

#[tokio::test]
async fn the_backup_schema_is_dropped_a_week_after_the_restore() {
    let db = TestDb::new().await;
    let st = state(db.pool.clone(), recorded_config()).await;
    let exists = || async {
        scalar::<bool>(
            &db.pool,
            "SELECT EXISTS (SELECT 1 FROM information_schema.schemata WHERE schema_name = 'backup')",
        )
        .await
    };
    sqlx::query("CREATE SCHEMA backup")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE backup.posts (id integer)")
        .execute(&db.pool)
        .await
        .unwrap();
    // No date: today's recorded, the schema kept.
    restorer::drop_backup_schema(&st).await.unwrap();
    assert!(exists().await);
    let dated: i64 = scalar(
        &db.pool,
        "SELECT COUNT(*) FROM backup_metadata WHERE name = 'last_restore_date'",
    )
    .await;
    assert_eq!(dated, 1);
    restorer::drop_backup_schema(&st).await.unwrap();
    assert!(exists().await);
    // Eight days on: dropped.
    sqlx::query(
        "UPDATE backup_metadata SET value = to_char((now() - interval '8 days') AT TIME ZONE 'UTC', \
         'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') WHERE name = 'last_restore_date'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    restorer::drop_backup_schema(&st).await.unwrap();
    assert!(!exists().await);
}
