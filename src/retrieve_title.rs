//! lib/retrieve_title.rb: another site's page title, read from the
//! beginning of the page (FinalDestination#get, one second at a time),
//! for its inline onebox.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use sqlx::PgConnection;

use crate::AppError;
use crate::final_destination::{Fetcher, Options, Read, Uri};
use crate::pretty_text::cleanup::{attr, element_name, parse_document, text};

/// `RetrieveTitle::CRAWL_TIMEOUT`
const CRAWL_TIMEOUT: Duration = Duration::from_secs(1);

static AMAZON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|\.)amazon\.(com|ca|co\.uk|es|fr|de|it|com\.au|com\.br|cn|in|co\.jp|com\.mx)$")
        .unwrap()
});
static YOUTUBE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|\.)youtube\.com$|(^|\.)youtu\.be$").unwrap());
static GITHUB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\.)github\.com$").unwrap());
static DOCUMENT_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"document\.title *= *"(.*)";"#).unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" +").unwrap());

/// `max_chunk_size(uri)`, in kilobytes: sites that leave the title late
/// are read further.
fn max_chunk_size(uri: &Uri) -> usize {
    if AMAZON.is_match(&uri.host) {
        500
    } else if YOUTUBE.is_match(&uri.host) {
        300
    } else if GITHUB.is_match(&uri.host) {
        50
    } else {
        20
    }
}

/// `RetrieveTitle.extract_title`: the `<title>`'s text (YouTube's
/// document.title for "YouTube"), else og:title, its whitespace folded.
pub fn extract_title(html: &str) -> Option<String> {
    if html.contains("<title>") && !html.contains("</title>") {
        return None;
    }
    let dom = parse_document(html);
    let elements = all_elements_of(&dom);
    let mut title = elements
        .iter()
        .find(|e| element_name(e) == Some("title"))
        .map(text);
    if title.as_deref() == Some("YouTube")
        && let Some(c) = DOCUMENT_TITLE.captures(html)
    {
        title = Some(c[1].strip_suffix(" - YouTube").unwrap_or(&c[1]).to_string());
    }
    if title.is_none() {
        title = elements
            .iter()
            .find(|e| {
                element_name(e) == Some("meta")
                    && attr(e, "property").as_deref() == Some("og:title")
            })
            .and_then(|e| attr(e, "content"));
    }
    let title = title?;
    if title.trim().is_empty() {
        return None;
    }
    let title = title.replace('\n', " ");
    Some(SPACES.replace_all(&title, " ").trim().to_string())
}

fn all_elements_of(dom: &markup5ever_rcdom::RcDom) -> Vec<markup5ever_rcdom::Handle> {
    let mut out = Vec::new();
    let mut stack = vec![dom.document.clone()];
    while let Some(node) = stack.pop() {
        if element_name(&node).is_some() {
            out.push(node.clone());
        }
        for child in node.children.borrow().iter().rev() {
            stack.push(child.clone());
        }
    }
    out
}

/// The page's charset from its content type, if it names one Ruby knows;
/// only UTF-8 (or ASCII) pages are read here.
fn readable(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else {
        return true;
    };
    let ct = ct.trim().to_lowercase();
    match ct.split("charset=").nth(1) {
        Some(cs) => {
            let cs = cs.trim_matches(|c| c == '"' || c == ' ' || c == ';');
            matches!(cs, "utf-8" | "utf8" | "us-ascii" | "ascii")
        }
        None => true,
    }
}

/// `RetrieveTitle.crawl(url, max_redirects:, headers:)`: the title, none
/// when the page can't be read or has none.
pub async fn crawl(
    conn: &mut PgConnection,
    fetcher: &Fetcher<'_>,
    url: &str,
    max_redirects: u32,
    https_redirect_ignore_limit: bool,
    headers: Vec<(String, String)>,
) -> Result<Option<String>, AppError> {
    let mut headers = headers;
    headers.push(("Accept".into(), "text/html,*/*".into()));
    let opts = Options {
        timeout: CRAWL_TIMEOUT,
        max_redirects,
        https_redirect_ignore_limit,
        stop_at_blocked_pages: true,
        headers,
    };
    let read = fetcher
        .get(
            conn,
            url,
            &opts,
            |uri| max_chunk_size(uri) * 1024,
            |body| extract_title(&String::from_utf8_lossy(body)).is_some(),
        )
        .await?;
    match read {
        Read::Page {
            body, content_type, ..
        } => {
            if !readable(content_type.as_deref()) {
                return Err(
                    crate::Unsupported("titles of pages in a charset other than UTF-8").into(),
                );
            }
            Ok(extract_title(&String::from_utf8_lossy(&body)))
        }
        Read::Nothing | Read::Failed { .. } => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_titles_as_rails() {
        assert_eq!(
            extract_title("<html><head><title>\n  Hello\n   world </title></head></html>"),
            Some("Hello world".into())
        );
        assert_eq!(extract_title("<title>Partial"), None);
        assert_eq!(
            extract_title(r#"<meta property="og:title" content="From OG">"#),
            Some("From OG".into())
        );
        assert_eq!(
            extract_title(
                r#"<title>YouTube</title><script>document.title = "A video - YouTube";</script>"#
            ),
            Some("A video".into())
        );
        assert_eq!(extract_title("<p>no title</p>"), None);
    }
}
