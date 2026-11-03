//! CSS: parsing, selector matching, the cascade and computed styles.

mod cascade;
mod media;
mod properties;
mod selector;
mod style;
mod stylesheet;
mod tokenizer;
mod values;
mod variables;

// The LR tables validate selectors; the reductions are not used because the stylesheet
// parser compiles a selector from its tokens once it is known to be valid.
#[allow(dead_code)]
#[path = "../../generated/css_parser.rs"]
mod css_parser;

pub use cascade::{
    SheetSource, StyleProgress, Styles, compute_styles, document_sheet_sources,
    document_stylesheets, extend_styles,
};
pub use media::{Viewport, matches_media};
pub use properties::{
    BackgroundRepeat, BorderCollapse, BorderStyle, BoxSizing, Clear, Display, Float, FontStyle,
    LINE_THROUGH, ListStyleType, OVERLINE, Overflow, Position, TextAlign, TextTransform, UNDERLINE,
    VerticalAlign, Visibility, WhiteSpace,
};
pub use selector::{
    AttributeOperator, Combinator, Compound, PseudoClass, Selector, SimpleSelector, TypeSelector,
    matches,
};
pub use style::{ComputedStyle, LineHeight, Sides, Size, VerticalPosition};
pub use stylesheet::{Declaration, StyleRule, Stylesheet};
pub use tokenizer::{CssToken, CssTokenKind, tokenize};
pub use values::Color;
pub use variables::CustomProperties;
