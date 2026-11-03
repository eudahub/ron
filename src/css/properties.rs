//! The properties ron understands, and reading declarations into them. Shorthands expand
//! here. A value that is not understood drops its declaration, as CSS requires, which also
//! means that `var()` and `calc()` are ignored for now.

use std::sync::Arc;

use super::values::{Color, Length, Unit, parse_color, parse_length, parse_url, split_words};

macro_rules! keywords {
    ($(#[$meta:meta])* $name:ident { $($variant:ident $text:literal)* }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            fn parse(text: &str) -> Option<Self> {
                $(if text.eq_ignore_ascii_case($text) {
                    return Some(Self::$variant);
                })*
                None
            }
        }
    };
}

keywords!(Display {
    Inline "inline" Block "block" InlineBlock "inline-block" ListItem "list-item" None "none"
    Table "table" InlineTable "inline-table" TableRowGroup "table-row-group"
    TableHeaderGroup "table-header-group" TableFooterGroup "table-footer-group"
    TableRow "table-row" TableColumnGroup "table-column-group" TableColumn "table-column"
    TableCell "table-cell" TableCaption "table-caption" Contents "contents"
    Flex "flex" InlineFlex "inline-flex" Grid "grid" InlineGrid "inline-grid"
});
keywords!(Float { None "none" Left "left" Right "right" });
keywords!(BackgroundRepeat {
    Repeat "repeat" RepeatX "repeat-x" RepeatY "repeat-y" NoRepeat "no-repeat"
});
keywords!(Clear { None "none" Left "left" Right "right" Both "both" });
keywords!(Position {
    Static "static" Relative "relative" Absolute "absolute" Fixed "fixed" Sticky "sticky"
});
keywords!(TextAlign {
    Start "start" End "end" Left "left" Right "right" Center "center" Justify "justify"
});
keywords!(FontStyle { Normal "normal" Italic "italic" Oblique "oblique" });
keywords!(WhiteSpace {
    Normal "normal" Pre "pre" Nowrap "nowrap" PreWrap "pre-wrap" PreLine "pre-line"
});
keywords!(ListStyleType {
    None "none" Disc "disc" Circle "circle" Square "square" Decimal "decimal"
    LowerAlpha "lower-alpha" UpperAlpha "upper-alpha" LowerRoman "lower-roman"
    UpperRoman "upper-roman" LowerLatin "lower-latin" UpperLatin "upper-latin"
});
keywords!(Visibility { Visible "visible" Hidden "hidden" Collapse "collapse" });
keywords!(BorderStyle {
    None "none" Hidden "hidden" Dotted "dotted" Dashed "dashed" Solid "solid" Double "double"
    Groove "groove" Ridge "ridge" Inset "inset" Outset "outset"
});
keywords!(TextTransform {
    None "none" Uppercase "uppercase" Lowercase "lowercase" Capitalize "capitalize"
});
keywords!(BoxSizing { ContentBox "content-box" BorderBox "border-box" });
keywords!(BorderCollapse { Separate "separate" Collapse "collapse" });
keywords!(Overflow { Visible "visible" Hidden "hidden" Scroll "scroll" Auto "auto" Clip "clip" });
keywords!(VerticalAlign {
    Baseline "baseline" Sub "sub" Super "super" Top "top" TextTop "text-top" Middle "middle"
    Bottom "bottom" TextBottom "text-bottom"
});

macro_rules! properties {
    ($($variant:ident $text:literal)*) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u8)]
        pub(super) enum Prop {
            $($variant,)*
        }

        pub(super) const PROPERTY_COUNT: usize = [$(Prop::$variant),*].len();
        pub(super) const ALL_PROPERTIES: [Prop; PROPERTY_COUNT] = [$(Prop::$variant),*];

        fn longhand(name: &str) -> Option<Prop> {
            match name {
                $($text => Some(Prop::$variant),)*
                _ => None,
            }
        }
    };
}

properties! {
    Display "display" Float "float" Clear "clear" Position "position"
    Top "top" Right "right" Bottom "bottom" Left "left"
    Color "color" BackgroundColor "background-color" BackgroundImage "background-image"
    BackgroundRepeat "background-repeat" BackgroundPositionX "background-position-x"
    BackgroundPositionY "background-position-y"
    FontFamily "font-family" FontSize "font-size" FontStyle "font-style" FontWeight "font-weight"
    LineHeight "line-height" TextAlign "text-align" TextIndent "text-indent"
    WhiteSpace "white-space" TextDecoration "text-decoration-line"
    TextTransform "text-transform" VerticalAlign "vertical-align"
    ListStyleType "list-style-type" ListStyleImage "list-style-image"
    Visibility "visibility" Overflow "overflow" BoxSizing "box-sizing"
    BorderCollapse "border-collapse" BorderSpacing "border-spacing"
    MarginTop "margin-top" MarginRight "margin-right" MarginBottom "margin-bottom"
    MarginLeft "margin-left"
    PaddingTop "padding-top" PaddingRight "padding-right" PaddingBottom "padding-bottom"
    PaddingLeft "padding-left"
    BorderTopWidth "border-top-width" BorderRightWidth "border-right-width"
    BorderBottomWidth "border-bottom-width" BorderLeftWidth "border-left-width"
    BorderTopStyle "border-top-style" BorderRightStyle "border-right-style"
    BorderBottomStyle "border-bottom-style" BorderLeftStyle "border-left-style"
    BorderTopColor "border-top-color" BorderRightColor "border-right-color"
    BorderBottomColor "border-bottom-color" BorderLeftColor "border-left-color"
    Width "width" Height "height" MinWidth "min-width" MinHeight "min-height"
    MaxWidth "max-width" MaxHeight "max-height"
}

/// `text-decoration-line` bits.
pub const UNDERLINE: u8 = 1;
pub const OVERLINE: u8 = 2;
pub const LINE_THROUGH: u8 = 4;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Inherit,
    Initial,
    /// `auto`, `none` or `normal`, whichever the property's empty state is called.
    Empty,
    CurrentColor,
    Length(Length),
    /// A bare number, as `line-height` and `font-weight` take.
    Number(f32),
    Color(Color),
    Url(Arc<str>),
    Family(Arc<str>),
    Decoration(u8),
    Display(Display),
    Float(Float),
    BackgroundRepeat(BackgroundRepeat),
    Clear(Clear),
    Position(Position),
    TextAlign(TextAlign),
    FontStyle(FontStyle),
    WhiteSpace(WhiteSpace),
    ListStyleType(ListStyleType),
    Visibility(Visibility),
    BorderStyle(BorderStyle),
    TextTransform(TextTransform),
    BoxSizing(BoxSizing),
    BorderCollapse(BorderCollapse),
    Overflow(Overflow),
    VerticalAlign(VerticalAlign),
    /// `larger` or `smaller` when true or false.
    FontRelative(bool),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Compiled {
    pub(super) prop: Prop,
    pub(super) value: Value,
    pub(super) important: bool,
}

/// Reads one declaration into longhands, or nothing when it is not understood.
pub(super) fn compile(name: &str, value: &str, important: bool, out: &mut Vec<Compiled>) {
    let start = out.len();
    if !compile_inner(name, value, important, out) {
        out.truncate(start);
    }
}

fn compile_inner(name: &str, value: &str, important: bool, out: &mut Vec<Compiled>) -> bool {
    let text = value.trim();
    if text.contains("var(") || text.contains("calc(") {
        return false;
    }
    let mut push = |prop: Prop, value: Value| {
        out.push(Compiled {
            prop,
            value,
            important,
        });
    };
    let wide = if text.eq_ignore_ascii_case("inherit") {
        Some(Value::Inherit)
    } else if text.eq_ignore_ascii_case("initial") || text.eq_ignore_ascii_case("unset") {
        Some(Value::Initial)
    } else {
        None
    };
    if let Some(longhands) = shorthand_longhands(name) {
        if let Some(value) = wide {
            for &prop in longhands {
                push(prop, value.clone());
            }
            return true;
        }
        return expand_shorthand(name, text, &mut push);
    }
    let Some(prop) = longhand(name) else {
        return alias(name, text, wide, &mut push);
    };
    if let Some(value) = wide {
        push(prop, value);
        return true;
    }
    match parse_longhand(prop, text) {
        Some(value) => {
            push(prop, value);
            true
        }
        None => false,
    }
}

fn alias(name: &str, text: &str, wide: Option<Value>, push: &mut impl FnMut(Prop, Value)) -> bool {
    if name != "text-decoration" {
        return false;
    }
    if let Some(value) = wide {
        push(Prop::TextDecoration, value);
        return true;
    }
    // Only the lines are kept from this shorthand; a style or color does not void it.
    let mut lines = None;
    for word in split_words(text) {
        match word.to_ascii_lowercase().as_str() {
            "underline" => lines = Some(lines.unwrap_or(0) | UNDERLINE),
            "overline" => lines = Some(lines.unwrap_or(0) | OVERLINE),
            "line-through" => lines = Some(lines.unwrap_or(0) | LINE_THROUGH),
            "none" => lines = Some(lines.unwrap_or(0)),
            "blink" | "solid" | "double" | "dotted" | "dashed" | "wavy" => {}
            _ if parse_color(word).is_some() => {}
            _ => return false,
        }
    }
    push(Prop::TextDecoration, Value::Decoration(lines.unwrap_or(0)));
    true
}

fn parse_longhand(prop: Prop, text: &str) -> Option<Value> {
    use Prop as P;
    let empty = |word: &str| text.eq_ignore_ascii_case(word).then_some(Value::Empty);
    let length = || parse_length(text, false).map(Value::Length);
    match prop {
        P::Display => Display::parse(text).map(Value::Display),
        P::Float => Float::parse(text).map(Value::Float),
        P::BackgroundRepeat => BackgroundRepeat::parse(text).map(Value::BackgroundRepeat),
        P::BackgroundPositionX | P::BackgroundPositionY => {
            parse_length(text, false).map(Value::Length)
        }
        P::Clear => Clear::parse(text).map(Value::Clear),
        P::Position => Position::parse(text).map(Value::Position),
        P::TextAlign => match text.to_ascii_lowercase().as_str() {
            "-webkit-center" | "-moz-center" => Some(Value::TextAlign(TextAlign::Center)),
            "-webkit-left" | "-moz-left" => Some(Value::TextAlign(TextAlign::Left)),
            "-webkit-right" | "-moz-right" => Some(Value::TextAlign(TextAlign::Right)),
            _ => TextAlign::parse(text).map(Value::TextAlign),
        },
        P::FontStyle => FontStyle::parse(text).map(Value::FontStyle),
        P::WhiteSpace => WhiteSpace::parse(text).map(Value::WhiteSpace),
        P::ListStyleType => ListStyleType::parse(text).map(Value::ListStyleType),
        P::Visibility => Visibility::parse(text).map(Value::Visibility),
        P::TextTransform => TextTransform::parse(text).map(Value::TextTransform),
        P::BoxSizing => BoxSizing::parse(text).map(Value::BoxSizing),
        P::BorderCollapse => BorderCollapse::parse(text).map(Value::BorderCollapse),
        P::Overflow => Overflow::parse(text).map(Value::Overflow),
        P::BorderTopStyle | P::BorderRightStyle | P::BorderBottomStyle | P::BorderLeftStyle => {
            BorderStyle::parse(text).map(Value::BorderStyle)
        }
        P::VerticalAlign => VerticalAlign::parse(text)
            .map(Value::VerticalAlign)
            .or_else(length),
        P::Top
        | P::Right
        | P::Bottom
        | P::Left
        | P::Width
        | P::Height
        | P::MinWidth
        | P::MinHeight
        | P::MarginTop
        | P::MarginRight
        | P::MarginBottom
        | P::MarginLeft => empty("auto").or_else(length),
        P::MaxWidth | P::MaxHeight => empty("none").or_else(length),
        P::PaddingTop
        | P::PaddingRight
        | P::PaddingBottom
        | P::PaddingLeft
        | P::TextIndent
        | P::BorderSpacing => non_negative_or_any(length(), prop),
        P::BorderTopWidth | P::BorderRightWidth | P::BorderBottomWidth | P::BorderLeftWidth => {
            parse_border_width(text)
        }
        P::Color
        | P::BackgroundColor
        | P::BorderTopColor
        | P::BorderRightColor
        | P::BorderBottomColor
        | P::BorderLeftColor => parse_color_value(text),
        P::BackgroundImage | P::ListStyleImage => {
            empty("none").or_else(|| parse_url(text).map(Value::Url))
        }
        P::FontFamily => parse_family(text),
        P::FontSize => parse_font_size(text),
        P::FontWeight => parse_font_weight(text),
        P::LineHeight => empty("normal").or_else(|| {
            text.parse()
                .ok()
                .filter(|number: &f32| number.is_finite() && *number >= 0.0)
                .map(Value::Number)
                .or_else(length)
        }),
        P::TextDecoration => empty("none").map_or_else(
            || {
                let mut lines = 0;
                for word in split_words(text) {
                    lines |= match word.to_ascii_lowercase().as_str() {
                        "underline" => UNDERLINE,
                        "overline" => OVERLINE,
                        "line-through" => LINE_THROUGH,
                        _ => return None,
                    };
                }
                Some(Value::Decoration(lines))
            },
            |_| Some(Value::Decoration(0)),
        ),
    }
}

/// Padding cannot be negative; the others in that group can, or have their own rules.
fn non_negative_or_any(value: Option<Value>, prop: Prop) -> Option<Value> {
    let padding = matches!(
        prop,
        Prop::PaddingTop | Prop::PaddingRight | Prop::PaddingBottom | Prop::PaddingLeft
    );
    match value {
        Some(Value::Length(length)) if padding && length.value < 0.0 => None,
        other => other,
    }
}

fn parse_color_value(text: &str) -> Option<Value> {
    if text.eq_ignore_ascii_case("currentcolor") {
        return Some(Value::CurrentColor);
    }
    parse_color(text).map(Value::Color)
}

fn parse_border_width(text: &str) -> Option<Value> {
    let px = |value: f32| Some(Value::Number(value));
    match text.to_ascii_lowercase().as_str() {
        "thin" => px(1.0),
        "medium" => px(3.0),
        "thick" => px(5.0),
        _ => parse_length(text, false)
            .filter(|length| length.value >= 0.0)
            .map(Value::Length),
    }
}

fn parse_family(text: &str) -> Option<Value> {
    if text.is_empty() {
        return None;
    }
    Some(Value::Family(text.into()))
}

fn parse_font_size(text: &str) -> Option<Value> {
    let px = |value: f32| Some(Value::Number(value));
    match text.to_ascii_lowercase().as_str() {
        "xx-small" => px(9.0),
        "x-small" => px(10.0),
        "small" => px(13.0),
        "medium" => px(16.0),
        "large" => px(18.0),
        "x-large" => px(24.0),
        "xx-large" => px(32.0),
        "xxx-large" => px(48.0),
        "larger" => Some(Value::FontRelative(true)),
        "smaller" => Some(Value::FontRelative(false)),
        _ => parse_length(text, false)
            .filter(|length| length.value >= 0.0)
            .map(Value::Length),
    }
}

fn parse_font_weight(text: &str) -> Option<Value> {
    match text.to_ascii_lowercase().as_str() {
        "normal" => Some(Value::Number(400.0)),
        "bold" => Some(Value::Number(700.0)),
        "bolder" => Some(Value::FontRelative(true)),
        "lighter" => Some(Value::FontRelative(false)),
        _ => text
            .parse()
            .ok()
            .filter(|weight: &f32| (1.0..=1000.0).contains(weight))
            .map(Value::Number),
    }
}

fn shorthand_longhands(name: &str) -> Option<&'static [Prop]> {
    use Prop as P;
    Some(match name {
        "margin" => &[P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft],
        "padding" => &[
            P::PaddingTop,
            P::PaddingRight,
            P::PaddingBottom,
            P::PaddingLeft,
        ],
        "border-width" => &[
            P::BorderTopWidth,
            P::BorderRightWidth,
            P::BorderBottomWidth,
            P::BorderLeftWidth,
        ],
        "border-style" => &[
            P::BorderTopStyle,
            P::BorderRightStyle,
            P::BorderBottomStyle,
            P::BorderLeftStyle,
        ],
        "border-color" => &[
            P::BorderTopColor,
            P::BorderRightColor,
            P::BorderBottomColor,
            P::BorderLeftColor,
        ],
        "border" => &BORDER_ALL,
        "border-top" => &[P::BorderTopWidth, P::BorderTopStyle, P::BorderTopColor],
        "border-right" => &[
            P::BorderRightWidth,
            P::BorderRightStyle,
            P::BorderRightColor,
        ],
        "border-bottom" => &[
            P::BorderBottomWidth,
            P::BorderBottomStyle,
            P::BorderBottomColor,
        ],
        "border-left" => &[P::BorderLeftWidth, P::BorderLeftStyle, P::BorderLeftColor],
        "background" => &[
            P::BackgroundColor,
            P::BackgroundImage,
            P::BackgroundRepeat,
            P::BackgroundPositionX,
            P::BackgroundPositionY,
        ],
        "background-position" => &[P::BackgroundPositionX, P::BackgroundPositionY],
        "font" => &[
            P::FontStyle,
            P::FontWeight,
            P::FontSize,
            P::LineHeight,
            P::FontFamily,
        ],
        "list-style" => &[P::ListStyleType, P::ListStyleImage],
        _ => return None,
    })
}

const BORDER_ALL: [Prop; 12] = [
    Prop::BorderTopWidth,
    Prop::BorderRightWidth,
    Prop::BorderBottomWidth,
    Prop::BorderLeftWidth,
    Prop::BorderTopStyle,
    Prop::BorderRightStyle,
    Prop::BorderBottomStyle,
    Prop::BorderLeftStyle,
    Prop::BorderTopColor,
    Prop::BorderRightColor,
    Prop::BorderBottomColor,
    Prop::BorderLeftColor,
];

fn expand_shorthand(name: &str, text: &str, push: &mut impl FnMut(Prop, Value)) -> bool {
    let Some(longhands) = shorthand_longhands(name) else {
        return false;
    };
    match name {
        "margin" | "padding" | "border-width" | "border-style" | "border-color" => {
            expand_sides(longhands, text, push)
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            expand_border(longhands, text, push)
        }
        "background" => expand_background(text, push),
        "background-position" => match parse_position(&split_words(text)) {
            Some((x, y)) => {
                push(Prop::BackgroundPositionX, Value::Length(x));
                push(Prop::BackgroundPositionY, Value::Length(y));
                true
            }
            None => false,
        },
        "font" => expand_font(text, push),
        "list-style" => expand_list_style(text, push),
        _ => false,
    }
}

fn expand_sides(longhands: &[Prop], text: &str, push: &mut impl FnMut(Prop, Value)) -> bool {
    let words = split_words(text);
    let order: [usize; 4] = match words.len() {
        1 => [0, 0, 0, 0],
        2 => [0, 1, 0, 1],
        3 => [0, 1, 2, 1],
        4 => [0, 1, 2, 3],
        _ => return false,
    };
    let mut values = Vec::with_capacity(4);
    for (side, &prop) in longhands.iter().enumerate() {
        let Some(value) = parse_longhand(prop, words[order[side]]) else {
            return false;
        };
        values.push((prop, value));
    }
    for (prop, value) in values {
        push(prop, value);
    }
    true
}

/// `border` takes a width, a style and a color in any order, all optional.
fn expand_border(longhands: &[Prop], text: &str, push: &mut impl FnMut(Prop, Value)) -> bool {
    let (mut width, mut style, mut color) = (None, None, None);
    for word in split_words(text) {
        if style.is_none()
            && let Some(value) = BorderStyle::parse(word)
        {
            style = Some(Value::BorderStyle(value));
        } else if width.is_none()
            && let Some(value) = parse_border_width(word)
        {
            width = Some(value);
        } else if color.is_none()
            && let Some(value) = parse_color_value(word)
        {
            color = Some(value);
        } else {
            return false;
        }
    }
    let count = longhands.len() / 3;
    for &prop in &longhands[..count] {
        push(prop, width.clone().unwrap_or(Value::Number(3.0)));
    }
    for &prop in &longhands[count..2 * count] {
        push(
            prop,
            style
                .clone()
                .unwrap_or(Value::BorderStyle(BorderStyle::None)),
        );
    }
    for &prop in &longhands[2 * count..] {
        push(prop, color.clone().unwrap_or(Value::CurrentColor));
    }
    true
}

fn percent(value: f32) -> Length {
    Length {
        value,
        unit: Unit::Percent,
    }
}

/// Reads up to two position words, which keywords can give in either order.
fn parse_position(words: &[&str]) -> Option<(Length, Length)> {
    if words.is_empty() || words.len() > 2 {
        return None;
    }
    let keyword = |word: &str| match word.to_ascii_lowercase().as_str() {
        "left" => Some((true, 0.0)),
        "right" => Some((true, 100.0)),
        "top" => Some((false, 0.0)),
        "bottom" => Some((false, 100.0)),
        _ => None,
    };
    let center = |word: &str| word.eq_ignore_ascii_case("center");
    let (mut x, mut y) = (None, None);
    for word in words {
        if let Some((horizontal, value)) = keyword(word) {
            let side = if horizontal { &mut x } else { &mut y };
            if side.is_some() {
                return None;
            }
            *side = Some(percent(value));
        }
    }
    for word in words {
        if keyword(word).is_some() {
            continue;
        }
        let value = if center(word) {
            percent(50.0)
        } else {
            parse_length(word, false)?
        };
        if x.is_none() {
            x = Some(value);
        } else if y.is_none() {
            y = Some(value);
        } else {
            return None;
        }
    }
    Some((x.unwrap_or(percent(50.0)), y.unwrap_or(percent(50.0))))
}

/// `background` takes a color, an image, a repeat and a position. What it cannot use, as its
/// size and attachment, is accepted so that it does not void the declaration.
fn expand_background(text: &str, push: &mut impl FnMut(Prop, Value)) -> bool {
    let (mut color, mut image, mut repeat) = (None, None, None);
    let mut position = Vec::new();
    let mut after_slash = false;
    for word in split_words(text) {
        let lower = word.to_ascii_lowercase();
        if lower == "none" {
            continue;
        }
        if let Some(url) = parse_url(word) {
            image = Some(Value::Url(url));
        } else if let Some(value) = parse_color_value(word) {
            color = Some(value);
        } else if let Some(value) = BackgroundRepeat::parse(word) {
            repeat = Some(value);
        } else if lower == "/" {
            after_slash = true;
        } else if matches!(
            lower.as_str(),
            "space"
                | "round"
                | "scroll"
                | "fixed"
                | "local"
                | "border-box"
                | "padding-box"
                | "content-box"
                | "cover"
                | "contain"
                | "auto"
        ) {
            continue;
        } else if after_slash {
            // The size that follows a slash is not kept.
        } else if matches!(
            lower.as_str(),
            "left" | "right" | "top" | "bottom" | "center"
        ) || parse_length(word, false).is_some()
        {
            position.push(word);
        } else {
            return false;
        }
    }
    let (x, y) = if position.is_empty() {
        (percent(0.0), percent(0.0))
    } else {
        let Some(place) = parse_position(&position) else {
            return false;
        };
        place
    };
    push(
        Prop::BackgroundColor,
        color.unwrap_or(Value::Color(Color::TRANSPARENT)),
    );
    push(Prop::BackgroundImage, image.unwrap_or(Value::Empty));
    push(
        Prop::BackgroundRepeat,
        Value::BackgroundRepeat(repeat.unwrap_or(BackgroundRepeat::Repeat)),
    );
    push(Prop::BackgroundPositionX, Value::Length(x));
    push(Prop::BackgroundPositionY, Value::Length(y));
    true
}

fn expand_font(text: &str, push: &mut impl FnMut(Prop, Value)) -> bool {
    let words = split_words(text);
    let (mut style, mut weight) = (FontStyle::Normal, Value::Number(400.0));
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if word.eq_ignore_ascii_case("normal") || word.eq_ignore_ascii_case("small-caps") {
            index += 1;
        } else if let Some(value) = FontStyle::parse(word) {
            style = value;
            index += 1;
        } else if let Some(value) = parse_font_weight(word) {
            weight = value;
            index += 1;
        } else {
            break;
        }
    }
    let Some(size_word) = words.get(index) else {
        return false;
    };
    let (size_text, line_height) = match size_word.split_once('/') {
        Some((size, line)) => (size, Some(line)),
        None => (*size_word, None),
    };
    let Some(size) = parse_font_size(size_text) else {
        return false;
    };
    let line = match line_height {
        Some(line) => match parse_longhand(Prop::LineHeight, line) {
            Some(value) => value,
            None => return false,
        },
        None => Value::Empty,
    };
    if index + 1 >= words.len() {
        return false;
    }
    let family = words[index + 1..].join(" ");
    let Some(family) = parse_family(&family) else {
        return false;
    };
    push(Prop::FontStyle, Value::FontStyle(style));
    push(Prop::FontWeight, weight);
    push(Prop::FontSize, size);
    push(Prop::LineHeight, line);
    push(Prop::FontFamily, family);
    true
}

fn expand_list_style(text: &str, push: &mut impl FnMut(Prop, Value)) -> bool {
    let (mut kind, mut image) = (None, Value::Empty);
    for word in split_words(text) {
        if word.eq_ignore_ascii_case("inside") || word.eq_ignore_ascii_case("outside") {
            continue;
        }
        if let Some(url) = parse_url(word) {
            image = Value::Url(url);
        } else if let Some(value) = ListStyleType::parse(word) {
            kind = Some(value);
        } else {
            return false;
        }
    }
    push(
        Prop::ListStyleType,
        Value::ListStyleType(kind.unwrap_or(ListStyleType::Disc)),
    );
    push(Prop::ListStyleImage, image);
    true
}

/// Legacy font sizes 1 to 7, from the `<font size>` attribute.
pub(super) fn legacy_font_size(size: i32) -> Value {
    Value::Number(match size.clamp(1, 7) {
        1 => 10.0,
        2 => 13.0,
        3 => 16.0,
        4 => 18.0,
        5 => 24.0,
        6 => 32.0,
        _ => 48.0,
    })
}
