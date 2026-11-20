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

Not in this unit of work: identity, which is named under Open Items with what it costs.

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
| Entry | A name, a colour, and a relay. Ends at *Look for matches*, which is what opens the socket. |
| Lobby | Connected and not at a match: what is open with a Join each, a field to host by name, a field to join by id, and the way out. |
| Waiting | Hosted and not started: shows the match id to pass to somebody, who has joined, and the start button. The host's screen only. |
| Playing | The seat's own view. The same panels the hot seat game draws: the table, the dialler, the history, the odds. |
| Watching | Eliminated and the match goes on. The same panels, with nothing to press: an eliminated seat is dealt and shown exactly what any seat is, and holds no dice. |
| Halted | Playing or Watching, with a line naming the seat that has gone quiet. Left by itself when they claim. |
| Over | The ranking, and the choice to play again or go. |

Playing and Watching are the same screen with the dialler removed, because that is what
the difference amounts to: `Seated::playing` is whether this seat still holds dice. The
`View` already returns nothing it should not, so an eliminated seat watching reveals
nothing a playing seat would not have seen.

**Halted is a line on that screen rather than a screen of its own**, which is a change
from what this document first said. A page that replaced the table with the words *waiting
on Bob* would hide the match that is waiting, and the match is the thing somebody is
looking at. So the three are one function of two facts, and the choice is a function and
not a field: a remembered screen would be a second place that knows whether this seat holds
dice, and the two would disagree.

**And a client cannot tell an absent player from a slow one.** Presence is deliberately not
in the log, so nothing is coming that says Bob's tab has closed; all this page knows is that
nothing has arrived for twenty seconds. The line therefore says what has been heard and
never who is there — it offers both readings and says it cannot tell them apart — and
crossing the threshold changes no rule and sends no message. A seat is never halted on its
own turn, however long it sits there, because then the match is stopped by the person
reading the screen and blaming somebody else for it would be a lie.

**Leave is available from Playing, Watching and Halted.** From Watching it is the
ordinary way out, and that is the case the whole mechanism exists for. From Playing it
forfeits, because `A player who leaves a match is eliminated`. From Halted it is how
somebody stops waiting on a player who is not coming back — the match stays halted, which
is the point: it sits there, and whoever is left can host another, join another, or go.

A person reaches the match through `Hand(Rc<Cell<Option<Call>>>)` implementing `Decides`,
exactly as in the hot seat game: the screen puts a call in and the seat takes it out, so
nothing between a person and the relay branches on whether a seat is human.

**Which is why the call buttons are drawn only on this seat's turn.** A call made out of
turn is not refused, it is *kept*: `Seated` asks the driver only when the turn is this
seat's, so a call put in the hand early sits there and fires the instant the turn comes
round — a mis-click while waiting becoming a committed call a minute later, with nothing
having said so. The dialler stays, because dialling commits nothing and thinking ahead is
what it is for. Every screen that offers a call is one where `may_call` holds, and that is
checked over every combination of the facts rather than reasoned about.

### A halted match is not a problem to solve

When it is an absent player's turn the match stops, and nothing times out, forfeits, or
cleans up. The log is retained whether or not a match completes, so a halted match waits
indefinitely for its player and the screen says whose turn it is. Everyone else is free to
leave it and start or join another; nothing about one match constrains another.

Two consequences worth having written down. Abandoned matches are permanent by design,
which is a standing reason the lobby's scan over the log table matters — see
`relay-service.md`'s Open Item. And a host who leaves before the start leaves a match that
is listed as joinable and can never begin, which belongs to the lobby unit rather than
here.

### Finding a match

Two ways, and both are kept. The lobby lists what is open — host, player count, and a
button each — and a pasted id reaches a match whoever holds it. The second is not a
leftover: a listing shows only what is open to everybody, so an id is how somebody is
invited to a match before anyone else takes the seat.

**Connecting is a step of its own, before hosting or joining.** The lobby's question is
the opening frame, so the socket is open before the client has decided anything, and
hosting or joining later travels on that same socket. That is why the entry screen ends
at *Look for matches* rather than at a Host button: a client that connected only in order
to host could never have looked first.

Which also decides what leaving means. Leaving a match drops the seat and keeps the
socket, putting somebody back in the lobby where another match is — leaving the relay is
its own button, one screen on. A refusal does the same and says why, because a refusal is
the relay declining a message rather than the transport going away; only a socket that has
actually gone takes the page back to the entry screen.

## Boundaries

- **Depends on** `seat`, `render-minimal`, `rules`, `protocol`, `coordinator`, `eframe` and
  `web-sys`.
- **Does not depend on** `relay` directly, `table`, `wire`, or any AWS crate. The relay
  reaches it only as frames, and the one thing that knows the frame format is `wire`,
  through `seat`. `MatchId` and `Refused` are re-exported by `seat` so that naming a match
  and reporting a refusal do not cost a dependency on the service: both are the protocol,
  not the service that speaks it.
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

**The account is the typed name, and the host's match name is the nonce.** Both are chosen
so that nothing taken from a clock reaches the log. A match's identity is
`hmac(secret, account, nonce)`, so a nonce read from a clock would make every reload a new
match and would put a reading of the browser's clock into a permanent record; a nonce the
person typed makes `A repeated request to host creates no second match` useful rather than
merely true — host *tuesday night* twice and you are taken back to the match you already
have. The same for the account: derived from the name, it is stable across reloads without
anything being stored, and the collision between two people who type the same name is the
identity hole below rather than a new one.

**The relay's address comes from the page, not from the bundle.** The gate builds the wasm
and has no AWS credentials, so it cannot know where the relay is; the deploy knows and
publishes the page. So `index.html` carries `<meta name="relay" content="RELAY_SOCKET_URL">`
and the deploy writes the real address over the placeholder, which is checked for rather
than assumed — a silent miss would ship a page that cannot connect. The entry screen shows
whatever it found and lets it be edited, which is the only way a local `trunk serve` can
reach a deployed relay.

**No socket outside a browser.** The native build exists so that the screens, the queue and
the choice of screen are compiled, linted and tested by the same gate as everything else,
and its `socket::open` says there is no transport. A second socket implementation here
would be a second thing to keep in step with the first, tested by nothing, to no end:
`apps/probe` already plays whole matches over a real socket from a terminal, which is what
a native build of this would be for.

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

- The frame queue preserves order, through a clone, which is how it is used: the callback
  holds one handle and the frame loop holds the other.
- A call put in the hand is taken exactly once, so pressing a button sends one claim rather
  than one per frame until the relay answers.
- The choice of screen, over **every** combination of the four facts it reads. It is a free
  function of those facts rather than a method on the page, so each case is stated rather
  than reached: a finished match shows its result whatever else is true; a match that has
  not started can show nothing but Waiting; a seat is never halted on its own turn; and
  quiet decides nothing until another seat has the turn.
- A seat opened without a browser leaves the page at the entry screen with the reason on it,
  rather than in a match it has no socket for.
- A call is offered only on this seat's turn, over every combination of the facts, and no
  halted screen offers one.
- The panels themselves are checked by eye, as the shield indicator was. No test can read a
  canvas, and saying so is better than a test that pretends to.
- Two browser tabs playing each other, by hand.

What a browser has shown so far and what it has not, because the difference matters: the
page is served, the bundle boots, the entry screen draws, and the palette, the fields and
the buttons are right. The socket's own paths — a frame arriving, a socket refusing to
open, a close reported to the screen — have not been watched in a browser, because the
deployed relay is a protocol behind this code and will refuse every opening frame until a
deploy lands. They are the one thing here that only a browser can show, and saying they are
untried is better than counting the tests that do not reach them.

## Open Items

**A host who leaves before the start.** Host powers are the seat in the log, so a match
whose host has gone can never be started and is still listed as joinable. The lobby now
exists and still lists it, because not listing it would need a statement nobody has
approved: `Every match that can be joined is listed` is true of it, and whether a match
that cannot begin counts as joinable is a question about the specification rather than
about the fold.

**A second join from one account takes a second seat.** Found in the log of the first
match two people played on the published page: the same account appears on seat 1 and
seat 2. `join` never asks whether the account is already there, so reloading and joining
again occupies two seats — and because `attach` moves a socket to the newest, the earlier
seat is left with no connection and halts the match at its turn, permanently. It is in the
log rather than the connection table, so no reconnection clears it. The fix is to answer a
repeat with the seat already held, exactly as `host` answers with the match already made,
and it wants a statement first.

**Reconnection is written and unused.** `Message::Resume` exists and the relay replays
everything a seat may see. Nothing in this client calls it yet, so closing a tab loses
the seat until the page knows to resume rather than to join.

**An asserted account is recorded as a Discord one.** The log says `discord:web-sean` for
somebody Discord has never heard of, because `Account` has no variant for a name a client
merely claimed. `Account::Asserted(String)` would make the hole visible in every line it
appears on and would tell a pre-identity log from a real one at a glance, which is worth
more than the protocol bump it costs. It is a protocol decision and wants its own statement,
so it is named here rather than slipped in.

## Status
**Built.** Published at `/liars-dice/multiplayer/`, linked from the root, and the deploy
writes the relay's address into the page. The entry, waiting, playing, watching, halted and
over screens are all drawn, the lobby lists what is open, and leaving works from every
screen that offers it. 14 tests here, 249 across the workspace with DynamoDB Local up,
clippy clean against both the host target and `wasm32`.

Confirmed in a browser, and not by the author: two people on the published page hosted a
match and joined it from a second tab. That is the socket, the frame format, the wasm
build and the page, end to end — everything `apps/probe` could not reach.

Not done, and both of it above: `Resume`, so closing a tab loses the seat; and identity,
so an account is whatever a client says it is.
