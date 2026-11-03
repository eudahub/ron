//! Property values: colors, lengths and keywords, as read from tokens.

use std::sync::Arc;

use super::tokenizer::{CssToken, CssTokenKind as K, tokenize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl Color {
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const TRANSPARENT: Self = Self {
        red: 0,
        green: 0,
        blue: 0,
        alpha: 0,
    };

    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha: 255,
        }
    }

    const fn from_packed(value: u32) -> Self {
        Self::rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    /// Reads a color name, `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
    pub(super) fn from_name_or_hash(text: &str) -> Option<Self> {
        if let Some(hex) = text.strip_prefix('#') {
            return Self::from_hex(hex);
        }
        let lower = text.to_ascii_lowercase();
        if lower == "transparent" {
            return Some(Self::TRANSPARENT);
        }
        let index = NAMED_COLORS
            .binary_search_by(|(name, _)| (*name).cmp(lower.as_str()))
            .ok()?;
        Some(Self::from_packed(NAMED_COLORS[index].1))
    }

    pub(super) fn from_hex(hex: &str) -> Option<Self> {
        let digit = |index: usize| {
            hex.as_bytes()
                .get(index)
                .and_then(|byte| char::from(*byte).to_digit(16))
                .and_then(|value| u8::try_from(value).ok())
        };
        let short = |index: usize| digit(index).map(|value| value * 17);
        let long = |index: usize| Some(digit(index)? * 16 + digit(index + 1)?);
        match hex.len() {
            3 => Some(Self::rgb(short(0)?, short(1)?, short(2)?)),
            4 => Some(Self {
                alpha: short(3)?,
                ..Self::rgb(short(0)?, short(1)?, short(2)?)
            }),
            6 => Some(Self::rgb(long(0)?, long(2)?, long(4)?)),
            8 => Some(Self {
                alpha: long(6)?,
                ..Self::rgb(long(0)?, long(2)?, long(4)?)
            }),
            _ => None,
        }
    }

    /// Reads the color of `rgb()`, `rgba()`, `hsl()` or `hsla()` from its arguments.
    fn from_function(name: &str, arguments: &[CssToken], source: &str) -> Option<Self> {
        let numbers: Vec<(f32, bool)> = arguments
            .iter()
            .filter_map(|token| match token.kind {
                K::Number => Some((token.text(source).parse().ok()?, false)),
                K::Percentage => Some((token.text(source).strip_suffix('%')?.parse().ok()?, true)),
                K::Dimension => {
                    // Only hue takes a unit, and `deg` is the only common one.
                    let text = token.text(source).to_ascii_lowercase();
                    Some((text.strip_suffix("deg")?.parse().ok()?, false))
                }
                _ => None,
            })
            .collect();
        if !(numbers.len() == 3 || numbers.len() == 4) {
            return None;
        }
        let alpha = numbers.get(3).map_or(255, |&(value, percent)| {
            let fraction = if percent { value / 100.0 } else { value };
            to_byte(fraction * 255.0)
        });
        if name.starts_with("rgb") {
            let channel =
                |(value, percent): (f32, bool)| to_byte(if percent { value * 2.55 } else { value });
            Some(Self {
                red: channel(numbers[0]),
                green: channel(numbers[1]),
                blue: channel(numbers[2]),
                alpha,
            })
        } else {
            let (red, green, blue) = hsl_to_rgb(numbers[0].0, numbers[1].0, numbers[2].0);
            Some(Self {
                red,
                green,
                blue,
                alpha,
            })
        }
    }
}

fn to_byte(value: f32) -> u8 {
    // The cast cannot overflow after clamping.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        value.round().clamp(0.0, 255.0) as u8
    }
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
    let hue = hue.rem_euclid(360.0) / 30.0;
    let saturation = (saturation / 100.0).clamp(0.0, 1.0);
    let lightness = (lightness / 100.0).clamp(0.0, 1.0);
    let amount = saturation * lightness.min(1.0 - lightness);
    let channel = |shift: f32| {
        let k = (shift + hue) % 12.0;
        to_byte(255.0 * (lightness - amount * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)))
    };
    (channel(0.0), channel(8.0), channel(4.0))
}

/// Parses a declaration's color, including functions; `None` when it is something else.
pub(super) fn parse_color(text: &str) -> Option<Color> {
    let tokens = tokenize(text);
    let tokens = significant(&tokens);
    match tokens.as_slice() {
        [token] if matches!(token.kind, K::Ident | K::Hash) => {
            Color::from_name_or_hash(token.text(text))
        }
        [function, arguments @ .., close]
            if function.kind == K::Function && close.kind == K::RightParen =>
        {
            let name = function.text(text).to_ascii_lowercase();
            let name = name.strip_suffix('(')?;
            if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla") {
                return None;
            }
            Color::from_function(name, arguments, text)
        }
        _ => None,
    }
}

/// The tokens without white space and comments.
pub(super) fn significant(tokens: &[CssToken]) -> Vec<CssToken> {
    tokens
        .iter()
        .filter(|token| !matches!(token.kind, K::Whitespace | K::Comment))
        .copied()
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Unit {
    Px,
    Em,
    Rem,
    Ex,
    Ch,
    Percent,
    Vw,
    Vh,
    Vmin,
    Vmax,
}

/// A length or percentage as written, with absolute units already in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Length {
    pub(super) value: f32,
    pub(super) unit: Unit,
}

/// Reads a length; a bare zero counts, and other bare numbers only with `unitless`.
pub(super) fn parse_length(text: &str, unitless: bool) -> Option<Length> {
    let text = text.trim();
    if let Some(number) = text.strip_suffix('%') {
        return Some(Length {
            value: finite(number.parse().ok()?)?,
            unit: Unit::Percent,
        });
    }
    let split = text
        .char_indices()
        .find(|&(index, c)| {
            let exponent = matches!(c, 'e' | 'E')
                && text[index + 1..]
                    .starts_with(|n: char| n.is_ascii_digit() || n == '-' || n == '+');
            !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-') || exponent)
        })
        .map_or(text.len(), |(index, _)| index);
    let (number, unit) = text.split_at(split);
    let value = finite(number.parse().ok()?)?;
    let px = |value: f32| {
        Some(Length {
            value,
            unit: Unit::Px,
        })
    };
    let relative = |unit: Unit| Some(Length { value, unit });
    match unit.to_ascii_lowercase().as_str() {
        "" if value == 0.0 || unitless => px(value),
        "px" => px(value),
        "pt" => px(value * 96.0 / 72.0),
        "pc" => px(value * 16.0),
        "in" => px(value * 96.0),
        "cm" => px(value * 96.0 / 2.54),
        "mm" => px(value * 96.0 / 25.4),
        "q" => px(value * 96.0 / 101.6),
        "em" => relative(Unit::Em),
        "rem" => relative(Unit::Rem),
        "ex" => relative(Unit::Ex),
        "ch" => relative(Unit::Ch),
        "vw" => relative(Unit::Vw),
        "vh" => relative(Unit::Vh),
        "vmin" => relative(Unit::Vmin),
        "vmax" => relative(Unit::Vmax),
        _ => None,
    }
}

fn finite(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

/// Splits a value at top-level white space, keeping functions and strings whole.
pub(super) fn split_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut parens = 0_usize;
    let mut start = None;
    for token in tokenize(text) {
        let blank = matches!(token.kind, K::Whitespace | K::Comment);
        match token.kind {
            K::Function | K::LeftParen => parens += 1,
            K::RightParen => parens = parens.saturating_sub(1),
            _ => {}
        }
        if blank && parens == 0 {
            if let Some(begin) = start.take() {
                words.push(&text[begin..token.start as usize]);
            }
        } else if start.is_none() {
            start = Some(token.start as usize);
        }
    }
    if let Some(begin) = start {
        words.push(&text[begin..]);
    }
    words
}

/// Reads a `url(...)` value, quoted or not.
pub(super) fn parse_url(text: &str) -> Option<Arc<str>> {
    let tokens = tokenize(text);
    let tokens = significant(&tokens);
    match tokens.as_slice() {
        [token] if token.kind == K::Url => {
            let raw = token.text(text);
            let body = raw.get(4..)?;
            let body = body.strip_suffix(')').unwrap_or(body).trim();
            Some(super::tokenizer::unescape(body).into())
        }
        [function, string, ..]
            if function.kind == K::Function
                && function.text(text).eq_ignore_ascii_case("url(")
                && string.kind == K::String =>
        {
            let raw = string.text(text);
            let quote = raw.chars().next()?;
            let body = &raw[1..];
            Some(super::tokenizer::unescape(body.strip_suffix(quote).unwrap_or(body)).into())
        }
        _ => None,
    }
}

/// CSS color names, sorted.
const NAMED_COLORS: [(&str, u32); 148] = [
    ("aliceblue", 0xf0f8ff),
    ("antiquewhite", 0xfaebd7),
    ("aqua", 0x00ffff),
    ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff),
    ("beige", 0xf5f5dc),
    ("bisque", 0xffe4c4),
    ("black", 0x000000),
    ("blanchedalmond", 0xffebcd),
    ("blue", 0x0000ff),
    ("blueviolet", 0x8a2be2),
    ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887),
    ("cadetblue", 0x5f9ea0),
    ("chartreuse", 0x7fff00),
    ("chocolate", 0xd2691e),
    ("coral", 0xff7f50),
    ("cornflowerblue", 0x6495ed),
    ("cornsilk", 0xfff8dc),
    ("crimson", 0xdc143c),
    ("cyan", 0x00ffff),
    ("darkblue", 0x00008b),
    ("darkcyan", 0x008b8b),
    ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xa9a9a9),
    ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b),
    ("darkolivegreen", 0x556b2f),
    ("darkorange", 0xff8c00),
    ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000),
    ("darksalmon", 0xe9967a),
    ("darkseagreen", 0x8fbc8f),
    ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f),
    ("darkslategrey", 0x2f4f4f),
    ("darkturquoise", 0x00ced1),
    ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493),
    ("deepskyblue", 0x00bfff),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff),
    ("firebrick", 0xb22222),
    ("floralwhite", 0xfffaf0),
    ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff),
    ("gainsboro", 0xdcdcdc),
    ("ghostwhite", 0xf8f8ff),
    ("gold", 0xffd700),
    ("goldenrod", 0xdaa520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xadff2f),
    ("grey", 0x808080),
    ("honeydew", 0xf0fff0),
    ("hotpink", 0xff69b4),
    ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082),
    ("ivory", 0xfffff0),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5),
    ("lawngreen", 0x7cfc00),
    ("lemonchiffon", 0xfffacd),
    ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080),
    ("lightcyan", 0xe0ffff),
    ("lightgoldenrodyellow", 0xfafad2),
    ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90),
    ("lightgrey", 0xd3d3d3),
    ("lightpink", 0xffb6c1),
    ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa),
    ("lightskyblue", 0x87cefa),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de),
    ("lightyellow", 0xffffe0),
    ("lime", 0x00ff00),
    ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6),
    ("magenta", 0xff00ff),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd),
    ("mediumorchid", 0xba55d3),
    ("mediumpurple", 0x9370db),
    ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee),
    ("mediumspringgreen", 0x00fa9a),
    ("mediumturquoise", 0x48d1cc),
    ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xf5fffa),
    ("mistyrose", 0xffe4e1),
    ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead),
    ("navy", 0x000080),
    ("oldlace", 0xfdf5e6),
    ("olive", 0x808000),
    ("olivedrab", 0x6b8e23),
    ("orange", 0xffa500),
    ("orangered", 0xff4500),
    ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa),
    ("palegreen", 0x98fb98),
    ("paleturquoise", 0xafeeee),
    ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5),
    ("peachpuff", 0xffdab9),
    ("peru", 0xcd853f),
    ("pink", 0xffc0cb),
    ("plum", 0xdda0dd),
    ("powderblue", 0xb0e0e6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xff0000),
    ("rosybrown", 0xbc8f8f),
    ("royalblue", 0x4169e1),
    ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072),
    ("sandybrown", 0xf4a460),
    ("seagreen", 0x2e8b57),
    ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d),
    ("silver", 0xc0c0c0),
    ("skyblue", 0x87ceeb),
    ("slateblue", 0x6a5acd),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xfffafa),
    ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4),
    ("tan", 0xd2b48c),
    ("teal", 0x008080),
    ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347),
    ("turquoise", 0x40e0d0),
    ("violet", 0xee82ee),
    ("wheat", 0xf5deb3),
    ("white", 0xffffff),
    ("whitesmoke", 0xf5f5f5),
    ("yellow", 0xffff00),
    ("yellowgreen", 0x9acd32),
];
