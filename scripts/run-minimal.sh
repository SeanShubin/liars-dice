#!/bin/sh
# Runs the minimal game: one human against AI opponents, two dimensional, single touch.
set -e
cd "$(dirname "$0")/.."
exec cargo run --release -p minimal "$@"
