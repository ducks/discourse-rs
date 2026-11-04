//! Jobs::Chat::ProcessMessage and Chat::MessageProcessor
//! (plugins/chat/spec/jobs/regular/chat/process_message_spec.rb,
//! spec/lib/chat/message_processor_spec.rb): oneboxes of the site's own
//! topics and of other sites (from the onebox cache), the processed
//! message published, uploaded images given their lightbox, hotlinked
//! images localized from their tracked rows (and the rest handed to
//! Jobs::Chat::PullHotlinkedImages), optimized videos, and the job's
//! notifications.

use crate::common;

use common::{TestDb, bus_messages, config, set_setting, state};
use discourse_rs::AppState;
use discourse_rs::config::RailsEnv;
use serde_json::{Value, json};

const BASE62: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// `Upload.base62_sha1`
fn base62(sha1: &str) -> String {
    let mut digits: Vec<u32> = sha1.chars().map(|c| c.to_digit(16).unwrap()).collect();
    let mut out = Vec::new();
    while digits.iter().any(|&d| d != 0) {
        let mut remainder = 0u32;
        for digit in digits.iter_mut() {
            let value = remainder * 16 + *digit;
            *digit = value / 62;
            remainder = value % 62;
        }
        out.push(BASE62[remainder as usize]);
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

async fn set(st: &AppState, name: &str, value: &str) {
    let data_type = st.site_setting_defs.get(name).unwrap().data_type as i32;
    set_setting(&st.pool, name, data_type, value).await;
}

/// The site, downloading remote images as Discourse does by default (the
/// test database has it off).
async fn site(db: &TestDb) -> AppState {
    let st = state(db.pool.clone(), config(RailsEnv::Test, &[])).await;
    set(&st, "download_remote_images_to_local", "t").await;
    st
}

/// A message of user1's (3) in General (2). Returns its id.
async fn message(db: &TestDb, raw: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO chat_messages (chat_channel_id, user_id, created_at, updated_at, message, cooked, cooked_version, last_editor_id) \
         VALUES (2, 3, now(), now(), $1, '', 1, 3) RETURNING id",
    )
    .bind(raw)
    .fetch_one(&db.pool)
    .await
    .unwrap()
}

/// An upload of `width` by `height`. Returns (id, url, sha1).
async fn upload(db: &TestDb, name: &str, width: i32, height: i32) -> (i32, String, String) {
    let n: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(id), 0)::int8 + 1 FROM uploads")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let sha1 = format!("{:040x}", 0xab_c000 + n);
    let ext = name.rsplit('.').next().unwrap();
    let url = format!("/uploads/default/original/1X/{sha1}.{ext}");
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO uploads (user_id, original_filename, filesize, width, height, thumbnail_width, \
           thumbnail_height, url, sha1, extension, dominant_color, created_at, updated_at) \
         VALUES (3, $1, 2048, $2, $3, $2, $3, $4, $5, $6, 'aabbcc', now(), now()) RETURNING id",
    )
    .bind(name)
    .bind(width)
    .bind(height)
    .bind(&url)
    .bind(&sha1)
    .bind(ext)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    (id, url, sha1)
}

async fn process(st: &AppState, args: Value) -> Result<(), discourse_rs::AppError> {
    let mut conn = st.pool.acquire().await.unwrap();
    let settings = discourse_rs::site_settings::SiteSettings::load(
        &mut conn,
        &st.site_setting_defs,
        &st.config.globals,
    )
    .await
    .unwrap();
    discourse_rs::plugins::chat::create::process_message(st, &mut conn, &settings, &args).await
}

async fn processed(st: &AppState, id: i64) {
    process(st, json!({ "chat_message_id": id })).await.unwrap();
}

async fn cooked(db: &TestDb, id: i64) -> String {
    sqlx::query_scalar("SELECT cooked FROM chat_messages WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .unwrap()
}

/// The jobs enqueued under `name`, their args.
async fn jobs(db: &TestDb, name: &str) -> Vec<Value> {
    sqlx::query_scalar("SELECT args FROM discourse_rs.jobs WHERE name = $1 ORDER BY id")
        .bind(name)
        .fetch_all(&db.pool)
        .await
        .unwrap()
}

const PULL: &str = "Jobs::Chat::PullHotlinkedImages";

/// A topic user1 can see: its url on the site, and its title.
async fn topic(db: &TestDb) -> (String, String) {
    let (id, slug, title): (i32, String, String) = sqlx::query_as(
        "SELECT t.id, t.slug, t.title FROM topics t JOIN categories c ON c.id = t.category_id \
         WHERE t.archetype = 'regular' AND t.deleted_at IS NULL AND t.visible \
           AND NOT c.read_restricted AND t.posts_count > 0 ORDER BY t.id LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    (format!("http://test.localhost/t/{slug}/{id}"), title)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_to_a_topic_is_oneboxed_and_published_processed() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let (url, title) = topic(&db).await;
    let id = message(&db, &url).await;
    let from = st.bus.now().await.unwrap();
    processed(&st, id).await;
    let html = cooked(&db, id).await;
    assert!(html.starts_with("<aside class=\"quote"), "{html}");
    assert!(html.contains(&title), "{html}");
    assert!(!html.contains("class=\"onebox\""), "{html}");
    // Chat::Publisher.publish_processed!: the message as processed.
    let published = bus_messages(&st, from, &["/chat/2"]).await;
    let processed: Vec<&Value> = published
        .iter()
        .map(|m| &m.data)
        .filter(|d| d["type"] == "processed")
        .collect();
    assert_eq!(processed.len(), 1, "{published:?}");
    assert_eq!(processed[0]["chat_message"]["id"], id);
    assert_eq!(processed[0]["chat_message"]["cooked"], html.as_str());
}

/// The spec's onebox of https://discourse.org/team, here of a host that
/// never resolves (.invalid), taken from Oneboxer's cache as Rails
/// takes a computed one; no request is made while it is cached.
#[tokio::test(flavor = "multi_thread")]
async fn a_link_to_another_site_is_oneboxed_from_the_onebox_cache() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let url = "https://onebox.invalid/team";
    let onebox = format!(
        "<aside class=\"onebox allowlistedgeneric\" data-onebox-src=\"{url}\">\n  <header class=\"source\">\n\n      <a href=\"{url}\" target=\"_blank\" rel=\"nofollow ugc noopener\">onebox.invalid</a>\n  </header>\n\n  <article class=\"onebox-body\">\n    \n\n<h3><a href=\"{url}\" target=\"_blank\" rel=\"nofollow ugc noopener\">a</a></h3>\n\n\n\n  </article>\n\n  <div class=\"onebox-metadata\">\n    \n    \n  </div>\n\n  <div style=\"clear: both\"></div>\n</aside>\n"
    );
    sqlx::query(
        "INSERT INTO discourse_rs.cached_values (key, value, expires_at) \
         VALUES ($1, $2, clock_timestamp() + interval '1 day')",
    )
    .bind(format!("onebox__{url}"))
    .bind(json!({ "onebox": onebox, "preview": onebox }).to_string())
    .execute(&db.pool)
    .await
    .unwrap();
    let id = message(&db, url).await;
    processed(&st, id).await;
    assert_eq!(cooked(&db, id).await, onebox);
    // The link extracted (Chat::MessageLink).
    let links: Vec<String> =
        sqlx::query_scalar("SELECT url FROM chat_message_links WHERE chat_message_id = $1")
            .bind(id)
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(links, vec![url.to_string()]);

    // invalidate_oneboxes: the cached onebox dropped and computed again;
    // the host doesn't resolve, so the link stays a link.
    process(
        &st,
        json!({ "chat_message_id": id, "invalidate_oneboxes": true }),
    )
    .await
    .unwrap();
    assert_eq!(
        cooked(&db, id).await,
        format!(
            "<p><a href=\"{url}\" class=\"onebox\" target=\"_blank\" rel=\"noopener nofollow ugc\">{url}</a></p>"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_action_keeps_its_formatting() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let id = message(&db, "/me waves").await;
    processed(&st, id).await;
    assert_eq!(
        cooked(&db, id).await,
        "<p><em class=\"chat-message-action\">user1 waves</em></p>"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_message_is_left_alone() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    process(&st, json!({ "chat_message_id": 999_999 }))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_uploaded_image_gets_its_lightbox_also_quoted() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let (_, url, sha1) = upload(&db, "cat.png", 800, 600).await;
    let b62 = base62(&sha1);
    let expected_img = |extra: &str| {
        format!(
            "<img src=\"{url}\" alt=\"cat\" data-base62-sha1=\"{b62}\" width=\"500\" height=\"375\"{extra} data-large-src=\"//test.localhost{url}\" data-download-href=\"/uploads/short-url/{b62}.png\" data-target-width=\"800\" data-target-height=\"600\" class=\"lightbox\">"
        )
    };
    let id = message(&db, &format!("![cat|500x375](upload://{b62}.png)")).await;
    processed(&st, id).await;
    let html = cooked(&db, id).await;
    assert_eq!(html, format!("<p>{}</p>", expected_img("")), "{html}");

    let quoted = message(&db, &format!("> ![cat|500x375](upload://{b62}.png)")).await;
    processed(&st, quoted).await;
    let html = cooked(&db, quoted).await;
    assert!(html.contains("class=\"lightbox\""), "{html}");
    assert!(html.contains("data-target-width=\"800\""), "{html}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_downloaded_hotlinked_image_is_swapped_for_its_upload() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let (upload_id, url, sha1) = upload(&db, "image.gif", 100, 100).await;
    let id = message(&db, "![](http://example.com/image.gif)").await;
    sqlx::query(
        "INSERT INTO chat_message_hotlinked_media (chat_message_id, url, status, upload_id, created_at, updated_at) \
         VALUES ($1, '//example.com/image.gif', 'downloaded', $2, now(), now())",
    )
    .bind(id)
    .bind(upload_id)
    .execute(&db.pool)
    .await
    .unwrap();
    processed(&st, id).await;
    let html = cooked(&db, id).await;
    let b62 = base62(&sha1);
    assert!(
        html.contains(&format!(
            "src=\"//test.localhost{url}\" alt=\"\" role=\"presentation\" data-base62-sha1=\"{b62}\" data-dominant-color=\"aabbcc\""
        )),
        "{html}"
    );
    assert!(html.contains("class=\"lightbox\""), "{html}");
    // Localized: nothing left to pull.
    assert!(jobs(&db, PULL).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hotlinked_image_too_large_keeps_its_src() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let id = message(&db, "![](http://example.com/image.gif)").await;
    sqlx::query(
        "INSERT INTO chat_message_hotlinked_media (chat_message_id, url, status, created_at, updated_at) \
         VALUES ($1, '//example.com/image.gif', 'too_large', now(), now())",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .unwrap();
    processed(&st, id).await;
    assert!(
        cooked(&db, id)
            .await
            .contains("src=\"http://example.com/image.gif\"")
    );
    // A terminal failure isn't pulled again.
    assert!(jobs(&db, PULL).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn hotlinked_media_gone_from_the_message_is_removed() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let (upload_id, ..) = upload(&db, "old.png", 10, 10).await;
    let id = message(&db, "no image").await;
    sqlx::query(
        "INSERT INTO chat_message_hotlinked_media (chat_message_id, url, status, upload_id, created_at, updated_at) \
         VALUES ($1, '//example.com/old.png', 'downloaded', $2, now(), now())",
    )
    .bind(id)
    .bind(upload_id)
    .execute(&db.pool)
    .await
    .unwrap();
    processed(&st, id).await;
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat_message_hotlinked_media WHERE chat_message_id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn external_images_are_handed_to_the_pull_job() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let hotlinked = message(&db, "![](https://example.com/img.png)").await;
    processed(&st, hotlinked).await;
    assert_eq!(
        jobs(&db, PULL).await,
        vec![json!({ "chat_message_id": hotlinked })]
    );

    // Nothing external: no job.
    let plain = message(&db, "just words").await;
    processed(&st, plain).await;
    // A local upload's url that resolves to nothing is pulled; a live
    // upload's isn't.
    let orphan = message(
        &db,
        "![](http://test.localhost/uploads/default/original/1X/0123456789012345678901234567890123456789.png)",
    )
    .await;
    processed(&st, orphan).await;
    let (_, url, _) = upload(&db, "here.png", 10, 10).await;
    let local = message(&db, &format!("![]({url})")).await;
    processed(&st, local).await;
    // The pull job's own re-cook doesn't ask again.
    let again = message(&db, "![](https://example.com/again.png)").await;
    process(
        &st,
        json!({ "chat_message_id": again, "skip_pull_hotlinked_images": true }),
    )
    .await
    .unwrap();
    assert_eq!(
        jobs(&db, PULL).await,
        vec![
            json!({ "chat_message_id": hotlinked }),
            json!({ "chat_message_id": orphan })
        ]
    );

    // Not while downloads or chat uploads are off.
    set(&st, "download_remote_images_to_local", "f").await;
    processed(&st, hotlinked).await;
    set(&st, "download_remote_images_to_local", "t").await;
    set(&st, "chat_allow_uploads", "f").await;
    processed(&st, hotlinked).await;
    assert_eq!(jobs(&db, PULL).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pull_job_reprocesses_what_it_localized_and_refuses_downloads() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let (upload_id, ..) = upload(&db, "img.png", 10, 10).await;
    let id = message(&db, "![](https://example.com/img.png)").await;
    sqlx::query(
        "UPDATE chat_messages SET cooked = '<p><img src=\"https://example.com/img.png\" alt=\"\"></p>' WHERE id = $1",
    )
    .bind(id)
    .execute(&db.pool)
    .await
    .unwrap();
    // A downloaded row, its re-cook lost: processed again, quietly.
    sqlx::query(
        "INSERT INTO chat_message_hotlinked_media (chat_message_id, url, status, upload_id, created_at, updated_at) \
         VALUES ($1, '//example.com/img.png', 'downloaded', $2, now(), now())",
    )
    .bind(id)
    .bind(upload_id)
    .execute(&db.pool)
    .await
    .unwrap();
    discourse_rs::plugins::chat::processor::pull_hotlinked_images(
        &st,
        &json!({ "chat_message_id": id }),
    )
    .await
    .unwrap();
    assert_eq!(
        jobs(&db, "Jobs::Chat::ProcessMessage").await,
        vec![json!({
            "chat_message_id": id,
            "skip_pull_hotlinked_images": true,
            "skip_notifications": true,
        })]
    );

    // An image not tracked yet would be downloaded.
    sqlx::query("DELETE FROM chat_message_hotlinked_media WHERE chat_message_id = $1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    set(&st, "download_remote_images_threshold", "0").await;
    let refused = discourse_rs::plugins::chat::processor::pull_hotlinked_images(
        &st,
        &json!({ "chat_message_id": id }),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{refused:?}").contains("HotlinkedMedia.download"),
        "{refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_optimized_video_replaces_the_uploaded_one() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let (video_id, video_url, sha1) = upload(&db, "clip.mov", 0, 0).await;
    let (optimized_id, optimized_url, _) = upload(&db, "clip.mp4", 0, 0).await;
    sqlx::query(
        "INSERT INTO optimized_videos (upload_id, optimized_upload_id, adapter, created_at, updated_at) \
         VALUES ($1, $2, 'aws_mediaconvert', now(), now())",
    )
    .bind(video_id)
    .bind(optimized_id)
    .execute(&db.pool)
    .await
    .unwrap();
    let id = message(
        &db,
        &format!("![clip|video](upload://{}.mov)", base62(&sha1)),
    )
    .await;
    processed(&st, id).await;
    let html = cooked(&db, id).await;
    assert!(
        html.contains(&format!(
            "data-video-src=\"{optimized_url}\" data-original-video-src=\"{video_url}\""
        )) || html.contains(&format!("data-original-video-src=\"{video_url}\""))
            && html.contains(&format!("data-video-src=\"{optimized_url}\"")),
        "{html}"
    );
    // UploadReference.ensure_exist!(target: the message): under its class
    // name.
    let refs: Vec<(String, i64)> = sqlx::query_as(
        "SELECT target_type, upload_id::int8 FROM upload_references WHERE target_id = $1 \
         AND target_type = 'Chat::Message' ORDER BY upload_id",
    )
    .bind(id)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        refs,
        vec![
            ("Chat::Message".to_string(), i64::from(video_id)),
            ("Chat::Message".to_string(), i64::from(optimized_id))
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skip_notifications_skips_the_mention_jobs() {
    let db = TestDb::new().await;
    let st = site(&db).await;
    let id = message(&db, "Hey @user2!").await;
    process(
        &st,
        json!({ "chat_message_id": id, "skip_notifications": true }),
    )
    .await
    .unwrap();
    assert!(jobs(&db, "Jobs::Chat::NotifyMentioned").await.is_empty());
    assert!(jobs(&db, "Jobs::Chat::NotifyWatching").await.is_empty());
    processed(&st, id).await;
    assert!(!jobs(&db, "Jobs::Chat::NotifyMentioned").await.is_empty());
}
