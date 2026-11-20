# Ending a Match

How a match stops, and how the relay knows. Three ways in, one state out, and the first
clock reading the relay has ever been allowed to make.

## Realizes
/specification/multiplayer.md — Ending, all of it.
/specification/architecture.md — State, the two statements about recorded time.
/specification/glossary.md — Over, In progress, Inactive, Cancelled, Abandoned.

## Approach

### One state, three ways into it

`Over` is the state: complete or cancelled, and no further event is recorded in it. Three
events reach it, and the log says which:

```rust
/// Every seat present said the match is over, and they agreed.
Complete,
/// The host ended the match. `seat` is the host's.
Cancel { seat: Seat },
/// Nothing was recorded in the match for an hour.
Abandon,
```

`MatchView` grows one field rather than three booleans, because the three are exclusive
and a reader should not be able to ask a question with two answers:

```rust
pub enum Ending {
    Complete,
    Cancelled { by: Seat },
    Abandoned,
}

pub ending: Option<Ending>,

pub fn over(&self) -> bool {
    self.ending.is_some()
}
```

`A log says whether its match is complete`, `A log says whether its match was cancelled`
and `A log says whether a cancelled match was cancelled by its host` are all one `match`
on that field, which is why it is one field.

Three events rather than one with a reason, because the archive should read `cancel 0` or
`abandon` and say what happened, and because after the fact the cause is not recoverable:
the times that distinguish *the host ended this* from *nobody came back* are not in the
log, by design. An `Option<Seat>` on a single `Cancel` was the alternative and is worse in
the one way that matters — it makes *abandoned* a cancel with a missing field rather than
its own fact.

### `No event is recorded in a match that is over`

One guard for messages, in `attempt`, before dispatch. This statement is the general form
of what started as a cancel-specific rule, and the general form earns its keep — it
refuses a start, a claim, a join and a leave into a dead match with one line, and it stops
the hour rule from cancelling a match somebody already won, which would silently overwrite
a real ending.

The expiry sweep needs its own test of the same thing, because the sweep is not a message:
nobody sent it and nobody is answered, so the predicate in `attempt` cannot reach it. In
`open_matches` one `view.over()` does both jobs — it implements `A match that is over is
not listed`, and being ahead of the hour check it is what makes abandoning idempotent: an
abandoned match is over, so the next pass stops there. Two statements, one test, and no
separate guard against writing a second `Abandon`.

The guard cannot be blanket, because two messages must still work on a match that is over:
`List`, which is about every match, and `Resume`, because somebody reconnecting to a
finished match wants to see how it ended. So `Message` grows a predicate:

```rust
/// Whether this message would write, and so needs a match that is not over.
fn writes(&self) -> bool
```

A `match` over every variant, which is the point: a message added later cannot be
forgotten here, because the compiler asks.

### Completion is counted, not judged

The relay never writes a resolution event. Every `LoseDie`, `Eliminate`, `Winner` and
`NextTurn` in the repository is written by `apps/minimal`, the desktop game; a hosted
match's resolution is each client's own fold of the rules over the commits, deals, claims
and reveals that the relay *does* write. That is right — resolution is derivable, so
recording it would be a second copy of what the log already says — but it means the relay
cannot tell a finished match from one in progress, and deriving it would mean knowing what
elimination means, which `The relay knows no game rules` forbids.

So the relay is told, and counts. `RoundOver` already does exactly this, and its comment
is the whole argument: *"a count of messages, not a judgement about the round, which is how
the relay times the reveal without knowing any game rule."* Completion is the same trick
one level up.

`Message::Over { match_id }` from a seated connection records `Event::MatchOver { seat }`.
When the distinct seats that have said it cover every present participant seat, the same
message also records `Event::Complete` — one append, because `A message either happens
entirely or not at all`.

Counting rather than taking the first client's word for it, because the rules are
deterministic and every client derives the same ending from the same events, so a seat
that disagrees is lying or broken. One seat saying so is enough to end a match that one
seat is losing. The cost of counting is that an absent seat delays the record, and the
hour rule collects those, so the cost is bounded and the grief is not possible without
everyone present colluding.

Presence comes from the connection table, which is not in the log, so a replay cannot
check that a completion was warranted. That is already true of every reveal — `waiting_on`
counts present seats — so this is the existing seam, not a new one.

### Cancelling

`Message::Cancel { match_id }`, and the asker's seat comes from the connection table the
same way `Bar` and `Lift` find the host. Not the host, `NotTheHost`.

The gate is the match's inactivity: the time between now and the last event the relay
recorded in it.

- **Not started** — allowed at once. There is no turn to wait on, and a minute of the host
  watching a timer for no reason is a minute of the host wondering what is broken.
- **Started, last event within the minute** — refused, with the time remaining.
- **Started, last event a minute ago or more** — allowed.

A refused cancel records nothing, so it does not reset the clock. Worth saying because the
alternative is a host who presses the button twice and never gets there, which is the kind
of bug that looks like the feature simply not working.

Inactivity is a property of the match, not of the player whose turn it is. In a started
match the two coincide in every case anybody will meet, because only the current player
can produce an event — nearly: another seat's `round-over`, or a `leave`, resets it. The
gain is that inactivity is one subtraction on one number read straight off the log, with no
judgement about whose fault the silence is.

### Abandonment, swept lazily

`A match in which no event has been recorded for an hour is cancelled`, and nothing is
scheduled to notice. There is no monitor, no timer, no second deployable: the check runs
where the matches are already being read, which is the lobby pass. For each match that is
not over and whose last recorded event is more than an hour old, `Event::Abandon` is
appended at the position just read — conditional, like every append, so a race loses
harmlessly. The loser re-reads, finds the match already abandoned, and leaves it alone.

This makes `list` a message that writes, which it was not before. The Lambda's role
already grants `PutItem` on the log table, so nothing changes in the deployment, but the
listing's cost is now a scan, a fold per match, and — only for matches crossing the hour —
one conditional put each. Normally that count is zero.

The one thing this does **not** fix is what `relay-service.md` already has an open item
for. `matches()` scans every row in the table whatever state the matches are in, so
abandoning a match tidies the lobby and saves nothing on the scan. The usability problem
and the cost problem looked like one problem and are two.

### Where the time comes from

Two things are needed and neither exists today: the relay has no clock, and a log row
holds only `match`, `position`, `kind` and the event's own fields.

**The row carries the time, and the event does not.** `Log::append` writes a `recorded`
attribute of unix milliseconds. It is storage metadata, written by the log implementation
rather than by `Relay`, and it never becomes part of an `Event`. That matters more than it
looks: if the time were an event field it would reach `wire`, the archive's line format,
every golden-file test and every client's fold, and `Every client derives the same match
state from the same events` would start depending on when they arrived. As a row attribute
the fold *structurally cannot* see it, which is a better guarantee than a rule nobody
breaks on purpose.

`Log::read` returns both, so there is one read and one source:

```rust
pub struct Recording {
    pub events: Vec<Event>,
    /// When the relay recorded the last of them. `None` if it recorded none, or if the
    /// last row predates recorded times.
    pub last: Option<Moment>,
}

fn read(&self, match_id: &MatchId) -> Recording;
```

Call sites take `.events`, so the fold is untouched. A second method instead of a wider
return was the alternative and is rejected twice over: on DynamoDB it is a second query
per match, which doubles what the lobby costs, and it is a second reader of the same rows,
which is a second thing to get wrong. Returning both means the cancel decision and the
append that commits it come from the same read — the discipline `The position is the
version` already insists on.

`None` means *inactive since forever*. A match always has events, since hosting writes
three, so in practice `None` is an untimed row from before this unit deployed — which is
provably older than the deploy and therefore older than an hour. One meaning, not two.

**The clock is an argument, not a dependency.** `receive` grows a parameter:

```rust
pub fn receive(&mut self, from: &ConnectionId, message: Message, now: Moment) -> Outcome
```

The relay gains no clock field and no third type parameter. `The relay holds no game state
between messages except what the log holds` stays literally true, the service passes the
time it already knows, and a test controls time by passing a number. The alternative — a
`Clock` trait behind a generic or a box — is more machinery for a value that is an input to
one message, in the same way `from` is.

### The refusal that carries a timer

`Refused::StillActive { seconds: u32 }`: how many whole seconds remain, rounded up, so a
host told *3* and retrying on 3 does not get told *0* and see nothing happen. `answer.rs`
already states the convention this fits — refusals *"carry numbers only where there is
something to say beyond the name"* — and `VersionMismatch` is the precedent, so the wire
needs no new machinery. On the wire: `why: "still-active"`, `seconds: Count`.

`Refused::MatchIsOver` is the only other refusal, and it replaces `HostHasLeft`. One
refusal for all three endings, because `No event is recorded in a match that is over` is
one rule: it is what a join into a cancelled match gets and what a claim into a finished
one gets. A separate `Cancelled` was drafted and dropped while this was built — it would
have been a second name for *this match is finished*, which is the duplication that
folding `HostHasLeft` away was meant to remove. How a match ended is in the log, for
anybody the log is sent to, which is how the Over screen names it.

The client counts the refusal down locally from the number it was given, on the browser's
own frame clock. That is display, not a decision, and nothing from it reaches the log.

### What the clients do with it

`seat::Seated` sets `over` on any of the three terminal events, outside the rules path —
`Stopped::Over` already exists for a message arriving after the end. `Seated` also grows
`ending()`, because the Over screen must say *the host cancelled this match* rather than
present a winner that was never recorded.

`apps/web` gains a Cancel button for the host on the waiting and table screens, the
countdown on refusal, and a branch in `over()` for the three endings. `PROTOCOL_VERSION`
goes to 8: three new events, two new messages, two refusals changed.

## Boundaries

- **`crates/protocol`** gains `Event::Complete`, `Event::Cancel`, `Event::Abandon` and
  `Event::MatchOver`, with their archive lines. No dependency changes.
- **`crates/relay`** owns `Ending`, the inactivity rule, the two thresholds as named
  constants, the completion count, and the expiry inside the lobby pass. It owns the
  `Recording` shape and the `now` parameter. `present_seats` was extracted while this was
  built: the reveal and the completion both ask *who are we waiting for*, and two copies of
  that question would be two answers to it. `Sitting` likewise — hosting and joining ask
  the same four things, and the clock pushed both parameter lists past what a lint will
  tolerate, which was a fair warning.
- **`crates/wire`** renders and parses the four events and the changed refusals. A client
  that cannot read a frame it is sent is what the version line exists to prevent, so this
  is what makes the version bump necessary.
- **`crates/seat`** turns a terminal event into `over` and an `ending()`, without consulting
  `rules`.
- **`apps/relay`** writes the `recorded` attribute in `DynamoLog::append` and reads it back
  in `read`. Nothing else in the service changes; the role already grants `PutItem`.
- **`apps/web`** owns the button, the countdown and the three endings on screen. The
  countdown is `remaining`, a free function over `(seconds, when)` and `now`, for the same
  reason `screen_for` is one: the arithmetic is the part that can be wrong.
- **The lobby unit** owns what a listing contains and which matches appear in it. This unit
  owns only the expiry that happens during that pass. Both touch `open_matches`, and the
  lobby document is where its final shape is written down.

## Decisions

**A completion is counted, not asserted.** Argued above: one client's word would let a
losing player end a match, and the delay that counting costs is collected by the hour rule.

**`now` is passed in rather than held.** A clock dependency would be a third type
parameter on `Relay` or a boxed trait object, for a value that belongs to one message.
Passing it keeps the relay stateless between messages in the literal sense the
specification claims.

**The time lives on the row, not in the event.** The fold cannot see what it is not given.
The cost is that `Log::read` returns a struct rather than a `Vec`, which is a mechanical
change at four call sites.

**Expiry is lazy.** A scheduled sweep would be a second deployable, an IAM role, a
schedule to get wrong, and a thing that runs when nobody is looking. The lobby pass already
reads every match, so the check is free where it is and the write happens only on the day a
match crosses the hour. The cost is that `list` is no longer read-only.

**An untimed row means inactive since forever.** The alternative is *unknown, so not
expired*, which would leave every match that predates this unit in the lobby permanently —
which is the exact problem this unit exists to fix. The four dead matches in production
will be deleted by hand as well, so both the old and the new path are exercised.

**A refused cancel is not an event.** Otherwise pressing the button resets the minute it is
waiting for.

**Inactivity is the match's, not the player's.** One number, no judgement about fault. The
seam is that another seat's event resets the host's minute, which nobody will notice
without trying to.

## Determinism

**The clock is an input at the composition root, like a keystroke.** `now` arrives as an
argument to `receive`; the service reads the system clock once per invocation and passes
it. Nothing derives a value from it and puts it in the log: the three terminal events carry
no time, and the `recorded` attribute is never part of an event.

**No match state derives from a recorded time.** `MatchView::fold` takes `&[Event]` and
always will, so this is enforced by what the function is handed rather than by care. That
is the statement `The time the relay received an event decides nothing but whether a match
is inactive`, made structural.

**Two honest wrinkles, both pre-existing in kind.** Whether `Complete` appears depends on
who is connected, exactly as a reveal already does. And whether `Abandon` appears depends on
wall-clock time, so the same sequence of messages replayed an hour apart produces different
logs. Neither affects a replay: given a log, every reader derives the same match, which is
what the architecture statements actually require.

Tests pass `Moment`s. Nothing sleeps, and the hour is reached by arithmetic.

## Testing

- A host cancels a match that has not started, at once, with no refusal.
- A host cannot cancel a started match 59 seconds after its last event, and the refusal
  says one second remains.
- Rounding up: 59 and a half seconds elapsed says one, not zero.
- A refused cancel does not reset the minute — refused at 30, refused at 59, allowed at 60,
  with no event recorded in between.
- A player who is not the host cannot cancel, before or after the start.
- A match untouched for an hour is abandoned, and the listing that found it does not show
  it.
- The sweep is idempotent: two listings, one `Abandon` event.
- A match whose rows carry no recorded time is abandoned on the first listing.
- A match that is already over is not abandoned, and not cancelled — the hour rule and the
  host's button both bounce off `No event is recorded in a match that is over`.
- A claim, a join, a start and a leave into an over match are each refused.
- A join into a cancelled match is refused with `Cancelled`, and a resume into one succeeds
  and reports the ending.
- A host who leaves before the start cancels the match, which replaces the `HostHasLeft`
  test rather than adding to it.
- Completion needs every present seat: with one seat present of two, saying it is over
  records `MatchOver` and no `Complete`; the second seat's word records both.
- A whole match through `relay::memory` ending in a real `Complete`, which is the test that
  proves the clients and the count agree about when a match is finished.
- Round trips through `wire`, both directions, for the four events and the two refusals,
  including `StillActive`'s number.
- Raced against DynamoDB Local: two connections list simultaneously over a table of expired
  matches, and exactly one `Abandon` lands per match. Run repeatedly, as the connection
  tests are.

## Open Items

**The minute and the hour are constants in one place and nothing configures them.** They
are the kind of number a real player will have an opinion about, and the opinion is cheap to
act on when it arrives.

**A finished match lingers for up to an hour** if everybody closed their tab at the moment
of victory, because the count never completes and the hour rule is the backstop. Visible
only in the lobby, and only as a match nobody can join.

**The lobby's scan cost is untouched.** Abandoning a match stops it being listed; it does
not stop its rows being scanned. `relay-service.md`'s open item about a projection survives
this unit intact.

## Status
Built.
