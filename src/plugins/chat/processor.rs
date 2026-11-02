//! Chat::MessageProcessor (lib/chat/message_processor.rb), which
//! Jobs::Chat::ProcessMessage runs: the message's raw cooked again, then
//! CookedProcessorMixin's oneboxes (block and inline, through
//! crate::onebox::post_process as posts have them), the images its
//! chat_message_hotlinked_media localized, the thumbnails of its large
//! image uploads, its videos' optimized versions, and the lightbox data
//! on its uploaded images. Also Jobs::Chat::PullHotlinkedImages, which
//! the processor hands the images it could localize; downloading them is
//! refused as it is for posts.

use std::collections::HashMap;

use markup5ever_rcdom::{Handle, NodeData};
use serde_json::{Value, json};
use sqlx::PgConnection;

use crate::hotlinked_media::{BLOCKED_HOTLINKED_SRC_ATTR, download_src_for, normalize_src};
use crate::pretty_text::cleanup::{all_elements, attr, element_name, has_class, parse, set_attr};
use crate::pretty_text::cooked_post_processor::ancestors;
use crate::site_settings::SiteSettings;
use crate::url::Urls;
use crate::{AppError, AppState, Unsupported};

/// The job that downloads a message's hotlinked images.
pub const PULL_HOTLINKED_IMAGES_JOB: &str = "Jobs::Chat::PullHotlinkedImages";

/// `PrettyText::BLOCKED_HOTLINKED_SRCSET_ATTR`
const BLOCKED_HOTLINKED_SRCSET_ATTR: &str = "data-blocked-hotlinked-srcset";

/// `Chat::MessageProcessor::IMG_FILETYPES`
const IMG_FILETYPES: [&str; 7] = ["jpg", "jpeg", "gif", "png", "heic", "heif", "webp"];

/// The message as the processor reads it.
pub struct Message {
    pub id: i64,
    pub user_id: i32,
    pub last_editor_id: i32,
    pub username: String,
    pub message: String,
}

/// What `run!` leaves: the processed cooked (`html`), the tracked
/// hotlinked media it didn't use, and the srcs of the images that may
/// need downloading (`hotlinked_media_pending?`'s candidates, not yet
/// checked).
pub struct Processed {
    pub html: String,
    pub stale_hotlinked_media_ids: Vec<i64>,
    pending_srcs: Vec<String>,
    tracked_urls: Vec<String>,
}

/// A chat_message_hotlinked_media row and its upload.
#[derive(sqlx::FromRow)]
struct Hotlinked {
    id: i64,
    url: String,
    status: String,
    upload_id: Option<i64>,
    upload_url: Option<String>,
    sha1: Option<String>,
    dominant_color: Option<String>,
    secure: Option<bool>,
}

/// One of the message's uploads.
#[derive(sqlx::FromRow)]
struct MessageUpload {
    #[sqlx(flatten)]
    upload: crate::optimized_images::Upload,
    thumbnail_width: Option<i32>,
    thumbnail_height: Option<i32>,
    optimized_upload_id: Option<i32>,
}

/// An upload the lightbox reads.
#[derive(sqlx::FromRow, Clone)]
struct LightboxUpload {
    url: String,
    sha1: Option<String>,
    extension: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
    secure: bool,
}

fn remove_attr(node: &Handle, name: &str) {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs.borrow_mut().retain(|a| &*a.name.local != name);
    }
}

fn cook_error(e: crate::pretty_text::CookError) -> AppError {
    match e {
        crate::pretty_text::CookError::App(e) => e,
        crate::pretty_text::CookError::Unsupported(u) => u.into(),
        e => AppError::from(std::io::Error::other(e.to_string())),
    }
}

/// `extract_images`: images with a src (or a blocked hotlinked one) but
/// data urls and emoji.
fn extract_images(dom: &markup5ever_rcdom::RcDom) -> Vec<Handle> {
    all_elements(dom)
        .into_iter()
        .filter(|e| {
            element_name(e) == Some("img")
                && (attr(e, "src").is_some() || attr(e, BLOCKED_HOTLINKED_SRC_ATTR).is_some())
                && !attr(e, "src").is_some_and(|s| s.starts_with("data"))
                && !has_class(e, "emoji")
        })
        .collect()
}

/// `HotlinkedMedia.extract_candidates` narrowed to the images, which is
/// all chat localizes: `img[src]` or one with a blocked hotlinked src,
/// less avatars and the images in a lightbox.
fn image_candidates(dom: &markup5ever_rcdom::RcDom) -> Vec<String> {
    all_elements(dom)
        .into_iter()
        .filter(|e| element_name(e) == Some("img"))
        .filter(|e| attr(e, "src").is_some() || attr(e, BLOCKED_HOTLINKED_SRC_ATTR).is_some())
        .filter(|e| !has_class(e, "avatar"))
        .filter(|e| {
            attr(e, "src").is_none() || !ancestors(e).iter().any(|a| has_class(a, "lightbox"))
        })
        .map(|e| {
            attr(&e, "src")
                .or_else(|| attr(&e, BLOCKED_HOTLINKED_SRC_ATTR))
                .unwrap_or_default()
        })
        .collect()
}

/// `FileHelper.is_supported_video?`
fn supported_video(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["mov", "mp4", "webm", "ogv", "m4v", "3gp", "avi", "mpeg"]
        .iter()
        .any(|e| lower.ends_with(&format!(".{e}")))
}

/// An upload's optimized video, by the upload's url
/// (`Upload.get_from_url` and `OptimizedVideo.find_by(upload_id:)`): the
/// upload's id, the optimized upload's id and url.
async fn optimized_for_url(
    conn: &mut PgConnection,
    src: &str,
) -> Result<Option<(i64, i64, String)>, AppError> {
    let Some((upload_id, _)) = crate::upload_references::get_from_url(&mut *conn, src).await?
    else {
        return Ok(None);
    };
    let optimized: Option<(i32, String)> = sqlx::query_as(
        "SELECT u.id, u.url FROM optimized_videos v JOIN uploads u ON u.id = v.optimized_upload_id \
         WHERE v.upload_id = $1 ORDER BY v.id LIMIT 1",
    )
    .bind(upload_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(optimized.map(|(id, url)| (i64::from(upload_id), i64::from(id), url)))
}

/// `Chat::MessageProcessor.new(message, invalidate_oneboxes:).run!`
pub async fn run(
    state: &AppState,
    conn: &mut PgConnection,
    settings: &SiteSettings,
    message: &Message,
    invalidate_oneboxes: bool,
) -> Result<Processed, AppError> {
    if settings.get("secure_uploads")?.truthy() {
        return Err(Unsupported("secure uploads in the chat message processor").into());
    }
    let config = &state.config;
    let store = crate::file_store::FileStore::for_site(config, settings)?;
    if config.globals.cdn_url().is_some()
        || config.globals.s3_cdn_url().is_some()
        || store.has_cdn()
    {
        return Err(Unsupported("a CDN in the chat message processor").into());
    }
    let urls = Urls { config, settings };
    let base_path = config.globals.relative_url_root();
    let host = crate::pretty_text::Host::from_state(state);
    // block_hotlinked_media (PrettyText.sanitize_hotlinked_media) is
    // refused by the cook.
    let cooked = super::cook::cook(
        &host,
        &message.message,
        message.last_editor_id,
        &message.username,
    )
    .await?;

    // post_process_oneboxes
    let html = if cooked.contains("onebox") {
        if invalidate_oneboxes {
            crate::onebox::invalidate_marked(conn, settings, &cooked).await?;
        }
        let site = crate::onebox::Site {
            settings,
            i18n: &state.i18n,
            urls: Urls { config, settings },
            base_path,
            host: None,
        };
        crate::onebox::post_process(
            conn,
            &site,
            &cooked,
            &crate::onebox::Opts {
                editor_id: Some(message.last_editor_id),
                author_id: Some(message.user_id),
                category_id: None,
                topic_id: None,
            },
        )
        .await?
    } else {
        cooked
    };
    let strip_nofollow = !settings.get("add_rel_nofollow_to_user_content")?.truthy();

    // hotlinked_media_map, read once (process_hotlinked_images snapshots
    // the tracked ids from it).
    let hotlinked: Vec<Hotlinked> = sqlx::query_as(
        "SELECT h.id, h.url, h.status, h.upload_id, u.url AS upload_url, u.sha1, u.dominant_color, \
                u.secure FROM chat_message_hotlinked_media h LEFT JOIN uploads u ON u.id = h.upload_id \
         WHERE h.chat_message_id = $1 ORDER BY h.id",
    )
    .bind(message.id)
    .fetch_all(&mut *conn)
    .await?;
    let mut map: HashMap<String, &Hotlinked> = HashMap::new();
    for h in &hotlinked {
        map.insert(h.url.clone(), h);
    }
    let force_https = settings.get("force_https")?.truthy();

    // What the steps look up: the uploads the images name by sha1, and
    // the optimized versions of the videos.
    let (base62s, video_srcs, source_srcs) = {
        let dom = parse(&html);
        let elements = all_elements(&dom);
        let base62s: Vec<String> = elements
            .iter()
            .filter(|e| element_name(e) == Some("img"))
            .filter_map(|e| attr(e, "data-base62-sha1"))
            .filter(|b| !crate::ruby::is_blank(b))
            .collect();
        let video_srcs: Vec<String> = elements
            .iter()
            .filter(|e| has_class(e, "video-placeholder-container"))
            .map(|e| attr(e, "data-video-src").unwrap_or_default())
            .collect();
        let source_srcs: Vec<String> = elements
            .iter()
            .filter(|e| {
                element_name(e) == Some("source")
                    && ancestors(e)
                        .iter()
                        .any(|a| element_name(a) == Some("video"))
            })
            .map(|e| attr(e, "src").unwrap_or_default())
            .collect();
        (base62s, video_srcs, source_srcs)
    };
    let sha1s: Vec<String> = base62s
        .iter()
        .filter_map(|b| crate::pretty_text::helpers::sha1_from_base62(b))
        .collect();
    let by_sha1: Vec<LightboxUpload> = sqlx::query_as(
        "SELECT url, sha1, extension, width, height, secure FROM uploads WHERE sha1 = ANY($1) ORDER BY id",
    )
    .bind(&sha1s)
    .fetch_all(&mut *conn)
    .await?;
    let mut uploads_by_sha1: HashMap<String, LightboxUpload> = HashMap::new();
    for u in by_sha1 {
        if let Some(sha1) = u.sha1.clone() {
            uploads_by_sha1.entry(sha1).or_insert(u);
        }
    }
    // hotlinked_uploads: the tracked rows' uploads by base62.
    let mut hotlinked_uploads: HashMap<String, LightboxUpload> = HashMap::new();
    for h in &hotlinked {
        let Some(upload_id) = h.upload_id else {
            continue;
        };
        let u: LightboxUpload = sqlx::query_as(
            "SELECT url, sha1, extension, width, height, secure FROM uploads WHERE id = $1",
        )
        .bind(upload_id)
        .fetch_one(&mut *conn)
        .await?;
        if let Some(base62) = u
            .sha1
            .as_deref()
            .and_then(crate::pretty_text::helpers::base62_sha1)
        {
            hotlinked_uploads.insert(base62, u);
        }
    }
    let mut video_edits: Vec<Option<(i64, i64, String)>> = Vec::new();
    for src in &video_srcs {
        video_edits.push(if crate::ruby::is_blank(src) {
            None
        } else {
            optimized_for_url(conn, src).await?
        });
    }
    let mut source_edits: Vec<Option<(i64, i64, String)>> = Vec::new();
    for src in &source_srcs {
        source_edits.push(if crate::ruby::is_blank(src) {
            None
        } else {
            optimized_for_url(conn, src).await?
        });
    }
    // The message's uploads, as the job preloaded them, for
    // process_thumbnails and post_process_videos.
    let message_uploads: Vec<MessageUpload> = sqlx::query_as(
        "SELECT u.id, u.url, u.sha1, u.extension, u.original_filename, u.filesize, u.width, u.height, \
                u.animated, u.dominant_color, u.thumbnail_width, u.thumbnail_height, \
                (SELECT v.optimized_upload_id FROM optimized_videos v WHERE v.upload_id = u.id \
                 ORDER BY v.id LIMIT 1) AS optimized_upload_id \
         FROM upload_references ur JOIN uploads u ON u.id = ur.upload_id \
         WHERE ur.target_type = 'ChatMessage' AND ur.target_id = $1 ORDER BY ur.id",
    )
    .bind(message.id)
    .fetch_all(&mut *conn)
    .await?;

    // The steps on the document.
    let cook_url = |url: &str| -> Result<String, AppError> {
        crate::pretty_text::cooked_post_processor::cook_url(url, &urls, base_path, &store)
            .map_err(cook_error)
    };
    let mut used: Vec<i64> = Vec::new();
    let mut references: Vec<Vec<i64>> = Vec::new();
    let (html, pending_srcs) = {
        let dom = parse(&html);
        if strip_nofollow {
            for a in all_elements(&dom) {
                if element_name(&a) != Some("a")
                    || !ancestors(&a)
                        .iter()
                        .any(|p| has_class(p, "onebox-body") || has_class(p, "onebox"))
                {
                    continue;
                }
                let Some(rel) = attr(&a, "rel") else { continue };
                let values: Vec<String> = rel
                    .split(' ')
                    .map(str::to_lowercase)
                    .filter(|v| v != "nofollow" && v != "ugc")
                    .collect();
                if values.iter().all(|v| v.is_empty()) {
                    remove_attr(&a, "rel");
                } else {
                    set_attr(&a, "rel", &values.join(" "));
                }
            }
        }
        // process_hotlinked_images: chat keeps the hotlinked src where the
        // download failed or was too large.
        for img in extract_images(&dom) {
            let original = attr(&img, "src")
                .or_else(|| attr(&img, BLOCKED_HOTLINKED_SRC_ATTR))
                .unwrap_or_default();
            let download_src = download_src_for(&original, force_https)?;
            let Some(info) = map.get(&normalize_src(&download_src, true)?) else {
                continue;
            };
            used.push(info.id);
            let (Some(upload_url), "downloaded") = (&info.upload_url, info.status.as_str()) else {
                continue;
            };
            if info.secure == Some(true) {
                return Err(Unsupported("secure hotlinked uploads in chat messages").into());
            }
            set_attr(&img, "src", &cook_url(upload_url)?);
            let base62 = info
                .sha1
                .as_deref()
                .and_then(crate::pretty_text::helpers::base62_sha1)
                .unwrap_or_default();
            set_attr(&img, "data-base62-sha1", &base62);
            // dominant_color(calculate_if_missing: true).presence
            match info.dominant_color.as_deref() {
                Some("") => remove_attr(&img, "data-dominant-color"),
                Some(color) => set_attr(&img, "data-dominant-color", color),
                None => {
                    return Err(Unsupported(
                        "calculating a hotlinked upload's dominant color in chat",
                    )
                    .into());
                }
            }
            remove_attr(&img, BLOCKED_HOTLINKED_SRC_ATTR);
            remove_attr(&img, BLOCKED_HOTLINKED_SRCSET_ATTR);
        }

        // post_process_videos: each container and video source whose
        // upload has an optimized video points at it, the original kept,
        // and both uploads referenced (by the class name, Chat::Message).
        let containers: Vec<Handle> = all_elements(&dom)
            .into_iter()
            .filter(|e| has_class(e, "video-placeholder-container"))
            .collect();
        for (container, edit) in containers.iter().zip(&video_edits) {
            let Some((upload_id, optimized_id, url)) = edit else {
                continue;
            };
            let src = attr(container, "data-video-src").unwrap_or_default();
            if &src != url {
                if attr(container, "data-original-video-src").is_none() {
                    set_attr(container, "data-original-video-src", &src);
                }
                set_attr(container, "data-video-src", url);
            }
            references.push(vec![*upload_id, *optimized_id]);
        }
        let sources: Vec<Handle> = all_elements(&dom)
            .into_iter()
            .filter(|e| {
                element_name(e) == Some("source")
                    && ancestors(e)
                        .iter()
                        .any(|a| element_name(a) == Some("video"))
            })
            .collect();
        for (source, edit) in sources.iter().zip(&source_edits) {
            let Some((upload_id, optimized_id, url)) = edit else {
                continue;
            };
            let src = attr(source, "src").unwrap_or_default();
            if &src != url {
                if attr(source, "data-original-src").is_none() {
                    set_attr(source, "data-original-src", &src);
                }
                set_attr(source, "src", url);
            }
            references.push(vec![*upload_id, *optimized_id]);
        }
        for u in &message_uploads {
            if let Some(optimized) = u.optimized_upload_id
                && supported_video(&u.upload.original_filename)
            {
                references.push(vec![i64::from(optimized)]);
            }
        }

        // add_lightbox_to_images: uploaded images outside oneboxes.
        for img in all_elements(&dom) {
            if element_name(&img) != Some("img") {
                continue;
            }
            let class = attr(&img, "class").unwrap_or_default();
            let Some(base62) = attr(&img, "data-base62-sha1").filter(|b| !crate::ruby::is_blank(b))
            else {
                continue;
            };
            if class.contains("emoji")
                || class.contains("avatar")
                || has_class(&img, "onebox")
                || ancestors(&img)
                    .iter()
                    .any(|a| has_class(a, "onebox") || has_class(a, "onebox-body"))
            {
                continue;
            }
            let upload = match hotlinked_uploads.get(&base62) {
                Some(u) => Some(u),
                None => crate::pretty_text::helpers::sha1_from_base62(&base62)
                    .and_then(|sha1| uploads_by_sha1.get(&sha1)),
            };
            let Some(upload) = upload else { continue };
            if upload.secure {
                return Err(Unsupported("secure uploads in chat lightboxes").into());
            }
            set_attr(&img, "data-large-src", &cook_url(&upload.url)?);
            set_attr(
                &img,
                "data-download-href",
                &crate::file_store::short_path(
                    base_path,
                    upload.sha1.as_deref().unwrap_or_default(),
                    upload.extension.as_deref(),
                ),
            );
            let (Some(width), Some(height)) = (upload.width, upload.height) else {
                return Err(Unsupported("a chat lightbox of an upload without dimensions").into());
            };
            set_attr(&img, "data-target-width", &width.to_string());
            set_attr(&img, "data-target-height", &height.to_string());
            set_attr(&img, "class", format!("{class} lightbox").trim());
        }
        (
            crate::pretty_text::cleanup::to_html(&dom),
            image_candidates(&dom),
        )
    };

    process_thumbnails(conn, settings, &store, &message_uploads).await?;
    for ids in references {
        crate::upload_references::ensure_exist_many(conn, "Chat::Message", message.id, &ids)
            .await?;
    }

    let tracked_ids: Vec<i64> = hotlinked.iter().map(|h| h.id).collect();
    Ok(Processed {
        html,
        stale_hotlinked_media_ids: tracked_ids
            .into_iter()
            .filter(|id| !used.contains(id))
            .collect(),
        pending_srcs,
        tracked_urls: hotlinked.iter().map(|h| h.url.clone()).collect(),
    })
}

/// `process_thumbnails`: each image upload larger than max_image_width
/// by max_image_height gets its thumbnail and the responsive ones, until
/// the first that isn't (Rails returns there).
async fn process_thumbnails(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    store: &crate::file_store::FileStore,
    uploads: &[MessageUpload],
) -> Result<(), AppError> {
    if !settings.get("create_thumbnails")?.truthy() {
        return Ok(());
    }
    let max_width = settings.get("max_image_width")?.to_i();
    let max_height = settings.get("max_image_height")?.to_i();
    let min_ratio_to_crop = settings.get("min_ratio_to_crop")?.to_f();
    // each_responsive_ratio
    let mut ratios: Vec<f64> = settings
        .get("responsive_post_image_sizes")?
        .to_s()
        .split('|')
        .map(|r| r.trim().parse::<f64>().unwrap_or(0.0))
        .collect();
    ratios.sort_by(|a, b| a.total_cmp(b));
    for u in uploads {
        let upload = &u.upload;
        let ext = upload.extension.clone().unwrap_or_default().to_lowercase();
        if !IMG_FILETYPES.contains(&ext.as_str()) {
            continue;
        }
        let (Some(width), Some(height)) = (upload.width, upload.height) else {
            return Err(Unsupported("a chat image upload without its dimensions").into());
        };
        if i64::from(width) <= max_width && i64::from(height) <= max_height {
            return Ok(());
        }
        let (Some(tw), Some(th)) = (u.thumbnail_width, u.thumbnail_height) else {
            return Err(Unsupported("a chat image upload without its thumbnail size").into());
        };
        let crop =
            min_ratio_to_crop > 0.0 && f64::from(width) / f64::from(height) < min_ratio_to_crop;
        crate::optimized_images::create_thumbnail(conn, settings, store, upload, tw, th, crop)
            .await?;
        for ratio in ratios.iter().filter(|r| **r > 1.0) {
            let (rw, rh) = (
                (f64::from(tw) * ratio) as i32,
                (f64::from(th) * ratio) as i32,
            );
            if rw <= width {
                crate::optimized_images::create_thumbnail(
                    conn, settings, store, upload, rw, rh, crop,
                )
                .await?;
            }
        }
    }
    Ok(())
}

impl Processed {
    /// `hotlinked_media_pending?`: an image the pull job would download
    /// that no tracked row covers yet.
    pub async fn hotlinked_media_pending(
        &self,
        state: &AppState,
        conn: &mut PgConnection,
        settings: &SiteSettings,
    ) -> Result<bool, AppError> {
        if self.pending_srcs.is_empty() {
            return Ok(false);
        }
        let store = crate::file_store::FileStore::for_site(&state.config, settings)?;
        let local_bases = crate::jobs::hotlinked_images::local_bases(state, settings)?;
        let force_https = settings.get("force_https")?.truthy();
        for src in &self.pending_srcs {
            let download_src = download_src_for(src, force_https)?;
            if !downloadable(conn, settings, &store, &local_bases, &download_src).await? {
                continue;
            }
            if !self
                .tracked_urls
                .contains(&normalize_src(&download_src, true)?)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// `Chat::MessageHotlinkedMedia.downloadable?(src)`: whether the pull job
/// would fetch it. Another site's image where its domain may be
/// downloaded from, or one of the site's upload urls whose upload is
/// gone.
async fn downloadable(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    store: &crate::file_store::FileStore,
    local_bases: &[String],
    src: &str,
) -> Result<bool, AppError> {
    if crate::ruby::is_blank(src)
        || !settings.get("download_remote_images_to_local")?.truthy()
        || !settings.get("chat_allow_uploads")?.truthy()
        || src.to_lowercase().starts_with("data:")
        || crate::jobs::hotlinked_images::secure_uploads_url(src)
    {
        return Ok(false);
    }
    // HotlinkedMedia.remote_src?
    let remote = !(src.starts_with('/') && !src.starts_with("//"))
        && !store.has_been_uploaded(src)
        && !local_bases
            .iter()
            .any(|b| normalize_src(src, true).is_ok_and(|n| n.starts_with(b.as_str())));
    if !remote {
        if !src.contains("/uploads/") {
            return Ok(false);
        }
        // Upload.consider_for_reuse(upload, nil) gives the upload back.
        return Ok(crate::upload_references::get_from_url(&mut *conn, src)
            .await?
            .is_none());
    }
    let Some(host) = crate::onebox::uri::RubyUri::parse(src)
        .and_then(|u| u.host)
        .filter(|h| !h.is_empty())
    else {
        return Ok(false);
    };
    // SiteSetting.should_download_images?(src)
    let disabled = settings.get("disabled_image_download_domains")?.to_s();
    Ok(crate::ruby::is_blank(&disabled) || !disabled.split('|').any(|d| d == host))
}

/// `Jobs::Chat::PullHotlinkedImages#execute`: each image of the
/// message's that is downloadable and not tracked yet downloaded into an
/// upload (refused: HotlinkedMedia.download is not ported), and the
/// message processed again (without pulling or notifying) when a tracked
/// image came back still external.
pub async fn pull_hotlinked_images(state: &AppState, args: &Value) -> Result<(), AppError> {
    let message_id = match args.get("chat_message_id") {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(v)) if !crate::ruby::is_blank(v) => Some(crate::ruby::to_i(v)),
        _ => None,
    };
    let Some(message_id) = message_id else {
        return Err(Unsupported(
            "chat pull_hotlinked_images without a chat_message_id (InvalidParameters)",
        )
        .into());
    };
    let mut tx = state.pool.begin().await?;
    let settings =
        SiteSettings::load(&mut tx, &state.site_setting_defs, &state.config.globals).await?;
    if !settings.get("chat_allow_uploads")?.truthy() {
        return Ok(());
    }
    // DistributedMutex "chat_pull_hotlinked_images_<id>"
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("chat_pull_hotlinked_images_{message_id}"))
        .execute(&mut *tx)
        .await?;
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT cooked, message FROM chat_messages WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(message_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((Some(cooked), message)) = row else {
        return Ok(());
    };
    // System and webhook messages' hand-written cooked isn't cooked again.
    if crate::ruby::is_blank(&cooked) || message.as_deref().is_none_or(crate::ruby::is_blank) {
        return Ok(());
    }
    let tracked: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT url, status, upload_id FROM chat_message_hotlinked_media WHERE chat_message_id = $1",
    )
    .bind(message_id)
    .fetch_all(&mut *tx)
    .await?;
    let store = crate::file_store::FileStore::for_site(&state.config, &settings)?;
    let local_bases = crate::jobs::hotlinked_images::local_bases(state, &settings)?;
    let force_https = settings.get("force_https")?.truthy();
    let candidates = image_candidates(&parse(&cooked));
    let mut needs_recook = false;
    let mut enough_disk_space: Option<bool> = None;
    for src in candidates {
        let download_src = download_src_for(&src, force_https)?;
        if !downloadable(&mut tx, &settings, &store, &local_bases, &download_src).await? {
            continue;
        }
        let normalized = normalize_src(&download_src, true)?;
        if let Some((_, status, upload_id)) = tracked.iter().find(|(url, ..)| *url == normalized) {
            // Still external with a downloaded row: the localizing re-cook
            // was lost. Terminal failures aren't tried again.
            if status == "downloaded" && upload_id.is_some() {
                needs_recook = true;
            }
            continue;
        }
        let enough = match enough_disk_space {
            Some(enough) => enough,
            None => {
                let disabled = crate::jobs::hotlinked_images::disable_if_low_on_disk_space(
                    state, &mut tx, &settings, &store,
                )
                .await?;
                *enough_disk_space.insert(!disabled)
            }
        };
        if !enough {
            continue;
        }
        return Err(Unsupported("downloading hotlinked media (HotlinkedMedia.download)").into());
    }
    if needs_recook {
        crate::jobs::enqueue(
            &mut tx,
            super::create::PROCESS_MESSAGE_JOB,
            json!({
                "chat_message_id": message_id,
                "skip_pull_hotlinked_images": true,
                "skip_notifications": true,
            }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
