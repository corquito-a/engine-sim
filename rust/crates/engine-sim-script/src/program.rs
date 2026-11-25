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
use crate::objects::EngineContext;
use crate::values::Value;

use engine_sim_core::constants::PI;
use engine_sim_core::function::Function;
use engine_sim_core::units::THOU;

use std::collections::BTreeMap;

/// What `NodeProgram` owns that is not the graph: run state and the console's
/// text. piranha writes to `std::cout` directly; a library returns it instead.
pub struct Program {
    pub graph: Graph,
    initialized: bool,
    evaluated: Vec<bool>,
    evaluating: Vec<bool>,
    console: String,
    runtime_error: Option<String>,
    /// The samples `add_sample` pushed onto each `function` node. The node holds
    /// them, the generated `Function` reads them, and the order they arrive in is
    /// the order the actions run in.
    samples: BTreeMap<usize, Vec<(f64, f64)>>,
    /// `FunctionNode::setFilterRadius`, which a generator (`generate_harmonic_
    /// cam_lobe` does) calls after the node read its `filter_radius` input.
    filter_radius: BTreeMap<usize, f64>,
    /// The objects the data nodes generate, one per node.
    pub context: EngineContext,
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
            samples: BTreeMap::new(),
            filter_radius: BTreeMap::new(),
            context: EngineContext::new(),
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

        // `NodeContainer::_evaluate` only ever reaches the nodes of the container
        // a node owns, so a container whose owning node nothing evaluates -- an
        // engine part's group, reached only through an object input -- would never
        // run. Those are the containers the graph still has nodes in and the walk
        // above did not touch; run them in pre-order, the way `initialize` visits
        // them, and let the evaluated flag keep the order deterministic.
        for container in self.unreached_containers(root) {
            for node in self.graph.node(container).nodes.clone() {
                self.evaluate(node)?;
            }
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
            // `Node::evaluate`: the input's dependency runs before the node the
            // port is wired to. A data node's object input is the reachable case:
            // the dependency is the container the object was declared in, and
            // running it is what lets that container's `add_sample`s run at all.
            if let Some(dependency) = input.dependency {
                self.evaluate(dependency)?;
            }

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
    /// The containers under `root` that still hold nodes nothing has evaluated.
    fn unreached_containers(&self, root: usize) -> Vec<usize> {
        let mut containers = Vec::new();
        let mut stack = vec![root];

        while let Some(container) = stack.pop() {
            containers.push(container);

            for child in self.graph.node(container).children.iter().rev() {
                stack.push(*child);
            }
        }

        containers
            .into_iter()
            .filter(|container| {
                self.graph
                    .node(*container)
                    .nodes
                    .iter()
                    .any(|node| !self.evaluated[*node])
            })
            .collect()
    }

    fn run(&mut self, node: usize) -> Result<(), String> {
        let data = self.graph.node(node);

        match data.class {
            // `NodeContainer::_evaluate`: a container node run means the nodes in
            // the container it owns run, in order, and a failure anywhere in them
            // fails the container.
            NodeClass::Container => {
                for child in data.children.clone() {
                    for node in self.graph.node(child).nodes.clone() {
                        self.evaluate(node)?;
                    }
                }

                return Ok(());
            }

            NodeClass::Builtin => {}

            _ => return Ok(()),
        }

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

            // `AddSampleNode::_evaluate`: read the point, then push it onto the
            // node it references -- the function is generated later, from these.
            "add_sample" => {
                let x = self.number(node, "x");
                let y = self.number(node, "y");

                if let (Some(x), Some(y), Some(function)) = (x, y, self.object_source(node, "function"))
                {
                    self.samples.entry(function).or_default().push((x, y));
                }

                Ok(())
            }

            // `GenerateHarmonicCamLobeNode::_evaluate`: a symmetric lobe profile
            // written straight into the node's sample list, then the node's filter
            // radius set to the step -- after the node had read its input.
            "generate_harmonic_cam_lobe" => {
                let duration = self.number(node, "duration_at_50_thou").unwrap_or(0.0);
                let gamma = self.number(node, "gamma").unwrap_or(1.0);
                let lift = self.number(node, "lift").unwrap_or(300.0);
                let steps = self.number(node, "steps").unwrap_or(100.0) as i32;

                if let Some(target) = self.object_source(node, "function") {
                    let angle = duration / 4.0;
                    let s = (2.0 * 50.0 * THOU / lift).powf(1.0 / gamma) - 1.0;
                    let k = s.acos() / angle;
                    let extents = PI / k;
                    let step = extents / (steps as f64 - 5.0);

                    let points = self.samples.entry(target).or_default();
                    for i in 0..steps {
                        if i == 0 {
                            points.push((0.0, lift));
                        }
                        else {
                            let x = i as f64 * step;
                            let value = if x >= extents {
                                0.0
                            }
                            else {
                                lift * (0.5 + 0.5 * (k * x).cos()).powf(gamma)
                            };

                            points.push((x, value));
                            points.push((-x, value));
                        }
                    }

                    self.filter_radius.insert(target, step);
                }

                Ok(())
            }

            // `ObjectReferenceNode::_evaluate` sets its output to itself and
            // reads its inputs into its parameter fields; the object is generated
            // on demand. Nothing to do at run time.

            // `IntProbeNode` computes its input into a local and drops it, and
            // every other class computes on demand.
            _ => Ok(()),
        }
    }

    /// The node an object input references: `InputTarget::Type::Object` reads
    /// the pointer out of an `ObjectReferenceNodeOutput`, which is the node on
    /// the other end of the connection.
    fn object_source(&self, node: usize, name: &str) -> Option<usize> {
        let port = self
            .graph
            .node(node)
            .inputs
            .iter()
            .find(|input| input.name == name)?
            .connection?;

        let port = self.graph.port(port);
        let owner = port.owner?;

        // An object input can be wired to an interface output, which forwards the
        // pointer the referenced node published; follow it to the node itself.
        if self.graph.node(owner).builtin_name.is_empty() {
            if let Some(interface) = port.interface {
                return Some(interface);
            }
        }

        Some(owner)
    }

    fn number(&self, node: usize, name: &str) -> Option<f64> {
        let value = self.input(node, name)?;

        match value {
            Value::Float(x) => Some(x),
            Value::Int(x) => Some(x as f64),
            _ => None,
        }
    }

    /// `FunctionNode::generate`: the curve this node's samples describe, built
    /// once. `None` for a node that is not a function.
    pub fn function(&mut self, node: usize) -> Option<&Function> {
        if self
            .graph
            .node(node)
            .builtin_name
            .is_empty()
        {
            return None;
        }

        let samples = self.samples.get(&node).cloned().unwrap_or_default();
        let filter_radius = self
            .filter_radius
            .get(&node)
            .copied()
            .or_else(|| self.number(node, "filter_radius"))
            .unwrap_or(0.0);

        Some(self.context.generate_function(node, &samples, filter_radius))
    }

    /// How many points `add_sample` has pushed onto a node so far.
    pub fn sample_count(&self, node: usize) -> usize {
        self.samples.get(&node).map(|points| points.len()).unwrap_or(0)
    }

    /// The nodes whose class is `function`, in graph order.
    pub fn function_nodes(&self) -> Vec<usize> {
        self.graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, data)| class_name(&data.builtin_name) == Some("function"))
            .map(|(node, _)| node)
            .collect()
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
