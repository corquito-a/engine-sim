//! Parses every `.mr` file passed on the command line and reports failures.
//!
//! ```text
//! corpus_check <file.mr>...
//! ```
//!
//! Files are parsed in isolation (imports are not followed), which is what the
//! parity gate wants: every script in the repository has to be syntactically
//! acceptable to the front end on its own. Exit status is the number of files
//! that failed to parse, capped at 100.

use engine_sim_script::parser::parse_unit;

use std::path::Path;

fn collect(path: &Path, out: &mut Vec<String>) {
    if path.is_dir() {
        let mut entries: Vec<_> = match std::fs::read_dir(path) {
            Ok(entries) => entries.filter_map(|entry| entry.ok()).map(|e| e.path()).collect(),
            Err(_) => return,
        };
        entries.sort();

        for entry in entries {
            collect(&entry, out);
        }
    }
    else if path.extension().and_then(|e| e.to_str()) == Some("mr") {
        out.push(path.to_string_lossy().into_owned());
    }
}

fn main() {
    let mut files = Vec::new();
    for argument in std::env::args().skip(1) {
        collect(Path::new(&argument), &mut files);
    }

    if files.is_empty() {
        eprintln!("usage: corpus_check <file-or-directory>...");
        std::process::exit(2);
    }

    let mut failures = 0;
    let mut total_lines = 0;

    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) => {
                println!("FAIL {} read error: {}", file, error);
                failures += 1;
                continue;
            }
        };

        total_lines += source.lines().count();

        match parse_unit(file, &source) {
            Ok(_) => println!("OK {}", file),
            Err(error) => {
                println!("FAIL {} {}:{}: {}", file, error.line, error.col, error.message);
                failures += 1;
            }
        }
    }

    eprintln!("{} files, {} lines, {} failures", files.len(), total_lines, failures);
    std::process::exit(failures.min(100) as i32);
}
