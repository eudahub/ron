//! Evaluates `@media` conditions against the size of the view.

use super::values::{Unit, parse_length};

/// What conditions and viewport-relative lengths are measured against, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1024.0,
            height: 768.0,
        }
    }
}

/// Whether a media query list holds. What is not understood counts as false, as a
/// browser that lacks a feature must treat it.
#[must_use]
pub fn matches_media(list: &str, viewport: Viewport) -> bool {
    let list = list.trim();
    list.is_empty() || list.split(',').any(|query| matches_query(query, viewport))
}

fn matches_query(query: &str, viewport: Viewport) -> bool {
    let lower = query.trim().to_ascii_lowercase();
    let (negated, rest) = match lower.strip_prefix("not ") {
        Some(rest) => (true, rest),
        None => (false, lower.strip_prefix("only ").unwrap_or(&lower)),
    };
    let result = rest.split(" and ").all(|part| {
        let part = part.trim();
        match part.strip_prefix('(').and_then(|p| p.strip_suffix(')')) {
            Some(feature) => matches_feature(feature, viewport),
            None => matches!(part, "all" | "screen"),
        }
    });
    result != negated
}

fn matches_feature(feature: &str, viewport: Viewport) -> bool {
    if let Some((first, comparison, rest)) = split_comparison(feature) {
        return matches_range(first, comparison, rest, viewport);
    }
    let Some((name, value)) = feature.split_once(':') else {
        return matches!(feature.trim(), "color" | "hover" | "pointer");
    };
    let (name, value) = (name.trim(), value.trim());
    let compare = |actual: f32, holds: fn(f32, f32) -> bool| {
        pixels(value).is_some_and(|px| holds(actual, px))
    };
    match name {
        "width" => compare(viewport.width, |a, b| (a - b).abs() < 0.5),
        "min-width" => compare(viewport.width, |a, b| a >= b),
        "max-width" => compare(viewport.width, |a, b| a <= b),
        "height" => compare(viewport.height, |a, b| (a - b).abs() < 0.5),
        "min-height" => compare(viewport.height, |a, b| a >= b),
        "max-height" => compare(viewport.height, |a, b| a <= b),
        "orientation" => {
            let landscape = viewport.width >= viewport.height;
            value == if landscape { "landscape" } else { "portrait" }
        }
        "prefers-color-scheme" => value == "light",
        "prefers-reduced-motion" => value == "no-preference",
        "color" | "min-color" => true,
        "hover" | "any-hover" => value == "hover",
        "pointer" | "any-pointer" => value == "fine",
        _ => false,
    }
}

/// A length in pixels, also as the sum or difference of two, which pages use to stay off the
/// exact width where another condition starts.
fn pixels(value: &str) -> Option<f32> {
    if let Some(inner) = value
        .strip_prefix("calc(")
        .and_then(|v| v.strip_suffix(')'))
    {
        if let Some((first, second)) = inner.split_once(" - ") {
            return Some(pixels(first.trim())? - pixels(second.trim())?);
        }
        if let Some((first, second)) = inner.split_once(" + ") {
            return Some(pixels(first.trim())? + pixels(second.trim())?);
        }
        return pixels(inner.trim());
    }
    let length = parse_length(value, false)?;
    match length.unit {
        Unit::Px => Some(length.value),
        Unit::Em | Unit::Rem => Some(length.value * 16.0),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Comparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
}

impl Comparison {
    fn holds(self, a: f32, b: f32) -> bool {
        match self {
            Self::Less => a < b,
            Self::LessOrEqual => a <= b,
            Self::Greater => a > b,
            Self::GreaterOrEqual => a >= b,
            Self::Equal => (a - b).abs() < 0.5,
        }
    }

    /// The same with its two sides changing places.
    fn turned(self) -> Self {
        match self {
            Self::Less => Self::Greater,
            Self::LessOrEqual => Self::GreaterOrEqual,
            Self::Greater => Self::Less,
            Self::GreaterOrEqual => Self::LessOrEqual,
            Self::Equal => Self::Equal,
        }
    }
}

/// Splits `width>=768px` at its first comparison.
fn split_comparison(text: &str) -> Option<(&str, Comparison, &str)> {
    let at = text.find(['<', '>', '='])?;
    let (first, rest) = text.split_at(at);
    let (comparison, length) = match rest.as_bytes() {
        [b'<', b'=', ..] => (Comparison::LessOrEqual, 2),
        [b'>', b'=', ..] => (Comparison::GreaterOrEqual, 2),
        [b'<', ..] => (Comparison::Less, 1),
        [b'>', ..] => (Comparison::Greater, 1),
        _ => (Comparison::Equal, 1),
    };
    Some((first.trim(), comparison, rest[length..].trim()))
}

/// `width>=768px`, `768px<=width` or `400px<=width<=700px`.
fn matches_range(first: &str, comparison: Comparison, rest: &str, viewport: Viewport) -> bool {
    let size_of = |name: &str| match name {
        "width" => Some(viewport.width),
        "height" => Some(viewport.height),
        _ => None,
    };
    if let Some((middle, second, last)) = split_comparison(rest) {
        let (Some(actual), Some(low), Some(high)) = (size_of(middle), pixels(first), pixels(last))
        else {
            return false;
        };
        return comparison.holds(low, actual) && second.holds(actual, high);
    }
    match (size_of(first), size_of(rest)) {
        (Some(actual), None) => pixels(rest).is_some_and(|px| comparison.holds(actual, px)),
        (None, Some(actual)) => {
            pixels(first).is_some_and(|px| comparison.turned().holds(actual, px))
        }
        _ => false,
    }
}
