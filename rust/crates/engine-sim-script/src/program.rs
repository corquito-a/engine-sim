//! `NodeProgram`: the part of the runtime that walks the graph.
//!
//! The value layer ([`crate::values`]) is pull-only, so a program is only
//! needed for the nodes that *do* something when they run: `console_out` writes
//! its input, `throw_runtime_error` turns a flag into a failed run, and the
//! stateful `es_script` classes (M4) will advance state. Everything else has an
//! empty `_evaluate`, which is why `execute` here is short and why the graph's
//! values never depend on it.
//!
//! The walk is `NodeProgram::initialize` plus `Node::evaluate`, step for step:
//!
//! * `initialize` visits the *top level's* nodes and each container's children
//!   through `NodeContainer::_initialize` -- `Node::initialize` itself does not
//!   recurse;
//! * `evaluate` returns early when the node is already evaluated or is being
//!   evaluated (the cycle guard that also makes diamond-shaped graphs evaluate
//!   once), evaluates every connected input's node first, then runs `_evaluate`,
//!   then the nodes behind its output references.
//!
//! What is *not* here: `checkEnabled` (no fixture enables an input),
//! `optimize`/`markDeadNodes`, and `free`.

use crate::graph::{class_name, Graph, NodeClass};
use crate::values::Value;

/// What `NodeProgram` owns that is not the graph: run state and the console's
/// text. piranha writes to `std::cout` directly; a library returns it instead.
pub struct Program {
    pub graph: Graph,
    initialized: bool,
    evaluated: Vec<bool>,
    evaluating: Vec<bool>,
    console: String,
    runtime_error: Option<String>,
}

impl Program {
    pub fn new(graph: Graph) -> Program {
        let count = graph.nodes.len();
        Program {
            graph,
            initialized: false,
            evaluated: vec![false; count],
            evaluating: vec![false; count],
            console: String::new(),
            runtime_error: None,
        }
    }

    /// Everything `console_out` has written so far.
    pub fn console(&self) -> &str {
        &self.console
    }

    pub fn runtime_error(&self) -> Option<&str> {
        self.runtime_error.as_deref()
    }

    /// `NodeProgram::initialize`: the top level's nodes, and every container's
    /// children in order.
    pub fn initialize(&mut self) {
        if self.initialized {
            return;
        }

        if let Some(root) = self.graph.root {
            self.initialize_container(root);
        }

        self.initialized = true;
    }

    fn initialize_container(&mut self, container: usize) {
        for node in self.graph.node(container).nodes.clone() {
            self.initialize_node(node);
        }

        for child in self.graph.node(container).children.clone() {
            self.initialize_container(child);
        }
    }

    /// `Node::initialize`: `_initialize`, then the port registrations. The ports
    /// already exist on a built graph, so only the flag and the classes'
    /// `_initialize` state can matter.
    fn initialize_node(&mut self, node: usize) {
        if self.graph.node(node).initialized {
            return;
        }

        self.graph.nodes[node].initialized = true;
    }

    /// `NodeProgram::execute`: one pass over the top level, in order. `Err` is
    /// the original's `false`, with the message `throwError` recorded.
    pub fn execute(&mut self) -> Result<(), String> {
        self.initialize();

        let Some(root) = self.graph.root else {
            return Ok(());
        };

        for node in self.graph.node(root).nodes.clone() {
            self.evaluate(node)?;
        }

        Ok(())
    }

    /// `Node::evaluate`.
    fn evaluate(&mut self, node: usize) -> Result<(), String> {
        if self.evaluated[node] || self.evaluating[node] {
            return Ok(());
        }

        self.evaluating[node] = true;

        // `(*node)->evaluate()` for every connected input: the node that owns
        // the output a port is wired to runs first.
        let inputs = self.graph.node(node).inputs.clone();
        for input in &inputs {
            if let Some(port) = input.connection {
                if let Some(owner) = self.graph.port(port).owner {
                    self.evaluate(owner)?;
                }
            }
        }

        self.run(node)?;

        // `m_outputReferences`' `nodeOutput`s.
        for reference in self.graph.node(node).output_references.clone() {
            if let Some(node_output) = reference.node_output {
                self.evaluate(node_output)?;
            }
        }

        self.evaluated[node] = true;
        self.evaluating[node] = false;

        Ok(())
    }

    /// `Node::_evaluate`, for the classes that have one.
    fn run(&mut self, node: usize) -> Result<(), String> {
        let data = self.graph.node(node);
        let NodeClass::Builtin = data.class else {
            return Ok(());
        };

        let Some(class) = class_name(&data.builtin_name) else {
            return Ok(());
        };

        match class {
            "console_out" => {
                if let Some(value) = self.input(node, "data") {
                    self.console.push_str(&value.text());
                }

                Ok(())
            }

            "throw_runtime_error" => {
                let throws = matches!(self.input(node, "throw"), Some(Value::Bool(true)));
                if throws {
                    let message = "Planned error".to_string();
                    self.runtime_error = Some(message.clone());
                    return Err(message);
                }

                Ok(())
            }

            // `IntProbeNode` computes its input into a local and drops it, and
            // every other class computes on demand.
            _ => Ok(()),
        }
    }

    fn input(&self, node: usize, name: &str) -> Option<Value> {
        let port = self
            .graph
            .node(node)
            .inputs
            .iter()
            .find(|input| input.name == name)?
            .connection?;

        self.graph.value_of(port)
    }
}
