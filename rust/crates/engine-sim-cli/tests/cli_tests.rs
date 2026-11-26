//! The command line's own contract: what it accepts, what it refuses, and that a
//! run writes the audio it says it wrote.
//!
//! The physics underneath has its own gates -- `run_script_parity.sh` compares it
//! with the C++ original line for line -- so nothing here re-tests the simulation.
//! These tests pin the executable: the options, the errors, the wave file's shape,
//! and the one behaviour that is easy to lose silently, that a script's engine
//! actually runs once the starter is released.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Test runs stand in the repository root, which is where the default import
/// search paths (`../../es/`, `../es/`, `es/`) and `assets/main.mr` make sense.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("the repository root")
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_engine-sim-cli"))
        .args(arguments)
        .current_dir(repo_root())
        .output()
        .expect("the binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A scratch path under the system temporary directory, unique per test.
fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("engine-sim-cli-{}-{}", std::process::id(), name))
}

/// The summary's `rpm <min> to <max> (final <final>)` line, as a number.
fn final_rpm(summary: &str) -> f64 {
    let line = summary
        .lines()
        .find(|line| line.starts_with("rpm "))
        .expect("the summary reports the rpm");

    line.rsplit_once("(final ")
        .and_then(|(_, rest)| rest.trim_end_matches(')').parse().ok())
        .expect("the rpm line ends with the final speed")
}

#[test]
fn help_says_what_the_options_are() {
    let output = run(&["--help"]);

    assert!(output.status.success(), "--help exits cleanly");
    assert!(stdout(&output).contains("usage: engine-sim-cli"));
    assert!(stdout(&output).contains("--throttle"));
    assert!(stdout(&output).contains("--seconds"));
}

#[test]
fn a_run_without_a_script_is_refused() {
    let output = run(&[]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("no script given"));
}

#[test]
fn an_unknown_option_is_refused() {
    let output = run(&["--nonsense"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("unknown option `--nonsense`"));
}

#[test]
fn a_throttle_outside_the_range_is_refused() {
    let output = run(&["assets/main.mr", "--throttle", "1.5"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("--throttle is a position from 0 to 1"));
}

#[test]
fn a_gear_the_transmission_does_not_have_is_refused() {
    // The default transmission has six gears, so 99 is out of range; the check
    // happens after the script builds an engine, which is the only way to know.
    let output = run(&["assets/main.mr", "--gear", "99", "--seconds", "0.01"]);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("--gear is 0 to 6"));
}

#[test]
fn a_run_writes_a_wave_file_of_the_length_it_simulated() {
    let wave = scratch("run.wav");
    let csv = scratch("run.csv");

    let output = run(&[
        "assets/main.mr",
        "--seconds",
        "0.2",
        "--wav",
        wave.to_str().unwrap(),
        "--csv",
        csv.to_str().unwrap(),
    ]);

    assert!(output.status.success(), "{}", stderr(&output));

    let text = stdout(&output);
    let samples: usize = text
        .lines()
        .find(|line| line.starts_with("audio "))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|count| count.parse().ok())
        .expect("the summary counts the samples");

    assert!(
        samples >= 8820 && samples <= 8820 + 441,
        "0.2 s of audio, not {} samples",
        samples
    );

    let bytes = std::fs::read(&wave).expect("the wave file was written");

    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    assert_eq!(&bytes[12..16], b"fmt ");
    assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1, "mono");
    assert_eq!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        44100
    );
    assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16, "16-bit");
    assert_eq!(
        u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize,
        samples * 2,
        "the data block is one 16-bit sample per sample"
    );
    assert_eq!(bytes.len(), 44 + samples * 2);

    let rows = std::fs::read_to_string(&csv).expect("the csv was written");
    let lines: Vec<&str> = rows.lines().collect();

    assert_eq!(
        lines[0],
        "frame,time_s,rpm,vehicle_kmh,crank_rad_s,throttle,dyno_torque_nm,dyno_power_w,\
exhaust_pa,chamber0_pa,audio_samples"
    );
    assert!(lines.len() > 2, "one row per frame");

    let _ = std::fs::remove_file(wave);
    let _ = std::fs::remove_file(csv);
}

#[test]
fn the_engine_runs_once_the_starter_is_released() {
    // The ignition module is built disabled -- the application's `A` key is what
    // turns it on -- so a run without it compresses and never fires. This is the
    // difference between a wave file of an engine and a wave file of a starter
    // motor, and it is invisible in any single frame.
    let running = run(&[
        "assets/main.mr",
        "--seconds",
        "1.5",
        "--starter",
        "0.3",
        "--throttle",
        "1",
    ]);

    assert!(running.status.success(), "{}", stderr(&running));

    // Wide open at 1.5 s, with the starter gone after 0.3 s, the engine is far
    // past the speed the starter could have given it: this is combustion.
    assert!(
        final_rpm(&stdout(&running)) > 1500.0,
        "the engine revs after the starter lets go: {}",
        stdout(&running)
    );

    let dead = run(&[
        "assets/main.mr",
        "--seconds",
        "1.5",
        "--starter",
        "0.3",
        "--throttle",
        "1",
        "--no-ignition",
    ]);

    assert!(dead.status.success(), "{}", stderr(&dead));

    // Without a spark the engine can only spin down from the starter's own speed
    // (`starter_speed`, 500 rpm on this engine), so anything near idle is ignition
    // rather than compression -- 162 rpm and falling here.
    assert!(
        final_rpm(&stdout(&dead)) < 500.0,
        "an engine with no spark never reaches idle: {}",
        stdout(&dead)
    );
}
