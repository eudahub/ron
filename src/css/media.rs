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
pub(super) fn matches_media(list: &str, viewport: Viewport) -> bool {
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
    let Some((name, value)) = feature.split_once(':') else {
        return matches!(feature.trim(), "color" | "hover" | "pointer");
    };
    let (name, value) = (name.trim(), value.trim());
    let pixels = || match parse_length(value, false) {
        Some(length) => match length.unit {
            Unit::Px => Some(length.value),
            Unit::Em | Unit::Rem => Some(length.value * 16.0),
            _ => None,
        },
        None => None,
    };
    let compare =
        |actual: f32, holds: fn(f32, f32) -> bool| pixels().is_some_and(|px| holds(actual, px));
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
        _ => false,
    }
}
