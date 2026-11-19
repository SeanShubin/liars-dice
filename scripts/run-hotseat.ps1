# The hotseat harness, natively. A whole match in one window, every seat a virtual player.
Set-Location (Join-Path $PSScriptRoot "..")
cargo run --release -p hotseat
