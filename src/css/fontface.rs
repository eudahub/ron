//! `@font-face` rules, which say where fonts are to be found. What is done with those is up to the
//! caller, who can tell where the sheet came from and whether it wants them.

use super::values::{parse_url, split_words};

/// More sources for one font are left out, as a page needs no more than a few formats.
pub(super) const MAX_FONT_SOURCES: usize = 8;

/// One place to find a font.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontSource {
    /// As written, so relative to the sheet.
    pub url: Box<str>,
    /// What the rule says it is, in lowercase, if it says so.
    pub format: Option<Box<str>>,
}

/// A font that a sheet says exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontFace {
    /// The name that `font-family` finds it by.
    pub family: Box<str>,
    /// In the order to try them.
    pub sources: Vec<FontSource>,
    /// The lightest and the boldest weight that it covers, which are the same unless it varies.
    pub weight: (u16, u16),
    pub italic: bool,
    /// Which characters it is for, and all of them if this is empty.
    pub unicode_range: Vec<(u32, u32)>,
    /// Index into [`super::Stylesheet::media_queries`] of the condition around the rule.
    pub media: Option<u32>,
}

/// Splits at commas that are not within parentheses or quotes.
fn split_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut depth, mut quote, mut escaped) = (0, 0_usize, None, false);
    for (at, character) in text.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if let Some(open) = quote {
            if character == open {
                quote = None;
            }
        } else {
            match character {
                '"' | '\'' => quote = Some(character),
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(&text[start..at]);
                    start = at + 1;
                }
                _ => {}
            }
        }
    }
    parts.push(&text[start..]);
    parts
}

fn unquoted(text: &str) -> &str {
    let text = text.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|inner| inner.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// The formats that can be read. Others are left out, so that the next source gets its turn.
fn is_known_format(format: &str) -> bool {
    matches!(format, "woff2" | "woff" | "truetype" | "opentype")
}

fn parse_source(text: &str) -> Option<FontSource> {
    let words = split_words(text);
    let url = parse_url(words.first()?)?;
    let format = words.iter().skip(1).find_map(|word| {
        let lower = word.to_ascii_lowercase();
        let inner = lower.strip_prefix("format(")?.strip_suffix(')')?;
        Some(unquoted(inner).to_owned())
    });
    if format
        .as_deref()
        .is_some_and(|format| !is_known_format(format))
    {
        return None;
    }
    Some(FontSource {
        url: url.as_ref().into(),
        format: format.map(Into::into),
    })
}

/// The sources of a `src` declaration that can be used. `local()` cannot: which fonts a computer
/// has is none of the business of a page.
pub(super) fn parse_sources(text: &str) -> Vec<FontSource> {
    split_commas(text)
        .into_iter()
        .filter_map(parse_source)
        .take(MAX_FONT_SOURCES)
        .collect()
}

fn parse_one_weight(text: &str) -> Option<u16> {
    match text.to_ascii_lowercase().as_str() {
        "normal" => Some(400),
        "bold" => Some(700),
        other => other
            .parse::<f32>()
            .ok()
            .filter(|weight| (1.0..=1000.0).contains(weight))
            .map(|weight| {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    weight as u16
                }
            }),
    }
}

/// A weight or a range of them.
pub(super) fn parse_weight(text: &str) -> Option<(u16, u16)> {
    match split_words(text).as_slice() {
        [one] => parse_one_weight(one).map(|weight| (weight, weight)),
        [low, high] => {
            let (low, high) = (parse_one_weight(low)?, parse_one_weight(high)?);
            Some((low.min(high), low.max(high)))
        }
        _ => None,
    }
}

pub(super) fn parse_italic(text: &str) -> bool {
    split_words(text).first().is_some_and(|word| {
        word.eq_ignore_ascii_case("italic") || word.eq_ignore_ascii_case("oblique")
    })
}

/// `U+26`, `U+0-7F` and `U+4??`, of which what is wrong is left out.
pub(super) fn parse_unicode_range(text: &str) -> Vec<(u32, u32)> {
    let hex = |digits: &str| u32::from_str_radix(digits, 16).ok();
    split_commas(text)
        .into_iter()
        .filter_map(|part| {
            let part = part.trim();
            let digits = part
                .strip_prefix("U+")
                .or_else(|| part.strip_prefix("u+"))?;
            if digits.contains('?') {
                return Some((
                    hex(&digits.replace('?', "0"))?,
                    hex(&digits.replace('?', "F"))?,
                ));
            }
            Some(match digits.split_once('-') {
                Some((low, high)) => (hex(low)?, hex(high)?),
                None => (hex(digits)?, hex(digits)?),
            })
        })
        .collect()
}

pub(super) fn parse_family(text: &str) -> Option<Box<str>> {
    let family = unquoted(text);
    (!family.is_empty()).then(|| family.into())
}
