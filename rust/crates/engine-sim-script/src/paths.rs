//! The two path rules a script can reach.
//!
//! `ImpulseResponseNode::generate` is the only `es_script` class that touches the
//! filesystem: an `impulse_response(filename: "smooth/smooth_39.wav")` is
//! resolved against the directory of the *unit that wrote it*, so a library node
//! and a node in `assets/` mean the same file. `piranha::Path` is a thin wrapper
//! over `boost::filesystem::path`, so the resolution is `parent_path() / filename`
//! -- a lexical join, with no normalisation of `.` or `..`.
//!
//! [`display`] is not the original's: the object holds an absolute path, and a
//! dump that printed it would pin the checkout's directory into a fixture. Every
//! dump prints the path relative to the working directory instead, which is the
//! repository root for every harness here.

/// `parent_path() / filename`, unless the filename is already absolute.
///
/// `unit_path` is the unit the node's definition was written in
/// (`IrParserStructure::getParentUnit()->getPath()`); the engine's own units are
/// absolute, since the loader hands the parser resolved paths.
pub fn resolve(unit_path: Option<&str>, filename: &str) -> String {
    if filename.starts_with('/') {
        return filename.to_string();
    }

    match unit_path.map(parent) {
        Some(directory) if !directory.is_empty() => join(&directory, filename),
        _ => filename.to_string(),
    }
}

/// `boost::filesystem::path::parent_path`, which is `""` for a bare filename.
fn parent(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_string(),
        Some(index) => path[..index].to_string(),
        None => String::new(),
    }
}

/// `boost::filesystem::path::operator/`: one separator between the two, unless
/// the left side already ends in one.
fn join(directory: &str, filename: &str) -> String {
    if directory.ends_with('/') {
        format!("{}{}", directory, filename)
    } else {
        format!("{}/{}", directory, filename)
    }
}

/// A path as a dump prints it: relative to the deepest directory containing the
/// working directory that also contains it, unchanged otherwise.
///
/// The engine's units are absolute, so the object's filename is too. Printing it
/// would pin the checkout's directory into a fixture without telling the gate
/// anything, so every dump prints the repository-relative form instead -- and the
/// walk up from the working directory is what makes that the same string whether
/// the harness runs from the repository root or from a crate inside it.
pub fn display(path: &str) -> String {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(_) => return path.to_string(),
    };

    // Deepest first: the crate directory is a better base than the repository
    // when a file happens to live under both.
    for ancestor in cwd.ancestors() {
        let Some(ancestor) = ancestor.to_str() else {
            continue;
        };

        if ancestor.is_empty() {
            continue;
        }

        let prefix = format!("{}/", ancestor);
        if let Some(rest) = path.strip_prefix(&prefix) {
            return rest.to_string();
        }
    }

    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_filename_is_resolved_against_the_defining_unit() {
        assert_eq!(
            resolve(Some("es/sound-library/impulse_responses.mr"), "smooth/smooth_39.wav"),
            "es/sound-library/smooth/smooth_39.wav"
        );
    }

    #[test]
    fn an_absolute_filename_is_left_alone() {
        assert_eq!(
            resolve(Some("es/sound-library/impulse_responses.mr"), "/tmp/x.wav"),
            "/tmp/x.wav"
        );
    }

    #[test]
    fn a_unit_at_the_top_of_the_tree_contributes_nothing() {
        assert_eq!(resolve(Some("main.mr"), "x.wav"), "x.wav");
        assert_eq!(resolve(None, "x.wav"), "x.wav");
    }

    #[test]
    fn the_join_does_not_double_a_separator() {
        assert_eq!(parent("/es/a.mr"), "/es");
        assert_eq!(join("/es", "a.wav"), "/es/a.wav");
        assert_eq!(join("/es/", "a.wav"), "/es/a.wav");
    }

    #[test]
    fn display_strips_the_working_directory() {
        let cwd = std::env::current_dir().unwrap();
        let full = cwd.join("es/sound-library/smooth/smooth_39.wav");
        assert_eq!(
            display(full.to_str().unwrap()),
            "es/sound-library/smooth/smooth_39.wav"
        );
        assert_eq!(display("/elsewhere/x.wav"), "/elsewhere/x.wav");
    }

    #[test]
    fn display_also_strips_an_ancestor_of_the_working_directory() {
        // The tests run from a crate directory, three levels below the
        // repository root; the dumps have to print what the gate prints.
        let cwd = std::env::current_dir().unwrap();
        let root = cwd
            .ancestors()
            .find(|dir| dir.join("assets/main.mr").is_file())
            .unwrap();

        let full = root.join("es/sound-library/new/mild_exhaust.wav");
        assert_eq!(
            display(full.to_str().unwrap()),
            "es/sound-library/new/mild_exhaust.wav"
        );
    }
}
