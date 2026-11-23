//! Diagnostic: run the TRX520 fixture and print the cylinder state.
//!
//! `cargo run --release --example trx520_debug -- 2000`

include!("../fixtures/trx520.rs");

fn main() {
    let steps: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    let mut sim = build_simulator();
    sim.set_starter_enabled(true);

    let atm = units::pressure(1.0, units::ATM);

    println!(
        "{:>6} {:>8} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "step", "rpm", "vol(L)", "p(atm)", "T(K)", "fuel%", "o2%", "intake", "lit",
        "plen", "runner", "liftI", "cycAng"
    );

    for i in 0..steps {
        sim.step(1);

        if i % 25 == 0 || i < 5 {
            let c: &CombustionChamber = &sim.engine.chambers[0];
            let mix = c.system.mix();
            let crank_angle = sim.engine.crankshafts[0].get_angle(&sim.solver.bodies);
            let cycle = sim.engine.crankshafts[0].get_cycle_angle(0.0, &sim.solver.bodies);
            let lift_i = sim.engine.heads[0].intake_valve_lift(0, crank_angle, false);
            println!(
                "{:>6} {:>8.1} {:>9.4} {:>9.3} {:>9.1} {:>9.4} {:>9.4} {:>9.2e} {:>7} {:>9.4} {:>9.4} {:>9.2e} {:>9.3}",
                i,
                sim.engine.get_rpm(&sim.solver.bodies),
                c.get_volume(&sim.solver.bodies) * 1000.0,
                c.system.pressure() / atm,
                c.system.temperature(),
                mix.p_fuel,
                mix.p_o2,
                c.last_timestep_intake_flow(),
                c.lit,
                sim.engine.intakes[0].system.pressure() / atm,
                c.intake_runner_and_manifold.pressure() / atm,
                lift_i,
                cycle
            );
        }
    }
}
