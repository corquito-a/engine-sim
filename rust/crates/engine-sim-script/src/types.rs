//! The type check: `IrParserStructure::checkTypes` and its overrides.
//!
//! `resolveAll` runs this last, after expansion and the reference phase, and it
//! is where the two type errors come from:
//!
//! * `IrAttribute::_checkTypes` -- an argument whose reference resolves to a
//!   type that has no conversion to the port that consumes it (`T0010`);
//! * `IrAttributeDefinition::_checkTypes` -- a port's default value, checked
//!   the same way against the port's own type (`T0011`).
//!
//! Two entry points walk the tree, and both are needed:
//!
//! * `checkTypes()` (no argument) -- every structure in turn; a node re-checks
//!   its *definition* against a fresh main context, which is what makes a
//!   definition written for one node type get checked per instantiation. The
//!   per-structure `m_typesChecked` flag keeps that to once.
//! * `checkTypes(context)` -- every structure with the same context, which is
//!   how the unit's main context reaches a definition body.
//!
//! The three early exits are the interesting part, and they are copied
//! verbatim: a reference that only "touched the main context" is checked
//! against the unit's own context, a *static* type is only interesting in an
//! empty context, and a type that is already known to be right is not a
//! candidate for an error.

use crate::context::{instantiation_frames, ContextId};
use crate::error::{CompilationError, ErrorCode};
use crate::ir::Direction;
use crate::lower::{Arena, Kind, StructureId};
use crate::reference::{ReferenceQuery, ReferenceState};

impl Arena {
    /// `IrCompilationUnit::resolveAll`'s final step, for every unit.
    pub fn check_types_all(&mut self) {
        let mut errors = Vec::new();

        // The expansions are shared, as in every other phase: a reference that
        // was resolved during expansion is what the type check reads back.
        let mut state = std::mem::take(&mut self.expansion_state);

        for unit in self.units.clone() {
            self.check_types_no_args(unit, &mut state, &mut errors);
        }

        self.expansion_state = state;
        self.errors.extend(errors);
    }

    /// `IrParserStructure::checkTypes()` -- components first, then `_checkTypes()`
    /// without a context. Only units and nodes override the latter.
    fn check_types_no_args(
        &mut self,
        structure: StructureId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        if self.entry(structure).flags.types_checked {
            return;
        }

        self.entry_mut(structure).flags.types_checked = true;

        for child in self.components(structure) {
            self.check_types_no_args(child, state, errors);
        }

        match self.kind(structure).clone() {
            // `IrCompilationUnit::_checkTypes()`: one main context for the unit.
            Kind::Unit(_) => {
                let main = state.contexts.new_root(None, true);

                for child in self.components(structure) {
                    self.check_types_with_context(child, main, state, errors);
                }
            }

            // `IrNode::_checkTypes()`: the definition, focused on this node, with
            // a *main* child context of a fresh (non-main) root.
            Kind::Node(data) => {
                let parent = state.contexts.new_root(None, false);
                let main = state.contexts.main_child(parent, Some(structure));

                if let Some(definition) = data.definition {
                    self.check_types_with_context(definition, main, state, errors);
                }
            }

            _ => {}
        }
    }

    /// `IrParserStructure::checkTypes(IrContextTree *)`
    fn check_types_with_context(
        &mut self,
        structure: StructureId,
        context: ContextId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        for child in self.components(structure) {
            self.check_types_with_context(child, context, state, errors);
        }

        self.check_types_self(structure, context, state, errors);
    }

    /// `IrParserStructure::_checkTypes(IrContextTree *)` and its overrides.
    fn check_types_self(
        &mut self,
        structure: StructureId,
        context: ContextId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        match self.kind(structure).clone() {
            Kind::Attribute(_) => self.check_attribute_types(structure, context, state, errors),

            Kind::AttributeDefinition(_) => {
                self.check_port_default_types(structure, context, state, errors)
            }

            // `IrNode::_checkTypes(context)`: re-check the definition with this
            // node as the focus, unless this node is already being checked.
            Kind::Node(data) => {
                if state.contexts.find_context(context, structure).is_some() {
                    return;
                }

                let main = state.contexts.child(context, Some(structure));

                if let Some(definition) = data.definition {
                    self.check_types_with_context(definition, main, state, errors);
                }
            }

            _ => {}
        }
    }

    /// `IrAttribute::_checkTypes` -- `T0010`.
    fn check_attribute_types(
        &mut self,
        structure: StructureId,
        context: ContextId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        let Kind::Attribute(data) = self.kind(structure).clone() else {
            return;
        };

        let Some(definition) = data.definition else {
            return;
        };

        // `getReference(query, &info)` is called on the attribute itself, which
        // stands for its value.
        let query = ReferenceQuery {
            input_context: Some(context),
            record_errors: false,
            record_infinite_loops: false,
        };
        let (mut final_reference, info) = self.reference(structure, query, state);

        if info.failed {
            return;
        }

        if info.reached_dead_end {
            match info.fixed_type {
                Some(fixed) => final_reference = Some(fixed),
                None => return,
            }
        }

        let empty = state.contexts.is_empty(context);

        if !info.touched_main_context && !empty {
            return;
        }

        if info.is_static_type() && !empty {
            return;
        }

        let type_definition = self.effective_type_definition(definition);
        let Some(type_definition) = type_definition else {
            return;
        };

        let is_input = matches!(
            self.port_direction(definition),
            Some(Direction::Input | Direction::Modify | Direction::Toggle)
        );

        if !is_input {
            return;
        }

        if info.is_fixed_type() {
            if info.fixed_type == Some(type_definition) {
                return;
            }
        } else if let Some(node) = final_reference.and_then(|reference| self.as_node(reference)) {
            let Kind::Node(node_data) = self.kind(node) else {
                return;
            };
            let Some(node_data_definition) = node_data.definition else {
                return;
            };
            let definition = self.alias_type(node_data_definition);

            // `getAliasType()` cannot return null here, the port's own
            // `effective_type_definition` has already unwrapped the chain.
            if definition == type_definition {
                return;
            }
        }

        let actual_type = match info.fixed_type {
            Some(fixed) => self.definition_channel_type(fixed),
            None => final_reference.and_then(|reference| self.immediate_channel_type_of(reference)),
        };
        let expected_type = self.definition_channel_type(type_definition);

        if actual_type.is_some() && actual_type == expected_type {
            return;
        }

        if self.rules().check_conversion(actual_type, expected_type) {
            return;
        }

        errors.push(CompilationError::with_instantiation(
            self.entry(structure).summary.clone(),
            ErrorCode::IncompatibleType,
            instantiation_frames(&state.contexts, Some(context)),
        ));
    }

    /// `IrAttributeDefinition::_checkTypes` -- `T0011`. The error is reported at
    /// the *default value*'s token, not the port's.
    fn check_port_default_types(
        &mut self,
        structure: StructureId,
        context: ContextId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        let Kind::AttributeDefinition(data) = self.kind(structure).clone() else {
            return;
        };

        let Some(type_definition) = self.effective_type_definition(structure) else {
            return;
        };

        let Some(default_value) = data.default_value else {
            return;
        };

        let query = ReferenceQuery {
            input_context: Some(context),
            record_errors: false,
            record_infinite_loops: false,
        };
        let (mut default_reference, info) = self.reference(default_value, query, state);

        if info.failed {
            return;
        }

        if info.reached_dead_end {
            match info.fixed_type {
                Some(fixed) => default_reference = Some(fixed),
                None => return,
            }
        }

        let empty = state.contexts.is_empty(context);

        if !info.touched_main_context {
            if !empty {
                return;
            }

            if !state.contexts.is_main_context(context) {
                return;
            }
        }

        if info.is_static_type() && !empty {
            return;
        }

        if let Some(node) = default_reference.and_then(|reference| self.as_node(reference)) {
            let Kind::Node(node_data) = self.kind(node) else {
                return;
            };
            let Some(node_data_definition) = node_data.definition else {
                return;
            };

            if self.alias_type(node_data_definition) == type_definition {
                return;
            }
        }

        let actual_type = match info.fixed_type {
            Some(fixed) => self.definition_channel_type(fixed),
            None => default_reference.and_then(|reference| self.immediate_channel_type_of(reference)),
        };
        let expected_type = self.definition_channel_type(type_definition);

        if actual_type.is_some() && actual_type == expected_type {
            return;
        }

        if self.rules().check_conversion(actual_type, expected_type) {
            return;
        }

        // The original's `else if` for outputs is unreachable: the first branch
        // already covers every port direction that can have a default.
        errors.push(CompilationError::with_instantiation(
            self.entry(default_value).summary.clone(),
            ErrorCode::IncompatibleDefaultType,
            instantiation_frames(&state.contexts, Some(context)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_unit;
    use crate::Rules;

    /// The phases `resolveAll` runs, in order, for a script with no imports:
    /// the builtin types come from the piranha rules and the channel types are
    /// declared in the script.
    fn codes(source: &str) -> Vec<String> {
        let unit = parse_unit("test.mr", source).expect("parses");
        let mut arena = Arena::default();
        arena.set_rules(Rules::Piranha);
        arena.lower_unit(&unit);
        arena.link_imports(&[Vec::new()]);
        arena.resolve_all_definitions();
        arena.check_circular_definitions_all();
        arena.expand_all();
        arena.check_references_all();
        arena.check_types_all();
        arena
            .errors()
            .iter()
            .map(|error| format!("{}{}", error.code.stage, error.code.code))
            .collect()
    }

    /// The fixture this pair of tests distils (`general-tests/general_syntax_test_11.mr`
    /// in the original's suite). The `literal_int` definition is load-bearing:
    /// it is what gives an integer literal a channel type to compare, so without
    /// it neither check reports anything.
    const INT_LITERAL_TYPE: &str = "\
node string_channel => __piranha__string {}
node float_channel => __piranha__float {}
node int_channel => __piranha__int {}
public node literal_int => __piranha__literal_int {
    alias output __out [int_channel];
}
node string {
    input __in [string_channel];
    alias output __out [string_channel]: __in;
}
";

    #[test]
    fn a_default_that_does_not_convert_is_incompatible() {
        let source = format!("{INT_LITERAL_TYPE}node test {{ input a [string]: 5; }}\ntest()\n");
        assert_eq!(codes(&source), ["T0011"]);
    }

    #[test]
    fn an_argument_that_does_not_convert_is_incompatible() {
        let source = format!("{INT_LITERAL_TYPE}node test {{ input a [string]; }}\ntest(5)\n");
        assert_eq!(codes(&source), ["T0010"]);
    }

    #[test]
    fn an_argument_that_converts_is_accepted() {
        // A string literal against a string port: same channel type, no error.
        let source = format!("{INT_LITERAL_TYPE}node test {{ input a [string]; }}\ntest(\"hi\")\n");
        assert_eq!(codes(&source), Vec::<String>::new());
    }
}
