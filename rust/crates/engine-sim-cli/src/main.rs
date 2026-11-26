//! Runs an Engine Simulator `.mr` script headlessly: build the engine the script
//! configures, simulate it, and write what it did.
//!
//! This is the application's job with the window removed. `EngineSimApplication`
//! reads a script, builds a `Simulator` around the engine, and then, once per
//! rendered frame, asks the simulation to run the frame, pulls the audio the
//! synthesizer produced and paints the gauges. The CLI does the same steps with
//! the same calls and writes the audio to a wave file and the gauges' numbers to
//! a CSV.
//!
//! Everything underneath is the port's verified path: the front end that matches
//! piranha's IR byte for byte, `buildEngine`, and the `engine-sim-core` simulation
//! whose physics and audio are bit-identical to the C++ original (see
//! `rust/tools/parity/run_script_parity.sh`).
//!
//! Four things are deliberate:
//!
//! * **The audio is always rendered**, even without `--wav`. The synthesizer and
//!   the combustion chamber share one random sequence in the original, so
//!   skipping the audio would change the physics; the reference harness renders
//!   it for the same reason.
//! * **The starter is released after `--starter` seconds**, because its constraint
//!   drives the crankshaft at the starter's speed and would hold it there forever.
//!   The application only spins it while `S` is down; this is that key press and
//!   its release, not a permanently cranked engine.
//! * **The ignition is on by default.** `IgnitionModule` is built disabled -- the
//!   application's `A` key is what enables it -- so a script run without it builds
//!   an engine that compresses and never fires. `--no-ignition` is the old shape.
//! * **The script's own vehicle and transmission are what the car is**, with the
//!   application's 1597 kg / six-speed pair only where the script published none
//!   (`EngineSimApplication::loadScript`). Every engine under `assets/engines/`
//!   publishes both.
//!
//! The driveline's clutch is engaged (`--clutch`, default 1) because that is the
//! application's resting state, but the transmission starts in neutral, so
//! `--gear` is what actually couples the engine to the car -- and in neutral the
//! clutch's limits are zero anyway.
//!
//! A script that only *defines* an engine -- most of `assets/engines/` -- needs a
//! two-line wrapper before it can run, exactly as the parity harness uses:
//!
//! ```text
//! import "engines/atg-video-2/03_2jz.mr"
//! main()
//! ```

use std::process::ExitCode;

use engine_sim_core::simulator::Simulator;
use engine_sim_core::units;

use engine_sim_script::{build, resolve};

mod wav;

const USAGE: &str = "\
usage: engine-sim-cli <script.mr> [options]

  --seconds <s>      engine time to simulate            (default 5)
  --fps <n>          application frames per second      (default 60)
  --wav <path>       write the rendered audio: 44.1 kHz, 16-bit, mono
  --csv <path>       write one row of telemetry per frame
  --throttle <0..1>  throttle, 0 closed, 1 wide open      (default 0)
  --dyno <rpm>       hold the engine at <rpm> on the dynamometer
  --gear <n>         transmission gear, 1-based, 0 for neutral
  --shift <s>        select the gear after <s> seconds       (default 0)
  --clutch <0..1>    clutch pressure                        (default 1)
  --starter <s>      hold the starter for <s> seconds   (default 1)
  --no-starter       do not spin the starter at all
  --no-ignition      leave the ignition disabled (the engine will not fire)
  --search <dir>     add an import search path          (default: ../../es/, ../es/, es/)
  --quiet            the summary only

The starter is released after --starter seconds and the ignition is enabled, so
the engine runs on its own; --no-starter and --no-ignition turn those off.

Writes nothing unless --wav or --csv is given; the audio is rendered either way,
because the synthesizer draws from the same random sequence as combustion.

The dynamometer's torque is the constraint's reaction, so it is negative while the
engine drives it: `--dyno 3000` reports the engine's output as -torque.
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {}", message);
            ExitCode::FAILURE
        }
    }
}

/// What the command line asked for.
struct Options {
    script: String,
    seconds: f64,
    fps: f64,
    wav: Option<String>,
    csv: Option<String>,
    throttle: f64,
    dyno: Option<f64>,
    gear: Option<usize>,
    shift_seconds: f64,
    clutch: f64,
    starter_seconds: f64,
    ignition: bool,
    search: Vec<String>,
    quiet: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            script: String::new(),
            seconds: 5.0,
            fps: 60.0,
            wav: None,
            csv: None,
            throttle: 0.0,
            dyno: None,
            gear: None,
            shift_seconds: 0.0,
            clutch: 1.0,
            starter_seconds: 1.0,
            ignition: true,
            search: vec![
                "../../es/".to_string(),
                "../es/".to_string(),
                "es/".to_string(),
            ],
            quiet: false,
        }
    }
}

impl Options {
    fn parse(arguments: Vec<String>) -> Result<Self, String> {
        let mut options = Options::default();
        let mut arguments = arguments.into_iter();

        while let Some(argument) = arguments.next() {
            // A value is either the next argument or what follows `=` in this one.
            let (flag, inline) = match argument.split_once('=') {
                Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
                None => (argument.clone(), None),
            };

            let mut value = || -> Result<String, String> {
                match inline.clone() {
                    Some(value) => Ok(value),
                    None => arguments
                        .next()
                        .ok_or_else(|| format!("{} needs a value", flag)),
                }
            };

            match flag.as_str() {
                "--help" | "-h" => {
                    print(USAGE);
                    std::process::exit(0);
                }
                "--seconds" => options.seconds = number(&value()?, "--seconds")?,
                "--fps" => options.fps = number(&value()?, "--fps")?,
                "--wav" => options.wav = Some(value()?),
                "--csv" => options.csv = Some(value()?),
                "--throttle" => options.throttle = number(&value()?, "--throttle")?,
                "--dyno" => options.dyno = Some(number(&value()?, "--dyno")?),
                "--gear" => {
                    let gear = number(&value()?, "--gear")?;

                    if gear < 0.0 || gear.fract() != 0.0 {
                        return Err(format!("--gear is a whole number, not `{}`", gear));
                    }

                    options.gear = Some(gear as usize);
                }
                "--clutch" => options.clutch = number(&value()?, "--clutch")?,
                "--shift" => options.shift_seconds = number(&value()?, "--shift")?,
                "--starter" => options.starter_seconds = number(&value()?, "--starter")?,
                "--no-starter" => options.starter_seconds = 0.0,
                "--no-ignition" => options.ignition = false,
                "--quiet" => options.quiet = true,
                "--search" => {
                    if options.search == Options::default().search {
                        options.search.clear();
                    }
                    options.search.push(value()?);
                }
                other => {
                    if other.starts_with('-') {
                        return Err(format!("unknown option `{}`\n\n{}", other, USAGE));
                    }

                    if !options.script.is_empty() {
                        return Err(format!("two scripts given: `{}` and `{}`", options.script, other));
                    }

                    options.script = other.to_string();
                }
            }
        }

        if options.script.is_empty() {
            return Err(format!("no script given\n\n{}", USAGE));
        }

        if !(options.seconds > 0.0) {
            return Err("--seconds must be positive".to_string());
        }

        if !(options.fps >= 1.0) {
            return Err("--fps must be at least 1".to_string());
        }

        if !(0.0..=1.0).contains(&options.throttle) {
            return Err(format!("--throttle is a position from 0 to 1, not {}", options.throttle));
        }

        if !(0.0..=1.0).contains(&options.clutch) {
            return Err(format!("--clutch is a pressure from 0 to 1, not {}", options.clutch));
        }

        if options.shift_seconds < 0.0 {
            return Err("--shift cannot be negative".to_string());
        }

        if options.starter_seconds < 0.0 {
            return Err("--starter cannot be negative".to_string());
        }

        if let Some(rpm) = options.dyno {
            if !(rpm > 0.0) {
                return Err("--dyno wants a speed above zero".to_string());
            }
        }

        Ok(options)
    }
}

/// Writes to stdout without caring whether anything is reading: a closed pipe is
/// not an error for a tool whose output can be long.
fn print(text: &str) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(text.as_bytes());
}

fn number(text: &str, flag: &str) -> Result<f64, String> {
    text.parse()
        .map_err(|_| format!("{} wants a number, not `{}`", flag, text))
}

fn run(arguments: Vec<String>) -> Result<(), String> {
    let options = Options::parse(arguments)?;

    let search: Vec<&str> = options.search.iter().map(|path| path.as_str()).collect();
    let mut program = resolve::program(&options.script, &search)
        .map_err(|error| format!("{}", error))?;

    program
        .execute()
        .map_err(|message| format!("the script failed: {}", message))?;

    let mut simulator = build::simulator(&mut program)?;

    // The starter is a velocity constraint: while it is enabled it holds the
    // crankshaft at its own speed, which is why the application only spins it
    // while `S` is down. A run that held it would sit at the starter's speed
    // forever, so it is released after `--starter` seconds, as a user would.
    simulator.set_starter_enabled(options.starter_seconds > 0.0);

    // The ignition module ships disabled -- the application enables it with `A`
    // (which is why its on-screen log says "IGNITION ENABLED" the first time the
    // key is pressed) -- so a run that wants a running engine must turn it on.
    simulator.engine.ignition.enabled = options.ignition;

    if let Some(rpm) = options.dyno {
        simulator.set_dyno_enabled(true);
        simulator.set_dyno_hold(true);
        simulator.set_dyno_rotation_speed(units::rpm(rpm));
    }

    if let Some(gear) = options.gear {
        let gears = simulator.transmission.lock().unwrap().gear_ratios.len();

        if gear > gears {
            return Err(format!("--gear is 0 to {}, not {}", gears, gear));
        }
    }

    // The application sets the clutch pressure every frame, ramping to one within
    // a millisecond unless the clutch key is held, so an engaged clutch is its
    // resting state; the transmission itself starts at zero. In neutral the clutch
    // limits are zero anyway, which is why `--gear` is what makes it matter.
    simulator.set_clutch_pressure(options.clutch);

    // The application drives the throttle through its *speed control*: the keys
    // set a speed setting and the engine's linkage turns that into a plate
    // position (`position = 1 - control^gamma`), so a control of one is wide open
    // -- the same sense `--throttle` has, and the reason a run reproduces the
    // application's user by setting the control alone. The plate is set as well,
    // the complement of the control, because a script can wire a plain throttle
    // that ignores the control, and then the position is the only channel.
    simulator.engine.set_speed_control(options.throttle);
    simulator.engine.set_throttle(1.0 - options.throttle);

    let run = simulate(&mut simulator, &options);

    if let Some(path) = &options.wav {
        wav::write(path, &run.audio, 44100)?;
    }

    if let Some(path) = &options.csv {
        let mut csv = String::from(
            "frame,time_s,rpm,vehicle_kmh,crank_rad_s,throttle,dyno_torque_nm,dyno_power_w,exhaust_pa,chamber0_pa,audio_samples\n",
        );

        for row in &run.rows {
            csv.push_str(&format!(
                "{},{:.6},{:.3},{:.3},{:.6},{:.4},{:.4},{:.3},{:.3},{:.3},{}\n",
                row.frame,
                row.time,
                row.rpm,
                row.vehicle_speed,
                row.crank_speed,
                row.throttle,
                row.torque,
                row.power,
                row.exhaust_pressure,
                row.chamber_pressure,
                row.audio_samples,
            ));
        }

        std::fs::write(path, csv).map_err(|error| format!("could not write {}: {}", path, error))?;
    }

    if !options.quiet {
        let mut text = summary(&simulator, &options, &run);

        if let Some(path) = &options.wav {
            text.push_str(&format!("  wrote   {} ({} samples)\n", path, run.audio.len()));
        }

        if let Some(path) = &options.csv {
            text.push_str(&format!("  wrote   {} ({} rows)\n", path, run.rows.len()));
        }

        print(&text);
    }

    Ok(())
}

/// One frame's worth of the numbers the application puts on its gauges.
struct Row {
    frame: usize,
    time: f64,
    rpm: f64,
    vehicle_speed: f64,
    crank_speed: f64,
    throttle: f64,
    torque: f64,
    power: f64,
    exhaust_pressure: f64,
    chamber_pressure: f64,
    audio_samples: usize,
}

struct Run {
    audio: Vec<i16>,
    rows: Vec<Row>,
    engine_time: f64,
    steps: usize,
}

/// The application's frame loop: run the frame, end it, render what the
/// synthesizer made, and read it back.
///
/// The loop stops at the first frame that has produced `--seconds` of audio
/// rather than at a frame count, because `Simulator::startFrame` decides how many
/// simulation steps a frame gets -- it rounds `dt` to the simulation frequency and
/// then scales it by 1.1 or 0.9 to steer the synthesizer's latency, so a frame is
/// not exactly `1/fps` seconds of engine time. Asking for the duration in audio
/// is the same as asking for it in engine time, and it is the number the wave file
/// ends up with.
fn simulate(simulator: &mut Simulator, options: &Options) -> Run {
    let frame_time = 1.0 / options.fps;
    let target_samples = (options.seconds * 44100.0).round() as usize;

    // A frame that renders nothing would otherwise spin: the guard is generous
    // (four times the frames the duration should need) but finite.
    let max_frames = ((options.seconds * options.fps) * 4.0) as usize + 16;

    let mut run = Run {
        audio: Vec::new(),
        rows: Vec::new(),
        engine_time: 0.0,
        steps: 0,
    };

    let mut buffer = [0i16; 8192];
    let mut frame = 0;
    let mut starting = options.starter_seconds > 0.0;

    // `--shift 0` engages before the first frame; a later shift waits for the
    // engine's own clock, which is not the frame count.
    let mut gear = if options.shift_seconds <= 0.0 {
        options.gear
    } else {
        None
    };

    while run.audio.len() < target_samples {
        if frame >= max_frames {
            break;
        }

        if starting && run.engine_time >= options.starter_seconds {
            simulator.set_starter_enabled(false);
            starting = false;
        }

        // The gear is selected once, when the driver says so. A gear engaged from
        // the first frame leaves the starter turning the whole car, which is why
        // the application's user starts in neutral and shifts after the engine is
        // running; `--shift` is that wait.
        if gear.is_none() && run.engine_time >= options.shift_seconds {
            gear = options.gear;
        }

        if let Some(gear) = gear.take() {
            // `change_gear(-1)` is neutral; the command line counts gears from one.
            simulator.change_gear(gear as i32 - 1);
        }

        simulator.start_frame(frame_time);

        let steps = simulator.simulation_steps();
        while simulator.simulate_step() {}
        simulator.end_frame();

        run.steps += steps;
        run.engine_time += steps as f64 * simulator.get_timestep();

        simulator.render_audio_block();
        let produced = simulator.read_audio_output(&mut buffer);
        run.audio.extend_from_slice(&buffer[..produced]);

        run.rows.push(Row {
            frame,
            time: run.engine_time,
            rpm: simulator.engine.get_rpm(&simulator.solver.bodies),
            vehicle_speed: simulator.vehicle.get_speed(&simulator.solver.bodies) * 3.6,
            crank_speed: simulator.engine.get_speed(&simulator.solver.bodies),
            throttle: simulator.engine.get_throttle(),
            torque: simulator.get_filtered_dyno_torque(),
            power: simulator.get_dyno_power(),
            exhaust_pressure: simulator
                .engine
                .exhausts
                .first()
                .map(|exhaust| exhaust.system.pressure())
                .unwrap_or(0.0),
            chamber_pressure: simulator
                .engine
                .chambers
                .first()
                .map(|chamber| chamber.system.pressure())
                .unwrap_or(0.0),
            audio_samples: produced,
        });

        frame += 1;
    }

    run
}

fn summary(simulator: &Simulator, options: &Options, run: &Run) -> String {
    let engine = &simulator.engine;

    let rpm_min = run.rows.iter().map(|row| row.rpm).fold(f64::MAX, f64::min);
    let rpm_max = run.rows.iter().map(|row| row.rpm).fold(0.0, f64::max);

    // The dynamometer's torque is the reaction on the crankshaft, so the engine's
    // own output is its negation. The first frames are skipped: the filter fills
    // from zero and the very first constraint impulse is enormous.
    let settled = (run.rows.len() / 4).min(8);
    let output_torque = |row: &Row| -row.torque;
    let torque = run.rows[settled..]
        .iter()
        .map(output_torque)
        .fold(0.0, f64::max);
    let power = run.rows[settled..]
        .iter()
        .map(|row| -row.power)
        .fold(0.0, f64::max);

    let mut text = String::new();

    text.push_str(&format!("engine    {}\n", engine.name));
    text.push_str(&format!(
        "layout    {} banks, {} cylinders, {} exhaust system(s), {} intake(s)\n",
        engine.banks.len(),
        engine.pistons.len(),
        engine.exhausts.len(),
        engine.intakes.len(),
    ));
    text.push_str(&format!(
        "frames    {} frames, {} simulation steps ({:.2} s of engine time at {:.0} fps)\n",
        run.rows.len(),
        run.steps,
        run.engine_time,
        options.fps,
    ));
    text.push_str(&format!(
        "rpm       {:.0} to {:.0} (final {:.0})\n",
        if rpm_min.is_finite() { rpm_min } else { 0.0 },
        rpm_max,
        run.rows.last().map(|row| row.rpm).unwrap_or(0.0),
    ));

    if let Some(rpm) = options.dyno {
        text.push_str(&format!(
            "dyno      holding {:.0} rpm: peak {:.1} Nm ({:.1} kW), \
final {:.1} Nm ({:.1} kW)\n",
            rpm,
            torque,
            power / units::KW,
            run.rows.last().map(output_torque).unwrap_or(0.0),
            run.rows.last().map(|row| -row.power).unwrap_or(0.0) / units::KW,
        ));
    } else {
        text.push_str("dyno      not engaged (--dyno <rpm> holds the engine and fills torque/power)\n");
    }

    text.push_str(&format!(
        "audio     {} samples ({:.2} s at 44.1 kHz)\n",
        run.audio.len(),
        run.audio.len() as f64 / 44100.0,
    ));

    text
}
