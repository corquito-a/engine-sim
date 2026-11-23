//! Rust half of the whole-core parity check.
//!
//! Mirrors `rust/tools/parity/core_reference.cpp` step for step and prints
//! `<step>,<field>,<16 hex digits>` so the two outputs can be diffed
//! bit-for-bit (see `rust/tools/parity/run_core_parity.sh`).

include!("../fixtures/trx520.rs");

fn emit(step: usize, name: &str, value: f64) {
    println!("{step},{name},{:016x}", value.to_bits());
}

fn main() {
    let step_count: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    let mut sim = build_simulator();
    sim.set_starter_enabled(true);

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
        emit(step, "dyno_torque", sim.dyno_torque());
    }
}
