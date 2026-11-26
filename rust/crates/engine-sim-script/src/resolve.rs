//! Import resolution.
//!
//! Mirrors `piranha::Compiler::analyze`: units are read in pre-order (a unit is
//! recorded before its imports are followed), imports are resolved relative to
//! the importing file first and then against the compiler's search paths, and
//! `.mr` is appended when the author left the extension off.
//!
//! Path arithmetic follows `std::filesystem::path::operator/` (the patched
//! `piranha::Path`), because that is what decides which key a unit is filed
//! under.

use crate::ir::{Statement, Unit};
use crate::program::Program;
use crate::parser::{parse_unit, ParseError};

use std::collections::HashSet;
use std::path::Path;

#[derive(Debug)]
pub enum LoadError {
    Parse { path: String, error: ParseError },
    Read { path: String, error: std::io::Error },
    ImportNotFound { path: String, import: String },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse { path, error } => write!(f, "{}: {}", path, error),
            LoadError::Read { path, error } => write!(f, "{}: {}", path, error),
            LoadError::ImportNotFound { path, import } => {
                write!(f, "{}: cannot resolve import \"{}\"", path, import)
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// `std::filesystem::path::operator/`
pub fn join(base: &str, path: &str) -> String {
    if is_absolute(path) {
        return path.to_string();
    }

    if base.is_empty() {
        return path.to_string();
    }

    if base.ends_with('/') {
        return format!("{}{}", base, path);
    }

    format!("{}/{}", base, path)
}

pub fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
}

/// `std::filesystem::path::parent_path()`
pub fn parent(path: &str) -> String {
    match path.rfind('/') {
        None => String::new(),
        Some(0) => "/".to_string(),
        Some(index) => path[..index].to_string(),
    }
}

/// Reads a script and every unit it transitively imports.
///
/// `search_paths` are tried in order, like `Compiler::addSearchPath`.
pub fn load(root: &str, search_paths: &[&str]) -> Result<Vec<Unit>, LoadError> {
    Ok(load_graph(root, search_paths)?.units)
}

/// A loaded import graph: the units in pre-order, plus -- for each unit, in
/// order -- the paths of the units its `import` statements resolved to.
///
/// Resolution needs the second half: `IrCompilationUnit::resolveNodeDefinition`
/// walks `m_importStatements` and follows `IrImportStatement::getUnit()`.
#[derive(Debug)]
pub struct LoadedGraph {
    pub units: Vec<Unit>,
    pub imports: Vec<Vec<String>>,
}

/// The whole front end in one call: load `entry` and everything it imports,
/// lower it into an arena, resolve the definitions, expand, and build the node
/// graph into a [`Program`].
///
/// Every driver does exactly this sequence, in this order, and the order is
/// observable (`lower` before `link_imports`, expansion before the graph), so it
/// lives here rather than in each of them.
pub fn program(entry: &str, search_paths: &[&str]) -> Result<Program, LoadError> {
    let graph = load_graph(entry, search_paths)?;

    let mut arena = crate::lower::Arena::lower(&graph.units);
    arena.set_rules(crate::builtin_types::Rules::EngineSim);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    arena.expand_all();

    Ok(Program::new(arena.build_graph()))
}

pub fn load_graph(root: &str, search_paths: &[&str]) -> Result<LoadedGraph, LoadError> {
    let mut units = Vec::new();
    let mut imports = Vec::new();
    let mut visited = HashSet::new();

    analyze(root, search_paths, &mut units, &mut imports, &mut visited)?;

    Ok(LoadedGraph { units, imports })
}

fn analyze(
    path: &str,
    search_paths: &[&str],
    units: &mut Vec<Unit>,
    imports: &mut Vec<Vec<String>>,
    visited: &mut HashSet<String>,
) -> Result<(), LoadError> {
    // `Compiler::getUnit` compares paths with `std::filesystem::equivalent`, so
    // `es/objects/objects.mr` and `es/actions/../objects/objects.mr` are one
    // unit. Canonicalising here reproduces that (and keeps the recorded paths
    // readable, instead of growing a `..` chain per import cycle).
    let canonical = std::fs::canonicalize(path).map_err(|error| LoadError::Read {
        path: path.to_string(),
        error,
    })?;
    let path = canonical.to_string_lossy().into_owned();

    if !visited.insert(path.clone()) {
        return Ok(());
    }

    let source = std::fs::read_to_string(&path)
        .map_err(|error| LoadError::Read { path: path.clone(), error })?;

    let unit = parse_unit(&path, &source)
        .map_err(|error| LoadError::Parse { path: path.clone(), error })?;

    let root_dir = parent(&path);

    // Imports are followed only after the unit itself is recorded, matching
    // the order `Compiler::analyze` pushes units onto `m_units`.
    let mut resolved = Vec::new();
    for statement in &unit.statements {
        let Statement::Import(import) = statement else {
            continue;
        };

        let mut lib_name = import.lib_name.data.clone();
        if !lib_name.ends_with(".mr") {
            lib_name.push_str(".mr");
        }

        let full = if is_absolute(&lib_name) {
            lib_name.clone()
        }
        else {
            join(&root_dir, &lib_name)
        };

        let full = if Path::new(&full).exists() {
            full
        }
        else {
            let mut found = None;
            for search_path in search_paths {
                let candidate = join(search_path, &lib_name);
                if Path::new(&candidate).exists() {
                    found = Some(candidate);
                    break;
                }
            }

            match found {
                Some(candidate) => candidate,
                None => {
                    return Err(LoadError::ImportNotFound {
                        path: path.clone(),
                        import: lib_name,
                    })
                }
            }
        };

        resolved.push(full);
    }

    // The same canonical spelling the unit is filed under, so `link_imports`
    // can match an import to a unit by path.
    let resolved: Vec<String> = resolved
        .into_iter()
        .map(|path| match std::fs::canonicalize(&path) {
            Ok(canonical) => canonical.to_string_lossy().into_owned(),
            Err(_) => path,
        })
        .collect();

    units.push(unit);
    imports.push(resolved.clone());

    for import in resolved {
        analyze(&import, search_paths, units, imports, visited)?;
    }

    Ok(())
}
