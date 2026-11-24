//! `checkCircularDefinitions`: the phase that runs between resolution and
//! expansion.
//!
//! It answers one question -- does a definition, followed through the nodes in
//! its body, ever come back to itself -- and it does so *before* `expand()` so
//! that expansion never chases a cycle.
//!
//! The shape is a small state machine, and the details matter:
//!
//! * each top-level definition is checked with its own fresh empty context
//!   (`IrNodeDefinition::checkCircularDefinitions`);
//! * the walk goes through `IrParserStructure::checkCircularDefinitions`, which
//!   visits every component whose `checkReferences` flag is set and then calls
//!   `_checkCircularDefinitions` -- which **only `IrNode` overrides**;
//! * a node whose definition is the root being checked is the cycle: report
//!   `CircularDefinition` and stop descending there;
//! * `IrNode`'s override calls the **base** `checkCircularDefinitions` on the
//!   definition it reaches, deliberately bypassing the definition's own override
//!   (`IrNodeDefinition::checkCircularDefinitions()` would have started a new
//!   context chain);
//! * a node already present in the context chain is not descended into again,
//!   which is what stops a cycle inside a definition that is *not* the root.

use crate::context::ContextArena;
use crate::error::{CompilationError, ErrorCode, InstantiationFrame};
use crate::lower::{Arena, Kind, StructureId};

impl Arena {
    /// `IrCompilationUnit::resolveAll`'s loop over definitions.
    pub fn check_circular_definitions_all(&mut self) {
        for unit in self.units.clone() {
            let definitions = match self.kind(unit) {
                Kind::Unit(data) => data.definitions.clone(),
                _ => continue,
            };

            for definition in definitions {
                self.check_circular_definitions_for(definition);
            }
        }
    }

    /// `IrNodeDefinition::checkCircularDefinitions`
    fn check_circular_definitions_for(&mut self, definition: StructureId) {
        let mut contexts = ContextArena::new();
        let root_context = contexts.empty();

        self.check_circular_definitions(definition, root_context, definition, &mut contexts);
    }

    /// `IrParserStructure::checkCircularDefinitions`
    fn check_circular_definitions(
        &mut self,
        structure: StructureId,
        context: usize,
        root: StructureId,
        contexts: &mut ContextArena,
    ) {
        for child in self.components(structure) {
            if self.entry(child).check_references {
                self.check_circular_definitions(child, context, root, contexts);
            }
        }

        self.check_circular_of(structure, context, root, contexts);
    }

    /// `IrNode::_checkCircularDefinitions` -- the only override.
    fn check_circular_of(
        &mut self,
        structure: StructureId,
        context: usize,
        root: StructureId,
        contexts: &mut ContextArena,
    ) {
        let Kind::Node(data) = self.kind(structure) else {
            return;
        };

        let definition = data.definition;

        if definition == Some(root) {
            let location = self.entry(structure).summary.clone();
            let frames = instantiation_frames(contexts, context);

            self.errors.push(CompilationError::with_instantiation(
                location,
                ErrorCode::CircularDefinition,
                frames,
            ));
            return;
        }

        // Loop guard for a cycle inside a definition that is not the focus.
        if contexts.find_context(context, structure).is_some() {
            return;
        }

        let new_context = contexts.new_child(context, Some(structure), true);

        if let Some(definition) = definition {
            // The base implementation, not the definition's own override.
            self.check_circular_definitions(definition, new_context, root, contexts);
        }
    }
}

/// The context chain below `context`, oldest frame first -- the shape
/// `es_script::Compiler::printError` prints as `While instantiating:`.
fn instantiation_frames(contexts: &ContextArena, context: usize) -> Vec<InstantiationFrame> {
    let mut frames = Vec::new();
    let mut current = Some(context);

    while let Some(id) = current {
        if let Some(node) = contexts.context(id) {
            frames.push(InstantiationFrame { node });
        }

        current = contexts.parent(id);
    }

    frames.reverse();
    frames
}

#[cfg(test)]
mod tests {
    use crate::lower::Arena;
    use crate::parser::parse_unit;

    fn run(source: &str) -> Arena {
        let unit = parse_unit("t.mr", source).expect("parses");
        let mut arena = Arena::default();
        arena.lower_unit(&unit);
        arena.link_imports(&[Vec::new()]);
        arena.resolve_all_definitions();
        arena.check_circular_definitions_all();
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
    fn a_definition_that_contains_itself_is_circular() {
        let arena = run("public node a { input x; a(1) }\n");
        assert_eq!(codes(&arena), vec!["E0020"]);
    }

    #[test]
    fn a_definition_that_contains_another_is_not() {
        let arena = run("public node b { input y: 1; }\npublic node a { input x; b(1) }\n");
        assert!(codes(&arena).is_empty(), "{:?}", codes(&arena));
    }

    #[test]
    fn a_self_cycle_is_not_an_instantiation_error() {
        // The cycle is found in the root definition itself, so the context is
        // the empty tree and `isInstantiationError()` is false.
        let arena = run("public node a { input x; a(1) }\n");
        let error = arena.errors().first().expect("an error");

        assert_eq!(error.code.stage, "E");
        assert!(!error.is_instantiation_error());
    }

    #[test]
    fn a_deeper_cycle_carries_an_instantiation_chain() {
        let arena = run("public node a { input x; b(1) }\npublic node b { input y; a(1) }\n");

        // Checking `a` reaches `a` through `b`, so that error has a context.
        let error = arena.errors().first().expect("an error");
        assert!(error.is_instantiation_error());
        assert_eq!(error.instantiation.as_ref().map(|frames| frames.len()), Some(1));
    }

    #[test]
    fn a_two_definition_cycle_is_detected_from_either_root() {
        // `a` contains `b`, `b` contains `a`: checking `a` reports the cycle,
        // and checking `b` reports it again (once per root definition).
        let arena = run("public node a { input x; b(1) }\npublic node b { input y; a(1) }\n");
        assert_eq!(codes(&arena), vec!["E0020", "E0020"]);
    }
}
