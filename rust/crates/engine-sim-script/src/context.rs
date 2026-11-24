//! `piranha::IrContextTree`: the instantiation context a reference is resolved
//! in.
//!
//! A context tree is a chain of frames, each naming the `IrNode` instance whose
//! ports are in scope. It is what makes `inline` arguments work: resolving
//! `input a: b;` inside the *definition* of `b` happens in a context that maps
//! the definition's ports onto the arguments the caller passed.
//!
//! The original allocates these freely and compares them **by identity**
//! (`IsEqual` walks both chains comparing `getContext()` pointers), so they live
//! in an arena here and are referred to by [`ContextId`].
//!
//! Two subtleties worth stating, because both are load-bearing for circular
//! reference detection:
//!
//! * an *empty* tree (`IrContextTree()`, context == null) is not "no context":
//!   `IR_EMPTY_CONTEXT()` tests exactly that, and a chain that reaches the empty
//!   tree has run out of scopes rather than out of parents;
//! * two chains are equal only if they have the same length *and* the same
//!   contexts — `is_equal` returns false as soon as one chain is longer.

use crate::lower::{Arena, Kind, StructureId};

pub type ContextId = usize;

#[derive(Clone, Debug)]
struct ContextNode {
    context: Option<StructureId>,
    main: bool,
    parent: Option<ContextId>,
}

#[derive(Clone, Debug, Default)]
pub struct ContextArena {
    nodes: Vec<ContextNode>,
    /// The shared empty context, created on demand.
    empty: Option<ContextId>,
}

impl ContextArena {
    pub fn new() -> Self {
        ContextArena::default()
    }

    /// `IrContextTree()` -- a tree with no context at all.
    pub fn empty(&mut self) -> ContextId {
        if let Some(empty) = self.empty {
            return empty;
        }

        let id = self.push(ContextNode { context: None, main: false, parent: None });
        self.empty = Some(id);
        id
    }

    /// `IrContextTree(context, mainContext)`
    pub fn new_root(&mut self, context: Option<StructureId>, main: bool) -> ContextId {
        self.push(ContextNode { context, main, parent: None })
    }

    /// `IrContextTree::newChild`
    pub fn new_child(
        &mut self,
        parent: ContextId,
        context: Option<StructureId>,
        main: bool,
    ) -> ContextId {
        self.push(ContextNode { context, main, parent: Some(parent) })
    }

    fn push(&mut self, node: ContextNode) -> ContextId {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn context(&self, id: ContextId) -> Option<StructureId> {
        self.nodes[id].context
    }

    /// `IrContextTree::isEmpty`
    pub fn is_empty(&self, id: ContextId) -> bool {
        self.nodes[id].context.is_none()
    }

    pub fn is_main_context(&self, id: ContextId) -> bool {
        self.nodes[id].main
    }

    pub fn parent(&self, id: ContextId) -> Option<ContextId> {
        self.nodes[id].parent
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn get(&self, id: ContextId) -> Option<StructureId> {
        self.nodes.get(id).and_then(|node| node.context)
    }

    /// `IrContextTree::getRoot`
    pub fn root(&self, id: ContextId) -> ContextId {
        let mut current = id;
        while let Some(parent) = self.parent(current) {
            current = parent;
        }
        current
    }

    /// `IrContextTree::getMain`
    pub fn main(&self, id: ContextId) -> Option<ContextId> {
        self.find_main(self.root(id))
    }

    /// `IrContextTree::_getMain` -- depth first, first main context wins.
    fn find_main(&self, id: ContextId) -> Option<ContextId> {
        if self.is_main_context(id) {
            return Some(id);
        }

        // Children are not stored on the parent, so the walk scans the arena;
        // creation order is arena order, which is the order the original's
        // `m_children` vector holds.
        for (candidate, node) in self.nodes.iter().enumerate() {
            if node.parent != Some(id) {
                continue;
            }

            if let Some(main) = self.find_main(candidate) {
                return Some(main);
            }
        }

        None
    }

    /// `IrContextTree::hasParent` -- is `other` an ancestor of `id`?
    pub fn has_parent(&self, id: ContextId, other: ContextId) -> bool {
        let mut current = self.parent(id);
        while let Some(candidate) = current {
            if self.is_equal(candidate, other) {
                return true;
            }
            current = self.parent(candidate);
        }

        false
    }

    /// `IrContextTree::findContext` -- the nearest frame holding this node.
    pub fn find_context(&self, id: ContextId, context: StructureId) -> Option<ContextId> {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if self.context(candidate) == Some(context) {
                return Some(candidate);
            }
            current = self.parent(candidate);
        }

        None
    }

    /// `IrContextTree::resolveDefinition` -- in this context, which attribute of
    /// the instance provides the value for this port?
    pub fn resolve_definition(
        &self,
        id: ContextId,
        definition: StructureId,
        arena: &Arena,
    ) -> Option<StructureId> {
        let context = self.context(id)?;
        let Kind::Node(data) = arena.kind(context) else {
            return None;
        };

        let Kind::AttributeList(attributes) = arena.kind(data.attributes) else {
            return None;
        };

        attributes.iter().copied().find(|attribute| match arena.kind(*attribute) {
            Kind::Attribute(data) => data.definition == Some(definition),
            _ => false,
        })
    }

    /// `IrContextTree::isEqual`
    pub fn is_equal(&self, left: ContextId, right: ContextId) -> bool {
        let mut current = left;
        let mut reference = right;

        loop {
            if self.context(current) != self.context(reference) {
                return false;
            }

            if self.is_empty(current) || self.is_empty(reference) {
                break;
            }

            match (self.parent(current), self.parent(reference)) {
                (None, None) => return true,
                (Some(parent), Some(reference_parent)) => {
                    current = parent;
                    reference = reference_parent;
                }
                // One chain is longer: only equal if both ran out at once.
                (left_parent, right_parent) => return left_parent.is_none() && right_parent.is_none(),
            }
        }

        self.context(current) == self.context(reference)
    }

    /// `IrContextTree::isOutside`
    pub fn is_outside(&self, id: ContextId, reference: ContextId) -> bool {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if self.is_equal(reference, candidate) {
                return false;
            }
            else if self.has_parent(reference, candidate) {
                return true;
            }

            current = self.parent(candidate);
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_unit;

    fn one_node() -> (Arena, StructureId) {
        let unit = parse_unit("t.mr", "int a(1)\n").expect("parses");
        let mut arena = Arena::default();
        let unit_id = arena.lower_unit(&unit);

        let node = match arena.kind(unit_id) {
            Kind::Unit(data) => data.nodes[0],
            _ => panic!("not a unit"),
        };

        (arena, node)
    }

    #[test]
    fn empty_contexts_are_shared_and_empty() {
        let mut contexts = ContextArena::new();
        let first = contexts.empty();
        let second = contexts.empty();

        assert_eq!(first, second);
        assert!(contexts.is_empty(first));
        assert_eq!(contexts.context(first), None);
    }

    #[test]
    fn roots_walk_to_the_main_context() {
        let mut contexts = ContextArena::new();
        let main = contexts.new_root(None, true);
        let child = contexts.new_child(main, None, false);
        let grandchild = contexts.new_child(child, None, false);

        assert_eq!(contexts.root(grandchild), main);
        assert_eq!(contexts.main(grandchild), Some(main));
    }

    #[test]
    fn equality_compares_the_whole_chain() {
        let (arena, node) = one_node();
        let other = {
            let Kind::Node(data) = arena.kind(node) else {
                panic!("not a node");
            };
            data.attributes
        };

        let mut contexts = ContextArena::new();
        let a = contexts.new_root(Some(node), false);
        let b = contexts.new_root(Some(node), false);
        assert!(contexts.is_equal(a, b), "same context, same depth");

        let a_child = contexts.new_child(a, Some(other), false);
        assert!(!contexts.is_equal(a_child, b), "a deeper chain is not equal");

        let b_child = contexts.new_child(b, Some(other), false);
        assert!(contexts.is_equal(a_child, b_child));
    }

    #[test]
    fn different_contexts_are_never_equal() {
        let (arena, node) = one_node();
        let unit = arena.entry(node).parent_unit.unwrap();

        let mut contexts = ContextArena::new();
        let a = contexts.new_root(Some(node), false);
        let b = contexts.new_root(Some(unit), false);

        assert!(!contexts.is_equal(a, b));
        assert!(!contexts.is_outside(a, b));
    }

    #[test]
    fn a_context_inside_another_is_not_outside_it() {
        let (arena, node) = one_node();
        let attributes = match arena.kind(node) {
            Kind::Node(data) => data.attributes,
            _ => panic!(),
        };

        let mut contexts = ContextArena::new();
        let root = contexts.new_root(Some(node), false);
        let child = contexts.new_child(root, Some(attributes), false);

        // The root is outside its own child...
        assert!(contexts.is_outside(root, child));
        // ... and the child is not outside the root.
        assert!(!contexts.is_outside(child, root));

        // A tree is never outside itself.
        assert!(!contexts.is_outside(root, root));
    }

    #[test]
    fn parents_are_searched_by_value_of_the_chain() {
        let (arena, node) = one_node();
        let attributes = match arena.kind(node) {
            Kind::Node(data) => data.attributes,
            _ => panic!(),
        };

        let mut contexts = ContextArena::new();
        let root = contexts.new_root(Some(node), false);
        let child = contexts.new_child(root, Some(attributes), false);

        assert!(contexts.has_parent(child, root));
        assert!(!contexts.has_parent(root, child));

        // A different tree with the same shape is "the same" parent.
        let copy = contexts.new_root(Some(node), false);
        let copy_child = contexts.new_child(copy, Some(attributes), false);
        assert!(contexts.has_parent(copy_child, root));
    }

    #[test]
    fn find_context_looks_up_the_chain() {
        let (arena, node) = one_node();
        let attributes = match arena.kind(node) {
            Kind::Node(data) => data.attributes,
            _ => panic!(),
        };

        let mut contexts = ContextArena::new();
        let root = contexts.new_root(Some(node), false);
        let child = contexts.new_child(root, Some(attributes), false);

        assert_eq!(contexts.find_context(child, attributes), Some(child));
        assert_eq!(contexts.find_context(child, node), Some(root));
        assert_eq!(contexts.find_context(child, usize::MAX), None);
    }
}
