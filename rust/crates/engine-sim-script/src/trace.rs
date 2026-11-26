//! The `RUN` gate's writer: one line per field per frame, the same fields in the
//! same order as `rust/tools/parity/core_trace_common.h`'s `traceFrames`.
//!
//! This is the port's own engine stepping -- not a recording. `core_trace.rs`
//! prints the same shape for the hand-built TRX520; the layout is shared because
//! the C++ side shares it, and a field added to one without the other shows up
//! as a diff.
//!
//! Format: `<step>,<field>,<16 hex digits of the f64 bit pattern>`.

use engine_sim_core::simulator::Simulator;
use engine_sim_core::units;

fn emit(out: &mut String, step: usize, name: &str, value: f64) {
    out.push_str(&format!("{},{},{:016x}\n", step, name, value.to_bits()));
}

/// How a run is driven, the port's half of `core_trace_common.h`'s
/// `TraceOptions`. The defaults are what every gate uses: the script's own
/// throttle, the starter held (as the application holds `S`), the ignition as the
/// script left it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drive {
    /// Throttle plate position, applied through the engine's speed control the
    /// way the application applies its keys (`position = 1 - speed_control^gamma`).
    pub throttle: Option<f64>,

    /// Engine time after which the starter is released. Without it the starter's
    /// velocity constraint holds the crankshaft at the starter's speed forever.
    pub starter_seconds: Option<f64>,

    /// Whether the ignition fires. The module is built disabled, and the
    /// application only enables it when `A` is pressed, so a run that wants a
    /// running engine has to ask.
    pub ignition: Option<bool>,

    /// Dyno hold speed in rpm. A held dyno takes the place of the starter.
    pub dyno_rpm: Option<f64>,
}

impl Drive {
    /// Applies this frame's part of the drive, before the frame is stepped.
    fn apply(&self, sim: &mut Simulator, time: f64) {
        if let Some(throttle) = self.throttle {
            // Only the speed control: the engine's linkage turns it into a plate
            // position (and the reference harness does exactly this much).
            sim.engine.set_speed_control(1.0 - throttle);
        }

        if let Some(seconds) = self.starter_seconds {
            if time >= seconds {
                sim.set_starter_enabled(false);
            }
        }

        if let Some(rpm) = self.dyno_rpm {
            sim.set_starter_enabled(false);
            sim.set_dyno_enabled(true);
            sim.set_dyno_hold(true);
            sim.set_dyno_rotation_speed(units::rpm(rpm));
        }
    }
}

/// Runs `steps` application frames, emitting the physics, the synthesizer input
/// and every rendered audio sample.
pub fn frames(sim: &mut Simulator, steps: usize) -> String {
    frames_driven(sim, steps, &Drive::default())
}

/// `frames`, driven: see [`Drive`].
pub fn frames_driven(sim: &mut Simulator, steps: usize, drive: &Drive) -> String {
    let mut out = String::new();
    let timestep = sim.get_timestep();

    for step in 0..steps {
        if let Some(ignition) = drive.ignition {
            sim.engine.ignition.enabled = ignition;
        }

        drive.apply(sim, step as f64 * timestep);

        // `startFrame` with a sub-half-timestep `dt` yields exactly one
        // simulation step per frame, matching the reference harness.
        sim.start_frame(timestep * 0.1);
        while sim.simulate_step() {}
        sim.end_frame();

        {
            let n = (2000i64 - sim.synthesizer.audio_buffer.size() as i64).max(0) as usize;
            let n = n.min(sim.synthesizer.input_channels[0].data.size());
            let block = sim.synthesizer.input_channels[0].data.read_block(n);

            for (i, &value) in block.iter().enumerate() {
                emit(&mut out, step, &format!("in{}", i), value as f64);
            }
        }

        sim.render_audio_block();

        let mut audio = [0i16; 4096];
        let produced = sim.read_audio_output(&mut audio);
        for (i, &sample) in audio[..produced].iter().enumerate() {
            emit(&mut out, step, &format!("audio{}", i), sample as f64);
        }

        let crank = &sim.solver.bodies[sim.engine.crankshafts[0].body.get()];
        let piston = &sim.solver.bodies[sim.engine.pistons[0].body];
        let rod = &sim.solver.bodies[sim.engine.rods[0].body.get()];
        let chamber = &sim.engine.chambers[0];

        emit(&mut out, step, "crank_theta", crank.theta);
        emit(&mut out, step, "crank_v_theta", crank.v_theta);
        emit(&mut out, step, "piston_p_x", piston.p_x);
        emit(&mut out, step, "piston_p_y", piston.p_y);
        emit(&mut out, step, "piston_v_x", piston.v_x);
        emit(&mut out, step, "piston_v_y", piston.v_y);
        emit(&mut out, step, "rod_theta", rod.theta);
        emit(&mut out, step, "rod_p_x", rod.p_x);
        emit(&mut out, step, "rod_p_y", rod.p_y);
        emit(&mut out, step, "chamber_pressure", chamber.system.pressure());
        emit(
            &mut out,
            step,
            "chamber_temperature",
            chamber.system.temperature(),
        );
        emit(&mut out, step, "chamber_n", chamber.system.n());
        emit(
            &mut out,
            step,
            "chamber_mix_fuel",
            chamber.system.mix().p_fuel,
        );
        emit(&mut out, step, "chamber_mix_o2", chamber.system.mix().p_o2);
        emit(
            &mut out,
            step,
            "intake_pressure",
            sim.engine.intakes[0].system.pressure(),
        );
        emit(
            &mut out,
            step,
            "runner_pressure",
            chamber.intake_runner_and_manifold.pressure(),
        );
        emit(
            &mut out,
            step,
            "exhaust_pressure",
            sim.engine.exhausts[0].system.pressure(),
        );
        emit(
            &mut out,
            step,
            "crank_cycle_angle",
            sim.engine.crankshafts[0].get_cycle_angle(0.0, &sim.solver.bodies),
        );
        emit(
            &mut out,
            step,
            "ignition_advance",
            sim.engine.ignition.get_timing_advance(
                sim.solver.bodies[sim.engine.crankshafts[0].body.get()].v_theta,
            ),
        );
        emit(
            &mut out,
            step,
            "chamber_mix_inert",
            chamber.system.mix().p_inert,
        );
        emit(&mut out, step, "chamber_volume", chamber.get_volume(&sim.solver.bodies));
        emit(&mut out, step, "chamber_lit", if chamber.lit { 1.0 } else { 0.0 });
        emit(&mut out, step, "chamber_afr", chamber.last_event_afr());
        emit(
            &mut out,
            step,
            "chamber_exhaust_flow",
            chamber.last_iteration_exhaust_flow(),
        );
        {
            let attenuation = sim.filtered_engine_speed.abs().min(40.0) / 40.0;
            emit(&mut out, step, "filtered_speed", sim.filtered_engine_speed);
            emit(&mut out, step, "attenuation_3", attenuation * attenuation * attenuation);
            emit(
                &mut out,
                step,
                "exhaust_dp",
                chamber.exhaust_runner_and_primary.pressure() - units::pressure(1.0, units::ATM),
            );
            emit(
                &mut out,
                step,
                "exhaust_dyn_p",
                chamber.exhaust_runner_and_primary.dynamic_pressure(1.0, 0.0),
            );
            emit(
                &mut out,
                step,
                "exhaust_dyn_n",
                chamber.exhaust_runner_and_primary.dynamic_pressure(-1.0, 0.0),
            );
        }
        emit(&mut out, step, "exhaust_signal", sim.exhaust_flow_staging_buffer[0]);
        emit(&mut out, step, "synth_latency", sim.synthesizer_latency());
        emit(
            &mut out,
            step,
            "synth_channels",
            sim.synthesizer.input_channel_count as f64,
        );
        emit(
            &mut out,
            step,
            "synth_input_size",
            sim.synthesizer.input_channels[0].data.size() as f64,
        );
        emit(
            &mut out,
            step,
            "synth_write_offset",
            sim.synthesizer.input_write_offset,
        );
        emit(&mut out, step, "frame_steps", sim.steps as f64);
        emit(&mut out, step, "dyno_torque", sim.dyno_torque());
    }

    out
}
