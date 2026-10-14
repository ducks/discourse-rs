//! Backups where the write cases can't show them: the token the download
//! mail carries opens the download, and a merged upload is a backup the
//! staff are told about.

mod common;

use common::{TestDb, bus_messages, recorded_config, state};
use discourse_rs::backup_restore::{self, jobs, store::LocalBackupStore};
use serde_json::json;

fn scratch_public(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "discourse-rs-backups-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
async fn the_mailed_token_opens_the_download_once() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch_public("token");
    let st = state(db.pool.clone(), config).await;
    jobs::download_backup_email(
        &st,
        &json!({"user_id": 1, "backup_file_path": "http://localhost/admin/backups/x.tar.gz?old=1"}),
    )
    .await
    .unwrap();
    let sent = st.mailer.sent();
    assert_eq!(sent.len(), 1);
    let text = sent[0].text.clone();
    let token = text
        .split("x.tar.gz?token=")
        .nth(1)
        .map(|rest| rest[..32].to_string())
        .expect("a token in the link");
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    let mut conn = db.pool.acquire().await.unwrap();
    assert!(
        backup_restore::token::compare(&mut conn, 1, &token)
            .await
            .unwrap()
    );
    assert!(
        !backup_restore::token::compare(&mut conn, 2, &token)
            .await
            .unwrap()
    );
    backup_restore::token::del(&mut conn, 1).await.unwrap();
    assert!(
        !backup_restore::token::compare(&mut conn, 1, &token)
            .await
            .unwrap()
    );
    // A user that does not exist is InvalidParameters.
    assert!(
        jobs::download_backup_email(&st, &json!({"user_id": 999_999, "backup_file_path": "x"}))
            .await
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
}

#[tokio::test]
async fn merged_chunks_become_a_listed_backup() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch_public("merge");
    let st = state(db.pool.clone(), config).await;
    let store = LocalBackupStore::at(&st.config.public_dir).unwrap();
    for (n, part) in [(1, "part one, "), (2, "part two")] {
        let path = store.chunk_path("abc-123", "uploaded.tar.gz", &n.to_string());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, part).unwrap();
    }
    let from = st.bus.now().await.unwrap();
    jobs::backup_chunks_merger(
        &st,
        &json!({"filename": "uploaded.tar.gz", "identifier": "abc-123", "chunks": 2}),
    )
    .await
    .unwrap();
    let merged = std::fs::read_to_string(store.directory().join("uploaded.tar.gz")).unwrap();
    assert_eq!(merged, "part one, part two");
    assert!(!store.directory().join("tmp/abc-123").exists());
    let messages = bus_messages(&st, from, &[backup_restore::FILES_CHANNEL]).await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].data[0]["filename"], "uploaded.tar.gz");
    assert_eq!(messages[0].data[0]["size"], 18);
    for bad in [
        json!({"filename": "", "identifier": "a", "chunks": 1}),
        json!({"filename": "a.tar.gz", "identifier": "", "chunks": 1}),
        json!({"filename": "a.tar.gz", "identifier": "a", "chunks": 0}),
    ] {
        assert!(jobs::backup_chunks_merger(&st, &bad).await.is_err());
    }
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
}

/// `tar -tzf`: the archive's entries.
fn tar_entries(archive: &std::path::Path) -> Vec<String> {
    let out = std::process::Command::new("tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .unwrap();
    assert!(out.status.success(), "tar -tzf {}", archive.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// The SQL of a gzipped dump.
fn gunzip(bytes: &[u8]) -> String {
    use std::io::Write;
    let mut child = std::process::Command::new("gzip")
        .arg("-dc")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let input = bytes.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The `dump.sql.gz` inside a `.tar.gz`.
fn dump_in(archive: &std::path::Path) -> String {
    let out = std::process::Command::new("tar")
        .arg("-xzOf")
        .arg(archive)
        .arg("dump.sql.gz")
        .output()
        .unwrap();
    assert!(out.status.success());
    gunzip(&out.stdout)
}

/// What the archive of a backup holds, as Rails' Creator lays it out (a
/// Rails archive of the reference holds `dump.sql.gz` and
/// `uploads/default/...` without `uploads/default/optimized`): the
/// public schema's dump with the rewritten backup_metadata, the uploads
/// beside it, the name from the title, time and schema version.
#[tokio::test]
async fn a_backup_with_uploads_is_the_dump_and_the_uploads() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch_public("create");
    config.tmp_dir = scratch_public("create-tmp");
    let uploads = config.public_dir.join("uploads/default");
    std::fs::create_dir_all(uploads.join("original/1X")).unwrap();
    std::fs::create_dir_all(uploads.join("optimized/1X")).unwrap();
    std::fs::write(uploads.join("original/1X/abc.png"), "png").unwrap();
    std::fs::write(uploads.join("optimized/1X/abc_2_10x10.png"), "thumb").unwrap();
    let st = state(db.pool.clone(), config).await;
    let from = st.bus.now().await.unwrap();
    let outcome = backup_restore::creator::run(
        &st,
        1,
        backup_restore::creator::Options {
            publish_to_message_bus: true,
            with_uploads: Some(true),
            client_id: Some("abc".into()),
            filename: None,
        },
    )
    .await
    .unwrap();
    assert!(outcome.success, "{:#?}", outcome.logs);
    let name =
        regex::Regex::new(r"^discourse-\d{4}-\d{2}-\d{2}-\d{6}-v20260925054715\.tar\.gz$").unwrap();
    assert!(name.is_match(&outcome.filename), "{}", outcome.filename);
    let archive = st
        .config
        .public_dir
        .join("backups/default")
        .join(&outcome.filename);
    let entries = tar_entries(&archive);
    assert_eq!(entries[0], "dump.sql.gz");
    assert!(entries.contains(&"uploads/default/original/1X/abc.png".to_string()));
    assert!(!entries.iter().any(|e| e.contains("/optimized")));
    // Nothing left beside it: no .tar, no tmp directory.
    let names: Vec<String> = std::fs::read_dir(st.config.public_dir.join("backups/default"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![outcome.filename.clone()]);
    assert!(
        std::fs::read_dir(st.config.tmp_dir.join("backups/default"))
            .unwrap()
            .next()
            .is_none()
    );

    let sql = dump_in(&archive);
    assert!(sql.contains("CREATE TABLE public.posts ("));
    assert!(!sql.contains("CREATE SCHEMA discourse_rs"));
    assert!(sql.contains("COPY public.users "));
    // The metadata as rewritten before the dump.
    let metadata = sql
        .split("COPY public.backup_metadata (id, name, value) FROM stdin;\n")
        .nth(1)
        .and_then(|rest| rest.split("\\.\n").next())
        .unwrap();
    assert_eq!(
        metadata,
        "7\tbase_url\thttp://localhost:3042\n8\tcdn_url\t\\N\n9\ts3_base_url\t\\N\n\
         10\ts3_cdn_url\t\\N\n11\tdb_name\tdefault\n12\tmultisite\tf\n"
    );
    // nested_hot_post_scores: the table, not its rows.
    assert!(!sql.contains("COPY public.nested_hot_post_scores"));

    // The operation is over, and the admin heard it and got the logs.
    let mut conn = db.pool.acquire().await.unwrap();
    assert!(
        !backup_restore::is_operation_running(&mut conn)
            .await
            .unwrap()
    );
    // Delivered (bus_messages waits for that), then all of them: pg_dump's
    // verbose lines are more than its thousand.
    bus_messages(&st, from, &[backup_restore::LOGS_CHANNEL]).await;
    let messages = st
        .bus
        .backlog(
            from,
            &pg_bus::Filter {
                channels: vec![backup_restore::LOGS_CHANNEL.into()],
                tags: vec!["user:1".into()],
            },
            100_000,
        )
        .await
        .unwrap();
    assert_eq!(messages.len(), outcome.logs.len());
    let lines: Vec<&str> = messages
        .iter()
        .map(|m| m.data["message"].as_str().unwrap())
        .collect();
    assert_eq!(lines.first(), Some(&"[STARTED]"));
    assert_eq!(lines.get(1), Some(&"'admin' has started the backup!"));
    assert_eq!(lines.last(), Some(&"[SUCCESS]"));
    assert!(lines.contains(&"Archiving uploads..."));
    assert!(
        messages
            .iter()
            .all(|m| m.audience == Some(vec!["user:1".to_string()])
                && m.data["operation"] == "backup")
    );
    let (title, raw): (String, String) = sqlx::query_as(
        "SELECT t.title, p.raw FROM posts p JOIN topics t ON t.id = p.topic_id \
         JOIN topic_allowed_users tu ON tu.topic_id = t.id AND tu.user_id = 1 \
         WHERE t.archetype = 'private_message' AND p.user_id = -1 ORDER BY p.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(title, "Backup completed successfully");
    assert!(raw.starts_with("The backup was successful."));
    // pg_dump's lines make the logs longer than a post: a log.txt.zip
    // upload of the admin's, linked as an attachment.
    let link = regex::Regex::new(
        r"\[log\.txt\.zip\|attachment\]\(upload://[0-9a-zA-Z]+\.zip\) \(\d+(\.\d+)? KB\)",
    )
    .unwrap();
    assert!(link.is_match(&raw), "{raw}");
    let (url, user_id): (String, i32) = sqlx::query_as(
        "SELECT url, user_id FROM uploads WHERE original_filename = 'log.txt.zip' \
         ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(user_id, 1);
    let zip = std::fs::read(st.config.public_dir.join(url.trim_start_matches('/'))).unwrap();
    assert_eq!(&zip[..4], b"PK\x03\x04");
    let name_len = u16::from_le_bytes([zip[26], zip[27]]) as usize;
    assert_eq!(&zip[30..30 + name_len], b"log.txt");
    let csize = u32::from_le_bytes([zip[18], zip[19], zip[20], zip[21]]) as usize;
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut flate2::read::DeflateDecoder::new(&zip[30 + name_len..30 + name_len + csize]),
        &mut text,
    )
    .unwrap();
    assert!(text.contains("] 'admin' has started the backup!\n"));
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
    let _ = std::fs::remove_dir_all(&st.config.tmp_dir);
}

/// Without uploads the backup is the dump itself, `.sql.gz`; and a second
/// operation while one runs fails (and, as Rails' ensure does, clears the
/// running flag), telling the admin.
#[tokio::test]
async fn a_dump_only_backup_and_one_refused_while_running() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch_public("dump");
    config.tmp_dir = scratch_public("dump-tmp");
    let st = state(db.pool.clone(), config).await;
    let options = backup_restore::creator::Options {
        with_uploads: Some(true),
        ..Default::default()
    };
    // No local uploads: with_uploads true still leaves them out.
    let outcome = backup_restore::creator::run(&st, 1, options.clone())
        .await
        .unwrap();
    assert!(outcome.success, "{:#?}", outcome.logs);
    assert!(outcome.filename.ends_with("-v20260925054715.sql.gz"));
    let dump = std::fs::read(
        st.config
            .public_dir
            .join("backups/default")
            .join(&outcome.filename),
    )
    .unwrap();
    assert!(gunzip(&dump).contains("CREATE TABLE public.topics ("));

    let mut conn = db.pool.acquire().await.unwrap();
    backup_restore::mark_as_running(&mut conn).await.unwrap();
    let outcome = backup_restore::creator::run(&st, 1, options).await.unwrap();
    assert!(!outcome.success);
    assert!(
        outcome
            .logs
            .iter()
            .any(|l| l.ends_with("] EXCEPTION: BackupRestore::OperationRunningError"))
    );
    assert!(outcome.logs.last().unwrap().ends_with("] [FAILED]"));
    assert!(
        !backup_restore::is_operation_running(&mut conn)
            .await
            .unwrap()
    );
    let title: String = sqlx::query_scalar(
        "SELECT t.title FROM topics t JOIN topic_allowed_users tu ON tu.topic_id = t.id \
         AND tu.user_id = 1 WHERE t.archetype = 'private_message' ORDER BY t.id DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(title, "Backup failed");
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
    let _ = std::fs::remove_dir_all(&st.config.tmp_dir);
}

/// Cancelling: the shutdown flag stops the running backup, which says so
/// and leaves no archive.
#[tokio::test]
async fn a_cancelled_backup_stops() {
    let db = TestDb::new().await;
    let mut config = recorded_config();
    config.public_dir = scratch_public("cancel");
    config.tmp_dir = scratch_public("cancel-tmp");
    let st = state(db.pool.clone(), config).await;
    let run = {
        let st = st.clone();
        tokio::spawn(async move {
            backup_restore::creator::run(&st, 1, Default::default())
                .await
                .unwrap()
        })
    };
    // As soon as it runs, cancel it (the dump takes longer than this).
    let mut conn = db.pool.acquire().await.unwrap();
    for _ in 0..200 {
        if backup_restore::is_operation_running(&mut conn)
            .await
            .unwrap()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    backup_restore::set_shutdown_signal(&mut conn)
        .await
        .unwrap();
    let outcome = run.await.unwrap();
    assert!(!outcome.success);
    assert!(
        outcome
            .logs
            .iter()
            .any(|l| l.ends_with("] Backup process was cancelled!"))
    );
    let left: Vec<_> = std::fs::read_dir(st.config.public_dir.join("backups/default"))
        .unwrap()
        .collect();
    assert!(left.is_empty());
    let _ = std::fs::remove_dir_all(&st.config.public_dir);
    let _ = std::fs::remove_dir_all(&st.config.tmp_dir);
}
