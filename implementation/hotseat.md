# Hotseat

## Realizes

- [/specification/applications.md](/specification/applications.md) — Hotseat
- [/specification/game-minimal.md](/specification/game-minimal.md) — the presentation it is played in
- [/specification/game-rules.md](/specification/game-rules.md) — `Players know their own dice`
- [/specification/dice-integrity.md](/specification/dice-integrity.md) — `No player can learn another player's dice before the reveal`
- [/specification/architecture.md](/specification/architecture.md) — Deployment, the path it is served at

## Approach

Hotseat is every seat of one match in one browser, with the screen passed from hand to
hand. The thing that makes it worth building is the thing that makes it hard: one screen
and several players who must not see each other's dice.

### The screen shows a view, not a game

`rules::View` is a lens over a `Game` that hides another seat's dice as a type fact —
`dice_of` returns `None` for a seat that is not its own until the round is revealed
(`crates/rules/src/lib.rs:550`). Every panel on the screen is handed a `View` and nothing
else. That is what makes `The screen shows nothing that seat cannot see` structural rather
than careful: a panel cannot render a leak, because the dice are not reachable from what it
was given.

This replaces what the harness did before, which was to show every seat at once from
`game.view(HUMAN)` plus the relay's whole log. The log pane was the worse of the two: a
relay log holds a `Deal` for every seat, while a real client is only ever sent its own
(`crates/relay/src/lib.rs:511`). The pane now reads the seat's own received stream, so it
shows what that client actually holds.

### Handover

The screen belongs to one seat at a time. When the next seat to act is a human other than
the one being shown, a handover screen comes between them. It renders no part of the match
— only the name of the seat taking the screen and one button.

Nothing stops the previous player from pressing that button and peeking. That is inherent
to one device and every hotseat board game lives with it; it is recorded here so it is a
known limit rather than an oversight.

A virtual player never triggers a handover: its turns run inside `advance`, and the person
holding the screen learns what it claimed from the round's claim list when the screen comes
back to them.

### The lobby is the real lobby

A hotseat match is hosted, joined and started through the relay's own messages, which
`crates/table::seat` already sends (`crates/table/src/lib.rs:71`). The only difference from
a networked lobby is who fills the seats: the host adds virtual players instead of waiting
for people to arrive. So the lobby flow is exercised by hotseat rather than bypassed, and
there is no second seating path to keep in step with the first.

### Stopping where a person has to look

`Table` drove a match to completion, which is right for a test and useless for a player.
It gains a step function that reports why it stopped:

```rust
pub enum Waiting {
    /// This seat's driver returned `None`: a person has to choose.
    Call(usize),
    /// A round resolved. It stays on screen until the local player moves on.
    Outcome(Resolution),
    /// Nobody can move again.
    Over,
}
```

`advance` runs turns while drivers keep answering and returns the first of these it hits.
`resume` dismisses a shown outcome and begins the next round. `play` becomes a loop over
the two, so every existing caller and test keeps its behaviour.

The `Outcome` stop is what makes `A round's outcome is shown before the next round begins`
true. It works because of an accident of the rules that is worth naming: `Game::resolve`
sets `revealed` and leaves the round's claims alone, and it is `Game::next_round` that
clears them (`crates/rules/src/lib.rs:481`, `:500`). Between the two calls the game *is*
the reveal state — every seat's dice visible through any view, the round's claims intact.
So the reveal screen needs no snapshot of anything.

### One presentation, two deliveries

`apps/minimal` and `apps/hotseat` must look the same, because they are the same
presentation. The way to make that true is to have one copy of it, so the widgets and
panels move out of `apps/minimal` into `crates/render-minimal` — a crate that already
exists, already holds the palette, and whose own doc comment already calls itself "the two
dimensional presentation".

The seam is `(&View, &Seating)`. A `View` carries the match; a `Seating` carries the two
things it does not, which are the names and the palette colour of each seat. Both apps
build one.

```rust
pub struct Seating { pub names: Vec<String>, pub colours: Vec<usize> }

pub fn table(ui: &mut Ui, view: &View, seating: &Seating);
pub fn standing(ui: &mut Ui, view: &View, seating: &Seating);
pub fn odds(ui: &mut Ui, view: &View, dialled: Claim);
pub fn history(ui: &mut Ui, view: &View, seating: &Seating);
pub fn ranking(ui: &mut Ui, view: &View, seating: &Seating);
pub fn dialler(ui: &mut Ui, view: &View, pending: &mut Claim);
pub fn calls(ui: &mut Ui, view: &View, pending: Claim) -> Option<Call>;
```

`odds` keeps the three rows exactly as they are today: the standing claim's own chance, the
dialled raise's chance, and spot-on's chance, one row per button. The reason the standing
claim is shown rather than liar's complement is already written down where the function
lives and travels with it.

Moving `table` is also a correctness fix. Today it decides what to show with
`show = revealed || player == HUMAN` — hiding by convention, in a function that has the
whole `Game` in its hands. Over a `View` the convention is unnecessary, because the dice it
must not draw are not there to draw.

## Boundaries

| Owns | Depends on | Depended on by |
| --- | --- | --- |
| `crates/render-minimal` — every pixel of the minimal presentation | `egui`, `rules`, `odds`, `ai`, `readout` | `apps/minimal`, `apps/hotseat` |
| `crates/table` — a whole match in one process, stepped | `relay`, `coordinator`, `protocol`, `integrity`, `rules` | `apps/hotseat`, `apps/local` |
| `apps/hotseat` — the lobby, the screens, the handover, the human driver | `table`, `render-minimal`, `eframe` | the deployment |

`render-minimal` depends on no delivery. It takes a `&mut egui::Ui` and knows nothing about
eframe, Bevy, a window or a canvas, which is what lets a Bevy desktop app and a wasm web
app share it.

`apps/hotseat` holds the `Human` driver, because a driver fed by a person is the one kind
that only a composition root with a person in front of it can supply. It is an
`Rc<Cell<Option<Call>>>` shared between the driver and the screen: the screen puts a call
in, `advance` takes it out. `Decides` has no `Send` bound, and a browser tab is one thread,
so `Rc` is the honest choice over an `Arc` that would never be contended.

## Decisions

**One egui in the workspace.** `eframe 0.33` resolved egui 0.33.3 and `bevy_egui 0.43.1`
resolves egui 0.36.2, so there were two incompatible copies and no shared widget crate was
possible. The options were to pin `eframe` to 0.36, to hold `bevy_egui` back to an egui 0.33
release, or to move `apps/minimal` off Bevy. Pinning eframe to 0.36 is the smallest change
that unifies them and it keeps both apps on a supported release; moving `apps/minimal` off
Bevy is a larger question that the pretty presentation should decide, not this document.

**A virtual player is still only a driver.** Hotseat adds no account kind, no protocol
field and no branch in the relay. The seat a virtual player holds is `Account::Ai`, which
the log already carries, and the rest of the system cannot tell the difference. This is the
property that makes hotseat evidence about real play rather than about hotseat.

**The outcome screen is public, so it needs no handover.** A reveal is public by
`dice-integrity.md:25`, so whoever is holding the screen may see it. The handover happens
after it, before the next round's deal is shown.

**All-virtual plays itself.** With no human seat there is nobody to hand the screen to, so
the match runs to the end and is watched from the first seat's chair. This keeps the
deployment smoke test — one page, one button, a whole match — without a mode that shows
more than a seat may see.

## Determinism

Hotseat introduces no new non-determinism. The dice come from the integrity scheme as
always: a match secret, `seed = HMAC(secret, match_id ‖ round)`, and a deal derived from
it, so a hotseat match's log verifies by the same procedure as a networked one. The only
inputs the app itself supplies are the match secret and the turn order, both of which enter
at the composition root and both of which are a parameter, so a test can pin them and
replay a match exactly.

The `Human` driver is the one new source of input, and it is a value put into a cell rather
than a clock or a random number. A scripted driver substitutes for it with no change to
anything else, which is how the screens are tested without a person.

## Testing

In `crates/table`:

- `advance` stops at `Waiting::Call` for a seat whose driver returns `None`, and the match
  state is untouched by the stop.
- `advance` stops at `Waiting::Outcome` with the round's claims still readable and every
  seat's dice visible through every view — the reveal state the screen depends on.
- `resume` after an outcome begins the round the resolution named, and `advance` then
  reaches the next seat.
- `play` over the new primitives produces the same match, event for event, as the old loop
  did from the same secret and the same scripts.

In `crates/render-minimal`, as assertions over what a panel is given rather than over
pixels:

- A `View` of a live round yields `None` for every seat but its own, so the dice a panel
  could draw for another seat do not exist. Asserted over every seat of every round of a
  whole match, which is the leak test stated as a property.
- The same `View` after a reveal yields every seat's dice.
- `dialler` only ever produces a claim that `check_raise` accepts, over every reachable
  standing claim — `An invalid claim cannot be made`, enforced where the number is dialled.

In `apps/hotseat`:

- A scripted match of three seats, one human-shaped driver and two virtual, reaches a
  winner through `advance` and `resume` alone.
- The screen's seat changes only through a handover when the next actor is a different
  human seat, and never when the next actor is virtual.

And by hand, once, in a browser on the deployed page: a match played to a winner.

## Status

In progress.
