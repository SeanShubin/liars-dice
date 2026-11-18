//! The relay knows no game rules, enforced by the build rather than by discipline.
//!
//! /specification/architecture.md states it; this is what holds the code to it. If a
//! change here needs `rules`, the design has gone wrong — stop and re-read
//! /implementation/relay.md rather than adding the dependency.

use std::path::Path;

#[test]
fn the_relay_does_not_depend_on_the_rules() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("the relay's own manifest");
    assert!(
        !manifest.contains("rules"),
        "the relay must not depend on `rules`: it would then be able to decide whether a \
         claim was true, which is the one thing it may never know.\n{manifest}"
    );
}

/// The two things the relay is allowed to know about: the message format, and how dice
/// are committed to and derived. Neither says what a claim means.
#[test]
fn the_relay_depends_only_on_the_format_and_the_dice() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("the relay's own manifest");
    let dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a dependencies section");
    for line in dependencies.lines().filter(|line| line.contains('=')) {
        let name = line.split('.').next().unwrap_or("").trim();
        assert!(
            matches!(name, "protocol" | "integrity"),
            "unexpected dependency {name}: adding one here widens what the relay can know"
        );
    }
}
