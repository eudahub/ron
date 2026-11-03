//! The cascade: finds the rules that match each element and computes its style. Elements
//! with the same parent style and the same matched rules share one computed style.

use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hasher};
use std::sync::LazyLock;

use crate::html::{Document, NodeId, NodeKind, Tag};

use super::media::{Viewport, matches_media};
use super::properties::{Compiled, PROPERTY_COUNT, Prop, Value, compile, legacy_font_size};
use super::selector::{
    Combinator, ElementNames, MatchContext, PseudoClass, Selector, SimpleSelector, TypeSelector,
};
use super::style::{ComputedStyle, blockified, compute};
use super::stylesheet::{Declaration, StyleRule, Stylesheet};
use super::values::{Color, Length, Unit, parse_length};
use super::variables::{CustomProperties, merged, substitute, uses_variables};

/// The browser's own defaults, following the rendering section of the HTML standard.
const DEFAULT_STYLES: &str = include_str!("default.css");

/// Added to the defaults for documents in quirks mode.
const QUIRKS_STYLES: &str = include_str!("quirks.css");

/// Distinct styles beyond this reuse the parent's, so that hostile pages stay small.
const MAX_STYLES: usize = 1 << 20;
/// What all sets of custom properties of a page may take; beyond it, elements only inherit.
const MAX_CUSTOM_PROPERTY_BYTES: usize = 64 * 1024 * 1024;

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
    /// Those that only ask for a link, which few elements can be.
    links: Vec<IndexEntry<'a>>,
    others: Vec<IndexEntry<'a>>,
    uses_positions: bool,
}

/// What tests of a tag in its surroundings came to.
struct KnownTag {
    /// A number for this tag with everything around it, for the elements inside it to go by.
    context: u32,
    matched: Vec<(u32, u32)>,
    /// With no style attribute and no hints.
    plain: bool,
}

/// Whether nothing in this document can match, given all words it uses as an `id` or a `class`.
/// This is true of most rules of large style sheets, and testing them would take most of the time.
fn cannot_match(words: &HashSet<String>, selector: &Selector, quirks: bool) -> bool {
    selector.compounds.iter().any(|compound| {
        compound.simple.iter().any(|simple| match simple {
            SimpleSelector::Pseudo(PseudoClass::Dynamic) => true,
            SimpleSelector::Id(name) | SimpleSelector::Class(name) => {
                if quirks {
                    !words.contains(&name.to_ascii_lowercase())
                } else {
                    !words.contains(&**name)
                }
            }
            _ => false,
        })
    })
}

/// Whether a selector looks at more than an element's tag and those of its ancestors: at siblings,
/// at the place among them, or at what is inside.
fn looks_at_neighbours(selector: &Selector) -> bool {
    selector.compounds.iter().any(|compound| {
        matches!(
            compound.combinator,
            Some(Combinator::NextSibling | Combinator::SubsequentSibling)
        ) || compound.simple.iter().any(|simple| match simple {
            SimpleSelector::Pseudo(pseudo) => match pseudo {
                PseudoClass::Not(list) | PseudoClass::Is(list) => {
                    list.iter().any(looks_at_neighbours)
                }
                PseudoClass::Root
                | PseudoClass::AnyLink
                | PseudoClass::Checked
                | PseudoClass::Disabled
                | PseudoClass::Enabled
                | PseudoClass::Dynamic => false,
                _ => true,
            },
            _ => false,
        })
    })
}

fn add_selector<'a>(
    index: &mut RuleIndex<'a>,
    neighbour_index: &mut RuleIndex<'a>,
    words: &HashSet<String>,
    selector: &'a Selector,
    rule: u32,
    quirks: bool,
) {
    // Styles of parts such as `::before` belong to boxes that layout does not make yet.
    if selector.pseudo_element.is_some() || cannot_match(words, selector, quirks) {
        return;
    }
    if looks_at_neighbours(selector) {
        neighbour_index.add(selector, rule, quirks);
    } else {
        index.add(selector, rule, quirks);
    }
}

impl<'a> RuleIndex<'a> {
    fn add(&mut self, selector: &'a Selector, rule: u32, quirks: bool) {
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
        let asks_for_link = simple
            .iter()
            .any(|s| matches!(s, SimpleSelector::Pseudo(PseudoClass::AnyLink)));
        match tag {
            Some(tag) => self.tags.entry(tag).or_default().push(entry),
            None if asks_for_link => self.links.push(entry),
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
    /// Initial styles for every node, which takes almost no time, for when the real ones can only
    /// be computed later.
    #[must_use]
    pub fn unstyled(document: &Document) -> Self {
        Self {
            styles: vec![ComputedStyle::initial()],
            of_node: vec![0; document.node_count()],
            limited: false,
        }
    }

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

/// Where a style sheet of a document comes from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SheetSource {
    /// The content of a `<style>` element.
    Inline { text: String, media: Option<String> },
    /// A `<link rel=stylesheet>`, which the caller may fetch.
    Link { href: String, media: Option<String> },
}

/// The style sheets of a document in order, whatever the size of the view. Those of a type other
/// than CSS and alternative ones are left out.
#[must_use]
pub fn document_sheet_sources(document: &Document) -> Vec<SheetSource> {
    let mut sources = Vec::new();
    for id in document.descendants(document.root()) {
        if document.kind(id) != NodeKind::Element {
            continue;
        }
        let tag = document.tag(id);
        if tag != Tag::Style && tag != Tag::Link {
            continue;
        }
        let attribute = |name: &str| {
            document
                .attributes(id)
                .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
                .map(|attribute| attribute.decoded_value().into_owned())
        };
        if attribute("type").is_some_and(|kind| {
            let kind = kind.trim();
            !kind.is_empty() && !kind.eq_ignore_ascii_case("text/css")
        }) {
            continue;
        }
        let media = attribute("media");
        if tag == Tag::Style {
            if let Some(text) = document.first_child(id) {
                sources.push(SheetSource::Inline {
                    text: document.decoded_text(text).into_owned(),
                    media,
                });
            }
        } else if attribute("rel").is_some_and(|rel| {
            let mut words = rel.split_ascii_whitespace();
            let is_sheet = words
                .clone()
                .any(|word| word.eq_ignore_ascii_case("stylesheet"));
            is_sheet && !words.any(|word| word.eq_ignore_ascii_case("alternate"))
        }) && let Some(href) = attribute("href")
        {
            sources.push(SheetSource::Link { href, media });
        }
    }
    sources
}

/// Computes the style of every element from the default styles and these author sheets.
/// Walks the tree in document order, without recursion.
#[must_use]
pub fn compute_styles<Sheet: Borrow<Stylesheet>>(
    document: &Document,
    authors: &[Sheet],
    viewport: Viewport,
) -> Styles {
    let quirks = document.is_quirks();
    let mut rules: Vec<(u8, &[Compiled])> = Vec::new();
    let mut compiled_authors: Vec<Vec<Vec<Compiled>>> = Vec::with_capacity(authors.len());
    for sheet in authors {
        let sheet = sheet.borrow();
        compiled_authors.push(
            sheet
                .rules
                .iter()
                .map(|rule| compile_rule(sheet, rule))
                .collect(),
        );
    }
    let mut variable_rules: HashMap<u32, VariableRule<'_>> = HashMap::new();
    let mut custom_budget = MAX_CUSTOM_PROPERTY_BYTES;
    let names = ElementNames::of_document(document);
    let words = names.words(quirks);
    let mut index = RuleIndex::default();
    let mut neighbour_index = RuleIndex::default();
    let mut add_sheet = |sheet: &'static Stylesheet, compiled: &'static [Vec<Compiled>]| {
        for (rule, declarations) in sheet.rules.iter().zip(compiled) {
            let number = u32::try_from(rules.len()).unwrap_or(u32::MAX);
            rules.push((ORIGIN_DEFAULT, declarations));
            for selector in &rule.selectors {
                add_selector(
                    &mut index,
                    &mut neighbour_index,
                    &words,
                    selector,
                    number,
                    quirks,
                );
            }
        }
    };
    add_sheet(&DEFAULT_SHEET, &DEFAULT_RULES);
    if quirks {
        add_sheet(&QUIRKS_SHEET, &QUIRKS_RULES);
    }
    for (sheet, compiled) in authors.iter().zip(&compiled_authors) {
        let sheet = sheet.borrow();
        for (rule, declarations) in sheet.rules.iter().zip(compiled) {
            if rule
                .media
                .is_some_and(|media| !sheet.media_applies(media, viewport))
            {
                continue;
            }
            let number = u32::try_from(rules.len()).unwrap_or(u32::MAX);
            rules.push((ORIGIN_AUTHOR, declarations));
            if let Some(variable_rule) = variable_rule(sheet, rule) {
                variable_rules.insert(number, variable_rule);
            }
            for selector in &rule.selectors {
                add_selector(
                    &mut index,
                    &mut neighbour_index,
                    &words,
                    selector,
                    number,
                    quirks,
                );
            }
        }
    }

    let uses_positions = index.uses_positions || neighbour_index.uses_positions;
    let mut context = MatchContext::with_names(document, uses_positions, names);
    // For each element, what its tag and all those around it gave. Elements that are alike in that
    // match the same rules, apart from those that look at neighbours, so the rest are tested once.
    // Pages often repeat a few tags many thousands of times.
    let mut contexts: Vec<u32> = vec![0; document.node_count()];
    let mut known_tags: HashMap<(u32, u8, &str), KnownTag> = HashMap::new();
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
        let source = document.text(id);
        let next_context = u32::try_from(known_tags.len() + 1).unwrap_or(u32::MAX);
        let known = known_tags
            .entry((contexts[parent.index()], document.tag(id) as u8, source))
            .or_insert_with(|| {
                let mut matched = Vec::new();
                collect_matches(document, id, &index, &mut context, &mut matched);
                let has_style_attribute = document.attributes(id).any(|attribute| {
                    attribute.name.eq_ignore_ascii_case("style") && attribute.value.is_some()
                });
                KnownTag {
                    context: next_context,
                    matched,
                    plain: !has_style_attribute && presentational_hints(document, id).is_empty(),
                }
            });
        contexts[id.index()] = known.context;
        let plain = known.plain;
        matched.clear();
        matched.extend_from_slice(&known.matched);
        collect_matches(document, id, &neighbour_index, &mut context, &mut matched);
        // Sort by specificity, then by source order, so that later entries win.
        matched.sort_unstable();
        matched.dedup();
        // What the source of the tag holds decides the hints and the style attribute.
        let tag_text = if plain { "" } else { source };
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
        let hints = presentational_hints(document, id);
        let style_attribute = document
            .attributes(id)
            .find(|attribute| attribute.name.eq_ignore_ascii_case("style"))
            .filter(|attribute| attribute.value.is_some())
            .map(|attribute| attribute.decoded_value());
        let inline_sheet = style_attribute.map(|text| Stylesheet::parse_declarations(&text));
        let inline_declarations: Vec<RawDeclaration<'_>> = inline_sheet
            .as_ref()
            .map(|sheet| raw_declarations(sheet, sheet.declarations()))
            .unwrap_or_default();
        // Custom properties come first, as all else can refer to them.
        let parent_customs = result.styles[parent_style as usize]
            .custom_properties
            .clone();
        let mut declared_customs: Vec<(&str, &str)> = Vec::new();
        for important in [false, true] {
            for &(_, rule) in &matched {
                if let Some(variable_rule) = variable_rules.get(&rule) {
                    add_customs(
                        &mut declared_customs,
                        &variable_rule.declarations,
                        important,
                    );
                }
            }
            add_customs(&mut declared_customs, &inline_declarations, important);
        }
        let customs = if declared_customs.is_empty() {
            parent_customs
        } else {
            merged(
                parent_customs.as_ref(),
                &declared_customs,
                &mut custom_budget,
            )
        };
        let mut with_variables: HashMap<u32, Vec<Compiled>> = HashMap::new();
        for &(_, rule) in &matched {
            if let Some(variable_rule) = variable_rules.get(&rule)
                && variable_rule.uses_variables
            {
                with_variables.insert(
                    rule,
                    compile_raw(&variable_rule.declarations, customs.as_ref()),
                );
            }
        }
        let inline = compile_raw(&inline_declarations, customs.as_ref());
        let mut cascaded: [Option<&Value>; PROPERTY_COUNT] = [None; PROPERTY_COUNT];
        let apply_rules = |cascaded: &mut _, origin, important| {
            apply_origin(
                cascaded,
                &matched,
                &rules,
                &with_variables,
                origin,
                important,
            );
        };
        apply_rules(&mut cascaded, ORIGIN_DEFAULT, false);
        apply_list(&mut cascaded, &hints, false);
        apply_rules(&mut cascaded, ORIGIN_AUTHOR, false);
        apply_list(&mut cascaded, &inline, false);
        apply_rules(&mut cascaded, ORIGIN_AUTHOR, true);
        apply_list(&mut cascaded, &inline, true);
        apply_rules(&mut cascaded, ORIGIN_DEFAULT, true);
        let mut style = compute(
            &result.styles[parent_style as usize],
            &cascaded,
            root_font_size,
            viewport,
        );
        style.custom_properties = customs;
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
    with_variables: &'a HashMap<u32, Vec<Compiled>>,
    origin: u8,
    important: bool,
) {
    for &(_, rule) in matched {
        let (rule_origin, declarations) = rules[rule as usize];
        if rule_origin == origin {
            let declarations = with_variables
                .get(&rule)
                .map_or(declarations, |list| list.as_slice());
            apply_list(cascaded, declarations, important);
        }
    }
}

/// A declaration as written, for when it can only be read for one element at a time.
#[derive(Clone, Copy)]
struct RawDeclaration<'a> {
    name: &'a str,
    value: &'a str,
    important: bool,
}

/// A rule with custom properties or `var()` in it.
struct VariableRule<'a> {
    /// All of its declarations in order, as the last of a property wins.
    declarations: Vec<RawDeclaration<'a>>,
    uses_variables: bool,
}

fn raw_declarations<'a>(
    sheet: &'a Stylesheet,
    declarations: &'a [Declaration],
) -> Vec<RawDeclaration<'a>> {
    declarations
        .iter()
        .map(|declaration| RawDeclaration {
            name: &declaration.name,
            value: sheet.value(declaration),
            important: declaration.important,
        })
        .collect()
}

fn variable_rule<'a>(sheet: &'a Stylesheet, rule: &'a StyleRule) -> Option<VariableRule<'a>> {
    let declarations = raw_declarations(sheet, &rule.declarations);
    let is_custom = |declaration: &RawDeclaration<'_>| declaration.name.starts_with("--");
    let uses = declarations
        .iter()
        .any(|declaration| !is_custom(declaration) && uses_variables(declaration.value));
    (uses || declarations.iter().any(is_custom)).then_some(VariableRule {
        declarations,
        uses_variables: uses,
    })
}

fn add_customs<'a>(
    out: &mut Vec<(&'a str, &'a str)>,
    declarations: &[RawDeclaration<'a>],
    important: bool,
) {
    for declaration in declarations {
        if declaration.important == important && declaration.name.starts_with("--") {
            out.push((declaration.name, declaration.value));
        }
    }
}

/// Reads declarations for one element. One whose `var()` has no value counts as not there.
fn compile_raw(
    declarations: &[RawDeclaration<'_>],
    customs: Option<&CustomProperties>,
) -> Vec<Compiled> {
    let mut compiled = Vec::new();
    for declaration in declarations {
        if declaration.name.starts_with("--") {
            continue;
        }
        if !uses_variables(declaration.value) {
            compile(
                declaration.name,
                declaration.value,
                declaration.important,
                &mut compiled,
            );
        } else if let Some(value) = substitute(declaration.value, customs) {
            compile(
                declaration.name,
                &value,
                declaration.important,
                &mut compiled,
            );
        }
    }
    compiled
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
    // The values stay with the context, so these copies take its place only for a moment.
    let (element_id, element_class) = context.id_and_class(id);
    let element_id = element_id.map(str::to_owned);
    let element_class = element_class.map(str::to_owned);
    if let Some(value) = element_id {
        if quirks {
            test(index.ids.get(&value.to_ascii_lowercase()), context);
        } else {
            test(index.ids.get(value.as_str()), context);
        }
    }
    if let Some(value) = element_class {
        for class in value.split_ascii_whitespace() {
            if quirks {
                test(index.classes.get(&class.to_ascii_lowercase()), context);
            } else {
                test(index.classes.get(class), context);
            }
        }
    }
    test(index.tags.get(document.local_name(id).as_ref()), context);
    let lower = document.name(id).to_ascii_lowercase();
    if lower != document.local_name(id) {
        test(index.tags.get(&lower), context);
    }
    if matches!(document.tag(id), Tag::A | Tag::Area) {
        test(Some(&index.links), context);
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
