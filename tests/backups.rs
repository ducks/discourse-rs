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
