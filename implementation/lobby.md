# Lobby

What a player sees when they arrive: every match that is going on, which of them they can
join, and the one they are already in — which they are put back into without being asked.

## Realizes
/specification/multiplayer.md — Joining, the listing statements and the one-seat-at-a-time
statements; Hosting, `A player cannot host a match while holding a seat in another match`.
/specification/glossary.md — In progress.

## Approach

### The listing widens, and says more about each match

`Every match that can be joined is listed` was the whole rule, so the relay skipped every
started match and the lobby was a list of waiting rooms. `A started match is listed` and
`A match that is over is not listed` change what the pass selects, and two more statements
change what each row carries:

```rust
pub struct Listed {
    pub match_id: MatchId,
    pub host: String,
    pub players: usize,
    /// Whether the asking account could join it right now.
    pub joinable: bool,
    /// Whether the asking account already holds a seat in it.
    pub mine: bool,
}
```

`A match that is over is not listed` needs the relay to know a match ended, which is what
`ending.md` records. That is the one dependency between the two units, and it runs this
way: this unit reads `view.over()` and writes nothing about endings.

### The listing and the refusal cannot be allowed to disagree

This is the trap in `A listed match shows whether it can be joined`. Joinability would be
computed twice — once to set the flag, once in `join`'s guards — and two copies of a rule
are two rules that will eventually say different things. Then the lobby offers a match that
refuses you, or greys out one that would have let you in, and the bug is in neither place.

So there is one function and both callers use it:

```rust
/// Why this account cannot join this match, or `None` if it can.
fn why_not_joinable(view: &MatchView, asking: Option<&Account>) -> Option<Refused>
```

`join` returns what it returns; the listing sets `joinable: why_not_joinable(..).is_none()`.
The flag is then the refusal, rather than a second opinion about it.

It covers the match-level guards — over, started, full, barred, and a seat already held in
another match. The colour guard stays inside `join`, because a colour is part of the request
and not a property of the match, so there is nothing for a listing to say about it. That
boundary is worth stating precisely: a listing promises that the match will take you, not
that it will take you in green.

`mine` is `view.seat_of_account(asking).is_some()`, and `list` already carries the asking
account, so it costs nothing.

### Coming back to the match you are in

`A client returns its player to the match they hold a seat in, without asking`. The
mechanism already exists on both sides and has never been wired together: `Message::Resume`
is implemented in the relay and unused by any client, so closing a tab currently loses the
seat.

Now the opening `list` answers the question. The client sends its lobby ask as the first
frame on the socket, as it already does; if the listing contains a row with `mine`, it
sends `Resume` for that match instead of drawing the lobby. `screen_for` needs no change at
all — the stance becomes `Some`, and the waiting, playing, watching and over screens follow
from the four facts they already follow from.

No new message, no second round trip, and nothing to keep in sync: the one pass that already
reads every match is the pass that knows which one is yours.

**This unit cannot be built before identity.** The return is only as good as the account,
and `apps/web` today derives the account from the typed name — so on the published page
this rule would return a player to the seat of whoever last typed the same name. `Account::
Guest` with an id in local storage is what makes a page load recoverable, and that is
`identity.md`'s work.

### One match at a time

`A player cannot join a match while holding a seat in another match`, and the same for
hosting. The relay has to answer *which match is this account in*, which the log can answer
and nothing else can — the connection table knows only who is connected, and a seat is held
whether or not anybody is attached to it.

```rust
/// The match this account holds a seat in, if any. A match that is over holds nobody.
fn match_of(&self, account: &Account) -> Option<MatchId>
```

Refused as `AlreadyInAMatch { match_id }`, carrying the id so the client can offer to take
you there rather than leaving you to find it.

A match that is over must not count. Otherwise finishing a game would lock you out of the
next one until you remembered to leave the old one, which is a worse trap than the one this
statement closes.

**And it has to be a query, not a scan.** Hosting and joining read one match today. Asking
the log which match holds an account means asking about every match, and a full table scan
is exactly the cost `relay-service.md` already measures and calls *"the reason the lobby has
a cost worth measuring"*. Putting the hot path on the known-slow path, knowingly, is not a
decision to defer to a later measurement — the measurement is already in.

So the log table gains a global secondary index keyed on `account`. Only `join` rows carry
that attribute, so the index is sparse and holds one row per seat ever taken. A query
returns the handful of matches an account has ever joined; folding those few answers whether
any is still live. Adding a GSI is an in-place table update, not a replacement, so no data
moves.

The index is not a second source of truth, which is what ruled out an account-keyed table
earlier: DynamoDB derives it from the log's own rows, so it cannot disagree with the log any
more than a sorted view of a list can disagree with the list.

### What this does not promise

`A player cannot join a match while holding a seat in another match` promises the refusal.
It does not promise that a player never holds two seats, and the difference is not pedantry.

Position-as-version orders writes within one match's log. Two joins in flight to two
different matches contend on no shared key, so there is no conditional write that can make
the second fail — this is the same check-then-act shape that the log closed with position
and the connection table closed by putting the seat in the key, and here there is nothing to
put in a key. Making it impossible would mean an account-keyed row written in the same
transaction as the join events, which would put *who is in what match* somewhere other than
the log.

What the race costs is one match the player is not playing, and `ending.md`'s hour rule
cancels it. That is a self-healing cost, where the double-seat-in-one-match defect was
permanent and in the log. The statement says what can be guaranteed, and this document says
what cannot.

### On screen

The lobby grows a second group. Matches you can join keep their button; matches in progress
are listed with their host and player count, no button, and the reason — *in progress*, or
*full*, or *you are in another match*, which is the refusal's own text rather than a second
wording of it.

"In progress" is the display word for a started match that is not over. The statements and
the code say *started*, because that is the word the specification already uses everywhere
and two words for one thing means one of them is wrong. The glossary carries the mapping so
the screen can read naturally without the vocabulary forking.

## Boundaries

- **`crates/relay`** owns `Listed`'s fields, `why_not_joinable`, `match_of`, and
  `Refused::AlreadyInAMatch`. It owns which matches the lobby pass selects; `ending.md` owns
  the expiry that happens during the same pass.
- **`crates/wire`** carries the two new `Listed` fields and the new refusal. `Value::Many`
  already renders a listing, so the shape is there; the fields are additions to
  `listed_shape` and `listed_of`.
- **`crates/seat`** parses the wider listing in `lobby::listing`, and gains nothing else —
  the return is a `Resume` the client already knows how to send.
- **`apps/relay`** queries the new index. **`deploy/relay.yaml`** declares it.
- **`apps/web`** owns the two groups on the lobby screen and the decision to resume instead
  of drawing it.
- **Depends on `identity.md`** for an account that survives a page load, and on `ending.md`
  for `view.over()`. Nothing depends on this unit.

## Decisions

**Joining elsewhere is refused, not resolved by leaving for you.** Leaving is an elimination
in the rules and goes in the log permanently, so a misclick must not cause one. The client
is told which match you are in and offers to take you there; leaving stays something done on
purpose.

**One function decides joinability.** The listing's flag is the refusal rather than a second
opinion about it, which is the only version of this that cannot drift. The colour guard is
deliberately outside it, because a colour is part of a request and not a fact about a match.

**An index, not a scan.** Every host and every join would otherwise pay the lobby's cost,
including the common case of discovering that you are *not* in a match, which is the answer
that requires looking everywhere. The index is derived by the database from the log's rows,
so it adds no second truth — only deploy surface.

**A match that is over holds nobody**, so finishing a game does not lock you out of the
next.

**The race is left open and documented.** Closing it needs a row keyed by account, written
transactionally with the join, and that is a second place where *who is in what* lives. The
harm is one unplayed match, which expires.

**The listing carries flags, not a reason enum.** `joinable` plus the refusal the client can
already render is enough; a parallel `why` field on `Listed` would be a third copy of the
same rule.

## Determinism

Nothing new. A listing is a view, and `A view derived from the event log can change` —
`list` is already documented as the one answer that cannot be stale because it invalidates no
decision. The return-to-your-match path reads an id out of a listing and sends it; the guest
id that makes it stable is `identity.md`'s one new non-determinism, taken once at the
composition root and stored.

The expiry that now happens inside the lobby pass is `ending.md`'s, and its determinism is
argued there.

## Testing

- A started match is listed, and marked not joinable.
- A match that is over is not listed at all.
- A full match is listed and not joinable; a waiting match with room is both.
- A barred account sees neither the match nor a hole where it was.
- **`joinable` agrees with `join`** over every match shape the tests can build: for each,
  assert that the flag and the actual refusal say the same thing. This is the test that pays
  for the one-function design, and it fails if anybody adds a guard to one side only.
- `mine` is set on exactly the asker's match, and on no row of a listing asked for by
  somebody else.
- A join is refused `AlreadyInAMatch` while the account holds a seat elsewhere, and the
  refusal names the match.
- A host is refused the same way.
- Leaving the first match and then joining the second works.
- After the first match is over, joining a second needs no leave.
- A client with a seat in a match resumes it from a listing rather than drawing the lobby,
  tested over `relay::memory` as a whole sequence: list, resume, play.
- The race is tested for what is promised rather than what is not — two accounts joining two
  matches concurrently against DynamoDB Local, asserting that the refusal fires whenever the
  log already showed a seat, and that when it does not, both seats exist and the unplayed one
  expires.
- The listing round-trips through `wire` with both new fields, and the new refusal in both
  directions.

## Open Items

**Nothing watches a match in progress.** A started match is visible in the lobby and not
joinable, and that is all; spectating is a different thing and is not in the specification.

**The AI player button is still not built.** `identity.md` owns what a bot is, and
`crates/seat/tests/bots.rs` proves a host can play a whole match against two of them, each
on its own socket. The screen that adds one belongs to `multiplayer-client.md`.

**The lobby still scans the whole table.** This unit adds an index for *which match holds
this account*, not for *which matches exist*. `relay-service.md`'s open item about a
projection for the second question is untouched, and is now the only scan left on the path.

## Status
Built.

Two things came out of building it that the document did not say.

**The client resumes rather than re-joining.** `Message::Resume` was implemented in the
relay and sent by nothing, which is how closing a tab lost a seat; it is now `seat`'s third
`Opening`, with no colour, because a returning player is not choosing one — their seat
already has the colour the log recorded. Joining again would have worked, since a join from
an account that holds a seat returns it, but it would have meant inventing a colour nobody
uses to ask a question the log has already answered.

**The page also needs a keepalive, which belongs here by consequence.** API Gateway closes
a WebSocket after ten minutes with no traffic and every connection at two hours, and
neither quota can be raised. Resuming makes a drop survivable; it does not make one rare.
So `apps/web` sends an empty frame every four minutes — which the relay was already written
for: a bodyless frame is *"not a message... closing the socket over it would punish a
keepalive."*

The two-hour cap is why this unit mattered more than it looked. A drop is not an edge case,
it is a certainty on any session longer than two hours, and before this the only recovery
was the host re-sharing the match id.
