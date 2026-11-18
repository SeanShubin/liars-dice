# Game State: The Whole Game, One Player's View, and the Operations Between

## Realizes
- [Game Rules](/specification/game-rules.md) — in full; this is where the rules live
- [Architecture / Architecture](/specification/architecture.md) — every client derives the
  same match state from the same events, and no client derives any dice but its own before
  the round resolves
- [Dice Integrity / Secrecy](/specification/dice-integrity.md) — no player learns another's
  dice before the reveal

Supersedes the shape of `crates/rules` established in
[rules-engine.md](/implementation/rules-engine.md). The mechanics decided there — the dice
limit, shield tokens, the affected player — are unchanged. What changes is who is allowed
to hold what.

## The problem with the current shape

`Match` holds every seat's dice. That is correct for a desktop game, where the one human
is entitled to nothing they cannot already see, and wrong for every client of a networked
match, which must hold a rules engine while knowing only its own dice.

Patching around it — filling other seats with placeholder faces and correcting them at the
reveal — would make the type lie, and `truth` would quietly return a wrong number for the
whole round. The fix is to put the ignorance in the type.

## Approach

Three structures and a set of operations.

### `Public` — what every player knows

```rust
pub struct Public {
    counts: Vec<u32>,
    shields: Vec<u32>,
    order: Vec<usize>,
    eliminated: Vec<usize>,
    history: Vec<Entry>,
    variants: Variants,
    current: usize,
    round: u32,
}
```

Every one of these is derivable from the event log by anyone, and every client holds an
identical copy. This is the lockstep state: `Every client derives the same match state
from the same events` is a statement about exactly this structure.

Nothing secret is in it, which is what makes it safe to say that two clients disagreeing
about `Public` is a bug rather than a consequence of them knowing different things.

### `Game` — the whole game, including what is not yet known

```rust
pub struct Game {
    public: Public,
    dice: Vec<Option<[u8; DICE_LIMIT as usize]>>,
}
```

One `Option` per seat. A complete game — the archiver's, a replay's, the desktop game's —
has every entry `Some`. A client mid-round has exactly one `Some`: its own. At the reveal
it gains the rest.

**The ignorance is now in the type**, so the delayed-dice problem stops being a convention
to remember and becomes something the compiler asks about at every use.

### `View` — one player's view, as a lens rather than a copy

```rust
pub struct View<'a> {
    game: &'a Game,
    seat: usize,
}
```

Not a parallel structure that could drift from `Game`, and not a filtered copy that someone
could forget to refresh. A borrow plus a viewpoint.

The important property is what it does over a **complete** game: `View::dice_of(other)`
returns `None` whether or not the underlying `Game` knows them. So a test harness holding
every seat's dice still cannot let a driver see them — the lens hides what the game knows.
That is what makes "if it works with a virtual player it works with a human" a structural
claim rather than a hope.

`ai::View` is deleted and the AI takes this one. There is then exactly one answer to "what
may this seat see", used by the decision rule, by the probability readout, and by whatever
is driving the seat. `implementation/game-minimal.md` already claims the human sees
precisely what an AI in their seat would compute; this is what makes it true by
construction instead of by two implementations agreeing.

### The operations

Split by what they actually need:

```rust
// Public alone. No dice, so always available, to anyone.
pub fn legal_raises(public: &Public) -> Vec<Claim>;
pub fn check(public: &Public, call: Call) -> Result<(), Rejected>;
pub fn dice_in_play(public: &Public) -> u32;

// Recording a claim. A raise passes the turn; liar and spot on close the round.
pub fn claim(game: &mut Game, call: Call) -> Result<Claimed, Rejected>;

// Needs every die. `None` while any seat is unknown.
pub fn truth(game: &Game, face: u8) -> Option<u32>;
pub fn resolve(game: &mut Game) -> Option<Resolution>;

// What a client learns at the reveal.
pub fn install(game: &mut Game, seat: usize, dice: [u8; 5]);
```

`resolve` returning `Option` is the whole design in one signature. A client that has seen
`liar` but not the reveal gets `None` and must wait. A desktop game, which always knows
every die, gets `Some` immediately. **Both run the same code**, differing only in when the
dice arrive, which is the property that makes a networked match testable from a local one.

`install` is how a client takes the reveal. It cannot be used to inject dice unnoticed,
because the coordinator verifies each seat's faces against the round's seed before calling
it — but that check belongs to the caller, and this document does not pretend the type
enforces it.

## Boundaries

`crates/rules` still depends on nothing. `crates/ai` depends on it and loses its own view
type. `crates/odds` is untouched — it answers a question about dice and knows nothing of
matches.

`apps/minimal` changes shape but not behaviour: it builds a complete `Game`, so every
`resolve` returns `Some` on the first ask.

`Deals` stays as it is. A desktop game deals locally; a client is dealt by the relay and
installs what it is given.

## Decisions

**`Option` per seat rather than a separate partial type.** Two types would need two
implementations of every operation, and the day they disagree is the day a client resolves
a round differently from the archiver. One type with explicit unknowns cannot drift.

**`View` as a borrow, not a snapshot.** A snapshot has to be regenerated whenever the game
changes, and the bug is always the one you forget. A lens is correct by construction, and
it costs a lifetime parameter.

**`View` hides other seats' dice even when the game knows them.** The alternative — a view
that shows whatever the underlying game has — would make a test over a complete game prove
nothing about a client with partial knowledge. This is the single most important line in
the design and it is one `match` arm.

**One view type for the AI, the readout and the interface.** Two types that mean "what this
seat may see" is one too many, and the project's own rule is that if two words mean the
same thing, one of them is wrong.

**`Public` is not public in the Rust sense.** Its fields stay private with accessors; the
name is about who may know it, not about field visibility.

## Determinism

No change. `Deals` is still the only non-determinism and it is still injected. Splitting
the state does not add a source of difference between two clients — it removes one, by
making it impossible for a client to hold a die it was never sent.

Exact arithmetic throughout: counts, shields, faces and the round number are integers and
determine the content of events.

## Testing

The existing 23 tests move across and keep their assertions; they exercise a complete
`Game` and should be unaffected, which is itself the check that this is a restructure
rather than a change in behaviour.

New, and only possible after the split:

- `resolve` returns `None` for a game missing any seat's dice, and `Some` once the last is
  installed. The direct test for the delayed-dice property.
- A `View` over a **complete** game returns `None` for every seat but its own. The test
  that makes the virtual-player guarantee real.
- Two games fed the same claims in the same order hold identical `Public`, one of them
  knowing every die and the other only its own. Lockstep over public state, stated as a
  test.
- `legal_raises` and `check` give the same answers from a client's partial game as from a
  complete one, since neither consults a die.
- Installing a seat's dice twice with different faces is rejected, so a late or duplicated
  reveal cannot rewrite a round.

## Status
Built. `crates/rules` is `Public`, `Game` and `View` with the operations split by what
they need; 28 tests there, 11 in `ai`, 38 test targets green across the workspace, clippy
clean with warnings denied.

Three things went differently from this document, each because building it showed
something:

**The operations are methods on the type that owns what they need, not free functions.**
`Public::legal_raises`, `Game::resolve`, `View::dice_of`. The receiver states the
requirement as exactly as a free function's first parameter would, and reads better. The
split is what mattered and the split is unchanged.

**`Deals` is gone rather than kept.** With `install`, the rules no longer need a dealer at
all: a composition root deals and installs. `crates/rules` now contains no randomness and
no trait for it, which is a stronger reading of "consolidate all non-determinism into a
single pluggable abstraction" than the trait was.

**`ai::View` became `ai::Seen`, a trait, rather than being deleted outright.** The lens
cannot be constructed freely — which is the point — so the AI's unit tests could no longer
state a position directly. `Seen` is what a decision needs; `rules::View` is the
implementation that ships, and a test fixture implements it to state a position. A fixture
that lies misleads only its own test, while nothing in production can construct a view
that sees another seat.

`Game::dealt` exists for the log and for verification, as `dealt_of` did before.
`Game::next_round` is separate from `resolve`, so a resolved round stays readable until
the next is begun — which is what lets the reveal screen show every seat's dice.
