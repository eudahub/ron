//! The values of grid layouts: tracks, lines and named areas. Line names in track lists are left out,
//! so a line can only be named through an area. Lengths get resolved as these are read, which happens
//! once for checking and once for each style that uses them.

use std::sync::Arc;

use super::values::{Length, Unit, parse_length};

/// More than this many repeats or lines are not made, whatever a page asks for.
const MAX_REPEATS: i64 = 1000;
const MAX_LINE: i64 = 1000;

/// The size of a track, or either end of `minmax()`.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackSize {
    Auto,
    MinContent,
    MaxContent,
    Px(f32),
    Percent(f32),
    Fr(f32),
    MinMax(Box<TrackSize>, Box<TrackSize>),
}

/// A part of a list of tracks.
#[derive(Clone, Debug, PartialEq)]
pub enum GridTrack {
    One(TrackSize),
    /// `repeat(auto-fill, ...)`, or `auto-fit` when `fit` is true: as many as there is room for.
    AutoRepeat {
        fit: bool,
        tracks: Vec<TrackSize>,
    },
}

/// One end of where an item goes in a grid.
#[derive(Clone, Debug, PartialEq)]
pub enum GridLine {
    Auto,
    /// From the start when positive, and from the end when negative.
    Line(i32),
    Span(u32),
    /// An area of this name.
    Name(Arc<str>),
}

/// An area named in `grid-template-areas`, by the lines it lies between.
#[derive(Clone, Debug, PartialEq)]
pub struct GridArea {
    pub name: Arc<str>,
    pub row_start: u32,
    pub row_end: u32,
    pub column_start: u32,
    pub column_end: u32,
}

/// All that `grid-template-areas` says.
#[derive(Clone, Debug, PartialEq)]
pub struct GridAreas {
    pub rows: u32,
    pub columns: u32,
    pub areas: Vec<GridArea>,
}

/// Splits at white space outside of brackets, and leaves out line names.
fn words(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut depth = 0_u32;
    let mut start = None;
    let mut in_name = false;
    for (index, character) in text.char_indices() {
        match character {
            '[' if depth == 0 => {
                if let Some(from) = start.take() {
                    found.push(&text[from..index]);
                }
                in_name = true;
            }
            ']' if in_name => in_name = false,
            _ if in_name => {}
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => depth = depth.saturating_sub(1),
            _ if character.is_ascii_whitespace() && depth == 0 => {
                if let Some(from) = start.take() {
                    found.push(&text[from..index]);
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if let Some(from) = start {
        found.push(&text[from..]);
    }
    found
}

/// Splits what is between the brackets of a function at its commas.
fn arguments(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                found.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    found.push(text[start..].trim());
    found
}

/// What is between the brackets, if this is a call of the function of this name.
fn function<'a>(word: &'a str, name: &str) -> Option<&'a str> {
    let rest = word.get(..name.len())?;
    if !rest.eq_ignore_ascii_case(name) {
        return None;
    }
    word[name.len()..].strip_prefix('(')?.strip_suffix(')')
}

fn track_size(word: &str, pixels: &impl Fn(Length) -> Option<f32>) -> Option<TrackSize> {
    if word.eq_ignore_ascii_case("auto") {
        return Some(TrackSize::Auto);
    }
    if word.eq_ignore_ascii_case("min-content") {
        return Some(TrackSize::MinContent);
    }
    if word.eq_ignore_ascii_case("max-content") {
        return Some(TrackSize::MaxContent);
    }
    if function(word, "fit-content").is_some() {
        return Some(TrackSize::MaxContent);
    }
    if let Some(inner) = function(word, "minmax") {
        let parts = arguments(inner);
        let [least, most] = parts.as_slice() else {
            return None;
        };
        return Some(TrackSize::MinMax(
            Box::new(track_size(least, pixels)?),
            Box::new(track_size(most, pixels)?),
        ));
    }
    if let Some(number) = word.strip_suffix("fr") {
        let share: f32 = number.parse().ok()?;
        return (share.is_finite() && share >= 0.0).then_some(TrackSize::Fr(share));
    }
    let length = parse_length(word, false)?;
    if length.unit == Unit::Percent {
        return Some(TrackSize::Percent(length.value));
    }
    pixels(length).map(TrackSize::Px)
}

/// Reads a list of tracks. Lengths are resolved by `pixels`. `none` gives an empty list.
pub(super) fn parse_tracks(
    text: &str,
    pixels: &impl Fn(Length) -> Option<f32>,
) -> Option<Vec<GridTrack>> {
    if text.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut tracks = Vec::new();
    for word in words(text) {
        let Some(inner) = function(word, "repeat") else {
            tracks.push(GridTrack::One(track_size(word, pixels)?));
            continue;
        };
        let (count, list) = inner.split_once(',')?;
        let repeated: Vec<TrackSize> = words(list)
            .into_iter()
            .map(|one| track_size(one, pixels))
            .collect::<Option<_>>()?;
        if repeated.is_empty() {
            return None;
        }
        let count = count.trim();
        if count.eq_ignore_ascii_case("auto-fill") || count.eq_ignore_ascii_case("auto-fit") {
            tracks.push(GridTrack::AutoRepeat {
                fit: count.eq_ignore_ascii_case("auto-fit"),
                tracks: repeated,
            });
            continue;
        }
        let times = count.parse::<i64>().ok().filter(|times| *times > 0)?;
        for _ in 0..times.min(MAX_REPEATS) {
            tracks.extend(repeated.iter().cloned().map(GridTrack::One));
        }
    }
    (!tracks.is_empty()).then_some(tracks)
}

/// Reads one end of where an item goes, as `grid-column-start` takes it.
#[must_use]
pub(super) fn parse_line(text: &str) -> Option<GridLine> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.eq_ignore_ascii_case("auto") {
        return Some(GridLine::Auto);
    }
    let clamp = |number: i64| i32::try_from(number.clamp(-MAX_LINE, MAX_LINE)).unwrap_or(0);
    let mut parts = text.split_ascii_whitespace();
    let first = parts.next()?;
    if first.eq_ignore_ascii_case("span") {
        let count = parts
            .next()
            .and_then(|part| part.parse::<i64>().ok())
            .unwrap_or(1);
        return u32::try_from(clamp(count).max(1)).ok().map(GridLine::Span);
    }
    if let Ok(number) = first.parse::<i64>() {
        return (number != 0).then(|| GridLine::Line(clamp(number)));
    }
    Some(GridLine::Name(first.into()))
}

/// Reads `grid-template-areas`, which has to give rectangles, in rows of equal length.
#[must_use]
pub(super) fn parse_areas(text: &str) -> Option<GridAreas> {
    let mut rows: Vec<Vec<&str>> = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let end = rest[1..].find(quote)? + 1;
        rows.push(rest[1..end].split_ascii_whitespace().collect());
        rest = rest[end + 1..].trim_start();
    }
    let columns = rows.first()?.len();
    if columns == 0 || rows.iter().any(|row| row.len() != columns) {
        return None;
    }
    let mut areas: Vec<GridArea> = Vec::new();
    for (row, cells) in (0_u32..).zip(&rows) {
        for (column, &cell) in (0_u32..).zip(cells) {
            // Dots are cells without a name.
            if cell.bytes().all(|byte| byte == b'.') {
                continue;
            }
            if let Some(area) = areas.iter_mut().find(|area| &*area.name == cell) {
                area.row_end = area.row_end.max(row + 1);
                area.column_end = area.column_end.max(column + 1);
            } else {
                areas.push(GridArea {
                    name: cell.into(),
                    row_start: row,
                    row_end: row + 1,
                    column_start: column,
                    column_end: column + 1,
                });
            }
        }
    }
    Some(GridAreas {
        rows: u32::try_from(rows.len()).ok()?,
        columns: u32::try_from(columns).ok()?,
        areas,
    })
}
