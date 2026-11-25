//! `expand` and `m_expansions`: the phase between circularity and references.
//!
//! Expansion replaces each *value* that stands for work with the node that does
//! it: a literal becomes a literal node, an operator becomes an operation node
//! whose inputs are internal references to its operands, and a port with no
//! value becomes an interface node. The product is `m_expansions`, a map from
//! `(structure, context)` to the node -- and until that map is populated, a
//! reference through a literal has nowhere to go.
//!
//! Two entry points matter, and they are not the same operation:
//!
//! * `expand()` -- no context. Memoised by a per-structure flag, it walks every
//!   component and then gives each node a *fresh main context* of its own, so
//!   the node's definition body is expanded as if instantiated at top level.
//! * `expand(context)` -- memoised by `(structure, context)`, it writes a null
//!   entry first (that is how a re-entry during expansion is detected), expands
//!   only the components whose `checkReferences` is set, and then expands
//!   itself.
//!
//! The unit's `_expand()` runs the second form over its components with a main
//! context, so a node is normally expanded through both paths. Both are
//! reproduced: the second is what makes `m_expansions` keyed per instantiation.
//!
//! `expandChain` walks a value's references and expands everything it reaches,
//! which is what lets an operator see the type of its operands before the
//! reference phase has run.

use crate::context::ContextId;
use crate::error::{CompilationError, ErrorCode};
use crate::ir::{Direction, Token};
use crate::language_rules::LiteralKind;
use crate::lower::{Arena, Kind, Literal, NodeData, StructureId};
use crate::reference::{ReferenceQuery, ReferenceState};

impl Arena {
    /// `IrCompilationUnit::resolveAll`'s expansion step, in the same unit order
    /// the definition resolver uses.
    pub fn expand_all(&mut self) {
        let mut state = std::mem::take(&mut self.expansion_state);

        for unit in self.units.clone().into_iter().rev() {
            self.expand_memo(unit, &mut state);
        }

        self.expansion_state = state;
    }

    /// `IrParserStructure::expand()` -- components first, then `_expand()`.
    fn expand_memo(&mut self, structure: StructureId, state: &mut ReferenceState) {
        if self.entry(structure).flags.expanded {
            return;
        }

        self.entry_mut(structure).flags.expanded = true;

        for child in self.components(structure) {
            self.expand_memo(child, state);
        }

        self.expand_self_no_context(structure, state);
    }

    /// `IrParserStructure::_expand()` -- only `IrNode` and `IrCompilationUnit`
    /// override it.
    fn expand_self_no_context(&mut self, structure: StructureId, state: &mut ReferenceState) {
        if matches!(self.kind(structure), Kind::Unit(_)) {
            // `IrCompilationUnit::_expand`: every component, against a main
            // context of this unit's own. This is the walk that expands an
            // instance's attributes -- and therefore its literals -- in the
            // unit context the reference chain looks them up in.
            let main = state.contexts.new_root(None, true);

            for child in self.components(structure) {
                self.expand_with_context(child, main, state);
            }

            return;
        }

        let Kind::Node(data) = self.kind(structure) else {
            return;
        };

        let definition = data.definition;
        let parent = state.contexts.new_root(None, false);
        let main = state.contexts.main_child(parent, Some(structure));

        if let Some(definition) = definition {
            self.expand_with_context(definition, main, state);
        }
    }

    /// `IrParserStructure::expand(IrContextTree *)`
    pub fn expand_with_context(
        &mut self,
        structure: StructureId,
        context: ContextId,
        state: &mut ReferenceState,
    ) {
        if state.expansion(structure, Some(context)).is_some() {
            return;
        }

        state.set_expansion(structure, Some(context), None);

        for child in self.components(structure) {
            if self.entry(child).check_references {
                self.expand_with_context(child, context, state);
            }
        }

        self.expand_self(structure, context, state);
    }

    /// `IrParserStructure::_expand(IrContextTree *)`
    fn expand_self(&mut self, structure: StructureId, context: ContextId, state: &mut ReferenceState) {
        match self.kind(structure).clone() {
            Kind::Node(data) => {
                // The loop guard: this node is already being expanded here.
                if state.contexts.find_context(context, structure).is_some() {
                    return;
                }

                let main = state.contexts.child(context, Some(structure));

                if let Some(definition) = data.definition {
                    self.expand_with_context(definition, main, state);
                }
            }

            Kind::AttributeDefinition(_) => self.expand_port(structure, context, state),
            Kind::Literal { literal, .. } => self.expand_literal(structure, &literal, context, state),
            Kind::Unary { .. } => self.expand_unary(structure, context, state),
            Kind::Binary { .. } => self.expand_binary(structure, context, state),
            _ => {}
        }
    }

    /// `IrValueLiteral::_expand` -- the literal becomes a literal node of the
    /// builtin type the rules pick for its type code.
    ///
    /// The builtin definition is looked up, never required: the original's two
    /// `nullptr` branches are comment-only TODOs and expansion continues
    /// either way. When the unit does not define `__piranha__literal_*` the
    /// node is left with no definition *and no type token* (`IrLiteralNode`
    /// never sets `m_type`, only `setName`), so its own
    /// `resolveDefinitions()` reports `UndefinedNodeType` against a
    /// default-constructed token -- `R0010` on the unspecified line. That is
    /// the whole of the port's unspecified-token error population.
    fn expand_literal(
        &mut self,
        structure: StructureId,
        literal: &Literal,
        context: ContextId,
        state: &mut ReferenceState,
    ) {
        // "TODO: raise error, literal not supported" -- the original keeps the
        // empty name and carries on.
        let builtin = self.rules().literal_builtin(literal_kind(literal)).unwrap_or("");

        let Some(unit) = self.entry(structure).parent_unit else {
            return;
        };

        // "TODO: raise error, literal type not defined or not available"
        let (definition, _count) = self.resolve_builtin_node_definition(unit, builtin);

        let attributes = self.new_structure(structure, Kind::AttributeList(Vec::new()));

        let mut data = NodeData::new(
            Token::unspecified(),
            Token::unspecified(),
            Token::new(&format!("\"{}\"", literal_text(literal))),
            attributes,
        );
        data.definition = definition;

        let expansion = self.new_structure(structure, Kind::Node(data));

        self.resolve_definitions(expansion);
        self.expand_with_context(expansion, context, state);

        state.set_expansion(structure, Some(context), Some(expansion));
    }

    /// `IrAttributeDefinition::_expand`
    fn expand_port(&mut self, structure: StructureId, context: ContextId, state: &mut ReferenceState) {
        let Kind::AttributeDefinition(data) = self.kind(structure).clone() else {
            return;
        };

        let Some(type_definition) = self.effective_type_definition(structure) else {
            return;
        };

        // `IrAttributeDefinition::_expand`: this branch calls `setInterface(true)`
        // and leaves `m_build` alone, so the interface node it records can be
        // built. Only the dead-end branch below clears the flag.
        if data.direction == Direction::Output && data.default_value.is_none() {
            let expansion = self.new_interface_node(structure, Some(type_definition), true, true);
            self.expand_with_context(expansion, context, state);
            state.set_expansion(structure, Some(context), Some(expansion));
            return;
        }

        let immediate_query = ReferenceQuery::at(Some(context));
        let immediate_query = ReferenceQuery { record_errors: false, ..immediate_query };
        let (immediate_reference, immediate_info) =
            self.immediate_reference(structure, &immediate_query, state);

        let _ = immediate_reference;

        if immediate_info.failed {
            return;
        }

        if immediate_info.reached_dead_end {
            let expansion = self.new_interface_node(structure, Some(type_definition), true, false);
            self.expand_with_context(expansion, context, state);
            state.set_expansion(structure, Some(context), Some(expansion));
            return;
        }

        let Some(immediate_reference) = immediate_reference else {
            return;
        };

        self.expand_chain(immediate_reference, immediate_info.new_context, state);

        let reference_query = ReferenceQuery {
            input_context: immediate_info.new_context,
            record_errors: false,
            record_infinite_loops: false,
        };
        let (reference, reference_info) =
            self.reference(immediate_reference, reference_query, state);

        if reference_info.failed {
            return;
        }

        let (reference, fixed_type) = if reference_info.reached_dead_end {
            // A dead end is fine as long as the fixed type is known -- and the
            // fixed type then *is* the reference, as in the original.
            match reference_info.fixed_type {
                Some(fixed) => (Some(fixed), Some(fixed)),
                None => return,
            }
        } else {
            (reference, None)
        };

        let fixed_type_definition = fixed_type.or(reference_info.fixed_type);

        let reference_type = match fixed_type_definition {
            Some(fixed) => self.definition_channel_type(fixed),
            None => match reference {
                Some(reference) => self.immediate_channel_type_of(reference),
                None => return,
            },
        };

        let expected_type = self.definition_channel_type(type_definition);

        match (reference_type, expected_type) {
            (Some(actual), Some(expected)) if actual == expected => return,
            (Some(actual), Some(expected)) => {
                let Some(builtin) = self.rules().conversion_builtin(Some(actual), Some(expected))
                else {
                    return;
                };

                let Some(unit) = self.expansion_unit(context, structure, state) else {
                    return;
                };

                let (definition, _count) = self.resolve_builtin_node_definition(unit, builtin);
                let Some(reference) = reference else { return };

                let internal = self.new_structure(
                    structure,
                    Kind::InternalReference {
                        structure: reference,
                        context: reference_info.new_context,
                    },
                );

                let attribute = self.new_structure(
                    structure,
                    Kind::Attribute(crate::lower::AttributeData {
                        name: Token::unspecified(),
                        value: internal,
                        definition: None,
                    }),
                );

                let attributes = self.new_structure(structure, Kind::AttributeList(vec![attribute]));

                let mut node = NodeData::new(
                    Token::unspecified(),
                    Token::unspecified(),
                    Token::unspecified(),
                    attributes,
                );
                node.definition = definition;

                let expansion = self.new_structure(structure, Kind::Node(node));
                self.expand_with_context(expansion, context, state);
                self.resolve_definitions(expansion);

                state.set_expansion(structure, Some(context), Some(expansion));
            }
            _ => {
                // A type mismatch of this shape was already reported.
            }
        }
    }

    /// `The unit a new expansion belongs to`: the context's node when there is
    /// one, otherwise the structure's own unit.
    fn expansion_unit(
        &self,
        context: ContextId,
        structure: StructureId,
        state: &ReferenceState,
    ) -> Option<StructureId> {
        match state.contexts.context(context) {
            Some(node) => self.entry(node).parent_unit,
            None => self.entry(structure).parent_unit,
        }
    }

    /// `IrUnaryOperator::_expand`
    fn expand_unary(&mut self, structure: StructureId, context: ContextId, state: &mut ReferenceState) {
        let Kind::Unary { op, operand, .. } = self.kind(structure).clone() else {
            return;
        };

        let empty_context = state.contexts.is_empty(context);

        self.expand_chain(operand, Some(context), state);

        let query = ReferenceQuery { input_context: Some(context), ..Default::default() };
        let (reference, info) = self.reference(operand, query, state);

        if info.failed {
            return;
        }

        let reference = if info.reached_dead_end {
            match info.fixed_type {
                Some(fixed) => Some(fixed),
                None => return,
            }
        } else {
            reference
        };

        let type_name = match info.fixed_type {
            Some(fixed) => self.definition_channel_type(fixed),
            None => match reference {
                Some(reference) => self.immediate_channel_type_of(reference),
                None => return,
            },
        };

        let builtin = self.rules().unary_builtin(op, type_name);

        let Some(builtin) = builtin else {
            let touched_main_context = info.touched_main_context && !info.is_fixed_type();

            if touched_main_context || empty_context {
                let location = self.entry(structure).summary.clone();
                self.errors.push(CompilationError::with_instantiation(
                    location,
                    ErrorCode::InvalidOperandTypes,
                    crate::context::instantiation_frames(&state.contexts, Some(context)),
                ));
            }

            return;
        };

        let Some(unit) = self.entry(structure).parent_unit else {
            return;
        };

        let (definition, _count) = self.resolve_builtin_node_definition(unit, builtin);

        let internal = self.new_structure(
            structure,
            Kind::InternalReference { structure: operand, context: Some(context) },
        );

        let attribute = self.new_structure(
            structure,
            Kind::Attribute(crate::lower::AttributeData {
                name: Token::unspecified(),
                value: internal,
                definition: None,
            }),
        );

        let attributes = self.new_structure(structure, Kind::AttributeList(vec![attribute]));

        let mut node =
            NodeData::new(Token::unspecified(), Token::unspecified(), Token::unspecified(), attributes);
        node.definition = definition;

        let expansion = self.new_structure(structure, Kind::Node(node));
        self.expand_with_context(expansion, context, state);
        self.resolve_definitions(expansion);

        state.set_expansion(structure, Some(context), Some(expansion));
    }

    /// `IrBinaryOperator::_expand`
    fn expand_binary(&mut self, structure: StructureId, context: ContextId, state: &mut ReferenceState) {
        let Kind::Binary { op, left, right, .. } = self.kind(structure).clone() else {
            return;
        };

        // `.` is the reference operator: it only has to expand its left side.
        if op == crate::ir::BinOp::Dot {
            self.expand_chain(left, Some(context), state);
            return;
        }

        let empty_context = state.contexts.is_empty(context);

        self.expand_chain(left, Some(context), state);

        let query = ReferenceQuery { input_context: Some(context), ..Default::default() };
        let (left_reference, left_info) = self.reference(left, query, state);

        if left_info.failed {
            return;
        }

        let left_reference = if left_info.reached_dead_end {
            match left_info.fixed_type {
                Some(fixed) => Some(fixed),
                None => {
                    return;
                }
            }
        } else {
            left_reference
        };

        let right_query = ReferenceQuery { input_context: Some(context), ..Default::default() };
        self.expand_chain(right, Some(context), state);
        let (right_reference, right_info) = self.reference(right, right_query, state);

        if right_info.failed {
            return;
        }

        let right_reference = if right_info.reached_dead_end {
            match right_info.fixed_type {
                Some(fixed) => Some(fixed),
                None => {
                    return;
                }
            }
        } else {
            right_reference
        };

        let left_type = match left_info.fixed_type {
            Some(fixed) => self.definition_channel_type(fixed),
            None => left_reference.and_then(|reference| self.immediate_channel_type_of(reference)),
        };

        let right_type = match right_info.fixed_type {
            Some(fixed) => self.definition_channel_type(fixed),
            None => right_reference.and_then(|reference| self.immediate_channel_type_of(reference)),
        };

        let builtin = self.rules().operator_builtin(op, left_type, right_type);

        let Some(builtin) = builtin else {
            // `isStaticType()`, not `isFixedType()`: a fixed type that had to be
            // *resolved* still counts. The unary operator uses `isFixedType()`
            // here -- the asymmetry is the original's, so it is kept.
            let touched_main_context = (left_info.touched_main_context && !left_info.is_static_type())
                || (right_info.touched_main_context && !right_info.is_static_type());

            if touched_main_context || empty_context {
                let location = self.entry(structure).summary.clone();
                self.errors.push(CompilationError::with_instantiation(
                    location,
                    ErrorCode::InvalidOperandTypes,
                    crate::context::instantiation_frames(&state.contexts, Some(context)),
                ));
            }

            return;
        };

        let Some(unit) = self.entry(structure).parent_unit else {
            return;
        };

        let (definition, _count) = self.resolve_builtin_node_definition(unit, builtin);

        let mut attributes = Vec::new();
        for operand in [left, right] {
            let internal = self.new_structure(
                structure,
                Kind::InternalReference { structure: operand, context: Some(context) },
            );

            attributes.push(self.new_structure(
                structure,
                Kind::Attribute(crate::lower::AttributeData {
                    name: Token::unspecified(),
                    value: internal,
                    definition: None,
                }),
            ));
        }

        let attributes = self.new_structure(structure, Kind::AttributeList(attributes));

        let mut node =
            NodeData::new(Token::unspecified(), Token::unspecified(), Token::unspecified(), attributes);
        node.definition = definition;

        let expansion = self.new_structure(structure, Kind::Node(node));
        self.resolve_definitions(expansion);
        self.expand_with_context(expansion, context, state);

        state.set_expansion(structure, Some(context), Some(expansion));
    }

    /// `IrParserStructure::expandChain` -- expand this structure and everything
    /// its immediate reference reaches, stopping when a link repeats.
    fn expand_chain(
        &mut self,
        structure: StructureId,
        context: Option<ContextId>,
        state: &mut ReferenceState,
    ) {
        let mut chain = Vec::new();
        self.expand_chain_with(structure, context, &mut chain, state);
    }

    fn expand_chain_with(
        &mut self,
        structure: StructureId,
        context: Option<ContextId>,
        chain: &mut Vec<(StructureId, Option<ContextId>)>,
        state: &mut ReferenceState,
    ) {
        let repeats = chain.iter().any(|(candidate, candidate_context)| {
            *candidate == structure && state.contexts.same(*candidate_context, context)
        });

        if repeats {
            return;
        }

        chain.push((structure, context));

        if let Some(context) = context {
            self.expand_with_context(structure, context, state);
        }

        let query = ReferenceQuery::at(context);
        let (immediate, info) = self.immediate_reference(structure, &query, state);

        if let Some(immediate) = immediate {
            self.expand_chain_with(immediate, info.new_context, chain, state);
        }
    }

    /// `IrParserStructure::getImmediateChannelType` for the structures that
    /// have one: a node reports its definition's channel type, a port the type
    /// of the definition it declared.
    pub fn immediate_channel_type_of(&self, structure: StructureId) -> Option<&'static str> {
        match self.kind(structure) {
            Kind::Node(data) => data.definition.and_then(|definition| self.definition_channel_type(definition)),
            Kind::AttributeDefinition(_) => self.immediate_channel_type(structure),
            _ => None,
        }
    }

    /// `IrNodeDefinition::getChannelType`
    pub fn definition_channel_type(&self, definition: StructureId) -> Option<&'static str> {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return None;
        };

        if data.defines_builtin {
            if let Some(channel_type) = self.rules().channel_type(&data.builtin_name.data) {
                return Some(channel_type);
            }
        }

        let alias = self.alias_type(definition);
        if alias != definition {
            self.definition_channel_type(alias)
        } else {
            None
        }
    }

    /// `IrCompilationUnit::resolveBuiltinNodeDefinition` -- definitions that
    /// stand for a builtin, locally first and then through the imports.
    pub fn resolve_builtin_node_definition(
        &self,
        unit: StructureId,
        builtin_name: &str,
    ) -> (Option<StructureId>, i32) {
        self.resolve_builtin_node_definition_external(unit, builtin_name, false)
    }

    fn resolve_builtin_node_definition_external(
        &self,
        unit: StructureId,
        builtin_name: &str,
        external: bool,
    ) -> (Option<StructureId>, i32) {
        let Kind::Unit(data) = self.kind(unit) else {
            return (None, 0);
        };

        let mut count = 0;
        for definition in &data.definitions {
            if external && !self.allows_external_access(*definition) {
                continue;
            }

            let Kind::NodeDefinition(definition_data) = self.kind(*definition) else {
                continue;
            };

            if !definition_data.defines_builtin {
                continue;
            }

            if definition_data.builtin_name.data == builtin_name {
                count += 1;
                return (Some(*definition), count);
            }
        }

        let mut first = None;
        for import in &data.imports {
            let Kind::Import(import_data) = self.kind(*import) else {
                continue;
            };

            let Some(target) = import_data.unit else {
                continue;
            };

            if external && !self.allows_external_access(*import) {
                continue;
            }

            let (definition, secondary) =
                self.resolve_builtin_node_definition_external(target, builtin_name, true);

            if definition.is_some() {
                count += secondary;
                if first.is_none() {
                    first = definition;
                }
            }
        }

        (first, count)
    }

    /// A synthetic `IrNode` for an expansion: no tokens of its own, the given
    /// definition, and parented like `setLogicalParent`/`setScopeParent`.
    fn new_interface_node(
        &mut self,
        parent: StructureId,
        definition: Option<StructureId>,
        is_interface: bool,
        build: bool,
    ) -> StructureId {
        let attributes = self.new_structure(parent, Kind::AttributeList(Vec::new()));

        let mut data =
            NodeData::new(Token::unspecified(), Token::unspecified(), Token::unspecified(), attributes);
        data.definition = definition;
        data.is_interface = is_interface;
        data.build = build;

        self.new_structure(parent, Kind::Node(data))
    }

    /// Push a structure that `expand` invents, parented like
    /// `setLogicalParent`/`setScopeParent` and without registering a token, so
    /// its summary stays the "no position" span.
    fn new_structure(&mut self, parent: StructureId, kind: Kind) -> StructureId {
        let mut entry = crate::lower::Entry::synthetic(parent, self.unit_of(parent));
        entry.kind = kind;
        self.push_entry(entry)
    }

}

fn literal_kind(literal: &Literal) -> LiteralKind {
    match literal {
        Literal::Int(_) => LiteralKind::Integer,
        Literal::Float(_) => LiteralKind::Float,
        Literal::Bool(_) => LiteralKind::Boolean,
        Literal::Str(_) => LiteralKind::String,
    }
}

fn literal_text(literal: &Literal) -> String {
    match literal {
        Literal::Int(value) => value.to_string(),
        Literal::Float(value) => cxx_float_text(*value),
        Literal::Bool(value) => if *value { "1".to_string() } else { "0".to_string() },
        Literal::Str(value) => value.clone(),
    }
}

/// `std::ostream`'s default float formatting, which is what
/// `IrValueConstant::valueToString` gives a literal node its name with:
/// `%g` at precision 6 -- six significant digits, fixed notation while the
/// exponent stays in `[-4, 6)`, exponent trimmed of trailing zeros, and a
/// two-digit signed exponent otherwise.
///
/// Rust's `Display` for `f64` is the shortest round-trip instead, so
/// `0.45359237_f64 * 1.0` names a node `"0.45359237"` where the original names
/// it `"0.453592"`; the graph gate sees that as a different node.
fn cxx_float_text(value: f64) -> String {
    const PRECISION: i32 = 6;

    if value == 0.0 {
        return if value.is_sign_negative() { "-0".to_string() } else { "0".to_string() };
    }

    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let negative = mantissa.starts_with('-');

    let body = if exponent < -4 || exponent >= PRECISION {
        // `%e` with one fewer fraction digit than the precision, exponent
        // rebuilt the way the C++ stream spells it.
        let decimals = (PRECISION - 1) as usize;
        let formatted = format!("{:.*e}", decimals, value.abs());
        let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
        let mantissa = trim_fraction(mantissa);
        let exponent: i32 = exponent.parse().unwrap_or(0);
        let sign = if exponent < 0 { '-' } else { '+' };

        format!("{mantissa}e{sign}{:02}", exponent.abs())
    }
    else {
        // Fixed with `PRECISION - 1 - exponent` fraction digits, then trimmed.
        let decimals = (PRECISION - 1 - exponent).max(0) as usize;
        trim_fraction(&format!("{:.*}", decimals, value.abs()))
    };

    if negative {
        format!("-{body}")
    }
    else {
        body
    }
}

/// Strip the trailing zeros `%g` drops, and a dot left with nothing after it.
fn trim_fraction(text: &str) -> String {
    if !text.contains('.') {
        return text.to_string();
    }

    let trimmed = text.trim_end_matches('0');
    trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
}

#[cfg(test)]
mod float_text_tests {
    use super::cxx_float_text;

    /// The names the reference gave the `units` literals.
    #[test]
    fn matches_the_default_ostream_formatting() {
        assert_eq!(cxx_float_text(0.45359237), "0.453592");
        assert_eq!(cxx_float_text(1609.344), "1609.34");
        assert_eq!(cxx_float_text(3.785411784), "3.78541");
        assert_eq!(cxx_float_text(0.104719755), "0.10472");
        assert_eq!(cxx_float_text(4.44822), "4.44822");
        assert_eq!(cxx_float_text(1.5), "1.5");
        assert_eq!(cxx_float_text(1.0), "1");
        assert_eq!(cxx_float_text(1000.0), "1000");
        assert_eq!(cxx_float_text(273.15), "273.15");
        assert_eq!(cxx_float_text(-1.5), "-1.5");
        assert_eq!(cxx_float_text(0.0), "0");
        assert_eq!(cxx_float_text(1.0e7), "1e+07");
        assert_eq!(cxx_float_text(1.23456789e-7), "1.23457e-07");
    }
}
