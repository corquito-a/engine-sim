//! Parses the scripts that ship with the application.
//!
//! The Rust crate lives inside the engine-sim checkout, so the standard
//! library (`es/`) and the example assets (`assets/`) are right there. The test
//! is skipped when they are not, which keeps the crate usable on its own.

use engine_sim_script::parser::parse_unit;

use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(path) {
            Ok(entries) => entries.filter_map(|entry| entry.ok()).map(|e| e.path()).collect(),
            Err(_) => return,
        };
        entries.sort();

        for entry in entries {
            collect(&entry, out);
        }
    }
    else if path.extension().and_then(|extension| extension.to_str()) == Some("mr") {
        out.push(path.to_path_buf());
    }
}

fn parse_corpus(directory: &str) -> Option<(usize, usize)> {
    let root = repository_root().join(directory);
    if !root.is_dir() {
        return None;
    }

    let mut files = Vec::new();
    collect(&root, &mut files);

    let mut lines = 0;
    for file in &files {
        let source = std::fs::read_to_string(file).expect("readable script");
        lines += source.lines().count();

        if let Err(error) = parse_unit(&file.to_string_lossy(), &source) {
            panic!("{}: {}", file.display(), error);
        }
    }

    Some((files.len(), lines))
}

#[test]
fn standard_library_parses() {
    match parse_corpus("es") {
        Some((files, lines)) => {
            assert_eq!(files, 18, "the standard library has 18 units");
            assert!(lines > 1000);
        }
        None => eprintln!("skipping: es/ is not next to the crate"),
    }
}

#[test]
fn example_assets_parse() {
    match parse_corpus("assets") {
        Some((files, lines)) => {
            assert_eq!(files, 40, "assets/ holds 40 scripts");
            assert!(lines > 5000);
        }
        None => eprintln!("skipping: assets/ is not next to the crate"),
    }
}

#[test]
fn the_main_script_resolves_its_imports() {
    let root = repository_root();
    if !root.join("assets/main.mr").is_file() {
        eprintln!("skipping: assets/main.mr is not next to the crate");
        return;
    }

    // The application installs CWD-relative search paths (`../../es/`,
    // `../es/`, `es/`); the test spells the same directory absolutely so that
    // it does not depend on where cargo runs from.
    let library = root.join("es/");
    let units = engine_sim_script::resolve::load(
        &root.join("assets/main.mr").to_string_lossy(),
        &[&library.to_string_lossy()],
    )
    .expect("assets/main.mr resolves");

    // `assets/main.mr` pulls in the whole standard library, the default theme
    // and one engine.
    assert!(units.len() > 15, "expected the full import graph, got {}", units.len());
    assert!(units.iter().any(|unit| unit.path.ends_with("es/engine_sim.mr")));
    assert!(units.iter().any(|unit| unit.path.ends_with("themes/default.mr")));
}
