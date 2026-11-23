//! Port of `include/combustion_chamber.h` / `src/combustion_chamber.cpp`.
//!
//! The chamber is both a thermodynamic volume (the cylinder gas, plus the
//! intake runner/manifold and exhaust runner/primary lumps) and an
//! `atg_scs::ForceGenerator` that applies the gas force and piston friction to
//! the piston body.

use std::sync::Arc;

use crate::constants;
use crate::fuel::{Fuel, Rng};
use crate::function::Function;
use crate::gas::{FlowParameters, GasSystem, Mix};
use crate::parts::{CylinderBank, CylinderHead, ExhaustSystem, Intake};
use crate::scs::{ForceGenerator, RigidBody, SystemState};
use crate::units;
use crate::utilities::clamp01;

/// `CombustionChamber::FlameEvent`
#[derive(Clone, Copy, Debug, Default)]
pub struct FlameEvent {
    pub lit_n: f64,
    pub total_n: f64,
    pub percentage_lit: f64,
    pub efficiency: f64,
    pub flame_speed: f64,
    pub last_volume: f64,
    pub travel_x: f64,
    pub travel_y: f64,
    pub global_mix: Mix,
}

/// `CombustionChamber::FrictionModelParams`
#[derive(Clone, Copy, Debug)]
pub struct FrictionModelParams {
    pub friction_coeff: f64,
    pub breakaway_friction: f64,
    pub breakaway_friction_velocity: f64,
    pub viscous_friction_coefficient: f64,
}

impl Default for FrictionModelParams {
    fn default() -> Self {
        Self {
            friction_coeff: 0.06,
            breakaway_friction: units::force(50.0, units::N),
            breakaway_friction_velocity: units::distance(0.1, units::M),
            viscous_friction_coefficient: units::force(20.0, units::N),
        }
    }
}

/// Everything `CombustionChamber::initialize` reads through its pointers,
/// flattened into values.
#[derive(Clone)]
pub struct ChamberParameters {
    pub bank: Arc<CylinderBank>,
    pub head: Arc<CylinderHead>,
    pub fuel: Arc<Fuel>,
    pub mean_piston_speed_to_turbulence: Arc<Function>,

    pub piston_body: usize,
    pub cylinder_index: usize,

    pub compression_height: f64,
    pub piston_displacement: f64,
    pub piston_blowby_k: f64,

    pub crankcase_pressure: f64,

    // From the intake/exhaust attached to this cylinder.
    pub intake: usize,
    pub exhaust: usize,
    pub intake_runner_length: f64,
    pub intake_velocity_decay: f64,
    pub intake_runner_flow_rate: f64,
    pub exhaust_primary_tube_length: f64,
    pub exhaust_collector_cross_section_area: f64,
    pub exhaust_velocity_decay: f64,
    pub exhaust_primary_flow_rate: f64,
    pub header_primary_length: f64,
}

/// `CombustionChamber`
pub struct CombustionChamber {
    pub mean_piston_speed_to_turbulence: Arc<Function>,
    pub system: GasSystem,
    pub intake_runner_and_manifold: GasSystem,
    pub exhaust_runner_and_primary: GasSystem,
    pub flame_event: FlameEvent,
    pub lit: bool,

    pub friction_model: FrictionModelParams,

    pub peak_temperature: f64,
    pub n_burnt_fuel: f64,

    // cylinder geometry (cached from bank/head)
    bank: Arc<CylinderBank>,
    head: Arc<CylinderHead>,
    fuel: Arc<Fuel>,
    compression_height: f64,
    piston_displacement: f64,

    intake: usize,
    exhaust: usize,

    intake_flow_rate: f64,
    exhaust_flow_rate: f64,

    manifold_to_runner_flow_rate: f64,
    primary_to_collector_flow_rate: f64,
    cylinder_cross_section_surface_area: f64,
    cylinder_width_approximation: f64,

    last_timestep_total_exhaust_flow: f64,
    last_timestep_total_intake_flow: f64,
    exhaust_flow: f64,

    crankcase_pressure: f64,

    pressure: [f64; Self::STATE_SAMPLES],
    piston_speed: [f64; Self::STATE_SAMPLES],

    lit_last_frame: bool,

    /// Body index of the piston this chamber acts on.
    pub piston_body: usize,
    /// Cylinder index within the head.
    pub cylinder_index: usize,

    /// Blow-by coefficient of the piston (`Piston::getBlowbyK`).
    pub piston_blowby_k: f64,
    /// Normal force between piston and cylinder wall, refreshed from the
    /// solver's line constraint after each step.
    pub cylinder_wall_force: f64,

    // cached part scalars
    intake_velocity_decay: f64,
    exhaust_velocity_decay: f64,
    exhaust_collector_cross_section_area: f64,
}

impl CombustionChamber {
    pub const STATE_SAMPLES: usize = 256;

    /// `CombustionChamber::initialize`
    pub fn new(params: ChamberParameters) -> Self {
        let bank = params.bank;
        let head = params.head;

        let intake_runner_cross_section = head.intake_runner_cross_section_area;
        let manifold_runner_length = params.intake_runner_length;
        let manifold_runner_volume = intake_runner_cross_section * manifold_runner_length;
        let total_intake_runner_volume = head.intake_runner_volume + manifold_runner_volume;
        let overall_intake_runner_length = total_intake_runner_volume / intake_runner_cross_section;
        let intake_runner_width = intake_runner_cross_section.sqrt();

        let mut intake_runner_and_manifold = GasSystem::default();
        intake_runner_and_manifold.initialize(
            units::pressure(1.0, units::ATM),
            total_intake_runner_volume,
            units::celcius(25.0),
            Mix::DEFAULT,
            5,
        );
        intake_runner_and_manifold.set_geometry(
            overall_intake_runner_length,
            intake_runner_width,
            1.0,
            0.0,
        );

        let exhaust_runner_cross_section = head.exhaust_runner_cross_section_area;
        let exhaust_runner_width = exhaust_runner_cross_section.sqrt();
        let exhaust_tube_length = params.exhaust_primary_tube_length + params.header_primary_length;
        let exhaust_tube_volume = exhaust_runner_cross_section * exhaust_tube_length;
        let total_exhaust_runner_volume = head.exhaust_runner_volume + exhaust_tube_volume;
        let overall_exhaust_runner_length =
            total_exhaust_runner_volume / exhaust_runner_cross_section;

        let mut exhaust_runner_and_primary = GasSystem::default();
        exhaust_runner_and_primary.initialize(
            units::pressure(1.0, units::ATM),
            total_exhaust_runner_volume,
            units::celcius(25.0),
            Mix::DEFAULT,
            5,
        );
        exhaust_runner_and_primary.set_geometry(
            overall_exhaust_runner_length,
            exhaust_runner_width,
            1.0,
            0.0,
        );

        let bore_r = bank.bore() / 2.0;
        let cylinder_cross_section_surface_area = constants::PI * bore_r * bore_r;
        let cylinder_width_approximation = cylinder_cross_section_surface_area.sqrt();

        let mut chamber = Self {
            mean_piston_speed_to_turbulence: params.mean_piston_speed_to_turbulence,
            system: GasSystem::default(),
            intake_runner_and_manifold,
            exhaust_runner_and_primary,
            flame_event: FlameEvent {
                efficiency: 1.0,
                ..Default::default()
            },
            lit: false,
            friction_model: FrictionModelParams::default(),
            peak_temperature: 0.0,
            n_burnt_fuel: 0.0,
            bank,
            head,
            fuel: params.fuel,
            compression_height: params.compression_height,
            piston_displacement: params.piston_displacement,
            intake: params.intake,
            exhaust: params.exhaust,
            intake_flow_rate: 0.0,
            exhaust_flow_rate: 0.0,
            manifold_to_runner_flow_rate: params.intake_runner_flow_rate,
            primary_to_collector_flow_rate: params.exhaust_primary_flow_rate,
            cylinder_cross_section_surface_area,
            cylinder_width_approximation,
            last_timestep_total_exhaust_flow: 0.0,
            last_timestep_total_intake_flow: 0.0,
            exhaust_flow: 0.0,
            crankcase_pressure: params.crankcase_pressure,
            pressure: [0.0; Self::STATE_SAMPLES],
            piston_speed: [0.0; Self::STATE_SAMPLES],
            lit_last_frame: false,
            piston_body: params.piston_body,
            cylinder_index: params.cylinder_index,
            piston_blowby_k: params.piston_blowby_k,
            cylinder_wall_force: 0.0,
            intake_velocity_decay: params.intake_velocity_decay,
            exhaust_velocity_decay: params.exhaust_velocity_decay,
            exhaust_collector_cross_section_area: params.exhaust_collector_cross_section_area,
        };

        // The initial volume/geometry is set by the engine once the pistons have
        // been placed (`PistonEngineSimulator::placeAndInitialize`).
        let height = chamber.get_volume_empty() / chamber.cylinder_cross_section_surface_area;
        chamber.system.set_geometry(
            chamber.cylinder_width_approximation,
            height,
            1.0,
            0.0,
        );

        chamber
    }

    /// `CombustionChamber::getVolume` with the piston at its *current* solver
    /// position.
    pub fn get_volume(&self, bodies: &[RigidBody]) -> f64 {
        let combustion_port_volume = self.head.combustion_chamber_volume;
        let bank = &self.bank;

        let area = bank.bore_surface_area();
        let body = &bodies[self.piston_body];
        let rel_x = body.p_x - bank.x();
        let rel_y = body.p_y - bank.y();
        let s = rel_x * bank.dx() + rel_y * bank.dy();
        let sweep = area * (bank.deck_height() - s - self.compression_height);

        sweep + combustion_port_volume - self.piston_displacement
    }

    /// Volume with the piston assumed at the origin of its body (used only
    /// during construction, before the bodies exist).
    fn get_volume_empty(&self) -> f64 {
        let bank = &self.bank;
        let area = bank.bore_surface_area();
        let s = 0.0;
        area * (bank.deck_height() - s - self.compression_height) + self.head.combustion_chamber_volume
            - self.piston_displacement
    }

    /// `CombustionChamber::pistonSpeed`
    pub fn piston_speed(&self, bodies: &[RigidBody]) -> f64 {
        let bank = &self.bank;
        let body = &bodies[self.piston_body];
        body.v_x * bank.dx() + body.v_y * bank.dy()
    }

    /// `CombustionChamber::calculateMeanPistonSpeed`
    pub fn calculate_mean_piston_speed(&self) -> f64 {
        let mut avg = 0.0;
        for i in 0..Self::STATE_SAMPLES {
            avg += self.piston_speed[i];
        }

        avg /= Self::STATE_SAMPLES as f64;
        avg
    }

    /// `CombustionChamber::calculateFiringPressure`
    pub fn calculate_firing_pressure(&self) -> f64 {
        let mut firing_pressure = 0.0;
        for i in 0..Self::STATE_SAMPLES {
            if self.pressure[i] > firing_pressure {
                firing_pressure = self.pressure[i];
            }
        }

        firing_pressure
    }

    /// `CombustionChamber::popLitLastFrame`
    pub fn pop_lit_last_frame(&mut self) -> bool {
        let lit = self.lit_last_frame;
        self.lit_last_frame = false;
        lit
    }

    /// `CombustionChamber::ignite`
    pub fn ignite(&mut self, bodies: &[RigidBody], rng: &mut Rng) {
        if self.lit {
            return;
        }

        if self.system.mix().p_fuel == 0.0 {
            return;
        }

        let afr = self.system.mix().p_o2 / self.system.mix().p_fuel;
        let equivalence_ratio = afr / self.fuel.molecular_afr();
        if equivalence_ratio < 0.5 {
            return;
        } else if equivalence_ratio > 1.9 {
            return;
        }

        let ideal_inert = self.system.mix().p_o2 / 0.7;
        let dilution = (self.system.mix().p_inert / ideal_inert) - 1.0;

        self.flame_event.last_volume = self.get_volume(bodies);
        self.flame_event.travel_x = 0.0;
        self.flame_event.travel_y = 0.0;
        self.flame_event.lit_n = 0.0;
        self.flame_event.total_n = self.system.n();
        self.flame_event.percentage_lit = 0.0;
        self.flame_event.global_mix = self.system.mix();
        self.lit = true;
        self.lit_last_frame = true;

        let randomness = self.fuel.burning_efficiency_randomness();
        let low_efficiency_attenuation = self.fuel.low_efficiency_attenuation();
        let max_burning_efficiency = self.fuel.max_burning_efficiency();
        let max_turbulence_effect = self.fuel.max_turbulence_effect();
        let max_dilution_effect = self.fuel.max_dilution_effect();

        let turbulence = self
            .mean_piston_speed_to_turbulence
            .sample_triangle(self.calculate_mean_piston_speed());

        let mixing_factor = 1.0
            - (clamp01(turbulence / max_turbulence_effect)
                * clamp01(1.0 - dilution / max_dilution_effect));

        let rand_s = low_efficiency_attenuation
            * ((1.0 - randomness) + randomness * rng.next_unit());

        let efficiency_attenuation = mixing_factor * rand_s + (1.0 - mixing_factor);
        self.flame_event.efficiency =
            efficiency_attenuation * max_burning_efficiency;

        self.flame_event.flame_speed = self.fuel.flame_speed(
            turbulence,
            afr,
            self.system.temperature(),
            self.system.pressure(),
            self.calculate_firing_pressure(),
            units::pressure(160.0, units::PSI),
        );
    }

    /// `CombustionChamber::update`
    pub fn update(
        &mut self,
        bodies: &[RigidBody],
        crank_angle: f64,
        crank_cycle_angle: f64,
        vtec: bool,
    ) {
        self.system.set_volume(self.get_volume(bodies));

        self.update_cycle_states(crank_cycle_angle, bodies);

        self.intake_flow_rate = self
            .head
            .intake_flow_rate(self.cylinder_index, crank_angle, vtec);
        self.exhaust_flow_rate = self
            .head
            .exhaust_flow_rate(self.cylinder_index, crank_angle, vtec);
    }

    /// `CombustionChamber::updateCycleStates`
    fn update_cycle_states(&mut self, crank_cycle_angle: f64, bodies: &[RigidBody]) {
        let mut crank_angle = crank_cycle_angle;
        if crank_angle.is_nan() || crank_angle.is_infinite() {
            crank_angle = 0.0;
        }

        let i = ((crank_angle / (4.0 * constants::PI)) * (Self::STATE_SAMPLES as f64 - 1.0))
            .round() as usize;

        let i = i.min(Self::STATE_SAMPLES - 1);

        self.piston_speed[i] = self.piston_speed(bodies).abs();
        self.pressure[i] = self.system.pressure();
    }

    /// `CombustionChamber::flow`
    pub fn flow(
        &mut self,
        dt: f64,
        bodies: &[RigidBody],
        intakes: &mut [Intake],
        exhausts: &mut [ExhaustSystem],
    ) {
        if self.system.temperature() > self.peak_temperature {
            self.peak_temperature = self.system.temperature();
        }

        let bodies_volume = self.get_volume(bodies);

        let cylinder_height = bodies_volume / self.cylinder_cross_section_surface_area;
        let cylinder_surface_area = cylinder_height
            * constants::PI
            * self.bank.bore()
            + self.cylinder_cross_section_surface_area * 2.0;

        let d_t = units::celcius(90.0) - self.system.temperature();

        self.system.change_energy(d_t * cylinder_surface_area * 100.0 * dt);
        self.system.flow_env(
            self.piston_blowby_k,
            dt,
            self.crankcase_pressure,
            units::celcius(25.0),
            Mix::DEFAULT,
        );

        let intake = &mut intakes[self.intake];
        let exhaust = &mut exhausts[self.exhaust];

        let mut flow_params = FlowParameters {
            dt,
            ..Default::default()
        };

        // manifold -> runner/manifold lump
        flow_params.k_flow = self.manifold_to_runner_flow_rate;
        flow_params.cross_section_area_0 = intake.cross_section_area;
        flow_params.cross_section_area_1 = self.head.intake_runner_cross_section_area;
        flow_params.direction_x = 1.0;
        flow_params.direction_y = 0.0;
        GasSystem::flow(
            &mut intake.system,
            &mut self.intake_runner_and_manifold,
            &flow_params,
        );

        self.intake_runner_and_manifold.dissipate_excess_velocity();

        // runner -> cylinder
        flow_params.k_flow = self.intake_flow_rate;
        flow_params.cross_section_area_0 = self.head.intake_runner_cross_section_area;
        flow_params.cross_section_area_1 = bodies_volume / cylinder_height;
        flow_params.direction_x = 1.0;
        flow_params.direction_y = 0.0;
        let intake_flow = GasSystem::flow(
            &mut self.intake_runner_and_manifold,
            &mut self.system,
            &flow_params,
        );

        self.intake_runner_and_manifold.dissipate_excess_velocity();
        self.system.dissipate_excess_velocity();

        // cylinder -> exhaust runner
        flow_params.k_flow = self.exhaust_flow_rate;
        flow_params.cross_section_area_0 = bodies_volume / cylinder_height;
        flow_params.cross_section_area_1 = self.head.exhaust_runner_cross_section_area;
        flow_params.direction_x = 1.0;
        flow_params.direction_y = 0.0;
        let exhaust_flow = GasSystem::flow(
            &mut self.system,
            &mut self.exhaust_runner_and_primary,
            &flow_params,
        );

        self.system.dissipate_excess_velocity();
        self.exhaust_runner_and_primary.dissipate_excess_velocity();

        // exhaust runner -> collector
        flow_params.k_flow = self.primary_to_collector_flow_rate;
        flow_params.cross_section_area_0 = self.head.exhaust_runner_cross_section_area;
        flow_params.cross_section_area_1 = self.exhaust_collector_cross_section_area;
        flow_params.direction_x = 1.0;
        flow_params.direction_y = 0.0;
        GasSystem::flow(
            &mut self.exhaust_runner_and_primary,
            &mut exhaust.system,
            &flow_params,
        );

        self.intake_runner_and_manifold
            .update_velocity(dt, self.intake_velocity_decay);
        self.system.update_velocity(dt, 0.5);
        self.exhaust_runner_and_primary
            .update_velocity(dt, self.exhaust_velocity_decay);

        if intake_flow.abs() > 1e-9 && self.lit {
            self.lit = false;
        }

        self.exhaust_flow = exhaust_flow;
        self.last_timestep_total_exhaust_flow += exhaust_flow;
        self.last_timestep_total_intake_flow += intake_flow;

        if self.lit {
            let total_travel_x = self.bank.bore() / 2.0;
            let total_travel_y = bodies_volume / self.bank.bore_surface_area();
            let expansion = bodies_volume / self.flame_event.last_volume;
            let last_travel_x = self.flame_event.travel_x;
            let last_travel_y = self.flame_event.travel_y * expansion;
            let flame_speed = self.flame_event.flame_speed;

            self.flame_event.travel_x =
                (last_travel_x + dt * flame_speed).min(total_travel_x);
            self.flame_event.travel_y =
                (last_travel_y + dt * flame_speed).min(total_travel_y);

            if last_travel_x < self.flame_event.travel_x
                || last_travel_y < self.flame_event.travel_y
            {
                let burned_volume = self.flame_event.travel_x
                    * self.flame_event.travel_x
                    * constants::PI
                    * self.flame_event.travel_y;
                let prev_burned_volume =
                    last_travel_x * last_travel_x * constants::PI * last_travel_y;
                let lit_volume = burned_volume - prev_burned_volume;
                let n = (lit_volume / bodies_volume) * self.system.n();

                let fuel_burned = self
                    .system
                    .react(n * self.flame_event.efficiency, self.flame_event.global_mix);
                let mass_fuel_burned = fuel_burned * self.fuel.molecular_mass();
                self.system
                    .change_energy(mass_fuel_burned * self.fuel.energy_density());

                self.flame_event.lit_n += n;
                self.flame_event.percentage_lit += lit_volume / bodies_volume;

                self.n_burnt_fuel += mass_fuel_burned;
            } else {
                self.lit = false;
            }

            self.flame_event.last_volume = bodies_volume;
        }
    }

    /// `CombustionChamber::lastEventAfr`
    pub fn last_event_afr(&self) -> f64 {
        let total_fuel = self.flame_event.global_mix.p_fuel * self.flame_event.total_n;
        let total_oxygen = self.flame_event.global_mix.p_o2 * self.flame_event.total_n;
        let total_inert = self.flame_event.global_mix.p_inert * self.flame_event.total_n;

        let octane_molar_mass = units::mass(114.23, units::G);
        let oxygen_molar_mass = units::mass(31.9988, units::G);
        let nitrogen_molar_mass = units::mass(28.014, units::G);

        if total_fuel == 0.0 {
            0.0
        } else {
            (oxygen_molar_mass * total_oxygen + total_inert * nitrogen_molar_mass)
                / (total_fuel * octane_molar_mass)
        }
    }

    /// `CombustionChamber::calculateFrictionForce`
    fn calculate_friction_force(&self, v_s: f64) -> f64 {
        let cylinder_wall_force = self.cylinder_wall_force;

        let f_coul = self.friction_model.friction_coeff * cylinder_wall_force;
        let v_st = self.friction_model.breakaway_friction_velocity * constants::ROOT_2;
        let v_coul = self.friction_model.breakaway_friction_velocity / 10.0;
        let f_brk = self.friction_model.breakaway_friction;
        let v = v_s.abs();

        let f_0 = constants::ROOT_2 * constants::E * (f_brk - f_coul);
        let f_1 = v / v_st;
        let f_2 = (-f_1 * f_1).exp() * f_1;
        let f_3 = f_coul * (v / v_coul).tanh();
        let f_4 = self.friction_model.viscous_friction_coefficient * v;

        f_0 * f_2 + f_3 + f_4
    }

    /// `CombustionChamber::getFrictionForce`
    pub fn get_friction_force(&self, bodies: &[RigidBody]) -> f64 {
        let bank = &self.bank;
        let body = &bodies[self.piston_body];
        let v_s = body.v_x * bank.dx() + body.v_y * bank.dy();

        self.calculate_friction_force(v_s)
    }

    pub fn reset_last_timestep_exhaust_flow(&mut self) {
        self.last_timestep_total_exhaust_flow = 0.0;
    }

    pub fn last_timestep_exhaust_flow(&self) -> f64 {
        self.last_timestep_total_exhaust_flow
    }

    pub fn reset_last_timestep_intake_flow(&mut self) {
        self.last_timestep_total_intake_flow = 0.0;
    }

    pub fn last_timestep_intake_flow(&self) -> f64 {
        self.last_timestep_total_intake_flow
    }

    pub fn last_iteration_exhaust_flow(&self) -> f64 {
        self.exhaust_flow
    }
}

impl ForceGenerator for CombustionChamber {
    /// `CombustionChamber::apply`
    fn apply(&mut self, state: &mut SystemState) {
        let bank = &self.bank;
        let area = (bank.bore() * bank.bore() / 4.0) * constants::PI;
        let body = self.piston_body;
        let v_x = state.v_x[body];
        let v_y = state.v_y[body];

        let v_s = v_x * bank.dx() + v_y * bank.dy();

        let pressure_differential = self.system.pressure() - self.crankcase_pressure;
        let force = -area * pressure_differential;

        assert!(force.is_finite());

        let limit = 1e-3;
        let abs_v_s = v_s.abs().min(limit);
        let attenuation = abs_v_s / limit;

        let f = self.calculate_friction_force(v_s) * attenuation;
        let f_fric = if v_s > 0.0 { -f } else { f };

        state.apply_force(
            0.0,
            0.0,
            (force + f_fric) * bank.dx(),
            (force + f_fric) * bank.dy(),
            body,
        );
    }
}
