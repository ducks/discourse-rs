//! The `cooked` column of a post: `Post#cook` (PrettyText.cook with the
//! post's own options), then what `Jobs::ProcessPost` writes after
//! `CookedPostProcessor#post_process` (lib/cooked_post_processor.rb).
//!
//! Only the steps that change the html and need nothing outside the
//! database are ported: quotes marked as missing or modified, local urls
//! made absolute (`optimize_urls`), the `u` parameter taken off links to
//! the site (`remove_user_ids`), and `enforce_nofollow`. The processor
//! also writes (the post's and topic's image, badges, links to uploads,
//! thumbnails) and those are not part of the column.
//!
//! Refused: oneboxes (fetched from the network), images other than emoji
//! (sized from the upload or fetched, optimized, given lightboxes), video
//! placeholders of uploads (optimized videos), posts cooked from email,
//! secure uploads.

use sqlx::PgConnection;

use markup5ever_rcdom::{Handle, RcDom};

use super::cleanup::{
    add_rel_attributes, all_elements, append, attr, dom_text, element_name, has_class, new_element,
    new_text, parse, rel_settings, replace, set_attr, text, to_html, uri_host,
};
use super::{CookError, Host, MarkdownOptions, cook};
use crate::Unsupported;
use crate::config::Config;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

/// `Post.cook_methods`
const COOK_REGULAR: i32 = 1;
const COOK_RAW_HTML: i32 = 2;

/// `QuoteComparer.whitespace`
fn without_whitespace(s: &str) -> String {
    s.chars().filter(|c| !" \t\r\n".contains(*c)).collect()
}

/// `post_process_quotes`: a quote of a post that is gone, or whose text
/// that post no longer has, is marked.
async fn mark_quotes(
    conn: &mut PgConnection,
    html: &str,
) -> Result<Vec<(usize, &'static str)>, CookError> {
    // Collected first: the document cannot be held across a query.
    let quotes: Vec<(usize, i32, i32, String)> = {
        let dom = parse(html);
        all_elements(&dom)
            .iter()
            .filter(|e| element_name(e) == Some("aside") && has_class(e, "quote"))
            .enumerate()
            .filter_map(|(i, q)| {
                let topic = attr(q, "data-topic")?;
                let post = attr(q, "data-post")?;
                // `css("blockquote").text`: every blockquote inside.
                let mut quoted = String::new();
                fn blockquotes(node: &markup5ever_rcdom::Handle, out: &mut String) {
                    for child in node.children.borrow().iter() {
                        if element_name(child) == Some("blockquote") {
                            out.push_str(&text(child));
                        }
                        blockquotes(child, out);
                    }
                }
                blockquotes(q, &mut quoted);
                Some((
                    i,
                    crate::ruby::to_i(&topic) as i32,
                    crate::ruby::to_i(&post) as i32,
                    quoted,
                ))
            })
            .collect()
    };
    let mut marks = Vec::new();
    for (index, topic_id, post_number, quoted) in quotes {
        let parent: Option<String> = sqlx::query_scalar(
            "SELECT cooked FROM posts WHERE topic_id = $1 AND post_number = $2 \
             AND deleted_at IS NULL ORDER BY id LIMIT 1",
        )
        .bind(topic_id)
        .bind(post_number)
        .fetch_optional(&mut *conn)
        .await?;
        match parent {
            None => marks.push((index, "quote-post-not-found")),
            Some(cooked) => {
                let quoted = without_whitespace(&quoted);
                let parent_text = without_whitespace(&dom_text(&parse(&cooked)));
                if quoted.is_empty() || !parent_text.contains(&quoted) {
                    marks.push((index, "quote-modified"));
                }
            }
        }
    }
    Ok(marks)
}

/// `UrlHelper.cook_url` without a CDN or secure uploads: a local url made
/// absolute and schemaless, any other left alone.
pub(crate) fn cook_url(
    url: &str,
    urls: &Urls<'_>,
    base_path: &str,
    store: &crate::file_store::FileStore,
) -> Result<String, CookError> {
    let base_no_prefix = urls.base_url_no_prefix()?;
    let uploaded = store.has_been_uploaded(url);
    let assets = ["assets", "plugins", "images"]
        .iter()
        .any(|dir| url.starts_with(&format!("{base_path}/{dir}/")));
    let local = !url.is_empty() && (uploaded || assets || url.starts_with(&base_no_prefix));
    if !local {
        return Ok(url.to_string());
    }
    // absolute_without_cdn, then schemaless.
    let absolute = if url.starts_with('/') && !url.starts_with("//") && url.len() > 1 {
        format!("{base_no_prefix}{url}")
    } else {
        url.to_string()
    };
    Ok(match absolute.get(..5) {
        Some(head) if head.eq_ignore_ascii_case("http:") => absolute[5..].to_string(),
        _ => absolute,
    })
}

/// `Rack::Utils.escape`
fn rack_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b' ' => out.push('+'),
            b if b.is_ascii_alphanumeric() || b"*-._".contains(&b) => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `remove_user_ids` for one href: the `u` parameter taken off a link to
/// the site, the query rebuilt as Rack does. None leaves the href alone.
fn without_user_id(href: &str, hostname: &str) -> Result<Option<String>, CookError> {
    let Ok(Some(host)) = uri_host(href) else {
        return Ok(None);
    };
    if host != hostname {
        return Ok(None);
    }
    let (before_fragment, fragment) = match href.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (href, None),
    };
    let Some((base, query)) = before_fragment.split_once('?') else {
        return Ok(None);
    };
    let pairs: Vec<(String, String)> = query
        .split(['&', ';'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let decode = |s: &str| {
                mdurl::urlencode::decode(&s.replace('+', " "), mdurl::urlencode::AsciiSet::new())
                    .into_owned()
            };
            (decode(k), decode(v))
        })
        .collect();
    if !pairs.iter().any(|(k, _)| k == "u") {
        return Ok(None);
    }
    if pairs.iter().any(|(k, _)| k.contains(['[', ']'])) {
        return Err(Unsupported("nested query parameters in a link with a user id").into());
    }
    // parse_nested_query keeps the last value of a repeated key.
    let mut kept: Vec<(String, String)> = Vec::new();
    for (k, v) in pairs.into_iter().filter(|(k, _)| k != "u") {
        match kept.iter_mut().find(|(key, _)| *key == k) {
            Some(entry) => entry.1 = v,
            None => kept.push((k, v)),
        }
    }
    let query: Vec<String> = kept
        .iter()
        .map(|(k, v)| format!("{}={}", rack_escape(k), rack_escape(v)))
        .collect();
    let mut out = base.to_string();
    if !query.is_empty() {
        out.push('?');
        out.push_str(&query.join("&"));
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    Ok(Some(out))
}

/// `CookedProcessorMixin::MIN_LIGHTBOX_WIDTH` and `MIN_LIGHTBOX_HEIGHT`
const MIN_LIGHTBOX_WIDTH: i64 = 100;
const MIN_LIGHTBOX_HEIGHT: i64 = 100;

/// A node's parent, left in place.
fn parent_of(node: &Handle) -> Option<Handle> {
    let weak = node.parent.take();
    let parent = weak.as_ref().and_then(|w| w.upgrade());
    node.parent.set(weak);
    parent
}

fn ancestors(node: &Handle) -> Vec<Handle> {
    let mut out = Vec::new();
    let mut current = parent_of(node);
    while let Some(p) = current {
        current = parent_of(&p);
        out.push(p);
    }
    out
}

/// `GIF_SOURCES_REGEXP`
fn gif_source(src: &str) -> bool {
    src.contains("giphy.com/") || src.contains("tenor.com/")
}

fn add_class(node: &Handle, class: &str) {
    if has_class(node, class) {
        return;
    }
    let classes = match attr(node, "class").filter(|c| !c.trim().is_empty()) {
        Some(c) => format!("{} {class}", c.trim()),
        None => class.to_string(),
    };
    set_attr(node, "class", &classes);
}

/// `ratio.to_s.sub(/\.0\z/, "")`
fn ratio_label(ratio: f64) -> String {
    let text = format!("{ratio:?}");
    text.strip_suffix(".0").unwrap_or(&text).to_string()
}

/// `/\Ablob(\.png)?\z/i`
fn is_blob(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "blob" || lower == "blob.png"
}

/// `ImageSizer.resize` and `.crop` with max_image_width and
/// max_image_height.
struct ImageSizer {
    max_width: f64,
    max_height: f64,
}

impl ImageSizer {
    fn resize(&self, w: i64, h: i64) -> (i64, i64) {
        let (w, h) = (w as f64, h as f64);
        if w <= self.max_width && h <= self.max_height {
            return (w.floor() as i64, h.floor() as i64);
        }
        let ratio = (self.max_width / w).min(self.max_height / h);
        ((w * ratio).floor() as i64, (h * ratio).floor() as i64)
    }

    fn crop(&self, w: i64, h: i64) -> (i64, i64) {
        let (w, h) = (w as f64, h as f64);
        if w <= self.max_width && h <= self.max_height {
            return (w.floor() as i64, h.floor() as i64);
        }
        let ratio = self.max_width / w;
        (
            self.max_width.min(w).floor() as i64,
            self.max_height.min(h * ratio).floor() as i64,
        )
    }
}

/// An image as the steps read it: its attributes, and where it sits.
struct Img {
    src: String,
    width: String,
    height: String,
    title: Option<String>,
    alt: Option<String>,
    hyperlinked: bool,
    onebox: bool,
}

impl Img {
    /// Ruby's `to_i` on the attribute.
    fn width(&self) -> i64 {
        crate::ruby::to_i(&self.width)
    }

    fn height(&self) -> i64 {
        crate::ruby::to_i(&self.height)
    }
}

/// What the steps do to an image, in their order.
enum Edit {
    Set(&'static str, String),
    AddClass(&'static str),
    Lightbox {
        href: String,
        download: String,
        title: String,
        informations: String,
    },
}

/// What the image steps read besides the document.
struct ImageSteps<'a> {
    settings: &'a SiteSettings,
    urls: Urls<'a>,
    base_path: &'a str,
    store: &'a crate::file_store::FileStore,
    pasted_image_filename: String,
    sizer: ImageSizer,
    /// responsive_post_image_sizes over 1, ascending.
    ratios: Vec<f64>,
    min_ratio_to_crop: f64,
}

impl ImageSteps<'_> {
    /// `Upload.get_from_url`, the upload row.
    async fn upload_for(
        &self,
        conn: &mut PgConnection,
        url: &str,
    ) -> Result<Option<crate::optimized_images::Upload>, CookError> {
        match crate::upload_references::get_from_url(&mut *conn, url).await? {
            Some((id, _)) => Ok(crate::optimized_images::Upload::find(&mut *conn, id).await?),
            None => Ok(None),
        }
    }

    /// `get_size(url)`: an upload's own size. Anything else would be read
    /// over HTTP (FastImage), which is refused.
    async fn get_size(&self, conn: &mut PgConnection, src: &str) -> Result<(i64, i64), CookError> {
        let mut absolute = src.to_string();
        if src.starts_with('/') && !src.starts_with("//") {
            absolute = format!("{}{src}", self.urls.base_url_no_prefix()?);
        }
        if absolute.starts_with("//") {
            absolute = format!("{}:{absolute}", self.urls.scheme()?);
        }
        if let Some(upload) = self.upload_for(&mut *conn, &absolute).await?
            && let (Some(w), Some(h)) = (upload.width, upload.height)
            && w > 0
        {
            return Ok((i64::from(w), i64::from(h)));
        }
        Err(Unsupported("sizing images from other sites (FastImage over HTTP)").into())
    }

    /// `get_size_from_attributes(img)`
    async fn size_from_attributes(
        &self,
        conn: &mut PgConnection,
        img: &Img,
    ) -> Result<Option<(i64, i64)>, CookError> {
        let (w, h) = (img.width(), img.height());
        if w > 0 && h > 0 {
            return Ok(Some((w, h)));
        }
        if w > 0 || h > 0 {
            let (ow, oh) = self.get_size(&mut *conn, &img.src).await?;
            let (ow, oh) = (ow as f64, oh as f64);
            return Ok(Some(if w > 0 {
                (w, (oh * (w as f64 / ow)).floor() as i64)
            } else {
                ((ow * (h as f64 / oh)).floor() as i64, h)
            }));
        }
        Ok(None)
    }

    /// `convert_to_link!(img)`: the edits it makes.
    async fn convert_to_link(
        &self,
        conn: &mut PgConnection,
        img: &mut Img,
    ) -> Result<Vec<Edit>, CookError> {
        let mut edits = Vec::new();
        let set = |img: &mut Img, edits: &mut Vec<Edit>, name: &'static str, value: String| {
            match name {
                "width" => img.width = value.clone(),
                "height" => img.height = value.clone(),
                "src" => img.src = value.clone(),
                _ => {}
            }
            edits.push(Edit::Set(name, value));
        };
        let (w, h) = (img.width(), img.height());
        let user = if w > 0 && h > 0 {
            Some((w, h))
        } else {
            self.size_from_attributes(&mut *conn, img).await?
        };
        // limit_size!
        let limit = match self.size_from_attributes(&mut *conn, img).await? {
            Some(size) => size,
            None => self.get_size(&mut *conn, &img.src.clone()).await?,
        };
        let (lw, lh) = self.sizer.resize(limit.0, limit.1);
        set(img, &mut edits, "width", lw.to_string());
        set(img, &mut edits, "height", lh.to_string());

        if img.src.trim().is_empty() || img.hyperlinked {
            return Ok(edits);
        }
        let upload = self.upload_for(&mut *conn, &img.src).await?;
        let (original_width, original_height) = match &upload {
            Some(u) => (
                i64::from(u.width.unwrap_or(0)),
                i64::from(u.height.unwrap_or(0)),
            ),
            None => {
                // An image from elsewhere: Rails reads its size over HTTP,
                // which past this point only decides the animated class.
                if gif_source(&img.src) {
                    return Err(Unsupported(
                        "sizing images from other sites (FastImage over HTTP)",
                    )
                    .into());
                }
                user.unwrap_or(limit)
            }
        };
        if upload.as_ref().is_some_and(|u| u.animated == Some(true)) || gif_source(&img.src) {
            edits.push(Edit::AddClass("animated"));
        }
        let generate_thumbnail = original_width as f64 > self.sizer.max_width
            || original_height as f64 > self.sizer.max_height;
        let (mut width, mut height) = match user {
            Some((uw, uh)) if uw > 0 || uh > 0 => (uw, uh),
            _ => (original_width, original_height),
        };
        let crop =
            self.min_ratio_to_crop > 0.0 && (width as f64 / height as f64) < self.min_ratio_to_crop;
        if crop {
            (width, height) = self.sizer.crop(width, height);
            set(img, &mut edits, "width", width.to_string());
            set(img, &mut edits, "height", height.to_string());
        } else {
            (width, height) = self.sizer.resize(width, height);
        }
        let Some(upload) = upload else {
            return Ok(edits);
        };
        if generate_thumbnail {
            let mut sizes = vec![(width, height)];
            for ratio in &self.ratios {
                let (rw, rh) = (
                    (width as f64 * ratio) as i64,
                    (height as f64 * ratio) as i64,
                );
                if upload.width.is_some_and(|uw| rw <= i64::from(uw)) {
                    sizes.push((rw, rh));
                }
            }
            for (tw, th) in sizes {
                crate::optimized_images::create_thumbnail(
                    &mut *conn,
                    self.settings,
                    self.store,
                    &upload,
                    tw as i32,
                    th as i32,
                    crop,
                )
                .await?;
            }
        }
        if upload.animated == Some(true) {
            return Ok(edits);
        }
        if !img.onebox {
            edits.extend(self.add_lightbox(img, original_width, original_height, &upload)?);
        }
        if generate_thumbnail {
            let more = self.optimize_image(&mut *conn, img, &upload, crop).await?;
            for edit in more {
                if let Edit::Set(name, value) = edit {
                    set(img, &mut edits, name, value);
                }
            }
        }
        Ok(edits)
    }

    /// `add_lightbox!`
    fn add_lightbox(
        &self,
        img: &Img,
        original_width: i64,
        original_height: i64,
        upload: &crate::optimized_images::Upload,
    ) -> Result<Option<Edit>, CookError> {
        if original_width < MIN_LIGHTBOX_WIDTH || original_height < MIN_LIGHTBOX_HEIGHT {
            return Ok(None);
        }
        // get_filename: a pasted image is named for what it is.
        let filename = if is_blob(&upload.original_filename) {
            self.pasted_image_filename.clone()
        } else {
            upload.original_filename.clone()
        };
        Ok(Some(Edit::Lightbox {
            href: cook_url(&img.src, &self.urls, self.base_path, self.store)?,
            download: self
                .store
                .download_url(upload.sha1.as_deref().unwrap_or_default()),
            title: img
                .title
                .clone()
                .or_else(|| img.alt.clone())
                .unwrap_or(filename),
            informations: format!(
                "{original_width}×{original_height} {}",
                crate::uploads::human_size(upload.filesize)
            ),
        }))
    }

    /// `optimize_image!`: the attributes it sets.
    async fn optimize_image(
        &self,
        conn: &mut PgConnection,
        img: &Img,
        upload: &crate::optimized_images::Upload,
        cropped: bool,
    ) -> Result<Vec<Edit>, CookError> {
        let mut edits = Vec::new();
        let (w, h) = (img.width(), img.height());
        let thumbnail =
            crate::optimized_images::thumbnail(&mut *conn, upload.id, w as i32, h as i32).await?;
        match thumbnail.filter(|t| t.filesize.map_or(i64::MAX, i64::from) < upload.filesize) {
            Some(t) => {
                edits.push(Edit::Set("src", t.url.clone()));
                if !img.onebox {
                    let mut srcset = String::new();
                    for ratio in &self.ratios {
                        let (rw, rh) = ((w as f64 * ratio) as i64, (h as f64 * ratio) as i64);
                        let label = ratio_label(*ratio);
                        if !cropped && upload.width.is_some_and(|uw| rw > i64::from(uw)) {
                            let url =
                                cook_url(&upload.url, &self.urls, self.base_path, self.store)?;
                            srcset.push_str(&format!(", {url} {label}x"));
                        } else if let Some(r) = crate::optimized_images::thumbnail(
                            &mut *conn, upload.id, rw as i32, rh as i32,
                        )
                        .await?
                        {
                            let url = cook_url(&r.url, &self.urls, self.base_path, self.store)?;
                            srcset.push_str(&format!(", {url} {label}x"));
                        }
                    }
                    if !srcset.is_empty() {
                        let first = cook_url(&t.url, &self.urls, self.base_path, self.store)?;
                        edits.push(Edit::Set("srcset", format!("{first}{srcset}")));
                    }
                }
            }
            None => edits.push(Edit::Set("src", upload.url.clone())),
        }
        if let Some(color) = upload.dominant_color.as_deref().filter(|c| !c.is_empty()) {
            edits.push(Edit::Set("data-dominant-color", color.to_string()));
        }
        Ok(edits)
    }
}

/// `extract_images`: every image with a src (or a blocked hotlinked one)
/// but data urls and emoji, in document order.
fn extract_images(dom: &RcDom) -> Vec<Handle> {
    all_elements(dom)
        .into_iter()
        .filter(|e| element_name(e) == Some("img"))
        .filter(|e| attr(e, "src").is_some() || attr(e, "data-blocked-hotlinked-src").is_some())
        .filter(|e| !attr(e, "src").is_some_and(|s| s.starts_with("data")))
        .filter(|e| !has_class(e, "emoji"))
        .collect()
}

/// Wraps the image as add_lightbox! does: div.lightbox-wrapper >
/// a.lightbox > the image and its div.meta.
fn wrap_in_lightbox(img: &Handle, href: &str, download: &str, title: &str, informations: &str) {
    let wrapper = new_element("div", &[("class", "lightbox-wrapper")]);
    let link = new_element(
        "a",
        &[
            ("class", "lightbox"),
            ("href", href),
            ("data-download-href", download),
            ("title", title),
        ],
    );
    let meta = new_element("div", &[("class", "meta")]);
    let icon = |name: &str| {
        let svg = new_element(
            "svg",
            &[
                ("class", &format!("fa d-icon d-icon-{name} svg-icon")),
                ("aria-hidden", "true"),
            ],
        );
        append(&svg, new_element("use", &[("href", &format!("#{name}"))]));
        svg
    };
    let span = |class: &str, content: &str| {
        let span = new_element("span", &[("class", class)]);
        append(&span, new_text(content));
        span
    };
    append(&meta, icon("far-image"));
    append(&meta, span("filename", title));
    append(&meta, span("informations", informations));
    append(&meta, icon("discourse-expand"));
    replace(img, wrapper.clone());
    append(&wrapper, link.clone());
    append(&link, img.clone());
    append(&link, meta);
}

/// `post_process_images`: every image sized, thumbnailed and given a
/// lightbox: the edits for each image, decided from the document as
/// read (the decisions reach the database, so no DOM is held across an
/// await) and applied by apply_image_edits to the document the later
/// steps work on. Hotlinked media (pulled to uploads) are not ported.
async fn post_process_images(
    conn: &mut PgConnection,
    steps: &ImageSteps<'_>,
    html: &str,
    post_id: Option<i32>,
) -> Result<Vec<Vec<Edit>>, CookError> {
    let mut images: Vec<Img> = {
        let dom = parse(html);
        let mut out = Vec::new();
        for e in extract_images(&dom) {
            let Some(src) = attr(&e, "src") else {
                return Err(Unsupported("blocked hotlinked images").into());
            };
            out.push(Img {
                src,
                width: attr(&e, "width").unwrap_or_default(),
                height: attr(&e, "height").unwrap_or_default(),
                title: attr(&e, "title"),
                alt: attr(&e, "alt"),
                hyperlinked: ancestors(&e).iter().any(|a| element_name(a) == Some("a")),
                onebox: has_class(&e, "onebox")
                    || ancestors(&e)
                        .iter()
                        .any(|a| has_class(a, "onebox") || has_class(a, "onebox-body")),
            });
        }
        out
    };
    if images.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(id) = post_id {
        let hotlinked: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM post_hotlinked_media WHERE post_id = $1")
                .bind(id)
                .fetch_one(&mut *conn)
                .await?;
        if hotlinked > 0 {
            return Err(Unsupported("hotlinked media pulled to uploads").into());
        }
    }
    let mut edits = Vec::with_capacity(images.len());
    for img in images.iter_mut() {
        edits.push(steps.convert_to_link(&mut *conn, img).await?);
    }
    Ok(edits)
}

/// The image steps' edits applied to the document, image by image as
/// `extract_images` lists them.
fn apply_image_edits(dom: &RcDom, edits: Vec<Vec<Edit>>) {
    for (img, edits) in extract_images(dom).iter().zip(edits) {
        for edit in edits {
            match edit {
                Edit::Set(name, value) => set_attr(img, name, &value),
                Edit::AddClass(class) => add_class(img, class),
                Edit::Lightbox {
                    href,
                    download,
                    title,
                    informations,
                } => wrap_in_lightbox(img, &href, &download, &title, &informations),
            }
        }
    }
}

/// `post_process` on `Post#cook`'s html: what the job writes.
pub(crate) async fn post_process(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    config: &Config,
    i18n: &crate::i18n::I18n,
    html: &str,
    omit_nofollow: bool,
    post_id: Option<i32>,
) -> Result<String, CookError> {
    if settings.get("secure_uploads")?.truthy() {
        return Err(Unsupported("secure uploads in the post processor").into());
    }
    if config.globals.cdn_url().is_some() || config.globals.s3_cdn_url().is_some() {
        return Err(Unsupported("a CDN in the post processor").into());
    }
    let store = crate::file_store::FileStore::for_site(config, settings)?;
    {
        let dom = parse(html);
        for e in all_elements(&dom) {
            if has_class(&e, "onebox") || has_class(&e, "inline-onebox-loading") {
                return Err(Unsupported("oneboxes (fetched from the network)").into());
            }
            if has_class(&e, "video-placeholder-container")
                && attr(&e, "data-video-src").is_some_and(|s| s != "/404")
            {
                return Err(Unsupported("videos in the post processor (optimized videos)").into());
            }
        }
    }
    let base_path = config.globals.relative_url_root();
    let steps = ImageSteps {
        settings,
        urls: Urls { config, settings },
        base_path,
        store: &store,
        pasted_image_filename: i18n
            .t("upload.pasted_image_filename")
            .unwrap_or("Pasted image")
            .to_string(),
        sizer: ImageSizer {
            max_width: settings.get("max_image_width")?.to_f(),
            max_height: settings.get("max_image_height")?.to_f(),
        },
        ratios: {
            let mut ratios: Vec<f64> = settings
                .get("responsive_post_image_sizes")?
                .to_s()
                .split('|')
                .map(|r| r.trim().parse::<f64>().unwrap_or(0.0))
                .collect();
            ratios.sort_by(|a, b| a.total_cmp(b));
            ratios.into_iter().filter(|r| *r > 1.0).collect()
        },
        min_ratio_to_crop: settings.get("min_ratio_to_crop")?.to_f(),
    };
    let image_edits = post_process_images(conn, &steps, html, post_id).await?;
    let marks = mark_quotes(conn, html).await?;

    let urls = Urls { config, settings };
    let hostname = urls.current_hostname()?;
    let add_nofollow = !omit_nofollow && settings.get("add_rel_nofollow_to_user_content")?.truthy();
    let (site_host, allowlist) = rel_settings(settings, config)?;

    let dom = parse(html);
    apply_image_edits(&dom, image_edits);
    let quotes: Vec<_> = all_elements(&dom)
        .into_iter()
        .filter(|e| element_name(e) == Some("aside") && has_class(e, "quote"))
        .collect();
    for (index, mark) in marks {
        if let Some(q) = quotes.get(index) {
            let class = format!("{} {mark}", attr(q, "class").unwrap_or_default());
            set_attr(q, "class", class.trim());
        }
    }
    for e in all_elements(&dom) {
        match element_name(&e) {
            Some("a") => {
                for name in ["href", "data-download-href"] {
                    if let Some(value) = attr(&e, name) {
                        set_attr(&e, name, &cook_url(&value, &urls, base_path, &store)?);
                    }
                }
            }
            Some("img") | Some("video") => {
                if let Some(src) = attr(&e, "src") {
                    set_attr(&e, "src", &cook_url(&src, &urls, base_path, &store)?);
                }
            }
            _ => {}
        }
    }
    for a in all_elements(&dom) {
        if element_name(&a) != Some("a") {
            continue;
        }
        if let Some(href) = attr(&a, "href")
            && let Some(cleaned) = without_user_id(&href, &hostname)?
        {
            set_attr(&a, "href", &cleaned);
        }
    }
    add_rel_attributes(&dom, add_nofollow, &site_host, &allowlist);
    Ok(to_html(&dom))
}

/// The post as cooking reads it.
#[derive(sqlx::FromRow)]
struct PostRow {
    raw: String,
    topic_id: i32,
    last_editor_id: Option<i32>,
    cook_method: i32,
    /// The topic exists and is not deleted.
    topic_present: bool,
    /// `Post#add_nofollow?` turned around.
    omit_nofollow: bool,
}

/// The `cooked` column for a post, as creating it (or rebaking it) and the
/// post processor leave it.
pub async fn cooked_column(host: &Host, post_id: i64) -> Result<String, CookError> {
    let mut conn = host.pool.acquire().await?;
    let settings =
        SiteSettings::load(&mut conn, &host.site_setting_defs, &host.config.globals).await?;
    let tl3_no_follow = settings.get("tl3_links_no_follow")?.truthy();
    // add_nofollow?: false for staff, else true without a user, with
    // tl3_links_no_follow, or below trust level 3 (staged counts as 3).
    let post: Option<PostRow> = sqlx::query_as(
        "SELECT p.raw, p.topic_id, p.last_editor_id, p.cook_method, \
                (t.id IS NOT NULL) AS topic_present, \
                COALESCE(u.admin OR u.moderator \
                    OR (NOT $2 AND (u.staged OR u.trust_level >= 3)), FALSE) AS omit_nofollow \
         FROM posts p \
         LEFT JOIN topics t ON t.id = p.topic_id AND t.deleted_at IS NULL \
         LEFT JOIN users u ON u.id = p.user_id \
         WHERE p.id = $1",
    )
    .bind(post_id)
    .bind(tl3_no_follow)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(post) = post else {
        return Err(CookError::Db(sqlx::Error::RowNotFound));
    };
    match post.cook_method {
        COOK_RAW_HTML => return Ok(post.raw),
        COOK_REGULAR => {}
        _ => return Err(Unsupported("posts cooked from email").into()),
    }
    drop(conn);

    let opts = MarkdownOptions {
        topic_id: Some(i64::from(post.topic_id)),
        post_id: Some(post_id),
        user_id: post.last_editor_id.map(i64::from),
        force_quote_link: false,
        omit_nofollow: post.omit_nofollow,
    };
    let cooked = cook(host, &post.raw, &opts).await?;
    // Jobs::ProcessPost does nothing for a post whose topic is gone.
    if !post.topic_present {
        return Ok(cooked);
    }
    let mut conn = host.pool.acquire().await?;
    post_process(
        &mut conn,
        &settings,
        &host.config,
        &host.i18n,
        &cooked,
        post.omit_nofollow,
        i32::try_from(post_id).ok(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_come_off_links_to_the_site() {
        let strip = |href: &str| without_user_id(href, "localhost").unwrap();
        assert_eq!(
            strip("http://localhost:3000/t/x/1?u=bob"),
            Some("http://localhost:3000/t/x/1".to_string())
        );
        assert_eq!(
            strip("http://localhost/t/x/1?a=1&u=bob#p"),
            Some("http://localhost/t/x/1?a=1#p".to_string())
        );
        assert_eq!(strip("https://example.com/?u=bob"), None);
        assert_eq!(strip("/t/x/1?u=bob"), None);
    }
}
