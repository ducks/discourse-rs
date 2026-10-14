//! test/test_sanitize_css.rb, test_malicious_css.rb, test_clean_css.rb

use std::sync::Arc;

use super::clean_element::frag;
use crate::crass;
use crate::sanitize::config::relaxed_css;
use crate::sanitize::css::Css;
use crate::sanitize::{Config, CssConfig, set};

fn default() -> Css {
    Css::new(CssConfig::default())
}

fn relaxed() -> Css {
    Css::new(relaxed_css())
}

fn custom_config() -> CssConfig {
    CssConfig {
        properties: set(&["background", "color", "width"]),
        ..CssConfig::default()
    }
}

fn custom() -> Css {
    Css::new(custom_config())
}

#[test]
fn properties_sanitized() {
    let css = r#"background: #fff; width: expression(alert("hi"));"#;
    assert_eq!(default().properties(css), " ");
    assert_eq!(relaxed().properties(css), "background: #fff; ");
    assert_eq!(custom().properties(css), "background: #fff; ");
}

#[test]
fn allowlisted_url_protocols_allowed() {
    for css in [
        "background: url(relative.jpg)",
        "background: url('relative.jpg')",
        "background: url(http://example.com/http.jpg)",
        "background: url('ht\\tp://example.com/http.jpg')",
        "background: url(https://example.com/https.jpg)",
        "background: url('https://example.com/https.jpg')",
        "background: image-set('relative.jpg' 1x, 'relative-2x.jpg' 2x)",
        "background: image-set('https://example.com/https.jpg' 1x, 'https://example.com/https-2x.jpg' 2x)",
        "background: image-set('https://example.com/https.jpg' type('image/jpeg'), 'https://example.com/https.avif' type('image/avif'))",
        "background: -webkit-image-set('relative.jpg' 1x, 'relative-2x.jpg' 2x)",
        "background: -webkit-image-set('https://example.com/https.jpg' 1x, 'https://example.com/https-2x.jpg' 2x)",
        "background: -webkit-image-set('https://example.com/https.jpg' type('image/jpeg'), 'https://example.com/https.avif' type('image/avif'))",
        "background: image('relative.jpg');",
        "background: image('https://example.com/https.jpg');",
        "background: image(rtl 'https://example.com/https.jpg');",
    ] {
        assert_eq!(default().properties(css), "", "{css}");
        assert_eq!(relaxed().properties(css), css, "{css}");
        assert_eq!(custom().properties(css), "", "{css}");
    }
}

#[test]
fn non_allowlisted_url_protocols_removed() {
    for css in [
        "background: url(javascript:alert(0))",
        "background: url(ja\\56 ascript:alert(0))",
        "background: url('javascript:foo')",
        "background: url('ja\\56 ascript:alert(0)')",
        "background: url('ja\\va\\script\\:alert(0)')",
        "background: url('javas\\\ncript:alert(0)')",
        "background: url('java\\0script:foo')",
        "-moz-binding:url('http://ha.ckers.org/xssmoz.xml#xss')",
        "width:expression(alert(1))",
        "width:  /**/expression(alert(1)",
        "width:e\\78 pression(\n\nalert(\n1)",
        "width:\nexpression(alert(1));",
        "xss:expression(alert(1))",
        "height: foo(expression(alert(1)));",
        "behavior: url(xss.htc);",
    ] {
        assert_eq!(default().properties(css), "", "{css}");
        assert_eq!(relaxed().properties(css), "", "{css}");
        assert_eq!(custom().properties(css), "", "{css}");
    }
}

#[test]
fn comments_in_properties() {
    assert_eq!(
        relaxed().properties("color: #fff; /* comment */ width: 100px;"),
        "color: #fff; /* comment */ width: 100px;"
    );
    assert_eq!(
        relaxed().properties("color: #fff; /* \n\ncomment */ width: 100px;"),
        "color: #fff; /* \n\ncomment */ width: 100px;"
    );
    assert_eq!(
        custom().properties("color: #fff; /* comment */ width: 100px;"),
        "color: #fff;  width: 100px;"
    );
    assert_eq!(
        custom().properties("color: #fff; /* \n\ncomment */ width: 100px;"),
        "color: #fff;  width: 100px;"
    );
}

#[test]
fn hacks_in_properties() {
    assert_eq!(
        relaxed().properties("_border: 1px solid #fff; *width: 10px"),
        "_border: 1px solid #fff; *width: 10px"
    );
    assert_eq!(
        custom().properties("_border: 1px solid #fff; *width: 10px"),
        " "
    );
}

const STYLESHEET: &str = "/* Yay CSS! */
          .foo { color: #fff; }
          #bar { background: url(yay.jpg); }

          @media screen (max-width:480px) {
            .foo { width: 400px; }
            #bar:not(.baz) { height: 100px; }
          }";

#[test]
fn stylesheet_sanitized() {
    assert_eq!(
        crate::ruby::strip(&default().stylesheet(STYLESHEET)),
        ".foo {  }\n          #bar {  }"
    );
    assert_eq!(relaxed().stylesheet(STYLESHEET), STYLESHEET);
    assert_eq!(
        crate::ruby::strip(&custom().stylesheet(STYLESHEET)),
        ".foo { color: #fff; }\n          #bar {  }"
    );
}

#[test]
fn stylesheet_comments_and_hacks() {
    for css in [
        ".foo { color: #fff; /* comment */ width: 100px; }",
        ".foo { color: #fff; /* \n\ncomment */ width: 100px; }",
        ".foo { _border: 1px solid #fff; *width: 10px }",
    ] {
        assert_eq!(relaxed().stylesheet(css), css);
    }
    assert_eq!(
        custom().stylesheet(".foo { color: #fff; /* comment */ width: 100px; }"),
        ".foo { color: #fff;  width: 100px; }"
    );
    assert_eq!(
        custom().stylesheet(".foo { color: #fff; /* \n\ncomment */ width: 100px; }"),
        ".foo { color: #fff;  width: 100px; }"
    );
    assert_eq!(
        custom().stylesheet(".foo { _border: 1px solid #fff; *width: 10px }"),
        ".foo {  }"
    );
}

#[test]
fn tree_sanitized() {
    let tree = crass::parse(
        "@import url(foo.css);\n.foo { background: #fff; font: 16pt 'Comic Sans MS'; }\n#bar { top: 125px; background: green; }",
        crass::Options::default(),
    );
    let tree = custom().tree(tree);
    assert_eq!(
        crass::stringify(&tree),
        "\n.foo { background: #fff;  }\n#bar {  background: green; }"
    );
}

#[test]
fn media_rules_parsed() {
    let css = r#"@media { p[class="center"] { text-align: center; }}"#;
    assert_eq!(relaxed().stylesheet(css), css);

    let css = "@media (max-width: 720px) {
          p.foo > .bar { float: right; width: expression(body.scrollLeft + 50 + 'px'); }
          #baz { color: green; }

          @media (orientation: portrait) {
            #baz { color: red; }
          }
        }";
    assert_eq!(
        relaxed().stylesheet(css),
        "@media (max-width: 720px) {
          p.foo > .bar { float: right;  }
          #baz { color: green; }

          @media (orientation: portrait) {
            #baz { color: red; }
          }
        }"
    );
}

#[test]
fn page_rules_parsed() {
    let css = r#"@page { margin: 2cm } /* All margins set to 2cm */

        @page :right {
          @top-center { content: "Preliminary edition" }
          @bottom-center { content: counter(page) }
        }

        @page {
          size: 8.5in 11in;
          margin: 10%;

          @top-left {
            content: "Hamlet";
          }
          @top-right {
            content: "Page " counter(page);
          }
        }"#;
    assert_eq!(relaxed().stylesheet(css), css);
}

#[test]
fn blockless_at_rules() {
    let css = "@charset 'utf-8';
          @import url('foo.css');
          .foo { color: green; }";
    assert_eq!(
        crate::ruby::strip(&relaxed().stylesheet(css)),
        ".foo { color: green; }"
    );

    let allowing = Css::new(CssConfig {
        at_rules: set(&["charset", "import"]),
        ..relaxed_css()
    });
    assert_eq!(allowing.stylesheet(css), css);
    assert_eq!(
        crate::ruby::strip(&allowing.stylesheet(
            "@charset { color: green }
            @import { color: green }
            .foo { color: green; }"
        )),
        ".foo { color: green; }"
    );
}

#[test]
fn container_at_rules_kept() {
    let css = "@container (width > 400px) {
            h2 {
              font-size: 1.5em;
            }
          }

          /* with an optional <container-name> */
          @container tall (height > 30rem) {
            h2 {
              line-height: 1.6;
            }
          }

          /* multiple queries in a single condition */
          @container (width > 400px) and style(--responsive: true) {
            h2 {
              font-size: 1.5em;
            }
          }

          /* condition list */
          @container card (width > 400px), style(--responsive: true) {
            h2 {
              font-size: 1.5em;
            }
          }";
    assert_eq!(crate::ruby::strip(&relaxed().stylesheet(css)), css);
}

#[test]
fn import_urls_validated() {
    let any = Css::new(CssConfig {
        at_rules: set(&["import"]),
        ..relaxed_css()
    });
    let css = "@import url('https://somesite.com/something.css');";
    assert_eq!(crate::ruby::strip(&any.stylesheet(css)), css);

    let google = Css::new(CssConfig {
        at_rules: set(&["import"]),
        import_url_validator: Some(Arc::new(|url: &str| {
            url.starts_with("https://fonts.googleapis.com")
        })),
        ..relaxed_css()
    });
    let css = "@import 'https://fonts.googleapis.com/css?family=Indie+Flower';
              @import url('https://fonts.googleapis.com/css?family=Indie+Flower');";
    assert_eq!(crate::ruby::strip(&google.stylesheet(css)), css);
    let first = "@import 'https://fonts.googleapis.com/css?family=Indie+Flower';";
    assert_eq!(
        crate::ruby::strip(&google.stylesheet(&format!(
            "{first}
              @import 'https://nastysite.com/nasty_hax0r.css';
              @import url('https://nastysite.com/nasty_hax0r.css');"
        ))),
        first
    );
    assert_eq!(
        crate::ruby::strip(&google.stylesheet(&format!(
            "{first}
              @import '';
              @import url('');"
        ))),
        first
    );
}

// test_malicious_css.rb

#[test]
fn malicious_css() {
    let s = relaxed();
    for css in [
        "width:expr/*XSS*/ession(alert('XSS'))",
        "width:ex/*XSS*//*/*/pression(alert(\"XSS\"))",
        "width:\nexpression(alert('XSS'));",
        "background-image:url(\"javascript:alert('XSS')\");",
        "behavior: url(xss.htc);",
    ] {
        assert_eq!(s.properties(css), "", "{css}");
    }
    assert_eq!(
        frag(
            "<div style=\"background-image: url(&#1;javascript:alert('XSS'))\">",
            &Config::relaxed()
        ),
        "<div></div>"
    );
}

#[test]
fn style_element_cannot_be_ended_early() {
    assert_eq!(
        frag(
            "<style>@media</sty/**/le><iframe srcdoc='<script>alert(document.domain)</script>'></style>",
            &Config::relaxed()
        ),
        "<style>@media<\\/style><iframe srcdoc='<script>alert(document.domain)<\\/script>'>{}</style>"
    );
}

// test_clean_css.rb

#[test]
fn style_attributes_sanitized() {
    let c = Config::relaxed();
    assert_eq!(
        frag(
            r#"<div style="color: #fff; width: expression(alert(1)); /* <-- evil! */"></div>"#,
            &c
        ),
        r#"<div style="color: #fff;  /* <-- evil! */"></div>"#
    );
    assert_eq!(
        frag(r#"<div style="width: expression(alert(1))"></div>"#, &c),
        "<div></div>"
    );
}

#[test]
fn style_elements_sanitized() {
    let html = "<style>@import url(evil.css);
      /* Yay CSS! */
      .foo { color: #fff; }
      #bar { background: url(yay.jpg); bogus: wtf; }
      .evil { width: expression(xss()); }

      @media screen (max-width:480px) {
        .foo { width: 400px; }
        #bar:not(.baz) { height: 100px; }
      }
      </style>";
    assert_eq!(
        frag(html, &Config::relaxed()),
        "<style>
      /* Yay CSS! */
      .foo { color: #fff; }
      #bar { background: url(yay.jpg);  }
      .evil {  }

      @media screen (max-width:480px) {
        .foo { width: 400px; }
        #bar:not(.baz) { height: 100px; }
      }
      </style>"
    );
    assert_eq!(frag("<style></style>", &Config::relaxed()), "");
}
