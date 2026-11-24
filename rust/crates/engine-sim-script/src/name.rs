//! Name resolution: `IrParserStructure::resolveName` / `resolveLocalName`.
//!
//! Two rules carry the whole thing, and both are surprising enough to be worth
//! stating plainly:
//!
//! * a **node definition** resolves *locally only* -- `IrNodeDefinition::
//!   resolveName` overrides the scope-chain walk, so a definition can never see
//!   a name from the unit that contains it ("Node definitions are not able to
//!   see variables outside of themselves for now");
//! * a **compilation unit** resolves to its *nodes* by symbol name, never to its
//!   definitions -- definitions are reached through `getDefinition`, which is
//!   import-aware, not through `resolveName`.
//!
//! Implemented here: the scope walk and the per-structure local lookups for
//! units, nodes, node definitions and the generic value nodes. The binary
//! operator's overload (`ir_binary_operator.cpp`) belongs to member access and
//! comes with the reference chain.

use crate::lower::{Arena, Kind, StructureId};

impl Arena {
    /// `IrParserStructure::resolveName` -- try locally, then walk the scope
    /// parents. Node definitions stop the walk.
    pub fn resolve_name(&self, mut structure: StructureId, name: &str) -> Option<StructureId> {
        loop {
            if let Some(local) = self.resolve_local_name(structure, name) {
                return Some(local);
            }

            if matches!(self.kind(structure), Kind::NodeDefinition(_)) {
                // `IrNodeDefinition::resolveName` never leaves the definition.
                return None;
            }

            structure = self.entry(structure).scope_parent?;
        }
    }

    /// `IrParserStructure::resolveLocalName` and its overrides.
    pub fn resolve_local_name(&self, structure: StructureId, name: &str) -> Option<StructureId> {
        match self.kind(structure) {
            Kind::Unit(data) => data
                .nodes
                .iter()
                .copied()
                .find(|node| self.node_symbol(*node) == Some(name)),

            Kind::Node(data) => {
                if let Some(attribute) = self.get_attribute(structure, name, None) {
                    return Some(attribute);
                }

                // `IrNode::resolveLocalName` defers to its definition.
                let definition = data.definition?;
                self.resolve_local_name(definition, name)
            }

            Kind::NodeDefinition(data) => {
                let ports = match self.kind(data.attributes) {
                    Kind::AttributeDefinitionList(ports) => ports,
                    _ => return None,
                };

                if let Some(port) = ports.iter().copied().find(|port| match self.kind(*port) {
                    Kind::AttributeDefinition(port) => port.name.data == name,
                    _ => false,
                }) {
                    return Some(port);
                }

                // Then the body's named nodes.
                let body = data.body?;
                let nodes = match self.kind(body) {
                    Kind::NodeList(nodes) => nodes,
                    _ => return None,
                };

                nodes
                    .iter()
                    .copied()
                    .find(|node| self.node_symbol(*node) == Some(name))
            }

            // `IrBinaryOperator::resolveLocalName` is the member-access case and
            // is not ported yet; everything else has no local names.
            _ => None,
        }
    }

    /// The symbol a node was declared with (`IrNode::getName`), i.e. its own
    /// name token -- not its type name.
    pub fn node_symbol(&self, node: StructureId) -> Option<&str> {
        match self.kind(node) {
            Kind::Node(data) if data.name.specified => Some(data.name.data.as_str()),
            _ => None,
        }
    }

    /// `IrNode::getAttribute` -- matches either the resolved port's name or the
    /// attribute's own name, and returns the first match plus its incidence.
    pub fn get_attribute(
        &self,
        node: StructureId,
        name: &str,
        mut count: Option<&mut i32>,
    ) -> Option<StructureId> {
        let Kind::Node(data) = self.kind(node) else {
            return None;
        };

        let attributes = match self.kind(data.attributes) {
            Kind::AttributeList(attributes) => attributes,
            _ => return None,
        };

        if let Some(count) = count.as_deref_mut() {
            *count = 0;
        }

        let mut first = None;
        for attribute in attributes {
            let Kind::Attribute(attribute_data) = self.kind(*attribute) else {
                continue;
            };

            let definition_matches = match attribute_data.definition {
                Some(definition) => match self.kind(definition) {
                    Kind::AttributeDefinition(definition) => definition.name.data == name,
                    _ => false,
                },
                None => false,
            };

            if definition_matches || attribute_data.name.data == name {
                if first.is_none() {
                    first = Some(*attribute);
                }

                if let Some(count) = count.as_deref_mut() {
                    *count += 1;
                }
            }
        }

        first
    }

    /// `IrCompilationUnit::countSymbolIncidence` -- only named nodes count, and
    /// an empty name never matches.
    pub fn count_symbol_incidence(&self, unit: StructureId, name: &str) -> i32 {
        let Kind::Unit(data) = self.kind(unit) else {
            return 0;
        };

        if name.is_empty() {
            return 0;
        }

        data.nodes
            .iter()
            .filter(|node| self.node_symbol(**node) == Some(name))
            .count() as i32
    }
}

#[cfg(test)]
mod tests {
    use crate::lower::{Arena, Kind};
    use crate::parser::parse_unit;

    fn lower(source: &str) -> (Arena, usize) {
        let unit = parse_unit("test.mr", source).expect("parses");
        let mut arena = Arena::default();
        let id = arena.lower_unit(&unit);
        (arena, id)
    }

    fn first_definition(arena: &Arena, unit_id: usize) -> usize {
        match arena.kind(unit_id) {
            Kind::Unit(data) => data.definitions[0],
            _ => panic!("not a unit"),
        }
    }

    fn first_node(arena: &Arena, unit_id: usize) -> usize {
        match arena.kind(unit_id) {
            Kind::Unit(data) => data.nodes[0],
            _ => panic!("not a unit"),
        }
    }

    #[test]
    fn units_resolve_their_named_nodes() {
        let (arena, unit_id) = lower("int p(1)\nint q(2)\n");

        let p = arena.resolve_name(unit_id, "p").expect("p is a symbol");
        let q = arena.resolve_name(unit_id, "q").expect("q is a symbol");
        assert_ne!(p, q);

        assert_eq!(p, first_node(&arena, unit_id));
        assert!(arena.resolve_name(unit_id, "r").is_none());
    }

    #[test]
    fn units_do_not_resolve_their_definitions() {
        // `IrCompilationUnit::resolveLocalName` only walks `m_nodes`.
        let (arena, unit_id) = lower("int p(1)\npublic node d => __builtin { input x; }\n");

        assert!(arena.resolve_name(unit_id, "p").is_some());
        assert!(arena.resolve_name(unit_id, "d").is_none());
    }

    #[test]
    fn empty_node_names_are_not_symbols() {
        let (arena, unit_id) = lower("int p(1)\n");

        assert!(arena.resolve_name(unit_id, "").is_none());
        assert_eq!(arena.count_symbol_incidence(unit_id, ""), 0);
    }

    #[test]
    fn definitions_resolve_their_ports_then_their_body() {
        let (arena, unit_id) = lower(
            "public node d => __builtin {\n  input x;\n  int y(1)\n}\n",
        );
        let definition = first_definition(&arena, unit_id);

        let port = arena.resolve_name(definition, "x").expect("port x");
        assert!(matches!(arena.kind(port), Kind::AttributeDefinition(_)));

        let body_node = arena.resolve_name(definition, "y").expect("body node y");
        assert!(matches!(arena.kind(body_node), Kind::Node(_)));

        assert!(arena.resolve_name(definition, "z").is_none());
    }

    #[test]
    fn definitions_do_not_see_the_enclosing_unit() {
        // `IrNodeDefinition::resolveName` is local-only.
        let (arena, unit_id) =
            lower("int outside(1)\npublic node d => __builtin { input x; }\n");
        let definition = first_definition(&arena, unit_id);

        assert!(arena.resolve_name(unit_id, "outside").is_some());
        assert!(arena.resolve_name(definition, "outside").is_none());
    }

    #[test]
    fn nodes_resolve_their_attributes_first() {
        // `connection_block: '(' attribute_list ')'` is where attributes live;
        // a brace block holds node statements.
        let (arena, unit_id) = lower("int p(x: 1)\n");
        let node = first_node(&arena, unit_id);

        let attribute = arena.resolve_name(node, "x").expect("the attribute");
        assert!(matches!(arena.kind(attribute), Kind::Attribute(_)));
    }

    #[test]
    fn symbol_incidence_counts_named_nodes_only() {
        let (arena, unit_id) = lower("int p(1)\nint p(2)\nint q(3)\n");

        assert_eq!(arena.count_symbol_incidence(unit_id, "p"), 2);
        assert_eq!(arena.count_symbol_incidence(unit_id, "q"), 1);
    }
}
