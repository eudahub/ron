//! Reads attributes from the source text of a tag, on demand and without copying.

use std::borrow::Cow;

use super::entities::{Context, decode};
use super::tokenizer::{is_html_space, tag_name};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Attribute<'a> {
    /// The name as written; HTML names are case-insensitive.
    pub name: &'a str,
    /// The value as written, with character references not decoded.
    pub value: Option<&'a str>,
}

impl<'a> Attribute<'a> {
    /// The value with character references decoded; empty for an attribute without one.
    #[must_use]
    pub fn decoded_value(&self) -> Cow<'a, str> {
        let value = self.value.unwrap_or("");
        if value.contains('\0') {
            let replaced = value.replace('\0', "\u{fffd}");
            return Cow::Owned(decode(&replaced, Context::Attribute).into_owned());
        }
        decode(value, Context::Attribute)
    }
}

/// Iterates over the attributes of a start tag token's text.
#[must_use]
pub fn attributes(tag: &str) -> Attributes<'_> {
    let prefix = if tag.starts_with("</") { 2 } else { 1 };
    let name_length = tag_name(tag).len();
    Attributes {
        rest: tag.get(prefix + name_length..).unwrap_or(""),
    }
}

pub struct Attributes<'a> {
    rest: &'a str,
}

impl Attributes<'_> {
    fn skip_spaces(&mut self) {
        self.rest = self.rest.trim_start_matches(is_html_space);
    }
}

impl<'a> Iterator for Attributes<'a> {
    type Item = Attribute<'a>;

    fn next(&mut self) -> Option<Attribute<'a>> {
        self.rest = self
            .rest
            .trim_start_matches(|c| is_html_space(c) || c == '/');
        if self.rest.is_empty() || self.rest.starts_with('>') {
            return None;
        }
        // A leading '=' belongs to the name.
        let skip = usize::from(self.rest.starts_with('='));
        let name_end = self.rest[skip..]
            .find(|c: char| is_html_space(c) || matches!(c, '/' | '>' | '='))
            .map_or(self.rest.len(), |end| end + skip);
        let (name, rest) = self.rest.split_at(name_end);
        self.rest = rest;
        self.skip_spaces();
        let Some(after_equals) = self.rest.strip_prefix('=') else {
            return Some(Attribute { name, value: None });
        };
        self.rest = after_equals;
        self.skip_spaces();
        let quote = self.rest.chars().next().filter(|c| matches!(c, '"' | '\''));
        let value = if let Some(quote) = quote {
            let body = &self.rest[1..];
            let end = body.find(quote).unwrap_or(body.len());
            self.rest = body.get(end + 1..).unwrap_or("");
            &body[..end]
        } else {
            let end = self
                .rest
                .find(|c: char| is_html_space(c) || c == '>')
                .unwrap_or(self.rest.len());
            let (value, rest) = self.rest.split_at(end);
            self.rest = rest;
            value
        };
        Some(Attribute {
            name,
            value: Some(value),
        })
    }
}
