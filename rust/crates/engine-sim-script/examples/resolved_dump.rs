//! Emits the `--resolved` rows for a script, using only what the Rust side
//! knows *today*.
//!
//! `crates/engine-sim-script/fixtures/*.resolved` hold what the original's
//! resolution produced (see `tools/parity/script_reference.cpp --resolved`).
//! This example prints the same rows from the lowered IR, so diffing the two
//! shows exactly how much of M2 is still open: fields the Rust side cannot
//! compute yet come out as `-` and show up as a difference against the fixture.
//!
//! Run it from the repository root:
//!
//! ```text
//! cargo run -p engine-sim-script --example resolved_dump -- es/engine_sim.mr
//! ```

use engine_sim_script::ir::Direction;
use engine_sim_script::lower::{Arena, Kind, StructureId};
use engine_sim_script::resolve::load;

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Input => "input",
        Direction::Output => "output",
        Direction::Modify => "modify",
        Direction::Toggle => "toggle",
    }
}

/// A path as the fixture spells it: relative to the repository root.
fn fixture_path(path: &str) -> String {
    // Spell it the way the fixture does: from the library root down.
    for marker in ["/es/", "/assets/"] {
        if let Some(index) = path.rfind(marker) {
            return path[index + 1..].to_string();
        }
    }

    path.trim_start_matches('/').to_string()
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

    let units = load(&root.join(&entry).to_string_lossy(), &[&library_text])
        .unwrap_or_else(|error| {
            eprintln!("{}", error);
            std::process::exit(1);
        });

    let arena = Arena::lower(&units);

    for unit in arena.units() {
        let Kind::Unit(data) = arena.kind(*unit) else {
            continue;
        };

        let path = fixture_path(&data.path);

        println!("U,{},{},{}", path, data.nodes.len(), data.definitions.len());
        print_unit(&arena, &path, data);
    }
}

fn print_unit(arena: &Arena, path: &str, data: &engine_sim_script::lower::UnitData) {
    for (d, definition) in data.definitions.iter().enumerate() {
        let Kind::NodeDefinition(definition_data) = arena.kind(*definition) else {
            continue;
        };

        let ports: &[StructureId] = match arena.kind(definition_data.attributes) {
            Kind::AttributeDefinitionList(ports) => ports,
            _ => &[],
        };

        for (p, port) in ports.iter().enumerate() {
            let Kind::AttributeDefinition(port_data) = arena.kind(*port) else {
                continue;
            };

            // `typeDef` and `channelType` need `_resolveDefinitions` +
            // `_checkTypes`, so they are the open part of M2.
            println!(
                "P,{},{},{},{},{},{},{},-,-",
                path,
                d,
                definition_data.name.data,
                p,
                port_data.name.data,
                direction_name(port_data.direction),
                if port_data.is_alias { 1 } else { 0 }
            );
        }
    }

    for (n, node) in data.nodes.iter().enumerate() {
        let Kind::Node(node_data) = arena.kind(*node) else {
            continue;
        };

        let attributes: &[StructureId] = match arena.kind(node_data.attributes) {
            Kind::AttributeList(attributes) => attributes,
            _ => &[],
        };

        // The original only prints a row per *attribute*; a node with an empty
        // attribute list contributes nothing.
        for (a, attribute) in attributes.iter().enumerate() {
            let Kind::Attribute(attribute_data) = arena.kind(*attribute) else {
                continue;
            };

            // The port an attribute resolved to is `_resolveDefinitions`' job.
            println!(
                "A,{},{},{},{},{},-,-",
                path, n, node_data.name.data, a, attribute_data.name.data
            );
        }
    }
}
