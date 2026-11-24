//! The M2 gate: the Rust resolution has to reproduce the original's.
//!
//! `fixtures/*.resolved` were recorded from `script_reference --resolved`, which
//! runs the real `resolveDefinitions` + `checkTypes` over the same units. This
//! test re-derives the rows from the Rust IR and requires them to be identical:
//! every port's direction, alias flag, resolved (alias-followed) type
//! definition and channel type, and every unit-level attribute's resolved port.

use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::resolved::resolved_rows;

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn rows_for(entry: &str) -> Option<Vec<String>> {
    let root = repository_root();
    let path = root.join(entry);
    if !path.is_file() {
        return None;
    }

    let library = root.join("es/");
    let library_text = library.to_string_lossy().into_owned();

    let graph = load_graph(&path.to_string_lossy(), &[&library_text])
        .expect("the entry point loads");

    let mut arena = Arena::lower(&graph.units);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();

    assert!(
        arena.errors().is_empty(),
        "resolution reported {} error(s), first: {:?}",
        arena.errors().len(),
        arena.errors().first()
    );

    Some(resolved_rows(&arena))
}

fn reference_rows(name: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(format!("{name}.resolved"));

    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {}", path.display(), error))
        .lines()
        .map(|line| line.to_string())
        .filter(|line| !line.trim().is_empty())
        .collect()
}

fn check(entry: &str, fixture: &str) {
    let Some(rows) = rows_for(entry) else {
        return;
    };

    let reference = reference_rows(fixture);

    assert_eq!(rows.len(), reference.len(), "{entry}: row count");

    let mismatches: Vec<(usize, &String, &String)> = rows
        .iter()
        .zip(reference.iter())
        .enumerate()
        .filter(|(_, (left, right))| left != right)
        .map(|(index, (left, right))| (index, left, right))
        .collect();

    assert!(
        mismatches.is_empty(),
        "{entry}: {} row(s) differ from the reference, first three: {:#?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(3)]
    );
}

#[test]
fn standard_library_resolution_matches_the_reference() {
    check("es/engine_sim.mr", "es_engine_sim");
}

#[test]
fn asset_resolution_matches_the_reference() {
    check("assets/main.mr", "assets_main");
}
