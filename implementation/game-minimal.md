# Minimal Game

## Realizes
/specification/game-minimal.md, in full.
/specification/ai-opponents.md, in full.
/specification/game-rules.md — the rules the match is played by.
/specification/dice.md — values and identity.
/specification/architecture.md — thin composition root, consolidated non-determinism,
floating point and transcendental functions kept out of the model.

## Crate placement

```
crates/rules/       legal claims, resolution, match state. Pure.
crates/odds/        the probability model. Pure, no rules knowledge.
crates/ai/          the decision rule. Depends on odds and rules.
crates/render-minimal/  the two dimensional presentation
apps/minimal/       composition root
```

`apps/local` stays what it was for: running the networked game in one process against a
simulated environment. The minimal game is a product rather than a harness, so it gets its
own composition root.

`crates/odds` knows nothing about rules or turns. It answers one question about dice, and
both the AI and the human's readout ask it — the human sees exactly what an AI in their
seat would compute, which is also the fairness property that makes the readout honest.

## The probability model

Every probability in the game is one question: **given the dice I can see, how likely is it
that at least `wanted` of the remaining dice show a particular face?**

Let `known` be how many of the asker's own dice already count toward the face, `unknown`
the number of dice the asker cannot see, and `needed = wanted - known`. If `needed` is zero
or less the claim is already true. Otherwise it is a binomial tail:

```
P(at least needed) = sum over i from needed to unknown of
                     C(unknown, i) * p^i * (1-p)^(unknown-i)
```

`p` is the chance one unseen die counts toward the face:

| Case | p |
|------|---|
| Ones not wild | one sixth |
| Ones wild, face is not the one face | one third |
| Ones wild, claim names the one face | cannot happen — such a claim is illegal |

With ones wild the asker's own ones also count toward `known`.

**Spot on** asks for exactly rather than at least, so it is one term of the same sum.
**Liar** asks for the complement: one minus the probability the previous claim is true.

### Why this does not break the determinism statement

The architecture requires that floating point rounding and transcendental functions stay
out of the model. A binomial tail needs neither. It is built from addition, subtraction,
multiplication and division only, which IEEE 754 specifies exactly, so the same inputs
produce bit-identical results on every platform. Nothing here calls `exp`, `log`, `pow`,
or a gamma function, and binomial coefficients are accumulated by integer multiplication
and division rather than computed from factorials.

That is a constraint on the implementation, not a happy accident, and the tests check it by
comparing against exact integer arithmetic.

### Exact arithmetic as the check

For a given decision the number of unknown dice is fixed, so every claim's probability is
an integer numerator over the same denominator — five and six raised to integer powers. That
numerator is bounded by six to the power of the unknown dice count, which fits in a `u128`
up to forty-nine unknown dice and overflows at fifty.

Ten players holding five dice each is fifty dice, and the asker always holds at least one,
so at the start of a match the exact path covers every state with nothing to spare. It is
not a guarantee for the whole match: spot on grants a die and nothing caps the total, so a
long match can drift above fifty dice and a player down to one die would then see more than
forty-nine unknown.

So exact arithmetic is the test oracle rather than the shipping path. Floating point ships;
the oracle verifies it across every state it can reach, which is the opening position and
nearly everything after.

## The AI decision rule

Realizes the AI statements, which say what an AI opponent knows and does not model. This is
how it chooses, and it has no tuning knob.

1. Enumerate every legal claim: quantities from the current one up to the dice in play,
   each face value, excluding the one face when ones are wild. The rules bound this, since
   a claim must be possible according to the number of dice in play.
2. Compute each one's probability from the opponent's own dice.
3. **Raise** with the most probable. Among equal probabilities, take the lowest quantity and
   then the lowest face.
4. **Claim liar** instead when the previous claim is more likely false than the best raise is
   true.
5. **Claim spot on** instead when that is more likely than either.

Comparing probabilities directly is what makes "the minimum it can get away with" and
"calls liar when the other options drop too low" fall out with nothing to calibrate. Face
value does not change a claim's probability on its own; only the opponent's own holdings
break the tie, which is what step three's ordering is for.

An AI opponent is handed only its own dice and the public history. It cannot reach another
player's dice, so the knowledge statements are enforced by the function signature rather
than by discipline.

## Colour assignment

The human picks a colour from the palette; opponents take others. The statement is that the
assignment maximises the smallest difference between any two players' colours.

The palette is twenty colours: six hues at sixty degree spacing, each with a light, a base
and a dark variant, plus black and white.

| Hue | Light | Base | Dark |
|---------|-------------|---------|------------|
| Red | pink | red | maroon |
| Yellow | cream | yellow | olive |
| Green | light green | green | dark green |
| Cyan | pale cyan | cyan | teal |
| Blue | light blue | blue | navy |
| Magenta | orchid | magenta | purple |

Difference is measured in a perceptually uniform space rather than in sRGB, where equal
numeric distances are not equally visible.

This is a selection problem, not a generation one: choose the subset of the palette
containing the human's colour that maximises the minimum pairwise distance. With twenty
colours and at most ten players the search space is small enough to solve exactly rather
than greedily, so the result is the best assignment rather than a good one.

A palette larger than the player limit is deliberate. The selection never puts pink next to
red unless it has run out of better options, so extra entries only widen the gap at every
player count.

## Presentation

Values are drawn as numerals rather than pips. The minimal game is mechanics only, so a
numeral is the most direct reading of "a die is shown as its value in its owner's colour",
and pips would be the beginning of drawing dice as objects.

The claim history lists the current round only. Each entry is the claimer's dice count and
their claim, in their colour, which is what identifies them without naming them.

## Interaction

Every control is a tap target; nothing drags and nothing needs a keyboard. A mouse click and
a single touch arrive as the same input, so the same build serves both.

Quantity, face value and opponent count each have an increase and a decrease. An adjuster
whose result would be an illegal claim is disabled rather than clamped silently, so the
reason a move is unavailable is visible. Liar and spot on are unavailable on the opening
claim of a round, where there is no previous claim to refer to.

This is what satisfies "an invalid claim cannot be made": the illegal states are
unreachable, not rejected after the fact.

## Boundaries
- **`crates/rules`** — pure. Knows the rules, not probabilities, not rendering.
- **`crates/odds`** — pure. Knows dice, not rules.
- **`crates/ai`** — depends on both. Receives only its own dice and public history.
- **`crates/render-minimal`** — draws. Knows nothing of rules or probabilities; it is given
  what to show.
- **`apps/minimal`** — composition root. Wires the above and owns the input.
- **No dependency on** `protocol` or `integrity`. The minimal game has no network, and the
  Out of Scope statements are enforced by the dependency graph.

## Decisions

**Floating point with arithmetic only, rather than exact rational arithmetic.** Exact
comparison is possible and is used in tests, but the numerators overflow a `u128` at around
forty-eight unknown dice, and opponents have no upper bound. Restricting to the four
arithmetic operations keeps the result reproducible without a big-integer dependency.

**One probability function for the AI and for the human's readout.** Two implementations
would eventually disagree, and the one the human sees would be the one nobody tested.

**An exact search for colours rather than a greedy pick.** Twenty palette entries and at
most ten players is small enough to solve exactly, so the assignment is the best one rather
than a good one. A greedy farthest-point pick would be the fallback if either number grew.

**Numerals rather than pips.** See Presentation.

**A separate `apps/minimal` rather than reusing `apps/local`.** They are different things:
one is a product, the other a harness for the networked game.

## Determinism
- The probability model uses addition, subtraction, multiplication and division only. No
  transcendental functions anywhere in the model.
- Binomial coefficients are accumulated with integers, never from factorials.
- The AI's choice is a pure function of its own dice and the public history. Ties are broken
  by a stated total order, so there is no dependence on enumeration order.
- The roll is the only non-determinism, and it routes through the single pluggable
  abstraction, which is what lets a whole match be replayed or scripted.

## Testing
- The floating point probability agrees with exact integer arithmetic for every case small
  enough to compute exactly.
- Known values: at least one of a face among one unknown die is one sixth; at least one
  among none is zero; a claim already satisfied by the asker's own dice is one.
- A probability never rises as the quantity rises.
- With ones wild, an unseen die is twice as likely to count toward a face that is not the
  one face.
- An AI opponent never produces an illegal claim, over an exhaustive sweep of small states.
- An AI opponent never names the one face when ones are wild.
- A match played entirely by AI opponents terminates, and ends with exactly one player not
  eliminated.
- A match never has more than ten players.
- No sequence of adjuster presses reaches an illegal claim, over an exhaustive sweep of
  small states.
- Colour assignment: for every player count, no other subset of the palette containing the
  human's colour has a larger minimum pairwise distance.
- The same roll sequence replays to the same match.

## Open Items
Deferred: how the touch build is delivered — a browser on a phone or a native mobile
build. That is P9, still open for every application but this one's input model.

/specification/game-rules.md says a match has a configurable time limit for taking a turn,
and the minimal game does not implement one. A local single player game with a turn clock
would be hostile, so the limit is presumed configurable to off — but no statement says the
limit can be switched off, so this is a gap rather than a decision.

## Status
Built, apart from the presentation statements a renderer would carry. `crates/odds`,
`crates/rules`, `crates/ai`, `crates/render-minimal` and `apps/minimal`, run with
`scripts/run-minimal`.

Not yet done: the interface is an egui window rather than a laid out two dimensional
board, and opponents take their turns the moment the human's call is made rather than one
at a time. Both are presentation, and neither contradicts a statement.
