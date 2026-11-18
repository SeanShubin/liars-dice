# Minimal Desktop Game

## Realizes
/specification/game-minimal.md, in full.
/specification/ai-players.md, in full.
/specification/applications.md — the presentation statements, and the Desktop section.
/specification/game-rules.md — the rules the match is played by.
/specification/dice.md — values and identity.
/specification/architecture.md — thin composition root, consolidated non-determinism, and
the boundary between what is computed exactly and what is only displayed.

## Crate placement

```
crates/rules/       legal claims, resolution, match state. Pure, exact.
crates/odds/        the probability model. Pure, exact, no rules knowledge.
crates/ai/          the decision rule. Exact. Depends on odds and rules.
crates/readout/     an exact probability as a number to read. Depends on odds.
crates/render-minimal/  the two dimensional presentation
apps/minimal/       composition root
```

`apps/local` stays what it was for: running the networked game in one process against a
simulated environment. The minimal game is a product rather than a harness, so it gets its
own composition root.

`crates/odds` knows nothing about rules or turns. It answers one question about dice, and
both the AI and the human's readout ask it — the human sees exactly what an AI in their
seat would compute, which is also the fairness property that makes the readout honest.

`crates/readout` exists so that the fraction a person reads is produced somewhere the AI
cannot reach. It holds no model of its own; it scales the exact value `crates/odds`
returned. See the boundary section below for why that is a crate rather than a convention.

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

Nothing in that sum is evaluated as a fraction. `p` is a count of faces over a count of
faces, so the whole expression is a count of equally likely outcomes over a count of
equally likely outcomes, and every step of building it is integer arithmetic. Binomial
coefficients are accumulated one row at a time rather than from factorials, multiplying
before dividing, so each step lands on a whole number.

### Which side of the boundary each value is on

/specification/architecture.md draws the line at the event log: a value determining the
content of an event is exact, a value only displayed to a person may round, and nothing
crosses from the second to the first.

A claim an opponent makes is an event. So the AI's probabilities are on the exact side,
which is not a technicality — two things follow from it that would not hold otherwise.

The first is that the decision rule's tie break becomes reachable. Step three below says
that among equally probable claims the lowest is taken. Rounded, two claims of genuinely
equal probability generally do not compare equal, so that clause never fires and the
ordering falls to rounding noise instead of to the stated rule. Exactly, equal means equal.

The second is that an opponent's play does not depend on how the arithmetic is arranged.
Reordering the terms of a sum is a change with no meaning in the game, and rounded it moves
the last bits, flips a comparison somewhere, and diverges every recorded match being
replayed as a regression test. Exactly, the sum is the sum.

The readout a person sees is on the display side, and it is the only thing there. It is
`crates/readout`, a separate crate rather than a module or a convention, because then the
direction of the dependency is what keeps the two apart: `crates/odds` and `crates/ai` do
not depend on it, so a decision made from a rounded number is a compile error rather than
something to remember in review. Each of the three exact crates also carries a test that
greps its own sources for a floating point type, which catches the other way in — writing a
rounded value rather than importing one.

### No ceiling on the exact path

For a given decision the number of unknown dice is fixed, so every probability in it is a
count over the same denominator, six raised to that number of dice. The count outgrows a
`u128` at fifty unknown dice. Ten players holding five dice each is fifty dice, so the
machine integers do not cover a full table, and spot on grants a die with nothing capping
the total, so no fixed width removes the problem — it only moves the ceiling.

So `crates/odds` carries `Natural`, a non-negative integer of unbounded size: add,
subtract, multiply, divide by a small divisor, raise a small base to a power, compare. That
is every operation a binomial needs and no more. There is no size at which the model stops
working, so no state in any match falls back on anything, and the machine integers serve as
a cross-check over the range where they still hold the answer rather than as the thing that
ships.

## The AI decision rule

Realizes the AI statements, which say what an AI player knows and does not model. This is
how it chooses, and it has no tuning knob.

1. Enumerate every legal claim: quantities from the current one up to the dice in play,
   each face value, excluding the one face when ones are wild. The rules bound this, since
   a claim must be possible according to the number of dice in play.
2. Compute each one's probability from the opponent's own dice, exactly.
3. **Raise** with the most probable. Among equal probabilities, take the lowest quantity and
   then the lowest face.
4. **Claim liar** instead when the previous claim is more likely false than the best raise is
   true.
5. **Claim spot on** instead when that is more likely than either.

Comparing probabilities directly is what makes "the minimum it can get away with" and
"calls liar when the other options drop too low" fall out with nothing to calibrate. Face
value does not change a claim's probability on its own; only the opponent's own holdings
break the tie, which is what step three's ordering is for.

Step two does not count outcomes for every claim. Once the unknown dice and the chance per
die are fixed, a claim's probability depends on nothing but how many of those dice it still
needs, and it never rises as that number rises, so the best claim among those sharing a
chance per die is the one needing fewest — two integers, compared. Claims are reduced that
way first, and the counting happens once per distinct chance per die, of which the rules
allow one. Grouping rather than assuming that keeps the step correct if the rules ever allow
two: with ones wild a claim on the one face would carry a different chance, and such a claim
is illegal, but nothing in the implementation leans on that.

An AI player is handed only its own dice and the public history. It cannot reach another
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
- **`crates/rules`** — pure and exact. Knows the rules, not probabilities, not rendering.
- **`crates/odds`** — pure and exact. Knows dice, not rules.
- **`crates/ai`** — exact. Depends on both. Receives only its own dice and public history.
- **`crates/readout`** — the display side of the probability model, and the only floating
  point in it. Depends on `odds`. Nothing on the exact side depends on it, which is what
  enforces the boundary rather than describing it.
- **`crates/render-minimal`** — draws. Knows nothing of rules or probabilities; it is given
  what to show.
- **`apps/minimal`** — composition root. Wires the above and owns the input.
- **No dependency on** `protocol` or `integrity`. A desktop game has no network, and the
  statements that say so are enforced by the dependency graph.

## Decisions

**Exact arithmetic for anything an opponent decides from, floating point only for what a
person reads.** The alternative was reproducible floating point throughout, which is sound
on its own terms — Rust's basic operations are IEEE 754 and contract nothing, so the same
inputs do give the same bits on every platform the game targets. It fails for a different
reason: a rounded comparison cannot express "equally probable", so the decision rule's tie
break is unreachable, and it ties what an opponent plays to the order the arithmetic happens
to be written in. Exactness costs one unbounded integer type and buys both back.

**An unbounded integer rather than a wider fixed one or a cap on dice in play.** A `u256`
would move the ceiling from fifty unknown dice to about ninety-nine rather than remove it,
leaving a branch reachable only in a long match and therefore never exercised. Capping the
dice in play would remove it, but by changing the rules to suit the arithmetic. The
operations a binomial needs are few enough that unbounded is the smaller change, and it
leaves no branch at all.

**One probability function for the AI and for the human's readout.** Two implementations
would eventually disagree, and the one the human sees would be the one nobody tested. The
readout does not recompute; it scales the exact value, so there is one model and one place
that rounds.

**An exact search for colours rather than a greedy pick.** Twenty palette entries and at
most ten players is small enough to solve exactly, so the assignment is the best one rather
than a good one. A greedy farthest-point pick would be the fallback if either number grew.

**Numerals rather than pips.** See Presentation.

**A separate `apps/minimal` rather than reusing `apps/local`.** They are different things:
one is a product, the other a harness for the networked game.

## Determinism
- Every value deciding what an opponent plays is an exact integer count. Nothing on that
  path rounds, so there is nothing for a platform to round differently.
- Binomial coefficients are accumulated with integers, never from factorials.
- The counts have no ceiling, so no state in any match reaches a fallback.
- The AI's choice is a pure function of its own dice and the public history. Ties are broken
  by a stated total order, so there is no dependence on enumeration order — and because
  equal probabilities compare equal, the tie break is actually reached.
- Floating point appears only in `crates/readout` and in colour assignment, neither of which
  anything consequential depends on.
- The roll is the only non-determinism, and it routes through the single pluggable
  abstraction, which is what lets a whole match be replayed or scripted.

## Testing
- The unbounded integer arithmetic agrees with the machine integers across the whole range
  where the machine integers still hold the answer, which is up to forty-nine unknown dice.
- The model answers past that range rather than failing, out to two hundred and fifty dice.
- Every outcome is accounted for: the counts of exactly none through exactly all sum to the
  total number of outcomes.
- Two claims of equal probability compare equal, and the raise chosen does not depend on the
  order the legal claims arrive in.
- `crates/rules`, `crates/odds` and `crates/ai` name no floating point type.
- The readout agrees with a direct floating point binomial wherever that one is trustworthy,
  stays within zero and one, never rises as the quantity rises, and reads a probability too
  small to write as zero rather than failing.
- Known values: at least one of a face among one unknown die is one sixth; at least one
  among none is zero; a claim already satisfied by the asker's own dice is one.
- A probability never rises as the quantity rises.
- With ones wild, an unseen die is twice as likely to count toward a face that is not the
  one face.
- An AI player never produces an illegal claim, over an exhaustive sweep of small states.
- An AI player never names the one face when ones are wild.
- A match played entirely by AI players terminates, and ends with exactly one player not
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

Closed: the minimal game implemented no turn clock, which was a gap against a statement
that a match has a configurable time limit. That statement is gone — nothing in the game
is timed — so the absence is now correct rather than a gap.

## Status
Built, apart from the presentation statements a renderer would carry. `crates/odds`,
`crates/rules`, `crates/ai`, `crates/readout`, `crates/render-minimal` and `apps/minimal`,
run with `scripts/run-minimal`.

Not yet done: the interface is an egui window rather than a laid out two dimensional
board, and opponents take their turns the moment the human's call is made rather than one
at a time. Both are presentation, and neither contradicts a statement.
