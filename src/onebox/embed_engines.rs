//! The engines over StandardEmbed (lib/onebox/engine/*_onebox.rb): media
//! sites rendered from their OpenGraph or oEmbed data, and the ones that
//! only rewrite the URL into an embed.

use std::sync::LazyLock;

use serde_json::{Map, Value, json};

use super::embed::Embed;
use super::engines::{Ctx, Fail, Rendered, https, https_host, layout, link, placeholder};
use super::fetch::{Fetch, FetchError, FetchOpts};
use super::generic::{Normalizer, active_support_truncate, decode, helpers_truncate, ruby_to_s};
use super::matcher::Engine;
use super::normalize_url_for_output;
use super::uri::RubyUri;
use crate::Unsupported;

/// What `#{value}` interpolates.
pub(super) fn rs(v: &Value) -> Result<String, Fail> {
    Ok(ruby_to_s(v)?)
}

/// `og.title_attr`
pub(super) fn title_attr(n: &Normalizer) -> Result<String, Fail> {
    Ok(match n.value("title")? {
        Value::Null => String::new(),
        t => format!("title='{}'", rs(&t)?),
    })
}

/// `og.secure_image_url`: `URI(get(:image))` with its scheme https.
pub(super) fn secure_image_url(n: &Normalizer) -> Result<String, Fail> {
    let image = n.get("image", None, true)?;
    let Value::String(image) = image else {
        return Err(Fail::Raised(
            "URI::InvalidURIError: bad URI(is not URI?): nil".into(),
        ));
    };
    let Some(mut uri) = RubyUri::parse(&image) else {
        return Err(Fail::Raised(format!("URI::InvalidURIError: {image}")));
    };
    // The port the URI's class defaults to stays implied.
    if uri.port.is_some() && uri.port == default_port(uri.scheme.as_deref()) {
        uri.port = None;
    }
    if uri.opaque.is_some() {
        return Err(Unsupported("opaque image URIs").into());
    }
    uri.scheme = Some("https".into());
    let s = uri.to_s();
    Ok(s)
}

fn default_port(scheme: Option<&str>) -> Option<u16> {
    match scheme? {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

/// The engine's `@url`: https when it always is (or StandardEmbed's
/// https hosts say so).
pub(super) fn engine_url(engine: &Engine, url: &str, standard_embed: bool) -> String {
    let host = RubyUri::parse(url).and_then(|u| u.host).unwrap_or_default();
    if engine.always_https || (standard_embed && https_host(&host)) {
        https(url)
    } else {
        url.to_string()
    }
}

/// A `to_html` or `placeholder_html` as Preview takes it: a web error is
/// blank, anything else raised is kept apart.
pub(super) fn part(
    r: Result<Option<String>, Fail>,
) -> Result<(Option<String>, Option<String>), Fail> {
    match r {
        Ok(h) => Ok((h, None)),
        Err(Fail::Web(_)) => Ok((None, None)),
        Err(Fail::Raised(m)) => Ok((None, Some(m))),
        Err(Fail::App(e)) => Err(Fail::App(e)),
    }
}

pub(super) fn rendered(
    html: Result<Option<String>, Fail>,
    placeholder: Result<Option<String>, Fail>,
) -> Result<Rendered, Fail> {
    let (html, html_raised) = part(html)?;
    let (placeholder, placeholder_raised) = part(placeholder)?;
    Ok(Rendered {
        html,
        html_raised,
        placeholder,
        placeholder_raised,
        ..Default::default()
    })
}

fn nil_raised(what: &str) -> Fail {
    Fail::Raised(format!("NoMethodError: undefined method for nil ({what})"))
}

/// A Normalizer value as a string, raising for nil as a method call on it
/// would.
fn some_s(v: Value, what: &str) -> Result<String, Fail> {
    match v {
        Value::Null => Err(nil_raised(what)),
        v => rs(&v),
    }
}

// ---- OpenGraph images ----

/// FiveHundredPxOnebox, and OpengraphImage (FlickrShortenedOnebox).
async fn og_image<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
    class: &str,
) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    Ok(Some(format!(
        "<img src='{}' width='{}' height='{}' class='{class}' {}>",
        rs(&og.value("image")?)?,
        rs(&og.value("image_width")?)?,
        rs(&og.value("image_height")?)?,
        title_attr(&og)?
    )))
}

async fn animated_image<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    if og.value("image")?.is_null() {
        let escaped = normalize_url_for_output(&e.url);
        return Ok(Some(format!(
            "<img src='{escaped}' class='animated onebox'>"
        )));
    }
    og_image(e, fetch, "animated onebox").await
}

// ---- oEmbed players ----

async fn oembed_html<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    match e.oembed(fetch).await?.value("html")? {
        Value::Null => Ok(None),
        v => Ok(Some(rs(&v)?)),
    }
}

/// `oembed.html.gsub("visual=true", "visual=false")`
async fn oembed_html_visual<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let html = some_s(e.oembed(fetch).await?.value("html")?, "html")?;
    Ok(Some(html.replace("visual=true", "visual=false")))
}

/// The thumbnail placeholder, nil without one.
async fn oembed_thumbnail<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let oembed = e.oembed(fetch).await?;
    let thumb = oembed.value("thumbnail_url")?;
    if super::generic::blank(&thumb) {
        return Ok(None);
    }
    Ok(Some(format!(
        "<img src='{}' {}>",
        rs(&thumb)?,
        title_attr(&oembed)?
    )))
}

/// Audioboom's and Replit's placeholders.
async fn oembed_sized_thumbnail<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
    replit: bool,
) -> Result<Option<String>, Fail> {
    let o = e.oembed(fetch).await?;
    let (thumb, width, height, title) = (
        rs(&o.value("thumbnail_url")?)?,
        rs(&o.value("width")?)?,
        rs(&o.value("height")?)?,
        title_attr(&o)?,
    );
    Ok(Some(if replit {
        format!(
            "          <img src=\"{thumb}\" style=\"max-width: {width}px; max-height: {height}px;\" {title}>\n"
        )
    } else {
        format!(
            "          <img\n            src=\"{thumb}\"\n            style=\"max-width: {width}px; max-height: {height}px;\"\n            {title}\n          >\n"
        )
    }))
}

async fn coub_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let o = e.oembed(fetch).await?;
    Ok(Some(format!(
        "<img src='{}' height='{}' width='{}' {}>",
        rs(&o.value("thumbnail_url")?)?,
        rs(&o.value("thumbnail_height")?)?,
        rs(&o.value("thumbnail_width")?)?,
        title_attr(&o)?
    )))
}

async fn mixcloud_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let o = e.oembed(fetch).await?;
    Ok(Some(format!(
        "          <aside class=\"onebox mixcloud-preview\">\n            <article class=\"onebox-body\">\n              <img src=\"{}\">\n              <div class=\"video-icon\"></div>\n              <div class=\"mixcloud-text\">\n                <h3><a href=\"{}\" target=\"_blank\" rel=\"nofollow ugc noopener\">{}</a></h3>\n                <h4>{}</h4>\n              </div>\n            </article>\n          </aside>\n",
        rs(&o.value("image")?)?,
        rs(&o.value("url")?)?,
        rs(&o.value("title")?)?,
        rs(&o.value("author_name")?)?
    )))
}

async fn spotify_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let o = e.oembed(fetch).await?;
    let thumb = o.value("thumbnail_url")?;
    if super::generic::blank(&thumb) {
        return Ok(None);
    }
    let title = rs(&o.value("title")?)?;
    Ok(Some(format!(
        "<img src='{}' title='{title}' alt='{title}' height='{}' width='{}'>",
        rs(&thumb)?,
        rs(&o.value("thumbnail_height")?)?,
        rs(&o.value("thumbnail_width")?)?
    )))
}

async fn wistia<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    static SRC: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r#"iframe src="(.*?)""#).unwrap());
    let o = e.oembed(fetch).await?;
    let html = some_s(o.value("html")?, "html")?;
    let Some(c) = SRC.captures(&html) else {
        return Ok(Some(html));
    };
    Ok(Some(format!(
        "<iframe\n  src=\"{}\"\n  width=\"{}\"\n  height=\"{}\"\n  title=\"{}\"\n  frameborder=\"0\"\n  allowfullscreen\n></iframe>\n",
        &c[1],
        rs(&o.value("width")?)?,
        rs(&o.value("height")?)?,
        rs(&o.value("title")?)?
    )))
}

/// Ruby's `Float#to_s`.
pub(super) fn float_s(f: f64) -> Result<String, Unsupported> {
    if !f.is_finite() || (f != 0.0 && !(1e-4..1e16).contains(&f.abs())) {
        return Err(Unsupported("floats Ruby prints in exponent form"));
    }
    let s = format!("{f}");
    Ok(if s.contains('.') { s } else { format!("{s}.0") })
}

fn int_of(v: &Value, what: &str) -> Result<i64, Fail> {
    match v {
        Value::Number(n) if n.is_i64() => Ok(n.as_i64().unwrap()),
        Value::Null => Err(nil_raised(what)),
        _ => Err(Unsupported("oEmbed dimensions that aren't integers").into()),
    }
}

async fn tiktok_default<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let o = e.oembed(fetch).await?;
    let thumb_height = int_of(&o.value("thumbnail_height")?, "thumbnail_height")?;
    let video_height = if thumb_height < 1024 {
        998
    } else {
        thumb_height
    };
    let height = float_s((323.0 / 576.0) * video_height as f64)?;
    let id = rs(&o.value("embed_product_id")?)?;
    Ok(Some(format!(
        "          <iframe\n            class=\"tiktok-onebox\"\n            src=\"https://www.tiktok.com/embed/v2/{id}\"\n            sandbox=\"allow-popups allow-popups-to-escape-sandbox allow-scripts allow-top-navigation allow-same-origin\"\n            frameborder=\"0\"\n            seamless=\"seamless\"\n            scrolling=\"no\"\n            style=\"\n              min-width: 323px;\n              height: {height}px;\n              border: 4px solid #fff;\n              border-top: 3px solid #fff;\n              background-color: #fff;\n              border-radius: 9px;\n              \"\n          ></iframe>\n"
    )))
}

async fn tiktok_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let o = e.oembed(fetch).await?;
    let w = int_of(&o.value("thumbnail_width")?, "thumbnail_width")?;
    let h = int_of(&o.value("thumbnail_height")?, "thumbnail_height")?;
    Ok(Some(format!(
        "          <img\n            src=\"{}\"\n            title=\"{}\"\n            style=\"\n              max-width: {}px;\n              max-height: {}px;\"\n          >\n",
        rs(&o.value("thumbnail_url")?)?,
        rs(&o.value("title")?)?,
        w.div_euclid(2),
        h.div_euclid(2)
    )))
}

// ---- OpenGraph players ----

async fn band_camp_html<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    let src = match og.value("video_secure_url")? {
        Value::Null => og.value("video")?,
        v => v,
    };
    Ok(Some(format!(
        "          <iframe\n            src=\"{}\"\n            width=\"{}\"\n            height=\"{}\"\n            scrolling=\"no\"\n            frameborder=\"0\"\n            allowfullscreen\n          ></iframe>\n",
        rs(&src)?,
        rs(&og.value("video_width")?)?,
        rs(&og.value("video_height")?)?
    )))
}

async fn band_camp_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    Ok(Some(format!(
        "<img src='{}' height='{}' {}>",
        rs(&og.value("image")?)?,
        rs(&og.value("video_height")?)?,
        title_attr(&og)?
    )))
}

async fn cloud_app<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    static VIDEO: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)\.(mp4|ogv|webm)$").unwrap());
    let og = e.opengraph(fetch).await?;
    let title = rs(&og.value("title")?)?;
    if !og.value("image")?.is_null() {
        return Ok(Some(format!(
            "          <a href='{}' target='_blank' class='onebox' rel='noopener'>\n            <img src='{}' {} alt='CloudApp' width='480'>\n          </a>\n",
            rs(&og.value("url")?)?,
            rs(&og.value("image")?)?,
            title_attr(&og)?
        )));
    }
    if VIDEO.is_match(&title) {
        let direct =
            normalize_url_for_output(&format!("{}/{}", rs(&og.get("url", None, true)?)?, title));
        return Ok(Some(format!(
            "          <video width='480' height='360' {} controls loop>\n            <source src='{direct}' type='video/mp4'>\n          </video>\n",
            title_attr(&og)?
        )));
    }
    Ok(Some(format!(
        "          <a href='{}' target='_blank' rel='noopener'>\n            {title}\n          </a>\n",
        rs(&og.value("url")?)?
    )))
}

async fn flickr<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    let escaped = normalize_url_for_output(&e.url);
    if og
        .value("url")?
        .as_str()
        .is_some_and(|u| u.contains("/sets/"))
    {
        return Ok(Some(format!(
            "          <div class='onebox flickr-album'>\n            <a href='{escaped}' target='_blank' rel='noopener'>\n              <span class='outer-box' style='max-width:{}px'>\n                <span class='inner-box'>\n                  <span class='album-title'>[Album] {}</span>\n                </span>\n              </span>\n              <img src='{}' {} height='{}' width='{}'>\n            </a>\n          </div>\n",
            rs(&og.value("image_width")?)?,
            rs(&og.value("title")?)?,
            secure_image_url(&og)?,
            title_attr(&og)?,
            rs(&og.value("image_height")?)?,
            rs(&og.value("image_width")?)?
        )));
    }
    if og.value("image")?.is_null() {
        return Ok(None);
    }
    Ok(Some(format!(
        "          <a href='{escaped}' target='_blank' rel='noopener' class=\"onebox\">\n            <img src='{}' {} alt='Imgur' height='{}' width='{}'>\n          </a>\n",
        secure_image_url(&og)?,
        title_attr(&og)?,
        rs(&og.value("image_height")?)?,
        rs(&og.value("image_width")?)?
    )))
}

async fn google_photos<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    let escaped = normalize_url_for_output(&e.url);
    if !og.value("video_secure_url")?.is_null() {
        let raw = e.raw(fetch).await?;
        let r = |k: &str| rs(raw.get(k).unwrap_or(&Value::Null));
        return Ok(Some(format!(
            "          <aside class=\"onebox google-photos\">\n            <header class=\"source\">\n              <img src=\"{}\" class=\"site-icon\" width=\"16\" height=\"16\">\n              <a href=\"{escaped}\" target=\"_blank\" rel=\"nofollow ugc noopener\">{}</a>\n            </header>\n            <article class=\"onebox-body\">\n              <h3><a href=\"{escaped}\" target=\"_blank\" rel=\"nofollow ugc noopener\">{}</a></h3>\n              <div class=\"aspect-image-full-size\">\n                <a href=\"{escaped}\" target=\"_blank\" rel=\"nofollow ugc noopener\" class=\"image-wrapper\">\n                  <img src=\"{}\" class=\"scale-image\"/>\n                  <span class=\"video-icon\"></span>\n                </a>\n              </div>\n            </article>\n          </aside>\n",
            r("favicon")?,
            r("site_name")?,
            rs(&og.value("title")?)?,
            secure_image_url(&og)?
        )));
    }
    if og.value("type")?.as_str() == Some("google_photos:photo_album") {
        let title = rs(&og.value("title")?)?;
        let album_title = match og.value("description")? {
            Value::Null => title,
            d => format!("[{}] {title}", rs(&d)?),
        };
        return Ok(Some(format!(
            "          <div class='onebox google-photos-album'>\n            <a href='{escaped}' target='_blank' rel='noopener'>\n              <span class='outer-box' style='width:{}px'>\n                <span class='inner-box'>\n                  <span class='album-title'>{}</span>\n                </span>\n              </span>\n              <img src='{}' {} height='{}' width='{}'>\n            </a>\n          </div>\n",
            rs(&og.value("image_width")?)?,
            helpers_truncate(&album_title, 80),
            secure_image_url(&og)?,
            title_attr(&og)?,
            rs(&og.value("image_height")?)?,
            rs(&og.value("image_width")?)?
        )));
    }
    if og.value("image")?.is_null() {
        return Ok(None);
    }
    Ok(Some(format!(
        "          <a href='{escaped}' target='_blank' rel='noopener' class=\"onebox\">\n            <img src='{}' {} alt='Google Photos' height='{}' width='{}'>\n          </a>\n",
        secure_image_url(&og)?,
        title_attr(&og)?,
        rs(&og.value("image_height")?)?,
        rs(&og.value("image_width")?)?
    )))
}

async fn imgur<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    let escaped = normalize_url_for_output(&e.url);
    let video = og.value("video_secure_url")?;
    if !video.is_null() {
        let src = rs(&video)?;
        return Ok(Some(format!(
            "            <video width='{}' height='{}' {} controls loop>\n              <source src='{src}' type='video/mp4'>\n              <source src='{}' type='video/webm'>\n            </video>\n",
            rs(&og.value("video_width")?)?,
            rs(&og.value("video_height")?)?,
            title_attr(&og)?,
            src.replace("mp4", "webm")
        )));
    }
    // is_album?
    let response = match fetch
        .fetch_response(
            &format!("https://api.imgur.com/oembed.json?url={}", e.url),
            &FetchOpts::default(),
        )
        .await?
    {
        Ok(b) => b,
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => "{}".to_string(),
    };
    let data: Value = serde_json::from_str(&response)
        .map_err(|e| Fail::Raised(format!("MultiJson::ParseError: {e}")))?;
    let html = match &data {
        Value::Object(o) => match o.get("html") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(_) => return Err(Unsupported("imgur oEmbed html that isn't a string").into()),
        },
        _ => return Err(Fail::Raised("TypeError: oEmbed that isn't a hash".into())),
    };
    let data_id = {
        let dom = super::html_doc::parse(&html);
        crate::pretty_text::select::select_first(&dom.document, "blockquote")
            .and_then(|b| crate::pretty_text::cleanup::attr(&b, "data-id"))
            .unwrap_or_default()
    };
    if data_id.contains("a/") {
        return Ok(Some(format!(
            "            <div class='onebox imgur-album'>\n              <a href='{escaped}' target='_blank' rel='noopener'>\n                <span class='outer-box' style='width:{}px'>\n                  <span class='inner-box'>\n                    <span class='album-title'>[Album] {}</span>\n                  </span>\n                </span>\n                <img src='{}' {} height='{}' width='{}'>\n              </a>\n            </div>\n",
            rs(&og.value("image_width")?)?,
            rs(&og.value("title")?)?,
            secure_image_url(&og)?,
            title_attr(&og)?,
            rs(&og.value("image_height")?)?,
            rs(&og.value("image_width")?)?
        )));
    }
    if og.value("image")?.is_null() {
        return Ok(None);
    }
    let src = secure_image_url(&og)?;
    let src = src.strip_suffix("?fb").unwrap_or(&src);
    Ok(Some(format!(
        "            <a href='{escaped}' target='_blank' rel='noopener' class=\"onebox\">\n              <img src='{src}' {} alt='Imgur'>\n            </a>\n",
        title_attr(&og)?
    )))
}

async fn kaltura<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    Ok(Some(format!(
        "<iframe\n  src=\"{}\"\n  width=\"{}\"\n  height=\"{}\"\n  frameborder='0'\n  allowfullscreen\n></iframe>\n",
        rs(&og.value("video_secure_url")?)?,
        rs(&og.value("video_width")?)?,
        rs(&og.value("video_height")?)?
    )))
}

async fn sketch_fab<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    let src = some_s(og.value("video_url")?, "video_url")?.replace("autostart=1", "");
    Ok(Some(format!(
        "          <iframe\n            src=\"{src}\"\n            width=\"{}\"\n            height=\"{}\"\n            scrolling=\"no\"\n            frameborder=\"0\"\n            allowfullscreen\n          ></iframe>\n",
        rs(&og.value("video_width")?)?,
        rs(&og.value("video_height")?)?
    )))
}

async fn steam_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let og = e.opengraph(fetch).await?;
    Ok(Some(format!(
        "          <div style='width:100%; height:190px; background-color:#262626; color:#9e9e9e; margin:15px 0;'>\n            <div style='padding:10px'>\n              <h3 style='color:#fff; margin:10px 0 10px 5px;'>{}</h3>\n              <img src='{}' style='float:left; max-width:184px; margin:5px 15px 0 5px'/>\n              <p>{}</p>\n            </div>\n          </div>\n",
        rs(&og.value("title")?)?,
        rs(&og.value("image")?)?,
        rs(&og.value("description")?)?
    )))
}

fn steam_html(url: &str) -> Result<Option<String>, Fail> {
    static APP: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"https?://store\.steampowered\.com/app/\d+").unwrap());
    let Some(m) = APP.find(url) else {
        return Err(nil_raised("gsub"));
    };
    let src = normalize_url_for_output(&m.as_str().replace("/app/", "/widget/"));
    Ok(Some(format!(
        "          <iframe\n            src='{src}'\n            frameborder='0'\n            width='100%'\n            height='190'\n          ></iframe>\n"
    )))
}

/// FacebookMediaOnebox: a player card, else the page as the generic
/// engine (without the preview's options) renders it.
async fn facebook<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let twitter = e.twitter(fetch).await?;
    let get = |k: &str| twitter.get(k).and_then(Value::as_str);
    if !twitter.is_empty()
        && get("card") == Some("player")
        && get("player").is_some_and(|p| !crate::ruby::is_blank(p))
    {
        return Ok(Some(format!(
            "            <iframe\n              src=\"{}\"\n              width=\"{}\"\n              height=\"{}\"\n              scrolling=\"no\"\n              frameborder=\"0\"\n              allowfullscreen\n            ></iframe>\n",
            get("player").unwrap_or(""),
            get("player_width").unwrap_or(""),
            get("player_height").unwrap_or("")
        )));
    }
    let html = bare_generic(&e.url, e.ctx, fetch).await?;
    Ok(html.filter(|h| !crate::ruby::is_blank(h)))
}

/// `AllowlistedGenericOnebox.new(@url, @timeout).to_html`: no iframe
/// allowlist, no cookie.
pub(super) async fn bare_generic<F: Fetch>(
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let bare = Ctx {
        iframes: &[],
        cookie: None,
        hostname: ctx.hostname,
        disable_media_download_controls: ctx.disable_media_download_controls,
        i18n: ctx.i18n,
        settings: ctx.settings.clone(),
    };
    let r = super::generic::preview(url, &bare, fetch).await?;
    if let Some(m) = r.html_raised {
        return Err(Fail::Raised(m));
    }
    Ok(r.html)
}

async fn vimeo_default<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let response = match fetch
        .fetch_response(
            &format!("https://vimeo.com/api/oembed.json?url={}", e.url),
            &FetchOpts::default(),
        )
        .await?
    {
        Ok(b) => Some(b),
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => None,
    };
    // `rescue StandardError` gives the String "{}", which `[:html]` raises on.
    let data: Map<String, Value> = match response.as_deref().map(serde_json::from_str::<Value>) {
        Some(Ok(Value::Object(o))) => o,
        _ => {
            return Err(Fail::Raised(
                "TypeError: no implicit conversion of Symbol into Integer".into(),
            ));
        }
    };
    let html = match data.get("html") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(_) => return Err(Unsupported("vimeo oEmbed html that isn't a string").into()),
    };
    let src = {
        let dom = crate::pretty_text::cleanup::parse(&html);
        crate::pretty_text::select::select_first(
            &crate::pretty_text::cleanup::fragment_root(&dom),
            "iframe",
        )
        .and_then(|i| crate::pretty_text::cleanup::attr(&i, "src"))
    };
    let src = match src.filter(|s| !crate::ruby::is_blank(s)) {
        Some(s) => s,
        None => format!(
            "https://player.vimeo.com/video/{}",
            rs(data.get("video_id").unwrap_or(&Value::Null))?
        ),
    };
    let src = src.replace("autoplay=1", "");
    let src = src.strip_suffix('?').unwrap_or(&src);
    Ok(Some(format!(
        "          <iframe\n            class=\"vimeo-onebox\"\n            src=\"{src}\"\n            data-original-href=\"{}\"\n            frameborder=\"0\"\n            allowfullscreen\n          ></iframe>\n",
        link(&e.url)?
    )))
}

// ---- URL-only embeds ----

fn asciinema(url: &str) -> Result<(String, String), Fail> {
    static ID: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
        super::matcher::ruby_regex(
            r"asciinema\.org\/a\/(?<asciinema_id>[\p{Alnum}_\-]+)$",
            false,
        )
    });
    let id = ID
        .captures(url)
        .ok()
        .flatten()
        .and_then(|c| c.name("asciinema_id"))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| nil_raised("match"))?;
    Ok((
        format!(
            "<script type='text/javascript' src='https://asciinema.org/a/{id}.js' id='asciicast-{id}' async></script>"
        ),
        format!("<img src='https://asciinema.org/a/{id}.png'>"),
    ))
}

fn loom(url: &str) -> String {
    let id = url.split('/').rfind(|s| !s.is_empty()).unwrap_or("");
    format!(
        "<iframe\n  class=\"loom-onebox\"\n  src=\"https://www.loom.com/embed/{id}\"\n  frameborder=\"0\"\n  webkitallowfullscreen\n  mozallowfullscreen\n  allowfullscreen\n</iframe>\n"
    )
}

fn restream(url: &str) -> String {
    format!(
        "<iframe\n  class=\"restream-onebox\"\n  src=\"{url}\"\n  width=\"695\"\n  height=\"391\"\n  frameborder=\"0\"\n  allowfullscreen\n></iframe>\n"
    )
}

fn trello(url: &str) -> Result<String, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"trello\.com/(?<type>[^/]+)/(?<key>[^/]+)/?\W*").unwrap()
    });
    let c = M.captures(url).ok_or_else(|| nil_raised("match"))?;
    let height = if &c["type"] == "b" { 400 } else { 200 };
    Ok(format!(
        "          <iframe src=\"https://trello.com/{}/{}.html\" width=\"100%\" height=\"{height}\" frameborder=\"0\" style=\"border:0\"></iframe>\n",
        &c["type"], &c["key"]
    ))
}

fn slides_html(url: &str) -> Result<String, Fail> {
    let path = RubyUri::parse(url).map(|u| u.path).unwrap_or_default();
    Ok(format!(
        "          <iframe\n            src=\"https://slides.com{path}/embed?style=light\"\n            width=\"576\"\n            height=\"420\"\n            scrolling=\"no\"\n            frameborder=\"0\"\n            webkitallowfullscreen\n            mozallowfullscreen\n            allowfullscreen\n          ></iframe>\n"
    ))
}

/// `URI.encode_www_form` of string pairs.
pub(super) fn encode_www_form(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                super::uri::encode_component(k, www_form_safe).replace("%20", "+"),
                super::uri::encode_component(v, www_form_safe).replace("%20", "+")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn www_form_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"*-._".contains(&b)
}

fn reddit(url: &str) -> Result<String, Fail> {
    static COMMENT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^/(?:r|user)/[^/]+/comments/[^/]+/[^/]*/[^/]+/?$").unwrap()
    });
    let uri = RubyUri::parse(url).ok_or(Unsupported("reddit URLs URI can't parse"))?;
    let path = if uri.path.ends_with('/') {
        uri.path.clone()
    } else {
        format!("{}/", uri.path)
    };
    let comment = COMMENT.is_match(&uri.path);
    let mut params: Vec<(&str, String)> = vec![
        ("embed", "true".into()),
        ("ref_source", "embed".into()),
        ("ref", "share".into()),
    ];
    if comment {
        params.extend([
            ("showmedia", "false".into()),
            ("showmore", "false".into()),
            ("depth", "1".into()),
            ("context", "1".into()),
        ]);
    }
    if !path.is_ascii() || path.contains(['?', '#', ' ']) {
        return Err(Unsupported("reddit paths URI::HTTPS.build would refuse").into());
    }
    let embed = normalize_url_for_output(&format!(
        "https://embed.reddit.com{path}?{}",
        encode_www_form(&params)
    ));
    Ok(format!(
        "<iframe\n  class=\"reddit-onebox\"\n  src=\"{embed}\"\n  width=\"640\"\n  height=\"{}\"\n  allowfullscreen\n></iframe>\n",
        if comment { 300 } else { 500 }
    ))
}

// ---- Layout engines ----

const GOOGLE_DOCS: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/googledocs.mustache");
const GOOGLE_DRIVE: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/googledrive.mustache");
const DISCOURSE_TOPIC: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/discoursetopic.mustache");
const GITHUB_FOLDER: &str =
    include_str!("../../vendor/discourse/lib/onebox/templates/githubfolder.mustache");

async fn google_docs<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    static M: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"^(https?:)?//(docs\.google\.com)/(?<endpoint>(spreadsheets|document|forms|presentation))/d/((?<key>[\w-]*)).+$",
        )
        .unwrap()
    });
    let og = e.opengraph(fetch).await?;
    let endpoint = M
        .captures(&e.url)
        .map(|c| c["endpoint"].to_string())
        .ok_or_else(|| nil_raised("match"))?;
    let short = match endpoint.as_str() {
        "spreadsheets" => "sheets",
        "document" => "docs",
        "presentation" => "slides",
        _ => "forms",
    };
    let capitalize = |s: &str| {
        let mut c = s.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    };
    let description = match og.value("description")? {
        Value::String(d) if !crate::ruby::is_blank(&d) => helpers_truncate(&d, 250),
        _ => format!("This {} is private", capitalize(&short[..short.len() - 1])),
    };
    let title = match og.value("title")? {
        Value::Null => json!(format!("Google {}", capitalize(short))),
        t => t,
    };
    let data = json!({
        "link": link(&e.url)?,
        "title": title,
        "description": description,
        "type": short,
    });
    Ok(Some(layout("googledocs", GOOGLE_DOCS, &data)))
}

async fn google_drive<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    static VIDEO: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?m)^video[/.]").unwrap());
    let og = e.opengraph(fetch).await?;
    let og_title = og.value("title")?;
    let mut title = match &og_title {
        Value::Null => "Google Drive".to_string(),
        t => rs(t)?,
    };
    if og
        .value("type")?
        .as_str()
        .is_some_and(|t| VIDEO.is_match(t))
    {
        title = format!("{} (video)", rs(&og_title)?);
    }
    let description = match og.value("description")? {
        Value::Null => "Google Drive file.".to_string(),
        d => rs(&d)?,
    };
    let data = json!({
        "link": link(&e.url)?,
        "title": title,
        "description": helpers_truncate(&description, 250),
        "image": og.value("image")?,
    });
    Ok(Some(layout("googledrive", GOOGLE_DRIVE, &data)))
}

/// InstagramOnebox#data
async fn instagram_data<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Value, Fail> {
    static CLEAN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^https?://(?:www\.)?(?:instagram\.com|instagr\.am)/?(?:.*)/(?:p|tv)/[a-zA-Z\d_-]+")
            .unwrap()
    });
    let o = e.oembed(fetch).await?;
    if o.data.is_empty() {
        return Err(Fail::Raised(
            "No oEmbed data found. Ensure 'facebook_app_access_token' is valid".into(),
        ));
    }
    let clean = CLEAN
        .find(&e.url)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| nil_raised("clean_url"))?;
    let author = rs(&o.value("author_name")?)?;
    let title = o.value("title")?;
    Ok(json!({
        "link": clean.replace(&format!("/{author}/"), "/") + "/embed",
        "title": format!("@{author}"),
        "image": o.value("thumbnail_url")?,
        "image_width": o.data.get("thumbnail_width").cloned().unwrap_or(Value::Null),
        "image_height": o.data.get("thumbnail_height").cloned().unwrap_or(Value::Null),
        "description": match title {
            Value::String(t) => json!(helpers_truncate(&t, 250)),
            v => v,
        },
    }))
}

async fn instagram_html<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let data = instagram_data(e, fetch).await?;
    let height = match &data["image_height"] {
        Value::Null => 0,
        Value::Number(n) => n
            .as_i64()
            .ok_or(Unsupported("instagram heights that aren't integers"))?,
        Value::String(s) => crate::ruby::to_i(s),
        _ => return Err(Unsupported("instagram heights that aren't integers").into()),
    };
    Ok(Some(format!(
        "          <iframe\n            src=\"{}\"\n            width=\"{}\"\n            height=\"{}\"\n            frameborder=\"0\"\n          ></iframe>\n",
        rs(&data["link"])?,
        rs(&data["image_width"])?,
        height + 98
    )))
}

/// DiscourseTopicOnebox#data
async fn discourse_topic_data<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Value, Fail> {
    let raw = e.raw(fetch).await?;
    let get = |k: &str| raw.get(k).cloned().unwrap_or(Value::Null);
    let text = |k: &str| -> Result<String, Fail> {
        match raw.get(k) {
            Some(Value::String(s)) => Ok(s.clone()),
            _ => Err(nil_raised(k)),
        }
    };
    let sections = match get("article_sections") {
        Value::Null => Vec::new(),
        Value::Array(a) => a,
        v => vec![v],
    };
    let colors = get("article_section_colors");
    let mut categories = Vec::new();
    for (i, name) in sections.into_iter().enumerate() {
        let color = match &colors {
            Value::Array(c) => c.get(i).cloned().unwrap_or(Value::Null),
            Value::Null => return Err(nil_raised("article_section_colors")),
            _ => return Err(Unsupported("section colors that aren't a list").into()),
        };
        categories.push(json!({ "name": name, "color": color }));
    }
    let published = match super::ruby_time::parse(&text("published_time")?)? {
        super::ruby_time::Parsed::Time(f) => f,
        super::ruby_time::Parsed::Raised => {
            return Err(Fail::Raised("ArgumentError: published_time".into()));
        }
    };
    let image = super::uri::absolute_url(raw.get("image").and_then(Value::as_str), &e.url);
    let image = normalize_url_for_output(&decode(image.as_deref().unwrap_or(""))?);
    let render_tags = super::generic::present(raw.get("article_tags"));
    let mut data = Map::new();
    data.insert("categories".into(), json!(categories.clone()));
    data.insert("link".into(), json!(link(&e.url)?));
    data.insert("article_published_time".into(), json!(published.short()));
    data.insert(
        "article_published_time_title".into(),
        json!(published.long()),
    );
    data.insert(
        "domain".into(),
        json!(decode(&active_support_truncate(
            &text("site_name")?,
            80,
            "..."
        ))?),
    );
    data.insert(
        "description".into(),
        json!(decode(&active_support_truncate(
            &text("description")?,
            250,
            "..."
        ))?),
    );
    data.insert(
        "title".into(),
        json!(decode(&active_support_truncate(
            &text("title")?,
            80,
            "..."
        ))?),
    );
    data.insert("image".into(), json!(image));
    data.insert("render_tags?".into(), json!(render_tags));
    data.insert(
        "render_category_block?".into(),
        json!(render_tags || !categories.is_empty()),
    );
    // reverse_merge(raw)
    for (k, v) in raw {
        data.entry(k).or_insert(v);
    }
    Ok(Value::Object(data))
}

/// GithubFolderOnebox#data
async fn github_folder_data<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Value, Fail> {
    static PATH: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"github\.com/([\w\-]+/[\w\-]+)/tree/(.+)").unwrap());
    let og = e.opengraph(fetch).await?;
    let max = 250;
    let clean = e
        .url
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");
    let (display_path, fallback_title) = match PATH.captures(clean) {
        Some(c) => {
            let path = c[2].to_string();
            let chars: Vec<char> = path.chars().collect();
            let p = if chars.len() > max {
                chars[chars.len() - max..].iter().collect()
            } else {
                path
            };
            (Some(p), Some(c[1].to_string()))
        }
        None => (None, None),
    };
    let og_title = og.value("title")?;
    let description = match og.value("description")? {
        Value::Null => Value::Null,
        d => {
            let mut d = rs(&d)?;
            let desc_end = format!(" - {}", rs(&og_title)?);
            let n = desc_end.chars().count();
            let chars: Vec<char> = d.chars().collect();
            if chars.len() >= n && chars[chars.len() - n..].iter().collect::<String>() == desc_end {
                d = chars[..chars.len() - n].iter().collect();
            }
            json!(helpers_truncate(&d, max))
        }
    };
    let mut title = match og_title {
        Value::Null => fallback_title.map(Value::String).unwrap_or(Value::Null),
        t => t,
    };
    if let Some(fragment) = RubyUri::parse(&e.url)
        .and_then(|u| u.fragment)
        .or_else(|| e.url.split_once('#').map(|(_, f)| f.to_string()))
    {
        let fragment = super::uri::cgi_unescape(&fragment.replace('+', "%2B"))
            .ok_or(Unsupported("fragments that aren't UTF-8"))?;
        let subtitle = {
            let html = e.html_doc(fetch).await?;
            let dom = super::html_doc::parse(html.as_deref().unwrap_or(""));
            let root = &dom.document;
            if crate::pretty_text::select::select(root, ".Box.md, .Box.rdoc").is_empty() {
                None
            } else {
                crate::pretty_text::select::select(root, "a.anchor")
                    .into_iter()
                    .find(|n| {
                        crate::pretty_text::cleanup::attr(n, "href").as_deref()
                            == Some(&format!("#{fragment}"))
                    })
                    .and_then(|n| crate::pretty_text::select::parent(&n))
                    .map(|p| crate::pretty_text::cleanup::text(&p))
            }
        };
        if let Some(sub) = subtitle {
            title = json!(format!("{} - {sub}", rs(&title)?));
        }
    }
    let title = match title {
        Value::String(t) => json!(helpers_truncate(&t, 250)),
        v => v,
    };
    Ok(json!({
        "link": e.url,
        "title": title,
        "path": display_path,
        "description": description,
        "favicon": e.favicon(fetch).await?,
    }))
}

// ---- YouTube ----

const YT_WIDTH: i64 = 480;
const YT_HEIGHT: i64 = 360;

fn yt_id(raw: Option<&str>) -> Option<String> {
    raw.filter(|r| {
        !r.is_empty()
            && r.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    })
    .map(str::to_string)
}

/// `CGI.parse(uri.query)`, each key's first value.
fn cgi_params(query: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for pair in query.split(['&', ';']) {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = super::uri::cgi_unescape(k).unwrap_or_default();
        let v = super::uri::cgi_unescape(v).unwrap_or_default();
        if !out.iter().any(|(e, _)| *e == k) {
            out.push((k, v));
        }
    }
    out
}

struct YouTube {
    uri: RubyUri,
    params: Vec<(String, String)>,
}

impl YouTube {
    fn new(url: &str) -> Result<YouTube, Fail> {
        let uri = RubyUri::parse(url).ok_or(Unsupported("YouTube URLs URI can't parse"))?;
        let params = uri.query.as_deref().map(cgi_params).unwrap_or_default();
        Ok(YouTube { uri, params })
    }

    fn param(&self, k: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(e, _)| e == k)
            .map(|(_, v)| v.as_str())
    }

    fn has(&self, k: &str) -> bool {
        self.params.iter().any(|(e, _)| e == k)
    }

    fn video_id(&self) -> Option<String> {
        let path = &self.uri.path;
        let after = |marker: &str| -> Option<String> {
            let re = regex::Regex::new(&format!(r"{marker}([\w\-]+)")).unwrap();
            re.captures(path).map(|c| c[1].to_string())
        };
        let mut id = None;
        if self
            .uri
            .host
            .as_deref()
            .is_some_and(|h| h.contains("youtu.be"))
        {
            id = after("/");
        }
        if id.is_none() && path.contains("/embed/") {
            id = after("/embed/");
        }
        if id.is_none() && path.contains("/shorts/") {
            id = after("/shorts/");
        }
        if id.is_none() && path.contains("/live/") {
            id = after("/live/");
        }
        let id = id.or_else(|| self.param("v").map(str::to_string));
        yt_id(id.as_deref())
    }

    fn list_id(&self) -> Option<String> {
        yt_id(self.param("list"))
    }

    /// `parse_timestring`
    fn timestring(s: &str) -> i64 {
        static T: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"(\d+h)?(\d+m)?(\d+s?)?").unwrap());
        let c = T.captures(s).unwrap();
        let n = |i: usize| c.get(i).map_or(0, |m| crate::ruby::to_i(m.as_str()));
        n(1) * 3600 + n(2) * 60 + n(3)
    }

    fn embed_params(&self) -> String {
        let video_id = self.video_id().unwrap_or_default();
        let mut p: Vec<(&str, String)> =
            vec![("feature", "oembed".into()), ("wmode", "opaque".into())];
        if let Some(l) = self.list_id() {
            p.push(("list", l));
        }
        let start = self
            .param("start")
            .or_else(|| self.param("t"))
            .map(str::to_string)
            .or_else(|| {
                self.uri
                    .fragment
                    .as_deref()
                    .and_then(|f| f.strip_prefix("t="))
                    .map(str::to_string)
            });
        if let Some(s) = start {
            p.push(("start", Self::timestring(&s).to_string()));
        }
        if let Some(end) = self.param("end") {
            p.push(("end", Self::timestring(end).to_string()));
        }
        if self.has("loop") {
            p.push(("loop", "1".into()));
            p.push(("playlist", video_id));
        }
        if self.has("rel") {
            p.push(("rel", "0".into()));
        }
        if self.has("enablejsapi") {
            p.push((
                "enablejsapi",
                self.param("enablejsapi").unwrap_or("").to_string(),
            ));
        }
        encode_www_form(&p)
    }
}

async fn youtube_default<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let yt = YouTube::new(&e.url)?;
    if let Some(id) = yt.video_id() {
        return Ok(Some(format!(
            "            <img class=\"youtube-thumbnail onebox\" style=\"display: none;\" src=\"https://img.youtube.com/vi/{id}/maxresdefault.jpg\">\n            <iframe\n              src=\"https://www.youtube.com/embed/{id}?{}\"\n              width=\"{YT_WIDTH}\"\n              height=\"{YT_HEIGHT}\"\n              frameborder=\"0\"\n              allowfullscreen\n              class=\"youtube-onebox\"\n            ></iframe>\n",
            yt.embed_params()
        )));
    }
    if let Some(list) = yt.list_id() {
        return Ok(Some(format!(
            "            <iframe\n              src=\"https://www.youtube.com/embed/videoseries?list={list}&wmode=transparent&rel=0&autohide=1&showinfo=1&enablejsapi=1\"\n              width=\"{YT_WIDTH}\"\n              height=\"{YT_HEIGHT}\"\n              frameborder=\"0\"\n              allowfullscreen\n              class=\"youtube-onebox\"\n            ></iframe>\n"
        )));
    }
    // A channel page: the generic engine's HTML, protocol-relative URLs
    // made https.
    static REL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r#"['"]//"#).unwrap());
    let html = bare_generic(&e.url, e.ctx, fetch).await?;
    Ok(html
        .filter(|h| !crate::ruby::is_blank(h))
        .map(|h| REL.replace_all(&h, "https://").into_owned()))
}

/// `parse_embed_response`: the video's title and thumbnail, from its
/// oEmbed or else its embed page.
async fn youtube_embed_response<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
    video_id: Option<&str>,
) -> Result<Option<(Value, Value)>, Fail> {
    let Some(id) = video_id else {
        // Without a video there is no embed page to read.
        return Err(nil_raised("xpath"));
    };
    match e.oembed(fetch).await {
        Ok(o) => {
            let thumb = o.data.get("thumbnail_url").cloned().unwrap_or(Value::Null);
            let title = o.data.get("title").cloned().unwrap_or(Value::Null);
            let truthy = |v: &Value| !matches!(v, Value::Null | Value::Bool(false));
            if truthy(&thumb) && truthy(&title) {
                return Ok(Some((thumb, title)));
            }
        }
        Err(Fail::App(err)) => return Err(Fail::App(err)),
        Err(_) => {}
    }
    let html =
        super::html_doc::fetch_html_doc(fetch, &format!("https://www.youtube.com/embed/{id}"), &[])
            .await?;
    let script = {
        let dom = super::html_doc::parse(html.as_deref().unwrap_or(""));
        crate::pretty_text::select::select(&dom.document, "script")
            .into_iter()
            .map(|s| crate::pretty_text::cleanup::node_html(&s))
            .find(|s| s.contains("ytcfg.set"))
    };
    let Some(script) = script else {
        return Ok(None);
    };
    // Any step that fails is rescued into nil.
    static CFG: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"ytcfg\.set\((?<json>.*)\)").unwrap());
    let title = (|| -> Option<String> {
        let json = CFG.captures(&script)?.name("json")?.as_str().to_string();
        let cfg: Value = serde_json::from_str(&json).ok()?;
        let response = cfg
            .get("PLAYER_VARS")?
            .get("embedded_player_response")?
            .as_str()?;
        let response: Value = serde_json::from_str(response).ok()?;
        let renderer = response
            .get("embedPreview")?
            .get("thumbnailPreviewRenderer")?;
        let text = renderer
            .get("title")?
            .get("runs")?
            .as_array()?
            .first()?
            .get("text")?;
        Some(match text {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        })
    })();
    Ok(title.map(|t| {
        (
            json!(format!("https://img.youtube.com/vi/{id}/hqdefault.jpg")),
            json!(t),
        )
    }))
}

async fn youtube_placeholder<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
) -> Result<Option<String>, Fail> {
    let yt = YouTube::new(&e.url)?;
    let video_id = yt.video_id();
    if video_id.is_none() && yt.list_id().is_none() {
        return youtube_html(e, fetch).await;
    }
    let result = match youtube_embed_response(e, fetch, video_id.as_deref()).await {
        Ok(r) => r,
        // `rescue StandardError` around the embed page's parsing.
        Err(Fail::Raised(_)) if video_id.is_none() => None,
        Err(err) => return Err(err),
    };
    let (image, title) = match result {
        Some(r) => r,
        None => {
            let og = e.opengraph(fetch).await?;
            (
                og.data.get("image").cloned().unwrap_or(Value::Null),
                og.data.get("title").cloned().unwrap_or(Value::Null),
            )
        }
    };
    let Value::String(title) = title else {
        return Err(Fail::Raised("TypeError: CGI.escapeHTML(nil)".into()));
    };
    Ok(Some(format!(
        "<img src='{}' width='{YT_WIDTH}' height='{YT_HEIGHT}' title='{}'>",
        rs(&image)?,
        crate::discourse_diff::escape_html(&title)
    )))
}

// ---- discourse-lazy-videos ----

/// TiktokOnebox#to_html, as discourse-lazy-videos overrides it.
async fn tiktok_html<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    if e.ctx.settings.lazy_tiktok {
        let o = e.oembed(fetch).await?;
        let id = o.value("embed_product_id")?;
        if !id.is_null() {
            return Ok(Some(crate::plugins::lazy_videos::tiktok_html(
                &rs(&id)?,
                &rs(&o.value("title")?)?,
                &e.url,
                &rs(&o.value("thumbnail_url")?)?,
            )));
        }
    }
    tiktok_default(e, fetch).await
}

/// VimeoOnebox#to_html, as discourse-lazy-videos overrides it.
async fn vimeo<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    if !e.ctx.settings.lazy_vimeo {
        return vimeo_default(e, fetch).await;
    }
    static UNLISTED: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"videos/\d+:.+").unwrap());
    let response = match fetch
        .fetch_response(
            &format!("https://vimeo.com/api/oembed.json?url={}", e.url),
            &FetchOpts::default(),
        )
        .await?
    {
        Ok(b) => Some(b),
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => None,
    };
    let data: Map<String, Value> = match response.as_deref().map(serde_json::from_str::<Value>) {
        Some(Ok(Value::Object(o))) => o,
        _ => {
            return Err(Fail::Raised(
                "TypeError: no implicit conversion of Symbol into Integer".into(),
            ));
        }
    };
    let uri = match data.get("uri") {
        Some(Value::String(u)) => u.clone(),
        _ => return Err(nil_raised("uri")),
    };
    let full_id = uri.replacen("/videos/", "", 1).replacen(':', "/", 1);
    let iframe_id = if !UNLISTED.is_match(&uri) {
        full_id.clone()
    } else {
        let html = match data.get("html") {
            Some(Value::String(h)) => h.clone(),
            _ => String::new(),
        };
        let src = {
            let dom = crate::pretty_text::cleanup::parse(&html);
            crate::pretty_text::select::select_first(
                &crate::pretty_text::cleanup::fragment_root(&dom),
                "iframe",
            )
            .and_then(|i| crate::pretty_text::cleanup::attr(&i, "src"))
        };
        src.ok_or_else(|| nil_raised("src"))?
            .replacen("https://player.vimeo.com/video/", "", 1)
    };
    let og = e.opengraph(fetch).await?;
    Ok(Some(crate::plugins::lazy_videos::vimeo_html(
        &iframe_id,
        &full_id,
        &rs(&og.value("title")?)?,
        &rs(&og.value("image")?)?,
    )))
}

/// The video's thumbnail and title (`parse_embed_response ||
/// get_opengraph.data`).
async fn youtube_result<F: Fetch>(
    e: &mut Embed<'_>,
    fetch: &mut F,
    video_id: Option<&str>,
) -> Result<(Value, Value), Fail> {
    if let Some(r) = youtube_embed_response(e, fetch, video_id).await? {
        return Ok(r);
    }
    let og = e.opengraph(fetch).await?;
    Ok((
        og.data.get("image").cloned().unwrap_or(Value::Null),
        og.data.get("title").cloned().unwrap_or(Value::Null),
    ))
}

/// YoutubeOnebox#to_html, as discourse-lazy-videos overrides it.
async fn youtube_html<F: Fetch>(e: &mut Embed<'_>, fetch: &mut F) -> Result<Option<String>, Fail> {
    let yt = YouTube::new(&e.url)?;
    let Some(id) = yt.video_id().filter(|_| e.ctx.settings.lazy_youtube) else {
        return youtube_default(e, fetch).await;
    };
    let (image, _) = youtube_result(e, fetch, Some(&id)).await?;
    let mut thumbnail = format!("https://img.youtube.com/vi/{id}/maxresdefault.jpg");
    match fetch
        .fetch_response(&thumbnail, &FetchOpts::default())
        .await?
    {
        Ok(_) => {}
        Err(FetchError::Refused(u)) => return Err(u.into()),
        Err(_) => thumbnail = rs(&image)?,
    }
    // video_title
    let (_, title) = youtube_result(e, fetch, Some(&id)).await?;
    let title = match title {
        Value::String(t) => t,
        Value::Null => return Err(nil_raised("chomp")),
        v => rs(&v)?,
    };
    let chomped = title.strip_suffix("- YouTube").unwrap_or(&title);
    if crate::ruby::is_blank(chomped) || crate::ruby::is_blank(&thumbnail) {
        return youtube_default(e, fetch).await;
    }
    Ok(Some(crate::plugins::lazy_videos::youtube_html(
        &id,
        &title,
        yt.param("t"),
        yt.list_id().as_deref(),
        &thumbnail,
    )))
}
/// The StandardEmbed engines' previews.
pub async fn preview<F: Fetch>(
    engine: &Engine,
    url: &str,
    ctx: &Ctx<'_>,
    fetch: &mut F,
) -> Result<Rendered, Fail> {
    let url = engine_url(engine, url, true);
    let mut e = Embed::new(&url, ctx);
    let name = engine.name;
    // The engines' own `get_oembed_url`.
    e.oembed_url = match name {
        "AudioComOnebox" => Some(format!(
            "https://api.audio.com/oembed?url={url}{}",
            if url.contains("/collections/") {
                ""
            } else {
                "&maxheight=228"
            }
        )),
        "MotokoOnebox" => Some(format!(
            "https://embed.smartcontracts.org/services/onebox?url={url}"
        )),
        "SimplecastOnebox" => Some(format!("https://api.simplecast.com/oembed?url={url}")),
        "SoundCloudOnebox" => Some(format!(
            "https://soundcloud.com/oembed.json?url={url}{}",
            if url.contains("/sets/") {
                ""
            } else {
                "&maxheight=166"
            }
        )),
        "SpotifyOnebox" => Some(format!("https://open.spotify.com/oembed?url={url}")),
        "TiktokOnebox" => Some(format!("https://www.tiktok.com/oembed?url={url}")),
        "WistiaOnebox" => Some(format!(
            "https://fast.wistia.com/oembed?embedType=iframe&url={url}"
        )),
        "InstagramOnebox" => Some(if ctx.settings.facebook_token.is_empty() {
            format!(
                "https://api.instagram.com/oembed/?url={}",
                instagram_clean(&url).unwrap_or_default()
            )
        } else {
            format!(
                "https://graph.facebook.com/v9.0/instagram_oembed?url={}&access_token={}",
                instagram_clean(&url).unwrap_or_default(),
                ctx.settings.facebook_token
            )
        }),
        _ => None,
    };
    let e = &mut e;
    match name {
        "AnimatedImageOnebox" => {
            let h = animated_image(e, fetch).await;
            let p = animated_image(e, fetch).await;
            rendered(h, p)
        }
        "AsciinemaOnebox" => {
            let r = asciinema(&url);
            let p = r.as_ref().map(|r| Some(r.1.clone())).map_err(clone_fail);
            rendered(r.map(|r| Some(r.0)), p)
        }
        "AudioboomOnebox" => {
            let h = oembed_html(e, fetch).await;
            let p = oembed_sized_thumbnail(e, fetch, false).await;
            rendered(h, p)
        }
        "ReplitOnebox" => {
            let h = oembed_html(e, fetch).await;
            let p = oembed_sized_thumbnail(e, fetch, true).await;
            rendered(h, p)
        }
        "AudioComOnebox" | "SoundCloudOnebox" => {
            let h = oembed_html_visual(e, fetch).await;
            let p = oembed_thumbnail(e, fetch).await;
            rendered(h, p)
        }
        "SimplecastOnebox" => {
            let h = oembed_html(e, fetch).await;
            let p = oembed_thumbnail(e, fetch).await;
            rendered(h, p)
        }
        "MotokoOnebox" => {
            let h = oembed_html(e, fetch).await;
            rendered(h, Ok(Some(placeholder("generic"))))
        }
        "CoubOnebox" => {
            let h = oembed_html(e, fetch).await;
            let p = coub_placeholder(e, fetch).await;
            rendered(h, p)
        }
        "MixcloudOnebox" => {
            let h = oembed_html(e, fetch).await;
            let p = mixcloud_placeholder(e, fetch).await;
            rendered(h, p)
        }
        "SpotifyOnebox" => {
            let h = oembed_html(e, fetch).await;
            let p = spotify_placeholder(e, fetch).await;
            rendered(h, p)
        }
        "WistiaOnebox" => {
            let h = wistia(e, fetch).await;
            let p = oembed_thumbnail(e, fetch).await;
            rendered(h, p)
        }
        "TiktokOnebox" => {
            let h = tiktok_html(e, fetch).await;
            let p = tiktok_placeholder(e, fetch).await;
            rendered(h, p)
        }
        "BandCampOnebox" => {
            let h = band_camp_html(e, fetch).await;
            let p = band_camp_placeholder(e, fetch).await;
            rendered(h, p)
        }
        "CloudAppOnebox" => {
            let h = cloud_app(e, fetch).await;
            let p = cloud_app(e, fetch).await;
            rendered(h, p)
        }
        "FiveHundredPxOnebox" | "FlickrShortenedOnebox" => {
            let h = og_image(e, fetch, "onebox").await;
            let p = og_image(e, fetch, "onebox").await;
            rendered(h, p)
        }
        "FlickrOnebox" => {
            let h = flickr(e, fetch).await;
            let p = flickr(e, fetch).await;
            rendered(h, p)
        }
        "GooglePhotosOnebox" => {
            let h = google_photos(e, fetch).await;
            let p = google_photos(e, fetch).await;
            rendered(h, p)
        }
        "ImgurOnebox" => {
            let h = imgur(e, fetch).await;
            let p = imgur(e, fetch).await;
            rendered(h, p)
        }
        "KalturaOnebox" => {
            let h = kaltura(e, fetch).await;
            let p = kaltura(e, fetch).await;
            rendered(h, p)
        }
        "SketchFabOnebox" => {
            let h = sketch_fab(e, fetch).await;
            let p = async {
                let og = e.opengraph(fetch).await?;
                Ok(Some(format!("<img src='{}'>", rs(&og.value("image")?)?)))
            }
            .await;
            rendered(h, p)
        }
        "SteamStoreOnebox" => {
            let h = steam_html(&url);
            let p = steam_placeholder(e, fetch).await;
            rendered(h, p)
        }
        "FacebookMediaOnebox" => {
            let h = facebook(e, fetch).await;
            let p = facebook(e, fetch).await;
            rendered(h, p)
        }
        "VimeoOnebox" => {
            let h = vimeo(e, fetch).await;
            rendered(h, Ok(Some(placeholder("video"))))
        }
        "LoomOnebox" => rendered(Ok(Some(loom(&url))), Ok(Some(placeholder("video")))),
        "RestreamOnebox" => rendered(Ok(Some(restream(&url))), Ok(Some(placeholder("video")))),
        "TrelloOnebox" => rendered(trello(&url).map(Some), Ok(Some(placeholder("generic")))),
        "RedditMediaOnebox" => rendered(reddit(&url).map(Some), Ok(Some(placeholder("generic")))),
        "SlidesOnebox" => {
            let h = slides_html(&url).map(Some);
            let p = async {
                let raw = e.raw(fetch).await?;
                let image = rs(raw.get("image").unwrap_or(&Value::Null))?;
                Ok(Some(format!(
                    "<img src='{}'>",
                    normalize_url_for_output(&image)
                )))
            }
            .await;
            rendered(h, p)
        }
        "GoogleDocsOnebox" => {
            let h = google_docs(e, fetch).await;
            let p = google_docs(e, fetch).await;
            rendered(h, p)
        }
        "GoogleDriveOnebox" => {
            let h = google_drive(e, fetch).await;
            let p = google_drive(e, fetch).await;
            rendered(h, p)
        }
        "InstagramOnebox" => {
            let h = instagram_html(e, fetch).await;
            rendered(h, Ok(Some(placeholder("image"))))
        }
        "DiscourseTopicOnebox" => {
            let data = discourse_topic_data(e, fetch).await;
            if let Err(Fail::App(_)) = &data {
                return data.map(|_| Rendered::default());
            }
            let h = data
                .as_ref()
                .map(|d| Some(layout("discoursetopic", DISCOURSE_TOPIC, d)))
                .map_err(clone_fail);
            let mut r = {
                let h2 = h.clone_ok();
                rendered(h, h2)
            }?;
            if let Ok(d) = data {
                r.verified_data = Some(d);
            }
            Ok(r)
        }
        "GithubFolderOnebox" => {
            let data = github_folder_data(e, fetch).await;
            if let Err(Fail::App(_)) = &data {
                return data.map(|_| Rendered::default());
            }
            let h = data
                .as_ref()
                .map(|d| Some(layout("githubfolder", GITHUB_FOLDER, d)))
                .map_err(clone_fail);
            {
                let h2 = h.clone_ok();
                rendered(h, h2)
            }
        }
        "YoutubeOnebox" => {
            let h = youtube_html(e, fetch).await;
            let p = youtube_placeholder(e, fetch).await;
            rendered(h, p)
        }
        _ => Err(Unsupported("onebox engines not ported").into()),
    }
}

fn instagram_clean(url: &str) -> Result<String, Fail> {
    static CLEAN: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^https?://(?:www\.)?(?:instagram\.com|instagr\.am)/?(?:.*)/(?:p|tv)/[a-zA-Z\d_-]+")
            .unwrap()
    });
    CLEAN
        .find(url)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| nil_raised("clean_url"))
}

/// A copy of a failure, for the parts that share one computation.
pub(super) fn clone_fail(f: &Fail) -> Fail {
    match f {
        Fail::Web(m) => Fail::Web(m.clone()),
        Fail::Raised(m) => Fail::Raised(m.clone()),
        Fail::App(_) => Fail::App(Unsupported("a refused onebox engine part").into()),
    }
}

pub(super) trait CloneOk {
    fn clone_ok(&self) -> Self;
}

impl CloneOk for Result<Option<String>, Fail> {
    fn clone_ok(&self) -> Self {
        match self {
            Ok(v) => Ok(v.clone()),
            Err(f) => Err(clone_fail(f)),
        }
    }
}
