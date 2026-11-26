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
    ApplicationSettings, CrankData, CylinderRefs, EngineContext, PistonData, RodData,
    RodJournalData, VehicleData,
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
    /// `Compiler::output()->applicationSettings`, written by the settings action.
    pub settings: ApplicationSettings,
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
            settings: ApplicationSettings::default(),
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

        // The top level's nodes, in order. A container node reaches its own
        // nodes through `_evaluate` (see `run`), and every other container is
        // reached through the *dependency* of an input, so this is the whole
        // walk -- there is no second pass over containers that "nothing ran".
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
    fn run(&mut self, node: usize) -> Result<(), String> {
        let data = self.graph.node(node);

        match data.class {
            // `NodeContainer::_evaluate`: a container node *is* the container it
            // owns -- `aContainerNode.nodes` are the nodes inside it -- so running
            // it runs them, in order, and a failure anywhere in them fails it.
            NodeClass::Container => {
                for node in self.graph.node(node).nodes.clone() {
                    self.evaluate(node)?;
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
                    let index = self.context.add_cylinder(bank, cylinder);

                    // `CylinderBankNode::addCylinder` ends with
                    // `wire->connect(this, getCylinderCount() - 1)`.
                    if let Some(wire) = self.object_source(node, "ignition_wire") {
                        self.context.connect_wire(wire, bank, index);
                    }
                }

                Ok(())
            }

            // `CylinderBankNode::_evaluate`: six numbers; the cylinder count and
            // the index are `buildEngine`'s.
            "cylinder_bank" => {
                let mut bank = crate::objects::BankData {
                    angle: self.number(node, "angle").unwrap_or(0.0),
                    bore: self.number(node, "bore").unwrap_or(0.0),
                    deck_height: self.number(node, "deck_height").unwrap_or(0.0),
                    position_x: self.number(node, "position_x").unwrap_or(0.0),
                    position_y: self.number(node, "position_y").unwrap_or(0.0),
                    display_depth: self.number(node, "display_depth").unwrap_or(0.0),
                    ..Default::default()
                };

                // The cylinders and the head survive a re-evaluation.
                if let Some(existing) = self.context.bank(node) {
                    bank.cylinders = existing.cylinders.clone();
                    bank.head = existing.head;
                }

                self.context.set_bank(node, bank);

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

            // `SetApplicationSettingsNode::_evaluate`: `readAllInputs` then the
            // whole struct replaces the output's, so an unconnected input keeps
            // the header's default.
            "set_application_settings" => {
                let mut settings = ApplicationSettings::default();

                if let Some(value) = self.text(node, "power_units") {
                    settings.power_units = value;
                }
                if let Some(value) = self.text(node, "torque_units") {
                    settings.torque_units = value;
                }
                if let Some(value) = self.text(node, "speed_units") {
                    settings.speed_units = value;
                }
                if let Some(value) = self.text(node, "pressure_units") {
                    settings.pressure_units = value;
                }
                if let Some(value) = self.text(node, "boost_units") {
                    settings.boost_units = value;
                }

                settings.start_fullscreen = matches!(
                    self.input(node, "start_fullscreen"),
                    Some(Value::Bool(true))
                );

                let colors = [
                    ("color_background", 0x0E1012),
                    ("color_foreground", 0xFFFFFF),
                    ("color_shadow", 0x0E1012),
                    ("color_highlight1", 0xEF4545),
                    ("color_highlight2", 0xFFFFFF),
                    ("color_pink", 0xF394BE),
                    ("color_red", 0xEE4445),
                    ("color_orange", 0xF4802A),
                    ("color_yellow", 0xFDBD2E),
                    ("color_blue", 0x77CEE0),
                    ("color_green", 0xBDD869),
                ];

                for (name, default) in colors {
                    if let Some(value) = self.int(node, name) {
                        settings.set_color(name, value);
                    }
                    else {
                        settings.set_color(name, default);
                    }
                }

                self.settings = settings;

                Ok(())
            }

            // The data nodes. Each copies its inputs into the parameters its
            // `generate` will hand to the part, so nothing is computed yet.
            "intake" => {
                let mut intake = crate::objects::IntakeData::default();

                for (port, slot) in [
                    ("plenum_volume", &mut intake.volume),
                    ("plenum_cross_section_area", &mut intake.cross_section_area),
                    ("intake_flow_rate", &mut intake.input_flow_k),
                    ("idle_flow_rate", &mut intake.idle_flow_k),
                    ("runner_flow_rate", &mut intake.runner_flow_rate),
                    ("molecular_afr", &mut intake.molecular_afr),
                    ("idle_throttle_plate_position", &mut intake.idle_throttle_plate_position),
                    ("runner_length", &mut intake.runner_length),
                    ("velocity_decay", &mut intake.velocity_decay),
                ] {
                    if let Some(value) = self.number(node, port) {
                        *slot = value;
                    }
                }

                self.context.intake_mut(node).clone_from(&intake);

                Ok(())
            }

            "exhaust_system" => {
                let mut exhaust = crate::objects::ExhaustData::default();

                for (port, slot) in [
                    ("length", &mut exhaust.length),
                    (
                        "collector_cross_section_area",
                        &mut exhaust.collector_cross_section_area,
                    ),
                    ("outlet_flow_rate", &mut exhaust.outlet_flow_rate),
                    ("primary_tube_length", &mut exhaust.primary_tube_length),
                    ("primary_flow_rate", &mut exhaust.primary_flow_rate),
                    ("audio_volume", &mut exhaust.audio_volume),
                    ("velocity_decay", &mut exhaust.velocity_decay),
                ] {
                    if let Some(value) = self.number(node, port) {
                        *slot = value;
                    }
                }

                exhaust.impulse_response = self.object_source(node, "impulse_response");
                self.context.exhaust_mut(node).clone_from(&exhaust);

                Ok(())
            }

            // `ImpulseResponseNode`: the one `es_script` class that reads the
            // filesystem, and it reads it late -- `generate` resolves the
            // filename against the directory of the unit that wrote the node.
            "impulse_response" => {
                let mut response = crate::objects::ImpulseResponseData::default();

                if let Some(filename) = self.text(node, "filename") {
                    let unit_path = self
                        .graph
                        .nodes
                        .get(node)
                        .and_then(|node| node.unit_path.as_deref());

                    response.filename = crate::paths::resolve(unit_path, &filename);
                }

                if let Some(volume) = self.number(node, "volume") {
                    response.volume = volume;
                }

                self.context.set_impulse_response(node, response);

                Ok(())
            }

            // `CylinderHeadNode::_evaluate` reads its inputs and then clears the
            // four object pointers, so only the numbers survive on the node.
            "cylinder_head" => {
                let mut head = crate::objects::HeadData::default();

                for (port, slot) in [
                    ("chamber_volume", &mut head.combustion_chamber_volume),
                    (
                        "intake_runner_volume",
                        &mut head.intake_runner_volume,
                    ),
                    (
                        "intake_runner_cross_section_area",
                        &mut head.intake_runner_cross_section_area,
                    ),
                    (
                        "exhaust_runner_volume",
                        &mut head.exhaust_runner_volume,
                    ),
                    (
                        "exhaust_runner_cross_section_area",
                        &mut head.exhaust_runner_cross_section_area,
                    ),
                ] {
                    if let Some(value) = self.number(node, port) {
                        *slot = value;
                    }
                }

                head.flip_display = self.number(node, "flip_display").map(|v| v != 0.0).unwrap_or(false);
                head.intake_port_flow = self.object_source(node, "intake_port_flow");
                head.exhaust_port_flow = self.object_source(node, "exhaust_port_flow");
                head.valvetrain = self.object_source(node, "valvetrain");

                self.context.head_mut(node).clone_from(&head);

                Ok(())
            }

            // `IgnitionModuleNode`: its parameters, plus the posts the wire
            // actions hang on it. `generate` is what turns those into a firing
            // order, so nothing is resolved here.
            "ignition_module" => {
                let mut module = crate::objects::IgnitionModuleData::default();

                module.timing_curve = self.object_source(node, "timing_curve");
                module.rev_limit = self.number(node, "rev_limit").unwrap_or(module.rev_limit);
                module.limiter_duration = self
                    .number(node, "limiter_duration")
                    .unwrap_or(module.limiter_duration);

                self.context.ignition_module_mut(node).clone_from(&module);

                Ok(())
            }

            // `ConnectIgnitionWireNode`? The wire posts belong to the module.
            "connect_ignition_wire" => {
                if let (Some(module), Some(wire)) = (
                    self.object_source(node, "ignition_module"),
                    self.object_source(node, "wire"),
                ) {
                    let angle = self.number(node, "angle").unwrap_or(0.0);
                    self.context.ignition_module_mut(module).posts.push((wire, angle));
                }

                Ok(())
            }

            // `CamshaftNode::_evaluate`: two numbers and the lobe profile; the
            // lobes themselves are `add_lobe`'s.
            "camshaft" => {
                let mut camshaft = crate::objects::CamshaftData {
                    advance: self.number(node, "advance").unwrap_or(0.0),
                    base_radius: self.number(node, "base_radius").unwrap_or(0.0),
                    lobe_profile: self.object_source(node, "lobe_profile"),
                    lobes: Vec::new(),
                };

                camshaft.lobes = self
                    .context
                    .camshaft(node)
                    .map(|camshaft| camshaft.lobes.clone())
                    .unwrap_or_default();

                self.context.camshaft_mut(node).clone_from(&camshaft);

                Ok(())
            }

            // `AddLobeNode::_evaluate`: one more centreline, in order.
            "add_lobe" => {
                if let Some(camshaft) = self.object_source(node, "camshaft") {
                    let centerline = self.number(node, "centerline").unwrap_or(0.0);
                    self.context.camshaft_mut(camshaft).lobes.push(centerline);
                }

                Ok(())
            }

            "standard_valvetrain" => {
                self.context.set_valvetrain(
                    node,
                    crate::objects::ValvetrainData::Standard {
                        intake_camshaft: self.object_source(node, "intake_camshaft"),
                        exhaust_camshaft: self.object_source(node, "exhaust_camshaft"),
                    },
                );

                Ok(())
            }

            "vtec_valvetrain" => {
                self.context.set_valvetrain(
                    node,
                    crate::objects::ValvetrainData::Vtec {
                        intake_camshaft: self.object_source(node, "intake_camshaft"),
                        exhaust_camshaft: self.object_source(node, "exhaust_camshaft"),
                        vtec_intake_camshaft: self.object_source(node, "vtec_intake_camshaft"),
                        vtec_exhaust_camshaft: self.object_source(node, "vtec_exhaust_camshaft"),
                        min_rpm: self.number(node, "min_rpm").unwrap_or(0.0),
                        min_speed: self.number(node, "min_speed").unwrap_or(0.0),
                        min_throttle_position: self
                            .number(node, "min_throttle_position")
                            .unwrap_or(0.0),
                        manifold_vacuum: self.number(node, "manifold_vacuum").unwrap_or(0.0),
                    },
                );

                Ok(())
            }

            "direct_throttle_linkage" => {
                self.context.set_throttle(
                    node,
                    crate::objects::ThrottleData {
                        gamma: self.number(node, "gamma").unwrap_or(0.0),
                    },
                );

                Ok(())
            }

            // `AddSlaveJournalNode::_evaluate`: a journal hung on a rod rather
            // than on the crankshaft, which is what a shared crank pin looks
            // like.
            "add_slave_journal" => {
                if let (Some(rod), Some(journal)) = (
                    self.object_source(node, "rod"),
                    self.object_source(node, "rod_journal"),
                ) {
                    self.context.attach_slave_journal(rod, journal);
                }

                Ok(())
            }

            // `AddCylinderBankNode`/`AddCrankshaftNode`: the engine node gathers
            // what it will build from, in the order the actions run.
            "add_cylinder_bank" => {
                if let (Some(engine), Some(bank)) = (
                    self.object_source(node, "engine"),
                    self.object_source(node, "cylinder_bank"),
                ) {
                    self.context.engine_mut(engine).cylinder_banks.push(bank);
                }

                Ok(())
            }

            "add_crankshaft" => {
                if let (Some(engine), Some(crankshaft)) = (
                    self.object_source(node, "engine"),
                    self.object_source(node, "crankshaft"),
                ) {
                    self.context.engine_mut(engine).crankshafts.push(crankshaft);
                }

                Ok(())
            }

            "add_ignition_module" => {
                if let (Some(engine), Some(module)) = (
                    self.object_source(node, "engine"),
                    self.object_source(node, "ignition_module"),
                ) {
                    self.context.engine_mut(engine).ignition_module = Some(module);
                }

                Ok(())
            }

            // `EngineNode::_evaluate` reads its inputs; `set_engine` is what makes
            // the engine exist, so this is where the parameters are captured and
            // the engine is published.
            "set_engine" => {
                if let Some(target) = self.object_source(node, "engine") {
                    let mut engine = self.context.engine(target).cloned().unwrap_or_default();

                    if let Some(value) = self.input(target, "name") {
                        engine.name = value.text();
                    }

                    // Every remaining input is optional: an unconnected one keeps
                    // the `Engine::Parameters` default the struct was born with.
                    for (port, slot) in [
                        ("starter_torque", &mut engine.starter_torque),
                        ("starter_speed", &mut engine.starter_speed),
                        ("redline", &mut engine.redline),
                        ("dyno_min_speed", &mut engine.dyno_min_speed),
                        ("dyno_max_speed", &mut engine.dyno_max_speed),
                        ("dyno_hold_step", &mut engine.dyno_hold_step),
                    ] {
                        if let Some(value) = self.number(target, port) {
                            *slot = value;
                        }
                    }

                    engine.initial_simulation_frequency = self
                        .number(target, "simulation_frequency")
                        .unwrap_or(engine.initial_simulation_frequency);
                    engine.initial_high_frequency_gain =
                        self.number(target, "hf_gain").unwrap_or(engine.initial_high_frequency_gain);
                    engine.initial_jitter =
                        self.number(target, "jitter").unwrap_or(engine.initial_jitter);
                    engine.initial_noise =
                        self.number(target, "noise").unwrap_or(engine.initial_noise);

                    engine.fuel = self.object_source(target, "fuel");
                    engine.throttle = self.object_source(target, "throttle");
                    engine.published = true;

                    *self.context.engine_mut(target) = engine;
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

    fn text(&self, node: usize, name: &str) -> Option<String> {
        match self.input(node, name)? {
            Value::Text(value) => Some(value),
            _ => None,
        }
    }

    fn int(&self, node: usize, name: &str) -> Option<i32> {
        match self.input(node, name)? {
            Value::Int(value) => Some(value),
            _ => None,
        }
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

        // The original dereferences this pointer without checking it, so a fuel
        // with no curve through it is undefined behaviour there; here it is a
        // failed run, which is the closest thing to a crash that a library can
        // report and is far better than quietly feeding the engine a flat curve.
        let turbulence = self
            .object_source(node, "turbulence_to_flame_speed_ratio")
            .and_then(|target| self.function(target).cloned())
            .map(std::sync::Arc::new);

        let Some(turbulence) = turbulence else {
            self.runtime_error = Some("fuel has no turbulence curve".to_string());
            return None;
        };

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
