//! Lowers every script that ships with the application and raises it back.
//!
//! The arena is the substrate for resolution, so a lowering that loses anything
//! would silently corrupt every later milestone. The check is exact: lowering
//! then raising has to reproduce the parsed unit, field for field, for every
//! unit reachable from the two entry points.

use engine_sim_script::ir::{Statement, Unit};
use engine_sim_script::lower::{Arena, Kind};
use engine_sim_script::resolve::load;
use engine_sim_script::parse_unit;

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Loads an entry point and every unit it imports, or `None` when the crate is
/// being tested outside the engine-sim checkout.
fn loaded_units(entry: &str) -> Option<Vec<Unit>> {
    let root = repository_root();
    let path = root.join(entry);
    if !path.is_file() {
        return None;
    }

    // The application installs CWD-relative search paths (`../../es/`, `../es/`,
    // `es/`); spell the same directory absolutely so the test does not depend on
    // where cargo runs from.
    let library = root.join("es/");
    let search = [library.to_string_lossy().into_owned()];
    let search: Vec<&str> = search.iter().map(|path| path.as_str()).collect();

    Some(load(&path.to_string_lossy(), &search).expect("the entry point loads"))
}

fn lower_and_check(units: &[Unit]) -> Arena {
    let arena = Arena::lower(units);
    assert_eq!(arena.units().len(), units.len());

    for (id, unit) in arena.units().iter().zip(units.iter()) {
        let raised = arena.to_unit(*id);
        assert_eq!(&raised, unit, "{} did not survive the round trip", unit.path);
    }

    arena
}

/// Counts nodes, attributes and ports below a structure.
fn census(arena: &Arena, id: usize) -> (usize, usize, usize) {
    let mut nodes = 0;
    let mut attributes = 0;
    let mut ports = 0;

    arena.visit(id, |structure| match arena.kind(structure) {
        Kind::Node(_) => nodes += 1,
        Kind::Attribute(_) => attributes += 1,
        Kind::AttributeDefinition(_) => ports += 1,
        _ => {}
    });

    (nodes, attributes, ports)
}

#[test]
fn standard_library_survives_the_round_trip() {
    let Some(units) = loaded_units("es/engine_sim.mr") else {
        return;
    };

    let arena = lower_and_check(&units);

    let mut nodes = 0;
    let mut attributes = 0;
    let mut ports = 0;
    for unit in arena.units() {
        let (n, a, p) = census(&arena, *unit);
        nodes += n;
        attributes += a;
        ports += p;
    }

    // Measured, so a change in the parser shows up here as well as in the dump.
    assert_eq!(units.len(), 18, "the standard library resolves to 18 units");
    assert_eq!((nodes, attributes, ports), (253, 653, 641));
}

#[test]
fn assets_survive_the_round_trip() {
    let Some(units) = loaded_units("assets/main.mr") else {
        return;
    };

    let arena = lower_and_check(&units);

    let (mut nodes, mut attributes, mut ports) = (0, 0, 0);
    for unit in arena.units() {
        let (n, a, p) = census(&arena, *unit);
        nodes += n;
        attributes += a;
        ports += p;
    }

    assert_eq!(units.len(), 21, "the asset graph resolves to 21 units");
    assert_eq!((nodes, attributes, ports), (409, 1026, 677));
}

/// Round-tripping has to preserve the order of a unit's statements, because
/// `IrCompilationUnit`'s components drive the traversal order of every phase.
#[test]
fn statement_order_is_preserved() {
    let Some(units) = loaded_units("assets/main.mr") else {
        return;
    };

    let arena = lower_and_check(&units);

    for (id, unit) in arena.units().iter().zip(units.iter()) {
        let raised = arena.to_unit(*id);

        let kinds = |statements: &[Statement]| -> Vec<&'static str> {
            statements
                .iter()
                .map(|statement| match statement {
                    Statement::Import(_) => "import",
                    Statement::AutoDefinition(_) => "auto",
                    Statement::Definition(_) => "definition",
                    Statement::Node(_) => "node",
                })
                .collect()
        };

        assert!(!unit.statements.is_empty());
        assert_eq!(kinds(&unit.statements), kinds(&raised.statements));
    }
}

/// The component edges are what the resolution phases walk, so check them
/// structurally on a small hand-written unit.
#[test]
fn component_edges_follow_register_component() {
    let source = "node a => __builtin {\n  input x: 1;\n  b(2)\n}\n";
    let unit = parse_unit("test.mr", source).expect("parses");

    let mut arena = Arena::default();
    let unit_id = arena.lower_unit(&unit);

    let components = arena.components(unit_id);
    assert_eq!(components.len(), 1, "the unit has one statement");

    // `setAttributeDefinitionList` then `setBody`.
    let definition = components[0];
    let definition_components = arena.components(definition);
    assert_eq!(definition_components.len(), 2, "ports then body");

    let ports = definition_components[0];
    let port = arena.components(ports)[0];
    assert_eq!(arena.components(port).len(), 1, "the port's default value");

    let body = definition_components[1];
    let node = arena.components(body)[0];
    assert_eq!(arena.components(node).len(), 1, "the node's attribute list");

    // Every edge points back at the structure that registered it, and the scope
    // chain follows the same path.
    assert_eq!(arena.entry(definition).parent, Some(unit_id));
    assert_eq!(arena.entry(ports).parent, Some(definition));
    assert_eq!(arena.entry(port).parent, Some(ports));
    assert_eq!(arena.entry(body).parent, Some(definition));
    assert_eq!(arena.entry(node).parent, Some(body));

    assert_eq!(arena.entry(port).scope_parent, Some(ports));
    assert_eq!(arena.entry(port).logical_parent, Some(ports));
}

/// A node reference is a value, and its value is a node: that is the shape the
/// expansion phase depends on.
#[test]
fn node_references_lower_as_values() {
    let source = "node a {\n  input x: b(1).c;\n}\n";
    let unit = parse_unit("test.mr", source).expect("parses");

    let mut arena = Arena::default();
    let unit_id = arena.lower_unit(&unit);

    let mut refs = 0;
    let mut names = Vec::new();
    arena.visit(unit_id, |structure| {
        if let Kind::NodeRef(node) = arena.kind(structure) {
            refs += 1;
            if let Kind::Node(data) = arena.kind(*node) {
                names.push(data.type_name.data.clone());
            }
        }
    });

    assert_eq!(refs, 1);
    assert_eq!(names, vec!["b".to_string()]);
}

/// `node_member: data_access '.' node` calls `IrNode::setThis`, which appends an
/// attribute literally named `this`. That attribute is how the member's own
/// ports get a value, so the arena has to keep it in place.
#[test]
fn this_attributes_are_preserved() {
    let source = "node a {\n  q.z(1)\n}\n";
    let unit = parse_unit("test.mr", source).expect("parses");

    let mut arena = Arena::default();
    let unit_id = arena.lower_unit(&unit);
    assert_eq!(arena.to_unit(unit_id), unit);

    let mut this_count = 0;
    arena.visit(unit_id, |structure| {
        if let Kind::Attribute(data) = arena.kind(structure) {
            if data.name.data == "this" {
                this_count += 1;
            }
        }
    });

    assert_eq!(this_count, 1, "the member's `this` attribute survives lowering");
}

/// Both `IrAttributeDefinition` constructors call
/// `setVisibility(IrVisibility::Public)`; `IrParserStructure`'s own default is
/// private, so a port that inherits it makes `a.b` fail with
/// `AccessingInternalMember`.
#[test]
fn ports_allow_external_access() {
    let unit = parse_unit("test.mr", "node n { output o [float]: 1.0; }\n").expect("parses");
    let mut arena = Arena::default();
    let unit = arena.lower_unit(&unit);

    let Kind::Unit(data) = arena.kind(unit) else {
        panic!("not a unit");
    };
    let definition = data.definitions[0];
    let Kind::NodeDefinition(definition) = arena.kind(definition) else {
        panic!("not a definition");
    };
    let Kind::AttributeDefinitionList(ports) = arena.kind(definition.attributes) else {
        panic!("not a port list");
    };

    let port = ports[0];
    assert!(arena.allows_external_access(port), "a port is public");
}
