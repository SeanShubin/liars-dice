# Scripts

One named script per thing you might want to run. Each exists as both a PowerShell and a
POSIX shell version, with the same name and the same behaviour.

| Script | What it does |
|--------|--------------|
| `run-prototype` | The die rendering prototype. One die, a ground, and live appearance settings. |
| `verify` | Formats check, builds everything, runs every test. What continuous integration should run. |

Run them from anywhere; each one moves to the repository root first.

```
scripts/run-prototype.ps1        scripts/run-prototype.sh
scripts/verify.ps1               scripts/verify.sh
```

`run-prototype` builds in release. A renderer in a debug build is slow enough to be
misleading about appearance, which is the one thing the prototype exists to judge.
