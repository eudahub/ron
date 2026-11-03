//! Stylesheets, read as CSS Syntax requires: errors drop one rule or declaration, and
//! the input ending inside a block closes it.

use super::media::{Viewport, matches_media};
use super::selector::Selector;
use super::tokenizer::{CssToken, CssTokenKind as K, tokenize, unescape};

/// Deeper `@media` nesting is skipped, which bounds repeated scans of the same tokens.
const MAX_AT_RULE_NESTING: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Declaration {
    /// Lowercase, except custom properties, which keep their case.
    pub name: Box<str>,
    /// Byte range of the value in the stylesheet source.
    value_start: u32,
    value_end: u32,
    pub important: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// Index into [`Stylesheet::media_queries`] of the `@media` condition around this
    /// rule, the innermost one when they nest.
    pub media: Option<u32>,
}

/// What the at-rules around a rule say of it.
#[derive(Clone, Copy, Default)]
struct Place {
    media: Option<u32>,
    in_layer: bool,
}

/// A parsed stylesheet. It keeps its source, which declarations refer to.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Stylesheet {
    source: String,
    pub rules: Vec<StyleRule>,
    /// The conditions of `@media` rules, as written.
    pub media_queries: Vec<Box<str>>,
    /// For each condition, the one around it when they nest.
    media_parents: Vec<Option<u32>>,
    /// The targets of `@import` rules, which the caller may fetch.
    pub imports: Vec<Box<str>>,
}

impl Stylesheet {
    /// Parses a stylesheet. Accepts every input; what is invalid is left out.
    ///
    /// # Panics
    ///
    /// Panics for sources of 4 GiB or more.
    #[must_use]
    pub fn parse(source: &str) -> Self {
        let tokens = tokenize(source);
        let mut stylesheet = Self {
            source: source.to_owned(),
            ..Self::default()
        };
        stylesheet.read_rules(&tokens, source, Place::default(), 0, true);
        stylesheet.rules.shrink_to_fit();
        stylesheet
    }

    /// Parses the content of a `style` attribute, a list of declarations.
    ///
    /// # Panics
    ///
    /// Panics for sources of 4 GiB or more.
    #[must_use]
    pub fn parse_declarations(source: &str) -> Self {
        let tokens = tokenize(source);
        Self {
            source: source.to_owned(),
            rules: vec![StyleRule {
                selectors: Vec::new(),
                declarations: read_declarations(&tokens, source),
                media: None,
            }],
            ..Self::default()
        }
    }

    /// The declarations from [`Self::parse_declarations`].
    #[must_use]
    pub fn declarations(&self) -> &[Declaration] {
        self.rules.first().map_or(&[], |rule| &rule.declarations)
    }

    /// Whether a condition and all those around it hold for this viewport.
    #[must_use]
    pub fn media_applies(&self, media: u32, viewport: Viewport) -> bool {
        let mut current = Some(media);
        while let Some(index) = current {
            let index = index as usize;
            if !matches_media(&self.media_queries[index], viewport) {
                return false;
            }
            current = self.media_parents[index];
        }
        true
    }

    /// The value of a declaration as written, without `!important`.
    #[must_use]
    pub fn value(&self, declaration: &Declaration) -> &str {
        &self.source[declaration.value_start as usize..declaration.value_end as usize]
    }

    fn read_rules(
        &mut self,
        tokens: &[CssToken],
        source: &str,
        place: Place,
        depth: usize,
        top_level: bool,
    ) {
        let mut index = 0;
        while let Some(token) = tokens.get(index) {
            match token.kind {
                K::Whitespace | K::Comment | K::Semicolon => index += 1,
                K::Cdo | K::Cdc if top_level => index += 1,
                K::AtKeyword => index = self.read_at_rule(tokens, index, source, place, depth),
                _ => index = self.read_style_rule(tokens, index, source, place),
            }
        }
    }

    fn read_at_rule(
        &mut self,
        tokens: &[CssToken],
        start: usize,
        source: &str,
        place: Place,
        depth: usize,
    ) -> usize {
        let keyword = unescape(&tokens[start].text(source)[1..]).to_ascii_lowercase();
        let (prelude_end, block) = find_prelude_end(tokens, start + 1, true);
        let prelude = &tokens[start + 1..prelude_end];
        let Some(block) = block else {
            if keyword == "import" {
                self.read_import(prelude, source);
            }
            return prelude_end + 1;
        };
        let end = find_block_end(tokens, block);
        if keyword == "media" && depth < MAX_AT_RULE_NESTING {
            let condition = source_of(prelude, source).trim().into();
            let index = u32::try_from(self.media_queries.len()).unwrap_or(u32::MAX);
            self.media_queries.push(condition);
            self.media_parents.push(place.media);
            let inner = &tokens[block + 1..end.min(tokens.len())];
            let place = Place {
                media: Some(index),
                ..place
            };
            self.read_rules(inner, source, place, depth + 1, false);
        } else if keyword == "supports" && depth < MAX_AT_RULE_NESTING {
            // Conditions are not evaluated yet; what cannot be used is ignored later anyway.
            let inner = &tokens[block + 1..end.min(tokens.len())];
            self.read_rules(inner, source, place, depth + 1, false);
        } else if keyword == "layer" && depth < MAX_AT_RULE_NESTING {
            // The layers stay in the order they come in, whatever order `@layer a, b;` gives them.
            let inner = &tokens[block + 1..end.min(tokens.len())];
            let place = Place {
                in_layer: true,
                ..place
            };
            self.read_rules(inner, source, place, depth + 1, false);
        }
        end + 1
    }

    fn read_import(&mut self, prelude: &[CssToken], source: &str) {
        let target = prelude
            .iter()
            .find(|token| matches!(token.kind, K::String | K::Url));
        let Some(target) = target else {
            return;
        };
        let text = target.text(source);
        let inner = match target.kind {
            K::String => {
                let quote = text.chars().next().unwrap_or('"');
                let body = &text[1..];
                body.strip_suffix(quote).unwrap_or(body)
            }
            _ => {
                let body = text.get(4..).unwrap_or("");
                body.strip_suffix(')').unwrap_or(body).trim()
            }
        };
        self.imports.push(unescape(inner).into());
    }

    fn read_style_rule(
        &mut self,
        tokens: &[CssToken],
        start: usize,
        source: &str,
        place: Place,
    ) -> usize {
        let (_, block) = find_prelude_end(tokens, start, false);
        let Some(block) = block else {
            return tokens.len();
        };
        let end = find_block_end(tokens, block);
        if let Some(mut selectors) = Selector::parse_list(&tokens[start..block], source) {
            let inner = &tokens[block + 1..end.min(tokens.len())];
            let declarations = read_declarations(inner, source);
            if !declarations.is_empty() {
                for selector in &mut selectors {
                    selector.in_layer = place.in_layer;
                }
                self.rules.push(StyleRule {
                    selectors,
                    declarations,
                    media: place.media,
                });
            }
        }
        end + 1
    }
}

fn source_of<'a>(tokens: &[CssToken], source: &'a str) -> &'a str {
    match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => &source[first.start as usize..last.end as usize],
        _ => "",
    }
}

/// Finds where a prelude ends: the `{` of its block, or for an at-rule a `;`. Returns that
/// position and, when it is a block, the same position again.
fn find_prelude_end(tokens: &[CssToken], start: usize, at_rule: bool) -> (usize, Option<usize>) {
    let mut closers = Vec::new();
    for (offset, token) in tokens[start..].iter().enumerate() {
        let index = start + offset;
        match token.kind {
            K::LeftBrace if closers.is_empty() => return (index, Some(index)),
            K::Semicolon if closers.is_empty() && at_rule => return (index, None),
            K::LeftParen | K::Function => closers.push(K::RightParen),
            K::LeftBracket => closers.push(K::RightBracket),
            K::LeftBrace => closers.push(K::RightBrace),
            K::RightParen | K::RightBracket | K::RightBrace
                if closers.last() == Some(&token.kind) =>
            {
                closers.pop();
            }
            _ => {}
        }
    }
    (tokens.len(), None)
}

/// Finds the `}` matching the `{` at `open`, or the end of the tokens.
fn find_block_end(tokens: &[CssToken], open: usize) -> usize {
    let mut closers = vec![K::RightBrace];
    for (offset, token) in tokens[open + 1..].iter().enumerate() {
        match token.kind {
            K::LeftParen | K::Function => closers.push(K::RightParen),
            K::LeftBracket => closers.push(K::RightBracket),
            K::LeftBrace => closers.push(K::RightBrace),
            K::RightParen | K::RightBracket | K::RightBrace
                if closers.last() == Some(&token.kind) =>
            {
                closers.pop();
                if closers.is_empty() {
                    return open + 1 + offset;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}

fn read_declarations(tokens: &[CssToken], source: &str) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let mut closers = Vec::new();
        let mut end = index;
        let mut nested_block = false;
        while let Some(token) = tokens.get(end) {
            match token.kind {
                K::Semicolon if closers.is_empty() => break,
                K::LeftBrace if closers.is_empty() => {
                    // A nested rule, which is not supported: it ends the item.
                    nested_block = true;
                    end = find_block_end(tokens, end);
                    break;
                }
                K::LeftParen | K::Function => closers.push(K::RightParen),
                K::LeftBracket => closers.push(K::RightBracket),
                K::LeftBrace => closers.push(K::RightBrace),
                K::RightParen | K::RightBracket | K::RightBrace
                    if closers.last() == Some(&token.kind) =>
                {
                    closers.pop();
                }
                _ => {}
            }
            end += 1;
        }
        if !nested_block && let Some(declaration) = read_declaration(&tokens[index..end], source) {
            declarations.push(declaration);
        }
        index = end + 1;
    }
    declarations.shrink_to_fit();
    declarations
}

fn is_blank(token: &CssToken) -> bool {
    matches!(token.kind, K::Whitespace | K::Comment)
}

fn read_declaration(tokens: &[CssToken], source: &str) -> Option<Declaration> {
    let mut tokens: Vec<CssToken> = tokens.to_vec();
    tokens.retain(|token| token.kind != K::Comment);
    let start = tokens.iter().position(|token| !is_blank(token))?;
    let name_token = tokens[start];
    if name_token.kind != K::Ident {
        return None;
    }
    let colon = start
        + 1
        + tokens[start + 1..]
            .iter()
            .position(|token| !is_blank(token))?;
    if tokens[colon].kind != K::Colon {
        return None;
    }
    let mut value = &tokens[colon + 1..];
    while value.first().is_some_and(is_blank) {
        value = &value[1..];
    }
    while value.last().is_some_and(is_blank) {
        value = &value[..value.len() - 1];
    }
    let mut important = false;
    if let [rest @ .., bang, word] = value
        && bang.text(source) == "!"
        && word.kind == K::Ident
        && word.text(source).eq_ignore_ascii_case("important")
    {
        important = true;
        value = rest;
    } else if let [rest @ .., bang, blank, word] = value
        && bang.text(source) == "!"
        && is_blank(blank)
        && word.kind == K::Ident
        && word.text(source).eq_ignore_ascii_case("important")
    {
        important = true;
        value = rest;
    }
    while value.last().is_some_and(is_blank) {
        value = &value[..value.len() - 1];
    }
    let name = unescape(name_token.text(source));
    let name = if name.starts_with("--") {
        name
    } else {
        name.to_ascii_lowercase()
    };
    if value.is_empty() && !name.starts_with("--") {
        return None;
    }
    let value_start = value.first().map_or(name_token.end, |token| token.start);
    let value_end = value.last().map_or(value_start, |token| token.end);
    Some(Declaration {
        name: name.into(),
        value_start,
        value_end,
        important,
    })
}
