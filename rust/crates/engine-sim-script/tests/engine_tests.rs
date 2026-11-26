//! The engine header: what `EngineNode::buildEngine` decided, before anything
//! is simulated.
//!
//! `tools/parity/engine/main.engine` is recorded from `script_reference --engine`
//! on `assets/main.mr` -- the only script that builds an engine. `run_graph`'s
//! ENGINE gate compares the two binaries; this is the same comparison without a
//! C++ build behind it, and it pins the two properties the header is easy to get
//! wrong: a script that builds no engine prints nothing, and the displacement
//! fields are zero until the application calls `calculateDisplacement`.

mod support;

use engine_sim_script::engine_dump;

use std::path::Path;

use support::{build_program, parity_dir, repo_root};

#[test]
fn the_engine_main_mr_configures_matches_the_recorded_header() {
    let script = repo_root().join("assets/main.mr");
    let mut program = build_program(&script);
    program.execute().expect("assets/main.mr runs");

    let expected = std::fs::read_to_string(
        parity_dir().join("engine/main.engine"),
    )
    .expect("the recorded engine header is missing");

    let actual: String = engine_dump::dump(&mut program)
        .iter()
        .map(|line| format!("{}\n", line))
        .collect();

    assert_eq!(actual, expected, "the engine header differs");

    // The parts of it that are easy to lose track of.
    let lines: Vec<&str> = actual.lines().collect();
    assert!(lines.contains(&"E,name,Subaru EJ25"));
    assert!(lines.contains(&"E,cylinder_count,4"));
    assert!(lines.contains(&"E,crankshaft_count,1"));

    // `calculateDisplacement` is the application's call, so the script's engine
    // reports no displacement yet -- and neither do its pistons.
    assert!(lines.contains(&"E,displacement,0000000000000000"));
    assert!(lines.contains(&"E,cyl0_piston_displacement,0000000000000000"));

    // The audio the exhaust runs on: the file the script's library node named,
    // resolved against that node's own unit (`es/sound-library/`), at the volume
    // the node asked for. `main.mr` imports `01_subaru_ej25_eh.mr`, which uses
    // `ir_lib.minimal_muffling_02`.
    assert!(lines.contains(&"E,exhaust0_impulse_response,es/sound-library/new/minimal_muffling_02.wav"));
    assert!(lines.contains(&"E,exhaust0_impulse_response_volume,3f847ae147ae147b"));
}

/// The response an exhaust names is a file, and the audio layer reads it: the
/// path the node resolved has to exist, and has to decode to the sample count
/// the synthesizer will then convolve with.
#[test]
fn the_exhaust_impulse_response_is_a_file_that_decodes() {
    let script = repo_root().join("assets/main.mr");
    let mut program = build_program(&script);
    program.execute().expect("assets/main.mr runs");

    let built = engine_sim_script::build::build(&mut program)
        .expect("the engine builds")
        .expect("the script builds an engine");

    assert_eq!(built.impulse_responses.len(), built.engine.exhausts.len());

    let response = built.impulse_responses[0]
        .as_ref()
        .expect("the exhaust names an impulse response");

    assert!(response.filename.ends_with("new/minimal_muffling_02.wav"));
    assert_eq!(response.volume, 0.01);

    let wave = engine_sim_script::wav::read(&response.filename).expect("the file decodes");

    assert_eq!(wave.channels, 1);
    assert_eq!(wave.sample_rate, 44100);
    assert_eq!(wave.bits_per_sample, 16);
    assert_eq!(wave.frames, 17555);
    assert_eq!(wave.samples.len(), 17555);
}

#[test]
fn a_script_without_an_engine_has_no_header() {
    let script = parity_dir().join("execute/groups.mr");
    let mut program = build_program(&script);
    program.execute().expect("the fixture runs");

    assert!(engine_dump::dump(&mut program).is_empty());
}

/// Every engine input lands in the field it names, not in a neighbour: the
/// header this fixture produces has a different value in every numeric slot.
///
/// It is a port-only test. `tests/engine_fixtures/minimal.mr` builds an engine
/// with no crankshaft, banks, fuel or ignition module, and `buildEngine`
/// dereferences all four unconditionally, so the original has no answer to
/// compare against -- it segfaults. What the parity gate covers is
/// `assets/main.mr`, which does build all of them; what this covers is the
/// wiring the header's defaults would otherwise hide, since `assets/main.mr`
/// leaves the dyno speeds and every `initial_*` value at its class default.
#[test]
fn every_engine_input_reaches_the_field_it_names() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/engine_fixtures/minimal.mr");
    let mut program = build_program(&script);
    program.execute().expect("the fixture runs");

    let lines: Vec<String> = engine_dump::dump(&mut program);

    let expected = [
        "E,name,Tiny Single",
        "E,cylinder_banks,0",
        "E,cylinder_count,0",
        "E,crankshaft_count,0",
        "E,exhaust_system_count,0",
        "E,intake_count,0",
        "E,displacement,0000000000000000",
        "E,redline,40905c5f0ee4496b",
        "E,starter_torque,403045123bca641a",
        "E,starter_speed,400c7bd89227e044",
        "E,dyno_min_speed,4060272c76a62497",
        "E,dyno_max_speed,408294ca4758034c",
        "E,dyno_hold_step,40273f6ee0b35e92",
        "E,simulation_frequency,40b0e10000000000",
        "E,initial_noise,3fe0000000000000",
        "E,initial_jitter,3fc0000000000000",
        "E,initial_high_frequency_gain,3fd0000000000000",
        // `es/objects/objects.mr`'s `engine` class routes its `fuel` input through
        // `engine_parameters()`, which defaults to a real fuel, so the engine has
        // one even though the fixture never mentions it.
        "E,fuel_molecular_mass,3fb999999999999a",
        "E,fuel_energy_density,4186ef9500000000",
        "E,fuel_density,408797fffffffffe",
        "E,fuel_molecular_afr,4029000000000000",
        "E,fuel_max_burning_efficiency,3fe999999999999a",
        "E,fuel_burning_efficiency_randomness,3fe0000000000000",
        "E,fuel_low_efficiency_attenuation,3fe3333333333333",
        "E,fuel_max_turbulence_effect,4000000000000000",
        "E,fuel_max_dilution_effect,4024000000000000",
    ];

    assert_eq!(&lines[..expected.len()], &expected[..]);
    assert_eq!(lines.len(), expected.len());
}
