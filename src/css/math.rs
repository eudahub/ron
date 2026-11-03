//! `calc()`, `min()`, `max()` and `clamp()`. What they give is resolved along with the rest of a
//! style, apart from percentages, which only layout can know what they are of. So a size that still
//! holds one has a form that layout can work out quickly: a number of pixels and a percentage, kept
//! between two more of those. Nearly all of what pages write fits that, and what does not, such as
//! the smallest of three percentages, drops its declaration as anything not understood does.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use super::values::{Length, Unit, parse_length};

/// However deep a page nests brackets, it goes no further than this.
const MAX_DEPTH: u32 = 32;
const MAX_ARGUMENTS: usize = 64;
/// Sizes that hold a percentage are kept for good, of which pages need a few at most. This bounds
/// the memory that a page asking for others each time can take up, which is about 8 MB.
const MAX_KEPT_SIZES: usize = 1 << 16;

/// Pixels and a percentage of something layout knows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Linear {
    pub px: f32,
    pub percent: f32,
}

impl Linear {
    fn of(self, whole: f32) -> f32 {
        self.px + whole * self.percent / 100.0
    }

    fn shifted(self, other: Self) -> Self {
        Self {
            px: self.px + other.px,
            percent: self.percent + other.percent,
        }
    }

    fn scaled(self, factor: f32) -> Self {
        Self {
            px: self.px * factor,
            percent: self.percent * factor,
        }
    }
}

/// A size that depends on what a percentage is of: `value`, kept between `low` and `high`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CalcSize {
    pub value: Linear,
    pub low: Option<Linear>,
    pub high: Option<Linear>,
}

/// A [`CalcSize`], as all sizes in a style hold one of these. There are many sizes in each style and
/// very few of them are like this, so they all stay small, and these are kept together, each once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalcRef(u32);

impl CalcRef {
    /// What it comes to, given what a percentage is of.
    #[must_use]
    pub fn of(self, whole: f32) -> f32 {
        table()
            .read()
            .ok()
            .and_then(|table| table.sizes.get(self.0 as usize).map(|size| size.of(whole)))
            .unwrap_or(0.0)
    }
}

/// All sizes of this kind that have come up, which last as long as the program does.
#[derive(Default)]
struct CalcTable {
    sizes: Vec<CalcSize>,
    numbers: HashMap<[u32; 8], u32>,
}

fn table() -> &'static RwLock<CalcTable> {
    static TABLE: OnceLock<RwLock<CalcTable>> = OnceLock::new();
    TABLE.get_or_init(RwLock::default)
}

/// Gives the place of a size, unless a page has asked for so many that this has to stop.
pub(super) fn keep_calc(size: CalcSize) -> Option<CalcRef> {
    let bits = |linear: Option<Linear>| {
        linear.map_or([u32::MAX, u32::MAX], |linear| {
            [linear.px.to_bits(), linear.percent.to_bits()]
        })
    };
    let (low, high) = (bits(size.low), bits(size.high));
    let key = [
        size.value.px.to_bits(),
        size.value.percent.to_bits(),
        low[0],
        low[1],
        high[0],
        high[1],
        u32::from(size.low.is_some()),
        u32::from(size.high.is_some()),
    ];
    let mut table = table().write().ok()?;
    if let Some(&number) = table.numbers.get(&key) {
        return Some(CalcRef(number));
    }
    if table.sizes.len() >= MAX_KEPT_SIZES {
        return None;
    }
    let number = u32::try_from(table.sizes.len()).ok()?;
    table.sizes.push(size);
    table.numbers.insert(key, number);
    Some(CalcRef(number))
}

impl CalcSize {
    /// What it comes to, given what a percentage is of. Below `low` goes before above `high`.
    #[must_use]
    pub fn of(&self, whole: f32) -> f32 {
        let mut result = self.value.of(whole);
        if let Some(high) = self.high {
            result = result.min(high.of(whole));
        }
        if let Some(low) = self.low {
            result = result.max(low.of(whole));
        }
        result
    }

    fn holds_percent(&self) -> bool {
        self.value.percent != 0.0
            || self.low.is_some_and(|low| low.percent != 0.0)
            || self.high.is_some_and(|high| high.percent != 0.0)
    }

    /// A plain number of pixels if it comes to one.
    pub(super) fn pixels(&self) -> Option<f32> {
        (!self.holds_percent()).then(|| self.of(0.0))
    }

    fn plain(value: Linear) -> Self {
        Self {
            value,
            low: None,
            high: None,
        }
    }

    fn is_plain(&self) -> bool {
        self.low.is_none() && self.high.is_none()
    }

    /// Only a plain percentage needs no more than that.
    pub(super) fn as_plain_percent(&self) -> Option<f32> {
        (self.is_plain() && self.value.px == 0.0).then_some(self.value.percent)
    }
}

/// An expression as written, with its lengths waiting for what they are relative to.
#[derive(Debug, PartialEq)]
pub(super) enum Math {
    Number(f32),
    Length(Length),
    Add(Box<Math>, Box<Math>),
    Subtract(Box<Math>, Box<Math>),
    Multiply(Box<Math>, Box<Math>),
    Divide(Box<Math>, Box<Math>),
    Min(Vec<Math>),
    Max(Vec<Math>),
    Clamp(Box<Math>, Box<Math>, Box<Math>),
}

/// What an expression comes to on the way.
enum Result {
    Number(f32),
    Size(CalcSize),
}

impl Math {
    /// Works it out, with lengths other than percentages given by `pixels`. Nothing when it makes
    /// no sense, as when it adds a number to a length or divides by zero.
    pub(super) fn size(&self, pixels: &impl Fn(Length) -> Option<f32>) -> Option<CalcSize> {
        let size = match self.result(pixels)? {
            Result::Size(size) => size,
            Result::Number(0.0) => CalcSize::plain(Linear {
                px: 0.0,
                percent: 0.0,
            }),
            Result::Number(_) => return None,
        };
        let finite = |linear: Linear| linear.px.is_finite() && linear.percent.is_finite();
        (finite(size.value) && size.low.is_none_or(finite) && size.high.is_none_or(finite))
            .then_some(size)
    }

    fn result(&self, pixels: &impl Fn(Length) -> Option<f32>) -> Option<Result> {
        match self {
            Self::Number(number) => Some(Result::Number(*number)),
            Self::Length(length) if length.unit == Unit::Percent => {
                Some(Result::Size(CalcSize::plain(Linear {
                    px: 0.0,
                    percent: length.value,
                })))
            }
            Self::Length(length) => Some(Result::Size(CalcSize::plain(Linear {
                px: pixels(*length)?,
                percent: 0.0,
            }))),
            Self::Add(left, right) => add(left.result(pixels)?, right.result(pixels)?, 1.0),
            Self::Subtract(left, right) => add(left.result(pixels)?, right.result(pixels)?, -1.0),
            Self::Multiply(left, right) => match (left.result(pixels)?, right.result(pixels)?) {
                (Result::Number(left), Result::Number(right)) => Some(Result::Number(left * right)),
                (Result::Number(factor), Result::Size(size))
                | (Result::Size(size), Result::Number(factor)) => {
                    Some(Result::Size(scaled(size, factor)))
                }
                _ => None,
            },
            Self::Divide(left, right) => {
                let Result::Number(divisor) = right.result(pixels)? else {
                    return None;
                };
                if divisor == 0.0 {
                    return None;
                }
                match left.result(pixels)? {
                    Result::Number(number) => Some(Result::Number(number / divisor)),
                    Result::Size(size) => Some(Result::Size(scaled(size, 1.0 / divisor))),
                }
            }
            Self::Min(parts) => extreme(parts, pixels, true),
            Self::Max(parts) => extreme(parts, pixels, false),
            Self::Clamp(low, value, high) => {
                let lowered = extreme_of(vec![value.result(pixels)?, high.result(pixels)?], true)?;
                extreme_of(vec![low.result(pixels)?, lowered], false)
            }
        }
    }
}

fn add(left: Result, right: Result, sign: f32) -> Option<Result> {
    match (left, right) {
        (Result::Number(left), Result::Number(right)) => Some(Result::Number(left + sign * right)),
        (Result::Size(left), Result::Size(right)) => {
            // Bounds move along with what is added, as long as that does not have bounds too.
            let (kept, added) = if right.is_plain() {
                (left, right.value.scaled(sign))
            } else if left.is_plain() && sign > 0.0 {
                (right, left.value)
            } else {
                return None;
            };
            Some(Result::Size(CalcSize {
                value: kept.value.shifted(added),
                low: kept.low.map(|low| low.shifted(added)),
                high: kept.high.map(|high| high.shifted(added)),
            }))
        }
        _ => None,
    }
}

fn scaled(size: CalcSize, factor: f32) -> CalcSize {
    let (low, high) = if factor < 0.0 {
        (size.high, size.low)
    } else {
        (size.low, size.high)
    };
    CalcSize {
        value: size.value.scaled(factor),
        low: low.map(|low| low.scaled(factor)),
        high: high.map(|high| high.scaled(factor)),
    }
}

fn extreme(
    parts: &[Math],
    pixels: &impl Fn(Length) -> Option<f32>,
    smallest: bool,
) -> Option<Result> {
    let results = parts
        .iter()
        .map(|part| part.result(pixels))
        .collect::<Option<Vec<_>>>()?;
    extreme_of(results, smallest)
}

/// The smallest or largest of these. Plain pixels can be settled now, and leave one bound.
fn extreme_of(results: Vec<Result>, smallest: bool) -> Option<Result> {
    let better = |left: f32, right: f32| {
        if smallest {
            left.min(right)
        } else {
            left.max(right)
        }
    };
    if results
        .iter()
        .all(|result| matches!(result, Result::Number(_)))
    {
        let mut numbers = results.into_iter().map(|result| match result {
            Result::Number(number) => number,
            Result::Size(_) => 0.0,
        });
        let first = numbers.next()?;
        return Some(Result::Number(numbers.fold(first, better)));
    }
    let mut settled: Option<f32> = None;
    let mut open: Vec<CalcSize> = Vec::new();
    for result in results {
        let Result::Size(size) = result else {
            return None;
        };
        match size.pixels() {
            Some(px) => settled = Some(settled.map_or(px, |known| better(known, px))),
            None => open.push(size),
        }
    }
    let bound = settled.map(|px| Linear { px, percent: 0.0 });
    match (open.len(), bound) {
        (0, Some(bound)) => Some(Result::Size(CalcSize::plain(bound))),
        (1, None) => Some(Result::Size(open[0])),
        (1, Some(bound)) => with_bound(open[0], bound, smallest),
        (2, None) if open[1].is_plain() => with_bound(open[0], open[1].value, smallest),
        (2, None) if open[0].is_plain() => with_bound(open[1], open[0].value, smallest),
        _ => None,
    }
}

/// Adds a bound to a size that does not have that one yet.
fn with_bound(size: CalcSize, bound: Linear, smallest: bool) -> Option<Result> {
    let mut size = size;
    let side = if smallest {
        &mut size.high
    } else {
        &mut size.low
    };
    if side.is_some() {
        return None;
    }
    *side = Some(bound);
    Some(Result::Size(size))
}

/// Reads a value that is one of these functions as a whole. Anything else gives nothing.
pub(super) fn parse_math(text: &str) -> Option<Arc<Math>> {
    let lower = text.trim_start().get(..6)?.to_ascii_lowercase();
    if !["calc(", "min(", "max(", "clamp("]
        .iter()
        .any(|name| lower.starts_with(name))
    {
        return None;
    }
    let mut reader = Reader {
        text: text.trim(),
        at: 0,
    };
    let math = reader.factor(0)?;
    reader.skip_space();
    if reader.at != reader.text.len() {
        return None;
    }
    // What makes no sense goes now, as it drops the declaration, and leaves what came before it.
    math.size(&|_| Some(1.0))?;
    Some(Arc::new(math))
}

struct Reader<'a> {
    text: &'a str,
    at: usize,
}

impl Reader<'_> {
    fn rest(&self) -> &str {
        &self.text[self.at..]
    }

    fn skip_space(&mut self) -> bool {
        let before = self.at;
        while self.rest().starts_with(|c: char| c.is_ascii_whitespace()) {
            self.at += 1;
        }
        self.at > before
    }

    fn sum(&mut self, depth: u32) -> Option<Math> {
        let mut left = self.product(depth)?;
        loop {
            let spaced = self.skip_space();
            let operator = self.rest().chars().next();
            if !matches!(operator, Some('+' | '-')) {
                return Some(left);
            }
            self.at += 1;
            // The standard asks for space around these, as they can also start a number.
            if !spaced || !self.skip_space() {
                return None;
            }
            let right = Box::new(self.product(depth)?);
            left = if operator == Some('+') {
                Math::Add(Box::new(left), right)
            } else {
                Math::Subtract(Box::new(left), right)
            };
        }
    }

    fn product(&mut self, depth: u32) -> Option<Math> {
        let mut left = self.factor(depth)?;
        loop {
            let before = self.at;
            self.skip_space();
            let operator = self.rest().chars().next();
            if !matches!(operator, Some('*' | '/')) {
                self.at = before;
                return Some(left);
            }
            self.at += 1;
            self.skip_space();
            let right = Box::new(self.factor(depth)?);
            left = if operator == Some('*') {
                Math::Multiply(Box::new(left), right)
            } else {
                Math::Divide(Box::new(left), right)
            };
        }
    }

    fn factor(&mut self, depth: u32) -> Option<Math> {
        if depth >= MAX_DEPTH {
            return None;
        }
        let depth = depth + 1;
        self.skip_space();
        if self.rest().starts_with('(') {
            self.at += 1;
            let inner = self.sum(depth)?;
            return self.closing(inner);
        }
        let end = self
            .rest()
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '(' | ')' | ',' | '*' | '/'))
            .unwrap_or(self.rest().len());
        let word = self.rest()[..end].to_ascii_lowercase();
        let after = self.rest()[end..].chars().next();
        self.at += end;
        if after == Some('(') {
            self.at += 1;
            return self.function(&word, depth);
        }
        if let Ok(number) = word.parse::<f32>() {
            return number.is_finite().then_some(Math::Number(number));
        }
        parse_length(&word, false).map(Math::Length)
    }

    fn closing(&mut self, inner: Math) -> Option<Math> {
        self.skip_space();
        self.rest().starts_with(')').then(|| {
            self.at += 1;
            inner
        })
    }

    fn function(&mut self, name: &str, depth: u32) -> Option<Math> {
        if name == "calc" {
            let inner = self.sum(depth)?;
            return self.closing(inner);
        }
        let mut parts = vec![self.sum(depth)?];
        loop {
            self.skip_space();
            if self.rest().starts_with(')') {
                self.at += 1;
                break;
            }
            if !self.rest().starts_with(',') || parts.len() >= MAX_ARGUMENTS {
                return None;
            }
            self.at += 1;
            parts.push(self.sum(depth)?);
        }
        match name {
            "min" => Some(Math::Min(parts)),
            "max" => Some(Math::Max(parts)),
            "clamp" if parts.len() == 3 => {
                let high = parts.pop()?;
                let value = parts.pop()?;
                let low = parts.pop()?;
                Some(Math::Clamp(Box::new(low), Box::new(value), Box::new(high)))
            }
            _ => None,
        }
    }
}
