# Composition

Where every program's dependencies are built, where its observations of the outside world
are made, and why the two are the same place.

## Realizes
/specification/architecture.md — Composition, all of it.
/specification/glossary.md — Composition root, Wiring, Composition stage, Observation,
Simulated outside world.

## Approach

### The rule, and why it is about observations rather than about dependencies

An **observation** is a value a program reads from outside itself: a clock, a random
source, an environment variable, a stored record, a keystroke, a frame off a socket.
Everything a program cannot repeat on its own comes in as one, and
`Every program can be composed over a simulated outside world` is the whole point —
it is what makes a match replay exactly, and it is why multiplayer could be proved
before any infrastructure existed.

The rule is one sentence: **only a composition root observes anything outside its
program.** Every library crate is a function of what it is handed.

That is checkable rather than aspirational, and the check is a grep:

```
grep -rn "SystemTime::now\|Instant::now\|thread_rng\|std::env\|std::fs\|TcpStream" crates/
```

Today it finds nothing but three golden-file tests reading their own fixtures and
`relay/tests/boundaries.rs` reading a `Cargo.toml` to assert a dependency is absent. No
library crate reads a clock, a random source, an environment variable, a file or a socket.
`apps/*` do all of it.

### A value where you can, a trait where you must

`An observation reaches the program as a value, unless the program decides when to observe
it.` This is the distinction worth having written down, because the obvious reading of
*put all non-determinism behind one interface* is weaker than what this codebase already
does in two places.

**A value is stronger.** `game-state.md` records the move and the reason: *"a composition
root deals and installs. `crates/rules` now contains no randomness and no trait for it,
which is a stronger reading of 'consolidate all non-determinism into a single pluggable
abstraction' than the trait was."* A trait lets the core reach out and ask; a value means
the non-determinism never gets inside at all. There is no seam to stub because there is no
seam.

**A trait is for when the core decides when, how often, or with what.** The relay cannot be
handed the log, because which events it reads depends on which match a message names, and
whether it writes depends on what it read. So `Log` and `Connections` are traits. A browser
socket is the same: `Wire` exists because the page decides what to send and when.

Which way round a given observation goes is therefore not a style choice:

| Observation | How it arrives | Why |
|---|---|---|
| The clock | a value, `now: Moment` | The relay never needs a *second* reading, and one reading per message is what keeps the decision and the row stamp agreeing. |
| The HMAC secret | a value, `Secret` | Read once per cold start; the same for every match. |
| Random bytes for a guest id | a value (`identity.md`) | Taken once and stored. An id is something the world hands the program, like a keystroke. |
| The relay's URL | a value, read from a `<meta>` tag | One reading at startup. |
| The log | a trait, `Log` | What is read depends on the message; what is written depends on what was read. |
| The connection table | a trait, `Connections` | Same. |
| The socket | a trait, `Wire` | The page decides what to send. |
| Keystrokes and clicks | a trait, `Decides` | A driver is asked when it is its seat's turn. |

Four traits rather than one `Integrations`, and deliberately: a consumer should depend on
what it uses. `crates/relay` has no business being able to send a frame, and the test that
keeps `rules` out of it is the same instinct one level down.

### One reading, passed inward

`One clock reading serves one message` and `One clock reading serves one frame`. Both
programs that read a clock read it exactly once per pass of their loop, and everything
below gets that value.

In the relay that is load-bearing rather than tidy. `Service::serve` reads it, and the one
reading decides whether a match is inactive, stamps every row the message writes, and
timestamps the metric line. A `Clock` trait injected into `DynamoLog` would have read it
again inside the adapter, and then the stamp on a row and the decision that wrote it could
disagree about when the message happened. Passing the value makes that unrepresentable.

`meter.rs` used to hold its own `unix_millis`, with a comment claiming it reached nothing
but a metric. That comment stopped being true the moment a clock reading could decide
anything, so the function is gone and the meter takes the reading like everything else.

In the browser the unit is a frame: `Page::ui` takes `ui.input(|input| input.time)` once
and hands it to `pump` and `draw`. The countdown on the Cancel button runs off it, which
is display and decides nothing — the relay decides, and the page shows the answer.

### The pipeline: wiring, work, wiring, work

`A composition root that needs a computed value wires a stage to compute it, then wires the
rest over the result.` This is the shape for the case the plain rule does not cover: the
root cannot build everything up front, because part of what it needs has to be fetched
first, and fetching is work.

The rule is that no stage does both. Wiring constructs and connects; work is a call into
something that was wired. The logic lives in the thing being called, never in the call.

**`apps/relay`, three stages.**

```
let config = aws_config::load_defaults(...).await;   // integrations
let service = Service::new(config).await?;           // stage: wire, then work
                                                     //   env vars, then SSM
lambda_runtime::run(service_fn(|event| { ... }))     // wire the handler

// per invocation:
let incoming = incoming::read(&event.payload)?;      // work
let now = now();                                     // observe, once
let relay = Relay::new(log, connections, secret);    // wire, no logic
relay.receive(&from, message, now)                   // work
```

The second stage cannot be wired before the first has run, because the relay needs the
secret and the secret is in SSM. That is exactly the case the pipeline exists for. Failing
in `Service::new` fails the cold start, which is right: *a service that cannot read its
secret must not serve a single message rather than serve them unverifiably.*

The per-invocation stage is a second pipeline inside the first, and `A relay may be stopped
and replaced between any two messages` is what makes it legal to throw the whole thing away
afterwards — which `relay-service.md` already points out is that property under test in
production on every message.

**`apps/web`, two stages.**

```
let canvas = document.get_element_by_id("table");    // observe
let page = Page::new();                              // wire
//   relay_from_page() reads the <meta> tag          //   observe, once
eframe::WebRunner::new().start(canvas, ..., page)    // wire the loop

// per frame:
let now = ui.input(|input| input.time);              // observe, once
self.pump(now); self.draw(ui, now);                  // work
```

`identity.md` adds one observation to the first stage — sixteen random bytes from
`crypto.getRandomValues`, or the id already in local storage — and one stage after it: the
Discord ticket, which cannot be fetched until the page has seen a `?code=` in its own URL.

**`apps/auth`, when it is built, is one stage.** Read the client secret from SSM at cold
start, then handle requests. It is the first thing in the system that talks to something
outside AWS, which makes it the first composition root whose simulated outside world has to
include a stubbed HTTP peer.

**Everything else is one stage.** `apps/local`, `apps/hotseat`, `apps/minimal`,
`apps/admin`, `apps/probe` and `apps/prototype` each build what they need and run. A
pipeline is not a pattern to apply; it is what you get when a value you need has to be
fetched, and most programs have no such value.

### What a composition root may contain

`A composition root's wiring contains no logic` — the wiring, not the crate. This is the
one statement whose wording I narrowed while writing it down, because the shorter version
was false on arrival: `apps/web/src/main.rs` holds `screen_for`, `may_call` and `remaining`,
all of them tested, and `multiplayer-client.md` calls that file a composition root in the
same breath.

That is not drift. A screen is this program's own, nothing else needs it, and a crate for it
would be a crate with one consumer. What matters is that the *wiring* — `Page::new`,
`Relay::new`, `Service::new`'s construction — holds no decision, so there is no branch in
it that a test cannot reach. The logic beside it is ordinary logic and is unit tested like
any other: `screen_for` over every combination of four facts, `remaining` at its
boundaries.

So the honest shape of an `apps/*` crate is: the wiring, the observations, and whatever
logic exists only for this program. The rule that keeps it from becoming a dumping ground is
the other one — `Only a composition root observes anything outside its program` — plus
the boundary tests, like the one holding `apps/relay` to knowing no game rules, which exists
precisely because *a composition root is where someone would reach for `rules` to answer a
question the relay is not allowed to ask.*

## Boundaries

- **`specification/architecture.md`** owns the statements. This document owns how each
  program realizes them and nothing else; a rule that cannot be traced back there is a gap
  to raise, not a decision to take here.
- **`apps/*`** each own one composition root. Every observation in the system is in one of
  them.
- **`crates/*`** own no observations. `Log`, `Connections`, `Wire` and `Decides` are the
  four seams through which an observation reaches a library that has to ask for it; every
  other observation arrives as a value.
- **Nothing depends on this document** in the sense that no code realizes it alone. It is
  the rule every other document's Determinism section is answering, which is why that
  section is in the template.

## Decisions

**Four seams rather than one `Integrations` interface.** One interface is the usual advice
and would give one place to audit and one fake to write. Rejected because a consumer should
depend on what it uses: `crates/relay` has no business being able to send a frame, and
widening its dependency to a single god-interface would hand it the ability. The audit that
one interface would have given is a grep instead, and the grep is stronger — it finds an
observation made anywhere, including one that bypassed the interface.

**A value in preference to a trait, always, where the choice exists.** Argued above. The
test is whether the program decides *when* to observe. If it does not, there is nothing for
a trait to do but add a seam to stub.

**The clock is a parameter on `receive`, not a field on `Relay`.** The alternative was a
third type parameter or a boxed trait object, for a value that belongs to one message.
Passing it keeps `The relay holds no game state between messages except what the log holds`
literally true, and it is what makes the row stamp and the inactivity decision provably the
same reading.

**`A composition root's wiring contains no logic`, narrowed from the whole root.** The
shorter statement was false against existing, deliberate code. Recorded here rather than
fixed, because the fix — a crate per screen — would be worse than what it replaced.

**One clock statement per loop shape, rather than one general one.** `One clock reading
serves one message` and `One clock reading serves one frame` are two statements because
they are two checkable facts about two programs, and a single statement covering both would
have had to be vague about what a unit of work is.

## Determinism

This document is the determinism rule, so it has nothing of its own to route. What it adds
is where to look: every Determinism section in `implementation/` answers *which observation
does this unit add, and does it arrive as a value or behind one of the four seams.*

The current answer across the system: the dice derive from a secret and a round number, the
turn order from the same secret, the clock arrives as a parameter, and the only observation
still to be added is `identity.md`'s guest id. Nothing derives a value from an unrepeatable
source and puts it in the log, which is the property `Every client derives the same match
state from the same events` rests on.

## Testing

- **The grep is a test**: `crates/protocol/tests/observations.rs` walks every `.rs` file
  under `crates/*/src` and fails on twelve ways of observing, each paired with what to do
  instead. Checked by planting a `SystemTime::now` in `relay::memory` and watching it get
  named with its line number. It lives in `protocol` because the assertion belongs to no
  single crate and `protocol` is the floor the others stand on; it scans its siblings,
  which is unusual for a test and is the point.
  The list is a tripwire on the mistake, not a proof of its absence — nothing short of a
  sandbox would be — but it covers every way this system has actually wanted to reach
  outside, and the fix when it fires is almost never to add a pattern.
- `crates/table` plays a whole match over `relay::memory` with a fixed secret and a fixed
  clock, and it replays exactly. That is `Every program can be composed over a simulated
  outside world` for the relay and every client at once.
- `crates/relay`'s tests pass `Moment`s and make time pass by arithmetic. Nothing sleeps,
  so no test about inactivity is flaky and no test about something else expires underneath
  itself.
- `apps/web`'s own logic is tested as logic: `screen_for` over every combination of its four
  facts, `remaining` at the boundaries where rounding decides the answer.
- `apps/relay`'s `storage.rs` composes the real seams against DynamoDB Local, which is the
  simulated outside world one layer out — the same program, a real database, a fake account.

## Open Items

**No program has a fake of its own outside world end to end.** The relay's seams are faked
by `relay::memory` and its storage by DynamoDB Local, and `apps/web`'s socket by `Wire` —
but `Service::new` itself cannot be composed over a fake SSM, so the cold start is the one
stage no test covers. `apps/auth` will have the same gap against Discord, and that is the
point at which it is worth closing both.

## Status
Built. Mostly a description of what was already there, plus the boundary test that turns
`Only a composition root observes anything outside its program` from a habit into a
failing build.
