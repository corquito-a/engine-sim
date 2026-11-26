//! Shared by the integration tests: where the fixtures are, and how a script
//! becomes a `Program`.

use engine_sim_script::lower::Arena;
use engine_sim_script::program::Program;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::Rules;

use std::path::{Path, PathBuf};

/// The repository root, which is where `assets/` and the search paths live.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// `rust/tools/parity`, where the recorded fixtures live.
#[allow(dead_code)]
pub fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/parity")
}

/// Loads, resolves and builds a script's node graph through the port's own front
/// end, with the paths `es_script::Compiler::initialize` installs.
pub fn build_program(script: &Path) -> Program {
    let search = repo_root().join("es/");
    let search = search.to_string_lossy().to_string();
    let graph =
        load_graph(&script.to_string_lossy(), &[search.as_str()]).expect("the script loads");

    let mut arena = Arena::lower(&graph.units);
    arena.set_rules(Rules::EngineSim);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    arena.expand_all();

    Program::new(arena.build_graph())
}
