//! HTML tokenizer driven by the DFA generated from `grammars/html.ag`.
//!
//! [`Tokenizer`] reads a complete document. [`StreamTokenizer`] takes it piece by
//! piece and returns each token as soon as later input can no longer change it.

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

/// Splits a complete document into tokens. Accepts every input and runs in linear time.
pub struct Tokenizer<'a> {
    source: &'a str,
    scanner: Scanner,
}

impl<'a> Tokenizer<'a> {
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            scanner: Scanner::default(),
        }
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        self.scanner.next(self.source, true)
    }
}

/// Splits a document that arrives in pieces, keeping the text received so far.
///
/// Tokens returned early never change, and spans stay valid as input grows, so a tree
/// built from them can be shown at any moment. Text and raw text are returned as far as
/// they have arrived, so one run may come as several adjacent tokens. Other tokens wait
/// until they are complete. The total work is linear whatever the sizes of the pieces.
#[derive(Default)]
pub struct StreamTokenizer {
    source: String,
    scanner: Scanner,
    finished: bool,
}

impl StreamTokenizer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves the buffer up front when the document size is known.
    #[must_use]
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            source: String::with_capacity(bytes),
            ..Self::default()
        }
    }

    /// Appends the next piece of the document.
    ///
    /// # Panics
    ///
    /// Panics when called after [`Self::finish`].
    pub fn feed(&mut self, piece: &str) {
        assert!(!self.finished, "feed called after finish");
        self.source.push_str(piece);
    }

    /// Marks the end of the document, which completes any token still waiting.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Returns the next finished token. `None` means more input is needed or, after
    /// [`Self::finish`], that the document has ended.
    pub fn next_token(&mut self) -> Option<Token> {
        self.scanner.next(&self.source, self.finished)
    }

    /// The text received so far; token spans refer to it.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// A DFA run over the token at the current offset, resumed when more input arrives.
#[derive(Default)]
struct DfaScan {
    state: usize,
    cursor: usize,
    matched: Option<(&'static str, usize)>,
}

enum Mode {
    Markup(DfaScan),
    Comment {
        body: usize,
        search: usize,
    },
    /// Raw text up to the end tag with the name at this source range.
    RawText(Range<usize>),
    /// `<plaintext>`: raw text that no end tag can close.
    PlainText,
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Markup(DfaScan::default())
    }
}

#[derive(Default)]
struct Scanner {
    offset: usize,
    mode: Mode,
}

impl Scanner {
    /// Returns the next token that is final, given whether `source` is the whole input.
    fn next(&mut self, source: &str, complete: bool) -> Option<Token> {
        loop {
            match &mut self.mode {
                Mode::Markup(scan) => {
                    if self.offset >= source.len() {
                        return None;
                    }
                    let (rule, end) = match run_dfa(scan, source, complete) {
                        DfaOutcome::Waiting => return None,
                        DfaOutcome::Matched(rule, end) => (rule, end),
                        // The grammar has a rule for every first character; this only
                        // guards against a defect in the generated tables.
                        DfaOutcome::NoMatch => {
                            let first = source[self.offset..].chars().next();
                            ("TEXT", self.offset + first.map_or(1, char::len_utf8))
                        }
                    };
                    let start = self.offset;
                    let kind = match rule {
                        "STRAY_LT" => TokenKind::StrayLt,
                        "DECLARATION" => TokenKind::Declaration,
                        "END_TAG" => TokenKind::EndTag,
                        "START_TAG" => TokenKind::StartTag,
                        "COMMENT_OPEN" => {
                            self.offset = end;
                            self.mode = Mode::Comment {
                                body: end,
                                search: end,
                            };
                            continue;
                        }
                        _ => TokenKind::Text,
                    };
                    self.mode = if kind == TokenKind::StartTag {
                        mode_after_start_tag(source, start..end)
                    } else {
                        Mode::Markup(DfaScan {
                            cursor: end,
                            ..DfaScan::default()
                        })
                    };
                    self.offset = end;
                    return Some(Token {
                        kind,
                        span: start..end,
                    });
                }
                Mode::Comment { body, search } => {
                    let end = comment_end(source.as_bytes(), *body, search, complete)?;
                    // The offset already moved past `<!--`, which is four bytes.
                    let start = *body - 4;
                    self.offset = end;
                    self.mode = Mode::Markup(DfaScan {
                        cursor: end,
                        ..DfaScan::default()
                    });
                    return Some(Token {
                        kind: TokenKind::Comment,
                        span: start..end,
                    });
                }
                Mode::RawText(name) => {
                    let name = &source[name.clone()];
                    let (end, closed) =
                        raw_text_end(source.as_bytes(), self.offset, name, complete);
                    if let Some(token) = self.finish_raw_step(end, closed) {
                        return Some(token);
                    }
                    if !closed {
                        return None;
                    }
                }
                Mode::PlainText => {
                    if let Some(token) = self.finish_raw_step(source.len(), complete) {
                        return Some(token);
                    }
                    if !complete {
                        return None;
                    }
                }
            }
        }
    }

    /// Moves past raw text ending at `end`, returning it as a token unless it is empty.
    fn finish_raw_step(&mut self, end: usize, closed: bool) -> Option<Token> {
        let start = self.offset;
        self.offset = end;
        if closed {
            self.mode = Mode::Markup(DfaScan {
                cursor: end,
                ..DfaScan::default()
            });
        }
        (end > start).then_some(Token {
            kind: TokenKind::RawText,
            span: start..end,
        })
    }
}

enum DfaOutcome {
    /// Later input could still change the token.
    Waiting,
    Matched(&'static str, usize),
    NoMatch,
}

/// Advances the DFA as far as the input allows.
fn run_dfa(scan: &mut DfaScan, source: &str, complete: bool) -> DfaOutcome {
    let mut stopped = false;
    let start = scan.cursor;
    for (index, character) in source[start..].char_indices() {
        let point = u32::from(character);
        let Some(transition) =
            LEXER_TABLES.dfa_states[scan.state]
                .transitions
                .iter()
                .find(|transition| {
                    transition
                        .ranges
                        .iter()
                        .any(|range| range.first <= point && point <= range.last)
                })
        else {
            stopped = true;
            break;
        };
        scan.state = transition.target as usize;
        scan.cursor = start + index + character.len_utf8();
        if let Some(rule) = LEXER_TABLES.dfa_states[scan.state].accepting_rule {
            scan.matched = Some((LEXER_TABLES.rules[rule as usize].name, scan.cursor));
        }
    }
    stopped |= LEXER_TABLES.dfa_states[scan.state].transitions.is_empty();
    match scan.matched {
        // Text is final as far as it goes: what follows only adds another piece.
        Some((rule, end)) if stopped || complete || rule == "TEXT" => {
            DfaOutcome::Matched(rule, end)
        }
        None if stopped || complete => DfaOutcome::NoMatch,
        _ => DfaOutcome::Waiting,
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

/// Finds where a comment ends, given the offset just after `<!--`. Returns `None` while
/// the end has not arrived; `search` keeps the place so that nothing is scanned twice.
fn comment_end(source: &[u8], body: usize, search: &mut usize, complete: bool) -> Option<usize> {
    let rest = &source[body..];
    if rest.starts_with(b">") {
        return Some(body + 1);
    }
    if rest.starts_with(b"->") {
        return Some(body + 2);
    }
    if !complete && (rest.is_empty() || rest == b"-") {
        return None;
    }
    let mut index = *search;
    while index + 1 < source.len() {
        if source[index] == b'-' && source[index + 1] == b'-' {
            if source[index + 2..].starts_with(b">") {
                return Some(index + 3);
            }
            if source[index + 2..].starts_with(b"!>") {
                return Some(index + 4);
            }
        }
        index += 1;
    }
    if complete {
        return Some(source.len());
    }
    // The longest end sequence, `--!>`, may have its first three bytes here already.
    *search = source.len().saturating_sub(3).max(body);
    None
}

/// Chooses how to continue after a start tag.
fn mode_after_start_tag(source: &str, tag: Range<usize>) -> Mode {
    let name = tag_name(&source[tag.clone()]);
    if name.eq_ignore_ascii_case("plaintext") {
        return Mode::PlainText;
    }
    if RAW_TEXT_ELEMENTS
        .iter()
        .any(|raw| name.eq_ignore_ascii_case(raw))
    {
        let name_start = tag.start + 1;
        return Mode::RawText(name_start..name_start + name.len());
    }
    Mode::Markup(DfaScan {
        cursor: tag.end,
        ..DfaScan::default()
    })
}

/// Finds how far raw text reaches from `offset`, and whether its end tag or the end of
/// input was reached. While waiting, holds back what could be the start of the end tag.
fn raw_text_end(source: &[u8], offset: usize, name: &str, complete: bool) -> (usize, bool) {
    let mut index = offset;
    while index + 2 + name.len() < source.len() {
        let after_name = index + 2 + name.len();
        if source[index] == b'<'
            && source[index + 1] == b'/'
            && source[index + 2..after_name].eq_ignore_ascii_case(name.as_bytes())
            && matches!(
                source[after_name],
                b' ' | b'\t' | b'\r' | b'\n' | 0x0c | b'/' | b'>'
            )
        {
            return (index, true);
        }
        index += 1;
    }
    if complete {
        return (source.len(), true);
    }
    let end_tag_start = source.len().saturating_sub(2 + name.len()).max(offset);
    let held = (end_tag_start..source.len()).find(|&start| {
        let received = &source[start..];
        received[0] == b'<'
            && received.get(1).is_none_or(|&byte| byte == b'/')
            && received
                .get(2..)
                .is_none_or(|rest| rest.eq_ignore_ascii_case(&name.as_bytes()[..rest.len()]))
    });
    (held.unwrap_or(source.len()), false)
}
