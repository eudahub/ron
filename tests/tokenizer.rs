//! Tokenizer and attribute reader behavior on broken and hostile markup.

use ron::html::TokenKind::{Comment, Declaration, EndTag, RawText, StartTag, StrayLt, Text};
use ron::html::{TokenKind, Tokenizer, attributes, tag_name};

fn tokens(source: &str) -> Vec<(TokenKind, &str)> {
    Tokenizer::new(source)
        .map(|token| (token.kind, &source[token.span]))
        .collect()
}

fn attrs(tag: &str) -> Vec<(&str, Option<&str>)> {
    attributes(tag)
        .map(|attribute| (attribute.name, attribute.value))
        .collect()
}

/// Spans must follow each other without gaps and cover the whole source.
fn assert_covers(source: &str) {
    let mut expected = 0;
    for token in Tokenizer::new(source) {
        assert_eq!(token.span.start, expected, "gap in {source:?}");
        assert!(
            token.span.end > token.span.start,
            "empty token in {source:?}"
        );
        let text = &source[token.span.clone()];
        if matches!(token.kind, StartTag | EndTag) {
            let _ = tag_name(text);
            let _ = attributes(text).count();
        }
        expected = token.span.end;
    }
    assert_eq!(expected, source.len(), "uncovered end of {source:?}");
}

#[test]
fn empty_input_has_no_tokens() {
    assert_eq!(tokens(""), []);
}

#[test]
fn splits_tags_and_text() {
    assert_eq!(
        tokens("<p>a</p >\r\n"),
        [
            (StartTag, "<p>"),
            (Text, "a"),
            (EndTag, "</p >"),
            (Text, "\r\n")
        ]
    );
}

#[test]
fn less_than_without_a_name_is_not_markup() {
    assert_eq!(
        tokens("a < b <3"),
        [
            (Text, "a "),
            (StrayLt, "<"),
            (Text, " b "),
            (StrayLt, "<"),
            (Text, "3")
        ]
    );
    assert_eq!(tokens("</"), [(StrayLt, "<"), (Text, "/")]);
}

#[test]
fn end_of_input_inside_a_tag_ends_the_tag() {
    for source in [
        "<a",
        "<a ",
        "<a href",
        "<a href=",
        "<a href=x",
        "<a href=\"x",
        "<a href='x>y",
        "<a/",
    ] {
        assert_eq!(tokens(source), [(StartTag, source)]);
    }
    assert_eq!(tokens("</p"), [(EndTag, "</p")]);
}

#[test]
fn greater_than_inside_quotes_does_not_end_a_tag() {
    assert_eq!(
        tokens("<a b=\"x>y\">t"),
        [(StartTag, "<a b=\"x>y\">"), (Text, "t")]
    );
    assert_eq!(
        tokens("<a b='x>y'>t"),
        [(StartTag, "<a b='x>y'>"), (Text, "t")]
    );
    assert_eq!(
        tokens("<a b = \"x>y\">t"),
        [(StartTag, "<a b = \"x>y\">"), (Text, "t")]
    );
}

#[test]
fn quote_outside_a_value_does_not_open_a_string() {
    let tag = "<body background=\"black.jpg\"\">";
    assert_eq!(tokens(&format!("{tag}x")), [(StartTag, tag), (Text, "x")]);
    assert_eq!(tokens("<a \"b>x"), [(StartTag, "<a \"b>"), (Text, "x")]);
}

#[test]
fn recognizes_declarations() {
    for source in [
        "<!DOCTYPE html>",
        "<!>",
        "<!-x>",
        "<!->",
        "<?xml version=\"1.0\"?>",
        "</>",
        "</ x>",
        "<!DOCTYPE",
    ] {
        assert_eq!(tokens(source), [(Declaration, source)]);
    }
}

#[test]
fn finds_the_end_of_comments() {
    for comment in [
        "<!-- a > b -->",
        "<!---->",
        "<!-->",
        "<!--->",
        "<!-- a --->",
        "<!-- a --!>",
    ] {
        assert_eq!(
            tokens(&format!("{comment}x")),
            [(Comment, comment), (Text, "x")]
        );
    }
    for unterminated in ["<!--", "<!-- a", "<!-- a -", "<!-- a --", "<!-- a --!"] {
        assert_eq!(tokens(unterminated), [(Comment, unterminated)]);
    }
}

#[test]
fn raw_text_elements_hold_no_markup() {
    assert_eq!(
        tokens("<script>if(a<b)x=\"</p>\";</script>y"),
        [
            (StartTag, "<script>"),
            (RawText, "if(a<b)x=\"</p>\";"),
            (EndTag, "</script>"),
            (Text, "y"),
        ]
    );
    assert_eq!(
        tokens("<title>a<b>ż</title>"),
        [
            (StartTag, "<title>"),
            (RawText, "a<b>ż"),
            (EndTag, "</title>")
        ]
    );
    assert_eq!(
        tokens("<style></style>"),
        [(StartTag, "<style>"), (EndTag, "</style>")]
    );
}

#[test]
fn raw_text_end_tag_is_case_insensitive_and_needs_a_delimiter() {
    assert_eq!(
        tokens("<SCRIPT src=\"a>b\">x</scriptx>y</Script >"),
        [
            (StartTag, "<SCRIPT src=\"a>b\">"),
            (RawText, "x</scriptx>y"),
            (EndTag, "</Script >"),
        ]
    );
    assert_eq!(
        tokens("<style>a</style"),
        [(StartTag, "<style>"), (RawText, "a</style")]
    );
}

#[test]
fn unterminated_raw_text_runs_to_the_end() {
    assert_eq!(
        tokens("<style>a<b"),
        [(StartTag, "<style>"), (RawText, "a<b")]
    );
    assert_eq!(
        tokens("<plaintext>a</plaintext>b"),
        [(StartTag, "<plaintext>"), (RawText, "a</plaintext>b")]
    );
}

#[test]
fn similar_names_are_not_raw_text() {
    assert_eq!(
        tokens("<scriptx><b>"),
        [(StartTag, "<scriptx>"), (StartTag, "<b>")]
    );
}

#[test]
fn reads_tag_names() {
    assert_eq!(tag_name("<IMG SRC=x>"), "IMG");
    assert_eq!(tag_name("</p >"), "p");
    assert_eq!(tag_name("<br/>"), "br");
    assert_eq!(tag_name("<a"), "a");
    assert_eq!(tag_name("<aż>"), "aż");
}

#[test]
fn reads_unquoted_and_quoted_attributes() {
    assert_eq!(
        attrs("<IMG SRC=ab150.gif align=right border=2>"),
        [
            ("SRC", Some("ab150.gif")),
            ("align", Some("right")),
            ("border", Some("2"))
        ]
    );
    assert_eq!(
        attrs("<a b=\"x y\" c='z' d e = f>"),
        [
            ("b", Some("x y")),
            ("c", Some("z")),
            ("d", None),
            ("e", Some("f"))
        ]
    );
    assert_eq!(
        attrs("<a b=\"ż\" ć=1>"),
        [("b", Some("ż")), ("ć", Some("1"))]
    );
    assert_eq!(attrs("<p>"), []);
}

#[test]
fn slash_ends_a_name_but_not_an_unquoted_value() {
    assert_eq!(
        attrs("<a href=http://x/y/>"),
        [("href", Some("http://x/y/"))]
    );
    assert_eq!(attrs("<a b/c>"), [("b", None), ("c", None)]);
    assert_eq!(attrs("<br/>"), []);
}

#[test]
fn reads_attributes_of_broken_tags() {
    assert_eq!(attrs("<a =x>"), [("=x", None)]);
    assert_eq!(attrs("<a b=>"), [("b", Some(""))]);
    assert_eq!(attrs("<a b="), [("b", Some(""))]);
    assert_eq!(attrs("<a b=\"x"), [("b", Some("x"))]);
    assert_eq!(attrs("<a b==c>"), [("b", Some("=c"))]);
    assert_eq!(
        attrs("<body background=\"black.jpg\"\">"),
        [("background", Some("black.jpg")), ("\"", None)]
    );
}

#[test]
fn random_markup_is_covered_without_gaps() {
    const ATOMS: [&str; 30] = [
        "<",
        ">",
        "/",
        "=",
        "\"",
        "'",
        "-",
        "--",
        "!",
        "?",
        " ",
        "\n",
        "a",
        "ż",
        "€",
        "😀",
        "\0",
        "<script",
        "</script",
        "<style>",
        "</style>",
        "<!--",
        "-->",
        "--!>",
        "<!",
        "</",
        "<a ",
        "b=\"c",
        "<title",
        "<plaintext>",
    ];
    // A fixed linear congruential sequence keeps failures reproducible.
    let mut seed = 1234_u32;
    let mut next = |limit: usize| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 16) as usize % limit
    };
    for _ in 0..5000 {
        let source: String = (0..next(60)).map(|_| ATOMS[next(ATOMS.len())]).collect();
        assert_covers(&source);
    }
}

#[test]
fn long_repetitions_are_covered_without_gaps() {
    for unit in [
        "<", "<a ", "<a b=\"", "<script>", "<!--", "</", "<!-", "-", "ż",
    ] {
        assert_covers(&unit.repeat(100_000 / unit.len()));
    }
}
