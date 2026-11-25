//! The node-graph gate: `build` has to produce the original's graph.
//!
//! `tools/parity/graph/*.graph` are recorded from `script_reference --graph`,
//! which builds the graph with the real `IrCompilationUnit::build` over the IR
//! this crate's parser produced. Each fixture here is built by the port's own
//! `Arena::build_graph` and compared record for record.
//!
//! `tools/parity/run_graph.py` runs the same comparison through the two
//! binaries; this test is the same gate without a C++ build behind it.

use engine_sim_script::graph::dump;
use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::Rules;

use std::path::{Path, PathBuf};

fn graph_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity/graph")
}

/// Every `.mr` fixture keeps its expected records next to it.
fn fixtures() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(graph_dir()) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("mr"))
        .collect();

    paths.sort();
    paths
}

fn build(script: &Path) -> Vec<String> {
    let graph = load_graph(&script.to_string_lossy(), &[]).expect("loads");

    let mut arena = Arena::lower(&graph.units);
    arena.set_rules(Rules::Piranha);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    arena.expand_all();

    let built = arena.build_graph();

    // A fixture may only use classes the factory has: one it does not have is
    // a node the original would have built and this does not, which would show
    // up as a *smaller* graph rather than as a diff in the records.
    assert!(
        built.missing_builtins.is_empty(),
        "{}: unported builtin class(es): {:?}",
        script.display(),
        built.missing_builtins
    );

    dump(&built)
}

#[test]
fn every_graph_fixture_matches_the_reference() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty(), "the graph fixtures are missing");

    let mut checked = 0;
    for fixture in fixtures {
        let expected_path = fixture.with_extension("graph");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|error| panic!("{}: {}", expected_path.display(), error));

        let expected: Vec<String> = expected.lines().map(|line| line.to_string()).collect();
        let actual = build(&fixture);

        assert_eq!(
            actual,
            expected,
            "{} builds a different graph than the reference",
            fixture.display()
        );

        checked += 1;
    }

    assert_eq!(checked, 8, "every graph fixture is checked");
}
