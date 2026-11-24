//! Emits `code,line` for every error the port reports, in discovery order.
//!
//! This is the Rust half of the conformance runner (`tools/parity/run_conformance.py`):
//! the other half is `script_reference --errors` with `PARITY_LINES=1` fed the
//! line-carrying dump. The phases match `IrCompilationUnit::resolveAll` order --
//! definitions, circularity, then validation -- so the two lists are comparable
//! element by element.
//!
//! ```text
//! cargo run -p engine-sim-script --example error_dump -- <root.mr>
//! ```

use engine_sim_script::lower::Arena;
use engine_sim_script::Rules;
use engine_sim_script::resolve::load_graph;


fn main() {
    let mut rules = Rules::EngineSim;
    let mut arguments = std::env::args().skip(1);

    let entry = loop {
        match arguments.next() {
            Some(argument) if argument == "--rules=piranha" => rules = Rules::Piranha,
            Some(argument) => break argument,
            None => {
                eprintln!("usage: error_dump [--rules=piranha] <root.mr>");
                std::process::exit(2);
            }
        }
    };

    // Imports are resolved relative to the importing unit first, which is all
    // the conformance fixtures need (they import `../../../demos/...`).
    let graph = match load_graph(&entry, &[]) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    };

    let mut arena = Arena::lower(&graph.units);
    arena.set_rules(rules);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();

    // `resolveAll()` runs the phases in this order: definitions, circular
    // definitions, expansion, references, validation.
    arena.expand_all();

    arena.check_references_all();

    arena.validate_all();

    for error in arena.errors() {
        let line = error.location.line;
        println!("{}{},{}", error.code.stage, error.code.code, line);
    }
}
