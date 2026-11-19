# Scripts

One named script per thing you might want to run. Each exists as both a PowerShell and a
POSIX shell version, with the same name and the same behaviour.

| Script | What it does |
|--------|--------------|
| `run-hotseat` | The hotseat harness. A whole match in one window, every seat a virtual player. |
| `build-hotseat-web` | The same harness, built for the browser with trunk. |
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
```

`run-prototype` builds in release. A renderer in a debug build is slow enough to be
misleading about appearance, which is the one thing the prototype exists to judge.
