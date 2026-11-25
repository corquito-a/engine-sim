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

use engine_sim_core::fuel::{Fuel, FuelParameters};
use engine_sim_core::function::Function;

use std::collections::BTreeMap;

/// `es_script::EngineContext`, restricted to what has been ported: one cache per
/// object type, keyed by the node that generates it.
#[derive(Default)]
pub struct EngineContext {
    functions: BTreeMap<usize, Function>,
    fuels: BTreeMap<usize, Fuel>,
    pistons: BTreeMap<usize, PistonData>,
    rods: BTreeMap<usize, RodData>,
    crankshafts: BTreeMap<usize, CrankData>,
}

/// `ConnectingRodNode`'s `ConnectingRod::Parameters`, minus the pointers it
/// stamps in when it generates.
#[derive(Clone, Copy, Debug, Default)]
pub struct RodData {
    pub mass: f64,
    pub moment_of_inertia: f64,
    pub center_of_mass: f64,
    pub length: f64,
    pub slave_throw: f64,
}

/// `CrankshaftNode`'s `Crankshaft::Parameters`, minus the journal angles the
/// rod journal nodes contribute.
#[derive(Clone, Copy, Debug, Default)]
pub struct CrankData {
    pub throw: f64,
    pub flywheel_mass: f64,
    pub mass: f64,
    pub friction_torque: f64,
    pub moment_of_inertia: f64,
    pub position_x: f64,
    pub position_y: f64,
    pub tdc: f64,
}

/// `PistonNode`'s `Piston::Parameters`, minus the pointers it stamps in when it
/// generates: a piston node *holds* these five numbers, and the `Piston` itself
/// is made by the action that adds a cylinder to a bank.
#[derive(Clone, Copy, Debug, Default)]
pub struct PistonData {
    pub mass: f64,
    pub blowby_flow_coefficient: f64,
    pub compression_height: f64,
    pub wrist_pin_position: f64,
    pub displacement: f64,
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

    /// `EngineContext::getFuel`
    pub fn fuel(&self, node: usize) -> Option<&Fuel> {
        self.fuels.get(&node)
    }

    /// `FuelNode::generate`: the caller owns the `Fuel` in the original (`Fuel`
    /// is a stack object there); here the context keeps it, so the same node
    /// always answers with the same fuel.
    pub fn generate_fuel(&mut self, node: usize, parameters: FuelParameters) -> &Fuel {
        self.fuels.entry(node).or_insert_with(|| Fuel::new(parameters))
    }

    /// `PistonNode::_evaluate`'s `m_parameters`, once the inputs are read.
    pub fn piston(&self, node: usize) -> Option<&PistonData> {
        self.pistons.get(&node)
    }

    pub fn set_piston(&mut self, node: usize, piston: PistonData) {
        self.pistons.insert(node, piston);
    }

    pub fn rod(&self, node: usize) -> Option<&RodData> {
        self.rods.get(&node)
    }

    pub fn set_rod(&mut self, node: usize, rod: RodData) {
        self.rods.insert(node, rod);
    }

    pub fn crankshaft(&self, node: usize) -> Option<&CrankData> {
        self.crankshafts.get(&node)
    }

    pub fn set_crankshaft(&mut self, node: usize, crankshaft: CrankData) {
        self.crankshafts.insert(node, crankshaft);
    }

    /// The nodes that generated an object, in order.
    pub fn function_nodes(&self) -> Vec<usize> {
        self.functions.keys().copied().collect()
    }
}
