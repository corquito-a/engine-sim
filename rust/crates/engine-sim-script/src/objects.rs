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
/// `ApplicationSettings`: what `set_application_settings` writes, and the only
/// script state that is not an object graph. The defaults are the header's.
#[derive(Clone, Debug)]
pub struct ApplicationSettings {
    pub start_fullscreen: bool,
    pub power_units: String,
    pub torque_units: String,
    pub speed_units: String,
    pub pressure_units: String,
    pub boost_units: String,
    pub color_background: i32,
    pub color_foreground: i32,
    pub color_shadow: i32,
    pub color_highlight1: i32,
    pub color_highlight2: i32,
    pub color_pink: i32,
    pub color_red: i32,
    pub color_orange: i32,
    pub color_yellow: i32,
    pub color_blue: i32,
    pub color_green: i32,
}

impl ApplicationSettings {
    /// The field `set_application_settings` names, by input name.
    pub fn set_color(&mut self, name: &str, value: i32) {
        match name {
            "color_background" => self.color_background = value,
            "color_foreground" => self.color_foreground = value,
            "color_shadow" => self.color_shadow = value,
            "color_highlight1" => self.color_highlight1 = value,
            "color_highlight2" => self.color_highlight2 = value,
            "color_pink" => self.color_pink = value,
            "color_red" => self.color_red = value,
            "color_orange" => self.color_orange = value,
            "color_yellow" => self.color_yellow = value,
            "color_blue" => self.color_blue = value,
            "color_green" => self.color_green = value,
            _ => {}
        }
    }
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self {
            start_fullscreen: false,
            power_units: "hp".to_string(),
            torque_units: "lb-ft".to_string(),
            speed_units: "mph".to_string(),
            pressure_units: "inHg".to_string(),
            boost_units: "psi".to_string(),
            color_background: 0x0E1012,
            color_foreground: 0xFFFFFF,
            color_shadow: 0x0E1012,
            color_highlight1: 0xEF4545,
            color_highlight2: 0xFFFFFF,
            color_pink: 0xF394BE,
            color_red: 0xEE4445,
            color_orange: 0xF4802A,
            color_yellow: 0xFDBD2E,
            color_blue: 0x77CEE0,
            color_green: 0xBDD869,
        }
    }
}

#[derive(Default)]
pub struct EngineContext {
    functions: BTreeMap<usize, Function>,
    fuels: BTreeMap<usize, Fuel>,
    pistons: BTreeMap<usize, PistonData>,
    rods: BTreeMap<usize, RodData>,
    crankshafts: BTreeMap<usize, CrankData>,
    vehicles: BTreeMap<usize, VehicleData>,
    rod_journals: BTreeMap<usize, RodJournalData>,
    crankshaft_journals: BTreeMap<usize, Vec<usize>>,
    /// `CylinderHeadNode::m_bank`.
    head_banks: BTreeMap<usize, usize>,
    banks: BTreeMap<usize, BankData>,
}

/// `RodJournalNode`'s `m_angle`, plus the crankshaft and rod the actions stamp
/// onto it.
#[derive(Clone, Copy, Debug, Default)]
pub struct RodJournalData {
    pub angle: f64,
    pub crankshaft: Option<usize>,
    pub rod: Option<usize>,
}

/// One `CylinderBankNode::Cylinder`: the nodes `add_cylinder` handed the bank,
/// in the order it took them.
#[derive(Clone, Copy, Debug)]
pub struct CylinderRefs {
    pub piston: Option<usize>,
    pub rod: Option<usize>,
    pub rod_journal: Option<usize>,
    pub intake: Option<usize>,
    pub exhaust: Option<usize>,
    pub sound_attenuation: f64,
    pub primary_length: f64,
}

/// `CylinderBankNode`'s own state: its cylinders, and the head `set_cylinder_head`
/// gave it. The six `CylinderBank::Parameters` numbers stay on the node.
#[derive(Clone, Debug, Default)]
pub struct BankData {
    pub cylinders: Vec<CylinderRefs>,
    pub head: Option<usize>,
}

/// `VehicleNode`'s `Vehicle::Parameters`: six numbers, and nothing stamped in
/// later -- `generate` only calls `initialize`.
#[derive(Clone, Copy, Debug, Default)]
pub struct VehicleData {
    pub mass: f64,
    pub drag_coefficient: f64,
    pub cross_sectional_area: f64,
    pub diff_ratio: f64,
    pub tire_radius: f64,
    pub rolling_resistance: f64,
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

    pub fn vehicle(&self, node: usize) -> Option<&VehicleData> {
        self.vehicles.get(&node)
    }

    pub fn set_vehicle(&mut self, node: usize, vehicle: VehicleData) {
        self.vehicles.insert(node, vehicle);
    }

    pub fn rod_journal(&self, node: usize) -> Option<&RodJournalData> {
        self.rod_journals.get(&node)
    }

    pub fn set_rod_journal(&mut self, node: usize, journal: RodJournalData) {
        self.rod_journals.insert(node, journal);
    }

    /// `RodJournalNode::setAngle`/`CrankshaftNode::addRodJournal`'s stamping:
    /// a journal learns its crankshaft, and the crankshaft keeps the list.
    pub fn attach_rod_journal(&mut self, crankshaft: usize, journal: usize) {
        let entry = self.rod_journals.entry(journal).or_default();
        entry.crankshaft = Some(crankshaft);
        self.crankshaft_journals.entry(crankshaft).or_default().push(journal);
    }

    pub fn crankshaft_journals(&self, crankshaft: usize) -> &[usize] {
        self.crankshaft_journals
            .get(&crankshaft)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn bank(&self, node: usize) -> Option<&BankData> {
        self.banks.get(&node)
    }

    /// `CylinderBankNode::addCylinder`
    pub fn add_cylinder(&mut self, bank: usize, cylinder: CylinderRefs) -> usize {
        let entry = self.banks.entry(bank).or_default();
        entry.cylinders.push(cylinder);
        entry.cylinders.len() - 1
    }

    /// `CylinderBankNode::setCylinderHead` plus `CylinderHeadNode::setBank`: a
    /// bank takes one head, and a head belongs to one bank. The caller gets the
    /// message the original throws.
    pub fn set_cylinder_head(&mut self, bank: usize, head: usize) -> Result<(), &'static str> {
        if self.banks.get(&bank).and_then(|bank| bank.head).is_some() {
            return Err("Cylinder bank already has a head");
        }

        if self.head_banks.contains_key(&head) {
            return Err("Cylinder head already attached to a bank");
        }

        self.banks.entry(bank).or_default().head = Some(head);
        self.head_banks.insert(head, bank);
        Ok(())
    }

    /// The nodes that generated an object, in order.
    pub fn function_nodes(&self) -> Vec<usize> {
        self.functions.keys().copied().collect()
    }
}
