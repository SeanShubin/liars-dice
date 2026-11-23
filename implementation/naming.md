# Naming

What you are called, asked for where the answer can be known. A sign in stops asking for a
name, a seat starts asking for one, a match stops needing one at all, and a Discord player
can be told apart from another Discord player with the same display name.

## Realizes
/specification/multiplayer.md — Joining: `A listed match shows the names in it`,
`A player chooses their name when they take a seat`, and `A name another player in the
match has chosen cannot be chosen`.
/specification/applications.md — Web: `A sign in survives a reload`,
`A match shows which players have an account` and
`An authenticated player's Discord username is available on demand`. The deletion of
`A person who signs in without an account chooses a display name` is this unit's too.
/specification/glossary.md — Username.

## Approach

### The same defect as the colour, and the same fix

`screens.md` moved the colour off the sign-in screen because the taken set is unknowable
there, which made `Refused::ColourTaken` a refusal nobody could anticipate. The name has
the identical defect and has had it longer: it is typed before any match exists, and
nothing has ever checked it, so two players called Alice sit in one match and the only
thing telling them apart is the colour.

So the name follows the colour to the seat. Both of the things a player picks are now per
match per player, which `Each player's dice appearance is distinct within a match` already
implied for one of them.

### The name comes off the identity

```rust
pub enum Who {
    Guest { id: String },
    Authenticated { ticket: String },
}
```

`Who::Guest` loses its name and `Message::Host` and `Message::Join` gain one — the exact
mirror of the colour, which moved *off* those two messages in the same place.
`Who::Authenticated` is unchanged, because a ticket already carries the name and a client
could never name itself.

The consequence worth noticing is that `dashboard::ask` becomes nameless. Today a person
must be named to find out what matches exist, which is a question that has nothing to do
with what they are called.

This changes the opening frame of every client, so `PROTOCOL_VERSION` goes to 10.

### The listing carries the names, so the choice is informed

A name chosen at the join is only better than a name chosen at sign-in if the roster is
visible at the join. `A listed match shows the names in it` is what makes it so.

`Listed` already carries `host: String`, and the host holds the seat the match was created
with, so the host's name is the first name in the match. Carrying both a `host` and a
`names` would put one value on the wire twice, where the second can be forgotten when the
first is changed. So `host` is replaced rather than joined:

```rust
pub struct Listed {
    pub match_id: MatchId,
    /// Every name in the match, the host's first. `A listed match shows its host` is
    /// satisfied by the first element rather than by a field of its own.
    pub names: Vec<String>,
    pub players: usize,
    pub joinable: bool,
    pub mine: bool,
}
```

`players` goes as well. This document had it staying, on the grounds that it counted seats
that had not left where `names.len()` counts every seat that ever joined — and that was
simply wrong about the code: it was `view.seats.len()`, which is the same number. So one
field answers three statements rather than two fields answering three.

**A name a player who left chose is still taken.** Leaving is an elimination and stays in
the log, so reusing the name would put two differently-accounted players under one name in
one match's log and nothing could later say which of them a line referred to. `names` is
every join's name, left or not, and the uniqueness check reads all of them.

The refusal stays in the relay as a race guard — a listing is stale by the time anybody
presses anything — but it becomes unreachable in ordinary use, which is the standard
`screens.md` set.

### A sign in has nothing to type

Anonymous sign-in becomes a button. The guest id is already in `localStorage` and already
survives a reload, and the ticket already carries the Discord name.

**One thing does have to be stored, which this document first denied.** The id and the
ticket both outlive a reload; what did not was the *answer* to which of them to use. A
reloaded page holding both a guest id and no ticket cannot tell whether this person plays
as a guest or is about to sign in with Discord, and a `Who` is sent once per socket — so
connecting on a guess would hold a socket that says guest for somebody who had chosen
otherwise. `liars-dice-signed-in` keeps the choice, written when the button is pressed or
when a code comes back from Discord, and cleared by signing out.

The page may offer the last name used as a prefill when a seat is taken. That is a
browser-local convenience recorded nowhere, and it is the difference between typing your
name once and typing it every match.

### A reload is invisible

`A sign in survives a reload`, and with `A signed in player holding no seat sees the
dashboard` that settles what the page does on load: it connects, without being asked.

What makes this cheap is that **the screen is not stored and must not be.** `screen_for`
is a pure function of three facts and says so in its own comment — *a stored screen is a
second place that knows whether this seat holds dice, and the two would disagree*. Asking
which of its three inputs are durable answers the whole question:

- `connected` is not durable and cannot be. A socket is live by definition.
- `stance` is durable, in the log, and `dashboard.md`'s return path already recovers it.
- `quiet` is a clock reading, and is not durable either.

So exactly one input is missing on load, and opening a socket supplies it. Nothing stores
a screen, and the first listing answers *am I connected* and *which match is mine* in one
round trip — which is better than any stored answer, because it cannot be stale.

`Halted` is the one screen that does not come back, and that is correct rather than a
gap: it means nothing has arrived for a while, and on a new socket the replay just did.
A returning page lands on playing or watching and re-earns `Halted` from the new clock.

**The cost, accepted:** every page load opens a socket — a restored background tab, a
bookmark opened by accident — and each then sends a keepalive every four minutes for as
long as it is open. The page stops being something that does nothing until pressed.

### A match needs no name either

The host types a match name today, and the match id is derived from it:

```rust
MatchId(integrity::name_for(&self.secrets.dice, &protocol::account_text(&account), &nonce))
```

Nothing in the specification asks a person to produce that nonce. `Two matches never share
an identity` and `A repeated request to host creates no second match` are the only
statements constraining it, and both rest on one property: **the nonce is identical on a
retry.** A person is a poor way to guarantee that and a stored random value is a good one.

So the nonce is sixteen bytes from `getRandomValues`, kept in `sessionStorage` beside the
handle, and cleared when `Created` comes back. Four cases, and the third is the one that
pays for the design:

- **The socket drops before `Created`** — the nonce is still kept, the retry derives the
  same id, and the relay answers with the match that already exists.
- **The page reloads mid-host** — `sessionStorage` survived, so the same thing.
- **Host is pressed again while the player holds a seat** — the nonce was cleared, a fresh
  one derives a different id, and `match_of` refuses `AlreadyInAMatch` naming the match
  they are actually in, which the page already knows how to offer. Already built.
- **A match finishes and the player hosts again** — a fresh nonce, a new match, and no
  collision with the one in the log.

The guard in `host` is checked *after* the id is derived, deliberately, so that a repeated
request lands on its own match. That reasoning is unchanged by this, and the comment
recording it stays.

What is lost is deliberately re-hosting a match by retyping its name. `dashboard.md`'s
return path supersedes it: the listing names the match you are in and the page resumes it
without being asked.

Saves keep their names. `The host can name a save` names a different thing, and is still
unbuilt.

### Telling two Discord players apart

A Discord display name is not unique and arrives already decided, so it can never be
refused. Two players called Alice can sit in one match and both are entitled to.

Discord's own answer is the username: the unique name a person chooses for themselves,
which Discord keeps distinct from the display name. **The exchange already reads it.**
`apps/auth/src/discord.rs` tries `["global_name", "username"]` and keeps whichever comes
first, so the username is fetched on every sign in and thrown away after serving as a
fallback. Keeping both costs nothing at the Discord end.

**It travels in the ticket, not in a field the client fills in.** A client that could send
its own username could claim to be anyone, for exactly the reason a client cannot send its
own Discord display name. So the ticket carries both, `identify` extracts both, and the
relay writes both onto the join event — the one place a Discord account can come into
existence from a message stays the one place a Discord name of either kind can.

`Event::Join` therefore grows a username, absent for a guest and for an AI player, and the
roster picks it up:

```rust
pub struct Player {
    pub name: String,
    pub colour: String,
    pub left: bool,
    /// Which kind of account holds this seat, from the join. Answers `A match shows which
    /// players have an account` without a second field that could disagree with it.
    pub account: Account,
    /// The Discord username, for an authenticated player. Shown on demand, never as the
    /// name.
    pub username: Option<String>,
}
```

`Event::Join` already carries `account`, so the client has been receiving it all along and
`Seated::roster` has been dropping it on the floor. That fold ends in a wildcard arm, which
is how `Event::Colour` compiled and did nothing in two places last week; see Testing.

**It is called a username and never a handle.** `Handle` is in the glossary already and
means the name a browser files a player under — browser-local, and `A handle is never part
of a request`. A Discord username is sent on every join. One word for two things is the
collision that kept a lobby and a dashboard under the same name until this week.

## Boundaries

- **`crates/protocol`** owns `Who` without a name, the username on `Event::Join`, and
  `PROTOCOL_VERSION = 10`. `a_name_appears_on_a_join_line_and_on_no_other_line` keeps
  holding: both names are on the join line and on no other.
- **`crates/relay`** owns `Refused::NameTaken`, the uniqueness check against the fold, the
  name on `Host` and `Join`, `identify` returning the username, and `Listed::names`.
- **`crates/wire`** carries the reshaped `Who`, the wider join, the names on a listing, and
  the new refusal, in both directions.
- **`crates/seat`** owns `Player::account` and `Player::username` and the roster fold that
  fills them, and takes a name where `hosting` and `joining` took a match name.
- **`apps/auth`** keeps the username in the ticket it mints. **`crates/integrity`** carries
  it in the ticket payload, which is already length-prefixed for a name that may hold a
  space and needs the same treatment here.
- **`apps/web`** owns the nameless sign-in screen, the name field where a seat is taken,
  the badge and the tooltip, the random nonce in `sessionStorage`, and the connect on
  load. It stores no screen.
- **`apps/probe`**, hotseat and desktop pass a name where they passed one before. A local
  match has no listing and no uniqueness to check.
- **Depends on** `identity.md` for the ticket and the guest id, `screens.md` for where the
  screens are, and `dashboard.md` for the listing. Nothing depends on this unit.

## Decisions

**The name is asked for at the seat, not given and then changed.** The colour is *given*
and changed in the lobby because `No value crosses from the floating point side to the
exact side` forbade computing one and dice have to be drawn regardless. A name has nothing
to compute and no urgency, so it can be asked for once the answer is knowable. The
alternative — join nameless, take a placeholder, rename in the lobby — is more symmetric
with the colour and was rejected because it needs a name on a line that is not a join,
which weakens `A name appears on a join line and on no other line`. That test is one of the
strongest guards in the system and it is not worth a symmetry.

**A name cannot be changed once a seat is taken**, where a colour can be changed until the
match starts. `A player's name is recorded once, when they join a match` already said so;
the asymmetry is what keeps the join line the only place a name lives.

**The listing carries every name rather than a host and a count.** Replacing `host` with
`names` removes a value that appeared on the wire twice. The cost is that `A listed match
shows its host` is satisfied positionally, which is a convention a reader has to be told
about — hence the field comment.

**The username, not the Discord account id.** The id was the cheaper answer: it is already
the account, already on the join line, already at the client, so the tooltip would have
cost nothing at all. It was rejected because it answers the wrong question. A snowflake
needs developer mode to mean anything to a person, where a username is what Discord itself
shows when you ask who someone is. The id remains what it has always been — the account —
and is not displayed.

**A chosen name is constrained only by other chosen names.** A Discord display name is not
chosen, so it constrains nobody, and the statement says `has chosen` rather than `has`.
That is the whole of the namespace separation and it needs no concept of a namespace in the
code. The hole it leaves is two Discord players under one display name, which the username
closes by display rather than by refusal, because a refusal is not available.

**An exposure accepted on purpose.** A listing is answered to anybody connected, so every
name in every match becomes visible to anybody connected, where today only the host's is.
And a username is directly actionable in Discord's interface where a snowflake is not, so
putting it on a join line means every match's log permanently records the usernames of
everyone who played. For a game played among people who know each other that is the intent,
but it is a decision and not a side effect.

## Determinism

**One new observation, of a kind already accepted.** The host nonce is read once from
`crypto.getRandomValues`, stored, and passed inward — the same shape as the guest id and
justified the same way: an identity is something the world hands the program, like a
keystroke. What the architecture statements forbid is *deriving* a value from an
unrepeatable source and putting it in the log, and nothing here is derived. The match id
remains a pure function of the dice secret, the account text and the nonce.

The nonce is also strictly less observable than what it replaces. A typed match name was an
observation too, and a worse one, because a person cannot be relied on to repeat it.

Nothing else moves. A name is an input, carried and recorded, and `No value derives from a
player's name` is untouched: nothing is keyed on a name, and the uniqueness check reads the
fold rather than any index.

## Testing

- A join naming a name another seat chose is refused, and the match is unchanged.
- A join naming the name of a player who **left** is refused too.
- A guest may choose the display name of a Discord player in the same match, and a Discord
  player's arrival never refuses anybody.
- **Two Discord accounts with the same display name both join one match**, and their
  usernames differ on the roster. This is the test that says what the design actually
  promises rather than what it looks like it promises.
- A name taken while a join was being decided is refused on the retry, over `Meddling`, the
  way the colour's race is tested.
- The names on a listing are the names on the roster, and the first is the host's.
- **`Seated::roster` fills `account` and `username`.** That fold ends in a wildcard arm, so
  a join carrying a username the fold ignores would compile, would pass every relay test —
  the relay's tests read the log — and would show nothing on screen. `Event::Colour` did
  exactly this in two folds last week.
- **A username cannot be asserted by a client.** There is no field for one on `Host` or
  `Join`; the test is that the join event's username is the ticket's, for a ticket minted
  with a username the client never saw.
- A ticket round-trips a display name and a username, including a display name holding a
  space, which is what the length prefix is for.
- `dashboard::ask` is answered for a client that has sent no name at all.
- A repeated `Host` with the same nonce creates no second match; a `Host` with a fresh
  nonce while seated is refused `AlreadyInAMatch` naming the held match.
- A version 9 client is refused by a version 10 relay.
- `host_of` and `others_in` read the listing's one field, including the empty case the
  relay does not produce, so a row with no names draws rather than panicking.
- `name_is_free` refuses a name in the match, trims the way the page trims before
  sending, refuses an empty one, and leaves a name differing only in case free — because
  the relay compares exactly, and folding case here would be inventing a rule.

## Open Items

**Barring a seated player still ejects nobody.** Carried from `screens.md`:
`why_not_joinable` answers a held seat as a return before it reads the bar, so
`Only the host's lobby offers to bar a player` cannot be honestly built until
`Barring a player who holds a seat removes them from the match` exists.

**AI players have names nobody chose**, so they constrain nobody and two of them could
share one. The same class of hole as the Discord one, and the same answer would work —
something shown on demand that differs — but no statement asks for it yet.

**The dashboard has nothing to call a guest.** It says "You are Alice" today. A player who
has not taken a seat has no name at all, so the line goes. Arguably right, since the
dashboard is about matches rather than about you, but it is a visible hole where text used
to be.

## Status
Built.

Seven things came out of building it that the document did not say.

**The wire format had no list of strings.** `Value` carries `Counts` for numbers and `Many`
for shapes, and its own comment warns that a variant "cannot be added without every
rendering being made to handle it". `Value::Texts` is that addition, and the renderings are
JSON and the DynamoDB codec. The subtle part is in both readers: an array of strings has to
be matched *before* the array-of-numbers arm, the same way an array of objects already was,
or a list of names fails on every element instead of being read as what it is. Encoding the
names as one separated string was the alternative and is not available — a display name may
hold any character, so no separator is safe, which is the same reason a ticket's payload is
length-prefixed rather than delimited.

**`Shape::maybe_text` distinguishes absent from wrong.** A guest's join omits the username
field entirely rather than carrying an empty one, so the reader needed an accessor that
answers `None` for a missing field and still refuses one that is present and holds the wrong
kind of value. Conflating those would let a mistyped field pass as an omitted one.

**A name sent with a ticket is ignored rather than refused.** `identify` returns a `Proved`
carrying what the ticket said, and `proved.name.unwrap_or(name)` is where
`An authenticated player's name is their Discord display name` actually happens. Refusing
instead would turn a client's politeness into an error, and the screen that sends it offers
no field to fill in. `examples::every_message` sends one deliberately, so the format carries
what a client sends rather than what the relay does with it.

**The new guard caught two of our own fixtures.** `a_seat_is_given_the_first_colour_nobody_holds`
seated three joiners all called "Joiner", and the joinability-agreement test filled a match
with players all called "Filler" — distinct accounts, one name each time. Both now name
their seats separately. That is the rule working on real code the day it was written, which
is worth more than the tests written to demonstrate it.

**The page is stricter than the relay, deliberately.** The relay refuses only a name somebody
*chose*; a listing cannot say which of its names were chosen, so `name_is_free` compares
against all of them. That costs an occasional "pick another name" where the relay would have
accepted, and it buys avoiding a collision the relay is not allowed to prevent. It never
refuses a join the relay would take — it declines to send one.

**The ticket payload went from two length-prefixed fields to three.** The forgery test gained
a username case, so the MAC is now shown to cover all four fields rather than three of them;
and the awkward-text test puts the same hostile string in both name positions, so a field
leaking into its neighbour is caught whichever direction it leaks.

**The sign-in screen's absence of a field is not tested.** The Testing section first claimed
it would be, which would mean asserting on what egui drew. The two listing helpers and
`name_is_free` are tested instead, and `screen_for` is untouched by this unit and already
covered. What the screen shows is checked by looking at it.

Every behaviour was validated by reverting it and watching the right tests fail: the name
guard (three tests), the chosen-only filter (one), the ticket's name winning over the
client's (three), the roster fold keeping the account (one), and absent-reads-as-absent in
the wire (four).
