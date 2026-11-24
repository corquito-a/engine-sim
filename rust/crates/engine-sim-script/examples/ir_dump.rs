//! Prints the IR of a script and everything it imports as S-expressions.
//!
//! ```text
//! ir_dump <root.mr> [search-path...]
//! ```
//!
//! The default search paths are the ones `es_script::Compiler::initialize`
//! installs, so running from the repository root mirrors the application.

use engine_sim_script::{dump_units_with, resolve};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: ir_dump <root.mr> [search-path...]");
        std::process::exit(2);
    }

    // `--lines` carries token line numbers; the conformance oracle needs them,
    // the IR round-trip grammar does not.
    let with_lines = args.iter().any(|arg| arg == "--lines");
    let args: Vec<&String> = args.iter().filter(|arg| *arg != "--lines").collect();

    if args.is_empty() {
        eprintln!("usage: ir_dump [--lines] <root.mr> [search-path...]");
        std::process::exit(2);
    }

    let defaults = ["../../es/", "../es/", "es/"];
    let search: Vec<&str> = if args.len() > 1 {
        args[1..].iter().map(|entry| entry.as_str()).collect()
    }
    else {
        defaults.to_vec()
    };

    match resolve::load(&args[0], &search) {
        Ok(units) => print!("{}", dump_units_with(&units, with_lines)),
        Err(error) => {
            eprintln!("error: {}", error);
            std::process::exit(1);
        }
    }
}
