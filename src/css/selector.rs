//! Selectors: validated by the LALR(1) grammar, compiled from tokens, matched against a tree.

use std::sync::LazyLock;

use crate::html::{Document, NodeId, NodeKind, Tag};
use crate::lr::{ParseOutcome, Parser, RuntimeLimits};

use super::css_parser::PARSER_TABLES;
use super::tokenizer::{CssToken, CssTokenKind as K, unescape};

/// Longer selectors are invalid. Matching recurses once per compound, so this bounds it.
const MAX_COMPOUNDS: usize = 32;
/// Deeper `:not()` and `:is()` nesting is invalid, which bounds compiling and matching.
const MAX_NESTING: usize = 4;

static PARSER: LazyLock<Option<Parser<'static>>> =
    LazyLock::new(|| Parser::new(&PARSER_TABLES, RuntimeLimits::default()).ok());

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttributeOperator {
    Exists,
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeSelector {
    tag: Tag,
    name: Box<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PseudoClass {
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    Nth {
        a: i32,
        b: i32,
        from_end: bool,
        of_type: bool,
    },
    AnyLink,
    Checked,
    Disabled,
    Enabled,
    /// States that need a live page, such as `:hover`. They do not match for now.
    Dynamic,
    Not(Vec<Selector>),
    Is(Vec<Selector>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SimpleSelector {
    Universal,
    Type(TypeSelector),
    Id(Box<str>),
    Class(Box<str>),
    Attribute {
        name: Box<str>,
        operator: AttributeOperator,
        value: Box<str>,
        ignore_case: bool,
    },
    Pseudo(PseudoClass),
}

/// Simple selectors that must all match one element, and how the next compound relates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Compound {
    pub simple: Vec<SimpleSelector>,
    /// The relation to the compound after this one in the list, which stands to the left
    /// in the source. `None` for the leftmost.
    pub combinator: Option<Combinator>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selector {
    /// Compounds from right to left, as matching goes.
    pub compounds: Vec<Compound>,
    /// `before`, `after` and the like: the rule styles that part, not the element.
    pub pseudo_element: Option<Box<str>>,
    /// Counts of ids, classes and types, packed so that integer order is cascade order.
    pub specificity: u32,
}

impl Selector {
    /// Compiles a selector list. `None` means the list is invalid, which voids its rule.
    pub(super) fn parse_list(tokens: &[CssToken], source: &str) -> Option<Vec<Self>> {
        Self::parse_nested(tokens, source, 0)
    }

    fn parse_nested(tokens: &[CssToken], source: &str, depth: usize) -> Option<Vec<Self>> {
        if depth > MAX_NESTING {
            return None;
        }
        let tokens = significant_tokens(tokens);
        let terminals: Vec<u32> = tokens.iter().map(|token| token.terminal).collect();
        let parser = PARSER.as_ref()?;
        if !matches!(parser.parse(&terminals), Ok(ParseOutcome::Accepted)) {
            return None;
        }
        split_list(&tokens)
            .into_iter()
            .map(|part| compile_selector(part, source, depth))
            .collect()
    }
}

/// Removes comments and the white space that carries no meaning, leaving a white space
/// token only where it is a descendant combinator or inside a function's arguments.
fn significant_tokens(tokens: &[CssToken]) -> Vec<CssToken> {
    let mut result: Vec<CssToken> = Vec::with_capacity(tokens.len());
    let mut parens = 0_usize;
    let mut in_brackets = false;
    let is_combining = |kind: K| matches!(kind, K::Comma | K::Greater | K::Plus | K::Tilde);
    for &token in tokens {
        match token.kind {
            K::Comment => continue,
            K::Function | K::LeftParen => parens += 1,
            K::RightParen => parens = parens.saturating_sub(1),
            K::LeftBracket if parens == 0 => in_brackets = true,
            K::RightBracket if parens == 0 => in_brackets = false,
            _ => {}
        }
        if parens == 0 {
            if token.kind == K::Whitespace {
                let after_start = result
                    .last()
                    .is_none_or(|last| is_combining(last.kind) || last.kind == K::Whitespace);
                if after_start || in_brackets {
                    continue;
                }
            } else if (is_combining(token.kind) || token.kind == K::RightBracket)
                && result.last().is_some_and(|last| last.kind == K::Whitespace)
            {
                result.pop();
            }
        }
        result.push(token);
    }
    if result.last().is_some_and(|last| last.kind == K::Whitespace) {
        result.pop();
    }
    result
}

/// Splits a selector list at its top-level commas.
fn split_list(tokens: &[CssToken]) -> Vec<&[CssToken]> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut parens = 0_usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            K::Function | K::LeftParen => parens += 1,
            K::RightParen => parens = parens.saturating_sub(1),
            K::Comma if parens == 0 => {
                parts.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&tokens[start..]);
    parts
}

fn compile_selector(tokens: &[CssToken], source: &str, depth: usize) -> Option<Selector> {
    let mut compounds = Vec::new();
    let mut current = Vec::new();
    let mut pseudo_element = None;
    let mut specificity = 0;
    let mut pending = None;
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        let text = token.text(source);
        index += 1;
        let combinator = match token.kind {
            K::Whitespace => Some(Combinator::Descendant),
            K::Greater => Some(Combinator::Child),
            K::Plus => Some(Combinator::NextSibling),
            K::Tilde => Some(Combinator::SubsequentSibling),
            _ => None,
        };
        if let Some(combinator) = combinator {
            compounds.push(Compound {
                simple: std::mem::take(&mut current),
                combinator: pending.replace(combinator),
            });
            continue;
        }
        // Nothing may follow a pseudo-element except more of the same element's states.
        if pseudo_element.is_some() && token.kind != K::Colon {
            return None;
        }
        match token.kind {
            K::Star => current.push(SimpleSelector::Universal),
            K::Ident => {
                let name = unescape(text).to_ascii_lowercase();
                specificity += 1;
                current.push(SimpleSelector::Type(TypeSelector {
                    tag: Tag::from_name(&name),
                    name: name.into(),
                }));
            }
            K::Hash => {
                // A hash that does not form an identifier, such as `#1a`, is not an id.
                let digits = text[1..].strip_prefix('-').unwrap_or(&text[1..]);
                if digits.starts_with(|c: char| c.is_ascii_digit()) || text == "#-" {
                    return None;
                }
                let name = unescape(&text[1..]);
                specificity += 1 << 20;
                current.push(SimpleSelector::Id(name.into()));
            }
            K::Dot => {
                let name = unescape(tokens.get(index)?.text(source));
                index += 1;
                specificity += 1 << 10;
                current.push(SimpleSelector::Class(name.into()));
            }
            K::LeftBracket => {
                let end = index
                    + tokens[index..]
                        .iter()
                        .position(|t| t.kind == K::RightBracket)?;
                current.push(compile_attribute(&tokens[index..end], source)?);
                specificity += 1 << 10;
                index = end + 1;
            }
            K::Colon => {
                let double = tokens.get(index)?.kind == K::Colon;
                index += usize::from(double);
                let name_token = *tokens.get(index)?;
                index += 1;
                let name = unescape(name_token.text(source)).to_ascii_lowercase();
                if name_token.kind == K::Function {
                    let start = index;
                    let mut parens = 1_usize;
                    while parens > 0 {
                        match tokens.get(index)?.kind {
                            K::Function | K::LeftParen => parens += 1,
                            K::RightParen => parens -= 1,
                            _ => {}
                        }
                        index += 1;
                    }
                    let arguments = &tokens[start..index - 1];
                    let function = name.strip_suffix('(')?;
                    let (pseudo, weight) = compile_function(function, arguments, source, depth)?;
                    specificity += weight;
                    current.push(SimpleSelector::Pseudo(pseudo));
                } else if double || is_legacy_pseudo_element(&name) {
                    if !is_known_pseudo_element(&name) {
                        return None;
                    }
                    specificity += 1;
                    pseudo_element = Some(name.into());
                } else {
                    specificity += 1 << 10;
                    current.push(SimpleSelector::Pseudo(compile_pseudo_class(&name)?));
                }
            }
            _ => return None,
        }
    }
    compounds.push(Compound {
        simple: current,
        combinator: pending,
    });
    if compounds.len() > MAX_COMPOUNDS {
        return None;
    }
    compounds.reverse();
    Some(Selector {
        compounds,
        pseudo_element,
        specificity,
    })
}

fn is_legacy_pseudo_element(name: &str) -> bool {
    matches!(name, "before" | "after" | "first-line" | "first-letter")
}

fn is_known_pseudo_element(name: &str) -> bool {
    is_legacy_pseudo_element(name)
        || matches!(name, "marker" | "placeholder" | "selection" | "backdrop")
        || name.starts_with("-webkit-")
        || name.starts_with("-moz-")
}

fn compile_attribute(tokens: &[CssToken], source: &str) -> Option<SimpleSelector> {
    let name = unescape(tokens.first()?.text(source)).to_ascii_lowercase();
    let Some(operator_token) = tokens.get(1) else {
        return Some(SimpleSelector::Attribute {
            name: name.into(),
            operator: AttributeOperator::Exists,
            value: "".into(),
            ignore_case: false,
        });
    };
    let operator = match operator_token.kind {
        K::Equals => AttributeOperator::Equals,
        K::Includes => AttributeOperator::Includes,
        K::DashMatch => AttributeOperator::DashMatch,
        K::PrefixMatch => AttributeOperator::Prefix,
        K::SuffixMatch => AttributeOperator::Suffix,
        K::SubstringMatch => AttributeOperator::Substring,
        _ => return None,
    };
    let value_token = tokens.get(2)?;
    let text = value_token.text(source);
    let value = if value_token.kind == K::String {
        let quote = text.chars().next()?;
        let body = &text[1..];
        unescape(body.strip_suffix(quote).unwrap_or(body))
    } else {
        unescape(text)
    };
    let ignore_case = match tokens.get(3) {
        None => false,
        Some(flag) => match flag.text(source) {
            "i" | "I" => true,
            "s" | "S" => false,
            _ => return None,
        },
    };
    Some(SimpleSelector::Attribute {
        name: name.into(),
        operator,
        value: value.into(),
        ignore_case,
    })
}

fn compile_pseudo_class(name: &str) -> Option<PseudoClass> {
    Some(match name {
        "root" => PseudoClass::Root,
        "empty" => PseudoClass::Empty,
        "first-child" => PseudoClass::FirstChild,
        "last-child" => PseudoClass::LastChild,
        "only-child" => PseudoClass::OnlyChild,
        "first-of-type" => PseudoClass::FirstOfType,
        "last-of-type" => PseudoClass::LastOfType,
        "only-of-type" => PseudoClass::OnlyOfType,
        "link" | "any-link" => PseudoClass::AnyLink,
        "checked" => PseudoClass::Checked,
        "disabled" => PseudoClass::Disabled,
        "enabled" => PseudoClass::Enabled,
        "hover" | "active" | "focus" | "focus-visible" | "focus-within" | "visited" | "target"
        | "indeterminate" | "default" | "valid" | "invalid" | "required" | "optional"
        | "read-only" | "read-write" | "placeholder-shown" | "autofill" => PseudoClass::Dynamic,
        _ => return None,
    })
}

/// Returns the pseudo-class and the specificity it adds.
fn compile_function(
    name: &str,
    arguments: &[CssToken],
    source: &str,
    depth: usize,
) -> Option<(PseudoClass, u32)> {
    let class_weight = 1 << 10;
    let highest = |list: &[Selector]| list.iter().map(|s| s.specificity).max().unwrap_or(0);
    match name {
        "not" | "is" | "matches" | "where" => {
            let list = Selector::parse_nested(arguments, source, depth + 1)?;
            let weight = if name == "where" { 0 } else { highest(&list) };
            let pseudo = if name == "not" {
                PseudoClass::Not(list)
            } else {
                PseudoClass::Is(list)
            };
            Some((pseudo, weight))
        }
        "nth-child" | "nth-last-child" | "nth-of-type" | "nth-last-of-type" => {
            let text: String = arguments
                .iter()
                .filter(|token| !matches!(token.kind, K::Whitespace | K::Comment))
                .map(|token| token.text(source))
                .collect();
            let (a, b) = parse_nth(&text)?;
            Some((
                PseudoClass::Nth {
                    a,
                    b,
                    from_end: name.contains("last"),
                    of_type: name.ends_with("of-type"),
                },
                class_weight,
            ))
        }
        _ => None,
    }
}

/// Reads the `an+b` form, with its white space already removed.
fn parse_nth(text: &str) -> Option<(i32, i32)> {
    let text = text.to_ascii_lowercase();
    match text.as_str() {
        "even" => return Some((2, 0)),
        "odd" => return Some((2, 1)),
        _ => {}
    }
    let Some((a, b)) = text.split_once('n') else {
        return Some((0, text.parse().ok()?));
    };
    let a = match a {
        "" | "+" => 1,
        "-" => -1,
        _ => a.parse().ok()?,
    };
    let b = if b.is_empty() {
        0
    } else {
        b.strip_prefix('+').unwrap_or(b).parse().ok()?
    };
    Some((a, b))
}

/// Why a compound failed, which tells how far back trying again can still help. Without
/// this, descendant combinators make matching exponential in the length of the selector.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Failure {
    /// Another candidate for the nearest sibling combinator may work.
    TrySibling,
    /// Another ancestor for the nearest descendant combinator may work.
    TryAncestor,
    /// No other choice can work.
    Final,
}

/// Whether `element` is what the selector's rightmost compound styles.
#[must_use]
pub fn matches(document: &Document, element: NodeId, selector: &Selector) -> bool {
    match_from(document, element, &selector.compounds).is_ok()
}

fn match_from(document: &Document, element: NodeId, compounds: &[Compound]) -> Result<(), Failure> {
    let Some((compound, rest)) = compounds.split_first() else {
        return Ok(());
    };
    if !compound
        .simple
        .iter()
        .all(|simple| match_simple(document, element, simple))
    {
        return Err(Failure::TrySibling);
    }
    let Some(combinator) = compound.combinator else {
        return Ok(());
    };
    match combinator {
        Combinator::Child | Combinator::Descendant => {
            let mut candidate = element_parent(document, element);
            while let Some(parent) = candidate {
                match match_from(document, parent, rest) {
                    Ok(()) => return Ok(()),
                    Err(Failure::Final) => return Err(Failure::Final),
                    Err(failure) => {
                        if combinator == Combinator::Child {
                            return Err(if failure == Failure::TrySibling {
                                Failure::TryAncestor
                            } else {
                                failure
                            });
                        }
                    }
                }
                candidate = element_parent(document, parent);
            }
            Err(Failure::Final)
        }
        Combinator::NextSibling | Combinator::SubsequentSibling => {
            let mut candidate = previous_element(document, element);
            while let Some(sibling) = candidate {
                match match_from(document, sibling, rest) {
                    Ok(()) => return Ok(()),
                    Err(Failure::TrySibling) if combinator == Combinator::SubsequentSibling => {}
                    Err(Failure::TrySibling) => return Err(Failure::TryAncestor),
                    Err(failure) => return Err(failure),
                }
                candidate = previous_element(document, sibling);
            }
            Err(Failure::TryAncestor)
        }
    }
}

fn element_parent(document: &Document, element: NodeId) -> Option<NodeId> {
    document
        .parent(element)
        .filter(|&parent| document.kind(parent) == NodeKind::Element)
}

fn previous_element(document: &Document, element: NodeId) -> Option<NodeId> {
    std::iter::successors(document.previous_sibling(element), |&sibling| {
        document.previous_sibling(sibling)
    })
    .find(|&sibling| document.kind(sibling) == NodeKind::Element)
}

fn next_element(document: &Document, element: NodeId) -> Option<NodeId> {
    std::iter::successors(document.next_sibling(element), |&sibling| {
        document.next_sibling(sibling)
    })
    .find(|&sibling| document.kind(sibling) == NodeKind::Element)
}

fn attribute(document: &Document, element: NodeId, name: &str) -> Option<String> {
    document
        .attributes(element)
        .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
        .map(|attribute| attribute.decoded_value().into_owned())
}

fn has_attribute(document: &Document, element: NodeId, name: &str) -> bool {
    document
        .attributes(element)
        .any(|attribute| attribute.name.eq_ignore_ascii_case(name))
}

fn same_type(document: &Document, element: NodeId, other: NodeId) -> bool {
    document.tag(element) == document.tag(other)
        && document
            .name(element)
            .eq_ignore_ascii_case(document.name(other))
}

fn match_simple(document: &Document, element: NodeId, simple: &SimpleSelector) -> bool {
    match simple {
        SimpleSelector::Universal => true,
        SimpleSelector::Type(selector) => {
            let tag = document.tag(element);
            if selector.tag == Tag::Other || tag == Tag::Other {
                document.name(element).eq_ignore_ascii_case(&selector.name)
            } else {
                tag == selector.tag
            }
        }
        SimpleSelector::Id(id) => attribute(document, element, "id").is_some_and(|value| {
            if document.is_quirks() {
                value.eq_ignore_ascii_case(id)
            } else {
                value == **id
            }
        }),
        SimpleSelector::Class(class) => {
            attribute(document, element, "class").is_some_and(|value| {
                value.split_ascii_whitespace().any(|word| {
                    if document.is_quirks() {
                        word.eq_ignore_ascii_case(class)
                    } else {
                        word == &**class
                    }
                })
            })
        }
        SimpleSelector::Attribute {
            name,
            operator,
            value,
            ignore_case,
        } => attribute(document, element, name)
            .is_some_and(|actual| match_attribute(&actual, *operator, value, *ignore_case)),
        SimpleSelector::Pseudo(pseudo) => match_pseudo(document, element, pseudo),
    }
}

fn match_attribute(
    actual: &str,
    operator: AttributeOperator,
    wanted: &str,
    ignore_case: bool,
) -> bool {
    let lowered = |text: &str| {
        if ignore_case {
            text.to_ascii_lowercase()
        } else {
            text.to_owned()
        }
    };
    let (actual, wanted) = (lowered(actual), lowered(wanted));
    match operator {
        AttributeOperator::Exists => true,
        AttributeOperator::Equals => actual == wanted,
        AttributeOperator::Includes => {
            !wanted.is_empty() && actual.split_ascii_whitespace().any(|word| word == wanted)
        }
        AttributeOperator::DashMatch => {
            actual == wanted
                || actual
                    .strip_prefix(&wanted)
                    .is_some_and(|rest| rest.starts_with('-'))
        }
        AttributeOperator::Prefix => !wanted.is_empty() && actual.starts_with(&wanted),
        AttributeOperator::Suffix => !wanted.is_empty() && actual.ends_with(&wanted),
        AttributeOperator::Substring => !wanted.is_empty() && actual.contains(&wanted),
    }
}

fn match_pseudo(document: &Document, element: NodeId, pseudo: &PseudoClass) -> bool {
    let no_previous = || previous_element(document, element).is_none();
    let no_next = || next_element(document, element).is_none();
    let no_same_type = |walk: fn(&Document, NodeId) -> Option<NodeId>| {
        std::iter::successors(walk(document, element), |&node| walk(document, node))
            .all(|node| !same_type(document, element, node))
    };
    match pseudo {
        PseudoClass::Root => document
            .parent(element)
            .is_some_and(|parent| document.kind(parent) == NodeKind::Document),
        PseudoClass::Empty => document
            .children(element)
            .all(|child| match document.kind(child) {
                NodeKind::Element => false,
                NodeKind::Text => document.text(child).is_empty(),
                _ => true,
            }),
        PseudoClass::FirstChild => no_previous(),
        PseudoClass::LastChild => no_next(),
        PseudoClass::OnlyChild => no_previous() && no_next(),
        PseudoClass::FirstOfType => no_same_type(previous_element),
        PseudoClass::LastOfType => no_same_type(next_element),
        PseudoClass::OnlyOfType => no_same_type(previous_element) && no_same_type(next_element),
        PseudoClass::Nth {
            a,
            b,
            from_end,
            of_type,
        } => {
            let walk = if *from_end {
                next_element
            } else {
                previous_element
            };
            let before =
                std::iter::successors(walk(document, element), |&node| walk(document, node))
                    .filter(|&node| !*of_type || same_type(document, element, node))
                    .count();
            let position = i64::try_from(before).unwrap_or(i64::MAX - 1) + 1;
            let (a, b) = (i64::from(*a), i64::from(*b));
            if a == 0 {
                position == b
            } else {
                (position - b) % a == 0 && (position - b) / a >= 0
            }
        }
        PseudoClass::AnyLink => {
            matches!(document.tag(element), Tag::A | Tag::Area)
                && has_attribute(document, element, "href")
        }
        PseudoClass::Checked => {
            has_attribute(document, element, "checked")
                || has_attribute(document, element, "selected")
        }
        PseudoClass::Disabled => {
            is_form_control(document.tag(element)) && has_attribute(document, element, "disabled")
        }
        PseudoClass::Enabled => {
            is_form_control(document.tag(element)) && !has_attribute(document, element, "disabled")
        }
        PseudoClass::Dynamic => false,
        PseudoClass::Not(list) => !list
            .iter()
            .any(|selector| matches(document, element, selector)),
        PseudoClass::Is(list) => list
            .iter()
            .any(|selector| matches(document, element, selector)),
    }
}

fn is_form_control(tag: Tag) -> bool {
    matches!(
        tag,
        Tag::Button | Tag::Input | Tag::Select | Tag::Textarea | Tag::Option | Tag::Optgroup
    )
}
