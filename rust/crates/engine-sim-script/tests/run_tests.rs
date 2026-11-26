//! The `RUN` gate: the port's own engine, built from the script and simulated.
//!
//! `fixtures/main_engine.trace` is recorded from `script_reference --run` on
//! `assets/main.mr`: 97 engine-header lines, then 400 frames of physics, gas
//! dynamics, synthesizer input and rendered audio, `step,field,bits`. The port
//! builds the same engine from the same script through its own front end, puts
//! it in the same default vehicle and transmission, and has to reproduce every
//! line -- which is the whole of M4, because nothing in between is stubbed.
//!
//! `run_script_parity.sh` runs the same comparison through the two binaries.

mod support;

use engine_sim_script::engine_dump;
use engine_sim_script::{build, trace};

use support::{build_program, repo_root};

#[test]
fn the_scripts_engine_simulates_like_the_original() {
    let script = repo_root().join("assets/main.mr");

    let mut program = build_program(&script);
    program.execute().expect("assets/main.mr runs");

    let mut simulator = build::simulator(&mut program).expect("the engine builds");

    let mut actual = String::from("-- engine --\n");

    for line in engine_dump::dump_with_displacement(&mut program, simulator.engine.displacement) {
        actual.push_str(&line);
        actual.push('\n');
    }

    actual.push_str(&trace::frames(&mut simulator, 400));

    let expected = std::fs::read_to_string(
        repo_root().join("rust/crates/engine-sim-script/fixtures/main_engine.trace"),
    )
    .expect("the recorded trace is missing");

    if actual != expected {
        let actual: Vec<&str> = actual.lines().collect();
        let expected: Vec<&str> = expected.lines().collect();

        let first = actual
            .iter()
            .zip(expected.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(actual.len().min(expected.len()));

        panic!(
            "the trace differs at line {} (port has {}, fixture has {}):\n  port:    {}\n  fixture: {}",
            first + 1,
            actual.len(),
            expected.len(),
            actual.get(first).copied().unwrap_or("<end>"),
            expected.get(first).copied().unwrap_or("<end>"),
        );
    }

    assert_eq!(actual.lines().count(), 16_265);
}

#[test]
fn the_engine_has_the_parts_the_script_asked_for() {
    let script = repo_root().join("assets/main.mr");

    let mut program = build_program(&script);
    program.execute().expect("assets/main.mr runs");

    let built = build::build(&mut program)
        .expect("the engine builds")
        .expect("the script builds an engine");
    let engine = built.engine;

    assert_eq!(engine.banks.len(), 2);
    assert_eq!(engine.crankshafts.len(), 1);
    assert_eq!(engine.pistons.len(), 4);
    assert_eq!(engine.rods.len(), 4);
    assert_eq!(engine.heads.len(), 2);
    assert_eq!(engine.intakes.len(), 1);
    assert_eq!(engine.exhausts.len(), 1);

    // The rod journals the crankshaft indexed, and the angles the script set.
    assert_eq!(engine.crankshafts[0].rod_journal_count(), 4);

    // The head's cylinders point at the intakes and exhausts the script wired,
    // and carry the per-cylinder attenuation and header length.
    let head = &engine.heads[0];
    assert_eq!(head.cylinders.len(), 2);
    assert_eq!(head.cylinders[0].intake, Some(0));
    assert_eq!(head.cylinders[0].exhaust_system, Some(0));

    // The lobe centrelines the `add_lobe` actions wrote, halved by
    // `Camshaft::setLobeCenterline`.
    let valvetrain = engine.heads[0].valvetrain.clone();
    let camshaft = match valvetrain.as_ref() {
        engine_sim_core::parts::Valvetrain::Standard {
            intake_camshaft, ..
        } => intake_camshaft.clone(),
        _ => panic!("the EJ25 head is a standard valvetrain"),
    };

    assert_eq!(camshaft.lobe_angles.len(), 2);
    assert!(camshaft.lobe_angles[0] > 0.0);
}
