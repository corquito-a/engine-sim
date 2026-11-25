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
//! `--values` prints the other half of the gate instead: the records
//! `script_reference --values` produces by reading every port's value through
//! `fullCompute`. `--execute` runs the graph the way `NodeProgram` does and
//! prints whatever its side-effecting nodes wrote. `--objects` runs it and then
//! prints the objects the data nodes generate -- the curves `function` nodes
//! build out of the samples `add_sample` pushed onto them.
//!
//! The phases are `resolveAll`'s, in order, because the graph is built from what
//! resolution decided: definitions, circularity, expansion, then `build`.
//! References and types are not needed to *build* the graph (they need it, not
//! the other way round), so they are not run.

use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::Rules;

/// How many points of a curve `--objects` prints, inclusive of both ends.
const PROBES: usize = 16;

fn main() {
    let mut rules = Rules::EngineSim;
    let mut values = false;
    let mut execute = false;
    let mut objects = false;
    let mut arguments = std::env::args().skip(1);

    let entry = loop {
        match arguments.next() {
            Some(argument) if argument == "--rules=piranha" => rules = Rules::Piranha,
            Some(argument) if argument == "--rules=engine-sim" => rules = Rules::EngineSim,
            Some(argument) if argument == "--values" => values = true,
            Some(argument) if argument == "--execute" => execute = true,
            Some(argument) if argument == "--objects" => objects = true,
            Some(argument) => break argument,
            None => {
                eprintln!("usage: graph_dump [--values|--execute|--objects] [--rules=piranha|engine-sim] <root.mr> [search-path...]");
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

    if execute {
        let mut program = engine_sim_script::program::Program::new(graph);
        let result = program.execute();

        print!("{}", program.console());

        if let Err(message) = result {
            eprintln!("runtime error: {}", message);
            std::process::exit(1);
        }

        return;
    }

    if objects {
        // The ids the reference prints are the ones `GraphDumper::assign` gives
        // out (a container walk), not the graph's internal indices.
        let root = graph.root.unwrap_or(0);
        let (_, node_ids) = engine_sim_script::graph::assign_ids(&graph, root);

        let mut program = engine_sim_script::program::Program::new(graph);
        if let Err(message) = program.execute() {
            eprintln!("runtime error: {}", message);
            std::process::exit(1);
        }

        let mut functions: Vec<(usize, usize)> = program
            .function_nodes()
            .into_iter()
            .filter_map(|node| node_ids[node].map(|id| (id, node)))
            .collect();
        functions.sort();

        for (node, internal) in functions {
            let function = match program.function(internal) {
                Some(function) => function,
                None => continue,
            };

            let (x0, x1) = function.domain();
            let (y0, y1) = function.range();

            println!("O,{},domain,{:016x},{:016x}", node, x0.to_bits(), x1.to_bits());
            println!("O,{},range,{:016x},{:016x}", node, y0.to_bits(), y1.to_bits());
            println!("O,{},ordered,{}", node, function.is_ordered() as i32);

            for index in 0..=PROBES {
                let x = x0 + (x1 - x0) * (index as f64) / (PROBES as f64);
                let y = function.sample_triangle(x);

                println!(
                    "O,{},sample,{},{:016x},{:016x}",
                    node,
                    index,
                    x.to_bits(),
                    y.to_bits()
                );
            }
        }

        return;
    }

    let records = if values {
        engine_sim_script::values::dump(&graph)
    }
    else {
        engine_sim_script::graph::dump(&graph)
    };

    for line in records {
        println!("{}", line);
    }

    // The original recurses until its stack is gone on a circular build (a
    // `NoOpNode` instance is the reachable case), so there is no graph to
    // compare: fail rather than print a half-built one.
    if graph.recursion_exhausted {
        eprintln!("error: the script's node generation does not terminate");
        std::process::exit(1);
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
