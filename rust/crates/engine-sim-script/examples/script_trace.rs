//! The port's own simulation of a script's engine: the `RUN` gate's other half.
//!
//! `script_reference --run` builds the engine by running the `.mr` script through
//! the original piranha + es_script stack, wraps it in the default vehicle and
//! transmission `EngineSimApplication::loadScript` falls back to, installs the
//! impulse response each exhaust names (or the deterministic table when it names
//! none), holds the starter, and traces every field of
//! `core_trace_common.h`'s `traceFrames`. This does the same from the port's own
//! front end and its own `engine-sim-core` engine, so the two outputs can be
//! compared line for line.
//!
//!     script_trace <root.mr> [search-path...]
//!
//! `PARITY_STEPS` sets the frame count (400, the fixture's).

use engine_sim_script::build;
use engine_sim_script::engine_dump;
use engine_sim_script::lower::Arena;
use engine_sim_script::program::Program;
use engine_sim_script::resolve::load_graph;
use engine_sim_script::trace;
use engine_sim_script::Rules;

fn main() {
    let mut arguments = std::env::args().skip(1);
    let entry = match arguments.next() {
        Some(argument) => argument,
        None => {
            eprintln!("usage: script_trace <root.mr> [search-path...]");
            std::process::exit(2);
        }
    };

    let rest: Vec<String> = arguments.collect();
    let search: Vec<&str> = if rest.is_empty() {
        vec!["../../es/", "../es/", "es/"]
    }
    else {
        rest.iter().map(|argument| argument.as_str()).collect()
    };

    let steps: usize = std::env::var("PARITY_STEPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(400);

    let graph = match load_graph(&entry, &search) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    };

    let mut arena = Arena::lower(&graph.units);
    arena.set_rules(Rules::EngineSim);
    arena.link_imports(&graph.imports);
    arena.resolve_all_definitions();
    arena.check_circular_definitions_all();
    arena.expand_all();

    let mut program = Program::new(arena.build_graph());

    if let Err(message) = program.execute() {
        eprintln!("runtime error: {}", message);
        std::process::exit(1);
    }

    let mut simulator = match build::simulator(&mut program) {
        Ok(simulator) => simulator,
        Err(message) => {
            eprintln!("error: {}", message);
            std::process::exit(1);
        }
    };

    let displacement = simulator.engine.displacement;
    let mut out = String::from("-- engine --\n");

    for line in engine_dump::dump_with_displacement(&mut program, displacement) {
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str(&trace::frames(&mut simulator, steps));

    print!("{}", out);
}
