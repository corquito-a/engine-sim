//! The value gate: what the graph computes has to be the reference's values.
//!
//! `tools/parity/values/*.values` are recorded from `script_reference --values`,
//! which builds, initializes and executes the graph and then reads every port
//! through `fullCompute`. Each fixture here is built by the port's own
//! `Arena::build_graph` and read the same way.
//!
//! `tools/parity/run_graph.py --values` runs the same comparison through the
//! two binaries; this test is the same gate without a C++ build behind it.

use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::values::dump;
use engine_sim_script::Rules;

use std::path::{Path, PathBuf};

fn values_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity/values")
}

fn fixtures() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(values_dir()) else {
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

    assert!(
        built.missing_builtins.is_empty(),
        "{}: unported builtin class(es): {:?}",
        script.display(),
        built.missing_builtins
    );

    assert!(
        !built.recursion_exhausted,
        "{}: generation did not terminate",
        script.display()
    );

    dump(&built)
}

#[test]
fn every_value_fixture_matches_the_reference() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty(), "the value fixtures are missing");

    let mut checked = 0;
    for fixture in fixtures {
        let expected_path = fixture.with_extension("values");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|error| panic!("{}: {}", expected_path.display(), error));

        let expected: Vec<String> = expected.lines().map(|line| line.to_string()).collect();
        let actual = build(&fixture);

        assert_eq!(
            actual,
            expected,
            "{} computes different values than the reference",
            fixture.display()
        );

        checked += 1;
    }

    assert_eq!(checked, 3, "every value fixture is checked");
}

#[test]
fn the_string_to_float_conversion_reads_a_prefix() {
    use engine_sim_script::values::cxx_parse_float;

    // `std::stringstream >>` takes the longest parseable prefix and leaves the
    // rest; with no number at all the value stays at its default, 0.
    assert_eq!(cxx_parse_float("12.5 rpm"), 12.5);
    assert_eq!(cxx_parse_float("3.5e2 and then some"), 350.0);
    assert_eq!(cxx_parse_float("not a number"), 0.0);
    assert_eq!(cxx_parse_float("   -7"), -7.0);
    assert_eq!(cxx_parse_float("1e"), 1.0);
    assert_eq!(cxx_parse_float(""), 0.0);
}
