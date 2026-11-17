#!/bin/sh
# Runs the die rendering prototype: one die, a ground, and live appearance settings.
# Drag outside the settings window to rotate the die. Scroll to zoom.
set -e
cd "$(dirname "$0")/.."
exec cargo run --release -p prototype "$@"
