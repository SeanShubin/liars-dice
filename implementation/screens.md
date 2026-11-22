# Screens

The staging a person moves through in a web game — sign in, dashboard, lobby, match — and
the one thing that was being asked for in the wrong place: a colour.

## Realizes
/specification/applications.md — Web.
/specification/multiplayer.md — Joining, the three colour statements.
/specification/glossary.md — Sign in, Dashboard, Lobby.

## Approach

### Two words for one screen, which is how the defect hid

The staging was already right. What was wrong was that nobody could say it out loud,
because two of the four screens had swapped names:

| The screen | Was called | Is now called |
|---|---|---|
| A name, a colour, a relay | `Entry` | `SignIn` |
| Holding no seat: what is open | `Lobby` | `Dashboard` |
| In a match, before it starts | `Waiting` | `Lobby` |
| At the table | `Playing` / `Watching` / `Halted` | unchanged |
| The ranking | `Over` | unchanged |

Neither `Lobby` nor `Waiting` was in the glossary, which is why the collision survived
review. A per-match staging room is what *lobby* means in every game anybody has played,
so the rename follows the word people already use rather than asking them to learn ours.

`implementation/lobby.md` is renamed to `dashboard.md`, because the unit it describes is
the dashboard under the new naming. Nine documents reference it.

No behaviour moves in the rename, and `screen_for` keeps its shape: still a free function
of the facts, still never stored.

### A colour was asked for where the answer could not be known

`Page.colour` is one field on the browser, chosen on the sign-in screen before a match is
known to exist. Two things follow, and both are wrong.

**The colour was per person, not per seat.** `dice.md` has said `Each player's dice
appearance is distinct within a match` since the beginning, so per-match was always the
specification; the client was simply carrying one pick across every match it ever joined.

**`Refused::ColourTaken` existed only because the choice was made where the taken set was
unknowable.** The dashboard's listing does not carry colours, so a person picked blue,
pressed Join, and was bounced for a reason the screen gave them no way to foresee. A
refusal nobody can anticipate is a design defect wearing an error message.

So the choice moves to the lobby, which is the one screen that already knows every seat's
colour — it draws the roster with a die per player. `A lobby is the only screen that
offers to change a colour` and `A player can change their seat's colour until the match
starts`.

The refusal stays in the relay, and becomes unreachable through the page. That is the
right place for it: two clients can still choose the same free colour in the same instant,
and a guard against a race belongs where the commit happens, not where the button is.

### What gives a seat its colour, and why the architecture decides this

`A seat is given a colour when it is taken`, and a seat is taken before anybody has
reached the lobby to choose. Something must supply one.

The obvious answer is the one the project already owns: `render-minimal::assign` picks the
subset of the palette that maximises the smallest perceptual difference between players,
searched exhaustively in Oklab. It is built, tested, and exactly the rule `dice.md` states.

**It cannot be used here.** A colour in a `Join` event is the content of an event, and

> A value that determines the content of an event is computed exactly, with integer or
> rational arithmetic only
>
> No value crosses from the floating point side to the exact side

Oklab is transcendental. Had the relay — or the client, which is no better, since the
colour reaches the same log — chosen a seat's colour by perceptual distance, a float would
have decided a logged value. The architecture refuses it, and refuses it wherever the
computation is put.

This also settles what `Colours not chosen by a player maximise the smallest difference
between any two players' colours` governs. It is a statement about the float side: how a
client *draws* players who will never choose, which is the desktop and hotseat case
`assign` serves, where there is no log at all. It is not a rule the relay can be asked to
follow, and reading it as one is what would have produced the violation.

### So the spread is a declared constant, checked by the maths rather than decided by it

`protocol` gains an ordering — the sequence a free colour is taken from:

```rust
/// The order a seat's colour is assigned in, spread so that any prefix of it is
/// far apart.
pub const ASSIGNMENT_ORDER: [&str; 20] = [ /* names */ ];
```

The relay takes the first name in that order no seat holds. Integer comparison and an
ordering lookup: exact, and `No value that determines the content of an event depends on
the iteration order of a collection` is satisfied because the order is declared rather
than iterated over.

The perceptual rule then reaches the constant without deciding it at runtime. A test in
`render-minimal` — which has the swatches and is allowed the floats — re-derives the
greedy max-min sequence and asserts it equals `ASSIGNMENT_ORDER`. If somebody adds a
colour or edits an RGB value, that test fails and the constant is rewritten by a person.
The float checks the exact value; it never becomes it.

`PALETTE`'s own order is **not** reused, and this is the point of having two. It runs
pink, red, maroon, cream, yellow, olive — grouped by hue, which is what somebody browsing
a picker wants and the worst possible assignment order, since the first three seats would
all be shades of red. Two orders for two jobs, neither a copy of the other.

### One place for the names

`PALETTE` holds the names and `protocol::COLOURS` now holds them too, which is two
declarations of one fact. They are kept one fact by a test rather than by one of them
deriving from the other, and that is a deliberate choice rather than the easy way out:
`PALETTE` is a table of twenty colours whose whole value is being checkable by eye, and
`swatch(COLOURS[7], 48, 168, 70)` cannot be read at all. So the names stay literal in both
places and a test compares them element by element.

Which means `render-minimal` takes `protocol` as a **dev-dependency** and not a dependency.
The presentation does not depend on the protocol at run time; a test asserts the two agree.
That is a better boundary than the one this document first proposed.

The swatches stay in `render-minimal` either way. A log records the name a person chose;
what that name looks like is presentation, and the relay has no business knowing.

### Changing a colour

A new message and a new event, which is a protocol bump to 9. `Message::Host` and
`Message::Join` also **lose** their colour field, which the plan did not say and which
follows from the same statement: if a seat is given a colour, nobody sends one to take one.
That is what makes `Refused::ColourTaken` unreachable from a join rather than merely
unlikely.

```
Message::Colour { match_id, colour }   ->   Event::Colour { seat, colour }
```

Guards, in this order, each from the snapshot the event is appended against:

1. In this match at all, else `NotInThisMatch`.
2. The match has not started, else refused — `A player cannot change their seat's colour
   once the match has started`.
3. No other seat holds it, else `ColourTaken`. *Other*: re-choosing the colour already
   held is not a collision with oneself, and answering a repeat with success is what
   `host` and `join` already do.

The event goes to everyone in the match, so every lobby redraws. A seat's colour is then
the last `Colour` for it, or the one its `Join` carried.

### The sign-in screen loses a control and gains nothing

The colour picker and its die preview leave the sign-in screen. What is left is what
`A person who signs in without an account chooses a display name` asks for, plus the relay
address — and a signed-in player has no name to type, which is already the case.

The header line is also wrong and has been since handles landed: *One seat per tab, one
person per seat* is no longer true of a browser that can hold several players. It becomes
a statement about seats rather than tabs.

## Boundaries

- **`protocol`** gains the colour names, the assignment order, `Event::Colour`, and the
  version bump. It takes no dependency to do it.
- **`relay`** gains the `Colour` handler and assigns on join. It gains no dependency: the
  order is a list of names, and it needs no swatch to pick from a list.
- **`render-minimal`** keeps the swatches and `assign`, loses the names to `protocol`, and
  gains the test that holds the order honest.
- **`apps/web`** owns the renames, the picker's new home, and the lobby's controls.
- **`seat`** passes a `Colour` frame through as it does any other message.
- **Depends on** `dashboard.md` for the screen it renames and `multiplayer-client.md` for
  the screens either side of it. Nothing depends on this.

## Decisions

**The user's vocabulary, not the codebase's.** Renaming two screens costs a rename across
nine documents and buys a name a reader already has. The alternative — keeping `Waiting`
and `Lobby` and writing the statements in those terms — would have meant every future
conversation about the screen translating between two words for it, which is how this
defect lasted as long as it did.

**The colour is chosen in the lobby, not at the join.** Choosing at the join is what the
specification used to say and what the code did, and it is unimplementable without the
client knowing the taken set before it has a seat. The alternative was to put colours into
the dashboard's listing so a joining client could pre-select a free one; rejected because
it widens what a listing tells a stranger about a match, and because a pasted match id
reaches a match with no listing entry at all, so the lobby would still need the picker.

**A declared order rather than a computed one.** The honest alternatives were to move the
Oklab maths into `protocol` — forbidden outright — or to assign by the first free index in
`PALETTE`, which is exact and gives three reds to the first three seats. A constant that a
float checks but does not decide gets the good answer without the violation. The cost is
that adding a palette entry makes a test fail until a person rewrites a constant, which is
the correct amount of friction for changing what colour a logged event records.

**`Colour` as its own event rather than a rewritten `Join`.** The log is append-only, so a
colour change is a new fact and not an edit to an old one. This also means a reader can see
somebody changing their mind, which is worth more than a shorter log.

**The AI button and the bar button are not in this unit.** Two approved statements —
`Only the host's lobby offers to add an AI player` and `Only the host's lobby offers to bar
a player` — are left unrealized here and named in Open Items. They are features with their
own weight, not controls to drop onto a screen while renaming it.

## Determinism

The colour of a seat is an observation: a person chose it, and the choice enters at the
composition root as a value like any other. Nothing derives from it — it selects a swatch
for drawing and reaches nothing else, and no rule, deal or resolution reads it.

The assignment on join is the only new value that determines the content of an event, and
it is an index into a declared array compared against the names already in the view. No
float, no clock, no iteration order. The Oklab computation that justifies the array runs
only in a test.

## Testing

- `screen_for` over every combination of the facts, under the new names. The existing
  exhaustive cases are kept; what the rename must not do is change one of them.
- **A seat taken when nothing is held gets the first colour in the assignment order**, and
  a seat taken when the first three are held gets the fourth. Asserted against literal
  names, because the point of the constant is that it is the same every time.
- **`ASSIGNMENT_ORDER` is the greedy max-min sequence**, re-derived in `render-minimal` from
  the swatches and compared element by element. This is the test that fails when somebody
  edits a colour, and the only place a float touches the question.
- **Every palette name appears exactly once in the assignment order**, and the two arrays
  are the same length — otherwise a seat could be assigned a colour no picker can show.
- A colour change before the start is recorded and reaches every seat.
- A colour change after the start is refused.
- A colour change to a colour another seat holds is refused; to the colour this seat
  already holds, it succeeds.
- A colour change from a connection holding no seat in the match is refused.
- The colour a seat is drawn in is the last `Colour` for it, over a log with two changes
  for one seat, which is the fold and not the message.
- Two tabs, by hand: both reach a lobby, both see the same roster, one changes colour and
  the other's screen changes without being touched — which is also the socket half of the
  unattended fix, still unwitnessed.

## Open Items

**Barring a seated player ejects nobody.** `Message::Bar` and `Event::Bar` are built, and
`why_not_joinable` consults the bar only for somebody who does not already hold a seat —
a seat held is answered as a return before the bar is ever read. So the host can bar a
player who is already in the match and nothing happens. The statement this wants is
`Barring a player who holds a seat removes them from the match`, which composes with what
exists: record `Bar` and `Leave` together, and `has_left` then lets the existing bar guard
refuse the re-join. Not written, because it was not in the approved batch.

**Nothing shows which player a tab is**, carried over from `handles.md` and now sharper: a
colour is per seat per match, so it cannot serve as the tab's identity either.

**The dashboard's listing still shows no colours.** Not needed now that the picker is in
the lobby, and worth noting as the thing that would be needed if joining ever has to
pre-select.

## Status
**Built.** 314 tests pass across the workspace, eighteen of them new; fmt and clippy clean
on the host and on `wasm32-unknown-unknown`; the release bundle builds under trunk.

Each behaviour was validated by reverting it and watching the right test fail: assigning
the first palette entry instead of the first free colour in the order; letting a seat
collide with its own colour; dropping the started guard; dropping `Event::Colour` from the
relay's fold; dropping it from the client's roster; and counting a seat's own colour as
taken.

Four things the document did not say.

**`Message::Host` and `Message::Join` lose their colour**, written up above.

**The `No value derives from a player's name` test had to be made stricter, not looser.**
`protocol` has a test asserting that no event line but a join carries free text, and
`colour #3 light-blue` is free text by that test's reckoning. The fix was to teach it the
twenty palette names — which is only possible because `COLOURS` now lives in `protocol`,
and which turns the arm into an assertion that a colour line carries a *palette* colour.
That is the test that now fails if a client ever sends a player's name where a colour goes.

**Two folds had a wildcard arm, and a new event is silently ignored by both.**
`MatchView::fold` and `Seated::roster` each end in `_ => {}`, so adding `Event::Colour`
compiled and did nothing. `wire::event::shape_of` has no wildcard and failed to compile,
which is the difference between the two kinds of exhaustiveness and the reason the relay's
own tests could not have caught it — they read the log directly. Two tests exist for the
folds specifically: a colour somebody *changed into* is taken, and a change reaches the
other seat's roster.

**The lobby's picker decision became a free function.** `choosable(&roster, mine)` rather
than four lines inside `ui`, for the reason `frame_owed` became a method in `unattended.md`:
a decision in a frame loop can only be tested by running a frame loop. It had an off-by-one
waiting in it — a seat that counts its own colour as taken is a seat that can never keep
the colour it has — and that is now one assertion rather than something to be careful about.

The AI button and the bar button remain unbuilt, as the plan said, and the bar statement
in Open Items is still unwritten.
