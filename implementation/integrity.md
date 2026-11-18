# Dice Integrity: Commitment, Derivation, and Reveal

## Realizes
- [Dice Integrity / Verifiability](/specification/dice-integrity.md) — every die fixed
  before any claim, and the log sufficient to recompute them
- [Dice Integrity / Rolls](/specification/dice-integrity.md) — no player chooses or
  influences their roll, and none alters it afterwards
- [Dice Integrity / Secrecy](/specification/dice-integrity.md) — secret until the round
  resolves, then revealed
- [Dice Integrity / Trust](/specification/dice-integrity.md) — the relay is trusted to
  deal fairly
- [Architecture / State](/specification/architecture.md) — the relay holds nothing between
  messages except what the log holds or what derives from it plus a long-lived secret

## Approach

Commit, deal, reveal. Three steps per round, and the middle one is the only one anybody
sees while the round is live.

### The long-lived secret is the whole design

`The relay holds no game state between messages except what the log holds, or what derives
from the log plus a long-lived secret` is not a restriction here, it is the mechanism. A
relay that stored per-round seeds would have state to lose; a relay that *derives* them
has none.

```
seed    = HMAC-SHA-256(secret, match_id || round_number)
digest  = SHA-256(seed)
```

The secret is one 32-byte value for the deployment, held in SSM as a `SecureString` and
read by the relay's execution role. A replacement relay with the same secret recomputes the
same seed for the same round, which is precisely why `A round in progress survives a relay
restart` can be true of a process that keeps nothing.

### Faces, without modulo bias

A seat's dice come from a keyed stream, not from the seed directly, so one seat's faces
reveal nothing about another's:

```
stream  = HMAC-SHA-256(seed, seat || counter), counter rising from zero
```

Each byte of that stream becomes a face by rejection sampling: **a byte below 252 gives
`byte % 6 + 1`; a byte of 252 or above is discarded** and the next one taken. 252 is the
largest multiple of six that fits in a byte, so every face is exactly equally likely.

The naive `byte % 6` is biased: 256 is not a multiple of 6, so faces one to four come up
on 43 of the 256 byte values and faces five and six on 42. That is a 2.4% edge on low
faces, in a game whose whole premise is counting faces. Rejection sampling costs an
expected 1.6% extra bytes and removes it completely.

This is integer arithmetic throughout. A face determines the content of an event, so
nothing here may touch floating point, and nothing does.

### The order of the log

```
commit <digest>          ← before any claim in the round
deal #0 5 3 6 5 3        ← one per seat
deal #1 3 5 5 1 3
claim #2 1 3 100.00%
...
reveal <seed>            ← when the round resolves
```

`Every die in a round is fixed before any claim in that round` is the position of the
commit line. `Dice are secret only until the round resolves` is the position of the reveal
line.

**Secrecy is enforced by withholding, not by omission.** The log holds every seat's deal
from the moment it happens, because the log is the relay's record of what it did. What the
relay *sends* each client during a round is that client's own deal and nothing else; the
other seats' deal lines are released to everyone when the round resolves, together with
the reveal. The archived log of a finished match therefore reads in true order and hides
nothing, while no player learned anything early.

### Why log the deals when the seed derives them

The protocol's standing rule is that nothing derivable is written down, and after the
reveal every deal line *is* derivable from the seed. They are written anyway, deliberately,
and this is the third exception alongside `winner` and the luck lines.

The reason is that `A player can detect dice that disagree with the event log` needs two
things to compare. If the log held only the seed, the derived dice would be correct by
definition and the only disagreement possible would be between the log and a message a
player received during the round — which is not in the log, so verification would require
something beyond it, contradicting `Verification requires nothing beyond the event log`.
With both present, the log checks itself: derive from the seed, compare against the deal
lines, and a tampered deal is visible to anyone holding the file.

### The desktop game uses the same path

A desktop game has no relay, so it generates a random secret per match and runs the same
commit-deal-reveal. Keeping one code path means the format is uniform, the archive of a
desktop match verifies by the same procedure, and the scheme is exercised constantly rather
than only in production. It proves nothing about a player who edits their own binary, and
it is not meant to: in a single player game the only person cheated is the cheat.

## Boundaries

`crates/integrity` is pure: given a secret, a match, a round and a seat, it produces a
seed, a digest and faces, and it verifies a digest against a seed. No I/O, no clock, no
knowledge of rules, claims or players.

Depends on `sha2` and `hmac` from the ecosystem. **We do not implement SHA-256 here.**
Hand-rolling a primitive is how this kind of code goes quietly wrong, and
`Dependencies are kept at their latest released versions` already says the project takes
dependencies when they are the right answer.

Depended on by the relay, by `apps/minimal` for its own matches, and by the data browser
when it verifies an archived log.

## Decisions

**Derive the seed rather than store it.** The alternative is a seeds table, which is state
to lose, to migrate and to leak. The specification's long-lived-secret clause exists for
exactly this and nothing else uses it.

**Rejection sampling over modulo.** Explained above. The bias is small, the game counts
faces for a living, and the fix is six lines.

**The commitment buys detection, not prevention — and this is a real limit.** A relay that
wanted to cheat could derive several candidate seeds, look at the dice each produces, and
commit to the one it liked. The commitment stops alteration *after* publication; it does
not stop choosing *before*. Preventing that needs every player to contribute to the seed,
with their contributions committed first — several extra round trips per round. We are not
doing that, because `The relay is trusted to deal fairly` and `the adversary is other
players, not the operator`. This is written down so that nobody later reads the commitment
scheme as a stronger promise than it is.

**A trait for dealing a whole seat, replacing `Rolls`.** `rules::Rolls` hands out one face
at a time and `roll` calls it five times per seat in seat order, so an integrity-backed
implementation would have to produce faces in exactly the order `rules` happens to consume
them — a coupling that nothing states and a refactor could silently break.

```rust
pub trait Deals {
    fn seat(&mut self, seat: usize) -> [u8; DICE_LIMIT as usize];
}
```

This makes the seat explicit, so the derivation is keyed by what it is actually keyed by.
It is a second change to `crates/rules`, which is why it is called out rather than slipped
in: the scripted test sources become scripted per seat, which is also more readable than
counting faces off a cycling list.

## Determinism

The scheme is the non-determinism, and it is injected as `Deals`, the same way `Rolls` was.
A simulated environment supplies a fixed secret and gets a fixed match, so every networked
scenario is reproducible from one 32-byte value.

Integer arithmetic only. The secret, the seed, the digest and the faces are bytes; no value
here is ever rendered for a person, so the floating point side is not involved at all.

## Testing

- A face distribution over a large sample is flat to within tolerance, and the same test
  fails against a deliberately biased `byte % 6` implementation. The bias is the thing being
  tested, so the test must be able to see it.
- The same secret, match and round produce the same seed, digest and faces, across process
  restarts. This is the relay-restart property stated as a test.
- Different rounds, different matches and different seats all produce unrelated faces.
- A digest verifies against its seed and against no other seed.
- A tampered deal line is detected: derive from the revealed seed, compare, and disagree.
- Knowing one seat's five faces does not narrow the seed — tested as a property rather than
  proved, by checking that a seat's faces are independent of every other seat's across many
  seeds.

## Status
Built. `crates/integrity` is 10 tests; `apps/minimal` deals through it and writes `commit`
and `reveal` lines around every round. Workspace: 34 test targets green, clippy clean with
warnings denied, format clean.

Verified end to end, twice. `an_archived_match_verifies_from_the_log_alone` plays a whole
match, writes the log, then re-reads it holding nothing but the text and checks every
round's seed against its commitment and every deal line against the seed. Separately, a
Python implementation written from this document alone verified a real 14-round log, and
reported the exact seat and both values when one face was altered. The second check is the
one that matters for `Verification requires nothing beyond the event log`: a reader with
the file and the format, and no Rust, can audit a match.

`PROTOCOL_VERSION` is 2.

`Playing` no longer holds the PRNG. Once the dealer carries the match, the only things the
PRNG decides are the turn order and the match secret, both at match start, so the field
became dead and was removed rather than silenced.
