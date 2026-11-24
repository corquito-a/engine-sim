//! Native Rust front end for the Engine Simulator's `.mr` scripting language.
//!
//! The original implementation is `piranha` (a flex scanner plus a bison LALR
//! parser) driven by the `es_script` binding layer. This crate reproduces the
//! front end -- scanner, parser and the intermediate representation those two
//! produce -- with no dependencies, so it can be verified against the C++
//! original byte for byte.
//!
//! The IR types in [`ir`] mirror the piranha classes one for one; later
//! milestones (definition resolution, node graph construction, `EngineContext`
//! bindings) build on them.

pub mod dump;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod resolve;

pub use dump::{dump_unit, dump_units};
pub use ir::*;
pub use lexer::{tokenize, Kind, Literal, Scanner, Token as LexToken};
pub use parser::{parse_unit, ParseError, Parser};
pub use resolve::{load, LoadError};
