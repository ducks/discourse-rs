//! `PrettyText.excerpt` and lib/excerpt_parser.rb: a cooked post reduced to
//! `length` characters of escaped text, keeping links (and optionally
//! emoji images, newlines, quotes...), with `&hellip;` at the cut.
//!
//! Rails parses the HTML5 fragment, strips lightbox metadata and oneboxed
//! media, re-serializes, then walks it with a SAX parser. The walk here is
//! over the html5ever tree directly; the cooked HTML Discourse writes is
//! well-formed, so the two agree.

use html5ever::tendril::TendrilSink;
use html5ever::{ParseOpts, local_name, namespace_url, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// `ExcerptParser` options (the `options` hash of PrettyText.excerpt).
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub strip_links: bool,
    pub text_entities: bool,
    pub keep_newlines: bool,
    pub keep_emoji_images: bool,
    pub keep_onebox_source: bool,
    pub keep_onebox_body: bool,
    pub keep_quotes: bool,
    pub keep_svg: bool,
    pub remap_emoji: bool,
    pub image_mode: ImageMode,
    pub plain_hashtags: bool,
}

/// `IMAGE_MODES`: the default (no option) shows `[alt]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageMode {
    #[default]
    Alt,
    Strip,
    Markdown,
    Keep,
}

/// `PrettyText.excerpt(html, max_length, options)`
pub fn excerpt(html: &str, max_length: usize, options: &Options) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    let dom = parse(html);
    // get_excerpt: a custom excerpt span lifts the length limit.
    let length = if html.contains("excerpt") && has_custom_excerpt(html) {
        html.chars().count()
    } else {
        max_length
    };
    let mut parser = Parser {
        length,
        excerpt: String::new(),
        current_length: 0,
        options: options.clone(),
        in_a: false,
        in_quote: false,
        in_details_depth: 0,
        in_summary: false,
        in_svg: false,
        start_excerpt: false,
        start_hashtag_icon: false,
    };
    let _ = parser.walk_children(&dom.document);
    let mut out = parser.excerpt.trim().to_string();
    if options.keep_onebox_source || options.keep_onebox_body {
        out = collapse_newlines(&out);
    }
    if options.text_entities {
        out = html_escape::decode_html_entities(&out).into_owned();
    }
    out
}

fn parse(html: &str) -> RcDom {
    let context = html5ever::QualName::new(None, ns!(html), local_name!("body"));
    parse_fragment(RcDom::default(), ParseOpts::default(), context, vec![])
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .expect("reading from a byte slice cannot fail")
}

/// `CUSTOM_EXCERPT_REGEX`: `<span|div ... class="excerpt" ...>`
fn has_custom_excerpt(html: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r#"<\s*(span|div)[^>]*class\s*=\s*['"]excerpt['"][^>]*>"#).unwrap()
    })
    .is_match(html)
}

/// `gsub(/\s*\n+\s*/, "\n\n")`
fn collapse_newlines(s: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\s*\n+\s*").unwrap())
        .replace_all(s, "\n\n")
        .into_owned()
}

/// `ERB::Util.html_escape`
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

struct Done;

struct Parser {
    length: usize,
    excerpt: String,
    current_length: usize,
    options: Options,
    in_a: bool,
    in_quote: bool,
    in_details_depth: usize,
    in_summary: bool,
    in_svg: bool,
    start_excerpt: bool,
    start_hashtag_icon: bool,
}

type Attrs = Vec<(String, String)>;

fn attr<'a>(attrs: &'a Attrs, name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn class_includes(attrs: &Attrs, class: &str) -> bool {
    attr(attrs, "class").is_some_and(|c| c.contains(class))
}

impl Parser {
    fn walk_children(&mut self, node: &Handle) -> Result<(), Done> {
        let children: Vec<Handle> = node.children.borrow().iter().cloned().collect();
        for child in children {
            self.walk(&child)?;
        }
        Ok(())
    }

    fn walk(&mut self, node: &Handle) -> Result<(), Done> {
        match &node.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                self.characters(&text, true, true, true)
            }
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.to_string();
                let attrs: Attrs = attrs
                    .borrow()
                    .iter()
                    .map(|a| (a.name.local.to_string(), a.value.to_string()))
                    .collect();
                // The fragment parser wraps everything in an <html> root.
                if tag == "html" {
                    return self.walk_children(node);
                }
                // strip_image_wrapping / strip_oneboxed_media
                if (tag == "div" && class_includes(&attrs, "meta") && inside_lightbox(node))
                    || tag == "audio"
                    || tag == "video"
                    || class_includes(&attrs, "video-onebox")
                {
                    return Ok(());
                }
                // convert_hashtag_links_to_plaintext
                if self.options.plain_hashtags
                    && tag == "a"
                    && class_includes(&attrs, "hashtag-cooked")
                {
                    let slug = attr(&attrs, "data-slug").unwrap_or("");
                    return self.characters(&format!("#{slug}"), true, true, true);
                }
                if self.start_element(&tag, &attrs)? {
                    self.walk_children(node)?;
                }
                self.end_element(&tag)
            }
            NodeData::Document => self.walk_children(node),
            _ => Ok(()),
        }
    }

    fn include_tag(&mut self, name: &str, attrs: &Attrs) -> Result<(), Done> {
        let rendered: Vec<String> = attrs
            .iter()
            .map(|(k, v)| format!("{k}=\"{}\"", escape_attribute(v)))
            .collect();
        self.characters(
            &format!("<{name} {}>", rendered.join(" ")),
            false,
            false,
            false,
        )
    }

    /// Returns whether the element's children should be walked (images
    /// and the like are leaves either way).
    fn start_element(&mut self, name: &str, attrs: &Attrs) -> Result<bool, Done> {
        match name {
            "img" => {
                if class_includes(attrs, "emoji") {
                    if self.options.remap_emoji {
                        let alt = attr(attrs, "alt").unwrap_or("");
                        let title = alt.replace(':', "");
                        let text = crate::emoji::lookup_unicode(&title)
                            .map(str::to_string)
                            .unwrap_or_else(|| alt.to_string());
                        self.characters(&text, true, true, true)?;
                    } else if self.options.keep_emoji_images {
                        self.include_tag(name, attrs)?;
                    } else {
                        let alt = attr(attrs, "alt").unwrap_or("").to_string();
                        self.characters(&alt, true, true, true)?;
                    }
                    return Ok(false);
                }
                match self.options.image_mode {
                    ImageMode::Keep => self.include_tag(name, attrs)?,
                    ImageMode::Strip => {}
                    mode => {
                        if mode == ImageMode::Markdown {
                            self.characters("!", true, true, true)?;
                        }
                        if let Some(alt) = attr(attrs, "alt").filter(|a| !a.is_empty()) {
                            self.characters(&format!("[{alt}]"), true, true, true)?;
                        } else if let Some(title) = attr(attrs, "title").filter(|t| !t.is_empty()) {
                            self.characters(&format!("[{title}]"), true, true, true)?;
                        } else {
                            self.characters("[image]", true, true, true)?;
                        }
                        if mode == ImageMode::Markdown {
                            let src = attr(attrs, "src").unwrap_or("").to_string();
                            self.characters(&format!("({src})"), true, true, true)?;
                        }
                    }
                }
                Ok(false)
            }
            "a" => {
                if !self.options.strip_links {
                    self.include_tag(name, attrs)?;
                    self.in_a = true;
                }
                Ok(true)
            }
            "aside" => {
                if !(self.options.keep_onebox_source || self.options.keep_onebox_body)
                    || !class_includes(attrs, "onebox")
                {
                    self.in_quote = true;
                }
                if class_includes(attrs, "quote")
                    && (self.options.keep_quotes
                        || (self.options.keep_onebox_body
                            && attr(attrs, "data-topic").is_some_and(|t| !t.is_empty())))
                {
                    self.in_quote = false;
                }
                Ok(true)
            }
            "article" => {
                if attr(attrs, "class") == Some("onebox-body") {
                    self.in_quote = !self.options.keep_onebox_body;
                }
                Ok(true)
            }
            "header" => {
                if attr(attrs, "class") == Some("source") {
                    self.in_quote = !self.options.keep_onebox_source;
                }
                Ok(true)
            }
            "div" | "span" => {
                if class_includes(attrs, "excerpt")
                    && !attr(attrs, "class").is_some_and(|c| c.contains("excerpt hidden"))
                {
                    self.excerpt.clear();
                    self.current_length = 0;
                    self.start_excerpt = true;
                } else if class_includes(attrs, "hashtag-icon-placeholder") {
                    self.start_hashtag_icon = true;
                    self.include_tag(name, attrs)?;
                }
                Ok(true)
            }
            "details" => {
                self.in_details_depth += 1;
                Ok(true)
            }
            "summary" => {
                if self.in_details_depth == 1 && !self.in_summary {
                    self.in_summary = true;
                    self.characters("\u{25b6} ", false, false, false)?;
                }
                Ok(true)
            }
            "svg" => {
                if class_includes(attrs, "d-icon") && self.options.keep_svg {
                    self.include_tag(name, attrs)?;
                    self.in_svg = true;
                }
                Ok(true)
            }
            "use" => {
                if self.in_svg && self.options.keep_svg {
                    self.include_tag(name, attrs)?;
                }
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    fn end_element(&mut self, name: &str) -> Result<(), Done> {
        match name {
            "a" => {
                if !self.options.strip_links {
                    self.characters("</a>", false, false, false)?;
                    self.in_a = false;
                }
            }
            "p" | "br" => {
                if self.options.keep_newlines {
                    self.characters("<br>", false, false, false)?;
                } else {
                    self.characters(" ", true, true, true)?;
                }
            }
            "aside" => self.in_quote = false,
            "details" => self.in_details_depth -= 1,
            "summary" => {
                if self.in_details_depth == 1 {
                    self.in_summary = false;
                }
            }
            "div" | "span" => {
                if self.start_excerpt {
                    return Err(Done);
                }
                if self.start_hashtag_icon {
                    self.characters("</span>", false, false, false)?;
                }
            }
            "svg" => {
                if self.options.keep_svg {
                    self.characters("</svg>", false, false, false)?;
                }
                self.in_svg = false;
            }
            "use" => {
                if self.options.keep_svg {
                    self.characters("</use>", false, false, false)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn characters(
        &mut self,
        string: &str,
        truncate: bool,
        count_it: bool,
        encode: bool,
    ) -> Result<(), Done> {
        if self.in_quote
            || self.in_details_depth > 1
            || (self.in_details_depth == 1 && !self.in_summary)
        {
            return Ok(());
        }
        let encode = |s: &str| {
            if encode {
                html_escape(s)
            } else {
                s.to_string()
            }
        };
        let len = string.chars().count();
        if count_it && self.current_length + len > self.length {
            let length = self.length.saturating_sub(self.current_length + 1);
            if truncate && !is_emoji(string) {
                // string[0..length] is inclusive of `length`.
                let head: String = string.chars().take(length + 1).collect();
                self.excerpt.push_str(&encode(&head));
            }
            self.excerpt.push_str(if self.options.text_entities {
                "..."
            } else {
                "&hellip;"
            });
            if self.in_a {
                self.excerpt.push_str("</a>");
            }
            return Err(Done);
        }
        self.excerpt.push_str(&encode(string));
        if count_it {
            self.current_length += len;
        }
        Ok(())
    }
}

/// `/\A:\w+:\Z/`
fn is_emoji(s: &str) -> bool {
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.len() > 2
        && s.starts_with(':')
        && s.ends_with(':')
        && s[1..s.len() - 1]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
}

fn escape_attribute(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('"', "&#34;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `.lightbox-wrapper .meta`: a descendant of a lightbox wrapper.
fn inside_lightbox(node: &Handle) -> bool {
    let mut current: Option<Handle> = node.parent.take().and_then(|w| {
        let strong = w.upgrade();
        node.parent.set(Some(w));
        strong
    });
    while let Some(n) = current {
        if let NodeData::Element { attrs, .. } = &n.data {
            let classes: Vec<String> = attrs
                .borrow()
                .iter()
                .filter(|a| a.name.local.to_string() == "class")
                .map(|a| a.value.to_string())
                .collect();
            if classes
                .iter()
                .any(|c| c.split_whitespace().any(|w| w == "lightbox-wrapper"))
            {
                return true;
            }
        }
        current = n.parent.take().and_then(|w| {
            let strong = w.upgrade();
            n.parent.set(Some(w));
            strong
        });
    }
    false
}

/// `Nokogiri::HTML5.fragment(html).text`: every text node concatenated.
pub fn fragment_text(html: &str) -> String {
    fn collect(node: &Handle, out: &mut String) {
        match &node.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            _ => {
                for child in node.children.borrow().iter() {
                    collect(child, out);
                }
            }
        }
    }
    let mut out = String::new();
    collect(&parse(html).document, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(html: &str, len: usize) -> String {
        excerpt(html, len, &Options::default())
    }

    #[test]
    fn plain_paragraphs() {
        assert_eq!(plain("<p>hello world</p>", 300), "hello world");
        // The newline between paragraphs is a text node of its own.
        assert_eq!(plain("<p>one</p>\n<p>two</p>", 300), "one \ntwo");
        assert_eq!(plain("", 300), "");
        assert_eq!(plain("<p>a &amp; b &lt; c</p>", 300), "a &amp; b &lt; c");
    }

    #[test]
    fn truncates_with_an_ellipsis() {
        assert_eq!(plain("<p>abcdefghij</p>", 5), "abcde&hellip;");
        // The paragraph's trailing space is what crosses the limit.
        assert_eq!(plain("<p>abcde</p>", 5), "abcde &hellip;");
        // Inside a link the closing tag is appended after the cut.
        assert_eq!(
            plain("<p>see <a href=\"/x\">this link</a></p>", 6),
            "see <a href=\"/x\">th&hellip;</a>"
        );
    }

    #[test]
    fn keeps_links_and_images() {
        assert_eq!(
            plain(
                "<p><a href=\"https://x.test/?a=1&b=2\" rel=\"noopener\">x</a></p>",
                300
            ),
            "<a href=\"https://x.test/?a=1&amp;b=2\" rel=\"noopener\">x</a>"
        );
        let opts = Options {
            strip_links: true,
            ..Options::default()
        };
        assert_eq!(excerpt("<p><a href=\"/x\">x</a> y</p>", 300, &opts), "x y");
        assert_eq!(
            plain("<p><img src=\"/i.png\" alt=\"pic\"></p>", 300),
            "[pic]"
        );
        assert_eq!(plain("<p><img src=\"/i.png\"></p>", 300), "[image]");
        let opts = Options {
            keep_emoji_images: true,
            ..Options::default()
        };
        assert_eq!(
            excerpt(
                "<p>hi <img src=\"/e.png\" title=\":wave:\" class=\"emoji\" alt=\":wave:\"></p>",
                300,
                &opts
            ),
            "hi <img src=\"/e.png\" title=\":wave:\" class=\"emoji\" alt=\":wave:\">"
        );
        assert_eq!(
            plain(
                "<p>hi <img src=\"/e.png\" class=\"emoji\" alt=\":wave:\"></p>",
                300
            ),
            "hi :wave:"
        );
    }

    #[test]
    fn quotes_details_and_custom_excerpts() {
        assert_eq!(
            plain(
                "<aside class=\"quote\"><blockquote><p>quoted</p></blockquote></aside><p>mine</p>",
                300
            ),
            "mine"
        );
        let opts = Options {
            keep_quotes: true,
            ..Options::default()
        };
        assert_eq!(
            excerpt(
                "<aside class=\"quote\"><blockquote><p>quoted</p></blockquote></aside><p>mine</p>",
                300,
                &opts
            ),
            "quoted mine"
        );
        assert_eq!(
            plain(
                "<details><summary>sum</summary><p>hidden</p></details><p>after</p>",
                300
            ),
            "\u{25b6} sumafter"
        );
        assert_eq!(
            plain(
                "<p>intro</p><div class=\"excerpt\"><p>the excerpt</p></div><p>more</p>",
                5
            ),
            "the excerpt"
        );
        let opts = Options {
            keep_newlines: true,
            ..Options::default()
        };
        assert_eq!(excerpt("<p>a</p><p>b</p>", 300, &opts), "a<br>b<br>");
    }
}
