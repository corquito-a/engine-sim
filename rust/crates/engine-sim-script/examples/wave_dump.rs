//! The port's half of the `WAVE` gate: one line per file, describing what the
//! decoder made of it.
//!
//! `script_reference --wave <file>` prints the same record from
//! `ysWindowsAudioWaveFile`'s parse, so a difference is either a header field the
//! two disagree on or -- the hash -- a single sample.
//!
//! Format: `W,<path>,<channels>,<sample rate>,<bits per sample>,<frames>,<fnv64>`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("usage: wave_dump <file.wav>");
            return ExitCode::from(2);
        }
    };

    match engine_sim_script::wav::read(&path) {
        Ok(wave) => {
            println!(
                "W,{},{},{},{},{},{:016x}",
                path,
                wave.channels,
                wave.sample_rate,
                wave.bits_per_sample,
                wave.frames,
                engine_sim_script::wav::hash(&wave)
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("could not read {}: {}", path, message);
            ExitCode::FAILURE
        }
    }
}
