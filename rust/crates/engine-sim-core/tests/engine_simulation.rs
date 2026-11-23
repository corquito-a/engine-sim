//! End-to-end smoke tests for the ported simulation core.
//!
//! The fixture in `fixtures/trx520.rs` reproduces
//! `assets/engines/atg-video-1/01_honda_trx520.mr` with the same parameter
//! values the upstream scripting layer produces.

include!("../fixtures/trx520.rs");

fn assert_all_finite(sim: &Simulator) {
    for (i, body) in sim.solver.bodies.iter().enumerate() {
        assert!(
            body.p_x.is_finite()
                && body.p_y.is_finite()
                && body.theta.is_finite()
                && body.v_x.is_finite()
                && body.v_y.is_finite()
                && body.v_theta.is_finite(),
            "body {i} diverged: {body:?}"
        );
    }

    for (i, chamber) in sim.engine.chambers.iter().enumerate() {
        let pressure = chamber.system.pressure();
        let temperature = chamber.system.temperature();
        assert!(
            pressure.is_finite() && temperature.is_finite(),
            "chamber {i} diverged: p = {pressure}, T = {temperature}"
        );
        assert!(pressure > 0.0, "chamber {i} has non-positive pressure");
    }
}

/// Advances the simulation by exactly one physics step, wrapped in a frame the
/// way the reference application drives it. A sub-half-timestep frame `dt`
/// makes `simulateStep()` run a single step.
fn step_one_frame(sim: &mut Simulator) {
    let timestep = sim.get_timestep();
    sim.start_frame(timestep * 0.1);
    while sim.simulate_step() {}
    sim.end_frame();
}

#[test]
fn engine_can_be_cranked_by_the_starter_motor() {
    let mut sim = build_simulator();

    assert_eq!(sim.solver.bodies.len(), 4, "crank, vehicle mass, piston, rod");

    sim.set_starter_enabled(true);
    sim.step(1000); // 0.1 s at 10 kHz

    assert_all_finite(&sim);

    let speed = sim.engine.get_speed(&sim.solver.bodies);
    let rpm = sim.engine.get_rpm(&sim.solver.bodies);
    assert!(
        rpm > 50.0,
        "starter motor should spin the engine, got {rpm} rpm ({speed} rad/s)"
    );

    // The starter drives the crank clockwise (`v_theta < 0`).
    assert!(sim.engine.is_spinning_cw(&sim.solver.bodies));
}

#[test]
fn combustion_raises_cylinder_pressure_above_atmospheric() {
    let mut sim = build_simulator();

    // Note: the dynamometer is deliberately left disabled. Holding it at a
    // fixed speed stalls this fixture in the reference implementation too
    // (verified bit-for-bit against `tools/parity/core_reference.cpp` with
    // `PARITY_DYNO=1`), so the free-revving starter configuration is the one
    // that actually reaches combustion.
    sim.set_starter_enabled(true);

    // The intake has to fill the runner before the first charge reaches the
    // cylinder: in the reference implementation the first combustion event
    // lands around 0.9 s of simulated time at 10 kHz.
    //
    // One physics step per frame, exactly like the reference application
    // (`startFrame` / `simulateStep` ... / `endFrame`), which is also the
    // configuration the bit-exact parity harness uses.
    let mut peak_pressure: f64 = 0.0;
    let mut max_temperature: f64 = 0.0;

    for _ in 0..15000 {
        step_one_frame(&mut sim);
        let p = sim.engine.chambers[0].system.pressure();
        let t = sim.engine.chambers[0].system.temperature();
        peak_pressure = peak_pressure.max(p);
        max_temperature = max_temperature.max(t);
    }

    assert_all_finite(&sim);

    let atm = units::pressure(1.0, units::ATM);
    assert!(
        peak_pressure > 2.0 * atm,
        "expected combustion pressure rise, peak = {} atm (T_max = {} K)",
        peak_pressure / atm,
        max_temperature
    );
    assert!(
        max_temperature > units::celcius(400.0),
        "expected a temperature rise from combustion, max = {max_temperature} K"
    );

    // Something must have burned.
    assert!(
        sim.engine.chambers[0].n_burnt_fuel > 0.0,
        "no fuel was burned"
    );
}

#[test]
fn chamber_volume_sweeps_between_bdc_and_tdc() {
    let sim = build_simulator();
    let chamber = &sim.engine.chambers[0];

    let bodies = &sim.solver.bodies;
    let volume = chamber.get_volume(bodies);

    // 96 mm bore, 71.5 mm stroke, 60 cc chamber.
    let swept = constants::PI * units::distance(96.0, units::MM).powi(2) / 4.0
        * units::distance(71.5, units::MM);
    let clearance = units::volume(60.0, units::CC);

    assert!(volume > 0.0);
    assert!(
        volume >= clearance * 0.9,
        "volume {volume} below clearance volume"
    );
    assert!(
        volume <= clearance + swept * 1.1,
        "volume {volume} above clearance + swept volume"
    );

    // Displacement calculation should find the swept volume.
    let mut engine = sim.engine;
    engine.calculate_displacement();
    let two_pi_over = 2.0 * constants::PI;
    let _ = two_pi_over;
    assert!(
        (engine.displacement - swept).abs() / swept < 0.02,
        "displacement {} vs swept {swept}",
        engine.displacement
    );
}

#[test]
fn combustion_chamber_state_samples_stay_in_range() {
    // `CombustionChamber` indexes its per-cycle state arrays by crank angle; a
    // regression here would panic (index out of range) rather than produce a
    // wrong number.
    let mut sim = build_simulator();
    sim.set_starter_enabled(true);

    for _ in 0..500 {
        sim.step(1);
    }

    let chamber: &CombustionChamber = &sim.engine.chambers[0];
    assert!(chamber.calculate_firing_pressure().is_finite());
    assert!(chamber.calculate_mean_piston_speed().is_finite());
}
