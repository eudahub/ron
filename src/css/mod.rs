//! CSS parsing and selector matching. The cascade comes later.

mod selector;
mod stylesheet;
mod tokenizer;

// The LR tables validate selectors; the reductions are not used because the stylesheet
// parser compiles a selector from its tokens once it is known to be valid.
#[allow(dead_code)]
#[path = "../../generated/css_parser.rs"]
mod css_parser;

pub use selector::{
    AttributeOperator, Combinator, Compound, PseudoClass, Selector, SimpleSelector, TypeSelector,
    matches,
};
pub use stylesheet::{Declaration, StyleRule, Stylesheet};
pub use tokenizer::{CssToken, CssTokenKind, tokenize};
