//! Emits the `--resolved` rows for a script.
//!
//! `crates/engine-sim-script/fixtures/*.resolved` hold what the original's
//! resolution produced (`tools/parity/script_reference.cpp --resolved`); this
//! prints the same rows from the Rust IR. `compare_resolved.py` scores the two,
//! and `tests/resolved_tests.rs` requires them to be identical.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p engine-sim-script --example resolved_dump -- es/engine_sim.mr
//! ```

use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::resolved::resolved_rows;

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let entry = arguments.next().unwrap_or_else(|| {
        eprintln!("usage: resolved_dump <root.mr>");
        std::process::exit(2);
    });

    let root = repository_root();
    let library = root.join("es/");
    let library_text = library.to_string_lossy().into_owned();

    let graph = load_graph(&root.join(&entry).to_string_lossy(), &[&library_text])
        .unwrap_or_else(|error| {
            eprintln!("{}", error);
            std::process::exit(1);
        });

    let mut arena = Arena::lower(&graph.units);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();

    for row in resolved_rows(&arena) {
        println!("{}", row);
    }
}
