//! `dynamometer`, `starter_motor`, `transmission`, `vehicle`,
//! `vehicle_drag_constraint` and `governor`.

use std::cell::Cell;
use std::sync::Arc;

use crate::constants;
use crate::parts::Throttle;
use crate::scs::{
    Constraint, ConstraintForces, Output, RigidBody, SystemState, MAX_BODY_COUNT,
};
use crate::units;
use crate::utilities::clamp;

// ------------------------------------------------------------ dynamometer

/// `Dynamometer`
#[derive(Clone, Debug)]
pub struct Dynamometer {
    pub body: i32,
    pub rotation_speed: f64,
    pub enabled: bool,
    pub hold: bool,
    pub ks: f64,
    pub kd: f64,
    pub max_torque: f64,
    pub forces: ConstraintForces,
}

impl Default for Dynamometer {
    fn default() -> Self {
        Self {
            body: -1,
            rotation_speed: 0.0,
            enabled: false,
            hold: false,
            ks: 10.0,
            kd: 1.0,
            max_torque: units::torque(10000.0, units::FT_LB),
            forces: ConstraintForces::default(),
        }
    }
}

impl Dynamometer {
    pub fn connect_crankshaft(&mut self, body: usize) {
        self.body = body as i32;
    }

    /// `Dynamometer::getTorque`
    pub fn get_torque(&self, bodies: &[RigidBody]) -> f64 {
        let v_theta = bodies[self.body as usize].v_theta;
        if v_theta > 0.0 {
            -self.forces.f_t[0][0]
        } else {
            self.forces.f_t[0][0]
        }
    }
}

impl Constraint for Dynamometer {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body, -1]
    }

    /// `Dynamometer::calculate`
    fn calculate(&mut self, output: &mut Output, state: &SystemState, _bodies: &[RigidBody]) {
        output.j[0][0] = 0.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = 1.0;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = 0.0;

        output.ks[0] = self.ks;
        output.kd[0] = self.kd;

        output.c[0] = 0.0;

        let v_theta = state.v_theta[self.body as usize];
        if v_theta < 0.0 {
            output.v_bias[0] = self.rotation_speed;
            output.limits[0][0] = if self.hold && self.enabled {
                -self.max_torque
            } else {
                0.0
            };
            output.limits[0][1] = if self.enabled { self.max_torque } else { 0.0 };
        } else {
            output.v_bias[0] = -self.rotation_speed;
            output.limits[0][0] = if self.enabled { -self.max_torque } else { 0.0 };
            output.limits[0][1] = if self.hold && self.enabled {
                self.max_torque
            } else {
                0.0
            };
        }
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn output_torque(&self, bodies: &[RigidBody]) -> f64 {
        self.get_torque(bodies)
    }
}

// ------------------------------------------------------------ starter motor

/// `StarterMotor`
#[derive(Clone, Debug)]
pub struct StarterMotor {
    pub body: i32,
    pub ks: f64,
    pub kd: f64,
    pub max_torque: f64,
    pub rotation_speed: f64,
    pub enabled: bool,
    pub forces: ConstraintForces,
}

impl Default for StarterMotor {
    fn default() -> Self {
        Self {
            body: -1,
            ks: 10.0,
            kd: 1.0,
            max_torque: units::torque(80.0, units::FT_LB),
            rotation_speed: -units::rpm(200.0),
            enabled: false,
            forces: ConstraintForces::default(),
        }
    }
}

impl StarterMotor {
    pub fn connect_crankshaft(&mut self, body: usize) {
        self.body = body as i32;
    }
}

impl Constraint for StarterMotor {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body, -1]
    }

    /// `StarterMotor::calculate`
    fn calculate(&mut self, output: &mut Output, _state: &SystemState, _bodies: &[RigidBody]) {
        output.j[0][0] = 0.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = 1.0;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = 0.0;

        output.ks[0] = self.ks;
        output.kd[0] = self.kd;

        output.c[0] = 0.0;

        output.v_bias[0] = -self.rotation_speed;

        if self.rotation_speed < 0.0 {
            output.limits[0][0] = if self.enabled { -self.max_torque } else { 0.0 };
            output.limits[0][1] = 0.0;
        } else {
            output.limits[0][0] = 0.0;
            output.limits[0][1] = if self.enabled { self.max_torque } else { 0.0 };
        }
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------------ vehicle

/// `Vehicle`
#[derive(Clone, Debug, Default)]
pub struct Vehicle {
    pub mass: f64,
    pub drag_coefficient: f64,
    pub cross_section_area: f64,
    pub diff_ratio: f64,
    pub tire_radius: f64,
    pub travelled_distance: Cell<f64>,
    pub rolling_resistance: f64,

    /// Body index of the rotating mass (set by `add_to_system`).
    pub rotating_mass: Cell<usize>,
}

impl Vehicle {
    pub fn new(
        mass: f64,
        drag_coefficient: f64,
        cross_section_area: f64,
        diff_ratio: f64,
        tire_radius: f64,
        rolling_resistance: f64,
    ) -> Self {
        Self {
            mass,
            drag_coefficient,
            cross_section_area,
            diff_ratio,
            tire_radius,
            travelled_distance: Cell::new(0.0),
            rolling_resistance,
            rotating_mass: Cell::new(usize::MAX),
        }
    }

    pub fn add_to_system(&self, rotating_mass: usize) {
        self.rotating_mass.set(rotating_mass);
    }

    /// `Vehicle::update`
    pub fn update(&self, dt: f64, bodies: &[RigidBody]) {
        self.travelled_distance
            .set(self.travelled_distance.get() + self.get_speed(bodies) * dt);
    }

    /// `Vehicle::getSpeed`
    pub fn get_travelled_distance(&self) -> f64 {
        self.travelled_distance.get()
    }

    pub fn get_speed(&self, bodies: &[RigidBody]) -> f64 {
        let rotating_mass = &bodies[self.rotating_mass.get()];
        let e_r = 0.5 * rotating_mass.i * rotating_mass.v_theta * rotating_mass.v_theta;
        (2.0 * e_r / self.mass).sqrt()
    }

    /// `Vehicle::linearForceToVirtualTorque`
    pub fn linear_force_to_virtual_torque(&self, force: f64, bodies: &[RigidBody]) -> f64 {
        let rotating_mass = &bodies[self.rotating_mass.get()];
        let rotation_to_kinetic_ratio = (rotating_mass.i / self.mass).sqrt();
        rotation_to_kinetic_ratio * force
    }
}

/// `VehicleDragConstraint`
#[derive(Clone, Debug)]
pub struct VehicleDragConstraint {
    pub body: i32,
    pub vehicle: Arc<Vehicle>,
    pub ks: f64,
    pub kd: f64,
    pub forces: ConstraintForces,
}

impl VehicleDragConstraint {
    pub fn new(body: usize, vehicle: Arc<Vehicle>) -> Self {
        Self {
            body: body as i32,
            vehicle,
            ks: 10.0,
            kd: 1.0,
            forces: ConstraintForces::default(),
        }
    }
}

impl Constraint for VehicleDragConstraint {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.body, -1]
    }

    /// `VehicleDragConstraint::calculate`
    fn calculate(&mut self, output: &mut Output, _state: &SystemState, bodies: &[RigidBody]) {
        output.c[0] = 0.0;

        output.j[0][0] = 0.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = -1.0;

        output.j[0][3] = 0.0;
        output.j[0][4] = 0.0;
        output.j[0][5] = 1.0;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = 0.0;

        output.j_dot[0][3] = 0.0;
        output.j_dot[0][4] = 0.0;
        output.j_dot[0][5] = 0.0;

        output.kd[0] = self.kd;
        output.ks[0] = self.ks;

        output.v_bias[0] = 0.0;

        let air_density = units::AIR_MOLECULAR_MASS * units::pressure(1.0, units::ATM)
            / (constants::R * units::celcius(25.0));
        let vehicle = &self.vehicle;
        let v = vehicle.get_speed(bodies);
        let v_squared = v * v;
        let c_d = vehicle.drag_coefficient;
        let a = vehicle.cross_section_area;
        let rolling_resistance = vehicle.rolling_resistance;

        output.limits[0][0] = -vehicle.linear_force_to_virtual_torque(
            rolling_resistance + 0.5 * air_density * v_squared * c_d * a,
            bodies,
        );
        output.limits[0][1] = 0.0;
    }

    fn forces(&self) -> &ConstraintForces {
        &self.forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.forces
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ------------------------------------------------------------- transmission

/// `Transmission`
#[derive(Debug)]
pub struct Transmission {
    pub gear: i32,
    pub new_gear: i32,
    pub gear_ratios: Vec<f64>,
    pub max_clutch_torque: f64,
    pub clutch_pressure: f64,

    pub clutch_body1: i32,
    pub clutch_body2: i32,
    pub clutch_min_torque: f64,
    pub clutch_max_torque: f64,
    pub clutch_forces: ConstraintForces,

    pub rotating_mass: usize,
    pub vehicle: Option<Arc<Vehicle>>,
    pub crank_body: i32,
}

impl Default for Transmission {
    fn default() -> Self {
        Self {
            gear: -1,
            new_gear: -1,
            gear_ratios: Vec::new(),
            max_clutch_torque: units::torque(1000.0, units::FT_LB),
            clutch_pressure: 0.0,
            clutch_body1: -1,
            clutch_body2: -1,
            clutch_min_torque: -f64::MAX,
            clutch_max_torque: f64::MAX,
            clutch_forces: ConstraintForces::default(),
            rotating_mass: usize::MAX,
            vehicle: None,
            crank_body: -1,
        }
    }
}

impl Transmission {
    pub fn new(gear_ratios: Vec<f64>, max_clutch_torque: f64) -> Self {
        Self {
            gear_ratios,
            max_clutch_torque,
            ..Default::default()
        }
    }

    /// `Transmission::addToSystem`
    pub fn add_to_system(&mut self, crank_body: usize, rotating_mass: usize, vehicle: Arc<Vehicle>) {
        self.rotating_mass = rotating_mass;
        self.vehicle = Some(vehicle);

        self.clutch_body1 = crank_body as i32;
        self.clutch_body2 = rotating_mass as i32;
    }

    /// `Transmission::update`
    pub fn update(&mut self, _dt: f64) {
        if self.gear == -1 {
            self.clutch_min_torque = 0.0;
            self.clutch_max_torque = 0.0;
        } else {
            self.clutch_min_torque = -self.max_clutch_torque * self.clutch_pressure;
            self.clutch_max_torque = self.max_clutch_torque * self.clutch_pressure;
        }
    }

    /// `Transmission::changeGear`
    pub fn change_gear(&mut self, new_gear: i32, bodies: &mut [RigidBody]) {
        if new_gear < -1 || new_gear >= self.gear_ratios.len() as i32 {
            return;
        } else if new_gear != -1 {
            let vehicle = self.vehicle.as_ref().expect("vehicle not attached");
            let m_car = vehicle.mass;
            let gear_ratio = self.gear_ratios[new_gear as usize];
            let diff_ratio = vehicle.diff_ratio;
            let tire_radius = vehicle.tire_radius;
            let f = tire_radius / (diff_ratio * gear_ratio);

            let new_i = m_car * f * f;
            let rot = &bodies[self.rotating_mass];
            let e_r = 0.5 * rot.i * rot.v_theta * rot.v_theta;
            let new_v_theta = if rot.v_theta < 0.0 {
                -(e_r * 2.0 / new_i).sqrt()
            } else {
                (e_r * 2.0 / new_i).sqrt()
            };

            let rot = &mut bodies[self.rotating_mass];
            rot.i = new_i;
            rot.p_x = 0.0;
            rot.p_y = 0.0;
            rot.m = m_car;
            rot.v_theta = new_v_theta;
        }

        self.gear = new_gear;
    }
}

impl Constraint for Transmission {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; MAX_BODY_COUNT] {
        [self.clutch_body1, self.clutch_body2]
    }

    /// `atg_scs::ClutchConstraint::calculate` with the transmission's limits.
    fn calculate(&mut self, output: &mut Output, _state: &SystemState, _bodies: &[RigidBody]) {
        output.c[0] = 0.0;

        output.j[0][0] = 0.0;
        output.j[0][1] = 0.0;
        output.j[0][2] = -1.0;

        output.j[0][3] = 0.0;
        output.j[0][4] = 0.0;
        output.j[0][5] = 1.0;

        output.j_dot[0][0] = 0.0;
        output.j_dot[0][1] = 0.0;
        output.j_dot[0][2] = 0.0;

        output.j_dot[0][3] = 0.0;
        output.j_dot[0][4] = 0.0;
        output.j_dot[0][5] = 0.0;

        output.kd[0] = 1.0;
        output.ks[0] = 10.0;

        output.v_bias[0] = 0.0;

        output.limits[0][0] = self.clutch_min_torque;
        output.limits[0][1] = self.clutch_max_torque;
    }

    fn forces(&self) -> &ConstraintForces {
        &self.clutch_forces
    }

    fn forces_mut(&mut self) -> &mut ConstraintForces {
        &mut self.clutch_forces
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ----------------------------------------------------------------- governor

/// `Governor::Parameters`
#[derive(Clone, Copy, Debug)]
pub struct GovernorParameters {
    pub min_speed: f64,
    pub max_speed: f64,
    pub min_velocity: f64,
    pub max_velocity: f64,
    pub k_s: f64,
    pub k_d: f64,
    pub gamma: f64,
}

impl Default for GovernorParameters {
    fn default() -> Self {
        Self {
            min_speed: 0.0,
            max_speed: 0.0,
            min_velocity: 0.0,
            max_velocity: 0.0,
            k_s: 0.0,
            k_d: 0.0,
            gamma: 1.0,
        }
    }
}

/// `Governor`
#[derive(Clone, Copy, Debug)]
pub struct Governor {
    pub min_speed: f64,
    pub max_speed: f64,
    pub target_speed: f64,
    pub current_throttle: f64,
    pub velocity: f64,
    pub min_velocity: f64,
    pub max_velocity: f64,
    pub k_s: f64,
    pub k_d: f64,
    pub gamma: f64,
    pub speed_control: f64,
}

impl Governor {
    pub fn new(params: GovernorParameters) -> Self {
        Self {
            min_speed: params.min_speed,
            max_speed: params.max_speed,
            target_speed: 0.0,
            current_throttle: 1.0,
            velocity: 0.0,
            min_velocity: params.min_velocity,
            max_velocity: params.max_velocity,
            k_s: params.k_s,
            k_d: params.k_d,
            gamma: params.gamma,
            speed_control: 0.0,
        }
    }
}

impl Throttle for Governor {
    fn set_speed_control(&mut self, s: f64) {
        self.speed_control = s;
        self.target_speed = (1.0 - s) * self.min_speed + s * self.max_speed;
    }

    fn speed_control(&self) -> f64 {
        self.speed_control
    }

    /// `Governor::update` — returns the new throttle value.
    fn update(&mut self, dt: f64, conditions: &crate::parts::ThrottleConditions) -> Option<f64> {
        let current_speed = conditions.speed;
        let ds = self.target_speed * self.target_speed - current_speed * current_speed;

        self.velocity += dt * -ds * self.k_s - self.velocity * dt * self.k_d;
        self.velocity = clamp(self.velocity, self.min_velocity, self.max_velocity);

        if current_speed.abs() < (0.5 * self.min_speed).abs() {
            self.velocity = 0.0;
            self.current_throttle = 1.0;
        }

        self.current_throttle += self.velocity * dt;
        self.current_throttle = clamp(self.current_throttle, 0.0, 1.0);

        Some(1.0 - (1.0 - self.current_throttle).powf(self.gamma))
    }
}
