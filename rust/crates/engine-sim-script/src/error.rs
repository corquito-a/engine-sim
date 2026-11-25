//! The compilation error catalogue.
//!
//! Mirrors `piranha/src/compilation_error.cpp` exactly: the `stage`, `code` and
//! `info` triples are part of the observable behaviour (they are what
//! `es_script::Compiler::printError` writes to `error_log.log`), so they are
//! reproduced verbatim rather than tidied up.

use crate::ir::Token;

/// One entry of `piranha::ErrorCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErrorCode {
    /// `[IO]`, `[S]`, `[P]`, `[E]`, `[R]`, `[V]` or `[T]`.
    pub stage: &'static str,
    pub code: &'static str,
    pub info: &'static str,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}: {}", self.stage, self.code, self.info)
    }
}

macro_rules! error_codes {
    ($($name:ident => ($stage:literal, $code:literal, $info:literal),)*) => {
        #[allow(non_upper_case_globals)] // the names are the C++ ones on purpose
        impl ErrorCode {
            $(
                pub const $name: ErrorCode = ErrorCode {
                    stage: $stage,
                    code: $code,
                    info: $info,
                };
            )*

            /// Every code, in the order `compilation_error.cpp` defines them.
            pub const ALL: &'static [(&'static str, ErrorCode)] = &[
                $((stringify!($name), ErrorCode::$name),)*
            ];
        }
    };
}

error_codes! {
    // [IO] - IO errors
    FileOpenFailed => ("IO", "0010", "Could not open file"),
    // [S] - Scanning errors
    UnidentifiedToken => ("S", "0010", "Unidentified token"),
    // [P] - Parsing errors
    UnexpectedToken => ("P", "0010", "Unexpected token"),
    // [E] - Expansion errors
    InvalidOperandTypes => ("E", "0010", "Invalid operand types"),
    CircularDefinition => ("E", "0020", "Circular definition detected"),
    // [R] - Resolution errors
    UndefinedNodeType => ("R", "0010", "Undefined node type"),
    ArgumentPositionOutOfBounds => ("R", "0020", "Argument position out of bounds"),
    PortNotFound => ("R", "0030", "Port not found"),
    UsingOutputPortAsInput => ("R", "0040", "Using output port as input"),
    UnresolvedReference => ("R", "0050", "Unresolved reference"),
    UndefinedMember => ("R", "0060", "Undefined member"),
    AccessingInternalMember => ("R", "0061", "Invalid reference to an internal member"),
    CannotFindDefaultValue => ("R", "0070", "Cannot find default value"),
    CircularReference => ("R", "0080", "Circular reference detected"),
    // [V] - Validation errors
    InputSpecifiedMultipleTimes => ("V", "0010", "Input specified multiple times"),
    SymbolUsedMultipleTimes => ("V", "0030", "Symbol used multiple times"),
    InputNotConnected => ("V", "0040", "Input not connected"),
    OutputWithNoDefinition => ("V", "0050", "Output with no definition"),
    BuiltinOutputWithDefinition => ("V", "0051", "Builtin output given a definition"),
    BuiltinOutputMissingType => ("V", "0052", "Builtin output missing a type"),
    InputSpecifiedMultipleTimesPositional => ("V", "0011", "Input specified multiple times by position"),
    DuplicateNodeDefinition => ("V", "0060", "Multiple definitions with the same name"),
    UndefinedBuiltinType => ("V", "0070", "Undefined builtin type"),
    UndefinedBuiltinInput => ("V", "0080", "Undefined builtin input"),
    UndefinedBuiltinOutput => ("V", "0081", "Undefined builtin output"),
    ModifyAttributeMismatch => ("V", "0090", "Input modify flag doesn't match builtin port"),
    ToggleAttributeMismatch => ("V", "0100", "Input toggle flag doesn't match builtin port"),
    AliasAttributeMismatch => ("V", "0110", "Alias flag doesn't match builtin port"),
    // [T] - Type errors
    IncompatibleType => ("T", "0010", "Argument with incompatible type specified; valid conversion not found"),
    IncompatibleDefaultType => ("T", "0011", "Default with incompatible type specified; valid conversion not found"),
    IncompatibleOutputDefinitionType => ("T", "0012", "Incompatible output definition type; valid conversion not found"),
}

impl ErrorCode {
    /// Looks a code up by its C++ name (useful for tests and diagnostics).
    pub fn by_name(name: &str) -> Option<ErrorCode> {
        ErrorCode::ALL
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, code)| *code)
    }
}

/// A resolved instantiation context, i.e. the chain of nodes a definition was
/// expanded inside of (`IrContextTree`).
///
/// `es_script::Compiler::printError` walks this to print
/// `While instantiating: <file>(<line>): <instance> <definition>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstantiationFrame {
    /// Index of the node being instantiated in the lowered IR.
    pub node: usize,
}

/// `piranha::CompilationError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilationError {
    pub location: Token,
    pub code: ErrorCode,
    /// `None` when the error is not tied to an instantiation.
    pub instantiation: Option<Vec<InstantiationFrame>>,
}

impl CompilationError {
    pub fn new(location: Token, code: ErrorCode) -> Self {
        CompilationError { location, code, instantiation: None }
    }

    pub fn with_instantiation(
        location: Token,
        code: ErrorCode,
        instantiation: Vec<InstantiationFrame>,
    ) -> Self {
        CompilationError { location, code, instantiation: Some(instantiation) }
    }

    /// `CompilationError::isInstantiationError`
    pub fn is_instantiation_error(&self) -> bool {
        matches!(&self.instantiation, Some(frames) if !frames.is_empty())
    }

    /// The one-line form `es_script::Compiler::printError` writes. The line is
    /// `lineStart`, so an error on a synthesised token prints `2147483647`
    /// rather than the port's internal `0` -- the same translation the dumps do.
    pub fn format_line(&self, unit_stem: &str) -> String {
        format!(
            "{}({}): error {}{}: {}",
            unit_stem,
            self.location.line_start(),
            self.code.stage,
            self.code.code,
            self.code.info
        )
    }
}

/// `piranha::ErrorList`.
#[derive(Clone, Debug, Default)]
pub struct ErrorList {
    errors: Vec<CompilationError>,
}

impl ErrorList {
    pub fn new() -> Self {
        ErrorList::default()
    }

    pub fn add(&mut self, error: CompilationError) {
        self.errors.push(error);
    }

    pub fn errors(&self) -> &[CompilationError] {
        &self.errors
    }

    pub fn len(&self) -> usize {
        self.errors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Sorted `stage+code` multiset, which is what a differential comparison
    /// between implementations can reasonably require: the order errors are
    /// discovered in depends on traversal details that are not specified.
    pub fn code_histogram(&self) -> Vec<(String, usize)> {
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for error in &self.errors {
            let key = format!("{}{}", error.code.stage, error.code.code);
            *counts.entry(key).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_registered_once() {
        assert_eq!(ErrorCode::ALL.len(), 31);

        let mut stages: Vec<&str> = ErrorCode::ALL.iter().map(|(_, code)| code.stage).collect();
        stages.dedup();
        stages.sort_unstable();
        stages.dedup();
        assert_eq!(stages, vec!["E", "IO", "P", "R", "S", "T", "V"]);
    }

    #[test]
    fn codes_match_the_original_strings() {
        // Spot checks straight out of compilation_error.cpp.
        assert_eq!(ErrorCode::FileOpenFailed.code, "0010");
        assert_eq!(ErrorCode::AccessingInternalMember.code, "0061");
        assert_eq!(ErrorCode::InputSpecifiedMultipleTimesPositional.code, "0011");
        assert_eq!(ErrorCode::AliasAttributeMismatch.stage, "V");
        assert_eq!(ErrorCode::IncompatibleOutputDefinitionType.code, "0012");
        assert_eq!(
            ErrorCode::by_name("CircularReference").unwrap().info,
            "Circular reference detected"
        );
        assert!(ErrorCode::by_name("NoSuchCode").is_none());
    }

    #[test]
    fn error_lines_are_formatted_like_the_original() {
        let mut location = Token::new("x");
        location.line = 42;

        let error = CompilationError::new(location, ErrorCode::PortNotFound);
        assert_eq!(
            error.format_line("heads"),
            "heads(42): error R0030: Port not found"
        );
    }
}
