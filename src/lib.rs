#![deny(missing_docs)]
//! yajlish is a low-level, event-based json parser based on [yajl](https://github.com/yajl/yajl).

mod common;
mod lexer;

pub use common::{Context, Handler, Status};
