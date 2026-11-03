//! The cascade: finds the rules that match each element and computes its style. Elements
//! with the same parent style and the same matched rules share one computed style.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hasher};
use std::sync::LazyLock;

use crate::html::{Document, NodeId, NodeKind, Tag};

use super::media::{Viewport, matches_media};
use super::properties::{Compiled, PROPERTY_COUNT, Prop, Value, compile, legacy_font_size};
use super::selector::{MatchContext, PseudoClass, Selector, SimpleSelector, TypeSelector};
use super::style::{ComputedStyle, blockified, compute};
use super::stylesheet::Stylesheet;
use super::values::{Color, Length, Unit, parse_length};

/// The browser's own defaults, following the rendering section of the HTML standard.
const DEFAULT_STYLES: &str = include_str!("default.css");

/// Added to the defaults for documents in quirks mode.
const QUIRKS_STYLES: &str = include_str!("quirks.css");

/// Distinct styles beyond this reuse the parent's, so that hostile pages stay small.
const MAX_STYLES: usize = 1 << 20;

static DEFAULT_SHEET: LazyLock<Stylesheet> = LazyLock::new(|| Stylesheet::parse(DEFAULT_STYLES));
static QUIRKS_SHEET: LazyLock<Stylesheet> = LazyLock::new(|| Stylesheet::parse(QUIRKS_STYLES));
static QUIRKS_RULES: LazyLock<Vec<Vec<Compiled>>> = LazyLock::new(|| {
    QUIRKS_SHEET
        .rules
        .iter()
        .map(|rule| compile_rule(&QUIRKS_SHEET, rule))
        .collect()
});
static DEFAULT_RULES: LazyLock<Vec<Vec<Compiled>>> = LazyLock::new(|| {
    DEFAULT_SHEET
        .rules
        .iter()
        .map(|rule| compile_rule(&DEFAULT_SHEET, rule))
        .collect()
});

fn compile_rule(sheet: &Stylesheet, rule: &super::stylesheet::StyleRule) -> Vec<Compiled> {
    let mut compiled = Vec::new();
    for declaration in &rule.declarations {
        compile(
            &declaration.name,
            sheet.value(declaration),
            declaration.important,
            &mut compiled,
        );
    }
    compiled.shrink_to_fit();
    compiled
}

/// One selector of a rule, filed under the name that any element it matches must have.
#[derive(Clone, Copy)]
struct IndexEntry<'a> {
    selector: &'a Selector,
    rule: u32,
    specificity: u32,
}

#[derive(Default)]
struct RuleIndex<'a> {
    ids: HashMap<String, Vec<IndexEntry<'a>>>,
    classes: HashMap<String, Vec<IndexEntry<'a>>>,
    tags: HashMap<String, Vec<IndexEntry<'a>>>,
    others: Vec<IndexEntry<'a>>,
    uses_positions: bool,
}

impl<'a> RuleIndex<'a> {
    fn add(&mut self, selector: &'a Selector, rule: u32, quirks: bool) {
        // Styles of parts such as `::before` belong to boxes that layout does not make yet.
        if selector.pseudo_element.is_some() {
            return;
        }
        self.uses_positions |= selector_uses_positions(selector);
        let entry = IndexEntry {
            selector,
            rule,
            specificity: selector.specificity,
        };
        let simple = selector.compounds.first().map_or(&[][..], |c| &c.simple);
        let folded = |name: &str| {
            if quirks {
                name.to_ascii_lowercase()
            } else {
                name.to_owned()
            }
        };
        let id = simple.iter().find_map(|s| match s {
            SimpleSelector::Id(id) => Some(folded(id)),
            _ => None,
        });
        if let Some(id) = id {
            self.ids.entry(id).or_default().push(entry);
            return;
        }
        let class = simple.iter().find_map(|s| match s {
            SimpleSelector::Class(class) => Some(folded(class)),
            _ => None,
        });
        if let Some(class) = class {
            self.classes.entry(class).or_default().push(entry);
            return;
        }
        let tag = simple.iter().find_map(|s| match s {
            SimpleSelector::Type(TypeSelector { name, .. }) => Some(name.to_string()),
            _ => None,
        });
        match tag {
            Some(tag) => self.tags.entry(tag).or_default().push(entry),
            None => self.others.push(entry),
        }
    }
}

fn selector_uses_positions(selector: &Selector) -> bool {
    selector.compounds.iter().any(|compound| {
        compound.simple.iter().any(|simple| match simple {
            SimpleSelector::Pseudo(pseudo) => match pseudo {
                PseudoClass::Nth { .. }
                | PseudoClass::FirstOfType
                | PseudoClass::LastOfType
                | PseudoClass::OnlyOfType => true,
                PseudoClass::Not(list) | PseudoClass::Is(list) => {
                    list.iter().any(selector_uses_positions)
                }
                _ => false,
            },
            _ => false,
        })
    })
}

/// The computed styles of a document's elements.
pub struct Styles {
    styles: Vec<ComputedStyle>,
    /// The style index of each node, by node index.
    of_node: Vec<u32>,
    limited: bool,
}

impl Styles {
    /// The style of an element, or for any other node the style it inherits.
    #[must_use]
    pub fn get(&self, id: NodeId) -> &ComputedStyle {
        &self.styles[self.of_node[id.index()] as usize]
    }

    /// How many distinct styles the document has.
    #[must_use]
    pub fn distinct_count(&self) -> usize {
        self.styles.len()
    }

    /// Bytes allocated for the styles and the table of which node has which.
    #[must_use]
    pub fn capacity_bytes(&self) -> usize {
        self.styles.capacity() * size_of::<ComputedStyle>()
            + self.of_node.capacity() * size_of::<u32>()
    }

    /// Whether a limit stopped the cascade from finishing its work for every element.
    #[must_use]
    pub fn is_limited(&self) -> bool {
        self.limited
    }
}

/// The contents of a document's `<style>` elements that apply to this viewport, in order.
#[must_use]
pub fn document_stylesheets(document: &Document, viewport: Viewport) -> Vec<Stylesheet> {
    document
        .descendants(document.root())
        .filter(|&id| document.tag(id) == Tag::Style)
        .filter(|&id| {
            document
                .attributes(id)
                .find(|attribute| attribute.name.eq_ignore_ascii_case("media"))
                .and_then(|attribute| attribute.value)
                .is_none_or(|media| matches_media(media, viewport))
        })
        .filter_map(|id| document.first_child(id))
        .map(|text| Stylesheet::parse(&document.decoded_text(text)))
        .collect()
}

/// Computes the style of every element from the default styles and these author sheets.
/// Walks the tree in document order, without recursion.
#[must_use]
pub fn compute_styles(document: &Document, authors: &[Stylesheet], viewport: Viewport) -> Styles {
    let quirks = document.is_quirks();
    let mut rules: Vec<(u8, &[Compiled])> = Vec::new();
    let mut compiled_authors: Vec<Vec<Vec<Compiled>>> = Vec::with_capacity(authors.len());
    for sheet in authors {
        compiled_authors.push(
            sheet
                .rules
                .iter()
                .map(|rule| compile_rule(sheet, rule))
                .collect(),
        );
    }
    let mut index = RuleIndex::default();
    let mut add_sheet = |sheet: &'static Stylesheet, compiled: &'static [Vec<Compiled>]| {
        for (rule, declarations) in sheet.rules.iter().zip(compiled) {
            let number = u32::try_from(rules.len()).unwrap_or(u32::MAX);
            rules.push((ORIGIN_DEFAULT, declarations));
            for selector in &rule.selectors {
                index.add(selector, number, quirks);
            }
        }
    };
    add_sheet(&DEFAULT_SHEET, &DEFAULT_RULES);
    if quirks {
        add_sheet(&QUIRKS_SHEET, &QUIRKS_RULES);
    }
    for (sheet, compiled) in authors.iter().zip(&compiled_authors) {
        for (rule, declarations) in sheet.rules.iter().zip(compiled) {
            if rule
                .media
                .is_some_and(|media| !sheet.media_applies(media, viewport))
            {
                continue;
            }
            let number = u32::try_from(rules.len()).unwrap_or(u32::MAX);
            rules.push((ORIGIN_AUTHOR, declarations));
            for selector in &rule.selectors {
                index.add(selector, number, quirks);
            }
        }
    }

    let mut context = MatchContext::new(document, index.uses_positions);
    let initial = ComputedStyle::initial();
    let mut result = Styles {
        styles: vec![initial],
        of_node: vec![0; document.node_count()],
        limited: false,
    };
    let mut cache: HashMap<(u32, Vec<u32>, &str), u32> = HashMap::new();
    let mut by_fingerprint: HashMap<u64, u32> = HashMap::new();
    let mut matched: Vec<(u32, u32)> = Vec::new();
    let mut root_font_size = None;
    for id in document.descendants(document.root()).skip(1) {
        let parent = document.parent(id).unwrap_or(document.root());
        let parent_style = result.of_node[parent.index()];
        if document.kind(id) != NodeKind::Element {
            result.of_node[id.index()] = parent_style;
            continue;
        }
        matched.clear();
        collect_matches(document, id, &index, &mut context, &mut matched);
        // Sort by specificity, then by source order, so that later entries win.
        matched.sort_unstable();
        matched.dedup();
        let hints = presentational_hints(document, id);
        let style_attribute = document
            .attributes(id)
            .find(|attribute| attribute.name.eq_ignore_ascii_case("style"))
            .filter(|attribute| attribute.value.is_some())
            .map(|attribute| attribute.decoded_value());
        // What the source of the tag holds decides the hints and the style attribute.
        let tag_text = if hints.is_empty() && style_attribute.is_none() {
            ""
        } else {
            document.text(id)
        };
        let key = (
            parent_style,
            matched.iter().map(|&(_, rule)| rule).collect::<Vec<u32>>(),
            tag_text,
        );
        if let Some(&style) = cache.get(&key) {
            result.of_node[id.index()] = style;
            continue;
        }
        if result.styles.len() >= MAX_STYLES {
            result.limited = true;
            result.of_node[id.index()] = parent_style;
            continue;
        }
        let mut inline = Vec::new();
        if let Some(text) = style_attribute {
            let sheet = Stylesheet::parse_declarations(&text);
            for declaration in sheet.declarations() {
                compile(
                    &declaration.name,
                    sheet.value(declaration),
                    declaration.important,
                    &mut inline,
                );
            }
        }
        let mut cascaded: [Option<&Value>; PROPERTY_COUNT] = [None; PROPERTY_COUNT];
        apply_origin(&mut cascaded, &matched, &rules, ORIGIN_DEFAULT, false);
        apply_list(&mut cascaded, &hints, false);
        apply_origin(&mut cascaded, &matched, &rules, ORIGIN_AUTHOR, false);
        apply_list(&mut cascaded, &inline, false);
        apply_origin(&mut cascaded, &matched, &rules, ORIGIN_AUTHOR, true);
        apply_list(&mut cascaded, &inline, true);
        apply_origin(&mut cascaded, &matched, &rules, ORIGIN_DEFAULT, true);
        let mut style = compute(
            &result.styles[parent_style as usize],
            &cascaded,
            root_font_size,
            viewport,
        );
        if parent == document.root() {
            style.display = blockified(style.display);
            root_font_size = Some(style.font_size);
        }
        // Different tags often come to the same style, such as images with other sources.
        let fingerprint = fingerprint_of(&style);
        let same = by_fingerprint
            .get(&fingerprint)
            .copied()
            .filter(|&number| result.styles[number as usize] == style);
        let number = same.unwrap_or_else(|| {
            let number = u32::try_from(result.styles.len()).unwrap_or(u32::MAX);
            result.styles.push(style);
            by_fingerprint.insert(fingerprint, number);
            number
        });
        cache.insert(key, number);
        result.of_node[id.index()] = number;
    }
    result.limited |= context.is_limited();
    result.styles.shrink_to_fit();
    result
}

/// Feeds what is written into a hasher, so that nothing needs to be stored.
struct HashWriter(DefaultHasher);

impl std::fmt::Write for HashWriter {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0.write(text.as_bytes());
        Ok(())
    }
}

fn fingerprint_of(style: &ComputedStyle) -> u64 {
    let mut writer = HashWriter(DefaultHasher::new());
    // Writing to a hasher cannot fail. Styles hold floats, so they do not implement `Hash`.
    let _ = write!(writer, "{style:?}");
    writer.0.finish()
}

const ORIGIN_DEFAULT: u8 = 0;
const ORIGIN_AUTHOR: u8 = 1;

fn apply_origin<'a>(
    cascaded: &mut [Option<&'a Value>; PROPERTY_COUNT],
    matched: &[(u32, u32)],
    rules: &[(u8, &'a [Compiled])],
    origin: u8,
    important: bool,
) {
    for &(_, rule) in matched {
        let (rule_origin, declarations) = rules[rule as usize];
        if rule_origin == origin {
            apply_list(cascaded, declarations, important);
        }
    }
}

fn apply_list<'a>(
    cascaded: &mut [Option<&'a Value>; PROPERTY_COUNT],
    declarations: &'a [Compiled],
    important: bool,
) {
    for declaration in declarations {
        if declaration.important == important {
            cascaded[declaration.prop as usize] = Some(&declaration.value);
        }
    }
}

fn collect_matches(
    document: &Document,
    id: NodeId,
    index: &RuleIndex<'_>,
    context: &mut MatchContext<'_>,
    matched: &mut Vec<(u32, u32)>,
) {
    let mut test = |entries: Option<&Vec<IndexEntry<'_>>>, context: &mut MatchContext<'_>| {
        for entry in entries.into_iter().flatten() {
            if context.matches(id, entry.selector) {
                matched.push((entry.specificity, entry.rule));
            }
        }
    };
    let quirks = document.is_quirks();
    for attribute in document.attributes(id) {
        if attribute.name.eq_ignore_ascii_case("id") {
            let value = attribute.decoded_value();
            if quirks {
                test(index.ids.get(&value.to_ascii_lowercase()), context);
            } else {
                test(index.ids.get(value.as_ref()), context);
            }
            break;
        }
    }
    for attribute in document.attributes(id) {
        if attribute.name.eq_ignore_ascii_case("class") {
            let value = attribute.decoded_value();
            for class in value.split_ascii_whitespace() {
                if quirks {
                    test(index.classes.get(&class.to_ascii_lowercase()), context);
                } else {
                    test(index.classes.get(class), context);
                }
            }
            break;
        }
    }
    test(index.tags.get(document.local_name(id).as_ref()), context);
    let lower = document.name(id).to_ascii_lowercase();
    if lower != document.local_name(id) {
        test(index.tags.get(&lower), context);
    }
    test(Some(&index.others), context);
}

fn hint(prop: Prop, value: Value) -> Compiled {
    Compiled {
        prop,
        value,
        important: false,
    }
}

fn pixels(value: f32) -> Value {
    Value::Length(Length {
        value,
        unit: Unit::Px,
    })
}

/// Reads an HTML dimension: `60%`, `100` or `100px`.
fn dimension(text: &str) -> Option<Value> {
    let text = text.trim();
    if let Some(percent) = text.strip_suffix('%') {
        return parse_length(&format!("{percent}%"), false).map(Value::Length);
    }
    let digits = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .map_or(text, |end| &text[..end]);
    digits
        .parse()
        .ok()
        .filter(|number: &f32| number.is_finite())
        .map(pixels)
}

/// Reads a legacy color attribute, which may leave out the `#`.
fn legacy_color(text: &str) -> Option<Value> {
    let text = text.trim();
    Color::from_name_or_hash(text)
        .or_else(|| Color::from_hex(text))
        .map(Value::Color)
}

/// The style that old presentational attributes such as `bgcolor` ask for.
fn presentational_hints(document: &Document, id: NodeId) -> Vec<Compiled> {
    let tag = document.tag(id);
    if tag == Tag::Other {
        return Vec::new();
    }
    let mut hints = Vec::new();
    for attribute in document.attributes(id) {
        let Some(raw) = attribute.value else {
            if attribute.name.eq_ignore_ascii_case("nowrap") && matches!(tag, Tag::Td | Tag::Th) {
                hints.push(hint(
                    Prop::WhiteSpace,
                    Value::WhiteSpace(super::properties::WhiteSpace::Nowrap),
                ));
            }
            continue;
        };
        let value = attribute.decoded_value();
        let value = value.trim();
        let name = attribute.name.to_ascii_lowercase();
        match (name.as_str(), tag) {
            (
                "bgcolor",
                Tag::Body
                | Tag::Table
                | Tag::Tr
                | Tag::Td
                | Tag::Th
                | Tag::Tbody
                | Tag::Thead
                | Tag::Tfoot
                | Tag::Marquee,
            ) => {
                if let Some(color) = legacy_color(value) {
                    hints.push(hint(Prop::BackgroundColor, color));
                }
            }
            ("background", Tag::Body | Tag::Table | Tag::Td | Tag::Th | Tag::Tr) => {
                hints.push(hint(Prop::BackgroundImage, Value::Url(value.into())));
            }
            ("text", Tag::Body) | ("color", Tag::Font) => {
                if let Some(color) = legacy_color(value) {
                    hints.push(hint(Prop::Color, color));
                }
            }
            ("color", Tag::Hr) => {
                if let Some(color) = legacy_color(value) {
                    hints.push(hint(Prop::BorderTopColor, color.clone()));
                    hints.push(hint(Prop::BorderRightColor, color.clone()));
                    hints.push(hint(Prop::BorderBottomColor, color.clone()));
                    hints.push(hint(Prop::BorderLeftColor, color));
                }
            }
            ("face", Tag::Font) => {
                hints.push(hint(Prop::FontFamily, Value::Family(value.into())));
            }
            ("size", Tag::Font) => {
                if let Some(size) = font_size_attribute(value) {
                    hints.push(hint(Prop::FontSize, size));
                }
            }
            (
                "align",
                Tag::P
                | Tag::Div
                | Tag::H1
                | Tag::H2
                | Tag::H3
                | Tag::H4
                | Tag::H5
                | Tag::H6
                | Tag::Td
                | Tag::Th
                | Tag::Tr
                | Tag::Tbody
                | Tag::Thead
                | Tag::Tfoot
                | Tag::Caption,
            ) => {
                if let Some(align) = text_align_attribute(value) {
                    hints.push(hint(Prop::TextAlign, align));
                }
            }
            ("align", Tag::Img | Tag::Object | Tag::Embed | Tag::Iframe | Tag::Applet) => {
                image_align(value, &mut hints);
            }
            ("align", Tag::Table | Tag::Hr) => box_align(value, &mut hints),
            ("valign", Tag::Td | Tag::Th | Tag::Tr | Tag::Tbody | Tag::Thead | Tag::Tfoot) => {
                if let Some(align) = vertical_align_attribute(value) {
                    hints.push(hint(Prop::VerticalAlign, align));
                }
            }
            (
                "width",
                Tag::Img
                | Tag::Table
                | Tag::Td
                | Tag::Th
                | Tag::Hr
                | Tag::Col
                | Tag::Colgroup
                | Tag::Object
                | Tag::Embed
                | Tag::Iframe
                | Tag::Applet
                | Tag::Pre,
            ) => {
                if let Some(size) = dimension(value) {
                    hints.push(hint(Prop::Width, size));
                }
            }
            (
                "height",
                Tag::Img
                | Tag::Table
                | Tag::Td
                | Tag::Th
                | Tag::Tr
                | Tag::Object
                | Tag::Embed
                | Tag::Iframe
                | Tag::Applet,
            ) => {
                if let Some(size) = dimension(value) {
                    hints.push(hint(Prop::Height, size));
                }
            }
            ("size", Tag::Hr) => {
                if let Some(size) = dimension(value) {
                    hints.push(hint(Prop::Height, size));
                }
            }
            ("noshade", Tag::Hr) => {
                border_all_styles(super::properties::BorderStyle::Solid, &mut hints)
            }
            ("border", Tag::Img | Tag::Object | Tag::Table) => {
                if let Some(size) = dimension(value) {
                    border_widths(size, &mut hints);
                    let line = if tag == Tag::Table {
                        super::properties::BorderStyle::Outset
                    } else {
                        super::properties::BorderStyle::Solid
                    };
                    border_all_styles(line, &mut hints);
                }
            }
            ("hspace", Tag::Img | Tag::Object | Tag::Embed | Tag::Applet) => {
                if let Some(size) = dimension(value) {
                    hints.push(hint(Prop::MarginLeft, size.clone()));
                    hints.push(hint(Prop::MarginRight, size));
                }
            }
            ("vspace", Tag::Img | Tag::Object | Tag::Embed | Tag::Applet) => {
                if let Some(size) = dimension(value) {
                    hints.push(hint(Prop::MarginTop, size.clone()));
                    hints.push(hint(Prop::MarginBottom, size));
                }
            }
            ("cellspacing", Tag::Table) => {
                if let Some(size) = dimension(value) {
                    hints.push(hint(Prop::BorderSpacing, size));
                }
            }
            ("clear", Tag::Br) => {
                use super::properties::Clear;
                let clear = match value.to_ascii_lowercase().as_str() {
                    "left" => Some(Clear::Left),
                    "right" => Some(Clear::Right),
                    "all" | "both" => Some(Clear::Both),
                    _ => None,
                };
                if let Some(clear) = clear {
                    hints.push(hint(Prop::Clear, Value::Clear(clear)));
                }
            }
            ("type", Tag::Ul | Tag::Ol | Tag::Li) => {
                if let Some(kind) = list_type_attribute(raw) {
                    hints.push(hint(Prop::ListStyleType, Value::ListStyleType(kind)));
                }
            }
            _ => {}
        }
    }
    if matches!(tag, Tag::Td | Tag::Th) {
        cell_hints_from_table(document, id, &mut hints);
    }
    hints
}

fn font_size_attribute(text: &str) -> Option<Value> {
    let (relative, digits) = match text.strip_prefix('+') {
        Some(rest) => (Some(1), rest),
        None => match text.strip_prefix('-') {
            Some(rest) => (Some(-1), rest),
            None => (None, text),
        },
    };
    let number: i32 = digits.trim().parse().ok()?;
    Some(legacy_font_size(match relative {
        Some(sign) => 3 + sign * number,
        None => number,
    }))
}

fn text_align_attribute(text: &str) -> Option<Value> {
    use super::properties::TextAlign as A;
    Some(Value::TextAlign(match text.to_ascii_lowercase().as_str() {
        "left" => A::Left,
        "right" => A::Right,
        "center" | "middle" => A::Center,
        "justify" => A::Justify,
        _ => return None,
    }))
}

fn vertical_align_attribute(text: &str) -> Option<Value> {
    use super::properties::VerticalAlign as A;
    Some(Value::VerticalAlign(
        match text.to_ascii_lowercase().as_str() {
            "top" => A::Top,
            "middle" | "center" => A::Middle,
            "bottom" => A::Bottom,
            "baseline" => A::Baseline,
            _ => return None,
        },
    ))
}

fn image_align(text: &str, hints: &mut Vec<Compiled>) {
    use super::properties::Float;
    match text.to_ascii_lowercase().as_str() {
        "left" => hints.push(hint(Prop::Float, Value::Float(Float::Left))),
        "right" => hints.push(hint(Prop::Float, Value::Float(Float::Right))),
        other => {
            if let Some(align) = vertical_align_attribute(other) {
                hints.push(hint(Prop::VerticalAlign, align));
            }
        }
    }
}

fn box_align(text: &str, hints: &mut Vec<Compiled>) {
    use super::properties::Float;
    match text.to_ascii_lowercase().as_str() {
        "center" | "middle" => {
            hints.push(hint(Prop::MarginLeft, Value::Empty));
            hints.push(hint(Prop::MarginRight, Value::Empty));
        }
        "left" => hints.push(hint(Prop::Float, Value::Float(Float::Left))),
        "right" => hints.push(hint(Prop::Float, Value::Float(Float::Right))),
        _ => {}
    }
}

fn border_widths(size: Value, hints: &mut Vec<Compiled>) {
    for prop in [
        Prop::BorderTopWidth,
        Prop::BorderRightWidth,
        Prop::BorderBottomWidth,
        Prop::BorderLeftWidth,
    ] {
        hints.push(hint(prop, size.clone()));
    }
}

fn border_all_styles(line: super::properties::BorderStyle, hints: &mut Vec<Compiled>) {
    for prop in [
        Prop::BorderTopStyle,
        Prop::BorderRightStyle,
        Prop::BorderBottomStyle,
        Prop::BorderLeftStyle,
    ] {
        hints.push(hint(prop, Value::BorderStyle(line)));
    }
}

fn list_type_attribute(text: &str) -> Option<super::properties::ListStyleType> {
    use super::properties::ListStyleType as L;
    Some(match text.trim() {
        "1" => L::Decimal,
        "a" => L::LowerAlpha,
        "A" => L::UpperAlpha,
        "i" => L::LowerRoman,
        "I" => L::UpperRoman,
        other => match other.to_ascii_lowercase().as_str() {
            "disc" => L::Disc,
            "circle" => L::Circle,
            "square" => L::Square,
            _ => return None,
        },
    })
}

/// A cell takes its padding and border from the `<table>` attributes around it.
fn cell_hints_from_table(document: &Document, id: NodeId, hints: &mut Vec<Compiled>) {
    let table = std::iter::successors(document.parent(id), |&node| document.parent(node))
        .take(4)
        .find(|&node| document.tag(node) == Tag::Table);
    let Some(table) = table else {
        return;
    };
    for attribute in document.attributes(table) {
        let name = attribute.name;
        let Some(value) = attribute.value else {
            continue;
        };
        if name.eq_ignore_ascii_case("cellpadding") {
            if let Some(size) = dimension(value) {
                for prop in [
                    Prop::PaddingTop,
                    Prop::PaddingRight,
                    Prop::PaddingBottom,
                    Prop::PaddingLeft,
                ] {
                    hints.push(hint(prop, size.clone()));
                }
            }
        } else if name.eq_ignore_ascii_case("border")
            && dimension(value).is_some_and(|size| size != pixels(0.0))
        {
            border_widths(pixels(1.0), hints);
            border_all_styles(super::properties::BorderStyle::Inset, hints);
        }
    }
}
