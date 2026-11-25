//! `IrCompilationUnit::build`: the node graph `resolveAll` hands to the runtime.
//!
//! Resolution decides *what* every structure means; building decides what
//! executes. `build` walks the unit's node list, `generateNode` turns each
//! structure into a `Node`, and the `NodeOutput` objects a node registers are its
//! wires: an input port is a pointer to a slot that a connection writes through,
//! so the graph is a set of shared objects rather than a tree.
//!
//! Here that is two arenas -- [`GraphNode`] and [`Port`] -- and the same
//! traversal. Node and port identity is what the original's pointer comparisons
//! encode, and it survives as indices.
//!
//! Five behaviours are load-bearing and easy to get wrong, so each is named
//! where it is implemented:
//!
//! * `NodeContainer::addNode` refuses a node the top-level tree already holds,
//!   which is why an instance's container often ends up empty;
//! * a container keeps its outputs in `m_outputReferences`, never `m_outputs`;
//! * `NodeOutput::newInterface` answers only for vector outputs, so interface
//!   nodes do not appear in these graphs at all;
//! * generation order is skeletons, inputs, nested nodes, outputs -- and
//!   generating an input can generate a node, so the order is observable;
//! * a node's `primaryOutput` is how a port gets back to a value, including for
//!   a factory-built class, whose ports come from the class and not from the
//!   IR definition.
//!
//! What is *not* here: `initialize`, `evaluate`, `optimize`, and the node
//! classes' arithmetic. This is the structural half, which is what
//! `tools/parity/graph/*.graph` records; execution needs the same objects plus
//! their `process` overrides.

use crate::context::{ContextArena, ContextId};
use crate::ir::{BinOp, Direction};
use crate::language_rules::LiteralKind;
use crate::lower::{Arena, Kind, StructureId};
use crate::reference::{ReferenceQuery, ReferenceState};

pub type NodeId = usize;
pub type PortId = usize;

/// Which `piranha::Node` subclass a graph node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeClass {
    /// `NodeContainer`: a node instance's scope, holding other nodes.
    Container,
    /// `ChannelNode`: the class behind `__piranha__int` and friends.
    Channel,
    /// `DefaultLiteralNode<T>`: a literal expanded into a node.
    Literal(LiteralKind),
    /// Any other factory-built class whose ports come from the class itself:
    /// operations, conversions, no-ops. `builtin_name` says which.
    Builtin,
}

/// `Node::NodeInputPort`. `connection` is the slot `connectInput` writes
/// through, which the original holds as a `pNodeInput *`.
#[derive(Clone, Debug, Default)]
pub struct InputPort {
    pub name: String,
    pub connection: Option<PortId>,
    /// `NodeInputPort::dependency` -- the node that computes it.
    pub dependency: Option<NodeId>,
    /// `NodeInputPort::nodeInput` -- set when the input is a node.
    pub node_input: Option<NodeId>,
    pub modifies: bool,
    pub enable: bool,
}

/// `Node::NodeOutputPort`.
#[derive(Clone, Debug)]
pub struct OutputPort {
    pub name: String,
    pub port: PortId,
}

/// `Node::NodeOutputPortReference` -- how a container exposes an output.
#[derive(Clone, Debug)]
pub struct OutputReference {
    pub name: String,
    /// `*reference->output`, the connection slot's value.
    pub port: Option<PortId>,
    /// `reference->nodeOutput`.
    pub node_output: Option<NodeId>,
}

/// `Node::PortSkeleton`: where a port's value comes from once the node exists.
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub name: String,
    pub port: Option<PortId>,
    pub definition: StructureId,
    pub context: ContextId,
    pub container: Option<NodeId>,
}

/// One `piranha::Node`.
#[derive(Clone, Debug)]
pub struct GraphNode {
    pub class: NodeClass,
    pub name: String,
    /// `Node::m_builtinName`: set by the factory, and copied from the
    /// definition for interface nodes.
    pub builtin_name: String,
    pub is_virtual: bool,
    pub inputs: Vec<InputPort>,
    pub outputs: Vec<OutputPort>,
    pub output_references: Vec<OutputReference>,
    /// `Node::m_primaryOutput`: the port `generateAliasNode` looks through.
    pub primary_output: String,
    pub ir_structure: Option<StructureId>,
    pub context: Option<ContextId>,
    /// `Node::m_container` -- the container that holds this node.
    pub container: Option<NodeId>,
    /// `Node::m_interfaceInput`: the input slot `getInterfaceInput()` reads.
    /// Only `VectorSplitNode` sets one (`setInterfaceInput(&m_primaryInput)`),
    /// and it is what `generateAliasOutput` falls back to.
    pub interface_input: Option<usize>,
    /// `Node::m_outputReferences`' `registerOutputReference(&m_input, name)`
    /// form: the name of the input the output reads. Upstream stores a
    /// `pNodeInput *`, so the value is read when the output is used, not at
    /// registration time.
    pub slot_output: Option<String>,
    /// `Node::m_dependencyChain`.
    pub dependencies: Vec<NodeId>,
    /// A container's own nodes, in `addNode` order.
    pub nodes: Vec<NodeId>,
    /// A container's child containers, in `addChild` order.
    pub children: Vec<NodeId>,
    pub skeletons: Vec<Skeleton>,
    pub initialized: bool,
}

/// One `piranha::NodeOutput`. Outputs are shared, so identity matters: two
/// inputs wired to the same output point at the same [`PortId`].
#[derive(Clone, Debug)]
pub struct Port {
    /// `NodeOutput::m_parentNode`, set by `registerOutput`.
    pub owner: Option<NodeId>,
    /// `ChannelType::getType()` -- `FloatNodeType`, `IntNodeType`, ...
    pub channel_type: &'static str,
    /// `NodeOutput::m_interface`, from `generateInterface`.
    pub interface: Option<NodeId>,
    /// `NodeOutput::m_dependencyChain`.
    pub dependencies: Vec<NodeId>,
    /// `NodeOutput::m_modifyConnections`.
    pub modify_connections: Vec<NodeId>,
}

/// The program `IrCompilationUnit::build` fills in.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub ports: Vec<Port>,
    /// `NodeProgram::m_containers`: context to container, compared the way
    /// `PKeyValueLookup` compares keys (`IrContextTree::isEqual`).
    pub containers: Vec<(ContextId, NodeId)>,
    pub root: Option<NodeId>,
    pub root_context: Option<ContextId>,
    /// Builtin classes the factory does not have. Upstream
    /// `LanguageRules::generateNode` returns `nullptr` for these too, so the
    /// node is genuinely absent from the graph -- but an absent engine node is
    /// a missing port of the port, not a fact about the script, so it is
    /// recorded instead of being swallowed.
    pub missing_builtins: Vec<String>,
}

impl Graph {
    pub fn node(&self, id: NodeId) -> &GraphNode {
        &self.nodes[id]
    }

    pub fn port(&self, id: PortId) -> &Port {
        &self.ports[id]
    }


    fn new_node(&mut self, class: NodeClass) -> NodeId {
        self.nodes.push(GraphNode {
            class,
            name: String::new(),
            builtin_name: String::new(),
            is_virtual: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
            output_references: Vec::new(),
            primary_output: String::new(),
            ir_structure: None,
            context: None,
            container: None,
            interface_input: None,
            slot_output: None,
            dependencies: Vec::new(),
            nodes: Vec::new(),
            children: Vec::new(),
            skeletons: Vec::new(),
            initialized: false,
        });
        self.nodes.len() - 1
    }

    fn add_port(&mut self, channel_type: &'static str) -> PortId {
        self.ports.push(Port {
            owner: None,
            channel_type,
            interface: None,
            dependencies: Vec::new(),
            modify_connections: Vec::new(),
        });
        self.ports.len() - 1
    }

    /// `NodeProgram::addContainer`: keyed by context, first entry wins.
    fn add_container(&mut self, context: ContextId, container: NodeId, contexts: &ContextArena) {
        if self.container_for(context, contexts).is_some() {
            return;
        }

        self.containers.push((context, container));
    }

    /// `NodeProgram::getContainer`.
    fn container_for(&self, context: ContextId, contexts: &ContextArena) -> Option<NodeId> {
        self.containers
            .iter()
            .find(|(key, _)| contexts.is_equal(*key, context))
            .map(|(_, container)| *container)
    }

    /// `NodeProgram::getCachedInstance(IrParserStructure *, IrContextTree *)`:
    /// the first node generated for this structure in an equal context.
    fn cached_instance(
        &self,
        structure: StructureId,
        context: ContextId,
        contexts: &ContextArena,
    ) -> Option<NodeId> {
        self.nodes.iter().position(|node| {
            node.ir_structure == Some(structure) && contexts.same(node.context, Some(context))
        })
    }

    /// `NodeContainer::addNode` -- and **not** "push onto this container".
    ///
    /// The original starts with `if (getTopLevel()->findNode(node)) return;`, so
    /// a node generated for an instance's input, which the root container
    /// already took, is not added again to the instance's container.
    fn container_add_node(&mut self, container: NodeId, node: NodeId) {
        // The check starts at the *top level*, not at this container: a node the
        // root already holds is not added to a nested container either.
        let top = self.top_level(container);
        if self.find_node_in_tree(top, node) {
            return;
        }

        self.nodes[container].nodes.push(node);
        self.nodes[node].container = Some(container);
    }

    /// `NodeContainer::findNode` -- this container and its children, recursively.
    fn find_node_in_tree(&self, container: NodeId, node: NodeId) -> bool {
        if self.nodes[container].nodes.contains(&node) {
            return true;
        }

        self.nodes[container]
            .children
            .iter()
            .any(|child| self.find_node_in_tree(*child, node))
    }

    /// `NodeContainer::findContainer` -- is `node` this container or below it?
    fn find_container_in_tree(&self, container: NodeId, node: NodeId) -> bool {
        if container == node {
            return true;
        }

        self.nodes[container]
            .children
            .iter()
            .any(|child| self.find_container_in_tree(*child, node))
    }

    /// `NodeContainer::getTopLevel`.
    fn top_level(&self, container: NodeId) -> NodeId {
        match self.nodes[container].container {
            Some(parent) => self.top_level(parent),
            None => container,
        }
    }

    fn register_input(&mut self, node: NodeId, name: &str, modifies: bool, enable: bool) -> usize {
        self.nodes[node].inputs.push(InputPort {
            name: name.to_string(),
            modifies,
            enable,
            ..Default::default()
        });

        self.nodes[node].inputs.len() - 1
    }

    fn register_output(&mut self, node: NodeId, port: PortId, name: &str) {
        self.ports[port].owner = Some(node);
        self.nodes[node].outputs.push(OutputPort { name: name.to_string(), port });
    }

    fn register_output_reference(&mut self, node: NodeId, name: String, port: Option<PortId>, node_output: Option<NodeId>) {
        self.nodes[node].output_references.push(OutputReference { name, port, node_output });
    }

    /// `Node::getOutput` -- outputs first, then output references, then inputs.
    fn get_output(&self, node: NodeId, name: &str) -> Option<PortId> {
        let data = &self.nodes[node];

        if let Some(port) = data
            .outputs
            .iter()
            .find(|port| port.name == name)
            .map(|port| port.port)
        {
            return Some(port);
        }

        if let Some(reference) = data.output_references.iter().find(|entry| entry.name == name) {
            return reference.port;
        }

        if data.slot_output.as_deref() == Some(name) {
            return data.inputs.first().and_then(|input| input.connection);
        }

        data.inputs
            .iter()
            .find(|input| input.name == name)
            .and_then(|input| input.connection)
    }

    /// `Node::getNodeOutput`.
    fn get_node_output(&self, node: NodeId, name: &str) -> Option<NodeId> {
        let data = &self.nodes[node];

        if let Some(reference) = data.output_references.iter().find(|entry| entry.name == name) {
            return match reference.node_output {
                Some(node_output) => Some(node_output),
                None => reference.port.and_then(|port| self.ports[port].interface),
            };
        }

        if let Some(port) = data.outputs.iter().find(|port| port.name == name) {
            return self.ports[port.port].interface;
        }

        if data.slot_output.as_deref() == Some(name) {
            return data
                .inputs
                .first()
                .and_then(|input| input.connection)
                .and_then(|port| self.ports[port].interface);
        }

        data.inputs.iter().find(|input| input.name == name).and_then(|input| {
            match input.node_input {
                Some(node_input) => Some(node_input),
                None => input.connection.and_then(|port| self.ports[port].interface),
            }
        })
    }
}

/// What a builtin class registers: its ports, and what it can be executed as.
///
/// `LanguageRules::generateBuiltinType` looks the name up in
/// `m_builtinRules`, whose `buildNode` is a constructor. The constructors that
/// matter here are the ones `registerBuiltinNodeTypes` installs in
/// `piranha/test/test_rules.cpp`, because those are the names a graph fixture
/// can use. The engine's own library (`__engine_sim__*`) is `es_script`'s and
/// arrives with M4: those classes' ports live in `es_script`'s nodes, not in
/// piranha.
#[derive(Clone, Copy)]
struct ClassSpec {
    class: NodeClass,
    /// `name` -> channel type, in registration order.
    outputs: &'static [(&'static str, &'static str)],
    inputs: &'static [&'static str],
    /// `registerOutputReference` instead of `registerOutput`: the output is a
    /// view of an input rather than a channel of its own (`NoOpNode`).
    output_reference: bool,
    /// `setInterfaceInput` on the first input (`VectorSplitNode`).
    interface_input: bool,
}

const fn spec(
    class: NodeClass,
    outputs: &'static [(&'static str, &'static str)],
    inputs: &'static [&'static str],
) -> ClassSpec {
    ClassSpec {
        class,
        outputs,
        inputs,
        output_reference: false,
        interface_input: false,
    }
}

const OPERATION_FLOAT: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "FloatNodeType")],
    &["__in0", "__in1"],
);

const OPERATION_INT: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "IntNodeType")],
    &["__in0", "__in1"],
);

const OPERATION_STRING: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "StringNodeType")],
    &["__in0", "__in1"],
);

const UNARY_FLOAT: ClassSpec = spec(NodeClass::Builtin, &[("__out", "FloatNodeType")], &["__in"]);

const UNARY_INT: ClassSpec = spec(NodeClass::Builtin, &[("__out", "IntNodeType")], &["__in"]);

const UNARY_BOOL: ClassSpec = spec(NodeClass::Builtin, &[("__out", "BoolNodeType")], &["__in"]);

/// `NoOpNode`: `registerOutputReference(&m_input, "__out")`, so the node has
/// one input, no outputs of its own, and an output reference that reads the
/// input's connection slot.
const NO_OP: ClassSpec = ClassSpec {
    class: NodeClass::Builtin,
    outputs: &[],
    inputs: &["__in"],
    output_reference: true,
    interface_input: false,
};

const CONVERSION_TO_FLOAT: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "FloatNodeType")],
    &["__in"],
);

const CONVERSION_TO_STRING: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "StringNodeType")],
    &["__in"],
);

/// `ChannelNode`: no ports of its own, whatever its channel type.
const CHANNEL: ClassSpec = spec(NodeClass::Channel, &[], &[]);

/// `VectorSplitNode`: four inputs all named `__in`, the first of which is the
/// interface input, and one output per component.
const VECTOR_SPLIT: ClassSpec = ClassSpec {
    class: NodeClass::Builtin,
    outputs: &[
        ("x", "VectorNodeType"),
        ("y", "VectorNodeType"),
        ("z", "VectorNodeType"),
        ("w", "VectorNodeType"),
    ],
    inputs: &["__in", "__in", "__in", "__in"],
    output_reference: false,
    interface_input: true,
};

/// `es_script`'s `ImpulseResponseNode`, the one library class the engine
/// scripts reach before the rest of M4. Its ports are the ones the reference
/// registers (`filename`, `volume` -> `ImpulseResponseChannel`).
const IMPULSE_RESPONSE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "ImpulseResponseChannel")],
    &["filename", "volume"],
);

/// `es_script`'s own node classes: the engine objects and the actions that
/// register them.
///
/// The ports are the ones the reference registers, taken from the graph it
/// builds for `assets/main.mr` -- the same source `tools/parity/builtin_ports.txt`
/// uses for the type checker. What each class *does* is M4; until then a node of
/// one of these classes is structurally right and inert, which is all
/// `IrCompilationUnit::build` asks of it.

const ADD_CRANKSHAFT: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["crankshaft", "engine"],
);

const ADD_CYLINDER: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["connecting_rod", "cylinder_bank", "exhaust_system", "ignition_wire", "intake", "piston", "primary_length", "rod_journal", "sound_attenuation"],
);

const ADD_CYLINDER_BANK: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["cylinder_bank", "engine"],
);

const ADD_GEAR: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["ratio", "transmission"],
);

const ADD_IGNITION_MODULE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["engine", "ignition_module"],
);

const ADD_LOBE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["camshaft", "centerline"],
);

const ADD_ROD_JOURNAL: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["crankshaft", "rod_journal"],
);

const ADD_SAMPLE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["function", "x", "y"],
);

const CAMSHAFT: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "CamshaftChannel")],
    &["advance", "base_radius", "lobe_profile"],
);

const CONNECT_IGNITION_WIRE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["angle", "ignition_module", "wire"],
);

const CONNECTING_ROD: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "ConnectingRodChannel")],
    &["center_of_mass", "length", "mass", "moment_of_inertia", "slave_throw"],
);

const CRANKSHAFT: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "CrankshaftChannel")],
    &["flywheel_mass", "friction_torque", "mass", "moment_of_inertia", "position_x", "position_y", "tdc", "throw"],
);

const CYLINDER_BANK: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "CylinderBankChannel")],
    &["angle", "bore", "deck_height", "display_depth", "position_x", "position_y"],
);

const CYLINDER_HEAD: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "CylinderHeadChannel")],
    &["chamber_volume", "exhaust_port_flow", "exhaust_runner_cross_section_area", "exhaust_runner_volume", "flip_display", "intake_port_flow", "intake_runner_cross_section_area", "intake_runner_volume", "valvetrain"],
);

const DIRECT_THROTTLE_LINKAGE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "ThrottleChannel")],
    &["gamma"],
);

const ENGINE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "EngineChannel")],
    &["dyno_hold_step", "dyno_max_speed", "dyno_min_speed", "fuel", "hf_gain", "jitter", "name", "noise", "redline", "simulation_frequency", "starter_speed", "starter_torque", "throttle"],
);

const EXHAUST_SYSTEM: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "ExhaustSystemChannel")],
    &["audio_volume", "collector_cross_section_area", "impulse_response", "length", "outlet_flow_rate", "primary_flow_rate", "primary_tube_length", "velocity_decay"],
);

const FUEL: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "FuelChannel")],
    &["burning_efficiency_randomness", "density", "energy_density", "low_efficiency_attenuation", "max_burning_efficiency", "max_dilution_effect", "max_turbulence_effect", "molecular_afr", "molecular_mass", "name", "turbulence_to_flame_speed_ratio"],
);

const FUNCTION: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "FunctionChannel")],
    &["filter_radius"],
);

const GENERATE_HARMONIC_CAM_LOBE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["duration_at_50_thou", "function", "gamma", "lift", "steps"],
);

const IGNITION_MODULE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "IgnitionModuleChannel")],
    &["limiter_duration", "rev_limit", "timing_curve"],
);

const IGNITION_WIRE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "IgnitionWireChannel")],
    &[],
);

const INTAKE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "IntakeChannel")],
    &["idle_flow_rate", "idle_throttle_plate_position", "intake_flow_rate", "molecular_afr", "plenum_cross_section_area", "plenum_volume", "runner_flow_rate", "runner_length", "throttle_gamma", "velocity_decay"],
);

const K_28INH2O: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "FloatNodeType")],
    &["flow"],
);

const K_CARB: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "FloatNodeType")],
    &["flow"],
);

const PISTON: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "PistonChannel")],
    &["blowby", "compression_height", "displacement", "mass", "wrist_pin_position"],
);

const ROD_JOURNAL: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "RodJournalChannel")],
    &["angle"],
);

const SET_APPLICATION_SETTINGS: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["boost_units", "color_background", "color_blue", "color_foreground", "color_green", "color_highlight1", "color_highlight2", "color_orange", "color_pink", "color_red", "color_shadow", "color_yellow", "power_units", "pressure_units", "speed_units", "start_fullscreen", "torque_units"],
);

const SET_CYLINDER_HEAD: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["bank", "head"],
);

const SET_ENGINE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["engine"],
);

const SET_TRANSMISSION: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["transmission"],
);

const SET_VEHICLE: ClassSpec = spec(
    NodeClass::Builtin,
    &[],
    &["vehicle"],
);

const STANDARD_VALVETRAIN: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "ValvetrainChannel")],
    &["exhaust_camshaft", "intake_camshaft"],
);

const TRANSMISSION: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "VehicleChannel")],
    &["max_clutch_torque"],
);

const VEHICLE: ClassSpec = spec(
    NodeClass::Builtin,
    &[("__out", "VehicleChannel")],
    &["cross_sectional_area", "diff_ratio", "drag_coefficient", "mass", "rolling_resistance", "tire_radius"],
);


fn class_spec(builtin: &str) -> Option<ClassSpec> {
    // `__<rules>__<class>`, and a rules name may itself contain `__`
    // (`__engine_sim__float_multiply`): the class is everything after the
    // second separator.
    let (_, name) = builtin.strip_prefix("__")?.split_once("__")?;

    Some(match name {
        // `registerBuiltinType<ChannelNode>`
        "int" | "float" | "string" => CHANNEL,

        // `registerBuiltinType<VectorSplitNode>`
        "vector" | "bool" => VECTOR_SPLIT,

        // `DefaultLiteralNode<T>`
        "literal_float" => spec(
            NodeClass::Literal(LiteralKind::Float),
            &[("__out", "FloatNodeType")],
            &[],
        ),
        "literal_int" => spec(
            NodeClass::Literal(LiteralKind::Integer),
            &[("__out", "IntNodeType")],
            &[],
        ),
        "literal_bool" => spec(
            NodeClass::Literal(LiteralKind::Boolean),
            &[("__out", "BoolNodeType")],
            &[],
        ),
        "literal_string" => spec(
            NodeClass::Literal(LiteralKind::String),
            &[("__out", "StringNodeType")],
            &[],
        ),

        // `OperationNodeSpecialized<T, Output>`
        "int_add" | "int_subtract" | "int_multiply" | "int_divide" => OPERATION_INT,
        "float_add" | "float_subtract" | "float_multiply" | "float_divide" => OPERATION_FLOAT,
        "string_add" => OPERATION_STRING,

        // `NumNegateOperationNode<T>`
        "int_negate" => UNARY_INT,
        "float_negate" => UNARY_FLOAT,
        "bool_negate" => UNARY_BOOL,

        // `NoOpNode`
        "int_positive" | "float_positive" => NO_OP,

        // `*ConversionNode`
        "int_to_float" | "string_to_float" => CONVERSION_TO_FLOAT,
        "float_to_string" => CONVERSION_TO_STRING,

        // `es_script`
        "impulse_response" => IMPULSE_RESPONSE,
        "add_crankshaft" => ADD_CRANKSHAFT,
        "add_cylinder" => ADD_CYLINDER,
        "add_cylinder_bank" => ADD_CYLINDER_BANK,
        "add_gear" => ADD_GEAR,
        "add_ignition_module" => ADD_IGNITION_MODULE,
        "add_lobe" => ADD_LOBE,
        "add_rod_journal" => ADD_ROD_JOURNAL,
        "add_sample" => ADD_SAMPLE,
        "camshaft" => CAMSHAFT,
        "connect_ignition_wire" => CONNECT_IGNITION_WIRE,
        "connecting_rod" => CONNECTING_ROD,
        "crankshaft" => CRANKSHAFT,
        "cylinder_bank" => CYLINDER_BANK,
        "cylinder_head" => CYLINDER_HEAD,
        "direct_throttle_linkage" => DIRECT_THROTTLE_LINKAGE,
        "engine" => ENGINE,
        "exhaust_system" => EXHAUST_SYSTEM,
        "fuel" => FUEL,
        "function" => FUNCTION,
        "generate_harmonic_cam_lobe" => GENERATE_HARMONIC_CAM_LOBE,
        "ignition_module" => IGNITION_MODULE,
        "ignition_wire" => IGNITION_WIRE,
        "intake" => INTAKE,
        "k_28inH2O" => K_28INH2O,
        "k_carb" => K_CARB,
        "piston" => PISTON,
        "rod_journal" => ROD_JOURNAL,
        "set_application_settings" => SET_APPLICATION_SETTINGS,
        "set_cylinder_head" => SET_CYLINDER_HEAD,
        "set_engine" => SET_ENGINE,
        "set_transmission" => SET_TRANSMISSION,
        "set_vehicle" => SET_VEHICLE,
        "standard_valvetrain" => STANDARD_VALVETRAIN,
        "transmission" => TRANSMISSION,
        "vehicle" => VEHICLE,


        _ => return None,
    })
}

/// `LanguageRules::generateBuiltinType`: instantiate the class, then
/// `Node::initialize` registers its ports.
fn generate_builtin(graph: &mut Graph, builtin: &str) -> Option<NodeId> {
    let spec = match class_spec(builtin) {
        Some(spec) => spec,
        None => {
            if !graph.missing_builtins.iter().any(|name| name == builtin) {
                graph.missing_builtins.push(builtin.to_string());
            }
            return None;
        }
    };

    let node = graph.new_node(spec.class);
    graph.nodes[node].builtin_name = builtin.to_string();

    for name in spec.inputs {
        graph.register_input(node, name, false, false);
    }

    for (name, channel_type) in spec.outputs {
        let port = graph.add_port(channel_type);
        graph.register_output(node, port, name);

        // Every one of these classes calls `setPrimaryOutput` on the output it
        // registers, and `generateAliasOutput` is how a node is reached from a
        // port, so without it the node has no way back to its value.
        if graph.nodes[node].primary_output.is_empty() {
            graph.nodes[node].primary_output = (*name).to_string();
        }
    }

    if spec.interface_input {
        graph.nodes[node].interface_input = Some(0);
    }

    if spec.output_reference {
        graph.nodes[node].slot_output = Some(spec.inputs[0].to_string());
        graph.nodes[node].primary_output = "__out".to_string();
    }

    Some(node)
}

impl Arena {
    /// `IrCompilationUnit::build(NodeProgram *)`, on the entry unit.
    ///
    /// `load_graph` files units root-first, so `units[0]` is the unit the
    /// original calls `build` on. The context arena lives on the arena (it is
    /// shared with expansion), so this borrows it the way every other phase
    /// does.
    pub fn build_graph(&mut self) -> Graph {
        let mut state = std::mem::take(&mut self.expansion_state);
        let unit = self.units.first().copied();

        let graph = match unit {
            Some(unit) => self.build_graph_of(unit, &mut state),
            None => Graph::default(),
        };

        self.expansion_state = state;
        graph
    }

    /// A fresh root context becomes the program's context for the top-level
    /// container, and every node the unit declared is generated into it.
    fn build_graph_of(&mut self, unit: StructureId, state: &mut ReferenceState) -> Graph {
        let mut graph = Graph::default();

        let root_context = state.contexts.new_root(None, false);
        let root = graph.new_node(NodeClass::Container);
        graph.nodes[root].name = "root".to_string();
        graph.add_container(root_context, root, &state.contexts);
        graph.root = Some(root);
        graph.root_context = Some(root_context);

        let nodes = match self.kind(unit) {
            Kind::Unit(data) => data.nodes.clone(),
            _ => Vec::new(),
        };

        for node in nodes {
            self.generate_node(&mut graph, node, root_context, root, state);
        }

        graph
    }

    /// `IrParserStructure::generateNode`.
    fn generate_node(
        &mut self,
        graph: &mut Graph,
        structure: StructureId,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<NodeId> {
        let node = match graph.cached_instance(structure, context, &state.contexts) {
            Some(node) => node,
            None => self.generate_node_uncached(graph, structure, context, container, state)?,
        };

        Some(self.generate_alias_node(graph, node, state))
    }

    /// `IrParserStructure::_generateNode` and its overrides.
    fn generate_node_uncached(
        &mut self,
        graph: &mut Graph,
        structure: StructureId,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<NodeId> {
        match self.kind(structure).clone() {
            Kind::Node(data) => {
                self.generate_ir_node(graph, structure, &data, context, container, state)
            }

            Kind::AttributeDefinition(data) => {
                // `IrAttributeDefinition::_generateNode`'s interface branch: an
                // output port with a declared type and no value *is* the
                // interface, so the node it belongs to answers with that port's
                // interface -- which `newInterface` provides for vector outputs
                // only.
                if data.type_definition.is_some()
                    && data.direction == Direction::Output
                    && data.default_value.is_none()
                {
                    let scope = state.contexts.context(context)?;
                    let parent = state.contexts.parent(context)?;
                    let owner = self.generate_node(graph, scope, parent, container, state)?;
                    let port = graph.get_node_output(owner, &data.name.data)?;
                    return self.generate_interface(graph, port);
                }

                self.generate_referenced_node(graph, structure, context, container, state)
            }

            // `IrBinaryOperator::_generateNode`'s `.` case: the right operand
            // is a label, and the value is that output *of the left node* --
            // which is why it never reaches `getImmediateReference`.
            Kind::Binary {
                op: BinOp::Dot,
                left,
                right,
                ..
            } => {
                let name = match self.kind(right) {
                    Kind::Label(token) => token.data.clone(),
                    _ => return None,
                };

                let node = self.generate_node(graph, left, context, container, state)?;
                self.generate_node_output_of_node(graph, node, &name, state)
            }

            _ => self.generate_referenced_node(graph, structure, context, container, state),
        }
    }

    /// `IrParserStructure::_generateNode`: resolve the structure to whatever
    /// actually produces a node, then generate that.
    fn generate_referenced_node(
        &mut self,
        graph: &mut Graph,
        structure: StructureId,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<NodeId> {
        let query = ReferenceQuery {
            input_context: Some(context),
            record_errors: false,
            ..Default::default()
        };
        let (reference, info) = self.immediate_reference(structure, &query, state);
        let reference = reference?;
        let reference_context = info.new_context.unwrap_or(context);

        self.generate_node(graph, reference, reference_context, container, state)
    }

    /// `NodeOutput::generateInterface`. `newInterface` is overridden only by
    /// `VectorOutput`, so every other output answers `nullptr` -- which is why
    /// interface nodes do not appear in these graphs.
    fn generate_interface(&mut self, _graph: &mut Graph, _port: PortId) -> Option<NodeId> {
        None
    }

    /// `IrNode::_generateNode`.
    fn generate_ir_node(
        &mut self,
        graph: &mut Graph,
        structure: StructureId,
        data: &crate::lower::NodeData,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<NodeId> {
        // `if (!isBuildEnabled()) return nullptr;`
        if !data.build {
            return None;
        }

        // Find a container: walk up the context tree until one has been
        // registered.
        let mut parent_container = None;
        let mut cursor = Some(context);
        while let Some(current) = cursor {
            if let Some(found) = graph.container_for(current, &state.contexts) {
                parent_container = Some(found);
                break;
            }
            cursor = state.contexts.parent(current);
        }
        let parent_container = parent_container.unwrap_or(container);

        if let Some(cached) = graph.cached_instance(structure, context, &state.contexts) {
            return Some(cached);
        }

        if data.is_interface {
            let scope = self.entry(structure).scope_parent?;
            let node = self.generate_node(graph, scope, context, container, state)?;
            let definition = data.definition?;
            let builtin = self.definition_builtin_name(definition).unwrap_or_default();

            graph.nodes[node].builtin_name = builtin;
            graph.nodes[node].ir_structure = Some(structure);
            graph.nodes[node].context = Some(context);
            graph.container_add_node(parent_container, node);

            return Some(node);
        }

        let definition = data.definition?;
        let new_context = state.contexts.child(context, Some(structure));

        let ports = self.definition_ports(definition);
        let is_builtin = self.definition_is_builtin(definition);
        let is_inline = self.definition_is_inline(definition);

        // The node itself: a container for a user definition, a factory-built
        // node for a builtin one.
        let generated_container;
        let new_container;

        let new_node = if !is_builtin {
            let container_node = graph.new_node(NodeClass::Container);
            graph.nodes[container_node].name = data.name.data.clone();
            graph.nodes[container_node].ir_structure = Some(structure);
            graph.nodes[container_node].context = Some(context);
            graph.nodes[container_node].container = Some(parent_container);

            if !is_inline {
                graph.nodes[parent_container].children.push(container_node);
                graph.add_container(new_context, container_node, &state.contexts);
                new_container = container_node;
            }
            else {
                graph.nodes[container_node].is_virtual = true;
                new_container = parent_container;
            }

            generated_container = Some(container_node);
            container_node
        }
        else {
            let builtin = self.definition_builtin_name(definition).unwrap_or_default();
            if builtin.is_empty() {
                return None;
            }

            let node = generate_builtin(graph, &builtin)?;
            graph.nodes[node].name = data.name.data.clone();
            graph.nodes[node].ir_structure = Some(structure);
            graph.nodes[node].context = Some(context);

            generated_container = None;
            new_container = parent_container;

            node
        };

        // Port skeletons, then inputs, then nested nodes, then outputs. The
        // order is observable: generating an input can generate a node, and the
        // outputs are what make a container's references exist.
        for port_definition in &ports {
            let name = self.attribute_definition_name(*port_definition);
            let (direction, modifies, enable, is_alias) =
                self.attribute_definition_shape(*port_definition);

            let port = if is_builtin { graph.get_output(new_node, &name) } else { None };

            if !is_builtin {
                if direction != Direction::Output {
                    graph.register_input(new_container, &name, modifies, enable);
                }
                else if is_alias {
                    graph.nodes[new_node].primary_output = name.clone();
                }
            }

            graph.nodes[new_node].skeletons.push(Skeleton {
                name,
                port,
                definition: *port_definition,
                context: new_context,
                container: Some(new_container),
            });
        }

        for port_definition in &ports {
            let (direction, ..) = self.attribute_definition_shape(*port_definition);
            if direction == Direction::Output {
                continue;
            }

            let output =
                self.generate_node_output(graph, *port_definition, new_context, new_container, state);
            let node = self.generate_node(graph, *port_definition, new_context, new_container, state);

            if node.is_none() && output.is_none() {
                // `throw EmptyPort()` upstream: the build aborts, so a port with
                // nothing behind it cannot happen in a graph the original
                // produces.
                continue;
            }

            if let Some(node) = node {
                if !graph.nodes[node].is_virtual
                    && !graph.find_container_in_tree(new_container, node)
                {
                    graph.container_add_node(new_container, node);
                }
            }

            let name = self.attribute_definition_name(*port_definition);

            Self::connect_input(graph, new_node, &name, output, node);
        }

        if !graph.nodes[new_node].is_virtual && !graph.find_node_in_tree(parent_container, new_node) {
            graph.container_add_node(parent_container, new_node);
        }

        // Nested nodes.
        if let Some(body) = self.definition_body(definition) {
            let items = match self.kind(body) {
                Kind::NodeList(items) => items.clone(),
                _ => Vec::new(),
            };

            for item in items {
                if let Some(nested) = self.generate_node(graph, item, new_context, new_container, state)
                {
                    graph.container_add_node(new_container, nested);
                }
            }
        }

        for port_definition in &ports {
            let (direction, _, _, is_alias) = self.attribute_definition_shape(*port_definition);
            if direction != Direction::Output {
                continue;
            }

            let output =
                self.generate_node_output(graph, *port_definition, new_context, new_container, state);
            let node = self.generate_node(graph, *port_definition, new_context, new_container, state);

            if node.is_none() && output.is_none() {
                continue;
            }

            // A container's outputs are output *references*: `m_outputs` stays
            // empty and `addContainerOutput` registers the connection point.
            if let Some(container_node) = generated_container {
                if !is_inline {
                    if let Some(output) = output {
                        graph.ports[output].dependencies.push(new_node);
                    }
                    if let Some(node) = node {
                        graph.nodes[node].dependencies.push(new_node);
                    }
                }

                let name = self.attribute_definition_name(*port_definition);
                graph.register_output_reference(container_node, name.clone(), output, node);

                if is_alias {
                    graph.nodes[container_node].primary_output = name;
                }
            }

            if let Some(node) = node {
                if !graph.nodes[node].is_virtual
                    && !graph.find_container_in_tree(new_container, node)
                {
                    graph.container_add_node(new_container, node);
                }
            }

            if let Some(output) = output {
                if let Some(interface) = graph.ports[output].interface {
                    graph.container_add_node(new_container, interface);
                }
            }
        }

        Some(new_node)
    }

    /// `IrParserStructure::generateNodeOutput`.
    fn generate_node_output(
        &mut self,
        graph: &mut Graph,
        structure: StructureId,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<PortId> {
        let node = match graph.cached_instance(structure, context, &state.contexts) {
            Some(node) => Some(node),
            None => self.generate_node(graph, structure, context, container, state),
        };

        if let Some(node) = node {
            return self.generate_alias_output(graph, node, state);
        }

        // `IrAttributeDefinition::_generateNodeOutput`'s interface branch: the
        // output *is* the interface, so the port comes back without a
        // `generateInterface` step -- that one only exists on `_generateNode`.
        let interface_output = match self.kind(structure) {
            Kind::AttributeDefinition(data)
                if data.type_definition.is_some()
                    && data.direction == Direction::Output
                    && data.default_value.is_none() =>
            {
                Some(data.name.data.clone())
            }
            _ => None,
        };

        if let Some(name) = interface_output {
            let owner = state.contexts.context(context)?;
            let parent = state.contexts.parent(context)?;
            let node = self.generate_node(graph, owner, parent, container, state)?;

            return graph.get_output(node, &name);
        }

        // `IrNode::_generateNodeOutput`'s interface branch.
        if let Kind::Node(data) = self.kind(structure) {
            if data.is_interface {
                if !data.build {
                    return None;
                }

                let scope = self.entry(structure).scope_parent?;
                return self.generate_node_output(graph, scope, context, container, state);
            }
        }

        if let Kind::Binary {
            op: BinOp::Dot,
            left,
            right,
            ..
        } = self.kind(structure).clone()
        {
            return self.generate_dot_output(graph, left, right, context, container, state);
        }

        self.generate_referenced_node_output(graph, structure, context, container, state)
    }

    /// `IrBinaryOperator::_generateNodeOutput`'s `.` case.
    fn generate_dot_output(
        &mut self,
        graph: &mut Graph,
        left: StructureId,
        right: StructureId,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<PortId> {
        let Kind::Label(token) = self.kind(right) else {
            return None;
        };
        let name = token.data.clone();

        let node = self.generate_node(graph, left, context, container, state)?;
        self.generate_output(graph, node, &name, state)
    }

    /// `IrParserStructure::_generateNodeOutput`.
    fn generate_referenced_node_output(
        &mut self,
        graph: &mut Graph,
        structure: StructureId,
        context: ContextId,
        container: NodeId,
        state: &mut ReferenceState,
    ) -> Option<PortId> {
        let query = ReferenceQuery {
            input_context: Some(context),
            record_errors: false,
            ..Default::default()
        };
        let (reference, info) = self.immediate_reference(structure, &query, state);
        let reference = reference?;
        let reference_context = info.new_context.unwrap_or(context);

        self.generate_node_output(graph, reference, reference_context, container, state)
    }

    /// `Node::generateAliasOutput`.
    fn generate_alias_output(
        &mut self,
        graph: &mut Graph,
        node: NodeId,
        state: &mut ReferenceState,
    ) -> Option<PortId> {
        let primary = graph.nodes[node].primary_output.clone();

        if let Some(port) = self.generate_output(graph, node, &primary, state) {
            return match graph.ports[port].interface {
                Some(interface) => self.generate_alias_output(graph, interface, state),
                None => Some(port),
            };
        }

        match self.generate_node_output_of_node(graph, node, &primary, state) {
            Some(primary_node) => self.generate_alias_output(graph, primary_node, state),
            // `Node::getInterfaceInput`: a slot only `VectorSplitNode`
            // registers, null for everything else.
            None => graph.nodes[node]
                .interface_input
                .and_then(|index| graph.nodes[node].inputs[index].connection),
        }
    }

    /// `Node::generateAliasNode`.
    fn generate_alias_node(
        &mut self,
        graph: &mut Graph,
        node: NodeId,
        state: &mut ReferenceState,
    ) -> NodeId {
        let primary = graph.nodes[node].primary_output.clone();

        if let Some(port) = self.generate_output(graph, node, &primary, state) {
            return match graph.ports[port].interface {
                Some(interface) => self.generate_alias_node(graph, interface, state),
                None => node,
            };
        }

        match self.generate_node_output_of_node(graph, node, &primary, state) {
            Some(primary_node) => self.generate_alias_node(graph, primary_node, state),
            None => node,
        }
    }

    /// `Node::generateOutput`.
    fn generate_output(
        &mut self,
        graph: &mut Graph,
        node: NodeId,
        name: &str,
        state: &mut ReferenceState,
    ) -> Option<PortId> {
        if name.is_empty() {
            return None;
        }

        let Some(index) = graph.nodes[node]
            .skeletons
            .iter()
            .position(|entry| entry.name == name)
        else {
            // Not a skeleton: a port the node registered itself.
            return graph.get_output(node, name);
        };

        if let Some(port) = graph.nodes[node].skeletons[index].port {
            return Some(port);
        }

        let (definition, context, container) = {
            let skeleton = &graph.nodes[node].skeletons[index];
            (skeleton.definition, skeleton.context, skeleton.container)
        };
        let container = container.unwrap_or(node);

        let new_output = self.generate_node_output(graph, definition, context, container, state);

        if let Some(output) = new_output {
            if let Some(interface) = graph.ports[output].interface {
                graph.container_add_node(container, interface);
            }
        }

        graph.nodes[node].skeletons[index].port = new_output;

        new_output
    }

    /// `Node::generateNodeOutput`.
    fn generate_node_output_of_node(
        &mut self,
        graph: &mut Graph,
        node: NodeId,
        name: &str,
        state: &mut ReferenceState,
    ) -> Option<NodeId> {
        if name.is_empty() {
            return None;
        }

        let Some(index) = graph.nodes[node]
            .skeletons
            .iter()
            .position(|entry| entry.name == name)
        else {
            return graph.get_node_output(node, name);
        };

        if let Some(port) = graph.nodes[node].skeletons[index].port {
            if let Some(interface) = graph.ports[port].interface {
                return Some(interface);
            }
        }

        let (definition, context, container) = {
            let skeleton = &graph.nodes[node].skeletons[index];
            (skeleton.definition, skeleton.context, skeleton.container)
        };
        let container = container.unwrap_or(node);

        let new_node = self.generate_node(graph, definition, context, container, state);

        if let Some(new_node) = new_node {
            if !graph.find_container_in_tree(container, new_node) {
                graph.container_add_node(container, new_node);
            }
        }

        new_node
    }

    /// `Node::connectInput(pNodeInput, const std::string &, Node *, Node *)`.
    ///
    /// Every port with this name is connected -- the original explicitly does
    /// not break out of the loop.
    fn connect_input(
        graph: &mut Graph,
        node: NodeId,
        name: &str,
        connection: Option<PortId>,
        dependency: Option<NodeId>,
    ) {
        let matches: Vec<usize> = graph.nodes[node]
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.name == name)
            .map(|(index, _)| index)
            .collect();

        for index in matches {
            let modifies = graph.nodes[node].inputs[index].modifies;
            graph.nodes[node].inputs[index].connection = connection;
            graph.nodes[node].inputs[index].dependency = dependency;
            graph.nodes[node].inputs[index].node_input = dependency;

            if modifies {
                if let Some(port) = connection {
                    graph.ports[port].modify_connections.push(node);
                }
            }
        }
    }

    /// `IrNodeDefinition::getAttributeDefinitionList` -- the definition's ports
    /// in declaration order.
    fn definition_ports(&self, definition: StructureId) -> Vec<StructureId> {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return Vec::new();
        };

        match self.kind(data.attributes) {
            Kind::AttributeDefinitionList(ports) => ports.clone(),
            _ => Vec::new(),
        }
    }

    /// `IrNodeDefinition::getBuiltinName`.
    fn definition_builtin_name(&self, definition: StructureId) -> Option<String> {
        let Kind::NodeDefinition(data) = self.kind(definition) else {
            return None;
        };

        data.defines_builtin.then(|| data.builtin_name.data.clone())
    }

    /// `IrNodeDefinition::isBuiltin`.
    fn definition_is_builtin(&self, definition: StructureId) -> bool {
        match self.kind(definition) {
            Kind::NodeDefinition(data) => data.defines_builtin,
            _ => false,
        }
    }

    /// `IrNodeDefinition::isInline`.
    fn definition_is_inline(&self, definition: StructureId) -> bool {
        match self.kind(definition) {
            Kind::NodeDefinition(data) => data.is_inline,
            _ => false,
        }
    }

    /// `IrNodeDefinition::getBody`.
    fn definition_body(&self, definition: StructureId) -> Option<StructureId> {
        match self.kind(definition) {
            Kind::NodeDefinition(data) => data.body,
            _ => None,
        }
    }

    /// `IrAttributeDefinition::getName`.
    fn attribute_definition_name(&self, port: StructureId) -> String {
        match self.kind(port) {
            Kind::AttributeDefinition(data) => data.name.data.clone(),
            _ => String::new(),
        }
    }

    /// `isInput`, `modifiesInput`, `toggle` (enable), `isAlias`.
    fn attribute_definition_shape(&self, port: StructureId) -> (Direction, bool, bool, bool) {
        let Kind::AttributeDefinition(data) = self.kind(port) else {
            return (Direction::Input, false, false, false);
        };

        let modifies = data.direction == Direction::Modify;
        (data.direction, modifies, data.direction == Direction::Toggle, data.is_alias)
    }
}

/// The graph in `script_reference --graph`'s record format.
///
/// One record per line, fields comma separated (`-` for a null pointer):
///
/// ```text
/// G,<containerId>,<name>,<parentId>,<nodeCount>,<childCount>
/// CI,<containerId>,<index>,<name>,<modifies>,<enable>
/// CO,<containerId>,<index>,<name>,<targetNode>,<targetPort>,<nodeOutput>
/// N,<containerId>,<nodeId>,<name>,<builtinName>,<virtual>,<inputs>,<outputs>
/// O,<nodeId>,<index>,<name>,<channelType>,<interfaceNode>
/// I,<nodeId>,<index>,<name>,<connectionNode>,<connectionPort>,<dependencyNode>
/// ```
///
/// Ids are traversal order -- a container's own nodes in insertion order, then
/// its children depth first -- so the text describes the graph rather than the
/// allocator. `tools/parity/run_graph.py` compares this against recordings the
/// original produced.
pub fn dump(graph: &Graph) -> Vec<String> {
    let mut out = Vec::new();

    let Some(root) = graph.root else {
        return out;
    };

    let mut containers = vec![None; graph.nodes.len()];
    let mut nodes = vec![None; graph.nodes.len()];
    assign(graph, root, &mut containers, &mut nodes, &mut 0, &mut 0);

    dump_container(graph, root, None, &containers, &nodes, &mut out);
    out
}

fn node_id(nodes: &[Option<usize>], node: Option<usize>) -> String {
    match node.and_then(|node| nodes[node]) {
        Some(id) => id.to_string(),
        None => "-".to_string(),
    }
}

/// `GraphDumper::assign`: container ids pre-order, node ids in traversal order.
fn assign(
    graph: &Graph,
    container: usize,
    containers: &mut Vec<Option<usize>>,
    nodes: &mut Vec<Option<usize>>,
    next_container: &mut usize,
    next_node: &mut usize,
) {
    containers[container] = Some(*next_container);
    *next_container += 1;

    for node in &graph.node(container).nodes {
        nodes[*node] = Some(*next_node);
        *next_node += 1;
    }

    for child in &graph.node(container).children {
        assign(graph, *child, containers, nodes, next_container, next_node);
    }
}

/// `Node::getOutputName` -- the name the owning node gave an output.
fn port_name(graph: &Graph, owner: Option<usize>, port: Option<usize>) -> String {
    let (Some(owner), Some(port)) = (owner, port) else {
        return "-".to_string();
    };

    graph
        .node(owner)
        .outputs
        .iter()
        .find(|entry| entry.port == port)
        .map(|entry| entry.name.clone())
        .unwrap_or_default()
}

/// `GraphDumper::dump` in `script_reference.cpp`, record for record.
fn dump_container(
    graph: &Graph,
    container: usize,
    parent: Option<i64>,
    containers: &[Option<usize>],
    nodes: &[Option<usize>],
    out: &mut Vec<String>,
) {
    let data: &GraphNode = graph.node(container);
    let id = containers[container].unwrap_or(0);

    out.push(format!(
        "G,{},{},{},{},{}",
        id,
        data.name,
        match parent {
            Some(parent) => parent.to_string(),
            None => "-1".to_string(),
        },
        data.nodes.len(),
        data.children.len()
    ));

    for (index, input) in data.inputs.iter().enumerate() {
        out.push(format!(
            "CI,{},{},{},{},{}",
            id,
            index,
            input.name,
            if input.modifies { 1 } else { 0 },
            if input.enable { 1 } else { 0 }
        ));
    }

    for (index, reference) in data.output_references.iter().enumerate() {
        let owner = reference.port.and_then(|port| graph.port(port).owner);
        out.push(format!(
            "CO,{},{},{},{},{},{}",
            id,
            index,
            reference.name,
            node_id(nodes, owner),
            port_name(graph, owner, reference.port),
            node_id(nodes, reference.node_output)
        ));
    }

    for node in &data.nodes {
        let node_data = graph.node(*node);
        let node_number = nodes[*node].unwrap_or(0);

        out.push(format!(
            "N,{},{},{},{},{},{},{}",
            id,
            node_number,
            node_data.name,
            node_data.builtin_name,
            if node_data.is_virtual { 1 } else { 0 },
            node_data.inputs.len(),
            node_data.outputs.len()
        ));

        for (index, output) in node_data.outputs.iter().enumerate() {
            let port = graph.port(output.port);
            out.push(format!(
                "O,{},{},{},{},{}",
                node_number,
                index,
                output.name,
                port.channel_type,
                node_id(nodes, port.interface)
            ));
        }

        for (index, input) in node_data.inputs.iter().enumerate() {
            let owner = input.connection.and_then(|port| graph.port(port).owner);
            out.push(format!(
                "I,{},{},{},{},{},{}",
                node_number,
                index,
                input.name,
                node_id(nodes, owner),
                port_name(graph, owner, input.connection),
                node_id(nodes, input.dependency)
            ));
        }
    }

    for child in &data.children {
        dump_container(graph, *child, Some(id as i64), containers, nodes, out);
    }
}

