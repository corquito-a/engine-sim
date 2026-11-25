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

/// One curve: its domain, its range, whether the samples came out ordered, and
/// the curve at `PROBES` points across its own domain.
fn print_curve(prefix: &str, node: usize, function: &engine_sim_core::function::Function, tag: &str) {
    let (x0, x1) = function.domain();
    let (y0, y1) = function.range();

    println!(
        "{},{},{}domain,{:016x},{:016x}",
        prefix,
        node,
        tag,
        x0.to_bits(),
        x1.to_bits()
    );
    println!(
        "{},{},{}range,{:016x},{:016x}",
        prefix,
        node,
        tag,
        y0.to_bits(),
        y1.to_bits()
    );
    println!(
        "{},{},{}ordered,{}",
        prefix,
        node,
        tag,
        function.is_ordered() as i32
    );

    for index in 0..=PROBES {
        let x = x0 + (x1 - x0) * (index as f64) / (PROBES as f64);
        let y = function.sample_triangle(x);

        println!(
            "{},{},{}sample,{},{:016x},{:016x}",
            prefix,
            node,
            tag,
            index,
            x.to_bits(),
            y.to_bits()
        );
    }
}

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

        let mut objects: Vec<(usize, usize, &str)> = Vec::new();
        for internal in program.nodes_of_class("function") {
            if let Some(id) = node_ids[internal] {
                objects.push((id, internal, "function"));
            }
        }
        for internal in program.nodes_of_class("fuel") {
            if let Some(id) = node_ids[internal] {
                objects.push((id, internal, "fuel"));
            }
        }
        for internal in program.nodes_of_class("piston") {
            if let Some(id) = node_ids[internal] {
                objects.push((id, internal, "piston"));
            }
        }
        for internal in program.nodes_of_class("connecting_rod") {
            if let Some(id) = node_ids[internal] {
                objects.push((id, internal, "connecting_rod"));
            }
        }
        for internal in program.nodes_of_class("crankshaft") {
            if let Some(id) = node_ids[internal] {
                objects.push((id, internal, "crankshaft"));
            }
        }
        for internal in program.nodes_of_class("vehicle") {
            if let Some(id) = node_ids[internal] {
                objects.push((id, internal, "vehicle"));
            }
        }
        objects.sort();

        for (node, internal, kind) in objects {
            if kind == "function" {
                let function = match program.function(internal) {
                    Some(function) => function,
                    None => continue,
                };

                print_curve("O", node, function, "");
                continue;
            }

            if kind == "piston" {
                let piston = match program.context.piston(internal) {
                    Some(piston) => *piston,
                    None => continue,
                };

                let scalars = [
                    ("piston_mass", piston.mass),
                    ("piston_blowby", piston.blowby_flow_coefficient),
                    ("piston_compression_height", piston.compression_height),
                    ("piston_wrist_pin_position", piston.wrist_pin_position),
                    ("piston_displacement", piston.displacement),
                ];

                for (field, value) in scalars {
                    println!("O,{},{},{:016x}", node, field, value.to_bits());
                }

                continue;
            }

            if kind == "connecting_rod" {
                let rod = match program.context.rod(internal) {
                    Some(rod) => *rod,
                    None => continue,
                };

                let scalars = [
                    ("rod_mass", rod.mass),
                    ("rod_moment_of_inertia", rod.moment_of_inertia),
                    ("rod_center_of_mass", rod.center_of_mass),
                    ("rod_length", rod.length),
                    ("rod_slave_throw", rod.slave_throw),
                ];

                for (field, value) in scalars {
                    println!("O,{},{},{:016x}", node, field, value.to_bits());
                }

                continue;
            }

            if kind == "crankshaft" {
                let crankshaft = match program.context.crankshaft(internal) {
                    Some(crankshaft) => *crankshaft,
                    None => continue,
                };

                let scalars = [
                    ("crank_throw", crankshaft.throw),
                    ("crank_flywheel_mass", crankshaft.flywheel_mass),
                    ("crank_mass", crankshaft.mass),
                    ("crank_friction_torque", crankshaft.friction_torque),
                    ("crank_moment_of_inertia", crankshaft.moment_of_inertia),
                    ("crank_position_x", crankshaft.position_x),
                    ("crank_position_y", crankshaft.position_y),
                    ("crank_tdc", crankshaft.tdc),
                ];

                for (field, value) in scalars {
                    println!("O,{},{},{:016x}", node, field, value.to_bits());
                }

                continue;
            }

            if kind == "vehicle" {
                let vehicle = match program.context.vehicle(internal) {
                    Some(vehicle) => *vehicle,
                    None => continue,
                };

                let scalars = [
                    ("vehicle_mass", vehicle.mass),
                    ("vehicle_drag_coefficient", vehicle.drag_coefficient),
                    ("vehicle_cross_sectional_area", vehicle.cross_sectional_area),
                    ("vehicle_diff_ratio", vehicle.diff_ratio),
                    ("vehicle_tire_radius", vehicle.tire_radius),
                    ("vehicle_rolling_resistance", vehicle.rolling_resistance),
                ];

                for (field, value) in scalars {
                    println!("O,{},{},{:016x}", node, field, value.to_bits());
                }

                continue;
            }

            let fuel = match program.fuel(internal) {
                Some(fuel) => fuel,
                None => continue,
            };

            let scalars = [
                ("molecular_mass", fuel.molecular_mass()),
                ("energy_density", fuel.energy_density()),
                ("density", fuel.density()),
                ("molecular_afr", fuel.molecular_afr()),
                (
                    "burning_efficiency_randomness",
                    fuel.burning_efficiency_randomness(),
                ),
                ("low_efficiency_attenuation", fuel.low_efficiency_attenuation()),
                ("max_burning_efficiency", fuel.max_burning_efficiency()),
                ("max_turbulence_effect", fuel.max_turbulence_effect()),
                ("max_dilution_effect", fuel.max_dilution_effect()),
            ];

            for (field, value) in scalars {
                println!("O,{},fuel_{},{:016x}", node, field, value.to_bits());
            }

            // The turbulence curve belongs to the function node it references, whose
            // own block dumps it.
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
