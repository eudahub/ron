//! Streaming HTML tokenizer driven by the DFA generated from `grammars/html.ag`.

use std::ops::Range;

use super::html_parser::LEXER_TABLES;

/// Elements whose content is text up to the matching end tag, not markup.
const RAW_TEXT_ELEMENTS: [&str; 8] = [
    "script", "style", "title", "textarea", "xmp", "iframe", "noembed", "noframes",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TokenKind {
    Text,
    /// A `<` that does not start markup.
    StrayLt,
    Comment,
    /// `<!...>`, `<?...>` and `</ ...>`.
    Declaration,
    StartTag,
    EndTag,
    /// The content of a raw-text element.
    RawText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    /// Byte range in the source.
    pub span: Range<usize>,
}

/// Splits a document into tokens. Accepts every input and runs in linear time.
pub struct Tokenizer<'a> {
    source: &'a str,
    offset: usize,
    raw_text_end: Option<usize>,
}

impl<'a> Tokenizer<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            raw_text_end: None,
        }
    }

    /// Runs the DFA from the current offset; returns the rule and end of the longest match.
    fn longest_match(&self) -> Option<(&'static str, usize)> {
        let mut state = 0_usize;
        let mut matched = None;
        for (index, character) in self.source[self.offset..].char_indices() {
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
                let end = self.offset + index + character.len_utf8();
                matched = Some((LEXER_TABLES.rules[rule as usize].name, end));
            }
        }
        matched
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        if let Some(end) = self.raw_text_end.take()
            && end > self.offset
        {
            let span = self.offset..end;
            self.offset = end;
            return Some(Token {
                kind: TokenKind::RawText,
                span,
            });
        }
        if self.offset >= self.source.len() {
            return None;
        }
        // The grammar has a rule for every first character; the fallback only guards
        // against a defect in the generated tables.
        let start = self.offset;
        let fallback = || {
            let width = self.source[start..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
            ("TEXT", start + width)
        };
        let (rule, mut end) = self.longest_match().unwrap_or_else(fallback);
        let kind = match rule {
            "STRAY_LT" => TokenKind::StrayLt,
            "COMMENT_OPEN" => {
                end = comment_end(self.source.as_bytes(), end);
                TokenKind::Comment
            }
            "DECLARATION" => TokenKind::Declaration,
            "START_TAG" => {
                self.raw_text_end = raw_text_end(self.source, start..end);
                TokenKind::StartTag
            }
            "END_TAG" => TokenKind::EndTag,
            _ => TokenKind::Text,
        };
        self.offset = end;
        Some(Token {
            kind,
            span: start..end,
        })
    }
}

/// Returns the name of a start or end tag as written in the source.
#[must_use]
pub fn tag_name(tag: &str) -> &str {
    let name = tag
        .strip_prefix("</")
        .or(tag.strip_prefix('<'))
        .unwrap_or(tag);
    let end = name
        .find(|c: char| is_html_space(c) || c == '/' || c == '>')
        .unwrap_or(name.len());
    &name[..end]
}

pub(super) fn is_html_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\r' | '\n' | '\x0c')
}

/// Finds where a comment ends, given the offset just after `<!--`.
fn comment_end(source: &[u8], body: usize) -> usize {
    let rest = &source[body..];
    if rest.starts_with(b">") {
        return body + 1;
    }
    if rest.starts_with(b"->") {
        return body + 2;
    }
    let mut index = 0;
    while index + 1 < rest.len() {
        if rest[index] == b'-' && rest[index + 1] == b'-' {
            if rest[index + 2..].starts_with(b">") {
                return body + index + 3;
            }
            if rest[index + 2..].starts_with(b"!>") {
                return body + index + 4;
            }
        }
        index += 1;
    }
    source.len()
}

/// For a start tag of a raw-text element, finds where its content ends.
fn raw_text_end(source: &str, tag: Range<usize>) -> Option<usize> {
    let name = tag_name(&source[tag.clone()]);
    if name.eq_ignore_ascii_case("plaintext") {
        return Some(source.len());
    }
    if !RAW_TEXT_ELEMENTS
        .iter()
        .any(|raw| name.eq_ignore_ascii_case(raw))
    {
        return None;
    }
    let bytes = source.as_bytes();
    let mut index = tag.end;
    while index + 2 + name.len() < bytes.len() {
        let after_name = index + 2 + name.len();
        if bytes[index] == b'<'
            && bytes[index + 1] == b'/'
            && bytes[index + 2..after_name].eq_ignore_ascii_case(name.as_bytes())
            && matches!(
                bytes[after_name],
                b' ' | b'\t' | b'\r' | b'\n' | 0x0c | b'/' | b'>'
            )
        {
            return Some(index);
        }
        index += 1;
    }
    Some(source.len())
}
