# The Coordinator: A Client, and Whatever Drives It

## Realizes
- [Architecture / Module Structure](/specification/architecture.md) — rendering and
  behaviour do not know about each other; a higher level coordinator manages them
- [Architecture / Architecture](/specification/architecture.md) — every client derives the
  same match state from the same events
- [Multiplayer / Claims](/specification/multiplayer.md) — a client does not send an
  illegal claim, and ignores one it receives

Depends on [game-state.md](/implementation/game-state.md) landing first: the coordinator
holds a `Game` that knows only its own seat's dice, which is a shape that does not exist
until that restructure is done.

## Approach

### Account and driver are different things

| | what it is | who can see it |
|---|---|---|
| **Account** | who the seat belongs to — Discord, AI, local | in the log, visible to everyone |
| **Driver** | what chooses the next call | nobody but this client |

A virtual player is a **driver**, never an account. Nothing in the relay, the protocol, the
log or another client branches on it, so there is no place for a special case to grow. A
virtual driver behind a Discord account is indistinguishable from a person at a keyboard,
which is the only basis on which "it worked with a virtual player" says anything about a
real one.

This is why there is no `Account::Virtual`. The fourth variant is the obvious move and it
would quietly destroy the property the whole arrangement exists for.

### The coordinator

```rust
pub struct Client {
    seat: Seat,
    game: Game,
    pending: Option<Call>,
    awaiting_reveal: bool,
}

impl Client {
    pub fn receive(&mut self, event: &Event) -> Vec<Message>;
    pub fn view(&self) -> View<'_>;
    pub fn act(&mut self, call: Call) -> Result<Message, Rejected>;
}
```

Events in, messages out, and a view in between. It owns the only path from the wire to
the state: nothing else may construct or mutate a client's `Game`.

`act` refuses an illegal call rather than sending it, which is
`A client does not send an illegal claim` enforced at the one place a call can originate.
An illegal claim *received* is dropped on the floor by `receive`, which is the other half
of that pair.

### The round boundary, which is the whole reason this is delicate

A round-ending claim arrives before the dice that settle it. The sequence:

1. `claim(liar)` is recorded; the round is closed to further claims.
2. The client emits `round-over` — it has seen the round end, which is all that message
   means.
3. The relay reveals once every seat has said so.
4. Each revealed `deal` is verified against the round's seed, then installed.
5. `resolve` now returns `Some`, and the resolution is applied.

Steps 1 and 5 are separated by the network. `resolve` returning `Option` is what makes
that separation visible instead of a latent bug — the client cannot accidentally resolve
early, because there is nothing to resolve with.

A desktop game runs the identical path with steps 2 to 4 collapsed to nothing, since it
already holds every die. One code path, two timings.

### The driver

```rust
pub trait Decides {
    fn choose(&mut self, view: &View) -> Option<Call>;
}
```

`Option` because a human has not decided yet. An AI and a script always have.

Three implementations to begin with:

- **`ai`** — the existing decision rule, now taking `rules::View` directly.
- **`Scripted`** — a fixed sequence of calls, for tests that want an exact match.
- **the interface** — arrives with the renderer; returns `None` until a touch.

The driver receives a `View` and returns a `Call`. It cannot reach the `Game`, so it
cannot see another seat's dice even when the test harness around it knows them.

## Boundaries

`crates/coordinator` depends on `rules`, `protocol` and `integrity`. **Not** on `relay`:
a client speaks the protocol, it does not share a library with the service that routes it.
Not on any renderer, which is the statement this crate was created to realize.

`apps/local` composes N clients and a `relay::memory` relay in one process. That is the
end-to-end harness, and the first place a whole networked match is played.

Not here: the interface, the WebSocket transport, saves.

## Decisions

**No `Account::Virtual`.** Argued above. The marker would be a branch point, and a branch
point is where fidelity goes to die.

**The driver returns `Option<Call>` rather than blocking.** A blocking driver would force
the interface to run the client on another thread, and a WASM build has no thread to spare.
Polling suits a turn-based game and keeps the client single-threaded everywhere.

**The coordinator verifies the reveal before installing it.** `install` cannot enforce it,
so the obligation sits here, in the one place the reveal arrives. A deal that disagrees
with the round's seed is refused and reported rather than applied, which is
`A player can detect dice that disagree with the event log` happening live rather than only
in an audit afterwards.

**Illegal messages are dropped silently, not reported.** The specification says a client
ignores an illegal claim it receives; it says nothing about complaining. The relay already
records the attempt, so the audit trail exists without every client inventing an error
channel.

## Determinism

The coordinator holds no randomness. Given a sequence of events it produces the same state
and the same messages every time, which is what lets `apps/local` drive a whole match from
a script and get the same log twice.

Clock-free. Nothing here times anything, and the specification has no clock to honour.

## Testing

In `apps/local`, against the real relay over `relay::memory`:

- A whole three-player match, every seat driven by the AI, played to one survivor. The log
  is then verified the way the archive verifier verifies it — commitments, seeds, every
  die recomputed.
- The same match played twice from the same secret and the same script produces an
  identical log, byte for byte.
- A client is never sent, and never holds, another seat's dice before that round's reveal —
  asserted over every event of a whole match rather than at one moment.
- A scripted driver and an AI driver in the same match, proving a seat's behaviour is a
  property of its driver and of nothing else.
- A client refuses to send an illegal call, and ignores one delivered to it, with the other
  clients' state unaffected by the attempt.
- A client disconnected mid-round and reconnected reaches the same state as one that never
  left, given the same events. This is the reconnection property stated as an equivalence
  rather than as a procedure.
- Hotseat, incidentally: three clients in one process with scripted drivers is exactly
  hotseat, and it needs no product statement because the specification describes the game
  rather than the harness.

## Status
Not started
