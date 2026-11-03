//! Computed styles: the cascade's winners resolved against the parent's style.

use std::cell::Cell;
use std::sync::Arc;

use super::grid::{GridAreas, GridLine, GridTrack, parse_areas, parse_line, parse_tracks};
use super::math::{CalcRef, CalcSize, keep_calc, parse_transform};
use super::media::Viewport;
use super::properties::{
    BackgroundRepeat, BorderCollapse, BorderStyle, BoxSizing, Clear, Display, FlexAlign,
    FlexDirection, FlexWrap, Float, FontStyle, JustifyContent, ListStyleType, Overflow, Position,
    Prop, TextAlign, TextTransform, Value, VerticalAlign, Visibility, WhiteSpace,
};
use super::values::{Color, Length, Unit};
use super::variables::CustomProperties;

/// A size in CSS pixels or relative to a box that only layout knows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    /// `auto` or `none`, whichever the property's empty state is called.
    Auto,
    Px(f32),
    Percent(f32),
    /// Depends on a percentage in a way that takes more than the above.
    Calc(CalcRef),
}

impl Size {
    /// What a size of what comes from an expression is written as, when it is as simple as can be.
    fn of_calc(calc: CalcSize) -> Option<Self> {
        if let Some(px) = calc.pixels() {
            Some(Self::Px(px))
        } else if let Some(percent) = calc.as_plain_percent() {
            Some(Self::Percent(percent))
        } else {
            keep_calc(calc).map(Self::Calc)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineHeight {
    Normal,
    /// A multiple of the font size.
    Multiple(f32),
    Px(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VerticalPosition {
    Keyword(VerticalAlign),
    Px(f32),
    /// A percentage of the line height.
    Percent(f32),
}

/// Top, right, bottom and left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sides<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T: Copy> Sides<T> {
    const fn all(value: T) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

/// The computed style of an element. Many elements share one.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedStyle {
    // Inherited.
    pub color: Color,
    pub custom_properties: Option<CustomProperties>,
    pub font_family: Arc<str>,
    pub font_size: f32,
    pub font_style: FontStyle,
    pub font_weight: u16,
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub text_indent: Size,
    pub white_space: WhiteSpace,
    pub text_transform: TextTransform,
    pub list_style_type: ListStyleType,
    pub list_style_image: Option<Arc<str>>,
    pub visibility: Visibility,
    pub border_collapse: BorderCollapse,
    pub border_spacing: f32,
    // Not inherited.
    pub display: Display,
    pub float: Float,
    pub clear: Clear,
    pub position: Position,
    pub offsets: Sides<Size>,
    pub overflow: Overflow,
    pub box_sizing: BoxSizing,
    pub vertical_align: VerticalPosition,
    /// `UNDERLINE`, `OVERLINE` and `LINE_THROUGH` bits of this element alone; layout
    /// carries them down to its text.
    pub text_decoration: u8,
    /// Whether a length of this element depends on the size of the view.
    pub uses_view_width: bool,
    pub uses_view_height: bool,
    pub background_color: Color,
    /// What a style sheet says of the inside of SVG shapes, transparent for `none`. Whatever is
    /// left to come from the parent or from the attributes of the SVG is for what draws it.
    pub fill: Option<Color>,
    pub background_image: Option<Arc<str>>,
    pub background_repeat: BackgroundRepeat,
    /// Where the image goes. A percentage aligns that point of the image with that point of
    /// the box.
    pub background_position_x: Size,
    pub background_position_y: Size,
    pub margin: Sides<Size>,
    pub padding: Sides<Size>,
    /// Zero where the border style is `none` or `hidden`.
    pub border_width: Sides<f32>,
    pub border_style: Sides<BorderStyle>,
    pub border_color: Sides<Color>,
    pub width: Size,
    pub height: Size,
    pub min_width: Size,
    pub min_height: Size,
    pub max_width: Size,
    pub max_height: Size,
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub justify_content: JustifyContent,
    pub align_items: FlexAlign,
    pub align_self: FlexAlign,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Size,
    pub order: i32,
    pub row_gap: f32,
    pub column_gap: f32,
    pub z_index: Option<i32>,
    pub grid_template_columns: Option<Arc<[GridTrack]>>,
    pub grid_template_rows: Option<Arc<[GridTrack]>>,
    pub grid_auto_rows: Option<Arc<[GridTrack]>>,
    pub grid_template_areas: Option<Arc<GridAreas>>,
    pub grid_column_start: GridLine,
    pub grid_column_end: GridLine,
    pub grid_row_start: GridLine,
    pub grid_row_end: GridLine,
    pub justify_items: FlexAlign,
    pub justify_self: FlexAlign,
    pub opacity: f32,
    /// How far `transform` moves the box across and down, in percent of its own size. Any transform
    /// counts here, also when it does not move anything.
    pub transform: Option<(Size, Size)>,
}

impl ComputedStyle {
    /// The style of a root element with nothing set, which is also where `initial` looks.
    #[must_use]
    pub fn initial() -> Self {
        Self {
            color: Color::BLACK,
            custom_properties: None,
            font_family: "serif".into(),
            font_size: 16.0,
            font_style: FontStyle::Normal,
            font_weight: 400,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Start,
            text_indent: Size::Px(0.0),
            white_space: WhiteSpace::Normal,
            text_transform: TextTransform::None,
            list_style_type: ListStyleType::Disc,
            list_style_image: None,
            visibility: Visibility::Visible,
            border_collapse: BorderCollapse::Separate,
            border_spacing: 0.0,
            display: Display::Inline,
            float: Float::None,
            clear: Clear::None,
            position: Position::Static,
            offsets: Sides::all(Size::Auto),
            overflow: Overflow::Visible,
            box_sizing: BoxSizing::ContentBox,
            vertical_align: VerticalPosition::Keyword(VerticalAlign::Baseline),
            text_decoration: 0,
            uses_view_width: false,
            uses_view_height: false,
            background_color: Color::TRANSPARENT,
            fill: None,
            background_image: None,
            background_repeat: BackgroundRepeat::Repeat,
            background_position_x: Size::Percent(0.0),
            background_position_y: Size::Percent(0.0),
            margin: Sides::all(Size::Px(0.0)),
            padding: Sides::all(Size::Px(0.0)),
            border_width: Sides::all(3.0),
            border_style: Sides::all(BorderStyle::None),
            border_color: Sides::all(Color::BLACK),
            width: Size::Auto,
            height: Size::Auto,
            // Means zero, apart from for a flex item, which then is not narrower than its content.
            min_width: Size::Auto,
            min_height: Size::Px(0.0),
            max_width: Size::Auto,
            max_height: Size::Auto,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Nowrap,
            justify_content: JustifyContent::Normal,
            align_items: FlexAlign::Normal,
            align_self: FlexAlign::Auto,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Size::Auto,
            order: 0,
            row_gap: 0.0,
            column_gap: 0.0,
            z_index: None,
            grid_template_columns: None,
            grid_template_rows: None,
            grid_auto_rows: None,
            grid_template_areas: None,
            grid_column_start: GridLine::Auto,
            grid_column_end: GridLine::Auto,
            grid_row_start: GridLine::Auto,
            grid_row_end: GridLine::Auto,
            justify_items: FlexAlign::Normal,
            justify_self: FlexAlign::Auto,
            opacity: 1.0,
            transform: None,
        }
    }

    /// A new style that inherits from `parent` and has the initial values of the rest.
    fn inheriting(parent: &Self) -> Self {
        Self {
            color: parent.color,
            custom_properties: parent.custom_properties.clone(),
            font_family: parent.font_family.clone(),
            font_size: parent.font_size,
            font_style: parent.font_style,
            font_weight: parent.font_weight,
            line_height: parent.line_height,
            text_align: parent.text_align,
            text_indent: parent.text_indent,
            white_space: parent.white_space,
            text_transform: parent.text_transform,
            list_style_type: parent.list_style_type,
            list_style_image: parent.list_style_image.clone(),
            visibility: parent.visibility,
            border_collapse: parent.border_collapse,
            border_spacing: parent.border_spacing,
            ..Self::initial()
        }
    }
}

/// What lengths are relative to while one element is resolved.
struct Context<'a> {
    font_size: f32,
    root_font_size: f32,
    viewport: Viewport,
    /// Set when a length turns out to depend on the width and on the height of the view.
    used_viewport: &'a Cell<(bool, bool)>,
}

impl Context<'_> {
    fn pixels(&self, length: Length) -> Option<f32> {
        let value = length.value;
        let (width, height) = self.used_viewport.get();
        self.used_viewport.set((
            width || matches!(length.unit, Unit::Vw | Unit::Vmin | Unit::Vmax),
            height || matches!(length.unit, Unit::Vh | Unit::Vmin | Unit::Vmax),
        ));
        Some(match length.unit {
            Unit::Px => value,
            Unit::Em => value * self.font_size,
            Unit::Rem => value * self.root_font_size,
            // Without font metrics, both are taken as half the font size.
            Unit::Ex | Unit::Ch => value * self.font_size * 0.5,
            Unit::Vw => value * self.viewport.width / 100.0,
            Unit::Vh => value * self.viewport.height / 100.0,
            Unit::Vmin => value * self.viewport.width.min(self.viewport.height) / 100.0,
            Unit::Vmax => value * self.viewport.width.max(self.viewport.height) / 100.0,
            Unit::Percent => return None,
        })
    }

    fn size(&self, value: &Value) -> Option<Size> {
        match value {
            Value::Empty => Some(Size::Auto),
            Value::Length(length) if length.unit == Unit::Percent => {
                Some(Size::Percent(length.value))
            }
            Value::Length(length) => self.pixels(*length).map(Size::Px),
            Value::Math(math) => math
                .size(&|length| self.pixels(length))
                .and_then(Size::of_calc),
            _ => None,
        }
    }
}

/// Resolves the winning value of each property, given as an array indexed by [`Prop`].
pub(super) fn compute(
    parent: &ComputedStyle,
    cascaded: &[Option<&Value>],
    root_font_size: Option<f32>,
    viewport: Viewport,
) -> ComputedStyle {
    let initial = ComputedStyle::initial();
    let mut style = ComputedStyle::inheriting(parent);
    let used_viewport = Cell::new((false, false));
    let get = |prop: Prop| cascaded[prop as usize];
    let font_size = match get(Prop::FontSize) {
        None => inherited_font_size(parent, get(Prop::FontFamily)),
        Some(Value::Inherit) => parent.font_size,
        Some(Value::Initial) => initial.font_size,
        Some(Value::Number(px)) => *px,
        Some(Value::FontRelative(true)) => parent.font_size * 1.2,
        Some(Value::FontRelative(false)) => parent.font_size / 1.2,
        Some(Value::Length(length)) => {
            let relative = Context {
                font_size: parent.font_size,
                root_font_size: root_font_size.unwrap_or(16.0),
                viewport,
                used_viewport: &used_viewport,
            };
            match length.unit {
                Unit::Percent => parent.font_size * length.value / 100.0,
                _ => relative.pixels(*length).unwrap_or(parent.font_size),
            }
        }
        Some(Value::Math(math)) => {
            let relative = Context {
                font_size: parent.font_size,
                root_font_size: root_font_size.unwrap_or(16.0),
                viewport,
                used_viewport: &used_viewport,
            };
            math.size(&|length| relative.pixels(length))
                .map_or(parent.font_size, |size| size.of(parent.font_size).max(0.0))
        }
        Some(_) => parent.font_size,
    };
    style.font_size = font_size;
    let context = Context {
        font_size,
        root_font_size: root_font_size.unwrap_or(font_size),
        viewport,
        used_viewport: &used_viewport,
    };
    // The color comes first, as other properties can refer to it.
    match get(Prop::Color) {
        Some(Value::Color(color)) => style.color = *color,
        Some(Value::Initial) => style.color = initial.color,
        _ => {}
    }
    style.border_color = Sides::all(style.color);
    for (index, value) in cascaded.iter().enumerate() {
        if let Some(value) = value {
            apply(
                super::properties::ALL_PROPERTIES[index],
                value,
                &mut style,
                parent,
                &initial,
                &context,
            );
        }
    }
    (style.uses_view_width, style.uses_view_height) = used_viewport.get();
    if style.float != Float::None || matches!(style.position, Position::Absolute | Position::Fixed)
    {
        style.display = blockified(style.display);
    }
    let no_line = |border: BorderStyle| matches!(border, BorderStyle::None | BorderStyle::Hidden);
    if no_line(style.border_style.top) {
        style.border_width.top = 0.0;
    }
    if no_line(style.border_style.right) {
        style.border_width.right = 0.0;
    }
    if no_line(style.border_style.bottom) {
        style.border_width.bottom = 0.0;
    }
    if no_line(style.border_style.left) {
        style.border_width.left = 0.0;
    }
    style
}

/// Browsers show the generic monospace font smaller than others at the same size. Firefox on
/// Linux shows it at 12 pixels where others have 16, and this follows it. The size only
/// changes when it is inherited across a change between monospace and something else.
const MONOSPACE_RATIO: f32 = 0.75;

fn is_generic_monospace(family: &str) -> bool {
    family.eq_ignore_ascii_case("monospace")
}

fn inherited_font_size(parent: &ComputedStyle, family: Option<&Value>) -> f32 {
    let was = is_generic_monospace(&parent.font_family);
    let is = match family {
        Some(Value::Family(family)) => is_generic_monospace(family),
        _ => was,
    };
    match (was, is) {
        (false, true) => parent.font_size * MONOSPACE_RATIO,
        (true, false) => parent.font_size / MONOSPACE_RATIO,
        _ => parent.font_size,
    }
}

/// Floats, positioned boxes and root elements cannot be inline.
pub(super) fn blockified(display: Display) -> Display {
    match display {
        Display::Inline | Display::InlineBlock => Display::Block,
        Display::InlineTable => Display::Table,
        Display::InlineFlex => Display::Flex,
        Display::InlineGrid => Display::Grid,
        other => other,
    }
}

fn tracks_of(text: &str, context: &Context) -> Option<Arc<[GridTrack]>> {
    parse_tracks(text, &|length| context.pixels(length)).map(Arc::from)
}

fn line_of(text: &str) -> GridLine {
    parse_line(text).unwrap_or(GridLine::Auto)
}

fn apply(
    prop: Prop,
    value: &Value,
    style: &mut ComputedStyle,
    parent: &ComputedStyle,
    initial: &ComputedStyle,
    context: &Context,
) {
    use Prop as P;
    use Value as V;
    // Copies one field for `inherit` and `initial`, and otherwise reads the value.
    macro_rules! set {
        ($($field:ident).+, $pattern:pat => $result:expr) => {
            match value {
                V::Inherit => style.$($field).+ = parent.$($field).+.clone(),
                V::Initial => style.$($field).+ = initial.$($field).+.clone(),
                $pattern => style.$($field).+ = $result,
                _ => {}
            }
        };
    }
    macro_rules! size {
        ($($field:ident).+) => {
            match value {
                V::Inherit => style.$($field).+ = parent.$($field).+,
                V::Initial => style.$($field).+ = initial.$($field).+,
                other => {
                    if let Some(size) = context.size(other) {
                        style.$($field).+ = size;
                    }
                }
            }
        };
    }
    macro_rules! width {
        ($($field:ident).+) => {
            match value {
                V::Inherit => style.$($field).+ = parent.$($field).+,
                V::Initial => style.$($field).+ = initial.$($field).+,
                V::Number(px) => style.$($field).+ = *px,
                V::Length(length) => {
                    if let Some(px) = context.pixels(*length) {
                        style.$($field).+ = px;
                    }
                }
                V::Math(math) => {
                    // Percentages do not mean anything for these.
                    let size = math.size(&|length| context.pixels(length));
                    if let Some(px) = size.and_then(|size| size.pixels()) {
                        style.$($field).+ = px.max(0.0);
                    }
                }
                _ => {}
            }
        };
    }
    macro_rules! border_color {
        ($side:ident) => {
            match value {
                V::Inherit => style.border_color.$side = parent.border_color.$side,
                V::Color(color) => style.border_color.$side = *color,
                _ => style.border_color.$side = style.color,
            }
        };
    }
    match prop {
        // Resolved before the others.
        P::FontSize => {}
        P::Color => {
            if matches!(value, V::Inherit | V::CurrentColor) {
                style.color = parent.color;
            }
        }
        P::Display => set!(display, V::Display(display) => *display),
        P::Float => set!(float, V::Float(float) => *float),
        P::Clear => set!(clear, V::Clear(clear) => *clear),
        P::Position => set!(position, V::Position(position) => *position),
        P::Top => size!(offsets.top),
        P::Right => size!(offsets.right),
        P::Bottom => size!(offsets.bottom),
        P::Left => size!(offsets.left),
        P::BackgroundColor => match value {
            V::Inherit => style.background_color = parent.background_color,
            V::Initial => style.background_color = initial.background_color,
            V::CurrentColor => style.background_color = style.color,
            V::Color(color) => style.background_color = *color,
            _ => {}
        },
        P::Fill => match value {
            V::Inherit => style.fill = parent.fill,
            V::Initial => style.fill = initial.fill,
            V::CurrentColor => style.fill = Some(style.color),
            V::Color(color) => style.fill = Some(*color),
            _ => {}
        },
        P::BackgroundImage => match value {
            V::Inherit => style.background_image.clone_from(&parent.background_image),
            V::Initial | V::Empty => style.background_image = None,
            V::Url(url) => style.background_image = Some(url.clone()),
            _ => {}
        },
        P::BackgroundRepeat => set!(background_repeat, V::BackgroundRepeat(repeat) => *repeat),
        P::BackgroundPositionX => size!(background_position_x),
        P::BackgroundPositionY => size!(background_position_y),
        P::FontFamily => set!(font_family, V::Family(family) => family.clone()),
        P::FontStyle => set!(font_style, V::FontStyle(font_style) => *font_style),
        P::FontWeight => match value {
            V::Inherit => style.font_weight = parent.font_weight,
            V::Initial => style.font_weight = initial.font_weight,
            V::FontRelative(true) => {
                style.font_weight = match parent.font_weight {
                    0..=349 => 400,
                    350..=549 => 700,
                    _ => 900,
                };
            }
            V::FontRelative(false) => {
                style.font_weight = match parent.font_weight {
                    0..=549 => 100,
                    550..=749 => 400,
                    _ => 700,
                };
            }
            // Weights were checked to be between 1 and 1000 when read.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            V::Number(weight) => style.font_weight = *weight as u16,
            _ => {}
        },
        P::LineHeight => match value {
            V::Inherit => style.line_height = parent.line_height,
            V::Initial | V::Empty => style.line_height = LineHeight::Normal,
            V::Number(multiple) => style.line_height = LineHeight::Multiple(*multiple),
            V::Length(length) if length.unit == Unit::Percent => {
                style.line_height = LineHeight::Px(context.font_size * length.value / 100.0);
            }
            V::Length(length) => {
                if let Some(px) = context.pixels(*length) {
                    style.line_height = LineHeight::Px(px);
                }
            }
            V::Math(math) => {
                let size = math.size(&|length| context.pixels(length));
                if let Some(size) = size {
                    style.line_height = LineHeight::Px(size.of(context.font_size).max(0.0));
                }
            }
            _ => {}
        },
        P::TextAlign => set!(text_align, V::TextAlign(align) => *align),
        P::TextIndent => size!(text_indent),
        P::WhiteSpace => set!(white_space, V::WhiteSpace(space) => *space),
        P::TextDecoration => set!(text_decoration, V::Decoration(lines) => *lines),
        P::TextTransform => set!(text_transform, V::TextTransform(transform) => *transform),
        P::VerticalAlign => match value {
            V::Inherit => style.vertical_align = parent.vertical_align,
            V::Initial => style.vertical_align = initial.vertical_align,
            V::VerticalAlign(keyword) => {
                style.vertical_align = VerticalPosition::Keyword(*keyword);
            }
            V::Length(length) if length.unit == Unit::Percent => {
                style.vertical_align = VerticalPosition::Percent(length.value);
            }
            V::Length(length) => {
                if let Some(px) = context.pixels(*length) {
                    style.vertical_align = VerticalPosition::Px(px);
                }
            }
            _ => {}
        },
        P::ListStyleType => set!(list_style_type, V::ListStyleType(kind) => *kind),
        P::ListStyleImage => match value {
            V::Inherit => style.list_style_image.clone_from(&parent.list_style_image),
            V::Initial | V::Empty => style.list_style_image = None,
            V::Url(url) => style.list_style_image = Some(url.clone()),
            _ => {}
        },
        P::Visibility => set!(visibility, V::Visibility(visibility) => *visibility),
        P::Overflow => set!(overflow, V::Overflow(overflow) => *overflow),
        P::BoxSizing => set!(box_sizing, V::BoxSizing(sizing) => *sizing),
        P::BorderCollapse => set!(border_collapse, V::BorderCollapse(collapse) => *collapse),
        P::BorderSpacing => width!(border_spacing),
        P::MarginTop => size!(margin.top),
        P::MarginRight => size!(margin.right),
        P::MarginBottom => size!(margin.bottom),
        P::MarginLeft => size!(margin.left),
        P::PaddingTop => size!(padding.top),
        P::PaddingRight => size!(padding.right),
        P::PaddingBottom => size!(padding.bottom),
        P::PaddingLeft => size!(padding.left),
        P::BorderTopWidth => width!(border_width.top),
        P::BorderRightWidth => width!(border_width.right),
        P::BorderBottomWidth => width!(border_width.bottom),
        P::BorderLeftWidth => width!(border_width.left),
        P::BorderTopStyle => set!(border_style.top, V::BorderStyle(line) => *line),
        P::BorderRightStyle => set!(border_style.right, V::BorderStyle(line) => *line),
        P::BorderBottomStyle => set!(border_style.bottom, V::BorderStyle(line) => *line),
        P::BorderLeftStyle => set!(border_style.left, V::BorderStyle(line) => *line),
        P::BorderTopColor => border_color!(top),
        P::BorderRightColor => border_color!(right),
        P::BorderBottomColor => border_color!(bottom),
        P::BorderLeftColor => border_color!(left),
        P::Width => size!(width),
        P::Height => size!(height),
        P::MinWidth => size!(min_width),
        P::MinHeight => size!(min_height),
        P::MaxWidth => size!(max_width),
        P::MaxHeight => size!(max_height),
        P::FlexDirection => set!(flex_direction, V::FlexDirection(direction) => *direction),
        P::FlexWrap => set!(flex_wrap, V::FlexWrap(wrap) => *wrap),
        P::JustifyContent => set!(justify_content, V::JustifyContent(justify) => *justify),
        P::AlignItems => set!(align_items, V::FlexAlign(align) => *align),
        P::AlignSelf => set!(align_self, V::FlexAlign(align) => *align),
        P::FlexGrow => set!(flex_grow, V::Number(grow) => *grow),
        P::FlexShrink => set!(flex_shrink, V::Number(shrink) => *shrink),
        P::FlexBasis => size!(flex_basis),
        // It was read as a whole number, so nothing is lost.
        #[allow(clippy::cast_possible_truncation)]
        P::Order => set!(order, V::Number(order) => *order as i32),
        P::RowGap => width!(row_gap),
        P::ColumnGap => width!(column_gap),
        P::GridTemplateColumns => {
            set!(grid_template_columns, V::GridText(text) => tracks_of(text, context));
        }
        P::GridTemplateRows => {
            set!(grid_template_rows, V::GridText(text) => tracks_of(text, context))
        }
        P::GridAutoRows => set!(grid_auto_rows, V::GridText(text) => tracks_of(text, context)),
        P::GridTemplateAreas => match value {
            V::Inherit => style
                .grid_template_areas
                .clone_from(&parent.grid_template_areas),
            V::Initial | V::Empty => style.grid_template_areas = None,
            V::GridText(text) => style.grid_template_areas = parse_areas(text).map(Arc::new),
            _ => {}
        },
        P::GridColumnStart => set!(grid_column_start, V::GridText(text) => line_of(text)),
        P::GridColumnEnd => set!(grid_column_end, V::GridText(text) => line_of(text)),
        P::GridRowStart => set!(grid_row_start, V::GridText(text) => line_of(text)),
        P::GridRowEnd => set!(grid_row_end, V::GridText(text) => line_of(text)),
        P::JustifyItems => set!(justify_items, V::FlexAlign(align) => *align),
        P::JustifySelf => set!(justify_self, V::FlexAlign(align) => *align),
        P::Opacity => set!(opacity, V::Number(opacity) => *opacity),
        P::Transform => match value {
            V::Inherit => style.transform = parent.transform,
            V::Initial | V::Empty => style.transform = None,
            V::GridText(text) => {
                let moved = parse_transform(text, &|length| context.pixels(length));
                if let Some((across, down)) = moved {
                    let of = |size| Size::of_calc(size).unwrap_or(Size::Px(0.0));
                    style.transform = Some((of(across), of(down)));
                }
            }
            _ => {}
        },
        P::ZIndex => match value {
            V::Inherit => style.z_index = parent.z_index,
            V::Initial | V::Empty => style.z_index = None,
            // It was read as a whole number, so nothing is lost.
            #[allow(clippy::cast_possible_truncation)]
            V::Number(index) => style.z_index = Some(*index as i32),
            _ => {}
        },
    }
}
