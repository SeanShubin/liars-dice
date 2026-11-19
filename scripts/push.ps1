#!/usr/bin/env pwsh
# Push master and watch the pipeline through to its verdict.
#
# The pipeline is gate -> deploy -> latest-dependencies, and the last of those is
# notify-only by design: it throws the lockfile away and tests against the newest
# everything, so it can fail with production perfectly green. That is why the
# spoken verdict comes from the deploy job and the exit code comes from the run
# as a whole - one answers "is the site live", the other "is anything red".
#
# scripts/verify is the local half of the gate. Running it first turns a red
# pipeline into a red terminal, which is the cheaper of the two.

$ErrorActionPreference = 'Stop'
# Native commands report by exit code rather than by throwing, and the checks
# below depend on that. A shell configured the other way would abort the watch
# on the first transient gh error instead of polling through it.
$PSNativeCommandUseErrorActionPreference = $false

# Must match the job's `name:` in .github/workflows/pipeline.yml.
$DeployJob = 'Deploy to seanshubin.com'

function Say($words) {
    try {
        Add-Type -AssemblyName System.Speech
        (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak($words)
    } catch {
        # No speech on this machine. The written verdict says the same thing.
    }
}

Push-Location (Join-Path $PSScriptRoot '..')
try {
    $dirty = git status --porcelain
    if ($dirty) {
        Write-Host 'Uncommitted changes. These are not pushed and will not be deployed:'
        Write-Host $dirty
        Write-Host ''
    }

    git push
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $sha = (git rev-parse HEAD).Trim()
    $short = $sha.Substring(0, 8)
    Write-Host "looking for a run of $short"

    # A run does not exist the instant a push returns. Poll for one minute.
    $run = $null
    for ($i = 0; $i -lt 30; $i++) {
        try {
            $raw = gh run list --limit 20 --json databaseId,headSha,url 2>$null
            if ($raw) {
                $run = $raw | ConvertFrom-Json |
                    Where-Object { $_.headSha -eq $sha } | Select-Object -First 1
                if ($run) { break }
            }
        } catch {
            # Transient gh or API error. Keep polling.
        }
        Start-Sleep -Seconds 2
    }
    if (-not $run) {
        Write-Host "No run for $short after 60 seconds."
        Write-Host 'Either the push added no new commit, or the workflow did not trigger.'
        exit 1
    }
    Write-Host $run.url

    # Live progress. Its exit code is deliberately not the verdict: `gh run watch`
    # also exits non-zero on its own transient failures - an API blip, a dropped
    # connection - while the run is still perfectly healthy.
    gh run watch $run.databaseId

    # The verdict comes only from the run's real state, polled until it is
    # genuinely completed, which also covers a watch that bailed out early.
    # An hour, because a cold run builds Bevy and a release wasm bundle from
    # nothing - this backstop costs nothing when the watch above did its job.
    $finished = $null
    for ($i = 0; $i -lt 360; $i++) {
        try {
            $raw = gh run view $run.databaseId --json status,conclusion,jobs 2>$null
            if ($raw) {
                $view = $raw | ConvertFrom-Json
                if ($view.status -eq 'completed') { $finished = $view; break }
            }
        } catch {
            # Transient gh or API error. Keep polling.
        }
        Start-Sleep -Seconds 10
    }
    if (-not $finished) {
        Write-Host "Still running after an hour. See $($run.url)"
        exit 1
    }

    Write-Host ''
    foreach ($job in $finished.jobs) {
        Write-Host ('{0,-10} {1}' -f $job.conclusion, $job.name)
    }
    Write-Host ''

    $deploy = $finished.jobs | Where-Object { $_.name -eq $DeployJob } | Select-Object -First 1
    if (-not $deploy) {
        Write-Host "No job named '$DeployJob'. This script is out of step with pipeline.yml."
    }

    if ($deploy -and $deploy.conclusion -eq 'success') {
        Write-Host "https://seanshubin.com/ is live with $short"
        Say 'deployed to production'
    } else {
        Write-Host 'The deploy did not succeed. Nothing was published.'
        Say 'deploy failed'
    }

    if ($finished.conclusion -ne 'success') {
        Write-Host ''
        Write-Host 'The run is red. The tail of what failed:'
        gh run view $run.databaseId --log-failed 2>$null | Select-Object -Last 40
        exit 1
    }
    exit 0
} finally {
    Pop-Location
}
