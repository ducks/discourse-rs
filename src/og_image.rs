//! TopicOgImageGenerator (lib/topic_og_image_generator.rb): a topic's
//! Open Graph card, its title, category, author, stats, logo and domain
//! drawn as an SVG and rendered to a 1200x630 PNG, kept as a
//! `topic_og_image` upload (generate_topic_og_image, crate::jobs) or
//! returned for the admin preview.
//!
//! Discourse renders the SVG with libvips or ImageMagick; here resvg
//! does, with the system's fonts for `sans-serif`, so the pixels are not
//! Discourse's own.

use std::sync::{Arc, LazyLock};

use base64::Engine;
use sqlx::PgConnection;

use crate::AppError;
use crate::site_settings::SiteSettings;
use crate::url::Urls;

const OG_WIDTH: i64 = 1200;
const OG_HEIGHT: i64 = 630;
const MAX_TITLE_LENGTH: usize = 100;
const MAX_TITLE_LINES: usize = 2;
const TITLE_LINE_CHARS: usize = 34;
const LOGO_HEIGHT: i64 = 100;
const LOGO_WIDTH: i64 = 300;
const AVATAR_SIZE: i64 = 72;
const SIDE_MARGIN: i64 = 80;
const FONT_FAMILY: &str = "sans-serif";
/// FileHelper.download's max_file_size for the logo and avatar.
const MAX_ASSET_BYTES: usize = 1024 * 1024;

/// `TopicOgImageGenerator.eligible?`: OG images go in public pages, so
/// never under login_required, for a PM or in a read-restricted category.
/// None for a topic that doesn't exist.
pub async fn eligible(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
) -> Result<Option<bool>, AppError> {
    let row: Option<(bool, Option<bool>)> = sqlx::query_as(
        "SELECT t.archetype = 'private_message', c.read_restricted \
         FROM topics t LEFT JOIN categories c ON c.id = t.category_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some((private_message, read_restricted)) = row else {
        return Ok(None);
    };
    Ok(Some(
        !settings.get("login_required")?.truthy()
            && !private_message
            && read_restricted != Some(true),
    ))
}

/// What the card shows of the topic.
#[derive(sqlx::FromRow)]
struct Topic {
    title: Option<String>,
    category_name: Option<String>,
    category_color: Option<String>,
    like_count: Option<i32>,
    posts_count: i32,
    word_count: Option<i32>,
    created_at: Option<chrono::NaiveDateTime>,
    user_id: Option<i32>,
    username: Option<String>,
    uploaded_avatar_id: Option<i32>,
}

/// The colors the card is drawn in: the default theme's palette, else the
/// base one.
struct Colors {
    primary: String,
    secondary: String,
    tertiary: String,
}

/// An image the card embeds: its data URI, and for the logo its upload's
/// size.
struct Asset {
    data_uri: String,
}

/// What generating needs of the site.
pub struct Cx<'a> {
    pub config: &'a crate::config::Config,
    pub settings: &'a SiteSettings,
    pub i18n: &'a crate::i18n::I18n,
}

impl Cx<'_> {
    fn urls(&self) -> Urls<'_> {
        Urls {
            config: self.config,
            settings: self.settings,
        }
    }
}

/// `generate_bytes`: the card as PNG bytes, None when it couldn't be
/// rendered.
pub async fn generate_bytes(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    topic_id: i32,
) -> Result<Option<Vec<u8>>, AppError> {
    let Some(svg) = build_svg(conn, cx, topic_id).await? else {
        return Ok(None);
    };
    let font = cx.config.letter_avatar_font.clone();
    let png = tokio::task::spawn_blocking(move || render_png(&svg, &font))
        .await
        .map_err(std::io::Error::other)?;
    let Some(png) = png else {
        tracing::warn!(topic_id, "Failed to render topic OG image");
        return Ok(None);
    };
    // FileHelper.optimize_image!: kept only when smaller.
    Ok(Some(
        crate::images::optimize(&png, crate::images::Format::Png, false).unwrap_or(png),
    ))
}

/// `generate`: the card made a `topic_og_image` upload by the system
/// user, skipping validations; its id, None when nothing was made.
pub async fn generate(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    topic_id: i32,
) -> Result<Option<i64>, AppError> {
    let Some(png) = generate_bytes(conn, cx, topic_id).await? else {
        return Ok(None);
    };
    let urls = cx.urls();
    let store = crate::file_store::FileStore::for_site(cx.config, cx.settings)?;
    let filename = format!("topic-og-{topic_id}.png");
    let outcome = crate::uploads::create(
        conn,
        cx.settings,
        cx.i18n,
        &urls,
        &store,
        &crate::uploads::NewUpload {
            user_id: -1,
            staff: true,
            upload_type: "topic_og_image",
            filename: &filename,
            bytes: &png,
            pasted: false,
            for_private_message: false,
            for_export: false,
            for_theme: false,
            skip_validations: true,
            origin: None,
            for_gravatar: false,
        },
    )
    .await?;
    Ok(match outcome {
        crate::uploads::Outcome::Created(upload) => upload["id"].as_i64(),
        crate::uploads::Outcome::Invalid(_) => None,
    })
}

/// `build_svg`: the card, None for a topic that is gone.
async fn build_svg(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    topic_id: i32,
) -> Result<Option<String>, AppError> {
    let topic: Option<Topic> = sqlx::query_as(
        "SELECT t.title, c.name AS category_name, c.color AS category_color, t.like_count, \
                t.posts_count, t.word_count, t.created_at, t.user_id, u.username, u.uploaded_avatar_id \
         FROM topics t LEFT JOIN categories c ON c.id = t.category_id \
         LEFT JOIN users u ON u.id = t.user_id WHERE t.id = $1",
    )
    .bind(topic_id)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(topic) = topic else {
        return Ok(None);
    };
    let settings = cx.settings;
    let urls = cx.urls();
    let title = truncated_title(topic.title.as_deref().unwrap_or(""));
    let category_name = topic.category_name.clone().unwrap_or_default();
    let category_color = topic
        .category_color
        .clone()
        .unwrap_or_else(|| "888888".to_string());
    let like_count = i64::from(topic.like_count.unwrap_or(0));
    let posts_count = (i64::from(topic.posts_count) - 1).max(0);
    let read_time = read_time(settings, &topic)?;
    let colors = colors(conn, settings).await?;

    // SiteSetting.logo.presence || SiteSetting.logo_small
    let mut logo_id = settings.get("logo")?.to_i();
    if logo_id == 0 {
        logo_id = settings.get("logo_small")?.to_i();
    }
    let logo: Option<(String, Option<i32>, Option<i32>)> = if logo_id == 0 {
        None
    } else {
        sqlx::query_as("SELECT url, width, height FROM uploads WHERE id = $1")
            .bind(i32::try_from(logo_id).unwrap_or(0))
            .fetch_optional(&mut *conn)
            .await?
    };
    let logo_asset = match &logo {
        Some((url, _, _)) => fetch_as_data_uri(conn, cx, url).await?,
        None => None,
    };

    let mut display_lines = word_wrap(&title, TITLE_LINE_CHARS);
    let truncated = display_lines.len() > MAX_TITLE_LINES;
    display_lines.truncate(MAX_TITLE_LINES);
    if truncated && let Some(last) = display_lines.last_mut() {
        *last = if last.chars().count() > TITLE_LINE_CHARS - 1 {
            format!(
                "{}…",
                last.chars().take(TITLE_LINE_CHARS - 1).collect::<String>()
            )
        } else {
            format!("{last}…")
        };
    }

    let title_start_y: i64 = 190;
    let title_end_y = title_start_y + (display_lines.len() as i64 - 1) * 58;
    let author_y = title_end_y + 55;

    // author.avatar_template_url (schemaless, absolute), at 120.
    let avatar_asset = match (topic.user_id, topic.username.as_deref()) {
        (Some(user_id), Some(username)) => {
            let logo_small = crate::admin::users::logo_small_url(conn, settings).await?;
            let template = crate::avatar::avatar_template(
                &urls,
                user_id,
                username,
                topic.uploaded_avatar_id,
                logo_small.as_deref(),
            )
            .map_err(|e| std::io::Error::other(e.to_string()))?;
            let absolute = urls.absolute(&template)?;
            let schemaless = absolute
                .strip_prefix("https:")
                .or_else(|| absolute.strip_prefix("http:"))
                .unwrap_or(&absolute)
                .to_string();
            fetch_as_data_uri(conn, cx, &schemaless.replace("{size}", "120")).await?
        }
        _ => None,
    };
    let username = topic.username.clone();
    let created_at = topic.created_at.map(|t| t.format("%b %-d, %Y").to_string());

    let logo_y = OG_HEIGHT - 60 - LOGO_HEIGHT;
    let stats_y = OG_HEIGHT - 60 - (LOGO_HEIGHT / 2) + 15;
    let domain = urls.current_hostname()?;

    let logo_size = logo.as_ref().map(|(_, w, h)| (*w, *h));
    Ok(Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"{OG_WIDTH}\" height=\"{OG_HEIGHT}\" viewBox=\"0 0 {OG_WIDTH} {OG_HEIGHT}\">\n  \
         <defs>\n    <clipPath id=\"avatar-clip\">\n      <rect x=\"{SIDE_MARGIN}\" y=\"{author_y}\" width=\"{AVATAR_SIZE}\" height=\"{AVATAR_SIZE}\" rx=\"20\"/>\n    </clipPath>\n  </defs>\n\n  \
         <!-- Background -->\n  <rect width=\"{OG_WIDTH}\" height=\"{OG_HEIGHT}\" fill=\"#{}\"/>\n\n  \
         <!-- Top accent bar -->\n  <rect width=\"{OG_WIDTH}\" height=\"18\" fill=\"#{}\"/>\n\n  \
         <!-- Category pill -->\n  {}\n\n  \
         <!-- Title -->\n  {}\n\n  \
         <!-- Author -->\n  {}\n\n  \
         <!-- Site logo -->\n  {}\n\n  \
         <!-- Stats -->\n  {}\n\n  \
         <!-- Site domain -->\n  {}\n</svg>\n",
        colors.secondary,
        colors.tertiary,
        category_pill_svg(&category_name, &category_color, SIDE_MARGIN, 60),
        title_svg(&display_lines, &colors.primary, SIDE_MARGIN, title_start_y),
        author_svg(
            avatar_asset.as_ref(),
            username.as_deref(),
            created_at.as_deref(),
            &colors,
            SIDE_MARGIN,
            author_y
        ),
        logo_svg(logo_asset.as_ref(), logo_size, SIDE_MARGIN, logo_y),
        stats_svg(
            like_count,
            posts_count,
            read_time,
            &colors,
            OG_WIDTH - SIDE_MARGIN,
            stats_y
        ),
        domain_svg(&domain, &colors, OG_WIDTH - SIDE_MARGIN, 88),
    )))
}

/// Ruby's `[a, b].min` of a Float and an Integer, as it prints: the
/// Integer when it is the smaller.
fn pill_width(name_chars: usize, max: i64) -> (f64, String) {
    let width = name_chars as f64 * 14.5 + 32.0;
    if (max as f64) < width {
        (max as f64, max.to_string())
    } else {
        (width, crate::ruby::float_to_s(width))
    }
}

/// `category_pill_svg`: the name in a tinted pill, cut to fit the canvas.
fn category_pill_svg(name: &str, color: &str, x: i64, y: i64) -> String {
    if crate::ruby::is_blank(name) {
        return String::new();
    }
    let pill_padding = 16;
    let max_pill_width = OG_WIDTH - 2 * SIDE_MARGIN;
    let chars = name.chars().count();
    let (width, width_s) = pill_width(chars, max_pill_width);
    let max_name_chars =
        (((width - (pill_padding * 2) as f64) / 14.5).floor() as i64).max(1) as usize;
    let display_name = if chars > max_name_chars {
        let keep = max_name_chars.saturating_sub(1).max(1);
        format!("{}…", name.chars().take(keep).collect::<String>())
    } else {
        name.to_string()
    };
    format!(
        "<rect x=\"{x}\" y=\"{y}\" width=\"{width_s}\" height=\"40\" rx=\"6\" fill=\"#{color}\" fill-opacity=\"0.15\"/>\n\
         <text x=\"{}\" y=\"{}\" font-family=\"{FONT_FAMILY}\" font-size=\"24\" font-weight=\"600\" fill=\"#{color}\">{}</text>\n",
        x + pill_padding,
        y + 30,
        escape_xml(&display_name)
    )
}

fn title_svg(lines: &[String], primary: &str, x: i64, start_y: i64) -> String {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            format!(
                "<text x=\"{x}\" y=\"{}\" font-family=\"{FONT_FAMILY}\" font-size=\"62\" font-weight=\"700\" fill=\"#{primary}\">{}</text>",
                start_y + i as i64 * 68,
                escape_xml(line)
            )
        })
        .collect::<Vec<_>>()
        .join("\n    ")
}

fn author_svg(
    avatar: Option<&Asset>,
    username: Option<&str>,
    created_at: Option<&str>,
    colors: &Colors,
    x: i64,
    y: i64,
) -> String {
    let Some(username) = username.filter(|u| !crate::ruby::is_blank(u)) else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(avatar) = avatar {
        parts.push(format!(
            "<image x=\"{x}\" y=\"{y}\" width=\"{AVATAR_SIZE}\" height=\"{AVATAR_SIZE}\" href=\"{}\" clip-path=\"url(#avatar-clip)\"/>\n\
             <rect x=\"{x}\" y=\"{y}\" width=\"{AVATAR_SIZE}\" height=\"{AVATAR_SIZE}\" rx=\"20\" fill=\"none\" stroke=\"#{}\" stroke-opacity=\"0.1\" stroke-width=\"1\"/>",
            avatar.data_uri, colors.secondary
        ));
    }
    let text_x = if avatar.is_some() {
        x + AVATAR_SIZE + 32
    } else {
        x
    };
    let text_y = y + AVATAR_SIZE / 2 + 8;
    let mut text = username.to_string();
    if let Some(created_at) = created_at.filter(|c| !c.is_empty()) {
        text = format!("{text}  ·  {created_at}");
    }
    parts.push(format!(
        "<text x=\"{text_x}\" y=\"{text_y}\" font-family=\"{FONT_FAMILY}\" font-size=\"34\" font-weight=\"400\" fill=\"#{}\" opacity=\"0.6\">{}</text>",
        colors.primary,
        escape_xml(&text)
    ));
    parts.join("\n    ")
}

/// `logo_svg`: the logo fitted into 300x100, its own proportions kept
/// when its upload knows them, centred in the logo's row.
fn logo_svg(
    logo: Option<&Asset>,
    size: Option<(Option<i32>, Option<i32>)>,
    x: i64,
    y: i64,
) -> String {
    let Some(logo) = logo else {
        return String::new();
    };
    let (orig_w, orig_h) = size
        .map(|(w, h)| (f64::from(w.unwrap_or(0)), f64::from(h.unwrap_or(0))))
        .unwrap_or((0.0, 0.0));
    let (w, h) = if orig_w > 0.0 && orig_h > 0.0 {
        let scale = (LOGO_WIDTH as f64 / orig_w).min(LOGO_HEIGHT as f64 / orig_h);
        (ruby_round(orig_w * scale), ruby_round(orig_h * scale))
    } else {
        (LOGO_WIDTH, LOGO_HEIGHT)
    };
    let offset_y = y + (LOGO_HEIGHT - h).div_euclid(2);
    format!(
        "<image x=\"{x}\" y=\"{offset_y}\" width=\"{w}\" height=\"{h}\" href=\"{}\"/>\n",
        logo.data_uri
    )
}

/// `Float#round`: half away from zero.
fn ruby_round(f: f64) -> i64 {
    f.round() as i64
}

fn stats_svg(
    likes: i64,
    replies: i64,
    read_time: Option<i64>,
    colors: &Colors,
    right_x: i64,
    y: i64,
) -> String {
    let mut items = Vec::new();
    if let Some(minutes) = read_time.filter(|m| *m >= 1) {
        items.push(format!("{minutes}m read"));
    }
    if replies > 0 {
        items.push(format!(
            "{replies} {}",
            if replies == 1 { "reply" } else { "replies" }
        ));
    }
    if likes > 0 {
        items.push(format!(
            "{likes} {}",
            if likes == 1 { "like" } else { "likes" }
        ));
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<text x=\"{right_x}\" y=\"{y}\" font-family=\"{FONT_FAMILY}\" font-size=\"30\" font-weight=\"400\" fill=\"#{}\" opacity=\"0.5\" text-anchor=\"end\">{}</text>",
        colors.primary,
        escape_xml(&items.join("  ·  "))
    )
}

fn domain_svg(domain: &str, colors: &Colors, right_x: i64, y: i64) -> String {
    if crate::ruby::is_blank(domain) {
        return String::new();
    }
    format!(
        "<text x=\"{right_x}\" y=\"{y}\" font-family=\"{FONT_FAMILY}\" font-size=\"30\" font-weight=\"400\" fill=\"#{}\" opacity=\"0.5\" text-anchor=\"end\">{}</text>",
        colors.primary,
        escape_xml(domain)
    )
}

/// `calculate_read_time`: the words at read_time_word_count a minute, or
/// four seconds a post, whichever is longer, in whole minutes up.
fn read_time(settings: &SiteSettings, topic: &Topic) -> Result<Option<i64>, AppError> {
    let words = i64::from(topic.word_count.unwrap_or(0));
    let per_minute = settings.get("read_time_word_count")?.to_i();
    if words <= 0 || per_minute <= 0 {
        return Ok(None);
    }
    let by_words = (words / per_minute) as f64;
    let by_posts = f64::from(topic.posts_count) * 4.0 / 60.0;
    Ok(Some(by_words.max(by_posts).ceil() as i64))
}

fn truncated_title(title: &str) -> String {
    if title.chars().count() > MAX_TITLE_LENGTH {
        format!(
            "{}…",
            title.chars().take(MAX_TITLE_LENGTH - 1).collect::<String>()
        )
    } else {
        title.to_string()
    }
}

/// `String#split` with no pattern: runs of ASCII whitespace.
fn ruby_split(text: &str) -> impl Iterator<Item = &str> {
    text.split([' ', '\t', '\n', '\x0b', '\x0c', '\r'])
        .filter(|w| !w.is_empty())
}

/// `word_wrap`: whole words to a line, a word longer than a line split at
/// grapheme boundaries.
fn word_wrap(text: &str, max_chars: usize) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in ruby_split(text) {
        let chunks: Vec<String> = if word.chars().count() > max_chars {
            let graphemes: Vec<&str> = word.graphemes(true).collect();
            graphemes.chunks(max_chars).map(|c| c.concat()).collect()
        } else {
            vec![word.to_string()]
        };
        for chunk in chunks {
            if current.is_empty() {
                current = chunk;
            } else if current.chars().count() + 1 + chunk.chars().count() <= max_chars {
                current.push(' ');
                current.push_str(&chunk);
            } else {
                lines.push(std::mem::replace(&mut current, chunk));
            }
        }
    }
    if !crate::ruby::is_blank(&current) {
        lines.push(current);
    }
    lines
}

/// `fetch_colors`
async fn colors(conn: &mut PgConnection, settings: &SiteSettings) -> Result<Colors, AppError> {
    let scheme_id: Option<i32> =
        sqlx::query_scalar("SELECT color_scheme_id FROM themes WHERE id = $1")
            .bind(i32::try_from(settings.get("default_theme_id")?.to_i()).unwrap_or(0))
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    let scheme = match scheme_id {
        Some(id) => crate::color_scheme::ColorScheme::find(&mut *conn, id).await?,
        None => None,
    };
    let resolved: Vec<(String, String)> = match &scheme {
        Some(scheme) => scheme.resolved_colors()?,
        None => crate::color_scheme::base_colors().to_vec(),
    };
    let get = |name: &str, default: &str| {
        resolved
            .iter()
            .find(|(n, _)| n == name)
            .map_or_else(|| default.to_string(), |(_, hex)| hex.clone())
    };
    Ok(Colors {
        primary: get("primary", "000000"),
        secondary: get("secondary", "ffffff"),
        tertiary: get("tertiary", "0088cc"),
    })
}

/// `fetch_as_data_uri` (and `materialize_asset`'s filter): the image
/// downloaded (FileHelper.download, a megabyte at most, redirects
/// followed) as a data URI typed by its path's extension; None when it
/// can't be had or isn't an image the card can draw.
async fn fetch_as_data_uri(
    conn: &mut PgConnection,
    cx: &Cx<'_>,
    url: &str,
) -> Result<Option<Asset>, AppError> {
    if crate::ruby::is_blank(url) {
        return Ok(None);
    }
    let urls = cx.urls();
    // get_absolute_url
    let absolute = if url.starts_with("http") {
        url.to_string()
    } else if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        format!("{}{url}", urls.base_url_no_prefix()?)
    };
    let fetcher = crate::final_destination::Fetcher {
        settings: cx.settings,
        urls: Urls {
            config: cx.config,
            settings: cx.settings,
        },
    };
    let opts = crate::final_destination::Options {
        timeout: std::time::Duration::from_secs(10),
        max_redirects: 5,
        https_redirect_ignore_limit: false,
        stop_at_blocked_pages: false,
        headers: Vec::new(),
    };
    let read = fetcher
        .get(conn, &absolute, &opts, |_| MAX_ASSET_BYTES, |_| false)
        .await;
    let bytes = match read {
        Ok(crate::final_destination::Read::Page { body, .. }) if body.len() <= MAX_ASSET_BYTES => {
            body
        }
        Ok(_) => return Ok(None),
        Err(e) => {
            tracing::warn!(url, error = %e, "Failed to fetch image for OG generation");
            return Ok(None);
        }
    };
    // MiniMime.lookup_by_filename(path), image/png without one; then only
    // the types materialize_asset keeps.
    let path = url::Url::parse(&absolute)
        .map(|u| u.path().to_string())
        .unwrap_or(absolute.clone());
    let extension = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    let content_type = match extension.as_deref() {
        None => "image/png",
        Some("png") => "image/png",
        Some("jpg" | "jpeg" | "jpe") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some(_) => return Ok(None),
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(Asset {
        data_uri: format!("data:{content_type};base64,{encoded}"),
    }))
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

/// The fonts the card is set in: the system's, with the letter avatars'
/// font as the last resort, `sans-serif` the first common sans family
/// present.
fn fonts(letter_avatar_font: &std::path::Path) -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: LazyLock<std::sync::Mutex<Option<Arc<resvg::usvg::fontdb::Database>>>> =
        LazyLock::new(|| std::sync::Mutex::new(None));
    let mut cached = FONTS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(db) = cached.as_ref() {
        return db.clone();
    }
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_system_fonts();
    let _ = db.load_font_file(letter_avatar_font);
    let families: Vec<String> = db
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
        .collect();
    let preferred = [
        "DejaVu Sans",
        "Liberation Sans",
        "Noto Sans",
        "Arial",
        "Helvetica",
        "Nimbus Sans",
        "Nimbus Sans L",
    ];
    if let Some(family) = preferred
        .iter()
        .find(|p| families.iter().any(|f| f == *p))
        .map(|p| p.to_string())
        .or_else(|| families.first().cloned())
    {
        db.set_sans_serif_family(family);
    }
    let db = Arc::new(db);
    *cached = Some(db.clone());
    db
}

/// `render_png`: the SVG drawn on a transparent 1200x630 canvas.
fn render_png(svg: &str, letter_avatar_font: &std::path::Path) -> Option<Vec<u8>> {
    let opts = resvg::usvg::Options {
        fontdb: fonts(letter_avatar_font),
        ..resvg::usvg::Options::default()
    };
    let tree = resvg::usvg::Tree::from_str(svg, &opts).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(OG_WIDTH as u32, OG_HEIGHT as u32)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

/// `Topic::OG_IMAGE_REGENERATION_COUNTER_STEP`
pub const COUNTER_STEP: i64 = 10;

/// `Topic.og_image_counter_regeneration_needed?`: the count crossed a
/// multiple of the step.
pub fn counter_regeneration_needed(previous: i64, current: i64) -> bool {
    current >= COUNTER_STEP && previous / COUNTER_STEP < current / COUNTER_STEP
}

/// `Topic#clear_generated_og_image!`
pub async fn clear_generated(conn: &mut PgConnection, topic_id: i32) -> Result<(), AppError> {
    let old: Option<i64> =
        sqlx::query_scalar("SELECT og_image_upload_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?
            .flatten();
    let Some(old) = old else {
        return Ok(());
    };
    sqlx::query("UPDATE topics SET og_image_upload_id = NULL WHERE id = $1")
        .bind(topic_id)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "DELETE FROM upload_references WHERE target_type = 'Topic' AND target_id = $1 AND upload_id = $2",
    )
    .bind(topic_id)
    .bind(old)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `Jobs.enqueue(:generate_topic_og_image, topic_id:)`
pub async fn enqueue(conn: &mut PgConnection, topic_id: i32) -> Result<(), AppError> {
    crate::jobs::enqueue(
        conn,
        "generate_topic_og_image",
        serde_json::json!({ "topic_id": topic_id }),
    )
    .await?;
    Ok(())
}

/// `enqueue_og_image_regeneration_after_counter_change`: a topic with a
/// generated card regenerates it when a count crosses a step.
pub async fn after_counter_change(
    conn: &mut PgConnection,
    settings: &SiteSettings,
    topic_id: i32,
    previous: i64,
    current: i64,
) -> Result<(), AppError> {
    if !settings.get("generate_topic_og_image")?.truthy() {
        return Ok(());
    }
    let has_og: bool =
        sqlx::query_scalar("SELECT og_image_upload_id IS NOT NULL FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or(false);
    if !has_og || eligible(conn, settings, topic_id).await? != Some(true) {
        return Ok(());
    }
    if !counter_regeneration_needed(previous, current) {
        return Ok(());
    }
    enqueue(conn, topic_id).await
}

/// Jobs::GenerateTopicOgImage: a topic with an image of its own drops
/// its generated card; an eligible one gets a new card, the old one's
/// reference let go.
pub async fn job(conn: &mut PgConnection, cx: &Cx<'_>, topic_id: i32) -> Result<(), AppError> {
    if !cx.settings.get("generate_topic_og_image")?.truthy() {
        return Ok(());
    }
    let row: Option<(Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT image_upload_id, og_image_upload_id FROM topics WHERE id = $1")
            .bind(topic_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((image_upload_id, old)) = row else {
        return Ok(());
    };
    if image_upload_id.is_some() {
        return clear_generated(conn, topic_id).await;
    }
    if eligible(conn, cx.settings, topic_id).await? != Some(true) {
        return Ok(());
    }
    let Some(upload_id) = generate(conn, cx, topic_id).await? else {
        return Ok(());
    };
    sqlx::query("UPDATE topics SET og_image_upload_id = $2 WHERE id = $1")
        .bind(topic_id)
        .bind(upload_id)
        .execute(&mut *conn)
        .await?;
    crate::upload_references::ensure_exist(
        &mut *conn,
        "Topic",
        i64::from(topic_id),
        Some(upload_id),
    )
    .await?;
    if let Some(old) = old {
        sqlx::query(
            "DELETE FROM upload_references WHERE target_type = 'Topic' AND target_id = $1 AND upload_id = $2",
        )
        .bind(topic_id)
        .bind(old)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_wrap_by_words_and_split_long_runs() {
        assert_eq!(
            word_wrap(
                "How do I configure the thing that makes the other thing work",
                34
            ),
            vec![
                "How do I configure the thing that",
                "makes the other thing work"
            ]
        );
        let long = "a".repeat(40);
        assert_eq!(word_wrap(&long, 34), vec!["a".repeat(34), "a".repeat(6)]);
        assert!(word_wrap("   ", 34).is_empty());
    }

    #[test]
    fn the_pill_width_prints_as_ruby_does() {
        assert_eq!(pill_width(4, 1040).1, "90.0");
        assert_eq!(pill_width(5, 1040).1, "104.5");
        assert_eq!(pill_width(200, 1040).1, "1040");
    }

    #[test]
    fn long_titles_are_cut() {
        let title = "x".repeat(120);
        let cut = truncated_title(&title);
        assert_eq!(cut.chars().count(), 100);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_card_renders() {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{OG_WIDTH}\" height=\"{OG_HEIGHT}\"><rect width=\"{OG_WIDTH}\" height=\"{OG_HEIGHT}\" fill=\"#ffffff\"/><text x=\"80\" y=\"190\" font-family=\"sans-serif\" font-size=\"62\">Hello</text></svg>"
        );
        let png = render_png(&svg, std::path::Path::new("/nonexistent")).expect("a png");
        let image = image::load_from_memory(&png).unwrap();
        assert_eq!((image.width(), image.height()), (1200, 630));
    }
}
