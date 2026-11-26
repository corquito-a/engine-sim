//! Port of `include/simulator.h`, `src/simulator.cpp`,
//! `include/piston_engine_simulator.h` and `src/piston_engine_simulator.cpp`.
//!
//! The audio path is wired in: `write_to_synthesizer` builds the per-exhaust
//! flow signal (including the pipe-length delay), and `synthesizer` turns it
//! into 44.1 kHz `int16` audio. The rendering "thread" of the original is
//! replaced by an explicit [`Simulator::render_audio_block`] call, which makes
//! the audio output reproducible sample-for-sample.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::constants;
use crate::drivetrain::{
    Dynamometer, StarterMotor, Transmission, Vehicle, VehicleDragConstraint,
};
use crate::engine::Engine;
use crate::dsp::DelayFilter;
use crate::fuel::Rng;
use crate::synth::{AudioParameters, Synthesizer, SynthesizerParameters};
use crate::scs::{
    ClutchConstraint, Constraint, FixedPositionConstraint, ForceGenerator, LineConstraint,
    LinkConstraint, RigidBody, RigidBodySystem, RotationFrictionConstraint,
};
use crate::units;

/// `Simulator::DynoTorqueSamples`
pub const DYNO_TORQUE_SAMPLES: usize = 512;

/// `Simulator`
pub struct Simulator {
    pub engine: Engine,
    pub vehicle: Arc<Vehicle>,
    pub transmission: Arc<Mutex<Transmission>>,

    pub solver: RigidBodySystem,

    pub simulation_speed: f64,
    pub target_synthesizer_latency: f64,
    pub simulation_frequency: f64,
    pub steps: usize,
    pub current_iteration: usize,

    pub filtered_engine_speed: f64,
    pub dyno_torque_samples: [f64; DYNO_TORQUE_SAMPLES],
    pub last_dyno_torque_sample: i32,

    pub physics_processing_time: f64,
    pub simulation_start: Instant,

    /// `Simulator::m_synthesizer`
    pub synthesizer: Synthesizer,
    /// `PistonEngineSimulator::m_delayFilters` — one per cylinder, delaying the
    /// exhaust pulse by the header-primary + pipe length.
    pub delay_filters: Vec<DelayFilter>,

    // constraint handles (indices into `solver.constraints`)
    pub crank_constraints: Vec<usize>,
    pub cylinder_wall_constraints: Vec<usize>,
    pub link_constraints: Vec<usize>,
    pub crankshaft_friction_constraints: Vec<usize>,
    pub dyno_constraint: usize,
    pub starter_constraint: usize,

    pub vehicle_mass_body: usize,

    pub fluid_simulation_steps: usize,
    pub exhaust_flow_staging_buffer: Vec<f64>,

    pub rng: Rng,
}

impl Simulator {
    /// `Simulator::initialize` + `PistonEngineSimulator::loadSimulation`.
    pub fn new(engine: Engine, vehicle: Vehicle, transmission: Transmission) -> Self {
        let vehicle = Arc::new(vehicle);

        let mut sim = Self {
            engine,
            vehicle: vehicle.clone(),
            transmission: Arc::new(Mutex::new(transmission)),
            solver: RigidBodySystem::new(),
            simulation_speed: 1.0,
            target_synthesizer_latency: 0.1,
            simulation_frequency: 10000.0,
            steps: 0,
            current_iteration: 0,
            filtered_engine_speed: 0.0,
            dyno_torque_samples: [0.0; DYNO_TORQUE_SAMPLES],
            last_dyno_torque_sample: 0,
            physics_processing_time: 0.0,
            simulation_start: Instant::now(),
            synthesizer: Synthesizer::new(),
            delay_filters: Vec::new(),
            crank_constraints: Vec::new(),
            cylinder_wall_constraints: Vec::new(),
            link_constraints: Vec::new(),
            crankshaft_friction_constraints: Vec::new(),
            dyno_constraint: usize::MAX,
            starter_constraint: usize::MAX,
            vehicle_mass_body: usize::MAX,
            fluid_simulation_steps: 8,
            exhaust_flow_staging_buffer: Vec::new(),
            rng: Rng::default(),
        };

        sim.load_simulation();

        sim.simulation_frequency = sim.engine.initial_simulation_frequency;

        sim
    }

    // ------------------------------------------------------------- assembly

    /// `PistonEngineSimulator::loadSimulation`
    fn load_simulation(&mut self) {
        let crank_count = self.engine.get_crankshaft_count();
        let cylinder_count = self.engine.get_cylinder_count();

        if crank_count == 0 {
            return;
        }

        let ks = 5000.0;
        let kd = 10.0;

        // Rigid bodies: crankshafts first, exactly as the original adds them.
        for i in 0..crank_count {
            let shaft = self.engine.crankshafts[i].clone();

            let mut body = RigidBody {
                p_x: shaft.pos_x,
                p_y: shaft.pos_y,
                theta: 0.0,
                m: shaft.mass + shaft.flywheel_mass,
                i: shaft.moment_of_inertia,
                ..Default::default()
            };
            body.index = i as i32;

            let index = self.solver.add_rigid_body(body);
            shaft.body.set(index);
        }

        // Vehicle rotating mass.
        let vehicle_mass_body = self.solver.add_rigid_body(RigidBody {
            m: 1.0,
            i: 1.0,
            ..Default::default()
        });
        self.vehicle_mass_body = vehicle_mass_body;

        // Per-cylinder bodies.
        for i in 0..cylinder_count {
            let piston = &self.engine.pistons[i];
            let rod = piston.rod.clone();

            let piston_body = self.solver.add_rigid_body(RigidBody {
                m: piston.mass,
                i: 1.0,
                ..Default::default()
            });
            self.engine.pistons[i].body = piston_body;

            let rod_body = self.solver.add_rigid_body(RigidBody {
                m: rod.mass,
                i: rod.moment_of_inertia,
                ..Default::default()
            });
            rod.body.set(rod_body);

            self.engine.rods[i].body.set(rod_body);
        }

        // Chambers need the body indices assigned above.
        self.engine.build_chambers();

        // ------------------------------------------------------- constraints
        for i in 0..crank_count {
            let shaft = self.engine.crankshafts[i].clone();
            let output_shaft = self.engine.crankshafts[0].clone();

            let mut crank_constraint = FixedPositionConstraint::default();
            crank_constraint.set_body(shaft.body.get() as i32);
            crank_constraint.set_world_position(shaft.pos_x, shaft.pos_y);
            crank_constraint.set_local_position(0.0, 0.0);
            crank_constraint.kd = kd;
            crank_constraint.ks = ks;
            let idx = self.add_constraint(crank_constraint);
            self.crank_constraints.push(idx);

            let mut friction = RotationFrictionConstraint::default();
            friction.min_torque = -shaft.friction_torque;
            friction.max_torque = shaft.friction_torque;
            friction.set_body(shaft.body.get() as i32);
            let idx = self.add_constraint(friction);
            self.crankshaft_friction_constraints.push(idx);

            if i != 0 {
                let mut link = ClutchConstraint::new();
                link.set_body1(output_shaft.body.get() as i32);
                link.set_body2(shaft.body.get() as i32);
                self.add_constraint(link);
            }
        }

        // Transmission + vehicle
        self.transmission.lock().unwrap().add_to_system(
            self.engine.crankshafts[0].body.get(),
            vehicle_mass_body,
            self.vehicle.clone(),
        );
        let clutch = TransmissionClutch {
            transmission: self.transmission.clone(),
            forces: Default::default(),
        };
        self.add_constraint(clutch);

        self.vehicle.add_to_system(vehicle_mass_body);
        self.add_constraint(VehicleDragConstraint::new(
            vehicle_mass_body,
            self.vehicle.clone(),
        ));

        // Per-cylinder constraints
        for i in 0..cylinder_count {
            let (piston_body, wrist_pin_location, rod, bank) = {
                let piston = &self.engine.pistons[i];
                (
                    piston.body,
                    piston.wrist_pin_location,
                    piston.rod.clone(),
                    piston.bank.clone(),
                )
            };

            let dx = (bank.angle() + constants::PI / 2.0).cos();
            let dy = (bank.angle() + constants::PI / 2.0).sin();


            let mut little_end = LinkConstraint::default();
            little_end.set_body1(rod.body.get() as i32);
            little_end.set_body2(piston_body as i32);
            little_end.set_local_position1(0.0, rod.little_end_local());
            little_end.set_local_position2(0.0, wrist_pin_location);
            little_end.ks = ks;
            little_end.kd = kd;
            let idx = self.add_constraint(little_end);
            self.link_constraints.push(idx);

            let (journal_x, journal_y, big_end_body) = match &rod.master {
                None => {
                    let shaft = rod.crankshaft.clone();
                    let (jx, jy) = shaft.rod_journal_position_local(rod.journal as usize);
                    (jx, jy, shaft.body.get())
                }
                Some(master) => {
                    let (jx, jy) = master
                        .rod_journal_position_local(rod.journal as usize);
                    (jx, jy, master.body.get())
                }
            };

            let mut big_end = LinkConstraint::default();
            big_end.set_body1(rod.body.get() as i32);
            big_end.set_body2(big_end_body as i32);
            big_end.set_local_position1(0.0, rod.big_end_local());
            big_end.set_local_position2(journal_x, journal_y);
            big_end.ks = ks;
            big_end.kd = kd;
            let idx = self.add_constraint(big_end);
            self.link_constraints.push(idx);
            let mut wall = LineConstraint::default();
            wall.set_body(piston_body as i32);
            wall.dx = dx;
            wall.dy = dy;
            wall.local_x = 0.0;
            wall.local_y = wrist_pin_location;
            wall.p0_x = bank.x();
            wall.p0_y = bank.y();
            wall.ks = ks;
            wall.kd = kd;
            let idx = self.add_constraint(wall);
            self.cylinder_wall_constraints.push(idx);
        }

        // Dynamometer + starter motor
        let mut dyno = Dynamometer::default();
        dyno.connect_crankshaft(self.engine.crankshafts[0].body.get());
        self.dyno_constraint = self.add_constraint(dyno);

        let mut starter = StarterMotor::default();
        starter.connect_crankshaft(self.engine.crankshafts[0].body.get());
        starter.max_torque = self.engine.starter_torque;
        starter.rotation_speed = -self.engine.starter_speed;
        self.starter_constraint = self.add_constraint(starter);

        self.place_and_initialize();
    }

    fn add_constraint<C: Constraint + 'static>(&mut self, constraint: C) -> usize {
        let index = self.solver.constraints.len();
        self.solver.add_constraint(Box::new(constraint));
        index
    }

    /// `PistonEngineSimulator::placeAndInitialize`
    fn place_and_initialize(&mut self) {
        let cylinder_count = self.engine.get_cylinder_count();

        for _pass in 0..2 {
            for i in 0..cylinder_count {
                if _pass == 0 {
                    let rod = &self.engine.rods[i];
                    if rod.rod_journal_count() != 0 {
                        self.place_cylinder(i);
                    }
                } else {
                    self.place_cylinder(i);
                }
            }
        }

        for i in 0..cylinder_count {
            let volume = self.engine.chambers[i].get_volume(&self.solver.bodies);
            self.engine.chambers[i].system.initialize(
                units::pressure(1.0, units::ATM),
                volume,
                units::celcius(25.0),
                crate::gas::Mix::DEFAULT,
                5,
            );
        }

        let cycle_angle = self.engine.crankshafts[0].get_cycle_angle(0.0, &self.solver.bodies);
        self.engine.ignition.reset(cycle_angle);

        self.exhaust_flow_staging_buffer = vec![0.0; self.engine.get_exhaust_system_count()];

        // `PistonEngineSimulator::placeAndInitialize`: one delay filter per
        // cylinder, from the header-primary + exhaust pipe length.
        self.delay_filters = Vec::with_capacity(cylinder_count);
        for i in 0..cylinder_count {
            let piston = &self.engine.pistons[i];
            let bank = piston.bank.clone();
            let head = self.engine.heads[bank.index().max(0) as usize].clone();
            let cylinder_index = piston.cylinder_index.max(0) as usize;

            let exhaust_index = head.exhaust_system(cylinder_index).expect("exhaust system");
            let exhaust_length =
                head.header_primary_length(cylinder_index) + self.engine.exhausts[exhaust_index].length;

            let delay = exhaust_length / crate::synth::SPEED_OF_SOUND;
            let mut filter = DelayFilter::new();
            filter.initialize(delay, 10000.0);
            self.delay_filters.push(filter);
        }

        self.initialize_synthesizer();
    }

    /// `Simulator::initializeSynthesizer`
    pub fn initialize_synthesizer(&mut self) {
        let params = SynthesizerParameters {
            input_channel_count: self.engine.get_exhaust_system_count(),
            input_buffer_size: 44100,
            audio_buffer_size: 44100,
            input_sample_rate: self.simulation_frequency as f32,
            audio_sample_rate: 44100.0,
            initial_audio_parameters: AudioParameters::default(),
        };

        self.synthesizer.initialize(&params);
    }

    /// `PistonEngineSimulator::placeCylinder`
    fn place_cylinder(&mut self, i: usize) {
        let rod = self.engine.rods[i].clone();
        let bank = self.engine.pistons[i].bank.clone();

        let (p_x, p_y) = match &rod.master {
            None => rod
                .crankshaft
                .rod_journal_position_global(rod.journal as usize, &self.solver.bodies),
            Some(master) => {
                master.rod_journal_position_global(rod.journal as usize, &self.solver.bodies)
            }
        };

        let a = bank.dx() * bank.dx() + bank.dy() * bank.dy();
        let b = -2.0 * bank.dx() * (p_x - bank.x()) - 2.0 * bank.dy() * (p_y - bank.y());
        let c = (p_x - bank.x()) * (p_x - bank.x()) + (p_y - bank.y()) * (p_y - bank.y())
            - rod.length * rod.length;

        let det = b * b - 4.0 * a * c;
        if det < 0.0 {
            return;
        }

        let sqrt_det = det.sqrt();
        let s0 = (-b + sqrt_det) / (2.0 * a);
        let s1 = (-b - sqrt_det) / (2.0 * a);

        let s = s0.max(s1);
        if s < 0.0 {
            return;
        }

        let e_x = s * bank.dx() + bank.x();
        let e_y = s * bank.dy() + bank.y();

        let theta = if (e_y - p_y) > 0.0 {
            ((e_x - p_x) / rod.length).acos()
        } else {
            2.0 * constants::PI - ((e_x - p_x) / rod.length).acos()
        };

        let rod_body = rod.body.get();
        self.solver.bodies[rod_body].theta = theta - constants::PI / 2.0;

        let (cl_x, cl_y) = {
            let body = &self.solver.bodies[rod_body];
            body.local_to_world(0.0, rod.big_end_local())
        };
        self.solver.bodies[rod_body].p_x += p_x - cl_x;
        self.solver.bodies[rod_body].p_y += p_y - cl_y;

        let piston_body = self.engine.pistons[i].body;
        self.solver.bodies[piston_body].p_x = e_x;
        self.solver.bodies[piston_body].p_y = e_y;
        self.solver.bodies[piston_body].theta = bank.angle() + constants::PI;
    }

    // ----------------------------------------------------------- simulation

    pub fn get_timestep(&self) -> f64 {
        1.0 / self.simulation_frequency
    }

    pub fn simulation_steps(&self) -> usize {
        self.steps
    }

    /// `Simulator::startFrame`
    pub fn start_frame(&mut self, dt: f64) {
        self.simulation_start = Instant::now();
        self.current_iteration = 0;

        self.synthesizer
            .set_input_sample_rate(self.simulation_frequency * self.simulation_speed);

        let timestep = self.get_timestep();
        self.steps = ((dt * self.simulation_speed) / timestep).round() as usize;

        let target_latency = self.target_synthesizer_latency;
        let synthesizer_latency = self.synthesizer.get_latency();
        if synthesizer_latency < target_latency {
            self.steps = ((self.steps + 1) as f64 * 1.1) as usize;
        } else if synthesizer_latency > target_latency {
            let adjusted = ((self.steps as f64 - 1.0) * 0.9) as i64;
            self.steps = if adjusted < 0 { 0 } else { adjusted as usize };
        }

        if self.steps > 0 {
            for intake in self.engine.intakes.iter_mut() {
                intake.flow_rate = 0.0;
            }
        }
    }

    /// Runs exactly `steps` simulation steps of the current timestep. This is
    /// the headless equivalent of `startFrame` + `simulateStep` × n + `endFrame`.
    pub fn step(&mut self, steps: usize) {
        self.current_iteration = 0;
        self.steps = steps;

        for intake in self.engine.intakes.iter_mut() {
            intake.flow_rate = 0.0;
        }

        while self.simulate_step() {}

        self.end_frame();
    }

    /// `Simulator::simulateStep`
    pub fn simulate_step(&mut self) -> bool {
        if self.current_iteration >= self.steps {
            let last_frame = self.simulation_start.elapsed().as_micros() as f64;
            self.physics_processing_time = self.physics_processing_time * 0.98 + 0.02 * last_frame;

            return false;
        }

        let timestep = self.get_timestep();

        self.step_solver(timestep);

        // Exchange the per-cylinder wall forces, which the original reads
        // directly through the piston's constraint pointer.
        self.update_cylinder_wall_forces();

        {
            let bodies = &self.solver.bodies;
            self.engine.update(timestep, bodies);
            self.vehicle.update(timestep, bodies);
        }

        self.transmission.lock().unwrap().update(timestep);

        // `Simulator::updateFilteredEngineSpeed`. Note the reference weights the
        // *new* value by `1 - alpha` (alpha ~= 1e-6 at 10 kHz), so this is
        // effectively an instantaneous read — the DSP's speed attenuation
        // depends on it, so it is transliterated rather than "fixed".
        {
            let alpha = timestep / (100.0 + timestep);
            let rpm = self.engine.get_rpm(&self.solver.bodies);
            self.filtered_engine_speed =
                alpha * self.filtered_engine_speed + (1.0 - alpha) * rpm;
        }

        // `resetAngle` + drift correction
        let output_theta = {
            let mut body = self.solver.bodies[self.engine.crankshafts[0].body.get()].theta;
            body %= 4.0 * constants::PI;
            body
        };

        for shaft in self.engine.crankshafts.iter() {
            self.solver.bodies[shaft.body.get()].theta = output_theta;
        }

        let cycle_angle = self.engine.crankshafts[0].get_cycle_angle(0.0, &self.solver.bodies);
        let index = (DYNO_TORQUE_SAMPLES as f64 * cycle_angle / (4.0 * constants::PI)).floor() as i32;
        let index = index.clamp(0, DYNO_TORQUE_SAMPLES as i32 - 1);
        let step_dir = if self.engine.is_spinning_cw(&self.solver.bodies) {
            1
        } else {
            -1
        };

        let torque = self.dyno_torque();
        self.dyno_torque_samples[index as usize] = torque;

        if self.last_dyno_torque_sample != index {
            // Mirrors the C++ `for` loop, whose `continue` still applies the
            // increment: the wrap-around assignments are followed by `+= step`.
            let mut i = self.last_dyno_torque_sample + step_dir;
            while i != index {
                if i >= DYNO_TORQUE_SAMPLES as i32 {
                    i = -1 + step_dir;
                    continue;
                } else if i < 0 {
                    i = DYNO_TORQUE_SAMPLES as i32 + step_dir;
                    continue;
                }

                self.dyno_torque_samples[i as usize] = torque;
                i += step_dir;
            }

            self.last_dyno_torque_sample = index;
        }

        self.simulate_step_internal();

        self.write_to_synthesizer();

        self.current_iteration += 1;
        true
    }

    fn step_solver(&mut self, timestep: f64) {
        let engine = &mut self.engine;
        self.solver.process(timestep, 1, &mut |state| {
            for chamber in engine.chambers.iter_mut() {
                chamber.apply(state);
            }
        });
    }

    fn update_cylinder_wall_forces(&mut self) {
        for i in 0..self.cylinder_wall_constraints.len() {
            let constraint = self.cylinder_wall_constraints[i];
            let forces = *self.solver.constraints[constraint].forces();
            let f_x = forces.f_x[0][0];
            let f_y = forces.f_y[0][0];
            let magnitude = (f_x * f_x + f_y * f_y).sqrt();

            self.engine.pistons[i].cylinder_wall_force = magnitude;
            self.engine.chambers[i].cylinder_wall_force = magnitude;
        }
    }

    /// `PistonEngineSimulator::simulateStep_`
    fn simulate_step_internal(&mut self) {
        let timestep = self.get_timestep();
        let cylinder_count = self.engine.get_cylinder_count();

        let cycle_angle = self.engine.crankshafts[0].get_cycle_angle(0.0, &self.solver.bodies);
        let v_theta = self.solver.bodies[self.engine.crankshafts[0].body.get()].v_theta;
        self.engine.ignition.update(timestep, cycle_angle, v_theta);

        let crank_angle = self.engine.crankshafts[0].get_angle(&self.solver.bodies);
        let vtec = self.engine.vtec_conditions(&self.solver.bodies);

        for i in 0..cylinder_count {
            let ignited = self.engine.ignition.get_ignition_event(i);
            let bodies = &self.solver.bodies;
            if ignited {
                self.engine.chambers[i].ignite(bodies, &mut self.rng);
            }
        }

        for i in 0..cylinder_count {
            let bodies = &self.solver.bodies;
            let bank_index = self.engine.pistons[i].bank.index().max(0) as usize;
            let vtec_enabled = self.engine.heads[bank_index]
                .valvetrain
                .is_vtec_enabled(&vtec);
            self.engine.chambers[i].update(bodies, crank_angle, cycle_angle, vtec_enabled);
        }

        for i in 0..cylinder_count {
            self.engine.chambers[i].reset_last_timestep_exhaust_flow();
            self.engine.chambers[i].reset_last_timestep_intake_flow();
        }

        let fluid_timestep = timestep / self.fluid_simulation_steps as f64;

        for _ in 0..self.fluid_simulation_steps {
            for exhaust in self.engine.exhausts.iter_mut() {
                exhaust.process(fluid_timestep);
            }

            for intake in self.engine.intakes.iter_mut() {
                intake.process(fluid_timestep);
                intake.flow_rate += intake.flow;
            }

            for j in 0..cylinder_count {
                self.engine.chambers[j].flow(
                    fluid_timestep,
                    &self.solver.bodies,
                    &mut self.engine.intakes,
                    &mut self.engine.exhausts,
                );
            }
        }

        self.engine.ignition.reset_ignition_events();
    }

    /// `PistonEngineSimulator::writeToSynthesizer` — collects the per-exhaust
    /// flow signal that the (not yet ported) synthesizer consumes.
    pub fn write_to_synthesizer(&mut self) {
        for value in self.exhaust_flow_staging_buffer.iter_mut() {
            *value = 0.0;
        }

        let filtered_speed = self.filtered_engine_speed;
        let attenuation = filtered_speed.abs().min(40.0) / 40.0;
        let attenuation_3 = attenuation * attenuation * attenuation;

        let cylinder_count = self.engine.get_cylinder_count();

        for i in 0..cylinder_count {
            // Everything here reads the engine and only the delay filter writes,
            // so the reads are taken in one immutable borrow and the filter
            // advanced afterwards. Cloning the head instead -- four `Arc`s and a
            // `Vec` per cylinder, per simulation step -- is what a borrow checker
            // workaround costs: on the nine-cylinder radial it was most of the
            // frame.
            let (exhaust_index, exhaust_length, audio_volume, attenuation_sound, exhaust_flow) = {
                let piston = &self.engine.pistons[i];
                let head = &self.engine.heads[piston.bank.index().max(0) as usize];
                let cylinder_index = piston.cylinder_index.max(0) as usize;
                let exhaust_index = head.exhaust_system(cylinder_index).expect("exhaust");
                let chamber = &self.engine.chambers[i];

                let exhaust_length = head.header_primary_length(cylinder_index)
                    + self.engine.exhausts[exhaust_index].length;

                let exhaust_flow = attenuation_3
                    * 1600.0
                    * (1.0 * (chamber.exhaust_runner_and_primary.pressure()
                        - units::pressure(1.0, units::ATM))
                        + 0.1 * chamber.exhaust_runner_and_primary.dynamic_pressure(1.0, 0.0)
                        + 0.1 * chamber.exhaust_runner_and_primary.dynamic_pressure(-1.0, 0.0));

                (
                    exhaust_index,
                    exhaust_length,
                    self.engine.exhausts[exhaust_index].audio_volume,
                    head.sound_attenuation(cylinder_index),
                    exhaust_flow,
                )
            };

            let delayed_exhaust_pulse = self.delay_filters[i].fast_f(exhaust_flow);

            let index = self.engine.exhausts[exhaust_index].index.max(0) as usize;

            self.exhaust_flow_staging_buffer[index] += attenuation_sound
                * (audio_volume * delayed_exhaust_pulse / cylinder_count as f64)
                * (1.0 / (exhaust_length * exhaust_length));
        }

        self.synthesizer.write_input(&self.exhaust_flow_staging_buffer);
    }

    // ------------------------------------------------------------- audio out

    /// `Synthesizer::renderAudio()` (one block) — the reference runs this on a
    /// dedicated audio thread; here the caller drives it, which is what makes
    /// the output reproducible.
    pub fn render_audio_block(&mut self) -> usize {
        // Nothing to render until the application installs the impulse response
        // recording (the reference reads an uninitialised buffer here).
        if !self.synthesizer.has_impulse_response() {
            return 0;
        }

        // The synthesizer and the physics share one `rand()` sequence in the
        // original, so they must share one `Rng` here too.
        let rng = &mut self.rng;
        self.synthesizer.render_block(rng)
    }

    /// `Simulator::readAudioOutput`
    pub fn read_audio_output(&mut self, buffer: &mut [i16]) -> usize {
        self.synthesizer.read_audio_output(buffer)
    }

    /// `Simulator::getSynthesizerInputLatency`
    pub fn synthesizer_latency(&self) -> f64 {
        self.synthesizer.get_latency()
    }

    /// `Simulator::getSynthesizerInputLatencyTarget`
    pub fn target_synthesizer_latency(&self) -> f64 {
        self.target_synthesizer_latency
    }

    /// `Simulator::endFrame`
    pub fn end_frame(&mut self) {
        if self.engine.get_crankshaft_count() == 0 {
            return;
        }

        self.synthesizer.end_input_block();

        let frame_timestep = self.steps as f64 * self.get_timestep();
        if frame_timestep == 0.0 {
            return;
        }

        for intake in self.engine.intakes.iter_mut() {
            intake.flow_rate /= frame_timestep;
        }
    }

    // ------------------------------------------------------------- telemetry

    /// `Dynamometer::getTorque` for the attached dynamometer.
    pub fn dyno_torque(&self) -> f64 {
        self.solver.constraints[self.dyno_constraint].output_torque(&self.solver.bodies)
    }

    /// `Simulator::getFilteredDynoTorque`
    pub fn get_filtered_dyno_torque(&self) -> f64 {
        let mut average_torque = 0.0;
        for i in 0..DYNO_TORQUE_SAMPLES {
            average_torque += self.dyno_torque_samples[i];
        }

        average_torque / DYNO_TORQUE_SAMPLES as f64
    }

    /// `Simulator::getDynoPower`
    pub fn get_dyno_power(&self) -> f64 {
        self.get_filtered_dyno_torque() * self.engine.get_speed(&self.solver.bodies)
    }

    /// `PistonEngineSimulator::getTotalExhaustFlow`
    pub fn get_total_exhaust_flow(&self) -> f64 {
        let mut total_flow = 0.0;
        for chamber in self.engine.chambers.iter() {
            total_flow += chamber.last_timestep_exhaust_flow();
        }

        total_flow
    }

    /// `PistonEngineSimulator::getAverageOutputSignal`
    pub fn get_average_output_signal(&self) -> f64 {
        let mut sum = 0.0;
        for exhaust in self.engine.exhausts.iter() {
            sum += exhaust.system.pressure();
        }

        sum / self.engine.exhausts.len() as f64
    }

    pub fn get_fluid_simulation_frequency(&self) -> f64 {
        self.fluid_simulation_steps as f64 * self.simulation_frequency
    }

    // ------------------------------------------------------------ dyno setup

    pub fn set_dyno_enabled(&mut self, enabled: bool) {
        if let Some(dyno) = self
            .solver
            .constraints[self.dyno_constraint]
            .as_any_mut()
            .downcast_mut::<Dynamometer>()
        {
            dyno.enabled = enabled;
        }
    }

    pub fn set_dyno_hold(&mut self, hold: bool) {
        if let Some(dyno) = self
            .solver
            .constraints[self.dyno_constraint]
            .as_any_mut()
            .downcast_mut::<Dynamometer>()
        {
            dyno.hold = hold;
        }
    }

    /// `EngineSimApplication::processEngineInput`'s `setClutchPressure`: the
    /// transmission's clutch is a torque limit scaled by this, and it starts at
    /// zero -- the application ramps it to one within a millisecond unless the
    /// clutch key is held.
    pub fn set_clutch_pressure(&mut self, pressure: f64) {
        self.transmission.lock().unwrap().clutch_pressure = pressure;
    }

    pub fn set_dyno_rotation_speed(&mut self, speed: f64) {
        if let Some(dyno) = self
            .solver
            .constraints[self.dyno_constraint]
            .as_any_mut()
            .downcast_mut::<Dynamometer>()
        {
            dyno.rotation_speed = speed;
        }
    }

    pub fn set_starter_enabled(&mut self, enabled: bool) {
        if let Some(starter) = self
            .solver
            .constraints[self.starter_constraint]
            .as_any_mut()
            .downcast_mut::<StarterMotor>()
        {
            starter.enabled = enabled;
        }
    }

    pub fn change_gear(&mut self, gear: i32) {
        self.transmission
            .lock()
            .unwrap()
            .change_gear(gear, &mut self.solver.bodies);
    }
}

/// The transmission's clutch, as a constraint. The transmission itself stays
/// owned by the simulator (behind its mutex) so `update`/`change_gear` can be
/// called between steps.
struct TransmissionClutch {
    transmission: Arc<Mutex<Transmission>>,
    forces: crate::scs::ConstraintForces,
}

impl Constraint for TransmissionClutch {
    fn constraint_count(&self) -> usize {
        1
    }

    fn bodies(&self) -> [i32; crate::scs::MAX_BODY_COUNT] {
        let t = self.transmission.lock().unwrap();
        [t.clutch_body1, t.clutch_body2]
    }

    fn calculate(
        &mut self,
        output: &mut crate::scs::Output,
        state: &crate::scs::SystemState,
        bodies: &[RigidBody],
    ) {
        let mut t = self.transmission.lock().unwrap();
        Constraint::calculate(&mut *t, output, state, bodies);
    }

    fn forces(&self) -> &crate::scs::ConstraintForces {
        unreachable!("forces are only read through `forces_mut` on this proxy")
    }

    fn forces_mut(&mut self) -> &mut crate::scs::ConstraintForces {
        // The forces live inside the transmission; hand out a stable pointer by
        // leaking into the constraint's own storage is not possible, so the
        // proxy keeps its own copy that is refreshed by `calculate`.
        &mut self.forces
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
