//! A document fed in pieces must give the same tokens as the whole document, as early
//! as possible and in linear time.

use ron::html::TokenKind::{Comment, EndTag, RawText, StartTag, Text};
use ron::html::{StreamTokenizer, Token, TokenKind, Tokenizer};

fn drain(stream: &mut StreamTokenizer) -> Vec<(TokenKind, String)> {
    let mut result = Vec::new();
    while let Some(token) = stream.next_token() {
        result.push((token.kind, stream.source()[token.span].to_owned()));
    }
    result
}

fn owned(expected: &[(TokenKind, &str)]) -> Vec<(TokenKind, String)> {
    expected
        .iter()
        .map(|(kind, text)| (*kind, (*text).to_owned()))
        .collect()
}

/// Joins adjacent pieces of text and raw text, which only streaming may split.
fn merge(tokens: Vec<Token>) -> Vec<Token> {
    let mut merged: Vec<Token> = Vec::new();
    for token in tokens {
        match merged.last_mut() {
            Some(last)
                if last.kind == token.kind
                    && matches!(token.kind, Text | RawText)
                    && last.span.end == token.span.start =>
            {
                last.span.end = token.span.end;
            }
            _ => merged.push(token),
        }
    }
    merged
}

/// Feeds the source in pieces of at most `size` bytes, cut at character boundaries.
fn stream_tokens(source: &str, mut size: impl FnMut() -> usize) -> Vec<Token> {
    let mut stream = StreamTokenizer::new();
    let mut tokens = Vec::new();
    let mut rest = source;
    while !rest.is_empty() {
        let mut end = size().clamp(1, rest.len());
        while !rest.is_char_boundary(end) {
            end += 1;
        }
        let (piece, tail) = rest.split_at(end);
        stream.feed(piece);
        rest = tail;
        tokens.extend(std::iter::from_fn(|| stream.next_token()));
    }
    stream.finish();
    tokens.extend(std::iter::from_fn(|| stream.next_token()));
    assert_eq!(stream.source(), source);
    tokens
}

#[test]
fn tokens_appear_as_soon_as_they_are_final() {
    let mut stream = StreamTokenizer::new();
    stream.feed("<p>Hel");
    assert_eq!(
        drain(&mut stream),
        owned(&[(StartTag, "<p>"), (Text, "Hel")])
    );
    stream.feed("lo <");
    assert_eq!(drain(&mut stream), owned(&[(Text, "lo ")]));
    stream.feed("b");
    assert_eq!(drain(&mut stream), []);
    stream.feed(">w</");
    assert_eq!(drain(&mut stream), owned(&[(StartTag, "<b>"), (Text, "w")]));
    stream.feed("b");
    assert_eq!(drain(&mut stream), []);
    stream.feed(">");
    assert_eq!(drain(&mut stream), owned(&[(EndTag, "</b>")]));
    stream.finish();
    assert_eq!(drain(&mut stream), []);
}

#[test]
fn unfinished_tag_waits_and_finish_completes_it() {
    let mut stream = StreamTokenizer::new();
    stream.feed("x<a href=\"y>z");
    assert_eq!(drain(&mut stream), owned(&[(Text, "x")]));
    stream.feed("\"");
    assert_eq!(drain(&mut stream), []);
    stream.feed(" b");
    assert_eq!(drain(&mut stream), []);
    stream.finish();
    assert_eq!(
        drain(&mut stream),
        owned(&[(StartTag, "<a href=\"y>z\" b")])
    );
}

#[test]
fn comment_waits_for_its_end() {
    let mut stream = StreamTokenizer::new();
    stream.feed("<!-- a --");
    assert_eq!(drain(&mut stream), []);
    stream.feed("!");
    assert_eq!(drain(&mut stream), []);
    stream.feed(">x");
    assert_eq!(
        drain(&mut stream),
        owned(&[(Comment, "<!-- a --!>"), (Text, "x")])
    );
}

#[test]
fn raw_text_holds_back_only_a_possible_end_tag() {
    let mut stream = StreamTokenizer::new();
    stream.feed("<title>a<b");
    assert_eq!(
        drain(&mut stream),
        owned(&[(StartTag, "<title>"), (RawText, "a<b")])
    );
    stream.feed("</ti");
    assert_eq!(drain(&mut stream), []);
    stream.feed("x>c</title");
    assert_eq!(drain(&mut stream), owned(&[(RawText, "</tix>c")]));
    stream.feed(">y");
    assert_eq!(
        drain(&mut stream),
        owned(&[(EndTag, "</title>"), (Text, "y")])
    );
}

#[test]
fn plaintext_is_returned_as_it_arrives() {
    let mut stream = StreamTokenizer::new();
    stream.feed("<plaintext>a<");
    assert_eq!(
        drain(&mut stream),
        owned(&[(StartTag, "<plaintext>"), (RawText, "a<")])
    );
    stream.feed("/plaintext>");
    assert_eq!(drain(&mut stream), owned(&[(RawText, "/plaintext>")]));
}

#[test]
fn every_split_gives_the_same_tokens_as_the_whole_document() {
    let samples = [
        "<p>Hello <b>wor</b>ld</p>\r\n",
        "a < b <3 </ </> </ x> <!DOCTYPE html><?xml?>",
        "<a b=\"x>y\" c='z' d=e>t<a =x><a b==c>",
        "<body background=\"black.jpg\"\">ż€😀",
        "<!-- a > b --->x<!-->y<!--->z<!-- c --!>w<!-- open -",
        "<SCRIPT src=\"a>b\">x</scriptx>y</Script >z<style>a</style",
        "<title>a<<b></title></title><textarea></textarea>",
        "<plaintext>a</plaintext>b",
        "<a href=\"unterminated",
    ];
    for source in samples {
        let whole: Vec<Token> = Tokenizer::new(source).collect();
        for size in 1..=source.len() {
            assert_eq!(
                merge(stream_tokens(source, || size)),
                whole,
                "{source:?} by {size}"
            );
        }
    }
}

#[test]
fn random_markup_in_random_pieces_matches_the_whole_document() {
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
    let mut seed = 4321_u32;
    let mut next = move |limit: usize| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 16) as usize % limit
    };
    for _ in 0..3000 {
        let count = next(60);
        let source: String = (0..count).map(|_| ATOMS[next(ATOMS.len())]).collect();
        let whole: Vec<Token> = Tokenizer::new(&source).collect();
        assert_eq!(
            merge(stream_tokens(&source, || next(12))),
            whole,
            "{source:?}"
        );
    }
}

/// A rescan on every piece would make these quadratic: hours instead of a moment.
#[test]
fn byte_by_byte_input_stays_fast_while_tokens_wait() {
    let size = 300_000;
    for (opening, filler) in [
        ("<a b=\"", "x"),
        ("<a ", "b "),
        ("<!--", "-"),
        ("<!-- ", "x"),
        ("<script>", "<"),
        ("<script>", "</scrip"),
        ("<!", "x"),
        ("", "<"),
    ] {
        let source = opening.to_owned() + &filler.repeat(size / filler.len());
        let tokens = stream_tokens(&source, || 1);
        assert_eq!(
            tokens.last().map(|token| token.span.end),
            Some(source.len())
        );
    }
}

#[test]
#[should_panic(expected = "feed called after finish")]
fn feeding_after_finish_is_a_bug_in_the_caller() {
    let mut stream = StreamTokenizer::new();
    stream.finish();
    stream.feed("x");
}
