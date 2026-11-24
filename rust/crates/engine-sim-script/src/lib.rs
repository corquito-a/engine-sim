//! Native Rust front end for the Engine Simulator's `.mr` scripting language.
//!
//! The original implementation is `piranha` (a flex scanner plus a bison LALR
//! parser) driven by the `es_script` binding layer. This crate reproduces the
//! front end -- scanner, parser and the intermediate representation those two
//! produce -- with no dependencies, so it can be verified against the C++
//! original byte for byte.
//!
//! The IR types in [`ir`] mirror the piranha classes one for one. Everything
//! after parsing works on [`lower`], which turns that tree into an
//! identity-preserving arena -- the shape piranha's in-place resolution and
//! pointer-identity checks actually need.

pub mod builtin_types;
pub mod channel_type;
pub mod context;
pub mod definitions;
pub mod dump;
pub mod error;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod name;
pub mod parser;
pub mod resolve;
pub mod resolved;

pub use builtin_types::builtin_channel_type;
pub use channel_type::{ChannelType, ChannelTypeRegistry};
pub use context::{ContextArena, ContextId};
pub use dump::{dump_unit, dump_units};
pub use error::{CompilationError, ErrorCode, ErrorList};
pub use ir::*;
pub use lexer::{tokenize, Kind, Literal, Scanner, Token as LexToken};
pub use parser::{parse_unit, ParseError, Parser};
pub use lower::{Arena, StructureId};
pub use resolve::{load, load_graph, LoadError, LoadedGraph};
pub use resolved::resolved_rows;
