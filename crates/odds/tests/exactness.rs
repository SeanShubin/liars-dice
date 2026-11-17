//! The boundary /specification/architecture.md states, checked rather than remembered.
//!
//! This crate decides the content of events, so no value in it may be a rounded one. The
//! dependency graph already stops it reaching `crates/readout`, but nothing stops it
//! writing a floating point number of its own, so that is what this looks for. A grep over
//! the crate's own sources is a blunt check, and being blunt is the point: it cannot be
//! satisfied by a plausible-looking exception.

use std::fs;
use std::path::{Path, PathBuf};

fn sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("a readable source directory") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|kind| kind == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn the_probability_model_names_no_floating_point_type() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(
        !files.is_empty(),
        "found no sources to check under {root:?}"
    );

    for path in files {
        let text = fs::read_to_string(&path).expect("a readable source file");
        for banned in ["f32", "f64"] {
            assert!(
                !text.contains(banned),
                "{} names {banned}. A value that decides the content of an event is \
                 computed exactly; if this one only reaches a person, it belongs in \
                 crates/readout instead.",
                path.display()
            );
        }
    }
}
