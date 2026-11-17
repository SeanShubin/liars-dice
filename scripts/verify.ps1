#!/usr/bin/env pwsh
# Builds and tests everything. This is what continuous integration should run.
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try {
    cargo fmt --all --check
    cargo build --workspace --all-targets
    cargo test --workspace
} finally { Pop-Location }
