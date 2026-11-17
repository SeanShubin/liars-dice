#!/usr/bin/env pwsh
# Runs the die rendering prototype: one die, a ground, and live appearance settings.
# Drag outside the settings window to rotate the die. Scroll to zoom.
$ErrorActionPreference = 'Stop'
Push-Location (Join-Path $PSScriptRoot '..')
try { cargo run --release -p prototype @args } finally { Pop-Location }
