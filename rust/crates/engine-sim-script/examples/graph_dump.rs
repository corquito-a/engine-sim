//! Emits the node graph the port builds, in the reference's `--graph` format.
//!
//! This is the Rust half of `tools/parity/run_graph.py`: the other half is
//! `script_reference --graph`, which builds the same graph with
//! `IrCompilationUnit::build`. Both print the same records, so the comparison is
//! a text diff:
//!
//! ```text
//! cargo run -p engine-sim-script --example graph_dump -- <root.mr>
//! ```
//!
//! The phases are `resolveAll`'s, in order, because the graph is built from what
//! resolution decided: definitions, circularity, expansion, then `build`.
//! References and types are not needed to *build* the graph (they need it, not
//! the other way round), so they are not run.

use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::Rules;

fn main() {
    let mut rules = Rules::EngineSim;
    let mut arguments = std::env::args().skip(1);

    let entry = loop {
        match arguments.next() {
            Some(argument) if argument == "--rules=piranha" => rules = Rules::Piranha,
            Some(argument) if argument == "--rules=engine-sim" => rules = Rules::EngineSim,
            Some(argument) => break argument,
            None => {
                eprintln!("usage: graph_dump [--rules=piranha|engine-sim] <root.mr> [search-path...]");
                std::process::exit(2);
            }
        }
    };

    let rest: Vec<String> = arguments.collect();
    let search: Vec<&str> = if rest.is_empty() {
        // The paths `es_script::Compiler::initialize` installs, so running from
        // the repository root mirrors the application.
        vec!["../../es/", "../es/", "es/"]
    }
    else {
        rest.iter().map(|argument| argument.as_str()).collect()
    };

    let graph_units = match load_graph(&entry, &search) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    };

    let mut arena = Arena::lower(&graph_units.units);
    arena.set_rules(rules);
    arena.link_imports(&graph_units.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    arena.expand_all();

    let graph = arena.build_graph();

    for line in engine_sim_script::graph::dump(&graph) {
        println!("{}", line);
    }

    // An empty graph and a graph missing its nodes look the same from here, so
    // say which classes could not be built. Upstream's factory answers
    // `nullptr` for these too; the difference is that upstream's rule set has
    // them.
    if !graph.missing_builtins.is_empty() {
        eprintln!(
            "note: {} builtin class(es) not ported: {}",
            graph.missing_builtins.len(),
            graph.missing_builtins.join(", ")
        );
    }
}
