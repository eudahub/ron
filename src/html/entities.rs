//! Decodes character references such as `&amp;` and `&#169;` when text is read. The source
//! keeps them as written, so nothing is decoded or stored unless it is asked for.

use std::borrow::Cow;

#[path = "../../generated/html_entities.rs"]
mod html_entities;

use html_entities::ENTITIES;

/// The longest name in the standard is about this long; longer ones are not looked up.
const MAX_NAME_BYTES: usize = 32;

/// Code points 0x80 to 0x9F of numeric references mean their Windows-1252 characters.
const WINDOWS_1252: [char; 32] = [
    '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
];

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Context {
    Text,
    /// A reference without `;` followed by `=` or a letter or digit stays as written.
    Attribute,
}

/// Decodes the references in `text`, borrowing it when there are none.
pub(super) fn decode(text: &str, context: Context) -> Cow<'_, str> {
    let Some(first) = text.find('&') else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..first]);
    let mut rest = &text[first..];
    while let Some(position) = rest.find('&') {
        out.push_str(&rest[..position]);
        rest = &rest[position..];
        let after = &rest[1..];
        let consumed = if after.starts_with('#') {
            decode_numeric(after, &mut out)
        } else {
            decode_named(after, context, &mut out)
        };
        match consumed {
            Some(length) => rest = &after[length..],
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Decodes `#123;` or `#x7b;`, returning the bytes used.
fn decode_numeric(after: &str, out: &mut String) -> Option<usize> {
    let body = &after[1..];
    let (radix, skip) = if body.starts_with(['x', 'X']) {
        (16, 1)
    } else {
        (10, 0)
    };
    let digits = &body[skip..];
    let count = digits
        .bytes()
        .take_while(|byte| char::from(*byte).is_digit(radix))
        .count();
    if count == 0 {
        return None;
    }
    let value = digits[..count].chars().fold(0_u32, |value, digit| {
        value
            .saturating_mul(radix)
            .saturating_add(digit.to_digit(radix).unwrap_or(0))
    });
    let character = match value {
        0x80..=0x9f => WINDOWS_1252[(value - 0x80) as usize],
        0 => '\u{fffd}',
        _ => char::from_u32(value).unwrap_or('\u{fffd}'),
    };
    out.push(character);
    let semicolon = usize::from(digits[count..].starts_with(';'));
    Some(1 + skip + count + semicolon)
}

/// Decodes the longest name found at the start of `after`, returning the bytes used.
fn decode_named(after: &str, context: Context, out: &mut String) -> Option<usize> {
    let run = after
        .bytes()
        .take_while(u8::is_ascii_alphanumeric)
        .take(MAX_NAME_BYTES)
        .count();
    if run == 0 {
        return None;
    }
    if after[run..].starts_with(';')
        && let Some(value) = lookup(&after[..=run])
    {
        out.push_str(value);
        return Some(run + 1);
    }
    for length in (1..=run).rev() {
        let Some(value) = lookup(&after[..length]) else {
            continue;
        };
        let next = after[length..].chars().next();
        if context == Context::Attribute
            && next.is_some_and(|c| c == '=' || c.is_ascii_alphanumeric())
        {
            return None;
        }
        out.push_str(value);
        return Some(length);
    }
    None
}

fn lookup(name: &str) -> Option<&'static str> {
    ENTITIES
        .binary_search_by(|(candidate, _)| candidate.as_bytes().cmp(name.as_bytes()))
        .ok()
        .map(|index| ENTITIES[index].1)
}
