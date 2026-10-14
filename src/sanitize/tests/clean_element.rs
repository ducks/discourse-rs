//! test/test_clean_element.rb

use std::collections::HashMap;

use crate::sanitize::{AttrAllow, Config, Protocols, RemoveContents, document, fragment, set};

pub(super) fn frag(html: &str, config: &Config) -> String {
    fragment(html, config).expect("sanitizes")
}

pub(super) fn doc(html: &str, config: &Config) -> String {
    document(html, config).expect("sanitizes")
}

/// html, default, restricted, basic, relaxed
type Case = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

const STRINGS: &[Case] = &[
    (
        r#"<b>Lo<!-- comment -->rem</b> <a href="pants" title="foo" style="text-decoration: underline;">ipsum</a> <a href="http://foo.com/"><strong>dolor</strong></a> sit<br/>amet <style>.foo { color: #fff; }</style> <script>alert("hello world");</script>"#,
        "Lorem ipsum dolor sit amet  ",
        "<b>Lorem</b> ipsum <strong>dolor</strong> sit amet  ",
        r#"<b>Lorem</b> <a href="pants" rel="nofollow">ipsum</a> <a href="http://foo.com/" rel="nofollow"><strong>dolor</strong></a> sit<br>amet  "#,
        r#"<b>Lorem</b> <a href="pants" title="foo" style="text-decoration: underline;">ipsum</a> <a href="http://foo.com/"><strong>dolor</strong></a> sit<br>amet <style>.foo { color: #fff; }</style> "#,
    ),
    (
        r#"Lo<!-- comment -->rem</b> <a href=pants title="foo>ipsum <a href="http://foo.com/"><strong>dolor</a></strong> sit<br/>amet <script>alert("hello world");"#,
        "Lorem dolor sit amet ",
        "Lorem <strong>dolor</strong> sit amet ",
        r#"Lorem <a href="pants" rel="nofollow"><strong>dolor</strong></a> sit<br>amet "#,
        r#"Lorem <a href="pants" title="foo>ipsum <a href="><strong>dolor</strong></a> sit<br>amet "#,
    ),
    (
        "<p>a</p><blockquote>b",
        " a  b ",
        " a  b ",
        "<p>a</p><blockquote>b</blockquote>",
        "<p>a</p><blockquote>b</blockquote>",
    ),
    (
        r#"<b>Lo<!-- comment -->rem</b> <a href="javascript:pants" title="foo">ipsum</a> <a href="http://foo.com/"><strong>dolor</strong></a> sit<br/>amet <<foo>script>alert("hello world");</script>"#,
        r#"Lorem ipsum dolor sit amet &lt;script&gt;alert("hello world");"#,
        r#"<b>Lorem</b> ipsum <strong>dolor</strong> sit amet &lt;script&gt;alert("hello world");"#,
        r#"<b>Lorem</b> <a rel="nofollow">ipsum</a> <a href="http://foo.com/" rel="nofollow"><strong>dolor</strong></a> sit<br>amet &lt;script&gt;alert("hello world");"#,
        r#"<b>Lorem</b> <a title="foo">ipsum</a> <a href="http://foo.com/"><strong>dolor</strong></a> sit<br>amet &lt;script&gt;alert("hello world");"#,
    ),
];

const A_FOO: Case = (
    "",
    "foo",
    "foo",
    r#"<a rel="nofollow">foo</a>"#,
    "<a>foo</a>",
);
const IMG: Case = ("", "", "", "", "<img>");

fn protocols() -> Vec<Case> {
    let a = |html| (html, A_FOO.1, A_FOO.2, A_FOO.3, A_FOO.4);
    let img = |html| (html, IMG.1, IMG.2, IMG.3, IMG.4);
    vec![
        a(r#"<a href="javascript:alert('XSS');">foo</a>"#),
        a(r#"<a href="javascript    :alert('XSS');">foo</a>"#),
        a(r#"<a href="javascript:    alert('XSS');">foo</a>"#),
        a(r#"<a href="javascript    :   alert('XSS');">foo</a>"#),
        a(r#"<a href=":javascript:alert('XSS');">foo</a>"#),
        a(r#"<a href="javascript&#58;">foo</a>"#),
        a(r#"<a href="javascript&#0058;">foo</a>"#),
        a(
            "<a href=&#0000106&#0000097&#0000118&#0000097&#0000115&#0000099&#0000114&#0000105&#0000112&#0000116&#0000058&#0000097&#0000108&#0000101&#0000114&#0000116&#0000040&#0000039&#0000088&#0000083&#0000083&#0000039&#0000041>foo</a>",
        ),
        a(r#"<a href="javascript&#x3A;">foo</a>"#),
        a(r#"<a href="javascript&#x003A;">foo</a>"#),
        a(
            "<a href=&#x6A&#x61&#x76&#x61&#x73&#x63&#x72&#x69&#x70&#x74&#x3A&#x61&#x6C&#x65&#x72&#x74&#x28&#x27&#x58&#x53&#x53&#x27&#x29>foo</a>",
        ),
        img("<img src=java\0script:alert(\"XSS\")>"),
        img(r#"<img src=java\script:alert("XSS")>"#),
        img(r#"<img src=" &#14;  javascript:alert('XSS');">"#),
        (
            r#"<a href=" http://example.com/"></a>"#,
            "",
            "",
            r#"<a href="http://example.com/" rel="nofollow"></a>"#,
            r#"<a href="http://example.com/"></a>"#,
        ),
    ]
}

fn check_all(column: usize, config: &Config) {
    for case in STRINGS.iter().copied().chain(protocols()) {
        let expected = match column {
            1 => case.1,
            2 => case.2,
            3 => case.3,
            _ => case.4,
        };
        assert_eq!(frag(case.0, config), expected, "input: {}", case.0);
    }
}

#[test]
fn default_removes_non_allowlisted_elements_leaving_safe_contents() {
    let c = Config::default();
    assert_eq!(
        frag(
            r##"foo <b>bar</b> <strong><a href="#a">baz</a></strong> quux"##,
            &c
        ),
        "foo bar baz quux"
    );
    assert_eq!(frag(r#"<script>alert("<xss>");</script>"#, &c), "");
    assert_eq!(
        frag(r#"<<script>script>alert("<xss>");</<script>>"#, &c),
        "&lt;"
    );
    assert_eq!(
        frag(r#"< script <>> alert("<xss>");</script>"#, &c),
        r#"&lt; script &lt;&gt;&gt; alert("");"#
    );
}

#[test]
fn default_surrounds_whitespace_elements_with_spaces() {
    let c = Config::default();
    assert_eq!(frag("foo<div>bar</div>baz", &c), "foo bar baz");
    assert_eq!(frag("foo<br>bar<br>baz", &c), "foo bar baz");
    assert_eq!(frag("foo<hr>bar<hr>baz", &c), "foo bar baz");
}

#[test]
fn default_handles_repeated_elements() {
    let img = r#"<img src="http://www.google.com/intl/en_ALL/images/logo.gif">"#;
    assert_eq!(frag(&img.repeat(4), &Config::default()), "");
}

#[test]
fn default_drops_contents_of_removed_unsafe_elements() {
    let c = Config::default();
    for tag in [
        "iframe", "math", "noembed", "noframes", "noscript", "script", "style", "svg", "xmp",
    ] {
        assert_eq!(
            frag(
                &format!("<{tag}>hello! <script>alert(0)</script></{tag}>"),
                &c
            ),
            "",
            "{tag}"
        );
    }
    assert_eq!(frag("<plaintext>hello! <script>alert(0)</script>", &c), "");
}

#[test]
fn default_config_cases() {
    check_all(1, &Config::default());
}

#[test]
fn restricted_config_cases() {
    check_all(2, &Config::restricted());
}

#[test]
fn basic_config_cases() {
    check_all(3, &Config::basic());
}

#[test]
fn relaxed_config_cases() {
    check_all(4, &Config::relaxed());
}

#[test]
fn basic_handles_valueless_and_uppercase_attributes() {
    let c = Config::basic();
    assert_eq!(
        frag("foo <a href>foo</a> bar", &c),
        r#"foo <a href="" rel="nofollow">foo</a> bar"#
    );
    assert_eq!(
        frag(r#"<a HREF="javascript:alert('foo')">bar</a>"#, &c),
        r#"<a rel="nofollow">bar</a>"#
    );
}

#[test]
fn relaxed_encodes_special_chars_in_attribute_values() {
    assert_eq!(
        frag(
            r#"<a href="http://example.com" title="<b>&eacute;xamples</b> & things">foo</a>"#,
            &Config::relaxed()
        ),
        r#"<a href="http://example.com" title="<b>éxamples</b> &amp; things">foo</a>"#
    );
}

fn elements(names: &[&str]) -> Config {
    Config {
        elements: set(names),
        ..Config::default()
    }
}

#[test]
fn attributes_allowed_under_all() {
    let input = r#"<p class="foo">bar</p>"#;
    assert_eq!(frag(input, &Config::default()), " bar ");

    let mut c = elements(&["p"]);
    c.attributes_all = Some(AttrAllow::new(&["class"]));
    assert_eq!(frag(input, &c), input);

    let mut c = elements(&["p"]);
    c.attributes
        .insert("div".into(), AttrAllow::new(&["class"]));
    assert_eq!(frag(input, &c), "<p>bar</p>");

    let mut c = elements(&["p"]);
    c.attributes.insert("p".into(), AttrAllow::new(&["title"]));
    c.attributes_all = Some(AttrAllow::new(&["class"]));
    assert_eq!(frag(input, &c), input);
}

fn links(protocols: Protocols) -> Config {
    let mut c = elements(&["a"]);
    c.attributes.insert("a".into(), AttrAllow::new(&["href"]));
    c.set_protocols("a", "href", protocols);
    c
}

#[test]
fn relative_urls_need_relative_allowlisted() {
    assert_eq!(
        frag(
            r#"<a href="/foo/bar">Link</a>"#,
            &links(Protocols::new(&["http"]))
        ),
        "<a>Link</a>"
    );
}

#[test]
fn relative_urls_with_colons_after_the_first_segment_or_in_anchors() {
    let c = links(Protocols::default().relative());
    for input in [
        r#"<a href="/wiki/Special:Random">Random Page</a>"#,
        r##"<a href="#fn:1">Footnote 1</a>"##,
        r#"<a href="somepage#fn:1">Footnote 1</a>"#,
    ] {
        assert_eq!(frag(input, &c), input);
    }
}

#[test]
fn remove_contents_true() {
    let c = Config {
        remove_contents: RemoveContents::All(true),
        ..Config::default()
    };
    assert_eq!(
        frag("foo bar <div>baz<span>quux</span></div>", &c),
        "foo bar   "
    );
}

#[test]
fn remove_contents_of_listed_elements() {
    let c = Config {
        remove_contents: RemoveContents::Elements(set(&["script", "span"])),
        ..Config::default()
    };
    assert_eq!(
        frag(
            r#"foo bar <div>baz<span>quux</span> <b>hi</b><script>alert("hello!");</script></div>"#,
            &c
        ),
        "foo bar  baz hi "
    );
}

#[test]
fn allowlisted_iframes_lose_their_contents() {
    assert_eq!(
        frag(
            "<iframe>hi <script>hello</script></iframe>",
            &elements(&["iframe"])
        ),
        "<iframe></iframe>"
    );
}

#[test]
fn data_attributes_not_allowed_by_default() {
    assert_eq!(
        frag(r#"<b data-foo="bar"></b>"#, &elements(&["b"])),
        "<b></b>"
    );
    let mut c = elements(&["b"]);
    c.attributes.insert("b".into(), AttrAllow::new(&["class"]));
    assert_eq!(
        frag(r#"<b class="foo" data-foo="bar"></b>"#, &c),
        r#"<b class="foo"></b>"#
    );
}

#[test]
fn data_attributes_allowed_with_data() {
    let mut c = elements(&["b"]);
    c.attributes.insert("b".into(), AttrAllow::data());
    assert_eq!(
        frag(r#"<b data-foo="valid" data-bar="valid"></b>"#, &c),
        r#"<b data-foo="valid" data-bar="valid"></b>"#
    );
    for input in [
        r#"<b data-="invalid"></b>"#,
        r#"<b data-xml="invalid"></b>"#,
        r#"<b data-xmlfoo="invalid"></b>"#,
        r#"<b data-f:oo="valid"></b>"#,
        r#"<b data-éfoo="valid"></b>"#,
    ] {
        assert_eq!(frag(input, &c), "<b></b>", "{input}");
    }
    assert_eq!(
        frag(r#"<b data-f/oo="partial"></b>"#, &c),
        r#"<b data-f=""></b>"#
    );
}

#[test]
fn whitespace_elements_with_configured_text() {
    let mut c = Config::default();
    for e in ["p", "div", "br"] {
        c.whitespace_elements
            .insert(e.into(), ("\n".into(), "\n".into()));
    }
    assert_eq!(frag("<p>foo</p>", &c), "\nfoo\n");
    assert_eq!(frag("<p>foo</p><p>bar</p>", &c), "\nfoo\n\nbar\n");
    assert_eq!(frag("foo<div>bar</div>baz", &c), "foo\nbar\nbaz");
    assert_eq!(frag("foo<br>bar<br>baz", &c), "foo\nbar\nbaz");
}

#[test]
fn protocols_regardless_of_case() {
    let c = links(Protocols::new(&["https"]));
    let input = r#"<a href="hTTpS://foo.com/">Text</a>"#;
    assert_eq!(frag(input, &c), input);
    assert_eq!(
        frag(
            r#"<a href="mailto:someone@example.com?Subject=Hello">Text</a>"#,
            &c
        ),
        "<a>Text</a>"
    );
}

#[test]
fn protocols_in_allowed_data_attributes() {
    let input = r#"<a data-url="mailto:someone@example.com">Text</a>"#;
    let mut c = elements(&["a"]);
    c.attributes.insert("a".into(), AttrAllow::data());
    c.set_protocols("a", "data-url", Protocols::new(&["https"]));
    assert_eq!(frag(input, &c), "<a>Text</a>");
    c.set_protocols("a", "data-url", Protocols::new(&["mailto"]));
    assert_eq!(frag(input, &c), input);
}

fn meta(elements_list: &[&str], attrs: &[&str]) -> Config {
    let mut c = elements(elements_list);
    c.attributes = HashMap::from([("meta".to_string(), AttrAllow::new(attrs))]);
    c
}

#[test]
fn meta_charset_forced_to_utf8() {
    assert_eq!(
        doc(
            r#"<html><head><meta charset="utf-8"></head><body>Howdy!</body></html>"#,
            &meta(&["html", "head", "meta", "body"], &["charset"])
        ),
        r#"<html><head><meta charset="utf-8"></head><body>Howdy!</body></html>"#
    );
    let c = meta(&["html", "meta"], &["charset"]);
    assert_eq!(
        doc(r#"<html><meta charset="utf-8">Howdy!</html>"#, &c),
        r#"<html><meta charset="utf-8">Howdy!</html>"#
    );
    assert_eq!(
        doc(r#"<html><meta charset="us-ascii">Howdy!</html>"#, &c),
        r#"<html><meta charset="utf-8">Howdy!</html>"#
    );
    let c = meta(&["html", "meta"], &["content", "http-equiv"]);
    assert_eq!(
        doc(
            r#"<html><meta http-equiv="content-type" content=" text/html; charset=us-ascii">Howdy!</html>"#,
            &c
        ),
        r#"<html><meta http-equiv="content-type" content=" text/html;charset=utf-8">Howdy!</html>"#
    );
    assert_eq!(
        doc(
            r#"<html><meta http-equiv="Content-Type" content="text/plain;charset = us-ascii">Howdy!</html>"#,
            &c
        ),
        r#"<html><meta http-equiv="Content-Type" content="text/plain;charset=utf-8">Howdy!</html>"#
    );
}

#[test]
fn meta_already_utf8_untouched() {
    let input = r#"<html><head><meta http-equiv="Content-Type" content="text/html;charset=utf-8"></head><body>Howdy!</body></html>"#;
    assert_eq!(
        doc(
            input,
            &meta(
                &["html", "head", "meta", "body"],
                &["content", "http-equiv"]
            )
        ),
        input
    );
}

#[test]
fn noscript_always_removed() {
    assert_eq!(
        frag("<noscript>foo</noscript>", &elements(&["noscript"])),
        ""
    );
}
