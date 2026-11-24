//! Definition resolution: `_resolveDefinitions` and the node-definition lookup
//! it is built on.
//!
//! This is the phase that answers "what does this name mean". For a port it
//! turns the declared type (`float_channel`, possibly `ns::float_channel`) into
//! the `IrNodeDefinition` it names, and reports `UndefinedNodeType` when
//! nothing matches. The lookup itself is import- and visibility-aware, in a way
//! that is easy to get subtly wrong:
//!
//! * a name **without** a library searches the local definitions first, and only
//!   then the imports -- so a local definition shadows an imported one;
//! * `ns::name` searches *only* imports whose short name is `ns` (a local
//!   definition can never satisfy a qualified name);
//! * an empty library (`::name`) forces the local scope, imports excluded;
//! * walking into an import sets `external` for that unit's search, so a
//!   `private` definition is invisible from outside even though it is visible
//!   from the unit that declares it;
//! * the first match wins, but *every* match is counted, which is what the
//!   "ambiguous type" diagnostic would eventually use.

use crate::error::{CompilationError, ErrorCode};
use crate::ir::Visibility;
use crate::lower::{Arena, Kind, StructureId};

impl Arena {
    /// `IrParserStructure::allowsExternalAccess`
    pub fn allows_external_access(&self, structure: StructureId) -> bool {
        let entry = self.entry(structure);
        let effective = if entry.visibility == Visibility::Default {
            entry.default_visibility
        }
        else {
            entry.visibility
        };

        effective == Visibility::Public
    }

    /// `IrCompilationUnit::resolveLocalNodeDefinition`
    pub fn resolve_local_node_definition(
        &self,
        unit: StructureId,
        name: &str,
        external: bool,
    ) -> (Option<StructureId>, i32) {
        let Kind::Unit(data) = self.kind(unit) else {
            return (None, 0);
        };

        let mut found = None;
        let mut count = 0;

        for definition in &data.definitions {
            if external && !self.allows_external_access(*definition) {
                continue;
            }

            let Kind::NodeDefinition(definition_data) = self.kind(*definition) else {
                continue;
            };

            if definition_data.name.data == name {
                count += 1;
                if found.is_none() {
                    found = Some(*definition);
                }
            }
        }

        (found, count)
    }

    /// `IrCompilationUnit::resolveNodeDefinition`
    pub fn resolve_node_definition(
        &self,
        unit: StructureId,
        name: &str,
        library: &str,
        external: bool,
    ) -> (Option<StructureId>, i32) {
        let mut count = 0;

        // A local search only happens when no library was named.
        if library.is_empty() {
            let (local, local_count) = self.resolve_local_node_definition(unit, name, external);
            count += local_count;
            if local.is_some() {
                return (local, count);
            }
        }

        let Kind::Unit(data) = self.kind(unit) else {
            return (None, count);
        };

        let mut first = None;
        for import in &data.imports {
            let Kind::Import(import_data) = self.kind(*import) else {
                continue;
            };

            // An import that failed to resolve is skipped outright.
            let Some(target) = import_data.unit else {
                continue;
            };

            let library_matches = match &import_data.short_name {
                Some(short) => library == short.data,
                None => false,
            };

            if !library.is_empty() && !library_matches {
                continue;
            }

            if external && !self.allows_external_access(*import) {
                continue;
            }

            // Crossing into another unit always counts as external access.
            let (definition, secondary) = self.resolve_node_definition(target, name, "", true);
            if definition.is_some() {
                count += secondary;
                if first.is_none() {
                    first = definition;
                }
            }
        }

        (first, count)
    }

    /// `IrParserStructure::resolveDefinitions` -- components first (the
    /// original recurses before calling `_resolveDefinitions`), once per
    /// structure.
    pub fn resolve_all_definitions(&mut self) {
        // `IrCompilationUnit::resolveAll` resolves its imports before itself,
        // and the dump lists units root-first, so the reverse order is the
        // dependency order: an imported unit's errors are reported first.
        for unit in self.units.clone().into_iter().rev() {
            self.resolve_definitions(unit);
        }
    }

    pub(crate) fn resolve_definitions(&mut self, structure: StructureId) {
        if self.entry(structure).flags.definitions_resolved {
            return;
        }
        self.entry_mut(structure).flags.definitions_resolved = true;

        for child in self.components(structure) {
            self.resolve_definitions(child);
        }

        self.resolve_definitions_of(structure);
    }

    fn resolve_definitions_of(&mut self, structure: StructureId) {
        match self.kind(structure) {
            Kind::AttributeDefinition(_) => self.resolve_attribute_definition_type(structure),
            // `IrNode::_resolveDefinitions`
            Kind::Node(_) => {
                self.resolve_node_definition_of(structure);
                self.resolve_attribute_definitions(structure);
            }
            _ => {}
        }
    }

    /// `IrNode::resolveNodeDefinition`
    fn resolve_node_definition_of(&mut self, node: StructureId) {
        let Kind::Node(data) = self.kind(node) else {
            return;
        };

        // `auto node` instances already know their definition.
        if data.definition.is_some() {
            return;
        }

        let type_name = data.type_name.data.clone();
        let type_token = data.type_name.clone();
        let library = data.library.clone();

        let Some(unit) = self.entry(node).parent_unit else {
            return;
        };

        let (definition, _count) = if library.specified {
            if !library.data.is_empty() {
                self.resolve_node_definition(unit, &type_name, &library.data, false)
            }
            else {
                self.resolve_local_node_definition(unit, &type_name, false)
            }
        }
        else {
            self.resolve_node_definition(unit, &type_name, "", false)
        };

        if definition.is_none() {
            let mut location = type_token;
            location.specified = true;
            self.errors.push(CompilationError::new(location, ErrorCode::UndefinedNodeType));
        }

        let Kind::Node(data) = &mut self.entries[node].kind else {
            unreachable!()
        };
        data.definition = definition;
    }

    /// The input-like ports of a definition, in declaration order:
    /// `IrAttributeDefinitionList::getInputDefinition` skipping outputs.
    pub(crate) fn input_definitions(&self, definition: StructureId) -> Vec<StructureId> {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return Vec::new();
        };

        let Kind::AttributeDefinitionList(ports) = self.kind(data.attributes) else {
            return Vec::new();
        };

        ports
            .iter()
            .copied()
            .filter(|port| match self.kind(*port) {
                Kind::AttributeDefinition(port) => port.direction != crate::ir::Direction::Output,
                _ => false,
            })
            .collect()
    }

    /// `IrNodeDefinition::getAttributeDefinition` -- the first port with this
    /// name.
    fn port_by_name(&self, definition: StructureId, name: &str) -> Option<StructureId> {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return None;
        };

        let Kind::AttributeDefinitionList(ports) = self.kind(data.attributes) else {
            return None;
        };

        ports.iter().copied().find(|port| match self.kind(*port) {
            Kind::AttributeDefinition(port) => port.name.data == name,
            _ => false,
        })
    }

    /// `IrNode::resolveAttributeDefinitions` -- every attribute is matched to
    /// the port it fills: positional attributes by index among the input-like
    /// ports, named ones by name.
    fn resolve_attribute_definitions(&mut self, node: StructureId) {
        let Some(definition) = (match self.kind(node) {
            Kind::Node(data) => data.definition,
            _ => None,
        }) else {
            return;
        };

        let attributes = match self.kind(node) {
            Kind::Node(data) => match self.kind(data.attributes) {
                Kind::AttributeList(attributes) => attributes.clone(),
                _ => return,
            },
            _ => return,
        };

        let inputs = self.input_definitions(definition);

        for (position, attribute) in attributes.iter().enumerate() {
            let (positional, name) = match self.kind(*attribute) {
                Kind::Attribute(data) => (data.name.data.is_empty(), data.name.data.clone()),
                _ => continue,
            };

            let summary = self.entry(*attribute).summary.clone();

            if positional {
                if position >= inputs.len() {
                    self.errors.push(CompilationError::new(
                        summary,
                        ErrorCode::ArgumentPositionOutOfBounds,
                    ));
                    self.set_attribute_definition(*attribute, None);

                    // The original stops at the first out-of-bounds argument.
                    return;
                }

                let port = inputs[position];
                self.set_attribute_definition(*attribute, Some(port));
                continue;
            }

            match self.port_by_name(definition, &name) {
                None => {
                    self.errors
                        .push(CompilationError::new(summary, ErrorCode::PortNotFound));
                    self.set_attribute_definition(*attribute, None);
                }
                Some(port) => {
                    let direction = match self.kind(port) {
                        Kind::AttributeDefinition(data) => data.direction,
                        _ => unreachable!(),
                    };

                    if direction == crate::ir::Direction::Output {
                        self.errors.push(CompilationError::new(
                            summary,
                            ErrorCode::UsingOutputPortAsInput,
                        ));
                        self.set_attribute_definition(*attribute, None);
                    }
                    else {
                        self.set_attribute_definition(*attribute, Some(port));
                    }
                }
            }
        }
    }

    fn set_attribute_definition(&mut self, attribute: StructureId, definition: Option<StructureId>) {
        let Kind::Attribute(data) = &mut self.entries[attribute].kind else {
            return;
        };

        data.definition = definition;
    }

    /// The port an attribute resolved to (`IrAttribute::getAttributeDefinition`).
    pub fn attribute_definition(&self, attribute: StructureId) -> Option<StructureId> {
        match self.kind(attribute) {
            Kind::Attribute(data) => data.definition,
            _ => None,
        }
    }

    /// `IrAttributeDefinition::getImmediateChannelType` -- the channel type of
    /// the port's *declared* type definition, when that definition is a
    /// builtin.
    pub fn immediate_channel_type(&self, port: StructureId) -> Option<&'static str> {
        let definition = self.type_definition(port)?;

        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return None;
        };

        if !data.defines_builtin {
            return None;
        }

        self.rules().channel_type(&data.builtin_name.data)
    }

    /// The name of a resolved port (`IrAttributeDefinition::getName`), for
    /// dumps.
    pub fn port_name(&self, port: StructureId) -> Option<&str> {
        match self.kind(port) {
            Kind::AttributeDefinition(data) => Some(data.name.data.as_str()),
            _ => None,
        }
    }

    /// The direction of a resolved port, for dumps.
    pub fn port_direction(&self, port: StructureId) -> Option<crate::ir::Direction> {
        match self.kind(port) {
            Kind::AttributeDefinition(data) => Some(data.direction),
            _ => None,
        }
    }

    /// `IrAttributeDefinition::_resolveDefinitions`
    fn resolve_attribute_definition_type(&mut self, structure: StructureId) {
        let Kind::AttributeDefinition(data) = self.kind(structure) else {
            return;
        };

        let Some((library_token, type_token)) = data.type_info.clone() else {
            return;
        };

        // "No action is needed if a type wasn't specified."
        if !type_token.specified {
            return;
        }

        let Some(unit) = self.entry(structure).parent_unit else {
            return;
        };

        let (definition, _count) = if library_token.specified {
            if !library_token.data.is_empty() {
                self.resolve_node_definition(unit, &type_token.data, &library_token.data, false)
            }
            else {
                // An empty library name forces the local scope.
                self.resolve_local_node_definition(unit, &type_token.data, false)
            }
        }
        else {
            self.resolve_node_definition(unit, &type_token.data, "", false)
        };

        if definition.is_none() {
            let mut location = type_token.clone();
            location.specified = true;
            self.errors.push(CompilationError::new(location, ErrorCode::UndefinedNodeType));
        }

        let Kind::AttributeDefinition(data) = &mut self.entries[structure].kind else {
            unreachable!()
        };
        data.type_definition = definition;
    }

    /// `IrAttributeDefinitionList::getAliasOutput` -- the first `alias output`
    /// port, which is what a type definition is allowed to stand in for.
    pub fn alias_output(&self, definition: StructureId) -> Option<StructureId> {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return None;
        };

        let Kind::AttributeDefinitionList(ports) = self.kind(data.attributes) else {
            return None;
        };

        ports.iter().copied().find(|port| match self.kind(*port) {
            Kind::AttributeDefinition(port) => {
                port.is_alias && port.direction == crate::ir::Direction::Output
            }
            _ => false,
        })
    }

    /// `IrNodeDefinition::getAliasType` -- follow the `alias output` chain of a
    /// type definition until it stops pointing at another type.
    ///
    /// A port's *effective* type is not the definition it named but that
    /// definition's alias type: `[::string]` resolves to the `string`
    /// definition, whose alias output is declared `[::string_channel]`, and the
    /// reference reports `string_channel`. The chain is followed recursively
    /// with no cycle guard, exactly as the original does.
    pub fn alias_type(&self, definition: StructureId) -> StructureId {
        let Some(alias) = self.alias_output(definition) else {
            return definition;
        };

        match self.type_definition(alias) {
            Some(next) => self.alias_type(next),
            None => definition,
        }
    }

    /// `IrAttributeDefinition::getTypeDefinition` -- the alias type of the
    /// definition the declared type resolved to.
    pub fn effective_type_definition(&self, port: StructureId) -> Option<StructureId> {
        let definition = self.type_definition(port)?;
        Some(self.alias_type(definition))
    }

    /// The `IrNodeDefinition` a port's type resolved to, if any.
    pub fn type_definition(&self, port: StructureId) -> Option<StructureId> {
        match self.kind(port) {
            Kind::AttributeDefinition(data) => data.type_definition,
            _ => None,
        }
    }

    /// The name a resolved definition was declared with, for dumps.
    pub fn definition_name(&self, definition: StructureId) -> Option<&str> {
        match self.kind(definition) {
            Kind::NodeDefinition(data) => Some(data.name.data.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ir::Visibility;
    use crate::lower::{Arena, Kind};
    use crate::parser::parse_unit;

    struct Graph {
        arena: Arena,
        units: Vec<usize>,
    }

    impl Graph {
        fn unit(&self, index: usize) -> usize {
            self.units[index]
        }

        fn definitions(&self, unit: usize) -> Vec<usize> {
            match self.arena.kind(unit) {
                Kind::Unit(data) => data.definitions.clone(),
                _ => Vec::new(),
            }
        }
    }

    /// Builds a graph by hand: units are given in pre-order, with the import
    /// paths each unit resolved, exactly as `resolve::load_graph` reports them.
    fn graph(units: &[(&str, &str)], imports: &[&[&str]]) -> Graph {
        let parsed: Vec<_> = units
            .iter()
            .map(|(path, source)| parse_unit(path, source).expect("parses"))
            .collect();

        let mut arena = Arena::default();
        for unit in &parsed {
            arena.lower_unit(unit);
        }

        let paths: Vec<Vec<String>> = imports
            .iter()
            .map(|list| list.iter().map(|path| path.to_string()).collect())
            .collect();

        arena.link_imports(&paths);

        let units = arena.units().to_vec();
        Graph { arena, units }
    }

    #[test]
    fn a_local_definition_satisfies_a_port_type() {
        let mut graph = graph(
            &[("a.mr", "public node ch { input x; }\npublic node holder { input p [ch]; }\n")],
            &[&[]],
        );

        let unit = graph.unit(0);
        let definitions = graph.definitions(unit);
        let holder = definitions[1];

        graph.arena.resolve_all_definitions();

        let Kind::NodeDefinition(data) = graph.arena.kind(holder) else {
            panic!("not a definition");
        };
        let Kind::AttributeDefinitionList(ports) = graph.arena.kind(data.attributes) else {
            panic!("no ports");
        };

        assert_eq!(graph.arena.type_definition(ports[0]), Some(definitions[0]));
        assert!(graph.arena.errors().is_empty());
    }

    #[test]
    fn a_port_type_follows_the_alias_chain() {
        // `[::inner]` resolves to `inner`, whose `alias output` names `base`,
        // so the effective type is `base` -- one hop further than the name the
        // author wrote.
        let mut graph = graph(
            &[(
                "a.mr",
                "public node base { input x; }\n\
                 public node inner { alias output __out [::base]; }\n\
                 public node holder { input p [::inner]; }\n",
            )],
            &[&[]],
        );

        let unit = graph.unit(0);
        let definitions = graph.definitions(unit);
        let holder = definitions[2];

        graph.arena.resolve_all_definitions();

        let Kind::NodeDefinition(data) = graph.arena.kind(holder) else {
            panic!("not a definition");
        };
        let Kind::AttributeDefinitionList(ports) = graph.arena.kind(data.attributes) else {
            panic!("no ports");
        };

        // The declared type is `inner` ...
        assert_eq!(graph.arena.type_definition(ports[0]), Some(definitions[1]));
        // ... but the effective type is what `inner`'s alias output names.
        assert_eq!(
            graph.arena.effective_type_definition(ports[0]),
            Some(definitions[0])
        );

        // A type with no alias output is its own alias type.
        assert_eq!(graph.arena.alias_type(definitions[0]), definitions[0]);
    }

    #[test]
    fn an_undefined_type_is_reported() {
        let mut graph = graph(
            &[("a.mr", "public node holder { input p [nope]; }\n")],
            &[&[]],
        );

        graph.arena.resolve_all_definitions();

        let errors = graph.arena.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, crate::error::ErrorCode::UndefinedNodeType);
        assert_eq!(errors[0].location.data, "nope");
    }

    #[test]
    fn a_private_definition_is_not_visible_through_an_import() {
        let mut graph = graph(
            &[
                ("root.mr", "import \"a.mr\" as lib\npublic node holder { input p [lib::hidden]; }\n"),
                ("a.mr", "private node hidden { input x; }\n"),
            ],
            &[&["a.mr"], &[]],
        );

        graph.arena.resolve_all_definitions();
        assert_eq!(graph.arena.errors().len(), 1);
    }

    #[test]
    fn a_public_definition_is_visible_through_a_qualified_name() {
        let mut graph = graph(
            &[
                ("root.mr", "import \"a.mr\" as lib\npublic node holder { input p [lib::shown]; }\n"),
                ("a.mr", "public node shown { input x; }\n"),
            ],
            &[&["a.mr"], &[]],
        );

        let unit = graph.unit(0);
        let holder = graph.definitions(unit)[0];

        graph.arena.resolve_all_definitions();

        let Kind::NodeDefinition(data) = graph.arena.kind(holder) else {
            panic!("not a definition");
        };
        let Kind::AttributeDefinitionList(ports) = graph.arena.kind(data.attributes) else {
            panic!("no ports");
        };

        let target = graph.arena.type_definition(ports[0]).expect("resolved");
        assert_eq!(graph.arena.definition_name(target), Some("shown"));
        assert!(graph.arena.errors().is_empty());
    }

    #[test]
    fn a_local_definition_shadows_an_imported_one() {
        let mut graph = graph(
            &[
                ("root.mr", "import \"a.mr\" as lib\npublic node shown { input x; }\npublic node holder { input p [shown]; }\n"),
                ("a.mr", "public node shown { input y; }\n"),
            ],
            &[&["a.mr"], &[]],
        );

        let unit = graph.unit(0);
        let definitions = graph.definitions(unit);
        let holder = definitions[1];

        graph.arena.resolve_all_definitions();

        let Kind::NodeDefinition(data) = graph.arena.kind(holder) else {
            panic!("not a definition");
        };
        let Kind::AttributeDefinitionList(ports) = graph.arena.kind(data.attributes) else {
            panic!("no ports");
        };

        // The local `shown` (definitions[0]), not the imported one.
        assert_eq!(graph.arena.type_definition(ports[0]), Some(definitions[0]));
        assert_eq!(
            graph.arena.entry(graph.arena.type_definition(ports[0]).unwrap()).parent_unit,
            Some(unit)
        );
    }

    #[test]
    fn an_empty_library_forces_the_local_scope() {
        let mut graph = graph(
            &[
                ("root.mr", "import \"a.mr\"\npublic node holder { input p [::only_here]; }\n"),
                ("a.mr", "public node only_here { input x; }\n"),
            ],
            &[&["a.mr"], &[]],
        );

        graph.arena.resolve_all_definitions();
        assert_eq!(graph.arena.errors().len(), 1, "the import is not searched");
    }

    #[test]
    fn visibility_defaults_to_private_but_definitions_default_to_public() {
        let graph = graph(
            &[("a.mr", "node d { input x; }\npublic node e { input x; }\nprivate node f { input x; }\n")],
            &[&[]],
        );

        let definitions = graph.definitions(graph.unit(0));

        // `IrNodeDefinition`'s constructor sets `m_defaultVisibility = Public`.
        assert!(graph.arena.allows_external_access(definitions[0]));
        assert!(graph.arena.allows_external_access(definitions[1]));
        assert_eq!(
            graph.arena.entry(definitions[2]).visibility,
            Visibility::Private
        );
        assert!(!graph.arena.allows_external_access(definitions[2]));
    }
}
