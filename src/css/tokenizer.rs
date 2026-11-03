//! CSS tokens, from the DFA generated from `grammars/css.ag`.

use std::sync::LazyLock;

use super::css_parser::LEXER_TABLES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CssTokenKind {
    Whitespace,
    Comment,
    Ident,
    Function,
    Url,
    AtKeyword,
    Hash,
    String,
    Number,
    Percentage,
    Dimension,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    LeftBracket,
    RightBracket,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    Dot,
    Greater,
    Plus,
    Tilde,
    Star,
    Equals,
    Includes,
    DashMatch,
    PrefixMatch,
    SuffixMatch,
    SubstringMatch,
    Delim,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CssToken {
    pub kind: CssTokenKind,
    /// The terminal of the grammar.
    pub(super) terminal: u32,
    pub start: u32,
    pub end: u32,
}

impl CssToken {
    #[must_use]
    pub fn text(self, source: &str) -> &str {
        &source[self.start as usize..self.end as usize]
    }
}

fn kind_of_rule(name: &str) -> CssTokenKind {
    use CssTokenKind as K;
    match name {
        "WS" => K::Whitespace,
        "COMMENT" => K::Comment,
        "IDENT" => K::Ident,
        "FUNCTION" => K::Function,
        "URL" => K::Url,
        "AT_KEYWORD" => K::AtKeyword,
        "HASH" => K::Hash,
        "STRING" => K::String,
        "NUMBER" => K::Number,
        "PERCENTAGE" => K::Percentage,
        "DIMENSION" => K::Dimension,
        "CDO" => K::Cdo,
        "CDC" => K::Cdc,
        "COLON" => K::Colon,
        "SEMICOLON" => K::Semicolon,
        "COMMA" => K::Comma,
        "LBRACKET" => K::LeftBracket,
        "RBRACKET" => K::RightBracket,
        "LPAREN" => K::LeftParen,
        "RPAREN" => K::RightParen,
        "LBRACE" => K::LeftBrace,
        "RBRACE" => K::RightBrace,
        "DOT" => K::Dot,
        "GT" => K::Greater,
        "PLUS" => K::Plus,
        "TILDE" => K::Tilde,
        "STAR" => K::Star,
        "EQUALS" => K::Equals,
        "INCLUDES" => K::Includes,
        "DASH_MATCH" => K::DashMatch,
        "PREFIX_MATCH" => K::PrefixMatch,
        "SUFFIX_MATCH" => K::SuffixMatch,
        "SUBSTRING_MATCH" => K::SubstringMatch,
        _ => K::Delim,
    }
}

static RULE_KINDS: LazyLock<Vec<CssTokenKind>> = LazyLock::new(|| {
    LEXER_TABLES
        .rules
        .iter()
        .map(|rule| kind_of_rule(rule.name))
        .collect()
});

/// Splits a stylesheet into tokens. Accepts every input and runs in linear time.
///
/// # Panics
///
/// Panics for sources of 4 GiB or more, which stylesheet parsing does not accept.
#[must_use]
pub fn tokenize(source: &str) -> Vec<CssToken> {
    assert!(u32::try_from(source.len()).is_ok(), "stylesheet too large");
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < source.len() {
        let (rule, end) = longest_match(source, offset);
        let table = &LEXER_TABLES.rules[rule];
        tokens.push(CssToken {
            kind: RULE_KINDS[rule],
            terminal: table.terminal.unwrap_or(0),
            start: offset as u32,
            end: end as u32,
        });
        offset = end;
    }
    tokens
}

fn longest_match(source: &str, offset: usize) -> (usize, usize) {
    let mut state = 0_usize;
    let mut matched = None;
    for (index, character) in source[offset..].char_indices() {
        let point = u32::from(character);
        let Some(transition) =
            LEXER_TABLES.dfa_states[state]
                .transitions
                .iter()
                .find(|transition| {
                    transition
                        .ranges
                        .iter()
                        .any(|range| range.first <= point && point <= range.last)
                })
        else {
            break;
        };
        state = transition.target as usize;
        if let Some(rule) = LEXER_TABLES.dfa_states[state].accepting_rule {
            matched = Some((rule as usize, offset + index + character.len_utf8()));
        }
    }
    // The last rule takes any single character, so this only guards against a defect in
    // the generated tables.
    matched.unwrap_or_else(|| {
        let first = source[offset..].chars().next().map_or(1, char::len_utf8);
        (LEXER_TABLES.rules.len() - 1, offset + first)
    })
}

/// Reads an identifier, string content or hash name, resolving its escapes.
pub(super) fn unescape(text: &str) -> String {
    if !text.contains('\\') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        let Some(&next) = characters.peek() else {
            break;
        };
        if next.is_ascii_hexdigit() {
            let mut value = 0_u32;
            for _ in 0..6 {
                let Some(digit) = characters.peek().and_then(|c| c.to_digit(16)) else {
                    break;
                };
                value = value * 16 + digit;
                characters.next();
            }
            if characters.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                characters.next();
            }
            let decoded = char::from_u32(value).filter(|c| *c != '\0');
            out.push(decoded.unwrap_or('\u{fffd}'));
        } else if matches!(next, '\n' | '\r' | '\x0c') {
            // An escaped line end inside a string joins the lines.
            characters.next();
            if next == '\r' && characters.peek() == Some(&'\n') {
                characters.next();
            }
        } else {
            out.push(next);
            characters.next();
        }
    }
    out
}
