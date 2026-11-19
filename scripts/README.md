# Scripts

One named script per thing you might want to run. Each exists as both a PowerShell and a
POSIX shell version, with the same name and the same behaviour - except `deploy-site`,
which is POSIX only because `push` replaced the reason to run it by hand.

| Script | What it does |
|--------|--------------|
| `run-hotseat` | A hotseat match in one window. Seats are added in the lobby, each a person or a virtual player. |
| `build-hotseat-web` | The same match, built for the browser with trunk. |
| `push` | Push master, watch the pipeline, and say out loud whether the site went live. |
| `deploy-site` | Build the site and publish it to seanshubin.com. What the pipeline does, by hand. |
| `check-site-links.py` | Every page reachable from the root, every link pointing at something. |
| `run-minimal` | The minimal game. One human against AI players, two dimensional, single touch. |
| `run-prototype` | The die rendering prototype. One die, a ground, and live appearance settings. |
| `verify` | Formats check, builds everything, runs every test. What continuous integration should run. |

Run them from anywhere; each one moves to the repository root first.

```
scripts/run-minimal.ps1          scripts/run-minimal.sh
scripts/run-prototype.ps1        scripts/run-prototype.sh
scripts/verify.ps1               scripts/verify.sh
scripts/push.ps1                 scripts/push.sh
```

`push` reads its verdict from the deploy job rather than from the run as a whole, because
the pipeline's last job is notify-only: it tests against the newest version of every
dependency, so it can fail with the site perfectly green. The exit code still follows the
run, so a red run is never reported as a clean one. `verify` is the local half of the
pipeline's gate, and running it first is the cheaper way to find out.

`run-prototype` builds in release. A renderer in a debug build is slow enough to be
misleading about appearance, which is the one thing the prototype exists to judge.
