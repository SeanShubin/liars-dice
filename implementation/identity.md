# Identity

Who a seat belongs to, and what that is worth. Two kinds of account on one axis — proven
and not — with AI players, guests and ranked matches all falling out of where a match sits
on it.

## Realizes
/specification/architecture.md — Authentication, all of it.
/specification/multiplayer.md — Hosting (AI players), Joining (one seat per player), and
Ranked.
/specification/glossary.md — Guest and Ranked.

## Approach

### The line, and which side of it anything is on

**An unranked match is not secured, and that is a decision rather than an oversight.** A
guest id is a claim and nothing checks it, so anybody who learns somebody else's guest id
can take their seat in an unranked match. That is accepted. The whole of the security
budget is spent on one question — *is this really that Discord account* — because that is
the only question a ranked match rests on.

Everything follows from putting the line there:

| | Guest | Discord |
|---|---|---|
| Identified by | an id their own client generated | their Discord account id |
| Proven | no, and deliberately not | yes, by a ticket the relay verifies |
| Can host, join, leave, play | yes | yes |
| Survives a new browser | no | yes |
| Makes a match ranked | no | only if every seat is one |

A guest costs nothing to become: no redirect, no account, no email. That is the point. A
dice game that asks for an OAuth handshake at the door loses most of the people who were
willing to try it, and the people it keeps are not more interesting to play against.

### A guest is an id, not a name

`Account::Guest(String)` joins `Discord`, `Ai` and `Local`. The id is sixteen random bytes
in hexadecimal, generated once by the client and kept in local storage; the typed name is
display only, recorded at join like any other name.

This replaces deriving the account from the typed name, which `multiplayer-client.md`
recorded as a hole and which the relay's log has already demonstrated: two people who both
typed the same name were one account, and the second would land on the first's seat on a
`Resume`. A random id has no such collision, and it costs nothing a typed name did not.

Local storage rather than session storage, because the point of a stable id is that
closing a tab does not lose a seat — `Message::Resume` exists precisely for that, and an
id that dies with the tab makes it unreachable. The cost is that a shared browser shares
an identity, which on the unranked side of the line is not a cost worth paying anything to
avoid.

### Proving a Discord account: a ticket, not a token

The authorization code flow, `identify` scope only, no guild gate.

1. The page sends the browser to Discord's authorize URL and gets it back at
   `/liars-dice/multiplayer/?code=…`.
2. The page posts that code to **the exchange**, a small Lambda behind a function URL,
   which is the only thing holding the client secret — already in SSM at
   `/liars-dice/discord/client-secret`, read per cold start exactly as the relay reads its
   own.
3. The exchange trades the code with Discord, reads `/users/@me` once, and mints a
   **ticket**: the account id, the display name, an expiry, and an HMAC over all three.
4. The page keeps the ticket and presents it when hosting, joining or resuming.
5. The relay verifies the HMAC itself and takes the account from inside the ticket.

The relay never calls Discord. An outbound call on the hot path would make a Discord
outage into a relay outage and add a round trip to every message that names an account,
and there is nothing it could learn that the ticket does not already carry. `integrity`
already holds the HMAC and the tagging convention that keeps one message space from
overlapping another, so the ticket is a third tag beside `match` and the round seed.

The ticket carries the display name because the relay records a name at join and
`An authenticated player's name is their Discord display name`. Without it the relay would
need a Discord call for exactly the reason `deployment.md` already rejects one: an OAuth
user token reads only its own account.

### Exactly one of the two, never both

A message that can carry an account *and* a ticket is a message where the two can
disagree, and then something has to decide which wins. So they are one field:

```rust
pub enum Who {
    /// An id a client generated for itself, and the name they chose. Neither is checked.
    Guest { id: String, name: String },
    /// A ticket from the exchange. Checked, and both the account and the name come out of
    /// it.
    Authenticated { ticket: String },
}
```

**The name travels inside `Who`, which is a correction to what this document first said.**
It had the name staying beside the `Who` as its own field, and that is the same defect one
field along: `An authenticated player's name is their Discord display name` and `An
unauthenticated player chooses their own name`, so a message carrying both a `Who` and a
name is a message where those two can disagree — and the relay would have had to ignore a
field it was sent. Inside, there is nowhere to claim a name you were not given.

`Message::Host`, `Message::Join`, `Message::Resume` **and `Message::List`** carry a `Who`
where they carried an `Account` today. `List` is in that set for the same reason as the
rest: one way to say who you are, not two. The result is worth stating as an invariant —
**a client never sends an account.** It says who it is, and the relay decides what that
means. An `Account` now appears on the wire only in an event, written by the relay.

The relay turns a `Who` into an `Account` in `identify`, the one place a ticket is checked.
It refuses `BadTicket` for a ticket that does not parse, does not verify, has expired, or
names an account that does not count as proven — one refusal for all of them, because
`check_ticket` gives one answer for all of them, and a relay that could tell a forgery from
an expiry is a relay that can be asked which it was.

**A client reads its own ticket without verifying it.** A `Seated` has to know which account
the relay will call it, so that it can recognise its own `join` coming back — and it holds
no secret. So `integrity` exposes `ticket_claims`, which parses and does not verify, named
so that using it to decide anything reads as the mistake it would be. `Seated` derives its
account from its `Who` and nothing else, so there is one answer to *who am I*.

### AI players take guest accounts

A bot joins as an ordinary guest, with an id its host's client generated. The relay cannot
tell it from a person, which is the property `coordinator.md` already insists on: *a
virtual player is a driver, never an account, so nothing in the relay, the protocol, the
log or another client branches on it.* There is no `if virtual` to drift out of fidelity,
which is the only basis on which a bot working says anything about a person working.

`An AI player's turn is taken by the client that added it`, which is forced rather than
chosen: `crates/relay` may not depend on `rules` — a test enforces it — so the relay cannot
play a bot even if it wanted to. The host's client runs one `Seated` per bot, each with
`seat::Ai` as its driver.

**One socket per seat, bot or not, which is the second correction to this document.** It
said the bots would run *over the same socket* as the host, and that does not work:
`Connections::attach` evicts a connection from wherever else it was, because a socket is in
one place — which is the invariant that decides who is sent whose dice, fixed deliberately
when the place table was reshaped. So the second seat on a socket takes the first's
connection away. `crates/seat/tests/bots.rs` was written against the one-socket design
first and failed with `NotTheHost`: the host's socket had been moved to the last bot's
seat, so the relay did not believe the host was the host.

A socket each costs nothing but sockets, and it makes a bot *exactly* a player at the
transport layer as well as in the log — which is more of the property this section is for,
not less.

Two consequences worth having written down. A host who closes their tab takes the bots with
them, and the match halts at the next bot's turn exactly as it halts for any absent player
— which is the existing rule applying, not a new one. And a bot never makes a match ranked,
because it never authenticates.

### Ranked is derived, never recorded

`A match is ranked when every seat in it is held by an authenticated player`, and
`Whether a match is ranked is derived from the accounts in its log`. So there is no flag:
`MatchView` grows `ranked()`, which is `seats.iter().all(|seat| seat.account.is_proven())`.

Recording it would be a second copy of something the log already says, and it would have to
be trusted. Derived, anybody replaying the log reaches the same answer without trusting the
relay at all — which is the same argument the commitment scheme rests on.

`Whether a match is ranked is settled when it starts` falls out rather than being enforced:
`A player cannot join a match that has started` fixes the seats at the start, so nothing
afterwards can change the answer. Leaving does not, because a seat that left is still a
seat in the log.

`Listed` gains `ranked`, derived in the same fold that produces the host and the player
count, so the dashboard can say which matches are which before anybody joins one.

## Boundaries

- **`crates/integrity`** gains `ticket`, `check_ticket` and `ticket_claims`. It already
  owns the HMAC, the secret and the tagging; a second place doing HMAC would be a second
  place to get it wrong. The ticket's payload is **length-prefixed**, not NUL-separated the
  way `name_for` is: a separator has to be a byte that cannot appear in a field, and here
  there is none — a display name may hold anything, and an expiry's big-endian bytes are
  mostly zeros for every plausible timestamp. The first version did separate on NUL and
  split inside the expiry, which the round-trip test caught at once.
- **`crates/protocol`** gains `Account::Guest(String)` and `Who`. No dependency changes.
- **`crates/relay`** turns a `Who` into an `Account` and refuses what does not verify. It
  gains the secret it already holds — nothing new reaches it. `MatchView` gains `ranked()`
  and `Listed` gains `ranked`.
- **`crates/seat`** gains `integrity` as a dependency, for `ticket_claims` alone. Its
  constructors return `Option<Seated>`, because an `Authenticated` holding something that
  is not a ticket is a client that could never recognise its own join.
- **`apps/auth`** is new: the exchange, one Lambda, one function URL, no database. It is
  the only thing that ever holds the Discord client secret, and the only thing that talks
  to Discord. Its library is the handler and its binary is the composition root, so every
  answer it can give is tested without a network — the Discord call is behind a `Discord`
  trait, a seam rather than a passed-in value because the exchange decides when to call it
  and with what.
- **`apps/web`** keeps the guest id, performs the redirect, posts the code, and holds the
  ticket. It is the only place that knows local storage exists. The fetches meet the frame
  loop through `FromExchange`, the same cell-and-drain seam `Arrivals` uses for the socket:
  a fetch answers on its own schedule and a frame loop does not wait.
- **Nothing depends on `apps/auth`** except the browser. The relay does not call it; it
  verifies what the browser brings.

## Decisions

**A guest id is unforgeable to nobody, on purpose.** The alternative is proving a guest
too — a token from the exchange for an anonymous id — which costs the same machinery as
Discord and buys security for matches that were explicitly said not to need it. If that
changes, the shape is already here: a guest ticket would be the same tag with no account
id in it.

**Bots are guests rather than a fourth account kind.** `Account::Ai(String)` was the
obvious alternative and is rejected: it would put *this seat is a bot* into the log, and
the first thing to read it would be something that treats bots differently, which is how
fidelity rots. `Account::Ai` stays as it is, used by desktop games, where there is no
relay to be fooled and the distinction is local.

The cost is that a reader cannot tell a bot from a human guest. For ranked that is no cost
at all — neither is authenticated — and for anything else it is the property, not the
price.

**A ticket, not a session.** The relay holds no session table and no login state: a ticket
is self-describing and verified from the secret alone, so a cold Lambda with no memory of
anything can check one. The alternative is a table of live logins, which is state to
expire, to replicate, and to get wrong.

**Two HMAC secrets, not one.** This is a correction to what the rest of this document
assumed. The exchange has to hold the key tickets are verified with, because it mints them,
and it is the one part of this system that talks to the public internet. Sharing the dice
secret with it would mean that compromising the exchange let somebody predict every roll,
and `Clients must not be able to choose their roll beforehand or alter it after the fact`
is the property this repository is arranged around. So `relay::Secrets { dice, ticket }`,
`/liars-dice/ticket/secret` beside `/liars-dice/relay/secret`, and only the relay ever
holds the one that matters most.

The alternative was one secret with the ticket tagged apart from the seeds, which
`integrity` already does and which this still relies on — but a tag stops one message space
being mistaken for another, it does not stop a second service from deriving seeds. The tag
is about confusion and this is about reach.

They fail differently too, and that is deliberate. The dice secret is mandatory, because an
invented one would make every commitment in every match unverifiable with nothing saying
so. The ticket secret is an `Option` in both services: missing, the relay serves guests and
refuses every ticket, and the exchange answers *not configured* — which is `A missing
Discord config disables login rather than failing` falling out rather than being arranged.

**Twenty-four hours.** Long enough that a person does not re-authenticate inside one
sitting, short enough that a leaked ticket stops working. What a stolen ticket buys is a
seat in a ranked match under somebody else's name, which today is worth nothing because
nothing consumes ranked — so this is a number to revisit when something does, and it is one
constant in one place.

**A missing Discord config disables login rather than failing.** `deployment.md` already
requires that the stack deploy before the secrets are set. So the exchange answers *not
configured*, the page offers guest only and says why, and every unranked thing still works.
The one thing that must never happen is a page that looks like it can authenticate and
cannot.

**The redirect comes back to the game, not to the exchange.** `redirect_uri` is the
multiplayer page itself, which keeps the whole flow on `seanshubin.com` and means the
exchange needs no HTML, no cookie and no redirect of its own — it takes a code and returns
a ticket. The page strips `?code=` from the address bar afterwards, because a code is
single-use and a URL with one in it is a URL somebody will paste.

## Determinism

**The guest id is the one new non-determinism, and it is an input rather than a derived
value.** Sixteen bytes from `crypto.getRandomValues`, taken once at the composition root
and stored; everything downstream reads it. An identity is something the world hands the
system, like a keystroke — what the architecture statements forbid is *deriving* a value
from an unrepeatable source and putting it in the log, and nothing here is derived.

This is a reversal worth naming. `apps/web` currently derives the account from the typed
name specifically so that nothing from a clock reaches the log, and that was the right call
while there was nowhere to keep an id. With local storage there is somewhere, and a stable
random id is strictly better: it collides with nobody, where a typed name collides with
everybody who typed it.

The ticket's expiry is a clock reading, and it reaches the ticket but never the log. The
relay compares it against its own clock, which is the one place a clock may decide
anything, and a match replays identically whatever it said.

Nothing else moves. The match nonce is still the name the host typed, the seed is still
derived from the secret and the round, and the log is still the only record anything is
replayed from.

## Testing

- A ticket round-trips: minted for an account and a name, verified back to the same two.
- A tampered ticket is refused — every field, one at a time, so a passing test means the
  HMAC covers all of them rather than most.
- An expired ticket is refused, with the clock injected rather than slept through.
- `Who::Authenticated` with a ticket from a different secret is refused, which is the case
  that matters if the relay and the exchange ever disagree about which secret they share.
- A guest id is never checked, asserted directly, because an accidental check would be a
  feature nobody asked for that fails closed in production.
- Ranked over every combination of seat kinds up to a small width: all Discord is ranked,
  any guest is not, any bot is not, and an empty match is not.
- Ranked does not change when a seat leaves after the start.
- A whole match in one process with bots at some seats, through `relay::memory`, which is
  what proves the host's client can drive more than one `Seated` on one socket.
- The exchange's own handler against a stubbed Discord, including the not-configured answer.

What is not tested here: the Discord redirect itself. It needs a browser, a real
application registration and a person to click *authorize*, so it is checked by hand once
and then by the fact that a ranked match is possible at all.

## Open Items

**Nothing consumes ranked.** It is a property the log proves and nobody reads. A rating, a
leaderboard, or a filter in the dashboard would each give it a point, and each is its own unit
of work. It is built now because it has to be decided at the start of a match and cannot be
reconstructed later for matches already played.

**A guest is one browser.** No way to move an identity between devices without
authenticating, which is the trade that makes guests free. Worth saying out loud in the
page rather than letting somebody discover it.

**The exchange is a second deployable.** One more Lambda, one more function URL, one more
thing the pipeline builds and the deploy updates — and the first piece of the system that
talks to something outside AWS.

**The Discord application's registration is the one thing not in version control.** The SSM
parameter now says `https://seanshubin.com/liars-dice/multiplayer/`, and the authorize URL
the page builds says the same; Discord compares the `redirect_uri` character for character,
so the portal has to agree and only a person can make it. The old value named
`/liars-dice/api/auth/discord/callback`, copied from the Pairwise Vote pattern, which
nothing serves — the distribution has one behaviour and it points at S3.

**The page learns the client id and the redirect URI from the exchange**, in one `GET` at
startup, rather than from the bundle. That keeps the configuration in one place — SSM, read
by the one thing that already reads SSM — so the pipeline needs no second placeholder to
`sed` and the deploy role needs no new permission. It also makes `A missing Discord config
disables login rather than failing` fall out: the exchange answers *not configured*, and the
page offers guest only and says why. The cost is one request per page load, including for
guests, and a failure to reach the exchange reads as *not configured*.

## Status
Built, except for one thing that belongs elsewhere.

Built: `Account::Guest`, `Who`, the ticket in `integrity`, the relay's `identify`,
`ranked` and `Listed.ranked`, the guest id in local storage, AI players as guests,
`apps/auth` with its template and pipeline wiring, and the page's sign-in panel.

Not built here: **the browser UI for adding an AI player to a hosted match.** This unit
owns *a bot holds a guest account*, which is done and proved by
`crates/seat/tests/bots.rs` — including the correction that each bot needs its own socket.
The button that adds one belongs to `multiplayer-client.md`, because
`The host can add an AI player to a hosted match` is a statement about the client and the
work left is a screen holding several sockets rather than anything about identity.

Waiting on two things outside this repository, neither of which stops a deploy:
`/liars-dice/ticket/secret` being put by hand, and
`https://seanshubin.com/liars-dice/multiplayer/` being registered in the Discord developer
portal. Until both, login reads as off and every match is unranked — which is the designed
behaviour and not a failure.
