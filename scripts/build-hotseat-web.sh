#!/usr/bin/env bash
# The hotseat harness, built for the browser.
#
# MSYS_NO_PATHCONV matters: under Git Bash on Windows, MSYS rewrites the leading slash of
# --public-url into a Windows path, and the built page then asks for its wasm at
# c:/Program Files/Git/liars-dice/... The page loads and stays blank, with nothing in the
# output to say why.
set -euo pipefail
cd "$(dirname "$0")/../apps/hotseat"
MSYS_NO_PATHCONV=1 trunk build --release --public-url /liars-dice/hotseat/
echo "built apps/hotseat/dist"
ls -la dist
