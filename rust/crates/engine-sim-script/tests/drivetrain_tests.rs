//! The drivetrain the script brings, and the application's where it brings none.
//!
//! `EngineSimApplication::loadScript` reads the vehicle and the transmission out
//! of `es_script::Compiler::output()` and falls back **per object**, so a script
//! that publishes a bike gets the bike's mass and the application's gearbox.
//! `build::simulator` has to make the same choice; the numbers are what the
//! simulation then moves, and the `DRIVE` gate in `run_script_parity.sh` is the
//! comparison against the original.
//!
//! These tests pin which branch each script takes and what the fallback is. The
//! ratios are visible here at all because the transmission is the port's own; in
//! the reference they are protected, which is why the gate is a driven run.

mod support;

use engine_sim_core::units;
use engine_sim_script::build;

use support::{build_program, parity_dir, repo_root};

/// `EngineSimApplication::loadScript`'s transmission, used when a script
/// publishes none.
const APPLICATION_GEARS: [f64; 6] = [2.97, 2.07, 1.43, 1.00, 0.84, 0.56];

fn simulator(script: &std::path::Path) -> engine_sim_core::Simulator {
    let mut program = build_program(script);
    program.execute().expect("the script runs");
    build::simulator(&mut program).expect("the engine builds")
}

#[test]
fn a_script_that_publishes_a_vehicle_and_a_transmission_gets_them() {
    let sim = simulator(&parity_dir().join("engines/engines_atg-video-1_01_honda_trx520.mr"));

    // `assets/engines/atg-video-1/01_honda_trx520.mr`.
    assert_eq!(sim.vehicle.mass, units::mass(500.0, units::KG));
    assert_eq!(sim.vehicle.diff_ratio, 3.33);
    assert_eq!(sim.vehicle.tire_radius, units::distance(11.0, units::INCH));
    assert_eq!(sim.vehicle.rolling_resistance, 200.0);

    let transmission = sim.transmission.lock().unwrap();
    assert_eq!(transmission.gear_ratios, [4.0, 3.5, 3.0, 2.5, 2.0]);
    assert_eq!(
        transmission.max_clutch_torque,
        units::torque(50.0, units::FT_LB)
    );
}

#[test]
fn a_script_that_publishes_only_a_vehicle_keeps_the_application_transmission() {
    let sim = simulator(&parity_dir().join("engines/vehicle_without_transmission.mr"));

    assert_eq!(sim.vehicle.mass, units::mass(900.0, units::KG));
    assert_eq!(sim.vehicle.diff_ratio, 4.11);
    assert_eq!(sim.vehicle.rolling_resistance, 250.0);

    let transmission = sim.transmission.lock().unwrap();
    assert_eq!(transmission.gear_ratios, APPLICATION_GEARS);
    assert_eq!(
        transmission.max_clutch_torque,
        units::torque(1000.0, units::FT_LB)
    );
}

#[test]
fn the_default_script_brings_the_car_its_engine_came_from() {
    // `assets/main.mr` runs the EJ25 asset's own `main`, which publishes the
    // Impreza it was recorded from -- so this is the case the feature is for.
    let sim = simulator(&repo_root().join("assets/main.mr"));

    assert_eq!(sim.vehicle.diff_ratio, 3.9);
    assert_ne!(sim.vehicle.mass, units::mass(1597.0, units::KG));

    let transmission = sim.transmission.lock().unwrap();
    assert_eq!(
        transmission.gear_ratios,
        [3.636, 2.375, 1.761, 1.346, 0.971, 0.756]
    );
}

#[test]
fn a_script_that_publishes_neither_gets_the_application_pair() {
    let sim = simulator(&parity_dir().join("engines/drivetrain_from_application.mr"));

    assert_eq!(sim.vehicle.mass, units::mass(1597.0, units::KG));
    assert_eq!(sim.vehicle.drag_coefficient, 0.25);
    assert_eq!(
        sim.vehicle.cross_section_area,
        units::distance(6.0, units::FOOT) * units::distance(6.0, units::FOOT)
    );
    assert_eq!(sim.vehicle.diff_ratio, 3.42);
    assert_eq!(sim.vehicle.tire_radius, units::distance(10.0, units::INCH));
    assert_eq!(sim.vehicle.rolling_resistance, 2000.0);

    let transmission = sim.transmission.lock().unwrap();
    assert_eq!(transmission.gear_ratios, APPLICATION_GEARS);
    assert_eq!(
        transmission.max_clutch_torque,
        units::torque(1000.0, units::FT_LB)
    );
}

#[test]
fn the_transmission_starts_in_neutral_with_its_clutch_out() {
    // `Transmission`'s own defaults, which are why `--gear` and `--clutch` exist:
    // nothing in the driveline turns until a driver asks it to.
    let sim = simulator(&repo_root().join("assets/main.mr"));

    let transmission = sim.transmission.lock().unwrap();
    assert_eq!(transmission.gear, -1);
    assert_eq!(transmission.clutch_pressure, 0.0);
}
