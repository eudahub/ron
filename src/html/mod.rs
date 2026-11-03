//! HTML parsing.

mod attributes;
mod dom;
mod entities;
mod parser;
mod tag;
mod tokenizer;

// Only the lexer tables are used: the grammar accepts every token sequence, so
// running its LR tables would not reject anything.
#[allow(dead_code)]
#[path = "../../generated/html_parser.rs"]
mod html_parser;

pub use attributes::{Attribute, Attributes, attributes};
pub use dom::{Children, Descendants, Document, NodeId, NodeKind};
pub use parser::{Parser, parse};
pub use tag::Tag;
pub use tokenizer::{StreamTokenizer, Token, TokenKind, Tokenizer, tag_name};
