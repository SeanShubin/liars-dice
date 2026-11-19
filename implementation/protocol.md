# Protocol: Seats, Versions, and Shield Tokens

## Realizes
- [Architecture / Authentication](/specification/architecture.md) — an event refers to a
  player by their seat, and a join event records the Discord account id
- [Multiplayer / Versions](/specification/multiplayer.md) — a match records its protocol
  version, and join and load both check it
- [Game Rules / Shield Tokens](/specification/game-rules.md) — gained and spent tokens are
  events like any other
- [Dice Integrity / Rolls](/specification/dice-integrity.md) — every round deals the dice
  limit to every seat

## Approach

`crates/protocol` is a line-oriented log: one event per line, a command and its arguments.
The module already argues for writing only what cannot be derived, and nothing here
weakens that. Four changes.

### 1. A seat replaces a name, everywhere but one line

Every one of the fifteen event variants carries `player: String`, and that string is a
display name. `An event refers to a player by their seat` makes all fifteen wrong.

```rust
pub struct Seat(pub u8);
```

A newtype rather than a bare integer, because a log line is otherwise three bare numbers
in a row — `claim 3 1 3` is seat, quantity, face — and nothing stops them being
transposed. `Seat` renders as `#3`, so the same line reads `claim #3 1 3`. One character
of format buys an unambiguous column and a type that cannot be confused with a face value.

`u8` because `A match has at most ten players`. `crates/rules` keeps `usize` because it
indexes vectors with it; conversion happens at this boundary and nowhere else.

### 2. The join event binds a seat to an account

`AddPlayer { name, colour }` becomes the one line in the log where a name appears:

```rust
Join { seat: Seat, name: String, colour: String, account: Account }

pub enum Account {
    /// A web player, authenticated with Discord.
    Discord(String),
    /// An AI player. A desktop game only.
    Ai,
    /// The human at the keyboard of a desktop game, which has no authentication.
    Local,
}
```

Three cases, because there really are three. `A desktop game has no authentication`, so the
local human has no account id; an AI player has no account at all; a web player has a
Discord snowflake. A single optional id would collapse the second and third into "absent"
and lose the distinction a reader needs to know what kind of match they are looking at.

This is what makes `A player's name is recorded once, when they join a match` and
`No value derives from a player's name` true of the format rather than of a convention:
after the join lines, a name cannot appear, because no other variant has a field for one.

### 3. A version line, first

```rust
Version { protocol: u32 }
```

Emitted before `started`, so a reader knows how to read everything after it.
`A match records the protocol version of the client that hosted it` lives here;
`A client cannot join a match whose protocol version differs from its own` and the same
for loading a save are checks the relay and the loader make against this value.

T5 asked whether logs need versioning and the answer falls out rather than being imposed:
a client cannot join or load across a version boundary, so no match can span two versions
and no log can contain two. One line at the top says which, and old logs are never
migrated because nothing can read them into a running match anyway.

### 4. Shield tokens, and dealing rather than rolling

Two new variants beside `LoseDie` and `GainDie`:

```rust
GainShield { seat: Seat }
SpendShield { seat: Seat }
```

`crates/rules` already reports these as `shield_gained` and `shield_spent`; without the
events, a desktop audit log records a resolution in which nothing happened.

`Roll { player, dice }` becomes `Deal { seat, dice }` carrying all five faces, because
`Every round deals the dice limit to every seat`. The held dice are the prefix and the rest
are not in play — both derivable from the counts a replay is already tracking, so the log
records what was dealt and nothing about what was held.

### What a log looks like afterwards

```
version 1
started 2026-11-17T06:51:36.139Z
join #0 You green local
join #1 Alice magenta ai
join #2 Bob white ai
player-order #2 #0 #1
variant ones-wild off
deal #0 5 3 6 5 3
deal #1 3 5 5 1 3
deal #2 4 2 3 3 3
claim #2 1 3 100.00%
claim #0 3 5 93.51%
liar #1 62.00%
lose-die #0
next-turn #0
```

## Boundaries

Owns what an event is and how it reads. Depends on nothing. Depended on by `apps/minimal`
today and by the relay, the clients and the data browser in due course.

Writing a log is an effect and stays in a composition root, as now. This crate gains no
I/O.

Not in this unit of work: the integrity scheme's commitment and reveal events. They will
add variants and bump `Version`, which is exactly what the version line is for.

## Decisions

**A newtype for the seat, not a type alias.** An alias would still let a face value be
passed where a seat belongs. The log format and the type system get the same guarantee from
one declaration.

**Three account cases rather than an `Option`.** An absent id would mean both "AI" and
"desktop human", which a reader of a log cannot tell apart and which the data browser would
have to guess at.

**The odds stay, and stay unverified.** In a networked match the odds on a claim arrive
from the claiming client, which could send anything. That is tolerable only because
`No value crosses from the floating point side to the exact side` — a false odds figure
corrupts a column a person reads and can never change a resolution. The relay records what
it received, which is what `The relay records every message it receives` requires anyway.
Worth stating because it looks like a hole and is not one.

**The log gets less human-readable, deliberately.** `claim #2 1 3` names nobody, where
`claim Carol 1 3` did. The join lines at the top are the key, and the data browser renders
names from them. The trade is forced: a name is mutable, a Discord display name can change
between two matches, and a log keyed on names would disagree with itself over time.

**No migration of existing logs.** `logs/` is gitignored and holds four local files from
development. Nothing reads a log back yet, so there is no compatibility surface to keep.

## Determinism

Nothing here touches the exact side except by making it narrower: a seat is an integer, a
version is an integer, and the one float in the format is still the odds, still written for
a person, still never read back.

The format remains the only representation a person reads, and the only one verification
uses, which is what makes `Verification requires nothing beyond the event log` achievable
rather than aspirational.

It is no longer the only representation. That sentence rested on `Nothing reads a log back
yet`, above, and the deployed relay reads its log back on every message; it also sends
events over a socket and receives messages from one. Three machine representations were
the risk, so there is one decomposition in `crates/wire` and the renderings of it are
mechanical — see `relay-service.md`. The line format is deliberately not one of them: it
rounds odds to two places and collapses the spaces in a name, which is correct for a line
somebody reads and wrong for the only copy of what happened. `the one float in the format
is still the odds, still written for a person, still never read back` holds of the archive
and no longer of the log.

## Testing

The existing display tests change shape: every assertion naming a player now names a seat.

New tests, each falsifying a statement directly:

- A name appears on a join line and on no other line, over every variant. This is the test
  for `No value derives from a player's name`, and it fails loudly if a future variant
  adds a name field.
- Every variant that refers to a player renders it as `#N`.
- A version line precedes everything, including `started`.
- A deal line carries exactly five faces.
- Round trip: the account cases render distinguishably and are not confusable with a name
  or a colour.

## Status
Built. `crates/protocol` carries seats, the version line, the three account cases and the
two shield events; 17 tests, clippy clean, workspace green.

One addition beyond this document, made because writing the log needed it:
`crates/rules` gained `dealt_of`, returning every face dealt to a seat rather than only
what its player holds. `Deal` records the whole seat, and `The event log is sufficient to
recompute every die of every round` is about what was dealt, which is also what the
integrity scheme will attest to. Its doc comment says plainly that it is for the log and
for verification, never for display or counting, because `dice_of` is the accessor that
keeps the undealt dice out of play.

The four logs in `logs/` are in the old name-keyed format. They are gitignored local
development files and nothing reads a log back, so they are left alone rather than
migrated or deleted.
