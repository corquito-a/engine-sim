//! Mechanical and fluid parts: port of `piston`, `connecting_rod`,
//! `crankshaft`, `cylinder_bank`, `camshaft`, `valvetrain`,
//! `standard_valvetrain`, `vtec_valvetrain`, `cylinder_head`, `intake`,
//! `exhaust_system` and `throttle`.
//!
//! Structural differences from the C++:
//!
//! * Parts reference each other through `Arc` (immutable data) or indices
//!   (mutable data), because the original uses raw pointers.
//! * `atg_scs::RigidBody` members live in the solver's body array; each part
//!   stores the body index (`m_body.index` in the original) and the accessor
//!   takes `&[RigidBody]`.

use std::cell::Cell;
use std::sync::Arc;

use crate::constants;
use crate::function::Function;
use crate::gas::{FlowParameters, GasSystem, Mix};
use crate::scs::RigidBody;
use crate::units;

// ------------------------------------------------------------------ piston

/// `Piston::Parameters`
#[derive(Clone)]
pub struct PistonParameters {
    pub rod: Option<Arc<ConnectingRod>>,
    pub bank: Option<Arc<CylinderBank>>,
    pub cylinder_index: i32,
    pub compression_height: f64,
    pub displacement: f64,
    pub wrist_pin_position: f64,
    pub mass: f64,
    pub blowby_flow_coefficient: f64,
}

impl Default for PistonParameters {
    fn default() -> Self {
        Self {
            rod: None,
            bank: None,
            cylinder_index: -1,
            compression_height: 0.0,
            displacement: 0.0,
            wrist_pin_position: 0.0,
            mass: 0.0,
            blowby_flow_coefficient: 0.0,
        }
    }
}

/// `Piston`
pub struct Piston {
    pub rod: Arc<ConnectingRod>,
    pub bank: Arc<CylinderBank>,
    pub cylinder_index: i32,
    pub compression_height: f64,
    pub displacement: f64,
    pub wrist_pin_location: f64,
    pub mass: f64,
    pub blowby_k: f64,

    /// Index of this piston's body in `RigidBodySystem::bodies`.
    pub body: usize,
    /// Solver force on the cylinder-wall constraint, cached after each step
    /// (`Piston::calculateCylinderWallForce`).
    pub cylinder_wall_force: f64,
}

impl Piston {
    pub fn new(params: PistonParameters) -> Self {
        Self {
            rod: params.rod.expect("piston requires a connecting rod"),
            bank: params.bank.expect("piston requires a cylinder bank"),
            cylinder_index: params.cylinder_index,
            compression_height: params.compression_height,
            displacement: params.displacement,
            wrist_pin_location: params.wrist_pin_position,
            mass: params.mass,
            blowby_k: params.blowby_flow_coefficient,
            body: usize::MAX,
            cylinder_wall_force: 0.0,
        }
    }

    /// `Piston::relativeX`
    pub fn relative_x(&self, bodies: &[RigidBody]) -> f64 {
        bodies[self.body].p_x - self.bank.x()
    }

    /// `Piston::relativeY`
    pub fn relative_y(&self, bodies: &[RigidBody]) -> f64 {
        bodies[self.body].p_y - self.bank.y()
    }

    /// `Piston::calculateCylinderWallForce`
    pub fn calculate_cylinder_wall_force(&self) -> f64 {
        self.cylinder_wall_force
    }
}

// --------------------------------------------------------- connecting rod

/// `ConnectingRod::Parameters`
#[derive(Clone, Default)]
pub struct ConnectingRodParameters {
    pub center_of_mass: f64,
    pub length: f64,
    pub mass: f64,
    pub moment_of_inertia: f64,
    pub journal: i32,
    pub crankshaft: Option<Arc<Crankshaft>>,
    pub slave_throw: f64,
    pub rod_journals: usize,
    pub master: Option<Arc<ConnectingRod>>,
    /// The bank of the piston this rod drives (used by `placeRod`).
    pub piston_bank: Option<Arc<CylinderBank>>,
    /// The cylinder index of the piston this rod drives.
    pub piston_cylinder_index: i32,
}

/// `ConnectingRod` (geometry only — the body lives in the solver).
pub struct ConnectingRod {
    pub center_of_mass: f64,
    pub length: f64,
    pub mass: f64,
    pub moment_of_inertia: f64,
    pub journal: i32,
    pub crankshaft: Arc<Crankshaft>,
    pub master: Option<Arc<ConnectingRod>>,
    pub slave_throw: f64,
    pub rod_journal_angles: Vec<f64>,

    /// The bank of the piston this rod drives (`placeRod` reads
    /// `rod.getPiston()->getCylinderBank()`).
    pub piston_bank: Option<Arc<CylinderBank>>,
    /// The cylinder index of the piston this rod drives.
    pub piston_cylinder_index: i32,

    /// Index of this rod's body in `RigidBodySystem::bodies`.
    pub body: Cell<usize>,
}

impl ConnectingRod {
    pub fn new(params: ConnectingRodParameters) -> Self {
        Self {
            center_of_mass: params.center_of_mass,
            length: params.length,
            mass: params.mass,
            moment_of_inertia: params.moment_of_inertia,
            journal: params.journal,
            crankshaft: params.crankshaft.expect("rod requires a crankshaft"),
            master: params.master,
            slave_throw: params.slave_throw,
            rod_journal_angles: vec![0.0; params.rod_journals],
            piston_bank: params.piston_bank,
            piston_cylinder_index: params.piston_cylinder_index,
            body: Cell::new(usize::MAX),
        }
    }

    /// `ConnectingRod::getBigEndLocal`
    pub fn big_end_local(&self) -> f64 {
        -(self.length / 2.0) + self.center_of_mass
    }

    /// `ConnectingRod::getLittleEndLocal`
    pub fn little_end_local(&self) -> f64 {
        (self.length / 2.0) - self.center_of_mass
    }

    pub fn set_rod_journal_angle(&mut self, i: usize, angle: f64) {
        self.rod_journal_angles[i] = angle;
    }

    pub fn rod_journal_count(&self) -> usize {
        self.rod_journal_angles.len()
    }

    pub fn rod_journal_angle(&self, i: usize) -> f64 {
        self.rod_journal_angles[i]
    }

    /// `ConnectingRod::getRodJournalPositionLocal`
    pub fn rod_journal_position_local(&self, i: usize) -> (f64, f64) {
        let journal_angle = self.rod_journal_angle(i);
        (
            journal_angle.cos() * self.slave_throw,
            journal_angle.sin() * self.slave_throw + self.big_end_local(),
        )
    }

    /// `ConnectingRod::getRodJournalPositionGlobal`
    pub fn rod_journal_position_global(&self, i: usize, bodies: &[RigidBody]) -> (f64, f64) {
        let (lx, ly) = self.rod_journal_position_local(i);

        let body = &bodies[self.body.get()];
        let angle = body.theta;
        let dx = angle.cos();
        let dy = angle.sin();

        (dx * lx - dy * ly + body.p_x, dy * lx + dx * ly + body.p_y)
    }

    /// `ConnectingRod::getLayer`
    pub fn layer(&self) -> i32 {
        match &self.master {
            Some(master) => master.layer(),
            None => self.journal,
        }
    }
}

// -------------------------------------------------------------- crankshaft

/// `Crankshaft::Parameters`
#[derive(Clone, Default)]
pub struct CrankshaftParameters {
    pub mass: f64,
    pub flywheel_mass: f64,
    pub moment_of_inertia: f64,
    pub crank_throw: f64,
    pub rod_journals: usize,
    pub pos_x: f64,
    pub pos_y: f64,
    pub tdc: f64,
    pub friction_torque: f64,
}

/// `Crankshaft`
pub struct Crankshaft {
    pub mass: f64,
    pub flywheel_mass: f64,
    pub moment_of_inertia: f64,
    pub crank_throw: f64,
    pub rod_journal_angles: Vec<f64>,
    pub pos_x: f64,
    pub pos_y: f64,
    pub tdc: f64,
    pub friction_torque: f64,

    /// Index of this crankshaft's body in `RigidBodySystem::bodies`.
    pub body: Cell<usize>,
}

impl Crankshaft {
    pub fn new(params: CrankshaftParameters) -> Self {
        Self {
            mass: params.mass,
            flywheel_mass: params.flywheel_mass,
            moment_of_inertia: params.moment_of_inertia,
            crank_throw: params.crank_throw,
            rod_journal_angles: vec![0.0; params.rod_journals],
            pos_x: params.pos_x,
            pos_y: params.pos_y,
            tdc: params.tdc,
            friction_torque: params.friction_torque,
            body: Cell::new(usize::MAX),
        }
    }

    /// `Crankshaft::getRodJournalPositionLocal`
    pub fn rod_journal_position_local(&self, i: usize) -> (f64, f64) {
        let theta = self.rod_journal_angles[i];
        (theta.cos() * self.crank_throw, theta.sin() * self.crank_throw)
    }

    /// `Crankshaft::getRodJournalPositionGlobal`
    pub fn rod_journal_position_global(&self, i: usize, bodies: &[RigidBody]) -> (f64, f64) {
        let (lx, ly) = self.rod_journal_position_local(i);
        let body = &bodies[self.body.get()];
        (lx + body.p_x, ly + body.p_y)
    }

    pub fn set_rod_journal_angle(&mut self, i: usize, angle: f64) {
        assert!(i < self.rod_journal_angles.len());
        self.rod_journal_angles[i] = angle;
    }

    pub fn rod_journal_count(&self) -> usize {
        self.rod_journal_angles.len()
    }

    /// `Crankshaft::resetAngle`
    pub fn reset_angle(&self, bodies: &mut [RigidBody]) {
        let body = &mut bodies[self.body.get()];
        body.theta = body.theta % (4.0 * constants::PI);
    }

    /// `Crankshaft::getAngle`
    pub fn get_angle(&self, bodies: &[RigidBody]) -> f64 {
        bodies[self.body.get()].theta - self.tdc
    }

    /// `Crankshaft::getCycleAngle`
    pub fn get_cycle_angle(&self, offset: f64, bodies: &[RigidBody]) -> f64 {
        let wrapped = (-self.get_angle(bodies) + offset) % (4.0 * constants::PI);
        if wrapped < 0.0 {
            wrapped + 4.0 * constants::PI
        } else {
            wrapped
        }
    }
}

// ------------------------------------------------------------ cylinder bank

/// `CylinderBank::Parameters`
#[derive(Clone, Default)]
pub struct CylinderBankParameters {
    pub angle: f64,
    pub bore: f64,
    pub deck_height: f64,
    pub cylinder_count: i32,
    pub position_x: f64,
    pub position_y: f64,
    pub display_depth: f64,
    pub index: i32,
}

/// `CylinderBank`
#[derive(Clone, Debug)]
pub struct CylinderBank {
    angle: f64,
    bore: f64,
    deck_height: f64,
    display_depth: f64,
    cylinder_count: i32,
    index: i32,
    dx: f64,
    dy: f64,
    x: f64,
    y: f64,
}

impl CylinderBank {
    pub fn new(params: CylinderBankParameters) -> Self {
        Self {
            angle: params.angle,
            bore: params.bore,
            deck_height: params.deck_height,
            display_depth: params.display_depth,
            cylinder_count: params.cylinder_count,
            index: params.index,
            dx: (params.angle + constants::PI / 2.0).cos(),
            dy: (params.angle + constants::PI / 2.0).sin(),
            x: params.position_x,
            y: params.position_y,
        }
    }

    pub fn angle(&self) -> f64 {
        self.angle
    }
    pub fn bore(&self) -> f64 {
        self.bore
    }
    pub fn deck_height(&self) -> f64 {
        self.deck_height
    }
    pub fn cylinder_count(&self) -> i32 {
        self.cylinder_count
    }
    pub fn index(&self) -> i32 {
        self.index
    }
    pub fn dx(&self) -> f64 {
        self.dx
    }
    pub fn dy(&self) -> f64 {
        self.dy
    }
    pub fn x(&self) -> f64 {
        self.x
    }
    pub fn y(&self) -> f64 {
        self.y
    }
    pub fn display_depth(&self) -> f64 {
        self.display_depth
    }

    /// `CylinderBank::getPositionAboveDeck`
    pub fn position_above_deck(&self, h: f64) -> (f64, f64) {
        (self.dx * (self.deck_height + h) + self.x, self.dy * (self.deck_height + h) + self.y)
    }

    /// `CylinderBank::boreSurfaceArea`
    pub fn bore_surface_area(&self) -> f64 {
        constants::PI * self.bore * self.bore / 4.0
    }
}

// ---------------------------------------------------------------- camshaft

/// `Camshaft::Parameters`
#[derive(Clone)]
pub struct CamshaftParameters {
    pub lobes: usize,
    pub lobe_profile: Arc<Function>,
    pub advance: f64,
    pub base_radius: f64,
}

/// `Camshaft`
#[derive(Clone)]
pub struct Camshaft {
    pub lobe_angles: Vec<f64>,
    pub lobe_profile: Arc<Function>,
    pub advance: f64,
    pub base_radius: f64,
}

impl Camshaft {
    pub fn new(params: CamshaftParameters) -> Self {
        Self {
            lobe_angles: vec![0.0; params.lobes],
            lobe_profile: params.lobe_profile,
            advance: params.advance,
            base_radius: params.base_radius,
        }
    }

    /// `Camshaft::getAngle` — takes the crankshaft angle
    /// (`crankshaft->getAngle()`), matching the original's use of
    /// `m_crankshaft->getAngle()`.
    pub fn get_angle(&self, crank_angle: f64) -> f64 {
        let angle = ((crank_angle + self.advance) * 0.5) % (2.0 * constants::PI);
        if angle < 0.0 {
            angle + 2.0 * constants::PI
        } else {
            angle
        }
    }

    /// `Camshaft::sampleLobe`
    pub fn sample_lobe(&self, theta: f64) -> f64 {
        let mut clamped_theta = theta % (2.0 * constants::PI);
        if clamped_theta < 0.0 {
            clamped_theta += 2.0 * constants::PI;
        }
        if clamped_theta >= constants::PI {
            clamped_theta -= 2.0 * constants::PI;
        }

        self.lobe_profile.sample_triangle(clamped_theta)
    }

    /// `Camshaft::setLobeCenterline` — the argument is a *crank* angle; the
    /// stored lobe angle is halved.
    pub fn set_lobe_centerline(&mut self, lobe: usize, crank_angle: f64) {
        self.lobe_angles[lobe] = crank_angle / 2.0;
    }

    /// `Camshaft::getLobeCenterline`
    pub fn lobe_centerline(&self, lobe: usize) -> f64 {
        self.lobe_angles[lobe]
    }

    /// `Camshaft::valveLift`
    pub fn valve_lift(&self, lobe: usize, crank_angle: f64) -> f64 {
        self.sample_lobe(self.get_angle(crank_angle) + self.lobe_angles[lobe])
    }

    pub fn lobes(&self) -> usize {
        self.lobe_angles.len()
    }
}

// -------------------------------------------------------------- valvetrain

/// `Valvetrain` (`StandardValvetrain` and `VtecValvetrain`).
#[derive(Clone)]
pub enum Valvetrain {
    /// `StandardValvetrain::Parameters`
    Standard {
        intake_camshaft: Arc<Camshaft>,
        exhaust_camshaft: Arc<Camshaft>,
    },
    /// `VtecValvetrain::Parameters`
    Vtec {
        intake_camshaft: Arc<Camshaft>,
        exhaust_camshaft: Arc<Camshaft>,
        vtec_intake_camshaft: Arc<Camshaft>,
        vtec_exhaust_camshaft: Arc<Camshaft>,
        min_rpm: f64,
        min_speed: f64,
        min_throttle_position: f64,
        manifold_vacuum: f64,
    },
}

impl Valvetrain {
    /// `VtecValvetrain::isVtecEnabled`
    pub fn is_vtec_enabled(&self, engine: &VtecConditions) -> bool {
        match self {
            Valvetrain::Standard { .. } => false,
            Valvetrain::Vtec {
                min_rpm,
                min_speed: _,
                min_throttle_position,
                manifold_vacuum,
                ..
            } => {
                engine.manifold_pressure > *manifold_vacuum
                    && engine.speed > *min_rpm
                    && (1.0 - engine.throttle) > *min_throttle_position
            }
        }
    }

    pub fn intake_valve_lift(&self, cylinder: usize, crank_angle: f64, vtec: bool) -> f64 {
        match self {
            Valvetrain::Standard { intake_camshaft, .. } => {
                intake_camshaft.valve_lift(cylinder, crank_angle)
            }
            Valvetrain::Vtec {
                intake_camshaft,
                vtec_intake_camshaft,
                ..
            } => {
                if vtec {
                    vtec_intake_camshaft.valve_lift(cylinder, crank_angle)
                } else {
                    intake_camshaft.valve_lift(cylinder, crank_angle)
                }
            }
        }
    }

    pub fn exhaust_valve_lift(&self, cylinder: usize, crank_angle: f64, vtec: bool) -> f64 {
        match self {
            Valvetrain::Standard {
                exhaust_camshaft, ..
            } => exhaust_camshaft.valve_lift(cylinder, crank_angle),
            Valvetrain::Vtec {
                exhaust_camshaft,
                vtec_exhaust_camshaft,
                ..
            } => {
                if vtec {
                    vtec_exhaust_camshaft.valve_lift(cylinder, crank_angle)
                } else {
                    exhaust_camshaft.valve_lift(cylinder, crank_angle)
                }
            }
        }
    }

    pub fn active_intake_camshaft(&self, vtec: bool) -> &Camshaft {
        match self {
            Valvetrain::Standard { intake_camshaft, .. } => intake_camshaft,
            Valvetrain::Vtec {
                intake_camshaft,
                vtec_intake_camshaft,
                ..
            } => {
                if vtec {
                    vtec_intake_camshaft
                } else {
                    intake_camshaft
                }
            }
        }
    }

    pub fn active_exhaust_camshaft(&self, vtec: bool) -> &Camshaft {
        match self {
            Valvetrain::Standard {
                exhaust_camshaft, ..
            } => exhaust_camshaft,
            Valvetrain::Vtec {
                exhaust_camshaft,
                vtec_exhaust_camshaft,
                ..
            } => {
                if vtec {
                    vtec_exhaust_camshaft
                } else {
                    exhaust_camshaft
                }
            }
        }
    }
}

/// Inputs needed by `VtecValvetrain::isVtecEnabled`.
#[derive(Clone, Copy, Debug, Default)]
pub struct VtecConditions {
    pub manifold_pressure: f64,
    pub speed: f64,
    pub throttle: f64,
}

// ------------------------------------------------------------ cylinder head

/// `CylinderHead::Cylinder`
#[derive(Clone, Debug)]
pub struct HeadCylinder {
    pub exhaust_system: Option<usize>,
    pub intake: Option<usize>,
    pub sound_attenuation: f64,
    pub header_primary_length: f64,
}

impl Default for HeadCylinder {
    fn default() -> Self {
        Self {
            exhaust_system: None,
            intake: None,
            sound_attenuation: 1.0,
            header_primary_length: 0.0,
        }
    }
}

/// `CylinderHead`
#[derive(Clone)]
pub struct CylinderHead {
    pub bank: Arc<CylinderBank>,
    pub valvetrain: Arc<Valvetrain>,
    pub exhaust_port_flow: Arc<Function>,
    pub intake_port_flow: Arc<Function>,

    pub intake_runner_volume: f64,
    pub intake_runner_cross_section_area: f64,
    pub exhaust_runner_volume: f64,
    pub exhaust_runner_cross_section_area: f64,

    pub combustion_chamber_volume: f64,
    pub flip_display: bool,

    pub cylinders: Vec<HeadCylinder>,
}

impl CylinderHead {
    /// `CylinderHead::intakeValveLift`
    pub fn intake_valve_lift(&self, cylinder: usize, crank_angle: f64, vtec: bool) -> f64 {
        self.valvetrain
            .intake_valve_lift(cylinder, crank_angle, vtec)
    }

    /// `CylinderHead::exhaustValveLift`
    pub fn exhaust_valve_lift(&self, cylinder: usize, crank_angle: f64, vtec: bool) -> f64 {
        self.valvetrain
            .exhaust_valve_lift(cylinder, crank_angle, vtec)
    }

    /// `CylinderHead::intakeFlowRate`
    pub fn intake_flow_rate(&self, cylinder: usize, crank_angle: f64, vtec: bool) -> f64 {
        self.intake_port_flow
            .sample_triangle(self.intake_valve_lift(cylinder, crank_angle, vtec))
    }

    /// `CylinderHead::exhaustFlowRate`
    pub fn exhaust_flow_rate(&self, cylinder: usize, crank_angle: f64, vtec: bool) -> f64 {
        self.exhaust_port_flow
            .sample_triangle(self.exhaust_valve_lift(cylinder, crank_angle, vtec))
    }

    pub fn exhaust_system(&self, cylinder: usize) -> Option<usize> {
        self.cylinders[cylinder].exhaust_system
    }

    pub fn intake(&self, cylinder: usize) -> Option<usize> {
        self.cylinders[cylinder].intake
    }

    pub fn sound_attenuation(&self, cylinder: usize) -> f64 {
        self.cylinders[cylinder].sound_attenuation
    }

    pub fn header_primary_length(&self, cylinder: usize) -> f64 {
        self.cylinders[cylinder].header_primary_length
    }

    pub fn set_all_exhaust_systems(&mut self, system: Option<usize>) {
        for c in self.cylinders.iter_mut() {
            c.exhaust_system = system;
        }
    }

    pub fn set_exhaust_system(&mut self, i: usize, system: Option<usize>) {
        self.cylinders[i].exhaust_system = system;
    }

    pub fn set_all_intakes(&mut self, intake: Option<usize>) {
        for c in self.cylinders.iter_mut() {
            c.intake = intake;
        }
    }

    pub fn set_intake(&mut self, i: usize, intake: Option<usize>) {
        self.cylinders[i].intake = intake;
    }

    pub fn set_all_header_primary_lengths(&mut self, length: f64) {
        for c in self.cylinders.iter_mut() {
            c.header_primary_length = length;
        }
    }

    pub fn set_header_primary_length(&mut self, i: usize, length: f64) {
        self.cylinders[i].header_primary_length = length;
    }

    pub fn set_sound_attenuation(&mut self, i: usize, attenuation: f64) {
        self.cylinders[i].sound_attenuation = attenuation;
    }
}

// ------------------------------------------------------------------ intake

/// `Intake::Parameters`
#[derive(Clone, Debug)]
pub struct IntakeParameters {
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

impl Default for IntakeParameters {
    fn default() -> Self {
        Self {
            volume: 0.0,
            cross_section_area: 0.0,
            input_flow_k: 0.0,
            idle_flow_k: 0.0,
            runner_flow_rate: 0.0,
            molecular_afr: 25.0 / 2.0,
            idle_throttle_plate_position: 0.975,
            runner_length: units::distance(4.0, units::INCH),
            velocity_decay: 0.5,
        }
    }
}

/// `Intake`
pub struct Intake {
    pub system: GasSystem,
    pub atmosphere: GasSystem,
    pub throttle: f64,

    pub flow: f64,
    pub flow_rate: f64,
    pub total_fuel_injected: f64,

    pub cross_section_area: f64,
    pub input_flow_k: f64,
    pub idle_flow_k: f64,
    pub runner_flow_rate: f64,
    pub molecular_afr: f64,
    pub idle_throttle_plate_position: f64,
    pub runner_length: f64,
    pub velocity_decay: f64,
}

impl Intake {
    /// `Intake::initialize`
    pub fn new(params: &IntakeParameters) -> Self {
        let width = params.cross_section_area.sqrt();
        let mut system = GasSystem::default();
        system.initialize(
            units::pressure(1.0, units::ATM),
            params.volume,
            units::celcius(25.0),
            Mix::DEFAULT,
            5,
        );
        system.set_geometry(width, params.volume / params.cross_section_area, 1.0, 0.0);

        let mut atmosphere = GasSystem::default();
        atmosphere.initialize(
            units::pressure(1.0, units::ATM),
            units::volume(1000.0, units::M3),
            units::celcius(25.0),
            Mix::DEFAULT,
            5,
        );
        atmosphere.set_geometry(
            units::distance(100.0, units::M),
            units::distance(100.0, units::M),
            1.0,
            0.0,
        );

        Self {
            system,
            atmosphere,
            throttle: 1.0,
            flow: 0.0,
            flow_rate: 0.0,
            total_fuel_injected: 0.0,
            cross_section_area: params.cross_section_area,
            input_flow_k: params.input_flow_k,
            idle_flow_k: params.idle_flow_k,
            runner_flow_rate: params.runner_flow_rate,
            molecular_afr: params.molecular_afr,
            idle_throttle_plate_position: params.idle_throttle_plate_position,
            runner_length: params.runner_length,
            velocity_decay: params.velocity_decay,
        }
    }

    pub fn throttle_plate_position(&self) -> f64 {
        self.idle_throttle_plate_position * self.throttle
    }

    /// `Intake::process`
    pub fn process(&mut self, dt: f64) {
        let ideal_afr = 0.8 * self.molecular_afr * 4.0;
        let current_afr =
            (self.system.mix().p_o2 + self.system.mix().p_inert) / self.system.mix().p_fuel;
        let _ = current_afr; // written in the original but only used for diagnostics

        let p_air = ideal_afr / (1.0 + ideal_afr);
        let fuel_air_mix = Mix::new(1.0 - p_air, p_air * 0.75, p_air * 0.25);

        let idle_afr = 2.0;
        let p_idle_air = idle_afr / (1.0 + idle_afr);
        let fuel_mix = Mix::new(1.0 - p_idle_air, p_idle_air * 0.75, p_idle_air * 0.25);

        let throttle = self.throttle_plate_position();
        let flow_attenuation = (throttle * constants::PI / 2.0).cos();

        let mut flow_params = FlowParameters {
            cross_section_area_0: units::area(10.0, units::M2),
            cross_section_area_1: self.cross_section_area,
            direction_x: 0.0,
            direction_y: -1.0,
            dt,
            k_flow: 0.0,
        };

        self.atmosphere.reset(
            units::pressure(1.0, units::ATM),
            units::celcius(25.0),
            fuel_air_mix,
        );
        flow_params.k_flow = flow_attenuation * self.input_flow_k;
        self.flow = GasSystem::flow(&mut self.atmosphere, &mut self.system, &flow_params);

        self.atmosphere.reset(
            units::pressure(1.0, units::ATM),
            units::celcius(25.0),
            fuel_mix,
        );
        flow_params.k_flow = self.idle_flow_k;
        let idle_circuit_flow = GasSystem::flow(&mut self.atmosphere, &mut self.system, &flow_params);

        self.system.dissipate_excess_velocity();
        self.system.update_velocity(dt, self.velocity_decay);

        if self.flow > 0.0 {
            self.total_fuel_injected += fuel_air_mix.p_fuel * self.flow;
        }

        if idle_circuit_flow > 0.0 {
            self.total_fuel_injected += fuel_mix.p_fuel * idle_circuit_flow;
        }
    }
}

// ---------------------------------------------------------- exhaust system

/// `ExhaustSystem::Parameters`
#[derive(Clone, Debug)]
pub struct ExhaustSystemParameters {
    pub length: f64,
    pub collector_cross_section_area: f64,
    pub outlet_flow_rate: f64,
    pub primary_tube_length: f64,
    pub primary_flow_rate: f64,
    pub velocity_decay: f64,
    pub audio_volume: f64,
    /// Handle into the audio layer's impulse-response table.
    pub impulse_response: Option<usize>,
}

impl Default for ExhaustSystemParameters {
    fn default() -> Self {
        Self {
            length: 0.0,
            collector_cross_section_area: 0.0,
            outlet_flow_rate: 0.0,
            primary_tube_length: 0.0,
            primary_flow_rate: 0.0,
            velocity_decay: 0.0,
            audio_volume: 0.0,
            impulse_response: None,
        }
    }
}

/// `ExhaustSystem`
pub struct ExhaustSystem {
    pub atmosphere: GasSystem,
    pub system: GasSystem,

    pub impulse_response: Option<usize>,

    pub length: f64,
    pub primary_tube_length: f64,
    pub collector_cross_section_area: f64,
    pub primary_flow_rate: f64,
    pub outlet_flow_rate: f64,
    pub audio_volume: f64,
    pub velocity_decay: f64,
    pub index: i32,

    pub flow: f64,
}

impl ExhaustSystem {
    /// `ExhaustSystem::initialize`
    pub fn new(params: &ExhaustSystemParameters) -> Self {
        let system_width = params.collector_cross_section_area.sqrt();
        let volume = params.collector_cross_section_area * params.length;
        let system_length = params.length;

        let mut system = GasSystem::default();
        system.initialize(
            units::pressure(1.0, units::ATM),
            volume,
            units::celcius(25.0),
            Mix::DEFAULT,
            5,
        );
        system.set_geometry(system_length, system_width, 1.0, 0.0);

        let mut atmosphere = GasSystem::default();
        atmosphere.initialize(
            units::pressure(1.0, units::ATM),
            units::volume(1000.0, units::M3),
            units::celcius(25.0),
            Mix::DEFAULT,
            5,
        );
        atmosphere.set_geometry(
            units::distance(10.0, units::M),
            units::distance(10.0, units::M),
            1.0,
            0.0,
        );

        Self {
            atmosphere,
            system,
            impulse_response: params.impulse_response,
            length: params.length,
            primary_tube_length: params.primary_tube_length,
            collector_cross_section_area: params.collector_cross_section_area,
            primary_flow_rate: params.primary_flow_rate,
            outlet_flow_rate: params.outlet_flow_rate,
            audio_volume: params.audio_volume,
            velocity_decay: params.velocity_decay,
            index: -1,
            flow: 0.0,
        }
    }

    /// `ExhaustSystem::process`
    pub fn process(&mut self, dt: f64) {
        let air_mix = Mix::new(0.0, 1.0, 0.0);

        self.atmosphere.reset(
            units::pressure(1.0, units::ATM),
            units::celcius(25.0),
            air_mix,
        );

        let flow_params = FlowParameters {
            cross_section_area_0: self.collector_cross_section_area,
            cross_section_area_1: units::area(10.0, units::M2),
            direction_x: 1.0,
            direction_y: 0.0,
            dt,
            k_flow: self.outlet_flow_rate,
        };

        self.flow = GasSystem::flow(&mut self.atmosphere, &mut self.system, &flow_params);

        self.system.dissipate_excess_velocity();
        self.system.update_velocity(dt, self.velocity_decay);
    }
}

// ----------------------------------------------------------------- throttle

/// `Throttle` — the engine only uses `setSpeedControl`/`getSpeedControl` and
/// `update`.
pub trait Throttle {
    fn set_speed_control(&mut self, s: f64);
    fn speed_control(&self) -> f64;
    fn update(&mut self, dt: f64, conditions: &ThrottleConditions) -> Option<f64>;
}

/// The subset of `Engine` that a throttle/governor reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct ThrottleConditions {
    pub speed: f64,
}

/// `DirectThrottleLinkage`: `throttle = 1 - speedControl^gamma`.
#[derive(Clone, Copy, Debug)]
pub struct DirectThrottleLinkage {
    speed_control: f64,
    throttle_position: f64,
    gamma: f64,
}

impl Default for DirectThrottleLinkage {
    fn default() -> Self {
        Self {
            speed_control: 0.0,
            throttle_position: 1.0,
            gamma: 1.0,
        }
    }
}

impl DirectThrottleLinkage {
    pub fn new(gamma: f64) -> Self {
        Self {
            gamma,
            ..Default::default()
        }
    }
}

impl Throttle for DirectThrottleLinkage {
    fn set_speed_control(&mut self, s: f64) {
        self.speed_control = s;
        self.throttle_position = 1.0 - s.powf(self.gamma);
    }

    fn speed_control(&self) -> f64 {
        self.speed_control
    }

    fn update(&mut self, _dt: f64, _conditions: &ThrottleConditions) -> Option<f64> {
        Some(self.throttle_position)
    }
}

/// `Throttle` proper: a plain speed-control holder that does nothing on update.
#[derive(Clone, Copy, Debug, Default)]
pub struct DirectThrottle {
    speed_control: f64,
}

impl Throttle for DirectThrottle {
    fn set_speed_control(&mut self, s: f64) {
        self.speed_control = s;
    }

    fn speed_control(&self) -> f64 {
        self.speed_control
    }

    fn update(&mut self, _dt: f64, _conditions: &ThrottleConditions) -> Option<f64> {
        None
    }
}
