# The hotseat harness, built for the browser. PowerShell does not rewrite the leading
# slash of --public-url, so no guard is needed here; the bash script needs one.
Set-Location (Join-Path $PSScriptRoot "..\apps\hotseat")
trunk build --release --public-url /liars-dice/hotseat/
