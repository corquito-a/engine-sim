//! `es_script`'s object graph: the nodes that *hold* data, and the objects they
//! generate on demand.
//!
//! A data node (`function`, `fuel`, `piston`, ...) is an `ObjectReferenceNode`:
//! its `_evaluate` does nothing but `setOutput(this)` and read its inputs into
//! its own parameter fields, so the node — not the object — is where the script's
//! numbers land. The object itself
//! ([`Function`], `Fuel`, `Piston`, ...) is built by `generate(context)` the
//! first time something asks for it, and `EngineContext` hands the same object
//! back on every later ask. That is why `add_sample` can run before the function
//! it feeds is generated: it pushes onto the *node*, and generation reads the
//! node.
//!
//! Only the pieces M4 has reached are here; `EngineContext` grows a map per
//! object type as the classes land.

use engine_sim_core::function::Function;

use std::collections::BTreeMap;

/// `es_script::EngineContext`, restricted to what has been ported: one cache per
/// object type, keyed by the node that generates it.
#[derive(Default)]
pub struct EngineContext {
    functions: BTreeMap<usize, Function>,
}

impl EngineContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// `EngineContext::getFunction`
    pub fn function(&self, node: usize) -> Option<&Function> {
        self.functions.get(&node)
    }

    /// `EngineContext::addFunction`
    pub fn add_function(&mut self, node: usize, function: Function) {
        self.functions.insert(node, function);
    }

    /// `FunctionNode::generate`: build the curve the first time, then hand the
    /// same one back.
    pub fn generate_function(
        &mut self,
        node: usize,
        samples: &[(f64, f64)],
        filter_radius: f64,
    ) -> &Function {
        if !self.functions.contains_key(&node) {
            let mut function = Function::new(samples.len(), filter_radius);
            for &(x, y) in samples {
                function.add_sample(x, y);
            }

            self.functions.insert(node, function);
        }

        self.functions.get(&node).expect("just inserted")
    }

    /// The nodes that generated an object, in order.
    pub fn function_nodes(&self) -> Vec<usize> {
        self.functions.keys().copied().collect()
    }
}
