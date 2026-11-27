//! What the front end costs, phase by phase.
//!
//!     cargo run --release -p engine-sim-script --example front_end_bench -- <root.mr> [search...]
//!
//! The phases are the ones `graph_dump` runs, because they are the ones that
//! have to happen before a script can be simulated: parse and lower, resolve
//! definitions, expand, build the graph. Setup is what a run pays before its
//! first sample, and it is where `radial_9` spends four seconds of a five second
//! invocation, so it is worth knowing which phase owns them.
use engine_sim_script::lower::Arena;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::Rules;
use std::time::Instant;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let entry = match arguments.first() {
        Some(entry) => entry.clone(),
        None => {
            eprintln!("usage: front_end_bench <root.mr> [search-path...]");
            std::process::exit(2);
        }
    };
    let search: Vec<&str> = if arguments.len() > 1 {
        arguments[1..].iter().map(|a| a.as_str()).collect()
    } else {
        vec!["../../es/", "../es/", "es/"]
    };

    let t = Instant::now();
    let graph_units = match load_graph(&entry, &search) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    };
    let parse = t.elapsed();

    let t = Instant::now();
    let mut arena = Arena::lower(&graph_units.units);
    arena.set_rules(Rules::Piranha);
    arena.link_imports(&graph_units.imports);
    let lower = t.elapsed();

    let t = Instant::now();
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    let resolve = t.elapsed();

    let t = Instant::now();
    arena.expand_all();
    let expand = t.elapsed();

    let t = Instant::now();
    let graph = arena.build_graph();
    let build = t.elapsed();

    let total = parse + lower + resolve + expand + build;
    println!("{entry}");
    println!("  parse + load     {:>8.3} s", parse.as_secs_f64());
    println!("  lower            {:>8.3} s", lower.as_secs_f64());
    println!("  resolve          {:>8.3} s", resolve.as_secs_f64());
    println!("  expand_all       {:>8.3} s", expand.as_secs_f64());
    println!("  build_graph      {:>8.3} s", build.as_secs_f64());
    println!("  total            {:>8.3} s", total.as_secs_f64());
    println!(
        "  GraphNode is {} bytes; a full scan of this graph walks {:.1} KB",
        std::mem::size_of::<engine_sim_script::graph::GraphNode>(),
        (std::mem::size_of::<engine_sim_script::graph::GraphNode>() * graph.nodes.len()) as f64 / 1024.0
    );
    println!(
        "  graph: {} nodes, {} ports, {} containers",
        graph.nodes.len(),
        graph.ports.len(),
        graph.containers.len()
    );
}
