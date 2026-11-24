//! Validation: `validate` / `validateAll` and the per-structure `_validate`
//! overrides.
//!
//! Validation runs after resolution and only reports; it never changes what was
//! resolved. There are exactly three implementations in the original -- for the
//! compilation unit, the node and the node definition (the attribute and
//! attribute-definition classes do not override `_validate`) -- and they check
//! four things:
//!
//! | structure | check | code |
//! |---|---|---|
//! | unit | two nodes with the same symbol | `SymbolUsedMultipleTimes` |
//! | unit | two definitions with the same name | `DuplicateNodeDefinition` |
//! | node | one input filled twice | `InputSpecifiedMultipleTimes` (`...Positional` when the argument was positional) |
//! | node | an input with neither a value nor a default | `InputNotConnected` |
//! | definition | a port or body node name used twice | `SymbolUsedMultipleTimes` |
//!
//! Note the asymmetry in the unit's duplicate-definition check: it counts matches
//! of `resolveLocalNodeDefinition`, so only definitions *visible from the same
//! unit* collide, and an `external` search never enters into it.

use crate::ir::Direction;
use crate::error::ErrorCode;
use crate::lower::{Arena, Kind, StructureId};

impl Arena {
    /// `IrParserStructure::validate` -- components first, once per structure.
    pub fn validate_all(&mut self) {
        for unit in self.units.clone() {
            self.validate(unit);
        }
    }

    fn validate(&mut self, structure: StructureId) {
        if self.entry(structure).flags.validated {
            return;
        }
        self.entry_mut(structure).flags.validated = true;

        for child in self.components(structure) {
            self.validate(child);
        }

        self.validate_of(structure);
    }

    fn validate_of(&mut self, structure: StructureId) {
        match self.kind(structure) {
            Kind::Unit(_) => self.validate_unit(structure),
            Kind::Node(_) => self.validate_node(structure),
            Kind::NodeDefinition(_) => self.validate_node_definition(structure),
            // `IrAttribute` and `IrAttributeDefinition` do not override
            // `_validate`; the base implementation is empty.
            _ => {}
        }
    }

    /// `IrCompilationUnit::_validate`
    fn validate_unit(&mut self, unit: StructureId) {
        let Kind::Unit(data) = self.kind(unit) else {
            return;
        };

        let nodes = data.nodes.clone();
        for node in nodes {
            let Some(symbol) = self.node_symbol(node) else {
                continue;
            };

            if self.count_symbol_incidence(unit, symbol) > 1 {
                let location = self.node_symbol_token(node);
                self.errors
                    .push(crate::error::CompilationError::new(location, ErrorCode::SymbolUsedMultipleTimes));
            }
        }

        let definitions = match self.kind(unit) {
            Kind::Unit(data) => data.definitions.clone(),
            _ => Vec::new(),
        };

        for definition in definitions {
            let name = match self.kind(definition) {
                Kind::NodeDefinition(data) => data.name.clone(),
                _ => continue,
            };

            let (_found, count) = self.resolve_local_node_definition(unit, &name.data, false);
            if count > 1 {
                self.errors.push(crate::error::CompilationError::new(
                    name,
                    ErrorCode::DuplicateNodeDefinition,
                ));
            }
        }
    }

    /// `IrNode::_validate`
    fn validate_node(&mut self, node: StructureId) {
        let attributes = match self.kind(node) {
            Kind::Node(data) => match self.kind(data.attributes) {
                Kind::AttributeList(attributes) => attributes.clone(),
                _ => return,
            },
            _ => return,
        };

        for attribute in &attributes {
            let Some(definition) = self.attribute_definition(*attribute) else {
                // An earlier resolution error; this step is skipped for it.
                continue;
            };

            let positional = match self.kind(*attribute) {
                Kind::Attribute(data) => data.name.data.is_empty(),
                _ => false,
            };

            let mut count = 0;
            self.get_attribute_by_definition(node, definition, Some(&mut count));

            if count > 1 {
                let location = self.entry(*attribute).summary.clone();
                let code = if positional {
                    ErrorCode::InputSpecifiedMultipleTimesPositional
                }
                else {
                    ErrorCode::InputSpecifiedMultipleTimes
                };

                self.errors
                    .push(crate::error::CompilationError::new(location, code));
            }
        }

        let Some(definition) = (match self.kind(node) {
            Kind::Node(data) => data.definition,
            _ => None,
        }) else {
            return;
        };

        let summary = self.entry(node).summary.clone();
        for input in self.input_definitions(definition) {
            let connected = self
                .get_attribute_by_definition(node, input, None)
                .is_some();

            let has_default = match self.kind(input) {
                Kind::AttributeDefinition(data) => data.default_value.is_some(),
                _ => false,
            };

            if !connected && !has_default {
                self.errors.push(crate::error::CompilationError::new(
                    summary.clone(),
                    ErrorCode::InputNotConnected,
                ));
            }
        }
    }

    /// `IrNodeDefinition::_validate`
    fn validate_node_definition(&mut self, definition: StructureId) {
        let (attributes, body) = match self.kind(definition) {
            Kind::NodeDefinition(data) => (data.attributes, data.body),
            _ => return,
        };

        let ports = match self.kind(attributes) {
            Kind::AttributeDefinitionList(ports) => ports.clone(),
            _ => Vec::new(),
        };

        let is_builtin = match self.kind(definition) {
            Kind::NodeDefinition(data) => data.defines_builtin,
            _ => false,
        };

        for port in ports {
            let (name, token, direction, has_default) = match self.kind(port) {
                Kind::AttributeDefinition(data) => (
                    data.name.data.clone(),
                    data.name.clone(),
                    data.direction,
                    data.default_value.is_some(),
                ),
                _ => continue,
            };

            if self.count_definition_symbol_incidence(definition, &name) > 1 {
                self.errors.push(crate::error::CompilationError::new(
                    token.clone(),
                    ErrorCode::SymbolUsedMultipleTimes,
                ));
            }

            // "Check that every output has a definition of the right type."
            if direction == crate::ir::Direction::Output {
                if has_default {
                    if is_builtin {
                        self.errors.push(crate::error::CompilationError::new(
                            token,
                            ErrorCode::BuiltinOutputWithDefinition,
                        ));
                    }
                }
                else if !is_builtin {
                    self.errors.push(crate::error::CompilationError::new(
                        token,
                        ErrorCode::OutputWithNoDefinition,
                    ));
                }
            }
        }

        let nodes = match body {
            Some(body) => match self.kind(body) {
                Kind::NodeList(nodes) => nodes.clone(),
                _ => Vec::new(),
            },
            None => Vec::new(),
        };

        for node in nodes {
            let Some(symbol) = self.node_symbol(node) else {
                continue;
            };

            if self.count_definition_symbol_incidence(definition, symbol) > 1 {
                let token = self.node_symbol_token(node);
                self.errors
                    .push(crate::error::CompilationError::new(token, ErrorCode::SymbolUsedMultipleTimes));
            }
        }

        self.validate_builtin_mapping(definition);
    }

    /// `IrNodeDefinition::validateBuiltinMappings`.
    ///
    /// The builtin name has to be one the language rules registered (`V0070`),
    /// and then every port the definition declares has to match a port on the
    /// builtin's *reference node*: undefined inputs and outputs (`V0080`/
    /// `V0081`), a `modify` port where the reference does not modify its input
    /// (`V0090`), a `toggle` port where it is not a toggle (`V0100`), and an
    /// `alias` output where the reference's primary output is not that name
    /// (`V0110`).
    ///
    /// The port list comes from `builtin_ports.txt`, recorded from the original;
    /// see `record_builtin_ports.sh`.
    fn validate_builtin_mapping(&mut self, definition: StructureId) {
        let (is_builtin, builtin_token, ports) = match self.kind(definition) {
            Kind::NodeDefinition(data) => {
                (data.defines_builtin, data.builtin_name.clone(), data.attributes)
            }
            _ => return,
        };

        if !is_builtin {
            return;
        }

        if !self.rules.is_builtin_node_type(&builtin_token.data) {
            let mut location = builtin_token;
            location.specified = true;
            self.errors.push(crate::error::CompilationError::new(
                location,
                ErrorCode::UndefinedBuiltinType,
            ));
            return;
        }

        // `const Node *reference = m_rules->getReferenceNode(builtinName);`
        // A registered name always has one.
        let Some(reference) = self.rules.builtin_ports(&builtin_token.data) else {
            return;
        };

        let definitions = match self.kind(ports) {
            Kind::AttributeDefinitionList(list) => list.clone(),
            _ => return,
        };

        for port in definitions {
            let Kind::AttributeDefinition(data) = self.kind(port).clone() else {
                continue;
            };

            let name = data.name.data.as_str();

            if data.direction == Direction::Output {
                let Some(reference_port) = reference.outputs.iter().find(|p| p.name == name)
                else {
                    self.errors.push(crate::error::CompilationError::new(
                        data.name,
                        ErrorCode::UndefinedBuiltinOutput,
                    ));
                    continue;
                };

                if reference_port.alias != data.is_alias {
                    self.errors.push(crate::error::CompilationError::new(
                        data.name,
                        ErrorCode::AliasAttributeMismatch,
                    ));
                }

                continue;
            }

            let is_input = matches!(
                data.direction,
                Direction::Input | Direction::Modify | Direction::Toggle
            );

            if !is_input {
                continue;
            }

            let Some(reference_port) = reference.inputs.iter().find(|p| p.name == name) else {
                self.errors.push(crate::error::CompilationError::new(
                    data.name,
                    ErrorCode::UndefinedBuiltinInput,
                ));
                continue;
            };

            // Both mismatches are reported at the direction token, not the name.
            if reference_port.modifies_input != (data.direction == Direction::Modify) {
                self.errors.push(crate::error::CompilationError::new(
                    data.direction_token.clone(),
                    ErrorCode::ModifyAttributeMismatch,
                ));
            }

            if reference_port.toggle != (data.direction == Direction::Toggle) {
                self.errors.push(crate::error::CompilationError::new(
                    data.direction_token.clone(),
                    ErrorCode::ToggleAttributeMismatch,
                ));
            }
        }
    }

    /// `IrNodeDefinition::countSymbolIncidence` -- ports and body nodes share
    /// one namespace.
    pub fn count_definition_symbol_incidence(&self, definition: StructureId, name: &str) -> i32 {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return 0;
        };

        let mut count = 0;

        if let Kind::AttributeDefinitionList(ports) = self.kind(data.attributes) {
            for port in ports {
                if let Kind::AttributeDefinition(port) = self.kind(*port) {
                    if port.name.data == name {
                        count += 1;
                    }
                }
            }
        }

        if let Some(body) = data.body {
            if let Kind::NodeList(nodes) = self.kind(body) {
                for node in nodes {
                    if self.node_symbol(*node) == Some(name) {
                        count += 1;
                    }
                }
            }
        }

        count
    }

    /// `IrNode::getAttribute(IrAttributeDefinition *, int *)` -- count the
    /// attributes that resolved to this port.
    pub fn get_attribute_by_definition(
        &self,
        node: StructureId,
        definition: StructureId,
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
            if self.attribute_definition(*attribute) != Some(definition) {
                continue;
            }

            if first.is_none() {
                first = Some(*attribute);
            }

            if let Some(count) = count.as_deref_mut() {
                *count += 1;
            }
        }

        first
    }

    /// `IrNode::getNameToken`
    fn node_symbol_token(&self, node: StructureId) -> crate::ir::Token {
        match self.kind(node) {
            Kind::Node(data) => data.name.clone(),
            _ => crate::ir::Token::unspecified(),
        }
    }
}

#[cfg(test)]
mod tests {
    //! The authoritative checks live in `tests/error_tests.rs`, which compares
    //! against codes recorded from the original. These only pin the mechanics
    //! on self-contained snippets (no stdlib import, so nothing else can fire).

    use crate::lower::Arena;
    use crate::parser::parse_unit;

    fn run(source: &str) -> Arena {
        let unit = parse_unit("t.mr", source).expect("parses");
        let mut arena = Arena::default();
        arena.lower_unit(&unit);
        arena.link_imports(&[Vec::new()]);
        arena.resolve_all_definitions();
        arena.validate_all();
        arena
    }

    fn codes(arena: &Arena) -> Vec<String> {
        arena
            .errors()
            .iter()
            .map(|error| format!("{}{}", error.code.stage, error.code.code))
            .collect()
    }

    #[test]
    fn an_unconnected_input_is_reported() {
        let arena = run("public node d { input a; }\nd()\n");
        assert_eq!(codes(&arena), vec!["V0040"]);
    }

    #[test]
    fn a_connected_input_is_fine() {
        let arena = run("public node d { input a; }\nd(a: 1)\n");
        assert!(codes(&arena).is_empty(), "{:?}", codes(&arena));
    }

    #[test]
    fn an_input_with_a_default_is_fine() {
        let arena = run("public node d { input a: 1; }\nd()\n");
        assert!(codes(&arena).is_empty(), "{:?}", codes(&arena));
    }

    #[test]
    fn an_output_without_a_definition_is_reported() {
        let arena = run("public node d { output a; }\n");
        assert_eq!(codes(&arena), vec!["V0050"]);
    }

    #[test]
    fn a_duplicate_port_name_is_reported() {
        let arena = run("public node d { input a; output a; }\n");
        assert_eq!(codes(&arena), vec!["V0030", "V0030", "V0050"]);
    }

    #[test]
    fn duplicate_definitions_are_reported_once_each() {
        // `IrCompilationUnit::_validate` reports per definition whose local
        // lookup count exceeds one, so two duplicates produce two errors.
        let arena = run("public node d { }\npublic node d { input a: 1; }\n");
        assert_eq!(codes(&arena), vec!["V0060", "V0060"]);
    }
}
