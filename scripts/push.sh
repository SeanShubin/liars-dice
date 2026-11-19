#!/usr/bin/env bash
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
set -euo pipefail
cd "$(dirname "$0")/.."

# Must match the job's `name:` in .github/workflows/pipeline.yml.
DEPLOY_JOB="Deploy to seanshubin.com"

say() {
  if command -v powershell.exe >/dev/null 2>&1; then
    powershell.exe -NoProfile -Command \
      "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak('$1')" \
      >/dev/null 2>&1 || true
  elif command -v say >/dev/null 2>&1; then
    command say "$1" >/dev/null 2>&1 || true
  else
    printf '\a'
  fi
}

dirty=$(git status --porcelain)
if [ -n "$dirty" ]; then
  echo "Uncommitted changes. These are not pushed and will not be deployed:"
  echo "$dirty"
  echo
fi

git push

sha=$(git rev-parse HEAD)
short=${sha:0:8}
echo "looking for a run of $short"

# A run does not exist the instant a push returns. Poll for one minute.
found=""
for _ in $(seq 30); do
  found=$(gh run list --limit 20 --json databaseId,headSha,url \
    --jq "[.[] | select(.headSha == \"$sha\")][0] | select(. != null) | \"\(.databaseId) \(.url)\"" \
    2>/dev/null || true)
  [ -n "$found" ] && break
  sleep 2
done
if [ -z "$found" ]; then
  echo "No run for $short after 60 seconds."
  echo "Either the push added no new commit, or the workflow did not trigger."
  exit 1
fi
run_id=${found%% *}
run_url=${found#* }
echo "$run_url"

# Live progress. Its exit code is deliberately not the verdict: `gh run watch`
# also exits non-zero on its own transient failures - an API blip, a dropped
# connection - while the run is still perfectly healthy.
gh run watch "$run_id" || true

# The verdict comes only from the run's real state, polled until it is genuinely
# completed, which also covers a watch that bailed out early. An hour, because a
# cold run builds Bevy and a release wasm bundle from nothing - this backstop
# costs nothing when the watch above did its job.
completed=""
for _ in $(seq 360); do
  completed=$(gh run view "$run_id" --json status,conclusion \
    --jq 'select(.status == "completed") | .conclusion' 2>/dev/null || true)
  [ -n "$completed" ] && break
  sleep 10
done
if [ -z "$completed" ]; then
  echo "Still running after an hour. See $run_url"
  exit 1
fi

echo
gh run view "$run_id" --json jobs --jq '.jobs[] | "\(.conclusion)\t\(.name)"'
echo

deploy=$(gh run view "$run_id" --json jobs \
  --jq ".jobs[] | select(.name == \"$DEPLOY_JOB\") | .conclusion" 2>/dev/null || true)
if [ -z "$deploy" ]; then
  echo "No job named '$DEPLOY_JOB'. This script is out of step with pipeline.yml."
fi

if [ "$deploy" = "success" ]; then
  echo "https://seanshubin.com/ is live with $short"
  say "deployed to production"
else
  echo "The deploy did not succeed. Nothing was published."
  say "deploy failed"
fi

if [ "$completed" != "success" ]; then
  echo
  echo "The run is red. The tail of what failed:"
  gh run view "$run_id" --log-failed 2>/dev/null | tail -40 || true
  exit 1
fi
