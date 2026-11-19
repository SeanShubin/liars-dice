#!/usr/bin/env bash
# The hotseat harness, natively. A whole match in one window, every seat a virtual player.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run --release -p hotseat
