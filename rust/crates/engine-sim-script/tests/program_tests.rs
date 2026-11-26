//! The execute gate: a run has to write what the original's run writes.
//!
//! `tools/parity/execute/*.execute` are recorded from `script_reference
//! --execute`, which builds, initializes and executes with the real
//! `NodeProgram` and lets the nodes that write do it. Each fixture here is built
//! and run by the port's own `Program`, and both the text and whether the run
//! succeeded have to match.
//!
//! `tools/parity/run_graph.py --execute` runs the same comparison through the
//! two binaries; this test is the same gate without a C++ build behind it.

use engine_sim_script::lower::Arena;
use engine_sim_script::program::Program;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::Rules;

use std::path::{Path, PathBuf};

fn execute_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity/execute")
}

fn fixtures() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(execute_dir()) else {
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

/// The recorded shape: a status line, `----`, then the raw text.
fn recording(path: &Path) -> (String, String) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {}", path.display(), error));

    match text.split_once("\n----\n") {
        Some((status, body)) => (status.to_string(), body.to_string()),
        None => panic!("{}: not a recording", path.display()),
    }
}

fn run(script: &Path) -> (String, String) {
    let graph = load_graph(&script.to_string_lossy(), &[]).expect("loads");

    let mut arena = Arena::lower(&graph.units);
    arena.set_rules(Rules::Piranha);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    arena.expand_all();

    let mut program = Program::new(arena.build_graph());
    let result = program.execute();

    let status = match result {
        Ok(()) => "status: ok".to_string(),
        Err(message) => format!("status: error: runtime error: {}", message),
    };

    (status, program.console().to_string())
}

#[test]
fn every_execute_fixture_writes_what_the_reference_writes() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty(), "the execute fixtures are missing");

    let mut checked = 0;
    for fixture in fixtures {
        let expected_path = fixture.with_extension("execute");
        let (expected_status, expected_text) = recording(&expected_path);
        let (status, text) = run(&fixture);

        assert_eq!(
            (status.clone(), text.clone()),
            (expected_status, expected_text),
            "{} runs differently than the reference",
            fixture.display()
        );

        checked += 1;
    }

    assert_eq!(checked, 3, "every execute fixture is checked");
}
