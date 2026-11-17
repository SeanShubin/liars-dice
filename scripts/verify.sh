#!/bin/sh
# Builds and tests everything. This is what continuous integration should run.
set -e
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo build --workspace --all-targets
cargo test --workspace
