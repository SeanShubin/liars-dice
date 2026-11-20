# The Multiplayer Client

The browser half of a networked match: a socket, one seat, and the presentation the hot
seat game already draws with. `crates/seat` is the state machine and is built and proven;
this is everything between it and a person.

## Realizes
/specification/multiplayer.md — Hosting, Joining, Claims and Versions, as a person plays
them rather than as a probe does.
/specification/applications.md — the web game.
/implementation/relay-service.md — the service this talks to, and the wire format.
/implementation/hotseat.md — the presentation, and the `Decides` trick that lets a person
sit behind the same trait an AI does.

Not in this unit of work: the lobby and identity. Both are named under Open Items with
what they cost.

## Approach

### What is already done

`crates/seat` holds one seat with no transport in it: frames in, frames out. `apps/probe`
gives it a socket from a terminal and plays whole matches through the deployed relay. So
the sequencing, the turn taking, the round settling and every defect that comes of a
client being a process rather than an index in a vector are already found and fixed —
three of them were, and none could have happened in one process.

What is left is a socket that lives in a browser, a person where the AI was, and screens.

### The socket, and the one shape a browser forces

`web_sys::WebSocket` is callback driven and `eframe` is a frame loop, so they meet through
a queue: the message callback pushes a frame onto `Rc<RefCell<VecDeque<String>>>` and
`update` drains it. `Rc<RefCell<_>>` rather than `Arc<Mutex<_>>` because a tab is one
thread and a lock nobody contends is a lie about the code.

Draining in `update` means frames are handled in the order they arrived, which is the
order the log is in, which is the only order anything may be applied in.

### A networked client may not pause the match

The hot seat game stops at a resolution and waits for the person to acknowledge it, which
it can do because it owns time. A networked client does not own time. The next round opens
the moment every seat acknowledges the last one, and whoever opens it may claim
immediately — so a client that held its game back to show an outcome would apply that
claim to the round it was still showing.

So the match advances as soon as the reveal arrives, and the resolution is *shown* from a
copy rather than by holding the game still. The outcome is a panel over a match already
in progress, not a screen that blocks it. `Seated` keeps the last resolution for drawing
and begins the next round itself.

This is the one place the two deliveries genuinely differ, and it is worth the asymmetry:
the alternative is a client that is wrong whenever somebody else is quick.

### Screens

| Screen | What it is for |
|--------|----------------|
| Entry | A name, a colour, and either host or join. Joining takes a match id, pasted. |
| Waiting | Hosted and not started: shows the match id to pass to somebody, who has joined, and the start button. The host's screen only. |
| Playing | The seat's own view. The same panels the hot seat game draws: the table, the dialler, the history, the odds. |
| Over | The ranking, and nothing to press. |

A person reaches the match through `Hand(Rc<Cell<Option<Call>>>)` implementing `Decides`,
exactly as in the hot seat game: the screen puts a call in and the seat takes it out, so
nothing between a person and the relay branches on whether a seat is human.

### Finding a match

By pasting a match id, until the lobby exists. The host is told its id in the `created`
frame and shows it; the other player types it in. That is enough for two people to play
and it needs no change to `crates/relay`, where the lobby does — see Open Items.

## Boundaries

- **Depends on** `seat`, `render-minimal`, `rules`, `protocol`, `eframe` and `web-sys`.
- **Does not depend on** `relay` directly, `table`, or any AWS crate. The relay reaches it
  only as frames, and the one thing that knows the frame format is `wire`, through `seat`.
- **Owns** the socket, the screens, and the person's half of a `Decides`.
- **Does not own** any rule, any secret, or any decision about what another seat holds. It
  draws from a `View`, which returns `None` for another seat's dice until the reveal, so
  the presentation cannot show what this seat may not see.
- **Nothing depends on this.** It is a binary.

## Decisions

**The same presentation as the hot seat game, not a second one.** `render-minimal` draws
from a `View` and knows nothing about where the view came from. A networked match is a
different source for the same panels, so the screens differ and the drawing does not.

**One seat, not a table.** `crates/table` owns the relay and every player, which is what
lets a whole match run in a unit test; it is the wrong shape for a client, which owns one
player and no relay. The two arrangements already share the odds calculation, and sharing
more would mean one of them pretending to be the other.

**The account is asserted, not proven, and this is a hole.** Until identity exists a
client sends whatever account it likes, so anyone who knows another player's account id
can claim their seat on a resume. It is written down here rather than left implicit
because the fix is a unit of work and not a patch: the relay must be given an account it
can trust, which means the identity service.

## Determinism

Nothing here derives a value that reaches the log. A call is chosen by a person, the odds
on it are computed by `ai::Seen::chance_of_call` and rounded for display, and everything
else in a frame was decided by the relay. The one clock is the frame loop, and it reaches
nothing but the screen.

## Testing

The logic is tested where the logic is: `crates/seat` plays a whole match through the wire
format with no socket, and `apps/probe` plays one through the deployed relay. This crate
is a composition root, so what is left to check is what a composition root can get wrong.

- The frame queue preserves order, tested over the queue alone without a socket.
- The screens are checked by eye, as the shield indicator was. No test can read a canvas,
  and saying so is better than a test that pretends to.
- Two browser tabs playing each other, by hand, once the page is published.

## Open Items

**Nothing asks for the lobby.** `Relay::open_matches` exists and no `Message` reaches it,
so a client cannot list matches. Pasting an id is the way around it, and the lobby needs a
message, an answer frame, and a statement about who may ask — which is a unit of work in
`crates/relay`, not here.

**Reconnection is written and unused.** `Message::Resume` exists and the relay replays
everything a seat may see. Nothing in this client calls it yet, so closing a tab loses
the seat until the page knows to resume rather than to join.

**The page is not published.** The pipeline builds and publishes one browser bundle, the
hot seat one. This needs a second `trunk build`, a second path, and a link from the root,
which the gate checks.

## Status
Not started.
