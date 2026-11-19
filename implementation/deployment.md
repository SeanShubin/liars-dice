# Deployment

## Realizes
- [Architecture / Deployment](/specification/architecture.md) — the pipeline and the routes
- [Architecture / Authentication](/specification/architecture.md) — Discord and seats
- [Architecture / State](/specification/architecture.md) — append-only log, connections
- [Applications / Deliveries](/specification/applications.md) — desktop and web
- [Multiplayer / Saves](/specification/multiplayer.md) — save storage and deletion
- [Data Browser / Scope](/specification/data-browser.md) — administrator access

## Reference Implementation

The `vote` project, a sibling checkout at `../vote` relative to this repository, already deploys a Discord-authenticated application to AWS from
GitHub. It is Kotlin and Gradle, so it is a reference for **pipeline shape, AWS wiring
and the OAuth flow** — not code to port. The build and artifact steps differ entirely
for a Rust target.

What to take from it:

| Concern | Where it lives in `../vote` | What to copy |
|---------|------------------------------|--------------|
| Pipeline | `.github/workflows/deploy.yml` | Single deploy mode. Every push to `master` runs the same pipeline; there are no deploy modes to choose between. |
| Concurrency | same | `concurrency: group: deploy, cancel-in-progress: false` — deploys queue rather than racing. |
| Credentials | same, `permissions: id-token: write` | OIDC federation, not stored access keys. No long-lived AWS secret exists anywhere in the repository or the workflow. |
| Build fan-out | same | Build jobs run in parallel on separate runners; one `deploy` job downloads their artifacts instead of rebuilding. |
| Stages | same | CloudFormation deploy, then S3 sync, then CloudFront invalidation, then a smoke test as the final gate. |
| Discord OAuth | `backend/src/main/kotlin/com/seanshubin/vote/backend/auth/` | `DiscordOAuthClient` for the flow, `DiscordConfigProvider` as the seam, `SsmDiscordConfigProvider` for the real source. |
| Secret storage | `SsmDiscordConfigProvider` | Client id and secret come from SSM Parameter Store at runtime, so they are never in the repository, the workflow, or an environment variable set at build time. |

## Approach

### The domain

seanshubin.com has a Route 53 hosted zone and no A record: the zone exists, the domain
serves nothing. **This repository owns the hostname.** The stack creates the
certificate, the distribution and the apex records, as `vote` does for
pairwisevote.com.

This is a decision with a shelf life. A hostname resolves to exactly one CloudFront
distribution, and path routing happens inside a distribution, not at DNS. So if
anything else ever needs to live on seanshubin.com, it cannot bring its own
distribution; either it joins this one, or a thin site stack takes ownership of the
zone and the distribution and this stack registers a behaviour with it. Taken now
because nothing else is there, and the refactor is only worth paying for when a second
thing exists.

### Edge routing

One distribution, three behaviours, and a WebSocket endpoint reached directly:

| Pattern | Origin | Caching |
|---------|--------|---------|
| `/*` | S3, through an Origin Access Control | the headers the sync sets |
| `/liars-dice/api/*` | HTTP API, Lambda | disabled, AllViewerExceptHostHeader |
| `wss://` | WebSocket API, Lambda | not through CloudFront |

Only the first exists so far. The API behaviours arrive with the relay.

**A CloudFront Function rewrites directory requests.** S3 has no directory index
below the bucket root, so a request for `/liars-dice/hotseat/` finds no object and fails.
The function appends `index.html` at the edge, before the origin is asked, which is a
third of a millisecond and no Lambda.

**The bucket is private.** CloudFront reads it with a signed request through an Origin
Access Control and nothing else can read it at all, so there is no second way in to keep
secure and no public bucket to misconfigure.

The static and API behaviours follow `vote` exactly, including the reason: the browser
sees a single origin, so cookies and authorization headers travel same-origin and there
is no CORS and no cross-site cookie problem.

The WebSocket deliberately does not go through CloudFront. CloudFront can pass a
WebSocket through, but it buys nothing: a WebSocket is not subject to CORS, and the
connection authenticates at `$connect` with a token rather than a cookie, so the
single-origin argument does not apply.

### Cache headers

Two-pass S3 sync, as `vote` does: HTML `no-cache`, everything else
`max-age=31536000, immutable`. This matters more here than it does for `vote` because
the WASM blob is the largest asset shipped, and content-hashed filenames make immutable
caching both free and correct.

### Bootstrap

A separate bootstrap stack, modelled on `vote`'s: one IAM role, and its ARN as an output
to set as the repository secret. The GitHub OIDC provider is not created here. It is an
account-wide singleton that `vote`'s bootstrap already put in this account, and a second
one fails with `EntityAlreadyExists`, so this template references it. The deploy workflow
assumes the role with `id-token: write`. No AWS access key exists in the repository, the
workflow, or a developer's machine.

The role's trust condition matches the subject claim GitHub puts in the token, and that
claim is not `repo:owner/name` here. This repository has immutable subjects turned on,
which is GitHub's newer default, so the claim carries the numeric owner and repository
ids instead:

```
repo:SeanShubin@3894337/liars-dice@1460374970:environment:production
```

`vote` predates the feature and still gets the legacy form. That is why the two roles'
conditions do not look alike, and why copying `vote`'s across was wrong in a way nothing
could catch until a workflow actually ran: every part of the setup was individually
correct, and the mismatch existed only in the one string neither side could check against
the other.

The ids are pinned rather than the feature turned off. Immutability is the point of the
format: renaming the repository or the account does not break the trust, and a new
repository claiming a freed name cannot inherit it. The cost is a template nobody can read
without knowing what the numbers mean, which the parameter's description carries, along
with the command that prints the real value rather than inviting someone to write one out.

### Identity

Discord OAuth server side, `identify` scope only. No guild gate: any Discord account
can authenticate. Config from SSM Parameter Store, cached, with a missing parameter
disabling login rather than failing the request, so the stack can deploy before the
secrets are set.

Events refer to a player by their seat, a small integer. The join event is the only one
carrying a Discord account id, and it binds that id to a seat. Names are resolved from
the OAuth handshake at join and recorded once; no later event repeats a name. Two
constraints force this. An OAuth user token reads only its own account, so resolving
another player's id to a name would need a bot token and a live call to Discord —
recording the name at join leaves historical logs readable with no extra credential.
And an AI player has no Discord account at all, so a log keyed on account ids could not
record a desktop match; a log keyed on seats records both deliveries with no special
case.

### Transport

API Gateway WebSocket with Lambda handlers, and a DynamoDB connection table keyed by
match. Turn-based play sends very few messages, so per-connection charges stay small and
the connection time limit is survivable. Each row carries a role, so admitting
spectators later changes no table.

The table is why the specification now says the relay holds no *game* state: a
connection id is transport plumbing the log cannot hold, and keeping it in DynamoDB
rather than in the process is what keeps "a relay may be stopped and replaced between
any two messages" true.

### Storage

Three artifacts, and only the first is canonical. This is the `vote` pattern — append-only
events as the source of truth, everything else derived — with one deliberate difference,
noted below.

**The event log.** DynamoDB, one item per event, partitioned by match, append-only. The
only thing that cannot be reconstructed from something else. S3 cannot play this role:
objects cannot be appended to, and a live match appends one event per claim.

**The game data.** The projection: whose turn it is, who holds how many dice and how many
shield tokens, which saves exist, which were deleted. Rebuildable from the event log at
any time, which is what lets a deleted save disappear from the view while the log stays
intact. `vote` stores its projection in a table; this project does not, at least to begin
with — see Decisions.

**The archive.** One S3 text file per finished or saved match, one command per line, the
format `crates/protocol` already writes, keyed by the host's Discord account id with the
save's name and timestamp in the key, which is what makes a save visible only to its host.
Derived from the event log, but a serialization rather than a reduction: it is the artifact
a person downloads, and it is what makes `Verification requires nothing beyond the event
log` literally true rather than aspirational. A player can read the archive of a complete
match they played in; the data browser, which reads everything, stays administrator-only.

### Dependency upgrades

Two jobs from one idea, borrowed from `../boardgame`'s pipeline shape. The gate builds
from the committed lockfile, so a deploy is reproducible and an upstream release can never
break a build that was already green. A notify-only job on every build runs `cargo update`
into a throwaway lockfile and runs the full test suite against it: a broken upgrade reds
the run and notifies, without touching production. That second job is the only thing that
can make `A broken upgrade is discovered automatically` true of the repository.

### The hotseat match

`seanshubin.com/liars-dice/hotseat` is the first thing deployed, and it is deliberately
the cheapest thing that proves the most: it exercises the build, the bucket, the
distribution, the routing, the directory-index function and the whole game, while needing
no relay, no authentication and no database.

It is a real match rather than a demonstration of one: the screen shows what one seat may
see and is handed over between seats, so hidden information survives having one device.
How that works is [Hotseat](/implementation/hotseat.md).

Measured: **3.96 MB raw, 1.55 MB gzipped** on eframe 0.37, against the 44.2 MB and
13.8 MB a Bevy page serves. That is the number the host decision was argued on and it is
now a measurement. It was 2.87 MB and 1.34 MB on eframe 0.33; the growth is egui's new CPU
renderer, and why that is paid rather than avoided is in
[Hotseat](/implementation/hotseat.md). Enabling both the `glow` and `wgpu` backends breaks
the wasm build outright; `glow` alone builds and carries nothing a two dimensional
presentation does not use.

### Desktop builds

The pipeline builds a desktop binary per presentation and publishes it to GitHub
Releases. Locally, `scripts/run-minimal` already launches one; the pretty game gets its
sibling. Desktop code, web code and common code sit in separate modules, so the web
build cannot reach the filesystem seam and the desktop build cannot reach the network
seam.

## Decisions

The configured administrator account is a Discord account id, held wherever `../vote`
holds its Discord configuration — SSM Parameter Store — rather than compiled in. This
follows from the specification saying *configured* rather than naming an account, and
from the same reasoning that keeps the OAuth secret out of the repository.

**The projection is computed on demand, not stored.** `vote` keeps its projection in a
table because elections accumulate ballots without bound; a match ends. A liars dice match
is a few hundred events, so replaying them to answer "whose turn is it" costs a
millisecond and leaves nothing that can be stale, be migrated, or fall out of step with
the log. It also keeps `The event log is the only state` true in the strongest available
sense. The alternative — a stored projection from the start — is the same architecture
with a cache added; the cache can be built later, when something measured asks for it.

**The minimal presentation does not use Bevy.** `apps/minimal` calls egui in 62 places and
Bevy in one: a startup system that spawns `Camera2d`. There are no queries, components,
sprites, meshes or transforms, and `crates/render-minimal` declares no dependencies at
all. The minimal game is an egui application hosted inside Bevy, so replacing that host
with `eframe` keeps every egui call site and runs the same code natively and in the
browser. Bevy stays for the pretty presentation, where there is 3D to render. The number
forcing this: `../boardgame`'s live Bevy bundle is 44.2 MB raw, 13.8 MB gzipped, against a
presentation that draws coloured rectangles and numerals. The eframe bundle has not been
measured yet and should be, as the first step of the web work rather than after it.

**Text entry is overlaid, not painted.** egui renders to a single canvas, so it cannot
raise a browser keyboard; eframe fakes one with a hidden `<input>` and its own
documentation says that does not always work, with fixes for Safari and Gboard still
landing in recent releases. `bevy_egui` inherits the same problem, so this is not a reason
to choose a different host. Since `Text is entered only to name something` and the only
thing named is a save, the mitigation is narrow: overlay a real HTML `<input>` over the
canvas for that one field and let the browser supply the keyboard, the IME and
autocorrect. This is what `Text entry works with an on-screen keyboard` depends on, and it
should be proved on a real tablet rather than assumed.

## Status
The static half is built and has run. One push drove the whole pipeline end to end for the
first time on 2026-11-18: the gate formatted, linted with warnings denied, tested, built
the release wasm bundle and checked every page reachable from the root, and the deploy
updated the stack, synced both cache classes, invalidated the edge and confirmed `/` and
`/liars-dice/hotseat/` answer 200. `scripts/push` is the way it is run.

Two things that first run settled, neither of which any amount of local green could have:
the workspace had never been compiled on Linux and it compiles clean, and the deploy role's
trust condition had never been exercised and was wrong - see Bootstrap above.

Not started: everything server side. The relay service, the DynamoDB log, the API Gateway
WebSocket connection table and Discord OAuth are all still stubs, and the deployed stack is
seven resources of static hosting. What is live is the hotseat match, which needs no server.
