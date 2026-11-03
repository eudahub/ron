//! Custom properties and `var()`. They inherit down the tree, so a value can only be known for one
//! element at a time, unlike all else in a style sheet. They are also the one way for text to grow
//! when it is read: `--b: var(--a) var(--a); --c: var(--b) var(--b)` doubles at each step, so what
//! they may grow to has limits.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

/// Longer values count as invalid.
const MAX_VALUE_BYTES: usize = 64 * 1024;
/// How deep values may refer to others, and fallbacks hold fallbacks.
const MAX_DEPTH: usize = 32;
/// All that one element's declarations may come to.
const MAX_ELEMENT_BYTES: usize = 1024 * 1024;

type Values = BTreeMap<Box<str>, Box<str>>;

/// The custom properties in force on an element, shared with those that inherit them.
#[derive(Clone)]
pub struct CustomProperties(Arc<Values>);

impl PartialEq for CustomProperties {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl fmt::Debug for CustomProperties {
    // Styles are told apart by what this writes. Many share a few sets of values that can be large,
    // so they go by which set it is.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CustomProperties({:p})", Arc::as_ptr(&self.0))
    }
}

impl CustomProperties {
    /// The value of a custom property, with all `var()` in it replaced.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(AsRef::as_ref)
    }
}

/// Whether a value needs `var()` replaced before it can be read.
pub(super) fn uses_variables(text: &str) -> bool {
    text.as_bytes()
        .windows(4)
        .any(|window| window.eq_ignore_ascii_case(b"var("))
}

/// What `next_var` finds.
struct VarCall<'a> {
    start: usize,
    end: usize,
    name: &'a str,
    fallback: Option<&'a str>,
}

/// Finds the next `var()` from a position, outside of strings and comments.
fn next_var(text: &str, from: usize) -> Option<VarCall<'_>> {
    let bytes = text.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => index = after_string(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = after_comment(bytes, index),
            b'\\' => index += 2,
            _ => {
                let is_start = bytes[index..]
                    .get(..4)
                    .is_some_and(|word| word.eq_ignore_ascii_case(b"var("))
                    && (index == 0 || !is_name_byte(bytes[index - 1]));
                if is_start {
                    return Some(read_var(text, index));
                }
                index += 1;
            }
        }
    }
    None
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte >= 0x80
}

fn after_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            byte if byte == quote => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn after_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")
        .map_or(bytes.len(), |offset| start + 2 + offset + 2)
}

/// Reads a `var()` that starts here. What is not closed ends with the text, as in all of CSS.
fn read_var(text: &str, start: usize) -> VarCall<'_> {
    let bytes = text.as_bytes();
    let inner_start = start + 4;
    let mut depth = 0_usize;
    let mut comma = None;
    let mut index = inner_start;
    let mut inner_end = bytes.len();
    let mut end = bytes.len();
    while index < bytes.len() {
        match bytes[index] {
            b'"' | b'\'' => {
                index = after_string(bytes, index);
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = after_comment(bytes, index);
                continue;
            }
            b'\\' => {
                index += 2;
                continue;
            }
            b'(' => depth += 1,
            b')' if depth == 0 => {
                inner_end = index;
                end = index + 1;
                break;
            }
            b')' => depth -= 1,
            b',' if depth == 0 && comma.is_none() => comma = Some(index),
            _ => {}
        }
        index += 1;
    }
    let inner_end = inner_end.min(bytes.len());
    let (name, fallback) = match comma {
        Some(comma) => (&text[inner_start..comma], Some(&text[comma + 1..inner_end])),
        None => (&text[inner_start..inner_end], None),
    };
    VarCall {
        start,
        end,
        name: name.trim(),
        fallback,
    }
}

/// Replaces each `var()`. `None` when one has no value and no fallback, or it all grows too large.
fn expand(
    text: &str,
    lookup: &mut dyn FnMut(&str, usize) -> Option<String>,
    depth: usize,
) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    while let Some(call) = next_var(text, copied) {
        out.push_str(&text[copied..call.start]);
        if !call.name.starts_with("--") {
            return None;
        }
        let value = match lookup(call.name, depth) {
            Some(value) => value,
            None => expand(call.fallback?.trim(), lookup, depth + 1)?,
        };
        out.push_str(&value);
        if out.len() > MAX_VALUE_BYTES {
            return None;
        }
        copied = call.end.min(text.len());
    }
    out.push_str(&text[copied..]);
    (out.len() <= MAX_VALUE_BYTES).then_some(out)
}

/// Replaces each `var()` in a value, with what is in force on an element.
pub(super) fn substitute(text: &str, customs: Option<&CustomProperties>) -> Option<String> {
    expand(
        text,
        &mut |name, _| {
            customs
                .and_then(|customs| customs.get(name))
                .map(str::to_owned)
        },
        0,
    )
}

/// Works out the new values of one element, which can refer to each other.
struct Resolver<'a> {
    inherited: Option<&'a Values>,
    declared: BTreeMap<&'a str, &'a str>,
    done: BTreeMap<&'a str, Option<String>>,
    visiting: HashSet<&'a str>,
    spent: usize,
}

impl<'a> Resolver<'a> {
    fn value_of(&mut self, name: &str, depth: usize) -> Option<String> {
        let Some((&name, &raw)) = self.declared.get_key_value(name) else {
            return self
                .inherited
                .and_then(|values| values.get(name))
                .map(|value| value.to_string());
        };
        if let Some(known) = self.done.get(name) {
            return known.clone();
        }
        // Going round in a circle makes all of those in it invalid.
        if !self.visiting.insert(name) {
            return None;
        }
        let value = expand(
            raw,
            &mut |other, depth| self.value_of(other, depth),
            depth + 1,
        );
        self.visiting.remove(name);
        let value = value.filter(|value| {
            self.spent += value.len();
            self.spent <= MAX_ELEMENT_BYTES
        });
        self.done.insert(name, value.clone());
        value
    }
}

/// What an element's custom properties come to, given what it inherits and what it declares, with
/// the last of a name winning. `budget` is what the page has left for keeping values; once it is
/// gone, elements only inherit.
pub(super) fn merged(
    parent: Option<&CustomProperties>,
    declared: &[(&str, &str)],
    budget: &mut usize,
) -> Option<CustomProperties> {
    let inherited = parent.map(|parent| parent.0.as_ref());
    let mut resolver = Resolver {
        inherited,
        declared: declared.iter().copied().collect(),
        done: BTreeMap::new(),
        visiting: HashSet::new(),
        spent: 0,
    };
    let names: Vec<&str> = resolver.declared.keys().copied().collect();
    let mut values = inherited.cloned().unwrap_or_default();
    for name in names {
        if let Some(value) = resolver.value_of(name, 0) {
            values.insert(name.into(), value.into());
        }
    }
    let size: usize = values
        .iter()
        .map(|(name, value)| name.len() + value.len() + 2 * size_of::<Box<str>>())
        .sum();
    if size > *budget {
        return parent.cloned();
    }
    *budget -= size;
    Some(CustomProperties(Arc::new(values)))
}
