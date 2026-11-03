//! HTML parsing.

mod attributes;
mod tokenizer;

// Only the lexer tables are used: the grammar accepts every token sequence, so
// running its LR tables would not reject anything.
#[allow(dead_code)]
#[path = "../../generated/html_parser.rs"]
mod html_parser;

pub use attributes::{Attribute, attributes};
pub use tokenizer::{Token, TokenKind, Tokenizer, tag_name};
