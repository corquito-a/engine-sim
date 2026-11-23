//! Rust half of the whole-core parity check.
//!
//! Mirrors `rust/tools/parity/core_reference.cpp` step for step and prints
//! `<step>,<field>,<16 hex digits>` so the two outputs can be diffed
//! bit-for-bit (see `rust/tools/parity/run_core_parity.sh`).

include!("../fixtures/trx520.rs");

fn emit(step: usize, name: &str, value: f64) {
    println!("{step},{name},{:016x}", value.to_bits());
}

/// Deterministic impulse response for the exhaust convolution, generated with
/// integer arithmetic only so both implementations build the identical table.
fn build_impulse_response(samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let value = ((i as i32 * 7919) % 4001) - 2000;
            let envelope = 255 - (i as i32 % 256);
            (value * envelope / 256) as i16
        })
        .collect()
}

fn main() {
    let step_count: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    let mut sim = build_simulator();
    sim.set_starter_enabled(true);

    // Parameter switches used to bisect DSP mismatches.
    {
        let mut ap = sim.synthesizer.audio_parameters();
        if std::env::var_os("PARITY_NO_JITTER").is_some() {
            ap.input_sample_noise = 0.0;
        }
        if std::env::var_os("PARITY_NO_AIR_NOISE").is_some() {
            ap.air_noise = 0.0;
        }
        if std::env::var_os("PARITY_NO_CONV").is_some() {
            ap.convolution = 0.0;
        }
        sim.synthesizer.set_audio_parameters(ap);
    }

    // Deterministic exhaust impulse response for the convolution stage.
    {
        let response = build_impulse_response(512);
        for i in 0..sim.engine.get_exhaust_system_count() {
            sim.synthesizer
                .initialize_impulse_response(&response, 1.0, i);
        }
    }

    // Optional dynamometer coverage: `PARITY_DYNO=1` holds the crank at a
    // fixed speed instead of free-revving it.
    if std::env::var_os("PARITY_DYNO").is_some() {
        sim.set_dyno_enabled(true);
        sim.set_dyno_hold(true);
        sim.set_dyno_rotation_speed(-units::rpm(1200.0));
    }

    let timestep = sim.get_timestep();

    for step in 0..step_count {
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
                emit(step, &format!("in{i}"), value as f64);
            }
        }

        sim.render_audio_block();

        let mut audio = [0i16; 4096];
        let produced = sim.read_audio_output(&mut audio);
        for (i, &sample) in audio[..produced].iter().enumerate() {
            emit(step, &format!("audio{i}"), sample as f64);
        }

        let crank = &sim.solver.bodies[sim.engine.crankshafts[0].body.get()];
        let piston = &sim.solver.bodies[sim.engine.pistons[0].body];
        let rod = &sim.solver.bodies[sim.engine.rods[0].body.get()];
        let chamber = &sim.engine.chambers[0];

        emit(step, "crank_theta", crank.theta);
        emit(step, "crank_v_theta", crank.v_theta);
        emit(step, "piston_p_x", piston.p_x);
        emit(step, "piston_p_y", piston.p_y);
        emit(step, "piston_v_x", piston.v_x);
        emit(step, "piston_v_y", piston.v_y);
        emit(step, "rod_theta", rod.theta);
        emit(step, "rod_p_x", rod.p_x);
        emit(step, "rod_p_y", rod.p_y);
        emit(step, "chamber_pressure", chamber.system.pressure());
        emit(step, "chamber_temperature", chamber.system.temperature());
        emit(step, "chamber_n", chamber.system.n());
        emit(step, "chamber_mix_fuel", chamber.system.mix().p_fuel);
        emit(step, "chamber_mix_o2", chamber.system.mix().p_o2);
        emit(step, "intake_pressure", sim.engine.intakes[0].system.pressure());
        emit(
            step,
            "runner_pressure",
            chamber.intake_runner_and_manifold.pressure(),
        );
        emit(step, "exhaust_pressure", sim.engine.exhausts[0].system.pressure());
        emit(
            step,
            "crank_cycle_angle",
            sim.engine.crankshafts[0].get_cycle_angle(0.0, &sim.solver.bodies),
        );
        emit(
            step,
            "ignition_advance",
            sim.engine
                .ignition
                .get_timing_advance(sim.solver.bodies[sim.engine.crankshafts[0].body.get()].v_theta),
        );
        emit(step, "chamber_mix_inert", chamber.system.mix().p_inert);
        emit(step, "chamber_volume", chamber.get_volume(&sim.solver.bodies));
        emit(step, "chamber_lit", if chamber.lit { 1.0 } else { 0.0 });
        emit(step, "chamber_afr", chamber.last_event_afr());
        emit(
            step,
            "chamber_exhaust_flow",
            chamber.last_iteration_exhaust_flow(),
        );
        {
            let attenuation = sim.filtered_engine_speed.abs().min(40.0) / 40.0;
            emit(step, "filtered_speed", sim.filtered_engine_speed);
            emit(step, "attenuation_3", attenuation * attenuation * attenuation);
            emit(
                step,
                "exhaust_dp",
                chamber.exhaust_runner_and_primary.pressure() - units::pressure(1.0, units::ATM),
            );
            emit(
                step,
                "exhaust_dyn_p",
                chamber.exhaust_runner_and_primary.dynamic_pressure(1.0, 0.0),
            );
            emit(
                step,
                "exhaust_dyn_n",
                chamber.exhaust_runner_and_primary.dynamic_pressure(-1.0, 0.0),
            );
        }
        emit(step, "exhaust_signal", sim.exhaust_flow_staging_buffer[0]);
        emit(step, "synth_latency", sim.synthesizer_latency());
        emit(
            step,
            "synth_channels",
            sim.synthesizer.input_channel_count as f64,
        );
        emit(
            step,
            "synth_input_size",
            sim.synthesizer.input_channels[0].data.size() as f64,
        );
        emit(step, "synth_write_offset", sim.synthesizer.input_write_offset);
        emit(step, "frame_steps", sim.steps as f64);
        emit(step, "dyno_torque", sim.dyno_torque());
    }
}
