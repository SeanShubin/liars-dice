# Rules Engine: Dice Limit, Shield Tokens, and the Affected Player

## Realizes
- [Game Rules / Game Mechanics](/specification/game-rules.md) — the five dice limit
- [Game Rules / Shield Tokens](/specification/game-rules.md) — holding, gaining, spending
- [Game Rules / Spot On](/specification/game-rules.md) — a win at the limit gains a shield
- [Game Rules / Exchanges](/specification/game-rules.md) — exactly one player is affected
- [Dice Integrity / Rolls](/specification/dice-integrity.md) — dealing the limit to every seat

## Approach

Three changes to `crates/rules`, which is otherwise sound. They are described in dependency
order: the third is only safe once the second exists.

### 1. The dice limit and shield tokens

`Match` gains `shields: Vec<u32>`, parallel to `counts`. Two places in `resolve` change,
and only those two, because they are the only places a dice count ever moves.

A gain, which today is unconditional:

```rust
if let Some(player) = gained {
    self.counts[player] += 1;          // no cap
}
```

becomes a gain *or* a shield: at the limit the die becomes a token instead. This is what
`A correct claim at the dice limit gains a shield token` means operationally, and it is
what keeps `A player holds at most five dice` true without a separate check anywhere.

A loss, which today is unconditional:

```rust
if let Some(player) = lost {
    self.counts[player] -= 1;          // no shield
}
```

becomes: if the player holds a shield, spend one and the count does not move; otherwise
lose the die as now. `A shield token prevents its holder's next die loss` and
`A shield token is spent when it prevents a die loss` are the same sentence read twice,
and a single `if` satisfies both.

Elimination is unaffected in its logic but changes in its reach: a player on one die with
a shield survives a loss they would previously not have. That is the point of the token.

### 2. The affected player

This is not a feature. It is a **panic that the approved statements make reachable.**
`resolve` currently reads:

```rust
let changed = gained
    .or(lost)
    .expect("a resolution always changes someone's dice count");
```

That expectation holds today because every resolution moves exactly one count by one. It
stops holding the moment change 1 lands, in two separate ways:

- a spot on won at the limit grants a shield, so nobody's count changes
- a loss absorbed by a shield spends the token, so nobody's count changes

Either one reaches the `expect` and panics the rules engine mid-match. The fix is the
statement: `Exactly one player in an exchange is affected by its resolution`, where
affected means *lost a die, gained a die, gained a shield, or spent a shield*. The
derivation `gained.or(lost)` is replaced by an `affected: usize` computed where the
resolution is already known, and `opens` is derived from it exactly as before, including
the existing fallback when the affected player was eliminated.

`Outcome::Resolved` gains `shield_gained: Option<usize>` and `shield_spent: Option<usize>`
alongside `lost` and `gained`, so a caller can render the exchange and the protocol can
record it. `affected` is reported too rather than left to be re-derived, because one of the
four cases is not recoverable from the other fields alone.

### 3. Dealing the limit to every seat

`Every round deals the dice limit to every seat` and `A player uses as many of their seat's
dice as they hold` exist so the relay can deal without knowing the rules — it would
otherwise have to replay every resolution to learn how many dice each player holds, which
is precisely the knowledge `The relay knows no game rules` denies it.

`roll` therefore fills every seat with five faces regardless of what that player holds.
The held dice are the **prefix**: the first `counts[player]` of them.

This is a trap, and the trap is not in `roll`. Two readers of `rolled` currently assume its
length *is* the count:

- `dice_of` returns the whole of `rolled[player]`, which would expose dice the player does
  not hold — a secrecy leak, not merely a wrong answer
- `truth` sums `count_face` over the whole of `rolled[player]`, which would count dice that
  are not in play and silently corrupt every resolution

Both must slice to `counts[player]`. Nothing else reads `rolled`, and the slicing belongs
in those two accessors rather than at the call sites, so a third reader added later
inherits the correct behaviour instead of repeating the bug.

## Boundaries

Owns the rules of a match: legal claims, resolution, elimination, ranking. Depends on
nothing but `Rolls`. Depended on by `ai` and `apps/minimal`, and in due course by the
client half of the networked game. `odds` depends on neither this crate nor anything
else — it answers a question about dice, not about matches — so none of this reaches it.

Unchanged at the seam: `Rolls` stays the single source of non-determinism, and the only
thing that changes about it is that `roll` now asks for a fixed five faces per seat per
round rather than a variable number. That makes the call pattern *more* predictable, which
the simulated environment benefits from.

Not in this unit of work: the protocol's seat identifiers and version field, the integrity
scheme, and the relay. Change 3 is what those depend on, which is why it is here and not
with them.

## Decisions

**Shields are a count, not a boolean.** `A player may hold any number of shield tokens`
leaves no choice, and a `u32` beside `counts` costs nothing. A boolean would also make
`Every player can see how many shield tokens each player holds` unanswerable.

**The cap lives at the point of gain, not as an invariant check.** A spot on win is the
only way a count rises, so capping there makes `A player holds at most five dice` true by
construction. An assertion elsewhere would be a second place for the same fact to live.

**`affected` is reported rather than derived.** A caller given `lost`, `gained`,
`shield_gained` and `shield_spent` could recover the affected player in three of the four
cases; reporting it directly removes the fourth case and the temptation to re-derive it
differently in two renderers.

**The dice limit is a constant, not a parameter.** `Match::start` currently takes
`dice_each: u32`. The specification states five twice — as the starting count and as the
holding limit — and they are the same five. A parameter would allow a match to start above
its own limit.

## Determinism

`Rolls` is the only non-determinism and it is already injected. The change makes the number
of calls per round constant at five times the number of seats, independent of the match
state, which removes a way for a replay to drift: a test that scripts faces no longer has
to know how many dice each player held at that point in the match.

No new floating point. Shield counts and dice counts are integers, they determine the
content of events, and they stay on the exact side of the boundary.

## Testing

Unit tests for each case in isolation: a spot on won below the limit gains a die, a spot on
won at the limit gains a token, a loss with a token spends it and keeps the die, a loss
without one takes the die, and a player on one die with a token survives.

Property tests for the invariants, which is what `D5` asked for:

- exactly one player is affected by every resolution — the direct falsifier for the panic
- no player ever holds more than five dice
- a true claim is always available to the current player
- the total of counts never rises except by a spot on won below the limit
- `dice_of` never returns more faces than the player holds, over every reachable state

The third and fifth are the ones worth having even though they are not new behaviour: the
third is a design principle with no test today, and the fifth is the secrecy leak that
change 3 introduces if the slicing is missed.

## Status
Built. `crates/rules` carries the dice limit, shield tokens and the affected player; the
suite is 23 tests, clippy is clean on the crate, and the workspace builds and passes.

Two consequences landed outside this crate:

- `apps/minimal` narrates a gained and a spent shield token, and no longer passes a dice
  count to `Match::start`.
- `crates/protocol` has no event for a shield token gained or spent, so a desktop audit
  log records the resolution without them. That belongs to the protocol unit of work,
  which is rewriting those events to carry seats anyway.
