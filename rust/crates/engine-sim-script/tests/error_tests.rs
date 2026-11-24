//! The error gate: the Rust front end has to report the same errors as the
//! original on scripts that are *wrong*.
//!
//! `tools/parity/negative/*.mr` are small broken scripts; the `.errors` file next
//! to each one was recorded from `script_reference --errors`, which runs the real
//! `resolve` + `validate` over the same IR the Rust parser produced. Codes are
//! compared as a multiset, because the order errors are discovered in depends on
//! traversal details the language does not specify.
//!
//! Not covered yet: `validateBuiltinMappings` (V0070-V0110), which needs the
//! language rules' reference nodes. The fixtures avoid builtins for that reason.

use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;

use std::path::{Path, PathBuf};

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity")
}

fn codes_for(script: &Path) -> Option<Vec<String>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    if !script.is_file() || !root.join("es/engine_sim.mr").is_file() {
        return None;
    }

    let library = root.join("es/");
    let library_text = library.to_string_lossy().into_owned();
    let fixture_dir = script.parent().expect("a parent").to_string_lossy().into_owned();
    let search = [library_text.as_str(), fixture_dir.as_str()];

    let graph = load_graph(&script.to_string_lossy(), &search).expect("loads");

    let mut arena = Arena::lower(&graph.units);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    // `IrCompilationUnit::resolveAll` order: definitions, circularity, then
    // validation reports on what resolution decided.
    arena.check_circular_definitions_all();
    // `IrCompilationUnit::resolveAll` expands before validating, so the
    // expansion's own errors (E0010 and friends) belong to this gate too.
    arena.expand_all();
    arena.validate_all();

    let mut codes: Vec<String> = arena
        .errors()
        .iter()
        .map(|error| format!("{}{}", error.code.stage, error.code.code))
        .collect();
    codes.sort();

    Some(codes)
}

fn expected(script: &Path) -> Vec<String> {
    let path = script.with_extension("errors");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {}", path.display(), error));

    let mut codes: Vec<String> = text
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    codes.sort();

    codes
}

fn scripts() -> Vec<PathBuf> {
    let dir = parity_dir().join("negative");
    let mut found = Vec::new();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return found;
    };

    let mut paths: Vec<PathBuf> = entries.filter_map(|entry| entry.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            let root = path.join("root.mr");
            if root.is_file() {
                found.push(root);
            }
        }
        else if path.extension().and_then(|extension| extension.to_str()) == Some("mr") {
            found.push(path);
        }
    }

    found
}

#[test]
fn negative_scripts_report_the_reference_errors() {
    let mut checked = 0;

    for script in scripts() {
        let Some(codes) = codes_for(&script) else {
            return;
        };

        let reference = expected(&script);
        assert_eq!(
            codes,
            reference,
            "{} reports different errors than the reference",
            script.display()
        );

        checked += 1;
    }

    assert_eq!(checked, 17, "every negative fixture is checked");
}
