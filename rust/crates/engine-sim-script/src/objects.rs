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

/// `EngineNode`'s `Engine::Parameters` and the two lists the actions fill: what
/// `set_engine` publishes, before any part exists.
#[derive(Clone, Debug)]
pub struct EngineData {
    pub name: String,
    pub starter_torque: f64,
    pub starter_speed: f64,
    pub redline: f64,
    pub dyno_min_speed: f64,
    pub dyno_max_speed: f64,
    pub dyno_hold_step: f64,
    pub initial_simulation_frequency: f64,
    pub initial_high_frequency_gain: f64,
    pub initial_jitter: f64,
    pub initial_noise: f64,

    /// `Engine::m_displacement`. Only `Engine::calculateDisplacement` fills it
    /// in, and that is the application's call after the script has run, so a
    /// script that has just configured the engine reports zero.
    pub displacement: f64,

    pub fuel: Option<usize>,
    pub throttle: Option<usize>,
    pub ignition_module: Option<usize>,

    pub cylinder_banks: Vec<usize>,
    pub crankshafts: Vec<usize>,

    /// `Compiler::output()->engine`, set by `set_engine` (`None` before that).
    pub published: bool,
}

impl Default for EngineData {
    fn default() -> Self {
        Self {
            name: String::new(),
            starter_torque: engine_sim_core::units::torque(90.0, engine_sim_core::units::FT_LB),
            starter_speed: engine_sim_core::units::rpm(200.0),
            redline: engine_sim_core::units::rpm(6500.0),
            dyno_min_speed: engine_sim_core::units::rpm(1000.0),
            dyno_max_speed: engine_sim_core::units::rpm(6500.0),
            dyno_hold_step: engine_sim_core::units::rpm(100.0),
            initial_simulation_frequency: 0.0,
            initial_high_frequency_gain: 0.0,
            initial_jitter: 0.0,
            initial_noise: 0.0,
            displacement: 0.0,
            fuel: None,
            throttle: None,
            ignition_module: None,
            cylinder_banks: Vec::new(),
            crankshafts: Vec::new(),
            published: false,
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
    transmissions: BTreeMap<usize, TransmissionData>,
    rod_journals: BTreeMap<usize, RodJournalData>,
    crankshaft_journals: BTreeMap<usize, Vec<usize>>,
    /// `CylinderHeadNode::m_bank`.
    head_banks: BTreeMap<usize, usize>,
    banks: BTreeMap<usize, BankData>,
    engines: BTreeMap<usize, EngineData>,
    intakes: BTreeMap<usize, IntakeData>,
    exhausts: BTreeMap<usize, ExhaustData>,
    heads: BTreeMap<usize, HeadData>,
    ignition_modules: BTreeMap<usize, IgnitionModuleData>,
    camshafts: BTreeMap<usize, CamshaftData>,
    valvetrains: BTreeMap<usize, ValvetrainData>,
    throttles: BTreeMap<usize, ThrottleData>,
    impulse_responses: BTreeMap<usize, ImpulseResponseData>,

    /// `IgnitionWireNode::m_connections`: the `(bank, local index)` pairs
    /// `add_cylinder` hangs on a wire, in order.
    ignition_wires: BTreeMap<usize, Vec<(usize, usize)>>,

    /// `EngineContext::m_cylinderIndices`.
    cylinder_indices: BTreeMap<(usize, usize), i32>,

    /// `EngineContext::m_rodJournals`: the index a journal answers with.
    rod_journal_indices: BTreeMap<usize, i32>,

    /// `ConnectingRodNode::m_rodJournals`, in order.
    rod_slave_journals: BTreeMap<usize, Vec<usize>>,

    /// `Compiler::Output::vehicle` and `::transmission`: the nodes
    /// `set_vehicle` and `set_transmission` chose, which is what the application
    /// drives. `None` means the script set neither and the application falls back
    /// to its own pair.
    selected_vehicle: Option<usize>,
    selected_transmission: Option<usize>,
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

/// `CylinderBankNode`'s own state: its six `CylinderBank::Parameters` numbers,
/// the cylinders `add_cylinder` gave it, and the head `set_cylinder_head` gave
/// it.
#[derive(Clone, Debug, Default)]
pub struct BankData {
    pub angle: f64,
    pub bore: f64,
    pub deck_height: f64,
    pub position_x: f64,
    pub position_y: f64,
    pub display_depth: f64,

    pub cylinders: Vec<CylinderRefs>,
    pub head: Option<usize>,
}

/// `IntakeNode`'s `Intake::Parameters`: copied verbatim into the `Intake`.
#[derive(Clone, Copy, Debug, Default)]
pub struct IntakeData {
    pub volume: f64,
    pub cross_section_area: f64,
    pub input_flow_k: f64,
    pub idle_flow_k: f64,
    pub runner_flow_rate: f64,
    pub molecular_afr: f64,
    pub idle_throttle_plate_position: f64,
    pub runner_length: f64,
    pub velocity_decay: f64,
}

/// `ImpulseResponseNode`'s `ImpulseResponse`: the file an exhaust's convolution
/// runs on and the volume it is scaled by.
///
/// `generate` resolves the filename against the directory of the unit the node
/// was written in -- `parent_path() / filename` -- which is why the resolved
/// string is what the object holds and what every dump prints.
#[derive(Clone, Debug)]
pub struct ImpulseResponseData {
    pub filename: String,
    pub volume: f64,
}

impl Default for ImpulseResponseData {
    /// `ImpulseResponse::ImpulseResponse` sets the volume and leaves the
    /// filename empty; `initialize` then overwrites both.
    fn default() -> Self {
        Self {
            filename: String::new(),
            volume: 1.0,
        }
    }
}

/// `ExhaustSystemNode`'s `ExhaustSystem::Parameters`, minus the impulse response
/// it stamps in when it generates.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExhaustData {
    pub length: f64,
    pub collector_cross_section_area: f64,
    pub outlet_flow_rate: f64,
    pub primary_tube_length: f64,
    pub primary_flow_rate: f64,
    pub audio_volume: f64,
    pub velocity_decay: f64,
    pub impulse_response: Option<usize>,
}

/// `CylinderHeadNode`'s `CylinderHead::Parameters`, minus the four pointers
/// `_evaluate` clears after reading them. `flip_display` arrives as a number.
#[derive(Clone, Copy, Debug, Default)]
pub struct HeadData {
    pub combustion_chamber_volume: f64,
    pub flip_display: bool,
    pub intake_runner_volume: f64,
    pub intake_runner_cross_section_area: f64,
    pub exhaust_runner_volume: f64,
    pub exhaust_runner_cross_section_area: f64,

    pub intake_port_flow: Option<usize>,
    pub exhaust_port_flow: Option<usize>,
    pub valvetrain: Option<usize>,
}

/// `CamshaftNode`'s `Camshaft::Parameters`, plus the lobes `add_lobe` hangs on
/// it. The lobe profile is the function node it names.
#[derive(Clone, Debug, Default)]
pub struct CamshaftData {
    pub advance: f64,
    pub base_radius: f64,
    pub lobe_profile: Option<usize>,
    pub lobes: Vec<f64>,
}

/// `StandardValvetrainNode` and `VtecValvetrainNode`. Both name camshaft nodes;
/// `generate` is what turns them into a `Valvetrain`.
#[derive(Clone, Debug)]
pub enum ValvetrainData {
    Standard {
        intake_camshaft: Option<usize>,
        exhaust_camshaft: Option<usize>,
    },
    Vtec {
        intake_camshaft: Option<usize>,
        exhaust_camshaft: Option<usize>,
        vtec_intake_camshaft: Option<usize>,
        vtec_exhaust_camshaft: Option<usize>,
        min_rpm: f64,
        min_speed: f64,
        min_throttle_position: f64,
        manifold_vacuum: f64,
    },
}

/// `direct_throttle_linkage`'s one input.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThrottleData {
    pub gamma: f64,
}

/// `IgnitionModuleNode`'s own state: its parameters plus the posts
/// `connect_ignition_wire` hangs on it.
#[derive(Clone, Debug, Default)]
pub struct IgnitionModuleData {
    pub timing_curve: Option<usize>,
    pub rev_limit: f64,
    pub limiter_duration: f64,

    /// `IgnitionModuleNode::Post`: the wire, and the angle it fires at.
    pub posts: Vec<(usize, f64)>,
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

/// `TransmissionNode`'s `Transmission::Parameters`: the clutch's limit, plus the
/// ratios `add_gear` pushed onto it. `Transmission::Parameters` has no default
/// initialisers in C++, so a node that never read its input holds whatever was
/// there; every script's `transmission` supplies `max_clutch_torque`, and the
/// port starts from `Transmission`'s own default.
#[derive(Clone, Debug)]
pub struct TransmissionData {
    pub max_clutch_torque: f64,
    pub gear_ratios: Vec<f64>,
}

impl Default for TransmissionData {
    fn default() -> Self {
        Self {
            max_clutch_torque: engine_sim_core::units::torque(1000.0, engine_sim_core::units::FT_LB),
            gear_ratios: Vec::new(),
        }
    }
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

    pub fn transmission(&self, node: usize) -> Option<&TransmissionData> {
        self.transmissions.get(&node)
    }

    pub fn set_transmission(&mut self, node: usize, transmission: TransmissionData) {
        self.transmissions.insert(node, transmission);
    }

    /// `TransmissionNode::addGear`. A ratio pushed onto a node that never
    /// evaluated is dropped, exactly as the C++ dereferences the node it was
    /// given.
    pub fn add_gear(&mut self, node: usize, ratio: f64) {
        if let Some(transmission) = self.transmissions.get_mut(&node) {
            transmission.gear_ratios.push(ratio);
        }
    }

    /// `SetVehicleNode::_evaluate`: `Compiler::output()->vehicle`.
    pub fn select_vehicle(&mut self, node: usize) {
        self.selected_vehicle = Some(node);
    }

    /// `SetTransmissionNode::_evaluate`: `Compiler::output()->transmission`.
    pub fn select_transmission(&mut self, node: usize) {
        self.selected_transmission = Some(node);
    }

    /// The vehicle the script published, if it published one.
    pub fn selected_vehicle(&self) -> Option<&VehicleData> {
        self.vehicles.get(&self.selected_vehicle?)
    }

    /// The transmission the script published, if it published one.
    pub fn selected_transmission(&self) -> Option<&TransmissionData> {
        self.transmissions.get(&self.selected_transmission?)
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

    pub fn set_bank(&mut self, node: usize, bank: BankData) {
        self.banks.insert(node, bank);
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

    pub fn intake(&self, node: usize) -> Option<&IntakeData> {
        self.intakes.get(&node)
    }

    pub fn intake_mut(&mut self, node: usize) -> &mut IntakeData {
        self.intakes.entry(node).or_default()
    }

    pub fn exhaust(&self, node: usize) -> Option<&ExhaustData> {
        self.exhausts.get(&node)
    }

    pub fn exhaust_mut(&mut self, node: usize) -> &mut ExhaustData {
        self.exhausts.entry(node).or_default()
    }

    /// The impulse response an exhaust names, as `ImpulseResponseNode::generate`
    /// makes it.
    pub fn impulse_response(&self, node: usize) -> Option<&ImpulseResponseData> {
        self.impulse_responses.get(&node)
    }

    pub fn set_impulse_response(&mut self, node: usize, response: ImpulseResponseData) {
        self.impulse_responses.insert(node, response);
    }

    pub fn head(&self, node: usize) -> Option<&HeadData> {
        self.heads.get(&node)
    }

    pub fn head_mut(&mut self, node: usize) -> &mut HeadData {
        self.heads.entry(node).or_default()
    }

    pub fn ignition_module(&self, node: usize) -> Option<&IgnitionModuleData> {
        self.ignition_modules.get(&node)
    }

    pub fn ignition_module_mut(&mut self, node: usize) -> &mut IgnitionModuleData {
        self.ignition_modules.entry(node).or_default()
    }

    pub fn camshaft(&self, node: usize) -> Option<&CamshaftData> {
        self.camshafts.get(&node)
    }

    pub fn camshaft_mut(&mut self, node: usize) -> &mut CamshaftData {
        self.camshafts.entry(node).or_default()
    }

    pub fn valvetrain(&self, node: usize) -> Option<&ValvetrainData> {
        self.valvetrains.get(&node)
    }

    pub fn set_valvetrain(&mut self, node: usize, valvetrain: ValvetrainData) {
        self.valvetrains.insert(node, valvetrain);
    }

    pub fn throttle(&self, node: usize) -> Option<&ThrottleData> {
        self.throttles.get(&node)
    }

    pub fn set_throttle(&mut self, node: usize, throttle: ThrottleData) {
        self.throttles.insert(node, throttle);
    }

    /// `IgnitionWireNode::connect`
    pub fn connect_wire(&mut self, wire: usize, bank: usize, index: usize) {
        self.ignition_wires
            .entry(wire)
            .or_default()
            .push((bank, index));
    }

    pub fn wire_connections(&self, wire: usize) -> &[(usize, usize)] {
        self.ignition_wires
            .get(&wire)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// `EngineContext::setCylinderIndex`
    pub fn set_cylinder_index(&mut self, bank: usize, local: usize, global: i32) {
        self.cylinder_indices.insert((bank, local), global);
    }

    /// `EngineContext::getCylinderIndex`. The original's `std::map` default is
    /// zero for a pair it has never seen, and so is this.
    pub fn cylinder_index(&self, bank: usize, local: usize) -> i32 {
        self.cylinder_indices
            .get(&(bank, local))
            .copied()
            .unwrap_or(0)
    }

    /// `EngineContext::addRodJournal`
    pub fn add_rod_journal_index(&mut self, journal: usize, index: i32) {
        self.rod_journal_indices.insert(journal, index);
    }

    pub fn rod_journal_index(&self, journal: usize) -> i32 {
        self.rod_journal_indices.get(&journal).copied().unwrap_or(0)
    }

    /// `ConnectingRodNode::addRodJournal`: the rod gathers the journal, and the
    /// journal learns which rod it is a slave of.
    pub fn attach_slave_journal(&mut self, rod: usize, journal: usize) {
        self.rod_slave_journals.entry(rod).or_default().push(journal);
        self.rod_journals.entry(journal).or_default().rod = Some(rod);
    }

    pub fn rod_slave_journals(&self, rod: usize) -> &[usize] {
        self.rod_slave_journals
            .get(&rod)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn engine(&self, node: usize) -> Option<&EngineData> {
        self.engines.get(&node)
    }

    /// `EngineNode::addCylinderBank`/`addCrankshaft`, in the order the actions ran.
    pub fn engine_mut(&mut self, node: usize) -> &mut EngineData {
        self.engines.entry(node).or_default()
    }

    /// The cylinder banks and crankshafts of the published engine, if any.
    pub fn published_engine(&self) -> Option<&EngineData> {
        self.engines.values().find(|engine| engine.published)
    }

    /// The nodes that generated an object, in order.
    pub fn function_nodes(&self) -> Vec<usize> {
        self.functions.keys().copied().collect()
    }
}
