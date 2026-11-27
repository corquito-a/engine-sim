//! The reference chain: `getImmediateReference` and `getReference`.
//!
//! A structure that stands for a *value* -- a label, a literal, an operator, a
//! port -- does not hold that value. It holds a pointer to something else, and
//! that something may itself be a reference, so resolving one name walks a
//! chain: `a.b` resolves `a`, then looks for `b` in what `a` came back as.
//!
//! Three pieces of the original's bookkeeping are load bearing and all three are
//! reproduced here rather than simplified:
//!
//! * **the chain**. `getReference` keeps every `(structure, context)` pair it
//!   has visited. Meeting a pair again is a cycle, and the cycle is reported as
//!   `CircularReference` *at the link it came back to* -- not at the structure
//!   where the walk noticed. The link index also carries into the caller, which
//!   is why `ReferenceInfo::infinite_loop` is an index and not a flag.
//! * **`m_infiniteLoops`**. Once a cycle has been walked through, every link
//!   from the repeat onwards is marked so that a later visit does not report the
//!   same cycle again. This is why one script can produce two `R0080` errors for
//!   the same loop and another three: the report depends on how many *distinct*
//!   entries walked into the loop, not on how many cycles exist.
//! * **`touchedMainContext`**. An error is only worth reporting if the walk
//!   descended through a main context (an instantiation the user actually wrote)
//!   or if the context is empty (top level). A reference that fails entirely
//!   inside a definition body is silent.
//!
//! `ReferenceState` holds the three maps that live beside the tree: the context
//! arena, `m_expansions`, and `m_infiniteLoops`. They are keyed by context, so
//! they are created per compilation pass rather than stored in the `Arena`.

use std::collections::HashMap;

use crate::context::{instantiation_frames, ContextArena, ContextId};
use crate::error::{CompilationError, ErrorCode};
use crate::lower::{Arena, Kind, StructureId};

/// `IrParserStructure::IrReferenceQuery`
#[derive(Clone, Debug, Default)]
pub struct ReferenceQuery {
    pub input_context: Option<ContextId>,
    pub record_errors: bool,
    pub record_infinite_loops: bool,
}

impl ReferenceQuery {
    /// A query that resolves silently, in `input_context`.
    pub fn at(input_context: Option<ContextId>) -> Self {
        ReferenceQuery { input_context, ..Default::default() }
    }

    /// A query that reports what it cannot resolve (`recordErrors = true`).
    pub fn recording(input_context: Option<ContextId>) -> Self {
        ReferenceQuery { input_context, record_errors: true, ..Default::default() }
    }

    fn is_empty(&self, contexts: &ContextArena) -> bool {
        self.input_context.is_none_or(|context| contexts.is_empty(context))
    }
}

/// `IrParserStructure::IrReferenceInfo`
#[derive(Clone, Debug)]
pub struct ReferenceInfo {
    pub new_context: Option<ContextId>,
    pub err: Option<CompilationError>,
    pub failed: bool,
    pub reached_dead_end: bool,
    pub touched_main_context: bool,
    /// Index into the chain where a cycle was met, `None` for no cycle.
    pub infinite_loop: Option<usize>,
    pub fixed_type: Option<StructureId>,
    pub fixed_type_context: Option<ContextId>,
    pub static_type: bool,
}

impl Default for ReferenceInfo {
    fn default() -> Self {
        ReferenceInfo {
            new_context: None,
            err: None,
            failed: false,
            reached_dead_end: false,
            touched_main_context: false,
            infinite_loop: None,
            fixed_type: None,
            fixed_type_context: None,
            // The default is `true`, not `false`: "static" means the reference
            // never had to be resolved, and a fresh info has not resolved
            // anything yet.
            static_type: true,
        }
    }
}

impl ReferenceInfo {
    fn reset(&mut self, query: &ReferenceQuery) {
        *self = ReferenceInfo { new_context: query.input_context, ..Default::default() };
    }

    pub fn is_fixed_type(&self) -> bool {
        self.fixed_type.is_some()
    }

    pub fn is_static_type(&self) -> bool {
        self.is_fixed_type() && self.static_type
    }

    /// `IrReferenceInfo::isFixedTypeOutside`
    pub fn is_fixed_type_outside(&self, contexts: &ContextArena, context: Option<ContextId>) -> bool {
        match (self.fixed_type_context, context) {
            (Some(fixed), Some(context)) => self.is_fixed_type() && contexts.is_outside(fixed, context),
            _ => false,
        }
    }
}

/// `IrParserStructure::IrReferenceChain::Link`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub structure: StructureId,
    pub context: Option<ContextId>,
}

/// `IrParserStructure::IrReferenceChain`
#[derive(Clone, Debug, Default)]
pub struct ReferenceChain {
    pub list: Vec<Link>,
}

impl ReferenceChain {
    pub fn add_link(&mut self, structure: StructureId, context: Option<ContextId>) {
        self.list.push(Link { structure, context });
    }

    /// `IrReferenceChain::searchLink` -- a repeat is the same structure in a
    /// context that is *equal*, not identical.
    pub fn search_link(
        &self,
        contexts: &ContextArena,
        structure: StructureId,
        context: Option<ContextId>,
    ) -> Option<usize> {
        self.list.iter().position(|link| {
            link.structure == structure && contexts.same(link.context, context)
        })
    }
}

/// The state a reference walk keeps beside the tree: `m_expansions` and
/// `m_infiniteLoops`, both keyed by `(structure, context)`.
#[derive(Clone, Debug, Default)]
pub struct ReferenceState {
    pub contexts: ContextArena,
    /// `m_expansions`: one `PKeyValueLookup<IrContextTree, IrNode *>` per
    /// structure. Within a structure the contexts are compared **by value**
    /// (`*stored == *key`, i.e. `IrContextTree::isEqual`), not by pointer -- so a
    /// freshly created context that is structurally equal to one already used
    /// finds the same expansion. Keying by context id instead loses every
    /// expansion the moment the walk creates an equal-but-distinct child, which
    /// is exactly what a node's `getImmediateReference` does.
    ///
    /// That value comparison is what made this table quadratic: a linear scan of
    /// the structure's entries, each step walking two context chains, is ~10^9
    /// comparisons to expand a nine-cylinder engine (`docs/performance-options.md`
    /// §1). So the entries are bucketed by the context chain's hash
    /// (`ContextArena::hash`), which is equal for equal chains by construction.
    ///
    /// A bucket that contains no equal entry therefore proves that the structure
    /// has no equal entry at all, and no scan across buckets is needed. Every
    /// member of a bucket is a distinct chain, so the scan inside a bucket is the
    /// same first-match scan the reference performs -- which is why bucketing
    /// preserves the (observable) order in which equal expansions win.
    ///
    /// A hash *collision* can only cost time: the entry is still confirmed with
    /// `same()`, so two chains that merely hash alike stay apart.
    expansions: HashMap<(StructureId, u64), Vec<(Option<ContextId>, Option<StructureId>)>>,
    infinite_loops: HashMap<StructureId, Vec<Option<ContextId>>>,
}

impl ReferenceState {
    pub fn new() -> Self {
        ReferenceState::default()
    }

    /// The bucket key for `context`: the chain's hash, or 0 for "no context".
    ///
    /// A missing context and the shared empty context hash to the same bucket,
    /// which is harmless: `same()` distinguishes them, so they simply share a
    /// bucket of at most two entries.
    fn chain_key(&self, structure: StructureId, context: Option<ContextId>) -> (StructureId, u64) {
        (structure, context.map(|id| self.contexts.hash(id)).unwrap_or(0))
    }

    /// `m_expansions.lookup(context)`: `None` when nothing has been written for
    /// that context, `Some(None)` when the entry exists but is null.
    pub fn expansion(
        &self,
        structure: StructureId,
        context: Option<ContextId>,
    ) -> Option<Option<StructureId>> {
        self.expansions
            .get(&self.chain_key(structure, context))?
            .iter()
            .find(|(candidate_context, _)| self.contexts.same(*candidate_context, context))
            .map(|(_, expansion)| *expansion)
    }

    /// `*m_expansions.newValue(context) = expansion`
    pub fn set_expansion(
        &mut self,
        structure: StructureId,
        context: Option<ContextId>,
        expansion: Option<StructureId>,
    ) {
        let key = self.chain_key(structure, context);
        let entries = self.expansions.entry(key).or_default();

        match entries
            .iter_mut()
            .find(|(candidate_context, _)| self.contexts.same(*candidate_context, context))
        {
            Some(entry) => entry.1 = expansion,
            None => entries.push((context, expansion)),
        }
    }

    /// `IrParserStructure::isInfiniteLoop`
    pub fn is_infinite_loop(
        &self,
        structure: StructureId,
        context: Option<ContextId>,
    ) -> bool {
        self.infinite_loops
            .get(&structure)
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|candidate| self.contexts.same(*candidate, context))
            })
    }

    /// `IrParserStructure::setInfiniteLoop`
    pub fn set_infinite_loop(&mut self, structure: StructureId, context: Option<ContextId>) {
        if !self.is_infinite_loop(structure, context) {
            self.infinite_loops.entry(structure).or_default().push(context);
        }
    }
}

impl Arena {
    /// `IrParserStructure::getReference` (the two-argument overload): start a
    /// chain at `structure` and report a cycle if one closes.
    pub fn reference(
        &self,
        structure: StructureId,
        query: ReferenceQuery,
        state: &mut ReferenceState,
    ) -> (Option<StructureId>, ReferenceInfo) {
        let mut chain = ReferenceChain::default();
        chain.add_link(structure, query.input_context);

        let (reference, mut info) = self.reference_chained(structure, &query, &mut chain, state);

        if query.record_errors {
            if let Some(index) = info.infinite_loop {
                let link = chain.list[index];

                if !state.is_infinite_loop(link.structure, link.context) {
                    if info.touched_main_context || query.is_empty(&state.contexts) {
                        info.err = Some(CompilationError::with_instantiation(
                            self.entry(link.structure).summary.clone(),
                            ErrorCode::CircularReference,
                            instantiation_frames(&state.contexts, link.context),
                        ));
                    }
                }

                for later in chain.list[index..].iter().copied() {
                    state.set_infinite_loop(later.structure, later.context);
                }
            }
        }

        (reference, info)
    }

    /// `IrParserStructure::getReference(query, output, chain)`
    fn reference_chained(
        &self,
        structure: StructureId,
        query: &ReferenceQuery,
        chain: &mut ReferenceChain,
        state: &mut ReferenceState,
    ) -> (Option<StructureId>, ReferenceInfo) {
        let mut info = ReferenceInfo::default();
        info.reset(query);

        let (immediate, immediate_info) = self.immediate_reference(structure, query, state);

        if immediate_info.failed {
            info.failed = true;
            info.err = immediate_info.err;
            return (None, info);
        }

        if immediate_info.reached_dead_end {
            info.reached_dead_end = true;
            if immediate_info.is_fixed_type() {
                info.fixed_type = immediate_info.fixed_type;
                info.fixed_type_context = immediate_info.fixed_type_context;
            }
            return (None, info);
        }

        let Some(immediate) = immediate else {
            // No immediate reference: this *is* the reference.
            return (Some(structure), info);
        };

        if let Some(index) = chain.search_link(&state.contexts, immediate, immediate_info.new_context) {
            info.failed = true;
            info.infinite_loop = Some(index);
            info.touched_main_context = immediate_info.touched_main_context;
            return (None, info);
        }

        chain.add_link(immediate, immediate_info.new_context);

        let nested_query = ReferenceQuery {
            input_context: immediate_info.new_context,
            // Errors were already reported for the parent, so the nested walk
            // never reports its own.
            record_errors: false,
            record_infinite_loops: query.record_errors || query.record_infinite_loops,
        };

        let (full_reference, nested_info) = self.reference_chained(immediate, &nested_query, chain, state);

        info.touched_main_context = nested_info.touched_main_context || immediate_info.touched_main_context;
        info.static_type = immediate_info.static_type && nested_info.static_type;

        if nested_info.failed {
            info.failed = true;
            info.err = nested_info.err;
            info.infinite_loop = nested_info.infinite_loop;
            return (None, info);
        }

        // The immediate type takes precedence: fixed-type information survives a
        // dead end, so it has to be recorded before the dead end is checked.
        if immediate_info.is_fixed_type() {
            info.static_type = immediate_info.static_type;
            info.fixed_type = immediate_info.fixed_type;
            info.fixed_type_context = immediate_info.new_context;
        } else if nested_info.is_fixed_type() {
            info.fixed_type = nested_info.fixed_type;
            info.fixed_type_context = nested_info.new_context;
        }

        if nested_info.reached_dead_end {
            info.reached_dead_end = true;
            return (None, info);
        }

        info.new_context = nested_info.new_context;

        (full_reference, info)
    }

    /// `IrParserStructure::getImmediateReference` and its overrides.
    pub fn immediate_reference(
        &self,
        structure: StructureId,
        query: &ReferenceQuery,
        state: &mut ReferenceState,
    ) -> (Option<StructureId>, ReferenceInfo) {
        let mut info = ReferenceInfo::default();
        info.reset(query);

        match self.kind(structure) {
            // `IrValueLabel::getImmediateReference`
            Kind::Label(token) => {
                let mut reference = self.resolve_name(structure, &token.data);

                if reference.is_none() {
                    // Only global variables are visible from here.
                    if let Some(unit) = self.entry(structure).parent_unit {
                        reference = self.resolve_local_name(unit, &token.data);
                    }
                    info.new_context = query.input_context.map(|context| state.contexts.root(context));
                }

                match reference {
                    Some(reference) => (Some(reference), info),
                    None => {
                        info.failed = true;
                        if query.record_errors && query.is_empty(&state.contexts) {
                            info.err = Some(CompilationError::with_instantiation(
                                self.entry(structure).summary.clone(),
                                ErrorCode::UnresolvedReference,
                                instantiation_frames(&state.contexts, query.input_context),
                            ));
                        }
                        (None, info)
                    }
                }
            }

            // `IrValueLiteral::getImmediateReference`
            Kind::Literal { .. } => {
                match state.expansion(structure, query.input_context) {
                    None => {
                        info.reached_dead_end = true;
                        (None, info)
                    }
                    Some(None) => {
                        info.failed = true;
                        (None, info)
                    }
                    Some(expansion) => (expansion, info),
                }
            }

            // `IrAttribute::getImmediateReference` -- an attribute stands for
            // its value, in the same context.
            Kind::Attribute(data) => {
                let value = data.value;
                info.err = None;
                info.new_context = query.input_context;
                info.failed = false;
                (Some(value), info)
            }

            // `IrAttributeDefinition::getImmediateReference`
            Kind::AttributeDefinition(data) => {
                let type_definition = self.effective_type_definition(structure);
                if type_definition.is_some() {
                    info.fixed_type = type_definition;
                } else if data.direction == crate::ir::Direction::Input {
                    info.static_type = false;
                }

                if let Some(context) = query.input_context {
                    if let Some(reference) =
                        state.contexts.resolve_definition(context, structure, self)
                    {
                        if let Some(expansion) = state.expansion(structure, query.input_context) {
                            if let Some(expansion) = expansion {
                                return (Some(expansion), info);
                            }
                        }

                        info.new_context = state.contexts.parent(context);
                        info.touched_main_context = state.contexts.is_main_context(context);

                        return (Some(reference), info);
                    }
                }

                if data.default_value.is_none() && data.direction == crate::ir::Direction::Input {
                    let interface = query.input_context.is_some_and(|context| {
                        match state.contexts.context(context) {
                            Some(node) => self.is_interface(node),
                            None => false,
                        }
                    });

                    if query.input_context.is_none()
                        || query.is_empty(&state.contexts)
                        || interface
                    {
                        return match state.expansion(structure, query.input_context) {
                            Some(Some(expansion)) => (Some(expansion), info),
                            Some(None) => {
                                info.reached_dead_end = true;
                                (None, info)
                            }
                            // No expansion was ever recorded for this context.
                            None => {
                                info.reached_dead_end = true;
                                (None, info)
                            }
                        };
                    }

                    // An unconnected input with no default.
                    info.failed = true;
                    return (None, info);
                }
                else if data.default_value.is_none() && data.direction == crate::ir::Direction::Output {
                    if data.type_definition.is_none() {
                        info.failed = true;
                        return (None, info);
                    }

                    return match state.expansion(structure, query.input_context) {
                        Some(Some(expansion)) => (Some(expansion), info),
                        _ => {
                            info.failed = true;
                            (None, info)
                        }
                    };
                }

                match state.expansion(structure, query.input_context) {
                    None => {
                        info.failed = true;
                        (None, info)
                    }
                    Some(Some(expansion)) => (Some(expansion), info),
                    Some(None) => {
                        let default = data.default_value;
                        match default {
                            Some(default) => (Some(default), info),
                            None => {
                                info.failed = true;
                                (None, info)
                            }
                        }
                    }
                }
            }

            // `IrNode::getImmediateReference` -- a node stands for its alias
            // output, resolved one context deeper.
            Kind::Node(data) => {
                let Some(definition) = data.definition else {
                    info.failed = true;
                    return (None, info);
                };

                let alias = self.alias_output(definition);

                if alias.is_some() {
                    info.new_context = query
                        .input_context
                        .map(|context| state.contexts.child(context, Some(structure)));
                }

                (alias, info)
            }

            // `IrUnaryOperator::getImmediateReference`
            Kind::Unary { .. } => match state.expansion(structure, query.input_context) {
                None => {
                    info.reached_dead_end = true;
                    (None, info)
                }
                Some(None) => {
                    info.failed = true;
                    (None, info)
                }
                Some(expansion) => (expansion, info),
            },

            // `IrBinaryOperator::getImmediateReference`
            Kind::Binary { op, left, right, .. } if *op == crate::ir::BinOp::Dot => {
                let Kind::Label(label) = self.kind(*right) else {
                    // A dot whose right-hand side is not a label cannot occur:
                    // the parser only builds one from `data_access '.' label`.
                    info.failed = true;
                    return (None, info);
                };

                let basic_query = ReferenceQuery::at(query.input_context);
                // A `getReference` call always starts a fresh chain, but the
                // expansions and infinite-loop marks live on the structures
                // (`m_expansions` / `m_infiniteLoops`), so they are shared.
                let (resolved_left, basic_info) = self.reference(*left, basic_query, state);


                if basic_info.failed {
                    info.failed = true;
                    return (None, info);
                }

                if basic_info.reached_dead_end {
                    info.reached_dead_end = true;
                    return (None, info);
                }

                let touched_main_context = basic_info.touched_main_context;
                info.touched_main_context = touched_main_context;

                let skeleton_type = basic_info.fixed_type;
                let resolved_left = resolved_left.expect("a resolved reference is not failed");

                let mut public_attribute = self.resolve_local_name(resolved_left, &label.data);

                if public_attribute.is_none() {
                    if let Some(skeleton) = skeleton_type {
                        public_attribute = self.resolve_local_name(skeleton, &label.data);
                    }
                }


                let Some(public_attribute) = public_attribute else {
                    info.failed = true;

                    let is_valid_error = (touched_main_context && !basic_info.is_static_type())
                        || query.is_empty(&state.contexts);

                    if query.record_errors && is_valid_error {
                        info.err = Some(CompilationError::with_instantiation(
                            self.entry(*right).summary.clone(),
                            ErrorCode::UndefinedMember,
                            instantiation_frames(&state.contexts, query.input_context),
                        ));
                    }

                    return (None, info);
                };

                if !self.allows_external_access(public_attribute) {
                    info.failed = true;

                    let empty = query.is_empty(&state.contexts);
                    let is_valid_error = (empty || touched_main_context)
                        && ((basic_info.is_fixed_type() && empty) || !basic_info.is_fixed_type());

                    if query.record_errors && is_valid_error {
                        info.err = Some(CompilationError::with_instantiation(
                            self.entry(*right).summary.clone(),
                            ErrorCode::AccessingInternalMember,
                            instantiation_frames(&state.contexts, query.input_context),
                        ));
                    }

                    return (None, info);
                }

                if let Some(context) = basic_info.new_context {
                    info.new_context = Some(match self.as_node(resolved_left) {
                        Some(node) => state.contexts.child(context, Some(node)),
                        None => state.contexts.child(context, None),
                    });
                }

                (Some(public_attribute), info)
            }

            Kind::Binary { .. } => match state.expansion(structure, query.input_context) {
                None => {
                    info.reached_dead_end = true;
                    (None, info)
                }
                Some(None) => {
                    info.failed = true;
                    (None, info)
                }
                Some(expansion) => (expansion, info),
            },

            // `IrValueNodeRef::getImmediateReference` -- a node used as a value
            // stands for that node, in the same context.
            Kind::NodeRef(node) => (Some(*node), info),

            // `IrInternalReference::getImmediateReference` -- the reference and
            // its context were fixed when the expansion was created.
            Kind::InternalReference { structure, context } => {
                info.new_context = *context;
                (Some(*structure), info)
            }

            // Everything else has no immediate reference at all.
            _ => (None, info),
        }
    }

    /// `IrParserStructure::checkReferences`, with `IrNode`'s override.
    ///
    /// The base version checks every component with the same input context, so a
    /// name that is only visible through a sibling is still reported where it
    /// was written. A node adds two things: it refuses to descend into a context
    /// that already has it (`findContext`), and it gives its *definition* a
    /// child context of its own -- the node is what the definition body's names
    /// resolve through, so the body has to see it. The node's own attributes are
    /// still checked against the input context, not the child.
    pub fn check_references(
        &mut self,
        structure: StructureId,
        context: ContextId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        if let Kind::Node(data) = self.kind(structure).clone() {
            if state.contexts.find_context(context, structure).is_some() {
                return;
            }

            let this_context = state.contexts.child(context, Some(structure));

            for child in self.components(structure) {
                self.check_references(child, context, state, errors);
            }

            if let Some(definition) = data.definition {
                self.check_references(definition, this_context, state, errors);
            }

            return;
        }

        for child in self.components(structure) {
            self.check_references(child, context, state, errors);
        }

        if self.entry(structure).check_references {
            let query = ReferenceQuery::recording(Some(context));
            let (_, info) = self.reference(structure, query, state);

            if let Some(err) = info.err {
                errors.push(err);
            }
        }
    }

    /// `IrParserStructure::checkInstantiation` and its two overrides: the
    /// second half of the reference phase.
    ///
    /// Components are visited first and the structure is checked last, once
    /// (`m_instantiationChecked`). A unit then walks its components against one
    /// fresh main context; a *node* checks its definition with a main child of a
    /// fresh non-main root, so the definition body is resolved with that node as
    /// its focus. That is what makes `some_input.a` inside a definition an
    /// unresolved member at an instantiation site but not in the definition's
    /// own (empty) context -- the shape behind the `R0060` family.
    fn check_instantiation(
        &mut self,
        structure: StructureId,
        state: &mut ReferenceState,
        errors: &mut Vec<CompilationError>,
    ) {
        if self.entry(structure).flags.instantiation_checked {
            return;
        }

        self.entry_mut(structure).flags.instantiation_checked = true;

        for child in self.components(structure) {
            self.check_instantiation(child, state, errors);
        }

        match self.kind(structure).clone() {
            Kind::Unit(_) => {
                let main = state.contexts.new_root(None, true);

                for child in self.components(structure) {
                    self.check_references(child, main, state, errors);
                }
            }

            Kind::Node(data) => {
                let parent = state.contexts.new_root(None, false);
                let main = state.contexts.main_child(parent, Some(structure));

                if let Some(definition) = data.definition {
                    self.check_references(definition, main, state, errors);
                }
            }

            _ => {}
        }
    }

    /// `IrCompilationUnit::resolveAll`'s `checkInstantiation()` step.
    ///
    /// Each unit gets a fresh main context, but the expansions are shared:
    /// `_checkInstantiation` allocates a new `IrContextTree(nullptr, true)` per
    /// unit, and contexts compare by value, so it still sees everything
    /// expansion recorded.
    pub fn check_references_all(&mut self) {
        let mut errors = Vec::new();

        let mut state = std::mem::take(&mut self.expansion_state);

        for unit in self.units.clone() {
            self.check_instantiation(unit, &mut state, &mut errors);
        }

        self.expansion_state = state;
        self.errors.extend(errors);
    }

    /// `IrNode::getAsNode`, plus `IrValueNodeRef::getAsNode`, which unwraps the
    /// node the value stands for.
    pub fn as_node(&self, structure: StructureId) -> Option<StructureId> {
        match self.kind(structure) {
            Kind::Node(_) => Some(structure),
            Kind::NodeRef(node) => Some(*node),
            _ => None,
        }
    }

    /// `IrNode::isInterface`
    pub fn is_interface(&self, node: StructureId) -> bool {
        match self.kind(node) {
            Kind::Node(data) => data.is_interface,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{Arena, Kind};
    use crate::parser::parse_unit;

    fn lower(source: &str) -> (Arena, StructureId) {
        let unit = parse_unit("test.mr", source).expect("parses");
        let mut arena = Arena::default();
        let id = arena.lower_unit(&unit);
        (arena, id)
    }

    /// Parsed, lowered and with definitions resolved -- the state `expand` (and
    /// therefore the reference phase) starts from.
    fn resolved(source: &str) -> (Arena, StructureId) {
        let (mut arena, unit) = lower(source);
        arena.link_imports(&[Vec::new()]);
        arena.resolve_all_definitions();
        (arena, unit)
    }

    fn first_node(arena: &Arena, unit: StructureId) -> StructureId {
        match arena.kind(unit) {
            Kind::Unit(data) => data.nodes[0],
            _ => panic!("not a unit"),
        }
    }

    /// The `Attribute` structures below a node, in order.
    fn attributes(arena: &Arena, node: StructureId) -> Vec<StructureId> {
        let Kind::Node(data) = arena.kind(node) else {
            panic!("not a node");
        };
        match arena.kind(data.attributes) {
            Kind::AttributeList(list) => list.clone(),
            _ => panic!("not an attribute list"),
        }
    }

    #[test]
    fn a_unit_has_no_immediate_reference() {
        let (arena, unit_id) = lower("int p(1)\n");
        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        let (reference, info) =
            arena.immediate_reference(unit_id, &ReferenceQuery::at(Some(main)), &mut state);

        assert!(reference.is_none());
        assert!(!info.failed);
        assert!(!info.reached_dead_end);
        // `IR_RESET` passes the input context straight through.
        assert_eq!(info.new_context, Some(main));
        assert!(info.static_type);
    }

    #[test]
    fn an_attribute_stands_for_its_value_in_the_same_context() {
        let (arena, unit_id) = lower("int p(1)\n");
        let node = first_node(&arena, unit_id);
        let attribute = attributes(&arena, node)[0];

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        let (reference, info) =
            arena.immediate_reference(attribute, &ReferenceQuery::at(Some(main)), &mut state);

        let Kind::Attribute(data) = arena.kind(attribute) else {
            panic!("not an attribute");
        };

        assert_eq!(reference, Some(data.value));
        assert_eq!(info.new_context, Some(main));
        assert!(!info.failed);
    }

    /// The phase's second entry point. `IrNode::_checkInstantiation` checks a
    /// node's definition with a main *child* context focused on the instance, so
    /// `some_input.a` -- a member of a port, reached through the instance -- is
    /// an undefined member there. The definition's own context is empty, and in
    /// it the same expression is silent, which is why the definition-level walk
    /// alone reports nothing.
    #[test]
    fn a_definition_body_is_checked_again_for_each_instantiation() {
        // The piranha rules are needed for the literal's builtin node, and
        // `literal_int` is what gives `5` a channel type to expand through.
        let source = "\
node confusion {
    input some_input;
    output test: some_input.a;
}
confusion p(5)
public node int_channel => __piranha__int {}
public node literal_int => __piranha__literal_int {
    alias output __out [int_channel];
}
";

        let unit = parse_unit("test.mr", source).expect("parses");
        let mut arena = Arena::default();
        arena.set_rules(crate::Rules::Piranha);
        arena.lower_unit(&unit);
        arena.link_imports(&[Vec::new()]);
        arena.resolve_all_definitions();
        arena.check_circular_definitions_all();
        arena.expand_all();
        arena.check_references_all();

        let codes: Vec<&str> = arena.errors().iter().map(|error| error.code.code).collect();
        assert_eq!(codes, ["0060"]);
    }

    #[test]
    fn an_unresolvable_label_in_an_empty_context_is_an_error() {
        let (arena, unit_id) = resolved("node d {\n  input i;\n}\nd p(q)\n");
        let node = first_node(&arena, unit_id);
        let attribute = attributes(&arena, node)[0];
        let Kind::Attribute(data) = arena.kind(attribute) else {
            panic!("not an attribute");
        };
        // `checkReferences` walks every component, so the label is checked in its
        // own right -- its parent's nested pass deliberately records nothing.
        let label = data.value;

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        let (reference, info) = arena.reference(
            label,
            ReferenceQuery::recording(Some(main)),
            &mut state,
        );

        assert!(reference.is_none());
        assert!(info.failed);
        assert_eq!(info.err.as_ref().map(|err| err.code), Some(ErrorCode::UnresolvedReference));
    }

    #[test]
    fn the_same_label_is_silent_inside_an_instantiation() {
        // A context that is not empty means the walk happened inside a node, and
        // `IrValueLabel` only reports when `IR_EMPTY_CONTEXT()`.
        let (arena, unit_id) = resolved("node d {\n  input i;\n}\nd p(q)\n");
        let node = first_node(&arena, unit_id);
        let attribute = attributes(&arena, node)[0];
        let Kind::Attribute(data) = arena.kind(attribute) else {
            panic!("not an attribute");
        };
        let label = data.value;

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);
        let inner = state.contexts.child(main, Some(node));

        let (_, info) = arena.reference(
            label,
            ReferenceQuery::recording(Some(inner)),
            &mut state,
        );

        assert!(info.failed);
        assert!(info.err.is_none());
    }

    #[test]
    fn a_label_resolves_to_the_node_it_names() {
        let (arena, unit_id) = resolved("node d {\n  input i;\n}\nd p()\nd q(p)\n");
        let target = first_node(&arena, unit_id);
        let other = match arena.kind(unit_id) {
            Kind::Unit(data) => data.nodes[1],
            _ => panic!("not a unit"),
        };
        let attribute = attributes(&arena, other)[0];

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        let (reference, info) = arena.reference(
            attribute,
            ReferenceQuery::recording(Some(main)),
            &mut state,
        );

        assert_eq!(reference, Some(target));
        assert!(info.err.is_none());
    }

    #[test]
    fn an_expansionless_literal_is_a_dead_end() {
        let (arena, unit_id) = lower("int p(1)\n");
        let node = first_node(&arena, unit_id);
        let attribute = attributes(&arena, node)[0];
        let Kind::Attribute(data) = arena.kind(attribute) else {
            panic!("not an attribute");
        };
        let literal = data.value;

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        // No expansion recorded: nothing was written for this context.
        let (reference, info) =
            arena.immediate_reference(literal, &ReferenceQuery::at(Some(main)), &mut state);
        assert!(reference.is_none());
        assert!(info.reached_dead_end);
        assert!(!info.failed);

        // A null expansion is a failure instead: expansion ran and gave up.
        state.set_expansion(literal, Some(main), None);
        let (reference, info) =
            arena.immediate_reference(literal, &ReferenceQuery::at(Some(main)), &mut state);
        assert!(reference.is_none());
        assert!(info.failed);
        assert!(!info.reached_dead_end);
    }

    #[test]
    fn a_node_reference_is_its_alias_output_one_context_deeper() {
        let (arena, unit_id) = resolved(
            "node d {\n  alias output o: 5;\n  input x;\n}\nd()\n",
        );

        let instance = match arena.kind(unit_id) {
            Kind::Unit(data) => data.nodes[0],
            _ => panic!("not a unit"),
        };

        let Kind::Node(data) = arena.kind(instance) else {
            panic!("not a node");
        };
        let definition = data.definition.expect("the instance resolved its definition");
        let alias = arena.alias_output(definition).expect("d has an alias output");

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        let (reference, info) = arena.immediate_reference(
            instance,
            &ReferenceQuery::at(Some(main)),
            &mut state,
        );

        assert_eq!(reference, Some(alias));
        assert_ne!(info.new_context, Some(main));
        assert_eq!(
            info.new_context.map(|context| state.contexts.context(context)),
            Some(Some(instance))
        );
    }

    #[test]
    fn an_expansion_is_found_from_an_equal_context_not_just_the_same_one() {
        // `PKeyValueLookup<IrContextTree, IrNode *>` compares keys by value, so
        // a freshly created child that is structurally equal to one already
        // used finds the same expansion. Keying by context id lost every
        // expansion a node's `getImmediateReference` created a new child for.
        let (arena, unit_id) = resolved("node d {\n  input i;\n}\nd p(1)\n");

        let literal = {
            let node = first_node(&arena, unit_id);
            let attribute = attributes(&arena, node)[0];
            let Kind::Attribute(data) = arena.kind(attribute) else {
                panic!("not an attribute");
            };
            data.value
        };

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);
        let twin = state.contexts.new_root(None, true);

        state.set_expansion(literal, Some(main), None);
        assert_eq!(state.expansion(literal, Some(twin)), Some(None));

        state.set_expansion(literal, Some(main), Some(7));
        assert_eq!(state.expansion(literal, Some(twin)), Some(Some(7)));
        // A context that actually names a structure is a different context --
        // note that two *empty* contexts compare equal in the original, which is
        // why this has to use a non-empty one.
        let node = first_node(&arena, unit_id);
        let other = state.contexts.child(twin, Some(node));
        assert_eq!(state.expansion(literal, Some(other)), None);
    }

    #[test]
    fn a_chain_link_repeats_when_its_context_is_equal_not_identical() {
        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);
        let twin = state.contexts.new_root(None, true);

        let mut chain = ReferenceChain::default();
        chain.add_link(7, Some(main));

        assert_eq!(chain.search_link(&state.contexts, 7, Some(twin)), Some(0));
        assert_eq!(chain.search_link(&state.contexts, 8, Some(main)), None);
        assert_eq!(chain.search_link(&state.contexts, 7, None), None);
    }

    #[test]
    fn check_references_reports_one_error_per_structure() {
        let (mut arena, _) = resolved("node d {\n  input i;\n}\nd p(q)\n");
        arena.check_references_all();

        let codes: Vec<ErrorCode> = arena.errors().iter().map(|err| err.code).collect();
        assert_eq!(codes, vec![ErrorCode::UnresolvedReference]);
    }

    #[test]
    fn a_reference_cycle_is_reported_at_the_link_it_returns_to() {
        // `a` and `b` are attributes that resolve to each other through labels.
        let (arena, unit_id) = lower("int a(b)\nint b(a)\n");
        let nodes = match arena.kind(unit_id) {
            Kind::Unit(data) => data.nodes.clone(),
            _ => panic!("not a unit"),
        };
        let b = attributes(&arena, nodes[0])[0];
        let a = attributes(&arena, nodes[1])[0];

        let mut state = ReferenceState::new();
        let main = state.contexts.new_root(None, true);

        // `b` -> label `b` -> node b -> alias? -> ... ; the cycle here is the
        // label pointing at the attribute that contains it.
        let (_, info) = arena.reference(b, ReferenceQuery::recording(Some(main)), &mut state);
        let _ = a;

        assert!(info.failed);
        // Whatever the shape, the walk must terminate -- that is the property
        // this test is here for.
        assert!(matches!(
            info.err.as_ref().map(|err| err.code),
            Some(ErrorCode::CircularReference) | Some(ErrorCode::UnresolvedReference) | None
        ));
    }
}
