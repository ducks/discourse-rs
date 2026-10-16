//! The S3 API backend against a real S3-compatible server (MinIO,
//! tests/common/s3.rs): every operation the stores use, with the bucket
//! addressed both ways (virtual-hosted and path-style). Skipped, saying
//! so, without MinIO.

mod common;

use common::s3::{ACCESS_KEY_ID, REGION, S3Server, SECRET_ACCESS_KEY};
use discourse_rs::object_storage::{Backend, Part, PutOptions, StorageError, s3};

fn server() -> Option<S3Server> {
    if let Err(why) = S3Server::available() {
        eprintln!("skipped: {why}");
        return None;
    }
    Some(S3Server::start().expect("MinIO starts"))
}

/// The bucket addressed in the path, on the server's IP.
fn path_style(server: &S3Server, bucket: &str) -> Backend {
    let port = server.endpoint.rsplit(':').next().unwrap();
    Backend::S3(
        s3::S3::new(&s3::S3Config {
            bucket: bucket.into(),
            region: REGION.into(),
            endpoint: Some(format!("http://127.0.0.1:{port}")),
            access_key_id: ACCESS_KEY_ID.into(),
            secret_access_key: SECRET_ACCESS_KEY.into(),
            session_token: None,
            force_path_style: false,
            dualstack: false,
            accelerate: false,
        })
        .unwrap(),
    )
}

async fn exercise(bucket: &Backend) {
    bucket.ensure_bucket().await.unwrap();
    // Twice is fine: it is there.
    bucket.ensure_bucket().await.unwrap();

    let opts = PutOptions {
        content_type: Some("text/plain".into()),
        cache_control: Some("max-age=60, public, immutable".into()),
        content_disposition: Some("inline; filename=\"a b.txt\"".into()),
        metadata: vec![("sha1-checksum".into(), "abc".into())],
        ..Default::default()
    };
    let etag = bucket
        .put("dir/a b.txt", b"hello".to_vec(), &opts)
        .await
        .unwrap();
    assert_eq!(etag, format!("{:x}", md5::compute(b"hello")));
    assert_eq!(
        bucket.get("dir/a b.txt").await.unwrap().as_deref(),
        Some(&b"hello"[..])
    );
    assert_eq!(bucket.get("dir/missing").await.unwrap(), None);
    let meta = bucket.head("dir/a b.txt").await.unwrap().unwrap();
    assert_eq!(meta.size, 5);
    assert_eq!(meta.content_type.as_deref(), Some("text/plain"));
    assert_eq!(
        meta.content_disposition.as_deref(),
        Some("inline; filename=\"a b.txt\"")
    );
    assert_eq!(
        meta.metadata.get("sha1-checksum").map(String::as_str),
        Some("abc")
    );
    assert!(bucket.head("dir/missing").await.unwrap().is_none());

    // A copy keeps the source's metadata, or takes the options given.
    bucket
        .copy("dir/a b.txt", "copy/kept.txt", None)
        .await
        .unwrap();
    let kept = bucket.head("copy/kept.txt").await.unwrap().unwrap();
    assert_eq!(kept.content_type.as_deref(), Some("text/plain"));
    let replaced = PutOptions {
        content_type: Some("application/gzip".into()),
        ..Default::default()
    };
    bucket
        .copy("dir/a b.txt", "copy/replaced.gz", Some(&replaced))
        .await
        .unwrap();
    let meta = bucket.head("copy/replaced.gz").await.unwrap().unwrap();
    assert_eq!(meta.content_type.as_deref(), Some("application/gzip"));
    assert!(matches!(
        bucket.copy("dir/missing", "copy/x", None).await,
        Err(StorageError::NotFound | StorageError::Service { .. })
    ));

    let keys: Vec<String> = bucket
        .list("copy/")
        .await
        .unwrap()
        .into_iter()
        .map(|o| o.key)
        .collect();
    assert_eq!(keys, ["copy/kept.txt", "copy/replaced.gz"]);

    // A presigned download, with the disposition asked for.
    let url = bucket
        .presigned_get("dir/a b.txt", 300, Some("attachment; filename=\"x y.txt\""))
        .unwrap();
    let response = reqwest::get(&url).await.unwrap();
    assert_eq!(response.status(), 200, "{url}");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"x y.txt\""
    );
    assert_eq!(response.text().await.unwrap(), "hello");

    // A presigned upload, sent with the headers it was signed with.
    let put = PutOptions {
        acl: Some("private".into()),
        metadata: vec![("sha1-checksum".into(), "def".into())],
        ..Default::default()
    };
    let (url, headers) = bucket.presigned_put("temp/up.bin", 600, &put).unwrap();
    let mut request = reqwest::Client::new().put(&url).body("uploaded");
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let meta = bucket.head("temp/up.bin").await.unwrap().unwrap();
    assert_eq!(
        meta.metadata.get("sha1-checksum").map(String::as_str),
        Some("def")
    );

    // A multipart upload, its parts put at presigned URLs.
    let upload_id = bucket
        .create_multipart(
            "temp/multi.gz",
            &PutOptions {
                content_type: Some("application/gzip".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        bucket
            .list_parts("temp/multi.gz", &upload_id, 1)
            .await
            .unwrap()
            .is_empty()
    );
    let mut parts = Vec::new();
    // Every part but the last at least 5 MB.
    for (number, body) in [(1u16, vec![b'a'; 5 * 1024 * 1024]), (2, b"end".to_vec())] {
        let url = bucket
            .presign_part("temp/multi.gz", &upload_id, number, 600)
            .unwrap();
        let response = reqwest::Client::new()
            .put(&url)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let etag = response.headers()["etag"]
            .to_str()
            .unwrap()
            .replace('"', "");
        parts.push(Part { number, etag });
    }
    assert_eq!(
        bucket
            .list_parts("temp/multi.gz", &upload_id, 1000)
            .await
            .unwrap(),
        parts
    );
    bucket
        .complete_multipart("temp/multi.gz", &upload_id, &parts)
        .await
        .unwrap();
    let meta = bucket.head("temp/multi.gz").await.unwrap().unwrap();
    assert_eq!(meta.size, 5 * 1024 * 1024 + 3);
    assert!(matches!(
        bucket.list_parts("temp/multi.gz", &upload_id, 1).await,
        Err(StorageError::NoSuchUpload)
    ));
    let aborted = bucket
        .create_multipart("temp/gone", &PutOptions::default())
        .await
        .unwrap();
    bucket.abort_multipart("temp/gone", &aborted).await.unwrap();

    // A file large enough to go up in parts.
    let dir = std::env::temp_dir().join(format!("discourse-rs-s3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.bin");
    std::fs::write(&big, vec![7u8; (s3::MULTIPART_THRESHOLD + 10) as usize]).unwrap();
    bucket
        .put_file("backups/big.bin", &big, &PutOptions::default())
        .await
        .unwrap();
    let copy = dir.join("copy.bin");
    bucket.download("backups/big.bin", &copy).await.unwrap();
    assert_eq!(
        std::fs::metadata(&copy).unwrap().len(),
        s3::MULTIPART_THRESHOLD + 10
    );
    std::fs::remove_dir_all(&dir).unwrap();

    // Expiry rules: one per id, the others kept.
    bucket.update_lifecycle("other", 3, "logs/").await.unwrap();
    bucket
        .update_lifecycle("purge_tombstone", 30, "tombstone/")
        .await
        .unwrap();
    bucket
        .update_lifecycle("purge-tombstone", 7, "tombstone/")
        .await
        .unwrap();
    let mut rules: Vec<(Option<String>, Option<i64>, Option<String>)> = bucket
        .lifecycle_rules()
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.id, r.days, r.prefix))
        .collect();
    rules.sort();
    assert_eq!(
        rules,
        [
            (Some("other".into()), Some(3), Some("logs/".into())),
            (
                Some("purge-tombstone".into()),
                Some(7),
                Some("tombstone/".into())
            ),
        ]
    );

    bucket.delete("dir/a b.txt").await.unwrap();
    bucket.delete("dir/a b.txt").await.unwrap();
    assert!(bucket.head("dir/a b.txt").await.unwrap().is_none());
}

#[tokio::test]
async fn virtual_hosted_buckets() {
    let Some(server) = server() else {
        return;
    };
    exercise(&server.bucket("virtual-hosted")).await;
}

#[tokio::test]
async fn path_style_buckets() {
    let Some(server) = server() else {
        return;
    };
    exercise(&path_style(&server, "path-style")).await;
}

#[tokio::test]
async fn errors_name_what_the_service_said() {
    let Some(server) = server() else {
        return;
    };
    let wrong = Backend::S3(
        s3::S3::new(&s3::S3Config {
            bucket: "nope".into(),
            region: REGION.into(),
            endpoint: Some(server.endpoint.clone()),
            access_key_id: ACCESS_KEY_ID.into(),
            secret_access_key: "not-the-secret".into(),
            session_token: None,
            force_path_style: false,
            dualstack: false,
            accelerate: false,
        })
        .unwrap(),
    );
    let e = wrong.list("").await.unwrap_err();
    assert_eq!(e.code(), "SignatureDoesNotMatch", "{e}");
    let e = server.bucket("no-such-bucket").list("").await.unwrap_err();
    assert_eq!(e.code(), "NoSuchBucket");
    assert_eq!(e.to_string(), "The specified bucket does not exist");
}
