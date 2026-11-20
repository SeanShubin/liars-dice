//! `Only a composition root observes anything outside its program`, enforced by the build.
//!
//! /specification/architecture.md states it and /implementation/composition.md explains
//! why it is the rule that every other Determinism section is answering. This is what
//! holds the code to it.
//!
//! Here rather than in any one crate because it is a fact about the whole library layer,
//! and `protocol` is the floor every other library stands on. It scans its siblings, which
//! is unusual for a test and is the point: the assertion belongs to no single crate, so
//! putting it in one of them is better than putting it in none.
//!
//! If this fails, the fix is almost never to add a pattern to the list. It is to take the
//! observation out of the library and hand the value in from `apps/*` — see *A value where
//! you can, a trait where you must* in the document.

use std::path::{Path, PathBuf};

/// Ways to observe something outside the program, and what to do instead.
///
/// Not an exhaustive list of everything a crate could reach for — nothing short of a
/// sandbox is — but every way this system has ever actually wanted to, which makes it a
/// tripwire on the mistake rather than a proof of its absence.
const OBSERVATIONS: [(&str, &str); 12] = [
    ("SystemTime::now", "take a `Moment` as a parameter"),
    ("Instant::now", "take the elapsed time as a parameter"),
    ("js_sys::Date", "take a `Moment` as a parameter"),
    ("Date::now", "take a `Moment` as a parameter"),
    ("thread_rng", "derive from a seed, or take the value"),
    ("rand::", "derive from a seed, or take the value"),
    ("getRandomValues", "take the bytes as a parameter"),
    ("std::env::var", "take the configuration as a parameter"),
    ("env::var", "take the configuration as a parameter"),
    (
        "std::fs::",
        "take the contents, or put the I/O behind a trait",
    ),
    (
        "TcpStream",
        "put the transport behind a trait, as `Wire` does",
    ),
    ("local_storage", "take the stored value as a parameter"),
];

/// Every `.rs` file under `crates/*/src`.
fn library_sources() -> Vec<PathBuf> {
    // `crates/protocol` -> `crates` -> the workspace root.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root")
        .join("crates");
    let mut found = Vec::new();
    let mut stack: Vec<PathBuf> = std::fs::read_dir(&root)
        .expect("the crates directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("src"))
        .filter(|path| path.is_dir())
        .collect();
    assert!(!stack.is_empty(), "no library crates found under {root:?}");
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)
            .expect("a source directory")
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|kind| kind == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// A library crate reads nothing from outside the program.
///
/// `src` only, deliberately. A test may read its own golden file: that is the test
/// observing, not the program, and three of them do.
#[test]
fn no_library_crate_observes_the_outside_world() {
    let sources = library_sources();
    assert!(
        sources.len() > 20,
        "only {} sources scanned: the walk is not finding them",
        sources.len()
    );
    let mut wrong = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path).expect("a source file");
        for (line, number) in text.lines().zip(1..) {
            for (pattern, instead) in OBSERVATIONS {
                if line.contains(pattern) {
                    wrong.push(format!(
                        "{}:{number} observes with `{pattern}` — {instead}",
                        path.display()
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a library crate observed the outside world. Only a composition root may:\n{}",
        wrong.join("\n")
    );
}
