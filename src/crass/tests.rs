use super::*;

fn opts(comments: bool, hacks: bool) -> Options {
    Options {
        preserve_comments: comments,
        preserve_hacks: hacks,
    }
}

#[test]
fn tokens_keep_their_original_text() {
    let css = "a { color: #fff; background: url( x.png ) } /* c */ @media (x:1) { b { y: 2px !important } }";
    let tokens = Tokenizer::tokenize(css, opts(true, false));
    let raw: String = tokens
        .iter()
        .filter_map(|t| t.token())
        .map(|t| t.raw.as_str())
        .collect();
    assert_eq!(raw, css);
    assert_eq!(stringify(&parse(css, opts(true, false))), css);
}

#[test]
fn tokenizes_escapes_urls_and_numbers() {
    let tokens = Tokenizer::tokenize(
        "e\\78 pression( url(a\\)b) 1.5em -2% u+00??",
        Options::default(),
    );
    let kinds: Vec<(TokenKind, String)> = tokens
        .iter()
        .filter_map(|t| t.token())
        .filter(|t| t.kind != TokenKind::Whitespace)
        .map(|t| (t.kind.clone(), t.value.clone()))
        .collect();
    assert_eq!(kinds[0], (TokenKind::Function, "expression".into()));
    assert_eq!(kinds[1], (TokenKind::Url, "a)b".into()));
    assert!(
        matches!(&kinds[2].0, TokenKind::Dimension { unit, number } if unit == "em" && number.repr == "1.5")
    );
    assert!(matches!(&kinds[3].0, TokenKind::Percentage(n) if n.repr == "-2"));
    assert_eq!(
        kinds[4].0,
        TokenKind::UnicodeRange {
            start: 0,
            end: 0xff
        }
    );
}

#[test]
fn comments_are_dropped_unless_preserved() {
    let css = "a/* x */b";
    assert_eq!(Tokenizer::tokenize(css, opts(false, false)).len(), 2);
    assert_eq!(Tokenizer::tokenize(css, opts(true, false)).len(), 3);
}

#[test]
fn ie_star_hack_is_an_ident_when_preserved() {
    let tokens = Tokenizer::tokenize("*width", opts(false, true));
    let t = tokens[0].token().expect("token");
    assert_eq!(t.kind, TokenKind::Ident);
    assert_eq!(t.value, "width");
    assert_eq!(t.raw, "*width");
}

#[test]
fn parses_properties_with_important() {
    let props = parse_properties("color: red !important; width : 1px", Options::default());
    let Node::Property(p) = &props[0] else {
        panic!("expected a property: {props:?}");
    };
    assert_eq!(p.name, "color");
    assert_eq!(p.value, "red");
    assert!(p.important);
    let Node::Property(w) = &props[3] else {
        panic!("expected a property: {props:?}");
    };
    assert_eq!(w.value, "1px");
    assert_eq!(stringify(&props), "color: red !important; width : 1px");
}

#[test]
fn deep_nesting_is_discarded_but_serialized() {
    let css = format!("a{{b:{}x{}}}", "(".repeat(30), ")".repeat(30));
    let tree = parse(&css, Options::default());
    assert_eq!(stringify(&tree), css);
}
