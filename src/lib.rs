//! Memory-safe web engine: takes UTF-8 text and never opens files or sockets itself.

// The generated parser refers to the runtime modules through these crate paths.
use agas_runtime::{ast, lexer, lr};

pub mod css;
pub mod html;
