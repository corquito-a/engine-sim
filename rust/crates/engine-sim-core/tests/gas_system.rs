//! Port of `test/gas_system_tests.cpp` (the upstream gtest suite).
//!
//! Tests that upstream only dumped to CSV are dropped here; the numerically
//! interesting ones are covered instead by the bit-exact parity harness in
//! `rust/tools/parity`, which is a much stronger check than a CSV dump.

use engine_sim_core::constants;
use engine_sim_core::gas::{FlowParameters, GasSystem, Mix};
use engine_sim_core::units;

fn near(actual: f64, expected: f64, tolerance: f64, what: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= tolerance,
        "{what}: actual = {actual:.17e}, expected = {expected:.17e}, delta = {delta:.3e} > {tolerance:.3e}"
    );
}

// GasSystemTests.GasSystemSanity
#[test]
fn gas_system_sanity() {
    let mut system = GasSystem::default();
    system.initialize(0.0, 0.0, 0.0, Mix::DEFAULT, 5);
}

// GasSystemTests.AdiabaticEnergyConservation
#[test]
fn adiabatic_energy_conservation() {
    let piston_area = units::area(1.0, units::CM2);
    let vessel_height = units::distance(1.0, units::CM);
    let compression = vessel_height * 0.5;
    let steps = 10_000;

    let mut system = GasSystem::default();
    system.initialize_default(
        units::pressure(1.0, units::ATM),
        units::volume(1.0, units::CC),
        units::celcius(25.0),
    );

    let initial_system_energy = system.kinetic_energy();
    let initial_molecules = system.n();

    let mut w = 0.0;
    let mut current_piston_height = vessel_height;
    for i in 1..=steps {
        let new_piston_height = vessel_height - (compression / steps as f64) * i as f64;
        let dh = current_piston_height - new_piston_height;
        let f = system.pressure() * piston_area;
        w += f * dh;

        system.change_volume((new_piston_height - current_piston_height) * piston_area);

        current_piston_height = new_piston_height;
    }

    let final_system_energy = system.kinetic_energy();
    let final_molecules = system.n();

    near(final_molecules, initial_molecules, 1e-6, "molecules conserved");
    near(
        final_system_energy - initial_system_energy,
        w,
        1e-4,
        "work in == energy change",
    );
}

// GasSystemTests.PressureEqualizationEnergyConservation
#[test]
fn pressure_equalization_energy_conservation() {
    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(1.0, units::ATM),
        units::volume(1000.0, units::CC),
        units::celcius(25.0),
    );

    let mut system2 = GasSystem::default();
    system2.initialize_default(
        units::pressure(2.0, units::ATM),
        units::volume(1000.0, units::CC),
        units::celcius(25.0),
    );

    let initial_system_energy = system1.total_energy() + system2.total_energy();
    let initial_molecules = system1.n() + system2.n();

    let params = FlowParameters {
        k_flow: 0.000001,
        cross_section_area_0: 1.0,
        cross_section_area_1: 1.0,
        direction_x: 1.0,
        direction_y: 0.0,
        dt: 1.0,
    };

    let steps = 1000;
    for _ in 1..=steps {
        GasSystem::flow(&mut system1, &mut system2, &params);
    }

    let final_system_energy = system1.total_energy() + system2.total_energy();
    let final_molecules = system1.n() + system2.n();

    near(final_molecules, initial_molecules, 1e-6, "molecules conserved");
    near(final_system_energy, initial_system_energy, 1e-4, "energy conserved");
}

// GasSystemTests.PressureEquilibriumMaxFlow
//
// NOTE: the upstream assertions (`EXPECT_NEAR(pressure1, pressure2, 1e-6)` in
// both halves) do NOT hold for the current implementation — verified against
// the original C++ sources with `g++` (see rust/tools/parity), which produces
// the same numbers below. `pressureEquilibriumMaxFlow` returns a *signed* flow
// measured out of `self`, so the upstream test's use of `gainN(negative)` makes
// the receiver lose energy until its pressure goes negative. This is a
// characterisation test: it pins the behaviour the port must reproduce, not the
// behaviour the original author intended.
#[test]
fn pressure_equilibrium_max_flow_is_characterised() {
    let mut system1 = GasSystem::default();
    system1.initialize(
        units::pressure(1.0, units::ATM),
        units::volume(1.0, units::CC),
        units::celcius(2500.0),
        Mix::DEFAULT,
        5,
    );

    let mut system2 = GasSystem::default();
    system2.initialize(
        units::pressure(2.0, units::ATM),
        units::volume(1.0, units::CC),
        units::celcius(25.0),
        Mix::DEFAULT,
        5,
    );

    let max_flow_in = system1.pressure_equilibrium_max_flow(&system2);
    near(max_flow_in, -2.04370222617880662e-05, 1e-18, "max flow (in)");

    let e_k_per_mol = system1.kinetic_energy_per_mol();
    system1.lose_n(max_flow_in, e_k_per_mol);
    system2.gain_n(max_flow_in, e_k_per_mol, system1.mix());

    near(system1.pressure(), 5.72546572614455945e+05, 1e-3, "P1 after transfer");
    near(system2.pressure(), -2.68571572614456003e+05, 1e-3, "P2 after transfer");
}

// GasSystemTests.PressureEquilibriumMaxFlowInfinite
//
// Same story as above: the upstream expectation (pressure reaches ambient)
// fails on the original C++ as well, because `gainN` is called with the
// signed (negative) outflow. Characterisation test.
#[test]
fn pressure_equilibrium_max_flow_infinite_is_characterised() {
    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(1.0, units::ATM),
        units::volume(1.0, units::CC),
        units::celcius(25.0),
    );

    let p_env = units::pressure(2.0, units::ATM);
    let t_env = units::celcius(25.0);

    let max_flow = system1.pressure_equilibrium_max_flow_env(p_env, t_env);
    near(max_flow, -4.08740445235761189e-05, 1e-18, "max flow");

    let e_k_per_mol = GasSystem::kinetic_energy_per_mol_t(t_env, system1.degrees_of_freedom());
    system1.gain_n(max_flow, e_k_per_mol, Mix::DEFAULT);

    // The volume ends up empty (n clamps at zero, energy at zero).
    assert_eq!(system1.n(), 0.0);
    assert_eq!(system1.pressure(), 0.0);
}

// GasSystemTests.PressureEquilibriumMaxFlowInfiniteOverpressure
//
// The upstream test removes moles carrying *ambient* energy per mole from a
// 2500 C volume, so the pressure does not land on the ambient value. Verified
// against the original C++ (same result); pinned here.
#[test]
fn pressure_equilibrium_max_flow_infinite_overpressure_is_characterised() {
    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(100.0, units::ATM),
        units::volume(1.0, units::M3),
        units::celcius(2500.0),
    );

    let p_env = units::pressure(2.0, units::ATM);
    let t_env = units::celcius(25.0);

    let max_flow = system1.pressure_equilibrium_max_flow_env(p_env, t_env);
    near(max_flow, 4.30660600660264720e+02, 1e-9, "max flow");

    system1.lose_n(max_flow, GasSystem::kinetic_energy_per_mol_t(t_env, 5));

    near(system1.pressure(), 9.06491087662045099e+06, 1e-2, "pressure after blowdown");
    near(system1.n(), 8.78899185020952700e+00, 1e-12, "moles after blowdown");
}

// GasSystemTests.FlowLimit
#[test]
fn flow_limit() {
    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(100.0, units::ATM),
        units::volume(1.0, units::M3),
        units::celcius(2000.0),
    );

    let p_env = units::pressure(1.0, units::ATM);
    let t_env = units::celcius(25.0);

    system1.flow_env_default(15.0, 10.0, p_env, t_env);

    near(system1.pressure(), p_env, 1e-6, "flow is clamped to equilibrium");
}

// GasSystemTests.IdealGasLaw
#[test]
fn ideal_gas_law() {
    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(100.0, units::ATM),
        units::volume(1.0, units::M3),
        units::celcius(2000.0),
    );

    let pv = system1.pressure() * system1.volume();
    let nrt = system1.n() * constants::R * system1.temperature();

    near(pv, nrt, 1e-6, "PV == nRT");
}

// GasSystemTests.CompositionSanityCheck
#[test]
fn composition_sanity_check() {
    let a = Mix::new(1.0, 1.0, 1.0);
    let b = Mix::new(0.0, 0.0, 0.0);

    let mut system1 = GasSystem::default();
    system1.initialize(
        units::pressure(100.0, units::ATM),
        units::volume(100.0, units::M3),
        units::celcius(2000.0),
        a,
        5,
    );

    let mut system2 = GasSystem::default();
    system2.initialize(
        units::pressure(1.0, units::ATM),
        units::volume(1.0, units::M3),
        units::celcius(2000.0),
        b,
        5,
    );

    let params = FlowParameters {
        k_flow: 1.0,
        cross_section_area_0: 1.0,
        cross_section_area_1: 1.0,
        direction_x: 1.0,
        direction_y: 0.0,
        dt: 1.0 / 60.0,
    };

    for _ in 0..200 {
        GasSystem::flow(&mut system1, &mut system2, &params);
    }

    let mix = system2.mix();
    near(mix.p_fuel, 1.0, 2e-2, "fuel fraction");
    near(mix.p_inert, 1.0, 2e-2, "inert fraction");
    near(mix.p_o2, 1.0, 2e-2, "oxygen fraction");
}

// GasSystemTests.FlowRateConstant
#[test]
fn flow_rate_constant() {
    let flow_k = GasSystem::flow_constant(
        units::flow(400.0, units::SCFM),
        units::pressure(2.5, units::ATM),
        units::pressure(0.5, units::ATM),
        units::celcius(2000.0),
        GasSystem::heat_capacity_ratio_of(5),
    );

    let flow_rate = GasSystem::flow_rate(
        flow_k,
        units::pressure(2.5, units::ATM),
        units::pressure(2.5 - 0.5, units::ATM),
        units::celcius(2000.0),
        units::celcius(25.0),
        GasSystem::heat_capacity_ratio_of(5),
        GasSystem::choked_flow_limit(5),
        GasSystem::choked_flow_rate(5),
    );

    near(
        flow_rate,
        units::flow(400.0, units::SCFM),
        1e-6,
        "flow constant reproduces target flow",
    );
}

// GasSystemTests.GasVelocityReducesStaticPressure
#[test]
fn gas_velocity_reduces_static_pressure() {
    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(15.0, units::PSI),
        units::volume(300.0, units::CC),
        units::celcius(25.0),
    );
    system1.set_geometry(
        units::distance(10.0, units::CM),
        units::distance(10.0, units::CM),
        1.0,
        0.0,
    );

    let mut system2 = GasSystem::default();
    system2.initialize_default(
        units::pressure(2.0, units::PSI),
        units::volume(1.0, units::L),
        units::celcius(25.0),
    );
    system2.set_geometry(
        units::distance(10.0, units::CM),
        units::distance(2.0, units::CM),
        1.0,
        0.0,
    );

    let initial_system_energy = system1.total_energy() + system2.total_energy();
    let initial_molecules = system1.n() + system2.n();

    let params = FlowParameters {
        k_flow: GasSystem::k_28in_h2o(500.0) * 1.0,
        cross_section_area_0: 50.0 * units::CM * units::CM,
        cross_section_area_1: 4.0 * units::CM * units::CM,
        direction_x: 1.0,
        direction_y: 0.0,
        dt: 1.0 / 10_000.0,
    };

    let steps = 1000;
    for _ in 1..=steps {
        GasSystem::flow(&mut system1, &mut system2, &params);
        system1.update_velocity_default(params.dt);
        system2.update_velocity_default(params.dt);
    }

    let final_system_energy = system1.total_energy() + system2.total_energy();
    let final_molecules = system1.n() + system2.n();

    near(initial_molecules, final_molecules, 1e-6, "molecules conserved");
    near(initial_system_energy, final_system_energy, 1e-4, "energy conserved");
}

// GasVelocityStabilizesInClosedSystem (upstream: CSV dump only).
//
// Upstream only dumped a CSV, so this is a stronger version: a one-way flow
// builds bulk momentum in the receiving volume, then `updateVelocity` runs on a
// closed system. The bulk velocity must decay and energy must be conserved.
#[test]
fn gas_velocity_stabilizes_in_closed_system() {
    // Large, slowly-emptying source so the flow stays one-directional.
    let mut source = GasSystem::default();
    source.initialize_default(
        units::pressure(100.0, units::PSI),
        units::volume(20_000.0, units::CC),
        units::celcius(300.0),
    );
    source.set_geometry(
        units::distance(10.0, units::CM),
        units::distance(10.0, units::CM),
        1.0,
        0.0,
    );

    let mut system1 = GasSystem::default();
    system1.initialize_default(
        units::pressure(14.0, units::PSI),
        units::volume(1000.0, units::CC),
        units::celcius(300.0),
    );
    system1.set_geometry(
        units::distance(10.0, units::CM),
        units::distance(10.0, units::CM),
        1.0,
        0.0,
    );

    let params = FlowParameters {
        k_flow: GasSystem::k_28in_h2o(500.0),
        cross_section_area_0: 4.0 * units::CM * units::CM,
        cross_section_area_1: 4.0 * units::CM * units::CM,
        direction_x: 1.0,
        direction_y: 0.0,
        dt: 1.0 / 10_000.0,
    };

    for _ in 0..200 {
        GasSystem::flow(&mut source, &mut system1, &params);
    }

    let initial_velocity = system1.velocity_x();
    assert!(
        initial_velocity.abs() > 1e-6,
        "gas should pick up bulk velocity (got {initial_velocity:e})"
    );

    let dt = 1.0 / 10_000.0;
    let energy_before = system1.total_energy();
    for _ in 1..=10_000 {
        system1.update_velocity_default(dt);
    }

    assert!(
        system1.velocity_x().abs() < initial_velocity.abs(),
        "bulk velocity should decay in a closed volume"
    );
    near(system1.total_energy(), energy_before, 1e-4, "energy conserved");
}

/// Not an upstream test: invariant checks that the C++ relies on implicitly.
#[test]
fn invariants_hold_over_a_long_flow_run() {
    let mut hi = GasSystem::default();
    hi.initialize_default(
        units::pressure(20.0, units::PSI),
        units::volume(500.0, units::CC),
        units::celcius(900.0),
    );
    hi.set_geometry(
        units::distance(10.0, units::CM),
        units::distance(2.0, units::CM),
        1.0,
        0.0,
    );

    let mut lo = GasSystem::default();
    lo.initialize_default(
        units::pressure(14.7, units::PSI),
        units::volume(2000.0, units::CC),
        units::celcius(300.0),
    );
    lo.set_geometry(
        units::distance(10.0, units::CM),
        units::distance(2.0, units::CM),
        1.0,
        0.0,
    );

    let params = FlowParameters {
        k_flow: GasSystem::k_28in_h2o(400.0),
        cross_section_area_0: 6.0 * units::CM * units::CM,
        cross_section_area_1: 6.0 * units::CM * units::CM,
        direction_x: 1.0,
        direction_y: 0.0,
        dt: 1.0 / 20_000.0,
    };

    let energy0 = hi.total_energy() + lo.total_energy();
    let n0 = hi.n() + lo.n();

    for _ in 0..20_000 {
        GasSystem::flow(&mut hi, &mut lo, &params);
        hi.update_velocity_default(params.dt);
        lo.update_velocity_default(params.dt);
    }

    near(n0, hi.n() + lo.n(), 1e-6, "molecules conserved");
    near(energy0, hi.total_energy() + lo.total_energy(), 1e-4, "energy conserved");
    assert!(hi.pressure().is_finite() && lo.pressure().is_finite());
    assert!(hi.pressure() >= 0.0 && lo.pressure() >= 0.0);
}
