//! The `--resolved` row format, shared by the example and the test.
//!
//! `tools/parity/script_reference.cpp --resolved` writes these rows from the
//! *original*; this module writes them from the Rust IR, so the two can be
//! compared. See `tests/resolved_tests.rs` for the comparison and
//! `fixtures/*.resolved` for the recorded reference output.

use crate::ir::Direction;
use crate::lower::{Arena, Kind, StructureId, UnitData};

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

/// Every row for the whole graph, in the order the original prints them.
pub fn resolved_rows(arena: &Arena) -> Vec<String> {
    let mut rows = Vec::new();

    for unit in arena.units() {
        let Kind::Unit(data) = arena.kind(*unit) else {
            continue;
        };

        let path = fixture_path(&data.path);

        rows.push(format!("U,{},{},{}", path, data.nodes.len(), data.definitions.len()));
        append_unit(arena, &path, data, &mut rows);
    }

    rows
}

fn append_unit(arena: &Arena, path: &str, data: &UnitData, rows: &mut Vec<String>) {
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

            // `channelType` needs `_checkTypes` (and the language rules), so it
            // is still the open part of M2.
            let type_definition = arena
                .effective_type_definition(*port)
                .and_then(|definition| arena.definition_name(definition))
                .unwrap_or("-");

            let channel_type = arena.immediate_channel_type(*port).unwrap_or("-");

            rows.push(format!(
                "P,{},{},{},{},{},{},{},{},{}",
                path,
                d,
                definition_data.name.data,
                p,
                port_data.name.data,
                direction_name(port_data.direction),
                if port_data.is_alias { 1 } else { 0 },
                type_definition,
                channel_type
            ));
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

            let (port_name, direction) = match arena.attribute_definition(*attribute) {
                Some(port) => (
                    arena.port_name(port).unwrap_or("-").to_string(),
                    arena
                        .port_direction(port)
                        .map(direction_name)
                        .unwrap_or("-")
                        .to_string(),
                ),
                None => ("-".to_string(), "-".to_string()),
            };

            rows.push(format!(
                "A,{},{},{},{},{},{},{}",
                path, n, node_data.name.data, a, attribute_data.name.data, port_name, direction
            ));
        }
    }
}
