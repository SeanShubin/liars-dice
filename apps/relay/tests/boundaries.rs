//! The service knows no game rules either, enforced by the build rather than by discipline.
//!
//! `crates/relay/tests/boundaries.rs` holds the library to this. The same test lives here
//! because a composition root is exactly where someone would reach for `rules` to answer
//! a question the relay is not supposed to be asking — "is that claim true" is one
//! `use` away from any handler, and the only thing stopping it is this.

use std::path::Path;

fn manifest() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("the service's own manifest")
}

#[test]
fn the_relay_service_does_not_depend_on_the_rules() {
    let manifest = manifest();
    assert!(
        !manifest.contains("rules"),
        "the relay service must not depend on `rules`: deploying the relay must not widen \
         what the relay can know.\n{manifest}"
    );
}

/// The library it serves, the format, and the SDK it is deployed on. Nothing else.
///
/// Narrower than the library's list on purpose: that one allows `protocol` and
/// `integrity`, and this one has to allow the AWS SDK as well, so it names what it allows
/// rather than trusting that nobody adds a game crate next to a transport crate.
#[test]
fn the_relay_service_depends_only_on_storage_and_the_format() {
    let manifest = manifest();
    let dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a dependencies section");
    for line in dependencies.lines().filter(|line| line.contains('=')) {
        let name = line.split('.').next().unwrap_or("").trim();
        assert!(
            matches!(
                name,
                "relay"
                    | "wire"
                    | "protocol"
                    | "integrity"
                    | "aws-config"
                    | "aws-sdk-dynamodb"
                    | "tokio"
            ),
            "unexpected dependency {name}: adding one here widens what the deployed relay \
             can know, which is the thing the library's own boundary test exists to prevent"
        );
    }
}
