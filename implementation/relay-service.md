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

### One decomposition, three renderings

A value has to survive three crossings: into a DynamoDB item, out to a client, and in
from one. Three independent mappings would be three places to drift, and
`protocol.md` had recorded the opposite property — *the format remains the only
representation* — on the premise that nothing read a log back. The relay reads its log
back on every message, so that premise is gone and the property needs a mechanism instead
of an assumption.

So `crates/wire` holds one decomposition: a `Shape` — a kind, and named `Value`s, where a
`Value` is text, a count, the one float, a flag, a list of counts, or a nested shape.
`Event` and `Message` each decompose by an exhaustive `match` with no wildcard arm, so
adding a variant to either stops the build rather than arriving as nothing. Renderings are
then mechanical:

| Rendering | Where | Why there |
|-----------|-------|-----------|
| JSON | `crates/wire` | A browser client and the service both speak it, and so does everything on the path: API Gateway, CloudWatch, and whatever connects later that is not this Rust client. |
| DynamoDB item | `apps/relay` | `AttributeValue` is an AWS type, and the browser must never compile it. |

Both read an unknown kind as an error rather than as something to skip: a log that can be
partially read is not a log that may be partially trusted, and the position it sits at is
exactly where a replay would go quietly wrong.

`Value` is narrow on purpose. It is every kind of value the log and the message format
actually hold, so a rendering answers for a finite set of cases and a new kind of value
cannot be introduced without every rendering being made to handle it.

**Not the archive's line format.** `Display` already turns an `Event` into one line, and
reusing it would have meant one representation instead of two. It is the wrong one,
because it is a rendering for a person and renderings round: `percent` cuts odds to two
places, and `token` collapses whitespace, so a player called `Mary Jane` comes back as
`Mary-Jane` and is renamed by their own reconnection. The archive is allowed to do that.
The only copy of what happened is not. So `Display` stays the archive's, and the archive
stays derived.

**Not serde derives.** `protocol` and `relay` declare no dependency on serde and gain
none: `protocol` is the crate every other crate agrees on, including the one that compiles
to wasm. `serde_json` is used inside `wire` to parse and write the JSON text, because
escapes and unicode in a hand-rolled parser are a bug waiting for a player with an
interesting name. The mapping is `Shape`; the library types stay plain.

`Event::Variant` holds a `&'static str`, which nothing can deserialize into without
leaking a string or consulting a list of the names the relay writes. Consulting the list
is the better answer, so `relay::VARIANTS` is public and an unknown name is refused:
guessing which rules were in force is the one thing a replay must never do.

`Moment` is carried as unix nanoseconds, so `protocol` gains the inverse of the
`from_unix_nanos` it already has — a reversible pair tested against each other, rather
than a date formatted into a string that something would have to parse back.

Keys are written in alphabetical order, which is `serde_json`'s default and worth keeping:
a frame's bytes are then a function of what the shape holds, not of the order somebody
declared the fields in.

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
| `log_reads` | The divisor. Time without it cannot distinguish more matches from a slower table. On a lobby view it is the matches folded; on a message it is the appends, because the relay reads to find the end of the log before each one. |
| `events_read` | The actual work, independent of how fast the hardware was that day. |
| `read_capacity` | What it costs and what gets throttled, from `ReturnConsumedCapacity`. |

The line is CloudWatch Embedded Metric Format: structured JSON on stdout, which costs one
log line, needs no `PutMetricData` call on the hot path, and arrives as a CloudWatch metric
with percentiles. So the thresholds below can be alarms rather than a habit of looking.

Two separate timers because they fail for different reasons and have different fixes. A
slow fold is answered by the projection. A slow scan is answered by not scanning, and no
projection of open matches fixes it.

### The stack it deploys to

Its own stack, `liars-dice-relay`, rather than part of `seanshubin-com`. That one owns the
hostname and is shared by path; this is one project's backend, nothing else depends on it,
and it can be torn down and rebuilt without touching the site.

| Resource | Note |
|----------|------|
| The log table | Point-in-time recovery on, and deletion protection. The one thing that cannot be rebuilt from something else. |
| The connection table | Neither, deliberately: every row in it can be rebuilt by clients reconnecting. |
| The WebSocket API | Three routes, all to one integration. `RouteSelectionExpression` is set and unused, because the relay decides what a frame is and a route that read the body would be a second place deciding. |
| The function | `provided.al2023`, a Rust binary named `bootstrap`, ten second timeout. |
| Its log group | Declared rather than left to the first invocation, so retention applies from the first line instead of after thirty days of unbounded logs. |

The execution role grants `PutItem`, `Query` and `Scan` on the log and **not** `DeleteItem`
or `UpdateItem`. The log is append-only, and this is where that is enforced rather than
hoped for: a bug that tried to rewrite history would be refused by IAM before it reached
the table. It reads exactly one SSM parameter — the Discord client secret lives under the
same prefix and this function has no business reading it.

**Two things are done by hand, once.** CloudFormation cannot create a `SecureString`
parameter, and a plaintext one would put the HMAC key in the template, the change set and
the stack events. So the secret is put at a terminal, as the Discord client secret already
was. The artifact bucket is in the bootstrap stack rather than this one, because a stack
cannot create a bucket and read an object out of it in the same deploy.

### Building a Rust Lambda in the gate

The runner's glibc is newer than Amazon Linux 2023's, so a plain `cargo build` there links
against symbols the runtime does not have. It compiles, it uploads, it deploys, and it
fails at the first invocation — the worst available order.

The binary is built **inside the image the function runs on**, with Docker, in the gate.
The alternatives all answer the glibc question; this one removes it. Zig can target an
older glibc, but then the build depends on a particular zig and `cargo-zigbuild` being
compatible, which is a question with an answer that changes. A static musl build would
need a musl C toolchain, because `aws-lc-sys` is a C library. Building in `al2023` needs
none of that: the thing that defines the target is doing the compiling.

Measured rather than assumed, which is the only reason to believe any of the above: the
binary needs at most `GLIBC_2.34`, the image provides `glibc-2.34`, and it links nothing
but `libc`, `libm` and `libgcc_s`. Stripped it is 20 MB, and 7.1 MB zipped. The whole
build takes under two minutes from a cold container, which is cheaper than the cross
-compiling toolchain it replaces.

The zip the deploy publishes is the one the gate built, as with the browser bundle. The
object's key carries the commit, which is what makes CloudFormation replace the function's
code: redeploying the same commit is a no-op and deploying a new one cannot be mistaken
for it.

## Boundaries

- **Depends on** `crates/relay`, `wire`, `protocol`, `integrity`, and the AWS SDK.
- **Does not depend on** `rules`. The library's boundary test reads its manifest and
  fails if `rules` ever appears; this crate carries the same test, because a composition
  root is exactly where someone would reach for the rules to answer a question the relay
  is not supposed to be asking.
- **Owns** `DynamoLog`, `GatewayConnections`, the DynamoDB rendering of a `Shape`, the
  three handlers, and the SSM read that produces the `Secret`.
- **Does not own** the decomposition. `crates/wire` does, because a client needs it too,
  and a format only one side of a socket agrees with is not a format.
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

- **Both renderings round-trip, against one list.** `wire::examples::every_event` and
  `every_message` are public and part of the crate rather than of a test, because more
  than one rendering has to be held to them. Two lists would drift, and the one that
  drifted would be the one not covering the variant that broke. Adding a variant is
  already a compile error in `shape_of`; adding it to that list is what makes it handled
  *correctly*, in every rendering at once.
- **The item rendering round-trips, with no AWS at all.** One of every `Event` variant, encoded to
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

`crates/seat` is the client half, and it holds no transport. Frames in, frames out: the
caller owns the socket, so `apps/probe` gives it one from a terminal and `apps/web` will
give it one from a browser, and neither has any logic to get wrong because there is none
there to duplicate. `crates/table` is the other arrangement of the same pieces — it owns
the relay and every player in one process — and the two now share the odds calculation
rather than each having one, because that figure goes in the log and two answers to it
would be two logs.

Which means the socket is the only thing `apps/probe` adds, and the only thing it cannot
make certain. So the whole match is played through the wire format in a unit test first:
a relay in the same process, reached only through JSON, with two seats taking turns. That
covers the message rendering, the answer parsing, a seat learning which seat it is, the
turn taking and the round settling. The probe then adds a real WebSocket, real API
Gateway and a real Lambda, and nothing else.

A wrinkle that only appears once a client is a process rather than an index in a vector:
`Client::new` takes the seat up front, and a joiner does not know its seat until the relay
assigns one. `crates/table` sidesteps this by choosing the seats itself, which a client on
a socket cannot. The answer needs no protocol change — buffer arriving events until the
`join` naming this client's own account arrives, then construct the `Client` and replay the
buffer in order. The replay is exact because the log is ordered, which is the whole point
of the conditional write. Worth knowing before the client is written, because the
alternative looks easier and is a new answer frame carrying a seat, which would be a second
place that decides who is sitting where.
- The counters count. Against DynamoDB Local: a lobby view over three matches of known
  length reports exactly those matches and exactly that many events. The timings cannot be
  asserted on, which is why the counts are asserted on instead — a measurement nobody
  checked is a decoration, and the numbers that trigger the projection are the two that
  tests can pin.

## Open Items

**Fixed.** The position is the version now, and a match is named by
`HMAC(secret, account, nonce)` rather than by a count — see `The position is the version`
and `What a match is called` in `relay.md`. Hosting reads nothing, so the 38 ms scan is off
that path; the lobby's own scan remains, below. What follows is kept as the record of what
a deployment found that a single process could not.

**Hosting raced, and the conditional write did not save it.** `host` names a match
`match-{n}` where `n` is the number of matches the log holds, then appends through `write`,
which retries a `Taken` position by moving to the next one. That is right for appending to
a match you are already in: losing a race means your event goes after the winner's. It is
wrong for creating one. Two Lambdas serving two `host` frames at the same moment both scan,
both see `n`, and both pick `match-n`; the loser's `Version` is refused at position 0 and
then *appended at position 1 of the winner's match*. Two hosts, two accounts, one log,
nobody refused, and both told they created it.

In one process this could not happen, because `receive` is sequential — which is why the
library's tests did not see it and why it took a deployment to find.

**Nothing asks for the lobby.** `Relay::open_matches` exists and no `Message` reaches it,
so a client on a socket cannot list open matches — the in-process harness calls the method
directly, and a browser cannot. That is a gap in `crates/relay` rather than here, and it
needs a message, an answer frame and a statement about who may ask. Deliberately not
added as part of the transport: a host learns its match id from the `created` frame and a
joiner can be told it out of band, which is enough for two command line clients to play a
whole match and prove the service. It also means the measurement in this document reports
`events_read` for per-message folds and nothing for a lobby view, because there are none
yet.

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

**The relay is deployed and has served a message.** `wss://071ye3vz8h.execute-api.us-east-1.amazonaws.com/live`.
A synthetic `$connect` returns 200 with no function error, which is the cold start reading
the secret out of SSM. A synthetic `$default` carrying a `host` frame wrote three events to
DynamoDB at positions 0, 1 and 2, attached the connection at seat 0, and emitted its metric
line. The name in the log is `Mary Jane`, with the space the archive's line format would
have turned into a hyphen — the codec decision, confirmed in production storage rather than
in a test.

It cost 104 ms, and **38 ms of that was the scan**. On an empty table, on a route that has
nothing to do with the lobby. The measurement earned its keep on the first message it ever
reported: `Log::matches()` is called by `host` to name the match, so hosting scans the whole
log table. That is the Open Item below, now with a number against it.

`match-0` in the log is that probe and is not a real match. Nothing deletes it, because the
log is append-only by design and the function's role deliberately holds no `DeleteItem` on
it.


The storage is written. `apps/relay` holds the codec, `DynamoLog`, `GatewayConnections`
and the meter, with the boundary test that keeps `rules` out of the composition root.

What is proven: every `Event` variant survives storage and reads back identically,
including the two things the archive's line format would have quietly changed — a name
with a space in it, and odds past two decimal places. That suite needs nothing running
and is in the gate. `protocol` gained `Moment::to_unix_nanos`, and the test that it
inverses `from_unix_nanos` found an overflow in it on the first run, in the year 2262,
where nanoseconds pass `i64`.

The three handlers, the SSM read, the stack and the pipeline step are written. Everything
up to the point of deploying has been run; nothing has been deployed.

The seven tests against DynamoDB Local pass: the conditional write refuses a taken
position without overwriting it, eight threads racing one position produce exactly one
winner, `matches()` lists a match once however many events it holds, the reverse index
answers `$disconnect`, and the meter reports one scan, three reads and six events for a
lobby view over three matches. They are `#[ignore]` so a machine without Docker still
gets an honest green, and the gate runs them with `--include-ignored`.

`crates/wire` is written and tested: one `Shape` decomposition for `Event` and `Message`,
the JSON rendering, and the DynamoDB rendering in `apps/relay` over the same shapes. The
round-trip test for the item rendering found a real collision while the wire format was
being added — `Message::Join` has a field genuinely called `match`, and the item reader
was assuming the word always meant the log's partition key, so it dropped it. Nothing in
production would have found that, because a message is never stored.

The Lambda build has been run, locally, in the same container the gate uses and with the
same command: it produces a 7.1 MB zip holding one file called `bootstrap` that needs no
glibc newer than the runtime's. The artifact bucket and the SSM secret both exist.

Not run: the deploy. The stack has never been created, and a CloudFormation stack that
fails during its first creation rolls back to `ROLLBACK_COMPLETE`, which `deploy` cannot
update — it has to be deleted before a retry.

Not started: the command line client that plays a whole match over a real socket, which is
what would actually prove any of this. Until then `relay::memory` remains the only
arrangement that plays a whole match, and the deployed service has served no message.
