//! Port of `include/engine.h` / `src/engine.cpp`.
//!
//! `Engine` is the aggregate that owns (or shares) every part. The C++ version
//! allocates arrays of parts and wires them with raw pointers during
//! simulation loading; here the wiring is done in Rust constructors and the
//! solver bodies live in [`crate::scs::RigidBodySystem`].

use std::sync::Arc;

use crate::chamber::{ChamberParameters, CombustionChamber};
use crate::constants;
use crate::drivetrain::Governor;
use crate::fuel::Fuel;
use crate::function::Function;
use crate::ignition::IgnitionModule;
use crate::parts::{
    ConnectingRod, Crankshaft, CylinderBank, CylinderHead, ExhaustSystem, Intake, Piston,
    Throttle, ThrottleConditions, VtecConditions,
};
use crate::scs::RigidBody;
use crate::units;

/// `placeRod` — solves for the piston position given a crankshaft angle.
///
/// Returns `(p_x, p_y, theta, s)` or `None` when the rod cannot be placed (the
/// original returns `false`).
pub fn place_rod(
    rod: &ConnectingRod,
    bank: &CylinderBank,
    crank_angle: f64,
) -> Option<(f64, f64, f64, f64)> {
    let p_x_0;
    let p_y_0;
    let l_x;
    let l_y;
    let theta_0;

    if let Some(master) = &rod.master {
        let master_bank = master
            .piston_bank
            .as_ref()
            .expect("master rod must reference its piston's bank");

        let (px, py, theta, _s) = place_rod(master, master_bank, crank_angle)?;

        p_x_0 = px;
        p_y_0 = py;
        theta_0 = theta;

        let (lx, ly) =
            master.rod_journal_position_local(rod.piston_cylinder_index.max(0) as usize);
        l_x = lx;
        l_y = ly;
    } else {
        theta_0 = crank_angle;
        p_x_0 = rod.crankshaft.pos_x;
        p_y_0 = rod.crankshaft.pos_y;

        let (lx, ly) = rod
            .crankshaft
            .rod_journal_position_local(rod.piston_cylinder_index.max(0) as usize);
        l_x = lx;
        l_y = ly;
    }

    let dx = theta_0.cos();
    let dy = theta_0.sin();
    let p_x = p_x_0 + (dx * l_x - dy * l_y);
    let p_y = p_y_0 + (dy * l_x + dx * l_y);

    // (bank->m_x + bank->m_dx * s - p_x)^2 + (bank->m_y + bank->m_dy * s - p_y)^2 = L^2
    let a = bank.dx() * bank.dx() + bank.dy() * bank.dy();
    let b = -2.0 * bank.dx() * (p_x - bank.x()) - 2.0 * bank.dy() * (p_y - bank.y());
    let c = (p_x - bank.x()) * (p_x - bank.x()) + (p_y - bank.y()) * (p_y - bank.y())
        - rod.length * rod.length;

    let det = b * b - 4.0 * a * c;
    if det < 0.0 {
        return None;
    }

    let sqrt_det = det.sqrt();
    let s0 = (-b + sqrt_det) / (2.0 * a);
    let s1 = (-b - sqrt_det) / (2.0 * a);

    let s = s0.max(s1);
    if s < 0.0 {
        return None;
    }

    let ddx = (bank.x() + bank.dx() * s) - p_x;
    let ddy = (bank.y() + bank.dy() * s) - p_y;

    let theta = if ddy > 0.0 {
        ddx.acos()
    } else {
        -ddx.acos()
    };

    Some((p_x, p_y, theta, s))
}

/// `Engine`
pub struct Engine {
    pub name: String,

    pub crankshafts: Vec<Arc<Crankshaft>>,
    pub banks: Vec<Arc<CylinderBank>>,
    pub heads: Vec<Arc<CylinderHead>>,
    pub pistons: Vec<Piston>,
    pub rods: Vec<Arc<ConnectingRod>>,
    pub exhausts: Vec<ExhaustSystem>,
    pub intakes: Vec<Intake>,
    pub chambers: Vec<CombustionChamber>,

    pub ignition: IgnitionModule,
    pub fuel: Arc<Fuel>,
    pub throttle: Box<dyn Throttle>,

    pub throttle_value: f64,

    pub starter_torque: f64,
    pub starter_speed: f64,
    pub dyno_min_speed: f64,
    pub dyno_max_speed: f64,
    pub dyno_hold_step: f64,
    pub redline: f64,
    pub displacement: f64,

    pub initial_simulation_frequency: f64,
    pub initial_high_frequency_gain: f64,
    pub initial_jitter: f64,
    pub initial_noise: f64,

    pub mean_piston_speed_to_turbulence: Arc<Function>,
    pub crankcase_pressure: f64,
}

impl Engine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: String,
        fuel: Arc<Fuel>,
        mean_piston_speed_to_turbulence: Arc<Function>,
        ignition: IgnitionModule,
    ) -> Self {
        Self {
            name,
            crankshafts: Vec::new(),
            banks: Vec::new(),
            heads: Vec::new(),
            pistons: Vec::new(),
            rods: Vec::new(),
            exhausts: Vec::new(),
            intakes: Vec::new(),
            chambers: Vec::new(),
            ignition,
            fuel,
            throttle: Box::new(crate::parts::DirectThrottle::default()),
            throttle_value: 0.0,
            starter_torque: 0.0,
            starter_speed: 0.0,
            dyno_min_speed: 0.0,
            dyno_max_speed: 0.0,
            dyno_hold_step: 0.0,
            redline: 0.0,
            displacement: 0.0,
            initial_simulation_frequency: 10000.0,
            initial_high_frequency_gain: 0.01,
            initial_jitter: 0.5,
            initial_noise: 1.0,
            mean_piston_speed_to_turbulence,
            crankcase_pressure: units::pressure(1.0, units::ATM),
        }
    }

    // ------------------------------------------------------------- accessors

    pub fn get_crankshaft(&self, i: usize) -> &Arc<Crankshaft> {
        &self.crankshafts[i]
    }

    pub fn get_output_crankshaft(&self) -> &Arc<Crankshaft> {
        &self.crankshafts[0]
    }

    pub fn get_crankshaft_count(&self) -> usize {
        self.crankshafts.len()
    }

    pub fn get_cylinder_count(&self) -> usize {
        self.pistons.len()
    }

    pub fn get_piston(&self, i: usize) -> &Piston {
        &self.pistons[i]
    }

    pub fn get_connecting_rod(&self, i: usize) -> &Arc<ConnectingRod> {
        &self.rods[i]
    }

    pub fn get_chamber(&self, i: usize) -> &CombustionChamber {
        &self.chambers[i]
    }

    pub fn get_head(&self, i: usize) -> &Arc<CylinderHead> {
        &self.heads[i]
    }

    pub fn get_intake(&self, i: usize) -> &Intake {
        &self.intakes[i]
    }

    pub fn get_intake_count(&self) -> usize {
        self.intakes.len()
    }

    pub fn get_exhaust_system(&self, i: usize) -> &ExhaustSystem {
        &self.exhausts[i]
    }

    pub fn get_exhaust_system_count(&self) -> usize {
        self.exhausts.len()
    }

    pub fn get_ignition_module(&mut self) -> &mut IgnitionModule {
        &mut self.ignition
    }

    pub fn get_fuel(&self) -> &Arc<Fuel> {
        &self.fuel
    }

    // ---------------------------------------------------------------- state

    pub fn set_speed_control(&mut self, s: f64) {
        self.throttle.set_speed_control(s);
    }

    pub fn get_speed_control(&self) -> f64 {
        self.throttle.speed_control()
    }

    /// `Engine::setThrottle`
    pub fn set_throttle(&mut self, throttle: f64) {
        for intake in self.intakes.iter_mut() {
            intake.throttle = throttle;
        }

        self.throttle_value = throttle;
    }

    pub fn get_throttle(&self) -> f64 {
        self.throttle_value
    }

    /// `Engine::getThrottlePlateAngle`
    pub fn get_throttle_plate_angle(&self) -> f64 {
        (1.0 - self.intakes[0].throttle_plate_position()) * (constants::PI / 2.0)
    }

    /// `Engine::getManifoldPressure`
    pub fn get_manifold_pressure(&self) -> f64 {
        let mut pressure_sum = 0.0;
        for intake in self.intakes.iter() {
            pressure_sum += intake.system.pressure();
        }

        pressure_sum / self.intakes.len() as f64
    }

    /// `Engine::getSpeed`
    pub fn get_speed(&self, bodies: &[RigidBody]) -> f64 {
        if self.crankshafts.is_empty() {
            0.0
        } else {
            bodies[self.crankshafts[0].body.get()].v_theta.abs()
        }
    }

    /// `Engine::getRpm`
    pub fn get_rpm(&self, bodies: &[RigidBody]) -> f64 {
        if self.crankshafts.is_empty() {
            0.0
        } else {
            units::to_rpm(bodies[self.crankshafts[0].body.get()].v_theta).abs()
        }
    }

    /// `Engine::isSpinningCw`
    pub fn is_spinning_cw(&self, bodies: &[RigidBody]) -> bool {
        bodies[self.crankshafts[0].body.get()].v_theta <= 0.0
    }

    /// `Engine::getIntakeFlowRate`
    pub fn get_intake_flow_rate(&self) -> f64 {
        let mut air_intake = 0.0;
        for intake in self.intakes.iter() {
            air_intake += intake.flow_rate;
        }

        air_intake
    }

    /// `Engine::getIntakeAfr`
    pub fn get_intake_afr(&self) -> f64 {
        let mut total_oxygen = 0.0;
        let mut total_fuel = 0.0;
        for intake in self.intakes.iter() {
            total_oxygen += intake.system.n_o2();
            total_fuel += intake.system.n_fuel();
        }

        let octane_molar_mass = units::mass(114.23, units::G);
        let oxygen_molar_mass = units::mass(31.9988, units::G);

        if total_fuel == 0.0 {
            0.0
        } else {
            (oxygen_molar_mass * total_oxygen / 0.21) / (total_fuel * octane_molar_mass)
        }
    }

    /// `Engine::getExhaustO2`
    pub fn get_exhaust_o2(&self) -> f64 {
        let mut total_inert = 0.0;
        let mut total_oxygen = 0.0;
        let mut total_fuel = 0.0;
        for exhaust in self.exhausts.iter() {
            total_inert += exhaust.system.n_inert();
            total_oxygen += exhaust.system.n_o2();
            total_fuel += exhaust.system.n_fuel();
        }

        let octane_molar_mass = units::mass(114.23, units::G);
        let oxygen_molar_mass = units::mass(31.9988, units::G);
        let nitrogen_molar_mass = units::mass(28.014, units::G);

        if total_fuel == 0.0 {
            0.0
        } else {
            (oxygen_molar_mass * total_oxygen)
                / (total_fuel * octane_molar_mass
                    + nitrogen_molar_mass * total_inert
                    + oxygen_molar_mass * total_oxygen)
        }
    }

    pub fn reset_fuel_consumption(&mut self) {
        for intake in self.intakes.iter_mut() {
            intake.total_fuel_injected = 0.0;
        }
    }

    /// `Engine::getTotalFuelMassConsumed`
    pub fn get_total_fuel_mass_consumed(&self) -> f64 {
        let mut n_fuel_consumed = 0.0;
        for intake in self.intakes.iter() {
            n_fuel_consumed += intake.total_fuel_injected;
        }

        n_fuel_consumed * self.fuel.molecular_mass()
    }

    /// `Engine::getTotalVolumeFuelConsumed`
    pub fn get_total_volume_fuel_consumed(&self) -> f64 {
        self.get_total_fuel_mass_consumed() / self.fuel.density()
    }

    /// `Engine::update`
    pub fn update(&mut self, dt: f64, bodies: &[RigidBody]) {
        let conditions = ThrottleConditions {
            speed: self.get_speed(bodies),
        };
        if let Some(throttle) = self.throttle.update(dt, &conditions) {
            self.set_throttle(throttle);
        }
    }

    /// Conditions for `VtecValvetrain::isVtecEnabled`.
    pub fn vtec_conditions(&self, bodies: &[RigidBody]) -> VtecConditions {
        VtecConditions {
            manifold_pressure: self.get_manifold_pressure(),
            speed: self.get_speed(bodies),
            throttle: self.throttle_value,
        }
    }

    /// `Engine::calculateDisplacement`
    pub fn calculate_displacement(&mut self) {
        // Numerical approximation, exactly as the original.
        const RESOLUTION: usize = 1000;

        let cylinder_count = self.pistons.len();
        let mut min_s = vec![f64::MAX; cylinder_count];
        let mut max_s = vec![-f64::MAX; cylinder_count];

        for j in 0..RESOLUTION {
            let crankshaft_angle = 2.0 * (j as f64 / RESOLUTION as f64) * constants::PI;

            for i in 0..cylinder_count {
                let piston = &self.pistons[i];
                let rod = &piston.rod;
                let bank = &piston.bank;

                if let Some((_, _, _, s)) = place_rod(rod, bank, crankshaft_angle) {
                    min_s[i] = min_s[i].min(s);
                    max_s[i] = max_s[i].max(s);
                }
            }
        }

        let mut displacement = 0.0;
        for i in 0..cylinder_count {
            let bank = &self.pistons[i].bank;

            if min_s[i] < max_s[i] {
                let r = bank.bore() / 2.0;
                displacement += constants::PI * r * r * (max_s[i] - min_s[i]);
            }
        }

        self.displacement = displacement;
    }

    /// Builds the combustion chambers. Called once the solver bodies exist and
    /// have been placed, mirroring `PistonEngineSimulator::loadSimulation` +
    /// `placeAndInitialize`.
    pub fn build_chambers(&mut self) {
        self.chambers.clear();

        for i in 0..self.pistons.len() {
            let piston = &self.pistons[i];
            let cylinder_index = piston.cylinder_index.max(0) as usize;
            let head = self.heads[piston.bank.index().max(0) as usize].clone();

            let intake_index = head
                .intake(cylinder_index)
                .expect("cylinder head must have an intake");
            let exhaust_index = head
                .exhaust_system(cylinder_index)
                .expect("cylinder head must have an exhaust system");

            let intake = &self.intakes[intake_index];
            let exhaust = &self.exhausts[exhaust_index];
            let header_primary_length = head.header_primary_length(cylinder_index);

            let params = ChamberParameters {
                bank: piston.bank.clone(),
                head,
                fuel: self.fuel.clone(),
                mean_piston_speed_to_turbulence: self.mean_piston_speed_to_turbulence.clone(),
                piston_body: piston.body,
                cylinder_index,
                compression_height: piston.compression_height,
                piston_displacement: piston.displacement,
                piston_blowby_k: piston.blowby_k,
                crankcase_pressure: self.crankcase_pressure,
                intake: intake_index,
                exhaust: exhaust_index,
                intake_runner_length: intake.runner_length,
                intake_velocity_decay: intake.velocity_decay,
                intake_runner_flow_rate: intake.runner_flow_rate,
                exhaust_primary_tube_length: exhaust.primary_tube_length,
                exhaust_collector_cross_section_area: exhaust.collector_cross_section_area,
                exhaust_velocity_decay: exhaust.velocity_decay,
                exhaust_primary_flow_rate: exhaust.primary_flow_rate,
                header_primary_length,
            };

            self.chambers.push(CombustionChamber::new(params));
        }
    }

    /// Sets the governor as the throttle controller.
    pub fn set_governor(&mut self, governor: Governor) {
        self.throttle = Box::new(governor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parts::{
        ConnectingRodParameters, CrankshaftParameters, CylinderBankParameters,
    };

    fn simple_geometry() -> (Arc<Crankshaft>, Arc<ConnectingRod>, Arc<CylinderBank>) {
        let crank = Arc::new(Crankshaft::new(CrankshaftParameters {
            mass: 10.0,
            flywheel_mass: 5.0,
            moment_of_inertia: 0.1,
            crank_throw: 0.05,
            rod_journals: 1,
            pos_x: 0.0,
            pos_y: 0.0,
            tdc: 0.0,
            friction_torque: 1.0,
        }));

        let bank = Arc::new(CylinderBank::new(CylinderBankParameters {
            angle: 0.0,
            bore: 0.08,
            deck_height: 0.2,
            cylinder_count: 1,
            position_x: 0.0,
            position_y: 0.0,
            display_depth: 0.4,
            index: 0,
        }));

        let rod = Arc::new(ConnectingRod::new(ConnectingRodParameters {
            center_of_mass: 0.0,
            length: 0.15,
            mass: 0.5,
            moment_of_inertia: 0.001,
            journal: 0,
            crankshaft: Some(crank.clone()),
            slave_throw: 0.0,
            rod_journals: 1,
            master: None,
            piston_bank: Some(bank.clone()),
            piston_cylinder_index: 0,
        }));

        (crank, rod, bank)
    }

    #[test]
    fn place_rod_solves_the_piston_travel_limits() {
        let (_crank, rod, bank) = simple_geometry();

        // The bank axis is +y (`angle = 0` gives dx = cos(pi/2) = 0, dy = 1),
        // so `s` is the distance from the crank centre to the wrist pin.
        let (_, _, _, s_mid) = place_rod(&rod, &bank, 0.0).expect("rod placeable");
        let (_, _, _, s_min) = place_rod(&rod, &bank, 3.0 * constants::PI / 2.0).expect("rod placeable");
        let (_, _, _, s_max) = place_rod(&rod, &bank, constants::PI / 2.0).expect("rod placeable");

        // Journal at +-x: s = sqrt(L^2 - throw^2).
        let expected_mid = (rod.length * rod.length - 0.05 * 0.05).sqrt();
        assert!((s_mid - expected_mid).abs() < 1e-12);

        // Journal aligned with the bank: s = L - throw and s = L + throw.
        assert!((s_min - (rod.length - 0.05)).abs() < 1e-12);
        assert!((s_max - (rod.length + 0.05)).abs() < 1e-12);
        assert!((s_max - s_min - 2.0 * 0.05).abs() < 1e-12);
    }
}
