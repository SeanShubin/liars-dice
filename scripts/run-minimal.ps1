#!/usr/bin/env pwsh
# Runs the minimal game: one human against AI opponents, two dimensional, single touch.
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try { cargo run --release -p minimal @args } finally { Pop-Location }
