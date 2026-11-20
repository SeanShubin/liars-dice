# The Relay and the Lobby

## Realizes
- [Architecture / State](/specification/architecture.md) — the log is the only state, the
  relay knows no game rules, and a relay may be replaced between any two messages
- [Multiplayer / Scope, Hosting, Joining](/specification/multiplayer.md) — hosting,
  listing, joining, barring, starting
- [Multiplayer / Claims](/specification/multiplayer.md) — the relay records every message
  it receives, and a message either happens entirely or not at all
- [Multiplayer / Hosting](/specification/multiplayer.md) — two matches never share an
  identity, and a repeated request to host creates no second match
- [Multiplayer / Joining](/specification/multiplayer.md) — two players never share a seat
- [Multiplayer / Versions](/specification/multiplayer.md) — a match records its protocol
  version and refuses a mismatched join
- [Game Rules / Match](/specification/game-rules.md) — absence, and a player who
  reconnects resuming their seat
- [Dice Integrity / Secrecy](/specification/dice-integrity.md) — no player learns another's
  dice before the reveal

Not in this unit of work: saves. They are host-side, independent of a live match, and get
their own cycle.

## Approach

### A library, so the first multiplayer match needs no AWS

`crates/relay` holds the behaviour and knows nothing about API Gateway, Lambda or
DynamoDB. Two traits are the whole seam:

```rust
pub trait Log {
    /// Append at `position`, failing if something is already there.
    fn append(&mut self, match_id: &MatchId, position: u64, event: &Event) -> Result<(), Taken>;
    fn read(&self, match_id: &MatchId) -> Vec<Event>;
}

pub trait Connections {
    fn of(&self, match_id: &MatchId) -> Vec<Connected>;
    fn attach(&mut self, match_id: &MatchId, connection: ConnectionId, seat: Seat);
    fn detach(&mut self, connection: ConnectionId);
}
```

`apps/relay` wires them to DynamoDB. `apps/local` wires them to memory and runs the relay
in-process with two clients, so a real networked match can be played and tested before any
of the infrastructure exists. That ordering is deliberate: it keeps the pipeline off the
critical path for proving multiplayer works.

### The log is the state, and reading it is the state machine

Every message the relay receives is appended and then answered. To answer anything — who
is in this match, has it started, whose turn is it, which version does it speak — the relay
reads the log back and folds it. Nothing is cached between messages.

This is affordable because a match is a few hundred events and it is what
`A relay may be stopped and replaced between any two messages without affecting a match in
progress` actually requires. The only things that survive outside the log are the
connection table and the long-lived secret, both of which the specification names.

**Append is conditional.** `append` writes at an explicit position and fails if that
position is taken, which is DynamoDB's `attribute_not_exists` on the sort key. Two
messages racing cannot both land at position 41; the loser re-reads and retries. This is
what gives the log a total order without a lock, and a total order is what makes
`Every client derives the same match state from the same events` true.

### The position is the version

Every handler folds the log into a view, decides from it, and writes. That is check-then-
act, and for most of this project's life it could not fail: `receive` is sequential, so
the fold and the write happen inside one uninterrupted call and the window is zero wide.
Deployment made the window real, because a Lambda per message means two decisions can be
made against the same log state.

What that produced, found by two concurrent `host` frames against DynamoDB Local and then
confirmed to apply to `join` as well:

| Guard | What two simultaneous messages produce |
|-------|----------------------------------------|
| `match-{n}` from the match count | two hosts, one match, both told they created it |
| `Seat(view.seats.len())` | two accounts at one seat, and both sockets attached to it |
| a colour already taken | two players with the same colour |
| `seats.len() >= MAX_PLAYERS` | eleven players |
| `view.started` | a join landing after the start |
| `is_barred` | a barred player joining |

The seat row is the worst of them. The host race leaves a match that cannot start, so the
dice leak needs a third player to arrive; the seat race leaves a perfectly ordinary match
that starts normally and sends one seat's `Deal` to two different people. `Public`, `Game`
and `View` do not help, because the relay is not confused about who may see that seat —
it correctly sends the deal to every connection attached to it, and there are two.

**The remedy is the primitive that was already there.** `Log::append` is a compare-and-set
and the position is a version number; the mistake was retrying at the wrong level. `write`
retried a lost race by *moving forward*, which is right for appending to a match you are
already in — your event goes after the winner's — and wrong for any append whose position
was part of the decision. The `Taken` return carried exactly the information needed to
detect the conflict, and the retry discarded it.

So a message is served as optimistic concurrency control:

1. Read the log. Note its length `L`. Decide everything from that snapshot.
2. Append the message's events at `L..L+k`, each conditional on being free, **all or
   nothing**.
3. On `Taken`, discard the decision, re-read, and redo the whole message.

All-or-nothing is not optional. `host` writes three events and `start` writes a dozen or
more, so a conflict part-way through would otherwise leave a partial message in the log —
and `A message either happens entirely or not at all` is now a statement. DynamoDB
expresses this directly with `TransactWriteItems`: up to a hundred items, each with its
own condition, which is comfortably more than any message needs.

This moves the retry from `write` to `receive`, and it fixes every row of that table at
once, because each guard becomes validated by the commit rather than merely consulted
before it.

### What a match is called

`match-{n}`, where `n` was the number of matches the log held, had three faults and only
one of them was the race. It read the whole table to name one match, which the deployed
relay measured at 38 ms of a 104 ms message on an empty table. And a count is not an
identity: it is a function of global history, so hosting is not idempotent — a client that
times out waiting for its answer and resends gets a second match, and nobody is told.

A match is instead named by what was asked for:

```
match_id = HMAC(secret, "match" || account || nonce)   // leading bytes, hex
```

The client supplies the nonce; it need only be distinct per intent, not secret and not
unpredictable. Guessing a match id reveals nothing, because every seed is
`HMAC(secret, match_id || round)` and the secret is not the client's.

This buys three things at once. There is no read before the write, so there is no window
and no scan. The id is a function of the intent, so resending the same nonce produces the
same id, the conditional claim at position 0 fails, and the relay answers with the match
the sender already has — idempotency falls out rather than being added. And the id is
fixed-length hex, so nothing has to validate a client-supplied key or defend the log's
readability against one.

The alternative of keeping `match-{n}` and retrying on the number would also be correct
once the position is the version, and was rejected for the other two faults: it keeps the
scan the measurement indicted, and it leaves hosting non-idempotent.

The nonce changes the message format, so `PROTOCOL_VERSION` is 4. That is what the version
line is for, and the only existing log is the probe the deployed relay wrote, which becomes
unjoinable by a current client — correctly, since its message format is not this one.

**A seat is held by one connection.** Writing the idempotency test turned this up, and it
is not a detail. `Connections::attach` evicted the connection from wherever else it was,
because a socket is in one place; it did not evict whoever else held the seat. So a resent
`host` from a new socket left both sockets on seat zero — and the relay sends a seat's deal
to every connection on that seat, which is the dice leak the concurrency fix had just
removed, reintroduced through the front door. `attach` now evicts both ways. A second tab
on one account takes the seat and the first goes quiet, which is the right trade: before
identity exists there is nothing to distinguish a second tab from somebody else claiming
the account.

**And evicting both ways is not enough, because an eviction is a procedure.** The
idempotency test found the missing eviction; the *concurrent* idempotency test, run against
real storage, found that having it does not help. Two invocations of `attach` for one seat
both read the match's rows, both found nobody to evict, and both wrote. This is the
check-then-act defect the log was already fixed for, in the one store the fix never reached
— the log carries a version in its key and this table carried nothing.

What it costs is not what the comment on `attach` claimed. Both sockets carry the same
account, because the only calls that name one seat are a host resend, a join resend and a
`Resume`; with an account worth trusting, nobody reads another player's dice. The harm is a
deadlock. `waiting_on` counted rows while `round_over_count` counts distinct seats, so one
extra row lifts the threshold above a number it can never reach: the round never reveals
and the match is held still for good by a tab that is gone.

The remedy is the one the seat race already taught — put the thing being claimed in the
key. `relay-service.md` has the table. Nothing in this crate changes, because `Connections`
already says `attach`, `of` and `detach`, and which of those was safe under concurrency was
never visible here.

**The reveal waits on seats, not on sockets.** Counting rows and counting seats were the
same number only while a seat held one row, and `round_over_count` was always the second
kind. Counting the seats that are present and have not left is what the paragraph above
always meant, and it is what stays right once a place can be a watcher, because a watcher
is sent the round and never acknowledges it.

### Dealing, and the one line the relay walks

The relay deals: `Every round deals the dice limit to every seat`, which needs no rules
knowledge, only the seat roster. It commits before the round and reveals when the round
ends.

Knowing *when* a round ends is the delicate part, because `The relay knows no game rules`.

The first design here was wrong and building it showed why. It said the relay would treat
`liar` and `spot-on` as round-ending markers. But the relay does not know whose turn it
is — that needs rules — so it would have acted on a round-ending message from *any* seat
at *any* time. One client, honest or not, sending `liar` out of turn would make the relay
reveal the seed and deal a new round while every other client's rules engine said the
round was still going. Not griefing: a desync, between the relay and everybody.

So the reveal is triggered by agreement instead. Each client, when its own engine says the
round ended, sends `round-over`. The relay counts those, and when every connected seat has
sent one it reveals the seed and deals the next round. Counting acknowledgements is not a
game rule, no single client can force a reveal, and a client that withholds one stalls the
match exactly as an absent player does — which is already the accepted behaviour and gains
the withholder nothing.

The relay still never computes who was right, who loses a die, who is eliminated or whose
turn it is. Which player opens the next round comes from the clients' own agreement.

Stated as a rule the code can be held to: **the relay may know which messages end a round;
it may not know what a round means.** If it ever needs to know whether a claim was true,
the design has gone wrong.

### Secrecy by withholding

The log holds every seat's deal from the moment it happens, because the log is the record
of what the relay did. The *fan-out* is what keeps the secret:

| | during the round | at the reveal |
|---|---|---|
| your own deal | sent | — |
| other seats' deals | withheld | sent |
| the seed | withheld | sent |
| claims | sent | sent |

A client that reconnects mid-round is replayed under the same filter, so reconnecting
reveals nothing that staying would not have.

### The lobby

Hosting, joining, barring and starting are events in the match's own log, so a lobby is
just a match that has not started. The list of open matches is a view folded over the
logs: a match appears if it has a `host` event, no `start` event, and no `bar` naming the
asking player.

`Every match that can be joined is listed` is actorless, so the list is served without
authentication; hosting and joining require it. The list carries the host's name and the
number of players, which is what the statements ask for and no more.

**`Message::List` is the one message that names no match**, because asking which there are
is what a client does before it has one. It carries the account, and only so that
`A match is not listed to a player barred from it` can be honoured — the relay has no
other use for knowing who asked. It is also the one message that writes nothing and reads
everything, so it cannot be stale and has no retry: there is no decision for a concurrent
append to invalidate.

A listing is answered on `Outcome::listed`, beside `created`, rather than as events on
`sends`. A listing is not something that happened to a match; it is an answer to the
socket that asked, and the two travel differently for that reason. It goes to the asker
and to nobody else.

**A listing is stale the moment it is sent, and that is not a defect to design out.** A
match in it can fill or start before anybody acts on one, so joining from a listing can
still be refused — `AlreadyStarted`, `MatchIsFull`, `ColourTaken` — and those refusals are
what make acting on a stale listing safe. The alternative is a subscription the relay has
to cancel on every path a client can leave by, including the ones where it never says
goodbye.

The version check happens at join: the match's `version` line against the joining client's
own, refusing on mismatch rather than letting two builds disagree later about the rules.

**A repeated join is a return, and returning is resuming.** `A player holds one seat in a
match`, so a join from an account that already holds one answers with the seat it has —
the same answer `host` gives to a repeated host, and literally the same code path as
`Resume`, so a client that reloaded is caught up rather than seated twice. The check comes
before the guards on starting, fullness and colour, because every one of those is about
*taking* a seat: a match that has started is one they are already in, a full match is full
counting them, and the colour that is taken is their own. Before this, a return was
refused `ColourTaken` by its own colour, or — with a different colour — given a second
seat, which is worse than untidy: `attach` moves the socket to the newest seat, so the
first is left holding none and halts the match at its turn for good, in the log where no
reconnection can clear it.

**A match whose host has left is finished without having started.** Host powers are the
seat in the log and nothing transfers them, so there is no start to wait for. It is
refused rather than left to be discovered, and it is not listed either, because listing it
would be offering something that is not on offer.

That began as its own refusal, `HostHasLeft`, and `ending.md` folds it into cancellation:
`A host who leaves a match that has not started cancels it`, so the refusal is `Cancelled`
and a departing host produces the same `Event::Cancel { seat }` the Cancel button does. Two
refusals for *this match is finished* were two names for one fact.

`Refused::SeatIsAbsent` is gone in the same breath. Nothing had produced it since absence
stopped being enforced, and a refusal a client must understand and can never receive is a
case that will be wrong whenever somebody finally reads it.

### Absence and reconnection

A connection drops; the row leaves the connection table; the seat is absent. The relay then
refuses to append a claim until every seat is connected again, which is what
`A match cannot continue while any of its players is absent` means operationally. The host
is told who is missing.

A returning client authenticates, names the match, and the relay finds its seat by matching
the Discord account id in the join events. No new seat, no substitute — `A player who
reconnects resumes their seat`. The log is replayed to them through the secrecy filter.

The relay never times anyone out. There is no clock in this specification and nothing here
adds one.

### New events, and a third protocol version

`host`, `bar`, `lift`, `start` and `round-over` join the format. `join` already exists.
That makes `PROTOCOL_VERSION` 3 — the second bump in two cycles, which is the version line
doing its job rather than a problem.

## Boundaries

`crates/relay` depends on `protocol` and `integrity`, and **not** on `rules`. That
dependency being absent is how `The relay knows no game rules` is enforced by the build
rather than by discipline, the same way `apps/minimal` not depending on `protocol::relay`
enforced the desktop game having no network.

`apps/relay` is the composition root for Lambda. `apps/local` is the composition root for
a whole match in one process.

The archiver is not the relay and is not here. It is allowed to know the rules, because it
reads a finished log and derives from it.

## Decisions

**The relay does not write the `winner` line.** Deciding a match is over requires the
rules, which the relay does not have. A client could send an `end` message, but then a
single malicious client could seal a live match, and the others ignoring it would not undo
the relay going quiet. So the winner is appended when the match is archived, by the step
that replays the log with the rules engine. `winner` already exists in the format for
exactly this purpose — telling a reader the log is whole rather than truncated — and the
archiver is who knows that.

**Reading the log per message, rather than a cached projection.** Decided in
`deployment.md` and restated here because this is where it is paid for: a few hundred
events folded in a millisecond, against a cache that can be stale, can be lost, and would
have to be rebuilt by a replacement relay anyway.

**Absence is not enforced at all, and used to be.** The relay refused every append while
any seat lacked a connection, which realized `A match cannot continue while any of its
players is absent` by stopping the whole match the moment anybody's tab closed — and
stopping it forever if an eliminated player left, since the relay cannot know who is
eliminated and never will.

That statement is now `A match halts at the turn of an absent player`, and the halt needs
no enforcement whatever: when it is the absent seat's turn, no claim arrives, because no
other client will send one and `A client does not send an illegal claim` already covers
it. So `absent()` and `Refused::SeatIsAbsent` are gone. A live absent player loses nothing
by it — `liar` and `spot-on` affect only the caller and the previous claimer, and an
absent player is neither — and they catch up through `Resume`.

The reveal now waits on the seats that are present and have not left, which is what the
two paragraphs above always said and what the code got wrong by counting every seat that
ever joined. Without that an absent player would halt the match at the reveal rather than
at their turn.

**Membership is in the log; presence is not.** Joining and leaving are permanent facts
about who was in a match, so `Join` and `Left` are events. Connecting and disconnecting
are transport state that says nothing permanent, so they stay in the connection table and
a flapping socket writes nothing. This is the line between the two layers: who is taking
part is coordination, which the relay owns outright, while what a departure *means* is a
rule, which it must never know. `Bar` is the older precedent — the relay has always
decided who may participate without knowing a rule.

**Leaving is coordination; being eliminated by it is a rule.** `Message::Leave` records
`Event::Left`, and that is the whole of the relay's involvement. `rules::Game::concede`
is the rules answering the external input, and it is where the cost of the design is
paid — see the decision recorded on that function.

**Unanimous `round-over` rather than a marker message.** Reading `liar` and `spot-on` as
markers looked narrower, and was not: without knowing whose turn it is, the relay would
act on either message from any seat at any time, desynchronising itself from every client.
Requiring every connected seat to acknowledge costs one message per player per round and
removes the possibility entirely. The cost is paid in log lines, which are cheap, and in
liveness, which is already tied to everyone being connected.

**The retry belongs to the message, not to the append.** Stated in the Approach and
repeated here because it is the correction of a decision this document previously got
wrong: the old `write` looped `position += 1` until the append landed, and the old test
suite asserted that behaviour as the contract — *two appends racing for the same position:
one wins, the loser retries and lands after*. That is the correct contract for an event
whose position nobody reasoned about, and the wrong one for every event in this relay,
because each is written after a decision made against a particular log length. A retry
that cannot tell *you lost a race, go again* from *you asked for something that already
belongs to someone else* will eventually do the wrong thing, and did.

**A match id derived from the secret rather than from a count or a clock.** Deriving it
keeps the relay free of both a random number generator and a clock, which matters because
the id lands in the log as the partition key and feeds every seed. A random or
time-ordered id would work and would introduce a source of values into this crate that is
not the secret, which `Determinism` below exists to prevent. The nonce is the client's, is
recorded in nothing, and influences only which name the match gets.

**A host is sent its own join.** `host` used to answer with the match id and no events,
which looked harmless and was not: a client learns which seat it is on from the join that
names its own account, and from nothing else. A joiner gets that for free, because `join`
sends the new event to everyone including the joiner. A host got nothing, so a host client
could never build a client at all — found the first time two seats tried to play through
the wire format rather than through `crates/table`, which hands out seats itself and so
cannot notice. The join is read back from the log rather than taken from the message, so a
repeated request is answered with the join that is really recorded: the name and colour of
the first request are the ones that stand.

**No `rules` dependency, enforced by Cargo.** Stated above, repeated here because it is a
decision rather than an accident: if a future change needs `rules` in the relay, that is
the signal to stop and re-read this document.

## Determinism

The relay's own non-determinism is the dealing, which is already the integrity scheme and
already injected. Given a fixed secret and a fixed message order, a match replays exactly,
which is what lets `apps/local` drive a whole match from a script in a test.

A match's identity is derived too, from the secret and the nonce the hosting client sent,
so no clock and no entropy source enters this crate. The order of a match's events is the
log's conditional write, which means it is decided by the store rather than by which
invocation won a race — and a decision that was made against a stale log is not committed
at all rather than committed out of order.

The connection table is the one thing that is neither derived nor deterministic. It holds
no game state, only which socket is which seat, and `The connections a relay holds survive
the replacement of the relay` is why it lives in storage rather than in a process.

## Testing

In `apps/local`, against in-memory implementations of both traits:

- A whole match, two clients, start to finish, with the log checked afterwards the way the
  archive verifier checks it.
- A relay replaced between two messages mid-round: a second relay instance over the same
  log and connection table continues the match, and the round in progress is unaffected.
- A client that disconnects mid-round, reconnects, and resumes its seat — asserting it is
  sent its own deal and no other seat's until the reveal.
- A claim recorded while another seat is absent, with nothing sent to the seat that is
  not there. This test used to assert the opposite and is the one the old gate was
  written against.
- A reveal that proceeds on the acknowledgements of the seats that are present, and of
  the seats that have not left.
- A departure recorded once however often it is sent, with the socket detached, and
  refused outright from somebody who is not in the match.
- A join refused for a mismatched protocol version, and for a barred player, and for a
  match that has started.
- Two appends racing for the same position: one wins, and the loser is told `Taken` rather
  than being moved along. This bullet used to say *the loser retries and lands after*,
  which is what the defect above was, written down as the contract.
- A message is all or nothing: a conflict part-way through a multi-event message leaves
  none of its events in the log, not the first two of three.
- Hosting the same nonce twice creates one match, the second answer names it, and the
  resend's socket is the one left holding the seat.
- Hosting with a different nonce creates a second match, so idempotency is not an
  accidental ban on hosting twice.
- Taking a seat evicts whoever was on it, asserted on both implementations of
  `Connections`, because this is the invariant that decides who is sent whose dice. The
  concurrent half of it needs real storage and lives in `apps/relay`.
- A reveal that waits on one seat holding two sockets, which is the deadlock, asserted here
  by putting two connections on one seat directly. That is unreachable through `attach`
  once the key holds the seat, and asserting it keeps the reveal's own arithmetic honest
  rather than resting on the key to hide an error in it.
- Interference at *every* append a message makes, not one chosen point. Behind the `Log`
  seam the places a schedule can diverge are finite and countable, so the test enumerates
  them instead of sampling one. Each interference test also asserts that the interference
  fired: a test that checks an invariant after something that never happened is green and
  proves nothing, which is a worse failure than a flaky one because it is silent.
- Two hosts at once create two matches. Two joins at once take two seats, two colours, and
  attach to two different seats. These need real concurrent storage, so they live with the
  deployed implementations in `apps/relay` rather than here — see `relay-service.md`.
- A barred player does not see the match in the list.
- The relay crate does not depend on `rules`, asserted by a test over the dependency graph
  so the boundary cannot be crossed by accident.

## Status
Built. `crates/relay` is the library; 14 tests across two files, clippy clean with
warnings denied, workspace green at 38 test targets.

`apps/relay` is renamed `relay-service` as a package, because the library wanted the plain
name and two packages cannot share one. It is still a stub: nothing wires DynamoDB or API
Gateway yet, and it declares no dependency on `crates/relay` until it does, so the manifest
does not claim a connection that is not there.

The boundary is tested, not just stated. `tests/boundaries.rs` reads the crate's own
manifest and fails if `rules` ever appears in it, or if any dependency other than
`protocol` and `integrity` does.

Not built here, and all of it since built elsewhere: `crates/coordinator` is the client,
`apps/local` drives a whole match at a terminal through `relay::memory`, and `apps/web`
drives one seat of a real match in a browser. `crates/model` is the one thing on that list
still a stub.
