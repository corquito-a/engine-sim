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
use crate::objects::{
    CrankData, CylinderRefs, EngineContext, PistonData, RodData, RodJournalData, VehicleData,
};
use crate::values::Value;

use engine_sim_core::constants::PI;
use engine_sim_core::fuel::{Fuel, FuelParameters};
use engine_sim_core::units::{G, KG, KJ, L};
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
        // run. Those are the containers the walk above did not touch, and they are
        // run in *reverse* id order: a dependency in this graph points forward
        // (`add_cylinder`'s bank input depends on a later container), so last-first
        // is a topological order of the dependency forest -- run the other way
        // round, a bank's cylinders come out reversed.
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
    /// The containers under `root` that still hold nodes nothing has evaluated,
    /// last first (see `execute`).
    fn unreached_containers(&self, root: usize) -> Vec<usize> {
        let mut containers = Vec::new();
        let mut stack = vec![root];

        while let Some(container) = stack.pop() {
            containers.push(container);

            for child in self.graph.node(container).children.iter().rev() {
                stack.push(*child);
            }
        }

        let mut unreached: Vec<usize> = containers
            .into_iter()
            .filter(|container| {
                self.graph
                    .node(*container)
                    .nodes
                    .iter()
                    .any(|node| !self.evaluated[*node])
            })
            .collect();

        unreached.reverse();
        unreached
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

            // `PistonNode::_evaluate`: the five numbers it holds. The `Piston`
            // itself is the action's (`add_cylinder`), which stamps in the bank,
            // the rod and the cylinder index.
            "piston" => {
                self.context.set_piston(
                    node,
                    PistonData {
                        mass: self.number(node, "mass").unwrap_or(0.0),
                        blowby_flow_coefficient: self.number(node, "blowby").unwrap_or(0.0),
                        compression_height: self.number(node, "compression_height").unwrap_or(0.0),
                        wrist_pin_position: self
                            .number(node, "wrist_pin_position")
                            .unwrap_or(0.0),
                        displacement: self.number(node, "displacement").unwrap_or(0.0),
                    },
                );

                Ok(())
            }

            // `ConnectingRodNode::_evaluate`: five numbers, and the rod journal
            // count is stamped in by the crankshaft that gathers them.
            "connecting_rod" => {
                self.context.set_rod(
                    node,
                    RodData {
                        mass: self.number(node, "mass").unwrap_or(0.0),
                        moment_of_inertia: self.number(node, "moment_of_inertia").unwrap_or(0.0),
                        center_of_mass: self.number(node, "center_of_mass").unwrap_or(0.0),
                        length: self.number(node, "length").unwrap_or(0.0),
                        slave_throw: self.number(node, "slave_throw").unwrap_or(0.0),
                    },
                );

                Ok(())
            }

            // `CrankshaftNode::_evaluate`: eight numbers; the journal count is
            // whatever the `add_rod_journal` actions put on it.
            "crankshaft" => {
                self.context.set_crankshaft(
                    node,
                    CrankData {
                        throw: self.number(node, "throw").unwrap_or(0.0),
                        flywheel_mass: self.number(node, "flywheel_mass").unwrap_or(0.0),
                        mass: self.number(node, "mass").unwrap_or(0.0),
                        friction_torque: self.number(node, "friction_torque").unwrap_or(0.0),
                        moment_of_inertia: self.number(node, "moment_of_inertia").unwrap_or(0.0),
                        position_x: self.number(node, "position_x").unwrap_or(0.0),
                        position_y: self.number(node, "position_y").unwrap_or(0.0),
                        tdc: self.number(node, "tdc").unwrap_or(0.0),
                    },
                );

                Ok(())
            }

            // `VehicleNode::_evaluate`: six numbers.
            "vehicle" => {
                self.context.set_vehicle(
                    node,
                    VehicleData {
                        mass: self.number(node, "mass").unwrap_or(0.0),
                        drag_coefficient: self.number(node, "drag_coefficient").unwrap_or(0.0),
                        cross_sectional_area: self
                            .number(node, "cross_sectional_area")
                            .unwrap_or(0.0),
                        diff_ratio: self.number(node, "diff_ratio").unwrap_or(0.0),
                        tire_radius: self.number(node, "tire_radius").unwrap_or(0.0),
                        rolling_resistance: self.number(node, "rolling_resistance").unwrap_or(0.0),
                    },
                );

                Ok(())
            }

            // `RodJournalNode::_evaluate`: one angle; the crankshaft and the rod
            // are stamped onto it by the actions.
            "rod_journal" => {
                let mut journal = *self
                    .context
                    .rod_journal(node)
                    .unwrap_or(&RodJournalData::default());
                journal.angle = self.number(node, "angle").unwrap_or(0.0);
                self.context.set_rod_journal(node, journal);

                Ok(())
            }

            // `AddRodJournalNode::_evaluate`: the journal joins this crankshaft's
            // list, in the order the actions run.
            "add_rod_journal" => {
                if let (Some(crankshaft), Some(journal)) = (
                    self.object_source(node, "crankshaft"),
                    self.object_source(node, "rod_journal"),
                ) {
                    self.context.attach_rod_journal(crankshaft, journal);
                }

                Ok(())
            }

            // `AddCylinderNode::_evaluate`: the bank takes the seven objects and
            // two numbers, in order; the bank's cylinder count is that list.
            "add_cylinder" => {
                let cylinder = CylinderRefs {
                    piston: self.object_source(node, "piston"),
                    rod: self.object_source(node, "connecting_rod"),
                    rod_journal: self.object_source(node, "rod_journal"),
                    intake: self.object_source(node, "intake"),
                    exhaust: self.object_source(node, "exhaust_system"),
                    sound_attenuation: self.number(node, "sound_attenuation").unwrap_or(1.0),
                    primary_length: self.number(node, "primary_length").unwrap_or(0.0),
                };

                if let Some(bank) = self.object_source(node, "cylinder_bank") {
                    self.context.add_cylinder(bank, cylinder);
                }

                Ok(())
            }

            // `SetCylinderHeadNode::_evaluate`: the bank takes one head, and a
            // second one fails the run.
            "set_cylinder_head" => {
                if let (Some(bank), Some(head)) = (
                    self.object_source(node, "bank"),
                    self.object_source(node, "head"),
                ) {
                    if let Err(message) = self.context.set_cylinder_head(bank, head) {
                        let message = message.to_string();
                        self.runtime_error = Some(message.clone());
                        return Err(message);
                    }
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

    /// `FuelNode::generate`: the fuel this node describes, built once. The
    /// turbulence curve is another data node's object, so generating a fuel can
    /// generate a function.
    pub fn fuel(&mut self, node: usize) -> Option<&Fuel> {
        if self.context.fuel(node).is_some() {
            return self.context.fuel(node);
        }

        let turbulence = self
            .object_source(node, "turbulence_to_flame_speed_ratio")
            .and_then(|target| self.function(target).cloned())
            .map(std::sync::Arc::new)
            .or_else(|| Some(std::sync::Arc::new(Function::new(0, 0.0))))?;

        let parameters = FuelParameters {
            molecular_mass: self.number(node, "molecular_mass").unwrap_or(100.0 * G),
            energy_density: self
                .number(node, "energy_density")
                .unwrap_or(48.1 * KJ / (1.0 * G)),
            density: self.number(node, "density").unwrap_or(0.755 * KG / (1.0 * L)),
            turbulence_to_flame_speed_ratio: turbulence,
            molecular_afr: self.number(node, "molecular_afr").unwrap_or(25.0 / 2.0),
            burning_efficiency_randomness: self
                .number(node, "burning_efficiency_randomness")
                .unwrap_or(0.5),
            max_burning_efficiency: self
                .number(node, "max_burning_efficiency")
                .unwrap_or(0.8),
            max_dilution_effect: self.number(node, "max_dilution_effect").unwrap_or(50.0),
            max_turbulence_effect: self.number(node, "max_turbulence_effect").unwrap_or(2.0),
            low_efficiency_attenuation: self
                .number(node, "low_efficiency_attenuation")
                .unwrap_or(0.6),
        };

        Some(self.context.generate_fuel(node, parameters))
    }

    /// The nodes whose class is `name`, in graph order.
    pub fn nodes_of_class(&self, name: &str) -> Vec<usize> {
        self.graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, data)| class_name(&data.builtin_name) == Some(name))
            .map(|(node, _)| node)
            .collect()
    }

    /// The nodes whose class is `function`, in graph order.
    pub fn function_nodes(&self) -> Vec<usize> {
        self.nodes_of_class("function")
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
