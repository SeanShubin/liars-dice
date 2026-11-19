# The Relay Service

The deployed half of the relay: the two storage implementations and the Lambda that wires
them to a WebSocket. `crates/relay` is the state machine and is built; this is everything
between it and a socket.

## Realizes
/specification/multiplayer.md — Hosting, Joining, Claims and Versions, as deployed rather
than as proven in one process.
/specification/architecture.md — a relay may be stopped and replaced between any two
messages, and the thin composition root.
/implementation/relay.md — the library this serves.
/implementation/deployment.md — Transport and Storage, which chose API Gateway WebSocket,
Lambda and DynamoDB. Those choices are not revisited here; this is how they are met.

## Approach

### There is no server state to design

`Relay` holds a log, a connection table and a secret, and nothing else. `receive` reads
what it needs, appends what it decides, and returns what to send. So a Lambda invocation
is the whole lifetime of a relay: construct one over AWS-backed storage, hand it one
message, send what it returns, drop it.

This is not an awkward fit that has to be worked around. `A relay may be stopped and
replaced between any two messages` is already a tested property of the library, by
constructing a second `Relay` over the same storage mid-match. The Lambda model is that
test, run in production on every message.

### Three routes

| Route | What it does |
|-------|--------------|
| `$connect` | Nothing but accept. A connection belongs to no match until a message says so, and `Connections::attach` is what says so. |
| `$disconnect` | `Connections::detach`. The seat becomes absent, which the library already turns into whatever the match rules require. |
| `$default` | Parse a `Message`, construct the relay, `receive`, post every `(ConnectionId, Event)` it returned, answer the sender with the refusal or with nothing. |

`$default` is the only one with anything in it, and it is a dozen lines. Everything that
could be a judgement call — whether a join is legal, which seats see which deal, what the
log says — happened inside `receive` and was decided by code that already has tests.

### The log is a conditional write

`Log::append(match_id, position, event) -> Result<(), Taken>` was written for exactly one
storage primitive, and DynamoDB has it:

```
PutItem  Key { match: <match id>, position: <n> }
         ConditionExpression: attribute_not_exists(position)
```

`ConditionalCheckFailedException` is `Taken`, and nothing else is. Two messages racing for
the same position cannot both land, the loser re-reads and retries, and the total order
the whole protocol depends on comes from the database rather than from a lock. The trait
did not have to be bent to fit; it was shaped around this.

`read` is a Query on the partition, ordered by the sort key, which is the position. Events
are appended once and never changed, so the read is consistent without being made so.

### What an item holds

One item per event, and the event is stored field by field: the kind in its own attribute,
then one attribute per field of that variant. Numbers as numbers, seats as numbers, names
as strings.

Encoding is an exhaustive `match` on `Event` with no wildcard arm, so adding an event to
the protocol is a compile error here rather than an event that silently stores as nothing.
Decoding matches on the kind and treats an unknown one as an error, not as a line to skip:
a log that can be partially read is not a log that may be partially trusted, and the
position it sits at is exactly where a replay would go quietly wrong.

**Not the archive's line format.** `Display` already turns an `Event` into one line, and
reusing it here would mean one format instead of two. It is the wrong format, because it
is a rendering and renderings round: `percent` cuts odds to two places, and `token`
collapses whitespace, so a player called `Mary Jane` comes back as `Mary-Jane` and is
renamed by their own reconnection. The archive is allowed to do that — it is defending a
line-based format for a reader who never needs more precision than that. A record is not
allowed to do that. So `Display` stays the archive's, and the archive stays derived.

**Not serde.** `protocol` declares no dependencies, which is load-bearing: it is the crate
every other crate agrees on, including the one that compiles to wasm. `Event::Variant`
also holds a `&'static str`, which nothing can deserialize into without leaking a string
or consulting a list of the names the relay actually writes. Consulting the list is the
better answer and it belongs beside the storage, so `relay::VARIANTS` becomes public: the
names are part of the log this service writes, and whatever reads that log back needs to
know them.

`Moment` is stored as unix nanoseconds, so `protocol` gains the inverse of the
`from_unix_nanos` it already has — a reversible pair tested against each other, rather
than a date formatted into a string that something here would have to parse.

### The connection table

Keyed by match id with the connection id as the sort key, each row carrying the seat and
the role. `of` is a Query on the match. `attach` is a PutItem. `detach` arrives from
`$disconnect` knowing only the connection id, so it needs the reverse direction: a global
secondary index on connection id, which is one extra write per attach and one Query per
disconnect.

Role lives in the row rather than being derived, which is what `deployment.md` means by
admitting spectators later changing no table.

### Posting what the relay returned

`sends` is a list of `(ConnectionId, Event)`. Each becomes a `PostToConnection` on the
API Gateway Management API. A `GoneException` means the connection died between the Query
and the post, which is not an error: detach it and carry on. Every other failure is real
and is logged with the match and the position, because a send that was lost while the log
already holds the event is the one state a client cannot detect by itself — it recovers by
re-reading, which `Resume` already does.

### The async seam

The AWS SDK is async and `Relay::receive` is sync, and the sync side is the one worth
keeping. Making `Log` and `Connections` async would spread `async` through
`crates/relay`, its memory implementations, `crates/table` and every in-process test —
that is, through the only arrangement that proves multiplayer without AWS existing.

So the handler runs `receive` on a blocking thread and the trait implementations block on
the SDK from inside it:

```rust
let outcome = tokio::task::spawn_blocking(move || relay.receive(&from, message)).await?;
```

`spawn_blocking` hands the closure to a thread that is not a runtime worker, so
`Handle::block_on` inside the trait methods is legal there rather than a panic or a
stalled executor. The async-ness stops at the edge of this crate, which is where the
decision is paid for and recorded.

### Measuring what the lobby costs

`deployment.md` deferred the projection "until something measured asks for it", and this
is the measuring. A lobby view does work that grows with the whole table, so the question
is not whether it is slow today but which day it stops being fast, and that is answerable
only by a number nobody had to go looking for.

`DynamoLog` counts what it did. It holds a read counter, an event counter and a consumed
capacity counter, incremented inside `read` and `matches` — interior mutability, since the
trait takes `&self`, and no contention, since the instance lives for one invocation. The
handler times the call it wraps and emits one line per invocation:

| Field | Why it is the number and not another |
|-------|--------------------------------------|
| `route` | So a lobby view and a game message are never averaged together. |
| `scan_millis` | Finding which matches exist, which grows with every event ever written. |
| `fold_millis` | Reading and folding them, which grows with the matches still open. |
| `matches_folded` | The divisor. Time without it cannot distinguish more matches from a slower table. |
| `events_read` | The actual work, independent of how fast the hardware was that day. |
| `read_capacity` | What it costs and what gets throttled, from `ReturnConsumedCapacity`. |

The line is CloudWatch Embedded Metric Format: structured JSON on stdout, which costs one
log line, needs no `PutMetricData` call on the hot path, and arrives as a CloudWatch metric
with percentiles. So the thresholds below can be alarms rather than a habit of looking.

Two separate timers because they fail for different reasons and have different fixes. A
slow fold is answered by the projection. A slow scan is answered by not scanning, and no
projection of open matches fixes it.

## Boundaries

- **Depends on** `crates/relay`, `protocol`, `integrity`, and the AWS SDK.
- **Does not depend on** `rules`. The library's boundary test reads its manifest and
  fails if `rules` ever appears; this crate carries the same test, because a composition
  root is exactly where someone would reach for the rules to answer a question the relay
  is not supposed to be asking.
- **Owns** `DynamoLog`, `GatewayConnections`, the three handlers, and the SSM read that
  produces the `Secret`.
- **Does not own** identity. A `Message` arrives carrying an `Account`; who decided that
  account is the identity service's business and gets its own document.
- **Nothing depends on this.** It is a binary.

## Decisions

**Blocking inside the trait implementations rather than async traits.** The alternative
makes the whole library async to suit its deployment, and the library's value is that it
runs in one process with no deployment at all. The cost is that the handler must remember
`spawn_blocking`; the benefit is that `crates/table` keeps playing whole matches in a unit
test. Recorded in the Approach because it is the one place this crate's shape is not
obvious from the library's.

**The lobby list folds every log, and says so in writing.** `Relay::open_matches` folds
every match's log to answer who is hosting and how many have joined. That is a read of
every event of every match, per lobby view. At this project's size it is nothing; at some
size it is the first thing that breaks, and the fix is a projection — a second item per
match, updated on the events that change it.

Not built now, because `deployment.md` decided the projection waits for a measurement, and
a lobby nobody is looking at is not evidence. The measurement is built now, because
"it got slow" is not a trigger anyone can act on and a graph is. The trigger, so that
neither taste nor memory is involved:

- `fold_millis` p99 over a week above **200 ms** — the lobby is a page load, and a fifth
  of a second of folding alone, before the scan and the socket, is where a list stops
  arriving and starts loading.
- or `events_read` for any single lobby view above **5,000** — because a warm Lambda on an
  idle table can hold the time down long after the work has become absurd, and the event
  count is the leading indicator that does not depend on the weather.

Either one is sufficient and no further argument is needed. Until one of them fires, the
fold stands, and that is a position held on evidence rather than on hope.

**One table or three.** Separate tables for the log and the connections, rather than one
table with a discriminated key. They have different lifetimes — a log is permanent, a
connection row is transport plumbing — and different access patterns, and the only thing
a single table saves is a line of infrastructure.

**Secret from SSM, read per cold start and cached.** It is the HMAC key the per-round seed
derives from, so it must be the same for every invocation serving a match and must never
reach a client. SSM with the Lambda's role granted read on exactly that parameter. A
missing parameter fails the invocation rather than inventing a secret, because an invented
one would silently make every commitment unverifiable.

## Determinism

- The seed is `HMAC(secret, match_id ‖ round)` and stays so. Nothing here introduces a
  roll; dealing is the library's, through `integrity`, and this crate only supplies the
  key.
- The order of events is the log's conditional write, so it is decided by the database
  rather than by arrival order, scheduling, or which invocation won a race.
- Time appears nowhere in this crate. Saves and the archive carry timestamps and are a
  different unit of work.
- Nothing about an invocation — its id, its cold start, its concurrency — can reach a
  value that lands in the log.
- The measurement is outside the log and cannot reach into it. The counters are fields of
  `DynamoLog`, read by the handler after `receive` has returned, and emitted to a log
  stream; no event, claim, seed or commitment is derived from a duration. A metric that
  could change a value in the log would make replay disagree with the original match.

## Testing

- **The codec round-trips, with no AWS at all.** One of every `Event` variant, encoded to
  an item and decoded back, compared for equality. This is the test that matters most and
  the one that needs nothing running, so it is an ordinary unit test in the gate. The
  encoder's exhaustive `match` forces a new variant to be handled; only this list forces
  it to be handled *correctly*, so the list is named in a comment on the encoder as the
  other place to edit.
- `DynamoLog` against DynamoDB Local in a container: an append at a taken position returns
  `Taken` and does not overwrite; two concurrent appends at the same position produce one
  winner and one `Taken`; a read returns the events in position order.
- `GatewayConnections` against the same: attach, query by match, detach by connection id
  through the index.
- The tests that need DynamoDB Local are `#[ignore]`, and the gate runs
  `cargo test --workspace -- --include-ignored` with the container up. So `cargo test` on
  a machine with no Docker is still green and still honest: the runner prints what it
  skipped, which a test that quietly passes when it cannot reach an endpoint does not.
- The library's own suite is the behaviour suite and is not repeated. What is tested here
  is that these two implementations honour the traits' contracts, which is the only thing
  that can differ from the memory ones.
- **End to end, without a browser.** A command line client that opens a real WebSocket to
  the deployed relay and drives one seat with an AI. Two of them play a whole match
  against each other through API Gateway, Lambda and DynamoDB. This is the first time any
  message crosses a process boundary, and it proves the entire service with no presentation
  and no identity work in the way. It is also what makes the browser client's first failure
  diagnosable: if the socket already plays a match, the fault is in the page.
- A run of that client is checked against the match's log read straight from DynamoDB:
  same events, same order, every round's seed matching its commitment.
- The counters count. Against DynamoDB Local: a lobby view over three matches of known
  length reports exactly those matches and exactly that many events. The timings cannot be
  asserted on, which is why the counts are asserted on instead — a measurement nobody
  checked is a decoration, and the numbers that trigger the projection are the two that
  tests can pin.

## Open Items

The pipeline does not build a Lambda yet. A Rust function needs a cross-compiled artifact
for the Lambda runtime, which is a new toolchain step in the gate and a new upload in the
deploy. That is part of this unit of work, not a separate one, since nothing here can be
proven until it deploys.

Neither table exists in `deploy/template.yaml`, which still holds only the seven
static-hosting resources. The tests create their own, which is why they pass without it.

`Log::matches()` has no cheap implementation on the table as described. Listing distinct
partition keys means a Scan, which grows with every event ever written rather than with
the matches still open, so it is probably what the measurement above will indict first —
and the projection would not fix it. The likely answer is a small item per match written
on `host`, making the lobby a Query on one partition. Not decided here, because this is
the document that just agreed to measure before guessing, and `scan_millis` is reported
separately so the evidence arrives already separated from the fold's.

## Status

The storage is written. `apps/relay` holds the codec, `DynamoLog`, `GatewayConnections`
and the meter, with the boundary test that keeps `rules` out of the composition root.

What is proven: every `Event` variant survives storage and reads back identically,
including the two things the archive's line format would have quietly changed — a name
with a space in it, and odds past two decimal places. That suite needs nothing running
and is in the gate. `protocol` gained `Moment::to_unix_nanos`, and the test that it
inverses `from_unix_nanos` found an overflow in it on the first run, in the year 2262,
where nanoseconds pass `i64`.

What is written but not yet run: the seven tests against DynamoDB Local. The gate has the
container and runs them with `--include-ignored`; they are `#[ignore]` so that a machine
without Docker still gets an honest green.

Not started: the three handlers, the SSM read for the `Secret`, the two tables in
CloudFormation, and the Lambda artifact in the pipeline. Until those exist this crate
deploys nowhere, and `relay::memory` remains the only arrangement that plays a whole
match.
