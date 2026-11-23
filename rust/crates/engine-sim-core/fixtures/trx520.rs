// Shared fixture, `include!`d inside a module by the integration tests and the
// debug/parity examples (`mod fixture { include!("../fixtures/trx520.rs"); }`).
//
// Reproduces `assets/engines/atg-video-1/01_honda_trx520.mr` (single cylinder,
// 96 mm bore, 71.5 mm stroke, 6 in rod) using the same parameter values the
// upstream scripting layer would produce.
//
// Each includer uses a different subset of these helpers, hence the blanket
// `dead_code` allow on the module that wraps this file.

use std::sync::Arc;

#[allow(unused_imports)]
use engine_sim_core::chamber::CombustionChamber;
use engine_sim_core::constants;
use engine_sim_core::drivetrain::{Transmission, Vehicle};
use engine_sim_core::engine::Engine;
use engine_sim_core::fuel::{Fuel, FuelParameters};
use engine_sim_core::function::Function;
use engine_sim_core::gas::GasSystem;
use engine_sim_core::ignition::IgnitionModule;
use engine_sim_core::parts::{
    Camshaft, CamshaftParameters, ConnectingRod, ConnectingRodParameters, Crankshaft,
    CrankshaftParameters, CylinderBank, CylinderBankParameters, CylinderHead, DirectThrottleLinkage,
    ExhaustSystem, ExhaustSystemParameters, HeadCylinder, Intake, IntakeParameters, Piston,
    PistonParameters, Valvetrain,
};
use engine_sim_core::simulator::Simulator;
use engine_sim_core::units;

#[allow(dead_code)]
fn thou() -> f64 {
    units::THOU
}

/// `harmonic_cam_lobe` from `es/actions/actions.mr`
/// (`GenerateHarmonicCamLobeNode::_evaluate`).
#[allow(dead_code)]
fn harmonic_cam_lobe(duration_at_50_thou: f64, gamma: f64, lift: f64, steps: usize) -> Function {
    let angle = duration_at_50_thou / 4.0;
    let s = (2.0 * units::distance(50.0, thou()) / lift).powf(1.0 / gamma) - 1.0;
    let k = s.acos() / angle;
    let extents = constants::PI / k;

    let step = extents / (steps as f64 - 5.0);

    let mut function = Function::new(steps * 2 + 1, 0.0);
    for i in 0..steps {
        if i == 0 {
            function.add_sample(0.0, lift);
        } else {
            let x = i as f64 * step;
            let y = if x >= extents {
                0.0
            } else {
                lift * (0.5 + 0.5 * (k * x).cos()).powf(gamma)
            };
            function.add_sample(x, y);
            function.add_sample(-x, y);
        }
    }

    function.set_filter_radius(step);
    function
}

/// `add_flow_sample` / port-flow table from `es/part-library/parts/heads.mr`.
#[allow(dead_code)]
fn port_flow_function(samples: &[(f64, f64)], attenuation: f64) -> Function {
    let mut f = Function::new(samples.len(), units::distance(50.0, thou()));
    for &(lift_thou, flow) in samples {
        f.add_sample(lift_thou * thou(), GasSystem::k_28in_h2o(flow * attenuation));
    }
    f
}

#[allow(dead_code)]
fn test_fuel() -> Arc<Fuel> {
    // `fuel` node defaults, with max_dilution_effect from `es/objects/objects.mr`.
    let mut turbulence = Function::new(10, 5.0);
    for i in 0..10 {
        let s = 5.0 * i as f64;
        turbulence.add_sample(s, 1.5 * s);
    }
    turbulence.add_sample(0.0, 3.0);

    Arc::new(Fuel::new(FuelParameters {
        molecular_mass: units::mass(100.0, units::G),
        energy_density: units::energy(48.1, units::KJ) / units::mass(1.0, units::G),
        density: units::mass(0.755, units::KG) / units::volume(1.0, units::L),
        turbulence_to_flame_speed_ratio: Arc::new(turbulence),
        molecular_afr: 25.0 / 2.0,
        burning_efficiency_randomness: 0.5,
        max_burning_efficiency: 0.8,
        max_dilution_effect: 10.0,
        max_turbulence_effect: 2.0,
        low_efficiency_attenuation: 0.6,
    }))
}

/// `mean_piston_speed_to_turbulence` from `scripting/include/engine_node.h`.
#[allow(dead_code)]
fn mean_piston_speed_to_turbulence() -> Arc<Function> {
    let mut f = Function::new(30, 1.0);
    for i in 0..30 {
        let s = i as f64;
        f.add_sample(s, s * 0.5);
    }
    Arc::new(f)
}

/// `timing_curve` from the TRX520 script (degrees -> radians).
#[allow(dead_code)]
fn timing_curve() -> Arc<Function> {
    let mut f = Function::new(5, units::rpm(1000.0));
    for &(rpm, advance_deg) in &[
        (0.0, 12.0),
        (1000.0, 12.0),
        (2000.0, 20.0),
        (3000.0, 35.0),
        (4000.0, 35.0),
    ] {
        f.add_sample(units::rpm(rpm), units::angle(advance_deg, units::DEG));
    }
    Arc::new(f)
}

#[allow(dead_code)]
fn build_simulator() -> Simulator {
    let deg = units::DEG;

    // ------------------------------------------------------------- crankshaft
    let crank = Arc::new(Crankshaft::new(CrankshaftParameters {
        mass: units::mass(5.0, units::LB),
        flywheel_mass: units::mass(5.0, units::LB),
        moment_of_inertia: 0.22986844776863666 * 0.2,
        crank_throw: units::distance(71.5, units::MM) / 2.0,
        rod_journals: 1,
        pos_x: 0.0,
        pos_y: 0.0,
        tdc: constants::PI / 2.0,
        friction_torque: units::torque(5.0, units::FT_LB),
    }));

    // ------------------------------------------------------------ cylinder bank
    let bank = Arc::new(CylinderBank::new(CylinderBankParameters {
        angle: 0.0,
        bore: units::distance(96.0, units::MM),
        deck_height: units::distance(71.5, units::MM) / 2.0
            + units::distance(6.0, units::INCH)
            + units::distance(1.0, units::INCH),
        cylinder_count: 1,
        position_x: 0.0,
        position_y: 0.0,
        display_depth: 0.4,
        index: 0,
    }));

    // --------------------------------------------------------------- intake
    let intake = Intake::new(&IntakeParameters {
        volume: units::volume(1.5, units::L),
        cross_section_area: units::area(10.0, units::CM2),
        input_flow_k: GasSystem::k_carb(100.0),
        idle_flow_k: GasSystem::k_carb(0.0),
        runner_flow_rate: GasSystem::k_carb(300.0),
        idle_throttle_plate_position: 0.993,
        velocity_decay: 0.5,
        ..Default::default()
    });

    // --------------------------------------------------------- exhaust system
    let exhaust_area = constants::PI * units::distance(2.0, units::INCH).powi(2);
    let exhaust = ExhaustSystem::new(&ExhaustSystemParameters {
        length: units::volume(5.0, units::L) / exhaust_area,
        collector_cross_section_area: exhaust_area,
        outlet_flow_rate: GasSystem::k_carb(500.0),
        primary_tube_length: units::distance(20.0, units::INCH),
        primary_flow_rate: GasSystem::k_carb(200.0),
        velocity_decay: 0.5,
        audio_volume: 1.0,
        impulse_response: None,
    });

    // ------------------------------------------------------------- valve train
    let lobe = Arc::new(harmonic_cam_lobe(
        units::angle(180.0, deg),
        1.0,
        units::distance(200.0, thou()),
        100,
    ));

    let lobe_separation = units::angle(100.0, deg);
    let rot360 = units::angle(360.0, deg);

    let mut intake_cam = Camshaft::new(CamshaftParameters {
        lobes: 1,
        lobe_profile: lobe.clone(),
        advance: 0.0,
        base_radius: units::distance(500.0, thou()),
    });
    intake_cam.set_lobe_centerline(0, rot360 + lobe_separation);

    let mut exhaust_cam = Camshaft::new(CamshaftParameters {
        lobes: 1,
        lobe_profile: lobe.clone(),
        advance: 0.0,
        base_radius: units::distance(500.0, thou()),
    });
    exhaust_cam.set_lobe_centerline(0, rot360 - lobe_separation);

    // `generic_small_engine_head` with `flow_attenuation: 2.0`.
    let intake_flow = Arc::new(port_flow_function(
        &[
            (0.0, 0.0),
            (50.0, 25.0),
            (100.0, 75.0),
            (150.0, 100.0),
            (200.0, 130.0),
            (250.0, 180.0),
            (300.0, 190.0),
            (350.0, 220.0),
            (400.0, 240.0),
            (450.0, 250.0),
            (500.0, 260.0),
            (550.0, 260.0),
            (600.0, 260.0),
            (650.0, 255.0),
            (700.0, 250.0),
        ],
        2.0,
    ));

    let exhaust_flow = Arc::new(port_flow_function(
        &[
            (0.0, 0.0),
            (50.0, 25.0),
            (100.0, 50.0),
            (150.0, 75.0),
            (200.0, 100.0),
            (250.0, 125.0),
            (300.0, 160.0),
            (350.0, 175.0),
            (400.0, 180.0),
            (450.0, 190.0),
            (500.0, 200.0),
            (550.0, 205.0),
            (600.0, 210.0),
            (650.0, 210.0),
            (700.0, 210.0),
        ],
        2.0,
    ));

    let head = Arc::new(CylinderHead {
        bank: bank.clone(),
        valvetrain: Arc::new(Valvetrain::Standard {
            intake_camshaft: Arc::new(intake_cam),
            exhaust_camshaft: Arc::new(exhaust_cam),
        }),
        exhaust_port_flow: exhaust_flow,
        intake_port_flow: intake_flow,
        intake_runner_volume: units::volume(100.0, units::CC),
        intake_runner_cross_section_area: units::area(30.0, units::CM2),
        exhaust_runner_volume: units::volume(100.0, units::CC),
        exhaust_runner_cross_section_area: units::area(30.0, units::CM2),
        combustion_chamber_volume: units::volume(60.0, units::CC),
        flip_display: false,
        cylinders: vec![HeadCylinder {
            exhaust_system: Some(0),
            intake: Some(0),
            sound_attenuation: 1.0,
            header_primary_length: 0.0,
        }],
    });

    // ------------------------------------------------------------ rod / piston
    let rod = Arc::new(ConnectingRod::new(ConnectingRodParameters {
        center_of_mass: 0.0,
        length: units::distance(6.0, units::INCH),
        mass: units::mass(100.0, units::G),
        moment_of_inertia: 0.0015884918028487504,
        journal: 0,
        crankshaft: Some(crank.clone()),
        slave_throw: 0.0,
        rod_journals: 1,
        master: None,
        piston_bank: Some(bank.clone()),
        piston_cylinder_index: 0,
    }));

    let piston = Piston::new(PistonParameters {
        rod: Some(rod.clone()),
        bank: Some(bank.clone()),
        cylinder_index: 0,
        compression_height: units::distance(1.0, units::INCH),
        displacement: 0.0,
        wrist_pin_position: 0.0,
        mass: units::mass(100.0, units::G),
        blowby_flow_coefficient: GasSystem::k_28in_h2o(0.1),
    });

    // ---------------------------------------------------------------- ignition
    let mut ignition = IgnitionModule::new(
        1,
        timing_curve(),
        units::rpm(6000.0),
        0.5 * units::SEC,
    );
    ignition.set_firing_order(0, units::angle(0.0, deg));
    ignition.enabled = true;

    // ------------------------------------------------------------------ engine
    let mut engine = Engine::new(
        "Honda TRX520 (ATV)".to_string(),
        test_fuel(),
        mean_piston_speed_to_turbulence(),
        ignition,
    );

    engine.crankshafts.push(crank.clone());
    engine.banks.push(bank.clone());
    engine.heads.push(head);
    engine.pistons.push(piston);
    engine.rods.push(rod);
    engine.intakes.push(intake);
    engine.exhausts.push(exhaust);

    engine.starter_torque = units::torque(50.0, units::FT_LB);
    engine.starter_speed = units::rpm(500.0);
    engine.redline = units::rpm(5000.0);
    engine.initial_simulation_frequency = 10000.0;
    engine.throttle = Box::new(DirectThrottleLinkage::default());
    engine.set_speed_control(1.0); // WOT

    let vehicle = Vehicle::new(
        units::mass(500.0, units::KG),
        0.25,
        units::distance(47.0, units::INCH) * units::distance(47.0, units::INCH),
        3.33,
        units::distance(11.0, units::INCH),
        units::force(200.0, units::N),
    );

    let transmission = Transmission::new(
        vec![4.0, 3.5, 3.0, 2.5, 2.0],
        units::torque(50.0, units::FT_LB),
    );

    Simulator::new(engine, vehicle, transmission)
}

