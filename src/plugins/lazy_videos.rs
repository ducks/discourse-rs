//! discourse-lazy-videos: YouTube, Vimeo and TikTok oneboxes as a
//! thumbnail the client swaps for the player (its
//! lib/discourse_lazy_videos/lazy_*.rb `to_html` overrides). The onebox
//! engines call these where the plugin's settings are on; the data comes
//! from the engine.

use crate::discourse_diff::escape_html;

/// LazyYoutube's HTML.
pub fn youtube_html(
    video_id: &str,
    title: &str,
    start_time: Option<&str>,
    list_id: Option<&str>,
    thumbnail_url: &str,
) -> String {
    let escaped_title = escape_html(title);
    let escaped_start = escape_html(start_time.unwrap_or(""));
    let escaped_list = escape_html(list_id.unwrap_or(""));
    let t_param = if escaped_start.trim().is_empty() {
        String::new()
    } else {
        format!("&t={escaped_start}")
    };
    let list_param = if escaped_list.trim().is_empty() {
        String::new()
    } else {
        format!("&list={escaped_list}")
    };
    format!(
        "<div class=\"youtube-onebox lazy-video-container\"\n  data-video-id=\"{video_id}\"\n  data-video-title=\"{escaped_title}\"\n  data-video-start-time=\"{escaped_start}\"\n  data-video-list-id=\"{escaped_list}\"\n  data-provider-name=\"youtube\">\n  <a href=\"https://www.youtube.com/watch?v={video_id}{list_param}{t_param}\" target=\"_blank\" class=\"video-thumbnail\">\n    <img class=\"youtube-thumbnail\"\n      src=\"{thumbnail_url}\"\n      title=\"{escaped_title}\">\n  </a>\n</div>\n"
    )
}

/// LazyVimeo's HTML.
pub fn vimeo_html(iframe_id: &str, full_video_id: &str, title: &str, image: &str) -> String {
    let escaped_title = escape_html(title);
    format!(
        "<div class=\"vimeo-onebox lazy-video-container\"\n  data-video-id=\"{iframe_id}\"\n  data-video-title=\"{escaped_title}\"\n  data-provider-name=\"vimeo\">\n  <a href=\"https://vimeo.com/{full_video_id}\" target=\"_blank\">\n    <img class=\"vimeo-thumbnail\"\n      src=\"{image}\"\n      title=\"{escaped_title}\">\n  </a>\n</div>\n"
    )
}

/// LazyTiktok's HTML.
pub fn tiktok_html(video_id: &str, title: &str, url: &str, thumbnail_url: &str) -> String {
    let escaped_title = escape_html(title);
    format!(
        "<div class=\"tiktok-onebox lazy-video-container\"\n  data-video-id=\"{video_id}\"\n  data-video-title=\"{escaped_title}\"\n  data-provider-name=\"tiktok\">\n  <a href=\"{url}\" target=\"_blank\">\n    <img class=\"tiktok-thumbnail\"\n      src=\"{thumbnail_url}\"\n      title=\"{escaped_title}\">\n  </a>\n</div>\n"
    )
}
